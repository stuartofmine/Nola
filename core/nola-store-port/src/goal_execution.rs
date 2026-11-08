// SPDX-License-Identifier: Apache-2.0
//! Immutable Goal execution snapshots, joining the canonical Start transaction.
use nola_domain::goal_execution::GoalExecutionBindingIdentity;
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredGoalExecutionBinding {
    pub identity: GoalExecutionBindingIdentity,
    pub binding_json: String,
    pub binding_sha256: String,
    pub created_at_ms: i64,
}
pub trait GoalExecutionRepository {
    type Error: std::fmt::Display;
    fn load_goal_execution_binding(
        &self,
        conversation_id: &str,
        goal_id: &str,
    ) -> Result<Option<StoredGoalExecutionBinding>, Self::Error>;
    /// Requires the caller's transaction. Identical retries are idempotent;
    /// neither a different operation nor different execution facts may replace it.
    fn insert_goal_execution_binding(
        &mut self,
        binding: &StoredGoalExecutionBinding,
    ) -> Result<(), Self::Error>;
}
