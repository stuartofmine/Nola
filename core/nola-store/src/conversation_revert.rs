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
