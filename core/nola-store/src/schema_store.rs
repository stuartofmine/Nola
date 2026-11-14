// SPDX-License-Identifier: Apache-2.0
//! Current-schema gate and atomic first-run initialization. No upgrades.

use super::*;

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
        return if epoch.as_deref() == Some(STORE_SCHEMA_EPOCH.to_string().as_str()) {
            Ok(false)
        } else {
            Err(StoreError::IncompatibleSchema)
        };
    }
    if application_id != 0 || version != 0 {
        return Err(StoreError::IncompatibleSchema);
    }
    let objects: i64 = connection.query_row(
        "SELECT COUNT(*) FROM sqlite_master
         WHERE name NOT LIKE 'sqlite_%' AND type IN ('table','index','trigger','view')",
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
}
