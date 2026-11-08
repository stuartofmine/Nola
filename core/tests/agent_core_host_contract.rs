#[path = "support/model_identity.rs"]
mod model_identity;
#[path = "support/requirement_result_contract_tests.rs"]
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
        "contract": "nola.tool_result.v2", "data": data,
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

#[test]
fn delegated_reused_rpc_has_distinct_permission_occurrences_and_preserves_control_target() {
    let mut config = core_config(Vec::new());
    config
        .permission_rules
        .rules
        .push(nola_core::agent::permissions::PermissionRule {
            capability_pattern: "shell".into(),
            resource_pattern: "*".into(),
            decision: nola_core::agent::permissions::PermissionPolicyDecision::Ask,
            source: nola_core::agent::permissions::PermissionRuleSource::Conversation,
        });
    config.host_catalog.as_mut().unwrap().capabilities.insert(
        "agent.delegate".into(),
        capability("agent.delegate", HostCapabilityRoute::Model, None),
    );
    config.turn_identity.execution_backend =
        nola_domain::execution_backend::ExecutionBackendSelection {
            backend_id: "codex".into(),
            ..Default::default()
        };
    admit_external_fixture_workspace(&mut config);
    config.turn_identity.prompt.stable_prefix_digest = "a".repeat(64);
    let mut core = AgentCore::try_new(config.clone()).unwrap();
    let started = core
        .submit(user_input(
            "user",
            "Inspect project",
            AgentInputDelivery::CurrentTurn,
        ))
        .unwrap();
    let delegated = request_matching(&started.host_requests, |payload| {
        matches!(payload, HostRequest::DelegatedAgent { .. })
    });
    submit_response(
        &mut core,
        &delegated,
        1,
        HostResponse::DelegatedAgentEvent {
            event: DelegatedAgentEventV1::SessionStarted {
                backend_id: "codex".into(),
                backend_session_id: "backend-session".into(),
            },
        },
    );
    for (index, occurrence_id) in ["occurrence-one", "occurrence-two"].into_iter().enumerate() {
        let asked = submit_response(
            &mut core,
            &delegated,
            index as u64 + 2,
            HostResponse::DelegatedAgentEvent {
                event: DelegatedAgentEventV1::PermissionRequested {
                    occurrence: nola_host_protocol::DelegatedPermissionOccurrence {
                        occurrence_id: occurrence_id.into(),
                        worker_generation: "generation".into(),
                    },
                    request_id: "reused-rpc".into(),
                    call_id: "reused-call".into(),
                    capability_id: "shell".into(),
                    resource: "/workspace".into(),
                    reason: "fixture".into(),
                    input: serde_json::json!({"command":"fixture"}),
                },
            },
        );
        assert_eq!(
            asked.snapshot.permissions.pending.len(),
            1,
            "index {index}: {asked:?}"
        );
        let pending = asked.snapshot.permissions.pending[0].clone();
        assert_eq!(pending.request_id.as_str(), occurrence_id);
        if index == 1 {
            let before =
                serde_json::to_value(core.checkpoint(CheckpointReason::Manual).state).unwrap();
            assert!(core
                .submit(AgentSubmission::PermissionResponse {
                    request_id: nola_core::agent::PermissionRequestId::from_string(
                        "occurrence-one".into()
                    ),
                    decision: PermissionUserDecision::AllowOnce,
                    grant: None
                })
                .is_err());
            assert_eq!(
                serde_json::to_value(core.checkpoint(CheckpointReason::Manual).state).unwrap(),
                before
            );
        }
        // Guidance rotates the UI request while preserving backend occurrence and RPC.
        let ask = request_matching(&asked.host_requests, |payload| {
            matches!(payload, HostRequest::RequestPermission { .. })
        });
        let guided = core
            .submit(custom_response(&ask, "retain exact original call"))
            .unwrap();
        let rotated = guided.snapshot.permissions.pending[0].request_id.clone();
        assert_ne!(rotated, pending.request_id);
        let approved = core
            .submit(AgentSubmission::PermissionResponse {
                request_id: rotated,
                decision: PermissionUserDecision::AllowOnce,
                grant: None,
            })
            .unwrap();
        let control = request_matching(&approved.host_requests, |payload| {
            matches!(payload, HostRequest::ResolveDelegatedPermission { .. })
        });
        let HostRequest::ResolveDelegatedPermission {
            occurrence,
            permission_request_id,
            target_effect_id,
            delivery,
            ..
        } = &control.payload
        else {
            unreachable!()
        };
        assert_eq!(occurrence.occurrence_id, occurrence_id);
        assert_eq!(permission_request_id, "reused-rpc");
        assert_eq!(target_effect_id, &delegated.effect_id);
        assert!(delivery.is_none());
        submit_response(
            &mut core,
            &control,
            1,
            HostResponse::DelegatedPermissionTransportWritten {
                delivery_id: format!("delivery-{index}"),
            },
        );
    }
}

#[test]
fn delegated_agent_events_have_one_core_owned_lifecycle_after_cold_restore() {
    const ANSWER: &str = "DELEGATED_FINAL_7319\n\n- first\n- second";
    const REASONING: &str = "Inspect the canonical request before answering.";

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
    config.turn_identity.execution_backend =
        nola_domain::execution_backend::ExecutionBackendSelection {
            backend_id: "codex".to_string(),
            ..Default::default()
        };
    admit_external_fixture_workspace(&mut config);
    config.turn_identity.prompt.stable_prefix_digest = "a".repeat(64);

    let mut core = AgentCore::try_new(config.clone()).expect("create delegated Core");
    let started = core
        .submit(user_input(
            "delegated-user",
            "Inspect the project and report the result.",
            AgentInputDelivery::CurrentTurn,
        ))
        .expect("start delegated Turn");
    let mut delegated = request_matching(&started.host_requests, |payload| {
        matches!(payload, HostRequest::DelegatedAgent { .. })
    });
    assert!(delegated.delegated_invocation_id.is_none());
    delegated.delegated_invocation_id = Some("5c1487b2-e878-43fa-8f23-d1e2a53f9524".into());
    // Binding delivery metadata may not move an existing effect to another
    // session/Turn, even before its delegated invocation has been attached.
    let before_binding = serde_json::to_value(core.checkpoint(CheckpointReason::Manual)).unwrap();
    for foreign_turn in [false, true] {
        let mut wrong = delegated.clone();
        if foreign_turn {
            wrong.turn_id = Some("foreign-turn".into());
        } else {
            wrong.session_id = "foreign-session".into();
        }
        assert_eq!(
            core.bind_committed_host_effect(&wrong).unwrap_err(),
            if foreign_turn {
                "Host binding changed immutable effect/session/Turn ownership"
            } else {
                "committed Host effect belongs to another session"
            }
        );
        assert_eq!(
            serde_json::to_value(core.checkpoint(CheckpointReason::Manual)).unwrap(),
            before_binding
        );
    }
    let mut wrong_payload = delegated.clone();
    wrong_payload.payload = HostRequest::Cancel {
        target_effect_id: delegated.effect_id.clone(),
        target_not_started: None,
    };
    assert_eq!(core.bind_committed_host_effect(&wrong_payload).unwrap_err(),
        "Host binding cancellation policy kind mismatch");
    assert_eq!(
        serde_json::to_value(core.checkpoint(CheckpointReason::Manual)).unwrap(),
        before_binding
    );
    core.bind_committed_host_effect(&delegated)
        .expect("bind Application committed invocation");
    for replacement in [
        None,
        Some("fc4e320e-7ed5-4daa-bb8e-737c2185b4c5".to_string()),
    ] {
        let mut wrong = delegated.clone();
        wrong.delegated_invocation_id = replacement;
        let before_replacement =
            serde_json::to_value(core.checkpoint(CheckpointReason::Manual)).unwrap();
        assert_eq!(
            core.bind_committed_host_effect(&wrong).unwrap_err(),
            "Host binding changed immutable delegated invocation"
        );
        assert_eq!(
            serde_json::to_value(core.checkpoint(CheckpointReason::Manual)).unwrap(),
            before_replacement
        );
    }
    core.bind_committed_host_effect(&delegated)
        .expect("same invocation binding is idempotent");
    let (text_part_id, reasoning_part_id) = match &delegated.payload {
        HostRequest::DelegatedAgent { request } => (
            request.text_part_id.clone(),
            request.reasoning_part_id.clone(),
        ),
        _ => unreachable!(),
    };

    submit_response(
        &mut core,
        &delegated,
        1,
        HostResponse::DelegatedAgentEvent {
            event: DelegatedAgentEventV1::SessionStarted {
                backend_id: "codex".to_string(),
                backend_session_id: "codex-session-7319".to_string(),
            },
        },
    );
    submit_response(
        &mut core,
        &delegated,
        2,
        HostResponse::DelegatedAgentEvent {
            event: DelegatedAgentEventV1::ReasoningDelta {
                part_id: reasoning_part_id,
                delta: REASONING.to_string(),
            },
        },
    );
    submit_response(
        &mut core,
        &delegated,
        3,
        HostResponse::DelegatedAgentEvent {
            event: DelegatedAgentEventV1::TextDelta {
                part_id: text_part_id.clone(),
                delta: ANSWER.to_string(),
            },
        },
    );
    submit_response(
        &mut core,
        &delegated,
        4,
        HostResponse::DelegatedAgentEvent {
            event: DelegatedAgentEventV1::ToolStarted {
                call_id: "delegated-call-7319".to_string(),
                capability_id: "read_file".to_string(),
                input: json!({"path": "README.md"}),
            },
        },
    );
    submit_response(
        &mut core,
        &delegated,
        5,
        HostResponse::DelegatedAgentEvent {
            event: DelegatedAgentEventV1::ToolCompleted {
                call_id: "delegated-call-7319".to_string(),
                capability_id: "read_file".to_string(),
                output: json!({"text": "project evidence"}),
                outcome: nola_core::agent::host::HostToolOutcome::Succeeded,
                full_output_ref: None,
                artifacts: Vec::new(),
            },
        },
    );
    submit_response(
        &mut core,
        &delegated,
        6,
        HostResponse::DelegatedAgentEvent {
            event: DelegatedAgentEventV1::Usage {
                used_tokens: 4_096,
                context_window_tokens: 128_000,
                input_tokens: Some(3_800),
                output_tokens: Some(296),
            },
        },
    );
    let checkpoint = core.checkpoint(CheckpointReason::Manual);
    let mut recovery_config = config.clone();
    recovery_config.turn_identity = checkpoint.state.configuration.turn_identity.clone();
    let mut invalid_core = AgentCore::restore(recovery_config, checkpoint)
        .expect("restore pre-terminal delegated source");
    let before_invalid = invalid_core.checkpoint(CheckpointReason::Manual)
        .state.execution.requirements.model_source.clone();
    assert!(invalid_core.submit(AgentSubmission::HostResponse {
        response: response_for(&delegated, 7, HostResponse::DelegatedAgentEvent {
            event: DelegatedAgentEventV1::Completed { items: vec![json!({"type":"not_an_agent_item"})], usage: json!({}) },
        }),
    }).is_err());
    let after_invalid = invalid_core.checkpoint(CheckpointReason::Manual)
        .state.execution.requirements.model_source.clone();
    assert_eq!(after_invalid, before_invalid,
        "invalid delegated completion cannot admit a model decision source");
    let completed = submit_response(
        &mut core,
        &delegated,
        7,
        HostResponse::DelegatedAgentEvent {
            event: DelegatedAgentEventV1::Completed {
                items: vec![serde_json::to_value(AgentItem::assistant_message(
                    ItemId::from_string(text_part_id),
                    ANSWER,
                ))
                .expect("encode completed assistant item")],
                usage: serde_json::to_value(TokenUsage {
                    input_tokens: Some(3_800),
                    output_tokens: Some(296),
                    ..Default::default()
                })
                .expect("encode delegated usage"),
            },
        },
    );

    assert!(completed.event_envelopes.iter().any(|envelope| matches!(
        &envelope.event,
        AgentEvent::RunResultDecided { decision, .. }
            if matches!(&decision.source, nola_domain::core_requirements::CoreDecisionSource::Model {
                effect_id, request_id, attempt_id, call_id
            } if effect_id == &delegated.effect_id.0 && request_id == &delegated.request_id
                && attempt_id == &delegated.attempt_id && call_id.is_none())
    )), "delegated terminal decision retains its exact accepted Host source before effect retirement");
    let text_parts = completed
        .snapshot
        .projection
        .parts
        .values()
        .filter_map(|part| match part {
            AgentPart::Text(part) if part.text.contains("DELEGATED_FINAL_7319") => Some(part),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(text_parts.len(), 1, "delegated final body was duplicated");
    assert_eq!(text_parts[0].text, ANSWER);
    assert!(text_parts[0].is_complete);
    assert_eq!(
        completed
            .snapshot
            .projection
            .parts
            .values()
            .filter(|part| matches!(part, AgentPart::Reasoning(part) if part.detail == REASONING))
            .count(),
        1,
        "delegated reasoning was lost or duplicated"
    );
    assert_eq!(
        completed
            .snapshot
            .projection
            .parts
            .values()
            .filter(|part| matches!(part, AgentPart::Tool(part) if part.call_id.as_str() == "delegated-call-7319" && part.state == nola_core::agent::parts::ToolPartState::Completed))
            .count(),
        1,
        "delegated tool lifecycle was lost or duplicated"
    );
    assert_eq!(completed.snapshot.context_usage.used_tokens, 4_096);
    assert_eq!(completed.snapshot.context_window_tokens, 128_000);

    let checkpoint = checkpoint_after(&core, &completed);
    assert_eq!(
        checkpoint
            .state
            .item_ownership
            .get("delegated-user")
            .map(|owner| owner.message_id.as_str()),
        Some("delegated-user"),
        "the first user item must be owned by the delegated Turn"
    );
    config.turn_identity = checkpoint.state.configuration.turn_identity.clone();
    let restored = AgentCore::restore(config, checkpoint).expect("cold restore delegated Core");
    assert_eq!(
        restored.snapshot().projection,
        completed.snapshot.projection
    );
}

#[test]
fn delegated_agent_repeated_tool_calls_are_cancelled_by_core() {
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
    config.turn_identity.prompt.stable_prefix_digest = "b".repeat(64);
    let mut core = AgentCore::try_new(config).expect("create delegated Core");
    let started = core
        .submit(user_input(
            "delegated-loop-user",
            "Inspect the project.",
            AgentInputDelivery::CurrentTurn,
        ))
        .expect("start delegated Turn");
    let delegated = request_matching(&started.host_requests, |payload| {
        matches!(payload, HostRequest::DelegatedAgent { .. })
    });

    let mut transition = started;
    for sequence in 1..=3 {
        transition = submit_response(
            &mut core,
            &delegated,
            sequence,
            HostResponse::DelegatedAgentEvent {
                event: DelegatedAgentEventV1::ToolStarted {
                    call_id: format!("delegated-loop-{sequence}"),
                    capability_id: "read_file".to_string(),
                    input: json!({"path": "README.md"}),
                },
            },
        );
    }

    assert!(transition.host_requests.iter().any(|request| {
        matches!(
            &request.payload,
            HostRequest::Cancel { target_effect_id, .. }
                if target_effect_id == &delegated.effect_id
        )
    }));
    assert_eq!(core.turn_status(), nola_core::agent::AgentTurnStatus::Cancelling);
    assert!(!transition.event_envelopes.iter().any(|event|
        matches!(&event.event, AgentEvent::TurnFailed { .. } | AgentEvent::RunResultDecided { .. })));
    let cancel = request_matching(&transition.host_requests, |payload| matches!(payload, HostRequest::Cancel { .. }));
    let stopped = submit_response(&mut core, &cancel, 1,
        HostResponse::CancellationConfirmed { target_effect_id: delegated.effect_id.clone() });
    assert!(stopped.event_envelopes.iter().any(|event| matches!(&event.event, AgentEvent::TurnFailed { .. })));
    assert!(stopped.event_envelopes.iter().any(|event|
        matches!(&event.event, AgentEvent::RunResultDecided { .. })),
        "delegated guard must settle through the unified terminal decision boundary");
}

#[test]
fn capacity_recovery_preserves_model_choice_and_does_not_force_append() {
    for (name, arguments, failure_code, max_iterations) in [
        ("write_file", json!({"path":"/workspace/existing.txt", "content":"replacement"}), "write_payload_too_large", 0),
        ("bash", json!({"command":"printf complete", "operation_kind":"inspect"}), "tool_arguments_truncated", 0),
        ("write_file", json!({"path":"/workspace/existing.txt", "content":"replacement"}), "write_payload_too_large", 4),
        ("bash", json!({"command":"printf complete", "operation_kind":"inspect"}), "tool_arguments_truncated", 4),
    ] {
        let mut config = source_core_config();
        config.max_model_iterations = max_iterations;
        // Recovery semantics, not permission policy: explicitly authorize this
        // fixture's exact simulated operation rather than trusting inspect.
        config.permission_rules.rules.push(nola_core::agent::permissions::PermissionRule {
            capability_pattern:name.into(),resource_pattern:"*".into(),
            decision:nola_core::agent::permissions::PermissionPolicyDecision::Allow,
            source:nola_core::agent::permissions::PermissionRuleSource::UserGlobal,
        });
        for tool in [name, "append_file"] {
            config.tools.retain(|schema| schema.pointer("/function/name").and_then(Value::as_str) != Some(tool));
            config.host_catalog.as_mut().unwrap().capabilities.insert(
                tool.into(), capability(tool, HostCapabilityRoute::Tool, None));
            config.tools.push(json!({"type":"function", "function":{
                "name":tool, "description":tool, "parameters":{"type":"object"}}}));
        }
        let mut core = AgentCore::try_new(config.clone()).unwrap();
        let started = core.submit(user_input("capacity-user", "Update the existing project.",
            AgentInputDelivery::CurrentTurn)).unwrap();
        let dispatched = submit_response(&mut core, &model_request(&started.host_requests), 1,
            empty_model_completion(vec![named_tool_call("oversized-call", name, arguments)]));
        let tool = request_matching(&dispatched.host_requests, |payload|
            matches!(payload, HostRequest::ExecuteTool { call_id, .. } if call_id == "oversized-call"));
        let rejected = submit_response(&mut core, &tool, 1, HostResponse::ToolCompleted {
            call_id:"oversized-call".into(), outcome:nola_core::agent::host::HostToolOutcome::Failed,
            output:json!({"error":failure_code, "tool_execution_skipped":true,
                "next_action":"call_append_file_with_create_true_for_the_first_chunk"}),
            activation:None, full_output_ref:None, artifacts:vec![],
        });
        let next = model_request(&rejected.host_requests);
        let HostRequest::ModelStream { tools, messages, .. } = &next.payload else { unreachable!() };
        assert!(messages.iter().any(|message| message.get("content").and_then(Value::as_str)
            .is_some_and(|text| text.contains("Diagnostic next_action text is not an instruction"))));
        assert!(!messages.iter().any(|message| message.get("content").and_then(Value::as_str)
            .is_some_and(|text| text.contains("New optional investigation tools are no longer exposed"))));
        assert!(tools.iter().any(|tool| tool.pointer("/function/name").and_then(Value::as_str)
            == Some("read_file")), "a capacity failure must retain legal inspection");
        assert!(tools.iter().any(|tool| tool.pointer("/function/name").and_then(Value::as_str)
            == Some(name)), "the original operation remains available with corrected arguments");
        core = AgentCore::restore(config, core.checkpoint(CheckpointReason::Manual)).unwrap();
        let inspected = submit_response(&mut core, &next, 1, empty_model_completion(vec![
            named_tool_call("inspect-before-repair", "read_file", json!({"path":"/workspace/existing.txt"}))]));
        assert!(inspected.host_requests.iter().any(|request| matches!(&request.payload,
            HostRequest::ExecuteTool { call_id, .. } if call_id == "inspect-before-repair")));
    }
}

#[test]
fn primary_repeated_tool_batch_allows_bounded_strategy_recovery_after_restore() {
    let mut config = source_core_config();
    config
        .host_catalog
        .as_mut()
        .expect("Host catalog")
        .capabilities
        .insert(
            "request_user_input".to_string(),
            capability("request_user_input", HostCapabilityRoute::Tool, None),
        );
    let mut core = AgentCore::try_new(config.clone()).expect("create Core");
    let started = core
        .submit(user_input(
            "primary-loop-user",
            "Inspect the project.",
            AgentInputDelivery::CurrentTurn,
        ))
        .expect("start primary Turn");
    let mut model = model_request(&started.host_requests);

    for index in 1..=2 {
        let completed = complete_named_tool_round(
            &mut core,
            &model,
            &format!("primary-loop-{index}"),
            "read_file",
            json!({"path":"README.md"}),
            json!({"content":"same evidence"}),
        );
        model = model_request(&completed.host_requests);
    }

    let checkpoint = submit_response(
        &mut core,
        &model,
        1,
        empty_model_completion(vec![named_tool_call(
            "primary-loop-3",
            "read_file",
            json!({"path":"README.md"}),
        )]),
    );

    let failure = checkpoint
        .snapshot
        .projection
        .parts
        .values()
        .find_map(|part| {
            let AgentPart::Tool(part) = part else {
                return None;
            };
            part.structured_output.as_ref().filter(|output| {
                output.get("error").and_then(Value::as_str) == Some("repeated_tool_batch")
            })
        });
    assert_eq!(
        failure.and_then(|output| output.get("consecutive_count")),
        Some(&json!(3))
    );

    let decision_request = model_request(&checkpoint.host_requests);
    let HostRequest::ModelStream {
        tools, messages, ..
    } = &decision_request.payload
    else {
        panic!("loop checkpoint must request the user's decision")
    };
    let exposed = tools
        .iter()
        .filter_map(|schema| schema.pointer("/function/name").and_then(Value::as_str))
        .collect::<Vec<_>>();
    assert!(exposed.contains(&"read_file"));
    assert!(exposed.contains(&"requirements_update"));
    assert!(messages.iter().any(|message| {
        message
            .get("content")
            .and_then(Value::as_str)
            .is_some_and(|content| {
                content.contains("Answer from existing evidence")
                    && content.contains("repetition alone is not such a decision")
            })
    }));

    // Cold restore must not turn a noncompliant recovery into a synthetic
    // user question, nor dispatch another copy of the rejected operation.
    let saved = core.checkpoint(CheckpointReason::Manual);
    for recovery in ["empty", "invalid_control"] {
        let mut restored = AgentCore::restore(config.clone(), saved.clone()).unwrap();
        let response = if recovery == "invalid_control" {
            empty_model_completion(vec![named_tool_call(
                "invalid-control", "requirements_update", json!({}),
            )])
        } else {
            empty_model_completion(vec![])
        };
        let stopped = submit_response(&mut restored, &decision_request, 1, response);
        assert!(stopped.snapshot.open_user_input_waits.is_empty());
        assert!(stopped.host_requests.iter().all(|request|
            !matches!(request.payload, HostRequest::ExecuteTool { .. } | HostRequest::ModelStream { .. })));
        let state = serde_json::to_value(restored.checkpoint(CheckpointReason::Manual)).unwrap();
        assert!(state.to_string().contains("execution.no_progress"));
        assert_eq!(
            state["state"]["conversation"]["context_window"]["cumulative_usage"]["input_tokens"].as_u64(),
            Some(u64::try_from(saved.state.conversation.context_window.cumulative_usage.input_tokens.unwrap() + 8).unwrap()),
            "rejected recovery must still account for the completed provider attempt",
        );
    }

    let mut retry = AgentCore::restore(config.clone(), saved.clone()).unwrap();
    let mut retry_model = decision_request.clone();
    for attempt in 0..config.max_strategy_recovery_attempts {
        let rejected = submit_response(&mut retry, &retry_model, 1,
            empty_model_completion(vec![named_tool_call(
                &format!("retry-{attempt}"), "read_file", json!({"path":"README.md"}))]));
        assert!(!rejected.host_requests.iter().any(|request|
            matches!(request.payload, HostRequest::ExecuteTool { .. })));
        if attempt + 1 < config.max_strategy_recovery_attempts {
            let checkpoint = retry.checkpoint(CheckpointReason::Manual);
            retry = AgentCore::restore(config.clone(), checkpoint).unwrap();
            retry_model = model_request(&rejected.host_requests);
        } else {
            assert!(!rejected.host_requests.iter().any(|request|
                matches!(request.payload, HostRequest::ModelStream { .. })));
            assert!(serde_json::to_value(retry.checkpoint(CheckpointReason::Manual)).unwrap()
                .to_string().contains("execution.no_progress"));
        }
    }

    let mut alternative = AgentCore::restore(config.clone(), saved.clone()).unwrap();
    let changed = submit_response(&mut alternative, &decision_request, 1,
        empty_model_completion(vec![named_tool_call("different-evidence", "read_file",
            json!({"path":"Cargo.toml"}))]));
    assert!(changed.host_requests.iter().any(|request|
        matches!(&request.payload, HostRequest::ExecuteTool { call_id, .. } if call_id == "different-evidence")));
    let tool_request = request_matching(&changed.host_requests, |payload|
        matches!(payload, HostRequest::ExecuteTool { call_id, .. } if call_id == "different-evidence"));
    let observed = submit_response(&mut alternative, &tool_request, 1,
        HostResponse::ToolCompleted {
            call_id: "different-evidence".into(), output: json!({"content":"new dependency evidence"}),
            outcome: nola_core::agent::host::HostToolOutcome::Succeeded,
            activation: None, full_output_ref: None, artifacts: Vec::new(),
        });
    assert!(observed.host_requests.iter().any(|request|
        matches!(request.payload, HostRequest::ModelStream { .. })));
    let observed_state = serde_json::to_value(alternative.checkpoint(CheckpointReason::Manual)).unwrap();
    let engineering = &observed_state["state"]["execution"]["active_turn"]["engineering"];
    assert_eq!(engineering["no_progress_recovery"]["state"], "inactive");
    assert_eq!(engineering["strategy_recovery"]["model_requests"].as_array().unwrap().len(), 1);
    assert!(engineering["strategy_recovery"]["observation_witnesses"]["different-evidence"].is_object());

    let mut direct = AgentCore::restore(config.clone(), saved.clone()).unwrap();
    let mut answered_directly = submit_response(&mut direct, &decision_request, 1,
        visible_model_completion("loop-answer", "The inspected file contains the same evidence."));
    // The ordinary post-tool synthesis can request one final publication step;
    // it must not turn a sufficient answer into a mandatory user question.
    if answered_directly.host_requests.iter().any(|request| matches!(request.payload, HostRequest::ModelStream { .. })) {
        answered_directly = submit_response(&mut direct, &model_request(&answered_directly.host_requests), 1,
            visible_model_completion("loop-answer-final", "The inspected file contains the same evidence."));
    }
    assert!(answered_directly.snapshot.open_user_input_waits.is_empty());
    assert!(!answered_directly.host_requests.iter().any(|request|
        matches!(request.payload, HostRequest::ExecuteTool { .. } | HostRequest::ModelStream { .. })));

    let asked = submit_response(
        &mut core,
        &decision_request,
        1,
        empty_model_completion(vec![named_tool_call("primary-loop-decision", "request_user_input", durable_question_arguments("loop-choice"))]),
    );
    let answered = core.submit(answer_wait(&asked.snapshot.open_user_input_waits[0],"continue-loop-response")).unwrap();
    let resumed_model = model_request(&answered.host_requests);
    let resumed = submit_response(
        &mut core,
        &resumed_model,
        1,
        empty_model_completion(vec![named_tool_call(
            "primary-loop-continued",
            "read_file",
            json!({"path":"new-evidence.md"}),
        )]),
    );
    assert!(resumed.host_requests.iter().any(|request| {
        matches!(
            &request.payload,
            HostRequest::ExecuteTool { call_id, .. } if call_id == "primary-loop-continued"
        )
    }));
}

#[test]
fn renamed_exact_writes_enter_recovery_and_allow_new_evidence_after_restore() {
    let mut config = source_core_config();
    config.host_catalog.as_mut().unwrap().capabilities.insert(
        "write_file".into(), capability("write_file", HostCapabilityRoute::Tool, None));
    let mut core = AgentCore::try_new(config.clone()).unwrap();
    let started = core.submit(user_input("copy-loop", "完成文件任务，然后检查输出。", AgentInputDelivery::CurrentTurn)).unwrap();
    let mut model = model_request(&started.host_requests);
    for index in 0..6 {
        let path = format!("/workspace/check-{index}.py");
        let uri = format!("file://{path}");
        let completed = complete_named_tool_round(&mut core, &model, &format!("copy-{index}"),
            "write_file", json!({"path":path,"content":"repeated checker\n".repeat(100)}),
            observed_output(json!({"written":true}), &[(uri.as_str(), "same-content-version")], &[]));
        model = model_request(&completed.host_requests);
        core = AgentCore::restore(config.clone(), core.checkpoint(CheckpointReason::Manual)).unwrap();
    }
    let state = core.checkpoint(CheckpointReason::Manual);
    let turn = state.state.execution.active_turn.as_ref().unwrap();
    assert!(turn.engineering.no_progress_recovery.reason().is_some());
    assert_eq!(turn.engineering.strategy_recovery.model_requests.len(), 1);
    let mut stuck = AgentCore::restore(config.clone(), state.clone()).unwrap();
    let mut retry_model = model.clone();
    for index in 0..config.max_strategy_recovery_attempts {
        let path = format!("/workspace/still-copying-{index}.py");
        let uri = format!("file://{path}");
        let transition = complete_named_tool_round(&mut stuck, &retry_model, &format!("still-copying-{index}"),
            "write_file", json!({"path":path,"content":"repeated checker\n".repeat(100)}),
            observed_output(json!({"written":true}), &[(uri.as_str(), "same-content-version")], &[]));
        if index + 1 < config.max_strategy_recovery_attempts {
            retry_model = model_request(&transition.host_requests);
            stuck = AgentCore::restore(config.clone(), stuck.checkpoint(CheckpointReason::Manual)).unwrap();
        } else {
            assert!(!transition.host_requests.iter().any(|request| matches!(request.payload, HostRequest::ModelStream { .. })));
            assert!(serde_json::to_value(stuck.checkpoint(CheckpointReason::Manual)).unwrap()
                .to_string().contains("execution.no_progress"));
        }
    }
    let observed = complete_named_tool_round(&mut core, &model, "inspect-output", "read_file",
        json!({"path":"/workspace/result.json"}), json!({"text":"output is not present"}));
    assert!(observed.host_requests.iter().any(|request| matches!(request.payload, HostRequest::ModelStream { .. })));
    let state = core.checkpoint(CheckpointReason::Manual);
    let turn = state.state.execution.active_turn.as_ref().unwrap();
    assert!(turn.engineering.no_progress_recovery.reason().is_none());
    assert_eq!(turn.engineering.strategy_recovery.model_requests.len(), 1);
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

#[test]
fn conversation_revert_uses_core_message_ownership_for_regeneration() {
    let first_user = ItemId::from_string("revert-user-1".to_string());
    let first_assistant = ItemId::from_string("revert-assistant-1".to_string());
    let second_user = ItemId::from_string("revert-user-2".to_string());
    let second_assistant = ItemId::from_string("revert-assistant-2".to_string());
    let items = vec![
        AgentItem::user_message(first_user.clone(), "first question"),
        AgentItem::assistant_message(first_assistant.clone(), "first answer"),
        AgentItem::user_message(second_user.clone(), "second question"),
        AgentItem::assistant_message(second_assistant.clone(), "second answer"),
    ];
    let ownership = BTreeMap::from([
        (
            first_user.as_str().to_string(),
            AgentItemOwnership {
                source: Default::default(),
                turn_id: "turn-1".to_string(),
                message_id: "web-user-1".to_string(),
            },
        ),
        (
            first_assistant.as_str().to_string(),
            AgentItemOwnership {
                source: Default::default(),
                turn_id: "turn-1".to_string(),
                message_id: "web-assistant-1".to_string(),
            },
        ),
        (
            second_user.as_str().to_string(),
            AgentItemOwnership {
                source: Default::default(),
                turn_id: "turn-2".to_string(),
                message_id: "web-user-2".to_string(),
            },
        ),
        (
            second_assistant.as_str().to_string(),
            AgentItemOwnership {
                source: Default::default(),
                turn_id: "turn-2".to_string(),
                message_id: "web-assistant-2".to_string(),
            },
        ),
    ]);

    let mut core = AgentCore::try_new(core_config(Vec::new())).expect("create Core");
    let _rebased = core
        .submit(AgentSubmission::RebaseConversation {
            items,
            item_ownership: ownership,
        })
        .expect("seed durable conversation");

    let transition = core
        .submit(AgentSubmission::RevertConversation {
            through_message_id: "web-user-2".to_string(),
            keep_message: true,
            submission_id: Some("revert-submission-1".to_string()),
        })
        .expect("revert through the second user message");
    assert_eq!(transition.snapshot.history_item_count, 3);
    assert_eq!(transition.snapshot.model_context_item_count, 3);

    let duplicate = core
        .submit(AgentSubmission::RevertConversation {
            through_message_id: "web-user-2".to_string(),
            keep_message: true,
            submission_id: Some("revert-submission-1".to_string()),
        })
        .expect("duplicate revert is idempotent");
    assert_eq!(duplicate.snapshot.history_item_count, 3);
}

#[test]
fn branch_checkpoint_requires_a_complete_turn_and_preserves_parent_lineage() {
    let user_id = ItemId::from_string("parent-user".to_string());
    let assistant_id = ItemId::from_string("parent-assistant".to_string());
    let items = vec![
        AgentItem::user_message(user_id.clone(), "inspect this repository"),
        AgentItem::assistant_message(assistant_id.clone(), "Here is the result."),
    ];
    let ownership = BTreeMap::from([
        (
            user_id.as_str().to_string(),
            AgentItemOwnership {
                source: Default::default(),
                turn_id: "parent:turn:1".to_string(),
                message_id: user_id.as_str().to_string(),
            },
        ),
        (
            assistant_id.as_str().to_string(),
            AgentItemOwnership {
                source: Default::default(),
                turn_id: "parent:turn:1".to_string(),
                message_id: "parent:turn:1:assistant".to_string(),
            },
        ),
    ]);

    let mut core = AgentCore::try_new(core_config(Vec::new())).expect("create Core");
    let _rebased = core
        .submit(AgentSubmission::RebaseConversation {
            items,
            item_ownership: ownership,
        })
        .expect("rebase parent conversation");

    let partial_turn = core.branch_checkpoint("branch-session", user_id.as_str());
    assert!(
        partial_turn.is_err(),
        "a branch cannot split a durable Turn"
    );

    let checkpoint = core
        .branch_checkpoint("branch-session", assistant_id.as_str())
        .expect("create branch checkpoint at completed Turn boundary");
    assert_eq!(
        checkpoint.state.session_id,
        SessionId::from_string("branch-session".to_string())
    );
    assert_eq!(checkpoint.state.conversation.transcript.len(), 2);
    assert_eq!(checkpoint.state.item_ownership.len(), 2);
    let lineage = checkpoint
        .state
        .lineage
        .as_ref()
        .expect("branch lineage must be durable");
    assert_eq!(lineage.through_item_id, assistant_id.as_str());

    let parent = core.snapshot();
    assert_ne!(parent.session_id, checkpoint.state.session_id);
    assert_eq!(parent.history_item_count, 2);

    let grandchild = AgentCore::branch_checkpoint_from_checkpoint(
        &checkpoint,
        "grandchild-session",
        "parent:turn:1:assistant",
    )
    .expect("branch from persisted checkpoint by Core message identity");
    assert_eq!(
        grandchild.state.session_id,
        SessionId::from_string("grandchild-session".to_string())
    );
    assert_eq!(
        grandchild
            .state
            .lineage
            .as_ref()
            .expect("grandchild lineage")
            .parent_session_id,
        "branch-session"
    );
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

#[test]
fn host_capability_schema_enforces_numeric_bounds_before_execution() {
    let mut descriptor = capability("bounded", HostCapabilityRoute::Tool, None);
    descriptor.input_schema = json!({
        "type": "object",
        "properties": {
            "context_lines": {"type": "integer", "minimum": 0, "maximum": 20}
        },
        "additionalProperties": false
    });

    assert!(descriptor
        .validate_input(&json!({"context_lines": 20}))
        .is_ok());
    let error = descriptor
        .validate_input(&json!({"context_lines": 30}))
        .expect_err("out-of-range model input must not reach the Host executor");
    assert!(error.contains("$.context_lines exceeds the maximum value 20"));
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

#[test]
fn text_only_model_contract_controls_the_frozen_projection_and_guidance() {
    for prompt in [
        "你好",
        "你编译下",
        "search current",
        "Continue",
        "权限模型怎么设计",
    ] {
        let mut config = engineering_core_config();
        config.turn_identity.model_capability = nola_core::agent::ModelCapabilityProfile {
            profile_id: "explicit-text-only".into(),
            revision: 1,
            native_tool_calls: false,
            stream_text: true,
            ..Default::default()
        };
        let mut core = AgentCore::try_new(config).unwrap();
        let started = core
            .submit(user_input(
                "text-only-user",
                prompt,
                AgentInputDelivery::CurrentTurn,
            ))
            .unwrap();
        let request = model_request(&started.host_requests);
        let HostRequest::ModelStream {
            tools,
            messages,
            settings,
            ..
        } = request.payload
        else {
            panic!("expected a model step");
        };
        assert!(
            tools.is_empty(),
            "Host schemas cannot bypass the model contract"
        );
        assert_eq!(
            settings["model_invocation_plan"]["native_tool_calls"],
            false
        );
        let step: nola_core::agent::protocol::AgentStepContextSnapshot =
            serde_json::from_value(settings["step_context"].clone()).unwrap();
        assert!(step.tool_projection.capability_ids.is_empty());
        let guidance = serde_json::to_string(&messages).unwrap();
        assert!(guidance.contains("selected model route has tool calls disabled"));
        assert!(guidance.contains("No command execution, file access, tool discovery"));
        assert!(!started.host_requests.iter().any(|request| matches!(
            request.payload,
            HostRequest::ExecuteTool { .. } | HostRequest::RequestPermission { .. }
        )));
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

#[test]
fn no_prompt_capability_still_obeys_explicit_deny_and_ask() {
    use nola_core::agent::permissions::{
        PermissionPolicyDecision, PermissionRule, PermissionRuleSource,
    };
    for decision in [
        PermissionPolicyDecision::Deny,
        PermissionPolicyDecision::Ask,
    ] {
        let mut config = engineering_core_config();
        config.permission_rules.rules.push(PermissionRule {
            capability_pattern: "bash".into(),
            resource_pattern: "*".into(),
            decision,
            source: PermissionRuleSource::UserGlobal,
        });
        let mut core = AgentCore::try_new(config).unwrap();
        let started = core
            .submit(user_input(
                "permission-contract",
                "inspect",
                AgentInputDelivery::CurrentTurn,
            ))
            .unwrap();
        let request = model_request(&started.host_requests);
        let result = submit_response(
            &mut core,
            &request,
            1,
            empty_model_completion(vec![named_tool_call(
                "inspect-1",
                "bash",
                json!({"command":"pwd", "operation_kind":"inspect"}),
            )]),
        );
        assert!(!result
            .host_requests
            .iter()
            .any(|request| matches!(request.payload, HostRequest::ExecuteTool { .. })));
        assert_eq!(
            result
                .host_requests
                .iter()
                .any(|request| matches!(request.payload, HostRequest::RequestPermission { .. })),
            decision == PermissionPolicyDecision::Ask
        );
    }
}

#[test]
fn conflicting_execution_receipt_does_not_consume_pending_response() {
    let mut core = AgentCore::try_new(engineering_core_config()).unwrap();
    let started = core
        .submit(user_input(
            "receipt-contract",
            "inspect",
            AgentInputDelivery::CurrentTurn,
        ))
        .unwrap();
    let request = model_request(&started.host_requests);
    let scheduled = submit_response(
        &mut core,
        &request,
        1,
        empty_model_completion(vec![named_tool_call(
            "receipt-call",
            "bash",
            json!({"command":"pwd", "operation_kind":"inspect"}),
        )]),
    );
    let execution = request_matching(&scheduled.host_requests, |request| {
        matches!(request, HostRequest::ExecuteTool { .. })
    });
    let before = serde_json::to_value(core.checkpoint(CheckpointReason::Manual).state).unwrap();
    let mut output = json!({"contract":"nola.tool_result.v2", "data":{"text":"done"},
        "presentation":{"text":"done","summary":"done"},
        "execution":{"state":"not_started", "local_stop":null}, "full_output_ref":null,"memory_usage":null});
    let response = |output| HostResponse::ToolCompleted {
        call_id: "receipt-call".into(),
        output,
        outcome: nola_host_protocol::HostToolOutcome::Succeeded,
        activation: None,
        full_output_ref: None,
        artifacts: Vec::new(),
    };
    assert!(core
        .submit(AgentSubmission::HostResponse {
            response: response_for(&execution, 1, response(output.clone()))
        })
        .is_err());
    assert_eq!(
        serde_json::to_value(core.checkpoint(CheckpointReason::Manual).state).unwrap(),
        before
    );
    output["execution"]["state"] = json!("completed");
    submit_response(&mut core, &execution, 1, response(output));
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

#[test]
fn invalid_host_artifact_is_rejected_before_consuming_the_pending_effect() {
    let mut core = AgentCore::try_new(source_core_config()).expect("create Core");
    let started = core
        .submit(user_input(
            "artifact-contract-user",
            "Read the requested file.",
            AgentInputDelivery::CurrentTurn,
        ))
        .expect("start turn");
    let model = model_request(&started.host_requests);
    let requested = submit_response(
        &mut core,
        &model,
        1,
        empty_model_completion(vec![named_tool_call(
            "artifact-contract-call",
            "read_file",
            json!({"path": "/tmp/Nola report.txt"}),
        )]),
    );
    let tool = request_matching(
        &requested.host_requests,
        |payload| matches!(payload, HostRequest::ExecuteTool { call_id, .. } if call_id == "artifact-contract-call"),
    );
    let invalid = core.submit(AgentSubmission::HostResponse {
        response: terminal_host_response(
            &tool,
            HostResponse::ToolCompleted {
                call_id: "artifact-contract-call".to_string(),
                output: json!({"text": "hello"}),
                outcome: nola_core::agent::host::HostToolOutcome::Succeeded,
                activation: None,
                full_output_ref: None,
                artifacts: vec![HostArtifact {
                    uri: "/tmp/Nola report.txt".to_string(),
                    name: "Nola report.txt".to_string(),
                    mime_type: Some("text/plain".to_string()),
                    operation: Some("read".to_string()),
                }],
            },
        ),
    });
    assert_eq!(
        invalid.expect_err("raw paths must be rejected"),
        "Host artifact URI must be a canonical absolute URI"
    );

    let completed = core
        .submit(AgentSubmission::HostResponse {
            response: terminal_host_response(
                &tool,
                HostResponse::ToolCompleted {
                    call_id: "artifact-contract-call".to_string(),
                    output: json!({"text": "hello"}),
                    outcome: nola_core::agent::host::HostToolOutcome::Succeeded,
                    activation: None,
                    full_output_ref: None,
                    artifacts: vec![HostArtifact {
                        uri: "file:///tmp/Nola%20report.txt".to_string(),
                        name: "Nola report.txt".to_string(),
                        mime_type: Some("text/plain".to_string()),
                        operation: Some("read".to_string()),
                    }],
                },
            ),
        })
        .expect("the pending effect remains available for a corrected response");
    assert!(completed.snapshot.projection.parts.values().any(|part| {
        matches!(
            part,
            AgentPart::Tool(tool)
                if tool.artifacts.iter().any(|artifact| artifact.uri == "file:///tmp/Nola%20report.txt")
        )
    }));
}

#[test]
fn session_close_is_core_owned_durable_and_rejects_new_input() {
    let mut core = AgentCore::try_new(core_config(Vec::new())).expect("create Core");
    let closing = core
        .submit(AgentSubmission::CloseSession {
            submission_id: Some("close-session-contract".to_string()),
        })
        .expect("begin session close");
    assert!(closing.snapshot.session_closing);
    assert!(!closing.snapshot.session_closed);

    let rejected = core.submit(user_input(
        "late-user",
        "this input arrived after shutdown began",
        AgentInputDelivery::CurrentTurn,
    ));
    assert!(rejected.is_err(), "closing sessions must reject new Turns");

    let lifecycle_request = closing
        .host_requests
        .iter()
        .find(|request| matches!(request.payload, HostRequest::RunLifecycleHook { .. }))
        .expect("Core must emit the SessionEnd lifecycle effect")
        .clone();
    let repeated_close = core
        .submit(AgentSubmission::CloseSession {
            submission_id: Some("close-session-contract-retry".to_string()),
        })
        .expect("retry closing session");
    let repeated_lifecycle_requests = repeated_close
        .host_requests
        .iter()
        .filter(|request| matches!(request.payload, HostRequest::RunLifecycleHook { .. }))
        .collect::<Vec<_>>();
    assert!(
        repeated_lifecycle_requests.is_empty(),
        "a close retry must not automatically replay a non-idempotent lifecycle effect"
    );
    let closed = core
        .submit(AgentSubmission::HostResponse {
            response: terminal_host_response(
                &lifecycle_request,
                HostResponse::Completed {
                    output: json!({"lifecycle": "session_end"}),
                    usage: None,
                    full_output_ref: None,
                    artifacts: Vec::new(),
                },
            ),
        })
        .expect("complete session close");
    assert!(!closed.snapshot.session_closing);
    assert!(closed.snapshot.session_closed);
    let repeated_closed = core
        .submit(AgentSubmission::CloseSession {
            submission_id: Some("close-session-contract-after-closed".to_string()),
        })
        .expect("closed session shutdown is idempotent");
    assert!(repeated_closed.snapshot.session_closed);
    assert!(repeated_closed.host_requests.is_empty());

    let checkpoint = checkpoint_after(&core, &closed);
    let restored = AgentCore::restore(core_config(Vec::new()), checkpoint)
        .expect("restore closed session checkpoint");
    assert!(restored.snapshot().session_closed);
    assert!(!restored.snapshot().session_closing);
}

#[test]
fn activation_missing_descriptor_rejects_without_consuming_the_host_response() {
    let (mut core, tool) = pending_capability_activation();
    let before = serde_json::to_value(core.checkpoint(CheckpointReason::Manual).state).unwrap();
    let fixture: Value = serde_json::from_str(include_str!(
        "fixtures/capability_activation_v1.json"
    )).unwrap();
    let valid: HostResponse = serde_json::from_value(fixture["typed_host_response"].clone()).unwrap();
    let mut invalid = valid.clone();
    if let HostResponse::ToolCompleted { activation: Some(activation), .. } = &mut invalid {
        activation.capability_descriptors.pop();
    } else {
        panic!("expected typed activation");
    }
    let error = core.submit(AgentSubmission::HostResponse {
        response: response_for(&tool, 1, invalid),
    }).expect_err("history must not fill a missing current descriptor");
    assert!(error.to_string().contains("did not provide descriptor"), "{error}");
    assert_eq!(serde_json::to_value(core.checkpoint(CheckpointReason::Manual).state).unwrap(), before);
    let accepted = submit_response(&mut core, &tool, 1, valid);
    assert_eq!(accepted.event_envelopes.iter().filter(|envelope| matches!(
        envelope.event, AgentEvent::CapabilitiesActivated { .. }
    )).count(), 1);
    model_request(&accepted.host_requests);
}

#[test]
fn typed_host_activation_is_authoritative_and_emits_one_core_event() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/capability_activation_v1.json"))
            .expect("decode shared activation fixture");
    let response: HostResponse = serde_json::from_value(fixture["typed_host_response"].clone())
        .expect("decode typed Host response");
    let transition = capability_activation_transition(response);
    let activations = transition
        .event_envelopes
        .iter()
        .filter_map(|envelope| match &envelope.event {
            AgentEvent::CapabilitiesActivated {
                source,
                capability_ids,
                ..
            } => Some((source.as_str(), capability_ids.as_slice())),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(activations.len(), 1);
    assert_eq!(activations[0].0, "tool_search");
    assert_eq!(activations[0].1, ["web_search", "web_fetch"]);
    assert!(!activations[0].1.iter().any(|id| id == "obsolete_only"));

    let next = model_request(&transition.host_requests);
    let HostRequest::ModelStream { tools, messages, settings, .. } = &next.payload else {
        panic!("successful capability discovery must continue with a model request")
    };
    let exposed = tools
        .iter()
        .filter_map(|schema| schema.pointer("/function/name").and_then(Value::as_str))
        .collect::<Vec<_>>();
    assert!(exposed.contains(&"web_search"), "{exposed:?}");
    assert!(exposed.contains(&"web_fetch"), "{exposed:?}");
    assert!(exposed.windows(2).all(|pair| pair[0] < pair[1]),
        "the actual emitted schemas, not only their digest, must have stable order");
    let projection: nola_core::agent::capability_projection::AgentModelToolProjection =
        serde_json::from_value(settings["step_context"]["tool_projection"].clone()).unwrap();
    assert!(projection.matches_model_definitions(tools));
    assert!(messages.iter().all(|message| !message.to_string().contains("_nola_available_capabilities")),
        "discovery must not rewrite prior tool evidence with the current live catalog");
}

#[test]
fn obsolete_activation_text_cannot_change_the_core_catalog() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/capability_activation_v1.json"))
            .expect("decode shared activation fixture");
    let response: HostResponse = serde_json::from_value(fixture["obsolete_host_response"].clone())
        .expect("decode obsolete Host response");
    let transition = capability_activation_transition(response);
    let activations = transition
        .event_envelopes
        .iter()
        .filter_map(|envelope| match &envelope.event {
            AgentEvent::CapabilitiesActivated {
                source,
                capability_ids,
                ..
            } => Some((source.as_str(), capability_ids.as_slice())),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert!(activations.is_empty());
}

#[test]
fn failed_host_result_exposes_typed_preservation_recovery_in_next_request() {
    let response = HostResponse::ToolCompleted {
        call_id: "activation-call".into(),
        output: json!({"text": "Host resource preservation denied the operation"}),
        outcome: nola_core::agent::host::HostToolOutcome::Failed,
        activation: Some(nola_core::agent::host::HostCapabilityActivation {
            source: "host_resource_preservation".into(),
            capability_ids: vec!["web_fetch".into()],
            capability_descriptors: vec![serde_json::to_value(capability(
                "web_fetch", HostCapabilityRoute::Tool, None,
            )).unwrap()],
        }),
        full_output_ref: None,
        artifacts: Vec::new(),
    };
    let transition = capability_activation_transition(response);
    assert!(transition.event_envelopes.iter().any(|envelope| matches!(
        &envelope.event,
        AgentEvent::CapabilitiesActivated { source, capability_ids, .. }
            if source == "host_resource_preservation" && capability_ids == &["web_fetch"]
    )));
    let next = model_request(&transition.host_requests);
    let HostRequest::ModelStream { tools, .. } = &next.payload else {
        panic!("failed operation must offer a model recovery step")
    };
    assert!(tools.iter().any(|schema|
        schema.pointer("/function/name").and_then(Value::as_str) == Some("web_fetch")
    ));
}

#[test]
fn model_tool_projection_obeys_host_catalog_exposure() {
    let mut config = core_config(Vec::new());
    let catalog = config.host_catalog.as_mut().expect("host catalog");

    for (name, exposure) in [
        ("visible_tool", HostCapabilityExposure::Visible),
        ("deferred_tool", HostCapabilityExposure::Deferred),
        ("internal_tool", HostCapabilityExposure::Internal),
        ("hidden_tool", HostCapabilityExposure::Hidden),
    ] {
        let mut descriptor = capability(name, HostCapabilityRoute::Tool, None);
        descriptor.exposure = exposure;
        catalog.capabilities.insert(name.to_string(), descriptor);
        config.tools.push(json!({
            "type": "function",
            "function": {
                "name": name,
                "description": name,
                "parameters": {"type": "object"}
            }
        }));
    }

    let mut unavailable = capability("unavailable_tool", HostCapabilityRoute::Tool, None);
    unavailable.availability = CapabilityAvailability::Unavailable;
    catalog
        .capabilities
        .insert("unavailable_tool".to_string(), unavailable);
    config.tools.push(json!({
        "type": "function",
        "function": {
            "name": "unavailable_tool",
            "parameters": {"type": "object"}
        }
    }));

    let mut internal_model = capability("internal_model", HostCapabilityRoute::Model, None);
    internal_model.exposure = HostCapabilityExposure::Visible;
    catalog
        .capabilities
        .insert("internal_model".to_string(), internal_model);
    config.tools.push(json!({
        "type": "function",
        "function": {
            "name": "internal_model",
            "parameters": {"type": "object"}
        }
    }));

    let mut core = AgentCore::try_new(config).expect("create Core");
    let transition = core
        .submit(user_input(
            "user-visible-tools",
            "show available tools",
            AgentInputDelivery::CurrentTurn,
        ))
        .expect("start turn");
    let request = model_request(&transition.host_requests);
    let (tools, settings) = match request.payload {
        HostRequest::ModelStream {
            tools, settings, ..
        } => (tools, settings),
        _ => unreachable!(),
    };
    let names = tools
        .iter()
        .filter_map(|tool| {
            tool.get("function")
                .and_then(|function| function.get("name"))
                .or_else(|| tool.get("name"))
                .and_then(serde_json::Value::as_str)
        })
        .collect::<Vec<_>>();

    assert!(names.contains(&"write_file"));
    assert!(names.contains(&"visible_tool"));
    assert!(!names.contains(&"deferred_tool"));
    assert!(!names.contains(&"internal_tool"));
    assert!(!names.contains(&"hidden_tool"));
    assert!(!names.contains(&"unavailable_tool"));
    assert!(!names.contains(&"internal_model"));
    assert_eq!(
        settings
            .get("defer_text_presentation")
            .and_then(Value::as_bool),
        Some(false),
        "ordinary model text must stream even when tools are available"
    );
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

#[test]
fn durable_history_rebase_drops_prior_turn_tool_transactions() {
    let call_id = nola_core::agent::CallId::from_string("old-call".to_string());
    let old_history = vec![
        AgentItem::user_message(
            ItemId::from_string("old-user".to_string()),
            "write the first file",
        ),
        AgentItem::ToolCall {
            id: ItemId::from_string("old-tool-call".to_string()),
            call_id: call_id.clone(),
            name: "write_file".to_string(),
            arguments: json!({"path": "/tmp/first.txt"}),
        },
        AgentItem::ToolResult {
            id: ItemId::from_string("old-tool-result".to_string()),
            call_id,
            name: "write_file".to_string(),
            output: json!({"path": "/tmp/first.txt", "status": "completed"}),
            is_error: false,
            full_output_ref: None,
            artifacts: Vec::new(),
        },
        AgentItem::assistant_message(
            ItemId::from_string("old-assistant".to_string()),
            "The first file is ready.",
        ),
    ];
    let mut core = AgentCore::try_new(core_config(old_history)).expect("create Core");
    let durable_history = vec![
        AgentItem::user_message(
            ItemId::from_string("durable-user".to_string()),
            "write the first file",
        ),
        AgentItem::assistant_message(
            ItemId::from_string("durable-assistant".to_string()),
            "The first file is ready.",
        ),
    ];

    let rebased = core
        .submit(AgentSubmission::RebaseConversation {
            items: durable_history,
            item_ownership: Default::default(),
        })
        .expect("rebase idle conversation");
    assert_eq!(rebased.snapshot.history_item_count, 2);
    assert_eq!(rebased.snapshot.model_context_item_count, 2);

    let started = core
        .submit(user_input(
            "second-user",
            "write the second file",
            AgentInputDelivery::CurrentTurn,
        ))
        .expect("start second turn");
    let request = model_request(&started.host_requests);
    let messages = match request.payload {
        HostRequest::ModelStream { messages, .. } => messages,
        _ => unreachable!(),
    };
    assert!(
        messages.iter().all(|message| {
            message.get("role").and_then(serde_json::Value::as_str) != Some("tool")
                && !message.to_string().contains("/tmp/first.txt")
        }),
        "prior Turn tool transaction leaked into the next model request"
    );
    assert!(messages.iter().any(|message| {
        message.get("content").and_then(serde_json::Value::as_str) == Some("write the second file")
    }));
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

#[test]
fn oversized_model_tool_batch_is_rejected_atomically_before_execution() {
    let mut core = AgentCore::try_new(core_config(Vec::new())).unwrap();
    let started = core
        .submit(user_input(
            "oversized-batch",
            "审计未提交变更",
            AgentInputDelivery::CurrentTurn,
        ))
        .unwrap();
    let request = model_request(&started.host_requests);
    let calls = (0..121)
        .map(|index| ModelToolCall {
            call_id: CallId::from_string(format!("call-{index}")),
            name: "read_file".into(),
            arguments: json!({"path": format!("file-{index}.rs")}),
            permission_scope: None,
            permission_rationale: None,
        })
        .collect();
    let rejected = submit_response(&mut core, &request, 1, empty_model_completion(calls));
    assert!(
        rejected.host_requests.is_empty(),
        "no prefix of the batch may execute"
    );
    assert!(!rejected
        .snapshot
        .projection
        .parts
        .values()
        .any(|part| matches!(part, AgentPart::Tool(_))));
    assert!(rejected.event_envelopes.iter().any(|envelope| matches!(
        &envelope.event, AgentEvent::TurnFailed { error, .. }
        if error == nola_core::agent::model_tool_batch::MODEL_TOOL_BATCH_LIMIT_EXCEEDED
    )));
}

#[test]
fn streamed_tool_batch_overflow_settles_drafts_without_allocating_another_part() {
    use nola_core::agent::model_tool_batch::MAX_MODEL_TOOL_CALLS_PER_BATCH;
    let mut core = AgentCore::try_new(core_config(Vec::new())).unwrap();
    let started = core
        .submit(user_input(
            "oversized-stream",
            "inspect files",
            AgentInputDelivery::CurrentTurn,
        ))
        .unwrap();
    let request = model_request(&started.host_requests);
    for index in 0..=MAX_MODEL_TOOL_CALLS_PER_BATCH {
        let transition = submit_response(
            &mut core,
            &request,
            (index + 1) as u64,
            HostResponse::ModelToolCallDelta {
                part_id: format!("part-{index}"),
                call_id: format!("call-{index}"),
                capability_id: "read_file".into(),
                arguments_delta: "{".into(),
            },
        );
        assert!(transition.host_requests.is_empty());
        if index == MAX_MODEL_TOOL_CALLS_PER_BATCH {
            let tools = transition
                .snapshot
                .projection
                .parts
                .values()
                .filter_map(|part| match part {
                    AgentPart::Tool(tool) => Some(tool),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(tools.len(), MAX_MODEL_TOOL_CALLS_PER_BATCH);
            assert!(tools.iter().all(|tool| tool.state.is_terminal()));
            assert!(transition
                .event_envelopes
                .iter()
                .any(|envelope| matches!(envelope.event, AgentEvent::TurnFailed { .. })));
        }
    }
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

#[test]
fn duplicate_batch_calls_settle_individually_and_survive_checkpoint() {
    let config = source_core_config();
    let mut core = AgentCore::try_new(config.clone()).unwrap();
    let started = core
        .submit(user_input(
            "duplicate-user",
            "检查文件 / current search results",
            AgentInputDelivery::CurrentTurn,
        ))
        .unwrap();
    let request = model_request(&started.host_requests);
    let calls = ["original", "duplicate"]
        .into_iter()
        .map(|id| ModelToolCall {
            call_id: CallId::from_string(id.to_string()),
            name: "read_file".into(),
            arguments: json!({"path": "file.rs"}),
            permission_scope: None,
            permission_rationale: None,
        })
        .collect();
    let scheduled = submit_response(&mut core, &request, 1, empty_model_completion(calls));
    assert_eq!(
        scheduled.host_requests.len(),
        1,
        "only the original is executed"
    );
    let checkpoint = serde_json::from_slice(
        &serde_json::to_vec(&core.checkpoint(CheckpointReason::Manual)).unwrap(),
    )
    .unwrap();
    core = AgentCore::restore(config, checkpoint).unwrap();
    let completed = complete_tool(&mut core, &scheduled.host_requests[0], "original");
    let tools = completed
        .snapshot
        .projection
        .parts
        .values()
        .filter_map(|part| match part {
            AgentPart::Tool(tool) => Some(tool),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        tools.len(),
        2,
        "duplicate IDs are not silently removed from the transcript"
    );
    assert!(tools.iter().all(|tool| tool.state.is_terminal()));
    let model = model_request(&completed.host_requests);
    let serialized = serde_json::to_string(&model).unwrap();
    assert!(serialized.contains("duplicate_tool_call_in_batch"));
    assert!(serialized.contains("tool_execution_skipped"));
}

#[test]
fn budget_rejection_keeps_call_and_result_without_host_execution() {
    let mut config = source_core_config();
    config
        .host_catalog
        .as_mut()
        .unwrap()
        .capabilities
        .get_mut("read_file")
        .unwrap()
        .input_normalization
        .turn_budget = Some(nola_core::agent::host::HostCapabilityTurnBudget {
        group: "fixture_reads".into(),
        standard_max_calls: 1,
        finalize_on_exhaustion: true,
        expansion: None,
    });
    let mut core = AgentCore::try_new(config).unwrap();
    let started = core
        .submit(user_input(
            "budget-user",
            "Read two files",
            AgentInputDelivery::CurrentTurn,
        ))
        .unwrap();
    let request = model_request(&started.host_requests);
    let calls = ["first", "over-budget"]
        .into_iter()
        .map(|id| ModelToolCall {
            call_id: CallId::from_string(id.to_string()),
            name: "read_file".into(),
            arguments: json!({"path": format!("{id}.rs")}),
            permission_scope: None,
            permission_rationale: None,
        })
        .collect();
    let scheduled = submit_response(&mut core, &request, 1, empty_model_completion(calls));
    assert_eq!(scheduled.host_requests.len(), 1);
    let completed = complete_tool(&mut core, &scheduled.host_requests[0], "first");
    let tools = completed
        .snapshot
        .projection
        .parts
        .values()
        .filter_map(|part| match part {
            AgentPart::Tool(tool) => Some(tool),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(tools.len(), 2);
    assert!(tools.iter().all(|tool| tool.state.is_terminal()));
    let model = model_request(&completed.host_requests);
    let serialized = serde_json::to_string(&model).unwrap();
    assert!(serialized.contains("capability_budget_exhausted"));
    assert!(serialized.contains("over-budget"));
    assert!(serialized.contains("tool_execution_skipped"));
}

#[test]
fn a_parallel_invalid_batch_can_recover_without_spending_many_correction_rounds() {
    let mut config = source_core_config();
    config
        .host_catalog
        .as_mut()
        .unwrap()
        .capabilities
        .get_mut("read_file")
        .unwrap()
        .input_schema = json!({
        "type":"object", "properties":{"path":{"type":"string"}, "max_bytes":{"type":"integer","maximum":6000}},
        "required":["path"]
    });
    let mut core = AgentCore::try_new(config).unwrap();
    let started = core
        .submit(user_input(
            "batch-correction",
            "审计这些文件 / review these files",
            AgentInputDelivery::CurrentTurn,
        ))
        .unwrap();
    let calls = |prefix: &str, count: usize, max_bytes: usize| {
        (0..count)
            .map(|index| ModelToolCall {
                call_id: CallId::from_string(format!("{prefix}-{index}")),
                name: "read_file".into(),
                arguments: json!({"path":format!("file-{index}.rs"), "max_bytes":max_bytes}),
                permission_scope: None,
                permission_rationale: None,
            })
            .collect()
    };
    let rejected = submit_response(
        &mut core,
        &model_request(&started.host_requests),
        1,
        empty_model_completion(calls("bad", 7, 12000)),
    );
    let state = serde_json::to_value(core.checkpoint(CheckpointReason::Manual)).unwrap();
    assert_eq!(
        state.pointer("/state/execution/active_turn/engineering/correction_count_since_progress"),
        Some(&json!(1))
    );
    let mut executing = submit_response(
        &mut core,
        &model_request(&rejected.host_requests),
        1,
        empty_model_completion(calls("fixed", 7, 6000)),
    );
    for index in 0..7 {
        assert_eq!(executing.host_requests.len(), 1);
        executing = complete_tool(
            &mut core,
            &executing.host_requests[0],
            &format!("fixed-{index}"),
        );
    }
    let state = serde_json::to_value(core.checkpoint(CheckpointReason::Manual)).unwrap();
    assert_eq!(
        state.pointer("/state/execution/active_turn/engineering/correction_count_since_progress"),
        Some(&json!(0))
    );
    let final_rejection = submit_response(
        &mut core,
        &model_request(&executing.host_requests),
        1,
        empty_model_completion(calls("new-error", 1, 12000)),
    );
    let state = serde_json::to_value(core.checkpoint(CheckpointReason::Manual)).unwrap();
    assert_eq!(
        state.pointer("/state/execution/active_turn/engineering/no_progress_recovery/state"),
        Some(&json!("inactive"))
    );
    let _ = model_request(&final_rejection.host_requests);
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

#[test]
fn model_iteration_budget_survives_checkpoint_and_cannot_be_evaded_by_new_arguments() {
    let mut config = core_config(Vec::new());
    config.max_model_iterations = 2;
    config
        .permission_rules
        .rules
        .push(nola_core::agent::permissions::PermissionRule {
            capability_pattern: "*".to_string(),
            resource_pattern: "*".to_string(),
            decision: nola_core::agent::permissions::PermissionPolicyDecision::Allow,
            source: nola_core::agent::permissions::PermissionRuleSource::UserGlobal,
        });
    let (core, final_model) = reach_iteration_budget_synthesis(&config);
    let HostRequest::ModelStream {
        tools, messages, ..
    } = &final_model.payload
    else {
        panic!("expected final model request")
    };
    assert_eq!(model_tool_names(tools), vec!["requirements_update"]);
    assert!(messages.iter().any(|message| {
        message
            .get("content")
            .and_then(Value::as_str)
            .is_some_and(|content| content.contains("No further external actions are available"))
    }));

    let wire = serde_json::to_vec(&core.checkpoint(CheckpointReason::Manual)).unwrap();
    let checkpoint: CoreCheckpoint<AgentThreadState> = serde_json::from_slice(&wire).unwrap();
    config.turn_identity = checkpoint.state.configuration.turn_identity.clone();
    let mut restored = AgentCore::restore(config, checkpoint).expect("restore bounded Core");
    let resumed = restored
        .resume_committed_host_effect(&final_model)
        .expect("resume final synthesis effect");
    let pending = model_request(&resumed.host_requests);
    assert_eq!(pending.effect_id, final_model.effect_id);
    let failed = submit_response(
        &mut restored,
        &pending,
        1,
        visible_model_completion(
            "bounded-answer",
            "Completed work is recorded; validation remains incomplete.",
        ),
    );
    assert_eq!(
        failed.snapshot.turn_status,
        nola_core::agent::AgentTurnStatus::Completed
    );
    assert!(failed.event_envelopes.iter().any(|event| {
        matches!(&event.event, AgentEvent::RunResultDecided { decision, .. }
            if matches!(&decision.value, nola_domain::run_result::RunResultValue::Incomplete { reason, .. }
                if reason.code == "verification.pending"))
    }));
    assert!(failed.snapshot.projection.parts.values().any(|part| {
        matches!(part, AgentPart::Text(text)
            if text.text.contains("validation remains incomplete") && text.is_complete)
    }));
}

#[test]
fn compaction_at_execution_budget_does_not_consume_the_final_sampling() {
    let mut config = source_core_config();
    config.max_model_iterations = 1;
    let mut core = AgentCore::try_new(config.clone()).unwrap();
    let started = core
        .submit(user_input(
            "compacting-review",
            "Review the evidence",
            AgentInputDelivery::CurrentTurn,
        ))
        .unwrap();
    let planned = submit_response(
        &mut core,
        &model_request(&started.host_requests),
        1,
        empty_model_completion(vec![named_tool_call(
            "last-search",
            "web_search",
            json!({"query":"evidence"}),
        )]),
    );
    let search = request_matching(&planned.host_requests, |payload| {
        matches!(payload,
        HostRequest::ExecuteTool { call_id, .. } if call_id == "last-search")
    });
    let mut checkpoint = core.checkpoint(CheckpointReason::Manual);
    checkpoint
        .state
        .conversation
        .context_window
        .config
        .auto_compact_tokens = Some(1);
    core = AgentCore::restore(config, checkpoint).unwrap();
    let mut observed = complete_tool(&mut core, &search, "last-search");
    if let Some(compact) = observed
        .host_requests
        .iter()
        .find(|request| matches!(request.payload, HostRequest::Compact { .. }))
        .cloned()
    {
        observed = submit_response(
            &mut core,
            &compact,
            1,
            HostResponse::CompactionCompleted {
                summary: "Retained search evidence; only the final report remains.".into(),
                usage: json!({}),
            },
        );
    }
    let final_request = model_request(&observed.host_requests);
    assert!(
        matches!(final_request.payload, HostRequest::ModelStream { ref tools, .. } if model_tool_names(tools) == vec!["requirements_update"])
    );
    let checkpoint = core.checkpoint(CheckpointReason::Manual);
    let turn = checkpoint.state.execution.active_turn.unwrap();
    assert_eq!(
        turn.model_request_count, 2,
        "compaction is not a model sampling"
    );
    assert!(turn.iteration_budget_final_synthesis);
}

#[test]
fn tool_planning_receives_remaining_execution_budget_before_the_final_boundary() {
    let mut config = source_core_config();
    config.max_model_iterations = 3;
    let mut core = AgentCore::try_new(config).unwrap();
    let started = core
        .submit(user_input(
            "budget-awareness",
            "Review several requested areas",
            AgentInputDelivery::CurrentTurn,
        ))
        .unwrap();
    let planned = submit_response(
        &mut core,
        &model_request(&started.host_requests),
        1,
        empty_model_completion(vec![named_tool_call(
            "initial-evidence",
            "web_search",
            json!({"query":"evidence"}),
        )]),
    );
    let search = request_matching(&planned.host_requests, |payload| {
        matches!(payload,
        HostRequest::ExecuteTool { call_id, .. } if call_id == "initial-evidence")
    });
    let observed = complete_tool(&mut core, &search, "initial-evidence");
    let request = model_request(&observed.host_requests);
    let HostRequest::ModelStream { messages, .. } = request.payload else {
        panic!("expected model")
    };
    let prompt = serde_json::to_string(&messages).unwrap();
    let facts = messages.iter().filter_map(|message| {
        let content = message["content"].as_str()?;
        let (_, payload) = content.split_once('\n')?;
        serde_json::from_str::<Value>(payload).ok()
    }).find(|value| value["kind"] == "execution-budget").expect("typed budget facts");
    assert_eq!(facts["facts"]["decision"], 2);
    assert_eq!(facts["facts"]["maximum"], 3);
    assert_eq!(facts["facts"]["remaining_after_request"], 1);
    assert!(prompt.contains("every area the user explicitly requested"));
    assert!(!prompt.contains("tool-capable execution budget is closed"));
}

#[test]
fn investigation_budget_finishes_directly_without_reopening_tools_or_review() {
    let mut config = source_core_config();
    config.max_model_iterations = 1;
    let mut core = AgentCore::try_new(config.clone()).unwrap();
    let started = core
        .submit(user_input(
            "bounded-review",
            "Report supported findings and coverage",
            AgentInputDelivery::CurrentTurn,
        ))
        .unwrap();
    let planned = submit_response(
        &mut core,
        &model_request(&started.host_requests),
        1,
        empty_model_completion(vec![named_tool_call(
            "bounded-search",
            "web_search",
            json!({"query":"evidence"}),
        )]),
    );
    let search = request_matching(&planned.host_requests, |payload| {
        matches!(payload,
        HostRequest::ExecuteTool { call_id, .. } if call_id == "bounded-search")
    });
    let observed = complete_tool(&mut core, &search, "bounded-search");
    let request = model_request(&observed.host_requests);
    let HostRequest::ModelStream {
        tools,
        response_mode,
        ..
    } = &request.payload
    else {
        unreachable!()
    };
    assert_eq!(model_tool_names(tools), vec!["requirements_update"],
        "finalization permits requirement accounting, never renewed external execution");
    assert_eq!(*response_mode, ModelResponseMode::Conversational);
    let wire = serde_json::to_vec(&core.checkpoint(CheckpointReason::Manual)).unwrap();
    core = AgentCore::restore(config, serde_json::from_slice(&wire).unwrap()).unwrap();
    let resumed = core.resume_committed_host_effect(&request).unwrap();
    let request = model_request(&resumed.host_requests);
    let completed = submit_response(
        &mut core,
        &request,
        1,
        visible_model_completion(
            "final-phase",
            "Supported finding; remaining scope unassessed.",
        ),
    );
    assert_eq!(
        completed.snapshot.turn_status,
        nola_core::agent::AgentTurnStatus::Completed
    );
    assert!(completed.host_requests.is_empty());
}

#[test]
fn unusable_iteration_budget_synthesis_records_incomplete_requirements() {
    let mut config = core_config(Vec::new());
    config.max_model_iterations = 2;
    config
        .permission_rules
        .rules
        .push(nola_core::agent::permissions::PermissionRule {
            capability_pattern: "*".to_string(),
            resource_pattern: "*".to_string(),
            decision: nola_core::agent::permissions::PermissionPolicyDecision::Allow,
            source: nola_core::agent::permissions::PermissionRuleSource::UserGlobal,
        });
    let (mut core, final_model) = reach_iteration_budget_synthesis(&config);
    let failed = submit_response(
        &mut core,
        &final_model,
        1,
        empty_model_completion(Vec::new()),
    );
    assert_eq!(
        failed.snapshot.turn_status,
        nola_core::agent::AgentTurnStatus::Completed
    );
    assert_eq!(failed.snapshot.engineering.mutation_count, 2);
    assert!(failed.snapshot.engineering.validation_required);
    assert!(failed.event_envelopes.iter().any(|event| matches!(&event.event,
        AgentEvent::RunResultDecided { decision, .. }
            if matches!(&decision.value, nola_domain::run_result::RunResultValue::Incomplete {
                reason, remaining_requirement_refs, ..
            } if reason.code == "verification.pending" && !remaining_requirement_refs.is_empty())
                && matches!(&decision.source, nola_domain::core_requirements::CoreDecisionSource::Model {
                    effect_id, request_id, ..
                } if effect_id == &final_model.effect_id.0 && request_id == &final_model.request_id)
    )));
    assert!(!failed.event_envelopes.iter().any(|event|
        matches!(&event.event, AgentEvent::TurnFailed { .. })));
    assert!(failed.host_requests.is_empty());
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

#[test]
fn reasoning_only_completion_retries_once_with_tools_then_stops_if_repeated() {
    let mut core = AgentCore::try_new(core_config(Vec::new())).expect("create Core");
    let started = core
        .submit(user_input(
            "reasoning-only-user",
            "use the appropriate skill and create the requested artifact",
            AgentInputDelivery::CurrentTurn,
        ))
        .expect("start turn");
    let first_model = model_request(&started.host_requests);

    let continued = submit_response(
        &mut core,
        &first_model,
        1,
        reasoning_only_completion(&first_model, "I should invoke the matching skill."),
    );
    assert_eq!(
        continued.snapshot.turn_status,
        nola_core::agent::AgentTurnStatus::AwaitingModel
    );
    assert!(!continued
        .event_envelopes
        .iter()
        .any(|envelope| matches!(envelope.event, AgentEvent::TurnCompleted { .. })));
    let retry = model_request(&continued.host_requests);
    match &retry.payload {
        HostRequest::ModelStream {
            messages, tools, ..
        } => {
            assert!(
                !tools.is_empty(),
                "reasoning-only recovery must retain tools"
            );
            assert!(
                serde_json::to_string(messages)
                    .expect("serialize messages")
                    .contains("previous model step ended after internal reasoning"),
                "recovery request must explain why the model is being continued"
            );
        }
        _ => panic!("expected retry model request"),
    }

    let failed = submit_response(
        &mut core,
        &retry,
        1,
        reasoning_only_completion(&retry, "I am still deciding which tool to use."),
    );
    assert_eq!(
        failed.snapshot.turn_status,
        nola_core::agent::AgentTurnStatus::Failed
    );
    assert!(failed.event_envelopes.iter().any(|envelope| {
        matches!(
            &envelope.event,
            AgentEvent::TurnFailed { error, .. }
                if error.contains("repeatedly completed after reasoning")
        )
    }));
}

#[test]
fn output_limit_is_recovered_once_by_core_without_a_fabricated_user_message() {
    let mut core = AgentCore::try_new(core_config(Vec::new())).expect("create Core");
    let started = core
        .submit(user_input(
            "output-limit-user",
            "provide a detailed explanation",
            AgentInputDelivery::CurrentTurn,
        ))
        .expect("start turn");
    let first_model = model_request(&started.host_requests);

    let continued = submit_response(
        &mut core,
        &first_model,
        1,
        limited_model_completion(
            "limited-part-1",
            "This is the first valid section of a long response that reached its provider limit.",
        ),
    );
    assert_eq!(
        continued.snapshot.turn_status,
        nola_core::agent::AgentTurnStatus::AwaitingModel
    );
    let retry = model_request(&continued.host_requests);
    match &retry.payload {
        HostRequest::ModelStream { messages, .. } => {
            let encoded = serde_json::to_string(messages).expect("serialize retry messages");
            assert!(messages.iter().any(|message| {
                message["role"] == "system"
                    && message["content"].as_str().is_some_and(|text| {
                        text.contains("output will be appended, not substituted")
                            && text.contains("Add only the missing suffix")
                    })
            }), "continuation must explicitly preserve the existing answer and append only its missing suffix");
            assert!(encoded.contains("first valid section"));
            let HostRequest::ModelStream { messages: original, .. } = &first_model.payload else {
                panic!("expected original model request");
            };
            assert_eq!(
                messages.iter().filter(|message| message["role"] == "user").collect::<Vec<_>>(),
                original.iter().filter(|message| message["role"] == "user").collect::<Vec<_>>(),
                "output recovery must not fabricate or rewrite user messages",
            );
        }
        _ => panic!("expected retry model request"),
    }

    let completed = submit_response(
        &mut core,
        &retry,
        1,
        limited_model_completion(
            "limited-part-2",
            "This is the final remainder; Core must not schedule a third model request.",
        ),
    );
    assert_eq!(
        completed.snapshot.turn_status,
        nola_core::agent::AgentTurnStatus::Completed
    );
    assert!(completed.host_requests.is_empty());
}

#[test]
fn output_limit_recovery_discards_a_duplicate_second_segment() {
    let mut core = AgentCore::try_new(core_config(Vec::new())).expect("create Core");
    let started = core
        .submit(user_input(
            "output-limit-duplicate-user",
            "provide a detailed explanation",
            AgentInputDelivery::CurrentTurn,
        ))
        .expect("start turn");
    let first_model = model_request(&started.host_requests);
    let repeated =
        "A substantive response segment that must be retained exactly once in the conversation.";
    let continued = submit_response(
        &mut core,
        &first_model,
        1,
        limited_model_completion("retained-limited-part", repeated),
    );
    let retry = model_request(&continued.host_requests);
    let completed = submit_response(
        &mut core,
        &retry,
        1,
        limited_model_completion("duplicate-limited-part", repeated),
    );

    assert_eq!(
        completed.snapshot.turn_status,
        nola_core::agent::AgentTurnStatus::Completed
    );
    assert!(completed.host_requests.is_empty());
    let snapshot = serde_json::to_string(&completed.snapshot).expect("serialize snapshot");
    assert!(snapshot.contains("retained-limited-part"));
    assert!(!snapshot.contains("duplicate-limited-part"));
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

#[test]
fn skill_required_tool_descriptor_is_installed_before_skill_run_starts() {
    let mut core = AgentCore::try_new(skill_runtime_core_config()).expect("create Core");
    let started = core
        .submit(user_input(
            "skill-required-tool-user",
            "Use the document Skill.",
            AgentInputDelivery::CurrentTurn,
        ))
        .expect("start Skill Turn");
    let dispatched = submit_response(
        &mut core,
        &model_request(&started.host_requests),
        1,
        empty_model_completion(vec![named_tool_call(
            "skill-required-tool-call",
            "Skill",
            json!({"skill": "document-create"}),
        )]),
    );
    let tool_request = request_matching(
        &dispatched.host_requests,
        |payload| matches!(payload, HostRequest::ExecuteTool { call_id, .. } if call_id == "skill-required-tool-call"),
    );
    let digest = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let write_file = capability("write_file", HostCapabilityRoute::Tool, None);
    let invoked = submit_response(
        &mut core,
        &tool_request,
        1,
        HostResponse::ToolCompleted {
            call_id: "skill-required-tool-call".to_string(),
            output: json!({
                "type": "invoked_skill",
                "skill_id": "document-create",
                "version": "1.0.0",
                "display_name": "Document create",
                "instructions_ref": format!("skill://document-create/1.0.0/SKILL.md#sha256={digest}"),
                "instructions_digest": digest,
                "required_tools": [{"provider": "builtin", "tool_name": "write_file"}],
                "mode": "inline",
                "execution_contract": {"schemaVersion": 1}
            }),
            outcome: nola_core::agent::host::HostToolOutcome::Succeeded,
            activation: Some(nola_core::agent::host::HostCapabilityActivation {
                source: "skill".to_string(),
                capability_ids: vec!["write_file".to_string()],
                capability_descriptors: vec![serde_json::to_value(&write_file).unwrap()],
            }),
            full_output_ref: None,
            artifacts: Vec::new(),
        },
    );

    assert_eq!(invoked.snapshot.skill_runtime.runs.len(), 1);
    let next = model_request(&invoked.host_requests);
    let HostRequest::ModelStream { tools, .. } = next.payload else {
        panic!("Skill invocation must continue with a model request")
    };
    assert!(tools.iter().any(|schema| {
        schema.pointer("/function/name").and_then(Value::as_str) == Some("write_file")
    }));
}

#[test]
fn extraction_enabled_turn_completion_enqueues_a_session_owned_memory_job() {
    let mut config = core_config(Vec::new());
    config.memory_policy.long_term_enabled = true;
    config.memory_policy.automatic_extraction_enabled = true;
    let mut core = AgentCore::try_new(config).expect("create Core");
    let started = core
        .submit(user_input(
            "automatic-memory-user",
            "Keep the durable preference from this exchange.",
            AgentInputDelivery::CurrentTurn,
        ))
        .expect("start Turn");
    let completed = submit_response(
        &mut core,
        &model_request(&started.host_requests),
        1,
        HostResponse::ModelCompleted {
            items: vec![json!({
                "type": "message",
                "id": "automatic-memory-assistant",
                "role": "assistant",
                "content": "Understood."
            })],
            tool_calls: Vec::new(),
            usage: json!({}),
        },
    );

    assert_eq!(completed.snapshot.memory_runtime.job_order.len(), 1);
    let job_id = &completed.snapshot.memory_runtime.job_order[0];
    let job = completed
        .snapshot
        .memory_runtime
        .job(job_id)
        .expect("queued extraction job");
    assert_eq!(job.state, nola_core::agent::MemoryJobState::Queued);
    assert_eq!(job.sources.len(), 1);
    assert_eq!(job.sources[0].session_id, "session-contract");
}

#[test]
fn truncated_memory_search_result_does_not_update_usage_rank() {
    let mut config = core_config(Vec::new());
    let mut memory_search = capability("memory_search", HostCapabilityRoute::Tool, None);
    memory_search.input_normalization.permission_contract.risk = nola_domain::permission_resource_plan::PermissionOperationRisk::SensitiveRead;
    config.permission_rules.rules.push(nola_core::agent::permissions::PermissionRule {
        capability_pattern:"memory_search".into(),resource_pattern:"*".into(),
        decision:nola_core::agent::permissions::PermissionPolicyDecision::Allow,
        source:nola_core::agent::permissions::PermissionRuleSource::UserGlobal,
    });
    memory_search.output_policy.preview_char_limit = 32;
    config
        .host_catalog
        .as_mut()
        .expect("host catalog")
        .capabilities
        .insert("memory_search".to_string(), memory_search);
    config.tools.push(json!({
        "type": "function",
        "function": {
            "name": "memory_search",
            "description": "typed memory lookup",
            "parameters": {"type": "object"}
        }
    }));
    let mut core = AgentCore::try_new(config).expect("create Core");
    let started = core
        .submit(user_input(
            "memory-usage-user",
            "Use the selected durable context.",
            AgentInputDelivery::CurrentTurn,
        ))
        .expect("start memory Turn");
    let dispatched = submit_response(
        &mut core,
        &model_request(&started.host_requests),
        1,
        empty_model_completion(vec![named_tool_call(
            "memory-search-call",
            "memory_search",
            json!({}),
        )]),
    );
    let tool_request = request_matching(
        &dispatched.host_requests,
        |payload| matches!(payload, HostRequest::ExecuteTool { call_id, .. } if call_id == "memory-search-call"),
    );
    let completed = submit_response(
        &mut core,
        &tool_request,
        1,
        HostResponse::ToolCompleted {
            call_id: "memory-search-call".to_string(),
            output: json!({
                "text": "A bounded prefix followed by memory results that cannot all fit in the model-visible tool envelope.",
                "_nola_memory_usage": [{
                    "memory_id": "memory-hidden-in-truncated-tail",
                    "used_at_epoch_ms": 1_785_600_000_123_i64,
                    "context_payload_digest": "sha256:memory-search-result",
                    "citation_ids": ["memory-search:memory-hidden-in-truncated-tail"]
                }]
            }),
            outcome: nola_core::agent::host::HostToolOutcome::Succeeded,
            activation: None,
            full_output_ref: Some("artifact://memory-search/full".to_string()),
            artifacts: Vec::new(),
        },
    );

    assert!(completed.snapshot.memory_runtime.usage_ledger.is_empty());
    assert!(completed.snapshot.memory_runtime.usage_stats.is_empty());
    let next_model = model_request(&completed.host_requests);
    let HostRequest::ModelStream { messages, .. } = next_model.payload else {
        unreachable!()
    };
    assert!(
        messages.iter().any(|message| {
            message.get("type").and_then(Value::as_str) == Some("tool_result")
                && message
                    .get("output")
                    .and_then(|output| output.get("truncated"))
                    .and_then(Value::as_bool)
                    == Some(true)
        }),
        "{messages:#?}"
    );
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

#[test]
fn host_skill_updates_drive_one_core_owned_run_through_contract_completion() {
    let mut core = AgentCore::try_new(skill_runtime_core_config()).expect("create Core");
    let completed = complete_verified_document_skill_run(&mut core);
    let run = completed
        .snapshot
        .skill_runtime
        .runs
        .values()
        .next()
        .expect("SkillRun");
    assert_eq!(
        run.status,
        nola_core::agent::capability::SkillRunStatus::Completed
    );
    assert!(run.completed_progress_steps.contains("render"));
    assert_eq!(run.artifacts["document"], "file:///tmp/report.docx");
    assert!(run.verifications["render-check"].passed);
    assert_eq!(run.resume_point.as_deref(), Some("after-render"));
}

#[test]
fn one_writer_artifact_satisfies_one_active_skill_artifact_without_uri_copying() {
    let mut config = skill_runtime_core_config();
    let mut writer = capability("persist_record", HostCapabilityRoute::Tool, None);
    writer.input_normalization.permission_contract.risk = nola_domain::permission_resource_plan::PermissionOperationRisk::NewFile;
    writer.input_normalization.effect =
        nola_core::agent::host::HostCapabilityEffectDeclaration::by_argument(
            "operation_kind",
            nola_core::agent::host::HostCapabilityEffect::Execute,
            [(
                "mutate_and_validate".to_string(),
                nola_core::agent::host::HostCapabilityEffect::MutateAndValidate,
            )],
        );
    config
        .host_catalog
        .as_mut()
        .expect("host catalog")
        .capabilities
        .insert(writer.id.clone(), writer);
    config.tools.push(json!({
        "type": "function",
        "function": {
            "name": "persist_record",
            "description": "persist one record",
            "parameters": {"type": "object"}
        }
    }));
    let mut core = AgentCore::try_new(config).expect("create Core");
    let started = core
        .submit(user_input(
            "skill-artifact-link-user",
            "Persist the requested record.",
            AgentInputDelivery::CurrentTurn,
        ))
        .expect("start Skill Turn");
    let digest = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let invoked = complete_named_tool_round(
        &mut core,
        &model_request(&started.host_requests),
        "skill-artifact-link-call",
        "Skill",
        json!({"skill": "record-create"}),
        json!({
            "type": "invoked_skill",
            "skill_id": "record-create",
            "version": "1.0.0",
            "display_name": "Record create",
            "instructions_ref": format!("skill://record-create/1.0.0/SKILL.md#sha256={digest}"),
            "instructions_digest": digest,
            "required_tools": [],
            "mode": "inline",
            "execution_contract": {
                "schemaVersion": 1,
                "progressSteps": [],
                "artifacts": [{"id": "record", "mediaType": "application/json", "required": true}],
                "verifiers": [],
                "resumePoints": []
            }
        }),
    );
    let dispatched = submit_response(
        &mut core,
        &model_request(&invoked.host_requests),
        1,
        empty_model_completion(vec![named_tool_call(
            "persist-record-call",
            "persist_record",
            json!({"operation_kind":"mutate_and_validate"}),
        )]),
    );
    let request = request_matching(
        &dispatched.host_requests,
        |payload| matches!(payload, HostRequest::ExecuteTool { call_id, .. } if call_id == "persist-record-call"),
    );
    let completed = submit_response(
        &mut core,
        &request,
        1,
        HostResponse::ToolCompleted {
            call_id: "persist-record-call".to_string(),
            output: json!({"record_id": "record-42"}),
            outcome: nola_core::agent::host::HostToolOutcome::Succeeded,
            activation: None,
            full_output_ref: None,
            artifacts: vec![HostArtifact {
                uri: "nola://record/record-42".to_string(),
                name: "record 42".to_string(),
                mime_type: Some("application/json".to_string()),
                operation: Some("create".to_string()),
            }],
        },
    );

    let run = completed
        .snapshot
        .skill_runtime
        .runs
        .values()
        .next()
        .expect("SkillRun");
    assert_eq!(run.artifacts["record"], "nola://record/record-42");
}

#[test]
fn failed_skill_update_at_iteration_boundary_terminates_skill_before_final_synthesis() {
    let mut config = skill_runtime_core_config();
    config.max_model_iterations = 2;
    let mut core = AgentCore::try_new(config).expect("create Core");
    let started = core
        .submit(user_input(
            "skill-protocol-boundary-user",
            "Produce the requested artifact.",
            AgentInputDelivery::CurrentTurn,
        ))
        .expect("start Skill Turn");
    let digest = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let invoked = complete_named_tool_round(
        &mut core,
        &model_request(&started.host_requests),
        "skill-protocol-boundary-call",
        "Skill",
        json!({"skill": "artifact-create"}),
        json!({
            "type": "invoked_skill",
            "skill_id": "artifact-create",
            "version": "1.0.0",
            "display_name": "Artifact create",
            "instructions_ref": format!("skill://artifact-create/1.0.0/SKILL.md#sha256={digest}"),
            "instructions_digest": digest,
            "required_tools": [],
            "mode": "inline",
            "execution_contract": {
                "schemaVersion": 1,
                "progressSteps": [],
                "artifacts": [{"id": "artifact", "mediaType": "application/json", "required": true}],
                "verifiers": [],
                "resumePoints": []
            }
        }),
    );
    let run_id = invoked
        .snapshot
        .skill_runtime
        .runs
        .keys()
        .next()
        .expect("SkillRun id")
        .clone();
    let dispatched = submit_response(
        &mut core,
        &model_request(&invoked.host_requests),
        1,
        empty_model_completion(vec![named_tool_call(
            "invalid-skill-artifact-call",
            "skill_update",
            json!({
                "run_id": run_id,
                "action": "artifact",
                "artifact_id": "artifact",
                "artifact_ref": "relative-record-id"
            }),
        )]),
    );
    let request = request_matching(
        &dispatched.host_requests,
        |payload| matches!(payload, HostRequest::ExecuteTool { call_id, .. } if call_id == "invalid-skill-artifact-call"),
    );
    let recovery = submit_response(
        &mut core,
        &request,
        1,
        HostResponse::ToolCompleted {
            call_id: "invalid-skill-artifact-call".to_string(),
            output: json!({
                "error": "invalid_skill_artifact_ref",
                "message": "artifact_ref must be a canonical absolute URI",
                "tool_execution_skipped": true,
                "next_action": "Use a canonical artifact URI"
            }),
            outcome: nola_core::agent::host::HostToolOutcome::Failed,
            activation: None,
            full_output_ref: None,
            artifacts: Vec::new(),
        },
    );

    let run = &recovery.snapshot.skill_runtime.runs[&run_id];
    assert_eq!(
        run.status,
        nola_core::agent::capability::SkillRunStatus::Failed
    );
    let final_request = model_request(&recovery.host_requests);
    let HostRequest::ModelStream { tools, .. } = &final_request.payload else {
        panic!("budget boundary must request final synthesis")
    };
    assert_eq!(model_tool_names(tools), vec!["requirements_update"]);
}

#[test]
fn unique_active_skill_accepts_its_skill_id_as_update_alias() {
    let mut core = AgentCore::try_new(skill_runtime_core_config()).expect("create Core");
    let started = core
        .submit(user_input(
            "skill-alias-user",
            "Create the document.",
            AgentInputDelivery::CurrentTurn,
        ))
        .expect("start Skill Turn");
    let digest = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let invoked = complete_named_tool_round(
        &mut core,
        &model_request(&started.host_requests),
        "skill-alias-call",
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
                "artifacts": [],
                "verifiers": [],
                "resumePoints": []
            }
        }),
    );
    let updated = complete_named_tool_round(
        &mut core,
        &model_request(&invoked.host_requests),
        "skill-alias-progress",
        "skill_update",
        json!({"run_id": "document-create", "action": "progress", "step_id": "render", "completed": true}),
        json!({"type": "skill_runtime_update", "run_id": "document-create", "action": "progress", "step_id": "render", "completed": true}),
    );
    let run = updated
        .snapshot
        .skill_runtime
        .runs
        .values()
        .next()
        .expect("active SkillRun");
    assert!(run.completed_progress_steps.contains("render"));
}

#[test]
fn compaction_preserves_live_work_permission_task_and_skill_evidence() {
    use nola_core::agent::goal::{
        GoalRecordStatus, GoalRequirement, GoalRuntimeCommand, GoalRuntimeCommandEnvelope,
    };
    use nola_core::agent::permissions::{
        PendingPermissionRequest, PermissionContinuation, PermissionEvaluation,
        PermissionEvaluationReason, PermissionPolicyDecision,
    };
    use nola_core::agent::task::{
        TaskDurability, TaskHandle, TaskKind, TaskLineage, TaskProjection, TaskState,
    };
    use nola_core::agent::{PermissionRequestId, TurnId};

    let initial_items = (0..24)
        .map(|index| AgentItem::Message {
            id: ItemId::from_string(format!("work-history-{index}")),
            role: if index % 2 == 0 {
                AgentRole::User
            } else {
                AgentRole::Assistant
            },
            content: format!("work history {index} {}", "x".repeat(160)),
        })
        .collect::<Vec<_>>();
    let config = skill_runtime_core_config_with_items(initial_items);
    let mut core = AgentCore::try_new(config.clone()).expect("create Core");
    let skill_completed = complete_verified_document_skill_run(&mut core);
    let skill_run = skill_completed
        .snapshot
        .skill_runtime
        .runs
        .values()
        .next()
        .expect("verified SkillRun");
    assert!(!skill_run.artifacts.is_empty());
    assert!(skill_run.verifications.values().any(|result| result.passed));

    let mut checkpoint = core.checkpoint(CheckpointReason::Manual);
    checkpoint
        .state
        .goal_projection
        .apply(GoalRuntimeCommandEnvelope {
            operation_id: "work-goal-start".to_string(),
            command: GoalRuntimeCommand::Start {
                id: "work-goal".to_string(),
                objective: "Deliver the independently verified document".to_string(),
                constraints: vec!["Preserve evidence across compaction".to_string()],
                requirements: vec![GoalRequirement {
                    acceptance_policy: nola_core::agent::goal::GoalAcceptancePolicy::ModelJudgment,
                    semantic_revision: 1, id: "verified-document".to_string(),
                    description: "The document has independent render evidence".to_string(),
                    dependencies: Vec::new(),
                    status: GoalRecordStatus::Pending,
                    evidence_refs: Vec::new(),
                }],
                now_epoch_ms: 100,
            },
        })
        .expect("seed active Goal");

    let task_id = "background-render-task".to_string();
    checkpoint.state.task_projection = TaskProjection {
        revision: 1,
        order: vec![task_id.clone()],
        tasks: BTreeMap::from([(
            task_id.clone(),
            TaskHandle {
                id: task_id,
                owner_session_id: SessionId::from_string("session-contract".to_string()),
                owner_turn_id: TurnId::from_string("work-turn".to_string()),
                kind: TaskKind::Skill,
                state: TaskState::Waiting,
                progress: Some(0.75),
                current_step: Some("awaiting user permission".to_string()),
                result_ref: Some("artifact://document/final.docx".to_string()),
                cancel_requested: false,
                cancel_requested_at_epoch_ms: None,
                lineage: TaskLineage {
                    permission_scope_id: Some("workspace.write".to_string()),
                    cancellation_scope_id: Some("work-goal".to_string()),
                    ..TaskLineage::default()
                },
                durability: TaskDurability {
                    input_watermark: Some(4),
                    completion_watermark: Some(3),
                    ..TaskDurability::default()
                },
            },
        )]),
        guidance: BTreeMap::new(),
    };

    checkpoint
        .state
        .execution
        .permissions
        .upsert(PendingPermissionRequest {
            allow_conversation: true,
            permission_instructions: Vec::new(),
            invocation_context: None,
            request_id: PermissionRequestId::from_string("permission-work-write".to_string()),
            turn_id: TurnId::from_string("work-turn".to_string()),
            call_id: CallId::from_string("work-write-call".to_string()),
            capability_id: "write_file".to_string(),
            resource: "/tmp/report.docx".to_string(),
            arguments: json!({"path": "/tmp/report.docx", "content": "verified"}),
            permission_domain: "workspace.write".to_string(),
            rationale: "Publish the verified document".to_string(),
            evaluation: PermissionEvaluation {
                decision: PermissionPolicyDecision::Ask,
                reason: PermissionEvaluationReason::NoMatchingRule,
                rule_index: None,
                rule: None,
            },
            continuation: PermissionContinuation::CoreTool,
        });

    let mut core = AgentCore::restore(config, checkpoint).expect("restore active Work");
    let before = core.snapshot();
    assert_eq!(before.tasks.tasks.len(), 1);
    assert!(before.goal.active.is_some());
    assert_eq!(before.permissions.pending.len(), 1);
    assert_eq!(before.skill_runtime.runs.len(), 1);

    let requested = core
        .submit(AgentSubmission::Compact {
            operation_id: "compact-active-work".to_string(),
        })
        .expect("request Work compaction");
    let compact = request_matching(&requested.host_requests, |payload| {
        matches!(payload, HostRequest::Compact { .. })
    });
    let completed = submit_response(
        &mut core,
        &compact,
        1,
        HostResponse::CompactionCompleted {
            summary: "Preserve the active Work contract and evidence.".to_string(),
            usage: serde_json::to_value(TokenUsage {
                input_tokens: Some(1_000),
                output_tokens: Some(80),
                total_tokens: Some(1_080),
                ..TokenUsage::default()
            })
            .expect("serialize usage"),
        },
    );

    assert_eq!(completed.snapshot.tasks, before.tasks);
    assert_eq!(completed.snapshot.goal, before.goal);
    assert_eq!(completed.snapshot.permissions, before.permissions);
    assert_eq!(completed.snapshot.skill_runtime, before.skill_runtime);
    let preserved_run = completed
        .snapshot
        .skill_runtime
        .runs
        .values()
        .next()
        .expect("preserved SkillRun");
    assert_eq!(
        preserved_run.artifacts["document"],
        "file:///tmp/report.docx"
    );
    assert!(preserved_run.verifications["render-check"].passed);
}

#[test]
fn post_tool_answer_is_private_until_independent_audit_completes() {
    let mut core = AgentCore::try_new(source_core_config()).expect("create Core");
    let started = core
        .submit(user_input(
            "audited-answer-user",
            "List only entries proven eligible by current evidence.",
            AgentInputDelivery::CurrentTurn,
        ))
        .expect("start audited answer turn");
    let discovery_request = model_request(&started.host_requests);
    let premature = submit_response(
        &mut core,
        &discovery_request,
        1,
        model_requested_review(
            "review-premature",
            "Here's a thinking process: analyze the user input.",
        ),
    );
    assert!(premature
        .snapshot
        .projection
        .parts
        .values()
        .all(|part| !matches!(part, AgentPart::Text(_))));
    assert!(!premature
        .event_envelopes
        .iter()
        .any(|event| matches!(event.event, AgentEvent::TurnCompleted { .. })));
    let admission_audit = model_request(&premature.host_requests);
    assert!(matches!(&admission_audit.payload,
        HostRequest::ModelStream { tools, response_mode: ModelResponseMode::StructuredAudit, .. }
            if tools.is_empty()));
    let resumed = submit_response(
        &mut core,
        &admission_audit,
        1,
        visible_model_completion(
            "missing-work",
            r#"{"decision":"execute","outcome":"incomplete","constraints":["current evidence"],"items":[],"violations":["No source was inspected; the candidate only describes the input."]}"#,
        ),
    );
    let discovery_request = model_request(&resumed.host_requests);
    assert!(matches!(&discovery_request.payload,
        HostRequest::ModelStream { tools, response_mode: ModelResponseMode::Conversational, .. }
            if !tools.is_empty()));
    let checkpoint = core.checkpoint(CheckpointReason::Manual);
    let turn = checkpoint.state.execution.active_turn.as_ref().unwrap();
    assert_eq!(turn.completion_recovery_count, 1);
    assert!(turn.completion_recovery_feedback.is_some());
    core = AgentCore::restore(
        source_core_config(),
        serde_json::from_slice(&serde_json::to_vec(&checkpoint).unwrap()).unwrap(),
    )
    .unwrap();
    let dispatched = submit_response(
        &mut core,
        &discovery_request,
        1,
        empty_model_completion(vec![named_tool_call(
            "search-current",
            "web_search",
            json!({"query": "current eligible entries"}),
        )]),
    );
    let search_request = request_matching(
        &dispatched.host_requests,
        |payload| matches!(payload, HostRequest::ExecuteTool { call_id, .. } if call_id == "search-current"),
    );
    let after_search = submit_response(
        &mut core,
        &search_request,
        1,
        HostResponse::ToolCompleted {
            call_id: "search-current".to_string(),
            output: json!({
                "results": [
                    {"name": "Eligible", "status": "open"},
                    {"name": "Expired", "status": "closed"}
                ]
            }),
            outcome: nola_core::agent::host::HostToolOutcome::Succeeded,
            activation: None,
            full_output_ref: None,
            artifacts: Vec::new(),
        },
    );
    let synthesis_request = model_request(&after_search.host_requests);
    let candidate = submit_response(
        &mut core,
        &synthesis_request,
        1,
        model_requested_review(
            "review-unsafe-candidate",
            "Eligible and Expired are both available.",
        ),
    );

    assert!(!candidate
        .event_envelopes
        .iter()
        .any(|event| matches!(event.event, AgentEvent::TurnCompleted { .. })));
    assert!(candidate
        .snapshot
        .projection
        .parts
        .values()
        .all(|part| { !matches!(part, AgentPart::Text(text) if text.text.contains("Expired")) }));
    let audit_request = model_request(&candidate.host_requests);
    assert!(
        candidate.event_envelopes.iter().any(|event| matches!(
            &event.event, AgentEvent::HostProgress { title, .. } if title == "model_review_started"
        )),
        "review must have an observable phase distinct from generation"
    );
    match &audit_request.payload {
        HostRequest::ModelStream {
            messages,
            tools,
            response_mode,
            ..
        } => {
            assert_eq!(*response_mode, ModelResponseMode::StructuredAudit);
            assert!(tools.is_empty(), "the audit boundary must be tool-free");
            assert_eq!(
                messages.len(),
                3,
                "isolated review packet plus the Core-owned requirements context"
            );
            let isolated = serde_json::to_value(messages).expect("audit messages");
            assert_eq!(isolated[0]["role"], "system");
            assert_eq!(isolated[1]["role"], "user");
            assert_eq!(isolated[2]["role"], "user");
            let control = isolated[2]["content"].as_str().unwrap();
            assert!(control.starts_with("Core task observations, not a new user request."));
            let state: Value = serde_json::from_str(control.split_once("<requirements_state>")
                .unwrap().1.strip_suffix("</requirements_state>").unwrap()).unwrap();
            assert!(state.get("scope_id").is_none());
            assert!(state.get("input_frontier_token").is_none());
            assert!(state["requirements"].is_array());
            let packet: Value =
                serde_json::from_str(isolated[1]["content"].as_str().unwrap()).unwrap();
            assert_eq!(packet["schema"], "nola.completion_review.v1");
            assert!(packet["current_permission_rules"]["rules"].is_array());
            assert_eq!(
                packet["candidate_answer"],
                "Eligible and Expired are both available."
            );
            let serialized = serde_json::to_string(messages).expect("serialize audit messages");
            assert!(serialized.contains("original_user_messages"));
            assert!(serialized.contains("Eligible and Expired are both available."));
            assert!(serialized.contains("independent constraint-audit boundary"));
            assert!(serialized.contains("eligibility"));
            assert!(serialized.contains("eligible|rejected|unknown"));
        }
        other => panic!("expected audit model request, got {other:?}"),
    }

    let hidden_audit_delta = submit_response(
        &mut core,
        &audit_request,
        1,
        model_delta(
            &audit_request,
            "{\"decision\":\"revise\",\"outcome\":\"incomplete\",\"constraints\":[\"eligible only\"],",
        ),
    );
    assert!(hidden_audit_delta
        .snapshot
        .projection
        .parts
        .values()
        .all(|part| !matches!(part, AgentPart::Text(_))));
    let invalid_audit = submit_response(
        &mut core,
        &audit_request,
        2,
        visible_model_completion(
            "invalid-audit-report",
            "I should provide the final answer now.",
        ),
    );
    assert!(!invalid_audit
        .event_envelopes
        .iter()
        .any(|event| matches!(event.event, AgentEvent::TurnCompleted { .. })));
    let audit_retry = model_request(&invalid_audit.host_requests);
    match &audit_retry.payload {
        HostRequest::ModelStream {
            messages,
            tools,
            response_mode,
            ..
        } => {
            assert_eq!(*response_mode, ModelResponseMode::StructuredAudit);
            assert!(tools.is_empty());
            let serialized = serde_json::to_string(messages).expect("serialize audit retry");
            assert!(serialized.contains("did not satisfy the JSON protocol"));
            assert!(!serialized.contains("Produce the supported final answer now"));
        }
        other => panic!("expected audit retry request, got {other:?}"),
    }
    // A provider can violate even a tool-free structured response contract.
    // Reject the proposal before tool dispatch; it must not poison finalization
    // with an execution recovery obligation that this phase cannot satisfy.
    let illegal_audit_call = submit_response(
        &mut core,
        &audit_retry,
        1,
        empty_model_completion(vec![named_tool_call(
            "illegal-audit-bash",
            "bash",
            json!({"command": "git status --porcelain"}),
        )]),
    );
    assert!(illegal_audit_call
        .host_requests
        .iter()
        .all(|request| { matches!(request.payload, HostRequest::ModelStream { .. }) }));
    let audit_retry = model_request(&illegal_audit_call.host_requests);
    assert!(matches!(&audit_retry.payload,
        HostRequest::ModelStream { tools, response_mode: ModelResponseMode::StructuredAudit, .. }
            if tools.is_empty()));
    let audit_completed = submit_response(
        &mut core,
        &audit_retry,
        1,
        visible_model_completion(
            "audit-report",
            "Audit follows:\n```json\n{\"decision\":\"revise\",\"outcome\":\"incomplete\",\"constraints\":[\"eligible only\"],\"items\":[{\"label\":\"Eligible\",\"eligibility\":\"eligible\",\"evidence\":\"open\"},{\"label\":\"Expired\",\"eligibility\":\"rejected\",\"evidence\":\"closed\"}],\"violations\":[\"Expired must be removed\"]}\n```",
        ),
    );
    assert!(!audit_completed
        .event_envelopes
        .iter()
        .any(|event| matches!(event.event, AgentEvent::TurnCompleted { .. })));
    assert!(audit_completed
        .snapshot
        .projection
        .parts
        .values()
        .all(|part| !matches!(part, AgentPart::Text(_))));
    let final_request = model_request(&audit_completed.host_requests);
    match &final_request.payload {
        HostRequest::ModelStream {
            messages,
            tools,
            response_mode,
            ..
        } => {
            assert_eq!(*response_mode, ModelResponseMode::Conversational);
            assert!(tools.is_empty());
            let serialized = serde_json::to_string(messages).expect("serialize final messages");
            assert!(serialized.contains("final_answer_audit_report"));
            assert!(serialized.contains("Expired must be removed"));
            assert!(serialized.contains("private candidate revision boundary"));
        }
        other => panic!("expected final presentation request, got {other:?}"),
    }
    let illegal_final_call = submit_response(
        &mut core,
        &final_request,
        1,
        empty_model_completion(vec![named_tool_call(
            "illegal-final-bash",
            "bash",
            json!({"command": "git status"}),
        )]),
    );
    assert!(illegal_final_call
        .host_requests
        .iter()
        .all(|request| { matches!(request.payload, HostRequest::ModelStream { .. }) }));
    let final_request = model_request(&illegal_final_call.host_requests);
    assert!(matches!(&final_request.payload,
        HostRequest::ModelStream { tools, response_mode: ModelResponseMode::Conversational, .. }
            if tools.is_empty()));
    let rejected_presentation = submit_response(
        &mut core,
        &final_request,
        1,
        visible_model_completion(
            "rejected-presentation",
            "Eligible is available. Expired is also worth checking.",
        ),
    );
    assert!(!rejected_presentation
        .event_envelopes
        .iter()
        .any(|event| matches!(event.event, AgentEvent::TurnCompleted { .. })));
    assert!(rejected_presentation
        .snapshot
        .projection
        .parts
        .values()
        .all(|part| { !matches!(part, AgentPart::Text(text) if text.text.contains("Expired")) }));
    let revision_audit = model_request(&rejected_presentation.host_requests);
    assert!(matches!(
        &revision_audit.payload,
        HostRequest::ModelStream {
            response_mode: ModelResponseMode::StructuredAudit,
            ..
        }
    ));
    let rejected_again = submit_response(
        &mut core,
        &revision_audit,
        1,
        visible_model_completion(
            "revision-audit",
            r#"{"decision":"revise","outcome":"incomplete","constraints":[],"items":[],"violations":["Expired is still included."]}"#,
        ),
    );
    let final_retry = model_request(&rejected_again.host_requests);
    match &final_retry.payload {
        HostRequest::ModelStream {
            messages, tools, ..
        } => {
            assert!(tools.is_empty());
            let serialized = serde_json::to_string(messages).expect("serialize retry messages");
            assert!(serialized.contains("Expired is still included"));
        }
        other => panic!("expected final retry request, got {other:?}"),
    }
    let visible_final_delta = submit_response(
        &mut core,
        &final_retry,
        1,
        model_delta(&final_retry, "Eligible is available."),
    );
    assert!(visible_final_delta
        .snapshot
        .projection
        .parts
        .values()
        .all(|part| !matches!(part, AgentPart::Text(_))));
    let final_part_id = match &final_retry.payload {
        HostRequest::ModelStream { text_part_id, .. } => text_part_id.clone(),
        _ => unreachable!(),
    };

    let revised_candidate = submit_response(
        &mut core,
        &final_retry,
        2,
        visible_model_completion(&final_part_id, "Eligible is available."),
    );
    assert!(revised_candidate
        .snapshot
        .projection
        .parts
        .values()
        .all(|part| !matches!(part, AgentPart::Text(_))));
    let review = model_request(&revised_candidate.host_requests);
    let completed = submit_response(
        &mut core,
        &review,
        1,
        visible_model_completion(
            "accepted-revision",
            r#"{"decision":"accept","outcome":"answered","constraints":[],"items":[{"label":"Fixture task requirement","eligibility":"eligible","evidence":"Observed fixture evidence supports the candidate"}],"violations":[]}"#,
        ),
    );
    assert!(
        completed.host_requests.is_empty(),
        "no unchecked final generator"
    );
    assert!(completed
        .event_envelopes
        .iter()
        .any(|event| matches!(event.event, AgentEvent::TurnCompleted { .. })));
    assert!(completed.snapshot.projection.parts.values().any(|part| {
        matches!(part, AgentPart::Text(text) if text.text == "Eligible is available." && text.is_complete)
    }));
    assert!(completed
        .snapshot
        .projection
        .parts
        .values()
        .all(|part| { !matches!(part, AgentPart::Text(text) if text.text.contains("Expired")) }));
}

#[test]
fn reviewed_stream_keeps_one_candidate_identity_through_cold_restore_and_approval() {
    use nola_core::agent::parts::TextReviewState;
    let mut config = source_core_config();
    config.turn_identity.supports_reviewed_text_parts = true;
    let mut core = AgentCore::try_new(config.clone()).unwrap();
    let started = core
        .submit(user_input(
            "stream-review",
            "你好",
            AgentInputDelivery::CurrentTurn,
        ))
        .unwrap();
    let model = model_request(&started.host_requests);
    assert!(
        matches!(&model.payload, HostRequest::ModelStream { settings, .. } if settings["defer_text_presentation"] == false)
    );
    let candidate = submit_response(
        &mut core,
        &model,
        1,
        model_requested_review("review-stream-candidate", "你好，朋友。"),
    );
    let audit = model_request(&candidate.host_requests);
    assert!(
        matches!(&audit.payload, HostRequest::ModelStream { settings, response_mode: ModelResponseMode::StructuredAudit, .. } if settings["defer_text_presentation"] == true)
    );
    let id = candidate
        .snapshot
        .projection
        .parts
        .values()
        .find_map(|part| match part {
            AgentPart::Text(text) if text.review == Some(TextReviewState::Pending) => {
                Some(text.id.clone())
            }
            _ => None,
        })
        .unwrap();
    assert!(
        matches!(&candidate.snapshot.projection.parts[&id], AgentPart::Text(text)
        if text.review == Some(TextReviewState::Pending) && text.text == "你好，朋友。")
    );
    let mut core = AgentCore::restore(config, core.checkpoint(CheckpointReason::Manual)).unwrap();
    let hidden = submit_response(
        &mut core,
        &audit,
        1,
        model_delta(&audit, "PRIVATE REVIEW JSON"),
    );
    assert!(hidden
        .snapshot
        .projection
        .parts
        .values()
        .all(|part| !matches!(part, AgentPart::Text(text) if text.text.contains("PRIVATE"))));
    let accepted = submit_response(
        &mut core,
        &audit,
        2,
        visible_model_completion(
            "review-verdict",
            r#"{"decision":"accept","outcome":"answered","items":[{"label":"greeting","eligibility":"eligible","evidence":"candidate greets user"}],"violations":[]}"#,
        ),
    );
    let texts: Vec<_> = accepted
        .snapshot
        .projection
        .parts
        .values()
        .filter_map(|part| match part {
            AgentPart::Text(text) if !text.text.is_empty() => Some(text),
            _ => None,
        })
        .collect();
    assert_eq!(texts.len(), 1);
    assert_eq!(texts[0].id, id);
    assert_eq!(texts[0].text, "你好，朋友。");
    assert_eq!(texts[0].review, Some(TextReviewState::Approved));
    assert!(accepted.host_requests.is_empty());
}

#[test]
fn failed_review_retains_an_interrupted_draft_not_a_final_answer() {
    use nola_core::agent::parts::TextReviewState;
    let mut config = source_core_config();
    config.turn_identity.supports_reviewed_text_parts = true;
    let mut core = AgentCore::try_new(config).unwrap();
    let started = core
        .submit(user_input(
            "fail-review",
            "Hello",
            AgentInputDelivery::CurrentTurn,
        ))
        .unwrap();
    let model = model_request(&started.host_requests);
    let candidate = submit_response(
        &mut core,
        &model,
        1,
        model_requested_review("review-failed-candidate", "Candidate."),
    );
    let review = model_request(&candidate.host_requests);
    let failed = submit_response(
        &mut core,
        &review,
        1,
        HostResponse::Failed {
            error: HostError {
                code: "provider_unavailable".into(),
                message: "offline".into(),
                retryable: false,
                http_status: None,
            },
        },
    );
    assert_eq!(
        failed.snapshot.turn_status,
        nola_core::agent::AgentTurnStatus::Failed
    );
    assert!(failed
        .snapshot
        .projection
        .parts
        .values()
        .any(|part| matches!(part, AgentPart::Text(text)
        if text.text == "Candidate." && text.review == Some(TextReviewState::Interrupted))));
    assert!(failed
        .snapshot
        .projection
        .parts
        .values()
        .all(AgentPart::is_terminal));
}

#[test]
fn cancellation_settles_visible_review_drafts_without_publishing_them() {
    use nola_core::agent::parts::TextReviewState;
    let mut config = source_core_config();
    config.turn_identity.supports_reviewed_text_parts = true;
    let mut core = AgentCore::try_new(config).unwrap();
    let started = core
        .submit(user_input(
            "cancel-draft",
            "Hello",
            AgentInputDelivery::CurrentTurn,
        ))
        .unwrap();
    let model = model_request(&started.host_requests);
    let candidate = submit_response(
        &mut core,
        &model,
        1,
        model_requested_review("cancel-review", "Unverified draft."),
    );
    let review = model_request(&candidate.host_requests);
    let interrupted = core
        .submit(AgentSubmission::Interrupt { goal_control: None, cancellation_id: "test-cancellation".into(),
            reason: Some("stop".into()),
        })
        .unwrap();
    let cancel = request_matching(&interrupted.host_requests, |request| {
        matches!(request, HostRequest::Cancel { .. })
    });
    let ended = submit_response(
        &mut core,
        &cancel,
        1,
        HostResponse::CancellationConfirmed {
            target_effect_id: review.effect_id,
        },
    );
    assert_eq!(
        ended.snapshot.turn_status,
        nola_core::agent::AgentTurnStatus::Cancelled
    );
    let drafts: Vec<_> = ended
        .snapshot
        .projection
        .parts
        .values()
        .filter_map(|part| match part {
            AgentPart::Text(text) if !text.text.is_empty() => Some(text),
            _ => None,
        })
        .collect();
    assert_eq!(drafts.len(), 1);
    assert_eq!(drafts[0].text, "Unverified draft.");
    assert_eq!(drafts[0].review, Some(TextReviewState::Interrupted));
    assert!(ended
        .snapshot
        .projection
        .parts
        .values()
        .all(AgentPart::is_terminal));
}

#[test]
fn rejected_streamed_draft_is_preserved_and_revision_is_appended() {
    use nola_core::agent::parts::TextReviewState;
    let mut config = source_core_config();
    config.turn_identity.supports_reviewed_text_parts = true;
    let mut core = AgentCore::try_new(config).unwrap();
    let started = core
        .submit(user_input(
            "stream-revision",
            "Answer from evidence",
            AgentInputDelivery::CurrentTurn,
        ))
        .unwrap();
    let model = model_request(&started.host_requests);
    submit_response(
        &mut core,
        &model,
        1,
        model_delta(&model, "Unsupported answer."),
    );
    let candidate = submit_response(
        &mut core,
        &model,
        2,
        model_requested_review("review-unsupported", "Unsupported answer."),
    );
    let audit = model_request(&candidate.host_requests);
    let rejected = submit_response(
        &mut core,
        &audit,
        1,
        visible_model_completion(
            "reject",
            r#"{"decision":"revise","outcome":"incomplete","items":[],"violations":["unsupported claim"]}"#,
        ),
    );
    assert!(rejected
        .snapshot
        .projection
        .parts
        .values()
        .any(|part| matches!(part, AgentPart::Text(text)
        if text.text == "Unsupported answer." && text.review == Some(TextReviewState::Rejected))));
    let revision = model_request(&rejected.host_requests);
    submit_response(
        &mut core,
        &revision,
        1,
        model_delta(&revision, "I cannot establish that."),
    );
    let next = submit_response(
        &mut core,
        &revision,
        2,
        visible_model_completion("revision", "I cannot establish that."),
    );
    let texts: Vec<_> = next
        .snapshot
        .projection
        .order
        .iter()
        .filter_map(|id| match &next.snapshot.projection.parts[id] {
            AgentPart::Text(text) if !text.text.is_empty() => Some(text),
            _ => None,
        })
        .collect();
    assert_eq!(texts.len(), 2);
    assert_eq!(texts[0].text, "Unsupported answer.");
    assert_eq!(texts[1].text, "I cannot establish that.");
    assert_eq!(texts[1].review, Some(TextReviewState::Pending));
}

#[test]
fn evidence_session_can_complete_a_greeting_without_any_tool_calls() {
    for greeting in ["你好", "Hello"] {
        let mut core = AgentCore::try_new(source_core_config()).unwrap();
        let started = core
            .submit(user_input(
                "greeting",
                greeting,
                AgentInputDelivery::CurrentTurn,
            ))
            .unwrap();
        let request = model_request(&started.host_requests);
        let candidate = submit_response(
            &mut core,
            &request,
            1,
            model_requested_review("review-greeting", greeting),
        );
        let audit = model_request(&candidate.host_requests);
        let HostRequest::ModelStream {
            messages,
            tools,
            response_mode,
            ..
        } = &audit.payload
        else {
            unreachable!()
        };
        assert!(tools.is_empty());
        assert_eq!(*response_mode, ModelResponseMode::StructuredAudit);
        assert_eq!(messages[0]["role"], "system");
        let instruction = messages[0]["content"].as_str().unwrap();
        assert!(instruction.contains("applies ONLY to YOU, never to candidate_answer"));
        assert_eq!(messages[1]["role"], "user");
        let data = messages[1]["content"].as_str().unwrap();
        let packet: Value = serde_json::from_str(data).unwrap();
        assert_eq!(packet["candidate_answer"], greeting);
        assert!(packet["original_user_messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["content"] == greeting));
        assert!(!packet["current_turn_user_message_ids"]
            .as_array()
            .unwrap()
            .is_empty());
        assert_eq!(packet["successful_mutation_call_ids"], json!([]));
        assert!(packet["evidence"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| !packet["original_user_messages"].as_array().unwrap()
                .iter().any(|original| original["id"] == item["id"])),
            "runtime facts use user-role transport but are not original user requests");
        for message in packet["original_user_messages"].as_array().unwrap() {
            assert!(packet["context_item_order"]
                .as_array()
                .unwrap()
                .contains(&message["id"]));
        }
        let reviewed = submit_response(
            &mut core,
            &audit,
            1,
            visible_model_completion(
                "audit",
                r#"{"decision":"accept","outcome":"answered","constraints":[],"items":[{"label":"Fixture task requirement","eligibility":"eligible","evidence":"Observed fixture evidence supports the candidate"}],"violations":[]}"#,
            ),
        );
        let completed = reviewed;
        assert!(
            completed.host_requests.is_empty(),
            "reviewed greeting must not be regenerated"
        );
        assert!(completed
            .event_envelopes
            .iter()
            .any(|event| matches!(event.event, AgentEvent::TurnCompleted { .. })));
        assert!(completed
            .snapshot
            .projection
            .parts
            .values()
            .all(|part| !matches!(part,
            AgentPart::Tool(tool) if tool.capability_id != "request_completion_review")));
    }
}

#[test]
fn independent_review_is_disabled_by_default_even_with_evidence_capabilities() {
    for greeting in ["你好", "Hello"] {
        let config = source_core_config();
        let mut core = AgentCore::try_new(config).unwrap();
        let started = core
            .submit(user_input(
                "direct-greeting",
                greeting,
                AgentInputDelivery::CurrentTurn,
            ))
            .unwrap();
        let request = model_request(&started.host_requests);
        let completed = submit_response(
            &mut core,
            &request,
            1,
            visible_model_completion("direct-answer", greeting),
        );
        assert!(
            completed.host_requests.is_empty(),
            "ordinary answer must not request a reviewer"
        );
        assert_eq!(
            completed.snapshot.turn_status,
            nola_core::agent::AgentTurnStatus::Completed
        );
    }
}

#[test]
fn executed_evidence_tool_does_not_enable_independent_review_without_opt_in() {
    let config = source_core_config();
    let mut core = AgentCore::try_new(config).unwrap();
    let started = core
        .submit(user_input(
            "direct-evidence",
            "Summarize the result",
            AgentInputDelivery::CurrentTurn,
        ))
        .unwrap();
    let planned = submit_response(
        &mut core,
        &model_request(&started.host_requests),
        1,
        empty_model_completion(vec![named_tool_call(
            "search-once",
            "web_search",
            json!({"query":"fixture"}),
        )]),
    );
    let search = request_matching(&planned.host_requests, |payload| {
        matches!(payload,
        HostRequest::ExecuteTool { call_id, .. } if call_id == "search-once")
    });
    let observed = complete_tool(&mut core, &search, "search-once");
    let synthesis = model_request(&observed.host_requests);
    let completed = submit_response(
        &mut core,
        &synthesis,
        1,
        visible_model_completion("direct-summary", "The search returned a result."),
    );
    assert!(
        completed.host_requests.is_empty(),
        "tool use must not schedule a reviewer by default"
    );
    assert_eq!(
        completed.snapshot.turn_status,
        nola_core::agent::AgentTurnStatus::Completed
    );
}

#[test]
fn goal_work_control_is_durable_without_host_execution_or_user_provenance_forgery() {
    use nola_core::agent::{GoalRuntimeCommand, GoalRuntimeCommandEnvelope, GoalRequirement, GoalRecordStatus};
    let config = source_core_config();
    let mut core = AgentCore::try_new(config.clone()).unwrap();
    core.submit(AgentSubmission::GoalRuntime { envelope: GoalRuntimeCommandEnvelope {
        operation_id:"start-work-goal".into(), command:GoalRuntimeCommand::Start {
            id:"work-goal".into(),objective:"Implement with no deployment".into(), constraints:vec!["No deployment".into()],
            requirements:vec![GoalRequirement { acceptance_policy: nola_core::agent::goal::GoalAcceptancePolicy::ModelJudgment, semantic_revision: 1, id:"r".into(),description:"Tests pass".into(),dependencies:vec![],
                status:GoalRecordStatus::Pending,evidence_refs:vec![] }],now_epoch_ms:0,
        }
    }}).unwrap();
    let started = core.submit(AgentSubmission::GoalUserInput {
        items:vec![AgentItem::user_message(ItemId::from_string("user-extra".into()), "保留已有数据，不要部署")],
        turn_context_items:vec![],turn_context_platform:Default::default(),turn_context_components:vec![],
        submission_id:Some("goal-user".into()),submitted_at_epoch_ms:None,delivery:AgentInputDelivery::CurrentTurn,
    }).unwrap();
    assert_eq!(started.snapshot.goal.active.as_ref().unwrap().work.user_updates[0].content, "保留已有数据，不要部署");
    let revision = started.snapshot.goal.revision;
    let request = model_request(&started.host_requests);
    let HostRequest::ModelStream { tools, .. } = &request.payload else { unreachable!() };
    assert!(tools.iter().any(|tool| tool["function"]["name"] == "goal_update"));
    let args = json!({"goal_id":"work-goal","expected_revision":revision,"entries":[{
        "id":"next-check","kind":"next_action","content":"Run remaining tests",
        "requirement_ids":["r"],"task_ids":[],"evidence_refs":[],"progress":null,"supersedes":null
    }]});
    let updated = submit_response(&mut core, &request, 1,
        empty_model_completion(vec![named_tool_call("update-work", "goal_update", args.clone())]));
    assert_eq!(updated.snapshot.goal.revision, revision + 1);
    assert!(updated.host_requests.iter().all(|request| matches!(request.payload, HostRequest::ModelStream { .. })));
    let goal = updated.snapshot.goal.active.as_ref().unwrap();
    assert_eq!(goal.work.current()[0].origin, nola_core::agent::GoalWorkOrigin::Model);
    assert_eq!(goal.requirements["r"].status, GoalRecordStatus::Pending);
    let checkpoint = core.checkpoint(CheckpointReason::GoalUpdated);
    let mut restored = AgentCore::restore(config, checkpoint).unwrap();
    assert_eq!(restored.snapshot().goal, updated.snapshot.goal);
    let next = model_request(&updated.host_requests);
    let mut forged = args;
    forged["expected_revision"] = json!(revision + 1);
    forged["entries"][0]["id"] = json!("forged");
    forged["entries"][0]["origin"] = json!("user");
    let rejected = submit_response(&mut restored, &next, 1,
        empty_model_completion(vec![named_tool_call("forged-work", "goal_update", forged)]));
    assert_eq!(rejected.snapshot.goal, updated.snapshot.goal);
    let queued = restored.submit(AgentSubmission::GoalUserInput {
        items:vec![AgentItem::user_message(ItemId::from_string("a-later-message".into()), "只验证本地结果")],
        turn_context_items:vec![],turn_context_platform:Default::default(),turn_context_components:vec![],
        submission_id:Some("goal-follow-up".into()),submitted_at_epoch_ms:None,delivery:AgentInputDelivery::CurrentTurn,
    }).unwrap();
    let user_updates = &queued.snapshot.goal.active.as_ref().unwrap().work.user_updates;
    assert_eq!(user_updates.len(), 2);
    assert_eq!(user_updates[1].message_id, "a-later-message");
    assert_eq!(user_updates[1].content, "只验证本地结果");
    assert_eq!(queued.snapshot.goal.revision, rejected.snapshot.goal.revision + 1);
}

#[test]
fn model_can_request_review_with_a_typed_control_call() {
    let config = source_core_config();
    let mut core = AgentCore::try_new(config).unwrap();
    let started = core
        .submit(user_input(
            "model-selected-review",
            "Check this answer",
            AgentInputDelivery::CurrentTurn,
        ))
        .unwrap();
    let generation = model_request(&started.host_requests);
    let HostRequest::ModelStream { tools, .. } = &generation.payload else {
        unreachable!()
    };
    assert!(tools
        .iter()
        .any(|tool| tool["function"]["name"] == "request_completion_review"));
    let requested = submit_response(
        &mut core,
        &generation,
        1,
        empty_model_completion(vec![named_tool_call(
            "review-call",
            "request_completion_review",
            json!({"candidate_answer":"Here is the proposed answer."}),
        )]),
    );
    let review = model_request(&requested.host_requests);
    assert!(matches!(
        review.payload,
        HostRequest::ModelStream {
            response_mode: ModelResponseMode::StructuredAudit,
            ..
        }
    ));
    let completed = submit_response(
        &mut core,
        &review,
        1,
        visible_model_completion(
            "model-selected-report",
            r#"{"decision":"accept","outcome":"answered","constraints":[],"items":[{"label":"answer","eligibility":"eligible","evidence":"The candidate answers the request."}],"violations":[]}"#,
        ),
    );
    assert_eq!(
        completed.snapshot.turn_status,
        nola_core::agent::AgentTurnStatus::Completed
    );
    assert!(completed
        .snapshot
        .projection
        .parts
        .values()
        .any(|part| matches!(part,
        AgentPart::Text(text) if text.text == "Here is the proposed answer.")));
}

#[test]
fn goal_acceptance_requires_exact_claims_and_committed_evidence_without_auditor() {
    use nola_core::agent::*;
    let mut core = AgentCore::try_new(source_core_config()).unwrap();
    let command = |operation: &str, command| AgentSubmission::GoalRuntime {
        envelope: GoalRuntimeCommandEnvelope { operation_id: operation.into(), command }
    };
    core.submit(command("start", GoalRuntimeCommand::Start {
        id:"accept-goal".into(), objective:"Write a greeting".into(), constraints:vec![],
        requirements:vec![GoalRequirement { acceptance_policy: nola_core::agent::goal::GoalAcceptancePolicy::ModelJudgment, semantic_revision: 1, id:"r".into(),description:"Greeting delivered".into(),
            dependencies:vec![],status:GoalRecordStatus::Pending,evidence_refs:vec![] }], now_epoch_ms:0
    })).unwrap();
    core.submit(command("plan", GoalRuntimeCommand::PlanContract {
        contract:GoalSubtaskContract { execution_run_id: None, id:"contract".into(), round:1, objective:"Greeting".into(),
            requirement_versions: std::collections::BTreeMap::from([("r".into(), 1)]),
            proposed_checks:vec!["Greeting delivered".into()],additional_execution_constraints:vec![],
            relevant_record_ids:vec!["r".into()],status:GoalContractStatus::Planned,created_at_epoch_ms:1 },
        now_epoch_ms:1
    })).unwrap();
    core.submit(command("begin", GoalRuntimeCommand::BeginExecution { run_id: "greeting".into(), contract_id:"contract".into(),now_epoch_ms:2 })).unwrap();
    let started = core.submit(AgentSubmission::GoalUserInput {
        items:vec![AgentItem::user_message(ItemId::from_string("greeting-input".into()), "Say hello")],
        turn_context_items:vec![],turn_context_platform:Default::default(),turn_context_components:vec![],
        submission_id:Some("greeting".into()),submitted_at_epoch_ms:None,delivery:AgentInputDelivery::CurrentTurn,
    }).unwrap();
    let request = model_request(&started.host_requests);
    submit_response(&mut core, &request, 1, visible_model_completion("answer", "Hello!"));
    core.submit(command("report", GoalRuntimeCommand::RecordExecution { report:GoalExecutionReport {
        contract_id:"contract".into(),summary:"Greeting delivered".into(),claimed_record_ids:vec!["r".into()],
        artifact_refs:vec![],blockers:vec![],reported_at_epoch_ms:3
    }})).unwrap();
    let before = core.snapshot().goal;
    // A sealed source turn is not proof of task completion. This must remain
    // true after checkpoint restore, before any Goal acceptance can mutate it.
    for outcome in [
        Some(nola_domain::TaskCompletionOutcome::Blocked),
        Some(nola_domain::TaskCompletionOutcome::Incomplete),
        Some(nola_domain::TaskCompletionOutcome::NeedsInput),
        None,
    ] {
        let mut checkpoint = core.checkpoint(nola_core::agent::recovery::CheckpointReason::Manual);
        checkpoint.state.execution.active_turn.as_mut().unwrap().completion_outcome = outcome;
        let mut restored = AgentCore::restore(source_core_config(), checkpoint).unwrap();
        let error = restored.submit(command("unfinished-accept", GoalRuntimeCommand::AcceptCompletion {
            expected_revision:before.revision,contract_id:"contract".into(),
            claims:vec![GoalCompletionClaim { execution_evidence_refs: vec![], task_replacements: vec![],  requirement_revision: 1, requirement_id:"r".into(),summary:"claimed done".into(),evidence_refs:vec!["response".into()] }],now_epoch_ms:4
        })).unwrap_err();
        assert!(error.contains("unfinished source task outcome"), "{error}");
        assert_eq!(restored.snapshot().goal, before);
    }
    let mut checkpoint = core.checkpoint(nola_core::agent::recovery::CheckpointReason::Manual);
    checkpoint.state.execution.requirements.decision = None;
    let mut restored = AgentCore::restore(source_core_config(), checkpoint).unwrap();
    let error = restored.submit(command("missing-result-accept", GoalRuntimeCommand::AcceptCompletion {
        expected_revision:before.revision,contract_id:"contract".into(),
        claims:vec![GoalCompletionClaim { execution_evidence_refs: vec![], task_replacements: vec![],  requirement_revision: 1, requirement_id:"r".into(),summary:"claimed done".into(),evidence_refs:vec!["response".into()] }],now_epoch_ms:4
    })).unwrap_err();
    assert!(error.contains("canonical source result decision"), "{error}");
    assert_eq!(restored.snapshot().goal, before);
    assert!(core.submit(command("stale-accept", GoalRuntimeCommand::AcceptCompletion {
        expected_revision:before.revision.saturating_sub(1),contract_id:"contract".into(),
        claims:vec![GoalCompletionClaim { execution_evidence_refs: vec![], task_replacements: vec![],  requirement_revision: 1, requirement_id:"r".into(),summary:"done".into(),evidence_refs:vec!["response".into()] }],now_epoch_ms:4
    })).is_err());
    assert_eq!(core.snapshot().goal, before);
    for (operation, claims) in [
        ("empty", vec![]),
        ("wrong-requirement-revision", vec![GoalCompletionClaim { execution_evidence_refs: vec![], task_replacements: vec![],  requirement_revision: 2, requirement_id:"r".into(),summary:"done".into(),evidence_refs:vec!["response".into()] }]),
        ("fabricated", vec![GoalCompletionClaim { execution_evidence_refs: vec![], task_replacements: vec![],  requirement_revision: 1, requirement_id:"r".into(),summary:"done".into(),evidence_refs:vec!["tool:nonexistent".into()] }]),
    ] {
        assert!(core.submit(command(operation, GoalRuntimeCommand::AcceptCompletion {
            expected_revision:before.revision,contract_id:"contract".into(),claims,now_epoch_ms:4
        })).is_err());
        assert_eq!(core.snapshot().goal, before);
    }
    let accepted = core.submit(command("accept", GoalRuntimeCommand::AcceptCompletion {
        expected_revision:before.revision,contract_id:"contract".into(),
        claims:vec![GoalCompletionClaim { execution_evidence_refs: vec![], task_replacements: vec![],  requirement_revision: 1, requirement_id:"r".into(),summary:"Greeting appears in the answer".into(),evidence_refs:vec!["response".into()] }],now_epoch_ms:4
    })).unwrap();
    assert!(accepted.host_requests.is_empty());
    let goal = accepted.snapshot.goal.active.unwrap();
    assert_eq!(goal.status, GoalStatus::Completed);
    let audit = goal.audit_reports.last().unwrap();
    let acceptance = goal.requirement_acceptances.last().unwrap();
    assert_eq!(acceptance.requirement_id, "r");
    assert_eq!(acceptance.requirement_revision, goal.requirements["r"].semantic_revision);
    assert_eq!(acceptance.audit_id, audit.id);
    assert!(matches!(acceptance.assessor, nola_core::agent::goal::GoalAssessor::ModelTurn { .. }));
    assert!(audit.evidence.iter().all(|item| item.source_verified));
    assert!(audit.findings.iter().all(|finding| finding.assessment_method
        == nola_core::agent::goal::GoalAssessmentMethod::ModelJudgment));
    let encoded = serde_json::to_value(audit).unwrap();
    assert!(encoded["evidence"][0].get("independently_verified").is_none());
    let mut missing_method = encoded["findings"][0].clone();
    missing_method.as_object_mut().unwrap().remove("assessment_method");
    assert!(serde_json::from_value::<nola_core::agent::goal::GoalAuditFinding>(missing_method).is_err(),
        "old findings must not silently acquire an assessment method");
    let checkpoint = core.checkpoint(CheckpointReason::Manual);
    let mut restored = AgentCore::restore(source_core_config(),
        serde_json::from_value(serde_json::to_value(checkpoint).unwrap()).unwrap()).unwrap();
    assert_eq!(restored.snapshot().goal.active.unwrap().audit_reports, goal.audit_reports);
    let before_replay = restored.snapshot();
    let replay = restored.submit(command("accept", GoalRuntimeCommand::AcceptCompletion {
        expected_revision:before.revision,contract_id:"contract".into(),
        claims:vec![GoalCompletionClaim { execution_evidence_refs:vec![],task_replacements:vec![],
            requirement_revision:1,requirement_id:"r".into(),summary:"Greeting appears in the answer".into(),
            evidence_refs:vec!["response".into()] }],now_epoch_ms:4,
    })).unwrap();
    assert_eq!(replay.snapshot.goal,before_replay.goal);
    assert_eq!(replay.snapshot.revision,before_replay.revision);
    assert!(replay.event_envelopes.is_empty());
}

#[test]
fn review_control_call_rejects_mixed_tool_batch_without_executing_it() {
    let mut core = AgentCore::try_new(source_core_config()).unwrap();
    let started = core
        .submit(user_input(
            "mixed-review",
            "Check and write",
            AgentInputDelivery::CurrentTurn,
        ))
        .unwrap();
    let generation = model_request(&started.host_requests);
    let rejected = submit_response(
        &mut core,
        &generation,
        1,
        empty_model_completion(vec![
            named_tool_call(
                "review-mixed",
                "request_completion_review",
                json!({"candidate_answer":"Proposed answer."}),
            ),
            named_tool_call(
                "write-mixed",
                "write_file",
                json!({"path":"/tmp/should-not-write","content":"unsafe"}),
            ),
        ]),
    );
    assert!(rejected
        .host_requests
        .iter()
        .all(|request| matches!(request.payload, HostRequest::ModelStream { .. })));
    let next = model_request(&rejected.host_requests);
    assert!(matches!(
        next.payload,
        HostRequest::ModelStream {
            response_mode: ModelResponseMode::Conversational,
            ..
        }
    ));
    assert!(core
        .checkpoint(CheckpointReason::Manual)
        .state
        .conversation
        .model_context
        .iter()
        .any(|item| matches!(item, AgentItem::ToolResult { is_error: true, .. })));
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

#[test]
fn completed_mutation_requires_effect_accounting_without_reexecution() {
    for prompt in ["保存这个结果", "Save this result"] {
        let mut config = core_config(Vec::new());
        let descriptor = config
            .host_catalog
            .as_mut()
            .unwrap()
            .capabilities
            .get_mut("write_file")
            .unwrap();
        descriptor
            .input_normalization
            .requires_validation_after_mutation = false;
        config
            .permission_rules
            .rules
            .push(nola_core::agent::permissions::PermissionRule {
                capability_pattern: "*".into(),
                resource_pattern: "*".into(),
                decision: nola_core::agent::permissions::PermissionPolicyDecision::Allow,
                source: nola_core::agent::permissions::PermissionRuleSource::UserGlobal,
            });
        let mut core = AgentCore::try_new(config.clone()).unwrap();
        let started = core
            .submit(user_input(
                "effect-accounting",
                prompt,
                AgentInputDelivery::CurrentTurn,
            ))
            .unwrap();
        let scheduled = submit_response(
            &mut core,
            &model_request(&started.host_requests),
            1,
            empty_model_completion(vec![named_tool_call(
                "write-once",
                "write_file",
                json!({"path":"/tmp/result", "content":"value"}),
            )]),
        );
        let execute = request_matching(&scheduled.host_requests, |request| {
            matches!(request, HostRequest::ExecuteTool { .. })
        });
        let written = submit_response(
            &mut core,
            &execute,
            1,
            HostResponse::ToolCompleted {
                call_id: "write-once".into(),
                output: observed_output(
                    json!({"written":true}),
                    &[("file:///tmp/result", "v1")],
                    &[],
                ),
                outcome: nola_core::agent::host::HostToolOutcome::Succeeded,
                activation: None,
                full_output_ref: None,
                artifacts: Vec::new(),
            },
        );
        let resumable = core.checkpoint(CheckpointReason::Manual);
        let candidate = submit_response(
            &mut core,
            &model_request(&written.host_requests),
            1,
            model_requested_review("review-saved-result", "Saved the result."),
        );
        let audit = model_request(&candidate.host_requests);
        let report = |outcome: &str, ids: Value| {
            json!({"decision":"accept", "outcome":outcome,
            "effect_call_ids":ids, "constraints":["save result"],
            "items":[{"label":"saved result", "eligibility":"eligible", "evidence":"write-once returned actual v1"}], "violations":[]}).to_string()
        };
        let retry = submit_response(
            &mut core,
            &audit,
            1,
            visible_model_completion("misclassified", &report("answered", json!([]))),
        );
        assert!(!retry
            .event_envelopes
            .iter()
            .any(|event| matches!(event.event, AgentEvent::TurnCompleted { .. })));
        let audit_retry = model_request(&retry.host_requests);
        assert!(
            matches!(&audit_retry.payload, HostRequest::ModelStream { tools, response_mode: ModelResponseMode::StructuredAudit, .. } if tools.is_empty())
        );
        assert_eq!(
            core.checkpoint(CheckpointReason::Manual)
                .state
                .execution
                .active_turn
                .as_ref()
                .unwrap()
                .final_answer_draft
                .as_deref(),
            Some("Saved the result.")
        );
        let complete = submit_response(
            &mut core,
            &audit_retry,
            1,
            visible_model_completion(
                "corrected",
                &report("action_completed", json!(["write-once"])),
            ),
        );
        assert!(complete.host_requests.is_empty());
        assert!(complete.event_envelopes.iter().any(|event| matches!(&event.event,
            AgentEvent::RunResultDecided { decision, .. }
                if matches!(decision.value, nola_domain::run_result::RunResultValue::Incomplete { .. }))));
        // Review prose is not a requirement assessment. The model explicitly
        // binds the observed write evidence and proposes completion, without
        // executing the mutation for a second time.
        core = AgentCore::restore(config, resumable).unwrap();
        let scoped=declare_fixture_action_scope(&mut core,&model_request(&written.host_requests),"file:///tmp/result","v1",false);
        let response = observed_action_completion(&core, "Saved the result.", &["write-once"]);
        let complete = submit_response(&mut core, &scoped, 1,
            response);
        assert!(complete.host_requests.is_empty());
        assert!(complete.event_envelopes.iter().any(|event| matches!(
            event.event,
            AgentEvent::TurnCompleted {
                outcome: Some(nola_domain::TaskCompletionOutcome::ActionCompleted),
                ..
            }
        )));
        assert_eq!(complete.snapshot.engineering.mutation_count, 1);
    }
}

#[test]
fn commit_command_prose_and_fabricated_receipts_cannot_complete_an_action() {
    let mut core = AgentCore::try_new(source_core_config()).unwrap();
    let started = core
        .submit(user_input(
            "commit",
            "提交下",
            AgentInputDelivery::CurrentTurn,
        ))
        .unwrap();
    let request = model_request(&started.host_requests);
    let candidate = submit_response(
        &mut core,
        &request,
        1,
        model_requested_review(
            "review-commit-claim",
            "$ git add -A\n$ git commit -m example",
        ),
    );
    let review = model_request(&candidate.host_requests);
    let rejected = submit_response(
        &mut core,
        &review,
        1,
        visible_model_completion(
            "false-receipt",
            r#"{"decision":"accept","outcome":"action_completed","effect_call_ids":["invented-commit"],"constraints":[],"items":[{"label":"Fixture task requirement","eligibility":"eligible","evidence":"Observed fixture evidence supports the candidate"}],"violations":[]}"#,
        ),
    );
    assert!(!rejected
        .event_envelopes
        .iter()
        .any(|event| matches!(event.event, AgentEvent::TurnCompleted { .. })));
    assert!(rejected
        .snapshot
        .projection
        .parts
        .values()
        .all(|part| !matches!(part, AgentPart::Text(_))));
    assert_eq!(rejected.snapshot.completion_outcome, None);
    let resumed = model_request(&rejected.host_requests);
    assert!(
        matches!(&resumed.payload,
        HostRequest::ModelStream { tools, response_mode: ModelResponseMode::StructuredAudit, .. }
            if tools.is_empty()),
        "invalid receipt accounting must not reopen execution"
    );
    let correction = submit_response(
        &mut core,
        &resumed,
        1,
        visible_model_completion(
            "correct-audit",
            r#"{"decision":"revise","outcome":"incomplete","constraints":[],"items":[],"violations":["Commands are only a plan; ask the missing scope question without claiming execution."]}"#,
        ),
    );
    let resumed = model_request(&correction.host_requests);
    let question = "发现未跟踪的大型可执行文件。提交是否排除它？当前尚未提交。";
    let candidate = submit_response(
        &mut core,
        &resumed,
        1,
        visible_model_completion("question", question),
    );
    let review = model_request(&candidate.host_requests);
    let waiting = submit_response(
        &mut core,
        &review,
        1,
        visible_model_completion(
            "clarification",
            r#"{"decision":"clarify","outcome":"needs_input","question_set":{"questions":[{"id":"exclude","prompt":"是否排除未跟踪的可执行文件？","options":[{"id":"one","label":"排除","description":null}],"required":true,"allow_freeform":true,"max_text_chars":256}]},"constraints":[],"items":[{"label":"是否排除","eligibility":"unknown","evidence":"user choice missing"}],"violations":[]}"#,
        ),
    );
    assert!(
        waiting.host_requests.is_empty(),
        "no final rewrite may delete the question"
    );
    assert_eq!(
        waiting.snapshot.completion_outcome,
        Some(nola_domain::TaskCompletionOutcome::NeedsInput)
    );
    assert_eq!(waiting.snapshot.turn_status,nola_domain::AgentTurnStatus::AwaitingInput);
    assert!(!waiting.event_envelopes.iter().any(|event| matches!(event.event,AgentEvent::TurnCompleted {..})));
    assert!(waiting
        .snapshot
        .projection
        .parts
        .values()
        .any(|part| matches!(part, AgentPart::Text(text) if text.text == question)));
    let answered = core.submit(answer_wait(&waiting.snapshot.open_user_input_waits[0],"reviewed-question-answer")).unwrap();
    model_request(&answered.host_requests);
    assert!(answered.snapshot.open_user_input_waits.is_empty());
}

#[test]
fn reviewed_blocker_without_observed_fact_is_incomplete_not_execution_failure() {
    let mut core = AgentCore::try_new(source_core_config()).unwrap();
    let started = core
        .submit(user_input(
            "blocked",
            "完成操作",
            AgentInputDelivery::CurrentTurn,
        ))
        .unwrap();
    let candidate = submit_response(
        &mut core,
        &model_request(&started.host_requests),
        1,
        model_requested_review("review-blocker", "操作尚未完成：所需服务不可用。"),
    );
    let reviewed = submit_response(
        &mut core,
        &model_request(&candidate.host_requests),
        1,
        visible_model_completion(
            "audit",
            r#"{"decision":"blocked","outcome":"blocked","constraints":[],"items":[],"violations":[]}"#,
        ),
    );
    assert_eq!(
        reviewed.snapshot.turn_status,
        nola_core::agent::AgentTurnStatus::Completed
    );
    assert_eq!(
        reviewed.snapshot.completion_outcome,
        Some(nola_domain::TaskCompletionOutcome::Incomplete)
    );
    assert!(reviewed.event_envelopes.iter().any(|event| matches!(&event.event,
        AgentEvent::RunResultDecided { decision, .. }
            if matches!(&decision.value, nola_domain::run_result::RunResultValue::Incomplete {
                reason, remaining_requirement_refs, ..
            } if reason.code == "requirements.unassessed" && !remaining_requirement_refs.is_empty())
    )));
    assert!(!reviewed.event_envelopes.iter().any(|event|
        matches!(&event.event, AgentEvent::TurnFailed { .. })));
    assert!(reviewed.host_requests.is_empty());
}

#[test]
fn engineering_loop_corrects_failed_validation_before_completing() {
    let mut core = AgentCore::try_new(engineering_core_config()).expect("create Core");
    let started = core
        .submit(user_input(
            "engineering-user",
            "fix the implementation and test it",
            AgentInputDelivery::CurrentTurn,
        ))
        .expect("start engineering turn");
    let first_model = model_request(&started.host_requests);
    let permission = submit_response(
        &mut core,
        &first_model,
        1,
        empty_model_completion(vec![named_tool_call(
            "write-1",
            "write_file",
            json!({"path": "/tmp/example.rs", "content": "broken"}),
        )]),
    );
    let permission_request = request_matching(&permission.host_requests, |payload| {
        matches!(payload, HostRequest::RequestPermission { .. })
    });
    let permission_request_id = match &permission_request.payload {
        HostRequest::RequestPermission {
            permission_request_id,
            ..
        } => permission_request_id.clone(),
        _ => unreachable!(),
    };
    let allowed = core
        .submit(AgentSubmission::PermissionResponse {
            request_id: nola_core::agent::PermissionRequestId::from_string(permission_request_id),
            decision: PermissionUserDecision::AllowConversation,
            grant: Some(json!({
                "scope": "conversation",
                "resource_pattern": "*"
            })),
        })
        .expect("allow workspace writes for the conversation");
    let write_request = request_matching(
        &allowed.host_requests,
        |payload| matches!(payload, HostRequest::ExecuteTool { call_id, .. } if call_id == "write-1"),
    );
    let after_write = submit_response(
        &mut core,
        &write_request,
        1,
        HostResponse::ToolCompleted {
            call_id: "write-1".to_string(),
            output: observed_output(
                json!({"path": "/tmp/example.rs", "status": "written"}),
                &[("file:///tmp/example.rs", "v1")],
                &[],
            ),
            outcome: nola_core::agent::host::HostToolOutcome::Succeeded,
            activation: None,
            full_output_ref: None,
            artifacts: Vec::new(),
        },
    );
    assert_eq!(
        after_write.snapshot.engineering.phase,
        AgentEngineeringPhase::Modifying
    );
    assert!(after_write.snapshot.engineering.validation_required);
    assert_eq!(after_write.snapshot.engineering.mutation_count, 1);
    assert!(core
        .checkpoint(CheckpointReason::Manual)
        .state
        .execution
        .active_turn
        .as_ref()
        .unwrap()
        .successful_mutation_calls
        .contains("write-1"));
    assert!(after_write.trace_events.iter().any(|trace| {
        trace.kind == nola_core::agent_trace::AgentTraceKind::ValidationUpdated
            && trace.payload.phase.as_deref() == Some("modifying")
            && trace
                .payload
                .validation
                .as_ref()
                .is_some_and(|validation| validation.status == "required")
    }));
    let checkpoint = checkpoint_after(&core, &after_write);
    let restored = AgentCore::restore(engineering_core_config(), checkpoint)
        .expect("restore engineering checkpoint");
    assert!(restored
        .checkpoint(CheckpointReason::Manual)
        .state
        .execution
        .active_turn
        .as_ref()
        .unwrap()
        .successful_mutation_calls
        .contains("write-1"));
    assert_eq!(
        restored.snapshot().engineering,
        after_write.snapshot.engineering,
        "engineering-loop evidence must survive process recovery"
    );
    let after_write_model = model_request(&after_write.host_requests);
    // An unfinished write must not prevent the independent task review from
    // discovering missing work. Exercise the boundary after a cold restore.
    for decision in ["execute", "accept"] {
        let mut checkpoint = core.checkpoint(CheckpointReason::Manual);
        checkpoint
            .state
            .execution
            .active_turn
            .as_mut()
            .unwrap()
            .final_answer_audit_required = true;
        let mut audited = AgentCore::restore(engineering_core_config(), checkpoint).unwrap();
        submit_response(
            &mut audited,
            &after_write_model,
            1,
            model_delta(&after_write_model, "All requested files are done."),
        );
        let candidate = submit_response(
            &mut audited,
            &after_write_model,
            2,
            visible_model_completion("pending-candidate", "All requested files are done."),
        );
        let review = model_request(&candidate.host_requests);
        assert!(matches!(
            review.payload,
            HostRequest::ModelStream {
                response_mode: ModelResponseMode::StructuredAudit,
                ..
            }
        ));
        assert!(candidate.snapshot.engineering.validation_required);
        let report = json!({"decision":decision,
            "outcome":if decision == "execute" { "incomplete" } else { "action_completed" },
            "effect_call_ids":["write-1"], "constraints":["all requested files"],
            "items":[{"label":"scope", "eligibility":"eligible", "evidence":"write-1 completed"}],
            "violations":[]})
        .to_string();
        submit_response(&mut audited, &review, 1, model_delta(&review, &report));
        let resumed = submit_response(
            &mut audited,
            &review,
            2,
            visible_model_completion("pending-review", &report),
        );
        assert!(resumed.snapshot.engineering.validation_required);
        if decision == "accept" {
            assert!(resumed.event_envelopes.iter().any(|event| matches!(
                &event.event, AgentEvent::RunResultDecided { decision, .. }
                    if matches!(decision.value, nola_domain::run_result::RunResultValue::Incomplete { .. })
            )));
            assert!(!resumed.host_requests.iter().any(|request| matches!(request.payload, HostRequest::ModelStream { .. })));
            continue;
        }
        assert!(!resumed
            .event_envelopes
            .iter()
            .any(|event| matches!(event.event, AgentEvent::TurnCompleted { .. })));
        let next = model_request(&resumed.host_requests);
        if let HostRequest::ModelStream {
            tools,
            response_mode,
            ..
        } = &next.payload
        {
            assert!(tools.iter().any(|tool| tool.pointer("/function/name") == Some(&json!("write_file"))));
            assert_eq!(*response_mode, ModelResponseMode::Conversational);
        } else {
            panic!("expected resumed model request");
        }
    }
    let after_write_messages = match &after_write_model.payload {
        HostRequest::ModelStream { messages, .. } => messages,
        _ => unreachable!(),
    };
    assert!(after_write_messages.iter().any(|message| {
        message
            .get("content")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|content| content.contains("no successful validation"))
    }));

    let resumable = core.checkpoint(CheckpointReason::Manual);
    submit_response(&mut core, &after_write_model, 1,
        model_delta(&after_write_model, "The fix is complete."));
    let premature = submit_response(
        &mut core,
        &after_write_model,
        2,
        empty_model_completion(Vec::new()),
    );
    assert!(premature.event_envelopes.iter().any(|event| matches!(&event.event,
        AgentEvent::RunResultDecided { decision, .. }
            if matches!(decision.value, nola_domain::run_result::RunResultValue::Incomplete { .. }))));
    assert!(premature.host_requests.is_empty());
    // A model that elects to continue validates from the pre-completion fork;
    // Core does not revive a terminal Run or force another tool request.
    core = AgentCore::restore(engineering_core_config(), resumable).unwrap();
    let validation_model = after_write_model.clone();
    if let HostRequest::ModelStream {
        response_mode,
        tools,
        ..
    } = &validation_model.payload
    {
        assert_eq!(*response_mode, ModelResponseMode::Conversational);
        let shell = tools
            .iter()
            .find(|tool| tool.pointer("/function/name") == Some(&json!("bash")))
            .unwrap();
        let HostRequest::ModelStream { tools: original_tools, .. } = &first_model.payload else {
            unreachable!("initial request must carry the model tool catalog")
        };
        let original_shell = original_tools.iter()
            .find(|tool| tool.pointer("/function/name") == Some(&json!("bash"))).unwrap();
        assert_eq!(shell.pointer("/function/parameters"), original_shell.pointer("/function/parameters"),
            "validation debt must preserve the actual catalog schema, not impose a validate-only protocol");
        assert!(tools
            .iter()
            .any(|tool| tool.pointer("/function/name") == Some(&json!("write_file"))));
    } else {
        panic!("expected validation request");
    }
    let validation_dispatch = submit_response(
        &mut core,
        &validation_model,
        1,
        empty_model_completion(vec![named_tool_call(
            "validate-1",
            "bash",
            json!({"command": "cargo test --quiet", "operation_kind": "validate"}),
        )]),
    );
    let validation_request = request_matching(
        &validation_dispatch.host_requests,
        |payload| matches!(payload, HostRequest::ExecuteTool { call_id, .. } if call_id == "validate-1"),
    );
    let failed_validation = submit_response(
        &mut core,
        &validation_request,
        1,
        HostResponse::ToolCompleted {
            call_id: "validate-1".to_string(),
            output: json!({"exit_code": 101, "stderr": "assertion failed"}),
            outcome: nola_core::agent::host::HostToolOutcome::Failed,
            activation: None,
            full_output_ref: None,
            artifacts: Vec::new(),
        },
    );
    assert_eq!(
        failed_validation.snapshot.engineering.phase,
        AgentEngineeringPhase::Correcting
    );
    assert!(failed_validation.snapshot.engineering.validation_required);
    assert_eq!(
        failed_validation
            .snapshot
            .engineering
            .validation_attempt_count,
        1
    );
    assert!(failed_validation
        .snapshot
        .engineering
        .last_failure
        .as_deref()
        .is_some_and(|failure| failure.contains("assertion failed")));
    assert!(failed_validation.trace_events.iter().any(|trace| {
        trace.kind == nola_core::agent_trace::AgentTraceKind::ValidationUpdated
            && trace.payload.phase.as_deref() == Some("correcting")
    }));
    let correction_model = model_request(&failed_validation.host_requests);
    let correction_messages = match &correction_model.payload {
        HostRequest::ModelStream { messages, .. } => messages,
        _ => unreachable!(),
    };
    assert!(correction_messages.iter().any(|message| {
        message["role"] == "system" && message
            .get("content")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|content| {
                content.contains("previous tool attempt failed")
                    && content.contains("runtime facts contain its diagnostic")
                    && !content.contains("assertion failed")
            })
    }));
    let (fact_message, facts) = correction_messages.iter().filter_map(|message| {
        let (_, payload) = message["content"].as_str()?.split_once('\n')?;
        let facts: Value = serde_json::from_str(payload).ok()?;
        (facts["schema"] == "nola.runtime_facts.v1" && facts["kind"] == "engineering-state")
            .then_some((message, facts))
    }).next().expect("separate engineering runtime facts");
    assert_eq!(fact_message["role"], "user");
    assert_eq!(fact_message["_nola_user_input"], false);
    assert_eq!(facts["facts"]["phase"], "correcting");
    assert_eq!(facts["facts"]["last_failure"],
        serde_json::to_value(&failed_validation.snapshot.engineering.last_failure).unwrap());
    assert!(facts["facts"]["last_failure"].as_str().unwrap().contains("assertion failed"));
    assert!(correction_messages.iter().filter(|message| message["role"] == "system")
        .all(|message| !message["content"].as_str().unwrap_or_default().contains("assertion failed")),
        "tool failure diagnostics must not be promoted into system policy");

    let correction_dispatch = submit_response(
        &mut core,
        &correction_model,
        1,
        empty_model_completion(vec![named_tool_call(
            "write-2",
            "write_file",
            json!({"path": "/tmp/example.rs", "content": "fixed"}),
        )]),
    );
    let correction_request = request_matching(
        &correction_dispatch.host_requests,
        |payload| matches!(payload, HostRequest::ExecuteTool { call_id, .. } if call_id == "write-2"),
    );
    let after_correction = submit_response(
        &mut core,
        &correction_request,
        1,
        HostResponse::ToolCompleted {
            call_id: "write-2".to_string(),
            output: observed_output(
                json!({"path": "/tmp/example.rs", "status": "written"}),
                &[("file:///tmp/example.rs", "v2")],
                &[],
            ),
            outcome: nola_core::agent::host::HostToolOutcome::Succeeded,
            activation: None,
            full_output_ref: None,
            artifacts: Vec::new(),
        },
    );
    assert_eq!(after_correction.snapshot.engineering.mutation_count, 2);
    assert!(after_correction.snapshot.engineering.validation_required);

    let second_validation_model = model_request(&after_correction.host_requests);
    let second_validation_dispatch = submit_response(
        &mut core,
        &second_validation_model,
        1,
        empty_model_completion(vec![named_tool_call(
            "validate-2",
            "bash",
            json!({"command": "cargo test --quiet", "operation_kind": "validate", "validation_targets": ["/tmp/example.rs"]}),
        )]),
    );
    let second_validation_request = request_matching(
        &second_validation_dispatch.host_requests,
        |payload| matches!(payload, HostRequest::ExecuteTool { call_id, .. } if call_id == "validate-2"),
    );
    let validated = submit_response(
        &mut core,
        &second_validation_request,
        1,
        HostResponse::ToolCompleted {
            call_id: "validate-2".to_string(),
            output: observed_output(
                json!({"exit_code": 0, "stdout": "all tests passed"}),
                &[],
                &[("file:///tmp/example.rs", "v2")],
            ),
            outcome: nola_core::agent::host::HostToolOutcome::Succeeded,
            activation: None,
            full_output_ref: None,
            artifacts: Vec::new(),
        },
    );
    assert_eq!(
        validated.snapshot.engineering.phase,
        AgentEngineeringPhase::AwaitingAssessment
    );
    assert!(!validated.snapshot.engineering.validation_required);
    assert_eq!(validated.snapshot.engineering.validation_attempt_count, 2);

    let final_model = model_request(&validated.host_requests);
    let final_model=declare_fixture_action_scope(&mut core,&final_model,"file:///tmp/example.rs","v2",true);
    let completion = observed_action_completion(&core, "The fix is complete and tests pass.",
        &["write-1", "write-2", "validate-2"]);
    let draft_completed = submit_response(
        &mut core,
        &final_model,
        1,
        completion,
    );
    assert!(
        draft_completed
            .event_envelopes
            .iter()
            .any(|event| matches!(event.event, AgentEvent::TurnCompleted {
                outcome: Some(nola_domain::TaskCompletionOutcome::ActionCompleted), .. })),
        "an explicit assessed action proposal after validation is terminal"
    );
    assert!(
        !draft_completed
            .host_requests
            .iter()
            .any(|request| matches!(request.payload, HostRequest::ModelStream { .. })),
        "a validated answer must not trigger duplicate synthesis"
    );
}

#[test]
fn external_artifact_contract_blocks_text_until_matching_mutation_and_validation() {
    let mut config = engineering_core_config();
    config
        .permission_rules
        .rules
        .push(nola_core::agent::permissions::PermissionRule {
            capability_pattern: "*".into(),
            resource_pattern: "*".into(),
            decision: nola_core::agent::permissions::PermissionPolicyDecision::Allow,
            source: nola_core::agent::permissions::PermissionRuleSource::UserGlobal,
        });
    let mut core = AgentCore::try_new(config.clone()).expect("create Core");
    let expected = "/workspace/arbitrary-output.bin";
    let started = core
        .submit(AgentSubmission::UserInput {
            items: vec![AgentItem::user_message(
                ItemId::from_string("artifact-user".to_string()),
                "produce the externally declared result",
            )],
            turn_context_items: Vec::new(),
            turn_context_platform: ContextPlatform::Unknown,
            turn_context_components: vec![ContextComponent {
                kind: ContextComponentKind::ProjectInstruction,
                id: TURN_ARTIFACT_CONTRACT_COMPONENT_ID.to_string(),
                content: serde_json::to_value(AgentTurnArtifactContract {
                    schema_version: 1,
                    expected_artifact_refs: vec![expected.to_string()],
                })
                .unwrap(),
                source: None,
            }],
            submission_id: Some("artifact-submission".to_string()),
            submitted_at_epoch_ms: None,
            delivery: AgentInputDelivery::CurrentTurn,
        })
        .expect("start contracted Turn");
    let first_model = model_request(&started.host_requests);
    assert!(matches!(
        first_model.payload,
        HostRequest::ModelStream {
            response_mode: ModelResponseMode::RequiredTool,
            ..
        }
    ));

    let resumable = core.checkpoint(CheckpointReason::Manual);
    let premature = submit_response(
        &mut core,
        &first_model,
        1,
        visible_model_completion("premature-artifact-answer", "done"),
    );
    assert!(premature.event_envelopes.iter().any(|event| matches!(&event.event,
        AgentEvent::RunResultDecided { decision, .. }
            if matches!(decision.value, nola_domain::run_result::RunResultValue::Incomplete { .. }))));
    assert!(premature.host_requests.is_empty());
    core = AgentCore::restore(config, resumable).unwrap();
    let mutation_model = first_model;
    let dispatched = submit_response(
        &mut core,
        &mutation_model,
        1,
        empty_model_completion(vec![named_tool_call(
            "artifact-write",
            "write_file",
            json!({"path": expected, "content": "payload"}),
        )]),
    );
    let write = request_matching(
        &dispatched.host_requests,
        |payload| matches!(payload, HostRequest::ExecuteTool { call_id, .. } if call_id == "artifact-write"),
    );
    let mutated = submit_response(
        &mut core,
        &write,
        1,
        HostResponse::ToolCompleted {
            call_id: "artifact-write".to_string(),
            output: observed_output(
                json!({"path": expected}),
                &[("file:///workspace/arbitrary-output.bin", "v1")],
                &[],
            ),
            outcome: nola_core::agent::host::HostToolOutcome::Succeeded,
            activation: None,
            full_output_ref: None,
            artifacts: Vec::new(),
        },
    );
    assert!(mutated.snapshot.engineering.artifact_completion_pending());

    let validation_model = model_request(&mutated.host_requests);
    let validation_dispatched = submit_response(
        &mut core,
        &validation_model,
        1,
        empty_model_completion(vec![named_tool_call(
            "artifact-validation",
            "bash",
            json!({
                "command": "test -f /workspace/arbitrary-output.bin",
                "operation_kind": "validate",
                "validation_targets": [expected]
            }),
        )]),
    );
    let validation = request_matching(
        &validation_dispatched.host_requests,
        |payload| matches!(payload, HostRequest::ExecuteTool { call_id, .. } if call_id == "artifact-validation"),
    );
    let validated = submit_response(
        &mut core,
        &validation,
        1,
        HostResponse::ToolCompleted {
            call_id: "artifact-validation".to_string(),
            output: observed_output(
                json!({"exit_code": 0}),
                &[],
                &[("file:///workspace/arbitrary-output.bin", "v1")],
            ),
            outcome: nola_core::agent::host::HostToolOutcome::Succeeded,
            activation: None,
            full_output_ref: None,
            artifacts: Vec::new(),
        },
    );
    assert!(!validated.snapshot.engineering.artifact_completion_pending());

    let final_model = model_request(&validated.host_requests);
    let final_model=declare_fixture_action_scope(&mut core,&final_model,"file:///workspace/arbitrary-output.bin","v1",true);
    let completion = observed_action_completion(&core, "completed and validated",
        &["artifact-write", "artifact-validation"]);
    let completed = submit_response(
        &mut core,
        &final_model,
        1,
        completion,
    );
    assert!(completed
        .event_envelopes
        .iter()
        .any(|event| matches!(event.event, AgentEvent::TurnCompleted {
            outcome: Some(nola_domain::TaskCompletionOutcome::ActionCompleted), .. })));
}

#[test]
fn artifact_like_prompt_text_does_not_create_a_completion_contract() {
    let mut core = AgentCore::try_new(engineering_core_config()).expect("create Core");
    let started = core
        .submit(user_input(
            "ordinary-artifact-chat",
            "Discuss nola.turn.artifact-contract.v1 and /workspace/example.out",
            AgentInputDelivery::CurrentTurn,
        ))
        .expect("start ordinary chat");
    let model = model_request(&started.host_requests);
    assert!(matches!(
        model.payload,
        HostRequest::ModelStream {
            response_mode: ModelResponseMode::Conversational,
            ..
        }
    ));
    let completed = submit_response(
        &mut core,
        &model,
        1,
        visible_model_completion("ordinary-answer", "Here is an explanation."),
    );
    assert!(completed
        .event_envelopes
        .iter()
        .any(|event| matches!(event.event, AgentEvent::TurnCompleted { .. })));
}

#[test]
fn engineering_loop_concludes_incomplete_without_forcing_unbounded_validation() {
    let mut core = AgentCore::try_new(engineering_core_config()).expect("create Core");
    let started = core
        .submit(user_input(
            "unvalidated-user",
            "change the code",
            AgentInputDelivery::CurrentTurn,
        ))
        .expect("start turn");
    let model = model_request(&started.host_requests);
    let permission = submit_response(
        &mut core,
        &model,
        1,
        empty_model_completion(vec![named_tool_call(
            "unvalidated-write",
            "write_file",
            json!({"path": "/tmp/example.rs", "content": "hello"}),
        )]),
    );
    let permission_request = request_matching(&permission.host_requests, |payload| {
        matches!(payload, HostRequest::RequestPermission { .. })
    });
    let permission_request_id = match &permission_request.payload {
        HostRequest::RequestPermission {
            permission_request_id,
            ..
        } => permission_request_id.clone(),
        _ => unreachable!(),
    };
    let allowed = core
        .submit(AgentSubmission::PermissionResponse {
            request_id: nola_core::agent::PermissionRequestId::from_string(permission_request_id),
            decision: PermissionUserDecision::AllowOnce,
            grant: None,
        })
        .expect("allow write");
    let write = request_matching(&allowed.host_requests, |payload| {
        matches!(payload, HostRequest::ExecuteTool { .. })
    });
    let after_write = submit_response(
        &mut core,
        &write,
        1,
        HostResponse::ToolCompleted {
            call_id: "unvalidated-write".to_string(),
            output: json!({"ok": true}),
            outcome: nola_core::agent::host::HostToolOutcome::Succeeded,
            activation: None,
            full_output_ref: None,
            artifacts: Vec::new(),
        },
    );
    let first_attempt = model_request(&after_write.host_requests);
    submit_response(
        &mut core,
        &first_attempt,
        1,
        model_delta(&first_attempt, "Done without tests."),
    );
    let corrected = submit_response(
        &mut core,
        &first_attempt,
        2,
        empty_model_completion(Vec::new()),
    );
    assert!(corrected.host_requests.is_empty());
    assert!(corrected.snapshot.engineering.validation_required);
    assert_eq!(corrected.snapshot.engineering.mutation_count, 1);
    assert!(corrected.event_envelopes.iter().any(|event| matches!(&event.event,
        AgentEvent::RunResultDecided { decision, .. }
            if matches!(decision.value, nola_domain::run_result::RunResultValue::Incomplete { .. }))));
    assert!(!corrected
        .event_envelopes
        .iter()
        .any(|event| matches!(event.event, AgentEvent::TurnFailed { .. })));
    assert!(corrected
        .event_envelopes
        .iter()
        .any(|event| matches!(event.event, AgentEvent::TurnCompleted { .. })));
}

#[test]
fn streaming_updates_one_stable_part_without_rewriting_prior_snapshots() {
    let mut core = AgentCore::try_new(core_config(Vec::new())).expect("create Core");
    let started = core
        .submit(user_input(
            "user-1",
            "hello",
            AgentInputDelivery::CurrentTurn,
        ))
        .expect("start turn");
    let request = model_request(&started.host_requests);
    let text_part_id = match &request.payload {
        HostRequest::ModelStream { text_part_id, .. } => text_part_id.clone(),
        _ => unreachable!(),
    };

    let first = submit_response(
        &mut core,
        &request,
        1,
        HostResponse::ModelDelta {
            channel: ModelDeltaChannel::Text,
            part_id: text_part_id.clone(),
            delta: "Hello".to_string(),
        },
    );
    let first_snapshot = first
        .event_envelopes
        .iter()
        .filter_map(|envelope| match &envelope.event {
            AgentEvent::PartUpdated {
                part: AgentPart::Text(part),
                ..
            } if part.id == text_part_id => Some(part.clone()),
            _ => None,
        })
        .last()
        .expect("first text snapshot");

    let second = submit_response(
        &mut core,
        &request,
        2,
        HostResponse::ModelDelta {
            channel: ModelDeltaChannel::Text,
            part_id: text_part_id.clone(),
            delta: " world".to_string(),
        },
    );
    let second_snapshot = second
        .event_envelopes
        .iter()
        .filter_map(|envelope| match &envelope.event {
            AgentEvent::PartUpdated {
                part: AgentPart::Text(part),
                ..
            } if part.id == text_part_id => Some(part.clone()),
            _ => None,
        })
        .last()
        .expect("second text snapshot");

    assert_eq!(first_snapshot.id, second_snapshot.id);
    let owning_turn_id = request.turn_id.as_deref().expect("model request Turn");
    assert_eq!(first_snapshot.ownership.turn_id, owning_turn_id);
    assert_eq!(second_snapshot.ownership.turn_id, owning_turn_id);
    assert_eq!(
        first_snapshot.ownership.message_id,
        format!("{owning_turn_id}:assistant")
    );
    assert_eq!(
        second_snapshot.ownership.message_id,
        first_snapshot.ownership.message_id
    );
    assert_eq!(first_snapshot.text, "Hello");
    assert_eq!(second_snapshot.text, "Hello world");
    assert_eq!(first_snapshot.text, "Hello", "older event snapshot mutated");

    let completed = submit_response(&mut core, &request, 3, empty_model_completion(Vec::new()));
    assert!(completed
        .event_envelopes
        .iter()
        .any(|envelope| matches!(envelope.event, AgentEvent::TurnCompleted { .. })));
}

#[test]
fn large_prompt_control_survives_real_streaming_and_cold_restore_without_body_projection() {
    let config = core_config(Vec::new());
    let mut core = AgentCore::try_new(config.clone()).unwrap();
    let prompt = "source evidence\n".repeat(4096);
    let started = core
        .submit(user_input(
            "control-large-user",
            &prompt,
            AgentInputDelivery::CurrentTurn,
        ))
        .unwrap();
    let request = model_request(&started.host_requests);
    let initial = core
        .committed_host_effect_control(&request.effect_id.0)
        .unwrap()
        .unwrap();
    assert!(serde_json::to_vec(&request).unwrap().len() > prompt.len());
    assert!(serde_json::to_vec(&initial).unwrap().len() < 2048);
    for sequence in 1..=64 {
        let (next, _) = core
            .reduce_owned_at(
                AgentSubmission::HostResponse {
                    response: response_for(&request, sequence, model_delta(&request, "🙂")),
                },
                1000 + sequence,
            )
            .unwrap();
        core = next;
        let control = core
            .committed_host_effect_control(&request.effect_id.0)
            .unwrap()
            .unwrap();
        assert_eq!(control.payload, initial.payload);
        assert_eq!(control.resume_after_sequence, Some(sequence));
        assert!(serde_json::to_vec(&control).unwrap().len() < 2048);
    }
    let checkpoint = core.checkpoint(CheckpointReason::Manual);
    let encoded = serde_json::to_vec(&checkpoint).unwrap();
    let restored = AgentCore::restore(config, serde_json::from_slice(&encoded).unwrap()).unwrap();
    assert_eq!(
        serde_json::to_value(restored.checkpoint(CheckpointReason::Manual)).unwrap(),
        serde_json::to_value(checkpoint).unwrap()
    );
    assert_eq!(
        restored
            .committed_host_effect_control(&request.effect_id.0)
            .unwrap(),
        core.committed_host_effect_control(&request.effect_id.0)
            .unwrap()
    );
}

#[test]
fn committed_semantic_batches_are_monotonic_and_complete_exactly_once() {
    const FIRST: &str = "## Verified result\n\nThe first point is stable.";
    const SECOND: &str = "\n\nThe second point is stable.";
    let expected = format!("{FIRST}{SECOND}");
    let mut core = AgentCore::try_new(core_config(Vec::new())).expect("create Core");
    let started = core
        .submit(user_input(
            "user-committed-stream",
            "give a verified result",
            AgentInputDelivery::CurrentTurn,
        ))
        .expect("start turn");
    let request = model_request(&started.host_requests);
    let text_part_id = match &request.payload {
        HostRequest::ModelStream { text_part_id, .. } => text_part_id.clone(),
        _ => unreachable!(),
    };

    let first = submit_response(
        &mut core,
        &request,
        1,
        HostResponse::ModelCommittedDelta {
            part_id: text_part_id.clone(),
            delta: FIRST.to_string(),
        },
    );
    let first_part = first
        .event_envelopes
        .iter()
        .find_map(|envelope| match &envelope.event {
            AgentEvent::PartUpdated {
                update: nola_core::agent::parts::PartUpdate::TextCommittedDelta { .. },
                part: AgentPart::Text(part),
                ..
            } => Some(part.clone()),
            _ => None,
        })
        .expect("first committed Part snapshot");
    assert_eq!(first_part.text, FIRST);
    assert!(!first_part.is_complete);

    let second = submit_response(
        &mut core,
        &request,
        2,
        HostResponse::ModelCommittedDelta {
            part_id: text_part_id.clone(),
            delta: SECOND.to_string(),
        },
    );
    let second_part = second
        .event_envelopes
        .iter()
        .find_map(|envelope| match &envelope.event {
            AgentEvent::PartUpdated {
                update: nola_core::agent::parts::PartUpdate::TextCommittedDelta { .. },
                part: AgentPart::Text(part),
                ..
            } => Some(part.clone()),
            _ => None,
        })
        .expect("second committed Part snapshot");
    assert!(second_part.text.starts_with(&first_part.text));
    assert_eq!(second_part.text, expected);

    let completed = submit_response(
        &mut core,
        &request,
        3,
        visible_model_completion(&text_part_id, &expected),
    );
    let final_part = completed
        .snapshot
        .projection
        .parts
        .get(&text_part_id)
        .and_then(|part| match part {
            AgentPart::Text(part) => Some(part),
            _ => None,
        })
        .expect("final canonical Text Part");
    assert_eq!(final_part.text, expected);
    assert!(final_part.is_complete);
    assert_eq!(
        completed
            .snapshot
            .projection
            .parts
            .values()
            .filter(|part| matches!(part, AgentPart::Text(text) if text.ownership.message_id == final_part.ownership.message_id))
            .count(),
        1,
        "one model turn must own one canonical assistant Text Part"
    );
}

#[test]
fn committed_text_is_not_retracted_when_the_same_model_step_calls_a_tool() {
    const PREAMBLE: &str = "I will inspect the requested file before answering.";
    let mut core = AgentCore::try_new(core_config(Vec::new())).expect("create Core");
    let started = core
        .submit(user_input(
            "user-committed-before-tool",
            "inspect and explain the file",
            AgentInputDelivery::CurrentTurn,
        ))
        .expect("start turn");
    let request = model_request(&started.host_requests);
    let text_part_id = match &request.payload {
        HostRequest::ModelStream { text_part_id, .. } => text_part_id.clone(),
        _ => unreachable!(),
    };

    submit_response(
        &mut core,
        &request,
        1,
        HostResponse::ModelCommittedDelta {
            part_id: text_part_id.clone(),
            delta: PREAMBLE.to_string(),
        },
    );
    let completed = submit_response(
        &mut core,
        &request,
        2,
        visible_model_completion_with_tools(
            &text_part_id,
            PREAMBLE,
            vec![tool_call("committed-before-tool-call")],
        ),
    );

    let part = completed
        .snapshot
        .projection
        .parts
        .get(&text_part_id)
        .and_then(|part| match part {
            AgentPart::Text(part) => Some(part),
            _ => None,
        })
        .expect("committed preamble remains projected");
    assert_eq!(part.text, PREAMBLE);
    assert!(part.is_complete);
    assert!(!completed.event_envelopes.iter().any(|envelope| {
        matches!(
            &envelope.event,
            AgentEvent::PartUpdated {
                update: nola_core::agent::parts::PartUpdate::TextReplaced { text, .. },
                ..
            } if text.is_empty()
        )
    }));
    assert!(completed.host_requests.iter().any(|request| matches!(
        request.payload,
        HostRequest::ExecuteTool { .. } | HostRequest::RequestPermission { .. }
    )));
}

#[test]
fn restarted_model_stream_discards_provisional_text_and_accepts_replacement_part() {
    let mut core = AgentCore::try_new(core_config(Vec::new())).expect("create Core");
    let started = core
        .submit(user_input(
            "user-restarted-stream",
            "resume this interrupted answer",
            AgentInputDelivery::CurrentTurn,
        ))
        .expect("start turn");
    let request = model_request(&started.host_requests);
    let original_part_id = match &request.payload {
        HostRequest::ModelStream { text_part_id, .. } => text_part_id.clone(),
        _ => unreachable!(),
    };

    let partial = submit_response(
        &mut core,
        &request,
        1,
        HostResponse::ModelCommittedDelta {
            part_id: original_part_id.clone(),
            delta: "obsolete partial answer".to_string(),
        },
    );
    assert_eq!(
        partial
            .snapshot
            .projection
            .parts
            .get(&original_part_id)
            .and_then(|part| match part {
                AgentPart::Text(part) => Some(part.text.as_str()),
                _ => None,
            }),
        Some("obsolete partial answer")
    );

    let restarted = submit_response(
        &mut core,
        &request,
        2,
        HostResponse::ModelAttemptStarted {
            provider: "fixture-provider".into(),
            model: "test-model".into(),
            reason: "resampling".into(),
        },
    );
    let discarded = restarted
        .snapshot
        .projection
        .parts
        .get(&original_part_id)
        .and_then(|part| match part {
            AgentPart::Text(part) => Some(part),
            _ => None,
        })
        .expect("discarded provisional Text Part");
    assert!(discarded.text.is_empty());
    assert!(discarded.is_complete);

    let replacement_part_id = format!("{original_part_id}:replay:3");
    let replacement = submit_response(
        &mut core,
        &request,
        3,
        HostResponse::ModelCommittedDelta {
            part_id: replacement_part_id.clone(),
            delta: "replacement answer".to_string(),
        },
    );
    assert_eq!(
        replacement
            .snapshot
            .projection
            .parts
            .get(&replacement_part_id)
            .and_then(|part| match part {
                AgentPart::Text(part) => Some(part.text.as_str()),
                _ => None,
            }),
        Some("replacement answer")
    );

    let completed = submit_response(
        &mut core,
        &request,
        4,
        visible_model_completion(&replacement_part_id, "replacement answer"),
    );
    assert!(completed
        .event_envelopes
        .iter()
        .any(|envelope| matches!(envelope.event, AgentEvent::TurnCompleted { .. })));
    assert_eq!(
        completed
            .snapshot
            .projection
            .parts
            .get(&replacement_part_id)
            .and_then(|part| match part {
                AgentPart::Text(part) => Some((part.text.as_str(), part.is_complete)),
                _ => None,
            }),
        Some(("replacement answer", true))
    );
}

#[test]
fn restarted_model_stream_clears_reasoning_and_partial_tool_arguments() {
    let mut core = AgentCore::try_new(core_config(Vec::new())).expect("create Core");
    let started = core
        .submit(user_input(
            "user-restarted-tool-stream",
            "inspect a file",
            AgentInputDelivery::CurrentTurn,
        ))
        .expect("start turn");
    let request = model_request(&started.host_requests);
    let reasoning_part_id = match &request.payload {
        HostRequest::ModelStream {
            reasoning_part_id, ..
        } => reasoning_part_id.clone(),
        _ => unreachable!(),
    };
    let old_tool_part_id = "tool-part-before-restart".to_string();

    submit_response(
        &mut core,
        &request,
        1,
        HostResponse::ModelDelta {
            channel: ModelDeltaChannel::Reasoning,
            part_id: reasoning_part_id.clone(),
            delta: "obsolete reasoning".to_string(),
        },
    );
    submit_response(
        &mut core,
        &request,
        2,
        HostResponse::ModelToolCallDelta {
            part_id: old_tool_part_id.clone(),
            call_id: "old-call".to_string(),
            capability_id: "read_file".to_string(),
            arguments_delta: r#"{"path":"old"#.to_string(),
        },
    );
    let restarted = submit_response(
        &mut core,
        &request,
        3,
        HostResponse::ModelAttemptStarted {
            provider: "fixture-provider".into(),
            model: "test-model".into(),
            reason: "resampling".into(),
        },
    );

    let old_reasoning = restarted
        .snapshot
        .projection
        .parts
        .get(&reasoning_part_id)
        .and_then(|part| match part {
            AgentPart::Reasoning(part) => Some(part),
            _ => None,
        })
        .expect("discarded reasoning Part");
    assert!(old_reasoning.title.is_empty());
    assert!(old_reasoning.detail.is_empty());
    assert!(old_reasoning.is_complete);
    let old_tool = restarted
        .snapshot
        .projection
        .parts
        .get(&old_tool_part_id)
        .and_then(|part| match part {
            AgentPart::Tool(part) => Some(part),
            _ => None,
        })
        .expect("discarded partial Tool Part");
    assert!(old_tool.raw_input.is_empty());
    assert_eq!(old_tool.input, Value::Null);
    assert_eq!(old_tool.state, ToolPartState::Cancelled);

    let replacement_part_id = "tool-part-after-restart".to_string();
    let replacement = submit_response(
        &mut core,
        &request,
        4,
        HostResponse::ModelToolCallDelta {
            part_id: replacement_part_id.clone(),
            call_id: "replacement-call".to_string(),
            capability_id: "read_file".to_string(),
            arguments_delta: r#"{"path":"new"}"#.to_string(),
        },
    );
    let replacement_tool = replacement
        .snapshot
        .projection
        .parts
        .get(&replacement_part_id)
        .and_then(|part| match part {
            AgentPart::Tool(part) => Some(part),
            _ => None,
        })
        .expect("replacement Tool Part");
    assert_eq!(replacement_tool.raw_input, r#"{"path":"new"}"#);
    assert!(!replacement_tool.raw_input.contains("old"));
}

#[test]
fn tool_round_preamble_never_enters_the_public_assistant_stream() {
    let mut core = AgentCore::try_new(core_config(Vec::new())).expect("create Core");
    let started = core
        .submit(user_input(
            "user-preamble",
            "write it",
            AgentInputDelivery::CurrentTurn,
        ))
        .expect("start turn");
    let model = model_request(&started.host_requests);
    submit_response(&mut core, &model, 1, model_delta(&model, "I will check."));
    let with_preamble = submit_response(
        &mut core,
        &model,
        2,
        empty_model_completion(vec![tool_call("call-preamble")]),
    );
    assert!(!serde_json::to_string(&with_preamble.trace_events)
        .unwrap()
        .contains("I will check."));

    let mut empty_core = AgentCore::try_new(core_config(Vec::new())).expect("create Core");
    let empty_started = empty_core
        .submit(user_input(
            "user-empty-preamble",
            "write it",
            AgentInputDelivery::CurrentTurn,
        ))
        .expect("start turn");
    let empty_model = model_request(&empty_started.host_requests);
    let without_preamble = submit_response(
        &mut empty_core,
        &empty_model,
        1,
        empty_model_completion(vec![tool_call("call-empty-preamble")]),
    );
    assert!(!without_preamble.trace_events.iter().any(|trace| {
        serde_json::to_value(trace).unwrap()["payload"]
            .get("delta")
            .is_some()
    }));

    let mut goal_config = core_config(Vec::new());
    goal_config.host_catalog.as_mut().unwrap().capabilities.insert("goal_complete".into(),
        capability("goal_complete", HostCapabilityRoute::Tool, None));
    let mut goal_core = AgentCore::try_new(goal_config).unwrap();
    let started = goal_core.submit(user_input("goal-response", "Write a guide", AgentInputDelivery::CurrentTurn)).unwrap();
    let model = model_request(&started.host_requests);
    submit_response(&mut goal_core, &model, 1, model_delta(&model, "Actual response deliverable."));
    let reported = submit_response(&mut goal_core, &model, 2,
        empty_model_completion(vec![named_tool_call("goal-report", "goal_complete", json!({
            "status":"completed", "claims":[{"evidence_refs":["response"]}]
        }))]));
    assert!(reported.snapshot.projection.parts.values().any(|part|
        matches!(part, AgentPart::Text(text) if text.text == "Actual response deliverable.")));
    assert!(!reported.snapshot.projection.parts.values().any(|part|
        matches!(part, AgentPart::Reasoning(reasoning) if reasoning.detail.contains("Actual response deliverable."))));
}

#[test]
fn tool_round_preamble_is_private_but_the_first_post_tool_stop_is_final() {
    let mut core = AgentCore::try_new(core_config(Vec::new())).expect("create Core");
    let started = core
        .submit(user_input(
            "user-post-tool-synthesis",
            "inspect it and report the result",
            AgentInputDelivery::CurrentTurn,
        ))
        .expect("start turn");
    let first_model = model_request(&started.host_requests);
    submit_response(
        &mut core,
        &first_model,
        1,
        model_delta(&first_model, "I will inspect it first."),
    );
    let tool_round = submit_response(
        &mut core,
        &first_model,
        2,
        visible_model_completion_with_tools(
            "tool-round-preamble",
            "I will inspect it first.",
            vec![named_tool_call(
                "post-tool-call",
                "write_file",
                json!({"path":"/tmp/example.txt","content":"hello","ephemeral":true}),
            )],
        ),
    );
    assert!(tool_round
        .snapshot
        .projection
        .parts
        .values()
        .all(|part| { !matches!(part, AgentPart::Text(text) if !text.text.trim().is_empty()) }));

    let permission = request_matching(&tool_round.host_requests, |payload| {
        matches!(payload, HostRequest::RequestPermission { .. })
    });
    let permission_request_id = match &permission.payload {
        HostRequest::RequestPermission {
            permission_request_id,
            ..
        } => permission_request_id.clone(),
        _ => unreachable!(),
    };
    let allowed = core
        .submit(AgentSubmission::PermissionResponse {
            request_id: nola_core::agent::PermissionRequestId::from_string(permission_request_id),
            decision: PermissionUserDecision::AllowOnce,
            grant: None,
        })
        .expect("allow tool");
    let tool = request_matching(
        &allowed.host_requests,
        |payload| matches!(payload, HostRequest::ExecuteTool { call_id, .. } if call_id == "post-tool-call"),
    );
    let after_tool = submit_response(
        &mut core,
        &tool,
        1,
        HostResponse::ToolCompleted {
            call_id: "post-tool-call".to_string(),
            output: json!({"result": "verified evidence"}),
            outcome: nola_core::agent::host::HostToolOutcome::Succeeded,
            activation: None,
            full_output_ref: None,
            artifacts: Vec::new(),
        },
    );
    let ordinary_follow_up = model_request(&after_tool.host_requests);
    submit_response(
        &mut core,
        &ordinary_follow_up,
        1,
        model_delta(&ordinary_follow_up, "I will inspect it first."),
    );
    let completed = submit_response(
        &mut core,
        &ordinary_follow_up,
        2,
        visible_model_completion(
            "post-tool-final-answer",
            "The inspection completed with verified evidence.",
        ),
    );
    assert!(completed
        .event_envelopes
        .iter()
        .any(|event| matches!(event.event, AgentEvent::TurnCompleted { .. })));
    assert!(
        completed
            .host_requests
            .iter()
            .all(|request| !matches!(&request.payload, HostRequest::ModelStream { .. })),
        "a terminal post-tool answer must not trigger a duplicate synthesis request"
    );
}

#[test]
fn tool_backed_answer_completes_once_without_a_duplicate_synthesis_request() {
    let mut core = AgentCore::try_new(core_config(Vec::new())).expect("create Core");
    let started = core
        .submit(user_input(
            "user-tool-backed-synthesis",
            "inspect the evidence and obey every output constraint",
            AgentInputDelivery::CurrentTurn,
        ))
        .expect("start turn");
    let first_model = model_request(&started.host_requests);
    let tool_round = submit_response(
        &mut core,
        &first_model,
        1,
        empty_model_completion(vec![named_tool_call(
            "synthesis-boundary-call",
            "write_file",
            json!({"path":"/tmp/example.txt","content":"hello","ephemeral":true}),
        )]),
    );
    let permission = request_matching(&tool_round.host_requests, |payload| {
        matches!(payload, HostRequest::RequestPermission { .. })
    });
    let permission_request_id = match &permission.payload {
        HostRequest::RequestPermission {
            permission_request_id,
            ..
        } => permission_request_id.clone(),
        _ => unreachable!(),
    };
    let allowed = core
        .submit(AgentSubmission::PermissionResponse {
            request_id: nola_core::agent::PermissionRequestId::from_string(permission_request_id),
            decision: PermissionUserDecision::AllowOnce,
            grant: None,
        })
        .expect("allow tool");
    let tool = request_matching(
        &allowed.host_requests,
        |payload| matches!(payload, HostRequest::ExecuteTool { call_id, .. } if call_id == "synthesis-boundary-call"),
    );
    let after_tool = submit_response(
        &mut core,
        &tool,
        1,
        HostResponse::ToolCompleted {
            call_id: "synthesis-boundary-call".to_string(),
            output: json!({"result": "verified evidence"}),
            outcome: nola_core::agent::host::HostToolOutcome::Succeeded,
            activation: Some(nola_core::agent::host::HostCapabilityActivation {
                source: "tool_search".to_string(),
                capability_ids: vec!["write_file".to_string()],
                capability_descriptors: vec![serde_json::to_value(capability(
                    "write_file", HostCapabilityRoute::Tool, Some("workspace.write"),
                )).unwrap()],
            }),
            full_output_ref: None,
            artifacts: Vec::new(),
        },
    );
    let tool_capable_follow_up = model_request(&after_tool.host_requests);
    submit_response(
        &mut core,
        &tool_capable_follow_up,
        1,
        model_delta(
            &tool_capable_follow_up,
            "A supported answer grounded in the verified tool evidence.",
        ),
    );
    let completed = submit_response(
        &mut core,
        &tool_capable_follow_up,
        2,
        visible_model_completion(
            "tool-backed-answer",
            "A supported answer grounded in the verified tool evidence.",
        ),
    );
    assert!(completed
        .event_envelopes
        .iter()
        .any(|event| matches!(event.event, AgentEvent::TurnCompleted { .. })));
    assert!(
        completed
            .host_requests
            .iter()
            .all(|request| !matches!(&request.payload, HostRequest::ModelStream { .. })),
        "Core requested a second model synthesis after an accepted stop"
    );
    let public_text_parts = completed
        .snapshot
        .projection
        .parts
        .values()
        .filter_map(|part| match part {
            AgentPart::Text(text) if !text.text.trim().is_empty() => Some(text.text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        public_text_parts,
        vec!["A supported answer grounded in the verified tool evidence."]
    );
}

#[test]
fn allow_conversation_resumes_the_same_tool_without_a_second_core_prompt() {
    let mut config = engineering_core_config();
    config
        .host_catalog
        .as_mut()
        .and_then(|catalog| catalog.capabilities.get_mut("bash"))
        .expect("bash capability")
        .permission_domain = Some("workspace.execute".to_string());
    let mut core = AgentCore::try_new(config).expect("create Core");
    let started = core
        .submit(user_input(
            "user-1",
            "write it",
            AgentInputDelivery::CurrentTurn,
        ))
        .expect("start turn");
    let first_model = model_request(&started.host_requests);
    let permission = submit_response(
        &mut core,
        &first_model,
        1,
        empty_model_completion(vec![tool_call("call-1")]),
    );
    let permission_request = request_matching(&permission.host_requests, |payload| {
        matches!(payload, HostRequest::RequestPermission { .. })
    });
    let permission_request_id = match &permission_request.payload {
        HostRequest::RequestPermission {
            permission_request_id,
            ..
        } => permission_request_id.clone(),
        _ => unreachable!(),
    };

    let allowed = core
        .submit(AgentSubmission::PermissionResponse {
            request_id: nola_core::agent::PermissionRequestId::from_string(permission_request_id),
            decision: PermissionUserDecision::AllowConversation,
            grant: Some(json!({"scope": "conversation"})),
        })
        .expect("allow conversation");
    let first_tool = request_matching(&allowed.host_requests, |payload| {
        matches!(
            payload,
            HostRequest::ExecuteTool { call_id, .. } if call_id == "call-1"
        )
    });
    assert_eq!(
        allowed
            .host_requests
            .iter()
            .filter(|request| matches!(request.payload, HostRequest::ExecuteTool { .. }))
            .count(),
        1
    );

    let after_tool = submit_response(
        &mut core,
        &first_tool,
        1,
        HostResponse::ToolCompleted {
            call_id: "call-1".to_string(),
            output: json!({"ok": true}),
            outcome: nola_core::agent::host::HostToolOutcome::Succeeded,
            activation: None,
            full_output_ref: None,
            artifacts: Vec::new(),
        },
    );
    let second_model = model_request(&after_tool.host_requests);
    let reused = submit_response(
        &mut core,
        &second_model,
        1,
        empty_model_completion(vec![tool_call("call-2")]),
    );

    assert!(!reused
        .host_requests
        .iter()
        .any(|request| { matches!(&request.payload, HostRequest::RequestPermission { .. }) }));
    assert!(reused.host_requests.iter().any(|request| {
        matches!(
            &request.payload,
            HostRequest::ExecuteTool { call_id, .. } if call_id == "call-2"
        )
    }));

    let second_tool = request_matching(
        &reused.host_requests,
        |payload| matches!(payload, HostRequest::ExecuteTool { call_id, .. } if call_id == "call-2"),
    );
    let after_second_tool = submit_response(
        &mut core,
        &second_tool,
        1,
        HostResponse::ToolCompleted {
            call_id: "call-2".to_string(),
            output: json!({"ok": true}),
            outcome: nola_core::agent::host::HostToolOutcome::Succeeded,
            activation: None,
            full_output_ref: None,
            artifacts: Vec::new(),
        },
    );
    let third_model = model_request(&after_second_tool.host_requests);
    let different_capability = submit_response(
        &mut core,
        &third_model,
        1,
        empty_model_completion(vec![named_tool_call(
            "call-3",
            "bash",
            json!({"command": "printf different"}),
        )]),
    );
    assert!(different_capability
        .host_requests
        .iter()
        .any(|request| matches!(request.payload, HostRequest::RequestPermission { .. })));
}

#[test]
fn mac_style_allow_conversation_reuses_the_same_bash_family_without_explicit_cwd() {
    let mut config = engineering_core_config();
    config.permission_rules.rules.clear();
    config
        .host_catalog
        .as_mut()
        .and_then(|catalog| catalog.capabilities.get_mut("bash"))
        .expect("bash capability")
        .permission_domain = Some("bash".to_string());
    let mut core = AgentCore::try_new(config).expect("create Core");
    let started = core
        .submit(user_input(
            "mac-user-1",
            "remove both files",
            AgentInputDelivery::CurrentTurn,
        ))
        .expect("start turn");
    let first_model = model_request(&started.host_requests);
    let permission = submit_response(
        &mut core,
        &first_model,
        1,
        empty_model_completion(vec![named_tool_call(
            "mac-rm-1",
            "bash",
            json!({"command": "rm first.txt", "operation_kind": "mutate"}),
        )]),
    );
    let permission_request = request_matching(&permission.host_requests, |payload| {
        matches!(payload, HostRequest::RequestPermission { .. })
    });
    let (permission_request_id, resource) = match &permission_request.payload {
        HostRequest::RequestPermission {
            permission_request_id,
            resource,
            ..
        } => (permission_request_id.clone(), resource.clone()),
        _ => unreachable!(),
    };

    let allowed = core
        .submit(AgentSubmission::PermissionResponse {
            request_id: nola_core::agent::PermissionRequestId::from_string(permission_request_id),
            decision: PermissionUserDecision::AllowConversation,
            grant: Some(serde_json::Value::String(resource)),
        })
        .expect("allow conversation");
    let first_tool = request_matching(
        &allowed.host_requests,
        |payload| matches!(payload, HostRequest::ExecuteTool { call_id, .. } if call_id == "mac-rm-1"),
    );
    let after_tool = submit_response(
        &mut core,
        &first_tool,
        1,
        HostResponse::ToolCompleted {
            call_id: "mac-rm-1".to_string(),
            output: json!({"ok": true}),
            outcome: nola_core::agent::host::HostToolOutcome::Succeeded,
            activation: None,
            full_output_ref: None,
            artifacts: Vec::new(),
        },
    );
    let second_model = model_request(&after_tool.host_requests);
    let reused = submit_response(
        &mut core,
        &second_model,
        1,
        empty_model_completion(vec![named_tool_call(
            "mac-rm-2",
            "bash",
            json!({"command": "rm second.txt", "operation_kind": "mutate"}),
        )]),
    );

    assert!(
        !reused
            .host_requests
            .iter()
            .any(|request| matches!(request.payload, HostRequest::RequestPermission { .. })),
        "unexpected permission request after conversation grant: {:#?}",
        reused.host_requests
    );
    assert!(reused.host_requests.iter().any(|request| {
        matches!(
            &request.payload,
            HostRequest::ExecuteTool { call_id, .. } if call_id == "mac-rm-2"
        )
    }));
}

#[test]
fn active_policy_releases_the_original_call_without_model_resampling_and_survives_restore() {
    use nola_core::agent::{PermissionPolicyDecision, PermissionRule, PermissionRuleSet, PermissionRuleSource};
    let mut config = engineering_core_config();
    config.host_catalog.as_mut().unwrap().capabilities.get_mut("bash").unwrap()
        .permission_domain = Some("bash".into());
    config.permission_rules = PermissionRuleSet { rules: vec![PermissionRule {
        capability_pattern: "bash".into(), resource_pattern: "*".into(),
        decision: PermissionPolicyDecision::Ask, source: PermissionRuleSource::Host,
    }] }.with_profile(nola_domain::PermissionProfile::Cautious);
    let mut core = AgentCore::try_new(config.clone()).unwrap();
    let started = core.submit(user_input("policy-user", "Perform the requested operation", AgentInputDelivery::CurrentTurn)).unwrap();
    let model = model_request(&started.host_requests);
    let asked = submit_response(&mut core, &model, 1,
        empty_model_completion(vec![named_tool_call("original-call","bash",json!({"command":"printf done","operation_kind":"execute"}))]));
    assert!(asked.host_requests.iter().any(|request| matches!(request.payload, HostRequest::RequestPermission { .. })));
    let policy = nola_domain::ActivePermissionPolicy {
        run_id: asked.snapshot.active_turn_id.as_ref().unwrap().as_str().into(),
        configuration_revision: 2, profile: nola_domain::PermissionProfile::AllowAll,
    };
    let applied = core.submit(AgentSubmission::ApplyPermissionPolicy { policy: policy.clone() }).unwrap();
    assert!(applied.snapshot.permissions.pending.is_empty());
    assert!(!applied.host_requests.iter().any(|request| matches!(request.payload,
        HostRequest::ModelStream { .. } | HostRequest::RequestPermission { .. })));
    assert!(applied.host_requests.iter().any(|request| matches!(&request.payload,
        HostRequest::ExecuteTool { call_id, permission_profile: nola_domain::PermissionProfile::AllowAll, .. } if call_id == "original-call")));
    assert!(applied.snapshot.permissions.receipts.iter().any(|receipt|
        matches!(&receipt.outcome,nola_domain::permission_settlement::PermissionReceiptOutcome::PolicyApplied { policy: value } if value == &policy)));
    let restored = AgentCore::restore(config,core.checkpoint(CheckpointReason::Manual)).unwrap();
    assert_eq!(restored.snapshot().active_permission_policy,Some(policy.clone()));
    assert_eq!(restored.snapshot().permission_rules.profile(),nola_domain::PermissionProfile::AllowAll);
    let replay = core.submit(AgentSubmission::ApplyPermissionPolicy { policy }).unwrap();
    assert!(replay.host_requests.is_empty(),"a policy replay must not dispatch the call again");
    let old = core.submit(AgentSubmission::ApplyPermissionPolicy {policy:nola_domain::ActivePermissionPolicy {
        run_id:replay.snapshot.active_turn_id.as_ref().unwrap().as_str().into(),configuration_revision:1,
        profile:nola_domain::PermissionProfile::Cautious,
    }}).unwrap();
    assert!(old.host_requests.is_empty());
    assert_eq!(old.snapshot.active_permission_policy.unwrap().profile,nola_domain::PermissionProfile::AllowAll);
}

#[test]
fn policy_downgrade_rechecks_an_unstarted_effect_but_preserves_started_execution() {
    for execution_started in [false,true] {
        let mut config = engineering_core_config();
        config.permission_rules = nola_core::agent::PermissionRuleSet::default()
            .with_profile(nola_domain::PermissionProfile::AllowAll);
        let mut core = AgentCore::try_new(config).unwrap();
        let started = core.submit(user_input("downgrade-user","Perform the operation",AgentInputDelivery::CurrentTurn)).unwrap();
        let model = model_request(&started.host_requests);
        let planned = submit_response(&mut core,&model,1,empty_model_completion(vec![named_tool_call(
            "undispatched-call","bash",json!({"command":"opaque operation","operation_kind":"execute"}))]));
        let tool_request = request_matching(&planned.host_requests,|request| matches!(request,HostRequest::ExecuteTool {..}));
        if execution_started { submit_response(&mut core,&tool_request,1,HostResponse::ExecutionStarted); }
        let changed = core.submit(AgentSubmission::ApplyPermissionPolicy {policy:nola_domain::ActivePermissionPolicy {
            run_id:planned.snapshot.active_turn_id.unwrap().as_str().into(),configuration_revision:2,
            profile:nola_domain::PermissionProfile::Cautious,
        }}).unwrap();
        assert!(!changed.host_requests.iter().any(|request|matches!(request.payload,HostRequest::ModelStream {..}|HostRequest::ExecuteTool {..})));
        assert_eq!(changed.host_requests.iter().any(|request|matches!(request.payload,HostRequest::RequestPermission {..})),!execution_started);
        assert_eq!(core.checkpoint(CheckpointReason::Manual).state.host.pending_effects.get(&tool_request.effect_id.0).is_some(),execution_started);
    }
}

#[test]
fn process_scope_can_be_authorized_for_conversation_after_checkpoint_restore() {
    use nola_domain::permission_resource_plan::*;
    use nola_domain::permission_scope::PermissionResource;
    for variant in ["same", "command", "cwd", "risk"] {
        let mut config = engineering_core_config();
        config.permission_rules = nola_core::agent::PermissionRuleSet::default().with_profile(nola_domain::PermissionProfile::Cautious);
        config.host_catalog.as_mut().unwrap().capabilities.get_mut("bash").unwrap()
            .input_normalization.permission_contract = PermissionOperationContract {
            resource_plan:Some(PermissionResourcePlan {capability:"bash".into(),operation:"process".into(),requirements:vec![
                ResourceRequirement {requirement_id:"command".into(),locator_kind:ResourceLocatorKind::ProcessCommand,
                    access:"execute".into(),selector:ResourceSelector::One {pointer:"/command".into()}}]}),..Default::default()};
        let mut core = AgentCore::try_new(config.clone()).unwrap();
        let started = core.submit(user_input("opaque-user","Perform this operation",AgentInputDelivery::CurrentTurn)).unwrap();
        let model = model_request(&started.host_requests);
        let resolving = submit_response(&mut core,&model,1,empty_model_completion(vec![named_tool_call("opaque-call","bash",
            json!({"command":"sender --body @payload $DESTINATION","operation_kind":"execute"}))]));
        let resolution = request_matching(&resolving.host_requests,|request|matches!(request,HostRequest::ResolvePermissionResources {..}));
        let HostRequest::ResolvePermissionResources {call_id,request,..} = &resolution.payload else {unreachable!()};
        let facts = PermissionResolutionFacts {resolution_id:request.resolution_id.clone(),execution_target:request.execution_target.clone(),
            invocation_digest:request.invocation_digest,bindings:vec![ResolvedDemandBinding {demand:request.demands[0].clone(),
                resource:PermissionResource::ProcessInvocation {invocation_digest:request.invocation_digest,
                    command:"sender --body @payload $DESTINATION".into(),cwd:"/workspace".into()},
                observation:PermissionOperationRisk::Unknown,evidence_id:"host-command-only".into(),browser:None,derived:None}]};
        let waiting = submit_response(&mut core,&resolution,1,HostResponse::PermissionResourcesResolved {call_id:call_id.clone(),facts});
        let pending = &waiting.snapshot.permissions.pending[0];
        assert!(pending.allow_conversation);
        assert!(waiting.host_requests.iter().any(|request|matches!(&request.payload,
            HostRequest::RequestPermission {allow_conversation:true,..})));
        let request_id = pending.request_id.clone();
        core = AgentCore::restore(config,core.checkpoint(CheckpointReason::Manual)).unwrap();
        let allowed = core.submit(AgentSubmission::PermissionResponse {request_id,
            decision:PermissionUserDecision::AllowConversation,grant:None}).unwrap();
        assert_eq!(allowed.host_requests.iter().filter(|request|matches!(&request.payload,
            HostRequest::ExecuteTool {call_id,..} if call_id == "opaque-call")).count(),1);
        assert!(core.checkpoint(CheckpointReason::Manual).state.execution.permission_rules.rules.iter()
            .any(|rule|rule.source == nola_core::agent::permissions::PermissionRuleSource::Conversation));
        let execution = request_matching(&allowed.host_requests,|request|matches!(request,HostRequest::ExecuteTool {..}));
        let completed = submit_response(&mut core,&execution,1,HostResponse::ToolCompleted {
            call_id:"opaque-call".into(), output:json!({"ok":true}), outcome:nola_core::agent::host::HostToolOutcome::Succeeded,
            activation:None, full_output_ref:None, artifacts:vec![],
        });
        let model = model_request(&completed.host_requests);
        let second_command = if variant == "command" { "another-command" } else { "sender --body @payload $DESTINATION" };
        let resolving = submit_response(&mut core,&model,1,empty_model_completion(vec![named_tool_call("second-call","bash",
            json!({"command":second_command,"operation_kind":"execute"}))]));
        let resolution = request_matching(&resolving.host_requests,|request|matches!(request,HostRequest::ResolvePermissionResources {..}));
        let HostRequest::ResolvePermissionResources {call_id,request,..} = &resolution.payload else {unreachable!()};
        let facts = PermissionResolutionFacts {resolution_id:request.resolution_id.clone(),execution_target:request.execution_target.clone(),
            invocation_digest:request.invocation_digest,bindings:vec![ResolvedDemandBinding {demand:request.demands[0].clone(),
                resource:PermissionResource::ProcessInvocation {invocation_digest:request.invocation_digest,
                    command:second_command.into(),cwd:if variant == "cwd" { "/other".into() } else { "/workspace".into() }},
                observation:if variant == "risk" { PermissionOperationRisk::SensitiveRead } else { PermissionOperationRisk::Unknown },evidence_id:"fresh-host-observation".into(),browser:None,derived:None}]};
        let reused = submit_response(&mut core,&resolution,1,HostResponse::PermissionResourcesResolved {call_id:call_id.clone(),facts});
        assert_eq!(reused.snapshot.permissions.pending.is_empty(), variant == "same", "{variant}");
        assert_eq!(reused.host_requests.iter().any(|request|matches!(request.payload,HostRequest::RequestPermission {..})), variant != "same", "{variant}");
        assert_eq!(reused.host_requests.iter().filter(|request|matches!(&request.payload,
            HostRequest::ExecuteTool {call_id,..} if call_id == "second-call")).count(),usize::from(variant == "same"), "{variant}");
    }
}

#[test]
fn host_resource_resolution_precedes_permission_and_reopens_without_losing_request_identity() {
    use nola_domain::permission_resource_plan::*;
    use nola_domain::permission_scope::PermissionResource;
    let mut config = engineering_core_config();
    config.permission_rules = nola_core::agent::PermissionRuleSet::default().with_profile(nola_domain::PermissionProfile::Cautious);
    let descriptor = config.host_catalog.as_mut().unwrap().capabilities.get_mut("write_file").unwrap();
    descriptor.input_normalization.permission_contract = PermissionOperationContract {resource_plan:Some(PermissionResourcePlan {
        capability:"write_file".into(),operation:"filesystem".into(),requirements:vec![ResourceRequirement {
            requirement_id:"target".into(),locator_kind:ResourceLocatorKind::FilePath,access:"write".into(),
            selector:ResourceSelector::One {pointer:"/path".into()} }],
    }),..Default::default()};
    let mut core = AgentCore::try_new(config.clone()).unwrap();
    let started = core.submit(user_input("fact-user","Create a file",AgentInputDelivery::CurrentTurn)).unwrap();
    let model = model_request(&started.host_requests);
    let resolving = submit_response(&mut core,&model,1,empty_model_completion(vec![named_tool_call("create-call","write_file",
        json!({"path":"notes.txt","content":"hello"}))]));
    let request = request_matching(&resolving.host_requests,|request|matches!(request,HostRequest::ResolvePermissionResources {..}));
    assert!(!resolving.host_requests.iter().any(|request|matches!(request.payload,HostRequest::RequestPermission {..}|HostRequest::ExecuteTool {..})));
    core = AgentCore::restore(config,core.checkpoint(CheckpointReason::Manual)).unwrap();
    let HostRequest::ResolvePermissionResources {call_id,request:resolution,..} = &request.payload else {unreachable!()};
    let facts = PermissionResolutionFacts {resolution_id:resolution.resolution_id.clone(),execution_target:resolution.execution_target.clone(),
        invocation_digest:resolution.invocation_digest,bindings:vec![ResolvedDemandBinding {browser:None,derived:None,
            demand:resolution.demands[0].clone(),resource:PermissionResource::File {filesystem_id:"volume".into(),
                components:vec!["notes.txt".into()],access:"write".into()},evidence_id:"new-file-state".into(),observation:PermissionOperationRisk::NewFile,
        }]};
    let admitted = submit_response(&mut core,&request,1,HostResponse::PermissionResourcesResolved {call_id:call_id.clone(),facts});
    assert!(admitted.host_requests.iter().any(|request|matches!(request.payload,HostRequest::ExecuteTool {..})));
    assert!(!admitted.host_requests.iter().any(|request|matches!(request.payload,HostRequest::RequestPermission {..}|HostRequest::ModelStream {..})));
}

#[test]
fn conversation_grant_cannot_expand_to_a_different_pending_resource() {
    exercise_pending_conversation_grant(false);
}

#[test]
fn conversation_grant_reuse_retains_original_decision_source() {
    exercise_pending_conversation_grant(true);
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

#[test]
fn automated_batch_permission_recheck_preserves_origin_and_selected_call_authorization() {
    use nola_core::agent::permissions::{
        PermissionPolicyDecision, PermissionRule, PermissionRuleSource,
    };
    use nola_core::agent::AgentInputOrigin;

    for origin in [AgentInputOrigin::ScheduledAction, AgentInputOrigin::Replay] {
        for source in [
            PermissionRuleSource::Conversation,
            PermissionRuleSource::Turn,
        ] {
            let mut config = engineering_core_config();
            config
                .host_catalog
                .as_mut()
                .unwrap()
                .capabilities
                .get_mut("write_file")
                .unwrap()
                .parallelism = HostCapabilityParallelism::ConcurrentSafe;
            config.permission_rules.rules.push(PermissionRule {
                capability_pattern: "workspace.write".into(),
                resource_pattern: "/tmp/sibling.txt".into(),
                decision: PermissionPolicyDecision::Allow,
                source,
            });
            let mut core = AgentCore::try_new(config).unwrap();
            let started = core
                .submit(AgentSubmission::AutomatedInput {
                    origin,
                    items: vec![AgentItem::user_message(
                        ItemId::from_string("automated-user".into()),
                        "write both files",
                    )],
                    turn_context_items: vec![],
                    turn_context_platform: Default::default(),
                    turn_context_components: vec![],
                    submission_id: Some("automated-batch".into()),
                    submitted_at_epoch_ms: None,
                })
                .unwrap();
            let model = model_request(&started.host_requests);
            let requested = submit_response(
                &mut core,
                &model,
                1,
                empty_model_completion(vec![
                    named_tool_call(
                        "selected",
                        "write_file",
                        json!({"path":"/tmp/selected.txt", "content":"a"}),
                    ),
                    named_tool_call(
                        "sibling",
                        "write_file",
                        json!({"path":"/tmp/sibling.txt", "content":"b"}),
                    ),
                ]),
            );
            assert_eq!(
                requested.snapshot.permissions.pending.len(),
                2,
                "{origin:?}/{source:?}"
            );
            let selected = requested
                .snapshot
                .permissions
                .pending
                .iter()
                .find(|p| p.call_id.as_str() == "selected")
                .unwrap()
                .request_id
                .clone();
            let sibling = requested
                .snapshot
                .permissions
                .pending
                .iter()
                .find(|p| p.call_id.as_str() == "sibling")
                .unwrap()
                .clone();
            // Conversation case: even a new broad grant cannot authorize its
            // automated siblings. Turn case: a narrow new grant must not make
            // the previously excluded Turn rule usable during re-evaluation.
            let resource_pattern = if source == PermissionRuleSource::Conversation {
                "*"
            } else {
                "/tmp/selected.txt"
            };
            let allowed = core
                .submit(AgentSubmission::PermissionResponse {
                    request_id: selected,
                    decision: PermissionUserDecision::AllowConversation,
                    grant: Some(
                        json!({"scope":"conversation", "resource_pattern":resource_pattern}),
                    ),
                })
                .unwrap();
            let executed = allowed
                .host_requests
                .iter()
                .filter_map(|r| match &r.payload {
                    HostRequest::ExecuteTool { call_id, .. } => Some(call_id.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(executed, ["selected"], "{origin:?}/{source:?}");
            assert_eq!(allowed.snapshot.permissions.pending, vec![sibling.clone()]);
            assert_eq!(
                checkpoint_after(&core, &allowed)
                    .state
                    .execution
                    .active_turn
                    .as_ref()
                    .unwrap()
                    .context
                    .input_origin,
                origin
            );
            let directly_allowed = core
                .submit(AgentSubmission::PermissionResponse {
                    request_id: sibling.request_id,
                    decision: PermissionUserDecision::AllowOnce,
                    grant: None,
                })
                .unwrap();
            assert!(directly_allowed.snapshot.permissions.pending.is_empty());
            let executed = directly_allowed
                .host_requests
                .iter()
                .filter_map(|r| match &r.payload {
                    HostRequest::ExecuteTool { call_id, .. } => Some(call_id.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(
                executed,
                ["sibling"],
                "explicitly approving a call remains valid"
            );
        }
    }
}

#[test]
fn denied_permission_ends_tool_execution_for_the_turn() {
    let mut core = AgentCore::try_new(engineering_core_config()).expect("create Core");
    let started = core
        .submit(user_input(
            "deny-user",
            "delete the protected file",
            AgentInputDelivery::CurrentTurn,
        ))
        .expect("start turn");
    let first_model = model_request(&started.host_requests);
    let permission = submit_response(
        &mut core,
        &first_model,
        1,
        empty_model_completion(vec![tool_call("denied-write")]),
    );
    let permission_request = request_matching(&permission.host_requests, |payload| {
        matches!(payload, HostRequest::RequestPermission { .. })
    });
    let permission_request_id = match &permission_request.payload {
        HostRequest::RequestPermission {
            permission_request_id,
            ..
        } => permission_request_id.clone(),
        _ => unreachable!(),
    };

    let denied = core
        .submit(AgentSubmission::PermissionResponse {
            request_id: nola_core::agent::PermissionRequestId::from_string(permission_request_id),
            decision: PermissionUserDecision::Deny,
            grant: None,
        })
        .expect("deny protected action");
    let checkpoint = serde_json::to_value(core.checkpoint(CheckpointReason::Manual).state).unwrap();
    assert_eq!(
        checkpoint["execution"]["permissions"]["receipts"][0]["outcome"]["decision"],
        "deny"
    );
    let follow_up = model_request(&denied.host_requests);
    let (messages, tools) = match &follow_up.payload {
        HostRequest::ModelStream {
            messages, tools, ..
        } => (messages, tools),
        _ => unreachable!(),
    };
    assert_eq!(model_tool_names(tools), vec!["requirements_update"],
        "denied Turn may account for the denied requirement but cannot expose business tools");
    assert!(messages.iter().any(|message| {
        message
            .get("content")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|content| {
                content.contains("user denied the requested permission")
                    && content.contains("Do not call any tool")
            })
    }));

    let bypass_attempt = submit_response(
        &mut core,
        &follow_up,
        1,
        empty_model_completion(vec![named_tool_call(
            "bypass-shell",
            "bash",
            json!({"command": "rm /tmp/example.txt", "operation_kind": "mutate"}),
        )]),
    );
    assert!(bypass_attempt.host_requests.iter().all(|request| {
        !matches!(
            request.payload,
            HostRequest::ExecuteTool { .. } | HostRequest::RequestPermission { .. }
        )
    }));
    let retry = model_request(&bypass_attempt.host_requests);
    match retry.payload {
        HostRequest::ModelStream { tools, .. } => assert_eq!(model_tool_names(&tools), vec!["requirements_update"]),
        _ => unreachable!(),
    }
}

#[test]
fn denied_permission_final_answer_supersedes_an_earlier_validation_gate() {
    let mut core = AgentCore::try_new(engineering_core_config()).expect("create Core");
    let started = core
        .submit(user_input(
            "deny-after-mutation-user",
            "update the source, then attempt a second protected write",
            AgentInputDelivery::CurrentTurn,
        ))
        .expect("start turn");
    let first_model = model_request(&started.host_requests);
    let write_permission = submit_response(
        &mut core,
        &first_model,
        1,
        empty_model_completion(vec![named_tool_call(
            "write-before-denial",
            "write_file",
            json!({"path": "/tmp/example.rs", "content": "fn main() {}"}),
        )]),
    );
    let write_permission_request = request_matching(&write_permission.host_requests, |payload| {
        matches!(payload, HostRequest::RequestPermission { .. })
    });
    let write_permission_request_id = match &write_permission_request.payload {
        HostRequest::RequestPermission {
            permission_request_id,
            ..
        } => permission_request_id.clone(),
        _ => unreachable!(),
    };
    let write_allowed = core
        .submit(AgentSubmission::PermissionResponse {
            request_id: nola_core::agent::PermissionRequestId::from_string(
                write_permission_request_id,
            ),
            decision: PermissionUserDecision::AllowOnce,
            grant: None,
        })
        .expect("allow source write");
    let write_tool = request_matching(
        &write_allowed.host_requests,
        |payload| matches!(payload, HostRequest::ExecuteTool { call_id, .. } if call_id == "write-before-denial"),
    );
    let after_write = submit_response(
        &mut core,
        &write_tool,
        1,
        HostResponse::ToolCompleted {
            call_id: "write-before-denial".to_string(),
            output: json!({"ok": true}),
            outcome: nola_core::agent::host::HostToolOutcome::Succeeded,
            activation: None,
            full_output_ref: None,
            artifacts: Vec::new(),
        },
    );
    assert!(after_write.snapshot.engineering.validation_required);

    let next_model = model_request(&after_write.host_requests);
    let denied_write_permission = submit_response(
        &mut core,
        &next_model,
        1,
        empty_model_completion(vec![named_tool_call(
            "denied-write-after-write",
            "write_file",
            json!({"path": "/tmp/protected.rs", "content": "protected"}),
        )]),
    );
    let denied_write_permission_request =
        request_matching(&denied_write_permission.host_requests, |payload| {
            matches!(payload, HostRequest::RequestPermission { .. })
        });
    let denied_write_permission_request_id = match &denied_write_permission_request.payload {
        HostRequest::RequestPermission {
            permission_request_id,
            ..
        } => permission_request_id.clone(),
        _ => unreachable!(),
    };
    let denied = core
        .submit(AgentSubmission::PermissionResponse {
            request_id: nola_core::agent::PermissionRequestId::from_string(
                denied_write_permission_request_id,
            ),
            decision: PermissionUserDecision::Deny,
            grant: None,
        })
        .expect("deny delete");
    let final_model = model_request(&denied.host_requests);
    match &final_model.payload {
        HostRequest::ModelStream { tools, .. } => assert_eq!(model_tool_names(tools), vec!["requirements_update"]),
        _ => unreachable!(),
    }
    submit_response(
        &mut core,
        &final_model,
        1,
        model_delta(
            &final_model,
            "The protected write was not performed because permission was denied.",
        ),
    );
    let completed = submit_response(
        &mut core,
        &final_model,
        2,
        empty_model_completion(Vec::new()),
    );

    assert!(completed
        .event_envelopes
        .iter()
        .any(|envelope| matches!(envelope.event, AgentEvent::TurnCompleted { .. })));
    assert!(!completed
        .host_requests
        .iter()
        .any(|request| matches!(request.payload, HostRequest::ModelStream { .. })));
}

#[test]
fn provider_may_reuse_call_id_across_sequential_tool_cycles() {
    let mut core = AgentCore::try_new(core_config(Vec::new())).expect("create Core");
    let started = core
        .submit(user_input(
            "user-reused-call-id",
            "write two files",
            AgentInputDelivery::CurrentTurn,
        ))
        .expect("start turn");
    let first_model = model_request(&started.host_requests);
    submit_response(
        &mut core,
        &first_model,
        1,
        HostResponse::ModelToolCallDelta {
            part_id: "tool:model-cycle-1:call_text_0".to_string(),
            call_id: "call_text_0".to_string(),
            capability_id: "write_file".to_string(),
            arguments_delta: "{\"path\":\"/tmp/first.txt\"}".to_string(),
        },
    );
    let first_permission = submit_response(
        &mut core,
        &first_model,
        2,
        empty_model_completion(vec![tool_call("call_text_0")]),
    );
    let first_permission_request = request_matching(&first_permission.host_requests, |payload| {
        matches!(payload, HostRequest::RequestPermission { .. })
    });
    let first_permission_id = match &first_permission_request.payload {
        HostRequest::RequestPermission {
            permission_request_id,
            ..
        } => permission_request_id.clone(),
        _ => unreachable!(),
    };
    let first_allowed = core
        .submit(AgentSubmission::PermissionResponse {
            request_id: nola_core::agent::PermissionRequestId::from_string(first_permission_id),
            decision: PermissionUserDecision::AllowOnce,
            grant: None,
        })
        .expect("allow first tool");
    let first_tool = request_matching(&first_allowed.host_requests, |payload| {
        matches!(payload, HostRequest::ExecuteTool { .. })
    });
    let after_first_tool = submit_response(
        &mut core,
        &first_tool,
        1,
        HostResponse::ToolCompleted {
            call_id: "call_text_0".to_string(),
            output: json!({"ok": true, "path": "/tmp/first.txt"}),
            outcome: nola_core::agent::host::HostToolOutcome::Succeeded,
            activation: None,
            full_output_ref: None,
            artifacts: Vec::new(),
        },
    );

    let second_model = model_request(&after_first_tool.host_requests);
    let second_stream = submit_response(
        &mut core,
        &second_model,
        1,
        HostResponse::ModelToolCallDelta {
            part_id: "tool:model-cycle-2:call_text_0".to_string(),
            call_id: "call_text_0".to_string(),
            capability_id: "write_file".to_string(),
            arguments_delta: "{\"path\":\"/tmp/second.txt\"}".to_string(),
        },
    );
    assert!(second_stream.event_envelopes.iter().any(|envelope| {
        matches!(
            &envelope.event,
            AgentEvent::PartUpdated {
                part: AgentPart::Tool(tool),
                ..
            } if tool.id == "tool:model-cycle-2:call_text_0"
        )
    }));
    let second_permission = submit_response(
        &mut core,
        &second_model,
        2,
        empty_model_completion(vec![tool_call("call_text_0")]),
    );

    assert!(second_permission
        .host_requests
        .iter()
        .any(|request| { matches!(request.payload, HostRequest::RequestPermission { .. }) }));
    assert!(second_permission.event_envelopes.iter().any(|envelope| {
        matches!(
            &envelope.event,
            AgentEvent::PartUpdated {
                part: AgentPart::Tool(tool),
                ..
            } if tool.call_id == "call_text_0" && !tool.state.is_terminal()
        )
    }));
}

#[test]
fn empty_completion_after_mutation_keeps_validation_tools_available() {
    let mut core = AgentCore::try_new(engineering_core_config()).expect("create Core");
    let started = core
        .submit(user_input(
            "user-final",
            "write it and confirm",
            AgentInputDelivery::CurrentTurn,
        ))
        .expect("start turn");
    let first_model = model_request(&started.host_requests);
    let permission = submit_response(
        &mut core,
        &first_model,
        1,
        empty_model_completion(vec![named_tool_call(
            "call-final",
            "write_file",
            json!({"path": "/tmp/example.rs", "content": "hello"}),
        )]),
    );
    assert!(!permission.trace_events.iter().any(|trace| {
        serde_json::to_value(trace).unwrap()["payload"]
            .get("delta")
            .is_some()
    }));
    let permission_request = request_matching(&permission.host_requests, |payload| {
        matches!(payload, HostRequest::RequestPermission { .. })
    });
    let permission_request_id = match &permission_request.payload {
        HostRequest::RequestPermission {
            permission_request_id,
            ..
        } => permission_request_id.clone(),
        _ => unreachable!(),
    };
    let allowed = core
        .submit(AgentSubmission::PermissionResponse {
            request_id: nola_core::agent::PermissionRequestId::from_string(permission_request_id),
            decision: PermissionUserDecision::AllowOnce,
            grant: None,
        })
        .expect("allow tool");
    let tool = request_matching(&allowed.host_requests, |payload| {
        matches!(payload, HostRequest::ExecuteTool { .. })
    });
    let after_tool = submit_response(
        &mut core,
        &tool,
        1,
        HostResponse::ToolCompleted {
            call_id: "call-final".to_string(),
            output: observed_output(
                json!({"ok": true}),
                &[("file:///tmp/example.rs", "v1")],
                &[],
            ),
            outcome: nola_core::agent::host::HostToolOutcome::Succeeded,
            activation: None,
            full_output_ref: None,
            artifacts: Vec::new(),
        },
    );
    let ordinary_follow_up = model_request(&after_tool.host_requests);
    let resumable = core.checkpoint(CheckpointReason::Manual);
    let forced = submit_response(
        &mut core,
        &ordinary_follow_up,
        1,
        empty_model_completion(Vec::new()),
    );

    assert!(forced.event_envelopes.iter().any(|event| matches!(&event.event,
        AgentEvent::RunResultDecided { decision, .. }
            if matches!(decision.value, nola_domain::run_result::RunResultValue::Incomplete { .. }))));
    assert!(forced.host_requests.is_empty());
    core = AgentCore::restore(engineering_core_config(), resumable).unwrap();
    let forced_request = ordinary_follow_up;
    match &forced_request.payload {
        HostRequest::ModelStream {
            messages, tools, ..
        } => {
            assert!(!tools.is_empty(), "validation must keep tools available");
            assert!(messages.iter().any(|message| {
                message.get("role").and_then(serde_json::Value::as_str) == Some("system")
                    && message
                        .get("content")
                        .and_then(serde_json::Value::as_str)
                        .is_some_and(|content| content.contains("no successful validation"))
            }));
        }
        _ => unreachable!(),
    }

    let validation_dispatch = submit_response(
        &mut core,
        &forced_request,
        1,
        empty_model_completion(vec![named_tool_call(
            "validate-empty-recovery",
            "bash",
            json!({"command": "cargo test --quiet", "operation_kind": "validate", "validation_targets": ["/tmp/example.rs"]}),
        )]),
    );
    let validation_request = request_matching(
        &validation_dispatch.host_requests,
        |payload| matches!(payload, HostRequest::ExecuteTool { call_id, .. } if call_id == "validate-empty-recovery"),
    );
    let validated = submit_response(
        &mut core,
        &validation_request,
        1,
        HostResponse::ToolCompleted {
            call_id: "validate-empty-recovery".to_string(),
            output: observed_output(
                json!({"exit_code": 0}),
                &[],
                &[("file:///tmp/example.rs", "v1")],
            ),
            outcome: nola_core::agent::host::HostToolOutcome::Succeeded,
            activation: None,
            full_output_ref: None,
            artifacts: Vec::new(),
        },
    );
    let final_request = model_request(&validated.host_requests);
    let final_request=declare_fixture_action_scope(&mut core,&final_request,"file:///tmp/example.rs","v1",true);
    let completion = observed_action_completion(&core, "The file is ready and validation passed.",
        &["call-final", "validate-empty-recovery"]);
    let draft_completed = submit_response(
        &mut core,
        &final_request,
        1,
        completion,
    );
    assert!(
        draft_completed
            .event_envelopes
            .iter()
            .any(|envelope| matches!(envelope.event, AgentEvent::TurnCompleted {
                outcome: Some(nola_domain::TaskCompletionOutcome::ActionCompleted), .. })),
        "an explicit assessed action proposal after validation is terminal"
    );
    assert!(
        !draft_completed
            .host_requests
            .iter()
            .any(|request| matches!(request.payload, HostRequest::ModelStream { .. })),
        "a validated answer must not trigger duplicate synthesis"
    );
}

#[test]
fn invalid_tool_input_fails_before_permission_and_returns_to_the_model() {
    let mut core = AgentCore::try_new(core_config(Vec::new())).expect("create Core");
    let started = core
        .submit(user_input(
            "user-invalid",
            "write it",
            AgentInputDelivery::CurrentTurn,
        ))
        .expect("start turn");
    let first_model = model_request(&started.host_requests);
    let invalid_call = ModelToolCall {
        call_id: nola_core::agent::CallId::from_string("call-invalid".to_string()),
        name: "write_file".to_string(),
        arguments: json!({}),
        permission_scope: None,
        permission_rationale: None,
    };
    let transition = submit_response(
        &mut core,
        &first_model,
        1,
        empty_model_completion(vec![invalid_call]),
    );

    assert!(!transition.host_requests.iter().any(|request| matches!(
        request.payload,
        HostRequest::RequestPermission { .. } | HostRequest::ExecuteTool { .. }
    )));
    assert!(transition.trace_events.iter().any(|trace| {
        trace.kind == nola_core::agent_trace::AgentTraceKind::ToolCompleted
            && trace.status == nola_core::agent_trace::AgentTraceStatus::Failed
            && trace.payload.call_id.as_deref() == Some("call-invalid")
            && trace
                .payload
                .result
                .as_ref()
                .and_then(|value| value.get("output_preview"))
                .and_then(serde_json::Value::as_str)
                .is_some_and(|value| value.contains("invalid_tool_arguments"))
    }));
    assert!(transition
        .host_requests
        .iter()
        .any(|request| matches!(request.payload, HostRequest::ModelStream { .. })));
    let retry = model_request(&transition.host_requests);
    let HostRequest::ModelStream { tools, .. } = &retry.payload else {
        panic!("invalid arguments must return to a model request")
    };
    assert!(
        tools.iter().any(|schema| {
            schema
                .pointer("/function/name")
                .and_then(serde_json::Value::as_str)
                == Some("write_file")
        }),
        "the rejected capability must remain exposed for one schema-correcting retry: {tools:?}"
    );
}

#[test]
fn tool_response_call_identity_is_checked_before_consuming_the_effect() {
    let mut core = AgentCore::try_new(source_core_config()).expect("create Core");
    let started = core
        .submit(user_input(
            "user-tool-identity",
            "inspect the file",
            AgentInputDelivery::CurrentTurn,
        ))
        .expect("start turn");
    let model = model_request(&started.host_requests);
    let dispatched = submit_response(
        &mut core,
        &model,
        1,
        empty_model_completion(vec![named_tool_call(
            "call-read-stable",
            "read_file",
            json!({"path": "notes.txt"}),
        )]),
    );
    let tool = request_matching(
        &dispatched.host_requests,
        |payload| matches!(payload, HostRequest::ExecuteTool { call_id, .. } if call_id == "call-read-stable"),
    );

    let mismatched_progress = core
        .submit(AgentSubmission::HostResponse {
            response: response_for(
                &tool,
                1,
                HostResponse::ToolCallProgress {
                    call_id: "call-from-another-effect".to_string(),
                    channel: ToolProgressChannel::Status,
                    title: Some("wrong call".to_string()),
                    detail: None,
                    delta: None,
                    metadata: json!({}),
                },
            ),
        })
        .expect_err("tool progress cannot claim another call identity");
    assert!(mismatched_progress.contains("code=tool_call_identity_mismatch"));

    let mismatched = core
        .submit(AgentSubmission::HostResponse {
            response: response_for(
                &tool,
                1,
                HostResponse::ToolCompleted {
                    call_id: "call-from-another-effect".to_string(),
                    output: json!({"text": "wrong result"}),
                    outcome: nola_core::agent::host::HostToolOutcome::Succeeded,
                    activation: None,
                    full_output_ref: None,
                    artifacts: Vec::new(),
                },
            ),
        })
        .expect_err("a tool response cannot claim another call identity");
    assert!(mismatched.contains("code=tool_call_identity_mismatch"));
    assert!(mismatched.contains("expected_call_id=call-read-stable"));
    assert!(mismatched.contains("received_call_id=call-from-another-effect"));

    let completed = submit_response(
        &mut core,
        &tool,
        1,
        HostResponse::ToolCompleted {
            call_id: "call-read-stable".to_string(),
            output: json!({"text": "canonical result"}),
            outcome: nola_core::agent::host::HostToolOutcome::Succeeded,
            activation: None,
            full_output_ref: None,
            artifacts: Vec::new(),
        },
    );
    assert!(completed.snapshot.projection.parts.values().any(|part| {
        matches!(
            part,
            AgentPart::Tool(tool)
                if tool.call_id == "call-read-stable"
                    && tool.state == ToolPartState::Completed
        )
    }));
    assert!(!completed.snapshot.projection.parts.values().any(|part| {
        matches!(part, AgentPart::Tool(tool) if tool.call_id == "call-from-another-effect")
    }));
}

#[test]
fn unknown_model_tool_preserves_identity_and_returns_typed_recovery_to_the_model() {
    let mut core = AgentCore::try_new(core_config(Vec::new())).expect("create Core");
    let started = core
        .submit(user_input(
            "user-unknown",
            "继续处理；search 和 current 只是普通文字",
            AgentInputDelivery::CurrentTurn,
        ))
        .expect("start turn");
    let first_model = model_request(&started.host_requests);
    let arguments = json!({"query": "继续处理；search 和 current 只是普通文字"});
    let transition = submit_response(
        &mut core,
        &first_model,
        1,
        empty_model_completion(vec![ModelToolCall {
            call_id: CallId::from_string("call-unknown-stable".to_string()),
            name: "remembered_or_hallucinated_tool".to_string(),
            arguments: arguments.clone(),
            permission_scope: None,
            permission_rationale: None,
        }]),
    );

    assert!(!transition.host_requests.iter().any(|request| matches!(
        request.payload,
        HostRequest::RequestPermission { .. } | HostRequest::ExecuteTool { .. }
    )));
    assert!(transition.trace_events.iter().any(|trace| {
        trace.kind == nola_core::agent_trace::AgentTraceKind::ToolCompleted
            && trace.status == nola_core::agent_trace::AgentTraceStatus::Failed
            && trace.payload.call_id.as_deref() == Some("call-unknown-stable")
            && trace
                .payload
                .result
                .as_ref()
                .and_then(|value| value.get("output_preview"))
                .and_then(Value::as_str)
                .is_some_and(|value| {
                    value.contains("tool_not_exposed_in_model_step")
                        && value.contains("remembered_or_hallucinated_tool")
                })
    }));
    assert!(transition
        .host_requests
        .iter()
        .any(|request| matches!(request.payload, HostRequest::ModelStream { .. })));

    let checkpoint = core.checkpoint(CheckpointReason::Manual);
    assert!(checkpoint
        .state
        .conversation
        .transcript
        .iter()
        .any(|item| matches!(
            item,
            AgentItem::ToolCall { call_id, name, arguments: recorded, .. }
                if call_id.as_str() == "call-unknown-stable"
                    && name == "remembered_or_hallucinated_tool"
                    && recorded == &arguments
        )));
}

#[test]
fn current_turn_and_next_turn_inputs_are_consumed_at_named_boundaries() {
    let mut core = AgentCore::try_new(core_config(Vec::new())).expect("create Core");
    let started = core
        .submit(user_input(
            "user-1",
            "start",
            AgentInputDelivery::CurrentTurn,
        ))
        .expect("start turn");
    let first_turn_id = started.snapshot.active_turn_id.clone().expect("turn id");
    let first_model = model_request(&started.host_requests);

    core.submit(user_input(
        "user-steer",
        "steer",
        AgentInputDelivery::CurrentTurn,
    ))
    .expect("queue steering");
    let queued = core
        .submit(user_input(
            "user-next",
            "next",
            AgentInputDelivery::NextTurn,
        ))
        .expect("queue next turn");
    assert_eq!(queued.snapshot.queued_input_count, 2);

    let steered = submit_response(
        &mut core,
        &first_model,
        1,
        empty_model_completion(Vec::new()),
    );
    let second_model = model_request(&steered.host_requests);
    assert_eq!(
        steered.snapshot.active_turn_id.as_ref(),
        Some(&first_turn_id)
    );
    assert_eq!(steered.snapshot.queued_input_count, 1);

    let advanced = submit_response(
        &mut core,
        &second_model,
        1,
        visible_model_completion("steered-answer", "Steering handled."),
    );
    assert_eq!(
        advanced.snapshot.active_turn_id.as_ref(),
        Some(&first_turn_id)
    );
    assert_eq!(advanced.snapshot.queued_input_count, 1);
    assert!(advanced
        .event_envelopes
        .iter()
        .any(|envelope| matches!(envelope.event, AgentEvent::TurnCompleted { .. })));
    assert!(!advanced
        .event_envelopes
        .iter()
        .any(|envelope| matches!(envelope.event, AgentEvent::TurnStarted { .. })));
    let pending = core.checkpoint(CheckpointReason::Manual).state.execution.input_queue.pending_successor().unwrap().clone();
    let promoted = core.submit(AgentSubmission::PromoteQueuedInput {
        turn_context_components: Vec::new(),
        admission_id: pending.admission.admission_id,
        expected_previous_turn_id: first_turn_id.as_str().into(), successor_turn_id: "successor".into(),
    }).unwrap();
    assert_eq!(promoted.snapshot.queued_input_count, 0);
    assert_ne!(promoted.snapshot.active_turn_id.as_ref(), Some(&first_turn_id));
}

#[test]
fn input_admissions_survive_restart_and_preserve_cross_boundary_application_order() {
    let config = core_config(Vec::new());
    let mut core = AgentCore::try_new(config.clone()).unwrap();
    let started = core.submit(user_input("origin", "start", AgentInputDelivery::CurrentTurn)).unwrap();
    let first_model = model_request(&started.host_requests);
    let accepted = |transition: &nola_core::agent::AgentTransition| transition.event_envelopes.iter().find_map(|event|
        match &event.event { AgentEvent::InputAccepted { admission } => Some(admission.clone()), _ => None }).unwrap();
    let first = accepted(&started);
    assert_eq!(first.accepted_sequence, 1);
    let next_submission = user_input("next-source", "next", AgentInputDelivery::NextTurn);
    let next = core.submit(next_submission.clone()).unwrap();
    let next = accepted(&next);
    let premature = AgentSubmission::PromoteQueuedInput {
        turn_context_components: Vec::new(),
        admission_id: next.admission_id.clone(), expected_previous_turn_id: first.submission_id.clone().unwrap(),
        successor_turn_id: next.submission_id.clone().unwrap(),
    };
    let before = serde_json::to_value(core.checkpoint(CheckpointReason::Manual)).unwrap();
    assert!(core.submit(premature.clone()).is_err());
    assert_eq!(serde_json::to_value(core.checkpoint(CheckpointReason::Manual)).unwrap(), before);
    let current = core.submit(user_input("current-source", "steer", AgentInputDelivery::CurrentTurn)).unwrap();
    let current = accepted(&current);
    assert_eq!((next.accepted_sequence, current.accepted_sequence), (2, 3));
    let checkpoint = core.checkpoint(CheckpointReason::Manual);
    let checkpoint = serde_json::from_str(&serde_json::to_string(&checkpoint).unwrap()).unwrap();
    let mut core = AgentCore::restore(config, checkpoint).unwrap();
    let replay = core.submit(next_submission).unwrap();
    assert!(replay.event_envelopes.is_empty());
    assert!(replay.host_requests.is_empty());
    let before = serde_json::to_value(core.checkpoint(CheckpointReason::Manual)).unwrap();
    assert!(core.submit(user_input("next-source", "changed", AgentInputDelivery::NextTurn)).unwrap_err().contains("identity was reused"));
    assert_eq!(serde_json::to_value(core.checkpoint(CheckpointReason::Manual)).unwrap(), before);
    assert!(core.submit(user_input("next-source", "next", AgentInputDelivery::CurrentTurn)).is_err());
    let steered = submit_response(&mut core, &first_model, 1, empty_model_completion(Vec::new()));
    let application = steered.event_envelopes.iter().find_map(|event| match &event.event {
        AgentEvent::InputApplied { applications, .. } => applications.first(), _ => None }).unwrap();
    assert_eq!(application.admission_id, current.admission_id);
    assert_eq!(application.applied_sequence, 2);
    let second_model = model_request(&steered.host_requests);
    let advanced = submit_response(&mut core, &second_model, 1, visible_model_completion("answer", "Steering handled."));
    assert!(advanced.host_requests.is_empty());
    let advanced = core.submit(AgentSubmission::PromoteQueuedInput {
        turn_context_components: Vec::new(),
        admission_id: next.admission_id.clone(), expected_previous_turn_id: first.submission_id.clone().unwrap(),
        successor_turn_id: next.submission_id.clone().unwrap(),
    }).unwrap();
    let application = advanced.event_envelopes.iter().find_map(|event| match &event.event {
        AgentEvent::InputApplied { applications, .. } => applications.first(), _ => None }).unwrap();
    assert_eq!(application.admission_id, next.admission_id);
    assert_eq!(application.applied_sequence, 1);
    assert_ne!(application.destination_turn_id, first.submission_id.unwrap());
    let before = serde_json::to_value(core.checkpoint(CheckpointReason::Manual)).unwrap();
    assert!(core.submit(premature).is_err(), "already promoted input cannot start a second Turn");
    assert_eq!(serde_json::to_value(core.checkpoint(CheckpointReason::Manual)).unwrap(), before);
}

#[test]
fn input_admission_rejection_and_cancellation_do_not_leak_current_input_into_successor() {
    let mut core = AgentCore::try_new(core_config(Vec::new())).unwrap();
    core.submit(user_input("origin", "start", AgentInputDelivery::CurrentTurn)).unwrap();
    let current = core.submit(user_input("current", "steer", AgentInputDelivery::CurrentTurn)).unwrap();
    let current_id = current.event_envelopes.iter().find_map(|e| match &e.event {
        AgentEvent::InputAccepted { admission } => Some(admission.admission_id.clone()), _ => None }).unwrap();
    core.submit(user_input("next", "next", AgentInputDelivery::NextTurn)).unwrap();
    let before = serde_json::to_value(core.checkpoint(CheckpointReason::Manual)).unwrap();
    let mut oversized = user_input("overflow", "overflow", AgentInputDelivery::CurrentTurn);
    if let AgentSubmission::UserInput { items, .. } = &mut oversized {
        *items = (0..257).map(|i| AgentItem::user_message(ItemId::from_string(format!("overflow-{i}")), "overflow")).collect();
    }
    assert!(core.submit(oversized).is_err());
    assert_eq!(serde_json::to_value(core.checkpoint(CheckpointReason::Manual)).unwrap(), before);
    let cancelled = core.submit(AgentSubmission::Interrupt { goal_control: None, cancellation_id: "test-cancellation".into(), reason: Some("stop current".into()) }).unwrap();
    assert_eq!(cancelled.snapshot.queued_input_count, 1, "next-turn intent survives current cancellation");
    assert!(cancelled.event_envelopes.iter().any(|e| matches!(&e.event,
        AgentEvent::InputWithdrawn { admission_ids, .. } if admission_ids == &vec![current_id.clone()])));
    let checkpoint = serde_json::to_value(core.checkpoint(CheckpointReason::Manual)).unwrap();
    assert_eq!(checkpoint["state"]["execution"]["input_queue"]["withdrawn"][&current_id], "stop current");
    let reset = core.submit(AgentSubmission::Reset).unwrap();
    assert_eq!(reset.snapshot.queued_input_count, 0);
    assert!(reset.event_envelopes.iter().any(|event| matches!(&event.event,
        AgentEvent::InputWithdrawn { reason, .. } if reason == "session reset")));
    let replay = core.submit(user_input("next", "next", AgentInputDelivery::NextTurn)).unwrap();
    assert!(replay.event_envelopes.is_empty(), "reset must not reopen a withdrawn admission identity");
    assert!(replay.host_requests.is_empty());
    let checkpoint = serde_json::to_value(core.checkpoint(CheckpointReason::Manual)).unwrap();
    assert_eq!(checkpoint["state"]["execution"]["input_queue"]["accepted_sequence"], 3);
    assert_eq!(checkpoint["state"]["execution"]["input_queue"]["applications"].as_object().unwrap().len(), 1);
}

#[test]
fn failed_predecessor_withdraws_current_but_preserves_fifo_successor_authority() {
    let config = core_config(Vec::new());
    let mut core = AgentCore::try_new(config.clone()).unwrap();
    let initial = core.submit(user_input("origin", "start", AgentInputDelivery::CurrentTurn)).unwrap();
    let model = model_request(&initial.host_requests);
    core.submit(user_input("steer", "obsolete steering", AgentInputDelivery::CurrentTurn)).unwrap();
    core.submit(user_input("next", "independent next", AgentInputDelivery::NextTurn)).unwrap();
    core.submit(user_input("later", "independent later", AgentInputDelivery::NextTurn)).unwrap();
    let failed = submit_response(&mut core, &model, 1, HostResponse::Failed {
        error: HostError { code: "provider_error".into(), message: "failed".into(), retryable: false, http_status: None },
    });
    assert!(failed.host_requests.is_empty());
    assert_eq!(failed.snapshot.queued_input_count, 2);
    assert!(failed.event_envelopes.iter().any(|event| matches!(&event.event,
        AgentEvent::InputWithdrawn { admission_ids, .. } if admission_ids.len() == 1)));
    let checkpoint = core.checkpoint(CheckpointReason::Manual);
    let pending = checkpoint.state.execution.input_queue.pending_successor().unwrap().clone();
    assert_eq!(pending.admission.submission_id.as_deref(), Some("submission-next"));
    let mut core = AgentCore::restore(config, serde_json::from_value(serde_json::to_value(checkpoint).unwrap()).unwrap()).unwrap();
    assert!(core.submit(user_input("overtake", "new input", AgentInputDelivery::CurrentTurn)).is_err());
    let promoted = core.submit(AgentSubmission::PromoteQueuedInput {
        turn_context_components: Vec::new(),
        admission_id: pending.admission.admission_id, expected_previous_turn_id: "submission-origin".into(), successor_turn_id: "submission-next".into(),
    }).unwrap();
    let applied = promoted.event_envelopes.iter().find_map(|event| match &event.event {
        AgentEvent::InputApplied { items, .. } => Some(items), _ => None,
    }).unwrap();
    assert_eq!(applied.len(), 1);
    assert_eq!(applied[0].id().as_str(), "next");
    assert_eq!(promoted.snapshot.queued_input_count, 1);
    let checkpoint = core.checkpoint(CheckpointReason::Manual);
    let later = checkpoint.state.execution.input_queue.pending_successor().unwrap();
    assert_eq!(later.admission.submission_id.as_deref(), Some("submission-later"));
    assert_eq!(later.admission.anchor_turn_id.as_deref(), Some("submission-origin"), "never rewrite original source");
}

#[test]
fn turn_cancellation_requests_current_promoted_tasks_but_preserves_other_turns() {
    use nola_core::agent::task::{TaskHandle, TaskKind, TaskState, TaskLineage, TaskDurability};
    use nola_core::agent::TurnId;
    let config = core_config(Vec::new());
    let mut core = AgentCore::try_new(config.clone()).unwrap();
    core.submit(user_input("task-owner", "start", AgentInputDelivery::CurrentTurn)).unwrap();
    let mut checkpoint = core.checkpoint(CheckpointReason::Manual);
    let turn = checkpoint.state.execution.active_turn.as_ref().unwrap().turn_id.clone();
    for (id, owner) in [("first", turn.clone()), ("second", turn), ("earlier", TurnId::from_string("earlier-turn".into()))] {
        checkpoint.state.task_projection.order.push(id.into());
        checkpoint.state.task_projection.revision += 1;
        checkpoint.state.task_projection.tasks.insert(id.into(), TaskHandle {
            id: id.into(), owner_session_id: checkpoint.state.session_id.clone(), owner_turn_id: owner,
            kind: TaskKind::Agent, state: TaskState::Running, progress: None, current_step: None,
            result_ref: None, cancel_requested: false, cancel_requested_at_epoch_ms: None,
            lineage: TaskLineage::default(), durability: TaskDurability::default(),
        });
    }
    let mut core = AgentCore::restore(config, checkpoint).unwrap();
    let result = core.submit(AgentSubmission::Interrupt { goal_control: None, cancellation_id: "stop-parent".into(), reason: Some("stop".into()) }).unwrap();
    let mut cancelled = result.host_requests.iter().filter_map(|request| match &request.payload {
        HostRequest::CancelTask { task_id } => Some(task_id.as_str()), _ => None,
    }).collect::<Vec<_>>();
    cancelled.sort_unstable();
    assert_eq!(cancelled, vec!["first", "second"]);
    let checkpoint = core.checkpoint(CheckpointReason::Manual);
    assert!(checkpoint.state.task_projection.tasks["first"].cancel_requested);
    assert!(checkpoint.state.task_projection.tasks["second"].cancel_requested);
    assert!(!checkpoint.state.task_projection.tasks["earlier"].cancel_requested);
    // A task cancellation ACK may be last, after the model stop receipt.
    for request in result.host_requests.iter().rev() {
        let payload = match &request.payload {
            HostRequest::CancelTask { task_id } => HostResponse::Completed {
                output: json!({"task_id":task_id,"cancel_requested":true}), usage: None,
                full_output_ref: None, artifacts: Vec::new(),
            },
            HostRequest::Cancel { target_effect_id, .. } => HostResponse::CancellationConfirmed { target_effect_id: target_effect_id.clone() },
            _ => panic!("unexpected cancellation effect"),
        };
        submit_response(&mut core, request, 1, payload);
    }
    assert_eq!(core.checkpoint(CheckpointReason::Manual).state.execution.active_turn.unwrap().status,
        nola_core::agent::AgentTurnStatus::Cancelled);

}

#[test]
fn cancellation_is_correlated_and_rejects_a_response_from_another_host() {
    let mut core = AgentCore::try_new(core_config(Vec::new())).expect("create Core");
    let started = core
        .submit(user_input(
            "user-1",
            "start",
            AgentInputDelivery::CurrentTurn,
        ))
        .expect("start turn");
    let model = model_request(&started.host_requests);
    let interrupted = core
        .submit(AgentSubmission::Interrupt { goal_control: None, cancellation_id: "test-cancellation".into(),
            reason: Some("stop".to_string()),
        })
        .expect("interrupt");
    let cancel = request_matching(&interrupted.host_requests, |payload| {
        matches!(
            payload,
            HostRequest::Cancel { target_effect_id, .. }
                if target_effect_id == &model.effect_id
        )
    });

    let mut wrong_host = response_for(
        &cancel,
        1,
        HostResponse::CancellationConfirmed {
            target_effect_id: model.effect_id.clone(),
        },
    );
    wrong_host.host_id = HostId("other-host".to_string());
    let error = core
        .submit(AgentSubmission::HostResponse {
            response: wrong_host,
        })
        .expect_err("mismatched Host must be rejected");
    assert!(error.contains("rejected Host response"));

    let acknowledged = submit_response(
        &mut core,
        &cancel,
        1,
        HostResponse::CancellationConfirmed {
            target_effect_id: model.effect_id,
        },
    );
    assert!(matches!(
        acknowledged.snapshot.turn_status,
        nola_core::agent::AgentTurnStatus::Cancelled
    ));
}

#[test]
fn unsupported_tool_cancellation_blocks_turn_terminal_transition() {
    let mut config = core_config(Vec::new());
    let mut read = capability("read_file", HostCapabilityRoute::Tool, None);
    read.cancellation = HostCancellationMode::Unsupported;
    config
        .host_catalog
        .as_mut()
        .expect("Host catalog")
        .capabilities
        .insert("read_file".to_string(), read);
    config.tools.push(json!({
        "type":"function",
        "function":{
            "name":"read_file",
            "parameters":{"type":"object"}
        }
    }));
    let mut core = AgentCore::try_new(config).expect("create Core");
    let started = core
        .submit(user_input(
            "unsupported-cancel-user",
            "read the file",
            AgentInputDelivery::CurrentTurn,
        ))
        .expect("start turn");
    let model = model_request(&started.host_requests);
    let scheduled = submit_response(
        &mut core,
        &model,
        1,
        empty_model_completion(vec![named_tool_call(
            "unsupported-cancel-call",
            "read_file",
            json!({"path":"README.md"}),
        )]),
    );
    assert!(scheduled.host_requests.iter().any(|request| {
        matches!(
            &request.payload,
            HostRequest::ExecuteTool { call_id, .. }
                if call_id.as_str() == "unsupported-cancel-call"
        )
    }));

    let error = core
        .submit(AgentSubmission::Interrupt { goal_control: None, cancellation_id: "test-cancellation".into(),
            reason: Some("stop".to_string()),
        })
        .expect_err("non-cancellable tool must keep the Turn active");
    assert!(error.contains("unsupported-cancel-call"));
    assert!(matches!(
        core.snapshot().turn_status,
        nola_core::agent::AgentTurnStatus::AwaitingTools
    ));
}

#[test]
fn in_process_host_binds_an_omitted_response_turn_from_the_pending_effect() {
    let mut config = core_config(Vec::new());
    config.host_catalog.as_mut().expect("Host catalog").lease = None;
    let mut core = AgentCore::try_new(config).expect("create Core");
    let started = core
        .submit(user_input(
            "user-in-process",
            "answer",
            AgentInputDelivery::CurrentTurn,
        ))
        .expect("start turn");
    let model = model_request(&started.host_requests);
    let mut response = response_for(
        &model,
        1,
        visible_model_completion("assistant-in-process", "done"),
    );
    response.turn_id = None;

    core.submit(AgentSubmission::HostResponse { response })
        .expect("an in-process Host response can inherit the pending Turn identity");
}

#[test]
fn leased_host_must_echo_the_response_turn_identity() {
    let mut core = AgentCore::try_new(core_config(Vec::new())).expect("create Core");
    let started = core
        .submit(user_input(
            "user-leased",
            "answer",
            AgentInputDelivery::CurrentTurn,
        ))
        .expect("start turn");
    let model = model_request(&started.host_requests);
    let mut response = response_for(
        &model,
        1,
        visible_model_completion("assistant-leased", "done"),
    );
    response.turn_id = None;

    let error = core
        .submit(AgentSubmission::HostResponse { response })
        .expect_err("a leased Host must not omit the Turn identity");
    assert!(error.contains("code=wrong_turn"));
}

#[test]
fn cancelling_a_stream_closes_parts_and_absorbs_one_late_terminal_response() {
    let mut core = AgentCore::try_new(core_config(Vec::new())).expect("create Core");
    let started = core
        .submit(user_input(
            "cancel-stream-user",
            "start streaming",
            AgentInputDelivery::CurrentTurn,
        ))
        .expect("start turn");
    let model = model_request(&started.host_requests);
    let streamed = submit_response(&mut core, &model, 1, model_delta(&model, "partial"));
    assert!(streamed.snapshot.projection.parts.values().any(|part| {
        matches!(part, AgentPart::Text(text) if text.text == "partial" && !text.is_complete)
    }));

    let interrupted = core
        .submit(AgentSubmission::Interrupt { goal_control: None, cancellation_id: "test-cancellation".into(),
            reason: Some("stop".to_string()),
        })
        .expect("interrupt stream");
    assert_eq!(
        interrupted.snapshot.turn_status,
        nola_core::agent::AgentTurnStatus::Cancelling
    );

    let late = submit_response(&mut core, &model, 2, empty_model_completion(Vec::new()));
    assert!(late.event_envelopes.is_empty());
    assert_eq!(
        late.snapshot.turn_status,
        nola_core::agent::AgentTurnStatus::Cancelling
    );
    let acknowledged = confirm_cancellation(&mut core, &interrupted, &model.effect_id);
    assert!(matches!(
        acknowledged.snapshot.turn_status,
        nola_core::agent::AgentTurnStatus::Cancelled
    ));
    assert!(acknowledged.snapshot.projection.parts.values().all(|part| {
        part.turn_id() != model.turn_id.as_deref().expect("model Turn") || part.is_terminal()
    }));
    let duplicate = core.submit(AgentSubmission::HostResponse {
        response: response_for(&model, 3, empty_model_completion(Vec::new())),
    });
    assert!(
        duplicate.is_err(),
        "a terminal response must be accepted exactly once"
    );
}

#[test]
fn model_failure_closes_stream_part_once_and_rejects_duplicate_failure() {
    let mut core = AgentCore::try_new(core_config(Vec::new())).expect("create Core");
    let started = core
        .submit(user_input(
            "failed-stream-user",
            "start streaming",
            AgentInputDelivery::CurrentTurn,
        ))
        .expect("start turn");
    let model = model_request(&started.host_requests);
    submit_response(&mut core, &model, 1, model_delta(&model, "partial"));
    let failure = HostResponse::Failed {
        error: HostError {
            code: "provider_error".to_string(),
            message: "provider failed".to_string(),
            retryable: false,
            http_status: None,
        },
    };
    let failed = submit_response(&mut core, &model, 2, failure.clone());

    assert!(matches!(
        failed.snapshot.turn_status,
        nola_core::agent::AgentTurnStatus::Failed
    ));
    assert!(failed.snapshot.projection.parts.values().all(|part| {
        part.turn_id() != model.turn_id.as_deref().expect("model Turn") || part.is_terminal()
    }));
    assert!(failed.event_envelopes.iter().any(|event| {
        matches!(&event.event, AgentEvent::TurnFailed { error, .. } if error == "provider failed")
    }));
    assert!(core
        .submit(AgentSubmission::HostResponse {
            response: response_for(&model, 3, failure),
        })
        .is_err());
}

#[test]
fn execution_rejection_is_normalized_to_a_terminal_core_failure() {
    let mut core = AgentCore::try_new(core_config(Vec::new())).expect("create Core");
    let started = core
        .submit(user_input(
            "rejected-before-start-user",
            "start streaming",
            AgentInputDelivery::CurrentTurn,
        ))
        .expect("start turn");
    let model = model_request(&started.host_requests);

    let rejected = submit_response(
        &mut core,
        &model,
        1,
        HostResponse::ExecutionRejected {
            code: "deadline_exceeded".into(),
            message: "expired before execution".into(),
            retryable: false,
        },
    );

    assert!(matches!(
        rejected.snapshot.turn_status,
        nola_core::agent::AgentTurnStatus::Failed
    ));
    assert!(rejected.event_envelopes.iter().any(|event| {
        matches!(&event.event, AgentEvent::TurnFailed { error, .. } if error == "expired before execution")
    }));
    assert!(core
        .submit(AgentSubmission::HostResponse {
            response: response_for(
                &model,
                2,
                HostResponse::ExecutionRejected {
                    code: "deadline_exceeded".into(),
                    message: "duplicate".into(),
                    retryable: false,
                },
            ),
        })
        .is_err());
}

#[test]
fn web_search_result_projects_stable_sources_and_citations() {
    let mut core = AgentCore::try_new(source_core_config()).expect("create Core");
    let started = core
        .submit(user_input(
            "web-source-user",
            "find primary evidence",
            AgentInputDelivery::CurrentTurn,
        ))
        .expect("start turn");
    let model = model_request(&started.host_requests);
    let planned = submit_response(
        &mut core,
        &model,
        1,
        empty_model_completion(vec![named_tool_call(
            "web-source-call",
            "web_search",
            json!({"query": "primary evidence"}),
        )]),
    );
    let tool = request_matching(
        &planned.host_requests,
        |payload| matches!(payload, HostRequest::ExecuteTool { call_id, .. } if call_id == "web-source-call"),
    );
    let completed = submit_response(
        &mut core,
        &tool,
        1,
        HostResponse::ToolCompleted {
            call_id: "web-source-call".to_string(),
            output: json!({
                "results": [
                    {
                        "title": "Primary report",
                        "url": "https://example.com/report",
                        "mime_type": "text/html",
                        "page": 3,
                        "range": {"start": 12, "end": 28}
                    },
                    {
                        "title": "Primary report",
                        "url": "https://example.com/report#results",
                        "mime_type": "text/html",
                        "page": 3,
                        "range": {"start": 12, "end": 28}
                    }
                ]
            }),
            outcome: nola_core::agent::host::HostToolOutcome::Succeeded,
            activation: None,
            full_output_ref: None,
            artifacts: Vec::new(),
        },
    );

    let source = completed
        .snapshot
        .projection
        .parts
        .values()
        .find_map(|part| match part {
            AgentPart::Source(source) => Some(source),
            _ => None,
        })
        .expect("Source Part");
    let citation = completed
        .snapshot
        .projection
        .parts
        .values()
        .find_map(|part| match part {
            AgentPart::Citation(citation) => Some(citation),
            _ => None,
        })
        .expect("Citation Part");
    assert_eq!(source.uri, "https://example.com/report");
    assert_eq!(source.title, "Primary report");
    assert_eq!(citation.source_id, source.source_id);
    assert_eq!(citation.origin_tool_call_id, "web-source-call");
    assert_eq!(
        citation
            .location
            .as_ref()
            .and_then(|location| location.page),
        Some(3)
    );
    assert_eq!(
        citation
            .range
            .as_ref()
            .map(|range| (range.start, range.end)),
        Some((12, 28))
    );
    assert_eq!(
        completed
            .snapshot
            .projection
            .parts
            .values()
            .filter(|part| matches!(part, AgentPart::Source(_)))
            .count(),
        1
    );
    assert_eq!(
        completed
            .snapshot
            .projection
            .parts
            .values()
            .filter(|part| matches!(part, AgentPart::Citation(_)))
            .count(),
        1
    );
    assert!(completed.event_envelopes.iter().any(|event| {
        matches!(
            &event.event,
            AgentEvent::PartUpdated {
                part: AgentPart::Source(_),
                ..
            }
        )
    }));
    assert!(completed.event_envelopes.iter().any(|event| {
        matches!(
            &event.event,
            AgentEvent::PartUpdated {
                part: AgentPart::Citation(_),
                ..
            }
        )
    }));

    let value = serde_json::to_value(&completed.snapshot.projection).expect("serialize projection");
    let restored: ConversationProjection =
        serde_json::from_value(value).expect("restore projection");
    assert_eq!(restored, completed.snapshot.projection);
}

#[test]
fn file_read_stays_in_tool_part_and_recovers_from_checkpoint() {
    let mut core = AgentCore::try_new(source_core_config()).expect("create Core");
    let started = core
        .submit(user_input(
            "file-source-user",
            "inspect the implementation",
            AgentInputDelivery::CurrentTurn,
        ))
        .expect("start turn");
    let model = model_request(&started.host_requests);
    let planned = submit_response(
        &mut core,
        &model,
        1,
        empty_model_completion(vec![named_tool_call(
            "file-source-call",
            "read_file",
            json!({"path": "src/runtime.rs", "start_line": 40, "end_line": 55}),
        )]),
    );
    let tool = request_matching(
        &planned.host_requests,
        |payload| matches!(payload, HostRequest::ExecuteTool { call_id, .. } if call_id == "file-source-call"),
    );
    let completed = submit_response(
        &mut core,
        &tool,
        1,
        HostResponse::ToolCompleted {
            call_id: "file-source-call".to_string(),
            output: json!({
                "text": "fn run() {}",
                "line_start": 40,
                "line_end": 55,
                "mime_type": "text/x-rust"
            }),
            outcome: nola_core::agent::host::HostToolOutcome::Succeeded,
            activation: None,
            full_output_ref: None,
            artifacts: Vec::new(),
        },
    );
    let tool_part = completed
        .snapshot
        .projection
        .parts
        .values()
        .find_map(|part| match part {
            AgentPart::Tool(tool) if tool.call_id == "file-source-call" => Some(tool),
            _ => None,
        })
        .expect("file Tool Part");
    assert_eq!(
        tool_part.structured_output.as_ref(),
        Some(&json!({
            "text": "fn run() {}",
            "line_start": 40,
            "line_end": 55,
            "mime_type": "text/x-rust"
        }))
    );
    assert!(completed
        .snapshot
        .projection
        .parts
        .values()
        .all(|part| { !matches!(part, AgentPart::Source(_) | AgentPart::Citation(_)) }));

    let checkpoint = checkpoint_after(&core, &completed);
    assert!(checkpoint
        .state
        .conversation
        .projection
        .parts
        .values()
        .any(|part| matches!(
            part,
            AgentPart::Tool(tool)
                if tool.call_id == "file-source-call"
                    && tool.structured_output.as_ref().and_then(|value| value.get("line_start")).and_then(Value::as_u64) == Some(40)
        )));
    assert!(checkpoint
        .state
        .conversation
        .projection
        .parts
        .values()
        .all(|part| !matches!(part, AgentPart::Source(_) | AgentPart::Citation(_))));
}

#[test]
fn repeated_file_reads_across_turns_never_become_reply_sources() {
    let mut core = AgentCore::try_new(source_core_config()).expect("create Core");
    let first_started = core
        .submit(user_input(
            "repeated-file-source-user-1",
            "inspect the implementation",
            AgentInputDelivery::CurrentTurn,
        ))
        .expect("start first turn");
    let first_model = model_request(&first_started.host_requests);
    let first_planned = submit_response(
        &mut core,
        &first_model,
        1,
        empty_model_completion(vec![named_tool_call(
            "repeated-file-source-call-1",
            "read_file",
            json!({"path": "src/runtime.rs"}),
        )]),
    );
    let first_tool = request_matching(
        &first_planned.host_requests,
        |payload| matches!(payload, HostRequest::ExecuteTool { call_id, .. } if call_id == "repeated-file-source-call-1"),
    );
    let first_completed = submit_response(
        &mut core,
        &first_tool,
        1,
        HostResponse::ToolCompleted {
            call_id: "repeated-file-source-call-1".to_string(),
            output: json!({"text": "fn run() {}", "mime_type": "text/x-rust"}),
            outcome: nola_core::agent::host::HostToolOutcome::Succeeded,
            activation: None,
            full_output_ref: None,
            artifacts: Vec::new(),
        },
    );
    assert!(first_completed
        .snapshot
        .projection
        .parts
        .values()
        .all(|part| { !matches!(part, AgentPart::Source(_) | AgentPart::Citation(_)) }));
    let first_continuation = model_request(&first_completed.host_requests);
    let first_candidate = submit_response(
        &mut core,
        &first_continuation,
        1,
        model_requested_review("review-first-inspection", "first inspection complete"),
    );
    let audit = model_request(&first_candidate.host_requests);
    let reviewed = submit_response(
        &mut core,
        &audit,
        1,
        visible_model_completion(
            "review",
            r#"{"decision":"accept","outcome":"answered","constraints":[],"items":[{"label":"Fixture task requirement","eligibility":"eligible","evidence":"Observed fixture evidence supports the candidate"}],"violations":[]}"#,
        ),
    );
    let finished = reviewed;
    assert!(finished
        .event_envelopes
        .iter()
        .any(|event| matches!(event.event, AgentEvent::TurnCompleted { .. })));

    let second_started = core
        .submit(user_input(
            "repeated-file-source-user-2",
            "inspect the same implementation again",
            AgentInputDelivery::CurrentTurn,
        ))
        .expect("start second turn");
    let second_model = model_request(&second_started.host_requests);
    let second_planned = submit_response(
        &mut core,
        &second_model,
        1,
        empty_model_completion(vec![named_tool_call(
            "repeated-file-source-call-2",
            "read_file",
            json!({"path": "src/runtime.rs"}),
        )]),
    );
    let second_tool = request_matching(
        &second_planned.host_requests,
        |payload| matches!(payload, HostRequest::ExecuteTool { call_id, .. } if call_id == "repeated-file-source-call-2"),
    );
    let second_completed = submit_response(
        &mut core,
        &second_tool,
        1,
        HostResponse::ToolCompleted {
            call_id: "repeated-file-source-call-2".to_string(),
            output: json!({"text": "fn run() {}", "mime_type": "text/x-rust"}),
            outcome: nola_core::agent::host::HostToolOutcome::Succeeded,
            activation: None,
            full_output_ref: None,
            artifacts: Vec::new(),
        },
    );
    assert!(second_completed
        .snapshot
        .projection
        .parts
        .values()
        .all(|part| { !matches!(part, AgentPart::Source(_) | AgentPart::Citation(_)) }));
    assert_eq!(
        second_completed
            .snapshot
            .projection
            .parts
            .values()
            .filter(
                |part| matches!(part, AgentPart::Tool(tool) if tool.capability_id == "read_file")
            )
            .count(),
        2
    );
}

#[test]
fn cancelling_permission_wait_closes_tool_and_ignores_late_decision() {
    let mut core = AgentCore::try_new(engineering_core_config()).expect("create Core");
    let started = core
        .submit(user_input(
            "cancel-permission-user",
            "write protected output",
            AgentInputDelivery::CurrentTurn,
        ))
        .expect("start turn");
    let model = model_request(&started.host_requests);
    let waiting = submit_response(
        &mut core,
        &model,
        1,
        empty_model_completion(vec![tool_call("cancel-permission-call")]),
    );
    let permission = request_matching(&waiting.host_requests, |payload| {
        matches!(payload, HostRequest::RequestPermission { .. })
    });
    let permission_request_id = match &permission.payload {
        HostRequest::RequestPermission {
            permission_request_id,
            ..
        } => permission_request_id.clone(),
        _ => unreachable!(),
    };

    let interrupted = core
        .submit(AgentSubmission::Interrupt { goal_control: None, cancellation_id: "test-cancellation".into(),
            reason: Some("stop while asking".to_string()),
        })
        .expect("interrupt permission wait");
    assert_eq!(
        interrupted.snapshot.turn_status,
        nola_core::agent::AgentTurnStatus::Cancelling
    );
    let execution = core.host_execution_state();
    assert!(!execution.pending_effects.iter().find(|effect| effect.effect_id == permission.effect_id.0)
        .unwrap().recovery_rebind_allowed);
    assert!(execution.pending_effects.iter().filter(|effect| effect.capability_id == "host.cancel")
        .all(|effect| effect.recovery_rebind_allowed));
    let error = core.submit(AgentSubmission::ResumeHostEffect { effect_id: permission.effect_id.0.clone() })
        .expect_err("cancellation must not reopen a permission prompt");
    assert!(error.contains("cancelling Host effect cannot be rebound"), "{error}");
    let cancelled = confirm_cancellation(&mut core, &interrupted, &permission.effect_id);
    assert!(cancelled
        .snapshot
        .projection
        .parts
        .values()
        .all(|part| { !matches!(part, AgentPart::Tool(tool) if !tool.state.is_terminal()) }));
    assert!(cancelled.snapshot.projection.parts.values().any(|part| {
        matches!(
            part,
            AgentPart::Tool(tool)
                if tool.call_id == "cancel-permission-call"
                    && tool.state == nola_core::agent::parts::ToolPartState::Cancelled
        )
    }));

    let late = core.submit(AgentSubmission::HostResponse {
        response: response_for(
            &permission,
            1,
            HostResponse::PermissionResolved {
                request_id: permission_request_id,
                resolution: nola_core::agent::host::HostPermissionResolution::allow_once(),
            },
        ),
    });
    assert!(
        late.is_err(),
        "a response after cancellation acknowledgement is stale"
    );
}

#[test]
fn foreign_permission_resolution_preserves_wait_and_allows_same_sequence_retry() {
    use nola_core::agent::host::{HostPermissionIntervention, HostPermissionResolution};

    let config = engineering_core_config();
    let mut core = AgentCore::try_new(config.clone()).unwrap();
    let started = core
        .submit(user_input(
            "permission-identity-user",
            "write protected output",
            AgentInputDelivery::CurrentTurn,
        ))
        .unwrap();
    let model = model_request(&started.host_requests);
    let waiting = submit_response(
        &mut core,
        &model,
        1,
        empty_model_completion(vec![tool_call("permission-identity-call")]),
    );
    let permission = request_matching(&waiting.host_requests, |payload| {
        matches!(payload, HostRequest::RequestPermission { .. })
    });
    let request_id = match &permission.payload {
        HostRequest::RequestPermission {
            permission_request_id,
            ..
        } => permission_request_id.clone(),
        _ => unreachable!(),
    };
    let before = core.checkpoint(CheckpointReason::Manual);
    let mut checkpoint_candidate = AgentCore::restore(config.clone(), before.clone()).unwrap();
    checkpoint_candidate
        .bind_committed_host_effect(&permission)
        .unwrap();
    let borrowed_checkpoint = checkpoint_candidate.checkpoint(CheckpointReason::Manual);
    {
        let view = checkpoint_candidate.checkpoint_view(CheckpointReason::Manual);
        assert!(matches!(&view.state, std::borrow::Cow::Borrowed(_)));
        assert_eq!(
            serde_json::to_value(&view).unwrap(),
            serde_json::to_value(&borrowed_checkpoint).unwrap()
        );
    }
    let moved_checkpoint = checkpoint_candidate.into_checkpoint(CheckpointReason::Manual);
    assert_eq!(
        serde_json::to_value(moved_checkpoint).unwrap(),
        serde_json::to_value(borrowed_checkpoint).unwrap()
    );
    for resolution in [
        HostPermissionResolution::return_to_composer(),
        HostPermissionResolution::allow_once(),
        HostPermissionResolution::Intervention {
            intervention: HostPermissionIntervention::ProvideInstruction {
                instruction: "change target".into(),
            },
        },
    ] {
        let submission = AgentSubmission::HostResponse {
            response: response_for(
                &permission,
                1,
                HostResponse::PermissionResolved {
                    request_id: "permission-from-another-request".into(),
                    resolution,
                },
            ),
        };
        assert!(core.submit_at(submission.clone(), 5_000).is_err());
        assert_eq!(
            serde_json::to_value(core.checkpoint(CheckpointReason::Manual)).unwrap(),
            serde_json::to_value(&before).unwrap()
        );
        // Application's consuming path must reject the same payload after a
        // cold restore/rebind, without returning an adoptable candidate.
        let mut candidate = AgentCore::restore(config.clone(), before.clone()).unwrap();
        candidate.bind_committed_host_effect(&permission).unwrap();
        assert!(candidate.reduce_owned_at(submission, 5_000).is_err());
    }
    let accepted = submit_response(
        &mut core,
        &permission,
        1,
        HostResponse::PermissionResolved {
            request_id,
            resolution: HostPermissionResolution::allow_once(),
        },
    );
    assert!(accepted
        .host_requests
        .iter()
        .any(|request| matches!(request.payload, HostRequest::ExecuteTool { .. })));
}

#[test]
fn composer_interruption_and_later_grant_remain_distinct_after_restore() {
    let config = engineering_core_config();
    let mut core = AgentCore::try_new(config.clone()).unwrap();
    let started = core
        .submit(user_input(
            "permission-history-user",
            "查看当前项目",
            AgentInputDelivery::CurrentTurn,
        ))
        .unwrap();
    let model = model_request(&started.host_requests);
    let waiting = submit_response(
        &mut core,
        &model,
        1,
        empty_model_completion(vec![tool_call("history-interrupted-call")]),
    );
    let permission = request_matching(&waiting.host_requests, |payload| {
        matches!(payload, HostRequest::RequestPermission { .. })
    });
    let request_id = match &permission.payload {
        HostRequest::RequestPermission {
            permission_request_id,
            ..
        } => permission_request_id.clone(),
        _ => unreachable!(),
    };
    let cancelled = submit_response(
        &mut core,
        &permission,
        1,
        HostResponse::PermissionResolved {
            request_id,
            resolution: nola_core::agent::host::HostPermissionResolution::return_to_composer(),
        },
    );
    assert_eq!(
        cancelled.snapshot.turn_status,
        nola_core::agent::AgentTurnStatus::Cancelled
    );
    let checkpoint = core.checkpoint(CheckpointReason::Manual);
    let serialized = serde_json::to_value(&checkpoint.state).unwrap();
    let receipts = &serialized["execution"]["permissions"]["receipts"];
    assert_eq!(receipts[0]["outcome"]["type"], "interrupted");
    assert_eq!(receipts[0]["outcome"]["reason"], "returned_to_composer");
    let mut core = AgentCore::restore(config, checkpoint).unwrap();
    let resumed = core
        .submit(user_input(
            "permission-history-followup",
            "继续 / continue",
            AgentInputDelivery::CurrentTurn,
        ))
        .unwrap();
    let model = model_request(&resumed.host_requests);
    let serialized = serde_json::to_string(&model.payload).unwrap();
    assert!(serialized.contains("returned_to_composer"));
    assert!(serialized.contains("permission_decision"));
    let waiting = submit_response(
        &mut core,
        &model,
        1,
        empty_model_completion(vec![tool_call("history-approved-call")]),
    );
    let permission = request_matching(&waiting.host_requests, |payload| {
        matches!(payload, HostRequest::RequestPermission { .. })
    });
    let request_id = match &permission.payload {
        HostRequest::RequestPermission {
            permission_request_id,
            ..
        } => permission_request_id.clone(),
        _ => unreachable!(),
    };
    submit_response(
        &mut core,
        &permission,
        1,
        HostResponse::PermissionResolved {
            request_id,
            resolution: nola_core::agent::host::HostPermissionResolution::allow_conversation(None),
        },
    );
    let serialized = serde_json::to_value(core.checkpoint(CheckpointReason::Manual).state).unwrap();
    let receipts = &serialized["execution"]["permissions"]["receipts"];
    assert_eq!(receipts.as_array().unwrap().len(), 2);
    assert_eq!(receipts[0]["outcome"]["type"], "interrupted");
    assert_eq!(receipts[1]["outcome"]["type"], "authorization");
    assert_eq!(receipts[1]["outcome"]["decision"], "allow_conversation");
}

#[test]
fn malformed_sop_lineage_settles_without_permission_or_execution() {
    let mut core = AgentCore::try_new(engineering_core_config()).unwrap();
    let started = core
        .submit(user_input(
            "invalid-lineage-user",
            "write the result",
            AgentInputDelivery::CurrentTurn,
        ))
        .unwrap();
    let model = model_request(&started.host_requests);
    let rejected = submit_response(
        &mut core,
        &model,
        1,
        empty_model_completion(vec![named_tool_call(
            "invalid-lineage-call",
            "write_file",
            json!({
                "path": "/tmp/must-not-be-written.txt", "content": "never",
                "_nolaExecution": {"sopRunId": "run", "sopStepId": ""}
            }),
        )]),
    );
    assert!(!rejected.host_requests.iter().any(|request| matches!(
        request.payload,
        HostRequest::ExecuteTool { .. } | HostRequest::RequestPermission { .. }
    )));
    let encoded = serde_json::to_string(&rejected.snapshot).unwrap();
    assert!(encoded.contains("invalid_sop_execution_lineage"));
    assert!(encoded.contains("tool_execution_skipped"));
}

#[test]
fn user_question_is_durable_without_host_effect_and_response_is_replay_safe() {
    let mut config = source_core_config();
    config.host_catalog.as_mut().unwrap().capabilities.insert(
        "request_user_input".into(),
        capability("request_user_input", HostCapabilityRoute::Tool, None),
    );
    config.tools.push(json!({"type":"function", "function":{"name":"request_user_input", "parameters":{"type":"object"}}}));
    let mut core = AgentCore::try_new(config.clone()).unwrap();
    let started = core
        .submit(user_input(
            "question-user",
            "Ask me about the unresolved scope",
            AgentInputDelivery::CurrentTurn,
        ))
        .unwrap();
    let model = model_request(&started.host_requests);
    let asked = submit_response(
        &mut core,
        &model,
        1,
        empty_model_completion(vec![named_tool_call(
            "scope-question",
            "request_user_input",
            json!({
                "questions":[{"id":"scope", "prompt":"Should the generated executable be excluded?", "options":[{"id":"exclude","label":"Exclude","description":null}], "allow_freeform":true, "required":true,"max_text_chars":8192}],
                "_nolaExecution":{"sopRunId":"terminal-work:resolve", "sopStepId":"resolve"}
            }),
        )]),
    );
    assert!(asked.host_requests.is_empty());
    assert_eq!(asked.snapshot.turn_status, nola_domain::AgentTurnStatus::AwaitingInput);
    assert!(!asked.event_envelopes.iter().any(|event| matches!(event.event, AgentEvent::TurnCompleted { .. })));
    let wait = asked.snapshot.open_user_input_waits[0].clone();
    assert!(asked.event_envelopes.iter().any(|event| matches!(&event.event,
        AgentEvent::UserInputWaitChanged { wait: found } if found == &wait)));
    let checkpoint = checkpoint_after(&core, &asked);
    checkpoint.validate_state_identity().unwrap();
    core = AgentCore::restore(config, checkpoint).unwrap();
    use nola_domain::user_input_wait::*;
    let input = RespondToUserInputWait { command_id:"answer-scope".into(), wait_id:wait.wait_id,
        expected_revision:1,response_schema_digest:wait.response_schema_digest,
        response:UserInputWaitResponse::Answers { answers:vec![QuestionAnswer {question_id:"scope".into(),value:QuestionAnswerValue::Option {option_id:"exclude".into()}}] } };
    let mut invalid = input.clone(); invalid.expected_revision = 2;
    assert!(core.submit(AgentSubmission::RespondToUserInputWait { input:invalid }).is_err());
    let answered = core.submit(AgentSubmission::RespondToUserInputWait { input:input.clone() }).unwrap();
    assert!(answered.snapshot.open_user_input_waits.is_empty());
    model_request(&answered.host_requests);
    let replay = core.submit(AgentSubmission::RespondToUserInputWait { input:input.clone() }).unwrap();
    assert!(replay.host_requests.is_empty() && replay.event_envelopes.is_empty());
    let mut conflict = input; conflict.response = UserInputWaitResponse::Decline {reason:None};
    assert!(core.submit(AgentSubmission::RespondToUserInputWait {input:conflict}).is_err());
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

#[test]
fn durable_question_multiple_waits_and_tool_join_are_order_independent() {
    for answer_before_tool in [false,true] {
        let config = durable_question_config();
        let mut core = AgentCore::try_new(config.clone()).unwrap();
        let started = core.submit(user_input("multi-question-user","Inspect and ask",AgentInputDelivery::CurrentTurn)).unwrap();
        let asked = submit_response(&mut core,&model_request(&started.host_requests),1,
            empty_model_completion(vec![named_tool_call("q1","request_user_input",durable_question_arguments("first")),
                named_tool_call("q2","request_user_input",durable_question_arguments("second")),
                named_tool_call("read-independent","read_file",json!({"path":"README.md"}))]));
        assert_eq!(asked.snapshot.open_user_input_waits.len(),2);
        let read = request_matching(&asked.host_requests,|payload| matches!(payload,HostRequest::ExecuteTool {call_id,..} if call_id=="read-independent"));
        let waits = asked.snapshot.open_user_input_waits.clone();
        let checkpoint = checkpoint_after(&core,&asked); checkpoint.validate_state_identity().unwrap();
        core = AgentCore::restore(config,checkpoint).unwrap();
        if answer_before_tool {
            for (index,wait) in waits.iter().rev().enumerate() {
                let answered = core.submit(answer_wait(wait,&format!("answer-{index}"))).unwrap();
                assert!(!answered.host_requests.iter().any(|r|matches!(r.payload,HostRequest::ModelStream {..})));
            }
        }
        let read_done = submit_response(&mut core,&read,1,HostResponse::ToolCompleted {
            call_id:"read-independent".into(), output:json!({"content":"read completed"}),
            outcome:nola_core::agent::host::HostToolOutcome::Succeeded,activation:None,full_output_ref:None,artifacts:Vec::new()});
        if answer_before_tool { model_request(&read_done.host_requests); }
        else {
            assert_eq!(read_done.snapshot.turn_status,nola_domain::AgentTurnStatus::AwaitingInput);
            assert_eq!(read_done.snapshot.open_user_input_waits.len(),2);
            assert!(read_done.host_requests.is_empty());
            let first = core.submit(answer_wait(&waits[1],"answer-last-first")).unwrap();
            assert!(first.host_requests.is_empty());
            assert_eq!(first.snapshot.open_user_input_waits.len(),1);
            let last = core.submit(answer_wait(&waits[0],"answer-first-last")).unwrap();
            model_request(&last.host_requests);
        }
    }
}

#[test]
fn durable_question_cancel_and_reset_withdraw_and_reject_late_answer() {
    for reset in [false,true] {
        let config = durable_question_config();
        let mut core = AgentCore::try_new(config.clone()).unwrap();
        let started=core.submit(user_input("cancel-q","Ask",AgentInputDelivery::CurrentTurn)).unwrap();
        let asked=submit_response(&mut core,&model_request(&started.host_requests),1,empty_model_completion(vec![named_tool_call("q","request_user_input",durable_question_arguments("scope"))]));
        let wait=asked.snapshot.open_user_input_waits[0].clone();
        let cancelled=core.submit(if reset {AgentSubmission::Reset} else {AgentSubmission::Interrupt { goal_control: None, cancellation_id: "test-cancellation".into(),reason:Some("user cancelled".into())}}).unwrap();
        assert!(cancelled.snapshot.open_user_input_waits.is_empty());
        assert!(cancelled.event_envelopes.iter().any(|event|matches!(&event.event,AgentEvent::UserInputWaitChanged {wait} if matches!(wait.state,nola_domain::user_input_wait::UserInputWaitState::Withdrawn {..}))));
        let checkpoint=checkpoint_after(&core,&cancelled);checkpoint.validate_state_identity().unwrap();
        core=AgentCore::restore(config,checkpoint).unwrap();
        assert!(core.submit(answer_wait(&wait,"late-answer")).is_err());
    }
}

#[test]
fn durable_question_decline_is_data_not_permission_or_run_cancellation() {
    let mut core=AgentCore::try_new(durable_question_config()).unwrap();
    let started=core.submit(user_input("decline-q","Ask",AgentInputDelivery::CurrentTurn)).unwrap();
    let asked=submit_response(&mut core,&model_request(&started.host_requests),1,empty_model_completion(vec![named_tool_call("q","request_user_input",durable_question_arguments("scope"))]));
    let wait=&asked.snapshot.open_user_input_waits[0];
    use nola_domain::user_input_wait::*;
    let answered=core.submit(AgentSubmission::RespondToUserInputWait {input:RespondToUserInputWait {
        command_id:"explicit-decline".into(),wait_id:wait.wait_id.clone(),expected_revision:1,response_schema_digest:wait.response_schema_digest.clone(),
        response:UserInputWaitResponse::Decline {reason:Some("cannot answer".into())}}}).unwrap();
    model_request(&answered.host_requests);
    assert!(!answered.snapshot.turn_status.is_terminal());
    assert!(answered.snapshot.open_user_input_waits.is_empty());
    assert!(answered.event_envelopes.iter().any(|event|matches!(&event.event,AgentEvent::UserInputWaitChanged {wait} if matches!(wait.state,UserInputWaitState::Resolved {..}))));
    assert!(!answered.host_requests.iter().any(|request|matches!(request.payload,HostRequest::RequestPermission {..})));
}

#[test]
fn sop_execution_lineage_survives_permission_wait_checkpoint_and_resume() {
    let mut core = AgentCore::try_new(engineering_core_config()).expect("create Core");
    let started = core
        .submit(AgentSubmission::UserInput {
            items: vec![AgentItem::user_message(
                ItemId::from_string("sop-permission-lineage-user".to_string()),
                "write the current SOP result",
            )],
            turn_context_items: Vec::new(),
            turn_context_platform: ContextPlatform::default(),
            turn_context_components: vec![ContextComponent {
                kind: ContextComponentKind::System,
                id: nola_core::agent::tool_lifecycle::ACTIVE_SOP_RUNS_COMPONENT_ID.to_string(),
                content: json!({
                    "contract": nola_core::agent::tool_lifecycle::ACTIVE_SOP_RUNS_CONTRACT_ID,
                    "runs": [{
                        "runId": "sop-run-1",
                        "status": "running",
                        "definitionPinned": true,
                        "steps": [{"id": "write-result", "status": "running"}]
                    }]
                }),
                source: None,
            }],
            submission_id: None,
            submitted_at_epoch_ms: None,
            delivery: AgentInputDelivery::CurrentTurn,
        })
        .expect("start turn");
    let model = model_request(&started.host_requests);
    let waiting = submit_response(
        &mut core,
        &model,
        1,
        empty_model_completion(vec![named_tool_call(
            "sop-permission-lineage-call",
            "write_file",
            json!({
                "path": "/tmp/sop-lineage.txt",
                "content": "lineage",
                "_nolaExecution": {
                    "sopRunId": "sop-run-1",
                    "sopStepId": "write-result"
                }
            }),
        )]),
    );

    let permission = request_matching(&waiting.host_requests, |payload| {
        matches!(payload, HostRequest::RequestPermission { .. })
    });
    let permission_request_id = match &permission.payload {
        HostRequest::RequestPermission {
            permission_request_id,
            input,
            ..
        } => {
            assert_eq!(input["_nolaExecution"]["sopRunId"], "sop-run-1");
            assert_eq!(input["_nolaExecution"]["sopStepId"], "write-result");
            permission_request_id.clone()
        }
        _ => unreachable!(),
    };

    let checkpoint = checkpoint_after(&core, &waiting);
    let pending = &checkpoint.state.execution.permissions.pending[0];
    assert_eq!(pending.arguments["path"], "/tmp/sop-lineage.txt");
    assert!(pending.arguments.get("_nolaExecution").is_none());
    assert_eq!(
        pending
            .invocation_context
            .as_ref()
            .expect("checkpoint lineage")
            .sop_step_id,
        "write-result"
    );
    assert_eq!(
        pending
            .invocation_context
            .as_ref()
            .expect("checkpoint lineage")
            .sop_run_id,
        "sop-run-1"
    );

    let allowed = core
        .submit(AgentSubmission::PermissionResponse {
            request_id: nola_core::agent::PermissionRequestId::from_string(permission_request_id),
            decision: PermissionUserDecision::AllowOnce,
            grant: None,
        })
        .expect("allow tool");
    let execute = request_matching(&allowed.host_requests, |payload| {
        matches!(payload, HostRequest::ExecuteTool { .. })
    });
    match execute.payload {
        HostRequest::ExecuteTool { input, .. } => {
            assert_eq!(input["_nolaExecution"]["sopRunId"], "sop-run-1");
            assert_eq!(input["_nolaExecution"]["sopStepId"], "write-result");
        }
        _ => unreachable!(),
    }
}

#[test]
fn cancelling_running_tool_keeps_cancelled_terminal_state_when_result_arrives_late() {
    let mut core = AgentCore::try_new(engineering_core_config()).expect("create Core");
    let started = core
        .submit(user_input(
            "cancel-running-tool-user",
            "write protected output",
            AgentInputDelivery::CurrentTurn,
        ))
        .expect("start turn");
    let model = model_request(&started.host_requests);
    let waiting = submit_response(
        &mut core,
        &model,
        1,
        empty_model_completion(vec![tool_call("cancel-running-call")]),
    );
    let permission_request_id = waiting
        .host_requests
        .iter()
        .find_map(|request| match &request.payload {
            HostRequest::RequestPermission {
                permission_request_id,
                ..
            } => Some(permission_request_id.clone()),
            _ => None,
        })
        .expect("permission request");
    let allowed = core
        .submit(AgentSubmission::PermissionResponse {
            request_id: nola_core::agent::PermissionRequestId::from_string(permission_request_id),
            decision: PermissionUserDecision::AllowOnce,
            grant: None,
        })
        .expect("allow tool");
    let tool = request_matching(&allowed.host_requests, |payload| {
        matches!(payload, HostRequest::ExecuteTool { .. })
    });
    assert!(allowed.snapshot.projection.parts.values().any(|part| {
        matches!(
            part,
            AgentPart::Tool(tool)
                if tool.call_id == "cancel-running-call"
                    && tool.state == nola_core::agent::parts::ToolPartState::Running
        )
    }));

    let interrupted = core
        .submit(AgentSubmission::Interrupt { goal_control: None, cancellation_id: "test-cancellation".into(),
            reason: Some("stop running tool".to_string()),
        })
        .expect("interrupt running tool");
    assert_eq!(
        interrupted.snapshot.turn_status,
        nola_core::agent::AgentTurnStatus::Cancelling
    );
    let cancelled = confirm_cancellation(&mut core, &interrupted, &tool.effect_id);
    assert!(cancelled.snapshot.projection.parts.values().any(|part| {
        matches!(
            part,
            AgentPart::Tool(tool)
                if tool.call_id == "cancel-running-call"
                    && tool.state == nola_core::agent::parts::ToolPartState::Cancelled
        )
    }));

    assert!(cancelled.snapshot.projection.parts.values().any(|part| {
        matches!(
            part,
            AgentPart::Tool(tool)
                if tool.call_id == "cancel-running-call"
                    && tool.state == nola_core::agent::parts::ToolPartState::Cancelled
        )
    }));
    assert!(core
        .submit(AgentSubmission::HostResponse {
            response: response_for(
                &tool,
                1,
                HostResponse::ToolCompleted {
                    call_id: "cancel-running-call".to_string(),
                    output: json!({"ok": true}),
                    outcome: nola_core::agent::host::HostToolOutcome::Succeeded,
                    activation: None,
                    full_output_ref: None,
                    artifacts: Vec::new(),
                },
            ),
        })
        .is_err());
}

#[test]
fn cancelling_active_compaction_ignores_late_summary_and_keeps_turn_cancelled() {
    let initial_items = (0..24)
        .map(|index| AgentItem::Message {
            id: ItemId::from_string(format!("cancel-context-{index}")),
            role: if index % 2 == 0 {
                AgentRole::User
            } else {
                AgentRole::Assistant
            },
            content: format!("historical message {index} {}", "x".repeat(4_000)),
        })
        .collect::<Vec<_>>();
    let mut config = core_config(initial_items);
    config.context_window = ContextWindowConfig {
        context_window_tokens: 128_000,
        reserved_output_tokens: 8_192,
        auto_compact_tokens: Some(120_000),
        compaction_target_tokens: Some(2_000),
        ..ContextWindowConfig::default()
    };
    let mut core = AgentCore::try_new(config).expect("create Core");
    let started = core
        .submit(user_input(
            "cancel-compaction-user",
            "continue",
            AgentInputDelivery::CurrentTurn,
        ))
        .expect("start turn");
    let model = model_request(&started.host_requests);
    let compacting = submit_response(
        &mut core,
        &model,
        1,
        HostResponse::Failed {
            error: HostError {
                code: "context_length_exceeded".to_string(),
                message: "too long".to_string(),
                retryable: false,
                http_status: None,
            },
        },
    );
    let compact = request_matching(&compacting.host_requests, |payload| {
        matches!(payload, HostRequest::Compact { .. })
    });
    let interrupted = core
        .submit(AgentSubmission::Interrupt { goal_control: None, cancellation_id: "test-cancellation".into(),
            reason: Some("stop compacting".to_string()),
        })
        .expect("interrupt compaction");
    assert_eq!(
        interrupted.snapshot.turn_status,
        nola_core::agent::AgentTurnStatus::Cancelling
    );

    let before_foreign = serde_json::to_value(core.checkpoint(CheckpointReason::Manual)).unwrap();
    let mut foreign = response_for(&compact, 1, HostResponse::CompactionCompleted {
        summary: "must not apply".into(), usage: json!({}),
    });
    foreign.request_id = "foreign-compaction-request".into();
    assert!(core.submit(AgentSubmission::HostResponse { response: foreign }).is_err());
    assert_eq!(serde_json::to_value(core.checkpoint(CheckpointReason::Manual)).unwrap(), before_foreign,
        "cancelled summary bypass must not bypass exact Host envelope admission");
    let late = submit_response(
        &mut core,
        &compact,
        1,
        HostResponse::CompactionCompleted {
            summary: "must not apply".to_string(),
            usage: serde_json::to_value(TokenUsage::default()).expect("usage"),
        },
    );
    assert!(late.event_envelopes.is_empty());
    assert_eq!(
        late.snapshot.turn_status,
        nola_core::agent::AgentTurnStatus::Cancelling
    );
    let cancelled = confirm_cancellation(&mut core, &interrupted, &compact.effect_id);
    assert!(matches!(
        cancelled.snapshot.turn_status,
        nola_core::agent::AgentTurnStatus::Cancelled
    ));
    assert!(!cancelled
        .host_requests
        .iter()
        .any(|request| matches!(request.payload, HostRequest::ModelStream { .. })));
}

#[test]
fn recorded_mac_compaction_host_cancellation_is_not_summary_failure() {
    let trace: serde_json::Value = serde_json::from_str(include_str!("fixtures/mac_compaction_20261010.json")).unwrap();
    let items = (0..24).map(|index| AgentItem::Message {
        id: ItemId::from_string(format!("recorded-source-{index}")),
        role: if index % 2 == 0 { AgentRole::User } else { AgentRole::Assistant },
        content: format!("Synthetic retained constraint {index}: {}", "x".repeat(4000)),
    }).collect::<Vec<_>>();
    let mut core = AgentCore::try_new(core_config(items)).unwrap();
    let before = core.checkpoint(CheckpointReason::Manual);
    let started = core.submit(AgentSubmission::Compact { operation_id:"recorded-mac-cancel".into() }).unwrap();
    let compact = request_matching(&started.host_requests, |payload| matches!(payload, HostRequest::Compact { .. }));
    let interrupted = core.submit(AgentSubmission::Interrupt {
        goal_control: None, cancellation_id: "recorded-user-stop".into(), reason: Some("Stop compaction".into()),
    }).unwrap();
    let outcome = submit_response(&mut core, &compact, 1, HostResponse::Failed {
        error: HostError {
            code: trace["attempts"][4]["failure_code"].as_str().unwrap().into(),
            message: "Compaction cancelled; original context retained".into(),
            retryable: false, http_status: None,
        },
    });
    assert_eq!(outcome.snapshot.turn_status, nola_core::agent::AgentTurnStatus::Cancelling);
    let confirmed = confirm_cancellation(&mut core, &interrupted, &compact.effect_id);
    assert_eq!(confirmed.snapshot.turn_status, nola_core::agent::AgentTurnStatus::Cancelled);
    assert!(confirmed.event_envelopes.iter().any(|event| matches!(&event.event,
        AgentEvent::TurnCancelled { .. })));
    // Manual maintenance has no admitted business requirement scope and must
    // not manufacture a conversational RunResult just to describe cancellation.
    assert!(!confirmed.event_envelopes.iter().any(|event| matches!(&event.event,
        AgentEvent::RunResultDecided { .. })));
    assert!(!outcome.snapshot.compaction_in_progress);
    assert!(!outcome.event_envelopes.iter().any(|event| matches!(event.event,
        AgentEvent::TurnFailed { .. } | AgentEvent::CompactionFailed { .. } | AgentEvent::CompactionCompleted { .. })));
    assert!(!outcome.host_requests.iter().any(|request| matches!(request.payload, HostRequest::ModelStream { .. } | HostRequest::Compact { .. })));
    let after = core.checkpoint(CheckpointReason::Manual);
    assert_eq!(after.state.execution.requirements.cancellation_id.as_deref(), Some("recorded-user-stop"));
    assert_eq!(after.state.conversation.model_context, before.state.conversation.model_context);
    assert_eq!(after.state.conversation.transcript, before.state.conversation.transcript);
    assert!(after.state.conversation.context_window.pending_compaction.is_none());
    assert_eq!(after.state.conversation.context_window.consecutive_compaction_failures, 0);
    assert_eq!(after.state.conversation.context_window.compaction_retry.failed_attempts, 0);
}

#[test]
fn compaction_host_stop_without_admitted_cancellation_cannot_forge_user_cancel() {
    let items = (0..24).map(|index| AgentItem::Message {
        id: ItemId::from_string(format!("host-stop-source-{index}")),
        role: if index % 2 == 0 { AgentRole::User } else { AgentRole::Assistant },
        content: "retained original context ".repeat(160),
    }).collect();
    let mut core = AgentCore::try_new(core_config(items)).unwrap();
    let before = core.checkpoint(CheckpointReason::Manual);
    let started = core.submit(AgentSubmission::Compact { operation_id: "host-stop-without-user".into() }).unwrap();
    let compact = request_matching(&started.host_requests, |payload| matches!(payload, HostRequest::Compact { .. }));
    let failed = submit_response(&mut core, &compact, 1, HostResponse::Failed { error: HostError {
        code: "cancelled".into(), message: "Host transport stopped".into(), retryable: false, http_status: None,
    }});
    assert_eq!(failed.snapshot.turn_status, nola_core::agent::AgentTurnStatus::Failed);
    assert!(!failed.event_envelopes.iter().any(|event| matches!(&event.event,
        AgentEvent::TurnCancelled { .. } | AgentEvent::RunResultDecided { decision: nola_domain::core_requirements::CoreRunResultDecision {
            value: nola_domain::run_result::RunResultValue::Cancelled { .. }, .. }, .. })));
    let after = core.checkpoint(CheckpointReason::Manual);
    assert!(after.state.execution.requirements.cancellation_id.is_none());
    assert_eq!(after.state.conversation.model_context, before.state.conversation.model_context);
    assert_eq!(after.state.conversation.transcript, before.state.conversation.transcript);
}

#[test]
fn incremental_compaction_preserves_previous_block_and_appends_new_block() {
    let previous = AgentItem::Compaction {
        id: ItemId::from_string("previous-summary".into()),
        summary: "User requires offline operation; prior validation succeeded.".into(),
        replaced_item_count: 2,
        source_item_ids: vec![ItemId::from_string("original-source".into())],
        source_fingerprint: "prior-fingerprint".into(),
        original_token_count: 4000,
    };
    let mut items = vec![previous.clone()];
    items.extend((0..24).map(|index| AgentItem::Message {
        id: ItemId::from_string(format!("incremental-{index}")),
        role: if index % 2 == 0 { AgentRole::User } else { AgentRole::Assistant },
        content: "new evidence ".repeat(400),
    }));
    let config = core_config(items);
    let mut core = AgentCore::try_new(config.clone()).unwrap();
    let before = core.checkpoint(CheckpointReason::Manual);
    let started = core.submit(AgentSubmission::Compact { operation_id: "incremental-compact".into() }).unwrap();
    let compact = request_matching(&started.host_requests, |payload| matches!(payload, HostRequest::Compact { .. }));
    let pending_checkpoint = core.checkpoint(CheckpointReason::Manual);
    let pending = pending_checkpoint.state.conversation.context_window.pending_compaction.as_ref().unwrap();
    assert!(!pending.source_item_ids.contains(previous.id()));
    let retained = pending.retained_items(&pending_checkpoint.state.conversation.model_context)
        .expect("resolve retained original text against the frozen working-set witness");
    assert!(retained.contains(&&previous));
    core = AgentCore::restore(config, core.checkpoint(CheckpointReason::Manual)).unwrap();
    submit_response(&mut core, &compact, 1, HostResponse::CompactionCompleted {
        summary: "New evidence recorded; continue the current task.".into(),
        usage: serde_json::json!({}),
    });
    let after = core.checkpoint(CheckpointReason::Manual);
    let context = &after.state.conversation.model_context;
    assert_eq!(context.first(), Some(&previous));
    assert!(matches!(&context[1], AgentItem::Compaction { source_item_ids, .. }
        if !source_item_ids.contains(previous.id())));
    assert!(after.state.conversation.transcript.starts_with(&before.state.conversation.transcript));
    let persisted = serde_json::to_vec(&after.state.conversation.model_context).unwrap();
    let restored: Vec<AgentItem> = serde_json::from_slice(&persisted).unwrap();
    assert_eq!(&restored, context);
    let retained_blocks = context.iter().filter(|item| matches!(item, AgentItem::Compaction { .. })).cloned().collect::<Vec<_>>();
    for index in 0..3 {
        let started = core.submit(user_input(&format!("after-compact-{index}"),
            &"Additional user constraint. ".repeat(200), AgentInputDelivery::CurrentTurn)).unwrap();
        let model = model_request(&started.host_requests);
        submit_response(&mut core, &model, 1, model_delta(&model, "New work remains pending."));
        submit_response(&mut core, &model, 2, empty_model_completion(Vec::new()));
    }
    let started = core.submit(AgentSubmission::Compact { operation_id: "incremental-compact-again".into() }).unwrap();
    let compact = request_matching(&started.host_requests, |payload| matches!(payload, HostRequest::Compact { .. }));
    let pending = core.checkpoint(CheckpointReason::Manual).state.conversation.context_window.pending_compaction.unwrap();
    assert!(retained_blocks.iter().all(|item| !pending.source_item_ids.contains(item.id())));
    submit_response(&mut core, &compact, 1, HostResponse::CompactionCompleted {
        summary: "Additional constraints recorded; new work remains pending.".into(), usage: serde_json::json!({}),
    });
    let final_checkpoint = core.checkpoint(CheckpointReason::Manual);
    let blocks = final_checkpoint.state.conversation.model_context.iter()
        .filter(|item| matches!(item, AgentItem::Compaction { .. })).collect::<Vec<_>>();
    assert_eq!(blocks.len(), 3);
    assert_eq!(blocks[0], &retained_blocks[0]);
    assert_eq!(blocks[1], &retained_blocks[1]);
}

#[test]
fn provider_overflow_tightens_an_overstated_target_without_rewriting_transcript() {
    let original =
        "START-CANONICAL\n".to_string() + &"不可丢失的中间原文-".repeat(20_000) + "\nEND-CANONICAL";
    let mut config = core_config(Vec::new());
    config.context_window = ContextWindowConfig {
        context_window_tokens: 1_000_000,
        reserved_output_tokens: 8_192,
        auto_compact_tokens: Some(900_000),
        // Deliberately overstate the configured target. The provider overflow
        // is authoritative evidence that recovery must tighten from the
        // rejected working set instead of reusing this value.
        compaction_target_tokens: Some(900_000),
        ..ContextWindowConfig::default()
    };
    let mut core = AgentCore::try_new(config).expect("create Core");
    let started = core
        .submit(user_input(
            "oversized-canonical-user",
            &original,
            AgentInputDelivery::CurrentTurn,
        ))
        .expect("start oversized turn");
    let first_model = model_request(&started.host_requests);

    let recovered = submit_response(
        &mut core,
        &first_model,
        1,
        HostResponse::Failed {
            error: HostError {
                code: "context_length_exceeded".to_string(),
                message: "provider rejected one oversized item".to_string(),
                retryable: false,
                http_status: None,
            },
        },
    );

    assert!(recovered
        .host_requests
        .iter()
        .all(|request| !matches!(request.payload, HostRequest::Compact { .. })));
    let retried = model_request(&recovered.host_requests);
    let messages = match retried.payload {
        HostRequest::ModelStream { messages, .. } => messages,
        _ => unreachable!(),
    };
    let bounded = messages
        .iter()
        .filter_map(|message| message.get("content").and_then(Value::as_str))
        .find(|content| content.contains("Exact canonical source item: oversized-canonical-user"))
        .expect("bounded source projection reaches retry");
    assert!(bounded.contains("START-CANONICAL"));
    assert!(bounded.contains("END-CANONICAL"));
    assert!(bounded.chars().count() < original.chars().count());

    let checkpoint = core.checkpoint(CheckpointReason::Manual);
    let compaction_id = checkpoint
        .state
        .conversation
        .transcript
        .iter()
        .find_map(|item| match item {
            AgentItem::Compaction { id, .. } => Some(id.as_str()),
            _ => None,
        })
        .expect("compaction item identity");
    assert_eq!(
        checkpoint
            .state
            .item_ownership
            .get(compaction_id)
            .map(|ownership| ownership.message_id.as_str()),
        Some("submission-oversized-canonical-user:assistant"),
        "the bounded window must retain the compaction Part under its active assistant message"
    );
    assert!(checkpoint
        .state
        .conversation
        .transcript
        .iter()
        .any(|item| matches!(
            item,
            AgentItem::Message { id, role: AgentRole::User, content }
                if id.as_str() == "oversized-canonical-user" && content == &original
        )));
    assert!(checkpoint
        .state
        .conversation
        .model_context
        .iter()
        .any(|item| matches!(
            item,
            AgentItem::Compaction { source_item_ids, source_fingerprint, .. }
                if source_item_ids.iter().any(|id| id.as_str() == "oversized-canonical-user")
                    && !source_fingerprint.is_empty()
        )));
    assert!(recovered.event_envelopes.iter().any(|event| matches!(
        event.event,
        AgentEvent::CompactionCompleted {
            replaced_item_count: 1,
            ..
        }
    )));
}

#[test]
fn invalid_compaction_response_does_not_consume_plan_or_response_sequence() {
    let oversized = "x".repeat(1_000_000);
    for (summary, usage, expected_error) in [
        (" \n\t", json!({}), "compaction summary must not be empty"),
        (
            "valid summary",
            json!({"input_tokens":"invalid"}),
            "invalid compaction usage",
        ),
        (
            oversized.as_str(),
            json!({}),
            "exceeds the planned token budget",
        ),
    ] {
        let items = (0..24)
            .map(|index| AgentItem::Message {
                id: ItemId::from_string(format!("history-{index}")),
                role: if index % 2 == 0 {
                    AgentRole::User
                } else {
                    AgentRole::Assistant
                },
                content: format!("Historical constraint {index} {}", "x".repeat(160)),
            })
            .collect::<Vec<_>>();
        let mut core = AgentCore::try_new(core_config(items)).unwrap();
        let requested = core
            .submit(AgentSubmission::Compact {
                operation_id: "invalid-summary-recovery".into(),
            })
            .unwrap();
        let compact = request_matching(&requested.host_requests, |payload| {
            matches!(payload, HostRequest::Compact { .. })
        });
        let result = core.submit(AgentSubmission::HostResponse {
            response: terminal_host_response(
                &compact,
                HostResponse::CompactionCompleted {
                    summary: summary.into(),
                    usage,
                },
            ),
        });
        assert!(result.err().unwrap().contains(expected_error));
        // Reuse the same effect and first response sequence. Validation must
        // not make a failed payload authoritative or discard retained history.
        let completed = submit_response(
            &mut core,
            &compact,
            1,
            HostResponse::CompactionCompleted {
                summary: "Recovered summary retaining the historical constraints".into(),
                usage: serde_json::to_value(TokenUsage::default()).unwrap(),
            },
        );
        assert_eq!(completed.snapshot.history_item_count, 25);
        assert!(completed.snapshot.model_context_item_count < 24);
        assert!(completed
            .event_envelopes
            .iter()
            .any(|event| { matches!(event.event, AgentEvent::CompactionCompleted { .. }) }));
    }
}

#[test]
fn manual_compaction_budget_uses_current_working_set_not_previous_provider_usage() {
    let initial_items = (0..24)
        .map(|index| AgentItem::Message {
            id: ItemId::from_string(format!("budget-history-{index}")),
            role: if index % 2 == 0 {
                AgentRole::User
            } else {
                AgentRole::Assistant
            },
            content: "historical constraints and decisions ".repeat(20),
        })
        .collect::<Vec<_>>();
    let config = core_config(initial_items);
    let core = AgentCore::try_new(config.clone()).unwrap();
    let mut checkpoint = core.checkpoint(CheckpointReason::Manual);
    checkpoint
        .state
        .conversation
        .context_window
        .record_provider_usage(&TokenUsage {
            input_tokens: Some(1),
            output_tokens: Some(1),
            total_tokens: Some(2),
            ..TokenUsage::default()
        });
    let mut core = AgentCore::restore(config, checkpoint).unwrap();
    let requested = core
        .submit(AgentSubmission::Compact {
            operation_id: "fresh-working-set-budget".into(),
        })
        .unwrap();
    let compact = request_matching(&requested.host_requests, |payload| {
        matches!(payload, HostRequest::Compact { .. })
    });
    let HostRequest::Compact { target_tokens, .. } = &compact.payload else {
        unreachable!()
    };
    assert!(
        *target_tokens > 100,
        "previous tiny provider usage must not dictate the current archive budget"
    );
    let completed = submit_response(
        &mut core,
        &compact,
        1,
        HostResponse::CompactionCompleted {
            summary: "Preserve historical constraints and decisions.".into(),
            usage: serde_json::to_value(TokenUsage::default()).unwrap(),
        },
    );
    assert_eq!(completed.snapshot.history_item_count, 25);
    assert!(completed.snapshot.model_context_item_count < 24);
}

#[test]
fn manual_compaction_replaces_model_context_but_preserves_transcript() {
    let initial_items = (0..24)
        .map(|index| AgentItem::Message {
            id: ItemId::from_string(format!("history-{index}")),
            role: if index % 2 == 0 {
                AgentRole::User
            } else {
                AgentRole::Assistant
            },
            content: format!("historical message {index} {}", "x".repeat(160)),
        })
        .collect::<Vec<_>>();
    let original_count = initial_items.len();
    let mut core = AgentCore::try_new(core_config(initial_items)).expect("create Core");

    let requested = core
        .submit(AgentSubmission::Compact {
            operation_id: "manual-compact-1".to_string(),
        })
        .expect("request compaction");
    let compact = request_matching(&requested.host_requests, |payload| {
        matches!(payload, HostRequest::Compact { .. })
    });
    let completed = submit_response(
        &mut core,
        &compact,
        1,
        HostResponse::CompactionCompleted {
            summary: "durable summary".to_string(),
            usage: serde_json::to_value(TokenUsage {
                input_tokens: Some(1_200),
                output_tokens: Some(80),
                total_tokens: Some(1_280),
                ..TokenUsage::default()
            })
            .expect("serialize usage"),
        },
    );

    assert_eq!(completed.snapshot.history_item_count, original_count + 1);
    assert!(completed.snapshot.model_context_item_count > 1);
    assert!(completed.snapshot.model_context_item_count < original_count);
    assert!(completed
        .event_envelopes
        .iter()
        .any(|envelope| matches!(envelope.event, AgentEvent::CompactionCompleted { .. })));
    let compacted_trace = completed
        .trace_events
        .iter()
        .find(|trace| trace.kind == nola_core::agent_trace::AgentTraceKind::ContextCompacted)
        .expect("context compaction trace");
    assert_eq!(
        compacted_trace.payload.summary.as_deref(),
        Some("durable summary")
    );
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

#[test]
fn manual_compaction_cannot_replace_an_inflight_model_effect() {
    let mut core = AgentCore::try_new(core_config(vec![])).unwrap();
    let started = core.submit(user_input("compact-busy-model", "hello",
        AgentInputDelivery::CurrentTurn)).unwrap();
    let model = model_request(&started.host_requests);
    assert_manual_compaction_busy_without_mutation(&mut core, "compact-during-model");
    // The original response remains admissible; rejection did not consume its
    // effect, change its sequence, or put the Turn into Compacting.
    let continued = submit_response(&mut core, &model, 1, empty_model_completion(vec![]));
    assert!(!continued.host_requests.iter().any(|request|
        matches!(request.payload, HostRequest::Compact { .. })));
}

#[test]
fn manual_compaction_preserves_permission_wait_and_cancellation_drain() {
    let (mut core, ask) = suspended_custom_fixture();
    assert_manual_compaction_busy_without_mutation(&mut core, "compact-during-permission");
    let continued = core.submit(custom_response(&ask, "keep waiting for my authorization")).unwrap();
    assert!(continued.host_requests.iter().any(|request|
        matches!(request.payload, HostRequest::RequestPermission { .. })));
    core.submit(AgentSubmission::Interrupt {
        goal_control: None, cancellation_id: "compact-busy-cancel".into(), reason: Some("stop".into()),
    }).unwrap();
    assert_eq!(core.snapshot().turn_status, nola_core::agent::AgentTurnStatus::Cancelling);
    assert_manual_compaction_busy_without_mutation(&mut core, "compact-during-cancellation");
}

#[test]
fn manual_compaction_requires_effect_quiescence_even_with_terminal_turn_label() {
    let config = core_config(vec![]);
    let mut core = AgentCore::try_new(config.clone()).unwrap();
    core.submit(user_input("compact-unsettled-effect", "hello",
        AgentInputDelivery::CurrentTurn)).unwrap();
    let mut checkpoint = core.checkpoint(CheckpointReason::Manual);
    // A terminal label alone must not authorize maintenance while an original
    // Host effect remains unacknowledged in durable recovery state.
    checkpoint.state.execution.active_turn.as_mut().unwrap().status =
        nola_core::agent::AgentTurnStatus::Failed;
    let mut restored = AgentCore::restore(config, checkpoint).unwrap();
    assert_manual_compaction_busy_without_mutation(&mut restored, "compact-unsettled-recovery");
}

#[test]
fn manual_compaction_after_a_completed_turn_does_not_restart_the_model() {
    let initial_items = (0..24)
        .map(|index| AgentItem::Message {
            id: ItemId::from_string(format!("history-{index}")),
            role: if index % 2 == 0 {
                AgentRole::User
            } else {
                AgentRole::Assistant
            },
            content: format!("historical message {index} {}", "x".repeat(160)),
        })
        .collect::<Vec<_>>();
    let mut core = AgentCore::try_new(core_config(initial_items)).expect("create Core");

    let started = core
        .submit(user_input(
            "current-user",
            "finish this turn",
            AgentInputDelivery::CurrentTurn,
        ))
        .expect("start turn");
    let model = model_request(&started.host_requests);
    submit_response(&mut core, &model, 1, model_delta(&model, "done"));
    let completed_turn = submit_response(&mut core, &model, 2, empty_model_completion(Vec::new()));
    assert_eq!(
        completed_turn.snapshot.turn_status,
        nola_core::agent::AgentTurnStatus::Completed
    );

    let requested = core
        .submit(AgentSubmission::Compact {
            operation_id: "manual-compact-2".to_string(),
        })
        .expect("request compaction");
    let compact = request_matching(&requested.host_requests, |payload| {
        matches!(payload, HostRequest::Compact { .. })
    });
    assert_eq!(
        requested
            .snapshot
            .active_turn_id
            .as_ref()
            .map(|turn_id| turn_id.0.as_str()),
        Some("manual-compact-2")
    );
    assert_eq!(
        requested.snapshot.turn_status,
        nola_core::agent::AgentTurnStatus::Compacting
    );

    let completed = submit_response(
        &mut core,
        &compact,
        1,
        HostResponse::CompactionCompleted {
            summary: "durable summary".to_string(),
            usage: serde_json::to_value(TokenUsage::default()).expect("serialize usage"),
        },
    );

    assert!(completed
        .event_envelopes
        .iter()
        .any(|envelope| matches!(envelope.event, AgentEvent::CompactionCompleted { .. })));
    assert!(
        !completed
            .host_requests
            .iter()
            .any(|request| matches!(request.payload, HostRequest::ModelStream { .. })),
        "manual compaction restarted a completed turn"
    );
    assert_eq!(
        completed.snapshot.turn_status,
        nola_core::agent::AgentTurnStatus::Completed
    );
}

#[test]
fn automatic_compaction_skips_an_oversized_first_round_without_history() {
    let mut config = core_config(Vec::new());
    config.context_window = ContextWindowConfig {
        // Exceed the compaction trigger, not the mandatory-input ceiling:
        // current tool contracts must fit for a model request to be legal.
        context_window_tokens: 16_000,
        reserved_output_tokens: 256,
        auto_compact_tokens: Some(300),
        compaction_target_tokens: Some(180),
        ..ContextWindowConfig::default()
    };
    let mut core = AgentCore::try_new(config).expect("create Core");

    let requested = core
        .submit(AgentSubmission::UserInput {
            items: vec![AgentItem::user_message(
                ItemId::from_string("oversized-current-user".to_string()),
                "analyze the supplied context",
            )],
            turn_context_items: vec![AgentItem::Message {
                id: ItemId::from_string("large-turn-context".to_string()),
                role: AgentRole::System,
                content: "x".repeat(2_000),
            }],
            turn_context_platform: ContextPlatform::Unknown,
            turn_context_components: Vec::new(),
            submission_id: Some("submission-oversized-current-user".to_string()),
            submitted_at_epoch_ms: None,
            delivery: AgentInputDelivery::CurrentTurn,
        })
        .expect("start oversized first turn");

    assert!(
        requested
            .host_requests
            .iter()
            .any(|request| matches!(request.payload, HostRequest::ModelStream { .. })),
        "expected model request after skipped compaction, got {requested:#?}"
    );
    assert!(!requested
        .host_requests
        .iter()
        .any(|request| matches!(request.payload, HostRequest::Compact { .. })));
    assert!(!requested.event_envelopes.iter().any(|envelope| matches!(
        envelope.event,
        AgentEvent::CompactionRequested { .. } | AgentEvent::CompactionFailed { .. }
    )));
}

#[test]
fn automatic_compaction_does_not_spend_a_model_request_on_a_tiny_archive() {
    let mut config = core_config(vec![
        AgentItem::user_message(ItemId::from_string("old-user".into()), "你好"),
        AgentItem::Message {
            id: ItemId::from_string("old-answer".into()),
            role: AgentRole::Assistant,
            content: "你好！我是 Nola。".into(),
        },
    ]);
    config.context_window = ContextWindowConfig {
        context_window_tokens: 128_000,
        reserved_output_tokens: 8_000,
        auto_compact_tokens: Some(300),
        compaction_target_tokens: Some(180),
        ..ContextWindowConfig::default()
    };
    let mut core = AgentCore::try_new(config).unwrap();
    let requested = core
        .submit(user_input(
            "long-current-user",
            &"Keep this exact constraint. ".repeat(100),
            AgentInputDelivery::CurrentTurn,
        ))
        .unwrap();
    assert!(requested
        .host_requests
        .iter()
        .any(|request| matches!(request.payload, HostRequest::ModelStream { .. })));
    assert!(!requested
        .host_requests
        .iter()
        .any(|request| matches!(request.payload, HostRequest::Compact { .. })));
    assert!(
        !core
            .checkpoint(CheckpointReason::Manual)
            .state
            .conversation
            .context_window
            .compaction_in_progress
    );
}

#[test]
fn optional_compaction_skips_an_archive_that_cannot_relieve_fixed_pressure() {
    let items = vec![
        AgentItem::Message { id: ItemId::from_string("fixed".into()), role: AgentRole::System,
            content: "Mandatory instruction. ".repeat(300) },
        AgentItem::user_message(ItemId::from_string("prior-user".into()), "Inspect the existing file."),
        AgentItem::Message { id: ItemId::from_string("prior-answer".into()), role: AgentRole::Assistant,
            content: "Previously observed evidence. ".repeat(40) },
    ];
    let mut config = core_config(items.clone());
    config.context_window = ContextWindowConfig {
        context_window_tokens: 128_000, reserved_output_tokens: 8_000,
        auto_compact_tokens: Some(1_000), compaction_target_tokens: Some(600),
        ..ContextWindowConfig::default()
    };
    let mut core = AgentCore::try_new(config).unwrap();
    let requested = core.submit(user_input("current-user", "Continue from the observed evidence.", AgentInputDelivery::CurrentTurn)).unwrap();
    assert!(requested.host_requests.iter().any(|request| matches!(request.payload, HostRequest::ModelStream { .. })));
    assert!(!requested.host_requests.iter().any(|request| matches!(request.payload, HostRequest::Compact { .. })));
    let state = core.checkpoint(CheckpointReason::Manual).state;
    assert!(state.conversation.model_context.starts_with(&items));
    assert!(!state.conversation.context_window.compaction_in_progress);
    assert_eq!(state.execution.active_turn.unwrap().automatic_compaction_count, 1);
    assert!(!requested.event_envelopes.iter().any(|event| matches!(event.event, AgentEvent::CompactionRequested { .. })));
}

#[test]
fn optional_compaction_never_defers_a_hard_request_overflow() {
    let mut config = core_config(vec![AgentItem::Message {
        id: ItemId::from_string("mandatory".into()), role: AgentRole::System,
        content: "Fixed instruction. ".repeat(2_000),
    }]);
    config.context_window = ContextWindowConfig {
        context_window_tokens: 16_000, reserved_output_tokens: 1_000,
        prompt_token_limit: Some(2_000), auto_compact_tokens: Some(1_000), compaction_target_tokens: Some(600),
        ..ContextWindowConfig::default()
    };
    let mut core = AgentCore::try_new(config).unwrap();
    let requested = core.submit(user_input("hard-user", "Continue.", AgentInputDelivery::CurrentTurn)).unwrap();
    assert!(!requested.host_requests.iter().any(|request| matches!(request.payload, HostRequest::ModelStream { .. })),
        "An optional optimization must never bypass the complete request hard limit");
}

#[test]
fn optional_compaction_failure_retains_completed_tools_and_resumes_once() {
    let mut items = Vec::new();
    for index in 0..12 {
        let call_id = CallId::from_string(format!("observed-call-{index}"));
        items.push(AgentItem::ToolCall { id: ItemId::from_string(format!("observed-call-item-{index}")),
            call_id: call_id.clone(), name: "read_file".into(), arguments: json!({"path":format!("src/{index}.rs")}) });
        items.push(AgentItem::ToolResult { id: ItemId::from_string(format!("observed-result-{index}")),
            call_id, name: "read_file".into(), output: json!({"evidence":format!("retained evidence {index} ").repeat(600)}),
            is_error: false, full_output_ref: Some(format!("blob://observed/{index}")), artifacts: Vec::new() });
    }
    let mut config = core_config(items.clone());
    config.context_window = ContextWindowConfig {
        context_window_tokens: 256_000, reserved_output_tokens: 8_000,
        auto_compact_tokens: Some(20_000), compaction_target_tokens: Some(12_000),
        ..ContextWindowConfig::default()
    };
    let mut core = AgentCore::try_new(config).unwrap();
    let started = core.submit(user_input("summary-user", "Explain the completed read results.", AgentInputDelivery::CurrentTurn)).unwrap();
    let compact = request_matching(&started.host_requests, |payload| matches!(payload, HostRequest::Compact { .. }));
    let before = core.checkpoint(CheckpointReason::Manual);
    let resumed = submit_response(&mut core, &compact, 1, HostResponse::Failed { error: HostError {
        code: "compaction_summary_budget_exceeded".into(), message: "Summary exceeds its admitted budget".into(),
        retryable: false, http_status: None,
    }});
    assert!(resumed.host_requests.iter().any(|request| matches!(request.payload, HostRequest::ModelStream { .. })));
    assert_eq!(resumed.host_requests.len(), 1, "Only the model resumes; completed tool effects are never replayed");
    if let HostRequest::ModelStream { messages, .. } = &resumed.host_requests[0].payload {
        assert!(serde_json::to_string(messages).unwrap().contains("retained evidence 0"),
            "The resumed model must receive the original oldest tool result, not the rejected summary");
    }
    assert_ne!(resumed.snapshot.turn_status, nola_core::agent::AgentTurnStatus::Compacting);
    assert!(resumed.event_envelopes.iter().any(|event| matches!(&event.event, AgentEvent::CompactionFailed { error } if error == "Summary exceeds its admitted budget")));
    assert!(!resumed.event_envelopes.iter().any(|event| matches!(event.event, AgentEvent::TurnFailed { .. })));
    let after = core.checkpoint(CheckpointReason::Manual);
    assert_eq!(after.state.conversation.model_context, before.state.conversation.model_context);
    assert!(after.state.conversation.model_context.starts_with(&items));
    assert!(!after.state.conversation.context_window.compaction_in_progress);
    assert_eq!(after.state.execution.active_turn.unwrap().automatic_compaction_count, 1);
}

#[test]
fn single_user_turn_compacts_completed_tool_batches_and_restores_exact_tail() {
    let mut items = vec![AgentItem::user_message(
        ItemId::from_string("active-request".into()),
        "审计项目，不修改任何文件。",
    )];
    for index in 0..20 {
        let call_id = CallId::from_string(format!("read-{index}"));
        items.push(AgentItem::ToolCall {
            id: ItemId::from_string(format!("call-{index}")),
            call_id: call_id.clone(),
            name: "read_file".into(),
            arguments: json!({"path": format!("src/{index}.rs")}),
        });
        items.push(AgentItem::ToolResult {
            id: ItemId::from_string(format!("result-{index}")),
            call_id,
            name: "read_file".into(),
            output: json!({"content": "evidence".repeat(200)}),
            is_error: false,
            full_output_ref: Some(format!("blob://read/{index}")),
            artifacts: Vec::new(),
        });
    }
    let mut config = core_config(items.clone());
    config.context_window.compaction_target_tokens = Some(1_200);
    let mut core = AgentCore::try_new(config.clone()).unwrap();
    let before = core.checkpoint(CheckpointReason::Manual);
    let requested = core
        .submit(AgentSubmission::Compact {
            operation_id: "long-round".into(),
        })
        .unwrap();
    let compact = request_matching(&requested.host_requests, |request| {
        matches!(request, HostRequest::Compact { .. })
    });
    let pending = core.checkpoint(CheckpointReason::Manual);
    let plan = pending
        .state
        .conversation
        .context_window
        .pending_compaction
        .as_ref()
        .unwrap();
    assert!(plan.source_item_ids.len() >= 20);
    let expected_tail: Vec<AgentItem> = plan
        .retained_items(&pending.state.conversation.model_context)
        .expect("retained IDs must resolve to the original canonical context")
        .into_iter().cloned().collect();
    assert!(expected_tail.contains(&items[0]));
    assert!(expected_tail.contains(items.last().unwrap()));
    assert!(!expected_tail.contains(&items[2]));
    submit_response(&mut core, &compact, 1, HostResponse::CompactionCompleted {
        summary: "Earlier source files were inspected read-only; no files were modified. Continue the audit from the retained recent results.".into(),
        usage: serde_json::to_value(TokenUsage::default()).unwrap(),
    });
    let completed = core.checkpoint(CheckpointReason::Manual);
    assert!(completed
        .state
        .conversation
        .transcript
        .starts_with(&before.state.conversation.transcript));
    assert_eq!(
        &completed.state.conversation.model_context[1..],
        expected_tail.as_slice()
    );
    let restored = AgentCore::restore(config, completed).unwrap();
    assert_eq!(
        &restored
            .checkpoint(CheckpointReason::Manual)
            .state
            .conversation
            .model_context[1..],
        expected_tail.as_slice()
    );
}

#[test]
fn automatic_compaction_does_not_reenter_when_host_keeps_an_oversized_context() {
    let mut initial_items = vec![AgentItem::Message {
        id: ItemId::from_string("history-user".to_string()),
        role: AgentRole::User,
        content: "inspect the repository".to_string(),
    }];
    for index in 0..12 {
        let call_id = CallId::from_string(format!("history-call-{index}"));
        initial_items.push(AgentItem::ToolCall {
            id: ItemId::from_string(format!("history-tool-call-{index}")),
            call_id: call_id.clone(),
            name: "read_file".to_string(),
            arguments: json!({"path": format!("src/{index}.rs")}),
        });
        initial_items.push(AgentItem::ToolResult {
            id: ItemId::from_string(format!("history-tool-result-{index}")),
            call_id,
            name: "read_file".to_string(),
            output: json!({"content": "x".repeat(4_000)}),
            is_error: false,
            full_output_ref: None,
            artifacts: Vec::new(),
        });
    }
    let mut config = core_config(initial_items);
    config.context_window = ContextWindowConfig {
        // Use an attainable trigger above the mandatory prefix so this
        // exercises a real compaction, not an infeasible optional maintenance.
        context_window_tokens: 128_000,
        reserved_output_tokens: 256,
        auto_compact_tokens: Some(10_000),
        compaction_target_tokens: Some(6_000),
        ..ContextWindowConfig::default()
    };
    let mut core = AgentCore::try_new(config).expect("create Core");

    let requested = core
        .submit(user_input(
            "current-user",
            "continue",
            AgentInputDelivery::CurrentTurn,
        ))
        .expect("start turn");
    let compact = request_matching(&requested.host_requests, |payload| {
        matches!(payload, HostRequest::Compact { .. })
    });
    let completed = submit_response(
        &mut core,
        &compact,
        1,
        HostResponse::CompactionCompleted {
            summary: "summary that did not reduce the retained context".to_string(),
            usage: serde_json::to_value(TokenUsage {
                input_tokens: Some(1_800),
                output_tokens: Some(80),
                total_tokens: Some(1_880),
                ..TokenUsage::default()
            })
            .expect("serialize usage"),
        },
    );

    assert!(completed
        .host_requests
        .iter()
        .any(|request| matches!(request.payload, HostRequest::ModelStream { .. })));
    assert!(!completed
        .host_requests
        .iter()
        .any(|request| matches!(request.payload, HostRequest::Compact { .. })));
    assert_eq!(completed.snapshot.compaction_in_progress, false);
}

#[test]
fn restoring_a_checkpoint_replays_the_same_next_core_event() {
    let config = core_config(Vec::new());
    let mut original = AgentCore::try_new(config.clone()).expect("create Core");
    let started = original
        .submit(user_input(
            "user-1",
            "hello",
            AgentInputDelivery::CurrentTurn,
        ))
        .expect("start turn");
    let model = model_request(&started.host_requests);
    let checkpoint = checkpoint_after(&original, &started);
    let mut restored = AgentCore::restore(config, checkpoint).expect("restore Core");

    let payload = HostResponse::ModelDelta {
        channel: ModelDeltaChannel::Text,
        part_id: match &model.payload {
            HostRequest::ModelStream { text_part_id, .. } => text_part_id.clone(),
            _ => unreachable!(),
        },
        delta: "same".to_string(),
    };
    let original_transition = original
        .submit(AgentSubmission::HostResponse {
            response: response_for(&model, 1, payload.clone()),
        })
        .expect("advance original");
    let restored_transition = restored
        .submit(AgentSubmission::HostResponse {
            response: response_for(&model, 1, payload),
        })
        .expect("advance restored");

    assert_eq!(
        serde_json::to_value(&original_transition.event_envelopes).expect("events"),
        serde_json::to_value(&restored_transition.event_envelopes).expect("events")
    );
    let mut original_traces = original_transition.trace_events.clone();
    let mut restored_traces = restored_transition.trace_events.clone();
    for trace in original_traces.iter_mut().chain(restored_traces.iter_mut()) {
        // Wall-clock observation time is intentionally assigned at projection
        // time. Deterministic replay guarantees the trace identity, sequence,
        // status, and payload, not that two executions cross the clock at the
        // same millisecond.
        trace.emitted_at.clear();
    }
    assert_eq!(original_traces, restored_traces);
    assert_eq!(
        original_transition.snapshot.projection,
        restored_transition.snapshot.projection
    );
}

#[test]
fn idle_checkpoint_accepts_new_turn_tool_catalog_without_losing_tool_history() {
    const TOOL_MARKER: &str = "PERSISTED_TOOL_RESULT_7319";
    let call_id = CallId::from_string("durable-read".to_string());
    let initial_items = vec![
        AgentItem::user_message(
            ItemId::from_string("history-user".to_string()),
            "read durable-fact.txt",
        ),
        AgentItem::ToolCall {
            id: ItemId::from_string("history-tool-call".to_string()),
            call_id: call_id.clone(),
            name: "read_file".to_string(),
            arguments: json!({"path": "durable-fact.txt"}),
        },
        AgentItem::ToolResult {
            id: ItemId::from_string("history-tool-result".to_string()),
            call_id,
            name: "read_file".to_string(),
            output: json!({"text": TOOL_MARKER}),
            is_error: false,
            full_output_ref: None,
            artifacts: Vec::new(),
        },
        AgentItem::assistant_message(
            ItemId::from_string("history-assistant".to_string()),
            "READ_COMPLETE",
        ),
    ];
    let original_config = core_config(initial_items);
    let mut core = AgentCore::try_new(original_config.clone()).expect("create Core");
    let started = core
        .submit(user_input(
            "current-user",
            "finish this turn",
            AgentInputDelivery::CurrentTurn,
        ))
        .expect("start turn");
    let model = model_request(&started.host_requests);
    let completed = submit_response(
        &mut core,
        &model,
        1,
        visible_model_completion("completed-answer", "Turn completed."),
    );
    let checkpoint = checkpoint_after(&core, &completed);
    let history_item_count = completed.snapshot.history_item_count;

    let mut no_tools_config = original_config;
    no_tools_config.tools.clear();
    assert!(
        checkpoint
            .state
            .configuration
            .matches_runtime_except_tools(&no_tools_config),
        "non-tool session configuration changed"
    );
    assert!(
        checkpoint
            .state
            .execution
            .active_turn
            .as_ref()
            .is_none_or(|turn| turn.status.is_terminal()),
        "checkpoint retained a non-terminal turn"
    );
    assert_eq!(checkpoint.state.host.pending_effects.len(), 0);
    let mut restored = AgentCore::restore(no_tools_config, checkpoint).expect("restore idle Core");
    assert_eq!(restored.snapshot().tool_count, 0);
    assert_eq!(restored.snapshot().history_item_count, history_item_count);

    let next = restored
        .submit(user_input(
            "recall-user",
            "return the previously read value without tools",
            AgentInputDelivery::CurrentTurn,
        ))
        .expect("start recall turn");
    let request = model_request(&next.host_requests);
    let HostRequest::ModelStream {
        messages, tools, ..
    } = request.payload
    else {
        unreachable!()
    };
    assert_eq!(model_tool_names(&tools), vec!["requirements_update", "task_scope_update"]);
    assert!(
        serde_json::to_string(&messages)
            .expect("serialize model messages")
            .contains(TOOL_MARKER),
        "restored model context lost the prior tool result"
    );
}

#[test]
fn idle_checkpoint_accepts_model_and_context_window_change_without_losing_history() {
    const HISTORY_MARKER: &str = "DURABLE_HISTORY_9327";
    let original_config = core_config(vec![
        AgentItem::user_message(
            ItemId::from_string("history-user-model-change".to_string()),
            "remember the durable marker",
        ),
        AgentItem::assistant_message(
            ItemId::from_string("history-assistant-model-change".to_string()),
            HISTORY_MARKER,
        ),
    ]);
    let mut core = AgentCore::try_new(original_config.clone()).expect("create Core");
    let started = core
        .submit(user_input(
            "complete-before-model-change",
            "finish this turn",
            AgentInputDelivery::CurrentTurn,
        ))
        .expect("start turn");
    let model = model_request(&started.host_requests);
    let completed = submit_response(
        &mut core,
        &model,
        1,
        visible_model_completion("completed-before-model-change", "Completed."),
    );
    let checkpoint = checkpoint_after(&core, &completed);

    assert!(checkpoint
        .state
        .execution
        .active_turn
        .as_ref()
        .is_none_or(|turn| turn.status.is_terminal()));
    assert_eq!(checkpoint.state.host.pending_effects.len(), 0);

    let mut changed_config = original_config;
    changed_config.model = Some("replacement-model".to_string());
    changed_config.max_model_iterations = 7;
    changed_config.context_window.context_window_tokens = 128_000;
    changed_config.context_window.reserved_output_tokens = 8_192;
    changed_config.context_window.prompt_token_limit = Some(117_504);
    changed_config.context_window.auto_compact_tokens = Some(94_003);
    changed_config.context_window.compaction_target_tokens = Some(52_876);

    let mut restored = AgentCore::restore(changed_config, checkpoint)
        .expect("restore idle Core with a new model and context window");
    assert_eq!(
        restored
            .checkpoint(CheckpointReason::Manual)
            .state
            .configuration
            .max_model_iterations,
        7
    );
    let next = restored
        .submit(user_input(
            "recall-after-model-change",
            "recall the durable marker",
            AgentInputDelivery::CurrentTurn,
        ))
        .expect("start turn after configuration replacement");
    let request = model_request(&next.host_requests);
    let HostRequest::ModelStream {
        model_id, messages, ..
    } = request.payload
    else {
        unreachable!()
    };
    assert_eq!(model_id, "replacement-model");
    assert!(
        serde_json::to_string(&messages)
            .expect("serialize messages")
            .contains(HISTORY_MARKER),
        "model/context replacement lost durable conversation history"
    );
    assert_eq!(next.snapshot.context_window_tokens, 128_000);
}

#[test]
fn idle_restore_retains_newer_checkpoint_host_catalog_after_host_counter_restart() {
    let config = core_config(Vec::new());
    let core = AgentCore::try_new(config.clone()).expect("create Core");
    let checkpoint = core.checkpoint(CheckpointReason::Manual);
    let mut newer_checkpoint = checkpoint;
    newer_checkpoint.state.host.catalog.revision = 19;
    newer_checkpoint.state.host.catalog.availability_epoch = 11;
    newer_checkpoint.capability_catalog_revision = 19;

    let mut restarted_host_config = config;
    let restarted_catalog = restarted_host_config
        .host_catalog
        .as_mut()
        .expect("Host catalog");
    restarted_catalog.revision = 1;
    restarted_catalog.availability_epoch = 1;

    let restored = AgentCore::restore(restarted_host_config, newer_checkpoint)
        .expect("idle restore must retain the newer committed Host catalog");
    let snapshot = restored.snapshot();
    assert_eq!(snapshot.host_catalog_revision, 19);
}

#[test]
fn checkpoint_retains_frozen_turn_context_and_replayable_pending_request() {
    const PRIVATE_CONTEXT: &str = "must-not-persist";
    let config = core_config(Vec::new());
    let mut core = AgentCore::try_new(config.clone()).expect("create Core");
    let started = core
        .submit(AgentSubmission::UserInput {
            items: vec![AgentItem::user_message(
                ItemId::from_string("private-user".to_string()),
                "summarize my current data",
            )],
            turn_context_items: vec![AgentItem::Message {
                id: ItemId::from_string("private-context-item".to_string()),
                role: AgentRole::System,
                content: format!("ephemeral payload: {PRIVATE_CONTEXT}"),
            }],
            turn_context_platform: ContextPlatform::Macos,
            turn_context_components: vec![ContextComponent {
                kind: ContextComponentKind::System,
                id: "private-source".to_string(),
                content: json!({"privateToken": PRIVATE_CONTEXT}),
                source: None,
            }],
            submission_id: Some("private-context-submission".to_string()),
            submitted_at_epoch_ms: None,
            delivery: AgentInputDelivery::CurrentTurn,
        })
        .expect("start private contextual turn");

    let checkpoint = checkpoint_after(&core, &started);
    let serialized = serde_json::to_string(&checkpoint).expect("serialize checkpoint");
    assert!(
        serialized.contains(PRIVATE_CONTEXT),
        "a recoverable active Turn must retain its frozen context"
    );

    let retained_turn = checkpoint
        .state
        .execution
        .active_turn
        .as_ref()
        .expect("recoverable Turn identity is retained");
    assert_eq!(retained_turn.context.items.len(), 2);
    assert_eq!(retained_turn.context.components.len(), 1);
    assert!(!retained_turn.context.component_metrics.is_empty());
    let retained_effects = checkpoint
        .state
        .host
        .pending_effects
        .iter()
        .collect::<Vec<_>>();
    assert_eq!(retained_effects.len(), 1);
    assert!(retained_effects[0].request.is_some());

    AgentCore::restore(config, checkpoint).expect("restore redacted checkpoint");
}

#[test]
fn abandoned_non_replayable_effect_can_be_cancelled_before_contextual_new_turn() {
    let mut config = core_config(Vec::new());
    config
        .host_catalog
        .as_mut()
        .expect("Host catalog")
        .capabilities
        .get_mut("model.stream")
        .expect("model capability")
        .replay_safety = HostReplaySafety::RequiresRecoveryDecision;
    let mut original = AgentCore::try_new(config.clone()).expect("create Core");
    let started = original
        .submit(user_input(
            "orphaned-user",
            "create a Word document",
            AgentInputDelivery::CurrentTurn,
        ))
        .expect("start turn");
    let model = model_request(&started.host_requests);
    let checkpoint = checkpoint_after(&original, &started);
    let mut restored = AgentCore::restore(config, checkpoint).expect("restore Core");

    assert!(restored
        .pending_host_requests()
        .expect("pending requests")
        .is_empty());
    assert_eq!(restored.snapshot().pending_recovery_effect_count, 1);

    let interrupted = restored
        .submit(AgentSubmission::Interrupt { goal_control: None, cancellation_id: "test-cancellation".into(),
            reason: Some("recover orphan before new input".to_string()),
        })
        .expect("cancel orphaned turn");
    let cancel = request_matching(&interrupted.host_requests, |payload| {
        matches!(
            payload,
            HostRequest::Cancel { target_effect_id, .. } if target_effect_id == &model.effect_id
        )
    });
    submit_response(
        &mut restored,
        &cancel,
        1,
        HostResponse::CancellationConfirmed {
            target_effect_id: model.effect_id,
        },
    );

    let next = restored
        .submit(AgentSubmission::UserInput {
            items: vec![AgentItem::user_message(
                ItemId::from_string("next-user".to_string()),
                "continue",
            )],
            turn_context_items: Vec::new(),
            turn_context_platform: ContextPlatform::Macos,
            turn_context_components: vec![ContextComponent {
                kind: ContextComponentKind::System,
                id: "mac-context".to_string(),
                content: json!("current Mac context"),
                source: None,
            }],
            submission_id: Some("submission-next-user".to_string()),
            submitted_at_epoch_ms: None,
            delivery: AgentInputDelivery::CurrentTurn,
        })
        .expect("start contextual turn after recovery");

    assert!(next
        .host_requests
        .iter()
        .any(|request| matches!(request.payload, HostRequest::ModelStream { .. })));
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

#[test]
fn permission_request_failures_resume_model_without_execution_grant_or_denial() {
    for code in ["tool_preflight_failed", "permission_ui_failed", "host_dispatch_failed", "invalid_sop_execution_lineage"] {
    for cold_restore in [false, true] {
    let (mut core, ask) = suspended_custom_fixture();
    if cold_restore {
        core = AgentCore::restore(engineering_core_config(), core.checkpoint(CheckpointReason::Manual)).unwrap();
    }
    let rules = core.snapshot().permission_rules.clone();
    let transition = submit_response(&mut core, &ask, 1, HostResponse::Failed {
        error: HostError {
            code: code.into(),
            message: "Command syntax cannot be safely analyzed".into(),
            retryable: false,
            http_status: None,
        },
    });
    assert!(transition.snapshot.permissions.pending.is_empty());
    assert_eq!(transition.snapshot.permission_rules, rules);
    assert!(!transition.event_envelopes.iter().any(|envelope|
        matches!(envelope.event, AgentEvent::PermissionSettled { .. })));
    assert!(transition.host_requests.iter().any(|request|
        matches!(request.payload, HostRequest::ModelStream { .. })));
    assert!(!transition.host_requests.iter().any(|request|
        matches!(request.payload, HostRequest::ExecuteTool { .. })));
    assert!(transition.snapshot.projection.parts.values().any(|part| matches!(part,
        AgentPart::Tool(tool) if tool.call_id == "custom-call"
            && tool.state == ToolPartState::Failed && tool.started_at_ms.is_none()
    )));
    let checkpoint = core.checkpoint(CheckpointReason::Manual);
    let wire = serde_json::to_value(&checkpoint).unwrap();
    assert!(wire.to_string().contains("tool_execution_skipped"));
    assert!(wire.to_string().contains(code));
    assert_eq!(wire["state"]["execution"]["permissions"]["receipts"], json!([]));
    AgentCore::restore(engineering_core_config(), checkpoint).expect("restore after preflight failure");
}
    }
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
fn custom_guidance_reasks_original_call_and_cold_approval_preserves_arguments() {
    let (mut core, ask) = suspended_custom_fixture();
    let original = core.snapshot().permissions.pending[0].clone();
    let guided = core
        .submit(custom_response(
            &ask,
            "keep the original operation; verify the destination",
        ))
        .unwrap();
    assert_eq!(guided.host_requests.len(), 1);
    let next = request_matching(&guided.host_requests, |payload| {
        matches!(payload, HostRequest::RequestPermission { .. })
    });
    assert_ne!(next.effect_id, ask.effect_id);
    let pending = &guided.snapshot.permissions.pending[0];
    assert_ne!(pending.request_id, original.request_id);
    assert_eq!(pending.call_id, original.call_id);
    assert_eq!(pending.arguments, original.arguments);
    assert_eq!(pending.resource, original.resource);
    assert_eq!(pending.permission_instructions.len(), 1);
    assert_eq!(
        pending.permission_instructions[0].permission_request_id,
        original.request_id.as_str()
    );
    let held = core
        .checkpoint(CheckpointReason::Manual)
        .state
        .execution
        .active_turn
        .unwrap();
    assert_eq!(
        held.tools[0].status,
        nola_core::agent::ToolCallStatus::AwaitingPermission
    );
    assert!(held.tools[0].output.is_none());
    assert!(!held.tools[0].is_error);
    assert_eq!(
        guided.snapshot.permission_rules,
        engineering_core_config().permission_rules
    );
    let checkpoint = core.checkpoint(CheckpointReason::Manual);
    let mut reopened = AgentCore::restore(
        engineering_core_config(),
        serde_json::from_slice(&serde_json::to_vec(&checkpoint).unwrap()).unwrap(),
    )
    .unwrap();
    let before_replay =
        serde_json::to_value(reopened.checkpoint(CheckpointReason::Manual).state).unwrap();
    let replay = reopened
        .submit(custom_response(
            &ask,
            "keep the original operation; verify the destination",
        ))
        .unwrap_err();
    assert!(replay.contains("benign_replay"));
    assert_eq!(
        serde_json::to_value(reopened.checkpoint(CheckpointReason::Manual).state).unwrap(),
        before_replay
    );
    let approved = reopened
        .submit(AgentSubmission::PermissionResponse {
            request_id: pending.request_id.clone(),
            decision: PermissionUserDecision::AllowOnce,
            grant: None,
        })
        .unwrap();
    let execute = request_matching(&approved.host_requests, |payload| {
        matches!(payload, HostRequest::ExecuteTool { .. })
    });
    let HostRequest::ExecuteTool {
        call_id,
        input,
        grant,
        permission_instructions,
        ..
    } = execute.payload
    else {
        unreachable!()
    };
    assert_eq!(call_id, original.call_id.as_str());
    assert_eq!(input, original.arguments);
    assert!(grant.is_none());
    assert_eq!(permission_instructions, pending.permission_instructions);
    assert!(approved
        .host_requests
        .iter()
        .all(|r| !matches!(r.payload, HostRequest::ModelStream { .. })));
}
#[test]
fn oversized_custom_guidance_does_not_consume_pending_effect_or_permission() {
    let (mut core, ask) = suspended_custom_fixture();
    let before = serde_json::to_value(core.checkpoint(CheckpointReason::Manual).state).unwrap();
    assert!(core
        .submit(custom_response(&ask, &"x".repeat(4097)))
        .is_err());
    assert_eq!(
        serde_json::to_value(core.checkpoint(CheckpointReason::Manual).state).unwrap(),
        before
    );
    let corrected = core
        .submit(custom_response(&ask, "bounded correction"))
        .unwrap();
    assert_eq!(corrected.snapshot.permissions.pending.len(), 1);
    assert!(matches!(
        corrected.host_requests[0].payload,
        HostRequest::RequestPermission { .. }
    ));
}
#[test]
fn delegated_custom_keeps_backend_identity_until_explicit_approval() {
    let (core, ask) = suspended_custom_fixture();
    let mut checkpoint = core.checkpoint(CheckpointReason::Manual);
    let original = checkpoint.state.execution.permissions.pending[0].clone();
    // A delegated permission belongs to a still-pending backend effect.
    let mut backend_effect = checkpoint
        .state
        .host
        .pending_effects
        .get(&ask.effect_id.0)
        .unwrap()
        .clone();
    backend_effect.effect_id = "original-backend-effect".into();
    backend_effect.request_id = "backend-original-request".into();
    backend_effect.capability_id = "agent.delegate".into();
    backend_effect.call_id = Some("backend-call".into());
    // Retain the complete typed delegated invocation, not just a synthetic
    // effect id. Restore must verify the frozen request against its journal.
    let mut backend_request = ask.clone();
    backend_request.effect_id = HostEffectId(backend_effect.effect_id.clone());
    backend_request.request_id = backend_effect.request_id.clone();
    backend_request.delegated_invocation_id = Some("custom-delegated-invocation".into());
    let delegated_request: nola_host_protocol::DelegatedAgentRequestV1 =
        serde_json::from_value(json!({
            "contract_version": 1,
            "backend_id": "backend",
            "backend_session_id": "backend-session",
            "text_part_id": "delegated-custom-text",
            "reasoning_part_id": "delegated-custom-reasoning",
            "context": {
                "mode": "resume", "identity_digest": "custom-delegated-context",
                "base_anchor_item_id": "custom-base",
                "target_anchor_item_id": "custom-user",
                "messages": [{"role":"user","content":"write the protected file"}]
            },
            "tools": [], "settings": {}
        }))
        .unwrap();
    delegated_request.validate().unwrap();
    backend_request.payload = HostRequest::DelegatedAgent {
        request: delegated_request,
    };
    backend_effect.request = Some(serde_json::to_value(&backend_request).unwrap());
    assert_eq!(backend_effect.validated_request().unwrap(), backend_request);
    assert!(checkpoint
        .state
        .host
        .pending_effects
        .register(backend_effect));
    checkpoint.state.execution.permissions.pending[0].continuation =
        nola_core::agent::PermissionContinuation::DelegatedAgent {
            occurrence: nola_host_protocol::DelegatedPermissionOccurrence {
                occurrence_id: "occurrence".into(),
                worker_generation: "worker".into(),
            },
            backend_permission_request_id: None,
            target_effect_id: "original-backend-effect".into(),
            backend_id: "backend".into(),
            backend_session_id: Some("backend-session".into()),
        };
    let original_tools = serde_json::to_value(
        &checkpoint
            .state
            .execution
            .active_turn
            .as_ref()
            .unwrap()
            .tools,
    )
    .unwrap();
    let mut core = AgentCore::restore(engineering_core_config(), checkpoint).unwrap();
    assert_eq!(
        core.checkpoint(CheckpointReason::Manual)
            .state
            .host
            .pending_effects
            .get("original-backend-effect")
            .unwrap()
            .validated_request()
            .unwrap(),
        backend_request,
        "cold restore must retain the exact delegated invocation envelope",
    );
    let guided = core
        .submit(custom_response(&ask, "keep delegated call unchanged"))
        .unwrap();
    assert_eq!(
        core.checkpoint(CheckpointReason::Manual)
            .state
            .host
            .pending_effects
            .get("original-backend-effect")
            .unwrap()
            .validated_request()
            .unwrap(),
        backend_request,
        "permission guidance must not replace the delegated backend request",
    );
    assert_eq!(
        serde_json::to_value(
            &core
                .checkpoint(CheckpointReason::Manual)
                .state
                .execution
                .active_turn
                .unwrap()
                .tools
        )
        .unwrap(),
        original_tools,
        "delegated guidance must not mutate a same-ID Core tool"
    );
    assert!(guided
        .host_requests
        .iter()
        .all(|r| matches!(r.payload, HostRequest::RequestPermission { .. })));
    let pending = &guided.snapshot.permissions.pending[0];
    assert_ne!(pending.request_id, original.request_id);
    let resolved = core
        .submit(AgentSubmission::PermissionResponse {
            request_id: pending.request_id.clone(),
            decision: PermissionUserDecision::AllowOnce,
            grant: None,
        })
        .unwrap();
    let request = request_matching(&resolved.host_requests, |payload| {
        matches!(payload, HostRequest::ResolveDelegatedPermission { .. })
    });
    let HostRequest::ResolveDelegatedPermission {
        target_effect_id,
        backend_id,
        backend_session_id,
        permission_request_id,
        permission_instructions,
        ..
    } = request.payload
    else {
        unreachable!()
    };
    assert_eq!(target_effect_id.0, "original-backend-effect");
    assert_eq!(backend_id, "backend");
    assert_eq!(backend_session_id.as_deref(), Some("backend-session"));
    assert_eq!(permission_request_id, original.request_id.as_str());
    assert_eq!(permission_instructions, pending.permission_instructions);
}

#[test]
fn custom_guidance_count_and_total_budget_leave_last_request_answerable() {
    for (accepted_count, instruction) in [(16, "short".to_string()), (4, "x".repeat(4000))] {
        let (mut core, mut ask) = suspended_custom_fixture();
        for _ in 0..accepted_count {
            let transition = core.submit(custom_response(&ask, &instruction)).unwrap();
            ask = request_matching(&transition.host_requests, |p| {
                matches!(p, HostRequest::RequestPermission { .. })
            });
        }
        let before = serde_json::to_value(core.checkpoint(CheckpointReason::Manual).state).unwrap();
        assert!(core
            .submit(custom_response(&ask, &instruction))
            .unwrap_err()
            .contains("bounded guidance budget"));
        assert_eq!(
            serde_json::to_value(core.checkpoint(CheckpointReason::Manual).state).unwrap(),
            before
        );
        let HostRequest::RequestPermission {
            permission_request_id,
            ..
        } = &ask.payload
        else {
            unreachable!()
        };
        let denied = core
            .submit(AgentSubmission::PermissionResponse {
                request_id: nola_core::agent::PermissionRequestId::from_string(
                    permission_request_id.clone(),
                ),
                decision: PermissionUserDecision::Deny,
                grant: None,
            })
            .unwrap();
        assert!(denied.snapshot.permissions.pending.is_empty());
        assert!(denied
            .host_requests
            .iter()
            .all(|r| !matches!(r.payload, HostRequest::ExecuteTool { .. })));
    }
}

#[test]
fn permission_transport_interruption_is_nonterminal_and_cold_resume_rebinds_original_ask() {
    use nola_core::agent::recovery::PendingHostEffectState;
    use nola_host_protocol::PermissionTransportInterruptionReason as Reason;
    for reason in [Reason::Timeout, Reason::Disconnected, Reason::Unavailable] {
        let (mut core, initial) = suspended_custom_fixture();
        let guided = core
            .submit(custom_response(&initial, "Retain original arguments"))
            .unwrap();
        let ask = request_matching(&guided.host_requests, |p| {
            matches!(p, HostRequest::RequestPermission { .. })
        });
        let before = core.checkpoint(CheckpointReason::Manual);
        let pending = before
            .state
            .host
            .pending_effects
            .get(&ask.effect_id.0)
            .unwrap()
            .clone();
        let response = HostResponse::PermissionTransportInterrupted { reason };
        assert!(!response.is_terminal());
        assert!(ask.payload.accepts_response(&response));
        let wire = serde_json::to_value(&response).unwrap();
        assert_eq!(wire["type"], "permission_transport_interrupted");
        assert_eq!(
            serde_json::from_value::<HostResponse>(wire).unwrap(),
            response
        );
        let interrupted = submit_response(&mut core, &ask, 1, response);
        assert!(interrupted.host_requests.is_empty());
        assert_eq!(
            interrupted.snapshot.permissions.pending,
            guided.snapshot.permissions.pending
        );
        assert_eq!(
            interrupted.snapshot.permission_rules,
            guided.snapshot.permission_rules
        );
        assert_eq!(
            interrupted.durability.unwrap().checkpoint_reason,
            CheckpointReason::PermissionTransportInterrupted
        );
        let checkpoint = core.checkpoint(CheckpointReason::Manual);
        let held = checkpoint
            .state
            .host
            .pending_effects
            .get(&ask.effect_id.0)
            .unwrap();
        assert_eq!(
            held.state,
            PendingHostEffectState::PermissionInterrupted { reason }
        );
        assert_eq!(held.request, pending.request);
        assert_eq!(held.request_id, pending.request_id);
        assert_eq!(held.last_response_sequence, Some(1));
        assert_eq!(
            serde_json::to_value(&checkpoint.state.execution.active_turn).unwrap(),
            serde_json::to_value(&before.state.execution.active_turn).unwrap()
        );
        let mut reopened = AgentCore::restore(
            engineering_core_config(),
            serde_json::from_slice(&serde_json::to_vec(&checkpoint).unwrap()).unwrap(),
        )
        .unwrap();
        let resumed = reopened
            .submit(AgentSubmission::ResumeHostEffect {
                effect_id: ask.effect_id.0.clone(),
            })
            .unwrap();
        assert_eq!(resumed.host_requests.len(), 1);
        let rebound = &resumed.host_requests[0];
        assert_eq!(rebound.effect_id, ask.effect_id);
        assert_ne!(rebound.request_id, ask.request_id);
        assert_eq!(rebound.payload, ask.payload);
        assert_eq!(rebound.resume_after_sequence, Some(1));
        assert_eq!(
            reopened
                .checkpoint(CheckpointReason::Manual)
                .state
                .host
                .pending_effects
                .get(&ask.effect_id.0)
                .unwrap()
                .state,
            PendingHostEffectState::Dispatched
        );
        let HostRequest::RequestPermission {
            permission_request_id,
            ..
        } = &rebound.payload
        else {
            unreachable!()
        };
        let approved = submit_response(
            &mut reopened,
            rebound,
            2,
            HostResponse::PermissionResolved {
                request_id: permission_request_id.clone(),
                resolution: nola_host_protocol::HostPermissionResolution::allow_once(),
            },
        );
        let execute = request_matching(&approved.host_requests, |p| {
            matches!(p, HostRequest::ExecuteTool { .. })
        });
        let HostRequest::ExecuteTool {
            input,
            permission_instructions,
            ..
        } = execute.payload
        else {
            unreachable!()
        };
        assert_eq!(input, guided.snapshot.permissions.pending[0].arguments);
        assert_eq!(
            permission_instructions,
            guided.snapshot.permissions.pending[0].permission_instructions
        );
    }
}

#[test]
fn permission_transport_interruption_rejects_non_permission_effect_without_mutation() {
    let mut core = AgentCore::try_new(engineering_core_config()).unwrap();
    let started = core
        .submit(user_input(
            "interrupt-model",
            "hello",
            AgentInputDelivery::CurrentTurn,
        ))
        .unwrap();
    let model = model_request(&started.host_requests);
    let before = serde_json::to_value(core.checkpoint(CheckpointReason::Manual).state).unwrap();
    let response = HostResponse::PermissionTransportInterrupted {
        reason: nola_host_protocol::PermissionTransportInterruptionReason::Unavailable,
    };
    assert!(!model.payload.accepts_response(&response));
    assert!(core
        .submit(AgentSubmission::HostResponse {
            response: response_for(&model, 1, response)
        })
        .is_err());
    assert_eq!(
        serde_json::to_value(core.checkpoint(CheckpointReason::Manual).state).unwrap(),
        before
    );
}

#[test]
fn pending_skill_completion_keeps_real_model_protocol_tools_after_premature_final_answer() {
    let mut core = AgentCore::try_new(skill_runtime_core_config()).unwrap();
    let started = core
        .submit(user_input(
            "skill-boundary-user",
            "Create the document.",
            AgentInputDelivery::CurrentTurn,
        ))
        .unwrap();
    let digest = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let invoked = complete_named_tool_round(
        &mut core,
        &model_request(&started.host_requests),
        "skill-boundary-call",
        "Skill",
        json!({"skill":"document-create"}),
        json!({"type":"invoked_skill","skill_id":"document-create","version":"1.0.0","display_name":"Document create","instructions_ref":format!("skill://document-create/1.0.0/SKILL.md#sha256={digest}"),"instructions_digest":digest,"required_tools":[],"mode":"inline","execution_contract":{"schemaVersion":1,"progressSteps":[{"id":"render","title":"Render document","required":true}],"artifacts":[],"verifiers":[],"resumePoints":[]}}),
    );
    let resumable = core.checkpoint(CheckpointReason::Manual);
    let next = model_request(&invoked.host_requests);
    let premature = submit_response(
        &mut core,
        &model_request(&invoked.host_requests),
        1,
        visible_model_completion("premature-final", "The document is ready."),
    );
    assert!(premature.event_envelopes.iter().any(|event| matches!(&event.event,
        AgentEvent::RunResultDecided { decision, .. }
            if matches!(decision.value, nola_domain::run_result::RunResultValue::Incomplete { .. }))));
    assert!(premature.host_requests.is_empty());
    core = AgentCore::restore(skill_runtime_core_config(), resumable).unwrap();
    let HostRequest::ModelStream { tools, .. } = &next.payload else {
        unreachable!()
    };
    assert!(
        tools.iter().any(
            |tool| tool.pointer("/function/name").and_then(Value::as_str) == Some("skill_update")
        ),
        "pending Skill work must prevent a read-only final-synthesis request"
    );
    assert!(premature
        .snapshot
        .skill_runtime
        .runs
        .values()
        .any(|run| !run.completed_progress_steps.contains("render")));
    let run_id = invoked.snapshot.skill_runtime.runs.keys().next().unwrap().clone();
    let progressed = complete_named_tool_round(&mut core, &next, "explicit-skill-progress", "skill_update",
        json!({"run_id":run_id,"action":"progress","step_id":"render","completed":true}),
        json!({"type":"skill_runtime_update","run_id":run_id,"action":"progress","step_id":"render","completed":true}));
    assert!(progressed.snapshot.skill_runtime.runs[&run_id].completed_progress_steps.contains("render"));
}

#[test]
fn final_skill_protocol_round_preserves_tool_prefix_and_identifies_remaining_records() {
    let mut config = skill_runtime_core_config();
    config.max_model_iterations = 3;
    // This fixture validates Skill convergence, not process authorization.
    // Keep opaque bash risky and grant only this simulated capability explicitly.
    config.permission_rules.rules.push(nola_core::agent::permissions::PermissionRule {
        capability_pattern: "bash".into(),
        resource_pattern: "*".into(),
        decision: nola_core::agent::permissions::PermissionPolicyDecision::Allow,
        source: nola_core::agent::permissions::PermissionRuleSource::UserGlobal,
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
            "description": "execute",
            "parameters": {"type": "object"}
        }
    }));

    let mut core = AgentCore::try_new(config).unwrap();
    let started = core
        .submit(user_input(
            "skill-convergence-user",
            "Produce the requested durable result.",
            AgentInputDelivery::CurrentTurn,
        ))
        .unwrap();
    let digest = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let invoked = complete_named_tool_round(
        &mut core,
        &model_request(&started.host_requests),
        "skill-convergence-call",
        "Skill",
        json!({"skill":"bounded-work"}),
        json!({"type":"invoked_skill","skill_id":"bounded-work","version":"1.0.0","display_name":"Bounded work","instructions_ref":format!("skill://bounded-work/1.0.0/SKILL.md#sha256={digest}"),"instructions_digest":digest,"required_tools":[],"mode":"inline","execution_contract":{"schemaVersion":1,"progressSteps":[{"id":"execute","title":"Execute work","required":true}],"artifacts":[],"verifiers":[],"resumePoints":[]}}),
    );
    let executed = complete_named_tool_round(
        &mut core,
        &model_request(&invoked.host_requests),
        "skill-convergence-execute",
        "bash",
        json!({"operation_kind":"mutate_and_validate", "cwd":"/workspace",
            "output_artifacts":[{"path":"result.txt","operation":"create"}],
            "validation_targets":["result.txt"]}),
        observed_output(
            json!({"ok":true}),
            &[("file:///workspace/result.txt", "v1")],
            &[("file:///workspace/result.txt", "v1")],
        ),
    );

    let final_tool_round = model_request(&executed.host_requests);
    let HostRequest::ModelStream {
        response_mode,
        tools,
        messages,
        ..
    } = &final_tool_round.payload
    else {
        unreachable!()
    };
    assert_eq!(*response_mode, ModelResponseMode::RequiredTool);
    assert_eq!(model_tool_names(tools), vec!["Skill", "bash", "requirements_update", "skill_update", "task_scope_update", "write_file"],
        "convergence guidance must not churn the previously exposed Host schemas");
    assert!(messages.iter().any(|message| {
        message["role"] == "system" && message
            .get("content")
            .and_then(Value::as_str)
            .is_some_and(|content| {
                content.contains("pending records in runtime facts")
                    && content.contains("exact internal skill_update calls")
                    && !content.contains("skill_update run_id=bounded-work")
            })
    }));
    let (fact_message, facts) = messages.iter().filter_map(|message| {
        let (_, payload) = message["content"].as_str()?.split_once('\n')?;
        let facts: Value = serde_json::from_str(payload).ok()?;
        (facts["schema"] == "nola.runtime_facts.v1" && facts["kind"] == "skill-pending-records")
            .then_some((message, facts))
    }).next().expect("separate pending Skill runtime facts");
    assert_eq!(fact_message["role"], "user");
    assert_eq!(fact_message["_nola_user_input"], false);
    assert!(facts["facts"]["records"].as_array().unwrap().iter().any(|record|
        record == "skill_update run_id=bounded-work action=progress step_id=execute completed=true (Execute work)"));
    assert!(messages.iter().filter(|message| message["role"] == "system")
        .all(|message| !message["content"].as_str().unwrap_or_default()
            .contains("skill_update run_id=bounded-work")),
        "pending Skill records must not be interpolated into system policy");
}

#[test]
fn active_skill_records_host_observed_model_elapsed_time() {
    let mut core = AgentCore::try_new(skill_runtime_core_config()).unwrap();
    let started = core
        .submit_at(
            user_input(
                "skill-elapsed-user",
                "Create the document.",
                AgentInputDelivery::CurrentTurn,
            ),
            1_000,
        )
        .unwrap();
    let first_model = model_request(&started.host_requests);
    let dispatched = core
        .submit_at(
            AgentSubmission::HostResponse {
                response: response_for(
                    &first_model,
                    1,
                    empty_model_completion(vec![named_tool_call(
                        "skill-elapsed-call",
                        "Skill",
                        json!({"skill":"document-create"}),
                    )]),
                ),
            },
            1_100,
        )
        .unwrap();
    let skill_request = request_matching(
        &dispatched.host_requests,
        |payload| matches!(payload, HostRequest::ExecuteTool { call_id, .. } if call_id == "skill-elapsed-call"),
    );
    let digest = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let invoked = core
        .submit_at(
            AgentSubmission::HostResponse {
                response: response_for(
                    &skill_request,
                    1,
                    HostResponse::ToolCompleted {
                        call_id: "skill-elapsed-call".to_string(),
                        output: json!({
                            "type":"invoked_skill",
                            "skill_id":"document-create",
                            "version":"1.0.0",
                            "instructions_ref":format!("skill://document-create/1.0.0/SKILL.md#sha256={digest}"),
                            "instructions_digest":digest,
                            "required_tools":[],
                            "mode":"inline",
                            "execution_contract":{"schemaVersion":1,"progressSteps":[{"id":"render","title":"Render","required":true}]}
                        }),
                        outcome: nola_core::agent::host::HostToolOutcome::Succeeded,
                        activation: None,
                        full_output_ref: None,
                        artifacts: Vec::new(),
                    },
                ),
            },
            1_200,
        )
        .unwrap();
    let skill_model = model_request(&invoked.host_requests);
    let continued = core
        .submit_at(
            AgentSubmission::HostResponse {
                response: response_for(
                    &skill_model,
                    1,
                    empty_model_completion(vec![named_tool_call(
                        "skill-work-call",
                        "read_file",
                        json!({"path":"source.md"}),
                    )]),
                ),
            },
            1_500,
        )
        .unwrap();

    let run = continued
        .snapshot
        .skill_runtime
        .runs
        .values()
        .next()
        .unwrap();
    assert_eq!(run.usage.elapsed_millis, 300);
    assert_eq!(run.usage.model_turns, 1);
}

#[test]
fn goal_declared_execution_check_cannot_be_certified_by_a_model_answer_after_restore() {
    use nola_core::agent::*;
    use nola_core::agent::goal::{GoalAcceptancePolicy, GoalRequiredCheck};
    let mut core = AgentCore::try_new(source_core_config()).unwrap();
    let command = |id: &str, command| AgentSubmission::GoalRuntime {
        envelope: GoalRuntimeCommandEnvelope { operation_id:id.into(), command },
    };
    core.submit(command("start-check", GoalRuntimeCommand::Start {
        id:"check-goal".into(), objective:"Run the declared check".into(), constraints:vec![],
        requirements:vec![GoalRequirement { acceptance_policy:GoalAcceptancePolicy::ExecutionChecks {
            checks:vec![GoalRequiredCheck { kind:nola_domain::execution_evidence::ExecutionCheckKind::Postcondition, resource_uri:"file:artifact".into(),version:"v1".into(),predicate:"test.suite".into() }] },
            id:"r".into(),semantic_revision:1,description:"Actual suite result".into(),dependencies:vec![],
            status:GoalRecordStatus::Pending,evidence_refs:vec![] }],now_epoch_ms:0,
    })).unwrap();
    core.submit(command("plan-check", GoalRuntimeCommand::PlanContract { contract:GoalSubtaskContract { execution_run_id: None,
        id:"check-contract".into(),round:1,objective:"Inspect".into(),proposed_checks:vec!["A statement is enough".into()],
        requirement_versions:std::collections::BTreeMap::from([("r".into(),1)]),
        additional_execution_constraints:vec![],relevant_record_ids:vec!["r".into()],
        status:GoalContractStatus::Planned,created_at_epoch_ms:1 },now_epoch_ms:1,
    })).unwrap();
    core.submit(command("begin-check",GoalRuntimeCommand::BeginExecution { run_id: "check-run".into(), contract_id:"check-contract".into(),now_epoch_ms:2 })).unwrap();
    let started = core.submit(AgentSubmission::GoalUserInput {
        items:vec![AgentItem::user_message(ItemId::from_string("check-input".into()),"Report the outcome")],
        turn_context_items:vec![],turn_context_platform:Default::default(),turn_context_components:vec![],
        submission_id:Some("check-run".into()),submitted_at_epoch_ms:None,delivery:AgentInputDelivery::CurrentTurn,
    }).unwrap();
    let request = model_request(&started.host_requests);
    submit_response(&mut core,&request,1,visible_model_completion("claimed-check","The test suite passed."));
    core.submit(command("report-check",GoalRuntimeCommand::RecordExecution { report:GoalExecutionReport {
        contract_id:"check-contract".into(),summary:"Claimed passed".into(),claimed_record_ids:vec!["r".into()],
        artifact_refs:vec![],blockers:vec![],reported_at_epoch_ms:3,
    }})).unwrap();
    let checkpoint = core.checkpoint(nola_core::agent::recovery::CheckpointReason::Manual);
    let mut restored = AgentCore::restore(source_core_config(),checkpoint).unwrap();
    let before = restored.snapshot().goal;
    let error = restored.submit(command("accept-check",GoalRuntimeCommand::AcceptCompletion {
        expected_revision:before.revision,contract_id:"check-contract".into(),now_epoch_ms:4,
        claims:vec![GoalCompletionClaim { requirement_id:"r".into(),requirement_revision:1,summary:"I said it passed".into(),
            evidence_refs:vec!["response".into()],execution_evidence_refs:vec![],task_replacements:vec![] }],
    })).unwrap_err();
    assert!(error.contains("Required execution check"),"{error}");
    assert_eq!(restored.snapshot().goal,before);
    assert!(restored.snapshot().goal.active.unwrap().requirement_acceptances.is_empty());
}

#[test]
fn completion_review_preserves_typed_ui_action_and_multilingual_user_intent() {
    let identity = json!({"owner":{"authority_id":"host","conversation_id":"conversation","participant_id":"agent",
        "context_id":"conversation","originating_run_id":"prior-run","message_id":"assistant","part_id":"tool"},
        "surface_id":"form","generation":1});
    let envelope = json!({"type":"ui_action","surface":identity,"message":{"version":"v0.9.1",
        "action":{"name":"submit","sourceComponentId":"submit","surfaceId":"form","timestamp":"2026-10-21T00:00:00Z",
            "context":{"name":"完整输入 / Complete input / 日本語","count":7}}}});
    let action = serde_json::from_value(json!({"surface":{"identity":identity,"revision":3},
        "summary":"提交表单", "envelope":envelope})).unwrap();
    let mut input = user_input("ordinary-user", "请确认 / Please confirm / 確認してください", AgentInputDelivery::CurrentTurn);
    if let AgentSubmission::UserInput { items, .. } = &mut input {
        items.push(AgentItem::UiAction { id: ItemId::from_string("canonical-action".into()), action });
    }
    let mut core = AgentCore::try_new(source_core_config()).unwrap();
    let started = core.submit(input).unwrap();
    let request = model_request(&started.host_requests);
    let candidate = submit_response(&mut core, &request, 1, model_requested_review("review-action", "已确认完整输入。"));
    let audit = model_request(&candidate.host_requests);
    let HostRequest::ModelStream { messages, response_mode, .. } = &audit.payload else { unreachable!() };
    assert_eq!(*response_mode, ModelResponseMode::StructuredAudit);
    let packet: Value = serde_json::from_str(messages[1]["content"].as_str().unwrap()).unwrap();
    let originals = packet["original_user_messages"].as_array().unwrap();
    assert_eq!(originals.len(), 2, "runtime user-role facts are not original user intent");
    assert!(originals.iter().any(|item| item["id"] == "ordinary-user" && item["content"] == "请确认 / Please confirm / 確認してください"));
    let typed = originals.iter().find(|item| item["id"] == "canonical-action").expect("canonical action retained");
    assert_eq!(typed["action"]["envelope"], envelope);
    assert_eq!(typed["action"]["surface"]["identity"], identity);
    assert_eq!(typed["action"]["surface"]["revision"], 3);
    let current = packet["current_turn_user_message_ids"].as_array().unwrap();
    assert!(current.contains(&json!("canonical-action")) && current.contains(&json!("ordinary-user")));
    assert!(packet["evidence"].as_array().unwrap().iter().any(|item|
        item["role"] == "user" && item["content"].as_str().is_some_and(|text| text.contains("nola.runtime_facts.v1"))));
    assert!(packet["evidence"].as_array().unwrap().iter().all(|item|
        !originals.iter().any(|original| original["id"] == item["id"])));
}
