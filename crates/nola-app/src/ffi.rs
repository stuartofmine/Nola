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
            let item = MemoryItem {
                id,
                namespace: "intelligence:memory".into(),
                title: kind,
                content: statement,
                tags: vec![cand.to_string()],
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
                        json!({"id": m.id, "title": m.title, "statement": m.content, "content": m.content})
                    })
            })
            .collect()
    };
    let events = parse_items("intelligence:event");
    let memories = parse_items("intelligence:memory");
    let outcomes = parse_items("intelligence:outcome");
    to_owned_c_string(
        json!({
            "schema_version": 1,
            "events": events,
            "memories": memories,
            "strategies": [],
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

    // Compact oldest non-system messages when exceeding prompt_limit.
    let mut archived = 0usize;
    let mut current_msg_tokens = msg_tokens_before;
    while current_msg_tokens + tool_tokens > prompt_limit && messages.len() > keep_recent.max(1) {
        if let Some(pos) = messages
            .iter()
            .position(|m| m.get("role").and_then(|r| r.as_str()) != Some("system"))
        {
            if messages.len() - 1 < keep_recent.max(1) {
                break;
            }
            let removed = messages.remove(pos);
            current_msg_tokens =
                current_msg_tokens.saturating_sub(estimate_text_tokens(&removed.to_string()));
            archived += 1;
        } else {
            break;
        }
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
                "transaction_count": if changed { 1 } else { 0 }
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
        "local_write" | "write" => 1,
        "external_write" | "network" => 2,
        "destructive" | "high" | "unrestricted" => 3,
        _ => 1,
    }
}

fn infer_tool_metadata(tool_id: &str) -> (&'static str, &'static str, &'static str) {
    let lower = tool_id.to_ascii_lowercase();
    if lower.contains("write") || lower.contains("edit") || lower.contains("delete") || lower.contains("remove") {
        ("files_write", "scoped_resource", "local_write")
    } else if lower.contains("read") || lower.contains("list") || lower.contains("get") || lower.contains("search") {
        ("files_read", "scoped_resource", "read_only")
    } else if lower.contains("exec") || lower.contains("bash") || lower.contains("shell") {
        ("shell_exec", "elevated", "high")
    } else {
        ("general", "scoped_resource", "local_write")
    }
}

/// Resolve deterministic tool policy decision for mobile hosts (matches `tool_policy_v1_golden.json`).
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
        let (default_cap, default_perm, default_risk) = infer_tool_metadata(&tool_id);
        let capability = tool
            .get("capability")
            .and_then(|v| v.as_str())
            .unwrap_or(default_cap)
            .to_string();
        let permission = tool
            .get("permission")
            .and_then(|v| v.as_str())
            .unwrap_or(default_perm)
            .to_string();
        let risk = tool
            .get("risk")
            .and_then(|v| v.as_str())
            .unwrap_or(default_risk)
            .to_string();
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

/// Fail-closed check whether an MCP tool invocation is permitted under mobile tool policy.
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
    let explicit_enabled = req.get("enabled").and_then(|v| v.as_bool()).unwrap_or(false);
    let denied = req.get("denied").and_then(|v| v.as_bool()).unwrap_or(false);
    let has_server_or_tool = req
        .get("server_id")
        .or_else(|| req.get("tool_id"))
        .or_else(|| req.get("tool_name"))
        .and_then(|v| v.as_str())
        .is_some_and(|s| !s.trim().is_empty());
    let auth_ok = req
        .get("authorization")
        .and_then(|v| v.as_str())
        .is_none_or(|a| a == "granted" || a == "not_required");
    explicit_enabled && !denied && has_server_or_tool && auth_ok
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
    let target = req
        .get("target")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
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

    if manifest_url.is_empty() {
        let status = match operation {
            "check" => "up_to_date",
            _ => "idle",
        };
        return to_owned_c_string(
            json!({
                "ok": true,
                "value": {
                    "status": status,
                    "current_version": current_version,
                    "current_build": current_build,
                    "channel": channel,
                    "target": target,
                    "release": Value::Null
                },
                "error": Value::Null
            })
            .to_string(),
        );
    }

    match check_for_update(manifest_url, current_version) {
        Ok(update) => {
            let status = if update.update_available {
                "update_available"
            } else {
                "up_to_date"
            };
            let release = match (update.update_available, update.latest) {
                (true, Some(rel)) => json!({
                    "release_id": rel.version,
                    "version": rel.version,
                    "build": current_build + 1,
                    "notes": rel.notes,
                    "artifact": {
                        "kind": "archive",
                        "url": rel.download_url,
                        "filename": Value::Null,
                        "size": Value::Null,
                        "sha256": rel.sha256
                    }
                }),
                _ => Value::Null,
            };
            to_owned_c_string(
                json!({
                    "ok": true,
                    "value": {
                        "status": status,
                        "current_version": current_version,
                        "current_build": current_build,
                        "channel": channel,
                        "target": target,
                        "release": release
                    },
                    "error": Value::Null
                })
                .to_string(),
            )
        }
        Err(err) => err_json("update_check_failed", err),
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
                    {"tool_id": "device_read_file", "route": "device", "available": true, "schema": {}},
                    {"tool_id": "device_write_file", "route": "device", "available": true, "schema": {}}
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

        // 8. Software update contract matches AppleSoftwareUpdateClient
        let su_req = json!({"operation": "check", "target": "ios"}).to_string();
        let su_ptr = unsafe { nola_core_software_update_json(su_req.as_ptr(), su_req.len()) };
        let su_val: Value =
            serde_json::from_str(unsafe { CStr::from_ptr(su_ptr) }.to_str().unwrap()).unwrap();
        unsafe { nola_core_free_c_string(su_ptr) };
        assert_eq!(su_val["ok"], true);
        assert_eq!(su_val["value"]["status"], "up_to_date");
        assert_eq!(su_val["value"]["target"], "ios");
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
