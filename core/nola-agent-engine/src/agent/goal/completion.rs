// SPDX-License-Identifier: Apache-2.0
//! A model's status declaration, not Goal acceptance or Host authorization.
use serde_json::{json, Map, Value};
pub const TOOL: &str = "goal_complete";
pub const DESCRIPTION: &str = "Report Goal status. Completion requires a claim for every pending requirement, grounded in committed tool results or the delivered response itself. Independent review is optional: request_completion_review is available when you judge it necessary. This report alone does not bypass Core acceptance.";
pub fn schema() -> Value {
    json!({"type":"function","function":{"name":TOOL,"description":DESCRIPTION,"parameters":{
                "type": "object",
                "properties": {
                    "claims": {"type":"array","description":"For completed: cover each pending requirement ID from durable_goal_state exactly once. Cite retained tool:<call_id> results from this Goal; use response only when the assistant response itself is the deliverable, never as proof of external work.","items":{
                        "type":"object","additionalProperties":false,"required":["requirement_id","requirement_revision","summary","evidence_refs","execution_evidence_refs","task_replacements"],
                        "properties":{"requirement_id":{"type":"string"},"requirement_revision":{"type":"integer","minimum":1},"summary":{"type":"string"},
                            "execution_evidence_refs":{"type":"array","items":{"type":"string"},"description":"Only IDs from this Goal with outcome=succeeded and state=completed qualify as typed execution evidence. A successful read with state=null does not qualify: cite tool:<call_id> in evidence_refs and leave this array empty. Required execution checks still need matching typed checks, resource and version; response claims cannot satisfy them."},
                            "task_replacements":{"type":"array","items":{"type":"object","additionalProperties":false,
                                "required":["task_id","terminal_receipt_id","result_digest","execution_evidence_refs"],"properties":{
                                    "task_id":{"type":"string"},"terminal_receipt_id":{"type":["string","null"]},
                                    "result_digest":{"type":["string","null"]},
                                    "execution_evidence_refs":{"type":"array","minItems":1,"items":{"type":"string"}}
                                }}},
                            "evidence_refs":{"type":"array","items":{"type":"string"}}}
                    }},
                    "status": {
                        "type": "string",
                        "enum": ["completed", "continue", "incomplete", "blocked", "needs_user", "failed"],
                        "description": "continue requests automatic progress; incomplete ends this attempt with unfinished requirements and no automatic wake. blocked/needs_user wait for a person. completed still requires canonical Core acceptance."
                    },
                    "summary": {"type": "string", "description": "Concise result summary from current progress."},
                    "evidence": {
                        "type": "array",
                        "description": "Concrete output, checks, diffs, findings, or files confirming current state.",
                        "items": {"type": "string"}
                    },
                    "blockers": {
                        "type": "array",
                        "description": "Known blockers still preventing completion.",
                        "items": {"type": "string"}
                    },
                    "next_action": {"type": "string", "description": "Suggested next action when status is not completed."}
                },
                "required": ["status", "summary"],
                "additionalProperties": false
            }}})
}
pub fn declare(args: &Map<String, Value>) -> Result<Value, String> {
    let status = match args.get("status").and_then(Value::as_str).map(str::trim) {
        Some(value)
            if matches!(
                value,
                "completed" | "continue" | "incomplete" | "blocked" | "needs_user" | "failed"
            ) =>
        {
            value.to_string()
        }
        _ => {
            return Err(
                "status must be one of: completed, continue, incomplete, blocked, needs_user, failed"
                    .to_string(),
            );
        }
    };
    let summary = match args.get("summary").and_then(Value::as_str).map(str::trim) {
        Some(value) if !value.is_empty() => value.to_string(),
        _ => return Err("summary is required".to_string()),
    };
    let evidence = match args.get("evidence") {
        Some(Value::Array(items)) => {
            let mut out = Vec::new();
            for item in items {
                let Some(text) = item
                    .as_str()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                else {
                    return Err("evidence items must be non-empty strings".to_string());
                };
                out.push(text.to_string());
            }
            out
        }
        Some(Value::Null) | None => Vec::new(),
        _ => return Err("evidence must be a list of strings".to_string()),
    };
    let blockers = match args.get("blockers") {
        Some(Value::Array(items)) => {
            let mut out = Vec::new();
            for item in items {
                let Some(text) = item
                    .as_str()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                else {
                    return Err("blockers items must be non-empty strings".to_string());
                };
                out.push(text.to_string());
            }
            out
        }
        Some(Value::Null) | None => Vec::new(),
        _ => return Err("blockers must be a list of strings".to_string()),
    };
    let next_action = args
        .get("next_action")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("");
    if status == "completed" && evidence.is_empty() {
        return Err("completed status requires at least one concrete evidence item".to_string());
    }
    if status == "completed" && !blockers.is_empty() {
        return Err("completed status cannot include unresolved blockers".to_string());
    }
    if matches!(status.as_str(), "blocked" | "needs_user" | "failed")
        && blockers.is_empty()
        && next_action.is_empty()
    {
        return Err(
            "blocked, needs_user, and failed statuses require blockers or next_action".to_string(),
        );
    }

    let claims: Vec<super::GoalCompletionClaim> = match args.get("claims") {
        Some(value) => match serde_json::from_value(value.clone()) {
            Ok(claims) => claims,
            Err(err) => return Err(format!("Invalid requirement claims: {err}")),
        },
        None => Vec::new(),
    };
    if status == "completed" && (claims.is_empty() || claims.iter().any(|claim|
        claim.requirement_id.trim().is_empty() || claim.requirement_revision == 0 || claim.summary.trim().is_empty()
            || (claim.evidence_refs.is_empty() && claim.execution_evidence_refs.is_empty()))) {
        return Err("completed requires one claim per pending requirement with real evidence references (tool:<call_id> or response for a response deliverable)".into());
    }
    Ok(json!({"goal_status":status,"summary":summary,"evidence":evidence,
        "blockers":blockers,"next_action":next_action,"claims":claims}))
}
