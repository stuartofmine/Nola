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
use chrono::{Datelike, FixedOffset, Local, NaiveDate, NaiveTime, TimeZone, Utc};
use nola_engine::{compact_session_now, load_session_plan, load_session_todos, load_session_usage};
use nola_llm::LlmClient;
use nola_modules::a2ui::{build_nodes_from_spec, catalog_digest, commit_ui_interaction, delete_ui_surface, list_conversation_surfaces, preview_ui_surface, read_ui_surface_changes, read_ui_surface_revision, upsert_ui_surface, UiDraftPreviewInput, UiInteraction, UiSurfaceIdentity, UiSurfaceOwner, UiSurfaceRevisionRef, DEFAULT_CATALOG_ID, MAX_ACTIVE_SURFACES_PER_CONVERSATION};
use nola_modules::scheduler::spawn_background_scheduler;
use nola_modules::ModulesSuite;
use nola_store::{default_socket_path, EventPage, ServiceLock, Store, SyncChangeRecord, PROTOCOL_VERSION};
use nola_types::{new_id, now_ms, sha256_hex, GoalRecord, GoalStatus, MemoryItem, PermissionProfile, Session};
use serde_json::{json, Value};

#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};

mod compaction_aux;
mod context;
mod domain;
mod projection;
mod rooms;
mod run_journal;
#[cfg(test)]
mod tests;

#[allow(unused_imports)]
pub(crate) use domain::{
    normalize_catalog_model_entry, normalize_wire_protocol, sync_connection_model_profiles,
};
#[allow(unused_imports)]
pub(crate) use rooms::canonical_agent_directory_payload;
pub(crate) use rooms::dispatch_agent_room_command;
pub(crate) use run_journal::{
    build_and_store_terminal_seal, journal_identity_value, publish_journal_part,
};

use compaction_aux::{
    dispatch_auxiliary_invocation, dispatch_evaluate_compaction_maintenance,
    dispatch_list_conversation_auxiliary_sources, dispatch_load_auxiliary_source_slot,
    dispatch_step_compaction_summary,
};
use context::{
    build_context_usage_json, build_conversation_projection_response, dispatch_context_command,
    load_conversation_settings_str, read_permission_state, resolve_remote_model_profile,
};
use domain::{dispatch_domain_command, parse_memory_item};
use projection::{
    append_projection_event, dispatch_acknowledge_agent_command,
    dispatch_apply_active_permission_policy, dispatch_begin_receive_assistant_run_terminal,
    dispatch_claim_next_agent_command, dispatch_commit_received_assistant_run_terminal,
    dispatch_execute_task_cancellation_for_effect, dispatch_execute_ui_tool,
    dispatch_inspect_cancellation_process_owners, dispatch_load_mobile_task_wake_plan,
    dispatch_peek_agent_command, dispatch_projection_cursor_wait,
    dispatch_projection_event_suffix, dispatch_projection_snapshot, dispatch_queue_task_guidance,
    dispatch_read_admitted_remote_run_identity, dispatch_read_conversation_part_derivation,
    dispatch_read_conversation_tool_source_range, dispatch_receive_remote_expert_parent_admission,
    dispatch_release_agent_command_lease, dispatch_request_task_cancellation,
    dispatch_stage_assistant_run_terminal_manifest,
    dispatch_validate_cancellation_process_observations, latest_projection_sequence,
    projection_event_scope_key,
};
use rooms::{dispatch_expert_template_command, dispatch_reconcile_room_routing};
use run_journal::{
    decode_std_base64, dispatch_assistant_journal_reception_state, dispatch_begin_assistant_turn,
    dispatch_claim_committed_host_requests, dispatch_heartbeat_effect_delivery,
    dispatch_inspect_run_process_owners, dispatch_load_committed_host_requests,
    dispatch_load_host_execution_state, dispatch_load_run_delivery_ownership,
    dispatch_load_runtime_effect_work_schedule, dispatch_local_assistant_journal_head,
    dispatch_local_assistant_run_terminal_seal, dispatch_local_run_assistant_journal_heads,
    dispatch_local_run_message_admission_head, dispatch_next_assistant_journal_content,
    dispatch_open_assistant_journal_snapshot, dispatch_read_assistant_journal,
    dispatch_read_assistant_journal_record_chunk,
    dispatch_read_local_assistant_run_terminal_manifest, dispatch_read_run_message_admissions,
    dispatch_read_source_submission_receipt, dispatch_receive_assistant_journal_record,
    dispatch_receive_run_message_admissions, dispatch_recover_expired_run_deliveries,
    dispatch_reduce_and_accept_host_response, dispatch_reduce_and_commit_active_submission,
    dispatch_register_source_submission, dispatch_reject_source_submission,
    dispatch_run_message_admission_reception_state,
};

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
                "--archive-incompatible-store"
                    if args.get(i + 1).is_some_and(|next| !next.starts_with('-')) =>
                {
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
        use std::os::unix::io::AsRawFd;
        let listener_fd = listener.as_raw_fd();

        while !shutdown.load(Ordering::Acquire) {
            client_handles.retain(|h| !h.is_finished());
            let mut pfd = libc::pollfd {
                fd: listener_fd,
                events: libc::POLLIN,
                revents: 0,
            };
            let poll_rc = unsafe { libc::poll(&mut pfd, 1, 50) };
            if poll_rc < 0 {
                let err = std::io::Error::last_os_error();
                if err.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                if shutdown.load(Ordering::Acquire) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(5));
                continue;
            }
            if poll_rc == 0 {
                continue;
            }
            // Drain all queued incoming UDS connections immediately without sleeping.
            loop {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let _ = stream.set_nonblocking(false);
                        if !peer_matches_euid(&stream) {
                            continue;
                        }
                        if active_clients.fetch_add(1, Ordering::SeqCst)
                            >= MAX_CONCURRENT_DAEMON_CLIENTS
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
                        break;
                    }
                    Err(_) => {
                        break;
                    }
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

fn extract_sub_command(args: &Value) -> (&str, &Value) {
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
            | "list_all"
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
        | "reconcile_room_routing"
        | "begin_receive_assistant_run_terminal"
        | "stage_assistant_run_terminal_manifest"
        | "commit_received_assistant_run_terminal"
        | "claim_next_agent_command"
        | "release_agent_command_lease"
        | "acknowledge_agent_command"
        | "execute_ui_tool"
        | "apply_active_permission_policy"
        | "request_task_cancellation"
        | "execute_task_cancellation_for_effect"
        | "queue_task_guidance"
        | "receive_remote_expert_parent_admission"
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
        "projection_snapshot" => dispatch_projection_snapshot(store, suite, args),
        "sessions_list" | "list_conversations" => {
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
            let effective_before = before_ordinal.or_else(|| limit.map(|_| i64::MAX));
            Ok(build_conversation_projection_response(
                store,
                &sess,
                0,
                limit,
                effective_before,
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
            let effective_before = before_ordinal.or_else(|| limit.map(|_| i64::MAX));
            let resp = build_conversation_projection_response(
                store,
                &sess,
                0,
                limit,
                effective_before,
                true,
            );
            let event_seq = resp
                .get("event_sequence")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let proj_seq = latest_projection_sequence(
                store,
                &projection_event_scope_key("conversation", Some(&sess.id)),
            ) as i64;
            if event_seq.max(proj_seq) <= after_seq {
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
            let active_run = id.and_then(|cid| lookup_active_run(store, cid));
            let active_run_id = active_run
                .as_ref()
                .and_then(|r| r.get("run_id").and_then(|v| v.as_str()))
                .filter(|s| !s.is_empty());
            let active_assistant_id = active_run
                .as_ref()
                .and_then(|r| r.get("assistant_message_id").and_then(|v| v.as_str()))
                .filter(|s| !s.is_empty())
                .map(ToString::to_string)
                .or_else(|| active_run_id.map(|rid| format!("{rid}:assistant")));
            let (owner, state) = if let Some(run_id) = active_run_id {
                (
                    json!({
                        "kind": "local",
                        "run_id": run_id,
                        "assistant_message_id": active_assistant_id.unwrap_or_else(|| format!("{run_id}:assistant"))
                    }),
                    "active",
                )
            } else {
                (json!({"kind": "inactive"}), "inactive")
            };
            Ok(json!({
                "conversation_id": id,
                "permission_profile": profile,
                "permission_mode": profile,
                "revision": rev,
                "configuration_revision": rev,
                "owner": owner,
                "state": state,
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
                    "revision": rev,
                    "created_at_ms": sess.created_at_ms,
                    "updated_at_ms": sess.updated_at_ms
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
                .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("all"));
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

            let mut entries: Vec<Value> = Vec::new();
            let include_conversation = source_filter.is_none_or(|src| {
                src.eq_ignore_ascii_case("conversation") || src.eq_ignore_ascii_case("agent")
            });
            let include_automation =
                source_filter.is_none_or(|src| src.eq_ignore_ascii_case("automation"));

            if include_conversation {
                let sessions = store
                    .list_sessions(200)
                    .map_err(|e| ("storage_error", e.to_string()))?;
                for (idx, s) in sessions.into_iter().enumerate() {
                    let is_agent_session = s.id.starts_with("agent-")
                        || store
                            .get_kv(&format!("conv_room_assoc:{}", s.id))
                            .ok()
                            .flatten()
                            .is_some();
                    let entry_source = if is_agent_session
                        && source_filter.is_some_and(|f| f.eq_ignore_ascii_case("agent"))
                    {
                        "agent"
                    } else {
                        "conversation"
                    };
                    if let Some(src) = source_filter {
                        if src.eq_ignore_ascii_case("agent") && !is_agent_session {
                            continue;
                        }
                    }
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
                    if needs_response_filter == Some(true) && !open_waits {
                        continue;
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
                        "source": entry_source,
                        "kind": entry_source,
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
                }
            }

            if include_automation {
                let runs = suite
                    .list_records::<Value>("automation_run")
                    .unwrap_or_default();
                for r in runs {
                    let run_id = r
                        .get("id")
                        .or_else(|| r.get("run_id"))
                        .or_else(|| r.get("occurrence_id"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    if run_id.is_empty() {
                        continue;
                    }
                    let auto_id = r
                        .get("automation_id")
                        .or_else(|| r.get("definition_id"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let conv_id = r
                        .get("conversation_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or(auto_id);
                    let title = r
                        .get("title")
                        .or_else(|| r.get("name"))
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.trim().is_empty())
                        .unwrap_or(if auto_id.is_empty() { "Automation Run" } else { auto_id });
                    let summary = r
                        .get("summary")
                        .or_else(|| r.get("output"))
                        .or_else(|| r.get("result"))
                        .and_then(|v| v.as_str())
                        .map(|t| t.chars().take(160).collect::<String>())
                        .unwrap_or_default();
                    let status = r
                        .get("status")
                        .and_then(|v| v.as_str())
                        .unwrap_or("completed");
                    let needs_resp = r
                        .get("needs_response")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(matches!(status, "waiting" | "needs_response" | "needs_user"));
                    if needs_response_filter == Some(true) && !needs_resp {
                        continue;
                    }
                    if let Some(ref q) = query_filter {
                        let title_match = title.to_ascii_lowercase().contains(q);
                        let summary_match = summary.to_ascii_lowercase().contains(q);
                        let id_match = auto_id.to_ascii_lowercase().contains(q);
                        if !title_match && !summary_match && !id_match {
                            continue;
                        }
                    }
                    let created_ms = r
                        .get("created_at_ms")
                        .or_else(|| r.get("started_at_ms"))
                        .or_else(|| r.get("scheduled_for_ms"))
                        .and_then(|v| v.as_i64())
                        .unwrap_or_else(now_ms);
                    let updated_ms = r
                        .get("updated_at_ms")
                        .or_else(|| r.get("completed_at_ms"))
                        .and_then(|v| v.as_i64())
                        .unwrap_or(created_ms);
                    let assistant_msg_id = r
                        .get("assistant_message_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    entries.push(json!({
                        "id": run_id,
                        "activity_id": run_id,
                        "run_id": run_id,
                        "automation_id": auto_id,
                        "conversation_id": conv_id,
                        "assistant_message_id": assistant_msg_id,
                        "source": "automation",
                        "kind": "automation",
                        "title": title,
                        "summary": summary,
                        "status": status,
                        "needs_response": needs_resp,
                        "is_current": false,
                        "message_count": 0,
                        "event_count": 0,
                        "created_at": crate::web_api::ms_to_rfc3339(created_ms),
                        "updated_at": crate::web_api::ms_to_rfc3339(updated_ms),
                        "created_at_ms": created_ms,
                        "completed_at_ms": updated_ms,
                        "updated_at_ms": updated_ms
                    }));
                }
            }

            entries.sort_by(|a, b| {
                let b_ts = b.get("updated_at_ms").and_then(|v| v.as_i64()).unwrap_or(0);
                let a_ts = a.get("updated_at_ms").and_then(|v| v.as_i64()).unwrap_or(0);
                b_ts.cmp(&a_ts)
            });
            entries.truncate(limit);

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
            let mut conversations: Vec<Value> = Vec::with_capacity(sessions.len());
            let mut attention_list: Vec<Value> = Vec::new();
            for s in &sessions {
                let conv = crate::web_api::session_to_conversation_json(store, s);
                let revision = conv
                    .get("revision")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(1)
                    .max(1);
                let open_waits: Vec<Value> = store
                    .get_kv(&format!("open_waits:{}", s.id))
                    .ok()
                    .flatten()
                    .and_then(|v| v.as_array().cloned())
                    .unwrap_or_default();
                let owned_task_waits: Vec<Value> = store
                    .get_kv(&format!("owned_task_waits:{}", s.id))
                    .ok()
                    .flatten()
                    .and_then(|v| v.as_array().cloned())
                    .unwrap_or_default();
                let owned_task_permissions: Vec<Value> = store
                    .get_kv(&format!("owned_task_permissions:{}", s.id))
                    .ok()
                    .flatten()
                    .and_then(|v| v.as_array().cloned())
                    .unwrap_or_default();
                let permission_projection = store
                    .get_kv(&format!("permission_projection:{}", s.id))
                    .ok()
                    .flatten()
                    .filter(|v| !v.is_null());
                let active_run = store
                    .get_kv(&format!("active_run_owner:{}", s.id))
                    .ok()
                    .flatten()
                    .filter(|v| {
                        v.get("run_id").and_then(|r| r.as_str()).is_some()
                            && v.get("assistant_message_id")
                                .and_then(|r| r.as_str())
                                .is_some()
                    });
                let needs_response = !open_waits.is_empty()
                    || !owned_task_waits.is_empty()
                    || !owned_task_permissions.is_empty();
                if needs_response {
                    attention_list.push(json!({
                        "conversation_id": s.id,
                        "title": s.title,
                        "reason": "waiting_for_user_input"
                    }));
                }
                conversations.push(json!({
                    "conversation_id": s.id,
                    "title": s.title,
                    "revision": revision,
                    "updated_at_ms": s.updated_at_ms,
                    "needs_response": needs_response,
                    "active_run": active_run.unwrap_or(Value::Null),
                    "permission_projection": permission_projection.unwrap_or(Value::Null),
                    "open_waits": open_waits,
                    "owned_task_waits": owned_task_waits,
                    "owned_task_permissions": owned_task_permissions,
                    "expert_contexts": []
                }));
            }
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
            let waits: Vec<Value> = conv_id
                .and_then(|cid| store.get_kv(&format!("open_waits:{cid}")).ok().flatten())
                .and_then(|v| v.as_array().cloned())
                .unwrap_or_default()
                .into_iter()
                .filter(|w| {
                    let matches_conv = conv_id.is_none_or(|cid| {
                        w.get("conversation_id").and_then(|v| v.as_str()) == Some(cid)
                    });
                    let is_open = w
                        .pointer("/wait/state/type")
                        .and_then(|v| v.as_str())
                        .map(|s| s == "open")
                        .unwrap_or(true);
                    matches_conv && is_open
                })
                .collect();
            Ok(Value::Array(waits))
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
            append_projection_event(
                store,
                id,
                json!({
                    "type": "set_title",
                    "title": title,
                    "revision": next_rev
                }),
            );
            append_projection_event(
                store,
                "__conversations__",
                json!({
                    "type": "invalidate_projection",
                    "revision": next_rev
                }),
            );
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
            append_projection_event(
                store,
                id,
                json!({
                    "type": "invalidate_projection",
                    "revision": next_cfg
                }),
            );
            append_projection_event(
                store,
                "__conversations__",
                json!({
                    "type": "invalidate_projection",
                    "revision": next_cfg
                }),
            );
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
        "events_after" => {
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
        "projection_cursor_wait" => dispatch_projection_cursor_wait(store, args),
        "projection_event_suffix" => dispatch_projection_event_suffix(store, args),
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
            let mut occurrences: Vec<Value> = suite
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
                .collect();

            let frequency = input
                .get("frequency")
                .or_else(|| args.get("frequency"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let once_at_ms = input
                .get("once_at_ms")
                .or_else(|| args.get("once_at_ms"))
                .and_then(|v| v.as_i64());

            if frequency.is_some() || once_at_ms.is_some() {
                let computed = compute_scheduled_automation_occurrences(
                    def_id,
                    input,
                    after_ms,
                    through_ms,
                    limit,
                );
                let mut seen_ts: HashSet<i64> = occurrences
                    .iter()
                    .filter_map(|r| r.get("scheduled_for_ms").and_then(|v| v.as_i64()))
                    .collect();
                for occ in computed {
                    let ts = occ.get("scheduled_for_ms").and_then(|v| v.as_i64());
                    if ts.is_none_or(|t| seen_ts.insert(t)) {
                        occurrences.push(occ);
                    }
                }
            }

            occurrences.sort_by_key(|r| {
                r.get("scheduled_for_ms")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(i64::MAX)
            });
            occurrences.truncate(limit);
            Ok(json!({"occurrences": occurrences, "items": occurrences}))
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
            let q_lower = query.to_lowercase();
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
                for (ordinal, m) in msgs.into_iter().enumerate() {
                    let content = m.get("content").and_then(|v| v.as_str()).unwrap_or("");
                    let match_char_idx = if q_lower.is_empty() {
                        Some(0usize)
                    } else {
                        let lower = content.to_lowercase();
                        lower
                            .find(&q_lower)
                            .map(|byte_idx| lower[..byte_idx].chars().count())
                    };
                    if let Some(start_char) = match_char_idx {
                        let msg_id = m
                            .get("id")
                            .and_then(|v| v.as_str())
                            .map(ToString::to_string)
                            .unwrap_or_else(|| format!("msg-{ordinal}"));
                        let part_id = format!("{msg_id}:text:0");
                        let snippet_start = start_char.saturating_sub(40);
                        let snippet: String =
                            content.chars().skip(snippet_start).take(240).collect();
                        matches.push(json!({
                            "conversation_id": cid,
                            "message_id": msg_id,
                            "part_id": part_id,
                            "message_ordinal": ordinal as i64,
                            "seq": m.get("seq").cloned().unwrap_or(json!(ordinal)),
                            "role": m.get("role").cloned().unwrap_or(json!("assistant")),
                            "snippet": snippet,
                            "start_character": start_char as i64
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
            Ok(Value::Array(matches))
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
                .or_else(|| input.get("goalID"))
                .or_else(|| input.get("id"))
                .or_else(|| args.get("goal_id"))
                .or_else(|| args.get("goalID"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let conv_id = input
                .get("conversation_id")
                .or_else(|| input.get("conversationID"))
                .or_else(|| args.get("conversation_id"))
                .or_else(|| args.get("conversationID"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let exec_id = input
                .get("run_id")
                .or_else(|| input.get("execution_id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(new_id);
            let contract_id = format!("goal-contract:{goal_id}:{exec_id}");
            let control_token = format!("goal-ctrl:{goal_id}");
            let assistant_message_id = format!("{exec_id}:assistant");
            let host_id = store
                .get_kv("host_id")
                .ok()
                .flatten()
                .and_then(|v| v.as_str().map(ToString::to_string))
                .unwrap_or_else(|| "local-host".into());
            let ts = now_ms();
            let cfg = crate::config::load_active_config(store);
            let primary_route = match crate::config::resolve_model_route(&cfg) {
                Ok(r) => json!({
                    "kind": "native",
                    "connection_id": r.connection_id,
                    "model_id": r.model,
                    "endpoint": r.endpoint,
                    "protocol": r.protocol.as_str(),
                    "context_window": r.context_window
                }),
                Err(_) => json!({
                    "kind": "native",
                    "connection_id": "default",
                    "model_id": "default",
                    "endpoint": "",
                    "protocol": "chat_completions",
                    "context_window": 128_000
                }),
            };
            let owner = json!({
                "execution_id": exec_id,
                "run_id": exec_id,
                "goal_id": goal_id,
                "conversation_id": conv_id,
                "contract_id": contract_id,
                "assistant_message_id": assistant_message_id,
                "control_token": control_token,
                "host_id": host_id,
                "prepared_at_ms": ts
            });
            if !goal_id.is_empty() {
                let _ = store.set_kv(&format!("goal_exec_owner:{goal_id}"), &owner);
                if !conv_id.is_empty() {
                    let _ = store.set_kv(
                        &format!("goal_exec_owner_run:{conv_id}:{goal_id}:{exec_id}"),
                        &owner,
                    );
                    let _ = store.set_kv(&format!("goal_for_conv:{conv_id}"), &json!(goal_id));
                }
            }
            if !conv_id.is_empty() {
                let _ = store.set_kv(&format!("goal_exec_owner_conv:{conv_id}"), &owner);
            }
            Ok(json!({
                "ok": true,
                "execution_id": exec_id,
                "goal_id": goal_id,
                "conversation_id": conv_id,
                "owner": owner,
                "ready": true,
                "identity": {
                    "conversation_id": conv_id,
                    "goal_id": goal_id,
                    "run_id": exec_id,
                    "contract_id": contract_id,
                    "assistant_message_id": assistant_message_id,
                    "control_token": control_token
                },
                "template": {
                    "core_config": {
                        "turn_identity": {
                            "conversation_id": conv_id,
                            "goal_id": goal_id,
                            "run_id": exec_id,
                            "assistant_message_id": assistant_message_id,
                            "host_invocation": {
                                "primary": primary_route
                            }
                        }
                    }
                }
            }))
        }
        "load_goal_execution_owner" => {
            let goal_id = args
                .get("goal_id")
                .or_else(|| args.get("goalID"))
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty());
            let conv_id = args
                .get("conversation_id")
                .or_else(|| args.get("conversationID"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty());
            let run_id = args
                .get("run_id")
                .or_else(|| args.get("runID"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty());
            let stored = match (conv_id, goal_id, run_id) {
                (Some(cid), Some(gid), Some(rid)) => store
                    .get_kv(&format!("goal_exec_owner_run:{cid}:{gid}:{rid}"))
                    .ok()
                    .flatten(),
                _ => None,
            }
            .or_else(|| {
                goal_id.and_then(|gid| {
                    store
                        .get_kv(&format!("goal_exec_owner:{gid}"))
                        .ok()
                        .flatten()
                })
            })
            .or_else(|| {
                conv_id.and_then(|cid| {
                    store
                        .get_kv(&format!("goal_exec_owner_conv:{cid}"))
                        .ok()
                        .flatten()
                })
            });
            let Some(owner) = stored.filter(|v| v.is_object()) else {
                return Ok(Value::Null);
            };
            let eff_goal_id = owner
                .get("goal_id")
                .and_then(|v| v.as_str())
                .or(goal_id)
                .unwrap_or("");
            let eff_run_id = run_id
                .or_else(|| owner.get("run_id").and_then(|v| v.as_str()))
                .or_else(|| owner.get("execution_id").and_then(|v| v.as_str()))
                .unwrap_or("");
            let contract_id = owner
                .get("contract_id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(|| format!("goal-contract:{eff_goal_id}:{eff_run_id}"));
            let assistant_message_id = owner
                .get("assistant_message_id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(|| format!("{eff_run_id}:assistant"));
            let control_token = owner
                .get("control_token")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(|| format!("goal-ctrl:{eff_goal_id}"));
            Ok(json!({
                "goal_id": eff_goal_id,
                "run_id": eff_run_id,
                "contract_id": contract_id,
                "assistant_message_id": assistant_message_id,
                "control_token": control_token,
                "conversation_id": conv_id.or_else(|| owner.get("conversation_id").and_then(|v| v.as_str())),
                "owner": owner
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
                        .unwrap_or(match ev.kind.as_str() {
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
                    .unwrap_or(match ev.kind.as_str() {
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
            let experts: Vec<Value> = input
                .get("selected_experts")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .enumerate()
                        .map(|(idx, e)| {
                            let agent_id = e
                                .get("agent_id")
                                .or_else(|| e.get("id"))
                                .and_then(|v| v.as_str())
                                .filter(|s| !s.trim().is_empty())
                                .unwrap_or("expert")
                                .to_string();
                            let participant_id = e
                                .get("participant_id")
                                .or_else(|| e.get("id"))
                                .and_then(|v| v.as_str())
                                .filter(|s| !s.trim().is_empty())
                                .map(ToString::to_string)
                                .unwrap_or_else(|| format!("participant-{agent_id}-{idx}"));
                            let context_conversation_id = e
                                .get("context_conversation_id")
                                .and_then(|v| v.as_str())
                                .filter(|s| !s.trim().is_empty())
                                .map(ToString::to_string)
                                .unwrap_or_else(|| format!("{conv_id}:expert:{participant_id}"));
                            let agent_revision = e
                                .get("agent_revision")
                                .and_then(|v| v.as_i64())
                                .unwrap_or(1);
                            json!({
                                "participant_id": participant_id,
                                "agent_id": agent_id,
                                "agent_revision": agent_revision,
                                "context_conversation_id": context_conversation_id
                            })
                        })
                        .collect()
                })
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
            let source_conv_id = input
                .get("source_conversation_id")
                .or_else(|| input.get("conversation_id"))
                .or_else(|| input.get("id"))
                .or_else(|| args.get("source_conversation_id"))
                .or_else(|| args.get("conversation_id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .ok_or(("invalid_argument", "missing conversation_id".into()))?;
            let target_conv_id = input
                .get("target_conversation_id")
                .or_else(|| args.get("target_conversation_id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .unwrap_or(source_conv_id);
            if let Some(from_seq) = input
                .get("from_seq")
                .or_else(|| input.get("truncate_from_seq"))
                .or_else(|| input.get("target_seq"))
                .and_then(|v| v.as_i64())
            {
                store
                    .truncate_events_from(source_conv_id, from_seq)
                    .map_err(|e| ("storage_error", e.to_string()))?;
                synchronize_session_state_after_truncate(store, source_conv_id);
            }
            let op_id = input
                .get("operation_id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(new_id);
            let source_msg_id = input
                .get("source_message_id")
                .or_else(|| input.get("message_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let edited_text = input
                .get("edited_text")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let title = input
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("Edited conversation");
            let created_at_ms = input
                .get("created_at_ms")
                .and_then(|v| v.as_i64())
                .unwrap_or_else(now_ms);
            let canonical_input = json!({
                "operation_id": op_id,
                "source_conversation_id": source_conv_id,
                "source_message_id": source_msg_id,
                "expected_source_revision": input.get("expected_source_revision").and_then(|v| v.as_i64()).unwrap_or(1),
                "expected_source_message_revision": input.get("expected_source_message_revision").and_then(|v| v.as_u64()).unwrap_or(1),
                "edited_text": edited_text,
                "target_conversation_id": target_conv_id,
                "title": title,
                "created_at_ms": created_at_ms
            });
            let cfg = crate::config::load_active_config(store);
            let (profile_id, endpoint, model_name, proto_str) =
                match crate::config::resolve_model_route(&cfg) {
                    Ok(r) => (
                        format!("{}:{}", r.connection_id, r.model),
                        r.endpoint,
                        r.model,
                        r.protocol.as_str().to_string(),
                    ),
                    Err(_) => (
                        "default".to_string(),
                        String::new(),
                        "default".to_string(),
                        "chat_completions".to_string(),
                    ),
                };
            let source_run_id = format!("src-run-{source_conv_id}");
            let source_assistant_id = format!("{source_run_id}:assistant");
            let body_sha = sha256_hex(edited_text.as_bytes());
            let config_sha = sha256_hex(format!("{profile_id}:{endpoint}:{proto_str}").as_bytes());
            let user_message_id = format!("{op_id}:user");
            let assistant_message_id = format!("{op_id}:assistant");
            let prepared = json!({
                "input": canonical_input,
                "source": {
                    "conversation_id": source_conv_id,
                    "message_id": source_msg_id,
                    "run_id": source_run_id,
                    "assistant_message_id": source_assistant_id,
                    "body_sha256": body_sha,
                    "invocation_config_sha256": config_sha,
                    "model": {
                        "version": 1,
                        "primary": {
                            "profile_id": profile_id,
                            "endpoint": endpoint,
                            "model_name": model_name,
                            "protocol": proto_str
                        },
                        "fallbacks": [],
                        "generation": {},
                        "output_admission": {},
                        "model_capability": {
                            "profileId": profile_id,
                            "revision": 1,
                            "instructionProfile": "default",
                            "nativeToolCalls": true,
                            "parallelToolCalls": true,
                            "reasoning": false,
                            "reasoningContinuity": false,
                            "promptCache": false,
                            "visionInput": false,
                            "nativePdfInput": false,
                            "contextWindowTokens": 128000,
                            "maxOutputTokens": 16384,
                            "requestContinuation": false,
                            "maxRequestContinuations": 0,
                            "streamText": true,
                            "streamReasoning": false,
                            "streamToolCalls": true,
                            "directToolCapabilityIds": []
                        }
                    }
                },
                "user_message_id": user_message_id,
                "assistant_message_id": assistant_message_id,
                "run_id": op_id,
                "admitted_run_id": Value::Null
            });
            store
                .set_kv(&format!("prepared_edited_resend:{target_conv_id}"), &prepared)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(prepared)
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
            let assistant_msg_id = identity
                .get("assistant_message_id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(|| format!("{sub_id}:assistant"));
            let source_host_id = identity
                .get("source_host_id")
                .and_then(|v| v.as_str())
                .unwrap_or("remote");
            let active_run = json!({
                "conversation_id": conv_id,
                "run_id": sub_id,
                "kind": "conversation",
                "assistant_message_id": assistant_msg_id,
                "source_host_id": source_host_id,
                "next_event_sequence": 1,
                "status": "running",
                "request_digest": digest,
                "updated_at_ms": now_ms()
            });
            let _ = store.set_kv(&format!("current_run:{conv_id}"), &active_run);
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
        "peek_agent_command" => dispatch_peek_agent_command(store, args),
        "claim_next_agent_command" => dispatch_claim_next_agent_command(store, args),
        "release_agent_command_lease" => dispatch_release_agent_command_lease(store, args),
        "acknowledge_agent_command" => dispatch_acknowledge_agent_command(store, args),
        "promote_queued_assistant_turn" | "apply_pending_task_notice" => Ok(Value::Null),
        "reconcile_room_routing" => dispatch_reconcile_room_routing(suite, args),
        "read_conversation_part_derivation" => {
            dispatch_read_conversation_part_derivation(store, args)
        }
        "read_conversation_tool_source_range" => {
            dispatch_read_conversation_tool_source_range(store, args)
        }
        "begin_receive_assistant_run_terminal" => {
            dispatch_begin_receive_assistant_run_terminal(store, args)
        }
        "stage_assistant_run_terminal_manifest" => {
            dispatch_stage_assistant_run_terminal_manifest(store, args)
        }
        "commit_received_assistant_run_terminal" => {
            dispatch_commit_received_assistant_run_terminal(store, args)
        }
        "list_conversation_auxiliary_sources" => {
            dispatch_list_conversation_auxiliary_sources(store, args)
        }
        "execute_ui_tool" => dispatch_execute_ui_tool(store, args),
        "apply_active_permission_policy" => dispatch_apply_active_permission_policy(store, args),
        "load_mobile_task_wake_plan" => dispatch_load_mobile_task_wake_plan(suite, args),
        "request_task_cancellation" => dispatch_request_task_cancellation(store, suite, args),
        "execute_task_cancellation_for_effect" => {
            dispatch_execute_task_cancellation_for_effect(store, suite, args)
        }
        "inspect_cancellation_process_owners" => {
            dispatch_inspect_cancellation_process_owners(args)
        }
        "validate_cancellation_process_observations" => {
            dispatch_validate_cancellation_process_observations(args)
        }
        "queue_task_guidance" => dispatch_queue_task_guidance(store, args),
        "receive_remote_expert_parent_admission" => {
            dispatch_receive_remote_expert_parent_admission(store, args)
        }
        "read_admitted_remote_run_identity" => {
            dispatch_read_admitted_remote_run_identity(store, args)
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
        "next_assistant_journal_content" => dispatch_next_assistant_journal_content(store, args),
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
            if let Some(identity) = args.get("identity").filter(|v| v.is_object()) {
                let conv_id = identity
                    .get("conversation_id")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
                    .ok_or(("invalid_argument", "missing 'conversation_id'".into()))?;
                let run_id = identity
                    .get("run_id")
                    .or_else(|| identity.get("submission_id"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if !run_id.is_empty() {
                    if let Ok(Some(saved)) =
                        store.get_kv(&format!("run_context_usage:{conv_id}:{run_id}"))
                    {
                        return Ok(saved);
                    }
                }
                return Ok(Value::Null);
            }
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
                        let conv_id = entry
                            .get("conversationID")
                            .or_else(|| entry.get("conversation_id"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        if let Some(msg) = entry.get("message") {
                            let mut upload_map: std::collections::HashMap<(String, String), String> =
                                std::collections::HashMap::new();
                            if let Some(uploads) = entry.get("uploads").and_then(|v| v.as_array()) {
                                for up in uploads {
                                    let uid = up
                                        .get("uploadID")
                                        .or_else(|| up.get("upload_id"))
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("");
                                    let pid = up
                                        .get("partID")
                                        .or_else(|| up.get("part_id"))
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("");
                                    let field = up
                                        .get("field")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("content_text");
                                    if !uid.is_empty() && !pid.is_empty() {
                                        if let Ok(Some(u)) =
                                            store.get_kv(&format!("content_upload:{uid}"))
                                        {
                                            if let Some(body) =
                                                u.get("content").and_then(|v| v.as_str())
                                            {
                                                upload_map.insert(
                                                    (pid.to_string(), field.to_string()),
                                                    body.to_string(),
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                            let mut restored_parts: Vec<Value> = Vec::new();
                            let mut text_segments: Vec<String> = Vec::new();
                            if let Some(parts) = msg.get("parts").and_then(|v| v.as_array()) {
                                for p in parts {
                                    let mut p_obj = p.clone();
                                    let pid = p
                                        .get("id")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .to_string();
                                    let uploaded_text = upload_map
                                        .get(&(pid.clone(), "content_text".to_string()))
                                        .or_else(|| {
                                            upload_map
                                                .get(&(pid.clone(), "contentText".to_string()))
                                        })
                                        .cloned();
                                    let uploaded_json = upload_map
                                        .get(&(pid.clone(), "content_json".to_string()))
                                        .or_else(|| {
                                            upload_map
                                                .get(&(pid.clone(), "contentJson".to_string()))
                                        })
                                        .cloned();
                                    let resolved_text = uploaded_text.unwrap_or_else(|| {
                                        p.get("content_text")
                                            .or_else(|| p.get("contentText"))
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("")
                                            .to_string()
                                    });
                                    let resolved_json = uploaded_json.or_else(|| {
                                        p.get("content_json")
                                            .or_else(|| p.get("contentJson"))
                                            .and_then(|v| v.as_str())
                                            .map(ToString::to_string)
                                    });
                                    if let Some(obj) = p_obj.as_object_mut() {
                                        obj.insert("content_text".into(), json!(resolved_text));
                                        if let Some(ref j) = resolved_json {
                                            obj.insert("content_json".into(), json!(j));
                                        }
                                    }
                                    let kind = p
                                        .get("kind")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("text");
                                    if kind == "text" && !resolved_text.is_empty() {
                                        text_segments.push(resolved_text);
                                    }
                                    restored_parts.push(p_obj);
                                }
                            }
                            if !conv_id.is_empty() {
                                let role = msg
                                    .get("role")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("user");
                                let event_kind = match role {
                                    "user" => "user_message",
                                    "system" => "system_message",
                                    _ => "assistant_message",
                                };
                                let msg_id = msg
                                    .get("id")
                                    .and_then(|v| v.as_str())
                                    .filter(|s| !s.trim().is_empty())
                                    .map(ToString::to_string)
                                    .unwrap_or_else(new_id);
                                let mut payload = json!({
                                    "id": msg_id,
                                    "message_id": msg_id,
                                    "role": role,
                                    "content": text_segments.join("\n"),
                                    "parts": restored_parts,
                                    "status": msg.get("status").cloned().unwrap_or_else(|| json!("completed")),
                                    "ordinal": msg.get("ordinal").cloned().unwrap_or(Value::Null),
                                    "created_at_ms": msg.get("created_at_ms").or_else(|| msg.get("createdAtMs")).cloned().unwrap_or_else(|| json!(now_ms())),
                                    "completed_at_ms": msg.get("completed_at_ms").or_else(|| msg.get("completedAtMs")).cloned().unwrap_or(Value::Null)
                                });
                                if let Some(run_id) = msg.get("run_id").or_else(|| msg.get("runID")).filter(|v| !v.is_null()) {
                                    payload["run_id"] = run_id.clone();
                                }
                                let _ = store.append_event(&conv_id, event_kind, payload, 50_000);
                                append_projection_event(
                                    store,
                                    &conv_id,
                                    json!({
                                        "type": "invalidate_projection",
                                        "revision": cur_rev + 1
                                    }),
                                );
                            }
                        }
                        message_count += 1;
                    }
                    Some("part_derivation") => {
                        if let Some(record) = entry.get("record") {
                            let cid = record
                                .pointer("/target/owner/conversation_id")
                                .or_else(|| record.pointer("/target/owner/conversationID"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            let mid = record
                                .pointer("/target/owner/message_id")
                                .or_else(|| record.pointer("/target/owner/messageID"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            let pid = record
                                .pointer("/target/owner/part_id")
                                .or_else(|| record.pointer("/target/owner/partID"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            if !cid.is_empty() && !mid.is_empty() && !pid.is_empty() {
                                let _ = store.set_kv(
                                    &format!("part_derivation:{cid}:{mid}:{pid}"),
                                    record,
                                );
                            }
                        }
                    }
                    Some("tool_source") => {
                        if let Some(claim) = entry.get("claim") {
                            let cid = claim
                                .pointer("/owner/conversation_id")
                                .or_else(|| claim.pointer("/owner/conversationID"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            let mid = claim
                                .pointer("/owner/message_id")
                                .or_else(|| claim.pointer("/owner/messageID"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            let pid = claim
                                .pointer("/owner/part_id")
                                .or_else(|| claim.pointer("/owner/partID"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            let uid = entry
                                .get("uploadID")
                                .or_else(|| entry.get("upload_id"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            let body_json = if !uid.is_empty() {
                                store
                                    .get_kv(&format!("content_upload:{uid}"))
                                    .ok()
                                    .flatten()
                                    .and_then(|u| {
                                        u.get("content")
                                            .and_then(|v| v.as_str())
                                            .map(ToString::to_string)
                                    })
                                    .unwrap_or_default()
                            } else {
                                String::new()
                            };
                            if !cid.is_empty() && !mid.is_empty() && !pid.is_empty() {
                                let key = format!("tool_sources:{cid}:{mid}:{pid}");
                                let mut list = store
                                    .get_kv(&key)
                                    .ok()
                                    .flatten()
                                    .and_then(|v| v.as_array().cloned())
                                    .unwrap_or_default();
                                list.push(json!({
                                    "claim": claim,
                                    "body_json": body_json
                                }));
                                let _ = store.set_kv(&key, &Value::Array(list));
                            }
                        }
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
                    if content.as_bytes()[offset_bytes..committed_now] != text.as_bytes()[..overlap] {
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
                "list_all" => {
                    let tasks: Vec<Value> = suite
                        .list_records("runtime_task")
                        .map_err(|e| ("storage_error", e.to_string()))?;
                    Ok(Value::Array(tasks))
                }
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
                .pointer("/header/identity")
                .or_else(|| record.get("identity"))
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
            let is_apple_ref = content_ref.get("content_generation").is_some();
            let content_id = content_ref
                .get("field")
                .or_else(|| content_ref.get("content_id"))
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
                    if let Some(b64) = ch.get("data_base64").and_then(|v| v.as_str()) {
                        if let Some(decoded) = decode_std_base64(b64) {
                            text.push_str(&String::from_utf8_lossy(&decoded));
                        }
                    } else if let Some(piece) = ch
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
            let content_generation = content_ref
                .get("content_generation")
                .and_then(|v| v.as_str())
                .unwrap_or("1");
            let chunk_count = content_ref
                .get("chunk_count")
                .and_then(|v| v.as_str())
                .unwrap_or(if total_bytes > 0 { "1" } else { "0" });
            let content_root = content_ref
                .get("content_root")
                .and_then(|v| v.as_str())
                .unwrap_or(&digest);
            let body_ref = if is_apple_ref {
                json!({
                    "field": content_id,
                    "content_generation": content_generation,
                    "utf8_bytes": total_bytes.to_string(),
                    "chunk_count": chunk_count,
                    "content_root": content_root,
                    "content_id": content_id,
                    "part_id": part_id,
                    "total_utf8_bytes": total_bytes.to_string(),
                    "sha256": digest,
                    "staged": true
                })
            } else {
                json!({
                    "content_id": content_id,
                    "part_id": part_id,
                    "total_utf8_bytes": total_bytes.to_string(),
                    "utf8_bytes": total_bytes,
                    "sha256": digest,
                    "staged": true
                })
            };
            let part_rec = json!({
                "identity": identity,
                "part_id": part_id,
                "generation": "1",
                "revision": next_rev.to_string(),
                "metadata_revision": next_rev.to_string(),
                "order_key": {"section": "body", "position": "0"},
                "metadata": record.pointer("/batch/head/metadata").or_else(|| record.get("metadata")).cloned().unwrap_or_else(|| json!({"kind": "text"})),
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
        "load_task_child_conversation_window" => {
            let dest = args.get("destination").unwrap_or(args);
            let child_conv_id = dest
                .get("child_conversation_id")
                .or_else(|| dest.get("childConversationID"))
                .or_else(|| args.get("child_conversation_id"))
                .or_else(|| args.get("conversation_id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .ok_or(("invalid_argument", "missing 'child_conversation_id'".into()))?;
            let before_ordinal = args
                .get("before_ordinal")
                .or_else(|| args.get("beforeOrdinal"))
                .and_then(|v| v.as_i64());
            let limit = args
                .get("limit")
                .and_then(|v| v.as_u64())
                .map(|n| n.clamp(1, 500) as usize);
            let effective_before = before_ordinal.or_else(|| limit.map(|_| i64::MAX));
            let sess = match get_session_case_insensitive(store, child_conv_id)
                .map_err(|e| ("storage_error", e.to_string()))?
            {
                Some(s) => s,
                None => Session {
                    id: child_conv_id.to_string(),
                    title: "Task Child".to_string(),
                    workspace_root: String::new(),
                    model_connection: None,
                    model_id: None,
                    created_at_ms: now_ms(),
                    updated_at_ms: now_ms(),
                },
            };
            let mut resp = build_conversation_projection_response(
                store,
                &sess,
                0,
                limit,
                effective_before,
                true,
            );
            let ownership = json!({
                "kind": "task_child",
                "task_id": dest.get("task_id").or_else(|| dest.get("taskID")).cloned().unwrap_or(Value::Null),
                "parent_conversation_id": dest.get("parent_conversation_id").or_else(|| dest.get("parentConversationID")).cloned().unwrap_or(Value::Null),
                "parent_run_id": dest.get("parent_run_id").or_else(|| dest.get("parentRunID")).cloned().unwrap_or(Value::Null),
                "child_conversation_id": child_conv_id,
                "child_run_id": dest.get("child_run_id").or_else(|| dest.get("childRunID")).cloned().unwrap_or(Value::Null)
            });
            if let Some(proj) = resp.get_mut("projection").and_then(|v| v.as_object_mut()) {
                proj.insert("execution_ownership".into(), ownership.clone());
            }
            if let Some(obj) = resp.as_object_mut() {
                obj.entry("assistant_journal_heads".to_string())
                    .or_insert_with(|| json!([]));
                obj.insert("execution_ownership".into(), ownership);
            }
            Ok(resp)
        }
        "load_exact_run_trace_replay" => {
            let conv_id = args
                .get("conversation_id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .ok_or(("invalid_argument", "missing 'conversation_id'".into()))?;
            let run_id = args
                .get("run_id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .ok_or(("invalid_argument", "missing 'run_id'".into()))?;
            let assistant_msg_id = args
                .get("assistant_message_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let after_seq = args
                .get("after_sequence")
                .and_then(|v| v.as_i64())
                .unwrap_or(0)
                .max(0);
            let requested_through = args.get("through_sequence").and_then(|v| v.as_i64());
            let limit = args
                .get("limit")
                .and_then(|v| v.as_u64())
                .unwrap_or(256)
                .clamp(1, 256) as usize;
            let trace_id = format!("trace-{run_id}");
            let stored = store
                .get_kv(&format!("run_trace:{conv_id}:{run_id}"))
                .ok()
                .flatten()
                .and_then(|v| v.as_array().cloned())
                .unwrap_or_default();
            let matching: Vec<Value> = stored
                .into_iter()
                .filter(|ev| {
                    assistant_msg_id.is_empty()
                        || ev
                            .get("message_id")
                            .or_else(|| ev.get("messageId"))
                            .and_then(|v| v.as_str())
                            .is_none_or(|m| m == assistant_msg_id)
                })
                .collect();
            let max_seq = matching
                .iter()
                .filter_map(|e| e.get("sequence").and_then(|v| v.as_i64()))
                .max()
                .unwrap_or(0);
            let frontier = requested_through
                .map(|t| t.max(after_seq))
                .unwrap_or_else(|| max_seq.max(after_seq));
            let mut page_events = Vec::new();
            let mut cursor = after_seq;
            for ev in matching {
                let seq = ev.get("sequence").and_then(|v| v.as_i64()).unwrap_or(0);
                if seq == cursor + 1 && seq <= frontier && page_events.len() < limit {
                    page_events.push(ev);
                    cursor += 1;
                }
            }
            let effective_trace_id = page_events
                .first()
                .and_then(|e| {
                    e.get("trace_id")
                        .or_else(|| e.get("traceId"))
                        .and_then(|v| v.as_str())
                })
                .map(ToString::to_string)
                .unwrap_or(trace_id);
            Ok(json!({
                "trace_id": effective_trace_id,
                "through_sequence": frontier,
                "next_sequence": cursor,
                "events": page_events
            }))
        }
        "recover_observed_orphaned_run" => {
            let conv_id = args
                .get("conversation_id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .ok_or(("invalid_argument", "missing 'conversation_id'".into()))?;
            let run_id = args
                .get("expected_run_id")
                .or_else(|| args.get("run_id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .ok_or(("invalid_argument", "missing 'expected_run_id'".into()))?;
            let interrupted_at_ms = args
                .get("interrupted_at_ms")
                .and_then(|v| v.as_i64())
                .unwrap_or_else(now_ms);
            for run_key in [
                format!("committed_run:{run_id}"),
                format!("active_run:{conv_id}:{run_id}"),
            ] {
                if let Ok(Some(mut run)) = store.get_kv(&run_key) {
                    if let Some(obj) = run.as_object_mut() {
                        obj.insert("turn_status".into(), json!("cancelled"));
                        obj.insert("orphaned_recovered".into(), json!(true));
                        obj.insert("interrupted_at_ms".into(), json!(interrupted_at_ms));
                        if let Some(p) =
                            obj.get_mut("pending_permissions").and_then(|v| v.as_array_mut())
                        {
                            p.clear();
                        }
                    }
                    let _ = store.set_kv(&run_key, &run);
                }
            }
            let _ = store.set_kv(&format!("open_waits:{conv_id}"), &json!([]));
            let _ = store.set_kv(
                &format!("orphaned_run_recovery:{conv_id}:{run_id}"),
                &json!({
                    "conversation_id": conv_id,
                    "run_id": run_id,
                    "status": "interrupted",
                    "interrupted_at_ms": interrupted_at_ms,
                    "interrupted_at": args.get("interrupted_at").cloned().unwrap_or(Value::Null),
                    "observed_deliveries": args.get("observed_deliveries").cloned().unwrap_or_else(|| json!([]))
                }),
            );
            Ok(json!({
                "ok": true,
                "recovered": true,
                "conversation_id": conv_id,
                "run_id": run_id,
                "status": "interrupted",
                "interrupted_at_ms": interrupted_at_ms
            }))
        }
        "query_claimable_memory_jobs" => {
            let input = args.get("input").unwrap_or(args);
            let conv_id = input
                .get("conversation_id")
                .or_else(|| args.get("conversation_id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .ok_or(("invalid_argument", "missing 'conversation_id'".into()))?;
            let now_epoch_ms = input
                .get("now_epoch_ms")
                .or_else(|| args.get("now_epoch_ms"))
                .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)))
                .unwrap_or_else(now_ms);
            let jobs = store
                .get_kv(&format!("memory_jobs:{conv_id}"))
                .ok()
                .flatten()
                .and_then(|v| v.as_array().cloned())
                .unwrap_or_default();
            let claimable: Vec<Value> = jobs
                .into_iter()
                .filter(|j| {
                    let status = j
                        .get("status")
                        .and_then(|v| v.as_str())
                        .unwrap_or("pending");
                    match status {
                        "pending" | "queued" => true,
                        "retry_scheduled" => j
                            .get("next_retry_at_ms")
                            .and_then(|v| v.as_i64())
                            .is_none_or(|t| t <= now_epoch_ms),
                        "claimed" => j
                            .get("lease_until_epoch_ms")
                            .and_then(|v| v.as_i64())
                            .is_some_and(|t| t <= now_epoch_ms),
                        _ => false,
                    }
                })
                .collect();
            Ok(Value::Array(claimable))
        }
        "build_memory_worker_plan" => {
            let input = args.get("input").unwrap_or(args);
            let conv_id = input
                .get("conversation_id")
                .or_else(|| args.get("conversation_id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .ok_or(("invalid_argument", "missing 'conversation_id'".into()))?;
            let job_id = input
                .get("job_id")
                .or_else(|| args.get("job_id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .ok_or(("invalid_argument", "missing 'job_id'".into()))?;
            let (messages, events) = crate::web_api::reconstruct_v2_messages(store, conv_id);
            let last_seq = events
                .last()
                .and_then(|e| e.get("seq").and_then(|v| v.as_i64()))
                .unwrap_or(0);
            let existing_memories: Vec<Value> = store
                .search_memory("", None, 50)
                .unwrap_or_default()
                .into_iter()
                .map(|m| crate::web_api::memory_item_to_record_with_store(store, &m))
                .collect();
            Ok(json!({
                "job_id": job_id,
                "conversation_id": conv_id,
                "input_watermark": format!("seq:{last_seq}"),
                "messages": messages,
                "existing_memories": existing_memories,
                "instructions": "Extract durable user preferences, facts, and project context."
            }))
        }
        "reduce_and_commit_idle_submission" => {
            let input = args.get("input").unwrap_or(args);
            let conv_id = input
                .get("conversation_id")
                .or_else(|| args.get("conversation_id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .ok_or(("invalid_argument", "missing 'conversation_id'".into()))?;
            let submission = input
                .get("submission")
                .or_else(|| args.get("submission"))
                .ok_or(("invalid_argument", "missing 'submission'".into()))?;
            let sub_type = submission
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let job_id = submission
                .get("job_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let committed_at_ms = input
                .get("committed_at_ms")
                .or_else(|| submission.get("now_epoch_ms"))
                .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)))
                .unwrap_or_else(now_ms);
            let jobs_key = format!("memory_jobs:{conv_id}");
            let mut jobs = store
                .get_kv(&jobs_key)
                .ok()
                .flatten()
                .and_then(|v| v.as_array().cloned())
                .unwrap_or_default();

            let mut updated_status = "committed".to_string();
            match sub_type {
                "claim_memory_worker" => {
                    let worker_id = submission
                        .get("worker_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let token = submission
                        .get("ownership_token")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let lease_until = submission
                        .get("lease_until_epoch_ms")
                        .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)))
                        .unwrap_or(committed_at_ms + 15 * 60 * 1_000);
                    let mut found = false;
                    for job in &mut jobs {
                        if job.get("id").and_then(|v| v.as_str()) == Some(job_id) {
                            job["status"] = json!("claimed");
                            job["worker_id"] = json!(worker_id);
                            job["ownership_token"] = json!(token);
                            job["claimed_at_ms"] = json!(committed_at_ms);
                            job["lease_until_epoch_ms"] = json!(lease_until);
                            found = true;
                            break;
                        }
                    }
                    if !found && !job_id.is_empty() {
                        jobs.push(json!({
                            "id": job_id,
                            "conversation_id": conv_id,
                            "status": "claimed",
                            "worker_id": worker_id,
                            "ownership_token": token,
                            "claimed_at_ms": committed_at_ms,
                            "lease_until_epoch_ms": lease_until
                        }));
                    }
                    updated_status = "claimed".to_string();
                }
                "heartbeat_memory_worker" => {
                    let token = submission
                        .get("ownership_token")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let lease_until = submission
                        .get("lease_until_epoch_ms")
                        .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)))
                        .unwrap_or(committed_at_ms + 15 * 60 * 1_000);
                    for job in &mut jobs {
                        if job.get("id").and_then(|v| v.as_str()) == Some(job_id) {
                            if token.is_empty()
                                || job.get("ownership_token").and_then(|v| v.as_str()) == Some(token)
                            {
                                job["lease_until_epoch_ms"] = json!(lease_until);
                                job["last_heartbeat_ms"] = json!(committed_at_ms);
                            }
                            break;
                        }
                    }
                    updated_status = "claimed".to_string();
                }
                "memory_worker_completed" => {
                    let watermark = submission
                        .get("completion_watermark")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    for job in &mut jobs {
                        if job.get("id").and_then(|v| v.as_str()) == Some(job_id) {
                            job["status"] = json!("completed");
                            job["completion_watermark"] = json!(watermark);
                            job["completed_at_ms"] = json!(committed_at_ms);
                            break;
                        }
                    }
                    if let Some(output) = submission.get("output") {
                        let items_arr = output
                            .get("memories")
                            .or_else(|| output.get("items"))
                            .or_else(|| output.get("records"))
                            .and_then(|v| v.as_array());
                        if let Some(arr) = items_arr {
                            for raw in arr {
                                let content = raw
                                    .get("content")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .trim();
                                if content.is_empty() {
                                    continue;
                                }
                                let mid = raw
                                    .get("id")
                                    .and_then(|v| v.as_str())
                                    .filter(|s| !s.trim().is_empty())
                                    .map(ToString::to_string)
                                    .unwrap_or_else(new_id);
                                let scope = raw
                                    .get("scope")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("global");
                                let kind = raw
                                    .get("kind")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("fact");
                                let title = raw
                                    .get("summary")
                                    .or_else(|| raw.get("title"))
                                    .and_then(|v| v.as_str())
                                    .unwrap_or(kind)
                                    .to_string();
                                let tags: Vec<String> = raw
                                    .get("tags")
                                    .and_then(|v| v.as_array())
                                    .map(|t| {
                                        t.iter()
                                            .filter_map(|x| x.as_str().map(ToString::to_string))
                                            .collect()
                                    })
                                    .unwrap_or_default();
                                let item = MemoryItem {
                                    id: mid,
                                    namespace: format!("{scope}:{kind}"),
                                    title,
                                    content: content.to_string(),
                                    tags,
                                    created_at_ms: committed_at_ms,
                                    updated_at_ms: committed_at_ms,
                                };
                                let _ = store.upsert_memory(&item);
                            }
                        }
                    }
                    updated_status = "completed".to_string();
                }
                "memory_worker_failed" => {
                    let retry_after = submission
                        .get("retry_after_ms")
                        .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)))
                        .unwrap_or(60_000);
                    let max_retries = submission
                        .get("max_retry_count")
                        .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)))
                        .unwrap_or(3);
                    let err_msg = submission
                        .get("error")
                        .and_then(|v| v.as_str())
                        .unwrap_or("memory worker failed");
                    for job in &mut jobs {
                        if job.get("id").and_then(|v| v.as_str()) == Some(job_id) {
                            let retries =
                                job.get("retry_count").and_then(|v| v.as_i64()).unwrap_or(0) + 1;
                            job["retry_count"] = json!(retries);
                            job["last_error"] = json!(err_msg);
                            if retries >= max_retries {
                                job["status"] = json!("failed");
                                updated_status = "failed".to_string();
                            } else {
                                job["status"] = json!("retry_scheduled");
                                job["next_retry_at_ms"] = json!(committed_at_ms + retry_after);
                                updated_status = "retry_scheduled".to_string();
                            }
                            break;
                        }
                    }
                }
                other => {
                    return Err((
                        "unsupported_command",
                        format!("unsupported idle submission type '{other}'"),
                    ));
                }
            }
            store
                .set_kv(&jobs_key, &Value::Array(jobs))
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({
                "ok": true,
                "conversation_id": conv_id,
                "submission_type": sub_type,
                "job_id": job_id,
                "status": updated_status,
                "committed_at_ms": committed_at_ms
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

type ProductSettingEntry = (Value, String, u64, i64);

fn load_product_setting_entry(
    store: &Store,
    key: &str,
) -> Result<Option<ProductSettingEntry>, (&'static str, String)> {
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

fn parse_schedule_timezone_offset(tz: Option<&str>) -> FixedOffset {
    let Some(raw) = tz.map(str::trim).filter(|s| !s.is_empty()) else {
        return *Local::now().offset();
    };
    if raw.eq_ignore_ascii_case("utc")
        || raw.eq_ignore_ascii_case("gmt")
        || raw.eq_ignore_ascii_case("etc/utc")
        || raw == "Z"
    {
        return FixedOffset::east_opt(0).unwrap();
    }
    if (raw.starts_with('+') || raw.starts_with('-')) && raw.len() >= 5 {
        let sign = if raw.starts_with('-') { -1 } else { 1 };
        let body = &raw[1..];
        let (hh, mm) = if let Some((h, m)) = body.split_once(':') {
            (h.parse::<i32>().ok(), m.parse::<i32>().ok())
        } else if body.len() == 4 {
            (body[0..2].parse::<i32>().ok(), body[2..4].parse::<i32>().ok())
        } else {
            (None, None)
        };
        if let (Some(h), Some(m)) = (hh, mm) {
            if let Some(off) = FixedOffset::east_opt(sign * (h * 3600 + m * 60)) {
                return off;
            }
        }
    }
    let secs = match raw {
        "Asia/Shanghai" | "Asia/Hong_Kong" | "Asia/Taipei" | "Asia/Singapore" | "Asia/Chongqing" => {
            Some(8 * 3600)
        }
        "Asia/Tokyo" | "Asia/Seoul" => Some(9 * 3600),
        "Asia/Kolkata" => Some(5 * 3600 + 1800),
        "Europe/London" => Some(0),
        "Europe/Berlin" | "Europe/Paris" | "Europe/Zurich" | "Europe/Amsterdam" => Some(3600),
        "America/New_York" => Some(-5 * 3600),
        "America/Chicago" => Some(-6 * 3600),
        "America/Denver" => Some(-7 * 3600),
        "America/Los_Angeles" => Some(-8 * 3600),
        "Australia/Sydney" => Some(10 * 3600),
        _ => None,
    };
    secs.and_then(FixedOffset::east_opt)
        .unwrap_or_else(|| *Local::now().offset())
}

fn parse_schedule_hour_minute(input: &Value) -> (u32, u32) {
    if let Some(time_str) = input.get("time").and_then(|v| v.as_str()) {
        if let Some((h_s, m_s)) = time_str.trim().split_once(':') {
            if let (Ok(h), Ok(m)) = (h_s.parse::<u32>(), m_s.parse::<u32>()) {
                return (h.min(23), m.min(59));
            }
        }
    }
    let h = input
        .get("hour")
        .and_then(|v| v.as_u64())
        .map(|n| (n as u32).min(23))
        .unwrap_or(9);
    let m = input
        .get("minute")
        .and_then(|v| v.as_u64())
        .map(|n| (n as u32).min(59))
        .unwrap_or(0);
    (h, m)
}

fn parse_schedule_iso_weekday(input: &Value) -> u32 {
    if let Some(n) = input.get("weekday").and_then(|v| v.as_i64()) {
        if (1..=7).contains(&n) {
            return n as u32;
        }
        if n == 0 {
            return 7; // Sunday
        }
    }
    if let Some(s) = input.get("weekday").and_then(|v| v.as_str()) {
        match s.trim().to_ascii_lowercase().as_str() {
            "mon" | "monday" => return 1,
            "tue" | "tuesday" => return 2,
            "wed" | "wednesday" => return 3,
            "thu" | "thursday" => return 4,
            "fri" | "friday" => return 5,
            "sat" | "saturday" => return 6,
            "sun" | "sunday" => return 7,
            _ => {}
        }
    }
    1
}

fn days_in_month(year: i32, month: u32) -> u32 {
    let (ny, nm) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    let next_first = NaiveDate::from_ymd_opt(ny, nm, 1).unwrap();
    let cur_first = NaiveDate::from_ymd_opt(year, month, 1).unwrap();
    (next_first - cur_first).num_days() as u32
}

fn compute_scheduled_automation_occurrences(
    def_id: Option<&str>,
    input: &Value,
    after_ms: Option<i64>,
    through_ms: Option<i64>,
    limit: usize,
) -> Vec<Value> {
    let after = after_ms.unwrap_or_else(now_ms);
    let through = through_ms.unwrap_or_else(|| after.saturating_add(31 * 86_400_000));
    if through <= after || limit == 0 {
        return Vec::new();
    }
    let id_str = def_id.unwrap_or("schedule");
    let make_occ = |ts: i64| -> Value {
        json!({
            "occurrence_id": format!("{id_str}:{ts}"),
            "automation_id": id_str,
            "scheduled_for_ms": ts,
            "status": "scheduled",
            "source": "schedule"
        })
    };

    if let Some(once_ts) = input.get("once_at_ms").and_then(|v| v.as_i64()) {
        if once_ts > after && once_ts <= through {
            return vec![make_occ(once_ts)];
        }
        return Vec::new();
    }

    let freq = input
        .get("frequency")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let offset = parse_schedule_timezone_offset(input.get("timezone").and_then(|v| v.as_str()));
    let (hour, minute) = parse_schedule_hour_minute(input);
    let Some(after_utc) = Utc.timestamp_millis_opt(after).single() else {
        return Vec::new();
    };
    let after_local = after_utc.with_timezone(&offset);
    let mut out = Vec::new();

    match freq.as_str() {
        "hourly" => {
            let base_hour = after_local.date_naive().and_hms_opt(
                after_local.time().format("%H").to_string().parse::<u32>().unwrap_or(0),
                minute,
                0,
            );
            let Some(mut cur_naive) = base_hour else {
                return Vec::new();
            };
            while out.len() < limit {
                if let Some(dt) = offset.from_local_datetime(&cur_naive).single() {
                    let ts = dt.timestamp_millis();
                    if ts > through {
                        break;
                    }
                    if ts > after {
                        out.push(make_occ(ts));
                    }
                }
                cur_naive += chrono::Duration::hours(1);
            }
        }
        "daily" => {
            let Some(t) = NaiveTime::from_hms_opt(hour, minute, 0) else {
                return Vec::new();
            };
            let mut date = after_local.date_naive();
            for _ in 0..400 {
                if out.len() >= limit {
                    break;
                }
                if let Some(dt) = offset.from_local_datetime(&date.and_time(t)).single() {
                    let ts = dt.timestamp_millis();
                    if ts > through {
                        break;
                    }
                    if ts > after {
                        out.push(make_occ(ts));
                    }
                }
                let Some(next_date) = date.succ_opt() else {
                    break;
                };
                date = next_date;
            }
        }
        "weekly" => {
            let Some(t) = NaiveTime::from_hms_opt(hour, minute, 0) else {
                return Vec::new();
            };
            let target_wd = parse_schedule_iso_weekday(input);
            let cur_wd = after_local.weekday().number_from_monday();
            let days_ahead = (target_wd + 7 - cur_wd) % 7;
            let mut date = after_local.date_naive() + chrono::Duration::days(days_ahead as i64);
            for _ in 0..104 {
                if out.len() >= limit {
                    break;
                }
                if let Some(dt) = offset.from_local_datetime(&date.and_time(t)).single() {
                    let ts = dt.timestamp_millis();
                    if ts > through {
                        break;
                    }
                    if ts > after {
                        out.push(make_occ(ts));
                    }
                }
                date += chrono::Duration::days(7);
            }
        }
        "monthly" => {
            let Some(t) = NaiveTime::from_hms_opt(hour, minute, 0) else {
                return Vec::new();
            };
            let target_day = input
                .get("day_of_month")
                .or_else(|| input.get("day"))
                .and_then(|v| v.as_u64())
                .map(|n| (n as u32).clamp(1, 31))
                .unwrap_or(1);
            let mut year = after_local.year();
            let mut month = after_local.month();
            for _ in 0..60 {
                if out.len() >= limit {
                    break;
                }
                let day = target_day.min(days_in_month(year, month));
                if let Some(date) = NaiveDate::from_ymd_opt(year, month, day) {
                    if let Some(dt) = offset.from_local_datetime(&date.and_time(t)).single() {
                        let ts = dt.timestamp_millis();
                        if ts > through {
                            break;
                        }
                        if ts > after {
                            out.push(make_occ(ts));
                        }
                    }
                }
                if month == 12 {
                    year += 1;
                    month = 1;
                } else {
                    month += 1;
                }
            }
        }
        "interval" => {
            let interval_ms = input
                .get("interval_ms")
                .and_then(|v| v.as_i64())
                .or_else(|| {
                    input
                        .get("interval_seconds")
                        .and_then(|v| v.as_i64())
                        .map(|s| s.saturating_mul(1_000))
                })
                .or_else(|| {
                    input
                        .get("interval_minutes")
                        .and_then(|v| v.as_i64())
                        .map(|m| m.saturating_mul(60_000))
                })
                .unwrap_or(3_600_000)
                .max(1_000);
            let anchor = input
                .get("anchor_ms")
                .and_then(|v| v.as_i64())
                .unwrap_or(after);
            let mut ts = if anchor > after {
                anchor
            } else {
                let steps = (after - anchor) / interval_ms + 1;
                anchor.saturating_add(steps.saturating_mul(interval_ms))
            };
            while out.len() < limit && ts <= through {
                if ts > after {
                    out.push(make_occ(ts));
                }
                let Some(next) = ts.checked_add(interval_ms) else {
                    break;
                };
                ts = next;
            }
        }
        _ => {}
    }

    out
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

