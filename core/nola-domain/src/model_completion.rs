// SPDX-License-Identifier: Apache-2.0
//! Facts and typed changes for a completed model step, without transcript ownership.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskCompletionOutcome {
    Answered,
    ActionCompleted,
    NeedsInput,
    Blocked,
    Incomplete,
}
impl TaskCompletionOutcome {
    /// Stable transport spelling. Ending an execution is not automatically
    /// successful task completion; adapters must preserve every outcome.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Answered => "answered",
            Self::ActionCompleted => "action_completed",
            Self::NeedsInput => "needs_input",
            Self::Blocked => "blocked",
            Self::Incomplete => "incomplete",
        }
    }
}
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModelCompletionFacts {
    /// The configured execution slice has spent its tool-capable model rounds.
    /// This response is the one allowed tool-free synthesis boundary.
    pub iteration_budget_final_synthesis: bool,
    pub has_tool_calls: bool,
    /// Derived from the actual request projection, not inferred from model text.
    pub tool_free_request: bool,
    pub has_visible_answer: bool,
    pub structurally_repetitive: bool,
    pub continuation_made_no_progress: bool,
    pub output_limited: bool,
    pub output_limit_retry_requested: bool,
    pub user_denied_permission: bool,
    pub final_answer_retry_requested: bool,
    pub final_answer_draft_present: bool,
    pub final_answer_audit_report_present: bool,
    pub normalized_audit_report_valid: bool,
    pub audit_requires_execution: bool,
    pub audit_accepts_candidate: bool,
    pub audit_requests_input: bool,
    pub audit_blocks_task: bool,
    pub completion_recovery_count: u8,
    pub answer_revision_count: u8,
    pub embedded_tool_protocol: bool,
    pub final_answer_audit_required: bool,
    pub has_executed_tools: bool,
    pub reasoning_in_items: bool,
    pub reasoning_only_retry_requested: bool,
    pub empty_completion_retry_requested: bool,
    pub no_progress_count: u8,
    pub validation_required: bool,
    pub recovery_required: bool,
    pub skill_completion_pending: bool,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ModelCompletionAuditUpdate {
    #[default]
    Unchanged,
    AcceptReport,
    SaveDraft,
    ResumeExecution,
    ReviseDraft,
}
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModelCompletionUpdate {
    pub no_progress_count: Option<u8>,
    pub request_final_answer: bool,
    pub request_reasoning_continuation: bool,
    pub request_empty_continuation: bool,
    pub audit: ModelCompletionAuditUpdate,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelCompletionFailure {
    IterationBudgetFinalSynthesis,
    RepetitiveOutput,
    ToolsAfterDenial,
    ToolsDuringTextOnlyResponse,
    NoDenialExplanation,
    InvalidAudit,
    EmbeddedToolProtocol,
    ReasoningOnly,
    EmptyCompletion,
    EmptyFinalSynthesis,
}
impl std::fmt::Display for ModelCompletionFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::IterationBudgetFinalSynthesis => "model iteration budget was exhausted before the Turn completed",
            Self::RepetitiveOutput => "model repeatedly produced structurally repetitive output",
            Self::ToolsAfterDenial => "model attempted to call tools after the user denied permission",
            Self::ToolsDuringTextOnlyResponse => "model repeatedly called tools during a tool-free response phase",
            Self::NoDenialExplanation => "model completed without explaining the denied permission",
            Self::InvalidAudit => "final answer audit repeatedly returned an invalid protocol report",
            Self::EmbeddedToolProtocol => "final synthesis repeatedly emitted embedded tool-call protocol",
            Self::ReasoningOnly => "model repeatedly completed after reasoning without producing an answer or tool call",
            Self::EmptyCompletion => "model repeatedly completed without text, reasoning, or a tool call",
            Self::EmptyFinalSynthesis => "final synthesis repeatedly completed without an answer",
        })
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelCompletionAction {
    /// A valid review identified unfinished work; this is not a protocol error.
    ConcludeIncomplete { reason: ModelCompletionIncompleteReason },
    PublishReviewedCandidate { blocked: bool },
    DiscardAndRequestModel,
    DiscardAndFail(ModelCompletionFailure),
    DiscardAndComplete,
    CommitAndRecoverOutputLimit,
    CommitCompletion {
        has_tool_calls: bool,
    },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelCompletionIncompleteReason {
    OutputLimitRecoveryExhausted,
    ExecutionRecoveryBudget,
    ReviewRevisionBudget,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelCompletionPlan {
    pub action: ModelCompletionAction,
    pub update: ModelCompletionUpdate,
}
