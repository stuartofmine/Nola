// SPDX-License-Identifier: Apache-2.0
//! Canonical transcript write contract. Application owns admission policy;
//! the repository owns atomic count CAS, message insertion and publication.
use nola_domain::MessagePartInput;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MessageInput {
    pub id: String,
    pub run_id: Option<String>,
    pub role: String,
    pub status: String,
    pub ordinal: i64,
    pub created_at_ms: i64,
    pub completed_at_ms: Option<i64>,
    pub parts: Vec<MessagePartInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppendMessagesInput {
    pub conversation_id: String,
    pub expected_message_count: usize,
    pub messages: Vec<MessageInput>,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppendMessagesResult {
    pub appended_message_count: usize,
    pub message_count: usize,
    pub conversation_revision: i64,
}

pub trait TranscriptWriteRepository {
    type Error: std::fmt::Display;
    /// Raw ledger count and greatest ordinal, including hidden messages.
    /// Visibility changes must never reuse an existing ledger ordinal.
    fn message_frontier(&self, conversation_id: &str) -> Result<(usize, i64), Self::Error>;
    fn has_active_run(&self, conversation_id: &str) -> Result<bool, Self::Error>;
    fn conversation_id_for_run(&self, run_id: &str) -> Result<Option<String>, Self::Error>;
    /// A failed count check or any Part/publication failure leaves no partial write.
    /// Recheck absence of an active Run in the same write unit as the count CAS;
    /// the earlier Application check is not a durable admission reservation.
    /// Known Run identities must still belong to the target conversation at commit.
    fn append_messages(
        &mut self,
        input: &AppendMessagesInput,
    ) -> Result<AppendMessagesResult, Self::Error>;
}

/// Canonical history mutations share the caller's receipt Unit of Work.
/// The adapter must atomically recheck idle Run state and conversation revision,
/// validate appended Run ownership, and publish visibility and new Parts together.
pub trait TranscriptEditRepository: TranscriptWriteRepository {
    fn apply_conversation_projection(
        &mut self,
        input: &crate::conversation_revert::ApplyConversationProjectionInput,
    ) -> Result<crate::conversation_revert::ApplyConversationProjectionResult, Self::Error>;
}
