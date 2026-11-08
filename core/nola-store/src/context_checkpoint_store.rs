// SPDX-License-Identifier: Apache-2.0
//! Context checkpoint and component persistence, counts, and reconstruction.

use super::*;

impl NolaStore {
    pub fn commit_context_checkpoint(
        &mut self,
        input: &CommitContextCheckpointInput,
    ) -> StoreResult<()> {
        require_id("context_checkpoint.id", &input.checkpoint.id)?;
        require_id(
            "context_checkpoint.conversation_id",
            &input.checkpoint.conversation_id,
        )?;
        require_id(
            "context_checkpoint.model_profile_id",
            &input.checkpoint.model_profile_id,
        )?;
        for component in &input.components {
            require_id("context_component.id", &component.id)?;
            if component.checkpoint_id != input.checkpoint.id {
                return Err(StoreError::Invariant(
                    "context component checkpoint identity mismatch".to_string(),
                ));
            }
            validate_json(
                "context_component.content_reference_json",
                &component.content_reference_json,
            )?;
            validate_json(
                "context_component.provenance_json",
                &component.provenance_json,
            )?;
        }
        let checkpoint = &input.checkpoint;
        let transaction = self.write_transaction()?;
        let changed = transaction.execute(
            "INSERT INTO context_checkpoints(
                id, conversation_id, run_id, model_profile_id, policy_version,
                covered_through_message_id, covered_through_sequence, summary_part_id,
                retained_tail_start_message_id, estimated_tokens, measured_tokens,
                context_window_tokens, prompt_limit_tokens, payload_digest, created_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
             ON CONFLICT(id) DO UPDATE SET
                run_id = excluded.run_id,
                model_profile_id = excluded.model_profile_id,
                policy_version = excluded.policy_version,
                covered_through_message_id = excluded.covered_through_message_id,
                covered_through_sequence = excluded.covered_through_sequence,
                summary_part_id = excluded.summary_part_id,
                retained_tail_start_message_id = excluded.retained_tail_start_message_id,
                estimated_tokens = excluded.estimated_tokens,
                measured_tokens = excluded.measured_tokens,
                context_window_tokens = excluded.context_window_tokens,
                prompt_limit_tokens = excluded.prompt_limit_tokens,
                payload_digest = excluded.payload_digest,
                created_at_ms = excluded.created_at_ms
             WHERE context_checkpoints.conversation_id = excluded.conversation_id",
            params![
                checkpoint.id,
                checkpoint.conversation_id,
                checkpoint.run_id,
                checkpoint.model_profile_id,
                checkpoint.policy_version,
                checkpoint.covered_through_message_id,
                checkpoint.covered_through_sequence,
                checkpoint.summary_part_id,
                checkpoint.retained_tail_start_message_id,
                checkpoint.estimated_tokens,
                checkpoint.measured_tokens,
                checkpoint.context_window_tokens,
                checkpoint.prompt_limit_tokens,
                checkpoint.payload_digest,
                checkpoint.created_at_ms
            ],
        )?;
        if changed != 1 {
            return Err(StoreError::Conflict(
                "context checkpoint identity belongs to another conversation".into(),
            ));
        }
        transaction.execute(
            "DELETE FROM context_components WHERE checkpoint_id = ?1",
            params![checkpoint.id],
        )?;
        for component in &input.components {
            transaction.execute(
                "INSERT INTO context_components(
                    id, checkpoint_id, kind, source_id, source_uri, title, mime_type,
                    authorization, observed_at_ms, maximum_age_minutes, freshness,
                    content_reference_json, content_digest, estimated_tokens, provenance_json
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
                params![
                    component.id,
                    component.checkpoint_id,
                    component.kind,
                    component.source_id,
                    component.source_uri,
                    component.title,
                    component.mime_type,
                    component.authorization,
                    component.observed_at_ms,
                    component.maximum_age_minutes,
                    component.freshness,
                    component.content_reference_json,
                    component.content_digest,
                    component.estimated_tokens,
                    component.provenance_json,
                ],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn count_context_checkpoints(&self, conversation_id: &str) -> StoreResult<u64> {
        require_id("conversation.id", conversation_id)?;
        let count: i64 = self.connection.query_row(
            "SELECT COUNT(*) FROM context_checkpoints WHERE conversation_id=?1",
            params![conversation_id],
            |row| row.get(0),
        )?;
        u64::try_from(count)
            .map_err(|_| StoreError::Invariant("invalid context checkpoint count".into()))
    }

    pub fn load_context_checkpoint(
        &self,
        checkpoint_id: &str,
    ) -> StoreResult<Option<ContextAggregateRecord>> {
        require_id("context_checkpoint.id", checkpoint_id)?;
        let checkpoint = self
            .connection
            .query_row(
                "SELECT id, conversation_id, run_id, model_profile_id, policy_version,
                        covered_through_message_id, covered_through_sequence, summary_part_id,
                        retained_tail_start_message_id, estimated_tokens, measured_tokens,
                        context_window_tokens, prompt_limit_tokens, payload_digest, created_at_ms
                 FROM context_checkpoints WHERE id = ?1",
                params![checkpoint_id],
                |row| {
                    Ok(ContextCheckpointInput {
                        id: row.get(0)?,
                        conversation_id: row.get(1)?,
                        run_id: row.get(2)?,
                        model_profile_id: row.get(3)?,
                        policy_version: row.get(4)?,
                        covered_through_message_id: row.get(5)?,
                        covered_through_sequence: row.get(6)?,
                        summary_part_id: row.get(7)?,
                        retained_tail_start_message_id: row.get(8)?,
                        estimated_tokens: row.get(9)?,
                        measured_tokens: row.get(10)?,
                        context_window_tokens: row.get(11)?,
                        prompt_limit_tokens: row.get(12)?,
                        payload_digest: row.get(13)?,
                        created_at_ms: row.get(14)?,
                    })
                },
            )
            .optional()?;
        checkpoint
            .map(|checkpoint| {
                let components = self.load_context_components(&checkpoint.id)?;
                Ok(ContextAggregateRecord {
                    checkpoint,
                    components,
                })
            })
            .transpose()
    }

    fn load_context_components(
        &self,
        checkpoint_id: &str,
    ) -> StoreResult<Vec<ContextComponentInput>> {
        let mut statement = self.connection.prepare(
            "SELECT id, checkpoint_id, kind, source_id, source_uri, title, mime_type,
                    authorization, observed_at_ms, maximum_age_minutes, freshness,
                    content_reference_json, content_digest, estimated_tokens, provenance_json
             FROM context_components WHERE checkpoint_id = ?1 ORDER BY id ASC",
        )?;
        let rows = statement.query_map(params![checkpoint_id], |row| {
            Ok(ContextComponentInput {
                id: row.get(0)?,
                checkpoint_id: row.get(1)?,
                kind: row.get(2)?,
                source_id: row.get(3)?,
                source_uri: row.get(4)?,
                title: row.get(5)?,
                mime_type: row.get(6)?,
                authorization: row.get(7)?,
                observed_at_ms: row.get(8)?,
                maximum_age_minutes: row.get(9)?,
                freshness: row.get(10)?,
                content_reference_json: row.get(11)?,
                content_digest: row.get(12)?,
                estimated_tokens: row.get(13)?,
                provenance_json: row.get(14)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }
}
