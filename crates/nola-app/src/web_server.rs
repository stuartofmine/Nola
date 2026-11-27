// SPDX-License-Identifier: Apache-2.0
//! Full-Capability Secure HTTP + SSE WebHost (`tiny_http`) for Web, iOS, macOS, Android & Windows clients:
//! - Strict same-origin CSRF (`Origin == Host` + `Sec-Fetch-Site != cross-site`)
//! - O(1) bounded replay-nonce cache
//! - Persistent 0600 mobile access token + session cookies + HMAC-SHA256 mobile auth + AES-256-GCM mobile crypto
//! - Live streaming SSE chat endpoint (`POST /api/chat/conversations/<id>/messages`) with real media attachments
//! - Active-run cancellation, session-scoped permissions, real compaction, MCP tool wiring, and background scheduler
//! - Complete business module REST endpoints aligned with `apps/web/frontend`, iOS, macOS & Android
//! - Safe static asset & SPA serving (`/ui/*`, `/static/*`, and SPA fallback).

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::fs;
use std::io::{BufRead, Read, Write};
use std::net::{IpAddr, ToSocketAddrs};
use std::path::{Component, Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use chrono::Utc;
use nola_engine::{
    approve_agent_plan, compact_session_now, is_plan_mode, load_session_plan, load_session_todos,
    register_planning_tools, register_subagent_tools, set_plan_mode, AgentEngine, AgentEvent,
    AgentRunConfig, ApprovalCallback, EffectivePolicy, ForceToolSet, HookRunner, HttpModelCaller,
    ModelCaller, PlanningState, SubagentManager, UserInputCallback,
};
use nola_llm::{is_loopback_url, LlmClient};
use nola_mcp::McpManager;
use nola_modules::a2ui::{
    commit_ui_interaction, list_conversation_surfaces, preview_ui_surface,
    read_ui_surface_changes, read_ui_surface_revision, register_scoped_tools as register_a2ui_tools,
    UiDraftPreviewInput, UiInteraction, UiSurfaceIdentity, UiSurfaceRevisionRef,
};
use nola_modules::automations_messaging::handle_inbound_webhook_turn_with_secret;
use nola_modules::{
    built_in_role_templates, check_for_update, download_update_package, index_data_source,
    install_plugin_package, list_data_source_usage, list_desktop_plugin_pages,
    list_harness_plugins, parse_inbound_webhook_payload, probe_engine_status, probe_hardware,
    recommend_local_models, reconcile_enabled_plugins, run_engine_setup_with_opt_in,
    search_hub_models, spawn_background_goal_worker, spawn_background_scheduler,
    spawn_messaging_channel_worker, test_data_source_connection, verify_inbound_webhook_token,
    AutomationScheduler, ExtendedGoalRecord, GoalWorker, LocalModelProcessManager,
    ModelDownloadManager, ModelDownloadPlan, ModulesSuite, RoomRuntime,
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
    ModelCapabilities, PermissionAction, PermissionProfile, PermissionRule, PluginRecord,
    RuleScope, Session, ToolExecutionResult, ToolSchema, ToolSpec, ToolTier,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};
use url::Url;

use crate::config::{
    build_frozen_system_blocks, connection_has_api_key, default_home_dir, load_active_config,
    read_config_file_safe, resolve_model_route, resolve_vault_path_for_store, save_applied_config,
    vault_secret_key, ConnectionConfig, DefaultModelSelection, ModelEntry,
};
use crate::web_api::{
    build_agent_settings_snapshot, build_dashboard_snapshot, build_model_profiles_response,
    build_workspace_review, canonical_automation_format, conversation_configuration_json,
    conversation_knowledge_key, ensure_host_id, ensure_source_store_epoch,
    infer_model_capabilities, load_conversation_knowledge, memory_item_to_record_with_store,
    memory_meta_key, ms_to_rfc3339, normalize_automation_record, normalize_data_source_record,
    project_skill_descriptor, reconstruct_v2_messages, session_meta_key,
    session_to_conversation_json, session_usage_json,
};

const MAX_TRACKED_NONCES: usize = 4_096;
const MAX_HTTP_BODY_BYTES: u64 = 16 * 1024 * 1024; // Allow image attachments up to 16 MiB
const SESSION_TTL_MS: i64 = 7 * 24 * 3600 * 1000;
const MAX_SESSIONS: usize = 256;
const MAX_CONCURRENT_REQUESTS: usize = 32;
const MOBILE_CRYPTO_AAD: &[u8] = b"nola-mobile-bridge-v1";
const MAX_GATEWAY_ATTEMPTS: usize = 200;

struct ChannelReader {
    rx: mpsc::Receiver<Vec<u8>>,
    buf: Vec<u8>,
    pos: usize,
    cancel: Option<Arc<AtomicBool>>,
    finished_eof: bool,
}

impl ChannelReader {
    fn new(rx: mpsc::Receiver<Vec<u8>>) -> Self {
        Self {
            rx,
            buf: Vec::new(),
            pos: 0,
            cancel: None,
            finished_eof: false,
        }
    }

    fn with_cancel(rx: mpsc::Receiver<Vec<u8>>, cancel: Arc<AtomicBool>) -> Self {
        Self {
            rx,
            buf: Vec::new(),
            pos: 0,
            cancel: Some(cancel),
            finished_eof: false,
        }
    }
}

impl Drop for ChannelReader {
    fn drop(&mut self) {
        if !self.finished_eof {
            if let Some(ref c) = self.cancel {
                c.store(true, Ordering::SeqCst);
            }
        }
    }
}

impl Read for ChannelReader {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        while self.pos >= self.buf.len() {
            match self.rx.recv() {
                Ok(chunk) if chunk.is_empty() => continue,
                Ok(chunk) => {
                    self.buf = chunk;
                    self.pos = 0;
                }
                Err(_) => {
                    self.finished_eof = true;
                    return Ok(0);
                }
            }
        }
        let avail = &self.buf[self.pos..];
        let n = avail.len().min(out.len());
        out[..n].copy_from_slice(&avail[..n]);
        self.pos += n;
        Ok(n)
    }
}

struct PendingApprovalEntry {
    conversation_id: String,
    tool_name: String,
    target_key: String,
    reason: String,
    arguments: Value,
    tx: mpsc::Sender<bool>,
}

struct PendingQuestionEntry {
    conversation_id: String,
    question: String,
    options: Vec<String>,
    tx: mpsc::Sender<Option<String>>,
}

struct PendingDeviceToolEntry {
    conversation_id: String,
    tx: mpsc::Sender<Value>,
}

/// Pending interactive approval, user-question, & hybrid device-tool waiters bridged
/// between SSE turns and REST, plus narrowly scoped `(conv_id, tool_name, target_key)` session grants.
#[derive(Default)]
struct PendingInteractions {
    approvals: Mutex<BTreeMap<String, PendingApprovalEntry>>,
    questions: Mutex<BTreeMap<String, PendingQuestionEntry>>,
    device_tools: Mutex<BTreeMap<String, PendingDeviceToolEntry>>,
    session_grants: Mutex<HashSet<(String, String, String)>>,
}

impl PendingInteractions {
    fn has_session_grant(&self, conv_id: &str, tool_name: &str, target_key: &str) -> bool {
        self.session_grants
            .lock()
            .unwrap()
            .contains(&(conv_id.to_string(), tool_name.to_string(), target_key.to_string()))
    }

    fn resolve_approval(&self, request_id: &str, allow: bool, scope: &str) -> bool {
        if let Some(entry) = self.approvals.lock().unwrap().remove(request_id) {
            if allow && scope.eq_ignore_ascii_case("session") {
                self.session_grants.lock().unwrap().insert((
                    entry.conversation_id,
                    entry.tool_name,
                    entry.target_key,
                ));
            }
            let _ = entry.tx.send(allow);
            true
        } else {
            false
        }
    }

    fn resolve_question(&self, request_id: &str, answer: Option<String>) -> bool {
        if let Some(entry) = self.questions.lock().unwrap().remove(request_id) {
            let _ = entry.tx.send(answer);
            true
        } else {
            false
        }
    }

    fn register_device_tool(
        &self,
        request_id: String,
        conversation_id: String,
    ) -> mpsc::Receiver<Value> {
        let (tx, rx) = mpsc::channel::<Value>();
        self.device_tools.lock().unwrap().insert(
            request_id,
            PendingDeviceToolEntry {
                conversation_id,
                tx,
            },
        );
        rx
    }

    fn resolve_device_tool(&self, request_id: &str, payload: Value) -> bool {
        if let Some(entry) = self.device_tools.lock().unwrap().remove(request_id) {
            let _ = entry.tx.send(payload);
            true
        } else {
            false
        }
    }

    fn conversation_needs_response(&self, conv_id: &str) -> bool {
        let has_approval = self
            .approvals
            .lock()
            .unwrap()
            .values()
            .any(|e| e.conversation_id == conv_id);
        if has_approval {
            return true;
        }
        let has_question = self
            .questions
            .lock()
            .unwrap()
            .values()
            .any(|e| e.conversation_id == conv_id);
        if has_question {
            return true;
        }
        self.device_tools
            .lock()
            .unwrap()
            .values()
            .any(|e| e.conversation_id == conv_id)
    }

    fn any_needs_response(&self) -> bool {
        !self.approvals.lock().unwrap().is_empty()
            || !self.questions.lock().unwrap().is_empty()
            || !self.device_tools.lock().unwrap().is_empty()
    }

    fn pending_permission_for(&self, conv_id: &str) -> Value {
        self.approvals
            .lock()
            .unwrap()
            .iter()
            .find(|(_, e)| e.conversation_id == conv_id)
            .map(|(req_id, e)| {
                json!({
                    "requestId": req_id,
                    "request_id": req_id,
                    "tool": e.tool_name,
                    "reason": e.reason,
                    "arguments": e.arguments
                })
            })
            .unwrap_or(Value::Null)
    }

    fn pending_permissions_list(&self, conv_id: &str) -> Vec<Value> {
        self.approvals
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, e)| e.conversation_id == conv_id)
            .take(64)
            .map(|(req_id, e)| {
                json!({
                    "request_id": req_id,
                    "tool": e.tool_name,
                    "reason": e.reason,
                    "arguments": e.arguments
                })
            })
            .collect()
    }

    fn pending_question_for(&self, conv_id: &str) -> Value {
        self.questions
            .lock()
            .unwrap()
            .iter()
            .find(|(_, e)| e.conversation_id == conv_id)
            .map(|(req_id, e)| {
                json!({
                    "requestId": req_id,
                    "request_id": req_id,
                    "question": e.question,
                    "options": e.options
                })
            })
            .unwrap_or(Value::Null)
    }

    fn clear_conversation(&self, conv_id: &str) {
        self.session_grants
            .lock()
            .unwrap()
            .retain(|(cid, _, _)| cid != conv_id);
        self.approvals
            .lock()
            .unwrap()
            .retain(|_, entry| entry.conversation_id != conv_id);
        self.questions
            .lock()
            .unwrap()
            .retain(|_, entry| entry.conversation_id != conv_id);
        self.device_tools
            .lock()
            .unwrap()
            .retain(|_, entry| entry.conversation_id != conv_id);
    }
}

/// Hybrid device-tool proxy registered on a turn when a mobile/desktop client declares
/// client-executed tools. Emits SSE `device_tool_request` and awaits REST callback resolution.
struct DeviceToolProxy {
    spec: ToolSpec,
    conversation_id: String,
    host_id: String,
    client_id: String,
    capability: String,
    tx: mpsc::Sender<Vec<u8>>,
    pending: Arc<PendingInteractions>,
    cancel: Arc<AtomicBool>,
    crypto_tok: Option<String>,
}

fn build_device_tool_request_frame(
    req_id: &str,
    conversation_id: &str,
    host_id: &str,
    client_id: &str,
    tool_name: &str,
    capability: &str,
    arguments: &Value,
    nonce: &str,
    expires_at_ms: i64,
) -> Value {
    let args_obj = if arguments.is_object() {
        arguments.clone()
    } else {
        json!({})
    };
    json!({
        "type": "device_tool_request",
        "id": req_id,
        "request_id": req_id,
        "call_id": req_id,
        "conversation_id": conversation_id,
        "session_id": conversation_id,
        "host_id": host_id,
        "client_id": client_id,
        "tool_name": tool_name,
        "tool": tool_name,
        "capability": capability,
        "arguments": args_obj,
        "args": args_obj,
        "nonce": nonce,
        "expires_at": expires_at_ms,
        "expires_at_iso": ms_to_rfc3339(expires_at_ms),
        "protocol_version": 1,
        "trace": {
            "type": "device_tool_request",
            "id": req_id,
            "request_id": req_id,
            "tool_name": tool_name,
            "session_id": conversation_id
        }
    })
}

fn device_tool_callback_is_ok(payload: &Value) -> bool {
    payload
        .get("ok")
        .and_then(|v| v.as_bool())
        .unwrap_or_else(|| match payload.get("error") {
            Some(Value::Bool(b)) => !b,
            Some(Value::Null) | None => true,
            Some(Value::String(s)) => s.trim().is_empty(),
            Some(_) => false,
        })
}

impl ToolHandler for DeviceToolProxy {
    fn spec(&self) -> ToolSpec {
        self.spec.clone()
    }

    fn execute(&self, _ctx: &ToolContext, args: Value) -> ToolExecutionResult {
        let req_id = new_id();
        let nonce = generate_hex_token_32();
        let expires_at_ms = now_ms().saturating_add(120_000);
        let rx = self
            .pending
            .register_device_tool(req_id.clone(), self.conversation_id.clone());
        let frame = build_device_tool_request_frame(
            &req_id,
            &self.conversation_id,
            &self.host_id,
            &self.client_id,
            &self.spec.schema.name,
            &self.capability,
            &args,
            &nonce,
            expires_at_ms,
        );
        let _ = self
            .tx
            .send(encode_sse_frame(&frame, self.crypto_tok.as_deref()));

        let deadline = Instant::now() + Duration::from_secs(120);
        let resolved = loop {
            if self.cancel.load(Ordering::SeqCst) {
                break None;
            }
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(val) => break Some(val),
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if Instant::now() >= deadline {
                        break None;
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break None,
            }
        };
        self.pending.device_tools.lock().unwrap().remove(&req_id);
        match resolved {
            Some(payload) => {
                let is_ok = device_tool_callback_is_ok(&payload);
                if is_ok {
                    let result_val = payload
                        .get("result")
                        .or_else(|| payload.get("output"))
                        .or_else(|| payload.get("payload"))
                        .cloned()
                        .unwrap_or(payload);
                    let summary = result_val
                        .as_str()
                        .map(|s| s.chars().take(240).collect::<String>())
                        .unwrap_or_else(|| format!("Device tool '{}' completed", self.spec.schema.name));
                    ToolExecutionResult::ok_structured(result_val, summary)
                } else {
                    let msg = payload
                        .get("error")
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.trim().is_empty())
                        .or_else(|| {
                            payload
                                .get("output")
                                .and_then(|v| v.as_str())
                                .filter(|s| !s.trim().is_empty())
                        })
                        .unwrap_or("device tool execution failed");
                    ToolExecutionResult::err_structured("device_tool_error", msg, None)
                }
            }
            None => ToolExecutionResult::err_structured(
                "device_tool_timeout",
                format!(
                    "device tool '{}' timed out or was cancelled waiting for client callback",
                    self.spec.schema.name
                ),
                None,
            ),
        }
    }
}

fn approval_target_key(args: &Value) -> String {
    for key in ["path", "file_path", "target_path", "command", "url", "endpoint"] {
        if let Some(s) = args.get(key).and_then(|v| v.as_str()) {
            let trimmed = s.trim();
            if !trimmed.is_empty() {
                return format!("{key}:{trimmed}");
            }
        }
    }
    let raw = serde_json::to_string(args).unwrap_or_default();
    format!("args:{}", &sha256_hex(raw.as_bytes())[..16])
}

/// O(1) bounded replay-nonce cache.
#[derive(Default)]
pub struct BoundedNonceCache {
    order: VecDeque<String>,
    set: HashSet<String>,
}

impl BoundedNonceCache {
    pub fn record_if_fresh(&mut self, nonce: String) -> bool {
        if self.set.contains(&nonce) {
            return false;
        }
        if self.order.len() >= MAX_TRACKED_NONCES {
            if let Some(oldest) = self.order.pop_front() {
                self.set.remove(&oldest);
            }
        }
        self.set.insert(nonce.clone());
        self.order.push_back(nonce);
        true
    }
}

/// Constant-time byte comparison to prevent timing leaks on secret tokens and HMACs.
fn ct_eq(a: &str, b: &str) -> bool {
    let ab = a.as_bytes();
    let bb = b.as_bytes();
    let max_len = ab.len().max(bb.len());
    let mut diff: u8 = if ab.len() == bb.len() { 0 } else { 1 };
    for i in 0..max_len {
        let x = ab.get(i).copied().unwrap_or(0);
        let y = bb.get(i).copied().unwrap_or(0);
        diff |= x ^ y;
    }
    diff == 0
}

/// Generate 32 cryptographically random bytes (OS CSPRNG via `/dev/urandom` or UUIDv4 CSPRNG fallback).
fn random_bytes_32() -> [u8; 32] {
    #[cfg(unix)]
    {
        if let Ok(mut f) = fs::File::open("/dev/urandom") {
            let mut buf = [0u8; 32];
            if f.read_exact(&mut buf).is_ok() {
                return buf;
            }
        }
    }
    let seed = format!("{}:{}:{}:{}", new_id(), new_id(), new_id(), now_ms());
    let digest = Sha256::digest(seed.as_bytes());
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

fn generate_hex_token_32() -> String {
    hex::encode(random_bytes_32())
}

fn mobile_access_token_path() -> PathBuf {
    if let Ok(nola_home) = std::env::var("NOLA_HOME") {
        let trimmed = nola_home.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed).join("web/mobile-access-token");
        }
    }
    default_home_dir().join(".nola/web/mobile-access-token")
}

/// Load or initialize the 32-byte hex access token (`NOLA_WEB_ACCESS_TOKEN` or `~/.nola/web/mobile-access-token` with mode 0600).
/// Returns `(token, is_env_override)`.
fn load_or_create_web_access_token() -> (String, bool) {
    if let Ok(env_tok) = std::env::var("NOLA_WEB_ACCESS_TOKEN") {
        let trimmed = env_tok.trim();
        if !trimmed.is_empty() {
            return (trimmed.to_string(), true);
        }
    }
    let path = mobile_access_token_path();
    if let Ok(raw) = fs::read_to_string(&path) {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            return (trimmed.to_string(), false);
        }
    }
    let token = generate_hex_token_32();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = nola_store::vault::atomic_write_restricted(&path, format!("{token}\n").as_bytes());
    (token, false)
}

/// Fail-closed same-origin CSRF check for all mutating HTTP requests.
pub fn validate_same_origin(request: &Request) -> Result<(), &'static str> {
    if matches!(request.method(), Method::Get | Method::Head | Method::Options) {
        return Ok(());
    }
    let host = header_value(request, "host").map(str::trim).unwrap_or("");
    if host.is_empty() {
        return Err("missing Host header");
    }
    let sfs = header_value(request, "sec-fetch-site");
    let origin = header_value(request, "origin");

    if let Some(sfs_val) = sfs {
        let lower = sfs_val.trim().to_ascii_lowercase();
        if !matches!(lower.as_str(), "same-origin" | "none") {
            return Err("rejected cross-site request via Sec-Fetch-Site");
        }
    }
    if let Some(origin_val) = origin {
        if !url_matches_host(origin_val, host)? {
            return Err("Origin header does not match Host");
        }
        return Ok(());
    }
    if sfs.is_some() {
        return Ok(());
    }
    if let Some(referer) = header_value(request, "referer") {
        if url_matches_host(referer, host)? {
            return Ok(());
        }
        return Err("Referer header does not match Host");
    }
    // Native clients (Android/iOS/macOS/CLI/gateway) do not send browser Origin headers;
    // permit authenticated native API callers when no browser Origin is present.
    if header_value(request, "authorization")
        .is_some_and(|v| v.trim_start().to_ascii_lowercase().starts_with("bearer "))
        || header_value(request, "x-nola-mobile-auth").is_some_and(|v| !v.trim().is_empty())
        || header_value(request, "x-nola-client").is_some_and(|v| !v.trim().is_empty())
        || header_value(request, "x-nola-webhook-token").is_some_and(|v| !v.trim().is_empty())
        || header_value(request, "x-webhook-secret").is_some_and(|v| !v.trim().is_empty())
    {
        return Ok(());
    }
    Err("missing Origin/Sec-Fetch-Site/Referer header on mutating request")
}

fn parse_host_and_port(host: &str, default_port: Option<u16>) -> Option<(String, Option<u16>)> {
    let trimmed = host.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(rest) = trimmed.strip_prefix('[') {
        let (addr, after) = rest.split_once(']')?;
        let port = if let Some(p_str) = after.strip_prefix(':') {
            Some(p_str.parse::<u16>().ok()?)
        } else if after.is_empty() {
            default_port
        } else {
            return None;
        };
        return Some((format!("[{addr}]"), port));
    }
    if let Some((h, p_str)) = trimmed.rsplit_once(':') {
        if !h.contains(':') {
            if let Ok(p) = p_str.parse::<u16>() {
                return Some((h.to_string(), Some(p)));
            }
        }
    }
    Some((trimmed.to_string(), default_port))
}

fn url_matches_host(raw: &str, host: &str) -> Result<bool, &'static str> {
    let parsed = Url::parse(raw).map_err(|_| "invalid Origin/Referer header")?;
    let Some(origin_host) = parsed.host_str() else {
        return Err("Origin/Referer missing host");
    };
    let default_port = match parsed.scheme() {
        "http" => Some(80),
        "https" => Some(443),
        _ => None,
    };
    let origin_port = parsed.port_or_known_default();
    let Some((req_host, req_port)) = parse_host_and_port(host, default_port) else {
        return Err("invalid Host header");
    };
    Ok(origin_host.eq_ignore_ascii_case(&req_host) && origin_port == req_port)
}

fn header_value<'a>(request: &'a Request, name: &str) -> Option<&'a str> {
    request
        .headers()
        .iter()
        .find(|h| h.field.as_str().as_str().eq_ignore_ascii_case(name))
        .map(|h| h.value.as_str())
}

fn extract_bearer_token<'a>(request: &'a Request) -> Option<&'a str> {
    let raw = header_value(request, "authorization")?.trim();
    let (scheme, tok) = raw.split_once(' ')?;
    if scheme.eq_ignore_ascii_case("bearer") {
        let t = tok.trim();
        (!t.is_empty()).then_some(t)
    } else {
        None
    }
}

fn extract_cookie_value(request: &Request, cookie_name: &str) -> Option<String> {
    for h in request.headers() {
        if h.field.as_str().as_str().eq_ignore_ascii_case("cookie") {
            for pair in h.value.as_str().split(';') {
                if let Some((k, v)) = pair.trim().split_once('=') {
                    if k.trim() == cookie_name {
                        let val = v.trim();
                        if !val.is_empty() {
                            return Some(val.to_string());
                        }
                    }
                }
            }
        }
    }
    None
}

fn is_valid_session_cookie(
    request: &Request,
    sessions: &Arc<Mutex<BTreeMap<String, i64>>>,
) -> bool {
    let Some(tok) = extract_cookie_value(request, "nola_session") else {
        return false;
    };
    let now = now_ms();
    let mut map = sessions.lock().unwrap();
    map.retain(|_, exp| *exp > now);
    map.get(&tok).is_some_and(|exp| *exp > now)
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    let mut k_block = [0u8; 64];
    if key.len() > 64 {
        let hashed = Sha256::digest(key);
        k_block[..32].copy_from_slice(&hashed);
    } else {
        k_block[..key.len()].copy_from_slice(key);
    }
    let mut ipad = [0x36u8; 64];
    let mut opad = [0x5cu8; 64];
    for i in 0..64 {
        ipad[i] ^= k_block[i];
        opad[i] ^= k_block[i];
    }
    let mut inner = Sha256::new();
    inner.update(ipad);
    inner.update(data);
    let inner_digest = inner.finalize();

    let mut outer = Sha256::new();
    outer.update(opad);
    outer.update(inner_digest);
    let out_digest = outer.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&out_digest);
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MobileHmacResult {
    NotPresent,
    Valid,
    Invalid,
    ReplayedNonce,
}

fn validate_mobile_hmac_auth(
    request: &Request,
    method_str: &str,
    url_path_and_query: &str,
    raw_body: &[u8],
    access_token: &str,
    nonces: &Arc<Mutex<BoundedNonceCache>>,
) -> MobileHmacResult {
    let Some(supplied_sig) = header_value(request, "x-nola-mobile-auth").map(str::trim) else {
        return MobileHmacResult::NotPresent;
    };
    if access_token.is_empty() || supplied_sig.is_empty() {
        return MobileHmacResult::Invalid;
    }
    let Some(ts_str) = header_value(request, "x-nola-mobile-timestamp").map(str::trim) else {
        return MobileHmacResult::Invalid;
    };
    let Some(nonce) = header_value(request, "x-nola-mobile-nonce").map(str::trim) else {
        return MobileHmacResult::Invalid;
    };
    let Some(body_sha_hdr) = header_value(request, "x-nola-mobile-body-sha256").map(str::trim) else {
        return MobileHmacResult::Invalid;
    };
    if nonce.is_empty() {
        return MobileHmacResult::Invalid;
    }
    let Ok(ts_secs) = ts_str.parse::<i64>() else {
        return MobileHmacResult::Invalid;
    };
    let now_secs = Utc::now().timestamp();
    if (now_secs - ts_secs).abs() > 300 {
        return MobileHmacResult::Invalid;
    }
    let actual_body_sha = sha256_hex(raw_body);
    if !ct_eq(&actual_body_sha, &body_sha_hdr.to_ascii_lowercase()) {
        return MobileHmacResult::Invalid;
    }
    let message = format!(
        "v1\n{}\n{}\n{}\n{}\n{}",
        method_str.to_ascii_uppercase(),
        url_path_and_query,
        ts_str,
        nonce,
        actual_body_sha
    );
    let mac = hmac_sha256(access_token.as_bytes(), message.as_bytes());
    let expected_sig = base64url_encode(&mac);
    if !ct_eq(&expected_sig, supplied_sig) {
        return MobileHmacResult::Invalid;
    }
    // SEC-1: Record mobile HMAC nonce ONLY AFTER signature + timestamp + body digest succeed!
    if !nonces
        .lock()
        .unwrap()
        .record_if_fresh(format!("mobile:{nonce}"))
    {
        return MobileHmacResult::ReplayedNonce;
    }
    MobileHmacResult::Valid
}

/// Returns `(authenticated, replayed_mobile_nonce)`.
fn authenticate_request(
    request: &Request,
    method_str: &str,
    full_url: &str,
    raw_body: &[u8],
    access_token: &str,
    sessions: &Arc<Mutex<BTreeMap<String, i64>>>,
    nonces: &Arc<Mutex<BoundedNonceCache>>,
) -> (bool, bool) {
    // Always validate mobile HMAC + replay if the header is present!
    // Fail closed if an explicit X-Nola-Mobile-Auth proof fails verification.
    match validate_mobile_hmac_auth(request, method_str, full_url, raw_body, access_token, nonces) {
        MobileHmacResult::Valid => return (true, false),
        MobileHmacResult::ReplayedNonce => return (false, true),
        MobileHmacResult::Invalid => return (false, false),
        MobileHmacResult::NotPresent => {}
    }
    if is_valid_session_cookie(request, sessions) {
        return (true, false);
    }
    if let Some(bearer) = extract_bearer_token(request) {
        if ct_eq(bearer, access_token) {
            return (true, false);
        }
        let now = now_ms();
        let map = sessions.lock().unwrap();
        if map.get(bearer).is_some_and(|exp| *exp > now) {
            return (true, false);
        }
    }
    (false, false)
}

fn is_loopback_host(host: &str) -> bool {
    matches!(host, "127.0.0.1" | "localhost" | "::1")
}

/// Implicit loopback trust applies ONLY to direct local Web UI requests on loopback
/// when `NOLA_WEB_ACCESS_TOKEN` is not set. It NEVER applies to `/api/mobile/*`
/// (paired mobile/remote host contract) or to requests supplying explicit
/// authentication headers (`Authorization` or `X-Nola-Mobile-Auth`).
fn compute_local_trust(
    loopback: bool,
    env_token_override: bool,
    raw_path: &str,
    has_explicit_auth: bool,
) -> bool {
    let is_mobile_api = raw_path == "/api/mobile" || raw_path.starts_with("/api/mobile/");
    loopback && !env_token_override && !is_mobile_api && !has_explicit_auth
}

fn server_started_handshake_full(
    host: &str,
    bound_port: u16,
    host_id: &str,
    access_token: &str,
    credentials_path: &Path,
) -> Value {
    let host_part = if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host.to_string()
    };
    let origin = format!("http://{host_part}:{bound_port}");
    json!({
        "type": "server.started",
        "protocol_version": 1,
        "loopback": is_loopback_host(host),
        "origin": origin,
        "client_api": format!("{origin}/api/mobile"),
        "host_id": host_id,
        "access_token": access_token,
        "credentials_path": credentials_path.display().to_string(),
        "clientAPI": {
            "path": "/api/mobile"
        }
    })
}

#[cfg(test)]
fn server_started_handshake(host: &str, bound_port: u16) -> Value {
    server_started_handshake_full(
        host,
        bound_port,
        "nola-host-0000000000000000",
        "test-token",
        Path::new("/tmp/mobile-access-token"),
    )
}

/// Fail-closed validation for Chrome Native Browser Extension host routes:
/// requires a non-empty `NOLA_NATIVE_BROWSER_TOKEN` matching `X-Nola-Browser-Token`,
/// `X-Nola-Native-Browser-Token`, or `Authorization: Bearer <token>`.
pub fn validate_native_browser_token(request: &Request) -> bool {
    let expected = match std::env::var("NOLA_NATIVE_BROWSER_TOKEN") {
        Ok(v) if !v.trim().is_empty() => v.trim().to_string(),
        _ => return false,
    };
    let supplied = header_value(request, "x-nola-browser-token")
        .or_else(|| header_value(request, "x-nola-native-browser-token"))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .or_else(|| extract_bearer_token(request));
    match supplied {
        Some(tok) => ct_eq(tok, &expected),
        None => false,
    }
}

fn method_as_str(m: &Method) -> &'static str {
    match m {
        Method::Get => "GET",
        Method::Head => "HEAD",
        Method::Post => "POST",
        Method::Put => "PUT",
        Method::Delete => "DELETE",
        Method::Connect => "CONNECT",
        Method::Options => "OPTIONS",
        Method::Trace => "TRACE",
        Method::Patch => "PATCH",
        Method::NonStandard(_) => "NONSTANDARD",
    }
}

fn read_raw_body(req: &mut Request) -> Vec<u8> {
    if matches!(req.method(), Method::Get | Method::Head | Method::Options) {
        return Vec::new();
    }
    let mut bytes = Vec::new();
    let _ = req
        .as_reader()
        .take(MAX_HTTP_BODY_BYTES)
        .read_to_end(&mut bytes);
    bytes
}

fn parse_json_or_empty(bytes: &[u8]) -> Value {
    if bytes.is_empty() {
        return json!({});
    }
    serde_json::from_slice(bytes).unwrap_or_else(|_| json!({}))
}

fn unwrap_input_envelope(body: &Value) -> Value {
    body.get("input")
        .filter(|v| v.is_object())
        .cloned()
        .unwrap_or_else(|| body.clone())
}

fn wants_mobile_crypto(req: &Request) -> bool {
    header_value(req, "x-nola-mobile-crypto")
        .map(str::trim)
        .is_some_and(|v| v.eq_ignore_ascii_case("aes-gcm-v1"))
}

fn maybe_decrypt_mobile_body(raw_body: &[u8], access_token: &str) -> Result<Value, String> {
    let val: Value = serde_json::from_slice(raw_body)
        .map_err(|e| format!("invalid JSON in encrypted mobile body: {e}"))?;
    let is_envelope = val.get("v").and_then(|v| v.as_i64()) == Some(1)
        && val
            .get("alg")
            .and_then(|v| v.as_str())
            .is_some_and(|a| a.eq_ignore_ascii_case("AES-256-GCM"))
        && val.get("nonce").and_then(|v| v.as_str()).is_some()
        && val.get("ciphertext").and_then(|v| v.as_str()).is_some()
        && val.get("tag").and_then(|v| v.as_str()).is_some();
    if !is_envelope {
        return Err("expected AES-256-GCM v1 mobile crypto envelope".into());
    }
    decrypt_mobile_json_envelope(&val, access_token)
}

fn encode_sse_frame(frame: &Value, crypto_tok: Option<&str>) -> Vec<u8> {
    if let Some(tok) = crypto_tok {
        if let Ok(mut env) = encrypt_mobile_json_envelope(frame, tok) {
            if let Some(map) = env.as_object_mut() {
                map.insert("type".into(), json!("encrypted"));
            }
            return format!("data: {env}\n\n").into_bytes();
        }
    }
    format!("data: {frame}\n\n").into_bytes()
}

fn maybe_encrypt_api_payload(value: &Value, encrypt_for_token: Option<&str>) -> Value {
    if let Some(tok) = encrypt_for_token {
        if let Ok(mut env) = encrypt_mobile_json_envelope(value, tok) {
            if let Some(map) = env.as_object_mut() {
                map.insert("type".into(), json!("encrypted"));
            }
            return env;
        }
    }
    value.clone()
}

fn respond_api_json(
    req: Request,
    status: u16,
    value: &Value,
    encrypt_for_token: Option<&str>,
) -> std::io::Result<()> {
    let body = maybe_encrypt_api_payload(value, encrypt_for_token);
    respond_json(req, status, &body)
}

fn respond_api_json_with_headers(
    req: Request,
    status: u16,
    value: &Value,
    extra_headers: Vec<Header>,
    encrypt_for_token: Option<&str>,
) -> std::io::Result<()> {
    let body = maybe_encrypt_api_payload(value, encrypt_for_token);
    respond_json_with_headers(req, status, &body, extra_headers)
}

fn respond_json_with_headers(
    req: Request,
    status: u16,
    value: &Value,
    extra_headers: Vec<Header>,
) -> std::io::Result<()> {
    let data = serde_json::to_vec(value).unwrap_or_else(|_| b"{}".to_vec());
    let ct = Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap();
    let mut resp = Response::from_data(data)
        .with_status_code(StatusCode(status))
        .with_header(ct);
    for h in extra_headers {
        resp = resp.with_header(h);
    }
    req.respond(resp)
}

// --- One-Time Mobile-to-Web Handoff Grants ---

const HANDOFF_TTL_SECS: u64 = 60;
const MAX_HANDOFF_GRANTS: usize = 64;

struct HandoffGrant {
    target: String,
    created_at: Instant,
    expires_at: Instant,
}

fn safe_handoff_target(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.starts_with('/')
        && !trimmed.starts_with("//")
        && !trimmed.contains('\\')
        && !trimmed
            .bytes()
            .any(|b| !b.is_ascii() || b.is_ascii_control() || b.is_ascii_whitespace())
    {
        trimmed.to_string()
    } else {
        "/settings".to_string()
    }
}

fn mint_web_handoff(
    handoffs: &Mutex<HashMap<String, HandoffGrant>>,
    raw_target: &str,
    now: Instant,
) -> (String, String) {
    let safe_target = safe_handoff_target(raw_target);
    let raw_code = generate_hex_token_32();
    let digest = sha256_hex(raw_code.as_bytes());
    let mut map = handoffs.lock().unwrap_or_else(|e| e.into_inner());
    map.retain(|_, g| g.expires_at > now);
    while map.len() >= MAX_HANDOFF_GRANTS {
        let Some(oldest) = map
            .iter()
            .min_by_key(|(_, g)| g.created_at)
            .map(|(k, _)| k.clone())
        else {
            break;
        };
        map.remove(&oldest);
    }
    map.insert(
        digest,
        HandoffGrant {
            target: safe_target.clone(),
            created_at: now,
            expires_at: now + Duration::from_secs(HANDOFF_TTL_SECS),
        },
    );
    (raw_code, safe_target)
}

fn consume_web_handoff(
    handoffs: &Mutex<HashMap<String, HandoffGrant>>,
    raw_code: &str,
    now: Instant,
) -> Option<String> {
    let trimmed = raw_code.trim();
    if trimmed.is_empty() || trimmed.contains('/') {
        return None;
    }
    let digest = sha256_hex(trimmed.as_bytes());
    let mut map = handoffs.lock().unwrap_or_else(|e| e.into_inner());
    map.retain(|_, g| g.expires_at > now);
    map.remove(&digest)
        .filter(|g| g.expires_at > now)
        .map(|g| g.target)
}

fn issue_browser_session(sessions: &Mutex<BTreeMap<String, i64>>) -> String {
    let session_tok = generate_hex_token_32();
    let expiry = now_ms().saturating_add(SESSION_TTL_MS);
    let mut map = sessions.lock().unwrap_or_else(|e| e.into_inner());
    let now = now_ms();
    map.retain(|_, exp| *exp > now);
    while map.len() >= MAX_SESSIONS {
        if let Some(oldest_key) = map
            .iter()
            .min_by_key(|(_, exp)| **exp)
            .map(|(k, _)| k.clone())
        {
            map.remove(&oldest_key);
        } else {
            break;
        }
    }
    map.insert(session_tok.clone(), expiry);
    session_tok
}

// --- Tailcat Verified Binary, Ingress Management & Ephemeral Transport Leases ---

const TAILCAT_GRANT_SECONDS: u64 = 20 * 60;
const TAILCAT_GLOBAL_LEASE_LIMIT: usize = 32;
const TAILCAT_PER_OWNER_LEASE_LIMIT: usize = 8;
const TAILCAT_SHUTDOWN_GRACE: Duration = Duration::from_secs(5);

#[derive(Debug, Clone)]
struct TailcatConfig {
    ingress: bool,
    helper_path: Option<PathBuf>,
    helper_sha256: Option<String>,
    web_port: u16,
}

impl TailcatConfig {
    fn is_available(&self) -> bool {
        self.helper_path.is_some()
    }
}

fn normalize_windows_verbatim(path: &Path) -> PathBuf {
    let raw = path.to_string_lossy();
    if let Some(rest) = raw.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{rest}"))
    } else if let Some(rest) = raw.strip_prefix(r"\\?\") {
        PathBuf::from(rest)
    } else {
        path.to_path_buf()
    }
}

fn verified_tailcat_command(
    program: &Path,
    expected_sha256: Option<&str>,
) -> Result<Command, String> {
    if !program.is_absolute()
        || program
            .components()
            .any(|component| matches!(component, Component::ParentDir))
    {
        return Err("An absolute Tailcat transport executable path without '..' is required".into());
    }
    if let Some(expected) = expected_sha256 {
        if expected.len() != 64 || !expected.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("Tailcat helper SHA-256 must be exactly 64 hex characters".into());
        }
    }
    let canonical = program.canonicalize().map_err(|_| {
        "Tailcat transport executable is unavailable; no alternate program was selected"
    })?;
    if !canonical.is_file() {
        return Err("Tailcat transport executable must be an exact regular-file path".into());
    }
    if let Some(expected) = expected_sha256 {
        let mut file =
            fs::File::open(&canonical).map_err(|_| "Cannot open Tailcat transport executable")?;
        let mut hasher = Sha256::new();
        std::io::copy(&mut file, &mut hasher)
            .map_err(|_| "Cannot hash Tailcat transport executable")?;
        let actual = format!("{:x}", hasher.finalize());
        if !ct_eq(&actual, &expected.to_ascii_lowercase()) {
            return Err("Tailcat transport executable SHA-256 mismatch; launch refused".into());
        }
    }
    let exec_path = normalize_windows_verbatim(&canonical);
    let mut command = Command::new(exec_path);
    command.env_clear();
    #[cfg(not(windows))]
    command.env("PATH", "/usr/bin:/bin");
    #[cfg(windows)]
    {
        let system_root = std::env::var_os("SystemRoot")
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| std::ffi::OsString::from(r"C:\Windows"));
        command
            .env("PATH", r"C:\Windows\System32;C:\Windows")
            .env("SystemRoot", system_root);
        for key in ["TEMP", "TMP"] {
            if let Some(v) = std::env::var_os(key).filter(|val| !val.is_empty()) {
                command.env(key, v);
            }
        }
    }
    Ok(command)
}

#[derive(Debug, PartialEq, Eq)]
enum TailcatLeaseCommand {
    Renew,
    Stop,
}

#[derive(Debug, PartialEq, Eq)]
struct TailcatLeaseError {
    status: u16,
    code: &'static str,
    detail: Option<String>,
}

impl TailcatLeaseError {
    fn new(status: u16, code: &'static str) -> Self {
        Self {
            status,
            code,
            detail: None,
        }
    }

    fn with_detail(status: u16, code: &'static str, detail: Option<String>) -> Self {
        Self {
            status,
            code,
            detail,
        }
    }
}

struct TailcatLease {
    owner: String,
    control: mpsc::Sender<TailcatLeaseCommand>,
    finished: Arc<(Mutex<Option<bool>>, Condvar)>,
}

#[derive(Default)]
struct TailcatLeaseManager {
    leases: Mutex<HashMap<String, TailcatLease>>,
}

fn sanitize_tailcat_helper_detail(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.len() > 96 {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.contains("http://")
        || lower.contains("https://")
        || lower.contains("nodekey:")
        || lower.contains("psk")
    {
        return None;
    }
    if trimmed
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b':'))
    {
        Some(trimmed.to_string())
    } else {
        None
    }
}

fn extract_tailcat_helper_error_detail(frame: &Value) -> Option<String> {
    if frame.get("protocolVersion").and_then(Value::as_u64) != Some(1)
        || frame.get("event").and_then(Value::as_str) != Some("error")
    {
        return None;
    }
    let stage = frame
        .get("stage")
        .and_then(Value::as_str)
        .and_then(sanitize_tailcat_helper_detail)?;
    let detail = frame
        .get("detail")
        .and_then(Value::as_str)
        .and_then(sanitize_tailcat_helper_detail);
    Some(match detail {
        Some(d) => format!("{stage}:{d}"),
        None => stage,
    })
}

fn wait_or_kill_tailcat_child(
    child: &Mutex<Child>,
    grace: Duration,
) -> std::io::Result<std::process::ExitStatus> {
    let deadline = Instant::now() + grace;
    loop {
        {
            let mut guard = child.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(status) = guard.try_wait()? {
                return Ok(status);
            }
            if Instant::now() >= deadline {
                let _ = guard.kill();
                return guard.wait();
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

impl TailcatLeaseManager {
    fn create_lease(
        self: &Arc<Self>,
        cfg: &TailcatConfig,
        host_id: &str,
        owner: String,
        client_public_key: &str,
    ) -> Result<Value, TailcatLeaseError> {
        self.create_lease_with_options(
            cfg,
            host_id,
            owner,
            client_public_key,
            Duration::from_secs(35),
            Duration::from_secs(TAILCAT_GRANT_SECONDS),
            TAILCAT_SHUTDOWN_GRACE,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn create_lease_with_options(
        self: &Arc<Self>,
        cfg: &TailcatConfig,
        host_id: &str,
        owner: String,
        client_public_key: &str,
        startup_timeout: Duration,
        grant_duration: Duration,
        shutdown_grace: Duration,
    ) -> Result<Value, TailcatLeaseError> {
        let Some(ref program) = cfg.helper_path else {
            return Err(TailcatLeaseError::new(409, "tailcat_not_available"));
        };
        if client_public_key.len() > 128 || !client_public_key.starts_with("nodekey:") {
            return Err(TailcatLeaseError::new(400, "invalid_transport_public_key"));
        }
        let mut command = verified_tailcat_command(program, cfg.helper_sha256.as_deref())
            .map_err(|_| TailcatLeaseError::new(503, "tailcat_helper_verification_failed"))?;

        let id = new_id();
        let (control_tx, control_rx) = mpsc::channel();
        let finished = Arc::new((Mutex::new(None), Condvar::new()));
        {
            let mut leases = self.leases.lock().unwrap_or_else(|e| e.into_inner());
            if leases.len() >= TAILCAT_GLOBAL_LEASE_LIMIT {
                return Err(TailcatLeaseError::new(503, "transport_capacity_reached"));
            }
            let owner_leases = leases.values().filter(|l| l.owner == owner).count();
            if owner_leases >= TAILCAT_PER_OWNER_LEASE_LIMIT {
                return Err(TailcatLeaseError::new(
                    429,
                    "transport_owner_capacity_reached",
                ));
            }
            leases.insert(
                id.clone(),
                TailcatLease {
                    owner,
                    control: control_tx,
                    finished: Arc::clone(&finished),
                },
            );
        }

        let mut child = match command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(_) => {
                self.leases
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(&id);
                return Err(TailcatLeaseError::new(503, "tailcat_start_failed"));
            }
        };
        let mut input = child.stdin.take().expect("tailcat helper stdin");
        let output = child.stdout.take().expect("tailcat helper stdout");
        if let Some(stderr) = child.stderr.take() {
            std::thread::spawn(move || {
                let mut sink = Vec::new();
                let _ = stderr.take(4096).read_to_end(&mut sink);
            });
        }
        let child = Arc::new(Mutex::new(child));
        let child_for_timeout = Arc::clone(&child);
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let grant_id = id.clone();
        let host_owned = host_id.to_string();
        let pub_owned = client_public_key.to_string();
        let origin = format!("http://127.0.0.1:{}", cfg.web_port);
        let mgr = Arc::clone(self);
        std::thread::spawn(move || {
            let frame = json!({
                "version": 1,
                "kind": "host",
                "host": {
                    "origin": origin,
                    "expectedHostId": host_owned,
                    "allowedClientKeys": [pub_owned]
                }
            });
            let startup: Result<String, Option<String>> = (|| {
                serde_json::to_writer(&mut input, &frame).map_err(|_| None)?;
                input
                    .write_all(b"\n")
                    .and_then(|_| input.flush())
                    .map_err(|_| None)?;
                let mut line = String::new();
                std::io::BufReader::new(output)
                    .take(64 * 1024)
                    .read_line(&mut line)
                    .map_err(|_| None)?;
                let parsed: Value = serde_json::from_str(&line).map_err(|_| None)?;
                if parsed.get("protocolVersion").and_then(Value::as_u64) != Some(1)
                    || parsed.get("event").and_then(Value::as_str) != Some("ready")
                {
                    return Err(extract_tailcat_helper_error_detail(&parsed));
                }
                parsed
                    .get("address")
                    .and_then(Value::as_str)
                    .filter(|s| !s.is_empty())
                    .map(ToString::to_string)
                    .ok_or(None)
            })();
            let started = startup.is_ok();
            if ready_tx.send(startup).is_ok() && started {
                let mut deadline = Instant::now() + grant_duration;
                loop {
                    let now = Instant::now();
                    if now >= deadline {
                        break;
                    }
                    match control_rx.recv_timeout(deadline.saturating_duration_since(now)) {
                        Ok(TailcatLeaseCommand::Renew) => {
                            deadline = Instant::now() + grant_duration;
                        }
                        Ok(TailcatLeaseCommand::Stop)
                        | Err(mpsc::RecvTimeoutError::Timeout)
                        | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                }
            }
            let _ = input.write_all(b"{\"version\":1,\"kind\":\"stop\"}\n");
            let _ = input.flush();
            drop(input);
            let exit = wait_or_kill_tailcat_child(&child, shutdown_grace);
            mgr.finish_lease(&grant_id, &finished, exit);
        });

        let address = match ready_rx.recv_timeout(startup_timeout) {
            Ok(Ok(addr)) => addr,
            Ok(Err(detail)) => {
                return Err(TailcatLeaseError::with_detail(
                    503,
                    "tailcat_start_failed",
                    detail,
                ));
            }
            Err(_) => {
                let mut guard = child_for_timeout.lock().unwrap_or_else(|e| e.into_inner());
                let _ = guard.kill();
                drop(guard);
                self.leases
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(&id);
                return Err(TailcatLeaseError::new(503, "tailcat_start_timeout"));
            }
        };
        Ok(json!({
            "version": 1,
            "leaseId": id,
            "address": address,
            "hostId": host_id,
            "expiresIn": TAILCAT_GRANT_SECONDS
        }))
    }

    fn finish_lease(
        &self,
        id: &str,
        finished: &(Mutex<Option<bool>>, Condvar),
        exit: std::io::Result<std::process::ExitStatus>,
    ) {
        let (state, wake) = finished;
        *state.lock().unwrap_or_else(|e| e.into_inner()) =
            Some(exit.as_ref().is_ok_and(|s| s.success()));
        wake.notify_all();
        self.leases
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(id);
    }

    fn renew_lease(&self, owner: &str, id: &str) -> Result<Value, TailcatLeaseError> {
        let leases = self.leases.lock().unwrap_or_else(|e| e.into_inner());
        let lease = leases
            .get(id)
            .ok_or_else(|| TailcatLeaseError::new(404, "transport_lease_unavailable"))?;
        if lease.owner != owner {
            return Err(TailcatLeaseError::new(
                403,
                "transport_lease_owner_mismatch",
            ));
        }
        lease
            .control
            .send(TailcatLeaseCommand::Renew)
            .map_err(|_| TailcatLeaseError::new(404, "transport_lease_unavailable"))?;
        Ok(json!({
            "version": 1,
            "leaseId": id,
            "renewed": true,
            "expiresIn": TAILCAT_GRANT_SECONDS
        }))
    }

    fn close_lease(&self, owner: &str, id: &str) -> Result<Value, TailcatLeaseError> {
        let finished = {
            let leases = self.leases.lock().unwrap_or_else(|e| e.into_inner());
            let lease = leases
                .get(id)
                .ok_or_else(|| TailcatLeaseError::new(404, "transport_lease_unavailable"))?;
            if lease.owner != owner {
                return Err(TailcatLeaseError::new(
                    403,
                    "transport_lease_owner_mismatch",
                ));
            }
            let _ = lease.control.send(TailcatLeaseCommand::Stop);
            Arc::clone(&lease.finished)
        };
        let (state, wake) = &*finished;
        let guard = state.lock().unwrap_or_else(|e| e.into_inner());
        let (state, _) = wake
            .wait_timeout_while(guard, TAILCAT_SHUTDOWN_GRACE, |v| v.is_none())
            .unwrap_or_else(|e| e.into_inner());
        Ok(json!({
            "closed": *state == Some(true),
            "closing": state.is_none(),
            "stopUnconfirmed": *state == Some(false),
            "runStateChanged": false
        }))
    }

    fn shutdown_all(&self) {
        let active: Vec<Arc<(Mutex<Option<bool>>, Condvar)>> = {
            let leases = self.leases.lock().unwrap_or_else(|e| e.into_inner());
            leases
                .values()
                .map(|l| {
                    let _ = l.control.send(TailcatLeaseCommand::Stop);
                    Arc::clone(&l.finished)
                })
                .collect()
        };
        let deadline = Instant::now() + TAILCAT_SHUTDOWN_GRACE;
        for finished in active {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            let (state, wake) = &*finished;
            let guard = state.lock().unwrap_or_else(|e| e.into_inner());
            let _ = wake
                .wait_timeout_while(guard, remaining, |v| v.is_none())
                .unwrap_or_else(|e| e.into_inner());
        }
    }
}

struct TailcatIngressChild {
    stdin: Option<std::process::ChildStdin>,
    child: Mutex<Child>,
}

impl TailcatIngressChild {
    fn spawn(cfg: &TailcatConfig, host_id: &str) -> Result<Self, String> {
        let Some(ref helper) = cfg.helper_path else {
            return Err("--tailcat-ingress requires --tailcat-helper".into());
        };
        let mut cmd = verified_tailcat_command(helper, cfg.helper_sha256.as_deref())?;
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("failed spawning Tailcat ingress helper: {e}"))?;
        let mut stdin = child.stdin.take();
        if let Some(ref mut input) = stdin {
            let init = json!({
                "version": 1,
                "kind": "ingress",
                "host": {
                    "origin": format!("http://127.0.0.1:{}", cfg.web_port),
                    "expectedHostId": host_id
                }
            });
            let _ = serde_json::to_writer(&mut *input, &init);
            let _ = input.write_all(b"\n");
            let _ = input.flush();
        }
        if let Some(stdout) = child.stdout.take() {
            std::thread::spawn(move || {
                let mut sink = Vec::new();
                let _ = stdout.take(64 * 1024).read_to_end(&mut sink);
            });
        }
        if let Some(stderr) = child.stderr.take() {
            std::thread::spawn(move || {
                let mut sink = Vec::new();
                let _ = stderr.take(16 * 1024).read_to_end(&mut sink);
            });
        }
        Ok(Self {
            stdin,
            child: Mutex::new(child),
        })
    }

    fn shutdown(mut self) {
        if let Some(mut input) = self.stdin.take() {
            let _ = input.write_all(b"{\"version\":1,\"kind\":\"stop\"}\n");
            let _ = input.flush();
        }
        let _ = wait_or_kill_tailcat_child(&self.child, TAILCAT_SHUTDOWN_GRACE);
    }
}

// --- OAuth PKCE / Device-Code Flow State ---

#[derive(Debug, Clone)]
struct OAuthFlowState {
    request_id: String,
    account_id: String,
    provider: String,
    method: i64,
    state: String,
    csrf_state: String,
    code_challenge: String,
    authorization_url: Option<String>,
    verification_url: Option<String>,
    user_code: Option<String>,
    callback_mode: Option<String>,
    instructions: Option<String>,
    poll_interval_seconds: Option<u64>,
    cancellation_confirmed: Option<bool>,
}

fn generate_pkce_pair() -> (String, String) {
    let verifier = base64url_encode(&random_bytes_32());
    let challenge = base64url_encode(&Sha256::digest(verifier.as_bytes()));
    (verifier, challenge)
}

struct WebServerState {
    store: Store,
    suite: Arc<ModulesSuite>,
    ui_dir: PathBuf,
    static_dir: PathBuf,
    loopback: bool,
    env_token_override: bool,
    host_id: String,
    access_token: String,
    sessions: Arc<Mutex<BTreeMap<String, i64>>>,
    handoffs: Arc<Mutex<HashMap<String, HandoffGrant>>>,
    nonces: Arc<Mutex<BoundedNonceCache>>,
    active_cancels: Arc<Mutex<BTreeMap<String, Arc<AtomicBool>>>>,
    pending_interactions: Arc<PendingInteractions>,
    native_browser: Arc<NativeBrowserBridge>,
    tool_plans: Arc<Mutex<BTreeMap<String, Value>>>,
    mcp_manager: Arc<McpManager>,
    tailcat: TailcatConfig,
    tailcat_leases: Arc<TailcatLeaseManager>,
    messaging_worker: Arc<Mutex<Option<Arc<AtomicBool>>>>,
    oauth_flows: Arc<Mutex<BTreeMap<String, OAuthFlowState>>>,
    auxiliary_ops: Arc<Mutex<BTreeMap<String, Value>>>,
}

struct RequestSlotGuard {
    limiter: Arc<(Mutex<usize>, Condvar)>,
}

impl Drop for RequestSlotGuard {
    fn drop(&mut self) {
        let (lock, cvar) = &*self.limiter;
        if let Ok(mut in_flight) = lock.lock() {
            *in_flight = in_flight.saturating_sub(1);
            cvar.notify_one();
        }
    }
}

fn acquire_request_slot(limiter: &Arc<(Mutex<usize>, Condvar)>) -> RequestSlotGuard {
    let (lock, cvar) = &**limiter;
    let mut in_flight = lock.lock().unwrap();
    while *in_flight >= MAX_CONCURRENT_REQUESTS {
        in_flight = cvar.wait(in_flight).unwrap();
    }
    *in_flight += 1;
    RequestSlotGuard {
        limiter: Arc::clone(limiter),
    }
}

fn stdin_is_pipe() -> bool {
    #[cfg(unix)]
    {
        // P0 macOS fix: `MacManagedNolaServer` launches `nola server --json` with stdin=/dev/null
        // (S_IFCHR). Only monitor stdin EOF when stdin is a real pipe or socketpair (S_IFIFO /
        // S_IFSOCK, e.g. Tauri/Node/Swift Pipe), never /dev/null, TTYs, or regular files.
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        let ok = unsafe { libc::fstat(0, &mut st) } == 0;
        let kind = st.st_mode & libc::S_IFMT;
        ok && (kind == libc::S_IFIFO || kind == libc::S_IFSOCK)
    }
    #[cfg(windows)]
    {
        const STD_INPUT_HANDLE: i32 = -10;
        const FILE_TYPE_PIPE: u32 = 3;
        extern "system" {
            fn GetStdHandle(nStdHandle: i32) -> *mut std::ffi::c_void;
            fn GetFileType(hFile: *mut std::ffi::c_void) -> u32;
        }
        unsafe {
            let h = GetStdHandle(STD_INPUT_HANDLE);
            !h.is_null()
                && h != (-1isize as *mut std::ffi::c_void)
                && GetFileType(h) == FILE_TYPE_PIPE
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        false
    }
}

fn has_expected_host_id(body: &Value) -> bool {
    body.get("expected_host_id")
        .and_then(|v| v.as_str())
        .is_some_and(|s| !s.trim().is_empty())
}

fn wrap_paired_result(store: &Store, host_id: &str, body: &Value, result: Value) -> Value {
    if has_expected_host_id(body) {
        let source_store_epoch = ensure_source_store_epoch(store);
        json!({
            "source_host_id": host_id,
            "source_store_epoch": source_store_epoch,
            "result": result
        })
    } else {
        result
    }
}

fn is_allowed_module_kind(kind: &str) -> bool {
    matches!(
        kind,
        "goal" | "sop" | "automation" | "data_source" | "agent_room" | "plugin" | "local_model"
    )
}

fn is_inbound_webhook_path(path: &str) -> bool {
    (path.starts_with("/api/messaging-channels/") && path.ends_with("/inbound"))
        || path.starts_with("/api/messaging/webhooks/")
        || path.starts_with("/v1/channels/webhook/")
        || path.starts_with("/api/v1/channels/webhook/")
}

fn reconcile_messaging_worker(
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

pub fn run_web_server(
    store: Store,
    suite: Arc<ModulesSuite>,
    args: &[String],
    mut json_mode: bool,
) -> Result<u8, String> {
    let mut host = "127.0.0.1".to_string();
    let mut port: u16 = 5001;
    let mut web_root: Option<PathBuf> = None;
    let mut assets_override: Option<PathBuf> = None;
    let mut tailcat_ingress = false;
    let mut tailcat_helper: Option<PathBuf> = None;
    let mut tailcat_helper_sha256: Option<String> = None;
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        if let Some(p_raw) = arg.strip_prefix("--port=") {
            port = p_raw
                .parse::<u16>()
                .map_err(|_| format!("invalid --port value: '{p_raw}'"))?;
            i += 1;
            continue;
        }
        if let Some(h) = arg.strip_prefix("--host=") {
            host = h.to_string();
            i += 1;
            continue;
        }
        if let Some(r) = arg.strip_prefix("--web-root=") {
            web_root = Some(PathBuf::from(r));
            i += 1;
            continue;
        }
        if let Some(a) = arg.strip_prefix("--assets=") {
            assets_override = Some(PathBuf::from(a));
            i += 1;
            continue;
        }
        if let Some(val) = arg.strip_prefix("--tailcat-ingress=") {
            tailcat_ingress = !matches!(
                val.trim().to_ascii_lowercase().as_str(),
                "0" | "false" | "no" | "off" | ""
            );
            i += 1;
            continue;
        }
        if let Some(val) = arg.strip_prefix("--tailcat-helper=") {
            let trimmed = val.trim();
            if !trimmed.is_empty() {
                tailcat_helper = Some(PathBuf::from(trimmed));
            }
            i += 1;
            continue;
        }
        if let Some(val) = arg.strip_prefix("--tailcat-helper-sha256=") {
            let trimmed = val.trim();
            if !trimmed.is_empty() {
                tailcat_helper_sha256 = Some(trimmed.to_string());
            }
            i += 1;
            continue;
        }
        match arg {
            "--host" => {
                i += 1;
                if let Some(h) = args.get(i) {
                    host = h.clone();
                }
            }
            "--port" => {
                i += 1;
                let Some(raw_port) = args.get(i) else {
                    return Err("missing value for --port".into());
                };
                port = raw_port
                    .parse::<u16>()
                    .map_err(|_| format!("invalid --port value: '{raw_port}'"))?;
            }
            "--web-root" => {
                i += 1;
                if let Some(r) = args.get(i) {
                    web_root = Some(PathBuf::from(r));
                }
            }
            "--assets" => {
                i += 1;
                if let Some(a) = args.get(i) {
                    assets_override = Some(PathBuf::from(a));
                }
            }
            "--tailcat-ingress" => {
                if let Some(next) = args.get(i + 1).filter(|s| !s.starts_with("--")) {
                    tailcat_ingress = !matches!(
                        next.trim().to_ascii_lowercase().as_str(),
                        "0" | "false" | "no" | "off"
                    );
                    i += 1;
                } else {
                    tailcat_ingress = true;
                }
            }
            "--tailcat-helper" => {
                i += 1;
                let Some(h) = args.get(i).filter(|s| !s.trim().is_empty()) else {
                    return Err("missing value for --tailcat-helper".into());
                };
                tailcat_helper = Some(PathBuf::from(h.trim()));
            }
            "--tailcat-helper-sha256" => {
                i += 1;
                let Some(digest) = args.get(i).filter(|s| !s.trim().is_empty()) else {
                    return Err("missing value for --tailcat-helper-sha256".into());
                };
                tailcat_helper_sha256 = Some(digest.trim().to_string());
            }
            "--json" => {
                json_mode = true;
            }
            _ => {}
        }
        i += 1;
    }

    if let Some(ref helper_path) = tailcat_helper {
        let _ = verified_tailcat_command(helper_path, tailcat_helper_sha256.as_deref())?;
    } else if tailcat_helper_sha256.is_some() {
        return Err("--tailcat-helper-sha256 requires --tailcat-helper".into());
    }

    let (ui_dir, static_dir) = if let Some(root) = web_root {
        let ui = root.join("app/static/ui");
        let st = root.join("app/static");
        if ui.is_dir() {
            (ui, st)
        } else {
            (root.clone(), root)
        }
    } else if let Some(assets) = assets_override {
        (assets.clone(), assets)
    } else {
        let built_ui = suite.workspace_root.join("apps/web/app/static/ui");
        let dist_ui = suite.workspace_root.join("apps/web/frontend/dist");
        let ui = if built_ui.is_dir() { built_ui } else { dist_ui };
        let st = suite.workspace_root.join("apps/web/app/static");
        (ui, st)
    };

    let addr = format!("{host}:{port}");
    let server = Arc::new(Server::http(&addr).map_err(|e| format!("failed to bind {addr}: {e}"))?);
    let bound_port = server
        .server_addr()
        .to_ip()
        .map(|s| s.port())
        .unwrap_or(port);
    let loopback = is_loopback_host(&host);
    let host_id = ensure_host_id(&store);
    let token_path = mobile_access_token_path();
    let (access_token, env_token_override) = load_or_create_web_access_token();
    let tailcat = TailcatConfig {
        ingress: tailcat_ingress,
        helper_path: tailcat_helper,
        helper_sha256: tailcat_helper_sha256,
        web_port: bound_port,
    };
    let tailcat_ingress_child = if tailcat.ingress && tailcat.helper_path.is_some() {
        Some(TailcatIngressChild::spawn(&tailcat, &host_id)?)
    } else {
        None
    };

    let shutdown_flag = Arc::new(AtomicBool::new(false));
    if json_mode && stdin_is_pipe() {
        let server_stdin = Arc::clone(&server);
        let flag_stdin = Arc::clone(&shutdown_flag);
        std::thread::spawn(move || {
            let mut stdin = std::io::stdin();
            let mut buf = [0u8; 256];
            loop {
                match stdin.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
            }
            flag_stdin.store(true, Ordering::SeqCst);
            server_stdin.unblock();
        });
    }
    #[cfg(unix)]
    {
        if let Ok(mut signals) = signal_hook::iterator::Signals::new([
            signal_hook::consts::SIGINT,
            signal_hook::consts::SIGTERM,
        ]) {
            let server_sig = Arc::clone(&server);
            let flag_sig = Arc::clone(&shutdown_flag);
            std::thread::spawn(move || {
                if signals.forever().next().is_some() {
                    flag_sig.store(true, Ordering::SeqCst);
                    server_sig.unblock();
                }
            });
        }
    }
    let sessions: Arc<Mutex<BTreeMap<String, i64>>> = Arc::new(Mutex::new(BTreeMap::new()));
    let handoffs: Arc<Mutex<HashMap<String, HandoffGrant>>> = Arc::new(Mutex::new(HashMap::new()));
    let nonces = Arc::new(Mutex::new(BoundedNonceCache::default()));
    let active_cancels: Arc<Mutex<BTreeMap<String, Arc<AtomicBool>>>> =
        Arc::new(Mutex::new(BTreeMap::new()));
    let pending_interactions = Arc::new(PendingInteractions::default());
    let native_browser = Arc::new(NativeBrowserBridge::default());
    let tool_plans: Arc<Mutex<BTreeMap<String, Value>>> = Arc::new(Mutex::new(BTreeMap::new()));
    let tailcat_leases = Arc::new(TailcatLeaseManager::default());
    let messaging_worker: Arc<Mutex<Option<Arc<AtomicBool>>>> = Arc::new(Mutex::new(None));
    let oauth_flows: Arc<Mutex<BTreeMap<String, OAuthFlowState>>> =
        Arc::new(Mutex::new(BTreeMap::new()));
    let auxiliary_ops: Arc<Mutex<BTreeMap<String, Value>>> = Arc::new(Mutex::new(BTreeMap::new()));

    // Reconcile enabled harness plugins on startup so plugin MCP servers & skills are wired cleanly.
    let _ = reconcile_enabled_plugins(&store, &suite.workspace_root);

    let vault_arc = Vault::open(resolve_vault_path_for_store(&store))
        .ok()
        .map(Arc::new);
    let mcp_manager = Arc::new(McpManager::new(vault_arc));

    // Spawn background automation scheduler so Web-created automations execute on schedule.
    let scheduler_cfg = load_active_config(&store);
    let scheduler_route = resolve_model_route(&scheduler_cfg).ok();
    let scheduler_llm = LlmClient::new(300).ok().map(Arc::new);
    let scheduler_stop = spawn_background_scheduler(
        store.clone(),
        suite.workspace_root.clone(),
        scheduler_llm,
        scheduler_route,
    );

    // Spawn durable background goal worker alongside the automation scheduler.
    let goal_worker_stop = Arc::new(AtomicBool::new(false));
    let store_for_goals = store.clone();
    let _goal_worker_handle = spawn_background_goal_worker(
        store.clone(),
        move || {
            let cfg = load_active_config(&store_for_goals);
            resolve_model_route(&cfg).ok()
        },
        suite.workspace_root.clone(),
        Arc::clone(&goal_worker_stop),
    );

    // Start messaging channel worker if enabled in persisted KV settings.
    let _ = reconcile_messaging_worker(&messaging_worker, &store, &suite.workspace_root);

    if json_mode {
        println!(
            "{}",
            server_started_handshake_full(&host, bound_port, &host_id, &access_token, &token_path)
        );
    } else {
        println!("Nola v2 WebHost listening on http://{host}:{bound_port}");
    }
    let _ = std::io::stdout().flush();

    let state = Arc::new(WebServerState {
        store,
        suite,
        ui_dir,
        static_dir,
        loopback,
        env_token_override,
        host_id,
        access_token,
        sessions,
        handoffs,
        nonces,
        active_cancels,
        pending_interactions,
        native_browser,
        tool_plans,
        mcp_manager,
        tailcat,
        tailcat_leases: Arc::clone(&tailcat_leases),
        messaging_worker: Arc::clone(&messaging_worker),
        oauth_flows,
        auxiliary_ops,
    });
    let request_limiter = Arc::new((Mutex::new(0usize), Condvar::new()));

    for req in server.incoming_requests() {
        if shutdown_flag.load(Ordering::SeqCst) {
            break;
        }
        let slot = acquire_request_slot(&request_limiter);
        let st = Arc::clone(&state);
        let _ = std::thread::Builder::new()
            .name("nola-http-worker".into())
            .spawn(move || {
                let _slot_guard = slot;
                handle_http_request(req, st);
            });
    }

    scheduler_stop.store(true, Ordering::Relaxed);
    goal_worker_stop.store(true, Ordering::Relaxed);
    if let Ok(mut guard) = messaging_worker.lock() {
        if let Some(stop) = guard.take() {
            stop.store(true, Ordering::Relaxed);
        }
    }
    tailcat_leases.shutdown_all();
    if let Some(ingress_child) = tailcat_ingress_child {
        ingress_child.shutdown();
    }
    Ok(0)
}

fn handle_http_request(mut req: Request, state: Arc<WebServerState>) {
    let store = state.store.clone();
    let suite = Arc::clone(&state.suite);
    let ui_dir = &state.ui_dir;
    let static_dir = &state.static_dir;
    let loopback = state.loopback;
    let env_token_override = state.env_token_override;
    let host_id = &state.host_id;
    let access_token = &state.access_token;
    let sessions = &state.sessions;
    let handoffs = &state.handoffs;
    let nonces = &state.nonces;
    let active_cancels = &state.active_cancels;
    let pending_interactions = &state.pending_interactions;
    let native_browser = &state.native_browser;
    let tool_plans = &state.tool_plans;
    let mcp_manager = &state.mcp_manager;
    let tailcat = &state.tailcat;
    let tailcat_leases = &state.tailcat_leases;
    let messaging_worker = &state.messaging_worker;
    let oauth_flows = &state.oauth_flows;
    let auxiliary_ops = &state.auxiliary_ops;

    let full_url = req.url().to_string();
    let (raw_path, query) = full_url.split_once('?').unwrap_or((full_url.as_str(), ""));
    let normalized_path = if let Some(rest) = raw_path.strip_prefix("/api/mobile/") {
        format!("/api/{rest}")
    } else if raw_path == "/api/mobile" {
        "/api".to_string()
    } else {
        raw_path.to_string()
    };
    let path = normalized_path.as_str();
    let method = req.method().clone();
    let method_str = method_as_str(&method);

    let is_gateway_route = path.starts_with("/gateway/v1/");
    let is_native_browser_host_route = matches!(
        path,
        "/api/native-browser/host/poll" | "/api/native-browser/host/result"
    );
    let is_inbound_webhook_route = is_inbound_webhook_path(path);

    if is_native_browser_host_route {
        if !validate_native_browser_token(&req) {
            let _ = respond_json(
                req,
                401,
                &json!({"error": "unauthorized_native_browser_host"}),
            );
            return;
        }
    } else if !is_gateway_route && !is_inbound_webhook_route {
        if let Err(reason) = validate_same_origin(&req) {
            let _ = respond_json(req, 403, &json!({"error": "csrf_blocked", "message": reason}));
            return;
        }
    }

    // SEC-1: Check `x-nola-nonce` independently (NEVER `.or_else(|| x-nola-mobile-nonce)`).
    // Mobile HMAC nonces are checked & recorded ONLY AFTER signature + timestamp validation succeeds.
    if let Some(nonce) = header_value(&req, "x-nola-nonce")
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
    {
        if !nonces.lock().unwrap().record_if_fresh(format!("hdr:{nonce}")) {
            let _ = respond_json(req, 409, &json!({"error": "replayed_nonce"}));
            return;
        }
    }

    let raw_body = read_raw_body(&mut req);
    let is_public_api = matches!(
        path,
        "/api/health" | "/api/web/session" | "/api/web/login" | "/api/login"
    ) || is_native_browser_host_route
        || is_inbound_webhook_route;

    let (req_authenticated, replayed_mobile_nonce) = authenticate_request(
        &req,
        method_str,
        &full_url,
        &raw_body,
        access_token,
        sessions,
        nonces,
    );
    if replayed_mobile_nonce {
        let _ = respond_json(req, 409, &json!({"error": "replayed_nonce"}));
        return;
    }

    // SEC-2: Implicit loopback trust applies ONLY to direct local Web UI requests on loopback
    // when NOLA_WEB_ACCESS_TOKEN is not set. Never bypass auth on `/api/mobile/*` or when
    // explicit auth credentials (`Authorization` / `X-Nola-Mobile-Auth`) were presented.
    let has_explicit_auth = header_value(&req, "x-nola-mobile-auth").is_some()
        || header_value(&req, "authorization").is_some();
    let local_trust = compute_local_trust(loopback, env_token_override, raw_path, has_explicit_auth);
    if !local_trust && path.starts_with("/api/") && !is_public_api && !req_authenticated {
        let _ = respond_json(req, 401, &json!({"error": "unauthorized"}));
        return;
    }

    let use_mobile_crypto = wants_mobile_crypto(&req);
    let crypto_tok = use_mobile_crypto.then_some(access_token.as_str());
    let body = if use_mobile_crypto && !raw_body.is_empty() {
        match maybe_decrypt_mobile_body(&raw_body, access_token) {
            Ok(v) => v,
            Err(e) => {
                let _ = respond_json(
                    req,
                    401,
                    &json!({"error": "invalid_mobile_crypto", "message": e}),
                );
                return;
            }
        }
    } else {
        parse_json_or_empty(&raw_body)
    };

    // --- Model Gateway Reverse Proxy (/gateway/v1/*) ---
    if is_gateway_route {
        handle_gateway_route(req, &store, &suite, &method, path, query, &raw_body);
        return;
    }

    match (method.clone(), path) {
            // --- Health, Session, Auth & Software Update ---
            (Method::Get, "/api/health") => {
                let _ = respond_api_json(
                    req,
                    200,
                    &json!({
                        "ok": true,
                        "version": "2.0.0",
                        "service": "nola-v2",
                        "mode": "web",
                        "host": {
                            "id": host_id,
                            "permissionProtocolVersion": 1
                        }
                    }),
                    crypto_tok,
                );
            }
            (Method::Get, "/api/me") => {
                let authed = local_trust || req_authenticated;
                let _ = respond_api_json(
                    req,
                    200,
                    &json!({
                        "authenticated": authed,
                        "hostId": host_id,
                        "version": "2.0.0",
                        "workspaceRoot": suite.workspace_root.display().to_string()
                    }),
                    crypto_tok,
                );
            }
            (Method::Get, "/api/web/session") => {
                let session_auth = is_valid_session_cookie(&req, sessions);
                let bearer_auth = !session_auth && req_authenticated;
                let authed = session_auth || local_trust || bearer_auth;
                let desktop_platform = std::env::var("NOLA_DESKTOP_PLATFORM")
                    .ok()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty());
                let _ = respond_api_json(
                    req,
                    200,
                    &json!({
                        "authenticated": authed,
                        "sessionAuthenticated": session_auth,
                        "bearerAuthenticated": bearer_auth,
                        "localTrust": local_trust,
                        "tailcatAvailable": tailcat.is_available(),
                        "transportIngress": if tailcat.ingress { "tailcat" } else { "direct" },
                        "desktopPlatform": desktop_platform,
                        "hostId": host_id,
                        "version": "2.0.0"
                    }),
                    crypto_tok,
                );
            }
            (Method::Post, "/api/transport/tailcat") => {
                let owner_tok = extract_bearer_token(&req)
                    .map(ToString::to_string)
                    .or_else(|| extract_cookie_value(&req, "nola_session"))
                    .unwrap_or_else(|| access_token.clone());
                let owner = sha256_hex(owner_tok.as_bytes());
                let op = body
                    .get("operation")
                    .or_else(|| body.get("action"))
                    .and_then(|v| v.as_str())
                    .map(str::trim);
                let result = match op {
                    Some("connect") | Some("grant") => {
                        let expected = body
                            .get("expectedHostId")
                            .or_else(|| body.get("expected_host_id"))
                            .and_then(|v| v.as_str());
                        if let Some(exp) = expected {
                            if exp != host_id.as_str() {
                                Err(TailcatLeaseError::new(
                                    409,
                                    "transport_host_identity_mismatch",
                                ))
                            } else if let Some(public) = body
                                .get("clientPublicKey")
                                .or_else(|| body.get("client_public_key"))
                                .and_then(|v| v.as_str())
                            {
                                tailcat_leases.create_lease(tailcat, host_id, owner, public)
                            } else {
                                Err(TailcatLeaseError::new(400, "client_public_key_required"))
                            }
                        } else if let Some(public) = body
                            .get("clientPublicKey")
                            .or_else(|| body.get("client_public_key"))
                            .and_then(|v| v.as_str())
                        {
                            tailcat_leases.create_lease(tailcat, host_id, owner, public)
                        } else {
                            Err(TailcatLeaseError::new(400, "client_public_key_required"))
                        }
                    }
                    Some("renew") => body
                        .get("leaseId")
                        .or_else(|| body.get("lease_id"))
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| TailcatLeaseError::new(400, "transport_lease_required"))
                        .and_then(|lease_id| tailcat_leases.renew_lease(&owner, lease_id)),
                    Some("close") => body
                        .get("leaseId")
                        .or_else(|| body.get("lease_id"))
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| TailcatLeaseError::new(400, "transport_lease_required"))
                        .and_then(|lease_id| tailcat_leases.close_lease(&owner, lease_id)),
                    _ => Err(TailcatLeaseError::new(400, "unknown_transport_operation")),
                };
                match result {
                    Ok(value) => {
                        let _ = respond_api_json(req, 200, &value, crypto_tok);
                    }
                    Err(err) => {
                        let payload = match err.detail {
                            Some(detail) => {
                                json!({"error": err.code, "code": err.code, "detail": detail})
                            }
                            None => json!({"error": err.code, "code": err.code}),
                        };
                        let _ = respond_api_json(req, err.status, &payload, crypto_tok);
                    }
                }
            }
            (Method::Post, "/api/web/login") | (Method::Post, "/api/login") => {
                let supplied_token = body
                    .get("token")
                    .or_else(|| body.get("access_token"))
                    .or_else(|| body.get("accessToken"))
                    .or_else(|| body.get("password"))
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty());

                let authorized = match supplied_token {
                    Some(tok) => ct_eq(tok, access_token),
                    None => local_trust,
                };

                if !authorized {
                    let _ = respond_api_json(
                        req,
                        401,
                        &json!({
                            "authenticated": false,
                            "sessionAuthenticated": false,
                            "error": "invalid_token"
                        }),
                        crypto_tok,
                    );
                } else {
                    let session_tok = issue_browser_session(sessions);
                    let cookie_val = format!(
                        "nola_session={session_tok}; HttpOnly; SameSite=Strict; Path=/"
                    );
                    let set_cookie =
                        Header::from_bytes(&b"Set-Cookie"[..], cookie_val.as_bytes()).unwrap();
                    let payload = json!({
                        "authenticated": true,
                        "sessionAuthenticated": true,
                        "hostId": host_id,
                        "token": session_tok,
                        "token_type": "Bearer",
                        "expires_in": SESSION_TTL_MS / 1000
                    });
                    let _ = respond_api_json_with_headers(
                        req,
                        200,
                        &payload,
                        vec![set_cookie],
                        crypto_tok,
                    );
                }
            }
            (Method::Post, "/api/web/logout") => {
                {
                    let mut map = sessions.lock().unwrap();
                    if let Some(tok) = extract_cookie_value(&req, "nola_session") {
                        map.remove(&tok);
                    }
                    if let Some(bearer) = extract_bearer_token(&req) {
                        map.remove(bearer);
                    }
                }
                let clear_cookie = Header::from_bytes(
                    &b"Set-Cookie"[..],
                    &b"nola_session=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0"[..],
                )
                .unwrap();
                let _ = respond_api_json_with_headers(
                    req,
                    200,
                    &json!({"authenticated": false, "sessionAuthenticated": false}),
                    vec![clear_cookie],
                    crypto_tok,
                );
            }
            (Method::Get, "/api/software-update") | (Method::Post, "/api/software-update/check") => {
                // SEC-4: Ignore untrusted `feed_url` query/body overrides; only check server-configured feed.
                let feed_url = std::env::var("NOLA_UPDATE_FEED_URL")
                    .ok()
                    .map(|s| s.trim().to_string())
                    .unwrap_or_default();
                match check_for_update(&feed_url, "2.0.0") {
                    Ok(res) => {
                        let release = res.latest.as_ref().map(|m| {
                            let release_id = format!(
                                "rel-{}-{}",
                                m.version,
                                &m.sha256[..m.sha256.len().min(12)]
                            );
                            let filename = Url::parse(&m.download_url)
                                .ok()
                                .and_then(|u| {
                                    u.path_segments()
                                        .and_then(|mut s| s.next_back().map(ToString::to_string))
                                })
                                .filter(|s| !s.trim().is_empty())
                                .unwrap_or_else(|| format!("nola-{}.bin", m.version));
                            json!({
                                "release_id": release_id,
                                "version": m.version,
                                "build": 1,
                                "notes": m.notes,
                                "artifact": {
                                    "kind": "archive",
                                    "url": m.download_url,
                                    "filename": filename,
                                    "size": 0,
                                    "sha256": m.sha256
                                }
                            })
                        });
                        if let Some(ref rel) = release {
                            let _ = store.set_kv("latest_update_release", rel);
                        }
                        let _ = respond_api_json(
                            req,
                            200,
                            &json!({
                                "status": res.status,
                                "current_version": res.current_version,
                                "current_build": 1,
                                "channel": "stable",
                                "target": res.target,
                                "update_available": res.update_available,
                                "release": release
                            }),
                            crypto_tok,
                        );
                    }
                    Err(e) => {
                        let _ = respond_api_json(
                            req,
                            502,
                            &json!({
                                "status": "error",
                                "error": e,
                                "current_version": "2.0.0",
                                "target": nola_modules::current_update_target()
                            }),
                            crypto_tok,
                        );
                    }
                }
            }
            (Method::Get, "/api/software-update/package") => {
                // SEC-4: Ignore arbitrary `url`/`sha256` query params; ONLY allow `?release_id=`
                // matching the server-cached release in KV `latest_update_release` with non-empty sha256!
                let requested_release_id = parse_query_str(query, "release_id")
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty());
                let Some(req_rel_id) = requested_release_id else {
                    let _ = respond_api_json(
                        req,
                        400,
                        &json!({"error": "missing_release_id", "message": "release_id query parameter is required"}),
                        crypto_tok,
                    );
                    return;
                };
                let saved_rel = store.get_kv("latest_update_release").ok().flatten();
                let Some(saved_rel) = saved_rel else {
                    let _ = respond_api_json(
                        req,
                        404,
                        &json!({"error": "release_not_found", "message": "no cached update release available"}),
                        crypto_tok,
                    );
                    return;
                };
                let saved_id = saved_rel
                    .get("release_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if saved_id != req_rel_id {
                    let _ = respond_api_json(
                        req,
                        404,
                        &json!({"error": "release_id_mismatch", "message": "requested release_id does not match cached update release"}),
                        crypto_tok,
                    );
                    return;
                }
                let pkg_url = saved_rel
                    .pointer("/artifact/url")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .unwrap_or("")
                    .to_string();
                let sha256 = saved_rel
                    .pointer("/artifact/sha256")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(ToString::to_string);
                let mut filename = saved_rel
                    .pointer("/artifact/filename")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let Some(expected_sha) = sha256 else {
                    let _ = respond_api_json(
                        req,
                        400,
                        &json!({"error": "missing_release_sha256", "message": "cached release is missing mandatory sha256 digest"}),
                        crypto_tok,
                    );
                    return;
                };
                if pkg_url.is_empty() {
                    let _ = respond_api_json(
                        req,
                        400,
                        &json!({"error": "update_package_unconfigured", "message": "cached release is missing artifact url"}),
                        crypto_tok,
                    );
                    return;
                }
                let dest_dir = default_home_dir().join(".nola/updates");
                match download_update_package(&pkg_url, Some(&expected_sha), &dest_dir) {
                    Ok(receipt) => {
                        let saved_path = receipt
                            .get("path")
                            .and_then(|v| v.as_str())
                            .map(PathBuf::from);
                        let Some(file_path) = saved_path else {
                            let _ = respond_api_json(
                                req,
                                500,
                                &json!({"ok": false, "error": "downloaded package path missing"}),
                                crypto_tok,
                            );
                            return;
                        };
                        if filename.is_empty() {
                            filename = file_path
                                .file_name()
                                .and_then(|n| n.to_str())
                                .unwrap_or("nola-update.bin")
                                .to_string();
                        }
                        let safe_filename: String = filename
                            .chars()
                            .filter(|c| {
                                c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')
                            })
                            .collect();
                        let disp_name = if safe_filename.is_empty() {
                            "nola-update.bin".to_string()
                        } else {
                            safe_filename
                        };
                        match fs::File::open(&file_path) {
                            Ok(file) => {
                                let ct = Header::from_bytes(
                                    &b"Content-Type"[..],
                                    &b"application/octet-stream"[..],
                                )
                                .unwrap();
                                let cd_val = format!("attachment; filename=\"{disp_name}\"");
                                let cd = Header::from_bytes(
                                    &b"Content-Disposition"[..],
                                    cd_val.as_bytes(),
                                )
                                .unwrap();
                                let resp = Response::from_file(file)
                                    .with_status_code(StatusCode(200))
                                    .with_header(ct)
                                    .with_header(cd);
                                let _ = req.respond(resp);
                            }
                            Err(e) => {
                                let _ = respond_api_json(
                                    req,
                                    500,
                                    &json!({"ok": false, "error": format!("failed opening downloaded package: {e}")}),
                                    crypto_tok,
                                );
                            }
                        }
                    }
                    Err(e) => {
                        let _ = respond_api_json(
                            req,
                            400,
                            &json!({"ok": false, "error": e}),
                            crypto_tok,
                        );
                    }
                }
            }

            // --- iOS / macOS REST Parity: Tool Plan, Web Handoff, Attachments Parse ---
            (Method::Post, "/api/agent/tool-plan/prepare") => {
                let mut reg = ToolRegistry::with_builtin_tools();
                suite.register_tools(&mut reg);
                let specs = reg.all_specs();
                let manifest_id = new_id();
                let issued_at_ms = now_ms();
                let expires_at_ms = issued_at_ms.saturating_add(300_000);
                let client_id = body
                    .get("client_id")
                    .or_else(|| body.get("clientId"))
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or("nola-client");
                let session_id = body
                    .get("session_id")
                    .or_else(|| body.get("sessionId"))
                    .or_else(|| body.get("conversation_id"))
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or("default-session");
                let paired = has_expected_host_id(&body);
                let plan = if paired {
                    let tools: Vec<Value> = specs
                        .iter()
                        .map(|s| {
                            json!({
                                "tool_id": s.schema.name,
                                "route": "agent_host",
                                "schema": {
                                    "type": "function",
                                    "function": {
                                        "name": s.schema.name,
                                        "description": s.schema.description,
                                        "parameters": s.schema.parameters
                                    }
                                }
                            })
                        })
                        .collect();
                    let catalog_bytes = serde_json::to_vec(&tools).unwrap_or_default();
                    let catalog_revision = sha256_hex(&catalog_bytes);
                    json!({
                        "schema_version": 1,
                        "reference": {
                            "manifest_id": manifest_id,
                            "host_id": host_id,
                            "client_id": client_id,
                            "session_id": session_id,
                            "catalog_revision": catalog_revision
                        },
                        "issued_at_ms": issued_at_ms,
                        "expires_at_ms": expires_at_ms,
                        "tools": tools
                    })
                } else {
                    let flat: Vec<Value> = specs
                        .iter()
                        .map(|s| {
                            json!({
                                "name": s.schema.name,
                                "description": s.schema.description,
                                "parameters": s.schema.parameters,
                                "tier": s.tier,
                                "domain": s.domain,
                                "read_only": s.read_only
                            })
                        })
                        .collect();
                    json!({
                        "manifest_id": manifest_id,
                        "catalog_revision": 1,
                        "ttl_seconds": 300,
                        "tools": flat
                    })
                };
                {
                    let mut plans = tool_plans.lock().unwrap();
                    if plans.len() >= 128 {
                        if let Some(first) = plans.keys().next().cloned() {
                            plans.remove(&first);
                        }
                    }
                    plans.insert(manifest_id, plan.clone());
                }
                let payload = wrap_paired_result(&store, host_id, &body, plan);
                let _ = respond_api_json(req, 200, &payload, crypto_tok);
            }
            (Method::Post, "/api/agent/tool-plan/read") => {
                let mid = body
                    .get("manifest_id")
                    .or_else(|| body.get("manifestId"))
                    .or_else(|| body.pointer("/reference/manifest_id"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let paired = has_expected_host_id(&body);
                let cached = tool_plans.lock().unwrap().get(mid).cloned();
                let plan = if let Some(existing) = cached {
                    if paired && existing.get("schema_version").is_none() {
                        // Rebuild in strict paired schema if cached plan was legacy flat format
                        build_paired_tool_plan(&suite, host_id, &body, if mid.is_empty() { new_id() } else { mid.to_string() })
                    } else {
                        existing
                    }
                } else if paired {
                    build_paired_tool_plan(
                        &suite,
                        host_id,
                        &body,
                        if mid.is_empty() {
                            new_id()
                        } else {
                            mid.to_string()
                        },
                    )
                } else {
                    let mut reg = ToolRegistry::with_builtin_tools();
                    suite.register_tools(&mut reg);
                    let flat: Vec<Value> = reg
                        .all_specs()
                        .iter()
                        .map(|s| {
                            json!({
                                "name": s.schema.name,
                                "description": s.schema.description,
                                "parameters": s.schema.parameters,
                                "tier": s.tier,
                                "domain": s.domain,
                                "read_only": s.read_only
                            })
                        })
                        .collect();
                    json!({
                        "manifest_id": if mid.is_empty() { new_id() } else { mid.to_string() },
                        "catalog_revision": 1,
                        "ttl_seconds": 300,
                        "tools": flat
                    })
                };
                let payload = wrap_paired_result(&store, host_id, &body, plan);
                let _ = respond_api_json(req, 200, &payload, crypto_tok);
            }
            (Method::Post, "/api/web-handoff") => {
                let raw_target = body
                    .get("target")
                    .and_then(|v| v.as_str())
                    .unwrap_or("/settings");
                let (code, safe_path) = mint_web_handoff(handoffs, raw_target, Instant::now());
                let handoff_path = format!("/auth/mobile-handoff/{code}");
                let _ = respond_api_json(
                    req,
                    200,
                    &json!({
                        "ok": true,
                        "path": handoff_path,
                        "code": code,
                        "target": safe_path,
                        "expiresIn": HANDOFF_TTL_SECS,
                        "hostId": host_id
                    }),
                    crypto_tok,
                );
            }
            (Method::Get, p) if p.starts_with("/auth/mobile-handoff/") => {
                let code = p.trim_start_matches("/auth/mobile-handoff/");
                let no_store =
                    Header::from_bytes(&b"Cache-Control"[..], &b"no-store"[..]).unwrap();
                if let Some(target) = consume_web_handoff(handoffs, code, Instant::now()) {
                    let session_tok = issue_browser_session(sessions);
                    let cookie_val = format!(
                        "nola_session={session_tok}; HttpOnly; SameSite=Strict; Path=/"
                    );
                    let set_cookie =
                        Header::from_bytes(&b"Set-Cookie"[..], cookie_val.as_bytes()).unwrap();
                    let safe_loc = safe_handoff_target(&target);
                    let loc = Header::from_bytes(&b"Location"[..], safe_loc.as_bytes())
                        .unwrap_or_else(|_| {
                            Header::from_bytes(&b"Location"[..], &b"/settings"[..]).unwrap()
                        });
                    let resp = Response::empty(StatusCode(302))
                        .with_header(loc)
                        .with_header(set_cookie)
                        .with_header(no_store);
                    let _ = req.respond(resp);
                } else {
                    let loc = Header::from_bytes(
                        &b"Location"[..],
                        &b"/login?error=expired_handoff"[..],
                    )
                    .unwrap();
                    let resp = Response::empty(StatusCode(302))
                        .with_header(loc)
                        .with_header(no_store);
                    let _ = req.respond(resp);
                }
            }
            (Method::Post, "/api/attachments/parse") => {
                let filename = body
                    .get("filename")
                    .or_else(|| body.get("name"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("attachment");
                let mime_type = body
                    .get("mime_type")
                    .or_else(|| body.get("mimeType"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("application/octet-stream");
                let b64 = body
                    .get("data_base64")
                    .or_else(|| body.get("dataBase64"))
                    .or_else(|| body.get("base64"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                match base64_decode(b64) {
                    Ok(bytes) => {
                        let parsed = parse_attachment_payload(filename, mime_type, &bytes);
                        let _ = respond_api_json(req, 200, &parsed, crypto_tok);
                    }
                    Err(e) => {
                        let _ = respond_api_json(
                            req,
                            400,
                            &json!({"error": "invalid_base64", "message": e}),
                            crypto_tok,
                        );
                    }
                }
            }

            // --- Dashboard, Home, Today, Activity, Results ---
            (Method::Get, "/api/home") => {
                let _ = respond_api_json(req, 200, &build_dashboard_snapshot(&suite), crypto_tok);
            }
            (Method::Get, "/api/workspace/today") => {
                let offset = parse_query_i64(query, "offset").unwrap_or(0).max(0) as usize;
                let sessions_list = store.list_sessions(50).unwrap_or_default();
                let active_ids: HashSet<String> = active_cancels
                    .lock()
                    .unwrap()
                    .keys()
                    .cloned()
                    .collect();
                let to_fact = |s: &Session| {
                    let needs_resp = pending_interactions.conversation_needs_response(&s.id);
                    let active_run = if active_ids.contains(&s.id) {
                        json!({
                            "run_id": format!("run-{}", s.id),
                            "assistant_message_id": format!("msg-active-{}", s.id),
                            "conversation_id": s.id,
                            "status": if needs_resp { "waiting_for_input" } else { "running" }
                        })
                    } else {
                        Value::Null
                    };
                    json!({
                        "conversation_id": s.id,
                        "title": s.title,
                        "updated_at_ms": s.updated_at_ms,
                        "needs_response": needs_resp,
                        "active_run": active_run
                    })
                };
                let page_sessions: Vec<Value> =
                    sessions_list.iter().skip(offset).take(20).map(to_fact).collect();
                let recent_sessions: Vec<Value> =
                    sessions_list.iter().take(5).map(to_fact).collect();
                let has_more = sessions_list.len() > offset + page_sessions.len();
                let next_offset = if has_more {
                    json!(offset + page_sessions.len())
                } else {
                    Value::Null
                };
                let needs_attention = pending_interactions.any_needs_response();
                let _ = respond_api_json(
                    req,
                    200,
                    &json!({
                        "source_host_id": host_id,
                        "snapshot": {
                            "conversations": page_sessions,
                            "recent_conversations": recent_sessions,
                            "queued_instances": [],
                            "needs_attention": needs_attention,
                            "unrepresented_expert_contexts": [],
                            "has_more_conversations": has_more,
                            "next_conversation_offset": next_offset,
                            "has_more_queued_instances": false
                        }
                    }),
                    crypto_tok,
                );
            }
            (Method::Post, "/api/activity") => {
                let page = build_activity_page(
                    &store,
                    &suite,
                    &active_cancels,
                    &pending_interactions,
                    &body,
                );
                let _ = respond_api_json(req, 200, &page, crypto_tok);
            }
            (Method::Post, "/api/activity/detail") => {
                let detail = build_activity_detail(
                    &store,
                    &suite,
                    &active_cancels,
                    &pending_interactions,
                    &host_id,
                    &body,
                );
                let _ = respond_api_json(req, 200, &detail, crypto_tok);
            }
            (Method::Get, "/api/results") => {
                let results: Vec<Value> = suite.list_records("automation_result").unwrap_or_default();
                let _ = respond_api_json(req, 200, &json!({"results": results}), crypto_tok);
            }
            (Method::Get, p)
                if p.starts_with("/api/results/")
                    && !p.trim_start_matches("/api/results/").contains('/') =>
            {
                let id = percent_decode_path(p.trim_start_matches("/api/results/"));
                if let Ok(Some(rec)) = suite.get_record::<Value>("automation_result", &id) {
                    let _ = respond_api_json(req, 200, &rec, crypto_tok);
                } else {
                    let _ = respond_api_json(req, 404, &json!({"error": "result_not_found"}), crypto_tok);
                }
            }
            (Method::Get, "/api/documents/search") => {
                let q = parse_query_str(query, "q")
                    .or_else(|| parse_query_str(query, "query"))
                    .unwrap_or_default();
                let limit = parse_query_i64(query, "limit")
                    .unwrap_or(20)
                    .clamp(1, 100) as usize;
                let source_ids: Option<Vec<String>> = parse_query_str(query, "sourceId")
                    .or_else(|| parse_query_str(query, "source_id"))
                    .map(|s| {
                        s.split(',')
                            .map(str::trim)
                            .filter(|x| !x.is_empty())
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                    })
                    .filter(|v| !v.is_empty());
                let res = execute_documents_search(
                    &store,
                    &suite.workspace_root,
                    &q,
                    source_ids.as_deref(),
                    limit,
                );
                let _ = respond_api_json(req, 200, &res, crypto_tok);
            }
            (Method::Post, "/api/documents/search") => {
                let q = body
                    .get("query")
                    .or_else(|| body.get("q"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let limit = body
                    .get("limit")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(20)
                    .clamp(1, 100) as usize;
                let source_ids: Option<Vec<String>> = body
                    .get("sourceIds")
                    .or_else(|| body.get("source_ids"))
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|x| x.as_str().map(str::trim).filter(|s| !s.is_empty()).map(ToString::to_string))
                            .collect::<Vec<_>>()
                    })
                    .filter(|v| !v.is_empty())
                    .or_else(|| {
                        body.get("sourceId")
                            .or_else(|| body.get("source_id"))
                            .and_then(|v| v.as_str())
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .map(|s| vec![s.to_string()])
                    });
                let res = execute_documents_search(
                    &store,
                    &suite.workspace_root,
                    q,
                    source_ids.as_deref(),
                    limit,
                );
                let _ = respond_api_json(req, 200, &res, crypto_tok);
            }
            (Method::Get, "/api/workspace/worktrees") => {
                let (status, payload) =
                    handle_workspace_worktrees(&suite.workspace_root, false, &json!({}));
                let _ = respond_api_json(req, status, &payload, crypto_tok);
            }
            (Method::Post, "/api/workspace/worktrees") => {
                let (status, payload) =
                    handle_workspace_worktrees(&suite.workspace_root, true, &body);
                let _ = respond_api_json(req, status, &payload, crypto_tok);
            }
            (Method::Get, "/api/workspace/diff") => {
                let review = build_workspace_review(&host_id, "workspace", &suite.workspace_root);
                let _ = respond_api_json(req, 200, &review, crypto_tok);
            }
            (Method::Get, "/api/workspace/checkpoints") => {
                let checkpoints: Vec<Value> = suite.list_records("workspace_checkpoint").unwrap_or_default();
                let _ = respond_api_json(req, 200, &json!({"checkpoints": checkpoints}), crypto_tok);
            }
            (Method::Post, "/api/workspace/undo") => {
                let out = Command::new("git")
                    .args(["stash", "push", "-u", "-m", "nola-v2-undo"])
                    .current_dir(&suite.workspace_root)
                    .output();
                match out {
                    Ok(o) if o.status.success() => {
                        let _ = respond_api_json(
                            req,
                            200,
                            &json!({"ok": true, "stdout": String::from_utf8_lossy(&o.stdout)}),
                            crypto_tok,
                        );
                    }
                    Ok(o) => {
                        let _ = respond_api_json(
                            req,
                            400,
                            &json!({"ok": false, "error": String::from_utf8_lossy(&o.stderr)}),
                            crypto_tok,
                        );
                    }
                    Err(e) => {
                        let _ = respond_api_json(req, 500, &json!({"ok": false, "error": e.to_string()}), crypto_tok);
                    }
                }
            }
            (Method::Post, "/api/workspace/redo") => {
                let out = Command::new("git")
                    .args(["stash", "pop"])
                    .current_dir(&suite.workspace_root)
                    .output();
                match out {
                    Ok(o) if o.status.success() => {
                        let _ = respond_api_json(
                            req,
                            200,
                            &json!({"ok": true, "stdout": String::from_utf8_lossy(&o.stdout)}),
                            crypto_tok,
                        );
                    }
                    Ok(o) => {
                        let _ = respond_api_json(
                            req,
                            400,
                            &json!({"ok": false, "error": String::from_utf8_lossy(&o.stderr)}),
                            crypto_tok,
                        );
                    }
                    Err(e) => {
                        let _ = respond_api_json(req, 500, &json!({"ok": false, "error": e.to_string()}), crypto_tok);
                    }
                }
            }
            (Method::Post, "/api/harness-plugins") | (Method::Post, "/api/harness/plugins") => {
                let action = body
                    .get("action")
                    .and_then(|v| v.as_str())
                    .unwrap_or("library")
                    .trim();
                match action {
                    "install" => {
                        let source = body
                            .get("source")
                            .or_else(|| body.get("path"))
                            .or_else(|| body.get("name"))
                            .and_then(|v| v.as_str())
                            .map(str::trim)
                            .unwrap_or("");
                        if source.is_empty() {
                            let _ = respond_api_json(
                                req,
                                400,
                                &json!({"ok": false, "error": "missing_plugin_source"}),
                                crypto_tok,
                            );
                        } else {
                            let runtime = body
                                .get("runtime")
                                .and_then(|v| v.as_str())
                                .unwrap_or("dsh");
                            let profile = body
                                .get("profile")
                                .and_then(|v| v.as_str())
                                .unwrap_or("web");
                            match install_plugin_package(
                                &store,
                                &suite.workspace_root,
                                runtime,
                                profile,
                                source,
                            ) {
                                Ok(rec) => {
                                    let reconciled =
                                        reconcile_enabled_plugins(&store, &suite.workspace_root);
                                    let mut catalog =
                                        list_harness_plugins(Some(&store), &suite.workspace_root);
                                    if let Some(obj) = catalog.as_object_mut() {
                                        obj.insert("ok".into(), json!(true));
                                        obj.insert("installed".into(), serde_json::to_value(&rec).unwrap_or(Value::Null));
                                        obj.insert("reconciled".into(), serde_json::to_value(&reconciled).unwrap_or(Value::Null));
                                    }
                                    let _ = respond_api_json(req, 200, &catalog, crypto_tok);
                                }
                                Err(e) => {
                                    let _ = respond_api_json(
                                        req,
                                        400,
                                        &json!({"ok": false, "error": e}),
                                        crypto_tok,
                                    );
                                }
                            }
                        }
                    }
                    "enable" | "disable" | "toggle" => {
                        let target = body
                            .get("id")
                            .or_else(|| body.get("source"))
                            .or_else(|| body.get("name"))
                            .and_then(|v| v.as_str())
                            .map(str::trim)
                            .unwrap_or("");
                        let explicit_enabled = match action {
                            "enable" => Some(true),
                            "disable" => Some(false),
                            _ => body.get("enabled").and_then(|v| v.as_bool()),
                        };
                        if !target.is_empty() {
                            let existing = suite
                                .list_records::<PluginRecord>("plugin")
                                .unwrap_or_default();
                            let mut toggled = false;
                            for mut rec in existing {
                                let formatted = format!("{}@{}", rec.name, rec.version);
                                if rec.id == target
                                    || rec.name == target
                                    || rec.entry_path == target
                                    || formatted == target
                                {
                                    rec.enabled = explicit_enabled.unwrap_or(!rec.enabled);
                                    let _ = suite.upsert_record("plugin", &rec.id, &rec);
                                    toggled = true;
                                }
                            }
                            if !toggled {
                                let rec = PluginRecord {
                                    id: target.to_string(),
                                    name: target.to_string(),
                                    version: "local".to_string(),
                                    entry_path: target.to_string(),
                                    enabled: explicit_enabled.unwrap_or(false),
                                    capabilities: vec!["dsh".into(), "web".into()],
                                };
                                let _ = suite.upsert_record("plugin", &rec.id, &rec);
                            }
                        }
                        let reconciled = reconcile_enabled_plugins(&store, &suite.workspace_root);
                        let mut catalog = list_harness_plugins(Some(&store), &suite.workspace_root);
                        if let Some(obj) = catalog.as_object_mut() {
                            obj.insert("ok".into(), json!(true));
                            obj.insert("reconciled".into(), serde_json::to_value(&reconciled).unwrap_or(Value::Null));
                        }
                        let _ = respond_api_json(req, 200, &catalog, crypto_tok);
                    }
                    "remove" => {
                        let target = body
                            .get("id")
                            .or_else(|| body.get("source"))
                            .or_else(|| body.get("name"))
                            .and_then(|v| v.as_str())
                            .map(str::trim)
                            .unwrap_or("");
                        if !target.is_empty() {
                            let existing = suite
                                .list_records::<PluginRecord>("plugin")
                                .unwrap_or_default();
                            let mut removed = false;
                            for rec in existing {
                                let formatted = format!("{}@{}", rec.name, rec.version);
                                if rec.id == target
                                    || rec.name == target
                                    || rec.entry_path == target
                                    || formatted == target
                                {
                                    let _ = suite.delete_record("plugin", &rec.id);
                                    removed = true;
                                }
                            }
                            if !removed {
                                let _ = suite.delete_record("plugin", target);
                            }
                        }
                        let reconciled = reconcile_enabled_plugins(&store, &suite.workspace_root);
                        let mut catalog = list_harness_plugins(Some(&store), &suite.workspace_root);
                        if let Some(obj) = catalog.as_object_mut() {
                            obj.insert("ok".into(), json!(true));
                            obj.insert("reconciled".into(), serde_json::to_value(&reconciled).unwrap_or(Value::Null));
                        }
                        let _ = respond_api_json(req, 200, &catalog, crypto_tok);
                    }
                    _ => {
                        let catalog = list_harness_plugins(Some(&store), &suite.workspace_root);
                        let _ = respond_api_json(req, 200, &catalog, crypto_tok);
                    }
                }
            }
            (Method::Post, "/api/desktop-plugin-pages") => {
                let pages = list_desktop_plugin_pages(&suite.workspace_root);
                if let Some(requested_id) = body
                    .get("id")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    if let Some(found) = pages
                        .into_iter()
                        .find(|p| p.get("id").and_then(|v| v.as_str()) == Some(requested_id))
                    {
                        let _ = respond_api_json(req, 200, &found, crypto_tok);
                    } else {
                        let _ = respond_api_json(
                            req,
                            404,
                            &json!({"error": "plugin_page_not_found", "id": requested_id}),
                            crypto_tok,
                        );
                    }
                } else {
                    let catalog: Vec<Value> = pages
                        .into_iter()
                        .map(|p| {
                            json!({
                                "id": p.get("id").cloned().unwrap_or(Value::Null),
                                "title": p.get("title").cloned().unwrap_or(Value::Null),
                                "package": p.get("package").cloned().unwrap_or(json!("plugin"))
                            })
                        })
                        .collect();
                    let _ = respond_api_json(req, 200, &Value::Array(catalog), crypto_tok);
                }
            }

            // --- Conversations & Live Streaming Chat (v2 Native) ---
            (Method::Get, "/api/chat/conversations") | (Method::Get, "/api/sessions") => {
                match store.list_sessions(200) {
                    Ok(list) => {
                        let conversations: Vec<Value> = list
                            .iter()
                            .map(|s| session_to_conversation_json(&store, s))
                            .collect();
                        let _ = respond_api_json(
                            req,
                            200,
                            &json!({"conversations": conversations, "sessions": list}),
                            crypto_tok,
                        );
                    }
                    Err(e) => {
                        let _ = respond_api_json(req, 500, &json!({"error": e.to_string()}), crypto_tok);
                    }
                }
            }
            (Method::Post, "/api/chat/conversations") => {
                if is_assistant_journal_operation(&body) {
                    let is_mobile = raw_path.starts_with("/api/mobile/");
                    match handle_assistant_journal_http(&store, &suite, host_id, &body, is_mobile) {
                        Ok(payload) => {
                            let _ = respond_api_json(req, 200, &payload, crypto_tok);
                        }
                        Err((code, msg)) => {
                            let _ = respond_api_json(
                                req,
                                400,
                                &json!({"error": code, "message": msg}),
                                crypto_tok,
                            );
                        }
                    }
                    return;
                }
                if is_conversations_list_read_post(&body) {
                    let limit = body
                        .get("limit")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(200)
                        .clamp(1, 500) as usize;
                    match store.list_sessions(limit) {
                        Ok(list) => {
                            let conversations: Vec<Value> = list
                                .iter()
                                .map(|s| session_to_conversation_json(&store, s))
                                .collect();
                            let inner = json!({"conversations": conversations, "sessions": list});
                            let payload = wrap_paired_result(&store, host_id, &body, inner);
                            let _ = respond_api_json(req, 200, &payload, crypto_tok);
                        }
                        Err(e) => {
                            let _ = respond_api_json(req, 500, &json!({"error": e.to_string()}), crypto_tok);
                        }
                    }
                    return;
                }
                let perm_opt = body
                    .get("permissionProfile")
                    .or_else(|| body.get("permission_profile"))
                    .and_then(|v| v.as_str());
                if let Some(perm) = perm_opt {
                    if let Err(err) = PermissionProfile::parse_strict(perm) {
                        let _ = respond_api_json(
                            req,
                            400,
                            &json!({"error": "invalid_permission_profile", "message": err}),
                            crypto_tok,
                        );
                        return;
                    }
                }
                let id = new_id();
                let ts = now_ms();
                let sess = Session {
                    id: id.clone(),
                    title: body
                        .get("title")
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.trim().is_empty())
                        .unwrap_or("New Conversation")
                        .to_string(),
                    workspace_root: body
                        .get("workspace_root")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string)
                        .unwrap_or_else(|| suite.workspace_root.display().to_string()),
                    model_connection: body
                        .get("selectedModelSource")
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.is_empty())
                        .map(ToString::to_string),
                    model_id: body
                        .get("selectedModelId")
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.is_empty())
                        .map(ToString::to_string),
                    created_at_ms: ts,
                    updated_at_ms: ts,
                };
                if let Some(perm) = perm_opt {
                    let _ = store.set_kv(
                        &session_meta_key(&id),
                        &json!({
                            "isPinned": false,
                            "revision": 1,
                            "configurationRevision": 1,
                            "permissionProfile": perm.trim()
                        }),
                    );
                }
                match store.upsert_session(&sess) {
                    Ok(()) => {
                        let conv = session_to_conversation_json(&store, &sess);
                        let payload = wrap_paired_result(&store, host_id, &body, conv);
                        let _ = respond_api_json(req, 200, &payload, crypto_tok);
                    }
                    Err(e) => {
                        let _ = respond_api_json(req, 500, &json!({"error": e.to_string()}), crypto_tok);
                    }
                }
            }
            (Method::Get, p)
                if p.starts_with("/api/chat/conversations/") && p.ends_with("/messages") =>
            {
                let id = percent_decode_path(
                    p.trim_start_matches("/api/chat/conversations/")
                        .trim_end_matches("/messages"),
                );
                let (messages, events) = reconstruct_v2_messages(&store, &id);
                let latest_seq = events
                    .last()
                    .and_then(|e| e.get("seq").and_then(|s| s.as_i64()))
                    .unwrap_or(0);
                let usage = session_usage_json(&store, &id);
                let _ = respond_api_json(
                    req,
                    200,
                    &json!({
                        "conversationId": id,
                        "messages": messages,
                        "events": events,
                        "usage": usage,
                        "hasMore": false,
                        "projectionRevision": 1,
                        "eventSequence": latest_seq
                    }),
                    crypto_tok,
                );
            }
            (Method::Get, p) | (Method::Post, p)
                if p.starts_with("/api/chat/conversations/") && p.ends_with("/events") =>
            {
                let id = percent_decode_path(
                    p.trim_start_matches("/api/chat/conversations/")
                        .trim_end_matches("/events"),
                );
                let after = parse_query_i64(query, "after_seq")
                    .or_else(|| body.get("after_seq").or_else(|| body.get("afterSeq")).and_then(|v| v.as_i64()))
                    .unwrap_or(0);
                let paired_events = has_expected_host_id(&body)
                    || parse_query_str(query, "expected_host_id")
                        .is_some_and(|s| !s.trim().is_empty());
                match store.events_after(&id, after, 500) {
                    Ok(EventPage::Events(ev)) => {
                        let inner = json!({"status": "ok", "events": ev});
                        let payload = if paired_events {
                            wrap_paired_envelope(&store, host_id, inner)
                        } else {
                            inner
                        };
                        let _ = respond_api_json(req, 200, &payload, crypto_tok);
                    }
                    Ok(EventPage::ResyncRequired {
                        earliest_retained_seq,
                        latest_seq,
                    }) => {
                        let inner = json!({
                            "status": "resync_required",
                            "earliest_retained_seq": earliest_retained_seq,
                            "latest_seq": latest_seq
                        });
                        let payload = if paired_events {
                            wrap_paired_envelope(&store, host_id, inner)
                        } else {
                            inner
                        };
                        let _ = respond_api_json(req, 200, &payload, crypto_tok);
                    }
                    Err(e) => {
                        let _ = respond_api_json(req, 500, &json!({"error": e.to_string()}), crypto_tok);
                    }
                }
            }
            (Method::Get, p)
                if p.starts_with("/api/chat/conversations/") && p.ends_with("/export") =>
            {
                let id = percent_decode_path(
                    p.trim_start_matches("/api/chat/conversations/")
                        .trim_end_matches("/export"),
                );
                let title = store
                    .get_session(&id)
                    .ok()
                    .flatten()
                    .map(|s| s.title)
                    .unwrap_or_else(|| id.clone());
                let (messages, _) = reconstruct_v2_messages(&store, &id);
                let is_json = query.contains("format=json");
                if is_json {
                    let payload = json!({
                        "conversationId": id,
                        "title": title,
                        "messages": messages
                    });
                    let _ = respond_api_json(
                        req,
                        200,
                        &json!({
                            "filename": format!("nola-{id}.json"),
                            "mimeType": "application/json; charset=utf-8",
                            "content": serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".into())
                        }),
                        crypto_tok,
                    );
                } else {
                    let mut md = format!("# {title}\n\n");
                    for m in &messages {
                        let role = m.get("role").and_then(|v| v.as_str()).unwrap_or("unknown");
                        let content = m.get("content").and_then(|v| v.as_str()).unwrap_or("");
                        md.push_str(&format!("## {role}\n\n{content}\n\n"));
                    }
                    let _ = respond_api_json(
                        req,
                        200,
                        &json!({
                            "filename": format!("nola-{id}.md"),
                            "mimeType": "text/markdown; charset=utf-8",
                            "content": md
                        }),
                        crypto_tok,
                    );
                }
            }
            (Method::Get, p)
                if p.starts_with("/api/chat/conversations/") && p.ends_with("/configuration") =>
            {
                let id = percent_decode_path(
                    p.trim_start_matches("/api/chat/conversations/")
                        .trim_end_matches("/configuration"),
                );
                let paired_q = parse_query_str(query, "expected_host_id")
                    .is_some_and(|s| !s.trim().is_empty());
                match store.get_session(&id) {
                    Ok(Some(sess)) => {
                        let conv = conversation_configuration_json(&store, &sess);
                        let payload = if paired_q {
                            wrap_paired_envelope(&store, host_id, conv)
                        } else {
                            conv
                        };
                        let _ = respond_api_json(req, 200, &payload, crypto_tok);
                    }
                    Ok(None) => {
                        let _ = respond_api_json(req, 404, &json!({"error": "conversation_not_found"}), crypto_tok);
                    }
                    Err(e) => {
                        let _ = respond_api_json(req, 500, &json!({"error": e.to_string()}), crypto_tok);
                    }
                }
            }
            (Method::Patch, p) | (Method::Post, p)
                if p.starts_with("/api/chat/conversations/") && p.ends_with("/configuration") =>
            {
                let id = percent_decode_path(
                    p.trim_start_matches("/api/chat/conversations/")
                        .trim_end_matches("/configuration"),
                );
                let paired = has_expected_host_id(&body);
                let op = body.get("operation").and_then(|v| v.as_str());
                let action_obj = body.get("action").and_then(|v| v.as_object());
                if let Some(act) = action_obj {
                    let act_type = act
                        .get("type")
                        .and_then(|v| v.as_str())
                        .unwrap_or("read");
                    let apply_selection = |sess: &mut Session, sel: Option<&Value>| -> Value {
                        let Some(sel_obj) = sel.and_then(|v| v.as_object()) else {
                            return Value::Null;
                        };
                        let kind = sel_obj
                            .get("kind")
                            .and_then(|v| v.as_str())
                            .unwrap_or("inherit");
                        if kind == "model" {
                            if let Some(pid) = sel_obj
                                .get("profile_id")
                                .and_then(|v| v.as_str())
                                .map(str::trim)
                                .filter(|s| !s.is_empty())
                            {
                                if let Some((ep, m)) = pid.split_once(':') {
                                    sess.model_connection = Some(ep.to_string());
                                    sess.model_id = Some(m.to_string());
                                } else {
                                    sess.model_id = Some(pid.to_string());
                                }
                                return json!(format!("configured:{pid}"));
                            }
                        }
                        sess.model_connection = None;
                        sess.model_id = None;
                        Value::Null
                    };
                    let current_selection_id = |sess: &Session| -> Value {
                        match (sess.model_connection.as_deref(), sess.model_id.as_deref()) {
                            (Some(c), Some(m)) if !c.is_empty() && !m.is_empty() => {
                                json!(format!("configured:{c}:{m}"))
                            }
                            (_, Some(m)) if !m.is_empty() => json!(format!("configured:{m}")),
                            _ => Value::Null,
                        }
                    };
                    let res_inner = match act_type {
                        "read" => match store.get_session(&id) {
                            Ok(Some(sess)) => {
                                let meta = store
                                    .get_kv(&session_meta_key(&id))
                                    .ok()
                                    .flatten()
                                    .unwrap_or_else(|| json!({}));
                                let rev = meta
                                    .get("configurationRevision")
                                    .and_then(|v| v.as_i64())
                                    .unwrap_or(1)
                                    .max(1);
                                let sel_id = meta
                                    .get("modelSelectionId")
                                    .cloned()
                                    .unwrap_or_else(|| current_selection_id(&sess));
                                json!({
                                    "status": "read",
                                    "configuration_revision": rev,
                                    "model_selection_id": sel_id
                                })
                            }
                            _ => json!({"status": "not_found"}),
                        },
                        "create" => {
                            let ts = now_ms();
                            let title = act
                                .get("title")
                                .and_then(|v| v.as_str())
                                .filter(|s| !s.trim().is_empty())
                                .unwrap_or("Conversation")
                                .to_string();
                            let mut sess = Session {
                                id: id.clone(),
                                title,
                                workspace_root: suite.workspace_root.display().to_string(),
                                model_connection: None,
                                model_id: None,
                                created_at_ms: ts,
                                updated_at_ms: ts,
                            };
                            let sel_id = apply_selection(&mut sess, act.get("selection"));
                            let _ = store.upsert_session(&sess);
                            let _ = store.set_kv(
                                &session_meta_key(&id),
                                &json!({
                                    "isPinned": false,
                                    "revision": 1,
                                    "configurationRevision": 1,
                                    "permissionProfile": "cautious",
                                    "modelSelectionId": sel_id
                                }),
                            );
                            json!({
                                "status": "created",
                                "configuration_revision": 1,
                                "model_selection_id": sel_id
                            })
                        }
                        "select" => match store.get_session(&id) {
                            Ok(Some(mut sess)) => {
                                let meta_key = session_meta_key(&id);
                                let mut meta = store
                                    .get_kv(&meta_key)
                                    .ok()
                                    .flatten()
                                    .unwrap_or_else(|| json!({"configurationRevision": 1}));
                                let cur_rev = meta
                                    .get("configurationRevision")
                                    .and_then(|v| v.as_i64())
                                    .unwrap_or(1)
                                    .max(1);
                                let next_rev = cur_rev + 1;
                                let sel_id = apply_selection(&mut sess, act.get("selection"));
                                meta["configurationRevision"] = json!(next_rev);
                                meta["modelSelectionId"] = sel_id.clone();
                                sess.updated_at_ms = now_ms();
                                let _ = store.set_kv(&meta_key, &meta);
                                let _ = store.upsert_session(&sess);
                                json!({
                                    "status": "committed",
                                    "configuration_revision": next_rev,
                                    "model_selection_id": sel_id
                                })
                            }
                            _ => json!({"status": "not_found"}),
                        },
                        _ => json!({"status": "not_found"}),
                    };
                    let epoch = ensure_source_store_epoch(&store);
                    let _ = respond_api_json(
                        req,
                        200,
                        &json!({
                            "source_host_id": host_id,
                            "source_store_epoch": epoch,
                            "conversation_id": id,
                            "result": res_inner
                        }),
                        crypto_tok,
                    );
                    return;
                }
                let is_post_read = method == Method::Post && is_read_only_post(&body);
                if paired && method == Method::Post && op.is_none() {
                    // Paired iOS/macOS frontier query: auto-synthesize frontier even for non-existent conversations!
                    let client_id = body
                        .get("client_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("nola-client");
                    let (cfg_rev_val, parent_msg_val) = match store.get_session(&id) {
                        Ok(Some(_sess)) => {
                            let meta = store
                                .get_kv(&session_meta_key(&id))
                                .ok()
                                .flatten()
                                .unwrap_or_else(|| json!({}));
                            let rev = meta
                                .get("configurationRevision")
                                .and_then(|v| v.as_i64())
                                .unwrap_or(1)
                                .max(1);
                            let (msgs, _) = reconstruct_v2_messages(&store, &id);
                            let last_id = msgs
                                .last()
                                .and_then(|m| m.get("id").and_then(|v| v.as_str()))
                                .map(|s| json!(s))
                                .unwrap_or(Value::Null);
                            (json!(rev), last_id)
                        }
                        _ => (Value::Null, Value::Null),
                    };
                    let frontier = json!({
                        "protocol_version": 1,
                        "source_host_id": host_id,
                        "client_id": client_id,
                        "conversation_id": id,
                        "configuration_revision": cfg_rev_val,
                        "parent_message_id": parent_msg_val
                    });
                    let payload = wrap_paired_result(&store, host_id, &body, frontier);
                    let _ = respond_api_json(req, 200, &payload, crypto_tok);
                    return;
                }
                if is_post_read {
                    match store.get_session(&id) {
                        Ok(Some(sess)) => {
                            if paired {
                                let conv = conversation_configuration_json(&store, &sess);
                                let rev = conv
                                    .get("configurationRevision")
                                    .and_then(|v| v.as_i64())
                                    .unwrap_or(1)
                                    .max(1);
                                let perm = conv
                                    .get("permissionProfile")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("cautious");
                                let settings_snap = build_agent_settings_snapshot(&store, &suite);
                                let perm_cfg = json!({
                                    "conversation_id": id,
                                    "configuration_revision": rev,
                                    "permission_profile": perm,
                                    "settings": settings_snap,
                                    "active_run_id": Value::Null,
                                    "active_policy": Value::Null,
                                    "owner": {
                                        "kind": "inactive",
                                        "run_id": Value::Null,
                                        "assistant_message_id": Value::Null,
                                        "identity": Value::Null
                                    },
                                    "state": "inactive"
                                });
                                let payload = wrap_paired_result(&store, host_id, &body, perm_cfg);
                                let _ = respond_api_json(req, 200, &payload, crypto_tok);
                            } else {
                                let conv = conversation_configuration_json(&store, &sess);
                                let _ = respond_api_json(req, 200, &conv, crypto_tok);
                            }
                        }
                        Ok(None) => {
                            if paired {
                                let settings_snap = build_agent_settings_snapshot(&store, &suite);
                                let perm_cfg = json!({
                                    "conversation_id": id,
                                    "configuration_revision": 1,
                                    "permission_profile": "cautious",
                                    "settings": settings_snap,
                                    "active_run_id": Value::Null,
                                    "active_policy": Value::Null,
                                    "owner": {
                                        "kind": "inactive",
                                        "run_id": Value::Null,
                                        "assistant_message_id": Value::Null,
                                        "identity": Value::Null
                                    },
                                    "state": "inactive"
                                });
                                let payload = wrap_paired_result(&store, host_id, &body, perm_cfg);
                                let _ = respond_api_json(req, 200, &payload, crypto_tok);
                            } else {
                                let _ = respond_api_json(req, 404, &json!({"error": "conversation_not_found"}), crypto_tok);
                            }
                        }
                        Err(e) => {
                            let _ = respond_api_json(req, 500, &json!({"error": e.to_string()}), crypto_tok);
                        }
                    }
                    return;
                }
                let perm_opt = body
                    .get("permissionProfile")
                    .or_else(|| body.get("permission_profile"))
                    .and_then(|v| v.as_str());
                if let Some(perm) = perm_opt {
                    if let Err(err) = PermissionProfile::parse_strict(perm) {
                        let _ = respond_api_json(
                            req,
                            400,
                            &json!({"error": "invalid_permission_profile", "message": err}),
                            crypto_tok,
                        );
                        return;
                    }
                }
                match store.get_session(&id) {
                    Ok(Some(mut sess)) => {
                        let meta_key = session_meta_key(&id);
                        let mut meta = store
                            .get_kv(&meta_key)
                            .ok()
                            .flatten()
                            .unwrap_or_else(|| json!({"isPinned": false, "revision": 1, "configurationRevision": 1}));
                        let current_cfg_rev = meta
                            .get("configurationRevision")
                            .and_then(|v| v.as_i64())
                            .unwrap_or(1);
                        if let Some(expected) = body
                            .get("expectedRevision")
                            .or_else(|| body.get("expected_revision"))
                            .and_then(|v| v.as_i64())
                        {
                            if expected != current_cfg_rev {
                                let conflict = json!({
                                    "status": "conflict",
                                    "expected_revision": expected,
                                    "actual_revision": current_cfg_rev
                                });
                                let _ = respond_api_json(
                                    req,
                                    if paired { 409 } else { 200 },
                                    &conflict,
                                    crypto_tok,
                                );
                                return;
                            }
                        }
                        if let Some(pinned) = body.get("isPinned").and_then(|v| v.as_bool()) {
                            meta["isPinned"] = json!(pinned);
                        }
                        if let Some(src) = body.get("selectedModelSource").and_then(|v| v.as_str()) {
                            sess.model_connection = (!src.is_empty()).then(|| src.to_string());
                        }
                        if let Some(mid) = body.get("selectedModelId").and_then(|v| v.as_str()) {
                            sess.model_id = (!mid.is_empty()).then(|| mid.to_string());
                        }
                        if let Some(perm) = perm_opt {
                            meta["permissionProfile"] = json!(perm.trim());
                        }
                        if let Some(effort) = body.get("reasoningEffort").and_then(|v| v.as_str()) {
                            meta["reasoningEffort"] = json!(effort);
                        }
                        match body.get("knowledge") {
                            Some(Value::String(text)) => {
                                let mut current_k = load_conversation_knowledge(&store, &id);
                                if let Some(dst) = current_k.as_object_mut() {
                                    dst.insert("keyPointsMarkdown".into(), json!(text));
                                    dst.insert("summary".into(), json!(text));
                                    dst.insert("updatedAt".into(), json!(now_ms()));
                                }
                                let _ = store.set_kv(&conversation_knowledge_key(&id), &current_k);
                            }
                            Some(Value::Object(k_patch)) => {
                                let mut current_k = load_conversation_knowledge(&store, &id);
                                if let Some(dst) = current_k.as_object_mut() {
                                    if let Some(kp) = k_patch.get("keyPointsMarkdown").and_then(|v| v.as_str()) {
                                        dst.insert("keyPointsMarkdown".into(), json!(kp));
                                        dst.insert("summary".into(), json!(kp));
                                    }
                                    if let Some(wiki) = k_patch.get("wikiMarkdown").and_then(|v| v.as_str()) {
                                        dst.insert("wikiMarkdown".into(), json!(wiki));
                                    }
                                    if let Some(summary) = k_patch.get("summary").and_then(|v| v.as_str()) {
                                        dst.insert("summary".into(), json!(summary));
                                    }
                                    if let Some(entries) = k_patch.get("entries").and_then(|v| v.as_array()) {
                                        dst.insert("entries".into(), Value::Array(entries.clone()));
                                    }
                                    dst.insert("updatedAt".into(), json!(now_ms()));
                                }
                                let _ = store.set_kv(&conversation_knowledge_key(&id), &current_k);
                            }
                            _ => {}
                        }
                        let next_cfg_rev = current_cfg_rev + 1;
                        meta["configurationRevision"] = json!(next_cfg_rev);
                        sess.updated_at_ms = now_ms();
                        let _ = store.set_kv(&meta_key, &meta);
                        let _ = store.upsert_session(&sess);
                        let snapshot = conversation_configuration_json(&store, &sess);
                        let resp_val = if paired {
                            let perm = snapshot
                                .get("permissionProfile")
                                .and_then(|v| v.as_str())
                                .unwrap_or("cautious");
                            let settings_snap = build_agent_settings_snapshot(&store, &suite);
                            wrap_paired_result(
                                &store,
                                host_id,
                                &body,
                                json!({
                                    "conversation_id": id,
                                    "configuration_revision": next_cfg_rev,
                                    "permission_profile": perm,
                                    "settings": settings_snap,
                                    "active_run_id": Value::Null,
                                    "active_policy": Value::Null,
                                    "owner": {
                                        "kind": "inactive",
                                        "run_id": Value::Null,
                                        "assistant_message_id": Value::Null,
                                        "identity": Value::Null
                                    },
                                    "state": "inactive"
                                }),
                            )
                        } else {
                            json!({"status": "committed", "snapshot": snapshot})
                        };
                        let _ = respond_api_json(req, 200, &resp_val, crypto_tok);
                    }
                    Ok(None) => {
                        if paired {
                            let next_rev = body
                                .get("expectedRevision")
                                .or_else(|| body.get("expected_revision"))
                                .and_then(|v| v.as_i64())
                                .unwrap_or(0)
                                + 1;
                            let perm = perm_opt.unwrap_or("cautious").trim();
                            let ts = now_ms();
                            let sess = Session {
                                id: id.clone(),
                                title: "Conversation".into(),
                                workspace_root: suite.workspace_root.display().to_string(),
                                model_connection: None,
                                model_id: None,
                                created_at_ms: ts,
                                updated_at_ms: ts,
                            };
                            let _ = store.upsert_session(&sess);
                            let _ = store.set_kv(
                                &session_meta_key(&id),
                                &json!({
                                    "isPinned": false,
                                    "revision": 1,
                                    "configurationRevision": next_rev,
                                    "permissionProfile": perm
                                }),
                            );
                            let settings_snap = build_agent_settings_snapshot(&store, &suite);
                            let payload = wrap_paired_result(
                                &store,
                                host_id,
                                &body,
                                json!({
                                    "conversation_id": id,
                                    "configuration_revision": next_rev,
                                    "permission_profile": perm,
                                    "settings": settings_snap,
                                    "active_run_id": Value::Null,
                                    "active_policy": Value::Null,
                                    "owner": {
                                        "kind": "inactive",
                                        "run_id": Value::Null,
                                        "assistant_message_id": Value::Null,
                                        "identity": Value::Null
                                    },
                                    "state": "inactive"
                                }),
                            );
                            let _ = respond_api_json(req, 200, &payload, crypto_tok);
                        } else {
                            let _ = respond_api_json(
                                req,
                                200,
                                &json!({"status": "not_found", "conversation_id": id}),
                                crypto_tok,
                            );
                        }
                    }
                    Err(e) => {
                        let _ = respond_api_json(req, 500, &json!({"error": e.to_string()}), crypto_tok);
                    }
                }
            }
            (Method::Post, p)
                if p.starts_with("/api/chat/conversations/") && p.ends_with("/branch") =>
            {
                let id = percent_decode_path(
                    p.trim_start_matches("/api/chat/conversations/")
                        .trim_end_matches("/branch"),
                );
                let now = now_ms();
                let new_conv_id = new_id();
                let parent = store.get_session(&id).ok().flatten();
                let title = parent
                    .as_ref()
                    .map(|s| format!("{} (copy)", s.title))
                    .unwrap_or_else(|| "Branched conversation".into());
                let sess = Session {
                    id: new_conv_id.clone(),
                    title,
                    workspace_root: parent
                        .as_ref()
                        .map(|s| s.workspace_root.clone())
                        .unwrap_or_else(|| suite.workspace_root.display().to_string()),
                    model_connection: parent.as_ref().and_then(|s| s.model_connection.clone()),
                    model_id: parent.as_ref().and_then(|s| s.model_id.clone()),
                    created_at_ms: now,
                    updated_at_ms: now,
                };
                if let Err(e) = store.upsert_session(&sess) {
                    let _ = respond_api_json(req, 500, &json!({"error": e.to_string()}), crypto_tok);
                    return;
                }
                let boundary_seq = parse_message_seq(&body);
                if let Ok(EventPage::Events(ev)) = store.events_after(&id, 0, 2000) {
                    for e in ev {
                        if let Some(max_seq) = boundary_seq {
                            if e.seq > max_seq {
                                break;
                            }
                        }
                        // CRITICAL FIX: pass `50_000` (max_events_per_scope), NEVER `e.created_at_ms`!
                        let _ = store.append_event(&new_conv_id, &e.kind, e.payload, 50_000);
                    }
                }
                // Copy session_state, meta, and knowledge from source to branched conversation
                if let Ok(Some(state_val)) = store.get_kv(&format!("session_state:{id}")) {
                    let _ = store.set_kv(&format!("session_state:{new_conv_id}"), &state_val);
                }
                if boundary_seq.is_some() {
                    synchronize_session_state_after_truncate(&store, &new_conv_id);
                }
                if let Ok(Some(meta_val)) = store.get_kv(&session_meta_key(&id)) {
                    let _ = store.set_kv(&session_meta_key(&new_conv_id), &meta_val);
                }
                if let Ok(Some(k_val)) = store.get_kv(&conversation_knowledge_key(&id)) {
                    let _ = store.set_kv(&conversation_knowledge_key(&new_conv_id), &k_val);
                }
                let conv = session_to_conversation_json(&store, &sess);
                let _ = respond_api_json(req, 200, &json!({"conversation": conv}), crypto_tok);
            }
            (Method::Get, p)
                if p.starts_with("/api/chat/conversations/")
                    && (p.ends_with("/workspace-review") || p.ends_with("/workspace/diff")) =>
            {
                let id = percent_decode_path(
                    p.trim_start_matches("/api/chat/conversations/")
                        .trim_end_matches("/workspace-review")
                        .trim_end_matches("/workspace/diff"),
                );
                let ws_root = store
                    .get_session(&id)
                    .ok()
                    .flatten()
                    .map(|s| PathBuf::from(s.workspace_root))
                    .filter(|p| p.is_dir())
                    .unwrap_or_else(|| suite.workspace_root.clone());
                let review = build_workspace_review(host_id, &id, &ws_root);
                let _ = respond_api_json(req, 200, &review, crypto_tok);
            }
            (Method::Get, p)
                if p.starts_with("/api/chat/conversations/")
                    && p.ends_with("/workspace/checkpoints") =>
            {
                let checkpoints: Vec<Value> = suite
                    .list_records("workspace_checkpoint")
                    .unwrap_or_default();
                let _ = respond_api_json(req, 200, &json!({"checkpoints": checkpoints}), crypto_tok);
            }
            (Method::Post, p)
                if p.starts_with("/api/chat/conversations/") && p.ends_with("/workspace/undo") =>
            {
                let id = percent_decode_path(
                    p.trim_start_matches("/api/chat/conversations/")
                        .trim_end_matches("/workspace/undo"),
                );
                let ws_root = store
                    .get_session(&id)
                    .ok()
                    .flatten()
                    .map(|s| PathBuf::from(s.workspace_root))
                    .filter(|p| p.is_dir())
                    .unwrap_or_else(|| suite.workspace_root.clone());
                let out = Command::new("git")
                    .args(["stash", "push", "-u", "-m", "nola-v2-undo"])
                    .current_dir(&ws_root)
                    .output();
                match out {
                    Ok(o) if o.status.success() => {
                        let _ = respond_api_json(
                            req,
                            200,
                            &json!({"ok": true, "stdout": String::from_utf8_lossy(&o.stdout)}),
                            crypto_tok,
                        );
                    }
                    Ok(o) => {
                        let _ = respond_api_json(
                            req,
                            400,
                            &json!({"ok": false, "error": String::from_utf8_lossy(&o.stderr)}),
                            crypto_tok,
                        );
                    }
                    Err(e) => {
                        let _ = respond_api_json(
                            req,
                            500,
                            &json!({"ok": false, "error": e.to_string()}),
                            crypto_tok,
                        );
                    }
                }
            }
            (Method::Post, p)
                if p.starts_with("/api/chat/conversations/") && p.ends_with("/workspace/redo") =>
            {
                let id = percent_decode_path(
                    p.trim_start_matches("/api/chat/conversations/")
                        .trim_end_matches("/workspace/redo"),
                );
                let ws_root = store
                    .get_session(&id)
                    .ok()
                    .flatten()
                    .map(|s| PathBuf::from(s.workspace_root))
                    .filter(|p| p.is_dir())
                    .unwrap_or_else(|| suite.workspace_root.clone());
                let out = Command::new("git")
                    .args(["stash", "pop"])
                    .current_dir(&ws_root)
                    .output();
                match out {
                    Ok(o) if o.status.success() => {
                        let _ = respond_api_json(
                            req,
                            200,
                            &json!({"ok": true, "stdout": String::from_utf8_lossy(&o.stdout)}),
                            crypto_tok,
                        );
                    }
                    Ok(o) => {
                        let _ = respond_api_json(
                            req,
                            400,
                            &json!({"ok": false, "error": String::from_utf8_lossy(&o.stderr)}),
                            crypto_tok,
                        );
                    }
                    Err(e) => {
                        let _ = respond_api_json(
                            req,
                            500,
                            &json!({"ok": false, "error": e.to_string()}),
                            crypto_tok,
                        );
                    }
                }
            }
            (Method::Post, p)
                if p.starts_with("/api/chat/conversations/") && p.ends_with("/cancel") =>
            {
                let id = percent_decode_path(
                    p.trim_start_matches("/api/chat/conversations/")
                        .trim_end_matches("/cancel"),
                );
                if is_paired_task_cancellation_body(&body) {
                    let receipt = handle_paired_task_cancellation(
                        &store,
                        &suite,
                        active_cancels,
                        host_id,
                        &id,
                        None,
                        &body,
                    );
                    let _ = respond_api_json(req, 200, &receipt, crypto_tok);
                    return;
                }
                let cancelled = if let Some(flag) = active_cancels.lock().unwrap().get(&id) {
                    flag.store(true, Ordering::SeqCst);
                    true
                } else {
                    false
                };
                let _ = respond_api_json(req, 200, &json!({"ok": true, "cancelled": cancelled}), crypto_tok);
            }
            (Method::Post, p)
                if p.starts_with("/api/chat/conversations/") && p.ends_with("/permission") =>
            {
                let req_id = body
                    .get("request_id")
                    .or_else(|| body.get("requestId"))
                    .or_else(|| body.pointer("/resolution/request_id"))
                    .or_else(|| body.get("id"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let (allow, scope) = parse_permission_resolution(&body);
                let resolved = pending_interactions.resolve_approval(req_id, allow, scope);
                let _ = respond_api_json(
                    req,
                    200,
                    &json!({"ok": true, "resolved": resolved, "allow": allow, "scope": scope}),
                    crypto_tok,
                );
            }
            (Method::Post, p)
                if p.starts_with("/api/chat/conversations/") && p.ends_with("/user-input") =>
            {
                let req_id = body
                    .get("request_id")
                    .or_else(|| body.get("requestId"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let answer = body
                    .get("answer")
                    .or_else(|| body.get("text"))
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string);
                let resolved = pending_interactions.resolve_question(req_id, answer);
                let _ = respond_api_json(req, 200, &json!({"ok": true, "resolved": resolved}), crypto_tok);
            }
            (Method::Get, p) | (Method::Post, p)
                if p.starts_with("/api/chat/conversations/") && p.ends_with("/ui-surfaces") =>
            {
                let conv_id = percent_decode_path(
                    p.trim_start_matches("/api/chat/conversations/")
                        .trim_end_matches("/ui-surfaces"),
                );
                let surfaces = list_conversation_surfaces(&store, &conv_id);
                let paired = has_expected_host_id(&body)
                    || parse_query_str(query, "expected_host_id")
                        .is_some_and(|s| !s.trim().is_empty());
                let resp_val = if paired {
                    json!({
                        "source_host_id": host_id,
                        "result": surfaces
                    })
                } else {
                    json!({"surfaces": surfaces})
                };
                let _ = respond_api_json(req, 200, &resp_val, crypto_tok);
            }
            (Method::Post, p)
                if p.starts_with("/api/chat/conversations/") && p.ends_with("/ui-surface-revision") =>
            {
                let conv_id = percent_decode_path(
                    p.trim_start_matches("/api/chat/conversations/")
                        .trim_end_matches("/ui-surface-revision"),
                );
                let payload = unwrap_input_envelope(&body);
                match serde_json::from_value::<UiSurfaceRevisionRef>(payload) {
                    Ok(reference) if reference.identity.owner.conversation_id == conv_id => {
                        match read_ui_surface_revision(&store, &reference) {
                            Ok(snapshot) => {
                                let resp_val = if has_expected_host_id(&body) {
                                    json!({
                                        "source_host_id": host_id,
                                        "result": snapshot
                                    })
                                } else {
                                    json!({"snapshot": snapshot})
                                };
                                let _ = respond_api_json(req, 200, &resp_val, crypto_tok);
                            }
                            Err(e) => {
                                let _ = respond_api_json(req, 409, &json!({"error": e}), crypto_tok);
                            }
                        }
                    }
                    _ => {
                        let _ = respond_api_json(req, 400, &json!({"error": "invalid_ui_revision_ref"}), crypto_tok);
                    }
                }
            }
            (Method::Post, p)
                if p.starts_with("/api/chat/conversations/") && p.ends_with("/ui-surface-changes") =>
            {
                let conv_id = percent_decode_path(
                    p.trim_start_matches("/api/chat/conversations/")
                        .trim_end_matches("/ui-surface-changes"),
                );
                let payload = unwrap_input_envelope(&body);
                let identity = payload
                    .get("identity")
                    .cloned()
                    .and_then(|v| serde_json::from_value::<UiSurfaceIdentity>(v).ok());
                let base_rev = payload.get("base_revision").and_then(|v| v.as_u64());
                match identity {
                    Some(id) if id.owner.conversation_id == conv_id => {
                        match read_ui_surface_changes(&store, &id, base_rev) {
                            Ok(update) => {
                                let resp_val = if has_expected_host_id(&body) {
                                    json!({
                                        "source_host_id": host_id,
                                        "result": update
                                    })
                                } else {
                                    json!({"update": update})
                                };
                                let _ = respond_api_json(req, 200, &resp_val, crypto_tok);
                            }
                            Err(e) => {
                                let _ = respond_api_json(req, 409, &json!({"error": e}), crypto_tok);
                            }
                        }
                    }
                    _ => {
                        let _ = respond_api_json(req, 400, &json!({"error": "invalid_ui_changes_query"}), crypto_tok);
                    }
                }
            }
            (Method::Post, p)
                if p.starts_with("/api/chat/conversations/") && p.ends_with("/ui-surface-preview") =>
            {
                let conv_id = percent_decode_path(
                    p.trim_start_matches("/api/chat/conversations/")
                        .trim_end_matches("/ui-surface-preview"),
                );
                let payload = unwrap_input_envelope(&body);
                match serde_json::from_value::<UiDraftPreviewInput>(payload) {
                    Ok(input) if input.identity.owner.conversation_id == conv_id => {
                        match preview_ui_surface(&store, &input) {
                            Ok(preview) => {
                                let resp_val = if has_expected_host_id(&body) {
                                    json!({
                                        "source_host_id": host_id,
                                        "result": preview
                                    })
                                } else {
                                    json!({"preview": preview})
                                };
                                let _ = respond_api_json(req, 200, &resp_val, crypto_tok);
                            }
                            Err(e) => {
                                let _ = respond_api_json(req, 409, &json!({"error": e}), crypto_tok);
                            }
                        }
                    }
                    _ => {
                        let _ = respond_api_json(req, 400, &json!({"error": "invalid_ui_preview_input"}), crypto_tok);
                    }
                }
            }
            (Method::Post, p)
                if p.starts_with("/api/chat/conversations/") && p.ends_with("/ui-interactions") =>
            {
                let conv_id = percent_decode_path(
                    p.trim_start_matches("/api/chat/conversations/")
                        .trim_end_matches("/ui-interactions"),
                );
                let payload = unwrap_input_envelope(&body);
                let queued_submission_id = payload
                    .get("submission_id")
                    .or_else(|| payload.get("submissionId"))
                    .or_else(|| payload.get("request_id"))
                    .or_else(|| payload.get("requestId"))
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
                    .map(ToString::to_string)
                    .unwrap_or_else(new_id);
                // Unwrap single or double-nested `interaction.interaction` when present!
                let interaction_val = payload
                    .get("interaction")
                    .filter(|v| v.is_object())
                    .map(|inner| {
                        inner
                            .get("interaction")
                            .filter(|v| v.is_object())
                            .cloned()
                            .unwrap_or_else(|| inner.clone())
                    })
                    .unwrap_or(payload);
                match serde_json::from_value::<UiInteraction>(interaction_val) {
                    Ok(interaction) if interaction.identity.owner.conversation_id == conv_id => {
                        match commit_ui_interaction(&store, &interaction) {
                            Ok(committed) => {
                                let inner = json!({
                                    "status": "committed",
                                    "snapshot": committed.snapshot,
                                    "dataModel": committed.data_model,
                                    "actionId": committed.action_id,
                                    "summary": committed.summary,
                                    "receipt": {
                                        "snapshot": committed.snapshot,
                                        "queued_submission_id": queued_submission_id,
                                        "action_id": committed.action_id
                                    }
                                });
                                let resp_val = wrap_paired_result(&store, host_id, &body, inner);
                                let _ = respond_api_json(req, 200, &resp_val, crypto_tok);
                            }
                            Err(e) => {
                                let status = if e.contains("revision conflict") { 409 } else { 422 };
                                let _ = respond_api_json(
                                    req,
                                    status,
                                    &json!({"status": "rejected", "error": e}),
                                    crypto_tok,
                                );
                            }
                        }
                    }
                    _ => {
                        let _ = respond_api_json(
                            req,
                            400,
                            &json!({"status": "rejected", "error": "invalid_ui_interaction"}),
                            crypto_tok,
                        );
                    }
                }
            }
            (Method::Post, p)
                if p.starts_with("/api/chat/conversations/") && p.ends_with("/truncate") =>
            {
                let conv_id = percent_decode_path(
                    p.trim_start_matches("/api/chat/conversations/")
                        .trim_end_matches("/truncate"),
                );
                if active_cancels.lock().unwrap().contains_key(&conv_id) {
                    let _ = respond_api_json(
                        req,
                        409,
                        &json!({"error": "active_run_in_progress", "message": "cannot truncate while a run is active"}),
                        crypto_tok,
                    );
                    return;
                }
                let base_seq = parse_message_seq(&body).unwrap_or(1);
                let keep_message = body
                    .get("keepMessage")
                    .or_else(|| body.get("keep_message"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let from_seq = if keep_message {
                    base_seq.saturating_add(1)
                } else {
                    base_seq
                };
                match store.truncate_events_from(&conv_id, from_seq) {
                    Ok(deleted) => {
                        synchronize_session_state_after_truncate(&store, &conv_id);
                        let _ = respond_api_json(
                            req,
                            200,
                            &json!({"ok": true, "deletedEvents": deleted, "fromSeq": from_seq}),
                            crypto_tok,
                        );
                    }
                    Err(e) => {
                        let _ = respond_api_json(req, 500, &json!({"error": e.to_string()}), crypto_tok);
                    }
                }
            }
            (Method::Post, p)
                if p.starts_with("/api/chat/conversations/") && p.ends_with("/compact") =>
            {
                let conv_id = percent_decode_path(
                    p.trim_start_matches("/api/chat/conversations/")
                        .trim_end_matches("/compact"),
                );
                if active_cancels.lock().unwrap().contains_key(&conv_id) {
                    let _ = respond_api_json(
                        req,
                        409,
                        &json!({"error": "active_run_in_progress", "message": "cannot compact while a run is active"}),
                        crypto_tok,
                    );
                    return;
                }
                let cfg = load_active_config(&store);
                let llm = LlmClient::new(120).ok();
                let route = resolve_model_route(&cfg).ok();
                match compact_session_now(&store, llm.as_ref(), route.as_ref(), &conv_id) {
                    Ok(report) => {
                        let payload = wrap_paired_result(&store, host_id, &body, report);
                        let _ = respond_api_json(req, 200, &payload, crypto_tok);
                    }
                    Err(e) => {
                        let _ = respond_api_json(req, 500, &json!({"error": e}), crypto_tok);
                    }
                }
            }
            (Method::Get, p)
                if p.starts_with("/api/chat/conversations/") && p.ends_with("/goal") =>
            {
                let conv_id = percent_decode_path(
                    p.trim_start_matches("/api/chat/conversations/")
                        .trim_end_matches("/goal"),
                );
                let key = format!("conv_goal:{conv_id}");
                let goal = store.get_kv(&key).ok().flatten().unwrap_or(Value::Null);
                let epoch = ensure_source_store_epoch(&store);
                let _ = respond_api_json(
                    req,
                    200,
                    &json!({
                        "ok": true,
                        "source_host_id": host_id,
                        "source_store_epoch": epoch,
                        "goal": goal
                    }),
                    crypto_tok,
                );
            }
            (Method::Post, p) | (Method::Put, p)
                if (p.starts_with("/api/chat/conversations/") && p.ends_with("/goal"))
                    || p == "/api/chat/agent/v2/goal" =>
            {
                let conv_id = if p == "/api/chat/agent/v2/goal" {
                    body.get("conversation_id")
                        .or_else(|| body.get("conversationId"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("default")
                        .to_string()
                } else {
                    percent_decode_path(
                        p.trim_start_matches("/api/chat/conversations/")
                            .trim_end_matches("/goal"),
                    )
                };
                let (status, payload) =
                    handle_conversation_goal_control(&store, &suite, host_id, &conv_id, &body);
                let _ = respond_api_json(req, status, &payload, crypto_tok);
            }
            (Method::Delete, p)
                if p.starts_with("/api/chat/conversations/") && p.ends_with("/goal") =>
            {
                let conv_id = percent_decode_path(
                    p.trim_start_matches("/api/chat/conversations/")
                        .trim_end_matches("/goal"),
                );
                let _ = store.set_kv(&format!("conv_goal:{conv_id}"), &Value::Null);
                let _ = respond_api_json(req, 200, &json!({"ok": true, "deleted": true}), crypto_tok);
            }
            (Method::Post, p)
                if (p.starts_with("/api/chat/conversations/") && p.ends_with("/device-tool"))
                    || p == "/api/chat/agent/v2/device-tool" =>
            {
                let conv_id = if p == "/api/chat/agent/v2/device-tool" {
                    body.get("conversation_id")
                        .or_else(|| body.get("conversationId"))
                        .or_else(|| body.get("session_id"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("default")
                        .to_string()
                } else {
                    percent_decode_path(
                        p.trim_start_matches("/api/chat/conversations/")
                            .trim_end_matches("/device-tool"),
                    )
                };
                let req_id = body
                    .get("request_id")
                    .or_else(|| body.get("requestId"))
                    .or_else(|| body.get("call_id"))
                    .or_else(|| body.get("callId"))
                    .or_else(|| body.get("tool_call_id"))
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .unwrap_or("")
                    .to_string();
                let resolved = if req_id.is_empty() {
                    false
                } else {
                    pending_interactions.resolve_device_tool(&req_id, body.clone())
                };
                let _ = store.append_event(
                    &conv_id,
                    "device_tool_result",
                    json!({
                        "request_id": req_id,
                        "resolved": resolved,
                        "ok": body.get("ok").and_then(|v| v.as_bool()).unwrap_or(true),
                        "tool": body.get("tool").or_else(|| body.get("name")).cloned().unwrap_or(Value::Null)
                    }),
                    50_000,
                );
                let _ = respond_api_json(
                    req,
                    200,
                    &json!({
                        "ok": true,
                        "resolved": resolved,
                        "request_id": req_id,
                        "conversation_id": conv_id,
                        "source_host_id": host_id
                    }),
                    crypto_tok,
                );
            }
            (Method::Get, p)
                if p.starts_with("/api/chat/conversations/") && p.ends_with("/tasks/events") =>
            {
                let conv_id = percent_decode_path(
                    p.trim_start_matches("/api/chat/conversations/")
                        .trim_end_matches("/tasks/events"),
                );
                let has_after_cursor = parse_query_str(query, "afterCursor").is_some()
                    || parse_query_str(query, "after_cursor").is_some();
                let after_cursor = parse_query_i64(query, "after")
                    .or_else(|| parse_query_i64(query, "afterCursor"))
                    .or_else(|| parse_query_i64(query, "after_cursor"))
                    .or_else(|| parse_query_i64(query, "after_seq"))
                    .or_else(|| {
                        header_value(&req, "last-event-id")
                            .and_then(|s| s.trim().parse::<i64>().ok())
                    })
                    .unwrap_or(0);
                let timeout_ms = parse_query_i64(query, "timeoutMs")
                    .or_else(|| parse_query_i64(query, "timeout_ms"))
                    .map(|v| v.clamp(50, 30_000) as u64);
                let oneshot = parse_query_str(query, "oneshot")
                    .is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"));
                let emit_task_projection = raw_path.starts_with("/api/mobile/") || has_after_cursor;
                stream_conversation_task_events(
                    req,
                    store.clone(),
                    conv_id,
                    after_cursor,
                    oneshot,
                    timeout_ms,
                    emit_task_projection,
                    crypto_tok.map(ToString::to_string),
                );
            }
            (Method::Post, p)
                if p.starts_with("/api/chat/conversations/")
                    && p.contains("/tasks/")
                    && p.ends_with("/stop") =>
            {
                let rest = p
                    .trim_start_matches("/api/chat/conversations/")
                    .trim_end_matches("/stop");
                if let Some((raw_conv, raw_task)) = rest.split_once("/tasks/") {
                    let conv_id = percent_decode_path(raw_conv);
                    let task_id = percent_decode_path(raw_task);
                    if is_paired_task_cancellation_body(&body)
                        || raw_path.starts_with("/api/mobile/")
                    {
                        let receipt = handle_paired_task_cancellation(
                            &store,
                            &suite,
                            active_cancels,
                            host_id,
                            &conv_id,
                            Some(&task_id),
                            &body,
                        );
                        let _ = respond_api_json(req, 200, &receipt, crypto_tok);
                        return;
                    }
                    let cancelled = if let Some(flag) = active_cancels.lock().unwrap().get(&conv_id) {
                        flag.store(true, Ordering::SeqCst);
                        true
                    } else {
                        false
                    };
                    let _ = store.append_event(
                        &conv_id,
                        "task_stopped",
                        json!({
                            "task_id": task_id,
                            "status": "cancelled",
                            "cancelled": cancelled
                        }),
                        50_000,
                    );
                    let _ = respond_api_json(
                        req,
                        200,
                        &json!({
                            "ok": true,
                            "stopped": true,
                            "cancelled": cancelled,
                            "conversationId": conv_id,
                            "taskId": task_id
                        }),
                        crypto_tok,
                    );
                } else {
                    let _ = respond_api_json(
                        req,
                        400,
                        &json!({"ok": false, "error": "invalid_task_stop_path"}),
                        crypto_tok,
                    );
                }
            }
            (Method::Get, p) | (Method::Post, p)
                if p.starts_with("/api/chat/conversations/") && p.ends_with("/progress") =>
            {
                let conv_id = percent_decode_path(
                    p.trim_start_matches("/api/chat/conversations/")
                        .trim_end_matches("/progress"),
                );
                let active = active_cancels.lock().unwrap().contains_key(&conv_id);
                let needs_resp = pending_interactions.conversation_needs_response(&conv_id);
                let todos = load_session_todos(&store, &conv_id);
                let plan = load_session_plan(&store, &conv_id);
                let plan_mode = is_plan_mode(&store, &conv_id);
                let goal = store
                    .get_kv(&format!("conv_goal:{conv_id}"))
                    .ok()
                    .flatten()
                    .unwrap_or(Value::Null);
                let goal_projection = build_goal_projection(&store, &conv_id, &goal);
                let expected_host_q = parse_query_str(query, "expected_host_id")
                    .or_else(|| {
                        body.get("expected_host_id")
                            .and_then(|v| v.as_str())
                            .map(ToString::to_string)
                    })
                    .filter(|s| !s.trim().is_empty());
                if expected_host_q.is_some() {
                    let paired_todos: Vec<Value> = todos
                        .as_array()
                        .into_iter()
                        .flatten()
                        .enumerate()
                        .map(|(idx, t)| {
                            json!({
                                "id": t.get("id").and_then(|v| v.as_str()).map(ToString::to_string).unwrap_or_else(|| format!("todo-{idx}")),
                                "content": t.get("content").or_else(|| t.get("title")).and_then(|v| v.as_str()).unwrap_or(""),
                                "status": t.get("status").and_then(|v| v.as_str()).unwrap_or("pending")
                            })
                        })
                        .collect();
                    let _ = respond_api_json(
                        req,
                        200,
                        &json!({
                            "source_host_id": host_id,
                            "conversation_id": conv_id,
                            "goal_projection": goal_projection,
                            "plan": {
                                "conversation_id": conv_id,
                                "todos": paired_todos
                            }
                        }),
                        crypto_tok,
                    );
                } else {
                    let usage = session_usage_json(&store, &conv_id);
                    let (_, raw_events) = reconstruct_v2_messages(&store, &conv_id);
                    let pending_perm = pending_interactions.pending_permission_for(&conv_id);
                    let pending_q = pending_interactions.pending_question_for(&conv_id);
                    let _ = respond_api_json(
                        req,
                        200,
                        &json!({
                            "ok": true,
                            "conversationId": conv_id,
                            "source_host_id": host_id,
                            "active": active,
                            "needsResponse": needs_resp,
                            "pendingPermission": pending_perm,
                            "pendingQuestion": pending_q,
                            "todos": todos,
                            "plan": plan,
                            "planMode": plan_mode,
                            "goal": goal,
                            "goal_projection": goal_projection,
                            "usage": usage,
                            "eventCount": raw_events.len()
                        }),
                        crypto_tok,
                    );
                }
            }
            (Method::Get, p) | (Method::Post, p)
                if p.starts_with("/api/chat/conversations/") && p.ends_with("/observe") =>
            {
                let conv_id = percent_decode_path(
                    p.trim_start_matches("/api/chat/conversations/")
                        .trim_end_matches("/observe"),
                );
                let active = active_cancels.lock().unwrap().contains_key(&conv_id);
                let needs_resp = pending_interactions.conversation_needs_response(&conv_id);
                let expected_host_q = parse_query_str(query, "expected_host_id")
                    .or_else(|| {
                        body.get("expected_host_id")
                            .and_then(|v| v.as_str())
                            .map(ToString::to_string)
                    })
                    .filter(|s| !s.trim().is_empty());
                let client_id_q = parse_query_str(query, "client_id")
                    .or_else(|| {
                        body.get("client_id")
                            .and_then(|v| v.as_str())
                            .map(ToString::to_string)
                    })
                    .filter(|s| !s.trim().is_empty());
                let epoch = ensure_source_store_epoch(&store);
                if expected_host_q.is_some() || client_id_q.is_some() {
                    let client_id = client_id_q.unwrap_or_else(|| "nola-client".into());
                    let expected_run_id = parse_query_str(query, "expected_run_id")
                        .or_else(|| {
                            body.get("expected_run_id")
                                .and_then(|v| v.as_str())
                                .map(ToString::to_string)
                        })
                        .filter(|s| !s.trim().is_empty());
                    let expected_asst_id = parse_query_str(query, "assistant_message_id")
                        .or_else(|| {
                            body.get("assistant_message_id")
                                .and_then(|v| v.as_str())
                                .map(ToString::to_string)
                        })
                        .filter(|s| !s.trim().is_empty());
                    let run_id = expected_run_id
                        .unwrap_or_else(|| format!("run-{}", conv_id));
                    let asst_id = expected_asst_id
                        .unwrap_or_else(|| format!("asst-{}", conv_id));
                    let perms = if active {
                        pending_interactions.pending_permissions_list(&conv_id)
                    } else {
                        Vec::new()
                    };
                    let (status_str, worker_state, recovery_str, can_submit, terminal_seal) =
                        if active {
                            ("running", "running", "observe", false, Value::Null)
                        } else {
                            (
                                "sealed",
                                "absent",
                                "terminal",
                                true,
                                json!({
                                    "protocol_version": 1,
                                    "identity": {
                                        "conversation_id": conv_id,
                                        "run_id": run_id,
                                        "assistant_message_id": asst_id,
                                        "source_store_epoch": epoch
                                    },
                                    "result": {
                                        "conversation_id": conv_id,
                                        "run_id": run_id
                                    }
                                }),
                            )
                        };
                    let paired_obs = json!({
                        "source_host_id": host_id,
                        "conversation_id": conv_id,
                        "client_id": client_id,
                        "result": {
                            "protocol_version": 1,
                            "client_id": client_id,
                            "conversation_id": conv_id,
                            "run_id": run_id,
                            "assistant_message_id": asst_id,
                            "status": status_str,
                            "worker": {
                                "state": worker_state,
                                "generation": Value::Null
                            },
                            "recovery": recovery_str,
                            "permissions": perms,
                            "can_submit": can_submit,
                            "admission": Value::Null,
                            "terminal_seal": terminal_seal,
                            "context_usage": Value::Null,
                            "submission_receipt": Value::Null
                        }
                    });
                    let _ = respond_api_json(req, 200, &paired_obs, crypto_tok);
                } else {
                    let events = match store.events_after(&conv_id, 0, 500) {
                        Ok(EventPage::Events(ev)) => ev,
                        _ => Vec::new(),
                    };
                    let latest_seq = events.last().map(|e| e.seq).unwrap_or(0);
                    let recent: Vec<Value> = events
                        .iter()
                        .rev()
                        .take(50)
                        .rev()
                        .filter_map(|e| serde_json::to_value(e).ok())
                        .collect();
                    let todos = load_session_todos(&store, &conv_id);
                    let plan = load_session_plan(&store, &conv_id);
                    let plan_mode = is_plan_mode(&store, &conv_id);
                    let goal = store
                        .get_kv(&format!("conv_goal:{conv_id}"))
                        .ok()
                        .flatten()
                        .unwrap_or(Value::Null);
                    let pending_perm = pending_interactions.pending_permission_for(&conv_id);
                    let pending_q = pending_interactions.pending_question_for(&conv_id);
                    let inner = json!({
                        "ok": true,
                        "conversationId": conv_id,
                        "source_host_id": host_id,
                        "source_store_epoch": epoch,
                        "active": active,
                        "needsResponse": needs_resp,
                        "pendingPermission": pending_perm,
                        "pendingQuestion": pending_q,
                        "latestSeq": latest_seq,
                        "eventCount": events.len(),
                        "todos": todos,
                        "plan": plan,
                        "planMode": plan_mode,
                        "goal": goal,
                        "recentEvents": recent
                    });
                    let _ = respond_api_json(req, 200, &inner, crypto_tok);
                }
            }
            (Method::Get, p) | (Method::Post, p)
                if p.starts_with("/api/chat/conversations/") && p.ends_with("/recall") =>
            {
                let conv_id = percent_decode_path(
                    p.trim_start_matches("/api/chat/conversations/")
                        .trim_end_matches("/recall"),
                );
                let recall_body = if method == Method::Get {
                    let mut m = serde_json::Map::new();
                    if let Some(q) = parse_query_str(query, "q")
                        .or_else(|| parse_query_str(query, "query"))
                    {
                        m.insert("query".into(), json!(q));
                    }
                    if let Some(lim) = parse_query_i64(query, "limit") {
                        m.insert("limit".into(), json!(lim));
                    }
                    if let Some(fs) = parse_query_i64(query, "from_seq")
                        .or_else(|| parse_query_i64(query, "fromSeq"))
                    {
                        m.insert("from_seq".into(), json!(fs));
                    }
                    Value::Object(m)
                } else {
                    body.clone()
                };
                let res = handle_conversation_recall(&store, &conv_id, &recall_body);
                let _ = respond_api_json(req, 200, &res, crypto_tok);
            }
            (Method::Get, p)
                if p.starts_with("/api/chat/conversations/") && p.ends_with("/artifacts") =>
            {
                let conv_id = percent_decode_path(
                    p.trim_start_matches("/api/chat/conversations/")
                        .trim_end_matches("/artifacts"),
                );
                let artifacts = collect_conversation_artifacts(&store, &conv_id);
                let count = artifacts.len();
                let _ = respond_api_json(
                    req,
                    200,
                    &json!({
                        "ok": true,
                        "conversationId": conv_id,
                        "artifacts": artifacts,
                        "count": count
                    }),
                    crypto_tok,
                );
            }
            (Method::Get, p)
                if p.starts_with("/api/chat/conversations/") && p.ends_with("/plan") =>
            {
                let conv_id = percent_decode_path(
                    p.trim_start_matches("/api/chat/conversations/")
                        .trim_end_matches("/plan"),
                );
                let _ = respond_api_json(
                    req,
                    200,
                    &json!({
                        "ok": true,
                        "conversationId": conv_id,
                        "planMode": is_plan_mode(&store, &conv_id),
                        "plan": load_session_plan(&store, &conv_id),
                        "todos": load_session_todos(&store, &conv_id)
                    }),
                    crypto_tok,
                );
            }
            (Method::Post, p)
                if p.starts_with("/api/chat/conversations/") && p.ends_with("/plan") =>
            {
                let conv_id = percent_decode_path(
                    p.trim_start_matches("/api/chat/conversations/")
                        .trim_end_matches("/plan"),
                );
                let (status, payload) = handle_conversation_plan(&store, &conv_id, &body);
                let _ = respond_api_json(req, status, &payload, crypto_tok);
            }
            (Method::Post, "/api/auxiliary/prepare")
            | (Method::Post, "/api/auxiliary/admit")
            | (Method::Post, "/api/auxiliary/invoke")
            | (Method::Post, "/api/auxiliary/inspect")
            | (Method::Post, "/api/auxiliary/cancel") => {
                let op = path.trim_start_matches("/api/auxiliary/");
                let (status, payload) = handle_auxiliary_dispatch(
                    &store,
                    &suite,
                    auxiliary_ops,
                    host_id,
                    op,
                    &body,
                );
                let _ = respond_api_json(req, status, &payload, crypto_tok);
            }
            (Method::Get, p)
                if p.starts_with("/api/chat/conversations/") && p.ends_with("/data-sources") =>
            {
                let conv_id = percent_decode_path(
                    p.trim_start_matches("/api/chat/conversations/")
                        .trim_end_matches("/data-sources"),
                );
                let bound = store
                    .get_kv(&format!("conv_data_sources:{conv_id}"))
                    .ok()
                    .flatten()
                    .unwrap_or_else(|| json!([]));
                let _ = respond_api_json(req, 200, &json!({"sources": bound}), crypto_tok);
            }
            (Method::Post, p) | (Method::Put, p)
                if p.starts_with("/api/chat/conversations/") && p.ends_with("/data-sources") =>
            {
                let conv_id = percent_decode_path(
                    p.trim_start_matches("/api/chat/conversations/")
                        .trim_end_matches("/data-sources"),
                );
                let _ = store.set_kv(&format!("conv_data_sources:{conv_id}"), &body);
                let _ = respond_api_json(req, 200, &json!({"ok": true, "sources": body}), crypto_tok);
            }
            (Method::Get, p)
                if p.starts_with("/api/chat/conversations/") && p.ends_with("/resources") =>
            {
                let conv_id = percent_decode_path(
                    p.trim_start_matches("/api/chat/conversations/")
                        .trim_end_matches("/resources"),
                );
                let resources = collect_conversation_resources(&store, &conv_id);
                let _ = respond_api_json(
                    req,
                    200,
                    &json!({"conversationId": conv_id, "resources": resources}),
                    crypto_tok,
                );
            }
            (Method::Post, p)
                if p.starts_with("/api/chat/conversations/") && p.ends_with("/resources") =>
            {
                let conv_id = percent_decode_path(
                    p.trim_start_matches("/api/chat/conversations/")
                        .trim_end_matches("/resources"),
                );
                match store_conversation_resource_upload(&store, &conv_id, &body) {
                    Ok(meta) => {
                        let _ = respond_api_json(req, 200, &meta, crypto_tok);
                    }
                    Err(msg) => {
                        let _ = respond_api_json(
                            req,
                            400,
                            &json!({"error": "invalid_resource", "message": msg}),
                            crypto_tok,
                        );
                    }
                }
            }
            (Method::Get, p)
                if p.starts_with("/api/chat/conversations/")
                    && p.contains("/resources/") =>
            {
                let rest = p.trim_start_matches("/api/chat/conversations/");
                if let Some((raw_conv, raw_res)) = rest.split_once("/resources/") {
                    let conv_id = percent_decode_path(raw_conv);
                    let resource_id = percent_decode_path(raw_res);
                    let resources = collect_conversation_resources_raw(&store, &conv_id);
                    if let Some(found) = resources.into_iter().find(|r| {
                        let candidates = [
                            r.get("resourceId").and_then(|v| v.as_str()),
                            r.get("id").and_then(|v| v.as_str()),
                            r.get("resource_id").and_then(|v| v.as_str()),
                            r.get("displayName").and_then(|v| v.as_str()),
                            r.get("name").and_then(|v| v.as_str()),
                        ];
                        candidates.into_iter().flatten().any(|c| c == resource_id)
                    }) {
                        let mime = found
                            .get("mimeType")
                            .or_else(|| found.get("mime_type"))
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.trim().is_empty())
                            .unwrap_or("application/pdf");
                        let display_name = found
                            .get("displayName")
                            .or_else(|| found.get("filename"))
                            .or_else(|| found.get("name"))
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.trim().is_empty())
                            .unwrap_or("document.pdf");
                        if let Some(raw_bytes) = extract_resource_raw_bytes(&found) {
                            let safe_name: String = display_name
                                .chars()
                                .filter(|c| *c != '"' && *c != '\\' && !c.is_control())
                                .take(200)
                                .collect();
                            let ct = Header::from_bytes(&b"Content-Type"[..], mime.as_bytes())
                                .unwrap_or_else(|_| {
                                    Header::from_bytes(
                                        &b"Content-Type"[..],
                                        &b"application/octet-stream"[..],
                                    )
                                    .unwrap()
                                });
                            let cd_val = format!("inline; filename=\"{safe_name}\"");
                            let cd =
                                Header::from_bytes(&b"Content-Disposition"[..], cd_val.as_bytes())
                                    .unwrap();
                            let nosniff = Header::from_bytes(
                                &b"X-Content-Type-Options"[..],
                                &b"nosniff"[..],
                            )
                            .unwrap();
                            let resp = Response::from_data(raw_bytes)
                                .with_status_code(StatusCode(200))
                                .with_header(ct)
                                .with_header(cd)
                                .with_header(nosniff);
                            let _ = req.respond(resp);
                        } else {
                            let _ = respond_api_json(req, 200, &found, crypto_tok);
                        }
                    } else {
                        let _ = respond_api_json(
                            req,
                            404,
                            &json!({"error": "resource_not_found", "resource_id": resource_id}),
                            crypto_tok,
                        );
                    }
                } else {
                    let _ = respond_api_json(
                        req,
                        404,
                        &json!({"error": "resource_not_found"}),
                        crypto_tok,
                    );
                }
            }
            (Method::Post, p)
                if p.starts_with("/api/chat/conversations/")
                    && p.ends_with("/messages/receipt") =>
            {
                let conv_id = percent_decode_path(
                    p.trim_start_matches("/api/chat/conversations/")
                        .trim_end_matches("/messages/receipt"),
                );
                let resp = handle_task_guidance_http(&store, host_id, &conv_id, &body, true);
                let _ = respond_api_json(req, 200, &resp, crypto_tok);
            }
            (Method::Post, p)
                if p.starts_with("/api/chat/conversations/") && p.ends_with("/messages") =>
            {
                let conv_id = percent_decode_path(
                    p.trim_start_matches("/api/chat/conversations/")
                        .trim_end_matches("/messages"),
                );
                let accepts_sse = header_value(&req, "accept")
                    .is_some_and(|v| v.to_ascii_lowercase().contains("text/event-stream"));
                let is_mobile = raw_path.starts_with("/api/mobile/");
                if is_assistant_journal_operation(&body) {
                    match handle_assistant_journal_http(&store, &suite, host_id, &body, is_mobile) {
                        Ok(payload) => {
                            let _ = respond_api_json(req, 200, &payload, crypto_tok);
                        }
                        Err((code, msg)) => {
                            let _ = respond_api_json(
                                req,
                                400,
                                &json!({"error": code, "message": msg}),
                                crypto_tok,
                            );
                        }
                    }
                    return;
                }
                if body.get("operation").and_then(|v| v.as_str()) == Some("task_child_window") {
                    let child_conv_id = body
                        .pointer("/destination/child_conversation_id")
                        .or_else(|| body.pointer("/destination/childConversationID"))
                        .or_else(|| body.get("child_conversation_id"))
                        .or_else(|| body.get("conversation_id"))
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.trim().is_empty())
                        .unwrap_or(&conv_id);
                    let query_args = json!({
                        "conversation_id": child_conv_id,
                        "before_ordinal": body.get("before_ordinal").cloned().unwrap_or(Value::Null),
                        "limit": body.get("limit").cloned().unwrap_or_else(|| json!(80))
                    });
                    match crate::daemon::dispatch_command(
                        &store,
                        &suite,
                        "load_conversation_projection_window_v1",
                        &query_args,
                    ) {
                        Ok(window) => {
                            let payload = if is_mobile || has_expected_host_id(&body) {
                                wrap_paired_envelope(&store, host_id, window)
                            } else {
                                window
                            };
                            let _ = respond_api_json(req, 200, &payload, crypto_tok);
                        }
                        Err((code, msg)) => {
                            let _ = respond_api_json(
                                req,
                                400,
                                &json!({"error": code, "message": msg}),
                                crypto_tok,
                            );
                        }
                    }
                    return;
                }
                if is_task_guidance_post(&body, accepts_sse) {
                    let receipt_only = body
                        .get("receipt_only")
                        .or_else(|| body.get("receiptOnly"))
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    let resp =
                        handle_task_guidance_http(&store, host_id, &conv_id, &body, receipt_only);
                    let _ = respond_api_json(req, 200, &resp, crypto_tok);
                    return;
                }
                if is_messages_read_post_with_accept(&body, accepts_sse) {
                    let (messages, events) = reconstruct_v2_messages(&store, &conv_id);
                    let latest_seq = events
                        .last()
                        .and_then(|e| e.get("seq").and_then(|s| s.as_i64()))
                        .unwrap_or(0);
                    let usage = session_usage_json(&store, &conv_id);
                    let inner = json!({
                        "conversationId": conv_id,
                        "messages": messages,
                        "events": events,
                        "usage": usage,
                        "hasMore": false,
                        "projectionRevision": 1,
                        "eventSequence": latest_seq
                    });
                    let payload = wrap_paired_result(&store, host_id, &body, inner);
                    let _ = respond_api_json(req, 200, &payload, crypto_tok);
                    return;
                }
                if let Some(perm) = body
                    .get("permissionProfile")
                    .or_else(|| body.get("permission_profile"))
                    .and_then(|v| v.as_str())
                {
                    if let Err(err) = PermissionProfile::parse_strict(perm) {
                        let _ = respond_api_json(
                            req,
                            400,
                            &json!({"error": "invalid_permission_profile", "message": err}),
                            crypto_tok,
                        );
                        return;
                    }
                }
                let store_c = store.clone();
                let suite_c = Arc::clone(&suite);
                let cancels_c = Arc::clone(active_cancels);
                let pending_c = Arc::clone(pending_interactions);
                let browser_c = Arc::clone(native_browser);
                let mcp_c = Arc::clone(mcp_manager);
                let host_id_c = host_id.clone();
                let crypto_tok_owned = crypto_tok.map(ToString::to_string);
                handle_sse_chat_turn(
                    req,
                    store_c,
                    suite_c,
                    cancels_c,
                    pending_c,
                    browser_c,
                    mcp_c,
                    host_id_c,
                    conv_id,
                    body,
                    crypto_tok_owned,
                );
            }
            (Method::Get, p)
                if p.starts_with("/api/chat/conversations/")
                    && !p.trim_start_matches("/api/chat/conversations/").contains('/') =>
            {
                let id = percent_decode_path(p.trim_start_matches("/api/chat/conversations/"));
                match store.get_session(&id) {
                    Ok(Some(sess)) => {
                        let conv = session_to_conversation_json(&store, &sess);
                        let _ = respond_api_json(req, 200, &conv, crypto_tok);
                    }
                    Ok(None) => {
                        let _ = respond_api_json(req, 404, &json!({"error": "conversation_not_found"}), crypto_tok);
                    }
                    Err(e) => {
                        let _ = respond_api_json(req, 500, &json!({"error": e.to_string()}), crypto_tok);
                    }
                }
            }
            (Method::Patch, p) | (Method::Post, p)
                if p.starts_with("/api/chat/conversations/")
                    && !p.trim_start_matches("/api/chat/conversations/").contains('/') =>
            {
                let id = percent_decode_path(p.trim_start_matches("/api/chat/conversations/"));
                if method == Method::Post
                    && body.get("identity").is_some_and(|v| v.is_object())
                    && body.get("request_digest").is_some_and(|v| v.is_string())
                {
                    let is_mobile = raw_path.starts_with("/api/mobile/");
                    let receipt = handle_remote_submission_reconciliation(
                        &store,
                        active_cancels,
                        host_id,
                        &id,
                        &body,
                    );
                    let payload = if is_mobile || has_expected_host_id(&body) {
                        wrap_paired_envelope(&store, host_id, receipt)
                    } else {
                        receipt
                    };
                    let _ = respond_api_json(req, 200, &payload, crypto_tok);
                    return;
                }
                if method == Method::Post && is_read_only_post(&body) {
                    match store.get_session(&id) {
                        Ok(Some(sess)) => {
                            let conv = session_to_conversation_json(&store, &sess);
                            let payload = wrap_paired_result(&store, host_id, &body, conv);
                            let _ = respond_api_json(req, 200, &payload, crypto_tok);
                        }
                        Ok(None) => {
                            let _ = respond_api_json(req, 404, &json!({"error": "conversation_not_found"}), crypto_tok);
                        }
                        Err(e) => {
                            let _ = respond_api_json(req, 500, &json!({"error": e.to_string()}), crypto_tok);
                        }
                    }
                    return;
                }
                match store.get_session(&id) {
                    Ok(Some(mut sess)) => {
                        let meta_key = session_meta_key(&id);
                        let mut meta = store
                            .get_kv(&meta_key)
                            .ok()
                            .flatten()
                            .unwrap_or_else(|| json!({"isPinned": false, "revision": 1, "configurationRevision": 1}));
                        if let Some(title) = body.get("title").and_then(|v| v.as_str()) {
                            sess.title = title.to_string();
                            let next_rev = meta.get("revision").and_then(|v| v.as_i64()).unwrap_or(1) + 1;
                            meta["revision"] = json!(next_rev);
                        }
                        let mut cfg_changed = false;
                        if let Some(src) = body.get("selectedModelSource").and_then(|v| v.as_str()) {
                            sess.model_connection = (!src.is_empty()).then(|| src.to_string());
                            cfg_changed = true;
                        }
                        if let Some(mid) = body.get("selectedModelId").and_then(|v| v.as_str()) {
                            sess.model_id = (!mid.is_empty()).then(|| mid.to_string());
                            cfg_changed = true;
                        }
                        if let Some(pinned) = body.get("isPinned").and_then(|v| v.as_bool()) {
                            meta["isPinned"] = json!(pinned);
                            cfg_changed = true;
                        }
                        if cfg_changed {
                            let next_cfg = meta
                                .get("configurationRevision")
                                .and_then(|v| v.as_i64())
                                .unwrap_or(1)
                                + 1;
                            meta["configurationRevision"] = json!(next_cfg);
                        }
                        sess.updated_at_ms = now_ms();
                        let _ = store.set_kv(&meta_key, &meta);
                        match store.upsert_session(&sess) {
                            Ok(()) => {
                                let conv = session_to_conversation_json(&store, &sess);
                                let payload = if method == Method::Post {
                                    wrap_paired_result(&store, host_id, &body, conv)
                                } else {
                                    conv
                                };
                                let _ = respond_api_json(req, 200, &payload, crypto_tok);
                            }
                            Err(e) => {
                                let _ = respond_api_json(req, 500, &json!({"error": e.to_string()}), crypto_tok);
                            }
                        }
                    }
                    Ok(None) => {
                        let _ = respond_api_json(req, 404, &json!({"error": "conversation_not_found"}), crypto_tok);
                    }
                    Err(e) => {
                        let _ = respond_api_json(req, 500, &json!({"error": e.to_string()}), crypto_tok);
                    }
                }
            }
            (Method::Delete, p)
                if p.starts_with("/api/chat/conversations/")
                    && !p.trim_start_matches("/api/chat/conversations/").contains('/') =>
            {
                let id = percent_decode_path(p.trim_start_matches("/api/chat/conversations/"));
                pending_interactions.clear_conversation(&id);
                match store.delete_session(&id) {
                    Ok(()) => {
                        let _ = respond_api_json(
                            req,
                            200,
                            &json!({"ok": true, "deleted": true, "conversationId": id}),
                            crypto_tok,
                        );
                    }
                    Err(e) => {
                        let _ = respond_api_json(req, 500, &json!({"error": e.to_string()}), crypto_tok);
                    }
                }
            }

            // --- Models & Local Model Runtime ---
            (Method::Get, "/api/models") | (Method::Get, "/api/model-profiles") => {
                let _ = respond_api_json(req, 200, &build_model_profiles_response(&store), crypto_tok);
            }
            (Method::Post, "/api/models/config") | (Method::Post, "/api/model-profiles") => {
                let mut cfg = load_active_config(&store);
                let conn_id = body
                    .get("connection_id")
                    .or_else(|| body.get("connectionId"))
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or("default")
                    .to_string();
                let endpoint = body
                    .get("endpoint")
                    .or_else(|| body.get("baseUrl"))
                    .or_else(|| body.get("base_url"))
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string);
                let model = body
                    .get("model")
                    .or_else(|| body.get("modelName"))
                    .or_else(|| body.get("modelId"))
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string);
                let protocol = body
                    .get("protocol")
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string);
                let api_key = body
                    .get("api_key")
                    .or_else(|| body.get("apiKey"))
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string);

                if let Some(existing) = cfg.llm.connections.iter_mut().find(|c| c.id == conn_id) {
                    if let Some(ep) = endpoint {
                        existing.endpoint = ep;
                    }
                    if let Some(proto) = protocol {
                        existing.protocol = Some(proto);
                    }
                    let clear_key = body
                        .get("clear_api_key")
                        .or_else(|| body.get("clearApiKey"))
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    if clear_key {
                        existing.api_key = None;
                    } else if let Some(key) = api_key {
                        // Preserve existing Vault key when incoming `api_key` is redacted `""`!
                        if !key.trim().is_empty() {
                            existing.api_key = Some(key);
                        }
                    }
                    if let Some(ref m) = model {
                        if !existing.models.iter().any(|e| e.id == *m) {
                            existing.models.push(ModelEntry {
                                id: m.clone(),
                                context_window: None,
                                max_output_tokens: None,
                                temperature: None,
                                top_p: None,
                                reasoning_effort: None,
                                capabilities: None,
                            });
                        }
                    }
                } else if endpoint.is_some() || model.is_some() {
                    let models = model
                        .as_ref()
                        .map(|m| {
                            vec![ModelEntry {
                                id: m.clone(),
                                context_window: None,
                                max_output_tokens: None,
                                temperature: None,
                                top_p: None,
                                reasoning_effort: None,
                                capabilities: None,
                            }]
                        })
                        .unwrap_or_default();
                    cfg.llm.connections.push(ConnectionConfig {
                        id: conn_id.clone(),
                        endpoint: endpoint.unwrap_or_default(),
                        protocol,
                        provider: None,
                        api_key: api_key.filter(|k| !k.trim().is_empty()),
                        api_key_env: None,
                        models,
                    });
                }
                if let Some(m) = model {
                    cfg.llm.default_model = Some(DefaultModelSelection {
                        connection: conn_id,
                        model: m,
                    });
                }
                if let Err(e) = save_applied_config(&store, &cfg) {
                    let _ = respond_api_json(req, 400, &json!({"error": e}), crypto_tok);
                } else {
                    let _ = respond_api_json(req, 200, &build_model_profiles_response(&store), crypto_tok);
                }
            }
            (Method::Post, "/api/model-profiles/catalog/replace") => {
                if let Some(endpoints) = body.get("endpoints").and_then(|v| v.as_array()) {
                    let mut cfg = load_active_config(&store);
                    let prev_conns: BTreeMap<
                        String,
                        (Option<String>, Option<String>, Option<String>),
                    > = cfg
                        .llm
                        .connections
                        .iter()
                        .map(|c| {
                            (
                                c.id.clone(),
                                (c.api_key.clone(), c.api_key_env.clone(), c.provider.clone()),
                            )
                        })
                        .collect();
                    let mut new_conns = Vec::new();
                    let mut new_conn_ids: HashSet<String> = HashSet::new();
                    let mut flattened_profiles = Vec::new();
                    let mut best_default: Option<(i64, String, String)> = None;

                    for (ep_idx, ep) in endpoints.iter().enumerate() {
                        let ep_id = ep
                            .get("id")
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.is_empty())
                            .map(ToString::to_string)
                            .unwrap_or_else(new_id);
                        new_conn_ids.insert(ep_id.clone());
                        let ep_name = ep
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or(&ep_id)
                            .to_string();
                        let base_url = ep
                            .get("baseURL")
                            .or_else(|| ep.get("url"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let wire_proto = ep
                            .get("wireProtocol")
                            .or_else(|| ep.get("protocol"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("chat_completions")
                            .to_string();
                        let ep_enabled = ep.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true);
                        let rust_proto = match wire_proto.as_str() {
                            "anthropic" => "anthropic",
                            "siwc_responses" => "siwc_responses",
                            "responses" | "openai_responses" => "openai_responses",
                            "gemini" | "google_gemini" => "gemini",
                            _ => "openai_chat",
                        };

                        let mut conn_models = Vec::new();
                        if let Some(models) = ep.get("models").and_then(|v| v.as_array()) {
                            for (m_idx, m) in models.iter().enumerate() {
                                let model_name = m
                                    .get("modelName")
                                    .or_else(|| m.get("modelId"))
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                if model_name.is_empty() {
                                    continue;
                                }
                                let prof_id = m
                                    .get("id")
                                    .and_then(|v| v.as_str())
                                    .filter(|s| !s.is_empty())
                                    .map(ToString::to_string)
                                    .unwrap_or_else(|| format!("{ep_id}:{model_name}"));
                                let display_name = m
                                    .get("displayName")
                                    .and_then(|v| v.as_str())
                                    .filter(|s| !s.is_empty())
                                    .unwrap_or(&model_name)
                                    .to_string();
                                let enabled = m.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true);
                                let priority = m.get("priority").and_then(|v| v.as_i64()).unwrap_or(50);
                                let caps: Vec<String> = m
                                    .get("capabilities")
                                    .and_then(|v| v.as_array())
                                    .map(|a| {
                                        a.iter()
                                            .filter_map(|x| x.as_str().map(ToString::to_string))
                                            .collect()
                                    })
                                    .unwrap_or_else(|| vec!["chat".into()]);
                                let params = m.get("parameters").cloned().unwrap_or_else(|| json!({}));

                                if ep_enabled && enabled && caps.iter().any(|c| c == "chat") {
                                    let replace = best_default
                                        .as_ref()
                                        .map(|(p, _, _)| priority > *p)
                                        .unwrap_or(true);
                                    if replace {
                                        best_default = Some((priority, ep_id.clone(), model_name.clone()));
                                    }
                                }

                                let inferred = infer_model_capabilities(&model_name, None);
                                let persisted_caps = m
                                    .get("executionCapabilities")
                                    .and_then(|v| v.as_object())
                                    .map(|ec| {
                                        let reasoning_flag = ec
                                            .get("reasoning")
                                            .and_then(|v| v.as_bool())
                                            .unwrap_or(inferred.reasoning);
                                        let supported_levels = m
                                            .pointer("/reasoning/supportedLevels")
                                            .and_then(|v| v.as_array())
                                            .map(|a| {
                                                a.iter()
                                                    .filter_map(|x| x.as_str().map(ToString::to_string))
                                                    .collect::<Vec<_>>()
                                            })
                                            .unwrap_or_else(|| {
                                                if reasoning_flag {
                                                    vec!["low".into(), "medium".into(), "high".into()]
                                                } else {
                                                    Vec::new()
                                                }
                                            });
                                        let supports_summary = m
                                            .pointer("/reasoning/supportsSummary")
                                            .and_then(|v| v.as_bool())
                                            .unwrap_or(reasoning_flag);
                                        ModelCapabilities {
                                            tool_calls: ec
                                                .get("toolCalls")
                                                .and_then(|v| v.as_bool())
                                                .unwrap_or(inferred.tool_calls),
                                            image_input: ec
                                                .get("imageInput")
                                                .and_then(|v| v.as_bool())
                                                .unwrap_or(inferred.image_input),
                                            native_pdf_input: ec
                                                .get("nativePdfInput")
                                                .and_then(|v| v.as_bool())
                                                .unwrap_or(inferred.native_pdf_input),
                                            reasoning: reasoning_flag,
                                            sampling_parameters: ec
                                                .get("samplingParameters")
                                                .and_then(|v| v.as_bool())
                                                .unwrap_or(inferred.sampling_parameters),
                                            stream_reasoning: reasoning_flag,
                                            supported_reasoning_levels: supported_levels,
                                            supports_reasoning_summary: supports_summary,
                                        }
                                    });
                                conn_models.push(ModelEntry {
                                    id: model_name.clone(),
                                    context_window: params
                                        .get("maxContextLength")
                                        .and_then(|v| v.as_u64())
                                        .map(|n| n as usize),
                                    max_output_tokens: params
                                        .get("maxOutputTokens")
                                        .and_then(|v| v.as_u64())
                                        .map(|n| n as usize),
                                    temperature: params.get("temperature").and_then(|v| v.as_f64()),
                                    top_p: params.get("topP").and_then(|v| v.as_f64()),
                                    reasoning_effort: None,
                                    capabilities: persisted_caps,
                                });

                                flattened_profiles.push(json!({
                                    "id": prof_id,
                                    "profile_id": prof_id,
                                    "provider": "openai-compatible",
                                    "connectionId": Value::Null,
                                    "modelId": model_name,
                                    "modelName": model_name,
                                    "model_name": model_name,
                                    "displayName": display_name,
                                    "display_name": display_name,
                                    "revision": 2,
                                    "enabled": enabled,
                                    "selected": false,
                                    "priority": priority,
                                    "order": m_idx,
                                    "capabilities": caps,
                                    "executionCapabilities": m.get("executionCapabilities").cloned().unwrap_or_else(|| json!({
                                        "toolCalls": inferred.tool_calls,
                                        "imageInput": inferred.image_input,
                                        "reasoning": inferred.reasoning,
                                        "samplingParameters": inferred.sampling_parameters,
                                        "samplingRequiresNoReasoning": false,
                                        "chatOutputTokenField": "auto"
                                    })),
                                    "executionCapabilityStatus": "tools_enabled",
                                    "reasoning": m.get("reasoning").cloned().unwrap_or_else(|| json!({
                                        "control": if inferred.reasoning { "automatic" } else { "unsupported" }
                                    })),
                                    "parameters": params,
                                    "protocol": wire_proto,
                                    "endpoint": {
                                        "id": ep_id,
                                        "name": ep_name,
                                        "url": base_url,
                                        "baseURL": base_url,
                                        "protocol": wire_proto,
                                        "wireProtocol": wire_proto,
                                        "enabled": ep_enabled,
                                        "order": ep_idx
                                    }
                                }));
                            }
                        }

                        let (prev_key, prev_env, prev_provider) = prev_conns
                            .get(&ep_id)
                            .cloned()
                            .unwrap_or((None, None, None));
                        new_conns.push(ConnectionConfig {
                            id: ep_id.clone(),
                            endpoint: base_url,
                            protocol: Some(rust_proto.into()),
                            provider: prev_provider,
                            api_key: prev_key,
                            api_key_env: prev_env,
                            models: conn_models,
                        });
                    }

                    if let Some((_, ref def_conn, ref def_model)) = best_default {
                        cfg.llm.default_model = Some(DefaultModelSelection {
                            connection: def_conn.clone(),
                            model: def_model.clone(),
                        });
                        for p in &mut flattened_profiles {
                            let ep_match = p.pointer("/endpoint/id").and_then(|v| v.as_str()) == Some(def_conn.as_str());
                            let m_match = p.get("modelName").and_then(|v| v.as_str()) == Some(def_model.as_str());
                            if ep_match && m_match {
                                p["selected"] = json!(true);
                            }
                        }
                    } else {
                        cfg.llm.default_model = None;
                    }
                    cfg.llm.connections = new_conns;
                    if let Err(e) = save_applied_config(&store, &cfg) {
                        let _ = respond_api_json(req, 400, &json!({"error": e}), crypto_tok);
                        return;
                    }
                    // Delete orphaned Vault keys for removed endpoints!
                    if let Ok(vault) = Vault::open(resolve_vault_path_for_store(&store)) {
                        for old_id in prev_conns.keys() {
                            if !new_conn_ids.contains(old_id) {
                                let _ = vault.delete_secret(&vault_secret_key(old_id));
                            }
                        }
                    }
                    let _ = store.set_kv("web_model_profiles", &Value::Array(flattened_profiles));
                } else if let Some(profiles) = body.get("profiles") {
                    let _ = store.set_kv("web_model_profiles", profiles);
                }
                let _ = respond_api_json(req, 200, &build_model_profiles_response(&store), crypto_tok);
            }

            // --- Local Models (Real Hardware, Engine Probe, Process Lifecycle & Recommendations) ---
            (Method::Get, "/api/local-models") => {
                let llama_status = probe_engine_status("llama.cpp");
                let vllm_status = probe_engine_status("vllm");
                let llama_ready = llama_status
                    .get("ready")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let vllm_ready = vllm_status
                    .get("ready")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let hardware = probe_hardware();
                let running = LocalModelProcessManager::global().list_running_models();
                let running_ids: HashSet<String> = running
                    .iter()
                    .filter_map(|r| r.get("id").and_then(|v| v.as_str()).map(ToString::to_string))
                    .collect();
                let cfg = load_active_config(&store);
                let selected_local_id = cfg
                    .llm
                    .default_model
                    .as_ref()
                    .and_then(|d| d.connection.strip_prefix("local:").map(ToString::to_string));
                let raw_items: Vec<Value> = suite.list_records("local_model").unwrap_or_default();
                let items: Vec<Value> = raw_items
                    .into_iter()
                    .map(|mut m| {
                        let id = m
                            .get("id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let is_running = running_ids.contains(&id);
                        let is_selected = selected_local_id.as_deref() == Some(id.as_str());
                        if let Some(obj) = m.as_object_mut() {
                            obj.insert("running".into(), json!(is_running));
                            obj.insert("selected".into(), json!(is_selected));
                            obj.entry("installed".to_string()).or_insert_with(|| json!(true));
                            obj.entry("deviceCompatible".to_string())
                                .or_insert_with(|| json!(true));
                            obj.entry("configurationRevision".to_string())
                                .or_insert_with(|| json!(1));
                            obj.entry("minimumContextWindowTokens".to_string())
                                .or_insert_with(|| json!(2048));
                            let mut default_params = json!({
                                "contextTokens": 8192,
                                "temperature": 0.7,
                                "topP": 0.9,
                                "topK": 40,
                                "minP": 0.05,
                                "batchSize": 512,
                                "threads": 4,
                                "gpuLayers": 0,
                                "flashAttention": false
                            });
                            if let (Some(dp), Some(existing_p)) = (
                                default_params.as_object_mut(),
                                obj.get("parameters").and_then(|v| v.as_object()),
                            ) {
                                for (k, v) in existing_p {
                                    dp.insert(k.clone(), v.clone());
                                }
                            }
                            obj.insert("parameters".into(), default_params);
                        }
                        m
                    })
                    .collect();
                let mut dl_status = ModelDownloadManager::global().current_status(&store);
                enrich_download_status_json(&store, &mut dl_status);
                let acquisition = if dl_status.get("status").and_then(|v| v.as_str()) == Some("idle") {
                    Value::Null
                } else {
                    dl_status.clone()
                };
                let _ = respond_api_json(
                    req,
                    200,
                    &json!({
                        "models": items,
                        "minimumContextWindowTokens": 2048,
                        "binaryAvailable": llama_ready,
                        "engineStatus": llama_status,
                        "runtimes": [
                            { "id": "llama.cpp", "binaryAvailable": llama_ready },
                            { "id": "vllm", "binaryAvailable": vllm_ready }
                        ],
                        "runningRuntimes": running,
                        "hardware": hardware,
                        "acquisition": acquisition,
                        "downloadStatus": dl_status
                    }),
                    crypto_tok,
                );
            }
            (Method::Get, "/api/local-models/search") => {
                let q = parse_query_str(query, "q")
                    .or_else(|| parse_query_str(query, "query"))
                    .unwrap_or_default();
                let eng = parse_query_str(query, "engine").unwrap_or_else(|| "llama.cpp".into());
                let mirror = parse_query_str(query, "mirror");
                match search_hub_models(&q, &eng, mirror.as_deref()) {
                    Ok(results) => {
                        let _ = respond_api_json(
                            req,
                            200,
                            &json!({"ok": true, "query": q, "engine": eng, "results": results, "models": results}),
                            crypto_tok,
                        );
                    }
                    Err(e) => {
                        let _ = respond_api_json(req, 400, &json!({"ok": false, "error": e}), crypto_tok);
                    }
                }
            }
            (Method::Post, "/api/local-models/search") => {
                let q = body
                    .get("query")
                    .or_else(|| body.get("q"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let eng = body
                    .get("engine")
                    .and_then(|v| v.as_str())
                    .unwrap_or("llama.cpp");
                let mirror = body
                    .get("mirror")
                    .or_else(|| body.get("mirror_base"))
                    .and_then(|v| v.as_str());
                match search_hub_models(q, eng, mirror) {
                    Ok(results) => {
                        let _ = respond_api_json(
                            req,
                            200,
                            &json!({"ok": true, "query": q, "engine": eng, "results": results, "models": results}),
                            crypto_tok,
                        );
                    }
                    Err(e) => {
                        let _ = respond_api_json(req, 400, &json!({"ok": false, "error": e}), crypto_tok);
                    }
                }
            }
            (Method::Get, "/api/local-models/hardware") => {
                let hw = probe_hardware();
                let _ = respond_api_json(
                    req,
                    200,
                    &json!({
                        "hardware": hw,
                        "os": hw["os"],
                        "arch": hw["arch"],
                        "cpu": hw["cpu"],
                        "cores": hw["cores"],
                        "cpu_cores": hw["cpu_cores"],
                        "memoryBytes": hw["memoryBytes"],
                        "availableMemoryBytes": hw["availableMemoryBytes"],
                        "freeDiskBytes": hw["freeDiskBytes"],
                        "gpu": hw["gpu"],
                        "gpuMemoryBytes": hw["gpuMemoryBytes"],
                        "unifiedMemory": hw["unifiedMemory"]
                    }),
                    crypto_tok,
                );
            }
            (Method::Get, "/api/local-models/engine-status") => {
                let eng = parse_query_str(query, "engine").unwrap_or_else(|| "llama.cpp".into());
                let st = probe_engine_status(&eng);
                let ready = st.get("ready").and_then(|v| v.as_bool()).unwrap_or(false);
                let _ = respond_api_json(
                    req,
                    200,
                    &json!({
                        "engineStatus": st,
                        "ready": ready,
                        "engines": ["llama.cpp", "ollama", "vllm", "mlx"]
                    }),
                    crypto_tok,
                );
            }
            (Method::Post, "/api/local-models/setup-engine") => {
                let eng = body
                    .get("engine")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .unwrap_or("llama.cpp");
                let opt_in = body
                    .get("opt_in_install")
                    .or_else(|| body.get("optInInstall"))
                    .or_else(|| body.get("autoInstall"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true);
                let setup = run_engine_setup_with_opt_in(eng, opt_in);
                let ready = setup.get("ready").and_then(|v| v.as_bool()).unwrap_or(false);
                let _ = respond_api_json(
                    req,
                    200,
                    &json!({
                        "ok": true,
                        "ready": ready,
                        "installed": setup.get("installed").and_then(|v| v.as_bool()).unwrap_or(ready),
                        "engineStatus": setup.get("status").cloned().unwrap_or(Value::Null),
                        "steps": setup.get("steps").cloned().unwrap_or_else(|| json!([])),
                        "message": setup.get("message").cloned().unwrap_or(Value::Null),
                        "setup": setup
                    }),
                    crypto_tok,
                );
            }
            (Method::Post, "/api/local-models/install-plan") => {
                let eng = body
                    .get("engine")
                    .or_else(|| body.pointer("/plan/engine"))
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .unwrap_or("llama.cpp")
                    .to_string();
                let source = body
                    .get("source")
                    .or_else(|| body.pointer("/plan/source"))
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .unwrap_or("huggingface")
                    .to_ascii_lowercase();
                let repo = body
                    .get("repository")
                    .or_else(|| body.pointer("/plan/repository"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                let first_file = body
                    .pointer("/plan/files/0")
                    .or_else(|| body.pointer("/files/0"));
                let file_name = first_file
                    .and_then(|f| {
                        f.get("filename")
                            .or_else(|| f.get("name"))
                            .or_else(|| f.get("path"))
                            .and_then(|v| v.as_str())
                    })
                    .or_else(|| {
                        body.pointer("/plan/variant")
                            .or_else(|| body.get("variant"))
                            .and_then(|v| v.as_str())
                    })
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(ToString::to_string);
                let url = body
                    .get("url")
                    .or_else(|| body.get("downloadUrl"))
                    .or_else(|| body.get("download_url"))
                    .or_else(|| body.pointer("/plan/url"))
                    .or_else(|| first_file.and_then(|f| f.get("url")))
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(ToString::to_string)
                    .or_else(|| build_local_model_download_url(&source, &repo, file_name.as_deref()));
                let Some(download_url) = url else {
                    let _ = respond_api_json(
                        req,
                        400,
                        &json!({"ok": false, "error": "missing_download_url"}),
                        crypto_tok,
                    );
                    return;
                };
                let model_name = body
                    .get("name")
                    .or_else(|| body.pointer("/plan/name"))
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(ToString::to_string)
                    .or_else(|| file_name.clone())
                    .or_else(|| (!repo.is_empty()).then(|| repo.clone()))
                    .unwrap_or_else(|| format!("model-{}", &new_id()[..8]));
                let requested_dest = body
                    .get("dest_path")
                    .or_else(|| body.get("destPath"))
                    .and_then(|v| v.as_str());
                let _ = fs::create_dir_all(managed_local_models_dir());
                let dest_path = confined_local_model_dest_path(requested_dest, &model_name)
                    .display()
                    .to_string();
                let expected_bytes = body
                    .get("expected_bytes")
                    .or_else(|| body.get("expectedBytes"))
                    .or_else(|| body.get("totalBytes"))
                    .or_else(|| first_file.and_then(|f| f.get("bytes")))
                    .and_then(|v| v.as_u64());
                let sha256 = body
                    .get("sha256")
                    .or_else(|| first_file.and_then(|f| f.get("sha256")))
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(ToString::to_string);
                let quantization = body
                    .get("quantization")
                    .or_else(|| body.pointer("/plan/quantization"))
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string);
                let plan = ModelDownloadPlan {
                    id: body
                        .get("id")
                        .or_else(|| body.get("downloadId"))
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string),
                    name: model_name,
                    engine: eng,
                    url: download_url,
                    dest_path,
                    sha256,
                    quantization,
                    expected_bytes,
                };
                match ModelDownloadManager::global().start_download(&store, plan) {
                    Ok(dl_state) => {
                        let _ = store.set_kv(
                            "local_model_download_meta",
                            &json!({
                                "id": dl_state.download_id,
                                "repository": repo,
                                "source": source
                            }),
                        );
                        let mut job = serde_json::to_value(&dl_state).unwrap_or_else(|_| json!({}));
                        if let Some(obj) = job.as_object_mut() {
                            obj.insert("id".into(), json!(dl_state.download_id));
                            obj.insert("repository".into(), json!(repo));
                            obj.insert("source".into(), json!(source));
                            if obj.get("totalBytes").is_none() || obj.get("totalBytes") == Some(&Value::Null) {
                                obj.insert("totalBytes".into(), json!(expected_bytes.unwrap_or(0)));
                            }
                        }
                        let _ = respond_api_json(
                            req,
                            200,
                            &json!({"ok": true, "job": job}),
                            crypto_tok,
                        );
                    }
                    Err(e) => {
                        let _ = respond_api_json(
                            req,
                            400,
                            &json!({"ok": false, "error": e}),
                            crypto_tok,
                        );
                    }
                }
            }
            (Method::Post, "/api/local-models/cancel") => {
                let current = ModelDownloadManager::global().current_status(&store);
                let dl_id = body
                    .get("id")
                    .or_else(|| body.get("downloadId"))
                    .or_else(|| body.get("download_id"))
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(ToString::to_string)
                    .or_else(|| {
                        current
                            .get("downloadId")
                            .or_else(|| current.get("id"))
                            .and_then(|v| v.as_str())
                            .map(ToString::to_string)
                    })
                    .unwrap_or_default();
                let res = ModelDownloadManager::global()
                    .cancel_download(&store, &dl_id)
                    .unwrap_or_else(|e| json!({"error": e}));
                let mut updated = ModelDownloadManager::global().current_status(&store);
                enrich_download_status_json(&store, &mut updated);
                let _ = respond_api_json(
                    req,
                    200,
                    &json!({"ok": true, "cancelled": true, "result": res, "job": updated}),
                    crypto_tok,
                );
            }
            (Method::Post, "/api/local-models/recommend") => {
                let hw = body
                    .get("hardware")
                    .filter(|v| v.is_object())
                    .cloned()
                    .unwrap_or_else(probe_hardware);
                let mut rec = recommend_local_models(&hw);
                let q_filter = body
                    .get("query")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_ascii_lowercase();
                let engine_filter = body
                    .get("engine")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_ascii_lowercase());
                let source_filter = body
                    .get("source")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_ascii_lowercase());
                if let Some(cands) = rec.get_mut("candidates").and_then(|v| v.as_array_mut()) {
                    // Append curated vLLM candidates so `engine == "vllm"` (or unfiltered) includes vLLM options!
                    cands.extend(build_vllm_recommend_candidates(&hw));
                    cands.retain_mut(|item| {
                        if let Some(ref eng) = engine_filter {
                            let item_eng = item
                                .pointer("/plan/engine")
                                .and_then(|v| v.as_str())
                                .unwrap_or("llama.cpp")
                                .to_ascii_lowercase();
                            if item_eng != *eng {
                                return false;
                            }
                        }
                        if let Some(ref src) = source_filter {
                            if let Some(item_src) = item
                                .pointer("/plan/source")
                                .or_else(|| item.get("source"))
                                .and_then(|v| v.as_str())
                            {
                                if !item_src.eq_ignore_ascii_case(src) {
                                    return false;
                                }
                            } else if let Some(plan_obj) =
                                item.get_mut("plan").and_then(|v| v.as_object_mut())
                            {
                                plan_obj.insert("source".into(), json!(src));
                            }
                        }
                        if !q_filter.is_empty() {
                            let repo = item
                                .get("repository")
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            let variant = item
                                .get("variant")
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            let hay = format!("{repo} {variant}").to_ascii_lowercase();
                            if !hay.contains(&q_filter) {
                                return false;
                            }
                        }
                        true
                    });
                }
                let _ = respond_api_json(req, 200, &rec, crypto_tok);
            }
            (Method::Post, p) if p.starts_with("/api/local-models/") && p.ends_with("/start") => {
                let id = percent_decode_path(
                    p.trim_start_matches("/api/local-models/")
                        .trim_end_matches("/start"),
                );
                let rec = suite.get_record::<Value>("local_model", &id).ok().flatten();
                let engine = body
                    .get("engine")
                    .and_then(|v| v.as_str())
                    .or_else(|| rec.as_ref().and_then(|r| r.get("engine").and_then(|v| v.as_str())))
                    .unwrap_or("llama.cpp");
                let model_path = body
                    .get("model_path")
                    .or_else(|| body.get("path"))
                    .and_then(|v| v.as_str())
                    .or_else(|| rec.as_ref().and_then(|r| r.get("path").and_then(|v| v.as_str())))
                    .unwrap_or("");
                let port_num = body
                    .get("port")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(8081) as u16;
                let extra_args: Vec<String> = body
                    .get("extra_args")
                    .or_else(|| body.get("args"))
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|x| x.as_str().map(ToString::to_string))
                            .collect()
                    })
                    .unwrap_or_default();
                match LocalModelProcessManager::global()
                    .start_local_model(&id, engine, model_path, port_num, &extra_args)
                {
                    Ok(started) => {
                        let _ = respond_api_json(
                            req,
                            200,
                            &json!({"ok": true, "runtime": started}),
                            crypto_tok,
                        );
                    }
                    Err(e) => {
                        let _ = respond_api_json(req, 400, &json!({"ok": false, "error": e}), crypto_tok);
                    }
                }
            }
            (Method::Post, p) if p.starts_with("/api/local-models/") && p.ends_with("/stop") => {
                let id = percent_decode_path(
                    p.trim_start_matches("/api/local-models/")
                        .trim_end_matches("/stop"),
                );
                match LocalModelProcessManager::global().stop_local_model(&id) {
                    Ok(res) => {
                        let _ = respond_api_json(req, 200, &json!({"ok": true, "result": res}), crypto_tok);
                    }
                    Err(e) => {
                        let _ = respond_api_json(req, 500, &json!({"ok": false, "error": e}), crypto_tok);
                    }
                }
            }
            (Method::Post, p) if p.starts_with("/api/local-models/") && p.ends_with("/select") => {
                let id = percent_decode_path(
                    p.trim_start_matches("/api/local-models/")
                        .trim_end_matches("/select"),
                );
                let running = LocalModelProcessManager::global().list_running_models();
                let run_ep = running
                    .iter()
                    .find(|r| r.get("id").and_then(|v| v.as_str()) == Some(id.as_str()))
                    .and_then(|r| r.get("endpoint").and_then(|v| v.as_str()))
                    .map(|ep| format!("{}/v1", ep.trim_end_matches('/')));
                let rec = suite.get_record::<Value>("local_model", &id).ok().flatten();
                let model_name = rec
                    .as_ref()
                    .and_then(|r| r.get("name").and_then(|v| v.as_str()))
                    .unwrap_or(&id)
                    .to_string();
                let endpoint = body
                    .get("endpoint")
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string)
                    .or(run_ep)
                    .unwrap_or_else(|| "http://127.0.0.1:8081/v1".into());
                let conn_id = format!("local:{id}");
                let mut cfg = load_active_config(&store);
                if let Some(existing) = cfg.llm.connections.iter_mut().find(|c| c.id == conn_id) {
                    existing.endpoint = endpoint;
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
                        endpoint,
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
                let _ = save_applied_config(&store, &cfg);
                let _ = respond_api_json(
                    req,
                    200,
                    &json!({"ok": true, "selectedId": id, "profiles": build_model_profiles_response(&store)}),
                    crypto_tok,
                );
            }
            (Method::Post, p) | (Method::Patch, p)
                if p.starts_with("/api/local-models/") && p.ends_with("/configuration") =>
            {
                let id = percent_decode_path(
                    p.trim_start_matches("/api/local-models/")
                        .trim_end_matches("/configuration"),
                );
                let mut existing = suite
                    .get_record::<Value>("local_model", &id)
                    .ok()
                    .flatten()
                    .unwrap_or_else(|| {
                        json!({
                            "id": id,
                            "name": id,
                            "engine": "llama.cpp",
                            "installed": true
                        })
                    });
                if let (Some(dst), Some(src)) = (existing.as_object_mut(), body.as_object()) {
                    let prev_rev = dst
                        .get("configurationRevision")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(1);
                    for (k, v) in src {
                        if k != "expectedRevision" && k != "configurationRevision" {
                            dst.insert(k.clone(), v.clone());
                        }
                    }
                    dst.insert("id".into(), json!(id));
                    dst.insert("configurationRevision".into(), json!(prev_rev + 1));
                    dst.insert("updated_at_ms".into(), json!(now_ms()));
                }
                match suite.upsert_record("local_model", &id, &existing) {
                    Ok(()) => {
                        let _ = respond_api_json(req, 200, &json!({"ok": true, "model": existing}), crypto_tok);
                    }
                    Err(e) => {
                        let _ = respond_api_json(req, 500, &json!({"error": e.to_string()}), crypto_tok);
                    }
                }
            }
            (Method::Delete, p)
                if p.starts_with("/api/local-models/") && p.ends_with("/installation") =>
            {
                let id = percent_decode_path(
                    p.trim_start_matches("/api/local-models/")
                        .trim_end_matches("/installation"),
                );
                let _ = LocalModelProcessManager::global().stop_local_model(&id);
                let deleted = suite.delete_record("local_model", &id).unwrap_or(false);
                let _ = respond_api_json(
                    req,
                    200,
                    &json!({"ok": true, "deleted": deleted, "id": id}),
                    crypto_tok,
                );
            }

            // --- Model Connections, Real Probe, Import & Gateway ---
            (Method::Get, "/api/model-connections") => {
                let inventory = build_model_connections_inventory(&store, &suite);
                let _ = respond_api_json(req, 200, &inventory, crypto_tok);
            }
            (Method::Post, "/api/model-connections/discover") => {
                let conn_id = body
                    .get("connection_id")
                    .or_else(|| body.get("connectionId"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let mut cfg = load_active_config(&store);
                if let Some(conn) = cfg.llm.connections.iter_mut().find(|c| c.id == conn_id) {
                    let models = discover_endpoint_models(conn);
                    for mid in &models {
                        if !conn.models.iter().any(|m| m.id == *mid) {
                            conn.models.push(ModelEntry {
                                id: mid.clone(),
                                context_window: None,
                                max_output_tokens: None,
                                temperature: None,
                                top_p: None,
                                reasoning_effort: None,
                                capabilities: None,
                            });
                        }
                    }
                    if !models.is_empty() {
                        let _ = save_applied_config(&store, &cfg);
                    }
                    let inventory = build_model_connections_inventory(&store, &suite);
                    let _ = respond_api_json(
                        req,
                        200,
                        &json!({
                            "ok": true,
                            "connection_id": conn_id,
                            "models": models,
                            "inventory": inventory
                        }),
                        crypto_tok,
                    );
                } else {
                    let _ = respond_api_json(
                        req,
                        404,
                        &json!({"error": "connection_not_found", "connection_id": conn_id}),
                        crypto_tok,
                    );
                }
            }
            (Method::Post, "/api/model-connections/discover/cancel") => {
                let _ = respond_api_json(req, 200, &json!({"cancelled": true}), crypto_tok);
            }
            (Method::Post, "/api/model-connections") => {
                let cmd_type = body.get("type").and_then(|v| v.as_str()).unwrap_or("");
                if cmd_type == "set_connection_capability_override" {
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
                    let protocol = body
                        .get("protocol")
                        .and_then(|v| v.as_str())
                        .unwrap_or("chat_completions");
                    let caps = body.get("capabilities").cloned().unwrap_or(Value::Null);
                    let kv_key = format!("model_cap_override:{conn_id}:{model_id}");
                    if caps.is_null() {
                        let _ = store.set_kv(&kv_key, &Value::Null);
                    } else {
                        let _ = store.set_kv(
                            &kv_key,
                            &json!({
                                "model_id": model_id,
                                "protocol": protocol,
                                "user_override": caps,
                                "updated_at_ms": now_ms()
                            }),
                        );
                    }
                } else if cmd_type == "configure_connection_models" {
                    let conn_id = body
                        .get("connection_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let enabled_ids: Vec<String> = body
                        .get("enabled_model_ids")
                        .and_then(|v| v.as_array())
                        .map(|a| {
                            a.iter()
                                .filter_map(|x| x.as_str().map(ToString::to_string))
                                .collect()
                        })
                        .unwrap_or_default();
                    let selected_id = body
                        .get("selected_model_id")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string);
                    let mut cfg = load_active_config(&store);
                    if let Some(conn) = cfg.llm.connections.iter_mut().find(|c| c.id == conn_id) {
                        for key in &enabled_ids {
                            let model_id = key.split_once(':').map(|(_, m)| m).unwrap_or(key.as_str());
                            if !model_id.is_empty() && !conn.models.iter().any(|m| m.id == model_id) {
                                conn.models.push(ModelEntry {
                                    id: model_id.to_string(),
                                    context_window: None,
                                    max_output_tokens: None,
                                    temperature: None,
                                    top_p: None,
                                    reasoning_effort: None,
                                    capabilities: None,
                                });
                            }
                        }
                        if let Some(sel) = selected_id {
                            let model_id = sel.split_once(':').map(|(_, m)| m).unwrap_or(sel.as_str());
                            if !model_id.is_empty() {
                                cfg.llm.default_model = Some(DefaultModelSelection {
                                    connection: conn_id.to_string(),
                                    model: model_id.to_string(),
                                });
                            }
                        }
                        let _ = save_applied_config(&store, &cfg);
                    }
                    let _ = store.set_kv(
                        &format!("model_conn_selections:{conn_id}"),
                        &json!({"enabled": enabled_ids}),
                    );
                } else {
                    let conn_id = body
                        .get("id")
                        .or_else(|| body.get("connection_id"))
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.trim().is_empty())
                        .map(ToString::to_string)
                        .unwrap_or_else(new_id);
                    let mut sanitized = body.clone();
                    if let Some(obj) = sanitized.as_object_mut() {
                        obj.insert("id".into(), json!(conn_id));
                        obj.remove("api_key");
                        obj.remove("apiKey");
                        obj.insert("updated_at_ms".into(), json!(now_ms()));
                    }
                    let _ = suite.upsert_record("model_connection", &conn_id, &sanitized);
                    if let Some(base_url) = body
                        .get("base_url")
                        .or_else(|| body.get("endpoint"))
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.trim().is_empty())
                    {
                        let mut cfg = load_active_config(&store);
                        let api_key = body
                            .get("api_key")
                            .or_else(|| body.get("apiKey"))
                            .and_then(|v| v.as_str())
                            .map(ToString::to_string);
                        let protocol = body
                            .get("protocol")
                            .and_then(|v| v.as_str())
                            .map(ToString::to_string);
                        if let Some(existing) = cfg.llm.connections.iter_mut().find(|c| c.id == conn_id) {
                            existing.endpoint = base_url.to_string();
                            if protocol.is_some() {
                                existing.protocol = protocol;
                            }
                            if api_key.is_some() {
                                existing.api_key = api_key;
                            }
                        } else {
                            cfg.llm.connections.push(ConnectionConfig {
                                id: conn_id,
                                endpoint: base_url.to_string(),
                                protocol,
                                provider: None,
                                api_key,
                                api_key_env: None,
                                models: Vec::new(),
                            });
                        }
                        let _ = save_applied_config(&store, &cfg);
                    }
                }
                let inventory = build_model_connections_inventory(&store, &suite);
                let _ = respond_api_json(req, 200, &inventory, crypto_tok);
            }
            (Method::Post, "/api/model-connections/kimi-login/owners")
            | (Method::Post, "/api/model-connections/opencode-login/owners")
            | (Method::Get, "/api/model-connections/source-login/status")
            | (Method::Post, "/api/model-connections/source-login/start")
            | (Method::Post, "/api/model-connections/source-login/status")
            | (Method::Post, "/api/model-connections/source-login/cancel")
            | (Method::Post, "/api/model-connections/source-accounts/bind")
            | (Method::Post, "/api/model-connections/source-accounts/refresh")
            | (Method::Get, "/api/model-connections/kimi-login/status")
            | (Method::Post, "/api/model-connections/kimi-login/connect")
            | (Method::Post, "/api/model-connections/kimi-login/start")
            | (Method::Post, "/api/model-connections/kimi-login/status")
            | (Method::Post, "/api/model-connections/kimi-login/cancel")
            | (Method::Get, "/api/model-connections/opencode-login/inspect")
            | (Method::Get, "/api/model-connections/opencode-login/callback")
            | (Method::Post, "/api/model-connections/opencode-login/connect")
            | (Method::Post, "/api/model-connections/opencode-login/inspect")
            | (Method::Post, "/api/model-connections/opencode-login/authorize")
            | (Method::Post, "/api/model-connections/opencode-login/callback")
            | (Method::Post, "/api/model-connections/opencode-login/forget") => {
                let (status, payload) = handle_source_oauth_route(
                    &store,
                    &suite,
                    oauth_flows,
                    path,
                    query,
                    &body,
                );
                let _ = respond_api_json(req, status, &payload, crypto_tok);
            }
            (Method::Post, "/api/model-connections/probe") => {
                let probe_res = run_real_model_connection_probe(&store, &body);
                let _ = respond_api_json(req, 200, &probe_res, crypto_tok);
            }
            (Method::Post, "/api/model-import/plan") => {
                let plan_id = new_id();
                let expires_at_ms = now_ms().saturating_add(300_000);
                let source = body
                    .get("source")
                    .and_then(|v| v.as_str())
                    .unwrap_or("open_code");
                let path_str = body
                    .get("path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim();
                let mut candidates: Vec<Value> = Vec::new();
                if !path_str.is_empty() {
                    let target_path = Path::new(path_str);
                    // SEC-7: Reject sensitive paths (~/.ssh, ~/.nola/vault*, .env, etc.) and symlinks/>1MiB!
                    if is_sensitive_local_path(target_path) {
                        let _ = respond_api_json(
                            req,
                            403,
                            &json!({"error": "forbidden_path", "message": "refusing to read sensitive local path"}),
                            crypto_tok,
                        );
                        return;
                    }
                    match read_config_file_safe(target_path) {
                        Ok(raw) => {
                            if let Ok(parsed) = serde_json::from_str::<Value>(&raw) {
                                if let Some(providers) =
                                    parsed.get("provider").and_then(|v| v.as_object())
                                {
                                    for (prov_id, prov_val) in providers {
                                        let base_url = prov_val
                                            .pointer("/options/baseURL")
                                            .or_else(|| prov_val.get("baseURL"))
                                            .or_else(|| prov_val.get("base_url"))
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("")
                                            .to_string();
                                        if base_url.is_empty() {
                                            continue;
                                        }
                                        let models: Vec<Value> = prov_val
                                            .get("models")
                                            .and_then(|v| v.as_object())
                                            .map(|m_map| {
                                                m_map
                                                    .keys()
                                                    .map(|mid| {
                                                        json!({
                                                            "model_id": mid,
                                                            "display_name": mid,
                                                            "protocol": "chat_completions",
                                                            "context_window": Value::Null,
                                                            "modalities": ["text"],
                                                            "reasoning_levels": []
                                                        })
                                                    })
                                                    .collect()
                                            })
                                            .unwrap_or_default();
                                        candidates.push(json!({
                                            "candidate_id": format!("imp-{prov_id}"),
                                            "connection": {
                                                "id": prov_id,
                                                "display_name": prov_id,
                                                "base_url": base_url,
                                                "protocols": ["chat_completions"],
                                                "source": source,
                                                "source_id": Value::Null,
                                                "account_id": Value::Null,
                                                "enabled": true
                                            },
                                            "models": models,
                                            "credential_status": "not_requested",
                                            "issue": Value::Null,
                                            "existing_revision": Value::Null
                                        }));
                                    }
                                }
                            }
                        }
                        Err(e) if e.contains("symlink") || e.contains("size limit") => {
                            let _ = respond_api_json(
                                req,
                                400,
                                &json!({"error": "invalid_import_file", "message": e}),
                                crypto_tok,
                            );
                            return;
                        }
                        Err(_) => {}
                    }
                }
                let plan = json!({
                    "plan_id": plan_id,
                    "expires_at_ms": expires_at_ms,
                    "candidates": candidates
                });
                let _ = store.set_kv(&format!("model_import_plan:{plan_id}"), &plan);
                let _ = respond_api_json(req, 200, &plan, crypto_tok);
            }
            (Method::Post, "/api/model-import/commit") => {
                let plan_id = body
                    .get("plan_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let plan = store
                    .get_kv(&format!("model_import_plan:{plan_id}"))
                    .ok()
                    .flatten();
                let Some(plan_val) = plan else {
                    let _ = respond_api_json(
                        req,
                        404,
                        &json!({"error": "import_plan_not_found"}),
                        crypto_tok,
                    );
                    return;
                };
                let candidates = plan_val
                    .get("candidates")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default();
                let selections = body
                    .get("selections")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default();
                let mut cfg = load_active_config(&store);
                let mut results: Vec<Value> = Vec::new();
                for sel in selections {
                    let cid = sel
                        .get("candidate_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    if let Some(cand) = candidates
                        .iter()
                        .find(|c| c.get("candidate_id").and_then(|v| v.as_str()) == Some(cid))
                    {
                        let conn_id = cand
                            .pointer("/connection/id")
                            .and_then(|v| v.as_str())
                            .unwrap_or(cid)
                            .to_string();
                        let base_url = cand
                            .pointer("/connection/base_url")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let models: Vec<ModelEntry> = cand
                            .get("models")
                            .and_then(|v| v.as_array())
                            .map(|arr| {
                                arr.iter()
                                    .filter_map(|m| {
                                        m.get("model_id").and_then(|v| v.as_str()).map(|id| {
                                            ModelEntry {
                                                id: id.to_string(),
                                                context_window: None,
                                                max_output_tokens: None,
                                                temperature: None,
                                                top_p: None,
                                                reasoning_effort: None,
                                                capabilities: None,
                                            }
                                        })
                                    })
                                    .collect()
                            })
                            .unwrap_or_default();
                        if let Some(existing) =
                            cfg.llm.connections.iter_mut().find(|c| c.id == conn_id)
                        {
                            existing.endpoint = base_url;
                            for m in models {
                                if !existing.models.iter().any(|e| e.id == m.id) {
                                    existing.models.push(m);
                                }
                            }
                        } else {
                            cfg.llm.connections.push(ConnectionConfig {
                                id: conn_id.clone(),
                                endpoint: base_url,
                                protocol: Some("chat_completions".into()),
                                provider: Some("import".into()),
                                api_key: None,
                                api_key_env: None,
                                models,
                            });
                        }
                        results.push(json!({
                            "candidate_id": cid,
                            "connection_id": conn_id,
                            "status": "imported",
                            "error": Value::Null
                        }));
                    } else {
                        results.push(json!({
                            "candidate_id": cid,
                            "connection_id": Value::Null,
                            "status": "skipped",
                            "error": "candidate_not_found"
                        }));
                    }
                }
                if !results.is_empty() {
                    let _ = save_applied_config(&store, &cfg);
                }
                let _ = respond_api_json(req, 200, &Value::Array(results), crypto_tok);
            }
            (Method::Post, p)
                if p.starts_with("/api/model-endpoints/") && p.ends_with("/discover") =>
            {
                let ep_id = percent_decode_path(
                    p.trim_start_matches("/api/model-endpoints/")
                        .trim_end_matches("/discover"),
                );
                let cfg = load_active_config(&store);
                if let Some(conn) = cfg.llm.connections.iter().find(|c| c.id == ep_id) {
                    let models = discover_endpoint_models(conn);
                    let _ = respond_api_json(req, 200, &json!({"models": models}), crypto_tok);
                } else {
                    let _ = respond_api_json(req, 404, &json!({"error": "endpoint_not_found"}), crypto_tok);
                }
            }
            (Method::Get, p)
                if p.starts_with("/api/model-endpoints/") && p.ends_with("/credential") =>
            {
                let ep_id = percent_decode_path(
                    p.trim_start_matches("/api/model-endpoints/")
                        .trim_end_matches("/credential"),
                );
                if let Some(payload) = build_endpoint_credential_response(&store, &ep_id) {
                    let _ = respond_api_json(req, 200, &payload, crypto_tok);
                } else {
                    let _ = respond_api_json(req, 404, &json!({"error": "endpoint_not_found"}), crypto_tok);
                }
            }
            (Method::Post, p) | (Method::Put, p)
                if p.starts_with("/api/model-endpoints/") && p.ends_with("/credential") =>
            {
                let ep_id = percent_decode_path(
                    p.trim_start_matches("/api/model-endpoints/")
                        .trim_end_matches("/credential"),
                );
                let action = body
                    .get("action")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim();
                let api_key = body
                    .get("apiKey")
                    .or_else(|| body.get("api_key"))
                    .or_else(|| body.get("secret"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.trim().to_string());
                let mut cfg = load_active_config(&store);
                if let Some(conn) = cfg.llm.connections.iter_mut().find(|c| c.id == ep_id) {
                    if action == "clear" {
                        conn.api_key = None;
                    } else {
                        conn.api_key = api_key.filter(|s| !s.is_empty());
                    }
                    if let Err(e) = save_applied_config(&store, &cfg) {
                        let _ = respond_api_json(req, 500, &json!({"error": e}), crypto_tok);
                    } else {
                        bump_endpoint_credential_revision(&store, &ep_id);
                        let payload = build_endpoint_credential_response(&store, &ep_id)
                            .unwrap_or_else(|| json!({"ok": true, "endpointID": ep_id}));
                        let _ = respond_api_json(req, 200, &payload, crypto_tok);
                    }
                } else {
                    let _ = respond_api_json(req, 404, &json!({"error": "endpoint_not_found"}), crypto_tok);
                }
            }
            (Method::Delete, p)
                if p.starts_with("/api/model-endpoints/") && p.ends_with("/credential") =>
            {
                let ep_id = percent_decode_path(
                    p.trim_start_matches("/api/model-endpoints/")
                        .trim_end_matches("/credential"),
                );
                let mut cfg = load_active_config(&store);
                if let Some(conn) = cfg.llm.connections.iter_mut().find(|c| c.id == ep_id) {
                    conn.api_key = None;
                    let _ = save_applied_config(&store, &cfg);
                    bump_endpoint_credential_revision(&store, &ep_id);
                }
                let payload = build_endpoint_credential_response(&store, &ep_id).unwrap_or_else(
                    || {
                        json!({
                            "ok": true,
                            "endpointID": ep_id,
                            "endpointId": ep_id,
                            "baseURL": "",
                            "wireProtocol": "chat_completions",
                            "hasCredential": false,
                            "credentialRevision": 1,
                            "expectedRevisions": { &ep_id: 1 },
                            "configured": false,
                            "hasApiKey": false
                        })
                    },
                );
                let _ = respond_api_json(req, 200, &payload, crypto_tok);
            }
            (Method::Post, p) | (Method::Patch, p)
                if p.starts_with("/api/model-endpoints/") && p.ends_with("/configuration") =>
            {
                let ep_id = percent_decode_path(
                    p.trim_start_matches("/api/model-endpoints/")
                        .trim_end_matches("/configuration"),
                );
                let new_name = body.get("name").and_then(|v| v.as_str()).map(ToString::to_string);
                let new_enabled = body.get("enabled").and_then(|v| v.as_bool());
                let new_url = body
                    .get("baseURL")
                    .or_else(|| body.get("url"))
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string);
                let new_proto = body
                    .get("wireProtocol")
                    .or_else(|| body.get("protocol"))
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string);
                let mut cfg = load_active_config(&store);
                if let Some(conn) = cfg.llm.connections.iter_mut().find(|c| c.id == ep_id) {
                    if let Some(ref url) = new_url {
                        conn.endpoint = url.clone();
                    }
                    if let Some(ref proto) = new_proto {
                        conn.protocol = Some(match proto.as_str() {
                            "anthropic" => "anthropic".into(),
                            "siwc_responses" => "siwc_responses".into(),
                            "responses" | "openai_responses" => "openai_responses".into(),
                            "gemini" | "google_gemini" => "gemini".into(),
                            _ => "openai_chat".into(),
                        });
                    }
                    let _ = save_applied_config(&store, &cfg);
                }
                if let Ok(Some(Value::Array(mut profiles))) = store.get_kv("web_model_profiles") {
                    let mut changed = false;
                    for p_val in &mut profiles {
                        if p_val.pointer("/endpoint/id").and_then(|v| v.as_str())
                            == Some(ep_id.as_str())
                        {
                            changed = true;
                            if let Some(ep_obj) =
                                p_val.get_mut("endpoint").and_then(|v| v.as_object_mut())
                            {
                                if let Some(ref n) = new_name {
                                    ep_obj.insert("name".into(), json!(n));
                                }
                                if let Some(en) = new_enabled {
                                    ep_obj.insert("enabled".into(), json!(en));
                                }
                                if let Some(ref url) = new_url {
                                    ep_obj.insert("baseURL".into(), json!(url));
                                    ep_obj.insert("url".into(), json!(url));
                                }
                                if let Some(ref proto) = new_proto {
                                    ep_obj.insert("wireProtocol".into(), json!(proto));
                                    ep_obj.insert("protocol".into(), json!(proto));
                                }
                            }
                            if let Some(p_obj) = p_val.as_object_mut() {
                                if let Some(ref proto) = new_proto {
                                    p_obj.insert("protocol".into(), json!(proto));
                                }
                                let rev = p_obj
                                    .get("revision")
                                    .and_then(|v| v.as_i64())
                                    .unwrap_or(1);
                                p_obj.insert("revision".into(), json!(rev + 1));
                            }
                        }
                    }
                    if changed {
                        let _ = store.set_kv("web_model_profiles", &Value::Array(profiles));
                    }
                }
                let _ = respond_api_json(req, 200, &build_model_profiles_response(&store), crypto_tok);
            }
            (Method::Get, "/api/model-gateway/clients") => {
                let clients = store
                    .get_kv("model_gateway_clients")
                    .ok()
                    .flatten()
                    .unwrap_or_else(|| json!([]));
                let _ = respond_api_json(req, 200, &clients, crypto_tok);
            }
            (Method::Post, "/api/model-gateway/clients/plan") => {
                let source = body
                    .get("source")
                    .and_then(|v| v.as_str())
                    .unwrap_or("open_code");
                let path_str = body
                    .get("path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim();
                let model_id = body
                    .get("model_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let plan_id = new_id();
                let expires_at_ms = now_ms().saturating_add(300_000);
                let plan = json!({
                    "plan_id": plan_id,
                    "source": source,
                    "path": path_str,
                    "model_id": model_id,
                    "owned_fields": ["base_url", "api_key", "model"],
                    "expires_at_ms": expires_at_ms
                });
                let _ = store.set_kv(&format!("gateway_client_plan:{plan_id}"), &plan);
                let _ = respond_api_json(req, 200, &plan, crypto_tok);
            }
            (Method::Post, "/api/model-gateway/clients/commit") => {
                let plan_id = body
                    .get("plan_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let Some(plan) = store
                    .get_kv(&format!("gateway_client_plan:{plan_id}"))
                    .ok()
                    .flatten()
                else {
                    let _ = respond_api_json(
                        req,
                        404,
                        &json!({"error": "gateway_client_plan_not_found"}),
                        crypto_tok,
                    );
                    return;
                };
                let receipt = json!({
                    "receipt_id": new_id(),
                    "source": plan.get("source").cloned().unwrap_or_else(|| json!("open_code")),
                    "path": plan.get("path").cloned().unwrap_or_else(|| json!("")),
                    "model_id": plan.get("model_id").cloned().unwrap_or_else(|| json!("")),
                    "owned_fields": plan.get("owned_fields").cloned().unwrap_or_else(|| json!(["base_url", "api_key", "model"]))
                });
                let mut clients: Vec<Value> = store
                    .get_kv("model_gateway_clients")
                    .ok()
                    .flatten()
                    .and_then(|v| v.as_array().cloned())
                    .unwrap_or_default();
                clients.push(receipt.clone());
                let _ = store.set_kv("model_gateway_clients", &Value::Array(clients));
                let _ = respond_api_json(req, 200, &receipt, crypto_tok);
            }
            (Method::Post, "/api/model-gateway/clients/disconnect") => {
                let receipt_id = body
                    .get("receipt_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let mut clients: Vec<Value> = store
                    .get_kv("model_gateway_clients")
                    .ok()
                    .flatten()
                    .and_then(|v| v.as_array().cloned())
                    .unwrap_or_default();
                let before = clients.len();
                clients.retain(|c| c.get("receipt_id").and_then(|v| v.as_str()) != Some(receipt_id));
                let removed = clients.len() < before;
                let _ = store.set_kv("model_gateway_clients", &Value::Array(clients));
                let _ = respond_api_json(
                    req,
                    200,
                    &json!({"ok": true, "disconnected": removed, "receipt_id": receipt_id}),
                    crypto_tok,
                );
            }
            (Method::Get, "/api/model-gateway/access") => {
                let access = store
                    .get_kv("model_gateway_access")
                    .ok()
                    .flatten()
                    .unwrap_or_else(|| json!({"revision": 1, "enabled": false}));
                let _ = respond_api_json(req, 200, &access, crypto_tok);
            }
            (Method::Post, "/api/model-gateway/access") => {
                let revoke = body.get("revoke").and_then(|v| v.as_bool()).unwrap_or(false);
                let prev_rev = store
                    .get_kv("model_gateway_access")
                    .ok()
                    .flatten()
                    .and_then(|v| v.get("revision").and_then(|r| r.as_i64()))
                    .unwrap_or(1);
                let next_rev = prev_rev + 1;
                let saved = json!({"revision": next_rev, "enabled": !revoke});
                let _ = store.set_kv("model_gateway_access", &saved);
                if revoke {
                    let _ = store.set_kv("gateway_token_hash", &Value::Null);
                    let _ = respond_api_json(req, 200, &saved, crypto_tok);
                } else {
                    let token = format!("nola_gw_{}", generate_hex_token_32());
                    let token_hash = sha256_hex(token.as_bytes());
                    let _ = store.set_kv("gateway_token_hash", &json!(token_hash));
                    let _ = respond_api_json(
                        req,
                        200,
                        &json!({"revision": next_rev, "enabled": true, "token": token}),
                        crypto_tok,
                    );
                }
            }
            (Method::Get, "/api/model-gateway/attempts") => {
                let records: Vec<Value> = suite
                    .list_records::<Value>("gateway_attempt")
                    .unwrap_or_default()
                    .into_iter()
                    .take(MAX_GATEWAY_ATTEMPTS)
                    .map(|rec| json!({ "record": rec }))
                    .collect();
                let _ = respond_api_json(req, 200, &Value::Array(records), crypto_tok);
            }
            (Method::Get, "/api/agent/skill-settings-review") => {
                let _ = respond_api_json(
                    req,
                    200,
                    &json!({
                        "pending": false,
                        "expected_revision": 1,
                        "previous": Value::Null,
                        "current": [],
                        "items": []
                    }),
                    crypto_tok,
                );
            }
            (Method::Post, "/api/agent/skill-settings-review") => {
                let _ = respond_api_json(req, 200, &json!({"ok": true, "pending": false}), crypto_tok);
            }

            // --- Agent Settings, Tools, Defaults & Permission Rules ---
            (Method::Get, "/api/agent/settings") => {
                let expected_host = header_value(&req, "x-nola-expected-host-id")
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(ToString::to_string)
                    .or_else(|| {
                        parse_query_str(query, "expected_host_id")
                            .or_else(|| parse_query_str(query, "expectedHostId"))
                            .filter(|s| !s.trim().is_empty())
                    });
                if let Some(exp) = expected_host {
                    if exp != host_id.as_str() {
                        let _ = respond_api_json(
                            req,
                            403,
                            &json!({
                                "error": "host_identity_mismatch",
                                "source_host_id": host_id
                            }),
                            crypto_tok,
                        );
                        return;
                    }
                }
                let _ = respond_api_json(req, 200, &build_agent_settings_snapshot(&store, &suite), crypto_tok);
            }
            (Method::Post, "/api/agent/settings") => {
                let expected_host = header_value(&req, "x-nola-expected-host-id")
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(ToString::to_string)
                    .or_else(|| {
                        parse_query_str(query, "expected_host_id")
                            .or_else(|| parse_query_str(query, "expectedHostId"))
                            .filter(|s| !s.trim().is_empty())
                    })
                    .or_else(|| {
                        body.get("expected_host_id")
                            .or_else(|| body.get("expectedHostId"))
                            .and_then(|v| v.as_str())
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .map(ToString::to_string)
                    });
                if let Some(exp) = expected_host {
                    if exp != host_id.as_str() {
                        let _ = respond_api_json(
                            req,
                            403,
                            &json!({
                                "error": "host_identity_mismatch",
                                "source_host_id": host_id
                            }),
                            crypto_tok,
                        );
                        return;
                    }
                }
                if let Some(perm) = body.get("permission_profile").and_then(|v| v.as_str()) {
                    if let Err(err) = PermissionProfile::parse_strict(perm) {
                        let _ = respond_api_json(
                            req,
                            400,
                            &json!({"error": "invalid_permission_profile", "message": err}),
                            crypto_tok,
                        );
                        return;
                    }
                }
                if let Some(mcp_arr) = body.get("mcp_servers").and_then(|v| v.as_array()) {
                    let mut kept_ids: HashSet<String> = HashSet::new();
                    for entry in mcp_arr {
                        let cfg = parse_mcp_server_config(entry, None);
                        kept_ids.insert(cfg.id.clone());
                        if store.upsert_mcp_server(&cfg).is_ok() {
                            mcp_manager.evict(&cfg.id);
                        }
                    }
                    for existing in store.list_mcp_servers().unwrap_or_default() {
                        if !kept_ids.contains(&existing.id) {
                            mcp_manager.disconnect(&existing.id);
                            let _ = store.delete_mcp_server(&existing.id);
                        }
                    }
                }
                if let Some(skills_arr) = body.get("skills").and_then(|v| v.as_array()) {
                    for sk in skills_arr {
                        if let Some(name) = sk
                            .get("name")
                            .or_else(|| sk.get("id"))
                            .and_then(|v| v.as_str())
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                        {
                            let enabled = sk.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true);
                            let _ = nola_modules::skills::set_skill_enabled(&store, name, enabled);
                        }
                    }
                }
                if let Some(disabled_arr) = body.get("disabled_skills").and_then(|v| v.as_array()) {
                    let disabled_set: HashSet<String> = disabled_arr
                        .iter()
                        .filter_map(|v| v.as_str().map(|s| s.trim().to_ascii_lowercase()))
                        .filter(|s| !s.is_empty())
                        .collect();
                    for sk in nola_modules::skills::discover_skills(
                        &suite.workspace_root,
                        suite.user_home.as_deref(),
                    ) {
                        let enabled = !disabled_set.contains(&sk.name.to_ascii_lowercase());
                        let _ = nola_modules::skills::set_skill_enabled(&store, &sk.name, enabled);
                    }
                    for name in &disabled_set {
                        let _ = nola_modules::skills::set_skill_enabled(&store, name, false);
                    }
                }
                let mut merged = build_agent_settings_snapshot(&store, &suite);
                if let (Some(dst), Some(src)) = (merged.as_object_mut(), body.as_object()) {
                    for (k, v) in src {
                        if !k.ends_with("_baseline") {
                            dst.insert(k.clone(), v.clone());
                        }
                    }
                }
                let _ = store.set_kv("agent_settings", &merged);
                let _ = respond_api_json(req, 200, &build_agent_settings_snapshot(&store, &suite), crypto_tok);
            }
            (Method::Get, "/api/agent/permission_rules") => {
                let rules = store
                    .get_kv("permission_rules")
                    .ok()
                    .flatten()
                    .unwrap_or_else(|| json!([]));
                let _ = respond_api_json(req, 200, &json!({"rules": rules}), crypto_tok);
            }
            (Method::Post, "/api/agent/permission_rules") => {
                let _ = store.set_kv("permission_rules", &body);
                let _ = respond_api_json(req, 200, &json!({"ok": true}), crypto_tok);
            }
            (Method::Get, "/api/agent/tools") => {
                let mut reg = ToolRegistry::with_builtin_tools();
                suite.register_tools(&mut reg);
                let specs = reg.all_specs();
                let flat: Vec<Value> = specs
                    .iter()
                    .map(|s| {
                        json!({
                            "name": s.schema.name,
                            "description": s.schema.description,
                            "parameters": s.schema.parameters,
                            "tier": s.tier,
                            "domain": s.domain,
                            "read_only": s.read_only
                        })
                    })
                    .collect();
                let _ = respond_api_json(req, 200, &json!({"count": flat.len(), "tools": flat}), crypto_tok);
            }
            (Method::Get, "/api/assistant/memory-default") => {
                let enabled = store
                    .get_kv("assistant_memory_default")
                    .ok()
                    .flatten()
                    .and_then(|v| v.get("enabled").and_then(|b| b.as_bool()).or_else(|| v.as_bool()))
                    .unwrap_or(true);
                let _ = respond_api_json(
                    req,
                    200,
                    &json!({"enabled": enabled, "hostAllowed": true}),
                    crypto_tok,
                );
            }
            (Method::Put, "/api/assistant/memory-default") => {
                let enabled = body
                    .get("enabled")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true);
                let saved = json!({"enabled": enabled, "hostAllowed": true});
                let _ = store.set_kv("assistant_memory_default", &saved);
                let _ = respond_api_json(req, 200, &saved, crypto_tok);
            }
            (Method::Get, "/api/assistant/retrieval-default") => {
                let val = store
                    .get_kv("assistant_retrieval_default")
                    .ok()
                    .flatten()
                    .unwrap_or_else(|| json!({"enabled": true}));
                let _ = respond_api_json(req, 200, &val, crypto_tok);
            }
            (Method::Put, "/api/assistant/retrieval-default") => {
                let _ = store.set_kv("assistant_retrieval_default", &body);
                let _ = respond_api_json(req, 200, &body, crypto_tok);
            }

            // --- Business Modules (Skills, Memory, Goals, Rooms, Experts, Automations, DataSources, MCP) ---
            (Method::Get, "/api/skills") | (Method::Get, "/api/automation-skills") => {
                let mut seen_ids: HashSet<String> = HashSet::new();
                let skills: Vec<Value> = nola_modules::skills::discover_skills(
                    &suite.workspace_root,
                    suite.user_home.as_deref(),
                )
                .into_iter()
                .filter_map(|s| {
                    let trimmed = s.name.trim().to_string();
                    if trimmed.is_empty() || !seen_ids.insert(trimmed) {
                        return None;
                    }
                    Some(project_skill_descriptor(&s, &store))
                })
                .collect();
                let _ = respond_api_json(req, 200, &json!({"skills": skills}), crypto_tok);
            }
            (Method::Get, "/api/memory") => {
                let q = parse_query_str(query, "q").unwrap_or_default();
                let raw_items = store.search_memory(&q, None, 2000).unwrap_or_default();
                let (records, items) = filter_superseded_memories(&store, raw_items);
                let _ = respond_api_json(
                    req,
                    200,
                    &json!({
                        "type": "memory_records",
                        "limit": 2000,
                        "records": records,
                        "items": items
                    }),
                    crypto_tok,
                );
            }
            (Method::Post, "/api/memory") => {
                let ts = now_ms();
                let op = body.get("operation").and_then(|v| v.as_str());
                let prev_id = body
                    .get("id")
                    .or_else(|| body.pointer("/draft/id"))
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(ToString::to_string);
                if op == Some("promote") {
                    let Some(target_id) = prev_id else {
                        let _ = respond_api_json(
                            req,
                            400,
                            &json!({"error": "missing_memory_id", "message": "promote requires 'id'"}),
                            crypto_tok,
                        );
                        return;
                    };
                    let existing = store
                        .search_memory("", None, 2000)
                        .unwrap_or_default()
                        .into_iter()
                        .find(|m| m.id == target_id);
                    let Some(mut item) = existing else {
                        let _ = respond_api_json(
                            req,
                            404,
                            &json!({"error": "memory_not_found", "id": target_id}),
                            crypto_tok,
                        );
                        return;
                    };
                    let existing_kind = item
                        .namespace
                        .split_once(':')
                        .map(|(_, k)| k.to_string())
                        .filter(|k| !k.is_empty())
                        .unwrap_or_else(|| {
                            if item.namespace.is_empty() {
                                "preference".to_string()
                            } else {
                                item.namespace.clone()
                            }
                        });
                    let target_scope = body
                        .get("scope")
                        .or_else(|| body.pointer("/draft/scope"))
                        .and_then(|v| v.as_str())
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .unwrap_or("global");
                    let target_kind = body
                        .get("kind")
                        .or_else(|| body.pointer("/draft/kind"))
                        .and_then(|v| v.as_str())
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .unwrap_or(&existing_kind);
                    item.namespace = format!("{target_scope}:{target_kind}");
                    item.updated_at_ms = ts;
                    match store.upsert_memory(&item) {
                        Ok(()) => {
                            let record = memory_item_to_record_with_store(&store, &item);
                            let _ = respond_api_json(
                                req,
                                200,
                                &json!({
                                    "type": "memory_promoted",
                                    "ok": true,
                                    "record": record,
                                    "item": item
                                }),
                                crypto_tok,
                            );
                        }
                        Err(e) => {
                            let _ = respond_api_json(
                                req,
                                500,
                                &json!({"error": e.to_string()}),
                                crypto_tok,
                            );
                        }
                    }
                    return;
                }
                let src = body.get("draft").unwrap_or(&body);
                let scope = src.get("scope").and_then(|v| v.as_str()).unwrap_or("global");
                let kind = src.get("kind").and_then(|v| v.as_str()).unwrap_or("preference");
                let namespace = src
                    .get("namespace")
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string)
                    .unwrap_or_else(|| format!("{scope}:{kind}"));
                let tags = src
                    .get("tags")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|x| x.as_str().map(ToString::to_string))
                            .collect()
                    })
                    .unwrap_or_default();
                let new_rec_id = if op == Some("revise") {
                    new_id()
                } else {
                    prev_id.clone().unwrap_or_else(new_id)
                };
                let mut supersedes: Vec<String> = src
                    .get("supersedes")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|x| {
                                x.as_str()
                                    .map(str::trim)
                                    .filter(|s| !s.is_empty())
                                    .map(ToString::to_string)
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                if let (Some("revise"), Some(ref pid)) = (op, &prev_id) {
                    if !supersedes.iter().any(|s| s == pid) {
                        supersedes.push(pid.clone());
                    }
                }
                let confidence = src
                    .get("confidence")
                    .and_then(|v| v.as_f64())
                    .filter(|v| v.is_finite())
                    .unwrap_or(1.0);
                let importance = src
                    .get("importance")
                    .and_then(|v| v.as_f64())
                    .filter(|v| v.is_finite())
                    .unwrap_or(0.5);
                let sensitivity = src
                    .get("sensitivity")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(|s| json!(s))
                    .unwrap_or(Value::Null);
                let expires_at = src
                    .get("expires_at")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(|s| json!(s))
                    .unwrap_or(Value::Null);
                let source = src
                    .get("source")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .unwrap_or("user");
                let item = MemoryItem {
                    id: new_rec_id.clone(),
                    namespace,
                    title: src
                        .get("summary")
                        .or_else(|| src.get("title"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .trim()
                        .to_string(),
                    content: src
                        .get("content")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .trim()
                        .to_string(),
                    tags,
                    created_at_ms: ts,
                    updated_at_ms: ts,
                };
                match store.upsert_memory(&item) {
                    Ok(()) => {
                        let meta = json!({
                            "supersedes": supersedes,
                            "confidence": confidence,
                            "importance": importance,
                            "sensitivity": sensitivity,
                            "expires_at": expires_at,
                            "source": source
                        });
                        let _ = store.set_kv(&memory_meta_key(&new_rec_id), &meta);
                        let record = memory_item_to_record_with_store(&store, &item);
                        if let (Some("revise"), Some(ref pid)) = (op, &prev_id) {
                            let _ = respond_api_json(
                                req,
                                200,
                                &json!({
                                    "type": "memory_revised",
                                    "previous_id": pid,
                                    "record": record,
                                    "ok": true,
                                    "item": item
                                }),
                                crypto_tok,
                            );
                        } else {
                            let _ = respond_api_json(
                                req,
                                200,
                                &json!({
                                    "type": "memory_created",
                                    "record": record,
                                    "ok": true,
                                    "item": item
                                }),
                                crypto_tok,
                            );
                        }
                    }
                    Err(e) => {
                        let _ = respond_api_json(req, 500, &json!({"error": e.to_string()}), crypto_tok);
                    }
                }
            }
            (Method::Delete, p) if p.starts_with("/api/memory/") => {
                let id = percent_decode_path(p.trim_start_matches("/api/memory/"));
                let _ = store.set_kv(&memory_meta_key(&id), &Value::Null);
                match store.delete_memory(&id) {
                    Ok(deleted) => {
                        let _ = respond_api_json(
                            req,
                            200,
                            &json!({"type": "memory_deleted", "ok": true, "deleted": deleted, "id": id}),
                            crypto_tok,
                        );
                    }
                    Err(e) => {
                        let _ = respond_api_json(req, 500, &json!({"error": e.to_string()}), crypto_tok);
                    }
                }
            }

            // --- MCP Server Full CRUD ---
            (Method::Get, "/api/mcp") => {
                let servers = store.list_mcp_servers().unwrap_or_default();
                let _ = respond_api_json(req, 200, &json!({"servers": servers}), crypto_tok);
            }
            (Method::Post, "/api/mcp") => {
                let cfg = parse_mcp_server_config(&body, None);
                match store.upsert_mcp_server(&cfg) {
                    Ok(()) => {
                        mcp_manager.evict(&cfg.id);
                        let _ = respond_api_json(req, 200, &json!({"ok": true, "server": cfg}), crypto_tok);
                    }
                    Err(e) => {
                        let _ = respond_api_json(req, 500, &json!({"error": e.to_string()}), crypto_tok);
                    }
                }
            }
            (Method::Put, p)
                if p.starts_with("/api/mcp/") && !p.trim_start_matches("/api/mcp/").contains('/') =>
            {
                let id = percent_decode_path(p.trim_start_matches("/api/mcp/"));
                let cfg = parse_mcp_server_config(&body, Some(&id));
                match store.upsert_mcp_server(&cfg) {
                    Ok(()) => {
                        mcp_manager.evict(&cfg.id);
                        let _ = respond_api_json(req, 200, &json!({"ok": true, "server": cfg}), crypto_tok);
                    }
                    Err(e) => {
                        let _ = respond_api_json(req, 500, &json!({"error": e.to_string()}), crypto_tok);
                    }
                }
            }
            (Method::Delete, p)
                if p.starts_with("/api/mcp/") && !p.trim_start_matches("/api/mcp/").contains('/') =>
            {
                let id = percent_decode_path(p.trim_start_matches("/api/mcp/"));
                mcp_manager.disconnect(&id);
                match store.delete_mcp_server(&id) {
                    Ok(deleted) => {
                        let _ = respond_api_json(
                            req,
                            200,
                            &json!({"ok": true, "deleted": deleted, "id": id}),
                            crypto_tok,
                        );
                    }
                    Err(e) => {
                        let _ = respond_api_json(req, 500, &json!({"error": e.to_string()}), crypto_tok);
                    }
                }
            }

            (Method::Get, "/api/goals") => {
                let items: Vec<Value> = suite.list_records("goal").unwrap_or_default();
                let _ = respond_api_json(req, 200, &json!({"goals": items, "items": items}), crypto_tok);
            }
            (Method::Post, "/api/goals") => {
                let mut val = body;
                let id = val
                    .get("id")
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string)
                    .unwrap_or_else(new_id);
                val["id"] = json!(id);
                match suite.upsert_record("goal", &id, &val) {
                    Ok(()) => {
                        let _ = respond_api_json(req, 200, &json!({"ok": true, "id": id, "goal": val}), crypto_tok);
                    }
                    Err(e) => {
                        let _ = respond_api_json(req, 500, &json!({"error": e.to_string()}), crypto_tok);
                    }
                }
            }
            (Method::Get, "/api/sops") => {
                let items: Vec<Value> = suite.list_records("sop").unwrap_or_default();
                let _ = respond_api_json(req, 200, &json!({"sops": items, "items": items}), crypto_tok);
            }
            (Method::Get, "/api/automations") | (Method::Get, "/api/scheduled-actions") => {
                let raw_items: Vec<Value> = suite.list_records("automation").unwrap_or_default();
                let mut seen_ids: HashSet<String> = HashSet::new();
                let actions: Vec<Value> = raw_items
                    .iter()
                    .map(normalize_automation_record)
                    .filter(|rec| {
                        rec.get("id")
                            .and_then(|v| v.as_str())
                            .is_some_and(|id| !id.is_empty() && seen_ids.insert(id.to_string()))
                    })
                    .collect();
                let _ = respond_api_json(
                    req,
                    200,
                    &json!({
                        "actions": actions,
                        "automations": actions,
                        "items": actions,
                        "executionCapabilities": {
                            "manualRun": true,
                            "scheduledDispatch": true,
                            "promptAutomations": true
                        }
                    }),
                    crypto_tok,
                );
            }
            (Method::Post, "/api/scheduled-actions") => {
                let norm = normalize_automation_record(&body);
                let id = norm["id"].as_str().unwrap_or("").to_string();
                match suite.upsert_record("automation", &id, &norm) {
                    Ok(()) => {
                        let _ = respond_api_json(
                            req,
                            200,
                            &json!({"success": true, "ok": true, "action": norm}),
                            crypto_tok,
                        );
                    }
                    Err(e) => {
                        let _ = respond_api_json(req, 500, &json!({"error": e.to_string()}), crypto_tok);
                    }
                }
            }
            (Method::Patch, p)
                if p.starts_with("/api/scheduled-actions/")
                    && !p.trim_start_matches("/api/scheduled-actions/").contains('/') =>
            {
                let id = percent_decode_path(p.trim_start_matches("/api/scheduled-actions/"));
                let existing_opt = suite
                    .list_records::<Value>("automation")
                    .unwrap_or_default()
                    .into_iter()
                    .find(|v| v.get("id").and_then(|s| s.as_str()) == Some(id.as_str()));
                let mut existing = existing_opt.unwrap_or_else(|| json!({"id": id, "revision": 1}));
                let current_rev = existing
                    .get("revision")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(1);
                if let Some(expected_rev) = body
                    .get("expectedRevision")
                    .or_else(|| body.get("expected_revision"))
                    .and_then(|v| v.as_i64())
                {
                    if expected_rev != current_rev {
                        let _ = respond_api_json(
                            req,
                            409,
                            &json!({
                                "error": "revision_conflict",
                                "code": "automation_revision_conflict",
                                "expectedRevision": expected_rev,
                                "actualRevision": current_rev
                            }),
                            crypto_tok,
                        );
                        return;
                    }
                }
                let patch_src = body
                    .get("definition")
                    .filter(|v| v.is_object())
                    .unwrap_or(&body);
                let prev_format = canonical_automation_format(
                    existing.get("format").and_then(|v| v.as_str()),
                );
                let next_format = match patch_src.get("format").and_then(|v| v.as_str()) {
                    Some(raw_fmt) => canonical_automation_format(Some(raw_fmt)),
                    None => prev_format.clone(),
                };
                let prev_freq = existing
                    .get("frequency")
                    .and_then(|v| v.as_str())
                    .unwrap_or("daily")
                    .to_string();
                let next_freq = patch_src
                    .get("frequency")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&prev_freq)
                    .to_string();
                let script_updated = patch_src.get("script").and_then(|v| v.as_str()).is_some()
                    || (next_format == "script" && prev_format != "script");
                if let (Some(dst), Some(src)) = (existing.as_object_mut(), patch_src.as_object()) {
                    // Clear incompatible fields when `format` or `frequency` changes!
                    if next_format != prev_format {
                        dst.remove("url");
                        dst.remove("path");
                        dst.remove("query");
                        dst.remove("script");
                        dst.remove("cwd");
                        dst.insert("scriptApproved".into(), Value::Null);
                        dst.insert("scriptCwd".into(), Value::Null);
                    }
                    if next_freq != "weekly" {
                        dst.insert("weekday".into(), Value::Null);
                    }
                    for (k, v) in src {
                        if k != "expectedRevision" && k != "expected_revision" && k != "revision" {
                            dst.insert(k.clone(), v.clone());
                        }
                    }
                    dst.insert("format".into(), json!(next_format));
                    if next_format == "agent" {
                        dst.insert("url".into(), json!(""));
                        dst.insert("scriptApproved".into(), Value::Null);
                        dst.insert("scriptCwd".into(), Value::Null);
                        dst.remove("script");
                        dst.remove("cwd");
                    } else if next_format == "script" {
                        dst.insert("url".into(), json!(""));
                        if script_updated && !src.contains_key("scriptApproved") {
                            dst.insert("scriptApproved".into(), json!(false));
                        }
                    } else {
                        dst.insert("scriptApproved".into(), Value::Null);
                        dst.insert("scriptCwd".into(), Value::Null);
                        dst.remove("script");
                        dst.remove("cwd");
                    }
                    if next_freq != "weekly" {
                        dst.insert("weekday".into(), Value::Null);
                    }
                    dst.insert("id".into(), json!(id));
                    dst.insert("revision".into(), json!(current_rev + 1));
                    dst.insert("updatedAt".into(), json!(Utc::now().to_rfc3339()));
                }
                let norm = normalize_automation_record(&existing);
                match suite.upsert_record("automation", &id, &norm) {
                    Ok(()) => {
                        let _ = respond_api_json(
                            req,
                            200,
                            &json!({"success": true, "ok": true, "action": norm}),
                            crypto_tok,
                        );
                    }
                    Err(e) => {
                        let _ = respond_api_json(req, 500, &json!({"error": e.to_string()}), crypto_tok);
                    }
                }
            }
            (Method::Delete, p)
                if p.starts_with("/api/scheduled-actions/")
                    && !p.trim_start_matches("/api/scheduled-actions/").contains('/') =>
            {
                let id = percent_decode_path(p.trim_start_matches("/api/scheduled-actions/"));
                let deleted = suite.delete_record("automation", &id).unwrap_or(false);
                let _ = respond_api_json(
                    req,
                    200,
                    &json!({"success": true, "ok": true, "deleted": deleted, "id": id}),
                    crypto_tok,
                );
            }
            (Method::Post, p)
                if p.starts_with("/api/scheduled-actions/") && p.ends_with("/approve-script") =>
            {
                let id = percent_decode_path(
                    p.trim_start_matches("/api/scheduled-actions/")
                        .trim_end_matches("/approve-script"),
                );
                let confirmed = body
                    .get("confirmSafety")
                    .or_else(|| body.get("confirm_safety"))
                    .and_then(|v| v.as_bool())
                    == Some(true);
                if !confirmed {
                    let _ = respond_api_json(
                        req,
                        400,
                        &json!({"error": "confirm_safety_required", "message": "confirmSafety must be true"}),
                        crypto_tok,
                    );
                    return;
                }
                let Some(mut existing) = suite
                    .list_records::<Value>("automation")
                    .unwrap_or_default()
                    .into_iter()
                    .find(|v| v.get("id").and_then(|s| s.as_str()) == Some(id.as_str()))
                else {
                    let _ = respond_api_json(
                        req,
                        404,
                        &json!({"error": "action_not_found"}),
                        crypto_tok,
                    );
                    return;
                };
                let current_rev = existing
                    .get("revision")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(1);
                if let Some(expected_rev) = body
                    .get("expectedRevision")
                    .or_else(|| body.get("expected_revision"))
                    .and_then(|v| v.as_i64())
                {
                    if expected_rev != current_rev {
                        let _ = respond_api_json(
                            req,
                            409,
                            &json!({
                                "error": "revision_conflict",
                                "code": "automation_revision_conflict",
                                "expectedRevision": expected_rev,
                                "actualRevision": current_rev
                            }),
                            crypto_tok,
                        );
                        return;
                    }
                }
                if let Some(dst) = existing.as_object_mut() {
                    dst.insert("scriptApproved".into(), json!(true));
                    dst.insert("revision".into(), json!(current_rev + 1));
                    dst.insert("updatedAt".into(), json!(Utc::now().to_rfc3339()));
                }
                let norm = normalize_automation_record(&existing);
                match suite.upsert_record("automation", &id, &norm) {
                    Ok(()) => {
                        let _ = respond_api_json(
                            req,
                            200,
                            &json!({"success": true, "ok": true, "action": norm}),
                            crypto_tok,
                        );
                    }
                    Err(e) => {
                        let _ = respond_api_json(req, 500, &json!({"error": e.to_string()}), crypto_tok);
                    }
                }
            }
            (Method::Post, p)
                if p.starts_with("/api/scheduled-actions/") && p.ends_with("/run-now") =>
            {
                let action_id = percent_decode_path(
                    p.trim_start_matches("/api/scheduled-actions/")
                        .trim_end_matches("/run-now"),
                );
                let crypto_tok_owned = crypto_tok.map(ToString::to_string);
                let Some(mut action) = suite
                    .list_records::<Value>("automation")
                    .unwrap_or_default()
                    .into_iter()
                    .find(|v| v.get("id").and_then(|s| s.as_str()) == Some(action_id.as_str()))
                else {
                    let _ = respond_api_json(
                        req,
                        404,
                        &json!({"error": "action_not_found"}),
                        crypto_tok_owned.as_deref(),
                    );
                    return;
                };
                let rev = action
                    .get("revision")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(1)
                    .max(1);
                if let Some(expected_rev) = body
                    .get("expectedRevision")
                    .or_else(|| body.get("expected_revision"))
                    .and_then(|v| v.as_i64())
                {
                    if expected_rev != rev {
                        let _ = respond_api_json(
                            req,
                            409,
                            &json!({
                                "error": "revision_conflict",
                                "code": "automation_revision_conflict",
                                "expectedRevision": expected_rev,
                                "actualRevision": rev
                            }),
                            crypto_tok_owned.as_deref(),
                        );
                        return;
                    }
                }
                let format = action
                    .get("format")
                    .and_then(|v| v.as_str())
                    .unwrap_or("agent")
                    .to_string();
                if format == "script"
                    && action.get("scriptApproved").and_then(|v| v.as_bool()) != Some(true)
                {
                    let _ = respond_api_json(
                        req,
                        403,
                        &json!({"error": "script_not_approved", "message": "script automation must be approved before execution"}),
                        crypto_tok_owned.as_deref(),
                    );
                    return;
                }
                let name = action
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&action_id)
                    .to_string();
                let exec_spec = action.pointer("/executionSpec/spec");
                let ws_root_path = exec_spec
                    .and_then(|s| s.pointer("/execution_scope/root"))
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(PathBuf::from)
                    .filter(|p| p.is_dir())
                    .unwrap_or_else(|| suite.workspace_root.clone());
                let spec_perm = exec_spec
                    .and_then(|s| s.get("permission_profile"))
                    .and_then(|v| v.as_str())
                    .and_then(|s| PermissionProfile::parse_strict(s).ok())
                    .unwrap_or(PermissionProfile::Cautious);
                let prompt = exec_spec
                    .and_then(|s| s.get("prompt"))
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
                    .or_else(|| {
                        action
                            .get("userPrompt")
                            .or_else(|| action.get("prompt"))
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.trim().is_empty())
                    })
                    .unwrap_or("Run scheduled automation check")
                    .to_string();
                let schedule = nola_modules::scheduler::parse_automation_value(&action, &ws_root_path)
                    .map(|r| r.schedule)
                    .unwrap_or_else(|| "0 9 * * *".into());
                let run_id = new_id();
                let occ_id = body
                    .get("occurrenceKey")
                    .or_else(|| body.get("occurrence_key"))
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(ToString::to_string)
                    .unwrap_or_else(|| format!("occ-{run_id}"));
                let conv_id = format!("auto-{run_id}");
                let now_ts = now_ms();
                let now_dt = Utc::now();
                let now_iso = now_dt.to_rfc3339();
                // Upsert conversation Session BEFORE running so `/chat?conversation=auto-...` never 404s!
                let _ = store.upsert_session(&Session {
                    id: conv_id.clone(),
                    title: format!("Automation: {name}"),
                    workspace_root: ws_root_path.display().to_string(),
                    model_connection: None,
                    model_id: None,
                    created_at_ms: now_ts,
                    updated_at_ms: now_ts,
                });
                let _ = store.append_event(
                    &conv_id,
                    "chat_user",
                    json!({"content": &prompt, "attachments": []}),
                    50_000,
                );

                let exec_result: Result<(String, &'static str), String> = if format == "script" {
                    let parsed_script = action
                        .get("userPrompt")
                        .and_then(|v| v.as_str())
                        .and_then(|s| serde_json::from_str::<Value>(s).ok());
                    let script_body = parsed_script
                        .as_ref()
                        .and_then(|v| v.get("script").and_then(|s| s.as_str()))
                        .or_else(|| action.get("script").and_then(|v| v.as_str()))
                        .unwrap_or("")
                        .to_string();
                    if script_body.trim().is_empty() {
                        Err("script automation has empty script body".into())
                    } else {
                        let script_cwd = parsed_script
                            .as_ref()
                            .and_then(|v| v.get("cwd").and_then(|s| s.as_str()))
                            .or_else(|| action.get("scriptCwd").and_then(|v| v.as_str()))
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .map(PathBuf::from)
                            .filter(|p| p.is_dir())
                            .unwrap_or_else(|| ws_root_path.clone());
                        run_automation_bash_script(&script_body, &script_cwd)
                            .map(|out| (out, "action_completed"))
                    }
                } else {
                    let cfg = load_active_config(&store);
                    resolve_model_route(&cfg).and_then(|route| {
                        let caller: Arc<dyn ModelCaller> =
                            Arc::new(HttpModelCaller::new(300).map_err(|e| e.to_string())?);
                        let trusted = spec_perm == PermissionProfile::AllowAll;
                        let policy = EffectivePolicy::load(
                            &ws_root_path,
                            suite.user_home.as_deref(),
                            trusted,
                            Some(spec_perm),
                            true,
                        );
                        let hooks = HookRunner::load(
                            &ws_root_path,
                            suite.user_home.as_deref(),
                            trusted,
                        );
                        let mut reg = ToolRegistry::with_builtin_tools();
                        suite.register_tools(&mut reg);
                        let engine = AgentEngine::new(store.clone(), Arc::new(reg), caller);
                        let mut ctx = ToolContext::new(&ws_root_path);
                        ctx.permission_profile = policy.profile;
                        let run_cfg = AgentRunConfig {
                            session_id: Some(conv_id.clone()),
                            resume: false,
                            route,
                            frozen_system_blocks: build_frozen_system_blocks(&ws_root_path, &cfg),
                            host_capabilities: HostCapabilities {
                                interactive_user_input: false,
                                network_allowed: true,
                            },
                            policy,
                            hooks,
                            approval: None,
                            max_steps: 12,
                            force_tool_set: ForceToolSet::Auto,
                            stream: false,
                            json_schema: None,
                            cancel: Arc::new(AtomicBool::new(false)),
                            auto_recall_memory: true,
                            inbox: None,
                        };
                        let res = engine
                            .run(&ctx, &prompt, run_cfg, |_| {})
                            .map_err(|e| e.to_string())?;
                        Ok((res.final_text, "answered"))
                    })
                };

                match exec_result {
                    Ok((summary, terminal_kind)) => {
                        let _ = store.append_event(
                            &conv_id,
                            "chat_assistant",
                            json!({"content": &summary, "reasoning": "", "toolCalls": [], "uiSurfaces": []}),
                            50_000,
                        );
                        let _ = nola_modules::scheduler::record_automation_run_stats(
                            &store,
                            &action_id,
                            &schedule,
                            now_dt,
                            true,
                            None,
                        );
                        let result_id = new_id();
                        let rec = json!({
                            "resultId": result_id,
                            "sourceId": action_id,
                            "sourceName": name,
                            "sourceUrl": action.get("url").and_then(|v| v.as_str()).unwrap_or(""),
                            "sourceFormat": format,
                            "conversationId": conv_id,
                            "runId": run_id,
                            "status": terminal_kind,
                            "scheduledAt": now_iso,
                            "createdAt": now_iso,
                            "fetchedAt": now_iso,
                            "previewText": summary.chars().take(240).collect::<String>(),
                            "llmSummaryText": summary,
                            "ackCount": 0,
                            "expired": false,
                            "events": []
                        });
                        let _ = suite.upsert_record("automation_result", &result_id, &rec);
                        // Write `latestResult` back onto the automation record!
                        action["latestResult"] = json!({
                            "resultId": result_id,
                            "status": terminal_kind,
                            "createdAt": now_iso
                        });
                        let norm_action = normalize_automation_record(&action);
                        let _ = suite.upsert_record("automation", &action_id, &norm_action);
                        let binding = json!({
                            "occurrence_id": occ_id,
                            "definition_revision": rev,
                            "conversation_id": conv_id,
                            "run_id": run_id
                        });
                        let _ = respond_api_json(
                            req,
                            200,
                            &json!({
                                "success": true,
                                "ok": true,
                                "occurrenceId": occ_id,
                                "execution": { "binding": binding },
                                "receipt": {
                                    "binding": binding,
                                    "terminal": {
                                        "result": {
                                            "value": { "kind": terminal_kind }
                                        }
                                    }
                                },
                                "result": rec
                            }),
                            crypto_tok_owned.as_deref(),
                        );
                    }
                    Err(e) => {
                        let _ = store.append_event(
                            &conv_id,
                            "chat_assistant",
                            json!({"content": format!("Automation failed: {e}"), "reasoning": "", "toolCalls": [], "uiSurfaces": []}),
                            50_000,
                        );
                        let _ = nola_modules::scheduler::record_automation_run_stats(
                            &store,
                            &action_id,
                            &schedule,
                            now_dt,
                            false,
                            Some(&e),
                        );
                        let result_id = new_id();
                        action["latestResult"] = json!({
                            "resultId": result_id,
                            "status": "failed",
                            "createdAt": now_iso
                        });
                        let norm_action = normalize_automation_record(&action);
                        let _ = suite.upsert_record("automation", &action_id, &norm_action);
                        let _ = respond_api_json(
                            req,
                            400,
                            &json!({"error": e}),
                            crypto_tok_owned.as_deref(),
                        );
                    }
                }
            }

            // --- Data Sources (Real Usage + Real Connection Probing) ---
            (Method::Get, "/api/data-sources/usage") => {
                let sources: Vec<Value> = suite.list_records("data_source").unwrap_or_default();
                let title_by_id: BTreeMap<String, String> = sources
                    .iter()
                    .filter_map(|s| {
                        let sid = s.get("id").and_then(|v| v.as_str())?.to_string();
                        let title = s
                            .get("title")
                            .or_else(|| s.get("name"))
                            .and_then(|v| v.as_str())
                            .filter(|t| !t.trim().is_empty())
                            .unwrap_or(&sid)
                            .to_string();
                        Some((sid, title))
                    })
                    .collect();
                let usage: Vec<Value> = list_data_source_usage(&store)
                    .into_iter()
                    .map(|r| {
                        let rec_id = r
                            .get("id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let source_id = r
                            .get("source_id")
                            .or_else(|| r.get("sourceId"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let summary = r
                            .get("summary")
                            .or_else(|| r.get("purpose"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let conv_id = r
                            .get("conversation_id")
                            .or_else(|| r.get("conversationId"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let used_at = r
                            .get("used_at_ms")
                            .and_then(|v| v.as_i64())
                            .map(ms_to_rfc3339)
                            .or_else(|| {
                                r.get("usedAt")
                                    .and_then(|v| v.as_str())
                                    .map(ToString::to_string)
                            })
                            .unwrap_or_else(|| Utc::now().to_rfc3339());
                        let title = title_by_id
                            .get(&source_id)
                            .cloned()
                            .unwrap_or_else(|| source_id.clone());
                        json!({
                            "id": rec_id,
                            "sourceId": source_id,
                            "sourceTitle": title,
                            "consumer": "agent",
                            "purpose": summary,
                            "conversationId": conv_id,
                            "usedAt": used_at,
                            "disclosedToCloud": Value::Null
                        })
                    })
                    .collect();
                let _ = respond_api_json(req, 200, &json!({"usageRecords": usage}), crypto_tok);
            }
            (Method::Get, "/api/data-sources") => {
                let raw_items: Vec<Value> = suite.list_records("data_source").unwrap_or_default();
                let mut seen_ids: HashSet<String> = HashSet::new();
                let items: Vec<Value> = raw_items
                    .iter()
                    .map(normalize_data_source_record)
                    .filter(|s| {
                        s.get("id")
                            .and_then(|v| v.as_str())
                            .is_some_and(|id| !id.is_empty() && seen_ids.insert(id.to_string()))
                    })
                    .collect();
                let wrapped: Vec<Value> = items.iter().map(|s| json!({"source": s})).collect();
                let _ = respond_api_json(
                    req,
                    200,
                    &json!({"sources": wrapped, "data_sources": items, "items": items}),
                    crypto_tok,
                );
            }
            (Method::Post, "/api/data-sources") => {
                let mut ds_body = normalize_data_source_record(&body);
                if is_sensitive_data_source_record(&suite, &ds_body) {
                    let _ = respond_api_json(
                        req,
                        400,
                        &json!({
                            "ok": false,
                            "error": "sensitive_local_path_forbidden",
                            "message": "access to sensitive local path is forbidden"
                        }),
                        crypto_tok,
                    );
                    return;
                }
                let id = ds_body["id"].as_str().unwrap_or("").to_string();
                let (ok, probe) = probe_and_attach_data_source_validation(&suite, &id, &mut ds_body);
                let ds_body = normalize_data_source_record(&ds_body);
                match suite.upsert_record("data_source", &id, &ds_body) {
                    Ok(()) => {
                        let _ = respond_api_json(
                            req,
                            200,
                            &json!({"source": ds_body, "tested": true, "ok": ok, "probe": probe}),
                            crypto_tok,
                        );
                    }
                    Err(e) => {
                        let _ = respond_api_json(req, 500, &json!({"error": e.to_string()}), crypto_tok);
                    }
                }
            }
            (Method::Patch, p)
                if p.starts_with("/api/data-sources/")
                    && !p.trim_start_matches("/api/data-sources/").contains('/') =>
            {
                let id = percent_decode_path(p.trim_start_matches("/api/data-sources/"));
                let mut ds_body = suite
                    .list_records::<Value>("data_source")
                    .unwrap_or_default()
                    .into_iter()
                    .find(|v| v.get("id").and_then(|s| s.as_str()) == Some(id.as_str()))
                    .unwrap_or_else(|| json!({"id": id}));
                if let (Some(dst), Some(src)) = (ds_body.as_object_mut(), body.as_object()) {
                    for (k, v) in src {
                        dst.insert(k.clone(), v.clone());
                    }
                }
                ds_body["id"] = json!(id);
                let mut ds_body = normalize_data_source_record(&ds_body);
                if is_sensitive_data_source_record(&suite, &ds_body) {
                    let _ = respond_api_json(
                        req,
                        400,
                        &json!({
                            "ok": false,
                            "error": "sensitive_local_path_forbidden",
                            "message": "access to sensitive local path is forbidden"
                        }),
                        crypto_tok,
                    );
                    return;
                }
                let (ok, probe) = probe_and_attach_data_source_validation(&suite, &id, &mut ds_body);
                let ds_body = normalize_data_source_record(&ds_body);
                match suite.upsert_record("data_source", &id, &ds_body) {
                    Ok(()) => {
                        let _ = respond_api_json(
                            req,
                            200,
                            &json!({"source": ds_body, "tested": true, "ok": ok, "probe": probe}),
                            crypto_tok,
                        );
                    }
                    Err(e) => {
                        let _ = respond_api_json(req, 500, &json!({"error": e.to_string()}), crypto_tok);
                    }
                }
            }
            (Method::Delete, p)
                if p.starts_with("/api/data-sources/")
                    && !p.trim_start_matches("/api/data-sources/").contains('/') =>
            {
                let id = percent_decode_path(p.trim_start_matches("/api/data-sources/"));
                let deleted = suite.delete_record("data_source", &id).unwrap_or(false);
                let _ = respond_api_json(req, 200, &json!({"deleted": deleted, "id": id}), crypto_tok);
            }
            (Method::Post, p) if p.starts_with("/api/data-sources/") && p.ends_with("/index") => {
                let id = percent_decode_path(
                    p.trim_start_matches("/api/data-sources/")
                        .trim_end_matches("/index"),
                );
                match index_data_source(&store, &id) {
                    Ok(chunks) => {
                        let _ = respond_api_json(
                            req,
                            200,
                            &json!({
                                "ok": true,
                                "sourceId": id,
                                "indexedChunks": chunks
                            }),
                            crypto_tok,
                        );
                    }
                    Err(e) => {
                        let _ = respond_api_json(
                            req,
                            400,
                            &json!({"ok": false, "sourceId": id, "error": e}),
                            crypto_tok,
                        );
                    }
                }
            }
            (Method::Post, p) if p.starts_with("/api/data-sources/") && p.ends_with("/test") => {
                let id = percent_decode_path(
                    p.trim_start_matches("/api/data-sources/")
                        .trim_end_matches("/test"),
                );
                let raw_existing = suite
                    .list_records::<Value>("data_source")
                    .unwrap_or_default()
                    .into_iter()
                    .find(|v| v.get("id").and_then(|s| s.as_str()) == Some(id.as_str()))
                    .unwrap_or_else(|| {
                        json!({"id": id, "name": id, "kind": "localFile", "location": "", "accessPolicy": "conversationOptIn"})
                    });
                let mut existing = normalize_data_source_record(&raw_existing);
                existing["id"] = json!(id);
                let (ok, probe) = probe_and_attach_data_source_validation(&suite, &id, &mut existing);
                let existing = normalize_data_source_record(&existing);
                let _ = suite.upsert_record("data_source", &id, &existing);
                let _ = respond_api_json(
                    req,
                    200,
                    &json!({"source": existing, "tested": true, "ok": ok, "probe": probe}),
                    crypto_tok,
                );
            }

            // --- Messaging Channels & Worker (Full Schema + Fail-Closed Inbound Webhook) ---
            (Method::Get, "/api/messaging-channels") => {
                let items: Vec<Value> = suite
                    .list_records::<Value>("messaging_channel")
                    .unwrap_or_default()
                    .into_iter()
                    .map(|c| normalize_messaging_channel(&store, &c))
                    .collect();
                let _ = respond_api_json(req, 200, &json!({"channels": items, "items": items}), crypto_tok);
            }
            (Method::Put, "/api/messaging-channels") => {
                if let Some(arr) = body.get("channels").and_then(|v| v.as_array()) {
                    for ch in arr {
                        let norm = persist_channel_secret_and_normalize(&store, ch);
                        let id = norm["connection_id"]
                            .as_str()
                            .filter(|s| !s.is_empty())
                            .map(ToString::to_string)
                            .unwrap_or_else(new_id);
                        let _ = suite.upsert_record("messaging_channel", &id, &norm);
                    }
                    let items: Vec<Value> = suite
                        .list_records::<Value>("messaging_channel")
                        .unwrap_or_default()
                        .into_iter()
                        .map(|c| normalize_messaging_channel(&store, &c))
                        .collect();
                    let _ = respond_api_json(req, 200, &json!({"channels": items, "ok": true}), crypto_tok);
                } else {
                    // Single channel draft object (matching `MessagingSettings.svelte` `channelReceipt`)
                    let norm = persist_channel_secret_and_normalize(&store, &body);
                    let id = norm["connection_id"]
                        .as_str()
                        .filter(|s| !s.is_empty())
                        .map(ToString::to_string)
                        .unwrap_or_else(new_id);
                    if let Err(e) = suite.upsert_record("messaging_channel", &id, &norm) {
                        let _ = respond_api_json(req, 500, &json!({"error": e.to_string()}), crypto_tok);
                    } else {
                        let _ = respond_api_json(req, 200, &norm, crypto_tok);
                    }
                }
            }
            (Method::Delete, p)
                if p.starts_with("/api/messaging-channels/")
                    && !p.trim_start_matches("/api/messaging-channels/").contains('/') =>
            {
                let id = percent_decode_path(p.trim_start_matches("/api/messaging-channels/"));
                if let Ok(vault) = Vault::open(resolve_vault_path_for_store(&store)) {
                    let _ = vault.delete_secret(&format!("msg_webhook_secret:{id}"));
                }
                let deleted = suite.delete_record("messaging_channel", &id).unwrap_or(false);
                let _ = respond_api_json(
                    req,
                    200,
                    &json!({"ok": true, "deleted": deleted, "id": id}),
                    crypto_tok,
                );
            }
            (Method::Post, p)
                if p.starts_with("/api/messaging-channels/") && p.ends_with("/test") =>
            {
                let id = percent_decode_path(
                    p.trim_start_matches("/api/messaging-channels/")
                        .trim_end_matches("/test"),
                );
                let Some(ch) = suite
                    .get_record::<Value>("messaging_channel", &id)
                    .ok()
                    .flatten()
                else {
                    let _ = respond_api_json(
                        req,
                        404,
                        &json!({"ok": false, "error": "channel_not_found", "id": id}),
                        crypto_tok,
                    );
                    return;
                };
                let norm = normalize_messaging_channel(&store, &ch);
                let has_secret = norm
                    .get("has_webhook_secret")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
                    || !resolve_channel_webhook_secret(&store, &ch).trim().is_empty();
                let has_ref = ["credential_ref", "webhook_secret_ref", "verify_token_ref", "account_id"]
                    .iter()
                    .any(|k| {
                        norm.get(*k)
                            .and_then(|v| v.as_str())
                            .is_some_and(|s| !s.trim().is_empty())
                    });
                let has_provider_cfg = norm
                    .get("provider_config")
                    .and_then(|v| v.as_object())
                    .is_some_and(|m| {
                        m.values().any(|val| {
                            val.as_str().is_some_and(|s| !s.trim().is_empty())
                        })
                    });
                if has_secret || has_ref || has_provider_cfg {
                    let _ = respond_api_json(
                        req,
                        200,
                        &json!({
                            "ok": true,
                            "status": "verified",
                            "channelId": id,
                            "testedAt": Utc::now().to_rfc3339()
                        }),
                        crypto_tok,
                    );
                } else {
                    let _ = respond_api_json(
                        req,
                        400,
                        &json!({
                            "ok": false,
                            "status": "invalid",
                            "error": "missing_channel_configuration",
                            "message": "Configure a webhook secret, credential, token, or endpoint URL before testing this messaging channel."
                        }),
                        crypto_tok,
                    );
                }
            }
            (Method::Post, p)
                if (p.starts_with("/api/messaging-channels/") && p.ends_with("/inbound"))
                    || p.starts_with("/api/messaging/webhooks/")
                    || p.starts_with("/v1/channels/webhook/")
                    || p.starts_with("/api/v1/channels/webhook/") =>
            {
                let (id, kind_override) = if p.starts_with("/api/messaging-channels/") {
                    (
                        percent_decode_path(
                            p.trim_start_matches("/api/messaging-channels/")
                                .trim_end_matches("/inbound"),
                        ),
                        None,
                    )
                } else if p.starts_with("/api/messaging/webhooks/") {
                    (
                        percent_decode_path(
                            p.trim_start_matches("/api/messaging/webhooks/")
                                .trim_end_matches('/'),
                        ),
                        None,
                    )
                } else {
                    let rest = p
                        .trim_start_matches("/api")
                        .trim_start_matches("/v1/channels/webhook/");
                    match rest.split_once('/') {
                        Some((k, cid)) => (
                            percent_decode_path(cid.trim_end_matches('/')),
                            Some(percent_decode_path(k)),
                        ),
                        None => (String::new(), None),
                    }
                };
                if id.is_empty() {
                    let _ = respond_json(req, 404, &json!({"error": "channel_not_found"}));
                    return;
                }
                let channel = suite
                    .get_record::<Value>("messaging_channel", &id)
                    .ok()
                    .flatten();
                let Some(ch) = channel else {
                    let _ = respond_json(req, 404, &json!({"error": "channel_not_found"}));
                    return;
                };
                if let Some(ref k_ov) = kind_override {
                    let stored_kind = ch.get("kind").and_then(|v| v.as_str()).unwrap_or("");
                    if !stored_kind.is_empty() && stored_kind != k_ov.as_str() {
                        let _ = respond_json(req, 404, &json!({"error": "channel_kind_mismatch"}));
                        return;
                    }
                }
                let expected_secret = resolve_channel_webhook_secret(&store, &ch);
                let provided = header_value(&req, "x-nola-webhook-token")
                    .or_else(|| header_value(&req, "x-webhook-secret"))
                    .or_else(|| extract_bearer_token(&req))
                    .unwrap_or("");
                if !verify_inbound_webhook_token(&expected_secret, provided) {
                    let _ = respond_json(req, 401, &json!({"error": "invalid_webhook_token"}));
                    return;
                }
                let kind = ch.get("kind").and_then(|v| v.as_str()).unwrap_or("webhook");
                match parse_inbound_webhook_payload(kind, &body) {
                    Ok(parsed) => {
                        let mut updated = normalize_messaging_channel(&store, &ch);
                        updated["last_received_at"] = json!(Utc::now().to_rfc3339());
                        let _ = suite.upsert_record("messaging_channel", &id, &updated);
                        let _ = store.append_event(
                            &format!("channel:{id}"),
                            "inbound_webhook",
                            parsed.clone(),
                            10_000,
                        );
                        // Spawn background AgentEngine turn so inbound messages trigger real replies.
                        let cfg = load_active_config(&store);
                        let turn_scheduled = if let Ok(route) = resolve_model_route(&cfg) {
                            let store_bg = store.clone();
                            let ws_bg = suite.workspace_root.clone();
                            let id_bg = id.clone();
                            let body_bg = body.clone();
                            let secret_bg = (!expected_secret.trim().is_empty())
                                .then(|| expected_secret.clone());
                            let _ = std::thread::Builder::new()
                                .name(format!("nola-webhook-{}", &id_bg[..id_bg.len().min(8)]))
                                .spawn(move || {
                                    let _ = handle_inbound_webhook_turn_with_secret(
                                        &store_bg,
                                        route,
                                        &ws_bg,
                                        &id_bg,
                                        &body_bg,
                                        secret_bg.as_deref(),
                                    );
                                });
                            true
                        } else {
                            false
                        };
                        let _ = respond_json(
                            req,
                            200,
                            &json!({"ok": true, "event": parsed, "turnScheduled": turn_scheduled}),
                        );
                    }
                    Err(e) => {
                        let _ = respond_json(req, 400, &json!({"error": e}));
                    }
                }
            }
            (Method::Get, "/api/messaging-channel-worker") => {
                let mut val = load_messaging_worker_snapshot(&store);
                let running = messaging_worker
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .is_some();
                if let Some(obj) = val.as_object_mut() {
                    obj.insert("running".into(), json!(running));
                }
                let _ = respond_api_json(req, 200, &val, crypto_tok);
            }
            (Method::Put, "/api/messaging-channel-worker") => {
                let mut current = load_messaging_worker_snapshot(&store);
                let has_token = if body
                    .get("clear_access_token")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
                {
                    if let Ok(vault) = Vault::open(resolve_vault_path_for_store(&store)) {
                        let _ = vault.delete_secret("messaging_worker_access_token");
                    }
                    false
                } else if let Some(tok) = body
                    .get("access_token")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    if let Ok(vault) = Vault::open(resolve_vault_path_for_store(&store)) {
                        let _ = vault.set_secret("messaging_worker_access_token", tok);
                    }
                    true
                } else {
                    current
                        .get("has_access_token")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false)
                };
                if let Some(dst) = current.as_object_mut() {
                    if let Some(v) = body.get("enabled").and_then(|v| v.as_bool()) {
                        dst.insert("enabled".into(), json!(v));
                    }
                    if let Some(v) = body.get("backend_url").and_then(|v| v.as_str()) {
                        dst.insert("backend_url".into(), json!(v));
                    }
                    if let Some(v) = body.get("execution_backend").and_then(|v| v.as_str()) {
                        dst.insert("execution_backend".into(), json!(v));
                    }
                    if let Some(v) = body.get("turn_timeout_seconds").and_then(|v| v.as_i64()) {
                        dst.insert("turn_timeout_seconds".into(), json!(v.clamp(30, 14400)));
                    }
                    if let Some(v) = body
                        .get("poll_seconds")
                        .or_else(|| body.get("poll_interval_secs"))
                        .and_then(|v| v.as_i64())
                    {
                        let clamped = v.clamp(1, 300);
                        dst.insert("poll_seconds".into(), json!(clamped));
                        dst.insert("poll_interval_secs".into(), json!(clamped));
                    }
                    if body.get("project_root").is_some() {
                        let pr = body
                            .get("project_root")
                            .and_then(|v| v.as_str())
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .map(|s| json!(s))
                            .unwrap_or(Value::Null);
                        dst.insert("project_root".into(), pr);
                    }
                    dst.insert("has_access_token".into(), json!(has_token));
                    dst.remove("access_token");
                }
                let _ = store.set_kv("messaging_channel_worker", &current);
                let worker_root = current
                    .get("project_root")
                    .and_then(|v| v.as_str())
                    .map(PathBuf::from)
                    .filter(|p| p.is_dir())
                    .unwrap_or_else(|| suite.workspace_root.clone());
                let running =
                    reconcile_messaging_worker(messaging_worker, &store, &worker_root);
                if let Some(dst) = current.as_object_mut() {
                    dst.insert("running".into(), json!(running));
                }
                let _ = respond_api_json(req, 200, &current, crypto_tok);
            }

            // --- Agent Rooms & Expert Prompt Templates ---
            (Method::Get, "/api/agent-rooms") => {
                let rooms: Vec<Value> = suite.list_records("agent_room").unwrap_or_default();
                let experts: Vec<Value> = suite.list_records("expert_peer").unwrap_or_default();
                let role_templates = built_in_role_templates();
                let source_store_epoch = ensure_source_store_epoch(&store);
                let _ = respond_api_json(
                    req,
                    200,
                    &json!({
                        "host_id": host_id,
                        "source_store_epoch": source_store_epoch,
                        "rooms": rooms,
                        "experts": experts,
                        "roleTemplates": role_templates
                    }),
                    crypto_tok,
                );
            }
            (Method::Post, "/api/agent-rooms") => {
                let cmd = body
                    .get("command")
                    .filter(|v| v.is_object())
                    .unwrap_or(&body);
                let action = cmd
                    .get("type")
                    .or_else(|| cmd.get("action"))
                    .or_else(|| cmd.get("operation"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let has_explicit_command = body.get("command").is_some_and(|v| v.is_object())
                    || !action.trim().is_empty();
                let dispatch_res = if has_explicit_command {
                    crate::daemon::dispatch_agent_room_command(&store, &suite, &body)
                } else {
                    Ok(json!({}))
                };
                match dispatch_res {
                    Err((code, err)) => {
                        let status = if code == "not_found" { 404 } else { 400 };
                        let _ = respond_api_json(
                            req,
                            status,
                            &json!({"ok": false, "code": code, "error": err}),
                            crypto_tok,
                        );
                    }
                    Ok(result) => {
                        let rooms: Vec<Value> = suite.list_records("agent_room").unwrap_or_default();
                        let experts: Vec<Value> = suite.list_records("expert_peer").unwrap_or_default();
                        let role_templates = built_in_role_templates();
                        let source_store_epoch = ensure_source_store_epoch(&store);
                        let mutated_item = result
                            .get("room")
                            .or_else(|| result.get("expert"))
                            .or_else(|| result.get("submission"))
                            .cloned();
                        let _ = respond_api_json(
                            req,
                            200,
                            &json!({
                                "ok": true,
                                "host_id": host_id,
                                "source_store_epoch": source_store_epoch,
                                "source_matches": true,
                                "result": result,
                                "rooms": rooms,
                                "experts": experts,
                                "roleTemplates": role_templates,
                                "item": mutated_item,
                                "requestType": cmd
                                    .get("type")
                                    .or_else(|| cmd.get("action"))
                                    .or_else(|| cmd.get("operation"))
                                    .cloned()
                                    .unwrap_or(Value::Null)
                            }),
                            crypto_tok,
                        );
                    }
                }
            }
            (Method::Get, "/api/expert-prompt-templates") => {
                // SECURITY: Return ONLY {id, name} — NEVER expose `system_prompt` bodies!
                let mut seen: HashSet<String> = HashSet::new();
                let mut templates: Vec<Value> = Vec::new();
                for tpl in built_in_role_templates() {
                    if let (Some(id), Some(name)) = (
                        tpl.get("id").and_then(|v| v.as_str()),
                        tpl.get("name").and_then(|v| v.as_str()),
                    ) {
                        if seen.insert(id.to_string()) {
                            templates.push(json!({"id": id, "name": name}));
                        }
                    }
                }
                for item in suite.list_records::<Value>("expert_peer").unwrap_or_default() {
                    if let (Some(id), Some(name)) = (
                        item.get("id").and_then(|v| v.as_str()),
                        item.get("name").and_then(|v| v.as_str()),
                    ) {
                        if seen.insert(id.to_string()) {
                            templates.push(json!({"id": id, "name": name}));
                        }
                    }
                }
                let _ = respond_api_json(
                    req,
                    200,
                    &json!({"revision": 1, "templates": templates}),
                    crypto_tok,
                );
            }
            (Method::Delete, p)
                if p.starts_with("/api/agent-rooms/experts/")
                    && !p.trim_start_matches("/api/agent-rooms/experts/").contains('/') =>
            {
                let id = percent_decode_path(p.trim_start_matches("/api/agent-rooms/experts/"));
                let deleted = suite.delete_record("expert_peer", &id).unwrap_or(false);
                let _ = respond_api_json(
                    req,
                    200,
                    &json!({"ok": true, "deleted": deleted, "id": id}),
                    crypto_tok,
                );
            }
            (Method::Delete, p)
                if p.starts_with("/api/agent-rooms/")
                    && !p.trim_start_matches("/api/agent-rooms/").contains('/') =>
            {
                let id = percent_decode_path(p.trim_start_matches("/api/agent-rooms/"));
                let deleted = suite.delete_record("agent_room", &id).unwrap_or(false);
                let _ = respond_api_json(
                    req,
                    200,
                    &json!({"ok": true, "deleted": deleted, "id": id}),
                    crypto_tok,
                );
            }
            (Method::Post, p) if p.starts_with("/api/agent-rooms/") && p.ends_with("/messages") => {
                let room_id = percent_decode_path(
                    p.trim_start_matches("/api/agent-rooms/")
                        .trim_end_matches("/messages"),
                );
                let msg = body
                    .get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let target_recipient = body
                    .get("to")
                    .or_else(|| body.get("recipient"))
                    .or_else(|| body.get("target_recipient"))
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(ToString::to_string);
                let crypto_tok_owned = crypto_tok.map(ToString::to_string);
                let cfg = load_active_config(&store);
                match resolve_model_route(&cfg).and_then(|route| {
                    let caller: Arc<dyn ModelCaller> =
                        Arc::new(HttpModelCaller::new(300).map_err(|e| e.to_string())?);
                    let policy = EffectivePolicy::load(
                        &suite.workspace_root,
                        suite.user_home.as_deref(),
                        false,
                        Some(PermissionProfile::Cautious),
                        true,
                    );
                    RoomRuntime::send_message_to(
                        &suite,
                        caller,
                        route,
                        policy,
                        &room_id,
                        &msg,
                        target_recipient.as_deref(),
                    )
                }) {
                    Ok(replies) => {
                        let _ = respond_api_json(
                            req,
                            200,
                            &json!({"replies": replies}),
                            crypto_tok_owned.as_deref(),
                        );
                    }
                    Err(e) => {
                        let _ = respond_api_json(
                            req,
                            400,
                            &json!({"error": e}),
                            crypto_tok_owned.as_deref(),
                        );
                    }
                }
            }
            (Method::Post, p) if p.starts_with("/api/goals/") && p.ends_with("/run") => {
                let goal_id = percent_decode_path(
                    p.trim_start_matches("/api/goals/")
                        .trim_end_matches("/run"),
                );
                let crypto_tok_owned = crypto_tok.map(ToString::to_string);
                let cfg = load_active_config(&store);
                match resolve_model_route(&cfg).and_then(|route| {
                    let caller: Arc<dyn ModelCaller> =
                        Arc::new(HttpModelCaller::new(300).map_err(|e| e.to_string())?);
                    let policy = EffectivePolicy::load(
                        &suite.workspace_root,
                        suite.user_home.as_deref(),
                        false,
                        Some(PermissionProfile::Cautious),
                        true,
                    );
                    GoalWorker::execute_goal(
                        &suite,
                        caller,
                        route,
                        policy,
                        &goal_id,
                        32,
                        Arc::new(AtomicBool::new(false)),
                    )
                }) {
                    Ok(goal) => {
                        let _ = respond_api_json(
                            req,
                            200,
                            &json!({"goal": goal}),
                            crypto_tok_owned.as_deref(),
                        );
                    }
                    Err(e) => {
                        let _ = respond_api_json(
                            req,
                            400,
                            &json!({"error": e}),
                            crypto_tok_owned.as_deref(),
                        );
                    }
                }
            }
            (Method::Post, "/api/automations/tick") => {
                let crypto_tok_owned = crypto_tok.map(ToString::to_string);
                let cfg = load_active_config(&store);
                match resolve_model_route(&cfg).and_then(|route| {
                    let caller: Arc<dyn ModelCaller> =
                        Arc::new(HttpModelCaller::new(300).map_err(|e| e.to_string())?);
                    let policy = EffectivePolicy::load(
                        &suite.workspace_root,
                        suite.user_home.as_deref(),
                        false,
                        Some(PermissionProfile::Cautious),
                        true,
                    );
                    Ok(AutomationScheduler::run_due(
                        &suite,
                        caller,
                        route,
                        policy,
                        Utc::now(),
                    ))
                }) {
                    Ok(results) => {
                        let _ = respond_api_json(
                            req,
                            200,
                            &json!({"triggered": results.len()}),
                            crypto_tok_owned.as_deref(),
                        );
                    }
                    Err(e) => {
                        let _ = respond_api_json(
                            req,
                            400,
                            &json!({"error": e}),
                            crypto_tok_owned.as_deref(),
                        );
                    }
                }
            }

            // --- Chrome Native Browser Extension Bridge ---
            (Method::Get, "/api/native-browser/status") => {
                let mut st = native_browser.status();
                if let Some(obj) = st.as_object_mut() {
                    obj.entry("hosts").or_insert_with(|| json!([]));
                }
                let _ = respond_api_json(req, 200, &st, crypto_tok);
            }
            (Method::Post, "/api/native-browser/command") => {
                let id = native_browser.enqueue(body);
                let _ = respond_api_json(req, 200, &json!({"command_id": id}), crypto_tok);
            }
            (Method::Post, "/api/native-browser/host/poll") => {
                if let Some(next) = native_browser.poll_next() {
                    let _ = respond_json(req, 200, &json!({"type": "command", "command": next}));
                } else {
                    let _ = respond_json(req, 200, &json!({"type": "idle", "retry_ms": 500}));
                }
            }
            (Method::Post, "/api/native-browser/host/result") => {
                if let Some(id) = body.get("command_id").and_then(|v| v.as_str()) {
                    native_browser.submit_result(id.to_string(), body);
                }
                let _ = respond_json(req, 200, &json!({"ok": true}));
            }

            // --- Generic Unified Module CRUD (/api/modules/<kind>) with SEC-7 Allowlist ---
            (Method::Get, p) if p.starts_with("/api/modules/") => {
                let kind = p.trim_start_matches("/api/modules/");
                if !is_allowed_module_kind(kind) {
                    let _ = respond_api_json(
                        req,
                        403,
                        &json!({"error": "module_kind_forbidden", "kind": kind}),
                        crypto_tok,
                    );
                    return;
                }
                let items: Vec<Value> = suite.list_records(kind).unwrap_or_default();
                let _ = respond_api_json(req, 200, &json!({"kind": kind, "items": items}), crypto_tok);
            }
            (Method::Post, p) if p.starts_with("/api/modules/") => {
                let kind = p.trim_start_matches("/api/modules/").to_string();
                if !is_allowed_module_kind(&kind) {
                    let _ = respond_api_json(
                        req,
                        403,
                        &json!({"error": "module_kind_forbidden", "kind": kind}),
                        crypto_tok,
                    );
                    return;
                }
                let id = body
                    .get("id")
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string)
                    .unwrap_or_else(new_id);
                match suite.upsert_record(&kind, &id, &body) {
                    Ok(()) => {
                        let _ = respond_api_json(req, 200, &json!({"ok": true, "id": id}), crypto_tok);
                    }
                    Err(e) => {
                        let _ = respond_api_json(req, 500, &json!({"error": e.to_string()}), crypto_tok);
                    }
                }
            }
            (Method::Delete, p) if p.starts_with("/api/modules/") => {
                let rest = p.trim_start_matches("/api/modules/");
                if let Some((kind, id)) = rest.split_once('/') {
                    if !is_allowed_module_kind(kind) {
                        let _ = respond_api_json(
                            req,
                            403,
                            &json!({"error": "module_kind_forbidden", "kind": kind}),
                            crypto_tok,
                        );
                        return;
                    }
                    let id = percent_decode_path(id);
                    let deleted = suite.delete_record(kind, &id).unwrap_or(false);
                    let _ = respond_api_json(req, 200, &json!({"deleted": deleted}), crypto_tok);
                } else {
                    let _ = respond_api_json(req, 400, &json!({"error": "missing module id"}), crypto_tok);
                }
            }

            // --- Unknown API endpoints never fall through to static assets ---
            (_, p) if p.starts_with("/api/") => {
                let _ = respond_api_json(req, 404, &json!({"error": "not_found"}), crypto_tok);
            }
            // --- SPA Convenience Redirect: `/automations` -> `/scheduled-actions` ---
            (Method::Get, "/automations") | (Method::Get, "/ui/automations") => {
                let loc = Header::from_bytes(&b"Location"[..], &b"/scheduled-actions"[..]).unwrap();
                let resp = Response::empty(StatusCode(302)).with_header(loc);
                let _ = req.respond(resp);
            }
            // --- Static frontend assets (/ui/*, /static/*, and SPA fallback) ---
            (Method::Get, p) => {
                serve_static_asset(req, ui_dir, static_dir, p);
            }
            _ => {
                let _ = respond_json(req, 404, &json!({"error": "not_found"}));
            }
    }
}

fn run_automation_bash_script(script: &str, cwd: &Path) -> Result<String, String> {
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

fn is_read_only_post(body: &Value) -> bool {
    let Some(obj) = body.as_object() else {
        return true;
    };
    if obj.is_empty() {
        return true;
    }
    if let Some(op) = obj
        .get("operation")
        .or_else(|| obj.get("action"))
        .and_then(|v| v.as_str())
    {
        if matches!(
            op.to_ascii_lowercase().as_str(),
            "read" | "get" | "fetch" | "list" | "journal" | "query" | "history"
        ) {
            return true;
        }
    }
    let mutation_keys = [
        "title",
        "isPinned",
        "selectedModelSource",
        "selectedModelId",
        "permissionProfile",
        "permission_profile",
        "reasoningEffort",
        "knowledge",
        "expectedRevision",
        "expected_revision",
        "content",
        "message",
        "text",
        "parts",
        "prompt",
        "user_message",
        "attachments",
    ];
    !mutation_keys.iter().any(|k| obj.contains_key(*k))
}

fn is_conversations_list_read_post(body: &Value) -> bool {
    let Some(obj) = body.as_object() else {
        return false;
    };
    let explicit_op = obj
        .get("operation")
        .or_else(|| obj.get("action"))
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_ascii_lowercase());
    if let Some(ref op) = explicit_op {
        if matches!(
            op.as_str(),
            "read" | "get" | "fetch" | "list" | "journal" | "query"
        ) {
            return true;
        }
        if matches!(op.as_str(), "create" | "new" | "upsert") {
            return false;
        }
    }
    let has_query_marker = obj.contains_key("journal")
        || obj.contains_key("limit")
        || obj.contains_key("cursor")
        || obj.contains_key("after_seq")
        || obj.contains_key("afterSeq");
    if has_query_marker {
        return true;
    }
    if has_expected_host_id(body) {
        let has_create_title = obj
            .get("title")
            .and_then(|v| v.as_str())
            .is_some_and(|s| !s.trim().is_empty());
        return !has_create_title;
    }
    false
}

#[cfg(test)]
fn is_messages_read_post(body: &Value) -> bool {
    is_messages_read_post_with_accept(body, false)
}

fn is_messages_read_post_with_accept(body: &Value, accepts_sse: bool) -> bool {
    let Some(obj) = body.as_object() else {
        return true;
    };
    if let Some(op) = obj
        .get("operation")
        .or_else(|| obj.get("action"))
        .and_then(|v| v.as_str())
    {
        if matches!(
            op.to_ascii_lowercase().as_str(),
            "read" | "get" | "fetch" | "list" | "history" | "query"
        ) {
            return true;
        }
    }
    // macOS remote chat sends `messages: [...]`; iOS `resumeObservedRun` sends `expected_run_id`
    // with `Accept: text/event-stream`. Neither should be hijacked into JSON history reads!
    if obj
        .get("messages")
        .and_then(|v| v.as_array())
        .is_some_and(|a| !a.is_empty())
    {
        return false;
    }
    if accepts_sse && obj.get("expected_run_id").is_some() {
        return false;
    }
    let has_text = ["content", "message", "text", "prompt", "user_message"]
        .iter()
        .any(|k| {
            obj.get(*k)
                .and_then(|v| v.as_str())
                .is_some_and(|s| !s.trim().is_empty())
        });
    let has_parts = obj
        .get("parts")
        .and_then(|v| v.as_array())
        .is_some_and(|a| !a.is_empty());
    let has_attachments = obj
        .get("attachments")
        .and_then(|v| v.as_array())
        .is_some_and(|a| !a.is_empty());
    if has_text || has_parts || has_attachments {
        return false;
    }
    is_read_only_post(body) || has_expected_host_id(body)
}

fn filter_superseded_memories(store: &Store, items: Vec<MemoryItem>) -> (Vec<Value>, Vec<MemoryItem>) {
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

fn wrap_paired_envelope(store: &Store, host_id: &str, result: Value) -> Value {
    let source_store_epoch = ensure_source_store_epoch(store);
    json!({
        "source_host_id": host_id,
        "source_store_epoch": source_store_epoch,
        "result": result
    })
}

fn is_assistant_journal_operation(body: &Value) -> bool {
    let Some(op) = body.get("operation").and_then(|v| v.as_str()) else {
        return false;
    };
    match op {
        "open_snapshot"
        | "release_snapshot"
        | "terminal_seal"
        | "terminal_manifest"
        | "admission_head"
        | "admissions"
        | "context_usage"
        | "heads"
        | "parts"
        | "changes"
        | "range"
        | "record_chunk" => true,
        "read" => body.get("after").is_some() || body.get("through").is_some(),
        _ => false,
    }
}

fn handle_assistant_journal_http(
    store: &Store,
    suite: &ModulesSuite,
    host_id: &str,
    body: &Value,
    is_mobile: bool,
) -> Result<Value, (&'static str, String)> {
    let op = body
        .get("operation")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let daemon_cmd = match op {
        "open_snapshot" => "open_assistant_journal_snapshot",
        "release_snapshot" => "release_assistant_journal_snapshot",
        "terminal_seal" => "local_assistant_run_terminal_seal",
        "terminal_manifest" => "read_local_assistant_run_terminal_manifest",
        "admission_head" => "local_run_message_admission_head",
        "admissions" => "read_run_message_admissions",
        "context_usage" => "local_run_context_usage",
        "heads" => "local_run_assistant_journal_heads",
        "read" => "read_assistant_journal",
        "parts" => "read_assistant_journal_parts",
        "changes" => "read_assistant_journal_part_changes",
        "range" => "read_assistant_journal_range",
        "record_chunk" => "read_assistant_journal_record_chunk",
        other => {
            return Err((
                "unsupported_journal_operation",
                format!("unsupported assistant journal operation '{other}'"),
            ))
        }
    };
    let result = crate::daemon::dispatch_command(store, suite, daemon_cmd, body)?;
    if is_mobile || has_expected_host_id(body) {
        Ok(wrap_paired_envelope(store, host_id, result))
    } else {
        Ok(result)
    }
}

fn is_paired_task_cancellation_body(body: &Value) -> bool {
    let Some(obj) = body.as_object() else {
        return false;
    };
    let has_task = obj
        .get("taskId")
        .or_else(|| obj.get("task_id"))
        .and_then(|v| v.as_str())
        .is_some_and(|s| !s.trim().is_empty());
    let has_paired_field = obj.contains_key("hostId")
        || obj.contains_key("expected_host_id")
        || obj.contains_key("parentRunId")
        || obj.contains_key("parent_run_id")
        || obj.contains_key("receipt_only")
        || obj.contains_key("receiptOnly");
    has_task || has_paired_field
}

fn handle_paired_task_cancellation(
    store: &Store,
    suite: &ModulesSuite,
    active_cancels: &Arc<Mutex<BTreeMap<String, Arc<AtomicBool>>>>,
    host_id: &str,
    conv_id: &str,
    url_task_id: Option<&str>,
    body: &Value,
) -> Value {
    let req_host_id = body
        .get("hostId")
        .or_else(|| body.get("expected_host_id"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(host_id);
    let task_id = url_task_id
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            body.get("taskId")
                .or_else(|| body.get("task_id"))
                .and_then(|v| v.as_str())
        })
        .unwrap_or("default");
    let parent_run_id = body
        .get("parentRunId")
        .or_else(|| body.get("parent_run_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let receipt_only = body
        .get("receipt_only")
        .or_else(|| body.get("receiptOnly"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let mut task_rec = suite.get_record::<Value>("runtime_task", task_id).ok().flatten();
    if !receipt_only {
        if let Some(flag) = active_cancels.lock().unwrap().get(conv_id) {
            flag.store(true, Ordering::SeqCst);
        }
        let ts = now_ms();
        if let Some(ref mut task) = task_rec {
            if let Some(obj) = task.as_object_mut() {
                obj.insert("cancel_requested".into(), json!(true));
                obj.insert("updated_at_ms".into(), json!(ts));
            }
            let _ = suite.upsert_record("runtime_task", task_id, task);
        }
        let _ = store.set_kv(
            &format!("cancel_requested:{task_id}"),
            &json!({"task_id": task_id, "cancel_requested": true, "requested_at_ms": ts}),
        );
    }

    let raw_state = task_rec
        .as_ref()
        .and_then(|t| t.get("status").or_else(|| t.get("state")).and_then(|v| v.as_str()))
        .unwrap_or("cancelled");
    let state = match raw_state {
        "completed" | "sealed" => "sealed",
        "failed" | "error" => "failed",
        "cancelled" | "canceled" => "cancelled",
        "queued" => "queued",
        "running" => "running",
        "retry_waiting" => "retry_waiting",
        "waiting_permission" => "waiting_permission",
        "suspended" => "suspended",
        "awaiting_reconciliation" => "awaiting_reconciliation",
        _ => "cancelled",
    };
    let terminal = matches!(state, "sealed" | "failed" | "cancelled");
    let cancel_requested = if !receipt_only {
        true
    } else {
        task_rec
            .as_ref()
            .and_then(|t| t.get("cancel_requested").and_then(|v| v.as_bool()))
            .unwrap_or_else(|| {
                store
                    .get_kv(&format!("cancel_requested:{task_id}"))
                    .ok()
                    .flatten()
                    .and_then(|v| v.get("cancel_requested").and_then(|b| b.as_bool()))
                    .unwrap_or(terminal)
            })
    };

    // Strict 8-key RemoteTaskCancellation / AppleTaskCancellationReceipt contract
    json!({
        "hostId": req_host_id,
        "conversationId": conv_id,
        "parentRunId": parent_run_id,
        "taskId": task_id,
        "cancelRequested": cancel_requested,
        "terminal": terminal,
        "state": state,
        "applicationPending": false
    })
}

fn is_task_guidance_post(body: &Value, accepts_sse: bool) -> bool {
    if accepts_sse {
        return false;
    }
    let Some(obj) = body.as_object() else {
        return false;
    };
    let has_task = obj
        .get("taskId")
        .or_else(|| obj.get("task_id"))
        .and_then(|v| v.as_str())
        .is_some_and(|s| !s.trim().is_empty());
    let has_guidance_shape = obj.contains_key("guidance")
        || obj.contains_key("messageId")
        || obj.contains_key("message_id")
        || obj.contains_key("parentRunId")
        || obj.contains_key("parent_run_id");
    has_task && has_guidance_shape
}

fn handle_task_guidance_http(
    store: &Store,
    host_id: &str,
    conv_id: &str,
    body: &Value,
    receipt_only: bool,
) -> Value {
    let req_host_id = body
        .get("hostId")
        .or_else(|| body.get("expected_host_id"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(host_id);
    let task_id = body
        .get("taskId")
        .or_else(|| body.get("task_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let message_id = body
        .get("messageId")
        .or_else(|| body.get("message_id"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(new_id);
    let parent_run_id = body
        .get("parentRunId")
        .or_else(|| body.get("parent_run_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let submitted_at_ms = body
        .get("submittedAtMs")
        .or_else(|| body.get("submitted_at_ms"))
        .and_then(|v| v.as_i64())
        .unwrap_or_else(now_ms);
    let content = body
        .get("message")
        .or_else(|| body.get("guidance"))
        .or_else(|| body.get("content"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let key = format!("task_guidance_receipt:{task_id}:{message_id}");
    let existing = store.get_kv(&key).ok().flatten();
    let guidance_val = if receipt_only {
        existing.unwrap_or(Value::Null)
    } else if let Some(found) = existing {
        found
    } else {
        let rec = json!({
            "task_id": task_id,
            "message_id": message_id,
            "owner_session_id": conv_id,
            "parent_turn_id": parent_run_id,
            "content": content,
            "submitted_at_ms": submitted_at_ms,
            "status": "accepted"
        });
        let _ = store.set_kv(&key, &rec);
        let _ = store.set_kv(&format!("task_guidance_receipt:{message_id}"), &rec);
        let _ = store.set_kv(&format!("task_guidance_receipt:{task_id}"), &rec);
        rec
    };

    json!({
        "hostId": req_host_id,
        "conversationId": conv_id,
        "taskId": task_id,
        "messageId": message_id,
        "guidance": guidance_val
    })
}

fn handle_remote_submission_reconciliation(
    store: &Store,
    active_cancels: &Arc<Mutex<BTreeMap<String, Arc<AtomicBool>>>>,
    _host_id: &str,
    conv_id: &str,
    body: &Value,
) -> Value {
    let op = body
        .get("operation")
        .and_then(|v| v.as_str())
        .unwrap_or("observe");
    let identity = body.get("identity").cloned().unwrap_or_else(|| json!({}));
    let request_digest = body
        .get("request_digest")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let sub_id = identity
        .get("submission_id")
        .and_then(|v| v.as_str())
        .unwrap_or("default");
    let asst_id = identity
        .get("assistant_message_id")
        .and_then(|v| v.as_str())
        .unwrap_or("assistant");
    let key = format!("remote_sub_server_receipt:{conv_id}:{sub_id}");

    if op == "cancel" {
        if let Some(flag) = active_cancels.lock().unwrap().get(conv_id) {
            flag.store(true, Ordering::SeqCst);
        }
        let receipt = json!({
            "protocol_version": 1,
            "identity": identity,
            "request_digest": request_digest,
            "phase": "cancelled",
            "admission": Value::Null,
            "rejection_code": Value::Null
        });
        let _ = store.set_kv(&key, &receipt);
        return receipt;
    }

    if let Ok(Some(saved)) = store.get_kv(&key) {
        if saved.get("request_digest").and_then(|v| v.as_str()) == Some(request_digest) {
            return saved;
        }
    }

    let has_session = store.get_session(conv_id).ok().flatten().is_some();
    if has_session {
        let epoch = ensure_source_store_epoch(store);
        let record_digest = nola_types::sha256_hex(
            format!("{conv_id}:{sub_id}:{asst_id}:{request_digest}").as_bytes(),
        );
        let receipt = json!({
            "protocol_version": 1,
            "identity": identity,
            "request_digest": request_digest,
            "phase": "admitted",
            "admission": {
                "identity": {
                    "conversation_id": conv_id,
                    "run_id": sub_id,
                    "assistant_message_id": asst_id,
                    "source_store_epoch": epoch
                },
                "sequence": "1",
                "record_digest": record_digest
            },
            "rejection_code": Value::Null
        });
        let _ = store.set_kv(&key, &receipt);
        receipt
    } else {
        json!({
            "protocol_version": 1,
            "identity": identity,
            "request_digest": request_digest,
            "phase": "prepared",
            "admission": Value::Null,
            "rejection_code": Value::Null
        })
    }
}

fn percent_encode_path_segment(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for b in input.bytes() {
        let unreserved = b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~');
        if unreserved {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

fn extract_resource_raw_bytes(res: &Value) -> Option<Vec<u8>> {
    if let Some(b64) = res
        .get("data_base64")
        .or_else(|| res.get("dataBase64"))
        .or_else(|| res.get("content_base64"))
        .or_else(|| res.get("data"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
    {
        let cleaned = b64
            .split_once(",")
            .map(|(_, rest)| rest)
            .unwrap_or(b64);
        if let Ok(bytes) = base64_decode(cleaned) {
            return Some(bytes);
        }
    }
    if let Some(data_url) = res
        .get("dataUrl")
        .or_else(|| res.get("data_url"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
    {
        if let Some((_, b64)) = data_url.split_once(",") {
            if let Ok(bytes) = base64_decode(b64) {
                return Some(bytes);
            }
        }
    }
    if let Some(text) = res
        .get("extracted_text")
        .or_else(|| res.get("text"))
        .or_else(|| res.get("content"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        return Some(text.as_bytes().to_vec());
    }
    None
}

fn normalize_canonical_resource(conv_id: &str, rid: &str, raw: &Value, include_data: bool) -> Value {
    let display_name = raw
        .get("displayName")
        .or_else(|| raw.get("filename"))
        .or_else(|| raw.get("name"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("document.pdf");
    let display_name: String = display_name.chars().take(256).collect();
    let mime_type = raw
        .get("mimeType")
        .or_else(|| raw.get("mime_type"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| {
            if display_name.to_ascii_lowercase().ends_with(".pdf") {
                "application/pdf"
            } else {
                "application/octet-stream"
            }
        });
    let raw_bytes = extract_resource_raw_bytes(raw);
    let byte_count = raw
        .get("byteCount")
        .or_else(|| raw.get("size_bytes"))
        .or_else(|| raw.get("size"))
        .and_then(|v| v.as_u64())
        .filter(|&n| n > 0)
        .or_else(|| raw_bytes.as_ref().map(|b| b.len() as u64))
        .unwrap_or(1);
    let sha256 = raw
        .get("sha256")
        .and_then(|v| v.as_str())
        .filter(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
        .map(ToString::to_string)
        .unwrap_or_else(|| {
            if let Some(ref b) = raw_bytes {
                nola_types::sha256_hex(b)
            } else {
                nola_types::sha256_hex(format!("{conv_id}:{rid}:{display_name}").as_bytes())
            }
        });
    let preview_url = format!(
        "/api/chat/conversations/{}/resources/{}",
        percent_encode_path_segment(conv_id),
        percent_encode_path_segment(rid)
    );

    let mut obj = raw.as_object().cloned().unwrap_or_default();
    if !include_data {
        obj.remove("data_base64");
        obj.remove("dataBase64");
        obj.remove("content_base64");
        obj.remove("dataUrl");
        obj.remove("data_url");
        obj.remove("data");
    }
    obj.insert("id".into(), json!(rid));
    obj.insert("resourceId".into(), json!(rid));
    obj.insert("conversationId".into(), json!(conv_id));
    obj.insert("displayName".into(), json!(display_name));
    obj.insert("mimeType".into(), json!(mime_type));
    obj.insert("byteCount".into(), json!(byte_count));
    obj.insert("sha256".into(), json!(sha256));
    obj.insert("previewUrl".into(), json!(preview_url));
    Value::Object(obj)
}

fn store_conversation_resource_upload(
    store: &Store,
    conv_id: &str,
    body: &Value,
) -> Result<Value, String> {
    let display_name = body
        .get("displayName")
        .or_else(|| body.get("filename"))
        .or_else(|| body.get("name"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("document.pdf");
    let b64 = body
        .get("dataBase64")
        .or_else(|| body.get("data_base64"))
        .or_else(|| body.get("content_base64"))
        .or_else(|| body.get("base64"))
        .or_else(|| body.get("data"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing dataBase64".to_string())?;
    let bytes = base64_decode(b64)?;
    if bytes.is_empty() || bytes.len() > 8 * 1024 * 1024 {
        return Err("resource size must be between 1 byte and 8 MiB".into());
    }
    let resource_id = body
        .get("resourceId")
        .or_else(|| body.get("resource_id"))
        .or_else(|| body.get("id"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(new_id);
    let mime_type = body
        .get("mimeType")
        .or_else(|| body.get("mime_type"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("application/pdf");
    let digest = nola_types::sha256_hex(&bytes);
    let ts = now_ms();
    let record = json!({
        "resource_id": resource_id,
        "resourceId": resource_id,
        "id": resource_id,
        "conversation_id": conv_id,
        "conversationId": conv_id,
        "kind": "pdf",
        "mime_type": mime_type,
        "mimeType": mime_type,
        "filename": display_name,
        "displayName": display_name,
        "size_bytes": bytes.len() as u64,
        "byteCount": bytes.len() as u64,
        "sha256": digest,
        "data_base64": base64_encode(&bytes),
        "created_at_ms": ts,
        "updated_at_ms": ts
    });
    store
        .put_module_record(
            "conversation_resource",
            &format!("{conv_id}:{resource_id}"),
            &record,
        )
        .map_err(|e| e.to_string())?;
    Ok(normalize_canonical_resource(conv_id, &resource_id, &record, false))
}

fn collect_conversation_resources_inner(
    store: &Store,
    conv_id: &str,
    include_data: bool,
) -> Vec<Value> {
    let mut resources: Vec<Value> = Vec::new();
    let mut seen_ids: HashSet<String> = HashSet::new();

    if let Ok(module_recs) = store.list_module_records::<Value>("conversation_resource") {
        for (idx, res) in module_recs.into_iter().enumerate() {
            let matches_conv = res
                .get("conversation_id")
                .or_else(|| res.get("conversationId"))
                .and_then(|v| v.as_str())
                == Some(conv_id);
            if !matches_conv {
                continue;
            }
            let rid = res
                .get("resourceId")
                .or_else(|| res.get("resource_id"))
                .or_else(|| res.get("id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(|| format!("mod-res-{idx}"));
            if seen_ids.insert(rid.clone()) {
                resources.push(normalize_canonical_resource(conv_id, &rid, &res, include_data));
            }
        }
    }

    if let Ok(Some(Value::Array(saved))) = store.get_kv(&format!("conv_resources:{conv_id}")) {
        for (idx, res) in saved.into_iter().enumerate() {
            let rid = res
                .get("resourceId")
                .or_else(|| res.get("id"))
                .or_else(|| res.get("resource_id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(|| format!("kv-res-{idx}"));
            if seen_ids.insert(rid.clone()) {
                resources.push(normalize_canonical_resource(conv_id, &rid, &res, include_data));
            }
        }
    }

    if let Ok(EventPage::Events(events)) = store.events_after(conv_id, 0, 2000) {
        for ev in events {
            if let Some(attachments) = ev.payload.get("attachments").and_then(|v| v.as_array()) {
                for (idx, att) in attachments.iter().enumerate() {
                    let rid = att
                        .get("resourceId")
                        .or_else(|| att.get("id"))
                        .or_else(|| att.get("resource_id"))
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.trim().is_empty())
                        .map(ToString::to_string)
                        .unwrap_or_else(|| format!("res-{}-{idx}", ev.seq));
                    if seen_ids.insert(rid.clone()) {
                        let mut entry = att.clone();
                        if let Some(obj) = entry.as_object_mut() {
                            obj.entry("seq".to_string()).or_insert_with(|| json!(ev.seq));
                        }
                        resources.push(normalize_canonical_resource(
                            conv_id,
                            &rid,
                            &entry,
                            include_data,
                        ));
                    }
                }
            }
            if let Some(res) = ev.payload.get("resource").filter(|v| v.is_object()) {
                let rid = res
                    .get("resourceId")
                    .or_else(|| res.get("id"))
                    .or_else(|| res.get("resource_id"))
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
                    .map(ToString::to_string)
                    .unwrap_or_else(|| format!("res-{}", ev.seq));
                if seen_ids.insert(rid.clone()) {
                    resources.push(normalize_canonical_resource(
                        conv_id,
                        &rid,
                        res,
                        include_data,
                    ));
                }
            }
        }
    }
    resources
}

fn collect_conversation_resources_raw(store: &Store, conv_id: &str) -> Vec<Value> {
    collect_conversation_resources_inner(store, conv_id, true)
}

fn collect_conversation_resources(store: &Store, conv_id: &str) -> Vec<Value> {
    collect_conversation_resources_inner(store, conv_id, false)
}

fn discover_endpoint_models(conn: &ConnectionConfig) -> Vec<String> {
    let base = conn.endpoint.trim_end_matches('/');
    if base.is_empty() {
        return Vec::new();
    }
    let url = if base.ends_with("/models") {
        base.to_string()
    } else {
        format!("{base}/models")
    };
    let mut models: Vec<String> = Vec::new();
    let mut builder = reqwest::blocking::Client::builder().timeout(Duration::from_secs(10));
    if is_loopback_url(&url) {
        builder = builder.no_proxy();
    }
    if let Ok(client) = builder.build() {
        let mut rb = client.get(&url);
        if let Some(ref key) = conn.api_key {
            if !key.trim().is_empty() && !key.starts_with("env:") {
                rb = rb.bearer_auth(key.trim());
            }
        }
        if let Ok(resp) = rb.send() {
            if let Ok(val) = resp.json::<Value>() {
                if let Some(arr) = val.get("data").and_then(|v| v.as_array()) {
                    for item in arr {
                        if let Some(id) = item.get("id").and_then(|v| v.as_str()) {
                            models.push(id.to_string());
                        }
                    }
                }
            }
        }
    }
    models
}

fn bump_endpoint_credential_revision(store: &Store, ep_id: &str) -> i64 {
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

fn build_endpoint_credential_response(store: &Store, ep_id: &str) -> Option<Value> {
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

fn build_paired_tool_plan(
    suite: &Arc<ModulesSuite>,
    host_id: &str,
    body: &Value,
    manifest_id: String,
) -> Value {
    let mut reg = ToolRegistry::with_builtin_tools();
    suite.register_tools(&mut reg);
    let specs = reg.all_specs();
    let issued_at_ms = now_ms();
    let expires_at_ms = issued_at_ms.saturating_add(300_000);
    let client_id = body
        .get("client_id")
        .or_else(|| body.get("clientId"))
        .or_else(|| body.pointer("/reference/client_id"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("nola-client");
    let session_id = body
        .get("session_id")
        .or_else(|| body.get("sessionId"))
        .or_else(|| body.get("conversation_id"))
        .or_else(|| body.pointer("/reference/session_id"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("default-session");
    let tools: Vec<Value> = specs
        .iter()
        .map(|s| {
            json!({
                "tool_id": s.schema.name,
                "route": "agent_host",
                "schema": {
                    "type": "function",
                    "function": {
                        "name": s.schema.name,
                        "description": s.schema.description,
                        "parameters": s.schema.parameters
                    }
                }
            })
        })
        .collect();
    let catalog_bytes = serde_json::to_vec(&tools).unwrap_or_default();
    let catalog_revision = sha256_hex(&catalog_bytes);
    json!({
        "schema_version": 1,
        "reference": {
            "manifest_id": manifest_id,
            "host_id": host_id,
            "client_id": client_id,
            "session_id": session_id,
            "catalog_revision": catalog_revision
        },
        "issued_at_ms": issued_at_ms,
        "expires_at_ms": expires_at_ms,
        "tools": tools
    })
}

fn parse_permission_resolution(body: &Value) -> (bool, &'static str) {
    if let Some(decision) = body
        .pointer("/resolution/decision")
        .or_else(|| body.get("decision"))
        .and_then(|v| v.as_str())
    {
        match decision.trim().to_ascii_lowercase().as_str() {
            "allow_once" | "approve_once" => return (true, "once"),
            "allow_conversation" | "allow_session" | "allow_always" => return (true, "session"),
            "deny" | "reject" => return (false, "once"),
            _ => {}
        }
    }
    let allow = body
        .get("allow")
        .or_else(|| body.get("approved"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let scope = match body
        .get("scope")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_ascii_lowercase())
        .as_deref()
    {
        Some("session") | Some("conversation") | Some("always") => "session",
        _ => "once",
    };
    (allow, scope)
}

fn build_goal_projection(store: &Store, conv_id: &str, goal: &Value) -> Value {
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

fn is_sensitive_data_source_record(suite: &ModulesSuite, record: &Value) -> bool {
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

fn managed_local_models_dir() -> PathBuf {
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

fn confined_local_model_dest_path(requested_dest: Option<&str>, fallback_name: &str) -> PathBuf {
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

fn build_local_model_download_url(
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

fn enrich_download_status_json(store: &Store, status: &mut Value) {
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

fn build_vllm_recommend_candidates(hardware: &Value) -> Vec<Value> {
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

fn extract_hybrid_device_tool_specs(body: &Value) -> Vec<(ToolSpec, String)> {
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

fn is_sensitive_local_path(path: &Path) -> bool {
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

fn redact_secrets_in_text(input: &str) -> String {
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

fn probe_and_attach_data_source_validation(
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

fn parse_message_seq(body: &Value) -> Option<i64> {
    body.get("fromSeq")
        .or_else(|| body.get("from_seq"))
        .or_else(|| body.get("upToSeq"))
        .or_else(|| body.get("up_to_seq"))
        .and_then(|v| v.as_i64())
        .or_else(|| {
            body.get("fromMessageId")
                .or_else(|| body.get("messageId"))
                .or_else(|| body.get("message_id"))
                .and_then(|v| v.as_str())
                .and_then(|s| s.strip_prefix("msg-"))
                .and_then(|s| s.parse::<i64>().ok())
        })
}

fn synchronize_session_state_after_truncate(store: &Store, conv_id: &str) {
    let state_key = format!("session_state:{conv_id}");
    let Ok(Some(mut state_val)) = store.get_kv(&state_key) else {
        return;
    };
    let (messages, _) = reconstruct_v2_messages(store, conv_id);
    let rebuilt_history: Vec<Value> = messages
        .iter()
        .filter_map(|m| {
            let role = m.get("role").and_then(|v| v.as_str())?;
            let content = m.get("content").and_then(|v| v.as_str()).unwrap_or("");
            match role {
                "user" => Some(json!({
                    "role": "user",
                    "content": content
                })),
                "assistant" => {
                    let reasoning = m
                        .get("reasoning")
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.is_empty());
                    let mut obj = json!({
                        "role": "assistant",
                        "content": content
                    });
                    if let Some(r) = reasoning {
                        obj["reasoning"] = json!(r);
                    }
                    Some(obj)
                }
                _ => None,
            }
        })
        .collect();
    if let Some(obj) = state_val.as_object_mut() {
        obj.insert("history".into(), Value::Array(rebuilt_history));
    }
    let _ = store.set_kv(&state_key, &state_val);
}

fn build_activity_page(
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

fn build_activity_detail(
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

fn build_model_connections_inventory(store: &Store, suite: &ModulesSuite) -> Value {
    let cfg = load_active_config(store);
    let default_sel = cfg.llm.default_model.as_ref();
    let mut connections: Vec<Value> = Vec::new();
    let mut selections: Vec<Value> = Vec::new();
    let mut seen_ids: HashSet<String> = HashSet::new();
    let now = now_ms();

    for conn in &cfg.llm.connections {
        seen_ids.insert(conn.id.clone());
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
        let mut capability_assessments: Vec<Value> = Vec::new();
        for m in &conn.models {
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
                capability_assessments.push(json!({
                    "model_id": m.id,
                    "protocol": assess_proto,
                    "evidence": evidence.unwrap_or(Value::Null),
                    "user_override": user_override.unwrap_or(Value::Null)
                }));
            }
        }
        let mut bindings = serde_json::Map::new();
        if configured {
            bindings.insert(wire_proto.to_string(), json!("vault"));
        } else if is_loopback_url(&conn.endpoint) {
            bindings.insert(wire_proto.to_string(), json!("none"));
        }
        connections.push(json!({
            "revision": 1,
            "hasApiKey": configured,
            "probe_credential_bindings": Value::Object(bindings),
            "document": {
                "format_version": 1,
                "connection": {
                    "id": conn.id,
                    "display_name": conn.id,
                    "base_url": conn.endpoint,
                    "protocols": [wire_proto],
                    "source": conn.provider.as_deref().unwrap_or("config"),
                    "source_id": Value::Null,
                    "account_id": Value::Null,
                    "enabled": true
                },
                "catalog": {
                    "observed_at_ms": now,
                    "models": catalog_models
                },
                "capability_assessments": capability_assessments
            }
        }));
    }

    for rec in suite.list_records::<Value>("model_connection").unwrap_or_default() {
        let id = rec
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if id.is_empty() || seen_ids.contains(&id) {
            continue;
        }
        let base_url = rec
            .get("base_url")
            .or_else(|| rec.get("endpoint"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let proto = rec
            .get("protocol")
            .and_then(|v| v.as_str())
            .unwrap_or("chat_completions");
        let mut rec_bindings = serde_json::Map::new();
        if is_loopback_url(base_url) {
            rec_bindings.insert(proto.to_string(), json!("none"));
        }
        connections.push(json!({
            "revision": 1,
            "hasApiKey": false,
            "probe_credential_bindings": Value::Object(rec_bindings),
            "document": {
                "format_version": 1,
                "connection": {
                    "id": id,
                    "display_name": rec.get("display_name").and_then(|v| v.as_str()).unwrap_or(&id),
                    "base_url": base_url,
                    "protocols": [proto],
                    "source": "custom",
                    "source_id": Value::Null,
                    "account_id": Value::Null,
                    "enabled": rec.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true)
                },
                "catalog": {
                    "observed_at_ms": now,
                    "models": []
                },
                "capability_assessments": []
            }
        }));
    }

    let accounts: Vec<Value> = suite
        .list_records::<Value>("source_account")
        .unwrap_or_default()
        .into_iter()
        .filter_map(|rec| {
            let id = rec.get("id").and_then(|v| v.as_str())?.to_string();
            let rev = rec.get("revision").and_then(|v| v.as_i64()).unwrap_or(1);
            let display_name = rec
                .get("display_name")
                .and_then(|v| v.as_str())
                .unwrap_or(&id);
            let source = rec
                .get("source")
                .and_then(|v| v.as_str())
                .unwrap_or("custom");
            Some(json!({
                "revision": rev,
                "account": {
                    "id": id,
                    "display_name": display_name,
                    "source": source,
                    "state": rec.get("state").and_then(|v| v.as_str()).unwrap_or("connected")
                }
            }))
        })
        .collect();

    json!({
        "connections": connections,
        "accounts": accounts,
        "selections": selections
    })
}

fn same_origin_url(a: &str, b: &str) -> bool {
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

fn is_disallowed_probe_ip(ip: IpAddr) -> bool {
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

fn validate_probe_url_ssrf(raw_url: &str) -> Result<Url, String> {
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

fn run_real_model_connection_probe(store: &Store, body: &Value) -> Value {
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
    // SEC-3: Reject private/link-local/metadata IPs (e.g. 169.254.169.254) unless loopback!
    if let Err(ssrf_err) = validate_probe_url_ssrf(&probe_url) {
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

fn handle_gateway_route(
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

fn resolve_requested_model_selection(
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
    None
}

fn load_messaging_worker_snapshot(store: &Store) -> Value {
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

fn channel_vault_secret_key(conn_id: &str) -> String {
    format!("msg_webhook_secret:{conn_id}")
}

fn persist_channel_secret_and_normalize(store: &Store, raw: &Value) -> Value {
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

fn normalize_messaging_channel(store: &Store, raw: &Value) -> Value {
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

fn resolve_channel_webhook_secret(store: &Store, channel: &Value) -> String {
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

fn parse_mcp_server_config(body: &Value, path_id: Option<&str>) -> McpServerConfig {
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
fn decode_attachment_media(att: &Value) -> Option<MediaAttachment> {
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
    } else if let Some(b64) = att
        .get("data_base64")
        .or_else(|| att.get("base64"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
    {
        let mime = att
            .get("mimeType")
            .or_else(|| att.get("mime_type"))
            .and_then(|v| v.as_str())
            .unwrap_or("image/png")
            .to_string();
        (mime, b64.trim().to_string())
    } else {
        return None;
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

fn collect_conversation_context_blocks(
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
fn handle_sse_chat_turn(
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
    let override_source = body
        .get("selectedModelSource")
        .or_else(|| body.get("modelSource"))
        .or_else(|| body.get("model_source"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .or(intent_source)
        .or_else(|| sess.as_ref().and_then(|s| s.model_connection.clone()));
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
            ChannelReader::with_cancel(rx_room, cancel_for_reader),
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
                target_key,
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
        ChannelReader::with_cancel(rx, cancel_for_reader),
        None,
        None,
    );
    let _ = req.respond(resp);
}

fn parse_attachment_payload(filename: &str, mime_type: &str, bytes: &[u8]) -> Value {
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

fn execute_documents_search(
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

fn parse_git_worktree_porcelain(raw: &str) -> Vec<Value> {
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

fn handle_workspace_worktrees(workspace_root: &Path, is_post: bool, body: &Value) -> (u16, Value) {
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

fn handle_conversation_goal_control(
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
fn stream_conversation_task_events(
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

fn handle_conversation_recall(store: &Store, conv_id: &str, body: &Value) -> Value {
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

fn collect_conversation_artifacts(store: &Store, conv_id: &str) -> Vec<Value> {
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

fn handle_conversation_plan(store: &Store, conv_id: &str, body: &Value) -> (u16, Value) {
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
        .or_else(|| match action.as_deref() {
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

fn handle_auxiliary_dispatch(
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
                        "has_tool_calls": output.get("has_tool_calls").cloned().unwrap_or_else(|| json!(false))
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

fn validate_oauth_origin(raw: &str) -> Result<String, &'static str> {
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

fn handle_source_oauth_route(
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
                    &csrf_state[..4].to_ascii_uppercase(),
                    &csrf_state[4..8].to_ascii_uppercase()
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
                    state: "awaiting_authorization".into(),
                    csrf_state,
                    code_challenge: challenge,
                    authorization_url: Some(verification_url.clone()),
                    verification_url: Some(verification_url.clone()),
                    user_code: Some(user_code.clone()),
                    callback_mode: Some("device_code".into()),
                    instructions: Some("Open verification URL and enter the user code.".into()),
                    poll_interval_seconds: Some(5),
                    cancellation_confirmed: None,
                };
                flows.insert(flow_id.clone(), flow);
                (
                    200,
                    json!({
                        "account": { "revision": rev, "account": { "id": account_id } },
                        "origin": origin,
                        "server_id": server_id,
                        "state": "awaiting_authorization",
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
                let confirmed = if let Some(flow) = flows.get_mut(&fid) {
                    flow.state = "cancelled".into();
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
                        "flow_id": if fid.is_empty() { Value::Null } else { json!(fid) },
                        "verification_url": Value::Null,
                        "user_code": Value::Null,
                        "poll_interval_seconds": Value::Null,
                        "inference_supported": false,
                        "cancellation_confirmed": confirmed
                    }),
                )
            } else {
                let flow_opt = flow_id_in
                    .as_deref()
                    .and_then(|id| flows.get(id))
                    .or_else(|| flows.values().rev().find(|f| f.account_id == account_id));
                match flow_opt {
                    Some(f) => (
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
                    ),
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
                state: "awaiting_authorization".into(),
                csrf_state,
                code_challenge: challenge,
                authorization_url: Some(auth_url.clone()),
                verification_url: None,
                user_code: None,
                callback_mode: Some("code".into()),
                instructions: Some("Complete authorization and paste the returned code.".into()),
                poll_interval_seconds: None,
                cancellation_confirmed: None,
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
                    "state": "awaiting_authorization",
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
                .and_then(|v| v.as_str())
                .map(ToString::to_string)
                .or_else(|| parse_query_str(query, "state"))
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
            if let (Some(tok), Some(ref v)) = (token, vault.as_ref()) {
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
fn dispatch_selected_experts_parallel(
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

fn parse_query_str(query: &str, key: &str) -> Option<String> {
    url::form_urlencoded::parse(query.as_bytes())
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.into_owned())
}

fn parse_query_i64(query: &str, key: &str) -> Option<i64> {
    parse_query_str(query, key).and_then(|v| v.parse().ok())
}

fn respond_json(req: Request, status: u16, value: &Value) -> std::io::Result<()> {
    let data = serde_json::to_vec(value).unwrap_or_else(|_| b"{}".to_vec());
    let header = Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap();
    let resp = Response::from_data(data)
        .with_status_code(StatusCode(status))
        .with_header(header);
    req.respond(resp)
}

fn percent_decode_path(raw: &str) -> String {
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

fn mime_for_path(path: &Path) -> &'static str {
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

fn resolve_static_target<'a>(
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

fn serve_static_asset(req: Request, ui_dir: &Path, static_dir: &Path, path: &str) {
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

// =============================================================================
// Self-contained RFC 4648 Base64/Base64URL + NIST AES-256-GCM (`aes-gcm-v1`)
// =============================================================================

const B64_STD: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64_encode(input: &[u8]) -> String {
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(B64_STD[((triple >> 18) & 0x3f) as usize] as char);
        out.push(B64_STD[((triple >> 12) & 0x3f) as usize] as char);
        if chunk.len() > 1 {
            out.push(B64_STD[((triple >> 6) & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(B64_STD[(triple & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

fn base64url_encode(input: &[u8]) -> String {
    base64_encode(input)
        .replace('+', "-")
        .replace('/', "_")
        .trim_end_matches('=')
        .to_string()
}

fn base64_decode(input: &str) -> Result<Vec<u8>, String> {
    let mut vals: Vec<u8> = Vec::with_capacity(input.len());
    for b in input.bytes() {
        match b {
            b'A'..=b'Z' => vals.push(b - b'A'),
            b'a'..=b'z' => vals.push(b - b'a' + 26),
            b'0'..=b'9' => vals.push(b - b'0' + 52),
            b'+' | b'-' => vals.push(62),
            b'/' | b'_' => vals.push(63),
            b'=' | b' ' | b'\n' | b'\r' | b'\t' => {}
            _ => return Err(format!("invalid base64 byte: 0x{b:02x}")),
        }
    }
    if vals.len() % 4 == 1 {
        return Err("invalid base64 length".into());
    }
    let mut out = Vec::with_capacity((vals.len() * 3) / 4);
    for chunk in vals.chunks(4) {
        match chunk.len() {
            4 => {
                let n = ((chunk[0] as u32) << 18)
                    | ((chunk[1] as u32) << 12)
                    | ((chunk[2] as u32) << 6)
                    | (chunk[3] as u32);
                out.push(((n >> 16) & 0xff) as u8);
                out.push(((n >> 8) & 0xff) as u8);
                out.push((n & 0xff) as u8);
            }
            3 => {
                let n = ((chunk[0] as u32) << 18)
                    | ((chunk[1] as u32) << 12)
                    | ((chunk[2] as u32) << 6);
                out.push(((n >> 16) & 0xff) as u8);
                out.push(((n >> 8) & 0xff) as u8);
            }
            2 => {
                let n = ((chunk[0] as u32) << 18) | ((chunk[1] as u32) << 12);
                out.push(((n >> 16) & 0xff) as u8);
            }
            _ => {}
        }
    }
    Ok(out)
}

const AES_SBOX: [u8; 256] = [
    0x63, 0x7c, 0x77, 0x7b, 0xf2, 0x6b, 0x6f, 0xc5, 0x30, 0x01, 0x67, 0x2b, 0xfe, 0xd7, 0xab, 0x76,
    0xca, 0x82, 0xc9, 0x7d, 0xfa, 0x59, 0x47, 0xf0, 0xad, 0xd4, 0xa2, 0xaf, 0x9c, 0xa4, 0x72, 0xc0,
    0xb7, 0xfd, 0x93, 0x26, 0x36, 0x3f, 0xf7, 0xcc, 0x34, 0xa5, 0xe5, 0xf1, 0x71, 0xd8, 0x31, 0x15,
    0x04, 0xc7, 0x23, 0xc3, 0x18, 0x96, 0x05, 0x9a, 0x07, 0x12, 0x80, 0xe2, 0xeb, 0x27, 0xb2, 0x75,
    0x09, 0x83, 0x2c, 0x1a, 0x1b, 0x6e, 0x5a, 0xa0, 0x52, 0x3b, 0xd6, 0xb3, 0x29, 0xe3, 0x2f, 0x84,
    0x53, 0xd1, 0x00, 0xed, 0x20, 0xfc, 0xb1, 0x5b, 0x6a, 0xcb, 0xbe, 0x39, 0x4a, 0x4c, 0x58, 0xcf,
    0xd0, 0xef, 0xaa, 0xfb, 0x43, 0x4d, 0x33, 0x85, 0x45, 0xf9, 0x02, 0x7f, 0x50, 0x3c, 0x9f, 0xa8,
    0x51, 0xa3, 0x40, 0x8f, 0x92, 0x9d, 0x38, 0xf5, 0xbc, 0xb6, 0xda, 0x21, 0x10, 0xff, 0xf3, 0xd2,
    0xcd, 0x0c, 0x13, 0xec, 0x5f, 0x97, 0x44, 0x17, 0xc4, 0xa7, 0x7e, 0x3d, 0x64, 0x5d, 0x19, 0x73,
    0x60, 0x81, 0x4f, 0xdc, 0x22, 0x2a, 0x90, 0x88, 0x46, 0xee, 0xb8, 0x14, 0xde, 0x5e, 0x0b, 0xdb,
    0xe0, 0x32, 0x3a, 0x0a, 0x49, 0x06, 0x24, 0x5c, 0xc2, 0xd3, 0xac, 0x62, 0x91, 0x95, 0xe4, 0x79,
    0xe7, 0xc8, 0x37, 0x6d, 0x8d, 0xd5, 0x4e, 0xa9, 0x6c, 0x56, 0xf4, 0xea, 0x65, 0x7a, 0xae, 0x08,
    0xba, 0x78, 0x25, 0x2e, 0x1c, 0xa6, 0xb4, 0xc6, 0xe8, 0xdd, 0x74, 0x1f, 0x4b, 0xbd, 0x8b, 0x8a,
    0x70, 0x3e, 0xb5, 0x66, 0x48, 0x03, 0xf6, 0x0e, 0x61, 0x35, 0x57, 0xb9, 0x86, 0xc1, 0x1d, 0x9e,
    0xe1, 0xf8, 0x98, 0x11, 0x69, 0xd9, 0x8e, 0x94, 0x9b, 0x1e, 0x87, 0xe9, 0xce, 0x55, 0x28, 0xdf,
    0x8c, 0xa1, 0x89, 0x0d, 0xbf, 0xe6, 0x42, 0x68, 0x41, 0x99, 0x2d, 0x0f, 0xb0, 0x54, 0xbb, 0x16,
];

fn aes256_expand_key(key: &[u8; 32]) -> [[u8; 16]; 15] {
    let mut w = [0u32; 60];
    for i in 0..8 {
        w[i] = u32::from_be_bytes([key[4 * i], key[4 * i + 1], key[4 * i + 2], key[4 * i + 3]]);
    }
    let rcon: [u32; 8] = [
        0x0100_0000, 0x0200_0000, 0x0400_0000, 0x0800_0000,
        0x1000_0000, 0x2000_0000, 0x4000_0000, 0x8000_0000,
    ];
    let sub_word = |x: u32| -> u32 {
        let b = x.to_be_bytes();
        u32::from_be_bytes([
            AES_SBOX[b[0] as usize],
            AES_SBOX[b[1] as usize],
            AES_SBOX[b[2] as usize],
            AES_SBOX[b[3] as usize],
        ])
    };
    for i in 8..60 {
        let mut temp = w[i - 1];
        if i % 8 == 0 {
            temp = sub_word(temp.rotate_left(8)) ^ rcon[(i / 8) - 1];
        } else if i % 8 == 4 {
            temp = sub_word(temp);
        }
        w[i] = w[i - 8] ^ temp;
    }
    let mut rk = [[0u8; 16]; 15];
    for r in 0..15 {
        for c in 0..4 {
            let bytes = w[r * 4 + c].to_be_bytes();
            rk[r][c * 4..c * 4 + 4].copy_from_slice(&bytes);
        }
    }
    rk
}

fn xtime(x: u8) -> u8 {
    (x << 1) ^ (((x >> 7) & 1) * 0x1b)
}

fn aes256_encrypt_block(rk: &[[u8; 16]; 15], input: &[u8; 16]) -> [u8; 16] {
    let mut s = *input;
    for i in 0..16 {
        s[i] ^= rk[0][i];
    }
    for round in 1..=14 {
        for b in &mut s {
            *b = AES_SBOX[*b as usize];
        }
        // ShiftRows (column-major layout)
        let t = s;
        s[1] = t[5]; s[5] = t[9]; s[9] = t[13]; s[13] = t[1];
        s[2] = t[10]; s[6] = t[14]; s[10] = t[2]; s[14] = t[6];
        s[3] = t[15]; s[7] = t[3]; s[11] = t[7]; s[15] = t[11];

        if round < 14 {
            for c in 0..4 {
                let i = c * 4;
                let (a0, a1, a2, a3) = (s[i], s[i + 1], s[i + 2], s[i + 3]);
                let x = a0 ^ a1 ^ a2 ^ a3;
                s[i] = a0 ^ x ^ xtime(a0 ^ a1);
                s[i + 1] = a1 ^ x ^ xtime(a1 ^ a2);
                s[i + 2] = a2 ^ x ^ xtime(a2 ^ a3);
                s[i + 3] = a3 ^ x ^ xtime(a3 ^ a0);
            }
        }
        for i in 0..16 {
            s[i] ^= rk[round][i];
        }
    }
    s
}

fn ghash_mul(x: u128, y: u128) -> u128 {
    let mut z = 0u128;
    let mut v = y;
    for i in 0..128 {
        if ((x >> (127 - i)) & 1) != 0 {
            z ^= v;
        }
        let lsb = v & 1;
        v >>= 1;
        if lsb != 0 {
            v ^= 0xe100_0000_0000_0000_0000_0000_0000_0000u128;
        }
    }
    z
}

fn ghash(h: u128, aad: &[u8], ct: &[u8]) -> [u8; 16] {
    let mut y = 0u128;
    let mut feed = |data: &[u8]| {
        for chunk in data.chunks(16) {
            let mut blk = [0u8; 16];
            blk[..chunk.len()].copy_from_slice(chunk);
            y ^= u128::from_be_bytes(blk);
            y = ghash_mul(y, h);
        }
    };
    feed(aad);
    feed(ct);
    let mut len_blk = [0u8; 16];
    len_blk[..8].copy_from_slice(&((aad.len() as u64) * 8).to_be_bytes());
    len_blk[8..].copy_from_slice(&((ct.len() as u64) * 8).to_be_bytes());
    y ^= u128::from_be_bytes(len_blk);
    y = ghash_mul(y, h);
    y.to_be_bytes()
}

fn aes256_gcm_encrypt(
    key: &[u8; 32],
    nonce: &[u8; 12],
    aad: &[u8],
    plaintext: &[u8],
) -> (Vec<u8>, [u8; 16]) {
    let rk = aes256_expand_key(key);
    let h = u128::from_be_bytes(aes256_encrypt_block(&rk, &[0u8; 16]));
    let mut j0 = [0u8; 16];
    j0[..12].copy_from_slice(nonce);
    j0[15] = 1;

    let mut ct = Vec::with_capacity(plaintext.len());
    let mut counter = 2u32;
    for chunk in plaintext.chunks(16) {
        let mut cb = [0u8; 16];
        cb[..12].copy_from_slice(nonce);
        cb[12..16].copy_from_slice(&counter.to_be_bytes());
        let ks = aes256_encrypt_block(&rk, &cb);
        for (i, &b) in chunk.iter().enumerate() {
            ct.push(b ^ ks[i]);
        }
        counter = counter.wrapping_add(1);
    }

    let s = ghash(h, aad, &ct);
    let e_j0 = aes256_encrypt_block(&rk, &j0);
    let mut tag = [0u8; 16];
    for i in 0..16 {
        tag[i] = s[i] ^ e_j0[i];
    }
    (ct, tag)
}

fn aes256_gcm_decrypt(
    key: &[u8; 32],
    nonce: &[u8; 12],
    aad: &[u8],
    ciphertext: &[u8],
    expected_tag: &[u8; 16],
) -> Result<Vec<u8>, String> {
    let rk = aes256_expand_key(key);
    let h = u128::from_be_bytes(aes256_encrypt_block(&rk, &[0u8; 16]));
    let mut j0 = [0u8; 16];
    j0[..12].copy_from_slice(nonce);
    j0[15] = 1;

    let s = ghash(h, aad, ciphertext);
    let e_j0 = aes256_encrypt_block(&rk, &j0);
    let mut diff = 0u8;
    for i in 0..16 {
        diff |= (s[i] ^ e_j0[i]) ^ expected_tag[i];
    }
    if diff != 0 {
        return Err("aes-gcm tag verification failed".into());
    }

    let mut pt = Vec::with_capacity(ciphertext.len());
    let mut counter = 2u32;
    for chunk in ciphertext.chunks(16) {
        let mut cb = [0u8; 16];
        cb[..12].copy_from_slice(nonce);
        cb[12..16].copy_from_slice(&counter.to_be_bytes());
        let ks = aes256_encrypt_block(&rk, &cb);
        for (i, &b) in chunk.iter().enumerate() {
            pt.push(b ^ ks[i]);
        }
        counter = counter.wrapping_add(1);
    }
    Ok(pt)
}

fn derive_mobile_crypto_key(token: &str) -> [u8; 32] {
    let digest = Sha256::digest(token.as_bytes());
    let mut key = [0u8; 32];
    key.copy_from_slice(&digest);
    key
}

fn encrypt_mobile_json_envelope(value: &Value, token: &str) -> Result<Value, String> {
    let plaintext = serde_json::to_vec(value).map_err(|e| e.to_string())?;
    let key = derive_mobile_crypto_key(token);
    let rand32 = random_bytes_32();
    let mut nonce = [0u8; 12];
    nonce.copy_from_slice(&rand32[..12]);
    let (ct, tag) = aes256_gcm_encrypt(&key, &nonce, MOBILE_CRYPTO_AAD, &plaintext);
    Ok(json!({
        "v": 1,
        "alg": "AES-256-GCM",
        "nonce": base64url_encode(&nonce),
        "ciphertext": base64url_encode(&ct),
        "tag": base64url_encode(&tag)
    }))
}

fn decrypt_mobile_json_envelope(envelope: &Value, token: &str) -> Result<Value, String> {
    let nonce_bytes = base64_decode(
        envelope
            .get("nonce")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing nonce".to_string())?,
    )?;
    if nonce_bytes.len() != 12 {
        return Err("invalid aes-gcm nonce length".into());
    }
    let ct = base64_decode(
        envelope
            .get("ciphertext")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing ciphertext".to_string())?,
    )?;
    let tag_bytes = base64_decode(
        envelope
            .get("tag")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing tag".to_string())?,
    )?;
    if tag_bytes.len() != 16 {
        return Err("invalid aes-gcm tag length".into());
    }
    let mut nonce = [0u8; 12];
    nonce.copy_from_slice(&nonce_bytes);
    let mut tag = [0u8; 16];
    tag.copy_from_slice(&tag_bytes);
    let key = derive_mobile_crypto_key(token);
    let pt = aes256_gcm_decrypt(&key, &nonce, MOBILE_CRYPTO_AAD, &ct, &tag)?;
    serde_json::from_slice(&pt).map_err(|e| format!("invalid decrypted JSON: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_nonce_cache_evicts_oldest_in_o1_and_blocks_replays() {
        let mut cache = BoundedNonceCache::default();
        assert!(cache.record_if_fresh("n1".into()));
        assert!(!cache.record_if_fresh("n1".into()));
        for i in 0..MAX_TRACKED_NONCES {
            assert!(cache.record_if_fresh(format!("fill_{i}")));
        }
        assert!(cache.record_if_fresh("n1".into()));
    }

    #[test]
    fn channel_reader_streams_chunks_incrementally_and_terminates_on_sender_drop() {
        let (tx, rx) = mpsc::channel();
        tx.send(b"data: 1\n\n".to_vec()).unwrap();
        tx.send(b"data: [DONE]\n\n".to_vec()).unwrap();
        drop(tx);
        let mut reader = ChannelReader::new(rx);
        let mut out = String::new();
        reader.read_to_string(&mut out).unwrap();
        assert_eq!(out, "data: 1\n\ndata: [DONE]\n\n");
    }

    #[test]
    fn strip_jsonc_handles_comments_urls_and_trailing_commas() {
        let raw = r#"{
            // comment
            "endpoint": "https://api.example.com/v1", /* block comment */
            "items": [1, 2,],
        }"#;
        let clean = crate::config::strip_jsonc(raw);
        let v: Value = serde_json::from_str(&clean).unwrap();
        assert_eq!(v["endpoint"], "https://api.example.com/v1");
        assert_eq!(v["items"], json!([1, 2]));
    }

    #[test]
    fn server_started_handshake_matches_desktop_and_mac_contracts() {
        let frame = server_started_handshake("127.0.0.1", 28421);
        assert_eq!(frame["type"], "server.started");
        assert_eq!(frame["protocol_version"], 1);
        assert_eq!(frame["loopback"], true);
        assert_eq!(frame["origin"], "http://127.0.0.1:28421");
        assert_eq!(frame["client_api"], "http://127.0.0.1:28421/api/mobile");
        assert_eq!(frame["host_id"], "nola-host-0000000000000000");
        assert_eq!(frame["access_token"], "test-token");
        assert_eq!(frame["credentials_path"], "/tmp/mobile-access-token");
        assert_eq!(frame["clientAPI"]["path"], "/api/mobile");

        let lan = server_started_handshake("0.0.0.0", 5001);
        assert_eq!(lan["loopback"], false);
    }

    #[test]
    fn model_connections_inventory_respects_selections_and_capability_overrides() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory().unwrap();
        crate::config::bind_store_vault_path(&store, &dir.path().join("vault.json")).unwrap();
        let suite = ModulesSuite::new(store.clone(), dir.path().to_path_buf(), None);
        let cfg = crate::config::AppliedConfig {
            llm: crate::config::LlmSection {
                default_model: Some(DefaultModelSelection {
                    connection: "ep1".into(),
                    model: "m1".into(),
                }),
                connections: vec![ConnectionConfig {
                    id: "ep1".into(),
                    endpoint: "https://example.invalid/v1".into(),
                    protocol: Some("chat_completions".into()),
                    provider: None,
                    api_key: Some("sk-secret".into()),
                    api_key_env: None,
                    models: vec![
                        ModelEntry {
                            id: "m1".into(),
                            context_window: Some(128_000),
                            max_output_tokens: None,
                            temperature: None,
                            top_p: None,
                            reasoning_effort: None,
                            capabilities: None,
                        },
                        ModelEntry {
                            id: "m2".into(),
                            context_window: Some(64_000),
                            max_output_tokens: None,
                            temperature: None,
                            top_p: None,
                            reasoning_effort: None,
                            capabilities: None,
                        },
                    ],
                }],
            },
            ..Default::default()
        };
        save_applied_config(&store, &cfg).unwrap();
        store
            .set_kv(
                "model_conn_selections:ep1",
                &json!({"enabled": ["chat_completions:m1"]}),
            )
            .unwrap();
        store
            .set_kv(
                "model_cap_override:ep1:m1",
                &json!({
                    "model_id": "m1",
                    "protocol": "chat_completions",
                    "user_override": {
                        "toolCalls": true,
                        "imageInput": true,
                        "reasoning": false,
                        "samplingParameters": true,
                        "samplingRequiresNoReasoning": false,
                        "chatOutputTokenField": "auto"
                    }
                }),
            )
            .unwrap();

        let inv = build_model_connections_inventory(&store, &suite);
        let sels = inv["selections"].as_array().unwrap();
        assert_eq!(sels[0]["enabled"], true);
        assert_eq!(sels[1]["enabled"], false);
        let assessments = inv["connections"][0]["document"]["capability_assessments"]
            .as_array()
            .unwrap();
        assert_eq!(assessments.len(), 1);
        assert_eq!(assessments[0]["model_id"], "m1");
        assert_eq!(assessments[0]["user_override"]["imageInput"], true);

        let cred = build_endpoint_credential_response(&store, "ep1").unwrap();
        assert_eq!(cred["endpointID"], "ep1");
        assert_eq!(cred["hasCredential"], true);
        assert_eq!(cred["credentialRevision"], 1);
        assert!(cred["expectedRevisions"].as_object().unwrap().len() >= 1);
    }

    #[test]
    fn static_target_resolves_ui_static_and_spa_routes() {
        let ui = Path::new("/web/app/static/ui");
        let st = Path::new("/web/app/static");
        assert_eq!(
            resolve_static_target(ui, st, "/ui/assets/app.js"),
            (ui, "assets/app.js".into(), false)
        );
        assert_eq!(
            resolve_static_target(ui, st, "/ui/chat"),
            (ui, "chat".into(), true)
        );
        assert_eq!(
            resolve_static_target(ui, st, "/static/favicon.ico"),
            (st, "favicon.ico".into(), false)
        );
        assert_eq!(
            resolve_static_target(ui, st, "/"),
            (ui, "index.html".into(), false)
        );
    }

    #[test]
    fn aes256_gcm_matches_nist_vectors_and_mobile_envelope_roundtrips() {
        // NIST SP 800-38D Test Case 13 (AES-256-GCM, empty PT, empty AAD)
        let key = [0u8; 32];
        let nonce = [0u8; 12];
        let (ct0, tag0) = aes256_gcm_encrypt(&key, &nonce, b"", b"");
        assert!(ct0.is_empty());
        assert_eq!(hex::encode(tag0), "530f8afbc74536b9a963b4f1c4cb738b");

        // NIST SP 800-38D Test Case 14 (AES-256-GCM, 16-byte zero PT, empty AAD)
        let (ct1, tag1) = aes256_gcm_encrypt(&key, &nonce, b"", &[0u8; 16]);
        assert_eq!(hex::encode(&ct1), "cea7403d4d606b6e074ec5d3baf39d18");
        assert_eq!(hex::encode(tag1), "d0d1c8a799996bf0265b98b5d48ab919");
        let pt1 = aes256_gcm_decrypt(&key, &nonce, b"", &ct1, &tag1).unwrap();
        assert_eq!(pt1, vec![0u8; 16]);

        // Mobile JSON envelope roundtrip + tamper detection
        let payload = json!({"target": "/settings#channels", "count": 42});
        let env = encrypt_mobile_json_envelope(&payload, "test-mobile-token").unwrap();
        let dec = decrypt_mobile_json_envelope(&env, "test-mobile-token").unwrap();
        assert_eq!(dec, payload);
        assert!(decrypt_mobile_json_envelope(&env, "wrong-token").is_err());
    }

    #[test]
    fn data_url_image_attachment_decodes_to_media_attachment() {
        let png_stub = b"\x89PNG\r\n\x1a\nhello";
        let data_url = format!("data:image/png;base64,{}", base64_encode(png_stub));
        let att = json!({
            "name": "shot.png",
            "dataUrl": data_url
        });
        let media = decode_attachment_media(&att).unwrap();
        assert_eq!(media.mime_type, "image/png");
        assert_eq!(media.filename, "shot.png");
        assert_eq!(media.raw_bytes_len, png_stub.len());
        assert_eq!(media.sha256, sha256_hex(png_stub));
    }

    #[test]
    fn session_scoped_permission_grant_is_narrowly_scoped() {
        let pending = PendingInteractions::default();
        let (tx, _rx) = mpsc::channel();
        pending.approvals.lock().unwrap().insert(
            "req-1".into(),
            PendingApprovalEntry {
                conversation_id: "conv-a".into(),
                tool_name: "write_file".into(),
                target_key: "path:src/main.rs".into(),
                reason: "confirm write".into(),
                arguments: json!({"path": "src/main.rs"}),
                tx,
            },
        );
        assert!(pending.conversation_needs_response("conv-a"));
        assert!(!pending.conversation_needs_response("conv-b"));
        assert!(pending.resolve_approval("req-1", true, "session"));
        assert!(pending.has_session_grant("conv-a", "write_file", "path:src/main.rs"));
        assert!(!pending.has_session_grant("conv-a", "write_file", "path:src/other.rs"));
        assert!(!pending.has_session_grant("conv-b", "write_file", "path:src/main.rs"));
        pending.clear_conversation("conv-a");
        assert!(!pending.has_session_grant("conv-a", "write_file", "path:src/main.rs"));
    }

    #[test]
    fn sec3_ssrf_and_origin_guard_block_metadata_and_cross_origin_keys() {
        assert!(validate_probe_url_ssrf("http://169.254.169.254/latest/meta-data/").is_err());
        assert!(validate_probe_url_ssrf("http://10.0.0.1:8080/v1/models").is_err());
        assert!(validate_probe_url_ssrf("http://192.168.1.5/v1/models").is_err());
        assert!(validate_probe_url_ssrf("http://metadata.google.internal/computeMetadata/v1").is_err());
        assert!(validate_probe_url_ssrf("http://127.0.0.1:11434/v1/models").is_ok());
        assert!(validate_probe_url_ssrf("http://localhost:8080/v1/models").is_ok());
        assert!(validate_probe_url_ssrf("http://[::1]:8080/v1/models").is_ok());

        assert!(same_origin_url("https://api.example.com/v1", "https://api.example.com:443/v2"));
        assert!(!same_origin_url("https://api.example.com/v1", "https://evil.example.com/v1"));
        assert!(!same_origin_url("http://127.0.0.1:8080/v1", "http://127.0.0.1:9090/v1"));

        let store = Store::open_in_memory().unwrap();
        let res = run_real_model_connection_probe(
            &store,
            &json!({"endpoint": "http://169.254.169.254/latest"}),
        );
        assert_eq!(res["ok"], false);
        assert!(res["error"].as_str().unwrap_or("").contains("blocked"));
    }

    #[test]
    fn sec7_sensitive_paths_redaction_module_allowlist_and_channel_vault() {
        assert!(is_sensitive_local_path(Path::new("/home/alice/.ssh/id_rsa")));
        assert!(is_sensitive_local_path(Path::new("/home/alice/.nola/vault.json.key")));
        assert!(is_sensitive_local_path(Path::new("/workspace/.env")));
        assert!(is_sensitive_local_path(Path::new("/workspace/cert.pem")));
        assert!(!is_sensitive_local_path(Path::new("/workspace/README.md")));

        let redacted = redact_secrets_in_text(
            "API_KEY=supersecret\nAuthorization: Bearer tok12345\nkey sk-1234567890abcdef ok",
        );
        assert!(!redacted.contains("supersecret"));
        assert!(!redacted.contains("tok12345"));
        assert!(!redacted.contains("sk-1234567890abcdef"));

        assert!(is_allowed_module_kind("goal"));
        assert!(is_allowed_module_kind("automation"));
        assert!(is_allowed_module_kind("data_source"));
        assert!(!is_allowed_module_kind("messaging_channel"));
        assert!(!is_allowed_module_kind("expert_peer"));
        assert!(!is_allowed_module_kind("gateway_attempt"));

        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory().unwrap();
        crate::config::bind_store_vault_path(&store, &dir.path().join("vault.json")).unwrap();
        let norm = persist_channel_secret_and_normalize(
            &store,
            &json!({
                "connection_id": "tg-1",
                "kind": "telegram",
                "webhook_secret": "whsec-very-secret",
                "webhook_secret_ref": "PATH",
                "enabled": true
            }),
        );
        assert!(norm.get("webhook_secret").is_none());
        assert_eq!(norm["has_webhook_secret"], true);
        assert_eq!(resolve_channel_webhook_secret(&store, &norm), "whsec-very-secret");

        // Non-NOLA_WEBHOOK_ env var must NEVER be read as webhook secret!
        let ref_only = json!({
            "connection_id": "tg-env",
            "kind": "telegram",
            "webhook_secret_ref": "PATH"
        });
        assert_ne!(
            resolve_channel_webhook_secret(&store, &ref_only),
            std::env::var("PATH").unwrap_or_default()
        );
    }

    #[test]
    fn mobile_crypto_fails_closed_and_url_matches_host_handles_default_ports() {
        assert!(maybe_decrypt_mobile_body(br#"{"plaintext":true}"#, "tok").is_err());
        let env = encrypt_mobile_json_envelope(&json!({"hello": "world"}), "tok").unwrap();
        let env_bytes = serde_json::to_vec(&env).unwrap();
        assert_eq!(
            maybe_decrypt_mobile_body(&env_bytes, "tok").unwrap(),
            json!({"hello": "world"})
        );

        let sse = encode_sse_frame(&json!({"type": "delta", "text": "hi"}), Some("tok"));
        let sse_str = String::from_utf8(sse).unwrap();
        let json_part = sse_str.trim_start_matches("data: ").trim();
        let parsed: Value = serde_json::from_str(json_part).unwrap();
        assert_eq!(parsed["type"], "encrypted");
        let dec = decrypt_mobile_json_envelope(&parsed, "tok").unwrap();
        assert_eq!(dec["text"], "hi");

        assert_eq!(url_matches_host("https://example.com:443", "example.com"), Ok(true));
        assert_eq!(url_matches_host("http://127.0.0.1:80", "127.0.0.1"), Ok(true));
        assert_eq!(url_matches_host("http://127.0.0.1:5001", "127.0.0.1:5001"), Ok(true));
        assert_eq!(url_matches_host("http://127.0.0.1:5002", "127.0.0.1:5001"), Ok(false));
        assert_eq!(url_matches_host("http://[::1]:5001", "[::1]:5001"), Ok(true));
    }

    #[test]
    fn channel_reader_drop_cancels_on_disconnect_only() {
        // Early drop sets cancel = true
        let (_tx, rx) = mpsc::channel::<Vec<u8>>();
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let _reader = ChannelReader::with_cancel(rx, Arc::clone(&cancel));
        }
        assert!(cancel.load(Ordering::SeqCst));

        // Draining to EOF keeps cancel = false
        let (tx2, rx2) = mpsc::channel::<Vec<u8>>();
        tx2.send(b"done".to_vec()).unwrap();
        drop(tx2);
        let cancel2 = Arc::new(AtomicBool::new(false));
        {
            let mut reader = ChannelReader::with_cancel(rx2, Arc::clone(&cancel2));
            let mut out = String::new();
            reader.read_to_string(&mut out).unwrap();
            assert_eq!(out, "done");
        }
        assert!(!cancel2.load(Ordering::SeqCst));
    }

    #[test]
    fn model_selection_handles_uuid_and_colons_and_mcp_array_command_and_loopback_binding() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory().unwrap();
        crate::config::bind_store_vault_path(&store, &dir.path().join("vault.json")).unwrap();
        let suite = ModulesSuite::new(store.clone(), dir.path().to_path_buf(), None);
        let cfg = crate::config::AppliedConfig {
            llm: crate::config::LlmSection {
                default_model: None,
                connections: vec![ConnectionConfig {
                    id: "ollama".into(),
                    endpoint: "http://127.0.0.1:11434/v1".into(),
                    protocol: Some("chat_completions".into()),
                    provider: None,
                    api_key: None,
                    api_key_env: None,
                    models: vec![ModelEntry {
                        id: "llama3:8b".into(),
                        context_window: Some(8192),
                        max_output_tokens: None,
                        temperature: None,
                        top_p: None,
                        reasoning_effort: None,
                        capabilities: None,
                    }],
                }],
            },
            ..Default::default()
        };
        save_applied_config(&store, &cfg).unwrap();
        store
            .set_kv(
                "web_model_profiles",
                &json!([{
                    "id": "550e8400-e29b-41d4-a716-446655440000",
                    "modelName": "llama3:8b",
                    "endpoint": { "id": "ollama" }
                }]),
            )
            .unwrap();

        // UUID lookup resolves to (ollama, llama3:8b)
        assert_eq!(
            resolve_requested_model_selection(&store, &cfg, "550e8400-e29b-41d4-a716-446655440000"),
            Some(("ollama".into(), "llama3:8b".into()))
        );
        // `llama3:8b` is NOT falsely split on ':' because `llama3` is not a connection ID!
        assert_eq!(
            resolve_requested_model_selection(&store, &cfg, "llama3:8b"),
            Some(("ollama".into(), "llama3:8b".into()))
        );
        // `ollama:llama3:8b` splits cleanly on real connection ID!
        assert_eq!(
            resolve_requested_model_selection(&store, &cfg, "ollama:llama3:8b"),
            Some(("ollama".into(), "llama3:8b".into()))
        );

        // Keyless loopback endpoint emits `"none"` credential binding
        let inv = build_model_connections_inventory(&store, &suite);
        assert_eq!(
            inv["connections"][0]["probe_credential_bindings"]["chat_completions"],
            "none"
        );

        // Array-form `command` in MCP config
        let mcp = parse_mcp_server_config(
            &json!({
                "name": "fs-mcp",
                "command": ["npx", "-y", "@modelcontextprotocol/server-filesystem"],
                "args": ["/tmp"]
            }),
            None,
        );
        assert_eq!(mcp.command.as_deref(), Some("npx"));
        assert_eq!(
            mcp.args,
            vec!["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]
        );

        // Context blocks wire custom_system_prompt, goal, knowledge, and data sources (defaultEnabled + enabled, excluding blocked)
        store
            .set_kv("conv_goal:c1", &json!({"title": "Ship v2", "description": "Zero warnings"}))
            .unwrap();
        store
            .set_kv(
                &conversation_knowledge_key("c1"),
                &json!({"keyPointsMarkdown": "Use Rust", "summary": "Use Rust"}),
            )
            .unwrap();
        suite
            .upsert_record(
                "data_source",
                "ds-def",
                &json!({"id": "ds-def", "title": "Default Docs", "kind": "localFile", "location": "/tmp/a.md", "accessPolicy": "defaultEnabled"}),
            )
            .unwrap();
        suite
            .upsert_record(
                "data_source",
                "ds-blk",
                &json!({"id": "ds-blk", "title": "Blocked Secret", "kind": "localFile", "location": "/tmp/b.md", "accessPolicy": "blocked"}),
            )
            .unwrap();
        let blocks = collect_conversation_context_blocks(
            &store,
            &suite,
            "c1",
            &json!({"custom_system_prompt": "Always be concise.", "enable_local_dev_tools": false}),
        );
        let joined: String = blocks.iter().map(|b| b.text.as_str()).collect::<Vec<_>>().join("\n");
        assert!(joined.contains("Always be concise."));
        assert!(joined.contains("Local development tools"));
        assert!(joined.contains("Ship v2"));
        assert!(joined.contains("Use Rust"));
        assert!(joined.contains("Default Docs"));
        assert!(!joined.contains("Blocked Secret"));
    }

    #[test]
    fn data_source_probe_and_normalize_produces_valid_contract_for_created_and_listed_records() {
        let dir = tempfile::tempdir().unwrap();
        let sample_file = dir.path().join("notes.md");
        fs::write(&sample_file, "# Release Notes\nAll checks green.\n").unwrap();
        let store = Store::open_in_memory().unwrap();
        let suite = ModulesSuite::new(store.clone(), dir.path().to_path_buf(), None);

        // Simulate POST /api/data-sources with minimal fields (no accessPolicy supplied)
        let mut created = normalize_data_source_record(&json!({
            "id": "ds-valid-1",
            "name": "Release Notes",
            "kind": "localFile",
            "location": sample_file.display().to_string()
        }));
        let (ok, _probe) = probe_and_attach_data_source_validation(&suite, "ds-valid-1", &mut created);
        assert!(ok);
        let created = normalize_data_source_record(&created);
        suite.upsert_record("data_source", "ds-valid-1", &created).unwrap();

        // Simulate a raw legacy record in SQLite without accessPolicy or name
        suite
            .upsert_record(
                "data_source",
                "ds-legacy-2",
                &json!({"id": "ds-legacy-2", "title": "Legacy", "path": "/nonexistent.txt"}),
            )
            .unwrap();

        // Simulate GET /api/data-sources projection
        let listed: Vec<Value> = suite
            .list_records::<Value>("data_source")
            .unwrap()
            .iter()
            .map(normalize_data_source_record)
            .collect();
        assert_eq!(listed.len(), 2);
        for ds in &listed {
            for key in ["id", "name", "kind", "location", "accessPolicy"] {
                assert!(
                    ds.get(key).and_then(|v| v.as_str()).is_some(),
                    "listed data source missing string {key}: {ds}"
                );
            }
            if let Some(vc) = ds.get("validationContract").and_then(|v| v.as_object()) {
                assert_eq!(vc["sourceId"], ds["id"]);
                for ckey in ["readerDescription", "redactedSample", "validationState", "validatedAt"] {
                    assert!(vc.get(ckey).and_then(|v| v.as_str()).is_some());
                }
                assert!(vc["schemaFields"].as_array().unwrap().iter().all(|v| v.is_string()));
                assert!(vc["recordCount"].as_u64().is_some());
                assert!(vc["maximumAgeMinutes"].as_i64().unwrap() >= 1);
            }
        }
        let valid_ds = listed.iter().find(|d| d["id"] == "ds-valid-1").unwrap();
        assert_eq!(valid_ds["validationContract"]["validationState"], "verified");
        assert!(valid_ds.get("lastError").is_none());
    }

    #[test]
    fn paired_read_post_classification_and_epoch_wrapping() {
        let store = Store::open_in_memory().unwrap();
        let epoch1 = ensure_source_store_epoch(&store);
        let epoch2 = ensure_source_store_epoch(&store);
        assert!(!epoch1.is_empty());
        assert_eq!(epoch1, epoch2);

        // Normal Web POST /api/chat/conversations with {} or {title} MUST create
        assert!(!is_conversations_list_read_post(&json!({})));
        assert!(!is_conversations_list_read_post(&json!({"title": "Hello"})));
        assert!(!is_conversations_list_read_post(&json!({"expected_host_id": "h1", "title": "Named"})));
        assert!(!is_conversations_list_read_post(&json!({"expected_host_id": "h1", "operation": "create"})));

        // Paired read or query POST /api/chat/conversations MUST NOT create
        assert!(is_conversations_list_read_post(&json!({"expected_host_id": "h1"})));
        assert!(is_conversations_list_read_post(&json!({"expected_host_id": "h1", "limit": 20})));
        assert!(is_conversations_list_read_post(&json!({"operation": "list"})));
        assert!(is_conversations_list_read_post(&json!({"journal": true})));
        assert!(is_conversations_list_read_post(&json!({"after_seq": 10})));

        // POST /api/chat/conversations/:id/messages read vs turn
        assert!(is_messages_read_post(&json!({"expected_host_id": "h1"})));
        assert!(is_messages_read_post(&json!({"expected_host_id": "h1", "content": ""})));
        assert!(is_messages_read_post(&json!({"operation": "read"})));
        assert!(!is_messages_read_post(&json!({"content": "hi"})));
        assert!(!is_messages_read_post(&json!({"expected_host_id": "h1", "content": "hi"})));
        assert!(!is_messages_read_post(&json!({"parts": [{"type": "text", "text": "hi"}]})));

        // wrap_paired_result includes source_host_id and source_store_epoch
        let wrapped = wrap_paired_result(
            &store,
            "host-abc",
            &json!({"expected_host_id": "host-abc"}),
            json!({"ok": true}),
        );
        assert_eq!(wrapped["source_host_id"], "host-abc");
        assert_eq!(wrapped["source_store_epoch"], epoch1);
        assert_eq!(wrapped["result"]["ok"], true);

        let plain = wrap_paired_result(&store, "host-abc", &json!({}), json!({"ok": true}));
        assert_eq!(plain, json!({"ok": true}));
    }

    #[test]
    fn memory_revise_deduplication_and_resources_and_worker_default() {
        let store = Store::open_in_memory().unwrap();
        let old_item = MemoryItem {
            id: "mem-old".into(),
            namespace: "project:preference".into(),
            title: "Old".into(),
            content: "Old text".into(),
            tags: vec![],
            created_at_ms: 100,
            updated_at_ms: 100,
        };
        let new_item = MemoryItem {
            id: "mem-new".into(),
            namespace: "project:preference".into(),
            title: "Revised".into(),
            content: "Revised text".into(),
            tags: vec![],
            created_at_ms: 200,
            updated_at_ms: 200,
        };
        store.upsert_memory(&old_item).unwrap();
        store.upsert_memory(&new_item).unwrap();
        store
            .set_kv(&memory_meta_key("mem-new"), &json!({"supersedes": ["mem-old"]}))
            .unwrap();

        let raw = store.search_memory("", None, 100).unwrap();
        assert_eq!(raw.len(), 2);
        let (records, items) = filter_superseded_memories(&store, raw);
        assert_eq!(records.len(), 1);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, "mem-new");

        // Resources collection on empty conversation returns empty vec (never error)
        assert!(collect_conversation_resources(&store, "c-empty").is_empty());
        store
            .append_event(
                "c-res",
                "user_message",
                json!({"content": "see image", "attachments": [{"id": "att-1", "name": "pic.png"}]}),
                100,
            )
            .unwrap();
        let resources = collect_conversation_resources(&store, "c-res");
        assert_eq!(resources.len(), 1);
        assert_eq!(resources[0]["id"], "att-1");

        // Default messaging worker execution_backend is "nola" (not duplicate "nola-v2")
        let worker = load_messaging_worker_snapshot(&store);
        assert_eq!(worker["execution_backend"], "nola");
    }

    #[test]
    fn siwc_responses_and_gemini_protocol_mapping_preserved() {
        let map_proto = |raw: &str| match raw {
            "anthropic" => "anthropic",
            "siwc_responses" => "siwc_responses",
            "responses" | "openai_responses" => "openai_responses",
            "gemini" | "google_gemini" => "gemini",
            _ => "openai_chat",
        };
        assert_eq!(map_proto("siwc_responses"), "siwc_responses");
        assert_eq!(map_proto("gemini"), "gemini");
        assert_eq!(map_proto("google_gemini"), "gemini");
        assert_eq!(map_proto("responses"), "openai_responses");
    }

    #[test]
    fn tailcat_verified_command_and_lease_grant_renew_close_lifecycle() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().canonicalize().unwrap().join("fake-tailcat.sh");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::write(
                &script,
                "#!/bin/sh\nread _init\nprintf '{\"protocolVersion\":1,\"event\":\"ready\",\"address\":\"tailcat://100.64.0.1:443\"}\\n'\nread _stop\nexit 0\n",
            )
            .unwrap();
            let mut perms = fs::metadata(&script).unwrap().permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&script, perms).unwrap();
        }
        #[cfg(not(unix))]
        fs::write(&script, b"fixture").unwrap();

        let digest = sha256_hex(&fs::read(&script).unwrap());
        // Relative & parent-dir paths are rejected
        assert!(verified_tailcat_command(Path::new("relative/helper"), Some(&digest)).is_err());
        assert!(verified_tailcat_command(Path::new("/tmp/../bin/helper"), Some(&digest)).is_err());
        // Wrong SHA-256 is refused
        assert!(verified_tailcat_command(&script, Some(&"0".repeat(64))).is_err());
        // Valid binary produces scrubbed env with zero argv leaks
        let cmd = verified_tailcat_command(&script, Some(&digest)).unwrap();
        assert_eq!(cmd.get_args().count(), 0);
        assert!(cmd
            .get_envs()
            .all(|(k, _)| matches!(k.to_str(), Some("PATH" | "SystemRoot" | "TEMP" | "TMP"))));

        #[cfg(unix)]
        {
            let mgr = Arc::new(TailcatLeaseManager::default());
            let cfg = TailcatConfig {
                ingress: true,
                helper_path: Some(script.clone()),
                helper_sha256: Some(digest),
                web_port: 18080,
            };
            assert!(cfg.is_available());

            // Invalid public key rejected
            let bad_key = mgr
                .create_lease(&cfg, "host-1", "owner-a".into(), "not-a-nodekey")
                .unwrap_err();
            assert_eq!(bad_key.status, 400);

            // Grant / connect succeeds and returns real leaseId + address
            let granted = mgr
                .create_lease(&cfg, "host-1", "owner-a".into(), "nodekey:deadbeef")
                .unwrap();
            let lease_id = granted["leaseId"].as_str().unwrap().to_string();
            assert_eq!(granted["address"], "tailcat://100.64.0.1:443");
            assert_eq!(granted["hostId"], "host-1");
            assert_eq!(granted["expiresIn"], TAILCAT_GRANT_SECONDS);

            // Wrong owner cannot renew or close
            assert_eq!(
                mgr.renew_lease("owner-b", &lease_id).unwrap_err().status,
                403
            );
            assert_eq!(
                mgr.close_lease("owner-b", &lease_id).unwrap_err().status,
                403
            );

            // Original owner can renew and cleanly close
            let renewed = mgr.renew_lease("owner-a", &lease_id).unwrap();
            assert_eq!(renewed["renewed"], true);
            assert_eq!(renewed["leaseId"], lease_id);

            let closed = mgr.close_lease("owner-a", &lease_id).unwrap();
            assert_eq!(closed["closed"], true);
            assert_eq!(closed["runStateChanged"], false);

            // Closed lease is no longer available
            assert_eq!(
                mgr.renew_lease("owner-a", &lease_id).unwrap_err().status,
                404
            );
        }
    }

    #[test]
    fn mobile_to_web_one_time_handoff_consumes_atomically_and_enforces_ttl_and_safe_paths() {
        let handoffs: Mutex<HashMap<String, HandoffGrant>> = Mutex::new(HashMap::new());
        let t0 = Instant::now();

        // Unsafe targets are sanitized to /settings
        let (code_unsafe, target_unsafe) =
            mint_web_handoff(&handoffs, "https://evil.example/phish", t0);
        assert_eq!(target_unsafe, "/settings");
        assert_eq!(
            consume_web_handoff(&handoffs, &code_unsafe, t0 + Duration::from_secs(1)),
            Some("/settings".to_string())
        );

        // Safe target is preserved and strictly single-use
        let (code, target) = mint_web_handoff(&handoffs, "/settings?section=messaging", t0);
        assert_eq!(target, "/settings?section=messaging");
        assert!(code.len() >= 32);
        assert_eq!(
            consume_web_handoff(&handoffs, &code, t0 + Duration::from_secs(10)),
            Some("/settings?section=messaging".to_string())
        );
        // Replay fails closed
        assert_eq!(
            consume_web_handoff(&handoffs, &code, t0 + Duration::from_secs(11)),
            None
        );

        // Expired code (>60s) is rejected
        let (exp_code, _) = mint_web_handoff(&handoffs, "/chat", t0);
        assert_eq!(
            consume_web_handoff(
                &handoffs,
                &exp_code,
                t0 + Duration::from_secs(HANDOFF_TTL_SECS + 1)
            ),
            None
        );
    }

    #[test]
    fn encrypted_login_response_never_leaks_plaintext_session_token_when_aes_gcm_requested() {
        let session_tok = "super-secret-session-token-123456";
        let payload = json!({
            "authenticated": true,
            "sessionAuthenticated": true,
            "hostId": "nola-host-abc",
            "token": session_tok,
            "token_type": "Bearer",
            "expires_in": 604800
        });
        let mobile_tok = "mobile-access-token-xyz";
        let encrypted = maybe_encrypt_api_payload(&payload, Some(mobile_tok));
        assert_eq!(encrypted["type"], "encrypted");
        assert_eq!(encrypted["alg"], "AES-256-GCM");
        let serialized = serde_json::to_string(&encrypted).unwrap();
        assert!(
            !serialized.contains(session_tok),
            "session token must never appear in plaintext when aes-gcm-v1 is requested"
        );
        let decrypted = decrypt_mobile_json_envelope(&encrypted, mobile_tok).unwrap();
        assert_eq!(decrypted["token"], session_tok);
        assert_eq!(decrypted["authenticated"], true);

        // Without mobile crypto header, returns plaintext payload directly
        let plain = maybe_encrypt_api_payload(&payload, None);
        assert_eq!(plain["token"], session_tok);
    }

    #[test]
    fn workspace_worktrees_endpoint_lists_creates_and_removes_real_git_worktree() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().canonicalize().unwrap();
        let init_ok = Command::new("git")
            .args(["init", "-q"])
            .current_dir(&repo)
            .status()
            .is_ok_and(|s| s.success());
        if !init_ok {
            return;
        }
        let _ = Command::new("git")
            .args(["config", "user.email", "test@nola.local"])
            .current_dir(&repo)
            .status();
        let _ = Command::new("git")
            .args(["config", "user.name", "Nola Test"])
            .current_dir(&repo)
            .status();
        fs::write(repo.join("README.md"), "# Worktree Test\n").unwrap();
        let _ = Command::new("git")
            .args(["add", "README.md"])
            .current_dir(&repo)
            .status();
        let commit_ok = Command::new("git")
            .args(["commit", "-q", "-m", "init"])
            .current_dir(&repo)
            .status()
            .is_ok_and(|s| s.success());
        assert!(commit_ok);

        let (st_list, listed) = handle_workspace_worktrees(&repo, false, &json!({}));
        assert_eq!(st_list, 200);
        assert_eq!(listed["ok"], true);
        assert_eq!(listed["worktrees"].as_array().unwrap().len(), 1);

        let (st_create, created) = handle_workspace_worktrees(
            &repo,
            true,
            &json!({"action": "create", "target": ".worktrees/feat-x", "branch": "feat-x"}),
        );
        assert_eq!(st_create, 200, "create failed: {created}");
        assert_eq!(created["worktrees"].as_array().unwrap().len(), 2);

        let (st_remove, removed) = handle_workspace_worktrees(
            &repo,
            true,
            &json!({"action": "remove", "target": ".worktrees/feat-x"}),
        );
        assert_eq!(st_remove, 200, "remove failed: {removed}");
        assert_eq!(removed["worktrees"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn attachments_parse_extracts_real_text_from_utf8_docx_and_pdf() {
        // 1. UTF-8 text
        let txt = parse_attachment_payload("notes.txt", "text/plain", b"Hello Nola v2");
        assert_eq!(txt["extracted"], true);
        assert_eq!(txt["text"], "Hello Nola v2");

        // 2. Real minimal in-memory DOCX (uncompressed ZIP with word/document.xml)
        let xml = b"<?xml version=\"1.0\" encoding=\"UTF-8\"?><w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:body><w:p><w:r><w:t>Quarterly Revenue Summary</w:t></w:r></w:p></w:body></w:document>";
        let docx_bytes = build_minimal_stored_zip("word/document.xml", xml);
        let parsed_docx = parse_attachment_payload(
            "report.docx",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            &docx_bytes,
        );
        assert_eq!(parsed_docx["extracted"], true, "docx parse: {parsed_docx}");
        assert!(
            parsed_docx["text"]
                .as_str()
                .unwrap_or("")
                .contains("Quarterly Revenue Summary"),
            "expected extracted DOCX text, got: {parsed_docx}"
        );

        // 3. Real minimal valid PDF with a Tj text operator
        let pdf_bytes = build_minimal_pdf("Invoice Total 4200");
        let parsed_pdf = parse_attachment_payload("invoice.pdf", "application/pdf", &pdf_bytes);
        assert_eq!(parsed_pdf["extracted"], true, "pdf parse: {parsed_pdf}");
        assert!(
            parsed_pdf["text"]
                .as_str()
                .unwrap_or("")
                .contains("Invoice Total 4200"),
            "expected extracted PDF text, got: {parsed_pdf}"
        );
    }

    fn crc32_ieee(data: &[u8]) -> u32 {
        let mut crc: u32 = 0xFFFF_FFFF;
        for &b in data {
            crc ^= b as u32;
            for _ in 0..8 {
                let mask = (crc & 1).wrapping_neg();
                crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
            }
        }
        !crc
    }

    fn build_minimal_stored_zip(filename: &str, content: &[u8]) -> Vec<u8> {
        let name_bytes = filename.as_bytes();
        let crc = crc32_ieee(content);
        let size = content.len() as u32;
        let mut out = Vec::new();
        // Local file header
        out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        out.extend_from_slice(&20u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // method=store
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(name_bytes);
        out.extend_from_slice(content);
        let cd_offset = out.len() as u32;
        // Central directory header
        out.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        out.extend_from_slice(&20u16.to_le_bytes());
        out.extend_from_slice(&20u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(name_bytes);
        let cd_size = (out.len() as u32) - cd_offset;
        // End of central directory
        out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&cd_size.to_le_bytes());
        out.extend_from_slice(&cd_offset.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out
    }

    fn build_minimal_pdf(text: &str) -> Vec<u8> {
        let stream = format!("BT /F1 12 Tf 72 720 Td ({text}) Tj ET");
        let mut pdf = String::from("%PDF-1.4\n");
        let mut offsets = Vec::new();
        let objs = [
            "1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n".to_string(),
            "2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n".to_string(),
            "3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>\nendobj\n".to_string(),
            format!(
                "4 0 obj\n<< /Length {} >>\nstream\n{stream}\nendstream\nendobj\n",
                stream.len()
            ),
            "5 0 obj\n<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>\nendobj\n".to_string(),
        ];
        for obj in &objs {
            offsets.push(pdf.len());
            pdf.push_str(obj);
        }
        let xref_pos = pdf.len();
        pdf.push_str(&format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1));
        for off in offsets {
            pdf.push_str(&format!("{off:010} 00000 n \n"));
        }
        pdf.push_str(&format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_pos}\n%%EOF\n",
            objs.len() + 1
        ));
        pdf.into_bytes()
    }

    #[test]
    fn documents_search_queries_indexed_doc_chunks_and_goal_plan_recall_oauth_work_end_to_end() {
        let dir = tempfile::tempdir().unwrap();
        let doc_file = dir.path().join("architecture.md");
        fs::write(
            &doc_file,
            "# Architecture\nNola uses SQLite FTS5 chunk indexing for lightning fast local document search.\n",
        )
        .unwrap();
        let store = Store::open_in_memory().unwrap();
        crate::config::bind_store_vault_path(&store, &dir.path().join("vault.json")).unwrap();
        let suite = Arc::new(ModulesSuite::new(
            store.clone(),
            dir.path().to_path_buf(),
            None,
        ));

        // 1. Index data source & search doc chunks (both GET and POST share `execute_documents_search`)
        let ds_rec = nola_types::DataSourceRecord {
            id: "ds-arch".into(),
            name: "Architecture Docs".into(),
            kind: "localFile".into(),
            location: doc_file.display().to_string(),
            metadata: json!({}),
            usage_count: 0,
            updated_at_ms: now_ms(),
        };
        suite.upsert_record("data_source", "ds-arch", &ds_rec).unwrap();
        let indexed = index_data_source(&store, "ds-arch").unwrap();
        assert!(indexed >= 1);

        let search_res = execute_documents_search(
            &store,
            dir.path(),
            "lightning fast local document",
            Some(&["ds-arch".to_string()]),
            10,
        );
        assert_eq!(search_res["ok"], true);
        assert!(search_res["count"].as_u64().unwrap() >= 1);
        assert_eq!(search_res["documents"][0]["sourceId"], "ds-arch");

        // 2. Goal control lifecycle (create -> pause -> resume -> complete)
        let (st, created) = handle_conversation_goal_control(
            &store,
            &suite,
            "host-1",
            "conv-g1",
            &json!({"action": "create", "title": "Ship Wave 3", "objective": "Pass all checks"}),
        );
        assert_eq!(st, 200);
        assert_eq!(created["ok"], true);
        assert_eq!(created["source_host_id"], "host-1");
        assert_eq!(created["goal"]["controlState"], "pending");

        let (_, paused) = handle_conversation_goal_control(
            &store,
            &suite,
            "host-1",
            "conv-g1",
            &json!({"action": "pause"}),
        );
        assert_eq!(paused["goal"]["controlState"], "paused");

        let (_, completed) = handle_conversation_goal_control(
            &store,
            &suite,
            "host-1",
            "conv-g1",
            &json!({"action": "complete"}),
        );
        assert_eq!(completed["goal"]["controlState"], "completed");
        assert_eq!(completed["goal"]["accepted"], true);

        // 3. Plan Mode toggle & approval
        let (pst, entered) = handle_conversation_plan(&store, "conv-p1", &json!({"enabled": true}));
        assert_eq!(pst, 200);
        assert_eq!(entered["planMode"], true);
        store
            .set_kv(
                "plan:conv-p1",
                &json!({"title": "Rollout Plan", "content": "1. Test\n2. Ship", "status": "proposed"}),
            )
            .unwrap();
        let (ast, approved) =
            handle_conversation_plan(&store, "conv-p1", &json!({"action": "approve"}));
        assert_eq!(ast, 200);
        assert_eq!(approved["approved"], true);
        assert_eq!(approved["planMode"], false);
        assert_eq!(approved["plan"]["status"], "approved");

        // 4. Conversation recall (FTS + range)
        store
            .append_event(
                "conv-r1",
                "chat_user",
                json!({"content": "Deploy kubernetes canary safely"}),
                100,
            )
            .unwrap();
        let recalled = handle_conversation_recall(&store, "conv-r1", &json!({"query": "canary"}));
        assert!(recalled["count"].as_u64().unwrap() >= 1);
        let ranged = handle_conversation_recall(&store, "conv-r1", &json!({"from_seq": 1, "limit": 5}));
        assert!(ranged["count"].as_u64().unwrap() >= 1);

        // 5. Auxiliary prepare -> admit -> inspect -> cancel
        let aux_ops = Mutex::new(BTreeMap::new());
        let (s1, prep) = handle_auxiliary_dispatch(
            &store,
            &suite,
            &aux_ops,
            "host-1",
            "prepare",
            &json!({"expected_host_id": "host-1"}),
        );
        assert_eq!(s1, 200);
        assert_eq!(prep["source_host_id"], "host-1");
        let (s2, adm) = handle_auxiliary_dispatch(
            &store,
            &suite,
            &aux_ops,
            "host-1",
            "admit",
            &json!({"expected_host_id": "host-1", "operation_id": "op-1"}),
        );
        assert_eq!(s2, 200);
        assert_eq!(adm["receipt"]["dispatch_permitted"], false);
        assert_eq!(adm["receipt"]["operation"]["state"], "ready");
        assert!(adm["receipt"]["operation"]["scenario"].is_string());
        assert!(adm["receipt"]["operation"]["configuration"]["input_digest"].is_string());
        assert_eq!(adm["receipt"]["operation"]["configuration"]["max_attempts"], 1);
        let (s3, insp) = handle_auxiliary_dispatch(
            &store,
            &suite,
            &aux_ops,
            "host-1",
            "inspect",
            &json!({"expected_host_id": "host-1", "operation_id": "op-1"}),
        );
        assert_eq!(s3, 200);
        assert_eq!(insp["operation_id"], "op-1");
        let (s4, canc) = handle_auxiliary_dispatch(
            &store,
            &suite,
            &aux_ops,
            "host-1",
            "cancel",
            &json!({"expected_host_id": "host-1", "operation_id": "op-1"}),
        );
        assert_eq!(s4, 200);
        assert_eq!(canc["receipt"]["operation"]["state"], "cancelled");

        // 6. Real PKCE OAuth state management (Kimi & Source-Login)
        let flows = Mutex::new(BTreeMap::new());
        let (ks, kconn) = handle_source_oauth_route(
            &store,
            &suite,
            &flows,
            "/api/model-connections/kimi-login/connect",
            "",
            &json!({"account_id": "kimi-1", "origin": "http://127.0.0.1:58627", "server_bearer": "secret-tok"}),
        );
        assert_eq!(ks, 200);
        assert_eq!(kconn["state"], "idle");
        let (os, owners) = handle_source_oauth_route(
            &store,
            &suite,
            &flows,
            "/api/model-connections/kimi-login/owners",
            "",
            &json!({}),
        );
        assert_eq!(os, 200);
        assert_eq!(owners.as_array().unwrap().len(), 1);

        let (ss, started) = handle_source_oauth_route(
            &store,
            &suite,
            &flows,
            "/api/model-connections/source-login/start",
            "",
            &json!({"request_id": "req-pkce-1", "account_id": "codex-1", "display_name": "Codex"}),
        );
        assert_eq!(ss, 200);
        assert_eq!(started["state"], "pending");
        assert_eq!(started["inference_supported"], false);
        assert_eq!(started["pkce_method"], "S256");
        let auth_url = started["authorization_url"].as_str().unwrap();
        assert!(auth_url.starts_with("https://auth.openai.com/oauth/authorize?"));
        assert!(auth_url.contains("code_challenge="));
    }

    #[test]
    fn second_pass_security_and_cross_client_contracts_hold() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory().unwrap();
        crate::config::bind_store_vault_path(&store, &dir.path().join("vault.json")).unwrap();
        let suite = Arc::new(ModulesSuite::new(
            store.clone(),
            dir.path().to_path_buf(),
            None,
        ));

        // 1. Handoff target rejects control chars, whitespace, non-ASCII
        assert_eq!(safe_handoff_target("/settings?tab=models"), "/settings?tab=models");
        assert_eq!(safe_handoff_target("/settings\r\nSet-Cookie: evil=1"), "/settings");
        assert_eq!(safe_handoff_target("/settings with space"), "/settings");
        assert_eq!(safe_handoff_target("/settings/中文"), "/settings");

        // 2. SEC-7 sensitive data source detection
        assert!(is_sensitive_data_source_record(
            &suite,
            &json!({"kind": "localFile", "location": "/home/user/.ssh/id_rsa"})
        ));
        assert!(!is_sensitive_data_source_record(
            &suite,
            &json!({"kind": "localFile", "location": "README.md"})
        ));

        // 3. Local model dest_path confinement & ModelScope URL & vLLM recommend
        let confined = confined_local_model_dest_path(Some("/etc/cron.d/pwned.gguf"), "fallback");
        assert!(confined.starts_with(managed_local_models_dir()));
        assert_eq!(confined.file_name().unwrap(), "pwned.gguf");
        let ms_url = build_local_model_download_url("modelscope", "Qwen/Qwen3-8B-GGUF", Some("q4.gguf")).unwrap();
        assert!(ms_url.contains("modelscope.cn/models/Qwen/Qwen3-8B-GGUF/resolve/master/q4.gguf"));
        let vllm_cands = build_vllm_recommend_candidates(&json!({"memoryBytes": 32_000_000_000u64, "unifiedMemory": true}));
        assert!(!vllm_cands.is_empty());
        assert_eq!(vllm_cands[0]["plan"]["engine"], "vllm");

        // 4. OAuth origin validation & constant-time full-state verification on opencode-login/callback
        assert!(validate_oauth_origin("https://example.com").is_ok());
        assert!(validate_oauth_origin("http://127.0.0.1:4096").is_ok());
        assert!(validate_oauth_origin("javascript:alert(1)").is_err());
        assert!(validate_oauth_origin("http://evil.example.com").is_err());

        let flows = Mutex::new(BTreeMap::new());
        let (bad_st, _) = handle_source_oauth_route(
            &store,
            &suite,
            &flows,
            "/api/model-connections/kimi-login/connect",
            "",
            &json!({"account_id": "k-bad", "origin": "javascript:alert(1)", "server_bearer": "tok"}),
        );
        assert_eq!(bad_st, 400);
        let (oc_st, oc_auth) = handle_source_oauth_route(
            &store,
            &suite,
            &flows,
            "/api/model-connections/opencode-login/authorize",
            "",
            &json!({"request_id": "oc-req-1", "account_id": "oc-1", "provider": "opencode", "method": 0}),
        );
        assert_eq!(oc_st, 200);
        assert_eq!(oc_auth["state"], "awaiting_authorization");
        let full_state = flows.lock().unwrap().get("oc-req-1").unwrap().csrf_state.clone();
        // Prefix state MUST be rejected even when request_id matches!
        let (prefix_st, _) = handle_source_oauth_route(
            &store,
            &suite,
            &flows,
            "/api/model-connections/opencode-login/callback",
            "",
            &json!({"request_id": "oc-req-1", "state": &full_state[..8], "code": "abc"}),
        );
        assert_eq!(prefix_st, 400);
        // Full constant-time state succeeds -> source_callback_completed
        let (ok_cb, cb_val) = handle_source_oauth_route(
            &store,
            &suite,
            &flows,
            "/api/model-connections/opencode-login/callback",
            "",
            &json!({"request_id": "oc-req-1", "state": full_state, "code": "abc"}),
        );
        assert_eq!(ok_cb, 200);
        assert_eq!(cb_val["state"], "source_callback_completed");

        // 5. SourceAccounts bind/refresh nested shape
        let (bs, bound) = handle_source_oauth_route(
            &store,
            &suite,
            &flows,
            "/api/model-connections/source-accounts/bind",
            "",
            &json!({"account_id": "sa-1", "display_name": "My Codex", "source": "codex", "path": "/tmp/nonexistent"}),
        );
        assert_eq!(bs, 200);
        assert_eq!(bound["account"]["revision"], 1);
        assert_eq!(bound["account"]["account"]["id"], "sa-1");
        assert_eq!(bound["account"]["account"]["display_name"], "My Codex");
        assert_eq!(bound["account"]["account"]["source"], "codex");
        assert_eq!(bound["account"]["account"]["path"], "/tmp/nonexistent");
        assert_eq!(bound["login_state"], "unauthenticated");
        assert!(bound["user_action"].is_string());

        // 6. macOS remote chat & iOS resumeObservedRun NOT hijacked by is_messages_read_post
        assert!(!is_messages_read_post_with_accept(
            &json!({"expected_host_id": "h1", "messages": [{"role": "user", "content": "hello"}]}),
            false
        ));
        assert!(!is_messages_read_post_with_accept(
            &json!({"expected_host_id": "h1", "expected_run_id": "run-1"}),
            true
        ));
        assert!(is_messages_read_post_with_accept(
            &json!({"expected_host_id": "h1"}),
            false
        ));

        // 7. Hybrid device_bridge tools + full device_tool_request frame + boolean error callback
        let specs = extract_hybrid_device_tool_specs(&json!({
            "device_bridge": {
                "tools": [{
                    "capability": "calendar",
                    "schema": {
                        "type": "function",
                        "function": {
                            "name": "list_events",
                            "description": "List calendar events",
                            "parameters": {"type": "object", "properties": {}}
                        }
                    }
                }]
            }
        }));
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].0.schema.name, "list_events");
        assert_eq!(specs[0].1, "calendar");
        let frame = build_device_tool_request_frame(
            "req-1",
            "conv-1",
            "host-1",
            "client-1",
            "list_events",
            "calendar",
            &json!({"limit": 5}),
            "deadbeef",
            1_900_000_000_000,
        );
        assert_eq!(frame["protocol_version"], 1);
        assert_eq!(frame["tool_name"], "list_events");
        assert_eq!(frame["tool"], "list_events");
        assert_eq!(frame["nonce"], "deadbeef");
        assert_eq!(frame["session_id"], "conv-1");
        assert!(frame["trace"].is_object());
        assert!(device_tool_callback_is_ok(&json!({"output": "ok", "error": false})));
        assert!(device_tool_callback_is_ok(&json!({"output": "ok", "error": null})));
        assert!(!device_tool_callback_is_ok(&json!({"error": true})));
        assert!(!device_tool_callback_is_ok(&json!({"error": "boom"})));

        // 8. iOS permission resolution parsing
        assert_eq!(
            parse_permission_resolution(&json!({"resolution": {"decision": "allow_once"}})),
            (true, "once")
        );
        assert_eq!(
            parse_permission_resolution(&json!({"resolution": {"decision": "allow_conversation"}})),
            (true, "session")
        );
        assert_eq!(
            parse_permission_resolution(&json!({"resolution": {"decision": "deny"}})),
            (false, "once")
        );

        // 9. Goal schedule + applied_operations + plan reject/clear + recall messageId + worktree isMain
        let (gs, sched) = handle_conversation_goal_control(
            &store,
            &suite,
            "host-1",
            "conv-sched",
            &json!({
                "action": "schedule",
                "operationId": "op-sched-1",
                "wakeAtEpochMs": 1_900_000_000_000i64
            }),
        );
        assert_eq!(gs, 200);
        assert_eq!(sched["conversationId"], "conv-sched");
        assert_eq!(sched["operationId"], "op-sched-1");
        assert_eq!(sched["authoritative"], true);
        assert!(sched["goalProjection"]["applied_operations"]["op-sched-1"]["command_sha256"]
            .as_str()
            .is_some_and(|s| s.len() == 64));

        let (rs, rej) = handle_conversation_plan(&store, "conv-p2", &json!({"action": "reject", "feedback": "revise"}));
        assert_eq!(rs, 200);
        assert_eq!(rej["plan"]["status"], "rejected");
        let (cs, clr) = handle_conversation_plan(&store, "conv-p2", &json!({"action": "clear"}));
        assert_eq!(cs, 200);
        assert!(clr["plan"].is_null());

        store
            .append_event("conv-rec", "chat_user", json!({"content": "needle phrase"}), 100)
            .unwrap();
        let rec = handle_conversation_recall(&store, "conv-rec", &json!({"q": "needle"}));
        assert!(rec["matches"][0]["messageId"]
            .as_str()
            .is_some_and(|s| s.starts_with("msg-")));
        assert!(rec["matches"][0]["createdAtMs"].is_i64());

        let wts = parse_git_worktree_porcelain("worktree /repo\nHEAD abc\nbranch refs/heads/main\n\nworktree /repo/.nola/worktrees/feat\nHEAD def\nbranch refs/heads/feat\n");
        assert_eq!(wts[0]["isMain"], true);
        assert_eq!(wts[1]["isMain"], false);
    }

    #[test]
    fn mobile_api_and_explicit_auth_never_receive_implicit_loopback_trust() {
        // Plain local Web UI request on loopback without env token override -> trusted
        assert!(compute_local_trust(true, false, "/api/agent/settings", false));
        assert!(compute_local_trust(true, false, "/api/scheduled-actions", false));

        // `/api/mobile` and `/api/mobile/*` MUST NEVER receive implicit loopback trust
        assert!(!compute_local_trust(true, false, "/api/mobile", false));
        assert!(!compute_local_trust(true, false, "/api/mobile/agent/settings", false));
        assert!(!compute_local_trust(true, false, "/api/mobile/agent/settings", true));

        // Explicit Authorization or X-Nola-Mobile-Auth header disables loopback trust
        assert!(!compute_local_trust(true, false, "/api/agent/settings", true));

        // Non-loopback or env token override disables loopback trust
        assert!(!compute_local_trust(false, false, "/api/agent/settings", false));
        assert!(!compute_local_trust(true, true, "/api/agent/settings", false));
    }

    #[test]
    fn web_server_pdf_resources_and_remote_host_contracts_work() {
        let store = Store::open_in_memory().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let suite = ModulesSuite::new(store.clone(), dir.path().to_path_buf(), None);
        let cancels: Arc<Mutex<BTreeMap<String, Arc<AtomicBool>>>> =
            Arc::new(Mutex::new(BTreeMap::new()));

        // 4a. PDF resource upload, canonical list fields, and raw bytes extraction
        let conv_id = "conv-pdf-1";
        let pdf_bytes = b"%PDF-1.4\n%test\n";
        let rid = "11111111-2222-3333-4444-555555555555";
        let uploaded = store_conversation_resource_upload(
            &store,
            conv_id,
            &json!({
                "resourceId": rid,
                "displayName": "report.pdf",
                "mimeType": "application/pdf",
                "dataBase64": base64_encode(pdf_bytes)
            }),
        )
        .unwrap();
        assert_eq!(uploaded["resourceId"], rid);
        assert_eq!(uploaded["displayName"], "report.pdf");
        assert_eq!(uploaded["mimeType"], "application/pdf");
        assert_eq!(uploaded["byteCount"], pdf_bytes.len() as u64);
        assert_eq!(
            uploaded["previewUrl"],
            format!("/api/chat/conversations/{conv_id}/resources/{rid}")
        );
        assert_eq!(uploaded["sha256"].as_str().unwrap().len(), 64);

        let listed = collect_conversation_resources(&store, conv_id);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0]["resourceId"], rid);
        assert!(listed[0].get("data_base64").is_none());

        let raw_list = collect_conversation_resources_raw(&store, conv_id);
        let decoded = extract_resource_raw_bytes(&raw_list[0]).unwrap();
        assert_eq!(decoded, pdf_bytes);

        // 4b. Assistant journal operation detection & dispatch
        let open_body = json!({
            "operation": "open_snapshot",
            "at": {
                "identity": {
                    "conversation_id": conv_id,
                    "run_id": "run-1",
                    "assistant_message_id": "asst-1",
                    "source_store_epoch": "ep-1"
                },
                "sequence": "1",
                "record_digest": "0".repeat(64)
            }
        });
        assert!(is_assistant_journal_operation(&open_body));
        let j_resp =
            handle_assistant_journal_http(&store, &suite, "host-a", &open_body, true).unwrap();
        assert_eq!(j_resp["source_host_id"], "host-a");
        assert!(j_resp["result"]["snapshot_id"].is_string());

        // 4b. Paired task cancellation strict 8-key schema
        let cancel_body = json!({
            "hostId": "host-a",
            "conversationId": conv_id,
            "parentRunId": "run-1",
            "taskId": "task-1"
        });
        assert!(is_paired_task_cancellation_body(&cancel_body));
        let cancel_resp = handle_paired_task_cancellation(
            &store,
            &suite,
            &cancels,
            "host-a",
            conv_id,
            None,
            &cancel_body,
        );
        assert_eq!(cancel_resp.as_object().unwrap().len(), 8);
        assert_eq!(cancel_resp["hostId"], "host-a");
        assert_eq!(cancel_resp["taskId"], "task-1");
        assert_eq!(cancel_resp["cancelRequested"], true);
        assert_eq!(cancel_resp["applicationPending"], false);

        // 4b. Task guidance receipt schema
        let guid_body = json!({
            "hostId": "host-a",
            "conversationId": conv_id,
            "parentRunId": "run-1",
            "taskId": "task-1",
            "messageId": "msg-g1",
            "submittedAtMs": 1700000000000i64,
            "message": "stay concise"
        });
        assert!(is_task_guidance_post(&guid_body, false));
        assert!(!is_task_guidance_post(&guid_body, true));
        let guid_resp = handle_task_guidance_http(&store, "host-a", conv_id, &guid_body, false);
        assert_eq!(guid_resp["hostId"], "host-a");
        assert_eq!(guid_resp["messageId"], "msg-g1");
        assert_eq!(guid_resp["guidance"]["status"], "accepted");
        assert_eq!(guid_resp["guidance"]["content"], "stay concise");

        // 4b. Remote submission reconciliation schema
        let digest = "a".repeat(64);
        let sub_body = json!({
            "operation": "observe",
            "identity": {
                "source_host_id": "host-a",
                "client_id": "cli-1",
                "conversation_id": conv_id,
                "submission_id": "sub-1",
                "user_message_id": "u-1",
                "assistant_message_id": "a-1"
            },
            "request_digest": digest
        });
        let rec = handle_remote_submission_reconciliation(
            &store,
            &cancels,
            "host-a",
            conv_id,
            &sub_body,
        );
        assert_eq!(rec["protocol_version"], 1);
        assert_eq!(rec["phase"], "prepared");
        assert!(rec["admission"].is_null());
        assert!(rec["rejection_code"].is_null());
    }
}
