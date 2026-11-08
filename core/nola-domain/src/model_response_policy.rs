// SPDX-License-Identifier: Apache-2.0
//! Explicit facts used to choose a model response boundary, never user-text routing.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalStatusOrigin {
    Audit,
    GoalUserOrContinuation,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoalStatusFacts {
    pub origin: GoalStatusOrigin,
    pub has_active_nonterminal_goal: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FinalSynthesisFacts {
    pub final_answer_retry_requested: bool,
    pub user_denied_permission: bool,
    pub skill_completion_pending: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextPresentationFacts {
    pub final_answer_audit_required: bool,
    pub has_executed_tools: bool,
    pub final_answer_retry_requested: bool,
    pub has_final_answer_draft: bool,
    pub has_final_answer_audit_report: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelResponseContract {
    Conversational,
    StructuredAudit,
    RequiredTool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompletionContractFacts {
    pub presentation: TextPresentationFacts,
    pub final_answer_audit_required: bool,
    pub has_executed_tools: bool,
    pub has_final_answer_draft: bool,
    pub has_final_answer_audit_report: bool,
    pub skill_protocol_convergence: bool,
    pub artifact_completion_pending: bool,
    pub final_synthesis: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompletionContractDecision {
    pub response: ModelResponseContract,
    pub defer_text_presentation: bool,
}
