// SPDX-License-Identifier: Apache-2.0
//! Conversation context, permission state, projection building, and remote model profile resolution.

use std::collections::HashSet;
use nola_engine::load_session_usage;
use nola_modules::ModulesSuite;
use nola_store::Store;
use nola_types::{new_id, now_ms, sha256_hex, PermissionProfile, Session};
use serde_json::{json, Value};

use super::projection::{latest_projection_sequence, projection_event_scope_key};
use super::{extract_sub_command, lookup_active_run};

pub(super) fn dispatch_context_command(
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
            Ok(json!(count))
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

pub(super) fn read_permission_state(store: &Store, conv_id: Option<&str>) -> (String, i64) {
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

pub(super) fn load_conversation_settings_str(store: &Store, id: &str) -> Option<String> {
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

pub(super) fn build_context_usage_json(store: &Store, conv_id: &str, rev: i64) -> Value {
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

pub(super) fn build_conversation_projection_response(
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
    // Read the cursor before the transcript so a racing append cannot stamp an
    // older snapshot with a newer cursor. A rewind also advances this cursor.
    let history_cursor = store.latest_event_sequence(&sess.id).unwrap_or(0);
    let (raw_messages, events) = crate::web_api::reconstruct_v2_messages(store, &sess.id);
    let total_message_count = raw_messages.len() as i64;
    let raw_event_sequence = events
        .last()
        .and_then(|e| e.get("seq").and_then(|v| v.as_i64()))
        .unwrap_or(0);
    let proj_sequence = latest_projection_sequence(
        store,
        &projection_event_scope_key("conversation", Some(&sess.id)),
    ) as i64;
    let event_sequence = raw_event_sequence.max(history_cursor).max(proj_sequence);

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
            let state = match m["status"].as_str() {
                Some("failed") => "failed",
                Some("cancelled") => "cancelled",
                _ => "completed",
            };
            let failure = m["error"].as_str().map(ToOwned::to_owned);
            if let Some(obj) = m.as_object_mut() {
                obj.insert("id".into(), json!(msg_id));
                obj.insert("message_id".into(), json!(msg_id));
                obj.insert("role".into(), json!(role));
                obj.insert("state".into(), json!(state));
                obj.insert("status".into(), json!(state));
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
                        if failure.as_deref() != Some(content.as_str()) {
                            parts.push(json!({
                                "kind": "text", "part_id": part_id, "revision": 1,
                                "markdown": content, "is_final": true
                            }));
                        }
                        if let Some(detail) = failure {
                            let (code, display_key) = if state == "cancelled" {
                                ("run_cancelled", "Response cancelled")
                            } else {
                                ("run_failed", "Response failed")
                            };
                            parts.push(json!({
                                "kind": "error", "part_id": format!("{msg_id}:error"),
                                "revision": 1, "code": code, "display_key": display_key,
                                "message": detail, "recoverable": false
                            }));
                        }
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
                if !already_present && !has_more {
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

    let visible_assistants: HashSet<&str> = canonical_messages
        .iter()
        .filter(|message| message["role"] == "assistant")
        .filter_map(|message| message["message_id"].as_str().or_else(|| message["id"].as_str()))
        .collect();
    let mut assistant_journal_heads = store
        .get_kv(&format!("journal_heads_by_conv:{}", sess.id))
        .ok()
        .flatten()
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();
    assistant_journal_heads.retain(|head| {
        head["identity"]["assistant_message_id"]
            .as_str()
            .is_some_and(|id| visible_assistants.contains(id))
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
        "stream_owners": Value::Null,
        "event_sequence": event_sequence,
        "total_message_count": total_message_count,
        "before_ordinal": next_before_ordinal,
        "assistant_journal_heads": assistant_journal_heads,
        "deferred_text_parts": []
    })
}

pub(super) fn resolve_remote_model_profile(
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

