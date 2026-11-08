// SPDX-License-Identifier: Apache-2.0
//! Durable ownership and recovery evidence for one physical Host-effect delivery attempt.
use serde::{Deserialize, Serialize};

/// Maximum duration granted by one Host-effect lease claim or renewal.
/// Long-running execution remains possible through bounded, durable renewals.
pub const MAX_EFFECT_DELIVERY_LEASE_EXTENSION_MS: i64 = 300_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectDeliveryReplaySafety {
    Idempotent,
    Recoverable,
    RequiresRecoveryDecision,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectDeliveryAttemptState {
    Prepared,
    Leased,
    ExecutionStarted,
    Settled,
    Interrupted,
    OutcomeUnknown,
}

impl EffectDeliveryAttemptState {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Settled | Self::Interrupted | Self::OutcomeUnknown
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectDeliverySettlement {
    Succeeded,
    Failed,
    Cancelled,
    RejectedBeforeExecution,
}

/// The only automatic action authorized after an interrupted attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectDeliveryRecoveryPlan {
    Retry,
    Resume,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum EffectDeliveryManualResolution {
    /// Core proves the original approval is still pending. This authorizes only
    /// redisplaying that question, never a tool execution or user decision.
    PendingPermissionRedisplay {
        permission_request_id: String,
        resolved_at_ms: i64,
    },
    RiskRetryAuthorized {
        /// Original typed user decision JSON; an authorization, not proof that
        /// the external operation did not execute.
        evidence: String,
        resolved_at_ms: i64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectDeliveryAttempt {
    pub id: String,
    pub run_id: String,
    pub effect_id: String,
    pub request_id: String,
    pub predecessor_id: Option<String>,
    pub attempt_number: u32,
    pub replay_safety: EffectDeliveryReplaySafety,
    pub state: EffectDeliveryAttemptState,
    pub worker_generation: Option<String>,
    pub lease_owner: Option<String>,
    pub lease_token: Option<String>,
    pub lease_expires_at_ms: Option<i64>,
    pub settlement: Option<EffectDeliverySettlement>,
    pub recovery_plan: Option<EffectDeliveryRecoveryPlan>,
    pub manual_resolution: Option<EffectDeliveryManualResolution>,
    pub interruption_reason: Option<String>,
    pub created_at_ms: i64,
    pub claimed_at_ms: Option<i64>,
    pub execution_started_at_ms: Option<i64>,
    pub heartbeat_at_ms: Option<i64>,
    pub finished_at_ms: Option<i64>,
    pub revision: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectDeliveryAttemptMutation {
    /// None inserts; Some compares the durable revision before replacing the record.
    pub expected_revision: Option<i64>,
    pub record: EffectDeliveryAttempt,
}

/// Both mutations must commit atomically before the successor worker may execute.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectDeliveryReclaimPlan {
    pub interrupted: EffectDeliveryAttemptMutation,
    pub successor: EffectDeliveryAttemptMutation,
}
