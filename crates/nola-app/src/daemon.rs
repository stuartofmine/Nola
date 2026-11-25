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

use std::collections::{BTreeMap, HashSet};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
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
use nola_store::{
    default_socket_path, EventPage, ServiceLock, Store, SyncChangeRecord, PROTOCOL_VERSION,
};
use nola_types::{
    new_id, now_ms, sha256_hex, AgentRoom, ExpertPeer, GoalRecord, GoalStatus, McpServerConfig,
    McpTransport, MemoryItem, PermissionProfile, Session,
};
use serde_json::{json, Value};

#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};

const MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;
const MAX_CONCURRENT_DAEMON_CLIENTS: usize = 32;

static WRITE_MUTEX: Mutex<()> = Mutex::new(());
static ACTIVE_RUNS: OnceLock<Mutex<BTreeMap<String, Value>>> = OnceLock::new();

fn active_runs_map() -> &'static Mutex<BTreeMap<String, Value>> {
    ACTIVE_RUNS.get_or_init(|| Mutex::new(BTreeMap::new()))
}

/// Register an active run for a conversation in daemon in-memory state and durable store KV.
pub fn register_active_run(store: &Store, conv_id: &str, run_info: Value) {
    if conv_id.trim().is_empty() {
        return;
    }
    if let Ok(mut map) = active_runs_map().lock() {
        map.insert(conv_id.to_string(), run_info.clone());
    }
    let _ = store.set_kv(&format!("current_run:{conv_id}"), &run_info);
}

/// Clear an active run for a conversation from daemon in-memory state and durable store KV.
pub fn clear_active_run(store: &Store, conv_id: &str) {
    if conv_id.trim().is_empty() {
        return;
    }
    if let Ok(mut map) = active_runs_map().lock() {
        map.remove(conv_id);
    }
    let _ = store.set_kv(&format!("current_run:{conv_id}"), &Value::Null);
}

fn lookup_active_run(store: &Store, conv_id: &str) -> Option<Value> {
    if conv_id.trim().is_empty() {
        return None;
    }
    if let Ok(map) = active_runs_map().lock() {
        if let Some(val) = map.get(conv_id) {
            if !val.is_null() {
                return Some(val.clone());
            }
        }
    }
    if let Ok(Some(val)) = store.get_kv(&format!("current_run:{conv_id}")) {
        if !val.is_null() {
            return Some(val);
        }
    }
    // Fall back to live running `durable_task` module records for this conversation.
    if let Ok(tasks) = store.list_module_records::<Value>("durable_task") {
        if let Some(t) = tasks.into_iter().find(|t| {
            t.get("session_id").and_then(|v| v.as_str()) == Some(conv_id)
                && t.get("status").and_then(|v| v.as_str()) == Some("running")
        }) {
            return Some(json!({
                "conversation_id": conv_id,
                "run_id": t.get("task_id").cloned().unwrap_or(Value::Null),
                "assistant_message_id": t
                    .get("task_id")
                    .and_then(|v| v.as_str())
                    .map(|tid| format!("{tid}:assistant")),
                "status": "running",
                "task": t
            }));
        }
    }
    None
}

fn load_durable_permission_grants(store: &Store, conv_id: Option<&str>) -> Vec<Value> {
    let mut grants: Vec<Value> = Vec::new();
    let mut seen_ids: HashSet<String> = HashSet::new();

    if let Ok(records) = store.list_module_records::<Value>("permission_grant") {
        for rec in records {
            let active = rec.get("active").and_then(|v| v.as_bool()).unwrap_or(true);
            if !active {
                continue;
            }
            let rec_session = rec
                .get("session_id")
                .or_else(|| rec.get("conversation_id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty() && *s != "global");
            let matches_conv = match (conv_id, rec_session) {
                (Some(cid), Some(rs)) => rs == cid,
                _ => true,
            };
            if !matches_conv {
                continue;
            }
            let gid = rec
                .get("id")
                .or_else(|| rec.get("grant_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if gid.is_empty() || seen_ids.insert(gid) {
                grants.push(rec);
            }
        }
    }

    if let Some(cid) = conv_id.filter(|s| !s.trim().is_empty()) {
        if let Ok(Some(Value::Array(conv_grants))) = store.get_kv(&format!("conv_grants:{cid}")) {
            for g in conv_grants {
                let gid = g
                    .get("id")
                    .or_else(|| g.get("grant_id"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if gid.is_empty() || seen_ids.insert(gid) {
                    grants.push(g);
                }
            }
        }
    }
    grants
}

fn load_pending_permission_requests(store: &Store, conv_id: Option<&str>) -> Vec<Value> {
    let Some(cid) = conv_id.filter(|s| !s.trim().is_empty()) else {
        return Vec::new();
    };
    let mut pending = store
        .get_kv(&format!("pending_permissions:{cid}"))
        .ok()
        .flatten()
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();
    if let Ok(Some(Value::Array(waits))) = store.get_kv(&format!("open_waits:{cid}")) {
        for w in waits {
            let kind = w
                .get("kind")
                .or_else(|| w.get("type"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if matches!(kind, "permission" | "approval" | "request_permission") {
                pending.push(w);
            }
        }
    }
    pending
}

fn synchronize_session_state_after_truncate(store: &Store, conv_id: &str) {
    let state_key = format!("session_state:{conv_id}");
    let Ok(Some(mut state_val)) = store.get_kv(&state_key) else {
        return;
    };
    let (messages, _) = crate::web_api::reconstruct_v2_messages(store, conv_id);
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

#[cfg(unix)]
struct ActiveClientGuard(Arc<AtomicUsize>);

#[cfg(unix)]
impl Drop for ActiveClientGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

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
        crate::config::bind_store_vault_path(&store, &crate::config::default_vault_path(db_path))?;
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
                "--archive-incompatible-store" => {}
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
                    let client_guard = ActiveClientGuard(Arc::clone(&active_clients));
                    let store_c = store.clone();
                    let suite_c = Arc::clone(&suite);
                    let handle = std::thread::spawn(move || {
                        let _guard = client_guard;
                        handle_client(stream, &store_c, &suite_c);
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
            .or_else(|| sub.get("action"))
            .or_else(|| sub.get("operation"))
            .or(Some(sub))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let payload = if sub.is_object() { sub } else { args };
        return (op, payload);
    }
    if let Some(action) = args.get("action").and_then(|v| v.as_str()) {
        return (action, args);
    }
    if let Some(ty) = args.get("type").and_then(|v| v.as_str()) {
        if !matches!(
            ty,
            "model"
                | "skill"
                | "memory"
                | "automation"
                | "mcp"
                | "data_source"
                | "agent_room"
                | "context"
                | "conversation_plan"
                | "runtime_task"
                | "host_expert_template_operation"
                | "native_expert_host_operation"
        ) {
            return (ty, args);
        }
    }
    ("list", args)
}

fn is_read_only_sub_op(op: &str) -> bool {
    matches!(
        op,
        "list"
            | "list_profiles"
            | "list_connections"
            | "list_accounts"
            | "list_editors"
            | "list_usage"
            | "list_rooms"
            | "list_experts"
            | "list_role_templates"
            | "role_templates"
            | "list_templates"
            | "get"
            | "get_profile"
            | "get_connection"
            | "get_account"
            | "get_runtime"
            | "get_editor"
            | "get_conversation_override"
            | "get_room"
            | "read"
            | "load"
            | "directory"
            | "for_conversation"
            | "child_conversations"
            | "child_history"
            | "peer_exchanges"
            | "submission"
            | "submissions"
            | "execution_intent"
            | "resolve_sources"
            | "resolve_for_conversation"
            | "prepare_compaction"
            | "prepare_maintenance"
            | "count_checkpoints"
            | "load_checkpoint"
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
        | "patch_conversation_metadata"
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
        | "put_product_setting_checked"
        | "delete_product_setting"
        | "set_conversation_permission"
        | "select_conversation_execution"
        | "step_goal"
        | "respond_to_conversation_wait"
        | "request_run_cancellation"
        | "resolve_unknown_effect_recovery"
        | "resolve_owned_task_recovery"
        | "prepare_in_place_resend"
        | "prepare_edited_resend"
        | "reconcile_automation_execution"
        | "ack_automation_notification"
        | "prepare_remote_submission"
        | "bind_remote_submission_request"
        | "accept_remote_submission_receipt"
        | "abandon_prepared_remote_submission"
        | "adopt_observed_remote_run"
        | "accept_remote_run_context_usage"
        | "resolve_sync_change"
        | "save_sync_cursor"
        | "apply_remote_sync_page"
        | "upsert_permission_grant"
        | "revoke_permission_grant"
        | "import_conversation_transcript"
        | "admit_host_output"
        | "submit_task_guidance"
        | "request_owned_cancellation"
        | "begin_archive_restore"
        | "commit_archive_restore"
        | "stage_archive_manifest"
        | "begin_content_upload"
        | "append_content_chunk"
        | "seal_content_upload"
        | "stage_assistant_journal_content"
        | "put_conversation_pdf_resource"
        | "delete_conversation_resource"
        | "admit_conversation_user_message"
        | "begin_queued_user_turn"
        | "backup" => true,
        "automation" => {
            let (op, payload) = extract_sub_command(cmd_args);
            if matches!(op, "definition" | "run") {
                let (sub_op, _) = extract_sub_command(payload);
                !is_read_only_sub_op(sub_op)
            } else {
                !is_read_only_sub_op(op)
            }
        }
        "model"
        | "skill"
        | "memory"
        | "mcp"
        | "data_source"
        | "agent_room"
        | "context"
        | "runtime_task"
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
    let _write_guard = if is_mutation {
        Some(WRITE_MUTEX.lock().unwrap_or_else(|e| e.into_inner()))
    } else {
        None
    };

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

    let dispatch_result = dispatch_command(store, suite, cmd_type, &cmd_args);
    let completed_at = Utc::now().to_rfc3339();
    let is_transient_storage_error = matches!(&dispatch_result, Err(("storage_error", _)));
    let resp = match dispatch_result {
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
                "recoverable": code == "storage_error"
            }
        }),
    };

    if let Some(digest) = payload_digest {
        if !is_transient_storage_error {
            let wrapped = json!({
                "payload_digest": digest,
                "receipt": resp
            });
            let _ = store.set_kv(&format!("cmd_receipt:{request_id}"), &wrapped);
        }
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
            let Some(sess) = store
                .get_session(id)
                .map_err(|e| ("storage_error", e.to_string()))?
            else {
                return Ok(Value::Null);
            };
            let before_ordinal = args.get("before_ordinal").and_then(|v| v.as_i64());
            let limit = args
                .get("limit")
                .and_then(|v| v.as_u64())
                .map(|n| n.clamp(1, 500) as usize);
            Ok(build_conversation_projection_response(
                store,
                &sess,
                0,
                limit,
                before_ordinal,
            ))
        }
        "load_conversation_projection_page_v1" => {
            let id = args
                .get("conversation_id")
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing conversation_id".into()))?;
            let Some(sess) = store
                .get_session(id)
                .map_err(|e| ("storage_error", e.to_string()))?
            else {
                return Ok(Value::Null);
            };
            let offset = args.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            let limit = args
                .get("limit")
                .and_then(|v| v.as_u64())
                .map(|n| n.clamp(1, 500) as usize);
            Ok(build_conversation_projection_response(
                store, &sess, offset, limit, None,
            ))
        }
        "load_conversation_projection_window_update_v1" => {
            let id = args
                .get("conversation_id")
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing conversation_id".into()))?;
            let Some(sess) = store
                .get_session(id)
                .map_err(|e| ("storage_error", e.to_string()))?
            else {
                return Ok(Value::Null);
            };
            let after_seq = args
                .get("after_sequence")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let before_ordinal = args.get("before_ordinal").and_then(|v| v.as_i64());
            let limit = args
                .get("limit")
                .and_then(|v| v.as_u64())
                .map(|n| n.clamp(1, 500) as usize);
            let resp =
                build_conversation_projection_response(store, &sess, 0, limit, before_ordinal);
            let event_seq = resp
                .get("event_sequence")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            if event_seq <= after_seq {
                Ok(Value::Null)
            } else {
                Ok(resp)
            }
        }
        "load_conversation_settings" => {
            let id = args
                .get("conversation_id")
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing conversation_id".into()))?;
            Ok(load_conversation_settings_str(store, id)
                .map(Value::String)
                .unwrap_or(Value::Null))
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
            let goals: Vec<GoalRecord> = suite.list_records("goal").unwrap_or_default();
            let active_goal = goals
                .iter()
                .find(|g| matches!(g.status, GoalStatus::Running | GoalStatus::Pending))
                .cloned();
            let goal_projection = json!({
                "conversation_id": id,
                "active_goal": active_goal,
                "goals": goals,
                "plan": plan
            });
            Ok(json!({
                "conversation_id": id,
                "todos": todos,
                "plan": plan,
                "usage": usage,
                "goal_projection": goal_projection
            }))
        }
        "load_conversation_context_usage_v1" => {
            let id = args
                .get("conversation_id")
                .or_else(|| args.get("session_id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing conversation_id".into()))?;
            let rev = store
                .get_kv(&crate::web_api::session_meta_key(id))
                .ok()
                .flatten()
                .and_then(|v| v.get("revision").and_then(|r| r.as_i64()))
                .unwrap_or(1)
                .max(1);
            Ok(build_context_usage_json(store, id, rev))
        }
        "read_permission_configuration" => {
            let id = args
                .get("conversation_id")
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str());
            let (profile, rev) = read_permission_state(store, id);
            let grants = load_durable_permission_grants(store, id);
            let pending = load_pending_permission_requests(store, id);
            let is_active = id.and_then(|cid| lookup_active_run(store, cid)).is_some();
            Ok(json!({
                "conversation_id": id,
                "permission_profile": profile,
                "permission_mode": profile,
                "revision": rev,
                "configuration_revision": rev,
                "owner": {"kind": "local"},
                "state": if is_active { "active" } else { "inactive" },
                "grants": grants,
                "pending_requests": pending
            }))
        }
        "load_conversation_permission_projection_v1" => {
            let id = args
                .get("conversation_id")
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing conversation_id".into()))?;
            let (profile, rev) = read_permission_state(store, Some(id));
            let grants = load_durable_permission_grants(store, Some(id));
            let pending = load_pending_permission_requests(store, Some(id));
            Ok(json!({
                "conversation_id": id,
                "permission_profile": profile,
                "revision": rev,
                "pending_requests": pending,
                "pending": pending,
                "grants": grants
            }))
        }
        "active_run" => {
            let conv_id = args
                .get("conversation_id")
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str());
            if let Some(cid) = conv_id.filter(|s| !s.trim().is_empty()) {
                let found = lookup_active_run(store, cid);
                let run_id = found
                    .as_ref()
                    .and_then(|r| r.get("run_id").cloned())
                    .unwrap_or(Value::Null);
                let assistant_message_id = found
                    .as_ref()
                    .and_then(|r| r.get("assistant_message_id").cloned())
                    .unwrap_or(Value::Null);
                Ok(json!({
                    "active": found.is_some(),
                    "run": found.unwrap_or(Value::Null),
                    "run_id": run_id,
                    "assistant_message_id": assistant_message_id,
                    "conversation_id": cid
                }))
            } else {
                let runs: Vec<Value> = active_runs_map()
                    .lock()
                    .map(|m| m.values().cloned().collect())
                    .unwrap_or_default();
                Ok(json!({
                    "active": !runs.is_empty(),
                    "run": runs.first().cloned().unwrap_or(Value::Null),
                    "runs": runs,
                    "conversation_id": Value::Null
                }))
            }
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
            let input = args.get("input").unwrap_or(args);
            let limit = input
                .get("limit")
                .or_else(|| args.get("limit"))
                .and_then(|v| v.as_u64())
                .unwrap_or(50)
                .clamp(1, 200) as usize;
            let source_filter = input
                .get("source")
                .or_else(|| args.get("source"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let needs_response_filter = input
                .get("needs_response")
                .or_else(|| args.get("needs_response"))
                .and_then(|v| v.as_bool());
            let query_filter = input
                .get("query")
                .or_else(|| args.get("query"))
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_ascii_lowercase())
                .filter(|s| !s.is_empty());
            let sessions = store
                .list_sessions(200)
                .map_err(|e| ("storage_error", e.to_string()))?;
            let mut entries = Vec::new();
            for (idx, s) in sessions.into_iter().enumerate() {
                let (msgs, evs) = crate::web_api::reconstruct_v2_messages(store, &s.id);
                let last_assistant = msgs
                    .iter()
                    .rev()
                    .find(|m| m.get("role").and_then(|v| v.as_str()) == Some("assistant"));
                let assistant_message_id = last_assistant
                    .and_then(|m| m.get("id").and_then(|v| v.as_str()))
                    .map(ToString::to_string)
                    .unwrap_or_default();
                let last_summary = msgs
                    .last()
                    .and_then(|m| m.get("content").and_then(|v| v.as_str()))
                    .map(|t| t.chars().take(160).collect::<String>())
                    .unwrap_or_default();
                let open_waits = store
                    .get_kv(&format!("open_waits:{}", s.id))
                    .ok()
                    .flatten()
                    .and_then(|v| v.as_array().map(|a| !a.is_empty()))
                    .unwrap_or(false);
                if let Some(src) = source_filter {
                    if !src.eq_ignore_ascii_case("conversation") {
                        continue;
                    }
                }
                if let Some(req_needs) = needs_response_filter {
                    if open_waits != req_needs {
                        continue;
                    }
                }
                if let Some(ref q) = query_filter {
                    let title_match = s.title.to_ascii_lowercase().contains(q);
                    let summary_match = last_summary.to_ascii_lowercase().contains(q);
                    if !title_match && !summary_match {
                        continue;
                    }
                }
                entries.push(json!({
                    "id": s.id,
                    "activity_id": s.id,
                    "run_id": format!("run-{}", s.id),
                    "conversation_id": s.id,
                    "assistant_message_id": assistant_message_id,
                    "source": "conversation",
                    "kind": "conversation",
                    "title": s.title,
                    "summary": last_summary,
                    "status": if open_waits { "waiting" } else { "completed" },
                    "needs_response": open_waits,
                    "is_current": idx == 0,
                    "message_count": msgs.len(),
                    "event_count": evs.len(),
                    "created_at": crate::web_api::ms_to_rfc3339(s.created_at_ms),
                    "updated_at": crate::web_api::ms_to_rfc3339(s.updated_at_ms),
                    "created_at_ms": s.created_at_ms,
                    "completed_at_ms": s.updated_at_ms,
                    "updated_at_ms": s.updated_at_ms
                }));
                if entries.len() >= limit {
                    break;
                }
            }
            Ok(json!({
                "entries": entries,
                "next_cursor": Value::Null,
                "items": entries,
                "activities": entries
            }))
        }
        "load_activity_detail" => {
            let id = args
                .get("activity_id")
                .or_else(|| args.get("run_id"))
                .or_else(|| args.get("conversation_id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing activity_id".into()))?;
            let lookup_id = id.strip_prefix("run-").unwrap_or(id);
            let sess = store
                .get_session(lookup_id)
                .map_err(|e| ("storage_error", e.to_string()))?
                .ok_or(("not_found", format!("activity '{id}' not found")))?;
            let (messages, events) = crate::web_api::reconstruct_v2_messages(store, &sess.id);
            let usage = load_session_usage(store, &sess.id);
            let plan = load_session_plan(store, &sess.id);
            let todos = load_session_todos(store, &sess.id);
            let last_assistant = messages
                .iter()
                .rev()
                .find(|m| m.get("role").and_then(|v| v.as_str()) == Some("assistant"));
            let assistant_message_id = last_assistant
                .and_then(|m| m.get("id").and_then(|v| v.as_str()))
                .map(ToString::to_string)
                .unwrap_or_default();
            let result_text = last_assistant
                .and_then(|m| m.get("content").and_then(|v| v.as_str()))
                .unwrap_or("")
                .to_string();
            let open_waits = store
                .get_kv(&format!("open_waits:{}", sess.id))
                .ok()
                .flatten()
                .and_then(|v| v.as_array().map(|a| !a.is_empty()))
                .unwrap_or(false);
            let host_id = store
                .get_kv("host_id")
                .ok()
                .flatten()
                .and_then(|v| v.as_str().map(ToString::to_string))
                .unwrap_or_else(|| "local-host".into());
            let entry = json!({
                "id": sess.id,
                "activity_id": sess.id,
                "run_id": format!("run-{}", sess.id),
                "conversation_id": sess.id,
                "assistant_message_id": assistant_message_id,
                "source": "conversation",
                "title": sess.title,
                "created_at_ms": sess.created_at_ms,
                "completed_at_ms": sess.updated_at_ms,
                "status": if open_waits { "waiting" } else { "completed" },
                "needs_response": open_waits,
                "is_current": true
            });
            Ok(json!({
                "id": sess.id,
                "entry": entry,
                "explanation": format!("Conversation '{}' ({} messages)", sess.title, messages.len()),
                "source_conversation_id": sess.id,
                "review_in_source": true,
                "result_text": result_text,
                "result_has_more": false,
                "execution_host_id": host_id,
                "context_scope": "conversation",
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
            let conversations: Vec<Value> = sessions
                .iter()
                .map(|s| crate::web_api::session_to_conversation_json(store, s))
                .collect();
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
            let mut attention_list: Vec<Value> = Vec::new();
            for s in &sessions {
                let has_wait = store
                    .get_kv(&format!("open_waits:{}", s.id))
                    .ok()
                    .flatten()
                    .and_then(|v| v.as_array().map(|a| !a.is_empty()))
                    .unwrap_or(false);
                if has_wait {
                    attention_list.push(json!({
                        "conversation_id": s.id,
                        "title": s.title,
                        "reason": "waiting_for_user_input"
                    }));
                }
            }
            for g in &goals {
                if g.status == GoalStatus::Failed {
                    attention_list.push(json!({
                        "goal_id": g.id,
                        "title": g.title,
                        "reason": "goal_failed"
                    }));
                }
            }
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
                "facts": facts,
                "conversations": conversations,
                "recent_conversations": conversations,
                "unrepresented_expert_contexts": [],
                "needs_attention": !attention_list.is_empty(),
                "attention_items": attention_list,
                "has_more_conversations": false,
                "next_conversation_offset": Value::Null,
                "queued_instances": [],
                "has_more_queued_instances": false
            }))
        }
        "agent_room" => dispatch_agent_room_command(store, suite, args),
        "host_expert_template_operation" | "native_expert_host_operation" => {
            dispatch_expert_template_command(suite, args)
        }
        "conversation_plan" => {
            let inner = args
                .get("command")
                .or_else(|| args.get("input"))
                .unwrap_or(args);
            let id = inner
                .get("conversation_id")
                .or_else(|| inner.get("id"))
                .or_else(|| args.get("conversation_id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing conversation_id".into()))?;
            let (op, payload) = extract_sub_command(args);
            if let Some(plan_val) = payload
                .get("plan")
                .or_else(|| inner.get("plan"))
                .or_else(|| args.get("plan"))
            {
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
                clear_active_run(store, conv_id);
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
                        "kind": "resolved",
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
                    "kind": "unavailable",
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
            if !supports_reasoning {
                return Ok(Value::Null);
            }
            let wire_protocol = args
                .get("wire_protocol")
                .and_then(|v| v.as_str())
                .unwrap_or("chat_completions");
            let output_limit = args.get("output_limit").and_then(|v| v.as_u64());
            let tier = args.get("tier").and_then(|v| v.as_str());
            let requested = args.get("requested").cloned().unwrap_or(Value::Null);
            let effective_effort = if let Some(t) = tier {
                json!(t)
            } else if !requested.is_null() {
                requested
            } else {
                json!("automatic")
            };
            let control = "effort";
            let selection = json!({
                "supported": true,
                "wire_protocol": wire_protocol,
                "output_limit": output_limit,
                "kind": control,
                "control": control,
                "value": effective_effort
            });
            Ok(json!({
                "supported": true,
                "wire_protocol": wire_protocol,
                "output_limit": output_limit,
                "kind": control,
                "control": control,
                "value": effective_effort,
                "selection": selection
            }))
        }
        "resolve_remote_model_profile" => resolve_remote_model_profile(store, suite, args),
        "read_ui_capability" => Ok(json!({
            "supported": true,
            "catalog_id": DEFAULT_CATALOG_ID,
            "catalog_digest": catalog_digest(),
            "max_surfaces_per_conversation": MAX_ACTIVE_SURFACES_PER_CONVERSATION,
            "model_tool": {
                "name": "render_ui_surface",
                "enabled": true
            }
        })),
        "session_create" | "create_conversation" => {
            let input = args.get("input").unwrap_or(args);
            let ts = now_ms();
            let created_at_ms = input
                .get("created_at_ms")
                .or_else(|| args.get("created_at_ms"))
                .and_then(|v| v.as_i64())
                .unwrap_or(ts);
            let updated_at_ms = input
                .get("updated_at_ms")
                .or_else(|| args.get("updated_at_ms"))
                .and_then(|v| v.as_i64())
                .unwrap_or(created_at_ms);
            let s = Session {
                id: input
                    .get("id")
                    .or_else(|| input.get("conversation_id"))
                    .or_else(|| args.get("id"))
                    .or_else(|| args.get("conversation_id"))
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
                    .map(ToString::to_string)
                    .unwrap_or_else(new_id),
                title: input
                    .get("title")
                    .or_else(|| args.get("title"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("New conversation")
                    .to_string(),
                workspace_root: input
                    .get("workspace_root")
                    .or_else(|| args.get("workspace_root"))
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string)
                    .unwrap_or_else(|| suite.workspace_root.display().to_string()),
                model_connection: input
                    .get("selected_model_source")
                    .or_else(|| input.get("selectedModelSource"))
                    .or_else(|| args.get("selected_model_source"))
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(ToString::to_string),
                model_id: input
                    .get("selected_model_id")
                    .or_else(|| input.get("selectedModelId"))
                    .or_else(|| args.get("selected_model_id"))
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(ToString::to_string),
                created_at_ms,
                updated_at_ms,
            };
            store
                .upsert_session(&s)
                .map_err(|e| ("storage_error", e.to_string()))?;
            if let Some(settings_val) = input
                .get("settings_json")
                .or_else(|| args.get("settings_json"))
                .filter(|v| !v.is_null())
            {
                let settings_str = settings_val
                    .as_str()
                    .map(ToString::to_string)
                    .unwrap_or_else(|| settings_val.to_string());
                store
                    .set_kv(&format!("conv_settings:{}", s.id), &json!(settings_str))
                    .map_err(|e| ("storage_error", e.to_string()))?;
            }
            let conv_json = crate::web_api::session_to_conversation_json(store, &s);
            let mut val = serde_json::to_value(&s).unwrap_or(json!({}));
            if let Some(obj) = val.as_object_mut() {
                obj.insert("created".into(), json!(true));
                obj.insert("initial_configuration_revision".into(), json!(1));
                obj.insert("conversation".into(), conv_json);
            }
            Ok(val)
        }
        "rename_conversation" => {
            let input = args.get("input").unwrap_or(args);
            let id = input
                .get("conversation_id")
                .or_else(|| input.get("id"))
                .or_else(|| args.get("conversation_id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing conversation_id".into()))?;
            let title = input
                .get("title")
                .or_else(|| args.get("title"))
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
            let input = args.get("input").unwrap_or(args);
            let id = input
                .get("conversation_id")
                .or_else(|| input.get("id"))
                .or_else(|| args.get("conversation_id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing conversation_id".into()))?;
            let mut sess = store
                .get_session(id)
                .map_err(|e| ("storage_error", e.to_string()))?
                .ok_or(("not_found", format!("conversation '{id}' not found")))?;

            // Merge input + optional patch_json / patch / selection into one flat map.
            let mut merged = serde_json::Map::new();
            if let Some(obj) = input.as_object() {
                for (k, v) in obj {
                    merged.insert(k.clone(), v.clone());
                }
            }
            for subkey in ["patch_json", "patch", "selection"] {
                if let Some(sub) = input.get(subkey).or_else(|| args.get(subkey)) {
                    let parsed_sub = if let Some(s) = sub.as_str() {
                        serde_json::from_str::<Value>(s).ok()
                    } else {
                        Some(sub.clone())
                    };
                    if let Some(Value::Object(map)) = parsed_sub {
                        for (k, v) in map {
                            merged.insert(k, v);
                        }
                    }
                }
            }
            let effective = Value::Object(merged);

            if let Some(src) = effective
                .get("selected_model_source")
                .or_else(|| effective.get("selectedModelSource"))
                .or_else(|| effective.get("model_connection"))
                .or_else(|| effective.get("connection_id"))
                .and_then(|v| v.as_str())
            {
                sess.model_connection = (!src.is_empty()).then(|| src.to_string());
            }
            if let Some(mid) = effective
                .get("selected_model_id")
                .or_else(|| effective.get("selectedModelId"))
                .or_else(|| effective.get("model_id"))
                .and_then(|v| v.as_str())
            {
                sess.model_id = (!mid.is_empty()).then(|| mid.to_string());
            }
            if let Some(title) = effective.get("title").and_then(|v| v.as_str()) {
                if !title.trim().is_empty() {
                    sess.title = title.to_string();
                }
            }
            if let Some(ws) = effective.get("workspace_root").and_then(|v| v.as_str()) {
                if !ws.trim().is_empty() {
                    sess.workspace_root = ws.to_string();
                }
            }

            let key = crate::web_api::session_meta_key(id);
            let mut meta = store
                .get_kv(&key)
                .ok()
                .flatten()
                .unwrap_or_else(
                    || json!({"isPinned": false, "revision": 1, "configurationRevision": 1}),
                );
            if let Some(pinned) = effective
                .get("is_pinned")
                .or_else(|| effective.get("isPinned"))
                .and_then(|v| v.as_bool())
            {
                meta["isPinned"] = json!(pinned);
            }
            if let Some(eff) = effective
                .get("reasoning_effort")
                .or_else(|| effective.get("reasoningEffort"))
                .and_then(|v| v.as_str())
            {
                meta["reasoningEffort"] = json!(eff);
            }
            if let Some(perm) = effective
                .get("permission_profile")
                .or_else(|| effective.get("permissionProfile"))
                .or_else(|| effective.get("profile"))
                .and_then(|v| v.as_str())
            {
                if let Ok(parsed) = PermissionProfile::parse_strict(perm) {
                    let canonical = parsed.as_canonical_str();
                    meta["permissionProfile"] = json!(canonical);
                    let _ = store.set_kv(&format!("conv_perm:{id}"), &json!(canonical));
                }
            }
            let next_cfg = meta
                .get("configurationRevision")
                .and_then(|v| v.as_i64())
                .unwrap_or(1)
                + 1;
            meta["configurationRevision"] = json!(next_cfg);
            let _ = store.set_kv(&key, &meta);

            // Merge updated settings into `conv_settings:{id}` so `load_conversation` & `load_conversation_settings` see them.
            let mut settings_map = load_conversation_settings_str(store, id)
                .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
                .and_then(|v| v.as_object().cloned())
                .unwrap_or_default();
            if let Some(sj) = effective.get("settings_json").filter(|v| !v.is_null()) {
                let parsed = if let Some(s) = sj.as_str() {
                    serde_json::from_str::<Value>(s).ok()
                } else {
                    Some(sj.clone())
                };
                if let Some(Value::Object(map)) = parsed {
                    for (k, v) in map {
                        settings_map.insert(k, v);
                    }
                }
            }
            if let Some(eff_obj) = effective.as_object() {
                for (k, v) in eff_obj {
                    if !matches!(
                        k.as_str(),
                        "type"
                            | "conversation_id"
                            | "id"
                            | "input"
                            | "expected_revision"
                            | "patch_json"
                            | "patch"
                            | "selection"
                            | "settings_json"
                    ) {
                        settings_map.insert(k.clone(), v.clone());
                    }
                }
            }
            let merged_settings_str =
                serde_json::to_string(&Value::Object(settings_map)).unwrap_or_else(|_| "{}".into());
            let _ = store.set_kv(&format!("conv_settings:{id}"), &json!(merged_settings_str));

            sess.updated_at_ms = now_ms();
            store
                .upsert_session(&sess)
                .map_err(|e| ("storage_error", e.to_string()))?;
            let mut conv_json = crate::web_api::session_to_conversation_json(store, &sess);
            if let Some(obj) = conv_json.as_object_mut() {
                obj.insert("settings_json".into(), json!(merged_settings_str));
                obj.insert("configuration_revision".into(), json!(next_cfg));
            }
            Ok(json!({
                "status": "committed",
                "snapshot": conv_json,
                "configuration_revision": next_cfg,
                "settings_json": merged_settings_str,
                "conversation": conv_json
            }))
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
            let input = args.get("input").unwrap_or(args);
            let key = input
                .get("key")
                .or_else(|| args.get("key"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'key'".into()))?;
            match load_product_setting_entry(store, key)? {
                Some((val, val_json, rev, updated_at_ms)) => Ok(json!({
                    "key": key,
                    "value_json": val_json,
                    "value": val,
                    "revision": rev,
                    "updated_at_ms": updated_at_ms
                })),
                None => Ok(Value::Null),
            }
        }
        "put_product_setting" => put_product_setting_impl(store, args, false),
        "put_product_setting_checked" => put_product_setting_impl(store, args, true),
        "delete_product_setting" => {
            let input = args.get("input").unwrap_or(args);
            let key = input
                .get("key")
                .or_else(|| args.get("key"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'key'".into()))?;
            let expected_rev = input
                .get("expected_revision")
                .or_else(|| args.get("expected_revision"))
                .and_then(|v| v.as_u64());
            let existing = load_product_setting_entry(store, key)?;
            let cur_rev = existing.as_ref().map(|(_, _, r, _)| *r).unwrap_or(0);
            if let Some(exp) = expected_rev {
                if cur_rev != exp {
                    return Err((
                        "revision_conflict",
                        format!("product_setting '{key}' revision conflict: expected {exp}, actual {cur_rev}"),
                    ));
                }
            }
            let next_rev = cur_rev + 1;
            store
                .set_kv(
                    &format!("product_setting:{key}"),
                    &json!({
                        "__nola_setting": true,
                        "deleted": true,
                        "revision": next_rev,
                        "updated_at_ms": now_ms()
                    }),
                )
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({
                "key": key,
                "deleted": existing.is_some(),
                "revision": next_rev,
                "ok": true
            }))
        }
        "set_conversation_permission" => {
            let input = args.get("input").unwrap_or(args);
            let id = input
                .get("conversation_id")
                .or_else(|| input.get("id"))
                .or_else(|| args.get("conversation_id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'conversation_id'".into()))?;
            let raw_mode = input
                .get("profile")
                .or_else(|| input.get("permission_mode"))
                .or_else(|| input.get("mode"))
                .or_else(|| input.get("permission_profile"))
                .or_else(|| args.get("profile"))
                .or_else(|| args.get("permission_mode"))
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
            let conv_json = store
                .get_session(id)
                .ok()
                .flatten()
                .map(|s| crate::web_api::session_to_conversation_json(store, &s))
                .unwrap_or_else(|| {
                    json!({
                        "conversationId": id,
                        "id": id,
                        "permissionProfile": canonical_mode,
                        "configurationRevision": next_cfg
                    })
                });
            Ok(json!({
                "status": "committed",
                "snapshot": conv_json,
                "conversation": conv_json,
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
            synchronize_session_state_after_truncate(store, id);
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
            let up_to_seq = args
                .get("up_to_seq")
                .or_else(|| args.get("upToSeq"))
                .or_else(|| args.get("branch_seq"))
                .or_else(|| args.get("message_seq"))
                .or_else(|| args.get("boundary_seq"))
                .and_then(|v| v.as_i64());
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

            // Copy transcript events paginated via read_event_range up to optional `up_to_seq`.
            let mut next_seq = 1i64;
            loop {
                let batch = store
                    .read_event_range(id, next_seq, 500)
                    .map_err(|e| ("storage_error", e.to_string()))?;
                if batch.is_empty() {
                    break;
                }
                let mut reached_limit = false;
                for e in &batch {
                    if let Some(max_seq) = up_to_seq {
                        if e.seq > max_seq {
                            reached_limit = true;
                            break;
                        }
                    }
                    let _ = store.append_event(&new_conv_id, &e.kind, e.payload.clone(), 50_000);
                    next_seq = e.seq + 1;
                }
                if reached_limit || batch.len() < 500 {
                    break;
                }
            }

            // Copy session_state, meta, and knowledge from source to branched conversation.
            if let Ok(Some(state_val)) = store.get_kv(&format!("session_state:{id}")) {
                let _ = store.set_kv(&format!("session_state:{new_conv_id}"), &state_val);
            }
            if up_to_seq.is_some() {
                synchronize_session_state_after_truncate(store, &new_conv_id);
            }
            if let Ok(Some(meta_val)) = store.get_kv(&crate::web_api::session_meta_key(id)) {
                let _ = store.set_kv(&crate::web_api::session_meta_key(&new_conv_id), &meta_val);
            }
            if let Ok(Some(k_val)) = store.get_kv(&crate::web_api::conversation_knowledge_key(id)) {
                let _ = store.set_kv(
                    &crate::web_api::conversation_knowledge_key(&new_conv_id),
                    &k_val,
                );
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
        "observe_automation_execution" => {
            let run_id = args
                .get("run_id")
                .or_else(|| args.get("execution_id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str());
            let runs: Vec<Value> = suite
                .list_records("automation_run")
                .map_err(|e| ("storage_error", e.to_string()))?;
            let found = run_id.and_then(|rid| {
                runs.iter()
                    .find(|r| r.get("id").and_then(|v| v.as_str()) == Some(rid))
                    .cloned()
            });
            Ok(json!({
                "execution": found,
                "runs": runs
            }))
        }
        "list_pending_automation_executions" => {
            let runs: Vec<Value> = suite
                .list_records("automation_run")
                .map_err(|e| ("storage_error", e.to_string()))?;
            let pending: Vec<Value> = runs
                .into_iter()
                .filter(|r| {
                    matches!(
                        r.get("status").and_then(|v| v.as_str()).unwrap_or("queued"),
                        "queued" | "pending" | "running"
                    )
                })
                .collect();
            Ok(json!({
                "executions": pending,
                "items": pending
            }))
        }
        "reconcile_automation_execution" => {
            let input = args.get("input").unwrap_or(args);
            let run_id = input
                .get("run_id")
                .or_else(|| input.get("execution_id"))
                .or_else(|| input.get("id"))
                .or_else(|| args.get("run_id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'run_id'".into()))?;
            let mut rec = suite
                .get_record::<Value>("automation_run", run_id)
                .map_err(|e| ("storage_error", e.to_string()))?
                .unwrap_or_else(|| json!({"id": run_id}));
            let status = input
                .get("status")
                .or_else(|| input.get("outcome"))
                .and_then(|v| v.as_str())
                .unwrap_or("completed");
            if let Some(obj) = rec.as_object_mut() {
                obj.insert("status".into(), json!(status));
                if let Some(out) = input.get("output").or_else(|| input.get("result")) {
                    obj.insert("output".into(), out.clone());
                }
                if let Some(err) = input.get("error") {
                    obj.insert("error".into(), err.clone());
                }
                obj.insert("reconciled_at_ms".into(), json!(now_ms()));
                obj.insert("updated_at_ms".into(), json!(now_ms()));
            }
            suite
                .upsert_record("automation_run", run_id, &rec)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"ok": true, "execution": rec, "run": rec}))
        }
        "observe_automation_worker" => {
            let runs: Vec<Value> = suite.list_records("automation_run").unwrap_or_default();
            let pending_count = runs
                .iter()
                .filter(|r| {
                    matches!(
                        r.get("status").and_then(|v| v.as_str()),
                        Some("queued" | "pending")
                    )
                })
                .count();
            let running_count = runs
                .iter()
                .filter(|r| r.get("status").and_then(|v| v.as_str()) == Some("running"))
                .count();
            Ok(json!({
                "status": if running_count > 0 { "active" } else { "idle" },
                "pending_count": pending_count,
                "running_count": running_count,
                "observed_at_ms": now_ms()
            }))
        }
        "ack_automation_notification" => {
            let input = args.get("input").unwrap_or(args);
            let id = input
                .get("notification_id")
                .or_else(|| input.get("id"))
                .or_else(|| args.get("notification_id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .unwrap_or("default");
            let ts = now_ms();
            store
                .set_kv(
                    &format!("automation_notif_ack:{id}"),
                    &json!({"id": id, "acknowledged_at_ms": ts}),
                )
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"ok": true, "acknowledged": true, "id": id, "acknowledged_at_ms": ts}))
        }
        "query_automation_occurrences" => {
            let def_id = args
                .get("automation_id")
                .or_else(|| args.get("definition_id"))
                .and_then(|v| v.as_str());
            let limit = args
                .get("limit")
                .and_then(|v| v.as_u64())
                .unwrap_or(100)
                .clamp(1, 500) as usize;
            let runs: Vec<Value> = suite
                .list_records::<Value>("automation_run")
                .map_err(|e| ("storage_error", e.to_string()))?
                .into_iter()
                .filter(|r| {
                    def_id.is_none_or(|did| {
                        r.get("automation_id")
                            .or_else(|| r.get("definition_id"))
                            .and_then(|v| v.as_str())
                            == Some(did)
                    })
                })
                .take(limit)
                .collect();
            Ok(json!({"occurrences": runs, "items": runs}))
        }
        "load_conversation_metadata" => {
            let id = args
                .get("conversation_id")
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing conversation_id".into()))?;
            let sess = store
                .get_session(id)
                .map_err(|e| ("storage_error", e.to_string()))?
                .ok_or(("not_found", format!("conversation '{id}' not found")))?;
            let meta = store
                .get_kv(&format!("conv_metadata:{id}"))
                .ok()
                .flatten()
                .unwrap_or_else(|| json!({}));
            let sess_meta = store
                .get_kv(&crate::web_api::session_meta_key(id))
                .ok()
                .flatten()
                .unwrap_or_else(|| json!({}));
            let rev = sess_meta
                .get("revision")
                .and_then(|v| v.as_i64())
                .unwrap_or(1)
                .max(1);
            let conv = crate::web_api::session_to_conversation_json(store, &sess);
            Ok(json!({
                "id": id,
                "conversation_id": id,
                "metadata": meta,
                "conversation": conv,
                "revision": rev
            }))
        }
        "patch_conversation_metadata" => {
            let input = args.get("input").unwrap_or(args);
            let id = input
                .get("conversation_id")
                .or_else(|| input.get("id"))
                .or_else(|| args.get("conversation_id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing conversation_id".into()))?;
            let mut sess = store
                .get_session(id)
                .map_err(|e| ("storage_error", e.to_string()))?
                .ok_or(("not_found", format!("conversation '{id}' not found")))?;
            let mut meta = store
                .get_kv(&format!("conv_metadata:{id}"))
                .ok()
                .flatten()
                .unwrap_or_else(|| json!({}));
            let patch_src = input
                .get("metadata")
                .or_else(|| input.get("patch"))
                .or_else(|| input.get("patch_json"))
                .unwrap_or(input);
            let patch_obj = if let Some(s) = patch_src.as_str() {
                serde_json::from_str::<Value>(s).unwrap_or_else(|_| json!({}))
            } else {
                patch_src.clone()
            };
            if let (Some(dst), Some(src)) = (meta.as_object_mut(), patch_obj.as_object()) {
                for (k, v) in src {
                    if !matches!(k.as_str(), "type" | "conversation_id" | "input") {
                        dst.insert(k.clone(), v.clone());
                    }
                }
            }
            if let Some(title) = patch_obj
                .get("title")
                .or_else(|| input.get("title"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
            {
                sess.title = title.to_string();
            }
            let sess_meta_key = crate::web_api::session_meta_key(id);
            let mut smeta = store
                .get_kv(&sess_meta_key)
                .ok()
                .flatten()
                .unwrap_or_else(
                    || json!({"isPinned": false, "revision": 1, "configurationRevision": 1}),
                );
            if let Some(pinned) = patch_obj
                .get("is_pinned")
                .or_else(|| patch_obj.get("isPinned"))
                .or_else(|| input.get("is_pinned"))
                .or_else(|| input.get("isPinned"))
                .and_then(|v| v.as_bool())
            {
                smeta["isPinned"] = json!(pinned);
            }
            let next_rev = smeta.get("revision").and_then(|v| v.as_i64()).unwrap_or(1) + 1;
            smeta["revision"] = json!(next_rev);
            let _ = store.set_kv(&sess_meta_key, &smeta);
            store
                .set_kv(&format!("conv_metadata:{id}"), &meta)
                .map_err(|e| ("storage_error", e.to_string()))?;
            sess.updated_at_ms = now_ms();
            store
                .upsert_session(&sess)
                .map_err(|e| ("storage_error", e.to_string()))?;
            let conv = crate::web_api::session_to_conversation_json(store, &sess);
            Ok(json!({
                "status": "committed",
                "conversation_id": id,
                "metadata": meta,
                "snapshot": conv,
                "conversation": conv,
                "revision": next_rev,
                "ok": true
            }))
        }
        "search_conversation_text" => {
            let conv_id = args
                .get("conversation_id")
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str());
            let query = args
                .get("query")
                .or_else(|| args.get("q"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            let q_lower = query.to_ascii_lowercase();
            let limit = args
                .get("limit")
                .and_then(|v| v.as_u64())
                .unwrap_or(50)
                .clamp(1, 200) as usize;
            let targets: Vec<String> = if let Some(cid) = conv_id {
                vec![cid.to_string()]
            } else {
                store
                    .list_sessions(50)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|s| s.id)
                    .collect()
            };
            let mut matches = Vec::new();
            for cid in targets {
                let (msgs, _) = crate::web_api::reconstruct_v2_messages(store, &cid);
                for m in msgs {
                    let content = m.get("content").and_then(|v| v.as_str()).unwrap_or("");
                    if q_lower.is_empty() || content.to_ascii_lowercase().contains(&q_lower) {
                        matches.push(json!({
                            "conversation_id": cid,
                            "message_id": m.get("id").cloned().unwrap_or(Value::Null),
                            "seq": m.get("seq").cloned().unwrap_or(json!(0)),
                            "role": m.get("role").cloned().unwrap_or(json!("assistant")),
                            "snippet": content.chars().take(240).collect::<String>()
                        }));
                        if matches.len() >= limit {
                            break;
                        }
                    }
                }
                if matches.len() >= limit {
                    break;
                }
            }
            let total = matches.len();
            Ok(json!({
                "conversation_id": conv_id,
                "query": query,
                "matches": matches,
                "total": total
            }))
        }
        "prepare_goal_control_intent" => {
            let input = args.get("input").unwrap_or(args);
            let goal_id = input
                .get("goal_id")
                .or_else(|| input.get("id"))
                .or_else(|| args.get("goal_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let conv_id = input
                .get("conversation_id")
                .or_else(|| args.get("conversation_id"))
                .and_then(|v| v.as_str());
            let action = input
                .get("action")
                .or_else(|| input.get("intent"))
                .or_else(|| args.get("action"))
                .and_then(|v| v.as_str())
                .unwrap_or("advance");
            let intent_id = new_id();
            let rec = json!({
                "intent_id": intent_id,
                "goal_id": goal_id,
                "conversation_id": conv_id,
                "action": action,
                "prepared_at_ms": now_ms(),
                "ready": true
            });
            let _ = store.set_kv(&format!("goal_intent:{intent_id}"), &rec);
            Ok(json!({
                "ok": true,
                "intent_id": intent_id,
                "goal_id": goal_id,
                "conversation_id": conv_id,
                "action": action,
                "ready": true
            }))
        }
        "prepare_goal_execution" => {
            let input = args.get("input").unwrap_or(args);
            let goal_id = input
                .get("goal_id")
                .or_else(|| input.get("id"))
                .or_else(|| args.get("goal_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let conv_id = input
                .get("conversation_id")
                .or_else(|| args.get("conversation_id"))
                .and_then(|v| v.as_str());
            let exec_id = new_id();
            let host_id = store
                .get_kv("host_id")
                .ok()
                .flatten()
                .and_then(|v| v.as_str().map(ToString::to_string))
                .unwrap_or_else(|| "local-host".into());
            let owner = json!({
                "execution_id": exec_id,
                "goal_id": goal_id,
                "conversation_id": conv_id,
                "host_id": host_id,
                "prepared_at_ms": now_ms()
            });
            if !goal_id.is_empty() {
                let _ = store.set_kv(&format!("goal_exec_owner:{goal_id}"), &owner);
            }
            if let Some(cid) = conv_id {
                let _ = store.set_kv(&format!("goal_exec_owner_conv:{cid}"), &owner);
            }
            Ok(json!({
                "ok": true,
                "execution_id": exec_id,
                "goal_id": goal_id,
                "conversation_id": conv_id,
                "owner": owner,
                "ready": true
            }))
        }
        "load_goal_execution_owner" => {
            let goal_id = args
                .get("goal_id")
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str());
            let conv_id = args.get("conversation_id").and_then(|v| v.as_str());
            let owner = goal_id
                .and_then(|gid| {
                    store
                        .get_kv(&format!("goal_exec_owner:{gid}"))
                        .ok()
                        .flatten()
                })
                .or_else(|| {
                    conv_id.and_then(|cid| {
                        store
                            .get_kv(&format!("goal_exec_owner_conv:{cid}"))
                            .ok()
                            .flatten()
                    })
                });
            Ok(json!({
                "goal_id": goal_id,
                "conversation_id": conv_id,
                "owner": owner.unwrap_or(Value::Null)
            }))
        }
        "effective_permission_grants" => {
            let conv_id = args
                .get("conversation_id")
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str());
            let (profile, rev) = read_permission_state(store, conv_id);
            let grants = conv_id
                .and_then(|cid| store.get_kv(&format!("conv_grants:{cid}")).ok().flatten())
                .and_then(|v| v.as_array().cloned())
                .unwrap_or_default();
            Ok(json!({
                "conversation_id": conv_id,
                "permission_profile": profile,
                "grants": grants,
                "revision": rev
            }))
        }
        "canonical_run_terminal" => {
            let run_id = args
                .get("run_id")
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str());
            let conv_id = args.get("conversation_id").and_then(|v| v.as_str());
            let stored = run_id.and_then(|rid| {
                store.get_kv(&format!("run_terminal:{rid}")).ok().flatten()
            });
            if let Some(val) = stored {
                return Ok(val);
            }
            Ok(json!({
                "run_id": run_id,
                "conversation_id": conv_id,
                "status": "completed",
                "terminal": true
            }))
        }
        "inspect_unknown_effect_recovery" => {
            let conv_id = args
                .get("conversation_id")
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let mut items = store
                .get_kv(&format!("unknown_effect_recovery:{conv_id}"))
                .ok()
                .flatten()
                .and_then(|v| v.as_array().cloned())
                .unwrap_or_default();
            let verification = store
                .get_kv(&format!("turn_verification:{conv_id}"))
                .ok()
                .flatten();
            if let Some(ref ver) = verification {
                if let Some(files) = ver.get("modified_files").and_then(|v| v.as_array()) {
                    for f in files {
                        if f.get("verified_after_edit").and_then(|v| v.as_bool()) == Some(false) {
                            items.push(json!({
                                "kind": "unverified_file_modification",
                                "uri": f.get("uri").cloned().unwrap_or(Value::Null),
                                "operation": f.get("operation").cloned().unwrap_or(Value::Null),
                                "last_modified_step": f.get("last_modified_step").cloned().unwrap_or(Value::Null)
                            }));
                        }
                    }
                }
            }
            Ok(json!({
                "conversation_id": conv_id,
                "pending": !items.is_empty(),
                "items": items,
                "turn_verification": verification.unwrap_or(Value::Null)
            }))
        }
        "resolve_unknown_effect_recovery" => {
            let input = args.get("input").unwrap_or(args);
            let conv_id = input
                .get("conversation_id")
                .or_else(|| input.get("id"))
                .or_else(|| args.get("conversation_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let _ = store.set_kv(&format!("unknown_effect_recovery:{conv_id}"), &json!([]));
            Ok(json!({
                "ok": true,
                "conversation_id": conv_id,
                "resolved": true
            }))
        }
        "inspect_owned_task_recovery" => {
            let conv_id = args
                .get("conversation_id")
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let mut tasks = store
                .get_kv(&format!("owned_task_recovery:{conv_id}"))
                .ok()
                .flatten()
                .and_then(|v| v.as_array().cloned())
                .unwrap_or_default();
            if let Ok(durable) = store.list_module_records::<Value>("durable_task") {
                for t in durable {
                    let sess_match = conv_id.is_empty()
                        || t.get("session_id")
                            .and_then(|v| v.as_str())
                            .is_some_and(|s| s == conv_id || s.starts_with(&format!("{conv_id}:")));
                    let status = t.get("status").and_then(|v| v.as_str()).unwrap_or("");
                    if sess_match && matches!(status, "running" | "interrupted") {
                        tasks.push(t);
                    }
                }
            }
            Ok(json!({
                "conversation_id": conv_id,
                "pending": !tasks.is_empty(),
                "tasks": tasks
            }))
        }
        "resolve_owned_task_recovery" => {
            let input = args.get("input").unwrap_or(args);
            let conv_id = input
                .get("conversation_id")
                .or_else(|| input.get("id"))
                .or_else(|| args.get("conversation_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let _ = store.set_kv(&format!("owned_task_recovery:{conv_id}"), &json!([]));
            Ok(json!({
                "ok": true,
                "conversation_id": conv_id,
                "resolved": true
            }))
        }
        "read_historical_part" => {
            let conv_id = args
                .get("conversation_id")
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing conversation_id".into()))?;
            let part_id = args
                .get("part_id")
                .or_else(|| args.get("message_id"))
                .and_then(|v| v.as_str());
            let target_seq = args.get("seq").and_then(|v| v.as_i64());
            let query = args.get("query").and_then(|v| v.as_str());

            let found_event = if let Some(seq) = target_seq {
                store
                    .read_event_range(conv_id, seq, 1)
                    .map_err(|e| ("storage_error", e.to_string()))?
                    .into_iter()
                    .find(|e| e.seq == seq)
            } else if let Some(q) = query.filter(|s| !s.trim().is_empty()) {
                store
                    .search_events_fts(Some(conv_id), q, 1)
                    .map_err(|e| ("storage_error", e.to_string()))?
                    .into_iter()
                    .next()
            } else if let Some(pid) = part_id {
                let direct_seq = pid
                    .strip_prefix("ev-")
                    .or_else(|| pid.strip_prefix("msg-"))
                    .and_then(|rest| rest.split(':').next())
                    .and_then(|s| s.parse::<i64>().ok());
                let mut hit = if let Some(sq) = direct_seq {
                    store
                        .read_event_range(conv_id, sq, 1)
                        .ok()
                        .and_then(|v| v.into_iter().find(|e| e.seq == sq))
                } else {
                    None
                };
                if hit.is_none() {
                    if let Ok(fts_hits) = store.search_events_fts(Some(conv_id), pid, 16) {
                        hit = fts_hits.into_iter().find(|e| {
                            e.payload.get("id").and_then(|v| v.as_str()) == Some(pid)
                                || e.payload.get("message_id").and_then(|v| v.as_str()) == Some(pid)
                                || e.payload.get("part_id").and_then(|v| v.as_str()) == Some(pid)
                        });
                    }
                }
                if hit.is_none() {
                    let mut cursor = 1i64;
                    while hit.is_none() {
                        let batch = store
                            .read_event_range(conv_id, cursor, 64)
                            .map_err(|e| ("storage_error", e.to_string()))?;
                        if batch.is_empty() {
                            break;
                        }
                        cursor = batch.last().map(|e| e.seq + 1).unwrap_or(cursor + 64);
                        hit = batch.into_iter().find(|e| {
                            format!("ev-{}", e.seq) == pid
                                || format!("msg-{}", e.seq) == pid
                                || e.payload.get("id").and_then(|v| v.as_str()) == Some(pid)
                                || e.payload.get("message_id").and_then(|v| v.as_str()) == Some(pid)
                                || e.payload.get("part_id").and_then(|v| v.as_str()) == Some(pid)
                        });
                    }
                }
                hit
            } else {
                None
            };

            match found_event {
                Some(ev) => {
                    let content = ev
                        .payload
                        .get("content")
                        .or_else(|| ev.payload.get("text"))
                        .or_else(|| ev.payload.get("delta"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let role = ev
                        .payload
                        .get("role")
                        .and_then(|v| v.as_str())
                        .unwrap_or_else(|| match ev.kind.as_str() {
                            "user_message" => "user",
                            "tool_result" => "tool",
                            _ => "assistant",
                        });
                    let resolved_id = ev
                        .payload
                        .get("id")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string)
                        .unwrap_or_else(|| format!("ev-{}", ev.seq));
                    Ok(json!({
                        "conversation_id": conv_id,
                        "part_id": part_id.unwrap_or(&resolved_id),
                        "seq": ev.seq,
                        "kind": ev.kind,
                        "content": content,
                        "message": {
                            "id": resolved_id,
                            "seq": ev.seq,
                            "role": role,
                            "content": content,
                            "created_at_ms": ev.created_at_ms
                        },
                        "found": true
                    }))
                }
                None => Ok(json!({
                    "conversation_id": conv_id,
                    "part_id": part_id,
                    "content": "",
                    "found": false
                })),
            }
        }
        "read_conversation_text_range" => {
            let conv_id = args
                .get("conversation_id")
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing conversation_id".into()))?;
            let query = args
                .get("query")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let from_seq = args
                .get("from_seq")
                .or_else(|| args.get("start_seq"))
                .and_then(|v| v.as_i64());
            let start_offset = args
                .get("start_offset")
                .or_else(|| args.get("offset"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize;
            let max_chars = args
                .get("max_chars")
                .or_else(|| args.get("length"))
                .or_else(|| args.get("limit"))
                .and_then(|v| v.as_u64())
                .unwrap_or(8192)
                .clamp(1, 200_000) as usize;

            let format_ev = |ev: &nola_store::StoredEvent| -> Option<String> {
                let text = ev
                    .payload
                    .get("content")
                    .or_else(|| ev.payload.get("text"))
                    .or_else(|| ev.payload.get("delta"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if text.is_empty() {
                    return None;
                }
                let role = ev
                    .payload
                    .get("role")
                    .and_then(|v| v.as_str())
                    .unwrap_or_else(|| match ev.kind.as_str() {
                        "user_message" => "user",
                        "tool_result" => "tool",
                        _ => "assistant",
                    });
                Some(format!("[{role}]\n{text}"))
            };

            if let Some(q) = query {
                let hits = store
                    .search_events_fts(Some(conv_id), q, 64)
                    .map_err(|e| ("storage_error", e.to_string()))?;
                let matched_text = hits
                    .iter()
                    .filter_map(format_ev)
                    .collect::<Vec<_>>()
                    .join("\n\n");
                let chars: Vec<char> = matched_text.chars().collect();
                let total = chars.len();
                let start = start_offset.min(total);
                let end = (start + max_chars).min(total);
                let slice: String = chars[start..end].iter().collect();
                return Ok(json!({
                    "conversation_id": conv_id,
                    "text": slice,
                    "start_offset": start,
                    "end_offset": end,
                    "total_chars": total,
                    "has_more": end < total,
                    "match_count": hits.len()
                }));
            }

            // Stream bounded slices via `Store::read_event_range` without materializing full transcript!
            let mut cursor_seq = from_seq.unwrap_or(1).max(1);
            let mut collected = String::new();
            let mut seen_chars = 0usize;
            let mut slice_out = String::new();
            let mut has_more = false;
            let target_end = start_offset.saturating_add(max_chars);

            'outer: loop {
                let batch = store
                    .read_event_range(conv_id, cursor_seq, 64)
                    .map_err(|e| ("storage_error", e.to_string()))?;
                if batch.is_empty() {
                    break;
                }
                let batch_len = batch.len();
                cursor_seq = batch.last().map(|e| e.seq + 1).unwrap_or(cursor_seq + 64);
                for ev in &batch {
                    let Some(block) = format_ev(ev) else {
                        continue;
                    };
                    if !collected.is_empty() {
                        collected.clear();
                        for ch in "\n\n".chars() {
                            if seen_chars >= start_offset && seen_chars < target_end {
                                slice_out.push(ch);
                            }
                            seen_chars += 1;
                            if seen_chars > target_end {
                                has_more = true;
                                break 'outer;
                            }
                        }
                    }
                    collected.push('x');
                    for ch in block.chars() {
                        if seen_chars >= start_offset && seen_chars < target_end {
                            slice_out.push(ch);
                        }
                        seen_chars += 1;
                        if seen_chars > target_end {
                            has_more = true;
                            break 'outer;
                        }
                    }
                }
                if batch_len < 64 {
                    break;
                }
            }
            let end_offset = start_offset + slice_out.chars().count();
            Ok(json!({
                "conversation_id": conv_id,
                "text": slice_out,
                "start_offset": start_offset,
                "end_offset": end_offset,
                "total_chars": seen_chars,
                "has_more": has_more
            }))
        }
        "load_conversation_task_projection_v1" => {
            let conv_id = args
                .get("conversation_id")
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing conversation_id".into()))?;
            let todos = load_session_todos(store, conv_id);
            let plan = load_session_plan(store, conv_id);
            let goals: Vec<GoalRecord> = suite.list_records("goal").unwrap_or_default();
            Ok(json!({
                "conversation_id": conv_id,
                "todos": todos,
                "tasks": todos,
                "plan": plan,
                "goals": goals
            }))
        }
        "context" => dispatch_context_command(store, args),
        "prepare_in_place_resend" => {
            let input = args.get("input").unwrap_or(args);
            let conv_id = input
                .get("conversation_id")
                .or_else(|| input.get("id"))
                .or_else(|| args.get("conversation_id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing conversation_id".into()))?;
            if let Some(from_seq) = input
                .get("from_seq")
                .or_else(|| input.get("truncate_from_seq"))
                .or_else(|| input.get("target_seq"))
                .and_then(|v| v.as_i64())
            {
                store
                    .truncate_events_from(conv_id, from_seq)
                    .map_err(|e| ("storage_error", e.to_string()))?;
                synchronize_session_state_after_truncate(store, conv_id);
            }
            let mut rec = input.clone();
            if let Some(obj) = rec.as_object_mut() {
                obj.insert("conversation_id".into(), json!(conv_id));
                obj.insert("prepared_at_ms".into(), json!(now_ms()));
                obj.insert("kind".into(), json!("in_place_resend"));
            }
            store
                .set_kv(&format!("prepared_in_place_resend:{conv_id}"), &rec)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"ok": true, "conversation_id": conv_id, "prepared": rec}))
        }
        "prepare_edited_resend" => {
            let input = args.get("input").unwrap_or(args);
            let conv_id = input
                .get("conversation_id")
                .or_else(|| input.get("id"))
                .or_else(|| args.get("conversation_id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing conversation_id".into()))?;
            if let Some(from_seq) = input
                .get("from_seq")
                .or_else(|| input.get("truncate_from_seq"))
                .or_else(|| input.get("target_seq"))
                .and_then(|v| v.as_i64())
            {
                store
                    .truncate_events_from(conv_id, from_seq)
                    .map_err(|e| ("storage_error", e.to_string()))?;
                synchronize_session_state_after_truncate(store, conv_id);
            }
            let mut rec = input.clone();
            if let Some(obj) = rec.as_object_mut() {
                obj.insert("conversation_id".into(), json!(conv_id));
                obj.insert("prepared_at_ms".into(), json!(now_ms()));
                obj.insert("kind".into(), json!("edited_resend"));
            }
            store
                .set_kv(&format!("prepared_edited_resend:{conv_id}"), &rec)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"ok": true, "conversation_id": conv_id, "prepared": rec}))
        }
        "load_prepared_edited_resend" => {
            let conv_id = args
                .get("conversation_id")
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing conversation_id".into()))?;
            let found = store
                .get_kv(&format!("prepared_edited_resend:{conv_id}"))
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(found.unwrap_or(Value::Null))
        }
        "load_prepared_in_place_resend" => {
            let conv_id = args
                .get("conversation_id")
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing conversation_id".into()))?;
            let found = store
                .get_kv(&format!("prepared_in_place_resend:{conv_id}"))
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(found.unwrap_or(Value::Null))
        }
        "load_conversation_source_owner" => {
            let _conv_id = args
                .get("conversation_id")
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing conversation_id".into()))?;
            Ok(json!({"kind": "local"}))
        }
        "prepare_remote_submission" => {
            let input = args.get("input").unwrap_or(args);
            let conv_id = input
                .get("conversation_id")
                .or_else(|| args.get("conversation_id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'conversation_id'".into()))?;
            let sub_id = input
                .get("submission_id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(new_id);
            let source_host_id = input
                .get("source_host_id")
                .and_then(|v| v.as_str())
                .unwrap_or("local-host");
            let client_id = input
                .get("client_id")
                .and_then(|v| v.as_str())
                .unwrap_or("local-client");
            let user_message_id = input
                .get("user_message_id")
                .and_then(|v| v.as_str())
                .unwrap_or(&sub_id);
            let assistant_message_id = input
                .get("assistant_message_id")
                .and_then(|v| v.as_str())
                .map(ToString::to_string)
                .unwrap_or_else(|| format!("{sub_id}:assistant"));
            let identity = json!({
                "source_host_id": source_host_id,
                "client_id": client_id,
                "conversation_id": conv_id,
                "submission_id": sub_id,
                "user_message_id": user_message_id,
                "assistant_message_id": assistant_message_id
            });
            let intent = json!({
                "identity": identity,
                "request_digest": Value::Null,
                "phase": "prepared",
                "created_at_ms": input.get("created_at_ms").and_then(|v| v.as_i64()).unwrap_or_else(now_ms)
            });
            store
                .set_kv(&format!("remote_sub_intent:{conv_id}"), &intent)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(intent)
        }
        "bind_remote_submission_request" => {
            let input = args.get("input").unwrap_or(args);
            let identity = input
                .get("identity")
                .or_else(|| args.get("identity"))
                .cloned()
                .ok_or(("invalid_argument", "missing 'identity'".into()))?;
            let request = input
                .get("request")
                .or_else(|| args.get("request"))
                .cloned()
                .unwrap_or_else(|| json!({}));
            let conv_id = identity
                .get("conversation_id")
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing identity.conversation_id".into()))?;
            let sub_id = identity
                .get("submission_id")
                .and_then(|v| v.as_str())
                .unwrap_or("default");
            let digest = sha256_hex(canonical_json_value(&request).to_string().as_bytes());
            store
                .set_kv(&format!("remote_sub_req:{conv_id}:{sub_id}"), &request)
                .map_err(|e| ("storage_error", e.to_string()))?;
            store
                .set_kv(&format!("remote_sub_req_digest:{conv_id}:{digest}"), &request)
                .map_err(|e| ("storage_error", e.to_string()))?;
            let intent = json!({
                "identity": identity,
                "request_digest": digest,
                "phase": "bound"
            });
            store
                .set_kv(&format!("remote_sub_intent:{conv_id}"), &intent)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(intent)
        }
        "read_remote_submission_intent" => {
            let conv_id = args
                .get("conversation_id")
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'conversation_id'".into()))?;
            let found = store
                .get_kv(&format!("remote_sub_intent:{conv_id}"))
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(found.unwrap_or(Value::Null))
        }
        "read_remote_submission_request" => {
            let input = args.get("input").unwrap_or(args);
            let identity = input.get("identity").or_else(|| args.get("identity"));
            let conv_id = identity
                .and_then(|i| i.get("conversation_id"))
                .or_else(|| input.get("conversation_id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing conversation_id".into()))?;
            let sub_id = identity
                .and_then(|i| i.get("submission_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("default");
            let digest = input
                .get("request_digest")
                .or_else(|| args.get("request_digest"))
                .and_then(|v| v.as_str());
            if let Some(d) = digest {
                if let Ok(Some(req)) = store.get_kv(&format!("remote_sub_req_digest:{conv_id}:{d}")) {
                    return Ok(req);
                }
            }
            let found = store
                .get_kv(&format!("remote_sub_req:{conv_id}:{sub_id}"))
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(found.unwrap_or(Value::Null))
        }
        "accept_remote_submission_receipt" => {
            let input = args.get("input").unwrap_or(args);
            let receipt = input
                .get("receipt")
                .or_else(|| args.get("receipt"))
                .cloned()
                .ok_or(("invalid_argument", "missing 'receipt'".into()))?;
            let identity = receipt
                .get("identity")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let conv_id = identity
                .get("conversation_id")
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing receipt.identity.conversation_id".into()))?;
            let sub_id = identity
                .get("submission_id")
                .and_then(|v| v.as_str())
                .unwrap_or("default");
            let phase = receipt
                .get("phase")
                .and_then(|v| v.as_str())
                .unwrap_or("accepted");
            let digest = receipt.get("request_digest").cloned().unwrap_or(Value::Null);
            let intent = json!({
                "identity": identity,
                "request_digest": digest,
                "phase": phase
            });
            store
                .set_kv(&format!("remote_sub_receipt:{conv_id}:{sub_id}"), &receipt)
                .map_err(|e| ("storage_error", e.to_string()))?;
            store
                .set_kv(&format!("remote_sub_intent:{conv_id}"), &intent)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(intent)
        }
        "abandon_prepared_remote_submission" => {
            let input = args.get("input").unwrap_or(args);
            let identity = input
                .get("identity")
                .or_else(|| args.get("identity"))
                .cloned()
                .ok_or(("invalid_argument", "missing 'identity'".into()))?;
            let conv_id = identity
                .get("conversation_id")
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing identity.conversation_id".into()))?;
            let intent = json!({
                "identity": identity,
                "request_digest": Value::Null,
                "phase": "abandoned"
            });
            store
                .set_kv(&format!("remote_sub_intent:{conv_id}"), &Value::Null)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(intent)
        }
        "adopt_observed_remote_run" => {
            let input = args.get("input").unwrap_or(args);
            let receipt = input
                .get("receipt")
                .or_else(|| args.get("receipt"))
                .cloned()
                .unwrap_or_else(|| json!({}));
            let conv_id = receipt
                .get("identity")
                .and_then(|i| i.get("conversation_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("default");
            let sub_id = receipt
                .get("identity")
                .and_then(|i| i.get("submission_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("default");
            store
                .set_kv(&format!("observed_remote_run:{conv_id}:{sub_id}"), &receipt)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({
                "ok": true,
                "adopted": true,
                "receipt": receipt
            }))
        }
        "inspect_run_recovery" => {
            let conv_id = args
                .get("conversation_id")
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let run_id = args
                .get("expected_run_id")
                .or_else(|| args.get("run_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let assistant_message_id = args
                .get("assistant_message_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let intent = store
                .get_kv(&format!("remote_sub_intent:{conv_id}"))
                .ok()
                .flatten()
                .unwrap_or(Value::Null);
            let active_run_val = lookup_active_run(store, conv_id);
            let verification = store
                .get_kv(&format!("turn_verification:{conv_id}"))
                .ok()
                .flatten();
            let has_unverified_edits = verification
                .as_ref()
                .and_then(|v| v.get("all_modified_verified").and_then(|b| b.as_bool()))
                == Some(false);
            let durable_tasks: Vec<Value> = store
                .list_module_records::<Value>("durable_task")
                .unwrap_or_default()
                .into_iter()
                .filter(|t| {
                    let match_conv = conv_id.is_empty()
                        || t.get("session_id")
                            .and_then(|v| v.as_str())
                            .is_some_and(|s| s == conv_id || s.starts_with(&format!("{conv_id}:")));
                    let match_run = run_id.is_empty()
                        || t.get("task_id").and_then(|v| v.as_str()) == Some(run_id);
                    match_conv && match_run
                })
                .collect();
            let has_interrupted_task = durable_tasks.iter().any(|t| {
                matches!(
                    t.get("status").and_then(|v| v.as_str()),
                    Some("interrupted" | "running" | "failed")
                )
            });
            let recoverable = has_interrupted_task
                || active_run_val.is_some()
                || !intent.is_null()
                || has_unverified_edits;
            let state = if has_interrupted_task {
                "interrupted"
            } else if active_run_val.is_some() {
                "active"
            } else if has_unverified_edits {
                "unverified"
            } else if !intent.is_null() {
                "pending_submission"
            } else {
                "idle"
            };
            Ok(json!({
                "conversation_id": conv_id,
                "run_id": run_id,
                "assistant_message_id": assistant_message_id,
                "recoverable": recoverable,
                "state": state,
                "pending_intent": intent,
                "active_run": active_run_val.unwrap_or(Value::Null),
                "durable_tasks": durable_tasks,
                "turn_verification": verification.unwrap_or(Value::Null)
            }))
        }
        "accept_remote_run_context_usage" => {
            let input = args.get("input").unwrap_or(args);
            let source_host_id = input
                .get("source_host_id")
                .or_else(|| args.get("source_host_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("remote");
            let record = input
                .get("record")
                .or_else(|| args.get("record"))
                .cloned()
                .unwrap_or_else(|| json!({}));
            let conv_id = record
                .get("identity")
                .and_then(|i| i.get("conversation_id"))
                .or_else(|| input.get("conversation_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("default");
            let run_id = record
                .get("identity")
                .and_then(|i| i.get("run_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("default");
            store
                .set_kv(
                    &format!("remote_run_ctx_usage:{conv_id}:{run_id}"),
                    &json!({
                        "source_host_id": source_host_id,
                        "record": record,
                        "accepted_at_ms": now_ms()
                    }),
                )
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"ok": true, "accepted": true}))
        }
        "pending_sync_changes" => {
            let scope = args
                .get("scope")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty());
            let limit = args
                .get("limit")
                .and_then(|v| v.as_u64())
                .unwrap_or(100)
                .clamp(1, 1000) as usize;
            let changes = store
                .list_pending_sync_changes(scope, limit)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(serde_json::to_value(changes).unwrap_or_else(|_| json!([])))
        }
        "resolve_sync_change" => {
            let input = args.get("input").unwrap_or(args);
            let mut resolved_any = false;
            if let Some(ids) = input.get("ids").and_then(|v| v.as_array()) {
                for id_val in ids {
                    if let Some(id) = id_val.as_str().filter(|s| !s.trim().is_empty()) {
                        if store
                            .resolve_sync_change(id)
                            .map_err(|e| ("storage_error", e.to_string()))?
                        {
                            resolved_any = true;
                        }
                    }
                }
            } else if let Some(id) = input
                .get("id")
                .or_else(|| input.get("change_id"))
                .or_else(|| args.get("id"))
                .or_else(|| args.get("change_id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
            {
                resolved_any = store
                    .resolve_sync_change(id)
                    .map_err(|e| ("storage_error", e.to_string()))?;
            }
            Ok(json!(resolved_any))
        }
        "get_sync_cursor" => {
            let peer_id = args
                .get("peer_id")
                .or_else(|| args.get("replica_id"))
                .or_else(|| args.get("replicaID"))
                .or_else(|| args.get("scope"))
                .or_else(|| args.get("source_host_id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .unwrap_or("default");
            let cursor = store
                .get_sync_cursor(peer_id)
                .map_err(|e| ("storage_error", e.to_string()))?;
            match cursor {
                Some(raw) => {
                    let parsed_cursor = raw
                        .parse::<i64>()
                        .map(|n| json!(n))
                        .unwrap_or_else(|_| json!(raw));
                    Ok(json!({
                        "peer_id": peer_id,
                        "replica_id": peer_id,
                        "cursor": parsed_cursor,
                        "cursor_str": raw,
                        "updated_at_ms": now_ms()
                    }))
                }
                None => Ok(Value::Null),
            }
        }
        "save_sync_cursor" => {
            let input = args.get("input").unwrap_or(args);
            let peer_id = input
                .get("peer_id")
                .or_else(|| input.get("replica_id"))
                .or_else(|| input.get("replicaID"))
                .or_else(|| input.get("scope"))
                .or_else(|| args.get("peer_id"))
                .or_else(|| args.get("replica_id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .unwrap_or("default");
            let cursor_val = input
                .get("cursor")
                .or_else(|| args.get("cursor"))
                .cloned()
                .unwrap_or_else(|| json!("0"));
            let cursor_str = match &cursor_val {
                Value::String(s) => s.clone(),
                Value::Number(n) => n.to_string(),
                other => other.to_string(),
            };
            store
                .save_sync_cursor(peer_id, &cursor_str)
                .map_err(|e| ("storage_error", e.to_string()))?;
            let updated_at_ms = input
                .get("updated_at_ms")
                .or_else(|| input.get("updatedAtMs"))
                .and_then(|v| v.as_i64())
                .unwrap_or_else(now_ms);
            Ok(json!({
                "ok": true,
                "peer_id": peer_id,
                "replica_id": peer_id,
                "cursor": cursor_val,
                "cursor_str": cursor_str,
                "updated_at_ms": updated_at_ms
            }))
        }
        "apply_remote_sync_page" => {
            let input = args.get("input").unwrap_or(args);
            let peer_id = input
                .get("peer_id")
                .or_else(|| input.get("replica_id"))
                .or_else(|| input.get("replicaID"))
                .or_else(|| input.get("source_host_id"))
                .or_else(|| args.get("peer_id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .unwrap_or("remote");
            let next_cursor_val = input
                .get("next_cursor")
                .or_else(|| input.get("nextCursor"))
                .or_else(|| input.get("cursor"))
                .or_else(|| args.get("next_cursor"))
                .cloned();
            let items: Vec<Value> = input
                .get("changes")
                .or_else(|| input.get("items"))
                .or_else(|| input.get("records"))
                .or_else(|| input.get("events"))
                .or_else(|| args.get("changes"))
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();

            let mut applied = 0usize;
            let mut skipped = 0usize;
            let ts = now_ms();

            for item in items {
                let change_id = item
                    .get("id")
                    .or_else(|| item.get("change_id"))
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
                    .map(ToString::to_string)
                    .unwrap_or_else(|| sha256_hex(item.to_string().as_bytes()));
                let dedup_key = format!("sync_applied:{peer_id}:{change_id}");
                if store.get_kv(&dedup_key).ok().flatten().is_some() {
                    skipped += 1;
                    continue;
                }

                let scope = item
                    .get("scope")
                    .or_else(|| item.get("conversation_id"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("global");
                let entity_kind = item
                    .get("entity_kind")
                    .or_else(|| item.get("kind"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("module_record");
                let entity_id = item
                    .get("entity_id")
                    .or_else(|| item.get("key"))
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or(&change_id);
                let payload: Value = if let Some(p) = item.get("payload") {
                    p.clone()
                } else if let Some(raw) = item.get("payload_json").and_then(|v| v.as_str()) {
                    serde_json::from_str(raw).unwrap_or_else(|_| json!({"raw": raw}))
                } else {
                    item.clone()
                };

                let apply_res: Result<(), String> = match entity_kind {
                    "session" | "conversation" => {
                        let sess = Session {
                            id: payload
                                .get("id")
                                .and_then(|v| v.as_str())
                                .unwrap_or(entity_id)
                                .to_string(),
                            title: payload
                                .get("title")
                                .and_then(|v| v.as_str())
                                .unwrap_or("Synced conversation")
                                .to_string(),
                            workspace_root: payload
                                .get("workspace_root")
                                .and_then(|v| v.as_str())
                                .map(ToString::to_string)
                                .unwrap_or_else(|| suite.workspace_root.display().to_string()),
                            model_connection: payload
                                .get("model_connection")
                                .and_then(|v| v.as_str())
                                .map(ToString::to_string),
                            model_id: payload
                                .get("model_id")
                                .and_then(|v| v.as_str())
                                .map(ToString::to_string),
                            created_at_ms: payload
                                .get("created_at_ms")
                                .and_then(|v| v.as_i64())
                                .unwrap_or(ts),
                            updated_at_ms: payload
                                .get("updated_at_ms")
                                .and_then(|v| v.as_i64())
                                .unwrap_or(ts),
                        };
                        store.upsert_session(&sess).map_err(|e| e.to_string())
                    }
                    "event" => {
                        let ev_kind = payload
                            .get("kind")
                            .or_else(|| item.get("event_kind"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("sync_event");
                        let ev_payload = payload.get("payload").cloned().unwrap_or(payload.clone());
                        store
                            .append_event(scope, ev_kind, ev_payload, 50_000)
                            .map(|_| ())
                            .map_err(|e| e.to_string())
                    }
                    "memory" => {
                        let mem = MemoryItem {
                            id: payload
                                .get("id")
                                .and_then(|v| v.as_str())
                                .unwrap_or(entity_id)
                                .to_string(),
                            namespace: payload
                                .get("namespace")
                                .and_then(|v| v.as_str())
                                .unwrap_or("default")
                                .to_string(),
                            title: payload
                                .get("title")
                                .and_then(|v| v.as_str())
                                .unwrap_or("Synced memory")
                                .to_string(),
                            content: payload
                                .get("content")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string(),
                            tags: payload
                                .get("tags")
                                .and_then(|v| v.as_array())
                                .map(|a| {
                                    a.iter()
                                        .filter_map(|v| v.as_str().map(ToString::to_string))
                                        .collect()
                                })
                                .unwrap_or_default(),
                            created_at_ms: payload
                                .get("created_at_ms")
                                .and_then(|v| v.as_i64())
                                .unwrap_or(ts),
                            updated_at_ms: payload
                                .get("updated_at_ms")
                                .and_then(|v| v.as_i64())
                                .unwrap_or(ts),
                        };
                        store.upsert_memory(&mem).map_err(|e| e.to_string())
                    }
                    "kv" => store.set_kv(entity_id, &payload).map_err(|e| e.to_string()),
                    other_kind => store
                        .put_module_record(other_kind, entity_id, &payload)
                        .map_err(|e| e.to_string()),
                };

                if apply_res.is_ok() {
                    let _ = store.record_sync_change(&SyncChangeRecord {
                        id: change_id.clone(),
                        scope: scope.to_string(),
                        entity_kind: entity_kind.to_string(),
                        entity_id: entity_id.to_string(),
                        payload_json: payload.to_string(),
                        created_at_ms: ts,
                        resolved: true,
                    });
                    let _ = store.set_kv(&dedup_key, &json!({"applied_at_ms": ts}));
                    applied += 1;
                } else {
                    skipped += 1;
                }
            }

            let cursor_out = if let Some(ref cv) = next_cursor_val {
                let c_str = match cv {
                    Value::String(s) => s.clone(),
                    Value::Number(n) => n.to_string(),
                    other => other.to_string(),
                };
                store
                    .save_sync_cursor(peer_id, &c_str)
                    .map_err(|e| ("storage_error", e.to_string()))?;
                cv.clone()
            } else {
                store
                    .get_sync_cursor(peer_id)
                    .ok()
                    .flatten()
                    .map(|s| s.parse::<i64>().map(|n| json!(n)).unwrap_or_else(|_| json!(s)))
                    .unwrap_or_else(|| json!(0))
            };

            Ok(json!({
                "applied": applied,
                "skipped": skipped,
                "cursor": cursor_out,
                "peer_id": peer_id
            }))
        }
        "peek_agent_command" => Ok(Value::Null),
        "load_run_identity" => {
            let run_id = args
                .get("run_id")
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str());
            let conv_id = args.get("conversation_id").and_then(|v| v.as_str());
            if let Some(rid) = run_id {
                if let Ok(Some(saved)) = store.get_kv(&format!("run_identity:{rid}")) {
                    return Ok(saved);
                }
            }
            let host_id = store
                .get_kv("host_id")
                .ok()
                .flatten()
                .and_then(|v| v.as_str().map(ToString::to_string))
                .unwrap_or_else(|| "local-host".into());
            Ok(json!({
                "run_id": run_id.or(conv_id),
                "conversation_id": conv_id,
                "host_id": host_id,
                "execution_backend": "nola-v2"
            }))
        }
        "upsert_permission_grant" => {
            let input = args.get("input").unwrap_or(args);
            let conv_id = input
                .get("conversation_id")
                .or_else(|| args.get("conversation_id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .unwrap_or("global");
            let grant_src = input.get("grant").unwrap_or(input);
            let grant_id = grant_src
                .get("id")
                .or_else(|| grant_src.get("grant_id"))
                .or_else(|| input.get("grant_id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(new_id);
            let ts = now_ms();
            let mut grant = grant_src.clone();
            if let Some(obj) = grant.as_object_mut() {
                obj.insert("id".into(), json!(grant_id));
                obj.insert("grant_id".into(), json!(grant_id));
                obj.insert("conversation_id".into(), json!(conv_id));
                if conv_id != "global" {
                    obj.entry("session_id".to_string())
                        .or_insert_with(|| json!(conv_id));
                }
                obj.entry("active".to_string()).or_insert_with(|| json!(true));
                obj.insert("updated_at_ms".into(), json!(ts));
                obj.entry("created_at_ms".to_string()).or_insert_with(|| json!(ts));
            }
            let key = format!("conv_grants:{conv_id}");
            let mut grants = store
                .get_kv(&key)
                .ok()
                .flatten()
                .and_then(|v| v.as_array().cloned())
                .unwrap_or_default();
            if let Some(existing) = grants.iter_mut().find(|g| {
                g.get("id")
                    .or_else(|| g.get("grant_id"))
                    .and_then(|v| v.as_str())
                    == Some(grant_id.as_str())
            }) {
                *existing = grant.clone();
            } else {
                grants.push(grant.clone());
            }
            store
                .set_kv(&key, &Value::Array(grants.clone()))
                .map_err(|e| ("storage_error", e.to_string()))?;
            let _ = store.put_module_record("permission_grant", &grant_id, &grant);
            Ok(json!({
                "ok": true,
                "conversation_id": conv_id,
                "grant_id": grant_id,
                "grant": grant,
                "grants": grants
            }))
        }
        "revoke_permission_grant" => {
            let input = args.get("input").unwrap_or(args);
            let conv_id = input
                .get("conversation_id")
                .or_else(|| args.get("conversation_id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .unwrap_or("global");
            let grant_id = input
                .get("grant_id")
                .or_else(|| input.get("id"))
                .or_else(|| args.get("grant_id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str());
            let tool = input.get("tool").and_then(|v| v.as_str());
            let key = format!("conv_grants:{conv_id}");
            let mut grants = store
                .get_kv(&key)
                .ok()
                .flatten()
                .and_then(|v| v.as_array().cloned())
                .unwrap_or_default();
            let before = grants.len();
            grants.retain(|g| {
                if let Some(gid) = grant_id {
                    g.get("id")
                        .or_else(|| g.get("grant_id"))
                        .and_then(|v| v.as_str())
                        != Some(gid)
                } else if let Some(t) = tool {
                    g.get("tool").and_then(|v| v.as_str()) != Some(t)
                } else {
                    true
                }
            });
            let mut revoked = grants.len() < before;
            store
                .set_kv(&key, &Value::Array(grants.clone()))
                .map_err(|e| ("storage_error", e.to_string()))?;
            if let Some(gid) = grant_id {
                if store.delete_module_record("permission_grant", gid).unwrap_or(false) {
                    revoked = true;
                }
            }
            Ok(json!({
                "ok": true,
                "conversation_id": conv_id,
                "grant_id": grant_id,
                "revoked": revoked,
                "grants": grants
            }))
        }
        "import_conversation_transcript" => {
            let input = args.get("input").unwrap_or(args);
            let conv_id = input
                .get("conversation_id")
                .or_else(|| input.get("id"))
                .or_else(|| args.get("conversation_id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(new_id);
            let title = input
                .get("title")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .unwrap_or("Imported Conversation");
            let ts = now_ms();
            let sess = match store.get_session(&conv_id).ok().flatten() {
                Some(mut existing) => {
                    existing.updated_at_ms = ts;
                    existing
                }
                None => Session {
                    id: conv_id.clone(),
                    title: title.to_string(),
                    workspace_root: input
                        .get("workspace_root")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string)
                        .unwrap_or_else(|| suite.workspace_root.display().to_string()),
                    model_connection: None,
                    model_id: None,
                    created_at_ms: ts,
                    updated_at_ms: ts,
                },
            };
            store
                .upsert_session(&sess)
                .map_err(|e| ("storage_error", e.to_string()))?;
            let entries = input
                .get("messages")
                .or_else(|| input.get("transcript"))
                .or_else(|| input.get("entries"))
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            let mut imported_count = 0usize;
            for entry in entries {
                let role = entry
                    .get("role")
                    .and_then(|v| v.as_str())
                    .unwrap_or("user");
                let content = entry
                    .get("content")
                    .or_else(|| entry.get("text"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let kind = if role.eq_ignore_ascii_case("assistant") {
                    "assistant_message"
                } else {
                    "user_message"
                };
                let _ = store.append_event(
                    &conv_id,
                    kind,
                    json!({
                        "role": role,
                        "content": content,
                        "imported": true
                    }),
                    50_000,
                );
                imported_count += 1;
            }
            let conv_json = crate::web_api::session_to_conversation_json(store, &sess);
            Ok(json!({
                "ok": true,
                "conversation_id": conv_id,
                "imported_count": imported_count,
                "conversation": conv_json
            }))
        }
        "admit_host_output" => {
            let input = args.get("input").unwrap_or(args);
            let output_id = input
                .get("output_id")
                .or_else(|| input.get("id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(new_id);
            let conv_id = input
                .get("conversation_id")
                .or_else(|| args.get("conversation_id"))
                .and_then(|v| v.as_str());
            let run_id = input
                .get("run_id")
                .or_else(|| args.get("run_id"))
                .and_then(|v| v.as_str());
            let ts = now_ms();
            let rec = json!({
                "output_id": output_id,
                "conversation_id": conv_id,
                "run_id": run_id,
                "output": input.get("output").or_else(|| input.get("content")).cloned().unwrap_or(Value::Null),
                "admitted_at_ms": ts
            });
            store
                .set_kv(&format!("host_output:{output_id}"), &rec)
                .map_err(|e| ("storage_error", e.to_string()))?;
            if let Some(cid) = conv_id {
                let _ = store.append_event(cid, "host_output_admitted", rec.clone(), 50_000);
            }
            Ok(json!({
                "ok": true,
                "admitted": true,
                "output_id": output_id,
                "record": rec
            }))
        }
        "load_task_guidance_receipt" => {
            let key_id = args
                .get("receipt_id")
                .or_else(|| args.get("task_id"))
                .or_else(|| args.get("conversation_id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing receipt_id/task_id/conversation_id".into()))?;
            let found = store
                .get_kv(&format!("task_guidance_receipt:{key_id}"))
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({
                "id": key_id,
                "found": found.is_some(),
                "receipt": found.unwrap_or(Value::Null)
            }))
        }
        "submit_task_guidance" => {
            let input = args.get("input").unwrap_or(args);
            let receipt_id = input
                .get("receipt_id")
                .or_else(|| input.get("id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(new_id);
            let task_id = input
                .get("task_id")
                .or_else(|| args.get("task_id"))
                .and_then(|v| v.as_str());
            let conv_id = input
                .get("conversation_id")
                .or_else(|| args.get("conversation_id"))
                .and_then(|v| v.as_str());
            let guidance = input
                .get("guidance")
                .or_else(|| input.get("content"))
                .or_else(|| input.get("message"))
                .cloned()
                .unwrap_or(Value::Null);
            let ts = now_ms();
            let receipt = json!({
                "receipt_id": receipt_id,
                "task_id": task_id,
                "conversation_id": conv_id,
                "guidance": guidance,
                "status": "accepted",
                "submitted_at_ms": ts
            });
            store
                .set_kv(&format!("task_guidance_receipt:{receipt_id}"), &receipt)
                .map_err(|e| ("storage_error", e.to_string()))?;
            if let Some(tid) = task_id {
                let _ = store.set_kv(&format!("task_guidance_receipt:{tid}"), &receipt);
            }
            if let Some(cid) = conv_id {
                let _ = store.set_kv(&format!("task_guidance_receipt:{cid}"), &receipt);
            }
            Ok(json!({
                "ok": true,
                "receipt_id": receipt_id,
                "receipt": receipt
            }))
        }
        "request_owned_cancellation" => {
            let input = args.get("input").unwrap_or(args);
            let target_id = input
                .get("run_id")
                .or_else(|| input.get("task_id"))
                .or_else(|| input.get("conversation_id"))
                .or_else(|| input.get("id"))
                .or_else(|| args.get("run_id"))
                .or_else(|| args.get("conversation_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("default");
            let ts = now_ms();
            let rec = json!({
                "target_id": target_id,
                "reason": input.get("reason").cloned().unwrap_or(Value::Null),
                "requested_at_ms": ts
            });
            store
                .set_kv(&format!("cancel_requested:{target_id}"), &rec)
                .map_err(|e| ("storage_error", e.to_string()))?;
            let _ = store.set_kv(&format!("owned_cancellation:{target_id}"), &rec);
            Ok(json!({
                "ok": true,
                "cancelled": true,
                "requested": true,
                "target_id": target_id,
                "requested_at_ms": ts
            }))
        }
        "current_run" => {
            let conv_id = args
                .get("conversation_id")
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let found = if !conv_id.is_empty() {
                store
                    .get_kv(&format!("current_run:{conv_id}"))
                    .ok()
                    .flatten()
                    .filter(|v| !v.is_null())
            } else {
                None
            };
            Ok(json!({
                "conversation_id": conv_id,
                "active": found.is_some(),
                "run": found.unwrap_or(Value::Null)
            }))
        }
        "query_canonical_history" => {
            let conv_id = args
                .get("conversation_id")
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'conversation_id'".into()))?;
            let after_seq = args.get("after_seq").and_then(|v| v.as_i64());
            let before_seq = args.get("before_seq").and_then(|v| v.as_i64());
            let offset = args.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            let limit = args
                .get("limit")
                .and_then(|v| v.as_u64())
                .unwrap_or(200)
                .clamp(1, 2000) as usize;
            let (msgs, events) = crate::web_api::reconstruct_v2_messages(store, conv_id);
            let latest_seq = events
                .last()
                .and_then(|e| e.get("seq").and_then(|v| v.as_i64()))
                .unwrap_or(0);
            let filtered: Vec<Value> = msgs
                .into_iter()
                .filter(|m| {
                    let sq = m.get("seq").and_then(|v| v.as_i64()).unwrap_or(0);
                    after_seq.is_none_or(|a| sq > a) && before_seq.is_none_or(|b| sq < b)
                })
                .collect();
            let total = filtered.len();
            let start = offset.min(total);
            let end = (start + limit).min(total);
            let page = filtered[start..end].to_vec();
            Ok(json!({
                "conversation_id": conv_id,
                "messages": page,
                "total": total,
                "has_more": end < total,
                "latest_seq": latest_seq
            }))
        }
        "local_run_context_usage" => {
            let conv_id = args
                .get("conversation_id")
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'conversation_id'".into()))?;
            let (_, rev) = read_permission_state(store, Some(conv_id));
            let mut usage = build_context_usage_json(store, conv_id, rev);
            if let (Some(obj), Some(run_id)) = (
                usage.as_object_mut(),
                args.get("run_id").and_then(|v| v.as_str()),
            ) {
                obj.insert("run_id".into(), json!(run_id));
            }
            Ok(usage)
        }
        "read_conversation_tool_sources" => {
            let conv_id = args
                .get("conversation_id")
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'conversation_id'".into()))?;
            let mcp_servers = store.list_mcp_servers().unwrap_or_default();
            let data_sources: Vec<Value> = suite
                .list_records::<Value>("data_source")
                .unwrap_or_default()
                .into_iter()
                .map(|d| crate::web_api::normalize_data_source_record(&d))
                .collect();
            let bound = store
                .get_kv(&format!("conv_data_sources:{conv_id}"))
                .ok()
                .flatten()
                .unwrap_or_else(|| json!([]));
            Ok(json!({
                "conversation_id": conv_id,
                "mcp_servers": mcp_servers,
                "data_sources": data_sources,
                "bound_sources": bound
            }))
        }
        "begin_archive_restore" => {
            let input = args.get("input").unwrap_or(args);
            let restore_id = input
                .get("restore_id")
                .or_else(|| input.get("id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(new_id);
            let ts = now_ms();
            let rec = json!({
                "restore_id": restore_id,
                "archive_id": input.get("archive_id").cloned().unwrap_or(Value::Null),
                "status": "in_progress",
                "started_at_ms": ts
            });
            store
                .set_kv(&format!("archive_restore:{restore_id}"), &rec)
                .map_err(|e| ("storage_error", e.to_string()))?;
            let _ = store.set_kv("archive_restore_active", &json!(restore_id));
            Ok(json!({
                "ok": true,
                "restore_id": restore_id,
                "status": "in_progress",
                "restore": rec
            }))
        }
        "commit_archive_restore" => {
            let input = args.get("input").unwrap_or(args);
            let restore_id = input
                .get("restore_id")
                .or_else(|| input.get("id"))
                .or_else(|| args.get("restore_id"))
                .and_then(|v| v.as_str())
                .map(ToString::to_string)
                .or_else(|| {
                    store
                        .get_kv("archive_restore_active")
                        .ok()
                        .flatten()
                        .and_then(|v| v.as_str().map(ToString::to_string))
                })
                .ok_or(("invalid_argument", "missing 'restore_id'".into()))?;
            let cur_rev = store
                .get_kv("archive_directory_revision")
                .ok()
                .flatten()
                .and_then(|v| v.as_u64())
                .unwrap_or(1);
            let next_rev = cur_rev + 1;
            let ts = now_ms();
            let rec = json!({
                "restore_id": restore_id,
                "status": "committed",
                "revision": next_rev,
                "committed_at_ms": ts
            });
            store
                .set_kv(&format!("archive_restore:{restore_id}"), &rec)
                .map_err(|e| ("storage_error", e.to_string()))?;
            store
                .set_kv("archive_directory_revision", &json!(next_rev))
                .map_err(|e| ("storage_error", e.to_string()))?;
            let _ = store.set_kv("archive_restore_active", &Value::Null);
            Ok(json!({
                "ok": true,
                "restore_id": restore_id,
                "status": "committed",
                "revision": next_rev
            }))
        }
        "archive_directory_revision" => {
            let rev = store
                .get_kv("archive_directory_revision")
                .ok()
                .flatten()
                .and_then(|v| v.as_u64())
                .unwrap_or(1);
            let epoch = crate::web_api::ensure_source_store_epoch(store);
            Ok(json!({
                "revision": rev,
                "source_store_epoch": epoch
            }))
        }
        "stage_archive_manifest" => {
            let input = args.get("input").unwrap_or(args);
            let manifest_id = input
                .get("manifest_id")
                .or_else(|| input.get("id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(new_id);
            let ts = now_ms();
            let rec = json!({
                "manifest_id": manifest_id,
                "manifest": input.get("manifest").or_else(|| input.get("entries")).cloned().unwrap_or_else(|| input.clone()),
                "staged_at_ms": ts
            });
            store
                .set_kv(&format!("archive_manifest:{manifest_id}"), &rec)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({
                "ok": true,
                "staged": true,
                "manifest_id": manifest_id,
                "manifest": rec
            }))
        }
        "begin_content_upload" => {
            let input = args.get("input").unwrap_or(args);
            let upload_id = input
                .get("upload_id")
                .or_else(|| input.get("id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(new_id);
            let ts = now_ms();
            let rec = json!({
                "upload_id": upload_id,
                "conversation_id": input.get("conversation_id").cloned().unwrap_or(Value::Null),
                "filename": input.get("filename").or_else(|| input.get("name")).cloned().unwrap_or(Value::Null),
                "mime_type": input.get("mime_type").or_else(|| input.get("content_type")).cloned().unwrap_or(Value::Null),
                "total_bytes": input.get("total_bytes").and_then(|v| v.as_u64()).unwrap_or(0),
                "bytes_received": 0,
                "chunk_count": 0,
                "sealed": false,
                "created_at_ms": ts,
                "updated_at_ms": ts
            });
            store
                .set_kv(&format!("content_upload:{upload_id}"), &rec)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({
                "ok": true,
                "upload_id": upload_id,
                "status": "open",
                "bytes_received": 0,
                "upload": rec
            }))
        }
        "append_content_chunk" => {
            let input = args.get("input").unwrap_or(args);
            let upload_id = input
                .get("upload_id")
                .or_else(|| args.get("upload_id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'upload_id'".into()))?;
            let mut upload = store
                .get_kv(&format!("content_upload:{upload_id}"))
                .map_err(|e| ("storage_error", e.to_string()))?
                .ok_or(("not_found", format!("content upload '{upload_id}' not found")))?;
            if upload.get("sealed").and_then(|v| v.as_bool()) == Some(true) {
                return Err(("invalid_state", format!("content upload '{upload_id}' is already sealed")));
            }
            let cur_chunks = upload
                .get("chunk_count")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let chunk_index = input
                .get("chunk_index")
                .or_else(|| input.get("index"))
                .and_then(|v| v.as_u64())
                .unwrap_or(cur_chunks);
            let data = input
                .get("data")
                .or_else(|| input.get("chunk_base64"))
                .or_else(|| input.get("chunk"))
                .cloned()
                .unwrap_or_else(|| json!(""));
            let chunk_len = input
                .get("byte_length")
                .and_then(|v| v.as_u64())
                .unwrap_or_else(|| {
                    data.as_str().map(|s| s.len() as u64).unwrap_or(0)
                });
            let chunk_rec = json!({
                "upload_id": upload_id,
                "chunk_index": chunk_index,
                "data": data,
                "byte_length": chunk_len,
                "appended_at_ms": now_ms()
            });
            store
                .set_kv(&format!("content_upload_chunk:{upload_id}:{chunk_index}"), &chunk_rec)
                .map_err(|e| ("storage_error", e.to_string()))?;
            let next_bytes = upload
                .get("bytes_received")
                .and_then(|v| v.as_u64())
                .unwrap_or(0)
                .saturating_add(chunk_len);
            let next_chunks = cur_chunks.max(chunk_index + 1);
            if let Some(obj) = upload.as_object_mut() {
                obj.insert("bytes_received".into(), json!(next_bytes));
                obj.insert("chunk_count".into(), json!(next_chunks));
                obj.insert("updated_at_ms".into(), json!(now_ms()));
            }
            store
                .set_kv(&format!("content_upload:{upload_id}"), &upload)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({
                "ok": true,
                "upload_id": upload_id,
                "chunk_index": chunk_index,
                "bytes_received": next_bytes,
                "chunk_count": next_chunks
            }))
        }
        "seal_content_upload" => {
            let input = args.get("input").unwrap_or(args);
            let upload_id = input
                .get("upload_id")
                .or_else(|| args.get("upload_id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'upload_id'".into()))?;
            let mut upload = store
                .get_kv(&format!("content_upload:{upload_id}"))
                .map_err(|e| ("storage_error", e.to_string()))?
                .ok_or(("not_found", format!("content upload '{upload_id}' not found")))?;
            let ts = now_ms();
            if let Some(obj) = upload.as_object_mut() {
                obj.insert("sealed".into(), json!(true));
                obj.insert("sealed_at_ms".into(), json!(ts));
                obj.insert("updated_at_ms".into(), json!(ts));
            }
            store
                .set_kv(&format!("content_upload:{upload_id}"), &upload)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({
                "ok": true,
                "upload_id": upload_id,
                "sealed": true,
                "bytes_received": upload.get("bytes_received").cloned().unwrap_or(json!(0)),
                "chunk_count": upload.get("chunk_count").cloned().unwrap_or(json!(0)),
                "upload": upload
            }))
        }
        "content_upload_progress" => {
            let upload_id = args
                .get("upload_id")
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'upload_id'".into()))?;
            let found = store
                .get_kv(&format!("content_upload:{upload_id}"))
                .map_err(|e| ("storage_error", e.to_string()))?;
            match found {
                Some(u) => Ok(json!({
                    "upload_id": upload_id,
                    "found": true,
                    "sealed": u.get("sealed").and_then(|v| v.as_bool()).unwrap_or(false),
                    "bytes_received": u.get("bytes_received").cloned().unwrap_or(json!(0)),
                    "total_bytes": u.get("total_bytes").cloned().unwrap_or(json!(0)),
                    "chunk_count": u.get("chunk_count").cloned().unwrap_or(json!(0)),
                    "upload": u
                })),
                None => Ok(json!({
                    "upload_id": upload_id,
                    "found": false,
                    "sealed": false,
                    "bytes_received": 0,
                    "total_bytes": 0,
                    "chunk_count": 0
                })),
            }
        }
        "read_content_upload_chunk" => {
            let upload_id = args
                .get("upload_id")
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'upload_id'".into()))?;
            let chunk_index = args
                .get("chunk_index")
                .or_else(|| args.get("index"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let found = store
                .get_kv(&format!("content_upload_chunk:{upload_id}:{chunk_index}"))
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({
                "upload_id": upload_id,
                "chunk_index": chunk_index,
                "found": found.is_some(),
                "chunk": found.unwrap_or(Value::Null)
            }))
        }
        "runtime_task" => {
            let (op, payload) = extract_sub_command(args);
            let input = payload.get("input").unwrap_or(payload);
            match op {
                "list" => {
                    let conv_filter = input
                        .get("conversation_id")
                        .or_else(|| args.get("conversation_id"))
                        .and_then(|v| v.as_str());
                    let mut tasks: Vec<Value> = suite
                        .list_records("runtime_task")
                        .map_err(|e| ("storage_error", e.to_string()))?;
                    if let Some(cid) = conv_filter {
                        tasks.retain(|t| {
                            t.get("conversation_id").and_then(|v| v.as_str()) == Some(cid)
                        });
                    }
                    Ok(json!({"tasks": tasks, "items": tasks}))
                }
                "get" | "status" => {
                    let id = input
                        .get("task_id")
                        .or_else(|| input.get("id"))
                        .or_else(|| args.get("id"))
                        .and_then(|v| v.as_str())
                        .ok_or(("invalid_argument", "missing 'task_id'".into()))?;
                    let found = suite
                        .get_record::<Value>("runtime_task", id)
                        .map_err(|e| ("storage_error", e.to_string()))?;
                    Ok(json!({"task_id": id, "found": found.is_some(), "task": found.unwrap_or(Value::Null)}))
                }
                "create" | "upsert" | "start" | "update" | "heartbeat" | "complete" | "cancel" => {
                    let id = input
                        .get("task_id")
                        .or_else(|| input.get("id"))
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.trim().is_empty())
                        .map(ToString::to_string)
                        .unwrap_or_else(new_id);
                    let mut rec = suite
                        .get_record::<Value>("runtime_task", &id)
                        .ok()
                        .flatten()
                        .unwrap_or_else(|| json!({"id": id, "task_id": id, "created_at_ms": now_ms()}));
                    if let (Some(dst), Some(src)) = (rec.as_object_mut(), input.as_object()) {
                        for (k, v) in src {
                            if !matches!(k.as_str(), "type" | "action" | "operation" | "command") {
                                dst.insert(k.clone(), v.clone());
                            }
                        }
                        dst.insert("id".into(), json!(id));
                        dst.insert("task_id".into(), json!(id));
                        let status = match op {
                            "complete" => "completed",
                            "cancel" => "cancelled",
                            "start" => "running",
                            _ => dst
                                .get("status")
                                .and_then(|v| v.as_str())
                                .unwrap_or("running"),
                        }
                        .to_string();
                        dst.insert("status".into(), json!(status));
                        dst.insert("updated_at_ms".into(), json!(now_ms()));
                    }
                    suite
                        .upsert_record("runtime_task", &id, &rec)
                        .map_err(|e| ("storage_error", e.to_string()))?;
                    Ok(json!({"ok": true, "task_id": id, "task": rec}))
                }
                "delete" => {
                    let id = input
                        .get("task_id")
                        .or_else(|| input.get("id"))
                        .or_else(|| args.get("id"))
                        .and_then(|v| v.as_str())
                        .ok_or(("invalid_argument", "missing 'task_id'".into()))?;
                    let deleted = suite
                        .delete_record("runtime_task", id)
                        .map_err(|e| ("storage_error", e.to_string()))?;
                    Ok(json!({"ok": true, "deleted": deleted, "task_id": id}))
                }
                other => Err((
                    "unsupported_command",
                    format!("unsupported runtime_task operation '{other}'"),
                )),
            }
        }
        "put_conversation_pdf_resource" => {
            let input = args.get("input").unwrap_or(args);
            let conv_id = input
                .get("conversation_id")
                .or_else(|| args.get("conversation_id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .ok_or(("invalid_argument", "missing 'conversation_id'".into()))?;
            let resource_id = input
                .get("resource_id")
                .or_else(|| input.get("id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(new_id);
            let filename = input
                .get("filename")
                .or_else(|| input.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("document.pdf")
                .to_string();
            let data_base64 = input
                .get("data_base64")
                .or_else(|| input.get("content_base64"))
                .or_else(|| input.get("data"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let extracted_text = input
                .get("extracted_text")
                .or_else(|| input.get("text"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let page_count = input.get("page_count").and_then(|v| v.as_u64()).unwrap_or(1);
            let digest = input
                .get("sha256")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(|| sha256_hex(data_base64.as_bytes()));
            let size_bytes = input
                .get("size_bytes")
                .and_then(|v| v.as_u64())
                .unwrap_or(data_base64.len() as u64);
            let ts = now_ms();
            let record = json!({
                "resource_id": resource_id,
                "id": resource_id,
                "conversation_id": conv_id,
                "kind": "pdf",
                "mime_type": input.get("mime_type").and_then(|v| v.as_str()).unwrap_or("application/pdf"),
                "filename": filename,
                "page_count": page_count,
                "size_bytes": size_bytes,
                "sha256": digest,
                "data_base64": data_base64,
                "extracted_text": extracted_text,
                "created_at_ms": ts,
                "updated_at_ms": ts
            });
            let store_key = format!("{conv_id}:{resource_id}");
            store
                .put_module_record("conversation_resource", &store_key, &record)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({
                "ok": true,
                "conversation_id": conv_id,
                "resource_id": resource_id,
                "sha256": digest,
                "size_bytes": size_bytes,
                "resource": record
            }))
        }
        "read_conversation_resource" => {
            let input = args.get("input").unwrap_or(args);
            let conv_id = input
                .get("conversation_id")
                .or_else(|| args.get("conversation_id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty());
            let resource_id = input
                .get("resource_id")
                .or_else(|| input.get("id"))
                .or_else(|| args.get("resource_id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .ok_or(("invalid_argument", "missing 'resource_id'".into()))?;
            let found = if let Some(cid) = conv_id {
                store
                    .get_module_record::<Value>("conversation_resource", &format!("{cid}:{resource_id}"))
                    .map_err(|e| ("storage_error", e.to_string()))?
            } else {
                store
                    .list_module_records::<Value>("conversation_resource")
                    .map_err(|e| ("storage_error", e.to_string()))?
                    .into_iter()
                    .find(|r| {
                        r.get("resource_id")
                            .or_else(|| r.get("id"))
                            .and_then(|v| v.as_str())
                            == Some(resource_id)
                    })
            };
            match found {
                Some(rec) => Ok(json!({
                    "found": true,
                    "resource_id": resource_id,
                    "resource": rec
                })),
                None => Err(("not_found", format!("conversation resource '{resource_id}' not found"))),
            }
        }
        "list_conversation_resources" => {
            let conv_id = args
                .get("conversation_id")
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty());
            let include_content = args
                .get("include_content")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let all = store
                .list_module_records::<Value>("conversation_resource")
                .map_err(|e| ("storage_error", e.to_string()))?;
            let resources: Vec<Value> = all
                .into_iter()
                .filter(|r| {
                    conv_id.is_none_or(|cid| {
                        r.get("conversation_id").and_then(|v| v.as_str()) == Some(cid)
                    })
                })
                .map(|mut r| {
                    if !include_content {
                        if let Some(obj) = r.as_object_mut() {
                            obj.remove("data_base64");
                        }
                    }
                    r
                })
                .collect();
            Ok(json!({
                "conversation_id": conv_id,
                "resources": resources
            }))
        }
        "delete_conversation_resource" => {
            let input = args.get("input").unwrap_or(args);
            let conv_id = input
                .get("conversation_id")
                .or_else(|| args.get("conversation_id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .ok_or(("invalid_argument", "missing 'conversation_id'".into()))?;
            let resource_id = input
                .get("resource_id")
                .or_else(|| input.get("id"))
                .or_else(|| args.get("resource_id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .ok_or(("invalid_argument", "missing 'resource_id'".into()))?;
            let deleted = store
                .delete_module_record("conversation_resource", &format!("{conv_id}:{resource_id}"))
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({
                "ok": true,
                "conversation_id": conv_id,
                "resource_id": resource_id,
                "deleted": deleted
            }))
        }
        "admit_conversation_user_message" => {
            let input = args.get("input").unwrap_or(args);
            let conv_id = input
                .get("conversation_id")
                .or_else(|| args.get("conversation_id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .ok_or(("invalid_argument", "missing 'conversation_id'".into()))?;
            let message_id = input
                .get("message_id")
                .or_else(|| input.get("id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(new_id);
            let adm_key = format!("msg_admission:{conv_id}:{message_id}");
            if let Ok(Some(existing)) = store.get_kv(&adm_key) {
                return Ok(existing);
            }
            let ts = input
                .get("created_at_ms")
                .and_then(|v| v.as_i64())
                .unwrap_or_else(now_ms);
            if store.get_session(conv_id).ok().flatten().is_none() {
                let sess = Session {
                    id: conv_id.to_string(),
                    title: "New Chat".into(),
                    workspace_root: suite.workspace_root.display().to_string(),
                    model_connection: None,
                    model_id: None,
                    created_at_ms: ts,
                    updated_at_ms: ts,
                };
                store
                    .upsert_session(&sess)
                    .map_err(|e| ("storage_error", e.to_string()))?;
            }
            let content = input
                .get("content")
                .or_else(|| input.get("text"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let ev_payload = json!({
                "id": message_id,
                "message_id": message_id,
                "role": "user",
                "content": content,
                "attachments": input.get("attachments").cloned().unwrap_or_else(|| json!([])),
                "created_at_ms": ts
            });
            let ev = store
                .append_event(conv_id, "user_message", ev_payload, 50_000)
                .map_err(|e| ("storage_error", e.to_string()))?;
            let receipt = json!({
                "ok": true,
                "admitted": true,
                "conversation_id": conv_id,
                "message_id": message_id,
                "seq": ev.seq,
                "created_at_ms": ts
            });
            store
                .set_kv(&adm_key, &receipt)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(receipt)
        }
        "begin_queued_user_turn" => {
            let input = args.get("input").unwrap_or(args);
            let conv_id = input
                .get("conversation_id")
                .or_else(|| args.get("conversation_id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .ok_or(("invalid_argument", "missing 'conversation_id'".into()))?;
            let run_id = input
                .get("command_id")
                .or_else(|| input.get("run_id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(new_id);
            let assistant_message_id = input
                .get("assistant_message_id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(|| format!("{run_id}:assistant"));
            let core_config = input
                .get("core_config")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let digest = sha256_hex(canonical_json_value(&core_config).to_string().as_bytes());
            let ts = input
                .get("now_ms")
                .and_then(|v| v.as_i64())
                .unwrap_or_else(now_ms);
            let (profile, rev) = read_permission_state(store, Some(conv_id));
            let grants = load_durable_permission_grants(store, Some(conv_id));
            let pending = load_pending_permission_requests(store, Some(conv_id));
            let host_invocation = core_config
                .pointer("/turn_identity/host_invocation")
                .cloned()
                .filter(|v| v.is_object())
                .unwrap_or_else(|| json!({"version": 1, "host_id": "local-host"}));
            let ctx_window = core_config
                .pointer("/context_window/context_window_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(128_000);
            let reserved = core_config
                .pointer("/context_window/reserved_output_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(16_384);
            let prompt_limit = ctx_window.saturating_sub(reserved).max(4_096);
            let exact_invocation = json!({
                "conversation_id": conv_id,
                "run_id": run_id,
                "assistant_message_id": assistant_message_id,
                "invocation_config_digest": digest,
                "status": "running",
                "host_invocation": host_invocation,
                "context_budget": {
                    "context_window_tokens": ctx_window,
                    "reserved_output_tokens": reserved,
                    "prompt_token_limit": prompt_limit,
                    "auto_compact_tokens": (prompt_limit * 60) / 100,
                    "compaction_target_tokens": (prompt_limit * 45) / 100,
                    "auto_compact_percent": 60,
                    "compaction_target_percent": 45
                },
                "core_session_id": core_config.get("session_id").cloned().unwrap_or_else(|| json!(conv_id)),
                "checkpoint_revision": Value::Null,
                "checkpoint_event_cursor": Value::Null,
                "checkpoint_sha256": Value::Null,
                "remote_tool_plan": Value::Null
            });
            store
                .set_kv(&format!("exact_run_invocation:{conv_id}:{run_id}"), &exact_invocation)
                .map_err(|e| ("storage_error", e.to_string()))?;
            register_active_run(
                store,
                conv_id,
                json!({
                    "conversation_id": conv_id,
                    "run_id": run_id,
                    "assistant_message_id": assistant_message_id,
                    "invocation_config_digest": digest,
                    "status": "running",
                    "started_at_ms": ts
                }),
            );
            Ok(json!({
                "conversation_id": conv_id,
                "run_id": run_id,
                "assistant_message_id": assistant_message_id,
                "invocation_config_digest": digest,
                "execution": {
                    "control": {
                        "status": "running",
                        "run_id": run_id,
                        "permission_projection_revision": rev
                    },
                    "permissions": {
                        "revision": rev,
                        "permission_profile": profile,
                        "pending": pending,
                        "grants": grants
                    }
                }
            }))
        }
        "load_exact_run_invocation" => {
            let conv_id = args
                .get("conversation_id")
                .or_else(|| args.get("conversationID"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .ok_or(("invalid_argument", "missing 'conversation_id'".into()))?;
            let expected_run_id = args
                .get("expected_run_id")
                .or_else(|| args.get("expectedRunID"))
                .or_else(|| args.get("run_id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .ok_or(("invalid_argument", "missing 'expected_run_id'".into()))?;
            let assistant_message_id = args
                .get("assistant_message_id")
                .or_else(|| args.get("assistantMessageID"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty());
            let stored = store
                .get_kv(&format!("exact_run_invocation:{conv_id}:{expected_run_id}"))
                .map_err(|e| ("storage_error", e.to_string()))?;
            match stored {
                Some(inv) if !inv.is_null() => {
                    if let Some(expected_am) = assistant_message_id {
                        if inv.get("assistant_message_id").and_then(|v| v.as_str()) != Some(expected_am) {
                            return Ok(Value::Null);
                        }
                    }
                    Ok(inv)
                }
                _ => Ok(Value::Null),
            }
        }
        "stage_assistant_journal_content" => {
            let input = args.get("input").unwrap_or(args);
            let part_id = input
                .get("part_id")
                .or_else(|| input.get("partID"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .ok_or(("invalid_argument", "missing 'part_id'".into()))?;
            let record = input.get("record").cloned().unwrap_or_else(|| json!({}));
            let identity = record
                .get("identity")
                .cloned()
                .unwrap_or_else(|| {
                    json!({
                        "conversation_id": input.get("conversation_id").and_then(|v| v.as_str()).unwrap_or("default"),
                        "run_id": input.get("run_id").and_then(|v| v.as_str()).unwrap_or("default"),
                        "assistant_message_id": input.get("assistant_message_id").and_then(|v| v.as_str()).unwrap_or("default")
                    })
                });
            let conv_id = identity
                .get("conversation_id")
                .and_then(|v| v.as_str())
                .unwrap_or("default");
            let run_id = identity
                .get("run_id")
                .and_then(|v| v.as_str())
                .unwrap_or("default");
            let assistant_id = identity
                .get("assistant_message_id")
                .and_then(|v| v.as_str())
                .unwrap_or("default");
            let content_ref = input.get("content").cloned().unwrap_or_else(|| json!({}));
            let content_id = content_ref
                .get("content_id")
                .or_else(|| content_ref.get("id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .unwrap_or(part_id)
                .to_string();
            let restart = input
                .get("restart")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let content_key = format!("assistant_journal_content:{conv_id}:{run_id}:{part_id}:{content_id}");
            let mut text = if restart {
                String::new()
            } else {
                store
                    .get_kv(&content_key)
                    .ok()
                    .flatten()
                    .and_then(|v| v.get("text").and_then(|s| s.as_str()).map(ToString::to_string))
                    .unwrap_or_default()
            };
            if let Some(chunks) = input.get("chunks").and_then(|v| v.as_array()) {
                for ch in chunks {
                    if let Some(piece) = ch
                        .get("text")
                        .or_else(|| ch.get("data"))
                        .or_else(|| ch.get("utf8"))
                        .and_then(|v| v.as_str())
                    {
                        text.push_str(piece);
                    }
                }
            } else if let Some(raw_text) = input.get("text").and_then(|v| v.as_str()) {
                text.push_str(raw_text);
            }
            let total_bytes = text.len() as u64;
            let digest = sha256_hex(text.as_bytes());
            let ts = now_ms();
            store
                .set_kv(
                    &content_key,
                    &json!({
                        "content_id": content_id,
                        "part_id": part_id,
                        "text": text,
                        "total_utf8_bytes": total_bytes,
                        "sha256": digest,
                        "updated_at_ms": ts
                    }),
                )
                .map_err(|e| ("storage_error", e.to_string()))?;

            let part_key = format!("{conv_id}:{run_id}:{assistant_id}:{part_id}");
            let prev_rev = store
                .get_module_record::<Value>("assistant_journal_part", &part_key)
                .ok()
                .flatten()
                .and_then(|p| {
                    p.get("revision")
                        .and_then(|v| v.as_str().and_then(|s| s.parse::<u64>().ok()).or_else(|| v.as_u64()))
                })
                .unwrap_or(0);
            let next_rev = prev_rev + 1;
            let body_ref = json!({
                "content_id": content_id,
                "part_id": part_id,
                "total_utf8_bytes": total_bytes.to_string(),
                "utf8_bytes": total_bytes,
                "sha256": digest,
                "staged": true
            });
            let part_rec = json!({
                "identity": identity,
                "part_id": part_id,
                "generation": "1",
                "revision": next_rev.to_string(),
                "metadata_revision": next_rev.to_string(),
                "order_key": {"section": "body", "position": "0"},
                "metadata": record.get("metadata").cloned().unwrap_or_else(|| json!({"kind": "text"})),
                "bodies": [body_ref]
            });
            store
                .put_module_record("assistant_journal_part", &part_key, &part_rec)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(body_ref)
        }
        "read_assistant_journal_parts" => {
            let snapshot = args.get("snapshot").cloned().unwrap_or_else(|| json!({}));
            let at = snapshot
                .get("at")
                .or_else(|| args.get("at"))
                .cloned()
                .unwrap_or_else(|| json!({"revision": "1"}));
            let identity = snapshot
                .get("identity")
                .or_else(|| at.get("identity"))
                .or_else(|| args.get("identity"));
            let conv_filter = identity
                .and_then(|i| i.get("conversation_id"))
                .or_else(|| args.get("conversation_id"))
                .and_then(|v| v.as_str());
            let run_filter = identity
                .and_then(|i| i.get("run_id"))
                .or_else(|| args.get("run_id"))
                .and_then(|v| v.as_str());
            let after_part_id = args
                .get("after_part_id")
                .or_else(|| args.get("afterPartID"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty());
            let max_records = args
                .get("max_records")
                .or_else(|| args.get("maxRecords"))
                .and_then(|v| v.as_u64())
                .unwrap_or(64)
                .clamp(1, 512) as usize;

            let mut parts: Vec<Value> = store
                .list_module_records::<Value>("assistant_journal_part")
                .map_err(|e| ("storage_error", e.to_string()))?
                .into_iter()
                .filter(|p| {
                    let conv_ok = conv_filter.is_none_or(|c| {
                        p.pointer("/identity/conversation_id").and_then(|v| v.as_str()) == Some(c)
                    });
                    let run_ok = run_filter.is_none_or(|r| {
                        p.pointer("/identity/run_id").and_then(|v| v.as_str()) == Some(r)
                    });
                    conv_ok && run_ok
                })
                .collect();
            parts.sort_by(|a, b| {
                a.get("part_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .cmp(b.get("part_id").and_then(|v| v.as_str()).unwrap_or(""))
            });
            let filtered: Vec<Value> = parts
                .into_iter()
                .filter(|p| {
                    after_part_id.is_none_or(|after| {
                        p.get("part_id").and_then(|v| v.as_str()).unwrap_or("") > after
                    })
                })
                .collect();
            let has_more = filtered.len() > max_records;
            let page: Vec<Value> = filtered.into_iter().take(max_records).collect();
            let next_part_id = if has_more {
                page.last()
                    .and_then(|p| p.get("part_id").and_then(|v| v.as_str()))
                    .map(ToString::to_string)
            } else {
                None
            };
            Ok(json!({
                "at": at,
                "parts": page,
                "next_part_id": next_part_id
            }))
        }
        "read_assistant_journal_part_changes" => {
            let snapshot = args.get("snapshot").cloned().unwrap_or_else(|| json!({}));
            let at = snapshot
                .get("at")
                .or_else(|| args.get("at"))
                .cloned()
                .unwrap_or_else(|| json!({"revision": "1"}));
            let base = args
                .get("base")
                .cloned()
                .unwrap_or_else(|| json!({"revision": "0"}));
            let base_rev = base
                .get("revision")
                .or_else(|| base.get("sequence"))
                .and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|s| s.parse().ok())))
                .unwrap_or(0);
            let identity = snapshot
                .get("identity")
                .or_else(|| at.get("identity"))
                .or_else(|| args.get("identity"));
            let conv_filter = identity
                .and_then(|i| i.get("conversation_id"))
                .or_else(|| args.get("conversation_id"))
                .and_then(|v| v.as_str());
            let run_filter = identity
                .and_then(|i| i.get("run_id"))
                .or_else(|| args.get("run_id"))
                .and_then(|v| v.as_str());
            let after_part_id = args
                .get("after_part_id")
                .or_else(|| args.get("afterPartID"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty());
            let max_records = args
                .get("max_records")
                .or_else(|| args.get("maxRecords"))
                .and_then(|v| v.as_u64())
                .unwrap_or(64)
                .clamp(1, 512) as usize;

            let mut parts: Vec<Value> = store
                .list_module_records::<Value>("assistant_journal_part")
                .map_err(|e| ("storage_error", e.to_string()))?
                .into_iter()
                .filter(|p| {
                    let conv_ok = conv_filter.is_none_or(|c| {
                        p.pointer("/identity/conversation_id").and_then(|v| v.as_str()) == Some(c)
                    });
                    let run_ok = run_filter.is_none_or(|r| {
                        p.pointer("/identity/run_id").and_then(|v| v.as_str()) == Some(r)
                    });
                    let rev = p
                        .get("revision")
                        .and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|s| s.parse().ok())))
                        .unwrap_or(0);
                    conv_ok && run_ok && rev > base_rev
                })
                .collect();
            parts.sort_by(|a, b| {
                a.get("part_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .cmp(b.get("part_id").and_then(|v| v.as_str()).unwrap_or(""))
            });
            let filtered: Vec<Value> = parts
                .into_iter()
                .filter(|p| {
                    after_part_id.is_none_or(|after| {
                        p.get("part_id").and_then(|v| v.as_str()).unwrap_or("") > after
                    })
                })
                .collect();
            let has_more = filtered.len() > max_records;
            let changes: Vec<Value> = filtered
                .into_iter()
                .take(max_records)
                .map(|p| {
                    let pid = p
                        .get("part_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    json!({"part_id": pid, "head": p})
                })
                .collect();
            let next_part_id = if has_more {
                changes
                    .last()
                    .and_then(|c| c.get("part_id").and_then(|v| v.as_str()))
                    .map(ToString::to_string)
            } else {
                None
            };
            Ok(json!({
                "base": base,
                "at": at,
                "changes": changes,
                "next_part_id": next_part_id
            }))
        }
        "read_assistant_journal_range" => {
            let snapshot = args.get("snapshot").cloned().unwrap_or_else(|| json!({}));
            let identity = snapshot
                .get("identity")
                .or_else(|| snapshot.pointer("/at/identity"))
                .or_else(|| args.get("identity"));
            let conv_id = identity
                .and_then(|i| i.get("conversation_id"))
                .or_else(|| args.get("conversation_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("default");
            let run_id = identity
                .and_then(|i| i.get("run_id"))
                .or_else(|| args.get("run_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("default");
            let part_id = args
                .get("part_id")
                .or_else(|| args.get("partID"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .ok_or(("invalid_argument", "missing 'part_id'".into()))?;
            let content_id = args
                .pointer("/content/content_id")
                .or_else(|| args.pointer("/content/id"))
                .or_else(|| args.get("content_id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .unwrap_or(part_id);
            let offset_bytes = args
                .get("offset_bytes")
                .or_else(|| args.get("offsetBytes"))
                .and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|s| s.parse().ok())))
                .unwrap_or(0) as usize;
            let limit_bytes = args
                .get("limit_bytes")
                .or_else(|| args.get("limitBytes"))
                .and_then(|v| v.as_u64())
                .unwrap_or(32_768)
                .clamp(1, 262_144) as usize;

            let content_key = format!("assistant_journal_content:{conv_id}:{run_id}:{part_id}:{content_id}");
            let full_text = store
                .get_kv(&content_key)
                .map_err(|e| ("storage_error", e.to_string()))?
                .and_then(|v| v.get("text").and_then(|s| s.as_str()).map(ToString::to_string))
                .unwrap_or_default();
            let total = full_text.len();
            let mut start = offset_bytes.min(total);
            while start < total && !full_text.is_char_boundary(start) {
                start += 1;
            }
            let mut end = start.saturating_add(limit_bytes).min(total);
            while end > start && !full_text.is_char_boundary(end) {
                end -= 1;
            }
            let slice = &full_text[start..end];
            Ok(json!({
                "offset_bytes": start.to_string(),
                "next_offset_bytes": end.to_string(),
                "total_utf8_bytes": total.to_string(),
                "text": slice,
                "end_of_content": end >= total
            }))
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

fn load_product_setting_entry(
    store: &Store,
    key: &str,
) -> Result<Option<(Value, String, u64, i64)>, (&'static str, String)> {
    let raw = store
        .get_kv(&format!("product_setting:{key}"))
        .map_err(|e| ("storage_error", e.to_string()))?;
    let Some(v) = raw else {
        return Ok(None);
    };
    if v.get("__nola_setting").and_then(|b| b.as_bool()) == Some(true) {
        if v.get("deleted").and_then(|b| b.as_bool()) == Some(true) {
            return Ok(None);
        }
        let val = v.get("value").cloned().unwrap_or(Value::Null);
        let val_json = v
            .get("value_json")
            .and_then(|s| s.as_str())
            .map(ToString::to_string)
            .unwrap_or_else(|| serde_json::to_string(&val).unwrap_or_else(|_| "null".into()));
        let rev = v.get("revision").and_then(|r| r.as_u64()).unwrap_or(1);
        let ts = v
            .get("updated_at_ms")
            .and_then(|t| t.as_i64())
            .unwrap_or(0);
        return Ok(Some((val, val_json, rev, ts)));
    }
    let val_json = serde_json::to_string(&v).unwrap_or_else(|_| "null".into());
    Ok(Some((v, val_json, 1, 0)))
}

fn put_product_setting_impl(
    store: &Store,
    args: &Value,
    require_check: bool,
) -> Result<Value, (&'static str, String)> {
    let input = args.get("input").unwrap_or(args);
    let key = input
        .get("key")
        .or_else(|| args.get("key"))
        .and_then(|v| v.as_str())
        .ok_or(("invalid_argument", "missing 'key'".into()))?;
    let raw_prev = store
        .get_kv(&format!("product_setting:{key}"))
        .map_err(|e| ("storage_error", e.to_string()))?;
    let cur_rev = match raw_prev {
        Some(ref v) if v.get("__nola_setting").and_then(|b| b.as_bool()) == Some(true) => {
            v.get("revision").and_then(|r| r.as_u64()).unwrap_or(0)
        }
        Some(_) => 1,
        None => 0,
    };
    let expected_rev = input
        .get("expected_revision")
        .or_else(|| args.get("expected_revision"))
        .and_then(|v| v.as_u64());
    if let Some(exp) = expected_rev {
        if cur_rev != exp {
            return Err((
                "revision_conflict",
                format!("product_setting '{key}' revision conflict: expected {exp}, actual {cur_rev}"),
            ));
        }
    } else if require_check && cur_rev > 0 && input.get("expected_revision").is_some() {
        // Explicit `expected_revision: null` on an already existing setting conflicts.
        return Err((
            "revision_conflict",
            format!("product_setting '{key}' already exists at revision {cur_rev}"),
        ));
    }

    let (val, val_json_str) = if let Some(vj) = input.get("value_json").or_else(|| args.get("value_json")) {
        if let Some(s) = vj.as_str() {
            let parsed = serde_json::from_str::<Value>(s).unwrap_or_else(|_| Value::String(s.to_string()));
            (parsed, s.to_string())
        } else {
            let s = serde_json::to_string(vj).unwrap_or_else(|_| "null".into());
            (vj.clone(), s)
        }
    } else {
        let v = input
            .get("value")
            .or_else(|| args.get("value"))
            .cloned()
            .unwrap_or(Value::Null);
        let s = serde_json::to_string(&v).unwrap_or_else(|_| "null".into());
        (v, s)
    };

    let next_rev = cur_rev + 1;
    let ts = now_ms();
    store
        .set_kv(
            &format!("product_setting:{key}"),
            &json!({
                "__nola_setting": true,
                "key": key,
                "value": val,
                "value_json": val_json_str,
                "revision": next_rev,
                "updated_at_ms": ts
            }),
        )
        .map_err(|e| ("storage_error", e.to_string()))?;
    Ok(json!({
        "key": key,
        "value_json": val_json_str,
        "value": val,
        "revision": next_rev,
        "updated_at_ms": ts,
        "ok": true
    }))
}

fn dispatch_context_command(
    store: &Store,
    args: &Value,
) -> Result<Value, (&'static str, String)> {
    let (op, payload) = extract_sub_command(args);
    let input = payload.get("input").unwrap_or(payload);
    let conv_id = input
        .get("conversation_id")
        .or_else(|| input.get("id"))
        .or_else(|| payload.get("conversation_id"))
        .or_else(|| args.get("conversation_id"))
        .and_then(|v| v.as_str())
        .ok_or(("invalid_argument", "missing conversation_id".into()))?;

    match op {
        "prepare_compaction" => {
            let (msgs, _) = crate::web_api::reconstruct_v2_messages(store, conv_id);
            let usage = load_session_usage(store, conv_id);
            Ok(json!({
                "conversation_id": conv_id,
                "needed": msgs.len() > 20,
                "message_count": msgs.len(),
                "usage": usage
            }))
        }
        "prepare_maintenance" => {
            let (msgs, _) = crate::web_api::reconstruct_v2_messages(store, conv_id);
            Ok(json!({
                "conversation_id": conv_id,
                "message_count": msgs.len(),
                "recommended_actions": []
            }))
        }
        "commit_maintenance" => {
            let ts = now_ms();
            let rec = json!({
                "conversation_id": conv_id,
                "committed_at_ms": ts,
                "details": input
            });
            store
                .set_kv(&format!("ctx_maintenance:{conv_id}"), &rec)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"ok": true, "conversation_id": conv_id, "committed_at_ms": ts}))
        }
        "replace_summary" => {
            let summary = input
                .get("summary")
                .or_else(|| input.get("text"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let ts = now_ms();
            store
                .set_kv(
                    &format!("ctx_summary:{conv_id}"),
                    &json!({"summary": summary, "updated_at_ms": ts}),
                )
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"ok": true, "conversation_id": conv_id, "summary": summary, "updated_at_ms": ts}))
        }
        "commit_checkpoint" => {
            let cp_id = input
                .get("checkpoint_id")
                .or_else(|| input.get("id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(new_id);
            let key = format!("ctx_checkpoints:{conv_id}");
            let mut list = store
                .get_kv(&key)
                .ok()
                .flatten()
                .and_then(|v| v.as_array().cloned())
                .unwrap_or_default();
            let cp = json!({
                "checkpoint_id": cp_id,
                "conversation_id": conv_id,
                "payload": input,
                "created_at_ms": now_ms()
            });
            list.push(cp.clone());
            store
                .set_kv(&key, &Value::Array(list.clone()))
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({
                "ok": true,
                "conversation_id": conv_id,
                "checkpoint_id": cp_id,
                "checkpoint": cp,
                "count": list.len()
            }))
        }
        "count_checkpoints" => {
            let count = store
                .get_kv(&format!("ctx_checkpoints:{conv_id}"))
                .ok()
                .flatten()
                .and_then(|v| v.as_array().map(Vec::len))
                .unwrap_or(0);
            Ok(json!({"conversation_id": conv_id, "count": count}))
        }
        "load_checkpoint" => {
            let cp_id = input
                .get("checkpoint_id")
                .or_else(|| input.get("id"))
                .and_then(|v| v.as_str());
            let list = store
                .get_kv(&format!("ctx_checkpoints:{conv_id}"))
                .ok()
                .flatten()
                .and_then(|v| v.as_array().cloned())
                .unwrap_or_default();
            let found = if let Some(cid) = cp_id {
                list.into_iter()
                    .find(|c| c.get("checkpoint_id").and_then(|v| v.as_str()) == Some(cid))
            } else {
                list.into_iter().last()
            };
            Ok(json!({
                "conversation_id": conv_id,
                "checkpoint": found.unwrap_or(Value::Null)
            }))
        }
        other => Err((
            "unsupported_command",
            format!("unsupported context operation '{other}'"),
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

fn load_conversation_settings_str(store: &Store, id: &str) -> Option<String> {
    store
        .get_kv(&format!("conv_settings:{id}"))
        .ok()
        .flatten()
        .and_then(|v| match v {
            Value::Null => None,
            Value::String(s) => Some(s),
            other => Some(other.to_string()),
        })
}

fn build_context_usage_json(store: &Store, conv_id: &str, rev: i64) -> Value {
    let usage = load_session_usage(store, conv_id);
    let cfg = crate::config::load_active_config(store);
    let context_window_tokens = crate::config::resolve_model_route(&cfg)
        .map(|r| r.context_window as u64)
        .unwrap_or(128_000)
        .max(1);
    let used_tokens = usage.input_tokens;
    let prompt_token_limit = context_window_tokens;
    let usage_percent =
        ((used_tokens as f64 / context_window_tokens as f64) * 100.0).min(100.0);
    let estimated = !usage.usage_reported;
    let usage_val = serde_json::to_value(&usage).unwrap_or_else(|_| json!({}));
    let mut val = usage_val.clone();
    if let Some(obj) = val.as_object_mut() {
        obj.insert("conversation_id".into(), json!(conv_id));
        obj.insert("usage".into(), usage_val);
        obj.insert("used_tokens".into(), json!(used_tokens));
        obj.insert(
            "context_window_tokens".into(),
            json!(context_window_tokens),
        );
        obj.insert("prompt_token_limit".into(), json!(prompt_token_limit));
        obj.insert("usage_percent".into(), json!(usage_percent));
        obj.insert("usage_is_estimated".into(), json!(estimated));
        obj.insert("estimated".into(), json!(estimated));
        obj.insert("measured_at_revision".into(), json!(rev.max(1)));
    }
    val
}

fn build_conversation_projection_response(
    store: &Store,
    sess: &Session,
    offset: usize,
    limit: Option<usize>,
    before_ordinal: Option<i64>,
) -> Value {
    let conv = crate::web_api::session_to_conversation_json(store, sess);
    let revision = conv
        .get("revision")
        .and_then(|v| v.as_i64())
        .unwrap_or(1)
        .max(1);
    let configuration_revision = conv
        .get("configurationRevision")
        .and_then(|v| v.as_i64())
        .unwrap_or(1)
        .max(1);
    let settings_json = load_conversation_settings_str(store, &sess.id);
    let (raw_messages, events) = crate::web_api::reconstruct_v2_messages(store, &sess.id);
    let total_message_count = raw_messages.len() as i64;
    let event_sequence = events
        .last()
        .and_then(|e| e.get("seq").and_then(|v| v.as_i64()))
        .unwrap_or(0);

    let (sliced, has_more, next_before_ordinal): (Vec<Value>, bool, Option<i64>) =
        if let Some(before) = before_ordinal {
            let cap = (before.max(0) as usize).min(raw_messages.len());
            let lim = limit.unwrap_or(raw_messages.len());
            let start = cap.saturating_sub(lim);
            let slice = raw_messages[start..cap].to_vec();
            let more = start > 0;
            let next_ord = if more { Some(start as i64) } else { None };
            (slice, more, next_ord)
        } else if let Some(lim) = limit {
            let start = offset.min(raw_messages.len());
            let end = (start + lim).min(raw_messages.len());
            let slice = raw_messages[start..end].to_vec();
            let more = end < raw_messages.len();
            let next_ord = if more { Some(end as i64) } else { None };
            (slice, more, next_ord)
        } else {
            let start = offset.min(raw_messages.len());
            (raw_messages[start..].to_vec(), false, None)
        };

    let canonical_messages: Vec<Value> = sliced
        .into_iter()
        .enumerate()
        .map(|(idx, mut m)| {
            let msg_id = m
                .get("id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(|| format!("msg-{}", idx + 1));
            let role = m
                .get("role")
                .and_then(|v| v.as_str())
                .unwrap_or("assistant")
                .to_string();
            let content = m
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let created_at_ms = m
                .get("createdAtMs")
                .or_else(|| m.get("created_at_ms"))
                .and_then(|v| v.as_i64())
                .unwrap_or(sess.created_at_ms);
            let part_id = format!("{msg_id}:text");
            if let Some(obj) = m.as_object_mut() {
                obj.insert("id".into(), json!(msg_id));
                obj.insert("message_id".into(), json!(msg_id));
                obj.insert("role".into(), json!(role));
                obj.insert("state".into(), json!("final"));
                obj.insert("revision".into(), json!(1));
                obj.insert("created_at_ms".into(), json!(created_at_ms));
                obj.insert(
                    "parts".into(),
                    json!([{
                        "kind": "text",
                        "part_id": part_id,
                        "revision": 1,
                        "markdown": content,
                        "is_final": true
                    }]),
                );
            }
            m
        })
        .collect();

    let committed_message_ids: Vec<String> = canonical_messages
        .iter()
        .filter_map(|m| m.get("message_id").and_then(|v| v.as_str()).map(ToString::to_string))
        .collect();
    let context_usage = build_context_usage_json(store, &sess.id, revision);
    let surface = json!({
        "revision": revision,
        "committed_message_ids": committed_message_ids,
        "live_message_id": Value::Null,
        "current_activity_part_id": Value::Null,
        "archived_activity_part_ids": [],
        "show_assistant_placeholder": false
    });
    let active_run_val = lookup_active_run(store, &sess.id).unwrap_or(Value::Null);
    let projection = json!({
        "contract_version": 1,
        "conversation_id": sess.id,
        "id": sess.id,
        "title": sess.title,
        "revision": revision,
        "configuration_revision": configuration_revision,
        "execution_ownership": {"kind": "user"},
        "active_run": active_run_val,
        "messages": canonical_messages,
        "context_usage": context_usage,
        "surface": surface
    });

    json!({
        "contract_version": 1,
        "conversation_id": sess.id,
        "id": sess.id,
        "title": sess.title,
        "revision": revision,
        "configuration_revision": configuration_revision,
        "created_at_ms": sess.created_at_ms,
        "updated_at_ms": sess.updated_at_ms,
        "settings_json": settings_json,
        "execution_ownership": {"kind": "user"},
        "active_run": active_run_val,
        "context_usage": context_usage,
        "surface": surface,
        "conversation": conv,
        "messages": canonical_messages,
        "events": events,
        "has_more": has_more,
        "projection": projection,
        "event_sequence": event_sequence,
        "total_message_count": total_message_count,
        "before_ordinal": next_before_ordinal,
        "assistant_journal_heads": {}
    })
}

fn resolve_remote_model_profile(
    store: &Store,
    suite: &ModulesSuite,
    args: &Value,
) -> Result<Value, (&'static str, String)> {
    let input = args.get("input").unwrap_or(args);
    let profile_id = input
        .get("profile_id")
        .or_else(|| input.get("id"))
        .or_else(|| args.get("profile_id"))
        .or_else(|| args.get("id"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .ok_or(("invalid_argument", "missing 'profile_id'".into()))?;

    if let Ok(Some(rec)) = suite.get_record::<Value>("model", profile_id) {
        let source_val: Value = rec
            .get("source_json")
            .and_then(|v| v.as_str())
            .and_then(|s| serde_json::from_str(s).ok())
            .or_else(|| rec.get("source").cloned())
            .unwrap_or_else(|| json!({}));
        let params_val: Value = rec
            .get("parameters_json")
            .and_then(|v| v.as_str())
            .and_then(|s| serde_json::from_str(s).ok())
            .or_else(|| rec.get("parameters").cloned())
            .unwrap_or_else(|| json!({}));
        let credential_ref = source_val
            .get("credentialRef")
            .or_else(|| source_val.get("credential_ref"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(ToString::to_string);
        let provider = rec
            .get("provider")
            .and_then(|v| v.as_str())
            .unwrap_or("openai");
        let model_name = rec
            .get("model_name")
            .or_else(|| rec.get("model"))
            .and_then(|v| v.as_str())
            .unwrap_or(profile_id);
        let endpoint = source_val
            .get("endpoint")
            .or_else(|| source_val.get("baseURL"))
            .or_else(|| rec.get("endpoint"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let wire_protocol = source_val
            .get("wire_protocol")
            .or_else(|| source_val.get("protocol"))
            .or_else(|| rec.get("protocol"))
            .and_then(|v| v.as_str())
            .unwrap_or("chat_completions");
        let revision = rec.get("revision").and_then(|v| v.as_u64()).unwrap_or(1);
        let resolved = json!({
            "profile_id": profile_id,
            "id": profile_id,
            "provider": provider,
            "model_name": model_name,
            "endpoint": endpoint,
            "wire_protocol": wire_protocol,
            "parameters": params_val,
            "revision": revision
        });
        return Ok(json!({
            "profile_id": profile_id,
            "id": profile_id,
            "provider": provider,
            "model_name": model_name,
            "endpoint": endpoint,
            "wire_protocol": wire_protocol,
            "revision": revision,
            "credential_ref": credential_ref,
            "credential_required": credential_ref.is_some(),
            "resolved": resolved,
            "profile": rec
        }));
    }

    let cfg = crate::config::load_active_config(store);
    for conn in &cfg.llm.connections {
        for m in &conn.models {
            let full_id = format!("{}:{}", conn.id, m.id);
            if m.id == profile_id || full_id == profile_id {
                let wire_protocol = conn
                    .protocol
                    .as_deref()
                    .or(conn.provider.as_deref())
                    .unwrap_or("chat_completions");
                let provider = conn.provider.as_deref().unwrap_or(&conn.id);
                let params = json!({
                    "context_window": m.context_window,
                    "max_output_tokens": m.max_output_tokens,
                    "temperature": m.temperature,
                    "top_p": m.top_p,
                    "reasoning_effort": m.reasoning_effort
                });
                let resolved = json!({
                    "profile_id": profile_id,
                    "id": profile_id,
                    "connection_id": conn.id,
                    "provider": provider,
                    "model_name": m.id,
                    "endpoint": conn.endpoint,
                    "wire_protocol": wire_protocol,
                    "parameters": params,
                    "revision": 1
                });
                return Ok(json!({
                    "profile_id": profile_id,
                    "id": profile_id,
                    "connection_id": conn.id,
                    "provider": provider,
                    "model_name": m.id,
                    "endpoint": conn.endpoint,
                    "wire_protocol": wire_protocol,
                    "revision": 1,
                    "credential_ref": Value::Null,
                    "credential_required": false,
                    "resolved": resolved
                }));
            }
        }
    }

    Err((
        "not_found",
        format!("remote model profile '{profile_id}' not found"),
    ))
}

fn canonical_skill_editor_envelope(raw: &Value, fallback_id: Option<&str>, fallback_rev: u64) -> Value {
    let rec = raw.get("record").unwrap_or(raw);
    let ts = now_ms();
    let id = rec
        .get("id")
        .or_else(|| rec.get("name"))
        .or_else(|| raw.get("id"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .or(fallback_id)
        .unwrap_or("skill")
        .to_string();
    let path_str = rec.get("path").and_then(|v| v.as_str()).unwrap_or("");
    let raw_source = rec
        .get("source")
        .and_then(|v| v.as_str())
        .unwrap_or(if path_str.starts_with("builtin://") {
            "built_in"
        } else {
            "user"
        });
    let source = match raw_source {
        "builtin" | "built_in" => "built_in",
        "user" => "user",
        _ => {
            if path_str.starts_with("builtin://") {
                "built_in"
            } else {
                "user"
            }
        }
    };
    let display_name = rec
        .get("display_name")
        .or_else(|| rec.get("name"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(&id)
        .to_string();
    let summary = rec
        .get("summary")
        .or_else(|| rec.get("description"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let category = rec
        .get("category")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("general")
        .to_string();
    let instructions_ref = rec
        .get("instructions_ref")
        .or_else(|| rec.get("path"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let instructions_text = rec
        .get("instructions_text")
        .or_else(|| rec.get("body"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let required_tools = rec
        .get("required_tools")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let version = rec
        .get("version")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("1.0.0")
        .to_string();
    let enabled = rec
        .get("enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let updated_at_ms = rec
        .get("updated_at_ms")
        .and_then(|v| v.as_i64())
        .unwrap_or(ts);
    let created_at_ms = rec
        .get("created_at_ms")
        .and_then(|v| v.as_i64())
        .unwrap_or(updated_at_ms);
    let revision = raw
        .get("revision")
        .or_else(|| rec.get("revision"))
        .and_then(|v| v.as_u64())
        .unwrap_or(fallback_rev)
        .max(1);

    json!({
        "record": {
            "id": id,
            "source": source,
            "display_name": display_name,
            "summary": summary,
            "category": category,
            "instructions_ref": instructions_ref,
            "instructions_text": instructions_text,
            "required_tools": required_tools,
            "version": version,
            "enabled": enabled,
            "created_at_ms": created_at_ms,
            "updated_at_ms": updated_at_ms
        },
        "revision": revision
    })
}

fn canonical_memory_envelope(item: &MemoryItem) -> Value {
    let ns = if item.namespace.trim().is_empty() {
        "default"
    } else {
        item.namespace.as_str()
    };
    let (scope, kind) = ns
        .split_once(':')
        .unwrap_or(("global", if ns == "default" { "preference" } else { ns }));
    json!({
        "record": {
            "id": item.id,
            "owner": {"kind": "local_profile", "id": "default"},
            "namespace": {"kind": "user", "name": ns},
            "scope": scope,
            "kind": kind,
            "title": item.title,
            "content": item.content,
            "tags": item.tags,
            "confidence": 1.0,
            "supersedes": [],
            "retention": {"mode": "permanent"},
            "provenance": {"kind": "explicit_user", "source": "user", "item_ids": []}
        },
        "created_at_ms": item.created_at_ms,
        "updated_at_ms": item.updated_at_ms
    })
}

fn parse_memory_item(payload: &Value) -> MemoryItem {
    let input = payload.get("input").unwrap_or(payload);
    let rec = input.get("record").unwrap_or(input);
    let ts = now_ms();
    let namespace = rec
        .get("namespace")
        .and_then(|v| {
            v.as_str().map(ToString::to_string).or_else(|| {
                v.as_object().and_then(|obj| {
                    obj.get("name")
                        .and_then(|n| n.as_str())
                        .filter(|s| !s.trim().is_empty())
                        .or_else(|| obj.get("kind").and_then(|k| k.as_str()))
                        .map(ToString::to_string)
                })
            })
        })
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
            .or_else(|| input.get("id"))
            .or_else(|| payload.get("id"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(ToString::to_string)
            .unwrap_or_else(new_id),
        namespace,
        title: rec
            .get("title")
            .or_else(|| rec.get("summary"))
            .or_else(|| rec.get("kind"))
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
        created_at_ms: input
            .get("created_at_ms")
            .or_else(|| rec.get("created_at_ms"))
            .and_then(|v| v.as_i64())
            .unwrap_or(ts),
        updated_at_ms: input
            .get("updated_at_ms")
            .or_else(|| input.get("proposed_at_ms"))
            .or_else(|| rec.get("updated_at_ms"))
            .and_then(|v| v.as_i64())
            .unwrap_or(ts),
    }
}

fn parse_mcp_server_config(payload: &Value) -> Result<McpServerConfig, (&'static str, String)> {
    let raw_inner = payload
        .get("input")
        .or_else(|| payload.get("server"))
        .or_else(|| payload.get("config"))
        .or_else(|| payload.get("record"))
        .unwrap_or(payload);
    // Merge optional `configuration_json` object/string into the config map so all fields are visible.
    let mut merged = serde_json::Map::new();
    if let Some(obj) = raw_inner.as_object() {
        for (k, v) in obj {
            merged.insert(k.clone(), v.clone());
        }
    }
    if let Some(cj) = raw_inner.get("configuration_json").or_else(|| payload.get("configuration_json")) {
        let parsed_cj = if let Some(s) = cj.as_str() {
            serde_json::from_str::<Value>(s).ok()
        } else {
            Some(cj.clone())
        };
        if let Some(Value::Object(map)) = parsed_cj {
            for (k, v) in map {
                merged.entry(k).or_insert(v);
            }
        }
    }
    let cfg_val = &Value::Object(merged);
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
        ("model", "get" | "get_profile") => {
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
        ("model", "list_connections" | "list_accounts") => {
            let cfg = crate::config::load_active_config(store);
            let mut list: Vec<Value> = crate::config::redacted_llm_config(&cfg.llm)
                .get("connections")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            for kind in ["model_connection", "model_account"] {
                for rec in suite.list_records::<Value>(kind).unwrap_or_default() {
                    let rid = rec.get("id").and_then(|v| v.as_str());
                    if !rid.is_some_and(|id| {
                        list.iter()
                            .any(|x| x.get("id").and_then(|v| v.as_str()) == Some(id))
                    }) {
                        list.push(rec);
                    }
                }
            }
            Ok(json!({"connections": list, "accounts": list, "items": list}))
        }
        ("model", "get_connection" | "get_account") => {
            let id = payload
                .get("id")
                .or_else(|| payload.get("connection_id"))
                .or_else(|| payload.get("account_id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            if let Ok(Some(rec)) = suite.get_record::<Value>("model_connection", id) {
                return Ok(rec);
            }
            if let Ok(Some(rec)) = suite.get_record::<Value>("model_account", id) {
                return Ok(rec);
            }
            let cfg = crate::config::load_active_config(store);
            let found = crate::config::redacted_llm_config(&cfg.llm)
                .get("connections")
                .and_then(|v| v.as_array())
                .and_then(|arr| {
                    arr.iter()
                        .find(|c| c.get("id").and_then(|v| v.as_str()) == Some(id))
                        .cloned()
                });
            Ok(found.unwrap_or(Value::Null))
        }
        ("model", "save_account") => {
            let input = payload
                .get("input")
                .or_else(|| payload.get("account"))
                .unwrap_or(payload);
            let id = input
                .get("id")
                .or_else(|| input.get("account_id"))
                .or_else(|| payload.get("id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(new_id);
            if let Some(secret) = input
                .get("api_key")
                .or_else(|| input.get("apiKey"))
                .or_else(|| input.get("token"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                if let Ok(vault) =
                    nola_store::Vault::open(crate::config::resolve_vault_path_for_store(store))
                {
                    let _ = vault.set_secret(&format!("model_account_secret:{id}"), secret);
                }
            }
            let mut rec = input.clone();
            if let Some(obj) = rec.as_object_mut() {
                obj.insert("id".into(), json!(id));
                obj.insert("account_id".into(), json!(id));
                obj.remove("api_key");
                obj.remove("apiKey");
                obj.remove("token");
                obj.insert("updated_at_ms".into(), json!(now_ms()));
            }
            suite
                .upsert_record("model_account", &id, &rec)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"ok": true, "id": id, "account": rec}))
        }
        ("model", "save_connection") => {
            let input = payload
                .get("input")
                .or_else(|| payload.get("connection"))
                .unwrap_or(payload);
            let id = input
                .get("id")
                .or_else(|| input.get("connection_id"))
                .or_else(|| payload.get("id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(new_id);
            let mut sanitized = input.clone();
            if let Some(obj) = sanitized.as_object_mut() {
                obj.insert("id".into(), json!(id));
                obj.insert("connection_id".into(), json!(id));
                obj.remove("api_key");
                obj.remove("apiKey");
                obj.insert("updated_at_ms".into(), json!(now_ms()));
            }
            suite
                .upsert_record("model_connection", &id, &sanitized)
                .map_err(|e| ("storage_error", e.to_string()))?;
            if let Some(ep) = input
                .get("endpoint")
                .or_else(|| input.get("base_url"))
                .or_else(|| input.get("baseURL"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                let mut cfg = crate::config::load_active_config(store);
                let protocol = input
                    .get("protocol")
                    .or_else(|| input.get("wire_protocol"))
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string);
                let provider = input
                    .get("provider")
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string);
                let api_key = input
                    .get("api_key")
                    .or_else(|| input.get("apiKey"))
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(ToString::to_string);
                let models: Vec<crate::config::ModelEntry> = input
                    .get("models")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|m| {
                                m.as_str()
                                    .or_else(|| m.get("id").and_then(|v| v.as_str()))
                                    .map(|mid| crate::config::ModelEntry {
                                        id: mid.to_string(),
                                        context_window: None,
                                        max_output_tokens: None,
                                        temperature: None,
                                        top_p: None,
                                        reasoning_effort: None,
                                        capabilities: None,
                                    })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                if let Some(existing) = cfg.llm.connections.iter_mut().find(|c| c.id == id) {
                    existing.endpoint = ep.to_string();
                    if protocol.is_some() {
                        existing.protocol = protocol;
                    }
                    if provider.is_some() {
                        existing.provider = provider;
                    }
                    if api_key.is_some() {
                        existing.api_key = api_key;
                    }
                    for m in models {
                        if !existing.models.iter().any(|e| e.id == m.id) {
                            existing.models.push(m);
                        }
                    }
                } else {
                    cfg.llm.connections.push(crate::config::ConnectionConfig {
                        id: id.clone(),
                        endpoint: ep.to_string(),
                        protocol,
                        provider,
                        api_key,
                        api_key_env: None,
                        models,
                    });
                }
                let _ = crate::config::save_applied_config(store, &cfg);
            }
            Ok(json!({"ok": true, "id": id, "connection": sanitized}))
        }
        ("model", "set_connection_capability_override") => {
            let input = payload.get("input").unwrap_or(payload);
            let conn_id = input
                .get("connection_id")
                .or_else(|| input.get("connectionId"))
                .or_else(|| args.get("connection_id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'connection_id'".into()))?;
            let model_id = input
                .get("model_id")
                .or_else(|| input.get("modelId"))
                .or_else(|| args.get("model_id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'model_id'".into()))?;
            let protocol = input
                .get("protocol")
                .and_then(|v| v.as_str())
                .unwrap_or("chat_completions");
            let caps = input
                .get("capabilities")
                .or_else(|| input.get("override"))
                .cloned()
                .unwrap_or(Value::Null);
            let kv_key = format!("model_cap_override:{conn_id}:{model_id}");
            let stored = if caps.is_null() {
                store
                    .set_kv(&kv_key, &Value::Null)
                    .map_err(|e| ("storage_error", e.to_string()))?;
                Value::Null
            } else {
                let rec = json!({
                    "connection_id": conn_id,
                    "model_id": model_id,
                    "protocol": protocol,
                    "user_override": caps,
                    "updated_at_ms": now_ms()
                });
                store
                    .set_kv(&kv_key, &rec)
                    .map_err(|e| ("storage_error", e.to_string()))?;
                rec
            };
            Ok(json!({
                "ok": true,
                "connection_id": conn_id,
                "model_id": model_id,
                "protocol": protocol,
                "override": stored
            }))
        }
        ("model", "refresh_connection_catalog") => {
            let input = payload.get("input").unwrap_or(payload);
            let conn_id = input
                .get("connection_id")
                .or_else(|| input.get("connectionId"))
                .or_else(|| input.get("id"))
                .or_else(|| args.get("connection_id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'connection_id'".into()))?;
            let mut cfg = crate::config::load_active_config(store);
            let mut models: Vec<String> = Vec::new();
            if let Some(conn) = cfg.llm.connections.iter_mut().find(|c| c.id == conn_id) {
                let mut changed = false;
                if let Some(incoming) = input.get("models").and_then(|v| v.as_array()) {
                    for m in incoming {
                        if let Some(mid) = m
                            .as_str()
                            .or_else(|| m.get("id").and_then(|v| v.as_str()))
                            .filter(|s| !s.trim().is_empty())
                        {
                            if !conn.models.iter().any(|e| e.id == mid) {
                                conn.models.push(crate::config::ModelEntry {
                                    id: mid.to_string(),
                                    context_window: None,
                                    max_output_tokens: None,
                                    temperature: None,
                                    top_p: None,
                                    reasoning_effort: None,
                                    capabilities: None,
                                });
                                changed = true;
                            }
                        }
                    }
                }
                models = conn.models.iter().map(|m| m.id.clone()).collect();
                if changed {
                    let _ = crate::config::save_applied_config(store, &cfg);
                }
            } else if let Ok(Some(rec)) = suite.get_record::<Value>("model_connection", conn_id) {
                if let Some(arr) = rec.get("models").and_then(|v| v.as_array()) {
                    models = arr
                        .iter()
                        .filter_map(|m| {
                            m.as_str()
                                .or_else(|| m.get("id").and_then(|v| v.as_str()))
                                .map(ToString::to_string)
                        })
                        .collect();
                }
            }
            let ts = now_ms();
            let _ = store.set_kv(
                &format!("model_catalog_refreshed:{conn_id}"),
                &json!({"connection_id": conn_id, "models": models, "refreshed_at_ms": ts}),
            );
            Ok(json!({
                "ok": true,
                "connection_id": conn_id,
                "models": models,
                "refreshed_at_ms": ts
            }))
        }
        ("model", "configure_connection_models") => {
            let input = payload.get("input").unwrap_or(payload);
            let conn_id = input
                .get("connection_id")
                .or_else(|| input.get("connectionId"))
                .or_else(|| args.get("connection_id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'connection_id'".into()))?;
            let enabled_ids: Vec<String> = input
                .get("enabled_model_ids")
                .or_else(|| input.get("models"))
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(ToString::to_string))
                        .collect()
                })
                .unwrap_or_default();
            let selected_id = input
                .get("selected_model_id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string);
            let mut cfg = crate::config::load_active_config(store);
            if let Some(conn) = cfg.llm.connections.iter_mut().find(|c| c.id == conn_id) {
                for key in &enabled_ids {
                    let mid = key.split_once(':').map(|(_, m)| m).unwrap_or(key.as_str());
                    if !mid.is_empty() && !conn.models.iter().any(|m| m.id == mid) {
                        conn.models.push(crate::config::ModelEntry {
                            id: mid.to_string(),
                            context_window: None,
                            max_output_tokens: None,
                            temperature: None,
                            top_p: None,
                            reasoning_effort: None,
                            capabilities: None,
                        });
                    }
                }
                if let Some(ref sel) = selected_id {
                    let mid = sel.split_once(':').map(|(_, m)| m).unwrap_or(sel.as_str());
                    if !mid.is_empty() {
                        cfg.llm.default_model = Some(crate::config::DefaultModelSelection {
                            connection: conn_id.to_string(),
                            model: mid.to_string(),
                        });
                    }
                }
                let _ = crate::config::save_applied_config(store, &cfg);
            }
            let selection = json!({
                "connection_id": conn_id,
                "enabled": enabled_ids,
                "selected_model_id": selected_id,
                "updated_at_ms": now_ms()
            });
            store
                .set_kv(&format!("model_conn_selections:{conn_id}"), &selection)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({
                "ok": true,
                "connection_id": conn_id,
                "enabled_model_ids": enabled_ids,
                "selected_model_id": selected_id,
                "selection": selection
            }))
        }
        ("model", "import_connection") => {
            let input = payload
                .get("input")
                .or_else(|| payload.get("connection"))
                .unwrap_or(payload);
            let id = input
                .get("id")
                .or_else(|| input.get("connection_id"))
                .or_else(|| payload.get("id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(new_id);
            let mut rec = input.clone();
            if let Some(obj) = rec.as_object_mut() {
                obj.insert("id".into(), json!(id));
                obj.insert("updated_at_ms".into(), json!(now_ms()));
            }
            suite
                .upsert_record("model_connection", &id, &rec)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"ok": true, "id": id, "connection": rec}))
        }
        ("model", "report_runtime") => {
            let input = payload.get("input").unwrap_or(payload);
            let id = input
                .get("runtime_id")
                .or_else(|| input.get("id"))
                .and_then(|v| v.as_str())
                .unwrap_or("default");
            let mut rec = input.clone();
            if let Some(obj) = rec.as_object_mut() {
                obj.insert("reported_at_ms".into(), json!(now_ms()));
            }
            store
                .set_kv(&format!("model_runtime:{id}"), &rec)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"ok": true, "runtime": rec}))
        }
        ("model", "get_runtime") => {
            let id = payload
                .get("runtime_id")
                .or_else(|| payload.get("id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .unwrap_or("default");
            let found = store
                .get_kv(&format!("model_runtime:{id}"))
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(found.unwrap_or(Value::Null))
        }
        ("model", "upsert_profile") => {
            let input = payload.get("input").unwrap_or(payload);
            let record = input
                .get("profile")
                .or_else(|| input.get("record"))
                .unwrap_or(input);
            let id = record
                .get("id")
                .or_else(|| record.get("profile_id"))
                .or_else(|| input.get("id"))
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
            let input = payload.get("input").unwrap_or(payload);
            let id = input
                .get("id")
                .or_else(|| input.get("profile_id"))
                .or_else(|| payload.get("id"))
                .or_else(|| payload.get("profile_id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            let mut rec = suite
                .get_record::<Value>("model", id)
                .map_err(|e| ("storage_error", e.to_string()))?
                .unwrap_or_else(|| json!({"id": id}));
            let (params, params_json_str) = if let Some(pj) = input
                .get("parameters_json")
                .or_else(|| payload.get("parameters_json"))
            {
                if let Some(s) = pj.as_str() {
                    let parsed = serde_json::from_str::<Value>(s).unwrap_or_else(|_| json!({}));
                    (parsed, s.to_string())
                } else {
                    let s = serde_json::to_string(pj).unwrap_or_else(|_| "{}".into());
                    (pj.clone(), s)
                }
            } else {
                let p = input.get("parameters").cloned().unwrap_or_else(|| {
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
                        if let Some(v) = input.get(k) {
                            m.insert(k.to_string(), v.clone());
                        }
                    }
                    Value::Object(m)
                });
                let s = serde_json::to_string(&p).unwrap_or_else(|_| "{}".into());
                (p, s)
            };
            let next_rev = rec.get("revision").and_then(|v| v.as_u64()).unwrap_or(0) + 1;
            if let Some(obj) = rec.as_object_mut() {
                obj.insert("parameters".into(), params);
                obj.insert("parameters_json".into(), json!(params_json_str));
                obj.insert("revision".into(), json!(next_rev));
                obj.insert("updated_at_ms".into(), json!(now_ms()));
            }
            suite
                .upsert_record("model", id, &rec)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({
                "ok": true,
                "id": id,
                "revision": next_rev,
                "parameters_json": params_json_str,
                "profile": rec
            }))
        }
        ("model", "reset_parameters") => {
            let input = payload.get("input").unwrap_or(payload);
            if let Some(ids_arr) = input
                .get("profile_ids")
                .or_else(|| payload.get("profile_ids"))
                .or_else(|| args.get("profile_ids"))
                .and_then(|v| v.as_array())
            {
                let mut updated = Vec::new();
                let mut ids = Vec::new();
                for id_val in ids_arr {
                    if let Some(id) = id_val.as_str().filter(|s| !s.trim().is_empty()) {
                        let mut rec = suite
                            .get_record::<Value>("model", id)
                            .map_err(|e| ("storage_error", e.to_string()))?
                            .unwrap_or_else(|| json!({"id": id}));
                        let next_rev =
                            rec.get("revision").and_then(|v| v.as_u64()).unwrap_or(0) + 1;
                        if let Some(obj) = rec.as_object_mut() {
                            obj.insert("parameters".into(), json!({}));
                            obj.insert("parameters_json".into(), json!("{}"));
                            obj.insert("revision".into(), json!(next_rev));
                            obj.insert("updated_at_ms".into(), json!(now_ms()));
                        }
                        suite
                            .upsert_record("model", id, &rec)
                            .map_err(|e| ("storage_error", e.to_string()))?;
                        ids.push(id.to_string());
                        updated.push(rec);
                    }
                }
                return Ok(json!({"ok": true, "profile_ids": ids, "profiles": updated}));
            }
            let id = input
                .get("id")
                .or_else(|| input.get("profile_id"))
                .or_else(|| payload.get("id"))
                .or_else(|| payload.get("profile_id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            let mut rec = suite
                .get_record::<Value>("model", id)
                .map_err(|e| ("storage_error", e.to_string()))?
                .unwrap_or_else(|| json!({"id": id}));
            let next_rev = rec.get("revision").and_then(|v| v.as_u64()).unwrap_or(0) + 1;
            if let Some(obj) = rec.as_object_mut() {
                obj.insert("parameters".into(), json!({}));
                obj.insert("parameters_json".into(), json!("{}"));
                obj.insert("revision".into(), json!(next_rev));
                obj.insert("updated_at_ms".into(), json!(now_ms()));
            }
            suite
                .upsert_record("model", id, &rec)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({
                "ok": true,
                "id": id,
                "revision": next_rev,
                "parameters_json": "{}",
                "profile": rec
            }))
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
            let mut raw_items = nola_modules::skills::discover_skills_detailed(
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
                    raw_items.iter().any(|x| {
                        x.get("id").and_then(|v| v.as_str()) == Some(id)
                            || x.get("name").and_then(|v| v.as_str()) == Some(id)
                    })
                });
                if !exists {
                    raw_items.push(user_skill);
                }
            }
            let items: Vec<Value> = raw_items
                .iter()
                .map(|raw| canonical_skill_editor_envelope(raw, None, 1))
                .collect();
            Ok(json!({"kind": "skill", "items": items, "skills": items}))
        }
        ("skill", "get" | "get_editor") => {
            let id = payload
                .get("id")
                .or_else(|| payload.get("skill_id"))
                .or_else(|| payload.get("name"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            if let Ok(Some(rec)) = suite.get_record::<Value>("skill", id) {
                return Ok(canonical_skill_editor_envelope(&rec, Some(id), 1));
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
                    || s.get("id")
                        .and_then(|v| v.as_str())
                        .is_some_and(|n| n.eq_ignore_ascii_case(id))
            });
            Ok(found
                .map(|raw| canonical_skill_editor_envelope(&raw, Some(id), 1))
                .unwrap_or(Value::Null))
        }
        ("skill", "upsert" | "create_user" | "configure_user") => {
            let input = payload.get("input").unwrap_or(payload);
            let record = input
                .get("skill")
                .or_else(|| input.get("record"))
                .unwrap_or(input);
            let id = record
                .get("id")
                .or_else(|| input.get("id"))
                .or_else(|| payload.get("id"))
                .or_else(|| record.get("name"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(new_id);
            let prev = suite.get_record::<Value>("skill", &id).ok().flatten();
            let prev_rev = prev
                .as_ref()
                .and_then(|v| v.get("revision").and_then(|r| r.as_u64()))
                .unwrap_or(0);
            let next_rev = prev_rev + 1;
            let ts = now_ms();
            let mut merged = record.clone();
            if let Some(obj) = merged.as_object_mut() {
                obj.insert("id".into(), json!(id));
                if let Some(src) = obj.get("source").and_then(|v| v.as_str()) {
                    if src == "builtin" {
                        obj.insert("source".into(), json!("built_in"));
                    }
                }
                if !obj.contains_key("created_at_ms") {
                    let prev_created = prev
                        .as_ref()
                        .and_then(|v| v.get("created_at_ms").and_then(|c| c.as_i64()))
                        .unwrap_or(ts);
                    obj.insert("created_at_ms".into(), json!(prev_created));
                }
                obj.insert("revision".into(), json!(next_rev));
                obj.insert("updated_at_ms".into(), json!(ts));
            }
            let envelope = canonical_skill_editor_envelope(&merged, Some(&id), next_rev);
            let canonical_rec = envelope.get("record").cloned().unwrap_or(merged.clone());
            if let Some(enabled) = canonical_rec.get("enabled").and_then(|v| v.as_bool()) {
                let skill_name = canonical_rec
                    .get("display_name")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&id);
                let _ = nola_modules::skills::set_skill_enabled(store, &id, enabled);
                if skill_name != id {
                    let _ = nola_modules::skills::set_skill_enabled(store, skill_name, enabled);
                }
            }
            let mut to_store = canonical_rec.clone();
            if let Some(obj) = to_store.as_object_mut() {
                obj.insert("revision".into(), json!(next_rev));
            }
            suite
                .upsert_record("skill", &id, &to_store)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({
                "ok": true,
                "id": id,
                "revision": next_rev,
                "record": canonical_rec,
                "skill": canonical_rec
            }))
        }
        ("skill", "synchronize_built_ins") => {
            let builtins = nola_modules::skills::builtin_skills();
            let count = builtins.len();
            Ok(json!({
                "ok": true,
                "synchronized": count,
                "skills": builtins
            }))
        }
        ("skill", "restore_built_ins") => {
            let builtins = nola_modules::skills::builtin_skills();
            let mut restored = 0usize;
            for b in &builtins {
                let _ = nola_modules::skills::set_skill_enabled(store, &b.name, true);
                let _ = store.set_kv(&format!("skill_builtin_removed:{}", b.name), &json!(false));
                restored += 1;
            }
            Ok(json!({
                "ok": true,
                "restored": restored,
                "skills": builtins
            }))
        }
        ("skill", "remove_built_in") => {
            let input = payload.get("input").unwrap_or(payload);
            let id = input
                .get("id")
                .or_else(|| input.get("name"))
                .or_else(|| payload.get("id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing skill 'id' or 'name'".into()))?;
            let _ = nola_modules::skills::set_skill_enabled(store, id, false);
            let _ = store.set_kv(&format!("skill_builtin_removed:{id}"), &json!(true));
            Ok(json!({"ok": true, "removed": true, "id": id}))
        }
        ("skill", "apply_changes") => {
            let input = payload.get("input").unwrap_or(payload);
            let mut applied = 0usize;
            if let Some(upserts) = input
                .get("upserts")
                .or_else(|| input.get("changes"))
                .and_then(|v| v.as_array())
            {
                for entry in upserts {
                    let sid = entry
                        .get("id")
                        .or_else(|| entry.get("name"))
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.trim().is_empty())
                        .map(ToString::to_string)
                        .unwrap_or_else(new_id);
                    let mut stored = entry.clone();
                    if let Some(obj) = stored.as_object_mut() {
                        obj.insert("id".into(), json!(sid));
                        obj.insert("updated_at_ms".into(), json!(now_ms()));
                    }
                    let _ = suite.upsert_record("skill", &sid, &stored);
                    if let Some(enabled) = stored.get("enabled").and_then(|v| v.as_bool()) {
                        let _ = nola_modules::skills::set_skill_enabled(store, &sid, enabled);
                    }
                    applied += 1;
                }
            }
            if let Some(deletes) = input.get("deletes").and_then(|v| v.as_array()) {
                for d in deletes {
                    if let Some(sid) = d.as_str() {
                        let _ = suite.delete_record("skill", sid);
                        applied += 1;
                    }
                }
            }
            let rev = store
                .get_kv("skills_revision")
                .ok()
                .flatten()
                .and_then(|v| v.as_u64())
                .unwrap_or(0)
                + 1;
            let _ = store.set_kv("skills_revision", &json!(rev));
            Ok(json!({"ok": true, "applied": applied, "revision": rev}))
        }
        ("skill", "set_enabled_checked") => {
            let input = payload.get("input").unwrap_or(payload);
            let id = input
                .get("id")
                .or_else(|| input.get("name"))
                .or_else(|| payload.get("id"))
                .or_else(|| payload.get("name"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing skill 'id' or 'name'".into()))?;
            let enabled = input
                .get("enabled")
                .or_else(|| payload.get("enabled"))
                .or_else(|| args.get("enabled"))
                .and_then(|v| v.as_bool())
                .ok_or(("invalid_argument", "missing boolean 'enabled'".into()))?;
            nola_modules::skills::set_skill_enabled(store, id, enabled)
                .map_err(|e| ("storage_error", e.to_string()))?;
            let base = if let Ok(Some(rec)) = suite.get_record::<Value>("skill", id) {
                rec
            } else {
                let detailed = nola_modules::skills::discover_skills_detailed(
                    Some(store),
                    &suite.workspace_root,
                    suite.user_home.as_deref(),
                );
                detailed
                    .into_iter()
                    .find(|s| {
                        s.get("name")
                            .and_then(|v| v.as_str())
                            .is_some_and(|n| n.eq_ignore_ascii_case(id))
                            || s.get("id")
                                .and_then(|v| v.as_str())
                                .is_some_and(|n| n.eq_ignore_ascii_case(id))
                    })
                    .unwrap_or_else(|| json!({"id": id, "source": "user"}))
            };
            let next_rev = base
                .get("revision")
                .and_then(|r| r.as_u64())
                .unwrap_or(0)
                + 1;
            let mut updated = base;
            if let Some(obj) = updated.as_object_mut() {
                obj.insert("id".into(), json!(id));
                obj.insert("enabled".into(), json!(enabled));
                obj.insert("revision".into(), json!(next_rev));
                obj.insert("updated_at_ms".into(), json!(now_ms()));
            }
            let envelope = canonical_skill_editor_envelope(&updated, Some(id), next_rev);
            let canonical_rec = envelope.get("record").cloned().unwrap_or(updated.clone());
            let mut to_store = canonical_rec.clone();
            if let Some(obj) = to_store.as_object_mut() {
                obj.insert("revision".into(), json!(next_rev));
            }
            let _ = suite.upsert_record("skill", id, &to_store);
            Ok(json!({
                "ok": true,
                "id": id,
                "enabled": enabled,
                "revision": next_rev,
                "record": canonical_rec
            }))
        }
        ("skill", "delete_user_checked" | "delete_user" | "delete") => {
            let input = payload.get("input").unwrap_or(payload);
            let id = input
                .get("id")
                .or_else(|| payload.get("id"))
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
            let input = payload.get("input").unwrap_or(payload);
            let q = input
                .get("query")
                .or_else(|| payload.get("query"))
                .or_else(|| args.get("query"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let ns = input
                .get("namespace")
                .or_else(|| payload.get("namespace"))
                .or_else(|| args.get("namespace"))
                .and_then(|v| v.as_str());
            let limit = input
                .get("limit")
                .or_else(|| payload.get("limit"))
                .or_else(|| args.get("limit"))
                .and_then(|v| v.as_u64())
                .unwrap_or(100) as usize;
            let items = store
                .search_memory(q, ns, limit)
                .map_err(|e| ("storage_error", e.to_string()))?;
            let records: Vec<Value> = items.iter().map(canonical_memory_envelope).collect();
            Ok(json!({"items": items, "records": records, "memories": records}))
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
                .map(|m| canonical_memory_envelope(&m));
            Ok(found.unwrap_or(Value::Null))
        }
        ("memory", "commit" | "save" | "upsert" | "update_if_revision") => {
            let item = parse_memory_item(payload);
            store
                .upsert_memory(&item)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(canonical_memory_envelope(&item))
        }
        ("memory", "replace") => {
            let input = payload.get("input").unwrap_or(payload);
            if let Some(arr) = input
                .get("candidates")
                .or_else(|| input.get("records"))
                .or_else(|| input.get("items"))
                .or_else(|| payload.get("candidates"))
                .or_else(|| payload.get("records"))
                .or_else(|| payload.get("items"))
                .and_then(|v| v.as_array())
            {
                let parsed_items: Vec<MemoryItem> = arr.iter().map(parse_memory_item).collect();
                let incoming_ids: HashSet<&str> =
                    parsed_items.iter().map(|m| m.id.as_str()).collect();
                let existing = store
                    .search_memory("", None, 10_000)
                    .map_err(|e| ("storage_error", e.to_string()))?;
                for old in existing {
                    if !incoming_ids.contains(old.id.as_str()) {
                        store
                            .delete_memory(&old.id)
                            .map_err(|e| ("storage_error", e.to_string()))?;
                    }
                }
                let mut records = Vec::with_capacity(parsed_items.len());
                for item in &parsed_items {
                    store
                        .upsert_memory(item)
                        .map_err(|e| ("storage_error", e.to_string()))?;
                    records.push(canonical_memory_envelope(item));
                }
                return Ok(Value::Array(records));
            }
            let item = parse_memory_item(input);
            store
                .upsert_memory(&item)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(Value::Array(vec![canonical_memory_envelope(&item)]))
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
            let enriched: Vec<Value> = servers
                .into_iter()
                .map(|s| enrich_mcp_server_json(store, &s))
                .collect();
            Ok(json!({"kind": "mcp", "items": enriched, "servers": enriched}))
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
                .map(|s| enrich_mcp_server_json(store, &s))
                .unwrap_or(Value::Null))
        }
        ("mcp", "create" | "configure" | "upsert") => {
            let input = payload.get("input").unwrap_or(payload);
            let cfg = parse_mcp_server_config(payload)?;
            store
                .upsert_mcp_server(&cfg)
                .map_err(|e| ("storage_error", e.to_string()))?;
            let prev_meta = store
                .get_kv(&format!("mcp_meta:{}", cfg.id))
                .ok()
                .flatten()
                .unwrap_or_else(|| json!({}));
            let next_rev = prev_meta
                .get("revision")
                .and_then(|r| r.as_u64())
                .unwrap_or(0)
                + 1;
            let ts = now_ms();
            let created_at_ms = prev_meta
                .get("created_at_ms")
                .and_then(|v| v.as_i64())
                .or_else(|| input.get("created_at_ms").and_then(|v| v.as_i64()))
                .unwrap_or(ts);
            let updated_at_ms = input
                .get("updated_at_ms")
                .and_then(|v| v.as_i64())
                .unwrap_or(ts);
            let display_name = input
                .get("display_name")
                .or_else(|| input.get("name"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .or_else(|| {
                    prev_meta
                        .get("display_name")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string)
                })
                .unwrap_or_else(|| cfg.name.clone());
            let configuration_json = input
                .get("configuration_json")
                .or_else(|| payload.get("configuration_json"))
                .map(|v| {
                    v.as_str()
                        .map(ToString::to_string)
                        .unwrap_or_else(|| v.to_string())
                })
                .or_else(|| {
                    prev_meta
                        .get("configuration_json")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string)
                })
                .unwrap_or_else(|| "{}".into());
            let _ = store.set_kv(
                &format!("mcp_meta:{}", cfg.id),
                &json!({
                    "display_name": display_name,
                    "enabled": cfg.enabled,
                    "configuration_json": configuration_json,
                    "revision": next_rev,
                    "created_at_ms": created_at_ms,
                    "updated_at_ms": updated_at_ms
                }),
            );
            let enriched = enrich_mcp_server_json(store, &cfg);
            Ok(json!({
                "ok": true,
                "id": cfg.id,
                "display_name": display_name,
                "enabled": cfg.enabled,
                "configuration_json": configuration_json,
                "revision": next_rev,
                "created_at_ms": created_at_ms,
                "updated_at_ms": updated_at_ms,
                "server": enriched
            }))
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
                .or_else(|| payload.get("data_source_id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            let found = suite
                .get_record::<Value>("data_source", id)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(found.unwrap_or(Value::Null))
        }
        ("data_source", "create_draft" | "create") => {
            let input = payload.get("input").unwrap_or(payload);
            let record = input.get("record").unwrap_or(input);
            let id = record
                .get("id")
                .or_else(|| input.get("id"))
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
            let input = payload.get("input").unwrap_or(payload);
            let record = input
                .get("configuration")
                .or_else(|| input.get("record"))
                .unwrap_or(input);
            let id = input
                .get("id")
                .or_else(|| input.get("data_source_id"))
                .or_else(|| record.get("id"))
                .or_else(|| payload.get("id"))
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
        ("data_source", "request_validation") => {
            let input = payload.get("input").unwrap_or(payload);
            let id = input
                .get("data_source_id")
                .or_else(|| input.get("id"))
                .or_else(|| payload.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            let ts = now_ms();
            let state = json!({"id": id, "status": "pending", "requested_at_ms": ts});
            let _ = store.set_kv(&format!("ds_validation:{id}"), &state);
            Ok(json!({"ok": true, "id": id, "status": "pending", "requested_at_ms": ts}))
        }
        ("data_source", "record_validation") => {
            let input = payload.get("input").unwrap_or(payload);
            let id = input
                .get("data_source_id")
                .or_else(|| input.get("id"))
                .or_else(|| payload.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            let ts = now_ms();
            let mut val = input.clone();
            if let Some(obj) = val.as_object_mut() {
                obj.insert("id".into(), json!(id));
                obj.insert("checked_at_ms".into(), json!(ts));
            }
            let _ = store.set_kv(&format!("ds_validation:{id}"), &val);
            if let Ok(Some(mut rec)) = suite.get_record::<Value>("data_source", id) {
                if let Some(obj) = rec.as_object_mut() {
                    obj.insert("validation".into(), val.clone());
                    obj.insert("updated_at_ms".into(), json!(ts));
                }
                let _ = suite.upsert_record("data_source", id, &rec);
            }
            Ok(json!({"ok": true, "id": id, "validation": val}))
        }
        ("data_source", "set_default_enabled") => {
            let input = payload.get("input").unwrap_or(payload);
            let id = input
                .get("data_source_id")
                .or_else(|| input.get("id"))
                .or_else(|| payload.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            let enabled = input
                .get("default_enabled")
                .or_else(|| input.get("enabled"))
                .and_then(|v| v.as_bool())
                .ok_or(("invalid_argument", "missing boolean 'enabled'".into()))?;
            let mut rec = suite
                .get_record::<Value>("data_source", id)
                .map_err(|e| ("storage_error", e.to_string()))?
                .unwrap_or_else(|| json!({"id": id}));
            if let Some(obj) = rec.as_object_mut() {
                obj.insert("default_enabled".into(), json!(enabled));
                obj.insert("enabled".into(), json!(enabled));
                obj.insert("updated_at_ms".into(), json!(now_ms()));
            }
            suite
                .upsert_record("data_source", id, &rec)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"ok": true, "id": id, "default_enabled": enabled, "record": rec}))
        }
        ("data_source", "get_conversation_override") => {
            let input = payload.get("input").unwrap_or(payload);
            let conv_id = input
                .get("conversation_id")
                .or_else(|| args.get("conversation_id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'conversation_id'".into()))?;
            let ds_id = input
                .get("source_id")
                .or_else(|| input.get("data_source_id"))
                .or_else(|| input.get("id"))
                .or_else(|| args.get("source_id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'source_id'".into()))?;
            let val = store
                .get_kv(&format!("ds_override:{conv_id}:{ds_id}"))
                .map_err(|e| ("storage_error", e.to_string()))?
                .unwrap_or_else(|| json!("inherit"));
            Ok(val)
        }
        ("data_source", "set_conversation_override") => {
            let input = payload.get("input").unwrap_or(payload);
            let conv_id = input
                .get("conversation_id")
                .or_else(|| args.get("conversation_id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'conversation_id'".into()))?;
            let ds_id = input
                .get("source_id")
                .or_else(|| input.get("data_source_id"))
                .or_else(|| input.get("id"))
                .or_else(|| args.get("source_id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'source_id'".into()))?;
            let override_val = input
                .get("value")
                .or_else(|| input.get("override"))
                .or_else(|| input.get("enabled"))
                .cloned()
                .unwrap_or_else(|| json!("inherit"));
            let key = format!("ds_override:{conv_id}:{ds_id}");
            store
                .set_kv(&key, &override_val)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!(true))
        }
        ("data_source", "record_usage") => {
            let input = payload.get("input").unwrap_or(payload);
            let source_id = input
                .get("source_id")
                .or_else(|| input.get("data_source_id"))
                .or_else(|| input.get("id"))
                .and_then(|v| v.as_str())
                .unwrap_or("default");
            let conv_id = input
                .get("conversation_id")
                .or_else(|| args.get("conversation_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let summary = input
                .get("summary")
                .or_else(|| input.get("purpose"))
                .or_else(|| input.get("source_title"))
                .and_then(|v| v.as_str())
                .unwrap_or("DataSource used");
            let mut recorded = nola_modules::data_sources::record_data_source_usage(
                store, conv_id, source_id, summary,
            )
            .map_err(|e| ("storage_error", e))?;
            if let Some(obj) = recorded.as_object_mut() {
                obj.insert("ok".into(), json!(true));
            }
            Ok(recorded)
        }
        ("data_source", "request_refresh") => {
            let input = payload.get("input").unwrap_or(payload);
            let id = input
                .get("data_source_id")
                .or_else(|| input.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            let ts = now_ms();
            let _ = store.set_kv(
                &format!("ds_refresh:{id}"),
                &json!({"id": id, "status": "refresh_requested", "requested_at_ms": ts}),
            );
            Ok(json!({"ok": true, "id": id, "status": "refresh_requested", "requested_at_ms": ts}))
        }
        ("data_source", "record_read") => {
            let input = payload.get("input").unwrap_or(payload);
            let id = input
                .get("data_source_id")
                .or_else(|| input.get("id"))
                .and_then(|v| v.as_str())
                .unwrap_or("default");
            let ts = now_ms();
            let _ = store.set_kv(
                &format!("ds_last_read:{id}"),
                &json!({"id": id, "recorded_at_ms": ts, "details": input}),
            );
            Ok(json!({"ok": true, "id": id, "recorded_at_ms": ts}))
        }
        ("data_source", "resolve_for_conversation") => {
            let input = payload.get("input").unwrap_or(payload);
            let conv_id = input
                .get("conversation_id")
                .or_else(|| args.get("conversation_id"))
                .and_then(|v| v.as_str());
            let all: Vec<Value> = suite
                .list_records("data_source")
                .map_err(|e| ("storage_error", e.to_string()))?;
            let mut selections = Vec::with_capacity(all.len());
            for rec in all {
                let ds_id = rec
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let source_revision = rec
                    .get("revision")
                    .or_else(|| rec.get("source_revision"))
                    .and_then(|v| v.as_i64())
                    .unwrap_or(1)
                    .max(1);
                let default_enabled = rec
                    .get("default_enabled")
                    .or_else(|| rec.get("enabled"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true);
                let (effective, reason) = if let Some(cid) = conv_id {
                    match store
                        .get_kv(&format!("ds_override:{cid}:{ds_id}"))
                        .ok()
                        .flatten()
                    {
                        Some(Value::Bool(b)) => (b, if b { "override_enabled" } else { "override_disabled" }),
                        Some(Value::String(ref s)) if s.eq_ignore_ascii_case("enabled") => {
                            (true, "override_enabled")
                        }
                        Some(Value::String(ref s)) if s.eq_ignore_ascii_case("disabled") => {
                            (false, "override_disabled")
                        }
                        Some(ref o) if o.get("enabled").and_then(|v| v.as_bool()).is_some() => {
                            let b = o
                                .get("enabled")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(default_enabled);
                            (b, if b { "override_enabled" } else { "override_disabled" })
                        }
                        _ => (
                            default_enabled,
                            if default_enabled { "default_enabled" } else { "default_disabled" },
                        ),
                    }
                } else {
                    (
                        default_enabled,
                        if default_enabled { "default_enabled" } else { "default_disabled" },
                    )
                };
                let content_reference = rec
                    .get("content_reference")
                    .cloned()
                    .unwrap_or(Value::Null);
                let observed_at_ms = rec
                    .get("observed_at_ms")
                    .or_else(|| rec.get("updated_at_ms"))
                    .and_then(|v| v.as_i64());
                let schema_version = rec
                    .get("schema_version")
                    .and_then(|v| v.as_i64())
                    .map(|n| n as i32);
                selections.push(json!({
                    "source_id": ds_id,
                    "source_revision": source_revision,
                    "enabled": effective,
                    "usable": effective,
                    "should_refresh": false,
                    "reason": reason,
                    "content_reference": content_reference,
                    "observed_at_ms": observed_at_ms,
                    "schema_version": schema_version
                }));
            }
            Ok(Value::Array(selections))
        }
        ("data_source", "resolve_sources") => {
            let input = payload.get("input").unwrap_or(payload);
            let conv_id = input
                .get("conversation_id")
                .or_else(|| args.get("conversation_id"))
                .and_then(|v| v.as_str());
            let all: Vec<Value> = suite
                .list_records("data_source")
                .map_err(|e| ("storage_error", e.to_string()))?;
            let mut resolved = Vec::new();
            for mut rec in all {
                let ds_id = rec
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let default_enabled = rec
                    .get("default_enabled")
                    .or_else(|| rec.get("enabled"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true);
                let effective = if let Some(cid) = conv_id {
                    match store.get_kv(&format!("ds_override:{cid}:{ds_id}")).ok().flatten() {
                        Some(Value::Bool(b)) => b,
                        Some(Value::String(ref s)) if s.eq_ignore_ascii_case("enabled") => true,
                        Some(Value::String(ref s)) if s.eq_ignore_ascii_case("disabled") => false,
                        Some(ref o) if o.get("enabled").and_then(|v| v.as_bool()).is_some() => {
                            o.get("enabled").and_then(|v| v.as_bool()).unwrap_or(default_enabled)
                        }
                        _ => default_enabled,
                    }
                } else {
                    default_enabled
                };
                if let Some(obj) = rec.as_object_mut() {
                    obj.insert("effective_enabled".into(), json!(effective));
                }
                resolved.push(rec);
            }
            Ok(json!({
                "conversation_id": conv_id,
                "sources": resolved,
                "items": resolved
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
        ("automation", "definition") => {
            let (sub_op, sub_payload) = extract_sub_command(payload);
            dispatch_automation_definition(suite, sub_op, sub_payload, args)
        }
        ("automation", "run") => {
            let (sub_op, sub_payload) = extract_sub_command(payload);
            dispatch_automation_run(suite, sub_op, sub_payload, args)
        }
        ("automation", "list" | "get" | "create" | "upsert" | "update" | "update_if_revision" | "pause" | "resume" | "delete") => {
            dispatch_automation_definition(suite, op, payload, args)
        }
        ("automation", "enqueue" | "claim" | "heartbeat" | "retry" | "load") => {
            dispatch_automation_run(suite, op, payload, args)
        }

        _ => Err((
            "unsupported_command",
            format!("unsupported {domain} operation '{op}'"),
        )),
    }
}

fn enrich_mcp_server_json(store: &Store, cfg: &McpServerConfig) -> Value {
    let mut val = serde_json::to_value(cfg).unwrap_or_else(|_| json!({}));
    if let Some(obj) = val.as_object_mut() {
        let meta = store
            .get_kv(&format!("mcp_meta:{}", cfg.id))
            .ok()
            .flatten()
            .unwrap_or_else(|| json!({}));
        let display_name = meta
            .get("display_name")
            .and_then(|v| v.as_str())
            .unwrap_or(&cfg.name);
        let revision = meta.get("revision").and_then(|r| r.as_u64()).unwrap_or(1);
        let configuration_json = meta
            .get("configuration_json")
            .and_then(|v| v.as_str())
            .unwrap_or("{}");
        let updated_at_ms = meta
            .get("updated_at_ms")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let created_at_ms = meta
            .get("created_at_ms")
            .and_then(|v| v.as_i64())
            .unwrap_or(updated_at_ms);
        obj.insert("display_name".into(), json!(display_name));
        obj.insert("enabled".into(), json!(cfg.enabled));
        obj.insert("revision".into(), json!(revision));
        obj.insert("configuration_json".into(), json!(configuration_json));
        obj.insert("created_at_ms".into(), json!(created_at_ms));
        obj.insert("updated_at_ms".into(), json!(updated_at_ms));
    }
    val
}

fn dispatch_automation_definition(
    suite: &ModulesSuite,
    op: &str,
    payload: &Value,
    args: &Value,
) -> Result<Value, (&'static str, String)> {
    let input = payload.get("input").unwrap_or(payload);
    match op {
        "list" => {
            let items: Vec<Value> = suite
                .list_records("automation")
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"kind": "automation", "items": items, "definitions": items}))
        }
        "get" => {
            let id = input
                .get("id")
                .or_else(|| input.get("definition_id"))
                .or_else(|| payload.get("id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            let found = suite
                .get_record::<Value>("automation", id)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(found.unwrap_or(Value::Null))
        }
        "create" | "upsert" => {
            let record = input
                .get("definition")
                .or_else(|| input.get("record"))
                .unwrap_or(input);
            let id = record
                .get("id")
                .or_else(|| input.get("id"))
                .or_else(|| payload.get("id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(new_id);
            let existing = suite
                .get_record::<Value>("automation", &id)
                .ok()
                .flatten();
            let prev_rev = existing
                .as_ref()
                .and_then(|v| v.get("revision").and_then(|r| r.as_u64()))
                .unwrap_or(0);
            let incoming_rev = record.get("revision").and_then(|r| r.as_u64());
            let next_rev = match incoming_rev {
                Some(r) if r > prev_rev => r,
                _ => prev_rev + 1,
            };
            let ts = now_ms();
            let mut stored = record.clone();
            if let Some(obj) = stored.as_object_mut() {
                obj.insert("id".into(), json!(id));
                obj.insert("revision".into(), json!(next_rev));
                if !obj.contains_key("created_at_ms") {
                    let prev_created = existing
                        .as_ref()
                        .and_then(|v| v.get("created_at_ms").and_then(|c| c.as_i64()))
                        .unwrap_or(ts);
                    obj.insert("created_at_ms".into(), json!(prev_created));
                }
                if !obj.contains_key("updated_at_ms") {
                    obj.insert("updated_at_ms".into(), json!(ts));
                }
            }
            suite
                .upsert_record("automation", &id, &stored)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(stored)
        }
        "update" | "update_if_revision" => {
            let record = input
                .get("definition")
                .or_else(|| input.get("record"))
                .unwrap_or(input);
            let id = input
                .get("id")
                .or_else(|| input.get("definition_id"))
                .or_else(|| record.get("id"))
                .or_else(|| payload.get("id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            let existing = suite
                .get_record::<Value>("automation", id)
                .map_err(|e| ("storage_error", e.to_string()))?;
            let cur_rev = existing
                .as_ref()
                .and_then(|v| v.get("revision").and_then(|r| r.as_u64()))
                .unwrap_or(if existing.is_some() { 1 } else { 0 });
            if let Some(exp) = input
                .get("expected_revision")
                .or_else(|| payload.get("expected_revision"))
                .and_then(|v| v.as_u64())
            {
                if cur_rev != exp {
                    return Err((
                        "revision_conflict",
                        format!("automation '{id}' revision conflict: expected {exp}, actual {cur_rev}"),
                    ));
                }
            }
            let prev_created = existing
                .as_ref()
                .and_then(|v| v.get("created_at_ms").and_then(|c| c.as_i64()));
            let mut merged = existing.unwrap_or_else(|| json!({"id": id}));
            if let (Some(dst), Some(src)) = (merged.as_object_mut(), record.as_object()) {
                for (k, v) in src {
                    if k != "type" {
                        dst.insert(k.clone(), v.clone());
                    }
                }
                dst.insert("id".into(), json!(id));
                let next_rev = match src.get("revision").and_then(|r| r.as_u64()) {
                    Some(r) if r > cur_rev => r,
                    _ => cur_rev + 1,
                };
                dst.insert("revision".into(), json!(next_rev));
                if !dst.contains_key("created_at_ms") {
                    dst.insert("created_at_ms".into(), json!(prev_created.unwrap_or_else(now_ms)));
                }
                if !src.contains_key("updated_at_ms") {
                    dst.insert("updated_at_ms".into(), json!(now_ms()));
                }
            }
            suite
                .upsert_record("automation", id, &merged)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(merged)
        }
        "pause" | "resume" => {
            let id = input
                .get("id")
                .or_else(|| input.get("definition_id"))
                .or_else(|| payload.get("id"))
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
        "delete" => {
            let id = input
                .get("id")
                .or_else(|| input.get("definition_id"))
                .or_else(|| payload.get("id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            let deleted = suite
                .delete_record("automation", id)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"ok": true, "deleted": deleted, "id": id}))
        }
        other => Err((
            "unsupported_command",
            format!("unsupported automation definition operation '{other}'"),
        )),
    }
}

fn dispatch_automation_run(
    suite: &ModulesSuite,
    op: &str,
    payload: &Value,
    args: &Value,
) -> Result<Value, (&'static str, String)> {
    let input = payload.get("input").unwrap_or(payload);
    match op {
        "enqueue" => {
            let id = input
                .get("run_id")
                .or_else(|| input.get("id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(new_id);
            let ts = now_ms();
            let mut rec = input.clone();
            if let Some(obj) = rec.as_object_mut() {
                obj.insert("id".into(), json!(id));
                obj.insert("run_id".into(), json!(id));
                obj.insert("status".into(), json!("queued"));
                obj.entry("attempts").or_insert_with(|| json!(0));
                obj.insert("created_at_ms".into(), json!(ts));
                obj.insert("updated_at_ms".into(), json!(ts));
            }
            suite
                .upsert_record("automation_run", &id, &rec)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"ok": true, "id": id, "run": rec}))
        }
        "claim" => {
            let worker_id = input
                .get("worker_id")
                .and_then(|v| v.as_str())
                .unwrap_or("local-worker");
            let target_id = input
                .get("run_id")
                .or_else(|| input.get("id"))
                .and_then(|v| v.as_str());
            let runs: Vec<Value> = suite
                .list_records("automation_run")
                .map_err(|e| ("storage_error", e.to_string()))?;
            let candidate = runs.into_iter().find(|r| {
                let status_ok = matches!(
                    r.get("status").and_then(|v| v.as_str()),
                    Some("queued" | "pending")
                );
                status_ok
                    && target_id
                        .is_none_or(|tid| r.get("id").and_then(|v| v.as_str()) == Some(tid))
            });
            let Some(mut rec) = candidate else {
                return Ok(json!({"ok": true, "claimed": false, "run": Value::Null}));
            };
            let rid = rec
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let ts = now_ms();
            if let Some(obj) = rec.as_object_mut() {
                obj.insert("status".into(), json!("running"));
                obj.insert("worker_id".into(), json!(worker_id));
                obj.insert("claimed_at_ms".into(), json!(ts));
                obj.insert("last_heartbeat_ms".into(), json!(ts));
                obj.insert("updated_at_ms".into(), json!(ts));
            }
            suite
                .upsert_record("automation_run", &rid, &rec)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"ok": true, "claimed": true, "run": rec}))
        }
        "heartbeat" => {
            let id = input
                .get("run_id")
                .or_else(|| input.get("id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'run_id'".into()))?;
            let mut rec = suite
                .get_record::<Value>("automation_run", id)
                .map_err(|e| ("storage_error", e.to_string()))?
                .ok_or(("not_found", format!("automation run '{id}' not found")))?;
            let ts = now_ms();
            if let Some(obj) = rec.as_object_mut() {
                obj.insert("last_heartbeat_ms".into(), json!(ts));
                obj.insert("updated_at_ms".into(), json!(ts));
            }
            suite
                .upsert_record("automation_run", id, &rec)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"ok": true, "id": id, "run": rec}))
        }
        "retry" => {
            let id = input
                .get("run_id")
                .or_else(|| input.get("id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'run_id'".into()))?;
            let mut rec = suite
                .get_record::<Value>("automation_run", id)
                .map_err(|e| ("storage_error", e.to_string()))?
                .ok_or(("not_found", format!("automation run '{id}' not found")))?;
            let attempts = rec.get("attempts").and_then(|a| a.as_u64()).unwrap_or(0) + 1;
            let ts = now_ms();
            if let Some(obj) = rec.as_object_mut() {
                obj.insert("status".into(), json!("queued"));
                obj.insert("attempts".into(), json!(attempts));
                obj.insert("updated_at_ms".into(), json!(ts));
            }
            suite
                .upsert_record("automation_run", id, &rec)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"ok": true, "id": id, "run": rec}))
        }
        "load" | "get" => {
            let id = input
                .get("run_id")
                .or_else(|| input.get("id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'run_id'".into()))?;
            let found = suite
                .get_record::<Value>("automation_run", id)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(found.unwrap_or(Value::Null))
        }
        "list" => {
            let items: Vec<Value> = suite
                .list_records("automation_run")
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"runs": items, "items": items}))
        }
        "delete" => {
            let id = input
                .get("run_id")
                .or_else(|| input.get("id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'run_id'".into()))?;
            let deleted = suite
                .delete_record("automation_run", id)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"ok": true, "deleted": deleted, "id": id}))
        }
        other => Err((
            "unsupported_command",
            format!("unsupported automation run operation '{other}'"),
        )),
    }
}

pub(crate) fn dispatch_agent_room_command(
    store: &Store,
    suite: &ModulesSuite,
    args: &Value,
) -> Result<Value, (&'static str, String)> {
    let (op, payload) = extract_sub_command(args);
    let input = payload.get("input").unwrap_or(payload);
    match op {
        "list" | "list_rooms" | "directory" => {
            let rooms: Vec<AgentRoom> = suite
                .list_records("agent_room")
                .map_err(|e| ("storage_error", e.to_string()))?;
            let experts: Vec<ExpertPeer> = suite
                .list_records("expert_peer")
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({
                "rooms": rooms,
                "entries": rooms,
                "experts": experts,
                "role_templates": built_in_role_templates()
            }))
        }
        "for_conversation" => {
            let conv_id = input
                .get("conversation_id")
                .or_else(|| args.get("conversation_id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'conversation_id'".into()))?;
            let rooms: Vec<AgentRoom> = suite
                .list_records("agent_room")
                .map_err(|e| ("storage_error", e.to_string()))?;
            let mapped_id = store
                .get_kv(&format!("conv_room:{conv_id}"))
                .ok()
                .flatten()
                .and_then(|v| v.as_str().map(ToString::to_string));
            let found = rooms.into_iter().find(|r| {
                r.session_id == conv_id
                    || r.id == conv_id
                    || mapped_id.as_deref() == Some(r.id.as_str())
            });
            Ok(json!({
                "conversation_id": conv_id,
                "room": found
            }))
        }
        "ensure_for_conversation" => {
            let conv_id = input
                .get("conversation_id")
                .or_else(|| args.get("conversation_id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'conversation_id'".into()))?;
            let rooms: Vec<AgentRoom> = suite
                .list_records("agent_room")
                .map_err(|e| ("storage_error", e.to_string()))?;
            let mapped_id = store
                .get_kv(&format!("conv_room:{conv_id}"))
                .ok()
                .flatten()
                .and_then(|v| v.as_str().map(ToString::to_string));
            if let Some(existing) = rooms.into_iter().find(|r| {
                r.session_id == conv_id
                    || r.id == conv_id
                    || mapped_id.as_deref() == Some(r.id.as_str())
            }) {
                return Ok(json!({"ok": true, "created": false, "room": existing}));
            }
            let ts = now_ms();
            let room_id = format!("room_{conv_id}");
            let room = AgentRoom {
                id: room_id.clone(),
                name: input
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Conversation Room")
                    .to_string(),
                workspace_root: suite.workspace_root.display().to_string(),
                participant_expert_ids: Vec::new(),
                session_id: conv_id.to_string(),
                created_at_ms: ts,
                updated_at_ms: ts,
            };
            suite
                .upsert_record("agent_room", &room.id, &room)
                .map_err(|e| ("storage_error", e.to_string()))?;
            let _ = store.set_kv(&format!("conv_room:{conv_id}"), &json!(room_id));
            Ok(json!({"ok": true, "created": true, "room": room}))
        }
        "child_conversations" => {
            let conv_id = input
                .get("conversation_id")
                .or_else(|| input.get("room_id"))
                .or_else(|| args.get("conversation_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let prefix = format!("{conv_id}:");
            let sessions = store
                .list_sessions(200)
                .map_err(|e| ("storage_error", e.to_string()))?;
            let children: Vec<Value> = sessions
                .iter()
                .filter(|s| !conv_id.is_empty() && s.id.starts_with(&prefix))
                .map(|s| crate::web_api::session_to_conversation_json(store, s))
                .collect();
            Ok(json!({
                "conversation_id": conv_id,
                "conversations": children,
                "items": children
            }))
        }
        "child_history" => {
            let cid = input
                .get("child_conversation_id")
                .or_else(|| input.get("conversation_id"))
                .or_else(|| args.get("conversation_id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'conversation_id'".into()))?;
            let (messages, events) = crate::web_api::reconstruct_v2_messages(store, cid);
            Ok(json!({
                "conversation_id": cid,
                "messages": messages,
                "events": events
            }))
        }
        "peer_exchanges" => {
            let rid = input
                .get("room_id")
                .or_else(|| input.get("conversation_id"))
                .or_else(|| input.get("id"))
                .or_else(|| args.get("room_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let exchanges = store
                .get_kv(&format!("room_exchanges:{rid}"))
                .ok()
                .flatten()
                .and_then(|v| v.as_array().cloned())
                .unwrap_or_default();
            Ok(json!({
                "room_id": rid,
                "exchanges": exchanges,
                "items": exchanges
            }))
        }
        "configure_agent" => {
            let id = input
                .get("expert_id")
                .or_else(|| input.get("agent_id"))
                .or_else(|| input.get("id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(new_id);
            let prev = suite
                .get_record::<ExpertPeer>("expert_peer", &id)
                .ok()
                .flatten();
            let expert = ExpertPeer {
                id: id.clone(),
                name: input
                    .get("name")
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string)
                    .or_else(|| prev.as_ref().map(|e| e.name.clone()))
                    .unwrap_or_else(|| "Expert".into()),
                role: input
                    .get("role")
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string)
                    .or_else(|| prev.as_ref().map(|e| e.role.clone()))
                    .unwrap_or_else(|| "specialist".into()),
                system_prompt: input
                    .get("system_prompt")
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string)
                    .or_else(|| prev.as_ref().map(|e| e.system_prompt.clone()))
                    .unwrap_or_default(),
                model_override: input
                    .get("model_override")
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string)
                    .or_else(|| prev.as_ref().and_then(|e| e.model_override.clone())),
                allowed_tools: input
                    .get("allowed_tools")
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(ToString::to_string))
                            .collect()
                    })
                    .or_else(|| prev.map(|e| e.allowed_tools))
                    .unwrap_or_default(),
                updated_at_ms: now_ms(),
            };
            suite
                .upsert_record("expert_peer", &expert.id, &expert)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"ok": true, "expert": expert}))
        }
        "join" | "rejoin" | "deactivate" => {
            let room_id = input
                .get("room_id")
                .or_else(|| input.get("id"))
                .or_else(|| args.get("room_id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'room_id'".into()))?;
            let expert_id = input
                .get("expert_id")
                .or_else(|| input.get("agent_id"))
                .or_else(|| input.get("participant_id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'expert_id'".into()))?;
            let mut room = suite
                .get_record::<AgentRoom>("agent_room", room_id)
                .map_err(|e| ("storage_error", e.to_string()))?
                .ok_or(("not_found", format!("agent_room '{room_id}' not found")))?;
            let active = op != "deactivate";
            if active {
                if !room.participant_expert_ids.iter().any(|x| x == expert_id) {
                    room.participant_expert_ids.push(expert_id.to_string());
                }
            } else {
                room.participant_expert_ids.retain(|x| x != expert_id);
            }
            room.updated_at_ms = now_ms();
            suite
                .upsert_record("agent_room", &room.id, &room)
                .map_err(|e| ("storage_error", e.to_string()))?;
            let _ = store.set_kv(
                &format!("room_member_state:{room_id}:{expert_id}"),
                &json!({"active": active, "updated_at_ms": room.updated_at_ms}),
            );
            Ok(json!({
                "ok": true,
                "room": room,
                "expert_id": expert_id,
                "active": active
            }))
        }
        "read" => {
            let room_id = input
                .get("room_id")
                .or_else(|| input.get("id"))
                .or_else(|| args.get("room_id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'room_id'".into()))?;
            let room = suite
                .get_record::<AgentRoom>("agent_room", room_id)
                .map_err(|e| ("storage_error", e.to_string()))?
                .ok_or(("not_found", format!("agent_room '{room_id}' not found")))?;
            let (messages, _) = crate::web_api::reconstruct_v2_messages(store, &room.session_id);
            let exchanges = store
                .get_kv(&format!("room_exchanges:{room_id}"))
                .ok()
                .flatten()
                .and_then(|v| v.as_array().cloned())
                .unwrap_or_default();
            Ok(json!({
                "room": room,
                "participants": room.participant_expert_ids,
                "messages": messages,
                "exchanges": exchanges
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
        "create" | "create_room" | "upsert_room" => {
            let ts = now_ms();
            let id = input
                .get("id")
                .or_else(|| payload.get("id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(new_id);
            let room = AgentRoom {
                id: id.clone(),
                name: input
                    .get("name")
                    .or_else(|| input.get("title"))
                    .or_else(|| payload.get("name"))
                    .or_else(|| payload.get("title"))
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or("Agent Room")
                    .to_string(),
                workspace_root: input
                    .get("workspace_root")
                    .or_else(|| payload.get("workspace_root"))
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string)
                    .unwrap_or_else(|| suite.workspace_root.display().to_string()),
                participant_expert_ids: input
                    .get("participant_expert_ids")
                    .or_else(|| input.get("participantExpertIds"))
                    .or_else(|| payload.get("participant_expert_ids"))
                    .or_else(|| payload.get("participantExpertIds"))
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(ToString::to_string))
                            .collect()
                    })
                    .unwrap_or_default(),
                session_id: input
                    .get("session_id")
                    .or_else(|| payload.get("session_id"))
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
        "update_room" => {
            let id = input
                .get("id")
                .or_else(|| input.get("room_id"))
                .or_else(|| input.get("roomId"))
                .or_else(|| payload.get("id"))
                .or_else(|| payload.get("room_id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .ok_or(("invalid_argument", "missing room id".into()))?;
            let mut room = suite
                .get_record::<AgentRoom>("agent_room", id)
                .map_err(|e| ("storage_error", e.to_string()))?
                .ok_or(("not_found", format!("room '{id}' not found")))?;
            if let Some(name) = input
                .get("name")
                .or_else(|| input.get("title"))
                .and_then(|v| v.as_str())
            {
                room.name = name.to_string();
            }
            if let Some(arr) = input
                .get("participant_expert_ids")
                .or_else(|| input.get("participantExpertIds"))
                .and_then(|v| v.as_array())
            {
                room.participant_expert_ids = arr
                    .iter()
                    .filter_map(|x| x.as_str().map(ToString::to_string))
                    .collect();
            }
            room.updated_at_ms = now_ms();
            suite
                .upsert_record("agent_room", id, &room)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"ok": true, "room": room}))
        }
        "submit" => {
            let room_id = input
                .get("room_id")
                .or_else(|| input.get("roomId"))
                .or_else(|| args.get("room_id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .ok_or(("invalid_argument", "missing 'room_id'".into()))?;
            let sub_id = input
                .get("submission_id")
                .or_else(|| input.get("id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(new_id);
            let ts = now_ms();
            let rec = json!({
                "id": sub_id,
                "submission_id": sub_id,
                "room_id": room_id,
                "conversation_id": input.get("conversation_id").cloned().unwrap_or(Value::Null),
                "content": input.get("content").or_else(|| input.get("message")).or_else(|| input.get("text")).cloned().unwrap_or_else(|| json!("")),
                "recipient_id": input.get("recipient_id").or_else(|| input.get("expert_id")).or_else(|| input.get("to")).cloned().unwrap_or(Value::Null),
                "status": "pending",
                "created_at_ms": ts,
                "updated_at_ms": ts
            });
            suite
                .upsert_record("agent_room_submission", &sub_id, &rec)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"ok": true, "submission_id": sub_id, "submission": rec}))
        }
        "submission" => {
            let sub_id = input
                .get("submission_id")
                .or_else(|| input.get("id"))
                .or_else(|| args.get("submission_id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'submission_id'".into()))?;
            let found = suite
                .get_record::<Value>("agent_room_submission", sub_id)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({
                "submission_id": sub_id,
                "found": found.is_some(),
                "submission": found.unwrap_or(Value::Null)
            }))
        }
        "submissions" => {
            let room_id = input
                .get("room_id")
                .or_else(|| input.get("roomId"))
                .or_else(|| args.get("room_id"))
                .and_then(|v| v.as_str());
            let conv_id = input
                .get("conversation_id")
                .or_else(|| args.get("conversation_id"))
                .and_then(|v| v.as_str());
            let status_filter = input.get("status").and_then(|v| v.as_str());
            let list: Vec<Value> = suite
                .list_records::<Value>("agent_room_submission")
                .map_err(|e| ("storage_error", e.to_string()))?
                .into_iter()
                .filter(|s| {
                    room_id.is_none_or(|rid| s.get("room_id").and_then(|v| v.as_str()) == Some(rid))
                        && conv_id.is_none_or(|cid| {
                            s.get("conversation_id").and_then(|v| v.as_str()) == Some(cid)
                        })
                        && status_filter
                            .is_none_or(|st| s.get("status").and_then(|v| v.as_str()) == Some(st))
                })
                .collect();
            Ok(json!({
                "room_id": room_id,
                "submissions": list,
                "items": list
            }))
        }
        "select_recipient" => {
            let room_id = input
                .get("room_id")
                .or_else(|| input.get("roomId"))
                .or_else(|| args.get("room_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let sub_id = input
                .get("submission_id")
                .and_then(|v| v.as_str());
            let explicit_recipient = input
                .get("recipient_id")
                .or_else(|| input.get("expert_id"))
                .or_else(|| input.get("agent_id"))
                .or_else(|| input.get("to"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string);
            let recipient = explicit_recipient.unwrap_or_else(|| {
                suite
                    .get_record::<AgentRoom>("agent_room", room_id)
                    .ok()
                    .flatten()
                    .and_then(|r| r.participant_expert_ids.into_iter().next())
                    .unwrap_or_else(|| "assistant".into())
            });
            let ts = now_ms();
            if !room_id.is_empty() {
                let _ = store.set_kv(
                    &format!("room_selected_recipient:{room_id}"),
                    &json!({"room_id": room_id, "recipient_id": recipient, "updated_at_ms": ts}),
                );
            }
            if let Some(sid) = sub_id {
                if let Ok(Some(mut sub)) = suite.get_record::<Value>("agent_room_submission", sid) {
                    if let Some(obj) = sub.as_object_mut() {
                        obj.insert("recipient_id".into(), json!(recipient));
                        obj.insert("updated_at_ms".into(), json!(ts));
                    }
                    let _ = suite.upsert_record("agent_room_submission", sid, &sub);
                }
            }
            Ok(json!({
                "ok": true,
                "room_id": room_id,
                "submission_id": sub_id,
                "recipient_id": recipient
            }))
        }
        "prepare_execution" => {
            let room_id = input
                .get("room_id")
                .or_else(|| input.get("roomId"))
                .or_else(|| args.get("room_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let sub_id = input
                .get("submission_id")
                .or_else(|| input.get("id"))
                .and_then(|v| v.as_str())
                .map(ToString::to_string);
            let exec_id = new_id();
            let ts = now_ms();
            let intent = json!({
                "execution_id": exec_id,
                "room_id": room_id,
                "submission_id": sub_id,
                "recipient_id": input.get("recipient_id").or_else(|| input.get("expert_id")).cloned().unwrap_or(Value::Null),
                "phase": "prepared",
                "prepared_at_ms": ts
            });
            if !room_id.is_empty() {
                let _ = store.set_kv(&format!("room_exec_intent:{room_id}"), &intent);
            }
            if let Some(ref sid) = sub_id {
                if let Ok(Some(mut sub)) = suite.get_record::<Value>("agent_room_submission", sid) {
                    if let Some(obj) = sub.as_object_mut() {
                        obj.insert("status".into(), json!("prepared"));
                        obj.insert("execution_id".into(), json!(exec_id));
                        obj.insert("updated_at_ms".into(), json!(ts));
                    }
                    let _ = suite.upsert_record("agent_room_submission", sid, &sub);
                }
            }
            Ok(json!({
                "ok": true,
                "ready": true,
                "execution_id": exec_id,
                "intent": intent
            }))
        }
        "claim_execution" => {
            let room_id = input
                .get("room_id")
                .or_else(|| input.get("roomId"))
                .or_else(|| args.get("room_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let sub_id = input
                .get("submission_id")
                .or_else(|| input.get("id"))
                .and_then(|v| v.as_str())
                .map(ToString::to_string);
            let exec_id = input
                .get("execution_id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(new_id);
            let ts = now_ms();
            let mut claimed_sub = Value::Null;
            if let Some(ref sid) = sub_id {
                if let Ok(Some(mut sub)) = suite.get_record::<Value>("agent_room_submission", sid) {
                    if let Some(obj) = sub.as_object_mut() {
                        obj.insert("status".into(), json!("claimed"));
                        obj.insert("execution_id".into(), json!(exec_id));
                        obj.insert("claimed_at_ms".into(), json!(ts));
                        obj.insert("updated_at_ms".into(), json!(ts));
                    }
                    let _ = suite.upsert_record("agent_room_submission", sid, &sub);
                    claimed_sub = sub;
                }
            }
            let intent = json!({
                "execution_id": exec_id,
                "room_id": room_id,
                "submission_id": sub_id,
                "phase": "claimed",
                "claimed_at_ms": ts
            });
            if !room_id.is_empty() {
                let _ = store.set_kv(&format!("room_exec_intent:{room_id}"), &intent);
            }
            Ok(json!({
                "ok": true,
                "claimed": true,
                "execution_id": exec_id,
                "intent": intent,
                "submission": claimed_sub
            }))
        }
        "execution_intent" => {
            let room_id = input
                .get("room_id")
                .or_else(|| input.get("roomId"))
                .or_else(|| args.get("room_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let found = if !room_id.is_empty() {
                store
                    .get_kv(&format!("room_exec_intent:{room_id}"))
                    .ok()
                    .flatten()
                    .filter(|v| !v.is_null())
            } else {
                None
            };
            Ok(json!({
                "room_id": room_id,
                "found": found.is_some(),
                "intent": found.unwrap_or(Value::Null)
            }))
        }
        "cancel_pending" => {
            let room_id = input
                .get("room_id")
                .or_else(|| input.get("roomId"))
                .or_else(|| args.get("room_id"))
                .and_then(|v| v.as_str());
            let target_sub = input.get("submission_id").and_then(|v| v.as_str());
            let ts = now_ms();
            let mut cancelled_count = 0usize;
            for mut sub in suite
                .list_records::<Value>("agent_room_submission")
                .unwrap_or_default()
            {
                let matches_room = room_id.is_none_or(|rid| {
                    sub.get("room_id").and_then(|v| v.as_str()) == Some(rid)
                });
                let matches_sub = target_sub.is_none_or(|sid| {
                    sub.get("submission_id")
                        .or_else(|| sub.get("id"))
                        .and_then(|v| v.as_str())
                        == Some(sid)
                });
                let is_pending = matches!(
                    sub.get("status").and_then(|v| v.as_str()),
                    Some("pending" | "prepared" | "claimed")
                );
                if matches_room && matches_sub && is_pending {
                    if let Some(sid) = sub
                        .get("submission_id")
                        .or_else(|| sub.get("id"))
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string)
                    {
                        if let Some(obj) = sub.as_object_mut() {
                            obj.insert("status".into(), json!("cancelled"));
                            obj.insert("cancelled_at_ms".into(), json!(ts));
                            obj.insert("updated_at_ms".into(), json!(ts));
                        }
                        let _ = suite.upsert_record("agent_room_submission", &sid, &sub);
                        cancelled_count += 1;
                    }
                }
            }
            if let Some(rid) = room_id {
                let _ = store.set_kv(&format!("room_exec_intent:{rid}"), &Value::Null);
            }
            Ok(json!({
                "ok": true,
                "room_id": room_id,
                "cancelled_count": cancelled_count
            }))
        }
        "reconcile_execution" => {
            let sub_id = input
                .get("submission_id")
                .or_else(|| input.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'submission_id'".into()))?;
            let status = input
                .get("status")
                .or_else(|| input.get("outcome"))
                .and_then(|v| v.as_str())
                .unwrap_or("completed");
            let ts = now_ms();
            let mut sub = suite
                .get_record::<Value>("agent_room_submission", sub_id)
                .map_err(|e| ("storage_error", e.to_string()))?
                .unwrap_or_else(|| json!({"id": sub_id, "submission_id": sub_id}));
            let room_id = sub
                .get("room_id")
                .and_then(|v| v.as_str())
                .or_else(|| input.get("room_id").and_then(|v| v.as_str()))
                .map(ToString::to_string);
            if let Some(obj) = sub.as_object_mut() {
                obj.insert("status".into(), json!(status));
                if let Some(out) = input.get("output").or_else(|| input.get("result")) {
                    obj.insert("output".into(), out.clone());
                }
                if let Some(err) = input.get("error") {
                    obj.insert("error".into(), err.clone());
                }
                obj.insert("reconciled_at_ms".into(), json!(ts));
                obj.insert("updated_at_ms".into(), json!(ts));
            }
            suite
                .upsert_record("agent_room_submission", sub_id, &sub)
                .map_err(|e| ("storage_error", e.to_string()))?;
            if let Some(ref rid) = room_id {
                if matches!(status, "completed" | "failed" | "cancelled") {
                    let _ = store.set_kv(&format!("room_exec_intent:{rid}"), &Value::Null);
                }
            }
            Ok(json!({
                "ok": true,
                "reconciled": true,
                "submission_id": sub_id,
                "submission": sub
            }))
        }
        "fail_submission" => {
            let sub_id = input
                .get("submission_id")
                .or_else(|| input.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'submission_id'".into()))?;
            let error_msg = input
                .get("error")
                .or_else(|| input.get("message"))
                .or_else(|| input.get("reason"))
                .cloned()
                .unwrap_or_else(|| json!("submission failed"));
            let ts = now_ms();
            let mut sub = suite
                .get_record::<Value>("agent_room_submission", sub_id)
                .map_err(|e| ("storage_error", e.to_string()))?
                .unwrap_or_else(|| json!({"id": sub_id, "submission_id": sub_id}));
            let room_id = sub
                .get("room_id")
                .and_then(|v| v.as_str())
                .or_else(|| input.get("room_id").and_then(|v| v.as_str()))
                .map(ToString::to_string);
            if let Some(obj) = sub.as_object_mut() {
                obj.insert("status".into(), json!("failed"));
                obj.insert("error".into(), error_msg);
                obj.insert("failed_at_ms".into(), json!(ts));
                obj.insert("updated_at_ms".into(), json!(ts));
            }
            suite
                .upsert_record("agent_room_submission", sub_id, &sub)
                .map_err(|e| ("storage_error", e.to_string()))?;
            if let Some(ref rid) = room_id {
                let _ = store.set_kv(&format!("room_exec_intent:{rid}"), &Value::Null);
            }
            Ok(json!({
                "ok": true,
                "failed": true,
                "submission_id": sub_id,
                "submission": sub
            }))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daemon_model_agent_room_and_top_level_gap_commands_work() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory().unwrap();
        let suite = ModulesSuite::new(store.clone(), dir.path().to_path_buf(), None);

        // 1. Model domain commands
        let acct = dispatch_command(
            &store,
            &suite,
            "model",
            &json!({"operation": "save_account", "id": "acct-1", "label": "Work"}),
        )
        .unwrap();
        assert_eq!(acct["ok"], true);
        assert_eq!(acct["id"], "acct-1");

        let conn = dispatch_command(
            &store,
            &suite,
            "model",
            &json!({
                "operation": "save_connection",
                "id": "conn-1",
                "endpoint": "https://example.invalid/v1",
                "protocol": "siwc_responses",
                "models": ["gpt-5"]
            }),
        )
        .unwrap();
        assert_eq!(conn["ok"], true);

        let cap = dispatch_command(
            &store,
            &suite,
            "model",
            &json!({
                "operation": "set_connection_capability_override",
                "connection_id": "conn-1",
                "model_id": "gpt-5",
                "capabilities": {"reasoning": true}
            }),
        )
        .unwrap();
        assert_eq!(cap["ok"], true);

        let catalog = dispatch_command(
            &store,
            &suite,
            "model",
            &json!({
                "operation": "refresh_connection_catalog",
                "connection_id": "conn-1",
                "models": ["gpt-5", "gpt-5-mini"]
            }),
        )
        .unwrap();
        assert_eq!(catalog["models"].as_array().unwrap().len(), 2);

        let cfg_models = dispatch_command(
            &store,
            &suite,
            "model",
            &json!({
                "operation": "configure_connection_models",
                "connection_id": "conn-1",
                "enabled_model_ids": ["conn-1:gpt-5"],
                "selected_model_id": "conn-1:gpt-5"
            }),
        )
        .unwrap();
        assert_eq!(cfg_models["ok"], true);

        // 2. Agent room create + submission lifecycle
        let room = dispatch_agent_room_command(
            &store,
            &suite,
            &json!({"command": {"type": "create", "id": "room-1", "name": "War Room"}}),
        )
        .unwrap();
        assert_eq!(room["room"]["id"], "room-1");

        let sub = dispatch_agent_room_command(
            &store,
            &suite,
            &json!({"command": {"type": "submit", "room_id": "room-1", "content": "Review PR"}}),
        )
        .unwrap();
        let sub_id = sub["submission_id"].as_str().unwrap().to_string();

        let got_sub = dispatch_agent_room_command(
            &store,
            &suite,
            &json!({"command": {"type": "submission", "submission_id": sub_id}}),
        )
        .unwrap();
        assert_eq!(got_sub["found"], true);

        let subs = dispatch_agent_room_command(
            &store,
            &suite,
            &json!({"command": {"type": "submissions", "room_id": "room-1"}}),
        )
        .unwrap();
        assert_eq!(subs["submissions"].as_array().unwrap().len(), 1);

        let sel = dispatch_agent_room_command(
            &store,
            &suite,
            &json!({"command": {"type": "select_recipient", "room_id": "room-1", "submission_id": sub_id, "recipient_id": "reviewer"}}),
        )
        .unwrap();
        assert_eq!(sel["recipient_id"], "reviewer");

        let prep = dispatch_agent_room_command(
            &store,
            &suite,
            &json!({"command": {"type": "prepare_execution", "room_id": "room-1", "submission_id": sub_id}}),
        )
        .unwrap();
        assert_eq!(prep["ready"], true);

        let claimed = dispatch_agent_room_command(
            &store,
            &suite,
            &json!({"command": {"type": "claim_execution", "room_id": "room-1", "submission_id": sub_id}}),
        )
        .unwrap();
        assert_eq!(claimed["claimed"], true);

        let intent = dispatch_agent_room_command(
            &store,
            &suite,
            &json!({"command": {"type": "execution_intent", "room_id": "room-1"}}),
        )
        .unwrap();
        assert_eq!(intent["found"], true);

        let reconciled = dispatch_agent_room_command(
            &store,
            &suite,
            &json!({"command": {"type": "reconcile_execution", "submission_id": sub_id, "status": "completed"}}),
        )
        .unwrap();
        assert_eq!(reconciled["reconciled"], true);

        let sub2 = dispatch_agent_room_command(
            &store,
            &suite,
            &json!({"command": {"type": "submit", "room_id": "room-1", "content": "Second"}}),
        )
        .unwrap();
        let sub2_id = sub2["submission_id"].as_str().unwrap().to_string();
        let failed = dispatch_agent_room_command(
            &store,
            &suite,
            &json!({"command": {"type": "fail_submission", "submission_id": sub2_id, "error": "boom"}}),
        )
        .unwrap();
        assert_eq!(failed["failed"], true);

        let _ = dispatch_agent_room_command(
            &store,
            &suite,
            &json!({"command": {"type": "submit", "room_id": "room-1", "content": "Third"}}),
        )
        .unwrap();
        let cancelled = dispatch_agent_room_command(
            &store,
            &suite,
            &json!({"command": {"type": "cancel_pending", "room_id": "room-1"}}),
        )
        .unwrap();
        assert_eq!(cancelled["cancelled_count"], 1);

        // 3. Top-level commands
        let g = dispatch_command(
            &store,
            &suite,
            "upsert_permission_grant",
            &json!({"conversation_id": "c1", "grant": {"id": "g1", "tool": "bash"}}),
        )
        .unwrap();
        assert_eq!(g["grants"].as_array().unwrap().len(), 1);

        let rg = dispatch_command(
            &store,
            &suite,
            "revoke_permission_grant",
            &json!({"conversation_id": "c1", "grant_id": "g1"}),
        )
        .unwrap();
        assert_eq!(rg["revoked"], true);

        let imp = dispatch_command(
            &store,
            &suite,
            "import_conversation_transcript",
            &json!({"conversation_id": "c1", "messages": [{"role": "user", "content": "hi"}, {"role": "assistant", "content": "hello"}]}),
        )
        .unwrap();
        assert_eq!(imp["imported_count"], 2);

        let hist = dispatch_command(
            &store,
            &suite,
            "query_canonical_history",
            &json!({"conversation_id": "c1"}),
        )
        .unwrap();
        assert_eq!(hist["total"], 2);

        let adm = dispatch_command(
            &store,
            &suite,
            "admit_host_output",
            &json!({"conversation_id": "c1", "output": "done"}),
        )
        .unwrap();
        assert_eq!(adm["admitted"], true);

        let guid = dispatch_command(
            &store,
            &suite,
            "submit_task_guidance",
            &json!({"task_id": "t1", "guidance": "focus"}),
        )
        .unwrap();
        assert_eq!(guid["ok"], true);
        let load_guid = dispatch_command(
            &store,
            &suite,
            "load_task_guidance_receipt",
            &json!({"task_id": "t1"}),
        )
        .unwrap();
        assert_eq!(load_guid["found"], true);

        let cancel_owned = dispatch_command(
            &store,
            &suite,
            "request_owned_cancellation",
            &json!({"run_id": "run-1"}),
        )
        .unwrap();
        assert_eq!(cancel_owned["cancelled"], true);

        let cur = dispatch_command(
            &store,
            &suite,
            "current_run",
            &json!({"conversation_id": "c1"}),
        )
        .unwrap();
        assert_eq!(cur["active"], false);

        let ctx_usage = dispatch_command(
            &store,
            &suite,
            "local_run_context_usage",
            &json!({"conversation_id": "c1"}),
        )
        .unwrap();
        assert_eq!(ctx_usage["conversation_id"], "c1");

        let sources = dispatch_command(
            &store,
            &suite,
            "read_conversation_tool_sources",
            &json!({"conversation_id": "c1"}),
        )
        .unwrap();
        assert!(sources["mcp_servers"].is_array());

        let stg = dispatch_command(
            &store,
            &suite,
            "stage_archive_manifest",
            &json!({"entries": []}),
        )
        .unwrap();
        assert_eq!(stg["staged"], true);

        let beg = dispatch_command(&store, &suite, "begin_archive_restore", &json!({})).unwrap();
        let rid = beg["restore_id"].as_str().unwrap().to_string();
        let com = dispatch_command(
            &store,
            &suite,
            "commit_archive_restore",
            &json!({"restore_id": rid}),
        )
        .unwrap();
        assert_eq!(com["status"], "committed");
        let rev = dispatch_command(&store, &suite, "archive_directory_revision", &json!({})).unwrap();
        assert_eq!(rev["revision"], 2);

        let up = dispatch_command(
            &store,
            &suite,
            "begin_content_upload",
            &json!({"filename": "a.txt", "total_bytes": 5}),
        )
        .unwrap();
        let uid = up["upload_id"].as_str().unwrap().to_string();
        let app_c = dispatch_command(
            &store,
            &suite,
            "append_content_chunk",
            &json!({"upload_id": uid, "chunk_index": 0, "data": "hello"}),
        )
        .unwrap();
        assert_eq!(app_c["bytes_received"], 5);
        let read_c = dispatch_command(
            &store,
            &suite,
            "read_content_upload_chunk",
            &json!({"upload_id": uid, "chunk_index": 0}),
        )
        .unwrap();
        assert_eq!(read_c["chunk"]["data"], "hello");
        let seal = dispatch_command(
            &store,
            &suite,
            "seal_content_upload",
            &json!({"upload_id": uid}),
        )
        .unwrap();
        assert_eq!(seal["sealed"], true);
        let prog = dispatch_command(
            &store,
            &suite,
            "content_upload_progress",
            &json!({"upload_id": uid}),
        )
        .unwrap();
        assert_eq!(prog["sealed"], true);

        let rt = dispatch_command(
            &store,
            &suite,
            "runtime_task",
            &json!({"operation": "start", "task_id": "rt-1", "conversation_id": "c1"}),
        )
        .unwrap();
        assert_eq!(rt["task"]["status"], "running");
        let rt_list = dispatch_command(
            &store,
            &suite,
            "runtime_task",
            &json!({"operation": "list", "conversation_id": "c1"}),
        )
        .unwrap();
        assert_eq!(rt_list["tasks"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn daemon_real_sync_engine_branch_state_recovery_and_canonical_rpcs() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory().unwrap();
        let suite = ModulesSuite::new(store.clone(), dir.path().to_path_buf(), None);

        // 1. Real Sync Engine backed by Store sync_changes & sync_cursors
        store
            .record_sync_change(&SyncChangeRecord {
                id: "chg-local-1".into(),
                scope: "conv-sync".into(),
                entity_kind: "kv".into(),
                entity_id: "k1".into(),
                payload_json: r#"{"v":1}"#.into(),
                created_at_ms: 1000,
                resolved: false,
            })
            .unwrap();
        let pending = dispatch_command(&store, &suite, "pending_sync_changes", &json!({})).unwrap();
        assert_eq!(pending.as_array().unwrap().len(), 1);
        assert_eq!(pending[0]["id"], "chg-local-1");

        let resolved = dispatch_command(
            &store,
            &suite,
            "resolve_sync_change",
            &json!({"id": "chg-local-1"}),
        )
        .unwrap();
        assert_eq!(resolved, json!(true));
        let pending_after =
            dispatch_command(&store, &suite, "pending_sync_changes", &json!({})).unwrap();
        assert!(pending_after.as_array().unwrap().is_empty());

        let saved_cur = dispatch_command(
            &store,
            &suite,
            "save_sync_cursor",
            &json!({"replica_id": "peer-a", "cursor": 42}),
        )
        .unwrap();
        assert_eq!(saved_cur["cursor"], 42);
        let got_cur = dispatch_command(
            &store,
            &suite,
            "get_sync_cursor",
            &json!({"replica_id": "peer-a"}),
        )
        .unwrap();
        assert_eq!(got_cur["cursor"], 42);

        // apply_remote_sync_page applies idempotently and updates cursor
        let page = json!({
            "input": {
                "replica_id": "peer-a",
                "next_cursor": 55,
                "changes": [
                    {"id": "rem-1", "scope": "global", "entity_kind": "kv", "entity_id": "synced_key", "payload": {"hello": "world"}},
                    {"id": "rem-2", "scope": "conv-rem", "entity_kind": "event", "entity_id": "ev-1", "payload": {"kind": "user_message", "payload": {"content": "from remote"}}}
                ]
            }
        });
        let r1 = dispatch_command(&store, &suite, "apply_remote_sync_page", &page).unwrap();
        assert_eq!(r1["applied"], 2);
        assert_eq!(r1["skipped"], 0);
        assert_eq!(r1["cursor"], 55);
        assert_eq!(store.get_kv("synced_key").unwrap().unwrap()["hello"], "world");

        // Re-applying same page MUST skip idempotently!
        let r2 = dispatch_command(&store, &suite, "apply_remote_sync_page", &page).unwrap();
        assert_eq!(r2["applied"], 0);
        assert_eq!(r2["skipped"], 2);

        // 2. branch_conversation copies session_state and supports up_to_seq truncation
        let sess = Session {
            id: "conv-src".into(),
            title: "Original".into(),
            workspace_root: dir.path().display().to_string(),
            model_connection: None,
            model_id: None,
            created_at_ms: 1000,
            updated_at_ms: 1000,
        };
        store.upsert_session(&sess).unwrap();
        let e1 = store
            .append_event("conv-src", "user_message", json!({"id": "u1", "role": "user", "content": "first"}), 50_000)
            .unwrap();
        let e2 = store
            .append_event("conv-src", "assistant_message", json!({"id": "a1", "role": "assistant", "content": "second"}), 50_000)
            .unwrap();
        let _e3 = store
            .append_event("conv-src", "user_message", json!({"id": "u2", "role": "user", "content": "third"}), 50_000)
            .unwrap();
        store
            .set_kv(
                "session_state:conv-src",
                &json!({
                    "history": [
                        {"role": "user", "content": "first"},
                        {"role": "assistant", "content": "second"},
                        {"role": "user", "content": "third"}
                    ],
                    "pinned": true
                }),
            )
            .unwrap();

        let branched = dispatch_command(
            &store,
            &suite,
            "branch_conversation",
            &json!({"conversation_id": "conv-src", "up_to_seq": e2.seq}),
        )
        .unwrap();
        let new_id = branched["conversation"]["id"].as_str().unwrap().to_string();
        let new_state = store.get_kv(&format!("session_state:{new_id}")).unwrap().unwrap();
        assert_eq!(new_state["pinned"], true);
        assert_eq!(new_state["history"].as_array().unwrap().len(), 2);

        // truncate_conversation & prepare_edited_resend atomically trim session_state
        let _ = dispatch_command(
            &store,
            &suite,
            "prepare_edited_resend",
            &json!({"conversation_id": "conv-src", "from_seq": e2.seq}),
        )
        .unwrap();
        let trimmed_state = store.get_kv("session_state:conv-src").unwrap().unwrap();
        assert_eq!(trimmed_state["history"].as_array().unwrap().len(), 1);
        assert_eq!(e1.seq, 1);

        // 3. Active run tracking, durable permission_grant projection, & recovery inspection
        store
            .put_module_record(
                "permission_grant",
                "pg-1",
                &json!({"id": "pg-1", "session_id": "conv-src", "tool": "bash", "active": true}),
            )
            .unwrap();
        let perm_proj = dispatch_command(
            &store,
            &suite,
            "load_conversation_permission_projection_v1",
            &json!({"conversation_id": "conv-src"}),
        )
        .unwrap();
        assert_eq!(perm_proj["grants"].as_array().unwrap().len(), 1);
        assert_eq!(perm_proj["grants"][0]["id"], "pg-1");

        store
            .put_module_record(
                "durable_task",
                "dt-1",
                &json!({"task_id": "dt-1", "session_id": "conv-src", "status": "interrupted", "role": "worker"}),
            )
            .unwrap();
        store
            .set_kv(
                "turn_verification:conv-src",
                &json!({
                    "all_modified_verified": false,
                    "modified_files": [{"uri": "file:///tmp/a.rs", "operation": "write", "last_modified_step": 1, "verified_after_edit": false}]
                }),
            )
            .unwrap();
        let rec_run = dispatch_command(
            &store,
            &suite,
            "inspect_run_recovery",
            &json!({"conversation_id": "conv-src"}),
        )
        .unwrap();
        assert_eq!(rec_run["recoverable"], true);
        assert_eq!(rec_run["state"], "interrupted");

        let rec_eff = dispatch_command(
            &store,
            &suite,
            "inspect_unknown_effect_recovery",
            &json!({"conversation_id": "conv-src"}),
        )
        .unwrap();
        assert_eq!(rec_eff["pending"], true);

        let rec_owned = dispatch_command(
            &store,
            &suite,
            "inspect_owned_task_recovery",
            &json!({"conversation_id": "conv-src"}),
        )
        .unwrap();
        assert_eq!(rec_owned["pending"], true);

        // 4. Direct event range / FTS historical reads
        let hist_part = dispatch_command(
            &store,
            &suite,
            "read_historical_part",
            &json!({"conversation_id": "conv-src", "seq": 1}),
        )
        .unwrap();
        assert_eq!(hist_part["found"], true);
        assert_eq!(hist_part["content"], "first");

        let range_fts = dispatch_command(
            &store,
            &suite,
            "read_conversation_text_range",
            &json!({"conversation_id": "conv-src", "query": "first"}),
        )
        .unwrap();
        assert!(range_fts["text"].as_str().unwrap().contains("first"));

        // 5. Missing canonical & resource RPCs
        let pdf = dispatch_command(
            &store,
            &suite,
            "put_conversation_pdf_resource",
            &json!({"conversation_id": "conv-src", "resource_id": "pdf-1", "filename": "spec.pdf", "data_base64": "JVBERi0=", "page_count": 3}),
        )
        .unwrap();
        assert_eq!(pdf["resource_id"], "pdf-1");
        let pdf_read = dispatch_command(
            &store,
            &suite,
            "read_conversation_resource",
            &json!({"conversation_id": "conv-src", "resource_id": "pdf-1"}),
        )
        .unwrap();
        assert_eq!(pdf_read["resource"]["filename"], "spec.pdf");
        let pdf_list = dispatch_command(
            &store,
            &suite,
            "list_conversation_resources",
            &json!({"conversation_id": "conv-src"}),
        )
        .unwrap();
        assert_eq!(pdf_list["resources"].as_array().unwrap().len(), 1);
        let pdf_del = dispatch_command(
            &store,
            &suite,
            "delete_conversation_resource",
            &json!({"conversation_id": "conv-src", "resource_id": "pdf-1"}),
        )
        .unwrap();
        assert_eq!(pdf_del["deleted"], true);

        let adm = dispatch_command(
            &store,
            &suite,
            "admit_conversation_user_message",
            &json!({"input": {"conversation_id": "conv-src", "message_id": "m-admit-1", "content": "queued hello"}}),
        )
        .unwrap();
        assert_eq!(adm["admitted"], true);

        let begun = dispatch_command(
            &store,
            &suite,
            "begin_queued_user_turn",
            &json!({"input": {"conversation_id": "conv-src", "command_id": "run-q-1", "core_config": {"model": "gpt-4o"}}}),
        )
        .unwrap();
        assert_eq!(begun["run_id"], "run-q-1");
        let active = dispatch_command(
            &store,
            &suite,
            "active_run",
            &json!({"conversation_id": "conv-src"}),
        )
        .unwrap();
        assert_eq!(active["active"], true);
        assert_eq!(active["run_id"], "run-q-1");

        let exact = dispatch_command(
            &store,
            &suite,
            "load_exact_run_invocation",
            &json!({"conversation_id": "conv-src", "expected_run_id": "run-q-1", "assistant_message_id": "run-q-1:assistant"}),
        )
        .unwrap();
        assert_eq!(exact["run_id"], "run-q-1");
        assert_eq!(exact["invocation_config_digest"].as_str().unwrap().len(), 64);

        let staged = dispatch_command(
            &store,
            &suite,
            "stage_assistant_journal_content",
            &json!({
                "input": {
                    "record": {"identity": {"conversation_id": "conv-src", "run_id": "run-q-1", "assistant_message_id": "run-q-1:assistant"}},
                    "part_id": "part-1",
                    "content": {"content_id": "c-1"},
                    "chunks": [{"text": "Hello journal world"}]
                }
            }),
        )
        .unwrap();
        assert_eq!(staged["staged"], true);

        let j_parts = dispatch_command(
            &store,
            &suite,
            "read_assistant_journal_parts",
            &json!({"snapshot": {"identity": {"conversation_id": "conv-src", "run_id": "run-q-1"}, "at": {"revision": "1"}}}),
        )
        .unwrap();
        assert_eq!(j_parts["parts"].as_array().unwrap().len(), 1);

        let j_changes = dispatch_command(
            &store,
            &suite,
            "read_assistant_journal_part_changes",
            &json!({"snapshot": {"identity": {"conversation_id": "conv-src", "run_id": "run-q-1"}, "at": {"revision": "1"}}, "base": {"revision": "0"}}),
        )
        .unwrap();
        assert_eq!(j_changes["changes"].as_array().unwrap().len(), 1);

        let j_range = dispatch_command(
            &store,
            &suite,
            "read_assistant_journal_range",
            &json!({"snapshot": {"identity": {"conversation_id": "conv-src", "run_id": "run-q-1"}}, "part_id": "part-1", "content": {"content_id": "c-1"}, "offset_bytes": "6", "limit_bytes": 7}),
        )
        .unwrap();
        assert_eq!(j_range["text"], "journal");
        assert_eq!(j_range["end_of_content"], false);

        clear_active_run(&store, "conv-src");
    }
}
