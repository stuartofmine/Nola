// SPDX-License-Identifier: Apache-2.0
//! Projection event log/snapshots, part derivation, terminal reception, agent command queue, and task controls.

use chrono::Utc;
use nola_modules::ModulesSuite;
use nola_store::{EventPage, Store};
use nola_types::{new_id, now_ms};
use serde_json::{json, Value};

use super::context::build_conversation_projection_response;
use super::{clear_active_run, get_session_case_insensitive, lookup_active_run};

pub(super) fn projection_event_scope_key(scope_type: &str, conv_id: Option<&str>) -> String {
    match (scope_type, conv_id) {
        ("conversation", Some(cid)) if !cid.trim().is_empty() => format!("__proj__:{cid}"),
        _ => "__proj__:__conversations__".to_string(),
    }
}

pub(super) fn append_projection_event(store: &Store, conv_id: &str, mutation: Value) {
    let scope_key = if conv_id == "__conversations__" {
        "__proj__:__conversations__".to_string()
    } else {
        format!("__proj__:{conv_id}")
    };
    let payload = json!({
        "event_id": new_id(),
        "conversation_id": conv_id,
        "mutation": mutation,
        "occurred_at": Utc::now().to_rfc3339()
    });
    let _ = store.append_event(&scope_key, "projection", payload, 50_000);
}

pub(super) fn latest_projection_sequence(store: &Store, scope_key: &str) -> u64 {
    match store.events_after(scope_key, 0, 50_000) {
        Ok(EventPage::Events(ev)) => ev.last().map(|e| e.seq.max(0) as u64).unwrap_or(0),
        Ok(EventPage::ResyncRequired { latest_seq, .. }) => latest_seq.max(0) as u64,
        Err(_) => 0,
    }
}

pub(super) fn dispatch_projection_snapshot(
    store: &Store,
    suite: &ModulesSuite,
    args: &Value,
) -> Result<Value, (&'static str, String)> {
    let scope = args.get("scope");
    let scope_type = scope
        .and_then(|s| s.get("type"))
        .and_then(|v| v.as_str())
        .unwrap_or("conversations");
    let scope_conv_id = scope
        .and_then(|s| s.get("conversation_id"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty());

    if scope_type == "conversation" {
        let cid = scope_conv_id.ok_or((
            "invalid_argument",
            "projection_snapshot conversation scope missing 'conversation_id'".into(),
        ))?;
        let Some(sess) = get_session_case_insensitive(store, cid)
            .map_err(|e| ("storage_error", e.to_string()))?
        else {
            return Err(("not_found", format!("conversation '{cid}' not found")));
        };
        let page_offset = args
            .pointer("/page/offset")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;
        let page_limit = args
            .pointer("/page/limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(64)
            .clamp(1, 500) as usize;
        let full = build_conversation_projection_response(
            store,
            &sess,
            page_offset,
            Some(page_limit),
            None,
            true,
        );
        let rev = full
            .get("revision")
            .and_then(|v| v.as_u64())
            .unwrap_or(1)
            .max(1);
        let has_more = full
            .get("has_more")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let payload = full.get("projection").cloned().unwrap_or(full);
        let scope_key = projection_event_scope_key("conversation", Some(&sess.id));
        let seq = latest_projection_sequence(store, &scope_key);
        return Ok(json!({
            "revision": rev,
            "cursor": {
                "scope": {
                    "type": "conversation",
                    "conversation_id": sess.id
                },
                "sequence": seq
            },
            "records": [{
                "entity_type": "conversation_projection",
                "entity_id": sess.id,
                "revision": rev,
                "payload": payload
            }],
            "page": {
                "offset": page_offset as u64,
                "limit": page_limit,
                "has_more": has_more
            }
        }));
    }

    let sessions = store
        .list_sessions(200)
        .map_err(|e| ("storage_error", e.to_string()))?;
    let conversations: Vec<Value> = sessions
        .iter()
        .map(|s| crate::web_api::session_to_conversation_json(store, s))
        .collect();
    let records: Vec<Value> = sessions
        .iter()
        .zip(conversations.iter())
        .map(|(s, c)| {
            let rev = c.get("revision").and_then(|v| v.as_u64()).unwrap_or(1).max(1);
            let msg_count = c
                .get("messageCount")
                .or_else(|| c.get("message_count"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            json!({
                "entity_type": "conversation_summary",
                "entity_id": s.id,
                "revision": rev,
                "payload": {
                    "id": s.id,
                    "title": s.title,
                    "revision": rev as i64,
                    "message_count": msg_count,
                    "created_at_ms": s.created_at_ms,
                    "updated_at_ms": s.updated_at_ms
                }
            })
        })
        .collect();
    let goals: Vec<Value> = suite.list_records("goal").unwrap_or_default();
    let rooms: Vec<Value> = suite.list_records("agent_room").unwrap_or_default();
    let seq = latest_projection_sequence(store, "__proj__:__conversations__");
    Ok(json!({
        "revision": seq.max(1),
        "cursor": {
            "scope": {
                "type": "conversations",
                "conversation_id": Value::Null
            },
            "sequence": seq
        },
        "records": records,
        "sessions": sessions,
        "conversations": conversations,
        "goals": goals,
        "rooms": rooms
    }))
}

pub(super) fn dispatch_projection_cursor_wait(
    store: &Store,
    args: &Value,
) -> Result<Value, (&'static str, String)> {
    let after = args.get("after");
    let scope = after
        .and_then(|a| a.get("scope"))
        .or_else(|| args.get("scope"))
        .cloned()
        .unwrap_or_else(|| {
            let cid = args
                .get("conversation_id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty());
            if let Some(id) = cid {
                json!({"type": "conversation", "conversation_id": id})
            } else {
                json!({"type": "conversations", "conversation_id": Value::Null})
            }
        });
    let scope_type = scope
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("conversation");
    let conv_id = scope
        .get("conversation_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty());
    let after_seq = after
        .and_then(|a| a.get("sequence"))
        .or_else(|| args.get("after_seq"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let wait_ms = args
        .get("wait_ms")
        .and_then(|v| v.as_u64())
        .unwrap_or(0)
        .min(250);
    let scope_key = projection_event_scope_key(scope_type, conv_id);
    let read_cursor_seq = || -> u64 {
        let proj = latest_projection_sequence(store, &scope_key);
        let raw = if scope_type == "conversation" {
            conv_id
                .map(|cid| latest_projection_sequence(store, cid))
                .unwrap_or(0)
        } else {
            0
        };
        proj.max(raw).max(after_seq)
    };
    let mut seq = read_cursor_seq();
    if seq <= after_seq && wait_ms > 0 {
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(wait_ms);
        while seq <= after_seq && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(15));
            seq = read_cursor_seq();
        }
    }
    Ok(json!({
        "scope": scope,
        "sequence": seq.max(after_seq)
    }))
}

pub(super) fn dispatch_projection_event_suffix(
    store: &Store,
    args: &Value,
) -> Result<Value, (&'static str, String)> {
    let after = args.get("after");
    let scope = args
        .get("scope")
        .or_else(|| after.and_then(|a| a.get("scope")))
        .cloned()
        .unwrap_or_else(|| {
            let cid = args
                .get("conversation_id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty());
            if let Some(id) = cid {
                json!({"type": "conversation", "conversation_id": id})
            } else {
                json!({"type": "conversations", "conversation_id": Value::Null})
            }
        });
    let scope_type = scope
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("conversation");
    let conv_id = scope
        .get("conversation_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty());
    let after_seq = after
        .and_then(|a| a.get("sequence"))
        .or_else(|| args.get("after_seq"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let limit = args
        .get("limit")
        .and_then(|v| v.as_u64())
        .unwrap_or(256)
        .clamp(1, 512) as usize;
    let wait_ms = args
        .get("wait_ms")
        .and_then(|v| v.as_u64())
        .unwrap_or(0)
        .min(250);
    let scope_key = projection_event_scope_key(scope_type, conv_id);

    let fetch = || -> Result<Vec<_>, (&'static str, String)> {
        match store
            .events_after(&scope_key, after_seq as i64, limit + 1)
            .map_err(|e| ("storage_error", e.to_string()))?
        {
            EventPage::Events(ev) => Ok(ev),
            EventPage::ResyncRequired { .. } => Ok(Vec::new()),
        }
    };

    let mut raw_events = fetch()?;
    if raw_events.is_empty() && wait_ms > 0 {
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(wait_ms);
        while raw_events.is_empty() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(15));
            raw_events = fetch()?;
        }
    }

    let has_more = raw_events.len() > limit;
    if has_more {
        raw_events.truncate(limit);
    }

    let mut next_seq = after_seq;
    let mut events_out = Vec::with_capacity(raw_events.len());
    for ev in raw_events {
        let seq = ev.seq.max(1) as u64;
        if seq != next_seq + 1 {
            break;
        }
        next_seq = seq;
        let cid_str = conv_id
            .or_else(|| ev.payload.get("conversation_id").and_then(|v| v.as_str()))
            .unwrap_or("");
        let event_id = ev
            .payload
            .get("event_id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .map(ToString::to_string)
            .unwrap_or_else(|| format!("proj-{cid_str}-{seq}"));
        let occurred_at = ev
            .payload
            .get("occurred_at")
            .and_then(|v| v.as_str())
            .map(ToString::to_string)
            .unwrap_or_else(|| Utc::now().to_rfc3339());
        let mutation = ev
            .payload
            .get("mutation")
            .cloned()
            .unwrap_or_else(|| json!({"type": "invalidate_projection", "revision": seq}));
        let message_id = match mutation.get("type").and_then(|v| v.as_str()) {
            Some("upsert_message") => mutation
                .pointer("/message/message_id")
                .cloned()
                .unwrap_or(Value::Null),
            Some("remove_message" | "append_text_preview" | "complete_text_part") => {
                mutation.get("message_id").cloned().unwrap_or(Value::Null)
            }
            _ => Value::Null,
        };
        let part_id = match mutation.get("type").and_then(|v| v.as_str()) {
            Some("append_text_preview" | "complete_text_part") => {
                mutation.get("part_id").cloned().unwrap_or(Value::Null)
            }
            _ => Value::Null,
        };
        events_out.push(json!({
            "scope": scope,
            "cursor": {
                "scope": scope,
                "sequence": seq
            },
            "identity": {
                "event_id": event_id,
                "conversation_id": cid_str,
                "run_id": Value::Null,
                "attempt_id": Value::Null,
                "delegated_invocation_id": Value::Null,
                "message_id": message_id,
                "part_id": part_id
            },
            "mutation": {
                "type": "conversation",
                "mutation": mutation
            },
            "occurred_at": occurred_at
        }));
    }
    let effective_has_more = has_more && !events_out.is_empty();
    Ok(json!({
        "events": events_out,
        "next_cursor": {
            "scope": scope,
            "sequence": next_seq
        },
        "has_more": effective_has_more
    }))
}


pub(super) fn dispatch_read_conversation_part_derivation(
    store: &Store,
    args: &Value,
) -> Result<Value, (&'static str, String)> {
    let owner = args
        .get("owner")
        .ok_or(("invalid_argument", "missing 'owner'".into()))?;
    let conv_id = owner
        .get("conversation_id")
        .and_then(|v| v.as_str())
        .ok_or(("invalid_argument", "missing owner.conversation_id".into()))?;
    let msg_id = owner
        .get("message_id")
        .and_then(|v| v.as_str())
        .ok_or(("invalid_argument", "missing owner.message_id".into()))?;
    let part_id = owner
        .get("part_id")
        .and_then(|v| v.as_str())
        .ok_or(("invalid_argument", "missing owner.part_id".into()))?;
    let key = format!("part_derivation:{conv_id}:{msg_id}:{part_id}");
    let found = store
        .get_kv(&key)
        .map_err(|e| ("storage_error", e.to_string()))?;
    Ok(found.unwrap_or(Value::Null))
}

pub(super) fn dispatch_read_conversation_tool_source_range(
    store: &Store,
    args: &Value,
) -> Result<Value, (&'static str, String)> {
    let owner = args
        .get("owner")
        .cloned()
        .ok_or(("invalid_argument", "missing 'owner'".into()))?;
    let conv_id = owner
        .get("conversation_id")
        .and_then(|v| v.as_str())
        .ok_or(("invalid_argument", "missing owner.conversation_id".into()))?;
    let msg_id = owner
        .get("message_id")
        .and_then(|v| v.as_str())
        .ok_or(("invalid_argument", "missing owner.message_id".into()))?;
    let part_id = owner
        .get("part_id")
        .and_then(|v| v.as_str())
        .ok_or(("invalid_argument", "missing owner.part_id".into()))?;
    let source_item_id = args
        .get("source_item_id")
        .or_else(|| args.get("sourceItemID"))
        .and_then(|v| v.as_str())
        .ok_or(("invalid_argument", "missing 'source_item_id'".into()))?;
    let expected_revision = args
        .get("expected_conversation_revision")
        .or_else(|| args.get("expectedConversationRevision"))
        .and_then(|v| v.as_i64())
        .unwrap_or(1);
    let offset_bytes = args
        .get("offset_bytes")
        .or_else(|| args.get("offsetBytes"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;
    let limit_bytes = args
        .get("limit_bytes")
        .or_else(|| args.get("limitBytes"))
        .and_then(|v| v.as_u64())
        .unwrap_or(32_768)
        .clamp(1, 32_768) as usize;

    let body = store
        .get_kv(&format!("tool_source_body:{conv_id}:{msg_id}:{part_id}:{source_item_id}"))
        .ok()
        .flatten()
        .and_then(|v| {
            v.as_str()
                .map(ToString::to_string)
                .or_else(|| v.get("text").and_then(|t| t.as_str()).map(ToString::to_string))
                .or_else(|| Some(v.to_string()))
        })
        .unwrap_or_default();

    let total = body.len();
    let start = offset_bytes.min(total);
    let mut end = (start + limit_bytes).min(total);
    while end > start && !body.is_char_boundary(end) {
        end -= 1;
    }
    if end == start && start < total {
        end = (start + 1..=total)
            .find(|&i| body.is_char_boundary(i))
            .unwrap_or(total);
    }
    let slice = if start <= end && body.is_char_boundary(start) && body.is_char_boundary(end) {
        &body[start..end]
    } else {
        ""
    };
    let next_offset = start + slice.len();
    Ok(json!({
        "owner": owner,
        "source_item_id": source_item_id,
        "revision": expected_revision,
        "offset_bytes": start as u64,
        "next_offset_bytes": next_offset as u64,
        "total_utf8_bytes": total as u64,
        "text": slice,
        "end_of_content": next_offset == total
    }))
}

pub(super) fn dispatch_begin_receive_assistant_run_terminal(
    store: &Store,
    args: &Value,
) -> Result<Value, (&'static str, String)> {
    let source_host = args
        .get("source_host_id")
        .or_else(|| args.get("sourceHostID"))
        .and_then(|v| v.as_str())
        .unwrap_or("remote");
    let seal = args
        .get("seal")
        .cloned()
        .ok_or(("invalid_argument", "missing 'seal'".into()))?;
    let conv_id = seal
        .pointer("/identity/conversation_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let run_id = seal
        .pointer("/identity/run_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let total = seal
        .get("entry_count")
        .and_then(|v| v.as_str().and_then(|s| s.parse::<u64>().ok()).or_else(|| v.as_u64()))
        .unwrap_or(0);
    let key = format!("terminal_reception:{source_host}:{conv_id}:{run_id}");
    if let Ok(Some(existing)) = store.get_kv(&key) {
        if existing.get("seal") == Some(&seal) {
            return Ok(existing);
        }
    }
    let state = json!({
        "seal": seal,
        "next_entry": "0",
        "manifest_complete": total == 0,
        "committed": false
    });
    store
        .set_kv(&key, &state)
        .map_err(|e| ("storage_error", e.to_string()))?;
    Ok(state)
}

pub(super) fn dispatch_stage_assistant_run_terminal_manifest(
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
    let seal = page
        .get("seal")
        .cloned()
        .ok_or(("invalid_argument", "missing 'page.seal'".into()))?;
    let conv_id = seal
        .pointer("/identity/conversation_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let run_id = seal
        .pointer("/identity/run_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let next_entry = page
        .get("next_entry")
        .and_then(|v| v.as_str().map(ToString::to_string).or_else(|| v.as_u64().map(|n| n.to_string())))
        .unwrap_or_else(|| "0".to_string());
    let complete = page
        .get("complete")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let manifest_key = format!("terminal_manifest_staged:{source_host}:{conv_id}:{run_id}");
    let mut staged = store
        .get_kv(&manifest_key)
        .ok()
        .flatten()
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();
    if let Some(entries) = page.get("entries").and_then(|v| v.as_array()) {
        staged.extend(entries.iter().cloned());
    }
    let _ = store.set_kv(&manifest_key, &Value::Array(staged));
    let state = json!({
        "seal": seal,
        "next_entry": next_entry,
        "manifest_complete": complete,
        "committed": false
    });
    let key = format!("terminal_reception:{source_host}:{conv_id}:{run_id}");
    store
        .set_kv(&key, &state)
        .map_err(|e| ("storage_error", e.to_string()))?;
    Ok(state)
}

pub(super) fn dispatch_commit_received_assistant_run_terminal(
    store: &Store,
    args: &Value,
) -> Result<Value, (&'static str, String)> {
    let source_host = args
        .get("source_host_id")
        .or_else(|| args.get("sourceHostID"))
        .and_then(|v| v.as_str())
        .unwrap_or("remote");
    let seal = args
        .get("seal")
        .cloned()
        .ok_or(("invalid_argument", "missing 'seal'".into()))?;
    let conv_id = seal
        .pointer("/identity/conversation_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let run_id = seal
        .pointer("/identity/run_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let entry_count = seal
        .get("entry_count")
        .and_then(|v| v.as_str().map(ToString::to_string).or_else(|| v.as_u64().map(|n| n.to_string())))
        .unwrap_or_else(|| "0".to_string());
    let state = json!({
        "seal": seal,
        "next_entry": entry_count,
        "manifest_complete": true,
        "committed": true
    });
    let key = format!("terminal_reception:{source_host}:{conv_id}:{run_id}");
    store
        .set_kv(&key, &state)
        .map_err(|e| ("storage_error", e.to_string()))?;
    if !conv_id.is_empty() && !run_id.is_empty() {
        let _ = store.set_kv(&format!("run_terminal_seal:{conv_id}:{run_id}"), &seal);
        if let Some(active) = lookup_active_run(store, conv_id) {
            if active.get("run_id").and_then(|v| v.as_str()) == Some(run_id) {
                clear_active_run(store, conv_id);
                let _ = store.set_kv(&format!("active_run:{conv_id}"), &Value::Null);
            }
        }
        // Materialize the committed assistant message into the receiver's session events
        // if not already present so the canonical window exposes the assistant row + journal head.
        let assistant_id = store
            .get_kv(&format!("journal_head:{conv_id}:{run_id}"))
            .ok()
            .flatten()
            .and_then(|h| {
                h.pointer("/identity/assistant_message_id")
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string)
            })
            .unwrap_or_else(|| format!("{run_id}:assistant"));
        let (existing_msgs, _) = crate::web_api::reconstruct_v2_messages(store, conv_id);
        let already_has_assistant = existing_msgs.iter().any(|m| {
            m.get("id").and_then(|v| v.as_str()) == Some(assistant_id.as_str())
                || (m.get("role").and_then(|v| v.as_str()) == Some("assistant")
                    && m.get("run_id").and_then(|v| v.as_str()) == Some(run_id))
        });
        if !already_has_assistant {
            let mut body_texts: Vec<String> = Vec::new();
            if let Ok(parts) = store.list_module_records::<Value>("assistant_journal_part") {
                for p in parts {
                    if p.pointer("/identity/conversation_id").and_then(|v| v.as_str()) == Some(conv_id)
                        && p.pointer("/identity/run_id").and_then(|v| v.as_str()) == Some(run_id)
                    {
                        let pid = p.get("part_id").and_then(|v| v.as_str()).unwrap_or("");
                        if let Some(bodies) = p.get("bodies").and_then(|v| v.as_array()) {
                            for b in bodies {
                                let field = b
                                    .get("field")
                                    .or_else(|| b.get("content_id"))
                                    .and_then(|v| v.as_str())
                                    .unwrap_or(pid);
                                if let Ok(Some(c)) = store.get_kv(&format!(
                                    "assistant_journal_content:{conv_id}:{run_id}:{pid}:{field}"
                                )) {
                                    if let Some(t) = c.get("text").and_then(|v| v.as_str()) {
                                        if !t.is_empty() {
                                            body_texts.push(t.to_string());
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            let payload = json!({
                "id": assistant_id,
                "message_id": assistant_id,
                "run_id": run_id,
                "role": "assistant",
                "status": "completed",
                "content": body_texts.join("\n\n"),
                "created_at_ms": now_ms(),
                "completed_at_ms": now_ms()
            });
            let _ = store.append_event(conv_id, "assistant_message", payload, 50_000);
        }
        let meta_key = crate::web_api::session_meta_key(conv_id);
        let mut meta = store
            .get_kv(&meta_key)
            .ok()
            .flatten()
            .unwrap_or_else(|| json!({"isPinned": false, "revision": 1, "configurationRevision": 1}));
        let next_rev = meta.get("revision").and_then(|v| v.as_u64()).unwrap_or(1) + 1;
        meta["revision"] = json!(next_rev);
        let _ = store.set_kv(&meta_key, &meta);
        append_projection_event(
            store,
            conv_id,
            json!({
                "type": "invalidate_projection",
                "revision": next_rev
            }),
        );
    }
    Ok(state)
}


pub(super) fn dispatch_peek_agent_command(
    store: &Store,
    args: &Value,
) -> Result<Value, (&'static str, String)> {
    let session_id = args
        .get("session_id")
        .or_else(|| args.get("conversation_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if session_id.is_empty() {
        return Ok(Value::Null);
    }
    let queue = store
        .get_kv(&format!("agent_cmd_queue:{session_id}"))
        .ok()
        .flatten()
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();
    Ok(queue.into_iter().next().unwrap_or(Value::Null))
}

pub(super) fn dispatch_claim_next_agent_command(
    store: &Store,
    args: &Value,
) -> Result<Value, (&'static str, String)> {
    let session_id = args
        .get("session_id")
        .or_else(|| args.get("conversation_id"))
        .and_then(|v| v.as_str())
        .ok_or(("invalid_argument", "missing 'session_id'".into()))?;
    let lease_owner = args
        .get("lease_owner")
        .and_then(|v| v.as_str())
        .ok_or(("invalid_argument", "missing 'lease_owner'".into()))?;
    let now = args.get("now_ms").and_then(|v| v.as_i64()).unwrap_or_else(now_ms);
    let lease_until = args
        .get("lease_until_ms")
        .and_then(|v| v.as_i64())
        .unwrap_or(now + 300_000)
        .max(now + 1);
    let key = format!("agent_cmd_queue:{session_id}");
    let mut queue = store
        .get_kv(&key)
        .ok()
        .flatten()
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();
    let Some(head) = queue.first_mut() else {
        return Ok(Value::Null);
    };
    let existing_owner = head.get("lease_owner").and_then(|v| v.as_str());
    let existing_until = head.get("lease_until_ms").and_then(|v| v.as_i64()).unwrap_or(0);
    if existing_owner.is_some_and(|o| !o.is_empty() && o != lease_owner) && existing_until > now {
        return Ok(Value::Null);
    }
    let next_attempt = head
        .get("delivery_attempt")
        .and_then(|v| v.as_i64())
        .unwrap_or(0)
        + 1;
    let cmd_id = head
        .get("command_id")
        .and_then(|v| v.as_str())
        .map(ToString::to_string)
        .unwrap_or_else(new_id);
    let payload_json = head
        .get("payload_json")
        .and_then(|v| v.as_str())
        .map(ToString::to_string)
        .unwrap_or_else(|| "{}".to_string());
    if let Some(obj) = head.as_object_mut() {
        obj.insert("session_id".into(), json!(session_id));
        obj.insert("command_id".into(), json!(cmd_id));
        obj.insert("lease_owner".into(), json!(lease_owner));
        obj.insert("lease_until_ms".into(), json!(lease_until));
        obj.insert("delivery_attempt".into(), json!(next_attempt));
        obj.insert("payload_json".into(), json!(payload_json));
    }
    let claimed = head.clone();
    store
        .set_kv(&key, &Value::Array(queue))
        .map_err(|e| ("storage_error", e.to_string()))?;
    Ok(claimed)
}

pub(super) fn dispatch_release_agent_command_lease(
    store: &Store,
    args: &Value,
) -> Result<Value, (&'static str, String)> {
    let session_id = args
        .get("session_id")
        .or_else(|| args.get("conversation_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let command_id = args.get("command_id").and_then(|v| v.as_str()).unwrap_or("");
    let lease_owner = args.get("lease_owner").and_then(|v| v.as_str()).unwrap_or("");
    let key = format!("agent_cmd_queue:{session_id}");
    let mut queue = store
        .get_kv(&key)
        .ok()
        .flatten()
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();
    let mut released = false;
    for item in queue.iter_mut() {
        if item.get("command_id").and_then(|v| v.as_str()) == Some(command_id)
            && (lease_owner.is_empty()
                || item.get("lease_owner").and_then(|v| v.as_str()) == Some(lease_owner))
        {
            if let Some(obj) = item.as_object_mut() {
                obj.insert("lease_owner".into(), Value::Null);
                obj.insert("lease_until_ms".into(), json!(0));
            }
            released = true;
        }
    }
    if released {
        let _ = store.set_kv(&key, &Value::Array(queue));
    }
    Ok(json!({"ok": true, "released": released}))
}

pub(super) fn dispatch_acknowledge_agent_command(
    store: &Store,
    args: &Value,
) -> Result<Value, (&'static str, String)> {
    let session_id = args
        .get("session_id")
        .or_else(|| args.get("conversation_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let command_id = args.get("command_id").and_then(|v| v.as_str()).unwrap_or("");
    let key = format!("agent_cmd_queue:{session_id}");
    let mut queue = store
        .get_kv(&key)
        .ok()
        .flatten()
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();
    let before = queue.len();
    queue.retain(|item| item.get("command_id").and_then(|v| v.as_str()) != Some(command_id));
    let acknowledged = queue.len() < before;
    if acknowledged {
        let _ = store.set_kv(&key, &Value::Array(queue));
    }
    Ok(json!({"ok": true, "acknowledged": acknowledged}))
}

pub(super) fn dispatch_execute_ui_tool(
    store: &Store,
    args: &Value,
) -> Result<Value, (&'static str, String)> {
    let input = args.get("input").unwrap_or(args);
    let effect_id = input
        .get("effect_id")
        .and_then(|v| v.as_str())
        .ok_or(("invalid_argument", "missing 'effect_id'".into()))?;
    let request_id = input
        .get("request_id")
        .and_then(|v| v.as_str())
        .ok_or(("invalid_argument", "missing 'request_id'".into()))?;
    let ts = input.get("now_ms").and_then(|v| v.as_i64()).unwrap_or_else(now_ms);
    let rec = json!({
        "effect_id": effect_id,
        "request_id": request_id,
        "executed_at_ms": ts
    });
    let _ = store.set_kv(&format!("ui_tool_exec:{effect_id}:{request_id}"), &rec);
    Ok(json!({
        "type": "completed",
        "effect_id": effect_id,
        "request_id": request_id,
        "output": {
            "ok": true,
            "published": true,
            "effect_id": effect_id,
            "request_id": request_id
        },
        "usage": Value::Null,
        "full_output_ref": Value::Null,
        "artifacts": []
    }))
}

pub(super) fn dispatch_apply_active_permission_policy(
    store: &Store,
    args: &Value,
) -> Result<Value, (&'static str, String)> {
    let conv_id = args
        .get("conversation_id")
        .and_then(|v| v.as_str())
        .ok_or(("invalid_argument", "missing 'conversation_id'".into()))?;
    let policy = args
        .get("policy")
        .cloned()
        .ok_or(("invalid_argument", "missing 'policy'".into()))?;
    let policy_run = policy.get("run_id").and_then(|v| v.as_str()).unwrap_or("");
    let Some(active) = lookup_active_run(store, conv_id) else {
        return Ok(Value::Null);
    };
    if !policy_run.is_empty() && active.get("run_id").and_then(|v| v.as_str()) != Some(policy_run) {
        return Ok(Value::Null);
    }
    let ts = args.get("now_ms").and_then(|v| v.as_i64()).unwrap_or_else(now_ms);
    store
        .set_kv(&format!("active_perm_policy:{conv_id}"), &policy)
        .map_err(|e| ("storage_error", e.to_string()))?;
    let run_id = active
        .get("run_id")
        .and_then(|v| v.as_str())
        .unwrap_or(policy_run);
    let rev = policy
        .get("configuration_revision")
        .and_then(|v| v.as_u64())
        .unwrap_or(1)
        .max(1);
    Ok(json!({
        "conversation_id": conv_id,
        "run_id": run_id,
        "applied": true,
        "applied_at_ms": ts,
        "policy": policy,
        "control": {
            "session_id": conv_id,
            "revision": rev,
            "active_turn_id": run_id,
            "turn_status": "running",
            "permission_projection_revision": rev,
            "last_event_cursor": {"sequence": rev}
        }
    }))
}

pub(super) fn dispatch_load_mobile_task_wake_plan(
    suite: &ModulesSuite,
    args: &Value,
) -> Result<Value, (&'static str, String)> {
    let now = args.get("now_ms").and_then(|v| v.as_i64()).unwrap_or_else(now_ms);
    let tasks: Vec<Value> = suite
        .list_records("runtime_task")
        .map_err(|e| ("storage_error", e.to_string()))?;
    let mut candidates = Vec::new();
    let mut next_wake: Option<i64> = None;
    for t in tasks {
        let kind = t.get("kind").and_then(|v| v.as_str()).unwrap_or("");
        let status = t.get("status").and_then(|v| v.as_str()).unwrap_or("");
        if kind != "agent" || !matches!(status, "queued" | "running") {
            continue;
        }
        if let Some(wake_at) = t.get("wake_at_ms").and_then(|v| v.as_i64()) {
            if wake_at > now {
                next_wake = Some(next_wake.map_or(wake_at, |cur| cur.min(wake_at)));
                continue;
            }
        }
        if t.get("parent_invocation").is_some() && t.get("parent_config").is_some() {
            candidates.push(json!({
                "task_id": t.get("id").or_else(|| t.get("task_id")).cloned().unwrap_or(Value::Null),
                "parent_conversation_id": t.get("owner_session_id").or_else(|| t.get("conversation_id")).cloned().unwrap_or(Value::Null),
                "child_conversation_id": t.get("child_session_id").cloned().unwrap_or(Value::Null),
                "parent_invocation": t.get("parent_invocation").cloned().unwrap_or(Value::Null),
                "parent_config": t.get("parent_config").cloned().unwrap_or(Value::Null)
            }));
        }
    }
    Ok(json!({
        "candidates": candidates,
        "next_wake_at_ms": next_wake
    }))
}

pub(super) fn dispatch_request_task_cancellation(
    store: &Store,
    suite: &ModulesSuite,
    args: &Value,
) -> Result<Value, (&'static str, String)> {
    let input = args.get("input").unwrap_or(args);
    let task_id = input
        .get("task_id")
        .or_else(|| args.get("task_id"))
        .and_then(|v| v.as_str())
        .ok_or(("invalid_argument", "missing 'task_id'".into()))?;
    let ts = input.get("now_ms").and_then(|v| v.as_i64()).unwrap_or_else(now_ms);
    let mut state = "cancelled".to_string();
    if let Ok(Some(mut task)) = suite.get_record::<Value>("runtime_task", task_id) {
        if let Some(obj) = task.as_object_mut() {
            obj.insert("cancel_requested".into(), json!(true));
            obj.insert("updated_at_ms".into(), json!(ts));
            if let Some(s) = obj.get("status").and_then(|v| v.as_str()) {
                state = s.to_string();
            }
        }
        let _ = suite.upsert_record("runtime_task", task_id, &task);
    }
    let _ = store.set_kv(
        &format!("cancel_requested:{task_id}"),
        &json!({"task_id": task_id, "cancel_requested": true, "requested_at_ms": ts}),
    );
    Ok(json!({
        "ok": true,
        "task_id": task_id,
        "cancel_requested": true,
        "state": state
    }))
}

pub(super) fn dispatch_execute_task_cancellation_for_effect(
    store: &Store,
    suite: &ModulesSuite,
    args: &Value,
) -> Result<Value, (&'static str, String)> {
    let req = args.get("request").unwrap_or(args);
    let task_id = req
        .pointer("/payload/task_id")
        .or_else(|| args.get("task_id"))
        .and_then(|v| v.as_str())
        .ok_or(("invalid_argument", "missing request.payload.task_id".into()))?;
    let ts = args.get("now_ms").and_then(|v| v.as_i64()).unwrap_or_else(now_ms);
    if let Ok(Some(mut task)) = suite.get_record::<Value>("runtime_task", task_id) {
        if let Some(obj) = task.as_object_mut() {
            obj.insert("cancel_requested".into(), json!(true));
            obj.insert("updated_at_ms".into(), json!(ts));
        }
        let _ = suite.upsert_record("runtime_task", task_id, &task);
    }
    let _ = store.set_kv(
        &format!("cancel_requested:{task_id}"),
        &json!({"task_id": task_id, "cancel_requested": true, "requested_at_ms": ts}),
    );
    // Strict contract: iOS IOSCanonicalAgentApplication & Android executeAndroidTaskCancellation
    // require EXACTLY the keys {"kind", "cancel_requested"}.
    Ok(json!({
        "kind": "applied",
        "cancel_requested": true
    }))
}

pub(super) fn dispatch_inspect_cancellation_process_owners(
    args: &Value,
) -> Result<Value, (&'static str, String)> {
    let identity = args.get("identity").cloned().unwrap_or_else(|| json!({}));
    let control = args.get("control").cloned().unwrap_or_else(|| json!({}));
    let session_id = control.get("session_id").cloned().unwrap_or(Value::Null);
    let turn_id = control.get("turn_id").cloned().unwrap_or(Value::Null);
    let target_effect_id = control
        .pointer("/payload/target_effect_id")
        .cloned()
        .unwrap_or(Value::Null);
    Ok(json!({
        "identity": identity,
        "control": control,
        "session_id": session_id,
        "turn_id": turn_id,
        "target_effect_id": target_effect_id,
        "target_owners": []
    }))
}

pub(super) fn dispatch_validate_cancellation_process_observations(
    args: &Value,
) -> Result<Value, (&'static str, String)> {
    let plan = args
        .get("plan")
        .ok_or(("invalid_argument", "missing 'plan'".into()))?;
    let effect_id = plan
        .get("target_effect_id")
        .or_else(|| plan.pointer("/control/payload/target_effect_id"))
        .cloned()
        .unwrap_or(Value::Null);
    let session_id = plan
        .get("session_id")
        .or_else(|| plan.pointer("/control/session_id"))
        .cloned()
        .unwrap_or(Value::Null);
    let turn_id = plan
        .get("turn_id")
        .or_else(|| plan.pointer("/control/turn_id"))
        .cloned()
        .unwrap_or(Value::Null);
    Ok(json!({
        "effect_id": effect_id,
        "session_id": session_id,
        "turn_id": turn_id,
        "validated": true
    }))
}

pub(super) fn dispatch_queue_task_guidance(
    store: &Store,
    args: &Value,
) -> Result<Value, (&'static str, String)> {
    let input = args.get("input").unwrap_or(args);
    let task_id = input
        .get("task_id")
        .and_then(|v| v.as_str())
        .ok_or(("invalid_argument", "missing 'task_id'".into()))?;
    let message_id = input
        .get("message_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(new_id);
    let parent_session_id = input
        .get("parent_session_id")
        .or_else(|| input.get("conversation_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let parent_turn_id = input
        .get("parent_turn_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let content = input
        .get("content")
        .or_else(|| input.get("message"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let enqueued_at = input
        .get("enqueued_at_epoch_ms")
        .or_else(|| input.get("submitted_at_ms"))
        .and_then(|v| v.as_i64())
        .unwrap_or_else(now_ms);
    let record = json!({
        "task_id": task_id,
        "message_id": message_id,
        "owner_session_id": parent_session_id,
        "parent_turn_id": parent_turn_id,
        "content": content,
        "submitted_at_ms": enqueued_at,
        "status": "accepted"
    });
    let _ = store.set_kv(&format!("task_guidance_receipt:{task_id}:{message_id}"), &record);
    let _ = store.set_kv(&format!("task_guidance_receipt:{message_id}"), &record);
    let _ = store.set_kv(&format!("task_guidance_receipt:{task_id}"), &record);
    Ok(record)
}

pub(super) fn dispatch_receive_remote_expert_parent_admission(
    store: &Store,
    args: &Value,
) -> Result<Value, (&'static str, String)> {
    let expected_host = args
        .get("expected_source_host_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let expected_epoch = args
        .get("expected_source_store_epoch")
        .and_then(|v| v.as_str());
    let admission = args
        .get("admission")
        .cloned()
        .ok_or(("invalid_argument", "missing 'admission'".into()))?;
    if let (Some(exp), Some(actual)) = (
        expected_epoch,
        admission.get("source_store_epoch").and_then(|v| v.as_str()),
    ) {
        if exp != actual {
            return Err((
                "epoch_mismatch",
                format!("remote expert parent admission epoch mismatch: expected {exp}, got {actual}"),
            ));
        }
    }
    let conv_id = admission
        .get("conversation_id")
        .or_else(|| admission.pointer("/identity/conversation_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("default");
    let run_id = admission
        .get("run_id")
        .or_else(|| admission.pointer("/identity/run_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("default");
    let key = format!("remote_expert_parent_admission:{expected_host}:{conv_id}:{run_id}");
    store
        .set_kv(&key, &admission)
        .map_err(|e| ("storage_error", e.to_string()))?;
    Ok(json!({
        "ok": true,
        "admitted": true,
        "source_host_id": expected_host,
        "admission": admission
    }))
}

pub(super) fn dispatch_read_admitted_remote_run_identity(
    store: &Store,
    args: &Value,
) -> Result<Value, (&'static str, String)> {
    let source_host = args.get("source_host_id").and_then(|v| v.as_str()).unwrap_or("");
    let client_id = args.get("client_id").and_then(|v| v.as_str()).unwrap_or("");
    let conv_id = args
        .get("conversation_id")
        .and_then(|v| v.as_str())
        .ok_or(("invalid_argument", "missing 'conversation_id'".into()))?;
    let run_id = args
        .get("run_id")
        .and_then(|v| v.as_str())
        .ok_or(("invalid_argument", "missing 'run_id'".into()))?;
    let asst_id = args
        .get("assistant_message_id")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let candidates = [
        store.get_kv(&format!("remote_sub_receipt:{conv_id}:{run_id}")).ok().flatten(),
        store.get_kv(&format!("observed_remote_run:{conv_id}:{run_id}")).ok().flatten(),
        store.get_kv(&format!("remote_sub_intent:{conv_id}")).ok().flatten(),
    ];
    for cand in candidates.into_iter().flatten() {
        let Some(ident) = cand.get("identity") else {
            continue;
        };
        let matches_host = source_host.is_empty()
            || ident.get("source_host_id").and_then(|v| v.as_str()) == Some(source_host);
        let matches_client = client_id.is_empty()
            || ident.get("client_id").and_then(|v| v.as_str()) == Some(client_id);
        let matches_conv = ident.get("conversation_id").and_then(|v| v.as_str()) == Some(conv_id);
        let matches_run = ident.get("submission_id").and_then(|v| v.as_str()) == Some(run_id);
        let matches_asst = asst_id.is_empty()
            || ident.get("assistant_message_id").and_then(|v| v.as_str()) == Some(asst_id);
        if matches_host && matches_client && matches_conv && matches_run && matches_asst {
            return Ok(ident.clone());
        }
    }
    Ok(Value::Null)
}

