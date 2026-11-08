// SPDX-License-Identifier: Apache-2.0
//! Application-owned revocation evidence, never an OS stop observation.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CancellationRevokedAttempt {
    pub attempt_id: String,
    pub revision: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CancellationTargetNotStarted {
    pub conversation_id: String,
    pub run_id: String,
    pub core_session_id: String,
    pub host_id: String,
    pub cancellation_effect_id: String,
    pub cancellation_request_id: String,
    pub target_effect_id: String,
    pub target_request_id: String,
    pub target_payload_identity: String,
    pub checkpoint_revision: i64,
    pub revoked_attempts: Vec<CancellationRevokedAttempt>,
}

impl CancellationTargetNotStarted {
    pub fn validate(&self) -> Result<(), String> {
        for id in [&self.conversation_id, &self.run_id, &self.core_session_id, &self.host_id,
            &self.cancellation_effect_id, &self.cancellation_request_id,
            &self.target_effect_id, &self.target_request_id] {
            if id.is_empty() || id.trim() != id || id.len() > 1024 || id.chars().any(char::is_control) {
                return Err("invalid cancellation NotStarted identity".into());
            }
        }
        if self.cancellation_effect_id == self.target_effect_id || self.checkpoint_revision <= 0
            || self.target_payload_identity.len() != 64
            || !self.target_payload_identity.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            || self.revoked_attempts.len() > 256 {
            return Err("invalid cancellation NotStarted source".into());
        }
        let mut ids = std::collections::BTreeSet::new();
        for attempt in &self.revoked_attempts {
            if attempt.attempt_id.is_empty() || attempt.attempt_id.trim() != attempt.attempt_id
                || attempt.attempt_id.len() > 1024 || attempt.attempt_id.chars().any(char::is_control)
                || attempt.revision <= 0 || !ids.insert(&attempt.attempt_id) {
                return Err("invalid cancellation NotStarted revocation".into());
            }
        }
        Ok(())
    }

    pub fn matches_control(&self, host: &str, session: &str, run: Option<&str>,
        effect: &str, request: &str, target: &str) -> bool {
        self.validate().is_ok() && self.host_id == host && self.core_session_id == session
            && run == Some(self.run_id.as_str()) && self.cancellation_effect_id == effect
            && self.cancellation_request_id == request && self.target_effect_id == target
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host_effect::HostRequest;

    #[test]
    fn cancellation_wire_requires_explicit_nullable_proof_and_rejects_malformed_sources() {
        assert!(serde_json::from_value::<HostRequest>(serde_json::json!({
            "type":"cancel","target_effect_id":"target"
        })).is_err());
        let request: HostRequest = serde_json::from_value(serde_json::json!({
            "type":"cancel","target_effect_id":"target","target_not_started":null
        })).unwrap();
        assert!(matches!(request,HostRequest::Cancel {target_not_started:None,..}));
        let source = serde_json::json!({
            "conversation_id":"conversation","run_id":"run","core_session_id":"core","host_id":"host",
            "cancellation_effect_id":"cancel","cancellation_request_id":"request",
            "target_effect_id":"target","target_request_id":"target-request",
            "target_payload_identity":"a".repeat(64),"checkpoint_revision":2,
            "revoked_attempts":[{"attempt_id":"attempt","revision":2}]
        });
        let proof: CancellationTargetNotStarted = serde_json::from_value(source.clone()).unwrap();
        proof.validate().unwrap();
        assert!(proof.matches_control("host","core",Some("run"),"cancel","request","target"));
        for key in ["conversation_id","target_request_id","revoked_attempts"] {
            let mut missing=source.clone();missing.as_object_mut().unwrap().remove(key);
            assert!(serde_json::from_value::<CancellationTargetNotStarted>(missing).is_err(),"{key}");
        }
        for bad in ["", " padded", "control\n", &"x".repeat(1025)] {
            let mut invalid=proof.clone();invalid.revoked_attempts[0].attempt_id=bad.into();
            assert!(invalid.validate().is_err());
        }
        let mut duplicate=proof.clone();duplicate.revoked_attempts.push(duplicate.revoked_attempts[0].clone());
        assert!(duplicate.validate().is_err());
        let mut excess=proof;
        excess.revoked_attempts=(0..257).map(|i|CancellationRevokedAttempt{attempt_id:format!("attempt-{i}"),revision:1}).collect();
        assert!(excess.validate().is_err(),"history may not be truncated to manufacture a proof");
    }
}
