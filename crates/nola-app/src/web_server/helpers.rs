// SPDX-License-Identifier: Apache-2.0
//! Domain helpers, SSE chat turn execution, probes, gateway routing, OAuth flows, and static asset serving.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::Read;
use std::net::{IpAddr, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::Utc;
use nola_engine::{
    approve_agent_plan, is_plan_mode, load_session_plan, load_session_todos,
    register_planning_tools, register_subagent_tools, set_plan_mode, AgentEngine, AgentEvent,
    AgentRunConfig, ApprovalCallback, EffectivePolicy, ForceToolSet, HookRunner, HttpModelCaller,
    ModelCaller, PlanningState, SubagentManager, UserInputCallback,
};
use nola_llm::is_loopback_url;
use nola_mcp::McpManager;
use nola_modules::a2ui::{
    list_conversation_surfaces, register_scoped_tools as register_a2ui_tools,
};
use nola_modules::{
    built_in_role_templates, spawn_messaging_channel_worker, test_data_source_connection,
    ExtendedGoalRecord, GoalWorker, LocalModelProcessManager, ModulesSuite, RoomRuntime,
};
use nola_store::{EventPage, Store, Vault};
use nola_tools::git::GitTool;
use nola_tools::{
    extract_document_text_from_bytes, BrowserManager, NativeBrowserBridge, SandboxMode,
    ToolContext, ToolHandler, ToolRegistry,
};
use nola_types::{
    new_id, now_ms, sha256_hex, AgentRoom, CapabilityDomain, ExpertPeer, FrozenSystemBlock,
    GoalStatus, HostCapabilities, McpServerConfig, McpTransport, MediaAttachment, MemoryItem,
    PermissionAction, PermissionProfile, PermissionRule, RuleScope, ToolSchema, ToolSpec, ToolTier,
};
use serde_json::{json, Value};
use tiny_http::{Header, Method, Request, Response, StatusCode};
use url::Url;

use crate::config::{
    build_frozen_system_blocks, connection_has_api_key, default_home_dir, load_active_config,
    resolve_model_route, resolve_vault_path_for_store, ConnectionConfig, DefaultModelSelection,
    ModelEntry,
};
use crate::web_api::{
    build_agent_settings_snapshot, build_model_profiles_response, ensure_source_store_epoch,
    infer_model_capabilities, load_conversation_knowledge, memory_item_to_record_with_store,
    ms_to_rfc3339, reconstruct_v2_messages, session_meta_key,
};

use super::auth::{
    base64_decode, base64_encode, ct_eq, encode_sse_frame, extract_bearer_token,
    generate_hex_token_32, generate_pkce_pair, header_value, method_as_str, respond_api_json,
    OAuthFlowState, MAX_HTTP_BODY_BYTES,
};
use super::interactions::{
    approval_target_key, ChannelReader, DeviceToolProxy, PendingApprovalEntry, PendingInteractions,
    PendingQuestionEntry,
};
use super::paired::collect_conversation_resources;

pub(super) const MAX_GATEWAY_ATTEMPTS: usize = 200;

pub(super) fn is_allowed_module_kind(kind: &str) -> bool {
    matches!(
        kind,
        "goal" | "sop" | "automation" | "data_source" | "agent_room" | "plugin" | "local_model"
    )
}

pub(super) fn is_inbound_webhook_path(path: &str) -> bool {
    (path.starts_with("/api/messaging-channels/") && path.ends_with("/inbound"))
        || path.starts_with("/api/messaging/webhooks/")
        || path.starts_with("/v1/channels/webhook/")
        || path.starts_with("/api/v1/channels/webhook/")
}

pub(super) fn reconcile_messaging_worker(
    worker_slot: &Mutex<Option<Arc<AtomicBool>>>,
    store: &Store,
    workspace_root: &Path,
) -> bool {
    let snap = load_messaging_worker_snapshot(store);
    let enabled = snap
        .get("enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let mut guard = worker_slot.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(prev) = guard.take() {
        prev.store(true, Ordering::Relaxed);
    }
    if enabled {
        let stop_flag = Arc::new(AtomicBool::new(false));
        let vault_arc = Vault::open(resolve_vault_path_for_store(store))
            .ok()
            .map(Arc::new);
        let store_for_route = store.clone();
        let _handle = spawn_messaging_channel_worker(
            store.clone(),
            vault_arc,
            move || {
                let cfg = load_active_config(&store_for_route);
                resolve_model_route(&cfg).ok()
            },
            workspace_root.to_path_buf(),
            Arc::clone(&stop_flag),
        );
        *guard = Some(stop_flag);
        true
    } else {
        false
    }
}


pub(super) fn run_automation_bash_script(script: &str, cwd: &Path) -> Result<String, String> {
    let mut child = Command::new("bash")
        .args(["-lc", script])
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to spawn bash script: {e}"))?;
    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();
    let out_handle = std::thread::spawn(move || -> Vec<u8> {
        let mut buf = Vec::new();
        if let Some(p) = stdout_pipe {
            let _ = p.take(256 * 1024).read_to_end(&mut buf);
        }
        buf
    });
    let err_handle = std::thread::spawn(move || -> Vec<u8> {
        let mut buf = Vec::new();
        if let Some(p) = stderr_pipe {
            let _ = p.take(64 * 1024).read_to_end(&mut buf);
        }
        buf
    });
    let deadline = Instant::now() + Duration::from_secs(60);
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break st,
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = out_handle.join();
                    let _ = err_handle.join();
                    return Err("script automation timed out after 60s".into());
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("script wait failed: {e}"));
            }
        }
    };
    let stdout = String::from_utf8_lossy(&out_handle.join().unwrap_or_default()).into_owned();
    let stderr = String::from_utf8_lossy(&err_handle.join().unwrap_or_default()).into_owned();
    if !status.success() {
        let detail = if stderr.trim().is_empty() {
            format!("script exited with status {status}")
        } else {
            stderr.trim().to_string()
        };
        return Err(detail);
    }
    if stderr.trim().is_empty() {
        Ok(stdout)
    } else if stdout.trim().is_empty() {
        Ok(stderr)
    } else {
        Ok(format!("{stdout}\n[stderr]\n{stderr}"))
    }
}


pub(super) fn filter_superseded_memories(store: &Store, items: Vec<MemoryItem>) -> (Vec<Value>, Vec<MemoryItem>) {
    let projected: Vec<(Value, MemoryItem)> = items
        .into_iter()
        .map(|item| {
            let rec = memory_item_to_record_with_store(store, &item);
            (rec, item)
        })
        .collect();
    let mut superseded_ids: HashSet<String> = HashSet::new();
    for (rec, _) in &projected {
        if let Some(arr) = rec.get("supersedes").and_then(|v| v.as_array()) {
            for id in arr.iter().filter_map(|v| v.as_str()) {
                if !id.trim().is_empty() {
                    superseded_ids.insert(id.to_string());
                }
            }
        }
    }
    let mut active_records = Vec::new();
    let mut active_items = Vec::new();
    for (rec, item) in projected {
        if !superseded_ids.contains(&item.id) {
            active_records.push(rec);
            active_items.push(item);
        }
    }
    (active_records, active_items)
}


pub(super) fn resolve_connection_api_key(conn: &ConnectionConfig) -> Option<String> {
    conn.api_key_env
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .and_then(|k| std::env::var(k).ok())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .or_else(|| {
            conn.api_key.as_deref().and_then(|raw| {
                if let Some(env_name) = raw.trim().strip_prefix("env:") {
                    std::env::var(env_name.trim())
                        .ok()
                        .map(|v| v.trim().to_string())
                        .filter(|v| !v.is_empty())
                } else {
                    let trimmed = raw.trim();
                    (!trimmed.is_empty()).then(|| trimmed.to_string())
                }
            })
        })
}

pub(super) fn discover_endpoint_models(conn: &ConnectionConfig) -> Result<Vec<String>, String> {
    let base = conn.endpoint.trim().trim_end_matches('/');
    if base.is_empty() {
        return Err("endpoint URL is empty".to_string());
    }
    let proto = conn
        .protocol
        .as_deref()
        .unwrap_or("openai_chat")
        .trim()
        .to_ascii_lowercase();
    let is_anthropic = matches!(proto.as_str(), "anthropic" | "anthropic_messages");
    let is_gemini = matches!(proto.as_str(), "gemini" | "google_gemini");

    let url = if base.ends_with("/models") {
        base.to_string()
    } else if is_anthropic {
        if base.ends_with("/v1") || base.contains("/v1/") {
            format!("{base}/models")
        } else {
            format!("{base}/v1/models")
        }
    } else if is_gemini {
        if base.ends_with("/v1beta") || base.ends_with("/v1") {
            format!("{base}/models")
        } else {
            format!("{base}/v1beta/models")
        }
    } else {
        format!("{base}/models")
    };

    validate_configured_endpoint_url(&url)?;

    let mut builder = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none());
    if is_local_or_lan_url(&url) {
        builder = builder.no_proxy();
    }
    let client = builder
        .build()
        .map_err(|e| format!("failed to build HTTP client: {e}"))?;

    let mut rb = client.get(&url);
    if let Some(key) = resolve_connection_api_key(conn) {
        if is_anthropic {
            rb = rb
                .header("x-api-key", key)
                .header("anthropic-version", "2023-06-01");
        } else if is_gemini {
            rb = rb.header("x-goog-api-key", key);
        } else {
            rb = rb.bearer_auth(key);
        }
    } else if is_anthropic {
        rb = rb.header("anthropic-version", "2023-06-01");
    }

    let resp = rb
        .send()
        .map_err(|e| format!("failed to reach {url}: {e}"))?;
    let status = resp.status();
    let body_text = resp
        .text()
        .map_err(|e| format!("failed to read discovery response from {url}: {e}"))?;
    if !status.is_success() {
        let snippet: String = body_text.trim().chars().take(240).collect();
        return Err(if snippet.is_empty() {
            format!("upstream returned HTTP {}", status.as_u16())
        } else {
            format!("upstream returned HTTP {}: {snippet}", status.as_u16())
        });
    }

    let val: Value = serde_json::from_str(&body_text)
        .map_err(|e| format!("invalid JSON from {url}: {e}"))?;

    let mut models: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    if let Some(arr) = val.get("data").and_then(|v| v.as_array()) {
        for item in arr {
            if let Some(id) = item
                .get("id")
                .or_else(|| item.get("name"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                let clean = id.strip_prefix("models/").unwrap_or(id).to_string();
                if seen.insert(clean.clone()) {
                    models.push(clean);
                }
            }
        }
    }
    if let Some(arr) = val.get("models").and_then(|v| v.as_array()) {
        for item in arr {
            if let Some(raw_name) = item
                .get("name")
                .or_else(|| item.get("id"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                let clean = raw_name.strip_prefix("models/").unwrap_or(raw_name).to_string();
                if seen.insert(clean.clone()) {
                    models.push(clean);
                }
            }
        }
    }
    Ok(models)
}

#[derive(Debug, Clone)]
pub(super) struct ParsedImportModel {
    pub(super) model_id: String,
    pub(super) display_name: String,
    pub(super) protocol: String,
    pub(super) context_window: Option<u64>,
}

#[derive(Debug, Clone)]
pub(super) struct ParsedImportCandidate {
    pub(super) id: String,
    pub(super) display_name: String,
    pub(super) base_url: String,
    pub(super) protocol: String,
    pub(super) source: String,
    pub(super) models: Vec<ParsedImportModel>,
    pub(super) api_key: Option<String>,
    pub(super) api_key_env: Option<String>,
}

fn strip_jsonc_comments_and_trailing_commas(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut no_comments = String::with_capacity(raw.len());
    let mut i = 0;
    let mut in_string = false;
    let mut escaped = false;
    while i < bytes.len() {
        let ch = bytes[i] as char;
        if in_string {
            no_comments.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            i += 1;
            continue;
        }
        if ch == '"' {
            in_string = true;
            no_comments.push(ch);
            i += 1;
            continue;
        }
        if ch == '/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            i += 2;
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if ch == '/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(bytes.len());
            continue;
        }
        no_comments.push(ch);
        i += 1;
    }

    // Remove trailing commas before '}' or ']'
    let chars: Vec<char> = no_comments.chars().collect();
    let mut out = String::with_capacity(no_comments.len());
    let mut idx = 0;
    in_string = false;
    escaped = false;
    while idx < chars.len() {
        let ch = chars[idx];
        if in_string {
            out.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            idx += 1;
            continue;
        }
        if ch == '"' {
            in_string = true;
            out.push(ch);
            idx += 1;
            continue;
        }
        if ch == ',' {
            let mut look = idx + 1;
            while look < chars.len() && chars[look].is_whitespace() {
                look += 1;
            }
            if look < chars.len() && matches!(chars[look], '}' | ']') {
                idx += 1;
                continue;
            }
        }
        out.push(ch);
        idx += 1;
    }
    out
}

fn unquote_toml_string(val: &str) -> String {
    let trimmed = val.trim();
    if trimmed.len() >= 2 && trimmed.starts_with('"') && trimmed.ends_with('"') {
        let inner = &trimmed[1..trimmed.len() - 1];
        inner
            .replace("\\\"", "\"")
            .replace("\\\\", "\\")
            .replace("\\n", "\n")
            .replace("\\t", "\t")
    } else if trimmed.len() >= 2 && trimmed.starts_with('\'') && trimmed.ends_with('\'') {
        trimmed[1..trimmed.len() - 1].to_string()
    } else {
        trimmed.to_string()
    }
}

fn strip_toml_inline_comment(line: &str) -> &str {
    let mut in_double = false;
    let mut in_single = false;
    let mut escaped = false;
    for (idx, ch) in line.char_indices() {
        if in_double {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_double = false;
            }
        } else if in_single {
            if ch == '\'' {
                in_single = false;
            }
        } else if ch == '"' {
            in_double = true;
        } else if ch == '\'' {
            in_single = true;
        } else if ch == '#' {
            return &line[..idx];
        }
    }
    line
}

fn parse_simple_toml_tables(raw: &str) -> Result<BTreeMap<String, BTreeMap<String, String>>, String> {
    let mut tables: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    let mut current_section = String::new();
    for (line_no, raw_line) in raw.lines().enumerate() {
        let cleaned = strip_toml_inline_comment(raw_line).trim();
        if cleaned.is_empty() {
            continue;
        }
        if cleaned.starts_with('[') && cleaned.ends_with(']') {
            let header = cleaned
                .trim_start_matches('[')
                .trim_end_matches(']')
                .trim();
            if header.is_empty() {
                return Err(format!("empty TOML table header at line {}", line_no + 1));
            }
            // Strip quotes on dotted path segments like [providers."my-prov"]
            let normalized: String = header
                .split('.')
                .map(|seg| unquote_toml_string(seg.trim()))
                .collect::<Vec<_>>()
                .join(".");
            current_section = normalized;
            tables.entry(current_section.clone()).or_default();
            continue;
        }
        if let Some((k, v)) = cleaned.split_once('=') {
            let key = unquote_toml_string(k.trim());
            let val = unquote_toml_string(v.trim());
            if !key.is_empty() {
                tables
                    .entry(current_section.clone())
                    .or_default()
                    .insert(key, val);
            }
        } else if !cleaned.starts_with(']') {
            return Err(format!("invalid TOML syntax at line {}", line_no + 1));
        }
    }
    Ok(tables)
}

fn resolve_import_secret(
    raw_val: Option<&str>,
    env_key: Option<&str>,
    cred_map: &BTreeMap<String, String>,
    prov_id: &str,
    include_credentials: bool,
    allow_environment: bool,
) -> (Option<String>, Option<String>) {
    if !include_credentials {
        return (None, None);
    }
    if let Some(raw) = raw_val.map(str::trim).filter(|s| !s.is_empty()) {
        // Handle OpenCode `{env:VAR}` or Nola `env:VAR`
        let env_ref = raw
            .strip_prefix("{env:")
            .and_then(|s| s.strip_suffix('}'))
            .or_else(|| raw.strip_prefix("env:"));
        if let Some(var_name) = env_ref.map(str::trim).filter(|s| !s.is_empty()) {
            if allow_environment {
                let resolved = std::env::var(var_name)
                    .ok()
                    .map(|v| v.trim().to_string())
                    .filter(|v| !v.is_empty());
                return (resolved, Some(var_name.to_string()));
            }
            return (None, None);
        }
        return (Some(raw.to_string()), None);
    }
    if let Some(ek) = env_key.map(str::trim).filter(|s| !s.is_empty()) {
        if let Some(found) = cred_map.get(ek) {
            return (Some(found.clone()), Some(ek.to_string()));
        }
        if allow_environment {
            if let Ok(val) = std::env::var(ek) {
                let trimmed = val.trim().to_string();
                if !trimmed.is_empty() {
                    return (Some(trimmed), Some(ek.to_string()));
                }
            }
        }
    }
    if let Some(found) = cred_map
        .get(prov_id)
        .or_else(|| cred_map.get(&prov_id.to_ascii_lowercase()))
        .or_else(|| cred_map.get(&format!("{}_API_KEY", prov_id.to_ascii_uppercase())))
        .or_else(|| cred_map.get("OPENAI_API_KEY"))
    {
        return (Some(found.clone()), None);
    }
    (None, None)
}

fn load_credential_file_map(raw: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let cleaned = strip_jsonc_comments_and_trailing_commas(raw);
    if let Ok(Value::Object(map)) = serde_json::from_str::<Value>(&cleaned) {
        for (k, v) in map {
            if let Some(s) = v.as_str().map(str::trim).filter(|s| !s.is_empty()) {
                out.insert(k, s.to_string());
            } else if let Some(obj) = v.as_object() {
                if let Some(secret) = obj
                    .get("key")
                    .or_else(|| obj.get("apiKey"))
                    .or_else(|| obj.get("api_key"))
                    .or_else(|| obj.get("token"))
                    .or_else(|| obj.get("access"))
                    .and_then(|x| x.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    out.insert(k, secret.to_string());
                }
            }
        }
    }
    out
}

fn infer_import_wire_protocol(raw: Option<&str>, base_url: &str) -> String {
    let lower_raw = raw.unwrap_or("").trim().to_ascii_lowercase();
    let lower_url = base_url.to_ascii_lowercase();
    if lower_raw.contains("anthropic") || lower_url.contains("anthropic.com") {
        "anthropic".to_string()
    } else if lower_raw.contains("gemini")
        || lower_raw.contains("google")
        || lower_url.contains("generativelanguage.googleapis.com")
    {
        "gemini".to_string()
    } else if lower_raw == "responses" || lower_raw == "openai_responses" {
        "responses".to_string()
    } else {
        "chat_completions".to_string()
    }
}

pub(super) fn parse_model_import_candidates(
    source: &str,
    path_str: &str,
    raw: &str,
    credential_raw: Option<&str>,
    include_credentials: bool,
    allow_environment: bool,
) -> Result<Vec<ParsedImportCandidate>, String> {
    let cred_map = credential_raw
        .map(load_credential_file_map)
        .unwrap_or_default();
    let is_toml_hint = matches!(source, "codex" | "kimi_code")
        || path_str.to_ascii_lowercase().ends_with(".toml");

    // 1. Try JSON / JSONC when not explicitly TOML (or as fallback)
    let cleaned_json = strip_jsonc_comments_and_trailing_commas(raw);
    if !is_toml_hint {
        if let Ok(parsed) = serde_json::from_str::<Value>(&cleaned_json) {
            let mut candidates = Vec::new();
            // OpenCode format: { "provider": { "<id>": { options: { baseURL, apiKey }, models: {...} } } }
            if let Some(providers) = parsed.get("provider").and_then(|v| v.as_object()) {
                for (prov_id, prov_val) in providers {
                    let base_url = prov_val
                        .pointer("/options/baseURL")
                        .or_else(|| prov_val.get("baseURL"))
                        .or_else(|| prov_val.get("base_url"))
                        .or_else(|| prov_val.get("endpoint"))
                        .and_then(|v| v.as_str())
                        .map(str::trim)
                        .unwrap_or("")
                        .to_string();
                    if base_url.is_empty() {
                        continue;
                    }
                    let wire_proto = infer_import_wire_protocol(
                        prov_val
                            .get("protocol")
                            .or_else(|| prov_val.get("npm"))
                            .and_then(|v| v.as_str()),
                        &base_url,
                    );
                    let raw_key = prov_val
                        .pointer("/options/apiKey")
                        .or_else(|| prov_val.get("apiKey"))
                        .or_else(|| prov_val.get("api_key"))
                        .and_then(|v| v.as_str());
                    let (api_key, api_key_env) = resolve_import_secret(
                        raw_key,
                        None,
                        &cred_map,
                        prov_id,
                        include_credentials,
                        allow_environment,
                    );
                    let display_name = prov_val
                        .get("name")
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.trim().is_empty())
                        .unwrap_or(prov_id)
                        .to_string();
                    let mut models = Vec::new();
                    if let Some(m_map) = prov_val.get("models").and_then(|v| v.as_object()) {
                        for (mid, mval) in m_map {
                            let m_display = mval
                                .get("name")
                                .and_then(|v| v.as_str())
                                .filter(|s| !s.trim().is_empty())
                                .unwrap_or(mid)
                                .to_string();
                            let ctx = mval
                                .pointer("/limit/context")
                                .or_else(|| mval.get("context_window"))
                                .and_then(|v| v.as_u64());
                            models.push(ParsedImportModel {
                                model_id: mid.clone(),
                                display_name: m_display,
                                protocol: wire_proto.clone(),
                                context_window: ctx,
                            });
                        }
                    } else if let Some(m_arr) = prov_val.get("models").and_then(|v| v.as_array()) {
                        for item in m_arr {
                            if let Some(mid) = item
                                .as_str()
                                .or_else(|| item.get("id").and_then(|v| v.as_str()))
                                .or_else(|| item.get("model_id").and_then(|v| v.as_str()))
                            {
                                models.push(ParsedImportModel {
                                    model_id: mid.to_string(),
                                    display_name: mid.to_string(),
                                    protocol: wire_proto.clone(),
                                    context_window: item.get("context_window").and_then(|v| v.as_u64()),
                                });
                            }
                        }
                    }
                    candidates.push(ParsedImportCandidate {
                        id: prov_id.clone(),
                        display_name,
                        base_url,
                        protocol: wire_proto,
                        source: source.to_string(),
                        models,
                        api_key,
                        api_key_env,
                    });
                }
            }
            // Nola format: { "llm": { "connections": [...] } } or { "connections": [...] }
            let nola_conns = parsed
                .pointer("/llm/connections")
                .or_else(|| parsed.get("connections"))
                .and_then(|v| v.as_array());
            if let Some(conns) = nola_conns {
                for c in conns {
                    let Some(cid) = c
                        .get("id")
                        .and_then(|v| v.as_str())
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                    else {
                        continue;
                    };
                    let base_url = c
                        .get("endpoint")
                        .or_else(|| c.get("base_url"))
                        .or_else(|| c.get("baseURL"))
                        .and_then(|v| v.as_str())
                        .map(str::trim)
                        .unwrap_or("")
                        .to_string();
                    if base_url.is_empty() {
                        continue;
                    }
                    let wire_proto = infer_import_wire_protocol(
                        c.get("protocol").and_then(|v| v.as_str()),
                        &base_url,
                    );
                    let raw_key = c
                        .get("api_key")
                        .or_else(|| c.get("apiKey"))
                        .and_then(|v| v.as_str());
                    let env_key = c.get("api_key_env").and_then(|v| v.as_str());
                    let (api_key, api_key_env) = resolve_import_secret(
                        raw_key,
                        env_key,
                        &cred_map,
                        cid,
                        include_credentials,
                        allow_environment,
                    );
                    let models = c
                        .get("models")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|m| {
                                    let mid = m
                                        .as_str()
                                        .or_else(|| m.get("id").and_then(|v| v.as_str()))
                                        .or_else(|| m.get("model_id").and_then(|v| v.as_str()))?;
                                    Some(ParsedImportModel {
                                        model_id: mid.to_string(),
                                        display_name: m
                                            .get("display_name")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or(mid)
                                            .to_string(),
                                        protocol: wire_proto.clone(),
                                        context_window: m
                                            .get("context_window")
                                            .and_then(|v| v.as_u64()),
                                    })
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    candidates.push(ParsedImportCandidate {
                        id: cid.to_string(),
                        display_name: c
                            .get("display_name")
                            .and_then(|v| v.as_str())
                            .unwrap_or(cid)
                            .to_string(),
                        base_url,
                        protocol: wire_proto,
                        source: if source.is_empty() { "nola".to_string() } else { source.to_string() },
                        models,
                        api_key,
                        api_key_env,
                    });
                }
            }
            if !candidates.is_empty() {
                return Ok(candidates);
            }
            return Err("no supported providers or llm.connections found in JSON config".to_string());
        } else if !path_str.to_ascii_lowercase().ends_with(".toml") {
            // Try TOML as fallback if JSON failed
            if let Ok(tables) = parse_simple_toml_tables(raw) {
                let toml_cands = extract_candidates_from_toml(
                    source,
                    &tables,
                    &cred_map,
                    include_credentials,
                    allow_environment,
                );
                if !toml_cands.is_empty() {
                    return Ok(toml_cands);
                }
            }
            return Err("failed to parse config file as JSON/JSONC or TOML".to_string());
        }
    }

    // 2. TOML parsing (Codex config.toml & Kimi Code CLI config.toml)
    let tables = parse_simple_toml_tables(raw)?;
    let candidates = extract_candidates_from_toml(
        source,
        &tables,
        &cred_map,
        include_credentials,
        allow_environment,
    );
    if candidates.is_empty() {
        return Err("no supported model_providers or providers tables found in TOML config".to_string());
    }
    Ok(candidates)
}

fn extract_candidates_from_toml(
    source: &str,
    tables: &BTreeMap<String, BTreeMap<String, String>>,
    cred_map: &BTreeMap<String, String>,
    include_credentials: bool,
    allow_environment: bool,
) -> Vec<ParsedImportCandidate> {
    let mut candidates = Vec::new();
    let root = tables.get("");
    let root_model = root.and_then(|m| m.get("model")).cloned();
    let root_provider = root.and_then(|m| m.get("model_provider")).cloned();

    // Codex: [model_providers.<id>]
    for (section, kv) in tables {
        if let Some(prov_id) = section.strip_prefix("model_providers.") {
            let base_url = kv
                .get("base_url")
                .or_else(|| kv.get("baseURL"))
                .cloned()
                .unwrap_or_default();
            if base_url.is_empty() {
                continue;
            }
            let wire_proto = infer_import_wire_protocol(
                kv.get("wire_api")
                    .or_else(|| kv.get("protocol"))
                    .map(String::as_str),
                &base_url,
            );
            let (api_key, api_key_env) = resolve_import_secret(
                kv.get("api_key").map(String::as_str),
                kv.get("env_key").map(String::as_str),
                cred_map,
                prov_id,
                include_credentials,
                allow_environment,
            );
            let mut models = Vec::new();
            if root_provider.as_deref() == Some(prov_id) {
                if let Some(ref mid) = root_model {
                    models.push(ParsedImportModel {
                        model_id: mid.clone(),
                        display_name: mid.clone(),
                        protocol: wire_proto.clone(),
                        context_window: None,
                    });
                }
            }
            if let Some(mid) = kv.get("model").filter(|s| !s.is_empty()) {
                if !models.iter().any(|m| m.model_id == *mid) {
                    models.push(ParsedImportModel {
                        model_id: mid.clone(),
                        display_name: mid.clone(),
                        protocol: wire_proto.clone(),
                        context_window: None,
                    });
                }
            }
            candidates.push(ParsedImportCandidate {
                id: prov_id.to_string(),
                display_name: kv
                    .get("name")
                    .filter(|s| !s.is_empty())
                    .cloned()
                    .unwrap_or_else(|| prov_id.to_string()),
                base_url,
                protocol: wire_proto,
                source: source.to_string(),
                models,
                api_key,
                api_key_env,
            });
        }
    }

    // Kimi Code CLI: [providers.<id>] + [models.<alias>]
    let mut kimi_models_by_prov: BTreeMap<String, Vec<ParsedImportModel>> = BTreeMap::new();
    for (section, kv) in tables {
        if let Some(alias) = section.strip_prefix("models.") {
            if let (Some(prov), Some(mid)) = (kv.get("provider"), kv.get("model")) {
                let ctx = kv
                    .get("max_context_size")
                    .or_else(|| kv.get("context_window"))
                    .and_then(|s| s.parse::<u64>().ok());
                kimi_models_by_prov
                    .entry(prov.clone())
                    .or_default()
                    .push(ParsedImportModel {
                        model_id: mid.clone(),
                        display_name: alias.to_string(),
                        protocol: "chat_completions".to_string(),
                        context_window: ctx,
                    });
            }
        }
    }
    for (section, kv) in tables {
        if let Some(prov_id) = section.strip_prefix("providers.") {
            let base_url = kv
                .get("base_url")
                .or_else(|| kv.get("baseURL"))
                .cloned()
                .unwrap_or_default();
            if base_url.is_empty() {
                continue;
            }
            let wire_proto = infer_import_wire_protocol(
                kv.get("type")
                    .or_else(|| kv.get("protocol"))
                    .map(String::as_str),
                &base_url,
            );
            let (api_key, api_key_env) = resolve_import_secret(
                kv.get("api_key").map(String::as_str),
                kv.get("env_key").map(String::as_str),
                cred_map,
                prov_id,
                include_credentials,
                allow_environment,
            );
            let mut models = kimi_models_by_prov.remove(prov_id).unwrap_or_default();
            for m in &mut models {
                m.protocol = wire_proto.clone();
            }
            candidates.push(ParsedImportCandidate {
                id: prov_id.to_string(),
                display_name: kv
                    .get("name")
                    .filter(|s| !s.is_empty())
                    .cloned()
                    .unwrap_or_else(|| prov_id.to_string()),
                base_url,
                protocol: wire_proto,
                source: source.to_string(),
                models,
                api_key,
                api_key_env,
            });
        }
    }

    candidates
}

pub(super) fn bump_endpoint_credential_revision(store: &Store, ep_id: &str) -> i64 {
    let key = format!("endpoint_cred_rev:{ep_id}");
    let prev = store
        .get_kv(&key)
        .ok()
        .flatten()
        .and_then(|v| v.as_i64())
        .unwrap_or(1);
    let next = prev + 1;
    let _ = store.set_kv(&key, &json!(next));
    next
}

pub(super) fn build_endpoint_credential_response(store: &Store, ep_id: &str) -> Option<Value> {
    let cfg = load_active_config(store);
    let conn = cfg.llm.connections.iter().find(|c| c.id == ep_id)?;
    let wire_proto = match conn.protocol.as_deref().unwrap_or("chat_completions") {
        "anthropic" | "anthropic_messages" => "anthropic",
        "responses" | "openai_responses" => "responses",
        "siwc_responses" => "siwc_responses",
        "gemini" | "google_gemini" => "gemini",
        _ => "chat_completions",
    };
    let configured = connection_has_api_key(conn);
    let rev = store
        .get_kv(&format!("endpoint_cred_rev:{ep_id}"))
        .ok()
        .flatten()
        .and_then(|v| v.as_i64())
        .unwrap_or(1);
    let mut expected_revs = serde_json::Map::new();
    let profiles_resp = build_model_profiles_response(store);
    if let Some(arr) = profiles_resp.get("profiles").and_then(|v| v.as_array()) {
        for p in arr {
            if p.pointer("/endpoint/id").and_then(|v| v.as_str()) == Some(ep_id) {
                if let Some(pid) = p.get("id").and_then(|v| v.as_str()) {
                    let prev = p.get("revision").and_then(|v| v.as_i64()).unwrap_or(1).max(0);
                    expected_revs.insert(pid.to_string(), json!(prev));
                }
            }
        }
    }
    if expected_revs.is_empty() {
        expected_revs.insert(ep_id.to_string(), json!(1));
    }
    Some(json!({
        "ok": true,
        "endpointID": ep_id,
        "endpointId": ep_id,
        "baseURL": conn.endpoint,
        "wireProtocol": wire_proto,
        "hasCredential": configured,
        "credentialRevision": rev,
        "expectedRevisions": Value::Object(expected_revs),
        "configured": configured,
        "hasApiKey": configured
    }))
}


pub(super) fn build_goal_projection(store: &Store, conv_id: &str, goal: &Value) -> Value {
    let applied_operations = store
        .get_kv(&format!("conv_goal_ops:{conv_id}"))
        .ok()
        .flatten()
        .filter(|v| v.is_object())
        .unwrap_or_else(|| json!({}));
    if let Some(obj) = goal.as_object() {
        let id = obj
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if !id.is_empty() {
            let objective = obj
                .get("objective")
                .or_else(|| obj.get("description"))
                .or_else(|| obj.get("title"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let status = obj
                .get("controlState")
                .or_else(|| obj.get("status"))
                .and_then(|v| v.as_str())
                .unwrap_or("pending")
                .to_ascii_lowercase();
            let control_token = obj
                .get("control_token")
                .and_then(|v| v.as_str())
                .map(|s| json!(s))
                .unwrap_or(Value::Null);
            let constraints = obj
                .get("constraints")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            let next_wake = obj
                .get("next_wake_at_epoch_ms")
                .and_then(|v| v.as_i64())
                .map(|w| json!(w))
                .unwrap_or(Value::Null);
            let is_terminal = matches!(status.as_str(), "completed" | "cancelled" | "failed");
            let can_pause = !is_terminal && status != "paused";
            let can_resume = status == "paused";
            let can_cancel = !is_terminal;
            return json!({
                "active": {
                    "id": id,
                    "objective": objective,
                    "constraints": constraints,
                    "status": status,
                    "next_wake_at_epoch_ms": next_wake,
                    "requirements": {},
                    "work": { "entries": [] }
                },
                "control_token": control_token,
                "can_pause": can_pause,
                "can_resume": can_resume,
                "can_cancel": can_cancel,
                "applied_operations": applied_operations
            });
        }
    }
    json!({
        "active": Value::Null,
        "control_token": Value::Null,
        "can_pause": false,
        "can_resume": false,
        "can_cancel": false,
        "applied_operations": applied_operations
    })
}

pub(super) fn is_sensitive_data_source_record(suite: &ModulesSuite, record: &Value) -> bool {
    let kind_lc = record
        .get("kind")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !matches!(
        kind_lc.as_str(),
        "localfile"
            | "local_file"
            | "file"
            | "localfolder"
            | "local_folder"
            | "folder"
            | "dir"
            | "directory"
    ) {
        return false;
    }
    let Some(loc) = record
        .get("location")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return false;
    };
    let candidate = if Path::new(loc).is_absolute() {
        PathBuf::from(loc)
    } else {
        suite.workspace_root.join(loc)
    };
    is_sensitive_local_path(&candidate)
}

pub(super) fn managed_local_models_dir() -> PathBuf {
    for key in ["NOLA_MODELS_ROOT", "NOLA_MODELS_DIR"] {
        if let Ok(val) = std::env::var(key) {
            let trimmed = val.trim();
            if !trimmed.is_empty() {
                return PathBuf::from(trimmed);
            }
        }
    }
    if let Ok(nola_home) = std::env::var("NOLA_HOME") {
        let trimmed = nola_home.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed).join("models");
        }
    }
    default_home_dir().join(".nola/models")
}

pub(super) fn confined_local_model_dest_path(requested_dest: Option<&str>, fallback_name: &str) -> PathBuf {
    let sanitize_name = |raw: &str| -> String {
        let cleaned: String = raw
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                    c
                } else {
                    '-'
                }
            })
            .collect();
        cleaned.trim_matches('.').to_string()
    };
    // Always ignore arbitrary directory components / absolute paths supplied by HTTP callers;
    // extract at most a sanitized basename inside the managed models directory.
    let from_req = requested_dest
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .and_then(|s| Path::new(s).file_name().and_then(|n| n.to_str()))
        .map(sanitize_name)
        .filter(|s| !s.is_empty() && s != "." && s != "..");
    let final_name = from_req
        .or_else(|| {
            let s = sanitize_name(fallback_name);
            (!s.is_empty() && s != "." && s != "..").then_some(s)
        })
        .unwrap_or_else(|| format!("model-{}.gguf", &new_id()[..8]));
    managed_local_models_dir().join(final_name)
}

pub(super) fn build_local_model_download_url(
    source: &str,
    repo: &str,
    file_name: Option<&str>,
) -> Option<String> {
    if repo.trim().is_empty() {
        return None;
    }
    let fname = file_name.unwrap_or("model.gguf").trim_start_matches('/');
    let clean_repo = repo.trim_matches('/');
    if source.eq_ignore_ascii_case("modelscope") {
        let ms_base = std::env::var("NOLA_MODELSCOPE_MIRROR")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "https://modelscope.cn".to_string());
        Some(format!(
            "{}/models/{}/resolve/master/{}",
            ms_base.trim_end_matches('/'),
            clean_repo,
            fname
        ))
    } else {
        let mirror = std::env::var("NOLA_HF_MIRROR")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "https://huggingface.co".to_string());
        Some(format!(
            "{}/{}/resolve/main/{}",
            mirror.trim_end_matches('/'),
            clean_repo,
            fname
        ))
    }
}

pub(super) fn enrich_download_status_json(store: &Store, status: &mut Value) {
    let meta = store
        .get_kv("local_model_download_meta")
        .ok()
        .flatten();
    if let Some(obj) = status.as_object_mut() {
        if !obj.contains_key("id") {
            let id_val = obj
                .get("downloadId")
                .cloned()
                .or_else(|| meta.as_ref().and_then(|m| m.get("id").cloned()))
                .unwrap_or(Value::Null);
            obj.insert("id".into(), id_val);
        }
        if !obj.contains_key("repository") {
            let repo_val = meta
                .as_ref()
                .and_then(|m| m.get("repository").cloned())
                .unwrap_or_else(|| json!(""));
            obj.insert("repository".into(), repo_val);
        }
        if !obj.contains_key("source") {
            let src_val = meta
                .as_ref()
                .and_then(|m| m.get("source").cloned())
                .unwrap_or_else(|| json!("huggingface"));
            obj.insert("source".into(), src_val);
        }
        if obj.get("totalBytes").is_none() || obj.get("totalBytes") == Some(&Value::Null) {
            obj.insert("totalBytes".into(), json!(0));
        }
    }
}

pub(super) fn build_vllm_recommend_candidates(hardware: &Value) -> Vec<Value> {
    let unified = hardware
        .get("unifiedMemory")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let budget_bytes = if unified {
        hardware.get("memoryBytes").and_then(|v| v.as_u64())
    } else {
        hardware
            .get("gpuMemoryBytes")
            .and_then(|v| v.as_u64())
            .or_else(|| hardware.get("memoryBytes").and_then(|v| v.as_u64()))
    }
    .unwrap_or(16 * 1024 * 1024 * 1024);
    let device = if unified {
        "unified"
    } else if hardware.get("gpuMemoryBytes").and_then(|v| v.as_u64()).is_some() {
        "gpu"
    } else {
        "cpu"
    };
    let catalog = [
        (
            "Qwen/Qwen2.5-Coder-3B-Instruct",
            "model.safetensors",
            6_200_000_000u64,
            7_500_000_000u64,
            32_768u64,
        ),
        (
            "Qwen/Qwen2.5-Coder-7B-Instruct",
            "model.safetensors",
            14_800_000_000u64,
            16_500_000_000u64,
            32_768u64,
        ),
        (
            "Qwen/Qwen3-8B",
            "model.safetensors",
            16_200_000_000u64,
            18_000_000_000u64,
            131_072u64,
        ),
    ];
    let max_fit = (budget_bytes as f64 * 0.9) as u64;
    let mut out: Vec<Value> = catalog
        .iter()
        .filter(|(_, _, _, est_mem, _)| *est_mem <= max_fit || *est_mem <= 8_000_000_000)
        .map(|(repo, variant, dl_bytes, est_mem, ctx)| {
            json!({
                "repository": repo,
                "revision": "main",
                "variant": variant,
                "quantized": false,
                "downloadBytes": dl_bytes,
                "estimatedMemoryBytes": est_mem,
                "contextTokens": ctx,
                "nativeContextTokens": ctx,
                "contextLimitReason": if *ctx >= 131_072 { "target_met" } else { "native_limit" },
                "plan": {
                    "engine": "vllm",
                    "repository": repo,
                    "variant": variant,
                    "quantized": false,
                    "device": device,
                    "files": [{ "path": variant, "bytes": dl_bytes }]
                }
            })
        })
        .collect();
    out.reverse();
    out
}

pub(super) fn extract_hybrid_device_tool_specs(body: &Value) -> Vec<(ToolSpec, String)> {
    let mut out = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let sources = [
        body.get("deviceTools").and_then(|v| v.as_array()),
        body.get("device_tools").and_then(|v| v.as_array()),
        body.pointer("/device_bridge/tools").and_then(|v| v.as_array()),
        body.pointer("/deviceBridge/tools").and_then(|v| v.as_array()),
    ];
    for arr in sources.into_iter().flatten() {
        for dt in arr {
            let fn_obj = dt.pointer("/schema/function");
            let Some(name) = dt
                .get("name")
                .or_else(|| dt.get("tool_id"))
                .or_else(|| fn_obj.and_then(|f| f.get("name")))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
            else {
                continue;
            };
            if !seen.insert(name.to_string()) {
                continue;
            }
            let desc = dt
                .get("description")
                .or_else(|| fn_obj.and_then(|f| f.get("description")))
                .and_then(|v| v.as_str())
                .unwrap_or("Client-executed hybrid device tool")
                .to_string();
            let params = dt
                .get("parameters")
                .or_else(|| fn_obj.and_then(|f| f.get("parameters")))
                .filter(|v| v.is_object())
                .cloned()
                .unwrap_or_else(|| json!({"type": "object", "properties": {}}));
            let capability = dt
                .get("capability")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or("none")
                .to_string();
            let spec = ToolSpec {
                schema: ToolSchema {
                    name: name.to_string(),
                    description: desc,
                    parameters: params,
                },
                tier: ToolTier::Core,
                domain: CapabilityDomain::InteractiveUi,
                read_only: dt.get("read_only").and_then(|v| v.as_bool()).unwrap_or(false),
            };
            out.push((spec, capability));
        }
    }
    out
}

pub(super) fn is_sensitive_local_path(path: &Path) -> bool {
    let check_one = |p: &Path| -> bool {
        let home_dirs: Vec<PathBuf> = [
            Some(default_home_dir()),
            std::env::var("HOME").ok().map(PathBuf::from),
            std::env::var("USERPROFILE").ok().map(PathBuf::from),
        ]
        .into_iter()
        .flatten()
        .collect();
        for home in &home_dirs {
            let home_canon = fs::canonicalize(home).unwrap_or_else(|_| home.clone());
            for dir_name in [".nola", ".ssh", ".aws", ".gnupg"] {
                let forbidden_raw = home.join(dir_name);
                let forbidden_canon = home_canon.join(dir_name);
                if p.starts_with(&forbidden_raw) || p.starts_with(&forbidden_canon) {
                    return true;
                }
            }
        }
        for comp in p.components() {
            let s = comp.as_os_str().to_string_lossy().to_ascii_lowercase();
            if matches!(s.as_str(), ".nola" | ".ssh" | ".aws" | ".gnupg") {
                return true;
            }
        }
        if let Some(fname) = p.file_name().and_then(|n| n.to_str()) {
            let lower = fname.to_ascii_lowercase();
            if lower == ".env"
                || lower.starts_with(".env.")
                || lower.ends_with(".env")
                || lower == "vault.json"
                || lower == "vault.json.key"
                || lower.starts_with("vault.json.")
                || lower == "mobile-access-token"
                || lower.ends_with(".pem")
                || lower.ends_with(".key")
                || lower.ends_with(".p12")
                || lower.ends_with(".pfx")
                || lower == "id_rsa"
                || lower == "id_ed25519"
            {
                return true;
            }
        }
        false
    };

    if check_one(path) {
        return true;
    }
    if let Ok(canon) = fs::canonicalize(path) {
        if check_one(&canon) {
            return true;
        }
    }
    false
}

pub(super) fn redact_secrets_in_text(input: &str) -> String {
    let mut lines_out = Vec::new();
    for line in input.lines() {
        let lower = line.to_ascii_lowercase();
        if lower.contains("-----begin") && lower.contains("private key") {
            lines_out.push("[REDACTED PRIVATE KEY]".to_string());
            continue;
        }
        // Redact KEY/SECRET/TOKEN/PASSWORD=value assignments
        if let Some((lhs, rhs)) = line.split_once('=') {
            let lhs_lc = lhs.trim().to_ascii_lowercase();
            if !rhs.trim().is_empty()
                && (lhs_lc.contains("secret")
                    || lhs_lc.contains("token")
                    || lhs_lc.contains("password")
                    || lhs_lc.contains("api_key")
                    || lhs_lc.contains("apikey")
                    || lhs_lc.ends_with("_key"))
            {
                lines_out.push(format!("{lhs}=[REDACTED]"));
                continue;
            }
        }
        let mut redacted_tokens = Vec::new();
        let mut prev_was_bearer = false;
        for tok in line.split_whitespace() {
            let stripped = tok.trim_matches(|c: char| matches!(c, '"' | '\'' | ',' | ';' | ')' | '('));
            if prev_was_bearer {
                redacted_tokens.push("[REDACTED]".to_string());
                prev_was_bearer = false;
                continue;
            }
            if stripped.eq_ignore_ascii_case("bearer") {
                prev_was_bearer = true;
                redacted_tokens.push(tok.to_string());
                continue;
            }
            let looks_like_secret = (stripped.starts_with("sk-") && stripped.len() >= 12)
                || (stripped.starts_with("ghp_") && stripped.len() >= 12)
                || (stripped.starts_with("gho_") && stripped.len() >= 12)
                || (stripped.starts_with("AKIA") && stripped.len() >= 16)
                || (stripped.starts_with("AIza") && stripped.len() >= 20)
                || (stripped.starts_with("xoxb-") && stripped.len() >= 12)
                || (stripped.starts_with("nola_gw_") && stripped.len() >= 12);
            if looks_like_secret {
                redacted_tokens.push("[REDACTED]".to_string());
            } else {
                redacted_tokens.push(tok.to_string());
            }
        }
        lines_out.push(redacted_tokens.join(" "));
    }
    lines_out.join("\n")
}

pub(super) fn probe_and_attach_data_source_validation(
    suite: &ModulesSuite,
    id: &str,
    record: &mut Value,
) -> (bool, Value) {
    let mut probe_rec = record.clone();
    let kind_lc = probe_rec
        .get("kind")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let kind = record
        .get("kind")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    let max_age = record
        .get("maximumAgeMinutes")
        .and_then(|v| v.as_i64())
        .unwrap_or(60)
        .clamp(1, 10_080);
    let now_iso = Utc::now().to_rfc3339();

    if matches!(
        kind_lc.as_str(),
        "localfile"
            | "local_file"
            | "file"
            | "localfolder"
            | "local_folder"
            | "folder"
            | "dir"
            | "directory"
    ) {
        if let Some(loc) = probe_rec.get("location").and_then(|v| v.as_str()) {
            let loc_trimmed = loc.trim();
            if !loc_trimmed.is_empty() {
                let candidate = if Path::new(loc_trimmed).is_absolute() {
                    PathBuf::from(loc_trimmed)
                } else {
                    suite.workspace_root.join(loc_trimmed)
                };
                // SEC-7: Canonicalize & block sensitive local paths (~/.nola, ~/.ssh, ~/.aws, ~/.gnupg, .env, .pem, .key, vault.json.key)
                if is_sensitive_local_path(&candidate) {
                    let probe = json!({
                        "ok": false,
                        "status": "blocked",
                        "error": "access to sensitive local path is forbidden"
                    });
                    record["validationContract"] = json!({
                        "sourceId": id,
                        "readerDescription": format!("{kind} reader"),
                        "schemaFields": ["content"],
                        "redactedSample": "",
                        "recordCount": 0,
                        "maximumAgeMinutes": max_age,
                        "validationState": "failed",
                        "validatedAt": now_iso,
                        "errorMessage": "access to sensitive local path is forbidden"
                    });
                    record["lastError"] = json!("access to sensitive local path is forbidden");
                    record["updatedAt"] = json!(now_iso);
                    return (false, probe);
                }
                let resolved = fs::canonicalize(&candidate).unwrap_or(candidate);
                probe_rec["location"] = json!(resolved.display().to_string());
            }
        }
    }
    let probe = test_data_source_connection(&probe_rec);
    let ok = probe.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
    let state = probe
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or(if ok { "verified" } else { "failed" });
    let raw_sample = probe
        .get("preview")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let sample = redact_secrets_in_text(raw_sample);
    let record_count = probe
        .get("entryCount")
        .or_else(|| probe.get("bytesRead"))
        .or_else(|| probe.get("sizeBytes"))
        .and_then(|v| v.as_u64())
        .unwrap_or(if ok { 1 } else { 0 })
        .min(9_007_199_254_740_991);
    let err_msg = probe
        .get("error")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let mut contract = json!({
        "sourceId": id,
        "readerDescription": format!("{kind} reader"),
        "schemaFields": ["content"],
        "redactedSample": sample,
        "recordCount": record_count,
        "maximumAgeMinutes": max_age,
        "validationState": state,
        "validatedAt": now_iso
    });
    if let Some(err) = err_msg {
        contract["errorMessage"] = json!(err);
        record["lastError"] = json!(err);
    } else if let Some(obj) = record.as_object_mut() {
        obj.remove("lastError");
    }
    record["validationContract"] = contract;
    record["updatedAt"] = json!(now_iso);
    (ok, probe)
}

pub(super) fn parse_message_boundary(
    body: &Value,
) -> Result<Option<nola_store::HistoryBoundary<'_>>, &'static str> {
    if let Some(value) = ["fromSeq", "from_seq", "upToSeq", "up_to_seq"]
        .iter()
        .find_map(|key| body.get(*key))
    {
        return value
            .as_i64()
            .filter(|seq| *seq > 0)
            .map(nola_store::HistoryBoundary::Sequence)
            .map(Some)
            .ok_or("invalid_message_sequence");
    }
    let Some(value) = ["fromMessageId", "messageId", "message_id"]
        .iter()
        .find_map(|key| body.get(*key))
    else {
        return Ok(None);
    };
    let id = value.as_str().ok_or("invalid_message_id")?;
    if id.is_empty() {
        return Ok(None);
    }
    Ok(Some(nola_store::HistoryBoundary::Message(id)))
}

pub(super) fn build_activity_page(
    store: &Store,
    suite: &ModulesSuite,
    active_cancels: &Arc<Mutex<BTreeMap<String, Arc<AtomicBool>>>>,
    pending: &Arc<PendingInteractions>,
    body: &Value,
) -> Value {
    let source_filter = body
        .get("source")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let needs_response_only = body
        .get("needs_response")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let query_filter = body
        .get("query")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let limit = body
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(40)
        .clamp(1, 200) as usize;
    let cursor_ts = body
        .pointer("/cursor/created_at_ms")
        .and_then(|v| v.as_i64());
    let cursor_run_id = body
        .pointer("/cursor/run_id")
        .and_then(|v| v.as_str())
        .map(ToString::to_string);

    let active_ids: HashSet<String> = active_cancels.lock().unwrap().keys().cloned().collect();
    let mut all_entries: Vec<(i64, String, Value)> = Vec::new();

    if source_filter.is_none() || source_filter == Some("conversation") || source_filter == Some("agent") {
        for sess in store.list_sessions(200).unwrap_or_default() {
            let is_auto = sess.id.starts_with("auto-") || sess.id.starts_with("auto_");
            let src = if is_auto { "automation" } else { "conversation" };
            if let Some(f) = source_filter {
                if f != src && !(f == "agent" && src == "conversation") {
                    continue;
                }
            }
            let needs_resp = pending.conversation_needs_response(&sess.id);
            if needs_response_only && !needs_resp {
                continue;
            }
            let is_running = active_ids.contains(&sess.id);
            let (messages, events) = reconstruct_v2_messages(store, &sess.id);
            let last_assistant = messages
                .iter()
                .rev()
                .find(|m| m.get("role").and_then(|v| v.as_str()) == Some("assistant"));
            let last_ev = events.last();
            let last_seq = last_ev
                .and_then(|e| e.get("seq").and_then(|v| v.as_i64()))
                .unwrap_or(0);
            let created_ms = last_ev
                .and_then(|e| e.get("created_at_ms").and_then(|v| v.as_i64()))
                .unwrap_or(sess.updated_at_ms);
            let run_id = format!("run-{}-{last_seq}", sess.id);
            let assistant_msg_id = last_assistant
                .and_then(|m| m.get("id").and_then(|v| v.as_str()))
                .map(ToString::to_string)
                .unwrap_or_else(|| format!("msg-{last_seq}"));
            let summary: String = last_assistant
                .and_then(|m| m.get("content").and_then(|v| v.as_str()))
                .unwrap_or("")
                .chars()
                .take(240)
                .collect();
            let status = if needs_resp {
                "waiting_for_input"
            } else if is_running {
                "running"
            } else if last_assistant.is_some() {
                "completed"
            } else {
                "idle"
            };
            if !query_filter.is_empty() {
                let hay = format!("{} {}", sess.title, summary).to_ascii_lowercase();
                if !hay.contains(&query_filter) {
                    continue;
                }
            }
            let entry = json!({
                "id": run_id,
                "conversation_id": sess.id,
                "conversationId": sess.id,
                "run_id": run_id,
                "assistant_message_id": assistant_msg_id,
                "source": src,
                "kind": src,
                "title": sess.title,
                "summary": summary,
                "created_at_ms": created_ms,
                "timestamp": ms_to_rfc3339(created_ms),
                "completed_at_ms": if is_running { Value::Null } else { json!(created_ms) },
                "status": status,
                "needs_response": needs_resp,
                "is_current": is_running
            });
            all_entries.push((created_ms, run_id, entry));
        }
    }

    if source_filter.is_none() || source_filter == Some("automation") {
        for rec in suite.list_records::<Value>("automation_result").unwrap_or_default() {
            if needs_response_only {
                continue;
            }
            let rid = rec
                .get("runId")
                .or_else(|| rec.get("resultId"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if rid.is_empty() {
                continue;
            }
            let conv_id = rec
                .get("conversationId")
                .and_then(|v| v.as_str())
                .unwrap_or(&rid)
                .to_string();
            let title = rec
                .get("sourceName")
                .and_then(|v| v.as_str())
                .unwrap_or("Automation")
                .to_string();
            let summary = rec
                .get("previewText")
                .or_else(|| rec.get("llmSummaryText"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if !query_filter.is_empty() {
                let hay = format!("{title} {summary}").to_ascii_lowercase();
                if !hay.contains(&query_filter) {
                    continue;
                }
            }
            let created_ms = rec
                .get("createdAt")
                .and_then(|v| v.as_str())
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .map(|dt| dt.timestamp_millis())
                .unwrap_or_else(now_ms);
            let status = rec
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("answered");
            let entry = json!({
                "id": rid,
                "conversation_id": conv_id,
                "conversationId": conv_id,
                "run_id": rid,
                "assistant_message_id": format!("auto-{rid}"),
                "source": "automation",
                "kind": "automation",
                "title": title,
                "summary": summary,
                "created_at_ms": created_ms,
                "timestamp": ms_to_rfc3339(created_ms),
                "completed_at_ms": created_ms,
                "status": status,
                "needs_response": false,
                "is_current": false
            });
            all_entries.push((created_ms, rid, entry));
        }
    }

    all_entries.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));

    if let (Some(c_ts), Some(c_rid)) = (cursor_ts, cursor_run_id) {
        all_entries.retain(|(ts, rid, _)| (*ts, rid.as_str()) < (c_ts, c_rid.as_str()));
    }

    let has_more = all_entries.len() > limit;
    let page_slice: Vec<Value> = all_entries
        .into_iter()
        .take(limit)
        .map(|(_, _, v)| v)
        .collect();
    let next_cursor = if has_more {
        page_slice.last().map(|last| {
            json!({
                "created_at_ms": last["created_at_ms"],
                "run_id": last["run_id"]
            })
        })
    } else {
        None
    };

    json!({
        "entries": page_slice,
        "next_cursor": next_cursor,
        "items": page_slice,
        "hasMore": has_more
    })
}

pub(super) fn build_activity_detail(
    store: &Store,
    suite: &ModulesSuite,
    active_cancels: &Arc<Mutex<BTreeMap<String, Arc<AtomicBool>>>>,
    pending: &Arc<PendingInteractions>,
    host_id: &str,
    body: &Value,
) -> Value {
    let conv_id = body
        .get("conversation_id")
        .or_else(|| body.get("conversationId"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let run_id = body
        .get("run_id")
        .or_else(|| body.get("runId"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let assistant_msg_id = body
        .get("assistant_message_id")
        .or_else(|| body.get("assistantMessageId"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if let Ok(Some(sess)) = store.get_session(conv_id) {
        let (messages, _) = reconstruct_v2_messages(store, conv_id);
        let msg = messages
            .iter()
            .rev()
            .find(|m| m.get("id").and_then(|v| v.as_str()) == Some(assistant_msg_id))
            .or_else(|| {
                messages
                    .iter()
                    .rev()
                    .find(|m| m.get("role").and_then(|v| v.as_str()) == Some("assistant"))
            });
        let result_text = msg
            .and_then(|m| m.get("content").and_then(|v| v.as_str()))
            .map(ToString::to_string);
        let explanation = msg
            .and_then(|m| m.get("reasoning").and_then(|v| v.as_str()))
            .filter(|s| !s.is_empty())
            .map(ToString::to_string);
        let is_running = active_cancels.lock().unwrap().contains_key(conv_id);
        let needs_resp = pending.conversation_needs_response(conv_id);
        let entry = json!({
            "conversation_id": conv_id,
            "run_id": if run_id.is_empty() { format!("run-{conv_id}") } else { run_id.to_string() },
            "assistant_message_id": if assistant_msg_id.is_empty() { "msg-0" } else { assistant_msg_id },
            "source": "conversation",
            "title": sess.title,
            "created_at_ms": sess.updated_at_ms,
            "completed_at_ms": if is_running { Value::Null } else { json!(sess.updated_at_ms) },
            "status": if needs_resp { "waiting_for_input" } else if is_running { "running" } else { "completed" },
            "needs_response": needs_resp,
            "is_current": is_running
        });
        return json!({
            "entry": entry,
            "explanation": explanation,
            "result_text": result_text,
            "result_has_more": false,
            "source_conversation_id": conv_id,
            "review_in_source": needs_resp,
            "execution_host_id": host_id,
            "context_scope": "conversation"
        });
    }

    if let Some(rec) = suite
        .list_records::<Value>("automation_result")
        .unwrap_or_default()
        .into_iter()
        .find(|r| {
            r.get("runId").and_then(|v| v.as_str()) == Some(run_id)
                || r.get("conversationId").and_then(|v| v.as_str()) == Some(conv_id)
        })
    {
        let created_ms = rec
            .get("createdAt")
            .and_then(|v| v.as_str())
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.timestamp_millis())
            .unwrap_or_else(now_ms);
        let entry = json!({
            "conversation_id": conv_id,
            "run_id": run_id,
            "assistant_message_id": assistant_msg_id,
            "source": "automation",
            "title": rec.get("sourceName").and_then(|v| v.as_str()).unwrap_or("Automation"),
            "created_at_ms": created_ms,
            "completed_at_ms": created_ms,
            "status": rec.get("status").and_then(|v| v.as_str()).unwrap_or("answered"),
            "needs_response": false,
            "is_current": false
        });
        return json!({
            "entry": entry,
            "explanation": Value::Null,
            "result_text": rec.get("llmSummaryText").cloned().unwrap_or(Value::Null),
            "result_has_more": false,
            "source_conversation_id": conv_id,
            "review_in_source": false,
            "execution_host_id": host_id,
            "context_scope": "task"
        });
    }

    Value::Null
}

pub(super) fn build_model_connections_inventory(store: &Store, suite: &ModulesSuite) -> Value {
    let cfg = load_active_config(store);
    let default_sel = cfg.llm.default_model.as_ref();
    let mut connections: Vec<Value> = Vec::new();
    let mut selections: Vec<Value> = Vec::new();
    let mut seen_ids: HashSet<String> = HashSet::new();
    let now = now_ms();

    let model_conn_records: BTreeMap<String, Value> = suite
        .list_records::<Value>("model_connection")
        .unwrap_or_default()
        .into_iter()
        .filter_map(|rec| {
            let id = rec
                .get("id")
                .or_else(|| rec.pointer("/connection/id"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())?
                .to_string();
            Some((id, rec))
        })
        .collect();

    let vault_opt = Vault::open(resolve_vault_path_for_store(store)).ok();

    for conn in &cfg.llm.connections {
        seen_ids.insert(conn.id.clone());
        let rec_opt = model_conn_records.get(&conn.id);
        let wire_proto = match conn.protocol.as_deref().unwrap_or("chat_completions") {
            "anthropic" | "anthropic_messages" => "anthropic",
            "responses" | "openai_responses" => "responses",
            "siwc_responses" => "siwc_responses",
            "gemini" | "google_gemini" => "gemini",
            _ => "chat_completions",
        };
        let configured = connection_has_api_key(conn);
        let saved_sel_enabled: Option<HashSet<String>> = store
            .get_kv(&format!("model_conn_selections:{}", conn.id))
            .ok()
            .flatten()
            .and_then(|v| v.get("enabled").and_then(|a| a.as_array()).cloned())
            .map(|arr| {
                arr.into_iter()
                    .filter_map(|x| x.as_str().map(ToString::to_string))
                    .collect()
            });
        let mut catalog_models: Vec<Value> = Vec::new();
        let mut seen_models: HashSet<String> = HashSet::new();
        let mut capability_assessments: Vec<Value> = Vec::new();
        let mut seen_assess: HashSet<String> = HashSet::new();
        for m in &conn.models {
            seen_models.insert(m.id.clone());
            let caps = infer_model_capabilities(&m.id, m.capabilities.as_ref());
            let mut modalities = vec!["text".to_string()];
            if caps.image_input {
                modalities.push("image".to_string());
            }
            let catalog_key = format!("{wire_proto}:{}", m.id);
            let is_enabled = match saved_sel_enabled.as_ref() {
                Some(set) => set.contains(&catalog_key) || set.contains(&m.id),
                None => true,
            };
            let is_selected = is_enabled
                && default_sel
                    .map(|d| d.connection == conn.id && d.model == m.id)
                    .unwrap_or(false);
            selections.push(json!({
                "connection_id": conn.id,
                "catalog_key": catalog_key,
                "enabled": is_enabled,
                "selected": is_selected
            }));
            catalog_models.push(json!({
                "model_id": m.id,
                "display_name": m.id,
                "protocol": wire_proto,
                "context_window": m.context_window,
                "modalities": modalities,
                "reasoning_levels": caps.supported_reasoning_levels,
                "summary_supported": caps.supports_reasoning_summary
            }));
            let override_kv = store
                .get_kv(&format!("model_cap_override:{}:{}", conn.id, m.id))
                .ok()
                .flatten()
                .filter(|v| v.is_object());
            let probe_kv = store
                .get_kv(&format!("model_cap_probe:{}:{}", conn.id, m.id))
                .ok()
                .flatten()
                .filter(|v| v.is_object());
            let user_override = override_kv
                .as_ref()
                .and_then(|v| v.get("user_override"))
                .cloned()
                .filter(|v| !v.is_null());
            let evidence = probe_kv
                .as_ref()
                .and_then(|v| v.get("evidence"))
                .cloned()
                .filter(|v| !v.is_null());
            if user_override.is_some() || evidence.is_some() {
                let assess_proto = override_kv
                    .as_ref()
                    .and_then(|v| v.get("protocol").and_then(|s| s.as_str()))
                    .or_else(|| {
                        probe_kv
                            .as_ref()
                            .and_then(|v| v.get("protocol").and_then(|s| s.as_str()))
                    })
                    .unwrap_or(wire_proto);
                seen_assess.insert(format!("{assess_proto}:{}", m.id));
                capability_assessments.push(json!({
                    "model_id": m.id,
                    "protocol": assess_proto,
                    "evidence": evidence.unwrap_or(Value::Null),
                    "user_override": user_override.unwrap_or(Value::Null)
                }));
            }
        }
        // Merge any additional catalog models or assessments stored on the `model_connection` record.
        if let Some(rec) = rec_opt {
            if let Some(rec_models) = rec
                .pointer("/catalog/models")
                .or_else(|| rec.get("models"))
                .and_then(|v| v.as_array())
            {
                for rm in rec_models {
                    let Some(mid) = rm
                        .get("model_id")
                        .or_else(|| rm.get("id"))
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.is_empty())
                    else {
                        continue;
                    };
                    if !seen_models.insert(mid.to_string()) {
                        continue;
                    }
                    let m_proto = rm
                        .get("protocol")
                        .and_then(|v| v.as_str())
                        .unwrap_or(wire_proto);
                    let ckey = format!("{m_proto}:{mid}");
                    let is_enabled = match saved_sel_enabled.as_ref() {
                        Some(set) => set.contains(&ckey) || set.contains(mid),
                        None => false,
                    };
                    selections.push(json!({
                        "connection_id": conn.id,
                        "catalog_key": ckey,
                        "enabled": is_enabled,
                        "selected": false
                    }));
                    catalog_models.push(rm.clone());
                }
            }
            if let Some(rec_assess) = rec
                .get("capability_assessments")
                .or_else(|| rec.pointer("/document/capability_assessments"))
                .and_then(|v| v.as_array())
            {
                for a in rec_assess {
                    let mid = a.get("model_id").and_then(|v| v.as_str()).unwrap_or("");
                    let proto = a.get("protocol").and_then(|v| v.as_str()).unwrap_or(wire_proto);
                    if !mid.is_empty() && seen_assess.insert(format!("{proto}:{mid}")) {
                        capability_assessments.push(a.clone());
                    }
                }
            }
        }
        let rec_rev = rec_opt
            .and_then(|r| r.get("revision").and_then(|v| v.as_i64()))
            .unwrap_or(1);
        let rec_display = rec_opt
            .and_then(|r| {
                r.get("display_name")
                    .or_else(|| r.pointer("/connection/display_name"))
                    .and_then(|v| v.as_str())
            })
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(&conn.id);
        let rec_source = rec_opt
            .and_then(|r| {
                r.get("source")
                    .or_else(|| r.pointer("/connection/source"))
                    .and_then(|v| v.as_str())
            })
            .or(conn.provider.as_deref())
            .unwrap_or("config");
        let rec_source_id = rec_opt
            .and_then(|r| {
                r.get("source_id")
                    .or_else(|| r.pointer("/connection/source_id"))
                    .cloned()
            })
            .unwrap_or(Value::Null);
        let rec_account_id = rec_opt
            .and_then(|r| {
                r.get("account_id")
                    .or_else(|| r.pointer("/connection/account_id"))
                    .cloned()
            })
            .unwrap_or(Value::Null);
        let rec_enabled = rec_opt
            .and_then(|r| {
                r.get("enabled")
                    .or_else(|| r.pointer("/connection/enabled"))
                    .and_then(|v| v.as_bool())
            })
            .unwrap_or(true);
        let observed_at_ms = rec_opt
            .and_then(|r| r.pointer("/catalog/observed_at_ms").and_then(|v| v.as_i64()))
            .unwrap_or(now);

        let mut bindings = serde_json::Map::new();
        if configured {
            bindings.insert(wire_proto.to_string(), json!("vault"));
        } else if is_loopback_url(&conn.endpoint) {
            bindings.insert(wire_proto.to_string(), json!("none"));
        }
        connections.push(json!({
            "revision": rec_rev,
            "hasApiKey": configured,
            "probe_credential_bindings": Value::Object(bindings),
            "document": {
                "format_version": 1,
                "connection": {
                    "id": conn.id,
                    "display_name": rec_display,
                    "base_url": conn.endpoint,
                    "protocols": [wire_proto],
                    "source": rec_source,
                    "source_id": rec_source_id,
                    "account_id": rec_account_id,
                    "enabled": rec_enabled
                },
                "catalog": {
                    "observed_at_ms": observed_at_ms,
                    "models": catalog_models
                },
                "capability_assessments": capability_assessments
            }
        }));
    }

    for (id, rec) in &model_conn_records {
        if seen_ids.contains(id) {
            continue;
        }
        let base_url = rec
            .get("base_url")
            .or_else(|| rec.get("endpoint"))
            .or_else(|| rec.pointer("/connection/base_url"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let protocols: Vec<String> = rec
            .get("protocols")
            .or_else(|| rec.pointer("/connection/protocols"))
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(ToString::to_string))
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
            })
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| {
                vec![rec
                    .get("protocol")
                    .and_then(|v| v.as_str())
                    .unwrap_or("chat_completions")
                    .to_string()]
            });
        let primary_proto = protocols
            .first()
            .map(String::as_str)
            .unwrap_or("chat_completions");
        let revision = rec
            .get("revision")
            .and_then(|v| v.as_i64())
            .unwrap_or(1);
        let account_id_val = rec
            .get("account_id")
            .or_else(|| rec.pointer("/connection/account_id"))
            .cloned()
            .unwrap_or(Value::Null);
        let account_id_str = account_id_val.as_str().filter(|s| !s.trim().is_empty());
        let source_str = rec
            .get("source")
            .or_else(|| rec.pointer("/connection/source"))
            .and_then(|v| v.as_str())
            .unwrap_or("custom");
        let source_id_val = rec
            .get("source_id")
            .or_else(|| rec.pointer("/connection/source_id"))
            .cloned()
            .unwrap_or(Value::Null);
        let display_name = rec
            .get("display_name")
            .or_else(|| rec.pointer("/connection/display_name"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(id);
        let enabled = rec
            .get("enabled")
            .or_else(|| rec.pointer("/connection/enabled"))
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let observed_at_ms = rec
            .pointer("/catalog/observed_at_ms")
            .and_then(|v| v.as_i64())
            .unwrap_or(now);

        let saved_sel_enabled: Option<HashSet<String>> = store
            .get_kv(&format!("model_conn_selections:{id}"))
            .ok()
            .flatten()
            .and_then(|v| v.get("enabled").and_then(|a| a.as_array()).cloned())
            .map(|arr| {
                arr.into_iter()
                    .filter_map(|x| x.as_str().map(ToString::to_string))
                    .collect()
            });

        let mut catalog_models: Vec<Value> = Vec::new();
        let mut capability_assessments: Vec<Value> = rec
            .get("capability_assessments")
            .or_else(|| rec.pointer("/document/capability_assessments"))
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let mut seen_assess: HashSet<String> = capability_assessments
            .iter()
            .filter_map(|a| {
                let mid = a.get("model_id").and_then(|v| v.as_str())?;
                let p = a.get("protocol").and_then(|v| v.as_str()).unwrap_or(primary_proto);
                Some(format!("{p}:{mid}"))
            })
            .collect();

        if let Some(raw_models) = rec
            .pointer("/catalog/models")
            .or_else(|| rec.get("models"))
            .and_then(|v| v.as_array())
        {
            for rm in raw_models {
                let Some(mid) = rm
                    .get("model_id")
                    .or_else(|| rm.get("id"))
                    .and_then(|v| v.as_str())
                    .or_else(|| rm.as_str())
                    .filter(|s| !s.is_empty())
                else {
                    continue;
                };
                let m_proto = rm
                    .get("protocol")
                    .and_then(|v| v.as_str())
                    .unwrap_or(primary_proto);
                let caps = infer_model_capabilities(mid, None);
                let mut modalities = vec!["text".to_string()];
                if caps.image_input {
                    modalities.push("image".to_string());
                }
                let catalog_key = format!("{m_proto}:{mid}");
                let is_enabled = match saved_sel_enabled.as_ref() {
                    Some(set) => set.contains(&catalog_key) || set.contains(mid),
                    None => true,
                };
                let is_selected = is_enabled
                    && default_sel
                        .map(|d| d.connection == *id && d.model == mid)
                        .unwrap_or(false);
                selections.push(json!({
                    "connection_id": id,
                    "catalog_key": catalog_key,
                    "enabled": is_enabled,
                    "selected": is_selected
                }));
                let normalized_model = if rm.is_object() {
                    let mut mobj = rm.as_object().cloned().unwrap_or_default();
                    mobj.entry("model_id".to_string()).or_insert_with(|| json!(mid));
                    mobj.entry("display_name".to_string()).or_insert_with(|| json!(mid));
                    mobj.entry("protocol".to_string()).or_insert_with(|| json!(m_proto));
                    mobj.entry("context_window".to_string()).or_insert(Value::Null);
                    mobj.entry("modalities".to_string()).or_insert_with(|| json!(modalities));
                    mobj.entry("reasoning_levels".to_string())
                        .or_insert_with(|| json!(caps.supported_reasoning_levels));
                    Value::Object(mobj)
                } else {
                    json!({
                        "model_id": mid,
                        "display_name": mid,
                        "protocol": m_proto,
                        "context_window": Value::Null,
                        "modalities": modalities,
                        "reasoning_levels": caps.supported_reasoning_levels,
                        "summary_supported": caps.supports_reasoning_summary
                    })
                };
                catalog_models.push(normalized_model);

                let override_kv = store
                    .get_kv(&format!("model_cap_override:{id}:{mid}"))
                    .ok()
                    .flatten()
                    .filter(|v| v.is_object());
                let probe_kv = store
                    .get_kv(&format!("model_cap_probe:{id}:{mid}"))
                    .ok()
                    .flatten()
                    .filter(|v| v.is_object());
                let user_override = override_kv
                    .as_ref()
                    .and_then(|v| v.get("user_override"))
                    .cloned()
                    .filter(|v| !v.is_null());
                let evidence = probe_kv
                    .as_ref()
                    .and_then(|v| v.get("evidence"))
                    .cloned()
                    .filter(|v| !v.is_null());
                if (user_override.is_some() || evidence.is_some())
                    && seen_assess.insert(format!("{m_proto}:{mid}"))
                {
                    capability_assessments.push(json!({
                        "model_id": mid,
                        "protocol": m_proto,
                        "evidence": evidence.unwrap_or(Value::Null),
                        "user_override": user_override.unwrap_or(Value::Null)
                    }));
                }
            }
        }

        let has_vault_key = vault_opt.as_ref().is_some_and(|v| {
            v.get_secret(&crate::config::vault_secret_key(id))
                .ok()
                .flatten()
                .is_some_and(|s| !s.trim().is_empty())
                || account_id_str.is_some_and(|aid| {
                    v.get_secret(&format!("model_account_secret:{aid}"))
                        .ok()
                        .flatten()
                        .is_some_and(|s| !s.trim().is_empty())
                        || v.get_secret(&format!("source_account_token:{aid}"))
                            .ok()
                            .flatten()
                            .is_some_and(|s| !s.trim().is_empty())
                })
        });
        let mut rec_bindings = serde_json::Map::new();
        for p in &protocols {
            if has_vault_key {
                rec_bindings.insert(p.clone(), json!("vault"));
            } else if is_loopback_url(base_url) {
                rec_bindings.insert(p.clone(), json!("none"));
            }
        }
        connections.push(json!({
            "revision": revision,
            "hasApiKey": has_vault_key,
            "probe_credential_bindings": Value::Object(rec_bindings),
            "document": {
                "format_version": 1,
                "connection": {
                    "id": id,
                    "display_name": display_name,
                    "base_url": base_url,
                    "protocols": protocols,
                    "source": source_str,
                    "source_id": source_id_val,
                    "account_id": account_id_val,
                    "enabled": enabled
                },
                "catalog": {
                    "observed_at_ms": observed_at_ms,
                    "models": catalog_models
                },
                "capability_assessments": capability_assessments
            }
        }));
    }

    let mut accounts: Vec<Value> = Vec::new();
    let mut seen_account_ids: HashSet<String> = HashSet::new();
    for kind in ["model_account", "source_account"] {
        for rec in suite.list_records::<Value>(kind).unwrap_or_default() {
            let Some(id) = rec
                .get("id")
                .or_else(|| rec.get("account_id"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
            else {
                continue;
            };
            if !seen_account_ids.insert(id.to_string()) {
                continue;
            }
            let rev = rec.get("revision").and_then(|v| v.as_i64()).unwrap_or(1);
            let display_name = rec
                .get("display_name")
                .or_else(|| rec.get("name"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .unwrap_or(id);
            let source = rec
                .get("source")
                .or_else(|| rec.get("provider"))
                .and_then(|v| v.as_str())
                .unwrap_or("custom");
            accounts.push(json!({
                "revision": rev,
                "account": {
                    "id": id,
                    "display_name": display_name,
                    "source": source,
                    "state": rec.get("state").and_then(|v| v.as_str()).unwrap_or("connected")
                }
            }));
        }
    }

    json!({
        "connections": connections,
        "accounts": accounts,
        "selections": selections
    })
}

pub(super) fn same_origin_url(a: &str, b: &str) -> bool {
    let (Ok(ua), Ok(ub)) = (Url::parse(a), Url::parse(b)) else {
        return false;
    };
    let (Some(ha), Some(hb)) = (ua.host_str(), ub.host_str()) else {
        return false;
    };
    ua.scheme().eq_ignore_ascii_case(ub.scheme())
        && ha.eq_ignore_ascii_case(hb)
        && ua.port_or_known_default() == ub.port_or_known_default()
}

pub(super) fn is_disallowed_probe_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            if v4.is_loopback() {
                return false;
            }
            let oct = v4.octets();
            v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_multicast()
                || v4.is_broadcast()
                || oct[0] == 0
                || (oct[0] == 100 && (oct[1] & 0b1100_0000) == 64) // CGNAT 100.64.0.0/10
        }
        IpAddr::V6(v6) => {
            if v6.is_loopback() {
                return false;
            }
            if let Some(mapped_v4) = v6.to_ipv4_mapped() {
                return is_disallowed_probe_ip(IpAddr::V4(mapped_v4));
            }
            let segs = v6.segments();
            v6.is_unspecified()
                || v6.is_multicast()
                || (segs[0] & 0xfe00) == 0xfc00 // ULA fc00::/7
                || (segs[0] & 0xffc0) == 0xfe80 // Link-local fe80::/10
        }
    }
}

pub(super) fn validate_probe_url_ssrf(raw_url: &str) -> Result<Url, String> {
    let parsed = Url::parse(raw_url).map_err(|e| format!("invalid probe URL: {e}"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("probe URL must use http:// or https://".into());
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| "probe URL missing host".to_string())?;
    let host_lc = host.to_ascii_lowercase();
    if matches!(
        host_lc.as_str(),
        "metadata.google.internal" | "metadata.goog" | "169.254.169.254"
    ) {
        return Err("probe target rejected: cloud metadata host is blocked".into());
    }
    let clean_host = host.trim_start_matches('[').trim_end_matches(']');
    if clean_host.eq_ignore_ascii_case("localhost") {
        return Ok(parsed);
    }
    if let Ok(ip) = clean_host.parse::<IpAddr>() {
        if is_disallowed_probe_ip(ip) {
            return Err(format!(
                "probe target rejected: private or link-local IP {ip} is blocked"
            ));
        }
        return Ok(parsed);
    }
    let port = parsed.port_or_known_default().unwrap_or(80);
    if let Ok(addrs) = (clean_host, port).to_socket_addrs() {
        for sock in addrs {
            if is_disallowed_probe_ip(sock.ip()) {
                return Err(format!(
                    "probe target rejected: host resolves to private or link-local IP {}",
                    sock.ip()
                ));
            }
        }
    }
    Ok(parsed)
}

fn is_disallowed_configured_endpoint_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            if v4.is_loopback() || v4.is_private() {
                return false;
            }
            let oct = v4.octets();
            v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_multicast()
                || v4.is_broadcast()
                || oct[0] == 0
        }
        IpAddr::V6(v6) => {
            if v6.is_loopback() {
                return false;
            }
            if let Some(mapped_v4) = v6.to_ipv4_mapped() {
                return is_disallowed_configured_endpoint_ip(IpAddr::V4(mapped_v4));
            }
            let segs = v6.segments();
            if (segs[0] & 0xfe00) == 0xfc00 {
                return false; // ULA LAN fc00::/7
            }
            v6.is_unspecified()
                || v6.is_multicast()
                || (segs[0] & 0xffc0) == 0xfe80 // Link-local fe80::/10
        }
    }
}

pub(super) fn is_local_or_lan_url(raw_url: &str) -> bool {
    if is_loopback_url(raw_url) {
        return true;
    }
    let Ok(parsed) = Url::parse(raw_url) else {
        return false;
    };
    let Some(host) = parsed.host_str() else {
        return false;
    };
    let clean = host.trim_start_matches('[').trim_end_matches(']');
    match clean.parse::<IpAddr>() {
        Ok(IpAddr::V4(v4)) => v4.is_loopback() || v4.is_private(),
        Ok(IpAddr::V6(v6)) => v6.is_loopback() || (v6.segments()[0] & 0xfe00) == 0xfc00,
        Err(_) => false,
    }
}

/// Validates a user-configured saved model endpoint URL: still blocks cloud metadata
/// (`169.254.169.254`, `metadata.google.internal`), link-local, unspecified, multicast,
/// and broadcast addresses, while allowing owner-configured LAN endpoints (RFC1918 / ULA).
pub(super) fn validate_configured_endpoint_url(raw_url: &str) -> Result<Url, String> {
    let parsed = Url::parse(raw_url).map_err(|e| format!("invalid probe URL: {e}"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("probe URL must use http:// or https://".into());
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| "probe URL missing host".to_string())?;
    let host_lc = host.to_ascii_lowercase();
    if matches!(
        host_lc.as_str(),
        "metadata.google.internal" | "metadata.goog" | "169.254.169.254"
    ) {
        return Err("probe target rejected: cloud metadata host is blocked".into());
    }
    let clean_host = host.trim_start_matches('[').trim_end_matches(']');
    if clean_host.eq_ignore_ascii_case("localhost") {
        return Ok(parsed);
    }
    if let Ok(ip) = clean_host.parse::<IpAddr>() {
        if is_disallowed_configured_endpoint_ip(ip) {
            return Err(format!(
                "probe target rejected: link-local or invalid IP {ip} is blocked"
            ));
        }
        return Ok(parsed);
    }
    let port = parsed.port_or_known_default().unwrap_or(80);
    if let Ok(addrs) = (clean_host, port).to_socket_addrs() {
        for sock in addrs {
            if is_disallowed_configured_endpoint_ip(sock.ip()) {
                return Err(format!(
                    "probe target rejected: host resolves to link-local or invalid IP {}",
                    sock.ip()
                ));
            }
        }
    }
    Ok(parsed)
}

pub(super) fn run_real_model_connection_probe(store: &Store, body: &Value) -> Value {
    let start = Instant::now();
    let cfg = load_active_config(store);
    let conn_id = body
        .get("connection_id")
        .or_else(|| body.get("connectionId"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let model_id = body
        .get("model_id")
        .or_else(|| body.get("modelId"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let profile_id = body.get("profile_id").cloned().unwrap_or(Value::Null);

    let conn = cfg
        .llm
        .connections
        .iter()
        .find(|c| c.id == conn_id)
        .or_else(|| {
            profile_id
                .as_str()
                .and_then(|pid| pid.split_once(':'))
                .and_then(|(cid, _)| cfg.llm.connections.iter().find(|c| c.id == cid))
        });

    let caller_endpoint = body
        .get("endpoint")
        .or_else(|| body.get("base_url"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string);

    let base_url = caller_endpoint
        .clone()
        .or_else(|| conn.map(|c| c.endpoint.clone()))
        .unwrap_or_default();

    let caps = infer_model_capabilities(
        model_id,
        conn.and_then(|c| c.models.iter().find(|m| m.id == model_id))
            .and_then(|m| m.capabilities.as_ref()),
    );

    if base_url.is_empty() {
        return json!({
            "ok": false,
            "reachable": false,
            "httpStatus": Value::Null,
            "latencyMs": start.elapsed().as_millis() as u64,
            "error": "missing endpoint/connection_id for probe",
            "profile_id": profile_id,
            "capabilities": {
                "toolCalls": caps.tool_calls,
                "imageInput": caps.image_input,
                "reasoning": caps.reasoning,
                "samplingParameters": caps.sampling_parameters
            }
        });
    }

    let trimmed = base_url.trim_end_matches('/');
    let probe_url = if trimmed.ends_with("/models") {
        trimmed.to_string()
    } else {
        format!("{trimmed}/models")
    };
    // SEC-3: Allow RFC1918 LAN endpoints when probing a saved owner-configured connection;
    // strictly block private/link-local/metadata IPs for arbitrary caller-supplied URLs.
    let matches_saved_connection = conn.is_some_and(|c| {
        caller_endpoint
            .as_deref()
            .is_none_or(|ep| same_origin_url(ep, &c.endpoint))
    });
    let ssrf_check = if matches_saved_connection {
        validate_configured_endpoint_url(&probe_url)
    } else {
        validate_probe_url_ssrf(&probe_url)
    };
    if let Err(ssrf_err) = ssrf_check {
        return json!({
            "ok": false,
            "reachable": false,
            "httpStatus": Value::Null,
            "latencyMs": start.elapsed().as_millis() as u64,
            "error": ssrf_err,
            "profile_id": profile_id,
            "capabilities": {
                "toolCalls": caps.tool_calls,
                "imageInput": caps.image_input,
                "reasoning": caps.reasoning,
                "samplingParameters": caps.sampling_parameters
            }
        });
    }

    let mut builder = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none());
    if is_loopback_url(&probe_url) {
        builder = builder.no_proxy();
    }
    let client = match builder.build() {
        Ok(c) => c,
        Err(e) => {
            return json!({
                "ok": false,
                "reachable": false,
                "httpStatus": Value::Null,
                "latencyMs": start.elapsed().as_millis() as u64,
                "error": e.to_string(),
                "profile_id": profile_id
            });
        }
    };

    let mut req_builder = client.get(&probe_url);
    // SEC-3: NEVER attach a saved connection's Vault/env API key when caller-supplied
    // `endpoint` has a different scheme/host/port than `conn.endpoint`!
    let allow_saved_credential = match (conn, caller_endpoint.as_deref()) {
        (Some(_), None) => true,
        (Some(c), Some(supplied)) => same_origin_url(supplied, &c.endpoint),
        (None, _) => false,
    };
    if let (Some(c), true) = (conn, allow_saved_credential) {
        let key = c
            .api_key_env
            .as_deref()
            .and_then(|k| std::env::var(k).ok())
            .filter(|v| !v.trim().is_empty())
            .or_else(|| {
                c.api_key.as_deref().and_then(|raw| {
                    if let Some(env_name) = raw.strip_prefix("env:") {
                        std::env::var(env_name.trim()).ok()
                    } else if !raw.trim().is_empty() {
                        Some(raw.trim().to_string())
                    } else {
                        None
                    }
                })
            });
        if let Some(k) = key {
            if c.protocol.as_deref() == Some("anthropic") {
                req_builder = req_builder
                    .header("x-api-key", k)
                    .header("anthropic-version", "2023-06-01");
            } else {
                req_builder = req_builder.bearer_auth(k);
            }
        }
    }

    match req_builder.send() {
        Ok(resp) => {
            let status_code = resp.status().as_u16();
            let ok = resp.status().is_success();
            if ok && !conn_id.is_empty() && !model_id.is_empty() {
                let proto = body
                    .get("protocol")
                    .and_then(|v| v.as_str())
                    .unwrap_or("chat_completions");
                let cred_binding = body
                    .get("credential_binding")
                    .cloned()
                    .unwrap_or_else(|| json!("vault"));
                let account_rev = body
                    .get("account_revision")
                    .cloned()
                    .unwrap_or(Value::Null);
                let _ = store.set_kv(
                    &format!("model_cap_probe:{conn_id}:{model_id}"),
                    &json!({
                        "model_id": model_id,
                        "protocol": proto,
                        "evidence": {
                            "text_generation": true,
                            "credential_binding": cred_binding,
                            "account_revision": account_rev,
                            "observed_at_ms": now_ms(),
                            "requests_sent": 1
                        }
                    }),
                );
            }
            json!({
                "ok": ok,
                "reachable": true,
                "httpStatus": status_code,
                "latencyMs": start.elapsed().as_millis() as u64,
                "error": if ok { Value::Null } else { json!(format!("upstream returned HTTP {status_code}")) },
                "profile_id": profile_id,
                "capabilities": {
                    "toolCalls": caps.tool_calls,
                    "imageInput": caps.image_input,
                    "reasoning": caps.reasoning,
                    "samplingParameters": caps.sampling_parameters
                }
            })
        }
        Err(e) => json!({
            "ok": false,
            "reachable": false,
            "httpStatus": Value::Null,
            "latencyMs": start.elapsed().as_millis() as u64,
            "error": e.to_string(),
            "profile_id": profile_id,
            "capabilities": {
                "toolCalls": caps.tool_calls,
                "imageInput": caps.image_input,
                "reasoning": caps.reasoning,
                "samplingParameters": caps.sampling_parameters
            }
        }),
    }
}

pub(super) fn handle_gateway_route(
    req: Request,
    store: &Store,
    suite: &Arc<ModulesSuite>,
    method: &Method,
    path: &str,
    query: &str,
    raw_body: &[u8],
) {
    let enabled = store
        .get_kv("model_gateway_access")
        .ok()
        .flatten()
        .and_then(|v| v.get("enabled").and_then(|b| b.as_bool()))
        .unwrap_or(false);
    let expected_hash = store
        .get_kv("gateway_token_hash")
        .ok()
        .flatten()
        .and_then(|v| v.as_str().map(ToString::to_string))
        .unwrap_or_default();
    let supplied = extract_bearer_token(&req).unwrap_or("");
    let supplied_hash = sha256_hex(supplied.as_bytes());
    if !enabled
        || expected_hash.is_empty()
        || supplied.is_empty()
        || !ct_eq(&supplied_hash, &expected_hash)
    {
        let _ = respond_json(
            req,
            401,
            &json!({"error": {"message": "invalid or disabled gateway bearer token", "type": "authentication_error"}}),
        );
        return;
    }

    let suffix = path.trim_start_matches("/gateway/v1");
    let decoded_suffix = percent_decode_path(suffix);
    if suffix.contains("..")
        || decoded_suffix.contains("..")
        || suffix.contains('\\')
        || decoded_suffix.contains('\\')
        || decoded_suffix.contains('\0')
    {
        let _ = respond_json(
            req,
            400,
            &json!({"error": {"message": "invalid gateway path"}}),
        );
        return;
    }

    let mut cfg = load_active_config(store);
    if *method == Method::Get && path == "/gateway/v1/models" {
        let mut data: Vec<Value> = Vec::new();
        for conn in &cfg.llm.connections {
            for m in &conn.models {
                data.push(json!({
                    "id": format!("{}:{}", conn.id, m.id),
                    "object": "model",
                    "owned_by": conn.id
                }));
            }
        }
        let attempt_id = new_id();
        let _ = suite.upsert_record(
            "gateway_attempt",
            &attempt_id,
            &json!({
                "id": attempt_id,
                "profile_id": "gateway:models",
                "path": path,
                "outcome": "succeeded",
                "status": 200,
                "usage": Value::Null,
                "created_at_ms": now_ms()
            }),
        );
        let _ = respond_json(req, 200, &json!({"object": "list", "data": data}));
        return;
    }

    // Resolve requested `body["model"]` against `cfg.llm.connections` (and `web_model_profiles`)!
    let mut forward_body = raw_body.to_vec();
    if let Ok(mut parsed_body) = serde_json::from_slice::<Value>(raw_body) {
        if let Some(req_model) = parsed_body
            .get("model")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string)
        {
            if let Some((conn_id, model_id)) = resolve_requested_model_selection(store, &cfg, &req_model) {
                cfg.llm.default_model = Some(DefaultModelSelection {
                    connection: conn_id,
                    model: model_id.clone(),
                });
                if model_id != req_model {
                    if let Some(obj) = parsed_body.as_object_mut() {
                        obj.insert("model".into(), json!(model_id));
                    }
                    if let Ok(rewritten) = serde_json::to_vec(&parsed_body) {
                        forward_body = rewritten;
                    }
                }
            }
        }
    }

    let route = match resolve_model_route(&cfg) {
        Ok(r) => r,
        Err(e) => {
            let attempt_id = new_id();
            let _ = suite.upsert_record(
                "gateway_attempt",
                &attempt_id,
                &json!({
                    "id": attempt_id,
                    "profile_id": "unconfigured",
                    "path": path,
                    "outcome": "failed",
                    "status": 503,
                    "usage": Value::Null,
                    "error": e,
                    "created_at_ms": now_ms()
                }),
            );
            let _ = respond_json(req, 503, &json!({"error": {"message": e}}));
            return;
        }
    };

    let base = route.endpoint.trim_end_matches('/');
    let upstream_url = if query.is_empty() {
        format!("{base}{suffix}")
    } else {
        format!("{base}{suffix}?{query}")
    };
    let profile_id = format!("{}:{}", route.connection_id, route.model);
    let mut builder = reqwest::blocking::Client::builder().timeout(Duration::from_secs(120));
    if is_loopback_url(&upstream_url) {
        builder = builder.no_proxy();
    }
    let Ok(client) = builder.build() else {
        let _ = respond_json(req, 500, &json!({"error": "failed to build proxy client"}));
        return;
    };

    let reqwest_method = reqwest::Method::from_bytes(method_as_str(method).as_bytes())
        .unwrap_or(reqwest::Method::POST);
    let mut proxy_req = client.request(reqwest_method, &upstream_url);
    if !matches!(method, Method::Get | Method::Head | Method::Options) && !forward_body.is_empty() {
        let ct_in = header_value(&req, "content-type").unwrap_or("application/json");
        proxy_req = proxy_req
            .header("Content-Type", ct_in)
            .body(forward_body);
    }
    if !route.api_key.trim().is_empty() {
        if path == "/gateway/v1/messages" {
            proxy_req = proxy_req
                .header("x-api-key", route.api_key.trim())
                .header("anthropic-version", "2023-06-01");
        } else {
            proxy_req = proxy_req.bearer_auth(route.api_key.trim());
        }
    }

    let attempt_id = new_id();
    match proxy_req.send() {
        Ok(mut resp) => {
            let status_code = resp.status().as_u16();
            let ct = resp
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("application/json")
                .to_string();
            let header = Header::from_bytes(&b"Content-Type"[..], ct.as_bytes()).unwrap_or_else(
                |_| Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap(),
            );
            let outcome = if (200..300).contains(&status_code) {
                "succeeded"
            } else {
                "failed"
            };
            // Stream live SSE responses chunk-by-chunk instead of buffering 16 MiB!
            if ct.to_ascii_lowercase().starts_with("text/event-stream") {
                let (tx, rx) = mpsc::channel::<Vec<u8>>();
                let suite_c = Arc::clone(suite);
                let attempt_id_c = attempt_id.clone();
                let profile_id_c = profile_id.clone();
                let path_c = path.to_string();
                std::thread::spawn(move || {
                    let mut buf = [0u8; 4096];
                    loop {
                        match resp.read(&mut buf) {
                            Ok(0) | Err(_) => break,
                            Ok(n) => {
                                if tx.send(buf[..n].to_vec()).is_err() {
                                    break;
                                }
                            }
                        }
                    }
                    let _ = suite_c.upsert_record(
                        "gateway_attempt",
                        &attempt_id_c,
                        &json!({
                            "id": attempt_id_c,
                            "profile_id": profile_id_c,
                            "path": path_c,
                            "outcome": outcome,
                            "status": status_code,
                            "usage": Value::Null,
                            "created_at_ms": now_ms()
                        }),
                    );
                });
                let headers = vec![
                    header,
                    Header::from_bytes(&b"Cache-Control"[..], &b"no-cache"[..]).unwrap(),
                ];
                let response = Response::new(
                    StatusCode(status_code),
                    headers,
                    ChannelReader::new(rx),
                    None,
                    None,
                );
                let _ = req.respond(response);
                return;
            }

            let mut resp_bytes = Vec::new();
            let _ = resp
                .by_ref()
                .take(MAX_HTTP_BODY_BYTES)
                .read_to_end(&mut resp_bytes);
            let parsed_usage = serde_json::from_slice::<Value>(&resp_bytes)
                .ok()
                .and_then(|v| v.get("usage").cloned())
                .map(|u| {
                    json!({
                        "input_tokens": u.get("prompt_tokens").or_else(|| u.get("input_tokens")).and_then(|x| x.as_u64()),
                        "output_tokens": u.get("completion_tokens").or_else(|| u.get("output_tokens")).and_then(|x| x.as_u64())
                    })
                })
                .unwrap_or(Value::Null);
            let _ = suite.upsert_record(
                "gateway_attempt",
                &attempt_id,
                &json!({
                    "id": attempt_id,
                    "profile_id": profile_id,
                    "path": path,
                    "outcome": outcome,
                    "status": status_code,
                    "usage": parsed_usage,
                    "created_at_ms": now_ms()
                }),
            );
            let response = Response::from_data(resp_bytes)
                .with_status_code(StatusCode(status_code))
                .with_header(header);
            let _ = req.respond(response);
        }
        Err(e) => {
            let _ = suite.upsert_record(
                "gateway_attempt",
                &attempt_id,
                &json!({
                    "id": attempt_id,
                    "profile_id": profile_id,
                    "path": path,
                    "outcome": "failed",
                    "status": 502,
                    "usage": Value::Null,
                    "error": e.to_string(),
                    "created_at_ms": now_ms()
                }),
            );
            let _ = respond_json(req, 502, &json!({"error": {"message": e.to_string()}}));
        }
    }
}

pub(super) fn resolve_requested_model_selection(
    store: &Store,
    cfg: &crate::config::AppliedConfig,
    mid: &str,
) -> Option<(String, String)> {
    // 1. Check `web_model_profiles` (handles user-created remote model UUIDs!)
    if let Ok(Some(Value::Array(profiles))) = store.get_kv("web_model_profiles") {
        if let Some(prof) = profiles.iter().find(|p| {
            p.get("id").and_then(|v| v.as_str()) == Some(mid)
                || p.get("profile_id").and_then(|v| v.as_str()) == Some(mid)
        }) {
            let ep_id = prof
                .pointer("/endpoint/id")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let model_name = prof
                .get("modelName")
                .or_else(|| prof.get("model_name"))
                .or_else(|| prof.get("modelId"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty());
            if let (Some(cid), Some(mname)) = (ep_id, model_name) {
                if cfg.llm.connections.iter().any(|c| c.id == cid) {
                    return Some((cid.to_string(), mname.to_string()));
                }
            }
        }
    }
    // 2. Check `conn_id:model_id` ONLY when `conn_id` matches a real `connection.id`
    //    (so model names like `llama3:8b` are NEVER split incorrectly!)
    if let Some((conn_id, model_name)) = mid.split_once(':') {
        if !model_name.is_empty() && cfg.llm.connections.iter().any(|c| c.id == conn_id) {
            return Some((conn_id.to_string(), model_name.to_string()));
        }
    }
    // 3. Direct match on any connection's model list
    if let Some(conn) = cfg
        .llm
        .connections
        .iter()
        .find(|c| c.models.iter().any(|m| m.id == mid))
    {
        return Some((conn.id.clone(), mid.to_string()));
    }
    // 4. Match running local models in LocalModelProcessManager cleanly
    let running = LocalModelProcessManager::global().list_running_models();
    let stripped_local = mid.strip_prefix("local:").unwrap_or(mid);
    if let Some(rt) = running.iter().find(|r| {
        let rid = r.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let rmodel = r.get("model").and_then(|v| v.as_str()).unwrap_or("");
        rid == mid || rid == stripped_local || rmodel == mid
    }) {
        if let Some(rid) = rt
            .get("id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
        {
            return Some((format!("local:{rid}"), rid.to_string()));
        }
    }
    None
}

pub(super) fn load_messaging_worker_snapshot(store: &Store) -> Value {
    let saved = store
        .get_kv("messaging_channel_worker")
        .ok()
        .flatten()
        .unwrap_or_else(|| json!({}));
    let poll_secs = saved
        .get("poll_seconds")
        .or_else(|| saved.get("poll_interval_secs"))
        .and_then(|v| v.as_i64())
        .unwrap_or(5)
        .clamp(1, 300);
    let mut out = json!({
        "enabled": saved.get("enabled").and_then(|v| v.as_bool()).unwrap_or(false),
        "backend_url": saved.get("backend_url").or_else(|| saved.get("url")).and_then(|v| v.as_str()).unwrap_or(""),
        "has_access_token": saved.get("has_access_token").and_then(|v| v.as_bool()).unwrap_or(false),
        "execution_backend": saved.get("execution_backend").and_then(|v| v.as_str()).unwrap_or("nola"),
        "turn_timeout_seconds": saved.get("turn_timeout_seconds").and_then(|v| v.as_i64()).unwrap_or(600).clamp(30, 14400),
        "poll_seconds": poll_secs,
        "poll_interval_secs": poll_secs,
        "project_root": saved.get("project_root").and_then(|v| v.as_str()).filter(|s| !s.is_empty()).map(|s| json!(s)).unwrap_or(Value::Null)
    });
    if let Some(dst) = out.as_object_mut() {
        for key in [
            "heartbeat_at_ms",
            "polls",
            "messages_processed",
            "outbound_sent",
            "last_error",
        ] {
            if let Some(val) = saved.get(key) {
                dst.insert(key.into(), val.clone());
            }
        }
    }
    out
}

pub(super) fn channel_vault_secret_key(conn_id: &str) -> String {
    format!("msg_webhook_secret:{conn_id}")
}

pub(super) fn persist_channel_secret_and_normalize(store: &Store, raw: &Value) -> Value {
    let conn_id = raw
        .get("connection_id")
        .or_else(|| raw.get("id"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(new_id);
    if let Some(secret) = raw
        .get("webhook_secret")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        if let Ok(vault) = Vault::open(resolve_vault_path_for_store(store)) {
            let _ = vault.set_secret(&channel_vault_secret_key(&conn_id), secret);
        }
    }
    let mut cleaned = raw.clone();
    if let Some(obj) = cleaned.as_object_mut() {
        obj.insert("connection_id".into(), json!(conn_id));
        obj.remove("webhook_secret");
    }
    normalize_messaging_channel(store, &cleaned)
}

pub(super) fn normalize_messaging_channel(store: &Store, raw: &Value) -> Value {
    let conn_id = raw
        .get("connection_id")
        .or_else(|| raw.get("id"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(new_id);
    let kind = raw
        .get("kind")
        .and_then(|v| v.as_str())
        .unwrap_or("telegram");
    let display_name = raw
        .get("display_name")
        .or_else(|| raw.get("name"))
        .and_then(|v| v.as_str())
        .unwrap_or(&conn_id);
    let account_id = raw
        .get("account_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let credential_ref = raw
        .get("credential_ref")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let webhook_secret_ref = raw
        .get("webhook_secret_ref")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let verify_token_ref = raw
        .get("verify_token_ref")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let provider_config = raw
        .get("provider_config")
        .filter(|v| v.is_object())
        .cloned()
        .unwrap_or_else(|| json!({}));
    let enabled = raw.get("enabled").and_then(|v| v.as_bool()).unwrap_or(false);
    let state = raw
        .get("state")
        .and_then(|v| v.as_str())
        .unwrap_or(if enabled { "ready" } else { "disabled" });
    let has_vault_secret = Vault::open(resolve_vault_path_for_store(store))
        .ok()
        .and_then(|v| v.get_secret(&channel_vault_secret_key(&conn_id)).ok().flatten())
        .is_some_and(|s| !s.trim().is_empty());
    let has_legacy_secret = raw
        .get("webhook_secret")
        .and_then(|v| v.as_str())
        .is_some_and(|s| !s.trim().is_empty());
    let has_webhook_secret = has_vault_secret || has_legacy_secret;

    let endpoint = raw
        .get("endpoint")
        .or_else(|| provider_config.get("endpoint"))
        .or_else(|| provider_config.get("webhook_url"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let updated_at_ms = raw
        .get("updated_at_ms")
        .and_then(|v| v.as_i64())
        .unwrap_or_else(now_ms);

    let mut out = json!({
        "id": conn_id,
        "connection_id": conn_id,
        "kind": kind,
        "name": display_name,
        "display_name": display_name,
        "endpoint": endpoint,
        "account_id": account_id,
        "credential_ref": credential_ref,
        "webhook_secret_ref": webhook_secret_ref,
        "verify_token_ref": verify_token_ref,
        "provider_config": provider_config,
        "enabled": enabled,
        "state": state,
        "has_webhook_secret": has_webhook_secret,
        "updated_at_ms": updated_at_ms
    });
    if let Some(last_rx) = raw.get("last_received_at").and_then(|v| v.as_str()) {
        out["last_received_at"] = json!(last_rx);
    }
    // SEC-7: NEVER expose plaintext `webhook_secret` in API responses!
    out
}

pub(super) fn resolve_channel_webhook_secret(store: &Store, channel: &Value) -> String {
    let conn_id = channel
        .get("connection_id")
        .or_else(|| channel.get("id"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .unwrap_or("");
    if !conn_id.is_empty() {
        if let Ok(vault) = Vault::open(resolve_vault_path_for_store(store)) {
            if let Ok(Some(sec)) = vault.get_secret(&channel_vault_secret_key(conn_id)) {
                if !sec.trim().is_empty() {
                    return sec;
                }
            }
        }
    }
    if let Some(s) = channel
        .get("webhook_secret")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return s.to_string();
    }
    if let Some(ref_name) = channel
        .get("webhook_secret_ref")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        if let Ok(vault) = Vault::open(resolve_vault_path_for_store(store)) {
            if let Ok(Some(sec)) = vault.get_secret(ref_name) {
                if !sec.trim().is_empty() {
                    return sec;
                }
            }
        }
        // SEC-7: Restrict `webhook_secret_ref` env var lookups to `NOLA_WEBHOOK_` prefix!
        if ref_name.starts_with("NOLA_WEBHOOK_") {
            if let Ok(env_val) = std::env::var(ref_name) {
                if !env_val.trim().is_empty() {
                    return env_val;
                }
            }
        }
    }
    std::env::var("NOLA_CHANNEL_WEBHOOK_SECRET").unwrap_or_default()
}

pub(super) fn parse_mcp_server_config(body: &Value, path_id: Option<&str>) -> McpServerConfig {
    let id = path_id
        .map(ToString::to_string)
        .or_else(|| {
            body.get("id")
                .or_else(|| body.get("server_id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
        })
        .unwrap_or_else(new_id);
    let name = body
        .get("name")
        .or_else(|| body.get("display_name"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(&id)
        .to_string();
    let url = body
        .get("url")
        .or_else(|| body.get("endpoint"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(ToString::to_string);
    // Support BOTH string-form `"command": "npx"` AND array-form `"command": ["npx", "-y", "pkg"]`!
    let mut cmd_args_from_array: Vec<String> = Vec::new();
    let command = match body.get("command") {
        Some(Value::String(s)) if !s.trim().is_empty() => Some(s.clone()),
        Some(Value::Array(arr)) => {
            let parts: Vec<String> = arr
                .iter()
                .filter_map(|x| x.as_str().map(ToString::to_string))
                .collect();
            let mut it = parts.into_iter();
            let first = it.next().filter(|s| !s.trim().is_empty());
            cmd_args_from_array = it.collect();
            first
        }
        _ => None,
    };
    let transport_raw = body
        .get("transport")
        .and_then(|v| v.as_str())
        .unwrap_or(if url.is_some() { "streamable_http" } else { "stdio" });
    let transport = match transport_raw.trim().to_ascii_lowercase().as_str() {
        "sse" | "server_sent_events" => McpTransport::ServerSentEvents,
        "http" | "streamable_http" => McpTransport::StreamableHttp,
        _ => McpTransport::Stdio,
    };
    let mut args: Vec<String> = cmd_args_from_array;
    if let Some(a) = body.get("args").and_then(|v| v.as_array()) {
        for x in a {
            if let Some(s) = x.as_str() {
                args.push(s.to_string());
            }
        }
    }
    let env: BTreeMap<String, String> = body
        .get("env")
        .and_then(|v| v.as_object())
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default();
    let headers: BTreeMap<String, String> = body
        .get("headers")
        .and_then(|v| v.as_object())
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default();
    let enabled = body.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true);
    let oauth_client_id = body
        .get("oauth_client_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(ToString::to_string);
    let oauth_scopes: Vec<String> = body
        .get("oauth_scopes")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(ToString::to_string))
                .collect()
        })
        .unwrap_or_default();

    McpServerConfig {
        id,
        name,
        transport,
        enabled,
        command,
        args,
        env,
        url,
        headers,
        oauth_client_id,
        oauth_scopes,
    }
}

/// Decode `att.dataUrl` (`data:<mime>;base64,<b64>`) or `att.data_base64` into a `MediaAttachment`.
pub(super) fn decode_attachment_media(att: &Value) -> Option<MediaAttachment> {
    let filename = att
        .get("name")
        .or_else(|| att.get("filename"))
        .and_then(|v| v.as_str())
        .unwrap_or("attachment")
        .to_string();

    let (mime_type, raw_b64) = if let Some(data_url) = att
        .get("dataUrl")
        .or_else(|| att.get("data_url"))
        .and_then(|v| v.as_str())
    {
        let rest = data_url.strip_prefix("data:")?;
        let (meta, payload) = rest.split_once(',')?;
        let mime = meta
            .split(';')
            .next()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("application/octet-stream");
        if !meta.to_ascii_lowercase().contains(";base64") {
            return None;
        }
        (mime.trim().to_string(), payload.trim().to_string())
    } else {
        let b64 = att
            .get("data_base64")
            .or_else(|| att.get("base64"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())?;
        let mime = att
            .get("mimeType")
            .or_else(|| att.get("mime_type"))
            .and_then(|v| v.as_str())
            .unwrap_or("image/png")
            .to_string();
        (mime, b64.trim().to_string())
    };

    let raw_bytes = base64_decode(&raw_b64).ok()?;
    if raw_bytes.is_empty() {
        return None;
    }
    let digest = sha256_hex(&raw_bytes);
    let normalized_b64 = base64_encode(&raw_bytes);
    Some(MediaAttachment {
        mime_type,
        filename,
        bytes_base64: normalized_b64,
        sha256: digest,
        raw_bytes_len: raw_bytes.len(),
    })
}

pub(super) fn collect_conversation_context_blocks(
    store: &Store,
    suite: &ModulesSuite,
    conversation_id: &str,
    settings: &Value,
) -> Vec<FrozenSystemBlock> {
    let mut extra = Vec::new();

    // 1. Agent Settings custom_system_prompt + local dev tools policy note
    if let Some(custom_prompt) = settings
        .get("custom_system_prompt")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        extra.push(FrozenSystemBlock {
            id: "agent_custom_system_prompt".into(),
            text: custom_prompt.to_string(),
        });
    }
    if settings
        .get("enable_local_dev_tools")
        .and_then(|v| v.as_bool())
        == Some(false)
    {
        extra.push(FrozenSystemBlock {
            id: "agent_local_dev_tools_disabled".into(),
            text: "Local development tools (shell, filesystem edits, git, code execution) are disabled by host settings.".into(),
        });
    }

    // 2. Per-conversation goal (`conv_goal:{id}`)
    if let Ok(Some(goal)) = store.get_kv(&format!("conv_goal:{conversation_id}")) {
        if let Some(obj) = goal.as_object() {
            let title = obj
                .get("title")
                .or_else(|| obj.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            let desc = obj
                .get("description")
                .or_else(|| obj.get("objective"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            let status = obj
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("active");
            if !title.is_empty() || !desc.is_empty() {
                extra.push(FrozenSystemBlock {
                    id: "conversation_goal".into(),
                    text: format!("[Conversation Goal ({status})]\nTitle: {title}\n{desc}"),
                });
            }
        }
    }

    // 3. Per-conversation knowledge (`conv_knowledge:{id}`)
    let knowledge = load_conversation_knowledge(store, conversation_id);
    let summary = knowledge
        .get("summary")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    let key_points = knowledge
        .get("keyPointsMarkdown")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    let wiki = knowledge
        .get("wikiMarkdown")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    let mut k_parts = Vec::new();
    if !summary.is_empty() {
        k_parts.push(format!("Summary:\n{summary}"));
    }
    if !key_points.is_empty() && key_points != summary {
        k_parts.push(format!("Key Points:\n{key_points}"));
    }
    if !wiki.is_empty() {
        k_parts.push(format!("Knowledge Notes:\n{wiki}"));
    }
    if !k_parts.is_empty() {
        extra.push(FrozenSystemBlock {
            id: "conversation_knowledge".into(),
            text: format!("[Conversation Knowledge]\n{}", k_parts.join("\n\n")),
        });
    }

    // 4. Per-conversation data sources (`conv_data_sources:{id}` + `defaultEnabled`, excluding `blocked`)
    let bound_raw = store
        .get_kv(&format!("conv_data_sources:{conversation_id}"))
        .ok()
        .flatten();
    let mut explicit_enabled: HashSet<String> = HashSet::new();
    let mut explicit_disabled: HashSet<String> = HashSet::new();
    if let Some(ref bv) = bound_raw {
        let mut extract_ids = |v: &Value, dst: &mut HashSet<String>| {
            if let Some(arr) = v.as_array() {
                for item in arr {
                    if let Some(s) = item.as_str().filter(|s| !s.trim().is_empty()) {
                        dst.insert(s.trim().to_string());
                    } else if let Some(obj) = item.as_object() {
                        if let Some(sid) = obj
                            .get("id")
                            .or_else(|| obj.get("sourceId"))
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.trim().is_empty())
                        {
                            if obj.get("enabled").and_then(|v| v.as_bool()) == Some(false)
                                || obj.get("blocked").and_then(|v| v.as_bool()) == Some(true)
                            {
                                explicit_disabled.insert(sid.trim().to_string());
                            } else {
                                dst.insert(sid.trim().to_string());
                            }
                        }
                    }
                }
            }
        };
        if bv.is_array() {
            extract_ids(bv, &mut explicit_enabled);
        } else if let Some(obj) = bv.as_object() {
            for key in ["sources", "enabled", "enabledIds", "enabledSourceIds", "sourceIds"] {
                if let Some(v) = obj.get(key) {
                    extract_ids(v, &mut explicit_enabled);
                }
            }
            for key in ["disabled", "disabledIds", "blocked", "blockedIds"] {
                if let Some(v) = obj.get(key) {
                    if let Some(arr) = v.as_array() {
                        for item in arr {
                            if let Some(s) = item.as_str().filter(|s| !s.trim().is_empty()) {
                                explicit_disabled.insert(s.trim().to_string());
                            }
                        }
                    }
                }
            }
        }
    }
    let all_sources: Vec<Value> = suite.list_records("data_source").unwrap_or_default();
    let mut ds_lines = Vec::new();
    for src in &all_sources {
        let sid = src
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        if sid.is_empty() || explicit_disabled.contains(sid) {
            continue;
        }
        let policy_str = src
            .get("accessPolicy")
            .and_then(|v| v.as_str())
            .unwrap_or("conversationOptIn");
        if policy_str.eq_ignore_ascii_case("blocked") {
            continue;
        }
        let include = policy_str.eq_ignore_ascii_case("defaultEnabled")
            || explicit_enabled.contains(sid);
        if !include {
            continue;
        }
        let title = src
            .get("title")
            .or_else(|| src.get("name"))
            .and_then(|v| v.as_str())
            .unwrap_or(sid);
        let kind = src.get("kind").and_then(|v| v.as_str()).unwrap_or("unknown");
        let loc = src.get("location").and_then(|v| v.as_str()).unwrap_or("");
        let sample = src
            .pointer("/validationContract/redactedSample")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let mut line = format!("- {title} (id={sid}, kind={kind}, location={loc})");
        if !sample.is_empty() {
            let preview: String = sample.chars().take(240).collect();
            line.push_str(&format!("\n  Sample: {preview}"));
        }
        ds_lines.push(line);
    }
    if !ds_lines.is_empty() {
        extra.push(FrozenSystemBlock {
            id: "conversation_data_sources".into(),
            text: format!("[Enabled Data Sources]\n{}", ds_lines.join("\n")),
        });
    }

    extra
}

#[allow(clippy::too_many_arguments)]
pub(super) fn handle_sse_chat_turn(
    req: Request,
    store: Store,
    suite: Arc<ModulesSuite>,
    cancels: Arc<Mutex<BTreeMap<String, Arc<AtomicBool>>>>,
    pending: Arc<PendingInteractions>,
    native_browser: Arc<NativeBrowserBridge>,
    mcp_manager: Arc<McpManager>,
    host_id: String,
    conversation_id: String,
    body: Value,
    crypto_tok: Option<String>,
    paired_mobile: bool,
) {
    // P1: Reject concurrent chat turns on the same conversation with HTTP 409 `conversation_busy`!
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut map = cancels.lock().unwrap();
        if map.contains_key(&conversation_id) {
            drop(map);
            let _ = respond_api_json(
                req,
                409,
                &json!({"error": "conversation_busy", "message": "a chat turn is already active for this conversation"}),
                crypto_tok.as_deref(),
            );
            return;
        }
        map.insert(conversation_id.clone(), Arc::clone(&cancel));
    }
    let release_cancel = |cancels: &Arc<Mutex<BTreeMap<String, Arc<AtomicBool>>>>,
                          conv_id: &str,
                          cancel_arc: &Arc<AtomicBool>| {
        if let Ok(mut map) = cancels.lock() {
            if map
                .get(conv_id)
                .is_some_and(|existing| Arc::ptr_eq(existing, cancel_arc))
            {
                map.remove(conv_id);
            }
        }
    };

    let mut raw_prompt = body
        .get("content")
        .or_else(|| body.get("prompt"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if raw_prompt.trim().is_empty() {
        if let Some(msgs) = body.get("messages").and_then(|v| v.as_array()) {
            if let Some(last_user) = msgs
                .iter()
                .rev()
                .find(|m| m.get("role").and_then(|v| v.as_str()) == Some("user"))
            {
                if let Some(s) = last_user.get("content").and_then(|v| v.as_str()) {
                    raw_prompt = s.to_string();
                } else if let Some(parts) = last_user.get("content").and_then(|v| v.as_array()) {
                    let joined: Vec<&str> = parts
                        .iter()
                        .filter_map(|p| {
                            p.get("text")
                                .or_else(|| p.get("content"))
                                .and_then(|v| v.as_str())
                        })
                        .collect();
                    raw_prompt = joined.join("\n");
                }
            }
        }
    }
    let attachments = body
        .get("attachments")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    // Compose effective prompt including text attachments, real media attachments, selected_experts, and ui_action!
    let mut prompt = raw_prompt.clone();
    let mut user_media: Vec<MediaAttachment> = Vec::new();
    for att in &attachments {
        let name = att.get("name").and_then(|v| v.as_str()).unwrap_or("attachment");
        if let Some(media) = decode_attachment_media(att) {
            user_media.push(media);
        } else if let Some(text) = att.get("text").and_then(|v| v.as_str()) {
            prompt.push_str(&format!("\n\n[Attached file: {name}]\n{text}"));
        } else if let Some(mime) = att.get("mimeType").and_then(|v| v.as_str()) {
            prompt.push_str(&format!("\n\n[Attached media: {name} ({mime})]"));
        }
    }

    // Honor `selected_experts` / `selectedExperts` (accepting strings AND `{agent_id}`/`{id}`/`{expert_id}` objects!)
    let selected_expert_ids: Vec<String> = body
        .get("selected_experts")
        .or_else(|| body.get("selectedExperts"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| {
                    v.as_str().or_else(|| {
                        v.get("agent_id")
                            .or_else(|| v.get("agentId"))
                            .or_else(|| v.get("expert_id"))
                            .or_else(|| v.get("id"))
                            .and_then(|s| s.as_str())
                    })
                })
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default();

    // Honor `ui_action` / `uiAction`
    if let Some(ui_act) = body
        .get("ui_action")
        .or_else(|| body.get("uiAction"))
        .filter(|v| !v.is_null())
    {
        if let Ok(act_json) = serde_json::to_string(ui_act) {
            prompt.push_str(&format!("\n\n[UI Action Context]\n{act_json}"));
        }
    }

    let mut cfg = load_active_config(&store);
    let sess = store.get_session(&conversation_id).ok().flatten();
    let conv_meta = store
        .get_kv(&session_meta_key(&conversation_id))
        .ok()
        .flatten()
        .unwrap_or_else(|| json!({}));

    // Respect per-conversation or per-request model override (including `model_intent`,
    // running `selectedModelSource == "local"`, AND user-created remote model UUIDs in KV `web_model_profiles`!)
    let model_intent_val = body.get("model_intent").or_else(|| body.get("modelIntent"));
    let intent_source = model_intent_val.and_then(|mi| {
        mi.get("source")
            .or_else(|| mi.get("connection_id"))
            .or_else(|| mi.get("model_source"))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string)
    });
    let intent_model = model_intent_val.and_then(|mi| {
        mi.as_str()
            .or_else(|| {
                mi.get("profile_id")
                    .or_else(|| mi.get("model_id"))
                    .or_else(|| mi.get("modelId"))
                    .and_then(|v| v.as_str())
            })
            .map(str::trim)
            .filter(|s| !s.is_empty() && *s != "automatic" && *s != "inherit")
            .map(ToString::to_string)
    });
    let explicit_source_field = body
        .get("selectedModelSource")
        .or_else(|| body.get("modelSource"))
        .or_else(|| body.get("model_source"))
        .and_then(|v| v.as_str());
    let override_source = match explicit_source_field {
        // Explicit `""` clears any previous session `local` binding when switching back to cloud.
        Some(raw) if raw.trim().is_empty() => None,
        Some(raw) => Some(raw.trim().to_string()),
        None => intent_source.or_else(|| sess.as_ref().and_then(|s| s.model_connection.clone())),
    };
    let override_model = body
        .get("modelId")
        .or_else(|| body.get("selectedModelId"))
        .or_else(|| body.get("model_id"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .or(intent_model)
        .or_else(|| sess.as_ref().and_then(|s| s.model_id.clone()));
    if override_source.as_deref() == Some("local")
        || override_source
            .as_deref()
            .is_some_and(|s| s.starts_with("local:"))
    {
        let running = LocalModelProcessManager::global().list_running_models();
        let wanted_id = override_source
            .as_deref()
            .and_then(|s| s.strip_prefix("local:"))
            .or(override_model.as_deref());
        let active_local = wanted_id
            .and_then(|wid| {
                running
                    .iter()
                    .find(|r| r.get("id").and_then(|v| v.as_str()) == Some(wid))
            })
            .or_else(|| running.first());
        if let Some(rt) = active_local {
            if let Some(ep) = rt.get("endpoint").and_then(|v| v.as_str()) {
                let local_id = rt
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("local-model")
                    .to_string();
                let conn_id = format!("local:{local_id}");
                let model_name = override_model
                    .clone()
                    .unwrap_or_else(|| local_id.clone());
                let v1_ep = format!("{}/v1", ep.trim_end_matches('/'));
                if let Some(existing) = cfg.llm.connections.iter_mut().find(|c| c.id == conn_id) {
                    existing.endpoint = v1_ep;
                    if !existing.models.iter().any(|m| m.id == model_name) {
                        existing.models.push(ModelEntry {
                            id: model_name.clone(),
                            context_window: None,
                            max_output_tokens: None,
                            temperature: None,
                            top_p: None,
                            reasoning_effort: None,
                            capabilities: None,
                        });
                    }
                } else {
                    cfg.llm.connections.push(ConnectionConfig {
                        id: conn_id.clone(),
                        endpoint: v1_ep,
                        protocol: Some("chat_completions".into()),
                        provider: Some("local".into()),
                        api_key: None,
                        api_key_env: None,
                        models: vec![ModelEntry {
                            id: model_name.clone(),
                            context_window: None,
                            max_output_tokens: None,
                            temperature: None,
                            top_p: None,
                            reasoning_effort: None,
                            capabilities: None,
                        }],
                    });
                }
                cfg.llm.default_model = Some(DefaultModelSelection {
                    connection: conn_id,
                    model: model_name,
                });
            }
        }
    } else if let Some(ref mid) = override_model {
        if let Some((conn_id, model_name)) = resolve_requested_model_selection(&store, &cfg, mid) {
            if let Some(local_id) = conn_id.strip_prefix("local:") {
                if !cfg.llm.connections.iter().any(|c| c.id == conn_id) {
                    let running = LocalModelProcessManager::global().list_running_models();
                    if let Some(ep) = running
                        .iter()
                        .find(|r| r.get("id").and_then(|v| v.as_str()) == Some(local_id))
                        .and_then(|r| r.get("endpoint").and_then(|v| v.as_str()))
                    {
                        cfg.llm.connections.push(ConnectionConfig {
                            id: conn_id.clone(),
                            endpoint: format!("{}/v1", ep.trim_end_matches('/')),
                            protocol: Some("chat_completions".into()),
                            provider: Some("local".into()),
                            api_key: None,
                            api_key_env: None,
                            models: vec![ModelEntry {
                                id: model_name.clone(),
                                context_window: None,
                                max_output_tokens: None,
                                temperature: None,
                                top_p: None,
                                reasoning_effort: None,
                                capabilities: None,
                            }],
                        });
                    }
                }
            }
            cfg.llm.default_model = Some(DefaultModelSelection {
                connection: conn_id,
                model: model_name,
            });
        } else if let Some(ref src) = override_source {
            if cfg.llm.connections.iter().any(|c| c.id == *src) {
                cfg.llm.default_model = Some(DefaultModelSelection {
                    connection: src.clone(),
                    model: mid.clone(),
                });
            }
        } else {
            // Fall back to matching a running local model by ID
            let running = LocalModelProcessManager::global().list_running_models();
            if let Some(rt) = running
                .iter()
                .find(|r| r.get("id").and_then(|v| v.as_str()) == Some(mid.as_str()))
            {
                if let Some(ep) = rt.get("endpoint").and_then(|v| v.as_str()) {
                    let conn_id = format!("local:{mid}");
                    let v1_ep = format!("{}/v1", ep.trim_end_matches('/'));
                    if !cfg.llm.connections.iter().any(|c| c.id == conn_id) {
                        cfg.llm.connections.push(ConnectionConfig {
                            id: conn_id.clone(),
                            endpoint: v1_ep,
                            protocol: Some("chat_completions".into()),
                            provider: Some("local".into()),
                            api_key: None,
                            api_key_env: None,
                            models: vec![ModelEntry {
                                id: mid.clone(),
                                context_window: None,
                                max_output_tokens: None,
                                temperature: None,
                                top_p: None,
                                reasoning_effort: None,
                                capabilities: None,
                            }],
                        });
                    }
                    cfg.llm.default_model = Some(DefaultModelSelection {
                        connection: conn_id,
                        model: mid.clone(),
                    });
                }
            }
        }
    }

    let mut route = match resolve_model_route(&cfg) {
        Ok(r) => r,
        Err(e) => {
            release_cancel(&cancels, &conversation_id, &cancel);
            let _ = respond_api_json(
                req,
                400,
                &json!({"error": e, "code": "model_not_configured"}),
                crypto_tok.as_deref(),
            );
            return;
        }
    };
    let effort_override = body
        .get("reasoningEffort")
        .and_then(|v| v.as_str())
        .or_else(|| conv_meta.get("reasoningEffort").and_then(|v| v.as_str()));
    if let Some(eff) = effort_override.filter(|s| !s.is_empty() && *s != "automatic") {
        route.reasoning_effort = Some(eff.to_string());
    }

    let caller: Arc<dyn ModelCaller> = match HttpModelCaller::new(300) {
        Ok(c) => Arc::new(c),
        Err(e) => {
            release_cancel(&cancels, &conversation_id, &cancel);
            let _ = respond_api_json(
                req,
                500,
                &json!({"error": e.to_string()}),
                crypto_tok.as_deref(),
            );
            return;
        }
    };

    // Persist user message immediately so reload reflects conversation history
    let _ = store.append_event(
        &conversation_id,
        "chat_user",
        json!({"content": &raw_prompt, "attachments": &attachments}),
        50_000,
    );

    // Use per-conversation `sess.workspace_root` (when valid dir) instead of global `suite.workspace_root`!
    let effective_workspace_root = sess
        .as_ref()
        .map(|s| PathBuf::from(&s.workspace_root))
        .filter(|p| p.is_dir())
        .unwrap_or_else(|| suite.workspace_root.clone());

    let settings = build_agent_settings_snapshot(&store, &suite);
    let effective_perm_str = body
        .get("permissionProfile")
        .or_else(|| body.get("permission_profile"))
        .and_then(|v| v.as_str())
        .or_else(|| conv_meta.get("permissionProfile").and_then(|v| v.as_str()))
        .or_else(|| settings.get("permission_profile").and_then(|v| v.as_str()))
        .unwrap_or("cautious");
    // Strict permission default: only "cautious" or "allow_all", defaulting to Cautious!
    let profile = PermissionProfile::parse_strict(effective_perm_str)
        .unwrap_or(PermissionProfile::Cautious);
    let workspace_trusted = profile == PermissionProfile::AllowAll;
    let mut policy = EffectivePolicy::load(
        &effective_workspace_root,
        suite.user_home.as_deref(),
        workspace_trusted,
        Some(profile),
        true,
    );
    let enable_local_dev_tools = settings
        .get("enable_local_dev_tools")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    if !enable_local_dev_tools {
        for tool_pat in [
            "bash",
            "bash_control",
            "pty_exec",
            "code_exec",
            "write_file",
            "edit_file",
            "apply_patch",
            "delete_file",
            "multi_edit_file",
            "move_file",
            "git",
            "notebook_edit",
            "workspace_checkpoint",
            "lsp_diagnostics",
        ] {
            policy.rules.push(PermissionRule {
                scope: RuleScope::CliOverride,
                action: PermissionAction::Deny,
                tool_pattern: tool_pat.into(),
                resource_pattern: None,
            });
        }
    }
    let hooks = HookRunner::load(
        &effective_workspace_root,
        suite.user_home.as_deref(),
        workspace_trusted,
    );

    // Respect user `assistant_memory_default` KV setting (default true) instead of hardcoding!
    let auto_recall_memory = store
        .get_kv("assistant_memory_default")
        .ok()
        .flatten()
        .and_then(|v| {
            v.get("enabled")
                .and_then(|b| b.as_bool())
                .or_else(|| v.as_bool())
        })
        .unwrap_or(true);

    // Real multi-agent `@expert` execution via `RoomRuntime::send_message_with_parallel` when `selectedExperts` is non-empty!
    if !selected_expert_ids.is_empty() {
        let suite_for_room = Arc::clone(&suite);
        let room_ws = effective_workspace_root.clone();
        let conv_id_room = conversation_id.clone();
        let cancels_room = Arc::clone(&cancels);
        let cancel_room = Arc::clone(&cancel);
        let cancel_for_reader = Arc::clone(&cancel);
        let store_room = store.clone();
        let crypto_for_room = crypto_tok;
        let requested_room_id = body
            .get("roomId")
            .or_else(|| body.get("room_id"))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string);
        let (tx_room, rx_room) = mpsc::channel::<Vec<u8>>();
        std::thread::spawn(move || {
            let room_res = dispatch_selected_experts_parallel(
                &suite_for_room,
                Arc::clone(&caller),
                route,
                policy,
                &room_ws,
                &conv_id_room,
                requested_room_id.as_deref(),
                &selected_expert_ids,
                &prompt,
            );
            match room_res {
                Ok((room_id, replies)) => {
                    let mut combined_sections: Vec<String> = Vec::new();
                    for reply in &replies {
                        let expert_name = reply
                            .get("expert_name")
                            .or_else(|| reply.get("name"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("Expert");
                        let content = reply
                            .get("content")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        let expert_frame = json!({
                            "type": "expert_reply",
                            "session_id": conv_id_room,
                            "room_id": room_id,
                            "expert_id": reply.get("expert_id").cloned().unwrap_or(Value::Null),
                            "expert_name": expert_name,
                            "role": reply.get("role").cloned().unwrap_or(Value::Null),
                            "content": content,
                            "reply": reply
                        });
                        let _ = tx_room
                            .send(encode_sse_frame(&expert_frame, crypto_for_room.as_deref()));
                        let section = if replies.len() > 1 {
                            format!("### @{expert_name}\n\n{content}")
                        } else {
                            content.to_string()
                        };
                        if !section.trim().is_empty() {
                            combined_sections.push(section);
                        }
                    }
                    let final_content = combined_sections.join("\n\n");
                    let _ = store_room.append_event(
                        &conv_id_room,
                        "chat_assistant",
                        json!({
                            "content": final_content,
                            "reasoning": "",
                            "toolCalls": [],
                            "uiSurfaces": [],
                            "roomId": room_id,
                            "expertReplies": replies
                        }),
                        50_000,
                    );
                    let completed_frame = json!({
                        "type": "turn_completed",
                        "session_id": conv_id_room,
                        "room_id": room_id,
                        "final_text": final_content,
                        "expert_replies": replies
                    });
                    let _ = tx_room
                        .send(encode_sse_frame(&completed_frame, crypto_for_room.as_deref()));
                }
                Err(e) => {
                    let err_frame = json!({
                        "type": "error",
                        "session_id": conv_id_room,
                        "message": e
                    });
                    let _ = tx_room.send(encode_sse_frame(&err_frame, crypto_for_room.as_deref()));
                }
            }
            let _ = tx_room.send(b"data: [DONE]\n\n".to_vec());
            if let Ok(mut map) = cancels_room.lock() {
                if map
                    .get(&conv_id_room)
                    .is_some_and(|existing| Arc::ptr_eq(existing, &cancel_room))
                {
                    map.remove(&conv_id_room);
                }
            }
        });
        let headers = vec![
            Header::from_bytes(&b"Content-Type"[..], &b"text/event-stream; charset=utf-8"[..])
                .unwrap(),
            Header::from_bytes(&b"Cache-Control"[..], &b"no-cache"[..]).unwrap(),
            Header::from_bytes(&b"X-Accel-Buffering"[..], &b"no"[..]).unwrap(),
        ];
        let resp = Response::new(
            StatusCode(200),
            headers,
            ChannelReader::for_chat(rx_room, cancel_for_reader, paired_mobile),
            None,
            None,
        );
        let _ = req.respond(resp);
        return;
    }

    let (tx, rx) = mpsc::channel::<Vec<u8>>();

    // Live interactive approval callback with narrow session-scoped grant check `(conv_id, tool_name, target_key)`
    let tx_approval = tx.clone();
    let pending_approval = Arc::clone(&pending);
    let cancel_approval = Arc::clone(&cancel);
    let conv_id_for_approval = conversation_id.clone();
    let crypto_for_approval = crypto_tok.clone();
    let approval_store = store.clone();
    let approval_workspace = effective_workspace_root.clone();
    let approval_cb: ApprovalCallback = Arc::new(move |spec, args, reason| {
        let tool_name = spec.schema.name.clone();
        let target_key = approval_target_key(args);
        if pending_approval.has_session_grant(&conv_id_for_approval, &tool_name, &target_key) {
            return true;
        }
        let req_id = new_id();
        let (resp_tx, resp_rx) = mpsc::channel::<bool>();
        pending_approval.approvals.lock().unwrap().insert(
            req_id.clone(),
            PendingApprovalEntry {
                conversation_id: conv_id_for_approval.clone(),
                tool_name: tool_name.clone(),
                target_key: target_key.clone(),
                reason: reason.to_string(),
                arguments: args.clone(),
                tx: resp_tx,
            },
        );
        let frame = json!({
            "type": "permission_requested",
            "request_id": req_id,
            "tool": tool_name,
            "reason": reason,
            "arguments": args
        });
        let _ = tx_approval.send(encode_sse_frame(&frame, crypto_for_approval.as_deref()));
        let deadline = Instant::now() + Duration::from_secs(120);
        let decision = loop {
            if cancel_approval.load(Ordering::SeqCst) {
                break false;
            }
            match resp_rx.recv_timeout(Duration::from_millis(100)) {
                Ok(allow) => break allow,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if Instant::now() >= deadline {
                        break false;
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break false,
            }
        };
        pending_approval.approvals.lock().unwrap().remove(&req_id);
        if decision && pending_approval.has_session_grant(&conv_id_for_approval, &tool_name, &target_key) {
            let _ = nola_engine::policy::persist_session_target_approval(
                &approval_store, &conv_id_for_approval, &approval_workspace, &tool_name, args,
            );
        }
        decision
    });

    // Live interactive user-input callback (`request_user_input`) -> emits SSE `user_question`
    let tx_question = tx.clone();
    let pending_question = Arc::clone(&pending);
    let cancel_question = Arc::clone(&cancel);
    let conv_id_for_question = conversation_id.clone();
    let crypto_for_question = crypto_tok.clone();
    let user_input_cb: UserInputCallback = Arc::new(move |question, options| {
        let req_id = new_id();
        let (resp_tx, resp_rx) = mpsc::channel::<Option<String>>();
        pending_question.questions.lock().unwrap().insert(
            req_id.clone(),
            PendingQuestionEntry {
                conversation_id: conv_id_for_question.clone(),
                question: question.to_string(),
                options: options.to_vec(),
                tx: resp_tx,
            },
        );
        let frame = json!({
            "type": "user_question",
            "request_id": req_id,
            "question": question,
            "options": options
        });
        let _ = tx_question.send(encode_sse_frame(&frame, crypto_for_question.as_deref()));
        let deadline = Instant::now() + Duration::from_secs(120);
        let answer = loop {
            if cancel_question.load(Ordering::SeqCst) {
                break None;
            }
            match resp_rx.recv_timeout(Duration::from_millis(100)) {
                Ok(ans) => break ans,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if Instant::now() >= deadline {
                        break None;
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break None,
            }
        };
        pending_question.questions.lock().unwrap().remove(&req_id);
        answer
    });

    let mut registry = ToolRegistry::with_builtin_tools();
    suite.register_tools(&mut registry);
    // Register enabled MCP server tools safely (a down MCP server never breaks chat).
    for srv in store.list_mcp_servers().unwrap_or_default() {
        if srv.enabled {
            if let Ok(tools) = mcp_manager.discover_tools(&srv) {
                for tool in tools {
                    registry.register(tool);
                }
            }
        }
    }
    // Override A2UI tools with conversation + host scoped authority!
    register_a2ui_tools(&suite, &mut registry, &host_id, &conversation_id);
    // Register any client-declared hybrid device tools (from `deviceTools`, `device_tools`,
    // or `device_bridge.tools[].schema.function`) so model calls emit full SSE `device_tool_request`
    // and suspend until `POST /api/chat/conversations/{id}/device-tool` resolves them!
    let client_id_for_device_tools = body
        .get("client_id")
        .or_else(|| body.get("clientId"))
        .or_else(|| body.pointer("/device_bridge/client_id"))
        .or_else(|| body.pointer("/deviceBridge/clientId"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("nola-client")
        .to_string();
    for (spec, capability) in extract_hybrid_device_tool_specs(&body) {
        registry.register(Arc::new(DeviceToolProxy {
            spec,
            conversation_id: conversation_id.clone(),
            host_id: host_id.clone(),
            client_id: client_id_for_device_tools.clone(),
            capability,
            tx: tx.clone(),
            pending: Arc::clone(&pending),
            cancel: Arc::clone(&cancel),
            crypto_tok: crypto_tok.clone(),
        }));
    }
    let subagents = Arc::new(SubagentManager::new(store.clone(), Arc::clone(&caller)));
    subagents.configure_parent(route.clone(), policy.clone());
    subagents.configure_runtime(hooks.clone(), Some(Arc::clone(&cancel)));
    register_subagent_tools(&subagents, &mut registry);
    let planning = Arc::new(PlanningState::default());
    planning.set_user_input_callback(Some(user_input_cb));
    register_planning_tools(store.clone(), &planning, &mut registry);
    // Drop local Arc<PlanningState> before req.respond() so no tx clone outlives the worker thread!
    drop(planning);

    let engine = AgentEngine::new(store.clone(), Arc::new(registry), caller);
    let mut ctx = ToolContext::new(&effective_workspace_root);
    ctx.permission_profile = policy.profile;
    ctx.sandbox_mode = SandboxMode::from_env();
    // Share the EXACT same `Arc<NativeBrowserBridge>` between HTTP routes and `ToolContext::browser`!
    ctx.browser = Arc::new(BrowserManager::with_native_bridge(native_browser));

    let mut frozen_blocks = build_frozen_system_blocks(&effective_workspace_root, &cfg);
    frozen_blocks.extend(collect_conversation_context_blocks(
        &store,
        &suite,
        &conversation_id,
        &settings,
    ));

    let run_cfg = AgentRunConfig {
        session_id: Some(conversation_id.clone()),
        resume: true,
        route,
        frozen_system_blocks: frozen_blocks,
        host_capabilities: HostCapabilities {
            interactive_user_input: true,
            network_allowed: true,
        },
        policy,
        hooks,
        approval: Some(approval_cb),
        max_steps: 0,
        force_tool_set: ForceToolSet::Auto,
        stream: true,
        json_schema: None,
        cancel: Arc::clone(&cancel),
        auto_recall_memory,
        inbox: None,
    };

    let conv_id_c = conversation_id.clone();
    let cancels_c = Arc::clone(&cancels);
    let cancel_c = Arc::clone(&cancel);
    let cancel_for_reader = Arc::clone(&cancel);
    let store_persist = store.clone();
    let crypto_for_stream = crypto_tok;
    std::thread::spawn(move || {
        let mut collected_text = String::new();
        let mut collected_reasoning = String::new();
        let mut collected_tools: Vec<Value> = Vec::new();
        let mut collected_surfaces: Vec<Value> = Vec::new();
        let run_res = engine.run_with_media(&ctx, &prompt, user_media, run_cfg, |ev| {
            match &ev {
                AgentEvent::TextDelta { delta } => collected_text.push_str(delta),
                AgentEvent::ReasoningDelta { delta } => collected_reasoning.push_str(delta),
                AgentEvent::ToolFinished {
                    call_id,
                    name,
                    is_error,
                    summary,
                    ..
                } => {
                    collected_tools.push(json!({
                        "callId": call_id,
                        "name": name,
                        "ok": !is_error,
                        "summary": summary
                    }));
                    if !is_error && (name == "render_ui_surface" || name == "delete_ui_surface") {
                        let surfaces = list_conversation_surfaces(&store_persist, &conv_id_c);
                        for snap in surfaces {
                            if let Ok(val) = serde_json::to_value(&snap) {
                                let sid = snap.identity.surface_id.clone();
                                collected_surfaces.retain(|existing| {
                                    existing
                                        .pointer("/identity/surface_id")
                                        .and_then(|v| v.as_str())
                                        != Some(sid.as_str())
                                });
                                collected_surfaces.push(val.clone());
                                let ui_frame = json!({ "type": "ui_surface", "snapshot": val });
                                let _ = tx.send(encode_sse_frame(
                                    &ui_frame,
                                    crypto_for_stream.as_deref(),
                                ));
                            }
                        }
                    }
                }
                _ => {}
            }
            if let Ok(ev_val) = serde_json::to_value(&ev) {
                let _ = tx.send(encode_sse_frame(&ev_val, crypto_for_stream.as_deref()));
            }
        });
        match run_res {
            Ok(res) => {
                let final_content = if !res.final_text.trim().is_empty() {
                    res.final_text
                } else {
                    collected_text
                };
                let _ = store_persist.append_event(
                    &conv_id_c,
                    "chat_assistant",
                    json!({
                        "content": final_content,
                        "reasoning": collected_reasoning,
                        "toolCalls": collected_tools,
                        "uiSurfaces": collected_surfaces,
                        "usage": res.usage
                    }),
                    50_000,
                );
            }
            Err(e) => {
                let err_frame = json!({
                    "type": "error",
                    "session_id": conv_id_c,
                    "message": e.to_string()
                });
                let _ = tx.send(encode_sse_frame(&err_frame, crypto_for_stream.as_deref()));
            }
        }
        let _ = tx.send(b"data: [DONE]\n\n".to_vec());
        ctx.pty.close_all();
        if let Ok(mut map) = cancels_c.lock() {
            if map
                .get(&conv_id_c)
                .is_some_and(|existing| Arc::ptr_eq(existing, &cancel_c))
            {
                map.remove(&conv_id_c);
            }
        }
    });

    let headers = vec![
        Header::from_bytes(&b"Content-Type"[..], &b"text/event-stream; charset=utf-8"[..]).unwrap(),
        Header::from_bytes(&b"Cache-Control"[..], &b"no-cache"[..]).unwrap(),
        Header::from_bytes(&b"X-Accel-Buffering"[..], &b"no"[..]).unwrap(),
    ];
    let resp = Response::new(
        StatusCode(200),
        headers,
        ChannelReader::for_chat(rx, cancel_for_reader, paired_mobile),
        None,
        None,
    );
    let _ = req.respond(resp);
}

pub(super) fn parse_attachment_payload(filename: &str, mime_type: &str, bytes: &[u8]) -> Value {
    let (text, extracted) = match extract_document_text_from_bytes(filename, mime_type, bytes) {
        Some(s) => (s.chars().take(200_000).collect::<String>(), true),
        None => (
            format!(
                "[Binary attachment '{filename}' ({mime_type}, {} bytes, sha256={})]",
                bytes.len(),
                &sha256_hex(bytes)[..16]
            ),
            false,
        ),
    };
    json!({
        "ok": true,
        "filename": filename,
        "mime_type": mime_type,
        "bytes": bytes.len(),
        "extracted": extracted,
        "text": text,
        "content": text
    })
}

pub(super) fn execute_documents_search(
    store: &Store,
    workspace_root: &Path,
    raw_query: &str,
    source_ids: Option<&[String]>,
    limit: usize,
) -> Value {
    let q = raw_query.trim();
    let doc_hits: Vec<Value> = store
        .search_doc_chunks(source_ids, q, limit)
        .unwrap_or_default()
        .into_iter()
        .map(|h| {
            let snippet: String = h.content.chars().take(400).collect();
            json!({
                "kind": "document_chunk",
                "id": h.id,
                "sourceId": h.source_id,
                "source_id": h.source_id,
                "path": h.path,
                "chunkIndex": h.chunk_index,
                "content": h.content,
                "snippet": snippet,
                "sha256": h.sha256,
                "score": h.score,
                "updatedAtMs": h.updated_at_ms
            })
        })
        .collect();
    let mem_hits: Vec<Value> = store
        .search_memory(q, None, limit.min(20))
        .unwrap_or_default()
        .into_iter()
        .map(|m| {
            json!({
                "kind": "memory",
                "id": m.id,
                "title": m.title,
                "snippet": m.content
            })
        })
        .collect();
    let mut file_hits: Vec<Value> = Vec::new();
    if !q.is_empty() {
        let q_lower = q.to_ascii_lowercase();
        if let Ok(entries) = fs::read_dir(workspace_root) {
            for entry in entries.flatten().take(100) {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.to_ascii_lowercase().contains(&q_lower) {
                    file_hits.push(json!({"kind": "file", "path": name}));
                }
            }
        }
    }
    json!({
        "ok": true,
        "query": q,
        "count": doc_hits.len(),
        "documents": doc_hits,
        "chunks": doc_hits,
        "hits": doc_hits,
        "memory": mem_hits,
        "files": file_hits
    })
}

pub(super) fn parse_git_worktree_porcelain(raw: &str) -> Vec<Value> {
    let mut out = Vec::new();
    let mut cur_path: Option<String> = None;
    let mut cur_head: Option<String> = None;
    let mut cur_branch: Option<String> = None;
    let mut cur_bare = false;
    let mut cur_detached = false;
    let flush = |out: &mut Vec<Value>,
                 path: &mut Option<String>,
                 head: &mut Option<String>,
                 branch: &mut Option<String>,
                 bare: &mut bool,
                 detached: &mut bool| {
        if let Some(p) = path.take() {
            let is_main = out.is_empty();
            out.push(json!({
                "path": p,
                "head": head.take().unwrap_or_default(),
                "branch": branch.take(),
                "bare": *bare,
                "detached": *detached,
                "isMain": is_main,
                "is_main": is_main
            }));
            *bare = false;
            *detached = false;
        }
    };
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            flush(
                &mut out,
                &mut cur_path,
                &mut cur_head,
                &mut cur_branch,
                &mut cur_bare,
                &mut cur_detached,
            );
        } else if let Some(p) = trimmed.strip_prefix("worktree ") {
            flush(
                &mut out,
                &mut cur_path,
                &mut cur_head,
                &mut cur_branch,
                &mut cur_bare,
                &mut cur_detached,
            );
            cur_path = Some(p.trim().to_string());
        } else if let Some(h) = trimmed.strip_prefix("HEAD ") {
            cur_head = Some(h.trim().to_string());
        } else if let Some(b) = trimmed.strip_prefix("branch ") {
            cur_branch = Some(
                b.trim()
                    .strip_prefix("refs/heads/")
                    .unwrap_or(b.trim())
                    .to_string(),
            );
        } else if trimmed == "bare" {
            cur_bare = true;
        } else if trimmed == "detached" {
            cur_detached = true;
        }
    }
    flush(
        &mut out,
        &mut cur_path,
        &mut cur_head,
        &mut cur_branch,
        &mut cur_bare,
        &mut cur_detached,
    );
    out
}

pub(super) fn handle_workspace_worktrees(workspace_root: &Path, is_post: bool, body: &Value) -> (u16, Value) {
    let raw_action = if !is_post {
        "worktree_list"
    } else {
        body.get("action")
            .or_else(|| body.get("operation"))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("worktree_list")
    };
    let git_action = match raw_action {
        "list" | "worktree_list" => "worktree_list",
        "create" | "add" | "worktree_create" => "worktree_create",
        "remove" | "delete" | "worktree_remove" => "worktree_remove",
        "integrate" | "merge" | "worktree_integrate" => "worktree_integrate",
        other => {
            return (
                400,
                json!({"ok": false, "error": "unsupported_worktree_action", "action": other}),
            );
        }
    };
    let mut args = json!({ "action": git_action });
    let explicit_target = body
        .get("target")
        .or_else(|| body.get("path"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let branch_opt = body
        .get("branch")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let mut generated_target: Option<String> = None;
    if let Some(target) = explicit_target {
        args["target"] = json!(target);
    } else if git_action == "worktree_create" {
        if let Some(branch) = branch_opt {
            let safe_branch: String = branch
                .chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                        c
                    } else {
                        '-'
                    }
                })
                .collect();
            let safe_branch = safe_branch.trim_matches(|c| c == '.' || c == '-');
            let folder = if safe_branch.is_empty() {
                format!("wt-{}", &new_id()[..8])
            } else {
                safe_branch.to_string()
            };
            let wt_parent = workspace_root.join(".nola/worktrees");
            let _ = fs::create_dir_all(&wt_parent);
            let rel_target = format!(".nola/worktrees/{folder}");
            args["target"] = json!(rel_target);
            generated_target = Some(rel_target);
        }
    }
    if let Some(branch) = branch_opt {
        args["branch"] = json!(branch);
    }
    if let Some(strategy) = body.get("strategy").and_then(|v| v.as_str()) {
        args["strategy"] = json!(strategy);
    }
    if git_action == "worktree_remove" {
        if let Some(force) = body.get("force").and_then(|v| v.as_bool()) {
            args["force"] = json!(force);
        }
    }

    let ctx = ToolContext::new(workspace_root);
    let res = GitTool.execute(&ctx, args);
    let model_val = res.payload.to_model_value();
    let summary_text = res.payload.ui_summary().to_string();
    if res.is_error {
        return (
            400,
            json!({
                "ok": false,
                "action": git_action,
                "error": summary_text,
                "detail": model_val
            }),
        );
    }
    if git_action == "worktree_list" {
        let stdout = model_val
            .get("output")
            .or_else(|| model_val.get("stdout"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let worktrees = parse_git_worktree_porcelain(stdout);
        return (
            200,
            json!({
                "ok": true,
                "action": git_action,
                "worktrees": worktrees,
                "output": stdout,
                "summary": summary_text
            }),
        );
    }
    // After create/remove/integrate, also return updated worktree list for UI convenience.
    let list_res = GitTool.execute(&ctx, json!({"action": "worktree_list"}));
    let worktrees = if !list_res.is_error {
        let mv = list_res.payload.to_model_value();
        let stdout = mv
            .get("output")
            .or_else(|| mv.get("stdout"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        parse_git_worktree_porcelain(&stdout)
    } else {
        Vec::new()
    };
    let created_wt = if git_action == "worktree_create" {
        worktrees
            .iter()
            .find(|w| {
                branch_opt.is_some_and(|b| w.get("branch").and_then(|v| v.as_str()) == Some(b))
                    || generated_target.as_deref().is_some_and(|t| {
                        w.get("path")
                            .and_then(|v| v.as_str())
                            .is_some_and(|p| p.ends_with(t))
                    })
            })
            .cloned()
    } else {
        None
    };
    (
        200,
        json!({
            "ok": true,
            "action": git_action,
            "result": model_val,
            "worktree": created_wt,
            "worktrees": worktrees,
            "summary": summary_text
        }),
    )
}

pub(super) fn handle_conversation_goal_control(
    store: &Store,
    suite: &Arc<ModulesSuite>,
    host_id: &str,
    conv_id: &str,
    body: &Value,
) -> (u16, Value) {
    let epoch = ensure_source_store_epoch(store);
    let key = format!("conv_goal:{conv_id}");
    let existing_kv = store.get_kv(&key).ok().flatten();
    let control_obj = body.get("control").filter(|v| v.is_object());
    let raw_action = body
        .get("action")
        .or_else(|| body.get("operation"))
        .or_else(|| control_obj.and_then(|c| c.get("action").or_else(|| c.get("kind"))))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let existing_id = body
        .get("id")
        .or_else(|| body.get("goal_id"))
        .or_else(|| body.get("goalId"))
        .and_then(|v| v.as_str())
        .map(ToString::to_string)
        .or_else(|| {
            existing_kv
                .as_ref()
                .and_then(|g| g.get("id").and_then(|v| v.as_str()).map(ToString::to_string))
        });

    let action = raw_action.unwrap_or_else(|| {
        if existing_id.is_some()
            && body.get("title").is_none()
            && body.get("objective").is_none()
            && body.get("description").is_none()
        {
            "run"
        } else {
            "create"
        }
    });

    let now = now_ms();
    let mut record = existing_id
        .as_deref()
        .and_then(|gid| suite.get_record::<ExtendedGoalRecord>("goal", gid).ok().flatten())
        .unwrap_or_else(|| {
            let gid = existing_id.clone().unwrap_or_else(new_id);
            let title = body
                .get("title")
                .or_else(|| control_obj.and_then(|c| c.get("title")))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .or_else(|| {
                    existing_kv
                        .as_ref()
                        .and_then(|g| g.get("title").and_then(|v| v.as_str()))
                })
                .unwrap_or("Conversation Goal")
                .to_string();
            let objective = body
                .get("objective")
                .or_else(|| body.get("description"))
                .or_else(|| control_obj.and_then(|c| c.get("objective").or_else(|| c.get("goal"))))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .or_else(|| {
                    existing_kv.as_ref().and_then(|g| {
                        g.get("objective")
                            .or_else(|| g.get("description"))
                            .and_then(|v| v.as_str())
                    })
                })
                .unwrap_or(&title)
                .to_string();
            ExtendedGoalRecord {
                id: gid,
                title,
                objective,
                workspace_root: suite.workspace_root.display().to_string(),
                status: GoalStatus::Pending,
                progress_notes: Vec::new(),
                created_at_ms: now,
                updated_at_ms: now,
                lease_until_ms: None,
                retry_count: 0,
                next_retry_at_ms: None,
                needs_user_question: None,
                acceptance_criteria: Vec::new(),
                accepted: false,
            }
        });

    if let Some(t) = body
        .get("title")
        .or_else(|| control_obj.and_then(|c| c.get("title")))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        record.title = t.to_string();
    }
    if let Some(obj) = body
        .get("objective")
        .or_else(|| body.get("description"))
        .or_else(|| control_obj.and_then(|c| c.get("objective").or_else(|| c.get("goal"))))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        record.objective = obj.to_string();
    }
    if let Some(criteria) = body
        .get("acceptance_criteria")
        .or_else(|| body.get("acceptanceCriteria"))
        .and_then(|v| v.as_array())
    {
        record.acceptance_criteria = criteria
            .iter()
            .filter_map(|x| x.as_str().map(str::trim).filter(|s| !s.is_empty()).map(ToString::to_string))
            .collect();
    }

    let mut cancelled = false;
    let mut wake_at_epoch_ms = body
        .get("wakeAtEpochMs")
        .or_else(|| body.get("wake_at_epoch_ms"))
        .or_else(|| {
            control_obj.and_then(|c| c.get("wakeAtEpochMs").or_else(|| c.get("wake_at_epoch_ms")))
        })
        .and_then(|v| v.as_i64())
        .or_else(|| {
            existing_kv
                .as_ref()
                .and_then(|g| g.get("next_wake_at_epoch_ms").and_then(|v| v.as_i64()))
        });
    match action.to_ascii_lowercase().as_str() {
        "create" | "upsert" => {
            record.status = GoalStatus::Pending;
            record.lease_until_ms = None;
            record.next_retry_at_ms = None;
        }
        "run" | "start" => {
            record.status = GoalStatus::Pending;
            record.lease_until_ms = None;
            record.next_retry_at_ms = None;
            record.needs_user_question = None;
        }
        "schedule" => {
            record.status = GoalStatus::Pending;
            record.lease_until_ms = None;
            record.next_retry_at_ms = wake_at_epoch_ms;
            if wake_at_epoch_ms.is_none() {
                wake_at_epoch_ms = Some(now.saturating_add(60_000));
                record.next_retry_at_ms = wake_at_epoch_ms;
            }
            record
                .progress_notes
                .push("Scheduled via goal control".into());
        }
        "pause" => {
            record.status = GoalStatus::Paused;
            record.lease_until_ms = None;
            record.progress_notes.push("Paused via goal control".into());
        }
        "resume" => {
            record.status = GoalStatus::Pending;
            record.lease_until_ms = None;
            record.next_retry_at_ms = None;
            record.needs_user_question = None;
            record.progress_notes.push("Resumed via goal control".into());
        }
        "cancel" | "stop" => {
            record.status = GoalStatus::Failed;
            record.lease_until_ms = None;
            record.progress_notes.push("Cancelled via goal control".into());
            cancelled = true;
        }
        "complete" | "finish" => {
            record.status = GoalStatus::Completed;
            record.lease_until_ms = None;
            record.accepted = true;
            record.progress_notes.push("Completed via goal control".into());
        }
        other => {
            return (
                400,
                json!({
                    "ok": false,
                    "error": "unknown_goal_action",
                    "action": other,
                    "source_host_id": host_id
                }),
            );
        }
    }
    record.updated_at_ms = now;
    let _ = suite.upsert_record("goal", &record.id, &record);

    let control_state = if cancelled {
        "cancelled"
    } else if action.eq_ignore_ascii_case("schedule") {
        "scheduled"
    } else {
        match record.status {
            GoalStatus::Pending => "pending",
            GoalStatus::Running => "running",
            GoalStatus::Paused => "paused",
            GoalStatus::Completed => "completed",
            GoalStatus::Failed => "failed",
        }
    };
    let prev_rev = existing_kv
        .as_ref()
        .and_then(|g| g.get("revision").and_then(|v| v.as_u64()))
        .unwrap_or(0);
    let goal_revision = prev_rev.saturating_add(1);
    let control_token = existing_kv
        .as_ref()
        .and_then(|g| g.get("control_token").and_then(|v| v.as_str()).map(ToString::to_string))
        .unwrap_or_else(new_id);
    let constraints: Vec<String> = body
        .get("constraints")
        .or_else(|| control_obj.and_then(|c| c.get("constraints")))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str().map(ToString::to_string))
                .collect()
        })
        .unwrap_or_else(|| {
            existing_kv
                .as_ref()
                .and_then(|g| g.get("constraints").and_then(|v| v.as_array()))
                .map(|arr| {
                    arr.iter()
                        .filter_map(|x| x.as_str().map(ToString::to_string))
                        .collect()
                })
                .unwrap_or_default()
        });
    let mut goal_json = serde_json::to_value(&record).unwrap_or_else(|_| json!({}));
    if let Some(obj) = goal_json.as_object_mut() {
        obj.insert("conversationId".into(), json!(conv_id));
        obj.insert("conversation_id".into(), json!(conv_id));
        obj.insert("description".into(), json!(record.objective));
        obj.insert("controlState".into(), json!(control_state));
        obj.insert("cancelled".into(), json!(cancelled));
        obj.insert("revision".into(), json!(goal_revision));
        obj.insert("control_token".into(), json!(control_token));
        obj.insert("constraints".into(), json!(constraints));
        obj.insert(
            "next_wake_at_epoch_ms".into(),
            wake_at_epoch_ms.map(|w| json!(w)).unwrap_or(Value::Null),
        );
        obj.insert("updatedAt".into(), json!(ms_to_rfc3339(record.updated_at_ms)));
    }
    let _ = store.set_kv(&key, &goal_json);

    let operation_id = body
        .get("operationId")
        .or_else(|| body.get("operation_id"))
        .or_else(|| {
            control_obj.and_then(|c| c.get("operationId").or_else(|| c.get("operation_id")))
        })
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(new_id);
    let cmd_bytes = serde_json::to_vec(control_obj.unwrap_or(body)).unwrap_or_default();
    let command_sha256 = sha256_hex(&cmd_bytes);
    let ops_key = format!("conv_goal_ops:{conv_id}");
    let mut applied_ops = store
        .get_kv(&ops_key)
        .ok()
        .flatten()
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default();
    applied_ops.insert(
        operation_id.clone(),
        json!({
            "command_sha256": command_sha256,
            "goal_revision": goal_revision,
            "rejection": Value::Null
        }),
    );
    let _ = store.set_kv(&ops_key, &Value::Object(applied_ops));
    let goal_projection = build_goal_projection(store, conv_id, &goal_json);

    let _ = store.append_event(
        conv_id,
        "goal_updated",
        json!({
            "action": action,
            "goal_id": record.id,
            "status": control_state,
            "title": record.title,
            "operation_id": operation_id
        }),
        50_000,
    );

    // If caller asked to "run"/"start", trigger an immediate background GoalWorker turn when a route is configured.
    if matches!(action.to_ascii_lowercase().as_str(), "run" | "start") {
        let cfg = load_active_config(store);
        if let Ok(route) = resolve_model_route(&cfg) {
            if let Ok(caller) = HttpModelCaller::new(300) {
                let suite_bg = Arc::clone(suite);
                let gid_bg = record.id.clone();
                let policy = EffectivePolicy::load(
                    &suite.workspace_root,
                    suite.user_home.as_deref(),
                    false,
                    Some(PermissionProfile::Cautious),
                    true,
                );
                let _ = std::thread::Builder::new()
                    .name(format!("nola-goal-run-{}", &gid_bg[..gid_bg.len().min(8)]))
                    .spawn(move || {
                        let _ = GoalWorker::execute_goal(
                            &suite_bg,
                            Arc::new(caller),
                            route,
                            policy,
                            &gid_bg,
                            24,
                            Arc::new(AtomicBool::new(false)),
                        );
                    });
            }
        }
    }

    (
        200,
        json!({
            "ok": true,
            "source_host_id": host_id,
            "source_store_epoch": epoch,
            "conversationId": conv_id,
            "operationId": operation_id,
            "authoritative": true,
            "controlPending": false,
            "action": action,
            "goal": goal_json,
            "goalProjection": goal_projection
        }),
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn stream_conversation_task_events(
    req: Request,
    store: Store,
    conv_id: String,
    after_cursor: i64,
    oneshot: bool,
    timeout_ms: Option<u64>,
    emit_task_projection: bool,
    crypto_tok: Option<String>,
) {
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_for_reader = Arc::clone(&cancel);
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    std::thread::Builder::new()
        .name(format!("nola-task-sse-{}", &conv_id[..conv_id.len().min(8)]))
        .spawn(move || {
            let mut cursor = after_cursor.max(0);
            let deadline = timeout_ms.map(|ms| Instant::now() + Duration::from_millis(ms));
            // Emit initial snapshot of current goal + todos/tasks so newly attached clients hydrate immediately.
            let goal = store
                .get_kv(&format!("conv_goal:{conv_id}"))
                .ok()
                .flatten()
                .unwrap_or(Value::Null);
            let todos = load_session_todos(&store, &conv_id);
            let tasks: Vec<Value> = todos
                .as_array()
                .into_iter()
                .flatten()
                .enumerate()
                .map(|(idx, t)| {
                    json!({
                        "id": t.get("id").and_then(|v| v.as_str()).map(ToString::to_string).unwrap_or_else(|| format!("task-{idx}")),
                        "title": t.get("content").or_else(|| t.get("title")).and_then(|v| v.as_str()).unwrap_or("Task"),
                        "status": t.get("status").and_then(|v| v.as_str()).unwrap_or("pending")
                    })
                })
                .collect();
            let initial_frame = if emit_task_projection {
                json!({
                    "type": "task_projection",
                    "conversationId": conv_id,
                    "conversation_id": conv_id,
                    "cursor": cursor,
                    "tasks": tasks,
                    "todos": todos,
                    "taskProjection": {
                        "session_id": conv_id,
                        "revision": cursor.max(1),
                        "tasks": tasks
                    }
                })
            } else {
                json!({
                    "type": "task_snapshot",
                    "conversationId": conv_id,
                    "conversation_id": conv_id,
                    "cursor": cursor,
                    "goal": goal,
                    "tasks": tasks,
                    "todos": todos,
                    "plan_mode": is_plan_mode(&store, &conv_id)
                })
            };
            if tx
                .send(encode_sse_frame(&initial_frame, crypto_tok.as_deref()))
                .is_err()
            {
                return;
            }

            let mut ticks_since_ping = 0u32;
            loop {
                if cancel.load(Ordering::SeqCst) {
                    break;
                }
                if let Ok(EventPage::Events(events)) = store.events_after(&conv_id, cursor, 100) {
                    for ev in events {
                        cursor = ev.seq;
                        let is_task_event = ev.kind.starts_with("goal_")
                            || ev.kind.starts_with("subagent_")
                            || ev.kind.starts_with("task_")
                            || ev.kind.starts_with("tool_")
                            || ev.kind.starts_with("plan_")
                            || ev.kind.starts_with("device_tool_");
                        if is_task_event {
                            let frame = if emit_task_projection {
                                let fresh_todos = load_session_todos(&store, &conv_id);
                                let fresh_tasks: Vec<Value> = fresh_todos
                                    .as_array()
                                    .into_iter()
                                    .flatten()
                                    .enumerate()
                                    .map(|(idx, t)| {
                                        json!({
                                            "id": t.get("id").and_then(|v| v.as_str()).map(ToString::to_string).unwrap_or_else(|| format!("task-{idx}")),
                                            "title": t.get("content").or_else(|| t.get("title")).and_then(|v| v.as_str()).unwrap_or("Task"),
                                            "status": t.get("status").and_then(|v| v.as_str()).unwrap_or("pending")
                                        })
                                    })
                                    .collect();
                                json!({
                                    "type": "task_projection",
                                    "conversationId": conv_id,
                                    "conversation_id": conv_id,
                                    "cursor": ev.seq,
                                    "tasks": fresh_tasks,
                                    "taskProjection": {
                                        "session_id": conv_id,
                                        "revision": ev.seq.max(1),
                                        "tasks": fresh_tasks
                                    }
                                })
                            } else {
                                json!({
                                    "type": "task_event",
                                    "id": ev.seq,
                                    "seq": ev.seq,
                                    "cursor": ev.seq,
                                    "kind": ev.kind,
                                    "conversationId": conv_id,
                                    "conversation_id": conv_id,
                                    "created_at_ms": ev.created_at_ms,
                                    "payload": ev.payload
                                })
                            };
                            if tx
                                .send(encode_sse_frame(&frame, crypto_tok.as_deref()))
                                .is_err()
                            {
                                return;
                            }
                        }
                    }
                }
                if oneshot || deadline.is_some_and(|d| Instant::now() >= d) {
                    let _ = tx.send(b"data: [DONE]\n\n".to_vec());
                    break;
                }
                ticks_since_ping += 1;
                if ticks_since_ping >= 40 {
                    ticks_since_ping = 0;
                    if tx.send(b": keepalive\n\n".to_vec()).is_err() {
                        break;
                    }
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        })
        .ok();

    let headers = vec![
        Header::from_bytes(&b"Content-Type"[..], &b"text/event-stream; charset=utf-8"[..]).unwrap(),
        Header::from_bytes(&b"Cache-Control"[..], &b"no-cache"[..]).unwrap(),
        Header::from_bytes(&b"X-Accel-Buffering"[..], &b"no"[..]).unwrap(),
    ];
    let resp = Response::new(
        StatusCode(200),
        headers,
        ChannelReader::with_cancel(rx, cancel_for_reader),
        None,
        None,
    );
    let _ = req.respond(resp);
}

pub(super) fn handle_conversation_recall(store: &Store, conv_id: &str, body: &Value) -> Value {
    let query = body
        .get("query")
        .or_else(|| body.get("q"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .unwrap_or("");
    let limit = body
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(20)
        .clamp(1, 200) as usize;
    let from_seq = body
        .get("from_seq")
        .or_else(|| body.get("fromSeq"))
        .and_then(|v| v.as_i64());

    let events = if let Some(seq) = from_seq {
        store.read_event_range(conv_id, seq, limit).unwrap_or_default()
    } else if !query.is_empty() {
        let scope_opt = if body
            .get("allConversations")
            .or_else(|| body.get("all_conversations"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            None
        } else {
            Some(conv_id)
        };
        store
            .search_events_fts(scope_opt, query, limit)
            .unwrap_or_default()
    } else {
        store.read_event_range(conv_id, 0, limit).unwrap_or_default()
    };

    let projected: Vec<Value> = events
        .into_iter()
        .map(|ev| {
            json!({
                "messageId": format!("msg-{}", ev.seq),
                "message_id": format!("msg-{}", ev.seq),
                "scope": ev.scope,
                "seq": ev.seq,
                "kind": ev.kind,
                "payload": ev.payload,
                "createdAtMs": ev.created_at_ms,
                "created_at_ms": ev.created_at_ms
            })
        })
        .collect();
    let count = projected.len();
    json!({
        "ok": true,
        "conversationId": conv_id,
        "query": query,
        "fromSeq": from_seq,
        "count": count,
        "matches": projected,
        "events": projected
    })
}

pub(super) fn collect_conversation_artifacts(store: &Store, conv_id: &str) -> Vec<Value> {
    let mut artifacts: Vec<Value> = Vec::new();
    for (idx, res) in collect_conversation_resources(store, conv_id)
        .into_iter()
        .enumerate()
    {
        let mut flat = serde_json::Map::new();
        if let Some(obj) = res.as_object() {
            for (k, v) in obj {
                flat.insert(k.clone(), v.clone());
            }
        }
        let id = flat
            .get("id")
            .and_then(|v| v.as_str())
            .map(ToString::to_string)
            .unwrap_or_else(|| format!("res-{idx}"));
        let title = flat
            .get("title")
            .or_else(|| flat.get("name"))
            .or_else(|| flat.get("filename"))
            .or_else(|| flat.get("path"))
            .and_then(|v| v.as_str())
            .map(ToString::to_string)
            .unwrap_or_else(|| id.clone());
        let created_ms = flat
            .get("createdAtMs")
            .or_else(|| flat.get("created_at_ms"))
            .and_then(|v| v.as_i64())
            .unwrap_or_else(now_ms);
        flat.insert("id".into(), json!(id));
        flat.insert("title".into(), json!(title));
        flat.insert("kind".into(), json!("attachment"));
        flat.insert("source".into(), json!("conversation_resource"));
        flat.insert("createdAtMs".into(), json!(created_ms));
        flat.insert("created_at_ms".into(), json!(created_ms));
        artifacts.push(Value::Object(flat));
    }
    for snap in list_conversation_surfaces(store, conv_id) {
        if let Ok(v) = serde_json::to_value(&snap) {
            let title = v
                .pointer("/document/title")
                .or_else(|| v.get("title"))
                .and_then(|x| x.as_str())
                .unwrap_or(&snap.identity.surface_id)
                .to_string();
            let created_ms = now_ms();
            artifacts.push(json!({
                "id": snap.identity.surface_id,
                "title": title,
                "kind": "ui_surface",
                "createdAtMs": created_ms,
                "created_at_ms": created_ms,
                "snapshot": v
            }));
        }
    }
    if let Ok(EventPage::Events(events)) = store.events_after(conv_id, 0, 1000) {
        for ev in events {
            if let Some(arr) = ev.payload.get("output_artifacts").and_then(|v| v.as_array()) {
                for (aidx, art) in arr.iter().enumerate() {
                    let mut flat = serde_json::Map::new();
                    if let Some(obj) = art.as_object() {
                        for (k, v) in obj {
                            flat.insert(k.clone(), v.clone());
                        }
                    } else if let Some(s) = art.as_str() {
                        flat.insert("content".into(), json!(s));
                    }
                    flat.entry("id".to_string())
                        .or_insert_with(|| json!(format!("art-{}-{aidx}", ev.seq)));
                    let default_title = flat
                        .get("title")
                        .or_else(|| flat.get("name"))
                        .or_else(|| flat.get("path"))
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string)
                        .unwrap_or_else(|| format!("Artifact {}", ev.seq));
                    flat.entry("title".to_string())
                        .or_insert_with(|| json!(default_title));
                    flat.entry("kind".to_string())
                        .or_insert_with(|| json!("tool_output_artifact"));
                    flat.insert("seq".into(), json!(ev.seq));
                    flat.insert("messageId".into(), json!(format!("msg-{}", ev.seq)));
                    flat.insert("createdAtMs".into(), json!(ev.created_at_ms));
                    flat.insert("created_at_ms".into(), json!(ev.created_at_ms));
                    artifacts.push(Value::Object(flat));
                }
            }
            if ev.kind == "tool_finished" || ev.kind == "file_modified" {
                if let Some(path) = ev.payload.get("path").and_then(|v| v.as_str()) {
                    artifacts.push(json!({
                        "id": format!("file-{}", ev.seq),
                        "title": path,
                        "kind": "file",
                        "seq": ev.seq,
                        "path": path,
                        "messageId": format!("msg-{}", ev.seq),
                        "createdAtMs": ev.created_at_ms,
                        "created_at_ms": ev.created_at_ms
                    }));
                }
            }
        }
    }
    artifacts
}

pub(super) fn handle_conversation_plan(store: &Store, conv_id: &str, body: &Value) -> (u16, Value) {
    let action = body
        .get("action")
        .or_else(|| body.get("operation"))
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_ascii_lowercase());
    let approve_req = action.as_deref() == Some("approve")
        || body
            .get("approve")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
    if approve_req {
        return match approve_agent_plan(store, conv_id) {
            Ok(plan) => {
                let _ = store.append_event(
                    conv_id,
                    "plan_approved",
                    json!({"plan": plan}),
                    50_000,
                );
                (
                    200,
                    json!({
                        "ok": true,
                        "approved": true,
                        "conversationId": conv_id,
                        "planMode": is_plan_mode(store, conv_id),
                        "plan": plan
                    }),
                )
            }
            Err(e) => (
                404,
                json!({
                    "ok": false,
                    "error": "plan_approval_failed",
                    "message": e,
                    "conversationId": conv_id
                }),
            ),
        };
    }

    if action.as_deref() == Some("reject") {
        let feedback = body
            .get("feedback")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let mut plan_val = store
            .get_kv(&format!("plan:{conv_id}"))
            .ok()
            .flatten()
            .filter(|v| v.is_object())
            .unwrap_or_else(|| json!({}));
        if let Some(obj) = plan_val.as_object_mut() {
            obj.insert("status".into(), json!("rejected"));
            obj.insert("updatedAt".into(), json!(Utc::now().to_rfc3339()));
            if let Some(fb) = feedback {
                obj.insert("feedback".into(), json!(fb));
            }
        }
        let _ = store.set_kv(&format!("plan:{conv_id}"), &plan_val);
        let _ = set_plan_mode(store, conv_id, false);
        let _ = store.append_event(
            conv_id,
            "plan_rejected",
            json!({"plan": plan_val, "feedback": feedback}),
            50_000,
        );
        return (
            200,
            json!({
                "ok": true,
                "rejected": true,
                "conversationId": conv_id,
                "planMode": false,
                "plan": plan_val,
                "todos": load_session_todos(store, conv_id)
            }),
        );
    }

    if action.as_deref() == Some("clear") {
        let _ = store.set_kv(&format!("plan:{conv_id}"), &Value::Null);
        let _ = set_plan_mode(store, conv_id, false);
        let _ = store.append_event(conv_id, "plan_cleared", json!({}), 50_000);
        return (
            200,
            json!({
                "ok": true,
                "cleared": true,
                "conversationId": conv_id,
                "planMode": false,
                "plan": Value::Null,
                "todos": load_session_todos(store, conv_id)
            }),
        );
    }

    let enabled_opt = body
        .get("enabled")
        .or_else(|| body.get("planMode"))
        .or_else(|| body.get("plan_mode"))
        .and_then(|v| v.as_bool())
        .or(match action.as_deref() {
            Some("enter") | Some("enable") | Some("on") => Some(true),
            Some("exit") | Some("disable") | Some("off") => Some(false),
            _ => None,
        });

    if let Some(enabled) = enabled_opt {
        if let Err(e) = set_plan_mode(store, conv_id, enabled) {
            return (500, json!({"ok": false, "error": e.to_string()}));
        }
        let _ = store.append_event(
            conv_id,
            "plan_mode_updated",
            json!({"enabled": enabled}),
            50_000,
        );
    }

    (
        200,
        json!({
            "ok": true,
            "conversationId": conv_id,
            "planMode": is_plan_mode(store, conv_id),
            "plan": load_session_plan(store, conv_id),
            "todos": load_session_todos(store, conv_id)
        }),
    )
}

pub(super) fn handle_auxiliary_dispatch(
    store: &Store,
    suite: &Arc<ModulesSuite>,
    auxiliary_ops: &Mutex<BTreeMap<String, Value>>,
    host_id: &str,
    action: &str,
    body: &Value,
) -> (u16, Value) {
    if let Some(exp) = body
        .get("expected_host_id")
        .or_else(|| body.get("expectedHostId"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
    {
        if exp != host_id {
            return (
                409,
                json!({
                    "ok": false,
                    "error": "host_identity_mismatch",
                    "source_host_id": host_id
                }),
            );
        }
    }
    let epoch = ensure_source_store_epoch(store);
    let op_id = body
        .get("operation_id")
        .or_else(|| body.get("operationId"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(new_id);
    let attempt_id = body
        .get("attempt_id")
        .or_else(|| body.get("attemptId"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| format!("{op_id}:physical"));

    let prev_env = auxiliary_ops
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&op_id)
        .cloned()
        .or_else(|| store.get_kv(&format!("aux_op:{op_id}")).ok().flatten());
    let scenario = body
        .get("scenario")
        .or_else(|| body.pointer("/operation/scenario"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(ToString::to_string)
        .or_else(|| {
            prev_env
                .as_ref()
                .and_then(|e| e.pointer("/receipt/operation/scenario").and_then(|v| v.as_str()))
                .map(ToString::to_string)
        })
        .unwrap_or_else(|| "completion".to_string());
    let input_digest = body
        .get("input_digest")
        .or_else(|| body.pointer("/configuration/input_digest"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(ToString::to_string)
        .or_else(|| {
            body.get("input_json")
                .and_then(|v| v.as_str())
                .map(|s| sha256_hex(s.as_bytes()))
        })
        .or_else(|| {
            prev_env.as_ref().and_then(|e| {
                e.pointer("/receipt/operation/configuration/input_digest")
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string)
            })
        })
        .unwrap_or_else(|| {
            let fallback = body
                .get("prompt")
                .or_else(|| body.get("content"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            sha256_hex(fallback.as_bytes())
        });
    let max_attempts = body
        .get("max_attempts")
        .or_else(|| body.pointer("/configuration/max_attempts"))
        .and_then(|v| v.as_i64())
        .or_else(|| {
            prev_env.as_ref().and_then(|e| {
                e.pointer("/receipt/operation/configuration/max_attempts")
                    .and_then(|v| v.as_i64())
            })
        })
        .unwrap_or(1)
        .max(1);
    let provider_str = body
        .pointer("/configuration/provider")
        .or_else(|| body.get("provider"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(ToString::to_string)
        .or_else(|| {
            body.get("profile_id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(|p| {
                    if p.starts_with("configured:") {
                        p.to_string()
                    } else {
                        format!("configured:{p}")
                    }
                })
        })
        .or_else(|| {
            prev_env.as_ref().and_then(|e| {
                e.pointer("/receipt/operation/configuration/provider")
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string)
            })
        })
        .unwrap_or_else(|| "configured:default".to_string());
    let configuration_revision = body
        .get("expected_profile_revision")
        .or_else(|| body.get("configuration_revision"))
        .or_else(|| body.pointer("/configuration/configuration_revision"))
        .or_else(|| body.get("revision"))
        .and_then(|v| v.as_u64())
        .or_else(|| {
            prev_env.as_ref().and_then(|e| {
                e.pointer("/receipt/operation/configuration/configuration_revision")
                    .and_then(|v| v.as_u64())
            })
        })
        .unwrap_or(1)
        .max(1);
    let maximum_duration_ms = body
        .get("maximum_duration_ms")
        .or_else(|| body.pointer("/configuration/maximum_duration_ms"))
        .and_then(|v| v.as_u64())
        .or_else(|| {
            prev_env.as_ref().and_then(|e| {
                e.pointer("/receipt/operation/configuration/maximum_duration_ms")
                    .and_then(|v| v.as_u64())
            })
        })
        .unwrap_or(300_000)
        .max(1_000);

    let make_receipt = |op_state: &str,
                        attempt_state: Option<&str>,
                        output: Option<&Value>,
                        error: Option<&str>|
     -> Value {
        let attempt_obj = match attempt_state {
            Some(st) => json!({
                "id": attempt_id,
                "operation_id": op_id,
                "state": st,
                "output": output.cloned().unwrap_or(Value::Null),
                "error": error
            }),
            None => Value::Null,
        };
        json!({
            "dispatch_permitted": false,
            "operation": {
                "id": op_id,
                "attempt_id": attempt_id,
                "scenario": scenario,
                "state": op_state,
                "revision": now_ms(),
                "owner": {
                    "kind": "source_authority",
                    "source_host_id": host_id
                },
                "configuration": {
                    "provider": provider_str,
                    "configuration_revision": configuration_revision,
                    "maximum_duration_ms": maximum_duration_ms,
                    "input_digest": input_digest,
                    "max_attempts": max_attempts
                },
                "output": output.cloned().unwrap_or(Value::Null),
                "error": error
            },
            "attempt": attempt_obj
        })
    };

    match action {
        "prepare" => {
            let cfg = load_active_config(store);
            let (conn_id, model_id, profile_id) = cfg
                .llm
                .default_model
                .as_ref()
                .map(|d| {
                    (
                        d.connection.clone(),
                        d.model.clone(),
                        format!("{}:{}", d.connection, d.model),
                    )
                })
                .unwrap_or_else(|| ("default".into(), "unconfigured".into(), "default:unconfigured".into()));
            (
                200,
                json!({
                    "ok": true,
                    "source_host_id": host_id,
                    "source_store_epoch": epoch,
                    "profile_id": profile_id,
                    "connection_id": conn_id,
                    "model_id": model_id,
                    "revision": 1,
                    "ready": model_id != "unconfigured"
                }),
            )
        }
        "admit" => {
            let receipt = make_receipt("ready", None, None, None);
            let envelope = json!({
                "ok": true,
                "source_host_id": host_id,
                "source_store_epoch": epoch,
                "operation_id": op_id,
                "attempt_id": attempt_id,
                "receipt": receipt
            });
            auxiliary_ops
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(op_id.clone(), envelope.clone());
            let _ = store.set_kv(&format!("aux_op:{op_id}"), &envelope);
            (200, envelope)
        }
        "inspect" => match prev_env {
            Some(env) => (200, env),
            None => (
                404,
                json!({
                    "ok": false,
                    "error": "auxiliary_operation_not_found",
                    "source_host_id": host_id,
                    "operation_id": op_id,
                    "attempt_id": attempt_id
                }),
            ),
        },
        "cancel" => {
            let receipt = make_receipt("cancelled", None, None, None);
            let envelope = json!({
                "ok": true,
                "cancelled": true,
                "source_host_id": host_id,
                "source_store_epoch": epoch,
                "operation_id": op_id,
                "attempt_id": attempt_id,
                "receipt": receipt
            });
            auxiliary_ops
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(op_id.clone(), envelope.clone());
            let _ = store.set_kv(&format!("aux_op:{op_id}"), &envelope);
            (200, envelope)
        }
        "invoke" => {
            let cfg = load_active_config(store);
            let route = match resolve_model_route(&cfg) {
                Ok(r) => r,
                Err(e) => {
                    let receipt = make_receipt("failed", Some("failed"), None, Some(&e));
                    let env = json!({
                        "ok": false,
                        "error": e,
                        "source_host_id": host_id,
                        "source_store_epoch": epoch,
                        "operation_id": op_id,
                        "attempt_id": attempt_id,
                        "receipt": receipt
                    });
                    auxiliary_ops
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .insert(op_id.clone(), env.clone());
                    let _ = store.set_kv(&format!("aux_op:{op_id}"), &env);
                    return (400, env);
                }
            };
            let prompt = body
                .get("prompt")
                .or_else(|| body.get("content"))
                .and_then(|v| v.as_str())
                .map(ToString::to_string)
                .or_else(|| {
                    body.get("input_json")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string)
                })
                .or_else(|| body.get("input").and_then(|v| serde_json::to_string(v).ok()))
                .unwrap_or_default();

            let room_id = body
                .get("room_id")
                .or_else(|| body.get("roomId"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let expert_id = body
                .get("expert_id")
                .or_else(|| body.get("expertId"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty());

            let caller: Arc<dyn ModelCaller> = match HttpModelCaller::new(120) {
                Ok(c) => Arc::new(c),
                Err(e) => {
                    let msg = e.to_string();
                    let receipt = make_receipt("failed", Some("failed"), None, Some(&msg));
                    return (
                        500,
                        json!({
                            "ok": false,
                            "error": msg,
                            "source_host_id": host_id,
                            "operation_id": op_id,
                            "attempt_id": attempt_id,
                            "receipt": receipt
                        }),
                    );
                }
            };

            let run_result: Result<Value, String> = if let Some(rid) = room_id {
                let policy = EffectivePolicy::load(
                    &suite.workspace_root,
                    suite.user_home.as_deref(),
                    false,
                    Some(PermissionProfile::Cautious),
                    true,
                );
                RoomRuntime::send_message_with_parallel(
                    suite,
                    caller,
                    route,
                    policy,
                    rid,
                    &prompt,
                    expert_id,
                    true,
                )
                .map(|replies| {
                    let text = replies
                        .iter()
                        .filter_map(|r| r.get("content").and_then(|v| v.as_str()))
                        .collect::<Vec<_>>()
                        .join("\n\n");
                    json!({
                        "text": text,
                        "content": text,
                        "replies": replies,
                        "finish_reason": "stop",
                        "has_tool_calls": false
                    })
                })
            } else {
                let req_body = nola_types::StructuredModelRequest {
                    frozen_system_prefix: vec![FrozenSystemBlock {
                        id: "auxiliary".into(),
                        text: "You are a concise Nola auxiliary completion worker.".into(),
                    }],
                    prefix_stable_tools: Vec::new(),
                    history: vec![nola_types::HistoryMessage::User {
                        content: prompt,
                        media: Vec::new(),
                    }],
                    ephemeral_tail_reminders: Vec::new(),
                    max_output_tokens: 2048,
                    json_schema: None,
                };
                caller
                    .call(&route, &req_body, false, &mut |_| {})
                    .map(|resp| {
                        json!({
                            "text": resp.content,
                            "content": resp.content,
                            "finish_reason": "stop",
                            "has_tool_calls": !resp.tool_calls.is_empty(),
                            "usage": resp.usage
                        })
                    })
                    .map_err(|e| e.to_string())
            };

            match run_result {
                Ok(output) => {
                    let receipt = make_receipt("succeeded", Some("succeeded"), Some(&output), None);
                    let env = json!({
                        "ok": true,
                        "source_host_id": host_id,
                        "source_store_epoch": epoch,
                        "operation_id": op_id,
                        "attempt_id": attempt_id,
                        "receipt": receipt,
                        "output": output,
                        "finish_reason": output.get("finish_reason").cloned().unwrap_or_else(|| json!("stop")),
                        "has_tool_calls": output.get("has_tool_calls").cloned().unwrap_or(Value::Bool(false))
                    });
                    auxiliary_ops
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .insert(op_id.clone(), env.clone());
                    let _ = store.set_kv(&format!("aux_op:{op_id}"), &env);
                    (200, env)
                }
                Err(e) => {
                    let receipt = make_receipt("failed", Some("failed"), None, Some(&e));
                    let env = json!({
                        "ok": false,
                        "error": e,
                        "source_host_id": host_id,
                        "source_store_epoch": epoch,
                        "operation_id": op_id,
                        "attempt_id": attempt_id,
                        "receipt": receipt
                    });
                    auxiliary_ops
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .insert(op_id.clone(), env.clone());
                    let _ = store.set_kv(&format!("aux_op:{op_id}"), &env);
                    (502, env)
                }
            }
        }
        other => (
            400,
            json!({"ok": false, "error": "unknown_auxiliary_action", "action": other}),
        ),
    }
}

pub(super) fn validate_oauth_origin(raw: &str) -> Result<String, &'static str> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("invalid_origin");
    }
    let parsed = Url::parse(trimmed).map_err(|_| "invalid_origin")?;
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("invalid_origin");
    }
    match parsed.scheme() {
        "https" => {}
        "http" if is_loopback_url(trimmed) => {}
        _ => return Err("invalid_origin"),
    }
    if parsed.host_str().is_none() {
        return Err("invalid_origin");
    }
    Ok(trimmed.trim_end_matches('/').to_string())
}

pub(super) fn handle_source_oauth_route(
    store: &Store,
    suite: &Arc<ModulesSuite>,
    oauth_flows: &Mutex<BTreeMap<String, OAuthFlowState>>,
    path: &str,
    query: &str,
    body: &Value,
) -> (u16, Value) {
    let vault = Vault::open(resolve_vault_path_for_store(store)).ok();
    match path {
        "/api/model-connections/kimi-login/owners" => {
            let owners: Vec<Value> = suite
                .list_records::<Value>("source_account")
                .unwrap_or_default()
                .into_iter()
                .filter(|r| r.get("source").and_then(|v| v.as_str()) == Some("kimi"))
                .map(|r| {
                    let id = r.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    json!({
                        "account_id": id,
                        "revision": r.get("revision").and_then(|v| v.as_i64()).unwrap_or(1),
                        "display_name": r.get("display_name").and_then(|v| v.as_str()).unwrap_or("Kimi Code"),
                        "origin": r.get("origin").and_then(|v| v.as_str()).unwrap_or(""),
                        "server_id": r.get("server_id").and_then(|v| v.as_str()).unwrap_or("kimi-local")
                    })
                })
                .collect();
            (200, Value::Array(owners))
        }
        "/api/model-connections/opencode-login/owners" => {
            let owners: Vec<Value> = suite
                .list_records::<Value>("source_account")
                .unwrap_or_default()
                .into_iter()
                .filter(|r| r.get("source").and_then(|v| v.as_str()) == Some("opencode"))
                .map(|r| {
                    let id = r.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    json!({
                        "account_id": id,
                        "revision": r.get("revision").and_then(|v| v.as_i64()).unwrap_or(1),
                        "display_name": r.get("display_name").and_then(|v| v.as_str()).unwrap_or("OpenCode"),
                        "origin": r.get("origin").and_then(|v| v.as_str()).unwrap_or(""),
                        "directory": r.get("directory").and_then(|v| v.as_str()).unwrap_or("")
                    })
                })
                .collect();
            (200, Value::Array(owners))
        }
        "/api/model-connections/kimi-login/connect" => {
            let account_id = body
                .get("account_id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(new_id);
            let raw_origin = body
                .get("origin")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            let bearer = body
                .get("server_bearer")
                .or_else(|| body.get("api_key"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            if raw_origin.is_empty() || bearer.is_empty() {
                return (
                    400,
                    json!({"ok": false, "error": "origin_and_bearer_required"}),
                );
            }
            let Ok(origin) = validate_oauth_origin(raw_origin) else {
                return (
                    400,
                    json!({"ok": false, "error": "invalid_origin", "message": "origin must be https:// or loopback http://"}),
                );
            };
            if let Some(ref v) = vault {
                let _ = v.set_secret(&format!("kimi_bearer:{account_id}"), &bearer);
            }
            let prev_rev = suite
                .get_record::<Value>("source_account", &account_id)
                .ok()
                .flatten()
                .and_then(|r| r.get("revision").and_then(|v| v.as_i64()))
                .unwrap_or(0);
            let rev = prev_rev + 1;
            let server_id = format!("kimi-{}", &sha256_hex(origin.as_bytes())[..8]);
            let display_name = body
                .get("display_name")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .unwrap_or("Kimi Code");
            let rec = json!({
                "id": account_id,
                "source": "kimi",
                "display_name": display_name,
                "origin": origin,
                "server_id": server_id,
                "revision": rev,
                "state": "idle",
                "updated_at_ms": now_ms()
            });
            let _ = suite.upsert_record("source_account", &account_id, &rec);
            (
                200,
                json!({
                    "account": { "revision": rev, "account": { "id": account_id } },
                    "origin": origin,
                    "server_id": server_id,
                    "state": "idle",
                    "flow_id": Value::Null,
                    "verification_url": Value::Null,
                    "user_code": Value::Null,
                    "poll_interval_seconds": Value::Null,
                    "inference_supported": false,
                    "cancellation_confirmed": Value::Null
                }),
            )
        }
        "/api/model-connections/kimi-login/start"
        | "/api/model-connections/kimi-login/status"
        | "/api/model-connections/kimi-login/cancel" => {
            let account_id = body
                .get("account_id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .or_else(|| parse_query_str(query, "account_id"))
                .unwrap_or_else(new_id);
            let saved = suite
                .get_record::<Value>("source_account", &account_id)
                .ok()
                .flatten()
                .unwrap_or_else(|| json!({"id": account_id, "revision": 1, "origin": "", "server_id": "kimi-local"}));
            let rev = saved.get("revision").and_then(|v| v.as_i64()).unwrap_or(1);
            let origin = saved
                .get("origin")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let server_id = saved
                .get("server_id")
                .and_then(|v| v.as_str())
                .unwrap_or("kimi-local")
                .to_string();
            let flow_id_in = body
                .get("flow_id")
                .or_else(|| body.get("request_id"))
                .and_then(|v| v.as_str())
                .map(ToString::to_string);
            let mut flows = oauth_flows.lock().unwrap_or_else(|e| e.into_inner());
            if path.ends_with("/start") {
                let flow_id = new_id();
                let (_verifier, challenge) = generate_pkce_pair();
                let csrf_state = generate_hex_token_32();
                let user_code = format!(
                    "{}-{}",
                    csrf_state[..4].to_ascii_uppercase(),
                    csrf_state[4..8].to_ascii_uppercase()
                );
                let base_origin = if origin.is_empty() {
                    "https://kimi.moonshot.cn".to_string()
                } else {
                    origin.trim_end_matches('/').to_string()
                };
                let verification_url = format!(
                    "{base_origin}/oauth/device?user_code={user_code}&code_challenge={challenge}&code_challenge_method=S256&state={csrf_state}"
                );
                let flow = OAuthFlowState {
                    request_id: flow_id.clone(),
                    account_id: account_id.clone(),
                    provider: "kimi".into(),
                    method: 0,
                    state: "pending".into(),
                    csrf_state,
                    code_challenge: challenge,
                    authorization_url: Some(verification_url.clone()),
                    verification_url: Some(verification_url.clone()),
                    user_code: Some(user_code.clone()),
                    callback_mode: Some("device_code".into()),
                    instructions: Some("Open verification URL and enter the user code.".into()),
                    poll_interval_seconds: Some(5),
                    cancellation_confirmed: None,
                    expires_at_ms: now_ms().saturating_add(600_000),
                };
                flows.insert(flow_id.clone(), flow);
                (
                    200,
                    json!({
                        "account": { "revision": rev, "account": { "id": account_id } },
                        "origin": origin,
                        "server_id": server_id,
                        "state": "pending",
                        "flow_id": flow_id,
                        "verification_url": verification_url,
                        "user_code": user_code,
                        "poll_interval_seconds": 5,
                        "inference_supported": false,
                        "cancellation_confirmed": Value::Null
                    }),
                )
            } else if path.ends_with("/cancel") {
                let fid = flow_id_in.unwrap_or_default();
                let mut cancelled_fid = fid.clone();
                let confirmed = if !fid.is_empty() {
                    if let Some(flow) = flows.get_mut(&fid) {
                        flow.state = "cancelled".into();
                        flow.verification_url = None;
                        flow.user_code = None;
                        flow.poll_interval_seconds = None;
                        flow.cancellation_confirmed = Some(true);
                    }
                    true
                } else if let Some((k, flow)) = flows
                    .iter_mut()
                    .rev()
                    .find(|(_, f)| f.account_id == account_id && f.state == "pending")
                {
                    cancelled_fid = k.clone();
                    flow.state = "cancelled".into();
                    flow.verification_url = None;
                    flow.user_code = None;
                    flow.poll_interval_seconds = None;
                    flow.cancellation_confirmed = Some(true);
                    true
                } else {
                    true
                };
                (
                    200,
                    json!({
                        "account": { "revision": rev, "account": { "id": account_id } },
                        "origin": origin,
                        "server_id": server_id,
                        "state": "cancelled",
                        "flow_id": if cancelled_fid.is_empty() { Value::Null } else { json!(cancelled_fid) },
                        "verification_url": Value::Null,
                        "user_code": Value::Null,
                        "poll_interval_seconds": Value::Null,
                        "inference_supported": false,
                        "cancellation_confirmed": confirmed
                    }),
                )
            } else {
                let target_key = flow_id_in
                    .as_deref()
                    .filter(|id| flows.contains_key(*id))
                    .map(ToString::to_string)
                    .or_else(|| {
                        flows
                            .iter()
                            .rev()
                            .find(|(_, f)| f.account_id == account_id)
                            .map(|(k, _)| k.clone())
                    });
                match target_key.and_then(|k| flows.get_mut(&k)) {
                    Some(f) => {
                        if f.state == "pending" && now_ms() >= f.expires_at_ms {
                            f.state = "expired".into();
                            f.verification_url = None;
                            f.user_code = None;
                            f.poll_interval_seconds = None;
                        }
                        (
                            200,
                            json!({
                                "account": { "revision": rev, "account": { "id": account_id } },
                                "origin": origin,
                                "server_id": server_id,
                                "state": f.state,
                                "flow_id": f.request_id,
                                "verification_url": f.verification_url,
                                "user_code": f.user_code,
                                "poll_interval_seconds": f.poll_interval_seconds,
                                "inference_supported": false,
                                "cancellation_confirmed": f.cancellation_confirmed
                            }),
                        )
                    }
                    None => (
                        200,
                        json!({
                            "account": { "revision": rev, "account": { "id": account_id } },
                            "origin": origin,
                            "server_id": server_id,
                            "state": "idle",
                            "flow_id": Value::Null,
                            "verification_url": Value::Null,
                            "user_code": Value::Null,
                            "poll_interval_seconds": Value::Null,
                            "inference_supported": false,
                            "cancellation_confirmed": Value::Null
                        }),
                    ),
                }
            }
        }
        "/api/model-connections/opencode-login/connect"
        | "/api/model-connections/opencode-login/inspect" => {
            let account_id = body
                .get("account_id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .or_else(|| parse_query_str(query, "account_id"))
                .unwrap_or_else(new_id);
            let existing = suite
                .get_record::<Value>("source_account", &account_id)
                .ok()
                .flatten();
            let (origin, directory, rev) = if path.ends_with("/connect") {
                let raw_origin = body
                    .get("origin")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim();
                let directory = body
                    .get("directory")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                let password = body
                    .get("password")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim();
                if raw_origin.is_empty() {
                    return (400, json!({"ok": false, "error": "origin_required"}));
                }
                let Ok(origin) = validate_oauth_origin(raw_origin) else {
                    return (
                        400,
                        json!({"ok": false, "error": "invalid_origin", "message": "origin must be https:// or loopback http://"}),
                    );
                };
                if !password.is_empty() {
                    if let Some(ref v) = vault {
                        let _ = v.set_secret(&format!("opencode_password:{account_id}"), password);
                    }
                }
                let rev = existing
                    .as_ref()
                    .and_then(|r| r.get("revision").and_then(|v| v.as_i64()))
                    .unwrap_or(0)
                    + 1;
                let rec = json!({
                    "id": account_id,
                    "source": "opencode",
                    "display_name": body.get("display_name").and_then(|v| v.as_str()).unwrap_or("OpenCode"),
                    "origin": origin,
                    "directory": directory,
                    "username": body.get("username").and_then(|v| v.as_str()).unwrap_or("opencode"),
                    "revision": rev,
                    "state": "idle",
                    "updated_at_ms": now_ms()
                });
                let _ = suite.upsert_record("source_account", &account_id, &rec);
                (origin, directory, rev)
            } else {
                let Some(ex) = existing else {
                    return (404, json!({"ok": false, "error": "opencode_owner_not_found"}));
                };
                (
                    ex.get("origin").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    ex.get("directory").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    ex.get("revision").and_then(|v| v.as_i64()).unwrap_or(1),
                )
            };
            let methods = json!([
                {
                    "provider": "opencode",
                    "method": 0,
                    "binding": "pkce-oauth",
                    "declaration": {
                        "type": "oauth",
                        "label": "PKCE OAuth Authorization Code",
                        "prompts": []
                    }
                }
            ]);
            (
                200,
                json!({
                    "account": { "revision": rev, "account": { "id": account_id } },
                    "origin": origin,
                    "directory": directory,
                    "state": "idle",
                    "methods": methods
                }),
            )
        }
        "/api/model-connections/opencode-login/authorize" => {
            let account_id = body
                .get("account_id")
                .and_then(|v| v.as_str())
                .unwrap_or("opencode")
                .to_string();
            let request_id = body
                .get("request_id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(new_id);
            let provider = body
                .get("provider")
                .and_then(|v| v.as_str())
                .unwrap_or("opencode")
                .to_string();
            let method_idx = body.get("method").and_then(|v| v.as_i64()).unwrap_or(0);
            let (_verifier, challenge) = generate_pkce_pair();
            let csrf_state = generate_hex_token_32();
            let origin = suite
                .get_record::<Value>("source_account", &account_id)
                .ok()
                .flatten()
                .and_then(|r| r.get("origin").and_then(|v| v.as_str()).map(ToString::to_string))
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "http://127.0.0.1:4096".to_string());
            let auth_url = format!(
                "{}/oauth/authorize?response_type=code&code_challenge={challenge}&code_challenge_method=S256&state={csrf_state}",
                origin.trim_end_matches('/')
            );
            let flow = OAuthFlowState {
                request_id: request_id.clone(),
                account_id,
                provider: provider.clone(),
                method: method_idx,
                state: "pending".into(),
                csrf_state: csrf_state.clone(),
                code_challenge: challenge,
                authorization_url: Some(auth_url.clone()),
                verification_url: None,
                user_code: None,
                callback_mode: Some("code".into()),
                instructions: Some("Complete authorization and paste the returned code.".into()),
                poll_interval_seconds: None,
                cancellation_confirmed: None,
                expires_at_ms: now_ms().saturating_add(600_000),
            };
            oauth_flows
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(request_id.clone(), flow);
            (
                200,
                json!({
                    "request_id": request_id,
                    "provider": provider,
                    "method": method_idx,
                    "state": "pending",
                    "oauth_state": csrf_state,
                    "state_token": csrf_state,
                    "authorization_url": auth_url,
                    "callback_mode": "code",
                    "instructions": "Complete authorization and paste the returned code."
                }),
            )
        }
        "/api/model-connections/opencode-login/callback" => {
            let request_id = body
                .get("request_id")
                .and_then(|v| v.as_str())
                .map(ToString::to_string)
                .or_else(|| parse_query_str(query, "request_id"))
                .unwrap_or_default();
            let provided_state = body
                .get("state")
                .or_else(|| body.get("oauth_state"))
                .or_else(|| body.get("state_token"))
                .and_then(|v| v.as_str())
                .map(ToString::to_string)
                .or_else(|| {
                    parse_query_str(query, "state")
                        .or_else(|| parse_query_str(query, "oauth_state"))
                        .or_else(|| parse_query_str(query, "state_token"))
                })
                .unwrap_or_default();
            let code = body
                .get("code")
                .and_then(|v| v.as_str())
                .map(ToString::to_string)
                .or_else(|| parse_query_str(query, "code"))
                .unwrap_or_default();
            if provided_state.trim().is_empty() {
                return (
                    400,
                    json!({"ok": false, "error": "invalid_oauth_state", "message": "full OAuth CSRF state is required"}),
                );
            }
            let mut flows = oauth_flows.lock().unwrap_or_else(|e| e.into_inner());
            let matched_key = if !request_id.is_empty() && flows.contains_key(&request_id) {
                Some(request_id.clone())
            } else {
                flows
                    .iter()
                    .find(|(_, f)| ct_eq(&f.csrf_state, provided_state.trim()))
                    .map(|(k, _)| k.clone())
            };
            let Some(flow_key) = matched_key else {
                return (404, json!({"ok": false, "error": "oauth_flow_not_found"}));
            };
            let Some(flow) = flows.get_mut(&flow_key) else {
                return (404, json!({"ok": false, "error": "oauth_flow_not_found"}));
            };
            // Always require constant-time full-state verification even when request_id is supplied!
            if !ct_eq(&flow.csrf_state, provided_state.trim()) {
                return (
                    400,
                    json!({"ok": false, "error": "invalid_oauth_state", "message": "OAuth CSRF state verification failed"}),
                );
            }
            if code.trim().is_empty() {
                return (400, json!({"ok": false, "error": "authorization_code_required"}));
            }
            if let Some(ref v) = vault {
                let _ = v.set_secret(&format!("source_account_token:{}", flow.account_id), code.trim());
            }
            flow.state = "source_callback_completed".into();
            flow.authorization_url = None;
            flow.instructions = None;
            (
                200,
                json!({
                    "request_id": flow.request_id,
                    "provider": flow.provider,
                    "method": flow.method,
                    "state": flow.state,
                    "authorization_url": Value::Null,
                    "callback_mode": flow.callback_mode,
                    "instructions": Value::Null
                }),
            )
        }
        "/api/model-connections/opencode-login/forget" => {
            let request_id = body
                .get("request_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let removed = oauth_flows
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&request_id);
            let provider = removed
                .as_ref()
                .map(|f| f.provider.as_str())
                .unwrap_or("opencode");
            let method_idx = removed.as_ref().map(|f| f.method).unwrap_or(0);
            (
                200,
                json!({
                    "ok": true,
                    "forgotten": true,
                    "request_id": request_id,
                    "provider": provider,
                    "method": method_idx,
                    "state": "idle",
                    "authorization_url": Value::Null,
                    "callback_mode": Value::Null,
                    "instructions": Value::Null
                }),
            )
        }
        "/api/model-connections/source-login/start" => {
            let request_id = body
                .get("request_id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(new_id);
            let account_id = body
                .get("account_id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(new_id);
            let display_name = body
                .get("display_name")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .unwrap_or("Codex");
            let source_kind = body
                .get("source")
                .and_then(|v| v.as_str())
                .unwrap_or("codex");
            let (_verifier, challenge) = generate_pkce_pair();
            let csrf_state = generate_hex_token_32();
            let auth_url = format!(
                "https://auth.openai.com/oauth/authorize?response_type=code&code_challenge={challenge}&code_challenge_method=S256&state={csrf_state}"
            );
            let prev_rev = suite
                .get_record::<Value>("source_account", &account_id)
                .ok()
                .flatten()
                .and_then(|r| r.get("revision").and_then(|v| v.as_i64()))
                .unwrap_or(0);
            let rev = prev_rev + 1;
            let account_rec = json!({
                "id": account_id,
                "source": source_kind,
                "display_name": display_name,
                "revision": rev,
                "state": "pending",
                "updated_at_ms": now_ms()
            });
            let _ = suite.upsert_record("source_account", &account_id, &account_rec);
            let flow = OAuthFlowState {
                request_id: request_id.clone(),
                account_id: account_id.clone(),
                provider: source_kind.into(),
                method: 0,
                state: "pending".into(),
                csrf_state,
                code_challenge: challenge.clone(),
                authorization_url: Some(auth_url.clone()),
                verification_url: None,
                user_code: None,
                callback_mode: Some("loopback".into()),
                instructions: Some("Complete PKCE authorization in browser.".into()),
                poll_interval_seconds: Some(2),
                cancellation_confirmed: None,
                expires_at_ms: now_ms().saturating_add(600_000),
            };
            oauth_flows
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(request_id.clone(), flow);
            (
                200,
                json!({
                    "request_id": request_id,
                    "state": "pending",
                    "authorization_url": auth_url,
                    "pkce_challenge": challenge,
                    "pkce_method": "S256",
                    "inference_supported": false,
                    "account": {
                        "revision": rev,
                        "account": {
                            "id": account_id,
                            "display_name": display_name,
                            "source": source_kind
                        }
                    }
                }),
            )
        }
        "/api/model-connections/source-login/status" => {
            let request_id = body
                .get("request_id")
                .and_then(|v| v.as_str())
                .map(ToString::to_string)
                .or_else(|| parse_query_str(query, "request_id"))
                .unwrap_or_default();
            let account_id = body
                .get("account_id")
                .and_then(|v| v.as_str())
                .map(ToString::to_string)
                .or_else(|| parse_query_str(query, "account_id"))
                .unwrap_or_default();
            let flows = oauth_flows.lock().unwrap_or_else(|e| e.into_inner());
            let flow = flows.get(&request_id).or_else(|| {
                (!account_id.is_empty())
                    .then(|| flows.values().rev().find(|f| f.account_id == account_id))
                    .flatten()
            });
            let eff_account_id = flow
                .map(|f| f.account_id.clone())
                .unwrap_or(account_id);
            let acct_val = suite
                .get_record::<Value>("source_account", &eff_account_id)
                .ok()
                .flatten()
                .map(|r| {
                    json!({
                        "revision": r.get("revision").and_then(|v| v.as_i64()).unwrap_or(1),
                        "account": {
                            "id": eff_account_id,
                            "display_name": r.get("display_name").and_then(|v| v.as_str()).unwrap_or("Account"),
                            "source": r.get("source").and_then(|v| v.as_str()).unwrap_or("codex")
                        }
                    })
                });
            match flow {
                Some(f) => (
                    200,
                    json!({
                        "request_id": f.request_id,
                        "state": f.state,
                        "authorization_url": if f.state == "pending" {
                            f.authorization_url.as_ref().map(|u| json!(u)).unwrap_or(Value::Null)
                        } else {
                            Value::Null
                        },
                        "pkce_challenge": f.code_challenge,
                        "inference_supported": false,
                        "account": acct_val
                    }),
                ),
                None => (
                    200,
                    json!({
                        "request_id": request_id,
                        "state": if acct_val.is_some() { "authenticated_source_only" } else { "starting" },
                        "authorization_url": Value::Null,
                        "inference_supported": false,
                        "account": acct_val
                    }),
                ),
            }
        }
        "/api/model-connections/source-login/cancel" => {
            let request_id = body
                .get("request_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let mut flows = oauth_flows.lock().unwrap_or_else(|e| e.into_inner());
            let acct_id = if let Some(f) = flows.get_mut(&request_id) {
                f.state = "cancelled".into();
                f.authorization_url = None;
                f.cancellation_confirmed = Some(true);
                f.account_id.clone()
            } else {
                body.get("account_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string()
            };
            let acct_val = suite
                .get_record::<Value>("source_account", &acct_id)
                .ok()
                .flatten()
                .map(|r| {
                    json!({
                        "revision": r.get("revision").and_then(|v| v.as_i64()).unwrap_or(1),
                        "account": {
                            "id": acct_id,
                            "display_name": r.get("display_name").and_then(|v| v.as_str()).unwrap_or("Account"),
                            "source": r.get("source").and_then(|v| v.as_str()).unwrap_or("codex")
                        }
                    })
                });
            (
                200,
                json!({
                    "request_id": request_id,
                    "state": "cancelled",
                    "cancelled": true,
                    "authorization_url": Value::Null,
                    "inference_supported": false,
                    "account": acct_val
                }),
            )
        }
        "/api/model-connections/source-accounts/bind" => {
            let account_id = body
                .get("account_id")
                .or_else(|| body.get("id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(new_id);
            let token = body
                .get("token")
                .or_else(|| body.get("access_token"))
                .or_else(|| body.get("api_key"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty());
            if let (Some(tok), Some(v)) = (token, vault.as_ref()) {
                let _ = v.set_secret(&format!("source_account_token:{account_id}"), tok);
            }
            let prev = suite
                .get_record::<Value>("source_account", &account_id)
                .ok()
                .flatten()
                .unwrap_or_else(|| json!({"id": account_id, "source": "codex", "display_name": account_id, "path": ""}));
            let rev = prev.get("revision").and_then(|v| v.as_i64()).unwrap_or(0) + 1;
            let display_name = body
                .get("display_name")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToString::to_string)
                .or_else(|| prev.get("display_name").and_then(|v| v.as_str()).map(ToString::to_string))
                .unwrap_or_else(|| account_id.clone());
            let source = body
                .get("source")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToString::to_string)
                .or_else(|| prev.get("source").and_then(|v| v.as_str()).map(ToString::to_string))
                .unwrap_or_else(|| "codex".to_string());
            let cred_path = body
                .get("path")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .map(ToString::to_string)
                .or_else(|| prev.get("path").and_then(|v| v.as_str()).map(ToString::to_string))
                .unwrap_or_default();
            let has_cred = token.is_some()
                || (!cred_path.is_empty() && Path::new(&cred_path).exists());
            let login_state = if has_cred {
                "authenticated_source_only"
            } else {
                "unauthenticated"
            };
            let user_action = if has_cred {
                "use_explicit_api_key_connection_for_inference"
            } else if source == "kimi_code" {
                "login_in_kimi_code_then_refresh"
            } else {
                "login_in_codex_then_refresh"
            };
            let mut rec = prev;
            if let Some(obj) = rec.as_object_mut() {
                obj.insert("id".into(), json!(account_id));
                obj.insert("display_name".into(), json!(display_name));
                obj.insert("source".into(), json!(source));
                obj.insert("path".into(), json!(cred_path));
                obj.insert("revision".into(), json!(rev));
                obj.insert("state".into(), json!(login_state));
                obj.insert("updated_at_ms".into(), json!(now_ms()));
                if let Some(conn_id) = body.get("connection_id").and_then(|v| v.as_str()) {
                    obj.insert("connection_id".into(), json!(conn_id));
                }
            }
            let _ = suite.upsert_record("source_account", &account_id, &rec);
            (
                200,
                json!({
                    "ok": true,
                    "bound": true,
                    "account": {
                        "revision": rev,
                        "account": {
                            "id": account_id,
                            "display_name": display_name,
                            "source": source,
                            "path": cred_path
                        }
                    },
                    "login_state": login_state,
                    "inference_supported": false,
                    "user_action": user_action,
                    "expires_at_ms": Value::Null
                }),
            )
        }
        "/api/model-connections/source-accounts/refresh" => {
            let account_id = body
                .get("account_id")
                .or_else(|| body.get("id"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let Some(mut rec) = suite
                .get_record::<Value>("source_account", account_id)
                .ok()
                .flatten()
            else {
                return (404, json!({"ok": false, "error": "source_account_not_found"}));
            };
            let has_token = vault
                .as_ref()
                .is_some_and(|v| {
                    v.get_secret(&format!("source_account_token:{account_id}"))
                        .ok()
                        .flatten()
                        .is_some()
                        || v.get_secret(&format!("kimi_bearer:{account_id}"))
                            .ok()
                            .flatten()
                            .is_some()
                        || v.get_secret(&format!("opencode_password:{account_id}"))
                            .ok()
                            .flatten()
                            .is_some()
                });
            let display_name = rec
                .get("display_name")
                .and_then(|v| v.as_str())
                .unwrap_or(account_id)
                .to_string();
            let source = rec
                .get("source")
                .and_then(|v| v.as_str())
                .unwrap_or("codex")
                .to_string();
            let cred_path = rec
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let has_cred =
                has_token || (!cred_path.is_empty() && Path::new(&cred_path).exists());
            let login_state = if has_cred {
                "authenticated_source_only"
            } else {
                "unauthenticated"
            };
            let user_action = if has_cred {
                "use_explicit_api_key_connection_for_inference"
            } else if source == "kimi_code" {
                "login_in_kimi_code_then_refresh"
            } else {
                "login_in_codex_then_refresh"
            };
            let rev = rec.get("revision").and_then(|v| v.as_i64()).unwrap_or(1) + 1;
            if let Some(obj) = rec.as_object_mut() {
                obj.insert("revision".into(), json!(rev));
                obj.insert("has_token".into(), json!(has_token));
                obj.insert("state".into(), json!(login_state));
                obj.insert("refreshed_at_ms".into(), json!(now_ms()));
            }
            let _ = suite.upsert_record("source_account", account_id, &rec);
            (
                200,
                json!({
                    "ok": true,
                    "refreshed": true,
                    "has_token": has_token,
                    "account": {
                        "revision": rev,
                        "account": {
                            "id": account_id,
                            "display_name": display_name,
                            "source": source,
                            "path": cred_path
                        }
                    },
                    "login_state": login_state,
                    "inference_supported": false,
                    "user_action": user_action,
                    "expires_at_ms": Value::Null
                }),
            )
        }
        _ => (404, json!({"ok": false, "error": "not_found"})),
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn dispatch_selected_experts_parallel(
    suite: &Arc<ModulesSuite>,
    caller: Arc<dyn ModelCaller>,
    route: nola_types::ModelRouteConfig,
    policy: EffectivePolicy,
    workspace_root: &Path,
    conversation_id: &str,
    requested_room_id: Option<&str>,
    selected_expert_ids: &[String],
    prompt: &str,
) -> Result<(String, Vec<Value>), String> {
    let stored_experts = suite
        .list_records::<ExpertPeer>("expert_peer")
        .unwrap_or_default();
    let builtins = built_in_role_templates();
    let mut participant_ids: Vec<String> = Vec::new();
    let mut mention_names: Vec<String> = Vec::new();
    let now = now_ms();

    for raw_eid in selected_expert_ids {
        if let Some(exp) = stored_experts
            .iter()
            .find(|e| e.id == *raw_eid || e.name.eq_ignore_ascii_case(raw_eid))
        {
            if !participant_ids.contains(&exp.id) {
                participant_ids.push(exp.id.clone());
                mention_names.push(exp.name.clone());
            }
        } else if let Some(tpl) = builtins.iter().find(|t| {
            t.get("id").and_then(|v| v.as_str()) == Some(raw_eid.as_str())
                || t.get("name")
                    .and_then(|v| v.as_str())
                    .is_some_and(|n| n.eq_ignore_ascii_case(raw_eid))
        }) {
            let id = tpl
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or(raw_eid)
                .to_string();
            let name = tpl
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or(&id)
                .to_string();
            let role = tpl
                .get("role")
                .and_then(|v| v.as_str())
                .unwrap_or("Expert")
                .to_string();
            let system_prompt = tpl
                .get("system_prompt")
                .and_then(|v| v.as_str())
                .unwrap_or("You are a helpful domain expert.")
                .to_string();
            let peer = ExpertPeer {
                id: id.clone(),
                name: name.clone(),
                role,
                system_prompt,
                model_override: None,
                allowed_tools: Vec::new(),
                updated_at_ms: now,
            };
            let _ = suite.upsert_record("expert_peer", &peer.id, &peer);
            if !participant_ids.contains(&id) {
                participant_ids.push(id);
                mention_names.push(name);
            }
        }
    }

    if participant_ids.is_empty() {
        return Err("no matching expert peers found for selectedExperts".into());
    }

    let room_id = requested_room_id
        .map(ToString::to_string)
        .unwrap_or_else(|| format!("conv-room-{conversation_id}"));
    let mut room = suite
        .get_record::<AgentRoom>("agent_room", &room_id)
        .ok()
        .flatten()
        .unwrap_or_else(|| AgentRoom {
            id: room_id.clone(),
            name: format!("Conversation {conversation_id}"),
            workspace_root: workspace_root.display().to_string(),
            participant_expert_ids: participant_ids.clone(),
            session_id: conversation_id.to_string(),
            created_at_ms: now,
            updated_at_ms: now,
        });
    room.participant_expert_ids = participant_ids.clone();
    room.workspace_root = workspace_root.display().to_string();
    room.updated_at_ms = now;
    suite
        .upsert_record("agent_room", &room.id, &room)
        .map_err(|e| e.to_string())?;

    let target_recipient = if participant_ids.len() == 1 {
        mention_names.first().map(String::as_str)
    } else {
        None
    };

    let replies = RoomRuntime::send_message_with_parallel(
        suite,
        caller,
        route,
        policy,
        &room.id,
        prompt,
        target_recipient,
        true,
    )?;
    Ok((room.id, replies))
}

pub(super) fn parse_query_str(query: &str, key: &str) -> Option<String> {
    url::form_urlencoded::parse(query.as_bytes())
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.into_owned())
}

pub(super) fn parse_query_i64(query: &str, key: &str) -> Option<i64> {
    parse_query_str(query, key).and_then(|v| v.parse().ok())
}

pub(super) fn respond_json(req: Request, status: u16, value: &Value) -> std::io::Result<()> {
    let data = serde_json::to_vec(value).unwrap_or_else(|_| b"{}".to_vec());
    let header = Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap();
    let resp = Response::from_data(data)
        .with_status_code(StatusCode(status))
        .with_header(header);
    req.respond(resp)
}

pub(super) fn percent_decode_path(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (
                (bytes[i + 1] as char).to_digit(16),
                (bytes[i + 2] as char).to_digit(16),
            ) {
                out.push(((h << 4) | l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub(super) fn mime_for_path(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" | "mjs" => "application/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "wasm" => "application/wasm",
        _ => "application/octet-stream",
    }
}

pub(super) fn resolve_static_target<'a>(
    ui_dir: &'a Path,
    static_dir: &'a Path,
    decoded_path: &str,
) -> (&'a Path, String, bool) {
    if decoded_path == "/favicon.ico" || decoded_path == "/favicon.png" {
        return (static_dir, "favicon.png".to_string(), false);
    }
    if let Some(rest) = decoded_path.strip_prefix("/static/") {
        return (static_dir, rest.trim_start_matches('/').to_string(), false);
    }
    let rel = if decoded_path == "/ui" || decoded_path == "/ui/" {
        ""
    } else if let Some(rest) = decoded_path.strip_prefix("/ui/") {
        rest
    } else {
        decoded_path.trim_start_matches('/')
    };
    let rel = if rel.is_empty() { "index.html" } else { rel };
    let looks_like_spa_route = !rel.rsplit('/').next().unwrap_or("").contains('.');
    (ui_dir, rel.to_string(), looks_like_spa_route)
}

pub(super) fn serve_static_asset(req: Request, ui_dir: &Path, static_dir: &Path, path: &str) {
    let decoded = percent_decode_path(path);
    if decoded.contains('\0') || decoded.contains('\\') || decoded.contains("..") {
        let _ = respond_json(req, 403, &json!({"error": "forbidden_path"}));
        return;
    }
    let (root_dir, rel, spa_fallback) = resolve_static_target(ui_dir, static_dir, &decoded);
    let Ok(canon_root) = fs::canonicalize(root_dir) else {
        let _ = respond_json(req, 404, &json!({"error": "not_found"}));
        return;
    };
    let candidate = match fs::canonicalize(canon_root.join(&rel)) {
        Ok(p) if p.starts_with(&canon_root) && p.is_file() => p,
        _ if spa_fallback => match fs::canonicalize(canon_root.join("index.html")) {
            Ok(idx) if idx.starts_with(&canon_root) && idx.is_file() => idx,
            _ => {
                let _ = respond_json(req, 404, &json!({"error": "not_found"}));
                return;
            }
        },
        Ok(_) => {
            let _ = respond_json(req, 403, &json!({"error": "forbidden_path"}));
            return;
        }
        Err(_) => {
            let _ = respond_json(req, 404, &json!({"error": "not_found"}));
            return;
        }
    };
    if let Ok(bytes) = fs::read(&candidate) {
        let ct = Header::from_bytes(&b"Content-Type"[..], mime_for_path(&candidate).as_bytes())
            .unwrap();
        let nosniff = Header::from_bytes(&b"X-Content-Type-Options"[..], &b"nosniff"[..]).unwrap();
        let resp = Response::from_data(bytes)
            .with_header(ct)
            .with_header(nosniff);
        let _ = req.respond(resp);
    } else {
        let _ = respond_json(req, 404, &json!({"error": "not_found"}));
    }
}

