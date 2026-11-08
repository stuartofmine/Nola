// SPDX-License-Identifier: Apache-2.0
//! Canonical historical reads without database ownership.
use crate::historical_part_projection::HistoricalPartProjectionRepository;
use crate::DeferredTextPart;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredPart {
    pub id: String,
    pub kind: String,
    pub ordinal: i64,
    pub state: String,
    pub content_text: String,
    pub content_json: Option<String>,
    /// Read projection metadata: the original JSON exists but was not hydrated.
    /// This is not a persisted Part field or authority to read its bytes.
    pub content_json_deferred: bool,
    pub content_digest: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredMessage {
    pub id: String,
    pub run_id: Option<String>,
    pub role: String,
    pub status: String,
    pub ordinal: i64,
    pub created_at_ms: i64,
    pub completed_at_ms: Option<i64>,
    pub parts: Vec<StoredPart>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredConversation {
    pub id: String,
    pub title: String,
    pub revision: i64,
    pub configuration_revision: i64,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub settings_json: Option<String>,
    pub messages: Vec<StoredMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredConversationMessageWindow {
    pub assistant_journal_heads: Vec<nola_domain::assistant_journal::AssistantJournalCursor>,
    pub conversation: StoredConversation,
    pub total_message_count: i64,
    pub before_ordinal: Option<i64>,
    pub has_more: bool,
    pub deferred_text_parts: Vec<DeferredTextPart>,
}

/// UTF-8 bounded canonical selection, including explicit original-length omissions.
/// Implementations share the caller's read snapshot/outer command transaction.
pub trait CanonicalHistoryRepository: HistoricalPartProjectionRepository {
    fn load_conversation_message_window(
        &self,
        conversation_id: &str,
        before_ordinal: Option<i64>,
        limit: usize,
        content_byte_limit: usize,
    ) -> Result<Option<StoredConversationMessageWindow>, Self::Error>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelTextOwner {
    pub run_id: String,
    pub attempt_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DelegatedTextOwner {
    pub run_id: String,
    pub invocation_id: String,
}

/// Presentation metadata reads; deliberately excludes permission/skill mutation
/// and whole-session checkpoint decoding. JSON inputs are bounded selected subsets.
pub trait ConversationProjectionRepository:
    CanonicalHistoryRepository
    + crate::run_effect::RunEffectRepository
    + crate::protocol_journal::ProjectionJournalRepository
{
    type ProjectionError: std::fmt::Display;
    fn conversation_source_owner(&self, id: &str) -> Result<nola_domain::ConversationSourceOwner, Self::ProjectionError>;
    fn conversation_execution_ownership(&self, id: &str) -> Result<nola_domain::ConversationExecutionOwnership, Self::ProjectionError>;
    fn conversation_cache_usage(&self, id: &str)
        -> Result<Option<nola_domain::ConversationCacheUsage>, Self::ProjectionError>;
    fn conversation_committed_context_usage(&self, id: &str)
        -> Result<Option<nola_domain::committed_run_context_usage::CommittedRunContextUsage>, Self::ProjectionError>;
    fn load_conversation(
        &self,
        id: &str,
    ) -> Result<Option<StoredConversation>, Self::ProjectionError>;
    fn load_conversation_page(
        &self,
        id: &str,
        offset: usize,
        limit: usize,
    ) -> Result<Option<StoredConversation>, Self::ProjectionError>;
    fn load_local_presentation_inputs(
        &self,
        id: &str,
        message_ids_json: &str,
        current: Option<&crate::run_effect::StoredCheckpoint>,
    ) -> Result<Option<String>, Self::ProjectionError>;
    fn has_remote_presentation_owner(&self, id: &str) -> Result<bool, Self::ProjectionError>;
    /// Rendering authority for an in-flight remote Run does not require a
    /// terminal session snapshot to have arrived yet.
    fn has_active_remote_run(&self, id: &str) -> Result<bool, Self::ProjectionError>;
    fn delegated_text_part_owner(
        &self,
        conversation_id: &str,
        part_id: &str,
    ) -> Result<Option<DelegatedTextOwner>, Self::ProjectionError>;
    fn model_text_part_owner(
        &self,
        conversation_id: &str,
        part_id: &str,
    ) -> Result<Option<ModelTextOwner>, Self::ProjectionError>;
}
