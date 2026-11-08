// SPDX-License-Identifier: Apache-2.0
//! Atomic replica persistence contracts; no SQLite or Application dependency.
use serde::{Deserialize, Serialize};

/// One durable, Application-owned change waiting to be copied to a remote replica.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredSyncChange {
    pub local_sequence: i64,
    pub operation_id: String,
    pub entity_type: String,
    pub entity_id: String,
    pub revision: i64,
    pub deleted: bool,
    pub payload_json: String,
    pub status: String,
    pub remote_cursor: Option<i64>,
    pub created_at_ms: i64,
    pub resolved_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredSyncCursor {
    pub replica_id: String,
    pub cursor: i64,
    pub updated_at_ms: i64,
}

/// One committed change returned by a remote replica transport. Hosts only
/// perform HTTP and map the wire payload into this typed repository input.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemoteSyncChangeInput {
    pub cursor: i64,
    pub operation_id: String,
    pub entity_type: String,
    pub entity_id: String,
    pub revision: i64,
    pub deleted: bool,
    pub payload_json: String,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ApplyRemoteSyncPageInput {
    pub replica_id: String,
    pub changes: Vec<RemoteSyncChangeInput>,
    pub next_cursor: i64,
    pub applied_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ApplyRemoteSyncPageResult {
    pub applied: usize,
    pub skipped: usize,
    pub cursor: i64,
}

/// All mutations participate in the caller's command unit of work. Applying
/// a page validates its full dependency set and advances its cursor atomically;
/// failures leave entities, outbox acknowledgements, and the cursor unchanged.
/// Pending pages must preserve dependency order and enforce encoded byte limits.
pub trait ReplicaSyncRepository {
    type Error: std::fmt::Display;
    fn pending_sync_changes(&self, limit: usize) -> Result<Vec<StoredSyncChange>, Self::Error>;
    fn resolve_sync_change(
        &mut self,
        operation_id: &str,
        status: &str,
        remote_cursor: Option<i64>,
        resolved_at_ms: i64,
    ) -> Result<bool, Self::Error>;
    fn sync_cursor(&self, replica_id: &str) -> Result<Option<StoredSyncCursor>, Self::Error>;
    fn save_sync_cursor(
        &mut self,
        replica_id: &str,
        cursor: i64,
        updated_at_ms: i64,
    ) -> Result<StoredSyncCursor, Self::Error>;
    fn apply_remote_sync_page(
        &mut self,
        input: &ApplyRemoteSyncPageInput,
    ) -> Result<ApplyRemoteSyncPageResult, Self::Error>;
}

/// Composition port that lends only replica synchronization persistence.
pub trait ReplicaSyncRepositoryProvider {
    type Error: std::fmt::Display;
    type Repository<'a>: ReplicaSyncRepository<Error = Self::Error>
    where
        Self: 'a;

    fn replica_sync_repository(&mut self) -> Self::Repository<'_>;
}
