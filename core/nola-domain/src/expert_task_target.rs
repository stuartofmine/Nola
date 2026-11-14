// SPDX-License-Identifier: Apache-2.0
//! Core-authored persistent expert target for the common Agent Task worker.
use crate::expert_message::ExpertParticipantReference;
use serde::{Deserialize, Serialize};

/// New dispatches originate in the actual main conversation. The legacy
/// expert shape is retained only to read and recover already frozen tasks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ExpertTaskSourceV1 {
    MainAgent(MainAgentTaskSourceV1),
    LegacyExpert(ExpertParticipantReference),
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MainAgentTaskSourceV1 {
    pub kind: MainAgentTaskSourceKind,
    pub context_conversation_id: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MainAgentTaskSourceKind {
    MainAgent,
}
impl ExpertTaskSourceV1 {
    pub fn main_agent(context: String) -> Self {
        Self::MainAgent(MainAgentTaskSourceV1 {
            kind: MainAgentTaskSourceKind::MainAgent,
            context_conversation_id: context,
        })
    }
    pub fn context_conversation_id(&self) -> &str {
        match self {
            Self::MainAgent(source) => &source.context_conversation_id,
            Self::LegacyExpert(source) => &source.context_conversation_id,
        }
    }
    pub fn legacy_expert(&self) -> Option<&ExpertParticipantReference> {
        match self {
            Self::LegacyExpert(source) => Some(source),
            _ => None,
        }
    }
    pub fn is_main_agent(&self) -> bool {
        matches!(self, Self::MainAgent(_))
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MainExpertSelectionV1 {
    pub agent_id: String,
    pub agent_revision: i64,
    #[serde(default)]
    pub participant_id: Option<String>,
    #[serde(default)]
    pub context_conversation_id: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MainExpertSelectionContextV1 {
    pub version: u32,
    pub selected_experts: Vec<MainExpertSelectionV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_source_store_epoch: Option<String>,
}
pub const MAIN_EXPERT_SELECTION_COMPONENT_ID: &str = "nola.main-expert-selection";

pub const EXPERT_TASK_TARGET_METADATA_KEY: &str = "expert_target";
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpertTaskTargetV1 {
    pub version: u32,
    pub task_id: String,
    pub exchange_id: String,
    pub parent_conversation_id: String,
    pub room_id: String,
    pub source: ExpertTaskSourceV1,
    pub target: ExpertParticipantReference,
    pub source_run_id: String,
    pub source_call_id: String,
}
impl ExpertTaskTargetV1 {
    pub fn validate(&self) -> Result<(), String> {
        let valid = |value: &str| {
            !value.is_empty()
                && value.len() <= 512
                && value.trim() == value
                && !value.chars().any(char::is_control)
        };
        if self.version != 1
            || self.task_id != self.exchange_id
            || [&self.task_id, &self.room_id, &self.source_run_id]
                .iter()
                .any(|value| uuid::Uuid::parse_str(value).is_err())
            || [
                &self.parent_conversation_id,
                &self.source_call_id,
                &self.target.participant_id,
                self.source.context_conversation_id(),
                &self.target.context_conversation_id,
            ]
            .iter()
            .any(|value| !valid(value))
            || self.target.agent_revision < 1
            || self.source.context_conversation_id() == self.target.context_conversation_id
            || self.parent_conversation_id == self.target.context_conversation_id
            || self.source.legacy_expert().is_some_and(|source| {
                source.agent_revision < 1
                    || source.participant_id == self.target.participant_id
                    || self.parent_conversation_id == source.context_conversation_id
            })
            || self.source.is_main_agent()
                && self.source.context_conversation_id() != self.parent_conversation_id
        {
            return Err("invalid_expert_task_target_v1".into());
        }
        Ok(())
    }
}

/// One cross-platform schema. Source identity and authority are injected by the Host, never arguments.
pub const EXPERT_EXCHANGE_DESCRIPTION: &str = "The main agent may delegate bounded work to experts attached to this conversation. Use directory for exact participant IDs, request to create a durable shared Task, and read/wait/cancel to manage it. Experts only execute their assigned work and return results; they cannot dispatch experts or spawn children. Target models, tools, skills and permissions remain independently frozen. An accepted request is not a result. The promoted task_id is also exchange_id; reuse that exact ID for read, wait or cancel. Never supply source identities or credentials.";

pub fn tool_schema() -> serde_json::Value {
    serde_json::json!({"type":"function","function":{"name":"delegate_expert",
        "description":EXPERT_EXCHANGE_DESCRIPTION,
        "parameters":{"type":"object","properties":{
            "action":{"type":"string","enum":["directory","request","read","wait","cancel"]},
            "target_participant_id":{"type":"string"},"text":{"type":"string","maxLength":262144},
            "exchange_id":{"type":"string"},"in_reply_to":{"type":"string"},
            "timeout_ms":{"type":"integer","minimum":1,"maximum":30000}},"required":["action"],"additionalProperties":false}}})
}

/// Exact control-schema identities, never inferred from user text.
pub fn expert_leaf_control(name: &str) -> bool {
    matches!(
        name,
        "spawn_agent"
            | "delegate_agent"
            | "delegate_expert"
            | "expert_exchange"
            | "read_task"
            | "wait_task"
            | "send_agent_message"
            | "cancel_task"
            | "request_user_input"
    )
}
pub fn schema_tool_name(tool: &serde_json::Value) -> Option<&str> {
    tool.pointer("/function/name")
        .or_else(|| tool.get("name"))
        .and_then(serde_json::Value::as_str)
}
