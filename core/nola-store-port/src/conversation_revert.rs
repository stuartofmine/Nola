// SPDX-License-Identifier: Apache-2.0
//! Metadata-only canonical revert planning. Never carries original Part bodies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationRevertBoundary {
    pub conversation_revision: i64,
    pub boundary_ordinal: i64,
    pub parent_message_id: Option<String>,
    pub hidden_message_ids: Vec<String>,
}

pub trait ConversationRevertRepository {
    type Error: std::fmt::Display;
    /// Same-connection atomic checkpoint/visibility CAS; nested in outer receipt savepoint.
    fn commit_checkpoint_projection(
        &mut self,
        input: &CommitCheckpointProjectionInput,
    ) -> Result<CommitCheckpointProjectionResult, Self::Error>;
    fn conversation_revert_boundary(
        &self,
        conversation_id: &str,
        through_message_id: &str,
        keep_message: bool,
    ) -> Result<ConversationRevertBoundary, Self::Error>;
}

use crate::run_effect::StoredCheckpoint;
use crate::transcript_write::MessageInput;
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MessageProjectionMutation {
    pub event_id: String,
    pub message_id: String,
    pub visible: bool,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ApplyConversationProjectionInput {
    pub operation_id: String,
    pub conversation_id: String,
    pub expected_revision: i64,
    pub operation_kind: String,
    pub parent_message_id: Option<String>,
    pub mutations: Vec<MessageProjectionMutation>,
    pub appended_messages: Vec<MessageInput>,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ApplyConversationProjectionResult {
    pub applied: bool,
    pub conversation_revision: i64,
    pub ledger_message_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommitCheckpointProjectionInput {
    pub checkpoint: SaveCheckpointInput,
    pub projection: ApplyConversationProjectionInput,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommitCheckpointProjectionResult {
    pub checkpoint: StoredCheckpoint,
    pub projection: ApplyConversationProjectionResult,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SaveCheckpointInput {
    pub conversation_id: String,
    pub expected_revision: i64,
    pub revision: i64,
    pub event_cursor: i64,
    pub checkpoint_json: String,
    pub updated_at_ms: i64,
}
