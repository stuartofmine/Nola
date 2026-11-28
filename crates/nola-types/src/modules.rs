// SPDX-License-Identifier: Apache-2.0
//! Domain models for all retained Nola business modules (Skills, Memory, Goals,
//! SOPs, Agent Rooms, Expert Peers, Data Sources, Automations, Messaging,
//! Local Models, Plugins, and MCP).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// MCP server transport kind (exhaustive; always included in connection digest).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpTransport {
    Stdio,
    StreamableHttp,
    ServerSentEvents,
}

/// Configured MCP server entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpServerConfig {
    pub id: String,
    pub name: String,
    pub transport: McpTransport,
    #[serde(default)]
    pub enabled: bool,
    /// Command + args for Stdio transport.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    /// Endpoint URL for StreamableHttp / ServerSentEvents.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
    /// Optional OAuth 2.1 client ID / scopes for remote servers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth_client_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub oauth_scopes: Vec<String>,
}

/// Persisted long-term memory entry indexed by FTS5.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryItem {
    pub id: String,
    pub namespace: String,
    pub title: String,
    pub content: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

/// Discovered Skill definition (`SKILL.md`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillDescriptor {
    pub name: String,
    pub description: String,
    pub path: String,
    pub body: String,
}

/// Autonomous Goal state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalStatus {
    Pending,
    Running,
    Paused,
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GoalRecord {
    pub id: String,
    pub title: String,
    pub objective: String,
    pub workspace_root: String,
    pub status: GoalStatus,
    pub progress_notes: Vec<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

/// Structured Standard Operating Procedure (SOP).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SopRecord {
    pub id: String,
    pub name: String,
    pub description: String,
    pub steps: Vec<String>,
    pub updated_at_ms: i64,
}

/// Expert Peer persona / prompt template.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExpertPeer {
    pub id: String,
    pub name: String,
    pub role: String,
    pub system_prompt: String,
    pub model_override: Option<String>,
    pub allowed_tools: Vec<String>,
    pub updated_at_ms: i64,
}

/// Multi-agent collaboration room.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentRoom {
    pub id: String,
    pub name: String,
    pub workspace_root: String,
    pub participant_expert_ids: Vec<String>,
    pub session_id: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

/// Registered data source (local folder, SQLite/Postgres URI, HTTP feed, etc.).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DataSourceRecord {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub location: String,
    #[serde(default)]
    pub metadata: Value,
    #[serde(default)]
    pub usage_count: u64,
    #[serde(default)]
    pub updated_at_ms: i64,
}

/// Scheduled or event-triggered automation rule.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AutomationRecord {
    pub id: String,
    pub name: String,
    pub schedule: String,
    pub prompt: String,
    pub workspace_root: String,
    pub enabled: bool,
    pub last_run_ms: Option<i64>,
    pub updated_at_ms: i64,
}

/// External messaging channel bridge config (Slack/Telegram/Webhook/etc.).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MessagingChannel {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub endpoint: String,
    pub enabled: bool,
    pub updated_at_ms: i64,
}

/// Local model catalog & acquisition record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LocalModelRecord {
    pub id: String,
    pub name: String,
    pub engine: String,
    pub path: Option<String>,
    pub size_bytes: u64,
    pub quantization: Option<String>,
    pub installed: bool,
    pub updated_at_ms: i64,
}

/// Harness plugin descriptor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PluginRecord {
    pub id: String,
    pub name: String,
    pub version: String,
    pub entry_path: String,
    pub enabled: bool,
    pub capabilities: Vec<String>,
}
