// SPDX-License-Identifier: Apache-2.0
//! Clean v2 WebHost JSON projections and workspace/CRUD helpers shared across Web & Windows clients.

use std::path::Path;
use std::process::Command;

use chrono::Utc;
use nola_modules::ModulesSuite;
use nola_store::{EventPage, Store};
use nola_types::{new_id, now_ms, MemoryItem, Session};
use serde_json::{json, Value};

use crate::config::{load_active_config, AppliedConfig};

pub fn ms_to_rfc3339(ms: i64) -> String {
    chrono::DateTime::<Utc>::from_timestamp_millis(ms)
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_else(|| Utc::now().to_rfc3339())
}

pub fn session_meta_key(id: &str) -> String {
    format!("conv_meta:{id}")
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
    json!({
        "conversationId": s.id,
        "id": s.id,
        "title": s.title,
        "isPinned": is_pinned,
        "createdAt": ms_to_rfc3339(s.created_at_ms),
        "updatedAt": ms_to_rfc3339(s.updated_at_ms),
        "selectedModelSource": s.model_connection.as_deref().unwrap_or(""),
        "selectedModelId": s.model_id.as_deref().unwrap_or(""),
        "executionBackend": "nola-v2",
        "revision": revision,
        "configurationRevision": config_rev,
        "workspace_root": s.workspace_root,
        "created_at_ms": s.created_at_ms,
        "updated_at_ms": s.updated_at_ms
    })
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
                messages.push(json!({
                    "id": format!("msg-{}", ev.seq),
                    "role": "user",
                    "content": text,
                    "reasoning": "",
                    "toolCalls": [],
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
                if !text.is_empty() || !reasoning.is_empty() {
                    let entry = json!({
                        "id": format!("msg-{}", ev.seq),
                        "role": "assistant",
                        "content": text,
                        "reasoning": reasoning,
                        "toolCalls": tool_calls,
                        "createdAtMs": ev.created_at_ms
                    });
                    // WebHost emits `chat_assistant` right after `AgentEngine`'s `turn_completed`
                    // with richer `reasoning` and `toolCalls`; upgrade in-place instead of duplicating.
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
        .unwrap_or(("global", if item.namespace.is_empty() { "preference" } else { &item.namespace }));
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
    let permission_profile = saved
        .get("permission_profile")
        .and_then(|v| v.as_str())
        .map(|p| if p == "allow_all" { "allow_all" } else { "cautious" })
        .unwrap_or("cautious");
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

    let mcp_servers: Vec<Value> = if let Some(arr) = saved.get("mcp_servers").and_then(|v| v.as_array()) {
        arr.clone()
    } else {
        store
            .list_mcp_servers()
            .unwrap_or_default()
            .into_iter()
            .map(|srv| {
                json!({
                    "server_id": srv.id,
                    "name": srv.id,
                    "display_name": srv.id,
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

pub fn build_model_profiles_response(store: &Store) -> Value {
    let cfg: AppliedConfig = load_active_config(store);
    let stored_profiles: Vec<Value> = store
        .get_kv("web_model_profiles")
        .ok()
        .flatten()
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();

    let profiles: Vec<Value> = if !stored_profiles.is_empty() {
        stored_profiles
    } else {
        let default_sel = cfg.llm.default_model.as_ref();
        let mut out = Vec::new();
        for (ep_idx, conn) in cfg.llm.connections.iter().enumerate() {
            let wire_protocol = match conn.protocol.as_deref().unwrap_or("openai_chat") {
                "anthropic" => "anthropic",
                "responses" | "openai_responses" => "responses",
                "siwc_responses" => "siwc_responses",
                _ => "chat_completions",
            };
            for (m_idx, m) in conn.models.iter().enumerate() {
                let selected = default_sel
                    .map(|d| d.connection == conn.id && d.model == m.id)
                    .unwrap_or(false);
                let capabilities = vec!["chat".to_string()];
                let exec_caps = match m.capabilities.as_ref() {
                    Some(c) => json!({
                        "toolCalls": c.tool_calls,
                        "imageInput": c.image_input,
                        "reasoning": c.reasoning,
                        "samplingParameters": c.sampling_parameters,
                        "samplingRequiresNoReasoning": false,
                        "chatOutputTokenField": "auto"
                    }),
                    None => json!({
                        "toolCalls": true,
                        "imageInput": true,
                        "reasoning": true,
                        "samplingParameters": true,
                        "samplingRequiresNoReasoning": false,
                        "chatOutputTokenField": "auto"
                    }),
                };
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
                    "executionCapabilityStatus": "tools_enabled",
                    "reasoning": { "control": "automatic" },
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
        "llm": cfg.llm
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
    let tasks: Vec<Value> = raw_items
        .iter()
        .map(|item| {
            let norm = normalize_automation_record(item);
            json!({
                "sourceId": norm["id"],
                "name": norm["name"],
                "format": norm["format"],
                "enabled": norm["enabled"],
                "scheduleFrequency": norm["frequency"],
                "scheduledTimeISO": norm["time"],
                "scheduledWeekday": norm["weekday"],
                "nextScheduledAt": Value::Null,
                "latestResult": norm["latestResult"]
            })
        })
        .collect();
    let total = tasks.len();
    json!({
        "tasks": tasks,
        "stats": {
            "totalTasks": total,
            "successCount": 0,
            "failedCount": 0
        },
        "backend": "cli-rust",
        "localFirst": true
    })
}

pub fn build_workspace_review(host_id: &str, conversation_id: &str, root: &Path) -> Value {
    let is_git = root.join(".git").exists()
        || Command::new("git")
            .args(["rev-parse", "--is-inside-work-tree"])
            .current_dir(root)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);

    if !is_git {
        let mut files = Vec::new();
        if let Ok(entries) = std::fs::read_dir(root) {
            for entry in entries.flatten().take(200) {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') {
                    continue;
                }
                if let Ok(meta) = entry.metadata() {
                    files.push(json!({
                        "path": name,
                        "size": meta.len(),
                        "is_dir": meta.is_dir(),
                        "modified_at_ms": now_ms()
                    }));
                }
            }
        }
        return json!({
            "source_host_id": host_id,
            "conversation_id": conversation_id,
            "workspace_root": root.display().to_string(),
            "is_git_repository": false,
            "is_session_sandbox": false,
            "truncated": false,
            "files": files
        });
    }

    let status_out = Command::new("git")
        .args(["status", "--porcelain=v1", "-uall"])
        .current_dir(root)
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    let entries: Vec<Value> = status_out
        .lines()
        .take(200)
        .filter(|l| l.len() >= 4)
        .map(|l| {
            let bytes = l.as_bytes();
            json!({
                "path": l[3..].trim(),
                "index_status": (bytes[0] as char).to_string(),
                "worktree_status": (bytes[1] as char).to_string()
            })
        })
        .collect();

    let unstaged = Command::new("git")
        .args(["diff"])
        .current_dir(root)
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    let staged = Command::new("git")
        .args(["diff", "--cached"])
        .current_dir(root)
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();

    json!({
        "source_host_id": host_id,
        "conversation_id": conversation_id,
        "workspace_root": root.display().to_string(),
        "is_git_repository": true,
        "is_session_sandbox": false,
        "review": {
            "inventory": {
                "entries": entries,
                "truncated": false
            },
            "staged": { "text": staged, "error": Value::Null },
            "unstaged": { "text": unstaged, "error": Value::Null }
        }
    })
}
