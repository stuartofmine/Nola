// SPDX-License-Identifier: Apache-2.0
//! Recovery facts and explicit mailbox acknowledgement, independent of SQLite.
use crate::{
    permission::{StoredPermissionGrant, StoredPermissionRequest},
    run_effect::StoredActiveRun,
    runtime_task::StoredRuntimeTask,
    work_step_transaction::StoredWorkStepTransaction,
};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeTaskReadCursor {
    pub task_id: String,
    pub updated_at_ms: i64,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AcknowledgeRuntimeTaskReadsInput {
    pub conversation_id: String,
    /// Lowest canonical checkpoint revision from which the task evidence was
    /// materialized. Goal continuation may legitimately advance the same
    /// lineage before the evidence is acknowledged.
    pub minimum_checkpoint_revision: i64,
    pub tasks: Vec<RuntimeTaskReadCursor>,
}
#[derive(Debug, Clone)]
pub struct ResumeSnapshotFacts {
    pub active_run: Option<StoredActiveRun>,
    pub runtime_tasks: Vec<StoredRuntimeTask>,
    pub permission_grants: Vec<StoredPermissionGrant>,
    pub pending_permissions: Vec<StoredPermissionRequest>,
    pub incomplete_effects: Vec<StoredWorkStepTransaction>,
}
/// All candidate, authoritative session and fact reads MUST participate in the
/// caller's ConsistentReadRepository scope; do not open another connection.
pub trait ResumeSnapshotRepository: crate::remote_session::SessionProjectionRepository {
    type ResumeError: std::fmt::Display;
    /// Preserve conversation recency order; read only settings/identity metadata.
    fn resumable_conversation_ids(
        &self,
        project_root: &str,
    ) -> Result<Vec<String>, Self::ResumeError>;
    fn load_resume_facts(
        &self,
        conversation_id: &str,
        project_root: &str,
        host_id: Option<&str>,
    ) -> Result<ResumeSnapshotFacts, Self::ResumeError>;
    /// Exact terminal unread cursor CAS plus non-regressing checkpoint lineage.
    /// All cursor validation and unread updates share the outer receipt UoW.
    fn acknowledge_runtime_task_reads(
        &mut self,
        input: &AcknowledgeRuntimeTaskReadsInput,
    ) -> Result<(), Self::ResumeError>;
}
