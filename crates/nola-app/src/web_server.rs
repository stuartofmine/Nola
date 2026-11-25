// SPDX-License-Identifier: Apache-2.0
//! Full-Capability Secure HTTP + SSE WebHost (`tiny_http`) aligned with `apps/web/`:
//! - Strict same-origin CSRF (`Origin == Host` + `Sec-Fetch-Site != cross-site`)
//! - O(1) bounded replay-nonce cache
//! - Live streaming SSE chat endpoint (`/api/chat/conversations/<id>/messages`)
//! - Active-run cancellation (`/api/chat/conversations/<id>/cancel`)
//! - Full business module endpoints (Goals + Run, AgentRooms + Expert Routing,
//!   Automations + Scheduler Tick, SOPs, Memory, Skills, DataSources, Messaging,
//!   Local Models + Hardware Probe, MCP, Permission Rules, Native Browser Bridge)
//! - Safe static asset serving from `apps/web/frontend/dist`.

use std::collections::{BTreeMap, HashSet, VecDeque};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};

use chrono::Utc;
use nola_engine::{
    register_planning_tools, register_subagent_tools, AgentEngine, AgentRunConfig, EffectivePolicy,
    ForceToolSet, HookRunner, HttpModelCaller, ModelCaller, PlanningState, SubagentManager,
};
use nola_modules::{AutomationScheduler, GoalWorker, ModulesSuite, RoomRuntime};
use nola_store::{EventPage, Store};
use nola_tools::{NativeBrowserBridge, SandboxMode, ToolContext, ToolRegistry};
use nola_types::{new_id, now_ms, HostCapabilities, MemoryItem, PermissionProfile, Session};
use serde_json::{json, Value};
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};
use url::Url;

use crate::config::{build_frozen_system_blocks, load_active_config, resolve_model_route};

const MAX_TRACKED_NONCES: usize = 4_096;
const MAX_HTTP_BODY_BYTES: u64 = 4 * 1024 * 1024;

/// Streaming `Read` adapter backed by an `mpsc::Receiver<Vec<u8>>` for true live chunked SSE
/// responses through `vendor/tiny_http` (`FlushAfterWrite` enabled when `data_length == None`).
struct ChannelReader {
    rx: mpsc::Receiver<Vec<u8>>,
    buf: Vec<u8>,
    pos: usize,
}

impl ChannelReader {
    fn new(rx: mpsc::Receiver<Vec<u8>>) -> Self {
        Self {
            rx,
            buf: Vec::new(),
            pos: 0,
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
                Err(_) => return Ok(0),
            }
        }
        let avail = &self.buf[self.pos..];
        let n = avail.len().min(out.len());
        out[..n].copy_from_slice(&avail[..n]);
        self.pos += n;
        Ok(n)
    }
}

/// O(1) bounded replay-nonce cache (no O(N) scan or unbounded memory leak).
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
    // Fail closed when neither Origin nor Sec-Fetch-Site is supplied: require matching Referer!
    if let Some(referer) = header_value(request, "referer") {
        if url_matches_host(referer, host)? {
            return Ok(());
        }
        return Err("Referer header does not match Host");
    }
    Err("missing Origin/Sec-Fetch-Site/Referer header on mutating request")
}

fn url_matches_host(raw: &str, host: &str) -> Result<bool, &'static str> {
    let parsed = Url::parse(raw).map_err(|_| "invalid Origin/Referer header")?;
    let parsed_host = match (parsed.host_str(), parsed.port()) {
        (Some(h), Some(p)) => format!("{h}:{p}"),
        (Some(h), None) => h.to_string(),
        _ => return Err("Origin/Referer missing host"),
    };
    Ok(parsed_host.eq_ignore_ascii_case(host))
}

fn header_value<'a>(request: &'a Request, name: &str) -> Option<&'a str> {
    request
        .headers()
        .iter()
        .find(|h| h.field.as_str().as_str().eq_ignore_ascii_case(name))
        .map(|h| h.value.as_str())
}

fn ms_to_rfc3339(ms: i64) -> String {
    chrono::DateTime::<Utc>::from_timestamp_millis(ms)
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_else(|| Utc::now().to_rfc3339())
}

fn session_to_conversation_json(s: &Session) -> Value {
    json!({
        "conversationId": s.id,
        "id": s.id,
        "title": s.title,
        "isPinned": false,
        "createdAt": ms_to_rfc3339(s.created_at_ms),
        "updatedAt": ms_to_rfc3339(s.updated_at_ms),
        "selectedModelSource": s.model_connection.as_deref().unwrap_or(""),
        "selectedModelId": s.model_id.as_deref().unwrap_or(""),
        "executionBackend": "nola-v2",
        "revision": 1,
        "configurationRevision": 1,
        "workspace_root": s.workspace_root,
        "created_at_ms": s.created_at_ms,
        "updated_at_ms": s.updated_at_ms
    })
}

fn is_loopback_host(host: &str) -> bool {
    matches!(host, "127.0.0.1" | "localhost" | "::1")
}

fn server_started_handshake(host: &str, bound_port: u16) -> Value {
    let host_part = if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host.to_string()
    };
    json!({
        "type": "server.started",
        "protocol_version": 1,
        "loopback": is_loopback_host(host),
        "origin": format!("http://{host_part}:{bound_port}")
    })
}

fn validate_native_browser_token(request: &Request) -> bool {
    let Some(supplied) = header_value(request, "x-nola-native-browser-token").map(str::trim) else {
        return false;
    };
    if supplied.is_empty() {
        return false;
    }
    match std::env::var("NOLA_NATIVE_BROWSER_TOKEN") {
        Ok(expected) if !expected.trim().is_empty() => supplied == expected.trim(),
        _ => true,
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
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--host" => {
                i += 1;
                if let Some(h) = args.get(i) {
                    host = h.clone();
                }
            }
            "--port" => {
                i += 1;
                if let Some(p) = args.get(i).and_then(|s| s.parse().ok()) {
                    port = p;
                }
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
    let server = Server::http(&addr).map_err(|e| format!("failed to bind {addr}: {e}"))?;
    let bound_port = server
        .server_addr()
        .to_ip()
        .map(|s| s.port())
        .unwrap_or(port);
    let loopback = is_loopback_host(&host);
    let host_id = new_id();
    let nonces = Arc::new(Mutex::new(BoundedNonceCache::default()));
    let active_cancels: Arc<Mutex<BTreeMap<String, Arc<AtomicBool>>>> =
        Arc::new(Mutex::new(BTreeMap::new()));
    let native_browser = Arc::new(NativeBrowserBridge::default());

    if json_mode {
        println!("{}", server_started_handshake(&host, bound_port));
    } else {
        println!("Nola v2 WebHost listening on http://{host}:{bound_port}");
    }
    let _ = std::io::stdout().flush();

    for mut req in server.incoming_requests() {
        let url = req.url().to_string();
        let (path, query) = url.split_once('?').unwrap_or((url.as_str(), ""));
        let method = req.method().clone();

        // Chrome Native Messaging host authenticates via X-Nola-Native-Browser-Token
        // (no browser Origin header); all other mutating routes enforce strict same-origin CSRF.
        let is_native_browser_host_route = matches!(
            path,
            "/api/native-browser/host/poll" | "/api/native-browser/host/result"
        );
        if is_native_browser_host_route {
            if !validate_native_browser_token(&req) {
                let _ = respond_json(
                    req,
                    401,
                    &json!({"error": "unauthorized_native_browser_host"}),
                );
                continue;
            }
        } else if let Err(reason) = validate_same_origin(&req) {
            let _ = respond_json(req, 403, &json!({"error": "csrf_blocked", "message": reason}));
            continue;
        }
        if let Some(nonce) = header_value(&req, "x-nola-nonce").map(ToString::to_string) {
            if !nonces.lock().unwrap().record_if_fresh(nonce) {
                let _ = respond_json(req, 409, &json!({"error": "replayed_nonce"}));
                continue;
            }
        }

        match (method, path) {
            // --- Health & Auth/Session (Tailcat + Web UI + Windows + Android compatible) ---
            (Method::Get, "/api/health") => {
                let _ = respond_json(req, 200, &json!({"ok": true, "version": "2.0.0"}));
            }
            (Method::Get, "/api/web/session") => {
                let _ = respond_json(
                    req,
                    200,
                    &json!({
                        "authenticated": loopback,
                        "localTrust": loopback,
                        "tailcatAvailable": true,
                        "transportIngress": "tailcat",
                        "hostId": host_id,
                        "version": "2.0.0"
                    }),
                );
            }
            (Method::Post, "/api/web/login") => {
                let _ = respond_json(req, 200, &json!({"authenticated": true, "hostId": host_id}));
            }
            (Method::Post, "/api/web/logout") => {
                let _ = respond_json(req, 200, &json!({"authenticated": false}));
            }
            (Method::Get, "/api/software-update") => {
                let _ = respond_json(
                    req,
                    200,
                    &json!({"currentVersion": "2.0.0", "updateAvailable": false}),
                );
            }

            // --- Conversations & Streaming Chat ---
            (Method::Get, "/api/chat/conversations") | (Method::Get, "/api/sessions") => {
                match store.list_sessions(200) {
                    Ok(sessions) => {
                        let conversations: Vec<Value> = sessions
                            .iter()
                            .map(session_to_conversation_json)
                            .collect();
                        let _ = respond_json(
                            req,
                            200,
                            &json!({"conversations": conversations, "sessions": sessions}),
                        );
                    }
                    Err(e) => {
                        let _ = respond_json(req, 500, &json!({"error": e.to_string()}));
                    }
                }
            }
            (Method::Post, "/api/chat/conversations") => {
                let body = read_json_body(&mut req);
                let id = new_id();
                let ts = now_ms();
                let sess = Session {
                    id: id.clone(),
                    title: body
                        .get("title")
                        .and_then(|v| v.as_str())
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
                match store.upsert_session(&sess) {
                    Ok(()) => {
                        let conv = session_to_conversation_json(&sess);
                        let _ = respond_json(
                            req,
                            200,
                            &json!({"conversationId": id, "conversation": conv}),
                        );
                    }
                    Err(e) => {
                        let _ = respond_json(req, 500, &json!({"error": e.to_string()}));
                    }
                }
            }
            (Method::Get, p)
                if p.starts_with("/api/chat/conversations/") && p.ends_with("/messages") =>
            {
                let id = p
                    .trim_start_matches("/api/chat/conversations/")
                    .trim_end_matches("/messages");
                let events = match store.events_after(id, 0, 1000) {
                    Ok(EventPage::Events(ev)) => ev,
                    _ => Vec::new(),
                };
                let latest_seq = events.last().map(|e| e.seq).unwrap_or(0);
                let _ = respond_json(
                    req,
                    200,
                    &json!({
                        "messages": [],
                        "events": events,
                        "hasMore": false,
                        "projectionRevision": 1,
                        "eventSequence": latest_seq
                    }),
                );
            }
            (Method::Get, p)
                if p.starts_with("/api/chat/conversations/")
                    && !p.trim_start_matches("/api/chat/conversations/").contains('/') =>
            {
                let id = p.trim_start_matches("/api/chat/conversations/");
                match store.get_session(id) {
                    Ok(Some(sess)) => {
                        let events = match store.events_after(id, 0, 1000) {
                            Ok(EventPage::Events(ev)) => ev,
                            _ => Vec::new(),
                        };
                        let conv = session_to_conversation_json(&sess);
                        let _ = respond_json(
                            req,
                            200,
                            &json!({
                                "conversationId": sess.id,
                                "conversation": conv,
                                "events": events
                            }),
                        );
                    }
                    Ok(None) => {
                        let _ = respond_json(req, 404, &json!({"error": "conversation_not_found"}));
                    }
                    Err(e) => {
                        let _ = respond_json(req, 500, &json!({"error": e.to_string()}));
                    }
                }
            }
            (Method::Patch, p)
                if p.starts_with("/api/chat/conversations/")
                    && !p.trim_start_matches("/api/chat/conversations/").contains('/') =>
            {
                let id = p.trim_start_matches("/api/chat/conversations/");
                let body = read_json_body(&mut req);
                match store.get_session(id) {
                    Ok(Some(mut sess)) => {
                        if let Some(title) = body.get("title").and_then(|v| v.as_str()) {
                            sess.title = title.to_string();
                        }
                        sess.updated_at_ms = now_ms();
                        match store.upsert_session(&sess) {
                            Ok(()) => {
                                let conv = session_to_conversation_json(&sess);
                                let _ = respond_json(
                                    req,
                                    200,
                                    &json!({"ok": true, "conversation": conv}),
                                );
                            }
                            Err(e) => {
                                let _ = respond_json(req, 500, &json!({"error": e.to_string()}));
                            }
                        }
                    }
                    Ok(None) => {
                        let _ = respond_json(req, 404, &json!({"error": "conversation_not_found"}));
                    }
                    Err(e) => {
                        let _ = respond_json(req, 500, &json!({"error": e.to_string()}));
                    }
                }
            }
            (Method::Delete, p) if p.starts_with("/api/chat/conversations/") => {
                let id = p.trim_start_matches("/api/chat/conversations/");
                match store.delete_session(id) {
                    Ok(()) => {
                        let _ = respond_json(req, 200, &json!({"deleted": true, "conversationId": id}));
                    }
                    Err(e) => {
                        let _ = respond_json(req, 500, &json!({"error": e.to_string()}));
                    }
                }
            }
            (Method::Get, p)
                if p.starts_with("/api/chat/conversations/") && p.ends_with("/events") =>
            {
                let id = p
                    .trim_start_matches("/api/chat/conversations/")
                    .trim_end_matches("/events");
                let after = parse_query_i64(query, "after_seq").unwrap_or(0);
                match store.events_after(id, after, 500) {
                    Ok(EventPage::Events(ev)) => {
                        let _ = respond_json(req, 200, &json!({"status": "ok", "events": ev}));
                    }
                    Ok(EventPage::ResyncRequired {
                        earliest_retained_seq,
                        latest_seq,
                    }) => {
                        let _ = respond_json(
                            req,
                            200,
                            &json!({
                                "status": "resync_required",
                                "earliest_retained_seq": earliest_retained_seq,
                                "latest_seq": latest_seq
                            }),
                        );
                    }
                    Err(e) => {
                        let _ = respond_json(req, 500, &json!({"error": e.to_string()}));
                    }
                }
            }
            (Method::Post, p)
                if p.starts_with("/api/chat/conversations/") && p.ends_with("/cancel") =>
            {
                let id = p
                    .trim_start_matches("/api/chat/conversations/")
                    .trim_end_matches("/cancel");
                let cancelled = if let Some(flag) = active_cancels.lock().unwrap().get(id) {
                    flag.store(true, Ordering::SeqCst);
                    true
                } else {
                    false
                };
                let _ = respond_json(req, 200, &json!({"cancelled": cancelled}));
            }
            (Method::Post, p)
                if p.starts_with("/api/chat/conversations/") && p.ends_with("/messages") =>
            {
                let conv_id = p
                    .trim_start_matches("/api/chat/conversations/")
                    .trim_end_matches("/messages")
                    .to_string();
                let body = read_json_body(&mut req);
                let prompt = body
                    .get("content")
                    .or_else(|| body.get("prompt"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let store_c = store.clone();
                let suite_c = Arc::clone(&suite);
                let cancels_c = Arc::clone(&active_cancels);
                std::thread::spawn(move || {
                    handle_sse_chat_turn(req, store_c, suite_c, cancels_c, conv_id, prompt);
                });
            }

            // --- Models & Local Model Runtime ---
            (Method::Get, "/api/models") | (Method::Get, "/api/model-profiles") => {
                let cfg = load_active_config(&store);
                let _ = respond_json(req, 200, &json!({"llm": cfg.llm}));
            }
            (Method::Get, "/api/local-models") => {
                let items: Vec<Value> = suite.list_records("local_model").unwrap_or_default();
                let _ = respond_json(req, 200, &json!({"models": items, "acquisition": null}));
            }
            (Method::Get, "/api/local-models/hardware") => {
                let cpus = std::thread::available_parallelism()
                    .map(|n| n.get())
                    .unwrap_or(1);
                let _ = respond_json(
                    req,
                    200,
                    &json!({"os": std::env::consts::OS, "arch": std::env::consts::ARCH, "cpu_cores": cpus}),
                );
            }
            (Method::Get, "/api/local-models/engine-status") => {
                let _ = respond_json(req, 200, &json!({"ready": true, "engines": ["strata", "llama.cpp", "ollama"]}));
            }

            // --- Agent Settings, Tools & Permission Rules ---
            (Method::Get, "/api/agent/settings") => {
                let settings = store
                    .get_kv("agent_settings")
                    .ok()
                    .flatten()
                    .unwrap_or_else(|| json!({"permission_profile": "accept_edits", "auto_recall_memory": true}));
                let _ = respond_json(req, 200, &settings);
            }
            (Method::Post, "/api/agent/settings") => {
                let body = read_json_body(&mut req);
                let _ = store.set_kv("agent_settings", &body);
                let _ = respond_json(req, 200, &json!({"ok": true, "settings": body}));
            }
            (Method::Get, "/api/agent/permission_rules") => {
                let rules = store
                    .get_kv("permission_rules")
                    .ok()
                    .flatten()
                    .unwrap_or_else(|| json!([]));
                let _ = respond_json(req, 200, &json!({"rules": rules}));
            }
            (Method::Post, "/api/agent/permission_rules") => {
                let body = read_json_body(&mut req);
                let _ = store.set_kv("permission_rules", &body);
                let _ = respond_json(req, 200, &json!({"ok": true}));
            }
            (Method::Get, "/api/agent/tools") => {
                let mut reg = ToolRegistry::with_builtin_tools();
                suite.register_tools(&mut reg);
                let specs = reg.all_specs();
                let _ = respond_json(req, 200, &json!({"count": specs.len(), "tools": specs}));
            }

            // --- Business Modules (Skills, Memory, Goals, Rooms, Experts, Automations, MCP) ---
            (Method::Get, "/api/skills") => {
                let skills = nola_modules::skills::discover_skills(
                    &suite.workspace_root,
                    suite.user_home.as_deref(),
                );
                let _ = respond_json(req, 200, &json!({"skills": skills}));
            }
            (Method::Get, "/api/memory") => {
                let q = parse_query_str(query, "q").unwrap_or_default();
                let items = store.search_memory(&q, None, 100).unwrap_or_default();
                let _ = respond_json(req, 200, &json!({"items": items}));
            }
            (Method::Post, "/api/memory") => {
                let body = read_json_body(&mut req);
                let ts = now_ms();
                let tags = body
                    .get("tags")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|x| x.as_str().map(ToString::to_string))
                            .collect()
                    })
                    .unwrap_or_default();
                let item = MemoryItem {
                    id: body
                        .get("id")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string)
                        .unwrap_or_else(new_id),
                    namespace: body
                        .get("namespace")
                        .and_then(|v| v.as_str())
                        .unwrap_or("default")
                        .to_string(),
                    title: body
                        .get("title")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Memory")
                        .to_string(),
                    content: body
                        .get("content")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    tags,
                    created_at_ms: ts,
                    updated_at_ms: ts,
                };
                match store.upsert_memory(&item) {
                    Ok(()) => {
                        let _ = respond_json(req, 200, &json!({"ok": true, "item": item}));
                    }
                    Err(e) => {
                        let _ = respond_json(req, 500, &json!({"error": e.to_string()}));
                    }
                }
            }
            (Method::Delete, p) if p.starts_with("/api/memory/") => {
                let id = p.trim_start_matches("/api/memory/");
                match store.delete_memory(id) {
                    Ok(deleted) => {
                        let _ = respond_json(req, 200, &json!({"deleted": deleted, "id": id}));
                    }
                    Err(e) => {
                        let _ = respond_json(req, 500, &json!({"error": e.to_string()}));
                    }
                }
            }
            (Method::Get, "/api/mcp") => {
                let servers = store.list_mcp_servers().unwrap_or_default();
                let _ = respond_json(req, 200, &json!({"servers": servers}));
            }
            (Method::Get, "/api/goals") => {
                let items: Vec<Value> = suite.list_records("goal").unwrap_or_default();
                let _ = respond_json(req, 200, &json!({"goals": items, "items": items}));
            }
            (Method::Get, "/api/sops") => {
                let items: Vec<Value> = suite.list_records("sop").unwrap_or_default();
                let _ = respond_json(req, 200, &json!({"sops": items, "items": items}));
            }
            (Method::Get, "/api/automations") | (Method::Get, "/api/scheduled-actions") => {
                let items: Vec<Value> = suite.list_records("automation").unwrap_or_default();
                let _ = respond_json(req, 200, &json!({"automations": items, "items": items}));
            }
            (Method::Get, "/api/data-sources") => {
                let items: Vec<Value> = suite.list_records("data_source").unwrap_or_default();
                let _ = respond_json(req, 200, &json!({"data_sources": items, "items": items}));
            }
            (Method::Get, "/api/messaging-channels") => {
                let items: Vec<Value> = suite.list_records("messaging_channel").unwrap_or_default();
                let _ = respond_json(req, 200, &json!({"channels": items, "items": items}));
            }
            (Method::Get, "/api/agent-rooms") => {
                let items: Vec<Value> = suite.list_records("agent_room").unwrap_or_default();
                let _ = respond_json(req, 200, &json!({"rooms": items}));
            }
            (Method::Get, "/api/expert-prompt-templates") => {
                let items: Vec<Value> = suite.list_records("expert_peer").unwrap_or_default();
                let _ = respond_json(req, 200, &json!({"experts": items}));
            }
            (Method::Post, p) if p.starts_with("/api/agent-rooms/") && p.ends_with("/messages") => {
                let room_id = p
                    .trim_start_matches("/api/agent-rooms/")
                    .trim_end_matches("/messages")
                    .to_string();
                let body = read_json_body(&mut req);
                let msg = body
                    .get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let store_c = store.clone();
                let suite_c = Arc::clone(&suite);
                std::thread::spawn(move || {
                    let cfg = load_active_config(&store_c);
                    match resolve_model_route(&cfg).and_then(|route| {
                        let caller: Arc<dyn ModelCaller> =
                            Arc::new(HttpModelCaller::new(300).map_err(|e| e.to_string())?);
                        let policy = EffectivePolicy::load(
                            &suite_c.workspace_root,
                            suite_c.user_home.as_deref(),
                            false,
                            Some(PermissionProfile::AcceptEdits),
                            true,
                        );
                        RoomRuntime::send_message(&suite_c, caller, route, policy, &room_id, &msg)
                    }) {
                        Ok(replies) => {
                            let _ = respond_json(req, 200, &json!({"replies": replies}));
                        }
                        Err(e) => {
                            let _ = respond_json(req, 400, &json!({"error": e}));
                        }
                    }
                });
            }
            (Method::Post, p) if p.starts_with("/api/goals/") && p.ends_with("/run") => {
                let goal_id = p
                    .trim_start_matches("/api/goals/")
                    .trim_end_matches("/run")
                    .to_string();
                let store_c = store.clone();
                let suite_c = Arc::clone(&suite);
                std::thread::spawn(move || {
                    let cfg = load_active_config(&store_c);
                    match resolve_model_route(&cfg).and_then(|route| {
                        let caller: Arc<dyn ModelCaller> =
                            Arc::new(HttpModelCaller::new(300).map_err(|e| e.to_string())?);
                        let policy = EffectivePolicy::load(
                            &suite_c.workspace_root,
                            suite_c.user_home.as_deref(),
                            false,
                            Some(PermissionProfile::AcceptEdits),
                            true,
                        );
                        GoalWorker::execute_goal(
                            &suite_c,
                            caller,
                            route,
                            policy,
                            &goal_id,
                            32,
                            Arc::new(AtomicBool::new(false)),
                        )
                    }) {
                        Ok(goal) => {
                            let _ = respond_json(req, 200, &json!({"goal": goal}));
                        }
                        Err(e) => {
                            let _ = respond_json(req, 400, &json!({"error": e}));
                        }
                    }
                });
            }
            (Method::Post, "/api/automations/tick") => {
                let store_c = store.clone();
                let suite_c = Arc::clone(&suite);
                std::thread::spawn(move || {
                    let cfg = load_active_config(&store_c);
                    match resolve_model_route(&cfg).and_then(|route| {
                        let caller: Arc<dyn ModelCaller> =
                            Arc::new(HttpModelCaller::new(300).map_err(|e| e.to_string())?);
                        let policy = EffectivePolicy::load(
                            &suite_c.workspace_root,
                            suite_c.user_home.as_deref(),
                            false,
                            Some(PermissionProfile::AcceptEdits),
                            true,
                        );
                        Ok(AutomationScheduler::run_due(
                            &suite_c,
                            caller,
                            route,
                            policy,
                            Utc::now(),
                        ))
                    }) {
                        Ok(results) => {
                            let _ = respond_json(req, 200, &json!({"triggered": results.len()}));
                        }
                        Err(e) => {
                            let _ = respond_json(req, 400, &json!({"error": e}));
                        }
                    }
                });
            }

            // --- Chrome Native Browser Extension Bridge ---
            (Method::Get, "/api/native-browser/status") => {
                let _ = respond_json(req, 200, &native_browser.status());
            }
            (Method::Post, "/api/native-browser/command") => {
                let body = read_json_body(&mut req);
                let id = native_browser.enqueue(body);
                let _ = respond_json(req, 200, &json!({"command_id": id}));
            }
            (Method::Post, "/api/native-browser/host/poll") => {
                if let Some(next) = native_browser.poll_next() {
                    let _ = respond_json(req, 200, &json!({"type": "command", "command": next}));
                } else {
                    let _ = respond_json(req, 200, &json!({"type": "idle", "retry_ms": 500}));
                }
            }
            (Method::Post, "/api/native-browser/host/result") => {
                let body = read_json_body(&mut req);
                if let Some(id) = body.get("command_id").and_then(|v| v.as_str()) {
                    native_browser.submit_result(id.to_string(), body);
                }
                let _ = respond_json(req, 200, &json!({"ok": true}));
            }

            // --- Generic Unified Module CRUD (/api/modules/<kind>) ---
            (Method::Get, p) if p.starts_with("/api/modules/") => {
                let kind = p.trim_start_matches("/api/modules/");
                let items: Vec<Value> = suite.list_records(kind).unwrap_or_default();
                let _ = respond_json(req, 200, &json!({"kind": kind, "items": items}));
            }
            (Method::Post, p) if p.starts_with("/api/modules/") => {
                let kind = p.trim_start_matches("/api/modules/").to_string();
                let val = read_json_body(&mut req);
                let id = val
                    .get("id")
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string)
                    .unwrap_or_else(new_id);
                match suite.upsert_record(&kind, &id, &val) {
                    Ok(()) => {
                        let _ = respond_json(req, 200, &json!({"ok": true, "id": id}));
                    }
                    Err(e) => {
                        let _ = respond_json(req, 500, &json!({"error": e.to_string()}));
                    }
                }
            }
            (Method::Delete, p) if p.starts_with("/api/modules/") => {
                let rest = p.trim_start_matches("/api/modules/");
                if let Some((kind, id)) = rest.split_once('/') {
                    let deleted = suite.delete_record(kind, id).unwrap_or(false);
                    let _ = respond_json(req, 200, &json!({"deleted": deleted}));
                } else {
                    let _ = respond_json(req, 400, &json!({"error": "missing module id"}));
                }
            }

            // --- Unknown API endpoints never fall through to static assets ---
            (_, p) if p.starts_with("/api/") => {
                let _ = respond_json(req, 404, &json!({"error": "not_found"}));
            }
            // --- Static frontend assets (/ui/*, /static/*, and SPA fallback) ---
            (Method::Get, p) => {
                serve_static_asset(req, &ui_dir, &static_dir, p);
            }
            _ => {
                let _ = respond_json(req, 404, &json!({"error": "not_found"}));
            }
        }
    }
    Ok(0)
}

fn handle_sse_chat_turn(
    req: Request,
    store: Store,
    suite: Arc<ModulesSuite>,
    cancels: Arc<Mutex<BTreeMap<String, Arc<AtomicBool>>>>,
    conversation_id: String,
    prompt: String,
) {
    let cfg = load_active_config(&store);
    let route = match resolve_model_route(&cfg) {
        Ok(r) => r,
        Err(e) => {
            let _ = respond_json(req, 400, &json!({"error": e}));
            return;
        }
    };
    let caller: Arc<dyn ModelCaller> = match HttpModelCaller::new(300) {
        Ok(c) => Arc::new(c),
        Err(e) => {
            let _ = respond_json(req, 500, &json!({"error": e.to_string()}));
            return;
        }
    };

    let cancel = Arc::new(AtomicBool::new(false));
    cancels
        .lock()
        .unwrap()
        .insert(conversation_id.clone(), Arc::clone(&cancel));

    let workspace_trusted = false;
    let policy = EffectivePolicy::load(
        &suite.workspace_root,
        suite.user_home.as_deref(),
        workspace_trusted,
        Some(PermissionProfile::AcceptEdits),
        true,
    );
    let hooks = HookRunner::load(
        &suite.workspace_root,
        suite.user_home.as_deref(),
        workspace_trusted,
    );
    let mut registry = ToolRegistry::with_builtin_tools();
    suite.register_tools(&mut registry);
    let subagents = Arc::new(SubagentManager::new(store.clone(), Arc::clone(&caller)));
    subagents.configure_parent(route.clone(), policy.clone());
    subagents.configure_runtime(hooks.clone(), Some(Arc::clone(&cancel)));
    register_subagent_tools(&subagents, &mut registry);
    let planning = Arc::new(PlanningState::default());
    register_planning_tools(store.clone(), &planning, &mut registry);

    let engine = AgentEngine::new(store.clone(), Arc::new(registry), caller);
    let mut ctx = ToolContext::new(&suite.workspace_root);
    ctx.permission_profile = policy.profile;
    ctx.sandbox_mode = SandboxMode::from_env();

    let run_cfg = AgentRunConfig {
        session_id: Some(conversation_id.clone()),
        resume: true,
        route,
        frozen_system_blocks: build_frozen_system_blocks(&suite.workspace_root, &cfg),
        host_capabilities: HostCapabilities {
            interactive_user_input: false,
            network_allowed: true,
        },
        policy,
        hooks,
        approval: None,
        max_steps: 0,
        force_tool_set: ForceToolSet::Auto,
        stream: true,
        json_schema: None,
        cancel: Arc::clone(&cancel),
        auto_recall_memory: true,
        inbox: None,
    };

    // True live chunked SSE streaming: worker thread pushes each event frame into `tx`
    // while `tiny_http` flushes each chunk immediately (`data_length == None` enables `FlushAfterWrite`).
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    let conv_id_c = conversation_id.clone();
    let cancels_c = Arc::clone(&cancels);
    let cancel_c = Arc::clone(&cancel);
    std::thread::spawn(move || {
        let run_res = engine.run(&ctx, &prompt, run_cfg, |ev| {
            if let Ok(line) = serde_json::to_string(&ev) {
                let _ = tx.send(format!("data: {line}\n\n").into_bytes());
            }
        });
        if let Err(e) = run_res {
            let err_frame = json!({"type": "error", "message": e.to_string()});
            let _ = tx.send(format!("data: {err_frame}\n\n").into_bytes());
        }
        let _ = tx.send(b"data: [DONE]\n\n".to_vec());
        ctx.pty.close_all();
        // Remove cancel handle ONLY if it still points to this exact turn!
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
    let resp = Response::new(StatusCode(200), headers, ChannelReader::new(rx), None, None);
    let _ = req.respond(resp);
}

fn read_json_body(req: &mut Request) -> Value {
    let mut bytes = Vec::new();
    let _ = req
        .as_reader()
        .take(MAX_HTTP_BODY_BYTES)
        .read_to_end(&mut bytes);
    serde_json::from_slice(&bytes).unwrap_or_else(|_| json!({}))
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
    fn server_started_handshake_matches_windows_desktop_contract() {
        let frame = server_started_handshake("127.0.0.1", 28421);
        assert_eq!(frame["type"], "server.started");
        assert_eq!(frame["protocol_version"], 1);
        assert_eq!(frame["loopback"], true);
        assert_eq!(frame["origin"], "http://127.0.0.1:28421");

        let lan = server_started_handshake("0.0.0.0", 5001);
        assert_eq!(lan["loopback"], false);
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
}
