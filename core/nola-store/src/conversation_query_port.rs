// SPDX-License-Identifier: Apache-2.0
use crate::{NolaStore, StoreError};
use nola_store_port::conversation_query::*;
impl ConversationQueryRepository for NolaStore {
    type Error = StoreError;
    fn conversation_has_admitted_history(&self, conversation_id: &str) -> Result<bool, Self::Error> {
        Ok(self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM runs WHERE conversation_id=?1)
                OR EXISTS(SELECT 1 FROM messages WHERE conversation_id=?1)",
            [conversation_id], |row| row.get(0),
        )?)
    }
    fn conversation_summary(
        &self,
        conversation_id: &str,
    ) -> Result<Option<ConversationSummary>, Self::Error> {
        NolaStore::conversation_summary(self, conversation_id)
    }
    fn list_conversations(&self) -> Result<Vec<ConversationSummary>, Self::Error> {
        NolaStore::list_conversations(self)
    }
    fn list_conversation_page(&self, query: &ConversationListQuery) -> Result<Vec<ConversationSummary>, Self::Error> {
        NolaStore::list_conversation_page(self, query)
    }
    fn load_conversation_settings(
        &self,
        conversation_id: &str,
    ) -> Result<Option<String>, Self::Error> {
        NolaStore::load_conversation_settings(self, conversation_id)
    }
    fn load_conversation_tail_snapshot(
        &self,
        conversation_id: &str,
        message_limit: usize,
        part_text_limit: usize,
        part_json_limit: usize,
    ) -> Result<Option<StoredConversationTailSnapshot>, Self::Error> {
        NolaStore::load_conversation_tail_snapshot(
            self,
            conversation_id,
            message_limit,
            part_text_limit,
            part_json_limit,
        )
    }
    fn load_session_permission_inputs(
        &self,
        conversation_id: &str,
        current: Option<&CurrentPermissionState>,
    ) -> Result<Option<String>, Self::Error> {
        NolaStore::load_session_permission_inputs(self, conversation_id, current)
    }
}
