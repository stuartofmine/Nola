// SPDX-License-Identifier: Apache-2.0
//! Typed original-text reads with response identity and progress validation.
use crate::ApplicationServiceClient;
use nola_protocol::{ConversationRecallCommand, ConversationTextRange};

impl ApplicationServiceClient {
    pub fn read_conversation_text_range(
        &self,
        conversation_id: &str,
        message_id: &str,
        part_id: &str,
        offset_bytes: u64,
        limit_bytes: usize,
    ) -> Result<ConversationTextRange, String> {
        let request = ConversationRecallCommand::ReadConversationTextRange {
            conversation_id: conversation_id.into(),
            message_id: message_id.into(),
            part_id: part_id.into(),
            offset_bytes,
            limit_bytes,
        };
        request.validate()?;
        let response: ConversationTextRange = self.execute(&request)?;
        response.validate_for(&request)?;
        Ok(response)
    }
}
