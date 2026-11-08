// SPDX-License-Identifier: Apache-2.0
//! Read-only persisted Run evidence, not a decision to retry or authorize work.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunRecoveryInspection {
    pub conversation_id: String,
    pub run_id: String,
    pub assistant_message_id: String,
    pub observation: RunRecoveryObservation,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum RunRecoveryObservation {
    /// Absence in this Store does not prove that submission never occurred.
    Missing,
    IdentityMismatch,
    Inconsistent {
        reason: RunRecoveryInconsistency,
    },
    Terminal {
        result: crate::run_result::RunResult,
        result_digest: crate::assistant_journal::AssistantDigest,
        completed_at_ms: i64,
    },
    /// Nonterminal persisted facts; no OS liveness or replay authorization.
    Active {
        run_status: String,
        checkpoint_present: bool,
        pending_effect_count: u64,
        source_host_id: Option<String>,
        has_session_lease: bool,
        deliveries: Vec<RunDeliveryStateCount>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunRecoveryInconsistency {
    AssistantOwnerInvalid,
    TerminalTimestampMissing,
    NonterminalHasCompletionTimestamp,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunDeliveryStateCount {
    pub state: crate::EffectDeliveryAttemptState,
    pub count: u64,
}
