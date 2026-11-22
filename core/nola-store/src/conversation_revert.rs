// SPDX-License-Identifier: Apache-2.0
//! Resolve canonical revert boundaries without materializing any original body.
use crate::{params, NolaStore, OptionalExtension, StoreError, StoreResult};
use nola_store_port::conversation_revert::{
    ConversationRevertBoundary, ConversationRevertRepository,
};

const VISIBLE: &str = "COALESCE((SELECT e.visible FROM message_projection_events e
    JOIN conversation_projection_operations o ON o.operation_id=e.operation_id
    WHERE e.conversation_id=m.conversation_id AND e.message_id=m.id
    ORDER BY o.resulting_revision DESC LIMIT 1),1)=1";
impl ConversationRevertRepository for NolaStore {
    type Error = StoreError;
    fn prepared_in_place_resend(&self, operation_id: &str) -> StoreResult<Option<nola_domain::edited_resend::PreparedInPlaceResend>> {
        crate::edited_resend::read(&self.connection, &format!("edited_resend/in_place/{operation_id}"))
    }
    fn put_prepared_in_place_resend(&mut self, prepared: &nola_domain::edited_resend::PreparedInPlaceResend) -> StoreResult<()> {
        if !self.has_write_unit_of_work() {
            return Err(StoreError::Conflict("in-place resend requires a transaction".into()));
        }
        crate::edited_resend::write_immutable(&self.connection,
            &format!("edited_resend/in_place/{}", prepared.input.operation_id), prepared, prepared.input.created_at_ms)
    }
    fn delegated_expert_rewind_points(
        &self,
        parent: &str,
        boundary_ordinal: i64,
    ) -> StoreResult<Vec<(String, String)>> {
        // Use the visible parent Run, not timestamps or display names. A previous
        // edit can leave sealed Tasks and their immutable bindings in the ledger.
        let sql = format!(
            "SELECT t.id, t.child_session_id, t.status FROM durable_tasks t
             WHERE t.owner_session_id=?1 AND t.kind='agent'
               AND json_type(t.metadata_json, '$.expert_target')='object'
               AND EXISTS (SELECT 1 FROM messages m WHERE m.conversation_id=?1
                   AND m.ordinal >= ?2 AND {VISIBLE}
                   AND m.run_id=json_extract(t.metadata_json, '$.expert_target.source_run_id'))"
        );
        let mut tasks = self.connection.prepare(&sql)?;
        let mut child_message = self.connection.prepare(&format!(
            "SELECT m.ordinal,m.id FROM runtime_task_run_bindings b JOIN runs r ON r.id=b.run_id
             JOIN messages m ON m.id=r.user_message_id AND m.run_id=r.id
             WHERE b.task_id=?1 AND b.owner_session_id=?2 AND b.child_session_id=?3
               AND r.conversation_id=?3 AND m.conversation_id=?3 AND {VISIBLE}"
        ))?;
        let mut first = std::collections::BTreeMap::<String, (i64, String)>::new();
        let rows = tasks.query_map(params![parent, boundary_ordinal], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?))
        })?;
        for row in rows {
            let (id, child, status) = row?;
            if !matches!(status.as_str(), "sealed" | "failed" | "cancelled" | "completed") {
                return Err(StoreError::Conflict("Wait for the expert task to finish before editing this message.".into()));
            }
            let point: Option<(i64, String)> = child_message
                .query_row(params![id, parent, child], |r| Ok((r.get(0)?, r.get(1)?)))
                .optional()?;
            if let Some(point) = point {
                let pin = self.validated_expert_task_target(&id, &child, true)?
                    .ok_or_else(|| StoreError::Conflict("Missing expert task ownership".into()))?;
                if !pin.source.is_main_agent() || pin.parent_conversation_id != parent {
                    return Err(StoreError::Conflict("Expert task parent differs from edit owner".into()));
                }
                let earliest = first.entry(child).or_insert_with(|| point.clone());
                if point.0 < earliest.0 { *earliest = point; }
            }
        }
        Ok(first.into_iter().map(|(child, (_, message))| (child, message)).collect())
    }
    fn commit_checkpoint_projection(
        &mut self,
        input: &nola_store_port::conversation_revert::CommitCheckpointProjectionInput,
    ) -> StoreResult<nola_store_port::conversation_revert::CommitCheckpointProjectionResult> {
        NolaStore::commit_checkpoint_projection(self, input)
    }
    fn conversation_revert_boundary(
        &self,
        conversation_id: &str,
        through_message_id: &str,
        keep_message: bool,
    ) -> StoreResult<ConversationRevertBoundary> {
        let _snapshot = crate::read_snapshot::pin(&self.connection)?;
        let (conversation_revision, boundary_ordinal): (i64, i64) = self.connection.query_row(
            &format!("SELECT c.revision,m.ordinal FROM conversations c JOIN messages m ON m.conversation_id=c.id WHERE c.id=?1 AND m.id=?2 AND {VISIBLE}"),
            params![conversation_id, through_message_id], |row| Ok((row.get(0)?, row.get(1)?)),
        ).optional()?.ok_or_else(|| StoreError::Conflict("conversation revert boundary is not visible".into()))?;
        let parent_message_id = if keep_message {
            Some(through_message_id.to_owned())
        } else {
            self.connection.query_row(&format!("SELECT m.id FROM messages m WHERE m.conversation_id=?1 AND m.ordinal<?2 AND {VISIBLE} ORDER BY m.ordinal DESC LIMIT 1"),
                params![conversation_id, boundary_ordinal], |row| row.get(0)).optional()?
        };
        let comparison = if keep_message { ">" } else { ">=" };
        let mut statement = self.connection.prepare(&format!("SELECT m.id FROM messages m WHERE m.conversation_id=?1 AND m.ordinal {comparison} ?2 AND {VISIBLE} ORDER BY m.ordinal"))?;
        let hidden_message_ids = statement
            .query_map(params![conversation_id, boundary_ordinal], |row| row.get(0))?
            .collect::<Result<Vec<String>, _>>()?;
        Ok(ConversationRevertBoundary {
            conversation_revision,
            boundary_ordinal,
            parent_message_id,
            hidden_message_ids,
        })
    }
}
