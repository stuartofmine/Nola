// SPDX-License-Identifier: Apache-2.0
//! SQLite connection ownership and transaction primitives for the canonical Store.

use std::fmt::{Display, Formatter};
use std::ops::Deref;
use std::path::Path;
use std::time::Duration;

use rusqlite::Connection;

use super::STORE_SCHEMA_VERSION;

#[derive(Debug)]
pub enum StoreError {
    Sqlite(rusqlite::Error),
    Json(serde_json::Error),
    Conflict(String),
    Invariant(String),
    IncompatibleSchema,
}

#[cfg(test)]
mod settlement_fence_tests {
    use super::*;

    #[test]
    fn reopening_transport_housekeeping_does_not_publish_unrelated_dirty_content() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("dirty-content.sqlite");
        let store = NolaStore::open_archive_mode(&path, false).unwrap();
        store.connection.execute_batch("INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms)
            VALUES('c','fixture',1,1,1);
            INSERT INTO messages(id,conversation_id,role,status,ordinal,created_at_ms)
            VALUES('m','c','assistant','completed',1,1);
            INSERT INTO message_parts(id,message_id,kind,ordinal,state,content_text,content_json,created_at_ms,updated_at_ms)
            VALUES('p','m','provider_message',1,'completed','', '{\"role\":\"assistant\",\"content\":\"source\"}',1,1);").unwrap();
        let pending = |store: &NolaStore| -> i64 {
            store.connection.query_row("SELECT count(*) FROM canonical_part_disclosures WHERE evaluated_generation IS NULL", [], |row| row.get(0)).unwrap()
        };
        assert_eq!(pending(&store), 1);
        drop(store);
        let reopened = NolaStore::open(&path).unwrap();
        assert_eq!(pending(&reopened), 1, "opening transport metadata cannot publish source content");
        let classifications: i64 = reopened.connection.query_row("SELECT count(*) FROM canonical_provider_classifications", [], |row| row.get(0)).unwrap();
        assert_eq!(classifications, 0);
    }

    #[test]
    fn rolled_back_host_measurements_do_not_escape_the_unit_of_work() {
        let mut store=NolaStore::open(":memory:").unwrap();
        store.begin_write_unit_of_work().unwrap();
        store.begin_command_mutations().unwrap();
        store.pending_measurements.push(nola_measurement::Context::default());
        store.rollback_command_mutations().unwrap();
        assert!(store.pending_measurements.is_empty());
        store.pending_measurements.push(nola_measurement::Context::default());
        store.rollback_write_unit_of_work().unwrap();
        assert!(store.pending_measurements.is_empty());
    }

    #[test]
    fn repository_rollback_preserves_only_parent_measurements() {
        let mut store = NolaStore::open(":memory:").unwrap();
        store.begin_write_unit_of_work().unwrap();
        store.pending_measurements.push(nola_measurement::Context {
            parent_span: Some(10), ..Default::default()
        });
        for explicit in [true, false] {
            let mut transaction = store.write_transaction().unwrap();
            transaction.with_store(|store| {
                store.pending_measurements.push(nola_measurement::Context {
                    parent_span: Some(20), ..Default::default()
                });
                Ok(())
            }).unwrap();
            if explicit { transaction.rollback().unwrap(); } else { drop(transaction); }
            assert_eq!(store.pending_measurements.len(), 1);
            assert_eq!(store.pending_measurements[0].parent_span, Some(10));
        }
        store.rollback_write_unit_of_work().unwrap();
        assert!(store.pending_measurements.is_empty());
    }

    #[test]
    fn failed_commit_fences_direct_sql_even_after_successful_cleanup() {
        let mut store = NolaStore::open(":memory:").unwrap();
        store.connection.execute_batch(
            "CREATE TABLE fixture_parent(id INTEGER PRIMARY KEY);
             CREATE TABLE fixture_child(id INTEGER REFERENCES fixture_parent(id)
                 DEFERRABLE INITIALLY DEFERRED);"
        ).unwrap();
        store.begin_write_unit_of_work().unwrap();
        store.connection.execute("INSERT INTO fixture_child VALUES(1)", []).unwrap();
        store.applied_transition_stamps.insert("uncommitted".into());
        assert!(store.commit_write_unit_of_work().is_err());
        assert!(store.applied_transition_stamps.is_empty());
        assert!(store.has_write_unit_of_work());
        assert!(store.connection.execute("INSERT INTO fixture_parent VALUES(1)", []).is_err());
        store.rollback_write_unit_of_work().unwrap();
        store.rollback_write_unit_of_work().unwrap();
        assert!(!store.has_write_unit_of_work());
        assert!(store.begin_write_unit_of_work().unwrap_err().to_string().contains("reopen required"));
        assert!(store.connection.execute("INSERT INTO fixture_parent VALUES(1)", []).is_err());
        let count: i64 = store.connection.query_row("SELECT count(*) FROM fixture_child", [], |row| row.get(0)).unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn lost_mutation_savepoint_fences_writes_but_allows_outer_cleanup() {
        let mut store = NolaStore::open(":memory:").unwrap();
        store.begin_write_unit_of_work().unwrap();
        store.begin_command_mutations().unwrap();
        store.connection.execute_batch("RELEASE SAVEPOINT nola_command_mutations").unwrap();
        assert!(store.rollback_command_mutations().is_err());
        assert!(store.begin_command_mutations().is_err());
        assert!(store.commit_write_unit_of_work().is_err());
        store.rollback_write_unit_of_work().unwrap();
        assert!(store.put_product_setting("fixture", "true", 1).is_err());
        assert!(store.product_setting("fixture").unwrap().is_none());
    }

    #[test]
    fn repository_commit_failure_requires_reopen_even_when_rollback_succeeds() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settlement.sqlite");
        let mut store = NolaStore::open(&path).unwrap();
        store.connection.execute_batch(
            "CREATE TABLE fixture_parent(id INTEGER PRIMARY KEY);
             CREATE TABLE fixture_child(id INTEGER REFERENCES fixture_parent(id)
                 DEFERRABLE INITIALLY DEFERRED);"
        ).unwrap();
        let tx = store.write_transaction().unwrap();
        tx.execute("INSERT INTO fixture_child VALUES(1)", []).unwrap();
        assert!(tx.commit().is_err());
        assert!(!store.has_write_unit_of_work());
        assert!(store.begin_write_unit_of_work().unwrap_err().to_string().contains("reopen required"));
        assert!(store.connection.execute("INSERT INTO fixture_parent VALUES(1)", []).is_err());
        drop(store);
        let mut reopened = NolaStore::open(&path).unwrap();
        let count: i64 = reopened.connection.query_row("SELECT count(*) FROM fixture_child", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 0);
        reopened.put_product_setting("after_reopen", "true", 1).unwrap();
    }

    #[test]
    fn repository_savepoint_settlement_failure_fences_commit_rollback_and_drop() {
        for settlement in ["commit", "rollback", "drop"] {
            let mut store = NolaStore::open(":memory:").unwrap();
            store.begin_write_unit_of_work().unwrap();
            store.put_product_setting("uncommitted", "true", 1).unwrap();
            store.applied_transition_stamps.insert("uncommitted".into());
            let tx = store.write_transaction().unwrap();
            // Simulate a nested participant losing the repository's scope.
            tx.execute_batch("RELEASE SAVEPOINT nola_repository_write").unwrap();
            match settlement {
                "commit" => assert!(tx.commit().is_err()),
                "rollback" => assert!(tx.rollback().is_err()),
                _ => drop(tx),
            }
            assert!(store.applied_transition_stamps.is_empty(), "{settlement}");
            assert!(store.commit_write_unit_of_work().is_err(), "{settlement}");
            store.rollback_write_unit_of_work().unwrap();
            assert!(store.product_setting("uncommitted").unwrap().is_none());
            assert!(store.put_product_setting("after_failure", "true", 2).is_err());
        }
    }

    #[test]
    fn ordinary_repository_rollback_preserves_the_enclosing_transaction() {
        let mut store = NolaStore::open(":memory:").unwrap();
        store.begin_write_unit_of_work().unwrap();
        store.put_product_setting("outer", "true", 1).unwrap();
        for explicit in [true, false] {
            let tx = store.write_transaction().unwrap();
            tx.execute("DELETE FROM product_settings WHERE key='outer'", []).unwrap();
            if explicit { tx.rollback().unwrap(); } else { drop(tx); }
            assert!(store.product_setting("outer").unwrap().is_some());
        }
        store.commit_write_unit_of_work().unwrap();
        assert!(store.product_setting("outer").unwrap().is_some());
        store.put_product_setting("after_commit", "true", 2).unwrap();
    }
}

impl StoreError {
    pub fn is_schema_incompatible(&self) -> bool {
        matches!(self, Self::IncompatibleSchema)
    }
}

impl Display for StoreError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sqlite(error) => write!(formatter, "SQLite error: {error}"),
            Self::Json(error) => write!(formatter, "JSON error: {error}"),
            Self::Conflict(message) => write!(formatter, "Store conflict: {message}"),
            Self::Invariant(message) => write!(formatter, "Store invariant failed: {message}"),
            Self::IncompatibleSchema => write!(
                formatter,
                "Unsupported database format. The database has been preserved; open it with a matching Nola version or use a documented upgrade path. No automatic reset is performed."
            ),
        }
    }
}

impl std::error::Error for StoreError {}

impl From<rusqlite::Error> for StoreError {
    fn from(value: rusqlite::Error) -> Self {
        Self::Sqlite(value)
    }
}

impl From<serde_json::Error> for StoreError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}

pub type StoreResult<T> = Result<T, StoreError>;

#[derive(Debug)]
pub struct NolaStore {
    pub(super) connection: Connection,
    pub(super) read_checkpoint_cache: std::cell::RefCell<Option<(i64, u64, nola_store_port::run_effect::StoredCheckpoint)>>,
    pub(super) archive_worker: Option<std::sync::Arc<crate::work_archive_worker::Worker>>,
    read_only: bool,
    /// A failed settlement permanently fences this connection until reopen.
    write_poison: Option<String>,
    /// Private evidence of transitions actually applied in the current outer UoW.
    pub(super) applied_ui_surface_stamps: std::collections::HashSet<String>,
    pub(super) applied_transition_stamps: std::collections::HashSet<String>,
    pub(super) applied_admission_stamps: std::collections::HashMap<String, i64>,
    pub(super) applied_idle_checkpoint_stamps: std::collections::HashMap<(String, i64), i64>,
    pending_measurements: Vec<nola_measurement::Context>,
}

/// Owns settlement as well as the SQL scope. A library transaction whose Drop
/// silently discards rollback errors cannot enforce the Store's reopen rule.
pub(super) struct WriteTransaction<'connection> {
    store: &'connection mut NolaStore,
    nested: bool,
    settled: bool,
    measurement_frontier: usize,
    ui_surface_stamps: std::collections::HashSet<String>,
}

impl Deref for WriteTransaction<'_> {
    type Target = Connection;

    fn deref(&self) -> &Self::Target {
        &self.store.connection
    }
}

impl WriteTransaction<'_> {
    /// Run repository helpers inside this owned settlement guard. The borrow
    /// cannot escape the closure; Drop still fences/rolls back every error path.
    pub(super) fn with_store<T>(&mut self, operation:impl FnOnce(&mut NolaStore)->StoreResult<T>)->StoreResult<T> {
        operation(self.store)
    }
    pub(super) fn commit(mut self) -> StoreResult<()> {
        let measurement = (!self.nested).then(||nola_measurement::Span::start(nola_measurement::Stage::StoreCommit));
        self.store.ensure_writable()?;
        crate::canonical_text_disclosure::flush(&self.store.connection)?;
        let sql = if self.nested { "RELEASE SAVEPOINT nola_repository_write" } else { "COMMIT" };
        if let Err(error) = self.store.connection.execute_batch(sql) {
            if let Some(measurement)=measurement {measurement.finish(nola_measurement::Outcome::Failed);}
            self.store.poison_writes(&error)?;
            // Drop still attempts cleanup, without clearing the poison.
            return Err(error.into());
        }
        self.settled = true;
        if let Some(measurement)=measurement {measurement.finish(nola_measurement::Outcome::Succeeded);}
        if !self.nested { self.store.notify_work_archive_worker(); }
        Ok(())
    }

    pub(super) fn rollback(mut self) -> StoreResult<()> {
        self.rollback_scope()
    }

    fn rollback_scope(&mut self) -> StoreResult<()> {
        self.settled = true;
        // Diagnostic observations follow the same savepoint boundary as the
        // mutations. Preserve the parent's observations, discard this scope's.
        self.store.pending_measurements.truncate(self.measurement_frontier);
        self.store.applied_ui_surface_stamps = self.ui_surface_stamps.clone();
        let sql = if self.nested {
            "ROLLBACK TO SAVEPOINT nola_repository_write; RELEASE SAVEPOINT nola_repository_write"
        } else {
            "ROLLBACK"
        };
        if let Err(error) = self.store.connection.execute_batch(sql) {
            self.store.poison_writes(&error)?;
            return Err(error.into());
        }
        Ok(())
    }
}

impl Drop for WriteTransaction<'_> {
    fn drop(&mut self) {
        if !self.settled {
            // Early returns and unwinding must fence failed cleanup too. The
            // caller may not receive this error, but cannot keep writing.
            let _ = self.rollback_scope();
        }
    }
}

impl NolaStore {
    pub fn open(path: impl AsRef<Path>) -> StoreResult<Self> {
        Self::open_archive_mode(path.as_ref(), true)
    }

    /// Opens a read-only WAL snapshot handle without schema DDL, pragma writes,
    /// housekeeping mutations, or background archive workers.
    pub fn open_read_only(path: impl AsRef<Path>) -> StoreResult<Self> {
        let path = path.as_ref();
        if path == Path::new(":memory:") {
            return Err(StoreError::Invariant(
                "read-only Store snapshot requires a file-backed WAL database".to_string(),
            ));
        }
        let connection = Connection::open_with_flags(
            path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
                | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        connection.busy_timeout(Duration::from_secs(5))?;
        crate::runtime_task_claim_store::register_workspace_admission(&connection)?;
        connection.pragma_update(None, "query_only", true)?;
        crate::schema_store::verify_read_only_schema(&connection)?;
        Ok(Self {
            connection,
            read_checkpoint_cache: Default::default(),
            archive_worker: None,
            read_only: true,
            write_poison: None,
            applied_ui_surface_stamps: Default::default(),
            applied_transition_stamps: Default::default(),
            applied_admission_stamps: Default::default(),
            applied_idle_checkpoint_stamps: Default::default(),
            pending_measurements: Vec::new(),
        })
    }

    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    pub(super) fn open_archive_mode(path: &Path, background: bool) -> StoreResult<Self> {
        let connection = Connection::open(path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        crate::runtime_task_claim_store::register_workspace_admission(&connection)?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        let mut store = Self { connection, read_checkpoint_cache: Default::default(), archive_worker:None, read_only: false, write_poison: None, applied_ui_surface_stamps: Default::default(), applied_transition_stamps: Default::default(), applied_admission_stamps: Default::default(), applied_idle_checkpoint_stamps: Default::default(), pending_measurements: Vec::new() };
        // Reject foreign or incompatible databases before changing persistent
        // journal settings. A failed open must preserve their original bytes.
        store.initialize_schema()?;
        store
            .connection
            .pragma_update(None, "journal_mode", "WAL")?;
        store
            .connection
            .pragma_update(None, "synchronous", "NORMAL")?;
        if background {store.prune_settled_outbox_rows()?;}
        if background && path != Path::new(":memory:") {
            store.archive_worker=Some(crate::work_archive_worker::register(path)?);
            store.notify_work_archive_worker();
        }
        Ok(store)
    }

    /// Starts the outer transaction owned by one Application command.
    pub fn has_write_unit_of_work(&self) -> bool {
        !self.connection.is_autocommit()
    }

    /// Starts the outer transaction owned by one Application command.
    /// Repository methods called inside this boundary use nested savepoints.
    pub fn begin_write_unit_of_work(&mut self) -> StoreResult<()> {
        self.ensure_writable()?;
        if !self.connection.is_autocommit() {
            return Err(StoreError::Invariant(
                "a write unit of work is already active".to_string(),
            ));
        }
        self.connection.execute_batch("BEGIN IMMEDIATE")?;
        self.pending_measurements.clear();
        self.applied_ui_surface_stamps.clear();
        self.applied_transition_stamps.clear();
        self.applied_admission_stamps.clear();
        self.applied_idle_checkpoint_stamps.clear();
        Ok(())
    }

    pub fn commit_write_unit_of_work(&mut self) -> StoreResult<()> {
        let measurement = nola_measurement::Span::start(nola_measurement::Stage::StoreCommit);
        self.ensure_writable()?;
        if self.connection.is_autocommit() {
            return Err(StoreError::Invariant(
                "no write unit of work is active".to_string(),
            ));
        }
        use crate::commit_diagnostics::{start, Outcome, Stage};
        for transition in &self.applied_transition_stamps {
            let unsealed: bool = self.connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM committed_transitions t JOIN runs r ON r.id=t.run_id
                 WHERE t.transition_id=?1 AND r.kind='conversation' AND r.status='sealed'
                 AND NOT EXISTS(SELECT 1 FROM assistant_run_terminal_seals s
                     WHERE s.run_id=r.id))",
                [transition], |row| row.get(0))?;
            if unsealed {
                return Err(StoreError::Invariant("terminal Run requires its result seal in the same unit of work".into()));
            }
        }
        let timer=start(Stage::DisclosureFlush);
        let disclosure=crate::canonical_text_disclosure::flush(&self.connection);
        timer.finish(if disclosure.is_ok(){Outcome::Succeeded}else{Outcome::Failed});
        disclosure?;
        let timer=start(Stage::SqlCommit);
        let commit=self.connection.execute_batch("COMMIT");
        timer.finish(if commit.is_ok(){Outcome::Succeeded}else{Outcome::Failed});
        let outcome = if commit.is_ok() { nola_measurement::Outcome::Succeeded }
            else { nola_measurement::Outcome::Failed };
        for mut context in self.pending_measurements.drain(..) {
            context.parent_span = measurement.child_context().parent_span;
            let _scope = nola_measurement::enter(context);
            nola_measurement::point_outcome(nola_measurement::Stage::StoreCommittedHostResponse,outcome);
        }
        measurement.finish(outcome);
        if let Err(error) = commit {
            self.poison_writes(&error)?;
            return Err(error.into());
        }
        self.applied_ui_surface_stamps.clear();
        self.applied_transition_stamps.clear();
        self.applied_admission_stamps.clear();
        self.applied_idle_checkpoint_stamps.clear();
        self.notify_work_archive_worker();
        Ok(())
    }

    /// Isolates command mutations from the admission receipt in the outer unit of work.
    pub fn begin_command_mutations(&mut self) -> StoreResult<()> {
        self.ensure_writable()?;
        if self.connection.is_autocommit() {
            return Err(StoreError::Invariant(
                "command mutations require a write unit of work".into(),
            ));
        }
        self.connection
            .execute_batch("SAVEPOINT nola_command_mutations")?;
        Ok(())
    }

    pub fn commit_command_mutations(&mut self) -> StoreResult<()> {
        self.ensure_writable()?;
        crate::canonical_text_disclosure::flush(&self.connection)?;
        if let Err(error) = self.connection.execute_batch("RELEASE SAVEPOINT nola_command_mutations") {
            self.poison_writes(&error)?;
            return Err(error.into());
        }
        Ok(())
    }

    pub fn rollback_command_mutations(&mut self) -> StoreResult<()> {
        self.pending_measurements.clear();
        if let Err(error) = self.connection.execute_batch(
            "ROLLBACK TO SAVEPOINT nola_command_mutations; RELEASE SAVEPOINT nola_command_mutations",
        ) {
            self.poison_writes(&error)?;
            return Err(error.into());
        }
        self.applied_ui_surface_stamps.clear();
        self.applied_transition_stamps.clear();
        self.applied_admission_stamps.clear();
        self.applied_idle_checkpoint_stamps.clear();
        Ok(())
    }

    pub fn rollback_write_unit_of_work(&mut self) -> StoreResult<()> {
        self.pending_measurements.clear();
        if !self.connection.is_autocommit() {
            if let Err(error) = self.connection.execute_batch("ROLLBACK") {
                self.poison_writes(&error)?;
                return Err(error.into());
            }
        }
        self.applied_ui_surface_stamps.clear();
        self.applied_transition_stamps.clear();
        self.applied_admission_stamps.clear();
        self.applied_idle_checkpoint_stamps.clear();
        Ok(())
    }

    pub(super) fn write_transaction(&mut self) -> StoreResult<WriteTransaction<'_>> {
        self.ensure_writable()?;
        let nested = !self.connection.is_autocommit();
        self.connection.execute_batch(if nested {
            "SAVEPOINT nola_repository_write"
        } else { "BEGIN IMMEDIATE" })?;
        // The exclusive Store borrow prevents a second repository scope from
        // being opened until this one has settled, so the savepoint is unique.
        let measurement_frontier = self.pending_measurements.len();
        let ui_surface_stamps = self.applied_ui_surface_stamps.clone();
        Ok(WriteTransaction { store: self, nested, settled: false, measurement_frontier, ui_surface_stamps })
    }

    fn ensure_writable(&self) -> StoreResult<()> {
        if self.read_only {
            return Err(StoreError::Invariant(
                "read-only Store snapshot cannot start a write transaction".to_string(),
            ));
        }
        match &self.write_poison {
            Some(reason) => Err(StoreError::Invariant(format!(
                "Store writes are poisoned; reopen required: {reason}"
            ))),
            None => Ok(()),
        }
    }

    fn poison_writes(&mut self, error: &rusqlite::Error) -> StoreResult<()> {
        self.pending_measurements.clear();
        self.write_poison.get_or_insert_with(|| error.to_string());
        self.applied_ui_surface_stamps.clear();
        self.applied_transition_stamps.clear();
        self.applied_admission_stamps.clear();
        self.applied_idle_checkpoint_stamps.clear();
        // Fence SQL itself, including repositories that issue direct writes
        // without write_transaction(). ROLLBACK and diagnostic reads remain
        // available. This connection-local flag is never cleared by cleanup.
        self.connection.pragma_update(None, "query_only", true)?;
        Ok(())
    }

    /// Diagnostics are staged until the real outer COMMIT. Savepoint success
    /// and rolled-back Host responses must never look durably committed.
    pub(super) fn observe_committed_host_input(&mut self, input: Option<&nola_store_port::run_effect::HostEventInput>) {
        if !nola_measurement::enabled() { return; }
        let Some(input)=input else{return};
        let mut context=nola_measurement::store_context(&input.effect_id,&input.request_id);
        // Bound optional identity decoding. Large payloads retain effect/request
        // correlation without scanning unbounded tool output for diagnostics.
        #[derive(serde::Deserialize)]
        struct Identity { attempt_id: Option<String> }
        if input.payload_json.len()<=65_536 {
            if let Ok(identity)=serde_json::from_str::<Identity>(&input.payload_json) {
                context=nola_measurement::with_attempt(context,identity.attempt_id.as_deref());
            }
        } else { nola_measurement::dropped(); }
        if self.has_write_unit_of_work() {
            if self.pending_measurements.len()<128 {self.pending_measurements.push(context);}
            else {nola_measurement::dropped();}
        } else {
            let _scope=nola_measurement::enter(context);
            nola_measurement::point_outcome(nola_measurement::Stage::StoreCommittedHostResponse,nola_measurement::Outcome::Succeeded);
        }
    }

    fn prune_settled_outbox_rows(&mut self) -> StoreResult<()> {
        // Housekeeping changes only transport metadata, never canonical body,
        // owner or disclosure inputs. Do not use the content-publication UoW:
        // its commit settles every pending disclosure generation, which would
        // make opening a Store parse all unrelated historical provider bodies.
        // Dirty/unknown content must remain conservative until its actual
        // content owner publishes it, not become a startup backfill.
        let transaction = self.connection.transaction()?;
        // Pending facts and their dependencies were committed atomically by
        // enqueue_sync_change. Opening a database must not reinterpret that
        // queue or parse potentially huge content payloads to re-coalesce it.
        // Acknowledged seals are durable prerequisites for later reference
        // publications, including after a restart. They are bounded metadata;
        // dropping them here would strand an otherwise complete import.
        transaction.execute(
            "DELETE FROM sync_outbox WHERE status!='pending' AND entity_type NOT IN (
                'canonical_text_seal','part_derivation','archive_restore_begin','archive_restore_entry','archive_restore_target'
            )", [],
        )?;
        // Host delivery may be acknowledged while Core still buffers this
        // terminal response behind another parallel batch member. Keep its
        // canonical request until Core has consumed it. Use the transactional
        // current witness, never the potentially older replay baseline body;
        // if a current witness is unavailable, conservatively retain the row.
        // Result provenance also owns the frozen request: compact rN/eN
        // assessments must be revalidated against its immutable binding after
        // restart. An acknowledged delivery is not permission to delete these
        // sources. They remain scoped to their Run and cascade with its removal.
        transaction.execute("DELETE FROM effect_outbox WHERE status='acknowledged'
            AND NOT EXISTS (
                SELECT 1 FROM run_requirement_sources s
                JOIN host_event_inbox h ON h.event_id=s.source_host_event_id
                WHERE s.run_id=effect_outbox.run_id
                  AND h.run_id=effect_outbox.run_id
                  AND h.effect_id=effect_outbox.effect_id
                  AND h.request_id=effect_outbox.request_id
            )
            AND EXISTS (
                SELECT 1 FROM runs r
                JOIN core_checkpoints cp ON cp.conversation_id=r.conversation_id
                JOIN core_delivery_witnesses w ON w.conversation_id=cp.conversation_id
                  AND w.checkpoint_revision=cp.revision AND w.event_cursor=cp.event_cursor
                WHERE r.id=effect_outbox.run_id
                  AND json_type(w.witness_json,'$.state.host.pending_host_effects.effects')='object'
                  AND NOT EXISTS (
                    SELECT 1 FROM json_each(w.witness_json,'$.state.host.pending_host_effects.effects') p
                    WHERE p.key=effect_outbox.effect_id
                  )
            )", [])?;
        transaction.commit()?;
        Ok(())
    }

    pub fn schema_version(&self) -> StoreResult<i64> {
        Ok(STORE_SCHEMA_VERSION)
    }

    /// Runs a read-only aggregate query against one SQLite snapshot.
    ///
    /// Application read models that combine checkpoints, permissions, runtime
    /// tasks and effect transactions must not observe a different WAL revision
    /// for each table. A savepoint pins the read view on the first query while
    /// allowing the existing typed Store readers to remain the only decoders.
    pub fn with_consistent_read<T>(
        &mut self,
        reader: impl FnOnce(&Self) -> Result<T, String>,
    ) -> Result<T, String> {
        self.connection
            .execute_batch("SAVEPOINT nola_consistent_read")
            .map_err(|error| format!("begin consistent Store read failed: {error}"))?;
        // SAVEPOINT alone leaves SQLite in SQLITE_TXN_NONE. Establish the
        // promised snapshot before cache validation or any aggregate read.
        let result = self.connection.query_row("SELECT COUNT(*) FROM sqlite_master", [],
            |row| row.get::<_, i64>(0))
            .map_err(|error| format!("pin consistent Store read failed: {error}"))
            .and_then(|_| reader(self));
        match result {
            Ok(value) => {
                self.connection
                    .execute_batch("RELEASE SAVEPOINT nola_consistent_read")
                    .map_err(|error| format!("finish consistent Store read failed: {error}"))?;
                Ok(value)
            }
            Err(error) => {
                let _ = self.connection.execute_batch(
                    "ROLLBACK TO SAVEPOINT nola_consistent_read; \
                     RELEASE SAVEPOINT nola_consistent_read",
                );
                Err(error)
            }
        }
    }

    /// Mutable-receiver variant of [`Self::with_consistent_read`] so wire-level
    /// read-only command dispatchers (`&mut R`) share one pinned WAL snapshot.
    pub fn with_consistent_read_mut<T>(
        &mut self,
        reader: impl FnOnce(&mut Self) -> Result<T, String>,
    ) -> Result<T, String> {
        self.connection
            .execute_batch("SAVEPOINT nola_consistent_read_mut")
            .map_err(|error| format!("begin consistent Store read failed: {error}"))?;
        let result = self
            .connection
            .query_row("SELECT COUNT(*) FROM sqlite_master", [], |row| {
                row.get::<_, i64>(0)
            })
            .map_err(|error| format!("pin consistent Store read failed: {error}"))
            .and_then(|_| reader(self));
        match result {
            Ok(value) => {
                self.connection
                    .execute_batch("RELEASE SAVEPOINT nola_consistent_read_mut")
                    .map_err(|error| format!("finish consistent Store read failed: {error}"))?;
                Ok(value)
            }
            Err(error) => {
                let _ = self.connection.execute_batch(
                    "ROLLBACK TO SAVEPOINT nola_consistent_read_mut; \
                     RELEASE SAVEPOINT nola_consistent_read_mut",
                );
                Err(error)
            }
        }
    }
}

#[cfg(test)]
mod read_only_tests {
    use super::*;
    use nola_store_port::memory::MemoryReadRepository;

    #[test]
    fn open_read_only_rejects_in_memory_and_uninitialized_files_without_mutation() {
        let err = NolaStore::open_read_only(":memory:").unwrap_err();
        assert!(matches!(err, StoreError::Invariant(_)));

        let directory = tempfile::tempdir().unwrap();
        let empty_path = directory.path().join("empty.sqlite");
        std::fs::write(&empty_path, b"").unwrap();
        let err = NolaStore::open_read_only(&empty_path).unwrap_err();
        assert!(matches!(err, StoreError::Invariant(_)));
        assert_eq!(std::fs::read(&empty_path).unwrap().len(), 0);
    }

    #[test]
    fn read_only_snapshot_reads_concurrently_under_writer_lock_and_fences_writes() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("wal_replica.sqlite");
        let mut writer = NolaStore::open(&path).unwrap();
        nola_store_port::product_setting::ProductSettingRepository::put_product_setting(
            &mut writer.product_setting_repository(),
            "theme",
            "\"dark\"",
            100,
        )
        .unwrap();
        writer
            .upsert_memory_record(&crate::StoredMemoryRecord {
                id: "mem_1".into(),
                owner_json: r#"{"kind":"profile","profile_id":"local"}"#.into(),
                namespace_json: r#"{"kind":"personal"}"#.into(),
                scope: "global".into(),
                kind: "preference".into(),
                content: "Configure PostgreSQL connection pooler timeout to 15 seconds.".into(),
                summary: None,
                tags_json: r#"["postgres_pool"]"#.into(),
                confidence: 0.9,
                importance: 0.8,
                sensitivity: Some("normal".into()),
                retention_json: r#"{"mode":"until_superseded"}"#.into(),
                expires_at: None,
                supersedes_json: "[]".into(),
                provenance_json: r#"{"kind":"explicit_user","source":"test","session_id":null,"turn_id":null,"item_ids":[],"observed_at":null}"#.into(),
                created_at_ms: 100,
                updated_at_ms: 100,
                last_used_at_ms: None,
                deleted_at_ms: None,
            })
            .unwrap();
        writer
            .connection
            .execute_batch(
                "INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms)
                 VALUES('conv-1','Title',1,100,100);
                 INSERT INTO messages VALUES('msg-1','conv-1',NULL,'user','completed',1,100,100);
                 INSERT INTO message_parts(id,message_id,kind,ordinal,state,content_text,created_at_ms,updated_at_ms)
                 VALUES('part-1','msg-1','text',1,'completed','read_only_transcript_needle_42',100,100);",
            )
            .unwrap();
        crate::canonical_text_disclosure::flush(&writer.connection).unwrap();

        let mut reader = NolaStore::open_read_only(&path).unwrap();
        assert!(reader.is_read_only());

        let write_err = reader.begin_write_unit_of_work().unwrap_err();
        assert_eq!(
            write_err.to_string(),
            "Store invariant failed: read-only Store snapshot cannot start a write transaction"
        );

        // Hold an uncommitted BEGIN IMMEDIATE write transaction on the writer.
        writer.begin_write_unit_of_work().unwrap();
        writer
            .connection
            .execute(
                "UPDATE product_settings SET value_json='\"light\"', revision=2 WHERE key='theme'",
                [],
            )
            .unwrap();

        // Reader observes committed WAL snapshot without blocking on writer lock,
        // including nested consistent reads, transcript search without FTS DDL,
        // and BM25 memory recall without FTS backfill writes.
        let observed = reader
            .with_consistent_read_mut(|store| {
                store.with_consistent_read_mut(|inner| {
                    let setting = nola_store_port::product_setting::ProductSettingRepository::product_setting(
                        &inner.product_setting_repository(),
                        "theme",
                    )
                    .map_err(|e| e.to_string())?
                    .unwrap();
                    assert_eq!(setting.value_json, "\"dark\"");

                    let hits = inner
                        .search_transcript_text("conv-1", "read_only_transcript_needle_42", 5)
                        .map_err(|e| e.to_string())?;
                    assert_eq!(hits.len(), 1);
                    assert_eq!(hits[0].part_id, "part-1");

                    let recalled = MemoryReadRepository::recall_memory_records(
                        inner,
                        r#"{"kind":"profile","profile_id":"local"}"#,
                        r#"{"kind":"personal"}"#,
                        "global",
                        Some("PostgreSQL timeout postgres_pool"),
                        None,
                        "2026-10-01T00:00:00Z",
                        10,
                    )
                    .map_err(|e| e.to_string())?;
                    assert_eq!(recalled.len(), 1);
                    assert_eq!(recalled[0].id, "mem_1");
                    Ok(setting.revision)
                })
            })
            .unwrap();
        assert_eq!(observed, 1);

        writer.rollback_write_unit_of_work().unwrap();
    }
}
