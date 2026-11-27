// SPDX-License-Identifier: Apache-2.0
//! Authentication, CSRF same-origin enforcement, mobile HMAC & AES-256-GCM crypto, handoffs, and OAuth PKCE.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::Utc;
use nola_types::{new_id, now_ms, sha256_hex};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tiny_http::{Header, Method, Request, Response, StatusCode};
use url::Url;

use crate::config::default_home_dir;

use super::helpers::respond_json;

pub(super) const MAX_TRACKED_NONCES: usize = 4_096;
pub(super) const MAX_HTTP_BODY_BYTES: u64 = 16 * 1024 * 1024; // Allow image attachments up to 16 MiB
pub(super) const SESSION_TTL_MS: i64 = 7 * 24 * 3600 * 1000;
pub(super) const MAX_SESSIONS: usize = 256;
pub(super) const MOBILE_CRYPTO_AAD: &[u8] = b"nola-mobile-bridge-v1";

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
pub(super) fn ct_eq(a: &str, b: &str) -> bool {
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
pub(super) fn random_bytes_32() -> [u8; 32] {
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

pub(super) fn generate_hex_token_32() -> String {
    hex::encode(random_bytes_32())
}

pub(super) fn mobile_access_token_path() -> PathBuf {
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
pub(super) fn load_or_create_web_access_token() -> (String, bool) {
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

pub(super) fn parse_host_and_port(host: &str, default_port: Option<u16>) -> Option<(String, Option<u16>)> {
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

pub(super) fn url_matches_host(raw: &str, host: &str) -> Result<bool, &'static str> {
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

pub(super) fn header_value<'a>(request: &'a Request, name: &str) -> Option<&'a str> {
    request
        .headers()
        .iter()
        .find(|h| h.field.as_str().as_str().eq_ignore_ascii_case(name))
        .map(|h| h.value.as_str())
}

pub(super) fn extract_bearer_token(request: &Request) -> Option<&str> {
    let raw = header_value(request, "authorization")?.trim();
    let (scheme, tok) = raw.split_once(' ')?;
    if scheme.eq_ignore_ascii_case("bearer") {
        let t = tok.trim();
        (!t.is_empty()).then_some(t)
    } else {
        None
    }
}

pub(super) fn extract_cookie_value(request: &Request, cookie_name: &str) -> Option<String> {
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

pub(super) fn is_valid_session_cookie(
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

pub(super) fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
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
pub(super) enum MobileHmacResult {
    NotPresent,
    Valid,
    Invalid,
    ReplayedNonce,
}

pub(super) fn validate_mobile_hmac_auth(
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
pub(super) fn authenticate_request(
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

pub(super) fn is_loopback_host(host: &str) -> bool {
    matches!(host, "127.0.0.1" | "localhost" | "::1")
}

/// Implicit loopback trust applies ONLY to direct local Web UI requests on loopback
/// when `NOLA_WEB_ACCESS_TOKEN` is not set. It NEVER applies to `/api/mobile/*`
/// (paired mobile/remote host contract) or to requests supplying explicit
/// authentication headers (`Authorization` or `X-Nola-Mobile-Auth`).
pub(super) fn compute_local_trust(
    loopback: bool,
    env_token_override: bool,
    raw_path: &str,
    has_explicit_auth: bool,
) -> bool {
    let is_mobile_api = raw_path == "/api/mobile" || raw_path.starts_with("/api/mobile/");
    loopback && !env_token_override && !is_mobile_api && !has_explicit_auth
}

pub(super) fn server_started_handshake_full(
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
pub(super) fn server_started_handshake(host: &str, bound_port: u16) -> Value {
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

pub(super) fn method_as_str(m: &Method) -> &'static str {
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

pub(super) fn read_raw_body(req: &mut Request) -> Vec<u8> {
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

pub(super) fn parse_json_or_empty(bytes: &[u8]) -> Value {
    if bytes.is_empty() {
        return json!({});
    }
    serde_json::from_slice(bytes).unwrap_or_else(|_| json!({}))
}

pub(super) fn unwrap_input_envelope(body: &Value) -> Value {
    body.get("input")
        .filter(|v| v.is_object())
        .cloned()
        .unwrap_or_else(|| body.clone())
}

pub(super) fn wants_mobile_crypto(req: &Request) -> bool {
    header_value(req, "x-nola-mobile-crypto")
        .map(str::trim)
        .is_some_and(|v| v.eq_ignore_ascii_case("aes-gcm-v1"))
}

pub(super) fn maybe_decrypt_mobile_body(raw_body: &[u8], access_token: &str) -> Result<Value, String> {
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

pub(super) fn encode_sse_frame(frame: &Value, crypto_tok: Option<&str>) -> Vec<u8> {
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

pub(super) fn maybe_encrypt_api_payload(value: &Value, encrypt_for_token: Option<&str>) -> Value {
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

pub(super) fn respond_api_json(
    req: Request,
    status: u16,
    value: &Value,
    encrypt_for_token: Option<&str>,
) -> std::io::Result<()> {
    let body = maybe_encrypt_api_payload(value, encrypt_for_token);
    respond_json(req, status, &body)
}

pub(super) fn respond_api_json_with_headers(
    req: Request,
    status: u16,
    value: &Value,
    extra_headers: Vec<Header>,
    encrypt_for_token: Option<&str>,
) -> std::io::Result<()> {
    let body = maybe_encrypt_api_payload(value, encrypt_for_token);
    respond_json_with_headers(req, status, &body, extra_headers)
}

pub(super) fn respond_json_with_headers(
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

pub(super) const HANDOFF_TTL_SECS: u64 = 60;
pub(super) const MAX_HANDOFF_GRANTS: usize = 64;

pub(super) struct HandoffGrant {
    pub(super) target: String,
    pub(super) created_at: Instant,
    pub(super) expires_at: Instant,
}

pub(super) fn safe_handoff_target(raw: &str) -> String {
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

pub(super) fn mint_web_handoff(
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

pub(super) fn consume_web_handoff(
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

pub(super) fn issue_browser_session(sessions: &Mutex<BTreeMap<String, i64>>) -> String {
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


// --- OAuth PKCE / Device-Code Flow State ---

#[derive(Debug, Clone)]
pub(super) struct OAuthFlowState {
    pub(super) request_id: String,
    pub(super) account_id: String,
    pub(super) provider: String,
    pub(super) method: i64,
    pub(super) state: String,
    pub(super) csrf_state: String,
    pub(super) code_challenge: String,
    pub(super) authorization_url: Option<String>,
    pub(super) verification_url: Option<String>,
    pub(super) user_code: Option<String>,
    pub(super) callback_mode: Option<String>,
    pub(super) instructions: Option<String>,
    pub(super) poll_interval_seconds: Option<u64>,
    pub(super) cancellation_confirmed: Option<bool>,
}

pub(super) fn generate_pkce_pair() -> (String, String) {
    let verifier = base64url_encode(&random_bytes_32());
    let challenge = base64url_encode(&Sha256::digest(verifier.as_bytes()));
    (verifier, challenge)
}


// =============================================================================
// Self-contained RFC 4648 Base64/Base64URL + NIST AES-256-GCM (`aes-gcm-v1`)
// =============================================================================

pub(super) const B64_STD: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub(super) fn base64_encode(input: &[u8]) -> String {
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

pub(super) fn base64url_encode(input: &[u8]) -> String {
    base64_encode(input)
        .replace('+', "-")
        .replace('/', "_")
        .trim_end_matches('=')
        .to_string()
}

pub(super) fn base64_decode(input: &str) -> Result<Vec<u8>, String> {
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

pub(super) const AES_SBOX: [u8; 256] = [
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

pub(super) fn aes256_expand_key(key: &[u8; 32]) -> [[u8; 16]; 15] {
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

pub(super) fn xtime(x: u8) -> u8 {
    (x << 1) ^ (((x >> 7) & 1) * 0x1b)
}

pub(super) fn aes256_encrypt_block(rk: &[[u8; 16]; 15], input: &[u8; 16]) -> [u8; 16] {
    let mut s = *input;
    for i in 0..16 {
        s[i] ^= rk[0][i];
    }
    for (round, round_key) in rk.iter().enumerate().skip(1) {
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
            s[i] ^= round_key[i];
        }
    }
    s
}

pub(super) fn ghash_mul(x: u128, y: u128) -> u128 {
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

pub(super) fn ghash(h: u128, aad: &[u8], ct: &[u8]) -> [u8; 16] {
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

pub(super) fn aes256_gcm_encrypt(
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

pub(super) fn aes256_gcm_decrypt(
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

pub(super) fn derive_mobile_crypto_key(token: &str) -> [u8; 32] {
    let digest = Sha256::digest(token.as_bytes());
    let mut key = [0u8; 32];
    key.copy_from_slice(&digest);
    key
}

pub(super) fn encrypt_mobile_json_envelope(value: &Value, token: &str) -> Result<Value, String> {
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

pub(super) fn decrypt_mobile_json_envelope(envelope: &Value, token: &str) -> Result<Value, String> {
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

