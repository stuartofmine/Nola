// SPDX-License-Identifier: Apache-2.0
//! Canonical memory persistence contracts, independent of SQLite and Agent Core.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct StoredMemoryRecord {
    pub id: String,
    pub owner_json: String,
    pub namespace_json: String,
    pub scope: String,
    pub kind: String,
    pub content: String,
    pub summary: Option<String>,
    pub tags_json: String,
    pub confidence: f64,
    pub importance: f64,
    pub sensitivity: Option<String>,
    pub retention_json: String,
    pub expires_at: Option<String>,
    pub supersedes_json: String,
    pub provenance_json: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub last_used_at_ms: Option<i64>,
    pub deleted_at_ms: Option<i64>,
}

/// One idempotent observation that a Core turn actually selected a durable
/// memory record for model context.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredMemoryUsageEntry {
    pub usage_id: String,
    pub memory_id: String,
    pub session_id: String,
    pub turn_id: String,
    pub used_at_ms: i64,
    pub context_payload_digest: String,
    pub citation_ids_json: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredMemoryUsageStats {
    pub memory_id: String,
    pub usage_count: u64,
    pub last_used_at_ms: i64,
}

pub trait MemoryReadRepository {
    type Error: std::fmt::Display;
    fn memory_record(&self, id: &str) -> Result<Option<StoredMemoryRecord>, Self::Error>;
    fn memory_record_with_revision(
        &self,
        id: &str,
    ) -> Result<Option<(StoredMemoryRecord, i64)>, Self::Error>;
    fn memory_records_by_ids(&self, ids: &[String])
        -> Result<Vec<StoredMemoryRecord>, Self::Error>;
    fn list_memory_records(&self, limit: usize) -> Result<Vec<StoredMemoryRecord>, Self::Error>;
    fn memory_usage_stats(
        &self,
        memory_ids: &[String],
    ) -> Result<Vec<StoredMemoryUsageStats>, Self::Error> {
        let _ = memory_ids;
        Ok(Vec::new())
    }
    /// Query is literal text, never an SQL pattern. Exclude tombstones and
    /// expired records; filter namespace and scope before limiting results.
    /// Implementations also return a bounded supersession-successor closure
    /// for matching rows so semantic selection cannot resurrect an old fact
    /// merely because its correction uses different words.
    fn recall_memory_records(
        &self,
        owner_json: &str,
        namespace_json: &str,
        scope: &str,
        query: Option<&str>,
        kind: Option<&str>,
        now_iso: &str,
        limit: usize,
    ) -> Result<Vec<StoredMemoryRecord>, Self::Error>;
}

/// Operations participate in the caller's write unit of work when one exists.
/// A replacement requires that unit before its snapshot check and all writes;
/// row updates, sync facts and command receipt must commit or roll back together.
pub trait MemoryRepository: MemoryReadRepository {
    fn has_write_unit_of_work(&self) -> bool;
    fn upsert_memory_record(
        &mut self,
        record: &StoredMemoryRecord,
    ) -> Result<StoredMemoryRecord, Self::Error>;
    fn save_memory_record_deduplicated(
        &mut self,
        record: &StoredMemoryRecord,
    ) -> Result<StoredMemoryRecord, Self::Error>;
    fn delete_memory_record(&mut self, id: &str, deleted_at_ms: i64) -> Result<bool, Self::Error>;
}

/// Atomically publishes the idle Core checkpoint and accepted memory output,
/// checking all memory input revisions inside the caller's receipt transaction.
pub trait IdleCheckpointRepository:
    MemoryReadRepository + crate::run_effect::RunEffectRepository
{
    fn save_checkpoint_with_memories(
        &mut self,
        input: &crate::conversation_revert::SaveCheckpointInput,
        memories: &[StoredMemoryRecord],
        expected_memories: &[(String, i64)],
        memory_usage_entries: &[StoredMemoryUsageEntry],
    ) -> Result<
        (crate::run_effect::StoredCheckpoint, Vec<StoredMemoryRecord>),
        <Self as MemoryReadRepository>::Error,
    >;
}
