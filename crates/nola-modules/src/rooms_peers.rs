// SPDX-License-Identifier: Apache-2.0
//! AgentRoom, ExpertPeer personas, and Subagent delegation module.

use std::sync::Arc;

use nola_tools::{ToolContext, ToolHandler, ToolRegistry};
use nola_types::{
    new_id, now_ms, AgentRoom, CapabilityDomain, ExpertPeer, ToolExecutionResult, ToolSchema,
    ToolSpec, ToolTier,
};
use serde_json::{json, Value};

use crate::ModulesSuite;

pub const KIND_EXPERT: &str = "expert_peer";
pub const KIND_ROOM: &str = "agent_room";

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
