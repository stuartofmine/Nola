// SPDX-License-Identifier: Apache-2.0
//! Canonical context checkpoint persistence contracts.
use serde::{Deserialize, Serialize};

/// Immutable source witness. Appends do not invalidate the original prefix;
/// edits and another context commit do.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ContextCompactionSource {
    pub conversation_id: String,
    pub message_count: u64,
    pub source_fingerprint: String,
    pub context_fingerprint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CommitContextCompactionInput {
    pub source: ContextCompactionSource,
    /// Complete source observed during the post-generation budget check.
    pub validated_source: ContextCompactionSource,
    pub summary: String,
    pub entry: serde_json::Value,
    pub committed_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextCheckpointInput {
    pub id: String,
    pub conversation_id: String,
    pub run_id: Option<String>,
    pub model_profile_id: String,
    pub policy_version: String,
    pub covered_through_message_id: Option<String>,
    pub covered_through_sequence: Option<i64>,
    pub summary_part_id: Option<String>,
    pub retained_tail_start_message_id: Option<String>,
    pub estimated_tokens: i64,
    pub measured_tokens: Option<i64>,
    pub context_window_tokens: i64,
    pub prompt_limit_tokens: i64,
    pub payload_digest: String,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextComponentInput {
    pub id: String,
    pub checkpoint_id: String,
    pub kind: String,
    pub source_id: Option<String>,
    pub source_uri: Option<String>,
    pub title: Option<String>,
    pub mime_type: Option<String>,
    pub authorization: String,
    pub observed_at_ms: Option<i64>,
    pub maximum_age_minutes: Option<i64>,
    pub freshness: String,
    pub content_reference_json: String,
    pub content_digest: String,
    pub estimated_tokens: i64,
    pub provenance_json: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommitContextCheckpointInput {
    pub checkpoint: ContextCheckpointInput,
    pub components: Vec<ContextComponentInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextAggregateRecord {
    pub checkpoint: ContextCheckpointInput,
    pub components: Vec<ContextComponentInput>,
}

pub trait ContextRepository {
    type Error: std::fmt::Display;
    fn prepare_context_compaction(&self, conversation_id: &str) -> Result<ContextCompactionSource, Self::Error>;
    fn commit_context_compaction(&mut self, input: &CommitContextCompactionInput) -> Result<(), Self::Error>;
    /// Atomically publish checkpoint and components. Reject checkpoint identity
    /// reuse across conversations before modifying any components. Participate
    /// in the caller's write unit of work, including rollback on receipt failure.
    fn commit_context_checkpoint(
        &mut self,
        input: &CommitContextCheckpointInput,
    ) -> Result<(), Self::Error>;
    fn load_context_checkpoint(
        &self,
        checkpoint_id: &str,
    ) -> Result<Option<ContextAggregateRecord>, Self::Error>;
    /// Count checkpoint metadata without loading components or content.
    fn count_context_checkpoints(&self, conversation_id: &str) -> Result<u64, Self::Error>;
}

/// Composition port that lends only context-checkpoint persistence for one use case.
pub trait ContextRepositoryProvider {
    type Error: std::fmt::Display;
    type Repository<'a>: ContextRepository<Error = Self::Error>
    where
        Self: 'a;

    fn context_repository(&mut self) -> Self::Repository<'_>;
}
