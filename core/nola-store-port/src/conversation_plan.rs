// SPDX-License-Identifier: Apache-2.0
//! Persistence contracts for plan/step CAS and atomic receipt participation.
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredConversationPlan {
    pub conversation_id: String,
    pub sop_id: String,
    pub sop_version: u32,
    pub run_id: String,
    pub todos_json: String,
    pub revision: i64,
    pub frozen_revision: Option<i64>,
    pub updated_at_ms: i64,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredConversationPlanRevision {
    pub conversation_id: String,
    pub sop_id: String,
    pub sop_version: u32,
    pub run_id: String,
    pub todos_json: String,
    pub revision: i64,
    pub change_kind: String,
    pub change_summary: String,
    pub updated_at_ms: i64,
}

#[allow(clippy::too_many_arguments)]
pub trait ConversationPlanRepository {
    type Error: std::fmt::Display;
    fn save_conversation_plan(
        &mut self,
        conversation_id: &str,
        sop_id: &str,
        sop_version: u32,
        run_id: &str,
        todos_json: &str,
        updated_at_ms: i64,
    ) -> Result<StoredConversationPlan, Self::Error>;
    fn conversation_plan(
        &self,
        conversation_id: &str,
    ) -> Result<Option<StoredConversationPlan>, Self::Error>;
    fn save_conversation_plan_if_revision(
        &mut self,
        conversation_id: &str,
        sop_id: &str,
        sop_version: u32,
        run_id: &str,
        todos_json: &str,
        expected_revision: i64,
        updated_at_ms: i64,
    ) -> Result<Option<StoredConversationPlan>, Self::Error>;
    fn save_conversation_plan_revision_if_revision(
        &mut self,
        conversation_id: &str,
        sop_id: &str,
        sop_version: u32,
        run_id: &str,
        todos_json: &str,
        expected_revision: i64,
        frozen_revision: Option<Option<i64>>,
        change_kind: &str,
        change_summary: &str,
        updated_at_ms: i64,
    ) -> Result<Option<StoredConversationPlan>, Self::Error>;
    fn conversation_plan_revisions(
        &self,
        conversation_id: &str,
    ) -> Result<Vec<StoredConversationPlanRevision>, Self::Error>;
}
