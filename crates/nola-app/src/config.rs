// SPDX-License-Identifier: Apache-2.0
//! Configuration loader, Vault-backed secret persistence, layered config discovery,
//! schema validation, and route resolver supporting Nola JSON/JSONC configs
//! (including Harbor/Terminal-Bench `model apply-config` payloads).

use std::collections::{BTreeMap, HashSet};
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

/// Optional fallback route declaration accepted on `llm.fallbacks`, top-level `fallbacks`,
/// or per-connection `fallbacks`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FallbackRouteConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, alias = "base_url", alias = "baseURL", skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol: Option<String>,
    #[serde(default, alias = "apiKey", skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(default, alias = "apiKeyEnv", skip_serializing_if = "Option::is_none")]
    pub api_key_env: Option<String>,
    #[serde(default, alias = "credentialRef", skip_serializing_if = "Option::is_none")]
    pub credential_ref: Option<String>,
}

/// Preserved extended metadata for a connection (`name`, `credential_ref`, `fallbacks`,
/// and per-model display `name`s) stored alongside `LlmSection` without changing
/// external struct literal layouts for `ConnectionConfig` / `ModelEntry`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionExtras {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, alias = "credentialRef", skip_serializing_if = "Option::is_none")]
    pub credential_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fallbacks: Vec<FallbackRouteConfig>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub model_names: BTreeMap<String, String>,
}

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
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub connection_extras: BTreeMap<String, ConnectionExtras>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fallbacks: Vec<FallbackRouteConfig>,
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

fn parse_fallback_entry(val: &Value) -> Option<FallbackRouteConfig> {
    if let Some(s) = val.as_str() {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return None;
        }
        if let Some((conn, model)) = trimmed.split_once('/') {
            return Some(FallbackRouteConfig {
                connection: Some(conn.trim().to_string()),
                model: Some(model.trim().to_string()),
                ..Default::default()
            });
        }
        return Some(FallbackRouteConfig {
            model: Some(trimmed.to_string()),
            ..Default::default()
        });
    }
    serde_json::from_value::<FallbackRouteConfig>(val.clone()).ok()
}

fn parse_fallback_array(val: Option<&Value>) -> Vec<FallbackRouteConfig> {
    val.and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(parse_fallback_entry).collect())
        .unwrap_or_default()
}

fn harvest_config_extras(raw: &Value, cfg: &mut AppliedConfig) {
    if let Some(conns) = raw.pointer("/llm/connections").and_then(|v| v.as_array()) {
        for c in conns {
            let Some(cid) = c
                .get("id")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
            else {
                continue;
            };
            let entry = cfg.connection_extras.entry(cid.to_string()).or_default();
            if let Some(name) = c
                .get("name")
                .or_else(|| c.get("display_name"))
                .or_else(|| c.get("displayName"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                entry.name = Some(name.to_string());
            }
            if let Some(cred_ref) = c
                .get("credential_ref")
                .or_else(|| c.get("credentialRef"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                entry.credential_ref = Some(cred_ref.to_string());
            }
            let conn_fallbacks = parse_fallback_array(c.get("fallbacks"));
            if !conn_fallbacks.is_empty() {
                entry.fallbacks = conn_fallbacks;
            }
            if let Some(models) = c.get("models").and_then(|v| v.as_array()) {
                for m in models {
                    let Some(mid) = m
                        .get("id")
                        .and_then(|v| v.as_str())
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                    else {
                        continue;
                    };
                    if let Some(mname) = m
                        .get("name")
                        .or_else(|| m.get("display_name"))
                        .or_else(|| m.get("displayName"))
                        .and_then(|v| v.as_str())
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                    {
                        entry.model_names.insert(mid.to_string(), mname.to_string());
                    }
                }
            }
        }
    }

    let llm_fallbacks = parse_fallback_array(raw.pointer("/llm/fallbacks"));
    if !llm_fallbacks.is_empty() {
        cfg.fallbacks = llm_fallbacks;
    } else if cfg.fallbacks.is_empty() {
        let root_fallbacks = parse_fallback_array(raw.get("fallbacks"));
        if !root_fallbacks.is_empty() {
            cfg.fallbacks = root_fallbacks;
        }
    }

    // Prune completely empty ConnectionExtras entries so serialized kv stays lean.
    cfg.connection_extras.retain(|_, ex| {
        ex.name.is_some()
            || ex.credential_ref.is_some()
            || !ex.fallbacks.is_empty()
            || !ex.model_names.is_empty()
    });
}

/// Parse cleaned JSON (after `strip_jsonc`) into `AppliedConfig`, preserving connection/model
/// `name` fields, `credential_ref` / `credentialRef`, and optional `fallbacks` arrays.
pub fn parse_applied_config_json(clean_json: &str) -> Result<AppliedConfig, String> {
    let raw: Value = serde_json::from_str(clean_json)
        .map_err(|e| format!("failed to parse config JSON: {e}"))?;
    let mut cfg: AppliedConfig = serde_json::from_value(raw.clone())
        .map_err(|e| format!("failed to decode config schema: {e}"))?;
    harvest_config_extras(&raw, &mut cfg);
    Ok(cfg)
}

/// Save `AppliedConfig` into `store`, extracting every non-empty literal `api_key` into the
/// AES-256-GCM `Vault` under `model_conn_key:<conn.id>` and NEVER writing plaintext keys to SQLite `kv`.
pub fn save_applied_config(store: &Store, cfg: &AppliedConfig) -> Result<(), String> {
    validate_config(cfg)?;
    let vault = open_vault_for_store(store)?;
    let mut persisted = cfg.clone();

    for conn in &mut persisted.llm.connections {
        let key_name = vault_secret_key(&conn.id);
        let has_cred_ref = persisted
            .connection_extras
            .get(&conn.id)
            .and_then(|e| e.credential_ref.as_deref())
            .is_some_and(|s| !s.trim().is_empty());
        match conn.api_key.as_deref().map(str::trim) {
            Some(raw) if !raw.is_empty() && !raw.starts_with("env:") => {
                // Only write to `model_conn_key:<id>` when not backed purely by an external credential_ref
                // (or update vault key if inline key was explicitly provided).
                if !has_cred_ref {
                    vault.set_secret(&key_name, raw).map_err(|e| {
                        format!("failed to store API key for '{}' in vault: {e}", conn.id)
                    })?;
                }
                // Strip plaintext secret before writing to SQLite kv!
                conn.api_key = None;
            }
            Some(raw) if raw.starts_with("env:") => {
                // Keep `env:VAR_NAME` reference intact (not a secret).
            }
            _ => {
                if !has_cred_ref {
                    let _ = vault.delete_secret(&key_name);
                }
                conn.api_key = None;
            }
        }
    }

    // Also strip any literal plaintext api_key inside fallback entries before writing to kv.
    for fb in &mut persisted.fallbacks {
        if let Some(raw) = fb.api_key.as_deref().map(str::trim) {
            if !raw.is_empty() && !raw.starts_with("env:") {
                let fb_key = format!(
                    "fallback_key:{}:{}",
                    fb.connection.as_deref().unwrap_or("default"),
                    fb.model.as_deref().unwrap_or("default")
                );
                let _ = vault.set_secret(&fb_key, raw);
                fb.credential_ref = Some(fb_key);
                fb.api_key = None;
            }
        }
    }
    for (cid, ex) in &mut persisted.connection_extras {
        for (idx, fb) in ex.fallbacks.iter_mut().enumerate() {
            if let Some(raw) = fb.api_key.as_deref().map(str::trim) {
                if !raw.is_empty() && !raw.starts_with("env:") {
                    let fb_key = format!("fallback_key:{cid}:{idx}");
                    let _ = vault.set_secret(&fb_key, raw);
                    fb.credential_ref = Some(fb_key);
                    fb.api_key = None;
                }
            }
        }
    }

    let val = serde_json::to_value(&persisted).map_err(|e| e.to_string())?;
    store
        .set_kv(KV_APPLIED_CONFIG, &val)
        .map_err(|e| e.to_string())
}

/// Apply a config file into `store` (supports JSON and JSONC, rejects symlinks & >1 MiB files,
/// validates config, preserves connection/model names, `credential_ref`, and `fallbacks`,
/// and stores API keys strictly in `Vault`).
pub fn apply_config_file(store: &Store, path: &Path) -> Result<AppliedConfig, String> {
    let raw = read_config_file_safe(path)?;
    let clean = strip_jsonc(&raw);
    let mut cfg = parse_applied_config_json(&clean)
        .map_err(|e| format!("failed to parse config JSON {}: {e}", path.display()))?;
    save_applied_config(store, &cfg)?;
    hydrate_secrets_from_vault(store, &mut cfg);
    Ok(cfg)
}

fn lookup_vault_credential_ref(vault: &Vault, cred_ref: &str) -> Option<String> {
    let trimmed = cred_ref.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.starts_with("env:") {
        return Some(trimmed.to_string());
    }
    if let Ok(Some(secret)) = vault.get_secret(trimmed) {
        if !secret.trim().is_empty() {
            return Some(secret);
        }
    }
    if let Ok(Some(secret)) = vault.get_secret(&format!("credential:{trimmed}")) {
        if !secret.trim().is_empty() {
            return Some(secret);
        }
    }
    None
}

/// Hydrate connection API keys from the AES-256-GCM `Vault` (honoring `credential_ref` /
/// `credentialRef` first, then `model_conn_key:<id>`, and preserving any legacy plaintext key
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
            if let Some(cred_ref) = cfg
                .connection_extras
                .get(&conn.id)
                .and_then(|e| e.credential_ref.as_deref())
            {
                if let Some(secret) = lookup_vault_credential_ref(&vault, cred_ref) {
                    conn.api_key = Some(secret);
                    continue;
                }
            }
            if let Ok(Some(secret)) = vault.get_secret(&vault_secret_key(&conn.id)) {
                if !secret.trim().is_empty() {
                    conn.api_key = Some(secret);
                }
            }
        }
    }
    for fb in &mut cfg.fallbacks {
        if fb.api_key.as_deref().map(|s| s.trim().is_empty()).unwrap_or(true) {
            if let Some(cred_ref) = fb.credential_ref.as_deref() {
                if let Some(secret) = lookup_vault_credential_ref(&vault, cred_ref) {
                    fb.api_key = Some(secret);
                }
            }
        }
    }
    for ex in cfg.connection_extras.values_mut() {
        for fb in &mut ex.fallbacks {
            if fb.api_key.as_deref().map(|s| s.trim().is_empty()).unwrap_or(true) {
                if let Some(cred_ref) = fb.credential_ref.as_deref() {
                    if let Some(secret) = lookup_vault_credential_ref(&vault, cred_ref) {
                        fb.api_key = Some(secret);
                    }
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
        if let Ok(mut cfg) = serde_json::from_value::<AppliedConfig>(val.clone()) {
            harvest_config_extras(&val, &mut cfg);
            hydrate_secrets_from_vault(store, &mut cfg);
            return cfg;
        }
    }
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    if let Some(discovered) = discover_config_path(&cwd) {
        if let Ok(raw) = read_config_file_safe(&discovered) {
            let clean = strip_jsonc(&raw);
            if let Ok(mut cfg) = parse_applied_config_json(&clean) {
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

/// Serialize `AppliedConfig`'s LLM section with all plaintext `api_key` values stripped,
/// preserving connection `name`, `credential_ref`, per-model `name`, and `fallbacks`.
pub fn redacted_applied_llm_config(cfg: &AppliedConfig) -> Value {
    let connections: Vec<Value> = cfg
        .llm
        .connections
        .iter()
        .map(|conn| {
            let configured = connection_has_api_key(conn);
            let extras = cfg.connection_extras.get(&conn.id);
            let models: Vec<Value> = conn
                .models
                .iter()
                .map(|m| {
                    let mut mv = serde_json::to_value(m).unwrap_or_else(|_| json!({"id": m.id}));
                    if let Some(mname) = extras.and_then(|e| e.model_names.get(&m.id)) {
                        if let Some(obj) = mv.as_object_mut() {
                            obj.insert("name".into(), json!(mname));
                        }
                    }
                    mv
                })
                .collect();
            let mut obj = json!({
                "id": conn.id,
                "endpoint": conn.endpoint,
                "protocol": conn.protocol,
                "provider": conn.provider,
                "api_key": "",
                "api_key_env": conn.api_key_env,
                "hasApiKey": configured,
                "apiKeyConfigured": configured,
                "has_api_key": configured,
                "models": models
            });
            if let Some(ex) = extras {
                if let Some(map) = obj.as_object_mut() {
                    if let Some(ref name) = ex.name {
                        map.insert("name".into(), json!(name));
                    }
                    if let Some(ref cred_ref) = ex.credential_ref {
                        map.insert("credential_ref".into(), json!(cred_ref));
                    }
                    if !ex.fallbacks.is_empty() {
                        let fbs: Vec<Value> = ex
                            .fallbacks
                            .iter()
                            .map(|fb| {
                                json!({
                                    "connection": fb.connection,
                                    "model": fb.model,
                                    "endpoint": fb.endpoint,
                                    "protocol": fb.protocol,
                                    "api_key_env": fb.api_key_env,
                                    "credential_ref": fb.credential_ref
                                })
                            })
                            .collect();
                        map.insert("fallbacks".into(), Value::Array(fbs));
                    }
                }
            }
            obj
        })
        .collect();
    let fallbacks: Vec<Value> = cfg
        .fallbacks
        .iter()
        .map(|fb| {
            json!({
                "connection": fb.connection,
                "model": fb.model,
                "endpoint": fb.endpoint,
                "protocol": fb.protocol,
                "api_key_env": fb.api_key_env,
                "credential_ref": fb.credential_ref
            })
        })
        .collect();
    let mut out = json!({
        "default_model": cfg.llm.default_model,
        "connections": connections
    });
    if !fallbacks.is_empty() {
        out["fallbacks"] = Value::Array(fallbacks);
    }
    out
}

/// Resolve any configured fallback routes (from the active connection's `fallbacks` and
/// top-level `cfg.fallbacks`) into concrete `ModelRouteConfig` entries.
pub fn resolve_fallback_routes(cfg: &AppliedConfig) -> Vec<ModelRouteConfig> {
    let mut specs: Vec<FallbackRouteConfig> = Vec::new();
    if let Some(sel) = &cfg.llm.default_model {
        if let Some(ex) = cfg.connection_extras.get(&sel.connection) {
            specs.extend(ex.fallbacks.iter().cloned());
        }
    }
    specs.extend(cfg.fallbacks.iter().cloned());

    let mut routes = Vec::new();
    for fb in specs {
        if let Some(conn_id) = fb.connection.as_deref().filter(|s| !s.trim().is_empty()) {
            if let Some(conn) = cfg.llm.connections.iter().find(|c| c.id == conn_id) {
                let model_id = fb
                    .model
                    .clone()
                    .or_else(|| conn.models.first().map(|m| m.id.clone()))
                    .unwrap_or_default();
                if model_id.is_empty() {
                    continue;
                }
                let entry = conn.models.iter().find(|m| m.id == model_id);
                let proto_raw = fb
                    .protocol
                    .as_deref()
                    .or(conn.protocol.as_deref())
                    .or(conn.provider.as_deref())
                    .unwrap_or("chat_completions");
                let protocol = ModelProtocol::parse(proto_raw).unwrap_or(ModelProtocol::OpenAiChat);
                routes.push(ModelRouteConfig {
                    connection_id: conn.id.clone(),
                    endpoint: fb.endpoint.clone().unwrap_or_else(|| conn.endpoint.clone()),
                    protocol,
                    model: model_id,
                    api_key: resolve_api_key(conn),
                    context_window: entry.and_then(|e| e.context_window).unwrap_or(128_000),
                    max_output_tokens: entry.and_then(|e| e.max_output_tokens),
                    temperature: entry.and_then(|e| e.temperature),
                    top_p: entry.and_then(|e| e.top_p),
                    capabilities: entry.and_then(|e| e.capabilities.clone()).unwrap_or_default(),
                    reasoning_effort: entry.and_then(|e| e.reasoning_effort.clone()),
                    prompt_cache_key: Some(format!("nola-v2-{}", conn.id)),
                });
                continue;
            }
        }
        if let (Some(ep), Some(model_id)) = (
            fb.endpoint.as_deref().filter(|s| !s.trim().is_empty()),
            fb.model.as_deref().filter(|s| !s.trim().is_empty()),
        ) {
            let protocol = fb
                .protocol
                .as_deref()
                .and_then(ModelProtocol::parse)
                .unwrap_or(ModelProtocol::OpenAiChat);
            let api_key = fb
                .api_key_env
                .as_deref()
                .and_then(|k| std::env::var(k).ok())
                .filter(|v| !v.trim().is_empty())
                .or_else(|| {
                    fb.api_key.as_deref().map(|raw| {
                        if let Some(env_name) = raw.strip_prefix("env:") {
                            std::env::var(env_name.trim()).unwrap_or_default()
                        } else {
                            raw.to_string()
                        }
                    })
                })
                .unwrap_or_default();
            routes.push(ModelRouteConfig {
                connection_id: fb.connection.unwrap_or_else(|| "fallback".into()),
                endpoint: ep.to_string(),
                protocol,
                model: model_id.to_string(),
                api_key,
                context_window: 128_000,
                max_output_tokens: None,
                temperature: None,
                top_p: None,
                capabilities: ModelCapabilities::default(),
                reasoning_effort: None,
                prompt_cache_key: Some("nola-v2-fallback".into()),
            });
        }
    }
    routes
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
            "You are Nola, a local-first personal AI assistant and software engineering agent.\n\
             Workspace root: {}\n\
             Principles:\n\
             - For greetings, general questions, or conversation that does not reference workspace files, answer directly without running workspace exploration tools (such as list_dir, glob, or bash).\n\
             - Inspect files before editing; make surgical, verified changes when working on a codebase or files.\n\
             - Use tool_search when you need specialized deferred tools (list_experts, delegate_expert, web_fetch, code_exec, lsp_diagnostics, skills, memory, goals, rooms, MCP).\n\
             - Never use bash, grep, or sqlite3 to probe Nola's own internal application data directories (~/.nola, Application Support/Nola/State, nola*.db, or vault files); use Nola's built-in tools instead.\n\
             - Verify your work with real tests/commands before concluding engineering tasks.",
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

    #[test]
    fn apply_config_resolves_credential_ref_preserves_names_and_parses_fallbacks() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory().unwrap();
        let vault_path = dir.path().join("vault.json");
        bind_store_vault_path(&store, &vault_path).unwrap();

        // Pre-populate Vault with secrets referenced via `credential_ref` and `credentialRef`
        let vault = Vault::open(vault_path).unwrap();
        vault.set_secret("vault_primary_key", "sk-vault-primary-123").unwrap();
        vault.set_secret("vault_backup_key", "sk-vault-backup-456").unwrap();

        let cfg_path = dir.path().join("config.jsonc");
        fs::write(
            &cfg_path,
            r#"{
              "llm": {
                "default_model": { "connection": "primary", "model": "gpt-4.1" },
                "connections": [
                  {
                    "id": "primary",
                    "name": "Primary OpenAI",
                    "endpoint": "https://api.openai.com/v1",
                    "protocol": "responses",
                    "credential_ref": "vault_primary_key",
                    "models": [
                      { "id": "gpt-4.1", "name": "GPT-4.1 Flagship" }
                    ],
                    "fallbacks": [
                      { "connection": "backup", "model": "claude-sonnet" }
                    ]
                  },
                  {
                    "id": "backup",
                    "name": "Backup Anthropic",
                    "endpoint": "https://api.anthropic.com/v1",
                    "protocol": "anthropic",
                    "credentialRef": "vault_backup_key",
                    "models": [
                      { "id": "claude-sonnet", "name": "Claude Sonnet" }
                    ]
                  }
                ],
                "fallbacks": [
                  {
                    "endpoint": "https://fallback.example.com/v1",
                    "model": "qwen3-coder",
                    "protocol": "chat_completions"
                  }
                ]
              }
            }"#,
        )
        .unwrap();

        let applied = apply_config_file(&store, &cfg_path).unwrap();
        assert_eq!(applied.llm.connections[0].api_key.as_deref(), Some("sk-vault-primary-123"));

        let loaded = load_applied_config(&store);
        assert_eq!(loaded.llm.connections[0].api_key.as_deref(), Some("sk-vault-primary-123"));
        assert_eq!(loaded.llm.connections[1].api_key.as_deref(), Some("sk-vault-backup-456"));
        assert_eq!(
            loaded.connection_extras.get("primary").and_then(|e| e.name.as_deref()),
            Some("Primary OpenAI")
        );
        assert_eq!(
            loaded
                .connection_extras
                .get("primary")
                .and_then(|e| e.model_names.get("gpt-4.1").map(String::as_str)),
            Some("GPT-4.1 Flagship")
        );

        let redacted = redacted_applied_llm_config(&loaded);
        let redacted_str = serde_json::to_string(&redacted).unwrap();
        assert!(!redacted_str.contains("sk-vault-primary-123"));
        assert!(!redacted_str.contains("sk-vault-backup-456"));
        assert_eq!(redacted["connections"][0]["name"], "Primary OpenAI");
        assert_eq!(redacted["connections"][0]["credential_ref"], "vault_primary_key");
        assert_eq!(redacted["connections"][0]["models"][0]["name"], "GPT-4.1 Flagship");
        assert_eq!(redacted["connections"][0]["hasApiKey"], true);
        assert_eq!(redacted["connections"][1]["name"], "Backup Anthropic");
        assert_eq!(redacted["connections"][1]["credential_ref"], "vault_backup_key");

        let fallbacks = resolve_fallback_routes(&loaded);
        assert_eq!(fallbacks.len(), 2);
        assert_eq!(fallbacks[0].connection_id, "backup");
        assert_eq!(fallbacks[0].model, "claude-sonnet");
        assert_eq!(fallbacks[0].api_key, "sk-vault-backup-456");
        assert_eq!(fallbacks[1].endpoint, "https://fallback.example.com/v1");
        assert_eq!(fallbacks[1].model, "qwen3-coder");
    }
}
