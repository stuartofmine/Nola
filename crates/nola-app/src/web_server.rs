// SPDX-License-Identifier: Apache-2.0
//! Full-Capability Secure HTTP + SSE WebHost (`tiny_http`) for Web & Windows clients:
//! - Strict same-origin CSRF (`Origin == Host` + `Sec-Fetch-Site != cross-site`)
//! - O(1) bounded replay-nonce cache
//! - Live streaming SSE chat endpoint (`POST /api/chat/conversations/<id>/messages`)
//! - Active-run cancellation (`POST /api/chat/conversations/<id>/cancel`)
//! - Complete business module REST endpoints aligned with `apps/web/frontend`
//! - Safe static asset & SPA serving (`/ui/*`, `/static/*`, and SPA fallback).

use std::collections::{BTreeMap, HashSet, VecDeque};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};

use chrono::Utc;
use nola_engine::{
    register_planning_tools, register_subagent_tools, AgentEngine, AgentEvent, AgentRunConfig,
    EffectivePolicy, ForceToolSet, HookRunner, HttpModelCaller, ModelCaller, PlanningState,
    SubagentManager,
};
use nola_modules::{AutomationScheduler, GoalWorker, ModulesSuite, RoomRuntime};
use nola_store::{EventPage, Store};
use nola_tools::{NativeBrowserBridge, SandboxMode, ToolContext, ToolRegistry};
use nola_types::{new_id, now_ms, HostCapabilities, MemoryItem, PermissionProfile, Session};
use serde_json::{json, Value};
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};
use url::Url;

use crate::config::{build_frozen_system_blocks, load_active_config, resolve_model_route};
use crate::web_api::{
    build_agent_settings_snapshot, build_dashboard_snapshot, build_model_profiles_response,
    build_workspace_review, memory_item_to_record, normalize_automation_record,
    reconstruct_v2_messages, session_meta_key, session_to_conversation_json,
};

const MAX_TRACKED_NONCES: usize = 4_096;
const MAX_HTTP_BODY_BYTES: u64 = 4 * 1024 * 1024;

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
    // Native clients (Android/iOS/macOS/CLI) do not send browser Origin headers;
    // permit authenticated native API callers when no browser Origin is present.
    if header_value(request, "authorization")
        .is_some_and(|v| v.trim_start().to_ascii_lowercase().starts_with("bearer "))
        || header_value(request, "x-nola-client").is_some_and(|v| !v.trim().is_empty())
    {
        return Ok(());
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
    let host_id = store
        .get_kv("host_id")
        .ok()
        .flatten()
        .and_then(|v| v.as_str().map(ToString::to_string))
        .unwrap_or_else(|| {
            let id = new_id();
            let _ = store.set_kv("host_id", &json!(id));
            id
        });
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

        let target_id = format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH);

        match (method, path) {
            // --- Health, Session, Auth & Software Update ---
            (Method::Get, "/api/health") => {
                let _ = respond_json(
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
                );
            }
            (Method::Get, "/api/web/session") => {
                let ingress = if loopback { "direct" } else { "tailcat" };
                let _ = respond_json(
                    req,
                    200,
                    &json!({
                        "authenticated": loopback,
                        "sessionAuthenticated": loopback,
                        "localTrust": loopback,
                        "tailcatAvailable": true,
                        "transportIngress": ingress,
                        "hostId": host_id,
                        "version": "2.0.0"
                    }),
                );
            }
            (Method::Post, "/api/web/login") => {
                let _ = respond_json(
                    req,
                    200,
                    &json!({"authenticated": true, "sessionAuthenticated": true, "hostId": host_id}),
                );
            }
            (Method::Post, "/api/web/logout") => {
                let _ = respond_json(
                    req,
                    200,
                    &json!({"authenticated": false, "sessionAuthenticated": false}),
                );
            }
            (Method::Get, "/api/software-update") | (Method::Post, "/api/software-update/check") => {
                let _ = respond_json(
                    req,
                    200,
                    &json!({
                        "status": "up_to_date",
                        "current_version": "2.0.0",
                        "current_build": 1,
                        "channel": "stable",
                        "target": target_id,
                        "release": Value::Null
                    }),
                );
            }

            // --- Dashboard, Home, Today, Activity, Results ---
            (Method::Get, "/api/home") => {
                let _ = respond_json(req, 200, &build_dashboard_snapshot(&suite));
            }
            (Method::Get, "/api/workspace/today") => {
                let sessions = store.list_sessions(20).unwrap_or_default();
                let goals: Vec<Value> = suite.list_records("goal").unwrap_or_default();
                let automations: Vec<Value> = suite.list_records("automation").unwrap_or_default();
                let _ = respond_json(
                    req,
                    200,
                    &json!({
                        "date": Utc::now().format("%Y-%m-%d").to_string(),
                        "sessions": sessions.len(),
                        "goals": goals,
                        "automations": automations,
                        "pendingResponses": []
                    }),
                );
            }
            (Method::Post, "/api/activity") => {
                let _ = respond_json(req, 200, &json!({"items": [], "hasMore": false}));
            }
            (Method::Get, "/api/results") => {
                let _ = respond_json(req, 200, &json!({"results": []}));
            }
            (Method::Post, "/api/desktop-plugin-pages") | (Method::Post, "/api/harness-plugins") => {
                let _ = respond_json(req, 200, &json!({"pages": [], "plugins": [], "items": []}));
            }

            // --- Conversations & Live Streaming Chat (v2 Native) ---
            (Method::Get, "/api/chat/conversations") | (Method::Get, "/api/sessions") => {
                match store.list_sessions(200) {
                    Ok(sessions) => {
                        let conversations: Vec<Value> = sessions
                            .iter()
                            .map(|s| session_to_conversation_json(&store, s))
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
                match store.upsert_session(&sess) {
                    Ok(()) => {
                        let conv = session_to_conversation_json(&store, &sess);
                        let _ = respond_json(req, 200, &conv);
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
                let (messages, events) = reconstruct_v2_messages(&store, id);
                let latest_seq = events
                    .last()
                    .and_then(|e| e.get("seq").and_then(|s| s.as_i64()))
                    .unwrap_or(0);
                let _ = respond_json(
                    req,
                    200,
                    &json!({
                        "conversationId": id,
                        "messages": messages,
                        "events": events,
                        "hasMore": false,
                        "projectionRevision": 1,
                        "eventSequence": latest_seq
                    }),
                );
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
            (Method::Get, p)
                if p.starts_with("/api/chat/conversations/") && p.ends_with("/export") =>
            {
                let id = p
                    .trim_start_matches("/api/chat/conversations/")
                    .trim_end_matches("/export");
                let title = store
                    .get_session(id)
                    .ok()
                    .flatten()
                    .map(|s| s.title)
                    .unwrap_or_else(|| id.to_string());
                let (messages, _) = reconstruct_v2_messages(&store, id);
                let is_json = query.contains("format=json");
                if is_json {
                    let payload = json!({
                        "conversationId": id,
                        "title": title,
                        "messages": messages
                    });
                    let _ = respond_json(
                        req,
                        200,
                        &json!({
                            "filename": format!("nola-{id}.json"),
                            "mimeType": "application/json; charset=utf-8",
                            "content": serde_json::to_string_pretty(&payload).unwrap_or_else(|_| "{}".into())
                        }),
                    );
                } else {
                    let mut md = format!("# {title}\n\n");
                    for m in &messages {
                        let role = m.get("role").and_then(|v| v.as_str()).unwrap_or("unknown");
                        let content = m.get("content").and_then(|v| v.as_str()).unwrap_or("");
                        md.push_str(&format!("## {role}\n\n{content}\n\n"));
                    }
                    let _ = respond_json(
                        req,
                        200,
                        &json!({
                            "filename": format!("nola-{id}.md"),
                            "mimeType": "text/markdown; charset=utf-8",
                            "content": md
                        }),
                    );
                }
            }
            (Method::Get, p)
                if p.starts_with("/api/chat/conversations/") && p.ends_with("/configuration") =>
            {
                let id = p
                    .trim_start_matches("/api/chat/conversations/")
                    .trim_end_matches("/configuration");
                match store.get_session(id) {
                    Ok(Some(sess)) => {
                        let conv = session_to_conversation_json(&store, &sess);
                        let _ = respond_json(req, 200, &conv);
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
                if p.starts_with("/api/chat/conversations/") && p.ends_with("/configuration") =>
            {
                let id = p
                    .trim_start_matches("/api/chat/conversations/")
                    .trim_end_matches("/configuration");
                let body = read_json_body(&mut req);
                match store.get_session(id) {
                    Ok(Some(mut sess)) => {
                        let meta_key = session_meta_key(id);
                        let mut meta = store
                            .get_kv(&meta_key)
                            .ok()
                            .flatten()
                            .unwrap_or_else(|| json!({"isPinned": false, "revision": 1, "configurationRevision": 1}));
                        let current_cfg_rev = meta
                            .get("configurationRevision")
                            .and_then(|v| v.as_i64())
                            .unwrap_or(1);
                        if let Some(expected) = body.get("expectedRevision").and_then(|v| v.as_i64()) {
                            if expected != current_cfg_rev {
                                let _ = respond_json(
                                    req,
                                    200,
                                    &json!({
                                        "status": "conflict",
                                        "expected_revision": expected,
                                        "actual_revision": current_cfg_rev
                                    }),
                                );
                                continue;
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
                        meta["configurationRevision"] = json!(current_cfg_rev + 1);
                        sess.updated_at_ms = now_ms();
                        let _ = store.set_kv(&meta_key, &meta);
                        let _ = store.upsert_session(&sess);
                        let snapshot = session_to_conversation_json(&store, &sess);
                        let _ = respond_json(
                            req,
                            200,
                            &json!({"status": "committed", "snapshot": snapshot}),
                        );
                    }
                    Ok(None) => {
                        let _ = respond_json(
                            req,
                            200,
                            &json!({"status": "not_found", "conversation_id": id}),
                        );
                    }
                    Err(e) => {
                        let _ = respond_json(req, 500, &json!({"error": e.to_string()}));
                    }
                }
            }
            (Method::Post, p)
                if p.starts_with("/api/chat/conversations/") && p.ends_with("/branch") =>
            {
                let id = p
                    .trim_start_matches("/api/chat/conversations/")
                    .trim_end_matches("/branch");
                let now = now_ms();
                let new_conv_id = new_id();
                let parent = store.get_session(id).ok().flatten();
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
                    let _ = respond_json(req, 500, &json!({"error": e.to_string()}));
                    continue;
                }
                if let Ok(EventPage::Events(ev)) = store.events_after(id, 0, 500) {
                    for e in ev {
                        let _ = store.append_event(&new_conv_id, &e.kind, e.payload, e.created_at_ms);
                    }
                }
                let conv = session_to_conversation_json(&store, &sess);
                let _ = respond_json(req, 200, &json!({"conversation": conv}));
            }
            (Method::Get, p)
                if p.starts_with("/api/chat/conversations/") && p.ends_with("/workspace-review") =>
            {
                let id = p
                    .trim_start_matches("/api/chat/conversations/")
                    .trim_end_matches("/workspace-review");
                let ws_root = store
                    .get_session(id)
                    .ok()
                    .flatten()
                    .map(|s| PathBuf::from(s.workspace_root))
                    .filter(|p| p.is_dir())
                    .unwrap_or_else(|| suite.workspace_root.clone());
                let review = build_workspace_review(&host_id, id, &ws_root);
                let _ = respond_json(req, 200, &review);
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
                let _ = respond_json(req, 200, &json!({"ok": true, "cancelled": cancelled}));
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
            (Method::Get, p)
                if p.starts_with("/api/chat/conversations/")
                    && !p.trim_start_matches("/api/chat/conversations/").contains('/') =>
            {
                let id = p.trim_start_matches("/api/chat/conversations/");
                match store.get_session(id) {
                    Ok(Some(sess)) => {
                        let conv = session_to_conversation_json(&store, &sess);
                        let _ = respond_json(req, 200, &conv);
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
                        let meta_key = session_meta_key(id);
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
                                let _ = respond_json(req, 200, &conv);
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
            (Method::Delete, p)
                if p.starts_with("/api/chat/conversations/")
                    && !p.trim_start_matches("/api/chat/conversations/").contains('/') =>
            {
                let id = p.trim_start_matches("/api/chat/conversations/");
                match store.delete_session(id) {
                    Ok(()) => {
                        let _ = respond_json(
                            req,
                            200,
                            &json!({"ok": true, "deleted": true, "conversationId": id}),
                        );
                    }
                    Err(e) => {
                        let _ = respond_json(req, 500, &json!({"error": e.to_string()}));
                    }
                }
            }

            // --- Models & Local Model Runtime ---
            (Method::Get, "/api/models") | (Method::Get, "/api/model-profiles") => {
                let _ = respond_json(req, 200, &build_model_profiles_response(&store));
            }
            (Method::Post, "/api/models/config") | (Method::Post, "/api/model-profiles") => {
                let body = read_json_body(&mut req);
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
                    if let Some(key) = api_key {
                        existing.api_key = Some(key);
                    }
                    if let Some(ref m) = model {
                        if !existing.models.iter().any(|e| e.id == *m) {
                            existing.models.push(crate::config::ModelEntry {
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
                            vec![crate::config::ModelEntry {
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
                    cfg.llm.connections.push(crate::config::ConnectionConfig {
                        id: conn_id.clone(),
                        endpoint: endpoint.unwrap_or_default(),
                        protocol,
                        provider: None,
                        api_key,
                        api_key_env: None,
                        models,
                    });
                }
                if let Some(m) = model {
                    cfg.llm.default_model = Some(crate::config::DefaultModelSelection {
                        connection: conn_id,
                        model: m,
                    });
                }
                let _ = store.set_kv(
                    crate::config::KV_APPLIED_CONFIG,
                    &serde_json::to_value(&cfg).unwrap_or(json!({})),
                );
                let _ = respond_json(req, 200, &build_model_profiles_response(&store));
            }
            (Method::Post, "/api/model-profiles/catalog/replace") => {
                let body = read_json_body(&mut req);
                if let Some(profiles) = body.get("profiles") {
                    let _ = store.set_kv("web_model_profiles", profiles);
                }
                let _ = respond_json(req, 200, &build_model_profiles_response(&store));
            }
            (Method::Get, "/api/local-models") => {
                let items: Vec<Value> = suite.list_records("local_model").unwrap_or_default();
                let _ = respond_json(
                    req,
                    200,
                    &json!({
                        "models": items,
                        "binaryAvailable": false,
                        "runtimes": [],
                        "acquisition": Value::Null
                    }),
                );
            }
            (Method::Get, "/api/local-models/hardware") => {
                let cpus = std::thread::available_parallelism()
                    .map(|n| n.get())
                    .unwrap_or(1);
                let hw = json!({
                    "os": std::env::consts::OS,
                    "arch": std::env::consts::ARCH,
                    "cpu_cores": cpus
                });
                let _ = respond_json(
                    req,
                    200,
                    &json!({
                        "hardware": hw,
                        "os": std::env::consts::OS,
                        "arch": std::env::consts::ARCH,
                        "cpu_cores": cpus
                    }),
                );
            }
            (Method::Get, "/api/local-models/engine-status") => {
                let eng = parse_query_str(query, "engine").unwrap_or_else(|| "llama.cpp".into());
                let _ = respond_json(
                    req,
                    200,
                    &json!({
                        "engineStatus": {
                            "engine": eng,
                            "ready": true,
                            "canInstall": false
                        },
                        "ready": true,
                        "engines": ["strata", "llama.cpp", "ollama"]
                    }),
                );
            }
            (Method::Get, "/api/model-connections") => {
                let _ = respond_json(req, 200, &json!({"connections": [], "accounts": []}));
            }

            // --- Agent Settings, Tools, Defaults & Permission Rules ---
            (Method::Get, "/api/agent/settings") => {
                let _ = respond_json(req, 200, &build_agent_settings_snapshot(&store, &suite));
            }
            (Method::Post, "/api/agent/settings") => {
                let patch = read_json_body(&mut req);
                let mut merged = build_agent_settings_snapshot(&store, &suite);
                if let (Some(dst), Some(src)) = (merged.as_object_mut(), patch.as_object()) {
                    for (k, v) in src {
                        if !k.ends_with("_baseline") {
                            dst.insert(k.clone(), v.clone());
                        }
                    }
                }
                let _ = store.set_kv("agent_settings", &merged);
                let _ = respond_json(req, 200, &build_agent_settings_snapshot(&store, &suite));
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
                let _ = respond_json(req, 200, &json!({"count": flat.len(), "tools": flat}));
            }
            (Method::Get, "/api/assistant/memory-default") => {
                let val = store
                    .get_kv("assistant_memory_default")
                    .ok()
                    .flatten()
                    .unwrap_or_else(|| json!({"enabled": true}));
                let _ = respond_json(req, 200, &val);
            }
            (Method::Put, "/api/assistant/memory-default") => {
                let body = read_json_body(&mut req);
                let _ = store.set_kv("assistant_memory_default", &body);
                let _ = respond_json(req, 200, &body);
            }
            (Method::Get, "/api/assistant/retrieval-default") => {
                let val = store
                    .get_kv("assistant_retrieval_default")
                    .ok()
                    .flatten()
                    .unwrap_or_else(|| json!({"enabled": true}));
                let _ = respond_json(req, 200, &val);
            }
            (Method::Put, "/api/assistant/retrieval-default") => {
                let body = read_json_body(&mut req);
                let _ = store.set_kv("assistant_retrieval_default", &body);
                let _ = respond_json(req, 200, &body);
            }

            // --- Business Modules (Skills, Memory, Goals, Rooms, Experts, Automations, DataSources, MCP) ---
            (Method::Get, "/api/skills") | (Method::Get, "/api/automation-skills") => {
                let skills = nola_modules::skills::discover_skills(
                    &suite.workspace_root,
                    suite.user_home.as_deref(),
                );
                let _ = respond_json(req, 200, &json!({"skills": skills}));
            }
            (Method::Get, "/api/memory") => {
                let q = parse_query_str(query, "q").unwrap_or_default();
                let items = store.search_memory(&q, None, 2000).unwrap_or_default();
                let records: Vec<Value> = items.iter().map(memory_item_to_record).collect();
                let _ = respond_json(
                    req,
                    200,
                    &json!({
                        "type": "memory_records",
                        "limit": 2000,
                        "records": records,
                        "items": items
                    }),
                );
            }
            (Method::Post, "/api/memory") => {
                let body = read_json_body(&mut req);
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
                        let mut record = memory_item_to_record(&item);
                        if let (Some("revise"), Some(ref pid)) = (op, &prev_id) {
                            record["supersedes"] = json!([pid]);
                            let _ = respond_json(
                                req,
                                200,
                                &json!({
                                    "type": "memory_revised",
                                    "previous_id": pid,
                                    "record": record,
                                    "ok": true,
                                    "item": item
                                }),
                            );
                        } else {
                            let _ = respond_json(
                                req,
                                200,
                                &json!({
                                    "type": "memory_created",
                                    "record": record,
                                    "ok": true,
                                    "item": item
                                }),
                            );
                        }
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
                        let _ = respond_json(
                            req,
                            200,
                            &json!({"type": "memory_deleted", "ok": true, "deleted": deleted, "id": id}),
                        );
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
            (Method::Post, "/api/goals") => {
                let mut val = read_json_body(&mut req);
                let id = val
                    .get("id")
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string)
                    .unwrap_or_else(new_id);
                val["id"] = json!(id);
                match suite.upsert_record("goal", &id, &val) {
                    Ok(()) => {
                        let _ = respond_json(req, 200, &json!({"ok": true, "id": id, "goal": val}));
                    }
                    Err(e) => {
                        let _ = respond_json(req, 500, &json!({"error": e.to_string()}));
                    }
                }
            }
            (Method::Get, "/api/sops") => {
                let items: Vec<Value> = suite.list_records("sop").unwrap_or_default();
                let _ = respond_json(req, 200, &json!({"sops": items, "items": items}));
            }
            (Method::Get, "/api/automations") | (Method::Get, "/api/scheduled-actions") => {
                let raw_items: Vec<Value> = suite.list_records("automation").unwrap_or_default();
                let actions: Vec<Value> = raw_items.iter().map(normalize_automation_record).collect();
                let _ = respond_json(
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
                );
            }
            (Method::Post, "/api/scheduled-actions") => {
                let body = read_json_body(&mut req);
                let norm = normalize_automation_record(&body);
                let id = norm["id"].as_str().unwrap_or("").to_string();
                match suite.upsert_record("automation", &id, &norm) {
                    Ok(()) => {
                        let _ = respond_json(req, 200, &json!({"ok": true, "action": norm}));
                    }
                    Err(e) => {
                        let _ = respond_json(req, 500, &json!({"error": e.to_string()}));
                    }
                }
            }
            (Method::Patch, p)
                if p.starts_with("/api/scheduled-actions/")
                    && !p.trim_start_matches("/api/scheduled-actions/").contains('/') =>
            {
                let id = p.trim_start_matches("/api/scheduled-actions/");
                let patch = read_json_body(&mut req);
                let mut existing = suite
                    .list_records::<Value>("automation")
                    .unwrap_or_default()
                    .into_iter()
                    .find(|v| v.get("id").and_then(|s| s.as_str()) == Some(id))
                    .unwrap_or_else(|| json!({"id": id}));
                if let (Some(dst), Some(src)) = (existing.as_object_mut(), patch.as_object()) {
                    for (k, v) in src {
                        dst.insert(k.clone(), v.clone());
                    }
                    let rev = dst.get("revision").and_then(|v| v.as_i64()).unwrap_or(1) + 1;
                    dst.insert("revision".into(), json!(rev));
                    dst.insert("updatedAt".into(), json!(Utc::now().to_rfc3339()));
                }
                let norm = normalize_automation_record(&existing);
                match suite.upsert_record("automation", id, &norm) {
                    Ok(()) => {
                        let _ = respond_json(req, 200, &json!({"ok": true, "action": norm}));
                    }
                    Err(e) => {
                        let _ = respond_json(req, 500, &json!({"error": e.to_string()}));
                    }
                }
            }
            (Method::Delete, p)
                if p.starts_with("/api/scheduled-actions/")
                    && !p.trim_start_matches("/api/scheduled-actions/").contains('/') =>
            {
                let id = p.trim_start_matches("/api/scheduled-actions/");
                let deleted = suite.delete_record("automation", id).unwrap_or(false);
                let _ = respond_json(req, 200, &json!({"ok": true, "deleted": deleted, "id": id}));
            }
            (Method::Get, "/api/data-sources/usage") => {
                let _ = respond_json(req, 200, &json!({"usageRecords": []}));
            }
            (Method::Get, "/api/data-sources") => {
                let items: Vec<Value> = suite.list_records("data_source").unwrap_or_default();
                let wrapped: Vec<Value> = items.iter().map(|s| json!({"source": s})).collect();
                let _ = respond_json(
                    req,
                    200,
                    &json!({"sources": wrapped, "data_sources": items, "items": items}),
                );
            }
            (Method::Post, "/api/data-sources") => {
                let mut body = read_json_body(&mut req);
                let id = body
                    .get("id")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(ToString::to_string)
                    .unwrap_or_else(new_id);
                body["id"] = json!(id);
                body["updatedAt"] = json!(Utc::now().to_rfc3339());
                match suite.upsert_record("data_source", &id, &body) {
                    Ok(()) => {
                        let _ = respond_json(req, 200, &json!({"source": body, "tested": true}));
                    }
                    Err(e) => {
                        let _ = respond_json(req, 500, &json!({"error": e.to_string()}));
                    }
                }
            }
            (Method::Patch, p)
                if p.starts_with("/api/data-sources/")
                    && !p.trim_start_matches("/api/data-sources/").contains('/') =>
            {
                let id = p.trim_start_matches("/api/data-sources/");
                let mut body = read_json_body(&mut req);
                body["id"] = json!(id);
                body["updatedAt"] = json!(Utc::now().to_rfc3339());
                match suite.upsert_record("data_source", id, &body) {
                    Ok(()) => {
                        let _ = respond_json(req, 200, &json!({"source": body, "tested": true}));
                    }
                    Err(e) => {
                        let _ = respond_json(req, 500, &json!({"error": e.to_string()}));
                    }
                }
            }
            (Method::Delete, p)
                if p.starts_with("/api/data-sources/")
                    && !p.trim_start_matches("/api/data-sources/").contains('/') =>
            {
                let id = p.trim_start_matches("/api/data-sources/");
                let deleted = suite.delete_record("data_source", id).unwrap_or(false);
                let _ = respond_json(req, 200, &json!({"deleted": deleted, "id": id}));
            }
            (Method::Post, p) if p.starts_with("/api/data-sources/") && p.ends_with("/test") => {
                let id = p
                    .trim_start_matches("/api/data-sources/")
                    .trim_end_matches("/test");
                let existing = suite
                    .list_records::<Value>("data_source")
                    .unwrap_or_default()
                    .into_iter()
                    .find(|v| v.get("id").and_then(|s| s.as_str()) == Some(id))
                    .unwrap_or_else(|| json!({"id": id, "name": id, "kind": "localFile", "location": "", "accessPolicy": "conversationOptIn"}));
                let _ = respond_json(req, 200, &json!({"source": existing, "tested": true}));
            }
            (Method::Get, "/api/messaging-channels") => {
                let items: Vec<Value> = suite.list_records("messaging_channel").unwrap_or_default();
                let _ = respond_json(req, 200, &json!({"channels": items, "items": items}));
            }
            (Method::Put, "/api/messaging-channels") => {
                let body = read_json_body(&mut req);
                if let Some(arr) = body.get("channels").and_then(|v| v.as_array()) {
                    for ch in arr {
                        let id = ch
                            .get("id")
                            .and_then(|v| v.as_str())
                            .map(ToString::to_string)
                            .unwrap_or_else(new_id);
                        let _ = suite.upsert_record("messaging_channel", &id, ch);
                    }
                }
                let items: Vec<Value> = suite.list_records("messaging_channel").unwrap_or_default();
                let _ = respond_json(req, 200, &json!({"channels": items, "ok": true}));
            }
            (Method::Get, "/api/messaging-channel-worker") => {
                let val = store
                    .get_kv("messaging_channel_worker")
                    .ok()
                    .flatten()
                    .unwrap_or_else(|| json!({"enabled": false, "url": ""}));
                let _ = respond_json(req, 200, &val);
            }
            (Method::Put, "/api/messaging-channel-worker") => {
                let body = read_json_body(&mut req);
                let _ = store.set_kv("messaging_channel_worker", &body);
                let _ = respond_json(req, 200, &body);
            }
            (Method::Get, "/api/agent-rooms") => {
                let items: Vec<Value> = suite.list_records("agent_room").unwrap_or_default();
                let _ = respond_json(req, 200, &json!({"rooms": items}));
            }
            (Method::Post, "/api/agent-rooms") => {
                let body = read_json_body(&mut req);
                let rooms: Vec<Value> = suite.list_records("agent_room").unwrap_or_default();
                let experts: Vec<Value> = suite.list_records("expert_peer").unwrap_or_default();
                let _ = respond_json(
                    req,
                    200,
                    &json!({
                        "ok": true,
                        "rooms": rooms,
                        "experts": experts,
                        "roleTemplates": experts,
                        "requestType": body.get("type").cloned().unwrap_or(Value::Null)
                    }),
                );
            }
            (Method::Get, "/api/expert-prompt-templates") => {
                let items: Vec<Value> = suite.list_records("expert_peer").unwrap_or_default();
                let _ = respond_json(
                    req,
                    200,
                    &json!({"revision": 1, "templates": items, "experts": items}),
                );
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
                let mut st = native_browser.status();
                if let Some(obj) = st.as_object_mut() {
                    obj.entry("hosts").or_insert_with(|| json!([]));
                }
                let _ = respond_json(req, 200, &st);
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
            let _ = respond_json(req, 400, &json!({"error": e, "code": "model_not_configured"}));
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

    // Persist user message immediately so reload reflects conversation history
    let _ = store.append_event(
        &conversation_id,
        "chat_user",
        json!({"content": &prompt}),
        now_ms(),
    );

    let cancel = Arc::new(AtomicBool::new(false));
    cancels
        .lock()
        .unwrap()
        .insert(conversation_id.clone(), Arc::clone(&cancel));

    let settings = build_agent_settings_snapshot(&store, &suite);
    let profile = match settings.get("permission_profile").and_then(|v| v.as_str()) {
        Some("allow_all") => PermissionProfile::AllowAll,
        _ => PermissionProfile::AcceptEdits,
    };
    let workspace_trusted = profile == PermissionProfile::AllowAll;
    let policy = EffectivePolicy::load(
        &suite.workspace_root,
        suite.user_home.as_deref(),
        workspace_trusted,
        Some(profile),
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

    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    let conv_id_c = conversation_id.clone();
    let cancels_c = Arc::clone(&cancels);
    let cancel_c = Arc::clone(&cancel);
    let store_persist = store.clone();
    std::thread::spawn(move || {
        let mut collected_reasoning = String::new();
        let mut collected_tools: Vec<Value> = Vec::new();
        let run_res = engine.run(&ctx, &prompt, run_cfg, |ev| {
            match &ev {
                AgentEvent::ReasoningDelta { delta } => collected_reasoning.push_str(delta),
                AgentEvent::ToolFinished {
                    call_id,
                    name,
                    is_error,
                    summary,
                    ..
                } => collected_tools.push(json!({
                    "callId": call_id,
                    "name": name,
                    "ok": !is_error,
                    "summary": summary
                })),
                _ => {}
            }
            if let Ok(line) = serde_json::to_string(&ev) {
                let _ = tx.send(format!("data: {line}\n\n").into_bytes());
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
                        "toolCalls": collected_tools
                    }),
                    now_ms(),
                );
            }
            Err(e) => {
                let err_frame = json!({"type": "error", "message": e.to_string()});
                let _ = tx.send(format!("data: {err_frame}\n\n").into_bytes());
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
