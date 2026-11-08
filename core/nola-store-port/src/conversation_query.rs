// SPDX-License-Identifier: Apache-2.0
//! Same-connection repository contracts used by the command dispatcher.
use crate::conversation_history::StoredConversation;
use serde::{Deserialize, Serialize};
/// Selective committed permission state; never a replacement checkpoint body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CurrentPermissionState {
    pub active_permission_policy: Option<nola_domain::ActivePermissionPolicy>,
    pub conversation_id: String,
    pub revision: i64,
    pub event_cursor: i64,
    pub session_id: String,
    pub state_session_id: String,
    pub active_turn_id: Option<String>,
    pub permissions: serde_json::Value,
}
/// Explicit bounded selection. Filtering happens before preview body reads.
/// Each page observes a fresh read snapshot. Offsets enumerate a quiescent list;
/// callers refresh from zero after concurrent list mutations, not splice pages
/// from different revisions into an authoritative projection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConversationListQuery {
    pub conversation_id: Option<String>,
    pub project_root: Option<String>,
    pub non_empty: bool,
    pub limit: usize,
    pub offset: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConversationSummary {
    pub id: String,
    pub title: String,
    pub revision: i64,
    pub message_count: i64,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub project_root: Option<String>,
    pub first_user_message: Option<String>,
    pub last_user_message_at_ms: Option<i64>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredConversationTailSnapshot {
    pub conversation: StoredConversation,
    pub total_message_count: i64,
    pub total_user_turns: i64,
}
/// Reads join the caller snapshot; mutations join its receipt Unit of Work.
pub trait ConversationQueryRepository {
    type Error: std::fmt::Display;
    /// Includes hidden messages and sealed Runs; configuration-only checkpoints
    /// do not constitute execution history.
    fn conversation_has_admitted_history(&self, conversation_id: &str) -> Result<bool, Self::Error>;
    fn conversation_summary(
        &self,
        conversation_id: &str,
    ) -> Result<Option<ConversationSummary>, Self::Error>;
    fn list_conversations(&self) -> Result<Vec<ConversationSummary>, Self::Error>;
    fn list_conversation_page(&self, query: &ConversationListQuery) -> Result<Vec<ConversationSummary>, Self::Error>;
    fn load_conversation_settings(
        &self,
        conversation_id: &str,
    ) -> Result<Option<String>, Self::Error>;
    /// Display tail: text limits count Unicode scalars; JSON limits count UTF-8
    /// bytes including review projections. Both limits must be positive.
    /// Omitted JSON is not an original-field absence claim; model history uses
    /// the owner-bound historical field read contract.
    fn load_conversation_tail_snapshot(
        &self,
        conversation_id: &str,
        message_limit: usize,
        part_text_limit: usize,
        part_json_limit: usize,
    ) -> Result<Option<StoredConversationTailSnapshot>, Self::Error>;
    fn load_session_permission_inputs(
        &self,
        conversation_id: &str,
        current: Option<&CurrentPermissionState>,
    ) -> Result<Option<String>, Self::Error>;
}
