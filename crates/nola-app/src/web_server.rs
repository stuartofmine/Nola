// SPDX-License-Identifier: Apache-2.0
//! Full-Capability Secure HTTP + SSE WebHost (`tiny_http`) for Web, iOS, macOS, Android & Windows clients:
//! - Strict same-origin CSRF (`Origin == Host` + `Sec-Fetch-Site != cross-site`)
//! - O(1) bounded replay-nonce cache
//! - Persistent 0600 mobile access token + session cookies + HMAC-SHA256 mobile auth + AES-256-GCM mobile crypto
//! - Live streaming SSE chat endpoint (`POST /api/chat/conversations/<id>/messages`) with real media attachments
//! - Active-run cancellation, session-scoped permissions, real compaction, MCP tool wiring, and background scheduler
//! - Complete business module REST endpoints aligned with `apps/web/frontend`, iOS, macOS & Android
//! - Safe static asset & SPA serving (`/ui/*`, `/static/*`, and SPA fallback).

use std::collections::{BTreeMap, HashSet, VecDeque};
use std::fs;
use std::io::{Read, Write};
use std::net::{IpAddr, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use chrono::Utc;
use nola_engine::{
    compact_session_now, register_planning_tools, register_subagent_tools, AgentEngine, AgentEvent,
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
use nola_modules::{
    built_in_role_templates, check_for_update, download_update_package, list_data_source_usage,
    list_desktop_plugin_pages, list_harness_plugins, parse_inbound_webhook_payload,
    probe_engine_status, probe_hardware, recommend_local_models, spawn_background_scheduler,
    test_data_source_connection, verify_inbound_webhook_token, AutomationScheduler, GoalWorker,
    LocalModelProcessManager, ModulesSuite, RoomRuntime,
};
use nola_store::{EventPage, Store, Vault};
use nola_tools::{BrowserManager, NativeBrowserBridge, SandboxMode, ToolContext, ToolRegistry};
use nola_types::{
    new_id, now_ms, sha256_hex, AgentRoom, ExpertPeer, FrozenSystemBlock, HostCapabilities,
    McpServerConfig, McpTransport, MediaAttachment, MemoryItem, ModelCapabilities,
    PermissionAction, PermissionProfile, PermissionRule, PluginRecord, RuleScope, Session,
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
    build_workspace_review, conversation_configuration_json, conversation_knowledge_key,
    ensure_host_id, infer_model_capabilities, load_conversation_knowledge,
    memory_item_to_record_with_store, memory_meta_key, ms_to_rfc3339, normalize_automation_record,
    reconstruct_v2_messages, session_meta_key, session_to_conversation_json, session_usage_json,
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
    tx: mpsc::Sender<bool>,
}

struct PendingQuestionEntry {
    conversation_id: String,
    tx: mpsc::Sender<Option<String>>,
}

/// Pending interactive approval & user-question waiters bridged between SSE turns and REST,
/// plus narrowly scoped `(conv_id, tool_name, target_key)` session permission grants.
#[derive(Default)]
struct PendingInteractions {
    approvals: Mutex<BTreeMap<String, PendingApprovalEntry>>,
    questions: Mutex<BTreeMap<String, PendingQuestionEntry>>,
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
        self.questions
            .lock()
            .unwrap()
            .values()
            .any(|e| e.conversation_id == conv_id)
    }

    fn any_needs_response(&self) -> bool {
        !self.approvals.lock().unwrap().is_empty() || !self.questions.lock().unwrap().is_empty()
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
    match validate_mobile_hmac_auth(request, method_str, full_url, raw_body, access_token, nonces) {
        MobileHmacResult::Valid => return (true, false),
        MobileHmacResult::ReplayedNonce => return (false, true),
        MobileHmacResult::Invalid => {}
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

fn respond_api_json(
    req: Request,
    status: u16,
    value: &Value,
    encrypt_for_token: Option<&str>,
) -> std::io::Result<()> {
    if let Some(tok) = encrypt_for_token {
        if let Ok(mut env) = encrypt_mobile_json_envelope(value, tok) {
            if let Some(map) = env.as_object_mut() {
                map.insert("type".into(), json!("encrypted"));
            }
            return respond_json(req, status, &env);
        }
    }
    respond_json(req, status, value)
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
    nonces: Arc<Mutex<BoundedNonceCache>>,
    active_cancels: Arc<Mutex<BTreeMap<String, Arc<AtomicBool>>>>,
    pending_interactions: Arc<PendingInteractions>,
    native_browser: Arc<NativeBrowserBridge>,
    tool_plans: Arc<Mutex<BTreeMap<String, Value>>>,
    mcp_manager: Arc<McpManager>,
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
    #[cfg(not(unix))]
    {
        true
    }
}

fn has_expected_host_id(body: &Value) -> bool {
    body.get("expected_host_id")
        .and_then(|v| v.as_str())
        .is_some_and(|s| !s.trim().is_empty())
}

fn wrap_paired_result(host_id: &str, body: &Value, result: Value) -> Value {
    if has_expected_host_id(body) {
        json!({
            "source_host_id": host_id,
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
        || path.starts_with("/v1/channels/webhook/")
        || path.starts_with("/api/v1/channels/webhook/")
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
        if arg.starts_with("--tailcat-ingress=")
            || arg.starts_with("--tailcat-helper=")
            || arg.starts_with("--tailcat-helper-sha256=")
        {
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
            "--tailcat-ingress" | "--tailcat-helper" | "--tailcat-helper-sha256" => {
                if args.get(i + 1).is_some_and(|next| !next.starts_with("--")) {
                    i += 1;
                }
            }
            "--json" => {
                json_mode = true;
            }
            _ => {}
        }
        i += 1;
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
    let nonces = Arc::new(Mutex::new(BoundedNonceCache::default()));
    let active_cancels: Arc<Mutex<BTreeMap<String, Arc<AtomicBool>>>> =
        Arc::new(Mutex::new(BTreeMap::new()));
    let pending_interactions = Arc::new(PendingInteractions::default());
    let native_browser = Arc::new(NativeBrowserBridge::default());
    let tool_plans: Arc<Mutex<BTreeMap<String, Value>>> = Arc::new(Mutex::new(BTreeMap::new()));

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
        nonces,
        active_cancels,
        pending_interactions,
        native_browser,
        tool_plans,
        mcp_manager,
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
    let nonces = &state.nonces;
    let active_cancels = &state.active_cancels;
    let pending_interactions = &state.pending_interactions;
    let native_browser = &state.native_browser;
    let tool_plans = &state.tool_plans;
    let mcp_manager = &state.mcp_manager;

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

    // SEC-2: When NOLA_WEB_ACCESS_TOKEN is set (`env_token_override`), disable implicit loopback trust.
    let local_trust = loopback && !env_token_override;
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
                let _ = respond_api_json(
                    req,
                    200,
                    &json!({
                        "authenticated": authed,
                        "sessionAuthenticated": session_auth,
                        "bearerAuthenticated": bearer_auth,
                        "localTrust": local_trust,
                        "tailcatAvailable": false,
                        "transportIngress": "direct",
                        "hostId": host_id,
                        "version": "2.0.0"
                    }),
                    crypto_tok,
                );
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
                    let session_tok = generate_hex_token_32();
                    let expiry = now_ms().saturating_add(SESSION_TTL_MS);
                    {
                        let mut map = sessions.lock().unwrap();
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
                    }
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
                    let _ = respond_json_with_headers(req, 200, &payload, vec![set_cookie]);
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
                let _ = respond_json_with_headers(
                    req,
                    200,
                    &json!({"authenticated": false, "sessionAuthenticated": false}),
                    vec![clear_cookie],
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
                let payload = wrap_paired_result(host_id, &body, plan);
                let _ = respond_api_json(req, 200, &payload, crypto_tok);
            }
            (Method::Post, "/api/agent/tool-plan/read") => {
                let mid = body
                    .get("manifest_id")
                    .or_else(|| body.get("manifestId"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let cached = tool_plans.lock().unwrap().get(mid).cloned();
                if let Some(plan) = cached {
                    let _ = respond_api_json(req, 200, &plan, crypto_tok);
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
                    let _ = respond_api_json(
                        req,
                        200,
                        &json!({
                            "manifest_id": if mid.is_empty() { new_id() } else { mid.to_string() },
                            "catalog_revision": 1,
                            "ttl_seconds": 300,
                            "tools": flat
                        }),
                        crypto_tok,
                    );
                }
            }
            (Method::Post, "/api/web-handoff") => {
                let raw_target = body
                    .get("target")
                    .and_then(|v| v.as_str())
                    .unwrap_or("/settings")
                    .trim();
                let safe_path = if raw_target.starts_with('/')
                    && !raw_target.starts_with("//")
                    && !raw_target.contains('\\')
                {
                    raw_target.to_string()
                } else {
                    "/settings".to_string()
                };
                let _ = respond_api_json(
                    req,
                    200,
                    &json!({
                        "ok": true,
                        "path": safe_path,
                        "hostId": host_id
                    }),
                    crypto_tok,
                );
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
                        let text = match String::from_utf8(bytes.clone()) {
                            Ok(s) => s.chars().take(65_536).collect::<String>(),
                            Err(_) => format!(
                                "[Binary attachment '{filename}' ({mime_type}, {} bytes, sha256={})]",
                                bytes.len(),
                                &sha256_hex(&bytes)[..16]
                            ),
                        };
                        let _ = respond_api_json(
                            req,
                            200,
                            &json!({
                                "ok": true,
                                "filename": filename,
                                "mime_type": mime_type,
                                "bytes": bytes.len(),
                                "text": text,
                                "content": text
                            }),
                            crypto_tok,
                        );
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
            (Method::Post, "/api/documents/search") => {
                let q = body
                    .get("query")
                    .or_else(|| body.get("q"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                let mem_hits: Vec<Value> = store
                    .search_memory(&q, None, 20)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|m| json!({"kind": "memory", "id": m.id, "title": m.title, "snippet": m.content}))
                    .collect();
                let mut file_hits: Vec<Value> = Vec::new();
                if !q.is_empty() {
                    if let Ok(entries) = fs::read_dir(&suite.workspace_root) {
                        for entry in entries.flatten().take(100) {
                            let name = entry.file_name().to_string_lossy().into_owned();
                            if name.to_ascii_lowercase().contains(&q.to_ascii_lowercase()) {
                                file_hits.push(json!({"kind": "file", "path": name}));
                            }
                        }
                    }
                }
                let _ = respond_api_json(
                    req,
                    200,
                    &json!({"query": q, "memory": mem_hits, "files": file_hits}),
                    crypto_tok,
                );
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
            (Method::Post, "/api/harness-plugins") => {
                let action = body
                    .get("action")
                    .and_then(|v| v.as_str())
                    .unwrap_or("library")
                    .trim();
                match action {
                    "install" => {
                        let source = body
                            .get("source")
                            .or_else(|| body.get("name"))
                            .and_then(|v| v.as_str())
                            .map(str::trim)
                            .unwrap_or("");
                        if source.is_empty() {
                            let _ = respond_api_json(
                                req,
                                400,
                                &json!({"error": "missing_plugin_source"}),
                                crypto_tok,
                            );
                        } else {
                            let cleaned = source.strip_prefix("npm:").unwrap_or(source);
                            let (parsed_name, parsed_ver) =
                                if let Some((n, v)) = cleaned.rsplit_once('@') {
                                    if !n.is_empty() && !v.is_empty() && !v.contains('/') {
                                        (n.to_string(), v.to_string())
                                    } else {
                                        (cleaned.to_string(), "latest".to_string())
                                    }
                                } else {
                                    (cleaned.to_string(), "latest".to_string())
                                };
                            let runtime = body
                                .get("runtime")
                                .and_then(|v| v.as_str())
                                .unwrap_or("dsh")
                                .to_string();
                            let profile = body
                                .get("profile")
                                .and_then(|v| v.as_str())
                                .unwrap_or("web")
                                .to_string();
                            let id = body
                                .get("id")
                                .and_then(|v| v.as_str())
                                .map(str::trim)
                                .filter(|s| !s.is_empty())
                                .map(ToString::to_string)
                                .unwrap_or_else(|| parsed_name.clone());
                            let rec = PluginRecord {
                                id: id.clone(),
                                name: parsed_name,
                                version: parsed_ver,
                                entry_path: source.to_string(),
                                enabled: true,
                                capabilities: vec![runtime, profile],
                            };
                            if let Err(e) = suite.upsert_record("plugin", &id, &rec) {
                                let _ = respond_api_json(
                                    req,
                                    500,
                                    &json!({"error": e.to_string()}),
                                    crypto_tok,
                                );
                            } else {
                                let catalog = list_harness_plugins(Some(&store), &suite.workspace_root);
                                let _ = respond_api_json(req, 200, &catalog, crypto_tok);
                            }
                        }
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
                        let catalog = list_harness_plugins(Some(&store), &suite.workspace_root);
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
                        let _ = respond_api_json(req, 200, &conv, crypto_tok);
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
                match store.events_after(&id, after, 500) {
                    Ok(EventPage::Events(ev)) => {
                        let _ = respond_api_json(req, 200, &json!({"status": "ok", "events": ev}), crypto_tok);
                    }
                    Ok(EventPage::ResyncRequired {
                        earliest_retained_seq,
                        latest_seq,
                    }) => {
                        let _ = respond_api_json(
                            req,
                            200,
                            &json!({
                                "status": "resync_required",
                                "earliest_retained_seq": earliest_retained_seq,
                                "latest_seq": latest_seq
                            }),
                            crypto_tok,
                        );
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
                match store.get_session(&id) {
                    Ok(Some(sess)) => {
                        let conv = conversation_configuration_json(&store, &sess);
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
                if p.starts_with("/api/chat/conversations/") && p.ends_with("/configuration") =>
            {
                let id = percent_decode_path(
                    p.trim_start_matches("/api/chat/conversations/")
                        .trim_end_matches("/configuration"),
                );
                let paired = has_expected_host_id(&body);
                let op = body.get("operation").and_then(|v| v.as_str());
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
                    let payload = wrap_paired_result(host_id, &body, frontier);
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
                                let perm_cfg = json!({
                                    "conversation_id": id,
                                    "configuration_revision": rev,
                                    "permission_profile": perm,
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
                                let payload = wrap_paired_result(host_id, &body, perm_cfg);
                                let _ = respond_api_json(req, 200, &payload, crypto_tok);
                            } else {
                                let conv = conversation_configuration_json(&store, &sess);
                                let _ = respond_api_json(req, 200, &conv, crypto_tok);
                            }
                        }
                        Ok(None) => {
                            if paired {
                                let perm_cfg = json!({
                                    "conversation_id": id,
                                    "configuration_revision": 1,
                                    "permission_profile": "cautious",
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
                                let payload = wrap_paired_result(host_id, &body, perm_cfg);
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
                            wrap_paired_result(
                                host_id,
                                &body,
                                json!({
                                    "conversation_id": id,
                                    "configuration_revision": next_cfg_rev,
                                    "permission_profile": perm,
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
                            let payload = wrap_paired_result(
                                host_id,
                                &body,
                                json!({
                                    "conversation_id": id,
                                    "configuration_revision": next_rev,
                                    "permission_profile": perm,
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
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let allow = body
                    .get("allow")
                    .or_else(|| body.get("approved"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let scope = body
                    .get("scope")
                    .and_then(|v| v.as_str())
                    .unwrap_or("once");
                let resolved = pending_interactions.resolve_approval(req_id, allow, scope);
                let _ = respond_api_json(
                    req,
                    200,
                    &json!({"ok": true, "resolved": resolved, "scope": scope}),
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
                let resp_val = if has_expected_host_id(&body) {
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
                                let resp_val = wrap_paired_result(host_id, &body, inner);
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
                        let _ = respond_api_json(req, 200, &report, crypto_tok);
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
                let _ = respond_api_json(req, 200, &json!({"goal": goal}), crypto_tok);
            }
            (Method::Post, p) | (Method::Put, p)
                if p.starts_with("/api/chat/conversations/") && p.ends_with("/goal") =>
            {
                let conv_id = percent_decode_path(
                    p.trim_start_matches("/api/chat/conversations/")
                        .trim_end_matches("/goal"),
                );
                let mut goal_body = body;
                let id = goal_body
                    .get("id")
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string)
                    .unwrap_or_else(new_id);
                goal_body["id"] = json!(id);
                goal_body["conversationId"] = json!(conv_id);
                goal_body["updatedAt"] = json!(Utc::now().to_rfc3339());
                let _ = store.set_kv(&format!("conv_goal:{conv_id}"), &goal_body);
                let _ = suite.upsert_record("goal", &id, &goal_body);
                let _ = respond_api_json(req, 200, &json!({"ok": true, "goal": goal_body}), crypto_tok);
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
            (Method::Post, p)
                if p.starts_with("/api/chat/conversations/") && p.ends_with("/messages") =>
            {
                let conv_id = percent_decode_path(
                    p.trim_start_matches("/api/chat/conversations/")
                        .trim_end_matches("/messages"),
                );
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
                if method == Method::Post && is_read_only_post(&body) {
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
                                let _ = respond_api_json(req, 200, &conv, crypto_tok);
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
                            "responses" | "siwc_responses" => "openai_responses",
                            "gemini" => "gemini",
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
                let acquisition = store
                    .get_kv("local_model_acquisition")
                    .ok()
                    .flatten()
                    .unwrap_or(Value::Null);
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
                        "acquisition": acquisition
                    }),
                    crypto_tok,
                );
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
                let st = probe_engine_status(eng);
                let ready = st.get("ready").and_then(|v| v.as_bool()).unwrap_or(false);
                let _ = respond_api_json(
                    req,
                    200,
                    &json!({
                        "ok": true,
                        "ready": ready,
                        "engineStatus": st
                    }),
                    crypto_tok,
                );
            }
            (Method::Post, "/api/local-models/install-plan") => {
                let eng = body
                    .get("engine")
                    .and_then(|v| v.as_str())
                    .unwrap_or("llama.cpp");
                let src = body
                    .get("source")
                    .and_then(|v| v.as_str())
                    .unwrap_or("huggingface");
                let repo = body
                    .get("repository")
                    .or_else(|| body.pointer("/plan/repository"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let plan = body.get("plan").cloned().unwrap_or_else(|| {
                    json!({"engine": eng, "repository": repo, "files": []})
                });
                let total_bytes: u64 = plan
                    .get("files")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|f| f.get("bytes").and_then(|b| b.as_u64()))
                            .sum()
                    })
                    .unwrap_or(0);
                let job = json!({
                    "id": new_id(),
                    "engine": eng,
                    "source": src,
                    "repository": repo,
                    "plan": plan,
                    "status": "downloading",
                    "downloadedBytes": 0,
                    "totalBytes": total_bytes
                });
                let _ = store.set_kv("local_model_acquisition", &job);
                let _ = respond_api_json(req, 200, &json!({"ok": true, "job": job}), crypto_tok);
            }
            (Method::Post, "/api/local-models/cancel") => {
                let mut job = store
                    .get_kv("local_model_acquisition")
                    .ok()
                    .flatten()
                    .filter(|v| v.is_object())
                    .unwrap_or_else(|| {
                        json!({
                            "id": body.get("id").and_then(|v| v.as_str()).unwrap_or(""),
                            "engine": "llama.cpp",
                            "source": "huggingface",
                            "repository": "",
                            "status": "cancelled",
                            "downloadedBytes": 0,
                            "totalBytes": 0
                        })
                    });
                if let Some(obj) = job.as_object_mut() {
                    obj.insert("status".into(), json!("cancelled"));
                }
                let _ = store.set_kv("local_model_acquisition", &job);
                let _ = respond_api_json(
                    req,
                    200,
                    &json!({"ok": true, "cancelled": true, "job": job}),
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
            | (Method::Post, "/api/model-connections/opencode-login/owners") => {
                let _ = respond_api_json(req, 200, &json!([]), crypto_tok);
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
                            "responses" | "siwc_responses" => "openai_responses".into(),
                            "gemini" => "gemini".into(),
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
                let _ = respond_api_json(req, 200, &build_agent_settings_snapshot(&store, &suite), crypto_tok);
            }
            (Method::Post, "/api/agent/settings") => {
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
                    if trimmed.is_empty() || !seen_ids.insert(trimmed.clone()) {
                        return None;
                    }
                    Some(json!({
                        "id": trimmed,
                        "name": trimmed,
                        "description": s.description,
                        "path": s.path,
                        "body": s.body
                    }))
                })
                .collect();
                let _ = respond_api_json(req, 200, &json!({"skills": skills}), crypto_tok);
            }
            (Method::Get, "/api/memory") => {
                let q = parse_query_str(query, "q").unwrap_or_default();
                let items = store.search_memory(&q, None, 2000).unwrap_or_default();
                let records: Vec<Value> = items
                    .iter()
                    .map(|item| memory_item_to_record_with_store(&store, item))
                    .collect();
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
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(ToString::to_string);
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
                let prev_format = existing
                    .get("format")
                    .and_then(|v| v.as_str())
                    .unwrap_or("agent")
                    .to_string();
                let next_format = patch_src
                    .get("format")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&prev_format)
                    .to_string();
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
                let items: Vec<Value> = suite.list_records("data_source").unwrap_or_default();
                let wrapped: Vec<Value> = items.iter().map(|s| json!({"source": s})).collect();
                let _ = respond_api_json(
                    req,
                    200,
                    &json!({"sources": wrapped, "data_sources": items, "items": items}),
                    crypto_tok,
                );
            }
            (Method::Post, "/api/data-sources") => {
                let mut ds_body = body;
                let id = ds_body
                    .get("id")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(ToString::to_string)
                    .unwrap_or_else(new_id);
                ds_body["id"] = json!(id);
                let (ok, probe) = probe_and_attach_data_source_validation(&suite, &id, &mut ds_body);
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
                let mut ds_body = body;
                ds_body["id"] = json!(id);
                let (ok, probe) = probe_and_attach_data_source_validation(&suite, &id, &mut ds_body);
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
            (Method::Post, p) if p.starts_with("/api/data-sources/") && p.ends_with("/test") => {
                let id = percent_decode_path(
                    p.trim_start_matches("/api/data-sources/")
                        .trim_end_matches("/test"),
                );
                let mut existing = suite
                    .list_records::<Value>("data_source")
                    .unwrap_or_default()
                    .into_iter()
                    .find(|v| v.get("id").and_then(|s| s.as_str()) == Some(id.as_str()))
                    .unwrap_or_else(|| {
                        json!({"id": id, "name": id, "kind": "localFile", "location": "", "accessPolicy": "conversationOptIn"})
                    });
                let (ok, probe) = probe_and_attach_data_source_validation(&suite, &id, &mut existing);
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
                if (p.starts_with("/api/messaging-channels/") && p.ends_with("/inbound"))
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
                        let _ = respond_json(req, 200, &json!({"ok": true, "event": parsed}));
                    }
                    Err(e) => {
                        let _ = respond_json(req, 400, &json!({"error": e}));
                    }
                }
            }
            (Method::Get, "/api/messaging-channel-worker") => {
                let val = load_messaging_worker_snapshot(&store);
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
                    if let Some(v) = body.get("poll_seconds").and_then(|v| v.as_i64()) {
                        dst.insert("poll_seconds".into(), json!(v.clamp(1, 300)));
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
                let _ = respond_api_json(req, 200, &current, crypto_tok);
            }

            // --- Agent Rooms & Expert Prompt Templates ---
            (Method::Get, "/api/agent-rooms") => {
                let rooms: Vec<Value> = suite.list_records("agent_room").unwrap_or_default();
                let experts: Vec<Value> = suite.list_records("expert_peer").unwrap_or_default();
                let role_templates = built_in_role_templates();
                let _ = respond_api_json(
                    req,
                    200,
                    &json!({
                        "host_id": host_id,
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
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let mut mutated_item: Option<Value> = None;
                let mut mutation_err: Option<String> = None;
                match action {
                    "create_room" => {
                        let now = now_ms();
                        let id = cmd
                            .get("id")
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.trim().is_empty())
                            .map(ToString::to_string)
                            .unwrap_or_else(new_id);
                        let name = cmd
                            .get("name")
                            .or_else(|| cmd.get("title"))
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.trim().is_empty())
                            .unwrap_or("Collaboration Room")
                            .to_string();
                        let participants: Vec<String> = cmd
                            .get("participant_expert_ids")
                            .or_else(|| cmd.get("participantExpertIds"))
                            .and_then(|v| v.as_array())
                            .map(|a| {
                                a.iter()
                                    .filter_map(|x| x.as_str().map(ToString::to_string))
                                    .collect()
                            })
                            .unwrap_or_default();
                        let room = AgentRoom {
                            id: id.clone(),
                            name,
                            workspace_root: suite.workspace_root.display().to_string(),
                            participant_expert_ids: participants,
                            session_id: format!("room-{id}"),
                            created_at_ms: now,
                            updated_at_ms: now,
                        };
                        match suite.upsert_record("agent_room", &id, &room) {
                            Ok(()) => mutated_item = serde_json::to_value(&room).ok(),
                            Err(e) => mutation_err = Some(e.to_string()),
                        }
                    }
                    "update_room" => {
                        let id = cmd
                            .get("id")
                            .or_else(|| cmd.get("room_id"))
                            .or_else(|| cmd.get("roomId"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        if id.is_empty() {
                            mutation_err = Some("missing room id".into());
                        } else if let Ok(Some(mut room)) = suite.get_record::<AgentRoom>("agent_room", id) {
                            if let Some(name) = cmd.get("name").or_else(|| cmd.get("title")).and_then(|v| v.as_str()) {
                                room.name = name.to_string();
                            }
                            if let Some(arr) = cmd
                                .get("participant_expert_ids")
                                .or_else(|| cmd.get("participantExpertIds"))
                                .and_then(|v| v.as_array())
                            {
                                room.participant_expert_ids = arr
                                    .iter()
                                    .filter_map(|x| x.as_str().map(ToString::to_string))
                                    .collect();
                            }
                            room.updated_at_ms = now_ms();
                            match suite.upsert_record("agent_room", id, &room) {
                                Ok(()) => mutated_item = serde_json::to_value(&room).ok(),
                                Err(e) => mutation_err = Some(e.to_string()),
                            }
                        } else {
                            mutation_err = Some(format!("room '{id}' not found"));
                        }
                    }
                    "delete_room" => {
                        let id = cmd
                            .get("id")
                            .or_else(|| cmd.get("room_id"))
                            .or_else(|| cmd.get("roomId"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        let _ = suite.delete_record("agent_room", id);
                    }
                    "create_expert" => {
                        let id = cmd
                            .get("id")
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.trim().is_empty())
                            .map(ToString::to_string)
                            .unwrap_or_else(new_id);
                        let name = cmd
                            .get("name")
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.trim().is_empty())
                            .unwrap_or("Expert")
                            .to_string();
                        let role = cmd
                            .get("role")
                            .and_then(|v| v.as_str())
                            .unwrap_or("specialist")
                            .to_string();
                        let system_prompt = cmd
                            .get("system_prompt")
                            .or_else(|| cmd.get("systemPrompt"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let model_override = cmd
                            .get("model_override")
                            .or_else(|| cmd.get("modelOverride"))
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.trim().is_empty())
                            .map(ToString::to_string);
                        let allowed_tools: Vec<String> = cmd
                            .get("allowed_tools")
                            .or_else(|| cmd.get("allowedTools"))
                            .and_then(|v| v.as_array())
                            .map(|a| {
                                a.iter()
                                    .filter_map(|x| x.as_str().map(ToString::to_string))
                                    .collect()
                            })
                            .unwrap_or_default();
                        let expert = ExpertPeer {
                            id: id.clone(),
                            name,
                            role,
                            system_prompt,
                            model_override,
                            allowed_tools,
                            updated_at_ms: now_ms(),
                        };
                        match suite.upsert_record("expert_peer", &id, &expert) {
                            Ok(()) => mutated_item = serde_json::to_value(&expert).ok(),
                            Err(e) => mutation_err = Some(e.to_string()),
                        }
                    }
                    "delete_expert" => {
                        let id = cmd
                            .get("id")
                            .or_else(|| cmd.get("expert_id"))
                            .or_else(|| cmd.get("expertId"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        let _ = suite.delete_record("expert_peer", id);
                    }
                    _ => {}
                }
                if let Some(err) = mutation_err {
                    let _ = respond_api_json(req, 400, &json!({"ok": false, "error": err}), crypto_tok);
                } else {
                    let rooms: Vec<Value> = suite.list_records("agent_room").unwrap_or_default();
                    let experts: Vec<Value> = suite.list_records("expert_peer").unwrap_or_default();
                    let role_templates = built_in_role_templates();
                    let _ = respond_api_json(
                        req,
                        200,
                        &json!({
                            "ok": true,
                            "host_id": host_id,
                            "rooms": rooms,
                            "experts": experts,
                            "roleTemplates": role_templates,
                            "item": mutated_item,
                            "requestType": cmd.get("type").cloned().unwrap_or(Value::Null)
                        }),
                        crypto_tok,
                    );
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
        if matches!(op.to_ascii_lowercase().as_str(), "read" | "get" | "fetch") {
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
    ];
    !mutation_keys.iter().any(|k| obj.contains_key(*k))
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
        .max(1);
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
                        "validationState": "blocked",
                        "validatedAt": now_iso,
                        "errorMessage": "access to sensitive local path is forbidden"
                    });
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
        .unwrap_or(if ok { 1 } else { 0 });
    let err_msg = probe.get("error").cloned().unwrap_or(Value::Null);
    record["validationContract"] = json!({
        "sourceId": id,
        "readerDescription": format!("{kind} reader"),
        "schemaFields": ["content"],
        "redactedSample": sample,
        "recordCount": record_count,
        "maximumAgeMinutes": max_age,
        "validationState": state,
        "validatedAt": now_iso,
        "errorMessage": err_msg
    });
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

    json!({
        "connections": connections,
        "accounts": [],
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
    json!({
        "enabled": saved.get("enabled").and_then(|v| v.as_bool()).unwrap_or(false),
        "backend_url": saved.get("backend_url").or_else(|| saved.get("url")).and_then(|v| v.as_str()).unwrap_or(""),
        "has_access_token": saved.get("has_access_token").and_then(|v| v.as_bool()).unwrap_or(false),
        "execution_backend": saved.get("execution_backend").and_then(|v| v.as_str()).unwrap_or("nola-v2"),
        "turn_timeout_seconds": saved.get("turn_timeout_seconds").and_then(|v| v.as_i64()).unwrap_or(600).clamp(30, 14400),
        "poll_seconds": saved.get("poll_seconds").and_then(|v| v.as_i64()).unwrap_or(5).clamp(1, 300),
        "project_root": saved.get("project_root").and_then(|v| v.as_str()).filter(|s| !s.is_empty()).map(|s| json!(s)).unwrap_or(Value::Null)
    })
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

    let mut out = json!({
        "id": conn_id,
        "connection_id": conn_id,
        "kind": kind,
        "display_name": display_name,
        "account_id": account_id,
        "credential_ref": credential_ref,
        "webhook_secret_ref": webhook_secret_ref,
        "verify_token_ref": verify_token_ref,
        "provider_config": provider_config,
        "enabled": enabled,
        "state": state,
        "has_webhook_secret": has_webhook_secret
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
    if let Some(selected_arr) = body
        .get("selected_experts")
        .or_else(|| body.get("selectedExperts"))
        .and_then(|v| v.as_array())
    {
        let ids: Vec<&str> = selected_arr
            .iter()
            .filter_map(|v| {
                v.as_str().or_else(|| {
                    v.get("agent_id")
                        .or_else(|| v.get("agentId"))
                        .or_else(|| v.get("expert_id"))
                        .or_else(|| v.get("id"))
                        .and_then(|s| s.as_str())
                })
            })
            .filter(|s| !s.trim().is_empty())
            .collect();
        if !ids.is_empty() {
            let stored_experts = suite.list_records::<ExpertPeer>("expert_peer").unwrap_or_default();
            let builtins = built_in_role_templates();
            let mut expert_notes = Vec::new();
            for eid in ids {
                if let Some(exp) = stored_experts
                    .iter()
                    .find(|e| e.id == eid || e.name.eq_ignore_ascii_case(eid))
                {
                    expert_notes.push(format!(
                        "- {} ({}): {}",
                        exp.name, exp.role, exp.system_prompt
                    ));
                } else if let Some(tpl) = builtins
                    .iter()
                    .find(|t| t.get("id").and_then(|v| v.as_str()) == Some(eid))
                {
                    let name = tpl.get("name").and_then(|v| v.as_str()).unwrap_or(eid);
                    let role = tpl.get("role").and_then(|v| v.as_str()).unwrap_or(eid);
                    let sp = tpl.get("system_prompt").and_then(|v| v.as_str()).unwrap_or("");
                    expert_notes.push(format!("- {name} ({role}): {sp}"));
                } else {
                    expert_notes.push(format!("- {eid}"));
                }
            }
            if !expert_notes.is_empty() {
                prompt.push_str(&format!(
                    "\n\n[Selected Expert Context]\n{}",
                    expert_notes.join("\n")
                ));
            }
        }
    }

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

    // Respect per-conversation or per-request model override (including running `selectedModelSource == "local"`
    // AND user-created remote model UUIDs in KV `web_model_profiles`!)
    let override_source = body
        .get("selectedModelSource")
        .or_else(|| body.get("modelSource"))
        .or_else(|| body.get("model_source"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .or_else(|| sess.as_ref().and_then(|s| s.model_connection.clone()));
    let override_model = body
        .get("modelId")
        .or_else(|| body.get("selectedModelId"))
        .or_else(|| body.get("model_id"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
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
        let mut collected_reasoning = String::new();
        let mut collected_tools: Vec<Value> = Vec::new();
        let mut collected_surfaces: Vec<Value> = Vec::new();
        let run_res = engine.run_with_media(&ctx, &prompt, user_media, run_cfg, |ev| {
            match &ev {
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
                let _ = store_persist.append_event(
                    &conv_id_c,
                    "chat_assistant",
                    json!({
                        "content": res.final_text,
                        "reasoning": collected_reasoning,
                        "toolCalls": collected_tools,
                        "uiSurfaces": collected_surfaces,
                        "usage": res.usage
                    }),
                    50_000,
                );
            }
            Err(e) => {
                let err_frame = json!({"type": "error", "message": e.to_string()});
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
}
