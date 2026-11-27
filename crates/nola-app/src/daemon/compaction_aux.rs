// SPDX-License-Identifier: Apache-2.0
//! Conversation compaction maintenance and auxiliary source/invocation dispatchers.

use nola_store::Store;
use nola_types::sha256_hex;
use serde_json::{json, Value};

use super::canonical_json_value;

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

pub(super) fn dispatch_step_compaction_summary(args: &Value) -> Result<Value, (&'static str, String)> {
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

pub(super) fn dispatch_evaluate_compaction_maintenance(
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

pub(super) fn dispatch_load_auxiliary_source_slot(
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

pub(super) fn dispatch_auxiliary_invocation(
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
            if let Some(conv_id) = owner
                .get("conversation_id")
                .or_else(|| owner.get("id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
            {
                let idx_key = format!("aux_conv_sources:{conv_id}");
                let mut refs = store
                    .get_kv(&idx_key)
                    .ok()
                    .flatten()
                    .and_then(|v| v.as_array().cloned())
                    .unwrap_or_default();
                let entry = json!({
                    "source_host_id": source_host,
                    "operation_id": op_id,
                    "attempt_id": format!("{op_id}:physical")
                });
                let exists = refs.iter().any(|r| {
                    r.get("source_host_id").and_then(|v| v.as_str()) == Some(source_host)
                        && r.get("operation_id").and_then(|v| v.as_str()) == Some(op_id)
                });
                if !exists {
                    refs.push(entry);
                    let _ = store.set_kv(&idx_key, &Value::Array(refs));
                }
            }
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


pub(super) fn dispatch_list_conversation_auxiliary_sources(
    store: &Store,
    args: &Value,
) -> Result<Value, (&'static str, String)> {
    let conv_id = args
        .get("conversation_id")
        .and_then(|v| v.as_str())
        .ok_or(("invalid_argument", "missing 'conversation_id'".into()))?;
    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(16)
        .clamp(1, 16) as usize;
    let after_host = args.pointer("/after/source_host_id").and_then(|v| v.as_str());
    let after_op = args.pointer("/after/operation_id").and_then(|v| v.as_str());
    let mut refs: Vec<Value> = store
        .get_kv(&format!("aux_conv_sources:{conv_id}"))
        .ok()
        .flatten()
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();
    refs.sort_by(|a, b| {
        let ka = (
            a.get("source_host_id").and_then(|v| v.as_str()).unwrap_or(""),
            a.get("operation_id").and_then(|v| v.as_str()).unwrap_or(""),
        );
        let kb = (
            b.get("source_host_id").and_then(|v| v.as_str()).unwrap_or(""),
            b.get("operation_id").and_then(|v| v.as_str()).unwrap_or(""),
        );
        ka.cmp(&kb)
    });
    let filtered: Vec<Value> = refs
        .into_iter()
        .filter(|r| match (after_host, after_op) {
            (Some(ah), Some(ao)) => {
                let rh = r.get("source_host_id").and_then(|v| v.as_str()).unwrap_or("");
                let ro = r.get("operation_id").and_then(|v| v.as_str()).unwrap_or("");
                (rh, ro) > (ah, ao)
            }
            _ => true,
        })
        .collect();
    let has_more = filtered.len() > limit;
    let page: Vec<Value> = filtered.into_iter().take(limit).collect();
    let next_cursor = if has_more {
        page.last().map(|last| {
            json!({
                "source_host_id": last.get("source_host_id").cloned().unwrap_or(Value::Null),
                "operation_id": last.get("operation_id").cloned().unwrap_or(Value::Null)
            })
        })
    } else {
        None
    };
    Ok(json!({
        "references": page,
        "next_cursor": next_cursor
    }))
}

