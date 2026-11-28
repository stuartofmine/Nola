// SPDX-License-Identifier: Apache-2.0
//! Paired mobile/remote host protocol helpers, task cancellation/guidance, resources, and tool plans.

use std::collections::{BTreeMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use nola_modules::ModulesSuite;
use nola_store::{EventPage, Store};
use nola_tools::ToolRegistry;
use nola_types::{new_id, now_ms, sha256_hex};
use serde_json::{json, Value};

use crate::web_api::ensure_source_store_epoch;

use super::auth::{base64_decode, base64_encode};

pub(super) fn has_expected_host_id(body: &Value) -> bool {
    body.get("expected_host_id")
        .and_then(|v| v.as_str())
        .is_some_and(|s| !s.trim().is_empty())
}

pub(super) fn wrap_paired_result(store: &Store, host_id: &str, body: &Value, result: Value) -> Value {
    if has_expected_host_id(body) {
        let source_store_epoch = ensure_source_store_epoch(store);
        json!({
            "source_host_id": host_id,
            "source_store_epoch": source_store_epoch,
            "result": result
        })
    } else {
        result
    }
}


pub(super) fn is_read_only_post(body: &Value) -> bool {
    let Some(obj) = body.as_object() else {
        return true;
    };
    if obj.is_empty() {
        return true;
    }
    if let Some(op) = obj
        .get("operation")
        .or_else(|| obj.get("action"))
        .and_then(|v| v.as_str())
    {
        if matches!(
            op.to_ascii_lowercase().as_str(),
            "read" | "get" | "fetch" | "list" | "journal" | "query" | "history"
        ) {
            return true;
        }
    }
    let mutation_keys = [
        "title",
        "isPinned",
        "selectedModelSource",
        "selectedModelId",
        "permissionProfile",
        "permission_profile",
        "reasoningEffort",
        "knowledge",
        "expectedRevision",
        "expected_revision",
        "content",
        "message",
        "text",
        "parts",
        "prompt",
        "user_message",
        "attachments",
    ];
    !mutation_keys.iter().any(|k| obj.contains_key(*k))
}

pub(super) fn is_conversations_list_read_post(body: &Value) -> bool {
    let Some(obj) = body.as_object() else {
        return false;
    };
    let explicit_op = obj
        .get("operation")
        .or_else(|| obj.get("action"))
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_ascii_lowercase());
    if let Some(ref op) = explicit_op {
        if matches!(
            op.as_str(),
            "read" | "get" | "fetch" | "list" | "journal" | "query"
        ) {
            return true;
        }
        if matches!(op.as_str(), "create" | "new" | "upsert") {
            return false;
        }
    }
    let has_query_marker = obj.contains_key("journal")
        || obj.contains_key("limit")
        || obj.contains_key("cursor")
        || obj.contains_key("after_seq")
        || obj.contains_key("afterSeq");
    if has_query_marker {
        return true;
    }
    if has_expected_host_id(body) {
        let has_create_title = obj
            .get("title")
            .and_then(|v| v.as_str())
            .is_some_and(|s| !s.trim().is_empty());
        return !has_create_title;
    }
    false
}

#[cfg(test)]
pub(super) fn is_messages_read_post(body: &Value) -> bool {
    is_messages_read_post_with_accept(body, false)
}

pub(super) fn is_messages_read_post_with_accept(body: &Value, accepts_sse: bool) -> bool {
    let Some(obj) = body.as_object() else {
        return true;
    };
    if let Some(op) = obj
        .get("operation")
        .or_else(|| obj.get("action"))
        .and_then(|v| v.as_str())
    {
        if matches!(
            op.to_ascii_lowercase().as_str(),
            "read" | "get" | "fetch" | "list" | "history" | "query"
        ) {
            return true;
        }
    }
    // macOS remote chat sends `messages: [...]`; iOS `resumeObservedRun` sends `expected_run_id`
    // with `Accept: text/event-stream`. Neither should be hijacked into JSON history reads!
    if obj
        .get("messages")
        .and_then(|v| v.as_array())
        .is_some_and(|a| !a.is_empty())
    {
        return false;
    }
    if accepts_sse && obj.get("expected_run_id").is_some() {
        return false;
    }
    let has_text = ["content", "message", "text", "prompt", "user_message"]
        .iter()
        .any(|k| {
            obj.get(*k)
                .and_then(|v| v.as_str())
                .is_some_and(|s| !s.trim().is_empty())
        });
    let has_parts = obj
        .get("parts")
        .and_then(|v| v.as_array())
        .is_some_and(|a| !a.is_empty());
    let has_attachments = obj
        .get("attachments")
        .and_then(|v| v.as_array())
        .is_some_and(|a| !a.is_empty());
    if has_text || has_parts || has_attachments {
        return false;
    }
    is_read_only_post(body) || has_expected_host_id(body)
}


pub(super) fn wrap_paired_envelope(store: &Store, host_id: &str, result: Value) -> Value {
    let source_store_epoch = ensure_source_store_epoch(store);
    json!({
        "source_host_id": host_id,
        "source_store_epoch": source_store_epoch,
        "result": result
    })
}

pub(super) fn is_assistant_journal_operation(body: &Value) -> bool {
    let Some(op) = body.get("operation").and_then(|v| v.as_str()) else {
        return false;
    };
    match op {
        "open_snapshot"
        | "release_snapshot"
        | "terminal_seal"
        | "terminal_manifest"
        | "admission_head"
        | "admissions"
        | "context_usage"
        | "heads"
        | "parts"
        | "changes"
        | "range"
        | "record_chunk" => true,
        "read" => body.get("after").is_some() || body.get("through").is_some(),
        _ => false,
    }
}

pub(super) fn is_paired_journal_operation(body: &Value) -> bool {
    if is_assistant_journal_operation(body) {
        return true;
    }
    let Some(op) = body.get("operation").and_then(|v| v.as_str()) else {
        return false;
    };
    if op == "task_child_window" {
        return true;
    }
    matches!(op, "prepare" | "observe" | "cancel")
        && body.get("identity").is_some_and(|v| v.is_object())
}

pub(super) fn handle_assistant_journal_http(
    store: &Store,
    suite: &ModulesSuite,
    host_id: &str,
    body: &Value,
    is_mobile: bool,
) -> Result<Value, (&'static str, String)> {
    let op = body
        .get("operation")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let daemon_cmd = match op {
        "open_snapshot" => "open_assistant_journal_snapshot",
        "release_snapshot" => "release_assistant_journal_snapshot",
        "terminal_seal" => "local_assistant_run_terminal_seal",
        "terminal_manifest" => "read_local_assistant_run_terminal_manifest",
        "admission_head" => "local_run_message_admission_head",
        "admissions" => "read_run_message_admissions",
        "context_usage" => "local_run_context_usage",
        "heads" => "local_run_assistant_journal_heads",
        "read" => "read_assistant_journal",
        "parts" => "read_assistant_journal_parts",
        "changes" => "read_assistant_journal_part_changes",
        "range" => "read_assistant_journal_range",
        "record_chunk" => "read_assistant_journal_record_chunk",
        other => {
            return Err((
                "unsupported_journal_operation",
                format!("unsupported assistant journal operation '{other}'"),
            ))
        }
    };
    let result = crate::daemon::dispatch_command(store, suite, daemon_cmd, body)?;
    if is_mobile || has_expected_host_id(body) || is_paired_journal_operation(body) {
        Ok(wrap_paired_envelope(store, host_id, result))
    } else {
        Ok(result)
    }
}

pub(super) fn is_paired_task_cancellation_body(body: &Value) -> bool {
    let Some(obj) = body.as_object() else {
        return false;
    };
    let has_task = obj
        .get("taskId")
        .or_else(|| obj.get("task_id"))
        .and_then(|v| v.as_str())
        .is_some_and(|s| !s.trim().is_empty());
    let has_paired_field = obj.contains_key("hostId")
        || obj.contains_key("expected_host_id")
        || obj.contains_key("parentRunId")
        || obj.contains_key("parent_run_id")
        || obj.contains_key("receipt_only")
        || obj.contains_key("receiptOnly");
    has_task || has_paired_field
}

pub(super) fn handle_paired_task_cancellation(
    store: &Store,
    suite: &ModulesSuite,
    active_cancels: &Arc<Mutex<BTreeMap<String, Arc<AtomicBool>>>>,
    host_id: &str,
    conv_id: &str,
    url_task_id: Option<&str>,
    body: &Value,
) -> Value {
    let req_host_id = body
        .get("hostId")
        .or_else(|| body.get("expected_host_id"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(host_id);
    let task_id = url_task_id
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            body.get("taskId")
                .or_else(|| body.get("task_id"))
                .and_then(|v| v.as_str())
        })
        .unwrap_or("default");
    let parent_run_id = body
        .get("parentRunId")
        .or_else(|| body.get("parent_run_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let receipt_only = body
        .get("receipt_only")
        .or_else(|| body.get("receiptOnly"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let mut task_rec = suite.get_record::<Value>("runtime_task", task_id).ok().flatten();
    if !receipt_only {
        if let Some(flag) = active_cancels.lock().unwrap().get(conv_id) {
            flag.store(true, Ordering::SeqCst);
        }
        let ts = now_ms();
        if let Some(ref mut task) = task_rec {
            if let Some(obj) = task.as_object_mut() {
                obj.insert("cancel_requested".into(), json!(true));
                obj.insert("updated_at_ms".into(), json!(ts));
            }
            let _ = suite.upsert_record("runtime_task", task_id, task);
        }
        let _ = store.set_kv(
            &format!("cancel_requested:{task_id}"),
            &json!({"task_id": task_id, "cancel_requested": true, "requested_at_ms": ts}),
        );
    }

    let raw_state = task_rec
        .as_ref()
        .and_then(|t| t.get("status").or_else(|| t.get("state")).and_then(|v| v.as_str()))
        .unwrap_or("cancelled");
    let state = match raw_state {
        "completed" | "sealed" => "sealed",
        "failed" | "error" => "failed",
        "cancelled" | "canceled" => "cancelled",
        "queued" => "queued",
        "running" => "running",
        "retry_waiting" => "retry_waiting",
        "waiting_permission" => "waiting_permission",
        "suspended" => "suspended",
        "awaiting_reconciliation" => "awaiting_reconciliation",
        _ => "cancelled",
    };
    let terminal = matches!(state, "sealed" | "failed" | "cancelled");
    let cancel_requested = if !receipt_only {
        true
    } else {
        task_rec
            .as_ref()
            .and_then(|t| t.get("cancel_requested").and_then(|v| v.as_bool()))
            .unwrap_or_else(|| {
                store
                    .get_kv(&format!("cancel_requested:{task_id}"))
                    .ok()
                    .flatten()
                    .and_then(|v| v.get("cancel_requested").and_then(|b| b.as_bool()))
                    .unwrap_or(terminal)
            })
    };

    // Strict 8-key RemoteTaskCancellation / AppleTaskCancellationReceipt contract
    json!({
        "hostId": req_host_id,
        "conversationId": conv_id,
        "parentRunId": parent_run_id,
        "taskId": task_id,
        "cancelRequested": cancel_requested,
        "terminal": terminal,
        "state": state,
        "applicationPending": false
    })
}

pub(super) fn is_task_guidance_post(body: &Value, accepts_sse: bool) -> bool {
    if accepts_sse {
        return false;
    }
    let Some(obj) = body.as_object() else {
        return false;
    };
    let has_task = obj
        .get("taskId")
        .or_else(|| obj.get("task_id"))
        .and_then(|v| v.as_str())
        .is_some_and(|s| !s.trim().is_empty());
    let has_guidance_shape = obj.contains_key("guidance")
        || obj.contains_key("messageId")
        || obj.contains_key("message_id")
        || obj.contains_key("parentRunId")
        || obj.contains_key("parent_run_id");
    has_task && has_guidance_shape
}

pub(super) fn handle_task_guidance_http(
    store: &Store,
    host_id: &str,
    conv_id: &str,
    body: &Value,
    receipt_only: bool,
) -> Value {
    let req_host_id = body
        .get("hostId")
        .or_else(|| body.get("expected_host_id"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(host_id);
    let task_id = body
        .get("taskId")
        .or_else(|| body.get("task_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let message_id = body
        .get("messageId")
        .or_else(|| body.get("message_id"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(new_id);
    let parent_run_id = body
        .get("parentRunId")
        .or_else(|| body.get("parent_run_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let submitted_at_ms = body
        .get("submittedAtMs")
        .or_else(|| body.get("submitted_at_ms"))
        .and_then(|v| v.as_i64())
        .unwrap_or_else(now_ms);
    let content = body
        .get("message")
        .or_else(|| body.get("guidance"))
        .or_else(|| body.get("content"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let key = format!("task_guidance_receipt:{task_id}:{message_id}");
    let existing = store.get_kv(&key).ok().flatten();
    let guidance_val = if receipt_only {
        existing.unwrap_or(Value::Null)
    } else if let Some(found) = existing {
        found
    } else {
        let rec = json!({
            "task_id": task_id,
            "message_id": message_id,
            "owner_session_id": conv_id,
            "parent_turn_id": parent_run_id,
            "content": content,
            "submitted_at_ms": submitted_at_ms,
            "status": "accepted"
        });
        let _ = store.set_kv(&key, &rec);
        let _ = store.set_kv(&format!("task_guidance_receipt:{message_id}"), &rec);
        let _ = store.set_kv(&format!("task_guidance_receipt:{task_id}"), &rec);
        rec
    };

    json!({
        "hostId": req_host_id,
        "conversationId": conv_id,
        "taskId": task_id,
        "messageId": message_id,
        "guidance": guidance_val
    })
}

pub(super) fn handle_remote_submission_reconciliation(
    store: &Store,
    active_cancels: &Arc<Mutex<BTreeMap<String, Arc<AtomicBool>>>>,
    _host_id: &str,
    conv_id: &str,
    body: &Value,
) -> Value {
    let op = body
        .get("operation")
        .and_then(|v| v.as_str())
        .unwrap_or("observe");
    let identity = body.get("identity").cloned().unwrap_or_else(|| json!({}));
    let raw_digest = body
        .get("request_digest")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let sub_id = identity
        .get("submission_id")
        .and_then(|v| v.as_str())
        .unwrap_or("default");
    let asst_id = identity
        .get("assistant_message_id")
        .and_then(|v| v.as_str())
        .unwrap_or("assistant");
    let key = format!("remote_sub_server_receipt:{conv_id}:{sub_id}");

    if op == "prepare" {
        let computed_digest = if raw_digest.len() == 64
            && raw_digest
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            raw_digest.to_string()
        } else {
            let req_canon = serde_json::to_string(
                body.get("request").unwrap_or(&Value::Null),
            )
            .unwrap_or_default();
            let id_canon = serde_json::to_string(&identity).unwrap_or_default();
            sha256_hex(format!("{conv_id}:{id_canon}:{req_canon}").as_bytes())
        };
        let receipt = json!({
            "protocol_version": 1,
            "identity": identity,
            "request_digest": computed_digest,
            "phase": "prepared",
            "admission": Value::Null,
            "rejection_code": Value::Null
        });
        let _ = store.set_kv(&key, &receipt);
        return receipt;
    }

    let request_digest = raw_digest;

    if op == "cancel" {
        if let Some(flag) = active_cancels.lock().unwrap().get(conv_id) {
            flag.store(true, Ordering::SeqCst);
        }
        let receipt = json!({
            "protocol_version": 1,
            "identity": identity,
            "request_digest": request_digest,
            "phase": "cancelled",
            "admission": Value::Null,
            "rejection_code": Value::Null
        });
        let _ = store.set_kv(&key, &receipt);
        return receipt;
    }

    if let Ok(Some(saved)) = store.get_kv(&key) {
        if saved.get("request_digest").and_then(|v| v.as_str()) == Some(request_digest) {
            return saved;
        }
    }

    let has_session = store.get_session(conv_id).ok().flatten().is_some();
    if has_session {
        let epoch = ensure_source_store_epoch(store);
        let record_digest = nola_types::sha256_hex(
            format!("{conv_id}:{sub_id}:{asst_id}:{request_digest}").as_bytes(),
        );
        let receipt = json!({
            "protocol_version": 1,
            "identity": identity,
            "request_digest": request_digest,
            "phase": "admitted",
            "admission": {
                "identity": {
                    "conversation_id": conv_id,
                    "run_id": sub_id,
                    "assistant_message_id": asst_id,
                    "source_store_epoch": epoch
                },
                "sequence": "1",
                "record_digest": record_digest
            },
            "rejection_code": Value::Null
        });
        let _ = store.set_kv(&key, &receipt);
        receipt
    } else {
        json!({
            "protocol_version": 1,
            "identity": identity,
            "request_digest": request_digest,
            "phase": "prepared",
            "admission": Value::Null,
            "rejection_code": Value::Null
        })
    }
}

pub(super) fn percent_encode_path_segment(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for b in input.bytes() {
        let unreserved = b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~');
        if unreserved {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

pub(super) fn extract_resource_raw_bytes(res: &Value) -> Option<Vec<u8>> {
    if let Some(b64) = res
        .get("data_base64")
        .or_else(|| res.get("dataBase64"))
        .or_else(|| res.get("content_base64"))
        .or_else(|| res.get("data"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
    {
        let cleaned = b64
            .split_once(",")
            .map(|(_, rest)| rest)
            .unwrap_or(b64);
        if let Ok(bytes) = base64_decode(cleaned) {
            return Some(bytes);
        }
    }
    if let Some(data_url) = res
        .get("dataUrl")
        .or_else(|| res.get("data_url"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
    {
        if let Some((_, b64)) = data_url.split_once(",") {
            if let Ok(bytes) = base64_decode(b64) {
                return Some(bytes);
            }
        }
    }
    if let Some(text) = res
        .get("extracted_text")
        .or_else(|| res.get("text"))
        .or_else(|| res.get("content"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        return Some(text.as_bytes().to_vec());
    }
    None
}

pub(super) fn normalize_canonical_resource(conv_id: &str, rid: &str, raw: &Value, include_data: bool) -> Value {
    let display_name = raw
        .get("displayName")
        .or_else(|| raw.get("filename"))
        .or_else(|| raw.get("name"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("document.pdf");
    let display_name: String = display_name.chars().take(256).collect();
    let mime_type = raw
        .get("mimeType")
        .or_else(|| raw.get("mime_type"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| {
            if display_name.to_ascii_lowercase().ends_with(".pdf") {
                "application/pdf"
            } else {
                "application/octet-stream"
            }
        });
    let raw_bytes = extract_resource_raw_bytes(raw);
    let byte_count = raw
        .get("byteCount")
        .or_else(|| raw.get("size_bytes"))
        .or_else(|| raw.get("size"))
        .and_then(|v| v.as_u64())
        .filter(|&n| n > 0)
        .or_else(|| raw_bytes.as_ref().map(|b| b.len() as u64))
        .unwrap_or(1);
    let sha256 = raw
        .get("sha256")
        .and_then(|v| v.as_str())
        .filter(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
        .map(ToString::to_string)
        .unwrap_or_else(|| {
            if let Some(ref b) = raw_bytes {
                nola_types::sha256_hex(b)
            } else {
                nola_types::sha256_hex(format!("{conv_id}:{rid}:{display_name}").as_bytes())
            }
        });
    let preview_url = format!(
        "/api/chat/conversations/{}/resources/{}",
        percent_encode_path_segment(conv_id),
        percent_encode_path_segment(rid)
    );

    let mut obj = raw.as_object().cloned().unwrap_or_default();
    if !include_data {
        obj.remove("data_base64");
        obj.remove("dataBase64");
        obj.remove("content_base64");
        obj.remove("dataUrl");
        obj.remove("data_url");
        obj.remove("data");
    }
    obj.insert("id".into(), json!(rid));
    obj.insert("resourceId".into(), json!(rid));
    obj.insert("conversationId".into(), json!(conv_id));
    obj.insert("displayName".into(), json!(display_name));
    obj.insert("mimeType".into(), json!(mime_type));
    obj.insert("byteCount".into(), json!(byte_count));
    obj.insert("sha256".into(), json!(sha256));
    obj.insert("previewUrl".into(), json!(preview_url));
    Value::Object(obj)
}

pub(super) fn store_conversation_resource_upload(
    store: &Store,
    conv_id: &str,
    body: &Value,
) -> Result<Value, String> {
    let display_name = body
        .get("displayName")
        .or_else(|| body.get("filename"))
        .or_else(|| body.get("name"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("document.pdf");
    let b64 = body
        .get("dataBase64")
        .or_else(|| body.get("data_base64"))
        .or_else(|| body.get("content_base64"))
        .or_else(|| body.get("base64"))
        .or_else(|| body.get("data"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing dataBase64".to_string())?;
    let bytes = base64_decode(b64)?;
    if bytes.is_empty() || bytes.len() > 8 * 1024 * 1024 {
        return Err("resource size must be between 1 byte and 8 MiB".into());
    }
    let resource_id = body
        .get("resourceId")
        .or_else(|| body.get("resource_id"))
        .or_else(|| body.get("id"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(new_id);
    let mime_type = body
        .get("mimeType")
        .or_else(|| body.get("mime_type"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("application/pdf");
    let digest = nola_types::sha256_hex(&bytes);
    let ts = now_ms();
    let record = json!({
        "resource_id": resource_id,
        "resourceId": resource_id,
        "id": resource_id,
        "conversation_id": conv_id,
        "conversationId": conv_id,
        "kind": "pdf",
        "mime_type": mime_type,
        "mimeType": mime_type,
        "filename": display_name,
        "displayName": display_name,
        "size_bytes": bytes.len() as u64,
        "byteCount": bytes.len() as u64,
        "sha256": digest,
        "data_base64": base64_encode(&bytes),
        "created_at_ms": ts,
        "updated_at_ms": ts
    });
    store
        .put_module_record(
            "conversation_resource",
            &format!("{conv_id}:{resource_id}"),
            &record,
        )
        .map_err(|e| e.to_string())?;
    Ok(normalize_canonical_resource(conv_id, &resource_id, &record, false))
}

pub(super) fn collect_conversation_resources_inner(
    store: &Store,
    conv_id: &str,
    include_data: bool,
) -> Vec<Value> {
    let mut resources: Vec<Value> = Vec::new();
    let mut seen_ids: HashSet<String> = HashSet::new();

    if let Ok(module_recs) = store.list_module_records::<Value>("conversation_resource") {
        for (idx, res) in module_recs.into_iter().enumerate() {
            let matches_conv = res
                .get("conversation_id")
                .or_else(|| res.get("conversationId"))
                .and_then(|v| v.as_str())
                == Some(conv_id);
            if !matches_conv {
                continue;
            }
            let rid = res
                .get("resourceId")
                .or_else(|| res.get("resource_id"))
                .or_else(|| res.get("id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(|| format!("mod-res-{idx}"));
            if seen_ids.insert(rid.clone()) {
                resources.push(normalize_canonical_resource(conv_id, &rid, &res, include_data));
            }
        }
    }

    if let Ok(Some(Value::Array(saved))) = store.get_kv(&format!("conv_resources:{conv_id}")) {
        for (idx, res) in saved.into_iter().enumerate() {
            let rid = res
                .get("resourceId")
                .or_else(|| res.get("id"))
                .or_else(|| res.get("resource_id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(|| format!("kv-res-{idx}"));
            if seen_ids.insert(rid.clone()) {
                resources.push(normalize_canonical_resource(conv_id, &rid, &res, include_data));
            }
        }
    }

    if let Ok(EventPage::Events(events)) = store.events_after(conv_id, 0, 2000) {
        for ev in events {
            if let Some(attachments) = ev.payload.get("attachments").and_then(|v| v.as_array()) {
                for (idx, att) in attachments.iter().enumerate() {
                    let rid = att
                        .get("resourceId")
                        .or_else(|| att.get("id"))
                        .or_else(|| att.get("resource_id"))
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.trim().is_empty())
                        .map(ToString::to_string)
                        .unwrap_or_else(|| format!("res-{}-{idx}", ev.seq));
                    if seen_ids.insert(rid.clone()) {
                        let mut entry = att.clone();
                        if let Some(obj) = entry.as_object_mut() {
                            obj.entry("seq".to_string()).or_insert_with(|| json!(ev.seq));
                        }
                        resources.push(normalize_canonical_resource(
                            conv_id,
                            &rid,
                            &entry,
                            include_data,
                        ));
                    }
                }
            }
            if let Some(res) = ev.payload.get("resource").filter(|v| v.is_object()) {
                let rid = res
                    .get("resourceId")
                    .or_else(|| res.get("id"))
                    .or_else(|| res.get("resource_id"))
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
                    .map(ToString::to_string)
                    .unwrap_or_else(|| format!("res-{}", ev.seq));
                if seen_ids.insert(rid.clone()) {
                    resources.push(normalize_canonical_resource(
                        conv_id,
                        &rid,
                        res,
                        include_data,
                    ));
                }
            }
        }
    }
    resources
}

pub(super) fn collect_conversation_resources_raw(store: &Store, conv_id: &str) -> Vec<Value> {
    collect_conversation_resources_inner(store, conv_id, true)
}

pub(super) fn collect_conversation_resources(store: &Store, conv_id: &str) -> Vec<Value> {
    collect_conversation_resources_inner(store, conv_id, false)
}


pub(super) fn build_paired_tool_plan(
    suite: &Arc<ModulesSuite>,
    host_id: &str,
    body: &Value,
    manifest_id: String,
) -> Value {
    let mut reg = ToolRegistry::with_builtin_tools();
    suite.register_tools(&mut reg);
    let specs = reg.all_specs();
    let issued_at_ms = now_ms();
    let expires_at_ms = issued_at_ms.saturating_add(300_000);
    let client_id = body
        .get("client_id")
        .or_else(|| body.get("clientId"))
        .or_else(|| body.pointer("/reference/client_id"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("nola-client");
    let session_id = body
        .get("session_id")
        .or_else(|| body.get("sessionId"))
        .or_else(|| body.get("conversation_id"))
        .or_else(|| body.pointer("/reference/session_id"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("default-session");
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
}

pub(super) fn parse_permission_resolution(body: &Value) -> (bool, &'static str) {
    if let Some(decision) = body
        .pointer("/resolution/decision")
        .or_else(|| body.get("decision"))
        .and_then(|v| v.as_str())
    {
        match decision.trim().to_ascii_lowercase().as_str() {
            "allow_once" | "approve_once" => return (true, "once"),
            "allow_conversation" | "allow_session" | "allow_always" => return (true, "session"),
            "deny" | "reject" => return (false, "once"),
            _ => {}
        }
    }
    let allow = body
        .get("allow")
        .or_else(|| body.get("approved"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let scope = match body
        .get("scope")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_ascii_lowercase())
        .as_deref()
    {
        Some("session") | Some("conversation") | Some("always") => "session",
        _ => "once",
    };
    (allow, scope)
}

