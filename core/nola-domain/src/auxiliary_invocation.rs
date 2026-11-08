// SPDX-License-Identifier: Apache-2.0
//! Non-Run model operations share the canonical physical Attempt ledger.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AuxiliaryInvocationOwner {
    Room { room_id: String },
    Conversation {
        conversation_id: String,
    },
    Account {
        account_id: String,
    },
    Task {
        task_id: String,
    },
    /// Host-derived reference to an authenticated authorization epoch. It is
    /// not a client-local Conversation or an arbitrary client supplied owner.
    SourceAuthority {
        source_host_id: String,
        authority_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuxiliaryInvocationScenario {
    RoomRouting,
    ContextSummary,
    EventSummary,
    EventProcessing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuxiliaryOperationState {
    Ready,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuxiliaryInvocationConfiguration {
    pub provider: String,
    pub model: Option<String>,
    pub configuration_revision: u64,
    /// Non-secret adapter snapshot/digest. Never provider objects or keys.
    pub configuration_digest: String,
    pub input_digest: String,
    pub max_attempts: u32,
    pub maximum_duration_ms: u64,
    pub max_output_tokens_per_attempt: Option<u64>,
    pub total_output_token_budget: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuxiliaryOperation {
    pub id: String,
    pub owner: AuxiliaryInvocationOwner,
    pub scenario: AuxiliaryInvocationScenario,
    pub configuration: AuxiliaryInvocationConfiguration,
    pub state: AuxiliaryOperationState,
    pub attempts_started: u32,
    pub active_attempt_id: Option<String>,
    pub cancellation_requested_at_ms: Option<i64>,
    pub created_at_ms: i64,
    pub revision: i64,
}

/// The auxiliary variant of the physical ledger row. It has no fabricated
/// Run, effect delivery or message Part. `operation_id` is its immutable owner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuxiliaryModelAttempt {
    pub id: String,
    pub operation_id: String,
    pub predecessor_id: Option<String>,
    pub state: crate::ModelAttemptState,
    pub provider: String,
    pub model: Option<String>,
    pub usage: serde_json::Value,
    pub failure: Option<crate::ModelAttemptFailure>,
    /// Structured successful auxiliary result, committed with the physical
    /// receipt so a lost HTTP response never authorizes redispatch.
    pub output: Option<serde_json::Value>,
    pub created_at_ms: i64,
    pub completed_at_ms: Option<i64>,
    pub revision: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum AuxiliaryInvocationCommand {
    Admit {
        operation_id: String,
        owner: AuxiliaryInvocationOwner,
        scenario: AuxiliaryInvocationScenario,
        configuration: AuxiliaryInvocationConfiguration,
        at_ms: i64,
    },
    StartAttempt {
        operation_id: String,
        attempt_id: String,
        expected_revision: i64,
        at_ms: i64,
    },
    RecordReceipt {
        operation_id: String,
        attempt_id: String,
        expected_attempt_revision: i64,
        state: crate::ModelAttemptState,
        usage: serde_json::Value,
        failure: Option<crate::ModelAttemptFailure>,
        output: Option<serde_json::Value>,
        at_ms: i64,
    },
    Cancel {
        operation_id: String,
        at_ms: i64,
    },
    Inspect {
        operation_id: String,
    },
    ObserveSource {
        local_owner: AuxiliaryInvocationOwner,
        observation: AuxiliarySourceObservation,
    },
    InspectSource {
        local_owner: AuxiliaryInvocationOwner,
        source_host_id: String,
        operation_id: String,
    },
}

/// A receipt observation, never a second physical Attempt. Projection joins
/// only this explicit local-owner reference, not all work of a remote account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuxiliarySourceObservation {
    pub source_host_id: String,
    pub operation_id: String,
    pub attempt_id: String,
    pub receipt: AuxiliaryInvocationReceipt,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuxiliaryInvocationReceipt {
    pub operation: AuxiliaryOperation,
    pub attempt: Option<AuxiliaryModelAttempt>,
    pub dispatch_permitted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuxiliarySourceCursor {
    pub source_host_id: String,
    pub operation_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuxiliarySourceReference {
    pub source_host_id: String,
    pub operation_id: String,
    pub attempt_id: String,
}

/// Bounded references only, not historical outputs or a second usage ledger.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationAuxiliarySourcePage {
    pub references: Vec<AuxiliarySourceReference>,
    pub next_cursor: Option<AuxiliarySourceCursor>,
}
