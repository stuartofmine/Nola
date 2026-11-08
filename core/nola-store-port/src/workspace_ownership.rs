// SPDX-License-Identifier: Apache-2.0
//! Persistence contracts; mutations participate in the caller's unit of work.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredWorkspaceCheckpoint {
    pub id: String,
    pub root: String,
    pub git_root: String,
    pub revision: String,
    pub source: String,
    pub label: String,
    pub created_at_ms: i64,
    pub status: String,
    pub untracked: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredSessionRunLease {
    pub session_id: String,
    pub owner: String,
    pub pid: u32,
    pub process_start: String,
    pub acquired_at_ms: i64,
}

/// Preserve atomic state/event and scheduling admission on the same repository.
/// No operation may commit an enclosing command receipt transaction.
#[allow(clippy::too_many_arguments)]
pub trait WorkspaceOwnershipRepository {
    type Error: std::fmt::Display;
    fn acquire_session_run_lease(
        &mut self,
        lease: &StoredSessionRunLease,
    ) -> Result<bool, Self::Error>;
    fn save_workspace_checkpoint(
        &mut self,
        checkpoint: &StoredWorkspaceCheckpoint,
    ) -> Result<bool, Self::Error>;
    fn session_run_lease(
        &self,
        session_id: &str,
    ) -> Result<Option<StoredSessionRunLease>, Self::Error>;
    fn session_run_leases(&self) -> Result<Vec<StoredSessionRunLease>, Self::Error>;
    fn workspace_checkpoints(
        &self,
        root: &str,
    ) -> Result<Vec<StoredWorkspaceCheckpoint>, Self::Error>;
    fn delete_workspace_checkpoint(&mut self, id: &str) -> Result<bool, Self::Error>;
    fn release_session_run_lease(
        &mut self,
        session_id: &str,
        owner: &str,
    ) -> Result<bool, Self::Error>;
}

/// Composition port that lends only workspace checkpoint and session-lease ownership.
pub trait WorkspaceOwnershipRepositoryProvider {
    type Error: std::fmt::Display;
    type Repository<'a>: WorkspaceOwnershipRepository<Error = Self::Error>
    where
        Self: 'a;

    fn workspace_ownership_repository(&mut self) -> Self::Repository<'_>;
}
