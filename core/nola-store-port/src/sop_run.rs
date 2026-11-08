// SPDX-License-Identifier: Apache-2.0
//! SOP records join the caller's transaction; Core decides valid transitions.
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredSopRun {
    pub run_id: String,
    pub sop_id: String,
    pub sop_version: i64,
    pub owner_session_id: Option<String>,
    pub project_root: String,
    pub status: String,
    pub definition_json: String,
    pub record_json: String,
    pub revision: i64,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

/// Exact durable workspace identity, not a path alias or authorization grant.
/// No owner exposes only workspace-owned runs, never another session's runs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VisibleActiveSopRuns {
    pub project_root: String,
    pub owner_session_id: Option<String>,
}

/// Resource guard, not pagination: exceeding it must fail, never truncate.
pub const MAX_VISIBLE_ACTIVE_SOP_RUNS: usize = 256;

pub trait SopRunRepository {
    type Error: std::fmt::Display;
    fn create_sop_run(&mut self, run: &StoredSopRun) -> Result<(), Self::Error>;
    fn update_sop_run_cas(
        &mut self,
        run: &StoredSopRun,
        expected_revision: i64,
    ) -> Result<bool, Self::Error>;
    fn sop_run(&self, run_id: &str) -> Result<Option<StoredSopRun>, Self::Error>;
    fn visible_active_sop_runs(
        &self,
        query: &VisibleActiveSopRuns,
    ) -> Result<Vec<StoredSopRun>, Self::Error>;
    fn list_sop_runs(
        &self,
        project_root: Option<&str>,
        owner_session_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<StoredSopRun>, Self::Error>;
}
