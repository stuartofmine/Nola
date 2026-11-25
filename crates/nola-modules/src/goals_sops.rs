// SPDX-License-Identifier: Apache-2.0
//! Autonomous Goals and Standard Operating Procedures (SOPs) module.
//! Includes extended `GoalRecord` durability fields (`lease_until_ms`, `retry_count`,
//! `next_retry_at_ms`, `needs_user_question`, `acceptance_criteria`, `accepted`),
//! `goal_complete` acceptance verification, and `sop_run_update` step progress persistence.

use std::sync::Arc;

use nola_tools::{ToolContext, ToolHandler, ToolRegistry};
use nola_types::{
    new_id, now_ms, CapabilityDomain, GoalStatus, SopRecord, ToolExecutionResult, ToolSchema,
    ToolSpec, ToolTier,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::ModulesSuite;

pub const KIND_GOAL: &str = "goal";
pub const KIND_SOP: &str = "sop";
pub const KIND_SOP_RUN: &str = "sop_run";

/// Extended durable Goal record compatible with `nola_types::GoalRecord` JSON serialization
/// (all newly added durability/acceptance fields carry `#[serde(default)]`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GoalRecord {
    pub id: String,
    pub title: String,
    pub objective: String,
    pub workspace_root: String,
    pub status: GoalStatus,
    #[serde(default)]
    pub progress_notes: Vec<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease_until_ms: Option<i64>,
    #[serde(default)]
    pub retry_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_retry_at_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub needs_user_question: Option<String>,
    #[serde(default)]
    pub acceptance_criteria: Vec<String>,
    #[serde(default)]
    pub accepted: bool,
}

impl GoalRecord {
    pub fn is_needs_user(&self) -> bool {
        self.status == GoalStatus::Paused && self.needs_user_question.is_some()
    }

    pub fn into_base(self) -> nola_types::GoalRecord {
        nola_types::GoalRecord {
            id: self.id,
            title: self.title,
            objective: self.objective,
            workspace_root: self.workspace_root,
            status: self.status,
            progress_notes: self.progress_notes,
            created_at_ms: self.created_at_ms,
            updated_at_ms: self.updated_at_ms,
        }
    }
}

impl From<nola_types::GoalRecord> for GoalRecord {
    fn from(base: nola_types::GoalRecord) -> Self {
        Self {
            id: base.id,
            title: base.title,
            objective: base.objective,
            workspace_root: base.workspace_root,
            status: base.status,
            progress_notes: base.progress_notes,
            created_at_ms: base.created_at_ms,
            updated_at_ms: base.updated_at_ms,
            lease_until_ms: None,
            retry_count: 0,
            next_retry_at_ms: None,
            needs_user_question: None,
            acceptance_criteria: Vec::new(),
            accepted: false,
        }
    }
}

impl From<GoalRecord> for nola_types::GoalRecord {
    fn from(ext: GoalRecord) -> Self {
        ext.into_base()
    }
}

/// Step progress entry within a persisted `SopRunRecord`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SopStepProgress {
    pub index: usize,
    pub title: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub updated_at_ms: i64,
}

/// Durable execution state for an SOP run stored in `module_records("sop_run")`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SopRunRecord {
    pub id: String,
    pub sop_id: String,
    pub sop_name: String,
    pub status: String,
    pub steps: Vec<SopStepProgress>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

pub fn register_tools(suite: &Arc<ModulesSuite>, registry: &mut ToolRegistry) {
    registry.register(Arc::new(GoalManageTool {
        suite: Arc::clone(suite),
    }));
    registry.register(Arc::new(GoalCompleteTool {
        suite: Arc::clone(suite),
    }));
    registry.register(Arc::new(SopManageTool {
        suite: Arc::clone(suite),
    }));
    registry.register(Arc::new(SopRunUpdateTool {
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
                description: "Create, list, update status/progress/needs_user/acceptance_criteria, or delete long-running autonomous Goals.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "action": { "type": "string", "enum": ["create", "list", "update", "resume", "delete"] },
                        "id": { "type": "string" },
                        "title": { "type": "string" },
                        "objective": { "type": "string" },
                        "status": { "type": "string", "enum": ["pending", "running", "paused", "needs_user", "completed", "failed"] },
                        "note": { "type": "string" },
                        "needs_user_question": { "type": "string" },
                        "user_answer": { "type": "string" },
                        "acceptance_criteria": { "type": "array", "items": { "type": "string" } }
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
                let acceptance_criteria = args
                    .get("acceptance_criteria")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|x| x.as_str().map(ToString::to_string))
                            .collect()
                    })
                    .unwrap_or_default();
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
                    lease_until_ms: None,
                    retry_count: 0,
                    next_retry_at_ms: None,
                    needs_user_question: None,
                    acceptance_criteria,
                    accepted: false,
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
                    return ToolExecutionResult::err_structured(
                        "not_found",
                        format!("goal '{id}' not found"),
                        None,
                    );
                };
                if let Some(st) = args.get("status").and_then(|v| v.as_str()) {
                    match st {
                        "running" => {
                            rec.status = GoalStatus::Running;
                            rec.needs_user_question = None;
                        }
                        "needs_user" => {
                            rec.status = GoalStatus::Paused;
                            rec.lease_until_ms = None;
                            let q = args
                                .get("needs_user_question")
                                .or_else(|| args.get("note"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("Additional user input required to proceed.");
                            rec.needs_user_question = Some(q.to_string());
                        }
                        "paused" => {
                            rec.status = GoalStatus::Paused;
                            rec.lease_until_ms = None;
                        }
                        "completed" => {
                            rec.status = GoalStatus::Completed;
                            rec.lease_until_ms = None;
                            rec.needs_user_question = None;
                        }
                        "failed" => {
                            rec.status = GoalStatus::Failed;
                            rec.lease_until_ms = None;
                        }
                        _ => {
                            rec.status = GoalStatus::Pending;
                            rec.lease_until_ms = None;
                        }
                    }
                }
                if let Some(q) = args.get("needs_user_question").and_then(|v| v.as_str()) {
                    if !q.trim().is_empty() {
                        rec.needs_user_question = Some(q.trim().to_string());
                        rec.status = GoalStatus::Paused;
                        rec.lease_until_ms = None;
                    }
                }
                if let Some(arr) = args.get("acceptance_criteria").and_then(|v| v.as_array()) {
                    rec.acceptance_criteria = arr
                        .iter()
                        .filter_map(|x| x.as_str().map(ToString::to_string))
                        .collect();
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
            "resume" => {
                let Some(id) = args.get("id").and_then(|v| v.as_str()) else {
                    return ToolExecutionResult::err_structured("missing_param", "missing 'id'", None);
                };
                let answer = args
                    .get("user_answer")
                    .or_else(|| args.get("note"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                match crate::goal_worker::GoalWorker::resume_from_user_input(
                    &self.suite.store,
                    id,
                    answer,
                ) {
                    Ok(rec) => ToolExecutionResult::ok_structured(
                        serde_json::to_value(&rec).unwrap_or(json!({})),
                        format!("Resumed goal '{}'", rec.title),
                    ),
                    Err(e) => ToolExecutionResult::err_structured("goal_error", e, None),
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

/// Structured acceptance-criteria verification & clean goal completion tool (`goal_complete`).
pub struct GoalCompleteTool {
    suite: Arc<ModulesSuite>,
}

impl GoalCompleteTool {
    pub fn new(suite: Arc<ModulesSuite>) -> Self {
        Self { suite }
    }
}

impl ToolHandler for GoalCompleteTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "goal_complete".into(),
                description: "Verify acceptance criteria and mark an autonomous Goal cleanly completed (or paused if criteria failed).".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "goal_id": { "type": "string", "description": "Target Goal ID" },
                        "summary": { "type": "string", "description": "Final completion summary" },
                        "criteria_results": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "criterion": { "type": "string" },
                                    "passed": { "type": "boolean" },
                                    "evidence": { "type": "string" }
                                },
                                "required": ["criterion", "passed"]
                            }
                        },
                        "accepted": { "type": "boolean", "description": "Explicit acceptance override; defaults to true when all criteria pass" }
                    },
                    "required": ["goal_id", "summary"]
                }),
            },
            tier: ToolTier::Deferred,
            domain: CapabilityDomain::System,
            read_only: false,
        }
    }

    fn execute(&self, _ctx: &ToolContext, args: Value) -> ToolExecutionResult {
        let Some(goal_id) = args
            .get("goal_id")
            .or_else(|| args.get("id"))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            return ToolExecutionResult::err_structured("missing_param", "missing 'goal_id'", None);
        };
        let Some(summary) = args
            .get("summary")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            return ToolExecutionResult::err_structured("missing_param", "missing 'summary'", None);
        };
        let Ok(Some(mut rec)) = self.suite.get_record::<GoalRecord>(KIND_GOAL, goal_id) else {
            return ToolExecutionResult::err_structured(
                "not_found",
                format!("goal '{goal_id}' not found"),
                None,
            );
        };

        let mut passed_count = 0usize;
        let mut failed_count = 0usize;
        let mut recorded_criteria = Vec::new();
        if let Some(arr) = args.get("criteria_results").and_then(|v| v.as_array()) {
            for item in arr {
                let crit = item
                    .get("criterion")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim();
                let passed = item.get("passed").and_then(|v| v.as_bool()).unwrap_or(false);
                let evidence = item.get("evidence").and_then(|v| v.as_str()).unwrap_or("");
                if !crit.is_empty() {
                    recorded_criteria.push(crit.to_string());
                    let badge = if passed { "PASS" } else { "FAIL" };
                    let note = if evidence.is_empty() {
                        format!("[criteria:{badge}] {crit}")
                    } else {
                        format!("[criteria:{badge}] {crit} — {evidence}")
                    };
                    rec.progress_notes.push(note);
                }
                if passed {
                    passed_count += 1;
                } else {
                    failed_count += 1;
                }
            }
        }
        if rec.acceptance_criteria.is_empty() && !recorded_criteria.is_empty() {
            rec.acceptance_criteria = recorded_criteria;
        }
        let accepted = args
            .get("accepted")
            .and_then(|v| v.as_bool())
            .unwrap_or(failed_count == 0);
        rec.accepted = accepted;
        rec.status = if accepted {
            GoalStatus::Completed
        } else {
            GoalStatus::Paused
        };
        rec.lease_until_ms = None;
        rec.next_retry_at_ms = None;
        rec.needs_user_question = None;
        rec.progress_notes.push(format!("[complete] {summary}"));
        rec.updated_at_ms = now_ms();

        match self.suite.upsert_record(KIND_GOAL, &rec.id, &rec) {
            Ok(()) => ToolExecutionResult::ok_structured(
                json!({
                    "goal": rec,
                    "accepted": accepted,
                    "criteria_passed": passed_count,
                    "criteria_failed": failed_count
                }),
                format!(
                    "Goal '{}' marked {} (passed={passed_count}, failed={failed_count})",
                    goal_id,
                    if accepted { "completed" } else { "paused" }
                ),
            ),
            Err(e) => ToolExecutionResult::err_structured("goal_error", e.to_string(), None),
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

/// Persist and update step-by-step progress of an SOP execution run (`sop_run_update`).
pub struct SopRunUpdateTool {
    suite: Arc<ModulesSuite>,
}

impl SopRunUpdateTool {
    pub fn new(suite: Arc<ModulesSuite>) -> Self {
        Self { suite }
    }
}

fn derive_sop_run_status(steps: &[SopStepProgress]) -> String {
    if steps.is_empty() {
        return "pending".into();
    }
    if steps.iter().any(|s| s.status == "failed") {
        return "failed".into();
    }
    if steps
        .iter()
        .all(|s| s.status == "completed" || s.status == "skipped")
    {
        return "completed".into();
    }
    if steps
        .iter()
        .any(|s| s.status == "running" || s.status == "completed" || s.status == "skipped")
    {
        return "running".into();
    }
    "pending".into()
}

impl ToolHandler for SopRunUpdateTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "sop_run_update".into(),
                description: "Create or update step progress (pending|running|completed|skipped|failed) for an SOP run in SQLite.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "run_id": { "type": "string", "description": "Existing SOP run ID (omit to start a new run from sop_id)" },
                        "sop_id": { "type": "string", "description": "SOP record ID (required when creating a new run)" },
                        "step_index": { "type": "integer", "minimum": 0, "description": "Zero-based step index to update" },
                        "status": { "type": "string", "enum": ["pending", "running", "completed", "skipped", "failed"] },
                        "note": { "type": "string" }
                    },
                    "required": ["step_index", "status"]
                }),
            },
            tier: ToolTier::Deferred,
            domain: CapabilityDomain::System,
            read_only: false,
        }
    }

    fn execute(&self, _ctx: &ToolContext, args: Value) -> ToolExecutionResult {
        let Some(status_raw) = args.get("status").and_then(|v| v.as_str()) else {
            return ToolExecutionResult::err_structured("missing_param", "missing 'status'", None);
        };
        let status = status_raw.trim().to_ascii_lowercase();
        if !matches!(
            status.as_str(),
            "pending" | "running" | "completed" | "skipped" | "failed"
        ) {
            return ToolExecutionResult::err_structured(
                "invalid_status",
                format!(
                    "invalid step status '{status_raw}'; expected pending|running|completed|skipped|failed"
                ),
                None,
            );
        }
        let Some(step_index) = args.get("step_index").and_then(|v| v.as_u64()).map(|n| n as usize) else {
            return ToolExecutionResult::err_structured("missing_param", "missing 'step_index'", None);
        };
        let note = args
            .get("note")
            .and_then(|v| v.as_str())
            .map(ToString::to_string);
        let ts = now_ms();

        let mut run = if let Some(run_id) = args
            .get("run_id")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            match self.suite.get_record::<SopRunRecord>(KIND_SOP_RUN, run_id) {
                Ok(Some(existing)) => existing,
                Ok(None) => {
                    return ToolExecutionResult::err_structured(
                        "not_found",
                        format!("SOP run '{run_id}' not found"),
                        None,
                    );
                }
                Err(e) => {
                    return ToolExecutionResult::err_structured("sop_error", e.to_string(), None);
                }
            }
        } else {
            let Some(sop_id) = args
                .get("sop_id")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
            else {
                return ToolExecutionResult::err_structured(
                    "missing_param",
                    "provide either 'run_id' or 'sop_id'",
                    None,
                );
            };
            let Ok(Some(sop)) = self.suite.get_record::<SopRecord>(KIND_SOP, sop_id) else {
                return ToolExecutionResult::err_structured(
                    "not_found",
                    format!("SOP '{sop_id}' not found"),
                    None,
                );
            };
            let steps = sop
                .steps
                .iter()
                .enumerate()
                .map(|(i, title)| SopStepProgress {
                    index: i,
                    title: title.clone(),
                    status: "pending".into(),
                    note: None,
                    updated_at_ms: ts,
                })
                .collect();
            SopRunRecord {
                id: new_id(),
                sop_id: sop.id,
                sop_name: sop.name,
                status: "pending".into(),
                steps,
                created_at_ms: ts,
                updated_at_ms: ts,
            }
        };

        if step_index >= run.steps.len() {
            return ToolExecutionResult::err_structured(
                "invalid_step_index",
                format!(
                    "step_index {step_index} out of bounds for SOP '{}' ({} step(s))",
                    run.sop_name,
                    run.steps.len()
                ),
                None,
            );
        }

        run.steps[step_index].status = status.clone();
        if note.is_some() {
            run.steps[step_index].note = note;
        }
        run.steps[step_index].updated_at_ms = ts;
        run.status = derive_sop_run_status(&run.steps);
        run.updated_at_ms = ts;

        match self.suite.upsert_record(KIND_SOP_RUN, &run.id, &run) {
            Ok(()) => ToolExecutionResult::ok_structured(
                serde_json::to_value(&run).unwrap_or(json!({})),
                format!(
                    "Updated SOP run '{}' step {} -> {} (run status: {})",
                    run.id, step_index, status, run.status
                ),
            ),
            Err(e) => ToolExecutionResult::err_structured("sop_error", e.to_string(), None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nola_store::Store;

    #[test]
    fn goal_complete_and_sop_run_update_persist_structured_state() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory().unwrap();
        let suite = Arc::new(ModulesSuite::new(store.clone(), dir.path().to_path_buf(), None));
        let mut reg = ToolRegistry::default();
        register_tools(&suite, &mut reg);
        let ctx = ToolContext::new(dir.path());

        // Create goal with acceptance criteria
        let created = reg.execute(
            "goal_manage",
            &ctx,
            json!({
                "action": "create",
                "id": "goal-42",
                "title": "Ship RAG",
                "objective": "Index and search local docs",
                "acceptance_criteria": ["Index folder", "Search hits"]
            }),
        );
        assert!(!created.is_error);

        // Verify backward-compatible deserialization as nola_types::GoalRecord!
        let base_rec: nola_types::GoalRecord =
            store.get_module_record(KIND_GOAL, "goal-42").unwrap().unwrap();
        assert_eq!(base_rec.title, "Ship RAG");

        // Complete goal with criteria verification
        let completed = reg.execute(
            "goal_complete",
            &ctx,
            json!({
                "goal_id": "goal-42",
                "summary": "All RAG checks verified",
                "criteria_results": [
                    {"criterion": "Index folder", "passed": true, "evidence": "20 chunks"},
                    {"criterion": "Search hits", "passed": true, "evidence": "BM25 top-1"}
                ]
            }),
        );
        assert!(!completed.is_error);
        let reloaded: GoalRecord = store.get_module_record(KIND_GOAL, "goal-42").unwrap().unwrap();
        assert!(reloaded.accepted);
        assert_eq!(reloaded.status, GoalStatus::Completed);

        // Create SOP and track step progress via sop_run_update
        let sop = reg.execute(
            "sop_manage",
            &ctx,
            json!({
                "action": "create",
                "id": "sop-deploy",
                "name": "Deploy Checklist",
                "steps": ["Run tests", "Build release"]
            }),
        );
        assert!(!sop.is_error);

        let r1 = reg.execute(
            "sop_run_update",
            &ctx,
            json!({"sop_id": "sop-deploy", "step_index": 0, "status": "completed", "note": "green"}),
        );
        assert!(!r1.is_error);
        let run_id = r1.payload.to_model_value()["id"].as_str().unwrap().to_string();
        assert_eq!(r1.payload.to_model_value()["status"], "running");

        let r2 = reg.execute(
            "sop_run_update",
            &ctx,
            json!({"run_id": run_id, "step_index": 1, "status": "completed"}),
        );
        assert!(!r2.is_error);
        assert_eq!(r2.payload.to_model_value()["status"], "completed");
    }
}
