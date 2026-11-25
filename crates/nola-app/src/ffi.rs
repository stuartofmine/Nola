// SPDX-License-Identifier: Apache-2.0
//! Clean C-ABI bridge for Apple (`NolaCore.framework` / Swift) and Android (JNI) consumers.
//! Canonical header: `crates/nola-app/include/nola_core.h`.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{c_char, CStr, CString};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::time::Duration;

use chrono::{DateTime, Utc};
use nola_engine::{
    AgentEngine, AgentEvent, AgentRunConfig, EffectivePolicy, ForceToolSet, HookRunner,
    HttpModelCaller,
};
use nola_llm::PrivacyRedactor;
use nola_mcp::{McpError, McpManager};
use nola_modules::updater::{
    download_update_package_with_size, evaluate_signed_feed, CURRENT_STORE_EPOCH,
    CURRENT_STORE_SCHEMA,
};
use nola_modules::{
    check_for_update, download_update_package, search_memory_v2, MemorySearchFilter, ModulesSuite,
};
use nola_store::{Store, Vault, PROTOCOL_VERSION};
use nola_tools::{ToolContext, ToolRegistry};
use nola_types::{
    estimate_text_tokens, new_id, now_ms, FrozenSystemBlock, HostCapabilities, MemoryItem,
    PermissionProfile, ToolSpec,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::daemon::{clear_active_run, handle_envelope, register_active_run};

const ABI_V1: u32 = 1;
static VERSION_CSTR: &[u8] = b"2.0.0\0";

thread_local! {
    static LAST_OPEN_ERROR: RefCell<Option<CString>> = const { RefCell::new(None) };
}

static ACTIVE_APP_STORE: OnceLock<Mutex<Option<Store>>> = OnceLock::new();
static ACTIVE_MCP_CANCELS: OnceLock<Mutex<BTreeMap<String, Arc<AtomicBool>>>> = OnceLock::new();
static PRE_CANCELLED_MCP_IDS: OnceLock<Mutex<BTreeSet<String>>> = OnceLock::new();

fn active_app_store_slot() -> &'static Mutex<Option<Store>> {
    ACTIVE_APP_STORE.get_or_init(|| Mutex::new(None))
}

fn active_mcp_cancels() -> &'static Mutex<BTreeMap<String, Arc<AtomicBool>>> {
    ACTIVE_MCP_CANCELS.get_or_init(|| Mutex::new(BTreeMap::new()))
}

fn pre_cancelled_mcp_ids() -> &'static Mutex<BTreeSet<String>> {
    PRE_CANCELLED_MCP_IDS.get_or_init(|| Mutex::new(BTreeSet::new()))
}

fn set_last_open_error(err: Option<String>) {
    LAST_OPEN_ERROR.with(|slot| {
        *slot.borrow_mut() = err.and_then(|msg| CString::new(msg.replace('\0', " ")).ok());
    });
}

struct AppInner {
    store: Store,
    suite: ModulesSuite,
    mcp: Arc<McpManager>,
}

/// Reference-counted opaque handle representing an opened Nola v2 Store + ModulesSuite across C-ABI.
/// `nola_core_mobile_mcp_retain` increments the inner `Arc` count; each `nola_core_application_free`
/// releases one handle safely.
pub struct NolaCoreApplication {
    inner: Arc<AppInner>,
}

struct ChildPumpEvent {
    kind: String,
    request: Option<Value>,
    event: Option<Value>,
    transport_finished: bool,
    error: Option<String>,
}

/// Opaque handle for a real background embedded mobile child execution pump.
pub struct NolaCoreChildPump {
    pump_id: String,
    cancel: Arc<AtomicBool>,
    closed: AtomicBool,
    event_rx: Mutex<mpsc::Receiver<ChildPumpEvent>>,
    event_tx: Mutex<Option<mpsc::Sender<ChildPumpEvent>>>,
    response_tx: Mutex<Option<mpsc::Sender<Value>>>,
    pending_tickets: Arc<Mutex<BTreeMap<u64, mpsc::Sender<Value>>>>,
    finished: Arc<AtomicBool>,
    terminal_error: Arc<Mutex<Option<String>>>,
    worker: Mutex<Option<std::thread::JoinHandle<()>>>,
}

fn to_owned_c_string(s: String) -> *mut c_char {
    CString::new(s)
        .unwrap_or_else(|_| CString::new("{}").unwrap())
        .into_raw()
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

unsafe fn parse_json_slice(ptr: *const u8, len: usize) -> Option<Value> {
    if ptr.is_null() || len == 0 {
        return None;
    }
    let slice = unsafe { std::slice::from_raw_parts(ptr, len) };
    serde_json::from_slice(slice).ok()
}

/// Free a C string previously returned by any `nola_core_*` function.
///
/// # Safety
/// `ptr` must be null or a pointer previously returned by `nola_core_*`.
#[no_mangle]
pub unsafe extern "C" fn nola_core_free_c_string(ptr: *mut c_char) {
    if !ptr.is_null() {
        drop(unsafe { CString::from_raw(ptr) });
    }
}

/// Alias for `nola_core_free_c_string` used by Apple/Android model privacy bridges.
///
/// # Safety
/// `ptr` must be null or a pointer previously returned by `nola_core_redact_model_request_json`.
#[no_mangle]
pub unsafe extern "C" fn nola_model_privacy_free(ptr: *mut c_char) {
    unsafe { nola_core_free_c_string(ptr) };
}

#[no_mangle]
pub extern "C" fn nola_core_application_abi_version() -> u32 {
    ABI_V1
}

#[no_mangle]
pub extern "C" fn nola_core_application_service_protocol_version() -> u32 {
    PROTOCOL_VERSION
}

#[no_mangle]
pub extern "C" fn nola_core_intelligence_abi_version() -> u32 {
    ABI_V1
}

#[no_mangle]
pub extern "C" fn nola_core_context_engine_abi_version() -> u32 {
    ABI_V1
}

#[no_mangle]
pub extern "C" fn nola_core_memory_selection_abi_version() -> u32 {
    ABI_V1
}

#[no_mangle]
pub extern "C" fn nola_core_data_source_abi_version() -> u32 {
    ABI_V1
}

#[no_mangle]
pub extern "C" fn nola_core_resource_access_abi_version() -> u32 {
    ABI_V1
}

#[no_mangle]
pub extern "C" fn nola_core_tool_policy_abi_version() -> u32 {
    ABI_V1
}

#[no_mangle]
pub extern "C" fn nola_core_prompt_composer_abi_version() -> u32 {
    ABI_V1
}

#[no_mangle]
pub extern "C" fn nola_v2_version() -> *const c_char {
    VERSION_CSTR.as_ptr().cast()
}

/// Open a Nola v2 application store handle.
///
/// # Safety
/// `path_utf8` must point to `path_len` readable UTF-8 bytes.
#[no_mangle]
pub unsafe extern "C" fn nola_core_application_open(
    path_utf8: *const u8,
    path_len: usize,
) -> *mut NolaCoreApplication {
    set_last_open_error(None);
    if path_utf8.is_null() || path_len == 0 {
        set_last_open_error(Some("empty database path".into()));
        return std::ptr::null_mut();
    }
    let slice = unsafe { std::slice::from_raw_parts(path_utf8, path_len) };
    let path_str = match std::str::from_utf8(slice) {
        Ok(s) => s,
        Err(e) => {
            set_last_open_error(Some(format!("invalid UTF-8 database path: {e}")));
            return std::ptr::null_mut();
        }
    };
    let db_path = PathBuf::from(path_str);
    let store = match Store::open(&db_path) {
        Ok(s) => s,
        Err(e) => {
            set_last_open_error(Some(format!("failed to open store: {e}")));
            return std::ptr::null_mut();
        }
    };
    if let Err(e) =
        crate::config::bind_store_vault_path(&store, &crate::config::default_vault_path(&db_path))
    {
        set_last_open_error(Some(format!("failed to bind store vault path: {e}")));
        return std::ptr::null_mut();
    }
    let ws = db_path
        .parent()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let suite = ModulesSuite::new(store.clone(), ws, None);
    let vault = Vault::open(crate::config::default_vault_path(&db_path))
        .ok()
        .map(Arc::new);
    let mcp = Arc::new(McpManager::new(vault));
    if let Ok(mut slot) = active_app_store_slot().lock() {
        *slot = Some(store.clone());
    }
    Box::into_raw(Box::new(NolaCoreApplication {
        inner: Arc::new(AppInner { store, suite, mcp }),
    }))
}

/// Return a borrowed pointer to the last error message from `nola_core_application_open`
/// on the current thread, or NULL. Valid until the next `nola_core_application_open` call
/// on the same thread; callers MUST NOT free this pointer.
#[no_mangle]
pub extern "C" fn nola_core_application_last_open_error() -> *const c_char {
    LAST_OPEN_ERROR.with(|slot| match slot.borrow().as_ref() {
        Some(cstr) => cstr.as_ptr(),
        None => std::ptr::null(),
    })
}

/// Free a handle returned by `nola_core_application_open` or `nola_core_mobile_mcp_retain`.
///
/// # Safety
/// `app` must be null or a valid pointer returned by `nola_core_application_open` / `retain`.
#[no_mangle]
pub unsafe extern "C" fn nola_core_application_free(app: *mut NolaCoreApplication) {
    if !app.is_null() {
        drop(unsafe { Box::from_raw(app) });
    }
}

/// Retain an additional handle pointing to the same `NolaCoreApplication` store.
///
/// # Safety
/// `app` must be null or a valid pointer returned by `nola_core_application_open`.
#[no_mangle]
pub unsafe extern "C" fn nola_core_mobile_mcp_retain(
    app: *mut NolaCoreApplication,
) -> *mut NolaCoreApplication {
    if app.is_null() {
        return std::ptr::null_mut();
    }
    let app_ref = unsafe { &*app };
    Box::into_raw(Box::new(NolaCoreApplication {
        inner: Arc::clone(&app_ref.inner),
    }))
}

/// Execute a JSON envelope against an opened `NolaCoreApplication` and return an owned JSON C string.
///
/// # Safety
/// `app` must be valid; `json_ptr` must point to `json_len` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn nola_core_application_execute(
    app: *mut NolaCoreApplication,
    json_ptr: *const u8,
    json_len: usize,
) -> *mut c_char {
    if app.is_null() || json_ptr.is_null() {
        return to_owned_c_string(
            r#"{"status":"rejected","error":{"message":"null pointer"}}"#.into(),
        );
    }
    let app_ref = unsafe { &*app };
    let Some(req) = (unsafe { parse_json_slice(json_ptr, json_len) }) else {
        return to_owned_c_string(
            r#"{"status":"rejected","error":{"message":"invalid json"}}"#.into(),
        );
    };
    let resp = handle_envelope(&app_ref.inner.store, &app_ref.inner.suite, &req);
    to_owned_c_string(resp.to_string())
}

/// Submit an idempotent command envelope wrapped in the mobile outer envelope `{"ok": true, "result": <Receipt>}`.
///
/// # Safety
/// `app` must be valid; `json_ptr` must point to `json_len` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn nola_core_application_submit(
    app: *mut NolaCoreApplication,
    json_ptr: *const u8,
    json_len: usize,
) -> *mut c_char {
    if app.is_null() {
        return to_owned_c_string(
            json!({"ok": false, "error": {"code": "null_application", "message": "null application handle"}})
                .to_string(),
        );
    }
    let app_ref = unsafe { &*app };
    let Some(req) = (unsafe { parse_json_slice(json_ptr, json_len) }) else {
        return to_owned_c_string(
            json!({"ok": false, "error": {"code": "invalid_json", "message": "invalid json request"}})
                .to_string(),
        );
    };
    let receipt = handle_envelope(&app_ref.inner.store, &app_ref.inner.suite, &req);
    to_owned_c_string(json!({"ok": true, "result": receipt}).to_string())
}

/// Dispatch trusted expert-host operations against the application store.
/// Handles both `{request_id, operation}` (Android/iOS/macOS) and `{request_id, command}` envelopes,
/// returning `{"ok": true, "request_id": ..., "result": ...}`.
///
/// # Safety
/// `app` must be valid; `json_ptr` must point to `json_len` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn nola_core_application_expert_host_operation(
    app: *mut NolaCoreApplication,
    json_ptr: *const u8,
    json_len: usize,
) -> *mut c_char {
    if app.is_null() || json_ptr.is_null() {
        return to_owned_c_string(
            json!({
                "ok": false,
                "request_id": "",
                "error": {"code": "null_pointer", "message": "null application or request pointer"}
            })
            .to_string(),
        );
    }
    let app_ref = unsafe { &*app };
    let Some(req) = (unsafe { parse_json_slice(json_ptr, json_len) }) else {
        return to_owned_c_string(
            json!({
                "ok": false,
                "request_id": "",
                "error": {"code": "invalid_json", "message": "invalid json request"}
            })
            .to_string(),
        );
    };
    let req_id = req
        .get("request_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let client_id = req
        .get("client_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let envelope_resp = if req.get("command").is_some() {
        handle_envelope(&app_ref.inner.store, &app_ref.inner.suite, &req)
    } else if let Some(op_val) = req.get("operation") {
        let wrapped_native = json!({
            "request_id": req_id,
            "client_id": client_id,
            "command": {
                "type": "native_expert_host_operation",
                "operation": op_val
            }
        });
        if let Some(op_type) = op_val.get("type").and_then(|v| v.as_str()) {
            let is_template_subop = matches!(
                op_type,
                "list" | "list_templates" | "get" | "upsert" | "create" | "update" | "delete"
            );
            if is_template_subop {
                handle_envelope(&app_ref.inner.store, &app_ref.inner.suite, &wrapped_native)
            } else {
                let direct_env = json!({
                    "request_id": req_id,
                    "client_id": client_id,
                    "command": op_val
                });
                let direct_resp =
                    handle_envelope(&app_ref.inner.store, &app_ref.inner.suite, &direct_env);
                let is_unsupported = direct_resp
                    .get("error")
                    .and_then(|e| e.get("code"))
                    .and_then(|c| c.as_str())
                    == Some("unsupported_command");
                if is_unsupported {
                    handle_envelope(&app_ref.inner.store, &app_ref.inner.suite, &wrapped_native)
                } else {
                    direct_resp
                }
            }
        } else {
            let fallback_env = json!({
                "request_id": req_id,
                "client_id": client_id,
                "command": {
                    "type": "native_expert_host_operation",
                    "arguments": req
                }
            });
            handle_envelope(&app_ref.inner.store, &app_ref.inner.suite, &fallback_env)
        }
    } else {
        handle_envelope(&app_ref.inner.store, &app_ref.inner.suite, &req)
    };

    if envelope_resp.get("status").and_then(|v| v.as_str()) == Some("committed") {
        let res = envelope_resp.get("result").cloned().unwrap_or(Value::Null);
        to_owned_c_string(
            json!({
                "ok": true,
                "request_id": req_id,
                "result": res
            })
            .to_string(),
        )
    } else {
        let err = envelope_resp
            .get("error")
            .cloned()
            .unwrap_or_else(|| json!({"code": "rejected", "message": "operation rejected"}));
        to_owned_c_string(
            json!({
                "ok": false,
                "request_id": req_id,
                "error": err
            })
            .to_string(),
        )
    }
}

/// Attest an edited resend host policy against the application store.
/// Validates `core_config` and `submission` structure and verifies target conversation/prepared resend
/// existence and consistency against `app.inner.store`.
///
/// # Safety
/// `app` must be valid; `target_ptr`/`config_ptr` must point to readable bytes when lengths > 0.
#[no_mangle]
pub unsafe extern "C" fn nola_core_application_attest_resend_host_policy(
    app: *mut NolaCoreApplication,
    target_ptr: *const u8,
    target_len: usize,
    config_ptr: *const u8,
    config_len: usize,
) -> *mut c_char {
    let fail = |code: &str, message: &str| {
        to_owned_c_string(
            json!({"ok": false, "error": {"code": code, "message": message}}).to_string(),
        )
    };
    if app.is_null() {
        return fail("null_application", "null application handle");
    }
    if target_ptr.is_null() || target_len == 0 || target_len > 512 {
        return fail("invalid_argument", "target must be non-empty and <= 512 bytes");
    }
    let target_slice = unsafe { std::slice::from_raw_parts(target_ptr, target_len) };
    let target_str = match std::str::from_utf8(target_slice) {
        Ok(s) if !s.trim().is_empty() => s.trim(),
        _ => return fail("invalid_argument", "target must be non-empty UTF-8"),
    };
    let Some(config) = (unsafe { parse_json_slice(config_ptr, config_len) }) else {
        return fail(
            "resend_host_policy_rejected",
            "host policy configuration must be valid JSON",
        );
    };
    let Some(cfg_obj) = config.as_object() else {
        return fail(
            "resend_host_policy_rejected",
            "host policy configuration must be a JSON object",
        );
    };
    let Some(core_config) = cfg_obj.get("core_config").filter(|v| v.is_object()) else {
        return fail(
            "resend_host_policy_rejected",
            "missing required 'core_config' object",
        );
    };
    let Some(submission) = cfg_obj.get("submission").filter(|v| v.is_object()) else {
        return fail(
            "resend_host_policy_rejected",
            "missing required 'submission' object",
        );
    };

    let app_ref = unsafe { &*app };
    let store = &app_ref.inner.store;
    let session = store.get_session(target_str).ok().flatten();
    let prepared_edited = store
        .get_kv(&format!("prepared_edited_resend:{target_str}"))
        .ok()
        .flatten();
    let prepared_in_place = store
        .get_kv(&format!("prepared_in_place_resend:{target_str}"))
        .ok()
        .flatten();
    if session.is_none() && prepared_edited.is_none() && prepared_in_place.is_none() {
        return fail(
            "resend_host_policy_rejected",
            "target conversation or prepared resend does not exist in store",
        );
    }

    if let Some(cfg_conv) = core_config
        .get("conversation_id")
        .or_else(|| core_config.get("session_id"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
    {
        if cfg_conv != target_str {
            return fail(
                "resend_host_policy_rejected",
                "core_config conversation_id does not match target",
            );
        }
    }
    if let Some(sub_conv) = submission
        .get("conversation_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
    {
        if sub_conv != target_str {
            return fail(
                "resend_host_policy_rejected",
                "submission conversation_id does not match target",
            );
        }
    }
    if let Some(prepared) = prepared_edited.as_ref().or(prepared_in_place.as_ref()) {
        if let Some(prep_conv) = prepared
            .get("conversation_id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
        {
            if prep_conv != target_str {
                return fail(
                    "resend_host_policy_rejected",
                    "prepared resend record conversation_id mismatch",
                );
            }
        }
        if let (Some(prep_msg), Some(sub_msg)) = (
            prepared
                .get("target_message_id")
                .or_else(|| prepared.get("message_id"))
                .and_then(|v| v.as_str()),
            submission
                .get("target_message_id")
                .or_else(|| submission.get("message_id"))
                .and_then(|v| v.as_str()),
        ) {
            if prep_msg != sub_msg {
                return fail(
                    "resend_host_policy_rejected",
                    "submission message_id does not match prepared resend target",
                );
            }
        }
    }

    let canonical_payload = json!({
        "target": target_str,
        "core_config": core_config,
        "submission": submission
    });
    let digest = sha256_hex(canonical_payload.to_string().as_bytes());
    let _ = store.set_kv(
        &format!("resend_attestation:{target_str}"),
        &json!({
            "attested": true,
            "target": target_str,
            "conversation_id": target_str,
            "policy_digest": digest,
            "attested_at_ms": now_ms()
        }),
    );
    to_owned_c_string(
        json!({
            "ok": true,
            "result": {
                "attested": true,
                "target": target_str,
                "conversation_id": target_str,
                "policy_digest": digest
            }
        })
        .to_string(),
    )
}

/// Ingest personal intelligence events, memory candidates, and task outcomes into SQLite FTS5.
///
/// # Safety
/// `app` must be valid; `json_ptr` must point to `json_len` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn nola_core_application_intelligence_ingest(
    app: *mut NolaCoreApplication,
    json_ptr: *const u8,
    json_len: usize,
) -> *mut c_char {
    if app.is_null() {
        return to_owned_c_string(json!({"ok": false, "error": "null application"}).to_string());
    }
    let app_ref = unsafe { &*app };
    let req = unsafe { parse_json_slice(json_ptr, json_len) }.unwrap_or_else(|| json!({}));
    let ts = now_ms();
    let mut ingested_events = 0usize;
    let mut ingested_memories = 0usize;
    let mut ingested_outcomes = 0usize;

    if let Some(events) = req.get("events").and_then(|v| v.as_array()) {
        for ev in events {
            let id = ev
                .get("id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(new_id);
            let title = ev
                .get("subject")
                .or_else(|| ev.get("kind"))
                .and_then(|v| v.as_str())
                .unwrap_or("event")
                .to_string();
            let summary = ev
                .get("summary")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let item = MemoryItem {
                id: id.clone(),
                namespace: "intelligence:event".into(),
                title,
                content: if summary.is_empty() { ev.to_string() } else { summary },
                tags: vec![ev.to_string()],
                created_at_ms: ts,
                updated_at_ms: ts,
            };
            if app_ref.inner.store.upsert_memory(&item).is_ok() {
                ingested_events += 1;
            }
        }
    }

    if let Some(cands) = req.get("memory_candidates").and_then(|v| v.as_array()) {
        for cand in cands {
            let id = cand
                .get("semantic_key")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(|k| format!("mem:{k}"))
                .unwrap_or_else(new_id);
            let statement = cand
                .get("statement")
                .or_else(|| cand.get("content"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let kind = cand
                .get("kind")
                .and_then(|v| v.as_str())
                .unwrap_or("semantic")
                .to_string();
            let mut cand_val = cand.clone();
            if let Some(obj) = cand_val.as_object_mut() {
                let missing_status = obj
                    .get("status")
                    .and_then(|v| v.as_str())
                    .map(|s| s.trim().is_empty())
                    .unwrap_or(true);
                if missing_status {
                    obj.insert("status".into(), json!("active"));
                }
            }
            let item = MemoryItem {
                id,
                namespace: "intelligence:memory".into(),
                title: kind,
                content: statement,
                tags: vec![cand_val.to_string()],
                created_at_ms: ts,
                updated_at_ms: ts,
            };
            if app_ref.inner.store.upsert_memory(&item).is_ok() {
                ingested_memories += 1;
            }
        }
    }

    if let Some(outcomes) = req.get("outcomes").and_then(|v| v.as_array()) {
        for out in outcomes {
            let id = out
                .get("id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(new_id);
            let task_type = out
                .get("task_type")
                .and_then(|v| v.as_str())
                .unwrap_or("task")
                .to_string();
            let item = MemoryItem {
                id,
                namespace: "intelligence:outcome".into(),
                title: task_type,
                content: out.to_string(),
                tags: vec![out.to_string()],
                created_at_ms: ts,
                updated_at_ms: ts,
            };
            if app_ref.inner.store.upsert_memory(&item).is_ok() {
                ingested_outcomes += 1;
            }
        }
    }

    // Also support flat `{title, content}` ingestion for simple callers.
    if ingested_events == 0 && ingested_memories == 0 && ingested_outcomes == 0 {
        let content = req
            .get("content")
            .or_else(|| req.get("text"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if !content.is_empty() {
            let id = req
                .get("id")
                .and_then(|v| v.as_str())
                .map(ToString::to_string)
                .unwrap_or_else(new_id);
            let title = req
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("Intelligence")
                .to_string();
            let item = MemoryItem {
                id: id.clone(),
                namespace: "intelligence:memory".into(),
                title,
                content,
                tags: Vec::new(),
                created_at_ms: ts,
                updated_at_ms: ts,
            };
            if app_ref.inner.store.upsert_memory(&item).is_ok() {
                ingested_memories += 1;
            }
        }
    }

    to_owned_c_string(
        json!({
            "ok": true,
            "ingested_events": ingested_events,
            "ingested_memories": ingested_memories,
            "ingested_outcomes": ingested_outcomes
        })
        .to_string(),
    )
}

/// Recall matching intelligence memory items and build formatted recall context.
///
/// # Safety
/// `app` must be valid; `json_ptr` must point to `json_len` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn nola_core_application_intelligence_recall(
    app: *mut NolaCoreApplication,
    json_ptr: *const u8,
    json_len: usize,
) -> *mut c_char {
    if app.is_null() {
        return to_owned_c_string(json!({"ok": false, "context": "", "items": []}).to_string());
    }
    let app_ref = unsafe { &*app };
    let req = unsafe { parse_json_slice(json_ptr, json_len) }.unwrap_or_else(|| json!({}));
    let query = req.get("query").and_then(|v| v.as_str()).unwrap_or("");
    let limit = req.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as usize;
    let mut items = app_ref
        .inner
        .store
        .search_memory(query, Some("intelligence:memory"), limit)
        .unwrap_or_default();
    if items.len() < limit {
        let mut evs = app_ref
            .inner
            .store
            .search_memory(query, Some("intelligence:event"), limit - items.len())
            .unwrap_or_default();
        items.append(&mut evs);
    }
    let context = items
        .iter()
        .map(|i| format!("- {}: {}", i.title, i.content))
        .collect::<Vec<_>>()
        .join("\n");
    to_owned_c_string(json!({"ok": true, "context": context, "items": items}).to_string())
}

/// Snapshot stored intelligence state in schema v1 format (`events`, `memories`, `strategies`, `outcomes`).
///
/// # Safety
/// `app` must be valid.
#[no_mangle]
pub unsafe extern "C" fn nola_core_application_intelligence_snapshot(
    app: *mut NolaCoreApplication,
) -> *mut c_char {
    if app.is_null() {
        return to_owned_c_string(
            json!({"schema_version": 1, "events": [], "memories": [], "strategies": [], "outcomes": []})
                .to_string(),
        );
    }
    let app_ref = unsafe { &*app };
    let parse_items = |ns: &str| -> Vec<Value> {
        app_ref
            .inner
            .store
            .search_memory("", Some(ns), 500)
            .unwrap_or_default()
            .into_iter()
            .map(|m| {
                m.tags
                    .first()
                    .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
                    .unwrap_or_else(|| {
                        json!({"id": m.id, "title": m.title, "statement": m.content, "content": m.content, "status": "active"})
                    })
            })
            .collect()
    };
    let events = parse_items("intelligence:event");
    let memories = parse_items("intelligence:memory");
    let outcomes = parse_items("intelligence:outcome");
    let strategies: Vec<Value> = app_ref
        .inner
        .store
        .list_module_records::<Value>("memory_strategy")
        .unwrap_or_default();
    to_owned_c_string(
        json!({
            "schema_version": 1,
            "events": events,
            "memories": memories,
            "strategies": strategies,
            "outcomes": outcomes
        })
        .to_string(),
    )
}

/// Clear stored intelligence memory items across all intelligence namespaces.
///
/// # Safety
/// `app` must be valid.
#[no_mangle]
pub unsafe extern "C" fn nola_core_application_intelligence_clear(
    app: *mut NolaCoreApplication,
) -> bool {
    if app.is_null() {
        return false;
    }
    let app_ref = unsafe { &*app };
    for ns in [
        "intelligence",
        "intelligence:event",
        "intelligence:memory",
        "intelligence:outcome",
    ] {
        let items = app_ref
            .inner
            .store
            .search_memory("", Some(ns), 1000)
            .unwrap_or_default();
        for item in items {
            let _ = app_ref.inner.store.delete_memory(&item.id);
        }
    }
    true
}

fn resolve_recall_store(req: &Value) -> Option<Store> {
    if let Some(path) = req
        .get("db_path")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
    {
        if let Ok(store) = Store::open(Path::new(path)) {
            return Some(store);
        }
    }
    if let Ok(guard) = active_app_store_slot().lock() {
        if let Some(store) = guard.as_ref() {
            return Some(store.clone());
        }
    }
    let default_db = crate::config::default_db_path();
    if default_db.exists() {
        if let Ok(store) = Store::open(&default_db) {
            return Some(store);
        }
    }
    None
}

/// Personal intelligence recall: scores caller-supplied `items`/`memories`/`records` when present,
/// or queries the active/default `Store` (`search_memory_v2` + `list_module_records("memory_strategy")`)
/// when `items` is omitted (matching macOS `MacPersonalIntelligenceCore`).
///
/// # Safety
/// `json_ptr` must point to `json_len` readable UTF-8 bytes.
#[no_mangle]
pub unsafe extern "C" fn nola_core_intelligence_recall(
    json_ptr: *const u8,
    json_len: usize,
) -> *mut c_char {
    let Some(req) = (unsafe { parse_json_slice(json_ptr, json_len) }) else {
        return to_owned_c_string(
            json!({"ok": true, "context": "", "items": [], "strategies": []}).to_string(),
        );
    };
    let query = req
        .get("query")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    let limit = req
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(20)
        .clamp(1, 200) as usize;
    let minimum_score = req
        .get("minimum_score")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);

    let caller_items = req
        .get("items")
        .or_else(|| req.get("memories"))
        .or_else(|| req.get("records"))
        .and_then(|v| v.as_array())
        .cloned();

    let query_tokens: Vec<String> = query
        .split(|c: char| !c.is_alphanumeric() && c != '_' && c != '-')
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| s.chars().count() >= 2)
        .collect();

    if let Some(raw_items) = caller_items {
        let mut scored: Vec<(usize, i64, Value)> = Vec::new();
        for item in raw_items {
            let title = item
                .get("title")
                .or_else(|| item.get("summary"))
                .or_else(|| item.get("kind"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let body = item
                .get("content")
                .or_else(|| item.get("statement"))
                .or_else(|| item.get("text"))
                .and_then(|v| v.as_str())
                .or_else(|| item.as_str())
                .unwrap_or("");
            let tags_text = item
                .get("tags")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|t| t.as_str())
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .unwrap_or_default();
            let haystack = format!("{title} {body} {tags_text}").to_ascii_lowercase();
            let score = if query_tokens.is_empty() {
                1usize
            } else {
                query_tokens
                    .iter()
                    .filter(|tok| haystack.contains(tok.as_str()))
                    .count()
            };
            if score == 0 {
                continue;
            }
            let ratio = if query_tokens.is_empty() {
                1.0
            } else {
                score as f64 / query_tokens.len() as f64
            };
            if ratio < minimum_score {
                continue;
            }
            let recency = item
                .get("updated_at_ms")
                .or_else(|| item.get("created_at_ms"))
                .or_else(|| item.get("timestamp_ms"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            scored.push((score, recency, item));
        }

        scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
        scored.truncate(limit);

        let items: Vec<Value> = scored.into_iter().map(|(_, _, v)| v).collect();
        let context = format_recall_context(&items, &[]);
        return to_owned_c_string(
            json!({"ok": true, "context": context, "items": items, "strategies": []}).to_string(),
        );
    }

    // Store-backed recall when `items` is omitted (macOS `MacPersonalIntelligenceCore`).
    let Some(store) = resolve_recall_store(&req) else {
        return to_owned_c_string(
            json!({"ok": true, "context": "", "items": [], "strategies": []}).to_string(),
        );
    };

    let filter = MemorySearchFilter::default();
    let mut recalled_v2 = search_memory_v2(&store, query, &filter, limit).unwrap_or_default();
    if recalled_v2.is_empty() && !query.is_empty() {
        // Fallback: broader token search over all stored memories
        if let Ok(all_mem) = store.search_memory("", None, 200) {
            for m in all_mem {
                let hay = format!("{} {} {}", m.title, m.content, m.tags.join(" "))
                    .to_ascii_lowercase();
                let hits = query_tokens
                    .iter()
                    .filter(|tok| hay.contains(tok.as_str()))
                    .count();
                if hits > 0 {
                    let meta = nola_modules::load_memory_metadata(&store, &m.id);
                    recalled_v2.push(nola_modules::MemoryRecordV2::from_parts(m, meta));
                }
            }
            recalled_v2.truncate(limit);
        }
    }

    let mut items: Vec<Value> = Vec::new();
    for rec in recalled_v2 {
        let hay = format!(
            "{} {} {}",
            rec.item.title,
            rec.item.content,
            rec.item.tags.join(" ")
        )
        .to_ascii_lowercase();
        let score = if query_tokens.is_empty() {
            1.0
        } else {
            let hits = query_tokens
                .iter()
                .filter(|tok| hay.contains(tok.as_str()))
                .count();
            if hits > 0 {
                (hits as f64 / query_tokens.len() as f64).max(0.5)
            } else {
                // FTS5 BM25 matched stem/CJK substring
                0.5
            }
        };
        if score < minimum_score {
            continue;
        }
        items.push(json!({
            "id": rec.item.id,
            "namespace": rec.item.namespace,
            "title": rec.item.title,
            "content": rec.item.content,
            "tags": rec.item.tags,
            "score": score,
            "confidence": rec.confidence,
            "importance": rec.importance,
            "updated_at_ms": rec.item.updated_at_ms
        }));
    }

    let raw_strategies = store
        .list_module_records::<Value>("memory_strategy")
        .unwrap_or_default();
    let mut scored_strategies: Vec<(usize, Value)> = Vec::new();
    for strat in raw_strategies {
        let text = strat.to_string().to_ascii_lowercase();
        let hits = if query_tokens.is_empty() {
            1
        } else {
            query_tokens
                .iter()
                .filter(|tok| text.contains(tok.as_str()))
                .count()
        };
        if hits > 0 {
            let ratio = if query_tokens.is_empty() {
                1.0
            } else {
                hits as f64 / query_tokens.len() as f64
            };
            if ratio >= minimum_score {
                scored_strategies.push((hits, strat));
            }
        }
    }
    scored_strategies.sort_by(|a, b| b.0.cmp(&a.0));
    scored_strategies.truncate(limit);
    let strategies: Vec<Value> = scored_strategies.into_iter().map(|(_, v)| v).collect();

    let context = format_recall_context(&items, &strategies);
    to_owned_c_string(
        json!({
            "ok": true,
            "context": context,
            "items": items,
            "strategies": strategies
        })
        .to_string(),
    )
}

fn format_recall_context(items: &[Value], strategies: &[Value]) -> String {
    let mut lines: Vec<String> = items
        .iter()
        .filter_map(|item| {
            if let Some(s) = item.as_str() {
                return Some(format!("- {s}"));
            }
            let title = item
                .get("title")
                .or_else(|| item.get("summary"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let content = item
                .get("content")
                .or_else(|| item.get("statement"))
                .or_else(|| item.get("text"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if title.is_empty() && content.is_empty() {
                None
            } else if title.is_empty() {
                Some(format!("- {content}"))
            } else {
                Some(format!("- {title}: {content}"))
            }
        })
        .collect();
    for strat in strategies {
        let title = strat
            .get("title")
            .or_else(|| strat.get("name"))
            .or_else(|| strat.get("id"))
            .and_then(|v| v.as_str())
            .unwrap_or("Strategy");
        let summary = strat
            .get("summary")
            .or_else(|| strat.get("content"))
            .or_else(|| strat.get("guidance"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if !summary.is_empty() {
            lines.push(format!("- [Strategy] {title}: {summary}"));
        }
    }
    lines.join("\n")
}

/// Deterministic context compaction planner for mobile/macOS hosts.
///
/// # Safety
/// `json_ptr` must point to `json_len` readable UTF-8 bytes.
#[no_mangle]
pub unsafe extern "C" fn nola_core_context_plan(
    json_ptr: *const u8,
    json_len: usize,
) -> *mut c_char {
    let Some(req) = (unsafe { parse_json_slice(json_ptr, json_len) }) else {
        return std::ptr::null_mut();
    };
    let window = req
        .get("context_window_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(128_000) as usize;
    let reserved = req
        .get("reserved_output_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(16_384) as usize;
    let prompt_limit = req
        .get("prompt_token_limit")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize)
        .unwrap_or_else(|| window.saturating_sub(reserved).max(4_096));
    let threshold_pct = req
        .get("threshold_percent")
        .and_then(|v| v.as_u64())
        .unwrap_or(60) as usize;
    let trigger_tokens = req
        .get("token_threshold")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize)
        .unwrap_or_else(|| (prompt_limit * threshold_pct) / 100);
    let estimated = req
        .get("estimated_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;
    let msg_count = req
        .get("message_count")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;
    let keep_recent = req
        .get("keep_recent_messages")
        .and_then(|v| v.as_u64())
        .unwrap_or(8) as usize;
    let min_msgs = req
        .get("min_messages_before_compaction")
        .and_then(|v| v.as_u64())
        .unwrap_or(4) as usize;

    let hard_limit_reached = estimated >= prompt_limit;
    let should_compact = msg_count >= min_msgs && estimated >= trigger_tokens;
    let archive_count = if should_compact {
        msg_count.saturating_sub(keep_recent)
    } else {
        0
    };
    let usage_pct = if prompt_limit > 0 {
        (estimated as f64 / prompt_limit as f64) * 100.0
    } else {
        0.0
    };
    let reason = if hard_limit_reached {
        "hard_limit_reached"
    } else if should_compact {
        "threshold_exceeded"
    } else {
        "within_budget"
    };
    to_owned_c_string(
        json!({
            "should_compact": should_compact,
            "hard_limit_reached": hard_limit_reached,
            "trigger_tokens": trigger_tokens,
            "prompt_token_limit": prompt_limit,
            "target_tokens": (trigger_tokens * 75) / 100,
            "keep_recent_messages": keep_recent,
            "archive_message_count": archive_count,
            "usage_percent": usage_pct,
            "reason": reason
        })
        .to_string(),
    )
}

fn message_tool_call_ids(msg: &Value) -> Vec<String> {
    let mut ids = Vec::new();
    for key in ["tool_calls", "toolCalls"] {
        if let Some(arr) = msg.get(key).and_then(|v| v.as_array()) {
            for tc in arr {
                if let Some(id) = tc.get("id").and_then(|v| v.as_str()) {
                    if !id.is_empty() {
                        ids.push(id.to_string());
                    }
                }
            }
        }
    }
    if let Some(arr) = msg.get("content").and_then(|v| v.as_array()) {
        for blk in arr {
            if blk.get("type").and_then(|v| v.as_str()) == Some("tool_use") {
                if let Some(id) = blk.get("id").and_then(|v| v.as_str()) {
                    if !id.is_empty() {
                        ids.push(id.to_string());
                    }
                }
            }
        }
    }
    ids
}

fn message_tool_result_ids(msg: &Value) -> Vec<String> {
    let mut ids = Vec::new();
    if let Some(id) = msg
        .get("tool_call_id")
        .or_else(|| msg.get("toolCallId"))
        .or_else(|| msg.get("tool_use_id"))
        .and_then(|v| v.as_str())
    {
        if !id.is_empty() {
            ids.push(id.to_string());
        }
    }
    if let Some(arr) = msg.get("content").and_then(|v| v.as_array()) {
        for blk in arr {
            if blk.get("type").and_then(|v| v.as_str()) == Some("tool_result") {
                if let Some(id) = blk
                    .get("tool_use_id")
                    .or_else(|| blk.get("tool_call_id"))
                    .and_then(|v| v.as_str())
                {
                    if !id.is_empty() {
                        ids.push(id.to_string());
                    }
                }
            }
        }
    }
    ids
}

fn is_tool_result_message(msg: &Value) -> bool {
    msg.get("role").and_then(|r| r.as_str()) == Some("tool")
        || !message_tool_result_ids(msg).is_empty()
}

/// Find the indices belonging to the oldest removable non-system atomic group
/// (never splitting a `tool_call` assistant message from its matching `tool_result` messages).
fn oldest_removable_group_indices(messages: &[Value]) -> Option<Vec<usize>> {
    let start = messages
        .iter()
        .position(|m| m.get("role").and_then(|r| r.as_str()) != Some("system"))?;
    let mut group = vec![start];
    let call_ids: BTreeSet<String> = message_tool_call_ids(&messages[start]).into_iter().collect();
    if !call_ids.is_empty() {
        for (idx, msg) in messages.iter().enumerate().skip(start + 1) {
            if msg.get("role").and_then(|r| r.as_str()) == Some("tool") {
                let res_ids = message_tool_result_ids(msg);
                if res_ids.is_empty() || res_ids.iter().any(|id| call_ids.contains(id)) {
                    group.push(idx);
                    continue;
                }
            }
            let res_ids = message_tool_result_ids(msg);
            if !res_ids.is_empty() && res_ids.iter().any(|id| call_ids.contains(id)) {
                group.push(idx);
                continue;
            }
            if !is_tool_result_message(msg) {
                break;
            }
        }
    }
    Some(group)
}

/// Deterministic context preparation using Nola v2 token estimation, tool-pair atomic pruning,
/// and real component token metrics.
///
/// # Safety
/// `json_ptr` must point to `json_len` readable UTF-8 bytes.
#[no_mangle]
pub unsafe extern "C" fn nola_core_context_prepare(
    json_ptr: *const u8,
    json_len: usize,
) -> *mut c_char {
    let Some(req) = (unsafe { parse_json_slice(json_ptr, json_len) }) else {
        return std::ptr::null_mut();
    };
    let mut messages = req
        .get("messages")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let tools = req
        .get("tools")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let budget = req.get("budget").cloned().unwrap_or_else(|| json!({}));
    let reserved = budget
        .get("reserved_output_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(16_384) as usize;
    let window = budget
        .get("context_window_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(128_000) as usize;
    let prompt_limit = budget
        .get("prompt_token_limit")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize)
        .unwrap_or_else(|| window.saturating_sub(reserved).max(4_096));
    let keep_recent = budget
        .get("keep_recent_messages")
        .and_then(|v| v.as_u64())
        .unwrap_or(4) as usize;

    let msg_tokens_before: usize = messages
        .iter()
        .map(|m| estimate_text_tokens(&m.to_string()))
        .sum();
    let tool_tokens: usize = tools
        .iter()
        .map(|t| estimate_text_tokens(&t.to_string()))
        .sum();
    let before_tokens = msg_tokens_before + tool_tokens;

    // Compact oldest non-system atomic groups when exceeding prompt_limit,
    // strictly preserving tool_call / tool_result pair integrity.
    let mut archived = 0usize;
    let mut transactions = 0usize;
    let mut current_msg_tokens = msg_tokens_before;
    while current_msg_tokens + tool_tokens > prompt_limit && messages.len() > keep_recent.max(1) {
        let Some(group_indices) = oldest_removable_group_indices(&messages) else {
            break;
        };
        if messages.len().saturating_sub(group_indices.len()) < keep_recent.max(1) {
            break;
        }
        for &idx in group_indices.iter().rev() {
            let removed = messages.remove(idx);
            current_msg_tokens =
                current_msg_tokens.saturating_sub(estimate_text_tokens(&removed.to_string()));
            archived += 1;
        }
        transactions += 1;
    }

    // Defensive orphan sweep: ensure no surviving tool_result references a pruned tool_call_id.
    let surviving_call_ids: BTreeSet<String> = messages
        .iter()
        .flat_map(message_tool_call_ids)
        .collect();
    if archived > 0 {
        let mut idx = 0;
        while idx < messages.len() {
            let res_ids = message_tool_result_ids(&messages[idx]);
            if !res_ids.is_empty() && res_ids.iter().all(|id| !surviving_call_ids.contains(id)) {
                let removed = messages.remove(idx);
                current_msg_tokens =
                    current_msg_tokens.saturating_sub(estimate_text_tokens(&removed.to_string()));
                archived += 1;
            } else {
                idx += 1;
            }
        }
    }

    // Compute real component metrics (both explicit request components and role/tool buckets).
    let mut component_metrics: Vec<Value> = Vec::new();
    if let Some(components) = req.get("components").and_then(|v| v.as_array()) {
        for comp in components {
            let id = comp
                .get("id")
                .or_else(|| comp.get("kind"))
                .and_then(|v| v.as_str())
                .unwrap_or("component");
            let kind = comp
                .get("kind")
                .and_then(|v| v.as_str())
                .unwrap_or("context");
            let text = comp
                .get("content")
                .and_then(|v| v.as_str())
                .map(ToString::to_string)
                .unwrap_or_else(|| comp.to_string());
            component_metrics.push(json!({
                "id": id,
                "component": id,
                "kind": kind,
                "tokens": estimate_text_tokens(&text),
                "chars": text.chars().count(),
                "included": true
            }));
        }
    }
    let mut role_stats: BTreeMap<String, (usize, usize, usize)> = BTreeMap::new();
    for m in &messages {
        let role = m
            .get("role")
            .and_then(|r| r.as_str())
            .unwrap_or("unknown")
            .to_string();
        let serialized = m.to_string();
        let entry = role_stats.entry(role).or_insert((0, 0, 0));
        entry.0 += 1;
        entry.1 += estimate_text_tokens(&serialized);
        entry.2 += serialized.chars().count();
    }
    for (role, (count, tokens, chars)) in role_stats {
        component_metrics.push(json!({
            "id": format!("messages:{role}"),
            "component": role,
            "kind": "messages",
            "message_count": count,
            "tokens": tokens,
            "chars": chars,
            "included": true
        }));
    }
    if !tools.is_empty() {
        let tools_chars: usize = tools.iter().map(|t| t.to_string().chars().count()).sum();
        component_metrics.push(json!({
            "id": "tools:schema",
            "component": "tools",
            "kind": "tool_schema",
            "tool_count": tools.len(),
            "tokens": tool_tokens,
            "chars": tools_chars,
            "included": true
        }));
    }

    let after_tokens = current_msg_tokens + tool_tokens;
    let changed = archived > 0;
    let action = if changed { "compacted" } else { "ready" };
    let reason = if changed {
        "compacted_to_budget"
    } else {
        "within_budget"
    };
    let usage_pct = if prompt_limit > 0 {
        (after_tokens as f64 / prompt_limit as f64) * 100.0
    } else {
        0.0
    };
    let payload_digest = sha256_hex(
        json!({"messages": &messages, "tools": &tools})
            .to_string()
            .as_bytes(),
    );

    to_owned_c_string(
        json!({
            "schema_version": 1,
            "action": action,
            "messages": messages,
            "tools": tools,
            "payload_digest": payload_digest,
            "changed": changed,
            "reason": reason,
            "metrics": {
                "message_tokens": current_msg_tokens,
                "tool_schema_tokens": tool_tokens,
                "total_prompt_tokens": after_tokens,
                "prompt_token_limit": prompt_limit,
                "reserved_output_tokens": reserved,
                "usage_percent": usage_pct,
                "before_tokens": before_tokens,
                "after_tokens": after_tokens,
                "trigger_tokens": prompt_limit,
                "target_tokens": prompt_limit,
                "archive_message_count": archived,
                "transaction_count": transactions
            },
            "component_metrics": component_metrics
        })
        .to_string(),
    )
}

/// Deterministic memory selection policy filter for mobile hosts.
///
/// # Safety
/// `json_ptr` must point to `json_len` readable UTF-8 bytes.
#[no_mangle]
pub unsafe extern "C" fn nola_core_memory_select_json(
    json_ptr: *const u8,
    json_len: usize,
) -> *mut c_char {
    let Some(req) = (unsafe { parse_json_slice(json_ptr, json_len) }) else {
        return std::ptr::null_mut();
    };
    let mode = req
        .get("mode")
        .and_then(|v| v.as_str())
        .unwrap_or("on_demand");
    let long_term_enabled = req
        .pointer("/policy/long_term_enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let min_conf = req
        .pointer("/policy/minimum_confidence")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let search_limit = req
        .pointer("/policy/search_limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(20) as usize;
    let allowed_sensitivities: Vec<String> = req
        .pointer("/policy/allowed_sensitivities")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(ToString::to_string))
                .collect()
        })
        .unwrap_or_default();
    let allowed_namespaces: Vec<Value> = req
        .get("allowed_namespaces")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let include_session = req
        .get("include_session_retention")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let max_tokens = req
        .get("maximum_input_tokens")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize);
    let token_accounting = req
        .get("token_accounting")
        .and_then(|v| v.as_str())
        .unwrap_or("core_estimate");
    let token_counts = req.get("token_counts").and_then(|v| v.as_object());
    let records = req
        .get("records")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    // Collect superseded memory IDs across all records.
    let mut superseded_ids = std::collections::BTreeSet::new();
    for rec in &records {
        if let Some(arr) = rec.get("supersedes").and_then(|v| v.as_array()) {
            for s in arr {
                if let Some(sid) = s.as_str() {
                    superseded_ids.insert(sid.to_string());
                }
            }
        }
    }

    let mut selected = Vec::new();
    let mut rejected = Vec::new();
    let mut used_tokens = 0usize;
    let reason_tag = if mode == "resident_profile" {
        "resident_profile"
    } else {
        "host_ranked_on_demand"
    };

    for rec in records {
        let id = rec
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if !long_term_enabled {
            rejected.push(json!({"memory_id": id, "reason": "disabled"}));
            continue;
        }
        if superseded_ids.contains(&id) {
            rejected.push(json!({"memory_id": id, "reason": "superseded"}));
            continue;
        }
        if !allowed_namespaces.is_empty() {
            if let Some(ns) = rec.get("namespace") {
                if !allowed_namespaces.contains(ns) {
                    rejected.push(json!({"memory_id": id, "reason": "namespace_not_allowed"}));
                    continue;
                }
            }
        }
        if !allowed_sensitivities.is_empty() {
            if let Some(sens) = rec.get("sensitivity").and_then(|v| v.as_str()) {
                if !allowed_sensitivities.iter().any(|a| a == sens) {
                    rejected.push(json!({"memory_id": id, "reason": "sensitivity_not_allowed"}));
                    continue;
                }
            }
        }
        if !include_session
            && rec.pointer("/retention/mode").and_then(|v| v.as_str()) == Some("session")
        {
            rejected.push(json!({"memory_id": id, "reason": "session_only"}));
            continue;
        }
        let conf = rec.get("confidence").and_then(|v| v.as_f64()).unwrap_or(1.0);
        if conf < min_conf {
            rejected.push(json!({"memory_id": id, "reason": "below_confidence"}));
            continue;
        }
        if selected.len() >= search_limit {
            rejected.push(json!({"memory_id": id, "reason": "limit_exceeded"}));
            continue;
        }
        let content = rec.get("content").and_then(|v| v.as_str()).unwrap_or("");
        let (tok, is_est) = if token_accounting == "host_exact" {
            if let Some(exact) = token_counts
                .and_then(|m| m.get(&id))
                .and_then(|v| v.as_u64())
            {
                (exact as usize, false)
            } else if max_tokens.is_some() {
                rejected.push(json!({"memory_id": id, "reason": "missing_token_count"}));
                continue;
            } else {
                (estimate_text_tokens(content), true)
            }
        } else {
            (estimate_text_tokens(content), true)
        };
        if let Some(limit) = max_tokens {
            if used_tokens + tok > limit {
                rejected.push(json!({"memory_id": id, "reason": "token_budget_exceeded"}));
                continue;
            }
        }
        used_tokens += tok;
        selected.push(json!({
            "record": rec,
            "requires_current_state_verification": false,
            "token_count": tok,
            "token_count_is_estimated": is_est,
            "selection_reason": reason_tag
        }));
    }

    let remaining = max_tokens.map(|m| m.saturating_sub(used_tokens));
    let digest = sha256_hex(json!(&selected).to_string().as_bytes());
    let any_estimated = token_accounting != "host_exact";
    to_owned_c_string(
        json!({
            "mode": mode,
            "selected": selected,
            "rejected": rejected,
            "used_input_tokens": used_tokens,
            "remaining_input_tokens": remaining,
            "token_counts_are_estimated": any_estimated,
            "plan_digest": digest
        })
        .to_string(),
    )
}

fn parse_timestamp_utc(val: &Value) -> Option<DateTime<Utc>> {
    if let Some(s) = val.as_str() {
        return DateTime::parse_from_rfc3339(s.trim())
            .ok()
            .map(|dt| dt.with_timezone(&Utc));
    }
    if let Some(n) = val.as_i64() {
        // Distinguish epoch milliseconds (>= 10^12) vs epoch seconds
        if n.abs() >= 1_000_000_000_000 {
            return DateTime::from_timestamp_millis(n);
        }
        return DateTime::from_timestamp(n, 0);
    }
    None
}

fn validate_data_source_contract(contract: &Value, expected_source_id: &str) -> Result<(), &'static str> {
    let Some(obj) = contract.as_object() else {
        return Err("contract_invalid");
    };
    if obj.is_empty() {
        return Err("contract_invalid");
    }
    if let Some(state) = obj
        .get("validationState")
        .or_else(|| obj.get("validation_state"))
        .and_then(|v| v.as_str())
    {
        if state.eq_ignore_ascii_case("failed") {
            return Err("validation_failed");
        }
    }
    if let Some(cid_val) = obj.get("sourceId").or_else(|| obj.get("source_id")) {
        let Some(cid) = cid_val.as_str().map(str::trim).filter(|s| !s.is_empty()) else {
            return Err("contract_invalid");
        };
        if !expected_source_id.is_empty() && cid != expected_source_id {
            return Err("contract_invalid");
        }
    }
    // Full DataSourceContract structural check when connector/recipe fields are present
    let has_full_contract_shape = obj.contains_key("connectorId")
        || obj.contains_key("connector_id")
        || obj.contains_key("readRecipe")
        || obj.contains_key("read_recipe")
        || obj.contains_key("readerDescription")
        || obj.contains_key("reader_description");
    if has_full_contract_shape {
        let connector_id = obj
            .get("connectorId")
            .or_else(|| obj.get("connector_id"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        let connector_ver = obj
            .get("connectorVersion")
            .or_else(|| obj.get("connector_version"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let reader_desc = obj
            .get("readerDescription")
            .or_else(|| obj.get("reader_description"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        if connector_id.is_empty() || connector_ver == 0 || reader_desc.is_empty() {
            return Err("contract_invalid");
        }
        let recipe = obj.get("readRecipe").or_else(|| obj.get("read_recipe"));
        let Some(recipe_obj) = recipe.and_then(|v| v.as_object()) else {
            return Err("contract_invalid");
        };
        let method = recipe_obj
            .get("method")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let op_id = recipe_obj
            .get("operationId")
            .or_else(|| recipe_obj.get("operation_id"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        let out_type = recipe_obj
            .get("outputSchema")
            .or_else(|| recipe_obj.get("output_schema"))
            .and_then(|s| s.get("type"))
            .and_then(|v| v.as_str());
        let valid_out_type = out_type.is_some_and(|k| {
            matches!(
                k,
                "array" | "object" | "string" | "number" | "integer" | "boolean" | "null"
            )
        });
        if method.eq_ignore_ascii_case("unknown") || op_id.is_empty() || !valid_out_type {
            return Err("contract_invalid");
        }
        let schema_fields_non_empty = obj
            .get("schemaFields")
            .or_else(|| obj.get("schema_fields"))
            .and_then(|v| v.as_array())
            .is_some_and(|a| !a.is_empty());
        let scalar_output = out_type.is_some_and(|k| k != "object" && k != "array");
        if !schema_fields_non_empty && !scalar_output {
            return Err("contract_invalid");
        }
    }
    Ok(())
}

fn is_data_source_contract_stale(contract: &Value, now: DateTime<Utc>) -> Result<bool, &'static str> {
    let Some(obj) = contract.as_object() else {
        return Err("contract_invalid");
    };
    let future_skew = chrono::Duration::minutes(5);
    // Schema A: `lastRefreshedAt` + `ttlSeconds`
    let refreshed_val = obj
        .get("lastRefreshedAt")
        .or_else(|| obj.get("last_refreshed_at"));
    let ttl_val = obj.get("ttlSeconds").or_else(|| obj.get("ttl_seconds"));
    if refreshed_val.is_some() || ttl_val.is_some() {
        let Some(refreshed_at) = refreshed_val.and_then(parse_timestamp_utc) else {
            return Ok(true);
        };
        let ttl_secs = ttl_val
            .and_then(|v| v.as_i64())
            .ok_or("contract_invalid")?;
        if ttl_secs < 0 {
            return Err("contract_invalid");
        }
        if refreshed_at > now + future_skew {
            return Ok(true);
        }
        let age_secs = now.signed_duration_since(refreshed_at).num_seconds();
        return Ok(age_secs > ttl_secs);
    }

    // Schema B: `observedAt` / `validatedAt` + `maximumAgeMinutes` (Apple/Android canonical contract)
    let observed_val = obj
        .get("observedAt")
        .or_else(|| obj.get("observed_at"))
        .or_else(|| obj.get("validatedAt"))
        .or_else(|| obj.get("validated_at"));
    let max_age_min = obj
        .get("maximumAgeMinutes")
        .or_else(|| obj.get("maximum_age_minutes"))
        .and_then(|v| v.as_u64());
    if let (Some(obs_v), Some(max_min)) = (observed_val, max_age_min) {
        let Some(observed_at) = parse_timestamp_utc(obs_v) else {
            return Err("contract_invalid");
        };
        if observed_at > now + future_skew {
            return Ok(true);
        }
        let capped_secs = (max_min.min(10_080) as i64) * 60;
        let age_secs = now.signed_duration_since(observed_at).num_seconds();
        return Ok(age_secs > capped_secs);
    }

    Ok(false)
}

/// Resolve data source availability enforcing all 4 access policies (`default_enabled`, `opt_in`,
/// `ask_each_time`, `disabled` plus camelCase aliases), contract validation, and TTL freshness.
///
/// # Safety
/// `json_ptr` must point to `json_len` readable UTF-8 bytes.
#[no_mangle]
pub unsafe extern "C" fn nola_core_data_source_resolve(
    json_ptr: *const u8,
    json_len: usize,
) -> *mut c_char {
    let Some(req) = (unsafe { parse_json_slice(json_ptr, json_len) }) else {
        return std::ptr::null_mut();
    };
    let source_id = req
        .get("sourceId")
        .or_else(|| req.get("source_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    let policy_raw = req
        .get("policy")
        .and_then(|v| v.as_str())
        .unwrap_or("disabled")
        .trim();
    let conv_override = req
        .get("conversationOverride")
        .or_else(|| req.get("conversation_override"))
        .and_then(|v| v.as_bool());
    let now = req
        .get("now")
        .and_then(parse_timestamp_utc)
        .unwrap_or_else(Utc::now);

    let emit = |enabled: bool,
                usable: bool,
                should_request_consent: bool,
                should_refresh: bool,
                stale: bool,
                reason: &str| {
        to_owned_c_string(
            json!({
                "schemaVersion": 1,
                "enabled": enabled,
                "usable": usable,
                "shouldRequestConsent": should_request_consent,
                "shouldRefresh": should_refresh,
                "stale": stale,
                "reason": reason
            })
            .to_string(),
        )
    };

    // Policy gate: strictly enforce all 4 access policies without consent bypass.
    match policy_raw {
        "disabled" | "blocked" | "off" => {
            return emit(false, false, false, false, false, "blocked");
        }
        "default_enabled" | "defaultEnabled" => {
            if conv_override == Some(false) {
                return emit(false, false, false, false, false, "disabled_for_conversation");
            }
        }
        "opt_in" | "optIn" | "conversation_opt_in" | "conversationOptIn" => match conv_override {
            Some(true) => {}
            Some(false) => {
                return emit(false, false, false, false, false, "disabled_for_conversation");
            }
            None => {
                return emit(
                    false,
                    false,
                    false,
                    false,
                    false,
                    "conversation_opt_in_required",
                );
            }
        },
        "ask_each_time" | "askEachTime" | "ask_when_relevant" | "askWhenRelevant" | "ask" => {
            match conv_override {
                Some(true) => {}
                Some(false) => {
                    return emit(false, false, false, false, false, "disabled_for_conversation");
                }
                None => {
                    return emit(false, false, true, false, false, "consent_required");
                }
            }
        }
        _ => {
            // Fail closed on unknown policy names.
            return emit(false, false, false, false, false, "blocked");
        }
    }

    let Some(contract) = req.get("contract").filter(|v| !v.is_null()) else {
        return emit(true, false, false, true, false, "contract_missing");
    };
    if let Err(reason) = validate_data_source_contract(contract, source_id) {
        return emit(true, false, false, true, false, reason);
    }
    match is_data_source_contract_stale(contract, now) {
        Err(reason) => emit(true, false, false, true, false, reason),
        Ok(true) => emit(true, false, false, true, true, "stale"),
        Ok(false) => emit(true, true, false, false, false, "ready"),
    }
}

/// Evaluate managed resource access control against owner & strictly bound grants.
///
/// # Safety
/// `json_ptr` must point to `json_len` readable UTF-8 bytes.
#[no_mangle]
pub unsafe extern "C" fn nola_core_resource_access_evaluate_json(
    json_ptr: *const u8,
    json_len: usize,
) -> *mut c_char {
    let Some(req) = (unsafe { parse_json_slice(json_ptr, json_len) }) else {
        return std::ptr::null_mut();
    };
    let resource = req.get("resource");
    let res_kind = resource.and_then(|r| r.get("kind")).and_then(|v| v.as_str());
    let res_id = resource
        .and_then(|r| r.get("resource_id"))
        .and_then(|v| v.as_str());
    let res_host = resource
        .and_then(|r| r.get("host_id"))
        .and_then(|v| v.as_str());
    let res_rev = resource
        .and_then(|r| r.get("revision"))
        .and_then(|v| v.as_u64());
    let res_digest = resource
        .and_then(|r| r.get("content_digest"))
        .and_then(|v| v.as_str());
    let owner = resource.and_then(|r| r.get("owner"));
    let actor = req.get("actor");
    let permission = req.get("permission").and_then(|v| v.as_str()).unwrap_or("");
    let now_epoch = req
        .get("now_epoch_ms")
        .and_then(|v| v.as_i64())
        .unwrap_or_else(now_ms);
    let is_owner = owner.is_some() && owner == actor && !permission.is_empty();
    let granted = if is_owner {
        true
    } else if res_kind.is_none() || res_id.is_none() || permission.is_empty() {
        false
    } else {
        req.get("grants")
            .and_then(|v| v.as_array())
            .is_some_and(|grants| {
                grants.iter().any(|g| {
                    let kind_match = g.get("resource_kind").and_then(|v| v.as_str()) == res_kind;
                    let id_match = g.get("resource_id").and_then(|v| v.as_str()) == res_id;
                    let host_match = match g.get("host_id").and_then(|v| v.as_str()) {
                        Some(gh) => res_host == Some(gh),
                        None => true,
                    };
                    let rev_match = match g.get("resource_revision").and_then(|v| v.as_u64()) {
                        Some(gr) => res_rev == Some(gr),
                        None => true,
                    };
                    let digest_match = match g.get("content_digest").and_then(|v| v.as_str()) {
                        Some(gd) => res_digest == Some(gd),
                        None => true,
                    };
                    let principal_match = g.get("principal") == actor
                        || g.pointer("/principal/kind").and_then(|v| v.as_str()) == Some("anyone");
                    let not_expired = g
                        .get("expires_at_epoch_ms")
                        .and_then(|v| v.as_i64())
                        .is_none_or(|exp| exp > now_epoch);
                    let perm_match = g
                        .get("permissions")
                        .and_then(|v| v.as_array())
                        .is_some_and(|ps| ps.iter().any(|p| p.as_str() == Some(permission)));
                    kind_match
                        && id_match
                        && host_match
                        && rev_match
                        && digest_match
                        && principal_match
                        && not_expired
                        && perm_match
                })
            })
    };
    let reason = if is_owner {
        "owner"
    } else if granted {
        "explicit_grant"
    } else {
        "denied"
    };
    to_owned_c_string(
        json!({
            "schema_version": 1,
            "decision": {
                "allowed": granted,
                "reason": reason,
                "tool_execution_authorized": false
            }
        })
        .to_string(),
    )
}

fn risk_rank(risk: &str) -> u8 {
    match risk {
        "read_only" => 0,
        "low" => 1,
        "local_write" | "write" | "medium" => 2,
        "external_write" | "network" | "high" => 3,
        "destructive" | "critical" | "unrestricted" => 4,
        _ => 2,
    }
}

fn declared_tool_metadata(tool: &Value) -> (String, String, String) {
    let effect = tool
        .get("effect")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let capability = tool
        .get("capability")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("general")
        .to_string();
    let default_perm = match effect {
        Some("read_only") => "none",
        Some("local_write" | "write") => "scoped_resource",
        _ => "agent_approval",
    };
    let permission = tool
        .get("permission")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(default_perm)
        .to_string();
    let risk = tool
        .get("risk")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .or(effect)
        .unwrap_or("medium")
        .to_string();
    (capability, permission, risk)
}

/// Resolve deterministic tool policy decision for mobile hosts (matches `tool_policy_v1_golden.json`).
/// Strictly derives policy from explicit declared `effect`/`capability`/`permission`/`risk` fields
/// rather than guessing from `tool_id` keywords.
///
/// # Safety
/// `json_ptr` must point to `json_len` readable UTF-8 bytes.
#[no_mangle]
pub unsafe extern "C" fn nola_core_tool_policy_resolve(
    json_ptr: *const u8,
    json_len: usize,
) -> *mut c_char {
    let Some(req) = (unsafe { parse_json_slice(json_ptr, json_len) }) else {
        return std::ptr::null_mut();
    };
    let platform = req
        .get("platform")
        .and_then(|v| v.as_str())
        .unwrap_or("ios");
    let now_ms_val = req.get("now_ms").and_then(|v| v.as_i64()).unwrap_or_else(now_ms);
    let runtime = req.get("runtime").cloned().unwrap_or_else(|| json!({}));
    let manifest_valid_until_ms = runtime
        .get("expires_at_ms")
        .and_then(|v| v.as_i64())
        .unwrap_or(now_ms_val + 60_000);
    let max_risk = req
        .pointer("/user_policy/max_risk")
        .and_then(|v| v.as_str())
        .unwrap_or("high");
    let user_deny_tools: Vec<String> = req
        .pointer("/user_policy/deny_tools")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str().map(ToString::to_string)).collect())
        .unwrap_or_default();
    let conv_deny_tools: Vec<String> = req
        .pointer("/conversation/deny_tools")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str().map(ToString::to_string)).collect())
        .unwrap_or_default();
    let user_deny_caps: Vec<String> = req
        .pointer("/user_policy/deny_capabilities")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str().map(ToString::to_string)).collect())
        .unwrap_or_default();
    let authorizations = runtime.get("authorizations").and_then(|v| v.as_object());
    let egress_routes: Vec<String> = req
        .pointer("/user_policy/allow_result_egress_routes")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str().map(ToString::to_string)).collect())
        .unwrap_or_default();
    let egress_grants = req
        .get("result_egress_grants")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let tools_raw: Vec<Value> = runtime
        .get("tools")
        .or_else(|| req.get("available_tools"))
        .or_else(|| req.get("tools"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut enabled_tools = Vec::new();
    let mut disabled_tools = Vec::new();

    for tool in &tools_raw {
        let tool_id = tool
            .get("tool_id")
            .or_else(|| tool.get("name"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let name = tool
            .pointer("/schema/function/name")
            .or_else(|| tool.get("name"))
            .or_else(|| tool.get("tool_id"))
            .and_then(|v| v.as_str())
            .unwrap_or(&tool_id)
            .to_string();
        let route = tool
            .get("route")
            .and_then(|v| v.as_str())
            .unwrap_or("device")
            .to_string();
        let available = tool.get("available").and_then(|v| v.as_bool()).unwrap_or(true);
        let schema = tool.get("schema").cloned().unwrap_or_else(|| json!({}));
        let (capability, permission, risk) = declared_tool_metadata(tool);
        let authorization = authorizations
            .and_then(|m| m.get(&capability))
            .and_then(|v| v.as_str())
            .unwrap_or("granted")
            .to_string();
        let requires_result_egress = true;

        let deny_reason = if !available {
            Some("tool_unavailable")
        } else if manifest_valid_until_ms <= now_ms_val {
            Some("manifest_expired")
        } else if user_deny_tools.iter().any(|d| d == &tool_id)
            || conv_deny_tools.iter().any(|d| d == &tool_id)
        {
            Some("denied_by_policy")
        } else if user_deny_caps.iter().any(|c| c == &capability) {
            Some("capability_denied")
        } else if authorization != "granted" && authorization != "not_required" {
            Some("authorization_not_granted")
        } else if risk_rank(&risk) > risk_rank(max_risk) {
            Some("risk_exceeds_user_policy")
        } else if !egress_routes.is_empty() && !egress_routes.iter().any(|r| r == &route) {
            Some("result_egress_route_denied")
        } else if !egress_grants.is_empty()
            && !egress_grants.iter().any(|g| {
                g.get("tool_id").and_then(|v| v.as_str()) == Some(tool_id.as_str())
                    && g.get("expires_at_ms")
                        .and_then(|v| v.as_i64())
                        .is_none_or(|exp| exp > now_ms_val)
            })
        {
            Some("result_egress_grant_missing")
        } else {
            None
        };

        let is_enabled = deny_reason.is_none();
        let resolved = json!({
            "tool_id": tool_id,
            "name": name,
            "enabled": is_enabled,
            "route": route,
            "capability": capability,
            "permission": permission,
            "authorization": authorization,
            "risk": risk,
            "requires_result_egress": requires_result_egress,
            "reason": deny_reason.unwrap_or("enabled"),
            "schema": schema
        });
        if is_enabled {
            enabled_tools.push(resolved);
        } else {
            disabled_tools.push(resolved);
        }
    }

    let manifest_digest = sha256_hex(runtime.to_string().as_bytes());
    let policy_digest = sha256_hex(
        json!({
            "platform": platform,
            "enabled_tools": &enabled_tools,
            "disabled_tools": &disabled_tools
        })
        .to_string()
        .as_bytes(),
    );

    to_owned_c_string(
        json!({
            "schema_version": 1,
            "platform": platform,
            "policy_digest": policy_digest,
            "manifest_digest": manifest_digest,
            "manifest_valid_until_ms": manifest_valid_until_ms,
            "enabled_tools": enabled_tools,
            "disabled_tools": disabled_tools
        })
        .to_string(),
    )
}

/// Fail-closed check whether an MCP tool invocation is permitted under mobile/macOS tool policy.
/// Accepts BOTH nested `{"policy": {...}, "descriptor": {...}}` (macOS) AND flat top-level fields.
///
/// # Safety
/// `json_ptr` must point to `json_len` readable UTF-8 bytes.
#[no_mangle]
pub unsafe extern "C" fn nola_core_tool_policy_mcp_allowed(
    json_ptr: *const u8,
    json_len: usize,
) -> bool {
    let Some(req) = (unsafe { parse_json_slice(json_ptr, json_len) }) else {
        return false;
    };

    let policy = req.get("policy");
    let desc = req.get("descriptor");

    let target_id = desc
        .and_then(|d| {
            d.get("tool_id")
                .or_else(|| d.get("id"))
                .or_else(|| d.get("tool_name"))
                .or_else(|| d.get("name"))
                .or_else(|| d.get("server_id"))
        })
        .or_else(|| {
            req.get("server_id")
                .or_else(|| req.get("tool_id"))
                .or_else(|| req.get("tool_name"))
                .or_else(|| req.get("id"))
        })
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let Some(tool_id) = target_id else {
        return false;
    };

    let denied = policy
        .and_then(|p| p.get("denied").and_then(|v| v.as_bool()))
        .or_else(|| desc.and_then(|d| d.get("denied").and_then(|v| v.as_bool())))
        .or_else(|| req.get("denied").and_then(|v| v.as_bool()))
        .unwrap_or(false);
    if denied {
        return false;
    }

    let auth_ok = policy
        .and_then(|p| p.get("authorization").and_then(|v| v.as_str()))
        .or_else(|| desc.and_then(|d| d.get("authorization").and_then(|v| v.as_str())))
        .or_else(|| req.get("authorization").and_then(|v| v.as_str()))
        .is_none_or(|a| a == "granted" || a == "not_required");
    if !auth_ok {
        return false;
    }

    // Case 1: Explicit boolean `enabled` in `policy` or top-level `req`
    if let Some(enabled) = policy
        .and_then(|p| p.get("enabled").and_then(|v| v.as_bool()))
        .or_else(|| req.get("enabled").and_then(|v| v.as_bool()))
    {
        if !enabled {
            return false;
        }
        if desc
            .and_then(|d| d.get("enabled").and_then(|v| v.as_bool()))
            == Some(false)
        {
            return false;
        }
        return true;
    }

    // Case 2: Full macOS `ToolPolicyInput` object inside `policy` (`user_policy` / `runtime`)
    if let Some(p) = policy {
        if p.get("user_policy").is_some() || p.get("runtime").is_some() {
            if desc
                .and_then(|d| d.get("enabled").and_then(|v| v.as_bool()))
                == Some(false)
            {
                return false;
            }
            let now_ms_val = p.get("now_ms").and_then(|v| v.as_i64()).unwrap_or_else(now_ms);
            if let Some(exp) = p.pointer("/runtime/expires_at_ms").and_then(|v| v.as_i64()) {
                if exp <= now_ms_val {
                    return false;
                }
            }
            let deny_hit = |ptr: &str| -> bool {
                p.pointer(ptr)
                    .and_then(|v| v.as_array())
                    .is_some_and(|arr| arr.iter().any(|x| x.as_str() == Some(tool_id)))
            };
            if deny_hit("/user_policy/deny_tools") || deny_hit("/conversation/deny_tools") {
                return false;
            }
            let capability = desc
                .and_then(|d| d.get("capability").and_then(|v| v.as_str()))
                .unwrap_or("mcp");
            if p.pointer("/user_policy/deny_capabilities")
                .and_then(|v| v.as_array())
                .is_some_and(|arr| arr.iter().any(|c| c.as_str() == Some(capability)))
            {
                return false;
            }
            let max_risk = p
                .pointer("/user_policy/max_risk")
                .and_then(|v| v.as_str())
                .unwrap_or("high");
            let risk = desc
                .and_then(|d| {
                    d.get("risk")
                        .or_else(|| d.get("effect"))
                        .and_then(|v| v.as_str())
                })
                .unwrap_or("medium");
            if risk_rank(risk) > risk_rank(max_risk) {
                return false;
            }
            if let Some(cap_auth) = p
                .pointer("/runtime/authorizations")
                .and_then(|v| v.as_object())
                .and_then(|m| m.get(capability))
                .and_then(|v| v.as_str())
            {
                if cap_auth != "granted" && cap_auth != "not_required" {
                    return false;
                }
            }
            return true;
        }
    }

    false
}

fn prompt_layer_rank(layer: &str) -> Option<u8> {
    match layer {
        "identity" => Some(0),
        "safety" => Some(1),
        "tool_policy" => Some(2),
        "execution_policy" => Some(3),
        "memory_policy" => Some(4),
        "host_capabilities" => Some(5),
        "runtime_state" => Some(6),
        "skill_context" => Some(7),
        "conversation_context" => Some(8),
        "response_style" => Some(9),
        _ => None,
    }
}

fn is_valid_prompt_trust(trust: &str) -> bool {
    matches!(
        trust,
        "policy" | "host_fact" | "host_instruction" | "user_instruction" | "reference"
    )
}

/// Deterministically compose stable cacheable prefix + dynamic suffix with canonical layer ordering,
/// strict trust/cacheability invariants (`ReferenceMarkedCacheable`, `RuntimeStateMarkedCacheable`,
/// `RuntimeStateInvalidTrust`), and SHA-256 digests.
///
/// # Safety
/// `json_ptr` must point to `json_len` readable UTF-8 bytes.
#[no_mangle]
pub unsafe extern "C" fn nola_core_compose_agent_prompt_json(
    json_ptr: *const u8,
    json_len: usize,
) -> *mut c_char {
    let err_resp = |code: &str, msg: String| {
        to_owned_c_string(
            json!({
                "schema_version": 1,
                "error": {
                    "code": code,
                    "message": msg
                }
            })
            .to_string(),
        )
    };
    let Some(req) = (unsafe { parse_json_slice(json_ptr, json_len) }) else {
        return err_resp("invalid_json", "invalid prompt composer JSON".into());
    };
    if let Some(sv) = req.get("schema_version").and_then(|v| v.as_u64()) {
        if sv != 1 {
            return err_resp(
                "incompatible_schema",
                format!("Unsupported prompt composer schema version: {sv}"),
            );
        }
    }
    let fragments = req
        .get("fragments")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    if fragments.len() > 256 {
        return err_resp(
            "too_many_fragments",
            format!("Prompt has {} fragments; maximum is 256", fragments.len()),
        );
    }

    let mut seen_ids = BTreeSet::new();
    let mut stable: Vec<((u8, usize), String, String, Value)> = Vec::new();
    let mut dynamic: Vec<((u8, usize), String, String)> = Vec::new();
    let mut total_chars = 0usize;

    for (idx, frag) in fragments.iter().enumerate() {
        let id = frag
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if id.is_empty() {
            return err_resp("empty_id", "Prompt fragment id is empty".into());
        }
        if !seen_ids.insert(id.clone()) {
            return err_resp("duplicate_id", id);
        }
        let layer = frag
            .get("layer")
            .and_then(|v| v.as_str())
            .unwrap_or("conversation_context")
            .trim();
        let Some(layer_rank) = prompt_layer_rank(layer) else {
            return err_resp("invalid_layer", format!("Unknown prompt layer '{layer}'"));
        };
        let trust = frag
            .get("trust")
            .and_then(|v| v.as_str())
            .unwrap_or("reference")
            .trim();
        if !is_valid_prompt_trust(trust) {
            return err_resp("invalid_trust", format!("Unknown prompt trust '{trust}'"));
        }
        let cacheable = frag
            .get("cacheable")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let content = frag
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if content.is_empty() {
            if trust == "policy" {
                return err_resp("empty_policy_fragment", id);
            }
            continue;
        }
        let chars = content.chars().count();
        if chars > 64_000 {
            return err_resp(
                "fragment_too_large",
                format!("Prompt fragment {id} has {chars} characters; maximum is 64000"),
            );
        }
        total_chars = total_chars.saturating_add(chars);
        if total_chars > 256_000 {
            return err_resp(
                "prompt_content_too_large",
                format!("Prompt fragments contain {total_chars} characters; maximum is 256000"),
            );
        }
        if trust == "reference" && cacheable {
            return err_resp("reference_marked_cacheable", id);
        }
        if layer == "runtime_state" {
            if cacheable {
                return err_resp("runtime_state_marked_cacheable", id);
            }
            if trust != "host_fact" {
                return err_resp("runtime_state_invalid_trust", id);
            }
        }

        if cacheable {
            let typed_frag = json!({
                "id": &id,
                "layer": layer,
                "trust": trust,
                "cacheable": true,
                "content": &content
            });
            stable.push(((layer_rank, idx), id, content, typed_frag));
        } else {
            dynamic.push(((layer_rank, idx), id, content));
        }
    }

    stable.sort_by_key(|(key, _, _, _)| *key);
    dynamic.sort_by_key(|(key, _, _)| *key);

    let mut stable_hashes = BTreeMap::new();
    let mut fragment_ids = Vec::with_capacity(stable.len() + dynamic.len());
    let mut stable_parts = Vec::with_capacity(stable.len());
    for (_, id, content, typed_frag) in stable {
        stable_hashes.insert(id.clone(), sha256_hex(typed_frag.to_string().as_bytes()));
        fragment_ids.push(id);
        stable_parts.push(content);
    }
    let mut dynamic_parts = Vec::with_capacity(dynamic.len());
    for (_, id, content) in dynamic {
        fragment_ids.push(id);
        dynamic_parts.push(content);
    }

    let stable_prefix = stable_parts.join("\n\n");
    let dynamic_suffix = dynamic_parts.join("\n\n");
    let rendered = match (stable_prefix.is_empty(), dynamic_suffix.is_empty()) {
        (true, _) => dynamic_suffix.clone(),
        (_, true) => stable_prefix.clone(),
        (false, false) => format!("{stable_prefix}\n\n{dynamic_suffix}"),
    };
    let stable_prefix_sha256 = sha256_hex(stable_prefix.as_bytes());

    to_owned_c_string(
        json!({
            "schema_version": 1,
            "composition": {
                "stable_prefix": stable_prefix,
                "dynamic_suffix": dynamic_suffix,
                "rendered": rendered,
                "fragment_ids": fragment_ids,
                "stable_prefix_sha256": stable_prefix_sha256,
                "stable_fragment_sha256": stable_hashes
            }
        })
        .to_string(),
    )
}

/// Normalize model output content, `<think>` reasoning blocks, and tool calls for mobile hosts.
///
/// # Safety
/// `json_ptr` must point to `json_len` readable UTF-8 bytes.
#[no_mangle]
pub unsafe extern "C" fn nola_core_parse_agent_model_output(
    json_ptr: *const u8,
    json_len: usize,
) -> *mut c_char {
    let Some(req) = (unsafe { parse_json_slice(json_ptr, json_len) }) else {
        return std::ptr::null_mut();
    };
    let raw_content = req.get("content").and_then(|v| v.as_str()).unwrap_or("");
    let mut reasoning = req
        .get("reasoning")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let allowed: Vec<String> = req
        .get("allowed_tool_names")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str().map(ToString::to_string)).collect())
        .unwrap_or_default();

    // Extract <think>...</think> reasoning blocks.
    let mut clean_content = String::new();
    let mut rest = raw_content;
    while let Some(start) = rest.find("<think>") {
        clean_content.push_str(&rest[..start]);
        let after_open = &rest[start + 7..];
        if let Some(end) = after_open.find("</think>") {
            let block = after_open[..end].trim();
            if !block.is_empty() {
                if !reasoning.is_empty() {
                    reasoning.push('\n');
                }
                reasoning.push_str(block);
            }
            rest = &after_open[end + 8..];
        } else {
            let block = after_open.trim();
            if !block.is_empty() {
                if !reasoning.is_empty() {
                    reasoning.push('\n');
                }
                reasoning.push_str(block);
            }
            rest = "";
            break;
        }
    }
    clean_content.push_str(rest);

    // Parse inline <tool_call>...</tool_call> JSON if native_tool_calls is empty.
    let mut raw_calls = req
        .get("native_tool_calls")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let mut warnings: Vec<String> = Vec::new();

    if raw_calls.is_empty() && clean_content.contains("<tool_call>") {
        let mut content_without_calls = String::new();
        let mut cursor = clean_content.as_str();
        let mut call_idx = 0usize;
        while let Some(start) = cursor.find("<tool_call>") {
            content_without_calls.push_str(&cursor[..start]);
            let after = &cursor[start + 11..];
            if let Some(end) = after.find("</tool_call>") {
                let payload = after[..end].trim();
                match serde_json::from_str::<Value>(payload) {
                    Ok(v) => {
                        call_idx += 1;
                        let name = v.get("name").and_then(|x| x.as_str()).unwrap_or("");
                        let args = v
                            .get("arguments")
                            .cloned()
                            .unwrap_or_else(|| json!({}));
                        let id = v
                            .get("id")
                            .and_then(|x| x.as_str())
                            .map(ToString::to_string)
                            .unwrap_or_else(|| format!("call_{call_idx}"));
                        if !name.is_empty() {
                            raw_calls.push(json!({"id": id, "name": name, "arguments": args}));
                        }
                    }
                    Err(e) => warnings.push(format!("invalid inline tool_call json: {e}")),
                }
                cursor = &after[end + 12..];
            } else {
                content_without_calls.push_str(&cursor[start..]);
                cursor = "";
                break;
            }
        }
        content_without_calls.push_str(cursor);
        clean_content = content_without_calls;
    }

    let mut filtered_calls = Vec::new();
    for call in raw_calls {
        let name = call.get("name").and_then(|v| v.as_str()).unwrap_or("");
        if !allowed.is_empty() && !allowed.iter().any(|a| a == name) {
            warnings.push(format!("disallowed tool call '{name}' dropped"));
            continue;
        }
        filtered_calls.push(call);
    }

    let finish_reason = if filtered_calls.is_empty() {
        "stop"
    } else {
        "tool_calls"
    };
    to_owned_c_string(
        json!({
            "content": clean_content.trim(),
            "reasoning": reasoning.trim(),
            "tool_calls": filtered_calls,
            "finish_reason": finish_reason,
            "warnings": warnings
        })
        .to_string(),
    )
}

/// Stitch a continuation chunk onto accumulated model output, stripping any overlapping suffix/prefix.
///
/// # Safety
/// `json_ptr` must point to `json_len` readable UTF-8 bytes.
#[no_mangle]
pub unsafe extern "C" fn nola_core_stitch_agent_model_continuation(
    json_ptr: *const u8,
    json_len: usize,
) -> *mut c_char {
    let Some(req) = (unsafe { parse_json_slice(json_ptr, json_len) }) else {
        return std::ptr::null_mut();
    };
    let acc = req
        .get("accumulated")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let cont = req
        .get("continuation")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let max_overlap = acc.len().min(cont.len()).min(256);
    let mut overlap = 0usize;
    for k in (1..=max_overlap).rev() {
        if acc.is_char_boundary(acc.len() - k)
            && cont.is_char_boundary(k)
            && acc.ends_with(&cont[..k])
        {
            overlap = k;
            break;
        }
    }
    to_owned_c_string(json!({"append": &cont[overlap..]}).to_string())
}

fn close_child_pump(pump: &NolaCoreChildPump) {
    pump.cancel.store(true, Ordering::Release);
    pump.closed.store(true, Ordering::Release);
    if let Ok(mut pending) = pump.pending_tickets.lock() {
        pending.clear();
    }
    if let Ok(mut tx) = pump.response_tx.lock() {
        tx.take();
    }
    if let Ok(mut tx) = pump.event_tx.lock() {
        tx.take();
    }
}

/// Start a real background embedded mobile child execution pump backed by `AgentEngine`
/// (when a model route is configured) or host-request ticket exchange over `mpsc::channel`
/// + `AtomicBool` cooperative cancellation.
///
/// # Safety
/// `app` must be valid; `json_ptr` must point to `json_len` readable bytes when `json_len > 0`;
/// `error_out` may be null or a valid writable pointer.
#[no_mangle]
pub unsafe extern "C" fn nola_core_child_pump_start(
    app: *mut NolaCoreApplication,
    json_ptr: *const u8,
    json_len: usize,
    error_out: *mut *mut c_char,
) -> *mut NolaCoreChildPump {
    if app.is_null() {
        if !error_out.is_null() {
            unsafe { *error_out = to_owned_c_string("null application handle".into()) };
        }
        return std::ptr::null_mut();
    }
    let req = unsafe { parse_json_slice(json_ptr, json_len) }.unwrap_or_else(|| json!({}));
    let app_ref = unsafe { &*app };
    let store = app_ref.inner.store.clone();
    let workspace_root = app_ref.inner.suite.workspace_root.clone();
    let suite = Arc::new(app_ref.inner.suite.clone());

    let pump_id = new_id();
    let cancel = Arc::new(AtomicBool::new(false));
    let finished = Arc::new(AtomicBool::new(false));
    let terminal_error: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let pending_tickets: Arc<Mutex<BTreeMap<u64, mpsc::Sender<Value>>>> =
        Arc::new(Mutex::new(BTreeMap::new()));
    let next_ticket = Arc::new(AtomicU64::new(1));

    let (ev_tx, ev_rx) = mpsc::channel::<ChildPumpEvent>();
    let (resp_tx, resp_rx) = mpsc::channel::<Value>();

    let ev_tx_worker = ev_tx.clone();
    let cancel_worker = Arc::clone(&cancel);
    let finished_worker = Arc::clone(&finished);
    let err_worker = Arc::clone(&terminal_error);
    let pending_worker = Arc::clone(&pending_tickets);
    let ticket_counter = Arc::clone(&next_ticket);
    let worker_name = format!("nola-child-pump-{}", &pump_id[..pump_id.len().min(8)]);

    let spawn_res = std::thread::Builder::new().name(worker_name).spawn(move || {
        let conv_id = req
            .get("conversation_id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(ToString::to_string)
            .unwrap_or_else(new_id);
        let run_id = req
            .get("run_id")
            .or_else(|| req.get("task_id"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(ToString::to_string)
            .unwrap_or_else(new_id);
        let assistant_message_id = req
            .get("assistant_message_id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(ToString::to_string)
            .unwrap_or_else(|| format!("{run_id}:assistant"));
        let cancellation_only = req
            .get("cancellation_only")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let prompt = req
            .get("prompt")
            .or_else(|| req.get("task_prompt"))
            .or_else(|| req.get("user_message"))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string);

        if cancellation_only {
            finished_worker.store(true, Ordering::Release);
            let _ = ev_tx_worker.send(ChildPumpEvent {
                kind: "closed".into(),
                request: None,
                event: Some(json!({"type": "cancelled", "run_id": run_id})),
                transport_finished: true,
                error: None,
            });
            return;
        }

        register_active_run(
            &store,
            &conv_id,
            json!({
                "conversation_id": conv_id,
                "run_id": run_id,
                "assistant_message_id": assistant_message_id,
                "status": "running",
                "started_at_ms": now_ms()
            }),
        );

        let outcome: Result<(), String> = if let Some(user_prompt) = prompt {
            let active_cfg = crate::config::load_active_config(&store);
            let resolved_route = crate::config::resolve_model_route(&active_cfg).ok();
            let force_host_exchange = req
                .get("host_driven")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            if let (Some(route), false) = (resolved_route, force_host_exchange) {
                match HttpModelCaller::new(120) {
                    Ok(caller) => {
                        let mut reg = ToolRegistry::with_builtin_tools();
                        suite.register_tools(&mut reg);
                        let engine =
                            AgentEngine::new(store.clone(), Arc::new(reg), Arc::new(caller));
                        let sys_prompt = req
                            .get("system_prompt")
                            .and_then(|v| v.as_str())
                            .unwrap_or("You are a helpful Nola child assistant.");
                        let max_steps = req
                            .get("max_steps")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(12) as usize;
                        let ev_tx_approval = ev_tx_worker.clone();
                        let pending_approval = Arc::clone(&pending_worker);
                        let ticket_approval = Arc::clone(&ticket_counter);
                        let cancel_approval = Arc::clone(&cancel_worker);
                        let conv_approval = conv_id.clone();
                        let run_approval = run_id.clone();
                        let approval_cb: nola_engine::ApprovalCallback =
                            Arc::new(move |spec: &ToolSpec, args: &Value, reason: &str| {
                                if cancel_approval.load(Ordering::Acquire) {
                                    return false;
                                }
                                let ticket = ticket_approval.fetch_add(1, Ordering::AcqRel);
                                let effect_id = format!("perm-{ticket}");
                                let (tx, rx) = mpsc::channel::<Value>();
                                if let Ok(mut map) = pending_approval.lock() {
                                    map.insert(ticket, tx);
                                }
                                let envelope = json!({
                                    "effect_id": effect_id,
                                    "conversation_id": conv_approval,
                                    "run_id": run_approval,
                                    "payload": {
                                        "type": "request_permission",
                                        "tool": spec.schema.name,
                                        "arguments": args,
                                        "reason": reason
                                    }
                                });
                                let _ = ev_tx_approval.send(ChildPumpEvent {
                                    kind: "request_permission".into(),
                                    request: Some(json!({"ticket": ticket, "envelope": envelope})),
                                    event: None,
                                    transport_finished: false,
                                    error: None,
                                });
                                loop {
                                    if cancel_approval.load(Ordering::Acquire) {
                                        return false;
                                    }
                                    match rx.recv_timeout(Duration::from_millis(50)) {
                                        Ok(reply) => {
                                            return reply
                                                .get("approved")
                                                .or_else(|| reply.pointer("/response/approved"))
                                                .and_then(|v| v.as_bool())
                                                .unwrap_or(false);
                                        }
                                        Err(mpsc::RecvTimeoutError::Timeout) => continue,
                                        Err(mpsc::RecvTimeoutError::Disconnected) => return false,
                                    }
                                }
                            });

                        let run_cfg = AgentRunConfig {
                            session_id: Some(conv_id.clone()),
                            resume: true,
                            route,
                            frozen_system_blocks: vec![FrozenSystemBlock {
                                id: "child_system".into(),
                                text: sys_prompt.into(),
                            }],
                            host_capabilities: HostCapabilities::default(),
                            policy: EffectivePolicy::load(
                                &workspace_root,
                                None,
                                true,
                                Some(PermissionProfile::Cautious),
                                true,
                            ),
                            hooks: HookRunner::default(),
                            approval: Some(approval_cb),
                            max_steps,
                            force_tool_set: ForceToolSet::default(),
                            stream: true,
                            json_schema: None,
                            cancel: Arc::clone(&cancel_worker),
                            auto_recall_memory: false,
                            inbox: None,
                        };
                        let ctx = ToolContext::new(&workspace_root).with_session_id(conv_id.clone());
                        let ev_tx_stream = ev_tx_worker.clone();
                        match engine.run(&ctx, &user_prompt, run_cfg, |agent_ev| {
                            let kind = match &agent_ev {
                                AgentEvent::SessionStarted { .. } => "session_started",
                                AgentEvent::TextDelta { .. } => "text_delta",
                                AgentEvent::ReasoningDelta { .. } => "reasoning_delta",
                                AgentEvent::ToolStarted { .. } => "tool_started",
                                AgentEvent::ToolFinished { .. } => "tool_finished",
                                AgentEvent::Compacted { .. } => "compacted",
                                AgentEvent::TurnCompleted { .. } => "turn_completed",
                            };
                            let _ = ev_tx_stream.send(ChildPumpEvent {
                                kind: kind.into(),
                                request: None,
                                event: serde_json::to_value(&agent_ev).ok(),
                                transport_finished: false,
                                error: None,
                            });
                        }) {
                            Ok(_) => Ok(()),
                            Err(e) => Err(e.to_string()),
                        }
                    }
                    Err(e) => Err(format!("failed initializing model caller: {e}")),
                }
            } else {
                // Host-driven child run exchange (mobile host executes model_stream/tool tickets).
                let ticket = ticket_counter.fetch_add(1, Ordering::AcqRel);
                let effect_id = format!("effect-{ticket}");
                let (t_tx, t_rx) = mpsc::channel::<Value>();
                if let Ok(mut map) = pending_worker.lock() {
                    map.insert(ticket, t_tx);
                }
                let envelope = json!({
                    "effect_id": effect_id,
                    "conversation_id": conv_id,
                    "run_id": run_id,
                    "assistant_message_id": assistant_message_id,
                    "payload": {
                        "type": "model_stream",
                        "prompt": user_prompt
                    }
                });
                let _ = ev_tx_worker.send(ChildPumpEvent {
                    kind: "host_request".into(),
                    request: Some(json!({"ticket": ticket, "envelope": envelope})),
                    event: None,
                    transport_finished: false,
                    error: None,
                });
                loop {
                    if cancel_worker.load(Ordering::Acquire) {
                        break Err("child pump cancelled".into());
                    }
                    match t_rx.recv_timeout(Duration::from_millis(50)) {
                        Ok(reply) => {
                            let final_text = reply
                                .pointer("/response/content")
                                .or_else(|| reply.pointer("/response/text"))
                                .or_else(|| reply.get("content"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            let _ = store.append_event(
                                &conv_id,
                                "assistant_message",
                                json!({
                                    "id": assistant_message_id,
                                    "run_id": run_id,
                                    "role": "assistant",
                                    "content": final_text
                                }),
                                50_000,
                            );
                            let _ = ev_tx_worker.send(ChildPumpEvent {
                                kind: "turn_completed".into(),
                                request: None,
                                event: Some(json!({
                                    "type": "turn_completed",
                                    "session_id": conv_id,
                                    "run_id": run_id,
                                    "final_text": final_text
                                })),
                                transport_finished: false,
                                error: None,
                            });
                            break Ok(());
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => continue,
                        Err(mpsc::RecvTimeoutError::Disconnected) => {
                            break Err("child pump closed before response".into());
                        }
                    }
                }
            }
        } else {
            // Idle/standby child pump waiting for host-delivered control messages until closed.
            while !cancel_worker.load(Ordering::Acquire) {
                match resp_rx.recv_timeout(Duration::from_millis(50)) {
                    Ok(msg) => {
                        if msg.get("finish").and_then(|v| v.as_bool()) == Some(true) {
                            break;
                        }
                        let _ = ev_tx_worker.send(ChildPumpEvent {
                            kind: "response_ack".into(),
                            request: None,
                            event: Some(msg),
                            transport_finished: false,
                            error: None,
                        });
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
            Ok(())
        };

        clear_active_run(&store, &conv_id);
        let err_msg = outcome.err();
        if let Ok(mut slot) = err_worker.lock() {
            *slot = err_msg.clone();
        }
        finished_worker.store(true, Ordering::Release);
        let _ = ev_tx_worker.send(ChildPumpEvent {
            kind: if err_msg.is_some() { "error" } else { "closed" }.into(),
            request: None,
            event: None,
            transport_finished: true,
            error: err_msg,
        });
    });

    match spawn_res {
        Ok(handle) => Box::into_raw(Box::new(NolaCoreChildPump {
            pump_id,
            cancel,
            closed: AtomicBool::new(false),
            event_rx: Mutex::new(ev_rx),
            event_tx: Mutex::new(Some(ev_tx)),
            response_tx: Mutex::new(Some(resp_tx)),
            pending_tickets,
            finished,
            terminal_error,
            worker: Mutex::new(Some(handle)),
        })),
        Err(e) => {
            if !error_out.is_null() {
                unsafe {
                    *error_out = to_owned_c_string(format!("failed spawning child pump: {e}"));
                }
            }
            std::ptr::null_mut()
        }
    }
}

/// Poll an embedded child pump for the next event, blocking up to `wait_ms` on `recv_timeout`.
/// Emits the exact iOS/Android envelope (`pump_id`, `transport_finished`, `kind`, `request`, `event`, `error`).
///
/// # Safety
/// `pump` must be null or a valid pointer from `nola_core_child_pump_start`.
#[no_mangle]
pub unsafe extern "C" fn nola_core_child_pump_poll(
    pump: *mut NolaCoreChildPump,
    wait_ms: u32,
) -> *mut c_char {
    if pump.is_null() {
        return to_owned_c_string(
            json!({
                "ok": true,
                "result": {
                    "pump_id": "",
                    "transport_finished": true,
                    "kind": "closed",
                    "request": Value::Null,
                    "error": Value::Null
                }
            })
            .to_string(),
        );
    }
    let pump = unsafe { &*pump };
    let timeout = Duration::from_millis(u64::from(wait_ms.min(60_000)));
    let rx_guard = match pump.event_rx.lock() {
        Ok(g) => g,
        Err(_) => {
            return to_owned_c_string(
                json!({
                    "ok": false,
                    "error": {"code": "poisoned", "message": "child pump lock poisoned"}
                })
                .to_string(),
            );
        }
    };
    match rx_guard.recv_timeout(timeout) {
        Ok(ev) => to_owned_c_string(
            json!({
                "ok": true,
                "result": {
                    "pump_id": pump.pump_id,
                    "transport_finished": ev.transport_finished,
                    "kind": ev.kind,
                    "request": ev.request,
                    "event": ev.event,
                    "error": ev.error
                }
            })
            .to_string(),
        ),
        Err(mpsc::RecvTimeoutError::Timeout) => {
            let done = pump.closed.load(Ordering::Acquire) || pump.finished.load(Ordering::Acquire);
            let err = if done {
                pump.terminal_error.lock().ok().and_then(|g| g.clone())
            } else {
                None
            };
            to_owned_c_string(
                json!({
                    "ok": true,
                    "result": {
                        "pump_id": pump.pump_id,
                        "transport_finished": done,
                        "kind": if done { "closed" } else { "idle" },
                        "request": Value::Null,
                        "event": Value::Null,
                        "error": err
                    }
                })
                .to_string(),
            )
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            let err = pump.terminal_error.lock().ok().and_then(|g| g.clone());
            to_owned_c_string(
                json!({
                    "ok": true,
                    "result": {
                        "pump_id": pump.pump_id,
                        "transport_finished": true,
                        "kind": "closed",
                        "request": Value::Null,
                        "event": Value::Null,
                        "error": err
                    }
                })
                .to_string(),
            )
        }
    }
}

/// Deliver a host response (approval, user input, or tool/model result) back to a running child pump.
///
/// # Safety
/// `pump` must be null or a valid pointer from `nola_core_child_pump_start`;
/// `json_ptr` must point to `json_len` readable bytes when `json_len > 0`.
#[no_mangle]
pub unsafe extern "C" fn nola_core_child_pump_respond(
    pump: *mut NolaCoreChildPump,
    json_ptr: *const u8,
    json_len: usize,
) -> *mut c_char {
    if pump.is_null() {
        return to_owned_c_string(
            json!({"ok": false, "error": {"code": "closed", "message": "child pump is unavailable"}})
                .to_string(),
        );
    }
    let pump = unsafe { &*pump };
    if pump.closed.load(Ordering::Acquire) {
        return to_owned_c_string(
            json!({"ok": false, "error": {"code": "closed", "message": "child pump is closed"}})
                .to_string(),
        );
    }
    let payload = unsafe { parse_json_slice(json_ptr, json_len) }.unwrap_or_else(|| json!({}));
    if let Some(pid) = payload.get("pump_id").and_then(|v| v.as_str()) {
        if !pid.is_empty() && pid != pump.pump_id {
            return to_owned_c_string(
                json!({
                    "ok": false,
                    "error": {"code": "foreign_pump_id", "message": "foreign child pump ticket"}
                })
                .to_string(),
            );
        }
    }

    if let Some(ticket) = payload.get("ticket").and_then(|v| v.as_u64()) {
        let sender = pump
            .pending_tickets
            .lock()
            .ok()
            .and_then(|mut map| map.remove(&ticket));
        if let Some(tx) = sender {
            if tx.send(payload).is_ok() {
                return to_owned_c_string(
                    json!({"ok": true, "result": {"committed": true, "pump_id": pump.pump_id, "ticket": ticket}})
                        .to_string(),
                );
            }
            return to_owned_c_string(
                json!({"ok": false, "error": {"code": "ticket_closed", "message": "child ticket waiter closed"}})
                    .to_string(),
            );
        }
        return to_owned_c_string(
            json!({"ok": false, "error": {"code": "unknown_ticket", "message": format!("unknown child pump ticket {ticket}")}})
                .to_string(),
        );
    }

    if let Ok(guard) = pump.response_tx.lock() {
        if let Some(tx) = guard.as_ref() {
            let _ = tx.send(payload);
        }
    }
    to_owned_c_string(
        json!({"ok": true, "result": {"committed": true, "pump_id": pump.pump_id}}).to_string(),
    )
}

/// Signal an embedded child pump to cancel and close its channels.
///
/// # Safety
/// `pump` must be null or a valid pointer from `nola_core_child_pump_start`.
#[no_mangle]
pub unsafe extern "C" fn nola_core_child_pump_close(pump: *mut NolaCoreChildPump) {
    if !pump.is_null() {
        close_child_pump(unsafe { &*pump });
    }
}

/// Cancel, join the background worker thread, and free an embedded child pump.
///
/// # Safety
/// `pump` must be null or a valid pointer from `nola_core_child_pump_start`.
#[no_mangle]
pub unsafe extern "C" fn nola_core_child_pump_free(pump: *mut NolaCoreChildPump) {
    if !pump.is_null() {
        let pump = unsafe { Box::from_raw(pump) };
        close_child_pump(&pump);
        let handle = pump.worker.lock().ok().and_then(|mut slot| slot.take());
        if let Some(handle) = handle {
            let _ = handle.join();
        }
    }
}

struct McpCancelGuard {
    keys: Vec<String>,
}

impl Drop for McpCancelGuard {
    fn drop(&mut self) {
        if let Ok(mut map) = active_mcp_cancels().lock() {
            for k in &self.keys {
                map.remove(k);
            }
        }
    }
}

fn split_qualified_mcp_tool(raw: &str) -> Option<(&str, &str)> {
    let rest = raw.strip_prefix("mcp__")?;
    let (srv, tool) = rest.split_once("__")?;
    if srv.is_empty() || tool.is_empty() {
        None
    } else {
        Some((srv, tool))
    }
}

/// Dispatch mobile MCP host management commands backed by `Store` & `McpManager`.
/// Supports iOS/Android multi-server `prepare` (`host_id`/`session_id`/`policy`),
/// nested `request` and top-level `execute` wired to `McpManager::call_tool_with_options`,
/// and real cooperative `cancel` flipping the in-flight `AtomicBool` cancel flag.
/// Always wraps success in `{"ok": true, "result": ...}`. Allows `app.is_null()` for `"cancel"`.
///
/// # Safety
/// `app` may be null for `"cancel"`, otherwise must be valid; `json_ptr` must point to `json_len` readable UTF-8 bytes.
#[no_mangle]
pub unsafe extern "C" fn nola_core_mobile_mcp_host(
    app: *mut NolaCoreApplication,
    json_ptr: *const u8,
    json_len: usize,
) -> *mut c_char {
    let req = unsafe { parse_json_slice(json_ptr, json_len) }.unwrap_or_else(|| json!({}));
    let op = req
        .get("operation")
        .or_else(|| req.get("type"))
        .and_then(|v| v.as_str())
        .unwrap_or("list");

    if op == "cancel" {
        let cancel_id = req
            .get("request_id")
            .or_else(|| req.get("call_id"))
            .or_else(|| req.get("effect_id"))
            .or_else(|| req.pointer("/request/request_id"))
            .or_else(|| req.pointer("/request/call_id"))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let mut matched = false;
        if let Some(id) = cancel_id {
            if let Ok(map) = active_mcp_cancels().lock() {
                if let Some(flag) = map.get(id) {
                    flag.store(true, Ordering::SeqCst);
                    matched = true;
                }
            }
            if !matched {
                if let Ok(mut pre) = pre_cancelled_mcp_ids().lock() {
                    if pre.len() > 256 {
                        pre.clear();
                    }
                    pre.insert(id.to_string());
                }
            }
        } else if let Ok(map) = active_mcp_cancels().lock() {
            for flag in map.values() {
                flag.store(true, Ordering::SeqCst);
                matched = true;
            }
        }
        return to_owned_c_string(
            json!({
                "ok": true,
                "result": {
                    "cancelled": true,
                    "matched": matched,
                    "request_id": cancel_id
                }
            })
            .to_string(),
        );
    }

    if app.is_null() {
        return to_owned_c_string(
            json!({"ok": false, "error": {"code": "null_application", "message": "null application handle"}})
                .to_string(),
        );
    }
    let app_ref = unsafe { &*app };

    match op {
        "list" | "list_servers" => {
            let servers = app_ref
                .inner
                .store
                .list_mcp_servers()
                .unwrap_or_default();
            to_owned_c_string(json!({"ok": true, "result": {"servers": servers}}).to_string())
        }
        "list_tools" => {
            let servers = app_ref
                .inner
                .store
                .list_mcp_servers()
                .unwrap_or_default();
            let target_id = req
                .get("server_id")
                .or_else(|| req.get("id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty());
            let mut tools = Vec::new();
            let candidates: Vec<_> = match target_id {
                Some(sid) => servers.into_iter().filter(|s| s.id == sid).collect(),
                None => servers.into_iter().filter(|s| s.enabled).collect(),
            };
            if let Some(sid) = target_id {
                if candidates.is_empty() {
                    return to_owned_c_string(
                        json!({"ok": false, "error": {"code": "not_found", "message": format!("MCP server '{sid}' not found")}})
                            .to_string(),
                    );
                }
            }
            for srv in candidates {
                match app_ref.inner.mcp.connect(&srv).and_then(|c| c.list_tools()) {
                    Ok(raw_tools) => {
                        for mut t in raw_tools {
                            if let Some(obj) = t.as_object_mut() {
                                obj.insert("server_id".into(), json!(srv.id));
                            }
                            tools.push(t);
                        }
                    }
                    Err(e) if target_id.is_some() => {
                        return to_owned_c_string(
                            json!({"ok": false, "error": {"code": "mcp_error", "message": e.to_string()}})
                                .to_string(),
                        );
                    }
                    Err(_) => {}
                }
            }
            to_owned_c_string(json!({"ok": true, "result": {"tools": tools}}).to_string())
        }
        "prepare" => {
            let explicit_server_id = req
                .get("server_id")
                .or_else(|| req.get("id"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let explicit_tool_name = req
                .get("tool_name")
                .or_else(|| req.get("name"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty());

            // Single-tool prepare (direct caller)
            if let (Some(server_id), Some(tool_name)) = (explicit_server_id, explicit_tool_name) {
                let arguments = req
                    .get("arguments")
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                let servers = app_ref
                    .inner
                    .store
                    .list_mcp_servers()
                    .unwrap_or_default();
                let Some(srv) = servers.into_iter().find(|s| s.id == server_id) else {
                    return to_owned_c_string(
                        json!({"ok": false, "error": {"code": "not_found", "message": format!("MCP server '{server_id}' not found")}})
                            .to_string(),
                    );
                };
                let tool_schema = app_ref
                    .inner
                    .mcp
                    .connect(&srv)
                    .and_then(|c| c.list_tools())
                    .ok()
                    .and_then(|tools| {
                        tools.into_iter().find(|t| {
                            t.get("name").and_then(|v| v.as_str()) == Some(tool_name)
                        })
                    });
                let plan_id = new_id();
                return to_owned_c_string(
                    json!({
                        "ok": true,
                        "result": {
                            "prepared": true,
                            "plan_id": plan_id,
                            "server_id": srv.id,
                            "server_name": srv.name,
                            "transport": srv.transport,
                            "enabled": srv.enabled,
                            "tool_name": tool_name,
                            "arguments": arguments,
                            "tool_schema": tool_schema
                        }
                    })
                    .to_string(),
                );
            }

            // Multi-server catalog prepare (iOS/Android `DeviceAgentCoordinator` contract)
            let host_id = req
                .get("host_id")
                .and_then(|v| v.as_str())
                .unwrap_or("mobile-host");
            let session_id = req
                .get("session_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let policy = req.get("policy");
            let deny_tools: BTreeSet<String> = policy
                .and_then(|p| p.pointer("/user_policy/deny_tools").or_else(|| p.get("deny_tools")))
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(ToString::to_string))
                        .collect()
                })
                .unwrap_or_default();
            let allowed_server_ids: Option<BTreeSet<String>> = req
                .get("allowed_server_ids")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(ToString::to_string))
                        .collect()
                });

            let servers = app_ref
                .inner
                .store
                .list_mcp_servers()
                .unwrap_or_default();
            let enabled_servers: Vec<_> = servers
                .into_iter()
                .filter(|s| {
                    s.enabled
                        && allowed_server_ids
                            .as_ref()
                            .is_none_or(|ids| ids.contains(&s.id))
                })
                .collect();

            let mut tools = Vec::new();
            let mut capabilities = serde_json::Map::new();
            let mut permission_rules = Vec::new();
            let mut server_summaries = Vec::new();

            for srv in &enabled_servers {
                let raw_tools = match app_ref.inner.mcp.connect(srv).and_then(|c| c.list_tools()) {
                    Ok(list) => {
                        let _ = app_ref
                            .inner
                            .store
                            .set_kv(&format!("mcp_tools_cache:{}", srv.id), &Value::Array(list.clone()));
                        list
                    }
                    Err(_) => app_ref
                        .inner
                        .store
                        .get_kv(&format!("mcp_tools_cache:{}", srv.id))
                        .ok()
                        .flatten()
                        .and_then(|v| v.as_array().cloned())
                        .unwrap_or_default(),
                };
                server_summaries.push(json!({
                    "id": srv.id,
                    "name": srv.name,
                    "transport": srv.transport,
                    "enabled": srv.enabled,
                    "tool_count": raw_tools.len()
                }));
                for t in raw_tools {
                    let raw_name = t
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .trim();
                    if raw_name.is_empty() {
                        continue;
                    }
                    let qualified = format!("mcp__{}__{}", srv.id, raw_name);
                    if deny_tools.contains(&qualified) || deny_tools.contains(raw_name) {
                        continue;
                    }
                    let desc = t
                        .get("description")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let params = t
                        .get("inputSchema")
                        .or_else(|| t.get("input_schema"))
                        .cloned()
                        .unwrap_or_else(|| json!({"type": "object", "properties": {}}));
                    tools.push(json!({
                        "type": "function",
                        "function": {
                            "name": qualified,
                            "description": desc,
                            "parameters": params
                        },
                        "server_id": srv.id,
                        "tool_name": raw_name
                    }));
                    capabilities.insert(
                        qualified.clone(),
                        json!({
                            "capability_id": qualified,
                            "server_id": srv.id,
                            "server_name": srv.name,
                            "tool_name": raw_name,
                            "route": "device",
                            "host_id": host_id,
                            "permission": "agent_approval",
                            "risk": "external_write"
                        }),
                    );
                    permission_rules.push(json!({
                        "tool_id": qualified,
                        "server_id": srv.id,
                        "effect": "ask"
                    }));
                }
            }

            to_owned_c_string(
                json!({
                    "ok": true,
                    "result": {
                        "prepared": true,
                        "host_id": host_id,
                        "session_id": session_id,
                        "servers": server_summaries,
                        "tools": tools,
                        "capabilities": Value::Object(capabilities),
                        "permission_rules": permission_rules
                    }
                })
                .to_string(),
            )
        }
        "call_tool" | "execute" => {
            let nested_req = req.get("request").filter(|v| v.is_object());
            let nested_payload = nested_req.and_then(|r| r.get("payload")).filter(|v| v.is_object());

            let cap_id = nested_payload
                .and_then(|p| p.get("capability_id"))
                .or_else(|| nested_req.and_then(|r| r.get("capability_id")))
                .or_else(|| req.get("capability_id"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty());

            let parsed_cap = cap_id.and_then(split_qualified_mcp_tool);

            let server_id = nested_req
                .and_then(|r| r.get("server_id").or_else(|| r.get("id")))
                .or_else(|| nested_payload.and_then(|p| p.get("server_id")))
                .or_else(|| req.get("server_id").or_else(|| req.get("id")))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .or_else(|| parsed_cap.map(|(s, _)| s));

            let tool_name = nested_req
                .and_then(|r| r.get("tool_name").or_else(|| r.get("name")))
                .or_else(|| nested_payload.and_then(|p| p.get("tool_name").or_else(|| p.get("name"))))
                .or_else(|| req.get("tool_name").or_else(|| req.get("name")))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .or_else(|| parsed_cap.map(|(_, t)| t));

            let Some(server_id) = server_id else {
                return to_owned_c_string(
                    json!({"ok": false, "error": {"code": "invalid_argument", "message": "missing 'server_id'"}})
                        .to_string(),
                );
            };
            let Some(tool_name) = tool_name else {
                return to_owned_c_string(
                    json!({"ok": false, "error": {"code": "invalid_argument", "message": "missing 'tool_name'"}})
                        .to_string(),
                );
            };
            let arguments = nested_req
                .and_then(|r| r.get("arguments"))
                .or_else(|| nested_payload.and_then(|p| p.get("arguments")))
                .or_else(|| req.get("arguments"))
                .cloned()
                .unwrap_or_else(|| json!({}));

            let mut cancel_keys: Vec<String> = Vec::new();
            for candidate in [
                nested_req.and_then(|r| r.get("request_id")).and_then(|v| v.as_str()),
                nested_req.and_then(|r| r.get("call_id")).and_then(|v| v.as_str()),
                nested_req.and_then(|r| r.get("effect_id")).and_then(|v| v.as_str()),
                nested_payload.and_then(|p| p.get("call_id")).and_then(|v| v.as_str()),
                req.get("request_id").and_then(|v| v.as_str()),
                req.get("call_id").and_then(|v| v.as_str()),
            ]
            .into_iter()
            .flatten()
            {
                let trimmed = candidate.trim();
                if !trimmed.is_empty() && !cancel_keys.iter().any(|k| k == trimmed) {
                    cancel_keys.push(trimmed.to_string());
                }
            }
            if cancel_keys.is_empty() {
                cancel_keys.push(new_id());
            }

            let cancel_flag = Arc::new(AtomicBool::new(false));
            if let Ok(mut pre) = pre_cancelled_mcp_ids().lock() {
                for key in &cancel_keys {
                    if pre.remove(key) {
                        cancel_flag.store(true, Ordering::SeqCst);
                    }
                }
            }
            if let Ok(mut map) = active_mcp_cancels().lock() {
                for key in &cancel_keys {
                    map.insert(key.clone(), Arc::clone(&cancel_flag));
                }
            }
            let _cancel_guard = McpCancelGuard { keys: cancel_keys };

            if cancel_flag.load(Ordering::SeqCst) {
                return to_owned_c_string(
                    json!({
                        "ok": false,
                        "error": {
                            "code": "cancelled",
                            "message": "MCP tool call was cancelled before execution"
                        }
                    })
                    .to_string(),
                );
            }

            let servers = app_ref
                .inner
                .store
                .list_mcp_servers()
                .unwrap_or_default();
            let Some(srv) = servers.into_iter().find(|s| s.id == server_id) else {
                return to_owned_c_string(
                    json!({"ok": false, "error": {"code": "not_found", "message": format!("MCP server '{server_id}' not found")}})
                        .to_string(),
                );
            };
            if let Err(e) = app_ref.inner.mcp.connect(&srv) {
                return to_owned_c_string(
                    json!({"ok": false, "error": {"code": "mcp_connect_error", "message": e.to_string()}})
                        .to_string(),
                );
            }
            match app_ref.inner.mcp.call_tool_with_options(
                &srv.id,
                tool_name,
                arguments,
                Some(cancel_flag),
                None,
            ) {
                Ok(res) => {
                    if nested_req.is_some() {
                        to_owned_c_string(
                            json!({
                                "ok": true,
                                "result": {
                                    "output": {
                                        "execution": {"state": "completed"},
                                        "result": res
                                    },
                                    "server_id": srv.id,
                                    "tool_name": tool_name
                                }
                            })
                            .to_string(),
                        )
                    } else {
                        to_owned_c_string(json!({"ok": true, "result": res}).to_string())
                    }
                }
                Err(McpError::Cancelled(msg)) => to_owned_c_string(
                    json!({"ok": false, "error": {"code": "cancelled", "message": msg}})
                        .to_string(),
                ),
                Err(e) => to_owned_c_string(
                    json!({"ok": false, "error": {"code": "mcp_call_error", "message": e.to_string()}})
                        .to_string(),
                ),
            }
        }
        "disconnect" => {
            if let Some(sid) = req
                .get("server_id")
                .or_else(|| req.get("id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
            {
                app_ref.inner.mcp.disconnect(sid);
                return to_owned_c_string(
                    json!({"ok": true, "result": {"disconnected": true, "server_id": sid}})
                        .to_string(),
                );
            }
            let servers = app_ref
                .inner
                .store
                .list_mcp_servers()
                .unwrap_or_default();
            for srv in &servers {
                app_ref.inner.mcp.disconnect(&srv.id);
            }
            to_owned_c_string(
                json!({"ok": true, "result": {"disconnected": true, "count": servers.len()}})
                    .to_string(),
            )
        }
        other => to_owned_c_string(
            json!({"ok": false, "error": {"code": "unsupported_operation", "message": format!("unsupported mobile MCP operation '{other}'")}})
                .to_string(),
        ),
    }
}

/// Prepare a GitHub skill package descriptor under a mobile skills root directory.
/// Matches `IOSGitHubSkillPackages.swift` (`{"ok": true, "value": ...}`).
///
/// # Safety
/// `root_path` and `request_json` must be valid NUL-terminated C strings.
#[no_mangle]
pub unsafe extern "C" fn nola_mobile_github_skill_package_json(
    root_path: *const c_char,
    request_json: *const c_char,
) -> *mut c_char {
    if root_path.is_null() || request_json.is_null() {
        return to_owned_c_string(json!({"ok": false, "error": "null argument"}).to_string());
    }
    let Ok(root_str) = unsafe { CStr::from_ptr(root_path) }.to_str() else {
        return to_owned_c_string(json!({"ok": false, "error": "invalid root path"}).to_string());
    };
    let Ok(req_str) = unsafe { CStr::from_ptr(request_json) }.to_str() else {
        return to_owned_c_string(json!({"ok": false, "error": "invalid request UTF-8"}).to_string());
    };
    let req: Value = match serde_json::from_str(req_str) {
        Ok(v) => v,
        Err(e) => {
            return to_owned_c_string(
                json!({"ok": false, "error": format!("invalid request JSON: {e}")}).to_string(),
            );
        }
    };
    let skills_dir = req
        .get("skills_dir")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(root_str));
    let operation = req
        .get("operation")
        .and_then(|v| v.as_str())
        .unwrap_or("prepare");
    let query_or_url = req
        .get("url")
        .or_else(|| req.get("query"))
        .or_else(|| req.get("repo"))
        .or_else(|| req.get("spec"))
        .or_else(|| req.get("id"))
        .or_else(|| req.get("name"))
        .or_else(|| req.get("instructions_ref"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let format_prepared = |pkg: &Value| -> Value {
        let name = pkg
            .get("name")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .unwrap_or("skill");
        let id = pkg
            .get("id")
            .and_then(|v| v.as_str())
            .map(ToString::to_string)
            .unwrap_or_else(|| {
                match (
                    pkg.get("owner").and_then(|v| v.as_str()),
                    pkg.get("repo").and_then(|v| v.as_str()),
                ) {
                    (Some(o), Some(r)) => format!("{o}/{r}"),
                    _ => name.to_string(),
                }
            });
        let dest_file = pkg
            .get("dest_file")
            .and_then(|v| v.as_str())
            .map(PathBuf::from);
        let content = dest_file
            .as_deref()
            .filter(|p| {
                std::fs::symlink_metadata(p)
                    .map(|m| !m.file_type().is_symlink() && m.is_file())
                    .unwrap_or(false)
            })
            .and_then(|p| std::fs::read_to_string(p).ok())
            .unwrap_or_default();
        let description = pkg
            .get("description")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(ToString::to_string)
            .unwrap_or_else(|| format!("GitHub skill package {name}"));
        let mut out = pkg.clone();
        if let Some(obj) = out.as_object_mut() {
            obj.insert("status".into(), json!("prepared"));
            obj.insert("id".into(), json!(id));
            obj.insert("name".into(), json!(name));
            obj.insert("description".into(), json!(description));
            obj.insert("version".into(), json!("1.0.0"));
            obj.insert(
                "instructions_ref".into(),
                json!(format!("nola://github-skills/{name}")),
            );
            obj.insert("content".into(), json!(content));
        }
        out
    };

    match operation {
        "prepare" | "resolve" => {
            match nola_modules::skills::prepare_github_skill_package(query_or_url, &skills_dir) {
                Ok(pkg) => {
                    let val = format_prepared(&pkg);
                    to_owned_c_string(json!({"ok": true, "value": val}).to_string())
                }
                Err(err) => to_owned_c_string(json!({"ok": false, "error": err}).to_string()),
            }
        }
        "find" => {
            let q = query_or_url.trim().to_ascii_lowercase();
            let mut entries: Vec<Value> =
                nola_modules::skills::discover_skills(&skills_dir, Some(&skills_dir))
                    .into_iter()
                    .filter(|s| {
                        q.is_empty()
                            || s.name.to_ascii_lowercase().contains(&q)
                            || s.description.to_ascii_lowercase().contains(&q)
                            || s.body.to_ascii_lowercase().contains(&q)
                    })
                    .map(|s| {
                        json!({
                            "status": "prepared",
                            "id": s.name,
                            "name": s.name,
                            "description": s.description,
                            "version": "1.0.0",
                            "instructions_ref": format!("nola://github-skills/{}", s.name),
                            "content": s.body
                        })
                    })
                    .collect();
            if entries.is_empty() && !query_or_url.trim().is_empty() {
                if let Ok(pkg) =
                    nola_modules::skills::prepare_github_skill_package(query_or_url, &skills_dir)
                {
                    entries.push(format_prepared(&pkg));
                }
            }
            let first = entries.first().cloned().unwrap_or_else(|| {
                let fallback_name = query_or_url.trim();
                json!({
                    "status": "prepared",
                    "id": fallback_name,
                    "name": fallback_name,
                    "description": "",
                    "version": "1.0.0",
                    "instructions_ref": format!("nola://github-skills/{fallback_name}"),
                    "content": ""
                })
            });
            let mut val = first;
            if let Some(obj) = val.as_object_mut() {
                obj.insert("items".into(), json!(entries));
                obj.insert("packages".into(), json!(entries));
            }
            to_owned_c_string(json!({"ok": true, "value": val}).to_string())
        }
        "read" => {
            let raw_name = query_or_url
                .trim()
                .strip_prefix("nola://github-skills/")
                .unwrap_or(query_or_url.trim());
            if raw_name.is_empty() {
                return to_owned_c_string(
                    json!({"ok": false, "error": "missing skill id, name, or instructions_ref"})
                        .to_string(),
                );
            }
            let discovered = nola_modules::skills::discover_skills(&skills_dir, Some(&skills_dir));
            if let Some(found) = discovered
                .into_iter()
                .find(|s| s.name.eq_ignore_ascii_case(raw_name))
            {
                let val = json!({
                    "status": "prepared",
                    "id": found.name,
                    "name": found.name,
                    "description": found.description,
                    "version": "1.0.0",
                    "instructions_ref": format!("nola://github-skills/{}", found.name),
                    "content": found.body
                });
                return to_owned_c_string(json!({"ok": true, "value": val}).to_string());
            }
            if let Ok(pkg) =
                nola_modules::skills::prepare_github_skill_package(query_or_url, &skills_dir)
            {
                let val = format_prepared(&pkg);
                return to_owned_c_string(json!({"ok": true, "value": val}).to_string());
            }
            let safe_name: String = raw_name
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
                .collect();
            let candidate = skills_dir.join(&safe_name).join("SKILL.md");
            let content = if !safe_name.is_empty()
                && std::fs::symlink_metadata(&candidate)
                    .map(|m| !m.file_type().is_symlink() && m.is_file())
                    .unwrap_or(false)
            {
                std::fs::read_to_string(&candidate).unwrap_or_default()
            } else {
                String::new()
            };
            let name = if safe_name.is_empty() {
                raw_name.to_string()
            } else {
                safe_name
            };
            let val = json!({
                "status": "prepared",
                "id": name,
                "name": name,
                "description": format!("GitHub skill package {name}"),
                "version": "1.0.0",
                "instructions_ref": format!("nola://github-skills/{name}"),
                "content": content
            });
            to_owned_c_string(json!({"ok": true, "value": val}).to_string())
        }
        other => to_owned_c_string(
            json!({"ok": false, "error": format!("unsupported github skill operation '{other}'")})
                .to_string(),
        ),
    }
}

/// Redact high-confidence secrets/tokens (and an optional active credential) in a JSON payload
/// before cloud egress without breaking JSON structure.
///
/// # Safety
/// `input` must point to `size` readable UTF-8 bytes; if `credential_len > 0`, `credential` must
/// point to `credential_len` readable UTF-8 bytes.
#[no_mangle]
pub unsafe extern "C" fn nola_core_redact_model_request_json(
    input: *const u8,
    size: usize,
    credential: *const u8,
    credential_len: usize,
) -> *mut c_char {
    if input.is_null() || size == 0 {
        return to_owned_c_string(String::new());
    }
    let slice = unsafe { std::slice::from_raw_parts(input, size) };
    let cred_str = if !credential.is_null() && credential_len > 0 {
        let cred_slice = unsafe { std::slice::from_raw_parts(credential, credential_len) };
        let trimmed = String::from_utf8_lossy(cred_slice).trim().to_string();
        (trimmed.len() >= 6).then_some(trimmed)
    } else {
        None
    };

    let extra_secrets: Vec<&str> = cred_str.as_deref().into_iter().collect();

    if let Ok(mut json_val) = serde_json::from_slice::<Value>(slice) {
        PrivacyRedactor::default().redact_json_value_for_endpoint(
            &mut json_val,
            "https://remote.invalid",
            &extra_secrets,
        );
        return to_owned_c_string(json_val.to_string());
    }

    let text = String::from_utf8_lossy(slice);
    let (out, _) = PrivacyRedactor::default().redact_text_for_endpoint(
        &text,
        "https://remote.invalid",
        &extra_secrets,
    );
    to_owned_c_string(out)
}

/// Check a signed software update feed manifest for Apple/Android/Desktop hosts.
/// Always emits `{ok, value, error: {code, message}}` matching `AppleSoftwareUpdateClient`.
///
/// # Safety
/// `input` must point to `size` readable UTF-8 bytes.
#[no_mangle]
pub unsafe extern "C" fn nola_core_software_update_json(
    input: *const u8,
    size: usize,
) -> *mut c_char {
    let err_json = |code: &str, msg: String| {
        to_owned_c_string(
            json!({
                "ok": false,
                "value": Value::Null,
                "error": {"code": code, "message": msg}
            })
            .to_string(),
        )
    };
    if input.is_null() || size == 0 {
        return err_json("invalid_request", "empty request".into());
    }
    let slice = unsafe { std::slice::from_raw_parts(input, size) };
    let req: Value = match serde_json::from_slice(slice) {
        Ok(v) => v,
        Err(e) => return err_json("invalid_json", format!("invalid json: {e}")),
    };
    let req_target = req
        .get("target")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty());
    let channel = req
        .get("channel")
        .and_then(|v| v.as_str())
        .unwrap_or("stable");
    let current_version = req
        .get("current_version")
        .and_then(|v| v.as_str())
        .unwrap_or("2.0.0");
    let current_build = req
        .get("current_build")
        .and_then(|v| v.as_u64())
        .unwrap_or(200);
    let manifest_url = req
        .get("manifest_url")
        .or_else(|| req.get("feed_url"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let operation = req
        .get("operation")
        .and_then(|v| v.as_str())
        .unwrap_or("status");

    if operation == "download" {
        let Some(url) = req
            .get("url")
            .or_else(|| req.get("download_url"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
        else {
            return err_json("invalid_request", "missing download 'url'".into());
        };
        let dest_dir = req
            .get("dest_dir")
            .or_else(|| req.get("destination_dir"))
            .or_else(|| req.get("directory"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let expected_sha256 = req
            .get("sha256")
            .or_else(|| req.get("expected_sha256"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty());
        let expected_size = req
            .get("size")
            .or_else(|| req.get("expected_size"))
            .and_then(|v| v.as_u64());
        let dl_res = if let Some(sz) = expected_size {
            match expected_sha256 {
                Some(sha) => download_update_package_with_size(url, sha, Some(sz), Path::new(&dest_dir)),
                None => Err("expected_sha256 is required".into()),
            }
        } else {
            download_update_package(url, expected_sha256, Path::new(&dest_dir))
        };
        return match dl_res {
            Ok(info) => to_owned_c_string(
                json!({
                    "ok": true,
                    "value": {
                        "status": "downloaded",
                        "path": info["path"],
                        "sha256": info["sha256"],
                        "bytes": info["bytes"],
                        "target": info["target"]
                    },
                    "error": Value::Null
                })
                .to_string(),
            ),
            Err(err) => err_json("update_download_failed", err),
        };
    }

    let effective_target_pref = std::env::var("NOLA_UPDATE_TARGET")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| req_target.map(ToString::to_string))
        .unwrap_or_else(nola_modules::current_update_target);

    // Inline Ed25519-verified signed feed evaluation (for offline/bundled checks & testing).
    if let (Some(feed_bytes), Some(pub_key)) = (
        req.get("signed_feed_json")
            .or_else(|| req.get("feed_json"))
            .and_then(|v| {
                v.as_str()
                    .map(|s| s.as_bytes().to_vec())
                    .or_else(|| serde_json::to_vec(v).ok())
            }),
        req.get("public_key")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty()),
    ) {
        let now_secs = req
            .get("now_secs")
            .and_then(|v| v.as_u64())
            .unwrap_or_else(|| (now_ms().max(0) as u64) / 1000);
        let store_schema = req
            .get("store_schema")
            .and_then(|v| v.as_i64())
            .unwrap_or(CURRENT_STORE_SCHEMA);
        let store_epoch = req
            .get("store_epoch")
            .and_then(|v| v.as_i64())
            .unwrap_or(CURRENT_STORE_EPOCH);
        return match evaluate_signed_feed(
            &feed_bytes,
            pub_key,
            Some(channel),
            current_version,
            current_build,
            store_schema,
            store_epoch,
            &effective_target_pref,
            now_secs,
        ) {
            Ok(update) => format_update_check_response(
                update,
                current_version,
                current_build,
                channel,
                &effective_target_pref,
            ),
            Err(err) => err_json("update_verification_failed", err),
        };
    }

    if operation != "check" && manifest_url.is_empty() {
        return to_owned_c_string(
            json!({
                "ok": true,
                "value": {
                    "status": "idle",
                    "current_version": current_version,
                    "current_build": current_build,
                    "channel": channel,
                    "target": effective_target_pref,
                    "release": Value::Null
                },
                "error": Value::Null
            })
            .to_string(),
        );
    }

    match check_for_update(manifest_url, current_version) {
        Ok(update) => {
            let effective_target = if std::env::var("NOLA_UPDATE_TARGET")
                .ok()
                .is_some_and(|s| !s.trim().is_empty())
            {
                update.target.clone()
            } else {
                req_target
                    .map(ToString::to_string)
                    .unwrap_or_else(|| update.target.clone())
            };
            format_update_check_response(
                update,
                current_version,
                current_build,
                channel,
                &effective_target,
            )
        }
        Err(err) => err_json("update_check_failed", err),
    }
}

fn format_update_check_response(
    update: nola_modules::UpdateCheckResult,
    current_version: &str,
    current_build: u64,
    channel: &str,
    effective_target: &str,
) -> *mut c_char {
    let artifact_kind = if effective_target.eq_ignore_ascii_case("ios") {
        "store"
    } else {
        "archive"
    };
    let release = match (update.update_available, update.latest) {
        (true, Some(rel)) => json!({
            "release_id": rel.release_id.unwrap_or_else(|| rel.version.clone()),
            "version": rel.version,
            "build": rel.build.unwrap_or(current_build + 1),
            "notes": rel.notes,
            "signature": rel.signature,
            "artifact": {
                "kind": artifact_kind,
                "url": rel.download_url,
                "filename": rel.filename,
                "size": rel.size,
                "sha256": rel.sha256
            }
        }),
        _ => Value::Null,
    };
    to_owned_c_string(
        json!({
            "ok": true,
            "value": {
                "status": update.status,
                "current_version": current_version,
                "current_build": current_build,
                "channel": channel,
                "target": effective_target,
                "release": release
            },
            "error": Value::Null
        })
        .to_string(),
    )
}

/// Safe UTF-8 token estimator exported across the C-ABI boundary.
///
/// # Safety
/// `text` must be either null or a valid NUL-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn nola_v2_estimate_tokens(text: *const c_char) -> usize {
    if text.is_null() {
        return 0;
    }
    match unsafe { CStr::from_ptr(text) }.to_str() {
        Ok(s) => estimate_text_tokens(s),
        Err(_) => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    #[test]
    fn ffi_open_execute_redact_and_mobile_roundtrip() {
        assert_eq!(nola_core_application_abi_version(), 1);
        assert_eq!(nola_core_application_service_protocol_version(), 1);
        assert_eq!(nola_core_context_engine_abi_version(), 1);
        assert_eq!(nola_core_prompt_composer_abi_version(), 1);
        let sample = CString::new("你好hello").unwrap();
        assert_eq!(unsafe { nola_v2_estimate_tokens(sample.as_ptr()) }, 4);

        // Open error reporting (borrowed const char* - MUST NOT be freed!)
        let bad = unsafe { nola_core_application_open(std::ptr::null(), 0) };
        assert!(bad.is_null());
        let err_ptr = nola_core_application_last_open_error();
        assert!(!err_ptr.is_null());
        let err_str = unsafe { CStr::from_ptr(err_ptr) }.to_str().unwrap().to_string();
        assert!(err_str.contains("empty database path"));

        let dir = tempfile::tempdir().unwrap();
        let db_str = dir.path().join("ffi.db").display().to_string();
        let app = unsafe { nola_core_application_open(db_str.as_ptr(), db_str.len()) };
        assert!(!app.is_null());

        // 1. Mobile submit envelope matches strict Android/iOS receipt validation
        let submit_req = json!({
            "protocol_version": 1,
            "client_id": "test-mobile",
            "request_id": "req-1",
            "command": {"type": "session_create", "arguments": {"title": "Mobile Chat"}}
        })
        .to_string();
        let submit_ptr =
            unsafe { nola_core_application_submit(app, submit_req.as_ptr(), submit_req.len()) };
        let submit_val: Value = serde_json::from_str(
            unsafe { CStr::from_ptr(submit_ptr) }.to_str().unwrap(),
        )
        .unwrap();
        unsafe { nola_core_free_c_string(submit_ptr) };
        assert_eq!(submit_val["ok"], true);
        let receipt = &submit_val["result"];
        assert_eq!(receipt["status"], "committed");
        assert!(receipt["received_at"].as_str().is_some());
        assert!(receipt["completed_at"].as_str().is_some());
        assert!(receipt["committed_revision"].as_u64().unwrap_or(0) >= 1);

        // 2. Resource access MUST reject cross-resource grants
        let ra_req = json!({
            "resource": {
                "kind": "skill",
                "resource_id": "secret-skill",
                "owner": {"kind": "local_user", "id": "alice"},
                "revision": 2
            },
            "actor": {"kind": "local_user", "id": "bob"},
            "permission": "read_content",
            "now_epoch_ms": 1000,
            "grants": [{
                "grant_id": "g1",
                "resource_kind": "skill",
                "resource_id": "other-skill",
                "principal": {"kind": "local_user", "id": "bob"},
                "permissions": ["read_content"]
            }]
        })
        .to_string();
        let ra_ptr = unsafe { nola_core_resource_access_evaluate_json(ra_req.as_ptr(), ra_req.len()) };
        let ra_val: Value =
            serde_json::from_str(unsafe { CStr::from_ptr(ra_ptr) }.to_str().unwrap()).unwrap();
        unsafe { nola_core_free_c_string(ra_ptr) };
        assert_eq!(ra_val["decision"]["allowed"], false);

        // 3. Tool policy matches tool_policy_v1_golden.json contract + MCP fail-closed
        let tp_req = json!({
            "schema_version": 1,
            "platform": "ios",
            "now_ms": 1000000,
            "runtime": {
                "expires_at_ms": 1060000,
                "authorizations": {"files_read": "granted", "files_write": "granted"},
                "tools": [
                    {"tool_id": "device_read_file", "capability": "files_read", "risk": "read_only", "route": "device", "available": true, "schema": {}},
                    {"tool_id": "device_write_file", "capability": "files_write", "risk": "local_write", "route": "device", "available": true, "schema": {}}
                ]
            },
            "user_policy": {"max_risk": "read_only", "allow_result_egress_routes": ["device"]}
        })
        .to_string();
        let tp_ptr = unsafe { nola_core_tool_policy_resolve(tp_req.as_ptr(), tp_req.len()) };
        let tp_val: Value =
            serde_json::from_str(unsafe { CStr::from_ptr(tp_ptr) }.to_str().unwrap()).unwrap();
        unsafe { nola_core_free_c_string(tp_ptr) };
        assert_eq!(tp_val["platform"], "ios");
        assert_eq!(tp_val["enabled_tools"].as_array().unwrap().len(), 1);
        assert_eq!(tp_val["enabled_tools"][0]["tool_id"], "device_read_file");
        assert_eq!(tp_val["disabled_tools"][0]["tool_id"], "device_write_file");
        assert_eq!(tp_val["disabled_tools"][0]["reason"], "risk_exceeds_user_policy");
        assert_eq!(tp_val["policy_digest"].as_str().unwrap().len(), 64);
        let mcp_empty = b"{}";
        assert!(!unsafe { nola_core_tool_policy_mcp_allowed(mcp_empty.as_ptr(), mcp_empty.len()) });

        // 4. Memory select emits exact iOS enum reject reasons
        let ms_req = json!({
            "mode": "on_demand",
            "policy": {"long_term_enabled": true, "minimum_confidence": 0.8, "search_limit": 1},
            "records": [
                {"id": "low", "content": "a", "confidence": 0.2},
                {"id": "ok1", "content": "b", "confidence": 0.9},
                {"id": "over", "content": "c", "confidence": 0.95}
            ]
        })
        .to_string();
        let ms_ptr = unsafe { nola_core_memory_select_json(ms_req.as_ptr(), ms_req.len()) };
        let ms_val: Value =
            serde_json::from_str(unsafe { CStr::from_ptr(ms_ptr) }.to_str().unwrap()).unwrap();
        unsafe { nola_core_free_c_string(ms_ptr) };
        assert_eq!(ms_val["rejected"][0]["reason"], "below_confidence");
        assert_eq!(ms_val["rejected"][1]["reason"], "limit_exceeded");

        // 5. Model output parser extracts <think> & <tool_call> and filters disallowed tools
        let mo_req = json!({
            "content": "<think>step 1</think>Answer<tool_call>{\"name\":\"read_file\",\"arguments\":{\"path\":\"a\"}}</tool_call>",
            "reasoning": "",
            "native_tool_calls": [],
            "allowed_tool_names": ["read_file"]
        })
        .to_string();
        let mo_ptr = unsafe { nola_core_parse_agent_model_output(mo_req.as_ptr(), mo_req.len()) };
        let mo_val: Value =
            serde_json::from_str(unsafe { CStr::from_ptr(mo_ptr) }.to_str().unwrap()).unwrap();
        unsafe { nola_core_free_c_string(mo_ptr) };
        assert_eq!(mo_val["reasoning"], "step 1");
        assert_eq!(mo_val["content"], "Answer");
        assert_eq!(mo_val["tool_calls"].as_array().unwrap().len(), 1);
        assert_eq!(mo_val["finish_reason"], "tool_calls");

        // 6. Context prepare compacts when exceeding prompt_token_limit
        let big = "word ".repeat(300);
        let cp_req = json!({
            "messages": [
                {"role": "system", "content": "sys"},
                {"role": "user", "content": big},
                {"role": "assistant", "content": big},
                {"role": "user", "content": "latest"}
            ],
            "tools": [],
            "budget": {"prompt_token_limit": 80, "keep_recent_messages": 2}
        })
        .to_string();
        let cp_ptr = unsafe { nola_core_context_prepare(cp_req.as_ptr(), cp_req.len()) };
        let cp_val: Value =
            serde_json::from_str(unsafe { CStr::from_ptr(cp_ptr) }.to_str().unwrap()).unwrap();
        unsafe { nola_core_free_c_string(cp_ptr) };
        assert_eq!(cp_val["changed"], true);
        assert_eq!(cp_val["action"], "compacted");
        assert!(cp_val["metrics"]["archive_message_count"].as_u64().unwrap() >= 1);

        // 7. Personal intelligence ingest/recall/snapshot contract
        let ing_req = json!({
            "events": [{"id": "e1", "source": "chat", "kind": "note", "summary": "User prefers Rust"}],
            "memory_candidates": [{"semantic_key": "lang", "kind": "semantic", "statement": "Prefers Rust v2"}],
            "outcomes": [{"id": "o1", "task_type": "code", "route": "local", "model": "m1", "success": true}]
        })
        .to_string();
        let ing_ptr = unsafe {
            nola_core_application_intelligence_ingest(app, ing_req.as_ptr(), ing_req.len())
        };
        let ing_val: Value =
            serde_json::from_str(unsafe { CStr::from_ptr(ing_ptr) }.to_str().unwrap()).unwrap();
        unsafe { nola_core_free_c_string(ing_ptr) };
        assert_eq!(ing_val["ingested_memories"], 1);
        let rec_req = json!({"query": "Rust", "limit": 5}).to_string();
        let rec_ptr = unsafe {
            nola_core_application_intelligence_recall(app, rec_req.as_ptr(), rec_req.len())
        };
        let rec_val: Value =
            serde_json::from_str(unsafe { CStr::from_ptr(rec_ptr) }.to_str().unwrap()).unwrap();
        unsafe { nola_core_free_c_string(rec_ptr) };
        assert!(rec_val["context"].as_str().unwrap().contains("Rust"));
        let snap_ptr = unsafe { nola_core_application_intelligence_snapshot(app) };
        let snap_val: Value =
            serde_json::from_str(unsafe { CStr::from_ptr(snap_ptr) }.to_str().unwrap()).unwrap();
        unsafe { nola_core_free_c_string(snap_ptr) };
        assert_eq!(snap_val["schema_version"], 1);
        assert_eq!(snap_val["events"].as_array().unwrap().len(), 1);
        assert_eq!(snap_val["memories"].as_array().unwrap().len(), 1);
        assert_eq!(snap_val["outcomes"].as_array().unwrap().len(), 1);

        // 8. Software update contract matches AppleSoftwareUpdateClient (truthful unconfigured!)
        let su_req = json!({"operation": "check", "target": "ios"}).to_string();
        let su_ptr = unsafe { nola_core_software_update_json(su_req.as_ptr(), su_req.len()) };
        let su_val: Value =
            serde_json::from_str(unsafe { CStr::from_ptr(su_ptr) }.to_str().unwrap()).unwrap();
        unsafe { nola_core_free_c_string(su_ptr) };
        assert_eq!(su_val["ok"], true);
        assert_eq!(su_val["value"]["status"], "unconfigured");
        assert_eq!(su_val["value"]["target"], "ios");
        assert!(nola_core_application_last_open_error().is_null());

        // 9. Nested macOS MCP policy check, mobile MCP cancel with null app, attest result envelope,
        //    child pump result envelope, stateless intelligence recall, and GitHub skill package
        let mac_mcp = json!({
            "policy": {"enabled": true, "denied": false, "authorization": "granted"},
            "descriptor": {"server_id": "docs", "tool_name": "search_docs"}
        })
        .to_string();
        assert!(unsafe { nola_core_tool_policy_mcp_allowed(mac_mcp.as_ptr(), mac_mcp.len()) });

        let cancel_req = json!({"type": "cancel"}).to_string();
        let cancel_ptr = unsafe {
            nola_core_mobile_mcp_host(std::ptr::null_mut(), cancel_req.as_ptr(), cancel_req.len())
        };
        let cancel_val: Value =
            serde_json::from_str(unsafe { CStr::from_ptr(cancel_ptr) }.to_str().unwrap()).unwrap();
        unsafe { nola_core_free_c_string(cancel_ptr) };
        assert_eq!(cancel_val["ok"], true);
        assert_eq!(cancel_val["result"]["cancelled"], true);

        let created_conv_id = submit_val["result"]["result"]["session"]["id"]
            .as_str()
            .or_else(|| submit_val["result"]["result"]["conversation"]["id"].as_str())
            .unwrap()
            .to_string();
        let target_bytes = created_conv_id.as_bytes();
        let cfg_str = json!({
            "core_config": {"conversation_id": created_conv_id, "model": "gpt-4o"},
            "submission": {"conversation_id": created_conv_id, "type": "user_input", "text": "retry"}
        })
        .to_string();
        let cfg_bytes = cfg_str.as_bytes();
        let att_ptr = unsafe {
            nola_core_application_attest_resend_host_policy(
                app,
                target_bytes.as_ptr(),
                target_bytes.len(),
                cfg_bytes.as_ptr(),
                cfg_bytes.len(),
            )
        };
        let att_val: Value =
            serde_json::from_str(unsafe { CStr::from_ptr(att_ptr) }.to_str().unwrap()).unwrap();
        unsafe { nola_core_free_c_string(att_ptr) };
        assert_eq!(att_val["ok"], true);
        assert_eq!(att_val["result"]["attested"], true);
        assert_eq!(att_val["result"]["policy_digest"].as_str().unwrap().len(), 64);

        // Missing conversation MUST be rejected by attest_resend_host_policy
        let bad_target = b"missing-conv-999";
        let bad_att_ptr = unsafe {
            nola_core_application_attest_resend_host_policy(
                app,
                bad_target.as_ptr(),
                bad_target.len(),
                cfg_bytes.as_ptr(),
                cfg_bytes.len(),
            )
        };
        let bad_att_val: Value =
            serde_json::from_str(unsafe { CStr::from_ptr(bad_att_ptr) }.to_str().unwrap()).unwrap();
        unsafe { nola_core_free_c_string(bad_att_ptr) };
        assert_eq!(bad_att_val["ok"], false);
        assert_eq!(bad_att_val["error"]["code"], "resend_host_policy_rejected");

        let pump = unsafe {
            nola_core_child_pump_start(app, std::ptr::null(), 0, std::ptr::null_mut())
        };
        assert!(!pump.is_null());
        let poll_ptr = unsafe { nola_core_child_pump_poll(pump, 10) };
        let poll_val: Value =
            serde_json::from_str(unsafe { CStr::from_ptr(poll_ptr) }.to_str().unwrap()).unwrap();
        unsafe { nola_core_free_c_string(poll_ptr) };
        assert_eq!(poll_val["ok"], true);
        assert_eq!(poll_val["result"]["kind"], "idle");
        let resp_ptr = unsafe { nola_core_child_pump_respond(pump, b"{}".as_ptr(), 2) };
        let resp_val: Value =
            serde_json::from_str(unsafe { CStr::from_ptr(resp_ptr) }.to_str().unwrap()).unwrap();
        unsafe { nola_core_free_c_string(resp_ptr) };
        assert_eq!(resp_val["ok"], true);
        assert_eq!(resp_val["result"]["committed"], true);
        unsafe {
            nola_core_child_pump_close(pump);
            nola_core_child_pump_free(pump);
        }

        let recall_req = json!({
            "query": "Rust memory safety",
            "limit": 1,
            "items": [
                {"title": "Cooking", "content": "Pasta recipe", "updated_at_ms": 100},
                {"title": "Systems", "content": "Rust ensures memory safety", "updated_at_ms": 200}
            ]
        })
        .to_string();
        let recall_ptr =
            unsafe { nola_core_intelligence_recall(recall_req.as_ptr(), recall_req.len()) };
        let recall_val: Value =
            serde_json::from_str(unsafe { CStr::from_ptr(recall_ptr) }.to_str().unwrap()).unwrap();
        unsafe { nola_core_free_c_string(recall_ptr) };
        assert_eq!(recall_val["items"].as_array().unwrap().len(), 1);
        assert!(recall_val["context"].as_str().unwrap().contains("Rust"));

        let root_c = CString::new(dir.path().to_str().unwrap()).unwrap();
        let pkg_req_c =
            CString::new(r#"{"url":"https://github.com/acme/nola-skills/blob/main/audit/SKILL.md"}"#)
                .unwrap();
        let pkg_ptr =
            unsafe { nola_mobile_github_skill_package_json(root_c.as_ptr(), pkg_req_c.as_ptr()) };
        let pkg_val: Value =
            serde_json::from_str(unsafe { CStr::from_ptr(pkg_ptr) }.to_str().unwrap()).unwrap();
        unsafe { nola_core_free_c_string(pkg_ptr) };
        assert_eq!(pkg_val["ok"], true);
        assert_eq!(pkg_val["value"]["name"], "audit");

        // Retain + free safety
        let retained = unsafe { nola_core_mobile_mcp_retain(app) };
        assert!(!retained.is_null());
        unsafe { nola_core_application_free(retained) };

        let cmd = r#"{"request_id":"r1","client_id":"c1","command":{"type":"health","arguments":{}}}"#;
        let out_ptr = unsafe { nola_core_application_submit(app, cmd.as_ptr(), cmd.len()) };
        let out_str = unsafe { CStr::from_ptr(out_ptr) }.to_str().unwrap().to_string();
        unsafe { nola_core_free_c_string(out_ptr) };
        assert!(out_str.contains("\"status\":\"committed\""));

        // Mobile MCP host: prepare + execute on configured server
        let add_mcp = json!({
            "request_id": "mcp-add-1",
            "client_id": "c1",
            "command": {
                "type": "mcp",
                "arguments": {
                    "operation": "upsert",
                    "server": {
                        "id": "srv-1",
                        "name": "Test MCP",
                        "transport": "streamable_http",
                        "url": "http://127.0.0.1:1/mcp",
                        "enabled": true
                    }
                }
            }
        })
        .to_string();
        let add_ptr = unsafe { nola_core_application_submit(app, add_mcp.as_ptr(), add_mcp.len()) };
        unsafe { nola_core_free_c_string(add_ptr) };

        let prep_req = json!({
            "operation": "prepare",
            "server_id": "srv-1",
            "tool_name": "echo",
            "arguments": {"msg": "hello"}
        })
        .to_string();
        let prep_ptr =
            unsafe { nola_core_mobile_mcp_host(app, prep_req.as_ptr(), prep_req.len()) };
        let prep_val: Value =
            serde_json::from_str(unsafe { CStr::from_ptr(prep_ptr) }.to_str().unwrap()).unwrap();
        unsafe { nola_core_free_c_string(prep_ptr) };
        assert_eq!(prep_val["ok"], true);
        assert_eq!(prep_val["result"]["prepared"], true);
        assert_eq!(prep_val["result"]["server_id"], "srv-1");
        assert_eq!(prep_val["result"]["tool_name"], "echo");

        // execute aliases call_tool and returns clean not_found for missing server
        let exec_req = json!({
            "operation": "execute",
            "server_id": "missing-srv",
            "tool_name": "echo"
        })
        .to_string();
        let exec_ptr =
            unsafe { nola_core_mobile_mcp_host(app, exec_req.as_ptr(), exec_req.len()) };
        let exec_val: Value =
            serde_json::from_str(unsafe { CStr::from_ptr(exec_ptr) }.to_str().unwrap()).unwrap();
        unsafe { nola_core_free_c_string(exec_ptr) };
        assert_eq!(exec_val["ok"], false);
        assert_eq!(exec_val["error"]["code"], "not_found");

        // Prompt composer
        let prompt_in = r#"{"schema_version":1,"fragments":[{"id":"sys","layer":"identity","trust":"host_instruction","cacheable":true,"content":"You are Nola."},{"id":"dyn","layer":"runtime_state","trust":"host_fact","cacheable":false,"content":"Now"}]}"#;
        let comp_ptr =
            unsafe { nola_core_compose_agent_prompt_json(prompt_in.as_ptr(), prompt_in.len()) };
        let comp_str = unsafe { CStr::from_ptr(comp_ptr) }.to_str().unwrap().to_string();
        unsafe { nola_core_free_c_string(comp_ptr) };
        let comp_val: Value = serde_json::from_str(&comp_str).unwrap();
        assert_eq!(comp_val["composition"]["stable_prefix"], "You are Nola.");
        assert_eq!(comp_val["composition"]["dynamic_suffix"], "Now");

        // Continuation stitching overlap removal
        let stitch_in = r#"{"accumulated":"Hello world","continuation":"world!"}"#;
        let st_ptr =
            unsafe { nola_core_stitch_agent_model_continuation(stitch_in.as_ptr(), stitch_in.len()) };
        let st_str = unsafe { CStr::from_ptr(st_ptr) }.to_str().unwrap().to_string();
        unsafe { nola_core_free_c_string(st_ptr) };
        assert_eq!(serde_json::from_str::<Value>(&st_str).unwrap()["append"], "!");

        // Privacy redaction (4-arg + credential scrub)
        let payload = r#"{"prompt":"key=sk-1234567890abcdefghijklmnop token=custom-secret-999"}"#;
        let cred = "custom-secret-999";
        let red_ptr = unsafe {
            nola_core_redact_model_request_json(
                payload.as_ptr(),
                payload.len(),
                cred.as_ptr(),
                cred.len(),
            )
        };
        let red_str = unsafe { CStr::from_ptr(red_ptr) }.to_str().unwrap().to_string();
        unsafe { nola_model_privacy_free(red_ptr) };
        let parsed: Value = serde_json::from_str(&red_str).unwrap();
        assert_eq!(
            parsed["prompt"],
            "key=[CREDENTIAL] token=[CREDENTIAL]"
        );

        unsafe { nola_core_application_free(app) };
    }

    #[test]
    fn data_source_resolve_enforces_all_consent_policies_and_freshness() {
        let eval = |payload: Value| -> Value {
            let raw = payload.to_string();
            let ptr = unsafe { nola_core_data_source_resolve(raw.as_ptr(), raw.len()) };
            assert!(!ptr.is_null());
            let parsed: Value =
                serde_json::from_str(unsafe { CStr::from_ptr(ptr) }.to_str().unwrap()).unwrap();
            unsafe { nola_core_free_c_string(ptr) };
            parsed
        };

        // 1. opt_in MUST be disabled & not usable without explicit conversationOverride: true
        let opt_in_default = eval(json!({
            "sourceId": "system:calendar",
            "policy": "opt_in",
            "conversationOverride": null,
            "contract": {
                "sourceId": "system:calendar",
                "lastRefreshedAt": "2026-11-25T12:00:00Z",
                "ttlSeconds": 600
            },
            "now": "2026-11-25T12:05:00Z"
        }));
        assert_eq!(opt_in_default["enabled"], false);
        assert_eq!(opt_in_default["usable"], false);
        assert_eq!(opt_in_default["shouldRequestConsent"], false);
        assert_eq!(opt_in_default["reason"], "conversation_opt_in_required");

        let opt_in_enabled = eval(json!({
            "sourceId": "system:calendar",
            "policy": "opt_in",
            "conversationOverride": true,
            "contract": {
                "sourceId": "system:calendar",
                "lastRefreshedAt": "2026-11-25T12:00:00Z",
                "ttlSeconds": 600
            },
            "now": "2026-11-25T12:05:00Z"
        }));
        assert_eq!(opt_in_enabled["enabled"], true);
        assert_eq!(opt_in_enabled["usable"], true);
        assert_eq!(opt_in_enabled["stale"], false);
        assert_eq!(opt_in_enabled["shouldRefresh"], false);

        // 2. ask_each_time MUST require consent and be unusable until explicitly consented
        let ask_default = eval(json!({
            "sourceId": "system:contacts",
            "policy": "ask_each_time",
            "conversationOverride": null,
            "contract": {
                "sourceId": "system:contacts",
                "lastRefreshedAt": "2026-11-25T12:00:00Z",
                "ttlSeconds": 600
            },
            "now": "2026-11-25T12:01:00Z"
        }));
        assert_eq!(ask_default["enabled"], false);
        assert_eq!(ask_default["usable"], false);
        assert_eq!(ask_default["shouldRequestConsent"], true);
        assert_eq!(ask_default["reason"], "consent_required");

        let ask_consented = eval(json!({
            "sourceId": "system:contacts",
            "policy": "ask_each_time",
            "conversationOverride": true,
            "contract": {
                "sourceId": "system:contacts",
                "lastRefreshedAt": "2026-11-25T12:00:00Z",
                "ttlSeconds": 600
            },
            "now": "2026-11-25T12:01:00Z"
        }));
        assert_eq!(ask_consented["enabled"], true);
        assert_eq!(ask_consented["usable"], true);
        assert_eq!(ask_consented["shouldRequestConsent"], false);

        // 3. disabled MUST always stay disabled even with conversationOverride: true
        let disabled = eval(json!({
            "sourceId": "system:mail",
            "policy": "disabled",
            "conversationOverride": true,
            "contract": {"sourceId": "system:mail", "lastRefreshedAt": "2026-11-25T12:00:00Z", "ttlSeconds": 600}
        }));
        assert_eq!(disabled["enabled"], false);
        assert_eq!(disabled["usable"], false);
        assert_eq!(disabled["reason"], "blocked");

        // 4. Stale contract evaluation via now vs lastRefreshedAt + ttlSeconds
        let stale = eval(json!({
            "sourceId": "system:notes",
            "policy": "default_enabled",
            "contract": {
                "sourceId": "system:notes",
                "lastRefreshedAt": "2026-11-25T10:00:00Z",
                "ttlSeconds": 300
            },
            "now": "2026-11-25T12:00:00Z"
        }));
        assert_eq!(stale["enabled"], true);
        assert_eq!(stale["usable"], false);
        assert_eq!(stale["stale"], true);
        assert_eq!(stale["shouldRefresh"], true);
        assert_eq!(stale["reason"], "stale");

        // 5. Missing contract -> enabled true (for Swift isEnabledForConversation), usable false, shouldRefresh true
        let missing_contract = eval(json!({
            "sourceId": "system:notes",
            "policy": "default_enabled"
        }));
        assert_eq!(missing_contract["enabled"], true);
        assert_eq!(missing_contract["usable"], false);
        assert_eq!(missing_contract["shouldRefresh"], true);
        assert_eq!(missing_contract["reason"], "contract_missing");
    }

    #[test]
    fn child_pump_real_host_roundtrip_and_cancellation() {
        let dir = tempfile::tempdir().unwrap();
        let db_str = dir.path().join("child_pump.db").display().to_string();
        let app = unsafe { nola_core_application_open(db_str.as_ptr(), db_str.len()) };
        assert!(!app.is_null());

        let start_req = json!({
            "task_id": "task-child-1",
            "conversation_id": "conv-child-1",
            "run_id": "run-child-1",
            "assistant_message_id": "run-child-1:assistant",
            "prompt": "Summarize workspace",
            "host_driven": true
        })
        .to_string();
        let mut err_ptr: *mut c_char = std::ptr::null_mut();
        let pump = unsafe {
            nola_core_child_pump_start(app, start_req.as_ptr(), start_req.len(), &mut err_ptr)
        };
        assert!(!pump.is_null());
        assert!(err_ptr.is_null());

        // 1. Poll should yield real host_request ticket with pump_id & transport_finished=false
        let p1_ptr = unsafe { nola_core_child_pump_poll(pump, 1000) };
        let p1: Value =
            serde_json::from_str(unsafe { CStr::from_ptr(p1_ptr) }.to_str().unwrap()).unwrap();
        unsafe { nola_core_free_c_string(p1_ptr) };
        assert_eq!(p1["ok"], true);
        assert_eq!(p1["result"]["transport_finished"], false);
        assert_eq!(p1["result"]["kind"], "host_request");
        let pump_id = p1["result"]["pump_id"].as_str().unwrap().to_string();
        assert!(!pump_id.is_empty());
        let ticket = p1["result"]["request"]["ticket"].as_u64().unwrap();
        assert_eq!(
            p1["result"]["request"]["envelope"]["payload"]["type"],
            "model_stream"
        );

        // 2. Deliver host response back to running child pump
        let reply = json!({
            "pump_id": pump_id,
            "ticket": ticket,
            "response": {"content": "Child summary complete."}
        })
        .to_string();
        let r_ptr = unsafe { nola_core_child_pump_respond(pump, reply.as_ptr(), reply.len()) };
        let r_val: Value =
            serde_json::from_str(unsafe { CStr::from_ptr(r_ptr) }.to_str().unwrap()).unwrap();
        unsafe { nola_core_free_c_string(r_ptr) };
        assert_eq!(r_val["ok"], true);
        assert_eq!(r_val["result"]["committed"], true);

        // 3. Poll receives turn_completed event, then transport_finished=true
        let p2_ptr = unsafe { nola_core_child_pump_poll(pump, 1000) };
        let p2: Value =
            serde_json::from_str(unsafe { CStr::from_ptr(p2_ptr) }.to_str().unwrap()).unwrap();
        unsafe { nola_core_free_c_string(p2_ptr) };
        assert_eq!(p2["result"]["kind"], "turn_completed");
        assert_eq!(p2["result"]["event"]["final_text"], "Child summary complete.");

        let p3_ptr = unsafe { nola_core_child_pump_poll(pump, 1000) };
        let p3: Value =
            serde_json::from_str(unsafe { CStr::from_ptr(p3_ptr) }.to_str().unwrap()).unwrap();
        unsafe { nola_core_free_c_string(p3_ptr) };
        assert_eq!(p3["result"]["transport_finished"], true);
        assert_eq!(p3["result"]["pump_id"], pump_id);

        unsafe {
            nola_core_child_pump_close(pump);
            nola_core_child_pump_free(pump);
            nola_core_application_free(app);
        }
    }

    #[test]
    fn mobile_mcp_host_prepare_nested_execute_and_cancel() {
        let dir = tempfile::tempdir().unwrap();
        let db_str = dir.path().join("mcp_host.db").display().to_string();
        let app = unsafe { nola_core_application_open(db_str.as_ptr(), db_str.len()) };
        assert!(!app.is_null());

        let app_ref = unsafe { &*app };
        app_ref
            .inner
            .store
            .upsert_mcp_server(&nola_types::McpServerConfig {
                id: "docs-srv".into(),
                name: "Docs MCP".into(),
                transport: nola_types::McpTransport::StreamableHttp,
                command: None,
                args: Vec::new(),
                env: BTreeMap::new(),
                url: Some("http://127.0.0.1:1/mcp".into()),
                headers: BTreeMap::new(),
                oauth_client_id: None,
                oauth_scopes: Vec::new(),
                enabled: true,
            })
            .unwrap();
        // Seed tool cache for offline deterministic discovery verification
        app_ref
            .inner
            .store
            .set_kv(
                "mcp_tools_cache:docs-srv",
                &json!([
                    {"name": "search", "description": "Search docs", "inputSchema": {"type": "object"}},
                    {"name": "blocked_op", "description": "Blocked", "inputSchema": {"type": "object"}}
                ]),
            )
            .unwrap();

        // 1. iOS multi-server prepare (host_id + session_id + policy)
        let prep_req = json!({
            "type": "prepare",
            "host_id": "ios-host-1",
            "session_id": "sess-1",
            "policy": {"user_policy": {"deny_tools": ["blocked_op"]}}
        })
        .to_string();
        let prep_ptr =
            unsafe { nola_core_mobile_mcp_host(app, prep_req.as_ptr(), prep_req.len()) };
        let prep: Value =
            serde_json::from_str(unsafe { CStr::from_ptr(prep_ptr) }.to_str().unwrap()).unwrap();
        unsafe { nola_core_free_c_string(prep_ptr) };
        assert_eq!(prep["ok"], true);
        assert_eq!(prep["result"]["prepared"], true);
        assert!(prep["result"]["capabilities"]["mcp__docs-srv__search"].is_object());
        assert!(prep["result"]["capabilities"]["mcp__docs-srv__blocked_op"].is_null());
        assert_eq!(prep["result"]["tools"].as_array().unwrap().len(), 1);

        // 2. Pre-cancel request_id via cancel (works with null app!) then execute nested request -> aborted immediately!
        let cancel_req = json!({"type": "cancel", "request_id": "req-cancel-42"}).to_string();
        let c_ptr = unsafe {
            nola_core_mobile_mcp_host(std::ptr::null_mut(), cancel_req.as_ptr(), cancel_req.len())
        };
        let c_val: Value =
            serde_json::from_str(unsafe { CStr::from_ptr(c_ptr) }.to_str().unwrap()).unwrap();
        unsafe { nola_core_free_c_string(c_ptr) };
        assert_eq!(c_val["ok"], true);
        assert_eq!(c_val["result"]["cancelled"], true);

        let exec_req = json!({
            "type": "execute",
            "host_id": "ios-host-1",
            "session_id": "sess-1",
            "request": {
                "server_id": "docs-srv",
                "tool_name": "search",
                "arguments": {"q": "rust"},
                "request_id": "req-cancel-42"
            }
        })
        .to_string();
        let e_ptr = unsafe { nola_core_mobile_mcp_host(app, exec_req.as_ptr(), exec_req.len()) };
        let e_val: Value =
            serde_json::from_str(unsafe { CStr::from_ptr(e_ptr) }.to_str().unwrap()).unwrap();
        unsafe { nola_core_free_c_string(e_ptr) };
        assert_eq!(e_val["ok"], false);
        assert_eq!(e_val["error"]["code"], "cancelled");

        unsafe { nola_core_application_free(app) };
    }

    #[test]
    fn context_prepare_preserves_tool_pairs_and_emits_component_metrics() {
        let huge = "payload ".repeat(200);
        let req = json!({
            "messages": [
                {"role": "system", "content": "sys"},
                {"role": "assistant", "content": huge, "tool_calls": [{"id": "tc-1", "name": "read", "arguments": {}}]},
                {"role": "tool", "tool_call_id": "tc-1", "content": "result-1"},
                {"role": "user", "content": "follow up"},
                {"role": "assistant", "content": "done"}
            ],
            "tools": [{"name": "read", "parameters": {}}],
            "budget": {"prompt_token_limit": 80, "keep_recent_messages": 2}
        })
        .to_string();
        let ptr = unsafe { nola_core_context_prepare(req.as_ptr(), req.len()) };
        let val: Value =
            serde_json::from_str(unsafe { CStr::from_ptr(ptr) }.to_str().unwrap()).unwrap();
        unsafe { nola_core_free_c_string(ptr) };
        assert_eq!(val["changed"], true);
        let msgs = val["messages"].as_array().unwrap();
        // Both assistant(tool_calls: tc-1) and tool(tool_call_id: tc-1) MUST be pruned atomically together!
        assert!(!msgs.iter().any(|m| m["role"] == "tool"));
        assert!(!msgs
            .iter()
            .any(|m| m.get("tool_calls").and_then(|v| v.as_array()).is_some()));
        assert!(!val["component_metrics"].as_array().unwrap().is_empty());
    }

    #[test]
    fn compose_agent_prompt_enforces_layer_order_and_trust_invariants() {
        // 1. Canonical layer ordering (identity before response_style even when supplied reversed)
        let ordered_in = json!({
            "schema_version": 1,
            "fragments": [
                {"id": "style", "layer": "response_style", "trust": "host_instruction", "cacheable": true, "content": "Be concise."},
                {"id": "ident", "layer": "identity", "trust": "host_instruction", "cacheable": true, "content": "You are Nola."},
                {"id": "rt", "layer": "runtime_state", "trust": "host_fact", "cacheable": false, "content": "cwd=/tmp"}
            ]
        })
        .to_string();
        let ptr = unsafe { nola_core_compose_agent_prompt_json(ordered_in.as_ptr(), ordered_in.len()) };
        let val: Value =
            serde_json::from_str(unsafe { CStr::from_ptr(ptr) }.to_str().unwrap()).unwrap();
        unsafe { nola_core_free_c_string(ptr) };
        assert_eq!(val["composition"]["fragment_ids"], json!(["ident", "style", "rt"]));

        // 2. Reject ReferenceMarkedCacheable
        let bad_ref = json!({
            "schema_version": 1,
            "fragments": [
                {"id": "ref1", "layer": "conversation_context", "trust": "reference", "cacheable": true, "content": "untrusted"}
            ]
        })
        .to_string();
        let p_ref = unsafe { nola_core_compose_agent_prompt_json(bad_ref.as_ptr(), bad_ref.len()) };
        let v_ref: Value =
            serde_json::from_str(unsafe { CStr::from_ptr(p_ref) }.to_str().unwrap()).unwrap();
        unsafe { nola_core_free_c_string(p_ref) };
        assert_eq!(v_ref["error"]["code"], "reference_marked_cacheable");

        // 3. Reject RuntimeStateMarkedCacheable
        let bad_rt_cache = json!({
            "schema_version": 1,
            "fragments": [
                {"id": "rt1", "layer": "runtime_state", "trust": "host_fact", "cacheable": true, "content": "now"}
            ]
        })
        .to_string();
        let p_rtc =
            unsafe { nola_core_compose_agent_prompt_json(bad_rt_cache.as_ptr(), bad_rt_cache.len()) };
        let v_rtc: Value =
            serde_json::from_str(unsafe { CStr::from_ptr(p_rtc) }.to_str().unwrap()).unwrap();
        unsafe { nola_core_free_c_string(p_rtc) };
        assert_eq!(v_rtc["error"]["code"], "runtime_state_marked_cacheable");

        // 4. Reject RuntimeStateInvalidTrust
        let bad_rt_trust = json!({
            "schema_version": 1,
            "fragments": [
                {"id": "rt2", "layer": "runtime_state", "trust": "user_instruction", "cacheable": false, "content": "now"}
            ]
        })
        .to_string();
        let p_rtt =
            unsafe { nola_core_compose_agent_prompt_json(bad_rt_trust.as_ptr(), bad_rt_trust.len()) };
        let v_rtt: Value =
            serde_json::from_str(unsafe { CStr::from_ptr(p_rtt) }.to_str().unwrap()).unwrap();
        unsafe { nola_core_free_c_string(p_rtt) };
        assert_eq!(v_rtt["error"]["code"], "runtime_state_invalid_trust");
    }

    #[test]
    fn intelligence_recall_without_items_queries_store_and_strategies_and_policy_ignores_tool_id_keywords() {
        let dir = tempfile::tempdir().unwrap();
        let db_str = dir.path().join("intel.db").display().to_string();
        let app = unsafe { nola_core_application_open(db_str.as_ptr(), db_str.len()) };
        assert!(!app.is_null());

        let app_ref = unsafe { &*app };
        app_ref
            .inner
            .store
            .upsert_memory(&MemoryItem {
                id: "m-mac-1".into(),
                namespace: "intelligence:memory".into(),
                title: "Build Preference".into(),
                content: "Always use cargo test --lib for Rust unit checks".into(),
                tags: vec!["rust".into()],
                created_at_ms: 1000,
                updated_at_ms: 2000,
            })
            .unwrap();
        app_ref
            .inner
            .store
            .put_module_record(
                "memory_strategy",
                "strat-1",
                &json!({"id": "strat-1", "title": "Rust Verification", "summary": "Run cargo test after edits"}),
            )
            .unwrap();

        // macOS path: omits `items` completely!
        let recall_req = json!({
            "query": "cargo test",
            "limit": 6,
            "minimum_score": 0.2,
            "db_path": db_str
        })
        .to_string();
        let r_ptr =
            unsafe { nola_core_intelligence_recall(recall_req.as_ptr(), recall_req.len()) };
        let r_val: Value =
            serde_json::from_str(unsafe { CStr::from_ptr(r_ptr) }.to_str().unwrap()).unwrap();
        unsafe { nola_core_free_c_string(r_ptr) };
        assert_eq!(r_val["ok"], true);
        assert!(!r_val["items"].as_array().unwrap().is_empty());
        assert!(!r_val["strategies"].as_array().unwrap().is_empty());
        assert!(r_val["context"].as_str().unwrap().contains("cargo test"));

        // Snapshot includes real strategies
        let snap_ptr = unsafe { nola_core_application_intelligence_snapshot(app) };
        let snap: Value =
            serde_json::from_str(unsafe { CStr::from_ptr(snap_ptr) }.to_str().unwrap()).unwrap();
        unsafe { nola_core_free_c_string(snap_ptr) };
        assert_eq!(snap["strategies"].as_array().unwrap().len(), 1);

        // Tool policy MUST NOT use tool_id substring keywords (AGENTS.md guardrail)
        let tp_req = json!({
            "schema_version": 1,
            "platform": "ios",
            "now_ms": 1000,
            "runtime": {
                "expires_at_ms": 999999,
                "tools": [
                    {"tool_id": "write_delete_exec_name", "effect": "read_only", "capability": "safe_cap", "route": "device", "available": true},
                    {"tool_id": "read_list_get_name", "effect": "high", "capability": "danger_cap", "route": "device", "available": true}
                ]
            },
            "user_policy": {"max_risk": "read_only", "allow_result_egress_routes": ["device"]}
        })
        .to_string();
        let tp_ptr = unsafe { nola_core_tool_policy_resolve(tp_req.as_ptr(), tp_req.len()) };
        let tp: Value =
            serde_json::from_str(unsafe { CStr::from_ptr(tp_ptr) }.to_str().unwrap()).unwrap();
        unsafe { nola_core_free_c_string(tp_ptr) };
        assert_eq!(tp["enabled_tools"].as_array().unwrap().len(), 1);
        assert_eq!(tp["enabled_tools"][0]["tool_id"], "write_delete_exec_name");
        assert_eq!(tp["disabled_tools"][0]["tool_id"], "read_list_get_name");
        assert_eq!(tp["disabled_tools"][0]["reason"], "risk_exceeds_user_policy");

        unsafe { nola_core_application_free(app) };
    }
}
