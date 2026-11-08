// SPDX-License-Identifier: Apache-2.0
//! Persistence contracts for plan/step CAS and atomic receipt participation.
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredWorkStepTransaction {
    pub id: String,
    pub work_id: String,
    pub step_id: String,
    pub tool_call_id: String,
    pub root: String,
    pub effect_kind: String,
    pub phase: String,
    pub workspace_checkpoint_id: Option<String>,
    pub request_json: String,
    pub result_json: Option<String>,
    pub result_ref: Option<String>,
    pub error: Option<String>,
    pub revision: i64,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

/// Plan-linked terminal advance MUST atomically CAS both revisions, publish
/// both changes and participate in the existing receipt transaction.
#[allow(clippy::too_many_arguments)]
pub trait WorkStepTransactionRepository:
    crate::conversation_plan::ConversationPlanRepository
{
    fn prepare_work_step_transaction(
        &mut self,
        input: &StoredWorkStepTransaction,
    ) -> Result<(StoredWorkStepTransaction, bool), Self::Error>;
    fn advance_work_step_transaction(
        &mut self,
        id: &str,
        expected_revision: i64,
        phase: &str,
        workspace_checkpoint_id: Option<&str>,
        result_json: Option<&str>,
        result_ref: Option<&str>,
        error: Option<&str>,
        updated_at_ms: i64,
    ) -> Result<Option<StoredWorkStepTransaction>, Self::Error>;
    fn advance_work_step_transaction_and_link_plan(
        &mut self,
        id: &str,
        expected_revision: i64,
        phase: &str,
        workspace_checkpoint_id: Option<&str>,
        result_json: Option<&str>,
        result_ref: Option<&str>,
        error: Option<&str>,
        updated_at_ms: i64,
        conversation_id: &str,
        plan_expected_revision: i64,
        sop_id: &str,
        sop_version: u32,
        run_id: &str,
        todos_json: &str,
    ) -> Result<Option<StoredWorkStepTransaction>, Self::Error>;
    fn work_step_transaction(
        &self,
        id: &str,
    ) -> Result<Option<StoredWorkStepTransaction>, Self::Error>;
    fn incomplete_work_step_transactions(
        &self,
        work_id: Option<&str>,
    ) -> Result<Vec<StoredWorkStepTransaction>, Self::Error>;
}
