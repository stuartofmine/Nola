// SPDX-License-Identifier: Apache-2.0
//! Single-writer Unix Domain Socket Application Service daemon (`nola server`).
//!
//! Synced with `apps/macos/Services/MacApplicationServiceClient.swift`:
//! - Acquires exclusive `ServiceLock` and publishes `<db>.service.json` atomically.
//! - Authenticates every connecting peer via `SO_PEERCRED` (same EUID only).
//! - Persists idempotent `request_id` receipts (`command_receipt`) for crash recovery.
//! - Supports `health`, `projection_snapshot`, `projection_event_suffix`,
//!   `projection_cursor_wait`, `command_receipt`, `sessions_list`, `session_create`,
//!   `session_delete`, `events_after`, `module_list`, `module_upsert`, `module_delete`,
//!   `memory_search`, `memory_save`, `backup`.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use nola_modules::a2ui::{
    build_nodes_from_spec, commit_ui_interaction, delete_ui_surface, list_conversation_surfaces,
    preview_ui_surface, read_ui_surface_changes, read_ui_surface_revision, upsert_ui_surface,
    UiDraftPreviewInput, UiInteraction, UiSurfaceIdentity, UiSurfaceOwner, UiSurfaceRevisionRef,
};
use nola_modules::ModulesSuite;
use nola_store::{default_socket_path, EventPage, ServiceLock, Store, PROTOCOL_VERSION};
use nola_types::{new_id, now_ms, MemoryItem, Session};
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
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&socket_path, std::fs::Permissions::from_mode(0o600));
        }

        if let Some(ppid) = parent_pid {
            let sock_cleanup = socket_path.clone();
            std::thread::spawn(move || loop {
                std::thread::sleep(Duration::from_secs(1));
                let alive = unsafe { libc::kill(ppid, 0) == 0 }
                    || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM);
                if !alive {
                    let _ = std::fs::remove_file(&sock_cleanup);
                    std::process::exit(0);
                }
            });
        }

        println!(
            "Nola v2 Application Service listening on {} (protocol v{})",
            socket_path.display(),
            PROTOCOL_VERSION
        );
        let _ = std::io::stdout().flush();

        let active_clients = Arc::new(AtomicUsize::new(0));
        for stream in listener.incoming().flatten() {
            if !peer_matches_euid(&stream) {
                continue;
            }
            if active_clients.fetch_add(1, Ordering::SeqCst) >= MAX_CONCURRENT_DAEMON_CLIENTS {
                active_clients.fetch_sub(1, Ordering::SeqCst);
                continue;
            }
            let store_c = store.clone();
            let suite_c = Arc::clone(&suite);
            let active_c = Arc::clone(&active_clients);
            std::thread::spawn(move || {
                handle_client(stream, &store_c, &suite_c);
                active_c.fetch_sub(1, Ordering::SeqCst);
            });
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

/// Process a complete UDS / FFI JSON envelope with idempotent `request_id` receipts.
/// Conforms strictly to Android `ApplicationReceiptValidation` & iOS/macOS receipt contracts.
pub fn handle_envelope(store: &Store, suite: &ModulesSuite, req: &Value) -> Value {
    let received_at = chrono::Utc::now().to_rfc3339();
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

    let is_mutation = matches!(
        cmd_type,
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
            | "backup"
    );

    if is_mutation && !request_id.is_empty() {
        let receipt_key = format!("cmd_receipt:{request_id}");
        if let Ok(Some(existing)) = store.get_kv(&receipt_key) {
            return existing;
        }
    }

    let completed_at = chrono::Utc::now().to_rfc3339();
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

    if is_mutation && !request_id.is_empty() {
        let _ = store.set_kv(&format!("cmd_receipt:{request_id}"), &resp);
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
        "health" => Ok(json!({"ok": true, "version": "2.0.0", "protocol_version": PROTOCOL_VERSION})),
        "command_receipt" => {
            let rid = args.get("request_id").and_then(|v| v.as_str()).unwrap_or("");
            let rec = store
                .get_kv(&format!("cmd_receipt:{rid}"))
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(rec.unwrap_or(Value::Null))
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
        "load_conversation" | "load_conversation_projection_v1" | "load_conversation_projection_window_v1" => {
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
                    .unwrap_or_else(|| json!({"isPinned": false, "revision": 1, "configurationRevision": 1}));
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
            let mode = args
                .get("permission_mode")
                .or_else(|| args.get("mode"))
                .or_else(|| args.get("permission_profile"))
                .and_then(|v| v.as_str())
                .unwrap_or("cautious");
            store
                .set_kv(&format!("conv_perm:{id}"), &json!(mode))
                .map_err(|e| ("storage_error", e.to_string()))?;
            let meta_key = crate::web_api::session_meta_key(id);
            let mut meta = store
                .get_kv(&meta_key)
                .ok()
                .flatten()
                .unwrap_or_else(|| json!({"isPinned": false, "revision": 1, "configurationRevision": 1}));
            meta["permissionProfile"] = json!(mode);
            let next_cfg = meta
                .get("configurationRevision")
                .and_then(|v| v.as_i64())
                .unwrap_or(1)
                + 1;
            meta["configurationRevision"] = json!(next_cfg);
            let _ = store.set_kv(&meta_key, &meta);
            Ok(json!({
                "conversation_id": id,
                "permission_mode": mode,
                "permission_profile": mode,
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
            Ok(json!({"ok": true, "conversation_id": id, "from_seq": from_seq, "deleted_events": deleted}))
        }
        "compact_conversation" => {
            let id = args
                .get("conversation_id")
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'conversation_id'".into()))?;
            let (messages, _) = crate::web_api::reconstruct_v2_messages(store, id);
            let summary = format!("Compacted {} message(s)", messages.len());
            let _ = store.append_event(
                id,
                "compaction_marker",
                json!({"summary": &summary, "messageCount": messages.len()}),
                50_000,
            );
            Ok(json!({"ok": true, "conversation_id": id, "summary": summary}))
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
            Ok(json!({"conversation": crate::web_api::session_to_conversation_json(store, &sess)}))
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
                .map(|s| std::path::PathBuf::from(s.workspace_root))
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
            let (root_id, nodes) = build_nodes_from_spec(args)
                .map_err(|e| ("invalid_ui_spec", e))?;
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
            let snap = upsert_ui_surface(store, owner, surface_id, summary, root_id, nodes, data_model)
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
            let snap = read_ui_surface_revision(store, &UiSurfaceRevisionRef { identity, revision })
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
            let preview = preview_ui_surface(store, &input)
                .map_err(|e| ("ui_preview_error", e))?;
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
            let ts = now_ms();
            let item = MemoryItem {
                id: args
                    .get("id")
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string)
                    .unwrap_or_else(new_id),
                namespace: args
                    .get("namespace")
                    .and_then(|v| v.as_str())
                    .unwrap_or("default")
                    .to_string(),
                title: args
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Memory")
                    .to_string(),
                content: args
                    .get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                tags: args
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
            };
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

fn dispatch_domain_command(
    store: &Store,
    suite: &ModulesSuite,
    domain: &str,
    args: &Value,
) -> Result<Value, (&'static str, String)> {
    let op = args
        .get("operation")
        .and_then(|v| v.get("type").or(Some(v)))
        .and_then(|v| v.as_str())
        .unwrap_or("list");
    match (domain, op) {
        ("memory", "list" | "recall" | "search") => {
            let q = args
                .pointer("/operation/query")
                .or_else(|| args.get("query"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let items = store
                .search_memory(q, None, 100)
                .map_err(|e| ("storage_error", e.to_string()))?;
            let records: Vec<Value> = items
                .iter()
                .map(crate::web_api::memory_item_to_record)
                .collect();
            Ok(json!({"items": items, "records": records}))
        }
        ("memory", "upsert" | "save" | "update_if_revision") => {
            let payload = args.get("operation").unwrap_or(args);
            let ts = now_ms();
            let item = MemoryItem {
                id: payload
                    .get("id")
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string)
                    .unwrap_or_else(new_id),
                namespace: payload
                    .get("namespace")
                    .and_then(|v| v.as_str())
                    .unwrap_or("default")
                    .to_string(),
                title: payload
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Memory")
                    .to_string(),
                content: payload
                    .get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                tags: Vec::new(),
                created_at_ms: ts,
                updated_at_ms: ts,
            };
            store
                .upsert_memory(&item)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(crate::web_api::memory_item_to_record(&item))
        }
        ("memory", "delete") => {
            let id = args
                .pointer("/operation/id")
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            store
                .delete_memory(id)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"deleted": id, "ok": true}))
        }
        (_, "list" | "list_profiles" | "list_editors" | "list_usage") => {
            let items: Vec<Value> = suite
                .list_records(domain)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"kind": domain, "items": items, "profiles": items}))
        }
        (_, "get") => {
            let id = args
                .pointer("/operation/id")
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            let items: Vec<Value> = suite
                .list_records(domain)
                .map_err(|e| ("storage_error", e.to_string()))?;
            let found = items
                .into_iter()
                .find(|v| v.get("id").and_then(|x| x.as_str()) == Some(id));
            Ok(found.unwrap_or(Value::Null))
        }
        (_, "delete") => {
            let id = args
                .pointer("/operation/id")
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            let deleted = suite
                .delete_record(domain, id)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"deleted": deleted, "id": id}))
        }
        _ => {
            let payload = args.get("operation").unwrap_or(args);
            let id = payload
                .get("id")
                .or_else(|| payload.pointer("/record/id"))
                .or_else(|| payload.pointer("/profile/id"))
                .and_then(|v| v.as_str())
                .map(ToString::to_string)
                .unwrap_or_else(new_id);
            let record = payload
                .get("record")
                .or_else(|| payload.get("profile"))
                .unwrap_or(payload);
            suite
                .upsert_record(domain, &id, record)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"ok": true, "id": id, "record": record}))
        }
    }
}
