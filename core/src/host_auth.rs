// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Shared host permission challenge contract.

use serde::{Deserialize, Serialize};

pub use nola_host_protocol::{
    HostPermissionAuthorizationDecision, HostPermissionIntervention, HostPermissionResolution,
};

pub const HOST_PERMISSION_PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostPermissionChallenge {
    pub allow_conversation: bool,
    pub protocol_version: u32,
    pub nonce: String,
    pub request_id: String,
    pub host_id: String,
    pub client_id: String,
    pub session_id: String,
    pub tool_name: String,
    pub expires_at: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub permission_instructions: Vec<nola_host_protocol::PermissionInstruction>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostPermissionResponse {
    #[serde(default)]
    pub protocol_version: Option<u32>,
    pub nonce: String,
    pub request_id: String,
    pub host_id: String,
    pub client_id: String,
    pub session_id: String,
    pub tool_name: String,
    pub resolution: HostPermissionResolution,
}

pub fn validate_host_permission_decision(
    challenge: &HostPermissionChallenge,
    decision: &HostPermissionResponse,
    now: u64,
) -> Result<(), String> {
    if challenge.protocol_version != HOST_PERMISSION_PROTOCOL_VERSION
        || decision.protocol_version != Some(challenge.protocol_version) {
        return Err("host permission protocol version does not match".to_string());
    }
    if challenge.expires_at <= now {
        return Err("host permission challenge expired".to_string());
    }
    if challenge.nonce != decision.nonce {
        return Err("host permission nonce does not match".to_string());
    }
    if challenge.request_id != decision.request_id {
        return Err("host permission request ID does not match".to_string());
    }
    if challenge.host_id != decision.host_id {
        return Err("host identity does not match".to_string());
    }
    if challenge.client_id != decision.client_id {
        return Err("mobile client identity does not match".to_string());
    }
    if challenge.session_id != decision.session_id {
        return Err("host permission session does not match".to_string());
    }
    if challenge.tool_name != decision.tool_name {
        return Err("host permission tool does not match".to_string());
    }
    if !challenge.allow_conversation && matches!(&decision.resolution, HostPermissionResolution::Authorization {
        decision: HostPermissionAuthorizationDecision::AllowConversation, ..
    }) { return Err("conversation approval is not available for this invocation; choose allow once".into()); }
    if let HostPermissionResolution::Intervention {
        intervention: HostPermissionIntervention::ProvideInstruction { instruction },
    } = &decision.resolution
    {
        nola_host_protocol::PermissionInstruction::validate_append(
            &challenge.permission_instructions,
            &challenge.request_id,
            instruction,
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn challenge_binds_every_security_field() {
        let challenge = HostPermissionChallenge {
            allow_conversation: true,
            protocol_version: HOST_PERMISSION_PROTOCOL_VERSION,
            nonce: "nonce".into(),
            request_id: "call".into(),
            host_id: "host".into(),
            client_id: "client".into(),
            session_id: "session".into(),
            tool_name: "bash".into(),
            expires_at: 200,
            permission_instructions: Vec::new(),
        };
        let decision = HostPermissionResponse {
            protocol_version: Some(HOST_PERMISSION_PROTOCOL_VERSION),
            nonce: "nonce".into(),
            request_id: "call".into(),
            host_id: "host".into(),
            client_id: "client".into(),
            session_id: "session".into(),
            tool_name: "bash".into(),
            resolution: HostPermissionResolution::allow_once(),
        };
        assert!(validate_host_permission_decision(&challenge, &decision, 100).is_ok());
        let mut missing_protocol = decision.clone();
        missing_protocol.protocol_version = None;
        assert!(validate_host_permission_decision(&challenge, &missing_protocol, 100).is_err());
        assert!(validate_host_permission_decision(&challenge, &decision, 201).is_err());
        let mut wrong_protocol = decision.clone();
        wrong_protocol.protocol_version = Some(HOST_PERMISSION_PROTOCOL_VERSION + 1);
        assert!(validate_host_permission_decision(&challenge, &wrong_protocol, 100).is_err());
        let mut empty_instruction = decision.clone();
        empty_instruction.resolution = HostPermissionResolution::provide_instruction("   ");
        assert!(validate_host_permission_decision(&challenge, &empty_instruction, 100).is_err());
        assert!(
            serde_json::from_value::<HostPermissionResponse>(serde_json::json!({
                "protocol_version": HOST_PERMISSION_PROTOCOL_VERSION,
                "nonce": "nonce",
                "request_id": "call",
                "host_id": "host",
                "client_id": "client",
                "session_id": "session",
                "tool_name": "bash",
                "decision": "allow_once",
                "grant": null,
                "instruction": null
            }))
            .is_err()
        );
        let mut replay = decision;
        replay.request_id = "changed".into();
        assert!(validate_host_permission_decision(&challenge, &replay, 100).is_err());
    }
}

#[cfg(test)]
mod instruction_challenge_tests {
    use super::*;
    fn current_challenge() -> HostPermissionChallenge {
        serde_json::from_value(serde_json::json!({"allow_conversation":true,"protocol_version":HOST_PERMISSION_PROTOCOL_VERSION,"nonce":"n","request_id":"successor","host_id":"host","client_id":"client","session_id":"session","tool_name":"write_file","expires_at":200})).unwrap()
    }
    fn response(
        challenge: &HostPermissionChallenge,
        resolution: HostPermissionResolution,
    ) -> HostPermissionResponse {
        HostPermissionResponse {
            protocol_version: Some(challenge.protocol_version),
            nonce: challenge.nonce.clone(),
            request_id: challenge.request_id.clone(),
            host_id: challenge.host_id.clone(),
            client_id: challenge.client_id.clone(),
            session_id: challenge.session_id.clone(),
            tool_name: challenge.tool_name.clone(),
            resolution,
        }
    }
    #[test]
    fn empty_guidance_is_not_authority() {
        let challenge = current_challenge();
        assert!(challenge.permission_instructions.is_empty());
        assert!(serde_json::to_value(&challenge)
            .unwrap()
            .get("permission_instructions")
            .is_none());
        let decision = response(
            &challenge,
            HostPermissionResolution::provide_instruction("keep original arguments"),
        );
        let before = serde_json::to_value(&decision).unwrap();
        validate_host_permission_decision(&challenge, &decision, 100).unwrap();
        assert_eq!(serde_json::to_value(&decision).unwrap(), before);
        assert!(matches!(
            decision.resolution,
            HostPermissionResolution::Intervention { .. }
        ));
        assert!(before["resolution"].get("decision").is_none());
    }
    #[test]
    fn once_only_challenge_rejects_conversation_without_downgrading_and_requires_the_typed_field() {
        let mut challenge = current_challenge();
        challenge.allow_conversation = false;
        assert!(validate_host_permission_decision(&challenge,&response(&challenge,
            HostPermissionResolution::allow_once()),100).is_ok());
        assert!(validate_host_permission_decision(&challenge,&response(&challenge,
            HostPermissionResolution::allow_conversation(None)),100).is_err());
        assert!(validate_host_permission_decision(&challenge,&response(&challenge,
            HostPermissionResolution::deny()),100).is_ok());
        let mut wire = serde_json::to_value(challenge).unwrap();
        wire.as_object_mut().unwrap().remove("allow_conversation");
        assert!(serde_json::from_value::<HostPermissionChallenge>(wire).is_err());
    }
    #[test]
    fn exhausted_guidance_rejects_only_new_instruction_and_preserves_explicit_choices() {
        let mut challenge = current_challenge();
        challenge.permission_instructions = (0..16)
            .map(|id| nola_host_protocol::PermissionInstruction {
                permission_request_id: format!("old-{id}"),
                instruction: "keep original".into(),
            })
            .collect();
        let before = serde_json::to_value(&challenge).unwrap();
        let rejected = response(
            &challenge,
            HostPermissionResolution::provide_instruction("one more"),
        );
        assert!(validate_host_permission_decision(&challenge, &rejected, 100).is_err());
        assert!(matches!(
            rejected.resolution,
            HostPermissionResolution::Intervention { .. }
        ));
        for resolution in [
            HostPermissionResolution::allow_once(),
            HostPermissionResolution::allow_conversation(None),
            HostPermissionResolution::deny(),
        ] {
            validate_host_permission_decision(&challenge, &response(&challenge, resolution), 100)
                .unwrap();
        }
        assert_eq!(serde_json::to_value(&challenge).unwrap(), before);
        let roundtrip: HostPermissionChallenge = serde_json::from_value(before).unwrap();
        assert_eq!(
            roundtrip.permission_instructions,
            challenge.permission_instructions
        );
    }
    #[test]
    fn instruction_budget_and_metadata_do_not_bypass_challenge_identity() {
        let challenge = current_challenge();
        let valid = response(
            &challenge,
            HostPermissionResolution::provide_instruction("bounded"),
        );
        for field in [
            "nonce",
            "request_id",
            "host_id",
            "client_id",
            "session_id",
            "tool_name",
        ] {
            let mut encoded = serde_json::to_value(&valid).unwrap();
            encoded[field] = serde_json::json!("other");
            let changed = serde_json::from_value(encoded).unwrap();
            assert!(
                validate_host_permission_decision(&challenge, &changed, 100).is_err(),
                "{field}"
            );
        }
        assert!(validate_host_permission_decision(&challenge, &valid, 200).is_err());
        assert!(validate_host_permission_decision(
            &challenge,
            &response(
                &challenge,
                HostPermissionResolution::provide_instruction("字".repeat(1366))
            ),
            100
        )
        .is_err());
        let mut near = challenge.clone();
        near.permission_instructions = vec![
            nola_host_protocol::PermissionInstruction {
                permission_request_id: "r".into(),
                instruction: "x".repeat(4095)
            };
            3
        ];
        // 12288 + new 4096 + the successor request ID exceeds the shared cap.
        assert!(validate_host_permission_decision(
            &near,
            &response(
                &near,
                HostPermissionResolution::provide_instruction("x".repeat(4096))
            ),
            100
        )
        .is_err());
    }
}
