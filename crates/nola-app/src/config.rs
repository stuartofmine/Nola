// SPDX-License-Identifier: Apache-2.0
//! Configuration loader, Vault-backed secret persistence, layered config discovery,
//! schema validation, and route resolver supporting Nola JSON/JSONC configs
//! (including Harbor/Terminal-Bench `model apply-config` payloads).

use std::collections::HashSet;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use nola_store::{Store, Vault};
use nola_types::{FrozenSystemBlock, ModelCapabilities, ModelProtocol, ModelRouteConfig};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const KV_APPLIED_CONFIG: &str = "nola_v2_applied_config";
pub const KV_VAULT_PATH: &str = "nola_vault_path";
pub const MAX_CONFIG_FILE_BYTES: u64 = 1024 * 1024; // 1 MiB

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

/// Alias for callers referencing `LlmConfig`.
pub type LlmConfig = LlmSection;

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

fn resolve_home_dir_from_lookup<F>(lookup: F) -> PathBuf
where
    F: Fn(&str) -> Option<String>,
{
    for key in ["NOLA_HOME", "HOME", "USERPROFILE"] {
        if let Some(val) = lookup(key) {
            let trimmed = val.trim();
            if !trimmed.is_empty() {
                return PathBuf::from(trimmed);
            }
        }
    }
    PathBuf::from(".")
}

pub fn default_home_dir() -> PathBuf {
    resolve_home_dir_from_lookup(|k| std::env::var(k).ok())
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

pub(crate) fn vault_secret_key(conn_id: &str) -> String {
    format!("model_conn_key:{conn_id}")
}

pub(crate) fn resolve_vault_path_for_store(store: &Store) -> PathBuf {
    if let Ok(Some(val)) = store.get_kv(KV_VAULT_PATH) {
        if let Some(p) = val.as_str().map(str::trim).filter(|s| !s.is_empty()) {
            return PathBuf::from(p);
        }
    }
    if let Ok(explicit) = std::env::var("NOLA_VAULT_PATH") {
        let trimmed = explicit.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }
    default_vault_path(&default_db_path())
}

fn open_vault_for_store(store: &Store) -> Result<Vault, String> {
    let vault_path = resolve_vault_path_for_store(store);
    Vault::open(vault_path).map_err(|e| format!("failed to open secret vault: {e}"))
}

/// Bind an explicit Vault file path to `store` (useful for isolated tests or custom DB dirs).
pub fn bind_store_vault_path(store: &Store, vault_path: &Path) -> Result<(), String> {
    store
        .set_kv(KV_VAULT_PATH, &json!(vault_path.display().to_string()))
        .map_err(|e| e.to_string())
}

/// Strip JSONC `//` line comments, `/* ... */` block comments, and trailing commas
/// outside of string literals so standard `serde_json` parses both JSON and JSONC cleanly.
pub fn strip_jsonc(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut no_comments = String::with_capacity(input.len());
    let mut i = 0;
    let mut in_str = false;
    let mut escaped = false;
    while i < bytes.len() {
        let ch = bytes[i];
        if in_str {
            if ch < 0x80 {
                no_comments.push(ch as char);
                if escaped {
                    escaped = false;
                } else if ch == b'\\' {
                    escaped = true;
                } else if ch == b'"' {
                    in_str = false;
                }
                i += 1;
            } else if let Some(c) = input[i..].chars().next() {
                no_comments.push(c);
                escaped = false;
                i += c.len_utf8();
            } else {
                break;
            }
            continue;
        }
        if ch == b'"' {
            in_str = true;
            no_comments.push('"');
            i += 1;
            continue;
        }
        if ch == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            i += 2;
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if ch == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                if bytes[i] == b'\n' {
                    no_comments.push('\n');
                }
                i += 1;
            }
            i = (i + 2).min(bytes.len());
            continue;
        }
        if ch < 0x80 {
            no_comments.push(ch as char);
            i += 1;
        } else {
            let rest = &input[i..];
            if let Some(c) = rest.chars().next() {
                no_comments.push(c);
                i += c.len_utf8();
            } else {
                break;
            }
        }
    }

    // Remove trailing commas before `}` or `]` outside strings
    let nb = no_comments.as_bytes();
    let mut out = String::with_capacity(no_comments.len());
    let mut j = 0;
    in_str = false;
    escaped = false;
    while j < nb.len() {
        let ch = nb[j];
        if in_str {
            if ch < 0x80 {
                out.push(ch as char);
                if escaped {
                    escaped = false;
                } else if ch == b'\\' {
                    escaped = true;
                } else if ch == b'"' {
                    in_str = false;
                }
                j += 1;
            } else if let Some(c) = no_comments[j..].chars().next() {
                out.push(c);
                escaped = false;
                j += c.len_utf8();
            } else {
                break;
            }
            continue;
        }
        if ch == b'"' {
            in_str = true;
            out.push('"');
            j += 1;
            continue;
        }
        if ch == b',' {
            let mut k = j + 1;
            while k < nb.len() && matches!(nb[k], b' ' | b'\t' | b'\r' | b'\n') {
                k += 1;
            }
            if k < nb.len() && matches!(nb[k], b'}' | b']') {
                j += 1;
                continue;
            }
        }
        if ch < 0x80 {
            out.push(ch as char);
            j += 1;
        } else if let Some(c) = no_comments[j..].chars().next() {
            out.push(c);
            j += c.len_utf8();
        } else {
            break;
        }
    }
    out
}

/// Discover the highest-priority config file path using layered resolution:
/// 1. Walk upward from `cwd` to the git root (inclusive), checking `.nola/config.jsonc` then `.nola/config.json`.
/// 2. Fall back to `$HOME/.nola/config.jsonc` then `$HOME/.nola/config.json`.
pub fn discover_config_path(cwd: &Path) -> Option<PathBuf> {
    let mut dir = Some(cwd);
    while let Some(cur) = dir {
        for rel in [".nola/config.jsonc", ".nola/config.json"] {
            let cand = cur.join(rel);
            if fs::symlink_metadata(&cand).is_ok() {
                return Some(cand);
            }
        }
        if cur.join(".git").exists() {
            break;
        }
        dir = cur.parent();
    }

    let home = default_home_dir();
    for rel in [".nola/config.jsonc", ".nola/config.json"] {
        let cand = home.join(rel);
        if fs::symlink_metadata(&cand).is_ok() {
            return Some(cand);
        }
    }
    None
}

/// Read a config file safely: rejects symlinks and enforces a strict 1 MiB file size cap.
pub fn read_config_file_safe(path: &Path) -> Result<String, String> {
    let meta = fs::symlink_metadata(path)
        .map_err(|e| format!("failed to inspect config {}: {e}", path.display()))?;
    if meta.file_type().is_symlink() {
        return Err(format!(
            "refusing to load symlinked config file: {}",
            path.display()
        ));
    }
    if !meta.file_type().is_file() {
        return Err(format!(
            "config path is not a regular file: {}",
            path.display()
        ));
    }
    if meta.len() > MAX_CONFIG_FILE_BYTES {
        return Err(format!(
            "config file {} exceeds {} byte size limit",
            path.display(),
            MAX_CONFIG_FILE_BYTES
        ));
    }
    let f = fs::File::open(path)
        .map_err(|e| format!("failed to open config {}: {e}", path.display()))?;
    let mut buf = Vec::new();
    f.take(MAX_CONFIG_FILE_BYTES + 1)
        .read_to_end(&mut buf)
        .map_err(|e| format!("failed to read config {}: {e}", path.display()))?;
    if buf.len() as u64 > MAX_CONFIG_FILE_BYTES {
        return Err(format!(
            "config file {} exceeds {} byte size limit",
            path.display(),
            MAX_CONFIG_FILE_BYTES
        ));
    }
    String::from_utf8(buf)
        .map_err(|e| format!("config file {} is not valid UTF-8: {e}", path.display()))
}

/// Validate `AppliedConfig` invariants:
/// - No empty or duplicate connection IDs
/// - No empty or duplicate model IDs within a connection
/// - `default_model`, if set, must reference an existing connection (and an existing model when models are declared).
pub fn validate_config(cfg: &AppliedConfig) -> Result<(), String> {
    let mut seen_conns: HashSet<&str> = HashSet::new();
    for conn in &cfg.llm.connections {
        let cid = conn.id.trim();
        if cid.is_empty() {
            return Err("connection id must not be empty".into());
        }
        if !seen_conns.insert(cid) {
            return Err(format!("duplicate connection id '{cid}'"));
        }
        let mut seen_models: HashSet<&str> = HashSet::new();
        for model in &conn.models {
            let mid = model.id.trim();
            if mid.is_empty() {
                return Err(format!("connection '{cid}' contains an empty model id"));
            }
            if !seen_models.insert(mid) {
                return Err(format!(
                    "duplicate model id '{mid}' in connection '{cid}'"
                ));
            }
        }
    }

    if let Some(sel) = &cfg.llm.default_model {
        let sel_conn = sel.connection.trim();
        let sel_model = sel.model.trim();
        if sel_conn.is_empty() || sel_model.is_empty() {
            return Err("default_model requires non-empty 'connection' and 'model'".into());
        }
        let Some(conn) = cfg
            .llm
            .connections
            .iter()
            .find(|c| c.id.trim() == sel_conn)
        else {
            return Err(format!(
                "default_model references unknown connection '{sel_conn}'"
            ));
        };
        if !conn.models.is_empty() && !conn.models.iter().any(|m| m.id.trim() == sel_model) {
            return Err(format!(
                "default_model references unknown model '{sel_model}' in connection '{sel_conn}'"
            ));
        }
    }
    Ok(())
}

/// Save `AppliedConfig` into `store`, extracting every non-empty literal `api_key` into the
/// AES-256-GCM `Vault` under `model_conn_key:<conn.id>` and NEVER writing plaintext keys to SQLite `kv`.
pub fn save_applied_config(store: &Store, cfg: &AppliedConfig) -> Result<(), String> {
    validate_config(cfg)?;
    let vault = open_vault_for_store(store)?;
    let mut persisted = cfg.clone();

    for conn in &mut persisted.llm.connections {
        let key_name = vault_secret_key(&conn.id);
        match conn.api_key.as_deref().map(str::trim) {
            Some(raw) if !raw.is_empty() && !raw.starts_with("env:") => {
                vault
                    .set_secret(&key_name, raw)
                    .map_err(|e| format!("failed to store API key for '{}' in vault: {e}", conn.id))?;
                // Strip plaintext secret before writing to SQLite kv!
                conn.api_key = None;
            }
            Some(raw) if raw.starts_with("env:") => {
                // Keep `env:VAR_NAME` reference intact (not a secret).
            }
            _ => {
                let _ = vault.delete_secret(&key_name);
                conn.api_key = None;
            }
        }
    }

    let val = serde_json::to_value(&persisted).map_err(|e| e.to_string())?;
    store
        .set_kv(KV_APPLIED_CONFIG, &val)
        .map_err(|e| e.to_string())
}

/// Apply a config file into `store` (supports JSON and JSONC, rejects symlinks & >1 MiB files,
/// validates config, and stores API keys strictly in `Vault`).
pub fn apply_config_file(store: &Store, path: &Path) -> Result<AppliedConfig, String> {
    let raw = read_config_file_safe(path)?;
    let clean = strip_jsonc(&raw);
    let cfg: AppliedConfig = serde_json::from_str(&clean)
        .map_err(|e| format!("failed to parse config JSON {}: {e}", path.display()))?;
    save_applied_config(store, &cfg)?;
    Ok(cfg)
}

/// Hydrate connection API keys from the AES-256-GCM `Vault` (preserving any legacy plaintext key
/// still in `kv` for graceful backward-compatible fallback until the next save migrates it).
fn hydrate_secrets_from_vault(store: &Store, cfg: &mut AppliedConfig) {
    let Ok(vault) = open_vault_for_store(store) else {
        return;
    };
    for conn in &mut cfg.llm.connections {
        let needs_hydration = conn
            .api_key
            .as_deref()
            .map(|s| s.trim().is_empty())
            .unwrap_or(true);
        if needs_hydration {
            if let Ok(Some(secret)) = vault.get_secret(&vault_secret_key(&conn.id)) {
                if !secret.trim().is_empty() {
                    conn.api_key = Some(secret);
                }
            }
        }
    }
}

/// Load applied config from `store` (hydrating API keys from `Vault`), falling back to
/// layered config file discovery (`.nola/config.jsonc` / `.nola/config.json` up to git root,
/// then `$HOME/.nola/config.jsonc` / `$HOME/.nola/config.json`).
pub fn load_applied_config(store: &Store) -> AppliedConfig {
    if let Ok(Some(val)) = store.get_kv(KV_APPLIED_CONFIG) {
        if let Ok(mut cfg) = serde_json::from_value::<AppliedConfig>(val) {
            hydrate_secrets_from_vault(store, &mut cfg);
            return cfg;
        }
    }
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    if let Some(discovered) = discover_config_path(&cwd) {
        if let Ok(raw) = read_config_file_safe(&discovered) {
            let clean = strip_jsonc(&raw);
            if let Ok(mut cfg) = serde_json::from_str::<AppliedConfig>(&clean) {
                hydrate_secrets_from_vault(store, &mut cfg);
                return cfg;
            }
        }
    }
    AppliedConfig::default()
}

/// Backward-compatible alias for `load_applied_config`.
pub fn load_active_config(store: &Store) -> AppliedConfig {
    load_applied_config(store)
}

/// Returns true if a connection currently has a configured credential (Vault/literal key or non-empty env var).
pub fn connection_has_api_key(conn: &ConnectionConfig) -> bool {
    if let Some(env_name) = &conn.api_key_env {
        if std::env::var(env_name)
            .ok()
            .is_some_and(|v| !v.trim().is_empty())
        {
            return true;
        }
    }
    if let Some(raw) = &conn.api_key {
        let trimmed = raw.trim();
        if let Some(env_name) = trimmed.strip_prefix("env:") {
            return std::env::var(env_name.trim())
                .ok()
                .is_some_and(|v| !v.trim().is_empty());
        }
        return !trimmed.is_empty();
    }
    false
}

/// Serialize `LlmSection` for public HTTP APIs with all plaintext `api_key` values stripped to `""`
/// and explicit `hasApiKey` / `apiKeyConfigured` booleans exposed instead.
pub fn redacted_llm_config(llm: &LlmSection) -> Value {
    let connections: Vec<Value> = llm
        .connections
        .iter()
        .map(|conn| {
            let configured = connection_has_api_key(conn);
            json!({
                "id": conn.id,
                "endpoint": conn.endpoint,
                "protocol": conn.protocol,
                "provider": conn.provider,
                "api_key": "",
                "api_key_env": conn.api_key_env,
                "hasApiKey": configured,
                "apiKeyConfigured": configured,
                "models": conn.models
            })
        })
        .collect();
    json!({
        "default_model": llm.default_model,
        "connections": connections
    })
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
            temperature: None,
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
            // NEVER return literal `"env:VAR_NAME"` as a bearer token when unset!
            return std::env::var(env_name.trim()).unwrap_or_default();
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

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_config_with_key(api_key: Option<&str>) -> AppliedConfig {
        AppliedConfig {
            llm: LlmSection {
                default_model: Some(DefaultModelSelection {
                    connection: "openai".into(),
                    model: "gpt-4o".into(),
                }),
                connections: vec![ConnectionConfig {
                    id: "openai".into(),
                    endpoint: "https://api.openai.com/v1".into(),
                    protocol: Some("chat_completions".into()),
                    provider: None,
                    api_key: api_key.map(ToString::to_string),
                    api_key_env: None,
                    models: vec![ModelEntry {
                        id: "gpt-4o".into(),
                        context_window: Some(128_000),
                        max_output_tokens: None,
                        temperature: None,
                        top_p: None,
                        reasoning_effort: None,
                        capabilities: None,
                    }],
                }],
            },
            ..Default::default()
        }
    }

    #[test]
    fn vault_stores_api_key_and_kv_never_contains_plaintext() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory().unwrap();
        bind_store_vault_path(&store, &dir.path().join("vault.json")).unwrap();

        let cfg = sample_config_with_key(Some("sk-super-secret-xyz"));
        save_applied_config(&store, &cfg).unwrap();

        // Raw SQLite kv MUST NOT contain the plaintext API key!
        let raw_kv = store.get_kv(KV_APPLIED_CONFIG).unwrap().unwrap();
        let raw_str = serde_json::to_string(&raw_kv).unwrap();
        assert!(!raw_str.contains("sk-super-secret-xyz"));

        // Hydrated load restores the API key from Vault
        let loaded = load_applied_config(&store);
        assert_eq!(
            loaded.llm.connections[0].api_key.as_deref(),
            Some("sk-super-secret-xyz")
        );

        // Redacted projection strips plaintext key and reports configured booleans
        let redacted = redacted_llm_config(&loaded.llm);
        let redacted_str = serde_json::to_string(&redacted).unwrap();
        assert!(!redacted_str.contains("sk-super-secret-xyz"));
        assert_eq!(redacted["connections"][0]["api_key"], "");
        assert_eq!(redacted["connections"][0]["hasApiKey"], true);
        assert_eq!(redacted["connections"][0]["apiKeyConfigured"], true);

        // Clearing api_key deletes from Vault
        let cleared = sample_config_with_key(None);
        save_applied_config(&store, &cleared).unwrap();
        let reloaded = load_applied_config(&store);
        assert!(reloaded.llm.connections[0].api_key.is_none());
        let redacted2 = redacted_llm_config(&reloaded.llm);
        assert_eq!(redacted2["connections"][0]["hasApiKey"], false);
    }

    #[test]
    fn validate_config_rejects_duplicates_and_invalid_default_model() {
        let mut dup_conn = sample_config_with_key(None);
        dup_conn.llm.connections.push(dup_conn.llm.connections[0].clone());
        assert!(validate_config(&dup_conn).is_err());

        let mut dup_model = sample_config_with_key(None);
        let cloned_model = dup_model.llm.connections[0].models[0].clone();
        dup_model.llm.connections[0].models.push(cloned_model);
        assert!(validate_config(&dup_model).is_err());

        let mut bad_default = sample_config_with_key(None);
        bad_default.llm.default_model = Some(DefaultModelSelection {
            connection: "openai".into(),
            model: "nonexistent".into(),
        });
        assert!(validate_config(&bad_default).is_err());
    }

    #[test]
    fn discover_config_path_walks_up_to_git_root_and_prefers_jsonc() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        let sub = repo.join("a/b/c");
        fs::create_dir_all(repo.join(".git")).unwrap();
        fs::create_dir_all(repo.join(".nola")).unwrap();
        fs::create_dir_all(&sub).unwrap();
        fs::write(repo.join(".nola/config.json"), "{}").unwrap();
        assert_eq!(discover_config_path(&sub), Some(repo.join(".nola/config.json")));

        fs::write(repo.join(".nola/config.jsonc"), "{}").unwrap();
        assert_eq!(
            discover_config_path(&sub),
            Some(repo.join(".nola/config.jsonc"))
        );
    }

    #[cfg(unix)]
    #[test]
    fn read_config_file_safe_rejects_symlinks_and_oversized_files() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("real.json");
        fs::write(&target, "{}").unwrap();
        let link = dir.path().join("link.json");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(read_config_file_safe(&link).unwrap_err().contains("symlink"));

        let huge = dir.path().join("huge.json");
        fs::write(&huge, vec![b' '; (MAX_CONFIG_FILE_BYTES as usize) + 16]).unwrap();
        assert!(read_config_file_safe(&huge).unwrap_err().contains("limit"));
    }

    #[test]
    fn resolve_home_dir_prefers_nola_home_then_home_then_userprofile() {
        assert_eq!(
            resolve_home_dir_from_lookup(|k| match k {
                "NOLA_HOME" => Some("/custom/nola".into()),
                "HOME" => Some("/home/alice".into()),
                "USERPROFILE" => Some(r"C:\Users\Alice".into()),
                _ => None,
            }),
            PathBuf::from("/custom/nola")
        );
        assert_eq!(
            resolve_home_dir_from_lookup(|k| match k {
                "NOLA_HOME" => Some("   ".into()),
                "HOME" => Some("/home/alice".into()),
                "USERPROFILE" => Some(r"C:\Users\Alice".into()),
                _ => None,
            }),
            PathBuf::from("/home/alice")
        );
        assert_eq!(
            resolve_home_dir_from_lookup(|k| match k {
                "USERPROFILE" => Some(r"C:\Users\Alice".into()),
                _ => None,
            }),
            PathBuf::from(r"C:\Users\Alice")
        );
        assert_eq!(resolve_home_dir_from_lookup(|_| None), PathBuf::from("."));
    }
}
