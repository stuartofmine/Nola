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
    for ev in &events {
        raw_events.push(serde_json::to_value(ev).unwrap_or(json!({})));
        match ev.kind.as_str() {
            "user_message" | "chat_user" => {
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
                messages.push(json!({
                    "id": format!("msg-{}", ev.seq),
                    "seq": ev.seq,
                    "role": "user",
                    "content": text,
                    "reasoning": "",
                    "toolCalls": [],
                    "attachments": attachments,
                    "createdAtMs": ev.created_at_ms
                }));
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
                let usage = ev.payload.get("usage").cloned();
                let has_surfaces = ui_surfaces.as_array().is_some_and(|a| !a.is_empty());
                if !text.is_empty() || !reasoning.is_empty() || has_surfaces {
                    let mut entry = json!({
                        "id": format!("msg-{}", ev.seq),
                        "seq": ev.seq,
                        "role": "assistant",
                        "content": text,
                        "reasoning": reasoning,
                        "toolCalls": tool_calls,
                        "uiSurfaces": ui_surfaces,
                        "createdAtMs": ev.created_at_ms
                    });
                    if let Some(u) = usage {
                        entry["usage"] = u;
                    }
                    // WebHost emits `chat_assistant` right after `AgentEngine`'s `turn_completed`
                    // with richer `reasoning`, `toolCalls`, and `uiSurfaces`; upgrade in-place.
                    let should_replace_prev = ev.kind == "chat_assistant"
                        && messages.last().is_some_and(|prev| {
                            prev.get("role").and_then(|v| v.as_str()) == Some("assistant")
                                && prev.get("content").and_then(|v| v.as_str()) == Some(text)
                                && prev
                                    .get("reasoning")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .is_empty()
                        });
                    if should_replace_prev {
                        if let Some(last) = messages.last_mut() {
                            *last = entry;
                        }
                    } else {
                        messages.push(entry);
                    }
                }
            }
            _ => {}
        }
    }
    (messages, raw_events)
}

pub fn memory_item_to_record(item: &MemoryItem) -> Value {
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
    json!({
        "id": item.id,
        "scope": scope,
        "kind": kind,
        "content": item.content,
        "summary": if item.title.is_empty() { Value::Null } else { Value::String(item.title.clone()) },
        "tags": item.tags,
        "source": "user",
        "confidence": 1.0,
        "supersedes": [],
        "importance": 0.5,
        "sensitivity": Value::Null,
        "expires_at": Value::Null,
        "created_at": created,
        "updated_at": updated
    })
}

pub fn build_agent_settings_snapshot(store: &Store, suite: &ModulesSuite) -> Value {
    let saved = store
        .get_kv("agent_settings")
        .ok()
        .flatten()
        .unwrap_or_else(|| json!({}));
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

    let skills: Vec<Value> = if let Some(arr) = saved.get("skills").and_then(|v| v.as_array()) {
        arr.clone()
    } else {
        nola_modules::skills::discover_skills(&suite.workspace_root, suite.user_home.as_deref())
            .into_iter()
            .map(|s| {
                json!({
                    "id": s.name,
                    "name": s.name,
                    "display_name": s.name,
                    "description": s.description,
                    "content": s.body,
                    "enabled": true,
                    "source": "discovered",
                    "revision": 1
                })
            })
            .collect()
    };

    let mcp_servers: Vec<Value> =
        if let Some(arr) = saved.get("mcp_servers").and_then(|v| v.as_array()) {
            arr.clone()
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
                        "command": srv.command,
                        "args": srv.args,
                        "revision": 1
                    })
                })
                .collect()
        };

    json!({
        "permission_profile": permission_profile,
        "custom_system_prompt": custom_system_prompt,
        "enable_local_dev_tools": enable_local_dev_tools,
        "allowed_dirs": allowed_dirs,
        "bash_prefixes": bash_prefixes,
        "auto_recall_memory": auto_recall_memory,
        "skills": skills,
        "mcp_servers": mcp_servers,
        "skill_settings_backup_key": Value::Null,
        "execution_backends": [
            {
                "id": "nola-v2",
                "display_name": "Nola v2 Rust Engine",
                "capabilities": { "supports_permission_bridge": true }
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
                        "supportsSummary": caps.supports_reasoning_summary
                    })
                } else {
                    json!({ "control": "unsupported" })
                };
                out.push(json!({
                    "id": format!("{}:{}", conn.id, m.id),
                    "provider": "openai-compatible",
                    "connectionId": Value::Null,
                    "modelId": m.id,
                    "modelName": m.id,
                    "displayName": m.id,
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

pub fn normalize_automation_record(raw: &Value) -> Value {
    let id = raw
        .get("id")
        .and_then(|v| v.as_str())
        .map(ToString::to_string)
        .unwrap_or_else(new_id);
    let name = raw
        .get("name")
        .or_else(|| raw.get("title"))
        .and_then(|v| v.as_str())
        .unwrap_or("Automation");
    let format = raw
        .get("format")
        .and_then(|v| v.as_str())
        .unwrap_or("agent");
    let url = raw
        .get("url")
        .or_else(|| raw.get("path"))
        .or_else(|| raw.get("query"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let prompt = raw
        .get("userPrompt")
        .or_else(|| raw.get("prompt"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let frequency = raw
        .get("frequency")
        .and_then(|v| v.as_str())
        .unwrap_or("daily");
    let time = raw
        .get("time")
        .and_then(|v| v.as_str())
        .unwrap_or("09:00");
    let timezone = raw
        .get("timezone")
        .and_then(|v| v.as_str())
        .unwrap_or("local");
    let enabled = raw
        .get("enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let updated_at = raw
        .get("updatedAt")
        .and_then(|v| v.as_str())
        .map(ToString::to_string)
        .unwrap_or_else(|| Utc::now().to_rfc3339());
    let revision = raw.get("revision").and_then(|v| v.as_i64()).unwrap_or(1);
    json!({
        "id": id,
        "name": name,
        "url": url,
        "format": format,
        "userPrompt": prompt,
        "revision": revision,
        "frequency": frequency,
        "weekday": raw.get("weekday").cloned().unwrap_or(Value::Null),
        "time": time,
        "timezone": timezone,
        "enabled": enabled,
        "updatedAt": updated_at,
        "scriptApproved": raw.get("scriptApproved").cloned().unwrap_or(Value::Null),
        "scriptCwd": raw.get("scriptCwd").cloned().unwrap_or(Value::Null),
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

        // Unknown/legacy profile normalizes to cautious
        store
            .set_kv(&session_meta_key("conv-1"), &json!({"permissionProfile": "accept_edits"}))
            .unwrap();
        let proj2 = conversation_configuration_json(&store, &sess);
        assert_eq!(proj2["permissionProfile"], "cautious");
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
}
