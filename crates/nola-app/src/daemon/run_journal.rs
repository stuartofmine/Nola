// SPDX-License-Identifier: Apache-2.0
//! Source submissions, assistant turn/host request journal, terminal seals, and delivery ownership.

use std::collections::HashSet;
use nola_modules::ModulesSuite;
use nola_store::Store;
use nola_types::{new_id, now_ms, sha256_hex, Session};
use serde_json::{json, Value};

use super::{canonical_json_value, clear_active_run, lookup_active_run, register_active_run};

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

pub(super) fn dispatch_register_source_submission(
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

pub(super) fn dispatch_reject_source_submission(
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

pub(super) fn dispatch_read_source_submission_receipt(
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

    let is_final_part = match kind {
        "text" => metadata_data
            .get("is_final")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        _ => metadata_data
            .get("state")
            .and_then(|v| v.as_str())
            .is_some_and(|s| s != "running" && s != "pending"),
    };

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

        // Keep at most 2 rolling intermediate root snapshots during streaming (so any 1-tick-behind
        // UI read still resolves byte-for-byte), and purge stale interim roots when this part finishes.
        let root_tracker_key = format!("{part_id}\0{field}");
        let mut tracked_roots: Vec<String> = run
            .get("journal_recent_roots")
            .and_then(|m| m.get(&root_tracker_key))
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(ToString::to_string))
                    .collect()
            })
            .unwrap_or_default();
        if tracked_roots.last().map(String::as_str) != Some(root_key.as_str()) {
            tracked_roots.push(root_key.clone());
        }
        let max_keep = if is_final_part { 1 } else { 2 };
        while tracked_roots.len() > max_keep {
            let old_key = tracked_roots.remove(0);
            if old_key != root_key {
                let _ = store.delete_kv(&old_key);
            }
        }
        if !run.get("journal_recent_roots").is_some_and(|v| v.is_object()) {
            run["journal_recent_roots"] = json!({});
        }
        if let Some(map) = run.get_mut("journal_recent_roots").and_then(|v| v.as_object_mut()) {
            map.insert(root_tracker_key, json!(tracked_roots));
        }

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
    // O(1) indexed per-record write (never O(N^2) read-append-rewrite of a multi-MB JSON array!).
    // During streaming deltas, reuse the active per-part slot so DB size stays O(parts), not O(ticks).
    let record_id = if is_final_part {
        let interim_id = format!("{conv_id}:{run_id}:live:{part_id}");
        let _ = store.delete_module_record("assistant_journal_record", &interim_id);
        format!("{conv_id}:{run_id}:{jseq:010}")
    } else {
        format!("{conv_id}:{run_id}:live:{part_id}")
    };
    let _ = store.put_module_record("assistant_journal_record", &record_id, &record);
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
        "cancelled" if run["blocked_model_effect"].is_string() => json!({
            "kind": "blocked", "reason": {"code": "model_stop_unconfirmed", "detail": Value::Null},
            "blocking_fact_refs": [run["blocked_model_effect"]], "resolution_condition": "verify_provider_outcome"
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

pub(super) fn dispatch_begin_assistant_turn(
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

pub(super) fn dispatch_claim_committed_host_requests(
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
    let interrupted_request = interrupted_model_request(&run);
    if let Some(request) = interrupted_request.filter(|r| model_delivery_never_started(&run, r)) {
        // Claim installs the lease before releasing a delivery to any worker.
        // A request without any lease or started attempt was never sent.
        reduce_host_response(store, &json!({"run_id": run_id, "response": {
            "request_id": request["request_id"], "payload": {"type": "model_cancelled_before_delivery"}
        }}), None)?;
        return Ok(json!({"run_id": run_id, "requests": []}));
    }
    let interrupted_model = interrupted_request.is_some();
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
            // Only the live owner can confirm its interrupted producer. A cold reader
            // inspects the old lease and explicitly closes it with its outcome unknown.
            if interrupted_model {
                continue;
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

pub(super) fn dispatch_load_committed_host_requests(
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

// A model-only interrupt can be closed explicitly after its producer disappears.
// A pending tool, a second delivery, or an unknown lease requires its own recovery.
fn interrupted_model_request(run: &Value) -> Option<&Value> {
    if run["status"] != "running" || run["turn_status"] != "cancelled" {
        return None;
    }
    let consumed = run["consumed_request_ids"].as_array();
    let mut pending = run["requests"].as_array()?.iter().filter(|request| {
        !consumed.is_some_and(|ids| ids.contains(&request["request_id"]))
    });
    let request = pending.next()?;
    (pending.next().is_none() && request.pointer("/payload/type")?.as_str()? == "model_stream"
        && request["effect_id"].is_string() && request["request_id"].is_string()).then_some(request)
}

fn model_delivery_never_started(run: &Value, request: &Value) -> bool {
    request["delivery"].is_null() && request["attempt_started"] != true
        && !run["started_attempt_request_ids"].as_array().is_some_and(|ids| ids.contains(&request["request_id"]))
}

pub(super) fn interrupted_model_inspection(
    store: &Store, conversation: &str, run_id: &str,
) -> Result<Option<Value>, (&'static str, String)> {
    let Some(run) = store.get_kv(&format!("committed_run:{run_id}")).map_err(|e| ("storage_error", e.to_string()))?
    else { return Ok(None) };
    let Some(request) = interrupted_model_request(&run) else { return Ok(None) };
    if lookup_active_run(store, conversation).as_ref().and_then(|v| v["run_id"].as_str()) != Some(run_id) {
        return Ok(None);
    }
    let mut delivery = request["delivery"].clone();
    if delivery.is_null() && !model_delivery_never_started(&run, request) {
        // Older recovery could already have released a started attempt. This is a
        // decision fence for that absent lease; it can never authorize a dispatch.
        delivery = json!({"attempt_id": request["attempt_id"], "worker_generation": "released",
            "lease_owner": "released", "lease_token": "released"});
    }
    if run["conversation_id"] != conversation || !run["invocation_config_digest"].is_string()
        || !["attempt_id", "worker_generation", "lease_owner", "lease_token"].iter()
            .all(|field| delivery[field].is_string()) {
        return Ok(None);
    }
    Ok(Some(json!({
        "conversation_id": conversation, "run_id": run_id, "assistant_message_id": run["assistant_message_id"],
        "invocation_config_digest": run["invocation_config_digest"], "retry_allowed": false, "independent_pending_effects": 0,
        "targets": [{"request_id": request["request_id"], "effect_id": request["effect_id"],
            "attempt_id": delivery["attempt_id"], "worker_generation": delivery["worker_generation"],
            "lease_owner": delivery["lease_owner"], "lease_token": delivery["lease_token"],
            "expected_attempt_revision": run["sequence"]}]
    })))
}

pub(super) fn preserve_unknown_model_outcome(store: &Store, decision: &Value) -> Result<Value, (&'static str, String)> {
    let conv = decision["conversation_id"].as_str().unwrap_or_default();
    let run_id = decision["run_id"].as_str().unwrap_or_default();
    let Some(plan) = interrupted_model_inspection(store, conv, run_id)? else {
        return Ok(json!({"outcome": "conflict", "code": "recovery_target_changed"}));
    };
    if plan["targets"] != decision["targets"] || plan["invocation_config_digest"] != decision["invocation_config_digest"]
        || decision.pointer("/choice/kind").and_then(Value::as_str) != Some("keep_unknown_blocked")
        || decision["resolution_id"].as_str().is_none_or(str::is_empty) {
        return Ok(json!({"outcome": "conflict", "code": "recovery_target_changed"}));
    }
    let args = json!({"conversation_id": conv, "run_id": run_id, "response": {
        "request_id": plan["targets"][0]["request_id"], "payload": {"type": "cancelled"}}});
    reduce_host_response(store, &args, Some(&plan["targets"][0]))?;
    Ok(json!({"outcome": "unknown_preserved_blocked", "resolution_id": decision["resolution_id"], "run_id": run_id}))
}

pub(super) fn dispatch_reduce_and_accept_host_response(
    store: &Store,
    args: &Value,
) -> Result<Value, (&'static str, String)> {
    reduce_host_response(store, args, None)
}

fn reduce_host_response(store: &Store, args: &Value, unknown: Option<&Value>) -> Result<Value, (&'static str, String)> {
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

    // A joined model producer can confirm its own interrupt on the original lease.
    // Never let that receipt retire a concurrent tool or a replacement delivery.
    let unissued_stop = ev_type == "model_cancelled_before_delivery"
        && interrupted_model_request(&run).is_some_and(|request| model_delivery_never_started(&run, request)
            && request["request_id"] == response["request_id"] && run["conversation_id"] == eff_conv)
        && lookup_active_run(store, &eff_conv).as_ref().and_then(|v| v["run_id"].as_str()) == Some(run_id);
    if unknown.is_none() && interrupted_model_request(&run).is_some() && ev_type != "cancellation_confirmed" && !unissued_stop {
        return Err(("conflict", "Model delivery was interrupted; await its stop or resolve its outcome".into()));
    }
    if let Some(target) = unknown {
        let request = interrupted_model_request(&run);
        if request.is_none_or(|request| request["request_id"] != target["request_id"]
            || request["effect_id"] != target["effect_id"] || !(request["delivery"]["lease_token"] == target["lease_token"]
                || (request["delivery"].is_null() && target["lease_token"] == "released" && request["attempt_id"] == target["attempt_id"])))
            || run["sequence"] != target["expected_attempt_revision"] {
            return Err(("conflict", "Model recovery target changed".into()));
        }
        run["blocked_model_effect"] = target["effect_id"].clone();
    }
    let model_confirmation = ev_type == "cancellation_confirmed" && run["requests"].as_array().into_iter().flatten()
        .find(|r| r["request_id"] == response["request_id"])
        .is_none_or(|r| r.pointer("/payload/type").and_then(Value::as_str) == Some("model_stream")
            || r["effect_id"] != response["effect_id"]);
    let ev_type = if model_confirmation {
        let request = interrupted_model_request(&run);
        if run["conversation_id"] != eff_conv
            || lookup_active_run(store, &eff_conv).as_ref().and_then(|v| v["run_id"].as_str()) != Some(run_id)
            || request.is_none_or(|request| request["request_id"] != response["request_id"]
            || request["effect_id"] != payload["target_effect_id"] || request["effect_id"] != response["effect_id"]
            || request["session_id"] != response["session_id"] || request["turn_id"] != response["turn_id"]
            || !["lease_token", "worker_generation", "lease_owner"].iter().all(|field|
                request["delivery"][field].is_string() && request["delivery"][field] == response["delivery"][field])) {
            return Err(("invalid_argument", "Cancellation does not match the outstanding model delivery".into()));
        }
        "cancelled"
    } else if unissued_stop {
        "cancelled"
    } else {
        ev_type
    };
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
                    .filter(|s| matches!(*s, "cautious" | "allow_all"))
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
                            .filter(|s| matches!(*s, "cautious" | "allow_all"))
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

pub(super) fn dispatch_reduce_and_commit_active_submission(
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

    if sub_type == "interrupt" || sub_type == "cancel" {
        let cancellation_id = submission
            .get("cancellation_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let reason = submission
            .get("reason")
            .and_then(|v| v.as_str())
            .unwrap_or("interrupted")
            .to_string();
        run["turn_status"] = json!("cancelled");
        run["interrupted"] = json!(true);
        run["cancellation_id"] = json!(cancellation_id);
        run["cancellation_reason"] = json!(reason);
        run["interrupted_at_ms"] = json!(input
            .get("committed_at_ms")
            .and_then(|v| v.as_i64())
            .unwrap_or_else(now_ms));
        if let Some(p_arr) = run.get_mut("pending_permissions").and_then(|v| v.as_array_mut()) {
            p_arr.clear();
        }
        if !conv_id.is_empty() {
            let _ = store.set_kv(
                &format!("cancel_requested:{conv_id}"),
                &json!({
                    "run_id": run_id,
                    "cancellation_id": run["cancellation_id"],
                    "reason": run["cancellation_reason"],
                    "requested_at_ms": run["interrupted_at_ms"]
                }),
            );
        }
    } else if sub_type == "permission_response" {
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

pub(super) fn dispatch_local_run_message_admission_head(
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

pub(super) fn dispatch_read_run_message_admissions(
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

pub(super) fn dispatch_run_message_admission_reception_state(
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

pub(super) fn dispatch_receive_run_message_admissions(
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

pub(super) fn dispatch_local_run_assistant_journal_heads(
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

pub(super) fn dispatch_local_assistant_journal_head(
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

pub(super) fn dispatch_open_assistant_journal_snapshot(args: &Value) -> Result<Value, (&'static str, String)> {
    let at = args
        .get("at")
        .cloned()
        .ok_or(("invalid_argument", "missing 'at'".into()))?;
    Ok(json!({
        "snapshot_id": new_id(),
        "at": at
    }))
}

pub(super) fn dispatch_read_assistant_journal(
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
    let indexed = store
        .list_module_records_with_prefix::<Value>(
            "assistant_journal_record",
            &format!("{conv_id}:{run_id}:"),
        )
        .unwrap_or_default();
    let mut all = if !indexed.is_empty() {
        indexed
    } else {
        store
            .get_kv(&format!("journal_records:{conv_id}:{run_id}"))
            .ok()
            .flatten()
            .and_then(|v| v.as_array().cloned())
            .unwrap_or_default()
    };
    all.sort_by_key(|r| {
        r.pointer("/header/sequence")
            .and_then(|v| v.as_str().and_then(|x| x.parse::<u64>().ok()).or_else(|| v.as_u64()))
            .unwrap_or(0)
    });
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

pub(super) fn dispatch_read_assistant_journal_record_chunk(
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

pub(super) fn dispatch_assistant_journal_reception_state(
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

pub(super) fn decode_std_base64(input: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    let mut buf = 0u32;
    let mut bits = 0u8;
    for b in input.bytes() {
        let val = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' | b'\n' | b'\r' | b' ' | b'\t' => continue,
            _ => return None,
        };
        buf = (buf << 6) | (val as u32);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((buf >> bits) & 0xff) as u8);
        }
    }
    Some(out)
}

pub(super) fn dispatch_next_assistant_journal_content(
    store: &Store,
    args: &Value,
) -> Result<Value, (&'static str, String)> {
    let record = args.get("record").unwrap_or(&Value::Null);
    let identity = record.pointer("/header/identity").unwrap_or(&Value::Null);
    let conv_id = identity.get("conversation_id").and_then(|v| v.as_str()).unwrap_or("");
    let run_id = identity.get("run_id").and_then(|v| v.as_str()).unwrap_or("");
    let Some(head) = record.pointer("/batch/head") else {
        return Ok(Value::Null);
    };
    let part_id = head.get("part_id").and_then(|v| v.as_str()).unwrap_or("");
    if part_id.is_empty() || conv_id.is_empty() || run_id.is_empty() {
        return Ok(Value::Null);
    }
    if let Some(bodies) = head.get("bodies").and_then(|v| v.as_array()) {
        for body in bodies {
            let field = body.get("field").and_then(|v| v.as_str()).unwrap_or(part_id);
            let target_bytes = body
                .get("utf8_bytes")
                .and_then(|v| v.as_str().and_then(|s| s.parse::<u64>().ok()).or_else(|| v.as_u64()))
                .unwrap_or(0);
            if target_bytes == 0 {
                continue;
            }
            let content_key = format!("assistant_journal_content:{conv_id}:{run_id}:{part_id}:{field}");
            let staged_bytes = store
                .get_kv(&content_key)
                .ok()
                .flatten()
                .and_then(|v| v.get("total_utf8_bytes").and_then(|b| b.as_u64()))
                .unwrap_or(0);
            if staged_bytes < target_bytes {
                let gen = body
                    .get("content_generation")
                    .and_then(|v| v.as_str())
                    .unwrap_or("1");
                return Ok(json!({
                    "part_id": part_id,
                    "content": body,
                    "expected_prefix": {
                        "field": field,
                        "content_generation": gen,
                        "utf8_bytes": "0",
                        "chunk_count": "0",
                        "content_root": "0".repeat(64)
                    }
                }));
            }
        }
    }
    Ok(Value::Null)
}

pub(super) fn dispatch_receive_assistant_journal_record(
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
    let assistant_id = identity
        .get("assistant_message_id")
        .and_then(|v| v.as_str())
        .unwrap_or("default");
    let cursor = json!({
        "identity": identity,
        "sequence": header.get("sequence").cloned().unwrap_or_else(|| json!("1")),
        "record_digest": header.get("record_digest").cloned().unwrap_or_else(|| json!("0".repeat(64)))
    });
    if let Some(head) = record.pointer("/batch/head") {
        if let Some(pid) = head.get("part_id").and_then(|v| v.as_str()) {
            let mut stored_head = head.clone();
            if let Some(obj) = stored_head.as_object_mut() {
                obj.entry("identity".to_string())
                    .or_insert_with(|| identity.clone());
            }
            let _ = store.put_module_record(
                "assistant_journal_part",
                &format!("{conv_id}:{run_id}:{assistant_id}:{pid}"),
                &stored_head,
            );
        }
    }
    let _ = store.set_kv(&format!("received_journal:{source_host}:{conv_id}:{run_id}"), &cursor);
    let _ = store.set_kv(&format!("journal_head:{conv_id}:{run_id}"), &cursor);
    if !conv_id.is_empty() {
        let heads_key = format!("journal_heads_by_conv:{conv_id}");
        let mut heads = store
            .get_kv(&heads_key)
            .ok()
            .flatten()
            .and_then(|v| v.as_array().cloned())
            .unwrap_or_default();
        heads.retain(|h| {
            h.pointer("/identity/assistant_message_id")
                .and_then(|v| v.as_str())
                != Some(assistant_id)
        });
        heads.push(cursor.clone());
        let _ = store.set_kv(&heads_key, &Value::Array(heads));
    }
    Ok(json!({
        "applied": true,
        "cursor": cursor
    }))
}

pub(super) fn dispatch_local_assistant_run_terminal_seal(
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

pub(super) fn dispatch_read_local_assistant_run_terminal_manifest(
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

pub(super) fn dispatch_load_run_delivery_ownership(
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

pub(super) fn dispatch_load_runtime_effect_work_schedule(
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
        if interrupted_model_request(&run).is_some_and(|r| !model_delivery_never_started(&run, r)) {
            continue;
        }
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

pub(super) fn dispatch_recover_expired_run_deliveries(
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
    if interrupted_model_request(&run).is_some() {
        return Ok(json!({"ok": true, "run_id": run_id, "recovered_count": 0}));
    }
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

pub(super) fn dispatch_load_host_execution_state(
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

pub(super) fn dispatch_heartbeat_effect_delivery(
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

pub(super) fn dispatch_inspect_run_process_owners(
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

