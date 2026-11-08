// SPDX-License-Identifier: Apache-2.0
//! Conversation checkpoint persistence, memory-linked commit, revision lookup, and deletion.

use super::*;

/// A freshly admitted Run can replay from its initial checkpoint immediately;
/// the first Host response must not create a second full baseline merely to
/// establish ownership. Only the Run-creation transaction may use this gate.
pub(crate) fn bind_initial_run_baseline(
    connection: &Connection, conversation_id: &str, run_id: &str, revision: i64,
) -> StoreResult<()> {
    if connection.is_autocommit() {
        return Err(StoreError::Invariant("Run baseline binding requires admission transaction".into()));
    }
    let inserted = connection.execute(
        "INSERT INTO core_checkpoint_replay_bases
         (conversation_id,run_id,checkpoint_revision,through_ordinal,through_reduction_bytes)
         SELECT c.conversation_id,r.id,c.revision,0,0
         FROM runs r JOIN core_checkpoints c ON c.conversation_id=r.conversation_id
         JOIN core_delivery_witnesses w ON w.conversation_id=c.conversation_id
             AND w.checkpoint_revision=c.revision AND w.event_cursor=c.event_cursor
         WHERE r.id=?1 AND c.conversation_id=?2 AND c.revision=?3
             AND NOT EXISTS(SELECT 1 FROM host_event_replay_heads WHERE run_id=r.id)",
        params![run_id, conversation_id, revision],
    )?;
    if inserted != 1 {
        return Err(StoreError::Invariant("Run admission has no exact fresh replay baseline".into()));
    }
    Ok(())
}

#[cfg(test)]
mod body_tests {
    use super::*;

    #[test]
    fn baseline_header_body_and_witness_share_rollback_and_missing_body_fails_closed() {
        let directory = tempfile::tempdir().unwrap();
        let mut store = NolaStore::open(directory.path().join("rollback.db")).unwrap();
        store
            .connection
            .execute("INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms) VALUES('c','fixture',1,1,1)", [])
            .unwrap();
        let json = r#"{"session_id":"c","state":{"session_id":"c"}}"#;
        {
            let tx = store.write_transaction().unwrap();
            write_baseline(&tx, "c", 1, 3, json, 1).unwrap();
            tx.commit().unwrap();
        }
        {
            let tx = store.write_transaction().unwrap();
            write_baseline(&tx, "c", 2, 4, "{}", 2).unwrap();
            tx.rollback().unwrap();
        }
        let baseline = store.load_checkpoint("c").unwrap().unwrap();
        assert_eq!((baseline.revision, baseline.event_cursor), (1, 3));
        assert_eq!(baseline.checkpoint_json, json);
        let witness:(i64,i64) = store.connection.query_row(
            "SELECT checkpoint_revision,event_cursor FROM core_delivery_witnesses WHERE conversation_id='c'",
            [],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
        assert_eq!(witness, (1, 3));
        store
            .connection
            .execute(
                "DELETE FROM core_checkpoint_bodies WHERE conversation_id='c'",
                [],
            )
            .unwrap();
        assert!(
            store.load_checkpoint("c").is_err(),
            "a corrupt baseline is not an absent session"
        );
        assert!(crate::run_effect_port::read_recovery_checkpoint(&store.connection, "c").is_err());
        assert_eq!(
            store
                .connection
                .query_row("SELECT count(*) FROM core_delivery_witnesses", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        {
            let tx = store.write_transaction().unwrap();
            write_baseline(&tx, "c", 2, 4, json, 2).unwrap();
            tx.commit().unwrap();
        }
        store
            .connection
            .execute("DELETE FROM core_checkpoints WHERE conversation_id='c'", [])
            .unwrap();
        for table in [
            "core_checkpoint_bodies",
            "core_delivery_witnesses",
            "core_checkpoint_replay_bases",
        ] {
            assert_eq!(
                store
                    .connection
                    .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                        .get::<_, i64>(0))
                    .unwrap(),
                0
            );
        }
    }

}

/// Caller owns the admission/transition transaction and has checked its CAS.
/// Header first, body second: body triggers derive the witness from the new
/// revision, while replay-only header writes never parse or rewrite the body.
pub(super) fn write_baseline(
    connection: &Connection,
    conversation_id: &str,
    revision: i64,
    event_cursor: i64,
    checkpoint_json: &str,
    updated_at_ms: i64,
) -> StoreResult<()> {
    if connection.is_autocommit() {
        return Err(StoreError::Invariant(
            "checkpoint baseline requires a unit of work".into(),
        ));
    }
    connection.execute(
        "INSERT INTO core_checkpoints(conversation_id,revision,event_cursor,updated_at_ms)
         VALUES(?1,?2,?3,?4) ON CONFLICT(conversation_id) DO UPDATE SET
         revision=excluded.revision,event_cursor=excluded.event_cursor,updated_at_ms=excluded.updated_at_ms",
        params![conversation_id,revision,event_cursor,updated_at_ms],
    )?;
    connection.execute(
        "INSERT INTO core_checkpoint_bodies(conversation_id,checkpoint_json) VALUES(?1,?2)
         ON CONFLICT(conversation_id) DO UPDATE SET checkpoint_json=excluded.checkpoint_json",
        params![conversation_id, checkpoint_json],
    )?;
    Ok(())
}

impl NolaStore {
    /// Saves a Core checkpoint with optimistic concurrency control. This is
    /// used for durable idle/bootstrap checkpoints that are not part of a run
    /// transition; active-run transitions should use `commit_transition` so
    /// events, parts, checkpoint and outbox effects remain atomic.
    pub fn save_checkpoint(
        &mut self,
        input: &SaveCheckpointInput,
    ) -> StoreResult<StoredCheckpoint> {
        require_id("checkpoint.conversation_id", &input.conversation_id)?;
        validate_json("checkpoint.checkpoint_json", &input.checkpoint_json)?;
        if input.revision <= input.expected_revision {
            return Err(StoreError::Invariant(
                "checkpoint revision must advance".to_string(),
            ));
        }
        let transaction = self.write_transaction()?;
        let conversation_exists = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM conversations WHERE id = ?1)",
            params![input.conversation_id],
            |row| row.get::<_, bool>(0),
        )?;
        if !conversation_exists {
            return Err(StoreError::Invariant(format!(
                "unknown conversation {}",
                input.conversation_id
            )));
        }
        let stored_revision = transaction
            .query_row(
                "SELECT revision FROM core_checkpoints WHERE conversation_id = ?1",
                params![input.conversation_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()?
            .unwrap_or(0);
        if stored_revision != input.expected_revision {
            return Err(StoreError::Invariant(format!(
                "checkpoint revision conflict: expected {}, found {stored_revision}",
                input.expected_revision
            )));
        }
        write_baseline(
            &transaction,
            &input.conversation_id,
            input.revision,
            input.event_cursor,
            &input.checkpoint_json,
            input.updated_at_ms,
        )?;
        transaction.commit()?;
        Ok(StoredCheckpoint {
            conversation_id: input.conversation_id.clone(),
            revision: input.revision,
            event_cursor: input.event_cursor,
            checkpoint_json: input.checkpoint_json.clone(),
            updated_at_ms: input.updated_at_ms,
        })
    }

    /// Atomically commits an idle Core checkpoint and the semantic memories
    /// accepted by that same Core transition. Callers must not publish either
    /// side independently: a checkpoint that marks a memory job completed and
    /// missing memory rows describe two different product states.
    pub fn save_checkpoint_with_memories(
        &mut self,
        input: &SaveCheckpointInput,
        memories: &[StoredMemoryRecord],
        expected_memories: &[(String, i64)],
        memory_usage_entries: &[StoredMemoryUsageEntry],
    ) -> StoreResult<(StoredCheckpoint, Vec<StoredMemoryRecord>)> {
        require_id("checkpoint.conversation_id", &input.conversation_id)?;
        validate_json("checkpoint.checkpoint_json", &input.checkpoint_json)?;
        if input.revision <= input.expected_revision {
            return Err(StoreError::Invariant(
                "checkpoint revision must advance".to_string(),
            ));
        }
        for memory in memories {
            validate_memory_record(memory)?;
        }
        let transaction = self.write_transaction()?;
        for (id, expected_revision) in expected_memories {
            let actual = transaction
                .query_row(
                    "SELECT revision FROM memory_records WHERE id=?1 AND deleted_at_ms IS NULL",
                    params![id],
                    |row| row.get::<_, i64>(0),
                )
                .optional()?;
            if actual != Some(*expected_revision) {
                return Err(StoreError::Conflict(format!(
                    "memory_input_conflict: memory {id} changed before checkpoint commit"
                )));
            }
        }
        let conversation_exists = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM conversations WHERE id = ?1)",
            params![input.conversation_id],
            |row| row.get::<_, bool>(0),
        )?;
        if !conversation_exists {
            return Err(StoreError::Invariant(format!(
                "unknown conversation {}",
                input.conversation_id
            )));
        }
        let stored_revision = transaction
            .query_row(
                "SELECT revision FROM core_checkpoints WHERE conversation_id = ?1",
                params![input.conversation_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()?
            .unwrap_or(0);
        if stored_revision != input.expected_revision {
            return Err(StoreError::Invariant(format!(
                "checkpoint revision conflict: expected {}, found {stored_revision}",
                input.expected_revision
            )));
        }
        write_baseline(
            &transaction,
            &input.conversation_id,
            input.revision,
            input.event_cursor,
            &input.checkpoint_json,
            input.updated_at_ms,
        )?;
        let mut stored_memories = Vec::with_capacity(memories.len());
        for memory in memories {
            // Core has already applied extraction/consolidation and committed
            // this identity to its memory projection. Store must preserve that
            // identity; content-level deduplication here would make the
            // checkpoint and the durable memory projection disagree.
            let previous = validate_memory_candidate_identity(&transaction, memory)?;
            let mut canonical = memory.clone();
            if let Some(previous) = previous {
                canonical.last_used_at_ms = canonical.last_used_at_ms.max(previous.last_used_at_ms);
            }
            stored_memories.push(upsert_memory_projection(&transaction, &canonical)?);
        }
        apply_memory_usage_entries(&transaction, memory_usage_entries)?;
        transaction.commit()?;
        if self.has_write_unit_of_work() && self.active_run(&input.conversation_id)?.is_none() {
            self.applied_idle_checkpoint_stamps.insert(
                (input.conversation_id.clone(), input.revision),
                input.event_cursor,
            );
        }
        Ok((
            StoredCheckpoint {
                conversation_id: input.conversation_id.clone(),
                revision: input.revision,
                event_cursor: input.event_cursor,
                checkpoint_json: input.checkpoint_json.clone(),
                updated_at_ms: input.updated_at_ms,
            },
            stored_memories,
        ))
    }

    pub fn delete_checkpoint(&mut self, conversation_id: &str) -> StoreResult<bool> {
        require_id("checkpoint.conversation_id", conversation_id)?;
        Ok(self.connection.execute(
            "DELETE FROM core_checkpoints WHERE conversation_id = ?1",
            params![conversation_id],
        )? == 1)
    }
}
