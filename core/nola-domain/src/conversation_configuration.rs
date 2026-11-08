// SPDX-License-Identifier: Apache-2.0
//! Configuration commit outcomes preserve owner and revision identity.
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all="snake_case")]
pub enum ConversationTitleSource { Automatic, FirstMessage, User }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConversationConfigurationSnapshot {
    pub conversation_id: String,
    pub configuration_revision: i64,
    pub execution_host_raw_value: Value,
    pub execution_host_binding: Value,
    pub client_host_parameters: Value,
    pub client_model_presentation: Value,
    #[serde(rename = "modelSelectionID")]
    pub model_selection_id: Value,
    pub knowledge: Value,
    pub retrieval_enabled_override: Value,
    pub memory_enabled_override: Value,
    pub is_pinned: bool,
    pub model_overrides: Value,
    pub permission_profile: Value,
    pub language: Value,
    #[serde(rename = "selectedPromptTemplateID")]
    pub selected_prompt_template_id: Value,
    pub data_source_overrides: Value,
    pub data_source_ids: Value,
    pub execution_backend: Value,
    pub external_execution_sessions: Value,
    pub persistence_enabled: Value,
    pub prompt_response_style: Value,
    pub prompt_template_selection_is_explicit: Value,
    pub context_limit_override: Value,
    #[serde(rename = "externalExecutionModelIDs")]
    pub external_execution_model_ids: Value,
    #[serde(rename = "externalExecutionModelProviderIDs")]
    pub external_execution_model_provider_ids: Value,
    pub execution_policy: Option<crate::execution_policy::ConversationExecutionPolicy>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum ConfigurationCommitResult {
    Committed { snapshot: ConversationConfigurationSnapshot },
    Conflict { conversation_id: String, expected_revision: i64, actual_revision: i64 },
    NotFound { conversation_id: String },
}

impl ConfigurationCommitResult {
    pub fn into_committed(self) -> Result<ConversationConfigurationSnapshot, Self> {
        match self { Self::Committed { snapshot } => Ok(snapshot), other => Err(other) }
    }
    pub fn is_committed(&self) -> bool { matches!(self, Self::Committed { .. }) }
    pub fn is_conflict(&self) -> bool { matches!(self, Self::Conflict { .. }) }
}
