// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Defines the deterministic lifecycle for a model-requested tool call.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::protocol::{CallId, ModelToolCall, PermissionRequestId, ToolInvocationContext};

pub const TOOL_INVOCATION_CONTEXT_ARGUMENT: &str = "_nolaExecution";
/// Canonical business arguments, before deduplication, permission evaluation,
/// and execution. Reserved lineage is validated separately, never repaired.
pub(crate) fn canonicalize_tool_arguments(
    arguments: &mut Value,
    descriptor: Option<&super::host::HostCapabilityDescriptor>,
) {
    normalize_encoded_tool_arguments(arguments);
    let lineage = arguments.as_object_mut()
        .and_then(|object| object.remove(TOOL_INVOCATION_CONTEXT_ARGUMENT));
    if let Some(descriptor) = descriptor {
        normalize_unambiguous_schema_scalars(&descriptor.input_schema, arguments);
        if descriptor.validate_input(arguments).err().is_some_and(|error|
            error.contains("unexpected field(s): description")) {
            if let Some(object) = arguments.as_object_mut() {
                object.remove("description");
            }
        }
    }
    if let (Some(lineage), Some(object)) = (lineage, arguments.as_object_mut()) {
        object.insert(TOOL_INVOCATION_CONTEXT_ARGUMENT.into(), lineage);
    }
}

/// Execution identity excludes separately validated invocation ownership.
/// Ownership still participates in within-batch admission and authorization.
pub(crate) fn business_arguments(arguments: &Value) -> Value {
    let mut arguments = arguments.clone();
    normalize_encoded_tool_arguments(&mut arguments);
    if let Some(object) = arguments.as_object_mut() {
        object.remove(TOOL_INVOCATION_CONTEXT_ARGUMENT);
    }
    arguments
}

pub(crate) fn normalize_unambiguous_schema_scalars(schema: &Value, value: &mut Value) -> bool {
    // Do not coerce alternatives, enums, or composition: a string may itself
    // be a valid branch/value. Let strict schema validation explain failures.
    if ["enum", "const", "anyOf", "oneOf", "allOf", "$ref"].iter()
        .any(|key| schema.get(*key).is_some()) { return false; }
    let schema_type = schema.get("type").and_then(Value::as_str);
    match (schema_type, value) {
        (Some("object"), Value::Object(object)) => {
            let Some(properties) = schema.get("properties").and_then(Value::as_object) else {
                return false;
            };
            let mut changed = false;
            for (field, field_schema) in properties {
                if let Some(field_value) = object.get_mut(field) {
                    changed |= normalize_unambiguous_schema_scalars(field_schema, field_value);
                }
            }
            changed
        }
        (Some("array"), Value::Array(items)) => {
            let Some(item_schema) = schema.get("items") else {
                return false;
            };
            items.iter_mut().fold(false, |changed, item| {
                normalize_unambiguous_schema_scalars(item_schema, item) || changed
            })
        }
        (Some("boolean"), current @ Value::String(_)) => {
            let normalized = current.as_str().map(str::trim).map(str::to_ascii_lowercase);
            let replacement = match normalized.as_deref() {
                Some("true") => Some(Value::Bool(true)),
                Some("false") => Some(Value::Bool(false)),
                _ => None,
            };
            if let Some(replacement) = replacement {
                *current = replacement;
                true
            } else {
                false
            }
        }
        (Some("integer"), current @ Value::String(_)) => {
            let replacement = current
                .as_str()
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .and_then(|text| text.parse::<i64>().ok())
                .map(Value::from);
            if let Some(replacement) = replacement {
                *current = replacement;
                true
            } else {
                false
            }
        }
        (Some("number"), current @ Value::String(_)) => {
            let replacement = current
                .as_str()
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .and_then(|text| serde_json::from_str::<Value>(text).ok())
                .filter(Value::is_number);
            if let Some(replacement) = replacement {
                *current = replacement;
                true
            } else {
                false
            }
        }
        _ => false,
    }
}
pub const ACTIVE_SOP_RUNS_COMPONENT_ID: &str = "nola-active-sop-runs-v2";
pub const ACTIVE_SOP_RUNS_CONTRACT_ID: &str = "nola.active_sop_runs.v2";

/// Use one transport normalization boundary before inspecting reserved
/// metadata and before validating business arguments.
pub(crate) fn normalize_encoded_tool_arguments(arguments: &mut Value) -> bool {
    let mut candidate = arguments.clone();
    for _ in 0..4 {
        match candidate {
            Value::Object(_) => {
                if &candidate == arguments { return false; }
                *arguments = candidate;
                return true;
            }
            Value::String(encoded) => {
                let Ok(decoded) = nola_kernel::strict_json::decode(encoded.trim().as_bytes()) else { return false; };
                candidate = decoded;
            }
            _ => return false,
        }
    }
    false
}

/// SOP control capabilities create or transition durable runs. They are never
/// themselves execution of a step in one of those runs, so attaching step
/// lineage to them would be circular.
pub fn is_sop_control_capability(capability_id: &str) -> bool {
    matches!(capability_id, "SOP" | "sop_invoke" | "sop_run_update")
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ActiveSopRunsComponent {
    contract: String,
    runs: Vec<ActiveSopRun>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ActiveSopRun {
    run_id: String,
    status: String,
    definition_pinned: bool,
    steps: Vec<ActiveSopStep>,
}

#[derive(Deserialize)]
struct ActiveSopStep {
    id: String,
    status: String,
}

/// Validates the typed Turn-start snapshot and extracts its running steps.
/// This is contextual evidence, not an authorization source: the Host checks
/// current durable state before permission and execution, including new steps
/// activated after the snapshot was captured.
pub(crate) fn confirmed_sop_lineages_from_components(
    components: &[crate::context_engine::ContextComponent],
) -> Result<Vec<ToolInvocationContext>, String> {
    let matches = components
        .iter()
        .filter(|component| component.id == ACTIVE_SOP_RUNS_COMPONENT_ID)
        .collect::<Vec<_>>();
    if matches.len() > 1 {
        return Err("a Turn accepts at most one active SOP run component".to_string());
    }
    let Some(component) = matches.first() else {
        return Ok(Vec::new());
    };
    if component.kind != crate::context_engine::ContextComponentKind::System {
        return Err("active SOP run context must be a system component".to_string());
    }
    let parsed: ActiveSopRunsComponent = serde_json::from_value(component.content.clone())
        .map_err(|error| format!("invalid active SOP run context: {error}"))?;
    if parsed.contract != ACTIVE_SOP_RUNS_CONTRACT_ID {
        return Err("active SOP run context contract mismatch".to_string());
    }
    let mut confirmed = BTreeSet::new();
    for run in parsed.runs {
        if run.run_id.trim().is_empty() || run.status != "running" || !run.definition_pinned {
            continue;
        }
        for step in run.steps {
            if !step.id.trim().is_empty() && step.status == "running" {
                confirmed.insert(ToolInvocationContext {
                    sop_run_id: run.run_id.clone(),
                    sop_step_id: step.id,
                });
            }
        }
    }
    Ok(confirmed.into_iter().collect())
}

pub use nola_domain::ToolCallStatus;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "stage", rename_all = "snake_case")]
pub enum DelegationWorkspacePreparation {
    Inspect { call_id: CallId, destination: String },
    Create { call_id: CallId, selection: nola_domain::task_workspace::TaskWorktreeSelection,
        source_root:String, source_snapshot:Option<nola_domain::task_workspace::SourceSnapshotPlan> },
}
impl DelegationWorkspacePreparation {
    pub fn call(&self) -> (&CallId, &'static str) {
        match self {
            Self::Inspect { call_id, .. } => (call_id, "git_worktree_list"),
            Self::Create { call_id, .. } => (call_id, "git_worktree_create"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallState {
    /// Core-owned preparation of this exact delegation, retained in checkpoints.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_preparation: Option<DelegationWorkspacePreparation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_resolution_request: Option<nola_domain::permission_resource_plan::PermissionResolutionRequest>,
    #[serde(default)]
    pub permission_recheck: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_resolution: Option<(nola_domain::permission_resource_plan::PermissionResolutionRequest,
        nola_domain::permission_resource_plan::PermissionResolutionFacts)>,
    pub call_id: CallId,
    pub name: String,
    pub arguments: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invocation_context: Option<ToolInvocationContext>,
    /// Malformed execution metadata produces a deterministic skipped result;
    /// it must never degrade into an ordinary unowned tool invocation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invocation_context_rejection: Option<String>,
    /// Core admission decisions retain the original call and settle through
    /// the normal result lifecycle without creating a Host execution effect.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub admission_error: Option<Value>,
    pub status: ToolCallStatus,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub permission_instructions: Vec<nola_host_protocol::PermissionInstruction>,
    #[serde(default)]
    pub permission_scope: Option<String>,
    /// Canonical authorization subject frozen from the Host capability
    /// descriptor after input validation. Permission suspension, recovery,
    /// and conversation-rule re-evaluation must reuse this value rather than
    /// reinterpret a tool name or mutable catalog entry.
    #[serde(default)]
    pub permission_resource: Option<String>,
    #[serde(default)]
    pub permission_rationale: Option<String>,
    #[serde(default)]
    pub permission_request_id: Option<PermissionRequestId>,
    #[serde(default)]
    pub grant: Option<Value>,
    #[serde(default)]
    pub output: Option<Value>,
    #[serde(default)]
    pub is_error: bool,
}

impl ToolCallState {
    pub fn from_model(call: ModelToolCall) -> Self {
        let capability_id = call.name.clone();
        let mut arguments = call.arguments;
        // Some compatible transports hand Core encoded JSON strings. Normalize
        // before extracting protocol metadata, not later in schema repair;
        // both plain and nested encodings must reach the same lineage checks.
        normalize_encoded_tool_arguments(&mut arguments);
        let supplied_context = arguments
            .as_object_mut()
            .and_then(|object| object.remove(TOOL_INVOCATION_CONTEXT_ARGUMENT));
        let (invocation_context, invocation_context_rejection) = if is_sop_control_capability(
            &capability_id,
        ) || capability_id == "request_user_input" {
            // A top-level SOP start/update is the control plane that
            // establishes the run. Ignore provider-invented execution
            // metadata instead of turning a valid control call into a
            // reference to a run that cannot exist yet. User interaction is
            // also session control, not execution owned by a workflow step:
            // a malformed model-supplied step must not prevent asking the user.
            (None, None)
        } else if let Some(value) = supplied_context {
            match serde_json::from_value::<ToolInvocationContext>(value) {
                    Ok(context)
                        if !context.sop_run_id.trim().is_empty()
                            && !context.sop_step_id.trim().is_empty() =>
                    {
                        // This is a reference, never authorization. The Host
                        // validates its live owner/workspace/run/step before
                        // permission or execution. A Turn-start snapshot is
                        // stale after SOP start/resume in this same Turn.
                        (Some(context), None)
                    }
                    Ok(_) => (
                        None,
                        Some(
                            "SOP execution lineage requires non-empty run and step identifiers"
                                .to_string(),
                        ),
                    ),
                    Err(error) => (
                        None,
                        Some(format!("invalid SOP execution lineage: {error}")),
                    ),
                }
        } else {
            (None, None)
        };
        Self {
            workspace_preparation: None,
            call_id: call.call_id,
            permission_resolution: None,
            permission_resolution_request: None,
            permission_recheck: false,
            name: call.name,
            arguments,
            invocation_context,
            invocation_context_rejection,
            admission_error: None,
            status: ToolCallStatus::Pending,
            permission_instructions: Vec::new(),
            // Authorization is Host policy, never model-provided data. Core
            // resolves these fields from the active Host catalog immediately
            // before scheduling the tool.
            permission_scope: None,
            permission_resource: None,
            permission_rationale: None,
            permission_request_id: None,
            grant: None,
            output: None,
            is_error: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn canonical_admission_preserves_lineage_and_matches_executed_business_arguments() {
        use crate::agent::host::*;
        let descriptor = HostCapabilityDescriptor {
            id: "inspect".into(), title: "Inspect".into(), route: HostCapabilityRoute::Tool,
            availability: CapabilityAvailability::Available, exposure: Default::default(),
            permission_domain: None,
            input_schema: json!({"type":"object","additionalProperties":false,
                "properties":{"limit":{"type":"integer"},"path":{"type":"string"}}}),
            input_normalization: Default::default(),
            output_policy: HostOutputPolicy { preview_char_limit:1000, persist_full_output:true, allow_artifacts:true },
            supports_streaming:false, supports_argument_streaming:false,
            parallelism:Default::default(), cancellation:Default::default(), replay_safety:Default::default(),
        };
        let metadata = json!({"invalid":"must remain rejected"});
        let mut encoded = json!(json!({"path":"/workspace/a", "limit":"2",
            "description":"provider prose", "_nolaExecution":metadata}).to_string());
        canonicalize_tool_arguments(&mut encoded, Some(&descriptor));
        assert_eq!(encoded["_nolaExecution"], metadata);
        assert_eq!(business_arguments(&encoded), json!({"path":"/workspace/a","limit":2}));
        let canonical = encoded.clone();
        canonicalize_tool_arguments(&mut encoded, Some(&descriptor));
        assert_eq!(encoded, canonical, "canonicalization must be idempotent");
        let state = ToolCallState::from_model(ModelToolCall {
            call_id: CallId::from_string("canonical".into()), name:"inspect".into(),
            arguments:encoded, permission_scope:None, permission_rationale:None,
        });
        assert_eq!(state.arguments, business_arguments(&canonical));
        assert!(state.invocation_context_rejection.is_some(), "canonicalization cannot repair authorization lineage");
    }

    #[test]
    fn active_sop_payload_uses_its_schema_contract_independently_of_component_routing() {
        let components = vec![crate::context_engine::ContextComponent {
            kind: crate::context_engine::ContextComponentKind::System,
            id: ACTIVE_SOP_RUNS_COMPONENT_ID.to_string(),
            content: json!({
                "contract": ACTIVE_SOP_RUNS_CONTRACT_ID,
                "runs": [{
                    "runId": "run-1",
                    "status": "running",
                    "definitionPinned": true,
                    "steps": [{"id": "write", "status": "running"}]
                }]
            }),
            source: None,
        }];

        assert_eq!(
            confirmed_sop_lineages_from_components(&components).unwrap(),
            vec![ToolInvocationContext {
                sop_run_id: "run-1".into(),
                sop_step_id: "write".into(),
            }]
        );
    }

    #[test]
    fn execution_lineage_is_removed_from_business_arguments_and_preserved() {
        let state = ToolCallState::from_model(
            ModelToolCall {
                call_id: CallId::from_string("call-1".into()),
                name: "write_file".into(),
                arguments: json!({
                    "path": "output.txt",
                    "_nolaExecution": {
                        "sopRunId": "run-1",
                        "sopStepId": "write"
                    }
                }),
                permission_scope: None,
                permission_rationale: None,
            },
        );
        assert_eq!(state.arguments, json!({"path": "output.txt"}));
        let context = state.invocation_context.expect("execution lineage");
        assert_eq!(context.sop_run_id, "run-1");
        assert_eq!(context.sop_step_id, "write");
        assert!(state.invocation_context_rejection.is_none());
    }

    #[test]
    fn execution_lineage_is_preserved_for_authoritative_host_validation() {
        let state = ToolCallState::from_model(ModelToolCall {
            call_id: CallId::from_string("call-2".into()),
            name: "write_file".into(),
            arguments: json!({
                "path": "output.txt",
                "_nolaExecution": {"sopRunId": "invented", "sopStepId": "write"}
            }),
            permission_scope: None,
            permission_rationale: None,
        });
        assert_eq!(state.arguments, json!({"path": "output.txt"}));
        assert_eq!(state.invocation_context.unwrap().sop_run_id, "invented");
        assert!(state.invocation_context_rejection.is_none());
    }

    #[test]
    fn ambiguous_json_cannot_be_revived_by_transport_normalization() {
        let raw = r#"{"messages":[{"version":"0.9.1","version":"1.0"}]}"#;
        let mut encoded = Value::String(raw.into());
        for depth in 0..3 {
            let original = encoded.clone();
            let mut candidate = original.clone();
            assert!(!normalize_encoded_tool_arguments(&mut candidate));
            assert_eq!(candidate, original);
            let state = ToolCallState::from_model(ModelToolCall {
                call_id: CallId::from_string(format!("duplicate-{depth}")),
                name: "render_ui".into(), arguments: original.clone(),
                permission_scope: None, permission_rationale: None,
            });
            assert_eq!(state.arguments, original);
            assert!(!state.arguments.is_object());
            encoded = Value::String(serde_json::to_string(&encoded).unwrap());
        }
    }

    #[test]
    fn encoded_arguments_are_normalized_before_lineage_stripping() {
        let state = ToolCallState::from_model(ModelToolCall {
            call_id: CallId::from_string("call-encoded".into()),
            name: "memory_save".into(),
            arguments: Value::String(
                r#"{"content":"fact","_nolaExecution":{"sopRunId":"invented","sopStepId":"save"}}"#
                    .into(),
            ),
            permission_scope: None,
            permission_rationale: None,
        });
        assert_eq!(state.arguments, json!({"content": "fact"}));
        assert_eq!(state.invocation_context.unwrap().sop_run_id, "invented");
        assert!(state.invocation_context_rejection.is_none());
    }

    #[test]
    fn nested_transport_encoding_cannot_hide_execution_metadata() {
        let mut arguments = json!({"path": "file.txt", "_nolaExecution": {"sopRunId": "run", "sopStepId": ""}});
        for depth in 1..=3 {
            arguments = Value::String(serde_json::to_string(&arguments).unwrap());
            let state = ToolCallState::from_model(ModelToolCall {
                call_id: CallId::from_string(format!("encoded-{depth}")),
                name: "read_file".into(), arguments: arguments.clone(),
                permission_scope: None, permission_rationale: None,
            });
            assert_eq!(state.arguments, json!({"path": "file.txt"}));
            assert!(state.invocation_context_rejection.is_some());
        }
    }

    #[test]
    fn top_level_sop_control_never_carries_step_lineage() {
        let state = ToolCallState::from_model(ModelToolCall {
            call_id: CallId::from_string("call-3".into()),
            name: "SOP".into(),
            arguments: json!({
                "sop": "workflow.example",
                "inputs": {},
                "_nolaExecution": {"sopRunId": "workflow:resolve", "sopStepId": "resolve"}
            }),
            permission_scope: None,
            permission_rationale: None,
        });
        assert_eq!(
            state.arguments,
            json!({"sop": "workflow.example", "inputs": {}})
        );
        assert!(state.invocation_context.is_none());
        assert!(state.invocation_context_rejection.is_none());
    }

    #[test]
    fn user_interaction_is_not_owned_by_model_supplied_workflow_lineage() {
        for metadata in [json!({"sopRunId":"terminal-work:resolve", "sopStepId":"resolve"}), json!("malformed")] {
            let state = ToolCallState::from_model(ModelToolCall {
                call_id: CallId::from_string("question".into()),
                name: "request_user_input".into(),
                arguments: json!({"questions":[], "_nolaExecution":metadata}),
                permission_scope: None, permission_rationale: None,
            });
            assert_eq!(state.arguments, json!({"questions":[]}));
            assert!(state.invocation_context.is_none());
            assert!(state.invocation_context_rejection.is_none());
        }
    }
}
