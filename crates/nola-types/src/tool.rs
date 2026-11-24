// SPDX-License-Identifier: Apache-2.0
//! Strongly-typed tool contracts:
//! - `ToolModelPayload`: makes silent `outcome.text` loss unrepresentable.
//! - `VersionTransition`: makes missing `before_version`/`after_version` unrepresentable.
//! - `ToolSpec` + `ToolTier` + `CapabilityDomain`: single source of truth for tool catalog.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::MediaAttachment;

/// Explicit sum type for what the LLM receives from a tool call.
/// Eliminates the v1 bug where `ok(text).with_data(meta)` silently dropped `text`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ToolModelPayload {
    /// Plain text sent verbatim to the model.
    Text(String),
    /// Structured JSON sent verbatim to the model; `ui_summary` is strictly for
    /// human CLI/Web display and never confuses model payload fields.
    Structured {
        model_data: Value,
        ui_summary: String,
    },
}

impl ToolModelPayload {
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text(text.into())
    }

    pub fn structured(model_data: Value, ui_summary: impl Into<String>) -> Self {
        Self::Structured {
            model_data,
            ui_summary: ui_summary.into(),
        }
    }

    /// Canonical value projected into the LLM `role: "tool"` message.
    pub fn to_model_value(&self) -> Value {
        match self {
            Self::Text(text) => json!({ "text": text }),
            Self::Structured { model_data, .. } => model_data.clone(),
        }
    }

    /// Canonical string serialized for Chat Completions / Responses `tool` content.
    pub fn to_model_string(&self) -> String {
        match self {
            Self::Text(text) => text.clone(),
            Self::Structured { model_data, .. } => {
                serde_json::to_string(model_data).unwrap_or_else(|_| "{}".to_string())
            }
        }
    }

    /// Human-readable one-line summary for TUI/Web UI.
    pub fn ui_summary(&self) -> &str {
        match self {
            Self::Text(text) => text.lines().next().unwrap_or(""),
            Self::Structured { ui_summary, .. } => ui_summary,
        }
    }
}

/// Authoritative content version observed by the Host before or after a mutation.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum ContentVersion {
    Sha256(String),
    Fingerprint { size: u64, digest: u64 },
    GitCommit(String),
    Revision(u64),
}

/// Exhaustive file/resource version transition.
/// Compile-time impossible for a mutation to drop `before` or `after` accidentally.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum VersionTransition {
    Created {
        after: ContentVersion,
    },
    Modified {
        before: ContentVersion,
        after: ContentVersion,
    },
    Deleted {
        before: ContentVersion,
    },
    Unchanged {
        version: ContentVersion,
    },
    Unobservable {
        reason: String,
    },
}

impl VersionTransition {
    /// Returns true whenever a concrete state change occurred (create, modify, or delete).
    pub fn changed(&self) -> bool {
        match self {
            Self::Created { .. } | Self::Deleted { .. } => true,
            Self::Modified { before, after } => before != after,
            Self::Unchanged { .. } | Self::Unobservable { .. } => false,
        }
    }
}

/// Canonical resource mutation operation (shared by Tools, Engine, and Audit).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceOperation {
    Create,
    Edit,
    Delete,
    Move,
    Commit,
    Mkdir,
    Custom(String),
}

/// Typed mutation effect recorded by `FileMutationPipeline` / `ShellExecutionPipeline`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceEffect {
    pub uri: String,
    pub operation: ResourceOperation,
    pub transition: VersionTransition,
}

/// Unified outcome returned by every tool implementation in `nola-tools` and `nola-mcp`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolExecutionResult {
    pub payload: ToolModelPayload,
    pub is_error: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<ResourceEffect>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub media: Vec<MediaAttachment>,
}

impl ToolExecutionResult {
    pub fn ok_text(text: impl Into<String>) -> Self {
        Self {
            payload: ToolModelPayload::text(text),
            is_error: false,
            effects: Vec::new(),
            media: Vec::new(),
        }
    }

    pub fn ok_structured(model_data: Value, ui_summary: impl Into<String>) -> Self {
        Self {
            payload: ToolModelPayload::structured(model_data, ui_summary),
            is_error: false,
            effects: Vec::new(),
            media: Vec::new(),
        }
    }

    pub fn err_structured(code: &str, message: impl Into<String>, extra: Option<Value>) -> Self {
        let msg = message.into();
        let mut obj = serde_json::Map::new();
        obj.insert("error".into(), Value::String(code.into()));
        obj.insert("message".into(), Value::String(msg.clone()));
        if let Some(Value::Object(map)) = extra {
            for (k, v) in map {
                obj.insert(k, v);
            }
        }
        Self {
            payload: ToolModelPayload::structured(Value::Object(obj), format!("{code}: {msg}")),
            is_error: true,
            effects: Vec::new(),
            media: Vec::new(),
        }
    }

    pub fn with_effects(mut self, effects: Vec<ResourceEffect>) -> Self {
        self.effects = effects;
        self
    }

    pub fn with_media(mut self, media: Vec<MediaAttachment>) -> Self {
        self.media = media;
        self
    }

    pub fn has_changed_version(&self) -> bool {
        !self.is_error && self.effects.iter().any(|e| e.transition.changed())
    }
}

/// Single authoritative capability domain (eliminates v1 4-way policy drift on `pty_exec`/`Skill`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityDomain {
    FilesRead,
    FilesWrite,
    Process,
    Network,
    Browser,
    Memory,
    Mcp,
    InteractiveUi,
    System,
}

/// Exposure tier controlling Turn-0 schema budget (~12 core tools on Turn 0).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolTier {
    /// Eagerly exposed on Turn 0 (~12 high-frequency orthogonal primitives).
    Core,
    /// Discoverable on demand via `tool_search` and appended at tail (preserving KV prefix).
    Deferred,
    /// Exposed ONLY when the active Host declares the required capability (e.g. interactive UI).
    InteractiveOnly,
}

/// Wire JSON schema sent to LLM providers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

/// Single authoritative descriptor for every built-in or MCP tool.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolSpec {
    pub schema: ToolSchema,
    pub tier: ToolTier,
    pub domain: CapabilityDomain,
    pub read_only: bool,
}

/// Typed host runtime capabilities (never hidden inside prompt prose!).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostCapabilities {
    pub interactive_user_input: bool,
    pub network_allowed: bool,
}

impl Default for HostCapabilities {
    fn default() -> Self {
        Self {
            interactive_user_input: true,
            network_allowed: true,
        }
    }
}
