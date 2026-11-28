// SPDX-License-Identifier: Apache-2.0
//! Full-Capability Secure HTTP + SSE WebHost (`tiny_http`) for Web, iOS, macOS, Android & Windows clients:
//! - Strict same-origin CSRF (`Origin == Host` + `Sec-Fetch-Site != cross-site`)
//! - O(1) bounded replay-nonce cache
//! - Persistent 0600 mobile access token + session cookies + HMAC-SHA256 mobile auth + AES-256-GCM mobile crypto
//! - Live streaming SSE chat endpoint (`POST /api/chat/conversations/<id>/messages`) with real media attachments
//! - Active-run cancellation, session-scoped permissions, real compaction, MCP tool wiring, and background scheduler
//! - Complete business module REST endpoints aligned with `apps/web/frontend`, iOS, macOS & Android
//! - Safe static asset & SPA serving (`/ui/*`, `/static/*`, and SPA fallback).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Instant;

use chrono::Utc;
use nola_engine::{
    compact_session_now, is_plan_mode, load_session_plan, load_session_todos, AgentEngine,
    AgentRunConfig, EffectivePolicy, ForceToolSet, HookRunner, HttpModelCaller, ModelCaller,
};
use nola_llm::LlmClient;
use nola_mcp::McpManager;
use nola_modules::a2ui::{
    commit_ui_interaction, list_conversation_surfaces, preview_ui_surface,
    read_ui_surface_changes, read_ui_surface_revision, UiDraftPreviewInput, UiInteraction,
    UiSurfaceIdentity, UiSurfaceRevisionRef,
};
use nola_modules::automations_messaging::handle_inbound_webhook_turn_with_secret;
use nola_modules::{
    built_in_role_templates, check_for_update, download_update_package, index_data_source,
    install_plugin_package, list_data_source_usage, list_desktop_plugin_pages,
    list_harness_plugins, parse_inbound_webhook_payload, probe_engine_status, probe_hardware,
    recommend_local_models, reconcile_enabled_plugins, run_engine_setup_with_opt_in,
    search_hub_models, spawn_background_goal_worker, spawn_background_scheduler,
    verify_inbound_webhook_token, AutomationScheduler, GoalWorker, LocalModelProcessManager,
    ModelDownloadManager, ModelDownloadPlan, ModulesSuite, RoomRuntime,
};
use nola_store::{EventPage, Store, Vault};
use nola_tools::{NativeBrowserBridge, ToolContext, ToolRegistry};
use nola_types::{
    new_id, now_ms, sha256_hex, HostCapabilities, MemoryItem, ModelCapabilities, PermissionProfile,
    PluginRecord, Session,
};
use serde_json::{json, Value};
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};
use url::Url;

use crate::config::{
    build_frozen_system_blocks, default_home_dir, load_active_config, read_config_file_safe,
    resolve_model_route, resolve_vault_path_for_store, save_applied_config, vault_secret_key,
    ConnectionConfig, DefaultModelSelection, ModelEntry,
};
use crate::web_api::{
    build_agent_settings_snapshot, build_dashboard_snapshot, build_model_profiles_response,
    build_workspace_review, canonical_automation_format, conversation_configuration_json,
    conversation_knowledge_key, ensure_host_id, ensure_source_store_epoch,
    infer_model_capabilities, load_conversation_knowledge, memory_item_to_record_with_store,
    memory_meta_key, ms_to_rfc3339, normalize_automation_record, normalize_data_source_record,
    project_skill_descriptor, reconstruct_v2_messages, session_meta_key,
    session_to_conversation_json, session_usage_json, sync_agent_settings_mcp_servers,
    sync_web_model_profiles_cache,
};


mod auth;
mod helpers;
mod interactions;
mod paired;
mod tailcat;
#[cfg(test)]
mod tests;

pub use auth::{validate_native_browser_token, validate_same_origin, BoundedNonceCache};

use auth::*;
use helpers::*;
use interactions::*;
use paired::*;
use tailcat::*;

const MAX_CONCURRENT_REQUESTS: usize = 32;

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
                    active_cancels,
                    pending_interactions,
                    &body,
                );
                let _ = respond_api_json(req, 200, &page, crypto_tok);
            }
            (Method::Post, "/api/activity/detail") => {
                let detail = build_activity_detail(
                    &store,
                    &suite,
                    active_cancels,
                    pending_interactions,
                    host_id,
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
                let review = build_workspace_review(host_id, "workspace", &suite.workspace_root);
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
                let is_ui_queue_query = paired_events
                    || (method == Method::Post
                        && (body.get("expected_source_store_epoch").is_some()
                            || body.get("client_id").is_some()));
                match store.events_after(&id, after, 500) {
                    Ok(EventPage::Events(ev)) => {
                        let inner = if is_ui_queue_query {
                            json!({
                                "status": "ok",
                                "events": ev,
                                "head": Value::Null,
                                "dispatch": Value::Null
                            })
                        } else {
                            json!({"status": "ok", "events": ev})
                        };
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
                        let inner = if is_ui_queue_query {
                            json!({
                                "status": "resync_required",
                                "earliest_retained_seq": earliest_retained_seq,
                                "latest_seq": latest_seq,
                                "head": Value::Null,
                                "dispatch": Value::Null
                            })
                        } else {
                            json!({
                                "status": "resync_required",
                                "earliest_retained_seq": earliest_retained_seq,
                                "latest_seq": latest_seq
                            })
                        };
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
                let through = match parse_message_boundary(&body) {
                    Ok(seq) => seq,
                    Err(error) => {
                        let _ = respond_api_json(req, 400, &json!({"error": error}), crypto_tok);
                        return;
                    }
                };
                if let Err(e) = store.branch_session(&id, &sess, through) {
                    let status = if e.is_missing_row() { 404 } else { 500 };
                    let _ = respond_api_json(req, status, &json!({"error": e.to_string()}), crypto_tok);
                    return;
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
                if let Some(command_id) = body
                    .get("command_id")
                    .or_else(|| body.get("commandId"))
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    let _ = store.set_kv(
                        &format!("cancelled_ui_command:{id}:{command_id}"),
                        &json!({
                            "command_id": command_id,
                            "request_digest": body.get("request_digest").cloned().unwrap_or(Value::Null),
                            "cancelled_at_ms": now_ms()
                        }),
                    );
                    let inner = json!({
                        "cancelled": true,
                        "command_id": command_id
                    });
                    let payload =
                        if raw_path.starts_with("/api/mobile/") || has_expected_host_id(&body) {
                            wrap_paired_envelope(&store, host_id, inner)
                        } else {
                            inner
                        };
                    let _ = respond_api_json(req, 200, &payload, crypto_tok);
                    return;
                }
                if is_paired_task_cancellation_body(&body) {
                    let (status, receipt) = handle_paired_task_cancellation(
                        &store,
                        &suite,
                        active_cancels,
                        host_id,
                        &id,
                        None,
                        &body,
                    );
                    let _ = respond_api_json(req, status, &receipt, crypto_tok);
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
                let conversation_id = percent_decode_path(
                    p.trim_start_matches("/api/chat/conversations/").trim_end_matches("/permission"),
                );
                let resolved = pending_interactions.resolve_approval(&conversation_id, req_id, allow, scope);
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
                let boundary = match parse_message_boundary(&body) {
                    Ok(Some(seq)) => seq,
                    outcome => {
                        let error = outcome.err().unwrap_or("missing_message_boundary");
                        let _ = respond_api_json(req, 400, &json!({"error": error}), crypto_tok);
                        return;
                    }
                };
                let keep_message = body
                    .get("keepMessage")
                    .or_else(|| body.get("keep_message"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                match store.truncate_at(&conv_id, boundary, keep_message) {
                    Ok((from_seq, deleted)) => {
                        let _ = respond_api_json(
                            req,
                            200,
                            &json!({"ok": true, "deletedEvents": deleted, "fromSeq": from_seq}),
                            crypto_tok,
                        );
                    }
                    Err(e) => {
                        let status = if e.is_missing_row() { 404 } else { 500 };
                        let _ = respond_api_json(req, status, &json!({"error": e.to_string()}), crypto_tok);
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
                    Ok(mut report) => {
                        let before_msgs = report
                            .get("before_messages")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0);
                        let after_msgs = report
                            .get("after_messages")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0);
                        let replaced_item_count = before_msgs.saturating_sub(after_msgs);
                        let remaining_item_count = after_msgs;
                        let compacted = replaced_item_count > 0;
                        let used_tokens = report
                            .get("tokens_after")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0);
                        let context_window_tokens = route
                            .as_ref()
                            .map(|r| r.context_window as u64)
                            .unwrap_or(128_000)
                            .max(1);
                        let usage_percent = ((used_tokens as f64 / context_window_tokens as f64)
                            * 100.0)
                            .clamp(0.0, 100.0);
                        let rev_key = format!("conv_compact_rev:{conv_id}");
                        let prev_rev = store
                            .get_kv(&rev_key)
                            .ok()
                            .flatten()
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0);
                        let checkpoint_revision = prev_rev.saturating_add(1).max(1);
                        let _ = store.set_kv(&rev_key, &json!(checkpoint_revision));
                        let committed_at_ms = now_ms().max(1);
                        if let Some(obj) = report.as_object_mut() {
                            obj.insert("compacted".into(), json!(compacted));
                            obj.insert("checkpoint_revision".into(), json!(checkpoint_revision));
                            obj.insert("committed_at_ms".into(), json!(committed_at_ms));
                            obj.insert(
                                "context".into(),
                                json!({
                                    "used_tokens": used_tokens,
                                    "context_window_tokens": context_window_tokens,
                                    "usage_percent": usage_percent,
                                    "replaced_item_count": replaced_item_count,
                                    "remaining_item_count": remaining_item_count
                                }),
                            );
                        }
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
                    let paired = is_paired_task_cancellation_body(&body) || raw_path.starts_with("/api/mobile/");
                    let (status, receipt) = handle_paired_task_cancellation(
                        &store, &suite, active_cancels, host_id, &conv_id, Some(&task_id), &body,
                    );
                    let payload = if paired || status != 200 { receipt } else {
                        json!({"ok": true, "stopped": receipt["terminal"], "cancelled": receipt["state"] == "cancelled",
                               "cancelRequested": receipt["cancelRequested"], "conversationId": conv_id, "taskId": task_id})
                    };
                    let _ = respond_api_json(req, status, &payload, crypto_tok);
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
                    let journal_identity = json!({
                        "source_store_epoch": epoch,
                        "conversation_id": conv_id,
                        "run_id": run_id,
                        "assistant_message_id": asst_id
                    });
                    let admission_digest = sha256_hex(
                        format!("{epoch}:{conv_id}:{run_id}:{asst_id}:admission:1").as_bytes(),
                    );
                    let admission_cursor = json!({
                        "identity": journal_identity,
                        "sequence": "1",
                        "record_digest": admission_digest
                    });
                    let (status_str, worker_state, recovery_str, can_submit, terminal_seal) =
                        if active {
                            ("running", "running", "observe", false, Value::Null)
                        } else {
                            let latest_seq = match store.events_after(&conv_id, 0, 500) {
                                Ok(EventPage::Events(ev)) => {
                                    ev.last().map(|e| e.seq.max(1) as u64).unwrap_or(1)
                                }
                                _ => 1,
                            };
                            let seq_str = latest_seq.to_string();
                            let completed_at_ms = store
                                .get_session(&conv_id)
                                .ok()
                                .flatten()
                                .map(|s| s.updated_at_ms.max(1))
                                .unwrap_or_else(|| now_ms().max(1));
                            let run_result = json!({
                                "result_id": format!("res-{run_id}"),
                                "run_id": run_id,
                                "conversation_id": conv_id,
                                "originating_submission_id": run_id,
                                "input": {
                                    "watermark": "0",
                                    "applications_digest": "e0aeaeba0c6a61c02edcc186902f9b41dd82835d01b780eb0e479cc513ea66dd"
                                },
                                "decided_at_revision": "1",
                                "decision_event_cursor": seq_str,
                                "requirement_revision": "0",
                                "evidence_frontier": seq_str,
                                "value": {
                                    "kind": "answered",
                                    "answer_part_refs": [format!("{asst_id}:part:0")]
                                }
                            });
                            let result_digest = sha256_hex(
                                serde_json::to_string(&run_result)
                                    .unwrap_or_default()
                                    .as_bytes(),
                            );
                            let manifest_digest = sha256_hex(
                                format!("{epoch}:{conv_id}:{run_id}:manifest:1").as_bytes(),
                            );
                            let seal_digest = sha256_hex(
                                format!("{epoch}:{conv_id}:{run_id}:{result_digest}:{manifest_digest}")
                                    .as_bytes(),
                            );
                            (
                                "sealed",
                                "absent",
                                "terminal",
                                true,
                                json!({
                                    "protocol_version": 1,
                                    "admission": admission_cursor,
                                    "identity": journal_identity,
                                    "core_session_id": conv_id,
                                    "transition_id": format!("tr-{run_id}"),
                                    "checkpoint_revision": "1",
                                    "core_revision": "1",
                                    "event_cursor": seq_str,
                                    "result": run_result,
                                    "result_digest": result_digest,
                                    "completed_at_ms": completed_at_ms,
                                    "entry_count": "1",
                                    "heads_count": "1",
                                    "child_receipt_count": "0",
                                    "manifest_digest": manifest_digest,
                                    "seal_digest": seal_digest
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
                            "admission": admission_cursor,
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
                            let payload = if is_mobile
                                || has_expected_host_id(&body)
                                || is_paired_journal_operation(&body)
                            {
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
                let paired_mobile = is_mobile && req_authenticated && header_value(&req, "x-nola-mobile-auth").is_some();
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
                    paired_mobile,
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
                    && (body.get("request_digest").is_some_and(|v| v.is_string())
                        || body.get("operation").and_then(|v| v.as_str()) == Some("prepare"))
                {
                    let is_mobile = raw_path.starts_with("/api/mobile/");
                    let receipt = handle_remote_submission_reconciliation(
                        &store,
                        active_cancels,
                        host_id,
                        &id,
                        &body,
                    );
                    let payload = if is_mobile
                        || has_expected_host_id(&body)
                        || is_paired_journal_operation(&body)
                    {
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
                    sync_web_model_profiles_cache(&store);
                    let _ = respond_api_json(req, 200, &build_model_profiles_response(&store), crypto_tok);
                }
            }
            (Method::Post, "/api/model-profiles/catalog/replace") => {
                if let Some(endpoints) = body.get("endpoints").and_then(|v| v.as_array()) {
                    let mut cfg = load_active_config(&store);
                    let prev_default = cfg.llm.default_model.clone();
                    let prev_conns_full = cfg.llm.connections.clone();
                    let prev_conns: BTreeMap<_, _> = cfg
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
                    let existing_managed_profile_ids: HashSet<String> = store
                        .get_kv("web_model_profiles")
                        .ok()
                        .flatten()
                        .and_then(|v| v.as_array().cloned())
                        .unwrap_or_default()
                        .into_iter()
                        .filter_map(|p| {
                            let has_conn_id = p
                                .get("connectionId")
                                .and_then(|v| v.as_str())
                                .is_some_and(|s| !s.trim().is_empty());
                            if has_conn_id {
                                p.pointer("/endpoint/id")
                                    .and_then(|v| v.as_str())
                                    .map(ToString::to_string)
                            } else {
                                None
                            }
                        })
                        .collect();
                    let managed_record_ids: HashSet<String> = suite
                        .list_records::<Value>("model_connection")
                        .unwrap_or_default()
                        .into_iter()
                        .filter_map(|rec| {
                            let id = rec
                                .get("id")
                                .or_else(|| rec.pointer("/connection/id"))
                                .and_then(|v| v.as_str())?
                                .to_string();
                            let src = rec
                                .get("source")
                                .or_else(|| rec.pointer("/connection/source"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("custom");
                            (src != "manual_legacy").then_some(id)
                        })
                        .collect();
                    let is_managed_connection = |c: &ConnectionConfig| -> bool {
                        c.id.starts_with("local:")
                            || c.provider.as_deref().is_some_and(|p| {
                                !p.is_empty() && p != "manual_legacy" && p != "openai-compatible"
                            })
                            || managed_record_ids.contains(&c.id)
                            || existing_managed_profile_ids.contains(&c.id)
                    };

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
                        if !ep_name.trim().is_empty() && ep_name != ep_id {
                            cfg.connection_extras
                                .entry(ep_id.clone())
                                .or_default()
                                .name = Some(ep_name);
                        }
                    }

                    // Preserve managed / imported / local connections not included in legacy editable endpoints
                    // and NEVER delete their Vault secrets!
                    let mut preserved_ids: HashSet<String> = HashSet::new();
                    for old_conn in prev_conns_full {
                        if !new_conn_ids.contains(&old_conn.id) && is_managed_connection(&old_conn) {
                            preserved_ids.insert(old_conn.id.clone());
                            new_conn_ids.insert(old_conn.id.clone());
                            new_conns.push(old_conn);
                        }
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
                    } else if let Some(ref prev_def) = prev_default {
                        if preserved_ids.contains(&prev_def.connection) {
                            cfg.llm.default_model = Some(prev_def.clone());
                        } else {
                            cfg.llm.default_model = None;
                        }
                    } else {
                        cfg.llm.default_model = None;
                    }
                    cfg.llm.connections = new_conns;
                    if let Err(e) = save_applied_config(&store, &cfg) {
                        let _ = respond_api_json(req, 400, &json!({"error": e}), crypto_tok);
                        return;
                    }
                    // Delete orphaned Vault keys ONLY for removed legacy endpoints (never managed/local ones).
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
                let src = body
                    .get("source")
                    .and_then(|v| v.as_str())
                    .unwrap_or("huggingface");
                let mirror = body
                    .get("mirror")
                    .or_else(|| body.get("mirror_base"))
                    .and_then(|v| v.as_str());
                let hw = probe_hardware();
                match search_hub_models(q, eng, mirror) {
                    Ok(results) => {
                        let candidates: Vec<Value> = results
                            .iter()
                            .map(|item| {
                                let repo = item
                                    .get("repository")
                                    .or_else(|| item.get("id"))
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                let download_bytes = item
                                    .get("downloadBytes")
                                    .and_then(|v| v.as_u64())
                                    .unwrap_or(0);
                                let est_mem = item
                                    .get("estimatedMemoryBytes")
                                    .and_then(|v| v.as_u64())
                                    .unwrap_or(download_bytes);
                                let ctx_tokens = item
                                    .get("contextTokens")
                                    .and_then(|v| v.as_u64())
                                    .unwrap_or(32_768);
                                let plan = item
                                    .get("plan")
                                    .filter(|v| v.is_object())
                                    .cloned()
                                    .unwrap_or_else(|| {
                                        json!({
                                            "engine": eng,
                                            "source": src,
                                            "repository": repo,
                                            "device": if hw.get("unifiedMemory").and_then(|v| v.as_bool()) == Some(true) { "unified" } else { "cpu" }
                                        })
                                    });
                                let mut cand = item.clone();
                                if let Some(obj) = cand.as_object_mut() {
                                    obj.entry("repository".to_string()).or_insert_with(|| json!(repo));
                                    obj.entry("revision".to_string()).or_insert_with(|| json!("main"));
                                    obj.entry("downloadBytes".to_string()).or_insert_with(|| json!(download_bytes));
                                    obj.entry("estimatedMemoryBytes".to_string()).or_insert_with(|| json!(est_mem));
                                    obj.entry("contextTokens".to_string()).or_insert_with(|| json!(ctx_tokens));
                                    obj.insert("plan".into(), plan);
                                }
                                cand
                            })
                            .collect();
                        let rec_source = if results
                            .iter()
                            .any(|r| r.get("offlineFallback").and_then(|v| v.as_bool()) == Some(false))
                        {
                            "hub"
                        } else {
                            "mixed"
                        };
                        let _ = respond_api_json(
                            req,
                            200,
                            &json!({
                                "ok": true,
                                "query": q,
                                "engine": eng,
                                "source": src,
                                "hardware": hw,
                                "candidates": candidates,
                                "recommendationSource": rec_source,
                                "results": results,
                                "models": results
                            }),
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
                let mut extra_args: Vec<String> = body
                    .get("extra_args")
                    .or_else(|| body.get("args"))
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|x| x.as_str().map(ToString::to_string))
                            .collect()
                    })
                    .unwrap_or_default();
                let saved_params = body
                    .get("parameters")
                    .filter(|v| v.is_object())
                    .or_else(|| rec.as_ref().and_then(|r| r.get("parameters")).filter(|v| v.is_object()));
                if let Some(params) = saved_params {
                    if engine.eq_ignore_ascii_case("llama.cpp") || engine.eq_ignore_ascii_case("llama") {
                        let param_args = LocalModelProcessManager::llama_server_args_from_parameters(
                            params,
                            &extra_args,
                        );
                        extra_args.extend(param_args);
                    }
                }
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
                sync_web_model_profiles_cache(&store);
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
                    match discover_endpoint_models(conn) {
                        Ok(models) => {
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
                                sync_web_model_profiles_cache(&store);
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
                        }
                        Err(err) => {
                            let status = if err.contains("empty") || err.contains("invalid") {
                                400
                            } else {
                                502
                            };
                            let _ = respond_api_json(
                                req,
                                status,
                                &json!({
                                    "ok": false,
                                    "error": "model_discovery_failed",
                                    "message": err,
                                    "connection_id": conn_id
                                }),
                                crypto_tok,
                            );
                        }
                    }
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
                    sync_web_model_profiles_cache(&store);
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
                    sync_web_model_profiles_cache(&store);
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
                let include_credentials = body
                    .get("include_credentials")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let allow_environment = body
                    .get("allow_environment")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let cred_path_str = body
                    .get("credential_path")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty());

                if path_str.is_empty() {
                    let _ = respond_api_json(
                        req,
                        400,
                        &json!({"error": "import_path_required", "message": "import file path is required"}),
                        crypto_tok,
                    );
                    return;
                }
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
                let raw = match read_config_file_safe(target_path) {
                    Ok(content) => content,
                    Err(e) => {
                        let _ = respond_api_json(
                            req,
                            400,
                            &json!({"error": "invalid_import_file", "message": e}),
                            crypto_tok,
                        );
                        return;
                    }
                };
                let credential_raw = if include_credentials {
                    if let Some(cp) = cred_path_str {
                        let cred_path = Path::new(cp);
                        if is_sensitive_local_path(cred_path) {
                            let _ = respond_api_json(
                                req,
                                403,
                                &json!({"error": "forbidden_path", "message": "refusing to read sensitive credential path"}),
                                crypto_tok,
                            );
                            return;
                        }
                        match read_config_file_safe(cred_path) {
                            Ok(c) => Some(c),
                            Err(e) => {
                                let _ = respond_api_json(
                                    req,
                                    400,
                                    &json!({"error": "invalid_credential_file", "message": e}),
                                    crypto_tok,
                                );
                                return;
                            }
                        }
                    } else {
                        None
                    }
                } else {
                    None
                };

                let parsed_candidates = match parse_model_import_candidates(
                    source,
                    path_str,
                    &raw,
                    credential_raw.as_deref(),
                    include_credentials,
                    allow_environment,
                ) {
                    Ok(list) => list,
                    Err(e) => {
                        let _ = respond_api_json(
                            req,
                            400,
                            &json!({"error": "import_parse_failed", "message": e}),
                            crypto_tok,
                        );
                        return;
                    }
                };

                let cfg = load_active_config(&store);
                let existing_conn_revs: BTreeMap<String, i64> = {
                    let mut map = BTreeMap::new();
                    for c in &cfg.llm.connections {
                        map.insert(c.id.clone(), 1);
                    }
                    for rec in suite.list_records::<Value>("model_connection").unwrap_or_default() {
                        if let Some(cid) = rec.get("id").and_then(|v| v.as_str()) {
                            let rev = rec.get("revision").and_then(|v| v.as_i64()).unwrap_or(1);
                            map.insert(cid.to_string(), rev);
                        }
                    }
                    map
                };

                let mut candidates: Vec<Value> = Vec::new();
                let mut internal_secrets = serde_json::Map::new();
                for cand in parsed_candidates {
                    let candidate_id = format!("imp-{}", cand.id);
                    let has_cred = cand.api_key.is_some() || cand.api_key_env.is_some();
                    let credential_status = if !include_credentials {
                        "not_requested"
                    } else if has_cred {
                        "available"
                    } else {
                        "unavailable"
                    };
                    if has_cred {
                        internal_secrets.insert(
                            candidate_id.clone(),
                            json!({
                                "api_key": cand.api_key,
                                "api_key_env": cand.api_key_env
                            }),
                        );
                    }
                    let existing_revision = existing_conn_revs
                        .get(&cand.id)
                        .copied()
                        .map(|r| json!(r))
                        .unwrap_or(Value::Null);
                    let models: Vec<Value> = cand
                        .models
                        .iter()
                        .map(|m| {
                            let caps = infer_model_capabilities(&m.model_id, None);
                            let mut modalities = vec!["text".to_string()];
                            if caps.image_input {
                                modalities.push("image".to_string());
                            }
                            json!({
                                "model_id": m.model_id,
                                "display_name": m.display_name,
                                "protocol": m.protocol,
                                "context_window": m.context_window,
                                "modalities": modalities,
                                "reasoning_levels": caps.supported_reasoning_levels
                            })
                        })
                        .collect();
                    candidates.push(json!({
                        "candidate_id": candidate_id,
                        "connection": {
                            "id": cand.id,
                            "display_name": cand.display_name,
                            "base_url": cand.base_url,
                            "protocols": [cand.protocol],
                            "source": cand.source,
                            "source_id": Value::Null,
                            "account_id": Value::Null,
                            "enabled": true
                        },
                        "models": models,
                        "credential_status": credential_status,
                        "issue": Value::Null,
                        "existing_revision": existing_revision
                    }));
                }

                let public_plan = json!({
                    "plan_id": plan_id,
                    "expires_at_ms": expires_at_ms,
                    "candidates": candidates
                });
                let stored_plan = json!({
                    "plan_id": plan_id,
                    "expires_at_ms": expires_at_ms,
                    "candidates": candidates,
                    "internal_secrets": Value::Object(internal_secrets)
                });
                let _ = store.set_kv(&format!("model_import_plan:{plan_id}"), &stored_plan);
                let _ = respond_api_json(req, 200, &public_plan, crypto_tok);
            }
            (Method::Post, "/api/model-import/commit") => {
                let plan_id = body
                    .get("plan_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let plan_key = format!("model_import_plan:{plan_id}");
                let plan = store.get_kv(&plan_key).ok().flatten();
                let Some(plan_val) = plan else {
                    let _ = respond_api_json(
                        req,
                        404,
                        &json!({"error": "import_plan_not_found"}),
                        crypto_tok,
                    );
                    return;
                };
                if let Some(exp) = plan_val.get("expires_at_ms").and_then(|v| v.as_i64()) {
                    if now_ms() >= exp {
                        let _ = store.set_kv(&plan_key, &Value::Null);
                        let _ = respond_api_json(
                            req,
                            400,
                            &json!({"error": "import_plan_expired"}),
                            crypto_tok,
                        );
                        return;
                    }
                }
                let candidates = plan_val
                    .get("candidates")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default();
                let internal_secrets = plan_val
                    .get("internal_secrets")
                    .and_then(|v| v.as_object())
                    .cloned()
                    .unwrap_or_default();
                let selections = body
                    .get("selections")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default();
                let mut cfg = load_active_config(&store);
                let mut results: Vec<Value> = Vec::new();
                let mut any_imported = false;
                for sel in selections {
                    let cid = sel
                        .get("candidate_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let update_existing = sel
                        .get("update_existing")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    let enable_models = sel
                        .get("enable_models")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    let display_override = sel
                        .get("display_name")
                        .and_then(|v| v.as_str())
                        .map(str::trim)
                        .filter(|s| !s.is_empty());

                    if let Some(cand) = candidates
                        .iter()
                        .find(|c| c.get("candidate_id").and_then(|v| v.as_str()) == Some(cid))
                    {
                        let conn_id = cand
                            .pointer("/connection/id")
                            .and_then(|v| v.as_str())
                            .unwrap_or(cid)
                            .to_string();
                        let display_name = display_override
                            .or_else(|| cand.pointer("/connection/display_name").and_then(|v| v.as_str()))
                            .unwrap_or(&conn_id)
                            .to_string();
                        let base_url = cand
                            .pointer("/connection/base_url")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let wire_proto = cand
                            .pointer("/connection/protocols/0")
                            .and_then(|v| v.as_str())
                            .unwrap_or("chat_completions")
                            .to_string();
                        let rust_proto = match wire_proto.as_str() {
                            "anthropic" => "anthropic",
                            "responses" | "openai_responses" => "openai_responses",
                            "gemini" | "google_gemini" => "gemini",
                            _ => "openai_chat",
                        };
                        let conn_source = cand
                            .pointer("/connection/source")
                            .and_then(|v| v.as_str())
                            .unwrap_or("import")
                            .to_string();

                        let already_exists = cfg.llm.connections.iter().any(|c| c.id == conn_id)
                            || suite
                                .get_record::<Value>("model_connection", &conn_id)
                                .ok()
                                .flatten()
                                .is_some();
                        if already_exists && !update_existing && cand.get("existing_revision").is_some_and(|v| !v.is_null()) {
                            results.push(json!({
                                "candidate_id": cid,
                                "connection_id": conn_id,
                                "status": "skipped",
                                "error": "connection_already_exists"
                            }));
                            continue;
                        }

                        let sec_entry = internal_secrets.get(cid);
                        let imported_api_key = sec_entry
                            .and_then(|s| s.get("api_key"))
                            .and_then(|v| v.as_str())
                            .map(ToString::to_string);
                        let imported_api_key_env = sec_entry
                            .and_then(|s| s.get("api_key_env"))
                            .and_then(|v| v.as_str())
                            .map(ToString::to_string);

                        let raw_models = cand
                            .get("models")
                            .and_then(|v| v.as_array())
                            .cloned()
                            .unwrap_or_default();
                        let models: Vec<ModelEntry> = raw_models
                            .iter()
                            .filter_map(|m| {
                                m.get("model_id").and_then(|v| v.as_str()).map(|id| {
                                    ModelEntry {
                                        id: id.to_string(),
                                        context_window: m
                                            .get("context_window")
                                            .and_then(|v| v.as_u64())
                                            .map(|n| n as usize),
                                        max_output_tokens: None,
                                        temperature: None,
                                        top_p: None,
                                        reasoning_effort: None,
                                        capabilities: None,
                                    }
                                })
                            })
                            .collect();

                        if let Some(existing) =
                            cfg.llm.connections.iter_mut().find(|c| c.id == conn_id)
                        {
                            existing.endpoint = base_url.clone();
                            existing.protocol = Some(rust_proto.into());
                            existing.provider = Some(conn_source.clone());
                            if let Some(k) = imported_api_key.clone() {
                                existing.api_key = Some(k);
                            }
                            if let Some(env_k) = imported_api_key_env.clone() {
                                existing.api_key_env = Some(env_k);
                            }
                            for m in &models {
                                if !existing.models.iter().any(|e| e.id == m.id) {
                                    existing.models.push(m.clone());
                                }
                            }
                        } else {
                            cfg.llm.connections.push(ConnectionConfig {
                                id: conn_id.clone(),
                                endpoint: base_url.clone(),
                                protocol: Some(rust_proto.into()),
                                provider: Some(conn_source.clone()),
                                api_key: imported_api_key,
                                api_key_env: imported_api_key_env,
                                models: models.clone(),
                            });
                        }

                        // Upsert `model_connection` record alongside `cfg.llm.connections`
                        let prev_rec_rev = suite
                            .get_record::<Value>("model_connection", &conn_id)
                            .ok()
                            .flatten()
                            .and_then(|r| r.get("revision").and_then(|v| v.as_i64()))
                            .unwrap_or(0);
                        let conn_record = json!({
                            "id": conn_id,
                            "display_name": display_name,
                            "base_url": base_url,
                            "endpoint": base_url,
                            "protocol": wire_proto,
                            "protocols": [wire_proto],
                            "source": conn_source,
                            "source_id": Value::Null,
                            "account_id": Value::Null,
                            "enabled": true,
                            "revision": prev_rec_rev + 1,
                            "catalog": {
                                "observed_at_ms": now_ms(),
                                "models": raw_models
                            },
                            "capability_assessments": [],
                            "updated_at_ms": now_ms()
                        });
                        let _ = suite.upsert_record("model_connection", &conn_id, &conn_record);

                        let enabled_keys: Vec<String> = if enable_models {
                            models
                                .iter()
                                .map(|m| format!("{wire_proto}:{}", m.id))
                                .collect()
                        } else {
                            Vec::new()
                        };
                        if enable_models {
                            let _ = store.set_kv(
                                &format!("model_conn_selections:{conn_id}"),
                                &json!({"enabled": enabled_keys}),
                            );
                            if cfg.llm.default_model.is_none() {
                                if let Some(first_m) = models.first() {
                                    cfg.llm.default_model = Some(DefaultModelSelection {
                                        connection: conn_id.clone(),
                                        model: first_m.id.clone(),
                                    });
                                }
                            }
                        }

                        any_imported = true;
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
                if any_imported {
                    let _ = save_applied_config(&store, &cfg);
                    sync_web_model_profiles_cache(&store);
                }
                // Clear stored plan (including any ephemeral credentials) after commit
                let _ = store.set_kv(&plan_key, &Value::Null);
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
                    match discover_endpoint_models(conn) {
                        Ok(models) => {
                            let _ = respond_api_json(req, 200, &json!({"models": models}), crypto_tok);
                        }
                        Err(err) => {
                            let status = if err.contains("empty") || err.contains("invalid") {
                                400
                            } else {
                                502
                            };
                            let _ = respond_api_json(
                                req,
                                status,
                                &json!({
                                    "ok": false,
                                    "error": "model_discovery_failed",
                                    "message": err
                                }),
                                crypto_tok,
                            );
                        }
                    }
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
                        sync_agent_settings_mcp_servers(&store);
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
                        sync_agent_settings_mcp_servers(&store);
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
                        sync_agent_settings_mcp_servers(&store);
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
                        let req_client_id = body
                            .get("client_id")
                            .or_else(|| cmd.get("client_id"))
                            .cloned()
                            .unwrap_or(Value::Null);
                        let _ = respond_api_json(
                            req,
                            200,
                            &json!({
                                "ok": true,
                                "host_id": host_id,
                                "client_id": req_client_id,
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

