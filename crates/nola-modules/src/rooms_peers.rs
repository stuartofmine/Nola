// SPDX-License-Identifier: Apache-2.0
//! AgentRoom, ExpertPeer personas, and Subagent delegation module.

use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use nola_engine::{
    AgentEngine, AgentRunConfig, EffectivePolicy, ForceToolSet, HookRunner, ModelCaller,
};
use nola_store::DEFAULT_MAX_EVENTS_PER_SCOPE;
use nola_tools::{path_within_workspace, SandboxMode, ToolContext, ToolHandler, ToolRegistry};
use nola_types::{
    new_id, now_ms, AgentRoom, CapabilityDomain, ExpertPeer, FrozenSystemBlock, HostCapabilities,
    ModelRouteConfig, ToolExecutionResult, ToolSchema, ToolSpec, ToolTier,
};
use serde_json::{json, Value};

use crate::ModulesSuite;

pub const KIND_EXPERT: &str = "expert_peer";
pub const KIND_ROOM: &str = "agent_room";

/// Extract `@ExpertName` mentions from a room message.
pub fn parse_expert_mentions(message: &str) -> Vec<String> {
    let mut mentions = Vec::new();
    for token in message.split_whitespace() {
        if let Some(rest) = token.strip_prefix('@') {
            let clean: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
                .collect();
            if !clean.is_empty() && !mentions.contains(&clean) {
                mentions.push(clean);
            }
        }
    }
    mentions
}

/// Real multi-expert AgentRoom router & turn executor.
pub struct RoomRuntime;

impl RoomRuntime {
    pub fn send_message(
        suite: &Arc<ModulesSuite>,
        caller: Arc<dyn ModelCaller>,
        base_route: ModelRouteConfig,
        policy: EffectivePolicy,
        room_id: &str,
        user_message: &str,
    ) -> Result<Vec<Value>, String> {
        let room = suite
            .get_record::<AgentRoom>(KIND_ROOM, room_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("room '{room_id}' not found"))?;
        let room_ws = Path::new(&room.workspace_root);
        if !path_within_workspace(&suite.workspace_root, room_ws) {
            return Err(format!(
                "room workspace '{}' escapes active workspace root",
                room.workspace_root
            ));
        }

        let all_experts = suite
            .list_records::<ExpertPeer>(KIND_EXPERT)
            .unwrap_or_default();
        let scoped_experts: Vec<ExpertPeer> = if room.participant_expert_ids.is_empty() {
            all_experts
        } else {
            all_experts
                .into_iter()
                .filter(|e| room.participant_expert_ids.contains(&e.id))
                .collect()
        };

        let mentions = parse_expert_mentions(user_message);
        let selected: Vec<ExpertPeer> = if !mentions.is_empty() {
            scoped_experts
                .into_iter()
                .filter(|e| {
                    mentions.iter().any(|m| {
                        e.name.eq_ignore_ascii_case(m)
                            || e.id.eq_ignore_ascii_case(m)
                            || e.name.replace(' ', "_").eq_ignore_ascii_case(m)
                    })
                })
                .collect()
        } else {
            scoped_experts
        };

        if selected.is_empty() {
            return Err(if mentions.is_empty() {
                format!("room '{}' has no participating experts", room.name)
            } else {
                format!(
                    "no expert in room '{}' matched mention(s): {}",
                    room.name,
                    mentions.join(", ")
                )
            });
        }

        let _ = suite.store.append_event(
            &room.session_id,
            "room_user_message",
            json!({"room_id": room.id, "text": user_message}),
            DEFAULT_MAX_EVENTS_PER_SCOPE,
        );

        let hooks = HookRunner::load(
            room_ws,
            suite.user_home.as_deref(),
            policy.workspace_trusted,
        );
        let mut replies = Vec::new();
        for expert in selected {
            let mut route = base_route.clone();
            if let Some(m) = expert.model_override.clone().filter(|s| !s.trim().is_empty()) {
                route.model = m;
            }
            let mut registry = ToolRegistry::with_builtin_tools();
            suite.register_tools(&mut registry);
            if !expert.allowed_tools.is_empty() {
                let mut filtered = ToolRegistry::default();
                for spec in registry.all_specs() {
                    if expert
                        .allowed_tools
                        .iter()
                        .any(|a| a == "*" || a == &spec.schema.name)
                    {
                        if let Some(h) = registry.get(&spec.schema.name) {
                            filtered.register(h);
                        }
                    }
                }
                registry = filtered;
            }
            let engine =
                AgentEngine::new(suite.store.clone(), Arc::new(registry), Arc::clone(&caller));
            let mut ctx = ToolContext::new(room_ws);
            ctx.permission_profile = policy.profile;
            ctx.network_allowed = policy.network_allowed;
            ctx.sandbox_mode = SandboxMode::from_env();

            let cfg = AgentRunConfig {
                session_id: Some(format!("{}:{}", room.session_id, expert.id)),
                resume: true,
                route,
                frozen_system_blocks: vec![FrozenSystemBlock {
                    id: format!("expert_{}", expert.id),
                    text: format!(
                        "You are Expert '{}' (Role: {}) participating in AgentRoom '{}'.\n{}",
                        expert.name, expert.role, room.name, expert.system_prompt
                    ),
                }],
                host_capabilities: HostCapabilities {
                    interactive_user_input: false,
                    network_allowed: policy.network_allowed,
                },
                policy: policy.clone(),
                hooks: hooks.clone(),
                approval: None,
                max_steps: 16,
                force_tool_set: ForceToolSet::Auto,
                stream: false,
                json_schema: None,
                cancel: Arc::new(AtomicBool::new(false)),
                auto_recall_memory: true,
                inbox: None,
            };
            let answer = engine
                .run(&ctx, user_message, cfg, |_| {})
                .map(|r| r.final_text)
                .unwrap_or_else(|e| format!("(expert error: {e})"));
            let entry = json!({
                "room_id": room.id,
                "expert_id": expert.id,
                "expert_name": expert.name,
                "role": expert.role,
                "content": answer
            });
            let _ = suite.store.append_event(
                &room.session_id,
                "room_expert_reply",
                entry.clone(),
                DEFAULT_MAX_EVENTS_PER_SCOPE,
            );
            replies.push(entry);
        }
        Ok(replies)
    }
}

pub fn register_tools(suite: &Arc<ModulesSuite>, registry: &mut ToolRegistry) {
    registry.register(Arc::new(RoomManageTool {
        suite: Arc::clone(suite),
    }));
}

struct RoomManageTool {
    suite: Arc<ModulesSuite>,
}

impl ToolHandler for RoomManageTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "room_manage".into(),
                description: "Create and list Expert Peers and multi-agent Collaboration Rooms.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": ["create_expert", "list_experts", "create_room", "list_rooms", "delete_room"]
                        },
                        "id": { "type": "string" },
                        "name": { "type": "string" },
                        "role": { "type": "string" },
                        "system_prompt": { "type": "string" },
                        "participant_expert_ids": { "type": "array", "items": { "type": "string" } }
                    },
                    "required": ["action"]
                }),
            },
            tier: ToolTier::Deferred,
            domain: CapabilityDomain::System,
            read_only: false,
        }
    }

    fn execute(&self, _ctx: &ToolContext, args: Value) -> ToolExecutionResult {
        match args.get("action").and_then(|v| v.as_str()).unwrap_or("") {
            "create_expert" => {
                let peer = ExpertPeer {
                    id: args
                        .get("id")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string)
                        .unwrap_or_else(new_id),
                    name: args
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Expert")
                        .to_string(),
                    role: args
                        .get("role")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Specialist")
                        .to_string(),
                    system_prompt: args
                        .get("system_prompt")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    model_override: args
                        .get("model_override")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string),
                    allowed_tools: Vec::new(),
                    updated_at_ms: now_ms(),
                };
                match self.suite.upsert_record(KIND_EXPERT, &peer.id, &peer) {
                    Ok(()) => ToolExecutionResult::ok_structured(
                        serde_json::to_value(&peer).unwrap_or(json!({})),
                        format!("Saved expert peer '{}'", peer.name),
                    ),
                    Err(e) => ToolExecutionResult::err_structured("room_error", e.to_string(), None),
                }
            }
            "list_experts" => match self.suite.list_records::<ExpertPeer>(KIND_EXPERT) {
                Ok(items) => {
                    let count = items.len();
                    ToolExecutionResult::ok_structured(
                        json!({"count": count, "experts": items}),
                        format!("Listed {count} expert peer(s)"),
                    )
                }
                Err(e) => ToolExecutionResult::err_structured("room_error", e.to_string(), None),
            },
            "create_room" => {
                let ts = now_ms();
                let room = AgentRoom {
                    id: args
                        .get("id")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string)
                        .unwrap_or_else(new_id),
                    name: args
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Agent Room")
                        .to_string(),
                    workspace_root: self.suite.workspace_root.display().to_string(),
                    participant_expert_ids: args
                        .get("participant_expert_ids")
                        .and_then(|v| v.as_array())
                        .map(|a| {
                            a.iter()
                                .filter_map(|x| x.as_str().map(ToString::to_string))
                                .collect()
                        })
                        .unwrap_or_default(),
                    session_id: new_id(),
                    created_at_ms: ts,
                    updated_at_ms: ts,
                };
                match self.suite.upsert_record(KIND_ROOM, &room.id, &room) {
                    Ok(()) => ToolExecutionResult::ok_structured(
                        serde_json::to_value(&room).unwrap_or(json!({})),
                        format!("Created agent room '{}'", room.name),
                    ),
                    Err(e) => ToolExecutionResult::err_structured("room_error", e.to_string(), None),
                }
            }
            "list_rooms" => match self.suite.list_records::<AgentRoom>(KIND_ROOM) {
                Ok(items) => {
                    let count = items.len();
                    ToolExecutionResult::ok_structured(
                        json!({"count": count, "rooms": items}),
                        format!("Listed {count} agent room(s)"),
                    )
                }
                Err(e) => ToolExecutionResult::err_structured("room_error", e.to_string(), None),
            },
            "delete_room" => {
                let Some(id) = args.get("id").and_then(|v| v.as_str()) else {
                    return ToolExecutionResult::err_structured("missing_param", "missing 'id'", None);
                };
                match self.suite.delete_record(KIND_ROOM, id) {
                    Ok(deleted) => ToolExecutionResult::ok_structured(
                        json!({"id": id, "deleted": deleted}),
                        format!("Deleted room '{id}'"),
                    ),
                    Err(e) => ToolExecutionResult::err_structured("room_error", e.to_string(), None),
                }
            }
            other => ToolExecutionResult::err_structured(
                "invalid_action",
                format!("unsupported room action '{other}'"),
                None,
            ),
        }
    }
}
