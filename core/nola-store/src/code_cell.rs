// SPDX-License-Identifier: Apache-2.0
//! Conversation-owned observations; no worker restoration or source replay.
use crate::{params, OptionalExtension, StoreError};
use nola_store_port::code_cell::{CodeCellObservationRepository, CodeCellQuery, CodeCellRecord};

fn invalid(message: &str) -> StoreError { StoreError::Invariant(message.into()) }

#[cfg(test)]
mod tests {
    use super::*;
    use nola_domain::code_mode::*;

    #[test]
    fn observations_are_transactional_owner_bound_and_checkpoint_independent() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("code-observation.sqlite");
        let mut store = crate::NolaStore::open(&database).unwrap();
        store.connection.execute_batch("BEGIN;
            INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms) VALUES('conversation','',1,1,1);
            INSERT INTO runs(id,conversation_id,kind,status,next_event_sequence,created_at_ms,updated_at_ms)
                VALUES('run','conversation','conversation','running',1,1,1);
            INSERT INTO messages(id,conversation_id,run_id,role,status,ordinal,created_at_ms)
                VALUES('message','conversation','run','assistant','running',1,1);
            INSERT INTO message_parts(id,message_id,kind,ordinal,state,created_at_ms,updated_at_ms)
                VALUES('part','message','tool',1,'running',1,1);
            COMMIT;").unwrap();
        let epoch = store.connection.query_row("SELECT value FROM store_metadata WHERE key='assistant_source_epoch'",
            [], |row| row.get::<_, String>(0)).unwrap();
        let mut record = CodeCellRecord {
            store_epoch:epoch.clone(), conversation_id:"conversation".into(), run_id:"run".into(),
            parent_part_id:"part".into(), core_revision:1,
            cell:CodeCellIdentity {host_id:"host".into(), conversation_id:"core-session".into(),
                turn_id:"turn".into(), parent_call_id:"call".into(), cell_id:"cell".into(), generation:"g".into()},
            observation:CodeCellView {cell_id:"cell".into(), parent_call_id:"call".into(),
                phase:CodeCellPhase::Completed, terminal:true, settled:true, next_output_sequence:1,
                has_more_output:false, output:vec![CodeOutputChunk {sequence:1,text:"saved".into()}],
                unsettled_calls:0, completion:CodeCellCompletionDetails::default()},
        };
        let query = CodeCellQuery {store_epoch:epoch, conversation_id:"conversation".into(),
            cell_id:"cell".into(), after_output_sequence:0};
        assert!(store.publish_code_cell(&record).is_err(), "publication requires a transaction");
        store.connection.execute_batch("BEGIN").unwrap();
        store.publish_code_cell(&record).unwrap();
        store.connection.execute_batch("ROLLBACK").unwrap();
        assert!(store.read_code_cell(&query).unwrap().is_none());
        store.connection.execute_batch("BEGIN").unwrap();
        store.publish_code_cell(&record).unwrap();
        store.connection.execute_batch("COMMIT").unwrap();
        drop(store);
        let mut store = crate::NolaStore::open(&database).unwrap();
        let checkpoints: i64 = store.connection.query_row("SELECT count(*) FROM core_checkpoint_bodies",
            [], |row| row.get(0)).unwrap();
        assert_eq!(checkpoints, 0, "cold observation must not require a Core checkpoint");
        assert_eq!(store.read_code_cell(&query).unwrap().unwrap(), record.observation);
        let mut foreign = query.clone();
        foreign.conversation_id = "other".into();
        assert!(store.read_code_cell(&foreign).unwrap().is_none());
        foreign = query.clone();
        foreign.store_epoch = "other".into();
        assert!(store.read_code_cell(&foreign).is_err());
        store.connection.execute_batch("BEGIN").unwrap();
        record.core_revision = 2;
        store.publish_code_cell(&record).unwrap();
        record.observation.output[0].text = "rewritten".into();
        assert!(store.publish_code_cell(&record).is_err());
        store.connection.execute_batch("ROLLBACK").unwrap();
        assert_eq!(store.read_code_cell(&query).unwrap().unwrap().output[0].text, "saved");
        // Indexed ownership and the serialized record must agree on cold
        // reads, even when each independently satisfies SQLite constraints.
        store.connection.execute_batch("BEGIN;
            UPDATE code_cell_observations SET core_revision=99;").unwrap();
        assert!(store.read_code_cell(&query).is_err());
        store.connection.execute_batch("ROLLBACK").unwrap();
        store.connection.execute_batch("BEGIN;
            UPDATE message_parts SET kind='text' WHERE id='part';").unwrap();
        assert!(store.read_code_cell(&query).is_err());
        store.connection.execute_batch("ROLLBACK").unwrap();
        // Revert/deletion may remove the parent Part or the entire owner.
        // In either case observations disappear transactionally, not as a
        // later cleanup that could expose detached execution history.
        for deletion in [
            "DELETE FROM message_parts WHERE id='part'",
            "DELETE FROM runs WHERE id='run'",
            "DELETE FROM conversations WHERE id='conversation'",
        ] {
            store.connection.execute_batch("BEGIN").unwrap();
            store.connection.execute(deletion, []).unwrap();
            assert!(store.read_code_cell(&query).unwrap().is_none());
            store.connection.execute_batch("ROLLBACK").unwrap();
            assert_eq!(store.read_code_cell(&query).unwrap().unwrap().output[0].text, "saved");
        }
    }
}

impl CodeCellObservationRepository for crate::NolaStore {
    type Error = StoreError;

    fn publish_code_cell(&mut self, record: &CodeCellRecord) -> Result<(), StoreError> {
        if self.connection.is_autocommit() { return Err(invalid("cell publication requires owning transaction")); }
        record.validate().map_err(|error| invalid(&error))?;
        let epoch: String = self.connection.query_row(
            "SELECT value FROM store_metadata WHERE key='assistant_source_epoch'", [], |row| row.get(0))?;
        if epoch != record.store_epoch { return Err(invalid("cell observation store epoch mismatch")); }
        let owner: Option<(String, Option<String>, String)> = self.connection.query_row(
            "SELECT m.conversation_id,m.run_id,p.kind FROM message_parts p
             JOIN messages m ON m.id=p.message_id
             JOIN runs r ON r.id=m.run_id AND r.conversation_id=m.conversation_id WHERE p.id=?1",
            [&record.parent_part_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))).optional()?;
        if owner != Some((record.conversation_id.clone(), Some(record.run_id.clone()), "tool".into())) {
            return Err(invalid("cell observation parent Part owner mismatch"));
        }
        let previous: Option<String> = self.connection.query_row(
            "SELECT record_json FROM code_cell_observations WHERE conversation_id=?1 AND cell_id=?2",
            params![record.conversation_id, record.cell.cell_id], |row| row.get(0)).optional()?;
        if let Some(raw) = previous {
            let old: CodeCellRecord = serde_json::from_str(&raw)?;
            old.validate().map_err(|error| invalid(&error))?;
            if old == *record { return Ok(()); }
            if old.store_epoch != record.store_epoch || old.cell != record.cell
                || old.run_id != record.run_id || old.parent_part_id != record.parent_part_id
                || record.core_revision <= old.core_revision {
                return Err(invalid("stale or conflicting cell observation owner"));
            }
            // Unrelated Core transitions may publish the same immutable
            // observation again. They do not advance this record's revision.
            if old.observation == record.observation { return Ok(()); }
            use nola_domain::code_mode::CodeCellPhase;
            let phase_allowed = old.observation.phase == record.observation.phase || matches!(
                (&old.observation.phase, &record.observation.phase),
                (CodeCellPhase::Running, CodeCellPhase::Cancelling | CodeCellPhase::Completed
                    | CodeCellPhase::Failed | CodeCellPhase::Cancelled | CodeCellPhase::Interrupted)
                    | (CodeCellPhase::Cancelling, CodeCellPhase::Cancelled | CodeCellPhase::Interrupted));
            if old.observation.settled
                || !phase_allowed
                || !record.observation.output.starts_with(&old.observation.output)
                || (old.observation.terminal && old.observation.phase != record.observation.phase) {
                return Err(invalid("stale or conflicting cell observation"));
            }
        }
        let revision = i64::try_from(record.core_revision).map_err(|_| invalid("cell revision overflow"))?;
        self.connection.execute(
            "INSERT INTO code_cell_observations(conversation_id,cell_id,run_id,parent_part_id,core_revision,record_json)
             VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(conversation_id,cell_id) DO UPDATE SET core_revision=excluded.core_revision,record_json=excluded.record_json",
            params![record.conversation_id, record.cell.cell_id, record.run_id, record.parent_part_id,
                revision, serde_json::to_string(record)?])?;
        Ok(())
    }

    fn read_code_cell(&self, query: &CodeCellQuery) -> Result<Option<nola_domain::code_mode::CodeCellView>, StoreError> {
        if [&query.store_epoch, &query.conversation_id, &query.cell_id].iter().any(|id| id.trim().is_empty()) {
            return Err(invalid("cell observation requires exact query identities"));
        }
        let epoch: String = self.connection.query_row(
            "SELECT value FROM store_metadata WHERE key='assistant_source_epoch'", [], |row| row.get(0))?;
        if query.store_epoch != epoch { return Err(invalid("cell observation store epoch mismatch")); }
        let stored: Option<(String, String, String, i64)> = self.connection.query_row(
            "SELECT record_json,run_id,parent_part_id,core_revision FROM code_cell_observations
             WHERE conversation_id=?1 AND cell_id=?2",
            params![query.conversation_id, query.cell_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))).optional()?;
        let Some((raw, run_id, parent_part_id, revision)) = stored else { return Ok(None); };
        let record: CodeCellRecord = serde_json::from_str(&raw)?;
        record.validate().map_err(|error| invalid(&error))?;
        if record.store_epoch != epoch || record.conversation_id != query.conversation_id
            || record.cell.cell_id != query.cell_id || record.run_id != run_id
            || record.parent_part_id != parent_part_id
            || i64::try_from(record.core_revision).ok() != Some(revision) {
            return Err(invalid("cell observation owner mismatch"));
        }
        let parent_matches: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM message_parts p JOIN messages m ON m.id=p.message_id
             JOIN runs r ON r.id=m.run_id AND r.conversation_id=m.conversation_id
             WHERE p.id=?1 AND p.kind='tool' AND m.conversation_id=?2 AND r.id=?3)",
            params![parent_part_id, query.conversation_id, run_id], |row| row.get(0))?;
        if !parent_matches { return Err(invalid("cell observation parent Part owner mismatch")); }
        record.observation.page(query.after_output_sequence)
            .map(Some).map_err(|error| invalid(&error))
    }
}
