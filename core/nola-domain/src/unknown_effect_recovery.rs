// SPDX-License-Identifier: Apache-2.0
//! Explicit user recovery choices; none of these assertions proves an external effect.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct UnknownEffectRecoveryTarget {
    pub effect_id: String,
    pub request_id: String,
    pub attempt_id: String,
    pub worker_generation: String,
    pub lease_owner: String,
    pub lease_token: String,
    pub expected_attempt_revision: i64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UnknownEffectRecoveryDisposition { AuthorizeRiskRetry, KeepUnknownBlocked }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct UnknownEffectRecoveryDecision {
    pub resolution_id: String,
    pub targets: Vec<UnknownEffectRecoveryTarget>,
    pub disposition: UnknownEffectRecoveryDisposition,
    pub reason: String,
}
