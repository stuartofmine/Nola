// SPDX-License-Identifier: Apache-2.0
//! Core-authored semantics. Database ownership and event positions are bound at commit.
use crate::assistant_journal_operations::JournalNumber;
use crate::run_requirements::{RequirementAssessmentMethod, RequirementVerdict, RunRequirement};
use crate::run_result::RunResultValue;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoreAppliedInputRef {
    pub admission_id: String,
    pub turn_id: String,
    pub applied_sequence: JournalNumber,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CoreDecisionSource {
    Model {
        effect_id: String,
        request_id: String,
        attempt_id: Option<String>,
        call_id: Option<String>,
    },
    Runtime {
        code: String,
        source_refs: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoreRequirementScope {
    pub scope_id: String,
    pub revision: JournalNumber,
    pub applied_input_refs: Vec<CoreAppliedInputRef>,
    pub source_input_refs: Vec<String>,
    pub requirements: Vec<RunRequirement>,
    pub source: CoreDecisionSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoreRequirementAssessment {
    pub scope_id: String,
    pub assessment_id: String,
    pub requirement_id: String,
    pub requirement_set_revision: JournalNumber,
    pub applied_input_refs: Vec<CoreAppliedInputRef>,
    pub evidence_frontier: JournalNumber,
    pub method: RequirementAssessmentMethod,
    pub evidence_refs: Vec<String>,
    pub verdict: RequirementVerdict,
    pub rationale: String,
    pub source: CoreDecisionSource,
    /// Original request aliases, authenticated against the immutable outbox at
    /// commit and retained with the source event after outbox compaction.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_binding: Option<crate::requirement_control::RequirementRequestBinding>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoreRunResultDecision {
    pub result_id: String,
    pub scope_id: String,
    pub requirement_revision: JournalNumber,
    pub applied_input_refs: Vec<CoreAppliedInputRef>,
    pub evidence_frontier: JournalNumber,
    pub value: RunResultValue,
    pub source: CoreDecisionSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoreRuntimeDecisionFact {
    pub fact_id: String,
    pub kind: CoreRuntimeDecisionFactKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CoreRuntimeDecisionFactKind {
    RecoveryStopped {
        cause: RecoveryStopCause,
        detail: String,
        consumed_model_requests: Vec<u64>,
    },
    UnknownEffectRecovery { decision: crate::unknown_effect_recovery::UnknownEffectRecoveryDecision },
    HostCancellation {
        cancellation_id: String,
        request_id: String,
        effect_id: String,
        host_id: String,
        response_sequence: JournalNumber,
    },
    Cancellation {
        cancellation_id: String,
        reason: String,
    },
    Error {
        code: String,
        detail: String,
    },
    Blocker {
        code: String,
        detail: String,
        source_refs: Vec<String>,
    },
}

/// A Core execution-policy stop, not an execution error or a user blocker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RecoveryStopCause {
    BudgetExhausted { limit: u32 },
    EmptyModelContinuation,
    UnfinishedRequirementControl { call_id: String },
}
