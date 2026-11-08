// SPDX-License-Identifier: Apache-2.0
//! Explicit user recovery control; never exposed as a model tool.
use nola_domain::unknown_effect_recovery::*;
use serde::{Deserialize, Serialize};

/// Read-only, exact-owner recovery controls. Fencing fields are carried back
/// unchanged on a decision, never rendered as user text or treated as a lease.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnknownEffectRecoveryInspection {
    pub conversation_id: String,
    pub run_id: String,
    pub assistant_message_id: String,
    pub invocation_config_digest: String,
    pub targets: Vec<UnknownEffectRecoveryTarget>,
    pub independent_pending_effects: u64,
    /// False for unconfirmed cancellation: retaining unknown is the only choice.
    pub retry_allowed: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolveUnknownEffectRecovery {
    pub conversation_id: String,
    pub run_id: String,
    pub invocation_config_digest: String,
    pub resolution_id: String,
    pub targets: Vec<UnknownEffectRecoveryTarget>,
    pub choice: UnknownEffectRecoveryChoice,
    pub resolved_at_ms: i64,
}
impl ResolveUnknownEffectRecovery {
    pub fn validate(&self) -> Result<(), String> {
        // Identities are opaque, exact values. Never normalize them: doing so
        // would let the displayed owner and the fenced mutation disagree.
        let identity=|value:&str| !value.is_empty() && value.trim()==value
            && value.len()<=512 && !value.chars().any(char::is_control);
        if [&self.conversation_id,&self.run_id,&self.invocation_config_digest,&self.resolution_id].iter().any(|value|!identity(value))
            || self.resolved_at_ms<0 || self.targets.is_empty() || self.targets.len()>256 {
            return Err("recovery requires exact bounded owner identities".into());
        }
        for target in &self.targets {
            if [&target.effect_id,&target.request_id,&target.attempt_id,&target.worker_generation,&target.lease_owner,&target.lease_token].iter().any(|value|!identity(value))
                || target.expected_attempt_revision<1 || target.expected_attempt_revision==i64::MAX {
                return Err("recovery target requires exact attempt ownership and revision".into());
            }
        }
        match &self.choice {
            UnknownEffectRecoveryChoice::AuthorizeRiskRetry{reason} | UnknownEffectRecoveryChoice::KeepUnknownBlocked{reason}
                if reason.trim().is_empty() || reason.len()>4096 => Err("explicit bounded recovery reason required".into()),
            UnknownEffectRecoveryChoice::VerifyAppliedReceipt{host_event_id,response_json}
                if !identity(host_event_id) || response_json.is_empty() || response_json.len()>4*1024*1024 => Err("bounded original Host receipt required".into()),
            _=>Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> ResolveUnknownEffectRecovery {
        ResolveUnknownEffectRecovery {
            conversation_id: "conversation".into(), run_id: "run".into(),
            invocation_config_digest: "digest".into(), resolution_id: "resolution".into(),
            resolved_at_ms: 1,
            targets: vec![UnknownEffectRecoveryTarget {
                effect_id: "effect".into(), request_id: "request".into(), attempt_id: "attempt".into(),
                worker_generation: "generation".into(), lease_owner: "owner".into(),
                lease_token: "token".into(), expected_attempt_revision: 1,
            }],
            choice: UnknownEffectRecoveryChoice::AuthorizeRiskRetry { reason: "Explicit risk accepted".into() },
        }
    }

    #[test]
    fn recovery_rejects_ambiguous_identities_without_normalizing_them() {
        assert!(input().validate().is_ok());
        for invalid in ["", " conversation", "conversation ", "run\nother", "run\0other"] {
            let mut request = input();
            request.conversation_id = invalid.into();
            assert!(request.validate().is_err());
            assert_eq!(request.conversation_id, invalid);
            let mut request = input();
            request.targets[0].lease_token = invalid.into();
            assert!(request.validate().is_err());
            assert_eq!(request.targets[0].lease_token, invalid);
        }
    }

    #[test]
    fn decision_does_not_accept_missing_fences_or_extra_fields() {
        let request = serde_json::to_value(input()).unwrap();
        let mut missing = request.clone();
        missing["targets"][0].as_object_mut().unwrap().remove("lease_token");
        assert!(serde_json::from_value::<ResolveUnknownEffectRecovery>(missing).is_err());
        let mut extra = request;
        extra["choice"]["mark_success"] = serde_json::json!(true);
        assert!(serde_json::from_value::<ResolveUnknownEffectRecovery>(extra).is_err());
    }

    #[test]
    fn inspection_requires_exact_owner_and_fences_without_extra_fields() {
        let decision = input();
        let inspection = UnknownEffectRecoveryInspection {
            conversation_id: decision.conversation_id,
            run_id: decision.run_id,
            assistant_message_id: "assistant".into(),
            invocation_config_digest: decision.invocation_config_digest,
            targets: decision.targets,
            independent_pending_effects: 0, retry_allowed: true,
        };
        let value = serde_json::to_value(inspection).unwrap();
        assert!(serde_json::from_value::<UnknownEffectRecoveryInspection>(value.clone()).is_ok());
        for field in ["conversation_id", "run_id", "assistant_message_id", "invocation_config_digest"] {
            let mut missing = value.clone();
            missing.as_object_mut().unwrap().remove(field);
            assert!(serde_json::from_value::<UnknownEffectRecoveryInspection>(missing).is_err());
        }
        let mut missing_fence = value.clone();
        missing_fence["targets"][0].as_object_mut().unwrap().remove("lease_token");
        assert!(serde_json::from_value::<UnknownEffectRecoveryInspection>(missing_fence).is_err());
        let mut extra = value;
        extra["automatically_retry"] = serde_json::json!(true);
        assert!(serde_json::from_value::<UnknownEffectRecoveryInspection>(extra).is_err());
    }

    #[test]
    fn recovery_reason_limit_counts_utf8_bytes_not_characters() {
        let mut request = input();
        request.choice = UnknownEffectRecoveryChoice::AuthorizeRiskRetry { reason: "a".repeat(4096) };
        assert!(request.validate().is_ok());
        request.choice = UnknownEffectRecoveryChoice::AuthorizeRiskRetry { reason: "a".repeat(4097) };
        assert!(request.validate().is_err());
        request.choice = UnknownEffectRecoveryChoice::KeepUnknownBlocked { reason: "理".repeat(1365) };
        assert!(request.validate().is_ok());
        request.choice = UnknownEffectRecoveryChoice::KeepUnknownBlocked { reason: "理".repeat(1366) };
        assert!(request.validate().is_err());
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag="kind",rename_all="snake_case",deny_unknown_fields)]
pub enum UnknownEffectRecoveryChoice {
    AuthorizeRiskRetry { reason: String },
    KeepUnknownBlocked { reason: String },
    /// Exact already-accepted terminal receipt, not supplied evidence to execute.
    VerifyAppliedReceipt { host_event_id: String, response_json: String },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag="outcome",rename_all="snake_case",deny_unknown_fields)]
pub enum UnknownEffectRecoveryReply {
    RetryAuthorized { resolution_id: String, run_id: String },
    UnknownPreservedBlocked { resolution_id: String, run_id: String },
    AppliedReceiptVerified { resolution_id: String, run_id: String },
    Rejected { code: String },
    Conflict { code: String },
}
