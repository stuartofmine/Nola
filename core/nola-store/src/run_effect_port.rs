// SPDX-License-Identifier: Apache-2.0
//! SQLite Run/Effect adapter. Uses the same connection and existing savepoints.
use crate::{NolaStore, StoreError, StoreResult};
use nola_store_port::run_effect::*;

fn read_checkpoint_stamp(connection: &rusqlite::Connection) -> Result<Option<(i64, u64)>, StoreError> {
    // Reuse only inside a pinned read transaction. In particular, a result
    // derived inside a write transaction must not survive its rollback.
    // SAFETY: the handle belongs to this live connection; SQLite only inspects
    // its transaction state and does not retain the static schema name.
    let state = unsafe { rusqlite::ffi::sqlite3_txn_state(connection.handle(), c"main".as_ptr()) };
    if state != rusqlite::ffi::SQLITE_TXN_READ { return Ok(None); }
    let version = connection.query_row("PRAGMA data_version", [], |row| row.get(0))?;
    let changes = connection.query_row("SELECT total_changes()", [], |row| row.get(0))?;
    Ok(Some((version, changes)))
}

pub(crate) fn read_recovery_checkpoint(
    connection: &rusqlite::Connection, conversation_id: &str,
) -> Result<Option<StoredRecoveryCheckpoint>, StoreError> {
        use rusqlite::OptionalExtension;
        let _snapshot = crate::read_snapshot::pin(connection)?;
        let checkpoint = connection.query_row(
            "SELECT conversation_id,revision,event_cursor,checkpoint_json,updated_at_ms FROM core_checkpoints LEFT JOIN core_checkpoint_bodies USING(conversation_id) WHERE conversation_id=?1",
            [conversation_id], |row| Ok(StoredCheckpoint {
                conversation_id: row.get(0)?, revision: row.get(1)?, event_cursor: row.get(2)?,
                checkpoint_json: row.get(3)?, updated_at_ms: row.get(4)?,
            }),
        ).optional()?;
        let Some(checkpoint) = checkpoint else { return Ok(None); };
        let replay_base = connection.query_row(
            "SELECT run_id,checkpoint_revision,through_ordinal FROM core_checkpoint_replay_bases WHERE conversation_id=?1",
            [conversation_id], |row| Ok(StoredCheckpointReplayBase {
                run_id: row.get(0)?, checkpoint_revision: row.get(1)?, through_ordinal: row.get(2)?,
            }),
        ).optional()?;
        if replay_base.as_ref().is_some_and(|base| base.checkpoint_revision > checkpoint.revision) {
            return Err(StoreError::Conflict("checkpoint replay base revision mismatch".into()));
        }
        let mut replay_inputs = Vec::new();
        if let Some(base) = &replay_base {
            let (head,head_bytes,base_bytes): (i64,i64,i64) = connection.query_row(
                "SELECT COALESCE(h.ordinal,0),COALESCE(h.reduction_bytes,0),b.through_reduction_bytes
                 FROM core_checkpoint_replay_bases b LEFT JOIN host_event_replay_heads h ON h.run_id=b.run_id
                 WHERE b.run_id=?1 AND b.conversation_id=?2",
                rusqlite::params![base.run_id,conversation_id], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
            )?;
            if base.through_ordinal > head || base.through_ordinal < 0 {
                return Err(StoreError::Conflict(
                    "checkpoint replay prefix exceeds the durable Host replay frontier".into(),
                ));
            }
            let tail_bytes=head_bytes.checked_sub(base_bytes).filter(|bytes| *bytes>=0)
                .ok_or_else(||StoreError::Conflict("invalid Host replay byte frontier".into()))?;
            if head-base.through_ordinal > nola_store_port::host_replay::HOST_REPLAY_MAX_RECORDS as i64
                || tail_bytes > nola_store_port::host_replay::HOST_REPLAY_MAX_BYTES as i64 {
                return Err(StoreError::Conflict("Host replay budget exceeded; explicit recovery required".into()));
            }
            let mut query = connection.prepare_cached(
                "SELECT o.ordinal,COALESCE(o.submission_json,h.reduction_json),
                    COALESCE(o.decision_time_ms,h.received_at_ms),o.submission_id IS NOT NULL
                 FROM host_event_replay_order o LEFT JOIN host_event_inbox h
                 ON h.event_id=o.event_id AND h.run_id=o.run_id
                 WHERE o.run_id=?1 AND o.ordinal>?2 AND o.ordinal<=?3 ORDER BY o.ordinal",
            )?;
            let rows = query.query_map(rusqlite::params![base.run_id, base.through_ordinal, head], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?, row.get::<_, i64>(2)?, row.get::<_, bool>(3)?))
            })?;
            let mut consumed = base.through_ordinal;
            let mut consumed_bytes=0usize;
            for row in rows {
                let (ordinal, reduction_json, received_at_ms, is_submission) = row?;
                if consumed.checked_add(1) != Some(ordinal) || received_at_ms < 0 {
                    return Err(StoreError::Conflict("invalid Host replay order or decision time".into()));
                }
                let reduction_json = reduction_json.ok_or_else(|| StoreError::Conflict(
                    "Host replay tail has no recorded reducer input".into(),
                ))?;
                consumed_bytes=consumed_bytes.checked_add(reduction_json.len())
                    .filter(|bytes| *bytes<=nola_store_port::host_replay::HOST_REPLAY_MAX_BYTES)
                    .ok_or_else(||StoreError::Conflict("Host replay bytes exceed bounded recovery".into()))?;
                let kind = if is_submission { ReplayInputKind::Submission } else { ReplayInputKind::HostResponse };
                replay_inputs.push(StoredCoreReplayInput { kind, ordinal, reduction_json, received_at_ms });
                consumed = ordinal;
            }
            if consumed != head || consumed_bytes as i64 != tail_bytes {
                return Err(StoreError::Conflict("Host replay tail differs from its durable order or byte frontier".into()));
            }
        }
        let mut frozen_model_requests = Vec::new();
        if let Some(base) = &replay_base {
            if !replay_inputs.is_empty() {
                let ids = connection.prepare_cached(
                    "SELECT effect_id FROM effect_outbox WHERE run_id=?1 AND status='pending' AND effect_kind='model_stream' ORDER BY effect_id"
                )?.query_map([&base.run_id], |row| row.get::<_, String>(0))?
                    .collect::<Result<Vec<_>, _>>()?;
                for id in ids {
                    let effect = crate::outbox_payload::read(connection, &id)?
                        .ok_or_else(|| StoreError::Invariant("recovery model input disappeared".into()))?;
                    frozen_model_requests.push(effect.payload_json);
                }
            }
        }
        Ok(Some(StoredRecoveryCheckpoint { checkpoint, replay_base, replay_inputs, frozen_model_requests }))
}

impl RunEffectRepository for NolaStore {
    type Error = StoreError;
    fn prepare_unstarted_cancellation_target(&self, run: &str,
        cancellation: &nola_domain::host_control::HostEffectControl,
        target: &nola_domain::host_control::HostEffectControl, revision: i64, at: i64)
        -> StoreResult<Option<nola_store_port::run_effect::PreparedCancellationNotStarted>> {
        crate::cancellation_not_started::prepare(&self.connection,run,cancellation,target,revision,at)
    }
    fn cached_read_checkpoint(&self, conversation_id: &str) -> Result<Option<StoredCheckpoint>, Self::Error> {
        let stamp = read_checkpoint_stamp(&self.connection)?;
        let mut cache = self.read_checkpoint_cache.borrow_mut();
        if cache.as_ref().is_some_and(|(version, changes, _)| Some((*version, *changes)) != stamp) {
            *cache = None;
        }
        Ok(cache.as_ref().filter(|(_, _, value)| value.conversation_id == conversation_id)
            .map(|(_, _, value)| value.clone()))
    }
    fn cache_read_checkpoint(&self, checkpoint: &StoredCheckpoint) -> Result<(), Self::Error> {
        // Single-entry, eight-MiB ceiling: visiting many conversations cannot
        // retain every recovered execution state behind the transcript.
        let stamp = read_checkpoint_stamp(&self.connection)?;
        *self.read_checkpoint_cache.borrow_mut() = stamp
            .filter(|_| checkpoint.checkpoint_json.len() <= 8 * 1024 * 1024)
            .map(|(version, changes)| (version, changes, checkpoint.clone()));
        Ok(())
    }
    fn pending_outbox_effect_ids(&self, run_id: &str) -> Result<Vec<String>, Self::Error> {
        crate::require_id("run_id", run_id)?;
        let mut query = self.connection.prepare_cached(
            "SELECT effect_id FROM effect_outbox WHERE run_id=?1 AND status='pending' ORDER BY effect_id"
        )?;
        let ids = query.query_map([run_id], |row| row.get(0))?
            .collect::<Result<Vec<String>, _>>()?;
        Ok(ids)
    }
    fn committed_terminal_host_response(&self, conversation_id: &str, run_id: &str,
        event_id: &str, response_json: &str) -> Result<Option<nola_store_port::run_effect::CommittedHostResponseReceipt>, Self::Error> {
        use rusqlite::OptionalExtension;
        let evidence: Option<(String, String, bool, i64, i64)> = self.connection.query_row(
            "SELECT r.conversation_id,h.run_id,h.payload_json=?4,t.checkpoint_revision,t.next_event_sequence
             FROM host_event_inbox h JOIN runs r ON r.id=h.run_id
             JOIN committed_transitions t ON t.run_id=h.run_id
               AND t.transition_id='transition:'||h.event_id
             WHERE h.event_id=?3",
            rusqlite::params![conversation_id, run_id, event_id, response_json],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        ).optional()?;
        let Some((owner, original_run, exact, checkpoint_revision, next_event_sequence)) = evidence else { return Ok(None); };
        if owner != conversation_id || original_run != run_id || !exact {
            return Err(StoreError::Conflict("committed Host response identity or payload changed".into()));
        }
        Ok(Some(nola_store_port::run_effect::CommittedHostResponseReceipt { checkpoint_revision, next_event_sequence }))
    }
    fn outbox_effect_identity(&self, effect_id:&str)->Result<Option<StoredOutboxEffectIdentity>,Self::Error> {
        crate::outbox_payload::identity(&self.connection,effect_id)
    }
    fn checkpoint_replay_pending(&self, conversation_id: &str) -> Result<bool, Self::Error> {
        Ok(self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM core_checkpoint_replay_bases b JOIN host_event_replay_heads h ON h.run_id=b.run_id
             WHERE b.conversation_id=?1 AND h.ordinal>b.through_ordinal)", [conversation_id], |row| row.get(0),
        )?)
    }
    fn checkpoint_replay_ready(&self, conversation_id: &str, run_id: &str, revision: i64, additional_reduction_bytes: usize) -> Result<bool, Self::Error> {
        use nola_store_port::host_replay::{HOST_REPLAY_MAX_BYTES, HOST_REPLAY_CHECKPOINT_RECORDS};
        if additional_reduction_bytes > HOST_REPLAY_MAX_BYTES { return Ok(false); }
        Ok(self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM core_checkpoint_replay_bases b
             JOIN core_checkpoints c ON c.conversation_id=b.conversation_id
             JOIN core_delivery_witnesses w ON w.conversation_id=c.conversation_id AND w.checkpoint_revision=c.revision
             LEFT JOIN host_event_replay_heads h ON h.run_id=b.run_id
             WHERE b.conversation_id=?1 AND b.run_id=?2 AND c.revision=?3
             AND COALESCE(h.ordinal,0)-b.through_ordinal BETWEEN 0 AND ?4
             AND COALESCE(h.reduction_bytes,0)-b.through_reduction_bytes BETWEEN 0 AND ?5)",
            rusqlite::params![conversation_id,run_id,revision,
                HOST_REPLAY_CHECKPOINT_RECORDS-1,(HOST_REPLAY_MAX_BYTES-additional_reduction_bytes) as i64], |row| row.get(0),
        )?)
    }
    fn load_recovery_checkpoint(&self, conversation_id: &str) -> Result<Option<StoredRecoveryCheckpoint>, Self::Error> {
        read_recovery_checkpoint(&self.connection, conversation_id)
    }
    fn append_host_event_history(&mut self, run_id: &str, events: &[HostEventInput]) -> Result<(), Self::Error> {
        if events.is_empty() { return Ok(()); }
        if !self.has_write_unit_of_work() {
            return Err(StoreError::Invariant("Host event history requires its boundary transaction".into()));
        }
        crate::require_id("run_id", run_id)?;
        let transaction = self.write_transaction()?;
        for event in events {
            if crate::conversation_transition_store::matching_host_event_replay(&transaction, run_id, event)? {
                continue;
            }
            transaction.execute(
                "INSERT INTO host_event_inbox(event_id,run_id,effect_id,request_id,sequence,payload_json,received_at_ms,reduction_json)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
                rusqlite::params![event.event_id,run_id,event.effect_id,event.request_id,event.sequence,event.payload_json,event.received_at_ms,event.reduction_json],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }
    fn unrecorded_memory_usage_entries(
        &self,
        entries: &[nola_store_port::memory::StoredMemoryUsageEntry],
    ) -> Result<Vec<nola_store_port::memory::StoredMemoryUsageEntry>, Self::Error> {
        use rusqlite::OptionalExtension;
        let mut query = self.connection.prepare_cached(
            "SELECT usage_id,memory_id,session_id,turn_id,used_at_ms,context_payload_digest,citation_ids_json
             FROM memory_usage_ledger WHERE usage_id=?1",
        )?;
        let mut pending = Vec::new();
        for entry in entries {
            let stored = query.query_row([&entry.usage_id], |row| {
                Ok(nola_store_port::memory::StoredMemoryUsageEntry {
                    usage_id: row.get(0)?, memory_id: row.get(1)?,
                    session_id: row.get(2)?, turn_id: row.get(3)?,
                    used_at_ms: row.get(4)?, context_payload_digest: row.get(5)?,
                    citation_ids_json: row.get(6)?,
                })
            }).optional()?;
            match stored {
                None => pending.push(entry.clone()),
                Some(stored) if stored == *entry => {},
                Some(_) => return Err(StoreError::Conflict(format!(
                    "memory usage identity conflict for {}", entry.usage_id,
                ))),
            }
        }
        Ok(pending)
    }
    fn load_canonical_text_state(
        &self,
        owner: &nola_domain::CanonicalStreamOwner,
    ) -> Result<nola_domain::CanonicalTextState, Self::Error> {
        crate::canonical_stream_store::load_state(&self.connection, owner)
    }
    fn open_delegated_permissions_for_target(
        &self,
        effect_id: &str,
    ) -> Result<Vec<nola_domain::DelegatedPermissionDeliveryRecord>, Self::Error> {
        NolaStore::open_delegated_permissions_for_target(self, effect_id)
    }
    fn delegated_permission(
        &self,
        id: &str,
    ) -> Result<Option<nola_domain::DelegatedPermissionDeliveryRecord>, Self::Error> {
        NolaStore::delegated_permission(self, id)
    }
    fn delegated_permission_for_occurrence(
        &self,
        invocation_id: &str,
        occurrence_id: &str,
    ) -> Result<Option<nola_domain::DelegatedPermissionDeliveryRecord>, Self::Error> {
        NolaStore::delegated_permission_for_occurrence(self, invocation_id, occurrence_id)
    }
    fn delegated_permission_for_control(
        &self,
        effect_id: &str,
    ) -> Result<Option<nola_domain::DelegatedPermissionDeliveryRecord>, Self::Error> {
        NolaStore::delegated_permission_for_control(self, effect_id)
    }
    fn delegated_invocation(
        &self,
        id: &str,
    ) -> Result<Option<nola_domain::DelegatedInvocationRecord>, Self::Error> {
        NolaStore::delegated_invocation(self, id)
    }
    fn delegated_invocation_for_effect(
        &self,
        id: &str,
    ) -> Result<Option<nola_domain::DelegatedInvocationRecord>, Self::Error> {
        NolaStore::delegated_invocation_for_effect(self, id)
    }
    fn model_attempt(
        &self,
        id: &str,
    ) -> Result<Option<nola_domain::ModelAttemptRecord>, Self::Error> {
        NolaStore::model_attempt(self, id)
    }
    fn active_model_attempt_for_effect(
        &self,
        id: &str,
    ) -> Result<Option<nola_domain::ModelAttemptRecord>, Self::Error> {
        NolaStore::active_model_attempt_for_effect(self, id)
    }
    fn checkpoint_revision(&self, id: &str) -> Result<Option<i64>, Self::Error> {
        NolaStore::checkpoint_revision(self, id)
    }
    fn load_checkpoint(&self, id: &str) -> Result<Option<StoredCheckpoint>, Self::Error> {
        NolaStore::load_checkpoint(self, id)
    }
    fn active_run(&self, id: &str) -> Result<Option<StoredActiveRun>, Self::Error> {
        NolaStore::active_run(self, id)
    }
    fn load_run_invocation_config(
        &self,
        id: &str,
    ) -> Result<Option<StoredRunInvocationConfig>, Self::Error> {
        NolaStore::load_run_invocation_config(self, id)
    }
    fn outbox_effect(&self, id: &str) -> Result<Option<StoredOutboxEffect>, Self::Error> {
        NolaStore::outbox_effect(self, id)
    }
    fn load_committed_host_delivery(
        &self,
        run_id: &str,
    ) -> Result<CommittedHostDeliverySnapshot, Self::Error> {
        let _snapshot = crate::read_snapshot::pin(&self.connection)?;
        let effects = self.pending_effects(run_id)?;
        if effects.is_empty() {
            return Ok(CommittedHostDeliverySnapshot {
                conversation_id: None,
                checkpoint: None,
                effects,
            });
        }
        let conversation_id = self.conversation_id_for_run(run_id)?;
        let checkpoint = conversation_id
            .as_deref()
            .map(|id| self.load_checkpoint(id))
            .transpose()?
            .flatten();
        Ok(CommittedHostDeliverySnapshot {
            conversation_id,
            checkpoint,
            effects,
        })
    }
    fn load_selected_host_delivery_witness(
        &self,
        run_id: &str,
        effect_ids: Option<&std::collections::HashSet<String>>,
    ) -> Result<CommittedHostDeliverySnapshot<StoredDeliveryCheckpointWitness>, Self::Error> {
        use rusqlite::OptionalExtension;
        let _snapshot = crate::read_snapshot::pin(&self.connection)?;
        let effects = match effect_ids {
            None => self.pending_effects(run_id)?,
            Some(ids) => {
                if ids.len() > 64 {
                    return Err(StoreError::Invariant("selected delivery exceeds claim window".into()));
                }
                let mut effects = Vec::with_capacity(ids.len());
                let mut ids = ids.iter().collect::<Vec<_>>();
                ids.sort();
                for id in ids {
                    if let Some(effect) = self.outbox_effect(id)? {
                        if effect.run_id == run_id && effect.status == "pending" { effects.push(effect); }
                    }
                }
                effects
            }
        };
        if effects.is_empty() {
            return Ok(CommittedHostDeliverySnapshot { conversation_id: None, checkpoint: None, effects });
        }
        let conversation_id = self.conversation_id_for_run(run_id)?;
        let checkpoint = match conversation_id.as_deref() {
            None => None,
            Some(id) => self.connection.query_row(
                "SELECT w.conversation_id, w.witness_json
                 FROM core_delivery_witnesses w JOIN core_checkpoints c
                   ON c.conversation_id=w.conversation_id
                  AND c.revision=w.checkpoint_revision AND c.event_cursor=w.event_cursor
                 WHERE w.conversation_id=?1",
                [id], |row| Ok(StoredDeliveryCheckpointWitness {
                    conversation_id: row.get(0)?, witness_json: row.get(1)?,
                }),
            ).optional()?,
        };
        Ok(CommittedHostDeliverySnapshot { conversation_id, checkpoint, effects })
    }

    fn load_effect_heartbeat_snapshot(
        &self, run_id: &str, effect_id: &str,
    ) -> Result<Option<StoredEffectHeartbeatSnapshot>, Self::Error> {
        use rusqlite::OptionalExtension;
        // One statement supplies a consistent snapshot even outside a UoW.
        Ok(self.connection.query_row(
            "SELECT e.effect_id,e.run_id,e.request_id,w.conversation_id,w.witness_json
             FROM effect_outbox e JOIN runs r ON r.id=e.run_id
             LEFT JOIN core_checkpoints c ON c.conversation_id=r.conversation_id
             LEFT JOIN core_delivery_witnesses w ON w.conversation_id=c.conversation_id
                 AND w.checkpoint_revision=c.revision AND w.event_cursor=c.event_cursor
             WHERE e.effect_id=?1 AND e.run_id=?2 AND e.status='pending'",
            [effect_id, run_id], |row| {
                let owner: Option<String> = row.get(3)?;
                Ok(StoredEffectHeartbeatSnapshot {
                    effect_id: row.get(0)?, run_id: row.get(1)?, request_id: row.get(2)?,
                    checkpoint: match owner {
                        Some(conversation_id) => Some(StoredDeliveryCheckpointWitness {
                            conversation_id, witness_json: row.get(4)?,
                        }),
                        None => None,
                    },
                })
            },
        ).optional()?)
    }

    fn commit_transition(
        &mut self,
        input: &CommitTransitionInput,
    ) -> Result<CommitTransitionResult, Self::Error> {
        NolaStore::commit_transition(self, input)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BeginTurnInput;

    #[test]
    fn read_checkpoint_cache_is_snapshot_bound_and_rejects_rolled_back_writes() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("memo.sqlite");
        let store = NolaStore::open(&path).unwrap();
        store.connection.execute_batch("CREATE TABLE cache_fact(value INTEGER); INSERT INTO cache_fact VALUES(0)").unwrap();
        let checkpoint = StoredCheckpoint { conversation_id: "a".into(), revision: 1,
            event_cursor: 1, checkpoint_json: "{}".into(), updated_at_ms: 1 };
        {
            let _read = crate::read_snapshot::pin(&store.connection).unwrap();
            store.cache_read_checkpoint(&checkpoint).unwrap();
            assert_eq!(store.cached_read_checkpoint("a").unwrap(), Some(checkpoint.clone()));
            assert!(store.cached_read_checkpoint("b").unwrap().is_none());
        }
        {
            let _read = crate::read_snapshot::pin(&store.connection).unwrap();
            assert_eq!(store.cached_read_checkpoint("a").unwrap(), Some(checkpoint.clone()));
        }
        let writer = rusqlite::Connection::open(&path).unwrap();
        {
            let _read = crate::read_snapshot::pin(&store.connection).unwrap();
            writer.execute("UPDATE cache_fact SET value=1", []).unwrap();
            // A concurrent writer does not advance this already-pinned view.
            let value: i64 = store.connection.query_row("SELECT value FROM cache_fact", [], |r| r.get(0)).unwrap();
            assert_eq!(value, 0);
            assert_eq!(store.cached_read_checkpoint("a").unwrap(), Some(checkpoint.clone()));
        }
        {
            let _read = crate::read_snapshot::pin(&store.connection).unwrap();
            assert!(store.cached_read_checkpoint("a").unwrap().is_none());
            store.cache_read_checkpoint(&checkpoint).unwrap();
        }
        store.connection.execute_batch("BEGIN; UPDATE cache_fact SET value=2").unwrap();
        assert!(store.cached_read_checkpoint("a").unwrap().is_none());
        store.cache_read_checkpoint(&checkpoint).unwrap();
        store.connection.execute_batch("ROLLBACK").unwrap();
        {
            let _read = crate::read_snapshot::pin(&store.connection).unwrap();
            assert!(store.cached_read_checkpoint("a").unwrap().is_none());
            store.cache_read_checkpoint(&checkpoint).unwrap();
        }
        store.connection.execute("UPDATE cache_fact SET value=3", []).unwrap();
        {
            let _read = crate::read_snapshot::pin(&store.connection).unwrap();
            assert!(store.cached_read_checkpoint("a").unwrap().is_none());
            let mut large = checkpoint.clone();
            large.checkpoint_json = "x".repeat(8 * 1024 * 1024 + 1);
            store.cache_read_checkpoint(&large).unwrap();
            assert!(store.cached_read_checkpoint("a").unwrap().is_none());
        }
    }

    #[test]
    fn usage_filter_uses_transactional_ledger_and_preserves_deleted_history() {
        let mut store = NolaStore::open(":memory:").unwrap();
        store.connection.execute("INSERT INTO memory_records(
            id,owner_json,namespace_json,scope,kind,content,tags_json,confidence,importance,
            retention_json,supersedes_json,provenance_json,created_at_ms,updated_at_ms
        ) VALUES('memory','{\"kind\":\"profile\",\"profile_id\":\"local\"}','{}','global' ,'fact','fixture','[]',1,1,'{}','[]','{}',1,1)", []).unwrap();
        let entry = nola_store_port::memory::StoredMemoryUsageEntry {
            usage_id: "usage".into(), memory_id: "memory".into(),
            session_id: "session".into(), turn_id: "turn".into(), used_at_ms: 2,
            context_payload_digest: "digest".into(), citation_ids_json: "[]".into(),
        };
        store.begin_write_unit_of_work().unwrap();
        crate::memory_store::apply_memory_usage_entries(&store.connection, &[entry.clone()]).unwrap();
        assert!(store.unrecorded_memory_usage_entries(&[entry.clone()]).unwrap().is_empty());
        store.rollback_write_unit_of_work().unwrap();
        assert_eq!(store.unrecorded_memory_usage_entries(&[entry.clone()]).unwrap(), vec![entry.clone()]);
        crate::memory_store::apply_memory_usage_entries(&store.connection, &[entry.clone()]).unwrap();
        store.connection.execute("UPDATE memory_records SET deleted_at_ms=3 WHERE id='memory'", []).unwrap();
        assert!(store.unrecorded_memory_usage_entries(&[entry.clone()]).unwrap().is_empty());
        let mut conflict = entry.clone();
        conflict.context_payload_digest = "changed".into();
        assert!(store.unrecorded_memory_usage_entries(&[conflict]).is_err());
        let mut new_entry = entry;
        new_entry.usage_id = "new-usage".into();
        let pending = store.unrecorded_memory_usage_entries(&[new_entry.clone()]).unwrap();
        assert_eq!(pending, vec![new_entry]);
        assert!(crate::memory_store::apply_memory_usage_entries(&store.connection, &pending).is_err());
    }

    #[test]
    fn delivery_read_and_transition_use_the_callers_connection_and_outer_uow() {
        let mut store = NolaStore::open(":memory:").unwrap();
        store.begin_write_unit_of_work().unwrap();
        nola_store_port::run_creation::RunCreationRepository::begin_turn(
            &mut store,
            &BeginTurnInput {
                user_part_kind: "text".into(),
                expected_configuration_revision: None,
                expected_parent_message_id: None,
                initial_events: Vec::new(),
                initial_assistant_part_mutations: vec![],
                memory_usage_entries: vec![],
                delegated_invocation_mutations: vec![],
                delegated_permission_mutations: vec![],
                model_attempt_mutations: vec![],
                conversation_id: "conversation".into(),
                conversation_title: "fixture".into(),
                user_message_id: "user".into(),
                user_part_id: "user:text".into(),
                user_text: "hello".into(),
                user_content_json: None,
                additional_user_parts: vec![],
                assistant_message_id: "assistant".into(),
                assistant_part_id: "assistant:text".into(),
                assistant_turn_bindings: vec![],
                run_id: "run".into(),
                context_checkpoint_id: None,
                checkpoint_revision: 1,
                checkpoint_event_cursor: 0,
                checkpoint_json: "{}".into(),
                invocation_config_json: "{}".into(),
                invocation_config_digest: "fixture-config".into(),
                outbox_effects: vec![OutboxEffectInput {
                    delegated_invocation_id: None,
                    model_attempt_id: None,
                    effect_id: "effect".into(),
                    request_id: "request".into(),
                    effect_kind: "fixture".into(),
                    write: crate::outbox_fixture::tool("effect", "request", serde_json::json!({})),
                }],
                created_at_ms: 1,
            },
        )
        .unwrap();
        let delivery = RunEffectRepository::load_committed_host_delivery(&store, "run").unwrap();
        assert_eq!(delivery.conversation_id.as_deref(), Some("conversation"));
        assert_eq!(delivery.checkpoint.unwrap().revision, 1);
        assert_eq!(delivery.effects.len(), 1);
        assert!(store.has_write_unit_of_work());
        assert_eq!(store.pending_outbox_effect_ids("run").unwrap(), vec!["effect"]);
        assert!(store.pending_outbox_effect_ids("foreign-run").unwrap().is_empty());
        assert!(store.pending_outbox_effect_ids("").is_err());
        let large_checkpoint = serde_json::json!({
            "session_id":"core-session",
            "state": {
                "session_id":"core-session",
                "conversation":{"transcript":"private transcript ".repeat(20_000)},
                "execution":{"permissions":{"pending":[{
                    "request_id":"permission",
                    "arguments":{"private_input":"large tool input ".repeat(20_000)},
                    "continuation":{
                        "type":"delegated_agent", "target_effect_id":"parent", "backend_id":"backend",
                        "occurrence":{"occurrence_id":"occurrence", "worker_generation":"generation"}
                    }
                }]}},
                "host":{"pending_host_effects":{"effects":{"effect":{
                    "effect_id":"effect", "request_id":"request", "replay_safety":"recoverable",
                    "session_id":"core-session", "turn_id":"turn", "host_id":"host", "lease_id":"lease",
                    "catalog_revision":u64::MAX, "availability_epoch":u64::MAX,
                    "last_response_sequence":u64::MAX,
                    "request":{"messages":"frozen model request ".repeat(20_000),
                        "payload":{"type":"request_permission", "allow_conversation":false, "permission_request_id":"permission"}}
                }}}}
            }
        }).to_string();
        store.connection.execute("UPDATE core_checkpoint_bodies SET checkpoint_json=?1 WHERE conversation_id='conversation'",
            [&large_checkpoint]).unwrap();
        let witness = store.load_committed_host_delivery_witness("run").unwrap();
        assert_eq!(witness.conversation_id.as_deref(), Some("conversation"));
        assert_eq!(witness.effects.len(), 1);
        let witness = witness.checkpoint.unwrap();
        assert_eq!(witness.conversation_id, "conversation");
        assert!(witness.witness_json.len() < 1024);
        assert!(!witness.witness_json.contains("private transcript"));
        assert!(!witness.witness_json.contains("frozen model request"));
        assert!(!witness.witness_json.contains("large tool input"));
        let decoded: serde_json::Value = serde_json::from_str(&witness.witness_json).unwrap();
        assert_eq!(decoded["state"]["host"]["pending_host_effects"]["effects"]["effect"]["last_response_sequence"].as_u64(), Some(u64::MAX));
        let effect = &decoded["state"]["host"]["pending_host_effects"]["effects"]["effect"];
        assert_eq!(effect["catalog_revision"].as_u64(), Some(u64::MAX));
        assert_eq!(effect["availability_epoch"].as_u64(), Some(u64::MAX));
        assert_eq!(effect["session_id"], "core-session");
        assert_eq!(effect["turn_id"], "turn");
        assert_eq!(effect["host_id"], "host");
        assert_eq!(effect["lease_id"], "lease");
        assert_eq!(effect["permission_request_id"], "permission");
        let permission = &decoded["state"]["execution"]["permissions"]["pending"][0];
        assert_eq!(permission["request_id"], "permission");
        assert_eq!(permission["continuation"]["target_effect_id"], "parent");
        assert!(permission.get("arguments").is_none());
        let heartbeat = store.load_effect_heartbeat_snapshot("run", "effect").unwrap().unwrap();
        let candidates = store.pending_effect_delivery_candidates("run", None, 1).unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].effect_id, "effect");
        assert_eq!(candidates[0].request_id, "request");
        assert_eq!(candidates[0].run_id, "run");
        let after = nola_store_port::effect_delivery::EffectDeliveryCandidateCursor {
            created_at_ms: candidates[0].created_at_ms,
            effect_id: candidates[0].effect_id.clone(),
        };
        assert!(store.pending_effect_delivery_candidates("run", Some(&after), 1).unwrap().is_empty());
        assert_eq!(heartbeat.effect_id, "effect");
        assert_eq!(heartbeat.request_id, "request");
        assert_eq!(heartbeat.run_id, "run");
        assert_eq!(heartbeat.checkpoint.as_ref(), Some(&witness));
        let selected = ["effect".to_owned()].into_iter().collect();
        let selected_delivery = store.load_selected_host_delivery_witness("run", Some(&selected)).unwrap();
        assert_eq!(selected_delivery.effects.len(), 1);
        assert_eq!(selected_delivery.effects[0].effect_id, "effect");
        assert_eq!(selected_delivery.checkpoint.as_ref(), Some(&witness));
        let empty = std::collections::HashSet::new();
        let empty_delivery = store.load_selected_host_delivery_witness("run", Some(&empty)).unwrap();
        assert!(empty_delivery.effects.is_empty());
        assert!(empty_delivery.checkpoint.is_none(), "empty selection must not load a Run witness");
        assert!(store.load_selected_host_delivery_witness("foreign-run", Some(&selected)).unwrap().effects.is_empty());
        let excessive = (0..65).map(|index| format!("effect-{index}")).collect();
        assert!(store.load_selected_host_delivery_witness("run", Some(&excessive)).is_err());
        assert!(store.load_effect_heartbeat_snapshot("foreign-run", "effect").unwrap().is_none());
        assert!(store.load_effect_heartbeat_snapshot("run", "foreign-effect").unwrap().is_none());
        assert_eq!(store.load_checkpoint("conversation").unwrap().unwrap().checkpoint_json, large_checkpoint,
            "delivery reads must not redact or overwrite recovery state");
        assert_eq!(store.pending_outbox_effect_ids("run").unwrap(), vec!["effect"],
            "settlement identities must be independent of checkpoint and request body size");
        assert!(store.has_write_unit_of_work());
        store.connection.execute_batch("SAVEPOINT delivery_witness_rollback;
            UPDATE core_checkpoints SET revision=2, event_cursor=1 WHERE conversation_id='conversation';
            UPDATE core_checkpoint_bodies SET checkpoint_json='{}'
            WHERE conversation_id='conversation';").unwrap();
        let changed = store.load_committed_host_delivery_witness("run").unwrap().checkpoint.unwrap();
        assert_ne!(changed.witness_json, witness.witness_json);
        store.connection.execute_batch("ROLLBACK TO delivery_witness_rollback; RELEASE delivery_witness_rollback;").unwrap();
        assert_eq!(store.load_committed_host_delivery_witness("run").unwrap().checkpoint.unwrap(), witness);
        assert_eq!(store.load_effect_heartbeat_snapshot("run", "effect").unwrap().unwrap(), heartbeat);
        let committed_input = CommitTransitionInput {
                memory_usage_entries: vec![],
                delegated_invocation_mutations: vec![],
                delegated_permission_mutations: vec![],
                effect_delivery_attempt_mutations: vec![],
                model_attempt_mutations: vec![],
                transition_id: "transition".into(),
                run_id: "run".into(),
                expected_checkpoint_revision: 1,
                checkpoint_revision: 2,
                checkpoint_event_cursor: 1,
                checkpoint: nola_store_port::run_effect::CheckpointWrite::Baseline { checkpoint_json: "{}".into() },
                submission_reduction_json: None, host_event: Some(HostEventInput {
                    event_id: "host-event".into(), effect_id: "effect".into(), request_id: "request".into(),
                    sequence: 1, payload_json: "{\"delta\":\"committed\"}".into(), received_at_ms: 2,
                    reduction_json: None,
                }),
                events: vec![RunEventInput {
                    event_id: "event".into(),
                    sequence: 1,
                    event_kind: "fixture".into(),
                    payload_json: "{}".into(),
                    created_at_ms: 2,
                }],
                ensured_messages: vec![],
                assistant_turn_bindings: vec![],
                part_message_owners: Default::default(),
                part_mutations: vec![],
                outbox_effects: vec![],
                acknowledged_effect_ids: vec!["effect".into()],
                run_status: None,
                assistant_status: None,
                assistant_status_message_id: None,
                committed_at_ms: 2,
            };
        let history = [("parallel-a", 10), ("parallel-b", 1)].into_iter().map(|(effect, sequence)| HostEventInput {
            event_id: format!("history-{effect}"), effect_id: effect.into(), request_id: format!("request-{effect}"),
            sequence, payload_json: "{}".into(), reduction_json: None, received_at_ms: 1,
        }).collect::<Vec<_>>();
        store.append_host_event_history("run", &history).unwrap();
        store.append_host_event_history("run", &history).unwrap();
        let mut order_query = store.connection.prepare(
            "SELECT h.effect_id,o.ordinal FROM host_event_replay_order o JOIN host_event_inbox h USING(event_id)
             WHERE o.run_id='run' ORDER BY o.ordinal").unwrap();
        let ordered = order_query.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)))
            .unwrap().collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(ordered, vec![("parallel-a".into(), 1), ("parallel-b".into(), 2)],
            "cross-effect arrival order must not be sorted by per-effect sequence or duplicated on retry");
        drop(order_query);
        store.connection.execute_batch("SAVEPOINT submission_replay_contract;").unwrap();
        let mut submission = committed_input.clone();
        submission.transition_id = "submission-transition".into();
        submission.host_event = None;
        let facts = SubmissionReplayReduction {
            protocol_version: 1, session_id: "core-session".into(), decision_time_ms: 2,
            submission_json: serde_json::json!({"type":"interrupt","reason":"fixture"}).to_string(),
            bindings: vec![],
        };
        submission.submission_reduction_json = Some(serde_json::to_string(&facts).unwrap());
        for invalid in [
            SubmissionReplayReduction { session_id: "foreign-session".into(), ..facts.clone() },
            SubmissionReplayReduction { decision_time_ms: 3, ..facts.clone() },
            SubmissionReplayReduction { submission_json: "{\"type\":\"host_response\"}".into(), ..facts.clone() },
        ] {
            let mut rejected = submission.clone();
            rejected.submission_reduction_json = Some(serde_json::to_string(&invalid).unwrap());
            assert!(store.commit_transition(&rejected).is_err());
        }
        assert!(store.commit_transition(&submission).unwrap().applied);
        assert!(!store.commit_transition(&submission).unwrap().applied,
            "exact transaction retry must not append another reducer input");
        let mut conflicting = submission.clone();
        conflicting.submission_reduction_json = None;
        assert!(store.commit_transition(&conflicting).is_err(), "retry cannot erase its input identity");
        let count: i64 = store.connection.query_row(
            "SELECT count(*) FROM host_event_replay_order WHERE submission_id='submission-transition'",
            [], |row| row.get(0)).unwrap();
        assert_eq!(count, 1);
        store.connection.execute_batch("ROLLBACK TO submission_replay_contract; RELEASE submission_replay_contract;").unwrap();
        assert_eq!(store.pending_outbox_effect_ids("run").unwrap(), vec!["effect"]);
        RunEffectRepository::commit_transition(&mut store, &committed_input).unwrap();
        assert!(store.pending_outbox_effect_ids("run").unwrap().is_empty(),
            "settlement must be visible before the caller commits its outer transaction");
        let recovery = store.load_recovery_checkpoint("conversation").unwrap().unwrap();
        let base = recovery.replay_base.as_ref().unwrap();
        assert_eq!(base.run_id, "run");
        assert_eq!(base.checkpoint_revision, recovery.checkpoint.revision);
        assert_eq!(base.through_ordinal, 3);
        for frontier in [2, 4] {
            store.connection.execute_batch("SAVEPOINT inconsistent_replay_frontier;").unwrap();
            store.connection.execute(
                "UPDATE host_event_replay_heads SET ordinal=?1 WHERE run_id='run'", [frontier],
            ).unwrap();
            assert!(store.load_recovery_checkpoint("conversation").is_err(),
                "recovery must reject both a regressed head and an unapplied durable tail");
            store.connection.execute_batch(
                "ROLLBACK TO inconsistent_replay_frontier; RELEASE inconsistent_replay_frontier;",
            ).unwrap();
        }
        store.connection.execute_batch("SAVEPOINT recovery_tail;").unwrap();
        let tail_json = serde_json::json!({
            "request_id":"tail-request", "effect_id":"tail-effect", "host_id":"host",
            "session_id":"core-session", "turn_id":"turn", "catalog_revision":1,
            "availability_epoch":1, "sequence":1, "payload":{"type":"accepted"}
        }).to_string();
        let tail_fact=serde_json::to_string(&nola_store_port::host_replay::HostReplayReduction::Replayable {
            protocol_version:1,decision_time_ms:7,binding:None,
            normalized_response:serde_json::value::RawValue::from_string(tail_json.clone()).unwrap(),
        }).unwrap();
        store.append_host_event_history("run", &[HostEventInput {
            event_id: "tail-event".into(), effect_id: "tail-effect".into(), request_id: "tail-request".into(),
            sequence: 1, payload_json: tail_json, reduction_json: Some(tail_fact.clone()), received_at_ms: 7,
        }]).unwrap();
        let with_tail = store.load_recovery_checkpoint("conversation").unwrap().unwrap();
        assert_eq!(with_tail.replay_base, recovery.replay_base);
        assert_eq!(with_tail.replay_inputs, vec![StoredCoreReplayInput { kind: nola_store_port::run_effect::ReplayInputKind::HostResponse,
            ordinal: 4, reduction_json: tail_fact, received_at_ms: 7,
        }]);
        store.connection.execute("UPDATE host_event_inbox SET reduction_json=NULL WHERE event_id='tail-event'", []).unwrap();
        assert!(store.load_recovery_checkpoint("conversation").is_err(),
            "legacy wire payload cannot substitute for the exact normalized reducer input");
        store.connection.execute_batch("ROLLBACK TO recovery_tail; RELEASE recovery_tail;").unwrap();
        assert_eq!(store.load_recovery_checkpoint("conversation").unwrap().unwrap(), recovery);
        store.connection.execute_batch("SAVEPOINT unscoped_checkpoint;
            UPDATE core_checkpoint_bodies SET checkpoint_json=checkpoint_json WHERE conversation_id='conversation';").unwrap();
        assert!(store.load_recovery_checkpoint("conversation").unwrap().unwrap().replay_base.is_none(),
            "an unscoped checkpoint replacement cannot inherit the prior Run replay binding");
        store.connection.execute_batch("ROLLBACK TO unscoped_checkpoint; RELEASE unscoped_checkpoint;").unwrap();
        assert_eq!(store.load_recovery_checkpoint("conversation").unwrap().unwrap(), recovery);
        for new_transition in [false, true] {
            let mut conflicting = committed_input.clone();
            if new_transition { conflicting.transition_id = "other-transition".into(); }
            conflicting.host_event.as_mut().unwrap().payload_json = "{\"delta\":\"forged\"}".into();
            assert!(RunEffectRepository::commit_transition(&mut store, &conflicting).is_err());
        }
        let mut replay = committed_input.clone();
        let mut changed_reduction = committed_input.clone();
        changed_reduction.host_event.as_mut().unwrap().reduction_json = Some("{}".into());
        assert!(RunEffectRepository::commit_transition(&mut store, &changed_reduction).is_err(),
            "a replay may not replace the recorded reducer input");
        replay.transition_id = "same-host-event-new-transaction".into();
        replay.host_event.as_mut().unwrap().received_at_ms = 3;
        let replayed = RunEffectRepository::commit_transition(&mut store, &replay).unwrap();
        assert!(!replayed.applied);
        assert_eq!(replayed.checkpoint_revision, 2, "return the durable revision, not the stale input expectation");
        assert_eq!(
            RunEffectRepository::checkpoint_revision(&store, "conversation").unwrap(),
            Some(2)
        );
        assert!(
            RunEffectRepository::load_committed_host_delivery(&store, "run")
                .unwrap()
                .effects
                .is_empty()
        );
        assert!(store.has_write_unit_of_work());
        store.rollback_write_unit_of_work().unwrap();
        assert!(store.pending_outbox_effect_ids("run").unwrap().is_empty(),
            "rolling back admission must not leave pending settlement identities");
        let witness_count: i64 = store.connection.query_row(
            "SELECT count(*) FROM core_delivery_witnesses", [], |row| row.get(0)).unwrap();
        assert_eq!(witness_count, 0, "rolled-back checkpoint cannot leave an admission witness");
        let replay_heads: i64 = store.connection.query_row(
            "SELECT count(*) FROM host_event_replay_heads", [], |row| row.get(0)).unwrap();
        assert_eq!(replay_heads, 0, "replay ordering must roll back with its input history");
        assert!(store.load_conversation("conversation").unwrap().is_none());
        assert!(RunEffectRepository::load_checkpoint(&store, "conversation")
            .unwrap()
            .is_none());
        assert!(RunEffectRepository::outbox_effect(&store, "effect")
            .unwrap()
            .is_none());
        assert!(store.connection.is_autocommit());
        assert!(
            RunEffectRepository::load_committed_host_delivery(&store, "run")
                .unwrap()
                .effects
                .is_empty()
        );
        assert!(
            store.connection.is_autocommit(),
            "standalone read snapshot was released"
        );
    }
}

impl nola_store_port::run_creation::RunCreationRepository for NolaStore {
    fn ui_surfaces_for_context(
        &self,
        context_id: &str,
    ) -> Result<Vec<nola_domain::generative_ui::UiSurfaceState>, Self::Error> {
        self.context_ui_surfaces(context_id)
    }
    fn internal_assistant_target(
        &self,
        conversation_id: &str,
    ) -> Result<Option<nola_store_port::assistant_ownership::AssistantOwner>, Self::Error> {
        NolaStore::internal_assistant_target(self, conversation_id)
    }
    fn begin_turn(
        &mut self,
        input: &crate::BeginTurnInput,
    ) -> Result<crate::BeginTurnResult, Self::Error> {
        NolaStore::begin_turn(self, input)
    }
    fn begin_internal_turn(
        &mut self,
        input: &crate::BeginInternalTurnInput,
    ) -> Result<crate::BeginTurnResult, Self::Error> {
        NolaStore::begin_internal_turn(self, input)
    }
    fn begin_successor_turn(
        &mut self,
        input: &crate::BeginSuccessorTurnInput,
    ) -> Result<crate::BeginTurnResult, Self::Error> {
        NolaStore::begin_successor_turn(self, input)
    }
    fn begin_maintenance_run(
        &mut self,
        input: &crate::BeginMaintenanceRunInput,
    ) -> Result<crate::BeginMaintenanceRunResult, Self::Error> {
        NolaStore::begin_maintenance_run(self, input)
    }
}
