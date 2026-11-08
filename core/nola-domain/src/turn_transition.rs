// SPDX-License-Identifier: Apache-2.0
//! Turn lifecycle values and explicit transition facts.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TurnInputFacts {
    pub has_items: bool,
    pub live_user_authority: bool,
    pub has_context_items: bool,
    pub has_context_components: bool,
    pub active_status: Option<AgentTurnStatus>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnInputAction {
    Start,
    Enqueue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnInputRejection {
    Empty,
    AutomatedMerge,
    ContextAlreadyCaptured,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnCompletionAction {
    ContinueCurrent,
    Complete,
    CompleteAndStartNext,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CancellableEffectFact {
    pub belongs_to_turn: bool,
    pub is_cancellation_request: bool,
    pub is_cancelling: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnCancellationPlan {
    pub effect_indices: Vec<usize>,
    pub tool_indices: Vec<usize>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentTurnStatus {
    #[default]
    Idle,
    AwaitingModel,
    AwaitingTools,
    AwaitingPermission,
    AwaitingInput,
    Compacting,
    Cancelling,
    Completed,
    Cancelled,
    Failed,
}

impl AgentTurnStatus {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Cancelled | Self::Failed)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCallStatus {
    Pending,
    AwaitingPermission,
    AwaitingUserInput,
    Ready,
    Running,
    Completed,
    Failed,
    Denied,
    Cancelled,
}

impl ToolCallStatus {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Denied | Self::Cancelled
        )
    }
}
