// SPDX-License-Identifier: Apache-2.0
//! Conversation removal and sync publication share a single write transaction.
use super::{enqueue_sync_change, params, NolaStore, OptionalExtension, StoreError, StoreResult};

pub(crate) fn require_inactive_conversation(
    connection: &rusqlite::Connection,
    conversation_id: &str,
) -> StoreResult<()> {
    crate::agent_room_admission::require_unowned_context(connection, conversation_id)?;
    let active: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM runs WHERE conversation_id=?1
         AND status NOT IN ('sealed'))",
        params![conversation_id], |row| row.get(0),
    )?;
    if active {
        return Err(StoreError::Conflict(
            "cannot delete or replace a conversation while its Run is active".into(),
        ));
    }
    Ok(())
}

impl NolaStore {
    pub fn delete_conversation(&mut self, conversation_id: &str) -> StoreResult<bool> {
        let transaction = self.write_transaction()?;
        require_inactive_conversation(&transaction, conversation_id)?;
        let current = transaction
            .query_row(
                "SELECT revision,updated_at_ms FROM conversations WHERE id=?1",
                params![conversation_id],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()?;
        let Some((revision, updated_at_ms)) = current else {
            transaction.rollback()?;
            return Ok(false);
        };
        enqueue_sync_change(
            &transaction,
            "conversation",
            conversation_id,
            revision
                .checked_add(1)
                .ok_or_else(|| StoreError::Invariant("conversation revision overflow".into()))?,
            true,
            "null",
            updated_at_ms,
        )?;
        // Conversation override preferences are locally owned and are not part
        // of a replicated full transcript projection. Delete them only when
        // the conversation is actually removed, in the tombstone transaction.
        transaction.execute(
            "DELETE FROM data_source_overrides WHERE conversation_id=?1",
            params![conversation_id],
        )?;
        let deleted = transaction.execute(
            "DELETE FROM conversations WHERE id = ?1",
            params![conversation_id],
        )?;
        transaction.commit()?;
        Ok(deleted == 1)
    }
}
