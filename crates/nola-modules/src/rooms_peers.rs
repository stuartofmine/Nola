// SPDX-License-Identifier: Apache-2.0
//! AgentRoom, ExpertPeer personas, built-in role templates, and Subagent delegation module.

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

/// Seven built-in expert role templates (`architect`, `reviewer`, `debugger`,
/// `researcher`, `writer`, `planner`, `security`) for AgentRoom / CLI / Web UI.
pub fn built_in_role_templates() -> Vec<Value> {
    vec![
        json!({
            "id": "architect",
            "name": "System Architect",
            "role": "architect",
            "description": "Designs clean module boundaries, data flows, and resilient interfaces.",
            "system_prompt": "You are a principal software architect. Focus on clean boundaries, minimal coupling, clear invariants, and concrete trade-off analysis.",
            "suggested_tools": ["codebase_search", "codebase_read_symbol", "read_file", "grep"]
        }),
        json!({
            "id": "reviewer",
            "name": "Code Reviewer",
            "role": "reviewer",
            "description": "Audits diffs for correctness, regressions, edge cases, and maintainability.",
            "system_prompt": "You are a rigorous code reviewer. Identify bugs, missing edge-case handling, concurrency hazards, and API mismatches with exact file:line citations.",
            "suggested_tools": ["codebase_search", "codebase_text_search", "read_file", "grep"]
        }),
        json!({
            "id": "debugger",
            "name": "Root-Cause Debugger",
            "role": "debugger",
            "description": "Isolates failures, forms falsifiable hypotheses, and verifies minimal fixes.",
            "system_prompt": "You are a root-cause debugger. Reproduce issues deterministically, inspect failing logs/stack traces, verify hypotheses, and recommend the minimal fix plus regression test.",
            "suggested_tools": ["bash", "read_file", "grep", "codebase_search"]
        }),
        json!({
            "id": "researcher",
            "name": "Technical Researcher",
            "role": "researcher",
            "description": "Surveys codebases, documentation, and specs to synthesize grounded answers.",
            "system_prompt": "You are a thorough technical researcher. Gather primary evidence from workspace files and documentation, cite sources clearly, and highlight unknowns honestly.",
            "suggested_tools": ["codebase_search", "codebase_text_search", "read_file", "web_search"]
        }),
        json!({
            "id": "writer",
            "name": "Technical Writer",
            "role": "writer",
            "description": "Drafts concise documentation, changelogs, ADRs, and user guides.",
            "system_prompt": "You are a technical writer. Produce clear, accurate, concise documentation and release notes grounded in actual code behavior.",
            "suggested_tools": ["read_file", "write_file", "edit_file"]
        }),
        json!({
            "id": "planner",
            "name": "Execution Planner",
            "role": "planner",
            "description": "Breaks complex initiatives into ordered, verifiable milestones and checklists.",
            "system_prompt": "You are an engineering planner. Decompose goals into crisp, dependency-ordered steps with explicit acceptance checks and risk mitigations.",
            "suggested_tools": ["codebase_search", "read_file", "goal_manage", "sop_manage"]
        }),
        json!({
            "id": "security",
            "name": "Security Auditor",
            "role": "security",
            "description": "Reviews authentication, SSRF, path traversal, secrets, and sandbox boundaries.",
            "system_prompt": "You are a security auditor. Check trust boundaries, fail-closed auth, path traversal, command injection, SSRF, secret redaction, and least privilege.",
            "suggested_tools": ["codebase_text_search", "codebase_read_symbol", "read_file", "grep"]
        }),
    ]
}

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

fn expert_matches_target(expert: &ExpertPeer, target: &str) -> bool {
    let t = target.trim().trim_start_matches('@');
    if t.is_empty() {
        return false;
    }
    expert.id.eq_ignore_ascii_case(t)
        || expert.name.eq_ignore_ascii_case(t)
        || expert.role.eq_ignore_ascii_case(t)
        || expert.name.replace(' ', "_").eq_ignore_ascii_case(t)
        || expert.name.replace(' ', "-").eq_ignore_ascii_case(t)
}

/// Select experts inside a room based on explicit `target_recipient` (`--to`) or `@mentions`.
pub fn select_room_experts(
    scoped_experts: Vec<ExpertPeer>,
    user_message: &str,
    target_recipient: Option<&str>,
) -> Result<Vec<ExpertPeer>, String> {
    if let Some(target) = target_recipient.map(str::trim).filter(|s| !s.is_empty()) {
        let matched: Vec<ExpertPeer> = scoped_experts
            .into_iter()
            .filter(|e| expert_matches_target(e, target))
            .collect();
        if matched.is_empty() {
            return Err(format!("no expert in room matched recipient '{target}'"));
        }
        return Ok(matched);
    }

    let mentions = parse_expert_mentions(user_message);
    let selected: Vec<ExpertPeer> = if !mentions.is_empty() {
        scoped_experts
            .into_iter()
            .filter(|e| mentions.iter().any(|m| expert_matches_target(e, m)))
            .collect()
    } else {
        scoped_experts
    };

    if selected.is_empty() {
        return Err(if mentions.is_empty() {
            "room has no participating experts".into()
        } else {
            format!(
                "no expert in room matched mention(s): {}",
                mentions.join(", ")
            )
        });
    }
    Ok(selected)
}

/// Real multi-expert AgentRoom router & turn executor.
pub struct RoomRuntime;

impl RoomRuntime {
    /// Backward-compatible room message dispatch (routes via `@mentions` or broadcasts to room experts).
    pub fn send_message(
        suite: &Arc<ModulesSuite>,
        caller: Arc<dyn ModelCaller>,
        base_route: ModelRouteConfig,
        policy: EffectivePolicy,
        room_id: &str,
        user_message: &str,
    ) -> Result<Vec<Value>, String> {
        Self::send_message_to(suite, caller, base_route, policy, room_id, user_message, None)
    }

    /// Room message dispatch supporting explicit `target_recipient: Option<&str>` (`--to`).
    pub fn send_message_to(
        suite: &Arc<ModulesSuite>,
        caller: Arc<dyn ModelCaller>,
        base_route: ModelRouteConfig,
        policy: EffectivePolicy,
        room_id: &str,
        user_message: &str,
        target_recipient: Option<&str>,
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

        let selected = select_room_experts(scoped_experts, user_message, target_recipient)
            .map_err(|e| format!("room '{}': {e}", room.name))?;

        let _ = suite.store.append_event(
            &room.session_id,
            "room_user_message",
            json!({
                "room_id": room.id,
                "text": user_message,
                "target_recipient": target_recipient
            }),
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
                description: "Create and list Expert Peers, role templates, and multi-agent Collaboration Rooms.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": ["create_expert", "list_experts", "list_templates", "create_room", "list_rooms", "delete_room"]
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
            "list_templates" => {
                let templates = built_in_role_templates();
                let count = templates.len();
                ToolExecutionResult::ok_structured(
                    json!({"count": count, "templates": templates}),
                    format!("Listed {count} built-in expert role template(s)"),
                )
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_templates_and_explicit_target_routing_work() {
        let templates = built_in_role_templates();
        assert_eq!(templates.len(), 7);
        let ids: Vec<&str> = templates
            .iter()
            .filter_map(|t| t["id"].as_str())
            .collect();
        for expected in [
            "architect",
            "reviewer",
            "debugger",
            "researcher",
            "writer",
            "planner",
            "security",
        ] {
            assert!(ids.contains(&expected), "missing role template {expected}");
        }

        let experts = vec![
            ExpertPeer {
                id: "exp-1".into(),
                name: "Rust Auditor".into(),
                role: "security".into(),
                system_prompt: String::new(),
                model_override: None,
                allowed_tools: vec![],
                updated_at_ms: 0,
            },
            ExpertPeer {
                id: "exp-2".into(),
                name: "UI Architect".into(),
                role: "architect".into(),
                system_prompt: String::new(),
                model_override: None,
                allowed_tools: vec![],
                updated_at_ms: 0,
            },
        ];

        let targeted =
            select_room_experts(experts.clone(), "Review this patch", Some("security")).unwrap();
        assert_eq!(targeted.len(), 1);
        assert_eq!(targeted[0].id, "exp-1");

        let missing = select_room_experts(experts, "Hello", Some("nonexistent")).unwrap_err();
        assert!(missing.contains("nonexistent"));
    }
}
