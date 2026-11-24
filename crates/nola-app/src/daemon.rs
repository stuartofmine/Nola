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
        let mut i = 0;
        while i < args.len() {
            if args[i] == "--socket" {
                if let Some(p) = args.get(i + 1) {
                    socket_path = PathBuf::from(p);
                }
                i += 1;
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

        println!(
            "Nola v2 Application Service listening on {} (protocol v{})",
            socket_path.display(),
            PROTOCOL_VERSION
        );

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
pub fn handle_envelope(store: &Store, suite: &ModulesSuite, req: &Value) -> Value {
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
        .and_then(|c| c.get("arguments"))
        .cloned()
        .unwrap_or_else(|| json!({}));

    let is_mutation = matches!(
        cmd_type,
        "session_create" | "session_delete" | "module_upsert" | "module_delete" | "memory_save" | "backup"
    );

    if is_mutation && !request_id.is_empty() {
        let receipt_key = format!("cmd_receipt:{request_id}");
        if let Ok(Some(existing)) = store.get_kv(&receipt_key) {
            return existing;
        }
    }

    let resp = match dispatch_command(store, suite, cmd_type, &cmd_args) {
        Ok(result) => json!({
            "protocol_version": PROTOCOL_VERSION,
            "request_id": request_id,
            "client_id": client_id,
            "status": "committed",
            "result": result
        }),
        Err(msg) => json!({
            "protocol_version": PROTOCOL_VERSION,
            "request_id": request_id,
            "client_id": client_id,
            "status": "rejected",
            "error": {"message": msg}
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
) -> Result<Value, String> {
    match cmd_type {
        "health" => Ok(json!({"ok": true, "version": "2.0.0", "protocol_version": PROTOCOL_VERSION})),
        "command_receipt" => {
            let rid = args.get("request_id").and_then(|v| v.as_str()).unwrap_or("");
            let rec = store
                .get_kv(&format!("cmd_receipt:{rid}"))
                .map_err(|e| e.to_string())?;
            Ok(rec.unwrap_or(Value::Null))
        }
        "projection_snapshot" | "sessions_list" => {
            let sessions = store.list_sessions(200).map_err(|e| e.to_string())?;
            let goals: Vec<Value> = suite.list_records("goal").unwrap_or_default();
            let rooms: Vec<Value> = suite.list_records("agent_room").unwrap_or_default();
            Ok(json!({"sessions": sessions, "goals": goals, "rooms": rooms}))
        }
        "session_create" => {
            let ts = now_ms();
            let s = Session {
                id: args
                    .get("id")
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string)
                    .unwrap_or_else(new_id),
                title: args
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("New Session")
                    .to_string(),
                workspace_root: args
                    .get("workspace_root")
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string)
                    .unwrap_or_else(|| suite.workspace_root.display().to_string()),
                model_connection: None,
                model_id: None,
                created_at_ms: ts,
                updated_at_ms: ts,
            };
            store.upsert_session(&s).map_err(|e| e.to_string())?;
            Ok(serde_json::to_value(&s).unwrap_or(json!({})))
        }
        "session_delete" => {
            let id = args.get("id").and_then(|v| v.as_str()).ok_or("missing 'id'")?;
            store.delete_session(id).map_err(|e| e.to_string())?;
            Ok(json!({"deleted": id}))
        }
        "events_after" | "projection_event_suffix" | "projection_cursor_wait" => {
            let scope = args.get("scope").and_then(|v| v.as_str()).unwrap_or("");
            let after_seq = args.get("after_seq").and_then(|v| v.as_i64()).unwrap_or(0);
            match store.events_after(scope, after_seq, 200).map_err(|e| e.to_string())? {
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
        "module_list" => {
            let kind = args.get("kind").and_then(|v| v.as_str()).unwrap_or("goal");
            let items: Vec<Value> = suite.list_records(kind).map_err(|e| e.to_string())?;
            Ok(json!({"kind": kind, "items": items}))
        }
        "module_upsert" => {
            let kind = args.get("kind").and_then(|v| v.as_str()).ok_or("missing 'kind'")?;
            let record = args.get("record").ok_or("missing 'record'")?;
            let id = record
                .get("id")
                .and_then(|v| v.as_str())
                .map(ToString::to_string)
                .unwrap_or_else(new_id);
            suite
                .upsert_record(kind, &id, record)
                .map_err(|e| e.to_string())?;
            Ok(json!({"ok": true, "id": id}))
        }
        "module_delete" => {
            let kind = args.get("kind").and_then(|v| v.as_str()).ok_or("missing 'kind'")?;
            let id = args.get("id").and_then(|v| v.as_str()).ok_or("missing 'id'")?;
            let deleted = suite.delete_record(kind, id).map_err(|e| e.to_string())?;
            Ok(json!({"deleted": deleted}))
        }
        "memory_search" => {
            let q = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
            let items = store.search_memory(q, None, 25).map_err(|e| e.to_string())?;
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
                tags: Vec::new(),
                created_at_ms: ts,
                updated_at_ms: ts,
            };
            store.upsert_memory(&item).map_err(|e| e.to_string())?;
            Ok(serde_json::to_value(&item).unwrap_or(json!({})))
        }
        "backup" => {
            let dest = args
                .get("destination")
                .and_then(|v| v.as_str())
                .ok_or("missing 'destination'")?;
            store
                .backup_to(Path::new(dest))
                .map_err(|e| e.to_string())?;
            Ok(json!({"backup": dest}))
        }
        other => Err(format!("unsupported daemon command '{other}'")),
    }
}
