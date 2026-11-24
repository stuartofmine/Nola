// SPDX-License-Identifier: Apache-2.0
//! Current-schema gate, versioned forward migration framework, and atomic first-run initialization.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ForwardMigrationStep {
    pub from_version: u32,
    pub to_version: u32,
    pub name: &'static str,
    pub sql: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationPlan {
    pub current_version: u32,
    pub target_version: u32,
    pub steps: Vec<&'static ForwardMigrationStep>,
}

/// Forward migration registry for future schema versions beyond the 1.0.0 baseline (v1).
pub const REGISTERED_FORWARD_MIGRATIONS: &[ForwardMigrationStep] = &[];

/// Plans a strict contiguous forward migration chain (`v -> v+1 -> ... -> target_version`).
///
/// - Equal versions return an empty no-op plan.
/// - Downgrades (`current_version > target_version`) fail closed with `IncompatibleSchema`.
/// - Any missing intermediate step fails closed with `IncompatibleSchema`.
pub fn plan_forward_migrations(
    current_version: u32,
    target_version: u32,
    registry: &'static [ForwardMigrationStep],
) -> Result<MigrationPlan, StoreError> {
    if current_version == target_version {
        return Ok(MigrationPlan {
            current_version,
            target_version,
            steps: Vec::new(),
        });
    }
    if current_version > target_version {
        return Err(StoreError::IncompatibleSchema);
    }

    let mut steps = Vec::new();
    let mut cursor = current_version;
    while cursor < target_version {
        let next = cursor
            .checked_add(1)
            .ok_or(StoreError::IncompatibleSchema)?;
        let mut matches = registry
            .iter()
            .filter(|step| step.from_version == cursor && step.to_version == next);
        let Some(step) = matches.next() else {
            return Err(StoreError::IncompatibleSchema);
        };
        if matches.next().is_some() {
            return Err(StoreError::Invariant(format!(
                "duplicate forward migration step registered for v{cursor} -> v{next}"
            )));
        }
        steps.push(step);
        cursor = next;
    }

    Ok(MigrationPlan {
        current_version,
        target_version,
        steps,
    })
}

fn create_pre_migration_backup(
    conn: &Connection,
    source: &Path,
    from_version: u32,
    to_version: u32,
) -> StoreResult<PathBuf> {
    // Best-effort passive WAL checkpoint so the copied file includes committed WAL frames.
    let _ = conn.query_row("PRAGMA wal_checkpoint(PASSIVE)", [], |_| Ok(()));
    let timestamp_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let mut attempt: u32 = 0;
    let backup_path = loop {
        let mut candidate = source.as_os_str().to_os_string();
        if attempt == 0 {
            candidate.push(format!(
                ".{timestamp_ms}.pre-migrate-v{from_version}-to-v{to_version}.bak"
            ));
        } else {
            candidate.push(format!(
                ".{timestamp_ms}-{attempt}.pre-migrate-v{from_version}-to-v{to_version}.bak"
            ));
        }
        let path = PathBuf::from(candidate);
        if !path.exists() {
            break path;
        }
        attempt = attempt
            .checked_add(1)
            .ok_or_else(|| StoreError::Invariant("pre-migration backup path exhausted".into()))?;
    };
    std::fs::copy(source, &backup_path).map_err(|error| {
        StoreError::Invariant(format!("failed to create pre-migration backup: {error}"))
    })?;
    Ok(backup_path)
}

/// Atomically applies a planned forward migration chain inside a single `BEGIN IMMEDIATE`
/// transaction, optionally copying the source database file to a timestamped
/// `.pre-migrate-v{from}-to-v{to}.bak` backup before mutating.
pub fn apply_forward_migrations(
    conn: &mut Connection,
    plan: &MigrationPlan,
    backup_source: Option<&Path>,
) -> Result<Option<PathBuf>, StoreError> {
    if plan.current_version > plan.target_version {
        return Err(StoreError::IncompatibleSchema);
    }
    if plan.steps.is_empty() {
        if plan.current_version == plan.target_version {
            return Ok(None);
        }
        return Err(StoreError::Invariant(
            "forward migration plan has no steps for version upgrade".into(),
        ));
    }

    let mut expected = plan.current_version;
    for step in &plan.steps {
        let next = expected
            .checked_add(1)
            .ok_or(StoreError::IncompatibleSchema)?;
        if step.from_version != expected || step.to_version != next {
            return Err(StoreError::Invariant(format!(
                "non-contiguous forward migration step '{}' (expected v{}->v{}, got v{}->v{})",
                step.name, expected, next, step.from_version, step.to_version
            )));
        }
        expected = next;
    }
    if expected != plan.target_version {
        return Err(StoreError::Invariant(format!(
            "incomplete forward migration plan (ended at v{}, target v{})",
            expected, plan.target_version
        )));
    }
    if !conn.is_autocommit() {
        return Err(StoreError::Invariant(
            "forward migration requires an idle autocommit connection".into(),
        ));
    }

    let current_db_version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if current_db_version != i64::from(plan.current_version) {
        return Err(StoreError::IncompatibleSchema);
    }

    let backup_path = match backup_source {
        Some(source) => Some(create_pre_migration_backup(
            conn,
            source,
            plan.current_version,
            plan.target_version,
        )?),
        None => None,
    };

    conn.execute_batch("BEGIN IMMEDIATE")?;
    let apply_result: StoreResult<()> = (|| {
        let active_version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if active_version != i64::from(plan.current_version) {
            return Err(StoreError::IncompatibleSchema);
        }
        for step in &plan.steps {
            conn.execute_batch(step.sql)?;
            conn.pragma_update(None, "user_version", i64::from(step.to_version))?;
        }
        Ok(())
    })();
    if let Err(error) = apply_result {
        let _ = conn.execute_batch("ROLLBACK");
        return Err(error);
    }
    if let Err(error) = conn.execute_batch("COMMIT") {
        let _ = conn.execute_batch("ROLLBACK");
        return Err(error.into());
    }
    Ok(backup_path)
}

fn maybe_apply_registered_forward_migrations(connection: &mut Connection) -> StoreResult<()> {
    let application_id: i64 =
        connection.query_row("PRAGMA application_id", [], |row| row.get(0))?;
    let version: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if application_id == STORE_APPLICATION_ID && version > 0 && version < STORE_SCHEMA_VERSION {
        let current_version =
            u32::try_from(version).map_err(|_| StoreError::IncompatibleSchema)?;
        let target_version =
            u32::try_from(STORE_SCHEMA_VERSION).map_err(|_| StoreError::IncompatibleSchema)?;
        let plan =
            plan_forward_migrations(current_version, target_version, REGISTERED_FORWARD_MIGRATIONS)?;
        let backup_source = connection
            .path()
            .filter(|path| !path.is_empty() && *path != ":memory:")
            .map(PathBuf::from);
        let _ = apply_forward_migrations(connection, &plan, backup_source.as_deref())?;
    }
    Ok(())
}

/// Returns true only for an empty, unclaimed database. Existing databases must
/// already have this build's exact identity; rejection never repairs or writes.
fn requires_initialization(connection: &Connection) -> StoreResult<bool> {
    let application_id: i64 =
        connection.query_row("PRAGMA application_id", [], |row| row.get(0))?;
    let version: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if application_id == STORE_APPLICATION_ID && version == STORE_SCHEMA_VERSION {
        let epoch: Option<String> = connection
            .query_row(
                "SELECT value FROM store_metadata WHERE key='schema_epoch'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        if epoch.as_deref() != Some(STORE_SCHEMA_EPOCH.to_string().as_str()) {
            return Err(StoreError::IncompatibleSchema);
        }
        let has_legacy_skill_digest: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('skill_versions') WHERE name='instructions_digest')",
            [],
            |row| row.get(0),
        )?;
        return if has_legacy_skill_digest {
            Err(StoreError::IncompatibleSchema)
        } else {
            Ok(false)
        };
    }
    if application_id != 0 || version != 0 {
        return Err(StoreError::IncompatibleSchema);
    }
    let objects: i64 = connection.query_row(
        "SELECT COUNT(*) FROM sqlite_master
         WHERE name NOT GLOB 'sqlite_*' AND type IN ('table','index','trigger','view')",
        [],
        |row| row.get(0),
    )?;
    if objects != 0 {
        return Err(StoreError::IncompatibleSchema);
    }
    Ok(true)
}

pub(super) fn verify_read_only_schema(connection: &Connection) -> StoreResult<()> {
    if requires_initialization(connection)? {
        return Err(StoreError::Invariant(
            "canonical Store is not initialized".to_string(),
        ));
    }
    Ok(())
}

impl NolaStore {
    pub(super) fn initialize_schema(&mut self) -> StoreResult<()> {
        maybe_apply_registered_forward_migrations(&mut self.connection)?;
        if !requires_initialization(&self.connection)? {
            return Ok(());
        }
        let transaction = self.write_transaction()?;
        // Another opener may have completed initialization while we waited for
        // the write lock. Never create against an unchecked or partial schema.
        if requires_initialization(&transaction)? {
            create_current_schema(&transaction)?;
            transaction.pragma_update(None, "application_id", STORE_APPLICATION_ID)?;
            transaction.pragma_update(None, "user_version", STORE_SCHEMA_VERSION)?;
        }
        transaction.commit()
    }
}

/// Compose the unique per-feature DDL sources. No copied schema snapshot,
/// historical rows, inferred provenance, or version-dependent ALTER operations.
fn create_current_schema(connection: &Connection) -> StoreResult<()> {
    for ddl in [
        include_str!("schema.sql"),
        include_str!("generative_ui_schema.sql"),
        include_str!("prepared_host_tool_plan_schema.sql"),
        include_str!("plugin_schema.sql"),
        include_str!("agent_task_admission_schema.sql"),
        include_str!("canonical_text_disclosure_schema.sql"),
        include_str!("delivery_checkpoint_witness_projection.sql"),
        include_str!("delivery_checkpoint_witness_schema.sql"),
        include_str!("host_replay_order_schema.sql"),
        include_str!("checkpoint_replay_base_schema.sql"),
        include_str!("remote_assistant_schema.sql"),
        include_str!("assistant_journal_schema.sql"),
        include_str!("assistant_run_terminal_schema.sql"),
        include_str!("run_message_admission_schema.sql"),
        include_str!("committed_run_context_usage_schema.sql"),
        include_str!("remote_submission_schema.sql"),
        include_str!("assistant_journal_snapshot_lifecycle_schema.sql"),
        include_str!("assistant_journal_range_schema.sql"),
        include_str!("canonical_stream_schema.sql"),
        include_str!("canonical_message_owner_schema.sql"),
        include_str!("assistant_journal_publication_schema.sql"),
        include_str!("assistant_journal_planning_schema.sql"),
        include_str!("runtime_task_run_binding_schema.sql"),
        include_str!("runtime_task_guidance_index.sql"),
        include_str!("remote_assistant_journal_schema.sql"),
        include_str!("input_fact_schema.sql"),
        include_str!("user_input_wait_schema.sql"),
        include_str!("execution_evidence_schema.sql"),
        include_str!("run_requirement_schema.sql"),
        include_str!("run_input_frontier.sql"),
        include_str!("memory_index_schema.sql"),
    ] {
        connection.execute_batch(ddl)?;
    }
    crate::assistant_journal::order::initialize(connection)?;
    crate::work_archive_worker::initialize_schema(connection)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn existing_user_objects_with_sqlite_prefix_are_not_an_empty_database() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("foreign.db");
        let db = Connection::open(&path).unwrap();
        db.execute_batch("CREATE TABLE sqliteBackup(note TEXT); INSERT INTO sqliteBackup VALUES('keep');").unwrap();
        drop(db);
        let before = std::fs::read(&path).unwrap();
        assert!(NolaStore::open(&path).unwrap_err().is_schema_incompatible());
        assert_eq!(std::fs::read(path).unwrap(), before);
    }

    #[test]
    fn freshly_created_schema_has_the_exact_runtime_epoch_and_reopens() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("fresh.sqlite");
        let store = NolaStore::open(&path).unwrap();
        let epoch: String = store.connection.query_row(
            "SELECT value FROM store_metadata WHERE key='schema_epoch'", [], |row| row.get(0)).unwrap();
        assert_eq!(epoch, STORE_SCHEMA_EPOCH.to_string());
        assert!(!requires_initialization(&store.connection).unwrap());
        drop(store);
        let reopened = NolaStore::open(&path).unwrap();
        assert!(!requires_initialization(&reopened.connection).unwrap());
    }

    #[test]
    fn current_schema_installs_final_trigger_definitions_once() {
        let connection = Connection::open_in_memory().unwrap();
        create_current_schema(&connection).unwrap();
        for name in [
            "immutable_runtime_task_identity",
            "core_delivery_witness_insert",
            "core_delivery_witness_update",
            "checkpoint_replay_base_invalidate",
            "checkpoint_body_delete",
            "host_event_replay_order_insert",
            "checkpoint_replay_base_commit",
        ] {
            let count: i64 = connection.query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='trigger' AND name=?1",
                [name], |row| row.get(0),
            ).unwrap();
            assert_eq!(count, 1, "{name}");
        }
        for name in ["core_delivery_witness_update", "checkpoint_replay_base_invalidate"] {
            let sql: String = connection.query_row(
                "SELECT sql FROM sqlite_master WHERE type='trigger' AND name=?1",
                [name], |row| row.get(0),
            ).unwrap();
            assert!(sql.contains("UPDATE OF checkpoint_json ON core_checkpoint_bodies"), "{sql}");
        }
    }

    #[test]
    fn current_schema_initializes_and_reopens_without_recreation() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("current.sqlite");
        let store = NolaStore::open_archive_mode(&path, false).unwrap();
        store
            .connection
            .execute(
                "INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms) VALUES('owner','preserved',1,1,1)",
                [],
            )
            .unwrap();
        let schema: String = store
            .connection
            .query_row(
                "SELECT group_concat(sql, char(10)) FROM
             (SELECT sql FROM sqlite_master WHERE sql IS NOT NULL ORDER BY type,name)",
                [],
                |row| row.get(0),
            )
            .unwrap();
        drop(store);
        let reopened = NolaStore::open_archive_mode(&path, false).unwrap();
        let current: String = reopened
            .connection
            .query_row(
                "SELECT group_concat(sql, char(10)) FROM
             (SELECT sql FROM sqlite_master WHERE sql IS NOT NULL ORDER BY type,name)",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(schema, current);
        let title: String = reopened
            .connection
            .query_row(
                "SELECT title FROM conversations WHERE id='owner'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(title, "preserved");
        for table in [
            "run_input_applications",
            "input_admissions",
            "user_input_wait_heads",
            "execution_evidence_sources",
            "work_archive_publication",
            "assistant_journal_order",
        ] {
            let count: i64 = reopened
                .connection
                .query_row(
                    "SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?1",
                    [table],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 1, "{table}");
        }
    }

    #[test]
    fn schema_ddl_failure_rolls_back_all_objects_and_identity() {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch("BEGIN IMMEDIATE").unwrap();
        create_current_schema(&connection).unwrap();
        connection
            .pragma_update(None, "application_id", STORE_APPLICATION_ID)
            .unwrap();
        connection
            .pragma_update(None, "user_version", STORE_SCHEMA_VERSION)
            .unwrap();
        // An error after the entire schema must not leave a recognizable partial store.
        assert!(connection
            .execute_batch("CREATE TABLE conversations(id TEXT)")
            .is_err());
        connection.execute_batch("ROLLBACK").unwrap();
        assert!(requires_initialization(&connection).unwrap());
    }

    #[test]
    fn forward_migration_applies_contiguous_v1_to_v3_chain_with_backup() {
        static REGISTRY: &[ForwardMigrationStep] = &[
            ForwardMigrationStep {
                from_version: 1,
                to_version: 2,
                name: "add_items_notes_column",
                sql: "ALTER TABLE items ADD COLUMN notes TEXT NOT NULL DEFAULT '';
                      UPDATE items SET notes = 'migrated-v2' WHERE id = 1;",
            },
            ForwardMigrationStep {
                from_version: 2,
                to_version: 3,
                name: "add_audit_table",
                sql: "CREATE TABLE item_audit (id INTEGER PRIMARY KEY, item_id INTEGER NOT NULL, tag TEXT NOT NULL);
                      INSERT INTO item_audit (id, item_id, tag) VALUES (1, 1, 'v3-ready');",
            },
        ];

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("upgrade.sqlite");
        let mut conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE items (id INTEGER PRIMARY KEY, label TEXT NOT NULL);
             INSERT INTO items (id, label) VALUES (1, 'baseline');
             PRAGMA user_version = 1;",
        )
        .unwrap();

        let plan = plan_forward_migrations(1, 3, REGISTRY).unwrap();
        assert_eq!(plan.current_version, 1);
        assert_eq!(plan.target_version, 3);
        assert_eq!(plan.steps.len(), 2);
        assert_eq!(plan.steps[0].name, "add_items_notes_column");
        assert_eq!(plan.steps[1].name, "add_audit_table");

        let backup = apply_forward_migrations(&mut conn, &plan, Some(&path))
            .unwrap()
            .expect("backup path should be returned when backup_source is provided");
        assert!(backup.exists(), "backup file must exist on disk");
        let backup_name = backup.file_name().unwrap().to_string_lossy();
        assert!(
            backup_name.ends_with(".pre-migrate-v1-to-v3.bak"),
            "unexpected backup filename: {backup_name}"
        );

        let migrated_version: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(migrated_version, 3);
        let note: String = conn
            .query_row("SELECT notes FROM items WHERE id = 1", [], |row| row.get(0))
            .unwrap();
        assert_eq!(note, "migrated-v2");
        let tag: String = conn
            .query_row("SELECT tag FROM item_audit WHERE id = 1", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(tag, "v3-ready");

        // Backup must preserve the exact pre-migration v1 state.
        let backup_conn = Connection::open(&backup).unwrap();
        let backup_version: i64 = backup_conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(backup_version, 1);
        let audit_exists: bool = backup_conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='item_audit')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(!audit_exists);

        // No-op plan when already at target version does not create a backup or mutate DB.
        let noop_plan = plan_forward_migrations(3, 3, REGISTRY).unwrap();
        assert!(noop_plan.steps.is_empty());
        assert!(apply_forward_migrations(&mut conn, &noop_plan, Some(&path))
            .unwrap()
            .is_none());
    }

    #[test]
    fn forward_migration_rolls_back_atomically_on_broken_sql_step() {
        static BROKEN_REGISTRY: &[ForwardMigrationStep] = &[
            ForwardMigrationStep {
                from_version: 1,
                to_version: 2,
                name: "create_v2_table",
                sql: "CREATE TABLE v2_entries (id INTEGER PRIMARY KEY, body TEXT NOT NULL);
                      INSERT INTO v2_entries (id, body) VALUES (1, 'partial');",
            },
            ForwardMigrationStep {
                from_version: 2,
                to_version: 3,
                name: "broken_v3_step",
                sql: "CREATE TABLE broken_syntax (id INTEGER NOT NULL; INVALID SQL",
            },
        ];

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rollback.sqlite");
        let mut conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE items (id INTEGER PRIMARY KEY, label TEXT NOT NULL);
             INSERT INTO items (id, label) VALUES (1, 'untouched');
             PRAGMA user_version = 1;",
        )
        .unwrap();

        let plan = plan_forward_migrations(1, 3, BROKEN_REGISTRY).unwrap();
        let error = apply_forward_migrations(&mut conn, &plan, Some(&path)).unwrap_err();
        assert!(matches!(error, StoreError::Sqlite(_)));
        assert!(
            conn.is_autocommit(),
            "connection must return to autocommit after rollback"
        );

        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, 1, "failed migration must roll back user_version");
        let v2_exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='v2_entries')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(!v2_exists, "step 1 DDL must roll back when step 2 fails");
        let label: String = conn
            .query_row("SELECT label FROM items WHERE id = 1", [], |row| row.get(0))
            .unwrap();
        assert_eq!(label, "untouched");
    }

    #[test]
    fn forward_migration_plan_rejects_downgrade_and_missing_step() {
        static PARTIAL_REGISTRY: &[ForwardMigrationStep] = &[
            ForwardMigrationStep {
                from_version: 1,
                to_version: 2,
                name: "only_v1_to_v2",
                sql: "SELECT 1;",
            },
            ForwardMigrationStep {
                from_version: 1,
                to_version: 3,
                name: "non_contiguous_jump",
                sql: "SELECT 1;",
            },
        ];

        assert!(matches!(
            plan_forward_migrations(3, 2, PARTIAL_REGISTRY),
            Err(StoreError::IncompatibleSchema)
        ));
        assert!(matches!(
            plan_forward_migrations(1, 3, PARTIAL_REGISTRY),
            Err(StoreError::IncompatibleSchema)
        ));
        assert!(matches!(
            plan_forward_migrations(1, 2, REGISTERED_FORWARD_MIGRATIONS),
            Err(StoreError::IncompatibleSchema)
        ));
        let baseline_noop =
            plan_forward_migrations(1, 1, REGISTERED_FORWARD_MIGRATIONS).unwrap();
        assert!(baseline_noop.steps.is_empty());
    }
}
