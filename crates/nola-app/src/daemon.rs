// SPDX-License-Identifier: Apache-2.0
//! Single-writer Unix Domain Socket Application Service daemon (`nola server`).
//!
//! Synced with `apps/macos/Services/MacApplicationServiceClient.swift`,
//! `apps/ios/Services/IOSApplicationRuntime.swift`, and
//! `apps/android/app/src/main/java/com/nola/data/local/AndroidApplicationRuntime.kt`:
//! - Acquires exclusive `ServiceLock` and publishes `<db>.service.json` atomically.
//! - Authenticates every connecting peer via `SO_PEERCRED` / `getpeereid` (same EUID only).
//! - Persists canonical SHA-256 payload-bound idempotent `request_id` receipts.
//! - Spawns background automation scheduler and drains gracefully on parent-PID exit / signals.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::Utc;
use nola_engine::{compact_session_now, load_session_plan, load_session_todos, load_session_usage};
use nola_llm::LlmClient;
use nola_modules::a2ui::{
    build_nodes_from_spec, catalog_digest, commit_ui_interaction, delete_ui_surface,
    list_conversation_surfaces, preview_ui_surface, read_ui_surface_changes,
    read_ui_surface_revision, upsert_ui_surface, UiDraftPreviewInput, UiInteraction,
    UiSurfaceIdentity, UiSurfaceOwner, UiSurfaceRevisionRef, DEFAULT_CATALOG_ID,
    MAX_ACTIVE_SURFACES_PER_CONVERSATION,
};
use nola_modules::rooms_peers::built_in_role_templates;
use nola_modules::scheduler::spawn_background_scheduler;
use nola_modules::ModulesSuite;
use nola_store::{default_socket_path, EventPage, ServiceLock, Store, PROTOCOL_VERSION};
use nola_types::{
    new_id, now_ms, sha256_hex, AgentRoom, ExpertPeer, GoalRecord, GoalStatus, McpServerConfig,
    McpTransport, MemoryItem, PermissionProfile, Session,
};
use serde_json::{json, Value};

#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};

const MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;
const MAX_CONCURRENT_DAEMON_CLIENTS: usize = 32;

pub fn run_daemon(
    store: Store,
    suite: Arc<ModulesSuite>,
    db_path: &Path,
    args: &[String],
) -> Result<u8, String> {
    #[cfg(not(unix))]
    {
        let _ = (store, suite, db_path, args);
        return Err("UDS daemon requires Unix".into());
    }

    #[cfg(unix)]
    {
        let mut socket_path = default_socket_path(db_path);
        let mut parent_pid: Option<i32> = None;
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "--socket" => {
                    if let Some(p) = args.get(i + 1) {
                        socket_path = PathBuf::from(p);
                    }
                    i += 1;
                }
                "--parent-pid" => {
                    if let Some(p) = args.get(i + 1).and_then(|s| s.parse::<i32>().ok()) {
                        if p > 1 {
                            parent_pid = Some(p);
                        }
                    }
                    i += 1;
                }
                "--archive-incompatible-store" => {
                    i += 1;
                }
                _ => {}
            }
            i += 1;
        }

        let _lock = ServiceLock::acquire(db_path, &socket_path)
            .map_err(|e| format!("failed to acquire single-writer lock: {e}"))?;
        if socket_path.exists() {
            let _ = std::fs::remove_file(&socket_path);
        }
        let listener = UnixListener::bind(&socket_path)
            .map_err(|e| format!("failed to bind UDS {}: {e}", socket_path.display()))?;
        listener
            .set_nonblocking(true)
            .map_err(|e| format!("failed to set UDS nonblocking: {e}"))?;
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&socket_path, std::fs::Permissions::from_mode(0o600));
        }

        let shutdown = Arc::new(AtomicBool::new(false));
        let _ = signal_hook::flag::register(signal_hook::consts::SIGINT, Arc::clone(&shutdown));
        let _ = signal_hook::flag::register(signal_hook::consts::SIGTERM, Arc::clone(&shutdown));

        let cfg = crate::config::load_active_config(&store);
        let route = crate::config::resolve_model_route(&cfg).ok();
        let llm = LlmClient::new(120).ok().map(Arc::new);
        let scheduler_stop = spawn_background_scheduler(
            store.clone(),
            suite.workspace_root.clone(),
            llm,
            route,
        );

        let parent_watcher = parent_pid.map(|ppid| {
            let sock_cleanup = socket_path.clone();
            let shutdown_flag = Arc::clone(&shutdown);
            std::thread::spawn(move || {
                while !shutdown_flag.load(Ordering::Acquire) {
                    std::thread::sleep(Duration::from_millis(500));
                    let alive = unsafe { libc::kill(ppid, 0) == 0 }
                        || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM);
                    if !alive {
                        let _ = std::fs::remove_file(&sock_cleanup);
                        shutdown_flag.store(true, Ordering::Release);
                        break;
                    }
                }
            })
        });

        println!(
            "Nola v2 Application Service listening on {} (protocol v{})",
            socket_path.display(),
            PROTOCOL_VERSION
        );
        let _ = std::io::stdout().flush();

        let active_clients = Arc::new(AtomicUsize::new(0));
        let mut client_handles: Vec<std::thread::JoinHandle<()>> = Vec::new();

        while !shutdown.load(Ordering::Acquire) {
            client_handles.retain(|h| !h.is_finished());
            match listener.accept() {
                Ok((stream, _)) => {
                    let _ = stream.set_nonblocking(false);
                    if !peer_matches_euid(&stream) {
                        continue;
                    }
                    if active_clients.fetch_add(1, Ordering::SeqCst) >= MAX_CONCURRENT_DAEMON_CLIENTS
                    {
                        active_clients.fetch_sub(1, Ordering::SeqCst);
                        continue;
                    }
                    let store_c = store.clone();
                    let suite_c = Arc::clone(&suite);
                    let active_c = Arc::clone(&active_clients);
                    let handle = std::thread::spawn(move || {
                        handle_client(stream, &store_c, &suite_c);
                        active_c.fetch_sub(1, Ordering::SeqCst);
                    });
                    client_handles.push(handle);
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(_) => {
                    if shutdown.load(Ordering::Acquire) {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
        }

        // Graceful shutdown: stop scheduler, unlink socket, and drain active clients briefly.
        scheduler_stop.store(true, Ordering::Relaxed);
        let _ = std::fs::remove_file(&socket_path);

        let drain_deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < drain_deadline && active_clients.load(Ordering::SeqCst) > 0 {
            std::thread::sleep(Duration::from_millis(25));
        }
        for handle in client_handles {
            if handle.is_finished() {
                let _ = handle.join();
            }
        }
        if let Some(watcher) = parent_watcher {
            if watcher.is_finished() {
                let _ = watcher.join();
            }
        }

        Ok(0)
    }
}

#[cfg(unix)]
fn peer_matches_euid(stream: &UnixStream) -> bool {
    #[cfg(any(
        target_os = "linux",
        target_os = "macos",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd"
    ))]
    use std::os::unix::io::AsRawFd;
    #[cfg(target_os = "linux")]
    {
        let mut cred = libc::ucred {
            pid: 0,
            uid: 0,
            gid: 0,
        };
        let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        let rc = unsafe {
            libc::getsockopt(
                stream.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                (&mut cred as *mut libc::ucred).cast(),
                &mut len,
            )
        };
        rc == 0 && cred.uid == unsafe { libc::geteuid() }
    }
    #[cfg(any(
        target_os = "macos",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd"
    ))]
    {
        let mut euid: libc::uid_t = 0;
        let mut egid: libc::gid_t = 0;
        let rc = unsafe { libc::getpeereid(stream.as_raw_fd(), &mut euid, &mut egid) };
        rc == 0 && euid == unsafe { libc::geteuid() }
    }
    #[cfg(not(any(
        target_os = "linux",
        target_os = "macos",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd"
    )))]
    {
        let _ = stream;
        false
    }
}

#[cfg(unix)]
fn handle_client(mut stream: UnixStream, store: &Store, suite: &ModulesSuite) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(30)));
    let cloned = match stream.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    };
    let mut reader = BufReader::new(cloned.take((MAX_FRAME_BYTES as u64) + 1));
    let mut line = String::new();
    let n = match reader.read_line(&mut line) {
        Ok(0) => return,
        Ok(n) => n,
        Err(_) => return,
    };
    let resp = if n > MAX_FRAME_BYTES {
        json!({
            "protocol_version": PROTOCOL_VERSION,
            "status": "rejected",
            "error": {"message": format!("frame exceeds {MAX_FRAME_BYTES} byte limit")}
        })
    } else {
        match serde_json::from_str::<Value>(line.trim()) {
            Ok(req) => handle_envelope(store, suite, &req),
            Err(e) => json!({
                "protocol_version": PROTOCOL_VERSION,
                "status": "rejected",
                "error": {"message": format!("invalid JSON request: {e}")}
            }),
        }
    };
    if let Ok(mut bytes) = serde_json::to_vec(&resp) {
        bytes.push(b'\n');
        let _ = stream.write_all(&bytes);
    }
}

fn canonical_json_value(v: &Value) -> Value {
    match v {
        Value::Object(map) => {
            let mut sorted: BTreeMap<&String, Value> = BTreeMap::new();
            for (k, val) in map {
                sorted.insert(k, canonical_json_value(val));
            }
            let mut out = serde_json::Map::new();
            for (k, val) in sorted {
                out.insert(k.clone(), val);
            }
            Value::Object(out)
        }
        Value::Array(arr) => Value::Array(arr.iter().map(canonical_json_value).collect()),
        other => other.clone(),
    }
}

fn command_payload_digest(req: &Value) -> String {
    let canonical = canonical_json_value(req.get("command").unwrap_or(&Value::Null));
    sha256_hex(canonical.to_string().as_bytes())
}

fn extract_sub_command<'a>(args: &'a Value) -> (&'a str, &'a Value) {
    if let Some(sub) = args.get("command").or_else(|| args.get("operation")) {
        let op = sub
            .get("type")
            .or(Some(sub))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let payload = if sub.is_object() { sub } else { args };
        return (op, payload);
    }
    if let Some(action) = args.get("action").and_then(|v| v.as_str()) {
        return (action, args);
    }
    ("list", args)
}

fn is_read_only_sub_op(op: &str) -> bool {
    matches!(
        op,
        "list"
            | "list_profiles"
            | "list_editors"
            | "list_usage"
            | "list_rooms"
            | "list_experts"
            | "list_role_templates"
            | "role_templates"
            | "list_templates"
            | "get"
            | "get_room"
            | "read"
            | "search"
            | "query"
            | "recall"
            | "observe"
            | "status"
    )
}

fn is_mutating_command(cmd_type: &str, cmd_args: &Value) -> bool {
    match cmd_type {
        "session_create"
        | "create_conversation"
        | "rename_conversation"
        | "patch_conversation_configuration"
        | "session_delete"
        | "delete_conversation"
        | "truncate_conversation"
        | "compact_conversation"
        | "branch_conversation"
        | "render_ui_surface"
        | "submit_ui_interaction"
        | "delete_ui_surface"
        | "module_upsert"
        | "module_delete"
        | "memory_save"
        | "memory_delete"
        | "put_product_setting"
        | "set_conversation_permission"
        | "select_conversation_execution"
        | "step_goal"
        | "respond_to_conversation_wait"
        | "request_run_cancellation"
        | "backup" => true,
        "model"
        | "skill"
        | "memory"
        | "automation"
        | "mcp"
        | "data_source"
        | "agent_room"
        | "host_expert_template_operation"
        | "native_expert_host_operation" => {
            let (op, _) = extract_sub_command(cmd_args);
            !is_read_only_sub_op(op)
        }
        "conversation_plan" => {
            let (op, _) = extract_sub_command(cmd_args);
            cmd_args.get("plan").is_some() || !is_read_only_sub_op(op)
        }
        _ => false,
    }
}

/// Process a complete UDS / FFI JSON envelope with canonical-payload idempotent `request_id` receipts.
/// Conforms strictly to Android `ApplicationReceiptValidation` & iOS/macOS receipt contracts.
pub fn handle_envelope(store: &Store, suite: &ModulesSuite, req: &Value) -> Value {
    let received_at = Utc::now().to_rfc3339();
    let request_id = req
        .get("request_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let client_id = req
        .get("client_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let cmd_type = req
        .get("command")
        .and_then(|c| c.get("type"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let cmd_args = req
        .get("command")
        .and_then(|c| c.get("arguments").or(Some(c)))
        .cloned()
        .unwrap_or_else(|| json!({}));

    let is_mutation = is_mutating_command(cmd_type, &cmd_args);
    let payload_digest = if is_mutation && !request_id.is_empty() {
        Some(command_payload_digest(req))
    } else {
        None
    };

    if let Some(ref digest) = payload_digest {
        let receipt_key = format!("cmd_receipt:{request_id}");
        if let Ok(Some(existing)) = store.get_kv(&receipt_key) {
            if let (Some(stored_digest), Some(stored_receipt)) = (
                existing.get("payload_digest").and_then(|v| v.as_str()),
                existing.get("receipt"),
            ) {
                if stored_digest == digest {
                    return stored_receipt.clone();
                }
                return json!({
                    "protocol_version": PROTOCOL_VERSION,
                    "client_id": client_id,
                    "request_id": request_id,
                    "received_at": received_at,
                    "completed_at": Utc::now().to_rfc3339(),
                    "status": "rejected",
                    "error": {
                        "code": "idempotency_conflict",
                        "message": format!("request_id '{request_id}' was already used with a different command payload"),
                        "recoverable": false
                    }
                });
            }
            return existing;
        }
    }

    let completed_at = Utc::now().to_rfc3339();
    let resp = match dispatch_command(store, suite, cmd_type, &cmd_args) {
        Ok(result) => {
            if is_mutation {
                let next_rev = store
                    .get_kv("app_committed_revision")
                    .ok()
                    .flatten()
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0)
                    + 1;
                let _ = store.set_kv("app_committed_revision", &json!(next_rev));
                json!({
                    "protocol_version": PROTOCOL_VERSION,
                    "client_id": client_id,
                    "request_id": request_id,
                    "received_at": received_at,
                    "completed_at": completed_at,
                    "status": "committed",
                    "committed_revision": next_rev,
                    "result": result
                })
            } else {
                json!({
                    "protocol_version": PROTOCOL_VERSION,
                    "client_id": client_id,
                    "request_id": request_id,
                    "received_at": received_at,
                    "completed_at": completed_at,
                    "status": "committed",
                    "result": result
                })
            }
        }
        Err((code, msg)) => json!({
            "protocol_version": PROTOCOL_VERSION,
            "client_id": client_id,
            "request_id": request_id,
            "received_at": received_at,
            "completed_at": completed_at,
            "status": "rejected",
            "error": {
                "code": code,
                "message": msg,
                "recoverable": false
            }
        }),
    };

    if let Some(digest) = payload_digest {
        let wrapped = json!({
            "payload_digest": digest,
            "receipt": resp
        });
        let _ = store.set_kv(&format!("cmd_receipt:{request_id}"), &wrapped);
    }
    resp
}

pub fn dispatch_command(
    store: &Store,
    suite: &ModulesSuite,
    cmd_type: &str,
    args: &Value,
) -> Result<Value, (&'static str, String)> {
    match cmd_type {
        "health" => {
            Ok(json!({"ok": true, "version": "2.0.0", "protocol_version": PROTOCOL_VERSION}))
        }
        "command_receipt" => {
            let rid = args.get("request_id").and_then(|v| v.as_str()).unwrap_or("");
            let rec = store
                .get_kv(&format!("cmd_receipt:{rid}"))
                .map_err(|e| ("storage_error", e.to_string()))?;
            let unwrapped = match rec {
                Some(v) => v.get("receipt").cloned().unwrap_or(v),
                None => Value::Null,
            };
            Ok(unwrapped)
        }
        "projection_snapshot" | "sessions_list" | "list_conversations" => {
            let sessions = store
                .list_sessions(200)
                .map_err(|e| ("storage_error", e.to_string()))?;
            let conversations: Vec<Value> = sessions
                .iter()
                .map(|s| crate::web_api::session_to_conversation_json(store, s))
                .collect();
            let goals: Vec<Value> = suite.list_records("goal").unwrap_or_default();
            let rooms: Vec<Value> = suite.list_records("agent_room").unwrap_or_default();
            Ok(json!({
                "sessions": sessions,
                "conversations": conversations,
                "goals": goals,
                "rooms": rooms
            }))
        }
        "list_root_conversations" => {
            let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(200) as usize;
            let sessions = store
                .list_sessions(limit.clamp(1, 500))
                .map_err(|e| ("storage_error", e.to_string()))?;
            let root_sessions: Vec<&Session> = sessions
                .iter()
                .filter(|s| !s.id.contains(':'))
                .collect();
            let conversations: Vec<Value> = root_sessions
                .iter()
                .map(|s| crate::web_api::session_to_conversation_json(store, s))
                .collect();
            Ok(json!({
                "conversations": conversations,
                "sessions": root_sessions
            }))
        }
        "load_conversation"
        | "load_conversation_projection_v1"
        | "load_conversation_projection_window_v1" => {
            let id = args
                .get("conversation_id")
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing conversation_id".into()))?;
            let sess = store
                .get_session(id)
                .map_err(|e| ("storage_error", e.to_string()))?
                .ok_or(("not_found", format!("conversation '{id}' not found")))?;
            let conv = crate::web_api::session_to_conversation_json(store, &sess);
            let (messages, events) = crate::web_api::reconstruct_v2_messages(store, id);
            Ok(json!({
                "conversation": conv,
                "messages": messages,
                "events": events,
                "has_more": false
            }))
        }
        "load_session_projection" => {
            let id = args
                .get("conversation_id")
                .or_else(|| args.get("session_id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing conversation_id".into()))?;
            let todos = load_session_todos(store, id);
            let plan = load_session_plan(store, id);
            let usage = load_session_usage(store, id);
            Ok(json!({
                "conversation_id": id,
                "todos": todos,
                "plan": plan,
                "usage": usage
            }))
        }
        "load_conversation_context_usage_v1" => {
            let id = args
                .get("conversation_id")
                .or_else(|| args.get("session_id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing conversation_id".into()))?;
            let usage = load_session_usage(store, id);
            let mut val = serde_json::to_value(&usage).unwrap_or_else(|_| json!({}));
            if let Some(obj) = val.as_object_mut() {
                obj.insert("conversation_id".into(), json!(id));
                obj.insert("usage".into(), serde_json::to_value(&usage).unwrap_or(json!({})));
            }
            Ok(val)
        }
        "read_permission_configuration" => {
            let id = args
                .get("conversation_id")
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str());
            let (profile, rev) = read_permission_state(store, id);
            Ok(json!({
                "conversation_id": id,
                "permission_profile": profile,
                "permission_mode": profile,
                "revision": rev,
                "configuration_revision": rev
            }))
        }
        "load_conversation_permission_projection_v1" => {
            let id = args
                .get("conversation_id")
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing conversation_id".into()))?;
            let (profile, rev) = read_permission_state(store, Some(id));
            Ok(json!({
                "conversation_id": id,
                "permission_profile": profile,
                "revision": rev,
                "pending_requests": [],
                "grants": []
            }))
        }
        "active_run" => {
            let conv_id = args.get("conversation_id").and_then(|v| v.as_str());
            Ok(json!({
                "active": false,
                "run": Value::Null,
                "conversation_id": conv_id
            }))
        }
        "load_conversation_export_page" => {
            let id = args
                .get("conversation_id")
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing conversation_id".into()))?;
            let cursor = args
                .get("cursor")
                .or_else(|| args.get("offset"))
                .and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|s| s.parse().ok())))
                .unwrap_or(0) as usize;
            let limit = args
                .get("limit")
                .or_else(|| args.get("page_size"))
                .and_then(|v| v.as_u64())
                .unwrap_or(100)
                .clamp(1, 500) as usize;
            let (messages, events) = crate::web_api::reconstruct_v2_messages(store, id);
            let total = messages.len();
            let end = (cursor + limit).min(total);
            let page_msgs = if cursor < total {
                messages[cursor..end].to_vec()
            } else {
                Vec::new()
            };
            let ev_total = events.len();
            let ev_end = (cursor + limit).min(ev_total);
            let page_evs = if cursor < ev_total {
                events[cursor..ev_end].to_vec()
            } else {
                Vec::new()
            };
            let has_more = end < total || ev_end < ev_total;
            Ok(json!({
                "conversation_id": id,
                "messages": page_msgs,
                "events": page_evs,
                "cursor": cursor,
                "next_cursor": if has_more { json!(cursor + limit) } else { Value::Null },
                "has_more": has_more,
                "total_messages": total,
                "total_events": ev_total
            }))
        }
        "read_conversation_export_range" => {
            let id = args
                .get("conversation_id")
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing conversation_id".into()))?;
            let start_seq = args
                .get("start_seq")
                .or_else(|| args.get("from_seq"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let end_seq = args
                .get("end_seq")
                .or_else(|| args.get("to_seq"))
                .and_then(|v| v.as_i64());
            let (messages, events) = crate::web_api::reconstruct_v2_messages(store, id);
            let in_range = |v: &Value| -> bool {
                let seq = v.get("seq").and_then(|x| x.as_i64()).unwrap_or(0);
                seq >= start_seq && end_seq.is_none_or(|e| seq <= e)
            };
            let filtered_msgs: Vec<Value> = messages.into_iter().filter(&in_range).collect();
            let filtered_evs: Vec<Value> = events.into_iter().filter(&in_range).collect();
            Ok(json!({
                "conversation_id": id,
                "start_seq": start_seq,
                "end_seq": end_seq,
                "messages": filtered_msgs,
                "events": filtered_evs
            }))
        }
        "list_activity" => {
            let limit = args
                .get("limit")
                .and_then(|v| v.as_u64())
                .unwrap_or(50)
                .clamp(1, 200) as usize;
            let sessions = store
                .list_sessions(limit)
                .map_err(|e| ("storage_error", e.to_string()))?;
            let mut entries = Vec::new();
            for s in sessions {
                let (msgs, evs) = crate::web_api::reconstruct_v2_messages(store, &s.id);
                let last_summary = msgs
                    .last()
                    .and_then(|m| m.get("content").and_then(|v| v.as_str()))
                    .map(|t| t.chars().take(160).collect::<String>())
                    .unwrap_or_default();
                entries.push(json!({
                    "id": s.id,
                    "activity_id": s.id,
                    "conversation_id": s.id,
                    "kind": "conversation",
                    "title": s.title,
                    "summary": last_summary,
                    "message_count": msgs.len(),
                    "event_count": evs.len(),
                    "created_at": crate::web_api::ms_to_rfc3339(s.created_at_ms),
                    "updated_at": crate::web_api::ms_to_rfc3339(s.updated_at_ms),
                    "updated_at_ms": s.updated_at_ms
                }));
            }
            Ok(json!({ "items": entries, "activities": entries }))
        }
        "load_activity_detail" => {
            let id = args
                .get("activity_id")
                .or_else(|| args.get("conversation_id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing activity_id".into()))?;
            let sess = store
                .get_session(id)
                .map_err(|e| ("storage_error", e.to_string()))?
                .ok_or(("not_found", format!("activity '{id}' not found")))?;
            let (messages, events) = crate::web_api::reconstruct_v2_messages(store, id);
            let usage = load_session_usage(store, id);
            let plan = load_session_plan(store, id);
            let todos = load_session_todos(store, id);
            Ok(json!({
                "id": sess.id,
                "conversation": crate::web_api::session_to_conversation_json(store, &sess),
                "messages": messages,
                "events": events,
                "usage": usage,
                "plan": plan,
                "todos": todos
            }))
        }
        "load_today_facts" => {
            let today = Utc::now().date_naive();
            let start_of_day_ms = today
                .and_hms_opt(0, 0, 0)
                .map(|dt| dt.and_utc().timestamp_millis())
                .unwrap_or(0);
            let sessions = store.list_sessions(200).unwrap_or_default();
            let sessions_today: Vec<Value> = sessions
                .iter()
                .filter(|s| s.updated_at_ms >= start_of_day_ms)
                .map(|s| {
                    json!({
                        "id": s.id,
                        "title": s.title,
                        "updated_at": crate::web_api::ms_to_rfc3339(s.updated_at_ms)
                    })
                })
                .collect();
            let goals: Vec<GoalRecord> = suite.list_records("goal").unwrap_or_default();
            let active_goals: Vec<Value> = goals
                .iter()
                .filter(|g| matches!(g.status, GoalStatus::Pending | GoalStatus::Running))
                .map(|g| json!({"id": g.id, "title": g.title, "status": g.status}))
                .collect();
            let completed_today: Vec<Value> = goals
                .iter()
                .filter(|g| g.status == GoalStatus::Completed && g.updated_at_ms >= start_of_day_ms)
                .map(|g| json!({"id": g.id, "title": g.title, "status": g.status}))
                .collect();
            let facts = vec![
                format!("{} conversation(s) active today", sessions_today.len()),
                format!("{} active goal(s)", active_goals.len()),
                format!("{} goal(s) completed today", completed_today.len()),
            ];
            Ok(json!({
                "date": today.to_string(),
                "sessions_today": sessions_today,
                "active_goals": active_goals,
                "completed_goals_today": completed_today,
                "facts": facts
            }))
        }
        "agent_room" => dispatch_agent_room_command(suite, args),
        "host_expert_template_operation" | "native_expert_host_operation" => {
            dispatch_expert_template_command(suite, args)
        }
        "conversation_plan" => {
            let id = args
                .get("conversation_id")
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing conversation_id".into()))?;
            let (op, payload) = extract_sub_command(args);
            if let Some(plan_val) = payload.get("plan").or_else(|| args.get("plan")) {
                let record = json!({
                    "session_id": id,
                    "plan": plan_val,
                    "updated_at_ms": now_ms()
                });
                store
                    .set_kv(&format!("plan:{id}"), &record)
                    .map_err(|e| ("storage_error", e.to_string()))?;
                return Ok(json!({
                    "ok": true,
                    "conversation_id": id,
                    "plan": record,
                    "todos": load_session_todos(store, id)
                }));
            }
            if op == "clear" || op == "delete" {
                store
                    .set_kv(&format!("plan:{id}"), &Value::Null)
                    .map_err(|e| ("storage_error", e.to_string()))?;
                return Ok(json!({"ok": true, "conversation_id": id, "plan": Value::Null}));
            }
            Ok(json!({
                "conversation_id": id,
                "plan": load_session_plan(store, id),
                "todos": load_session_todos(store, id)
            }))
        }
        "load_goal_control_observation" => {
            let goal_id = args
                .get("goal_id")
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str());
            let conv_id = args.get("conversation_id").and_then(|v| v.as_str());
            let goals: Vec<GoalRecord> = suite
                .list_records("goal")
                .map_err(|e| ("storage_error", e.to_string()))?;
            let goal = goal_id.and_then(|gid| goals.iter().find(|g| g.id == gid).cloned());
            let plan = conv_id
                .map(|cid| load_session_plan(store, cid))
                .unwrap_or(Value::Null);
            Ok(json!({
                "goal": goal,
                "goals": goals,
                "plan": plan
            }))
        }
        "step_goal" => {
            let goal_id = args
                .get("goal_id")
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'goal_id'".into()))?;
            let mut goal = suite
                .get_record::<GoalRecord>("goal", goal_id)
                .map_err(|e| ("storage_error", e.to_string()))?
                .ok_or(("not_found", format!("goal '{goal_id}' not found")))?;
            let action = args
                .get("action")
                .or_else(|| args.get("status"))
                .or_else(|| args.get("transition"))
                .and_then(|v| v.as_str())
                .unwrap_or("advance");
            goal.status = match action.trim().to_ascii_lowercase().as_str() {
                "start" | "run" | "running" | "resume" => GoalStatus::Running,
                "pause" | "paused" => GoalStatus::Paused,
                "complete" | "completed" | "finish" | "done" => GoalStatus::Completed,
                "fail" | "failed" | "abort" => GoalStatus::Failed,
                "reset" | "pending" => GoalStatus::Pending,
                "advance" | "step" => match goal.status {
                    GoalStatus::Pending | GoalStatus::Paused => GoalStatus::Running,
                    GoalStatus::Running => GoalStatus::Completed,
                    other => other,
                },
                other => {
                    return Err((
                        "invalid_argument",
                        format!("unsupported goal transition '{other}'"),
                    ));
                }
            };
            if let Some(note) = args
                .get("note")
                .or_else(|| args.get("progress_note"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
            {
                goal.progress_notes.push(note.to_string());
            }
            goal.updated_at_ms = now_ms();
            suite
                .upsert_record("goal", &goal.id, &goal)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"ok": true, "goal": goal}))
        }
        "list_open_user_input_waits" => {
            let conv_id = args.get("conversation_id").and_then(|v| v.as_str());
            let waits = conv_id
                .and_then(|cid| store.get_kv(&format!("open_waits:{cid}")).ok().flatten())
                .and_then(|v| v.as_array().cloned())
                .unwrap_or_default();
            Ok(json!({
                "conversation_id": conv_id,
                "waits": waits
            }))
        }
        "respond_to_conversation_wait" => {
            let conv_id = args
                .get("conversation_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let wait_id = args
                .get("wait_id")
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if !conv_id.is_empty() {
                let _ = store.set_kv(&format!("open_waits:{conv_id}"), &json!([]));
            }
            Ok(json!({
                "ok": true,
                "accepted": true,
                "conversation_id": conv_id,
                "wait_id": wait_id
            }))
        }
        "request_run_cancellation" => {
            let conv_id = args
                .get("conversation_id")
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if !conv_id.is_empty() {
                let _ = store.set_kv(
                    &format!("cancel_requested:{conv_id}"),
                    &json!({"requested_at_ms": now_ms()}),
                );
            }
            Ok(json!({
                "ok": true,
                "cancelled": true,
                "conversation_id": conv_id
            }))
        }
        "resolve_chat_model" => {
            let cfg = crate::config::load_active_config(store);
            let req_model = args
                .get("model_id")
                .or_else(|| args.get("model"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty());
            let req_conn = args
                .get("connection_id")
                .or_else(|| args.get("selected_model_source"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty());
            match crate::config::resolve_model_route(&cfg) {
                Ok(mut route) => {
                    if let Some(m) = req_model {
                        route.model = m.to_string();
                    }
                    if let Some(c) = req_conn {
                        if let Some(found) = cfg.llm.connections.iter().find(|x| x.id == c) {
                            route.connection_id = found.id.clone();
                            route.endpoint = found.endpoint.clone();
                        }
                    }
                    Ok(json!({
                        "resolved": true,
                        "connection_id": route.connection_id,
                        "model_id": route.model,
                        "protocol": route.protocol.as_str(),
                        "endpoint": route.endpoint,
                        "context_window": route.context_window,
                        "max_output_tokens": route.max_output_tokens,
                        "reasoning_effort": route.reasoning_effort,
                        "capabilities": route.capabilities
                    }))
                }
                Err(msg) => Ok(json!({
                    "resolved": false,
                    "error": msg
                })),
            }
        }
        "resolve_model_reasoning" => {
            let supports_reasoning = args
                .get("supports_reasoning")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let wire_protocol = args
                .get("wire_protocol")
                .and_then(|v| v.as_str())
                .unwrap_or("chat_completions");
            let output_limit = args.get("output_limit").and_then(|v| v.as_u64());
            let tier = args.get("tier").and_then(|v| v.as_str());
            let requested = args.get("requested").cloned().unwrap_or(Value::Null);
            let effective_effort = if !supports_reasoning {
                Value::Null
            } else if let Some(t) = tier {
                json!(t)
            } else if !requested.is_null() {
                requested
            } else {
                json!("automatic")
            };
            Ok(json!({
                "supported": supports_reasoning,
                "wire_protocol": wire_protocol,
                "output_limit": output_limit,
                "control": if supports_reasoning { "effort" } else { "none" },
                "value": effective_effort
            }))
        }
        "read_ui_capability" => Ok(json!({
            "supported": true,
            "catalog_id": DEFAULT_CATALOG_ID,
            "catalog_digest": catalog_digest(),
            "max_surfaces_per_conversation": MAX_ACTIVE_SURFACES_PER_CONVERSATION
        })),
        "session_create" | "create_conversation" => {
            let ts = now_ms();
            let s = Session {
                id: args
                    .get("id")
                    .or_else(|| args.get("conversation_id"))
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string)
                    .unwrap_or_else(new_id),
                title: args
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("New conversation")
                    .to_string(),
                workspace_root: args
                    .get("workspace_root")
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string)
                    .unwrap_or_else(|| suite.workspace_root.display().to_string()),
                model_connection: args
                    .get("selected_model_source")
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string),
                model_id: args
                    .get("selected_model_id")
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string),
                created_at_ms: ts,
                updated_at_ms: ts,
            };
            store
                .upsert_session(&s)
                .map_err(|e| ("storage_error", e.to_string()))?;
            let mut val = serde_json::to_value(&s).unwrap_or(json!({}));
            if let Some(obj) = val.as_object_mut() {
                obj.insert(
                    "conversation".into(),
                    crate::web_api::session_to_conversation_json(store, &s),
                );
            }
            Ok(val)
        }
        "rename_conversation" => {
            let id = args
                .get("conversation_id")
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing conversation_id".into()))?;
            let title = args
                .get("title")
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing title".into()))?;
            let mut sess = store
                .get_session(id)
                .map_err(|e| ("storage_error", e.to_string()))?
                .ok_or(("not_found", format!("conversation '{id}' not found")))?;
            sess.title = title.to_string();
            sess.updated_at_ms = now_ms();
            store
                .upsert_session(&sess)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(crate::web_api::session_to_conversation_json(store, &sess))
        }
        "patch_conversation_configuration" | "select_conversation_execution" => {
            let id = args
                .get("conversation_id")
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing conversation_id".into()))?;
            let mut sess = store
                .get_session(id)
                .map_err(|e| ("storage_error", e.to_string()))?
                .ok_or(("not_found", format!("conversation '{id}' not found")))?;
            if let Some(src) = args
                .get("selected_model_source")
                .or_else(|| args.get("selectedModelSource"))
                .and_then(|v| v.as_str())
            {
                sess.model_connection = (!src.is_empty()).then(|| src.to_string());
            }
            if let Some(mid) = args
                .get("selected_model_id")
                .or_else(|| args.get("selectedModelId"))
                .and_then(|v| v.as_str())
            {
                sess.model_id = (!mid.is_empty()).then(|| mid.to_string());
            }
            if let Some(pinned) = args
                .get("is_pinned")
                .or_else(|| args.get("isPinned"))
                .and_then(|v| v.as_bool())
            {
                let key = crate::web_api::session_meta_key(id);
                let mut meta = store
                    .get_kv(&key)
                    .ok()
                    .flatten()
                    .unwrap_or_else(
                        || json!({"isPinned": false, "revision": 1, "configurationRevision": 1}),
                    );
                meta["isPinned"] = json!(pinned);
                let next_cfg = meta
                    .get("configurationRevision")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(1)
                    + 1;
                meta["configurationRevision"] = json!(next_cfg);
                let _ = store.set_kv(&key, &meta);
            }
            sess.updated_at_ms = now_ms();
            store
                .upsert_session(&sess)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(crate::web_api::session_to_conversation_json(store, &sess))
        }
        "session_delete" | "delete_conversation" => {
            let id = args
                .get("id")
                .or_else(|| args.get("conversation_id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            store
                .delete_session(id)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"deleted": id, "ok": true}))
        }
        "events_after" | "projection_event_suffix" | "projection_cursor_wait" => {
            let scope = args
                .get("scope")
                .or_else(|| args.get("conversation_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let after_seq = args.get("after_seq").and_then(|v| v.as_i64()).unwrap_or(0);
            match store
                .events_after(scope, after_seq, 200)
                .map_err(|e| ("storage_error", e.to_string()))?
            {
                EventPage::Events(ev) => Ok(json!({"status": "ok", "events": ev})),
                EventPage::ResyncRequired {
                    earliest_retained_seq,
                    latest_seq,
                } => Ok(json!({
                    "status": "resync_required",
                    "earliest_retained_seq": earliest_retained_seq,
                    "latest_seq": latest_seq
                })),
            }
        }
        "get_product_setting" => {
            let key = args
                .get("key")
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'key'".into()))?;
            let val = store
                .get_kv(&format!("product_setting:{key}"))
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"key": key, "value": val.unwrap_or(Value::Null)}))
        }
        "put_product_setting" => {
            let key = args
                .get("key")
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'key'".into()))?;
            let value = args.get("value").cloned().unwrap_or(Value::Null);
            store
                .set_kv(&format!("product_setting:{key}"), &value)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"key": key, "value": value, "ok": true}))
        }
        "set_conversation_permission" => {
            let id = args
                .get("conversation_id")
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'conversation_id'".into()))?;
            let raw_mode = args
                .get("permission_mode")
                .or_else(|| args.get("mode"))
                .or_else(|| args.get("permission_profile"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'permission_profile'".into()))?;
            let parsed = PermissionProfile::parse_strict(raw_mode)
                .map_err(|e| ("invalid_argument", e))?;
            let canonical_mode = parsed.as_canonical_str();
            store
                .set_kv(&format!("conv_perm:{id}"), &json!(canonical_mode))
                .map_err(|e| ("storage_error", e.to_string()))?;
            let meta_key = crate::web_api::session_meta_key(id);
            let mut meta = store
                .get_kv(&meta_key)
                .ok()
                .flatten()
                .unwrap_or_else(
                    || json!({"isPinned": false, "revision": 1, "configurationRevision": 1}),
                );
            meta["permissionProfile"] = json!(canonical_mode);
            let next_cfg = meta
                .get("configurationRevision")
                .and_then(|v| v.as_i64())
                .unwrap_or(1)
                + 1;
            meta["configurationRevision"] = json!(next_cfg);
            let _ = store.set_kv(&meta_key, &meta);
            Ok(json!({
                "conversation_id": id,
                "permission_mode": canonical_mode,
                "permission_profile": canonical_mode,
                "configuration_revision": next_cfg,
                "ok": true
            }))
        }
        "truncate_conversation" => {
            let id = args
                .get("conversation_id")
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'conversation_id'".into()))?;
            let from_seq = args
                .get("from_seq")
                .or_else(|| args.get("fromSeq"))
                .and_then(|v| v.as_i64())
                .unwrap_or(1);
            let deleted = store
                .truncate_events_from(id, from_seq)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(
                json!({"ok": true, "conversation_id": id, "from_seq": from_seq, "deleted_events": deleted}),
            )
        }
        "compact_conversation" => {
            let id = args
                .get("conversation_id")
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'conversation_id'".into()))?;
            compact_session_now(store, None, None, id).map_err(|e| ("compaction_error", e))
        }
        "branch_conversation" => {
            let id = args
                .get("conversation_id")
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'conversation_id'".into()))?;
            let parent = store
                .get_session(id)
                .map_err(|e| ("storage_error", e.to_string()))?;
            let ts = now_ms();
            let new_conv_id = new_id();
            let sess = Session {
                id: new_conv_id.clone(),
                title: parent
                    .as_ref()
                    .map(|s| format!("{} (copy)", s.title))
                    .unwrap_or_else(|| "Branched conversation".into()),
                workspace_root: parent
                    .as_ref()
                    .map(|s| s.workspace_root.clone())
                    .unwrap_or_else(|| suite.workspace_root.display().to_string()),
                model_connection: parent.as_ref().and_then(|s| s.model_connection.clone()),
                model_id: parent.as_ref().and_then(|s| s.model_id.clone()),
                created_at_ms: ts,
                updated_at_ms: ts,
            };
            store
                .upsert_session(&sess)
                .map_err(|e| ("storage_error", e.to_string()))?;
            if let Ok(EventPage::Events(ev)) = store.events_after(id, 0, 500) {
                for e in ev {
                    let _ = store.append_event(&new_conv_id, &e.kind, e.payload, 50_000);
                }
            }
            Ok(
                json!({"conversation": crate::web_api::session_to_conversation_json(store, &sess)}),
            )
        }
        "workspace_review" => {
            let id = args
                .get("conversation_id")
                .and_then(|v| v.as_str())
                .unwrap_or("workspace");
            let host_id = store
                .get_kv("host_id")
                .ok()
                .flatten()
                .and_then(|v| v.as_str().map(ToString::to_string))
                .unwrap_or_else(|| "local-host".into());
            let ws_root = store
                .get_session(id)
                .ok()
                .flatten()
                .map(|s| PathBuf::from(s.workspace_root))
                .filter(|p| p.is_dir())
                .unwrap_or_else(|| suite.workspace_root.clone());
            Ok(crate::web_api::build_workspace_review(&host_id, id, &ws_root))
        }
        "render_ui_surface" => {
            let conv_id = args
                .get("conversation_id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .unwrap_or("default");
            let surface_id = args
                .get("surface_id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .ok_or(("invalid_argument", "missing 'surface_id'".into()))?;
            let summary = args
                .get("summary")
                .and_then(|v| v.as_str())
                .unwrap_or("Interactive UI Surface");
            let data_model = args.get("data_model").cloned().unwrap_or_else(|| json!({}));
            let (root_id, nodes) =
                build_nodes_from_spec(args).map_err(|e| ("invalid_ui_spec", e))?;
            let authority_id = args
                .get("authority_id")
                .and_then(|v| v.as_str())
                .map(ToString::to_string)
                .or_else(|| {
                    store
                        .get_kv("host_id")
                        .ok()
                        .flatten()
                        .and_then(|v| v.as_str().map(ToString::to_string))
                })
                .unwrap_or_else(|| "local-host".into());
            let owner = UiSurfaceOwner {
                authority_id,
                conversation_id: conv_id.to_string(),
                participant_id: "assistant".into(),
                context_id: "conversation".into(),
                originating_run_id: new_id(),
                message_id: args
                    .get("message_id")
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string)
                    .unwrap_or_else(|| format!("ui-{}", now_ms())),
                part_id: format!("part-{surface_id}"),
            };
            let snap =
                upsert_ui_surface(store, owner, surface_id, summary, root_id, nodes, data_model)
                    .map_err(|e| ("ui_surface_error", e))?;
            Ok(serde_json::to_value(snap).unwrap_or(json!({})))
        }
        "list_ui_surfaces" => {
            let conv_id = args
                .get("conversation_id")
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'conversation_id'".into()))?;
            let surfaces = list_conversation_surfaces(store, conv_id);
            Ok(serde_json::to_value(surfaces).unwrap_or_else(|_| json!([])))
        }
        "read_ui_surface_revision" => {
            let identity: UiSurfaceIdentity = serde_json::from_value(
                args.get("identity").cloned().unwrap_or(Value::Null),
            )
            .map_err(|e| ("invalid_argument", format!("invalid identity: {e}")))?;
            let revision = args
                .get("revision")
                .and_then(|v| v.as_u64())
                .ok_or(("invalid_argument", "missing 'revision'".into()))?;
            let snap =
                read_ui_surface_revision(store, &UiSurfaceRevisionRef { identity, revision })
                    .map_err(|e| ("ui_surface_error", e))?;
            Ok(serde_json::to_value(snap).unwrap_or(Value::Null))
        }
        "read_ui_surface_changes" => {
            let identity: UiSurfaceIdentity = serde_json::from_value(
                args.get("identity").cloned().unwrap_or(Value::Null),
            )
            .map_err(|e| ("invalid_argument", format!("invalid identity: {e}")))?;
            let base_revision = args.get("base_revision").and_then(|v| v.as_u64());
            read_ui_surface_changes(store, &identity, base_revision)
                .map_err(|e| ("ui_surface_error", e))
        }
        "preview_ui_surface" => {
            let raw_input = args.get("input").unwrap_or(args).clone();
            let input: UiDraftPreviewInput = serde_json::from_value(raw_input)
                .map_err(|e| ("invalid_argument", format!("invalid preview input: {e}")))?;
            let preview =
                preview_ui_surface(store, &input).map_err(|e| ("ui_preview_error", e))?;
            Ok(serde_json::to_value(preview).unwrap_or(json!({})))
        }
        "submit_ui_interaction" => {
            let raw_input = args.get("input").unwrap_or(args);
            let raw_interaction = raw_input.get("interaction").unwrap_or(raw_input).clone();
            let submission_id = raw_input
                .get("submission_id")
                .and_then(|v| v.as_str())
                .map(ToString::to_string);
            let interaction: UiInteraction = serde_json::from_value(raw_interaction)
                .map_err(|e| ("invalid_argument", format!("invalid UI interaction: {e}")))?;
            let committed = commit_ui_interaction(store, &interaction)
                .map_err(|e| ("ui_interaction_rejected", e))?;
            Ok(json!({
                "snapshot": committed.snapshot,
                "queued_submission_id": if committed.action_id.is_some() { submission_id } else { None },
                "action_id": committed.action_id,
                "summary": committed.summary,
                "data_model": committed.data_model
            }))
        }
        "delete_ui_surface" => {
            let conv_id = args
                .get("conversation_id")
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'conversation_id'".into()))?;
            let surface_id = args
                .get("surface_id")
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'surface_id'".into()))?;
            let snap = delete_ui_surface(store, conv_id, surface_id)
                .map_err(|e| ("ui_surface_error", e))?;
            Ok(serde_json::to_value(snap).unwrap_or(Value::Null))
        }
        "module_list" => {
            let kind = args.get("kind").and_then(|v| v.as_str()).unwrap_or("goal");
            let items: Vec<Value> = suite
                .list_records(kind)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"kind": kind, "items": items}))
        }
        "module_upsert" => {
            let kind = args
                .get("kind")
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'kind'".into()))?;
            let record = args
                .get("record")
                .ok_or(("invalid_argument", "missing 'record'".into()))?;
            let id = record
                .get("id")
                .and_then(|v| v.as_str())
                .map(ToString::to_string)
                .unwrap_or_else(new_id);
            suite
                .upsert_record(kind, &id, record)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"ok": true, "id": id}))
        }
        "module_delete" => {
            let kind = args
                .get("kind")
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'kind'".into()))?;
            let id = args
                .get("id")
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            let deleted = suite
                .delete_record(kind, id)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"deleted": deleted}))
        }
        "memory_search" => {
            let q = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
            let ns = args.get("namespace").and_then(|v| v.as_str());
            let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(50) as usize;
            let items = store
                .search_memory(q, ns, limit)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"items": items}))
        }
        "memory_save" => {
            let item = parse_memory_item(args);
            store
                .upsert_memory(&item)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(serde_json::to_value(&item).unwrap_or(json!({})))
        }
        "memory_delete" => {
            let id = args
                .get("id")
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            store
                .delete_memory(id)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"deleted": id, "ok": true}))
        }
        "model" | "skill" | "memory" | "automation" | "mcp" | "data_source" => {
            dispatch_domain_command(store, suite, cmd_type, args)
        }
        "backup" => {
            let dest = args
                .get("destination")
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'destination'".into()))?;
            store
                .backup_to(Path::new(dest))
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"backup": dest}))
        }
        other => Err((
            "unsupported_command",
            format!("unsupported daemon command '{other}'"),
        )),
    }
}

fn read_permission_state(store: &Store, conv_id: Option<&str>) -> (String, i64) {
    let Some(id) = conv_id.filter(|s| !s.trim().is_empty()) else {
        return ("cautious".into(), 1);
    };
    let perm = store
        .get_kv(&format!("conv_perm:{id}"))
        .ok()
        .flatten()
        .and_then(|v| v.as_str().map(ToString::to_string))
        .and_then(|s| PermissionProfile::parse_strict(&s).ok())
        .map(|p| p.as_canonical_str().to_string())
        .unwrap_or_else(|| "cautious".into());
    let rev = store
        .get_kv(&crate::web_api::session_meta_key(id))
        .ok()
        .flatten()
        .and_then(|v| v.get("configurationRevision").and_then(|r| r.as_i64()))
        .unwrap_or(1)
        .max(1);
    (perm, rev)
}

fn parse_memory_item(payload: &Value) -> MemoryItem {
    let rec = payload.get("record").unwrap_or(payload);
    let ts = now_ms();
    let namespace = rec
        .get("namespace")
        .and_then(|v| v.as_str())
        .map(ToString::to_string)
        .unwrap_or_else(|| {
            let scope = rec.get("scope").and_then(|v| v.as_str()).unwrap_or("global");
            let kind = rec.get("kind").and_then(|v| v.as_str()).unwrap_or("default");
            if scope == "global" && kind == "default" {
                "default".to_string()
            } else {
                format!("{scope}:{kind}")
            }
        });
    MemoryItem {
        id: rec
            .get("id")
            .or_else(|| payload.get("id"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(ToString::to_string)
            .unwrap_or_else(new_id),
        namespace,
        title: rec
            .get("title")
            .or_else(|| rec.get("summary"))
            .and_then(|v| v.as_str())
            .unwrap_or("Memory")
            .to_string(),
        content: rec
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        tags: rec
            .get("tags")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(ToString::to_string))
                    .collect()
            })
            .unwrap_or_default(),
        created_at_ms: ts,
        updated_at_ms: ts,
    }
}

fn parse_mcp_server_config(payload: &Value) -> Result<McpServerConfig, (&'static str, String)> {
    let cfg_val = payload
        .get("server")
        .or_else(|| payload.get("config"))
        .or_else(|| payload.get("record"))
        .unwrap_or(payload);
    if let Ok(parsed) = serde_json::from_value::<McpServerConfig>(cfg_val.clone()) {
        if parsed.id.trim().is_empty() {
            return Err(("invalid_argument", "MCP server 'id' must not be empty".into()));
        }
        return Ok(parsed);
    }
    let id = cfg_val
        .get("id")
        .or_else(|| cfg_val.get("server_id"))
        .or_else(|| payload.get("id"))
        .or_else(|| payload.get("server_id"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(new_id);
    let name = cfg_val
        .get("name")
        .or_else(|| cfg_val.get("display_name"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(&id)
        .to_string();
    let transport_raw = cfg_val
        .get("transport")
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| {
            if cfg_val.get("command").and_then(|v| v.as_str()).is_some() {
                "stdio"
            } else {
                "streamable_http"
            }
        });
    let transport = match transport_raw.trim().to_ascii_lowercase().as_str() {
        "stdio" => McpTransport::Stdio,
        "http" | "streamable_http" | "streamable-http" => McpTransport::StreamableHttp,
        "sse" | "server_sent_events" | "server-sent-events" => McpTransport::ServerSentEvents,
        other => {
            return Err((
                "invalid_argument",
                format!("unsupported MCP transport '{other}'"),
            ));
        }
    };
    let enabled = cfg_val
        .get("enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let command = cfg_val
        .get("command")
        .and_then(|v| v.as_str())
        .map(ToString::to_string);
    let args = cfg_val
        .get("args")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(ToString::to_string))
                .collect()
        })
        .unwrap_or_default();
    let env = cfg_val
        .get("env")
        .and_then(|v| v.as_object())
        .map(|obj| {
            obj.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default();
    let url = cfg_val
        .get("url")
        .or_else(|| cfg_val.get("endpoint"))
        .and_then(|v| v.as_str())
        .map(ToString::to_string);
    let headers = cfg_val
        .get("headers")
        .and_then(|v| v.as_object())
        .map(|obj| {
            obj.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default();
    let oauth_client_id = cfg_val
        .get("oauth_client_id")
        .and_then(|v| v.as_str())
        .map(ToString::to_string);
    let oauth_scopes = cfg_val
        .get("oauth_scopes")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(ToString::to_string))
                .collect()
        })
        .unwrap_or_default();
    Ok(McpServerConfig {
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
    })
}

fn dispatch_domain_command(
    store: &Store,
    suite: &ModulesSuite,
    domain: &str,
    args: &Value,
) -> Result<Value, (&'static str, String)> {
    let (op, payload) = extract_sub_command(args);

    match (domain, op) {
        // --- Model Domain ---
        ("model", "list" | "list_profiles") => {
            let mut items: Vec<Value> = suite
                .list_records("model")
                .map_err(|e| ("storage_error", e.to_string()))?;
            if items.is_empty() {
                if let Some(arr) = crate::web_api::build_model_profiles_response(store)
                    .get("profiles")
                    .and_then(|v| v.as_array())
                {
                    items = arr.clone();
                }
            }
            Ok(json!({"kind": "model", "items": items, "profiles": items}))
        }
        ("model", "get") => {
            let id = payload
                .get("id")
                .or_else(|| payload.get("profile_id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            let found = suite
                .get_record::<Value>("model", id)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(found.unwrap_or(Value::Null))
        }
        ("model", "upsert_profile") => {
            let record = payload
                .get("profile")
                .or_else(|| payload.get("record"))
                .unwrap_or(payload);
            let id = record
                .get("id")
                .or_else(|| payload.get("id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(new_id);
            let mut stored = record.clone();
            if let Some(obj) = stored.as_object_mut() {
                obj.insert("id".into(), json!(id));
            }
            suite
                .upsert_record("model", &id, &stored)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"ok": true, "id": id, "profile": stored}))
        }
        ("model", "delete_profile" | "delete") => {
            let id = payload
                .get("id")
                .or_else(|| payload.get("profile_id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            let deleted = suite
                .delete_record("model", id)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"ok": true, "deleted": deleted, "id": id}))
        }
        ("model", "configure_parameters") => {
            let id = payload
                .get("id")
                .or_else(|| payload.get("profile_id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            let mut rec = suite
                .get_record::<Value>("model", id)
                .map_err(|e| ("storage_error", e.to_string()))?
                .unwrap_or_else(|| json!({"id": id}));
            let params = payload.get("parameters").cloned().unwrap_or_else(|| {
                let mut m = serde_json::Map::new();
                for k in [
                    "temperature",
                    "top_p",
                    "topP",
                    "max_output_tokens",
                    "maxOutputTokens",
                    "context_window",
                    "maxContextLength",
                    "reasoning_effort",
                ] {
                    if let Some(v) = payload.get(k) {
                        m.insert(k.to_string(), v.clone());
                    }
                }
                Value::Object(m)
            });
            if let Some(obj) = rec.as_object_mut() {
                obj.insert("parameters".into(), params);
                obj.insert("updated_at_ms".into(), json!(now_ms()));
            }
            suite
                .upsert_record("model", id, &rec)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"ok": true, "id": id, "profile": rec}))
        }
        ("model", "reset_parameters") => {
            let id = payload
                .get("id")
                .or_else(|| payload.get("profile_id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            let mut rec = suite
                .get_record::<Value>("model", id)
                .map_err(|e| ("storage_error", e.to_string()))?
                .unwrap_or_else(|| json!({"id": id}));
            if let Some(obj) = rec.as_object_mut() {
                obj.insert("parameters".into(), json!({}));
                obj.insert("updated_at_ms".into(), json!(now_ms()));
            }
            suite
                .upsert_record("model", id, &rec)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"ok": true, "id": id, "profile": rec}))
        }
        ("model", "replace_profiles") => {
            let profiles = payload
                .get("profiles")
                .or_else(|| args.get("profiles"))
                .and_then(|v| v.as_array())
                .ok_or(("invalid_argument", "missing 'profiles' array".into()))?;
            let existing: Vec<Value> = suite.list_records("model").unwrap_or_default();
            for old in existing {
                if let Some(oid) = old.get("id").and_then(|v| v.as_str()) {
                    let _ = suite.delete_record("model", oid);
                }
            }
            let mut saved = Vec::new();
            for p in profiles {
                let id = p
                    .get("id")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
                    .map(ToString::to_string)
                    .unwrap_or_else(new_id);
                let mut rec = p.clone();
                if let Some(obj) = rec.as_object_mut() {
                    obj.insert("id".into(), json!(id));
                }
                suite
                    .upsert_record("model", &id, &rec)
                    .map_err(|e| ("storage_error", e.to_string()))?;
                saved.push(rec);
            }
            let _ = store.set_kv("web_model_profiles", &json!(saved));
            Ok(json!({"ok": true, "count": saved.len(), "profiles": saved}))
        }

        // --- Skill Domain ---
        ("skill", "list" | "list_editors") => {
            let mut items = nola_modules::skills::discover_skills_detailed(
                Some(store),
                &suite.workspace_root,
                suite.user_home.as_deref(),
            );
            for user_skill in suite.list_records::<Value>("skill").unwrap_or_default() {
                let sid = user_skill
                    .get("id")
                    .or_else(|| user_skill.get("name"))
                    .and_then(|v| v.as_str());
                let exists = sid.is_some_and(|id| {
                    items.iter().any(|x| {
                        x.get("id").and_then(|v| v.as_str()) == Some(id)
                            || x.get("name").and_then(|v| v.as_str()) == Some(id)
                    })
                });
                if !exists {
                    items.push(user_skill);
                }
            }
            Ok(json!({"kind": "skill", "items": items, "skills": items}))
        }
        ("skill", "get") => {
            let id = payload
                .get("id")
                .or_else(|| payload.get("name"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            if let Ok(Some(rec)) = suite.get_record::<Value>("skill", id) {
                return Ok(rec);
            }
            let detailed = nola_modules::skills::discover_skills_detailed(
                Some(store),
                &suite.workspace_root,
                suite.user_home.as_deref(),
            );
            let found = detailed.into_iter().find(|s| {
                s.get("name")
                    .and_then(|v| v.as_str())
                    .is_some_and(|n| n.eq_ignore_ascii_case(id))
            });
            Ok(found.unwrap_or(Value::Null))
        }
        ("skill", "upsert" | "create_user" | "configure_user") => {
            let record = payload
                .get("skill")
                .or_else(|| payload.get("record"))
                .unwrap_or(payload);
            let id = record
                .get("id")
                .or_else(|| payload.get("id"))
                .or_else(|| record.get("name"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(new_id);
            let mut stored = record.clone();
            if let Some(obj) = stored.as_object_mut() {
                obj.insert("id".into(), json!(id));
                obj.insert("updated_at_ms".into(), json!(now_ms()));
            }
            if let Some(enabled) = stored.get("enabled").and_then(|v| v.as_bool()) {
                let skill_name = stored
                    .get("name")
                    .or_else(|| stored.get("display_name"))
                    .and_then(|v| v.as_str())
                    .unwrap_or(&id);
                let _ = nola_modules::skills::set_skill_enabled(store, skill_name, enabled);
            }
            suite
                .upsert_record("skill", &id, &stored)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"ok": true, "id": id, "skill": stored}))
        }
        ("skill", "set_enabled_checked") => {
            let id = payload
                .get("id")
                .or_else(|| payload.get("name"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing skill 'id' or 'name'".into()))?;
            let enabled = payload
                .get("enabled")
                .or_else(|| args.get("enabled"))
                .and_then(|v| v.as_bool())
                .ok_or(("invalid_argument", "missing boolean 'enabled'".into()))?;
            nola_modules::skills::set_skill_enabled(store, id, enabled)
                .map_err(|e| ("storage_error", e.to_string()))?;
            if let Ok(Some(mut rec)) = suite.get_record::<Value>("skill", id) {
                if let Some(obj) = rec.as_object_mut() {
                    obj.insert("enabled".into(), json!(enabled));
                    obj.insert("updated_at_ms".into(), json!(now_ms()));
                }
                let _ = suite.upsert_record("skill", id, &rec);
            }
            Ok(json!({"ok": true, "id": id, "enabled": enabled}))
        }
        ("skill", "delete_user_checked" | "delete") => {
            let id = payload
                .get("id")
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            let deleted = suite
                .delete_record("skill", id)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"ok": true, "deleted": deleted, "id": id}))
        }

        // --- Memory Domain ---
        ("memory", "list" | "recall" | "search" | "query") => {
            let q = payload
                .get("query")
                .or_else(|| args.get("query"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let ns = payload
                .get("namespace")
                .or_else(|| args.get("namespace"))
                .and_then(|v| v.as_str());
            let limit = payload
                .get("limit")
                .or_else(|| args.get("limit"))
                .and_then(|v| v.as_u64())
                .unwrap_or(100) as usize;
            let items = store
                .search_memory(q, ns, limit)
                .map_err(|e| ("storage_error", e.to_string()))?;
            let records: Vec<Value> = items
                .iter()
                .map(crate::web_api::memory_item_to_record)
                .collect();
            Ok(json!({"items": items, "records": records}))
        }
        ("memory", "get") => {
            let id = payload
                .get("id")
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            let items = store
                .search_memory("", None, 500)
                .map_err(|e| ("storage_error", e.to_string()))?;
            let found = items
                .into_iter()
                .find(|m| m.id == id)
                .map(|m| crate::web_api::memory_item_to_record(&m));
            Ok(found.unwrap_or(Value::Null))
        }
        ("memory", "commit" | "save" | "upsert" | "update_if_revision") => {
            let item = parse_memory_item(payload);
            store
                .upsert_memory(&item)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(crate::web_api::memory_item_to_record(&item))
        }
        ("memory", "replace") => {
            if let Some(arr) = payload
                .get("records")
                .or_else(|| payload.get("items"))
                .and_then(|v| v.as_array())
            {
                let mut records = Vec::new();
                for entry in arr {
                    let item = parse_memory_item(entry);
                    store
                        .upsert_memory(&item)
                        .map_err(|e| ("storage_error", e.to_string()))?;
                    records.push(crate::web_api::memory_item_to_record(&item));
                }
                return Ok(json!({"ok": true, "records": records}));
            }
            let item = parse_memory_item(payload);
            store
                .upsert_memory(&item)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(crate::web_api::memory_item_to_record(&item))
        }
        ("memory", "delete") => {
            let id = payload
                .get("id")
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            store
                .delete_memory(id)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"deleted": id, "ok": true}))
        }

        // --- MCP Domain ---
        ("mcp", "list") => {
            let servers = store
                .list_mcp_servers()
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"kind": "mcp", "items": servers, "servers": servers}))
        }
        ("mcp", "get") => {
            let id = payload
                .get("id")
                .or_else(|| payload.get("server_id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            let found = store
                .list_mcp_servers()
                .map_err(|e| ("storage_error", e.to_string()))?
                .into_iter()
                .find(|s| s.id == id);
            Ok(found
                .and_then(|s| serde_json::to_value(s).ok())
                .unwrap_or(Value::Null))
        }
        ("mcp", "create" | "configure" | "upsert") => {
            let cfg = parse_mcp_server_config(payload)?;
            store
                .upsert_mcp_server(&cfg)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"ok": true, "id": cfg.id, "server": cfg}))
        }
        ("mcp", "delete_checked" | "delete") => {
            let id = payload
                .get("id")
                .or_else(|| payload.get("server_id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            let deleted = store
                .delete_mcp_server(id)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"ok": true, "deleted": deleted, "id": id}))
        }

        // --- DataSource Domain ---
        ("data_source", "list") => {
            let items: Vec<Value> = suite
                .list_records("data_source")
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"kind": "data_source", "items": items}))
        }
        ("data_source", "list_usage") => {
            let usage = nola_modules::list_data_source_usage(store);
            Ok(json!({"kind": "data_source_usage", "items": usage, "usage": usage}))
        }
        ("data_source", "get") => {
            let id = payload
                .get("id")
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            let found = suite
                .get_record::<Value>("data_source", id)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(found.unwrap_or(Value::Null))
        }
        ("data_source", "create_draft" | "create") => {
            let record = payload.get("record").unwrap_or(payload);
            let id = record
                .get("id")
                .or_else(|| payload.get("id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(new_id);
            let mut stored = record.clone();
            if let Some(obj) = stored.as_object_mut() {
                obj.insert("id".into(), json!(id));
                obj.insert("updated_at_ms".into(), json!(now_ms()));
            }
            suite
                .upsert_record("data_source", &id, &stored)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"ok": true, "id": id, "record": stored}))
        }
        ("data_source", "update_configuration" | "update") => {
            let record = payload
                .get("configuration")
                .or_else(|| payload.get("record"))
                .unwrap_or(payload);
            let id = payload
                .get("id")
                .or_else(|| record.get("id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            let mut merged = suite
                .get_record::<Value>("data_source", id)
                .map_err(|e| ("storage_error", e.to_string()))?
                .unwrap_or_else(|| json!({"id": id}));
            if let (Some(dst), Some(src)) = (merged.as_object_mut(), record.as_object()) {
                for (k, v) in src {
                    if k != "type" {
                        dst.insert(k.clone(), v.clone());
                    }
                }
                dst.insert("id".into(), json!(id));
                dst.insert("updated_at_ms".into(), json!(now_ms()));
            }
            suite
                .upsert_record("data_source", id, &merged)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"ok": true, "id": id, "record": merged}))
        }
        ("data_source", "set_conversation_override") => {
            let conv_id = payload
                .get("conversation_id")
                .or_else(|| args.get("conversation_id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'conversation_id'".into()))?;
            let ds_id = payload
                .get("data_source_id")
                .or_else(|| payload.get("id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'data_source_id'".into()))?;
            let override_val = payload
                .get("override")
                .or_else(|| payload.get("enabled"))
                .cloned()
                .unwrap_or(Value::Null);
            let key = format!("ds_override:{conv_id}:{ds_id}");
            store
                .set_kv(&key, &override_val)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({
                "ok": true,
                "conversation_id": conv_id,
                "data_source_id": ds_id,
                "override": override_val
            }))
        }
        ("data_source", "delete") => {
            let id = payload
                .get("id")
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            let deleted = suite
                .delete_record("data_source", id)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"ok": true, "deleted": deleted, "id": id}))
        }

        // --- Automation Domain ---
        ("automation", "list") => {
            let items: Vec<Value> = suite
                .list_records("automation")
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"kind": "automation", "items": items}))
        }
        ("automation", "get") => {
            let id = payload
                .get("id")
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            let found = suite
                .get_record::<Value>("automation", id)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(found.unwrap_or(Value::Null))
        }
        ("automation", "create") => {
            let record = payload.get("record").unwrap_or(payload);
            let norm = crate::web_api::normalize_automation_record(record);
            let id = norm
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            suite
                .upsert_record("automation", &id, &norm)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"ok": true, "id": id, "record": norm}))
        }
        ("automation", "update") => {
            let record = payload.get("record").unwrap_or(payload);
            let id = payload
                .get("id")
                .or_else(|| record.get("id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            let mut merged = suite
                .get_record::<Value>("automation", id)
                .map_err(|e| ("storage_error", e.to_string()))?
                .unwrap_or_else(|| json!({"id": id}));
            if let (Some(dst), Some(src)) = (merged.as_object_mut(), record.as_object()) {
                for (k, v) in src {
                    if k != "type" {
                        dst.insert(k.clone(), v.clone());
                    }
                }
                dst.insert("id".into(), json!(id));
            }
            let norm = crate::web_api::normalize_automation_record(&merged);
            suite
                .upsert_record("automation", id, &norm)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"ok": true, "id": id, "record": norm}))
        }
        ("automation", "pause" | "resume") => {
            let id = payload
                .get("id")
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            let mut rec = suite
                .get_record::<Value>("automation", id)
                .map_err(|e| ("storage_error", e.to_string()))?
                .ok_or(("not_found", format!("automation '{id}' not found")))?;
            let enabled = op == "resume";
            if let Some(obj) = rec.as_object_mut() {
                obj.insert("enabled".into(), json!(enabled));
                obj.insert("updatedAt".into(), json!(Utc::now().to_rfc3339()));
                obj.insert("updated_at_ms".into(), json!(now_ms()));
            }
            suite
                .upsert_record("automation", id, &rec)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"ok": true, "id": id, "enabled": enabled, "record": rec}))
        }
        ("automation", "delete") => {
            let id = payload
                .get("id")
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            let deleted = suite
                .delete_record("automation", id)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"ok": true, "deleted": deleted, "id": id}))
        }

        _ => Err((
            "unsupported_command",
            format!("unsupported {domain} operation '{op}'"),
        )),
    }
}

fn dispatch_agent_room_command(
    suite: &ModulesSuite,
    args: &Value,
) -> Result<Value, (&'static str, String)> {
    let (op, payload) = extract_sub_command(args);
    match op {
        "list" | "list_rooms" => {
            let rooms: Vec<AgentRoom> = suite
                .list_records("agent_room")
                .map_err(|e| ("storage_error", e.to_string()))?;
            let experts: Vec<ExpertPeer> = suite
                .list_records("expert_peer")
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({
                "rooms": rooms,
                "experts": experts,
                "role_templates": built_in_role_templates()
            }))
        }
        "list_role_templates" | "role_templates" => Ok(json!({
            "role_templates": built_in_role_templates()
        })),
        "list_experts" => {
            let experts: Vec<ExpertPeer> = suite
                .list_records("expert_peer")
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({
                "experts": experts,
                "role_templates": built_in_role_templates()
            }))
        }
        "get" | "get_room" => {
            let id = payload
                .get("id")
                .or_else(|| payload.get("room_id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            let room = suite
                .get_record::<AgentRoom>("agent_room", id)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(serde_json::to_value(room).unwrap_or(Value::Null))
        }
        "create_room" | "upsert_room" => {
            let ts = now_ms();
            let id = payload
                .get("id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(new_id);
            let room = AgentRoom {
                id: id.clone(),
                name: payload
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Agent Room")
                    .to_string(),
                workspace_root: payload
                    .get("workspace_root")
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string)
                    .unwrap_or_else(|| suite.workspace_root.display().to_string()),
                participant_expert_ids: payload
                    .get("participant_expert_ids")
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(ToString::to_string))
                            .collect()
                    })
                    .unwrap_or_default(),
                session_id: payload
                    .get("session_id")
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string)
                    .unwrap_or_else(|| format!("room_{id}")),
                created_at_ms: ts,
                updated_at_ms: ts,
            };
            suite
                .upsert_record("agent_room", &room.id, &room)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"ok": true, "room": room}))
        }
        "delete_room" => {
            let id = payload
                .get("id")
                .or_else(|| payload.get("room_id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            let deleted = suite
                .delete_record("agent_room", id)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"ok": true, "deleted": deleted, "id": id}))
        }
        "create_expert" | "upsert_expert" => {
            let id = payload
                .get("id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(new_id);
            let expert = ExpertPeer {
                id: id.clone(),
                name: payload
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Expert")
                    .to_string(),
                role: payload
                    .get("role")
                    .and_then(|v| v.as_str())
                    .unwrap_or("specialist")
                    .to_string(),
                system_prompt: payload
                    .get("system_prompt")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                model_override: payload
                    .get("model_override")
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string),
                allowed_tools: payload
                    .get("allowed_tools")
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(ToString::to_string))
                            .collect()
                    })
                    .unwrap_or_default(),
                updated_at_ms: now_ms(),
            };
            suite
                .upsert_record("expert_peer", &expert.id, &expert)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"ok": true, "expert": expert}))
        }
        "delete_expert" => {
            let id = payload
                .get("id")
                .or_else(|| payload.get("expert_id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            let deleted = suite
                .delete_record("expert_peer", id)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"ok": true, "deleted": deleted, "id": id}))
        }
        other => Err((
            "unsupported_command",
            format!("unsupported agent_room operation '{other}'"),
        )),
    }
}

fn sanitize_template_record(raw: &Value) -> Value {
    let mut out = serde_json::Map::new();
    for key in [
        "id",
        "name",
        "role",
        "description",
        "system_prompt",
        "suggested_tools",
        "revision",
        "builtin",
        "updated_at_ms",
    ] {
        if let Some(v) = raw.get(key) {
            out.insert(key.to_string(), v.clone());
        }
    }
    if !out.contains_key("revision") {
        out.insert("revision".into(), json!(1));
    }
    Value::Object(out)
}

fn dispatch_expert_template_command(
    suite: &ModulesSuite,
    args: &Value,
) -> Result<Value, (&'static str, String)> {
    let (op, payload) = extract_sub_command(args);
    let mut builtins: Vec<Value> = built_in_role_templates()
        .into_iter()
        .map(|mut t| {
            if let Some(obj) = t.as_object_mut() {
                obj.insert("revision".into(), json!(1));
                obj.insert("builtin".into(), json!(true));
            }
            sanitize_template_record(&t)
        })
        .collect();

    match op {
        "list" | "list_templates" => {
            let custom: Vec<Value> = suite
                .list_records::<Value>("expert_template")
                .unwrap_or_default()
                .into_iter()
                .map(|v| sanitize_template_record(&v))
                .collect();
            builtins.extend(custom);
            Ok(json!({"templates": builtins, "items": builtins}))
        }
        "get" => {
            let id = payload
                .get("id")
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            if let Ok(Some(custom)) = suite.get_record::<Value>("expert_template", id) {
                return Ok(sanitize_template_record(&custom));
            }
            let found = builtins
                .into_iter()
                .find(|t| t.get("id").and_then(|v| v.as_str()) == Some(id));
            Ok(found.unwrap_or(Value::Null))
        }
        "upsert" | "create" | "update" => {
            let src = payload.get("template").unwrap_or(payload);
            let id = src
                .get("id")
                .or_else(|| payload.get("id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(new_id);
            let prev_rev = suite
                .get_record::<Value>("expert_template", &id)
                .ok()
                .flatten()
                .and_then(|v| v.get("revision").and_then(|r| r.as_u64()))
                .unwrap_or(0);
            let mut rec = sanitize_template_record(src);
            if let Some(obj) = rec.as_object_mut() {
                obj.insert("id".into(), json!(id));
                if !obj.contains_key("name") {
                    obj.insert("name".into(), json!(id));
                }
                obj.insert("revision".into(), json!(prev_rev + 1));
                obj.insert("builtin".into(), json!(false));
                obj.insert("updated_at_ms".into(), json!(now_ms()));
            }
            suite
                .upsert_record("expert_template", &id, &rec)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"ok": true, "id": id, "template": rec}))
        }
        "delete" => {
            let id = payload
                .get("id")
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            let deleted = suite
                .delete_record("expert_template", id)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"ok": true, "deleted": deleted, "id": id}))
        }
        other => Err((
            "unsupported_command",
            format!("unsupported expert template operation '{other}'"),
        )),
    }
}
