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

#[cfg(unix)]
const MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;
#[cfg(unix)]
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
                "--archive-incompatible-store" => {
                    if args.get(i + 1).is_some_and(|next| !next.starts_with('-')) {
                        i += 1;
                    }
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
            .or_else(|| sub.get("kind"))
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
        | "observe_automation_worker"
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
        | "capture_mobile_delegation_configuration"
        | "prepare_mobile_delegated_task"
        | "prepare_mobile_task_guidance"
        | "admit_mobile_task_child"
        | "register_source_submission"
        | "reject_source_submission"
        | "begin_assistant_turn"
        | "begin_automation_assistant_turn"
        | "claim_committed_host_requests"
        | "reduce_and_accept_host_response"
        | "reduce_and_commit_active_submission"
        | "open_assistant_journal_snapshot"
        | "release_assistant_journal_snapshot"
        | "receive_run_message_admissions"
        | "receive_assistant_journal_record"
        | "recover_expired_run_deliveries"
        | "heartbeat_effect_delivery"
        | "backup" => true,
        "auxiliary_invocation" => {
            let (op, _) = extract_sub_command(cmd_args);
            !matches!(op, "inspect" | "inspect_source")
        }
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
    let mut cmd_args = req
        .get("command")
        .and_then(|c| c.get("arguments").or(Some(c)))
        .cloned()
        .unwrap_or_else(|| json!({}));
    if let Some(env_exp_rev) = req.get("expected_revision").filter(|v| !v.is_null()) {
        if let Some(obj) = cmd_args.as_object_mut() {
            obj.insert("__envelope_expected_revision".into(), env_exp_rev.clone());
        }
    }

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
        Ok(mut result) => {
            let explicit_committed_rev = result
                .as_object_mut()
                .and_then(|obj| obj.remove("__committed_revision"))
                .and_then(|v| v.as_u64());
            if is_mutation {
                let next_rev = if let Some(rev) = explicit_committed_rev {
                    rev
                } else {
                    let cur = store
                        .get_kv("app_committed_revision")
                        .ok()
                        .flatten()
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0)
                        + 1;
                    let _ = store.set_kv("app_committed_revision", &json!(cur));
                    cur
                };
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
        Err(("revision_conflict", payload_str)) => {
            if let Ok(conflict_val) = serde_json::from_str::<Value>(&payload_str) {
                if conflict_val.get("scope_id").is_some()
                    && conflict_val.get("expected_revision").is_some()
                    && conflict_val.get("actual_revision").is_some()
                {
                    json!({
                        "protocol_version": PROTOCOL_VERSION,
                        "client_id": client_id,
                        "request_id": request_id,
                        "received_at": received_at,
                        "completed_at": completed_at,
                        "status": "conflict",
                        "conflict": conflict_val
                    })
                } else {
                    json!({
                        "protocol_version": PROTOCOL_VERSION,
                        "client_id": client_id,
                        "request_id": request_id,
                        "received_at": received_at,
                        "completed_at": completed_at,
                        "status": "rejected",
                        "error": {
                            "code": "revision_conflict",
                            "message": payload_str,
                            "recoverable": false
                        }
                    })
                }
            } else {
                json!({
                    "protocol_version": PROTOCOL_VERSION,
                    "client_id": client_id,
                    "request_id": request_id,
                    "received_at": received_at,
                    "completed_at": completed_at,
                    "status": "rejected",
                    "error": {
                        "code": "revision_conflict",
                        "message": payload_str,
                        "recoverable": false
                    }
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
        "load_conversation" => {
            let id = args
                .get("conversation_id")
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing conversation_id".into()))?;
            let Some(sess) = get_session_case_insensitive(store, id)
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
                false,
            ))
        }
        "load_conversation_projection_v1"
        | "load_conversation_projection_window_v1" => {
            let id = args
                .get("conversation_id")
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing conversation_id".into()))?;
            let Some(sess) = get_session_case_insensitive(store, id)
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
                true,
            ))
        }
        "load_conversation_projection_page_v1" => {
            let id = args
                .get("conversation_id")
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing conversation_id".into()))?;
            let Some(sess) = get_session_case_insensitive(store, id)
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
                store, &sess, offset, limit, None, true,
            ))
        }
        "load_conversation_projection_window_update_v1" => {
            let id = args
                .get("conversation_id")
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing conversation_id".into()))?;
            let Some(sess) = get_session_case_insensitive(store, id)
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
            let resp = build_conversation_projection_response(
                store,
                &sess,
                0,
                limit,
                before_ordinal,
                true,
            );
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
            let eff_id = get_session_case_insensitive(store, id)
                .ok()
                .flatten()
                .map(|s| s.id)
                .unwrap_or_else(|| id.to_string());
            Ok(load_conversation_settings_str(store, &eff_id)
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
            if let Ok(Some(stored)) = store.get_kv(&format!("session_projection:{id}")) {
                return Ok(stored);
            }
            Ok(Value::Null)
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
            let grants = load_durable_permission_grants(store, Some(id));
            if let Some(rid) = args.get("run_id").and_then(|v| v.as_str()) {
                let run = store.get_kv(&format!("committed_run:{rid}")).ok().flatten();
                let session_id = run
                    .as_ref()
                    .and_then(|r| r.get("session_id").and_then(|v| v.as_str()))
                    .unwrap_or(id);
                let is_terminal = run.as_ref().is_some_and(|r| {
                    matches!(
                        r.get("status").and_then(|v| v.as_str()),
                        Some("completed" | "finished" | "failed" | "cancelled")
                    )
                });
                let active_turn_id = if is_terminal {
                    Value::Null
                } else {
                    run.as_ref()
                        .and_then(|r| r.get("turn_id"))
                        .cloned()
                        .unwrap_or(Value::Null)
                };
                let perm_rev = run
                    .as_ref()
                    .and_then(|r| r.get("permission_projection_revision").and_then(|v| v.as_u64()))
                    .unwrap_or(1);
                let event_cursor = run
                    .as_ref()
                    .and_then(|r| r.get("sequence").and_then(|v| v.as_u64()))
                    .unwrap_or(1);
                let pending_perms = run
                    .as_ref()
                    .and_then(|r| r.get("pending_permissions").and_then(|v| v.as_array()))
                    .cloned()
                    .unwrap_or_default();
                return Ok(json!({
                    "conversation_id": id,
                    "run_id": rid,
                    "session_id": session_id,
                    "active_turn_id": active_turn_id,
                    "permission_projection_revision": perm_rev,
                    "event_cursor": event_cursor,
                    "permissions": {
                        "revision": perm_rev,
                        "pending": pending_perms
                    },
                    "grants": grants
                }));
            }
            let (profile, rev) = read_permission_state(store, Some(id));
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
                let Some(found) = lookup_active_run(store, cid) else {
                    return Ok(Value::Null);
                };
                let run_id = found.get("run_id").cloned().unwrap_or(Value::Null);
                let assistant_message_id = found
                    .get("assistant_message_id")
                    .cloned()
                    .unwrap_or(Value::Null);
                let kind = found
                    .get("kind")
                    .cloned()
                    .unwrap_or_else(|| json!("conversation"));
                let next_event_sequence = found
                    .get("next_event_sequence")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(1);
                let invocation_config_digest = found
                    .get("invocation_config_digest")
                    .cloned()
                    .unwrap_or(Value::Null);
                Ok(json!({
                    "active": true,
                    "run": found,
                    "run_id": run_id,
                    "kind": kind,
                    "assistant_message_id": assistant_message_id,
                    "next_event_sequence": next_event_sequence,
                    "invocation_config_digest": invocation_config_digest,
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
            let sess = store
                .get_session(id)
                .map_err(|e| ("storage_error", e.to_string()))?
                .ok_or(("not_found", format!("conversation '{id}' not found")))?;
            let meta = store
                .get_kv(&crate::web_api::session_meta_key(id))
                .ok()
                .flatten()
                .unwrap_or_else(|| json!({"revision": 1}));
            let rev = meta.get("revision").and_then(|v| v.as_i64()).unwrap_or(1).max(1);
            if let Some(exp) = args.get("expected_revision").and_then(|v| v.as_i64()) {
                if exp != rev {
                    return Err((
                        "revision_conflict",
                        json!({
                            "scope_id": format!("conversation:{id}"),
                            "expected_revision": exp,
                            "actual_revision": rev
                        })
                        .to_string(),
                    ));
                }
            }
            let after_ordinal = args.get("after_ordinal").and_then(|v| v.as_i64()).unwrap_or(0);
            let limit = args
                .get("limit")
                .or_else(|| args.get("page_size"))
                .and_then(|v| v.as_u64())
                .unwrap_or(100)
                .clamp(1, 500) as usize;
            let (raw_messages, _events) = crate::web_api::reconstruct_v2_messages(store, id);
            let mut all_pages: Vec<Value> = Vec::with_capacity(raw_messages.len());
            for (idx, m) in raw_messages.iter().enumerate() {
                let ordinal = (idx + 1) as i64;
                if ordinal <= after_ordinal {
                    continue;
                }
                let msg_id = m
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("msg")
                    .to_string();
                let role = m.get("role").and_then(|v| v.as_str()).unwrap_or("user");
                let parts: Vec<Value> = if let Some(ps) = m.get("parts").and_then(|v| v.as_array()) {
                    ps.iter()
                        .enumerate()
                        .map(|(pi, p)| {
                            let pid = p
                                .get("id")
                                .or_else(|| p.get("part_id"))
                                .and_then(|v| v.as_str())
                                .map(ToString::to_string)
                                .unwrap_or_else(|| format!("{msg_id}:part-{pi}"));
                            let pkind = p.get("kind").and_then(|v| v.as_str()).unwrap_or("text");
                            let text_bytes = p
                                .get("content_text")
                                .or_else(|| p.get("contentText"))
                                .or_else(|| p.get("text"))
                                .and_then(|v| v.as_str())
                                .map(|s| s.len())
                                .unwrap_or(0);
                            let json_bytes = p
                                .get("content_json")
                                .or_else(|| p.get("contentJson"))
                                .map(|v| {
                                    v.as_str()
                                        .map(|s| s.len())
                                        .unwrap_or_else(|| v.to_string().len())
                                })
                                .unwrap_or(0);
                            json!({
                                "id": pid,
                                "kind": pkind,
                                "content_text_utf8_bytes": text_bytes,
                                "content_json_utf8_bytes": json_bytes
                            })
                        })
                        .collect()
                } else {
                    let content = m.get("content").and_then(|v| v.as_str()).unwrap_or("");
                    vec![json!({
                        "id": format!("{msg_id}:text"),
                        "kind": "text",
                        "content_text_utf8_bytes": content.len(),
                        "content_json_utf8_bytes": 0
                    })]
                };
                all_pages.push(json!({
                    "id": msg_id,
                    "role": role,
                    "ordinal": ordinal,
                    "parts": parts
                }));
            }
            let has_more = all_pages.len() > limit;
            let page_msgs: Vec<Value> = all_pages.into_iter().take(limit).collect();
            let next_ordinal = if has_more {
                page_msgs.last().and_then(|m| m.get("ordinal")).cloned()
            } else {
                None
            };
            Ok(json!({
                "conversation": {
                    "id": sess.id,
                    "title": sess.title,
                    "revision": rev
                },
                "conversation_id": sess.id,
                "messages": page_msgs,
                "has_more": has_more,
                "next_ordinal": next_ordinal
            }))
        }
        "read_conversation_export_range" => {
            let id = args
                .get("conversation_id")
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing conversation_id".into()))?;
            let msg_id = args
                .get("message_id")
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing message_id".into()))?;
            let part_id = args
                .get("part_id")
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing part_id".into()))?;
            let field = args
                .get("field")
                .and_then(|v| v.as_str())
                .unwrap_or("content_text");
            let meta = store
                .get_kv(&crate::web_api::session_meta_key(id))
                .ok()
                .flatten()
                .unwrap_or_else(|| json!({"revision": 1}));
            let rev = meta.get("revision").and_then(|v| v.as_i64()).unwrap_or(1).max(1);
            if let Some(exp) = args.get("expected_revision").and_then(|v| v.as_i64()) {
                if exp != rev {
                    return Err((
                        "revision_conflict",
                        json!({
                            "scope_id": format!("conversation:{id}"),
                            "expected_revision": exp,
                            "actual_revision": rev
                        })
                        .to_string(),
                    ));
                }
            }
            let (raw_messages, _) = crate::web_api::reconstruct_v2_messages(store, id);
            let target_msg = raw_messages
                .iter()
                .find(|m| m.get("id").and_then(|v| v.as_str()) == Some(msg_id))
                .ok_or(("not_found", format!("message '{msg_id}' not found")))?;
            let full_text: String = if let Some(ps) = target_msg.get("parts").and_then(|v| v.as_array()) {
                let p = ps
                    .iter()
                    .enumerate()
                    .find(|(pi, p)| {
                        let pid = p
                            .get("id")
                            .or_else(|| p.get("part_id"))
                            .and_then(|v| v.as_str())
                            .map(ToString::to_string)
                            .unwrap_or_else(|| format!("{msg_id}:part-{pi}"));
                        pid == part_id
                    })
                    .map(|(_, p)| p)
                    .ok_or(("not_found", format!("part '{part_id}' not found")))?;
                if field == "content_json" {
                    p.get("content_json")
                        .or_else(|| p.get("contentJson"))
                        .map(|v| v.as_str().map(ToString::to_string).unwrap_or_else(|| v.to_string()))
                        .unwrap_or_default()
                } else {
                    p.get("content_text")
                        .or_else(|| p.get("contentText"))
                        .or_else(|| p.get("text"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string()
                }
            } else {
                if field == "content_json" {
                    String::new()
                } else {
                    target_msg
                        .get("content")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string()
                }
            };
            let total_bytes = full_text.len();
            let offset = args
                .get("offset_bytes")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize;
            let limit = args
                .get("limit_bytes")
                .and_then(|v| v.as_u64())
                .unwrap_or(32_768)
                .max(1) as usize;
            let mut start = offset.min(total_bytes);
            while start > 0 && !full_text.is_char_boundary(start) {
                start -= 1;
            }
            let mut end = start.saturating_add(limit).min(total_bytes);
            while end > start && !full_text.is_char_boundary(end) {
                end -= 1;
            }
            if end == start && start < total_bytes {
                // Ensure forward progress if limit_bytes < UTF-8 char length.
                let ch_len = full_text[start..]
                    .chars()
                    .next()
                    .map(|c| c.len_utf8())
                    .unwrap_or(0);
                end = (start + ch_len).min(total_bytes);
            }
            let slice = &full_text[start..end];
            Ok(json!({
                "revision": rev,
                "conversation_id": id,
                "message_id": msg_id,
                "part_id": part_id,
                "field": field,
                "total_utf8_bytes": total_bytes,
                "offset_bytes": start,
                "next_offset_bytes": end,
                "end_of_content": end >= total_bytes,
                "text": slice
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
            let existing_opt = store
                .get_kv(&format!("plan:{id}"))
                .map_err(|e| ("storage_error", e.to_string()))?
                .filter(|v| !v.is_null());
            if op == "save"
                || op == "upsert"
                || payload.get("todos").is_some()
                || payload.get("plan").is_some()
                || inner.get("todos").is_some()
                || inner.get("plan").is_some()
                || args.get("plan").is_some()
            {
                let cur_rev = existing_opt
                    .as_ref()
                    .and_then(|r| r.get("revision"))
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0);
                let expected_rev = payload
                    .get("expected_revision")
                    .or_else(|| inner.get("expected_revision"))
                    .or_else(|| args.get("expected_revision"))
                    .and_then(|v| v.as_i64());
                match expected_rev {
                    Some(exp) if exp != cur_rev => {
                        return Err((
                            "revision_conflict",
                            json!({
                                "scope_id": format!("conversation_plan:{id}"),
                                "expected_revision": exp.max(0) as u64,
                                "actual_revision": cur_rev.max(0) as u64
                            })
                            .to_string(),
                        ));
                    }
                    None if op == "save" && cur_rev > 0 => {
                        return Err((
                            "revision_conflict",
                            json!({
                                "scope_id": format!("conversation_plan:{id}"),
                                "expected_revision": 0u64,
                                "actual_revision": cur_rev as u64
                            })
                            .to_string(),
                        ));
                    }
                    _ => {}
                }
                let next_rev = cur_rev + 1;
                let updated_at_ms = payload
                    .get("updated_at_ms")
                    .or_else(|| inner.get("updated_at_ms"))
                    .or_else(|| args.get("updated_at_ms"))
                    .and_then(|v| v.as_i64())
                    .unwrap_or_else(now_ms);
                let todos_val = payload
                    .get("todos")
                    .or_else(|| inner.get("todos"))
                    .or_else(|| payload.get("plan"))
                    .or_else(|| inner.get("plan"))
                    .or_else(|| args.get("plan"))
                    .cloned()
                    .unwrap_or_else(|| json!([]));
                let sop_val = payload
                    .get("sop")
                    .or_else(|| inner.get("sop"))
                    .cloned()
                    .unwrap_or(Value::Null);
                let record = json!({
                    "conversation_id": id,
                    "session_id": id,
                    "todos": todos_val,
                    "plan": todos_val,
                    "sop": sop_val,
                    "revision": next_rev,
                    "updated_at_ms": updated_at_ms
                });
                store
                    .set_kv(&format!("plan:{id}"), &record)
                    .map_err(|e| ("storage_error", e.to_string()))?;
                return Ok(record);
            }
            if op == "clear" || op == "delete" {
                store
                    .set_kv(&format!("plan:{id}"), &Value::Null)
                    .map_err(|e| ("storage_error", e.to_string()))?;
                return Ok(json!({"ok": true, "conversation_id": id, "plan": Value::Null}));
            }
            if op == "get" {
                return Ok(existing_opt.unwrap_or(Value::Null));
            }
            Ok(json!({
                "conversation_id": id,
                "plan": existing_opt.unwrap_or_else(|| load_session_plan(store, id)),
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
            let found = if let Some(gid) = goal_id {
                goals.iter().find(|g| g.id == gid).cloned()
            } else if let Some(cid) = conv_id {
                let mapped_gid = store
                    .get_kv(&format!("goal_for_conv:{cid}"))
                    .ok()
                    .flatten()
                    .and_then(|v| v.as_str().map(ToString::to_string));
                goals
                    .iter()
                    .find(|g| g.id == cid || mapped_gid.as_deref() == Some(g.id.as_str()))
                    .cloned()
            } else {
                None
            };
            let Some(goal) = found else {
                return Ok(Value::Null);
            };
            let status_str = match goal.status {
                GoalStatus::Pending => "pending",
                GoalStatus::Running => "running",
                GoalStatus::Paused => "paused",
                GoalStatus::Completed => "completed",
                GoalStatus::Failed => "failed",
            };
            Ok(json!({
                "goal_id": goal.id,
                "conversation_id": conv_id,
                "control_token": format!("goal-ctrl:{}", goal.id),
                "status": status_str,
                "can_submit": matches!(goal.status, GoalStatus::Pending | GoalStatus::Completed | GoalStatus::Failed),
                "can_pause": matches!(goal.status, GoalStatus::Running),
                "can_resume": matches!(goal.status, GoalStatus::Paused),
                "can_cancel": matches!(goal.status, GoalStatus::Pending | GoalStatus::Running | GoalStatus::Paused),
                "goal": goal
            }))
        }
        "step_goal" => {
            let goal_id_opt = args
                .get("goal_id")
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty());
            if goal_id_opt.is_none() {
                if let Some(conv_id) = args
                    .get("conversation_id")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
                {
                    return Ok(json!({
                        "instruction": { "type": "idle" },
                        "conversation_id": conv_id
                    }));
                }
            }
            let goal_id = goal_id_opt
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
            if args.get("candidates").is_some() || args.get("intent").is_some() {
                return Ok(resolve_chat_model_from_candidates(args));
            }
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
            let output_limit = args.get("output_limit").and_then(|v| v.as_u64());
            let declaration = args.get("declaration").filter(|v| !v.is_null());
            let requested = args.get("requested").filter(|v| !v.is_null());
            let tier = args
                .get("tier")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty());

            // Validate declaration shape if supplied
            let mut declared_budgets: Option<[u64; 3]> = None;
            let mut declared_efforts: Vec<String> = Vec::new();
            let mut declared_control: Option<&str> = None;
            if let Some(decl) = declaration {
                let control = decl
                    .get("control")
                    .and_then(|v| v.as_str())
                    .unwrap_or("automatic");
                declared_control = Some(control);
                if control == "budget" {
                    if let Some(b_arr) = decl.get("budgets").filter(|v| !v.is_null()) {
                        let arr = b_arr.as_array().ok_or((
                            "invalid_argument",
                            "reasoning budgets declaration must be a 3-element ascending array"
                                .into(),
                        ))?;
                        if arr.len() != 3 {
                            return Err((
                                "invalid_argument",
                                "reasoning budgets declaration must contain exactly 3 budgets"
                                    .into(),
                            ));
                        }
                        let b0 = arr[0].as_u64().ok_or((
                            "invalid_argument",
                            "reasoning budget must be a positive integer".into(),
                        ))?;
                        let b1 = arr[1].as_u64().ok_or((
                            "invalid_argument",
                            "reasoning budget must be a positive integer".into(),
                        ))?;
                        let b2 = arr[2].as_u64().ok_or((
                            "invalid_argument",
                            "reasoning budget must be a positive integer".into(),
                        ))?;
                        if b0 == 0 || !(b0 < b1 && b1 < b2) {
                            return Err((
                                "invalid_argument",
                                "reasoning budgets must be positive and strictly ascending".into(),
                            ));
                        }
                        declared_budgets = Some([b0, b1, b2]);
                    } else {
                        declared_budgets = Some([1024, 4096, 8192]);
                    }
                } else if control == "effort" {
                    if let Some(e_arr) = decl.get("efforts").and_then(|v| v.as_array()) {
                        for v in e_arr {
                            if let Some(s) = v.as_str().map(str::trim).filter(|s| !s.is_empty()) {
                                declared_efforts.push(s.to_string());
                            }
                        }
                    }
                }
            }

            if !supports_reasoning {
                let has_active_control = matches!(declared_control, Some("budget" | "effort"));
                if requested.is_some() || (has_active_control && tier.is_some()) {
                    return Err((
                        "invalid_argument",
                        "selected model does not support reasoning".into(),
                    ));
                }
                return Ok(json!({ "selection": Value::Null }));
            }

            let validate_budget = |tokens: u64| -> Result<(), (&'static str, String)> {
                if tokens == 0 {
                    return Err((
                        "invalid_argument",
                        "reasoning budget must be greater than zero".into(),
                    ));
                }
                if let Some(limit) = output_limit {
                    if tokens >= limit {
                        return Err((
                            "invalid_argument",
                            format!(
                                "reasoning budget ({tokens}) must be strictly less than output limit ({limit})"
                            ),
                        ));
                    }
                }
                Ok(())
            };

            if let Some(req_val) = requested {
                let req_kind = req_val.get("kind").and_then(|v| v.as_str()).unwrap_or("");
                if req_kind == "budget" {
                    let tokens = req_val
                        .get("tokens")
                        .and_then(|v| v.as_u64())
                        .ok_or(("invalid_argument", "missing reasoning budget tokens".into()))?;
                    validate_budget(tokens)?;
                    return Ok(json!({
                        "selection": {
                            "kind": "budget",
                            "tokens": tokens
                        }
                    }));
                }
                if req_kind == "effort" {
                    let value = req_val
                        .get("value")
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.trim().is_empty())
                        .ok_or(("invalid_argument", "missing reasoning effort value".into()))?;
                    if !declared_efforts.is_empty()
                        && !declared_efforts.iter().any(|e| e == value)
                    {
                        return Err((
                            "invalid_argument",
                            format!("reasoning effort '{value}' is not declared by this model"),
                        ));
                    }
                    return Ok(json!({
                        "selection": {
                            "kind": "effort",
                            "value": value
                        }
                    }));
                }
                if let Some(s) = req_val.as_str() {
                    if s.eq_ignore_ascii_case("automatic") {
                        return Ok(json!({ "selection": Value::Null }));
                    }
                    if let Ok(tokens) = s.parse::<u64>() {
                        validate_budget(tokens)?;
                        return Ok(json!({
                            "selection": {
                                "kind": "budget",
                                "tokens": tokens
                            }
                        }));
                    }
                    return Ok(json!({
                        "selection": {
                            "kind": "effort",
                            "value": s
                        }
                    }));
                }
                return Err((
                    "invalid_argument",
                    "unsupported reasoning selection request".into(),
                ));
            }

            if let Some(t) = tier {
                let tier_lower = t.to_ascii_lowercase();
                if declared_control == Some("budget") {
                    let budgets = declared_budgets.unwrap_or([1024, 4096, 8192]);
                    let tokens = match tier_lower.as_str() {
                        "concise" | "low" | "minimal" => budgets[0],
                        "balanced" | "medium" => budgets[1],
                        "detailed" | "high" | "max" => budgets[2],
                        other => {
                            return Err((
                                "invalid_argument",
                                format!("unsupported reasoning tier '{other}'"),
                            ));
                        }
                    };
                    validate_budget(tokens)?;
                    return Ok(json!({
                        "selection": {
                            "kind": "budget",
                            "tokens": tokens
                        }
                    }));
                }
                if declared_control == Some("effort") {
                    let effort_rank = |e: &str| -> usize {
                        match e.to_ascii_lowercase().as_str() {
                            "none" => 0,
                            "minimal" => 1,
                            "low" => 2,
                            "medium" => 3,
                            "high" => 4,
                            "xhigh" => 5,
                            "max" => 6,
                            _ => 10,
                        }
                    };
                    let mut efforts = if declared_efforts.is_empty() {
                        vec!["low".into(), "medium".into(), "high".into()]
                    } else {
                        declared_efforts
                    };
                    efforts.sort_by_key(|e| effort_rank(e));
                    efforts.dedup();
                    let active_efforts: Vec<String> = efforts
                        .iter()
                        .filter(|e| !e.eq_ignore_ascii_case("none"))
                        .cloned()
                        .collect();
                    let pool = if active_efforts.is_empty() {
                        &efforts
                    } else {
                        &active_efforts
                    };
                    let chosen = match tier_lower.as_str() {
                        "concise" | "low" | "minimal" => pool.first().cloned(),
                        "balanced" | "medium" => {
                            if let Some(m) = pool.iter().find(|e| e.eq_ignore_ascii_case("medium"))
                            {
                                Some(m.clone())
                            } else if pool.len() == 2 {
                                Some(pool[0].clone())
                            } else {
                                pool.get(pool.len() / 2).cloned()
                            }
                        }
                        "detailed" | "high" | "max" => pool.last().cloned(),
                        other => {
                            return Err((
                                "invalid_argument",
                                format!("unsupported reasoning tier '{other}'"),
                            ));
                        }
                    }
                    .ok_or(("invalid_argument", "no declared reasoning efforts".into()))?;
                    return Ok(json!({
                        "selection": {
                            "kind": "effort",
                            "value": chosen
                        }
                    }));
                }
                return Ok(json!({ "selection": Value::Null }));
            }

            Ok(json!({ "selection": Value::Null }))
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
            },
            "descriptor": {
                "id": "render_ui",
                "title": "Render UI",
                "route": "tool",
                "availability": "available",
                "exposure": "visible",
                "input_schema": {"type": "object"},
                "output_policy": {
                    "preview_char_limit": 4000,
                    "persist_full_output": true,
                    "allow_artifacts": true
                },
                "supports_streaming": false,
                "parallelism": "serial",
                "cancellation": "cooperative",
                "replay_safety": "recoverable"
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
            let id = input
                .get("id")
                .or_else(|| input.get("conversation_id"))
                .or_else(|| args.get("id"))
                .or_else(|| args.get("conversation_id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(new_id);
            if cmd_type == "create_conversation" {
                if let Some(existing) = store
                    .get_session(&id)
                    .map_err(|e| ("storage_error", e.to_string()))?
                {
                    let conv_json = crate::web_api::session_to_conversation_json(store, &existing);
                    let mut val = serde_json::to_value(&existing).unwrap_or(json!({}));
                    if let Some(obj) = val.as_object_mut() {
                        obj.insert("created".into(), json!(false));
                        obj.insert("initial_configuration_revision".into(), Value::Null);
                        obj.insert("conversation".into(), conv_json);
                    }
                    return Ok(val);
                }
            }
            let s = Session {
                id,
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
            let meta_key = crate::web_api::session_meta_key(id);
            let mut meta = store
                .get_kv(&meta_key)
                .ok()
                .flatten()
                .unwrap_or_else(
                    || json!({"isPinned": false, "revision": 1, "configurationRevision": 1}),
                );
            let cur_rev = meta.get("revision").and_then(|v| v.as_u64()).unwrap_or(1).max(1);
            if let Some(exp) = args
                .get("__envelope_expected_revision")
                .or_else(|| input.get("expected_revision"))
                .or_else(|| args.get("expected_revision"))
                .and_then(|v| v.as_u64())
            {
                if exp != cur_rev {
                    return Err((
                        "revision_conflict",
                        json!({
                            "scope_id": format!("conversation:{id}"),
                            "expected_revision": exp,
                            "actual_revision": cur_rev
                        })
                        .to_string(),
                    ));
                }
            }
            let title_source = input
                .get("title_source")
                .or_else(|| input.get("titleSource"))
                .or_else(|| args.get("title_source"))
                .and_then(|v| v.as_str())
                .unwrap_or("manual");
            let mut settings_map = load_conversation_settings_str(store, id)
                .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
                .and_then(|v| v.as_object().cloned())
                .unwrap_or_default();
            if title_source == "first_message" {
                let existing_src = settings_map
                    .get("titleSource")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if matches!(existing_src, "user" | "manual") {
                    return Err((
                        "conflict",
                        "cannot overwrite manual conversation title with first_message".into(),
                    ));
                }
            }
            settings_map.insert("titleSource".into(), json!(title_source));
            if let Some(owner_id) = input
                .get("first_message_id")
                .or_else(|| args.get("first_message_id"))
                .and_then(|v| v.as_str())
            {
                settings_map.insert("titleOwnerMessageId".into(), json!(owner_id));
            }
            let merged_settings_str =
                serde_json::to_string(&Value::Object(settings_map)).unwrap_or_else(|_| "{}".into());
            let _ = store.set_kv(&format!("conv_settings:{id}"), &json!(merged_settings_str));

            let next_rev = cur_rev + 1;
            meta["revision"] = json!(next_rev);
            let _ = store.set_kv(&meta_key, &meta);

            sess.title = title.to_string();
            sess.updated_at_ms = input
                .get("updated_at_ms")
                .or_else(|| args.get("updated_at_ms"))
                .and_then(|v| v.as_i64())
                .unwrap_or_else(now_ms);
            store
                .upsert_session(&sess)
                .map_err(|e| ("storage_error", e.to_string()))?;
            let mut conv_json = crate::web_api::session_to_conversation_json(store, &sess);
            if let Some(obj) = conv_json.as_object_mut() {
                obj.insert("settings_json".into(), json!(merged_settings_str));
                obj.insert("revision".into(), json!(next_rev));
                obj.insert("__committed_revision".into(), json!(next_rev));
            }
            Ok(conv_json)
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
            let current_cfg = meta
                .get("configurationRevision")
                .or_else(|| meta.get("configuration_revision"))
                .and_then(|v| v.as_i64())
                .unwrap_or(1)
                .max(1);
            if let Some(expected_rev) = input
                .get("expected_revision")
                .or_else(|| args.get("expected_revision"))
                .and_then(|v| v.as_i64())
            {
                if expected_rev != current_cfg {
                    return Ok(json!({
                        "status": "conflict",
                        "expected_revision": expected_rev,
                        "actual_revision": current_cfg,
                        "conversation_id": id
                    }));
                }
            }
            // Validate explicit permissionProfile / permission_profile BEFORE mutating anything.
            let raw_perm_val = effective
                .get("permission_profile")
                .or_else(|| effective.get("permissionProfile"))
                .or_else(|| effective.get("profile"));
            let mut validated_perm: Option<Option<&'static str>> = None;
            if let Some(pv) = raw_perm_val {
                if pv.is_null() {
                    validated_perm = Some(None);
                } else if let Some(s) = pv.as_str() {
                    match PermissionProfile::parse_strict(s) {
                        Ok(parsed) => validated_perm = Some(Some(parsed.as_canonical_str())),
                        Err(err) => {
                            return Err((
                                "invalid_command",
                                format!("invalid permissionProfile '{s}': {err}"),
                            ));
                        }
                    }
                } else {
                    return Err((
                        "invalid_command",
                        "invalid permissionProfile: expected string or null".into(),
                    ));
                }
            }
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
            match validated_perm {
                Some(Some(canonical)) => {
                    meta["permissionProfile"] = json!(canonical);
                    let _ = store.set_kv(&format!("conv_perm:{id}"), &json!(canonical));
                }
                Some(None) => {
                    meta["permissionProfile"] = json!("cautious");
                    let _ = store.set_kv(&format!("conv_perm:{id}"), &Value::Null);
                }
                None => {}
            }
            let next_cfg = current_cfg + 1;
            meta["configurationRevision"] = json!(next_cfg);
            let _ = store.set_kv(&key, &meta);

            // Merge updated settings into `conv_settings:{id}` so `load_conversation`, `load_conversation_metadata`, & `load_conversation_settings` see them.
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
            // Handle `select_conversation_execution` selection variants explicitly.
            if let Some(selection_obj) = input
                .get("selection")
                .or_else(|| args.get("selection"))
                .and_then(|v| v.as_object())
            {
                match selection_obj.get("kind").and_then(|v| v.as_str()) {
                    Some("model") => {
                        if let Some(mid) = selection_obj
                            .get("model_id")
                            .or_else(|| selection_obj.get("id"))
                            .and_then(|v| v.as_str())
                        {
                            settings_map.insert("modelSelectionID".into(), json!(mid));
                            if let Some(params) = selection_obj.get("parameters").filter(|v| !v.is_null()) {
                                let mut mo = settings_map
                                    .get("modelOverrides")
                                    .and_then(|v| v.as_object())
                                    .cloned()
                                    .unwrap_or_default();
                                mo.insert(mid.to_string(), params.clone());
                                settings_map.insert("modelOverrides".into(), Value::Object(mo));
                            }
                            if let Some(pres) = selection_obj.get("presentation").filter(|v| !v.is_null()) {
                                let mut cmp = settings_map
                                    .get("clientModelPresentation")
                                    .and_then(|v| v.as_object())
                                    .cloned()
                                    .unwrap_or_default();
                                cmp.insert(mid.to_string(), pres.clone());
                                settings_map.insert("clientModelPresentation".into(), Value::Object(cmp));
                            }
                        }
                    }
                    Some("host") => {
                        if let Some(host) = selection_obj
                            .get("execution_host")
                            .or_else(|| selection_obj.get("host"))
                            .and_then(|v| v.as_str())
                        {
                            settings_map.insert("executionHostRawValue".into(), json!(host));
                        }
                        if let Some(params) = selection_obj.get("parameters").filter(|v| !v.is_null()) {
                            settings_map.insert("clientHostParameters".into(), params.clone());
                        }
                    }
                    Some("inherit") => {
                        settings_map.remove("modelSelectionID");
                    }
                    Some("location") => {
                        if let Some(host) = selection_obj
                            .get("execution_host")
                            .and_then(|v| v.as_str())
                        {
                            settings_map.insert("executionHostRawValue".into(), json!(host));
                        }
                    }
                    _ => {}
                }
            }
            if let Some(host) = input
                .get("execution_host")
                .or_else(|| args.get("execution_host"))
                .and_then(|v| v.as_str())
            {
                settings_map.insert("executionHostRawValue".into(), json!(host));
            }
            // Apply patch_json / patch object recursively (RFC 7396 merge patch:
            // nested objects merge recursively and null values remove keys while preserving
            // an empty parent map if nested keys are cleared).
            if let Some(patch_src) = input
                .get("patch_json")
                .or_else(|| input.get("patch"))
                .or_else(|| args.get("patch_json"))
                .or_else(|| args.get("patch"))
            {
                let parsed_patch = if let Some(s) = patch_src.as_str() {
                    serde_json::from_str::<Value>(s).ok()
                } else {
                    Some(patch_src.clone())
                };
                if let Some(Value::Object(map)) = parsed_patch {
                    fn apply_sparse_patch(
                        target: &mut serde_json::Map<String, Value>,
                        patch: serde_json::Map<String, Value>,
                    ) {
                        for (k, v) in patch {
                            if v.is_null() {
                                target.remove(&k);
                            } else if let Value::Object(child_patch) = v {
                                let mut child_target = target
                                    .get(&k)
                                    .and_then(|x| x.as_object())
                                    .cloned()
                                    .unwrap_or_default();
                                apply_sparse_patch(&mut child_target, child_patch);
                                target.insert(k, Value::Object(child_target));
                            } else {
                                target.insert(k, v);
                            }
                        }
                    }
                    apply_sparse_patch(&mut settings_map, map);
                }
            }
            // Normalize permission fields in settings_map: store canonical `permissionModeRawValue`
            // and never leave redundant `permissionProfile` or stale raw values on null reset.
            match validated_perm {
                Some(Some(canonical)) => {
                    settings_map.insert("permissionModeRawValue".into(), json!(canonical));
                    settings_map.remove("permissionProfile");
                    settings_map.remove("permission_profile");
                }
                Some(None) => {
                    settings_map.remove("permissionModeRawValue");
                    settings_map.remove("permissionProfile");
                    settings_map.remove("permission_profile");
                }
                None => {}
            }
            let merged_settings_str =
                serde_json::to_string(&Value::Object(settings_map.clone()))
                    .unwrap_or_else(|_| "{}".into());
            let _ = store.set_kv(&format!("conv_settings:{id}"), &json!(merged_settings_str));

            sess.updated_at_ms = now_ms();
            store
                .upsert_session(&sess)
                .map_err(|e| ("storage_error", e.to_string()))?;
            let mut conv_json = crate::web_api::session_to_conversation_json(store, &sess);
            if let Some(obj) = conv_json.as_object_mut() {
                for (k, v) in &settings_map {
                    obj.insert(k.clone(), v.clone());
                }
                obj.insert("settings_json".into(), json!(merged_settings_str));
                obj.insert("configurationRevision".into(), json!(next_cfg));
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
            let Some(sess) = store
                .get_session(id)
                .map_err(|e| ("storage_error", e.to_string()))?
            else {
                return Ok(json!({
                    "status": "not_found",
                    "conversation_id": id
                }));
            };
            let meta_key = crate::web_api::session_meta_key(id);
            let mut meta = store
                .get_kv(&meta_key)
                .ok()
                .flatten()
                .unwrap_or_else(
                    || json!({"isPinned": false, "revision": 1, "configurationRevision": 1}),
                );
            let cur_cfg = meta
                .get("configurationRevision")
                .or_else(|| meta.get("configuration_revision"))
                .and_then(|v| v.as_i64())
                .unwrap_or(1)
                .max(1);
            if let Some(exp) = input
                .get("expected_revision")
                .or_else(|| args.get("expected_revision"))
                .and_then(|v| v.as_i64())
            {
                if exp != cur_cfg {
                    return Ok(json!({
                        "status": "conflict",
                        "expected_revision": exp,
                        "actual_revision": cur_cfg,
                        "conversation_id": id
                    }));
                }
            }
            store
                .set_kv(&format!("conv_perm:{id}"), &json!(canonical_mode))
                .map_err(|e| ("storage_error", e.to_string()))?;
            meta["permissionProfile"] = json!(canonical_mode);
            let next_cfg = cur_cfg + 1;
            meta["configurationRevision"] = json!(next_cfg);
            let _ = store.set_kv(&meta_key, &meta);

            // Also update conv_settings:{id} so load_conversation / settings_json reflects the new mode.
            let mut settings_map = load_conversation_settings_str(store, id)
                .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
                .and_then(|v| v.as_object().cloned())
                .unwrap_or_default();
            settings_map.insert("permissionProfile".into(), json!(canonical_mode));
            settings_map.insert("permissionModeRawValue".into(), json!(canonical_mode));
            let merged_settings_str =
                serde_json::to_string(&Value::Object(settings_map.clone()))
                    .unwrap_or_else(|_| "{}".into());
            let _ = store.set_kv(&format!("conv_settings:{id}"), &json!(merged_settings_str));

            let mut conv_json = crate::web_api::session_to_conversation_json(store, &sess);
            if let Some(obj) = conv_json.as_object_mut() {
                obj.insert("settings_json".into(), json!(merged_settings_str));
                obj.insert("configurationRevision".into(), json!(next_cfg));
                obj.insert("configuration_revision".into(), json!(next_cfg));
            }
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
            let occ_id = args
                .get("occurrence_id")
                .or_else(|| args.get("run_id"))
                .or_else(|| args.get("execution_id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'occurrence_id'".into()))?;
            let run_rec = suite
                .get_record::<Value>("automation_run", occ_id)
                .map_err(|e| ("storage_error", e.to_string()))?;
            let obs_state = store
                .get_kv(&format!("automation_exec_state:{occ_id}"))
                .ok()
                .flatten()
                .unwrap_or_else(|| json!({}));
            let latest_worker = obs_state
                .get("latest_worker_observation")
                .cloned()
                .filter(|v| !v.is_null());
            let binding = obs_state.get("binding").cloned().filter(|v| !v.is_null());
            let receipt = obs_state.get("receipt").cloned().filter(|v| !v.is_null());
            let terminal = obs_state.get("terminal").cloned().filter(|v| !v.is_null());
            let notification_delivered = obs_state
                .get("notification_delivered")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let run_status = run_rec
                .as_ref()
                .and_then(|r| r.get("status").and_then(|v| v.as_str()))
                .map(ToString::to_string);
            let phase = if terminal.is_some()
                || matches!(
                    run_status.as_deref(),
                    Some("completed" | "failed" | "cancelled")
                ) {
                "terminal"
            } else if run_status.as_deref() == Some("launch_failed")
                || latest_worker
                    .as_ref()
                    .and_then(|w| w.get("kind").and_then(|v| v.as_str()))
                    == Some("launch_failed")
            {
                "launch_failed"
            } else if binding.is_some() {
                "admitted"
            } else if let Some(kind) = latest_worker
                .as_ref()
                .and_then(|w| w.get("kind").and_then(|v| v.as_str()))
            {
                match kind {
                    "preparing" => "preparing",
                    "admission_requested" | "exited" | "disconnected" => "admission_pending",
                    "launch_failed" => "launch_failed",
                    _ => "claimed",
                }
            } else if run_status.as_deref() == Some("running") {
                "claimed"
            } else {
                "queued"
            };
            let identity = obs_state
                .get("identity")
                .cloned()
                .filter(|v| v.is_object())
                .unwrap_or_else(|| {
                    json!({
                        "conversation_id": format!("automation:{occ_id}:conversation"),
                        "submission_id": format!("automation:{occ_id}:submission"),
                        "user_message_id": format!("automation:{occ_id}:user"),
                        "assistant_message_id": format!("automation:{occ_id}:assistant"),
                    })
                });
            Ok(json!({
                "phase": phase,
                "latest_worker_observation": latest_worker,
                "identity": identity,
                "binding": binding,
                "run_status": run_status,
                "terminal": terminal,
                "receipt": receipt,
                "notification_delivered": notification_delivered,
            }))
        }
        "list_pending_automation_executions" => {
            let after = args.get("after_occurrence_id").and_then(|v| v.as_str());
            let limit = args
                .get("limit")
                .and_then(|v| v.as_u64())
                .unwrap_or(200) as usize;
            let runs: Vec<Value> = suite
                .list_records("automation_run")
                .map_err(|e| ("storage_error", e.to_string()))?;
            let mut ids: Vec<String> = runs
                .into_iter()
                .filter(|r| {
                    matches!(
                        r.get("status").and_then(|v| v.as_str()).unwrap_or("queued"),
                        "queued" | "pending" | "running"
                    )
                })
                .filter_map(|r| r.get("id").and_then(|v| v.as_str()).map(ToString::to_string))
                .collect();
            ids.sort();
            if let Some(after_id) = after {
                ids.retain(|id| id.as_str() > after_id);
            }
            ids.truncate(limit);
            Ok(json!({
                "executions": ids,
                "occurrences": ids,
                "items": ids
            }))
        }
        "reconcile_automation_execution" => {
            let input = args.get("input").unwrap_or(args);
            let occ_id = input
                .get("occurrence_id")
                .or_else(|| input.get("run_id"))
                .or_else(|| input.get("execution_id"))
                .or_else(|| input.get("id"))
                .or_else(|| args.get("occurrence_id"))
                .or_else(|| args.get("run_id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'occurrence_id'".into()))?;
            let reconciled_at = input
                .get("reconciled_at_ms")
                .or_else(|| args.get("reconciled_at_ms"))
                .and_then(|v| v.as_i64())
                .unwrap_or_else(now_ms);
            let key = format!("automation_exec_state:{occ_id}");
            let mut obs_state = store
                .get_kv(&key)
                .ok()
                .flatten()
                .unwrap_or_else(|| json!({}));
            if let Some(existing_receipt) = obs_state
                .get("receipt")
                .cloned()
                .filter(|v| !v.is_null())
            {
                return Ok(existing_receipt);
            }
            // Only a bound occurrence whose bound assistant run reached a terminal seal
            // can produce a receipt. Unbound occurrences MUST NOT manufacture a result.
            let Some(binding) = obs_state.get("binding").cloned().filter(|v| v.is_object()) else {
                return Ok(Value::Null);
            };
            let Some(terminal) = obs_state.get("terminal").cloned().filter(|v| v.is_object()) else {
                return Ok(Value::Null);
            };
            let receipt = json!({
                "binding": binding,
                "terminal": terminal,
                "reconciled_at_ms": reconciled_at,
                "notification_id": format!("notif:{occ_id}")
            });
            if let Some(obj) = obs_state.as_object_mut() {
                obj.insert("receipt".into(), receipt.clone());
            }
            let _ = store.set_kv(&key, &obs_state);
            Ok(receipt)
        }
        "observe_automation_worker" => {
            let input = args.get("input").unwrap_or(args);
            let Some(lease) = input.get("lease").or_else(|| args.get("lease")) else {
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
                return Ok(json!({
                    "status": if running_count > 0 { "active" } else { "idle" },
                    "pending_count": pending_count,
                    "running_count": running_count,
                    "observed_at_ms": now_ms()
                }));
            };
            let occ_id = lease
                .get("occurrence_id")
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'lease.occurrence_id'".into()))?;
            let worker_id = lease.get("worker_id").and_then(|v| v.as_str());
            let lease_token = lease.get("lease_token").and_then(|v| v.as_str());
            let observation = input
                .get("observation")
                .or_else(|| args.get("observation"))
                .cloned()
                .ok_or(("invalid_argument", "missing 'observation'".into()))?;
            let observed_at_ms = input
                .get("observed_at_ms")
                .or_else(|| args.get("observed_at_ms"))
                .and_then(|v| v.as_i64())
                .unwrap_or_else(now_ms);
            let Some(mut run_rec) = suite
                .get_record::<Value>("automation_run", occ_id)
                .map_err(|e| ("storage_error", e.to_string()))?
            else {
                return Ok(json!(false));
            };
            let owner_matches = run_rec.get("worker_id").and_then(|v| v.as_str()) == worker_id
                && run_rec.get("lease_token").and_then(|v| v.as_str()) == lease_token;
            if !owner_matches {
                return Ok(json!(false));
            }
            let obs_kind = observation
                .get("kind")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let key = format!("automation_exec_state:{occ_id}");
            let mut obs_state = store
                .get_kv(&key)
                .ok()
                .flatten()
                .unwrap_or_else(|| json!({}));
            if let Some(obj) = obs_state.as_object_mut() {
                obj.insert("latest_worker_observation".into(), observation.clone());
                obj.insert("observed_at_ms".into(), json!(observed_at_ms));
                if matches!(
                    obs_kind,
                    "exited" | "disconnected" | "admission_requested" | "launch_failed"
                ) {
                    obj.insert("uncertain".into(), json!(true));
                }
            }
            let _ = store.set_kv(&key, &obs_state);
            if let Some(obj) = run_rec.as_object_mut() {
                if obs_kind == "launch_failed" {
                    obj.insert("status".into(), json!("launch_failed"));
                    obj.insert("finished_at_ms".into(), json!(observed_at_ms));
                    obj.insert("lease_token".into(), Value::Null);
                    obj.insert("lease_until_ms".into(), Value::Null);
                }
                obj.insert("updated_at_ms".into(), json!(observed_at_ms));
            }
            suite
                .upsert_record("automation_run", occ_id, &run_rec)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!(true))
        }
        "ack_automation_notification" => {
            let input = args.get("input").unwrap_or(args);
            let occ_id = input
                .get("occurrence_id")
                .or_else(|| args.get("occurrence_id"))
                .and_then(|v| v.as_str());
            let id = input
                .get("notification_id")
                .or_else(|| input.get("id"))
                .or_else(|| args.get("notification_id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .unwrap_or("default");
            let ts = input
                .get("delivered_at_ms")
                .or_else(|| args.get("delivered_at_ms"))
                .and_then(|v| v.as_i64())
                .unwrap_or_else(now_ms);
            if let Some(oid) = occ_id {
                let key = format!("automation_exec_state:{oid}");
                let mut obs_state = store
                    .get_kv(&key)
                    .ok()
                    .flatten()
                    .unwrap_or_else(|| json!({}));
                if let Some(obj) = obs_state.as_object_mut() {
                    obj.insert("notification_delivered".into(), json!(true));
                    obj.insert("notification_id".into(), json!(id));
                    obj.insert("notification_delivered_at_ms".into(), json!(ts));
                }
                let _ = store.set_kv(&key, &obs_state);
                return Ok(json!(true));
            }
            store
                .set_kv(
                    &format!("automation_notif_ack:{id}"),
                    &json!({"id": id, "acknowledged_at_ms": ts}),
                )
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"ok": true, "acknowledged": true, "id": id, "acknowledged_at_ms": ts}))
        }
        "query_automation_occurrences" => {
            let input = args.get("input").unwrap_or(args);
            let def_id = input
                .get("automation_id")
                .or_else(|| input.get("definition_id"))
                .or_else(|| args.get("automation_id"))
                .or_else(|| args.get("definition_id"))
                .and_then(|v| v.as_str());
            let after_ms = input
                .get("after_ms")
                .or_else(|| args.get("after_ms"))
                .and_then(|v| v.as_i64());
            let through_ms = input
                .get("through_ms")
                .or_else(|| args.get("through_ms"))
                .and_then(|v| v.as_i64());
            let limit = input
                .get("limit")
                .or_else(|| args.get("limit"))
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
                .filter(|r| {
                    let sched = r.get("scheduled_for_ms").and_then(|v| v.as_i64());
                    after_ms.is_none_or(|a| sched.is_some_and(|s| s > a))
                        && through_ms.is_none_or(|t| sched.is_some_and(|s| s <= t))
                })
                .take(limit)
                .collect();
            Ok(json!({"occurrences": runs, "items": runs}))
        }
        "inspect_mobile_permission_request" => {
            let input = args.get("input").unwrap_or(args);
            let request = input
                .get("request")
                .or_else(|| args.get("request"))
                .ok_or(("invalid_argument", "missing 'request'".into()))?;
            let payload = request
                .get("payload")
                .cloned()
                .ok_or(("invalid_argument", "missing 'request.payload'".into()))?;
            let perm_req_id = payload
                .get("permission_request_id")
                .or_else(|| payload.get("request_id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'permission_request_id'".into()))?;
            let req_session_id = request
                .get("session_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let req_effect_id = request
                .get("effect_id")
                .cloned()
                .unwrap_or(Value::Null);
            let req_effect_str = req_effect_id.as_str().unwrap_or("");
            let cap_id = payload
                .get("capability_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let mut matched_run: Option<Value> = None;
            let indexed_run_id = store
                .get_kv(&format!("perm_request_run:{perm_req_id}"))
                .ok()
                .flatten()
                .and_then(|v| v.as_str().map(ToString::to_string))
                .or_else(|| {
                    if !req_effect_str.is_empty() {
                        store
                            .get_kv(&format!("effect_run:{req_effect_str}"))
                            .ok()
                            .flatten()
                            .and_then(|v| v.as_str().map(ToString::to_string))
                    } else {
                        None
                    }
                })
                .or_else(|| {
                    if !req_session_id.is_empty() {
                        store
                            .get_kv(&format!("active_run:{req_session_id}"))
                            .ok()
                            .flatten()
                            .and_then(|v| v.get("run_id").and_then(|r| r.as_str()).map(ToString::to_string))
                    } else {
                        None
                    }
                });
            if let Some(rid) = indexed_run_id {
                matched_run = store.get_kv(&format!("committed_run:{rid}")).ok().flatten();
            }
            let run = matched_run.ok_or((
                "not_found",
                format!("permission request '{perm_req_id}' not found in active runs"),
            ))?;
            let conv_id = run
                .get("conversation_id")
                .and_then(|v| v.as_str())
                .unwrap_or(req_session_id);
            let run_id = run.get("run_id").and_then(|v| v.as_str()).unwrap_or("");
            let core_session_id = if !req_session_id.is_empty() {
                req_session_id
            } else {
                run.get("session_id").and_then(|v| v.as_str()).unwrap_or(conv_id)
            };
            let capability = run
                .pointer("/core_config/host_catalog/capabilities")
                .and_then(|c| c.get(cap_id))
                .cloned()
                .unwrap_or_else(|| json!({"id": cap_id, "title": cap_id}));
            let permission_domain = capability
                .pointer("/input_normalization/permission_contract/domain")
                .or_else(|| capability.get("permission_domain"))
                .cloned()
                .unwrap_or(Value::Null);
            let display_title = capability
                .get("title")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .unwrap_or(cap_id);
            Ok(json!({
                "conversation_id": conv_id,
                "run_id": run_id,
                "core_session_id": core_session_id,
                "effect_id": req_effect_id,
                "request_id": perm_req_id,
                "payload": payload,
                "capability": capability,
                "permission_domain": permission_domain,
                "display_title": display_title
            }))
        }
        "load_conversation_metadata" => {
            let id = args
                .get("conversation_id")
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing conversation_id".into()))?;
            let Some(sess) = get_session_case_insensitive(store, id)
                .map_err(|e| ("storage_error", e.to_string()))?
            else {
                return Ok(Value::Null);
            };
            let eff_id = sess.id.as_str();
            let meta = store
                .get_kv(&format!("conv_metadata:{eff_id}"))
                .ok()
                .flatten()
                .unwrap_or_else(|| json!({}));
            let sess_meta = store
                .get_kv(&crate::web_api::session_meta_key(eff_id))
                .ok()
                .flatten()
                .unwrap_or_else(|| json!({}));
            let rev = sess_meta
                .get("revision")
                .and_then(|v| v.as_i64())
                .unwrap_or(1)
                .max(1);
            let config_rev = sess_meta
                .get("configurationRevision")
                .or_else(|| sess_meta.get("configuration_revision"))
                .and_then(|v| v.as_i64())
                .unwrap_or(1)
                .max(1);
            let settings_json = load_conversation_settings_str(store, eff_id);
            let conv = crate::web_api::session_to_conversation_json(store, &sess);
            Ok(json!({
                "id": eff_id,
                "conversation_id": eff_id,
                "metadata": meta,
                "conversation": conv,
                "revision": rev,
                "configuration_revision": config_rev,
                "settings_json": settings_json
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
            let grants = load_durable_permission_grants(store, conv_id);
            let rules: Vec<Value> = grants
                .iter()
                .filter_map(|g| {
                    let cap = g
                        .get("capability_pattern")
                        .or_else(|| g.get("capability"))
                        .or_else(|| g.get("tool"))
                        .and_then(|v| v.as_str())?;
                    let res = g
                        .get("resource_pattern")
                        .or_else(|| g.get("resource"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("*");
                    let decision = g
                        .get("decision")
                        .or_else(|| g.get("action"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("allow");
                    let source = g
                        .get("source")
                        .and_then(|v| v.as_str())
                        .unwrap_or("grant");
                    Some(json!({
                        "capability_pattern": cap,
                        "resource_pattern": res,
                        "decision": decision,
                        "source": source
                    }))
                })
                .collect();
            Ok(json!({
                "conversation_id": conv_id,
                "permission_profile": profile,
                "grants": grants,
                "rules": rules,
                "revision": rev
            }))
        }
        "canonical_run_terminal" => {
            let run_id = args
                .get("run_id")
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str());
            let conv_id = args
                .get("conversation_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let stored = run_id.and_then(|rid| {
                store
                    .get_kv(&format!("run_terminal:{rid}"))
                    .ok()
                    .flatten()
                    .or_else(|| {
                        if conv_id.is_empty() {
                            return None;
                        }
                        store
                            .get_kv(&format!("run_terminal_seal:{conv_id}:{rid}"))
                            .ok()
                            .flatten()
                            .map(|seal| {
                                json!({
                                    "conversation_id": seal.pointer("/identity/conversation_id").and_then(|v| v.as_str()).unwrap_or(conv_id),
                                    "run_id": seal.pointer("/identity/run_id").and_then(|v| v.as_str()).unwrap_or(rid),
                                    "assistant_message_id": seal.pointer("/identity/assistant_message_id").and_then(|v| v.as_str()).unwrap_or(""),
                                    "status": "sealed",
                                    "result": seal.get("result").cloned().unwrap_or(Value::Null),
                                    "result_digest": seal.get("result_digest").cloned().unwrap_or(Value::Null),
                                    "completed_at_ms": seal.get("completed_at_ms").cloned().unwrap_or_else(|| json!(now_ms()))
                                })
                            })
                    })
            });
            Ok(stored.unwrap_or(Value::Null))
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
            if items.is_empty()
                && (args.get("expected_run_id").is_some()
                    || args.get("assistant_message_id").is_some())
            {
                return Ok(Value::Null);
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
            let active = lookup_active_run(store, conv_id);
            let active_run_id = active
                .as_ref()
                .and_then(|a| a.get("run_id").and_then(|v| v.as_str()));
            let committed = active_run_id
                .and_then(|rid| store.get_kv(&format!("committed_run:{rid}")).ok().flatten());
            let core_session_id = committed
                .as_ref()
                .and_then(|r| r.get("session_id").and_then(|v| v.as_str()))
                .unwrap_or(conv_id);
            let checkpoint_rev = committed
                .as_ref()
                .and_then(|r| r.get("checkpoint_revision").and_then(|v| v.as_str()))
                .unwrap_or("1");
            let checkpoint_cursor = committed
                .as_ref()
                .and_then(|r| r.get("checkpoint_event_cursor").and_then(|v| v.as_str()))
                .unwrap_or("1");
            Ok(json!({
                "conversation_id": conv_id,
                "core_session_id": core_session_id,
                "checkpoint_revision": checkpoint_rev,
                "checkpoint_event_cursor": checkpoint_cursor,
                "task_projection": {
                    "revision": 0,
                    "tasks": {},
                    "guidance": {},
                    "order": []
                },
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
            let op_id = input
                .get("operation_id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(new_id);
            let op_key = format!("prepared_in_place_resend_op:{op_id}");
            // Idempotent by operation_id: replay exact prepared record without re-truncating.
            if let Ok(Some(existing)) = store.get_kv(&op_key) {
                return Ok(existing);
            }
            let target_msg_id = input.get("message_id").and_then(|v| v.as_str());
            let explicit_from_seq = input
                .get("from_seq")
                .or_else(|| input.get("truncate_from_seq"))
                .or_else(|| input.get("target_seq"))
                .and_then(|v| v.as_i64());
            let truncate_seq = if let Some(seq) = explicit_from_seq {
                Some(seq)
            } else if let Some(mid) = target_msg_id {
                let (msgs, _) = crate::web_api::reconstruct_v2_messages(store, conv_id);
                msgs.iter()
                    .find(|m| m.get("id").and_then(|v| v.as_str()) == Some(mid))
                    .and_then(|m| m.get("seq").and_then(|v| v.as_i64()))
            } else {
                None
            };
            if let Some(from_seq) = truncate_seq {
                store
                    .truncate_events_from(conv_id, from_seq)
                    .map_err(|e| ("storage_error", e.to_string()))?;
                synchronize_session_state_after_truncate(store, conv_id);
            }
            let experts = input
                .get("selected_experts")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            let prepared = json!({
                "input": input,
                "run_id": op_id,
                "user_message_id": format!("{op_id}:user"),
                "experts": experts,
                "deferred_rewind": false
            });
            store
                .set_kv(&op_key, &prepared)
                .map_err(|e| ("storage_error", e.to_string()))?;
            store
                .set_kv(&format!("prepared_in_place_resend:{conv_id}"), &prepared)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(prepared)
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
            let op_id = args
                .get("operation_id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty());
            let conv_id = args
                .get("conversation_id")
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty());
            if let Some(oid) = op_id {
                if let Some(found) = store
                    .get_kv(&format!("prepared_in_place_resend_op:{oid}"))
                    .map_err(|e| ("storage_error", e.to_string()))?
                {
                    return Ok(found);
                }
            }
            if let Some(cid) = conv_id {
                let found = store
                    .get_kv(&format!("prepared_in_place_resend:{cid}"))
                    .map_err(|e| ("storage_error", e.to_string()))?;
                return Ok(found.unwrap_or(Value::Null));
            }
            if op_id.is_some() {
                return Ok(Value::Null);
            }
            Err(("invalid_argument", "missing conversation_id or operation_id".into()))
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
            let assistant_id = receipt
                .get("identity")
                .and_then(|i| i.get("assistant_message_id"))
                .and_then(|v| v.as_str())
                .map(ToString::to_string)
                .unwrap_or_else(|| format!("{sub_id}:assistant"));
            let source_host_id = receipt
                .get("identity")
                .and_then(|i| i.get("source_host_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("remote");
            store
                .set_kv(&format!("observed_remote_run:{conv_id}:{sub_id}"), &receipt)
                .map_err(|e| ("storage_error", e.to_string()))?;
            register_active_run(
                store,
                conv_id,
                json!({
                    "conversation_id": conv_id,
                    "run_id": sub_id,
                    "kind": "remote",
                    "assistant_message_id": assistant_id,
                    "source_host_id": source_host_id,
                    "next_event_sequence": 1i64,
                    "invocation_config_digest": receipt.get("request_digest").cloned().unwrap_or(Value::Null),
                    "status": "running"
                }),
            );
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
                "observation": {
                    "state": state,
                    "source_host_id": Value::Null
                },
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
        "peek_agent_command" | "promote_queued_assistant_turn" | "apply_pending_task_notice" => {
            Ok(Value::Null)
        }
        "load_run_identity" => {
            let run_id = args
                .get("run_id")
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty());
            if let Some(rid) = run_id {
                if let Ok(Some(saved)) = store.get_kv(&format!("run_identity:{rid}")) {
                    let conv_id = saved
                        .get("conversation_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    return Ok(json!({
                        "run_id": rid,
                        "conversation_id": conv_id
                    }));
                }
            }
            Ok(Value::Null)
        }
        "read_expert_exchange_capability" => {
            Ok(json!({
                "supported": true,
                "version": 1,
                "tool": {
                    "type": "function",
                    "function": {
                        "name": "delegate_expert",
                        "description": "Delegate a focused task to a joined expert in the current room.",
                        "parameters": {
                            "type": "object",
                            "properties": {
                                "expert_id": {"type": "string"},
                                "task": {"type": "string"}
                            },
                            "required": ["expert_id", "task"]
                        }
                    }
                },
                "delegation_tool": {
                    "name": "delegate_to_expert",
                    "enabled": true,
                    "main_only": true
                },
                "features": {
                    "main_agent_delegation": true,
                    "inline_expert_mentions": true,
                    "rejoin": true
                }
            }))
        }
        "load_child_instructions" => {
            let conv_id = args
                .get("conversation_id")
                .or_else(|| args.get("input").and_then(|i| i.get("conversation_id")))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let stored = if !conv_id.is_empty() {
                store
                    .get_kv(&format!("child_instructions:{conv_id}"))
                    .ok()
                    .flatten()
            } else {
                None
            };
            Ok(stored.unwrap_or(Value::Null))
        }
        "encode_host_worker_identity" => {
            let input = args.get("input").unwrap_or(args);
            let identity = input.get("identity").or_else(|| args.get("identity"));
            let nonce = identity
                .and_then(|i| i.get("nonce").and_then(|v| v.as_str()))
                .filter(|s| !s.is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(new_id);
            let owner = input
                .get("lease_owner")
                .or_else(|| input.get("owner"))
                .or_else(|| args.get("lease_owner"))
                .and_then(|v| v.as_str())
                .map(ToString::to_string)
                .unwrap_or_else(|| format!("owner:{nonce}"));
            let gen = input
                .get("worker_generation")
                .or_else(|| args.get("worker_generation"))
                .and_then(|v| v.as_str())
                .map(ToString::to_string)
                .unwrap_or_else(|| format!("gen:{nonce}"));
            Ok(json!({
                "worker_generation": gen,
                "lease_owner": owner
            }))
        }
        "load_session_goal_projection" => {
            let conv_id = args
                .get("conversation_id")
                .or_else(|| args.get("session_id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let goals: Vec<GoalRecord> = suite
                .list_records("goal")
                .map_err(|e| ("storage_error", e.to_string()))?;
            let mapped_gid = store
                .get_kv(&format!("goal_for_conv:{conv_id}"))
                .ok()
                .flatten()
                .and_then(|v| v.as_str().map(ToString::to_string));
            let Some(active_goal) = goals
                .into_iter()
                .find(|g| g.id == conv_id || mapped_gid.as_deref() == Some(g.id.as_str()))
            else {
                return Ok(Value::Null);
            };
            Ok(json!({
                "conversation_id": conv_id,
                "active_goal": active_goal,
                "goal": active_goal,
                "status": format!("{:?}", active_goal.status).to_lowercase(),
                "steps": []
            }))
        }
        "step_compaction_summary" => dispatch_step_compaction_summary(args),
        "evaluate_compaction_maintenance" => dispatch_evaluate_compaction_maintenance(args),
        "load_auxiliary_source_slot" => dispatch_load_auxiliary_source_slot(store, args),
        "auxiliary_invocation" => dispatch_auxiliary_invocation(store, args),
        "register_source_submission" => dispatch_register_source_submission(store, args),
        "reject_source_submission" => dispatch_reject_source_submission(store, args),
        "read_source_submission_receipt" => dispatch_read_source_submission_receipt(store, args),
        "begin_assistant_turn" | "begin_automation_assistant_turn" => {
            dispatch_begin_assistant_turn(store, suite, cmd_type, args)
        }
        "claim_committed_host_requests" => dispatch_claim_committed_host_requests(store, args),
        "load_committed_host_requests" => dispatch_load_committed_host_requests(store, args),
        "reduce_and_accept_host_response" => {
            dispatch_reduce_and_accept_host_response(store, args)
        }
        "reduce_and_commit_active_submission" => {
            dispatch_reduce_and_commit_active_submission(store, args)
        }
        "local_run_message_admission_head" => dispatch_local_run_message_admission_head(store, args),
        "read_run_message_admissions" => dispatch_read_run_message_admissions(store, args),
        "run_message_admission_reception_state" => {
            dispatch_run_message_admission_reception_state(store, args)
        }
        "receive_run_message_admissions" => dispatch_receive_run_message_admissions(store, args),
        "local_run_assistant_journal_heads" => {
            dispatch_local_run_assistant_journal_heads(store, args)
        }
        "local_assistant_journal_head" => dispatch_local_assistant_journal_head(store, args),
        "open_assistant_journal_snapshot" => dispatch_open_assistant_journal_snapshot(args),
        "release_assistant_journal_snapshot" => Ok(json!({"ok": true})),
        "read_assistant_journal" => dispatch_read_assistant_journal(store, args),
        "read_assistant_journal_record_chunk" => {
            dispatch_read_assistant_journal_record_chunk(store, args)
        }
        "assistant_journal_reception_state" => {
            dispatch_assistant_journal_reception_state(store, args)
        }
        "next_assistant_journal_content" => Ok(Value::Null),
        "receive_assistant_journal_record" => dispatch_receive_assistant_journal_record(store, args),
        "local_assistant_run_terminal_seal" => {
            dispatch_local_assistant_run_terminal_seal(store, args)
        }
        "read_local_assistant_run_terminal_manifest" => {
            dispatch_read_local_assistant_run_terminal_manifest(store, args)
        }
        "load_run_delivery_ownership" => dispatch_load_run_delivery_ownership(store, args),
        "prepare_run_effect_recovery" => Ok(json!({"rebound_count": 0})),
        "load_runtime_effect_work_schedule" => {
            dispatch_load_runtime_effect_work_schedule(store, args)
        }
        "recover_expired_run_deliveries" => dispatch_recover_expired_run_deliveries(store, args),
        "load_host_execution_state" => dispatch_load_host_execution_state(store, args),
        "heartbeat_effect_delivery" => dispatch_heartbeat_effect_delivery(store, args),
        "inspect_run_process_owners" => dispatch_inspect_run_process_owners(store, args),
        "validate_run_process_observations" => Ok(Value::Bool(false)),
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
                .or_else(|| input.get("conversationID"))
                .or_else(|| input.get("id"))
                .or_else(|| args.get("conversation_id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(new_id);
            let existing_msgs = crate::web_api::reconstruct_v2_messages(store, &conv_id).0;
            let existing_count = existing_msgs.len();
            if let Some(expected_count) = input
                .get("expected_message_count")
                .or_else(|| input.get("expectedMessageCount"))
                .and_then(|v| v.as_u64())
            {
                if existing_count != expected_count as usize {
                    return Err((
                        "conflict",
                        format!(
                            "expected_message_count {expected_count} does not match existing count {existing_count}"
                        ),
                    ));
                }
            }
            let title = input
                .get("title")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .unwrap_or("Imported Conversation");
            let ts = input
                .get("imported_at_ms")
                .or_else(|| input.get("importedAtMs"))
                .and_then(|v| v.as_i64())
                .unwrap_or_else(now_ms);
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
                let mut content = entry
                    .get("content")
                    .or_else(|| entry.get("text"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let parts = entry.get("parts").and_then(|v| v.as_array()).cloned();
                if content.is_empty() {
                    if let Some(ref ps) = parts {
                        content = ps
                            .iter()
                            .filter_map(|p| {
                                p.get("content_text")
                                    .or_else(|| p.get("contentText"))
                                    .or_else(|| p.get("text"))
                                    .and_then(|v| v.as_str())
                            })
                            .collect::<Vec<_>>()
                            .join("\n");
                    }
                }
                let kind = if role.eq_ignore_ascii_case("assistant") {
                    "assistant_message"
                } else {
                    "user_message"
                };
                let mut payload = json!({
                    "role": role,
                    "content": content,
                    "imported": true
                });
                if let Some(mid) = entry.get("id").and_then(|v| v.as_str()) {
                    payload["id"] = json!(mid);
                }
                if let Some(ps) = parts {
                    payload["parts"] = Value::Array(ps);
                }
                if let Some(created) = entry
                    .get("created_at_ms")
                    .or_else(|| entry.get("createdAtMs"))
                    .and_then(|v| v.as_i64())
                {
                    payload["created_at_ms"] = json!(created);
                }
                let _ = store.append_event(&conv_id, kind, payload, 50_000);
                imported_count += 1;
            }
            let sess_meta_key = crate::web_api::session_meta_key(&conv_id);
            let mut smeta = store
                .get_kv(&sess_meta_key)
                .ok()
                .flatten()
                .unwrap_or_else(|| json!({"isPinned": false, "revision": 1, "configurationRevision": 1}));
            let next_rev = smeta
                .get("revision")
                .and_then(|v| v.as_i64())
                .unwrap_or(1)
                + if imported_count > 0 { 1 } else { 0 };
            smeta["revision"] = json!(next_rev);
            let _ = store.set_kv(&sess_meta_key, &smeta);
            let total_count = existing_count + imported_count;
            let conv_json = crate::web_api::session_to_conversation_json(store, &sess);
            Ok(json!({
                "ok": true,
                "conversation_id": conv_id,
                "appended_message_count": imported_count,
                "imported_count": imported_count,
                "message_count": total_count,
                "conversation_revision": next_rev,
                "conversation": conv_json
            }))
        }
        "admit_host_output" => {
            const EMPTY_SHA256: &str =
                "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
            const PRODUCT_DEFAULT_OUTPUT_TOKENS: u64 = 16_384;
            let input = args
                .get("arguments")
                .or_else(|| args.get("input"))
                .unwrap_or(args);
            let routes = input
                .get("routes")
                .and_then(|v| v.as_array())
                .ok_or(("invalid_argument", "missing 'routes' array".into()))?;
            if routes.is_empty() {
                return Err(("invalid_argument", "routes array must not be empty".into()));
            }
            let mut min_known_cap: Option<u64> = None;
            for entry in routes {
                let route = entry
                    .get("route")
                    .and_then(|v| v.as_object())
                    .ok_or(("invalid_argument", "route entry missing 'route' object".into()))?;
                if let Some(max_out_val) = entry.get("max_output_tokens").filter(|v| !v.is_null()) {
                    let cap = max_out_val.as_u64().ok_or((
                        "invalid_argument",
                        "max_output_tokens must be a positive integer".into(),
                    ))?;
                    if cap == 0 {
                        return Err((
                            "invalid_argument",
                            "max_output_tokens must be greater than zero".into(),
                        ));
                    }
                    min_known_cap = Some(match min_known_cap {
                        Some(cur) => cur.min(cap),
                        None => cap,
                    });
                }
                let kind = route
                    .get("kind")
                    .and_then(|v| v.as_str())
                    .ok_or(("invalid_argument", "route missing 'kind'".into()))?;
                match kind {
                    "http" => {
                        let proto = route
                            .get("protocol")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        if !matches!(
                            proto,
                            "chat_completions"
                                | "responses"
                                | "anthropic"
                                | "gemini"
                                | "siwc_responses"
                                | "openai_chat"
                                | "openai_responses"
                        ) {
                            return Err((
                                "invalid_argument",
                                format!("unsupported http route protocol '{proto}'"),
                            ));
                        }
                        if let Some(cs) = route.get("credential_source").and_then(|v| v.as_object())
                        {
                            let cs_kind = cs.get("kind").and_then(|v| v.as_str()).unwrap_or("");
                            match cs_kind {
                                "none" => {
                                    if cs.len() > 1 {
                                        return Err((
                                            "invalid_argument",
                                            "credential_source kind 'none' cannot include extra fields"
                                                .into(),
                                        ));
                                    }
                                }
                                "ephemeral" => {
                                    let digest = route
                                        .get("credential_sha256")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("");
                                    if digest.is_empty()
                                        || digest.eq_ignore_ascii_case(EMPTY_SHA256)
                                    {
                                        return Err((
                                            "invalid_argument",
                                            "empty credential_sha256 cannot use ephemeral credential_source"
                                                .into(),
                                        ));
                                    }
                                }
                                "keychain" | "vault" | "account" | "profile" | "siwc" | "env" => {}
                                other => {
                                    return Err((
                                        "invalid_argument",
                                        format!("unsupported credential_source kind '{other}'"),
                                    ));
                                }
                            }
                        }
                    }
                    "native" => {
                        let adapter = route.get("adapter").and_then(|v| v.as_str()).unwrap_or("");
                        if adapter.trim().is_empty() {
                            return Err((
                                "invalid_argument",
                                "native route missing 'adapter'".into(),
                            ));
                        }
                    }
                    other => {
                        return Err((
                            "invalid_argument",
                            format!("unsupported route kind '{other}'"),
                        ));
                    }
                }
            }
            let preference = input
                .get("preference")
                .ok_or(("invalid_argument", "missing 'preference'".into()))?;
            let resolved = if preference.as_str() == Some("default") {
                if let Some(cap) = min_known_cap {
                    json!({
                        "source": "known_capability_default",
                        "tokens": cap
                    })
                } else {
                    json!({
                        "source": "product_default",
                        "tokens": PRODUCT_DEFAULT_OUTPUT_TOKENS
                    })
                }
            } else if let Some(explicit) = preference.get("explicit").and_then(|v| v.as_u64()) {
                if explicit == 0 {
                    return Err((
                        "invalid_argument",
                        "explicit output preference must be greater than zero".into(),
                    ));
                }
                if let Some(cap) = min_known_cap {
                    if explicit > cap {
                        return Err((
                            "invalid_argument",
                            format!(
                                "explicit output preference ({explicit}) exceeds route max_output_tokens ({cap})"
                            ),
                        ));
                    }
                }
                json!({
                    "source": "explicit_preference",
                    "tokens": explicit
                })
            } else {
                return Err((
                    "invalid_argument",
                    "unsupported host output preference".into(),
                ));
            };
            Ok(json!({
                "version": 1,
                "routes": routes,
                "resolved": resolved
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
            let cur_rev = store
                .get_kv("archive_directory_revision")
                .ok()
                .flatten()
                .and_then(|v| v.as_u64())
                .unwrap_or(1);
            if let Some(exp_rev) = input.get("expected_directory_revision").and_then(|v| v.as_u64())
            {
                if exp_rev != cur_rev {
                    return Err((
                        "revision_conflict",
                        format!(
                            "archive directory revision conflict: expected {exp_rev}, actual {cur_rev}"
                        ),
                    ));
                }
            }
            let ts = input
                .get("created_at_ms")
                .and_then(|v| v.as_i64())
                .unwrap_or_else(now_ms);
            let initial_digest = sha256_hex(b"");
            let rec = json!({
                "restore_id": restore_id,
                "archive_id": input.get("archive_id").cloned().unwrap_or(Value::Null),
                "status": "in_progress",
                "started_at_ms": ts,
                "next_sequence": 0u64,
                "manifest_digest": initial_digest,
                "completed": false,
                "entries": []
            });
            store
                .set_kv(&format!("archive_restore:{restore_id}"), &rec)
                .map_err(|e| ("storage_error", e.to_string()))?;
            let _ = store.set_kv("archive_restore_active", &json!(restore_id));
            Ok(json!({
                "ok": true,
                "restore_id": restore_id,
                "next_sequence": 0u64,
                "manifest_digest": initial_digest,
                "completed": false,
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
            if let Some(exp_rev) = input.get("expected_directory_revision").and_then(|v| v.as_u64())
            {
                if exp_rev != cur_rev {
                    return Err((
                        "revision_conflict",
                        format!(
                            "archive directory revision conflict: expected {exp_rev}, actual {cur_rev}"
                        ),
                    ));
                }
            }
            let existing = store
                .get_kv(&format!("archive_restore:{restore_id}"))
                .ok()
                .flatten()
                .unwrap_or_else(|| json!({}));
            let entries = existing
                .get("entries")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            if let Some(exp_count) = input.get("expected_entry_count").and_then(|v| v.as_u64()) {
                if exp_count != entries.len() as u64 {
                    return Err((
                        "invalid_argument",
                        format!(
                            "archive entry count mismatch: expected {exp_count}, actual {}",
                            entries.len()
                        ),
                    ));
                }
            }
            if let Some(exp_digest) = input.get("manifest_digest").and_then(|v| v.as_str()) {
                if let Some(cur_digest) = existing.get("manifest_digest").and_then(|v| v.as_str()) {
                    if exp_digest != cur_digest {
                        return Err((
                            "invalid_argument",
                            "archive manifest digest mismatch".into(),
                        ));
                    }
                }
            }
            let mut conversation_count = 0u64;
            let mut message_count = 0u64;
            for entry in &entries {
                match entry.get("kind").and_then(|v| v.as_str()) {
                    Some("profile") => {
                        if let Some(prof) = entry.get("profile") {
                            let conv_id = prof
                                .get("id")
                                .or_else(|| prof.get("conversation_id"))
                                .and_then(|v| v.as_str())
                                .filter(|s| !s.trim().is_empty())
                                .map(ToString::to_string)
                                .unwrap_or_else(new_id);
                            let created_at_ms = prof
                                .get("created_at_ms")
                                .and_then(|v| v.as_i64())
                                .unwrap_or_else(now_ms);
                            let updated_at_ms = prof
                                .get("updated_at_ms")
                                .and_then(|v| v.as_i64())
                                .unwrap_or(created_at_ms);
                            let sess = Session {
                                id: conv_id.clone(),
                                title: prof
                                    .get("title")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("Restored conversation")
                                    .to_string(),
                                workspace_root: suite.workspace_root.display().to_string(),
                                model_connection: None,
                                model_id: None,
                                created_at_ms,
                                updated_at_ms,
                            };
                            store
                                .upsert_session(&sess)
                                .map_err(|e| ("storage_error", e.to_string()))?;
                            if let Some(settings) =
                                prof.get("settings_json").filter(|v| !v.is_null())
                            {
                                let s = settings
                                    .as_str()
                                    .map(ToString::to_string)
                                    .unwrap_or_else(|| settings.to_string());
                                let _ = store.set_kv(&format!("conv_settings:{conv_id}"), &json!(s));
                            }
                            conversation_count += 1;
                        }
                    }
                    Some("message") => {
                        message_count += 1;
                    }
                    _ => {}
                }
            }
            let next_rev = cur_rev + 1;
            let ts = input
                .get("committed_at_ms")
                .and_then(|v| v.as_i64())
                .unwrap_or_else(now_ms);
            let rec = json!({
                "restore_id": restore_id,
                "status": "committed",
                "revision": next_rev,
                "directory_revision": next_rev,
                "conversation_count": conversation_count,
                "message_count": message_count,
                "completed": true,
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
                "conversation_count": conversation_count,
                "message_count": message_count,
                "directory_revision": next_rev,
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
            Ok(json!(rev))
        }
        "stage_archive_manifest" => {
            let input = args.get("input").unwrap_or(args);
            if let Some(restore_id) = input
                .get("restore_id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
            {
                let mut rec = store
                    .get_kv(&format!("archive_restore:{restore_id}"))
                    .map_err(|e| ("storage_error", e.to_string()))?
                    .ok_or((
                        "not_found",
                        format!("archive restore '{restore_id}' not found"),
                    ))?;
                let cur_seq = rec
                    .get("next_sequence")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                if let Some(seq) = input.get("sequence").and_then(|v| v.as_u64()) {
                    if seq != cur_seq {
                        return Err((
                            "invalid_argument",
                            format!("expected manifest sequence {cur_seq}, got {seq}"),
                        ));
                    }
                }
                let entry = input
                    .get("entry")
                    .cloned()
                    .unwrap_or_else(|| input.clone());
                let mut entries = rec
                    .get("entries")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default();
                entries.push(entry);
                let next_seq = cur_seq + 1;
                let prev_digest = rec
                    .get("manifest_digest")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let last_entry = entries.last().cloned().unwrap_or(Value::Null);
                let digest_input = format!(
                    "{prev_digest}:{cur_seq}:{}",
                    canonical_json_value(&last_entry)
                );
                let next_digest = sha256_hex(digest_input.as_bytes());
                rec["entries"] = Value::Array(entries);
                rec["next_sequence"] = json!(next_seq);
                rec["manifest_digest"] = json!(next_digest);
                rec["completed"] = json!(false);
                store
                    .set_kv(&format!("archive_restore:{restore_id}"), &rec)
                    .map_err(|e| ("storage_error", e.to_string()))?;
                return Ok(json!({
                    "ok": true,
                    "staged": true,
                    "restore_id": restore_id,
                    "next_sequence": next_seq,
                    "manifest_digest": next_digest,
                    "completed": false
                }));
            }
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
            let owner = input
                .get("owner")
                .cloned()
                .unwrap_or_else(|| {
                    json!({
                        "conversation_id": input.get("conversation_id").and_then(|v| v.as_str()).unwrap_or(""),
                        "import_id": input.get("import_id").and_then(|v| v.as_str()).unwrap_or(""),
                        "part_id": input.get("part_id").and_then(|v| v.as_str()).unwrap_or("")
                    })
                });
            let expected_bytes = input
                .get("expected_utf8_bytes")
                .or_else(|| input.get("total_bytes"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            if let Ok(Some(existing)) = store.get_kv(&format!("content_upload:{upload_id}")) {
                let ex_owner = existing.get("owner").cloned().unwrap_or(Value::Null);
                let ex_expected = existing
                    .get("expected_utf8_bytes")
                    .or_else(|| existing.get("total_bytes"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                if ex_owner == owner && ex_expected == expected_bytes {
                    let state = existing
                        .get("state")
                        .and_then(|v| v.as_str())
                        .unwrap_or("receiving");
                    let committed = existing
                        .get("committed_utf8_bytes")
                        .or_else(|| existing.get("bytes_received"))
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    return Ok(json!({
                        "upload_id": upload_id,
                        "owner": owner,
                        "state": state,
                        "committed_utf8_bytes": committed,
                        "expected_utf8_bytes": expected_bytes
                    }));
                }
                return Err((
                    "conflict",
                    format!("content upload '{upload_id}' already exists with different owner or byte count"),
                ));
            }
            let ts = now_ms();
            let rec = json!({
                "upload_id": upload_id,
                "owner": owner,
                "state": "receiving",
                "expected_utf8_bytes": expected_bytes,
                "committed_utf8_bytes": 0u64,
                "content": "",
                "sealed": false,
                "created_at_ms": ts,
                "updated_at_ms": ts
            });
            store
                .set_kv(&format!("content_upload:{upload_id}"), &rec)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({
                "upload_id": upload_id,
                "owner": owner,
                "state": "receiving",
                "committed_utf8_bytes": 0u64,
                "expected_utf8_bytes": expected_bytes
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
            if let Some(req_owner) = input.get("owner") {
                if upload.get("owner") != Some(req_owner) {
                    return Err(("conflict", format!("content upload '{upload_id}' owner mismatch")));
                }
            }
            let state = upload
                .get("state")
                .and_then(|v| v.as_str())
                .unwrap_or("receiving")
                .to_string();
            if state == "sealed" || upload.get("sealed").and_then(|v| v.as_bool()) == Some(true) {
                return Err(("invalid_state", format!("content upload '{upload_id}' is already sealed")));
            }
            let expected_bytes = upload
                .get("expected_utf8_bytes")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let mut content = upload
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let text = input
                .get("text")
                .or_else(|| input.get("data"))
                .or_else(|| input.get("chunk"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let offset_bytes = input
                .get("offset_bytes")
                .and_then(|v| v.as_u64())
                .unwrap_or(content.len() as u64) as usize;
            let committed_now = content.len();
            if offset_bytes < committed_now {
                // Replay check: verify exact prefix bytes match
                let end = offset_bytes.saturating_add(text.len());
                if end <= committed_now {
                    if &content.as_bytes()[offset_bytes..end] != text.as_bytes() {
                        return Err((
                            "conflict",
                            format!("replayed chunk at offset {offset_bytes} differs from committed bytes"),
                        ));
                    }
                } else {
                    let overlap = committed_now - offset_bytes;
                    if &content.as_bytes()[offset_bytes..committed_now] != &text.as_bytes()[..overlap] {
                        return Err((
                            "conflict",
                            format!("overlapping chunk at offset {offset_bytes} differs from committed bytes"),
                        ));
                    }
                    content.push_str(&text[overlap..]);
                }
            } else if offset_bytes == committed_now {
                content.push_str(text);
            } else {
                return Err((
                    "invalid_argument",
                    format!("chunk offset {offset_bytes} skips uncommitted bytes (current {committed_now})"),
                ));
            }
            let new_committed = content.len() as u64;
            if expected_bytes > 0 && new_committed > expected_bytes {
                return Err((
                    "invalid_argument",
                    format!("upload '{upload_id}' exceeds expected_utf8_bytes ({new_committed} > {expected_bytes})"),
                ));
            }
            if let Some(obj) = upload.as_object_mut() {
                obj.insert("content".into(), json!(content));
                obj.insert("committed_utf8_bytes".into(), json!(new_committed));
                obj.insert("bytes_received".into(), json!(new_committed));
                obj.insert("updated_at_ms".into(), json!(now_ms()));
            }
            store
                .set_kv(&format!("content_upload:{upload_id}"), &upload)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({
                "upload_id": upload_id,
                "owner": upload.get("owner").cloned().unwrap_or(Value::Null),
                "state": "receiving",
                "committed_utf8_bytes": new_committed,
                "expected_utf8_bytes": expected_bytes
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
            if let Some(req_owner) = input.get("owner") {
                if upload.get("owner") != Some(req_owner) {
                    return Err(("conflict", format!("content upload '{upload_id}' owner mismatch")));
                }
            }
            let expected_bytes = upload
                .get("expected_utf8_bytes")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let committed_bytes = upload
                .get("committed_utf8_bytes")
                .or_else(|| upload.get("bytes_received"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            if expected_bytes > 0 && committed_bytes != expected_bytes {
                return Err((
                    "invalid_state",
                    format!("cannot seal incomplete upload '{upload_id}' ({committed_bytes}/{expected_bytes})"),
                ));
            }
            let ts = now_ms();
            if let Some(obj) = upload.as_object_mut() {
                obj.insert("state".into(), json!("sealed"));
                obj.insert("sealed".into(), json!(true));
                obj.insert("sealed_at_ms".into(), json!(ts));
                obj.insert("updated_at_ms".into(), json!(ts));
            }
            store
                .set_kv(&format!("content_upload:{upload_id}"), &upload)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({
                "upload_id": upload_id,
                "owner": upload.get("owner").cloned().unwrap_or(Value::Null),
                "state": "sealed",
                "committed_utf8_bytes": committed_bytes,
                "expected_utf8_bytes": expected_bytes
            }))
        }
        "content_upload_progress" => {
            let input = args.get("input").unwrap_or(args);
            let upload_id = input
                .get("upload_id")
                .or_else(|| input.get("id"))
                .or_else(|| args.get("upload_id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'upload_id'".into()))?;
            let u = store
                .get_kv(&format!("content_upload:{upload_id}"))
                .map_err(|e| ("storage_error", e.to_string()))?
                .ok_or(("not_found", format!("content upload '{upload_id}' not found")))?;
            if let Some(req_owner) = input.get("owner") {
                if u.get("owner") != Some(req_owner) {
                    return Err(("conflict", format!("content upload '{upload_id}' owner mismatch")));
                }
            }
            Ok(json!({
                "upload_id": upload_id,
                "owner": u.get("owner").cloned().unwrap_or(Value::Null),
                "state": u.get("state").and_then(|v| v.as_str()).unwrap_or("receiving"),
                "committed_utf8_bytes": u.get("committed_utf8_bytes").or_else(|| u.get("bytes_received")).and_then(|v| v.as_u64()).unwrap_or(0),
                "expected_utf8_bytes": u.get("expected_utf8_bytes").or_else(|| u.get("total_bytes")).and_then(|v| v.as_u64()).unwrap_or(0)
            }))
        }
        "read_content_upload_chunk" => {
            let input = args.get("input").unwrap_or(args);
            let upload_id = input
                .get("upload_id")
                .or_else(|| input.get("id"))
                .or_else(|| args.get("upload_id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'upload_id'".into()))?;
            let u = store
                .get_kv(&format!("content_upload:{upload_id}"))
                .map_err(|e| ("storage_error", e.to_string()))?
                .ok_or(("not_found", format!("content upload '{upload_id}' not found")))?;
            if let Some(req_owner) = input.get("owner") {
                if u.get("owner") != Some(req_owner) {
                    return Err(("conflict", format!("content upload '{upload_id}' owner mismatch")));
                }
            }
            let content = u.get("content").and_then(|v| v.as_str()).unwrap_or("");
            let total = content.len();
            let offset = input
                .get("offset_bytes")
                .or_else(|| args.get("offset_bytes"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize;
            if offset > total || !content.is_char_boundary(offset) {
                return Err((
                    "invalid_argument",
                    format!("invalid UTF-8 offset_bytes {offset} for upload '{upload_id}' (len {total})"),
                ));
            }
            let max_chunk = 32 * 1024;
            let mut end = (offset + max_chunk).min(total);
            while end > offset && !content.is_char_boundary(end) {
                end -= 1;
            }
            let slice = &content[offset..end];
            Ok(json!({
                "upload_id": upload_id,
                "owner": u.get("owner").cloned().unwrap_or(Value::Null),
                "offset_bytes": offset as u64,
                "next_offset_bytes": end as u64,
                "text": slice,
                "end_of_content": end == total
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
                "checkpoint_revision": 1i64,
                "checkpoint_event_cursor": 0i64,
                "checkpoint_sha256": sha256_hex(format!("checkpoint:{conv_id}:{run_id}").as_bytes()),
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
        "validate_mobile_run_recovery_authority" | "validate_run_recovery_authority" => {
            let input = args.get("input").unwrap_or(args);
            let conv_id = input
                .get("conversation_id")
                .or_else(|| args.get("conversation_id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .ok_or(("invalid_argument", "missing 'conversation_id'".into()))?;
            let expected_run_id = input
                .get("expected_run_id")
                .or_else(|| input.get("run_id"))
                .or_else(|| args.get("expected_run_id"))
                .or_else(|| args.get("run_id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .ok_or(("invalid_argument", "missing 'expected_run_id'".into()))?;
            let expected_assistant = input
                .get("assistant_message_id")
                .or_else(|| args.get("assistant_message_id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .ok_or(("invalid_argument", "missing 'assistant_message_id'".into()))?;
            let expected_digest = input
                .get("expected_invocation_config_digest")
                .or_else(|| input.get("invocation_config_digest"))
                .or_else(|| args.get("expected_invocation_config_digest"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .ok_or(("invalid_argument", "missing 'expected_invocation_config_digest'".into()))?;
            let inv = store
                .get_kv(&format!("exact_run_invocation:{conv_id}:{expected_run_id}"))
                .map_err(|e| ("storage_error", e.to_string()))?
                .ok_or(("not_found", format!("exact run invocation '{expected_run_id}' not found")))?;
            if inv.get("assistant_message_id").and_then(|v| v.as_str()) != Some(expected_assistant)
                || inv.get("invocation_config_digest").and_then(|v| v.as_str()) != Some(expected_digest)
            {
                return Err((
                    "conflict",
                    "recovery authority does not match the admitted Run invocation".into(),
                ));
            }
            Ok(json!({
                "conversation_id": conv_id,
                "run_id": expected_run_id,
                "assistant_message_id": expected_assistant,
                "invocation_config_digest": expected_digest
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
            let assistant_filter = identity
                .and_then(|i| i.get("assistant_message_id"))
                .or_else(|| args.get("assistant_message_id"))
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
                    let asst_ok = assistant_filter.is_none_or(|a| {
                        p.pointer("/identity/assistant_message_id").and_then(|v| v.as_str()) == Some(a)
                    });
                    conv_ok && run_ok && asst_ok
                })
                .collect();
            let section_rank = |p: &Value| -> usize {
                match p.pointer("/order_key/section").and_then(|v| v.as_str()).unwrap_or("") {
                    "transcript" => 0,
                    "tasks" => 1,
                    "skills" => 2,
                    "model_attempts" => 3,
                    _ => 4,
                }
            };
            let pos_val = |p: &Value| -> u64 {
                p.pointer("/order_key/position")
                    .and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|s| s.parse().ok())))
                    .unwrap_or(0)
            };
            parts.sort_by(|a, b| {
                section_rank(a)
                    .cmp(&section_rank(b))
                    .then_with(|| pos_val(a).cmp(&pos_val(b)))
                    .then_with(|| {
                        a.get("part_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .cmp(b.get("part_id").and_then(|v| v.as_str()).unwrap_or(""))
                    })
            });
            let filtered: Vec<Value> = if let Some(after) = after_part_id {
                let start_idx = parts
                    .iter()
                    .position(|p| p.get("part_id").and_then(|v| v.as_str()) == Some(after))
                    .map(|idx| idx + 1)
                    .unwrap_or(0);
                parts.into_iter().skip(start_idx).collect()
            } else {
                parts
            };
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
                .pointer("/content/field")
                .or_else(|| args.pointer("/content/content_id"))
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

            let content_root = args
                .pointer("/content/content_root")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty());
            let content_key = format!("assistant_journal_content:{conv_id}:{run_id}:{part_id}:{content_id}");
            let full_text = content_root
                .and_then(|root| {
                    let root_key = format!(
                        "assistant_journal_content_by_root:{conv_id}:{run_id}:{part_id}:{content_id}:{root}"
                    );
                    store
                        .get_kv(&root_key)
                        .ok()
                        .flatten()
                        .and_then(|v| v.get("text").and_then(|s| s.as_str()).map(ToString::to_string))
                })
                .unwrap_or_else(|| {
                    store
                        .get_kv(&content_key)
                        .ok()
                        .flatten()
                        .and_then(|v| v.get("text").and_then(|s| s.as_str()).map(ToString::to_string))
                        .unwrap_or_default()
                });
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
        "capture_mobile_delegation_configuration" => {
            let mut config = args
                .get("config")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let settings = args
                .get("settings")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let session_id = config
                .get("session_id")
                .or_else(|| args.get("session_id"))
                .or_else(|| args.get("conversation_id"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or("default")
                .to_string();
            if let Some(cfg_obj) = config.as_object_mut() {
                cfg_obj
                    .entry("session_id".to_string())
                    .or_insert_with(|| json!(session_id));
                let identity = cfg_obj
                    .entry("turn_identity".to_string())
                    .or_insert_with(|| json!({}));
                if !identity.is_object() {
                    *identity = json!({});
                }
                if let Some(id_obj) = identity.as_object_mut() {
                    id_obj.insert("delegation".into(), settings.clone());
                }
            }
            store
                .set_kv(&format!("mobile_delegation_cfg:{session_id}"), &config)
                .map_err(|e| ("storage_error", e.to_string()))?;
            let _ = store.set_kv("mobile_delegation_cfg:latest", &config);
            Ok(config)
        }
        "prepare_mobile_delegated_task" => {
            let input = args.get("input").unwrap_or(args);
            let req = input.get("request").unwrap_or(input);
            let payload = req.get("payload").unwrap_or(req);
            let tool_input = payload
                .get("input")
                .or_else(|| payload.get("arguments"))
                .unwrap_or(payload);
            let now = input
                .get("now_ms")
                .and_then(|v| v.as_i64())
                .unwrap_or_else(now_ms);
            let parent_session_id = req
                .get("conversation_id")
                .or_else(|| req.get("session_id"))
                .or_else(|| input.get("parent_conversation_id"))
                .or_else(|| input.get("session_id"))
                .or_else(|| args.get("conversation_id"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or("default")
                .to_string();
            let call_id = payload
                .get("call_id")
                .or_else(|| req.get("effect_id"))
                .or_else(|| input.get("call_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            // Idempotent replay when the same parent_session_id + non-empty call_id already prepared a task.
            let existing = if !call_id.is_empty() {
                suite
                    .list_records::<Value>("mobile_delegated_task")
                    .ok()
                    .and_then(|list| {
                        list.into_iter().find(|r| {
                            r.pointer("/task/owner_session_id").and_then(|v| v.as_str())
                                == Some(parent_session_id.as_str())
                                && r.pointer("/task/call_id").and_then(|v| v.as_str())
                                    == Some(call_id.as_str())
                        })
                    })
            } else {
                None
            };

            let (task_id, task_obj, replayed) = if let Some(prev) = existing {
                let tid = prev
                    .pointer("/task/id")
                    .or_else(|| prev.get("id"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let task = prev.get("task").cloned().unwrap_or(prev);
                (tid, task, true)
            } else {
                let tid = tool_input
                    .get("task_id")
                    .or_else(|| input.get("task_id"))
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(ToString::to_string)
                    .unwrap_or_else(new_id);
                let child_session_id = tool_input
                    .get("child_session_id")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(ToString::to_string)
                    .unwrap_or_else(|| format!("child-{tid}"));
                let child_run_id = format!("run-{tid}");
                let assistant_message_id = format!("{tid}:assistant");
                let prompt = tool_input
                    .get("prompt")
                    .or_else(|| tool_input.get("task"))
                    .or_else(|| tool_input.get("instruction"))
                    .or_else(|| tool_input.get("message"))
                    .or_else(|| tool_input.get("description"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let title = tool_input
                    .get("title")
                    .or_else(|| tool_input.get("name"))
                    .or_else(|| tool_input.get("agent"))
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or("Delegated Task")
                    .to_string();

                let mut child_config = store
                    .get_kv(&format!("mobile_delegation_cfg:{parent_session_id}"))
                    .ok()
                    .flatten()
                    .filter(|v| v.is_object())
                    .or_else(|| {
                        store
                            .get_kv("mobile_delegation_cfg:latest")
                            .ok()
                            .flatten()
                            .filter(|v| v.is_object())
                    })
                    .unwrap_or_else(|| {
                        json!({
                            "tools": [],
                            "host_catalog": {"host_id": "mobile-device"},
                            "context_window": {
                                "context_window_tokens": 128_000,
                                "reserved_output_tokens": 4_096
                            },
                            "turn_identity": {}
                        })
                    });
                if let Some(cfg_obj) = child_config.as_object_mut() {
                    cfg_obj.insert("session_id".into(), json!(child_session_id));
                }

                let _ = store.upsert_session(&Session {
                    id: child_session_id.clone(),
                    title: title.clone(),
                    workspace_root: suite.workspace_root.display().to_string(),
                    model_connection: None,
                    model_id: None,
                    created_at_ms: now,
                    updated_at_ms: now,
                });

                let task = json!({
                    "id": tid,
                    "task_id": tid,
                    "kind": "agent",
                    "status": "queued",
                    "title": title,
                    "prompt": prompt,
                    "owner_session_id": parent_session_id,
                    "parent_conversation_id": parent_session_id,
                    "conversation_id": parent_session_id,
                    "child_session_id": child_session_id,
                    "child_conversation_id": child_session_id,
                    "run_id": child_run_id,
                    "assistant_message_id": assistant_message_id,
                    "call_id": call_id,
                    "attempt": 0,
                    "guidance": [],
                    "created_at_ms": now,
                    "updated_at_ms": now
                });
                let stored = json!({
                    "id": tid,
                    "task_id": tid,
                    "task": task,
                    "child_config": child_config,
                    "updated_at_ms": now
                });
                suite
                    .upsert_record("mobile_delegated_task", &tid, &stored)
                    .map_err(|e| ("storage_error", e.to_string()))?;
                let _ = suite.upsert_record("runtime_task", &tid, &task);
                let _ = store.set_kv(&format!("mobile_task_child_cfg:{tid}"), &child_config);
                (tid, task, false)
            };

            let child_sid = task_obj
                .get("child_session_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let status = task_obj
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("queued");
            Ok(json!({
                "ok": true,
                "prepared": {
                    "task": task_obj,
                    "replayed": replayed,
                    "source_admitted": false
                },
                "response": {
                    "type": "task_promoted",
                    "call_id": call_id,
                    "task_id": task_id,
                    "child_session_id": child_sid,
                    "status": status,
                    "output": {
                        "task_id": task_id,
                        "child_session_id": child_sid,
                        "status": status,
                        "replayed": replayed
                    }
                }
            }))
        }
        "prepare_mobile_task_guidance" => {
            let input = args.get("input").unwrap_or(args);
            let req = input.get("request").unwrap_or(input);
            let payload = req.get("payload").unwrap_or(req);
            let tool_input = payload
                .get("input")
                .or_else(|| payload.get("arguments"))
                .unwrap_or(payload);
            let now = input
                .get("now_ms")
                .and_then(|v| v.as_i64())
                .unwrap_or_else(now_ms);
            let task_id = tool_input
                .get("task_id")
                .or_else(|| tool_input.get("id"))
                .or_else(|| input.get("task_id"))
                .or_else(|| args.get("task_id"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or(("invalid_argument", "missing 'task_id'".into()))?;
            let call_id = payload
                .get("call_id")
                .or_else(|| req.get("effect_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let guidance = tool_input
                .get("message")
                .or_else(|| tool_input.get("guidance"))
                .or_else(|| tool_input.get("content"))
                .or_else(|| tool_input.get("text"))
                .cloned()
                .unwrap_or_else(|| json!(""));
            let receipt_id = new_id();
            let entry = json!({
                "receipt_id": receipt_id,
                "task_id": task_id,
                "guidance": guidance,
                "status": "accepted",
                "created_at_ms": now
            });
            if let Ok(Some(mut rec)) = suite.get_record::<Value>("mobile_delegated_task", task_id) {
                if let Some(task_obj) = rec.get_mut("task").and_then(|v| v.as_object_mut()) {
                    let list = task_obj
                        .entry("guidance".to_string())
                        .or_insert_with(|| json!([]));
                    if let Some(arr) = list.as_array_mut() {
                        arr.push(entry.clone());
                    }
                    task_obj.insert("updated_at_ms".into(), json!(now));
                }
                if let Some(obj) = rec.as_object_mut() {
                    obj.insert("updated_at_ms".into(), json!(now));
                }
                let _ = suite.upsert_record("mobile_delegated_task", task_id, &rec);
                if let Some(task_val) = rec.get("task") {
                    let _ = suite.upsert_record("runtime_task", task_id, task_val);
                }
            }
            let _ = store.set_kv(&format!("task_guidance_receipt:{receipt_id}"), &entry);
            let _ = store.set_kv(&format!("task_guidance_receipt:{task_id}"), &entry);
            Ok(json!({
                "ok": true,
                "type": "tool_completed",
                "call_id": call_id,
                "outcome": "succeeded",
                "task_id": task_id,
                "receipt_id": receipt_id,
                "accepted": true,
                "output": {
                    "execution": {"state": "completed"},
                    "result": entry
                },
                "full_output_ref": Value::Null,
                "artifacts": []
            }))
        }
        "observe_mobile_task" => {
            let input = args.get("input").unwrap_or(args);
            let req = input.get("request").unwrap_or(input);
            let payload = req.get("payload").unwrap_or(req);
            let tool_input = payload
                .get("input")
                .or_else(|| payload.get("arguments"))
                .unwrap_or(payload);
            let task_id = tool_input
                .get("task_id")
                .or_else(|| tool_input.get("id"))
                .or_else(|| input.get("task_id"))
                .or_else(|| args.get("task_id"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or(("invalid_argument", "missing 'task_id'".into()))?;
            let call_id = payload
                .get("call_id")
                .or_else(|| req.get("effect_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let capability = payload
                .get("capability_id")
                .or_else(|| payload.get("tool"))
                .and_then(|v| v.as_str())
                .unwrap_or("read_task");

            let stored = suite
                .get_record::<Value>("mobile_delegated_task", task_id)
                .map_err(|e| ("storage_error", e.to_string()))?;
            let task_val = stored
                .as_ref()
                .and_then(|r| r.get("task").cloned())
                .or_else(|| {
                    suite
                        .get_record::<Value>("runtime_task", task_id)
                        .ok()
                        .flatten()
                });
            let status = task_val
                .as_ref()
                .and_then(|t| t.get("status").and_then(|v| v.as_str()))
                .unwrap_or("not_found")
                .to_string();
            let is_terminal = matches!(
                status.as_str(),
                "completed" | "failed" | "cancelled" | "not_found"
            );
            let wait_ms: i64 = if capability == "wait_task" && !is_terminal {
                tool_input
                    .get("wait_ms")
                    .or_else(|| tool_input.get("timeout_ms"))
                    .and_then(|v| v.as_i64())
                    .unwrap_or(5_000)
                    .clamp(0, 300_000)
            } else {
                0
            };
            let latest_output = task_val
                .as_ref()
                .and_then(|t| t.get("result").or_else(|| t.get("output")).cloned())
                .filter(|v| !v.is_null())
                .or_else(|| {
                    let child_sid = task_val
                        .as_ref()
                        .and_then(|t| t.get("child_session_id").and_then(|v| v.as_str()))?;
                    let (msgs, _) = crate::web_api::reconstruct_v2_messages(store, child_sid);
                    msgs.into_iter()
                        .rev()
                        .find(|m| m.get("role").and_then(|v| v.as_str()) == Some("assistant"))
                        .and_then(|m| m.get("content").cloned())
                })
                .unwrap_or(Value::Null);
            let outcome = if matches!(status.as_str(), "failed" | "not_found") {
                "failed"
            } else {
                "succeeded"
            };
            let response = json!({
                "type": "tool_completed",
                "call_id": call_id,
                "outcome": outcome,
                "output": {
                    "execution": {"state": if is_terminal { "completed" } else { "running" }},
                    "task_id": task_id,
                    "status": status,
                    "terminal": is_terminal,
                    "task": task_val.clone().unwrap_or(Value::Null),
                    "result": latest_output
                },
                "full_output_ref": Value::Null,
                "artifacts": []
            });
            Ok(json!({
                "task_id": task_id,
                "status": status,
                "terminal": is_terminal,
                "wait_ms": wait_ms,
                "task": task_val.unwrap_or(Value::Null),
                "response": response
            }))
        }
        "load_mobile_task_child_configuration" => {
            let input = args.get("input").unwrap_or(args);
            let task_id = input
                .get("task_id")
                .or_else(|| input.get("id"))
                .or_else(|| args.get("task_id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or(("invalid_argument", "missing 'task_id'".into()))?;
            if let Ok(Some(cfg)) = store.get_kv(&format!("mobile_task_child_cfg:{task_id}")) {
                if cfg.is_object() {
                    return Ok(cfg);
                }
            }
            if let Ok(Some(rec)) = suite.get_record::<Value>("mobile_delegated_task", task_id) {
                if let Some(cfg) = rec.get("child_config").filter(|v| v.is_object()) {
                    return Ok(cfg.clone());
                }
                let owner_sid = rec
                    .pointer("/task/owner_session_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("default");
                let child_sid = rec
                    .pointer("/task/child_session_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or(task_id);
                let mut cfg = store
                    .get_kv(&format!("mobile_delegation_cfg:{owner_sid}"))
                    .ok()
                    .flatten()
                    .filter(|v| v.is_object())
                    .unwrap_or_else(|| {
                        json!({
                            "tools": [],
                            "host_catalog": {"host_id": "mobile-device"},
                            "context_window": {
                                "context_window_tokens": 128_000,
                                "reserved_output_tokens": 4_096
                            },
                            "turn_identity": {}
                        })
                    });
                if let Some(obj) = cfg.as_object_mut() {
                    obj.insert("session_id".into(), json!(child_sid));
                }
                return Ok(cfg);
            }
            Err((
                "not_found",
                format!("mobile delegated task configuration '{task_id}' not found"),
            ))
        }
        "admit_mobile_task_child" => {
            let input = args.get("input").unwrap_or(args);
            let task_id = input
                .get("task_id")
                .or_else(|| input.get("id"))
                .or_else(|| args.get("task_id"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or(("invalid_argument", "missing 'task_id'".into()))?;
            let now = input
                .get("now_ms")
                .and_then(|v| v.as_i64())
                .unwrap_or_else(now_ms);
            let lease_until_ms = input
                .get("lease_until_ms")
                .and_then(|v| v.as_i64())
                .unwrap_or(now + 60_000);
            let worker = input.get("worker").cloned().unwrap_or_else(|| json!({}));
            let candidate_cfg = input.get("candidate_config");

            let Some(mut rec) = suite
                .get_record::<Value>("mobile_delegated_task", task_id)
                .map_err(|e| ("storage_error", e.to_string()))?
            else {
                return Ok(Value::Null);
            };
            let current_status = rec
                .pointer("/task/status")
                .and_then(|v| v.as_str())
                .unwrap_or("queued")
                .to_string();
            if matches!(current_status.as_str(), "completed" | "failed") {
                return Ok(Value::Null);
            }
            // Reject overlapping admission while an existing lease by another worker is still live.
            let active_lease = rec
                .pointer("/lease/lease_until_ms")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let active_worker = rec.pointer("/lease/worker");
            if current_status == "running"
                && active_lease > now
                && active_worker.is_some_and(|w| w != &worker)
            {
                return Ok(Value::Null);
            }

            let cancellation_only = current_status == "cancelled";
            let attempt = rec
                .pointer("/task/attempt")
                .and_then(|v| v.as_u64())
                .unwrap_or(0)
                + 1;
            let parent_conv_id = input
                .get("parent_conversation_id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .or_else(|| {
                    rec.pointer("/task/owner_session_id")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string)
                })
                .unwrap_or_else(|| "default".into());
            let child_conv_id = rec
                .pointer("/task/child_session_id")
                .and_then(|v| v.as_str())
                .map(ToString::to_string)
                .unwrap_or_else(|| format!("child-{task_id}"));
            let core_session_id = candidate_cfg
                .and_then(|c| c.get("session_id").and_then(|v| v.as_str()))
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(|| child_conv_id.clone());
            let run_id = rec
                .pointer("/task/run_id")
                .and_then(|v| v.as_str())
                .map(ToString::to_string)
                .unwrap_or_else(|| format!("run-{task_id}"));
            let assistant_message_id = rec
                .pointer("/task/assistant_message_id")
                .and_then(|v| v.as_str())
                .map(ToString::to_string)
                .unwrap_or_else(|| format!("{task_id}:assistant"));
            let expert_target = rec
                .pointer("/task/expert_target")
                .cloned()
                .unwrap_or(Value::Null);
            let lease_info = json!({
                "attempt": attempt,
                "worker": worker,
                "lease_until_ms": lease_until_ms,
                "admitted_at_ms": now
            });
            if let Some(task_obj) = rec.get_mut("task").and_then(|v| v.as_object_mut()) {
                if !cancellation_only {
                    task_obj.insert("status".into(), json!("running"));
                }
                task_obj.insert("attempt".into(), json!(attempt));
                task_obj.insert("updated_at_ms".into(), json!(now));
            }
            if let Some(obj) = rec.as_object_mut() {
                obj.insert("lease".into(), lease_info.clone());
                obj.insert("updated_at_ms".into(), json!(now));
                if let Some(cfg) = candidate_cfg.filter(|v| v.is_object()) {
                    obj.insert("child_config".into(), cfg.clone());
                }
            }
            suite
                .upsert_record("mobile_delegated_task", task_id, &rec)
                .map_err(|e| ("storage_error", e.to_string()))?;
            if let Some(task_val) = rec.get("task") {
                let _ = suite.upsert_record("runtime_task", task_id, task_val);
            }
            Ok(json!({
                "task_id": task_id,
                "parent_conversation_id": parent_conv_id,
                "attempt": attempt,
                "current": lease_info,
                "cancellation_only": cancellation_only,
                "invocation": {
                    "conversation_id": child_conv_id,
                    "run_id": run_id,
                    "core_session_id": core_session_id,
                    "assistant_message_id": assistant_message_id
                },
                "expert_target": expert_target
            }))
        }
        other => Err((
            "unsupported_command",
            format!("unsupported daemon command '{other}'"),
        )),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ChatDefaultModelSelection {
    Automatic,
    Profile(String),
}

fn parse_chat_default_model_config(raw: &str) -> Result<ChatDefaultModelSelection, String> {
    let parsed: Value = serde_json::from_str(raw)
        .map_err(|e| format!("invalid chat.default_model JSON: {e}"))?;
    let Some(obj) = parsed.as_object() else {
        return Err("chat.default_model must be a JSON object".into());
    };
    if obj.len() != 2 || !obj.contains_key("version") || !obj.contains_key("selection") {
        return Err("chat.default_model must contain exact keys 'version' and 'selection'".into());
    }
    if obj.get("version").and_then(|v| v.as_i64()) != Some(1) {
        return Err("chat.default_model version must be 1".into());
    }
    let Some(sel) = obj.get("selection").and_then(|v| v.as_object()) else {
        return Err("chat.default_model selection must be a JSON object".into());
    };
    match sel.get("kind").and_then(|v| v.as_str()) {
        Some("automatic") if sel.len() == 1 => Ok(ChatDefaultModelSelection::Automatic),
        Some("profile") if sel.len() == 2 => {
            let pid = sel
                .get("profile_id")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| {
                    "chat.default_model profile selection requires non-empty 'profile_id'".to_string()
                })?;
            Ok(ChatDefaultModelSelection::Profile(pid.to_string()))
        }
        _ => Err("chat.default_model selection must be {kind:'automatic'} or {kind:'profile',profile_id}".into()),
    }
}

fn resolve_chat_model_from_candidates(args: &Value) -> Value {
    let candidates: Vec<(&str, bool, &str)> = args
        .get("candidates")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|c| {
                    let profile_id = c.get("profile_id").and_then(|v| v.as_str())?;
                    let allowed_for_chat = c
                        .get("allowed_for_chat")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(true);
                    let availability = c
                        .get("availability")
                        .and_then(|v| v.as_str())
                        .unwrap_or("available");
                    Some((profile_id, allowed_for_chat, availability))
                })
                .collect()
        })
        .unwrap_or_default();

    let evaluate_candidate = |profile_id: &str, origin: &str| -> Value {
        let Some(&(_, allowed_for_chat, availability)) =
            candidates.iter().find(|&&(id, _, _)| id == profile_id)
        else {
            return json!({
                "kind": "unavailable",
                "profile_id": profile_id,
                "origin": origin,
                "reason": "missing"
            });
        };
        if !allowed_for_chat {
            return json!({
                "kind": "unavailable",
                "profile_id": profile_id,
                "origin": origin,
                "reason": "not_allowed_for_chat"
            });
        }
        match availability {
            "available" => json!({
                "kind": "resolved",
                "profile_id": profile_id,
                "origin": origin
            }),
            "disabled" | "not_installed" | "unsupported_runtime" => json!({
                "kind": "unavailable",
                "profile_id": profile_id,
                "origin": origin,
                "reason": availability
            }),
            _ => json!({ "kind": "invalid_configuration" }),
        }
    };

    let intent_obj = args.get("intent").and_then(|v| v.as_object());
    let intent_kind = intent_obj
        .and_then(|o| o.get("kind"))
        .and_then(|v| v.as_str())
        .unwrap_or("inherit");

    match intent_kind {
        "explicit" => {
            let Some(profile_id) = intent_obj
                .and_then(|o| o.get("profile_id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
            else {
                return json!({ "kind": "invalid_configuration" });
            };
            evaluate_candidate(profile_id, "conversation")
        }
        "inherit" => {
            let default_val = args
                .get("default_value_json")
                .or_else(|| args.get("defaultValueJSON"));
            let selection = match default_val {
                None | Some(Value::Null) => ChatDefaultModelSelection::Automatic,
                Some(Value::String(raw)) => match parse_chat_default_model_config(raw) {
                    Ok(sel) => sel,
                    Err(_) => return json!({ "kind": "invalid_configuration" }),
                },
                Some(_) => return json!({ "kind": "invalid_configuration" }),
            };
            match selection {
                ChatDefaultModelSelection::Profile(profile_id) => {
                    evaluate_candidate(&profile_id, "default")
                }
                ChatDefaultModelSelection::Automatic => {
                    if let Some(&(profile_id, _, _)) = candidates
                        .iter()
                        .find(|&&(_, allowed, avail)| allowed && avail == "available")
                    {
                        json!({
                            "kind": "resolved",
                            "profile_id": profile_id,
                            "origin": "automatic"
                        })
                    } else {
                        json!({ "kind": "no_automatic_candidate" })
                    }
                }
            }
        }
        _ => json!({ "kind": "invalid_configuration" }),
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
    let existing = load_product_setting_entry(store, key)?;
    let raw_prev = store
        .get_kv(&format!("product_setting:{key}"))
        .map_err(|e| ("storage_error", e.to_string()))?;
    let stored_rev = match raw_prev {
        Some(ref v) if v.get("__nola_setting").and_then(|b| b.as_bool()) == Some(true) => {
            v.get("revision").and_then(|r| r.as_u64()).unwrap_or(0)
        }
        Some(_) => 1,
        None => 0,
    };
    let expected_rev = input
        .get("expected_revision")
        .or_else(|| args.get("expected_revision"))
        .and_then(|v| {
            v.as_u64()
                .or_else(|| v.as_i64().and_then(|i| u64::try_from(i).ok()))
        });
    if require_check {
        match (expected_rev, &existing) {
            (None, Some((_, _, rev, _))) => {
                return Err((
                    "revision_conflict",
                    format!("product_setting '{key}' already exists at revision {rev}"),
                ));
            }
            (None, None) => {}
            (Some(exp), Some((_, _, rev, _))) if *rev == exp => {}
            (Some(exp), Some((_, _, rev, _))) => {
                return Err((
                    "revision_conflict",
                    format!(
                        "product_setting '{key}' revision conflict: expected {exp}, actual {rev}"
                    ),
                ));
            }
            (Some(exp), None) => {
                return Err((
                    "revision_conflict",
                    format!(
                        "product_setting '{key}' does not exist (expected revision {exp})"
                    ),
                ));
            }
        }
    } else if let Some(exp) = expected_rev {
        let cur = existing.as_ref().map(|(_, _, r, _)| *r).unwrap_or(0);
        if cur != exp {
            return Err((
                "revision_conflict",
                format!("product_setting '{key}' revision conflict: expected {exp}, actual {cur}"),
            ));
        }
    }

    let (val, val_json_str) =
        if let Some(vj) = input.get("value_json").or_else(|| args.get("value_json")) {
            if let Some(s) = vj.as_str() {
                let parsed = serde_json::from_str::<Value>(s)
                    .unwrap_or_else(|_| Value::String(s.to_string()));
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

    if key == "chat.default_model" {
        parse_chat_default_model_config(&val_json_str)
            .map_err(|msg| ("invalid_argument", msg))?;
    }

    let next_rev = stored_rev + 1;
    let ts = input
        .get("updated_at_ms")
        .or_else(|| args.get("updated_at_ms"))
        .and_then(|v| v.as_i64())
        .unwrap_or_else(now_ms);
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

fn get_session_case_insensitive(
    store: &Store,
    id: &str,
) -> Result<Option<Session>, nola_store::StoreError> {
    if let Some(s) = store.get_session(id)? {
        return Ok(Some(s));
    }
    let lower = id.to_ascii_lowercase();
    if lower != id {
        if let Some(s) = store.get_session(&lower)? {
            return Ok(Some(s));
        }
    }
    let upper = id.to_ascii_uppercase();
    if upper != id && upper != lower {
        if let Some(s) = store.get_session(&upper)? {
            return Ok(Some(s));
        }
    }
    Ok(None)
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
        .or_else(|| input.get("source").and_then(|s| s.get("conversation_id")))
        .or_else(|| payload.get("conversation_id"))
        .or_else(|| payload.get("source").and_then(|s| s.get("conversation_id")))
        .or_else(|| args.get("conversation_id"))
        .and_then(|v| v.as_str())
        .ok_or(("invalid_argument", "missing conversation_id".into()))?;

    match op {
        "prepare_compaction" => {
            let (msgs, _) = crate::web_api::reconstruct_v2_messages(store, conv_id);
            let usage = load_session_usage(store, conv_id);
            let settings = load_conversation_settings_str(store, conv_id).unwrap_or_default();
            let src_fp = sha256_hex(
                json!({"conversation_id": conv_id, "messages": &msgs})
                    .to_string()
                    .as_bytes(),
            );
            let ctx_fp = sha256_hex(
                json!({"conversation_id": conv_id, "messages": &msgs, "settings": settings})
                    .to_string()
                    .as_bytes(),
            );
            Ok(json!({
                "conversation_id": conv_id,
                "needed": msgs.len() > 20,
                "message_count": msgs.len(),
                "source_fingerprint": src_fp,
                "context_fingerprint": ctx_fp,
                "usage": usage
            }))
        }
        "prepare_maintenance" => {
            let source = payload
                .get("source")
                .or_else(|| input.get("source"))
                .cloned()
                .unwrap_or_else(|| json!({"conversation_id": conv_id}));
            let prep_in = payload.get("input").cloned().unwrap_or_else(|| input.clone());
            let in_msgs = prep_in
                .get("messages")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_else(|| crate::web_api::reconstruct_v2_messages(store, conv_id).0);
            let tools = prep_in
                .get("tools")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            let mode = prep_in.get("mode").and_then(|v| v.as_str()).unwrap_or("budget");
            let keep_recent = prep_in
                .pointer("/budget/keep_recent_messages")
                .and_then(|v| v.as_u64())
                .unwrap_or(4) as usize;
            let ctx_window = prep_in
                .pointer("/budget/context_window_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(128_000);
            let reserved_output = prep_in
                .pointer("/budget/reserved_output_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(4_096);
            let prompt_limit = prep_in
                .pointer("/budget/prompt_token_limit")
                .and_then(|v| v.as_u64())
                .unwrap_or_else(|| ctx_window.saturating_sub(reserved_output));
            let target_summary_tokens = prep_in
                .pointer("/budget/target_summary_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(1_024);

            // Check already archived message IDs in conv_settings:{conv_id}
            let existing_settings = load_conversation_settings_str(store, conv_id)
                .and_then(|s| serde_json::from_str::<Value>(&s).ok())
                .unwrap_or_else(|| json!({}));
            let mut already_archived: HashSet<String> = HashSet::new();
            if let Some(entries) = existing_settings
                .get("compactedMemoryEntries")
                .and_then(|v| v.as_array())
            {
                for e in entries {
                    if let Some(ids) = e.get("archivedMessageIDs").and_then(|v| v.as_array()) {
                        for mid in ids {
                            if let Some(s) = mid.as_str() {
                                already_archived.insert(s.to_string());
                            }
                        }
                    }
                }
            }
            let unarchived: Vec<(usize, Value)> = in_msgs
                .iter()
                .enumerate()
                .filter(|(_, m)| {
                    m.get("id")
                        .and_then(|v| v.as_str())
                        .is_none_or(|mid| !already_archived.contains(mid))
                })
                .map(|(i, m)| (i, m.clone()))
                .collect();
            let msg_tokens: u64 = in_msgs
                .iter()
                .map(|m| nola_types::estimate_text_tokens(&m.to_string()) as u64)
                .sum();
            let tool_tokens: u64 = tools
                .iter()
                .map(|t| nola_types::estimate_text_tokens(&t.to_string()) as u64)
                .sum();
            let total_tokens = msg_tokens + tool_tokens;
            let need_archive = mode == "force_summary" && unarchived.len() > keep_recent;
            let insufficient_budget =
                ctx_window.saturating_sub(reserved_output) < 4_096 || prompt_limit < 4_096;
            let (action, reason, changed, archive_request) = if need_archive && insufficient_budget {
                ("cannot_fit", "insufficient_budget", false, Value::Null)
            } else if need_archive {
                let archive_count = unarchived.len().saturating_sub(keep_recent);
                let slice = &unarchived[..archive_count];
                let indices: Vec<usize> = slice.iter().map(|(i, _)| *i).collect();
                let ids: Vec<String> = slice
                    .iter()
                    .enumerate()
                    .map(|(k, (_, m))| {
                        m.get("id")
                            .and_then(|v| v.as_str())
                            .map(ToString::to_string)
                            .unwrap_or_else(|| format!("msg-{k}"))
                    })
                    .collect();
                let arc_msgs: Vec<Value> = slice.iter().map(|(_, m)| m.clone()).collect();
                let arc_digest = sha256_hex(
                    json!({"ids": &ids, "messages": &arc_msgs})
                        .to_string()
                        .as_bytes(),
                );
                let existing_summary = existing_settings
                    .get("sessionMemory")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
                    .map(ToString::to_string);
                (
                    "summary_required",
                    "force_summary",
                    true,
                    json!({
                        "archive_id": new_id(),
                        "archive_digest": arc_digest,
                        "message_indices": indices,
                        "message_ids": ids,
                        "messages": arc_msgs,
                        "existing_summary": existing_summary,
                        "target_summary_tokens": target_summary_tokens
                    }),
                )
            } else {
                ("ready", "within_budget", false, Value::Null)
            };
            let payload_digest = sha256_hex(
                json!({"messages": &in_msgs, "tools": &tools})
                    .to_string()
                    .as_bytes(),
            );
            let trigger_tokens = (prompt_limit as f64 * 0.85) as u64;
            let target_tokens = (prompt_limit as f64 * 0.65) as u64;
            let archive_msg_count = archive_request
                .get("message_ids")
                .and_then(|v| v.as_array())
                .map(|a| a.len() as u64)
                .unwrap_or(0);
            let output = json!({
                "schema_version": 1,
                "action": action,
                "messages": in_msgs,
                "tools": tools,
                "payload_digest": payload_digest,
                "changed": changed,
                "reason": reason,
                "metrics": {
                    "message_tokens": msg_tokens,
                    "tool_schema_tokens": tool_tokens,
                    "total_prompt_tokens": total_tokens,
                    "prompt_token_limit": prompt_limit,
                    "reserved_output_tokens": reserved_output,
                    "usage_percent": if prompt_limit > 0 { (total_tokens as f64 / prompt_limit as f64) * 100.0 } else { 0.0 },
                    "before_tokens": total_tokens,
                    "after_tokens": total_tokens,
                    "trigger_tokens": trigger_tokens,
                    "target_tokens": target_tokens,
                    "archive_message_count": archive_msg_count,
                    "transaction_count": if archive_msg_count > 0 { 1u64 } else { 0u64 }
                },
                "component_metrics": [],
                "archive_request": archive_request
            });
            Ok(json!({
                "conversation_id": conv_id,
                "message_count": in_msgs.len(),
                "recommended_actions": [],
                "source": source,
                "input": prep_in,
                "output": output,
                "instructions": "Summarize the archived conversation faithfully as a bounded checkpoint."
            }))
        }
        "commit_maintenance" => {
            let ts = input
                .get("committed_at_ms")
                .or_else(|| payload.get("committed_at_ms"))
                .and_then(|v| v.as_i64())
                .unwrap_or_else(now_ms);
            let summary = input
                .get("summary")
                .or_else(|| payload.get("summary"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let arc_req = input
                .get("archive_request")
                .or_else(|| payload.get("archive_request"))
                .or_else(|| payload.pointer("/plan/output/archive_request"));
            let mut archived_ids: Vec<String> = arc_req
                .and_then(|a| a.get("message_ids"))
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(ToString::to_string))
                        .collect()
                })
                .unwrap_or_default();
            if archived_ids.is_empty() {
                if let Some(orig) = input.get("original_input").or_else(|| payload.get("original_input")) {
                    let orig_msgs = orig.get("messages").and_then(|v| v.as_array());
                    let keep_recent = orig
                        .pointer("/budget/keep_recent_messages")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(4) as usize;
                    if let Some(msgs) = orig_msgs {
                        if msgs.len() > keep_recent {
                            let count = msgs.len().saturating_sub(keep_recent);
                            archived_ids = msgs[..count]
                                .iter()
                                .filter_map(|m| m.get("id").and_then(|v| v.as_str()).map(ToString::to_string))
                                .collect();
                        }
                    }
                }
            }
            // Reject fabricated lineage: every archived or input message_id must exist in this conversation.
            let mut lineage_ids: Vec<String> = archived_ids.clone();
            for field in ["original_input", "current_input"] {
                if let Some(msgs) = input
                    .get(field)
                    .or_else(|| payload.get(field))
                    .and_then(|v| v.get("messages"))
                    .and_then(|v| v.as_array())
                {
                    for m in msgs {
                        if let Some(mid) = m.get("id").and_then(|v| v.as_str()) {
                            lineage_ids.push(mid.to_string());
                        }
                    }
                }
            }
            if !lineage_ids.is_empty() {
                let (actual_msgs, _) = crate::web_api::reconstruct_v2_messages(store, conv_id);
                let actual_ids: HashSet<String> = actual_msgs
                    .iter()
                    .filter_map(|m| m.get("id").and_then(|v| v.as_str()).map(|s| s.to_ascii_lowercase()))
                    .collect();
                for aid in &lineage_ids {
                    if !actual_ids.contains(&aid.to_ascii_lowercase()) {
                        return Err((
                            "invalid_argument",
                            format!("fabricated lineage: message '{aid}' does not belong to conversation '{conv_id}'"),
                        ));
                    }
                }
            }
            // Persist sessionMemory, lastCompactedAt, compactedMemoryEntries into conv_settings:{conv_id}
            let mut settings_map = load_conversation_settings_str(store, conv_id)
                .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
                .and_then(|v| v.as_object().cloned())
                .unwrap_or_default();
            let now_iso = chrono::DateTime::from_timestamp_millis(ts)
                .map(|d| d.to_rfc3339())
                .unwrap_or_else(|| "2026-01-01T00:00:00Z".into());
            if !summary.trim().is_empty() {
                settings_map.insert("sessionMemory".into(), json!(summary));
            }
            settings_map.insert("lastCompactedAt".into(), json!(now_iso));
            let mut entries = settings_map
                .get("compactedMemoryEntries")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            if !archived_ids.is_empty() || !summary.trim().is_empty() {
                let raw_excerpt = input
                    .pointer("/archive_request/messages")
                    .or_else(|| input.pointer("/plan/output/archive_request/messages"))
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|m| m.get("content").and_then(|v| v.as_str()))
                            .collect::<Vec<_>>()
                            .join("\n")
                    })
                    .unwrap_or_default();
                let raw_excerpt: String = raw_excerpt.chars().take(512).collect();
                let archive_id = input
                    .pointer("/archive_request/archive_id")
                    .or_else(|| input.pointer("/plan/output/archive_request/archive_id"))
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string);
                let archive_digest = input
                    .pointer("/archive_request/archive_digest")
                    .or_else(|| input.pointer("/plan/output/archive_request/archive_digest"))
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string);
                let payload_digest_val = input
                    .pointer("/plan/output/payload_digest")
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string);
                entries.push(json!({
                    "id": new_id(),
                    "createdAt": now_iso,
                    "summary": summary,
                    "rawExcerpt": raw_excerpt,
                    "archivedMessageIDs": archived_ids,
                    "archiveID": archive_id,
                    "archiveDigest": archive_digest,
                    "payloadDigest": payload_digest_val
                }));
                settings_map.insert("compactedMemoryEntries".into(), Value::Array(entries));
            }
            let merged_str =
                serde_json::to_string(&Value::Object(settings_map)).unwrap_or_else(|_| "{}".into());
            let _ = store.set_kv(&format!("conv_settings:{conv_id}"), &json!(merged_str));

            let rec = json!({
                "conversation_id": conv_id,
                "committed_at_ms": ts,
                "details": input
            });
            store
                .set_kv(&format!("ctx_maintenance:{conv_id}"), &rec)
                .map_err(|e| ("storage_error", e.to_string()))?;

            let (msgs, _) = crate::web_api::reconstruct_v2_messages(store, conv_id);
            let msg_tokens: u64 = msgs
                .iter()
                .map(|m| nola_types::estimate_text_tokens(&m.to_string()) as u64)
                .sum();
            let payload_digest = sha256_hex(
                json!({"messages": &msgs, "summary": summary})
                    .to_string()
                    .as_bytes(),
            );
            Ok(json!({
                "ok": true,
                "conversation_id": conv_id,
                "committed_at_ms": ts,
                "schema_version": 1,
                "action": "ready",
                "messages": msgs,
                "tools": [],
                "payload_digest": payload_digest,
                "changed": true,
                "reason": "maintenance_committed",
                "metrics": {
                    "message_tokens": msg_tokens,
                    "tool_schema_tokens": 0u64,
                    "total_prompt_tokens": msg_tokens,
                    "prompt_token_limit": 112_000u64,
                    "reserved_output_tokens": 4_096u64,
                    "usage_percent": (msg_tokens as f64 / 112_000.0) * 100.0,
                    "before_tokens": msg_tokens,
                    "after_tokens": msg_tokens,
                    "trigger_tokens": 95_200u64,
                    "target_tokens": 72_800u64,
                    "archive_message_count": 0u64,
                    "transaction_count": 1u64
                },
                "component_metrics": []
            }))
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
    strict_projection_v1: bool,
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

    let mut canonical_messages: Vec<Value> = sliced
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
            let reasoning = m
                .get("reasoning")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let tool_calls = m
                .get("toolCalls")
                .or_else(|| m.get("tool_calls"))
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            let part_id = format!("{msg_id}:text");
            let ordinal = (idx + 1) as i64;
            if let Some(obj) = m.as_object_mut() {
                obj.insert("id".into(), json!(msg_id));
                obj.insert("message_id".into(), json!(msg_id));
                obj.insert("role".into(), json!(role));
                obj.insert("state".into(), json!("completed"));
                obj.insert("status".into(), json!("completed"));
                obj.insert("ordinal".into(), json!(ordinal));
                obj.insert("revision".into(), json!(1));
                obj.insert("created_at_ms".into(), json!(created_at_ms));
                obj.insert(
                    "parts".into(),
                    if strict_projection_v1 {
                        let mut parts: Vec<Value> = Vec::new();
                        if !reasoning.is_empty() {
                            parts.push(json!({
                                "kind": "reasoning",
                                "part_id": format!("{msg_id}:reasoning"),
                                "revision": 1,
                                "state": "completed",
                                "title": "Thinking",
                                "summary": reasoning
                            }));
                        }
                        for (tidx, tc) in tool_calls.iter().enumerate() {
                            let call_id = tc
                                .get("call_id")
                                .or_else(|| tc.get("id"))
                                .and_then(|v| v.as_str())
                                .filter(|s| !s.trim().is_empty())
                                .map(ToString::to_string)
                                .unwrap_or_else(|| format!("{msg_id}:call:{}", tidx + 1));
                            let name = tc
                                .get("name")
                                .or_else(|| tc.get("tool"))
                                .and_then(|v| v.as_str())
                                .filter(|s| !s.trim().is_empty())
                                .unwrap_or("tool");
                            let is_error = tc
                                .get("is_error")
                                .or_else(|| tc.get("isError"))
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false);
                            let summary = tc
                                .get("summary")
                                .and_then(|v| v.as_str())
                                .filter(|s| !s.is_empty());
                            let arguments = tc
                                .get("arguments")
                                .or_else(|| tc.get("input"))
                                .cloned()
                                .unwrap_or_else(|| json!({}));
                            parts.push(json!({
                                "kind": "tool",
                                "part_id": format!("tool:{call_id}"),
                                "revision": 1,
                                "execution_source": {"kind": "model"},
                                "tool_call_id": call_id,
                                "capability_id": name,
                                "title": name,
                                "summary": summary,
                                "origin": {"kind": "core"},
                                "permission": Value::Null,
                                "state": if is_error { "failed" } else { "completed" },
                                "display_key": Value::Null,
                                "input": arguments,
                                "output_preview": summary,
                                "full_output": Value::Null,
                                "artifacts": [],
                                "started_at_ms": tc.get("started_at_ms").and_then(|v| v.as_u64()),
                                "finished_at_ms": tc.get("finished_at_ms").and_then(|v| v.as_u64()),
                                "affected_resources": []
                            }));
                        }
                        parts.push(json!({
                            "kind": "text",
                            "part_id": part_id,
                            "revision": 1,
                            "markdown": content,
                            "is_final": true
                        }));
                        Value::Array(parts)
                    } else {
                        json!([{
                            "id": part_id,
                            "ordinal": 1,
                            "kind": "text",
                            "state": "completed",
                            "revision": 1,
                            "content_text": content,
                            "created_at_ms": created_at_ms,
                            "updated_at_ms": created_at_ms
                        }])
                    },
                );
            }
            m
        })
        .collect();

    let active_run_val = lookup_active_run(store, &sess.id)
        .map(|mut ar| {
            let rid = ar.get("run_id").and_then(|v| v.as_str()).unwrap_or("");
            let committed_run = if !rid.is_empty() {
                store.get_kv(&format!("committed_run:{rid}")).ok().flatten()
            } else {
                None
            };
            let turn_st = committed_run.as_ref().and_then(|r| {
                r.get("turn_status")
                    .or_else(|| r.get("status"))
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string)
            });
            let state = match turn_st.as_deref().or_else(|| ar.get("status").and_then(|v| v.as_str())) {
                Some("awaiting_permission" | "waiting_for_permission") => "waiting_for_permission",
                Some("awaiting_input" | "waiting_for_input") => "waiting_for_input",
                Some("pending") => "pending",
                Some("completed") => "completed",
                Some("sealed") => "sealed",
                Some("cancelled") => "cancelled",
                Some("failed") => "failed",
                _ => "running",
            };
            if let Some(obj) = ar.as_object_mut() {
                obj.insert("state".into(), json!(state));
            }
            if let Some(amid) = ar
                .get("assistant_message_id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
            {
                let already_present = canonical_messages.iter().any(|m| {
                    m.get("message_id")
                        .or_else(|| m.get("id"))
                        .and_then(|v| v.as_str())
                        == Some(amid)
                });
                if !already_present {
                    let ord = (canonical_messages.len() + 1) as i64;
                    let created = committed_run
                        .as_ref()
                        .and_then(|r| r.get("created_at_ms").and_then(|v| v.as_i64()))
                        .unwrap_or(sess.updated_at_ms);
                    canonical_messages.push(json!({
                        "id": amid,
                        "message_id": amid,
                        "role": "assistant",
                        "state": "running",
                        "status": "running",
                        "ordinal": ord,
                        "revision": 1,
                        "created_at_ms": created,
                        "parts": []
                    }));
                }
            }
            ar
        })
        .unwrap_or(Value::Null);
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

    let assistant_journal_heads = store
        .get_kv(&format!("journal_heads_by_conv:{}", sess.id))
        .ok()
        .flatten()
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();

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
        "stream_owners": Value::Null,
        "event_sequence": event_sequence,
        "total_message_count": total_message_count,
        "before_ordinal": next_before_ordinal,
        "assistant_journal_heads": assistant_journal_heads,
        "deferred_text_parts": []
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
        if let Some(exp) = input
            .get("expected_revision")
            .or_else(|| args.get("expected_revision"))
            .and_then(|v| v.as_u64())
        {
            if exp != revision {
                return Err((
                    "revision_conflict",
                    json!({
                        "scope_id": format!("model_profile:{profile_id}"),
                        "expected_revision": exp,
                        "actual_revision": revision
                    })
                    .to_string(),
                ));
            }
        }
        // Resolve runtime reasoning selection and capabilities from source_json + request + parameters.
        let req_obj = input
            .get("request")
            .or_else(|| args.get("request"))
            .and_then(|v| v.as_object());
        let src_reasoning = source_val
            .get("reasoning")
            .or_else(|| source_val.pointer("/capabilities/reasoning"))
            .and_then(|v| v.as_object());
        let control = src_reasoning
            .and_then(|r| r.get("control"))
            .and_then(|v| v.as_str());
        let efforts: Vec<String> = src_reasoning
            .and_then(|r| r.get("efforts").or_else(|| r.get("effort_levels")))
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(ToString::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let explicit_selection = req_obj.and_then(|r| r.get("reasoning_selection"));
        let requested_tier = req_obj
            .and_then(|r| r.get("reasoning_tier"))
            .and_then(|v| v.as_str())
            .or_else(|| {
                params_val
                    .get("reasoningTier")
                    .or_else(|| params_val.get("reasoning_tier"))
                    .or_else(|| params_val.get("reasoningPreference"))
                    .or_else(|| params_val.get("reasoning_preference"))
                    .and_then(|v| v.as_str())
            });
        let reasoning_selection = if let Some(sel) = explicit_selection {
            sel.clone()
        } else if control == Some("effort") && !efforts.is_empty() {
            let tier = requested_tier.unwrap_or("balanced");
            let idx = match tier {
                "concise" | "low" => 0,
                "detailed" | "high" => efforts.len() - 1,
                _ => efforts.len() / 2,
            };
            json!({"kind": "effort", "value": efforts[idx]})
        } else if control == Some("budget") {
            let min_t = src_reasoning
                .and_then(|r| r.get("min_tokens"))
                .and_then(|v| v.as_u64())
                .unwrap_or(1024);
            let max_t = src_reasoning
                .and_then(|r| r.get("max_tokens"))
                .and_then(|v| v.as_u64())
                .unwrap_or(16384);
            let tier = requested_tier.unwrap_or("balanced");
            let tokens = match tier {
                "concise" | "low" => min_t,
                "detailed" | "high" => max_t,
                _ => (min_t + max_t) / 2,
            };
            json!({"kind": "budget", "tokens": tokens})
        } else {
            Value::Null
        };
        let reasoning_summary_supported = source_val
            .pointer("/capabilities/reasoning_summary_supported")
            .or_else(|| source_val.get("reasoning_summary_supported"))
            .cloned()
            .unwrap_or(Value::Null);
        let runtime = json!({
            "reasoning_selection": reasoning_selection,
            "reasoning_tier": requested_tier,
            "model_capabilities": {
                "reasoning_summary_supported": reasoning_summary_supported
            }
        });
        let resolved = json!({
            "profile_id": profile_id,
            "id": profile_id,
            "provider": provider,
            "model": model_name,
            "model_name": model_name,
            "endpoint": endpoint,
            "wire_protocol": wire_protocol,
            "parameters": params_val,
            "revision": revision,
            "runtime": runtime
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
    let default_mode = rec
        .get("default_mode")
        .cloned()
        .unwrap_or(Value::Null);
    let deleted_at_ms = rec
        .get("deleted_at_ms")
        .cloned()
        .unwrap_or(Value::Null);
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
            "default_mode": default_mode,
            "version": version,
            "enabled": enabled,
            "created_at_ms": created_at_ms,
            "updated_at_ms": updated_at_ms,
            "deleted_at_ms": deleted_at_ms
        },
        "revision": revision
    })
}

fn canonicalize_data_source_record(mut rec: Value, fallback_id: Option<&str>) -> Value {
    let ts = now_ms();
    let Some(obj) = rec.as_object_mut() else {
        return json!({});
    };
    let id = obj
        .get("id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(ToString::to_string)
        .or_else(|| fallback_id.map(ToString::to_string))
        .unwrap_or_else(new_id);
    obj.insert("id".into(), json!(id));
    if obj
        .get("name")
        .and_then(|v| v.as_str())
        .is_none_or(|s| s.is_empty())
    {
        let fallback_name = obj
            .get("title")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(&id)
            .to_string();
        obj.insert("name".into(), json!(fallback_name));
    }
    obj.entry("kind".to_string()).or_insert_with(|| json!("http"));
    obj.entry("configuration".to_string()).or_insert_with(|| {
        json!({
            "connector_id": "generic",
            "location_kind": "http_url",
            "location": "",
            "parameters": {},
            "secret_references": []
        })
    });
    obj.entry("access_policy".to_string())
        .or_insert_with(|| json!("read_only"));
    obj.entry("authorization".to_string()).or_insert_with(|| {
        json!({
            "state": "not_required",
            "capability": Value::Null,
            "observed_at_ms": ts
        })
    });
    if !obj.get("validation").is_some_and(|v| v.is_object() && v.get("status").is_some()) {
        obj.insert(
            "validation".into(),
            json!({
                "status": "not_validated",
                "validation_token": Value::Null,
                "contract": Value::Null,
                "diagnostic": Value::Null,
                "attempted_at_ms": Value::Null,
                "completed_at_ms": Value::Null
            }),
        );
    }
    if !obj.get("read_state").is_some_and(|v| v.is_object() && v.get("status").is_some()) {
        obj.insert(
            "read_state".into(),
            json!({
                "status": "never_read",
                "read_token": Value::Null,
                "content_reference": Value::Null,
                "observed_at_ms": Value::Null,
                "schema_version": Value::Null,
                "record_count": Value::Null,
                "diagnostic": Value::Null,
                "requested_at_ms": Value::Null,
                "completed_at_ms": Value::Null
            }),
        );
    } else if let Some(rs_obj) = obj.get_mut("read_state").and_then(|v| v.as_object_mut()) {
        for field in [
            "read_token",
            "content_reference",
            "observed_at_ms",
            "schema_version",
            "record_count",
            "diagnostic",
            "requested_at_ms",
            "completed_at_ms",
        ] {
            rs_obj.entry(field.to_string()).or_insert(Value::Null);
        }
    }
    obj.entry("freshness_policy".to_string()).or_insert_with(|| {
        json!({
            "maximum_age_minutes": 60,
            "allow_stale_fallback": true
        })
    });
    let default_en = obj
        .get("default_enabled")
        .or_else(|| obj.get("enabled"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    obj.insert("default_enabled".into(), json!(default_en));
    obj.entry("lifecycle".to_string())
        .or_insert_with(|| json!("draft"));
    obj.entry("capabilities".to_string())
        .or_insert_with(|| json!([]));
    obj.entry("privacy".to_string())
        .or_insert_with(|| json!("local"));
    let rev = obj
        .get("revision")
        .and_then(|v| v.as_i64())
        .unwrap_or(1)
        .max(1);
    obj.insert("revision".into(), json!(rev));
    obj.entry("last_successful_read_at_ms".to_string())
        .or_insert(Value::Null);
    obj.entry("created_at_ms".to_string())
        .or_insert_with(|| json!(ts));
    obj.entry("updated_at_ms".to_string())
        .or_insert_with(|| json!(ts));
    obj.entry("deleted_at_ms".to_string()).or_insert(Value::Null);
    rec
}

fn save_memory_extra_meta(store: &Store, id: &str, payload: &Value) {
    let input = payload.get("input").unwrap_or(payload);
    let rec = input.get("record").unwrap_or(input);
    let mut meta = serde_json::Map::new();
    for k in [
        "owner",
        "namespace",
        "scope",
        "kind",
        "summary",
        "confidence",
        "importance",
        "sensitivity",
        "retention",
        "provenance",
        "supersedes",
    ] {
        if let Some(v) = rec.get(k) {
            meta.insert(k.to_string(), v.clone());
        }
    }
    if !meta.is_empty() {
        let _ = store.set_kv(&format!("mem_meta:{id}"), &Value::Object(meta));
    }
}

fn canonical_memory_envelope_with_meta(store: &Store, item: &MemoryItem) -> Value {
    let mut env = canonical_memory_envelope(item);
    if let Ok(Some(Value::Object(meta))) = store.get_kv(&format!("mem_meta:{}", item.id)) {
        if let Some(rec) = env.get_mut("record").and_then(|v| v.as_object_mut()) {
            for (k, v) in meta {
                rec.insert(k, v);
            }
        }
    }
    env
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
    let ns_kind = if matches!(ns, "long_term" | "user_preferences") {
        "custom"
    } else {
        "user"
    };
    json!({
        "record": {
            "id": item.id,
            "owner": {"kind": "profile", "profile_id": "local", "id": "local"},
            "namespace": {"kind": ns_kind, "name": ns},
            "scope": scope,
            "kind": kind,
            "title": item.title,
            "summary": item.title,
            "content": item.content,
            "tags": item.tags,
            "confidence": 1.0,
            "importance": 0.5,
            "sensitivity": Value::Null,
            "supersedes": [],
            "retention": {"mode": "permanent"},
            "provenance": {"kind": "explicit_user", "source": "user", "item_ids": []}
        },
        "created_at_ms": item.created_at_ms,
        "updated_at_ms": item.updated_at_ms,
        "last_used_at_ms": Value::Null,
        "deleted_at_ms": Value::Null
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
                .list_records::<Value>("model")
                .map_err(|e| ("storage_error", e.to_string()))?
                .into_iter()
                .filter(|r| r.get("deleted_at_ms").is_none_or(|v| v.is_null()))
                .collect();
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
                .map_err(|e| ("storage_error", e.to_string()))?
                .filter(|r| r.get("deleted_at_ms").is_none_or(|v| v.is_null()));
            Ok(found.unwrap_or(Value::Null))
        }
        ("model", "list_connections") => {
            let cfg = crate::config::load_active_config(store);
            let mut list: Vec<Value> = Vec::new();
            let mut seen_ids: HashSet<String> = HashSet::new();
            for rec in suite.list_records::<Value>("model_connection").unwrap_or_default() {
                let rid = rec.get("id").and_then(|v| v.as_str()).map(ToString::to_string);
                if let Some(ref id) = rid {
                    seen_ids.insert(id.clone());
                }
                let rev = rec.get("revision").and_then(|v| v.as_i64()).unwrap_or(1);
                let catalog = rec.get("catalog").cloned().unwrap_or(Value::Null);
                list.push(json!({
                    "id": rid,
                    "revision": rev,
                    "document": {
                        "format_version": 1,
                        "connection": rec,
                        "catalog": catalog
                    },
                    "connection": rec,
                    "catalog": catalog
                }));
            }
            if let Some(cfg_conns) = crate::config::redacted_llm_config(&cfg.llm)
                .get("connections")
                .and_then(|v| v.as_array())
            {
                for c in cfg_conns {
                    let cid = c.get("id").and_then(|v| v.as_str()).unwrap_or("");
                    if !cid.is_empty() && !seen_ids.contains(cid) {
                        list.push(c.clone());
                    }
                }
            }
            Ok(json!({"connections": list, "items": list}))
        }
        ("model", "list_accounts") => {
            let mut list: Vec<Value> = Vec::new();
            for rec in suite.list_records::<Value>("model_account").unwrap_or_default() {
                let rev = rec.get("revision").and_then(|v| v.as_i64()).unwrap_or(1);
                list.push(json!({
                    "revision": rev,
                    "account": rec
                }));
            }
            Ok(json!({"accounts": list, "items": list}))
        }
        ("model", "get_connection") => {
            let id = payload
                .get("id")
                .or_else(|| payload.get("connection_id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            if let Ok(Some(rec)) = suite.get_record::<Value>("model_connection", id) {
                let rev = rec.get("revision").and_then(|v| v.as_i64()).unwrap_or(1);
                let catalog = rec.get("catalog").cloned().unwrap_or(Value::Null);
                return Ok(json!({
                    "id": id,
                    "revision": rev,
                    "document": {
                        "format_version": 1,
                        "connection": rec,
                        "catalog": catalog
                    },
                    "connection": rec,
                    "catalog": catalog
                }));
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
        ("model", "get_account") => {
            let id = payload
                .get("id")
                .or_else(|| payload.get("account_id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            if let Ok(Some(rec)) = suite.get_record::<Value>("model_account", id) {
                let rev = rec.get("revision").and_then(|v| v.as_i64()).unwrap_or(1);
                return Ok(json!({
                    "revision": rev,
                    "account": rec
                }));
            }
            Ok(Value::Null)
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
            let prev_rev = suite
                .get_record::<Value>("model_account", &id)
                .ok()
                .flatten()
                .and_then(|v| v.get("revision").and_then(|r| r.as_i64()))
                .unwrap_or(0);
            let next_rev = prev_rev + 1;
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
                obj.insert("revision".into(), json!(next_rev));
                obj.remove("api_key");
                obj.remove("apiKey");
                obj.remove("token");
                obj.insert("updated_at_ms".into(), json!(now_ms()));
            }
            suite
                .upsert_record("model_account", &id, &rec)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"ok": true, "id": id, "revision": next_rev, "account": rec}))
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
            let prev_rev = suite
                .get_record::<Value>("model_connection", &id)
                .ok()
                .flatten()
                .and_then(|v| v.get("revision").and_then(|r| r.as_i64()))
                .unwrap_or(0);
            let next_rev = prev_rev + 1;
            let mut sanitized = input.clone();
            if let Some(obj) = sanitized.as_object_mut() {
                obj.insert("id".into(), json!(id));
                obj.insert("connection_id".into(), json!(id));
                obj.insert("revision".into(), json!(next_rev));
                obj.remove("api_key");
                obj.remove("apiKey");
                obj.insert("updated_at_ms".into(), json!(now_ms()));
            }
            let catalog = sanitized.get("catalog").cloned().unwrap_or(Value::Null);
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
            Ok(json!({
                "ok": true,
                "id": id,
                "revision": next_rev,
                "document": {
                    "format_version": 1,
                    "connection": sanitized,
                    "catalog": catalog
                },
                "connection": sanitized,
                "catalog": catalog
            }))
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
            let ts = payload
                .get("updated_at_ms")
                .and_then(|v| v.as_i64())
                .unwrap_or_else(now_ms);
            if let Some(acct_val) = payload.get("account").filter(|v| v.is_object()) {
                let acct_id = acct_val
                    .get("id")
                    .or_else(|| acct_val.get("account_id"))
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
                    .map(ToString::to_string)
                    .unwrap_or_else(new_id);
                let prev_acct_rev = suite
                    .get_record::<Value>("model_account", &acct_id)
                    .ok()
                    .flatten()
                    .and_then(|v| v.get("revision").and_then(|r| r.as_i64()))
                    .unwrap_or(0);
                let mut acct_rec = acct_val.clone();
                if let Some(obj) = acct_rec.as_object_mut() {
                    obj.insert("id".into(), json!(acct_id));
                    obj.insert("account_id".into(), json!(acct_id));
                    obj.insert("revision".into(), json!(prev_acct_rev + 1));
                    obj.remove("api_key");
                    obj.remove("apiKey");
                    obj.remove("token");
                    obj.insert("updated_at_ms".into(), json!(ts));
                }
                suite
                    .upsert_record("model_account", &acct_id, &acct_rec)
                    .map_err(|e| ("storage_error", e.to_string()))?;
            }
            let prev_rev = suite
                .get_record::<Value>("model_connection", &id)
                .ok()
                .flatten()
                .and_then(|v| v.get("revision").and_then(|r| r.as_i64()))
                .unwrap_or(0);
            let next_rev = prev_rev + 1;
            let catalog = payload
                .get("models")
                .and_then(|v| v.as_array())
                .map(|models| {
                    json!({
                        "observed_at_ms": ts,
                        "source": input.get("source").and_then(|v| v.as_str()).unwrap_or("manual"),
                        "models": models
                    })
                })
                .unwrap_or(Value::Null);
            let mut rec = input.clone();
            if let Some(obj) = rec.as_object_mut() {
                obj.insert("id".into(), json!(id));
                obj.insert("revision".into(), json!(next_rev));
                obj.insert("updated_at_ms".into(), json!(ts));
                if !catalog.is_null() {
                    obj.insert("catalog".into(), catalog.clone());
                }
            }
            suite
                .upsert_record("model_connection", &id, &rec)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({
                "ok": true,
                "id": id,
                "revision": next_rev,
                "document": {
                    "format_version": 1,
                    "connection": rec,
                    "catalog": catalog
                },
                "connection": rec,
                "catalog": catalog
            }))
        }
        ("model", "report_runtime") => {
            let input = payload.get("input").unwrap_or(payload);
            let id = input
                .get("host_id")
                .or_else(|| input.get("runtime_id"))
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
                .get("host_id")
                .or_else(|| payload.get("runtime_id"))
                .or_else(|| payload.get("id"))
                .or_else(|| args.get("host_id"))
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
            let existing = suite
                .get_record::<Value>("model", &id)
                .map_err(|e| ("storage_error", e.to_string()))?;
            let current_rev = existing
                .as_ref()
                .and_then(|r| r.get("revision"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let expected_rev = args
                .get("__envelope_expected_revision")
                .or_else(|| payload.get("expected_revision"))
                .or_else(|| input.get("expected_revision"))
                .and_then(|v| v.as_u64());
            if let Some(exp) = expected_rev {
                if exp != current_rev {
                    return Err((
                        "revision_conflict",
                        json!({
                            "scope_id": format!("model_profile:{id}"),
                            "expected_revision": exp,
                            "actual_revision": current_rev
                        })
                        .to_string(),
                    ));
                }
            }
            if let Some(ref ex) = existing {
                if ex.get("deleted_at_ms").is_some_and(|v| !v.is_null()) {
                    return Err((
                        "conflict",
                        format!("model profile '{id}' was deleted and cannot be resurrected"),
                    ));
                }
                let old_provider = ex.get("provider").and_then(|v| v.as_str()).unwrap_or("");
                let new_provider = record
                    .get("provider")
                    .and_then(|v| v.as_str())
                    .unwrap_or(old_provider);
                if !old_provider.is_empty() && !new_provider.is_empty() && old_provider != new_provider {
                    return Err((
                        "invalid_argument",
                        format!(
                            "model profile '{id}' cannot change provider ownership from '{old_provider}' to '{new_provider}'"
                        ),
                    ));
                }
            }
            let next_rev = current_rev + 1;
            let ts = record
                .get("updated_at_ms")
                .or_else(|| input.get("updated_at_ms"))
                .and_then(|v| v.as_i64())
                .unwrap_or_else(now_ms);
            let created_at = existing
                .as_ref()
                .and_then(|r| r.get("created_at_ms"))
                .and_then(|v| v.as_i64())
                .or_else(|| record.get("created_at_ms").and_then(|v| v.as_i64()))
                .unwrap_or(ts);
            let mut stored = record.clone();
            if let Some(obj) = stored.as_object_mut() {
                obj.insert("id".into(), json!(id));
                obj.insert("revision".into(), json!(next_rev));
                obj.insert("created_at_ms".into(), json!(created_at));
                obj.insert("updated_at_ms".into(), json!(ts));
                obj.entry("selected".to_string()).or_insert_with(|| json!(false));
                obj.entry("enabled".to_string()).or_insert_with(|| json!(true));
                obj.entry("source_json".to_string()).or_insert_with(|| json!("{}"));
                obj.entry("parameters_json".to_string()).or_insert_with(|| json!("{}"));
            }
            if stored.get("selected").and_then(|v| v.as_bool()) == Some(true) {
                if let Ok(others) = suite.list_records::<Value>("model") {
                    for mut other in others {
                        let oid = other
                            .get("id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        if !oid.is_empty()
                            && oid != id
                            && other.get("selected").and_then(|v| v.as_bool()) == Some(true)
                        {
                            if let Some(obj) = other.as_object_mut() {
                                obj.insert("selected".into(), json!(false));
                            }
                            let _ = suite.upsert_record("model", &oid, &other);
                        }
                    }
                }
            }
            suite
                .upsert_record("model", &id, &stored)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({
                "ok": true,
                "id": id,
                "revision": next_rev,
                "__committed_revision": next_rev,
                "profile": stored
            }))
        }
        ("model", "delete_profile" | "delete") => {
            let id = payload
                .get("id")
                .or_else(|| payload.get("profile_id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            let existing = suite
                .get_record::<Value>("model", id)
                .map_err(|e| ("storage_error", e.to_string()))?;
            let Some(mut rec) = existing.filter(|r| r.get("deleted_at_ms").is_none_or(|v| v.is_null())) else {
                return Ok(json!(false));
            };
            let cur_rev = rec.get("revision").and_then(|v| v.as_u64()).unwrap_or(1);
            let ts = payload
                .get("deleted_at_ms")
                .and_then(|v| v.as_i64())
                .unwrap_or_else(now_ms);
            if let Some(obj) = rec.as_object_mut() {
                obj.insert("selected".into(), json!(false));
                obj.insert("enabled".into(), json!(false));
                obj.insert("revision".into(), json!(cur_rev + 1));
                obj.insert("deleted_at_ms".into(), json!(ts));
                obj.insert("updated_at_ms".into(), json!(ts));
            }
            suite
                .upsert_record("model", id, &rec)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!(true))
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
            let existing = suite
                .get_record::<Value>("model", id)
                .map_err(|e| ("storage_error", e.to_string()))?;
            let current_rev = existing
                .as_ref()
                .and_then(|r| r.get("revision"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let expected_rev = args
                .get("__envelope_expected_revision")
                .or_else(|| payload.get("expected_revision"))
                .or_else(|| input.get("expected_revision"))
                .and_then(|v| v.as_u64());
            if let Some(exp) = expected_rev {
                if exp != current_rev {
                    return Err((
                        "revision_conflict",
                        json!({
                            "scope_id": format!("model_profile:{id}"),
                            "expected_revision": exp,
                            "actual_revision": current_rev
                        })
                        .to_string(),
                    ));
                }
            }
            let mut rec = match existing {
                Some(ex) => ex,
                None => {
                    let mut base = input.clone();
                    if let Some(obj) = base.as_object_mut() {
                        obj.insert("id".into(), json!(id));
                        obj.entry("display_name".to_string()).or_insert_with(|| json!(id));
                        obj.entry("provider".to_string()).or_insert_with(|| json!("local"));
                        obj.entry("model_name".to_string()).or_insert_with(|| json!(id));
                        obj.entry("runtime".to_string()).or_insert_with(|| json!("mlx"));
                        obj.entry("source_json".to_string()).or_insert_with(|| json!("{}"));
                        obj.entry("selected".to_string()).or_insert_with(|| json!(false));
                        obj.entry("enabled".to_string()).or_insert_with(|| json!(true));
                    }
                    base
                }
            };
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
            let next_rev = current_rev + 1;
            let ts = input
                .get("updated_at_ms")
                .and_then(|v| v.as_i64())
                .unwrap_or_else(now_ms);
            if let Some(obj) = rec.as_object_mut() {
                obj.insert("id".into(), json!(id));
                obj.insert("parameters".into(), params);
                obj.insert("parameters_json".into(), json!(params_json_str));
                obj.insert("revision".into(), json!(next_rev));
                obj.entry("created_at_ms".to_string()).or_insert_with(|| {
                    input
                        .get("created_at_ms")
                        .cloned()
                        .unwrap_or_else(|| json!(ts))
                });
                obj.insert("updated_at_ms".into(), json!(ts));
            }
            suite
                .upsert_record("model", id, &rec)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({
                "ok": true,
                "id": id,
                "revision": next_rev,
                "__committed_revision": next_rev,
                "parameters_json": params_json_str,
                "profile": rec
            }))
        }
        ("model", "reset_parameters") => {
            let input = payload.get("input").unwrap_or(payload);
            let params_json_str = input
                .get("parameters_json")
                .or_else(|| payload.get("parameters_json"))
                .and_then(|v| v.as_str())
                .unwrap_or("{}")
                .to_string();
            let params_val =
                serde_json::from_str::<Value>(&params_json_str).unwrap_or_else(|_| json!({}));
            let ts = input
                .get("updated_at_ms")
                .or_else(|| payload.get("updated_at_ms"))
                .and_then(|v| v.as_i64())
                .unwrap_or_else(now_ms);
            if let Some(ids_arr) = input
                .get("profile_ids")
                .or_else(|| payload.get("profile_ids"))
                .or_else(|| args.get("profile_ids"))
                .and_then(|v| v.as_array())
            {
                // Validate ALL profile_ids exist atomically before mutating any record.
                let mut staged: Vec<(String, Value)> = Vec::with_capacity(ids_arr.len());
                for id_val in ids_arr {
                    let id = id_val
                        .as_str()
                        .filter(|s| !s.trim().is_empty())
                        .ok_or(("invalid_argument", "profile_ids contains empty id".into()))?;
                    let mut rec = suite
                        .get_record::<Value>("model", id)
                        .map_err(|e| ("storage_error", e.to_string()))?
                        .ok_or(("not_found", format!("model profile '{id}' not found")))?;
                    let next_rev =
                        rec.get("revision").and_then(|v| v.as_u64()).unwrap_or(0) + 1;
                    if let Some(obj) = rec.as_object_mut() {
                        obj.insert("parameters".into(), params_val.clone());
                        obj.insert("parameters_json".into(), json!(params_json_str));
                        obj.insert("revision".into(), json!(next_rev));
                        obj.insert("updated_at_ms".into(), json!(ts));
                    }
                    staged.push((id.to_string(), rec));
                }
                let mut updated = Vec::with_capacity(staged.len());
                let mut ids = Vec::with_capacity(staged.len());
                for (id, rec) in staged {
                    suite
                        .upsert_record("model", &id, &rec)
                        .map_err(|e| ("storage_error", e.to_string()))?;
                    ids.push(id);
                    updated.push(rec);
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
                .ok_or(("not_found", format!("model profile '{id}' not found")))?;
            let next_rev = rec.get("revision").and_then(|v| v.as_u64()).unwrap_or(0) + 1;
            if let Some(obj) = rec.as_object_mut() {
                obj.insert("parameters".into(), params_val);
                obj.insert("parameters_json".into(), json!(params_json_str));
                obj.insert("revision".into(), json!(next_rev));
                obj.insert("updated_at_ms".into(), json!(ts));
            }
            suite
                .upsert_record("model", id, &rec)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({
                "ok": true,
                "id": id,
                "revision": next_rev,
                "__committed_revision": next_rev,
                "parameters_json": params_json_str,
                "profile": rec
            }))
        }
        ("model", "replace_profiles") => {
            let profiles = payload
                .get("profiles")
                .or_else(|| args.get("profiles"))
                .and_then(|v| v.as_array())
                .ok_or(("invalid_argument", "missing 'profiles' array".into()))?;
            let scope_provider = payload
                .get("provider")
                .or_else(|| args.get("provider"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty());
            let scope_runtime = payload
                .get("runtime")
                .or_else(|| args.get("runtime"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty());
            let expected_revs = payload
                .get("expected_revisions")
                .or_else(|| args.get("expected_revisions"))
                .and_then(|v| v.as_object());
            let ts = payload
                .get("updated_at_ms")
                .or_else(|| args.get("updated_at_ms"))
                .and_then(|v| v.as_i64())
                .unwrap_or_else(now_ms);
            let existing: Vec<Value> = suite
                .list_records::<Value>("model")
                .unwrap_or_default()
                .into_iter()
                .filter(|r| r.get("deleted_at_ms").is_none_or(|v| v.is_null()))
                .collect();
            let mut existing_by_id: BTreeMap<String, Value> = BTreeMap::new();
            let mut scoped_existing_ids: Vec<String> = Vec::new();
            for ex in &existing {
                if let Some(oid) = ex.get("id").and_then(|v| v.as_str()) {
                    existing_by_id.insert(oid.to_string(), ex.clone());
                    let matches_provider = scope_provider
                        .is_none_or(|sp| ex.get("provider").and_then(|v| v.as_str()) == Some(sp));
                    let matches_runtime = scope_runtime
                        .is_none_or(|sr| ex.get("runtime").and_then(|v| v.as_str()) == Some(sr));
                    if matches_provider && matches_runtime {
                        scoped_existing_ids.push(oid.to_string());
                    }
                }
            }
            if let Some(exp_map) = expected_revs {
                // Verify all scoped existing profiles and expected entries match revisions atomically.
                for oid in &scoped_existing_ids {
                    let actual = existing_by_id
                        .get(oid)
                        .and_then(|r| r.get("revision"))
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    let expected = exp_map.get(oid).and_then(|v| v.as_u64()).unwrap_or(0);
                    if expected != actual {
                        return Err((
                            "revision_conflict",
                            format!(
                                "revision_conflict: model profile '{oid}' expected revision {expected}, actual {actual}"
                            ),
                        ));
                    }
                }
                for (exp_id, exp_val) in exp_map {
                    let expected = exp_val.as_u64().unwrap_or(0);
                    let actual = existing_by_id
                        .get(exp_id)
                        .and_then(|r| r.get("revision"))
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    if expected != actual {
                        return Err((
                            "revision_conflict",
                            format!(
                                "revision_conflict: model profile '{exp_id}' expected revision {expected}, actual {actual}"
                            ),
                        ));
                    }
                }
            }
            let incoming_ids: HashSet<String> = profiles
                .iter()
                .filter_map(|p| p.get("id").and_then(|v| v.as_str()).map(ToString::to_string))
                .collect();
            for oid in &scoped_existing_ids {
                if !incoming_ids.contains(oid) {
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
                let prev = existing_by_id.get(&id);
                let prev_rev = prev
                    .and_then(|r| r.get("revision"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let unchanged = prev.is_some_and(|ex| {
                    ex.get("display_name") == p.get("display_name")
                        && ex.get("provider") == p.get("provider")
                        && ex.get("model_name") == p.get("model_name")
                        && ex.get("runtime") == p.get("runtime")
                        && ex.get("source_json") == p.get("source_json")
                        && ex.get("parameters_json") == p.get("parameters_json")
                        && ex.get("selected") == p.get("selected")
                        && ex.get("enabled") == p.get("enabled")
                        && ex.get("deleted_at_ms") == p.get("deleted_at_ms")
                });
                if unchanged {
                    if let Some(ex) = prev {
                        saved.push(ex.clone());
                        continue;
                    }
                }
                let next_rev = prev_rev + 1;
                let mut rec = p.clone();
                if let Some(obj) = rec.as_object_mut() {
                    obj.insert("id".into(), json!(id));
                    obj.insert("revision".into(), json!(next_rev));
                    if let Some(prev_rec) = prev {
                        if let Some(created) = prev_rec.get("created_at_ms") {
                            obj.insert("created_at_ms".into(), created.clone());
                        }
                    }
                    obj.entry("updated_at_ms".to_string()).or_insert_with(|| json!(ts));
                }
                suite
                    .upsert_record("model", &id, &rec)
                    .map_err(|e| ("storage_error", e.to_string()))?;
                saved.push(rec);
            }
            let all_now: Vec<Value> = suite.list_records("model").unwrap_or_default();
            let _ = store.set_kv("web_model_profiles", &json!(all_now));
            Ok(json!({"ok": true, "count": saved.len(), "profiles": saved}))
        }

        // --- Skill Domain ---
        ("skill", "list" | "list_editors") => {
            let persisted: Vec<Value> = suite
                .list_records::<Value>("skill")
                .unwrap_or_default()
                .into_iter()
                .filter(|r| r.get("deleted_at_ms").is_none_or(|v| v.is_null()))
                .collect();
            if op == "list_editors" {
                let items: Vec<Value> = persisted
                    .iter()
                    .map(|raw| {
                        let rev = raw.get("revision").and_then(|v| v.as_u64()).unwrap_or(1);
                        canonical_skill_editor_envelope(raw, None, rev)
                    })
                    .collect();
                return Ok(json!({"kind": "skill", "items": items, "skills": items}));
            }
            let items: Vec<Value> = persisted
                .iter()
                .map(|raw| {
                    let rev = raw.get("revision").and_then(|v| v.as_u64()).unwrap_or(1);
                    let env = canonical_skill_editor_envelope(raw, None, rev);
                    env.get("record").cloned().unwrap_or(env)
                })
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
                if rec.get("deleted_at_ms").is_some_and(|v| !v.is_null()) {
                    return Ok(Value::Null);
                }
                let rev = rec.get("revision").and_then(|v| v.as_u64()).unwrap_or(1);
                return Ok(canonical_skill_editor_envelope(&rec, Some(id), rev));
            }
            Ok(Value::Null)
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
            let is_tombstoned = prev
                .as_ref()
                .and_then(|v| v.get("deleted_at_ms"))
                .is_some_and(|v| !v.is_null());
            let prev_rev = prev
                .as_ref()
                .and_then(|v| v.get("revision").and_then(|r| r.as_u64()))
                .unwrap_or(0);
            if op == "create_user" {
                if prev.is_some() {
                    return Err((
                        "conflict",
                        format!("skill '{id}' already exists or was previously deleted"),
                    ));
                }
            } else if op == "configure_user" {
                if prev.is_none() || is_tombstoned {
                    return Err((
                        "skill_deleted",
                        format!("skill '{id}' not found or was deleted"),
                    ));
                }
                if let Some(exp) = payload
                    .get("expected_revision")
                    .or_else(|| input.get("expected_revision"))
                    .or_else(|| args.get("__envelope_expected_revision"))
                    .and_then(|v| v.as_u64())
                {
                    if exp != prev_rev {
                        return Err((
                            "skill_revision_conflict",
                            format!(
                                "skill_revision_conflict: skill '{id}' expected revision {exp}, actual {prev_rev}"
                            ),
                        ));
                    }
                }
            } else if is_tombstoned {
                return Err((
                    "conflict",
                    format!("skill '{id}' was deleted and cannot be resurrected"),
                ));
            }
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
            let mut editors = Vec::new();
            if let Some(inputs) = payload.get("inputs").and_then(|v| v.as_array()) {
                // Validate entire batch before persisting any item (atomic rollback on invalid package).
                let mut seen_ids: HashSet<String> = HashSet::new();
                for item in inputs {
                    let id = item
                        .get("id")
                        .or_else(|| item.get("name"))
                        .and_then(|v| v.as_str())
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .ok_or(("invalid_argument", "built-in skill input missing non-empty id".into()))?;
                    if !seen_ids.insert(id.to_string()) {
                        return Err(("invalid_argument", format!("duplicate built-in skill id '{id}'")));
                    }
                    let display_name = item
                        .get("display_name")
                        .or_else(|| item.get("name"))
                        .and_then(|v| v.as_str())
                        .map(str::trim)
                        .unwrap_or("");
                    if display_name.is_empty() {
                        return Err(("invalid_argument", format!("built-in skill '{id}' missing display_name")));
                    }
                    if let Some(inst) = item
                        .get("instructions_text")
                        .or_else(|| item.get("instructions"))
                        .and_then(|v| v.as_str())
                    {
                        if inst.trim().is_empty() {
                            return Err(("invalid_argument", format!("built-in skill '{id}' has empty instructions")));
                        }
                    }
                }
                for item in inputs {
                    let id = item
                        .get("id")
                        .or_else(|| item.get("name"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("skill")
                        .trim()
                        .to_string();
                    let prev = suite.get_record::<Value>("skill", &id).ok().flatten();
                    let prev_rev = prev
                        .as_ref()
                        .and_then(|v| v.get("revision").and_then(|r| r.as_u64()))
                        .unwrap_or(0);
                    let mut merged = item.clone();
                    if let (Some(prev_obj), Some(obj)) = (prev.as_ref().and_then(|v| v.as_object()), merged.as_object_mut()) {
                        if let Some(en) = prev_obj.get("enabled") {
                            obj.insert("enabled".into(), en.clone());
                        }
                        if let Some(created) = prev_obj.get("created_at_ms") {
                            obj.insert("created_at_ms".into(), created.clone());
                        }
                    }
                    let cand_env = canonical_skill_editor_envelope(&merged, Some(&id), prev_rev.max(1));
                    let cand_rec = cand_env.get("record").cloned().unwrap_or_else(|| merged.clone());
                    let changed = match &prev {
                        None => true,
                        Some(p) => {
                            let prev_canon = canonical_skill_editor_envelope(p, Some(&id), prev_rev.max(1));
                            let pr = prev_canon.get("record").unwrap_or(p);
                            pr.get("display_name") != cand_rec.get("display_name")
                                || pr.get("summary") != cand_rec.get("summary")
                                || pr.get("category") != cand_rec.get("category")
                                || pr.get("instructions_text") != cand_rec.get("instructions_text")
                                || pr.get("instructions_ref") != cand_rec.get("instructions_ref")
                                || pr.get("required_tools") != cand_rec.get("required_tools")
                                || pr.get("default_mode") != cand_rec.get("default_mode")
                                || pr.get("version") != cand_rec.get("version")
                        }
                    };
                    let next_rev = if prev_rev == 0 {
                        1
                    } else if changed {
                        prev_rev + 1
                    } else {
                        prev_rev
                    };
                    if !changed {
                        if let Some(ref p) = prev {
                            editors.push(canonical_skill_editor_envelope(p, Some(&id), next_rev));
                            continue;
                        }
                    }
                    let env = canonical_skill_editor_envelope(&merged, Some(&id), next_rev);
                    let mut to_store = env.get("record").cloned().unwrap_or_else(|| merged.clone());
                    if let Some(obj) = to_store.as_object_mut() {
                        obj.insert("revision".into(), json!(next_rev));
                    }
                    suite
                        .upsert_record("skill", &id, &to_store)
                        .map_err(|e| ("storage_error", e.to_string()))?;
                    editors.push(env);
                }
            } else {
                for b in nola_modules::skills::builtin_skills() {
                    let val = serde_json::to_value(&b).unwrap_or_else(|_| json!({}));
                    editors.push(canonical_skill_editor_envelope(&val, Some(&b.name), 1));
                }
            }
            Ok(Value::Array(editors))
        }
        ("skill", "restore_built_ins") => {
            let input = payload.get("input").unwrap_or(payload);
            let ts = payload
                .get("updated_at_ms")
                .or_else(|| input.get("updated_at_ms"))
                .and_then(|v| v.as_i64())
                .unwrap_or_else(now_ms);
            // 1. Always validate expected_visibility_revision against product_setting:skill.builtin.hidden
            // BEFORE mutating anything (omitted field means Swift passed nil => expects no hidden setting).
            {
                let exp_vis = payload
                    .get("expected_visibility_revision")
                    .or_else(|| input.get("expected_visibility_revision"))
                    .and_then(|v| v.as_u64());
                let cur_vis = load_product_setting_entry(store, "skill.builtin.hidden")?
                    .map(|(_, _, rev, _)| rev);
                if exp_vis != cur_vis {
                    return Err((
                        "revision_conflict",
                        json!({
                            "scope_id": "product_setting:skill.builtin.hidden",
                            "expected_revision": exp_vis.unwrap_or(0),
                            "actual_revision": cur_vis.unwrap_or(0)
                        })
                        .to_string(),
                    ));
                }
            }
            // 2. Validate expected_revisions on all named skills BEFORE mutating anything.
            let expected_revs = payload
                .get("expected_revisions")
                .or_else(|| payload.get("expectedRevisions"))
                .or_else(|| input.get("expected_revisions"))
                .and_then(|v| v.as_object());
            if let Some(exp_map) = expected_revs {
                for (sid, exp_val) in exp_map {
                    let exp = exp_val.as_u64().unwrap_or(0);
                    let actual = suite
                        .get_record::<Value>("skill", sid)
                        .ok()
                        .flatten()
                        .and_then(|r| r.get("revision").and_then(|v| v.as_u64()))
                        .unwrap_or(1);
                    if exp != actual {
                        return Err((
                            "revision_conflict",
                            json!({
                                "scope_id": format!("skill:{sid}"),
                                "expected_revision": exp,
                                "actual_revision": actual
                            })
                            .to_string(),
                        ));
                    }
                }
            }
            // 3. Clear product_setting:skill.builtin.hidden (tombstone so get_product_setting returns null).
            let prev_vis_rev = load_product_setting_entry(store, "skill.builtin.hidden")?
                .map(|(_, _, r, _)| r)
                .unwrap_or(0);
            let _ = store.set_kv(
                "product_setting:skill.builtin.hidden",
                &json!({
                    "__nola_setting": true,
                    "deleted": true,
                    "revision": prev_vis_rev + 1,
                    "updated_at_ms": ts
                }),
            );
            let mut editors = Vec::new();
            let builtins = nola_modules::skills::builtin_skills();
            for b in &builtins {
                let _ = nola_modules::skills::set_skill_enabled(store, &b.name, true);
                let _ = store.set_kv(&format!("skill_builtin_removed:{}", b.name), &json!(false));
            }
            for rec in suite.list_records::<Value>("skill").unwrap_or_default() {
                if rec.get("source").and_then(|v| v.as_str()) == Some("built_in") {
                    let id = rec
                        .get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("skill")
                        .to_string();
                    let cur_rev = rec.get("revision").and_then(|v| v.as_u64()).unwrap_or(1);
                    let was_enabled = rec.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true);
                    let next_rev = if was_enabled { cur_rev } else { cur_rev + 1 };
                    let mut updated = rec.clone();
                    if let Some(obj) = updated.as_object_mut() {
                        obj.insert("enabled".into(), json!(true));
                        obj.insert("deleted_at_ms".into(), Value::Null);
                        obj.insert("revision".into(), json!(next_rev));
                        if !was_enabled {
                            obj.insert("updated_at_ms".into(), json!(ts));
                        }
                    }
                    let _ = suite.upsert_record("skill", &id, &updated);
                    editors.push(canonical_skill_editor_envelope(&updated, Some(&id), next_rev));
                }
            }
            Ok(Value::Array(editors))
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
            let ts = payload
                .get("updated_at_ms")
                .or_else(|| input.get("updated_at_ms"))
                .and_then(|v| v.as_i64())
                .unwrap_or_else(now_ms);
            let mut rec = suite
                .get_record::<Value>("skill", id)
                .map_err(|e| ("storage_error", e.to_string()))?
                .ok_or(("not_found", format!("built-in skill '{id}' not found")))?;
            let cur_rev = rec.get("revision").and_then(|r| r.as_u64()).unwrap_or(1);
            if let Some(exp) = payload
                .get("expected_revision")
                .or_else(|| input.get("expected_revision"))
                .and_then(|v| v.as_u64())
            {
                if exp != cur_rev {
                    return Err((
                        "revision_conflict",
                        json!({
                            "scope_id": format!("skill_revision_conflict:skill:{id}"),
                            "expected_revision": exp,
                            "actual_revision": cur_rev
                        })
                        .to_string(),
                    ));
                }
            }
            let cur_vis = load_product_setting_entry(store, "skill.builtin.hidden")?;
            let cur_vis_rev = cur_vis.as_ref().map(|(_, _, r, _)| *r);
            let has_vis_field = payload.get("expected_visibility_revision").is_some()
                || input.get("expected_visibility_revision").is_some();
            if has_vis_field {
                let exp_vis = payload
                    .get("expected_visibility_revision")
                    .or_else(|| input.get("expected_visibility_revision"))
                    .and_then(|v| v.as_u64());
                if exp_vis != cur_vis_rev {
                    return Err((
                        "revision_conflict",
                        json!({
                            "scope_id": "product_setting:skill.builtin.hidden",
                            "expected_revision": exp_vis.unwrap_or(0),
                            "actual_revision": cur_vis_rev.unwrap_or(0)
                        })
                        .to_string(),
                    ));
                }
            }
            // Add `id` to product_setting:skill.builtin.hidden JSON {"schemaVersion":1,"ids":[...]}.
            let mut hidden_ids: Vec<String> = cur_vis
                .as_ref()
                .and_then(|(v, _, _, _)| {
                    let parsed = if let Some(s) = v.as_str() {
                        serde_json::from_str::<Value>(s).ok()
                    } else {
                        Some(v.clone())
                    };
                    parsed
                        .and_then(|p| p.get("ids").and_then(|a| a.as_array()).cloned())
                        .map(|arr| {
                            arr.into_iter()
                                .filter_map(|x| x.as_str().map(ToString::to_string))
                                .collect()
                        })
                })
                .unwrap_or_default();
            if !hidden_ids.iter().any(|x| x == id) {
                hidden_ids.push(id.to_string());
                hidden_ids.sort();
            }
            let hidden_val = json!({"schemaVersion": 1, "ids": hidden_ids});
            let hidden_json = serde_json::to_string(&hidden_val).unwrap_or_else(|_| "{}".into());
            let next_vis_rev = cur_vis_rev.unwrap_or(0) + 1;
            let _ = store.set_kv(
                "product_setting:skill.builtin.hidden",
                &json!({
                    "__nola_setting": true,
                    "deleted": false,
                    "value": hidden_val,
                    "value_json": hidden_json,
                    "revision": next_vis_rev,
                    "updated_at_ms": ts
                }),
            );
            let _ = nola_modules::skills::set_skill_enabled(store, id, false);
            let _ = store.set_kv(&format!("skill_builtin_removed:{id}"), &json!(true));
            let next_rev = cur_rev + 1;
            if let Some(obj) = rec.as_object_mut() {
                obj.insert("enabled".into(), json!(false));
                obj.insert("revision".into(), json!(next_rev));
                obj.insert("updated_at_ms".into(), json!(ts));
            }
            let env = canonical_skill_editor_envelope(&rec, Some(id), next_rev);
            let mut to_store = env.get("record").cloned().unwrap_or_else(|| rec.clone());
            if let Some(obj) = to_store.as_object_mut() {
                obj.insert("revision".into(), json!(next_rev));
            }
            suite
                .upsert_record("skill", id, &to_store)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(env)
        }
        ("skill", "apply_changes") => {
            let input = payload.get("input").unwrap_or(payload);
            if let Some(commands) = payload
                .get("commands")
                .or_else(|| input.get("commands"))
                .and_then(|v| v.as_array())
            {
                // Two-phase validation so any revision conflict or invalid item rolls back the full batch.
                let mut staged: Vec<(String, Value, Value, bool)> = Vec::with_capacity(commands.len());
                for cmd in commands {
                    let id = cmd
                        .get("id")
                        .or_else(|| cmd.get("name"))
                        .and_then(|v| v.as_str())
                        .ok_or(("invalid_argument", "skill batch command missing 'id'".into()))?;
                    let enabled = cmd
                        .get("enabled")
                        .and_then(|v| v.as_bool())
                        .ok_or(("invalid_argument", "skill batch command missing 'enabled'".into()))?;
                    let base = suite
                        .get_record::<Value>("skill", id)
                        .map_err(|e| ("storage_error", e.to_string()))?
                        .ok_or(("not_found", format!("skill '{id}' not found")))?;
                    let cur_rev = base.get("revision").and_then(|r| r.as_u64()).unwrap_or(1);
                    if let Some(exp) = cmd
                        .get("expected_revision")
                        .or_else(|| cmd.get("expectedRevision"))
                        .and_then(|v| v.as_u64())
                    {
                        if exp != cur_rev {
                            return Err((
                                "revision_conflict",
                                json!({
                                    "scope_id": format!("skill:{id}"),
                                    "expected_revision": exp,
                                    "actual_revision": cur_rev
                                })
                                .to_string(),
                            ));
                        }
                    }
                    let already_enabled = base.get("enabled").and_then(|v| v.as_bool()).unwrap_or(false);
                    if already_enabled == enabled {
                        let env = canonical_skill_editor_envelope(&base, Some(id), cur_rev);
                        staged.push((id.to_string(), base, env, false));
                        continue;
                    }
                    let next_rev = cur_rev + 1;
                    let ts = cmd
                        .get("updated_at_ms")
                        .or_else(|| cmd.get("updatedAtMs"))
                        .and_then(|v| v.as_i64())
                        .unwrap_or_else(now_ms);
                    let mut updated = base;
                    if let Some(obj) = updated.as_object_mut() {
                        obj.insert("id".into(), json!(id));
                        obj.insert("enabled".into(), json!(enabled));
                        obj.insert("revision".into(), json!(next_rev));
                        obj.insert("updated_at_ms".into(), json!(ts));
                    }
                    let env = canonical_skill_editor_envelope(&updated, Some(id), next_rev);
                    let mut to_store = env.get("record").cloned().unwrap_or(updated);
                    if let Some(obj) = to_store.as_object_mut() {
                        obj.insert("revision".into(), json!(next_rev));
                    }
                    staged.push((id.to_string(), to_store, env, true));
                }
                let mut editors = Vec::with_capacity(staged.len());
                for (id, to_store, env, mutated) in staged {
                    if mutated {
                        if let Some(en) = to_store.get("enabled").and_then(|v| v.as_bool()) {
                            let _ = nola_modules::skills::set_skill_enabled(store, &id, en);
                        }
                        suite
                            .upsert_record("skill", &id, &to_store)
                            .map_err(|e| ("storage_error", e.to_string()))?;
                    }
                    editors.push(env);
                }
                return Ok(Value::Array(editors));
            }
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
            let base = suite
                .get_record::<Value>("skill", id)
                .map_err(|e| ("storage_error", e.to_string()))?
                .filter(|r| r.get("deleted_at_ms").is_none_or(|v| v.is_null()))
                .ok_or(("not_found", format!("skill '{id}' not found or was deleted")))?;
            let cur_rev = base.get("revision").and_then(|r| r.as_u64()).unwrap_or(1);
            if let Some(exp) = input
                .get("expected_revision")
                .or_else(|| input.get("expectedRevision"))
                .or_else(|| payload.get("expected_revision"))
                .or_else(|| payload.get("expectedRevision"))
                .and_then(|v| v.as_u64())
            {
                if exp != cur_rev {
                    return Err((
                        "revision_conflict",
                        json!({
                            "scope_id": format!("skill_revision_conflict:skill:{id}"),
                            "expected_revision": exp,
                            "actual_revision": cur_rev
                        })
                        .to_string(),
                    ));
                }
            }
            if base.get("enabled").and_then(|v| v.as_bool()) == Some(enabled) {
                return Ok(canonical_skill_editor_envelope(&base, Some(id), cur_rev));
            }
            nola_modules::skills::set_skill_enabled(store, id, enabled)
                .map_err(|e| ("storage_error", e.to_string()))?;
            let next_rev = cur_rev + 1;
            let ts = input
                .get("updated_at_ms")
                .or_else(|| input.get("updatedAtMs"))
                .or_else(|| payload.get("updated_at_ms"))
                .or_else(|| payload.get("updatedAtMs"))
                .and_then(|v| v.as_i64())
                .unwrap_or_else(now_ms);
            let mut updated = base;
            if let Some(obj) = updated.as_object_mut() {
                obj.insert("id".into(), json!(id));
                obj.insert("enabled".into(), json!(enabled));
                obj.insert("revision".into(), json!(next_rev));
                obj.insert("updated_at_ms".into(), json!(ts));
            }
            let envelope = canonical_skill_editor_envelope(&updated, Some(id), next_rev);
            let canonical_rec = envelope.get("record").cloned().unwrap_or(updated.clone());
            let mut to_store = canonical_rec.clone();
            if let Some(obj) = to_store.as_object_mut() {
                obj.insert("revision".into(), json!(next_rev));
            }
            suite
                .upsert_record("skill", id, &to_store)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(envelope)
        }
        ("skill", "delete_user_checked" | "delete_user" | "delete") => {
            let input = payload.get("input").unwrap_or(payload);
            let id = input
                .get("id")
                .or_else(|| payload.get("id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            let existing = suite
                .get_record::<Value>("skill", id)
                .map_err(|e| ("storage_error", e.to_string()))?
                .filter(|r| r.get("deleted_at_ms").is_none_or(|v| v.is_null()))
                .ok_or(("not_found", format!("skill '{id}' not found")))?;
            let cur_rev = existing.get("revision").and_then(|r| r.as_u64()).unwrap_or(1);
            if let Some(exp) = input
                .get("expected_revision")
                .or_else(|| input.get("expectedRevision"))
                .or_else(|| payload.get("expected_revision"))
                .or_else(|| payload.get("expectedRevision"))
                .and_then(|v| v.as_u64())
            {
                if exp != cur_rev {
                    return Err((
                        "revision_conflict",
                        json!({
                            "scope_id": format!("skill_revision_conflict:skill:{id}"),
                            "expected_revision": exp,
                            "actual_revision": cur_rev
                        })
                        .to_string(),
                    ));
                }
            }
            let ts = input
                .get("deleted_at_ms")
                .or_else(|| input.get("deletedAtMs"))
                .or_else(|| payload.get("deleted_at_ms"))
                .and_then(|v| v.as_i64())
                .unwrap_or_else(now_ms);
            let mut tomb = existing;
            if let Some(obj) = tomb.as_object_mut() {
                obj.insert("enabled".into(), json!(false));
                obj.insert("revision".into(), json!(cur_rev + 1));
                obj.insert("deleted_at_ms".into(), json!(ts));
                obj.insert("updated_at_ms".into(), json!(ts));
            }
            suite
                .upsert_record("skill", id, &tomb)
                .map_err(|e| ("storage_error", e.to_string()))?;
            let _ = nola_modules::skills::set_skill_enabled(store, id, false);
            Ok(json!(true))
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
            let records: Vec<Value> = items
                .iter()
                .map(|m| canonical_memory_envelope_with_meta(store, m))
                .collect();
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
                .map(|m| canonical_memory_envelope_with_meta(store, &m));
            Ok(found.unwrap_or(Value::Null))
        }
        ("memory", "commit" | "save" | "upsert" | "update_if_revision") => {
            let item = parse_memory_item(payload);
            store
                .upsert_memory(&item)
                .map_err(|e| ("storage_error", e.to_string()))?;
            save_memory_extra_meta(store, &item.id, payload);
            Ok(canonical_memory_envelope_with_meta(store, &item))
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
                let parsed_items: Vec<(MemoryItem, &Value)> =
                    arr.iter().map(|v| (parse_memory_item(v), v)).collect();
                let incoming_ids: HashSet<&str> =
                    parsed_items.iter().map(|(m, _)| m.id.as_str()).collect();
                let existing = store
                    .search_memory("", None, 10_000)
                    .map_err(|e| ("storage_error", e.to_string()))?;
                if let Some(expected_arr) = input
                    .get("expected")
                    .or_else(|| payload.get("expected"))
                    .and_then(|v| v.as_array())
                {
                    let mut actual_map: BTreeMap<String, (String, i64)> = BTreeMap::new();
                    for item in &existing {
                        let env = canonical_memory_envelope_with_meta(store, item);
                        let id = env
                            .pointer("/record/id")
                            .and_then(|v| v.as_str())
                            .unwrap_or(&item.id)
                            .to_string();
                        let content = env
                            .pointer("/record/content")
                            .and_then(|v| v.as_str())
                            .unwrap_or(&item.content)
                            .to_string();
                        let updated = env
                            .get("updated_at_ms")
                            .and_then(|v| v.as_i64())
                            .unwrap_or(item.updated_at_ms);
                        actual_map.insert(id, (content, updated));
                    }
                    let mut exp_map: BTreeMap<String, (String, i64)> = BTreeMap::new();
                    for ev in expected_arr {
                        let rec = ev.get("record").unwrap_or(ev);
                        let id = rec
                            .get("id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let content = rec
                            .get("content")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let updated = ev
                            .get("updated_at_ms")
                            .or_else(|| rec.get("updated_at_ms"))
                            .and_then(|v| v.as_i64())
                            .unwrap_or(0);
                        exp_map.insert(id, (content, updated));
                    }
                    if actual_map != exp_map {
                        return Err((
                            "memory_snapshot_conflict",
                            "memory_snapshot_conflict: memory snapshot changed since editor loaded"
                                .into(),
                        ));
                    }
                }
                for old in existing {
                    if !incoming_ids.contains(old.id.as_str()) {
                        store
                            .delete_memory(&old.id)
                            .map_err(|e| ("storage_error", e.to_string()))?;
                        let _ = store.set_kv(&format!("mem_meta:{}", old.id), &Value::Null);
                    }
                }
                for (item, raw) in &parsed_items {
                    store
                        .upsert_memory(item)
                        .map_err(|e| ("storage_error", e.to_string()))?;
                    save_memory_extra_meta(store, &item.id, raw);
                }
                // Read back the persisted SQLite rows so returned created_at_ms/updated_at_ms
                // match subsequent memory.list snapshots bit-for-bit.
                let persisted = store
                    .search_memory("", None, 10_000)
                    .map_err(|e| ("storage_error", e.to_string()))?;
                let records: Vec<Value> = persisted
                    .iter()
                    .map(|m| canonical_memory_envelope_with_meta(store, m))
                    .collect();
                return Ok(Value::Array(records));
            }
            let item = parse_memory_item(input);
            store
                .upsert_memory(&item)
                .map_err(|e| ("storage_error", e.to_string()))?;
            save_memory_extra_meta(store, &item.id, input);
            Ok(Value::Array(vec![canonical_memory_envelope_with_meta(store, &item)]))
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
        ("mcp", "create" | "configure" | "update" | "update_checked" | "upsert") => {
            let input = payload.get("input").unwrap_or(payload);
            let cfg = parse_mcp_server_config(payload)?;
            let existing = store
                .list_mcp_servers()
                .map_err(|e| ("storage_error", e.to_string()))?
                .into_iter()
                .find(|s| s.id == cfg.id);
            let prev_meta = store
                .get_kv(&format!("mcp_meta:{}", cfg.id))
                .ok()
                .flatten();
            let is_tombstoned = prev_meta
                .as_ref()
                .and_then(|m| m.get("deleted_at_ms"))
                .is_some_and(|v| !v.is_null());
            let cur_rev = prev_meta
                .as_ref()
                .and_then(|m| m.get("revision"))
                .and_then(|r| r.as_u64())
                .unwrap_or(if existing.is_some() { 1 } else { 0 });
            if op == "create" {
                if existing.is_some() || is_tombstoned {
                    return Err((
                        "mcp_identity_conflict",
                        format!(
                            "mcp_identity_conflict: mcp server '{}' already exists or was previously deleted",
                            cfg.id
                        ),
                    ));
                }
            } else if matches!(op, "configure" | "update" | "update_checked") {
                if existing.is_none() || is_tombstoned {
                    return Err((
                        "not_found",
                        format!("mcp server '{}' is missing or deleted", cfg.id),
                    ));
                }
                if let Some(exp) = payload
                    .get("expected_revision")
                    .or_else(|| input.get("expected_revision"))
                    .or_else(|| args.get("__envelope_expected_revision"))
                    .and_then(|v| v.as_u64())
                {
                    if exp != cur_rev {
                        return Err((
                            "mcp_revision_conflict",
                            format!(
                                "mcp_revision_conflict: mcp server '{}' expected revision {}, actual {}",
                                cfg.id, exp, cur_rev
                            ),
                        ));
                    }
                }
            } else if is_tombstoned {
                return Err((
                    "mcp_identity_conflict",
                    format!(
                        "mcp_identity_conflict: mcp server '{}' was deleted and cannot be resurrected",
                        cfg.id
                    ),
                ));
            }
            store
                .upsert_mcp_server(&cfg)
                .map_err(|e| ("storage_error", e.to_string()))?;
            let prev_meta = prev_meta.unwrap_or_else(|| json!({}));
            let next_rev = cur_rev + 1;
            let ts = now_ms();
            let created_at_ms = prev_meta
                .get("created_at_ms")
                .and_then(|v| v.as_i64())
                .or_else(|| input.get("created_at_ms").and_then(|v| v.as_i64()))
                .unwrap_or(ts);
            let updated_at_ms = input
                .get("updated_at_ms")
                .or_else(|| payload.get("updated_at_ms"))
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
                    "updated_at_ms": updated_at_ms,
                    "deleted_at_ms": Value::Null
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
                "__committed_revision": next_rev,
                "created_at_ms": created_at_ms,
                "updated_at_ms": updated_at_ms,
                "deleted_at_ms": Value::Null,
                "server": enriched
            }))
        }
        ("mcp", "delete_checked" | "delete") => {
            let input = payload.get("input").unwrap_or(payload);
            let id = payload
                .get("id")
                .or_else(|| payload.get("server_id"))
                .or_else(|| input.get("id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            let existing = store
                .list_mcp_servers()
                .map_err(|e| ("storage_error", e.to_string()))?
                .into_iter()
                .find(|s| s.id == id);
            let prev_meta = store.get_kv(&format!("mcp_meta:{id}")).ok().flatten();
            let is_tombstoned = prev_meta
                .as_ref()
                .and_then(|m| m.get("deleted_at_ms"))
                .is_some_and(|v| !v.is_null());
            if existing.is_none() || is_tombstoned {
                if op == "delete_checked" {
                    return Err(("not_found", format!("mcp server '{id}' not found")));
                }
                return Ok(json!(false));
            }
            let cur_rev = prev_meta
                .as_ref()
                .and_then(|m| m.get("revision"))
                .and_then(|r| r.as_u64())
                .unwrap_or(1);
            if let Some(exp) = payload
                .get("expected_revision")
                .or_else(|| input.get("expected_revision"))
                .or_else(|| args.get("__envelope_expected_revision"))
                .and_then(|v| v.as_u64())
            {
                if exp != cur_rev {
                    return Err((
                        "mcp_revision_conflict",
                        format!(
                            "mcp_revision_conflict: mcp server '{id}' expected revision {exp}, actual {cur_rev}"
                        ),
                    ));
                }
            }
            let deleted = store
                .delete_mcp_server(id)
                .map_err(|e| ("storage_error", e.to_string()))?;
            let ts = payload
                .get("deleted_at_ms")
                .or_else(|| input.get("deleted_at_ms"))
                .and_then(|v| v.as_i64())
                .unwrap_or_else(now_ms);
            let mut tomb = prev_meta.unwrap_or_else(|| json!({}));
            if let Some(obj) = tomb.as_object_mut() {
                obj.insert("revision".into(), json!(cur_rev + 1));
                obj.insert("deleted_at_ms".into(), json!(ts));
                obj.insert("updated_at_ms".into(), json!(ts));
            }
            let _ = store.set_kv(&format!("mcp_meta:{id}"), &tomb);
            Ok(json!(deleted))
        }

        // --- DataSource Domain ---
        ("data_source", "list") => {
            let items: Vec<Value> = suite
                .list_records::<Value>("data_source")
                .map_err(|e| ("storage_error", e.to_string()))?
                .into_iter()
                .filter(|r| r.get("deleted_at_ms").is_none_or(|v| v.is_null()))
                .map(|r| canonicalize_data_source_record(r, None))
                .collect();
            Ok(json!({"kind": "data_source", "items": items}))
        }
        ("data_source", "list_usage") => {
            let limit = payload
                .get("limit")
                .or_else(|| args.get("limit"))
                .and_then(|v| v.as_u64())
                .unwrap_or(200) as usize;
            let source_filter = payload
                .get("source_id")
                .or_else(|| args.get("source_id"))
                .and_then(|v| v.as_str());
            let mut items: Vec<Value> = suite
                .list_records("data_source_usage")
                .map_err(|e| ("storage_error", e.to_string()))?;
            if let Some(sf) = source_filter {
                items.retain(|item| item.get("source_id").and_then(|v| v.as_str()) == Some(sf));
            }
            items.sort_by(|a, b| {
                let ta = a.get("used_at_ms").and_then(|v| v.as_i64()).unwrap_or(0);
                let tb = b.get("used_at_ms").and_then(|v| v.as_i64()).unwrap_or(0);
                tb.cmp(&ta)
            });
            items.truncate(limit);
            Ok(json!({"kind": "data_source_usage", "items": items, "usage": items}))
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
                .map_err(|e| ("storage_error", e.to_string()))?
                .filter(|r| r.get("deleted_at_ms").is_none_or(|v| v.is_null()))
                .map(|r| canonicalize_data_source_record(r, Some(id)));
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
            if op == "create_draft" {
                if let Some(ex) = suite
                    .get_record::<Value>("data_source", &id)
                    .map_err(|e| ("storage_error", e.to_string()))?
                {
                    if ex.get("deleted_at_ms").is_none_or(|v| v.is_null()) {
                        return Err((
                            "conflict",
                            format!("data source '{id}' already exists"),
                        ));
                    }
                }
            }
            let ts = record
                .get("updated_at_ms")
                .and_then(|v| v.as_i64())
                .unwrap_or_else(now_ms);
            let created_ts = record
                .get("created_at_ms")
                .and_then(|v| v.as_i64())
                .unwrap_or(ts);
            let mut stored = canonicalize_data_source_record(record.clone(), Some(&id));
            if let Some(obj) = stored.as_object_mut() {
                obj.insert("id".into(), json!(id));
                obj.insert("revision".into(), json!(1i64));
                obj.insert("lifecycle".into(), json!("draft"));
                obj.insert("default_enabled".into(), json!(false));
                obj.insert("created_at_ms".into(), json!(created_ts));
                obj.insert("updated_at_ms".into(), json!(ts));
                obj.insert("deleted_at_ms".into(), Value::Null);
            }
            suite
                .upsert_record("data_source", &id, &stored)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(stored)
        }
        ("data_source", "update_configuration" | "update") => {
            let input = payload.get("input").unwrap_or(payload);
            let record = input
                .get("configuration")
                .filter(|c| c.get("name").is_some())
                .or_else(|| input.get("record"))
                .unwrap_or(input);
            let id = payload
                .get("id")
                .or_else(|| payload.get("data_source_id"))
                .or_else(|| input.get("id"))
                .or_else(|| input.get("data_source_id"))
                .or_else(|| record.get("id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            let existing = suite
                .get_record::<Value>("data_source", id)
                .map_err(|e| ("storage_error", e.to_string()))?
                .filter(|r| r.get("deleted_at_ms").is_none_or(|v| v.is_null()))
                .ok_or(("not_found", format!("data source '{id}' not found")))?;
            let cur_rev = existing
                .get("revision")
                .and_then(|v| v.as_i64())
                .unwrap_or(1);
            if let Some(exp) = payload
                .get("expected_revision")
                .or_else(|| input.get("expected_revision"))
                .or_else(|| args.get("__envelope_expected_revision"))
                .and_then(|v| v.as_i64())
            {
                if exp != cur_rev {
                    return Err((
                        "revision_conflict",
                        json!({
                            "scope_id": format!("data_source:{id}"),
                            "expected_revision": exp.max(0) as u64,
                            "actual_revision": cur_rev.max(0) as u64
                        })
                        .to_string(),
                    ));
                }
            }
            let mut merged = canonicalize_data_source_record(existing, Some(id));
            let next_rev = cur_rev + 1;
            let ts = record
                .get("updated_at_ms")
                .or_else(|| input.get("updated_at_ms"))
                .and_then(|v| v.as_i64())
                .unwrap_or_else(now_ms);
            if let (Some(dst), Some(src)) = (merged.as_object_mut(), record.as_object()) {
                for (k, v) in src {
                    if !matches!(k.as_str(), "type" | "expected_revision") {
                        dst.insert(k.clone(), v.clone());
                    }
                }
                dst.insert("id".into(), json!(id));
                dst.insert("revision".into(), json!(next_rev));
                dst.insert("updated_at_ms".into(), json!(ts));
                // Invalidate any in-flight read_token when configuration changes.
                if let Some(rs) = dst.get_mut("read_state").and_then(|v| v.as_object_mut()) {
                    rs.insert("read_token".into(), Value::Null);
                    if rs.get("status").and_then(|v| v.as_str()) == Some("pending") {
                        rs.insert("status".into(), json!("never_read"));
                    }
                }
            }
            let merged = canonicalize_data_source_record(merged, Some(id));
            suite
                .upsert_record("data_source", id, &merged)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(merged)
        }
        ("data_source", "request_validation") => {
            let input = payload.get("input").unwrap_or(payload);
            let id = payload
                .get("id")
                .or_else(|| payload.get("data_source_id"))
                .or_else(|| input.get("data_source_id"))
                .or_else(|| input.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            let existing = suite
                .get_record::<Value>("data_source", id)
                .map_err(|e| ("storage_error", e.to_string()))?
                .filter(|r| r.get("deleted_at_ms").is_none_or(|v| v.is_null()))
                .ok_or(("not_found", format!("data source '{id}' not found")))?;
            let cur_rev = existing
                .get("revision")
                .and_then(|v| v.as_i64())
                .unwrap_or(1);
            if let Some(exp) = payload
                .get("expected_revision")
                .or_else(|| input.get("expected_revision"))
                .and_then(|v| v.as_i64())
            {
                if exp != cur_rev {
                    return Err((
                        "revision_conflict",
                        json!({
                            "scope_id": format!("data_source:{id}"),
                            "expected_revision": exp.max(0) as u64,
                            "actual_revision": cur_rev.max(0) as u64
                        })
                        .to_string(),
                    ));
                }
            }
            let ts = payload
                .get("requested_at_ms")
                .or_else(|| input.get("requested_at_ms"))
                .and_then(|v| v.as_i64())
                .unwrap_or_else(now_ms);
            let token = new_id();
            let validation = json!({
                "status": "pending",
                "validation_token": token,
                "contract": Value::Null,
                "diagnostic": Value::Null,
                "attempted_at_ms": ts,
                "completed_at_ms": Value::Null
            });
            let _ = store.set_kv(&format!("ds_validation:{id}"), &validation);
            let mut rec = canonicalize_data_source_record(existing, Some(id));
            let next_rev = cur_rev + 1;
            if let Some(obj) = rec.as_object_mut() {
                obj.insert("validation".into(), validation);
                obj.insert("revision".into(), json!(next_rev));
                obj.insert("updated_at_ms".into(), json!(ts));
            }
            suite
                .upsert_record("data_source", id, &rec)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(rec)
        }
        ("data_source", "record_validation") => {
            let input = payload.get("input").unwrap_or(payload);
            let id = payload
                .get("id")
                .or_else(|| payload.get("data_source_id"))
                .or_else(|| input.get("data_source_id"))
                .or_else(|| input.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            let existing = suite
                .get_record::<Value>("data_source", id)
                .map_err(|e| ("storage_error", e.to_string()))?
                .filter(|r| r.get("deleted_at_ms").is_none_or(|v| v.is_null()))
                .ok_or(("not_found", format!("data source '{id}' not found")))?;
            let mut rec = canonicalize_data_source_record(existing, Some(id));
            let expected_token = rec
                .get("validation")
                .and_then(|v| v.get("validation_token"))
                .and_then(|v| v.as_str());
            if let Some(provided_token) = payload
                .get("validation_token")
                .or_else(|| input.get("validation_token"))
                .and_then(|v| v.as_str())
            {
                if expected_token != Some(provided_token) {
                    return Err((
                        "conflict",
                        "data source validation token does not match pending validation".into(),
                    ));
                }
            }
            let ts = payload
                .get("recorded_at_ms")
                .or_else(|| input.get("recorded_at_ms"))
                .and_then(|v| v.as_i64())
                .unwrap_or_else(now_ms);
            let res_obj = payload
                .get("result")
                .or_else(|| input.get("result"))
                .unwrap_or(input);
            let raw_status = res_obj
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("succeeded");
            let (norm_status, lifecycle) = match raw_status {
                "succeeded" | "verified" => ("verified", "ready"),
                "succeeded_empty" | "verified_empty" => ("verified_empty", "ready"),
                "failed" => ("failed", "failed"),
                "pending" => ("pending", "draft"),
                _ => ("verified", "ready"),
            };
            let attempted_at = rec
                .get("validation")
                .and_then(|v| v.get("attempted_at_ms"))
                .cloned()
                .unwrap_or(json!(ts));
            let validation = json!({
                "status": norm_status,
                "validation_token": Value::Null,
                "contract": res_obj.get("contract").cloned().unwrap_or(Value::Null),
                "diagnostic": res_obj.get("diagnostic").cloned().unwrap_or(Value::Null),
                "attempted_at_ms": attempted_at,
                "completed_at_ms": ts
            });
            let _ = store.set_kv(&format!("ds_validation:{id}"), &validation);
            let cur_rev = rec.get("revision").and_then(|v| v.as_i64()).unwrap_or(1);
            let next_rev = cur_rev + 1;
            if let Some(obj) = rec.as_object_mut() {
                obj.insert("validation".into(), validation);
                obj.insert("lifecycle".into(), json!(lifecycle));
                obj.insert("revision".into(), json!(next_rev));
                obj.insert("updated_at_ms".into(), json!(ts));
            }
            suite
                .upsert_record("data_source", id, &rec)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(rec)
        }
        ("data_source", "set_default_enabled") => {
            let input = payload.get("input").unwrap_or(payload);
            let id = payload
                .get("id")
                .or_else(|| payload.get("data_source_id"))
                .or_else(|| input.get("data_source_id"))
                .or_else(|| input.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            let enabled = payload
                .get("enabled")
                .or_else(|| payload.get("default_enabled"))
                .or_else(|| input.get("default_enabled"))
                .or_else(|| input.get("enabled"))
                .and_then(|v| v.as_bool())
                .ok_or(("invalid_argument", "missing boolean 'enabled'".into()))?;
            let existing = suite
                .get_record::<Value>("data_source", id)
                .map_err(|e| ("storage_error", e.to_string()))?
                .filter(|r| r.get("deleted_at_ms").is_none_or(|v| v.is_null()))
                .ok_or(("not_found", format!("data source '{id}' not found")))?;
            let cur_rev = existing
                .get("revision")
                .and_then(|v| v.as_i64())
                .unwrap_or(1);
            if let Some(exp) = payload
                .get("expected_revision")
                .or_else(|| input.get("expected_revision"))
                .and_then(|v| v.as_i64())
            {
                if exp != cur_rev {
                    return Err((
                        "revision_conflict",
                        json!({
                            "scope_id": format!("data_source:{id}"),
                            "expected_revision": exp.max(0) as u64,
                            "actual_revision": cur_rev.max(0) as u64
                        })
                        .to_string(),
                    ));
                }
            }
            let ts = payload
                .get("updated_at_ms")
                .or_else(|| input.get("updated_at_ms"))
                .and_then(|v| v.as_i64())
                .unwrap_or_else(now_ms);
            let mut rec = canonicalize_data_source_record(existing, Some(id));
            let next_rev = cur_rev + 1;
            if let Some(obj) = rec.as_object_mut() {
                obj.insert("default_enabled".into(), json!(enabled));
                obj.insert("revision".into(), json!(next_rev));
                obj.insert("updated_at_ms".into(), json!(ts));
            }
            suite
                .upsert_record("data_source", id, &rec)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(rec)
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
            if store
                .get_session(conv_id)
                .map_err(|e| ("storage_error", e.to_string()))?
                .is_none()
            {
                return Err((
                    "not_found",
                    format!("conversation '{conv_id}' not found"),
                ));
            }
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
            let id = input
                .get("id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(new_id);
            let source_id = input
                .get("source_id")
                .or_else(|| input.get("data_source_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("default");
            let source_title = input
                .get("source_title")
                .and_then(|v| v.as_str())
                .unwrap_or(source_id);
            let consumer = input
                .get("consumer")
                .and_then(|v| v.as_str())
                .unwrap_or("chat");
            let purpose = input
                .get("purpose")
                .or_else(|| input.get("summary"))
                .and_then(|v| v.as_str())
                .unwrap_or("conversation_context");
            let conv_id = input
                .get("conversation_id")
                .or_else(|| args.get("conversation_id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(ToString::to_string);
            let disclosed = input.get("disclosed_to_cloud").cloned().unwrap_or(Value::Null);
            let metadata = input
                .get("metadata")
                .cloned()
                .filter(|v| v.is_object())
                .unwrap_or_else(|| json!({}));
            let used_at_ms = input
                .get("used_at_ms")
                .and_then(|v| v.as_i64())
                .unwrap_or_else(now_ms);
            let record = json!({
                "id": id,
                "source_id": source_id,
                "source_title": source_title,
                "consumer": consumer,
                "purpose": purpose,
                "conversation_id": conv_id,
                "disclosed_to_cloud": disclosed,
                "metadata": metadata,
                "used_at_ms": used_at_ms,
                "summary": input.get("summary").and_then(|v| v.as_str()).unwrap_or(purpose)
            });
            suite
                .upsert_record("data_source_usage", &id, &record)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!(true))
        }
        ("data_source", "request_refresh") => {
            let input = payload.get("input").unwrap_or(payload);
            let id = payload
                .get("id")
                .or_else(|| payload.get("data_source_id"))
                .or_else(|| input.get("data_source_id"))
                .or_else(|| input.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            let existing = suite
                .get_record::<Value>("data_source", id)
                .map_err(|e| ("storage_error", e.to_string()))?
                .filter(|r| r.get("deleted_at_ms").is_none_or(|v| v.is_null()))
                .ok_or(("not_found", format!("data source '{id}' not found")))?;
            let mut rec = canonicalize_data_source_record(existing, Some(id));
            let cur_rev = rec.get("revision").and_then(|v| v.as_i64()).unwrap_or(1);
            if let Some(exp) = payload
                .get("expected_revision")
                .or_else(|| input.get("expected_revision"))
                .and_then(|v| v.as_i64())
            {
                if exp != cur_rev {
                    return Err((
                        "revision_conflict",
                        json!({
                            "scope_id": format!("data_source:{id}"),
                            "expected_revision": exp.max(0) as u64,
                            "actual_revision": cur_rev.max(0) as u64
                        })
                        .to_string(),
                    ));
                }
            }
            if rec
                .get("read_state")
                .and_then(|rs| rs.get("status"))
                .and_then(|v| v.as_str())
                == Some("pending")
                && rec
                    .get("read_state")
                    .and_then(|rs| rs.get("read_token"))
                    .and_then(|v| v.as_str())
                    .is_some()
            {
                return Err((
                    "conflict",
                    format!("data source '{id}' already has a pending refresh"),
                ));
            }
            let ts = payload
                .get("requested_at_ms")
                .or_else(|| input.get("requested_at_ms"))
                .and_then(|v| v.as_i64())
                .unwrap_or_else(now_ms);
            let token = new_id();
            let prev_rs = rec.get("read_state").cloned().unwrap_or_else(|| json!({}));
            let next_rs = json!({
                "status": "pending",
                "read_token": token,
                "content_reference": prev_rs.get("content_reference").cloned().unwrap_or(Value::Null),
                "observed_at_ms": prev_rs.get("observed_at_ms").cloned().unwrap_or(Value::Null),
                "schema_version": prev_rs.get("schema_version").cloned().unwrap_or(Value::Null),
                "record_count": prev_rs.get("record_count").cloned().unwrap_or(Value::Null),
                "diagnostic": Value::Null,
                "requested_at_ms": ts,
                "completed_at_ms": prev_rs.get("completed_at_ms").cloned().unwrap_or(Value::Null)
            });
            let next_rev = cur_rev + 1;
            if let Some(obj) = rec.as_object_mut() {
                obj.insert("read_state".into(), next_rs);
                obj.insert("revision".into(), json!(next_rev));
                obj.insert("updated_at_ms".into(), json!(ts));
            }
            suite
                .upsert_record("data_source", id, &rec)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(rec)
        }
        ("data_source", "record_read") => {
            let input = payload.get("input").unwrap_or(payload);
            let id = payload
                .get("id")
                .or_else(|| payload.get("data_source_id"))
                .or_else(|| input.get("data_source_id"))
                .or_else(|| input.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            let existing = suite
                .get_record::<Value>("data_source", id)
                .map_err(|e| ("storage_error", e.to_string()))?
                .filter(|r| r.get("deleted_at_ms").is_none_or(|v| v.is_null()))
                .ok_or(("not_found", format!("data source '{id}' not found")))?;
            let mut rec = canonicalize_data_source_record(existing, Some(id));
            let expected_token = rec
                .get("read_state")
                .and_then(|rs| rs.get("read_token"))
                .and_then(|v| v.as_str());
            if let Some(provided) = payload
                .get("read_token")
                .or_else(|| input.get("read_token"))
                .and_then(|v| v.as_str())
            {
                if expected_token != Some(provided) {
                    return Err((
                        "conflict",
                        "data source read_token does not match pending refresh".into(),
                    ));
                }
            }
            let ts = payload
                .get("recorded_at_ms")
                .or_else(|| input.get("recorded_at_ms"))
                .and_then(|v| v.as_i64())
                .unwrap_or_else(now_ms);
            let res_obj = payload
                .get("result")
                .or_else(|| input.get("result"))
                .unwrap_or(input);
            let raw_status = res_obj
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("succeeded");
            let norm_status = match raw_status {
                "succeeded" | "ready" => "ready",
                "succeeded_empty" | "empty" => "empty",
                "denied" => "denied",
                "unavailable" => "unavailable",
                "failed" | "cancelled" => "failed",
                _ => "ready",
            };
            let prev_rs = rec.get("read_state").cloned().unwrap_or_else(|| json!({}));
            let is_success = matches!(norm_status, "ready" | "empty");
            let content_ref = if is_success {
                res_obj
                    .get("content_reference")
                    .cloned()
                    .unwrap_or(Value::Null)
            } else {
                prev_rs
                    .get("content_reference")
                    .cloned()
                    .unwrap_or(Value::Null)
            };
            let observed_at = if is_success {
                res_obj
                    .get("observed_at_ms")
                    .cloned()
                    .unwrap_or_else(|| json!(ts))
            } else {
                prev_rs.get("observed_at_ms").cloned().unwrap_or(Value::Null)
            };
            let schema_version = if is_success {
                res_obj.get("schema_version").cloned().unwrap_or(Value::Null)
            } else {
                prev_rs.get("schema_version").cloned().unwrap_or(Value::Null)
            };
            let record_count = if is_success {
                res_obj.get("record_count").cloned().unwrap_or(Value::Null)
            } else {
                prev_rs.get("record_count").cloned().unwrap_or(Value::Null)
            };
            let diagnostic_val = {
                let code = res_obj
                    .get("code")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty());
                let raw_diag = res_obj
                    .get("diagnostic")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty());
                match (code, raw_diag) {
                    (Some(c), Some(d)) => {
                        if d.starts_with(&format!("{c}:")) {
                            json!(d)
                        } else {
                            json!(format!("{c}: {d}"))
                        }
                    }
                    (Some(c), None) => json!(format!("{c}:")),
                    (None, Some(d)) => json!(d),
                    (None, None) => Value::Null,
                }
            };
            let next_rs = json!({
                "status": norm_status,
                "read_token": Value::Null,
                "content_reference": content_ref,
                "observed_at_ms": observed_at,
                "schema_version": schema_version,
                "record_count": record_count,
                "diagnostic": diagnostic_val,
                "requested_at_ms": prev_rs.get("requested_at_ms").cloned().unwrap_or(json!(ts)),
                "completed_at_ms": ts
            });
            let cur_rev = rec.get("revision").and_then(|v| v.as_i64()).unwrap_or(1);
            let next_rev = cur_rev + 1;
            if let Some(obj) = rec.as_object_mut() {
                obj.insert("read_state".into(), next_rs);
                if is_success {
                    obj.insert("last_successful_read_at_ms".into(), json!(ts));
                }
                obj.insert("revision".into(), json!(next_rev));
                obj.insert("updated_at_ms".into(), json!(ts));
            }
            suite
                .upsert_record("data_source", id, &rec)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(rec)
        }
        ("data_source", "resolve_for_conversation") => {
            let input = payload.get("input").unwrap_or(payload);
            let conv_id = payload
                .get("conversation_id")
                .or_else(|| input.get("conversation_id"))
                .or_else(|| args.get("conversation_id"))
                .and_then(|v| v.as_str());
            let now = payload
                .get("now_ms")
                .or_else(|| input.get("now_ms"))
                .and_then(|v| v.as_i64())
                .unwrap_or_else(now_ms);
            let all: Vec<Value> = suite
                .list_records::<Value>("data_source")
                .map_err(|e| ("storage_error", e.to_string()))?
                .into_iter()
                .filter(|r| r.get("deleted_at_ms").is_none_or(|v| v.is_null()))
                .map(|r| canonicalize_data_source_record(r, None))
                .collect();
            let mut selections = Vec::with_capacity(all.len());
            for rec in all {
                let ds_id = rec
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let source_revision = rec
                    .get("revision")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(1)
                    .max(1);
                let default_enabled = rec
                    .get("default_enabled")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let override_mode = conv_id.and_then(|cid| {
                    store
                        .get_kv(&format!("ds_override:{cid}:{ds_id}"))
                        .ok()
                        .flatten()
                });
                let (effective, disabled_reason) = match override_mode {
                    Some(Value::Bool(true)) => (true, None),
                    Some(Value::Bool(false)) => (false, Some("disabled_for_conversation")),
                    Some(Value::String(ref s)) if s.eq_ignore_ascii_case("enabled") => (true, None),
                    Some(Value::String(ref s)) if s.eq_ignore_ascii_case("disabled") => {
                        (false, Some("disabled_for_conversation"))
                    }
                    _ => {
                        if default_enabled {
                            (true, None)
                        } else {
                            (false, Some("disabled_by_default"))
                        }
                    }
                };
                let lifecycle = rec
                    .get("lifecycle")
                    .and_then(|v| v.as_str())
                    .unwrap_or("draft");
                let auth_state = rec
                    .get("authorization")
                    .and_then(|a| a.get("state"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("not_required");
                let val_status = rec
                    .get("validation")
                    .and_then(|v| v.get("status"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("not_validated");
                let rs = rec.get("read_state").cloned().unwrap_or_else(|| json!({}));
                let read_status = rs
                    .get("status")
                    .and_then(|v| v.as_str())
                    .unwrap_or("never_read");
                let content_reference = rs
                    .get("content_reference")
                    .cloned()
                    .unwrap_or(Value::Null);
                let observed_at_ms = rs.get("observed_at_ms").and_then(|v| v.as_i64());
                let schema_version = rs
                    .get("schema_version")
                    .and_then(|v| v.as_i64())
                    .map(|n| n as i32);
                let max_age_min = rec
                    .get("freshness_policy")
                    .and_then(|f| f.get("maximum_age_minutes"))
                    .and_then(|v| v.as_i64())
                    .unwrap_or(60);
                let allow_stale = rec
                    .get("freshness_policy")
                    .and_then(|f| f.get("allow_stale_fallback"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true);
                let is_stale = observed_at_ms.is_some_and(|obs| {
                    max_age_min > 0 && now.saturating_sub(obs) > max_age_min.saturating_mul(60_000)
                });
                let (usable, should_refresh, reason) = if !effective {
                    (false, false, disabled_reason.unwrap_or("disabled_by_default"))
                } else if lifecycle != "ready" {
                    (false, false, "lifecycle_not_ready")
                } else if matches!(auth_state, "denied" | "restricted" | "unavailable") {
                    (false, false, "authorization_unavailable")
                } else if !matches!(val_status, "verified" | "verified_empty") {
                    (false, false, "validation_unavailable")
                } else {
                    match read_status {
                        "pending" => (false, false, "read_pending"),
                        "never_read" => (false, true, "refresh_required"),
                        "ready" => {
                            if is_stale {
                                if allow_stale {
                                    (true, true, "ready")
                                } else {
                                    (false, true, "refresh_required")
                                }
                            } else {
                                (true, false, "ready")
                            }
                        }
                        "empty" => {
                            if is_stale {
                                if allow_stale {
                                    (true, true, "ready_empty")
                                } else {
                                    (false, true, "refresh_required")
                                }
                            } else {
                                (true, false, "ready_empty")
                            }
                        }
                        _ => (false, true, "read_unavailable"),
                    }
                };
                selections.push(json!({
                    "source_id": ds_id,
                    "source_revision": source_revision,
                    "enabled": effective,
                    "usable": usable,
                    "should_refresh": should_refresh,
                    "reason": reason,
                    "content_reference": if usable { content_reference } else { Value::Null },
                    "observed_at_ms": if usable { observed_at_ms.map(|v| json!(v)).unwrap_or(Value::Null) } else { Value::Null },
                    "schema_version": if usable { schema_version.map(|v| json!(v)).unwrap_or(Value::Null) } else { Value::Null }
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
            let input = payload.get("input").unwrap_or(payload);
            let id = payload
                .get("id")
                .or_else(|| input.get("id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            let existing = suite
                .get_record::<Value>("data_source", id)
                .map_err(|e| ("storage_error", e.to_string()))?
                .filter(|r| r.get("deleted_at_ms").is_none_or(|v| v.is_null()))
                .ok_or(("not_found", format!("data source '{id}' not found")))?;
            let cur_rev = existing
                .get("revision")
                .and_then(|v| v.as_i64())
                .unwrap_or(1);
            if let Some(exp) = payload
                .get("expected_revision")
                .or_else(|| input.get("expected_revision"))
                .or_else(|| args.get("__envelope_expected_revision"))
                .and_then(|v| v.as_i64())
            {
                if exp != cur_rev {
                    return Err((
                        "revision_conflict",
                        json!({
                            "scope_id": format!("data_source:{id}"),
                            "expected_revision": exp.max(0) as u64,
                            "actual_revision": cur_rev.max(0) as u64
                        })
                        .to_string(),
                    ));
                }
            }
            let deleted = suite
                .delete_record("data_source", id)
                .map_err(|e| ("storage_error", e.to_string()))?;
            if !deleted {
                return Err(("not_found", format!("data source '{id}' not found")));
            }
            Ok(json!(true))
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
        ("automation", "list" | "get" | "create" | "upsert" | "update" | "update_if_revision" | "reconcile" | "pause" | "resume" | "delete") => {
            dispatch_automation_definition(suite, op, payload, args)
        }
        ("automation", "enqueue" | "claim" | "heartbeat" | "complete" | "fail" | "retry" | "load") => {
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
            let unchanged = existing.as_ref().is_some_and(|ex| {
                ex.get("name") == record.get("name")
                    && ex.get("schedule_format") == record.get("schedule_format")
                    && ex.get("enabled") == record.get("enabled")
                    && ex.get("execution_spec") == record.get("execution_spec")
                    && ex.get("metadata_json") == record.get("metadata_json")
                    && ex.get("deleted_at_ms") == record.get("deleted_at_ms")
            });
            if unchanged {
                if let Some(ex) = existing {
                    return Ok(ex);
                }
            }
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
        "reconcile" => {
            let inputs = payload
                .get("inputs")
                .or_else(|| input.get("inputs"))
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            let schedule_formats: HashSet<String> = payload
                .get("schedule_formats")
                .or_else(|| input.get("schedule_formats"))
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(ToString::to_string))
                        .collect()
                })
                .unwrap_or_default();
            let incoming_ids: HashSet<String> = inputs
                .iter()
                .filter_map(|item| {
                    item.get("id")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string)
                })
                .collect();
            let existing: Vec<Value> = suite
                .list_records("automation")
                .map_err(|e| ("storage_error", e.to_string()))?;
            for ex in &existing {
                let Some(eid) = ex.get("id").and_then(|v| v.as_str()) else {
                    continue;
                };
                let fmt = ex
                    .get("schedule_format")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let in_scope = schedule_formats.is_empty() || schedule_formats.contains(fmt);
                if in_scope && !incoming_ids.contains(eid) {
                    let _ = suite.delete_record("automation", eid);
                }
            }
            let ts = payload
                .get("reconciled_at_ms")
                .or_else(|| input.get("reconciled_at_ms"))
                .and_then(|v| v.as_i64())
                .unwrap_or_else(now_ms);
            let mut result = Vec::with_capacity(inputs.len());
            for item in inputs {
                let id = item
                    .get("id")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
                    .map(ToString::to_string)
                    .unwrap_or_else(new_id);
                let prev = suite.get_record::<Value>("automation", &id).ok().flatten();
                let prev_rev = prev
                    .as_ref()
                    .and_then(|v| v.get("revision").and_then(|r| r.as_u64()))
                    .unwrap_or(0);
                let unchanged = prev.as_ref().is_some_and(|ex| {
                    ex.get("name") == item.get("name")
                        && ex.get("schedule_format") == item.get("schedule_format")
                        && ex.get("enabled") == item.get("enabled")
                        && ex.get("execution_spec") == item.get("execution_spec")
                        && ex.get("metadata_json") == item.get("metadata_json")
                });
                if unchanged {
                    if let Some(ex) = prev {
                        result.push(ex);
                        continue;
                    }
                }
                let next_rev = prev_rev + 1;
                let mut stored = item.clone();
                if let Some(obj) = stored.as_object_mut() {
                    obj.insert("id".into(), json!(id));
                    obj.insert("revision".into(), json!(next_rev));
                    obj.entry("created_at_ms".to_string()).or_insert_with(|| json!(ts));
                    obj.entry("updated_at_ms".to_string()).or_insert_with(|| json!(ts));
                }
                suite
                    .upsert_record("automation", &id, &stored)
                    .map_err(|e| ("storage_error", e.to_string()))?;
                result.push(stored);
            }
            Ok(Value::Array(result))
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
            Ok(json!(deleted))
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
            let automation_id = input
                .get("automation_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let occurrence_key = input
                .get("occurrence_key")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if !automation_id.is_empty() && !occurrence_key.is_empty() {
                let existing_runs: Vec<Value> = suite
                    .list_records("automation_run")
                    .map_err(|e| ("storage_error", e.to_string()))?;
                if let Some(dup) = existing_runs.into_iter().find(|r| {
                    r.get("automation_id").and_then(|v| v.as_str()) == Some(automation_id)
                        && r.get("occurrence_key").and_then(|v| v.as_str()) == Some(occurrence_key)
                }) {
                    return Ok(dup);
                }
            }
            let id = input
                .get("run_id")
                .or_else(|| input.get("id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(new_id);
            let def_rev = input
                .get("definition_revision")
                .and_then(|v| v.as_i64())
                .or_else(|| {
                    suite
                        .get_record::<Value>("automation", automation_id)
                        .ok()
                        .flatten()
                        .and_then(|d| d.get("revision").and_then(|v| v.as_i64()))
                })
                .unwrap_or(1);
            let ts = input
                .get("created_at_ms")
                .and_then(|v| v.as_i64())
                .unwrap_or_else(now_ms);
            let mut rec = input.clone();
            if let Some(obj) = rec.as_object_mut() {
                obj.insert("id".into(), json!(id));
                obj.insert("definition_revision".into(), json!(def_rev));
                obj.entry("status".to_string()).or_insert_with(|| json!("queued"));
                obj.entry("attempt".to_string()).or_insert_with(|| json!(0));
                obj.entry("created_at_ms".to_string()).or_insert_with(|| json!(ts));
                obj.entry("updated_at_ms".to_string()).or_insert_with(|| json!(ts));
            }
            suite
                .upsert_record("automation_run", &id, &rec)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(rec)
        }
        "claim" => {
            let worker_id = input
                .get("worker_id")
                .and_then(|v| v.as_str())
                .unwrap_or("local-worker");
            let lease_token = input
                .get("lease_token")
                .and_then(|v| v.as_str())
                .map(ToString::to_string)
                .unwrap_or_else(new_id);
            let now = input
                .get("now_ms")
                .and_then(|v| v.as_i64())
                .unwrap_or_else(now_ms);
            let lease_until = input
                .get("lease_until_ms")
                .and_then(|v| v.as_i64())
                .unwrap_or(now + 60_000);
            let target_id = input
                .get("run_id")
                .or_else(|| input.get("id"))
                .or_else(|| payload.get("id"))
                .and_then(|v| v.as_str());
            let runs: Vec<Value> = suite
                .list_records("automation_run")
                .map_err(|e| ("storage_error", e.to_string()))?;
            let candidate = runs.into_iter().find(|r| {
                let id_ok = target_id
                    .is_none_or(|tid| r.get("id").and_then(|v| v.as_str()) == Some(tid));
                if !id_ok {
                    return false;
                }
                let st = r.get("status").and_then(|v| v.as_str()).unwrap_or("");
                if matches!(st, "queued" | "pending") {
                    let next_retry = r.get("next_retry_at_ms").and_then(|v| v.as_i64()).unwrap_or(0);
                    return now >= next_retry;
                }
                if st == "running" {
                    // Lease expiry alone must never replay a running occurrence.
                    return false;
                }
                false
            });
            let Some(mut rec) = candidate else {
                return Ok(json!(false));
            };
            let rid = rec
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let attempt = rec.get("attempt").and_then(|v| v.as_i64()).unwrap_or(0) + 1;
            if let Some(obj) = rec.as_object_mut() {
                obj.insert("status".into(), json!("running"));
                obj.insert("attempt".into(), json!(attempt));
                obj.insert("worker_id".into(), json!(worker_id));
                obj.insert("lease_token".into(), json!(lease_token));
                obj.insert("lease_until_ms".into(), json!(lease_until));
                obj.entry("started_at_ms".to_string()).or_insert_with(|| json!(now));
                obj.insert("updated_at_ms".into(), json!(now));
            }
            suite
                .upsert_record("automation_run", &rid, &rec)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!(true))
        }
        "heartbeat" => {
            let id = input
                .get("run_id")
                .or_else(|| input.get("id"))
                .or_else(|| payload.get("id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            let Some(mut rec) = suite
                .get_record::<Value>("automation_run", id)
                .map_err(|e| ("storage_error", e.to_string()))?
            else {
                return Ok(json!(false));
            };
            let status = rec.get("status").and_then(|v| v.as_str()).unwrap_or("");
            if status != "running" {
                return Ok(json!(false));
            }
            if let Some(req_token) = input.get("lease_token").and_then(|v| v.as_str()) {
                if rec.get("lease_token").and_then(|v| v.as_str()) != Some(req_token) {
                    return Ok(json!(false));
                }
            }
            let now = input
                .get("now_ms")
                .and_then(|v| v.as_i64())
                .unwrap_or_else(now_ms);
            let existing_until = rec.get("lease_until_ms").and_then(|v| v.as_i64()).unwrap_or(i64::MAX);
            if now > existing_until {
                return Ok(json!(false));
            }
            let next_until = input
                .get("lease_until_ms")
                .and_then(|v| v.as_i64())
                .unwrap_or(now + 60_000);
            if let Some(obj) = rec.as_object_mut() {
                obj.insert("lease_until_ms".into(), json!(next_until));
                obj.insert("updated_at_ms".into(), json!(now));
            }
            suite
                .upsert_record("automation_run", id, &rec)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!(true))
        }
        "complete" | "fail" => {
            let id = input
                .get("run_id")
                .or_else(|| input.get("id"))
                .or_else(|| payload.get("id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            let Some(mut rec) = suite
                .get_record::<Value>("automation_run", id)
                .map_err(|e| ("storage_error", e.to_string()))?
            else {
                return Ok(json!(false));
            };
            if rec.get("status").and_then(|v| v.as_str()) != Some("running") {
                return Ok(json!(false));
            }
            if let Some(req_token) = input.get("lease_token").and_then(|v| v.as_str()) {
                if rec.get("lease_token").and_then(|v| v.as_str()) != Some(req_token) {
                    return Ok(json!(false));
                }
            }
            let finished_at = input
                .get("finished_at_ms")
                .or_else(|| input.get("now_ms"))
                .and_then(|v| v.as_i64())
                .unwrap_or_else(now_ms);
            let existing_until = rec
                .get("lease_until_ms")
                .and_then(|v| v.as_i64())
                .unwrap_or(i64::MAX);
            if finished_at > existing_until {
                return Ok(json!(false));
            }
            if let Some(obj) = rec.as_object_mut() {
                obj.insert(
                    "status".into(),
                    json!(if op == "complete" { "succeeded" } else { "failed" }),
                );
                obj.insert("finished_at_ms".into(), json!(finished_at));
                obj.insert("lease_token".into(), Value::Null);
                obj.insert("lease_until_ms".into(), Value::Null);
                if let Some(res_json) = input.get("result_json") {
                    obj.insert("result_json".into(), res_json.clone());
                }
                if let Some(err_json) = input.get("error_json") {
                    obj.insert("error_json".into(), err_json.clone());
                }
                obj.insert("updated_at_ms".into(), json!(finished_at));
            }
            suite
                .upsert_record("automation_run", id, &rec)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!(true))
        }
        "retry" => {
            let id = input
                .get("run_id")
                .or_else(|| input.get("id"))
                .or_else(|| payload.get("id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            let Some(mut rec) = suite
                .get_record::<Value>("automation_run", id)
                .map_err(|e| ("storage_error", e.to_string()))?
            else {
                return Ok(json!(false));
            };
            if rec.get("status").and_then(|v| v.as_str()) != Some("running") {
                return Ok(json!(false));
            }
            if let Some(req_token) = input.get("lease_token").and_then(|v| v.as_str()) {
                if rec.get("lease_token").and_then(|v| v.as_str()) != Some(req_token) {
                    return Ok(json!(false));
                }
            }
            let ts = input
                .get("updated_at_ms")
                .or_else(|| input.get("now_ms"))
                .and_then(|v| v.as_i64())
                .unwrap_or_else(now_ms);
            let existing_until = rec
                .get("lease_until_ms")
                .and_then(|v| v.as_i64())
                .unwrap_or(i64::MAX);
            if ts > existing_until {
                return Ok(json!(false));
            }
            if let Some(obj) = rec.as_object_mut() {
                obj.insert("status".into(), json!("queued"));
                obj.insert("lease_token".into(), Value::Null);
                obj.insert("lease_until_ms".into(), Value::Null);
                obj.insert("worker_id".into(), Value::Null);
                if let Some(next_retry) = input.get("next_retry_at_ms") {
                    obj.insert("next_retry_at_ms".into(), next_retry.clone());
                }
                if let Some(err_json) = input.get("error_json") {
                    obj.insert("error_json".into(), err_json.clone());
                }
                obj.insert("updated_at_ms".into(), json!(ts));
            }
            suite
                .upsert_record("automation_run", id, &rec)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!(true))
        }
        "load" | "get" => {
            let id = input
                .get("run_id")
                .or_else(|| input.get("id"))
                .or_else(|| payload.get("run_id"))
                .or_else(|| payload.get("id"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'run_id'".into()))?;
            let found = suite
                .get_record::<Value>("automation_run", id)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(found.unwrap_or(Value::Null))
        }
        "list" => {
            let filter_auto = input
                .get("automation_id")
                .or_else(|| payload.get("automation_id"))
                .and_then(|v| v.as_str());
            let limit = input
                .get("limit")
                .or_else(|| payload.get("limit"))
                .and_then(|v| v.as_u64())
                .unwrap_or(200) as usize;
            let mut items: Vec<Value> = suite
                .list_records("automation_run")
                .map_err(|e| ("storage_error", e.to_string()))?;
            if let Some(aid) = filter_auto {
                items.retain(|r| r.get("automation_id").and_then(|v| v.as_str()) == Some(aid));
            }
            for item in &mut items {
                if let Some(obj) = item.as_object_mut() {
                    obj.entry("result_json".to_string()).or_insert(Value::Null);
                    obj.entry("error_json".to_string()).or_insert(Value::Null);
                }
            }
            items.truncate(limit);
            Ok(Value::Array(items))
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

pub(crate) fn canonical_agent_directory_payload(store: &Store, suite: &ModulesSuite) -> Result<Value, (&'static str, String)> {
    let rooms: Vec<AgentRoom> = suite
        .list_records("agent_room")
        .map_err(|e| ("storage_error", e.to_string()))?;
    let experts: Vec<Value> = suite
        .list_records("expert_peer")
        .map_err(|e| ("storage_error", e.to_string()))?;
    let dir_rev = store
        .get_kv("agent_directory_revision")
        .ok()
        .flatten()
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let agents: Vec<Value> = experts
        .iter()
        .map(|exp| {
            let id = exp.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let name = exp.get("name").and_then(|v| v.as_str()).unwrap_or("Expert").to_string();
            let revision = exp.get("revision").and_then(|v| v.as_i64()).unwrap_or(1);
            let availability = exp
                .get("availability")
                .and_then(|v| v.as_str())
                .unwrap_or("active")
                .to_string();
            let defaults = exp
                .get("defaults")
                .filter(|v| v.is_object())
                .cloned()
                .unwrap_or_else(|| json!({}));
            let description = exp
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            json!({
                "id": id,
                "name": name,
                "revision": revision,
                "availability": availability,
                "defaults": defaults,
                "description": description,
            })
        })
        .collect();
    Ok(json!({
        "revision": dir_rev,
        "agents": agents,
        "rooms": rooms,
        "entries": rooms,
        "experts": experts,
        "role_templates": built_in_role_templates()
    }))
}

fn canonical_role_template_catalog() -> Value {
    let templates = vec![
        json!({
            "id": "11111111-1111-4111-8111-111111111101",
            "role_id": "code_writer",
            "name": {"en": "Code writer", "zh-Hans": "代码编写", "zh": "代码编写"},
            "description": {"en": "Implements focused, well-tested code changes.", "zh-Hans": "负责精准的代码实现与单元测试。", "zh": "负责精准的代码实现与单元测试。"},
            "prompt": "You are a senior software engineer. Inspect existing code before editing, make minimal well-tested changes, and verify builds and tests.",
            "tool_pack": {"id": "code-implementation", "tools": ["read_file", "edit_file", "write_file", "grep", "run_shell"]},
            "skills": [{
                "id": "code-implementation",
                "name": {"en": "Code implementation", "zh-Hans": "代码实现", "zh": "代码实现"},
                "instructions": "Inspect relevant files, keep edits minimal and idiomatic, and verify with focused tests."
            }]
        }),
        json!({
            "id": "11111111-1111-4111-8111-111111111102",
            "role_id": "architect",
            "name": {"en": "System Architect", "zh-Hans": "系统架构师", "zh": "系统架构师"},
            "description": {"en": "Designs clean module boundaries, data flows, and resilient interfaces.", "zh-Hans": "设计清晰的模块边界、数据流与可靠接口。", "zh": "设计清晰的模块边界、数据流与可靠接口。"},
            "prompt": "You are a principal software architect. Focus on clean boundaries, minimal coupling, clear invariants, and concrete trade-off analysis.",
            "tool_pack": {"id": "architecture-review", "tools": ["read_file", "grep", "codebase_search"]},
            "skills": [{
                "id": "architecture-design",
                "name": {"en": "Architecture design", "zh-Hans": "架构设计", "zh": "架构设计"},
                "instructions": "Evaluate boundaries, invariants, failure modes, and interface contracts before proposing changes."
            }]
        }),
        json!({
            "id": "11111111-1111-4111-8111-111111111103",
            "role_id": "reviewer",
            "name": {"en": "Code Reviewer", "zh-Hans": "代码审查员", "zh": "代码审查员"},
            "description": {"en": "Reviews diffs for correctness, security, and maintainability.", "zh-Hans": "审查变更的正确性、安全性与可维护性。", "zh": "审查变更的正确性、安全性与可维护性。"},
            "prompt": "You are a meticulous code reviewer. Identify bugs, edge cases, race conditions, and security issues with concrete evidence.",
            "tool_pack": {"id": "code-review", "tools": ["read_file", "grep", "git_diff"]},
            "skills": [{
                "id": "code-review",
                "name": {"en": "Code review", "zh-Hans": "代码审查", "zh": "代码审查"},
                "instructions": "Prioritize correctness, security boundaries, and regression risks with exact file and line citations."
            }]
        }),
        json!({
            "id": "11111111-1111-4111-8111-111111111104",
            "role_id": "debugger",
            "name": {"en": "Debugger", "zh-Hans": "故障排查", "zh": "故障排查"},
            "description": {"en": "Isolates root causes and verifies minimal fixes.", "zh-Hans": "定位根因并验证最小化修复。", "zh": "定位根因并验证最小化修复。"},
            "prompt": "You are an expert debugger. Reproduce failures, inspect logs and state, isolate root causes, and verify fixes with regression tests.",
            "tool_pack": {"id": "debugging", "tools": ["read_file", "grep", "run_shell"]},
            "skills": [{
                "id": "root-cause-analysis",
                "name": {"en": "Root cause analysis", "zh-Hans": "根因分析", "zh": "根因分析"},
                "instructions": "Reproduce reliably, gather direct evidence, and verify the fix eliminates the root cause."
            }]
        }),
        json!({
            "id": "11111111-1111-4111-8111-111111111105",
            "role_id": "researcher",
            "name": {"en": "Researcher", "zh-Hans": "调研分析", "zh": "调研分析"},
            "description": {"en": "Synthesizes docs, codebase facts, and external references.", "zh-Hans": "综合文档、代码库事实与外部资料。", "zh": "综合文档、代码库事实与外部资料。"},
            "prompt": "You are a thorough technical researcher. Verify claims against primary sources and present structured, cited findings.",
            "tool_pack": {"id": "research", "tools": ["read_file", "grep", "web_search", "web_fetch"]},
            "skills": [{
                "id": "technical-research",
                "name": {"en": "Technical research", "zh-Hans": "技术调研", "zh": "技术调研"},
                "instructions": "Cross-check primary sources and distinguish verified facts from assumptions."
            }]
        }),
        json!({
            "id": "11111111-1111-4111-8111-111111111106",
            "role_id": "writer",
            "name": {"en": "Technical Writer", "zh-Hans": "技术写作", "zh": "技术写作"},
            "description": {"en": "Drafts clear documentation, specs, and release notes.", "zh-Hans": "撰写清晰的文档、规格说明与发布日志。", "zh": "撰写清晰的文档、规格说明与发布日志。"},
            "prompt": "You are a technical writer. Produce concise, accurate, well-structured documentation tailored to the audience.",
            "tool_pack": {"id": "writing", "tools": ["read_file", "write_file", "edit_file"]},
            "skills": [{
                "id": "documentation",
                "name": {"en": "Documentation", "zh-Hans": "文档撰写", "zh": "文档撰写"},
                "instructions": "Keep documentation accurate, concise, and verified against actual behavior."
            }]
        }),
        json!({
            "id": "11111111-1111-4111-8111-111111111107",
            "role_id": "security",
            "name": {"en": "Security Auditor", "zh-Hans": "安全审计", "zh": "安全审计"},
            "description": {"en": "Audits auth, SSRF, secret redaction, and least privilege.", "zh-Hans": "审计鉴权、SSRF、密钥脱敏与最小权限。", "zh": "审计鉴权、SSRF、密钥脱敏与最小权限。"},
            "prompt": "You are an application security auditor. Inspect trust boundaries, input validation, credential handling, and sandboxing.",
            "tool_pack": {"id": "security-audit", "tools": ["read_file", "grep"]},
            "skills": [{
                "id": "security-review",
                "name": {"en": "Security review", "zh-Hans": "安全审查", "zh": "安全审查"},
                "instructions": "Audit trust boundaries, SSRF, command injection, path traversal, and secret handling."
            }]
        }),
    ];
    json!({
        "version": 1,
        "features": {
            "reasoning_effort": true,
            "explicit_generation_reasoning": true,
            "rejoin": true,
            "main_agent_delegation": true,
            "inline_expert_mentions": true,
            "external_expert_executors": true
        },
        "templates": templates,
        "role_templates": built_in_role_templates()
    })
}

fn room_revision(store: &Store, room_id: &str) -> i64 {
    store
        .get_kv(&format!("room_revision:{room_id}"))
        .ok()
        .flatten()
        .and_then(|v| v.as_i64())
        .unwrap_or(1)
        .max(1)
}

fn bump_room_revision(store: &Store, room_id: &str) -> i64 {
    let next = store
        .get_kv(&format!("room_revision:{room_id}"))
        .ok()
        .flatten()
        .and_then(|v| v.as_i64())
        .unwrap_or(0)
        + 1;
    let _ = store.set_kv(&format!("room_revision:{room_id}"), &json!(next));
    next
}

fn load_room_participants(store: &Store, room: &AgentRoom) -> Vec<Value> {
    if let Ok(Some(Value::Array(arr))) = store.get_kv(&format!("room_participants:{}", room.id)) {
        return arr;
    }
    Vec::new()
}

fn canonical_room_snapshot(store: &Store, room: &AgentRoom) -> Value {
    let rev = room_revision(store, &room.id);
    let participants = load_room_participants(store, room);
    json!({
        "revision": rev,
        "room": {
            "id": room.id,
            "title": room.name,
            "name": room.name,
            "session_id": room.session_id,
            "workspace_root": room.workspace_root,
            "participant_expert_ids": room.participant_expert_ids,
            "participants": participants,
            "created_at_ms": room.created_at_ms,
            "updated_at_ms": room.updated_at_ms
        }
    })
}

pub(crate) fn dispatch_agent_room_command(
    store: &Store,
    suite: &ModulesSuite,
    args: &Value,
) -> Result<Value, (&'static str, String)> {
    let (op, payload) = extract_sub_command(args);
    let input = payload.get("input").unwrap_or(payload);
    match op {
        "directory" => canonical_agent_directory_payload(store, suite),
        "list" | "list_rooms" => {
            if payload.get("limit").is_some() || payload.get("offset").is_some() {
                let offset = payload.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                let limit = payload.get("limit").and_then(|v| v.as_u64()).unwrap_or(50) as usize;
                let rooms: Vec<AgentRoom> = suite
                    .list_records("agent_room")
                    .map_err(|e| ("storage_error", e.to_string()))?;
                let snaps: Vec<Value> = rooms
                    .iter()
                    .skip(offset)
                    .take(limit)
                    .map(|r| canonical_room_snapshot(store, r))
                    .collect();
                return Ok(Value::Array(snaps));
            }
            canonical_agent_directory_payload(store, suite)
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
            let Some(found) = rooms.into_iter().find(|r| {
                r.session_id == conv_id
                    || r.id == conv_id
                    || mapped_id.as_deref() == Some(r.id.as_str())
            }) else {
                return Ok(Value::Null);
            };
            let snap = canonical_room_snapshot(store, &found);
            let rev = room_revision(store, &found.id);
            Ok(json!({
                "conversation_id": conv_id,
                "room_id": found.id,
                "revision": rev,
                "room": snap
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
                let snap = canonical_room_snapshot(store, &existing);
                let rev = room_revision(store, &existing.id);
                return Ok(json!({
                    "ok": true,
                    "created": false,
                    "conversation_id": conv_id,
                    "room_id": existing.id,
                    "revision": rev,
                    "room": snap
                }));
            }
            let ts = input
                .get("at_ms")
                .or_else(|| payload.get("at_ms"))
                .and_then(|v| v.as_i64())
                .unwrap_or_else(now_ms);
            let room_id = new_id();
            let room = AgentRoom {
                id: room_id.clone(),
                name: input
                    .get("name")
                    .or_else(|| input.get("title"))
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
            let rev = bump_room_revision(store, &room.id);
            let _ = store.set_kv(&format!("conv_room:{conv_id}"), &json!(room_id));
            let snap = canonical_room_snapshot(store, &room);
            Ok(json!({
                "ok": true,
                "created": true,
                "conversation_id": conv_id,
                "room_id": room.id,
                "revision": rev,
                "room": snap
            }))
        }
        "child_conversations" => {
            let conv_id = input
                .get("conversation_id")
                .or_else(|| input.get("room_id"))
                .or_else(|| args.get("conversation_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let limit = input
                .get("limit")
                .or_else(|| payload.get("limit"))
                .and_then(|v| v.as_u64())
                .unwrap_or(30) as usize;
            let offset = input
                .get("offset")
                .or_else(|| payload.get("offset"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize;
            let rooms: Vec<AgentRoom> = suite
                .list_records("agent_room")
                .map_err(|e| ("storage_error", e.to_string()))?;
            let mapped_id = store
                .get_kv(&format!("conv_room:{conv_id}"))
                .ok()
                .flatten()
                .and_then(|v| v.as_str().map(ToString::to_string));
            let found_room = rooms.into_iter().find(|r| {
                !conv_id.is_empty()
                    && (r.session_id == conv_id
                        || r.id == conv_id
                        || mapped_id.as_deref() == Some(r.id.as_str()))
            });
            let mut all_children: Vec<Value> = Vec::new();
            let (room_id_val, room_rev_val) = if let Some(ref rm) = found_room {
                let mut combined_parts = load_room_participants(store, rm);
                if let Ok(Some(Value::Array(hist))) =
                    store.get_kv(&format!("room_historical_participants:{}", rm.id))
                {
                    combined_parts.extend(hist);
                }
                for p in combined_parts {
                    let cid = p
                        .get("context_conversation_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let active = p.get("active").and_then(|v| v.as_bool()).unwrap_or(false);
                    let sess = get_session_case_insensitive(store, cid).ok().flatten();
                    let lookup_id = sess.as_ref().map(|s| s.id.as_str()).unwrap_or(cid);
                    let msg_count = crate::web_api::reconstruct_v2_messages(store, lookup_id).0.len();
                    let created = sess.as_ref().map(|s| s.created_at_ms).unwrap_or(rm.created_at_ms);
                    let updated = sess.as_ref().map(|s| s.updated_at_ms).unwrap_or(rm.updated_at_ms);
                    let title = p
                        .get("display_name")
                        .and_then(|v| v.as_str())
                        .or_else(|| sess.as_ref().map(|s| s.title.as_str()))
                        .unwrap_or("Expert");
                    all_children.push(json!({
                        "conversation_id": lookup_id,
                        "revision": 1i64,
                        "title": title,
                        "participant_id": p.get("id").cloned().unwrap_or(Value::Null),
                        "agent_id": p.get("agent_id").cloned().unwrap_or(Value::Null),
                        "agent_revision": p.get("agent_revision").cloned().unwrap_or(json!(1)),
                        "display_name": title,
                        "is_current": active,
                        "active": active,
                        "created_at_ms": created,
                        "updated_at_ms": updated,
                        "message_count": msg_count
                    }));
                }
                (json!(rm.id), json!(room_revision(store, &rm.id)))
            } else {
                (Value::Null, Value::Null)
            };
            let total = all_children.len();
            let page: Vec<Value> = all_children.into_iter().skip(offset).take(limit).collect();
            let has_more = offset + page.len() < total;
            let next_offset = if has_more {
                json!((offset + page.len()) as i64)
            } else {
                Value::Null
            };
            Ok(json!({
                "version": 1,
                "conversation_id": conv_id,
                "room_id": room_id_val,
                "room_revision": room_rev_val,
                "children": page,
                "conversations": page,
                "items": page,
                "has_more": has_more,
                "next_offset": next_offset
            }))
        }
        "child_history" => {
            let parent_conv_id = input
                .get("conversation_id")
                .or_else(|| args.get("conversation_id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'conversation_id'".into()))?;
            let child_conv_id = input
                .get("child_conversation_id")
                .or_else(|| args.get("child_conversation_id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'child_conversation_id'".into()))?;
            let rooms: Vec<AgentRoom> = suite
                .list_records("agent_room")
                .map_err(|e| ("storage_error", e.to_string()))?;
            let mapped_id = store
                .get_kv(&format!("conv_room:{parent_conv_id}"))
                .ok()
                .flatten()
                .and_then(|v| v.as_str().map(ToString::to_string));
            let rm = rooms
                .into_iter()
                .find(|r| {
                    r.session_id.eq_ignore_ascii_case(parent_conv_id)
                        || r.id.eq_ignore_ascii_case(parent_conv_id)
                        || mapped_id.as_deref() == Some(r.id.as_str())
                })
                .ok_or(("not_found", format!("room for conversation '{parent_conv_id}' not found")))?;
            let mut all_parts = load_room_participants(store, &rm);
            if let Ok(Some(Value::Array(hist))) =
                store.get_kv(&format!("room_historical_participants:{}", rm.id))
            {
                all_parts.extend(hist);
            }
            let owns_child = all_parts.iter().any(|p| {
                p.get("context_conversation_id")
                    .and_then(|v| v.as_str())
                    .is_some_and(|c| c.eq_ignore_ascii_case(child_conv_id))
            });
            if !owns_child {
                return Err((
                    "not_found",
                    format!("child conversation '{child_conv_id}' does not belong to room '{}'", rm.id),
                ));
            }
            let child_sess = get_session_case_insensitive(store, child_conv_id)
                .map_err(|e| ("storage_error", e.to_string()))?
                .ok_or(("not_found", format!("child conversation '{child_conv_id}' not found")))?;
            let limit = input.get("limit").and_then(|v| v.as_u64()).unwrap_or(50) as usize;
            let before_ord = input.get("before_ordinal").and_then(|v| v.as_i64());
            let window = build_conversation_projection_response(store, &child_sess, 0, Some(limit), before_ord, true);
            Ok(json!({
                "version": 1,
                "conversation_id": parent_conv_id,
                "child_conversation_id": child_sess.id,
                "room_id": rm.id,
                "room_revision": room_revision(store, &rm.id),
                "source_owner": {"kind": "local"},
                "window": window,
                "expert_exchanges": []
            }))
        }
        "peer_exchanges" => {
            let cid = input
                .get("conversation_id")
                .or_else(|| input.get("room_id"))
                .or_else(|| input.get("id"))
                .or_else(|| args.get("conversation_id"))
                .or_else(|| args.get("room_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let exchanges = store
                .get_kv(&format!("room_exchanges:{cid}"))
                .ok()
                .flatten()
                .and_then(|v| v.as_array().cloned())
                .unwrap_or_default();
            Ok(json!({
                "version": 1,
                "conversation_id": cid,
                "exchanges": exchanges,
                "has_more": false,
                "next_offset": Value::Null
            }))
        }
        "configure_agent" => {
            let id = input
                .get("expert_id")
                .or_else(|| input.get("agent_id"))
                .or_else(|| input.get("id"))
                .or_else(|| payload.get("expert_id"))
                .or_else(|| payload.get("agent_id"))
                .or_else(|| payload.get("id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(new_id);
            let prev_val = suite
                .get_record::<Value>("expert_peer", &id)
                .ok()
                .flatten();
            let prev = prev_val
                .as_ref()
                .and_then(|v| serde_json::from_value::<ExpertPeer>(v.clone()).ok());
            let model_override = if let Some(val) = input
                .get("model_override")
                .or_else(|| input.get("modelOverride"))
                .or_else(|| payload.get("model_override"))
                .or_else(|| payload.get("modelOverride"))
            {
                if val.is_null() {
                    None
                } else {
                    val.as_str()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(ToString::to_string)
                }
            } else {
                prev.as_ref().and_then(|e| e.model_override.clone())
            };
            let description = if let Some(val) = input
                .get("description")
                .or_else(|| payload.get("description"))
            {
                if val.is_null() {
                    None
                } else {
                    val.as_str().map(ToString::to_string)
                }
            } else {
                prev_val
                    .as_ref()
                    .and_then(|v| v.get("description").and_then(|d| d.as_str()))
                    .map(ToString::to_string)
            };
            let color = if let Some(val) = input.get("color").or_else(|| payload.get("color")) {
                if val.is_null() {
                    None
                } else {
                    val.as_str()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(ToString::to_string)
                }
            } else {
                prev_val
                    .as_ref()
                    .and_then(|v| v.get("color").and_then(|c| c.as_str()))
                    .map(ToString::to_string)
            };
            let expert = ExpertPeer {
                id: id.clone(),
                name: input
                    .get("name")
                    .or_else(|| payload.get("name"))
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string)
                    .or_else(|| prev.as_ref().map(|e| e.name.clone()))
                    .unwrap_or_else(|| "Expert".into()),
                role: input
                    .get("role")
                    .or_else(|| payload.get("role"))
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string)
                    .or_else(|| prev.as_ref().map(|e| e.role.clone()))
                    .unwrap_or_else(|| "specialist".into()),
                system_prompt: input
                    .get("system_prompt")
                    .or_else(|| payload.get("system_prompt"))
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string)
                    .or_else(|| prev.as_ref().map(|e| e.system_prompt.clone()))
                    .unwrap_or_default(),
                model_override,
                allowed_tools: input
                    .get("allowed_tools")
                    .or_else(|| payload.get("allowed_tools"))
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
            let prev_rev = prev_val
                .as_ref()
                .and_then(|v| v.get("revision").and_then(|r| r.as_i64()))
                .unwrap_or(0);
            let availability = input
                .get("availability")
                .or_else(|| payload.get("availability"))
                .and_then(|v| v.as_str())
                .map(ToString::to_string)
                .or_else(|| {
                    prev_val
                        .as_ref()
                        .and_then(|v| v.get("availability").and_then(|a| a.as_str()))
                        .map(ToString::to_string)
                })
                .unwrap_or_else(|| "active".to_string());
            let defaults = input
                .get("defaults")
                .or_else(|| payload.get("defaults"))
                .filter(|v| v.is_object())
                .cloned()
                .or_else(|| {
                    prev_val
                        .as_ref()
                        .and_then(|v| v.get("defaults").filter(|d| d.is_object()).cloned())
                })
                .unwrap_or_else(|| json!({}));
            let mut expert_val = serde_json::to_value(&expert).unwrap_or_else(|_| json!({}));
            if let Some(obj) = expert_val.as_object_mut() {
                if let Some(desc) = description {
                    obj.insert("description".into(), json!(desc));
                }
                if let Some(col) = color {
                    obj.insert("color".into(), json!(col));
                }
                obj.insert("revision".into(), json!(prev_rev + 1));
                obj.insert("availability".into(), json!(availability));
                obj.insert("defaults".into(), defaults);
            }
            suite
                .upsert_record("expert_peer", &expert.id, &expert_val)
                .map_err(|e| ("storage_error", e.to_string()))?;
            let next_dir_rev = store
                .get_kv("agent_directory_revision")
                .ok()
                .flatten()
                .and_then(|v| v.as_i64())
                .unwrap_or(0)
                + 1;
            let _ = store.set_kv("agent_directory_revision", &json!(next_dir_rev));
            let mut dir = canonical_agent_directory_payload(store, suite)?;
            if let Some(obj) = dir.as_object_mut() {
                obj.insert("ok".into(), json!(true));
                obj.insert("expert".into(), expert_val);
            }
            Ok(dir)
        }
        "join" | "rejoin" | "deactivate" => {
            let room_id = input
                .get("room_id")
                .or_else(|| input.get("id"))
                .or_else(|| payload.get("room_id"))
                .or_else(|| args.get("room_id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'room_id'".into()))?;
            let mut room = suite
                .get_record::<AgentRoom>("agent_room", room_id)
                .map_err(|e| ("storage_error", e.to_string()))?
                .ok_or(("not_found", format!("agent_room '{room_id}' not found")))?;
            let mut participants = load_room_participants(store, &room);
            let ts = input
                .get("at_ms")
                .or_else(|| payload.get("at_ms"))
                .and_then(|v| v.as_i64())
                .unwrap_or_else(now_ms);
            if op == "deactivate" {
                let target_id = input
                    .get("participant_id")
                    .or_else(|| payload.get("participant_id"))
                    .or_else(|| input.get("expert_id"))
                    .or_else(|| input.get("agent_id"))
                    .and_then(|v| v.as_str())
                    .ok_or(("invalid_argument", "missing 'participant_id'".into()))?;
                for p in &mut participants {
                    let pid = p.get("id").and_then(|v| v.as_str());
                    let aid = p.get("agent_id").and_then(|v| v.as_str());
                    if pid == Some(target_id) || aid == Some(target_id) {
                        if let Some(obj) = p.as_object_mut() {
                            obj.insert("active".into(), json!(false));
                        }
                    }
                }
                room.participant_expert_ids.retain(|x| x != target_id);
                room.updated_at_ms = ts;
                suite
                    .upsert_record("agent_room", &room.id, &room)
                    .map_err(|e| ("storage_error", e.to_string()))?;
                let _ = store.set_kv(
                    &format!("room_participants:{}", room.id),
                    &Value::Array(participants),
                );
                let _ = bump_room_revision(store, &room.id);
                return Ok(canonical_room_snapshot(store, &room));
            }
            let expert_id = input
                .get("agent_id")
                .or_else(|| input.get("expert_id"))
                .or_else(|| payload.get("agent_id"))
                .or_else(|| payload.get("expert_id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'agent_id'".into()))?;
            let expert_rec = suite
                .get_record::<Value>("expert_peer", expert_id)
                .map_err(|e| ("storage_error", e.to_string()))?;
            let (display_name, description, agent_rev) = if let Some(ref ex) = expert_rec {
                let avail = ex
                    .get("availability")
                    .and_then(|v| v.as_str())
                    .unwrap_or("active");
                if avail.eq_ignore_ascii_case("archived") {
                    return Err((
                        "invalid_state",
                        format!("expert '{expert_id}' is archived and cannot join"),
                    ));
                }
                let actual_rev = ex.get("revision").and_then(|v| v.as_i64()).unwrap_or(1);
                if let Some(exp_rev) = input
                    .get("expected_agent_revision")
                    .or_else(|| input.get("agent_revision"))
                    .and_then(|v| v.as_i64())
                {
                    if exp_rev != actual_rev {
                        return Err((
                            "revision_conflict",
                            json!({
                                "scope_id": format!("agent:{expert_id}"),
                                "expected_revision": exp_rev.max(0) as u64,
                                "actual_revision": actual_rev.max(0) as u64
                            })
                            .to_string(),
                        ));
                    }
                }
                (
                    ex.get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Expert")
                        .to_string(),
                    ex.get("description")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    actual_rev,
                )
            } else {
                (
                    input
                        .get("display_name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Expert")
                        .to_string(),
                    input
                        .get("description")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    input
                        .get("agent_revision")
                        .or_else(|| input.get("expected_agent_revision"))
                        .and_then(|v| v.as_i64())
                        .unwrap_or(1),
                )
            };
            if op == "rejoin" {
                let hist_key = format!("room_historical_participants:{}", room.id);
                let mut historical: Vec<Value> = store
                    .get_kv(&hist_key)
                    .ok()
                    .flatten()
                    .and_then(|v| v.as_array().cloned())
                    .unwrap_or_default();
                let mut kept = Vec::new();
                for mut p in participants {
                    if p.get("agent_id").and_then(|v| v.as_str()) == Some(expert_id) {
                        if let Some(obj) = p.as_object_mut() {
                            obj.insert("active".into(), json!(false));
                        }
                        historical.insert(0, p);
                    } else {
                        kept.push(p);
                    }
                }
                participants = kept;
                let _ = store.set_kv(&hist_key, &Value::Array(historical));
            }
            let participant_id = new_id();
            let context_conv_id = new_id();
            let ctx_session = Session {
                id: context_conv_id.clone(),
                title: display_name.clone(),
                workspace_root: room.workspace_root.clone(),
                model_connection: None,
                model_id: None,
                created_at_ms: ts,
                updated_at_ms: ts,
            };
            store
                .upsert_session(&ctx_session)
                .map_err(|e| ("storage_error", e.to_string()))?;
            let owner_settings = json!({
                "agentRoomOwner": {
                    "room_id": room.id,
                    "participant_id": participant_id,
                    "agent_id": expert_id
                }
            });
            let _ = store.set_kv(
                &format!("conv_settings:{context_conv_id}"),
                &json!(owner_settings.to_string()),
            );
            participants.insert(
                0,
                json!({
                    "id": participant_id,
                    "agent_id": expert_id,
                    "agent_revision": agent_rev,
                    "display_name": display_name,
                    "description": description,
                    "context_conversation_id": context_conv_id,
                    "active": true
                }),
            );
            if !room.participant_expert_ids.iter().any(|x| x == expert_id) {
                room.participant_expert_ids.push(expert_id.to_string());
            }
            room.updated_at_ms = ts;
            suite
                .upsert_record("agent_room", &room.id, &room)
                .map_err(|e| ("storage_error", e.to_string()))?;
            let _ = store.set_kv(
                &format!("room_participants:{}", room.id),
                &Value::Array(participants),
            );
            let _ = bump_room_revision(store, &room.id);
            Ok(canonical_room_snapshot(store, &room))
        }
        "read" => {
            let room_id = input
                .get("room_id")
                .or_else(|| input.get("id"))
                .or_else(|| payload.get("room_id"))
                .or_else(|| args.get("room_id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'room_id'".into()))?;
            let Some(room) = suite
                .get_record::<AgentRoom>("agent_room", room_id)
                .map_err(|e| ("storage_error", e.to_string()))?
            else {
                return Ok(Value::Null);
            };
            Ok(canonical_room_snapshot(store, &room))
        }
        "list_role_templates" | "role_templates" => Ok(canonical_role_template_catalog()),
        "list_experts" => {
            let experts: Vec<Value> = suite
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
            let _ = bump_room_revision(store, &room.id);
            Ok(canonical_room_snapshot(store, &room))
        }
        "update_room" => {
            let id = input
                .get("id")
                .or_else(|| input.get("room_id"))
                .or_else(|| input.get("roomId"))
                .or_else(|| payload.get("id"))
                .or_else(|| payload.get("room_id"))
                .or_else(|| payload.get("roomId"))
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
                .or_else(|| payload.get("name"))
                .or_else(|| payload.get("title"))
                .and_then(|v| v.as_str())
            {
                room.name = name.to_string();
            }
            if let Some(ws) = input
                .get("workspace_root")
                .or_else(|| payload.get("workspace_root"))
                .and_then(|v| v.as_str())
            {
                room.workspace_root = ws.to_string();
            }
            if let Some(arr) = input
                .get("participant_expert_ids")
                .or_else(|| input.get("participantExpertIds"))
                .or_else(|| payload.get("participant_expert_ids"))
                .or_else(|| payload.get("participantExpertIds"))
                .and_then(|v| v.as_array())
            {
                room.participant_expert_ids = arr
                    .iter()
                    .filter_map(|x| x.as_str().map(ToString::to_string))
                    .collect();
            }
            let detach = input
                .get("detach")
                .or_else(|| payload.get("detach"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if detach {
                room.session_id.clear();
            } else if let Some(sess_val) = input
                .get("session_id")
                .or_else(|| input.get("conversation_id"))
                .or_else(|| input.get("conversationId"))
                .or_else(|| payload.get("session_id"))
                .or_else(|| payload.get("conversation_id"))
                .or_else(|| payload.get("conversationId"))
            {
                if sess_val.is_null() {
                    room.session_id.clear();
                } else if let Some(s) = sess_val.as_str() {
                    room.session_id = s.trim().to_string();
                }
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
            let ts = input
                .get("at_ms")
                .and_then(|v| v.as_i64())
                .unwrap_or_else(now_ms);
            let text = input
                .get("text")
                .or_else(|| input.get("content"))
                .or_else(|| input.get("message"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let req_part_id = input
                .get("participant_id")
                .or_else(|| input.get("recipient_id"))
                .or_else(|| input.get("expert_id"))
                .and_then(|v| v.as_str());
            let room_opt = suite.get_record::<AgentRoom>("agent_room", room_id).ok().flatten();
            let room_rev = room_opt
                .as_ref()
                .map(|r| room_revision(store, &r.id))
                .unwrap_or(1);
            let mut matched_pid: Option<String> = None;
            let mut matched_ctx: Option<String> = None;
            if let (Some(rm), Some(pid)) = (&room_opt, req_part_id) {
                for p in load_room_participants(store, rm) {
                    if p.get("active").and_then(|v| v.as_bool()).unwrap_or(true)
                        && (p.get("id").and_then(|v| v.as_str()) == Some(pid)
                            || p.get("agent_id").and_then(|v| v.as_str()) == Some(pid))
                    {
                        matched_pid = p.get("id").and_then(|v| v.as_str()).map(ToString::to_string);
                        matched_ctx = p
                            .get("context_conversation_id")
                            .and_then(|v| v.as_str())
                            .map(ToString::to_string);
                        break;
                    }
                }
            }
            let state = if matched_pid.is_some() { "selected" } else { "accepted" };
            let rec = json!({
                "revision": 1i64,
                "input": {
                    "room_id": room_id,
                    "submission_id": sub_id,
                    "expected_room_revision": input.get("expected_room_revision").and_then(|v| v.as_i64()).unwrap_or(room_rev),
                    "text": text,
                    "participant_id": req_part_id,
                    "at_ms": ts,
                    "parent_message_id": input.get("parent_message_id").cloned().unwrap_or(Value::Null),
                    "conversation_id": input.get("conversation_id").cloned().unwrap_or(Value::Null)
                },
                "state": state,
                "selected_participant_id": matched_pid,
                "failure_code": Value::Null,
                "execution_id": Value::Null,
                "terminal": Value::Null,
                "context_conversation_id": matched_ctx,
                "id": sub_id,
                "submission_id": sub_id,
                "room_id": room_id
            });
            suite
                .upsert_record("agent_room_submission", &sub_id, &rec)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(rec)
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
            Ok(found.unwrap_or(Value::Null))
        }
        "submissions" => {
            let room_id = input
                .get("room_id")
                .or_else(|| input.get("roomId"))
                .or_else(|| args.get("room_id"))
                .and_then(|v| v.as_str());
            let offset = input.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            let limit = input.get("limit").and_then(|v| v.as_u64()).unwrap_or(100) as usize;
            let list: Vec<Value> = suite
                .list_records::<Value>("agent_room_submission")
                .map_err(|e| ("storage_error", e.to_string()))?
                .into_iter()
                .filter(|s| {
                    room_id.is_none_or(|rid| {
                        s.get("room_id")
                            .or_else(|| s.pointer("/input/room_id"))
                            .and_then(|v| v.as_str())
                            == Some(rid)
                    })
                })
                .skip(offset)
                .take(limit)
                .collect();
            Ok(Value::Array(list))
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
                    sub.get("status")
                        .or_else(|| sub.get("state"))
                        .and_then(|v| v.as_str()),
                    Some("pending" | "prepared" | "claimed" | "accepted" | "selected")
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
                            obj.insert("state".into(), json!("cancelled"));
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
            let id = input
                .get("id")
                .or_else(|| input.get("room_id"))
                .or_else(|| input.get("roomId"))
                .or_else(|| payload.get("id"))
                .or_else(|| payload.get("room_id"))
                .or_else(|| payload.get("roomId"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            let deleted = suite
                .delete_record("agent_room", id)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"ok": true, "deleted": deleted, "id": id}))
        }
        "create_expert" | "upsert_expert" => {
            let id = input
                .get("id")
                .or_else(|| input.get("expert_id"))
                .or_else(|| payload.get("id"))
                .or_else(|| payload.get("expert_id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(new_id);
            let prev_val = suite
                .get_record::<Value>("expert_peer", &id)
                .ok()
                .flatten();
            let prev = prev_val
                .as_ref()
                .and_then(|v| serde_json::from_value::<ExpertPeer>(v.clone()).ok());
            let model_override = if let Some(val) = input
                .get("model_override")
                .or_else(|| input.get("modelOverride"))
                .or_else(|| payload.get("model_override"))
                .or_else(|| payload.get("modelOverride"))
            {
                if val.is_null() {
                    None
                } else {
                    val.as_str()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(ToString::to_string)
                }
            } else {
                prev.as_ref().and_then(|e| e.model_override.clone())
            };
            let description = if let Some(val) = input
                .get("description")
                .or_else(|| payload.get("description"))
            {
                if val.is_null() {
                    None
                } else {
                    val.as_str().map(ToString::to_string)
                }
            } else {
                prev_val
                    .as_ref()
                    .and_then(|v| v.get("description").and_then(|d| d.as_str()))
                    .map(ToString::to_string)
            };
            let expert = ExpertPeer {
                id: id.clone(),
                name: input
                    .get("name")
                    .or_else(|| payload.get("name"))
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string)
                    .or_else(|| prev.as_ref().map(|e| e.name.clone()))
                    .unwrap_or_else(|| "Expert".into()),
                role: input
                    .get("role")
                    .or_else(|| payload.get("role"))
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string)
                    .or_else(|| prev.as_ref().map(|e| e.role.clone()))
                    .unwrap_or_else(|| "specialist".into()),
                system_prompt: input
                    .get("system_prompt")
                    .or_else(|| payload.get("system_prompt"))
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string)
                    .or_else(|| prev.as_ref().map(|e| e.system_prompt.clone()))
                    .unwrap_or_default(),
                model_override,
                allowed_tools: input
                    .get("allowed_tools")
                    .or_else(|| payload.get("allowed_tools"))
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
            let mut expert_val = serde_json::to_value(&expert).unwrap_or_else(|_| json!({}));
            if let Some(obj) = expert_val.as_object_mut() {
                if let Some(desc) = description {
                    obj.insert("description".into(), json!(desc));
                }
            }
            suite
                .upsert_record("expert_peer", &expert.id, &expert_val)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"ok": true, "expert": expert_val}))
        }
        "delete_expert" | "delete_agent" => {
            let id = input
                .get("id")
                .or_else(|| input.get("expert_id"))
                .or_else(|| input.get("agent_id"))
                .or_else(|| payload.get("id"))
                .or_else(|| payload.get("expert_id"))
                .or_else(|| payload.get("agent_id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
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
        "load_manifest" => {
            let conv_id = payload
                .get("conversation_id")
                .or_else(|| args.get("conversation_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let msg_id = payload
                .get("message_id")
                .or_else(|| payload.get("user_message_id"))
                .or_else(|| args.get("message_id"))
                .or_else(|| args.get("user_message_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let found = if !conv_id.is_empty() && !msg_id.is_empty() {
                suite
                    .store
                    .get_kv(&format!("expert_manifest:{conv_id}:{msg_id}"))
                    .ok()
                    .flatten()
            } else {
                None
            };
            Ok(found.unwrap_or(Value::Null))
        }
        "preview" | "preview_capture" | "admit" => Err((
            "requires_main_agent_turn",
            "requires_main_agent_turn: direct expert fanout requires a main agent turn".into(),
        )),
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

fn build_compaction_segment_request(state: &Value) -> Result<Value, (&'static str, String)> {
    let segments = state
        .get("segments")
        .and_then(|v| v.as_array())
        .ok_or(("invalid_argument", "compaction state missing segments".into()))?;
    let idx = state.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    let target_tokens = state
        .get("target_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(1024);
    let instructions = state
        .get("instructions")
        .and_then(|v| v.as_str())
        .unwrap_or("Summarize the conversation faithfully while preserving key facts and decisions.");
    let accumulated = state
        .get("accumulated")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    if idx >= segments.len() {
        let merged = accumulated
            .iter()
            .filter_map(|v| v.as_str())
            .collect::<Vec<_>>()
            .join("\n\n");
        return Ok(json!({
            "action": "complete",
            "summary": merged
        }));
    }
    let framed_instructions = if instructions.to_ascii_lowercase().contains("checkpoint") {
        instructions.to_string()
    } else {
        format!("Create a bounded context compaction checkpoint summary.\n\n{instructions}")
    };
    let mut messages = vec![json!({
        "role": "system",
        "content": framed_instructions
    })];
    if !accumulated.is_empty() {
        let prev = accumulated
            .iter()
            .filter_map(|v| v.as_str())
            .collect::<Vec<_>>()
            .join("\n\n");
        messages.push(json!({
            "role": "assistant",
            "content": format!("Prior summary:\n{prev}")
        }));
    }
    if let Some(seg) = segments[idx].as_array() {
        for m in seg {
            let role = m.get("role").and_then(|v| v.as_str()).unwrap_or("user");
            let content = m
                .get("content")
                .or_else(|| m.get("text"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            messages.push(json!({
                "role": role,
                "content": content
            }));
        }
    }
    Ok(json!({
        "action": "request",
        "state": state,
        "messages": messages,
        "target_tokens": target_tokens
    }))
}

fn dispatch_step_compaction_summary(args: &Value) -> Result<Value, (&'static str, String)> {
    let step = args.get("step").unwrap_or(args);
    let step_type = step
        .get("type")
        .and_then(|v| v.as_str())
        .ok_or(("invalid_argument", "missing compaction step 'type'".into()))?;
    match step_type {
        "start" => {
            let msgs = step
                .get("messages")
                .and_then(|v| v.as_array())
                .ok_or(("invalid_argument", "missing compaction 'messages' array".into()))?;
            let window = step
                .get("context_window_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(16_384);
            let output = step
                .get("output_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(2_048);
            let target = step
                .get("target_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(1_024);
            if window == 0 || output >= window || target == 0 || window.saturating_sub(output) < 4_096 {
                return Err((
                    "invalid_argument",
                    "invalid compaction token budget parameters: insufficient summary window".into(),
                ));
            }
            let instructions = step
                .get("instructions")
                .and_then(|v| v.as_str())
                .unwrap_or("Summarize the conversation accurately and concisely.");
            if msgs.is_empty() {
                return Ok(json!({
                    "action": "complete",
                    "summary": ""
                }));
            }
            let available_budget = window.saturating_sub(output).saturating_sub(512).max(256) as usize;
            let input_budget = available_budget.saturating_sub(1_024).max(256);
            let mut segments: Vec<Value> = Vec::new();
            let mut current: Vec<Value> = Vec::new();
            let mut cur_tokens: usize = 0;
            for m in msgs {
                let text = m
                    .get("content")
                    .or_else(|| m.get("text"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let est = nola_types::estimate_text_tokens(text).max(16);
                if !current.is_empty() && cur_tokens + est > input_budget {
                    segments.push(Value::Array(current));
                    current = Vec::new();
                    cur_tokens = 0;
                }
                current.push(m.clone());
                cur_tokens += est;
            }
            if !current.is_empty() {
                segments.push(Value::Array(current));
            }
            let state = json!({
                "segments": segments,
                "index": 0u64,
                "target_tokens": target,
                "instructions": instructions,
                "accumulated": []
            });
            build_compaction_segment_request(&state)
        }
        "split" => {
            let mut state = step
                .get("state")
                .cloned()
                .ok_or(("invalid_argument", "missing compaction 'state'".into()))?;
            let idx = state.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            let segments = state
                .get_mut("segments")
                .and_then(|v| v.as_array_mut())
                .ok_or(("invalid_argument", "invalid compaction state segments".into()))?;
            if idx >= segments.len() {
                return Err(("invalid_state", "no active compaction segment to split".into()));
            }
            let cur_seg = segments[idx]
                .as_array()
                .cloned()
                .unwrap_or_default();
            if cur_seg.len() <= 1 {
                return Err((
                    "invalid_state",
                    "cannot split single-message compaction segment further".into(),
                ));
            }
            let mid = cur_seg.len() / 2;
            let left = Value::Array(cur_seg[..mid].to_vec());
            let right = Value::Array(cur_seg[mid..].to_vec());
            segments[idx] = left;
            segments.insert(idx + 1, right);
            build_compaction_segment_request(&state)
        }
        "accept" => {
            let mut state = step
                .get("state")
                .cloned()
                .ok_or(("invalid_argument", "missing compaction 'state'".into()))?;
            let summary = step
                .get("summary")
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing compaction 'summary'".into()))?;
            if summary.trim().is_empty() {
                return Err(("invalid_argument", "compaction summary must not be empty".into()));
            }
            let idx = state.get("index").and_then(|v| v.as_u64()).unwrap_or(0);
            if let Some(acc) = state.get_mut("accumulated").and_then(|v| v.as_array_mut()) {
                acc.push(json!(summary));
            }
            state["index"] = json!(idx + 1);
            build_compaction_segment_request(&state)
        }
        other => Err((
            "invalid_argument",
            format!("unsupported compaction step type '{other}'"),
        )),
    }
}

fn dispatch_evaluate_compaction_maintenance(
    args: &Value,
) -> Result<Value, (&'static str, String)> {
    let facts = args.get("facts").unwrap_or(args);
    let summary = facts.get("summary").and_then(|v| v.as_str()).unwrap_or("");
    let cancelled = facts
        .get("cancelled")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let has_tool_calls = facts
        .get("has_tool_calls")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let output_limited = facts
        .get("output_limited")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let completed = facts
        .get("completed")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let continuation_count = facts
        .get("continuation_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let rewrite_count = facts
        .get("rewrite_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let can_continue = facts
        .get("can_continue")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let can_rewrite = facts
        .get("can_rewrite")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if cancelled || has_tool_calls || summary.trim().is_empty() {
        return Ok(json!({"action": "reject"}));
    }
    if output_limited {
        if can_continue && continuation_count < 2 {
            return Ok(json!({
                "action": "continue",
                "instructions": "Continue the summary from where you stopped without repeating."
            }));
        }
        if can_rewrite && rewrite_count < 2 {
            return Ok(json!({
                "action": "rewrite",
                "instructions": "Rewrite a more concise summary within the target token budget."
            }));
        }
        return Ok(json!({"action": "reject"}));
    }
    if !completed {
        return Ok(json!({"action": "reject"}));
    }
    Ok(json!({"action": "accept"}))
}

fn aux_owner_key(owner: &Value) -> String {
    sha256_hex(canonical_json_value(owner).to_string().as_bytes())
}

fn dispatch_load_auxiliary_source_slot(
    store: &Store,
    args: &Value,
) -> Result<Value, (&'static str, String)> {
    let owner = args
        .get("local_owner")
        .or_else(|| args.get("owner"))
        .ok_or(("invalid_argument", "missing 'local_owner'".into()))?;
    let scenario = args
        .get("scenario")
        .and_then(|v| v.as_str())
        .unwrap_or("default");
    let key = format!("aux_slot:{}:{scenario}", aux_owner_key(owner));
    let found = store
        .get_kv(&key)
        .map_err(|e| ("storage_error", e.to_string()))?
        .filter(|v| !v.is_null());
    Ok(found.unwrap_or(Value::Null))
}

fn dispatch_auxiliary_invocation(
    store: &Store,
    args: &Value,
) -> Result<Value, (&'static str, String)> {
    let cmd = args.get("command").unwrap_or(args);
    let op = cmd
        .get("type")
        .and_then(|v| v.as_str())
        .ok_or(("invalid_argument", "missing auxiliary_invocation command 'type'".into()))?;
    match op {
        "admit" => {
            let op_id = cmd
                .get("operation_id")
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'operation_id'".into()))?;
            let key = format!("aux_op:{op_id}");
            if let Ok(Some(existing)) = store.get_kv(&key) {
                return Ok(existing);
            }
            let receipt = json!({
                "operation": {
                    "id": op_id,
                    "revision": 1i64,
                    "state": "admitted",
                    "attempts_started": 0i64
                },
                "attempt": Value::Null,
                "dispatch_permitted": true
            });
            store
                .set_kv(&key, &receipt)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(receipt)
        }
        "start_attempt" => {
            let op_id = cmd
                .get("operation_id")
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'operation_id'".into()))?;
            let attempt_id = cmd
                .get("attempt_id")
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'attempt_id'".into()))?;
            let key = format!("aux_op:{op_id}");
            let mut rec = store
                .get_kv(&key)
                .map_err(|e| ("storage_error", e.to_string()))?
                .ok_or(("not_found", format!("auxiliary operation '{op_id}' not found")))?;
            let cur_rev = rec["operation"]["revision"].as_i64().unwrap_or(1);
            if let Some(exp) = cmd.get("expected_revision").and_then(|v| v.as_i64()) {
                if exp != cur_rev {
                    return Err((
                        "revision_conflict",
                        json!({
                            "scope_id": format!("auxiliary_operation:{op_id}"),
                            "expected_revision": exp.max(0) as u64,
                            "actual_revision": cur_rev.max(0) as u64
                        })
                        .to_string(),
                    ));
                }
            }
            let attempts = rec["operation"]["attempts_started"].as_i64().unwrap_or(0) + 1;
            let state = rec["operation"]["state"].as_str().unwrap_or("admitted");
            let permitted = state == "admitted" || state == "retryable";
            rec["operation"]["revision"] = json!(cur_rev + 1);
            rec["operation"]["attempts_started"] = json!(attempts);
            if permitted {
                rec["operation"]["state"] = json!("running");
                rec["attempt"] = json!({
                    "id": attempt_id,
                    "revision": 1i64,
                    "state": "running",
                    "usage": {"coverage": "unknown", "provider_raw": Value::Null, "normalized": Value::Null}
                });
            }
            rec["dispatch_permitted"] = json!(permitted);
            store
                .set_kv(&key, &rec)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(rec)
        }
        "record_receipt" => {
            let op_id = cmd
                .get("operation_id")
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'operation_id'".into()))?;
            let key = format!("aux_op:{op_id}");
            let mut rec = store
                .get_kv(&key)
                .map_err(|e| ("storage_error", e.to_string()))?
                .ok_or(("not_found", format!("auxiliary operation '{op_id}' not found")))?;
            let state = cmd
                .get("state")
                .and_then(|v| v.as_str())
                .unwrap_or("succeeded");
            let usage = cmd
                .get("usage")
                .cloned()
                .unwrap_or_else(|| json!({"coverage": "unknown", "provider_raw": Value::Null, "normalized": Value::Null}));
            let cur_op_rev = rec["operation"]["revision"].as_i64().unwrap_or(1);
            let cur_att_rev = rec["attempt"]["revision"].as_i64().unwrap_or(1);
            rec["operation"]["revision"] = json!(cur_op_rev + 1);
            rec["operation"]["state"] = json!(state);
            if rec["attempt"].is_object() {
                rec["attempt"]["revision"] = json!(cur_att_rev + 1);
                rec["attempt"]["state"] = json!(state);
                rec["attempt"]["usage"] = usage;
            }
            rec["dispatch_permitted"] = json!(false);
            store
                .set_kv(&key, &rec)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(rec)
        }
        "cancel" => {
            let op_id = cmd
                .get("operation_id")
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'operation_id'".into()))?;
            let key = format!("aux_op:{op_id}");
            let mut rec = store
                .get_kv(&key)
                .map_err(|e| ("storage_error", e.to_string()))?
                .ok_or(("not_found", format!("auxiliary operation '{op_id}' not found")))?;
            let cur_op_rev = rec["operation"]["revision"].as_i64().unwrap_or(1);
            rec["operation"]["revision"] = json!(cur_op_rev + 1);
            rec["operation"]["state"] = json!("cancelled");
            rec["dispatch_permitted"] = json!(false);
            store
                .set_kv(&key, &rec)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(rec)
        }
        "observe_source" => {
            let owner = cmd
                .get("local_owner")
                .ok_or(("invalid_argument", "missing 'local_owner'".into()))?;
            let obs = cmd
                .get("observation")
                .ok_or(("invalid_argument", "missing 'observation'".into()))?;
            let source_host = obs
                .get("source_host_id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .ok_or(("invalid_argument", "observation missing 'source_host_id'".into()))?;
            let receipt_in = obs.get("receipt").unwrap_or(obs);
            let op_id = obs
                .get("operation_id")
                .or_else(|| receipt_in.get("operation").and_then(|o| o.get("id")))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .ok_or(("invalid_argument", "observation missing 'operation_id'".into()))?;
            let op_obj = receipt_in.get("operation").cloned().unwrap_or_else(|| {
                let state = receipt_in
                    .get("state")
                    .or_else(|| receipt_in.get("attempt").and_then(|a| a.get("state")))
                    .and_then(|v| v.as_str())
                    .unwrap_or("running");
                let attempts_started = receipt_in
                    .get("attempts_started")
                    .and_then(|v| v.as_i64())
                    .unwrap_or_else(|| {
                        if receipt_in.get("attempt").is_some_and(|a| !a.is_null()) {
                            1
                        } else {
                            0
                        }
                    });
                json!({
                    "id": op_id,
                    "revision": 1i64,
                    "state": state,
                    "attempts_started": attempts_started
                })
            });
            let att_obj = receipt_in.get("attempt").cloned().unwrap_or(Value::Null);
            let scenario = op_obj
                .get("scenario")
                .or_else(|| obs.get("scenario"))
                .and_then(|v| v.as_str())
                .unwrap_or("summary");
            let attempt_id = obs
                .get("attempt_id")
                .or_else(|| att_obj.get("id"))
                .and_then(|v| v.as_str())
                .unwrap_or("physical");
            let owner_h = aux_owner_key(owner);
            let src_key = format!("aux_source:{owner_h}:{source_host}:{op_id}");
            let slot_key = format!("aux_slot:{owner_h}:{scenario}");
            if let Ok(Some(prev)) = store.get_kv(&src_key) {
                let prev_rev = prev["operation"]["revision"].as_i64().unwrap_or(0);
                let new_rev = op_obj.get("revision").and_then(|v| v.as_i64()).unwrap_or(0);
                if new_rev > 0 && new_rev < prev_rev {
                    return Ok(prev);
                }
            }
            let op_state = op_obj
                .get("state")
                .or_else(|| att_obj.get("state"))
                .and_then(|v| v.as_str())
                .unwrap_or("running");
            let is_terminal = matches!(op_state, "succeeded" | "failed" | "cancelled");
            let receipt = json!({
                "operation": op_obj,
                "attempt": att_obj,
                "dispatch_permitted": false
            });
            store
                .set_kv(&src_key, &receipt)
                .map_err(|e| ("storage_error", e.to_string()))?;
            if is_terminal {
                let should_clear = store
                    .get_kv(&slot_key)
                    .ok()
                    .flatten()
                    .and_then(|s| s.get("operation_id").and_then(|v| v.as_str()).map(|x| x == op_id))
                    .unwrap_or(true);
                if should_clear {
                    let _ = store.set_kv(&slot_key, &Value::Null);
                }
            } else {
                let _ = store.set_kv(
                    &slot_key,
                    &json!({
                        "source_host_id": source_host,
                        "operation_id": op_id,
                        "attempt_id": attempt_id,
                        "scenario": scenario,
                        "local_owner": owner
                    }),
                );
            }
            Ok(receipt)
        }
        "inspect_source" => {
            let owner = cmd
                .get("local_owner")
                .ok_or(("invalid_argument", "missing 'local_owner'".into()))?;
            let source_host = cmd
                .get("source_host_id")
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'source_host_id'".into()))?;
            let op_id = cmd
                .get("operation_id")
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'operation_id'".into()))?;
            let src_key = format!("aux_source:{}:{source_host}:{op_id}", aux_owner_key(owner));
            let found = store
                .get_kv(&src_key)
                .map_err(|e| ("storage_error", e.to_string()))?
                .ok_or(("not_found", format!("auxiliary source receipt '{op_id}' not found")))?;
            Ok(found)
        }
        "inspect" => {
            let op_id = cmd
                .get("operation_id")
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'operation_id'".into()))?;
            let found = store
                .get_kv(&format!("aux_op:{op_id}"))
                .map_err(|e| ("storage_error", e.to_string()))?
                .ok_or(("not_found", format!("auxiliary operation '{op_id}' not found")))?;
            Ok(found)
        }
        other => Err((
            "invalid_argument",
            format!("unsupported auxiliary_invocation command '{other}'"),
        )),
    }
}

fn source_submission_key(identity: &Value) -> String {
    let host = identity
        .get("source_host_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let client = identity
        .get("client_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let conv = identity
        .get("conversation_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let sub = identity
        .get("submission_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    format!("source_submission:{host}:{client}:{conv}:{sub}")
}

fn dispatch_register_source_submission(
    store: &Store,
    args: &Value,
) -> Result<Value, (&'static str, String)> {
    let identity = args
        .get("identity")
        .ok_or(("invalid_argument", "missing 'identity'".into()))?;
    let request = args
        .get("request")
        .ok_or(("invalid_argument", "missing 'request'".into()))?;
    let digest = sha256_hex(canonical_json_value(request).to_string().as_bytes());
    let key = source_submission_key(identity);
    if let Ok(Some(existing)) = store.get_kv(&key) {
        if existing.get("request_digest").and_then(|v| v.as_str()) != Some(digest.as_str()) {
            return Err((
                "conflict",
                "source submission identity already registered with a different request digest"
                    .into(),
            ));
        }
        return Ok(json!({
            "receipt": existing,
            "created": false
        }));
    }
    let receipt = json!({
        "protocol_version": 1,
        "identity": identity,
        "request_digest": digest,
        "phase": "bound",
        "admission": Value::Null,
        "rejection_code": Value::Null
    });
    store
        .set_kv(&key, &receipt)
        .map_err(|e| ("storage_error", e.to_string()))?;
    let conv_id = identity
        .get("conversation_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let sub_id = identity
        .get("submission_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if !conv_id.is_empty() && !sub_id.is_empty() {
        let _ = store.set_kv(&format!("source_submission_by_run:{conv_id}:{sub_id}"), &json!(key));
    }
    let _ = store.set_kv(&format!("source_submission_req:{digest}"), request);
    Ok(json!({
        "receipt": receipt,
        "created": true
    }))
}

fn dispatch_reject_source_submission(
    store: &Store,
    args: &Value,
) -> Result<Value, (&'static str, String)> {
    let identity = args
        .get("identity")
        .ok_or(("invalid_argument", "missing 'identity'".into()))?;
    let code = args
        .get("rejection_code")
        .or_else(|| args.get("code"))
        .and_then(|v| v.as_str())
        .unwrap_or("rejected");
    let key = source_submission_key(identity);
    let mut rec = store
        .get_kv(&key)
        .map_err(|e| ("storage_error", e.to_string()))?
        .ok_or(("not_found", "source submission not found".into()))?;
    rec["phase"] = json!("rejected");
    rec["rejection_code"] = json!(code);
    store
        .set_kv(&key, &rec)
        .map_err(|e| ("storage_error", e.to_string()))?;
    Ok(rec)
}

fn dispatch_read_source_submission_receipt(
    store: &Store,
    args: &Value,
) -> Result<Value, (&'static str, String)> {
    let identity = args
        .get("identity")
        .ok_or(("invalid_argument", "missing 'identity'".into()))?;
    let key = source_submission_key(identity);
    let mut found = store
        .get_kv(&key)
        .map_err(|e| ("storage_error", e.to_string()))?;
    if let Some(ref mut rec) = found {
        if rec.get("admission").is_none_or(|v| v.is_null()) {
            let conv = identity.get("conversation_id").and_then(|v| v.as_str()).unwrap_or("");
            let sub = identity.get("submission_id").and_then(|v| v.as_str()).unwrap_or("");
            if let Ok(Some(head)) = store.get_kv(&format!("run_admission_head:{conv}:{sub}")) {
                rec["phase"] = json!("admitted");
                rec["admission"] = head;
                let _ = store.set_kv(&key, rec);
            }
        }
    }
    Ok(found.unwrap_or(Value::Null))
}

const EMPTY_APPLICATIONS_DIGEST: &str =
    "e0aeaeba0c6a61c02edcc186902f9b41dd82835d01b780eb0e479cc513ea66dd";

fn ensure_source_store_epoch(store: &Store) -> String {
    if let Ok(Some(Value::String(ep))) = store.get_kv("source_store_epoch") {
        if !ep.is_empty() {
            return ep;
        }
    }
    let ep = new_id();
    let _ = store.set_kv("source_store_epoch", &json!(ep));
    ep
}

pub(crate) fn journal_identity_value(
    store: &Store,
    conv_id: &str,
    run_id: &str,
    assistant_msg_id: &str,
) -> Value {
    json!({
        "source_store_epoch": ensure_source_store_epoch(store),
        "conversation_id": conv_id,
        "run_id": run_id,
        "assistant_message_id": assistant_msg_id
    })
}

pub(crate) fn publish_journal_part(
    store: &Store,
    run: &mut Value,
    part_id: &str,
    position: u64,
    kind: &str,
    metadata_data: Value,
    bodies: &[(&str, String)],
) -> Value {
    let conv_id = run.get("conversation_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let run_id = run.get("run_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let assistant_msg_id = run
        .get("assistant_message_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let identity = run
        .get("journal_identity")
        .cloned()
        .unwrap_or_else(|| journal_identity_value(store, &conv_id, &run_id, &assistant_msg_id));
    let jseq = run.get("journal_seq").and_then(|v| v.as_u64()).unwrap_or(0) + 1;
    run["journal_seq"] = json!(jseq);

    let mut body_refs = Vec::new();
    for (field, text) in bodies {
        let content_key = format!("assistant_journal_content:{conv_id}:{run_id}:{part_id}:{field}");
        let digest = sha256_hex(text.as_bytes());
        let root_key = format!(
            "assistant_journal_content_by_root:{conv_id}:{run_id}:{part_id}:{field}:{digest}"
        );
        let payload_val = json!({"text": text});
        let _ = store.set_kv(&content_key, &payload_val);
        let _ = store.set_kv(&root_key, &payload_val);
        body_refs.push(json!({
            "field": field,
            "content_generation": "1",
            "utf8_bytes": text.len().to_string(),
            "chunk_count": if text.is_empty() { "0" } else { "1" },
            "content_root": digest
        }));
    }
    let head = json!({
        "identity": identity,
        "part_id": part_id,
        "generation": "1",
        "revision": jseq.to_string(),
        "metadata_revision": jseq.to_string(),
        "order_key": {
            "section": "transcript",
            "position": position.to_string()
        },
        "metadata": {
            "kind": kind,
            "data": metadata_data
        },
        "bodies": body_refs
    });
    let part_key = format!("{conv_id}:{run_id}:{part_id}");
    let _ = store.put_module_record("assistant_journal_part", &part_key, &head);

    let prev_digest = run
        .get("journal_last_digest")
        .and_then(|v| v.as_str())
        .map(ToString::to_string)
        .unwrap_or_else(|| "0".repeat(64));
    let batch_digest = sha256_hex(canonical_json_value(&head).to_string().as_bytes());
    let record_digest = sha256_hex(format!("{prev_digest}:{jseq}:{batch_digest}").as_bytes());
    run["journal_last_digest"] = json!(record_digest);
    let cursor = json!({
        "identity": identity,
        "sequence": jseq.to_string(),
        "record_digest": record_digest
    });
    let record = json!({
        "header": {
            "protocol_version": 1u16,
            "identity": identity,
            "sequence": jseq.to_string(),
            "previous_record_digest": prev_digest,
            "operation_id": new_id(),
            "source_commit_revision": jseq.to_string(),
            "operation_batch_digest": batch_digest,
            "record_digest": record_digest
        },
        "batch": {
            "part_id": part_id,
            "head": head
        }
    });
    let rec_key = format!("journal_records:{conv_id}:{run_id}");
    let mut recs = store
        .get_kv(&rec_key)
        .ok()
        .flatten()
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();
    recs.push(record);
    let _ = store.set_kv(&rec_key, &Value::Array(recs));
    let _ = store.set_kv(&format!("journal_head:{conv_id}:{run_id}"), &cursor);
    if !conv_id.is_empty() {
        let idx_key = format!("journal_heads_by_conv:{conv_id}");
        let mut heads = store
            .get_kv(&idx_key)
            .ok()
            .flatten()
            .and_then(|v| v.as_array().cloned())
            .unwrap_or_default();
        if let Some(pos) = heads.iter().position(|h| {
            h.pointer("/identity/run_id").and_then(|v| v.as_str()) == Some(run_id.as_str())
                && h.pointer("/identity/assistant_message_id")
                    .and_then(|v| v.as_str())
                    == Some(assistant_msg_id.as_str())
        }) {
            heads[pos] = cursor.clone();
        } else {
            heads.push(cursor.clone());
        }
        let _ = store.set_kv(&idx_key, &Value::Array(heads));
    }
    cursor
}

pub(crate) fn build_and_store_terminal_seal(store: &Store, run: &Value, text_part_id: &str) -> Value {
    let conv_id = run.get("conversation_id").and_then(|v| v.as_str()).unwrap_or("");
    let run_id = run.get("run_id").and_then(|v| v.as_str()).unwrap_or("");
    let assistant_msg_id = run
        .get("assistant_message_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let session_id = run
        .get("session_id")
        .and_then(|v| v.as_str())
        .unwrap_or(conv_id);
    let identity = run
        .get("journal_identity")
        .cloned()
        .unwrap_or_else(|| journal_identity_value(store, conv_id, run_id, assistant_msg_id));
    let admission = store
        .get_kv(&format!("run_admission_head:{conv_id}:{run_id}"))
        .ok()
        .flatten()
        .unwrap_or_else(|| {
            json!({
                "identity": identity,
                "sequence": "1",
                "record_digest": sha256_hex(format!("admit:{conv_id}:{run_id}").as_bytes())
            })
        });
    let seq = run.get("sequence").and_then(|v| v.as_u64()).unwrap_or(1);
    let completed_at_ms = now_ms();
    let status = run.get("status").and_then(|v| v.as_str()).unwrap_or("completed");
    let value = match status {
        "failed" => json!({
            "kind": "failed",
            "error_ref": run
                .pointer("/error/code")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .unwrap_or("run_failed"),
            "observed_effect_refs": [],
            "unknown_effect_refs": []
        }),
        "cancelled" => json!({
            "kind": "cancelled",
            "cancellation_id": run
                .get("cancellation_id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .unwrap_or(run_id),
            "observed_effect_refs": [],
            "unknown_effect_refs": []
        }),
        "incomplete" => json!({
            "kind": "incomplete",
            "reason": {
                "code": "incomplete",
                "detail": Value::Null
            },
            "remaining_requirement_refs": ["incomplete"],
            "obligation_refs": []
        }),
        _ => json!({
            "kind": "answered",
            "answer_part_refs": [text_part_id]
        }),
    };
    let result = json!({
        "result_id": new_id(),
        "run_id": run_id,
        "conversation_id": conv_id,
        "originating_submission_id": run_id,
        "input": {
            "watermark": "0",
            "applications_digest": EMPTY_APPLICATIONS_DIGEST
        },
        "decided_at_revision": seq.to_string(),
        "decision_event_cursor": seq.to_string(),
        "requirement_revision": "0",
        "evidence_frontier": seq.to_string(),
        "value": value
    });
    let result_digest = sha256_hex(canonical_json_value(&result).to_string().as_bytes());
    let head = store
        .get_kv(&format!("journal_head:{conv_id}:{run_id}"))
        .ok()
        .flatten()
        .unwrap_or_else(|| {
            json!({
                "identity": identity,
                "sequence": "1",
                "record_digest": sha256_hex(format!("head:{conv_id}:{run_id}").as_bytes())
            })
        });
    let manifest_entry = json!({
        "kind": "assistant",
        "through": head,
        "ordinal": 1i64,
        "created_at_ms": run.get("created_at_ms").and_then(|v| v.as_i64()).unwrap_or(completed_at_ms),
        "state": "completed",
        "completed_at_ms": completed_at_ms
    });
    let manifest_digest = sha256_hex(canonical_json_value(&manifest_entry).to_string().as_bytes());
    let transition_id = new_id();
    let seal_digest = sha256_hex(
        format!("{conv_id}:{run_id}:{result_digest}:{manifest_digest}:{transition_id}").as_bytes(),
    );
    let seal = json!({
        "protocol_version": 1u16,
        "admission": admission,
        "identity": identity,
        "core_session_id": session_id,
        "transition_id": transition_id,
        "checkpoint_revision": "1",
        "core_revision": seq.to_string(),
        "event_cursor": seq.to_string(),
        "result": result,
        "result_digest": result_digest,
        "completed_at_ms": completed_at_ms,
        "entry_count": "1",
        "heads_count": "1",
        "child_receipt_count": "0",
        "manifest_digest": manifest_digest,
        "seal_digest": seal_digest
    });
    let _ = store.set_kv(&format!("run_terminal_seal:{conv_id}:{run_id}"), &seal);
    let canonical_terminal = json!({
        "conversation_id": conv_id,
        "run_id": run_id,
        "assistant_message_id": assistant_msg_id,
        "status": "sealed",
        "result": result,
        "result_digest": result_digest,
        "completed_at_ms": completed_at_ms
    });
    let _ = store.set_kv(&format!("run_terminal:{run_id}"), &canonical_terminal);
    if !conv_id.is_empty() && !run_id.is_empty() {
        let exact_key = format!("exact_run_invocation:{conv_id}:{run_id}");
        if let Ok(Some(mut exact)) = store.get_kv(&exact_key) {
            if let Some(obj) = exact.as_object_mut() {
                obj.insert("status".into(), json!("sealed"));
            }
            let _ = store.set_kv(&exact_key, &exact);
        }
    }
    if !conv_id.is_empty()
        && lookup_active_run(store, conv_id)
            .and_then(|ar| ar.get("run_id").and_then(|v| v.as_str()).map(|r| r == run_id))
            .unwrap_or(false)
    {
        let _ = store.set_kv(&format!("active_run:{conv_id}"), &Value::Null);
    }
    let _ = store.set_kv(
        &format!("run_terminal_manifest:{conv_id}:{run_id}"),
        &json!([manifest_entry]),
    );
    if let Some(occ_id) = run
        .get("automation_occurrence_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        let akey = format!("automation_exec_state:{occ_id}");
        let mut obs = store
            .get_kv(&akey)
            .ok()
            .flatten()
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default();
        obs.insert("terminal".into(), seal.clone());
        let _ = store.set_kv(&akey, &Value::Object(obs));
    }
    seal
}

fn build_model_stream_payload(
    core_config: &Value,
    messages: Value,
    assistant_msg_id: &str,
) -> Value {
    let tools = core_config
        .get("tools")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let cap = core_config.pointer("/turn_identity/model_capability");
    let native_tool_calls = cap
        .and_then(|c| c.get("native_tool_calls"))
        .and_then(|v| v.as_bool())
        .unwrap_or(!tools.is_empty());
    let parallel_tool_calls = cap
        .and_then(|c| c.get("parallel_tool_calls"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let reasoning = cap
        .and_then(|c| c.get("reasoning"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let prompt_cache = cap
        .and_then(|c| c.get("prompt_cache"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let stream_text = cap
        .and_then(|c| c.get("stream_text"))
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let stream_reasoning = cap
        .and_then(|c| c.get("stream_reasoning"))
        .and_then(|v| v.as_bool())
        .unwrap_or(stream_text);
    let stream_tool_calls = cap
        .and_then(|c| c.get("stream_tool_calls"))
        .and_then(|v| v.as_bool())
        .unwrap_or(native_tool_calls && stream_text);
    let context_window_tokens = core_config
        .pointer("/context_window/context_window_tokens")
        .or_else(|| core_config.get("context_window_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(128_000)
        .max(1_024);
    let effective_max_output_tokens = core_config
        .pointer("/turn_identity/model_capability/max_output_tokens")
        .or_else(|| core_config.pointer("/context_window/reserved_output_tokens"))
        .or_else(|| core_config.get("max_output_tokens"))
        .or_else(|| core_config.get("max_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(4_096)
        .clamp(64, 1_000_000);
    let prompt_token_limit = context_window_tokens
        .saturating_sub(effective_max_output_tokens)
        .max(512);
    let stable_prefix_digest = core_config
        .pointer("/turn_identity/prompt/stable_prefix_digest")
        .cloned()
        .unwrap_or(Value::Null);
    let mut full_messages: Vec<Value> = core_config
        .get("initial_items")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    if let Some(arr) = messages.as_array() {
        for m in arr {
            let mut item = m.clone();
            if let Some(obj) = item.as_object_mut() {
                obj.entry("type".to_string()).or_insert_with(|| json!("message"));
            }
            full_messages.push(item);
        }
    }
    json!({
        "type": "model_stream",
        "model": core_config.get("model").cloned().unwrap_or_else(|| json!("default")),
        "messages": full_messages,
        "tools": tools,
        "text_part_id": format!("{assistant_msg_id}:text"),
        "reasoning_part_id": format!("{assistant_msg_id}:reasoning"),
        "settings": {
            "context_window_tokens": context_window_tokens,
            "prompt_token_limit": prompt_token_limit,
            "model_prefix_digest": stable_prefix_digest,
            "model_invocation_plan": {
                "effective_max_output_tokens": effective_max_output_tokens,
                "native_tool_calls": native_tool_calls,
                "parallel_tool_calls": parallel_tool_calls,
                "reasoning": reasoning,
                "prompt_cache": prompt_cache,
                "stream_text": stream_text,
                "stream_reasoning": stream_reasoning,
                "stream_tool_calls": stream_tool_calls
            }
        }
    })
}

fn dispatch_begin_assistant_turn(
    store: &Store,
    suite: &ModulesSuite,
    _cmd_type: &str,
    args: &Value,
) -> Result<Value, (&'static str, String)> {
    let input = args.get("input").unwrap_or(args);
    let conv_id = input
        .get("conversation_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .ok_or(("invalid_argument", "missing 'conversation_id'".into()))?;
    let core_config = input
        .get("core_config")
        .ok_or(("invalid_command", "missing 'core_config'".into()))?;
    if let Some(cfg_obj) = core_config.as_object() {
        const ALLOWED_CORE_CONFIG_FIELDS: &[&str] = &[
            "session_id",
            "turn_identity",
            "context_window",
            "max_steps",
            "max_tool_calls",
            "max_input_tokens",
            "max_output_tokens",
            "max_tokens",
            "temperature",
            "top_p",
            "reasoning_effort",
            "reasoning_selection",
            "reasoning_tier",
            "permission_profile",
            "permission_rules",
            "initial_items",
            "automation_execution_spec",
            "system_prompt",
            "model_id",
            "model",
            "provider",
            "wire_protocol",
            "host_catalog",
            "tools",
            "context_components",
            "budget",
            "limits",
            "timeout_ms",
            "stream",
            "metadata",
            "memory_policy",
            "compaction_policy",
            "execution_policy",
            "policy",
            "skills",
            "data_sources",
            "workspace_root",
            "locale",
            "language",
        ];
        for key in cfg_obj.keys() {
            if !ALLOWED_CORE_CONFIG_FIELDS.contains(&key.as_str()) {
                return Err((
                    "invalid_command",
                    format!("invalid core_config: unknown field `{key}`"),
                ));
            }
        }
    }
    let submission = input
        .get("submission")
        .ok_or(("invalid_command", "missing 'submission'".into()))?;
    let run_id = submission
        .get("submission_id")
        .or_else(|| input.get("run_id"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(new_id);
    let user_msg_id = input
        .get("user_message_id")
        .and_then(|v| v.as_str())
        .map(ToString::to_string)
        .unwrap_or_else(|| format!("{run_id}:user"));
    let assistant_msg_id = input
        .get("assistant_message_id")
        .and_then(|v| v.as_str())
        .map(ToString::to_string)
        .unwrap_or_else(|| format!("{run_id}:assistant"));
    let user_text = input
        .get("user_text")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let created_at_ms = input
        .get("created_at_ms")
        .and_then(|v| v.as_i64())
        .unwrap_or_else(now_ms);
    let conv_title = input
        .get("conversation_title")
        .and_then(|v| v.as_str())
        .unwrap_or("Conversation");

    if store.get_session(conv_id).ok().flatten().is_none() {
        let sess = Session {
            id: conv_id.to_string(),
            title: conv_title.to_string(),
            workspace_root: suite.workspace_root.display().to_string(),
            model_connection: None,
            model_id: None,
            created_at_ms,
            updated_at_ms: created_at_ms,
        };
        store
            .upsert_session(&sess)
            .map_err(|e| ("storage_error", e.to_string()))?;
    }
    let _ = store.append_event(
        conv_id,
        "user_message",
        json!({
            "id": user_msg_id,
            "role": "user",
            "content": user_text,
            "created_at_ms": created_at_ms
        }),
        50_000,
    );

    let inv_digest = sha256_hex(canonical_json_value(core_config).to_string().as_bytes());
    let host_catalog = core_config.get("host_catalog");
    let host_id = host_catalog
        .and_then(|h| h.get("host_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("local-host");
    let catalog_rev = host_catalog
        .and_then(|h| h.get("revision"))
        .and_then(|v| v.as_i64())
        .unwrap_or(1);
    let availability_epoch = host_catalog
        .and_then(|h| h.get("availability_epoch"))
        .and_then(|v| v.as_i64())
        .unwrap_or(1);
    let session_id = core_config
        .get("session_id")
        .and_then(|v| v.as_str())
        .unwrap_or(conv_id);
    let turn_id = run_id.clone();
    let req_id = new_id();
    let attempt_id = new_id();
    let effect_id = new_id();
    let initial_request = json!({
        "request_id": req_id,
        "attempt_id": attempt_id,
        "attempt_started": false,
        "delegated_invocation_id": Value::Null,
        "effect_id": effect_id,
        "host_id": host_id,
        "lease_id": Value::Null,
        "session_id": session_id,
        "turn_id": turn_id,
        "catalog_revision": catalog_rev,
        "availability_epoch": availability_epoch,
        "delivery": Value::Null,
        "payload": build_model_stream_payload(
            core_config,
            json!([{"role": "user", "content": user_text}]),
            &assistant_msg_id,
        )
    });
    let journal_id = journal_identity_value(store, conv_id, &run_id, &assistant_msg_id);
    let admission_digest = sha256_hex(
        format!("admission:{conv_id}:{run_id}:{user_msg_id}:{assistant_msg_id}").as_bytes(),
    );
    let admission_cursor = json!({
        "identity": journal_id,
        "sequence": "1",
        "record_digest": admission_digest
    });
    let admission_record = json!({
        "sequence": "1",
        "identity": journal_id,
        "user_message_id": user_msg_id,
        "assistant_message_id": assistant_msg_id,
        "user_text": user_text,
        "created_at_ms": created_at_ms,
        "record_digest": admission_digest
    });
    let _ = store.set_kv(&format!("run_admission_head:{conv_id}:{run_id}"), &admission_cursor);
    let _ = store.set_kv(
        &format!("run_admissions:{conv_id}:{run_id}"),
        &json!([admission_record]),
    );
    if let Ok(Some(Value::String(sub_key))) =
        store.get_kv(&format!("source_submission_by_run:{conv_id}:{run_id}"))
    {
        if let Ok(Some(mut sub_rec)) = store.get_kv(&sub_key) {
            sub_rec["phase"] = json!("admitted");
            sub_rec["admission"] = admission_cursor.clone();
            let _ = store.set_kv(&sub_key, &sub_rec);
        }
    }

    let host_invocation = core_config
        .pointer("/turn_identity/host_invocation")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let context_budget = core_config
        .get("context_window")
        .cloned()
        .unwrap_or_else(|| json!({"context_window_tokens": 128_000, "reserved_output_tokens": 4_096}));
    let checkpoint_sha256 = sha256_hex(format!("ckpt:{conv_id}:{run_id}:{inv_digest}").as_bytes());
    let _ = store.set_kv(
        &format!("exact_run_invocation:{conv_id}:{run_id}"),
        &json!({
            "conversation_id": conv_id,
            "run_id": run_id,
            "assistant_message_id": assistant_msg_id,
            "invocation_config_digest": inv_digest,
            "status": "running",
            "core_session_id": session_id,
            "host_invocation": host_invocation,
            "context_budget": context_budget,
            "checkpoint_sha256": checkpoint_sha256,
            "checkpoint_revision": "1",
            "checkpoint_event_cursor": "1"
        }),
    );

    let automation_occ_id = args
        .pointer("/lease/occurrence_id")
        .or_else(|| input.pointer("/lease/occurrence_id"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(ToString::to_string);
    if let Some(ref occ_id) = automation_occ_id {
        let def_rev = args
            .pointer("/lease/definition_revision")
            .or_else(|| input.pointer("/lease/definition_revision"))
            .and_then(|v| v.as_i64())
            .unwrap_or(1);
        let akey = format!("automation_exec_state:{occ_id}");
        let mut obs = store
            .get_kv(&akey)
            .ok()
            .flatten()
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default();
        obs.insert(
            "binding".into(),
            json!({
                "occurrence_id": occ_id,
                "definition_revision": def_rev,
                "conversation_id": conv_id,
                "run_id": run_id,
                "assistant_message_id": assistant_msg_id
            }),
        );
        let _ = store.set_kv(&akey, &Value::Object(obs));
    }

    let run_record = json!({
        "run_id": run_id,
        "conversation_id": conv_id,
        "automation_occurrence_id": automation_occ_id,
        "user_message_id": user_msg_id,
        "user_text": user_text,
        "assistant_message_id": assistant_msg_id,
        "invocation_config_digest": inv_digest,
        "core_config": core_config,
        "host_id": host_id,
        "session_id": session_id,
        "turn_id": turn_id,
        "status": "running",
        "turn_status": "running",
        "sequence": 1u64,
        "checkpoint_revision": "1",
        "checkpoint_event_cursor": "1",
        "permission_projection_revision": 1u64,
        "pending_permissions": [],
        "requests": [initial_request],
        "consumed_request_ids": [],
        "journal_identity": journal_id,
        "journal_seq": 0u64,
        "journal_reasoning_text": "",
        "assistant_text": "",
        "tool_parts": {},
        "created_at_ms": created_at_ms
    });
    store
        .set_kv(&format!("committed_run:{run_id}"), &run_record)
        .map_err(|e| ("storage_error", e.to_string()))?;
    let _ = store.set_kv(
        &format!("run_identity:{run_id}"),
        &json!({
            "run_id": run_id,
            "conversation_id": conv_id,
            "user_message_id": user_msg_id,
            "assistant_message_id": assistant_msg_id,
            "host_id": host_id,
            "invocation_config_digest": inv_digest
        }),
    );
    register_active_run(
        store,
        conv_id,
        json!({
            "conversation_id": conv_id,
            "run_id": run_id,
            "kind": "local",
            "assistant_message_id": assistant_msg_id,
            "next_event_sequence": 1i64,
            "invocation_config_digest": inv_digest,
            "status": "running"
        }),
    );
    Ok(json!({
        "run_id": run_id,
        "conversation_id": conv_id,
        "user_message_id": user_msg_id,
        "assistant_message_id": assistant_msg_id,
        "invocation_config_digest": inv_digest,
        "status": "running",
        "events": [],
        "traces": [],
        "execution": {
            "control": {
                "session_id": session_id,
                "revision": 1u64,
                "active_turn_id": turn_id,
                "turn_status": "running",
                "permission_projection_revision": 1u64,
                "last_event_cursor": {"sequence": 1u64}
            },
            "permissions": {
                "revision": 1u64,
                "pending": []
            }
        }
    }))
}

fn dispatch_claim_committed_host_requests(
    store: &Store,
    args: &Value,
) -> Result<Value, (&'static str, String)> {
    let input = args.get("input").unwrap_or(args);
    let run_id = input
        .get("run_id")
        .and_then(|v| v.as_str())
        .ok_or(("invalid_argument", "missing 'run_id'".into()))?;
    let worker_gen = input
        .get("worker_generation")
        .and_then(|v| v.as_str())
        .unwrap_or("worker");
    let lease_owner = input
        .get("lease_owner")
        .and_then(|v| v.as_str())
        .unwrap_or("owner");
    let claimed_at = input
        .get("claimed_at_ms")
        .and_then(|v| v.as_i64())
        .unwrap_or_else(now_ms);
    let expires_at = input
        .get("lease_expires_at_ms")
        .and_then(|v| v.as_i64())
        .unwrap_or(claimed_at + 240_000);
    let limit = input.get("limit").and_then(|v| v.as_u64()).unwrap_or(64) as usize;
    let key = format!("committed_run:{run_id}");
    let mut run = store
        .get_kv(&key)
        .map_err(|e| ("storage_error", e.to_string()))?
        .ok_or(("not_found", format!("committed run '{run_id}' not found")))?;
    let consumed: HashSet<String> = run
        .get("consumed_request_ids")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(ToString::to_string))
                .collect()
        })
        .unwrap_or_default();
    let mut claimed_out = Vec::new();
    if let Some(reqs) = run.get_mut("requests").and_then(|v| v.as_array_mut()) {
        for r in reqs.iter_mut() {
            if claimed_out.len() >= limit {
                break;
            }
            let rid = r.get("request_id").and_then(|v| v.as_str()).unwrap_or("");
            if consumed.contains(rid) {
                continue;
            }
            // Preserve existing active delivery lease for the same worker so repeated claims
            // never rebind an in-flight Host effect without a recovery boundary.
            if let Some(existing_del) = r.get("delivery").and_then(|v| v.as_object()) {
                let existing_exp = existing_del
                    .get("lease_expires_at_ms")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0);
                if existing_exp > claimed_at {
                    let same_worker = existing_del.get("worker_generation").and_then(|v| v.as_str())
                        == Some(worker_gen)
                        && existing_del.get("lease_owner").and_then(|v| v.as_str())
                            == Some(lease_owner);
                    if same_worker {
                        claimed_out.push(r.clone());
                    }
                    continue;
                }
            }
            let lease_token = new_id();
            let attempt_id = r
                .get("attempt_id")
                .and_then(|v| v.as_str())
                .map(ToString::to_string)
                .unwrap_or_else(new_id);
            let delivery = json!({
                "attempt_id": attempt_id,
                "worker_generation": worker_gen,
                "lease_owner": lease_owner,
                "lease_token": lease_token,
                "claimed_at_ms": claimed_at,
                "lease_expires_at_ms": expires_at
            });
            if let Some(obj) = r.as_object_mut() {
                obj.insert("attempt_id".into(), json!(attempt_id));
                obj.insert("lease_id".into(), json!(lease_token));
                obj.insert("delivery".into(), delivery);
            }
            claimed_out.push(r.clone());
        }
    }
    store
        .set_kv(&key, &run)
        .map_err(|e| ("storage_error", e.to_string()))?;
    Ok(json!({
        "run_id": run_id,
        "requests": claimed_out
    }))
}

fn dispatch_load_committed_host_requests(
    store: &Store,
    args: &Value,
) -> Result<Value, (&'static str, String)> {
    let run_id = args
        .get("run_id")
        .or_else(|| args.get("input").and_then(|i| i.get("run_id")))
        .and_then(|v| v.as_str())
        .ok_or(("invalid_argument", "missing 'run_id'".into()))?;
    let run = store
        .get_kv(&format!("committed_run:{run_id}"))
        .map_err(|e| ("storage_error", e.to_string()))?;
    let consumed: HashSet<String> = run
        .as_ref()
        .and_then(|r| r.get("consumed_request_ids"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(ToString::to_string))
                .collect()
        })
        .unwrap_or_default();
    let reqs: Vec<Value> = run
        .as_ref()
        .and_then(|r| r.get("requests"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|r| {
            let rid = r.get("request_id").and_then(|v| v.as_str()).unwrap_or("");
            !consumed.contains(rid)
        })
        .collect();
    Ok(json!({
        "run_id": run_id,
        "requests": reqs
    }))
}

fn dispatch_reduce_and_accept_host_response(
    store: &Store,
    args: &Value,
) -> Result<Value, (&'static str, String)> {
    let input = args.get("input").unwrap_or(args);
    let run_id = input
        .get("run_id")
        .and_then(|v| v.as_str())
        .ok_or(("invalid_argument", "missing 'run_id'".into()))?;
    let conv_id = input
        .get("conversation_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let response = input
        .get("response")
        .ok_or(("invalid_argument", "missing 'response'".into()))?;
    let key = format!("committed_run:{run_id}");
    let mut run = store
        .get_kv(&key)
        .map_err(|e| ("storage_error", e.to_string()))?
        .ok_or(("not_found", format!("committed run '{run_id}' not found")))?;
    let eff_conv = if !conv_id.is_empty() {
        conv_id.to_string()
    } else {
        run.get("conversation_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    };
    let req_id_str = response
        .get("request_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let seq = run
        .get("sequence")
        .and_then(|v| v.as_u64())
        .unwrap_or(0)
        + 1;
    let already_terminal = matches!(
        run.get("status").and_then(|v| v.as_str()),
        Some("completed" | "finished" | "failed" | "cancelled")
    );
    let payload = response.get("payload").cloned().unwrap_or(Value::Null);
    let ev_type = payload.get("type").and_then(|v| v.as_str()).unwrap_or("");
    let session_id = run
        .get("session_id")
        .and_then(|v| v.as_str())
        .unwrap_or(&eff_conv)
        .to_string();
    let turn_id = run
        .get("turn_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let assistant_msg_id = run
        .get("assistant_message_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let host_id = run
        .get("host_id")
        .and_then(|v| v.as_str())
        .unwrap_or("local-host")
        .to_string();

    if already_terminal {
        let perm_rev = run
            .get("permission_projection_revision")
            .and_then(|v| v.as_u64())
            .unwrap_or(1);
        let cur_seq = run.get("sequence").and_then(|v| v.as_u64()).unwrap_or(seq);
        return Ok(json!({
            "events": [],
            "traces": [],
            "control": {
                "session_id": session_id,
                "revision": cur_seq,
                "active_turn_id": Value::Null,
                "permission_projection_revision": perm_rev,
                "last_event_cursor": {"sequence": cur_seq}
            }
        }));
    }

    run["sequence"] = json!(seq);
    let mut is_terminal = false;
    let mut publications: Vec<Value> = Vec::new();
    match ev_type {
        "model_attempt_started" => {
            if !req_id_str.is_empty() {
                if let Some(obj) = run.as_object_mut() {
                    let arr = obj
                        .entry("started_attempt_request_ids".to_string())
                        .or_insert_with(|| json!([]));
                    if let Some(list) = arr.as_array_mut() {
                        if !list.iter().any(|v| v.as_str() == Some(req_id_str.as_str())) {
                            list.push(json!(req_id_str));
                        }
                    }
                }
            }
        }
        "completed" | "finished" | "incomplete" | "failed" | "cancelled" => {
            is_terminal = true;
            run["status"] = json!(ev_type);
            run["turn_status"] = json!(ev_type);
            if let Some(err) = payload.get("error") {
                run["error"] = err.clone();
            }
            if !req_id_str.is_empty() {
                if let Some(arr) = run.get_mut("consumed_request_ids").and_then(|v| v.as_array_mut()) {
                    arr.push(json!(req_id_str));
                }
            }
        }
        "text_delta" | "output_text_delta" | "model_committed_delta" | "model_delta" => {
            let channel = payload.get("channel").and_then(|v| v.as_str()).unwrap_or("text");
            if let Some(delta) = payload
                .get("delta")
                .or_else(|| payload.get("text"))
                .and_then(|v| v.as_str())
            {
                if channel == "reasoning" {
                    let mut r_acc = run
                        .get("journal_reasoning_text")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    r_acc.push_str(delta);
                    run["journal_reasoning_text"] = json!(r_acc);
                    let r_part_id = payload
                        .get("part_id")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string)
                        .unwrap_or_else(|| format!("{assistant_msg_id}:reasoning"));
                    run["journal_reasoning_part_id"] = json!(r_part_id);
                    let cursor = publish_journal_part(
                        store,
                        &mut run,
                        &r_part_id,
                        0,
                        "reasoning",
                        json!({
                            "state": "running",
                            "title": {"kind": "inline", "value": "Thinking"}
                        }),
                        &[("reasoning_summary", r_acc)],
                    );
                    publications.push(cursor);
                } else {
                    let mut acc = run
                        .get("assistant_text")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    acc.push_str(delta);
                    run["assistant_text"] = json!(acc);
                    let t_part_id = payload
                        .get("part_id")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string)
                        .unwrap_or_else(|| format!("{assistant_msg_id}:text"));
                    run["journal_text_part_id"] = json!(t_part_id);
                    let cursor = publish_journal_part(
                        store,
                        &mut run,
                        &t_part_id,
                        2,
                        "text",
                        json!({
                            "visibility": "public",
                            "is_final": false
                        }),
                        &[("markdown", acc)],
                    );
                    publications.push(cursor);
                }
            }
        }
        "tool_progress" => {
            let active_tool_id = run
                .get("active_tool_call_id")
                .and_then(|v| v.as_str())
                .map(ToString::to_string);
            if let Some(cid) = active_tool_id {
                let title = payload
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Running tool")
                    .to_string();
                let summary = payload
                    .get("detail")
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string);
                let tool_meta = run
                    .pointer(&format!("/tool_parts/{cid}"))
                    .cloned()
                    .unwrap_or_else(|| json!({"name": "tool", "arguments": {}}));
                let name = tool_meta.get("name").and_then(|v| v.as_str()).unwrap_or("tool");
                let arguments = tool_meta.get("arguments").cloned().unwrap_or_else(|| json!({}));
                let mut data = json!({
                    "tool_call_id": cid,
                    "capability_id": name,
                    "state": "running",
                    "execution_source": {"kind": "model"},
                    "title": {"kind": "inline", "value": title},
                    "origin": {"kind": "builtin"},
                    "artifacts": "tool_artifacts",
                    "affected_resources": {"kind": "inline", "value": []}
                });
                if let Some(s) = summary {
                    data["summary"] = json!({"kind": "inline", "value": s});
                }
                let input_body = json!({"protocol_version": 1, "value": arguments}).to_string();
                let artifacts_body = json!({"protocol_version": 1, "artifacts": []}).to_string();
                let cursor = publish_journal_part(
                    store,
                    &mut run,
                    &format!("tool:{cid}"),
                    1,
                    "tool",
                    data,
                    &[("tool_input", input_body), ("tool_artifacts", artifacts_body)],
                );
                publications.push(cursor);
            }
        }
        "permission_resolved" => {
            if !req_id_str.is_empty() {
                if let Some(arr) = run.get_mut("consumed_request_ids").and_then(|v| v.as_array_mut()) {
                    arr.push(json!(req_id_str));
                }
            }
            let perm_id = payload
                .get("request_id")
                .or_else(|| payload.get("permission_request_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let resolution = payload.get("resolution").unwrap_or(&payload);
            let decision = resolution
                .get("decision")
                .or_else(|| resolution.get("kind"))
                .or_else(|| resolution.get("outcome"))
                .and_then(|v| v.as_str())
                .unwrap_or("allow_once");
            let mut matched_pending: Option<Value> = None;
            if let Some(p_arr) = run.get_mut("pending_permissions").and_then(|v| v.as_array_mut()) {
                if let Some(pos) = p_arr.iter().position(|p| {
                    p.get("request_id").and_then(|v| v.as_str()) == Some(perm_id)
                        || p.get("permission_request_id").and_then(|v| v.as_str()) == Some(perm_id)
                }) {
                    matched_pending = Some(p_arr.remove(pos));
                }
            }
            let next_perm_rev = run
                .get("permission_projection_revision")
                .and_then(|v| v.as_u64())
                .unwrap_or(1)
                + 1;
            run["permission_projection_revision"] = json!(next_perm_rev);
            run["turn_status"] = json!("running");
            if let Some(pending) = matched_pending {
                if let Some(hreq_id) = pending.get("host_request_id").and_then(|v| v.as_str()) {
                    if let Some(arr) = run.get_mut("consumed_request_ids").and_then(|v| v.as_array_mut()) {
                        arr.push(json!(hreq_id));
                    }
                }
                let cap_id = pending.get("capability_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let call_id = pending.get("call_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let arguments = pending.get("arguments").cloned().unwrap_or_else(|| json!({}));
                let arguments_obj = if arguments.is_object() { arguments.clone() } else { json!({}) };
                let perm_profile = run
                    .pointer("/core_config/permission_profile")
                    .and_then(|v| v.as_str())
                    .filter(|s| matches!(*s, "cautious" | "accept_edits" | "allow_all"))
                    .unwrap_or("cautious")
                    .to_string();
                if decision.starts_with("allow") || decision == "approved" {
                    run["active_tool_call_id"] = json!(call_id);
                    if let Some(tp) = run.get_mut("tool_parts").and_then(|v| v.as_object_mut()) {
                        tp.insert(
                            call_id.clone(),
                            json!({"name": cap_id, "arguments": arguments}),
                        );
                    }
                    let host_req = json!({
                        "request_id": new_id(),
                        "attempt_id": new_id(),
                        "delegated_invocation_id": Value::Null,
                        "effect_id": new_id(),
                        "host_id": host_id,
                        "lease_id": Value::Null,
                        "session_id": session_id,
                        "turn_id": turn_id,
                        "catalog_revision": 1,
                        "availability_epoch": 1,
                        "delivery": Value::Null,
                        "payload": {
                            "type": "execute_tool",
                            "call_id": call_id,
                            "capability_id": cap_id,
                            "tool_name": cap_id,
                            "permission_profile": perm_profile,
                            "input": arguments_obj,
                            "arguments": arguments,
                            "grant": resolution.get("grant").cloned().unwrap_or(Value::Null)
                        }
                    });
                    if let Some(reqs) = run.get_mut("requests").and_then(|v| v.as_array_mut()) {
                        reqs.push(host_req);
                    }
                } else {
                    let user_text = run.get("user_text").and_then(|v| v.as_str()).unwrap_or("");
                    let follow_msgs = json!([
                        {"role": "user", "content": user_text},
                        {"role": "assistant", "content": format!("Permission denied for {cap_id}.")}
                    ]);
                    let next_payload = build_model_stream_payload(
                        run.get("core_config").unwrap_or(&Value::Null),
                        follow_msgs,
                        &assistant_msg_id,
                    );
                    let next_model_req = json!({
                        "request_id": new_id(),
                        "attempt_id": new_id(),
                        "attempt_started": false,
                        "delegated_invocation_id": Value::Null,
                        "effect_id": new_id(),
                        "host_id": host_id,
                        "lease_id": Value::Null,
                        "session_id": session_id,
                        "turn_id": turn_id,
                        "catalog_revision": 1,
                        "availability_epoch": 1,
                        "delivery": Value::Null,
                        "payload": next_payload
                    });
                    if let Some(reqs) = run.get_mut("requests").and_then(|v| v.as_array_mut()) {
                        reqs.push(next_model_req);
                    }
                }
            }
        }
        "model_completed" => {
            if !req_id_str.is_empty() {
                if let Some(arr) = run.get_mut("consumed_request_ids").and_then(|v| v.as_array_mut()) {
                    arr.push(json!(req_id_str));
                }
            }
            // Extract any text from non-reasoning assistant message items
            if let Some(items) = payload.get("items").and_then(|v| v.as_array()) {
                let mut text_parts = Vec::new();
                for item in items {
                    if item.get("type").and_then(|v| v.as_str()) == Some("reasoning") {
                        continue;
                    }
                    if let Some(t) = item
                        .get("text")
                        .or_else(|| item.get("content"))
                        .and_then(|v| v.as_str())
                    {
                        if !t.is_empty() {
                            text_parts.push(t.to_string());
                        }
                    }
                }
                if !text_parts.is_empty() {
                    run["assistant_text"] = json!(text_parts.join("\n"));
                }
            }
            let tool_calls = payload
                .get("tool_calls")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            if tool_calls.is_empty() {
                is_terminal = true;
                run["status"] = json!("completed");
                run["turn_status"] = json!("completed");
                let mut final_reasoning = String::new();
                if let Some(r_pid) = run
                    .get("journal_reasoning_part_id")
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string)
                {
                    let r_text = run
                        .get("journal_reasoning_text")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    final_reasoning = r_text.clone();
                    let _ = publish_journal_part(
                        store,
                        &mut run,
                        &r_pid,
                        0,
                        "reasoning",
                        json!({
                            "state": "completed",
                            "title": {"kind": "inline", "value": "Thinking"}
                        }),
                        &[("reasoning_summary", r_text)],
                    );
                }
                let final_text = run
                    .get("assistant_text")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let t_part_id = run
                    .get("journal_text_part_id")
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string)
                    .unwrap_or_else(|| format!("{assistant_msg_id}:text"));
                let cursor = publish_journal_part(
                    store,
                    &mut run,
                    &t_part_id,
                    2,
                    "text",
                    json!({
                        "visibility": "public",
                        "is_final": true
                    }),
                    &[("markdown", final_text.clone())],
                );
                if !final_reasoning.is_empty() {
                    run["journal_reasoning_text"] = json!(final_reasoning);
                }
                publications.push(cursor);
            } else {
                // Enqueue either request_permission or execute_tool for each tool call.
                let caps = run
                    .pointer("/core_config/host_catalog/capabilities")
                    .and_then(|v| v.as_object())
                    .cloned()
                    .unwrap_or_default();
                let rules = run
                    .pointer("/core_config/permission_rules/rules")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default();
                let saved_grants = store
                    .get_kv(&format!("conv_tool_grants:{eff_conv}"))
                    .ok()
                    .flatten()
                    .and_then(|v| v.as_object().cloned())
                    .unwrap_or_default();
                for tc in tool_calls {
                    let call_id = tc
                        .get("call_id")
                        .or_else(|| tc.get("id"))
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string)
                        .unwrap_or_else(new_id);
                    let name = tc
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let arguments = tc.get("arguments").cloned().unwrap_or_else(|| json!({}));
                    let cap = caps.get(&name);
                    let perm_domain = cap
                        .and_then(|c| c.pointer("/input_normalization/permission_contract/domain"))
                        .or_else(|| cap.and_then(|c| c.get("permission_domain")))
                        .and_then(|v| v.as_str());
                    let rule_asks = rules.iter().any(|r| {
                        let decision_ask = r.get("decision").and_then(|v| v.as_str()) == Some("ask");
                        let cap_match = r
                            .get("capability_pattern")
                            .or_else(|| r.get("capability_id"))
                            .or_else(|| r.get("tool"))
                            .or_else(|| r.get("tool_name"))
                            .or_else(|| r.get("matcher"))
                            .and_then(|v| v.as_str())
                            .is_some_and(|m| {
                                m == name || m == "*" || perm_domain.is_some_and(|d| m == d)
                            });
                        decision_ask && cap_match
                    });
                    let resource = arguments
                        .get("path")
                        .or_else(|| arguments.get("url"))
                        .or_else(|| arguments.get("uri"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let grant_key = format!("{name}:{resource}");
                    let saved_grant = saved_grants.get(&grant_key).cloned();
                    if (perm_domain.is_some() || rule_asks) && saved_grant.is_none() {
                        let perm_req_id = new_id();
                        let host_req_id = new_id();
                        let perm_effect_id = new_id();
                        let _ = store.set_kv(&format!("perm_request_run:{perm_req_id}"), &json!(run_id));
                        let _ = store.set_kv(&format!("effect_run:{perm_effect_id}"), &json!(run_id));
                        let next_perm_rev = run
                            .get("permission_projection_revision")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(1)
                            + 1;
                        run["permission_projection_revision"] = json!(next_perm_rev);
                        run["turn_status"] = json!("awaiting_permission");
                        let rationale = format!("Permission required for {name}");
                        let perm_domain_str = perm_domain.unwrap_or(&name);
                        let pending_entry = json!({
                            "allow_conversation": true,
                            "request_id": perm_req_id,
                            "permission_request_id": perm_req_id,
                            "host_request_id": host_req_id,
                            "capability_id": name,
                            "resource": resource,
                            "call_id": call_id,
                            "turn_id": turn_id,
                            "arguments": arguments,
                            "permission_domain": perm_domain_str,
                            "rationale": rationale,
                            "evaluation": {
                                "decision": "ask",
                                "capability": perm_domain_str,
                                "resource": resource
                            },
                            "permission_instructions": []
                        });
                        if let Some(p_arr) = run.get_mut("pending_permissions").and_then(|v| v.as_array_mut()) {
                            p_arr.push(pending_entry);
                        }
                        let host_req = json!({
                            "request_id": host_req_id,
                            "attempt_id": new_id(),
                            "delegated_invocation_id": Value::Null,
                            "effect_id": perm_effect_id,
                            "host_id": host_id,
                            "lease_id": Value::Null,
                            "session_id": session_id,
                            "turn_id": turn_id,
                            "catalog_revision": 1,
                            "availability_epoch": 1,
                            "delivery": Value::Null,
                            "payload": {
                                "type": "request_permission",
                                "permission_request_id": perm_req_id,
                                "capability_id": name,
                                "resource": resource,
                                "reason": format!("Permission required for {name}"),
                                "allow_conversation": true,
                                "permission_instructions": [],
                                "call_id": call_id,
                                "arguments": arguments
                            }
                        });
                        if let Some(reqs) = run.get_mut("requests").and_then(|v| v.as_array_mut()) {
                            reqs.push(host_req);
                        }
                    } else {
                        run["active_tool_call_id"] = json!(call_id);
                        if let Some(tp) = run.get_mut("tool_parts").and_then(|v| v.as_object_mut()) {
                            tp.insert(
                                call_id.clone(),
                                json!({"name": name, "arguments": arguments}),
                            );
                        }
                        let input_body = json!({"protocol_version": 1, "value": arguments}).to_string();
                        let artifacts_body = json!({"protocol_version": 1, "artifacts": []}).to_string();
                        let cursor = publish_journal_part(
                            store,
                            &mut run,
                            &format!("tool:{call_id}"),
                            1,
                            "tool",
                            json!({
                                "tool_call_id": call_id,
                                "capability_id": name,
                                "state": "running",
                                "execution_source": {"kind": "model"},
                                "title": {"kind": "inline", "value": name},
                                "origin": {"kind": "builtin"},
                                "artifacts": "tool_artifacts",
                                "affected_resources": {"kind": "inline", "value": []}
                            }),
                            &[("tool_input", input_body), ("tool_artifacts", artifacts_body)],
                        );
                        publications.push(cursor);
                        let arguments_obj = if arguments.is_object() { arguments.clone() } else { json!({}) };
                        let perm_profile = run
                            .pointer("/core_config/permission_profile")
                            .and_then(|v| v.as_str())
                            .filter(|s| matches!(*s, "cautious" | "accept_edits" | "allow_all"))
                            .unwrap_or("cautious")
                            .to_string();
                        let host_req = json!({
                            "request_id": new_id(),
                            "attempt_id": new_id(),
                            "delegated_invocation_id": Value::Null,
                            "effect_id": new_id(),
                            "host_id": host_id,
                            "lease_id": Value::Null,
                            "session_id": session_id,
                            "turn_id": turn_id,
                            "catalog_revision": 1,
                            "availability_epoch": 1,
                            "delivery": Value::Null,
                            "payload": {
                                "type": "execute_tool",
                                "call_id": call_id,
                                "capability_id": name,
                                "tool_name": name,
                                "permission_profile": perm_profile,
                                "input": arguments_obj,
                                "arguments": arguments,
                                "grant": saved_grant.unwrap_or(Value::Null)
                            }
                        });
                        if let Some(reqs) = run.get_mut("requests").and_then(|v| v.as_array_mut()) {
                            reqs.push(host_req);
                        }
                    }
                }
            }
        }
        "tool_completed" => {
            if !req_id_str.is_empty() {
                if let Some(arr) = run.get_mut("consumed_request_ids").and_then(|v| v.as_array_mut()) {
                    arr.push(json!(req_id_str));
                }
            }
            let call_id = payload
                .get("call_id")
                .and_then(|v| v.as_str())
                .or_else(|| run.get("active_tool_call_id").and_then(|v| v.as_str()))
                .unwrap_or("tool-call")
                .to_string();
            let tool_meta = run
                .pointer(&format!("/tool_parts/{call_id}"))
                .cloned()
                .unwrap_or_else(|| json!({"name": "tool", "arguments": {}}));
            let name = tool_meta.get("name").and_then(|v| v.as_str()).unwrap_or("tool");
            let arguments = tool_meta.get("arguments").cloned().unwrap_or_else(|| json!({}));
            let output_val = payload.get("output").cloned().unwrap_or_else(|| json!({}));
            let preview_text = output_val
                .get("text")
                .or_else(|| output_val.get("content"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let artifacts = payload
                .get("artifacts")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            let affected: Vec<Value> = output_val
                .get("resources")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|r| {
                            let uri = r.get("uri").and_then(|v| v.as_str())?;
                            let op = r.get("operation").and_then(|v| v.as_str()).unwrap_or("read");
                            let rtype = r.get("resource_type").and_then(|v| v.as_str()).unwrap_or("file");
                            Some(json!({"resource_type": rtype, "uri": uri, "operation": op}))
                        })
                        .collect()
                })
                .unwrap_or_default();
            let input_body = json!({"protocol_version": 1, "value": arguments}).to_string();
            let output_body = json!({"protocol_version": 1, "value": output_val}).to_string();
            let artifacts_body = json!({"protocol_version": 1, "artifacts": artifacts}).to_string();
            let cursor = publish_journal_part(
                store,
                &mut run,
                &format!("tool:{call_id}"),
                1,
                "tool",
                json!({
                    "tool_call_id": call_id,
                    "capability_id": name,
                    "state": "completed",
                    "execution_source": {"kind": "model"},
                    "title": {"kind": "inline", "value": name},
                    "origin": {"kind": "builtin"},
                    "artifacts": "tool_artifacts",
                    "affected_resources": {"kind": "inline", "value": affected}
                }),
                &[
                    ("tool_input", input_body),
                    ("tool_output", output_body),
                    ("tool_output_text", preview_text.clone()),
                    ("tool_artifacts", artifacts_body),
                ],
            );
            publications.push(cursor);
            if let Some(obj) = run.as_object_mut() {
                let arr = obj
                    .entry("completed_tools".to_string())
                    .or_insert_with(|| json!([]));
                if let Some(list) = arr.as_array_mut() {
                    list.push(json!({
                        "id": call_id,
                        "call_id": call_id,
                        "name": name,
                        "arguments": arguments,
                        "is_error": false,
                        "summary": preview_text,
                        "output": output_val
                    }));
                }
            }
            let user_text = run.get("user_text").and_then(|v| v.as_str()).unwrap_or("");
            let follow_msgs = json!([
                {"role": "user", "content": user_text},
                {"role": "assistant", "content": format!("Tool {name} completed.")}
            ]);
            let next_payload = build_model_stream_payload(
                run.get("core_config").unwrap_or(&Value::Null),
                follow_msgs,
                &assistant_msg_id,
            );
            let next_model_req = json!({
                "request_id": new_id(),
                "attempt_id": new_id(),
                "attempt_started": false,
                "delegated_invocation_id": Value::Null,
                "effect_id": new_id(),
                "host_id": host_id,
                "lease_id": Value::Null,
                "session_id": session_id,
                "turn_id": turn_id,
                "catalog_revision": 1,
                "availability_epoch": 1,
                "delivery": Value::Null,
                "payload": next_payload
            });
            if let Some(reqs) = run.get_mut("requests").and_then(|v| v.as_array_mut()) {
                reqs.push(next_model_req);
            }
        }
        _ => {}
    }
    if is_terminal && !eff_conv.is_empty() {
        clear_active_run(store, &eff_conv);
        let text = run
            .get("assistant_text")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let reasoning = run
            .get("journal_reasoning_text")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let completed_tools = run
            .get("completed_tools")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let _ = store.append_event(
            &eff_conv,
            "assistant_message",
            json!({
                "id": assistant_msg_id,
                "message_id": assistant_msg_id,
                "role": "assistant",
                "content": text,
                "reasoning": reasoning,
                "toolCalls": completed_tools,
                "status": run["status"]
            }),
            now_ms(),
        );
        let t_part_id = run
            .get("journal_text_part_id")
            .and_then(|v| v.as_str())
            .map(ToString::to_string)
            .unwrap_or_else(|| format!("{assistant_msg_id}:text"));
        let seal = build_and_store_terminal_seal(store, &run, &t_part_id);
        let _ = store.set_kv(
            &format!("run_terminal:{run_id}"),
            &json!({
                "conversation_id": eff_conv,
                "run_id": run_id,
                "assistant_message_id": assistant_msg_id,
                "status": "sealed",
                "result": seal["result"],
                "result_digest": seal["result_digest"],
                "completed_at_ms": seal["completed_at_ms"]
            }),
        );
    }
    store
        .set_kv(&key, &run)
        .map_err(|e| ("storage_error", e.to_string()))?;
    let perm_rev = run
        .get("permission_projection_revision")
        .and_then(|v| v.as_u64())
        .unwrap_or(1);
    let turn_status = run
        .get("turn_status")
        .and_then(|v| v.as_str())
        .unwrap_or(if is_terminal { "completed" } else { "running" });
    Ok(json!({
        "events": [],
        "traces": [],
        "assistant_publications": publications,
        "control": {
            "session_id": session_id,
            "revision": seq,
            "active_turn_id": if is_terminal { Value::Null } else { json!(turn_id) },
            "turn_status": turn_status,
            "permission_projection_revision": perm_rev,
            "last_event_cursor": {"sequence": seq}
        },
        "ok": true,
        "accepted": true,
        "run_id": run_id,
        "conversation_id": eff_conv,
        "sequence": seq,
        "terminal": is_terminal,
        "status": run["status"]
    }))
}

fn dispatch_reduce_and_commit_active_submission(
    store: &Store,
    args: &Value,
) -> Result<Value, (&'static str, String)> {
    let input = args.get("input").unwrap_or(args);
    let run_id = input
        .get("run_id")
        .and_then(|v| v.as_str())
        .ok_or(("invalid_argument", "missing 'run_id'".into()))?;
    let submission = input
        .get("submission")
        .ok_or(("invalid_argument", "missing 'submission'".into()))?;
    let key = format!("committed_run:{run_id}");
    let mut run = store
        .get_kv(&key)
        .map_err(|e| ("storage_error", e.to_string()))?
        .ok_or(("not_found", format!("committed run '{run_id}' not found")))?;
    let conv_id = run
        .get("conversation_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let session_id = run
        .get("session_id")
        .and_then(|v| v.as_str())
        .unwrap_or(&conv_id)
        .to_string();
    let turn_id = run
        .get("turn_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let host_id = run
        .get("host_id")
        .and_then(|v| v.as_str())
        .unwrap_or("local-host")
        .to_string();
    let sub_type = submission.get("type").and_then(|v| v.as_str()).unwrap_or("");
    let next_seq = run.get("sequence").and_then(|v| v.as_u64()).unwrap_or(1) + 1;
    run["sequence"] = json!(next_seq);

    if sub_type == "permission_response" {
        let req_id = submission
            .get("request_id")
            .or_else(|| submission.get("permission_request_id"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let decision = submission
            .get("decision")
            .and_then(|v| v.as_str())
            .unwrap_or("deny");
        let grant = submission.get("grant").cloned().unwrap_or(Value::Null);
        let mut matched_pending: Option<Value> = None;
        if let Some(p_arr) = run.get_mut("pending_permissions").and_then(|v| v.as_array_mut()) {
            if let Some(pos) = p_arr.iter().position(|p| {
                p.get("request_id").and_then(|v| v.as_str()) == Some(req_id)
                    || p.get("permission_request_id").and_then(|v| v.as_str()) == Some(req_id)
            }) {
                matched_pending = Some(p_arr.remove(pos));
            }
        }
        let next_perm_rev = run
            .get("permission_projection_revision")
            .and_then(|v| v.as_u64())
            .unwrap_or(1)
            + 1;
        run["permission_projection_revision"] = json!(next_perm_rev);
        run["turn_status"] = json!("running");
        if let Some(pending) = matched_pending {
            if let Some(hreq_id) = pending.get("host_request_id").and_then(|v| v.as_str()) {
                if let Some(arr) = run
                    .get_mut("consumed_request_ids")
                    .and_then(|v| v.as_array_mut())
                {
                    arr.push(json!(hreq_id));
                }
            }
            let cap_id = pending
                .get("capability_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let resource = pending
                .get("resource")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let call_id = pending
                .get("call_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let arguments = pending.get("arguments").cloned().unwrap_or_else(|| json!({}));
            if decision == "allow_conversation" && !conv_id.is_empty() {
                let gkey = format!("conv_tool_grants:{conv_id}");
                let mut gmap = store
                    .get_kv(&gkey)
                    .ok()
                    .flatten()
                    .and_then(|v| v.as_object().cloned())
                    .unwrap_or_default();
                gmap.insert(format!("{cap_id}:{resource}"), grant.clone());
                let _ = store.set_kv(&gkey, &Value::Object(gmap));
            }
            if decision.starts_with("allow") {
                let host_req = json!({
                    "request_id": new_id(),
                    "attempt_id": new_id(),
                    "delegated_invocation_id": Value::Null,
                    "effect_id": new_id(),
                    "host_id": host_id,
                    "lease_id": Value::Null,
                    "session_id": session_id,
                    "turn_id": turn_id,
                    "catalog_revision": 1,
                    "availability_epoch": 1,
                    "delivery": Value::Null,
                    "payload": {
                        "type": "execute_tool",
                        "call_id": call_id,
                        "tool_name": cap_id,
                        "arguments": arguments,
                        "grant": grant
                    }
                });
                if let Some(reqs) = run.get_mut("requests").and_then(|v| v.as_array_mut()) {
                    reqs.push(host_req);
                }
            }
        }
    }
    store
        .set_kv(&key, &run)
        .map_err(|e| ("storage_error", e.to_string()))?;
    let perm_rev = run
        .get("permission_projection_revision")
        .and_then(|v| v.as_u64())
        .unwrap_or(1);
    Ok(json!({
        "events": [],
        "traces": [],
        "control": {
            "session_id": session_id,
            "active_turn_id": turn_id,
            "turn_status": run.get("turn_status").and_then(|v| v.as_str()).unwrap_or("running"),
            "permission_projection_revision": perm_rev,
            "last_event_cursor": {"sequence": next_seq}
        }
    }))
}

fn dispatch_local_run_message_admission_head(
    store: &Store,
    args: &Value,
) -> Result<Value, (&'static str, String)> {
    let conv_id = args
        .get("conversation_id")
        .and_then(|v| v.as_str())
        .ok_or(("invalid_argument", "missing 'conversation_id'".into()))?;
    let run_id = args
        .get("run_id")
        .and_then(|v| v.as_str())
        .ok_or(("invalid_argument", "missing 'run_id'".into()))?;
    if let Ok(Some(head)) = store.get_kv(&format!("run_admission_head:{conv_id}:{run_id}")) {
        return Ok(head);
    }
    let identity = journal_identity_value(store, conv_id, run_id, &format!("{run_id}:assistant"));
    Ok(json!({
        "identity": identity,
        "sequence": "0",
        "record_digest": "0".repeat(64)
    }))
}

fn dispatch_read_run_message_admissions(
    store: &Store,
    args: &Value,
) -> Result<Value, (&'static str, String)> {
    let after = args
        .get("after")
        .cloned()
        .ok_or(("invalid_argument", "missing 'after'".into()))?;
    let through = args
        .get("through")
        .cloned()
        .ok_or(("invalid_argument", "missing 'through'".into()))?;
    let conv_id = through
        .pointer("/identity/conversation_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let run_id = through
        .pointer("/identity/run_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let after_seq = after
        .get("sequence")
        .and_then(|v| v.as_str().and_then(|s| s.parse::<u64>().ok()).or_else(|| v.as_u64()))
        .unwrap_or(0);
    let through_seq = through
        .get("sequence")
        .and_then(|v| v.as_str().and_then(|s| s.parse::<u64>().ok()).or_else(|| v.as_u64()))
        .unwrap_or(0);
    let max_records = args.get("max_records").and_then(|v| v.as_u64()).unwrap_or(64) as usize;
    let all = store
        .get_kv(&format!("run_admissions:{conv_id}:{run_id}"))
        .ok()
        .flatten()
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();
    let records: Vec<Value> = all
        .into_iter()
        .filter(|r| {
            let s = r
                .get("sequence")
                .and_then(|v| v.as_str().and_then(|x| x.parse::<u64>().ok()).or_else(|| v.as_u64()))
                .unwrap_or(0);
            s > after_seq && s <= through_seq
        })
        .take(max_records)
        .collect();
    Ok(json!({
        "after": after,
        "through": through,
        "next": through,
        "complete": true,
        "records": records
    }))
}

fn dispatch_run_message_admission_reception_state(
    store: &Store,
    args: &Value,
) -> Result<Value, (&'static str, String)> {
    let source_host = args
        .get("source_host_id")
        .or_else(|| args.get("sourceHostID"))
        .and_then(|v| v.as_str())
        .unwrap_or("remote");
    let through = args
        .get("through")
        .cloned()
        .ok_or(("invalid_argument", "missing 'through'".into()))?;
    let conv_id = through.pointer("/identity/conversation_id").and_then(|v| v.as_str()).unwrap_or("");
    let run_id = through.pointer("/identity/run_id").and_then(|v| v.as_str()).unwrap_or("");
    let key = format!("received_admission:{source_host}:{conv_id}:{run_id}");
    if let Ok(Some(cur)) = store.get_kv(&key) {
        return Ok(cur);
    }
    Ok(json!({
        "identity": through.get("identity").cloned().unwrap_or(Value::Null),
        "sequence": "0",
        "record_digest": "0".repeat(64)
    }))
}

fn dispatch_receive_run_message_admissions(
    store: &Store,
    args: &Value,
) -> Result<Value, (&'static str, String)> {
    let source_host = args
        .get("source_host_id")
        .or_else(|| args.get("sourceHostID"))
        .and_then(|v| v.as_str())
        .unwrap_or("remote");
    let page = args
        .get("page")
        .ok_or(("invalid_argument", "missing 'page'".into()))?;
    let next = page
        .get("next")
        .cloned()
        .ok_or(("invalid_argument", "missing 'page.next'".into()))?;
    let conv_id = next.pointer("/identity/conversation_id").and_then(|v| v.as_str()).unwrap_or("");
    let run_id = next.pointer("/identity/run_id").and_then(|v| v.as_str()).unwrap_or("");
    let key = format!("received_admission:{source_host}:{conv_id}:{run_id}");
    store
        .set_kv(&key, &next)
        .map_err(|e| ("storage_error", e.to_string()))?;
    Ok(next)
}

fn dispatch_local_run_assistant_journal_heads(
    store: &Store,
    args: &Value,
) -> Result<Value, (&'static str, String)> {
    let conv_id = args
        .get("conversation_id")
        .and_then(|v| v.as_str())
        .ok_or(("invalid_argument", "missing 'conversation_id'".into()))?;
    let run_id = args
        .get("run_id")
        .and_then(|v| v.as_str())
        .ok_or(("invalid_argument", "missing 'run_id'".into()))?;
    let mut heads = Vec::new();
    if let Ok(Some(cur)) = store.get_kv(&format!("journal_head:{conv_id}:{run_id}")) {
        if !cur.is_null() {
            heads.push(cur);
        }
    }
    Ok(Value::Array(heads))
}

fn dispatch_local_assistant_journal_head(
    store: &Store,
    args: &Value,
) -> Result<Value, (&'static str, String)> {
    let identity = args
        .get("identity")
        .ok_or(("invalid_argument", "missing 'identity'".into()))?;
    let conv_id = identity.get("conversation_id").and_then(|v| v.as_str()).unwrap_or("");
    let run_id = identity.get("run_id").and_then(|v| v.as_str()).unwrap_or("");
    if let Ok(Some(cur)) = store.get_kv(&format!("journal_head:{conv_id}:{run_id}")) {
        return Ok(cur);
    }
    Ok(json!({
        "identity": identity,
        "sequence": "0",
        "record_digest": "0".repeat(64)
    }))
}

fn dispatch_open_assistant_journal_snapshot(args: &Value) -> Result<Value, (&'static str, String)> {
    let at = args
        .get("at")
        .cloned()
        .ok_or(("invalid_argument", "missing 'at'".into()))?;
    Ok(json!({
        "snapshot_id": new_id(),
        "at": at
    }))
}

fn dispatch_read_assistant_journal(
    store: &Store,
    args: &Value,
) -> Result<Value, (&'static str, String)> {
    let after = args
        .get("after")
        .cloned()
        .ok_or(("invalid_argument", "missing 'after'".into()))?;
    let through = args
        .get("through")
        .cloned()
        .ok_or(("invalid_argument", "missing 'through'".into()))?;
    let conv_id = through.pointer("/identity/conversation_id").and_then(|v| v.as_str()).unwrap_or("");
    let run_id = through.pointer("/identity/run_id").and_then(|v| v.as_str()).unwrap_or("");
    let after_seq = after
        .get("sequence")
        .and_then(|v| v.as_str().and_then(|s| s.parse::<u64>().ok()).or_else(|| v.as_u64()))
        .unwrap_or(0);
    let through_seq = through
        .get("sequence")
        .and_then(|v| v.as_str().and_then(|s| s.parse::<u64>().ok()).or_else(|| v.as_u64()))
        .unwrap_or(0);
    let max_records = args.get("max_records").and_then(|v| v.as_u64()).unwrap_or(32) as usize;
    let all = store
        .get_kv(&format!("journal_records:{conv_id}:{run_id}"))
        .ok()
        .flatten()
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();
    let records: Vec<Value> = all
        .into_iter()
        .filter(|r| {
            let s = r
                .pointer("/header/sequence")
                .and_then(|v| v.as_str().and_then(|x| x.parse::<u64>().ok()).or_else(|| v.as_u64()))
                .unwrap_or(0);
            s > after_seq && s <= through_seq
        })
        .take(max_records)
        .collect();
    Ok(json!({
        "status": "records",
        "after": after,
        "through": through,
        "records": records,
        "next": through,
        "complete": true
    }))
}

fn dispatch_read_assistant_journal_record_chunk(
    store: &Store,
    args: &Value,
) -> Result<Value, (&'static str, String)> {
    let identity = args
        .pointer("/snapshot/at/identity")
        .or_else(|| args.get("identity"));
    let conv_id = identity.and_then(|i| i.get("conversation_id")).and_then(|v| v.as_str()).unwrap_or("");
    let run_id = identity.and_then(|i| i.get("run_id")).and_then(|v| v.as_str()).unwrap_or("");
    let part_id = args.get("part_id").and_then(|v| v.as_str()).unwrap_or("");
    let field = args
        .pointer("/content/field")
        .and_then(|v| v.as_str())
        .unwrap_or(part_id);
    let text = store
        .get_kv(&format!("assistant_journal_content:{conv_id}:{run_id}:{part_id}:{field}"))
        .ok()
        .flatten()
        .and_then(|v| v.get("text").and_then(|s| s.as_str()).map(ToString::to_string))
        .unwrap_or_default();
    let digest = sha256_hex(text.as_bytes());
    let b64 = encode_std_base64(text.as_bytes());
    Ok(json!({
        "digest": digest,
        "utf8_bytes": text.len().to_string(),
        "data_base64": b64
    }))
}

fn encode_std_base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((n >> 18) & 0x3f) as usize] as char);
        out.push(TABLE[((n >> 12) & 0x3f) as usize] as char);
        out.push(if chunk.len() > 1 { TABLE[((n >> 6) & 0x3f) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { TABLE[(n & 0x3f) as usize] as char } else { '=' });
    }
    out
}

fn dispatch_assistant_journal_reception_state(
    store: &Store,
    args: &Value,
) -> Result<Value, (&'static str, String)> {
    let source_host = args
        .get("source_host_id")
        .or_else(|| args.get("sourceHostID"))
        .and_then(|v| v.as_str())
        .unwrap_or("remote");
    let through = args
        .get("through")
        .cloned()
        .ok_or(("invalid_argument", "missing 'through'".into()))?;
    let conv_id = through.pointer("/identity/conversation_id").and_then(|v| v.as_str()).unwrap_or("");
    let run_id = through.pointer("/identity/run_id").and_then(|v| v.as_str()).unwrap_or("");
    let key = format!("received_journal:{source_host}:{conv_id}:{run_id}");
    if let Ok(Some(cur)) = store.get_kv(&key) {
        let complete = cur == through;
        return Ok(json!({"after": cur, "complete": complete}));
    }
    Ok(json!({
        "after": {
            "identity": through.get("identity").cloned().unwrap_or(Value::Null),
            "sequence": "0",
            "record_digest": "0".repeat(64)
        },
        "complete": false
    }))
}

fn dispatch_receive_assistant_journal_record(
    store: &Store,
    args: &Value,
) -> Result<Value, (&'static str, String)> {
    let source_host = args
        .get("source_host_id")
        .or_else(|| args.get("sourceHostID"))
        .and_then(|v| v.as_str())
        .unwrap_or("remote");
    let record = args
        .get("record")
        .ok_or(("invalid_argument", "missing 'record'".into()))?;
    let header = record.get("header").unwrap_or(record);
    let identity = header.get("identity").cloned().unwrap_or(Value::Null);
    let conv_id = identity.get("conversation_id").and_then(|v| v.as_str()).unwrap_or("");
    let run_id = identity.get("run_id").and_then(|v| v.as_str()).unwrap_or("");
    let cursor = json!({
        "identity": identity,
        "sequence": header.get("sequence").cloned().unwrap_or_else(|| json!("1")),
        "record_digest": header.get("record_digest").cloned().unwrap_or_else(|| json!("0".repeat(64)))
    });
    if let Some(head) = record.pointer("/batch/head") {
        if let Some(pid) = head.get("part_id").and_then(|v| v.as_str()) {
            let _ = store.put_module_record(
                "assistant_journal_part",
                &format!("{conv_id}:{run_id}:{pid}"),
                head,
            );
        }
    }
    let _ = store.set_kv(&format!("received_journal:{source_host}:{conv_id}:{run_id}"), &cursor);
    let _ = store.set_kv(&format!("journal_head:{conv_id}:{run_id}"), &cursor);
    Ok(json!({
        "applied": true,
        "cursor": cursor
    }))
}

fn dispatch_local_assistant_run_terminal_seal(
    store: &Store,
    args: &Value,
) -> Result<Value, (&'static str, String)> {
    let conv_id = args
        .get("conversation_id")
        .and_then(|v| v.as_str())
        .ok_or(("invalid_argument", "missing 'conversation_id'".into()))?;
    let run_id = args
        .get("run_id")
        .and_then(|v| v.as_str())
        .ok_or(("invalid_argument", "missing 'run_id'".into()))?;
    let found = store
        .get_kv(&format!("run_terminal_seal:{conv_id}:{run_id}"))
        .map_err(|e| ("storage_error", e.to_string()))?;
    Ok(found.unwrap_or(Value::Null))
}

fn dispatch_read_local_assistant_run_terminal_manifest(
    store: &Store,
    args: &Value,
) -> Result<Value, (&'static str, String)> {
    let seal = args
        .get("seal")
        .cloned()
        .ok_or(("invalid_argument", "missing 'seal'".into()))?;
    let after_entry = args
        .get("after_entry")
        .and_then(|v| v.as_str())
        .unwrap_or("0")
        .to_string();
    let conv_id = seal.pointer("/identity/conversation_id").and_then(|v| v.as_str()).unwrap_or("");
    let run_id = seal.pointer("/identity/run_id").and_then(|v| v.as_str()).unwrap_or("");
    let entries = store
        .get_kv(&format!("run_terminal_manifest:{conv_id}:{run_id}"))
        .ok()
        .flatten()
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();
    let next_entry = entries.len().to_string();
    Ok(json!({
        "seal": seal,
        "after_entry": after_entry,
        "next_entry": next_entry,
        "complete": true,
        "entries": entries
    }))
}

fn dispatch_load_run_delivery_ownership(
    store: &Store,
    args: &Value,
) -> Result<Value, (&'static str, String)> {
    let run_id = args
        .get("run_id")
        .and_then(|v| v.as_str())
        .ok_or(("invalid_argument", "missing 'run_id'".into()))?;
    let run = store
        .get_kv(&format!("committed_run:{run_id}"))
        .map_err(|e| ("storage_error", e.to_string()))?;
    let consumed: HashSet<String> = run
        .as_ref()
        .and_then(|r| r.get("consumed_request_ids"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(ToString::to_string))
                .collect()
        })
        .unwrap_or_default();
    let owners: Vec<Value> = run
        .as_ref()
        .and_then(|r| r.get("requests"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter(|r| {
                    let rid = r.get("request_id").and_then(|v| v.as_str()).unwrap_or("");
                    !consumed.contains(rid)
                })
                .map(|r| {
                    json!({
                        "request_id": r.get("request_id").cloned().unwrap_or(Value::Null),
                        "effect_id": r.get("effect_id").cloned().unwrap_or(Value::Null),
                        "delivery": r.get("delivery").cloned().unwrap_or(Value::Null)
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(json!({
        "run_id": run_id,
        "owners": owners
    }))
}

fn dispatch_load_runtime_effect_work_schedule(
    store: &Store,
    args: &Value,
) -> Result<Value, (&'static str, String)> {
    let input = args.get("input").unwrap_or(args);
    let now = input.get("now_ms").and_then(|v| v.as_i64()).unwrap_or_else(now_ms);
    let conv_ids: Vec<String> = input
        .get("conversation_ids")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(ToString::to_string)).collect())
        .unwrap_or_default();
    let mut ready_runs = Vec::new();
    let mut next_wake: Option<i64> = None;
    for conv_id in conv_ids {
        let Some(active) = lookup_active_run(store, &conv_id) else { continue };
        let Some(run_id) = active.get("run_id").and_then(|v| v.as_str()) else { continue };
        let Some(run) = store.get_kv(&format!("committed_run:{run_id}")).ok().flatten() else {
            continue;
        };
        let consumed: HashSet<String> = run
            .get("consumed_request_ids")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(ToString::to_string)).collect())
            .unwrap_or_default();
        if let Some(reqs) = run.get("requests").and_then(|v| v.as_array()) {
            let mut has_unleased = false;
            let mut has_expired = false;
            let mut earliest_expiry: Option<i64> = None;
            for r in reqs {
                let rid = r.get("request_id").and_then(|v| v.as_str()).unwrap_or("");
                if consumed.contains(rid) {
                    continue;
                }
                match r.get("delivery").and_then(|d| d.as_object()) {
                    None => has_unleased = true,
                    Some(del) => {
                        let exp = del.get("lease_expires_at_ms").and_then(|v| v.as_i64()).unwrap_or(0);
                        if now >= exp {
                            has_expired = true;
                        } else {
                            earliest_expiry = Some(earliest_expiry.map_or(exp, |e| e.min(exp)));
                        }
                    }
                }
            }
            if has_unleased {
                ready_runs.push(json!({"conversation_id": conv_id, "run_id": run_id, "reason": "pending"}));
            } else if has_expired {
                ready_runs.push(json!({"conversation_id": conv_id, "run_id": run_id, "reason": "recover_expired_lease"}));
            } else if let Some(exp) = earliest_expiry {
                next_wake = Some(next_wake.map_or(exp, |w| w.min(exp)));
            }
        }
    }
    Ok(json!({
        "runs": ready_runs,
        "next_wake_at_ms": next_wake.map(|w| w.to_string())
    }))
}

fn dispatch_recover_expired_run_deliveries(
    store: &Store,
    args: &Value,
) -> Result<Value, (&'static str, String)> {
    let run_id = args
        .get("expected_run_id")
        .or_else(|| args.get("run_id"))
        .and_then(|v| v.as_str())
        .ok_or(("invalid_argument", "missing 'expected_run_id'".into()))?;
    let now = args
        .get("interrupted_at_ms")
        .and_then(|v| v.as_i64())
        .unwrap_or_else(now_ms);
    let key = format!("committed_run:{run_id}");
    let mut run = store
        .get_kv(&key)
        .map_err(|e| ("storage_error", e.to_string()))?
        .ok_or(("not_found", format!("committed run '{run_id}' not found")))?;
    let consumed: HashSet<String> = run
        .get("consumed_request_ids")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(ToString::to_string))
                .collect()
        })
        .unwrap_or_default();
    let started_attempts: HashSet<String> = run
        .get("started_attempt_request_ids")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(ToString::to_string))
                .collect()
        })
        .unwrap_or_default();
    let mut recovered = 0u64;
    if let Some(reqs) = run.get_mut("requests").and_then(|v| v.as_array_mut()) {
        for r in reqs.iter_mut() {
            let rid = r.get("request_id").and_then(|v| v.as_str()).unwrap_or("");
            if consumed.contains(rid) {
                continue;
            }
            let expired = r
                .pointer("/delivery/lease_expires_at_ms")
                .and_then(|v| v.as_i64())
                .is_some_and(|exp| now >= exp);
            if expired {
                let was_started = started_attempts.contains(rid);
                if let Some(obj) = r.as_object_mut() {
                    obj.insert("attempt_id".into(), json!(new_id()));
                    if was_started {
                        obj.insert("attempt_started".into(), json!(true));
                    }
                    obj.insert("lease_id".into(), Value::Null);
                    obj.insert("delivery".into(), Value::Null);
                }
                recovered += 1;
            }
        }
    }
    store
        .set_kv(&key, &run)
        .map_err(|e| ("storage_error", e.to_string()))?;
    Ok(json!({
        "ok": true,
        "run_id": run_id,
        "recovered_count": recovered
    }))
}

fn dispatch_load_host_execution_state(
    store: &Store,
    args: &Value,
) -> Result<Value, (&'static str, String)> {
    let conv_id = args
        .get("conversation_id")
        .and_then(|v| v.as_str())
        .ok_or(("invalid_argument", "missing 'conversation_id'".into()))?;
    let active = lookup_active_run(store, conv_id);
    let run = active
        .as_ref()
        .and_then(|a| a.get("run_id").and_then(|v| v.as_str()))
        .and_then(|rid| store.get_kv(&format!("committed_run:{rid}")).ok().flatten());
    let session_id = run
        .as_ref()
        .and_then(|r| r.get("session_id").and_then(|v| v.as_str()))
        .unwrap_or(conv_id);
    let turn_id = run
        .as_ref()
        .and_then(|r| r.get("turn_id"))
        .cloned()
        .unwrap_or(Value::Null);
    let turn_status = run
        .as_ref()
        .and_then(|r| r.get("turn_status").and_then(|v| v.as_str()))
        .unwrap_or("idle");
    let perm_rev = run
        .as_ref()
        .and_then(|r| r.get("permission_projection_revision").and_then(|v| v.as_u64()))
        .unwrap_or(1);
    let seq = run
        .as_ref()
        .and_then(|r| r.get("sequence").and_then(|v| v.as_u64()))
        .unwrap_or(1);
    Ok(json!({
        "conversation_id": conv_id,
        "control": {
            "session_id": session_id,
            "revision": seq,
            "active_turn_id": turn_id,
            "turn_status": turn_status,
            "permission_projection_revision": perm_rev,
            "last_event_cursor": {"sequence": seq}
        }
    }))
}

fn dispatch_heartbeat_effect_delivery(
    store: &Store,
    args: &Value,
) -> Result<Value, (&'static str, String)> {
    let input = args.get("input").unwrap_or(args);
    let run_id = input
        .get("run_id")
        .and_then(|v| v.as_str())
        .ok_or(("invalid_argument", "missing 'run_id'".into()))?;
    let attempt_id = input.get("attempt_id").and_then(|v| v.as_str()).unwrap_or("");
    let requested_exp = input
        .get("requested_lease_expires_at_ms")
        .and_then(|v| v.as_i64())
        .unwrap_or_else(|| now_ms() + 240_000);
    let key = format!("committed_run:{run_id}");
    let mut run = store
        .get_kv(&key)
        .map_err(|e| ("storage_error", e.to_string()))?
        .ok_or(("not_found", format!("committed run '{run_id}' not found")))?;
    if let Some(reqs) = run.get_mut("requests").and_then(|v| v.as_array_mut()) {
        for r in reqs.iter_mut() {
            if r.get("attempt_id").and_then(|v| v.as_str()) == Some(attempt_id) {
                if let Some(del) = r.get_mut("delivery").and_then(|v| v.as_object_mut()) {
                    del.insert("lease_expires_at_ms".into(), json!(requested_exp));
                }
            }
        }
    }
    store
        .set_kv(&key, &run)
        .map_err(|e| ("storage_error", e.to_string()))?;
    Ok(json!({
        "ok": true,
        "run_id": run_id,
        "attempt_id": attempt_id,
        "lease_expires_at_ms": requested_exp
    }))
}

fn dispatch_inspect_run_process_owners(
    store: &Store,
    args: &Value,
) -> Result<Value, (&'static str, String)> {
    let conv_id = args
        .get("conversation_id")
        .and_then(|v| v.as_str())
        .ok_or(("invalid_argument", "missing 'conversation_id'".into()))?;
    let run_id = args
        .get("expected_run_id")
        .or_else(|| args.get("run_id"))
        .and_then(|v| v.as_str())
        .ok_or(("invalid_argument", "missing 'expected_run_id'".into()))?;
    let inv = store
        .get_kv(&format!("exact_run_invocation:{conv_id}:{run_id}"))
        .map_err(|e| ("storage_error", e.to_string()))?
        .ok_or(("not_found", format!("exact run invocation '{run_id}' not found")))?;
    Ok(json!({
        "conversation_id": conv_id,
        "run_id": run_id,
        "assistant_message_id": inv.get("assistant_message_id").cloned().unwrap_or(Value::Null),
        "invocation_config_digest": inv.get("invocation_config_digest").cloned().unwrap_or(Value::Null),
        "host_invocation": inv.get("host_invocation").cloned().unwrap_or_else(|| json!({})),
        "has_session_lease": false,
        "process_owners": [],
        "deliveries": []
    }))
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
        assert_eq!(got_sub["submission_id"], sub_id);

        let subs = dispatch_agent_room_command(
            &store,
            &suite,
            &json!({"command": {"type": "submissions", "room_id": "room-1"}}),
        )
        .unwrap();
        assert_eq!(subs.as_array().unwrap().len(), 1);

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
            &json!({
                "arguments": {
                    "routes": [{
                        "route": {
                            "kind": "native",
                            "adapter": "mlx",
                            "model_id": "fixture-native"
                        },
                        "max_output_tokens": 4096
                    }],
                    "preference": {"explicit": 2048}
                }
            }),
        )
        .unwrap();
        assert_eq!(adm["resolved"]["source"], "explicit_preference");
        assert_eq!(adm["resolved"]["tokens"], 2048);

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
        assert_eq!(rev, json!(2));

        // Canonical resolve_chat_model state machine & put_product_setting_checked CAS/validation
        let bad_default = dispatch_command(
            &store,
            &suite,
            "resolve_chat_model",
            &json!({"intent": {"kind": "inherit"}, "default_value_json": "{}", "candidates": []}),
        )
        .unwrap();
        assert_eq!(bad_default, json!({"kind": "invalid_configuration"}));
        let explicit_overrides_bad = dispatch_command(
            &store,
            &suite,
            "resolve_chat_model",
            &json!({
                "intent": {"kind": "explicit", "profile_id": "api:valid"},
                "default_value_json": "{}",
                "candidates": [{"profile_id": "api:valid", "allowed_for_chat": true, "availability": "available"}]
            }),
        )
        .unwrap();
        assert_eq!(
            explicit_overrides_bad,
            json!({"kind": "resolved", "profile_id": "api:valid", "origin": "conversation"})
        );
        let unavail = dispatch_command(
            &store,
            &suite,
            "resolve_chat_model",
            &json!({
                "intent": {"kind": "explicit", "profile_id": "api:restricted"},
                "candidates": [{"profile_id": "api:restricted", "allowed_for_chat": false, "availability": "available"}]
            }),
        )
        .unwrap();
        assert_eq!(
            unavail,
            json!({"kind": "unavailable", "profile_id": "api:restricted", "origin": "conversation", "reason": "not_allowed_for_chat"})
        );
        assert!(dispatch_command(
            &store,
            &suite,
            "put_product_setting_checked",
            &json!({"key": "chat.default_model", "value_json": "{}", "updated_at_ms": 1}),
        )
        .is_err());
        let valid_profile = r#"{"version":1,"selection":{"kind":"profile","profile_id":"local:b"}}"#;
        let saved = dispatch_command(
            &store,
            &suite,
            "put_product_setting_checked",
            &json!({"key": "chat.default_model", "value_json": valid_profile, "updated_at_ms": 10}),
        )
        .unwrap();
        assert_eq!(saved["revision"], 1);
        assert!(dispatch_command(
            &store,
            &suite,
            "put_product_setting_checked",
            &json!({"key": "chat.default_model", "value_json": valid_profile, "updated_at_ms": 11}),
        )
        .is_err());
        let valid_auto = r#"{"version":1,"selection":{"kind":"automatic"}}"#;
        let updated = dispatch_command(
            &store,
            &suite,
            "put_product_setting_checked",
            &json!({"key": "chat.default_model", "value_json": valid_auto, "expected_revision": 1, "updated_at_ms": 12}),
        )
        .unwrap();
        assert_eq!(updated["revision"], 2);
        let no_cand = dispatch_command(
            &store,
            &suite,
            "resolve_chat_model",
            &json!({"intent": {"kind": "inherit"}, "default_value_json": valid_auto, "candidates": []}),
        )
        .unwrap();
        assert_eq!(no_cand, json!({"kind": "no_automatic_candidate"}));

        let up = dispatch_command(
            &store,
            &suite,
            "begin_content_upload",
            &json!({"owner": {"conversation_id": "c1", "import_id": "imp1", "part_id": "p1"}, "expected_utf8_bytes": 5}),
        )
        .unwrap();
        let uid = up["upload_id"].as_str().unwrap().to_string();
        let app_c = dispatch_command(
            &store,
            &suite,
            "append_content_chunk",
            &json!({"upload_id": uid, "owner": {"conversation_id": "c1", "import_id": "imp1", "part_id": "p1"}, "offset_bytes": 0, "text": "hello"}),
        )
        .unwrap();
        assert_eq!(app_c["committed_utf8_bytes"], 5);
        let read_c = dispatch_command(
            &store,
            &suite,
            "read_content_upload_chunk",
            &json!({"upload_id": uid, "owner": {"conversation_id": "c1", "import_id": "imp1", "part_id": "p1"}, "offset_bytes": 0}),
        )
        .unwrap();
        assert_eq!(read_c["text"], "hello");
        let seal = dispatch_command(
            &store,
            &suite,
            "seal_content_upload",
            &json!({"upload_id": uid, "owner": {"conversation_id": "c1", "import_id": "imp1", "part_id": "p1"}}),
        )
        .unwrap();
        assert_eq!(seal["state"], "sealed");
        let prog = dispatch_command(
            &store,
            &suite,
            "content_upload_progress",
            &json!({"upload_id": uid, "owner": {"conversation_id": "c1", "import_id": "imp1", "part_id": "p1"}}),
        )
        .unwrap();
        assert_eq!(prog["state"], "sealed");

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

    #[test]
    fn daemon_expert_room_fixes_and_mobile_delegated_task_lifecycle() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory().unwrap();
        let suite = ModulesSuite::new(store.clone(), dir.path().to_path_buf(), None);

        // 1. Expert description preservation + explicit model_override clear + delete_agent alias with nested input
        let exp = dispatch_agent_room_command(
            &store,
            &suite,
            &json!({
                "command": {
                    "type": "configure_agent",
                    "input": {
                        "id": "exp-1",
                        "name": "Reviewer",
                        "role": "reviewer",
                        "description": "Careful Rust code reviewer",
                        "model_override": "gpt-5"
                    }
                }
            }),
        )
        .unwrap();
        assert_eq!(exp["expert"]["description"], "Careful Rust code reviewer");
        assert_eq!(exp["expert"]["model_override"], "gpt-5");

        let cleared = dispatch_agent_room_command(
            &store,
            &suite,
            &json!({
                "command": {
                    "type": "configure_agent",
                    "input": {
                        "id": "exp-1",
                        "model_override": null
                    }
                }
            }),
        )
        .unwrap();
        assert_eq!(cleared["expert"]["description"], "Careful Rust code reviewer");
        assert!(cleared["expert"]["model_override"].is_null());

        let del_agent = dispatch_agent_room_command(
            &store,
            &suite,
            &json!({
                "command": {
                    "type": "delete_agent",
                    "input": {"id": "exp-1", "expert_id": "exp-1"}
                }
            }),
        )
        .unwrap();
        assert_eq!(del_agent["deleted"], true);

        // 2. Room session_id attach + detach/null clear + delete_room with nested input
        let _ = dispatch_agent_room_command(
            &store,
            &suite,
            &json!({"command": {"type": "create_room", "input": {"id": "rm-2", "name": "Ops"}}}),
        )
        .unwrap();
        let attached = dispatch_agent_room_command(
            &store,
            &suite,
            &json!({"command": {"type": "update_room", "input": {"id": "rm-2", "conversation_id": "conv-attach-1"}}}),
        )
        .unwrap();
        assert_eq!(attached["room"]["session_id"], "conv-attach-1");
        let detached = dispatch_agent_room_command(
            &store,
            &suite,
            &json!({"command": {"type": "update_room", "input": {"id": "rm-2", "detach": true}}}),
        )
        .unwrap();
        assert_eq!(detached["room"]["session_id"], "");
        let attached2 = dispatch_agent_room_command(
            &store,
            &suite,
            &json!({"command": {"type": "update_room", "input": {"id": "rm-2", "session_id": "conv-attach-2"}}}),
        )
        .unwrap();
        assert_eq!(attached2["room"]["session_id"], "conv-attach-2");
        let cleared_null = dispatch_agent_room_command(
            &store,
            &suite,
            &json!({"command": {"type": "update_room", "input": {"id": "rm-2", "session_id": null}}}),
        )
        .unwrap();
        assert_eq!(cleared_null["room"]["session_id"], "");
        let del_room = dispatch_agent_room_command(
            &store,
            &suite,
            &json!({"command": {"type": "delete_room", "input": {"id": "rm-2", "room_id": "rm-2"}}}),
        )
        .unwrap();
        assert_eq!(del_room["deleted"], true);

        // 3. All 6 mobile delegated task commands
        let captured = dispatch_command(
            &store,
            &suite,
            "capture_mobile_delegation_configuration",
            &json!({
                "config": {
                    "session_id": "parent-sess-1",
                    "tools": [{"function": {"name": "read_file"}}],
                    "host_catalog": {"host_id": "ios-host-1"},
                    "context_window": {"context_window_tokens": 128000, "reserved_output_tokens": 4096},
                    "turn_identity": {
                        "host_invocation": {"primary": {}, "generation": {}, "output_admission": {}},
                        "host_dependencies": {},
                        "model_capability": {}
                    }
                },
                "settings": {"project_root": "ios-device"}
            }),
        )
        .unwrap();
        assert_eq!(
            captured["turn_identity"]["delegation"]["project_root"],
            "ios-device"
        );

        let prepared = dispatch_command(
            &store,
            &suite,
            "prepare_mobile_delegated_task",
            &json!({
                "input": {
                    "request": {
                        "conversation_id": "parent-sess-1",
                        "payload": {
                            "call_id": "call-spawn-1",
                            "capability_id": "spawn_agent",
                            "input": {"prompt": "Inspect logs", "title": "Log Inspector"}
                        }
                    },
                    "now_ms": 1000
                }
            }),
        )
        .unwrap();
        let task_id = prepared["prepared"]["task"]["id"]
            .as_str()
            .unwrap()
            .to_string();
        assert_eq!(prepared["response"]["type"], "task_promoted");
        assert_eq!(prepared["prepared"]["task"]["owner_session_id"], "parent-sess-1");

        let child_cfg = dispatch_command(
            &store,
            &suite,
            "load_mobile_task_child_configuration",
            &json!({"task_id": task_id}),
        )
        .unwrap();
        assert_eq!(
            child_cfg["session_id"],
            prepared["prepared"]["task"]["child_session_id"]
        );
        assert_eq!(child_cfg["host_catalog"]["host_id"], "ios-host-1");

        let admitted = dispatch_command(
            &store,
            &suite,
            "admit_mobile_task_child",
            &json!({
                "input": {
                    "task_id": task_id,
                    "parent_conversation_id": "parent-sess-1",
                    "worker": {"pid": 42},
                    "now_ms": 2000,
                    "lease_until_ms": 62000,
                    "candidate_config": child_cfg,
                    "current_tools": []
                }
            }),
        )
        .unwrap();
        assert_eq!(admitted["task_id"], task_id);
        assert_eq!(admitted["attempt"], 1);
        assert_eq!(admitted["cancellation_only"], false);
        assert_eq!(
            admitted["invocation"]["core_session_id"],
            child_cfg["session_id"]
        );

        let guided = dispatch_command(
            &store,
            &suite,
            "prepare_mobile_task_guidance",
            &json!({
                "input": {
                    "request": {
                        "payload": {
                            "call_id": "call-guide-1",
                            "input": {"task_id": task_id, "message": "Check stderr too"}
                        }
                    },
                    "now_ms": 2500
                }
            }),
        )
        .unwrap();
        assert_eq!(guided["type"], "tool_completed");
        assert_eq!(guided["outcome"], "succeeded");

        let observed = dispatch_command(
            &store,
            &suite,
            "observe_mobile_task",
            &json!({
                "input": {
                    "request": {
                        "payload": {
                            "call_id": "call-obs-1",
                            "capability_id": "wait_task",
                            "input": {"task_id": task_id, "wait_ms": 3000}
                        }
                    },
                    "now_ms": 3000
                }
            }),
        )
        .unwrap();
        assert_eq!(observed["terminal"], false);
        assert_eq!(observed["wait_ms"], 3000);
        assert_eq!(observed["response"]["type"], "tool_completed");
    }

    #[test]
    fn daemon_v2_cross_platform_canonical_contracts_hold() {
        let dir = std::env::temp_dir().join(format!("nola-v2-contracts-{}", new_id()));
        let _ = std::fs::create_dir_all(&dir);
        let db = dir.join("nola.db");
        let store = Store::open(&db).unwrap();
        let suite = ModulesSuite::new(store.clone(), dir.clone(), Some(dir.clone()));

        // 1. Conversation creation, case-insensitive lookup, load_conversation vs projection_v1 parts, UTF-8 export slicing
        let conv_id = "11111111-2222-3333-4444-555555555555";
        let _ = dispatch_command(
            &store,
            &suite,
            "create_conversation",
            &json!({"id": conv_id, "title": "V2 Contracts"}),
        )
        .unwrap();
        let utf8_text = "你好世界🌍".repeat(20);
        let _ = dispatch_command(
            &store,
            &suite,
            "import_conversation_transcript",
            &json!({
                "conversation_id": conv_id,
                "messages": [{"id": "m-1", "role": "user", "parts": [{"id": "p-1", "kind": "text", "content_text": utf8_text}]}]
            }),
        )
        .unwrap();
        let loaded = dispatch_command(
            &store,
            &suite,
            "load_conversation",
            &json!({"conversation_id": conv_id.to_uppercase()}),
        )
        .unwrap();
        assert_eq!(loaded["messages"][0]["id"], "m-1");
        assert!(loaded["messages"][0]["parts"][0].get("id").is_some());
        let proj = dispatch_command(
            &store,
            &suite,
            "load_conversation_projection_v1",
            &json!({"conversation_id": conv_id}),
        )
        .unwrap();
        let p0 = proj["messages"][0]["parts"][0].as_object().unwrap();
        assert_eq!(p0.len(), 5);
        assert!(p0.contains_key("part_id") && p0.contains_key("markdown") && !p0.contains_key("id"));

        let page = dispatch_command(
            &store,
            &suite,
            "load_conversation_export_page",
            &json!({"conversation_id": conv_id}),
        )
        .unwrap();
        assert_eq!(page["conversation"]["id"], conv_id);
        let range = dispatch_command(
            &store,
            &suite,
            "read_conversation_export_range",
            &json!({"conversation_id": conv_id, "message_id": "m-1", "part_id": "p-1", "field": "content_text", "offset_bytes": 0, "limit_bytes": 10}),
        )
        .unwrap();
        assert_eq!(range["end_of_content"], false);
        assert!(range["text"].as_str().unwrap().len() <= 10);

        // 2. Capabilities + strict core_config validation + strict 2-key load_run_identity + permission projection control owner
        let ui_cap = dispatch_command(&store, &suite, "read_ui_capability", &json!({})).unwrap();
        assert_eq!(ui_cap["descriptor"]["id"], "render_ui");
        let peer_cap = dispatch_command(&store, &suite, "read_expert_exchange_capability", &json!({})).unwrap();
        assert_eq!(peer_cap["tool"]["function"]["name"], "delegate_expert");
        assert!(dispatch_command(
            &store,
            &suite,
            "begin_assistant_turn",
            &json!({"input": {"conversation_id": conv_id, "core_config": {"host_id": "bad"}, "submission": {"type": "user_input"}}}),
        )
        .is_err());
        let begun = dispatch_command(
            &store,
            &suite,
            "begin_assistant_turn",
            &json!({"input": {"conversation_id": conv_id, "core_config": {"session_id": "s1", "model": "m1"}, "submission": {"type": "user_input", "submission_id": "run-1"}}}),
        )
        .unwrap();
        assert_eq!(begun["execution"]["control"]["session_id"], "s1");
        let ident = dispatch_command(&store, &suite, "load_run_identity", &json!({"run_id": "run-1"})).unwrap();
        assert_eq!(ident.as_object().unwrap().len(), 2);
        assert_eq!(ident["run_id"], "run-1");
        let perm = dispatch_command(
            &store,
            &suite,
            "load_conversation_permission_projection_v1",
            &json!({"conversation_id": conv_id, "run_id": "run-1"}),
        )
        .unwrap();
        assert_eq!(perm["session_id"], "s1");
        assert_eq!(perm["permissions"]["revision"], 1);

        // 3. In-place resend idempotency, fabricated compaction lineage rejection, compaction tight window rejection
        let resend = dispatch_command(
            &store,
            &suite,
            "prepare_in_place_resend",
            &json!({"input": {"operation_id": "op-1", "conversation_id": conv_id, "message_id": "m-1", "edited_text": "edited"}}),
        )
        .unwrap();
        assert_eq!(resend["run_id"], "op-1");
        assert!(resend.get("conversation_id").is_none());
        assert!(dispatch_command(
            &store,
            &suite,
            "context",
            &json!({"command": {"type": "commit_maintenance", "input": {"source": {"conversation_id": conv_id}, "summary": "bad", "original_input": {"messages": [{"id": "fabricated-id"}]}}}}),
        )
        .is_err());
        assert!(dispatch_command(
            &store,
            &suite,
            "step_compaction_summary",
            &json!({"step": {"type": "start", "messages": [{"role": "user", "content": "hi"}], "context_window_tokens": 8000, "output_tokens": 4096, "target_tokens": 1024}}),
        )
        .is_err());

        // 4. Skill sync idempotency, hidden visibility CAS, tombstone non-resurrection
        let sync_in = json!({"command": {"type": "synchronize_built_ins", "inputs": [{"id": "s-bi", "source": "built_in", "display_name": "BuiltIn", "summary": "sum", "category": "general", "instructions_ref": "builtin://x", "instructions_text": "body", "required_tools": [], "default_mode": "inline", "version": "1.0.0"}]}});
        let s1 = dispatch_command(&store, &suite, "skill", &sync_in).unwrap();
        let s2 = dispatch_command(&store, &suite, "skill", &sync_in).unwrap();
        assert_eq!(s1[0]["revision"], 1);
        assert_eq!(s2[0]["revision"], 1);
        let removed = dispatch_command(
            &store,
            &suite,
            "skill",
            &json!({"command": {"type": "remove_built_in", "id": "s-bi", "expected_revision": 1}}),
        )
        .unwrap();
        assert_eq!(removed["record"]["enabled"], false);
        assert!(dispatch_command(
            &store,
            &suite,
            "skill",
            &json!({"command": {"type": "restore_built_ins", "expected_revisions": {"s-bi": 2}}}),
        )
        .is_err());

        // 5. Memory snapshot conflict & read-back parity + auxiliary observe_source receipt unwrap
        let m1 = dispatch_command(
            &store,
            &suite,
            "memory",
            &json!({"command": {"type": "replace", "expected": [], "candidates": [{"record": {"id": "mem-1", "content": "alpha", "kind": "preference"}}]}}),
        )
        .unwrap();
        let m_list = dispatch_command(&store, &suite, "memory", &json!({"command": {"type": "list"}})).unwrap();
        assert_eq!(m1[0]["created_at_ms"], m_list["items"][0]["created_at_ms"]);
        assert!(dispatch_command(
            &store,
            &suite,
            "memory",
            &json!({"command": {"type": "replace", "expected": [], "candidates": []}}),
        )
        .is_err());

        let obs = dispatch_command(
            &store,
            &suite,
            "auxiliary_invocation",
            &json!({"command": {"type": "observe_source", "local_owner": {"kind": "account", "account_id": "a1"}, "observation": {"source_host_id": "h1", "operation_id": "op1", "attempt_id": "op1:physical", "receipt": {"operation": {"id": "op1", "scenario": "event_summary", "state": "running", "revision": 2, "attempts_started": 1}, "attempt": {"id": "op1:physical", "state": "running", "revision": 1}, "dispatch_permitted": false}}}}),
        )
        .unwrap();
        assert_eq!(obs["attempt"]["id"], "op1:physical");
        let slot = dispatch_command(
            &store,
            &suite,
            "load_auxiliary_source_slot",
            &json!({"local_owner": {"kind": "account", "account_id": "a1"}, "scenario": "event_summary"}),
        )
        .unwrap();
        assert_eq!(slot["attempt_id"], "op1:physical");

        // 6. Compaction prepare_maintenance cannot_fit on tight 8K/4K window, expert preview_capture rejection & load_manifest(message_id),
        //    and permission request consumption + resource-scoped allow_conversation grant reuse vs different-file re-prompt.
        let tight_plan = dispatch_command(
            &store,
            &suite,
            "context",
            &json!({"command": {"type": "prepare_maintenance", "source": {"conversation_id": conv_id}, "input": {"mode": "force_summary", "messages": [{"id": "m-1", "role": "user", "content": "a"}, {"id": "m-2", "role": "assistant", "content": "b"}, {"id": "m-3", "role": "user", "content": "c"}], "budget": {"context_window_tokens": 8000, "reserved_output_tokens": 4096, "keep_recent_messages": 1}}}}),
        )
        .unwrap();
        assert_eq!(tight_plan["output"]["action"], "cannot_fit");

        let preview_err = dispatch_command(
            &store,
            &suite,
            "native_expert_host_operation",
            &json!({"operation": {"kind": "preview_capture", "input": {"conversation_id": conv_id}}}),
        )
        .unwrap_err();
        assert_eq!(preview_err.0, "requires_main_agent_turn");
        let missing_manifest = dispatch_command(
            &store,
            &suite,
            "native_expert_host_operation",
            &json!({"operation": {"kind": "load_manifest", "conversation_id": conv_id, "message_id": "m-1"}}),
        )
        .unwrap();
        assert!(missing_manifest.is_null());

        let perm_begun = dispatch_command(
            &store,
            &suite,
            "begin_assistant_turn",
            &json!({"input": {"conversation_id": conv_id, "core_config": {"session_id": "s-perm", "model": "m1", "host_catalog": {"capabilities": {"device_read_file": {"permission_domain": "files_read"}}}}, "submission": {"type": "user_input", "submission_id": "run-perm"}}}),
        )
        .unwrap();
        assert_eq!(perm_begun["execution"]["control"]["session_id"], "s-perm");
        let reqs0 = dispatch_command(&store, &suite, "claim_committed_host_requests", &json!({"input": {"run_id": "run-perm"}})).unwrap();
        let m0_id = reqs0["requests"][0]["request_id"].as_str().unwrap().to_string();
        dispatch_command(
            &store,
            &suite,
            "reduce_and_accept_host_response",
            &json!({"input": {"conversation_id": conv_id, "run_id": "run-perm", "response": {"request_id": m0_id, "sequence": 2, "payload": {"type": "model_completed", "items": [], "tool_calls": [{"call_id": "c1", "name": "device_read_file", "arguments": {"uri": "file:///a.txt"}}]}}}}),
        )
        .unwrap();
        let reqs1 = dispatch_command(&store, &suite, "claim_committed_host_requests", &json!({"input": {"run_id": "run-perm"}})).unwrap();
        assert_eq!(reqs1["requests"].as_array().unwrap().len(), 1);
        assert_eq!(reqs1["requests"][0]["payload"]["type"], "request_permission");
        let perm_id = reqs1["requests"][0]["payload"]["permission_request_id"].as_str().unwrap().to_string();
        dispatch_command(
            &store,
            &suite,
            "reduce_and_commit_active_submission",
            &json!({"input": {"run_id": "run-perm", "submission": {"type": "permission_response", "request_id": perm_id, "decision": "allow_conversation", "grant": {"scope": "conversation"}}}}),
        )
        .unwrap();
        let reqs2 = dispatch_command(&store, &suite, "claim_committed_host_requests", &json!({"input": {"run_id": "run-perm"}})).unwrap();
        assert_eq!(reqs2["requests"].as_array().unwrap().len(), 1);
        assert_eq!(reqs2["requests"][0]["payload"]["type"], "execute_tool");
        let t1_id = reqs2["requests"][0]["request_id"].as_str().unwrap().to_string();
        dispatch_command(
            &store,
            &suite,
            "reduce_and_accept_host_response",
            &json!({"input": {"conversation_id": conv_id, "run_id": "run-perm", "response": {"request_id": t1_id, "sequence": 3, "payload": {"type": "tool_completed", "call_id": "c1"}}}}),
        )
        .unwrap();
        let reqs3 = dispatch_command(&store, &suite, "claim_committed_host_requests", &json!({"input": {"run_id": "run-perm"}})).unwrap();
        let m1_id = reqs3["requests"][0]["request_id"].as_str().unwrap().to_string();
        // Same URI reuses conversation grant without re-prompting; different URI must ask permission again.
        dispatch_command(
            &store,
            &suite,
            "reduce_and_accept_host_response",
            &json!({"input": {"conversation_id": conv_id, "run_id": "run-perm", "response": {"request_id": m1_id, "sequence": 4, "payload": {"type": "model_completed", "items": [], "tool_calls": [{"call_id": "c2", "name": "device_read_file", "arguments": {"uri": "file:///a.txt"}}, {"call_id": "c3", "name": "device_read_file", "arguments": {"uri": "file:///b.txt"}}]}}}}),
        )
        .unwrap();
        let reqs4 = dispatch_command(&store, &suite, "claim_committed_host_requests", &json!({"input": {"run_id": "run-perm"}})).unwrap();
        let types: Vec<&str> = reqs4["requests"].as_array().unwrap().iter().filter_map(|r| r.pointer("/payload/type").and_then(|v| v.as_str())).collect();
        assert_eq!(types, vec!["execute_tool", "request_permission"]);

        // 7. Assistant journal publication, snapshot/parts/range reader, message admissions, terminal seal, and expired lease recovery.
        let j_sub_id = json!({
            "source_host_id": "h-j", "client_id": "c-j", "conversation_id": conv_id,
            "submission_id": "run-j", "user_message_id": "u-j", "assistant_message_id": "a-j"
        });
        let reg = dispatch_command(
            &store,
            &suite,
            "register_source_submission",
            &json!({"identity": j_sub_id, "request": {"conversationId": conv_id}}),
        )
        .unwrap();
        assert_eq!(reg["created"], true);
        let _ = dispatch_command(
            &store,
            &suite,
            "begin_assistant_turn",
            &json!({"input": {"conversation_id": conv_id, "user_message_id": "u-j", "assistant_message_id": "a-j", "user_text": "hello", "core_config": {"session_id": "s-j", "model": "m1", "permission_rules": {"rules": [{"capability_id": "fixture_read", "decision": "ask"}]}}, "submission": {"type": "user_input", "submission_id": "run-j"}}}),
        )
        .unwrap();
        let adm_head = dispatch_command(
            &store,
            &suite,
            "local_run_message_admission_head",
            &json!({"conversation_id": conv_id, "run_id": "run-j"}),
        )
        .unwrap();
        assert_eq!(adm_head["sequence"], "1");
        let rec_state = dispatch_command(
            &store,
            &suite,
            "run_message_admission_reception_state",
            &json!({"source_host_id": "h-j", "through": adm_head}),
        )
        .unwrap();
        let adm_page = dispatch_command(
            &store,
            &suite,
            "read_run_message_admissions",
            &json!({"after": rec_state, "through": adm_head}),
        )
        .unwrap();
        assert_eq!(adm_page["complete"], true);
        let _ = dispatch_command(
            &store,
            &suite,
            "receive_run_message_admissions",
            &json!({"source_host_id": "h-j", "page": adm_page}),
        )
        .unwrap();
        // Model emits reasoning delta, then rule-gated permission, then after lease expiry recovers with fresh delivery, resolves permission, completes tool & answer, and seals.
        let j_reqs0 = dispatch_command(
            &store,
            &suite,
            "claim_committed_host_requests",
            &json!({"input": {"run_id": "run-j", "worker_generation": "g1", "lease_owner": "o1", "claimed_at_ms": 1000, "lease_expires_at_ms": 2000}}),
        )
        .unwrap();
        let jm0_id = j_reqs0["requests"][0]["request_id"].as_str().unwrap().to_string();
        assert_eq!(j_reqs0["requests"][0]["attempt_started"], false);
        let _ = dispatch_command(
            &store,
            &suite,
            "reduce_and_accept_host_response",
            &json!({"input": {"conversation_id": conv_id, "run_id": "run-j", "response": {"request_id": jm0_id, "payload": {"type": "model_attempt_started", "provider": "local", "model": "m1", "reason": "initial"}}}}),
        )
        .unwrap();
        let j_reqs0_again = dispatch_command(
            &store,
            &suite,
            "claim_committed_host_requests",
            &json!({"input": {"run_id": "run-j", "worker_generation": "g1", "lease_owner": "o1", "claimed_at_ms": 1005, "lease_expires_at_ms": 2005}}),
        )
        .unwrap();
        assert_eq!(j_reqs0_again["requests"][0], j_reqs0["requests"][0]);
        let jm0_r_pid = j_reqs0["requests"][0]["payload"]["reasoning_part_id"].as_str().unwrap().to_string();
        let jm0_t_pid = j_reqs0["requests"][0]["payload"]["text_part_id"].as_str().unwrap().to_string();
        let _ = dispatch_command(
            &store,
            &suite,
            "reduce_and_accept_host_response",
            &json!({"input": {"conversation_id": conv_id, "run_id": "run-j", "response": {"request_id": jm0_id, "payload": {"type": "model_delta", "channel": "reasoning", "part_id": jm0_r_pid, "delta": "thinking"}}}}),
        )
        .unwrap();
        let perm_tr = dispatch_command(
            &store,
            &suite,
            "reduce_and_accept_host_response",
            &json!({"input": {"conversation_id": conv_id, "run_id": "run-j", "response": {"request_id": jm0_id, "payload": {"type": "model_completed", "items": [], "tool_calls": [{"call_id": "jc1", "name": "fixture_read", "arguments": {}}]}}}}),
        )
        .unwrap();
        assert_eq!(perm_tr["control"]["turn_status"], "awaiting_permission");
        let awaiting_proj = dispatch_command(
            &store,
            &suite,
            "load_conversation_projection_v1",
            &json!({"conversation_id": conv_id}),
        )
        .unwrap();
        assert_eq!(awaiting_proj["active_run"]["state"], "waiting_for_permission");
        assert!(awaiting_proj["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["message_id"] == "a-j" && m["state"] == "running"));
        let j_preqs = dispatch_command(
            &store,
            &suite,
            "claim_committed_host_requests",
            &json!({"input": {"run_id": "run-j", "worker_generation": "g1", "lease_owner": "o1", "claimed_at_ms": 1000, "lease_expires_at_ms": 2000}}),
        )
        .unwrap();
        assert_eq!(j_preqs["requests"][0]["payload"]["type"], "request_permission");
        assert_eq!(j_preqs["requests"][0]["payload"]["allow_conversation"], true);
        let inspected_perm = dispatch_command(
            &store,
            &suite,
            "inspect_mobile_permission_request",
            &json!({"request": j_preqs["requests"][0], "now_ms": 1500}),
        )
        .unwrap();
        assert_eq!(inspected_perm["conversation_id"], conv_id);
        assert_eq!(inspected_perm["run_id"], "run-j");
        assert_eq!(inspected_perm["payload"], j_preqs["requests"][0]["payload"]);
        let exact_j = dispatch_command(
            &store,
            &suite,
            "load_exact_run_invocation",
            &json!({"conversation_id": conv_id, "expected_run_id": "run-j", "assistant_message_id": "a-j"}),
        )
        .unwrap();
        let auth_receipt = dispatch_command(
            &store,
            &suite,
            "validate_mobile_run_recovery_authority",
            &json!({
                "conversation_id": conv_id,
                "expected_run_id": "run-j",
                "assistant_message_id": "a-j",
                "expected_invocation_config_digest": exact_j["invocation_config_digest"]
            }),
        )
        .unwrap();
        assert_eq!(auth_receipt.as_object().unwrap().len(), 4);
        assert_eq!(auth_receipt["run_id"], "run-j");
        let perm_proj_j = dispatch_command(
            &store,
            &suite,
            "load_conversation_permission_projection_v1",
            &json!({"conversation_id": conv_id, "run_id": "run-j"}),
        )
        .unwrap();
        let pend0 = &perm_proj_j["permissions"]["pending"][0];
        assert_eq!(pend0["allow_conversation"], true);
        assert!(pend0["permission_domain"].is_string() && pend0["rationale"].is_string() && pend0["evaluation"].is_object());
        let sched = dispatch_command(
            &store,
            &suite,
            "load_runtime_effect_work_schedule",
            &json!({"input": {"conversation_ids": [conv_id], "now_ms": 3000}}),
        )
        .unwrap();
        assert_eq!(sched["runs"][0]["reason"], "recover_expired_lease");
        let exec_st = dispatch_command(
            &store,
            &suite,
            "load_host_execution_state",
            &json!({"conversation_id": conv_id}),
        )
        .unwrap();
        assert!(exec_st["control"]["revision"].as_u64().unwrap() >= 1);
        assert_eq!(exec_st["control"]["turn_status"], "awaiting_permission");
        let recov = dispatch_command(
            &store,
            &suite,
            "recover_expired_run_deliveries",
            &json!({"conversation_id": conv_id, "expected_run_id": "run-j", "interrupted_at_ms": 3000}),
        )
        .unwrap();
        assert_eq!(recov["recovered_count"], 1);
        let j_preqs2 = dispatch_command(
            &store,
            &suite,
            "claim_committed_host_requests",
            &json!({"input": {"run_id": "run-j", "worker_generation": "g2", "lease_owner": "o2", "claimed_at_ms": 3001, "lease_expires_at_ms": 5000}}),
        )
        .unwrap();
        let jp_req_id = j_preqs2["requests"][0]["request_id"].as_str().unwrap().to_string();
        let jp_perm_id = j_preqs2["requests"][0]["payload"]["permission_request_id"].as_str().unwrap().to_string();
        let _ = dispatch_command(
            &store,
            &suite,
            "reduce_and_accept_host_response",
            &json!({"input": {"conversation_id": conv_id, "run_id": "run-j", "response": {"request_id": jp_req_id, "payload": {"type": "permission_resolved", "request_id": jp_perm_id, "resolution": {"decision": "allow_once"}}}}}),
        )
        .unwrap();
        let jt_reqs = dispatch_command(&store, &suite, "claim_committed_host_requests", &json!({"input": {"run_id": "run-j"}})).unwrap();
        assert_eq!(jt_reqs["requests"][0]["payload"]["permission_profile"], "cautious");
        let jt_id = jt_reqs["requests"][0]["request_id"].as_str().unwrap().to_string();
        let _ = dispatch_command(
            &store,
            &suite,
            "reduce_and_accept_host_response",
            &json!({"input": {"conversation_id": conv_id, "run_id": "run-j", "response": {"request_id": jt_id, "payload": {"type": "tool_completed", "call_id": "jc1", "output": {"text": "ok"}, "artifacts": []}}}}),
        )
        .unwrap();
        let jm1_reqs = dispatch_command(&store, &suite, "claim_committed_host_requests", &json!({"input": {"run_id": "run-j"}})).unwrap();
        let jm1_id = jm1_reqs["requests"][0]["request_id"].as_str().unwrap().to_string();
        let _ = dispatch_command(
            &store,
            &suite,
            "reduce_and_accept_host_response",
            &json!({"input": {"conversation_id": conv_id, "run_id": "run-j", "response": {"request_id": jm1_id, "payload": {"type": "model_committed_delta", "part_id": jm0_t_pid, "delta": "Final answer"}}}}),
        )
        .unwrap();
        let term_tr = dispatch_command(
            &store,
            &suite,
            "reduce_and_accept_host_response",
            &json!({"input": {"conversation_id": conv_id, "run_id": "run-j", "response": {"request_id": jm1_id, "payload": {"type": "model_completed", "items": [{"type": "message", "id": jm0_t_pid, "role": "assistant", "content": "Final answer"}], "tool_calls": []}}}}),
        )
        .unwrap();
        assert_eq!(term_tr["control"]["turn_status"], "completed");
        let j_heads = dispatch_command(
            &store,
            &suite,
            "local_run_assistant_journal_heads",
            &json!({"conversation_id": conv_id, "run_id": "run-j", "limit": 64}),
        )
        .unwrap();
        let snap = dispatch_command(
            &store,
            &suite,
            "open_assistant_journal_snapshot",
            &json!({"at": j_heads[0]}),
        )
        .unwrap();
        let j_parts = dispatch_command(
            &store,
            &suite,
            "read_assistant_journal_parts",
            &json!({"snapshot": snap, "max_records": 64}),
        )
        .unwrap();
        assert!(j_parts["parts"].as_array().unwrap().len() >= 3);
        let text_part = j_parts["parts"].as_array().unwrap().iter().find(|p| p["metadata"]["kind"] == "text").unwrap();
        let j_range = dispatch_command(
            &store,
            &suite,
            "read_assistant_journal_range",
            &json!({"snapshot": snap, "part_id": text_part["part_id"], "content": text_part["bodies"][0], "offset_bytes": "0", "limit_bytes": 1024}),
        )
        .unwrap();
        assert_eq!(j_range["text"], "Final answer");
        let seal = dispatch_command(
            &store,
            &suite,
            "local_assistant_run_terminal_seal",
            &json!({"conversation_id": conv_id, "run_id": "run-j"}),
        )
        .unwrap();
        assert_eq!(seal["protocol_version"], 1);
        assert_eq!(seal["result"]["value"]["kind"], "answered");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
