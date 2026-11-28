// SPDX-License-Identifier: Apache-2.0
//! Agent rooms, peer directory, role templates, expert templates, and room routing reconciliation.

use nola_modules::rooms_peers::built_in_role_templates;
use nola_modules::ModulesSuite;
use nola_store::Store;
use nola_types::{new_id, now_ms, AgentRoom, ExpertPeer, Session};
use serde_json::{json, Value};

use super::context::build_conversation_projection_response;
use super::{extract_sub_command, get_session_case_insensitive};

pub(crate) fn canonical_agent_directory_payload(store: &Store, suite: &ModulesSuite) -> Result<Value, (&'static str, String)> {
    let rooms: Vec<AgentRoom> = suite
        .list_records("agent_room")
        .map_err(|e| ("storage_error", e.to_string()))?;
    let experts: Vec<Value> = suite
        .list_records("expert_peer")
        .map_err(|e| ("storage_error", e.to_string()))?;
    let dir_rev = store
        .get_kv("agent_directory_revision")
        .ok()
        .flatten()
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let agents: Vec<Value> = experts
        .iter()
        .map(|exp| {
            let id = exp.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let name = exp.get("name").and_then(|v| v.as_str()).unwrap_or("Expert").to_string();
            let revision = exp.get("revision").and_then(|v| v.as_i64()).unwrap_or(1);
            let availability = exp
                .get("availability")
                .and_then(|v| v.as_str())
                .unwrap_or("active")
                .to_string();
            let defaults = exp
                .get("defaults")
                .filter(|v| v.is_object())
                .cloned()
                .unwrap_or_else(|| json!({}));
            let description = exp
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            json!({
                "id": id,
                "name": name,
                "revision": revision,
                "availability": availability,
                "defaults": defaults,
                "description": description,
            })
        })
        .collect();
    Ok(json!({
        "revision": dir_rev,
        "agents": agents,
        "rooms": rooms,
        "entries": rooms,
        "experts": experts,
        "role_templates": built_in_role_templates()
    }))
}

fn canonical_role_template_catalog() -> Value {
    let templates = vec![
        json!({
            "id": "11111111-1111-4111-8111-111111111101",
            "role_id": "code_writer",
            "name": {"en": "Code writer", "zh-Hans": "代码编写", "zh": "代码编写"},
            "description": {"en": "Implements focused, well-tested code changes.", "zh-Hans": "负责精准的代码实现与单元测试。", "zh": "负责精准的代码实现与单元测试。"},
            "prompt": "You are a senior software engineer. Inspect existing code before editing, make minimal well-tested changes, and verify builds and tests.",
            "tool_pack": {"id": "code-implementation", "tools": ["read_file", "edit_file", "write_file", "grep", "run_shell"]},
            "skills": [{
                "id": "code-implementation",
                "name": {"en": "Code implementation", "zh-Hans": "代码实现", "zh": "代码实现"},
                "instructions": "Inspect relevant files, keep edits minimal and idiomatic, and verify with focused tests."
            }]
        }),
        json!({
            "id": "11111111-1111-4111-8111-111111111102",
            "role_id": "architect",
            "name": {"en": "System Architect", "zh-Hans": "系统架构师", "zh": "系统架构师"},
            "description": {"en": "Designs clean module boundaries, data flows, and resilient interfaces.", "zh-Hans": "设计清晰的模块边界、数据流与可靠接口。", "zh": "设计清晰的模块边界、数据流与可靠接口。"},
            "prompt": "You are a principal software architect. Focus on clean boundaries, minimal coupling, clear invariants, and concrete trade-off analysis.",
            "tool_pack": {"id": "architecture-review", "tools": ["read_file", "grep", "codebase_search"]},
            "skills": [{
                "id": "architecture-design",
                "name": {"en": "Architecture design", "zh-Hans": "架构设计", "zh": "架构设计"},
                "instructions": "Evaluate boundaries, invariants, failure modes, and interface contracts before proposing changes."
            }]
        }),
        json!({
            "id": "11111111-1111-4111-8111-111111111103",
            "role_id": "reviewer",
            "name": {"en": "Code Reviewer", "zh-Hans": "代码审查员", "zh": "代码审查员"},
            "description": {"en": "Reviews diffs for correctness, security, and maintainability.", "zh-Hans": "审查变更的正确性、安全性与可维护性。", "zh": "审查变更的正确性、安全性与可维护性。"},
            "prompt": "You are a meticulous code reviewer. Identify bugs, edge cases, race conditions, and security issues with concrete evidence.",
            "tool_pack": {"id": "code-review", "tools": ["read_file", "grep", "git_diff"]},
            "skills": [{
                "id": "code-review",
                "name": {"en": "Code review", "zh-Hans": "代码审查", "zh": "代码审查"},
                "instructions": "Prioritize correctness, security boundaries, and regression risks with exact file and line citations."
            }]
        }),
        json!({
            "id": "11111111-1111-4111-8111-111111111104",
            "role_id": "debugger",
            "name": {"en": "Debugger", "zh-Hans": "故障排查", "zh": "故障排查"},
            "description": {"en": "Isolates root causes and verifies minimal fixes.", "zh-Hans": "定位根因并验证最小化修复。", "zh": "定位根因并验证最小化修复。"},
            "prompt": "You are an expert debugger. Reproduce failures, inspect logs and state, isolate root causes, and verify fixes with regression tests.",
            "tool_pack": {"id": "debugging", "tools": ["read_file", "grep", "run_shell"]},
            "skills": [{
                "id": "root-cause-analysis",
                "name": {"en": "Root cause analysis", "zh-Hans": "根因分析", "zh": "根因分析"},
                "instructions": "Reproduce reliably, gather direct evidence, and verify the fix eliminates the root cause."
            }]
        }),
        json!({
            "id": "11111111-1111-4111-8111-111111111105",
            "role_id": "researcher",
            "name": {"en": "Researcher", "zh-Hans": "调研分析", "zh": "调研分析"},
            "description": {"en": "Synthesizes docs, codebase facts, and external references.", "zh-Hans": "综合文档、代码库事实与外部资料。", "zh": "综合文档、代码库事实与外部资料。"},
            "prompt": "You are a thorough technical researcher. Verify claims against primary sources and present structured, cited findings.",
            "tool_pack": {"id": "research", "tools": ["read_file", "grep", "web_search", "web_fetch"]},
            "skills": [{
                "id": "technical-research",
                "name": {"en": "Technical research", "zh-Hans": "技术调研", "zh": "技术调研"},
                "instructions": "Cross-check primary sources and distinguish verified facts from assumptions."
            }]
        }),
        json!({
            "id": "11111111-1111-4111-8111-111111111106",
            "role_id": "writer",
            "name": {"en": "Technical Writer", "zh-Hans": "技术写作", "zh": "技术写作"},
            "description": {"en": "Drafts clear documentation, specs, and release notes.", "zh-Hans": "撰写清晰的文档、规格说明与发布日志。", "zh": "撰写清晰的文档、规格说明与发布日志。"},
            "prompt": "You are a technical writer. Produce concise, accurate, well-structured documentation tailored to the audience.",
            "tool_pack": {"id": "writing", "tools": ["read_file", "write_file", "edit_file"]},
            "skills": [{
                "id": "documentation",
                "name": {"en": "Documentation", "zh-Hans": "文档撰写", "zh": "文档撰写"},
                "instructions": "Keep documentation accurate, concise, and verified against actual behavior."
            }]
        }),
        json!({
            "id": "11111111-1111-4111-8111-111111111107",
            "role_id": "security",
            "name": {"en": "Security Auditor", "zh-Hans": "安全审计", "zh": "安全审计"},
            "description": {"en": "Audits auth, SSRF, secret redaction, and least privilege.", "zh-Hans": "审计鉴权、SSRF、密钥脱敏与最小权限。", "zh": "审计鉴权、SSRF、密钥脱敏与最小权限。"},
            "prompt": "You are an application security auditor. Inspect trust boundaries, input validation, credential handling, and sandboxing.",
            "tool_pack": {"id": "security-audit", "tools": ["read_file", "grep"]},
            "skills": [{
                "id": "security-review",
                "name": {"en": "Security review", "zh-Hans": "安全审查", "zh": "安全审查"},
                "instructions": "Audit trust boundaries, SSRF, command injection, path traversal, and secret handling."
            }]
        }),
    ];
    json!({
        "version": 1,
        "features": {
            "reasoning_effort": true,
            "explicit_generation_reasoning": true,
            "rejoin": true,
            "main_agent_delegation": true,
            "inline_expert_mentions": true,
            "external_expert_executors": true
        },
        "templates": templates,
        "role_templates": built_in_role_templates()
    })
}

fn room_revision(store: &Store, room_id: &str) -> i64 {
    store
        .get_kv(&format!("room_revision:{room_id}"))
        .ok()
        .flatten()
        .and_then(|v| v.as_i64())
        .unwrap_or(1)
        .max(1)
}

fn bump_room_revision(store: &Store, room_id: &str) -> i64 {
    let next = store
        .get_kv(&format!("room_revision:{room_id}"))
        .ok()
        .flatten()
        .and_then(|v| v.as_i64())
        .unwrap_or(0)
        + 1;
    let _ = store.set_kv(&format!("room_revision:{room_id}"), &json!(next));
    next
}

fn load_room_participants(store: &Store, room: &AgentRoom) -> Vec<Value> {
    if let Ok(Some(Value::Array(arr))) = store.get_kv(&format!("room_participants:{}", room.id)) {
        return arr;
    }
    Vec::new()
}

fn canonical_room_snapshot(store: &Store, room: &AgentRoom) -> Value {
    let rev = room_revision(store, &room.id);
    let participants = load_room_participants(store, room);
    json!({
        "revision": rev,
        "room": {
            "id": room.id,
            "title": room.name,
            "name": room.name,
            "session_id": room.session_id,
            "workspace_root": room.workspace_root,
            "participant_expert_ids": room.participant_expert_ids,
            "participants": participants,
            "created_at_ms": room.created_at_ms,
            "updated_at_ms": room.updated_at_ms
        }
    })
}

pub(crate) fn dispatch_agent_room_command(
    store: &Store,
    suite: &ModulesSuite,
    args: &Value,
) -> Result<Value, (&'static str, String)> {
    let (op, payload) = extract_sub_command(args);
    let input = payload.get("input").unwrap_or(payload);
    match op {
        "directory" => canonical_agent_directory_payload(store, suite),
        "list" | "list_rooms" => {
            if payload.get("limit").is_some() || payload.get("offset").is_some() {
                let offset = payload.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                let limit = payload.get("limit").and_then(|v| v.as_u64()).unwrap_or(50) as usize;
                let rooms: Vec<AgentRoom> = suite
                    .list_records("agent_room")
                    .map_err(|e| ("storage_error", e.to_string()))?;
                let snaps: Vec<Value> = rooms
                    .iter()
                    .skip(offset)
                    .take(limit)
                    .map(|r| canonical_room_snapshot(store, r))
                    .collect();
                return Ok(Value::Array(snaps));
            }
            canonical_agent_directory_payload(store, suite)
        }
        "for_conversation" => {
            let conv_id = input
                .get("conversation_id")
                .or_else(|| args.get("conversation_id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'conversation_id'".into()))?;
            let rooms: Vec<AgentRoom> = suite
                .list_records("agent_room")
                .map_err(|e| ("storage_error", e.to_string()))?;
            let mapped_id = store
                .get_kv(&format!("conv_room:{conv_id}"))
                .ok()
                .flatten()
                .and_then(|v| v.as_str().map(ToString::to_string));
            let Some(found) = rooms.into_iter().find(|r| {
                r.session_id == conv_id
                    || r.id == conv_id
                    || mapped_id.as_deref() == Some(r.id.as_str())
            }) else {
                return Ok(Value::Null);
            };
            let snap = canonical_room_snapshot(store, &found);
            let rev = room_revision(store, &found.id);
            Ok(json!({
                "conversation_id": conv_id,
                "room_id": found.id,
                "revision": rev,
                "room": snap
            }))
        }
        "ensure_for_conversation" => {
            let conv_id = input
                .get("conversation_id")
                .or_else(|| args.get("conversation_id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'conversation_id'".into()))?;
            let rooms: Vec<AgentRoom> = suite
                .list_records("agent_room")
                .map_err(|e| ("storage_error", e.to_string()))?;
            let mapped_id = store
                .get_kv(&format!("conv_room:{conv_id}"))
                .ok()
                .flatten()
                .and_then(|v| v.as_str().map(ToString::to_string));
            if let Some(existing) = rooms.into_iter().find(|r| {
                r.session_id == conv_id
                    || r.id == conv_id
                    || mapped_id.as_deref() == Some(r.id.as_str())
            }) {
                let snap = canonical_room_snapshot(store, &existing);
                let rev = room_revision(store, &existing.id);
                return Ok(json!({
                    "ok": true,
                    "created": false,
                    "conversation_id": conv_id,
                    "room_id": existing.id,
                    "revision": rev,
                    "room": snap
                }));
            }
            let ts = input
                .get("at_ms")
                .or_else(|| payload.get("at_ms"))
                .and_then(|v| v.as_i64())
                .unwrap_or_else(now_ms);
            let room_id = new_id();
            let room = AgentRoom {
                id: room_id.clone(),
                name: input
                    .get("name")
                    .or_else(|| input.get("title"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("Conversation Room")
                    .to_string(),
                workspace_root: suite.workspace_root.display().to_string(),
                participant_expert_ids: Vec::new(),
                session_id: conv_id.to_string(),
                created_at_ms: ts,
                updated_at_ms: ts,
            };
            suite
                .upsert_record("agent_room", &room.id, &room)
                .map_err(|e| ("storage_error", e.to_string()))?;
            let rev = bump_room_revision(store, &room.id);
            let _ = store.set_kv(&format!("conv_room:{conv_id}"), &json!(room_id));
            let snap = canonical_room_snapshot(store, &room);
            Ok(json!({
                "ok": true,
                "created": true,
                "conversation_id": conv_id,
                "room_id": room.id,
                "revision": rev,
                "room": snap
            }))
        }
        "child_conversations" => {
            let conv_id = input
                .get("conversation_id")
                .or_else(|| input.get("room_id"))
                .or_else(|| args.get("conversation_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let limit = input
                .get("limit")
                .or_else(|| payload.get("limit"))
                .and_then(|v| v.as_u64())
                .unwrap_or(30) as usize;
            let offset = input
                .get("offset")
                .or_else(|| payload.get("offset"))
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize;
            let rooms: Vec<AgentRoom> = suite
                .list_records("agent_room")
                .map_err(|e| ("storage_error", e.to_string()))?;
            let mapped_id = store
                .get_kv(&format!("conv_room:{conv_id}"))
                .ok()
                .flatten()
                .and_then(|v| v.as_str().map(ToString::to_string));
            let found_room = rooms.into_iter().find(|r| {
                !conv_id.is_empty()
                    && (r.session_id == conv_id
                        || r.id == conv_id
                        || mapped_id.as_deref() == Some(r.id.as_str()))
            });
            let mut all_children: Vec<Value> = Vec::new();
            let (room_id_val, room_rev_val) = if let Some(ref rm) = found_room {
                let mut combined_parts = load_room_participants(store, rm);
                if let Ok(Some(Value::Array(hist))) =
                    store.get_kv(&format!("room_historical_participants:{}", rm.id))
                {
                    combined_parts.extend(hist);
                }
                for p in combined_parts {
                    let cid = p
                        .get("context_conversation_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let active = p.get("active").and_then(|v| v.as_bool()).unwrap_or(false);
                    let sess = get_session_case_insensitive(store, cid).ok().flatten();
                    let lookup_id = sess.as_ref().map(|s| s.id.as_str()).unwrap_or(cid);
                    let msg_count = crate::web_api::reconstruct_v2_messages(store, lookup_id).0.len();
                    let created = sess.as_ref().map(|s| s.created_at_ms).unwrap_or(rm.created_at_ms);
                    let updated = sess.as_ref().map(|s| s.updated_at_ms).unwrap_or(rm.updated_at_ms);
                    let title = p
                        .get("display_name")
                        .and_then(|v| v.as_str())
                        .or_else(|| sess.as_ref().map(|s| s.title.as_str()))
                        .unwrap_or("Expert");
                    all_children.push(json!({
                        "conversation_id": lookup_id,
                        "revision": 1i64,
                        "title": title,
                        "participant_id": p.get("id").cloned().unwrap_or(Value::Null),
                        "agent_id": p.get("agent_id").cloned().unwrap_or(Value::Null),
                        "agent_revision": p.get("agent_revision").cloned().unwrap_or(json!(1)),
                        "display_name": title,
                        "is_current": active,
                        "active": active,
                        "created_at_ms": created,
                        "updated_at_ms": updated,
                        "message_count": msg_count
                    }));
                }
                (json!(rm.id), json!(room_revision(store, &rm.id)))
            } else {
                (Value::Null, Value::Null)
            };
            let total = all_children.len();
            let page: Vec<Value> = all_children.into_iter().skip(offset).take(limit).collect();
            let has_more = offset + page.len() < total;
            let next_offset = if has_more {
                json!((offset + page.len()) as i64)
            } else {
                Value::Null
            };
            Ok(json!({
                "version": 1,
                "conversation_id": conv_id,
                "room_id": room_id_val,
                "room_revision": room_rev_val,
                "children": page,
                "conversations": page,
                "items": page,
                "has_more": has_more,
                "next_offset": next_offset
            }))
        }
        "child_history" => {
            let parent_conv_id = input
                .get("conversation_id")
                .or_else(|| args.get("conversation_id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'conversation_id'".into()))?;
            let child_conv_id = input
                .get("child_conversation_id")
                .or_else(|| args.get("child_conversation_id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'child_conversation_id'".into()))?;
            let rooms: Vec<AgentRoom> = suite
                .list_records("agent_room")
                .map_err(|e| ("storage_error", e.to_string()))?;
            let mapped_id = store
                .get_kv(&format!("conv_room:{parent_conv_id}"))
                .ok()
                .flatten()
                .and_then(|v| v.as_str().map(ToString::to_string));
            let rm = rooms
                .into_iter()
                .find(|r| {
                    r.session_id.eq_ignore_ascii_case(parent_conv_id)
                        || r.id.eq_ignore_ascii_case(parent_conv_id)
                        || mapped_id.as_deref() == Some(r.id.as_str())
                })
                .ok_or(("not_found", format!("room for conversation '{parent_conv_id}' not found")))?;
            let mut all_parts = load_room_participants(store, &rm);
            if let Ok(Some(Value::Array(hist))) =
                store.get_kv(&format!("room_historical_participants:{}", rm.id))
            {
                all_parts.extend(hist);
            }
            let owns_child = all_parts.iter().any(|p| {
                p.get("context_conversation_id")
                    .and_then(|v| v.as_str())
                    .is_some_and(|c| c.eq_ignore_ascii_case(child_conv_id))
            });
            if !owns_child {
                return Err((
                    "not_found",
                    format!("child conversation '{child_conv_id}' does not belong to room '{}'", rm.id),
                ));
            }
            let child_sess = get_session_case_insensitive(store, child_conv_id)
                .map_err(|e| ("storage_error", e.to_string()))?
                .ok_or(("not_found", format!("child conversation '{child_conv_id}' not found")))?;
            let limit = input.get("limit").and_then(|v| v.as_u64()).unwrap_or(50) as usize;
            let before_ord = input.get("before_ordinal").and_then(|v| v.as_i64());
            let window = build_conversation_projection_response(store, &child_sess, 0, Some(limit), before_ord, true);
            Ok(json!({
                "version": 1,
                "conversation_id": parent_conv_id,
                "child_conversation_id": child_sess.id,
                "room_id": rm.id,
                "room_revision": room_revision(store, &rm.id),
                "source_owner": {"kind": "local"},
                "window": window,
                "expert_exchanges": []
            }))
        }
        "peer_exchanges" => {
            let cid = input
                .get("conversation_id")
                .or_else(|| input.get("room_id"))
                .or_else(|| input.get("id"))
                .or_else(|| args.get("conversation_id"))
                .or_else(|| args.get("room_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let exchanges = store
                .get_kv(&format!("room_exchanges:{cid}"))
                .ok()
                .flatten()
                .and_then(|v| v.as_array().cloned())
                .unwrap_or_default();
            Ok(json!({
                "version": 1,
                "conversation_id": cid,
                "exchanges": exchanges,
                "has_more": false,
                "next_offset": Value::Null
            }))
        }
        "configure_agent" => {
            let id = input
                .get("expert_id")
                .or_else(|| input.get("agent_id"))
                .or_else(|| input.get("id"))
                .or_else(|| payload.get("expert_id"))
                .or_else(|| payload.get("agent_id"))
                .or_else(|| payload.get("id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(new_id);
            let prev_val = suite
                .get_record::<Value>("expert_peer", &id)
                .ok()
                .flatten();
            let prev = prev_val
                .as_ref()
                .and_then(|v| serde_json::from_value::<ExpertPeer>(v.clone()).ok());
            let model_override = if let Some(val) = input
                .get("model_override")
                .or_else(|| input.get("modelOverride"))
                .or_else(|| payload.get("model_override"))
                .or_else(|| payload.get("modelOverride"))
            {
                if val.is_null() {
                    None
                } else {
                    val.as_str()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(ToString::to_string)
                }
            } else {
                prev.as_ref().and_then(|e| e.model_override.clone())
            };
            let description = if let Some(val) = input
                .get("description")
                .or_else(|| payload.get("description"))
            {
                if val.is_null() {
                    None
                } else {
                    val.as_str().map(ToString::to_string)
                }
            } else {
                prev_val
                    .as_ref()
                    .and_then(|v| v.get("description").and_then(|d| d.as_str()))
                    .map(ToString::to_string)
            };
            let color = if let Some(val) = input.get("color").or_else(|| payload.get("color")) {
                if val.is_null() {
                    None
                } else {
                    val.as_str()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(ToString::to_string)
                }
            } else {
                prev_val
                    .as_ref()
                    .and_then(|v| v.get("color").and_then(|c| c.as_str()))
                    .map(ToString::to_string)
            };
            let expert = ExpertPeer {
                id: id.clone(),
                name: input
                    .get("name")
                    .or_else(|| payload.get("name"))
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string)
                    .or_else(|| prev.as_ref().map(|e| e.name.clone()))
                    .unwrap_or_else(|| "Expert".into()),
                role: input
                    .get("role")
                    .or_else(|| payload.get("role"))
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string)
                    .or_else(|| prev.as_ref().map(|e| e.role.clone()))
                    .unwrap_or_else(|| "specialist".into()),
                system_prompt: input
                    .get("system_prompt")
                    .or_else(|| payload.get("system_prompt"))
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string)
                    .or_else(|| {
                        prev.as_ref()
                            .map(|e| e.system_prompt.clone())
                            .filter(|s| !s.trim().is_empty())
                    })
                    .or_else(|| {
                        description
                            .as_deref()
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .map(ToString::to_string)
                    })
                    .unwrap_or_default(),
                model_override,
                allowed_tools: input
                    .get("allowed_tools")
                    .or_else(|| payload.get("allowed_tools"))
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(ToString::to_string))
                            .collect()
                    })
                    .or_else(|| prev.map(|e| e.allowed_tools))
                    .unwrap_or_default(),
                updated_at_ms: now_ms(),
            };
            let prev_rev = prev_val
                .as_ref()
                .and_then(|v| v.get("revision").and_then(|r| r.as_i64()))
                .unwrap_or(0);
            let availability = input
                .get("availability")
                .or_else(|| payload.get("availability"))
                .and_then(|v| v.as_str())
                .map(ToString::to_string)
                .or_else(|| {
                    prev_val
                        .as_ref()
                        .and_then(|v| v.get("availability").and_then(|a| a.as_str()))
                        .map(ToString::to_string)
                })
                .unwrap_or_else(|| "active".to_string());
            let defaults = input
                .get("defaults")
                .or_else(|| payload.get("defaults"))
                .filter(|v| v.is_object())
                .cloned()
                .or_else(|| {
                    prev_val
                        .as_ref()
                        .and_then(|v| v.get("defaults").filter(|d| d.is_object()).cloned())
                })
                .unwrap_or_else(|| json!({}));
            let mut expert_val = serde_json::to_value(&expert).unwrap_or_else(|_| json!({}));
            if let Some(obj) = expert_val.as_object_mut() {
                if let Some(desc) = description {
                    obj.insert("description".into(), json!(desc));
                }
                if let Some(col) = color {
                    obj.insert("color".into(), json!(col));
                }
                obj.insert("revision".into(), json!(prev_rev + 1));
                obj.insert("availability".into(), json!(availability));
                obj.insert("defaults".into(), defaults);
            }
            suite
                .upsert_record("expert_peer", &expert.id, &expert_val)
                .map_err(|e| ("storage_error", e.to_string()))?;
            let next_dir_rev = store
                .get_kv("agent_directory_revision")
                .ok()
                .flatten()
                .and_then(|v| v.as_i64())
                .unwrap_or(0)
                + 1;
            let _ = store.set_kv("agent_directory_revision", &json!(next_dir_rev));
            let mut dir = canonical_agent_directory_payload(store, suite)?;
            if let Some(obj) = dir.as_object_mut() {
                obj.insert("ok".into(), json!(true));
                obj.insert("expert".into(), expert_val);
            }
            Ok(dir)
        }
        "join" | "rejoin" | "deactivate" => {
            let room_id = input
                .get("room_id")
                .or_else(|| input.get("id"))
                .or_else(|| payload.get("room_id"))
                .or_else(|| args.get("room_id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'room_id'".into()))?;
            let mut room = suite
                .get_record::<AgentRoom>("agent_room", room_id)
                .map_err(|e| ("storage_error", e.to_string()))?
                .ok_or(("not_found", format!("agent_room '{room_id}' not found")))?;
            let mut participants = load_room_participants(store, &room);
            let ts = input
                .get("at_ms")
                .or_else(|| payload.get("at_ms"))
                .and_then(|v| v.as_i64())
                .unwrap_or_else(now_ms);
            if op == "deactivate" {
                let target_id = input
                    .get("participant_id")
                    .or_else(|| payload.get("participant_id"))
                    .or_else(|| input.get("expert_id"))
                    .or_else(|| input.get("agent_id"))
                    .and_then(|v| v.as_str())
                    .ok_or(("invalid_argument", "missing 'participant_id'".into()))?;
                for p in &mut participants {
                    let pid = p.get("id").and_then(|v| v.as_str());
                    let aid = p.get("agent_id").and_then(|v| v.as_str());
                    if pid == Some(target_id) || aid == Some(target_id) {
                        if let Some(obj) = p.as_object_mut() {
                            obj.insert("active".into(), json!(false));
                        }
                    }
                }
                room.participant_expert_ids.retain(|x| x != target_id);
                room.updated_at_ms = ts;
                suite
                    .upsert_record("agent_room", &room.id, &room)
                    .map_err(|e| ("storage_error", e.to_string()))?;
                let _ = store.set_kv(
                    &format!("room_participants:{}", room.id),
                    &Value::Array(participants),
                );
                let _ = bump_room_revision(store, &room.id);
                return Ok(canonical_room_snapshot(store, &room));
            }
            let expert_id = input
                .get("agent_id")
                .or_else(|| input.get("expert_id"))
                .or_else(|| payload.get("agent_id"))
                .or_else(|| payload.get("expert_id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'agent_id'".into()))?;
            let expert_rec = suite
                .get_record::<Value>("expert_peer", expert_id)
                .map_err(|e| ("storage_error", e.to_string()))?;
            let (display_name, description, agent_rev) = if let Some(ref ex) = expert_rec {
                let avail = ex
                    .get("availability")
                    .and_then(|v| v.as_str())
                    .unwrap_or("active");
                if avail.eq_ignore_ascii_case("archived") {
                    return Err((
                        "invalid_state",
                        format!("expert '{expert_id}' is archived and cannot join"),
                    ));
                }
                let actual_rev = ex.get("revision").and_then(|v| v.as_i64()).unwrap_or(1);
                if let Some(exp_rev) = input
                    .get("expected_agent_revision")
                    .or_else(|| input.get("agent_revision"))
                    .and_then(|v| v.as_i64())
                {
                    if exp_rev != actual_rev {
                        return Err((
                            "revision_conflict",
                            json!({
                                "scope_id": format!("agent:{expert_id}"),
                                "expected_revision": exp_rev.max(0) as u64,
                                "actual_revision": actual_rev.max(0) as u64
                            })
                            .to_string(),
                        ));
                    }
                }
                (
                    ex.get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Expert")
                        .to_string(),
                    ex.get("description")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    actual_rev,
                )
            } else {
                (
                    input
                        .get("display_name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Expert")
                        .to_string(),
                    input
                        .get("description")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    input
                        .get("agent_revision")
                        .or_else(|| input.get("expected_agent_revision"))
                        .and_then(|v| v.as_i64())
                        .unwrap_or(1),
                )
            };
            if op == "rejoin" {
                let hist_key = format!("room_historical_participants:{}", room.id);
                let mut historical: Vec<Value> = store
                    .get_kv(&hist_key)
                    .ok()
                    .flatten()
                    .and_then(|v| v.as_array().cloned())
                    .unwrap_or_default();
                let mut kept = Vec::new();
                for mut p in participants {
                    if p.get("agent_id").and_then(|v| v.as_str()) == Some(expert_id) {
                        if let Some(obj) = p.as_object_mut() {
                            obj.insert("active".into(), json!(false));
                        }
                        historical.insert(0, p);
                    } else {
                        kept.push(p);
                    }
                }
                participants = kept;
                let _ = store.set_kv(&hist_key, &Value::Array(historical));
            }
            let participant_id = new_id();
            let context_conv_id = new_id();
            let ctx_session = Session {
                id: context_conv_id.clone(),
                title: display_name.clone(),
                workspace_root: room.workspace_root.clone(),
                model_connection: None,
                model_id: None,
                created_at_ms: ts,
                updated_at_ms: ts,
            };
            store
                .upsert_session(&ctx_session)
                .map_err(|e| ("storage_error", e.to_string()))?;
            let owner_settings = json!({
                "agentRoomOwner": {
                    "room_id": room.id,
                    "participant_id": participant_id,
                    "agent_id": expert_id
                }
            });
            let _ = store.set_kv(
                &format!("conv_settings:{context_conv_id}"),
                &json!(owner_settings.to_string()),
            );
            participants.insert(
                0,
                json!({
                    "id": participant_id,
                    "agent_id": expert_id,
                    "agent_revision": agent_rev,
                    "display_name": display_name,
                    "description": description,
                    "context_conversation_id": context_conv_id,
                    "active": true
                }),
            );
            if !room.participant_expert_ids.iter().any(|x| x == expert_id) {
                room.participant_expert_ids.push(expert_id.to_string());
            }
            room.updated_at_ms = ts;
            suite
                .upsert_record("agent_room", &room.id, &room)
                .map_err(|e| ("storage_error", e.to_string()))?;
            let _ = store.set_kv(
                &format!("room_participants:{}", room.id),
                &Value::Array(participants),
            );
            let _ = bump_room_revision(store, &room.id);
            Ok(canonical_room_snapshot(store, &room))
        }
        "read" => {
            let room_id = input
                .get("room_id")
                .or_else(|| input.get("id"))
                .or_else(|| payload.get("room_id"))
                .or_else(|| args.get("room_id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'room_id'".into()))?;
            let Some(room) = suite
                .get_record::<AgentRoom>("agent_room", room_id)
                .map_err(|e| ("storage_error", e.to_string()))?
            else {
                return Ok(Value::Null);
            };
            Ok(canonical_room_snapshot(store, &room))
        }
        "list_role_templates" | "role_templates" => Ok(canonical_role_template_catalog()),
        "list_experts" => {
            let experts: Vec<Value> = suite
                .list_records("expert_peer")
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({
                "experts": experts,
                "role_templates": built_in_role_templates()
            }))
        }
        "get" | "get_room" => {
            let id = payload
                .get("id")
                .or_else(|| payload.get("room_id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            let room = suite
                .get_record::<AgentRoom>("agent_room", id)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(serde_json::to_value(room).unwrap_or(Value::Null))
        }
        "create" | "create_room" | "upsert_room" => {
            let ts = now_ms();
            let id = input
                .get("id")
                .or_else(|| payload.get("id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(new_id);
            let room = AgentRoom {
                id: id.clone(),
                name: input
                    .get("name")
                    .or_else(|| input.get("title"))
                    .or_else(|| payload.get("name"))
                    .or_else(|| payload.get("title"))
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or("Agent Room")
                    .to_string(),
                workspace_root: input
                    .get("workspace_root")
                    .or_else(|| payload.get("workspace_root"))
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string)
                    .unwrap_or_else(|| suite.workspace_root.display().to_string()),
                participant_expert_ids: input
                    .get("participant_expert_ids")
                    .or_else(|| input.get("participantExpertIds"))
                    .or_else(|| payload.get("participant_expert_ids"))
                    .or_else(|| payload.get("participantExpertIds"))
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(ToString::to_string))
                            .collect()
                    })
                    .unwrap_or_default(),
                session_id: input
                    .get("session_id")
                    .or_else(|| payload.get("session_id"))
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string)
                    .unwrap_or_else(|| format!("room_{id}")),
                created_at_ms: ts,
                updated_at_ms: ts,
            };
            suite
                .upsert_record("agent_room", &room.id, &room)
                .map_err(|e| ("storage_error", e.to_string()))?;
            let _ = bump_room_revision(store, &room.id);
            Ok(canonical_room_snapshot(store, &room))
        }
        "update_room" => {
            let id = input
                .get("id")
                .or_else(|| input.get("room_id"))
                .or_else(|| input.get("roomId"))
                .or_else(|| payload.get("id"))
                .or_else(|| payload.get("room_id"))
                .or_else(|| payload.get("roomId"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .ok_or(("invalid_argument", "missing room id".into()))?;
            let mut room = suite
                .get_record::<AgentRoom>("agent_room", id)
                .map_err(|e| ("storage_error", e.to_string()))?
                .ok_or(("not_found", format!("room '{id}' not found")))?;
            if let Some(name) = input
                .get("name")
                .or_else(|| input.get("title"))
                .or_else(|| payload.get("name"))
                .or_else(|| payload.get("title"))
                .and_then(|v| v.as_str())
            {
                room.name = name.to_string();
            }
            if let Some(ws) = input
                .get("workspace_root")
                .or_else(|| payload.get("workspace_root"))
                .and_then(|v| v.as_str())
            {
                room.workspace_root = ws.to_string();
            }
            if let Some(arr) = input
                .get("participant_expert_ids")
                .or_else(|| input.get("participantExpertIds"))
                .or_else(|| payload.get("participant_expert_ids"))
                .or_else(|| payload.get("participantExpertIds"))
                .and_then(|v| v.as_array())
            {
                room.participant_expert_ids = arr
                    .iter()
                    .filter_map(|x| x.as_str().map(ToString::to_string))
                    .collect();
            }
            let detach = input
                .get("detach")
                .or_else(|| payload.get("detach"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if detach {
                room.session_id.clear();
            } else if let Some(sess_val) = input
                .get("session_id")
                .or_else(|| input.get("conversation_id"))
                .or_else(|| input.get("conversationId"))
                .or_else(|| payload.get("session_id"))
                .or_else(|| payload.get("conversation_id"))
                .or_else(|| payload.get("conversationId"))
            {
                if sess_val.is_null() {
                    room.session_id.clear();
                } else if let Some(s) = sess_val.as_str() {
                    room.session_id = s.trim().to_string();
                }
            }
            room.updated_at_ms = now_ms();
            suite
                .upsert_record("agent_room", id, &room)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"ok": true, "room": room}))
        }
        "submit" => {
            let room_id = input
                .get("room_id")
                .or_else(|| input.get("roomId"))
                .or_else(|| args.get("room_id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .ok_or(("invalid_argument", "missing 'room_id'".into()))?;
            let sub_id = input
                .get("submission_id")
                .or_else(|| input.get("id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(new_id);
            let ts = input
                .get("at_ms")
                .and_then(|v| v.as_i64())
                .unwrap_or_else(now_ms);
            let text = input
                .get("text")
                .or_else(|| input.get("content"))
                .or_else(|| input.get("message"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let req_part_id = input
                .get("participant_id")
                .or_else(|| input.get("recipient_id"))
                .or_else(|| input.get("expert_id"))
                .and_then(|v| v.as_str());
            let room_opt = suite.get_record::<AgentRoom>("agent_room", room_id).ok().flatten();
            let room_rev = room_opt
                .as_ref()
                .map(|r| room_revision(store, &r.id))
                .unwrap_or(1);
            let mut matched_pid: Option<String> = None;
            let mut matched_ctx: Option<String> = None;
            if let (Some(rm), Some(pid)) = (&room_opt, req_part_id) {
                for p in load_room_participants(store, rm) {
                    if p.get("active").and_then(|v| v.as_bool()).unwrap_or(true)
                        && (p.get("id").and_then(|v| v.as_str()) == Some(pid)
                            || p.get("agent_id").and_then(|v| v.as_str()) == Some(pid))
                    {
                        matched_pid = p.get("id").and_then(|v| v.as_str()).map(ToString::to_string);
                        matched_ctx = p
                            .get("context_conversation_id")
                            .and_then(|v| v.as_str())
                            .map(ToString::to_string);
                        break;
                    }
                }
            }
            let state = if matched_pid.is_some() { "selected" } else { "accepted" };
            let rec = json!({
                "revision": 1i64,
                "input": {
                    "room_id": room_id,
                    "submission_id": sub_id,
                    "expected_room_revision": input.get("expected_room_revision").and_then(|v| v.as_i64()).unwrap_or(room_rev),
                    "text": text,
                    "participant_id": req_part_id,
                    "at_ms": ts,
                    "parent_message_id": input.get("parent_message_id").cloned().unwrap_or(Value::Null),
                    "conversation_id": input.get("conversation_id").cloned().unwrap_or(Value::Null)
                },
                "state": state,
                "selected_participant_id": matched_pid,
                "failure_code": Value::Null,
                "execution_id": Value::Null,
                "terminal": Value::Null,
                "context_conversation_id": matched_ctx,
                "id": sub_id,
                "submission_id": sub_id,
                "room_id": room_id
            });
            suite
                .upsert_record("agent_room_submission", &sub_id, &rec)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(rec)
        }
        "submission" => {
            let sub_id = input
                .get("submission_id")
                .or_else(|| input.get("id"))
                .or_else(|| args.get("submission_id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'submission_id'".into()))?;
            let found = suite
                .get_record::<Value>("agent_room_submission", sub_id)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(found.unwrap_or(Value::Null))
        }
        "submissions" => {
            let room_id = input
                .get("room_id")
                .or_else(|| input.get("roomId"))
                .or_else(|| args.get("room_id"))
                .and_then(|v| v.as_str());
            let offset = input.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            let limit = input.get("limit").and_then(|v| v.as_u64()).unwrap_or(100) as usize;
            let list: Vec<Value> = suite
                .list_records::<Value>("agent_room_submission")
                .map_err(|e| ("storage_error", e.to_string()))?
                .into_iter()
                .filter(|s| {
                    room_id.is_none_or(|rid| {
                        s.get("room_id")
                            .or_else(|| s.pointer("/input/room_id"))
                            .and_then(|v| v.as_str())
                            == Some(rid)
                    })
                })
                .skip(offset)
                .take(limit)
                .collect();
            Ok(Value::Array(list))
        }
        "select_recipient" => {
            let room_id = input
                .get("room_id")
                .or_else(|| input.get("roomId"))
                .or_else(|| args.get("room_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let sub_id = input
                .get("submission_id")
                .and_then(|v| v.as_str());
            let explicit_recipient = input
                .get("recipient_id")
                .or_else(|| input.get("expert_id"))
                .or_else(|| input.get("agent_id"))
                .or_else(|| input.get("to"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string);
            let recipient = explicit_recipient.unwrap_or_else(|| {
                suite
                    .get_record::<AgentRoom>("agent_room", room_id)
                    .ok()
                    .flatten()
                    .and_then(|r| r.participant_expert_ids.into_iter().next())
                    .unwrap_or_else(|| "assistant".into())
            });
            let ts = now_ms();
            if !room_id.is_empty() {
                let _ = store.set_kv(
                    &format!("room_selected_recipient:{room_id}"),
                    &json!({"room_id": room_id, "recipient_id": recipient, "updated_at_ms": ts}),
                );
            }
            if let Some(sid) = sub_id {
                if let Ok(Some(mut sub)) = suite.get_record::<Value>("agent_room_submission", sid) {
                    if let Some(obj) = sub.as_object_mut() {
                        obj.insert("recipient_id".into(), json!(recipient));
                        obj.insert("updated_at_ms".into(), json!(ts));
                    }
                    let _ = suite.upsert_record("agent_room_submission", sid, &sub);
                }
            }
            Ok(json!({
                "ok": true,
                "room_id": room_id,
                "submission_id": sub_id,
                "recipient_id": recipient
            }))
        }
        "prepare_execution" => {
            let room_id = input
                .get("room_id")
                .or_else(|| input.get("roomId"))
                .or_else(|| args.get("room_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let sub_id = input
                .get("submission_id")
                .or_else(|| input.get("id"))
                .and_then(|v| v.as_str())
                .map(ToString::to_string);
            let exec_id = new_id();
            let ts = now_ms();
            let intent = json!({
                "execution_id": exec_id,
                "room_id": room_id,
                "submission_id": sub_id,
                "recipient_id": input.get("recipient_id").or_else(|| input.get("expert_id")).cloned().unwrap_or(Value::Null),
                "phase": "prepared",
                "prepared_at_ms": ts
            });
            if !room_id.is_empty() {
                let _ = store.set_kv(&format!("room_exec_intent:{room_id}"), &intent);
            }
            if let Some(ref sid) = sub_id {
                if let Ok(Some(mut sub)) = suite.get_record::<Value>("agent_room_submission", sid) {
                    if let Some(obj) = sub.as_object_mut() {
                        obj.insert("status".into(), json!("prepared"));
                        obj.insert("execution_id".into(), json!(exec_id));
                        obj.insert("updated_at_ms".into(), json!(ts));
                    }
                    let _ = suite.upsert_record("agent_room_submission", sid, &sub);
                }
            }
            Ok(json!({
                "ok": true,
                "ready": true,
                "execution_id": exec_id,
                "intent": intent
            }))
        }
        "claim_execution" => {
            let room_id = input
                .get("room_id")
                .or_else(|| input.get("roomId"))
                .or_else(|| args.get("room_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let sub_id = input
                .get("submission_id")
                .or_else(|| input.get("id"))
                .and_then(|v| v.as_str())
                .map(ToString::to_string);
            let exec_id = input
                .get("execution_id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(new_id);
            let ts = now_ms();
            let mut claimed_sub = Value::Null;
            if let Some(ref sid) = sub_id {
                if let Ok(Some(mut sub)) = suite.get_record::<Value>("agent_room_submission", sid) {
                    if let Some(obj) = sub.as_object_mut() {
                        obj.insert("status".into(), json!("claimed"));
                        obj.insert("execution_id".into(), json!(exec_id));
                        obj.insert("claimed_at_ms".into(), json!(ts));
                        obj.insert("updated_at_ms".into(), json!(ts));
                    }
                    let _ = suite.upsert_record("agent_room_submission", sid, &sub);
                    claimed_sub = sub;
                }
            }
            let intent = json!({
                "execution_id": exec_id,
                "room_id": room_id,
                "submission_id": sub_id,
                "phase": "claimed",
                "claimed_at_ms": ts
            });
            if !room_id.is_empty() {
                let _ = store.set_kv(&format!("room_exec_intent:{room_id}"), &intent);
            }
            Ok(json!({
                "ok": true,
                "claimed": true,
                "execution_id": exec_id,
                "intent": intent,
                "submission": claimed_sub
            }))
        }
        "execution_intent" => {
            let room_id = input
                .get("room_id")
                .or_else(|| input.get("roomId"))
                .or_else(|| args.get("room_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let found = if !room_id.is_empty() {
                store
                    .get_kv(&format!("room_exec_intent:{room_id}"))
                    .ok()
                    .flatten()
                    .filter(|v| !v.is_null())
            } else {
                None
            };
            Ok(json!({
                "room_id": room_id,
                "found": found.is_some(),
                "intent": found.unwrap_or(Value::Null)
            }))
        }
        "cancel_pending" => {
            let room_id = input
                .get("room_id")
                .or_else(|| input.get("roomId"))
                .or_else(|| args.get("room_id"))
                .and_then(|v| v.as_str());
            let target_sub = input.get("submission_id").and_then(|v| v.as_str());
            let ts = now_ms();
            let mut cancelled_count = 0usize;
            for mut sub in suite
                .list_records::<Value>("agent_room_submission")
                .unwrap_or_default()
            {
                let matches_room = room_id.is_none_or(|rid| {
                    sub.get("room_id").and_then(|v| v.as_str()) == Some(rid)
                });
                let matches_sub = target_sub.is_none_or(|sid| {
                    sub.get("submission_id")
                        .or_else(|| sub.get("id"))
                        .and_then(|v| v.as_str())
                        == Some(sid)
                });
                let is_pending = matches!(
                    sub.get("status")
                        .or_else(|| sub.get("state"))
                        .and_then(|v| v.as_str()),
                    Some("pending" | "prepared" | "claimed" | "accepted" | "selected")
                );
                if matches_room && matches_sub && is_pending {
                    if let Some(sid) = sub
                        .get("submission_id")
                        .or_else(|| sub.get("id"))
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string)
                    {
                        if let Some(obj) = sub.as_object_mut() {
                            obj.insert("status".into(), json!("cancelled"));
                            obj.insert("state".into(), json!("cancelled"));
                            obj.insert("cancelled_at_ms".into(), json!(ts));
                            obj.insert("updated_at_ms".into(), json!(ts));
                        }
                        let _ = suite.upsert_record("agent_room_submission", &sid, &sub);
                        cancelled_count += 1;
                    }
                }
            }
            if let Some(rid) = room_id {
                let _ = store.set_kv(&format!("room_exec_intent:{rid}"), &Value::Null);
            }
            Ok(json!({
                "ok": true,
                "room_id": room_id,
                "cancelled_count": cancelled_count
            }))
        }
        "reconcile_execution" => {
            let sub_id = input
                .get("submission_id")
                .or_else(|| input.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'submission_id'".into()))?;
            let status = input
                .get("status")
                .or_else(|| input.get("outcome"))
                .and_then(|v| v.as_str())
                .unwrap_or("completed");
            let ts = now_ms();
            let mut sub = suite
                .get_record::<Value>("agent_room_submission", sub_id)
                .map_err(|e| ("storage_error", e.to_string()))?
                .unwrap_or_else(|| json!({"id": sub_id, "submission_id": sub_id}));
            let room_id = sub
                .get("room_id")
                .and_then(|v| v.as_str())
                .or_else(|| input.get("room_id").and_then(|v| v.as_str()))
                .map(ToString::to_string);
            if let Some(obj) = sub.as_object_mut() {
                obj.insert("status".into(), json!(status));
                if let Some(out) = input.get("output").or_else(|| input.get("result")) {
                    obj.insert("output".into(), out.clone());
                }
                if let Some(err) = input.get("error") {
                    obj.insert("error".into(), err.clone());
                }
                obj.insert("reconciled_at_ms".into(), json!(ts));
                obj.insert("updated_at_ms".into(), json!(ts));
            }
            suite
                .upsert_record("agent_room_submission", sub_id, &sub)
                .map_err(|e| ("storage_error", e.to_string()))?;
            if let Some(ref rid) = room_id {
                if matches!(status, "completed" | "failed" | "cancelled") {
                    let _ = store.set_kv(&format!("room_exec_intent:{rid}"), &Value::Null);
                }
            }
            Ok(json!({
                "ok": true,
                "reconciled": true,
                "submission_id": sub_id,
                "submission": sub
            }))
        }
        "fail_submission" => {
            let sub_id = input
                .get("submission_id")
                .or_else(|| input.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'submission_id'".into()))?;
            let error_msg = input
                .get("error")
                .or_else(|| input.get("message"))
                .or_else(|| input.get("reason"))
                .cloned()
                .unwrap_or_else(|| json!("submission failed"));
            let ts = now_ms();
            let mut sub = suite
                .get_record::<Value>("agent_room_submission", sub_id)
                .map_err(|e| ("storage_error", e.to_string()))?
                .unwrap_or_else(|| json!({"id": sub_id, "submission_id": sub_id}));
            let room_id = sub
                .get("room_id")
                .and_then(|v| v.as_str())
                .or_else(|| input.get("room_id").and_then(|v| v.as_str()))
                .map(ToString::to_string);
            if let Some(obj) = sub.as_object_mut() {
                obj.insert("status".into(), json!("failed"));
                obj.insert("error".into(), error_msg);
                obj.insert("failed_at_ms".into(), json!(ts));
                obj.insert("updated_at_ms".into(), json!(ts));
            }
            suite
                .upsert_record("agent_room_submission", sub_id, &sub)
                .map_err(|e| ("storage_error", e.to_string()))?;
            if let Some(ref rid) = room_id {
                let _ = store.set_kv(&format!("room_exec_intent:{rid}"), &Value::Null);
            }
            Ok(json!({
                "ok": true,
                "failed": true,
                "submission_id": sub_id,
                "submission": sub
            }))
        }
        "delete_room" => {
            let id = input
                .get("id")
                .or_else(|| input.get("room_id"))
                .or_else(|| input.get("roomId"))
                .or_else(|| payload.get("id"))
                .or_else(|| payload.get("room_id"))
                .or_else(|| payload.get("roomId"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            let deleted = suite
                .delete_record("agent_room", id)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"ok": true, "deleted": deleted, "id": id}))
        }
        "create_expert" | "upsert_expert" => {
            let id = input
                .get("id")
                .or_else(|| input.get("expert_id"))
                .or_else(|| payload.get("id"))
                .or_else(|| payload.get("expert_id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(new_id);
            let prev_val = suite
                .get_record::<Value>("expert_peer", &id)
                .ok()
                .flatten();
            let prev = prev_val
                .as_ref()
                .and_then(|v| serde_json::from_value::<ExpertPeer>(v.clone()).ok());
            let model_override = if let Some(val) = input
                .get("model_override")
                .or_else(|| input.get("modelOverride"))
                .or_else(|| payload.get("model_override"))
                .or_else(|| payload.get("modelOverride"))
            {
                if val.is_null() {
                    None
                } else {
                    val.as_str()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(ToString::to_string)
                }
            } else {
                prev.as_ref().and_then(|e| e.model_override.clone())
            };
            let description = if let Some(val) = input
                .get("description")
                .or_else(|| payload.get("description"))
            {
                if val.is_null() {
                    None
                } else {
                    val.as_str().map(ToString::to_string)
                }
            } else {
                prev_val
                    .as_ref()
                    .and_then(|v| v.get("description").and_then(|d| d.as_str()))
                    .map(ToString::to_string)
            };
            let expert = ExpertPeer {
                id: id.clone(),
                name: input
                    .get("name")
                    .or_else(|| payload.get("name"))
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string)
                    .or_else(|| prev.as_ref().map(|e| e.name.clone()))
                    .unwrap_or_else(|| "Expert".into()),
                role: input
                    .get("role")
                    .or_else(|| payload.get("role"))
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string)
                    .or_else(|| prev.as_ref().map(|e| e.role.clone()))
                    .unwrap_or_else(|| "specialist".into()),
                system_prompt: input
                    .get("system_prompt")
                    .or_else(|| payload.get("system_prompt"))
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string)
                    .or_else(|| {
                        prev.as_ref()
                            .map(|e| e.system_prompt.clone())
                            .filter(|s| !s.trim().is_empty())
                    })
                    .or_else(|| {
                        description
                            .as_deref()
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .map(ToString::to_string)
                    })
                    .unwrap_or_default(),
                model_override,
                allowed_tools: input
                    .get("allowed_tools")
                    .or_else(|| payload.get("allowed_tools"))
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(ToString::to_string))
                            .collect()
                    })
                    .or_else(|| prev.map(|e| e.allowed_tools))
                    .unwrap_or_default(),
                updated_at_ms: now_ms(),
            };
            let mut expert_val = serde_json::to_value(&expert).unwrap_or_else(|_| json!({}));
            if let Some(obj) = expert_val.as_object_mut() {
                if let Some(desc) = description {
                    obj.insert("description".into(), json!(desc));
                }
            }
            suite
                .upsert_record("expert_peer", &expert.id, &expert_val)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"ok": true, "expert": expert_val}))
        }
        "delete_expert" | "delete_agent" => {
            let id = input
                .get("id")
                .or_else(|| input.get("expert_id"))
                .or_else(|| input.get("agent_id"))
                .or_else(|| payload.get("id"))
                .or_else(|| payload.get("expert_id"))
                .or_else(|| payload.get("agent_id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            let deleted = suite
                .delete_record("expert_peer", id)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"ok": true, "deleted": deleted, "id": id}))
        }
        "parent" => {
            let conv_id = input
                .get("conversation_id")
                .or_else(|| payload.get("conversation_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let room = suite
                .list_records::<Value>("agent_room")
                .unwrap_or_default()
                .into_iter()
                .find(|r| {
                    !conv_id.is_empty()
                        && (r.get("conversation_id").and_then(|v| v.as_str()) == Some(conv_id)
                            || r.get("id").and_then(|v| v.as_str()) == Some(conv_id))
                })
                .unwrap_or_else(|| {
                    json!({
                        "id": if conv_id.is_empty() { new_id() } else { conv_id.to_string() },
                        "conversation_id": conv_id,
                        "participants": [],
                        "revision": 1
                    })
                });
            Ok(json!({
                "ok": true,
                "operation": "parent",
                "conversation_id": conv_id,
                "room": room
            }))
        }
        "admit" => {
            let conv_id = input
                .get("conversation_id")
                .or_else(|| payload.get("conversation_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let msg_id = input
                .get("message_id")
                .or_else(|| input.get("user_message_id"))
                .or_else(|| payload.get("message_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let room = suite
                .list_records::<Value>("agent_room")
                .unwrap_or_default()
                .into_iter()
                .find(|r| {
                    !conv_id.is_empty()
                        && (r.get("conversation_id").and_then(|v| v.as_str()) == Some(conv_id)
                            || r.get("id").and_then(|v| v.as_str()) == Some(conv_id))
                })
                .unwrap_or_else(|| {
                    json!({
                        "id": if conv_id.is_empty() { new_id() } else { conv_id.to_string() },
                        "conversation_id": conv_id,
                        "participants": [],
                        "revision": 1
                    })
                });
            let entries = input
                .get("selected_participants")
                .or_else(|| input.get("participants"))
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            let manifest = json!({
                "version": 1,
                "conversation_id": conv_id,
                "message_id": msg_id,
                "input": input,
                "entries": entries
            });
            if !conv_id.is_empty() && !msg_id.is_empty() {
                let _ = store.set_kv(&format!("expert_manifest:{conv_id}:{msg_id}"), &manifest);
            }
            Ok(json!({
                "ok": true,
                "operation": "admit",
                "conversation_id": conv_id,
                "message_id": msg_id,
                "room": room,
                "manifest": manifest
            }))
        }
        "manifest" | "load_manifest" => {
            let conv_id = input
                .get("conversation_id")
                .or_else(|| payload.get("conversation_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let msg_id = input
                .get("message_id")
                .or_else(|| payload.get("message_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let manifest = if !conv_id.is_empty() && !msg_id.is_empty() {
                store
                    .get_kv(&format!("expert_manifest:{conv_id}:{msg_id}"))
                    .ok()
                    .flatten()
                    .unwrap_or(Value::Null)
            } else {
                Value::Null
            };
            Ok(json!({
                "ok": true,
                "operation": "manifest",
                "conversation_id": conv_id,
                "message_id": msg_id,
                "manifest": manifest
            }))
        }
        other => Err((
            "unsupported_command",
            format!("unsupported agent_room operation '{other}'"),
        )),
    }
}

fn sanitize_template_record(raw: &Value) -> Value {
    let mut out = serde_json::Map::new();
    for key in [
        "id",
        "name",
        "role",
        "description",
        "system_prompt",
        "suggested_tools",
        "revision",
        "builtin",
        "updated_at_ms",
    ] {
        if let Some(v) = raw.get(key) {
            out.insert(key.to_string(), v.clone());
        }
    }
    if !out.contains_key("revision") {
        out.insert("revision".into(), json!(1));
    }
    Value::Object(out)
}

pub(super) fn dispatch_expert_template_command(
    suite: &ModulesSuite,
    args: &Value,
) -> Result<Value, (&'static str, String)> {
    let op_obj = args
        .get("operation")
        .filter(|v| v.is_object())
        .or_else(|| args.get("command").filter(|v| v.is_object()));
    let (op, payload) = if let Some(obj) = op_obj {
        let k = obj
            .get("kind")
            .or_else(|| obj.get("type"))
            .or_else(|| obj.get("operation"))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        (k, obj)
    } else {
        extract_sub_command(args)
    };
    let mut builtins: Vec<Value> = built_in_role_templates()
        .into_iter()
        .map(|mut t| {
            if let Some(obj) = t.as_object_mut() {
                obj.insert("revision".into(), json!(1));
                obj.insert("builtin".into(), json!(true));
            }
            sanitize_template_record(&t)
        })
        .collect();

    match op {
        "load_manifest" => {
            let conv_id = payload
                .get("conversation_id")
                .or_else(|| args.get("conversation_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let msg_id = payload
                .get("message_id")
                .or_else(|| payload.get("user_message_id"))
                .or_else(|| args.get("message_id"))
                .or_else(|| args.get("user_message_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let found = if !conv_id.is_empty() && !msg_id.is_empty() {
                suite
                    .store
                    .get_kv(&format!("expert_manifest:{conv_id}:{msg_id}"))
                    .ok()
                    .flatten()
            } else {
                None
            };
            Ok(found.unwrap_or(Value::Null))
        }
        "readable_templates" => {
            let custom: Vec<Value> = suite
                .list_records::<Value>("expert_template")
                .unwrap_or_default()
                .into_iter()
                .map(|v| sanitize_template_record(&v))
                .collect();
            builtins.extend(custom);
            let rev = builtins
                .iter()
                .filter_map(|t| t.get("revision").and_then(|v| v.as_i64()))
                .max()
                .unwrap_or(1)
                .max(1);
            Ok(json!({
                "revision": rev,
                "templates": builtins
            }))
        }
        "load_prepared_resend" => {
            let conv_id = payload
                .get("conversation_id")
                .or_else(|| args.get("conversation_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if conv_id.is_empty() {
                return Ok(Value::Null);
            }
            let found = suite
                .store
                .get_kv(&format!("prepared_expert_resend:{conv_id}"))
                .ok()
                .flatten()
                .or_else(|| {
                    suite
                        .store
                        .get_kv(&format!("prepared_edited_resend:{conv_id}"))
                        .ok()
                        .flatten()
                });
            Ok(found.unwrap_or(Value::Null))
        }
        "prepare_edited_resend" => {
            let input = payload.get("input").unwrap_or(payload);
            let source_conv = input
                .get("source_conversation_id")
                .or_else(|| input.get("conversation_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let target_conv = input
                .get("target_conversation_id")
                .or_else(|| input.get("conversation_id"))
                .or_else(|| input.get("source_conversation_id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing target_conversation_id".into()))?;
            let source_msg = input
                .get("source_message_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let op_id = input
                .get("operation_id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(new_id);
            let room_id = suite
                .store
                .get_kv(&format!("conv_room_assoc:{source_conv}"))
                .ok()
                .flatten()
                .and_then(|v| v.get("room_id").and_then(|r| r.as_str()).map(ToString::to_string))
                .unwrap_or_else(|| format!("room-{source_conv}"));
            let prepared_expert = json!({
                "input": input,
                "source_submission_id": op_id,
                "room_id": room_id,
                "parent_message_id": source_msg,
                "entries": []
            });
            suite
                .store
                .set_kv(&format!("prepared_expert_resend:{target_conv}"), &prepared_expert)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(prepared_expert)
        }
        "load_child_instructions" => {
            let conv_id = payload
                .get("conversation_id")
                .or_else(|| args.get("conversation_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let run_id = payload
                .get("expected_run_id")
                .or_else(|| payload.get("run_id"))
                .or_else(|| args.get("expected_run_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if conv_id.is_empty() {
                return Ok(Value::Null);
            }
            let found = if !run_id.is_empty() {
                suite
                    .store
                    .get_kv(&format!("expert_child_instructions:{conv_id}:{run_id}"))
                    .ok()
                    .flatten()
            } else {
                None
            }
            .or_else(|| {
                suite
                    .store
                    .get_kv(&format!("expert_child_instructions:{conv_id}"))
                    .ok()
                    .flatten()
            });
            Ok(found.unwrap_or(Value::Null))
        }
        "derive_peer_authority" => {
            if let Some(auth) = payload.get("authority").filter(|v| v.is_object()) {
                return Ok(auth.clone());
            }
            let request = payload.get("request").unwrap_or(payload);
            let host_id = request
                .get("host_id")
                .and_then(|v| v.as_str())
                .map(|s| json!(s))
                .unwrap_or(Value::Null);
            Ok(json!({
                "actor": {
                    "kind": "local_user",
                    "id": "local"
                },
                "request_host_id": host_id
            }))
        }
        "peer_directory" => {
            let source_ctx = payload
                .get("source_context_id")
                .or_else(|| args.get("source_context_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let rooms: Vec<Value> = suite
                .list_records::<Value>("agent_room")
                .unwrap_or_default();
            let mut peers = Vec::new();
            for room in rooms {
                if let Some(parts) = room.get("participants").and_then(|v| v.as_array()) {
                    let belongs = source_ctx.is_empty()
                        || parts.iter().any(|p| {
                            p.get("context_conversation_id").and_then(|v| v.as_str())
                                == Some(source_ctx)
                        });
                    if belongs {
                        for p in parts {
                            if p.get("context_conversation_id").and_then(|v| v.as_str())
                                != Some(source_ctx)
                            {
                                peers.push(p.clone());
                            }
                        }
                    }
                }
            }
            Ok(json!({
                "revision": 1,
                "peers": peers,
                "participants": peers
            }))
        }
        "prepare_peer_task" => {
            let exchange_id = payload
                .get("exchange_id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(new_id);
            let ts = now_ms();
            let rec = json!({
                "exchange_id": exchange_id,
                "status": "prepared",
                "request": payload.get("request").cloned().unwrap_or(Value::Null),
                "target_configuration": payload.get("target_configuration").cloned().unwrap_or(Value::Null),
                "target_invocation": payload.get("target_invocation").cloned().unwrap_or(Value::Null),
                "project_root": payload.get("project_root").cloned().unwrap_or(Value::Null),
                "prepared_at_ms": ts
            });
            suite
                .store
                .set_kv(&format!("peer_task:{exchange_id}"), &rec)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({
                "ok": true,
                "exchange_id": exchange_id,
                "status": "prepared",
                "task": rec
            }))
        }
        "abort_peer_task_preparation" => {
            let exchange_id = payload
                .get("exchange_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if !exchange_id.is_empty() {
                let _ = suite.store.set_kv(&format!("peer_task:{exchange_id}"), &Value::Null);
            }
            Ok(json!({
                "ok": true,
                "exchange_id": exchange_id,
                "aborted": true
            }))
        }
        "load_peer_for_effect" => {
            let req = payload.get("request").unwrap_or(payload);
            let exchange_id = payload
                .get("exchange_id")
                .or_else(|| req.pointer("/payload/input/exchange_id"))
                .or_else(|| req.get("effect_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if exchange_id.is_empty() {
                return Ok(Value::Null);
            }
            let found = suite
                .store
                .get_kv(&format!("peer_exchange:{exchange_id}"))
                .ok()
                .flatten()
                .filter(|v| !v.is_null());
            Ok(found.unwrap_or(Value::Null))
        }
        "peer_task_action" => {
            let req = payload.get("request").unwrap_or(payload);
            let input = req.pointer("/payload/input").unwrap_or(req);
            let action = input
                .get("action")
                .and_then(|v| v.as_str())
                .unwrap_or("read");
            let exchange_id = input
                .get("exchange_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            match action {
                "directory" => {
                    let peers: Vec<Value> = suite
                        .list_records::<Value>("agent_room")
                        .unwrap_or_default()
                        .into_iter()
                        .flat_map(|r| {
                            r.get("participants")
                                .and_then(|v| v.as_array())
                                .cloned()
                                .unwrap_or_default()
                        })
                        .collect();
                    Ok(json!({"action": "directory", "peers": peers, "participants": peers}))
                }
                _ => {
                    let found = if !exchange_id.is_empty() {
                        suite
                            .store
                            .get_kv(&format!("peer_exchange:{exchange_id}"))
                            .ok()
                            .flatten()
                            .filter(|v| !v.is_null())
                    } else {
                        None
                    };
                    Ok(json!({
                        "action": action,
                        "exchange_id": exchange_id,
                        "status": found.as_ref().and_then(|v| v.get("status").and_then(|s| s.as_str())).unwrap_or("idle"),
                        "exchange": found.unwrap_or(Value::Null)
                    }))
                }
            }
        }
        "preview_peer" => {
            let input = payload.get("input").unwrap_or(payload);
            let target_pid = input
                .get("target_participant_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let source_ctx = input
                .get("source_context_id")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let rooms: Vec<Value> = suite
                .list_records::<Value>("agent_room")
                .unwrap_or_default();
            let mut matched_part: Option<Value> = None;
            for room in rooms {
                if let Some(parts) = room.get("participants").and_then(|v| v.as_array()) {
                    if let Some(p) = parts
                        .iter()
                        .find(|p| p.get("id").and_then(|v| v.as_str()) == Some(target_pid))
                    {
                        matched_part = Some(p.clone());
                        break;
                    }
                }
            }
            let part = matched_part.unwrap_or_else(|| {
                json!({
                    "id": target_pid,
                    "agent_id": target_pid,
                    "agent_revision": 1,
                    "context_conversation_id": if source_ctx.is_empty() { target_pid } else { source_ctx }
                })
            });
            let pid = part.get("id").and_then(|v| v.as_str()).unwrap_or(target_pid);
            let aid = part.get("agent_id").and_then(|v| v.as_str()).unwrap_or(pid);
            let arev = part.get("agent_revision").and_then(|v| v.as_i64()).unwrap_or(1);
            let ctx_cid = part
                .get("context_conversation_id")
                .and_then(|v| v.as_str())
                .unwrap_or(pid);
            Ok(json!({
                "participants": [{
                    "participant": {
                        "participant_id": pid,
                        "agent_id": aid,
                        "agent_revision": arev,
                        "context_conversation_id": ctx_cid
                    },
                    "context_configuration_revision": 1,
                    "defaults": {},
                    "client_model_presentation": {},
                    "model_declaration": {
                        "profile_id": "default",
                        "profile_revision": 1,
                        "provider": "openai-compatible",
                        "model_name": "default",
                        "runtime": "remote",
                        "parameters": {}
                    },
                    "effective_parameters": {}
                }]
            }))
        }
        "admit_peer" => {
            let input = payload.get("input").unwrap_or(payload);
            let exchange_id = input
                .get("exchange_id")
                .or_else(|| input.get("call_id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(new_id);
            let room_id = input
                .get("room_id")
                .and_then(|v| v.as_str())
                .unwrap_or("default");
            let ts = now_ms();
            let rec = json!({
                "exchange_id": exchange_id,
                "room_id": room_id,
                "status": "admitted",
                "input": input,
                "context": payload.get("context").cloned().unwrap_or(Value::Null),
                "clarifications": [],
                "admitted_at_ms": ts
            });
            suite
                .store
                .set_kv(&format!("peer_exchange:{exchange_id}"), &rec)
                .map_err(|e| ("storage_error", e.to_string()))?;
            suite
                .store
                .set_kv(&format!("peer_exchange:{room_id}:{exchange_id}"), &rec)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(rec)
        }
        "load_peer_for_submission" | "load_peer" => {
            let room_id = payload
                .get("room_id")
                .or_else(|| args.get("room_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let exchange_id = payload
                .get("exchange_id")
                .or_else(|| args.get("exchange_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if exchange_id.is_empty() {
                return Ok(Value::Null);
            }
            let found = if !room_id.is_empty() {
                suite
                    .store
                    .get_kv(&format!("peer_exchange:{room_id}:{exchange_id}"))
                    .ok()
                    .flatten()
                    .filter(|v| !v.is_null())
            } else {
                None
            }
            .or_else(|| {
                suite
                    .store
                    .get_kv(&format!("peer_exchange:{exchange_id}"))
                    .ok()
                    .flatten()
                    .filter(|v| !v.is_null())
            });
            Ok(found.unwrap_or(Value::Null))
        }
        "clarify_peer" => {
            let exchange_id = payload
                .get("exchange_id")
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .ok_or(("invalid_argument", "missing 'exchange_id'".into()))?;
            let text = payload
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let key = format!("peer_exchange:{exchange_id}");
            let mut rec = suite
                .store
                .get_kv(&key)
                .ok()
                .flatten()
                .filter(|v| v.is_object())
                .unwrap_or_else(|| {
                    json!({
                        "exchange_id": exchange_id,
                        "status": "clarified",
                        "clarifications": []
                    })
                });
            let ts = now_ms();
            if let Some(obj) = rec.as_object_mut() {
                obj.insert("status".into(), json!("clarified"));
                obj.insert("updated_at_ms".into(), json!(ts));
                let arr = obj
                    .entry("clarifications".to_string())
                    .or_insert_with(|| json!([]));
                if let Some(list) = arr.as_array_mut() {
                    list.push(json!({
                        "call_id": payload.get("call_id").cloned().unwrap_or(Value::Null),
                        "source_context_id": payload.get("source_context_id").cloned().unwrap_or(Value::Null),
                        "source_run_id": payload.get("source_run_id").cloned().unwrap_or(Value::Null),
                        "text": text,
                        "at_ms": ts
                    }));
                }
            }
            suite
                .store
                .set_kv(&key, &rec)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(rec)
        }
        "preview" | "preview_capture" | "admit" => Err((
            "requires_main_agent_turn",
            "requires_main_agent_turn: direct expert fanout requires a main agent turn".into(),
        )),
        "list" | "list_templates" => {
            let custom: Vec<Value> = suite
                .list_records::<Value>("expert_template")
                .unwrap_or_default()
                .into_iter()
                .map(|v| sanitize_template_record(&v))
                .collect();
            builtins.extend(custom);
            Ok(json!({"templates": builtins, "items": builtins}))
        }
        "get" => {
            let id = payload
                .get("id")
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            if let Ok(Some(custom)) = suite.get_record::<Value>("expert_template", id) {
                return Ok(sanitize_template_record(&custom));
            }
            let found = builtins
                .into_iter()
                .find(|t| t.get("id").and_then(|v| v.as_str()) == Some(id));
            Ok(found.unwrap_or(Value::Null))
        }
        "upsert" | "create" | "update" => {
            let src = payload.get("template").unwrap_or(payload);
            let id = src
                .get("id")
                .or_else(|| payload.get("id"))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(ToString::to_string)
                .unwrap_or_else(new_id);
            let prev_rev = suite
                .get_record::<Value>("expert_template", &id)
                .ok()
                .flatten()
                .and_then(|v| v.get("revision").and_then(|r| r.as_u64()))
                .unwrap_or(0);
            let mut rec = sanitize_template_record(src);
            if let Some(obj) = rec.as_object_mut() {
                obj.insert("id".into(), json!(id));
                if !obj.contains_key("name") {
                    obj.insert("name".into(), json!(id));
                }
                obj.insert("revision".into(), json!(prev_rev + 1));
                obj.insert("builtin".into(), json!(false));
                obj.insert("updated_at_ms".into(), json!(now_ms()));
            }
            suite
                .upsert_record("expert_template", &id, &rec)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"ok": true, "id": id, "template": rec}))
        }
        "delete" => {
            let id = payload
                .get("id")
                .or_else(|| args.get("id"))
                .and_then(|v| v.as_str())
                .ok_or(("invalid_argument", "missing 'id'".into()))?;
            let deleted = suite
                .delete_record("expert_template", id)
                .map_err(|e| ("storage_error", e.to_string()))?;
            Ok(json!({"ok": true, "deleted": deleted, "id": id}))
        }
        other => Err((
            "unsupported_command",
            format!("unsupported expert template operation '{other}'"),
        )),
    }
}


pub(super) fn dispatch_reconcile_room_routing(
    suite: &ModulesSuite,
    args: &Value,
) -> Result<Value, (&'static str, String)> {
    let input = args.get("input").unwrap_or(args);
    let sub_id = input
        .get("submission_id")
        .or_else(|| input.get("id"))
        .and_then(|v| v.as_str())
        .ok_or(("invalid_argument", "missing 'submission_id'".into()))?;
    let Some(mut sub) = suite
        .get_record::<Value>("agent_room_submission", sub_id)
        .map_err(|e| ("storage_error", e.to_string()))?
    else {
        return Ok(Value::Null);
    };
    let ts = input
        .get("at_ms")
        .and_then(|v| v.as_i64())
        .unwrap_or_else(now_ms);
    if let Some(obj) = sub.as_object_mut() {
        if let Some(sh) = input
            .get("source_host_id")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
        {
            obj.insert("source_host_id".into(), json!(sh));
        }
        obj.insert("updated_at_ms".into(), json!(ts));
        obj.insert("reconciled_at_ms".into(), json!(ts));
    }
    suite
        .upsert_record("agent_room_submission", sub_id, &sub)
        .map_err(|e| ("storage_error", e.to_string()))?;
    Ok(sub)
}

