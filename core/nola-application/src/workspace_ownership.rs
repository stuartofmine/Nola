// SPDX-License-Identifier: Apache-2.0
//! Workspace recovery checkpoints and session execution ownership.

use nola_store_port::workspace_ownership::{
    StoredSessionRunLease, StoredWorkspaceCheckpoint, WorkspaceOwnershipRepository,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceCheckpoint {
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionRunLease {
    pub session_id: String,
    pub owner: String,
    pub pid: u32,
    pub process_start: String,
    pub acquired_at_ms: i64,
}

pub fn save_workspace_checkpoint<R: WorkspaceOwnershipRepository + ?Sized>(
    store: &mut R,
    checkpoint: WorkspaceCheckpoint,
) -> Result<bool, String> {
    store
        .save_workspace_checkpoint(&checkpoint.into_stored())
        .map_err(|error| error.to_string())
}

pub fn list_workspace_checkpoints<R: WorkspaceOwnershipRepository + ?Sized>(
    store: &R,
    root: &str,
) -> Result<Vec<WorkspaceCheckpoint>, String> {
    store
        .workspace_checkpoints(root)
        .map_err(|error| error.to_string())
        .map(|values| {
            values
                .into_iter()
                .map(WorkspaceCheckpoint::from_stored)
                .collect()
        })
}

pub fn acquire_session_run_lease<R: WorkspaceOwnershipRepository + ?Sized>(
    store: &mut R,
    lease: SessionRunLease,
) -> Result<bool, String> {
    store
        .acquire_session_run_lease(&lease.into_stored())
        .map_err(|error| error.to_string())
}

pub fn session_run_lease<R: WorkspaceOwnershipRepository + ?Sized>(
    store: &R,
    session_id: &str,
) -> Result<Option<SessionRunLease>, String> {
    store
        .session_run_lease(session_id)
        .map_err(|error| error.to_string())
        .map(|lease| lease.map(SessionRunLease::from_stored))
}

pub fn session_run_leases<R: WorkspaceOwnershipRepository + ?Sized>(
    store: &R,
) -> Result<Vec<SessionRunLease>, String> {
    store
        .session_run_leases()
        .map_err(|error| error.to_string())
        .map(|leases| {
            leases
                .into_iter()
                .map(SessionRunLease::from_stored)
                .collect()
        })
}

impl WorkspaceCheckpoint {
    fn into_stored(self) -> StoredWorkspaceCheckpoint {
        StoredWorkspaceCheckpoint {
            id: self.id,
            root: self.root,
            git_root: self.git_root,
            revision: self.revision,
            source: self.source,
            label: self.label,
            created_at_ms: self.created_at_ms,
            status: self.status,
            untracked: self.untracked,
        }
    }

    fn from_stored(value: StoredWorkspaceCheckpoint) -> Self {
        Self {
            id: value.id,
            root: value.root,
            git_root: value.git_root,
            revision: value.revision,
            source: value.source,
            label: value.label,
            created_at_ms: value.created_at_ms,
            status: value.status,
            untracked: value.untracked,
        }
    }
}

impl SessionRunLease {
    fn into_stored(self) -> StoredSessionRunLease {
        StoredSessionRunLease {
            session_id: self.session_id,
            owner: self.owner,
            pid: self.pid,
            process_start: self.process_start,
            acquired_at_ms: self.acquired_at_ms,
        }
    }

    fn from_stored(value: StoredSessionRunLease) -> Self {
        Self {
            session_id: value.session_id,
            owner: value.owner,
            pid: value.pid,
            process_start: value.process_start,
            acquired_at_ms: value.acquired_at_ms,
        }
    }
}
