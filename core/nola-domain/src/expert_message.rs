// SPDX-License-Identifier: Apache-2.0
//! Complete expert-send ownership. Definitions and model facts are data, never grants.
use crate::{
    edited_resend::{MessageSendWitness, PrepareEditedResend, PreparedEditedResend},
    persistent_agent::{AgentDefaults, AgentId},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Bound the serialized private instruction-capture fact at every storage and
/// Application read/write boundary. This is not a renderer message allowance.
pub const EXPERT_TEMPLATE_CAPTURE_MAX_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpertParticipantReference {
    pub participant_id: String,
    pub agent_id: AgentId,
    pub agent_revision: i64,
    pub context_conversation_id: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdmitExpertConversationMessage {
    pub conversation_id: String,
    pub expected_revision: i64,
    pub room_id: String,
    pub expected_room_revision: i64,
    pub submission_id: String,
    pub user_message_id: String,
    pub text: String,
    pub selected_participants: Vec<ExpertParticipantReference>,
    pub created_at_ms: i64,
}
impl AdmitExpertConversationMessage {
    pub fn validate(&self) -> Result<(), String> {
        uuid::Uuid::parse_str(&self.submission_id)
            .map_err(|_| "expert_message_requires_stable_submission_uuid")?;
        uuid::Uuid::parse_str(&self.room_id).map_err(|_| "expert_message_invalid_room")?;
        if self.conversation_id.trim().is_empty()
            || self.user_message_id.trim().is_empty()
            || self.expected_revision < 1
            || self.expected_room_revision < 1
            || self.created_at_ms < 0
            || self.text.trim().is_empty()
            || self.text.len() > 1_000_000
            || self.text.contains('\0')
            || self.selected_participants.is_empty()
            || self.selected_participants.len() > 32
        {
            return Err("expert_message_invalid_input".into());
        }
        let mut participants = BTreeSet::new();
        let mut agents = BTreeSet::new();
        let mut contexts = BTreeSet::new();
        for reference in &self.selected_participants {
            if reference.participant_id.trim().is_empty()
                || reference.context_conversation_id.trim().is_empty()
                || reference.agent_revision < 1
                || !participants.insert(&reference.participant_id)
                || !agents.insert(&reference.agent_id)
                || !contexts.insert(&reference.context_conversation_id)
            {
                return Err("expert_message_invalid_or_duplicate_selected_participant".into());
            }
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpertModelDeclaration {
    pub profile_id: String,
    pub profile_revision: i64,
    pub provider: String,
    pub model_name: String,
    pub runtime: String,
    pub parameters: serde_json::Value,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpertPromptDefinition {
    pub component_id: String,
    pub content: String,
    pub content_sha256: String,
    pub definition_revision: Option<u64>,
    pub platform: serde_json::Value,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpertInstructionWitness {
    pub checkpoint_sha256: String,
    pub component_payload_digest: String,
    /// Only typed PromptTemplate definitions. No system/Host policy/history/resources.
    pub prompt_definitions: Vec<ExpertPromptDefinition>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpertMessageEntry {
    pub participant: ExpertParticipantReference,
    pub display_name: String,
    pub description: String,
    pub defaults: AgentDefaults,
    pub model_declaration: ExpertModelDeclaration,
    pub context_configuration_revision: i64,
    pub child_submission_id: String,
    pub child_user_message_id: String,
    pub child_assistant_message_id: String,
    pub expected_prompt_definition: Option<ExpertPromptDefinition>,
    pub model_witness: Option<MessageSendWitness>,
    pub instruction_witness: Option<ExpertInstructionWitness>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpertMessageManifest {
    pub version: u32,
    pub input: AdmitExpertConversationMessage,
    pub entries: Vec<ExpertMessageEntry>,
}
impl ExpertMessageManifest {
    pub fn validate(&self) -> Result<(), String> {
        self.input.validate()?;
        if self.version != 1
            || self.entries.len() != self.input.selected_participants.len()
            || self
                .entries
                .iter()
                .map(|e| &e.participant)
                .ne(self.input.selected_participants.iter())
        {
            return Err("expert_message_manifest_is_not_complete_selected_set".into());
        }
        let mut submissions = BTreeSet::new();
        for entry in &self.entries {
            uuid::Uuid::parse_str(&entry.child_submission_id)
                .map_err(|_| "expert_message_invalid_child_identity")?;
            if !submissions.insert(&entry.child_submission_id)
                || entry.child_user_message_id != format!("{}:user", entry.child_submission_id)
                || entry.child_assistant_message_id
                    != format!("{}:assistant", entry.child_submission_id)
                || entry.model_declaration.profile_id.is_empty()
                || entry.model_declaration.profile_revision < 1
            {
                return Err("expert_message_child_owner_mismatch".into());
            }
            if let Some(witness) = &entry.model_witness {
                if witness.conversation_id != entry.participant.context_conversation_id
                    || witness.message_id != entry.child_user_message_id
                    || witness.assistant_message_id != entry.child_assistant_message_id
                    || witness.run_id != entry.child_submission_id
                {
                    return Err("expert_message_child_model_owner_mismatch".into());
                }
            }
        }
        Ok(())
    }
    pub fn require_complete_source(&self) -> Result<(), String> {
        self.validate()?;
        if self
            .entries
            .iter()
            .any(|entry| entry.model_witness.is_none() || entry.instruction_witness.is_none())
        {
            return Err("expert_message_complete_source_witness_required".into());
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedExpertResendEntry {
    pub original: ExpertMessageEntry,
    pub participant: ExpertParticipantReference,
    pub prepared: PreparedEditedResend,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedExpertResend {
    pub input: PrepareEditedResend,
    pub source_submission_id: String,
    pub room_id: String,
    pub parent_message_id: String,
    pub entries: Vec<PreparedExpertResendEntry>,
}

/// One pending canonical parent owns a participant context before its Run exists.
/// Terminal state stays in the original Room/Run facts, never a client busy flag.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpertContextReservation {
    pub room_id: String,
    pub parent_conversation_id: String,
    pub parent_message_id: String,
    pub context_conversation_id: String,
    pub child_submission_id: String,
}

/// Match distinct selection/profile and transport domains without exposing a
/// credential handle or copying a prior Host configuration.
pub fn validate_expert_model_route(
    declaration: &ExpertModelDeclaration,
    source: &serde_json::Value,
    defaults: &AgentDefaults,
    route: &serde_json::Value,
) -> Result<(), String> {
    use sha2::{Digest, Sha256};
    match route.get("kind").and_then(|v| v.as_str()) {
        Some("native") => {
            let reference = route.get("profile_ref").filter(|v| !v.is_null());
            let expected = if reference.is_some() {
                &declaration.profile_id
            } else {
                &declaration.model_name
            };
            if route.get("model_id").and_then(|v| v.as_str()) != Some(expected.as_str())
                || reference.is_some_and(|reference| {
                    reference.get("profile_id").and_then(|v| v.as_str())
                        != Some(declaration.profile_id.as_str())
                        || reference.get("admitted_revision").and_then(|v| v.as_i64())
                            != Some(declaration.profile_revision)
                })
            {
                return Err("expert_message_actual_native_profile_changed".into());
            }
        }
        Some("http") => {
            let configuration = defaults.conversation_configuration();
            let selection = configuration
                .get("modelSelectionID")
                .and_then(|v| v.as_str())
                .ok_or("expert_message_selected_model_identity_missing")?;
            let protocol = configuration
                .get("modelOverrides")
                .and_then(|v| v.get(selection))
                .and_then(|v| v.get("wireProtocol"))
                .and_then(|v| v.as_str())
                .or_else(|| source.get("wireProtocol").and_then(|v| v.as_str()))
                .ok_or("expert_message_declared_protocol_missing")?;
            let protocol = crate::generation::ModelProtocol::parse(protocol)?;
            let mut endpoint = url::Url::parse(
                source
                    .get("baseURL")
                    .and_then(|v| v.as_str())
                    .ok_or("expert_message_declared_endpoint_missing")?,
            )
            .map_err(|_| "expert_message_declared_endpoint_invalid")?;
            if !matches!(endpoint.scheme(), "http" | "https")
                || !endpoint.username().is_empty()
                || endpoint.password().is_some()
                || endpoint.fragment().is_some()
            {
                return Err("expert_message_declared_endpoint_invalid".into());
            }
            let query = endpoint
                .query()
                .map(|query| format!("{:x}", Sha256::digest(query.as_bytes())));
            endpoint.set_query(None);
            if route.get("model").and_then(|v| v.as_str()) != Some(declaration.model_name.as_str())
                || route.get("protocol").and_then(|v| v.as_str()) != Some(protocol.as_str())
                || route.get("endpoint").and_then(|v| v.as_str()) != Some(endpoint.as_str())
                || route.get("endpoint_query_sha256").and_then(|v| v.as_str()) != query.as_deref()
            {
                return Err("expert_message_actual_http_route_differs_from_declaration".into());
            }
        }
        _ => return Err("expert_message_unsupported_declared_transport".into()),
    }
    Ok(())
}
