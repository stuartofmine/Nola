// SPDX-License-Identifier: Apache-2.0
//! Clean v2 WebHost JSON projections and workspace/CRUD helpers shared across Web & Windows clients.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use chrono::Utc;
use nola_engine::load_session_usage;
use nola_modules::scheduler::{
    automation_stats_summary, next_scheduled_at_ms, parse_automation_value, AutomationRunStats,
    KIND_AUTOMATION_STATS,
};
use nola_modules::ModulesSuite;
use nola_store::{EventPage, Store};
use nola_types::{new_id, now_ms, MemoryItem, ModelCapabilities, PermissionProfile, Session};
use serde_json::{json, Value};

use crate::config::{load_active_config, redacted_llm_config, AppliedConfig};

pub const MAX_REVIEW_FILES: usize = 200;
pub const MAX_REVIEW_DIFF_BYTES: usize = 256 * 1024;
const GIT_REVIEW_TIMEOUT: Duration = Duration::from_secs(5);

pub fn ms_to_rfc3339(ms: i64) -> String {
    chrono::DateTime::<Utc>::from_timestamp_millis(ms)
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_else(|| Utc::now().to_rfc3339())
}

/// Load or generate a persistent host identifier stored in KV `"host_id"`.
/// Guaranteed to be 16..=128 characters so Android/iOS/macOS pairing validation always succeeds.
pub fn ensure_host_id(store: &Store) -> String {
    if let Ok(Some(val)) = store.get_kv("host_id") {
        if let Some(s) = val.as_str().map(str::trim) {
            if (16..=128).contains(&s.len()) {
                return s.to_string();
            }
        }
    }
    let id = format!("nola-host-{}", new_id().replace('-', ""));
    let _ = store.set_kv("host_id", &json!(id));
    id
}

/// Load or generate a persistent store epoch identifier stored in KV `"source_store_epoch"`.
/// Generated once per SQLite store using `new_id()` so paired clients can detect store resets.
pub(crate) fn ensure_source_store_epoch(store: &Store) -> String {
    if let Ok(Some(val)) = store.get_kv("source_store_epoch") {
        if let Some(s) = val.as_str().map(str::trim).filter(|s| !s.is_empty()) {
            return s.to_string();
        }
    }
    let epoch = new_id();
    let _ = store.set_kv("source_store_epoch", &json!(epoch));
    epoch
}

pub fn session_meta_key(id: &str) -> String {
    format!("conv_meta:{id}")
}

pub fn conversation_knowledge_key(id: &str) -> String {
    format!("conv_knowledge:{id}")
}

/// Strictly normalize any persisted or caller-supplied permission profile name to
/// `"cautious"` or `"allow_all"`, defaulting unknown/legacy values to `"cautious"`.
pub fn normalize_permission_profile(raw: Option<&str>) -> &'static str {
    raw.and_then(|s| PermissionProfile::parse_strict(s).ok())
        .unwrap_or(PermissionProfile::Cautious)
        .as_canonical_str()
}

/// Load a conversation's persisted knowledge document from `conv_knowledge:{id}`,
/// returning a valid schema-complete object (never missing/null) so `InformationKnowledge.svelte`
/// (`chatKnowledge.ts`) never throws `Missing knowledge document`.
pub fn load_conversation_knowledge(store: &Store, id: &str) -> Value {
    let raw = store
        .get_kv(&conversation_knowledge_key(id))
        .ok()
        .flatten();
    let obj = raw.as_ref().and_then(|v| v.as_object());
    let key_points = obj
        .and_then(|m| m.get("keyPointsMarkdown"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let wiki = obj
        .and_then(|m| m.get("wikiMarkdown"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let summary = obj
        .and_then(|m| m.get("summary"))
        .and_then(|v| v.as_str())
        .unwrap_or(key_points);
    let entries = obj
        .and_then(|m| m.get("entries"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let updated_at = obj
        .and_then(|m| m.get("updatedAt"))
        .and_then(|v| v.as_i64())
        .map(|n| json!(n))
        .unwrap_or(Value::Null);

    json!({
        "keyPointsMarkdown": key_points,
        "wikiMarkdown": wiki,
        "summary": summary,
        "entries": entries,
        "updatedAt": updated_at
    })
}

/// Project real persisted `TokenUsage` from `session_state:{session_id}`.
pub fn session_usage_json(store: &Store, session_id: &str) -> Value {
    let usage = load_session_usage(store, session_id);
    let total_tokens = usage
        .input_tokens
        .saturating_add(usage.output_tokens);
    json!({
        "inputTokens": usage.input_tokens,
        "outputTokens": usage.output_tokens,
        "totalTokens": total_tokens,
        "cacheReadTokens": usage.cache_read_tokens,
        "cacheWriteTokens": usage.cache_write_tokens,
        "reasoningTokens": usage.reasoning_tokens,
        "usageReported": usage.usage_reported
    })
}

/// Conservatively infer model capabilities when not explicitly declared.
/// Unknown models NEVER default to `image_input: true` or `reasoning: true`.
pub fn infer_model_capabilities(
    model_id: &str,
    declared: Option<&ModelCapabilities>,
) -> ModelCapabilities {
    if let Some(c) = declared {
        return c.clone();
    }
    let lower = model_id.trim().to_ascii_lowercase();
    let base_name = lower.rsplit('/').next().unwrap_or(lower.as_str());

    let image_input = base_name.starts_with("gpt-4o")
        || base_name.starts_with("gpt-4.1")
        || base_name.starts_with("gpt-4.5")
        || base_name.starts_with("gpt-5")
        || base_name.starts_with("claude-3")
        || base_name.starts_with("claude-4")
        || base_name.starts_with("claude-opus-4")
        || base_name.starts_with("claude-sonnet-4")
        || base_name.starts_with("claude-haiku-4")
        || base_name.starts_with("gemini-1.5")
        || base_name.starts_with("gemini-2")
        || base_name.starts_with("gemini-3")
        || base_name.contains("-vl")
        || base_name.contains("vision")
        || base_name.starts_with("pixtral");

    let reasoning = base_name == "o1"
        || base_name.starts_with("o1-")
        || base_name == "o3"
        || base_name.starts_with("o3-")
        || base_name == "o4"
        || base_name.starts_with("o4-")
        || base_name.starts_with("gpt-5")
        || base_name.contains("deepseek-r1")
        || base_name.contains("deepseek-reasoner")
        || base_name.starts_with("qwq")
        || base_name.contains("thinking")
        || base_name.starts_with("claude-3-7")
        || base_name.starts_with("claude-3.7")
        || base_name.starts_with("claude-4")
        || base_name.starts_with("claude-opus-4")
        || base_name.starts_with("claude-sonnet-4")
        || base_name.starts_with("gemini-2.5");

    ModelCapabilities {
        tool_calls: true,
        image_input,
        native_pdf_input: false,
        reasoning,
        sampling_parameters: true,
        stream_reasoning: reasoning,
        supported_reasoning_levels: if reasoning {
            vec!["low".into(), "medium".into(), "high".into()]
        } else {
            Vec::new()
        },
        supports_reasoning_summary: reasoning,
    }
}

pub fn session_to_conversation_json(store: &Store, s: &Session) -> Value {
    let meta = store
        .get_kv(&session_meta_key(&s.id))
        .ok()
        .flatten()
        .unwrap_or_else(|| json!({}));
    let is_pinned = meta
        .get("isPinned")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let revision = meta.get("revision").and_then(|v| v.as_i64()).unwrap_or(1).max(1);
    let config_rev = meta
        .get("configurationRevision")
        .and_then(|v| v.as_i64())
        .unwrap_or(1)
        .max(1);
    let permission_profile =
        normalize_permission_profile(meta.get("permissionProfile").and_then(|v| v.as_str()));
    let reasoning_effort = meta
        .get("reasoningEffort")
        .and_then(|v| v.as_str())
        .unwrap_or("automatic");
    let knowledge = load_conversation_knowledge(store, &s.id);
    let token_usage = session_usage_json(store, &s.id);
    let (reconstructed_msgs, _) = reconstruct_v2_messages(store, &s.id);
    let msg_count = reconstructed_msgs.len();

    json!({
        "conversationId": s.id,
        "id": s.id,
        "title": s.title,
        "isPinned": is_pinned,
        "createdAt": ms_to_rfc3339(s.created_at_ms),
        "updatedAt": ms_to_rfc3339(s.updated_at_ms),
        "selectedModelSource": s.model_connection.as_deref().unwrap_or(""),
        "selectedModelId": s.model_id.as_deref().unwrap_or(""),
        "permissionProfile": permission_profile,
        "reasoningEffort": reasoning_effort,
        "executionBackend": "nola-v2",
        "revision": revision,
        "configurationRevision": config_rev,
        "knowledge": knowledge,
        "tokenUsage": token_usage,
        "message_count": msg_count,
        "workspace_root": s.workspace_root,
        "created_at_ms": s.created_at_ms,
        "updated_at_ms": s.updated_at_ms
    })
}

/// Alias explicitly returning the conversation configuration snapshot (including `knowledge` and `tokenUsage`).
pub fn conversation_configuration_json(store: &Store, s: &Session) -> Value {
    session_to_conversation_json(store, s)
}

/// Reconstruct clean v2 conversation messages from persisted `session_events` for history reload.
pub fn reconstruct_v2_messages(store: &Store, conversation_id: &str) -> (Vec<Value>, Vec<Value>) {
    let events = match store.events_after(conversation_id, 0, 2000) {
        Ok(EventPage::Events(ev)) => ev,
        _ => Vec::new(),
    };
    let mut messages: Vec<Value> = Vec::new();
    let mut raw_events: Vec<Value> = Vec::new();
    let mut last_was_turn_completed = false;
    for ev in &events {
        raw_events.push(serde_json::to_value(ev).unwrap_or(json!({})));
        match ev.kind.as_str() {
            "user_message" | "chat_user" => {
                last_was_turn_completed = false;
                let text = ev
                    .payload
                    .get("content")
                    .or_else(|| ev.payload.get("text"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let attachments = ev
                    .payload
                    .get("attachments")
                    .cloned()
                    .unwrap_or_else(|| json!([]));
                let msg_id = ev
                    .payload
                    .get("id")
                    .or_else(|| ev.payload.get("message_id"))
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
                    .map(ToString::to_string)
                    .unwrap_or_else(|| format!("msg-{}", ev.seq));
                let mut entry = json!({
                    "id": msg_id,
                    "seq": ev.seq,
                    "role": "user",
                    "content": text,
                    "reasoning": "",
                    "toolCalls": [],
                    "attachments": attachments,
                    "createdAtMs": ev.created_at_ms
                });
                if let Some(parts) = ev.payload.get("parts").filter(|v| v.is_array()) {
                    entry["parts"] = parts.clone();
                }
                messages.push(entry);
            }
            "assistant_message" | "chat_assistant" | "turn_completed" => {
                let text = ev
                    .payload
                    .get("content")
                    .or_else(|| ev.payload.get("final_text"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let reasoning = ev
                    .payload
                    .get("reasoning")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let tool_calls = ev
                    .payload
                    .get("toolCalls")
                    .cloned()
                    .unwrap_or_else(|| json!([]));
                let ui_surfaces = ev
                    .payload
                    .get("uiSurfaces")
                    .cloned()
                    .unwrap_or_else(|| json!([]));
                let expert_replies = ev
                    .payload
                    .get("expertReplies")
                    .or_else(|| ev.payload.get("expert_replies"))
                    .cloned();
                let room_id = ev
                    .payload
                    .get("roomId")
                    .or_else(|| ev.payload.get("room_id"))
                    .cloned();
                let usage = ev.payload.get("usage").cloned();
                let has_tools = tool_calls.as_array().is_some_and(|a| !a.is_empty());
                let has_surfaces = ui_surfaces.as_array().is_some_and(|a| !a.is_empty());
                let has_expert_replies = expert_replies
                    .as_ref()
                    .and_then(|v| v.as_array())
                    .is_some_and(|a| !a.is_empty());
                if !text.is_empty()
                    || !reasoning.is_empty()
                    || has_surfaces
                    || has_tools
                    || has_expert_replies
                {
                    let msg_id = ev
                        .payload
                        .get("id")
                        .or_else(|| ev.payload.get("message_id"))
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.trim().is_empty())
                        .map(ToString::to_string)
                        .unwrap_or_else(|| format!("msg-{}", ev.seq));
                    let mut entry = json!({
                        "id": msg_id,
                        "seq": ev.seq,
                        "role": "assistant",
                        "content": text,
                        "reasoning": reasoning,
                        "toolCalls": tool_calls,
                        "uiSurfaces": ui_surfaces,
                        "createdAtMs": ev.created_at_ms
                    });
                    if let Some(er) = expert_replies.filter(|v| !v.is_null()) {
                        entry["expertReplies"] = er;
                    }
                    if let Some(rid) = room_id.filter(|v| !v.is_null()) {
                        entry["roomId"] = rid;
                    }
                    if let Some(u) = usage {
                        entry["usage"] = u;
                    }
                    if let Some(parts) = ev.payload.get("parts").filter(|v| v.is_array()) {
                        entry["parts"] = parts.clone();
                    }
                    // WebHost emits `chat_assistant` right after `AgentEngine`'s `turn_completed`
                    // with richer `reasoning`, `toolCalls`, and `uiSurfaces`; upgrade in-place.
                    let should_replace_prev = ev.kind == "chat_assistant"
                        && (last_was_turn_completed
                            || messages.last().is_some_and(|prev| {
                                prev.get("role").and_then(|v| v.as_str()) == Some("assistant")
                                    && prev.get("content").and_then(|v| v.as_str()) == Some(text)
                                    && prev
                                        .get("reasoning")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("")
                                        .is_empty()
                            }));
                    if should_replace_prev {
                        if let Some(last) = messages.last_mut() {
                            if text.is_empty() {
                                if let Some(prev_text) = last
                                    .get("content")
                                    .and_then(|v| v.as_str())
                                    .filter(|s| !s.is_empty())
                                {
                                    entry["content"] = json!(prev_text);
                                }
                            }
                            *last = entry;
                        }
                        last_was_turn_completed = false;
                    } else {
                        messages.push(entry);
                        last_was_turn_completed = ev.kind == "turn_completed";
                    }
                }
            }
            _ => {}
        }
    }
    (messages, raw_events)
}

pub fn memory_meta_key(id: &str) -> String {
    format!("memory_meta:{id}")
}

pub fn memory_item_to_record_with_store(store: &Store, item: &MemoryItem) -> Value {
    let meta = store.get_kv(&memory_meta_key(&item.id)).ok().flatten();
    memory_item_to_record_with_meta(item, meta.as_ref())
}

pub fn memory_item_to_record(item: &MemoryItem) -> Value {
    memory_item_to_record_with_meta(item, None)
}

fn memory_item_to_record_with_meta(item: &MemoryItem, meta: Option<&Value>) -> Value {
    let created = ms_to_rfc3339(item.created_at_ms);
    let updated = ms_to_rfc3339(item.updated_at_ms);
    let (scope, kind) = item
        .namespace
        .split_once(':')
        .unwrap_or((
            "global",
            if item.namespace.is_empty() {
                "preference"
            } else {
                &item.namespace
            },
        ));
    let source = meta
        .and_then(|m| m.get("source"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("user");
    let confidence = meta
        .and_then(|m| m.get("confidence"))
        .and_then(|v| v.as_f64())
        .filter(|v| v.is_finite())
        .unwrap_or(1.0);
    let importance = meta
        .and_then(|m| m.get("importance"))
        .and_then(|v| v.as_f64())
        .filter(|v| v.is_finite())
        .unwrap_or(0.5);
    let supersedes: Vec<String> = meta
        .and_then(|m| m.get("supersedes"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str().map(str::trim).filter(|s| !s.is_empty()).map(ToString::to_string))
                .collect()
        })
        .unwrap_or_default();
    let sensitivity = meta
        .and_then(|m| m.get("sensitivity"))
        .and_then(|v| v.as_str())
        .map(|s| json!(s))
        .unwrap_or(Value::Null);
    let expires_at = meta
        .and_then(|m| m.get("expires_at"))
        .and_then(|v| v.as_str())
        .map(|s| json!(s))
        .unwrap_or(Value::Null);

    json!({
        "id": item.id,
        "scope": scope,
        "kind": kind,
        "content": item.content,
        "summary": if item.title.is_empty() { Value::Null } else { Value::String(item.title.clone()) },
        "tags": item.tags,
        "source": source,
        "confidence": confidence,
        "supersedes": supersedes,
        "importance": importance,
        "sensitivity": sensitivity,
        "expires_at": expires_at,
        "created_at": created,
        "updated_at": updated,
        "provenance": {
            "kind": source,
            "source": source,
            "observed_at": updated
        }
    })
}

const BUILTIN_SKILL_CATEGORIES: &[&str] = &[
    "documents",
    "data-analysis",
    "design-presentations",
    "communication",
    "planning-automation",
    "knowledge-research",
    "workspace",
];

fn builtin_skill_category_id(name: &str) -> &'static str {
    match name.trim().to_ascii_lowercase().as_str() {
        "code-review" | "debugging" | "tdd" => "workspace",
        _ => "workspace",
    }
}

/// Project a discovered `SkillDescriptor` into the canonical JSON object expected by
/// `/api/agent/settings`, `/api/skills`, and `/api/automation-skills`.
pub fn project_skill_descriptor(s: &nola_types::SkillDescriptor, store: &Store) -> Value {
    let name = s.name.trim();
    let builtins = nola_modules::skills::builtin_skills();
    let builtin_match = builtins
        .iter()
        .find(|b| b.name.eq_ignore_ascii_case(name));
    let is_builtin = s.path.starts_with("builtin://") || builtin_match.is_some();
    let enabled = nola_modules::skills::is_skill_enabled(store, name);
    let category_id = if is_builtin {
        builtin_skill_category_id(name)
    } else {
        "custom"
    };
    let mut obj = json!({
        "id": name,
        "name": name,
        "display_name": name,
        "description": s.description,
        "path": s.path,
        "body": s.body,
        "content": s.body,
        "enabled": enabled,
        "source": if is_builtin { "built_in" } else { "discovered" },
        "built_in": is_builtin,
        "category_id": category_id,
        "revision": 1
    });
    if is_builtin {
        let default_body = builtin_match
            .map(|b| b.body.as_str())
            .unwrap_or(s.body.as_str());
        obj["default_content"] = json!(default_body);
    }
    obj
}

fn normalize_saved_skill_entry(
    raw: &Value,
    builtins: &[nola_types::SkillDescriptor],
    store: &Store,
) -> Value {
    let mut item = raw.clone();
    let Some(map) = item.as_object_mut() else {
        return item;
    };
    let name = map
        .get("name")
        .or_else(|| map.get("id"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .unwrap_or("")
        .to_string();
    let path = map
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let existing_source = map
        .get("source")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let builtin_match = builtins
        .iter()
        .find(|b| b.name.eq_ignore_ascii_case(&name));
    let is_builtin = builtin_match.is_some()
        || path.starts_with("builtin://")
        || map.get("built_in").and_then(|v| v.as_bool()) == Some(true)
        || map.get("builtin").and_then(|v| v.as_bool()) == Some(true)
        || map.get("isBuiltin").and_then(|v| v.as_bool()) == Some(true)
        || matches!(existing_source, "built_in" | "builtin");

    if is_builtin {
        map.insert("source".into(), json!("built_in"));
        map.insert("built_in".into(), json!(true));
        let valid_cat = map
            .get("category_id")
            .and_then(|v| v.as_str())
            .is_some_and(|c| BUILTIN_SKILL_CATEGORIES.contains(&c));
        if !valid_cat {
            map.insert("category_id".into(), json!(builtin_skill_category_id(&name)));
        }
        if !map.contains_key("default_content") {
            if let Some(b) = builtin_match {
                map.insert("default_content".into(), json!(b.body));
            }
        }
    } else {
        map.entry("built_in".to_string()).or_insert_with(|| json!(false));
        map.entry("category_id".to_string()).or_insert_with(|| json!("custom"));
    }
    if !name.is_empty() && !map.contains_key("enabled") {
        map.insert(
            "enabled".into(),
            json!(nola_modules::skills::is_skill_enabled(store, &name)),
        );
    }
    item
}

pub fn build_agent_settings_snapshot(store: &Store, suite: &ModulesSuite) -> Value {
    let saved = store
        .get_kv("agent_settings")
        .ok()
        .flatten()
        .unwrap_or_else(|| json!({}));
    let host_id = ensure_host_id(store);
    let source_store_epoch = ensure_source_store_epoch(store);
    let host_name = saved
        .get("host_name")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .or_else(|| {
            std::env::var("HOSTNAME")
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        })
        .or_else(|| {
            std::env::var("COMPUTERNAME")
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        })
        .unwrap_or_else(|| "Nola Host".into());
    let permission_profile =
        normalize_permission_profile(saved.get("permission_profile").and_then(|v| v.as_str()));
    let custom_system_prompt = saved
        .get("custom_system_prompt")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let enable_local_dev_tools = saved
        .get("enable_local_dev_tools")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let allowed_dirs = saved
        .get("allowed_dirs")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_else(|| vec![json!(suite.workspace_root.display().to_string())]);
    let bash_prefixes = saved
        .get("bash_prefixes")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let auto_recall_memory = saved
        .get("auto_recall_memory")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let builtins = nola_modules::skills::builtin_skills();
    let skills: Vec<Value> = if let Some(arr) = saved.get("skills").and_then(|v| v.as_array()) {
        arr.iter()
            .map(|entry| normalize_saved_skill_entry(entry, &builtins, store))
            .collect()
    } else {
        nola_modules::skills::discover_skills(&suite.workspace_root, suite.user_home.as_deref())
            .into_iter()
            .map(|s| project_skill_descriptor(&s, store))
            .collect()
    };

    let mcp_servers: Vec<Value> =
        if let Some(arr) = saved.get("mcp_servers").and_then(|v| v.as_array()) {
            arr.iter()
                .cloned()
                .map(|mut item| {
                    if let Some(map) = item.as_object_mut() {
                        let ep = map.get("endpoint").cloned();
                        let u = map.get("url").cloned();
                        if ep.is_some() && u.is_none() {
                            map.insert("url".into(), ep.unwrap_or(Value::Null));
                        } else if u.is_some() && ep.is_none() {
                            map.insert("endpoint".into(), u.unwrap_or(Value::Null));
                        }
                    }
                    item
                })
                .collect()
        } else {
            store
                .list_mcp_servers()
                .unwrap_or_default()
                .into_iter()
                .map(|srv| {
                    json!({
                        "server_id": srv.id,
                        "name": srv.name,
                        "display_name": srv.name,
                        "enabled": srv.enabled,
                        "endpoint": srv.url,
                        "url": srv.url,
                        "command": srv.command,
                        "args": srv.args,
                        "revision": 1
                    })
                })
                .collect()
        };

    let uploaded_documents = saved
        .get("uploaded_documents")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let worker_saved = store
        .get_kv("messaging_channel_worker")
        .ok()
        .flatten()
        .unwrap_or_else(|| json!({}));
    let messaging_worker = json!({
        "enabled": worker_saved.get("enabled").and_then(|v| v.as_bool()).unwrap_or(false),
        "backend_url": worker_saved.get("backend_url").or_else(|| worker_saved.get("url")).and_then(|v| v.as_str()).unwrap_or(""),
        "has_access_token": worker_saved.get("has_access_token").and_then(|v| v.as_bool()).unwrap_or(false),
        "execution_backend": worker_saved.get("execution_backend").and_then(|v| v.as_str()).unwrap_or("nola"),
        "turn_timeout_seconds": worker_saved.get("turn_timeout_seconds").and_then(|v| v.as_i64()).unwrap_or(600).clamp(30, 14400),
        "poll_seconds": worker_saved.get("poll_seconds").and_then(|v| v.as_i64()).unwrap_or(5).clamp(1, 300),
        "project_root": worker_saved.get("project_root").and_then(|v| v.as_str()).filter(|s| !s.is_empty()).map(|s| json!(s)).unwrap_or(Value::Null)
    });
    let messaging_channels: Vec<Value> = suite
        .list_records::<Value>("messaging_channel")
        .unwrap_or_default()
        .into_iter()
        .map(|mut ch| {
            if let Some(obj) = ch.as_object_mut() {
                obj.remove("webhook_secret");
            }
            ch
        })
        .collect();

    let cfg = load_active_config(store);
    let model_profiles_resp = build_model_profiles_response(store);
    let model_profiles = model_profiles_resp
        .get("profiles")
        .cloned()
        .unwrap_or_else(|| json!([]));
    let resolved_profile_id = cfg.llm.default_model.as_ref().map(|sel| {
        model_profiles
            .as_array()
            .and_then(|arr| {
                arr.iter().find_map(|p| {
                    let ep_id = p.pointer("/endpoint/id").and_then(|v| v.as_str());
                    let m_name = p
                        .get("modelName")
                        .or_else(|| p.get("model_name"))
                        .and_then(|v| v.as_str());
                    if ep_id == Some(sel.connection.as_str()) && m_name == Some(sel.model.as_str()) {
                        p.get("id")
                            .or_else(|| p.get("profile_id"))
                            .and_then(|v| v.as_str())
                            .map(ToString::to_string)
                    } else {
                        None
                    }
                })
            })
            .unwrap_or_else(|| format!("{}:{}", sel.connection, sel.model))
    });
    let default_chat_model_resolution = match (cfg.llm.default_model.as_ref(), resolved_profile_id) {
        (Some(sel), Some(pid)) => json!({
            "kind": "resolved",
            "origin": "default",
            "status": "resolved",
            "connection_id": sel.connection,
            "model_id": sel.model,
            "profile_id": pid
        }),
        _ => json!({
            "kind": "no_automatic_candidate",
            "origin": "default",
            "status": "no_automatic_candidate",
            "connection_id": Value::Null,
            "model_id": Value::Null,
            "profile_id": Value::Null
        }),
    };
    let native_browser_token_configured = std::env::var("NOLA_NATIVE_BROWSER_TOKEN")
        .ok()
        .is_some_and(|s| !s.trim().is_empty())
        || store
            .get_kv("native_browser_token")
            .ok()
            .flatten()
            .and_then(|v| v.as_str().map(|s| !s.trim().is_empty()))
            .unwrap_or(false);

    json!({
        "source_store_epoch": source_store_epoch,
        "host_identity": {
            "id": host_id,
            "name": host_name,
            "display_name": host_name,
            "source_store_epoch": source_store_epoch,
            "permission_protocol_version": 1
        },
        "permission_profile": permission_profile,
        "custom_system_prompt": custom_system_prompt,
        "enable_local_dev_tools": enable_local_dev_tools,
        "allowed_dirs": allowed_dirs,
        "bash_prefixes": bash_prefixes,
        "auto_recall_memory": auto_recall_memory,
        "skills": skills,
        "uploaded_skills": skills,
        "uploaded_documents": uploaded_documents,
        "mcp_servers": mcp_servers,
        "messaging": {
            "worker": messaging_worker,
            "channels": messaging_channels,
            "management_path": "/settings?section=messaging"
        },
        "computer_use": {
            "feature_enabled": false,
            "token_configured": false,
            "connected": false,
            "available": false
        },
        "native_browser": {
            "feature_enabled": true,
            "token_configured": native_browser_token_configured,
            "connected": false,
            "available": true
        },
        "model_profiles": model_profiles,
        "default_chat_model_resolution": default_chat_model_resolution,
        "default_chat_model_revision": 1,
        "skill_settings_backup_key": Value::Null,
        "execution_backends": [
            {
                "id": "nola",
                "display_name": "Nola v2 Rust Engine",
                "availability": "available",
                "capabilities": {
                    "streams_text": true,
                    "streams_reasoning": true,
                    "streams_tools": true,
                    "supports_resume": true,
                    "supports_cancel": true,
                    "supports_permission_bridge": true
                }
            }
        ]
    })
}

fn scrub_secret_fields(val: &mut Value) {
    match val {
        Value::Object(map) => {
            for key in ["api_key", "apiKey", "secret"] {
                if map.contains_key(key) {
                    map.insert(key.to_string(), json!(""));
                }
            }
            for v in map.values_mut() {
                scrub_secret_fields(v);
            }
        }
        Value::Array(arr) => {
            for item in arr {
                scrub_secret_fields(item);
            }
        }
        _ => {}
    }
}

pub fn build_model_profiles_response(store: &Store) -> Value {
    let cfg: AppliedConfig = load_active_config(store);
    let mut stored_profiles: Vec<Value> = store
        .get_kv("web_model_profiles")
        .ok()
        .flatten()
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();
    for p in &mut stored_profiles {
        scrub_secret_fields(p);
        ensure_profile_snake_case_aliases(p);
    }

    let profiles: Vec<Value> = if !stored_profiles.is_empty() {
        stored_profiles
    } else {
        let default_sel = cfg.llm.default_model.as_ref();
        let mut out = Vec::new();
        for (ep_idx, conn) in cfg.llm.connections.iter().enumerate() {
            let wire_protocol = match conn.protocol.as_deref().unwrap_or("openai_chat") {
                "anthropic" | "anthropic_messages" => "anthropic",
                "responses" | "openai_responses" => "responses",
                "siwc_responses" => "siwc_responses",
                "gemini" | "google_gemini" => "gemini",
                _ => "chat_completions",
            };
            for (m_idx, m) in conn.models.iter().enumerate() {
                let selected = default_sel
                    .map(|d| d.connection == conn.id && d.model == m.id)
                    .unwrap_or(false);
                let caps = infer_model_capabilities(&m.id, m.capabilities.as_ref());
                let capabilities = vec!["chat".to_string()];
                let exec_caps = json!({
                    "toolCalls": caps.tool_calls,
                    "imageInput": caps.image_input,
                    "reasoning": caps.reasoning,
                    "samplingParameters": caps.sampling_parameters,
                    "samplingRequiresNoReasoning": false,
                    "chatOutputTokenField": "auto"
                });
                let mut params = serde_json::Map::new();
                if let Some(t) = m.temperature {
                    params.insert("temperature".into(), json!(t));
                }
                if let Some(p) = m.top_p {
                    params.insert("topP".into(), json!(p));
                }
                if let Some(max_out) = m.max_output_tokens {
                    params.insert("maxOutputTokens".into(), json!(max_out));
                }
                if let Some(ctx) = m.context_window {
                    params.insert("maxContextLength".into(), json!(ctx));
                }
                let reasoning_obj = if caps.reasoning {
                    json!({
                        "control": "automatic",
                        "supportedLevels": caps.supported_reasoning_levels,
                        "efforts": caps.supported_reasoning_levels,
                        "supportsSummary": caps.supports_reasoning_summary
                    })
                } else {
                    json!({ "control": "unsupported" })
                };
                let prof_id = format!("{}:{}", conn.id, m.id);
                out.push(json!({
                    "id": prof_id,
                    "profile_id": prof_id,
                    "provider": "openai-compatible",
                    "connectionId": Value::Null,
                    "modelId": m.id,
                    "modelName": m.id,
                    "model_name": m.id,
                    "displayName": m.id,
                    "display_name": m.id,
                    "revision": 1,
                    "enabled": true,
                    "selected": selected,
                    "priority": if selected { 100 } else { 50 },
                    "order": m_idx,
                    "capabilities": capabilities,
                    "executionCapabilities": exec_caps,
                    "executionCapabilityStatus": if caps.tool_calls { "tools_enabled" } else { "text_only" },
                    "reasoning": reasoning_obj,
                    "parameters": Value::Object(params),
                    "protocol": wire_protocol,
                    "endpoint": {
                        "id": conn.id,
                        "name": conn.id,
                        "url": conn.endpoint,
                        "baseURL": conn.endpoint,
                        "protocol": wire_protocol,
                        "wireProtocol": wire_protocol,
                        "enabled": true,
                        "order": ep_idx
                    }
                }));
            }
        }
        out
    };

    json!({
        "profiles": profiles,
        "llm": redacted_llm_config(&cfg.llm)
    })
}

fn ensure_profile_snake_case_aliases(p: &mut Value) {
    let Some(obj) = p.as_object_mut() else {
        return;
    };
    let id = obj
        .get("id")
        .or_else(|| obj.get("profile_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let model_name = obj
        .get("modelName")
        .or_else(|| obj.get("model_name"))
        .or_else(|| obj.get("modelId"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let display_name = obj
        .get("displayName")
        .or_else(|| obj.get("display_name"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(&model_name)
        .to_string();
    if !id.is_empty() {
        obj.entry("profile_id".to_string()).or_insert_with(|| json!(id));
        obj.entry("id".to_string()).or_insert_with(|| json!(id));
    }
    if !model_name.is_empty() {
        obj.entry("model_name".to_string())
            .or_insert_with(|| json!(model_name));
        obj.entry("modelName".to_string())
            .or_insert_with(|| json!(model_name));
    }
    if !display_name.is_empty() {
        obj.entry("display_name".to_string())
            .or_insert_with(|| json!(display_name));
        obj.entry("displayName".to_string())
            .or_insert_with(|| json!(display_name));
    }
    if let Some(reasoning) = obj.get_mut("reasoning").and_then(|v| v.as_object_mut()) {
        let levels = reasoning.get("supportedLevels").cloned();
        let efforts = reasoning.get("efforts").cloned();
        if let (Some(l), None) = (levels.as_ref(), efforts.as_ref()) {
            reasoning.insert("efforts".to_string(), l.clone());
        } else if let (None, Some(e)) = (levels.as_ref(), efforts.as_ref()) {
            reasoning.insert("supportedLevels".to_string(), e.clone());
        }
    }
}

/// Normalize any automation `format` value to the canonical set expected by the Web UI
/// (`agent`, `html`, `search`, `rss`, `json`, `icalendar`, `file`, `folder`, `script`, `reminder`),
/// mapping legacy/alias `"prompt"` values to `"agent"`.
pub fn canonical_automation_format(raw: Option<&str>) -> String {
    let trimmed = raw.map(str::trim).unwrap_or("");
    if trimmed.is_empty() {
        return "agent".to_string();
    }
    match trimmed.to_ascii_lowercase().as_str() {
        "prompt" | "agent" | "agent_prompt" | "prompt_automation" => "agent".to_string(),
        "html" | "search" | "rss" | "json" | "icalendar" | "file" | "folder" | "script"
        | "reminder" => trimmed.to_ascii_lowercase(),
        _ => trimmed.to_string(),
    }
}

/// Strictly normalize a `data_source` record so it always satisfies `dataSources.ts`'s `parseSource`
/// validator (`id`, `name`, `kind`, `location`, `accessPolicy`, and optional `validationContract`).
pub fn normalize_data_source_record(raw: &Value) -> Value {
    const MAX_JS_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
    let id = raw
        .get("id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(new_id);
    let name = raw
        .get("name")
        .or_else(|| raw.get("title"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(&id)
        .to_string();
    let kind_raw = raw
        .get("kind")
        .or_else(|| raw.get("type"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("localFile");
    let kind = match kind_raw.to_ascii_lowercase().as_str() {
        "localfile" | "local_file" | "file" | "local" => "localFile".to_string(),
        "http" | "https" | "url" | "web" | "api" => "http".to_string(),
        _ => kind_raw.to_string(),
    };
    let location = raw
        .get("location")
        .or_else(|| raw.get("path"))
        .or_else(|| raw.get("url"))
        .or_else(|| raw.get("endpoint"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .unwrap_or("")
        .to_string();
    let policy_raw = raw
        .get("accessPolicy")
        .or_else(|| raw.get("access_policy"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("conversationOptIn");
    let access_policy = match policy_raw.to_ascii_lowercase().as_str() {
        "conversationoptin" | "conversation_opt_in" | "opt_in" => "conversationOptIn".to_string(),
        "askwhenrelevant" | "ask_when_relevant" | "ask" => "askWhenRelevant".to_string(),
        "defaultenabled" | "default_enabled" | "default" => "defaultEnabled".to_string(),
        "blocked" | "deny" | "disabled" => "blocked".to_string(),
        _ => policy_raw.to_string(),
    };
    let max_age = raw
        .get("maximumAgeMinutes")
        .or_else(|| raw.get("maximum_age_minutes"))
        .or_else(|| raw.pointer("/validationContract/maximumAgeMinutes"))
        .and_then(|v| v.as_i64())
        .unwrap_or(60)
        .clamp(1, 10_080);
    let updated_at = raw
        .get("updatedAt")
        .or_else(|| raw.get("updated_at"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| Utc::now().to_rfc3339());

    let mut out = serde_json::Map::new();
    out.insert("id".into(), json!(id));
    out.insert("name".into(), json!(name));
    out.insert("kind".into(), json!(kind));
    out.insert("location".into(), json!(location));
    out.insert("accessPolicy".into(), json!(access_policy));
    out.insert("maximumAgeMinutes".into(), json!(max_age));
    out.insert("updatedAt".into(), json!(updated_at));

    if let Some(last_err) = raw
        .get("lastError")
        .or_else(|| raw.get("last_error"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        out.insert("lastError".into(), json!(last_err));
    }

    if let Some(vc) = raw.get("validationContract").and_then(|v| v.as_object()) {
        let reader_description = vc
            .get("readerDescription")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string)
            .unwrap_or_else(|| format!("{kind} reader"));
        let schema_fields: Vec<String> = vc
            .get("schemaFields")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| item.as_str().map(ToString::to_string))
                    .collect()
            })
            .unwrap_or_else(|| vec!["content".to_string()]);
        let redacted_sample = vc
            .get("redactedSample")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let record_count = vc
            .get("recordCount")
            .and_then(|v| {
                v.as_u64()
                    .or_else(|| v.as_i64().filter(|&n| n >= 0).map(|n| n as u64))
            })
            .unwrap_or(0)
            .min(MAX_JS_SAFE_INTEGER);
        let contract_max_age = vc
            .get("maximumAgeMinutes")
            .and_then(|v| v.as_i64())
            .unwrap_or(max_age)
            .clamp(1, 10_080);
        let raw_state = vc
            .get("validationState")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("failed");
        let validation_state = match raw_state.to_ascii_lowercase().as_str() {
            "ok" | "verified" => "verified".to_string(),
            "empty" | "verifiedempty" | "verified_empty" => "verifiedEmpty".to_string(),
            "error" | "failed" => "failed".to_string(),
            "blocked" => "blocked".to_string(),
            _ => raw_state.to_string(),
        };
        let validated_at = vc
            .get("validatedAt")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string)
            .unwrap_or_else(|| updated_at.clone());
        let mut contract = serde_json::Map::new();
        contract.insert("sourceId".into(), json!(id));
        contract.insert("readerDescription".into(), json!(reader_description));
        contract.insert("schemaFields".into(), json!(schema_fields));
        contract.insert("redactedSample".into(), json!(redacted_sample));
        contract.insert("recordCount".into(), json!(record_count));
        contract.insert("maximumAgeMinutes".into(), json!(contract_max_age));
        contract.insert("validationState".into(), json!(validation_state));
        contract.insert("validatedAt".into(), json!(validated_at));
        if let Some(err_msg) = vc
            .get("errorMessage")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            contract.insert("errorMessage".into(), json!(err_msg));
        }
        if let Some(recipe) = vc.get("readRecipe").filter(|v| !v.is_null()) {
            contract.insert("readRecipe".into(), recipe.clone());
        }
        out.insert("validationContract".into(), Value::Object(contract));
    }

    Value::Object(out)
}

pub fn normalize_automation_record(raw: &Value) -> Value {
    let id = raw
        .get("id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(new_id);
    let name = raw
        .get("name")
        .or_else(|| raw.get("title"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("Automation");
    let format = canonical_automation_format(raw.get("format").and_then(|v| v.as_str()));
    let url = if format == "agent" || format == "script" {
        ""
    } else {
        raw.get("url")
            .or_else(|| raw.get("path"))
            .or_else(|| raw.get("query"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
    };
    let mut prompt = raw
        .get("userPrompt")
        .or_else(|| raw.get("prompt"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let (script_approved, script_cwd) = if format == "script" {
        let cwd_opt = raw
            .get("cwd")
            .or_else(|| raw.get("scriptCwd"))
            .and_then(|v| v.as_str());
        let cwd = cwd_opt.unwrap_or("");
        if let Some(script) = raw.get("script").and_then(|v| v.as_str()) {
            prompt = serde_json::to_string(&json!({
                "kind": "script_automation",
                "runtime": "bash",
                "script": script,
                "cwd": cwd
            }))
            .unwrap_or(prompt);
        }
        let approved = raw
            .get("scriptApproved")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let cwd_val = cwd_opt
            .map(|c| json!(c))
            .unwrap_or_else(|| raw.get("scriptCwd").cloned().unwrap_or(Value::Null));
        (json!(approved), cwd_val)
    } else {
        (Value::Null, Value::Null)
    };
    let frequency = raw
        .get("frequency")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("daily");
    let weekday = if frequency == "weekly" {
        raw.get("weekday").cloned().unwrap_or(Value::Null)
    } else {
        Value::Null
    };
    let time = raw
        .get("time")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("09:00");
    let timezone = raw
        .get("timezone")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("local");
    let enabled = raw
        .get("enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let updated_at = raw
        .get("updatedAt")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| Utc::now().to_rfc3339());
    let revision = raw
        .get("revision")
        .and_then(|v| v.as_i64())
        .unwrap_or(1)
        .max(1);
    json!({
        "id": id,
        "name": name,
        "url": url,
        "format": format,
        "userPrompt": prompt,
        "revision": revision,
        "frequency": frequency,
        "weekday": weekday,
        "time": time,
        "timezone": timezone,
        "enabled": enabled,
        "updatedAt": updated_at,
        "scriptApproved": script_approved,
        "scriptCwd": script_cwd,
        "latestResult": raw.get("latestResult").cloned().unwrap_or(Value::Null),
        "executionSpec": raw.get("executionSpec").cloned().unwrap_or(Value::Null)
    })
}

pub fn build_dashboard_snapshot(suite: &ModulesSuite) -> Value {
    let raw_items: Vec<Value> = suite.list_records("automation").unwrap_or_default();
    let (success_count, failed_count, mut earliest_next_ms) =
        automation_stats_summary(&suite.store);
    let now = Utc::now();

    let tasks: Vec<Value> = raw_items
        .iter()
        .map(|item| {
            let norm = normalize_automation_record(item);
            let id = norm["id"].as_str().unwrap_or("");
            let enabled = norm["enabled"].as_bool().unwrap_or(true);
            let persisted_next = suite
                .store
                .get_module_record::<AutomationRunStats>(KIND_AUTOMATION_STATS, id)
                .ok()
                .flatten()
                .and_then(|st| st.next_scheduled_at_ms);
            let task_next_ms = if enabled {
                persisted_next.or_else(|| {
                    parse_automation_value(item, &suite.workspace_root)
                        .and_then(|rec| next_scheduled_at_ms(&rec.schedule, now))
                })
            } else {
                None
            };
            if let Some(ms) = task_next_ms {
                earliest_next_ms = Some(match earliest_next_ms {
                    Some(cur) => cur.min(ms),
                    None => ms,
                });
            }
            json!({
                "sourceId": norm["id"],
                "name": norm["name"],
                "format": norm["format"],
                "enabled": enabled,
                "scheduleFrequency": norm["frequency"],
                "scheduledTimeISO": norm["time"],
                "scheduledWeekday": norm["weekday"],
                "nextScheduledAt": task_next_ms.map(ms_to_rfc3339),
                "latestResult": norm["latestResult"]
            })
        })
        .collect();

    let total = tasks.len();
    let next_scheduled_iso = earliest_next_ms.map(ms_to_rfc3339);
    json!({
        "tasks": tasks,
        "stats": {
            "totalTasks": total,
            "successCount": success_count,
            "failedCount": failed_count,
            "nextScheduledAt": next_scheduled_iso
        },
        "nextScheduledAt": next_scheduled_iso,
        "backend": "cli-rust",
        "localFirst": true
    })
}

/// Run a git command with a strict wall-clock timeout and bounded stdout capture.
/// Returns `Ok((stdout_text, truncated))` on exit 0, or `Err(error_message)` on spawn error,
/// timeout, or non-zero exit code.
fn run_git_bounded(
    root: &Path,
    args: &[&str],
    timeout: Duration,
    max_bytes: usize,
) -> Result<(String, bool), String> {
    let mut child = Command::new("git")
        .args(args)
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to spawn git {}: {e}", args.join(" ")))?;

    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();

    let out_handle = std::thread::spawn(move || -> (Vec<u8>, bool) {
        let mut buf = Vec::new();
        if let Some(pipe) = stdout_pipe {
            let cap = (max_bytes as u64).saturating_add(1);
            let _ = pipe.take(cap).read_to_end(&mut buf);
        }
        let truncated = buf.len() > max_bytes;
        if truncated {
            buf.truncate(max_bytes);
        }
        (buf, truncated)
    });
    let err_handle = std::thread::spawn(move || -> Vec<u8> {
        let mut buf = Vec::new();
        if let Some(pipe) = stderr_pipe {
            let _ = pipe.take(16 * 1024).read_to_end(&mut buf);
        }
        buf
    });

    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break st,
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = out_handle.join();
                    let _ = err_handle.join();
                    return Err(format!(
                        "git {} timed out after {}s",
                        args.join(" "),
                        timeout.as_secs()
                    ));
                }
                std::thread::sleep(Duration::from_millis(15));
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("git {} wait failed: {e}", args.join(" ")));
            }
        }
    };

    let (stdout_bytes, truncated) = out_handle.join().unwrap_or_default();
    let stderr_bytes = err_handle.join().unwrap_or_default();

    if !status.success() {
        let err_text = String::from_utf8_lossy(&stderr_bytes).trim().to_string();
        let msg = if err_text.is_empty() {
            format!("git {} exited with status {}", args.join(" "), status)
        } else {
            err_text
        };
        return Err(msg);
    }

    Ok((String::from_utf8_lossy(&stdout_bytes).into_owned(), truncated))
}

pub fn build_workspace_review(host_id: &str, conversation_id: &str, root: &Path) -> Value {
    let is_git = root.join(".git").exists()
        || run_git_bounded(
            root,
            &["rev-parse", "--is-inside-work-tree"],
            Duration::from_secs(2),
            256,
        )
        .map(|(out, _)| out.trim() == "true")
        .unwrap_or(false);

    if !is_git {
        let mut files = Vec::new();
        let mut truncated = false;
        let mut dir_error: Option<String> = None;
        match std::fs::read_dir(root) {
            Ok(entries) => {
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if name.starts_with('.') {
                        continue;
                    }
                    if files.len() >= MAX_REVIEW_FILES {
                        truncated = true;
                        break;
                    }
                    if let Ok(meta) = entry.metadata() {
                        let modified_ms = meta
                            .modified()
                            .ok()
                            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                            .map(|d| d.as_millis() as i64)
                            .unwrap_or_else(now_ms);
                        files.push(json!({
                            "path": name,
                            "size": meta.len(),
                            "is_dir": meta.is_dir(),
                            "modified_at_ms": modified_ms
                        }));
                    }
                }
            }
            Err(e) => {
                dir_error = Some(format!("failed to read workspace directory: {e}"));
            }
        }
        return json!({
            "source_host_id": host_id,
            "conversation_id": conversation_id,
            "workspace_root": root.display().to_string(),
            "is_git_repository": false,
            "is_session_sandbox": false,
            "truncated": truncated,
            "error": dir_error,
            "files": files
        });
    }

    let status_res = run_git_bounded(
        root,
        &["status", "--porcelain=v1", "-uall"],
        GIT_REVIEW_TIMEOUT,
        MAX_REVIEW_DIFF_BYTES,
    );
    let (entries, status_truncated, status_error) = match status_res {
        Ok((status_out, bytes_trunc)) => {
            let mut entries = Vec::new();
            let mut files_trunc = bytes_trunc;
            for line in status_out.lines().filter(|l| l.len() >= 4) {
                if entries.len() >= MAX_REVIEW_FILES {
                    files_trunc = true;
                    break;
                }
                let bytes = line.as_bytes();
                entries.push(json!({
                    "path": line[3..].trim(),
                    "index_status": (bytes[0] as char).to_string(),
                    "worktree_status": (bytes[1] as char).to_string()
                }));
            }
            (entries, files_trunc, Value::Null)
        }
        Err(err) => (Vec::new(), false, Value::String(err)),
    };

    let (unstaged_obj, unstaged_trunc, unstaged_err) =
        match run_git_bounded(root, &["diff"], GIT_REVIEW_TIMEOUT, MAX_REVIEW_DIFF_BYTES) {
            Ok((text, trunc)) => (
                json!({ "text": text, "error": Value::Null, "truncated": trunc }),
                trunc,
                None,
            ),
            Err(err) => (
                json!({ "text": Value::Null, "error": err, "truncated": false }),
                false,
                Some(err),
            ),
        };

    let (staged_obj, staged_trunc, staged_err) = match run_git_bounded(
        root,
        &["diff", "--cached"],
        GIT_REVIEW_TIMEOUT,
        MAX_REVIEW_DIFF_BYTES,
    ) {
        Ok((text, trunc)) => (
            json!({ "text": text, "error": Value::Null, "truncated": trunc }),
            trunc,
            None,
        ),
        Err(err) => (
            json!({ "text": Value::Null, "error": err, "truncated": false }),
            false,
            Some(err),
        ),
    };

    let any_truncated = status_truncated || unstaged_trunc || staged_trunc;
    let top_error = if !status_error.is_null() {
        status_error.clone()
    } else if let Some(e) = staged_err.or(unstaged_err) {
        Value::String(e)
    } else {
        Value::Null
    };

    json!({
        "source_host_id": host_id,
        "conversation_id": conversation_id,
        "workspace_root": root.display().to_string(),
        "is_git_repository": true,
        "is_session_sandbox": false,
        "truncated": any_truncated,
        "error": top_error,
        "review": {
            "inventory": {
                "entries": entries,
                "truncated": any_truncated,
                "error": status_error
            },
            "staged": staged_obj,
            "unstaged": unstaged_obj
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        bind_store_vault_path, save_applied_config, ConnectionConfig, DefaultModelSelection,
        LlmSection, ModelEntry,
    };
    use nola_modules::scheduler::record_automation_run_stats;

    #[test]
    fn conversation_json_defaults_to_cautious_and_includes_knowledge() {
        let store = Store::open_in_memory().unwrap();
        let sess = Session {
            id: "conv-1".into(),
            title: "Test".into(),
            workspace_root: "/tmp".into(),
            model_connection: None,
            model_id: None,
            created_at_ms: 1000,
            updated_at_ms: 1000,
        };
        store.upsert_session(&sess).unwrap();

        let proj = conversation_configuration_json(&store, &sess);
        assert_eq!(proj["permissionProfile"], "cautious");
        assert!(proj["knowledge"].is_object());
        assert_eq!(proj["knowledge"]["keyPointsMarkdown"], "");
        assert_eq!(proj["knowledge"]["wikiMarkdown"], "");
        assert!(proj["knowledge"]["entries"].is_array());

        // Known profile (`accept_edits`) is preserved; unknown/legacy profile normalizes to cautious
        store
            .set_kv(&session_meta_key("conv-1"), &json!({"permissionProfile": "accept_edits"}))
            .unwrap();
        let proj2 = conversation_configuration_json(&store, &sess);
        assert_eq!(proj2["permissionProfile"], "accept_edits");

        store
            .set_kv(&session_meta_key("conv-1"), &json!({"permissionProfile": "unknown_legacy_mode"}))
            .unwrap();
        let proj3 = conversation_configuration_json(&store, &sess);
        assert_eq!(proj3["permissionProfile"], "cautious");
    }

    #[test]
    fn model_profiles_redacts_api_keys_and_uses_honest_capabilities() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory().unwrap();
        bind_store_vault_path(&store, &dir.path().join("vault.json")).unwrap();
        let cfg = AppliedConfig {
            llm: LlmSection {
                default_model: Some(DefaultModelSelection {
                    connection: "custom".into(),
                    model: "my-local-7b".into(),
                }),
                connections: vec![ConnectionConfig {
                    id: "custom".into(),
                    endpoint: "http://127.0.0.1:8080/v1".into(),
                    protocol: Some("chat_completions".into()),
                    provider: None,
                    api_key: Some("sk-never-leak-me".into()),
                    api_key_env: None,
                    models: vec![ModelEntry {
                        id: "my-local-7b".into(),
                        context_window: Some(32_768),
                        max_output_tokens: None,
                        temperature: None,
                        top_p: None,
                        reasoning_effort: None,
                        capabilities: None,
                    }],
                }],
            },
            ..Default::default()
        };
        save_applied_config(&store, &cfg).unwrap();

        let resp = build_model_profiles_response(&store);
        let raw = serde_json::to_string(&resp).unwrap();
        assert!(!raw.contains("sk-never-leak-me"));
        assert_eq!(resp["llm"]["connections"][0]["hasApiKey"], true);
        assert_eq!(resp["profiles"][0]["executionCapabilities"]["imageInput"], false);
        assert_eq!(resp["profiles"][0]["executionCapabilities"]["reasoning"], false);
    }

    #[test]
    fn dashboard_snapshot_uses_real_automation_stats() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory().unwrap();
        bind_store_vault_path(&store, &dir.path().join("vault.json")).unwrap();
        save_applied_config(&store, &AppliedConfig::default()).unwrap();
        let suite = ModulesSuite::new(store.clone(), dir.path().to_path_buf(), None);
        suite
            .upsert_record(
                "automation",
                "a1",
                &json!({"id": "a1", "name": "Digest", "frequency": "daily", "time": "09:00", "enabled": true}),
            )
            .unwrap();
        record_automation_run_stats(&store, "a1", "0 9 * * *", Utc::now(), true, None).unwrap();
        record_automation_run_stats(&store, "a1", "0 9 * * *", Utc::now(), false, Some("err"))
            .unwrap();

        let snap = build_dashboard_snapshot(&suite);
        assert_eq!(snap["stats"]["successCount"], 1);
        assert_eq!(snap["stats"]["failedCount"], 1);
        assert!(snap["stats"]["nextScheduledAt"].is_string());
        assert!(snap["tasks"][0]["nextScheduledAt"].is_string());
    }

    #[test]
    fn agent_settings_snapshot_includes_host_identity_and_all_fields() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory().unwrap();
        bind_store_vault_path(&store, &dir.path().join("vault.json")).unwrap();
        save_applied_config(&store, &AppliedConfig::default()).unwrap();
        let suite = ModulesSuite::new(store.clone(), dir.path().to_path_buf(), None);
        store
            .upsert_mcp_server(&nola_types::McpServerConfig {
                id: "srv-1".into(),
                name: "Local MCP".into(),
                transport: nola_types::McpTransport::StreamableHttp,
                enabled: true,
                command: None,
                args: vec![],
                env: Default::default(),
                url: Some("http://127.0.0.1:9000/mcp".into()),
                headers: Default::default(),
                oauth_client_id: None,
                oauth_scopes: vec![],
            })
            .unwrap();

        let snap = build_agent_settings_snapshot(&store, &suite);
        let hid = snap["host_identity"]["id"].as_str().unwrap();
        assert!((16..=128).contains(&hid.len()));
        assert_eq!(snap["host_identity"]["permission_protocol_version"], 1);
        assert_eq!(snap["uploaded_skills"], snap["skills"]);
        assert_eq!(snap["uploaded_documents"], json!([]));
        assert_eq!(snap["messaging"]["management_path"], "/settings?section=messaging");
        assert_eq!(snap["computer_use"]["available"], false);
        assert_eq!(snap["computer_use"]["feature_enabled"], false);
        assert_eq!(snap["computer_use"]["token_configured"], false);
        assert_eq!(snap["computer_use"]["connected"], false);
        assert_eq!(snap["native_browser"]["available"], true);
        assert_eq!(snap["native_browser"]["feature_enabled"], true);
        assert!(snap["native_browser"]["token_configured"].is_boolean());
        assert_eq!(snap["native_browser"]["connected"], false);
        assert_eq!(snap["default_chat_model_revision"], 1);
        assert!(snap["model_profiles"].is_array());
        assert!(snap["default_chat_model_resolution"].is_object());
        assert_eq!(snap["default_chat_model_resolution"]["kind"], "no_automatic_candidate");
        assert_eq!(snap["default_chat_model_resolution"]["origin"], "default");
        assert_eq!(snap["execution_backends"][0]["id"], "nola");
        assert_eq!(snap["execution_backends"][0]["availability"], "available");
        assert_eq!(snap["execution_backends"][0]["capabilities"]["streams_text"], true);
        assert_eq!(snap["execution_backends"][0]["capabilities"]["streams_reasoning"], true);
        assert_eq!(snap["execution_backends"][0]["capabilities"]["streams_tools"], true);
        assert_eq!(snap["execution_backends"][0]["capabilities"]["supports_resume"], true);
        assert_eq!(snap["execution_backends"][0]["capabilities"]["supports_cancel"], true);
        assert_eq!(snap["execution_backends"][0]["capabilities"]["supports_permission_bridge"], true);
        assert_eq!(snap["mcp_servers"][0]["endpoint"], "http://127.0.0.1:9000/mcp");
        assert_eq!(snap["mcp_servers"][0]["url"], "http://127.0.0.1:9000/mcp");
        let epoch = snap["source_store_epoch"].as_str().unwrap();
        assert!(!epoch.is_empty());
        assert_eq!(snap["host_identity"]["source_store_epoch"], epoch);
        assert_eq!(ensure_source_store_epoch(&store), epoch);
        assert_eq!(snap["messaging"]["worker"]["execution_backend"], "nola");
    }

    #[test]
    fn memory_meta_and_conversation_message_count_and_profile_aliases_persist() {
        let store = Store::open_in_memory().unwrap();
        let sess = Session {
            id: "conv-cnt".into(),
            title: "Count Test".into(),
            workspace_root: "/tmp".into(),
            model_connection: None,
            model_id: None,
            created_at_ms: 1000,
            updated_at_ms: 1000,
        };
        store.upsert_session(&sess).unwrap();
        store
            .append_event("conv-cnt", "chat_user", json!({"content": "hi"}), 100)
            .unwrap();
        store
            .append_event("conv-cnt", "chat_assistant", json!({"content": "hello"}), 100)
            .unwrap();
        let proj = session_to_conversation_json(&store, &sess);
        assert_eq!(proj["message_count"], 2);

        let mem = MemoryItem {
            id: "mem-meta-1".into(),
            namespace: "global:preference".into(),
            title: "Editor".into(),
            content: "Prefers neovim".into(),
            tags: vec!["dev".into()],
            created_at_ms: 1000,
            updated_at_ms: 2000,
        };
        store
            .set_kv(
                &memory_meta_key("mem-meta-1"),
                &json!({
                    "supersedes": ["mem-old-0"],
                    "confidence": 0.9,
                    "importance": 0.85,
                    "sensitivity": "private",
                    "expires_at": "2027-01-01T00:00:00Z",
                    "source": "user"
                }),
            )
            .unwrap();
        let rec = memory_item_to_record_with_store(&store, &mem);
        assert_eq!(rec["supersedes"], json!(["mem-old-0"]));
        assert_eq!(rec["confidence"], 0.9);
        assert_eq!(rec["importance"], 0.85);
        assert_eq!(rec["sensitivity"], "private");
        assert_eq!(rec["expires_at"], "2027-01-01T00:00:00Z");

        // Ensure model profile response includes snake_case aliases alongside camelCase
        let dir = tempfile::tempdir().unwrap();
        bind_store_vault_path(&store, &dir.path().join("vault.json")).unwrap();
        save_applied_config(
            &store,
            &AppliedConfig {
                llm: LlmSection {
                    default_model: Some(DefaultModelSelection {
                        connection: "ep".into(),
                        model: "gpt-4o".into(),
                    }),
                    connections: vec![ConnectionConfig {
                        id: "ep".into(),
                        endpoint: "http://127.0.0.1:8080/v1".into(),
                        protocol: Some("chat_completions".into()),
                        provider: None,
                        api_key: None,
                        api_key_env: None,
                        models: vec![ModelEntry {
                            id: "gpt-4o".into(),
                            context_window: Some(128_000),
                            max_output_tokens: None,
                            temperature: None,
                            top_p: None,
                            reasoning_effort: None,
                            capabilities: None,
                        }],
                    }],
                },
                ..Default::default()
            },
        )
        .unwrap();
        let profs = build_model_profiles_response(&store);
        assert_eq!(profs["profiles"][0]["profile_id"], "ep:gpt-4o");
        assert_eq!(profs["profiles"][0]["display_name"], "gpt-4o");
        assert_eq!(profs["profiles"][0]["model_name"], "gpt-4o");
    }

    #[test]
    fn memory_item_to_record_includes_provenance_and_script_automation_normalizes() {
        let mem = MemoryItem {
            id: "m1".into(),
            namespace: "global:fact".into(),
            title: "Pref".into(),
            content: "Likes Rust".into(),
            tags: vec!["rust".into()],
            created_at_ms: 1000,
            updated_at_ms: 2000,
        };
        let rec = memory_item_to_record(&mem);
        assert_eq!(rec["provenance"]["kind"], "user");
        assert_eq!(rec["provenance"]["source"], "user");
        assert_eq!(rec["provenance"]["observed_at"], rec["updated_at"]);

        let norm = normalize_automation_record(&json!({
            "id": "auto-1",
            "name": "Backup",
            "format": "script",
            "script": "echo hi",
            "cwd": "/tmp"
        }));
        let decoded: Value = serde_json::from_str(norm["userPrompt"].as_str().unwrap()).unwrap();
        assert_eq!(decoded["kind"], "script_automation");
        assert_eq!(decoded["runtime"], "bash");
        assert_eq!(decoded["script"], "echo hi");
        assert_eq!(decoded["cwd"], "/tmp");
        assert_eq!(norm["scriptCwd"], "/tmp");
        assert_eq!(norm["scriptApproved"], false);
    }

    #[test]
    fn data_source_normalization_passes_strict_frontend_contract() {
        // Legacy/partial record missing accessPolicy, using title/path, and broken contract fields
        let norm = normalize_data_source_record(&json!({
            "id": "ds-1",
            "title": "Project Notes",
            "kind": "local_file",
            "path": "/tmp/notes.md",
            "validationContract": {
                "sourceId": "wrong-id",
                "recordCount": -5,
                "maximumAgeMinutes": 0,
                "schemaFields": ["content", 123],
                "validationState": "ok",
                "errorMessage": null
            }
        }));
        for key in ["id", "name", "kind", "location", "accessPolicy"] {
            assert!(
                norm.get(key).and_then(|v| v.as_str()).is_some(),
                "missing required string field {key}: {norm}"
            );
        }
        assert_eq!(norm["id"], "ds-1");
        assert_eq!(norm["name"], "Project Notes");
        assert_eq!(norm["kind"], "localFile");
        assert_eq!(norm["location"], "/tmp/notes.md");
        assert_eq!(norm["accessPolicy"], "conversationOptIn");

        let vc = norm["validationContract"].as_object().unwrap();
        assert_eq!(vc["sourceId"], "ds-1");
        assert!(vc["readerDescription"].is_string());
        assert!(vc["redactedSample"].is_string());
        assert_eq!(vc["validationState"], "verified");
        assert!(vc["validatedAt"].as_str().is_some_and(|s| !s.is_empty()));
        assert_eq!(vc["schemaFields"], json!(["content"]));
        assert_eq!(vc["recordCount"], 0);
        assert_eq!(vc["maximumAgeMinutes"], 1);
        assert!(vc.get("errorMessage").is_none());

        // Minimal newly created record without validationContract
        let minimal = normalize_data_source_record(&json!({
            "name": "API Feed",
            "kind": "http",
            "location": "https://example.com/feed.json"
        }));
        assert!(minimal["id"].as_str().is_some_and(|s| !s.is_empty()));
        assert_eq!(minimal["name"], "API Feed");
        assert_eq!(minimal["kind"], "http");
        assert_eq!(minimal["location"], "https://example.com/feed.json");
        assert_eq!(minimal["accessPolicy"], "conversationOptIn");
        assert!(minimal.get("validationContract").is_none());
    }

    #[test]
    fn builtin_skills_have_builtin_source_and_non_custom_category() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory().unwrap();
        bind_store_vault_path(&store, &dir.path().join("vault.json")).unwrap();
        save_applied_config(&store, &AppliedConfig::default()).unwrap();
        let suite = ModulesSuite::new(store.clone(), dir.path().to_path_buf(), None);

        // Fresh discovery
        let snap = build_agent_settings_snapshot(&store, &suite);
        let skills = snap["skills"].as_array().unwrap();
        for name in ["code-review", "debugging", "tdd"] {
            let sk = skills
                .iter()
                .find(|s| s["name"] == name)
                .unwrap_or_else(|| panic!("expected built-in skill {name}"));
            assert_eq!(sk["built_in"], true);
            assert_eq!(sk["source"], "built_in");
            assert_eq!(sk["category_id"], "workspace");
            assert!(sk["default_content"].as_str().is_some_and(|s| !s.is_empty()));
        }

        // Legacy persisted agent_settings with source="discovered" upgrades cleanly
        store
            .set_kv(
                "agent_settings",
                &json!({
                    "skills": [
                        {"id": "code-review", "name": "code-review", "source": "discovered", "enabled": true},
                        {"id": "my-custom", "name": "my-custom", "source": "custom", "enabled": true}
                    ]
                }),
            )
            .unwrap();
        let upgraded = build_agent_settings_snapshot(&store, &suite);
        let upgraded_skills = upgraded["skills"].as_array().unwrap();
        let cr = upgraded_skills.iter().find(|s| s["name"] == "code-review").unwrap();
        assert_eq!(cr["built_in"], true);
        assert_eq!(cr["source"], "built_in");
        assert_eq!(cr["category_id"], "workspace");
        assert!(cr["default_content"].as_str().is_some_and(|s| !s.is_empty()));

        let custom = upgraded_skills.iter().find(|s| s["name"] == "my-custom").unwrap();
        assert_eq!(custom["built_in"], false);
        assert_eq!(custom["category_id"], "custom");
    }

    #[test]
    fn prompt_automation_normalizes_to_canonical_agent_format_and_multistep_messages_reconstruct() {
        let prompt_auto = normalize_automation_record(&json!({
            "id": "auto-prompt-1",
            "name": "Daily Briefing",
            "format": "prompt",
            "userPrompt": "Summarize open PRs",
            "frequency": "daily",
            "time": "08:30"
        }));
        assert_eq!(prompt_auto["format"], "agent");
        assert_eq!(prompt_auto["url"], "");
        assert_eq!(prompt_auto["userPrompt"], "Summarize open PRs");

        // Multi-step turn_completed + chat_assistant deduplicates in-place without losing Markdown
        let store = Store::open_in_memory().unwrap();
        store
            .append_event("conv-multi", "chat_user", json!({"content": "show table"}), 100)
            .unwrap();
        store
            .append_event(
                "conv-multi",
                "turn_completed",
                json!({"final_text": "| a | b |\n|---|---|\n| 1 | 2 |\n\nRendered UI."}),
                100,
            )
            .unwrap();
        store
            .append_event(
                "conv-multi",
                "chat_assistant",
                json!({
                    "content": "| a | b |\n|---|---|\n| 1 | 2 |\n\nRendered UI.",
                    "reasoning": "checked metrics",
                    "toolCalls": [{"callId": "c1", "name": "render_ui_surface", "ok": true, "summary": "rendered"}],
                    "uiSurfaces": [],
                    "roomId": "room-1",
                    "expertReplies": [{"expert_id": "exp-1", "expert_name": "Architect", "content": "Looks good"}]
                }),
                100,
            )
            .unwrap();
        let (msgs, _) = reconstruct_v2_messages(&store, "conv-multi");
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[1]["role"], "assistant");
        assert_eq!(msgs[1]["content"], "| a | b |\n|---|---|\n| 1 | 2 |\n\nRendered UI.");
        assert_eq!(msgs[1]["reasoning"], "checked metrics");
        assert_eq!(msgs[1]["toolCalls"][0]["name"], "render_ui_surface");
        assert_eq!(msgs[1]["roomId"], "room-1");
        assert_eq!(msgs[1]["expertReplies"][0]["expert_name"], "Architect");
    }
}
