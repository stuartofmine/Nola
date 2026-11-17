// SPDX-License-Identifier: Apache-2.0
//! Consolidated `agent_core_host_contract` module preserving shared fixtures and CI filter paths.

use crate::support::model_identity;
#[path = "../../support/requirement_result_contract_tests.rs"]
mod requirement_result_contract_tests;

// SPDX-License-Identifier: Apache-2.0

use std::collections::BTreeMap;

use nola_core::agent::host::{
    CapabilityAvailability, DelegatedAgentEventV1, HostArtifact, HostCancellationMode,
    HostCapabilityCatalog, HostCapabilityDescriptor, HostCapabilityExposure,
    HostCapabilityParallelism, HostCapabilityRoute, HostEffectId, HostError, HostId, HostLease,
    HostOutputPolicy, HostReplaySafety, HostRequest, HostRequestEnvelope, HostResponse,
    HostResponseEnvelope, ModelDeltaChannel, ModelResponseMode, ToolProgressChannel,
};

use nola_core::agent::parts::{AgentPart, ConversationProjection, ToolPartState};

use nola_core::agent::protocol::ModelToolCall;

use nola_core::agent::recovery::{CheckpointReason, CoreCheckpoint};

use nola_core::agent::{
    AgentCore, AgentCoreConfig, AgentEngineeringPhase, AgentEvent, AgentInputDelivery, AgentItem,
    AgentItemOwnership, AgentRole, AgentSubmission, AgentThreadState, AgentTurnArtifactContract,
    CallId, ContextComponent, ContextPlatform, ContextWindowConfig, ItemId, PermissionUserDecision,
    SessionId, TokenUsage, TURN_ARTIFACT_CONTRACT_COMPONENT_ID,
};

use nola_core::context_engine::ContextComponentKind;

use serde_json::{json, Value};

// These protocol fixtures admit a non-Git workspace, never a real executor or
// an HTTP credential. Keep the same admission boundary as Host composition.
fn admit_external_fixture_workspace(config: &mut AgentCoreConfig) {
    use nola_host_protocol::delegation::{
        DelegatedModel, DelegatedRuntimeConfiguration, DelegationParentSnapshot,
        ValidatedDelegationParent,
    };
    config.turn_identity.delegation = Some(
        ValidatedDelegationParent::new(DelegationParentSnapshot {
            process_sandbox: None,
            bounded_file_tools: Default::default(),
            project_repository_identity: None,
            task_worktree: None,
            task_sandbox_requirement: None,
            authorized_workspace_roots: vec!["/fixture".into()],
            requested_native_sandbox: None,
            model: DelegatedModel {
                endpoint: String::new(),
                model: "fixture-model".into(),
                swe_mode: false,
            },
            runtime: DelegatedRuntimeConfiguration::default(),
            credential: None,
            fallback_credentials: vec![],
            profiles: nola_domain::delegation::effective_agent_profiles(&Default::default()),
            project_root: "/fixture".into(),
            read_only_tools: Default::default(),
            profile_credentials: Default::default(),
            execution_backend: config.turn_identity.execution_backend.backend_id.clone(),
            external_execution_model: None,
            external_execution_model_provider: None,
            max_iterations: config.max_model_iterations,
        })
        .unwrap(),
    );
}

// Explicit simulated Host observations, never inferred from model arguments.
fn observed_output(data: Value, effects: &[(&str, &str)], checks: &[(&str, &str)]) -> Value {
    json!({
        "contract": "nola.tool_result.v1", "data": data,
        "presentation": {"text": data.to_string(), "summary": "Host result"},
        "execution": {"state": "completed", "local_stop":null,
            "effects": effects.iter().map(|(uri, version)| json!({
                "resource_uri": uri, "operation": "write", "before_version": null, "after_version": version
            })).collect::<Vec<_>>(),
            "checks": checks.iter().map(|(uri, version)| json!({
                "kind":"postcondition", "resource_uri": uri, "version": version, "predicate": "fixture_check", "passed": true
            })).collect::<Vec<_>>()},
        "full_output_ref": null, "memory_usage": null
    })
}

fn capability(
    id: &str,
    route: HostCapabilityRoute,
    permission_domain: Option<&str>,
) -> HostCapabilityDescriptor {
    let mut input_normalization = if id == "write_file" {
        nola_core::agent::host::HostInputNormalization {
            permission_resource:
                nola_core::agent::host::HostPermissionResourceDeclaration::FirstOf {
                    keys: vec!["path".to_string()],
                },
            ..Default::default()
        }
    } else {
        Default::default()
    };
    match id {
        "write_file" => {
            // This simulated writer only creates fixture outputs. Permission
            // scenarios declare a domain and retain the conservative contract.
            if permission_domain.is_none() {
                input_normalization.permission_contract.risk = nola_domain::permission_resource_plan::PermissionOperationRisk::NewFile;
            }
            input_normalization.effect =
                nola_core::agent::host::HostCapabilityEffectDeclaration::fixed(
                    nola_core::agent::host::HostCapabilityEffect::Mutate,
                );
            input_normalization.requires_validation_after_mutation = true;
            input_normalization.resource_projection =
                nola_core::agent::host::HostResourceProjectionDeclaration {
                    resource_keys: vec!["path".to_string()],
                    ephemeral_when_true_keys: vec!["ephemeral".to_string(), "dry_run".to_string()],
                    ..Default::default()
                };
        }
        "bash" => {
            input_normalization.effect =
                nola_core::agent::host::HostCapabilityEffectDeclaration::by_argument(
                    "operation_kind",
                    nola_core::agent::host::HostCapabilityEffect::Execute,
                    [
                        (
                            "inspect".to_string(),
                            nola_core::agent::host::HostCapabilityEffect::Investigate,
                        ),
                        (
                            "mutate".to_string(),
                            nola_core::agent::host::HostCapabilityEffect::Mutate,
                        ),
                        (
                            "validate".to_string(),
                            nola_core::agent::host::HostCapabilityEffect::Validate,
                        ),
                        (
                            "mutate_and_validate".to_string(),
                            nola_core::agent::host::HostCapabilityEffect::MutateAndValidate,
                        ),
                    ],
                );
            input_normalization.requires_validation_after_mutation = true;
            input_normalization.resource_projection =
                nola_core::agent::host::HostResourceProjectionDeclaration {
                    preferred_scope_keys: vec![
                        "validation_targets".to_string(),
                        "output_artifacts".to_string(),
                    ],
                    resource_keys: vec!["path".into()],
                    mutation_scope_keys: vec!["output_artifacts".into()],
                    validation_scope_keys: vec!["validation_targets".into()],
                    base_key: Some("cwd".to_string()),
                    ..Default::default()
                };
        }
        "read_file" | "web_search" | "web_fetch" => {
            input_normalization.permission_contract.risk = nola_domain::permission_resource_plan::PermissionOperationRisk::OrdinaryRead;
            input_normalization.effect =
                nola_core::agent::host::HostCapabilityEffectDeclaration::fixed(
                    nola_core::agent::host::HostCapabilityEffect::Investigate,
                );
        }
        "tool_search" | "Skill" | "skill_update" | "request_user_input" | "goal_complete" => {
            // Deterministic in-memory control fixtures cannot authorize any
            // external operation; actual tools keep their own trusted risks.
            input_normalization.permission_contract.risk = nola_domain::permission_resource_plan::PermissionOperationRisk::OrdinaryRead;
        }
        _ => {}
    }
    HostCapabilityDescriptor {
        id: id.to_string(),
        title: id.to_string(),
        route,
        availability: CapabilityAvailability::Available,
        exposure: HostCapabilityExposure::Visible,
        permission_domain: permission_domain.map(str::to_string),
        input_schema: if id == "write_file" {
            json!({
                "type": "object",
                "required": ["path", "content"],
                "properties": {
                    "path": {"type": "string", "minLength": 1},
                    "content": {"type": "string"}
                }
            })
        } else {
            json!({"type": "object"})
        },
        input_normalization,
        output_policy: HostOutputPolicy {
            preview_char_limit: 4_000,
            persist_full_output: true,
            allow_artifacts: true,
        },
        supports_streaming: id == "model.stream",
        supports_argument_streaming: false,
        parallelism: HostCapabilityParallelism::Serial,
        cancellation: HostCancellationMode::Cooperative,
        replay_safety: HostReplaySafety::Recoverable,
    }
}

fn host_catalog() -> HostCapabilityCatalog {
    let mut capabilities = BTreeMap::new();
    for (id, route, permission) in [
        ("model.stream", HostCapabilityRoute::Model, None),
        ("context.compact", HostCapabilityRoute::Compaction, None),
        ("permission.ask", HostCapabilityRoute::Permission, None),
        ("host.cancel", HostCapabilityRoute::Tool, None),
        (
            "write_file",
            HostCapabilityRoute::Tool,
            Some("workspace.write"),
        ),
    ] {
        capabilities.insert(id.to_string(), capability(id, route, permission));
    }
    HostCapabilityCatalog {
        host_id: HostId("test-host".to_string()),
        revision: 7,
        availability_epoch: 3,
        lease: Some(HostLease {
            lease_id: "lease-1".to_string(),
            issued_at_ms: 1,
            last_heartbeat_ms: 1,
            expires_at_ms: u64::MAX,
        }),
        capabilities,
    }
}

fn core_config(initial_items: Vec<AgentItem>) -> AgentCoreConfig {
    AgentCoreConfig {
        turn_identity: model_identity::host_identity(),
        session_id: Some(SessionId::from_string("session-contract".to_string())),
        model: Some("test-model".to_string()),
        initial_items,
        tools: vec![json!({
            "type": "function",
            "function": {
                "name": "write_file",
                "description": "write a file",
                "parameters": {"type": "object"}
            }
        })],
        host_catalog: Some(host_catalog()),
        ..AgentCoreConfig::default()
    }
}

fn engineering_core_config() -> AgentCoreConfig {
    let mut config = core_config(Vec::new());
    // Engineering-loop fixtures carry explicit authority for their simulated
    // process. Opaque process execution no longer gains a read exemption from
    // permission_domain=None or operation_kind supplied by the model.
    config.permission_rules.rules.push(nola_core::agent::permissions::PermissionRule {
        capability_pattern:"bash".into(),resource_pattern:"*".into(),
        decision:nola_core::agent::permissions::PermissionPolicyDecision::Allow,
        source:nola_core::agent::permissions::PermissionRuleSource::UserGlobal,
    });
    let catalog = config.host_catalog.as_mut().expect("host catalog");
    catalog.capabilities.insert(
        "bash".to_string(),
        capability("bash", HostCapabilityRoute::Tool, None),
    );
    config.tools.push(json!({
        "type": "function",
        "function": {
            "name": "bash",
            "description": "run a command",
            "parameters": {
                "type": "object",
                "required": ["command", "operation_kind"],
                "properties": {
                    "command": {"type": "string"},
                    "operation_kind": {
                        "type": "string",
                        "enum": ["inspect", "mutate", "validate", "mutate_and_validate", "execute"]
                    }
                }
            }
        }
    }));
    config
}

fn source_core_config() -> AgentCoreConfig {
    let mut config = core_config(Vec::new());
    let catalog = config.host_catalog.as_mut().expect("host catalog");
    for name in ["web_search", "read_file"] {
        catalog.capabilities.insert(
            name.to_string(),
            capability(name, HostCapabilityRoute::Tool, None),
        );
        config.tools.push(json!({
            "type": "function",
            "function": {
                "name": name,
                "description": name,
                "parameters": {"type": "object"}
            }
        }));
    }
    config
}

fn capability_activation_core_config() -> AgentCoreConfig {
    let mut config = core_config(Vec::new());
    let catalog = config.host_catalog.as_mut().expect("host catalog");
    catalog.capabilities.insert(
        "tool_search".to_string(),
        capability("tool_search", HostCapabilityRoute::Tool, None),
    );
    for capability_id in ["web_search", "web_fetch", "obsolete_only"] {
        let mut descriptor = capability(capability_id, HostCapabilityRoute::Tool, None);
        descriptor.exposure = HostCapabilityExposure::Deferred;
        catalog
            .capabilities
            .insert(capability_id.to_string(), descriptor);
    }
    config.tools.push(json!({
        "type": "function",
        "function": {
            "name": "tool_search",
            "description": "discover deferred host capabilities",
            "parameters": {"type": "object"}
        }
    }));
    config
}

fn capability_activation_transition(response: HostResponse) -> nola_core::agent::AgentTransition {
    let (mut core, tool) = pending_capability_activation();
    submit_response(&mut core, &tool, 1, response)
}

fn pending_capability_activation() -> (AgentCore, HostRequestEnvelope) {
    let mut core =
        AgentCore::try_new(capability_activation_core_config()).expect("create activation Core");
    let started = core
        .submit(user_input(
            "activation-user",
            "Use the capability selected by the model.",
            AgentInputDelivery::CurrentTurn,
        ))
        .expect("start activation turn");
    let model = model_request(&started.host_requests);
    let requested = submit_response(
        &mut core,
        &model,
        1,
        empty_model_completion(vec![named_tool_call(
            "activation-call",
            "tool_search",
            json!({"query": "current primary sources"}),
        )]),
    );
    let tool = request_matching(
        &requested.host_requests,
        |payload| matches!(payload, HostRequest::ExecuteTool { call_id, .. } if call_id == "activation-call"),
    );
    (core, tool)
}

fn terminal_host_response(
    request: &HostRequestEnvelope,
    payload: HostResponse,
) -> HostResponseEnvelope {
    HostResponseEnvelope {
        delegated_invocation_id: request.delegated_invocation_id.clone(),
        attempt_id: None,
        request_id: request.request_id.clone(),
        effect_id: request.effect_id.clone(),
        host_id: request.host_id.clone(),
        lease_id: request.lease_id.clone(),
        session_id: request.session_id.clone(),
        turn_id: request.turn_id.clone(),
        catalog_revision: request.catalog_revision,
        availability_epoch: request.availability_epoch,
        delivery: request.delivery.clone(),
        sequence: request.resume_after_sequence.unwrap_or(0).saturating_add(1),
        payload,
    }
}

fn user_input(id: &str, text: &str, delivery: AgentInputDelivery) -> AgentSubmission {
    AgentSubmission::UserInput {
        items: vec![AgentItem::user_message(
            ItemId::from_string(id.to_string()),
            text,
        )],
        turn_context_items: Vec::new(),
        turn_context_platform: ContextPlatform::Unknown,
        turn_context_components: Vec::new(),
        submission_id: Some(format!("submission-{id}")),
        submitted_at_epoch_ms: None,
        delivery,
    }
}

fn request_matching(
    requests: &[HostRequestEnvelope],
    predicate: impl Fn(&HostRequest) -> bool,
) -> HostRequestEnvelope {
    requests
        .iter()
        .find(|request| predicate(&request.payload))
        .cloned()
        .expect("expected Host request")
}

fn model_tool_names(tools: &[Value]) -> Vec<&str> {
    tools.iter().map(|schema| schema.pointer("/function/name").and_then(Value::as_str)
        .expect("every exposed model tool has a declared function name")).collect()
}

fn response_for(
    request: &HostRequestEnvelope,
    sequence: u64,
    payload: HostResponse,
) -> HostResponseEnvelope {
    HostResponseEnvelope {
        delegated_invocation_id: request.delegated_invocation_id.clone(),
        attempt_id: None,
        request_id: request.request_id.clone(),
        effect_id: request.effect_id.clone(),
        host_id: request.host_id.clone(),
        lease_id: request.lease_id.clone(),
        session_id: request.session_id.clone(),
        turn_id: request.turn_id.clone(),
        catalog_revision: request.catalog_revision,
        availability_epoch: request.availability_epoch,
        delivery: request.delivery.clone(),
        sequence,
        payload,
    }
}

fn submit_response(
    core: &mut AgentCore,
    request: &HostRequestEnvelope,
    sequence: u64,
    payload: HostResponse,
) -> nola_core::agent::AgentTransition {
    core.submit(AgentSubmission::HostResponse {
        response: response_for(request, sequence, payload),
    })
    .expect("Host response should be accepted")
}

fn confirm_cancellation(
    core: &mut AgentCore,
    interrupted: &nola_core::agent::AgentTransition,
    target_effect_id: &HostEffectId,
) -> nola_core::agent::AgentTransition {
    let cancel = request_matching(&interrupted.host_requests, |payload| {
        matches!(
            payload,
            HostRequest::Cancel { target_effect_id: target, .. }
                if target == target_effect_id
        )
    });
    submit_response(
        core,
        &cancel,
        1,
        HostResponse::CancellationConfirmed {
            target_effect_id: target_effect_id.clone(),
        },
    )
}

fn checkpoint_after(
    core: &AgentCore,
    transition: &nola_core::agent::AgentTransition,
) -> CoreCheckpoint<AgentThreadState> {
    let reason = transition
        .durability
        .as_ref()
        .map(|intent| intent.checkpoint_reason)
        .unwrap_or(CheckpointReason::Manual);
    core.checkpoint(reason)
}

fn model_request(requests: &[HostRequestEnvelope]) -> HostRequestEnvelope {
    request_matching(requests, |payload| {
        matches!(payload, HostRequest::ModelStream { .. })
    })
}

fn empty_model_completion(tool_calls: Vec<ModelToolCall>) -> HostResponse {
    HostResponse::ModelCompleted {
        items: Vec::new(),
        tool_calls: tool_calls
            .into_iter()
            .map(|call| serde_json::to_value(call).expect("serialize tool call"))
            .collect(),
        usage: serde_json::to_value(TokenUsage {
            input_tokens: Some(8),
            output_tokens: Some(3),
            total_tokens: Some(11),
            ..TokenUsage::default()
        })
        .expect("serialize usage"),
    }
}

fn model_requested_review(call_id: &str, candidate_answer: &str) -> HostResponse {
    empty_model_completion(vec![named_tool_call(
        call_id,
        "request_completion_review",
        json!({"candidate_answer": candidate_answer}),
    )])
}

fn visible_model_completion(item_id: &str, content: &str) -> HostResponse {
    HostResponse::ModelCompleted {
        items: vec![serde_json::to_value(AgentItem::assistant_message(
            ItemId::from_string(item_id.to_string()),
            content,
        ))
        .expect("serialize assistant message")],
        tool_calls: Vec::new(),
        usage: serde_json::to_value(TokenUsage {
            input_tokens: Some(8),
            output_tokens: Some(3),
            total_tokens: Some(11),
            ..TokenUsage::default()
        })
        .expect("serialize usage"),
    }
}

fn limited_model_completion(item_id: &str, content: &str) -> HostResponse {
    HostResponse::ModelOutputLimited {
        items: vec![serde_json::to_value(AgentItem::assistant_message(
            ItemId::from_string(item_id.to_string()),
            content,
        ))
        .expect("serialize limited assistant message")],
        tool_calls: Vec::new(),
        usage: serde_json::to_value(TokenUsage {
            input_tokens: Some(8),
            output_tokens: Some(16_384),
            total_tokens: Some(16_392),
            ..TokenUsage::default()
        })
        .expect("serialize limited usage"),
    }
}

fn visible_model_completion_with_tools(
    item_id: &str,
    content: &str,
    tool_calls: Vec<ModelToolCall>,
) -> HostResponse {
    HostResponse::ModelCompleted {
        items: vec![serde_json::to_value(AgentItem::assistant_message(
            ItemId::from_string(item_id.to_string()),
            content,
        ))
        .expect("serialize assistant message")],
        tool_calls: tool_calls
            .into_iter()
            .map(|call| serde_json::to_value(call).expect("serialize tool call"))
            .collect(),
        usage: serde_json::to_value(TokenUsage {
            input_tokens: Some(8),
            output_tokens: Some(3),
            total_tokens: Some(11),
            ..TokenUsage::default()
        })
        .expect("serialize usage"),
    }
}

fn complete_tool(
    core: &mut AgentCore,
    request: &HostRequestEnvelope,
    call_id: &str,
) -> nola_core::agent::AgentTransition {
    submit_response(
        core,
        request,
        1,
        HostResponse::ToolCompleted {
            call_id: call_id.to_string(),
            output: json!({"ok": true}),
            outcome: nola_core::agent::host::HostToolOutcome::Succeeded,
            activation: None,
            full_output_ref: None,
            artifacts: Vec::new(),
        },
    )
}

fn reach_iteration_budget_synthesis(config: &AgentCoreConfig) -> (AgentCore, HostRequestEnvelope) {
    let mut core = AgentCore::try_new(config.clone()).expect("create bounded Core");
    let started = core
        .submit(user_input(
            "bounded-user",
            "perform a multi-step operation",
            AgentInputDelivery::CurrentTurn,
        ))
        .expect("start bounded turn");
    let mut model = model_request(&started.host_requests);

    for (index, destination) in ["/workspace/first", "/workspace/second"]
        .into_iter()
        .enumerate()
    {
        let call_id = format!("bounded-call-{index}");
        let planned = submit_response(
            &mut core,
            &model,
            1,
            empty_model_completion(vec![named_tool_call(
                &call_id,
                "write_file",
                json!({"path": destination, "content": format!("revision-{index}")}),
            )]),
        );
        let tool = planned
            .host_requests
            .iter()
            .find(|request| matches!(&request.payload, HostRequest::ExecuteTool { call_id: pending, .. } if pending == &call_id))
            .cloned()
            .unwrap_or_else(|| panic!("expected tool request after model iteration {index}: {planned:?}"));
        let completed = complete_tool(&mut core, &tool, &call_id);
        model = model_request(&completed.host_requests);
        if index == 0 {
            let HostRequest::ModelStream {
                tools, messages, ..
            } = &model.payload
            else {
                panic!("expected convergence model request")
            };
            assert!(!tools.is_empty());
            assert!(messages.iter().any(|message| {
                message
                    .get("content")
                    .and_then(Value::as_str)
                    .is_some_and(|content| content.contains("final tool-capable model iteration"))
            }));
        }
    }

    (core, model)
}

fn model_delta(request: &HostRequestEnvelope, delta: &str) -> HostResponse {
    HostResponse::ModelDelta {
        channel: ModelDeltaChannel::Text,
        part_id: match &request.payload {
            HostRequest::ModelStream { text_part_id, .. } => text_part_id.clone(),
            _ => panic!("expected model request"),
        },
        delta: delta.to_string(),
    }
}

fn reasoning_only_completion(request: &HostRequestEnvelope, content: &str) -> HostResponse {
    let reasoning_part_id = match &request.payload {
        HostRequest::ModelStream {
            reasoning_part_id, ..
        } => reasoning_part_id.clone(),
        _ => panic!("expected model request"),
    };
    HostResponse::ModelCompleted {
        items: vec![serde_json::to_value(AgentItem::Reasoning {
            id: ItemId::from_string(reasoning_part_id),
            summary: String::new(),
            content: content.to_string(),
            provider_continuation: None,
        })
        .expect("serialize reasoning")],
        tool_calls: Vec::new(),
        usage: serde_json::to_value(TokenUsage {
            input_tokens: Some(8),
            output_tokens: Some(64),
            total_tokens: Some(72),
            ..TokenUsage::default()
        })
        .expect("serialize usage"),
    }
}

fn tool_call(call_id: &str) -> ModelToolCall {
    ModelToolCall {
        call_id: nola_core::agent::CallId::from_string(call_id.to_string()),
        name: "write_file".to_string(),
        arguments: json!({"path": "/tmp/example.txt", "content": "hello"}),
        permission_scope: None,
        permission_rationale: None,
    }
}

fn named_tool_call(call_id: &str, name: &str, arguments: serde_json::Value) -> ModelToolCall {
    ModelToolCall {
        call_id: nola_core::agent::CallId::from_string(call_id.to_string()),
        name: name.to_string(),
        arguments,
        permission_scope: None,
        permission_rationale: None,
    }
}

fn skill_runtime_core_config_with_items(initial_items: Vec<AgentItem>) -> AgentCoreConfig {
    let mut config = core_config(initial_items);
    let catalog = config.host_catalog.as_mut().expect("host catalog");
    for name in ["Skill", "skill_update"] {
        catalog.capabilities.insert(
            name.to_string(),
            capability(name, HostCapabilityRoute::Tool, None),
        );
        config.tools.push(json!({
            "type": "function",
            "function": {
                "name": name,
                "description": name,
                "parameters": {"type": "object"}
            }
        }));
    }
    config
}

fn skill_runtime_core_config() -> AgentCoreConfig {
    skill_runtime_core_config_with_items(Vec::new())
}

fn complete_named_tool_round(
    core: &mut AgentCore,
    model_request: &HostRequestEnvelope,
    call_id: &str,
    capability_id: &str,
    arguments: Value,
    output: Value,
) -> nola_core::agent::AgentTransition {
    let dispatched = submit_response(
        core,
        model_request,
        1,
        empty_model_completion(vec![named_tool_call(call_id, capability_id, arguments)]),
    );
    let tool_request = request_matching(
        &dispatched.host_requests,
        |payload| matches!(payload, HostRequest::ExecuteTool { call_id: requested, .. } if requested == call_id),
    );
    submit_response(
        core,
        &tool_request,
        1,
        HostResponse::ToolCompleted {
            call_id: call_id.to_string(),
            output,
            outcome: nola_core::agent::host::HostToolOutcome::Succeeded,
            activation: None,
            full_output_ref: None,
            artifacts: Vec::new(),
        },
    )
}

fn complete_verified_document_skill_run(core: &mut AgentCore) -> nola_core::agent::AgentTransition {
    let started = core
        .submit(user_input(
            "skill-runtime-user",
            "Create and verify the requested document.",
            AgentInputDelivery::CurrentTurn,
        ))
        .expect("start Skill Turn");
    let digest = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let invoked = complete_named_tool_round(
        core,
        &model_request(&started.host_requests),
        "skill-call",
        "Skill",
        json!({"skill": "document-create"}),
        json!({
            "type": "invoked_skill",
            "skill_id": "document-create",
            "version": "1.0.0",
            "display_name": "Document create",
            "instructions_ref": format!("skill://document-create/1.0.0/SKILL.md#sha256={digest}"),
            "instructions_digest": digest,
            "required_tools": [],
            "mode": "inline",
            "execution_contract": {
                "schemaVersion": 1,
                "progressSteps": [{"id": "render", "title": "Render document", "required": true}],
                "artifacts": [{"id": "document", "mediaType": "application/vnd.openxmlformats-officedocument.wordprocessingml.document", "required": true}],
                "verifiers": [{"id": "render-check", "description": "Inspect rendered pages", "method":"model_judgment", "required": true}],
                "resumePoints": ["after-render"]
            }
        }),
    );
    let run_id = invoked
        .snapshot
        .skill_runtime
        .runs
        .keys()
        .next()
        .expect("active SkillRun id")
        .clone();
    let progress = complete_named_tool_round(
        core,
        &model_request(&invoked.host_requests),
        "skill-progress",
        "skill_update",
        json!({"run_id": run_id, "action": "progress", "step_id": "render", "completed": true}),
        json!({"type": "skill_runtime_update", "run_id": run_id, "action": "progress", "step_id": "render", "completed": true}),
    );
    let artifact = complete_named_tool_round(
        core,
        &model_request(&progress.host_requests),
        "skill-artifact",
        "skill_update",
        json!({"run_id": run_id, "action": "artifact", "artifact_id": "document", "artifact_ref": "file:///tmp/report.docx"}),
        json!({"type": "skill_runtime_update", "run_id": run_id, "action": "artifact", "artifact_id": "document", "artifact_ref": "file:///tmp/report.docx"}),
    );
    let resumed = complete_named_tool_round(
        core,
        &model_request(&artifact.host_requests),
        "skill-resume",
        "skill_update",
        json!({"run_id": run_id, "action": "resume_point", "resume_point": "after-render"}),
        json!({"type": "skill_runtime_update", "run_id": run_id, "action": "resume_point", "resume_point": "after-render"}),
    );
    let verified = complete_named_tool_round(
        core,
        &model_request(&resumed.host_requests),
        "skill-verification",
        "skill_update",
        json!({"run_id": run_id, "action": "verification", "verifier_id": "render-check", "passed": true, "evidence": {"kind":"model_judgment","sources":[]}}),
        json!({"type": "skill_runtime_update", "run_id": run_id, "action": "verification", "verifier_id": "render-check", "passed": true, "evidence": {"kind":"model_judgment","sources":[]}, "summary": "3 pages inspected"}),
    );
    assert_eq!(
        verified.snapshot.skill_runtime.runs[&run_id].status,
        nola_core::agent::capability::SkillRunStatus::Completed
    );
    let settled = submit_response(
        core,
        &model_request(&verified.host_requests),
        1,
        empty_model_completion(vec![named_tool_call(
            "stale-skill-update",
            "skill_update",
            json!({"run_id": "document-create", "action": "report", "step_id": "report"}),
        )]),
    );
    assert!(!settled.snapshot.engineering.recovery_required);
    assert!(!settled
        .host_requests
        .iter()
        .any(|request| matches!(request.payload, HostRequest::ExecuteTool { .. })));
    submit_response(
        core,
        &model_request(&settled.host_requests),
        1,
        visible_model_completion("skill-answer", "The verified document is ready."),
    )
}

fn observed_action_completion(core: &AgentCore, answer: &str, calls: &[&str]) -> HostResponse {
    let checkpoint = core.checkpoint(CheckpointReason::Manual);
    let state = &checkpoint.state.execution.requirements;
    let binding = checkpoint.state.execution.active_turn.as_ref().unwrap()
        .last_model_requirement_binding.as_ref().unwrap();
    let evidence: Vec<_> = calls.iter().map(|call| {
        let id = &state.observations.values().find(|o| o.call_id == *call).unwrap().evidence_id;
        binding.evidence.iter().find(|(_, value)| *value == id).unwrap().0.clone()
    }).collect();
    let assessments: Vec<_> = binding.requirements.keys().map(|reference|
        json!({"requirement":reference,"verdict":"satisfied","rationale":answer,"evidence":evidence})).collect();
    empty_model_completion(vec![named_tool_call("observed-completion", "requirements_update",
        json!({"outcome":"completed","assessments":assessments,"answer":answer}))])
}

// These assessed-action fixtures explicitly declare their requested output;
// observed writes alone must never manufacture the user's task scope.
fn declare_fixture_action_scope(core:&mut AgentCore,request:&HostRequestEnvelope,uri:&str,version:&str,validated:bool)->HostRequestEnvelope {
    let checkpoint=core.checkpoint(CheckpointReason::Manual);
    let scope=checkpoint.state.execution.requirements.scope.as_ref().unwrap();
    let binding=checkpoint.state.execution.active_turn.as_ref().unwrap().last_model_requirement_binding.as_ref().unwrap();
    let sources:Vec<_>=binding.user_sources.keys().cloned().collect();
    assert!(!sources.is_empty(),"scope must cite actual user input, not assistant prose");
    let mut requirements=Vec::new();let mut dispositions=Vec::new();
    for (index,(reference,id)) in binding.requirements.iter().enumerate() {
        let original=scope.requirements.iter().find(|r|r.requirement_id==*id).unwrap();
        let key=format!("fixture-output-{index}");
        requirements.push(json!({"key":key,"description":original.description,"source_refs":sources,"dependencies":[],
            "resource_scope":{"kind":"declared","required":[{"resource_uri":uri,"operation":"write","version":version,
                "predicate":if validated {Some("fixture_check")} else {None},"source_refs":sources}],"exclusions":[]}}));
        dispositions.push(json!({"kind":"replace","requirement_id":reference,"successors":[{"kind":"local_key","key":key}],
            "reason":"Declare the explicitly requested fixture output without reducing the original task","user_input_refs":sources}));
    }
    let declared=submit_response(core,request,1,empty_model_completion(vec![named_tool_call("declare-fixture-output","task_scope_update",json!({"requirements":requirements,"dispositions":dispositions}))]));
    assert!(!declared.event_envelopes.iter().any(|event|matches!(event.event,AgentEvent::TurnCompleted{..})),"scope declaration must not itself finish the task");
    let after=core.checkpoint(CheckpointReason::Manual);
    assert!(after.state.execution.requirements.scope.as_ref().unwrap().requirements.iter().all(|requirement|
        matches!(requirement.resource_scope,nola_domain::run_requirements::RequirementResourceScope::Declared {..})),"scope declaration must be admitted");
    model_request(&declared.host_requests)
}

fn exercise_pending_conversation_grant(same_resource: bool) {
    let mut config = engineering_core_config();
    config
        .host_catalog
        .as_mut()
        .and_then(|catalog| catalog.capabilities.get_mut("write_file"))
        .expect("write capability")
        .parallelism = HostCapabilityParallelism::ConcurrentSafe;
    let mut core = AgentCore::try_new(config.clone()).expect("create Core");
    let started = core
        .submit(user_input(
            "user-parallel-permission",
            "write both files",
            AgentInputDelivery::CurrentTurn,
        ))
        .expect("start turn");
    let first_model = model_request(&started.host_requests);
    let permission = submit_response(
        &mut core,
        &first_model,
        1,
        empty_model_completion(vec![
            named_tool_call(
                "parallel-write-1",
                "write_file",
                json!({"path":"/tmp/parallel-a.txt","content":"a"}),
            ),
            named_tool_call(
                "parallel-write-2",
                "write_file",
                json!({"path": if same_resource { "/tmp/parallel-a.txt" } else { "/tmp/parallel-b.txt" },"content":"b"}),
            ),
        ]),
    );
    let permission_requests = permission
        .host_requests
        .iter()
        .filter(|request| matches!(request.payload, HostRequest::RequestPermission { .. }))
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(permission_requests.len(), 2);
    let permission_request_id = match &permission_requests[0].payload {
        HostRequest::RequestPermission {
            permission_request_id,
            ..
        } => permission_request_id.clone(),
        _ => unreachable!(),
    };

    let allowed = core
        .submit(AgentSubmission::PermissionResponse {
            request_id: nola_core::agent::PermissionRequestId::from_string(permission_request_id.clone()),
            decision: PermissionUserDecision::AllowConversation,
            grant: Some(json!({
                "scope": "conversation",
                "resource_pattern": "*"
            })),
        })
        .expect("allow only the selected concrete resource");

    if same_resource {
        assert!(allowed.snapshot.permissions.pending.is_empty());
        assert_eq!(allowed.host_requests.iter().filter(|request|
            matches!(request.payload, HostRequest::ExecuteTool { .. })).count(), 2);
        let checkpoint = core.checkpoint(CheckpointReason::Manual);
        let wire = serde_json::to_value(&checkpoint).unwrap();
        let receipts = &wire["state"]["execution"]["permissions"]["receipts"];
        assert_eq!(receipts.as_array().unwrap().len(), 2);
        assert_eq!(receipts[0]["outcome"]["type"], "authorization");
        assert_eq!(receipts[1]["outcome"]["type"], "conversation_rule_applied");
        assert_eq!(receipts[1]["outcome"]["source_request_id"], permission_request_id);
        assert!(receipts[1]["outcome"].get("decision").is_none());
        let settled = allowed.event_envelopes.iter().filter_map(|envelope| match &envelope.event {
            AgentEvent::PermissionSettled { receipt } => {
                assert_eq!(envelope.turn_id.as_deref(), Some(receipt.turn_id.as_str()));
                Some(serde_json::to_value(receipt).unwrap())
            }
            _ => None,
        }).collect::<Vec<_>>();
        assert_eq!(&settled, receipts.as_array().unwrap(), "canonical facts must retain the exact Core receipts");
        for (index, expected) in [(0, "allow_conversation"), (1, "conversation_rule_applied")] {
            let call_id = receipts[index]["call_id"].as_str().unwrap();
            assert!(allowed.snapshot.projection.parts.values().any(|part| matches!(part,
                AgentPart::Tool(tool) if tool.call_id == call_id
                    && tool.permission.as_ref().and_then(|permission| permission.decision.as_deref()) == Some(expected)
            )), "public Part must distinguish user selection from Core reuse");
        }
        AgentCore::restore(config.clone(), checkpoint).expect("restore sourced reuse");
        for invalid in ["missing", "self", "allow_once", "deny", "future"] {
            let mut bad = wire.clone();
            let rows = bad["state"]["execution"]["permissions"]["receipts"].as_array_mut().unwrap();
            match invalid {
                "missing" => rows[1]["outcome"]["source_request_id"] = json!("absent"),
                "self" => rows[1]["outcome"]["source_request_id"] = rows[1]["request_id"].clone(),
                "future" => rows.swap(0, 1),
                decision => rows[0]["outcome"]["decision"] = json!(decision),
            }
            assert!(AgentCore::restore(config.clone(), serde_json::from_value(bad).unwrap()).is_err(),
                "invalid permission provenance must fail recovery: {invalid}");
        }
        return;
    }

    assert!(!allowed
        .host_requests
        .iter()
        .any(|request| matches!(request.payload, HostRequest::RequestPermission { .. })));
    assert_eq!(
        allowed
            .host_requests
            .iter()
            .filter(|request| matches!(request.payload, HostRequest::ExecuteTool { .. }))
            .count(),
        1,
        "Host grant JSON cannot authorize the other file"
    );
    assert_eq!(allowed.snapshot.permissions.pending.len(), 1);
    let sibling = allowed.snapshot.permissions.pending[0].clone();
    let selected = allowed.host_requests.iter().find_map(|request| match &request.payload {
        HostRequest::ExecuteTool { call_id, .. } => Some(call_id.as_str()),
        _ => None,
    }).unwrap();
    assert_ne!(selected, sibling.call_id.as_str());
    let directly_allowed = core.submit(AgentSubmission::PermissionResponse {
        request_id: sibling.request_id,
        decision: PermissionUserDecision::AllowConversation,
        grant: None,
    }).expect("approve the remaining resource explicitly");
    assert!(directly_allowed.snapshot.permissions.pending.is_empty());
    let executed = directly_allowed.host_requests.iter().filter_map(|request| match &request.payload {
        HostRequest::ExecuteTool { call_id, .. } => Some(call_id.as_str()),
        _ => None,
    }).collect::<Vec<_>>();
    assert_eq!(executed, vec![sibling.call_id.as_str()]);
    let checkpoint = serde_json::to_value(core.checkpoint(CheckpointReason::Manual).state).unwrap();
    let receipts = checkpoint["execution"]["permissions"]["receipts"]
        .as_array()
        .unwrap();
    assert_eq!(
        receipts.len(),
        2,
        "separate explicit decisions must retain separate receipts"
    );
    assert!(receipts
        .iter()
        .all(|receipt| receipt["outcome"]["decision"] == "allow_conversation"));
}

fn durable_question_arguments(id: &str) -> Value {
    json!({"questions":[{"id":id,"prompt":"Choose a scope","options":[{"id":"one","label":"One","description":null}],"allow_freeform":true,"required":true,"max_text_chars":256}]})
}

fn durable_question_config() -> AgentCoreConfig {
    let mut config = source_core_config();
    config.host_catalog.as_mut().unwrap().capabilities.insert("request_user_input".into(), capability("request_user_input",HostCapabilityRoute::Tool,None));
    config.tools.push(json!({"type":"function","function":{"name":"request_user_input","parameters":{"type":"object"}}}));
    config
}

fn answer_wait(wait: &nola_domain::user_input_wait::UserInputWait, command_id: &str) -> AgentSubmission {
    use nola_domain::user_input_wait::*;
    AgentSubmission::RespondToUserInputWait { input:RespondToUserInputWait {
        command_id:command_id.into(),wait_id:wait.wait_id.clone(),expected_revision:wait.revision,
        response_schema_digest:wait.response_schema_digest.clone(),
        response:UserInputWaitResponse::Answers {answers:vec![QuestionAnswer {question_id:wait.question_set.questions[0].id.clone(), value:QuestionAnswerValue::Option {option_id:"one".into()}}]},
    }}
}

fn assert_manual_compaction_busy_without_mutation(core: &mut AgentCore, operation: &str) {
    let before = serde_json::to_value(core.checkpoint(CheckpointReason::Manual)).unwrap();
    let error = core.submit(AgentSubmission::Compact {
        operation_id: operation.to_string(),
    }).expect_err("manual maintenance must not replace active execution");
    assert!(error.contains("manual compaction busy"), "{error}");
    assert!(error.contains("no compaction was started or queued"), "{error}");
    assert_eq!(serde_json::to_value(core.checkpoint(CheckpointReason::Manual)).unwrap(), before);
}

fn suspended_custom_fixture() -> (AgentCore, HostRequestEnvelope) {
    let mut core = AgentCore::try_new(engineering_core_config()).unwrap();
    let started = core
        .submit(user_input(
            "custom-user",
            "write the protected file",
            AgentInputDelivery::CurrentTurn,
        ))
        .unwrap();
    let first = model_request(&started.host_requests);
    let pending = submit_response(
        &mut core,
        &first,
        1,
        empty_model_completion(vec![tool_call("custom-call")]),
    );
    let ask = request_matching(&pending.host_requests, |payload| {
        matches!(payload, HostRequest::RequestPermission { .. })
    });
    (core, ask)
}

fn custom_response(ask: &HostRequestEnvelope, instruction: &str) -> AgentSubmission {
    let HostRequest::RequestPermission {
        permission_request_id,
        ..
    } = &ask.payload
    else {
        panic!("ask")
    };
    AgentSubmission::HostResponse {
        response: response_for(
            ask,
            1,
            HostResponse::PermissionResolved {
                request_id: permission_request_id.clone(),
                resolution: nola_host_protocol::HostPermissionResolution::provide_instruction(
                    instruction,
                ),
            },
        ),
    }
}

#[test]
fn delegated_agent_repetitive_stream_is_cancelled_before_commit() {
    let mut config = core_config(Vec::new());
    config
        .host_catalog
        .as_mut()
        .expect("Host catalog")
        .capabilities
        .insert(
            "agent.delegate".to_string(),
            capability("agent.delegate", HostCapabilityRoute::Model, None),
        );
    config.turn_identity.execution_backend.backend_id = "codex".to_string();
    admit_external_fixture_workspace(&mut config);
    config.turn_identity.prompt.stable_prefix_digest = "c".repeat(64);
    let mut core = AgentCore::try_new(config).expect("create delegated Core");
    let started = core
        .submit(user_input(
            "delegated-output-user",
            "Inspect the project.",
            AgentInputDelivery::CurrentTurn,
        ))
        .expect("start delegated Turn");
    let delegated = request_matching(&started.host_requests, |payload| {
        matches!(payload, HostRequest::DelegatedAgent { .. })
    });
    let text_part_id = match &delegated.payload {
        HostRequest::DelegatedAgent { request } => request.text_part_id.clone(),
        _ => unreachable!(),
    };
    let repeated =
        "The same analysis is being reconsidered without new evidence or a changed result.\n"
            .repeat(180);
    let transition = submit_response(
        &mut core,
        &delegated,
        1,
        HostResponse::DelegatedAgentEvent {
            event: DelegatedAgentEventV1::TextDelta {
                part_id: text_part_id,
                delta: repeated,
            },
        },
    );

    assert!(transition
        .host_requests
        .iter()
        .any(|request| { matches!(&request.payload, HostRequest::Cancel { .. }) }));
    assert!(!transition.event_envelopes.iter().any(|event| matches!(&event.event, AgentEvent::TurnFailed { .. })));
    assert_eq!(core.turn_status(), nola_core::agent::AgentTurnStatus::Cancelling);
    let cancel = request_matching(&transition.host_requests, |payload| matches!(payload, HostRequest::Cancel { .. }));
    let stopped = submit_response(&mut core, &cancel, 1, HostResponse::CancellationConfirmed { target_effect_id: delegated.effect_id.clone() });
    assert!(stopped.event_envelopes.iter().any(|event| matches!(&event.event, AgentEvent::TurnFailed { .. })));
    assert_eq!(core.turn_status(), nola_core::agent::AgentTurnStatus::Failed);
}

mod part_1;
mod part_2;
mod part_3;
mod part_4;
mod part_5;
mod part_6;
