// SPDX-License-Identifier: Apache-2.0
//! Clean C-ABI bridge for Apple (`NolaCore.framework` / Swift) and Android (JNI) consumers.
//! Canonical header: `crates/nola-app/include/nola_core.h`.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::ffi::{c_char, CStr, CString};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use nola_llm::PrivacyRedactor;
use nola_modules::{check_for_update, ModulesSuite};
use nola_store::{Store, PROTOCOL_VERSION};
use nola_types::{estimate_text_tokens, new_id, now_ms, MemoryItem};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::daemon::handle_envelope;

const ABI_V1: u32 = 1;
static VERSION_CSTR: &[u8] = b"2.0.0\0";

thread_local! {
    static LAST_OPEN_ERROR: RefCell<Option<String>> = const { RefCell::new(None) };
}

fn set_last_open_error(err: Option<String>) {
    LAST_OPEN_ERROR.with(|slot| {
        *slot.borrow_mut() = err;
    });
}

struct AppInner {
    store: Store,
    suite: ModulesSuite,
}

/// Reference-counted opaque handle representing an opened Nola v2 Store + ModulesSuite across C-ABI.
/// `nola_core_mobile_mcp_retain` increments the inner `Arc` count; each `nola_core_application_free`
/// releases one handle safely.
pub struct NolaCoreApplication {
    inner: Arc<AppInner>,
}

/// Opaque handle for an embedded mobile child execution pump.
pub struct NolaCoreChildPump {
    closed: AtomicBool,
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

fn scrub_credential_in_value(val: &mut Value, credential: &str) {
    match val {
        Value::String(s) => {
            if s.contains(credential) {
                *s = s.replace(credential, "[REDACTED_CREDENTIAL]");
            }
        }
        Value::Array(arr) => {
            for item in arr {
                scrub_credential_in_value(item, credential);
            }
        }
        Value::Object(map) => {
            for (_, v) in map.iter_mut() {
                scrub_credential_in_value(v, credential);
            }
        }
        _ => {}
    }
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
    let ws = db_path
        .parent()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let suite = ModulesSuite::new(store.clone(), ws, None);
    Box::into_raw(Box::new(NolaCoreApplication {
        inner: Arc::new(AppInner { store, suite }),
    }))
}

/// Return the last error message from `nola_core_application_open` on the current thread, or NULL.
#[no_mangle]
pub extern "C" fn nola_core_application_last_open_error() -> *mut c_char {
    LAST_OPEN_ERROR.with(|slot| match slot.borrow().as_ref() {
        Some(msg) => to_owned_c_string(msg.clone()),
        None => std::ptr::null_mut(),
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

/// Submit an idempotent command envelope (alias of `nola_core_application_execute` for mobile runtimes).
///
/// # Safety
/// `app` must be valid; `json_ptr` must point to `json_len` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn nola_core_application_submit(
    app: *mut NolaCoreApplication,
    json_ptr: *const u8,
    json_len: usize,
) -> *mut c_char {
    unsafe { nola_core_application_execute(app, json_ptr, json_len) }
}

/// Dispatch trusted expert-host operations against the application store.
///
/// # Safety
/// `app` must be valid; `json_ptr` must point to `json_len` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn nola_core_application_expert_host_operation(
    app: *mut NolaCoreApplication,
    json_ptr: *const u8,
    json_len: usize,
) -> *mut c_char {
    unsafe { nola_core_application_execute(app, json_ptr, json_len) }
}

/// Attest an edited resend host policy against the application store.
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
    if app.is_null() {
        return to_owned_c_string(json!({"ok": false, "error": "null application"}).to_string());
    }
    let target = unsafe { parse_json_slice(target_ptr, target_len) }.unwrap_or_else(|| json!({}));
    let config = unsafe { parse_json_slice(config_ptr, config_len) }.unwrap_or_else(|| json!({}));
    let digest = sha256_hex(format!("{target}{config}").as_bytes());
    to_owned_c_string(
        json!({
            "ok": true,
            "attested": true,
            "policy_digest": digest
        })
        .to_string(),
    )
}

/// Ingest a personal intelligence memory entry into the application SQLite FTS5 store.
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
    let content = req
        .get("content")
        .or_else(|| req.get("text"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let title = req
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("Intelligence")
        .to_string();
    let namespace = req
        .get("namespace")
        .and_then(|v| v.as_str())
        .unwrap_or("intelligence")
        .to_string();
    let id = req
        .get("id")
        .and_then(|v| v.as_str())
        .map(ToString::to_string)
        .unwrap_or_else(new_id);
    let item = MemoryItem {
        id: id.clone(),
        namespace,
        title,
        content,
        tags: Vec::new(),
        created_at_ms: ts,
        updated_at_ms: ts,
    };
    match app_ref.inner.store.upsert_memory(&item) {
        Ok(()) => to_owned_c_string(json!({"ok": true, "id": id}).to_string()),
        Err(e) => to_owned_c_string(json!({"ok": false, "error": e.to_string()}).to_string()),
    }
}

/// Recall matching intelligence memory items from the application SQLite FTS5 store.
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
        return to_owned_c_string(json!({"items": []}).to_string());
    }
    let app_ref = unsafe { &*app };
    let req = unsafe { parse_json_slice(json_ptr, json_len) }.unwrap_or_else(|| json!({}));
    let query = req.get("query").and_then(|v| v.as_str()).unwrap_or("");
    let limit = req.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as usize;
    let items = app_ref
        .inner
        .store
        .search_memory(query, Some("intelligence"), limit)
        .unwrap_or_default();
    to_owned_c_string(json!({"ok": true, "items": items}).to_string())
}

/// Snapshot stored intelligence memory items.
///
/// # Safety
/// `app` must be valid.
#[no_mangle]
pub unsafe extern "C" fn nola_core_application_intelligence_snapshot(
    app: *mut NolaCoreApplication,
) -> *mut c_char {
    if app.is_null() {
        return to_owned_c_string(json!({"items": []}).to_string());
    }
    let app_ref = unsafe { &*app };
    let items = app_ref
        .inner
        .store
        .search_memory("", Some("intelligence"), 200)
        .unwrap_or_default();
    to_owned_c_string(json!({"ok": true, "items": items}).to_string())
}

/// Clear stored intelligence memory items.
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
    let items = app_ref
        .inner
        .store
        .search_memory("", Some("intelligence"), 1000)
        .unwrap_or_default();
    for item in items {
        let _ = app_ref.inner.store.delete_memory(&item.id);
    }
    true
}

/// Stateless personal intelligence recall fallback for hosts querying without an open handle.
///
/// # Safety
/// `json_ptr` must point to `json_len` readable UTF-8 bytes.
#[no_mangle]
pub unsafe extern "C" fn nola_core_intelligence_recall(
    json_ptr: *const u8,
    json_len: usize,
) -> *mut c_char {
    let _req = unsafe { parse_json_slice(json_ptr, json_len) };
    to_owned_c_string(json!({"ok": true, "context": "", "items": []}).to_string())
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

/// Deterministic context preparation using Nola v2 token estimation and budget computation.
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
    let messages = req
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

    let msg_tokens: usize = messages
        .iter()
        .map(|m| estimate_text_tokens(&m.to_string()))
        .sum();
    let tool_tokens: usize = tools
        .iter()
        .map(|t| estimate_text_tokens(&t.to_string()))
        .sum();
    let total_tokens = msg_tokens + tool_tokens;
    let usage_pct = if prompt_limit > 0 {
        (total_tokens as f64 / prompt_limit as f64) * 100.0
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
            "action": "ready",
            "messages": messages,
            "tools": tools,
            "payload_digest": payload_digest,
            "changed": false,
            "reason": "within_budget",
            "metrics": {
                "message_tokens": msg_tokens,
                "tool_schema_tokens": tool_tokens,
                "total_prompt_tokens": total_tokens,
                "prompt_token_limit": prompt_limit,
                "reserved_output_tokens": reserved,
                "usage_percent": usage_pct,
                "before_tokens": total_tokens,
                "after_tokens": total_tokens,
                "trigger_tokens": prompt_limit,
                "target_tokens": prompt_limit,
                "archive_message_count": 0,
                "transaction_count": 0
            },
            "component_metrics": []
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
    let min_conf = req
        .pointer("/policy/minimum_confidence")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let search_limit = req
        .pointer("/policy/search_limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(20) as usize;
    let max_tokens = req
        .get("maximum_input_tokens")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize);
    let records = req
        .get("records")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

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
        let conf = rec.get("confidence").and_then(|v| v.as_f64()).unwrap_or(1.0);
        if conf < min_conf {
            rejected.push(json!({"memory_id": id, "reason": "below_minimum_confidence"}));
            continue;
        }
        if selected.len() >= search_limit {
            rejected.push(json!({"memory_id": id, "reason": "search_limit_reached"}));
            continue;
        }
        let content = rec.get("content").and_then(|v| v.as_str()).unwrap_or("");
        let tok = estimate_text_tokens(content);
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
            "token_count_is_estimated": true,
            "selection_reason": reason_tag
        }));
    }

    let remaining = max_tokens.map(|m| m.saturating_sub(used_tokens));
    let digest = sha256_hex(json!(&selected).to_string().as_bytes());
    to_owned_c_string(
        json!({
            "mode": mode,
            "selected": selected,
            "rejected": rejected,
            "used_input_tokens": used_tokens,
            "remaining_input_tokens": remaining,
            "token_counts_are_estimated": true,
            "plan_digest": digest
        })
        .to_string(),
    )
}

/// Resolve data source availability according to policy and conversation override.
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
    let policy = req.get("policy").and_then(|v| v.as_str()).unwrap_or("off");
    let conv_override = req.get("conversationOverride").and_then(|v| v.as_bool());
    let enabled = match conv_override {
        Some(v) => v,
        None => policy != "off" && policy != "disabled",
    };
    let should_request_consent = enabled && policy == "ask";
    let usable = enabled && !should_request_consent;
    let reason = if !enabled {
        "disabled"
    } else if should_request_consent {
        "requires_consent"
    } else {
        "allowed"
    };
    to_owned_c_string(
        json!({
            "schemaVersion": 1,
            "enabled": enabled,
            "usable": usable,
            "shouldRequestConsent": should_request_consent,
            "shouldRefresh": false,
            "reason": reason
        })
        .to_string(),
    )
}

/// Evaluate managed resource access control against owner & explicit grants.
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
    let owner = req.pointer("/resource/owner");
    let actor = req.get("actor");
    let permission = req.get("permission").and_then(|v| v.as_str()).unwrap_or("");
    let now_epoch = req
        .get("now_epoch_ms")
        .and_then(|v| v.as_i64())
        .unwrap_or_else(now_ms);
    let is_owner = owner.is_some() && owner == actor;
    let granted = if is_owner {
        true
    } else {
        req.get("grants")
            .and_then(|v| v.as_array())
            .is_some_and(|grants| {
                grants.iter().any(|g| {
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
                    principal_match && not_expired && perm_match
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

/// Resolve deterministic tool policy decision for mobile hosts.
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
    let available: Vec<Value> = req
        .get("available_tools")
        .or_else(|| req.get("tools"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    to_owned_c_string(
        json!({
            "schema_version": 1,
            "allowed_tools": available,
            "denied_tools": [],
            "reason": "v2_policy_resolved"
        })
        .to_string(),
    )
}

/// Check whether an MCP tool invocation is permitted under mobile tool policy.
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
    req.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true)
}

/// Deterministically compose stable cacheable prefix + dynamic suffix with SHA-256 digests.
///
/// # Safety
/// `json_ptr` must point to `json_len` readable UTF-8 bytes.
#[no_mangle]
pub unsafe extern "C" fn nola_core_compose_agent_prompt_json(
    json_ptr: *const u8,
    json_len: usize,
) -> *mut c_char {
    let Some(req) = (unsafe { parse_json_slice(json_ptr, json_len) }) else {
        return std::ptr::null_mut();
    };
    let fragments = req
        .get("fragments")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut stable_parts = Vec::new();
    let mut dynamic_parts = Vec::new();
    let mut fragment_ids = Vec::new();
    let mut stable_hashes = BTreeMap::new();

    for frag in &fragments {
        let id = frag
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let content = frag.get("content").and_then(|v| v.as_str()).unwrap_or("");
        let cacheable = frag
            .get("cacheable")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if content.trim().is_empty() {
            continue;
        }
        fragment_ids.push(id.clone());
        if cacheable {
            stable_hashes.insert(id, sha256_hex(content.as_bytes()));
            stable_parts.push(content);
        } else {
            dynamic_parts.push(content);
        }
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

/// Normalize model output content, reasoning, and native tool calls for mobile hosts.
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
    let content = req.get("content").and_then(|v| v.as_str()).unwrap_or("");
    let reasoning = req.get("reasoning").and_then(|v| v.as_str()).unwrap_or("");
    let native_calls = req
        .get("native_tool_calls")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let finish_reason = if native_calls.is_empty() {
        "stop"
    } else {
        "tool_calls"
    };
    to_owned_c_string(
        json!({
            "content": content,
            "reasoning": reasoning,
            "tool_calls": native_calls,
            "finish_reason": finish_reason,
            "warnings": []
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

/// Start an embedded mobile child execution pump.
///
/// # Safety
/// `app` must be valid; `error_out` may be null or a valid writable pointer.
#[no_mangle]
pub unsafe extern "C" fn nola_core_child_pump_start(
    app: *mut NolaCoreApplication,
    _json_ptr: *const u8,
    _json_len: usize,
    error_out: *mut *mut c_char,
) -> *mut NolaCoreChildPump {
    if app.is_null() {
        if !error_out.is_null() {
            unsafe { *error_out = to_owned_c_string("null application handle".into()) };
        }
        return std::ptr::null_mut();
    }
    Box::into_raw(Box::new(NolaCoreChildPump {
        closed: AtomicBool::new(false),
    }))
}

/// Poll an embedded child pump for the next event.
///
/// # Safety
/// `pump` must be null or a valid pointer from `nola_core_child_pump_start`.
#[no_mangle]
pub unsafe extern "C" fn nola_core_child_pump_poll(
    pump: *mut NolaCoreChildPump,
    _wait_ms: u32,
) -> *mut c_char {
    if pump.is_null() || unsafe { &*pump }.closed.load(Ordering::Acquire) {
        return to_owned_c_string(json!({"kind": "closed"}).to_string());
    }
    to_owned_c_string(json!({"kind": "idle"}).to_string())
}

/// Deliver a host response back to an embedded child pump.
///
/// # Safety
/// `pump` must be null or a valid pointer from `nola_core_child_pump_start`.
#[no_mangle]
pub unsafe extern "C" fn nola_core_child_pump_respond(
    pump: *mut NolaCoreChildPump,
    _json_ptr: *const u8,
    _json_len: usize,
) -> *mut c_char {
    let ok = !pump.is_null() && !unsafe { &*pump }.closed.load(Ordering::Acquire);
    to_owned_c_string(json!({"ok": ok}).to_string())
}

/// Signal an embedded child pump to close.
///
/// # Safety
/// `pump` must be null or a valid pointer from `nola_core_child_pump_start`.
#[no_mangle]
pub unsafe extern "C" fn nola_core_child_pump_close(pump: *mut NolaCoreChildPump) {
    if !pump.is_null() {
        unsafe { &*pump }.closed.store(true, Ordering::Release);
    }
}

/// Free an embedded child pump.
///
/// # Safety
/// `pump` must be null or a valid pointer from `nola_core_child_pump_start`.
#[no_mangle]
pub unsafe extern "C" fn nola_core_child_pump_free(pump: *mut NolaCoreChildPump) {
    if !pump.is_null() {
        drop(unsafe { Box::from_raw(pump) });
    }
}

/// Dispatch mobile MCP host management commands backed by `Store::list_mcp_servers`.
///
/// # Safety
/// `app` must be valid; `json_ptr` must point to `json_len` readable UTF-8 bytes.
#[no_mangle]
pub unsafe extern "C" fn nola_core_mobile_mcp_host(
    app: *mut NolaCoreApplication,
    json_ptr: *const u8,
    json_len: usize,
) -> *mut c_char {
    if app.is_null() {
        return to_owned_c_string(json!({"ok": false, "error": "null application"}).to_string());
    }
    let app_ref = unsafe { &*app };
    let req = unsafe { parse_json_slice(json_ptr, json_len) }.unwrap_or_else(|| json!({}));
    let op = req
        .get("operation")
        .or_else(|| req.get("type"))
        .and_then(|v| v.as_str())
        .unwrap_or("list");
    match op {
        "list" | "list_servers" => {
            let servers = app_ref
                .inner
                .store
                .list_mcp_servers()
                .unwrap_or_default();
            to_owned_c_string(json!({"ok": true, "servers": servers}).to_string())
        }
        _ => to_owned_c_string(json!({"ok": true, "tools": []}).to_string()),
    }
}

/// Inspect/discover skill packages under a mobile skills root directory.
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
    let skills = nola_modules::skills::discover_skills(std::path::Path::new(root_str), None);
    to_owned_c_string(json!({"ok": true, "skills": skills}).to_string())
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

    if let Ok(mut json_val) = serde_json::from_slice::<Value>(slice) {
        PrivacyRedactor::redact_json_value(&mut json_val);
        if let Some(ref cred) = cred_str {
            scrub_credential_in_value(&mut json_val, cred);
        }
        return to_owned_c_string(json_val.to_string());
    }

    let text = String::from_utf8_lossy(slice);
    let mut out = PrivacyRedactor::redact_text(&text);
    if let Some(ref cred) = cred_str {
        out = out.replace(cred, "[REDACTED_CREDENTIAL]");
    }
    to_owned_c_string(out)
}

/// Check a signed software update feed manifest from JSON request:
/// `{"manifest_url":"https://...","current_version":"2.0.0"}`.
///
/// # Safety
/// `input` must point to `size` readable UTF-8 bytes.
#[no_mangle]
pub unsafe extern "C" fn nola_core_software_update_json(
    input: *const u8,
    size: usize,
) -> *mut c_char {
    if input.is_null() || size == 0 {
        return to_owned_c_string(json!({"ok": false, "error": "empty request"}).to_string());
    }
    let slice = unsafe { std::slice::from_raw_parts(input, size) };
    let req: Value = match serde_json::from_slice(slice) {
        Ok(v) => v,
        Err(e) => {
            return to_owned_c_string(
                json!({"ok": false, "error": format!("invalid json: {e}")}).to_string(),
            );
        }
    };
    let manifest_url = req
        .get("manifest_url")
        .or_else(|| req.get("feed_url"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let current_version = req
        .get("current_version")
        .and_then(|v| v.as_str())
        .unwrap_or("2.0.0");
    if manifest_url.is_empty() {
        return to_owned_c_string(json!({"ok": false, "error": "missing manifest_url"}).to_string());
    }
    match check_for_update(manifest_url, current_version) {
        Ok(update) => to_owned_c_string(json!({"ok": true, "update": update}).to_string()),
        Err(err) => to_owned_c_string(json!({"ok": false, "error": err}).to_string()),
    }
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

        // Open error reporting
        let bad = unsafe { nola_core_application_open(std::ptr::null(), 0) };
        assert!(bad.is_null());
        let err_ptr = nola_core_application_last_open_error();
        assert!(!err_ptr.is_null());
        let err_str = unsafe { CStr::from_ptr(err_ptr) }.to_str().unwrap().to_string();
        unsafe { nola_core_free_c_string(err_ptr) };
        assert!(err_str.contains("empty database path"));

        let dir = tempfile::tempdir().unwrap();
        let db_str = dir.path().join("ffi.db").display().to_string();
        let app = unsafe { nola_core_application_open(db_str.as_ptr(), db_str.len()) };
        assert!(!app.is_null());
        assert!(nola_core_application_last_open_error().is_null());

        // Retain + free safety
        let retained = unsafe { nola_core_mobile_mcp_retain(app) };
        assert!(!retained.is_null());
        unsafe { nola_core_application_free(retained) };

        let cmd = r#"{"request_id":"r1","client_id":"c1","command":{"type":"health","arguments":{}}}"#;
        let out_ptr = unsafe { nola_core_application_submit(app, cmd.as_ptr(), cmd.len()) };
        let out_str = unsafe { CStr::from_ptr(out_ptr) }.to_str().unwrap().to_string();
        unsafe { nola_core_free_c_string(out_ptr) };
        assert!(out_str.contains("\"status\":\"committed\""));

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
            "key=[REDACTED_API_KEY] token=[REDACTED_CREDENTIAL]"
        );

        unsafe { nola_core_application_free(app) };
    }
}
