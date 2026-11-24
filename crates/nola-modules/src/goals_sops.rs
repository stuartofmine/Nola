// SPDX-License-Identifier: Apache-2.0
//! Autonomous Goals and Standard Operating Procedures (SOPs) module.

use std::sync::Arc;

use nola_tools::{ToolContext, ToolHandler, ToolRegistry};
use nola_types::{
    new_id, now_ms, CapabilityDomain, GoalRecord, GoalStatus, SopRecord, ToolExecutionResult,
    ToolSchema, ToolSpec, ToolTier,
};
use serde_json::{json, Value};

use crate::ModulesSuite;

pub const KIND_GOAL: &str = "goal";
pub const KIND_SOP: &str = "sop";

pub fn register_tools(suite: &Arc<ModulesSuite>, registry: &mut ToolRegistry) {
    registry.register(Arc::new(GoalManageTool {
        suite: Arc::clone(suite),
    }));
    registry.register(Arc::new(SopManageTool {
        suite: Arc::clone(suite),
    }));
}

struct GoalManageTool {
    suite: Arc<ModulesSuite>,
}

impl ToolHandler for GoalManageTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "goal_manage".into(),
                description: "Create, list, update status/progress, or delete long-running autonomous Goals.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "action": { "type": "string", "enum": ["create", "list", "update", "delete"] },
                        "id": { "type": "string" },
                        "title": { "type": "string" },
                        "objective": { "type": "string" },
                        "status": { "type": "string", "enum": ["pending", "running", "paused", "completed", "failed"] },
                        "note": { "type": "string" }
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
            "create" => {
                let ts = now_ms();
                let rec = GoalRecord {
                    id: args
                        .get("id")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string)
                        .unwrap_or_else(new_id),
                    title: args
                        .get("title")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Untitled Goal")
                        .to_string(),
                    objective: args
                        .get("objective")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    workspace_root: self.suite.workspace_root.display().to_string(),
                    status: GoalStatus::Pending,
                    progress_notes: Vec::new(),
                    created_at_ms: ts,
                    updated_at_ms: ts,
                };
                match self.suite.upsert_record(KIND_GOAL, &rec.id, &rec) {
                    Ok(()) => ToolExecutionResult::ok_structured(
                        serde_json::to_value(&rec).unwrap_or(json!({})),
                        format!("Created goal '{}'", rec.title),
                    ),
                    Err(e) => ToolExecutionResult::err_structured("goal_error", e.to_string(), None),
                }
            }
            "list" => match self.suite.list_records::<GoalRecord>(KIND_GOAL) {
                Ok(items) => {
                    let count = items.len();
                    ToolExecutionResult::ok_structured(
                        json!({"count": count, "goals": items}),
                        format!("Listed {count} goal(s)"),
                    )
                }
                Err(e) => ToolExecutionResult::err_structured("goal_error", e.to_string(), None),
            },
            "update" => {
                let Some(id) = args.get("id").and_then(|v| v.as_str()) else {
                    return ToolExecutionResult::err_structured("missing_param", "missing 'id'", None);
                };
                let Ok(Some(mut rec)) = self.suite.get_record::<GoalRecord>(KIND_GOAL, id) else {
                    return ToolExecutionResult::err_structured("not_found", format!("goal '{id}' not found"), None);
                };
                if let Some(st) = args.get("status").and_then(|v| v.as_str()) {
                    rec.status = match st {
                        "running" => GoalStatus::Running,
                        "paused" => GoalStatus::Paused,
                        "completed" => GoalStatus::Completed,
                        "failed" => GoalStatus::Failed,
                        _ => GoalStatus::Pending,
                    };
                }
                if let Some(note) = args.get("note").and_then(|v| v.as_str()) {
                    rec.progress_notes.push(note.to_string());
                }
                rec.updated_at_ms = now_ms();
                match self.suite.upsert_record(KIND_GOAL, &rec.id, &rec) {
                    Ok(()) => ToolExecutionResult::ok_structured(
                        serde_json::to_value(&rec).unwrap_or(json!({})),
                        format!("Updated goal '{}'", rec.title),
                    ),
                    Err(e) => ToolExecutionResult::err_structured("goal_error", e.to_string(), None),
                }
            }
            "delete" => {
                let Some(id) = args.get("id").and_then(|v| v.as_str()) else {
                    return ToolExecutionResult::err_structured("missing_param", "missing 'id'", None);
                };
                match self.suite.delete_record(KIND_GOAL, id) {
                    Ok(deleted) => ToolExecutionResult::ok_structured(
                        json!({"id": id, "deleted": deleted}),
                        format!("Deleted goal '{id}'"),
                    ),
                    Err(e) => ToolExecutionResult::err_structured("goal_error", e.to_string(), None),
                }
            }
            other => ToolExecutionResult::err_structured(
                "invalid_action",
                format!("unsupported goal action '{other}'"),
                None,
            ),
        }
    }
}

struct SopManageTool {
    suite: Arc<ModulesSuite>,
}

impl ToolHandler for SopManageTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "sop_manage".into(),
                description: "Create, list, or delete reusable Standard Operating Procedures (SOPs).".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "action": { "type": "string", "enum": ["create", "list", "delete"] },
                        "id": { "type": "string" },
                        "name": { "type": "string" },
                        "description": { "type": "string" },
                        "steps": { "type": "array", "items": { "type": "string" } }
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
            "create" => {
                let rec = SopRecord {
                    id: args
                        .get("id")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string)
                        .unwrap_or_else(new_id),
                    name: args
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Untitled SOP")
                        .to_string(),
                    description: args
                        .get("description")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    steps: args
                        .get("steps")
                        .and_then(|v| v.as_array())
                        .map(|a| {
                            a.iter()
                                .filter_map(|x| x.as_str().map(ToString::to_string))
                                .collect()
                        })
                        .unwrap_or_default(),
                    updated_at_ms: now_ms(),
                };
                match self.suite.upsert_record(KIND_SOP, &rec.id, &rec) {
                    Ok(()) => ToolExecutionResult::ok_structured(
                        serde_json::to_value(&rec).unwrap_or(json!({})),
                        format!("Saved SOP '{}'", rec.name),
                    ),
                    Err(e) => ToolExecutionResult::err_structured("sop_error", e.to_string(), None),
                }
            }
            "list" => match self.suite.list_records::<SopRecord>(KIND_SOP) {
                Ok(items) => {
                    let count = items.len();
                    ToolExecutionResult::ok_structured(
                        json!({"count": count, "sops": items}),
                        format!("Listed {count} SOP(s)"),
                    )
                }
                Err(e) => ToolExecutionResult::err_structured("sop_error", e.to_string(), None),
            },
            "delete" => {
                let Some(id) = args.get("id").and_then(|v| v.as_str()) else {
                    return ToolExecutionResult::err_structured("missing_param", "missing 'id'", None);
                };
                match self.suite.delete_record(KIND_SOP, id) {
                    Ok(deleted) => ToolExecutionResult::ok_structured(
                        json!({"id": id, "deleted": deleted}),
                        format!("Deleted SOP '{id}'"),
                    ),
                    Err(e) => ToolExecutionResult::err_structured("sop_error", e.to_string(), None),
                }
            }
            other => ToolExecutionResult::err_structured(
                "invalid_action",
                format!("unsupported sop action '{other}'"),
                None,
            ),
        }
    }
}
