// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Reduces permission and tool lifecycles and plans executable tool effects.
// - Boundary: Private deterministic reducer layer; public clients use AgentCore.

use super::*;
use crate::agent::protocol::AgentInputOrigin;
use crate::agent::host::{validate_host_artifacts, HostToolOutcome};
use crate::agent::memory_runtime::{
    MemoryRuntimeCommand, MemoryRuntimeCommandEnvelope, MemoryUsageLedgerEntry,
};
use crate::agent::AgentInvokedSkill;

fn invoked_skill_from_output(output: &Value) -> Option<AgentInvokedSkill> {
    (output.get("type").and_then(Value::as_str) == Some("invoked_skill"))
        .then(|| AgentInvokedSkill {
            skill_id: output
                .get("skill_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            version: output
                .get("version")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            instructions_ref: output
                .get("instructions_ref")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            instructions_digest: output
                .get("instructions_digest")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            base_directory: output
                .get("base_directory")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            arguments: output
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| Value::Object(Default::default())),
            instructions: output
                .get("instructions")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            // MaterializedSkillInvocation is the typed authority for required
            // capabilities. Tool output owns instructions and display data,
            // but must not introduce a parallel stringly-typed tool contract.
            required_tools: Vec::new(),
            validation_steps: output
                .get("validation_steps")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect(),
            activation: match output
                .get("activation")
                .or_else(|| output.get("scope"))
                .and_then(Value::as_str)
            {
                Some("session" | "conversation" | "pinned") => {
                    crate::agent::AgentSkillActivation::Session
                }
                _ => crate::agent::AgentSkillActivation::Turn,
            },
        })
        .filter(|skill| !skill.skill_id.is_empty() && !skill.instructions.is_empty())
}

fn normalized_capability_activation(
    mut activation: crate::agent::host::HostCapabilityActivation,
) -> Option<crate::agent::host::HostCapabilityActivation> {
    activation.source = activation.source.trim().to_string();
    let mut seen = std::collections::BTreeSet::new();
    activation.capability_ids = activation
        .capability_ids
        .into_iter()
        .map(|capability_id| capability_id.trim().to_string())
        .filter(|capability_id| !capability_id.is_empty())
        .filter(|capability_id| seen.insert(capability_id.clone()))
        .collect::<Vec<_>>();
    (matches!(
        activation.source.as_str(),
        "tool_search" | "full_output_ref" | "skill" | "host_resource_preservation" | "mcp_connection" | "code_memory"
    ) && !activation.capability_ids.is_empty())
    .then_some(activation)
}

fn activation_from_tool_result(
    activation: Option<crate::agent::host::HostCapabilityActivation>,
    is_error: bool,
) -> Option<crate::agent::host::HostCapabilityActivation> {
    let activation = activation.and_then(normalized_capability_activation)?;
    // A Host-owned preservation guard can advertise a safe recovery schema
    // even when the guarded process fails. This exposes a tool, not a grant:
    // subsequent calls still pass the ordinary resource/permission checks.
    (!is_error || activation.source == "host_resource_preservation").then_some(activation)
}

fn is_capability_activation_protocol(capability_id: &str) -> bool {
    matches!(capability_id, "tool_search" | "Skill" | "skill_invoke" | "mcp_connect" | "codebase_index")
}

#[cfg(test)]
mod invoked_skill_output_tests {
    use super::*;

    #[test]
    fn accepts_only_root_structured_skill_payload() {
        let output = serde_json::json!({
            "type": "invoked_skill",
            "skill_id": "word-document-authoring",
            "version": "1.0.0",
            "instructions_ref": "skill://word-document-authoring/1.0.0/SKILL.md#sha256=0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "instructions_digest": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            "base_directory": "/tmp/skills/word-document-authoring",
            "arguments": "create report.docx",
            "instructions": "Follow the Word workflow",
            "required_tools": [
                {"provider":"builtin","tool_name":"read_file"},
                {"provider":"builtin","tool_name":"write_file"},
                {"provider":"builtin","tool_name":"bash"}
            ],
            "validation_steps": ["create_document.py --validate-only --output report.docx"]
        });
        let skill = invoked_skill_from_output(&output).expect("structured Skill payload");
        assert_eq!(skill.skill_id, "word-document-authoring");
        assert_eq!(skill.version, "1.0.0");
        assert!(skill.instructions_ref.ends_with(&skill.instructions_digest));
        assert_eq!(skill.instructions, "Follow the Word workflow");
        assert!(skill.required_tools.is_empty());
        assert_eq!(
            skill.validation_steps,
            vec!["create_document.py --validate-only --output report.docx"]
        );
        assert!(invoked_skill_from_output(&serde_json::json!({
            "text": output.to_string()
        }))
        .is_none());
    }

}
use crate::agent::permissions::{PermissionRule, PermissionRuleSource};

fn engineering_failure_summary(output: &Value) -> String {
    if let Value::Object(object) = output {
        let mut fields = Vec::new();
        for key in ["error", "message", "next_action"] {
            if let Some(value) = object.get(key) {
                let value = value
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| value.to_string());
                if !value.trim().is_empty() {
                    fields.push(format!("{key}={value}"));
                }
            }
        }
        if !fields.is_empty() {
            return fields.join("; ").chars().take(1_200).collect();
        }
    }
    let rendered = match output {
        Value::String(value) => value.clone(),
        value => serde_json::to_string(value).unwrap_or_else(|_| value.to_string()),
    };
    let mut summary = rendered.chars().take(1_200).collect::<String>();
    if rendered.chars().count() > 1_200 {
        summary.push_str("...");
    }
    summary
}

fn model_tool_not_exposed_output(tool_name: &str, catalog_revision: u64) -> Value {
    json!({
        "error": "tool_not_exposed_in_model_step",
        "requested_tool": tool_name,
        "tool_execution_skipped": true,
        "recovery": {
            "kind": "discover_capability",
            "tool": "tool_search",
            "arguments": { "query": format!("select:{tool_name}") },
            "catalog_revision": catalog_revision
        },
        "next_action": "If tool_search is exposed, use the exact recovery query once. Call the capability only after it appears in a later model-step schema. An exact missing result means that identifier is absent from the current catalog, not that every alternative is unavailable. Do not repeat an unchanged lookup; consider an authorized alternative or honestly report the limitation."
    })
}

impl AgentRuntime {
    pub(super) fn install_capability_activation_descriptors(
        &mut self,
        turn_id: &TurnId,
        activation: &crate::agent::host::HostCapabilityActivation,
    ) -> Result<(), String> {
        let selected = {
            let turn = self
                .session
                .execution
                .active_turn
                .as_mut()
                .ok_or_else(|| "capability activation has no active turn".to_string())?;
            if &turn.turn_id != turn_id {
                return Err("capability activation turn identity does not match".to_string());
            }
            turn.context
                .capability_projection
                .install_activated_descriptors(
                    &activation.capability_ids,
                    activation.capability_descriptors.clone(),
                )?;
            activation
                .capability_ids
                .iter()
                .filter_map(|id| {
                    turn.context
                        .capability_projection
                        .descriptor(id)
                        .cloned()
                        .map(|descriptor| (id.clone(), descriptor))
                })
                .collect::<Vec<_>>()
        };
        self.session.host.catalog.capabilities.extend(selected);
        Ok(())
    }

    pub(super) fn schedule_tools(
        &mut self,
        turn_id: &TurnId,
        events: &mut Vec<RuntimeEvent>,
        effects: &mut Vec<AgentEffect>,
    ) -> Result<(), String> {
        self.prepare_delegation_workspaces(turn_id)?;
        let primary_workspace_root=self.primary_native_workspace_root()?;
        let model_in_flight = self.model_request_is_in_flight(turn_id, effects);
        let permission_rules = self.session.execution.permission_rules.clone();
        let host_catalog = self
            .session
            .execution
            .active_turn
            .as_ref()
            .ok_or_else(|| "no active turn".to_string())?
            .context
            .capability_projection
            .frozen_host_catalog();
        let input_origin = self
            .session
            .execution
            .active_turn
            .as_ref()
            .ok_or_else(|| "no active turn".to_string())?
            .context
            .input_origin;
        let exposed_model_tools = self
            .session
            .execution
            .active_turn
            .as_ref()
            .and_then(|turn| turn.last_model_tool_projection.as_ref())
            .map(|projection| {
                projection
                    .capability_ids
                    .iter()
                    .cloned()
                    .collect::<std::collections::BTreeSet<_>>()
            })
            .unwrap_or_default();
        let explicit_delegation_call = self.session.execution.active_turn.as_ref()
            .and_then(|turn| turn.user_delegation_call.clone());
        let workspace_preparation_calls = self.session.execution.active_turn.as_ref()
            .map(|turn| turn.tools.iter().filter_map(|tool| tool.workspace_preparation.as_ref())
                .chain(turn.primary_workspace_preparation.as_ref())
                .map(|preparation| { let (id,name)=preparation.call(); (id.as_str().to_owned(),name) })
                .collect::<std::collections::BTreeMap<_,_>>()).unwrap_or_default();
        let completed_skill_runs = self
            .session
            .skill_runtime
            .runs
            .values()
            .filter(|run| {
                run.owner.parent_turn_id == turn_id.as_str()
                    && run.status == crate::agent::capability::SkillRunStatus::Completed
            })
            .map(|run| (run.run_id.clone(), run.descriptor.skill_id.clone()))
            .collect::<Vec<_>>();
        let execution_target_admission = self.execution_target_admission();
        let code_calls = self.session.execution.active_turn.as_ref()
            .map(|turn| turn.code_cells.values().flat_map(|cell| cell.pending_calls().map(|call|
                (cell.nested_call_id(call.sequence), call.tool_name.clone())))
                .collect::<std::collections::BTreeMap<_, _>>())
            .unwrap_or_default();
        let code_available_tools = self.session.execution.active_turn.as_ref()
            .map(|turn| turn.context.capability_projection.available_capabilities.values()
                .filter(|descriptor| descriptor.route == crate::agent::host::HostCapabilityRoute::Tool
                    && descriptor.availability == CapabilityAvailability::Available
                    && descriptor.exposure.is_model_discoverable())
                .map(|descriptor| descriptor.id.clone()).collect::<std::collections::BTreeSet<_>>())
            .unwrap_or_default();
        let batch = {
            let turn = self
                .session
                .execution
                .active_turn
                .as_ref()
                .ok_or_else(|| "no active turn".to_string())?;
            if &turn.turn_id != turn_id {
                return Err("turn id does not match the active turn".to_string());
            }
            ToolExecutionRuntime::new(&turn.schedulable_tools(), &host_catalog).next_batch()
        };
        let goal_reporting = matches!(input_origin, AgentInputOrigin::GoalInteractiveUser | AgentInputOrigin::GoalContinuation | AgentInputOrigin::GoalAudit)
            && self.session.goal_projection.active.as_ref().is_some_and(|goal| !goal.status.is_terminal());
        let id_session = self.session.session_id.as_str().to_string();
        let mut next_id_sequence = self.session.journal.next_id_sequence;
        let mut allocate_id = |kind: &str| {
            next_id_sequence = next_id_sequence.saturating_add(1);
            format!("{id_session}:{kind}:{next_id_sequence}")
        };
        let mut terminal_results = Vec::new();
        let mut pending_permission_requests = Vec::new();
        let mut opened_waits = Vec::new();
        let wait_revision = self.session.journal.revision.checked_add(1).ok_or("Core revision exhausted")?;
        let (all_terminal, has_pending_permission, has_running, has_pending) = {
            let turn = self
                .session
                .execution
                .active_turn
                .as_mut()
                .ok_or_else(|| "no active turn".to_string())?;
            if &turn.turn_id != turn_id {
                return Err("turn id does not match the active turn".to_string());
            }
            turn.status = AgentTurnStatus::AwaitingTools;
            for call_id in &batch.call_ids {
                let Some(tool) = turn.tools.iter_mut().find(|tool| &tool.call_id == call_id) else {
                    continue;
                };
                if tool.status != ToolCallStatus::Pending {
                    continue;
                }
                if primary_workspace_root.as_ref().is_some_and(|root|tool.permission_resolution.as_ref().is_some_and(|(_,facts)|
                    super::delegation_workspace::source_effect_requires_worker(std::path::Path::new(root),&tool.name,facts.bindings.iter().map(|binding|&binding.resource)))) {
                    tool.admission_error=Some(json!({"error":"dedicated_tool_required","tool_execution_skipped":true,
                        "message":"Code mode keeps the primary conversation as coordinator; source-checkout edits execute in a managed workspace worker.",
                        "recovery":{"tool":"spawn_agent","agent_type":"workspace"},
                        "next_action":"Delegate implementation, inspect the returned workspace paths, verify there and use git_worktree_integrate only for reviewed delivery. Do not evade source protection through another cwd or an absolute source path. Ordinary approvals remain required."}));
                }
                // Invalid execution lineage is a rejected call, not permission
                // to run the same business arguments without their owner.
                // Check the durable rejection here as well as at admission so
                // restored pending calls cannot bypass this boundary.
                let lineage_error = tool.invocation_context_rejection.as_ref().map(|message| json!({
                    "error": "invalid_sop_execution_lineage",
                    "message": message,
                    "tool_execution_skipped": true,
                    "recovery": {
                        "kind": "refresh_sop_execution_context",
                        "tool": "sop_run_update"
                    }
                }));
                if let Some(output) = lineage_error.or_else(|| tool.admission_error.take()) {
                    tool.status = ToolCallStatus::Failed;
                    tool.output = Some(output.clone());
                    tool.is_error = true;
                    terminal_results.push(AgentItem::ToolResult {
                        id: ItemId::from_string(allocate_id("tool_result")),
                        call_id: tool.call_id.clone(), name: tool.name.clone(),
                        output: output.clone(), is_error: true,
                        full_output_ref: None, artifacts: Vec::new(),
                    });
                    events.push(RuntimeEvent::ToolCompleted {
                        turn_id: turn_id.clone(), call_id: tool.call_id.clone(),
                        tool_name: tool.name.clone(), output,
                        outcome: HostToolOutcome::Failed,
                        full_output_ref: None, artifacts: Vec::new(),
                    });
                    continue;
                }
                let was_exposed = (tool.name == "spawn_agent" && explicit_delegation_call.as_ref() == Some(&tool.call_id))
                    || workspace_preparation_calls.get(tool.call_id.as_str()).is_some_and(|name| *name == tool.name)
                    || exposed_model_tools.contains(&tool.name)
                    || (code_calls.get(tool.call_id.as_str()) == Some(&tool.name)
                        && code_available_tools.contains(&tool.name)
                        && host_catalog.resolve_available(&tool.name).is_some_and(|descriptor|
                            descriptor.exposure.is_model_discoverable()));
                // Status reporting is a pure Core declaration on every Host.
                // Normal model exposure/lineage and later evidence acceptance
                // remain authoritative; no platform capability or side effect.
                if tool.name == crate::agent::goal::completion::TOOL && was_exposed {
                    let result = goal_reporting.then_some(&tool.arguments).ok_or_else(|| "No active Goal reporting scope".to_string())
                        .and_then(|arguments| arguments.as_object().ok_or_else(|| "Goal declaration must be an object".to_string()))
                        .and_then(crate::agent::goal::completion::declare);
                    let failed = result.is_err();
                    let output = result.unwrap_or_else(|error| json!({"error":error}));
                    tool.status = if failed { ToolCallStatus::Failed } else { ToolCallStatus::Completed };
                    tool.output = Some(output.clone()); tool.is_error = failed;
                    terminal_results.push(AgentItem::ToolResult {
                        id: ItemId::from_string(allocate_id("tool_result")), call_id: tool.call_id.clone(),
                        name: tool.name.clone(), output: output.clone(), is_error: failed,
                        full_output_ref: None, artifacts: Vec::new(),
                    });
                    events.push(RuntimeEvent::ToolCompleted { turn_id: turn_id.clone(),
                        call_id: tool.call_id.clone(), tool_name: tool.name.clone(), output,
                        outcome: if failed { HostToolOutcome::Failed } else { HostToolOutcome::Succeeded },
                        full_output_ref: None, artifacts: Vec::new(),
                    });
                    continue;
                }
                let settled_skill_update = if tool.name == "skill_update" {
                    tool.arguments
                        .get("run_id")
                        .and_then(Value::as_str)
                        .is_some_and(|requested| {
                            completed_skill_runs
                                .iter()
                                .any(|(run_id, _)| run_id == requested)
                                || completed_skill_runs
                                    .iter()
                                    .filter(|(_, skill_id)| skill_id == requested)
                                    .count()
                                    == 1
                        })
                } else {
                    false
                };
                if settled_skill_update {
                    let output = json!({
                        "type": "skill_runtime_update",
                        "status": "already_completed",
                        "tool_execution_skipped": true,
                        "next_action": "The Skill execution contract is already satisfied and closed. Provide the final answer without another skill_update."
                    });
                    tool.status = ToolCallStatus::Completed;
                    tool.output = Some(output.clone());
                    tool.is_error = false;
                    terminal_results.push(AgentItem::ToolResult {
                        id: ItemId::from_string(allocate_id("tool_result")),
                        call_id: tool.call_id.clone(),
                        name: tool.name.clone(),
                        output: output.clone(),
                        is_error: false,
                        full_output_ref: None,
                        artifacts: Vec::new(),
                    });
                    events.push(RuntimeEvent::ToolCompleted {
                        turn_id: turn_id.clone(),
                        call_id: tool.call_id.clone(),
                        tool_name: tool.name.clone(),
                        output,
                        outcome: HostToolOutcome::Succeeded,
                        full_output_ref: None,
                        artifacts: Vec::new(),
                    });
                    continue;
                }
                let capability = was_exposed
                    .then(|| host_catalog.resolve_available(&tool.name))
                    .flatten()
                    .filter(|capability| capability.route == HostCapabilityRoute::Tool);
                let Some(capability) = capability else {
                    let output = if was_exposed {
                        Value::String(format!(
                            "frozen Turn registry does not provide tool capability {}",
                            tool.name
                        ))
                    } else {
                        model_tool_not_exposed_output(&tool.name, host_catalog.revision)
                    };
                    tool.status = ToolCallStatus::Failed;
                    tool.output = Some(output.clone());
                    tool.is_error = true;
                    let tool_name = tool.name.clone();
                    turn.engineering
                        .observe_tool_result_for_call_with_resources(
                            &tool_name,
                            crate::agent::host::HostCapabilityEffect::Execute,
                            false,
                            true,
                            Some(engineering_failure_summary(&output)),
                            true,
                            None,
                            &crate::agent::host::HostResourceProjectionDeclaration::default(),
                        );
                    events.push(RuntimeEvent::EngineeringLoopUpdated {
                        turn_id: turn_id.clone(),
                        state: turn.engineering.clone(),
                    });
                    terminal_results.push(AgentItem::ToolResult {
                        id: ItemId::from_string(allocate_id("tool_result")),
                        call_id: tool.call_id.clone(),
                        name: tool.name.clone(),
                        output: output.clone(),
                        is_error: true,
                        full_output_ref: None,
                        artifacts: Vec::new(),
                    });
                    events.push(RuntimeEvent::ToolCompleted {
                        turn_id: turn_id.clone(),
                        call_id: tool.call_id.clone(),
                        tool_name: tool.name.clone(),
                        output,
                        outcome: HostToolOutcome::Failed,
                        full_output_ref: None,
                        artifacts: Vec::new(),
                    });
                    continue;
                };

                // Providers occasionally return the function arguments as a JSON
                // string (and some compatible gateways double-encode that string).
                // Canonicalize it before schema validation, permission matching,
                // failure signatures, and persistence all observe the call.
                crate::agent::tool_lifecycle::canonicalize_tool_arguments(
                    &mut tool.arguments, Some(&capability));
                let mut validation_error = capability.validate_input(&tool.arguments).err();
                if validation_error.is_none() && tool.name == "request_user_input" {
                    validation_error = serde_json::from_value::<nola_domain::user_input_wait::QuestionSet>(tool.arguments.clone())
                        .map_err(|error| error.to_string()).and_then(|questions| questions.validate()).err();
                }
                if let Some(message) = validation_error {
                    let tool_name = tool.name.clone();
                    let arguments_were_truncated = likely_truncated_tool_arguments(&tool.arguments);
                    let error_code = if arguments_were_truncated {
                        "tool_arguments_truncated"
                    } else {
                        "invalid_tool_arguments"
                    };
                    let next_action = if arguments_were_truncated {
                        truncated_argument_recovery_action(&tool_name)
                    } else {
                        format!(
                            "Call {tool_name} with one JSON object matching expected_schema. Do not wrap the arguments object in a string or array."
                        )
                    };
                    let output = json!({
                        "error": error_code,
                        "message": message,
                        "expected_schema": capability.input_schema.clone(),
                        "tool_execution_skipped": true,
                        "next_action": next_action,
                    });
                    tool.status = ToolCallStatus::Failed;
                    tool.output = Some(output.clone());
                    tool.is_error = true;
                    let arguments = tool.arguments.clone();
                    let effect = capability.effect_for(&arguments);
                    turn.engineering
                        .observe_tool_result_for_call_with_resources(
                            &tool_name,
                            effect,
                            capability.mutation_requires_validation_for(&arguments),
                            true,
                            Some(engineering_failure_summary(&output)),
                            true,
                            Some(&arguments),
                            &capability.input_normalization.resource_projection,
                        );
                    events.push(RuntimeEvent::EngineeringLoopUpdated {
                        turn_id: turn_id.clone(),
                        state: turn.engineering.clone(),
                    });
                    terminal_results.push(AgentItem::ToolResult {
                        id: ItemId::from_string(allocate_id("tool_result")),
                        call_id: tool.call_id.clone(),
                        name: tool.name.clone(),
                        output: output.clone(),
                        is_error: true,
                        full_output_ref: None,
                        artifacts: Vec::new(),
                    });
                    events.push(RuntimeEvent::ToolCompleted {
                        turn_id: turn_id.clone(),
                        call_id: tool.call_id.clone(),
                        tool_name: tool.name.clone(),
                        output,
                        outcome: HostToolOutcome::Failed,
                        full_output_ref: None,
                        artifacts: Vec::new(),
                    });
                    continue;
                }

                let tool_name = tool.name.clone();
                let arguments = tool.arguments.clone();
                if let Err(message) = execution_target_admission.validate(
                    capability.input_normalization.execution_target.as_ref(), &arguments) {
                    let output = json!({"error":"execution_target_constraint","message":message,
                        "tool_execution_skipped":true,"recovery":{"tool":"execution_constraints_update",
                            "preserve_recorded_target":true}});
                    tool.status = ToolCallStatus::Failed;
                    tool.output = Some(output.clone());
                    tool.is_error = true;
                    terminal_results.push(AgentItem::ToolResult {
                        id: ItemId::from_string(allocate_id("tool_result")),call_id:tool.call_id.clone(),
                        name:tool.name.clone(),output:output.clone(),is_error:true,full_output_ref:None,artifacts:Vec::new() });
                    events.push(RuntimeEvent::ToolCompleted { turn_id:turn_id.clone(),call_id:tool.call_id.clone(),
                        tool_name:tool.name.clone(),output,outcome:HostToolOutcome::Failed,full_output_ref:None,artifacts:Vec::new() });
                    continue;
                }
                if let Some(plan) = capability.input_normalization.permission_contract.resource_plan_for(&arguments) {
                    if tool.permission_resolution.is_none() {
                        let request = nola_domain::permission_resource_plan::PermissionResolutionRequest {
                            resolution_id: allocate_id("permission-resolution"), execution_target: host_catalog.host_id.0.clone(),
                            invocation_digest: nola_domain::permission_resource_plan::PermissionResolutionRequest::input_digest(&arguments),
                            capability: plan.capability.clone(), operation: plan.operation.clone(),
                            demands: plan.extract(&arguments).map_err(|error| error.to_string())?,
                        };
                        // This is a read-only Host resolution, not tool execution.
                        // Running keeps the original call owned while awaiting facts.
                        tool.status = ToolCallStatus::Running;
                        tool.permission_resolution_request = Some(request.clone());
                        effects.push(AgentEffect::ResolvePermissionResources { turn_id: turn_id.clone(),
                            call_id: tool.call_id.clone(), tool_name: tool.name.clone(), input: arguments.clone(), request });
                        continue;
                    }
                }
                let effect = capability.effect_for(&arguments);
                let admission = if std::mem::take(&mut tool.permission_recheck) { Ok(()) } else { turn.engineering.admit_tool_execution_with_progress(
                    &tool_name,
                    &arguments,
                    effect,
                    &capability.input_normalization.loop_progress,
                ) };
                if let Err(message) = admission {
                    let output = json!({
                        "error": message.split(';').next().unwrap_or("investigation_rejected"),
                        "message": message,
                        "tool_execution_skipped": true,
                        "next_action": "Choose a different relevant evidence source or strategy within remaining budgets, or report unfinished work."
                    });
                    tool.status = ToolCallStatus::Failed;
                    tool.output = Some(output.clone());
                    tool.is_error = true;
                    events.push(RuntimeEvent::EngineeringLoopUpdated {
                        turn_id: turn_id.clone(),
                        state: turn.engineering.clone(),
                    });
                    terminal_results.push(AgentItem::ToolResult {
                        id: ItemId::from_string(allocate_id("tool_result")),
                        call_id: tool.call_id.clone(),
                        name: tool.name.clone(),
                        output: output.clone(),
                        is_error: true,
                        full_output_ref: None,
                        artifacts: Vec::new(),
                    });
                    events.push(RuntimeEvent::ToolCompleted {
                        turn_id: turn_id.clone(),
                        call_id: tool.call_id.clone(),
                        tool_name: tool.name.clone(),
                        output,
                        outcome: HostToolOutcome::Failed,
                        full_output_ref: None,
                        artifacts: Vec::new(),
                    });
                    continue;
                }

                let mut subject = capability.permission_subject(&tool.arguments);
                let resolved_subject = tool.permission_resolution.as_ref().map(|(request,facts)|
                    facts.subject_for(request).map_err(|error|error.to_string())).transpose()?;
                if let (Some(subject),Some((request,facts))) = (subject.as_mut(),tool.permission_resolution.as_ref()) {
                    subject.resource = facts.subject_for(request).map_err(|error|error.to_string())?
                        .approval_resource_identity().map_err(|error|error.to_string())?;
                    let process_risks = facts.process_approval_risks();
                    if !process_risks.is_empty() {
                        subject.resource = json!({"resolved_scope":subject.resource,"process_risks":process_risks}).to_string();
                    }
                    let tree_evidence = facts.bindings.iter().filter(|binding|matches!(binding.demand.access.as_str(),"read_tree"|"read_instructions")
                        && binding.observation.requires_confirmation()).map(|binding|binding.evidence_id.as_str()).collect::<Vec<_>>();
                    if !tree_evidence.is_empty() {
                        // A tree can include symlinks to separately sensitive
                        // resources. An old scope must not follow a new target.
                        subject.resource = json!({"resolved_scope":subject.resource,"tree_evidence":tree_evidence}).to_string();
                    }
                }
                // No default prompt is not an exemption from configured rules.
                tool.permission_scope = subject.as_ref().map(|subject| subject.capability.clone());
                tool.permission_rationale = tool.permission_scope.as_ref()
                    .map(|domain| format!("{} requests {domain} access to {}", capability.title,
                        subject.as_ref().map(|subject| subject.resource.as_str()).unwrap_or("unspecified resource")));
                tool.permission_resource = subject.as_ref().map(|subject| subject.resource.clone());
                let permission_evaluation = subject.as_ref().map(|subject| {
                    permission_rules.evaluate_verified_resources_for_origin_with_explanation(subject, input_origin,
                        !crate::agent::generative_ui::is_local_publication(&capability)
                            && nola_kernel::permission_contract_requires_confirmation(
                            &capability.input_normalization.permission_contract.for_input(&tool.arguments), tool.permission_resolution.as_ref()),
                        resolved_subject.as_ref())
                });
                let rule_decision = permission_evaluation
                    .as_ref()
                    .map(|evaluation| evaluation.decision)
                    .unwrap_or(RuleDecision::Allow);
                if rule_decision == RuleDecision::Deny {
                    let output = Value::String("permission denied by policy".to_string());
                    tool.status = ToolCallStatus::Denied;
                    tool.output = Some(output.clone());
                    tool.is_error = true;
                    let tool_name = tool.name.clone();
                    let arguments = tool.arguments.clone();
                    let effect = capability.effect_for(&arguments);
                    turn.engineering
                        .observe_tool_result_for_call_with_resources(
                            &tool_name,
                            effect,
                            capability.mutation_requires_validation_for(&arguments),
                            true,
                            Some(engineering_failure_summary(&output)),
                            true,
                            Some(&arguments),
                            &capability.input_normalization.resource_projection,
                        );
                    events.push(RuntimeEvent::EngineeringLoopUpdated {
                        turn_id: turn_id.clone(),
                        state: turn.engineering.clone(),
                    });
                    terminal_results.push(AgentItem::ToolResult {
                        id: ItemId::from_string(allocate_id("tool_result")),
                        call_id: tool.call_id.clone(),
                        name: tool.name.clone(),
                        output: output.clone(),
                        is_error: true,
                        full_output_ref: None,
                        artifacts: Vec::new(),
                    });
                    events.push(RuntimeEvent::ToolCompleted {
                        turn_id: turn_id.clone(),
                        call_id: tool.call_id.clone(),
                        tool_name: tool.name.clone(),
                        output,
                        outcome: HostToolOutcome::Failed,
                        full_output_ref: None,
                        artifacts: Vec::new(),
                    });
                } else if rule_decision == RuleDecision::Ask {
                    let request_id = PermissionRequestId::from_string(allocate_id("permission"));
                    let scope = tool.permission_scope.clone().unwrap_or_default();
                    let rationale = tool.permission_rationale.clone().unwrap_or_default();
                    let evaluation = permission_evaluation.clone().ok_or_else(|| {
                        "permission request is missing its Core evaluation".to_string()
                    })?;
                    let resource = subject
                        .as_ref()
                        .map(|subject| subject.resource.clone())
                        .unwrap_or_default();
                    tool.permission_request_id = Some(request_id.clone());
                    tool.status = ToolCallStatus::AwaitingPermission;
                    turn.status = AgentTurnStatus::AwaitingPermission;
                    pending_permission_requests.push(
                        crate::agent::permissions::PendingPermissionRequest {
                            allow_conversation: resolved_subject.as_ref().is_none_or(|subject| subject.supports_reusable_approval()),
                            permission_instructions: Vec::new(),
                            request_id: request_id.clone(),
                            turn_id: turn_id.clone(),
                            call_id: tool.call_id.clone(),
                            capability_id: tool.name.clone(),
                            resource: resource.clone(),
                            arguments: tool.arguments.clone(),
                            invocation_context: tool.invocation_context.clone(),
                            permission_domain: scope.clone(),
                            rationale: rationale.clone(),
                            evaluation,
                            continuation:
                                crate::agent::permissions::PermissionContinuation::CoreTool,
                        },
                    );
                    events.push(RuntimeEvent::PermissionRequested {
                        turn_id: turn_id.clone(),
                        request_id: request_id.clone(),
                        call_id: tool.call_id.clone(),
                        tool_name: tool.name.clone(),
                        arguments: host_tool_arguments(
                            &tool.arguments,
                            tool.invocation_context.as_ref(),
                        ),
                        permission_domain: scope.clone(),
                        rationale: rationale.clone(),
                    });
                    effects.push(AgentEffect::RequestPermission {
                        allow_conversation: resolved_subject.as_ref().is_none_or(|subject| subject.supports_reusable_approval()),
                        permission_instructions: Vec::new(),
                        turn_id: turn_id.clone(),
                        request_id,
                        call_id: tool.call_id.clone(),
                        tool_name: tool.name.clone(),
                        arguments: host_tool_arguments(
                            &tool.arguments,
                            tool.invocation_context.as_ref(),
                        ),
                        resource,
                        permission_domain: scope,
                        rationale,
                        evaluation: permission_evaluation,
                    });
                } else {
                    if tool.name == "request_user_input" {
                        let question_set: nola_domain::user_input_wait::QuestionSet = serde_json::from_value(tool.arguments.clone())
                            .map_err(|error| format!("invalid user question schema: {error}"))?;
                        let wait = nola_domain::user_input_wait::UserInputWait {
                            wait_id: allocate_id("user-input-wait"), core_session_id: id_session.clone(),
                            turn_id: turn_id.as_str().to_owned(), originating_call_id: Some(tool.call_id.as_str().to_owned()),
                            revision: 1, created_at_revision: wait_revision,
                            response_schema_digest: question_set.digest()?, question_set,
                            state: nola_domain::user_input_wait::UserInputWaitState::Open,
                        };
                        tool.status = ToolCallStatus::AwaitingUserInput;
                        events.push(RuntimeEvent::UserInputWaitChanged { wait: wait.clone() });
                        opened_waits.push(wait);
                        continue;
                    }
                    tool.status = ToolCallStatus::Running;
                    events.push(RuntimeEvent::ToolStarted {
                        turn_id: turn_id.clone(),
                        call_id: tool.call_id.clone(),
                        tool_name: tool.name.clone(),
                        arguments: tool.arguments.clone(),
                    });
                    effects.push(AgentEffect::ExecuteTool {
                        permission_instructions: tool.permission_instructions.clone(),
                        turn_id: turn_id.clone(),
                        call_id: tool.call_id.clone(),
                        tool_name: tool.name.clone(),
                        arguments: host_tool_arguments(
                            &tool.arguments,
                            tool.invocation_context.as_ref(),
                        ),
                        grant: tool.grant.clone(),
                    });
                }
            }
            (
                turn.all_model_calls_returned(),
                turn.tools
                    .iter()
                    .any(|tool| tool.status == ToolCallStatus::AwaitingPermission),
                turn.tools
                    .iter()
                    .any(|tool| tool.status == ToolCallStatus::Running),
                turn.tools
                    .iter()
                    .any(|tool| tool.status == ToolCallStatus::Pending),
            )
        };
        for request in pending_permission_requests {
            self.session.execution.permissions.upsert(request);
        }
        for wait in opened_waits {
            self.session.execution.user_input_waits.insert(wait.wait_id.clone(), wait);
        }
        self.session.journal.next_id_sequence = next_id_sequence;
        self.session.record_items(terminal_results);
        if model_in_flight {
            if let Some(turn) = self.session.execution.active_turn.as_mut() {
                turn.status = AgentTurnStatus::AwaitingModel;
            }
        } else if all_terminal {
            self.request_model(turn_id, events, effects)?;
        } else if has_pending_permission {
            if let Some(turn) = self.session.execution.active_turn.as_mut() {
                turn.status = AgentTurnStatus::AwaitingPermission;
            }
        } else if has_running {
            if let Some(turn) = self.session.execution.active_turn.as_mut() {
                turn.status = AgentTurnStatus::AwaitingTools;
            }
        } else if has_pending && !batch.is_empty() {
            self.schedule_tools(turn_id, events, effects)?;
        } else if self.session.execution.user_input_waits.values().any(|wait| wait.is_open() && wait.turn_id == turn_id.as_str()) {
            if let Some(turn) = self.session.execution.active_turn.as_mut() { turn.status = AgentTurnStatus::AwaitingInput; }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn request_delegated_permission(
        &mut self,
        occurrence: nola_host_protocol::DelegatedPermissionOccurrence,
        turn_id: TurnId,
        target_effect_id: String,
        request_id: PermissionRequestId,
        call_id: CallId,
        capability_id: String,
        resource: String,
        reason: String,
        input: Value,
        events: &mut Vec<RuntimeEvent>,
        effects: &mut Vec<AgentEffect>,
    ) -> Result<(), String> {
        let turn = self.require_turn(&turn_id, AgentTurnStatus::AwaitingModel)?;
        let backend = turn.context.runtime_snapshot.execution_backend.clone();
        if backend.backend_id == "nola" {
            return Err("native model execution cannot request delegated permission".to_string());
        }
        let subject = crate::agent::permissions::PermissionSubject {
            capability: capability_id.clone(),
            resource: resource.clone(),
        };
        let evaluation = self
            .session
            .execution
            .permission_rules
            .evaluate_for_origin_with_explanation(&subject, turn.context.input_origin);
        match evaluation.decision {
            RuleDecision::Allow | RuleDecision::Deny => {
                effects.push(AgentEffect::ResolveDelegatedPermission {
                    occurrence: occurrence.clone(),
                    permission_instructions: Vec::new(),
                    turn_id,
                    target_effect_id,
                    backend_id: backend.backend_id,
                    backend_session_id: backend.backend_session_id,
                    permission_request_id: request_id,
                    resolution: if evaluation.decision == RuleDecision::Allow {
                        crate::agent::host::HostPermissionResolution::allow_once()
                    } else {
                        crate::agent::host::HostPermissionResolution::deny()
                    },
                });
            }
            RuleDecision::Ask => {
                let backend_permission_request_id = request_id;
                let request_id = PermissionRequestId::from_string(occurrence.occurrence_id.clone());
                let scope = capability_id.clone();
                self.session.execution.permissions.upsert(
                    crate::agent::permissions::PendingPermissionRequest {
                        allow_conversation: true,
                        permission_instructions: Vec::new(),
                        request_id: request_id.clone(),
                        turn_id: turn_id.clone(),
                        call_id: call_id.clone(),
                        capability_id: capability_id.clone(),
                        resource: resource.clone(),
                        arguments: input.clone(),
                        invocation_context: None,
                        permission_domain: scope.clone(),
                        rationale: reason.clone(),
                        evaluation: evaluation.clone(),
                        continuation:
                            crate::agent::permissions::PermissionContinuation::DelegatedAgent {
                                occurrence,
                                backend_permission_request_id: Some(backend_permission_request_id),
                                target_effect_id,
                                backend_id: backend.backend_id,
                                backend_session_id: backend.backend_session_id,
                            },
                    },
                );
                self.require_turn_mut(&turn_id, AgentTurnStatus::AwaitingModel)?
                    .status = AgentTurnStatus::AwaitingPermission;
                events.push(RuntimeEvent::PermissionRequested {
                    turn_id: turn_id.clone(),
                    request_id: request_id.clone(),
                    call_id: call_id.clone(),
                    tool_name: capability_id.clone(),
                    arguments: input.clone(),
                    permission_domain: scope.clone(),
                    rationale: reason.clone(),
                });
                effects.push(AgentEffect::RequestPermission {
                    allow_conversation: true,
                    permission_instructions: Vec::new(),
                    turn_id,
                    request_id,
                    call_id,
                    tool_name: capability_id,
                    arguments: input,
                    resource,
                    permission_domain: scope,
                    rationale: reason,
                    evaluation: Some(evaluation),
                });
            }
        }
        Ok(())
    }

    pub(super) fn resolve_permission(
        &mut self,
        request_id: PermissionRequestId,
        decision: PermissionUserDecision,
        grant: Option<Value>,
        events: &mut Vec<RuntimeEvent>,
        effects: &mut Vec<AgentEffect>,
    ) -> Result<(), String> {
        self.apply_permission_resolution(request_id, decision, grant,
            crate::agent::permissions::PermissionReceiptOutcome::Authorization { decision }, events, effects)
    }

    pub(super) fn apply_permission_resolution(
        &mut self,
        request_id: PermissionRequestId,
        decision: PermissionUserDecision,
        grant: Option<Value>,
        receipt_outcome: crate::agent::permissions::PermissionReceiptOutcome,
        events: &mut Vec<RuntimeEvent>,
        effects: &mut Vec<AgentEffect>,
    ) -> Result<(), String> {
        // Reject before closing the challenge or mutating any permission state.
        // Recompute from the frozen Host facts, not the displayed eligibility.
        if decision == PermissionUserDecision::AllowConversation {
            let pending = self.session.execution.permissions.pending.iter()
                .find(|pending| pending.request_id == request_id)
                .ok_or("permission request is not pending")?;
            if !pending.allow_conversation {
                return Err("this invocation has no valid reusable resource scope; choose allow once".into());
            }
            if let Some(tool) = self.session.execution.active_turn.as_ref()
                .and_then(|turn| turn.tools.iter().find(|tool| tool.permission_request_id.as_ref() == Some(&request_id))) {
                if let Some((request, facts)) = &tool.permission_resolution {
                    if !facts.subject_for(request).map_err(|error| error.to_string())?.supports_reusable_approval() {
                        return Err("this invocation has no valid reusable resource scope; choose allow once".into());
                    }
                }
            }
        }
        if let Some(pending) = self
            .session
            .execution
            .permissions
            .pending
            .iter()
            .find(|pending| pending.request_id == request_id)
            .cloned()
        {
            if matches!(
                pending.continuation,
                crate::agent::permissions::PermissionContinuation::DelegatedAgent { .. }
            ) {
                return self
                    .resolve_delegated_permission(pending, decision, grant, receipt_outcome, events, effects);
            }
        }
        let permission_call_id = self
            .session
            .execution
            .active_turn
            .as_ref()
            .and_then(|turn| {
                turn.tools
                    .iter()
                    .find(|tool| tool.permission_request_id.as_ref() == Some(&request_id))
            })
            .map(|tool| tool.call_id.clone())
            .ok_or_else(|| "permission request does not match an active tool".to_string())?;
        let permission_turn_id = self
            .session
            .execution
            .active_turn
            .as_ref()
            .map(|turn| turn.turn_id.as_str().to_string());
        self.session.host.pending_effects.complete_matching(
            permission_turn_id.as_deref(),
            Some("permission.ask"),
            Some(permission_call_id.as_str()),
        );
        let policy_applied = matches!(&receipt_outcome, crate::agent::permissions::PermissionReceiptOutcome::PolicyApplied { .. });
        self.session.execution.permissions.resolve(&request_id, receipt_outcome);
        let denied_result_id = matches!(&decision, PermissionUserDecision::Deny)
            .then(|| ItemId::from_string(self.session.allocate_id("tool_result")));
        let mut denied_result = None;
        let mut conversation_permission_rule = None;
        let permission_owner = self.session.execution.active_turn.as_ref()
            .ok_or("no active turn")?.turn_id.clone();
        let model_in_flight = self.model_request_is_in_flight(&permission_owner, effects);
        let turn = self
            .session
            .execution
            .active_turn
            .as_mut()
            .ok_or_else(|| "no active turn".to_string())?;
        let tool = turn
            .tools
            .iter_mut()
            .find(|tool| tool.permission_request_id.as_ref() == Some(&request_id))
            .ok_or_else(|| "permission request does not match an active tool".to_string())?;
        let user_denied_permission = matches!(&decision, PermissionUserDecision::Deny);
        let resolution_kind = if policy_applied { PermissionResolutionKind::PolicyApplied } else { match &decision {
            PermissionUserDecision::AllowOnce => PermissionResolutionKind::AllowOnce,
            PermissionUserDecision::AllowConversation => {
                PermissionResolutionKind::AllowConversation
            }
            PermissionUserDecision::Deny => PermissionResolutionKind::Deny,
        }};
        events.push(RuntimeEvent::PermissionResolved {
            turn_id: turn.turn_id.clone(),
            request_id: request_id.clone(),
            call_id: tool.call_id.clone(),
            decision: resolution_kind,
        });
        match decision {
            PermissionUserDecision::Deny => {
                turn.user_denied_permission |= user_denied_permission;
                let output = Value::String("Permission denied".to_string());
                tool.status = ToolCallStatus::Denied;
                tool.output = Some(output.clone());
                tool.is_error = true;
                denied_result = Some(AgentItem::ToolResult {
                    id: denied_result_id
                        .clone()
                        .ok_or_else(|| "permission denial result id is unavailable".to_string())?,
                    call_id: tool.call_id.clone(),
                    name: tool.name.clone(),
                    output: output.clone(),
                    is_error: true,
                    full_output_ref: None,
                    artifacts: Vec::new(),
                });
                events.push(RuntimeEvent::ToolCompleted {
                    turn_id: turn.turn_id.clone(),
                    call_id: tool.call_id.clone(),
                    tool_name: tool.name.clone(),
                    output,
                    outcome: HostToolOutcome::Failed,
                    full_output_ref: None,
                    artifacts: Vec::new(),
                });
            }
            PermissionUserDecision::AllowOnce | PermissionUserDecision::AllowConversation => {
                if matches!(decision, PermissionUserDecision::AllowConversation) {
                    let concrete_resource = tool.permission_resource.clone().unwrap_or_else(|| {
                        crate::agent::permissions::permission_resource_for_input(
                            &tool.name,
                            tool.permission_scope.as_deref(),
                            &tool.arguments,
                        )
                    });
                    let resource_pattern =
                        conversation_grant_resource_pattern(grant.as_ref(), concrete_resource);
                    conversation_permission_rule =
                        Some(crate::agent::permissions::conversation_permission_rule(
                            tool.permission_scope.as_deref().unwrap_or(&tool.name),
                            resource_pattern,
                        ));
                }
                tool.grant = grant.clone();
                tool.status = ToolCallStatus::Running;
                events.push(RuntimeEvent::ToolStarted {
                    turn_id: turn.turn_id.clone(),
                    call_id: tool.call_id.clone(),
                    tool_name: tool.name.clone(),
                    arguments: host_tool_arguments(
                        &tool.arguments,
                        tool.invocation_context.as_ref(),
                    ),
                });
                effects.push(AgentEffect::ExecuteTool {
                    permission_instructions: tool.permission_instructions.clone(),
                    turn_id: turn.turn_id.clone(),
                    call_id: tool.call_id.clone(),
                    tool_name: tool.name.clone(),
                    arguments: host_tool_arguments(
                        &tool.arguments,
                        tool.invocation_context.as_ref(),
                    ),
                    grant: grant.clone(),
                });
            }
        }
        let all_tools_terminal = turn.all_model_calls_returned();
        let awaiting_permission = turn
            .tools
            .iter()
            .any(|tool| tool.status == ToolCallStatus::AwaitingPermission);
        let turn_id = turn.turn_id.clone();
        if model_in_flight {
            turn.status = AgentTurnStatus::AwaitingModel;
        } else if !all_tools_terminal && awaiting_permission {
            turn.status = AgentTurnStatus::AwaitingPermission;
        } else if !all_tools_terminal {
            turn.status = AgentTurnStatus::AwaitingTools;
        }
        let mut permission_requests_resolved_by_conversation_rule = Vec::new();
        if let Some((capability_pattern, resource_pattern)) = conversation_permission_rule {
            self.session
                .execution
                .permission_rules
                .append_scope([PermissionRule {
                    capability_pattern,
                    resource_pattern,
                    decision: RuleDecision::Allow,
                    source: PermissionRuleSource::Conversation,
                }]);

            // A model response may contain several concurrently safe calls that
            // all enter AwaitingPermission before the user can answer the first
            // prompt. "Allow for this conversation" is a rule update, not an
            // approval for only the selected call. Re-evaluate every pending
            // call immediately so stale permission effects do not keep opening
            // dialogs after the rule already allows them.
            let permission_rules = self.session.execution.permission_rules.clone();
            let pending_turn = self
                .session
                .execution
                .active_turn
                .as_mut()
                .ok_or_else(|| "no active turn".to_string())?;
            for pending_tool in &mut pending_turn.tools {
                if pending_tool.status != ToolCallStatus::AwaitingPermission {
                    continue;
                }
                let Some(pending_request_id) = pending_tool.permission_request_id.clone() else {
                    continue;
                };
                let resource = pending_tool.permission_resource.clone().unwrap_or_else(|| {
                    crate::agent::permissions::permission_resource_for_input(
                        &pending_tool.name,
                        pending_tool.permission_scope.as_deref(),
                        &pending_tool.arguments,
                    )
                });
                let subject = crate::agent::permissions::PermissionSubject {
                    capability: pending_tool
                        .permission_scope
                        .clone()
                        .unwrap_or_else(|| pending_tool.name.clone()),
                    resource,
                };
                // Approving the selected call does not change the provenance
                // of its siblings. Automated work cannot acquire interactive
                // authority through this batch re-evaluation.
                let resolved = pending_tool.permission_resolution.as_ref().map(|(request,facts)|
                    facts.subject_for(request).map_err(|error|error.to_string())).transpose()?;
                if !pending_tool.permission_instructions.is_empty()
                    || permission_rules
                        .evaluate_verified_resources_for_origin_with_explanation(
                            &subject, pending_turn.context.input_origin, true, resolved.as_ref()).decision
                        != RuleDecision::Allow
                {
                    continue;
                }
                permission_requests_resolved_by_conversation_rule.push((
                    pending_request_id,
                    pending_tool.call_id.clone(),
                    pending_tool.name.clone(),
                    pending_tool.arguments.clone(),
                    pending_tool.invocation_context.clone(),
                    pending_tool.permission_instructions.clone(),
                ));
                pending_tool.permission_request_id = None;
                pending_tool.grant = grant.clone();
                pending_tool.status = ToolCallStatus::Running;
            }
            if !permission_requests_resolved_by_conversation_rule.is_empty() {
                pending_turn.status = AgentTurnStatus::AwaitingTools;
            }
        }
        for (
            pending_request_id,
            pending_call_id,
            pending_tool_name,
            pending_arguments,
            pending_invocation_context,
            pending_instructions,
        ) in &permission_requests_resolved_by_conversation_rule
        {
            self.session
                .execution
                .permissions
                .resolve(pending_request_id,
                    crate::agent::permissions::PermissionReceiptOutcome::ConversationRuleApplied {
                        source_request_id: request_id.as_str().into(),
                    });
            self.session.host.pending_effects.complete_matching(
                Some(turn_id.as_str()),
                Some("permission.ask"),
                Some(pending_call_id.as_str()),
            );
            events.push(RuntimeEvent::PermissionResolved {
                turn_id: turn_id.clone(),
                request_id: pending_request_id.clone(),
                call_id: pending_call_id.clone(),
                decision: PermissionResolutionKind::ConversationRuleApplied,
            });
            events.push(RuntimeEvent::ToolStarted {
                turn_id: turn_id.clone(),
                call_id: pending_call_id.clone(),
                tool_name: pending_tool_name.clone(),
                arguments: host_tool_arguments(
                    pending_arguments,
                    pending_invocation_context.as_ref(),
                ),
            });
            effects.push(AgentEffect::ExecuteTool {
                permission_instructions: pending_instructions.clone(),
                turn_id: turn_id.clone(),
                call_id: pending_call_id.clone(),
                tool_name: pending_tool_name.clone(),
                arguments: host_tool_arguments(
                    pending_arguments,
                    pending_invocation_context.as_ref(),
                ),
                grant: grant.clone(),
            });
        }
        if let Some(result) = denied_result {
            self.session.record_items([result]);
        }
        if all_tools_terminal {
            self.request_model(&turn_id, events, effects)?;
        }
        Ok(())
    }

    pub(super) fn intervene_permission(
        &mut self,
        request_id: PermissionRequestId,
        intervention: crate::agent::permissions::PermissionUserIntervention,
        events: &mut Vec<RuntimeEvent>,
        effects: &mut Vec<AgentEffect>,
    ) -> Result<(), String> {
        let crate::agent::permissions::PermissionUserIntervention::ProvideInstruction {
            instruction,
        } = intervention;
        let instruction = instruction.trim().to_string();
        if instruction.is_empty() {
            return Err("permission instruction must not be empty".to_string());
        }
        let mut pending = self
            .session
            .execution
            .permissions
            .pending
            .iter()
            .find(|pending| pending.request_id == request_id)
            .cloned()
            .ok_or_else(|| "permission request is no longer pending".to_string())?;
        nola_host_protocol::PermissionInstruction::validate_append(
            &pending.permission_instructions,
            request_id.as_str(),
            &instruction,
        )?;
        let turn = self
            .session
            .execution
            .active_turn
            .as_ref()
            .filter(|turn| turn.turn_id == pending.turn_id)
            .ok_or_else(|| "permission belongs to an inactive Turn".to_string())?;
        if matches!(
            pending.continuation,
            crate::agent::permissions::PermissionContinuation::CoreTool
        ) {
            let tool = turn
                .tools
                .iter()
                .find(|tool| {
                    tool.call_id == pending.call_id
                        && tool.permission_request_id.as_ref() == Some(&request_id)
                        && tool.status == ToolCallStatus::AwaitingPermission
                })
                .ok_or_else(|| "permission request does not match a suspended tool".to_string())?;
            if tool.arguments != pending.arguments || tool.name != pending.capability_id {
                return Err("permission continuation differs from its original invocation".into());
            }
        }
        pending
            .permission_instructions
            .push(nola_host_protocol::PermissionInstruction {
                permission_request_id: request_id.as_str().to_owned(),
                instruction,
            });
        if let crate::agent::permissions::PermissionContinuation::DelegatedAgent {
            backend_permission_request_id,
            ..
        } = &mut pending.continuation
        {
            if backend_permission_request_id.is_none() {
                *backend_permission_request_id = Some(request_id.clone());
            }
        }
        let next_request_id =
            PermissionRequestId::from_string(self.session.allocate_id("permission"));
        self.session.host.pending_effects.complete_matching(
            Some(pending.turn_id.as_str()),
            Some("permission.ask"),
            Some(pending.call_id.as_str()),
        );
        self.session.execution.permissions.resolve(&request_id,
            crate::agent::permissions::PermissionReceiptOutcome::GuidanceProvided);
        pending.request_id = next_request_id.clone();
        let turn = self
            .session
            .execution
            .active_turn
            .as_mut()
            .expect("validated active Turn");
        if matches!(
            pending.continuation,
            crate::agent::permissions::PermissionContinuation::CoreTool
        ) {
            let tool = turn
                .tools
                .iter_mut()
                .find(|tool| tool.call_id == pending.call_id)
                .expect("validated suspended Core tool");
            tool.permission_request_id = Some(next_request_id.clone());
            tool.permission_instructions = pending.permission_instructions.clone();
        }
        turn.status = AgentTurnStatus::AwaitingPermission;
        events.push(RuntimeEvent::PermissionResolved {
            turn_id: pending.turn_id.clone(),
            request_id,
            call_id: pending.call_id.clone(),
            decision: PermissionResolutionKind::ProvideInstruction,
        });
        let arguments =
            host_tool_arguments(&pending.arguments, pending.invocation_context.as_ref());
        events.push(RuntimeEvent::PermissionRequested {
            turn_id: pending.turn_id.clone(),
            request_id: next_request_id.clone(),
            call_id: pending.call_id.clone(),
            tool_name: pending.capability_id.clone(),
            arguments: arguments.clone(),
            permission_domain: pending.permission_domain.clone(),
            rationale: pending.rationale.clone(),
        });
        effects.push(AgentEffect::RequestPermission {
            allow_conversation: pending.allow_conversation,
            turn_id: pending.turn_id.clone(),
            request_id: next_request_id,
            call_id: pending.call_id.clone(),
            tool_name: pending.capability_id.clone(),
            arguments,
            resource: pending.resource.clone(),
            permission_domain: pending.permission_domain.clone(),
            rationale: pending.rationale.clone(),
            evaluation: Some(pending.evaluation.clone()),
            permission_instructions: pending.permission_instructions.clone(),
        });
        self.session.execution.permissions.upsert(pending);
        Ok(())
    }

    fn resolve_delegated_permission(
        &mut self,
        pending: crate::agent::permissions::PendingPermissionRequest,
        decision: PermissionUserDecision,
        grant: Option<Value>,
        receipt_outcome: crate::agent::permissions::PermissionReceiptOutcome,
        events: &mut Vec<RuntimeEvent>,
        effects: &mut Vec<AgentEffect>,
    ) -> Result<(), String> {
        let crate::agent::permissions::PermissionContinuation::DelegatedAgent {
            occurrence,
            backend_permission_request_id,
            target_effect_id,
            backend_id,
            backend_session_id,
        } = pending.continuation.clone()
        else {
            return Err("permission continuation is not delegated".to_string());
        };
        self.session.host.pending_effects.complete_matching(
            Some(pending.turn_id.as_str()),
            Some("permission.ask"),
            Some(pending.call_id.as_str()),
        );
        let policy_applied = matches!(&receipt_outcome, crate::agent::permissions::PermissionReceiptOutcome::PolicyApplied { .. });
        self.session
            .execution
            .permissions
            .resolve(&pending.request_id, receipt_outcome);
        let resolution_kind = if policy_applied { PermissionResolutionKind::PolicyApplied } else { match decision {
            PermissionUserDecision::AllowOnce => PermissionResolutionKind::AllowOnce,
            PermissionUserDecision::AllowConversation => {
                PermissionResolutionKind::AllowConversation
            }
            PermissionUserDecision::Deny => PermissionResolutionKind::Deny,
        }};
        if decision == PermissionUserDecision::AllowConversation {
            let resource_pattern =
                conversation_grant_resource_pattern(grant.as_ref(), pending.resource.clone());
            let (capability_pattern, resource_pattern) =
                crate::agent::permissions::conversation_permission_rule(
                    &pending.capability_id,
                    resource_pattern,
                );
            self.session
                .execution
                .permission_rules
                .append_scope([PermissionRule {
                    capability_pattern,
                    resource_pattern,
                    decision: RuleDecision::Allow,
                    source: PermissionRuleSource::Conversation,
                }]);
        }
        let turn = self
            .session
            .execution
            .active_turn
            .as_mut()
            .filter(|turn| turn.turn_id == pending.turn_id)
            .ok_or_else(|| "delegated permission belongs to an inactive Turn".to_string())?;
        if decision == PermissionUserDecision::Deny {
            turn.user_denied_permission = true;
        }
        turn.status = AgentTurnStatus::AwaitingModel;
        events.push(RuntimeEvent::PermissionResolved {
            turn_id: pending.turn_id.clone(),
            request_id: pending.request_id.clone(),
            call_id: pending.call_id,
            decision: resolution_kind,
        });
        effects.push(AgentEffect::ResolveDelegatedPermission {
            occurrence: occurrence.clone(),
            permission_instructions: pending.permission_instructions.clone(),
            turn_id: pending.turn_id,
            target_effect_id,
            backend_id,
            backend_session_id,
            permission_request_id: backend_permission_request_id.unwrap_or(pending.request_id),
            resolution: match decision {
                PermissionUserDecision::AllowOnce => {
                    crate::agent::host::HostPermissionResolution::allow_once()
                }
                PermissionUserDecision::AllowConversation => {
                    crate::agent::host::HostPermissionResolution::allow_conversation(grant)
                }
                PermissionUserDecision::Deny => {
                    crate::agent::host::HostPermissionResolution::deny()
                }
            },
        });
        Ok(())
    }

    pub(super) fn complete_tool(
        &mut self,
        turn_id: &TurnId,
        call_id: &CallId,
        evidence: Option<nola_domain::execution_evidence::ExecutionEvidenceObservation>,
        mut output: Value,
        outcome: HostToolOutcome,
        activation: Option<crate::agent::host::HostCapabilityActivation>,
        full_output_ref: Option<String>,
        artifacts: Vec<HostArtifact>,
        materialized_skill_capabilities: Option<Vec<String>>,
        unstarted_permission_failure: bool,
        events: &mut Vec<RuntimeEvent>,
        effects: &mut Vec<AgentEffect>,
    ) -> Result<(), String> {
        let is_error = outcome.is_error();
        validate_host_artifacts(&artifacts)?;
        if self.settle_yielded_code_execution(turn_id, call_id, outcome, &output, full_output_ref.as_deref())? {
            // No second ToolResult or model request for an already returned
            // exec call. Its independent wait call owns observation delivery.
            return Ok(());
        }
        let memory_usage = take_memory_usage_metadata(&mut output);
        let result_item_id = ItemId::from_string(self.session.allocate_id("tool_result"));
        let (capability_name, capability_arguments) = self
            .session
            .execution
            .active_turn
            .as_ref()
            .and_then(|turn| {
                turn.tools
                    .iter()
                    .find(|tool| &tool.call_id == call_id)
                    .map(|tool| (tool.name.clone(), tool.arguments.clone()))
            })
            .ok_or_else(|| "tool completion does not match an active call".to_string())?;
        if capability_name == "code_mode_wait" && !is_error {
            let request: nola_domain::code_mode::CodeCellWaitRequest = serde_json::from_value(capability_arguments.clone())
                .map_err(|error| format!("invalid code cell observation: {error}"))?;
            // The Host only advances/waits on computation. Core's committed
            // ledger is the sole authority for the returned phase and output.
            if self.session.execution.code_cell(&request.cell_id).is_some() {
                output = serde_json::to_value(self.code_cell_snapshot(
                    &self.session.host.catalog.host_id.0, &request.cell_id, request.after_output_sequence
                )?).map_err(|error| error.to_string())?;
            } else {
                if evidence.is_none() || request.terminate {
                    return Err("historical cell observation lacks admitted read evidence".into());
                }
                let view: nola_domain::code_mode::CodeCellView = serde_json::from_value(output.clone())
                    .map_err(|error| format!("invalid historical cell observation: {error}"))?;
                view.validate_state()?;
                let end = request.after_output_sequence.checked_add(view.output.len() as u64)
                    .ok_or("historical cell output cursor overflow")?;
                if view.cell_id != request.cell_id || view.next_output_sequence != end
                    || view.output.iter().enumerate().any(|(index, chunk)|
                        chunk.sequence != request.after_output_sequence + index as u64 + 1)
                    || view.output.len() > 64
                    || (view.output.len() > 1 && view.output.iter().map(|chunk| chunk.text.len())
                        .try_fold(0usize, |sum, len| sum.checked_add(len)).is_none_or(|sum| sum > 16 * 1024))
                    || (view.has_more_output && view.output.is_empty()) {
                    return Err("historical cell observation identity or cursor mismatch".into());
                }
                // Observation is a tool result only, never a restored live
                // cell or permission to control another Run's computation.
            }
            if evidence.is_some() && request.wait_ms > 0 && !request.terminate {
                // A committed Host-backed observation can restore polling;
                // cached output (even from a timed wait) cannot prove the
                // worker is reachable. Only authenticated Host evidence can.
                self.session.execution.active_turn.as_mut().ok_or("no active turn")?
                    .code_observation_failures.remove(&request.cell_id);
            }
        }
        let (
            capability_effect,
            mutation_needs_validation,
            resource_projection,
        ) = self
            .session
            .execution
            .active_turn
            .as_ref()
            .ok_or_else(|| "tool completion has no active turn".to_string())?
            .context
            .capability_projection
            .frozen_host_catalog()
            .resolve_available(&capability_name)
            .map(|capability| {
                (
                    capability.effect_for(&capability_arguments),
                    capability.mutation_requires_validation_for(&capability_arguments),
                    capability.input_normalization.resource_projection.clone(),
                )
            })
            .unwrap_or((
                crate::agent::host::HostCapabilityEffect::Execute,
                false,
                crate::agent::host::HostResourceProjectionDeclaration::default(),
            ));
        if let Some(observation) = &evidence {
            observation.validate()?;
            if observation.call_id != call_id.as_str()
                || observation.tool_name != capability_name
            {
                return Err("execution evidence differs from its completing tool".into());
            }
        }
        let capability_effect = if outcome == HostToolOutcome::Deferred {
            crate::agent::host::HostCapabilityEffect::Execute
        } else {
            capability_effect
        };
        let mutation_needs_validation = mutation_needs_validation
            && outcome != HostToolOutcome::Deferred;
        let workspace_observation = self.workspace_preparation_observation(call_id,&output);
        let model_output = self.tool_output_for_context_with_ref(
            &capability_name,
            &output,
            full_output_ref.as_deref(),
        );
        let memory_usage = memory_usage_visible_to_model(memory_usage, &model_output);
        if !is_error && capability_name == "memory_search" {
            for usage in memory_usage {
                let usage_id = format!(
                    "usage:{}:{}:{}:{}",
                    self.session.session_id.as_str(),
                    turn_id.as_str(),
                    call_id.as_str(),
                    usage.memory_id
                );
                self.session
                    .memory_runtime
                    .apply(MemoryRuntimeCommandEnvelope {
                        operation_id: usage_id.clone(),
                        command: MemoryRuntimeCommand::RecordUsage {
                            entry: MemoryUsageLedgerEntry {
                                usage_id,
                                memory_id: usage.memory_id,
                                session_id: self.session.session_id.as_str().to_string(),
                                turn_id: turn_id.as_str().to_string(),
                                used_at_epoch_ms: usage.used_at_epoch_ms,
                                context_payload_digest: usage.context_payload_digest,
                                citation_ids: usage.citation_ids,
                            },
                        },
                    })?;
            }
        }
        let execution_skipped = tool_execution_was_skipped(&output);
        let (name, engineering_update, all_terminal) = {
            let turn = self
                .session
                .execution
                .active_turn
                .as_mut()
                .ok_or_else(|| "no active turn".to_string())?;
            if &turn.turn_id != turn_id {
                return Err("turn id does not match the active turn".to_string());
            }
            let (name, arguments) = {
                let tool = turn
                    .tools
                    .iter_mut()
                    .find(|tool| &tool.call_id == call_id)
                    .ok_or_else(|| "tool completion does not match an active call".to_string())?;
                if tool.status != ToolCallStatus::Running
                    && !(unstarted_permission_failure && execution_skipped && is_error
                        && tool.status == ToolCallStatus::AwaitingPermission) {
                    return Err("tool is not running".to_string());
                }
                tool.output = Some(workspace_observation.clone().unwrap_or_else(||model_output.clone()));
                tool.is_error = is_error;
                tool.status = match outcome {
                    HostToolOutcome::Succeeded | HostToolOutcome::Deferred => ToolCallStatus::Completed,
                    HostToolOutcome::Failed => ToolCallStatus::Failed,
                    HostToolOutcome::Cancelled => ToolCallStatus::Cancelled,
                };
                (tool.name.clone(), tool.arguments.clone())
            };
            if name == "code_mode_exec" {
                if let Some(cell) = turn.code_cells.values_mut()
                    .find(|cell| cell.identity().parent_call_id == call_id.as_str()) {
                    use crate::agent::code_mode::CodeCellPhase;
                    let mut candidate = cell.clone();
                    let target = cell;
                    let cell = &mut candidate;
                    let owner = cell.identity().clone();
                    let unresolved = cell.pending_calls().next().is_some();
                    match outcome {
                        HostToolOutcome::Succeeded => cell.finish(&owner, CodeCellPhase::Completed)?,
                        HostToolOutcome::Failed if !unresolved => cell.finish(&owner, CodeCellPhase::Failed)?,
                        HostToolOutcome::Cancelled if !unresolved => {
                            cell.request_cancel(&owner)?;
                            cell.finish(&owner, CodeCellPhase::Cancelled)?;
                        },
                        HostToolOutcome::Failed | HostToolOutcome::Cancelled => {
                            // Preserve uncertain children for reconciliation;
                            // engine exit does not prove their effects stopped.
                            cell.finish(&owner, CodeCellPhase::Interrupted)?;
                        },
                        HostToolOutcome::Deferred if cell.has_yielded() => {
                            // Foreground delivery only. Keep the original cell
                            // and Host effect live for output/child callbacks.
                        },
                        HostToolOutcome::Deferred => return Err("code cell cannot complete through task promotion".into()),
                    }
                    if cell.is_terminal() { cell.record_completion_details(&output, full_output_ref.as_deref())?; }
                    *target = candidate;
                    // Fast completion and foreground yield expose the same
                    // Core-owned paged observation contract. Do not inject
                    // the Host's full output array as a second representation.
                    output = serde_json::to_value(target.view(&owner, 0)?)
                        .map_err(|error| error.to_string())?;
                }
            }
            let previous_engineering = turn.engineering.clone();
            let receipt = output.get("_nola_execution").cloned()
                .and_then(|value| serde_json::from_value::<nola_host_protocol::tool_result::ExecutionReceipt>(value).ok());
            if outcome == HostToolOutcome::Succeeded && !execution_skipped
                && !receipt.as_ref().is_some_and(crate::agent::turn::receipt_is_known_noop)
                && matches!(capability_effect, crate::agent::host::HostCapabilityEffect::Mutate
                    | crate::agent::host::HostCapabilityEffect::MutateAndValidate)
            {
                turn.successful_mutation_calls.insert(call_id.as_str().to_string());
            }
            turn.engineering
                .observe_tool_result_for_call_with_resources(
                    &name,
                    capability_effect,
                    mutation_needs_validation,
                    is_error,
                    is_error.then(|| engineering_failure_summary(&model_output)),
                    execution_skipped,
                    Some(&arguments),
                    &resource_projection,
                );
            if !execution_skipped && outcome != HostToolOutcome::Deferred {
            turn.engineering.reconcile_execution_receipt(&previous_engineering, receipt.as_ref(),
                capability_effect, mutation_needs_validation
                    && turn.engineering.scratch_mutation_count == previous_engineering.scratch_mutation_count,
                !is_error && !execution_skipped,
                &crate::agent::loop_guard::declared_effect_resources(&arguments, &resource_projection, false));
            }
            if outcome == HostToolOutcome::Succeeded
                && capability_effect == crate::agent::host::HostCapabilityEffect::Investigate
            {
                turn.engineering.observe_investigation_evidence_for_call(call_id.as_str(), &name, &arguments, &output);
            }
            if !is_error && name == "request_user_input" {
                // The answer starts a fresh loop-detection window so the
                // explicit "continue once" choice can actually retry the
                // suspended call. Durable evidence remains in model context.
                turn.reset_repeated_tool_batch_guard();
            }
            (
                name,
                (turn.engineering != previous_engineering).then(|| turn.engineering.clone()),
                turn.all_model_calls_returned(),
            )
        };
        let mut skill_engineering_update = None;
        if !is_error && matches!(name.as_str(), "Skill" | "skill_invoke") {
            if let Some(mut skill) = invoked_skill_from_output(&output) {
                skill.required_tools = materialized_skill_capabilities.unwrap_or_default();
                let requires_validation = !skill.validation_steps.is_empty();
                let activation_source = format!("skill:{}", skill.skill_id);
                let activated_capability_ids = self.session.record_invoked_skill(skill);
                if !activated_capability_ids.is_empty() {
                    events.push(RuntimeEvent::CapabilitiesActivated {
                        turn_id: turn_id.clone(),
                        source: activation_source,
                        capability_ids: activated_capability_ids,
                    });
                }
                if requires_validation {
                    if let Some(turn) = self.session.execution.active_turn.as_mut() {
                        let previous = turn.engineering.clone();
                        turn.engineering.require_skill_validation();
                        skill_engineering_update =
                            (turn.engineering != previous).then(|| turn.engineering.clone());
                    }
                }
            }
        }
        // A successful investigative capability is itself evidence that the
        // model selected and used this Turn-scoped schema. Keep that evidence
        // chain available across completion-reserve filtering. Mutating and
        // executing capabilities remain governed by completion policy so a
        // finished write cannot reopen another write round. Discovery/Skill
        // results already carry their own typed activation.
        if !is_error
            && activation.is_none()
            && capability_effect == crate::agent::host::HostCapabilityEffect::Investigate
            && !is_capability_activation_protocol(name.as_str())
        {
            if let Some(turn) = self.session.execution.active_turn.as_mut() {
                let activated_capability_ids =
                    turn.activate_capabilities("executed", std::iter::once(name.clone()));
                if !activated_capability_ids.is_empty() {
                    events.push(RuntimeEvent::CapabilitiesActivated {
                        turn_id: turn_id.clone(),
                        source: "executed".to_string(),
                        capability_ids: activated_capability_ids,
                    });
                }
            }
        }
        if let Some(activation) = activation_from_tool_result(activation, is_error) {
            self.install_capability_activation_descriptors(turn_id, &activation)?;
            if let Some(turn) = self.session.execution.active_turn.as_mut() {
                let source = activation.source;
                let activated_capability_ids =
                    turn.activate_capabilities(source.clone(), activation.capability_ids);
                if !activated_capability_ids.is_empty() {
                    events.push(RuntimeEvent::CapabilitiesActivated {
                        turn_id: turn_id.clone(),
                        source,
                        capability_ids: activated_capability_ids,
                    });
                }
            }
        }
        let model_result_item = AgentItem::ToolResult {
            id: result_item_id.clone(),
            call_id: call_id.clone(),
            name: name.clone(),
            output: model_output.clone(),
            is_error,
            full_output_ref: full_output_ref.clone(),
            // Model projection retains typed artifact identities. The Host
            // decides whether an explicitly capable model receives media
            // bytes; text-only models only see the bounded textual output.
            artifacts: artifacts.clone(),
        };
        let transcript_output = if full_output_ref.is_some() {
            model_output
        } else {
            output.clone()
        };
        let transcript_result_item = AgentItem::ToolResult {
            id: result_item_id,
            call_id: call_id.clone(),
            name: name.clone(),
            output: transcript_output,
            is_error,
            full_output_ref: full_output_ref.clone(),
            artifacts: artifacts.clone(),
        };
        self.session
            .record_item_views(turn_id, transcript_result_item, model_result_item)?;
        if let Some(evidence) = evidence {
            let goal_revision = self.session.goal_projection.revision;
            if let Some(goal) = self.session.goal_projection.active.as_mut().filter(|goal|
                !goal.status.is_terminal() && goal.work.source_turn_ids.contains(turn_id.as_str())) {
                let observation = crate::agent::goal::GoalExecutionObservation {
                    recorded_goal_revision: goal_revision.checked_add(1).ok_or("Goal revision exhausted")?,
                    turn_id: turn_id.as_str().to_owned(), source: evidence.source.clone(),
                    state: evidence.receipt.as_ref().map(|receipt| receipt.state),
                    checks: evidence.receipt.as_ref().map(|receipt| receipt.checks.clone()).unwrap_or_default(),
                    outcome: evidence.outcome.clone(),
                };
                if goal.execution_observations.contains_key(&evidence.evidence_id) {
                    return Err("Goal execution observation identity already exists".into());
                }
                goal.execution_observations.insert(evidence.evidence_id.clone(), observation);
                self.session.goal_projection.revision = goal_revision.checked_add(1)
                    .ok_or("Goal revision exhausted")?;
                events.push(RuntimeEvent::GoalUpdated {
                    goal_projection_revision: self.session.goal_projection.revision,
                    goal: self.session.goal_projection.clone(),
                });
            }
            self.record_requirement_evidence(turn_id, &evidence);
            events.push(RuntimeEvent::ExecutionEvidenceObserved {
                turn_id: turn_id.clone(),
                evidence,
            });
        }
        events.push(RuntimeEvent::ToolCompleted {
            turn_id: turn_id.clone(),
            call_id: call_id.clone(),
            tool_name: name,
            output,
            outcome,
            full_output_ref,
            artifacts,
        });
        if let Some(state) = skill_engineering_update.or(engineering_update) {
            events.push(RuntimeEvent::EngineeringLoopUpdated {
                turn_id: turn_id.clone(),
                state,
            });
        }
        if all_terminal {
            // A tool's Goal/Task state is not the owning Run's result. Let the
            // normal next response enter the same requirement arbiter as every
            // other tool batch; do not manufacture a model decision from output.
            self.request_model(turn_id, events, effects)?;
        } else {
            self.schedule_tools(turn_id, events, effects)?;
        }
        Ok(())
    }

    pub(super) fn tool_output_for_context_with_ref(
        &self,
        capability_id: &str,
        output: &Value,
        external_full_output_ref: Option<&str>,
    ) -> Value {
        let evidence = nola_domain::tool_evidence::ModelToolEvidence::from_host_output(output);
        let output = evidence.as_value();
        if matches!(capability_id, "Skill" | "skill_invoke") {
            if let Some(skill) = invoked_skill_from_output(output) {
                return compact_skill_activation_output(output, &skill.skill_id);
            }
        }
        let (preview, was_truncated) = self.tool_output_preview(capability_id, output);
        let limit = self.tool_output_preview_limit(capability_id);
        bounded_model_tool_output(
            output,
            preview,
            was_truncated,
            limit,
            external_full_output_ref,
        )
    }

    pub(super) fn tool_output_preview(
        &self,
        capability_id: &str,
        output: &Value,
    ) -> (String, bool) {
        let rendered = match output {
            Value::String(value) => value.clone(),
            // Structured command diagnostics take precedence over a display
            // label such as "exit=0". Exit status is not task validation.
            Value::Object(object) if object.contains_key("stdout") || object.contains_key("stderr") => {
                format!("stdout:\n{}\nstderr:\n{}",
                    object.get("stdout").and_then(Value::as_str).unwrap_or_default(),
                    object.get("stderr").and_then(Value::as_str).unwrap_or_default())
            }
            Value::Object(object) => object
                .get("text")
                .or_else(|| object.get("output_preview"))
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| {
                    serde_json::to_string_pretty(output).unwrap_or_else(|_| output.to_string())
                }),
            value => serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string()),
        };
        let limit = self.tool_output_preview_limit(capability_id);
        if rendered.chars().count() <= limit {
            return (rendered, false);
        }
        // Preserve the end as well: test summaries and final diagnostics
        // commonly follow long setup output. Do not infer success from text.
        let head = limit / 2;
        let tail: String = rendered.chars().rev().take(limit - head).collect::<Vec<_>>()
            .into_iter().rev().collect();
        let mut preview = format!("{}\n[… middle omitted …]\n{}",
            rendered.chars().take(head).collect::<String>(), tail);
        preview.push_str("\n[output truncated; open the full result for details]");
        (preview, true)
    }

    fn tool_output_preview_limit(&self, capability_id: &str) -> usize {
        self.session
            .host
            .catalog
            .capabilities
            .get(capability_id)
            .map(|capability| capability.output_policy.preview_char_limit)
            .unwrap_or(8_000)
            .max(1)
    }

    pub(super) fn tool_output_summary(&self, output: &Value) -> Option<String> {
        match output {
            Value::String(value) => (!value.trim().is_empty()).then(|| value.trim().to_string()),
            Value::Object(object) => object
                .get("display")
                .or_else(|| object.get("summary"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string),
            _ => None,
        }
    }
}

fn bounded_model_tool_output(
    output: &Value,
    preview: String,
    preview_was_truncated: bool,
    limit: usize,
    external_full_output_ref: Option<&str>,
) -> Value {
    let serialized_exceeds_limit = serde_json::to_string(output)
        .map(|encoded| encoded.chars().count() > limit)
        .unwrap_or(true);
    if !preview_was_truncated && !serialized_exceeds_limit {
        return output.clone();
    }
    let mut bounded = serde_json::json!({
        "output_preview": preview,
        "truncated": true,
    });
    // Keep bounded machine-readable facts outside the text excerpt. Cutting a
    // serialized receipt can otherwise remove its exit status, commit identity
    // or explicit non-completion flag while leaving a success-looking preview.
    let mut fact_budget = limit.min(2048) / 2;
    if let Some(fields) = output.as_object() {
        if let Some(region) = fields.get("structured_preview") {
            let cost = "structured_preview".len().saturating_add(region.to_string().chars().count());
            if cost <= fact_budget {
                bounded["structured_preview"] = region.clone();
                fact_budget -= cost;
            }
        }
        for (key, value) in fields {
            if matches!(key.as_str(), "output_preview" | "truncated" | "full_output_ref") {
                continue;
            }
            let scalar = value.is_boolean() || value.is_number() || value.is_null()
                || value.as_str().is_some_and(|text| text.chars().count() <= 256);
            let cost = key.len().saturating_add(value.to_string().len());
            if scalar && cost <= fact_budget {
                bounded[key] = value.clone();
                fact_budget -= cost;
            }
        }
    }
    if let Some(full_output_ref) = external_full_output_ref
        .or_else(|| output.get("full_output_ref").and_then(Value::as_str))
        .filter(|value| !value.trim().is_empty())
    {
        bounded["full_output_ref"] = Value::String(full_output_ref.to_string());
    }
    // Budget the complete serialized projection, not only its text. Escaping,
    // receipt facts and navigation references also occupy the model context.
    // Essential metadata survives even if a catalog declares a tiny limit.
    let text = bounded["output_preview"].as_str().unwrap_or_default().to_string();
    bounded["output_preview"] = Value::String(String::new());
    let overhead = bounded.to_string().chars().count();
    let available = limit.saturating_sub(overhead);
    let take = |chars: Vec<char>, budget: usize| -> String {
        let mut used = 0usize;
        chars.into_iter().take_while(|ch| {
            used += serde_json::to_string(&ch.to_string()).expect("JSON character")
                .chars().count().saturating_sub(2);
            used <= budget
        }).collect()
    };
    let excerpt = if serde_json::to_string(&text).expect("JSON text").chars().count().saturating_sub(2) <= available {
        text
    } else if available >= 32 {
        let marker = "\n[… omitted …]\n";
        let budget = available.saturating_sub(serde_json::to_string(marker).unwrap().chars().count() - 2);
        let head = take(text.chars().collect(), budget / 2);
        let tail = take(text.chars().rev().collect(), budget - budget / 2);
        format!("{head}{marker}{}", tail.chars().rev().collect::<String>())
    } else {
        take(text.chars().collect(), available)
    };
    bounded["output_preview"] = Value::String(excerpt);
    bounded
}

fn host_tool_arguments(
    arguments: &serde_json::Value,
    invocation_context: Option<&crate::agent::protocol::ToolInvocationContext>,
) -> serde_json::Value {
    let mut arguments = arguments.clone();
    if let (Some(object), Some(context)) = (arguments.as_object_mut(), invocation_context) {
        object.insert(
            crate::agent::tool_lifecycle::TOOL_INVOCATION_CONTEXT_ARGUMENT.to_string(),
            serde_json::to_value(context).unwrap_or(serde_json::Value::Null),
        );
    }
    arguments
}

fn compact_skill_activation_output(output: &Value, skill_id: &str) -> Value {
    let mut activated = serde_json::json!({
        "type": "skill_activated",
        "skill_id": skill_id,
        "message": "Instructions loaded, not task execution. The complete Skill instructions are active in this turn's system context. Now carry out the task using the exposed tools. Calling Skill again does not run the workflow. Use the execution context below directly; do not rediscover these paths."
    });
    for key in [
        "_nola_execution_context",
        "workspace_root",
        "base_directory",
    ] {
        if let Some(value) = output.get(key) {
            activated[key] = value.clone();
        }
    }
    activated
}

#[cfg(test)]
mod skill_activation_context_tests {
    use super::compact_skill_activation_output;
    use serde_json::json;

    #[test]
    fn compact_skill_result_preserves_authoritative_execution_paths() {
        let output = json!({
            "_nola_execution_context": {
                "workspace_root": "/tmp/workspace",
                "base_directory": "/app/skills/word"
            },
            "workspace_root": "/tmp/workspace",
            "base_directory": "/app/skills/word",
            "instructions": "very long instructions"
        });

        let compact = compact_skill_activation_output(&output, "word-document-authoring");

        assert_eq!(compact["skill_id"], "word-document-authoring");
        assert_eq!(
            compact["_nola_execution_context"]["workspace_root"],
            "/tmp/workspace"
        );
        assert_eq!(compact["workspace_root"], "/tmp/workspace");
        assert_eq!(compact["base_directory"], "/app/skills/word");
        assert!(compact.get("instructions").is_none());
    }
}

fn conversation_grant_resource_pattern(_grant: Option<&Value>, concrete_resource: String) -> String {
    // A Host grant may carry execution data, but is not an authority to
    // replace the pending Core subject. Wider reusable scopes must be selected
    // from Core-owned frozen offers, never reconstructed from response JSON.
    concrete_resource
}

struct ToolMemoryUsage {
    memory_id: String,
    used_at_epoch_ms: i64,
    context_payload_digest: String,
    citation_ids: Vec<String>,
}

/// A truncated tool preview is not sufficient evidence that every returned
/// memory reached the model. Until the bounded projection carries per-record
/// identities, fail closed instead of promoting unseen tail records through
/// usage-ranked recall.
fn memory_usage_visible_to_model(
    usage: Vec<ToolMemoryUsage>,
    model_output: &Value,
) -> Vec<ToolMemoryUsage> {
    if model_output
        .get("truncated")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        Vec::new()
    } else {
        usage
    }
}

fn take_memory_usage_metadata(output: &mut Value) -> Vec<ToolMemoryUsage> {
    let Some(metadata) = output
        .as_object_mut()
        .and_then(|object| object.remove("_nola_memory_usage"))
    else {
        return Vec::new();
    };
    metadata
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            Some(ToolMemoryUsage {
                memory_id: entry.get("memory_id")?.as_str()?.trim().to_string(),
                used_at_epoch_ms: entry.get("used_at_epoch_ms")?.as_i64()?,
                context_payload_digest: entry
                    .get("context_payload_digest")?
                    .as_str()?
                    .trim()
                    .to_string(),
                citation_ids: entry
                    .get("citation_ids")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect(),
            })
        })
        .filter(|entry| !entry.memory_id.is_empty() && !entry.context_payload_digest.is_empty())
        .collect()
}

#[cfg(test)]
mod memory_usage_visibility_contract_tests {
    use super::*;

    #[test]
    fn command_projection_preserves_diagnostics_not_just_success_label() {
        let runtime = AgentRuntime::try_new(AgentCoreConfig {
            session_id: Some(SessionId::from_string("output-projection-test".into())),
            turn_identity: crate::agent::model_capability::test_host_identity(),
            ..Default::default()
        }).unwrap();
        let output = json!({"text":"command exited 0", "exit_code":0,
            "stdout":format!("SETUP\n{}\nCONSTRAINT VIOLATIONS FOUND", "x".repeat(20_000)),
            "stderr":"diagnostic-tail"});
        let projected = runtime.tool_output_for_context_with_ref("bash", &output, Some("/output/full"));
        let preview = projected["output_preview"].as_str().unwrap();
        assert!(preview.contains("SETUP"));
        assert!(preview.contains("CONSTRAINT VIOLATIONS FOUND"));
        assert!(preview.contains("diagnostic-tail"));
        assert_eq!(projected["exit_code"], 0);
        assert_eq!(projected["full_output_ref"], "/output/full");
        assert!(projected.to_string().chars().count() <= 8_000);
    }

    fn usage(memory_id: &str) -> ToolMemoryUsage {
        ToolMemoryUsage {
            memory_id: memory_id.to_string(),
            used_at_epoch_ms: 1_785_600_000_123,
            context_payload_digest: "sha256:context".to_string(),
            citation_ids: Vec::new(),
        }
    }

    #[test]
    fn truncated_model_projection_cannot_promote_unverified_memory_usage() {
        let visible = memory_usage_visible_to_model(
            vec![usage("memory-1"), usage("memory-2")],
            &serde_json::json!({
                "output_preview": "only a bounded prefix",
                "truncated": true,
                "full_output_ref": "artifact://memory-search/full"
            }),
        );

        assert!(visible.is_empty());
    }

    #[test]
    fn complete_model_projection_preserves_typed_memory_usage() {
        let visible = memory_usage_visible_to_model(
            vec![usage("memory-1")],
            &serde_json::json!({"memories": [{"id": "memory-1"}]}),
        );

        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].memory_id, "memory-1");
    }
}

#[cfg(test)]
mod capability_activation_contract_tests {
    use super::*;

    #[test]
    fn failed_process_can_expose_host_preservation_recovery_but_not_failed_discovery() {
        for (source, accepted) in [
            ("host_resource_preservation", true),
            ("tool_search", false),
            ("skill", false),
            ("mcp_connection", false),
            ("code_memory", false),
            ("untrusted_output", false),
        ] {
            let activation = crate::agent::host::HostCapabilityActivation {
                source: source.into(),
                capability_ids: vec!["git_commit".into()],
                capability_descriptors: Vec::new(),
            };
            assert_eq!(activation_from_tool_result(Some(activation), true).is_some(), accepted, "{source}");
        }
    }

    #[test]
    fn typed_activation_is_normalized() {
        let activation = crate::agent::host::HostCapabilityActivation {
            source: "tool_search".to_string(),
            capability_ids: vec!["typed.tool".to_string(), "typed.tool".to_string()],
            capability_descriptors: Vec::new(),
        };
        let normalized = normalized_capability_activation(activation).unwrap();
        assert_eq!(normalized.source, "tool_search");
        assert_eq!(normalized.capability_ids, vec!["typed.tool"]);
    }

    #[test]
    fn code_memory_followups_require_successful_typed_activation() {
        let activation = crate::agent::host::HostCapabilityActivation {
            source: "code_memory".into(),
            capability_ids: vec!["codebase_search".into(), "codebase_read_symbol".into()],
            capability_descriptors: Vec::new(),
        };
        assert!(activation_from_tool_result(Some(activation.clone()), false).is_some());
        assert!(activation_from_tool_result(Some(activation), true).is_none());
        assert!(is_capability_activation_protocol("codebase_index"));
    }

    #[test]
    fn mcp_connection_activation_requires_success_and_is_discovery_protocol() {
        let activation = crate::agent::host::HostCapabilityActivation {
            source:"mcp_connection".into(),capability_ids:vec!["mcp.fixture.echo".into()],
            capability_descriptors:Vec::new(),
        };
        assert!(activation_from_tool_result(Some(activation.clone()),false).is_some());
        assert!(activation_from_tool_result(Some(activation),true).is_none());
        assert!(is_capability_activation_protocol("mcp_connect"));
        assert!(!is_capability_activation_protocol("mcp.fixture.echo"));
    }

    #[test]
    fn typed_full_output_reference_activates_navigation_capabilities() {
        let activation = crate::agent::host::HostCapabilityActivation {
            source: "full_output_ref".to_string(),
            capability_ids: vec![
                "tool_output_read".to_string(),
                "tool_output_search".to_string(),
            ],
            capability_descriptors: Vec::new(),
        };
        let normalized = normalized_capability_activation(activation).unwrap();
        assert_eq!(normalized.source, "full_output_ref");
        assert_eq!(
            normalized.capability_ids,
            vec!["tool_output_read", "tool_output_search"]
        );
    }
}

#[cfg(test)]
mod bounded_tool_context_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn serialized_projection_includes_escaped_preview_and_receipt_overhead() {
        let text = "\"\\\n中文".repeat(1000);
        let output = json!({"text":text, "exit_code":1, "success":false});
        let bounded = bounded_model_tool_output(
            &output, text, true, 1024, Some("tool-output://run/call"));
        assert!(bounded.to_string().chars().count() <= 1024);
        assert_eq!(bounded["exit_code"], 1);
        assert_eq!(bounded["success"], false);
        assert_eq!(bounded["full_output_ref"], "tool-output://run/call");
    }

    #[test]
    fn bounds_large_structured_payload_even_when_display_text_is_short() {
        let output = json!({
            "text": "42 matches",
            "rows": vec!["x".repeat(512); 64],
        });
        let bounded = bounded_model_tool_output(
            &output,
            "42 matches".to_string(),
            false,
            1_024,
            Some("tool-output://run/call"),
        );

        assert_eq!(bounded["output_preview"], "42 matches");
        assert_eq!(bounded["truncated"], true);
        assert_eq!(bounded["full_output_ref"], "tool-output://run/call");
        assert!(serde_json::to_string(&bounded).unwrap().len() < 256);
    }

    #[test]
    fn preserves_small_typed_tool_result() {
        let output = json!({"count": 2, "items": ["a", "b"]});
        assert_eq!(
            bounded_model_tool_output(&output, output.to_string(), false, 1_024, None,),
            output
        );
    }

    #[test]
    fn long_receipts_keep_noncompletion_and_observed_state_as_typed_facts() {
        let output = json!({"paths":vec!["long/path"; 4000],
            "commit":"abc123", "task_completion_attested":false,
            "workspace_clean_at_observation":false, "remaining_changed_file_count":1060,
            "exit_code":0});
        let bounded = bounded_model_tool_output(&output, "Committed selected files".into(), false, 8000, None);
        assert_eq!(bounded["task_completion_attested"], false);
        assert_eq!(bounded["workspace_clean_at_observation"], false);
        assert_eq!(bounded["remaining_changed_file_count"], 1060);
        assert_eq!(bounded["commit"], "abc123");
        assert_eq!(bounded["exit_code"], 0);
        assert!(bounded.get("paths").is_none());
    }
}

#[cfg(test)]
mod conversation_grant_contract_tests {
    use super::conversation_grant_resource_pattern;
    use serde_json::json;

    #[test]
    fn conversation_lifetime_preserves_the_concrete_tool_resource() {
        assert_eq!(
            conversation_grant_resource_pattern(
                Some(&json!({"scope": "conversation"})),
                "rm first.txt".to_string(),
            ),
            "rm first.txt"
        );
    }

    #[test]
    fn host_response_cannot_replace_the_pending_core_resource() {
        let original = json!({"source":"private/a", "destination":"public/b"}).to_string();
        for grant in [json!("*"), json!("/"), json!({"resource_pattern":"*"}),
            json!({"scope":"conversation", "resource_pattern":"/workspace"})] {
            assert_eq!(conversation_grant_resource_pattern(Some(&grant), original.clone()), original);
        }
    }
}

#[cfg(test)]
fn normalize_encoded_tool_arguments(arguments: &mut Value) -> bool {
    crate::agent::tool_lifecycle::normalize_encoded_tool_arguments(arguments)
}

fn likely_truncated_tool_arguments(arguments: &Value) -> bool {
    let Value::String(encoded) = arguments else {
        return false;
    };
    let trimmed = encoded.trim();
    if trimmed.len() < 4_096 {
        return false;
    }
    let missing_closer = (trimmed.starts_with('{') && !trimmed.ends_with('}'))
        || (trimmed.starts_with('[') && !trimmed.ends_with(']'));
    missing_closer && serde_json::from_str::<Value>(trimmed).is_err()
}

fn truncated_argument_recovery_action(tool_name: &str) -> String {
    format!("The {tool_name} argument was truncated; this call did not execute. Reduce the payload using a valid tool strategy while preserving the intended create, replace, or append semantics. Earlier calls may have executed: inspect current state before reconstructing a partial operation, then verify the final result.")
}

fn tool_execution_was_skipped(output: &Value) -> bool {
    let receipt = output.get("_nola_execution").cloned()
        .and_then(|value| serde_json::from_value(value).ok());
    execution_was_skipped(output, receipt.as_ref())
}

/// Only a receipt obtained through the caller's trusted normalization boundary
/// is authoritative. Business data must not impersonate execution evidence.
pub(super) fn execution_was_skipped(output: &Value,
    receipt: Option<&nola_host_protocol::tool_result::ExecutionReceipt>) -> bool {
    if let Some(receipt) = receipt {
        return receipt.state == nola_host_protocol::tool_result::ExecutionState::NotStarted;
    }
    let root = output
        .as_object()
        .and_then(|object| object.get("tool_execution_skipped"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let nested = output
        .get("result")
        .and_then(Value::as_object)
        .and_then(|object| object.get("tool_execution_skipped"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let bounded_text_marker = output
        .get("text")
        .and_then(Value::as_str)
        .is_some_and(|text| {
            text.lines()
                .take(256)
                .any(|line| line.trim() == "tool_execution_skipped: true")
        });
    root || nested || bounded_text_marker
}

#[cfg(test)]
fn repair_model_only_tool_metadata(arguments: &mut Value, validation_error: &str) -> bool {
    if normalize_encoded_tool_arguments(arguments) {
        return true;
    }
    validation_error.contains("unexpected field(s): description")
        && arguments
            .as_object_mut()
            .and_then(|object| object.remove("description"))
            .is_some()
}

/// Normalize only lossless scalar encodings described by the frozen tool
/// schema. Small and OpenAI-compatible models commonly serialize booleans and
/// numbers as JSON strings. Treating those transport encodings as a semantic
/// planning failure traps the Turn before the Host can execute an otherwise
/// valid call. Core still performs strict schema validation immediately after
/// this pass; arrays, objects, enum values, and ambiguous strings are untouched.
#[cfg(test)]
fn normalize_unambiguous_schema_scalars(schema: &Value, value: &mut Value) -> bool {
    crate::agent::tool_lifecycle::normalize_unambiguous_schema_scalars(schema, value)
}

#[cfg(test)]
mod model_argument_repair_tests {
    use super::{
        likely_truncated_tool_arguments, model_tool_not_exposed_output,
        normalize_encoded_tool_arguments, normalize_unambiguous_schema_scalars,
        repair_model_only_tool_metadata, tool_execution_was_skipped,
    };
    use serde_json::{json, Value};

    #[test]
    fn removes_model_description_only_after_schema_reports_it_as_unsupported() {
        let mut arguments = json!({
            "command": "cargo check",
            "description": "check the workspace"
        });
        assert!(repair_model_only_tool_metadata(
            &mut arguments,
            "$ contains unexpected field(s): description"
        ));
        assert_eq!(arguments, json!({"command": "cargo check"}));

        let mut supported = json!({"description": "real tool input"});
        assert!(!repair_model_only_tool_metadata(
            &mut supported,
            "$ is missing required field(s): prompt"
        ));
        assert_eq!(supported, json!({"description": "real tool input"}));
    }

    #[test]
    fn unsupported_operation_kind_is_not_lossless_metadata_to_discard() {
        let original = json!({
            "path": "transformer.cpp",
            "content": "// fixture",
            "operation_kind": "create"
        });
        let mut arguments = original.clone();
        assert!(!repair_model_only_tool_metadata(
            &mut arguments,
            "$ contains unexpected field(s): operation_kind"
        ));
        // Removing create intent could turn a rejected call into an overwrite.
        assert_eq!(arguments, original);
    }

    #[test]
    fn unwraps_model_arguments_encoded_as_a_json_string() {
        let mut arguments =
            Value::String(r#"{"path":"/tmp/result.txt","content":"ready"}"#.to_string());
        assert!(repair_model_only_tool_metadata(
            &mut arguments,
            "$ has JSON type string; expected object"
        ));
        assert_eq!(
            arguments,
            json!({"path": "/tmp/result.txt", "content": "ready"})
        );
    }

    #[test]
    fn unwraps_compatible_gateway_arguments_encoded_twice() {
        let encoded =
            serde_json::to_string(r#"{"path":"/tmp/result.txt","content":"ready"}"#).unwrap();
        let mut arguments = Value::String(encoded);

        assert!(normalize_encoded_tool_arguments(&mut arguments));
        assert_eq!(
            arguments,
            json!({"path": "/tmp/result.txt", "content": "ready"})
        );
    }

    #[test]
    fn normalizes_lossless_scalar_encodings_from_the_frozen_schema() {
        let schema = json!({
            "type": "object",
            "properties": {
                "query": {"type": "string"},
                "limit": {"type": "integer", "minimum": 1, "maximum": 20},
                "parse_pages": {"type": "boolean"},
                "threshold": {"type": "number"},
                "filters": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {"enabled": {"type": "boolean"}}
                    }
                }
            }
        });
        let mut arguments = json!({
            "query": "写诗 中南大学",
            "limit": "20",
            "parse_pages": "true",
            "threshold": "0.75",
            "filters": [{"enabled": "false"}]
        });

        assert!(normalize_unambiguous_schema_scalars(
            &schema,
            &mut arguments
        ));
        assert_eq!(arguments["query"], "写诗 中南大学");
        assert_eq!(arguments["limit"], 20);
        assert_eq!(arguments["parse_pages"], true);
        assert_eq!(arguments["threshold"], 0.75);
        assert_eq!(arguments["filters"][0]["enabled"], false);
    }

    #[test]
    fn leaves_ambiguous_or_structural_encodings_for_strict_validation() {
        let schema = json!({
            "type": "object",
            "properties": {
                "limit": {"type": "integer"},
                "parse_pages": {"type": "boolean"},
                "items": {"type": "array", "items": {"type": "string"}}
            }
        });
        let original = json!({
            "limit": "20.5",
            "parse_pages": "yes",
            "items": "[\"one\"]"
        });
        let mut arguments = original.clone();

        assert!(!normalize_unambiguous_schema_scalars(
            &schema,
            &mut arguments
        ));
        assert_eq!(arguments, original);
    }

    #[test]
    fn recognizes_large_unclosed_model_arguments_as_truncated() {
        let arguments = Value::String(format!(
            "{{\"path\":\"large.txt\",\"content\":\"{}",
            "x".repeat(8_000)
        ));
        assert!(likely_truncated_tool_arguments(&arguments));
        assert!(!likely_truncated_tool_arguments(&Value::String(
            r#"{"path":"small.txt""#.to_string()
        )));
        assert!(!likely_truncated_tool_arguments(&json!({
            "path": "large.txt",
            "content": "x".repeat(8_000)
        })));
    }

    #[test]
    fn truncated_argument_guidance_does_not_change_operation_semantics() {
        for name in ["write_file", "append_file", "bash", "custom_write"] {
            let guidance = super::truncated_argument_recovery_action(name);
            assert!(guidance.contains("this call did not execute"));
            assert!(guidance.contains("preserving the intended create, replace, or append semantics"));
            assert!(guidance.contains("Earlier calls may have executed"));
            assert!(!guidance.contains("create=true"));
            assert!(!guidance.contains("3,000"));
        }
    }

    #[test]
    fn recognizes_preflight_rejection_in_host_output_envelopes() {
        assert!(tool_execution_was_skipped(&json!({
            "display": "Rejected before execution",
            "text": "append chunk is too large\ntool_execution_skipped: true"
        })));
        assert!(tool_execution_was_skipped(&json!({
            "result": {"tool_execution_skipped": true}
        })));
        assert!(!tool_execution_was_skipped(&json!({
            "text": "Append failed after opening the file"
        })));
    }

    #[test]
    fn trusted_execution_state_wins_over_delegated_business_markers() {
        use nola_host_protocol::tool_result::{ExecutionReceipt, ExecutionState};
        let business = json!({"tool_execution_skipped":true,
            "_nola_execution":{"state":"not_started"}});
        let completed = ExecutionReceipt { state: ExecutionState::Completed, ..Default::default() };
        assert!(!super::execution_was_skipped(&business, Some(&completed)));
        let not_started = ExecutionReceipt { state: ExecutionState::NotStarted, ..Default::default() };
        assert!(super::execution_was_skipped(&json!({}), Some(&not_started)));
        // An untyped delegated payload cannot impersonate a trusted receipt.
        assert!(!super::execution_was_skipped(&json!({
            "_nola_execution":{"state":"not_started"}}), None));
        assert!(super::execution_was_skipped(&json!({
            "result":{"tool_execution_skipped":true}}), None));
    }

    #[test]
    fn scalar_normalization_does_not_choose_schema_alternatives() {
        for schema in [json!({"type":"integer","enum":[1,2]}),
            json!({"type":"integer","anyOf":[{"type":"string"},{"type":"integer"}]}),
            json!({"type":["string","integer"]})] {
            let mut value = json!("1");
            assert!(!normalize_unambiguous_schema_scalars(&schema, &mut value));
            assert_eq!(value, json!("1"));
        }
        let mut value = json!("9007199254740993");
        assert!(normalize_unambiguous_schema_scalars(&json!({"type":"number"}), &mut value));
        assert_eq!(value.as_u64(), Some(9007199254740993));
    }

    #[test]
    fn unexposed_tool_result_binds_recovery_to_frozen_catalog_revision() {
        let output = model_tool_not_exposed_output("remembered_tool", 41);
        assert_eq!(output["error"], "tool_not_exposed_in_model_step");
        assert_eq!(output["requested_tool"], "remembered_tool");
        assert_eq!(output["tool_execution_skipped"], true);
        assert_eq!(output["recovery"]["kind"], "discover_capability");
        assert_eq!(output["recovery"]["tool"], "tool_search");
        assert_eq!(output["recovery"]["arguments"]["query"], "select:remembered_tool");
        assert_eq!(output["recovery"]["catalog_revision"], 41);
    }
}
