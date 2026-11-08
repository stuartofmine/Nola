// SPDX-License-Identifier: Apache-2.0
//! Stable contracts for selecting an Agent execution backend.
//!
//! Detection and process execution belong to a Host. Core owns only the
//! portable identity, availability, session and event semantics shared by UI
//! clients and Hosts.

use serde::{Deserialize, Serialize};

pub const EXECUTION_BACKEND_SCHEMA_VERSION: u32 = 3;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionBackendModelDescriptor {
    pub id: String,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionBackendKind {
    Nola,
    Codex,
    ClaudeCode,
    OpenCode,
}

impl ExecutionBackendKind {
    pub const ALL: [Self; 4] = [Self::Nola, Self::Codex, Self::ClaudeCode, Self::OpenCode];

    pub const fn id(self) -> &'static str {
        match self {
            Self::Nola => "nola",
            Self::Codex => "codex",
            Self::ClaudeCode => "claude_code",
            Self::OpenCode => "opencode",
        }
    }

    pub const fn executable_name(self) -> Option<&'static str> {
        match self {
            Self::Nola => None,
            Self::Codex => Some("codex"),
            Self::ClaudeCode => Some("claude"),
            Self::OpenCode => Some("opencode"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionBackendAvailability {
    Available,
    NotInstalled,
    Unauthenticated,
    Incompatible,
    Unreachable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionBackendTransport {
    InProcess,
    JsonLinesProcess,
    StreamJsonProcess,
    JsonRpcStdio,
    HttpEventStream,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionBackendCapabilities {
    pub streams_text: bool,
    pub streams_reasoning: bool,
    pub streams_tools: bool,
    pub supports_resume: bool,
    pub supports_cancel: bool,
    pub supports_permission_bridge: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionBackendDescriptor {
    pub schema_version: u32,
    pub id: String,
    pub kind: ExecutionBackendKind,
    pub display_name: String,
    pub availability: ExecutionBackendAvailability,
    pub transport: ExecutionBackendTransport,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executable_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub capabilities: ExecutionBackendCapabilities,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostic: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<ExecutionBackendModelDescriptor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_model_id: Option<String>,
}

impl ExecutionBackendDescriptor {
    pub fn built_in_nola() -> Self {
        Self {
            schema_version: EXECUTION_BACKEND_SCHEMA_VERSION,
            id: ExecutionBackendKind::Nola.id().to_string(),
            kind: ExecutionBackendKind::Nola,
            display_name: "Nola".to_string(),
            availability: ExecutionBackendAvailability::Available,
            transport: ExecutionBackendTransport::InProcess,
            executable_path: None,
            version: None,
            capabilities: ExecutionBackendCapabilities {
                streams_text: true,
                streams_reasoning: true,
                streams_tools: true,
                supports_resume: true,
                supports_cancel: true,
                supports_permission_bridge: true,
            },
            diagnostic: None,
            models: Vec::new(),
            default_model_id: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionBackendSelection {
    pub schema_version: u32,
    pub backend_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend_session_id: Option<String>,
    /// Last Core-owned AgentItem known to be committed in the executor's
    /// conversation. A backend session ID without this anchor is never enough
    /// to authorize incremental context delivery.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_anchor_item_id: Option<String>,
    /// Identity of the stable instruction/model contract associated with the
    /// anchor. A mismatch forces a new bootstrap session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_identity_digest: Option<String>,
}

impl Default for ExecutionBackendSelection {
    fn default() -> Self {
        Self {
            schema_version: EXECUTION_BACKEND_SCHEMA_VERSION,
            backend_id: ExecutionBackendKind::Nola.id().to_string(),
            backend_session_id: None,
            context_anchor_item_id: None,
            context_identity_digest: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_ids_are_not_display_names() {
        assert_eq!(ExecutionBackendKind::ClaudeCode.id(), "claude_code");
        assert_eq!(
            ExecutionBackendKind::ClaudeCode.executable_name(),
            Some("claude")
        );
    }

    #[test]
    fn default_selection_uses_nola_without_an_external_session() {
        let selection = ExecutionBackendSelection::default();
        assert_eq!(selection.backend_id, "nola");
        assert_eq!(selection.backend_session_id, None);
    }
}
