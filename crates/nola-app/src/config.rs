// SPDX-License-Identifier: Apache-2.0
//! Configuration loader & route resolver supporting Nola JSON/JSONC configs
//! (including Harbor/Terminal-Bench `model apply-config` payloads).

use std::fs;
use std::path::{Path, PathBuf};

use nola_store::Store;
use nola_types::{FrozenSystemBlock, ModelCapabilities, ModelProtocol, ModelRouteConfig};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const KV_APPLIED_CONFIG: &str = "nola_v2_applied_config";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AppliedConfig {
    #[serde(default)]
    pub llm: LlmSection,
    #[serde(default)]
    pub network_policy: Option<String>,
    #[serde(default)]
    pub max_iterations: Option<usize>,
    #[serde(default)]
    pub instructions: Vec<String>,
    #[serde(default)]
    pub model_capabilities: Option<Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LlmSection {
    #[serde(default)]
    pub default_model: Option<DefaultModelSelection>,
    #[serde(default)]
    pub connections: Vec<ConnectionConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DefaultModelSelection {
    pub connection: String,
    pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionConfig {
    pub id: String,
    pub endpoint: String,
    #[serde(default)]
    pub protocol: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub api_key_env: Option<String>,
    #[serde(default)]
    pub models: Vec<ModelEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelEntry {
    pub id: String,
    #[serde(default)]
    pub context_window: Option<usize>,
    #[serde(default)]
    pub max_output_tokens: Option<usize>,
    #[serde(default)]
    pub temperature: Option<f64>,
    #[serde(default)]
    pub top_p: Option<f64>,
    #[serde(default)]
    pub reasoning_effort: Option<String>,
    #[serde(default)]
    pub capabilities: Option<ModelCapabilities>,
}

pub fn default_home_dir() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}

pub fn default_db_path() -> PathBuf {
    if let Ok(explicit) = std::env::var("NOLA_DB_PATH") {
        if !explicit.trim().is_empty() {
            return PathBuf::from(explicit);
        }
    }
    default_home_dir().join(".nola/nola-v2.db")
}

pub fn default_vault_path(db_path: &Path) -> PathBuf {
    db_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("vault.json")
}

/// Apply a config file into `store` (and persist a copy in `$HOME/.nola/config.json`).
pub fn apply_config_file(store: &Store, path: &Path) -> Result<AppliedConfig, String> {
    let raw = fs::read_to_string(path)
        .map_err(|e| format!("failed to read config {}: {e}", path.display()))?;
    let cfg: AppliedConfig = serde_json::from_str(&raw)
        .map_err(|e| format!("failed to parse config JSON {}: {e}", path.display()))?;
    let val = serde_json::to_value(&cfg).map_err(|e| e.to_string())?;
    store
        .set_kv(KV_APPLIED_CONFIG, &val)
        .map_err(|e| e.to_string())?;
    Ok(cfg)
}

/// Load applied config from `store`, falling back to `$HOME/.nola/config.json`.
pub fn load_active_config(store: &Store) -> AppliedConfig {
    if let Ok(Some(val)) = store.get_kv(KV_APPLIED_CONFIG) {
        if let Ok(cfg) = serde_json::from_value::<AppliedConfig>(val) {
            return cfg;
        }
    }
    let home_cfg = default_home_dir().join(".nola/config.json");
    if let Ok(raw) = fs::read_to_string(&home_cfg) {
        if let Ok(cfg) = serde_json::from_str::<AppliedConfig>(&raw) {
            return cfg;
        }
    }
    AppliedConfig::default()
}

/// Resolve the active `ModelRouteConfig` from `AppliedConfig` + environment fallbacks.
pub fn resolve_model_route(cfg: &AppliedConfig) -> Result<ModelRouteConfig, String> {
    if let Some(sel) = &cfg.llm.default_model {
        if let Some(conn) = cfg.llm.connections.iter().find(|c| c.id == sel.connection) {
            let entry = conn.models.iter().find(|m| m.id == sel.model);
            let proto_raw = conn
                .protocol
                .as_deref()
                .or(conn.provider.as_deref())
                .unwrap_or("chat_completions");
            let protocol = ModelProtocol::parse(proto_raw).unwrap_or(ModelProtocol::OpenAiChat);
            let api_key = resolve_api_key(conn);
            let caps = entry
                .and_then(|e| e.capabilities.clone())
                .unwrap_or_default();
            return Ok(ModelRouteConfig {
                connection_id: conn.id.clone(),
                endpoint: conn.endpoint.clone(),
                protocol,
                model: sel.model.clone(),
                api_key,
                context_window: entry.and_then(|e| e.context_window).unwrap_or(128_000),
                max_output_tokens: entry.and_then(|e| e.max_output_tokens),
                temperature: entry.and_then(|e| e.temperature),
                top_p: entry.and_then(|e| e.top_p),
                capabilities: caps,
                reasoning_effort: entry.and_then(|e| e.reasoning_effort.clone()),
                prompt_cache_key: Some(format!("nola-v2-{}", conn.id)),
            });
        }
    }

    // Fallback to environment variables for quick standalone CLI usage
    if let (Ok(endpoint), Ok(model)) = (
        std::env::var("NOLA_LLM_ENDPOINT"),
        std::env::var("NOLA_LLM_MODEL"),
    ) {
        let protocol = std::env::var("NOLA_LLM_PROTOCOL")
            .ok()
            .and_then(|s| ModelProtocol::parse(&s))
            .unwrap_or(ModelProtocol::OpenAiChat);
        return Ok(ModelRouteConfig {
            connection_id: "env".into(),
            endpoint,
            protocol,
            model,
            api_key: std::env::var("NOLA_API_KEY")
                .or_else(|_| std::env::var("OPENAI_API_KEY"))
                .unwrap_or_default(),
            context_window: 128_000,
            max_output_tokens: None,
            temperature: Some(0.2),
            top_p: None,
            capabilities: ModelCapabilities::default(),
            reasoning_effort: None,
            prompt_cache_key: Some("nola-v2-env".into()),
        });
    }

    Err("no model configured; run `nola model apply-config <config.json>` or set NOLA_LLM_ENDPOINT & NOLA_LLM_MODEL".into())
}

fn resolve_api_key(conn: &ConnectionConfig) -> String {
    if let Some(env_name) = &conn.api_key_env {
        if let Ok(val) = std::env::var(env_name) {
            if !val.trim().is_empty() {
                return val;
            }
        }
    }
    if let Some(raw) = &conn.api_key {
        if let Some(env_name) = raw.strip_prefix("env:") {
            if let Ok(val) = std::env::var(env_name.trim()) {
                return val;
            }
        }
        if !raw.trim().is_empty() {
            return raw.clone();
        }
    }
    std::env::var("NOLA_BENCHMARK_TOKEN")
        .or_else(|_| std::env::var("OPENAI_API_KEY"))
        .unwrap_or_default()
}

/// Build the immutable Turn-0 frozen system prompt prefix (NEVER mutated mid-session).
pub fn build_frozen_system_blocks(workspace_root: &Path, cfg: &AppliedConfig) -> Vec<FrozenSystemBlock> {
    let mut blocks = vec![FrozenSystemBlock {
        id: "nola_core_v2".into(),
        text: format!(
            "You are Nola v2, a precise software engineering and systems agent.\n\
             Workspace root: {}\n\
             Principles:\n\
             - Inspect files before editing; make surgical, verified changes.\n\
             - Use tool_search if you need specialized deferred tools (web_fetch, code_exec, lsp_diagnostics, skills, memory, goals, rooms, MCP).\n\
             - Verify your work with real tests/commands before concluding.",
            workspace_root.display()
        ),
    }];
    if !cfg.instructions.is_empty() {
        blocks.push(FrozenSystemBlock {
            id: "host_instructions".into(),
            text: cfg.instructions.join("\n\n"),
        });
    }
    blocks
}
