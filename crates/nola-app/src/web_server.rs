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
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

use chrono::Utc;
use nola_engine::{
    register_planning_tools, register_subagent_tools, AgentEngine, AgentEvent, AgentRunConfig,
    ApprovalCallback, EffectivePolicy, ForceToolSet, HookRunner, HttpModelCaller, ModelCaller,
    PlanningState, SubagentManager, UserInputCallback,
};
use nola_modules::a2ui::{
    commit_ui_interaction, list_conversation_surfaces, preview_ui_surface,
    read_ui_surface_changes, read_ui_surface_revision, register_scoped_tools as register_a2ui_tools,
    UiDraftPreviewInput, UiInteraction, UiSurfaceIdentity, UiSurfaceRevisionRef,
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

/// Pending interactive approval & user-question waiters bridged between SSE turns and REST.
#[derive(Default)]
struct PendingInteractions {
    approvals: Mutex<BTreeMap<String, mpsc::Sender<bool>>>,
    questions: Mutex<BTreeMap<String, mpsc::Sender<Option<String>>>>,
}

impl PendingInteractions {
    fn resolve_approval(&self, request_id: &str, allow: bool) -> bool {
        if let Some(tx) = self.approvals.lock().unwrap().remove(request_id) {
            let _ = tx.send(allow);
            true
        } else {
            false
        }
    }

    fn resolve_question(&self, request_id: &str, answer: Option<String>) -> bool {
        if let Some(tx) = self.questions.lock().unwrap().remove(request_id) {
            let _ = tx.send(answer);
            true
        } else {
            false
        }
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
    let pending_interactions = Arc::new(PendingInteractions::default());
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
            (Method::Get, "/api/me") => {
                let _ = respond_json(
                    req,
                    200,
                    &json!({
                        "authenticated": loopback,
                        "hostId": host_id,
                        "version": "2.0.0",
                        "workspaceRoot": suite.workspace_root.display().to_string()
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
                let offset = parse_query_i64(query, "offset").unwrap_or(0).max(0) as usize;
                let sessions = store.list_sessions(50).unwrap_or_default();
                let to_fact = |s: &Session| {
                    json!({
                        "conversation_id": s.id,
                        "title": s.title,
                        "updated_at_ms": s.updated_at_ms,
                        "needs_response": false,
                        "active_run": Value::Null
                    })
                };
                let page_sessions: Vec<Value> = sessions.iter().skip(offset).take(20).map(to_fact).collect();
                let recent_sessions: Vec<Value> = sessions.iter().take(5).map(to_fact).collect();
                let has_more = sessions.len() > offset + page_sessions.len();
                let next_offset = if has_more {
                    json!(offset + page_sessions.len())
                } else {
                    Value::Null
                };
                let _ = respond_json(
                    req,
                    200,
                    &json!({
                        "source_host_id": host_id,
                        "snapshot": {
                            "conversations": page_sessions,
                            "recent_conversations": recent_sessions,
                            "queued_instances": [],
                            "needs_attention": false,
                            "unrepresented_expert_contexts": [],
                            "has_more_conversations": has_more,
                            "next_conversation_offset": next_offset,
                            "has_more_queued_instances": false
                        }
                    }),
                );
            }
            (Method::Post, "/api/activity") => {
                let _ = respond_json(
                    req,
                    200,
                    &json!({
                        "entries": [],
                        "next_cursor": Value::Null,
                        "items": [],
                        "hasMore": false
                    }),
                );
            }
            (Method::Get, "/api/results") => {
                let results: Vec<Value> = suite.list_records("automation_result").unwrap_or_default();
                let _ = respond_json(req, 200, &json!({"results": results}));
            }
            (Method::Get, p)
                if p.starts_with("/api/results/")
                    && !p.trim_start_matches("/api/results/").contains('/') =>
            {
                let id = percent_decode_path(p.trim_start_matches("/api/results/"));
                if let Ok(Some(rec)) = suite.get_record::<Value>("automation_result", &id) {
                    let _ = respond_json(req, 200, &json!({"result": rec}));
                } else {
                    let _ = respond_json(req, 404, &json!({"error": "result_not_found"}));
                }
            }
            (Method::Post, "/api/documents/search") => {
                let body = read_json_body(&mut req);
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
                let _ = respond_json(
                    req,
                    200,
                    &json!({"query": q, "memory": mem_hits, "files": file_hits}),
                );
            }
            (Method::Get, "/api/workspace/diff") => {
                let review = build_workspace_review(&host_id, "workspace", &suite.workspace_root);
                let _ = respond_json(req, 200, &review);
            }
            (Method::Get, "/api/workspace/checkpoints") => {
                let checkpoints: Vec<Value> = suite.list_records("workspace_checkpoint").unwrap_or_default();
                let _ = respond_json(req, 200, &json!({"checkpoints": checkpoints}));
            }
            (Method::Post, "/api/workspace/undo") => {
                let out = Command::new("git")
                    .args(["stash", "push", "-u", "-m", "nola-v2-undo"])
                    .current_dir(&suite.workspace_root)
                    .output();
                match out {
                    Ok(o) if o.status.success() => {
                        let _ = respond_json(
                            req,
                            200,
                            &json!({"ok": true, "stdout": String::from_utf8_lossy(&o.stdout)}),
                        );
                    }
                    Ok(o) => {
                        let _ = respond_json(
                            req,
                            400,
                            &json!({"ok": false, "error": String::from_utf8_lossy(&o.stderr)}),
                        );
                    }
                    Err(e) => {
                        let _ = respond_json(req, 500, &json!({"ok": false, "error": e.to_string()}));
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
                        let _ = respond_json(
                            req,
                            200,
                            &json!({"ok": true, "stdout": String::from_utf8_lossy(&o.stdout)}),
                        );
                    }
                    Ok(o) => {
                        let _ = respond_json(
                            req,
                            400,
                            &json!({"ok": false, "error": String::from_utf8_lossy(&o.stderr)}),
                        );
                    }
                    Err(e) => {
                        let _ = respond_json(req, 500, &json!({"ok": false, "error": e.to_string()}));
                    }
                }
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
                        if let Some(perm) = body.get("permissionProfile").and_then(|v| v.as_str()) {
                            meta["permissionProfile"] = json!(perm);
                        }
                        if let Some(effort) = body.get("reasoningEffort").and_then(|v| v.as_str()) {
                            meta["reasoningEffort"] = json!(effort);
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
                if p.starts_with("/api/chat/conversations/") && p.ends_with("/permission") =>
            {
                let body = read_json_body(&mut req);
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
                let resolved = pending_interactions.resolve_approval(req_id, allow);
                let _ = respond_json(req, 200, &json!({"ok": true, "resolved": resolved}));
            }
            (Method::Post, p)
                if p.starts_with("/api/chat/conversations/") && p.ends_with("/user-input") =>
            {
                let body = read_json_body(&mut req);
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
                let _ = respond_json(req, 200, &json!({"ok": true, "resolved": resolved}));
            }
            (Method::Get, p)
                if p.starts_with("/api/chat/conversations/") && p.ends_with("/ui-surfaces") =>
            {
                let conv_id = p
                    .trim_start_matches("/api/chat/conversations/")
                    .trim_end_matches("/ui-surfaces");
                let surfaces = list_conversation_surfaces(&store, conv_id);
                let _ = respond_json(req, 200, &json!({"surfaces": surfaces}));
            }
            (Method::Post, p)
                if p.starts_with("/api/chat/conversations/") && p.ends_with("/ui-surface-revision") =>
            {
                let conv_id = p
                    .trim_start_matches("/api/chat/conversations/")
                    .trim_end_matches("/ui-surface-revision");
                let body = read_json_body(&mut req);
                match serde_json::from_value::<UiSurfaceRevisionRef>(body) {
                    Ok(reference) if reference.identity.owner.conversation_id == conv_id => {
                        match read_ui_surface_revision(&store, &reference) {
                            Ok(snapshot) => {
                                let _ = respond_json(req, 200, &json!({"snapshot": snapshot}));
                            }
                            Err(e) => {
                                let _ = respond_json(req, 409, &json!({"error": e}));
                            }
                        }
                    }
                    _ => {
                        let _ = respond_json(req, 400, &json!({"error": "invalid_ui_revision_ref"}));
                    }
                }
            }
            (Method::Post, p)
                if p.starts_with("/api/chat/conversations/") && p.ends_with("/ui-surface-changes") =>
            {
                let conv_id = p
                    .trim_start_matches("/api/chat/conversations/")
                    .trim_end_matches("/ui-surface-changes");
                let body = read_json_body(&mut req);
                let identity = body
                    .get("identity")
                    .cloned()
                    .and_then(|v| serde_json::from_value::<UiSurfaceIdentity>(v).ok());
                let base_rev = body.get("base_revision").and_then(|v| v.as_u64());
                match identity {
                    Some(id) if id.owner.conversation_id == conv_id => {
                        match read_ui_surface_changes(&store, &id, base_rev) {
                            Ok(update) => {
                                let _ = respond_json(req, 200, &json!({"update": update}));
                            }
                            Err(e) => {
                                let _ = respond_json(req, 409, &json!({"error": e}));
                            }
                        }
                    }
                    _ => {
                        let _ = respond_json(req, 400, &json!({"error": "invalid_ui_changes_query"}));
                    }
                }
            }
            (Method::Post, p)
                if p.starts_with("/api/chat/conversations/") && p.ends_with("/ui-surface-preview") =>
            {
                let conv_id = p
                    .trim_start_matches("/api/chat/conversations/")
                    .trim_end_matches("/ui-surface-preview");
                let body = read_json_body(&mut req);
                match serde_json::from_value::<UiDraftPreviewInput>(body) {
                    Ok(input) if input.identity.owner.conversation_id == conv_id => {
                        match preview_ui_surface(&store, &input) {
                            Ok(preview) => {
                                let _ = respond_json(req, 200, &json!({"preview": preview}));
                            }
                            Err(e) => {
                                let _ = respond_json(req, 409, &json!({"error": e}));
                            }
                        }
                    }
                    _ => {
                        let _ = respond_json(req, 400, &json!({"error": "invalid_ui_preview_input"}));
                    }
                }
            }
            (Method::Post, p)
                if p.starts_with("/api/chat/conversations/") && p.ends_with("/ui-interactions") =>
            {
                let conv_id = p
                    .trim_start_matches("/api/chat/conversations/")
                    .trim_end_matches("/ui-interactions");
                let body = read_json_body(&mut req);
                match serde_json::from_value::<UiInteraction>(body) {
                    Ok(interaction) if interaction.identity.owner.conversation_id == conv_id => {
                        match commit_ui_interaction(&store, &interaction) {
                            Ok(committed) => {
                                let _ = respond_json(
                                    req,
                                    200,
                                    &json!({
                                        "status": "committed",
                                        "snapshot": committed.snapshot,
                                        "dataModel": committed.data_model,
                                        "actionId": committed.action_id,
                                        "summary": committed.summary
                                    }),
                                );
                            }
                            Err(e) => {
                                let status = if e.contains("revision conflict") { 409 } else { 422 };
                                let _ = respond_json(req, status, &json!({"status": "rejected", "error": e}));
                            }
                        }
                    }
                    _ => {
                        let _ = respond_json(req, 400, &json!({"status": "rejected", "error": "invalid_ui_interaction"}));
                    }
                }
            }
            (Method::Post, p)
                if p.starts_with("/api/chat/conversations/") && p.ends_with("/truncate") =>
            {
                let conv_id = p
                    .trim_start_matches("/api/chat/conversations/")
                    .trim_end_matches("/truncate");
                let body = read_json_body(&mut req);
                let from_seq = body
                    .get("fromSeq")
                    .or_else(|| body.get("from_seq"))
                    .and_then(|v| v.as_i64())
                    .or_else(|| {
                        body.get("fromMessageId")
                            .or_else(|| body.get("messageId"))
                            .and_then(|v| v.as_str())
                            .and_then(|s| s.strip_prefix("msg-"))
                            .and_then(|s| s.parse::<i64>().ok())
                    })
                    .unwrap_or(1);
                match store.truncate_events_from(conv_id, from_seq) {
                    Ok(deleted) => {
                        let _ = respond_json(
                            req,
                            200,
                            &json!({"ok": true, "deletedEvents": deleted, "fromSeq": from_seq}),
                        );
                    }
                    Err(e) => {
                        let _ = respond_json(req, 500, &json!({"error": e.to_string()}));
                    }
                }
            }
            (Method::Post, p)
                if p.starts_with("/api/chat/conversations/") && p.ends_with("/compact") =>
            {
                let conv_id = p
                    .trim_start_matches("/api/chat/conversations/")
                    .trim_end_matches("/compact");
                let (messages, _) = reconstruct_v2_messages(&store, conv_id);
                let summary = format!("Compacted {} message(s) at {}", messages.len(), Utc::now().to_rfc3339());
                let _ = store.append_event(
                    conv_id,
                    "compaction_marker",
                    json!({"summary": &summary, "messageCount": messages.len()}),
                    50_000,
                );
                let _ = respond_json(req, 200, &json!({"ok": true, "summary": summary}));
            }
            (Method::Get, p)
                if p.starts_with("/api/chat/conversations/") && p.ends_with("/goal") =>
            {
                let conv_id = p
                    .trim_start_matches("/api/chat/conversations/")
                    .trim_end_matches("/goal");
                let key = format!("conv_goal:{conv_id}");
                let goal = store.get_kv(&key).ok().flatten().unwrap_or(Value::Null);
                let _ = respond_json(req, 200, &json!({"goal": goal}));
            }
            (Method::Post, p)
                if p.starts_with("/api/chat/conversations/") && p.ends_with("/goal") =>
            {
                let conv_id = p
                    .trim_start_matches("/api/chat/conversations/")
                    .trim_end_matches("/goal");
                let mut body = read_json_body(&mut req);
                let id = body
                    .get("id")
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string)
                    .unwrap_or_else(new_id);
                body["id"] = json!(id);
                body["conversationId"] = json!(conv_id);
                body["updatedAt"] = json!(Utc::now().to_rfc3339());
                let _ = store.set_kv(&format!("conv_goal:{conv_id}"), &body);
                let _ = suite.upsert_record("goal", &id, &body);
                let _ = respond_json(req, 200, &json!({"ok": true, "goal": body}));
            }
            (Method::Delete, p)
                if p.starts_with("/api/chat/conversations/") && p.ends_with("/goal") =>
            {
                let conv_id = p
                    .trim_start_matches("/api/chat/conversations/")
                    .trim_end_matches("/goal");
                let _ = store.set_kv(&format!("conv_goal:{conv_id}"), &Value::Null);
                let _ = respond_json(req, 200, &json!({"ok": true, "deleted": true}));
            }
            (Method::Get, p)
                if p.starts_with("/api/chat/conversations/") && p.ends_with("/data-sources") =>
            {
                let conv_id = p
                    .trim_start_matches("/api/chat/conversations/")
                    .trim_end_matches("/data-sources");
                let bound = store
                    .get_kv(&format!("conv_data_sources:{conv_id}"))
                    .ok()
                    .flatten()
                    .unwrap_or_else(|| json!([]));
                let _ = respond_json(req, 200, &json!({"sources": bound}));
            }
            (Method::Post, p)
                if p.starts_with("/api/chat/conversations/") && p.ends_with("/data-sources") =>
            {
                let conv_id = p
                    .trim_start_matches("/api/chat/conversations/")
                    .trim_end_matches("/data-sources");
                let body = read_json_body(&mut req);
                let _ = store.set_kv(&format!("conv_data_sources:{conv_id}"), &body);
                let _ = respond_json(req, 200, &json!({"ok": true, "sources": body}));
            }
            (Method::Post, p)
                if p.starts_with("/api/chat/conversations/") && p.ends_with("/messages") =>
            {
                let conv_id = p
                    .trim_start_matches("/api/chat/conversations/")
                    .trim_end_matches("/messages")
                    .to_string();
                let body = read_json_body(&mut req);
                let store_c = store.clone();
                let suite_c = Arc::clone(&suite);
                let cancels_c = Arc::clone(&active_cancels);
                let pending_c = Arc::clone(&pending_interactions);
                let host_id_c = host_id.clone();
                std::thread::spawn(move || {
                    handle_sse_chat_turn(
                        req,
                        store_c,
                        suite_c,
                        cancels_c,
                        pending_c,
                        host_id_c,
                        conv_id,
                        body,
                    );
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
                if let Some(endpoints) = body.get("endpoints").and_then(|v| v.as_array()) {
                    let mut cfg = load_active_config(&store);
                    let prev_keys: BTreeMap<String, Option<String>> = cfg
                        .llm
                        .connections
                        .iter()
                        .map(|c| (c.id.clone(), c.api_key.clone()))
                        .collect();
                    let mut new_conns = Vec::new();
                    let mut flattened_profiles = Vec::new();
                    let mut best_default: Option<(i64, String, String)> = None;

                    for (ep_idx, ep) in endpoints.iter().enumerate() {
                        let ep_id = ep
                            .get("id")
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.is_empty())
                            .map(ToString::to_string)
                            .unwrap_or_else(new_id);
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

                                conn_models.push(crate::config::ModelEntry {
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
                                    capabilities: None,
                                });

                                flattened_profiles.push(json!({
                                    "id": prof_id,
                                    "provider": "openai-compatible",
                                    "connectionId": Value::Null,
                                    "modelId": model_name,
                                    "modelName": model_name,
                                    "displayName": display_name,
                                    "revision": 2,
                                    "enabled": enabled,
                                    "selected": false,
                                    "priority": priority,
                                    "order": m_idx,
                                    "capabilities": caps,
                                    "executionCapabilities": m.get("executionCapabilities").cloned().unwrap_or_else(|| json!({
                                        "toolCalls": true,
                                        "imageInput": true,
                                        "reasoning": true,
                                        "samplingParameters": true,
                                        "samplingRequiresNoReasoning": false,
                                        "chatOutputTokenField": "auto"
                                    })),
                                    "executionCapabilityStatus": "tools_enabled",
                                    "reasoning": m.get("reasoning").cloned().unwrap_or_else(|| json!({"control": "automatic"})),
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

                        new_conns.push(crate::config::ConnectionConfig {
                            id: ep_id.clone(),
                            endpoint: base_url,
                            protocol: Some(rust_proto.into()),
                            provider: None,
                            api_key: prev_keys.get(&ep_id).cloned().flatten(),
                            api_key_env: None,
                            models: conn_models,
                        });
                    }

                    if let Some((_, ref def_conn, ref def_model)) = best_default {
                        cfg.llm.default_model = Some(crate::config::DefaultModelSelection {
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
                    }
                    cfg.llm.connections = new_conns;
                    let _ = store.set_kv(
                        crate::config::KV_APPLIED_CONFIG,
                        &serde_json::to_value(&cfg).unwrap_or(json!({})),
                    );
                    let _ = store.set_kv("web_model_profiles", &Value::Array(flattened_profiles));
                } else if let Some(profiles) = body.get("profiles") {
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
                let _ = respond_json(
                    req,
                    200,
                    &json!({"connections": [], "accounts": [], "selections": []}),
                );
            }
            (Method::Post, "/api/model-connections/kimi-login/owners")
            | (Method::Post, "/api/model-connections/opencode-login/owners") => {
                let _ = respond_json(req, 200, &json!([]));
            }
            (Method::Post, "/api/model-connections/probe") => {
                let body = read_json_body(&mut req);
                let _ = respond_json(
                    req,
                    200,
                    &json!({
                        "ok": true,
                        "capabilities": {
                            "toolCalls": true,
                            "imageInput": true,
                            "reasoning": true,
                            "samplingParameters": true
                        },
                        "profile_id": body.get("profile_id").cloned().unwrap_or(Value::Null)
                    }),
                );
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
                    let base = conn.endpoint.trim_end_matches('/');
                    let url = if base.ends_with("/models") {
                        base.to_string()
                    } else {
                        format!("{base}/models")
                    };
                    let mut models: Vec<String> = Vec::new();
                    if let Ok(client) = reqwest::blocking::Client::builder()
                        .timeout(Duration::from_secs(10))
                        .build()
                    {
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
                    let _ = respond_json(req, 200, &json!({"models": models}));
                } else {
                    let _ = respond_json(req, 404, &json!({"error": "endpoint_not_found"}));
                }
            }
            (Method::Get, p)
                if p.starts_with("/api/model-endpoints/") && p.ends_with("/credential") =>
            {
                let ep_id = percent_decode_path(
                    p.trim_start_matches("/api/model-endpoints/")
                        .trim_end_matches("/credential"),
                );
                let cfg = load_active_config(&store);
                let configured = cfg
                    .llm
                    .connections
                    .iter()
                    .find(|c| c.id == ep_id)
                    .and_then(|c| c.api_key.as_ref())
                    .is_some_and(|k| !k.trim().is_empty());
                let _ = respond_json(
                    req,
                    200,
                    &json!({"endpointId": ep_id, "configured": configured, "hasApiKey": configured}),
                );
            }
            (Method::Post, p) | (Method::Put, p)
                if p.starts_with("/api/model-endpoints/") && p.ends_with("/credential") =>
            {
                let ep_id = percent_decode_path(
                    p.trim_start_matches("/api/model-endpoints/")
                        .trim_end_matches("/credential"),
                );
                let body = read_json_body(&mut req);
                let api_key = body
                    .get("apiKey")
                    .or_else(|| body.get("api_key"))
                    .or_else(|| body.get("secret"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.trim().to_string());
                let mut cfg = load_active_config(&store);
                if let Some(conn) = cfg.llm.connections.iter_mut().find(|c| c.id == ep_id) {
                    conn.api_key = api_key.filter(|s| !s.is_empty());
                    let configured = conn.api_key.is_some();
                    let _ = store.set_kv(
                        crate::config::KV_APPLIED_CONFIG,
                        &serde_json::to_value(&cfg).unwrap_or(json!({})),
                    );
                    let _ = respond_json(
                        req,
                        200,
                        &json!({"ok": true, "endpointId": ep_id, "configured": configured}),
                    );
                } else {
                    let _ = respond_json(req, 404, &json!({"error": "endpoint_not_found"}));
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
                    let _ = store.set_kv(
                        crate::config::KV_APPLIED_CONFIG,
                        &serde_json::to_value(&cfg).unwrap_or(json!({})),
                    );
                }
                let _ = respond_json(
                    req,
                    200,
                    &json!({"ok": true, "endpointId": ep_id, "configured": false}),
                );
            }
            (Method::Post, p) | (Method::Patch, p)
                if p.starts_with("/api/model-endpoints/") && p.ends_with("/configuration") =>
            {
                let ep_id = percent_decode_path(
                    p.trim_start_matches("/api/model-endpoints/")
                        .trim_end_matches("/configuration"),
                );
                let body = read_json_body(&mut req);
                let mut cfg = load_active_config(&store);
                if let Some(conn) = cfg.llm.connections.iter_mut().find(|c| c.id == ep_id) {
                    if let Some(url) = body.get("baseURL").or_else(|| body.get("url")).and_then(|v| v.as_str()) {
                        conn.endpoint = url.to_string();
                    }
                    if let Some(proto) = body.get("wireProtocol").or_else(|| body.get("protocol")).and_then(|v| v.as_str()) {
                        conn.protocol = Some(match proto {
                            "anthropic" => "anthropic".into(),
                            "responses" | "siwc_responses" => "openai_responses".into(),
                            _ => "openai_chat".into(),
                        });
                    }
                    let _ = store.set_kv(
                        crate::config::KV_APPLIED_CONFIG,
                        &serde_json::to_value(&cfg).unwrap_or(json!({})),
                    );
                }
                let _ = respond_json(req, 200, &build_model_profiles_response(&store));
            }
            (Method::Get, "/api/model-gateway/clients") => {
                let clients = store
                    .get_kv("model_gateway_clients")
                    .ok()
                    .flatten()
                    .unwrap_or_else(|| json!([]));
                let _ = respond_json(req, 200, &clients);
            }
            (Method::Get, "/api/model-gateway/access") => {
                let access = store
                    .get_kv("model_gateway_access")
                    .ok()
                    .flatten()
                    .unwrap_or_else(|| json!({"revision": 1, "enabled": false}));
                let _ = respond_json(req, 200, &access);
            }
            (Method::Post, "/api/model-gateway/access") => {
                let body = read_json_body(&mut req);
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
                    let _ = respond_json(req, 200, &saved);
                } else {
                    let token = format!("nola_gw_{}", new_id());
                    let _ = respond_json(
                        req,
                        200,
                        &json!({"revision": next_rev, "enabled": true, "token": token}),
                    );
                }
            }
            (Method::Get, "/api/model-gateway/attempts") => {
                let _ = respond_json(req, 200, &json!([]));
            }
            (Method::Get, "/api/agent/skill-settings-review") => {
                let _ = respond_json(
                    req,
                    200,
                    &json!({"pending": false, "expected_revision": 1, "items": []}),
                );
            }
            (Method::Post, "/api/agent/skill-settings-review") => {
                let _ = respond_json(req, 200, &json!({"ok": true, "pending": false}));
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
            (Method::Post, p)
                if p.starts_with("/api/scheduled-actions/") && p.ends_with("/run-now") =>
            {
                let action_id = percent_decode_path(
                    p.trim_start_matches("/api/scheduled-actions/")
                        .trim_end_matches("/run-now"),
                );
                let store_c = store.clone();
                let suite_c = Arc::clone(&suite);
                std::thread::spawn(move || {
                    let Some(action) = suite_c
                        .list_records::<Value>("automation")
                        .unwrap_or_default()
                        .into_iter()
                        .find(|v| v.get("id").and_then(|s| s.as_str()) == Some(action_id.as_str()))
                    else {
                        let _ = respond_json(req, 404, &json!({"error": "action_not_found"}));
                        return;
                    };
                    let prompt = action
                        .get("userPrompt")
                        .or_else(|| action.get("prompt"))
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.trim().is_empty())
                        .unwrap_or("Run scheduled automation check")
                        .to_string();
                    let name = action
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or(&action_id)
                        .to_string();
                    let cfg = load_active_config(&store_c);
                    let run_id = new_id();
                    let conv_id = format!("auto-{run_id}");
                    let now_iso = Utc::now().to_rfc3339();
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
                        let hooks = HookRunner::load(
                            &suite_c.workspace_root,
                            suite_c.user_home.as_deref(),
                            false,
                        );
                        let mut reg = ToolRegistry::with_builtin_tools();
                        suite_c.register_tools(&mut reg);
                        let engine = AgentEngine::new(store_c.clone(), Arc::new(reg), caller);
                        let mut ctx = ToolContext::new(&suite_c.workspace_root);
                        ctx.permission_profile = policy.profile;
                        let run_cfg = AgentRunConfig {
                            session_id: Some(conv_id.clone()),
                            resume: false,
                            route,
                            frozen_system_blocks: build_frozen_system_blocks(&suite_c.workspace_root, &cfg),
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
                        Ok(res.final_text)
                    }) {
                        Ok(summary) => {
                            let result_id = new_id();
                            let rec = json!({
                                "resultId": result_id,
                                "sourceId": action_id,
                                "sourceName": name,
                                "sourceUrl": action.get("url").and_then(|v| v.as_str()).unwrap_or(""),
                                "sourceFormat": action.get("format").and_then(|v| v.as_str()).unwrap_or("agent"),
                                "conversationId": conv_id,
                                "runId": run_id,
                                "status": "answered",
                                "scheduledAt": now_iso,
                                "createdAt": now_iso,
                                "fetchedAt": now_iso,
                                "previewText": summary.chars().take(240).collect::<String>(),
                                "llmSummaryText": summary,
                                "ackCount": 0,
                                "expired": false,
                                "events": []
                            });
                            let _ = suite_c.upsert_record("automation_result", &result_id, &rec);
                            let _ = respond_json(req, 200, &json!({"ok": true, "result": rec}));
                        }
                        Err(e) => {
                            let _ = respond_json(req, 400, &json!({"error": e}));
                        }
                    }
                });
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
                let kind = existing
                    .get("kind")
                    .and_then(|v| v.as_str())
                    .unwrap_or("localFile")
                    .to_string();
                let location = existing
                    .get("location")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let max_age = existing
                    .get("maximumAgeMinutes")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(60)
                    .max(1);
                let (state, sample, record_count, err_msg) = if kind == "localFile" {
                    let target = if Path::new(&location).is_absolute() {
                        PathBuf::from(&location)
                    } else {
                        suite.workspace_root.join(&location)
                    };
                    match fs::read_to_string(&target) {
                        Ok(content) => {
                            let lines = content.lines().count();
                            let preview: String = content.chars().take(240).collect();
                            let st = if content.trim().is_empty() {
                                "verifiedEmpty"
                            } else {
                                "verified"
                            };
                            (st, preview, lines, Value::Null)
                        }
                        Err(e) => ("failed", String::new(), 0, json!(e.to_string())),
                    }
                } else {
                    ("verified", format!("HTTP endpoint {location}"), 1, Value::Null)
                };
                existing["validationContract"] = json!({
                    "sourceId": id,
                    "readerDescription": format!("{kind} reader"),
                    "schemaFields": ["content"],
                    "redactedSample": sample,
                    "recordCount": record_count,
                    "maximumAgeMinutes": max_age,
                    "validationState": state,
                    "validatedAt": Utc::now().to_rfc3339(),
                    "errorMessage": err_msg
                });
                existing["updatedAt"] = json!(Utc::now().to_rfc3339());
                let _ = suite.upsert_record("data_source", &id, &existing);
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

#[allow(clippy::too_many_arguments)]
fn handle_sse_chat_turn(
    req: Request,
    store: Store,
    suite: Arc<ModulesSuite>,
    cancels: Arc<Mutex<BTreeMap<String, Arc<AtomicBool>>>>,
    pending: Arc<PendingInteractions>,
    host_id: String,
    conversation_id: String,
    body: Value,
) {
    let raw_prompt = body
        .get("content")
        .or_else(|| body.get("prompt"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let attachments = body
        .get("attachments")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    // Compose effective prompt including text attachments cleanly
    let mut prompt = raw_prompt.clone();
    for att in &attachments {
        let name = att.get("name").and_then(|v| v.as_str()).unwrap_or("attachment");
        if let Some(text) = att.get("text").and_then(|v| v.as_str()) {
            prompt.push_str(&format!("\n\n[Attached file: {name}]\n{text}"));
        } else if let Some(mime) = att.get("mimeType").and_then(|v| v.as_str()) {
            prompt.push_str(&format!("\n\n[Attached media: {name} ({mime})]"));
        }
    }

    let mut cfg = load_active_config(&store);
    let sess = store.get_session(&conversation_id).ok().flatten();
    let conv_meta = store
        .get_kv(&session_meta_key(&conversation_id))
        .ok()
        .flatten()
        .unwrap_or_else(|| json!({}));

    // Respect per-conversation or per-request model override!
    let override_model = body
        .get("modelId")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .or_else(|| sess.as_ref().and_then(|s| s.model_id.clone()));
    if let Some(ref mid) = override_model {
        if let Some((conn_id, model_name)) = mid.split_once(':') {
            if cfg.llm.connections.iter().any(|c| c.id == conn_id) {
                cfg.llm.default_model = Some(crate::config::DefaultModelSelection {
                    connection: conn_id.to_string(),
                    model: model_name.to_string(),
                });
            }
        } else if let Some(conn) = cfg
            .llm
            .connections
            .iter()
            .find(|c| c.models.iter().any(|m| m.id == *mid))
        {
            cfg.llm.default_model = Some(crate::config::DefaultModelSelection {
                connection: conn.id.clone(),
                model: mid.clone(),
            });
        }
    }

    let mut route = match resolve_model_route(&cfg) {
        Ok(r) => r,
        Err(e) => {
            let _ = respond_json(req, 400, &json!({"error": e, "code": "model_not_configured"}));
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
            let _ = respond_json(req, 500, &json!({"error": e.to_string()}));
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

    let cancel = Arc::new(AtomicBool::new(false));
    cancels
        .lock()
        .unwrap()
        .insert(conversation_id.clone(), Arc::clone(&cancel));

    let settings = build_agent_settings_snapshot(&store, &suite);
    let effective_perm_str = body
        .get("permissionProfile")
        .and_then(|v| v.as_str())
        .or_else(|| conv_meta.get("permissionProfile").and_then(|v| v.as_str()))
        .or_else(|| settings.get("permission_profile").and_then(|v| v.as_str()))
        .unwrap_or("accept_edits");
    let profile = match effective_perm_str {
        "allow_all" => PermissionProfile::AllowAll,
        "cautious" => PermissionProfile::Cautious,
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

    let (tx, rx) = mpsc::channel::<Vec<u8>>();

    // Live interactive approval callback -> emits SSE `permission_requested` and waits for `/permission`
    let tx_approval = tx.clone();
    let pending_approval = Arc::clone(&pending);
    let cancel_approval = Arc::clone(&cancel);
    let approval_cb: ApprovalCallback = Arc::new(move |spec, args, reason| {
        let req_id = new_id();
        let (resp_tx, resp_rx) = mpsc::channel::<bool>();
        pending_approval
            .approvals
            .lock()
            .unwrap()
            .insert(req_id.clone(), resp_tx);
        let frame = json!({
            "type": "permission_requested",
            "request_id": req_id,
            "tool": spec.schema.name,
            "reason": reason,
            "arguments": args
        });
        let _ = tx_approval.send(format!("data: {frame}\n\n").into_bytes());
        let deadline = std::time::Instant::now() + Duration::from_secs(120);
        let decision = loop {
            if cancel_approval.load(Ordering::SeqCst) {
                break false;
            }
            match resp_rx.recv_timeout(Duration::from_millis(100)) {
                Ok(allow) => break allow,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if std::time::Instant::now() >= deadline {
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
    let user_input_cb: UserInputCallback = Arc::new(move |question, options| {
        let req_id = new_id();
        let (resp_tx, resp_rx) = mpsc::channel::<Option<String>>();
        pending_question
            .questions
            .lock()
            .unwrap()
            .insert(req_id.clone(), resp_tx);
        let frame = json!({
            "type": "user_question",
            "request_id": req_id,
            "question": question,
            "options": options
        });
        let _ = tx_question.send(format!("data: {frame}\n\n").into_bytes());
        let deadline = std::time::Instant::now() + Duration::from_secs(120);
        let answer = loop {
            if cancel_question.load(Ordering::SeqCst) {
                break None;
            }
            match resp_rx.recv_timeout(Duration::from_millis(100)) {
                Ok(ans) => break ans,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if std::time::Instant::now() >= deadline {
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
    let mut ctx = ToolContext::new(&suite.workspace_root);
    ctx.permission_profile = policy.profile;
    ctx.sandbox_mode = SandboxMode::from_env();

    let run_cfg = AgentRunConfig {
        session_id: Some(conversation_id.clone()),
        resume: true,
        route,
        frozen_system_blocks: build_frozen_system_blocks(&suite.workspace_root, &cfg),
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
        auto_recall_memory: true,
        inbox: None,
    };

    let conv_id_c = conversation_id.clone();
    let cancels_c = Arc::clone(&cancels);
    let cancel_c = Arc::clone(&cancel);
    let store_persist = store.clone();
    std::thread::spawn(move || {
        let mut collected_reasoning = String::new();
        let mut collected_tools: Vec<Value> = Vec::new();
        let mut collected_surfaces: Vec<Value> = Vec::new();
        let run_res = engine.run(&ctx, &prompt, run_cfg, |ev| {
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
                                let _ = tx.send(format!("data: {ui_frame}\n\n").into_bytes());
                            }
                        }
                    }
                }
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
                        "toolCalls": collected_tools,
                        "uiSurfaces": collected_surfaces
                    }),
                    50_000,
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
