// SPDX-License-Identifier: Apache-2.0
//! Partition 1 of `agent_core_host_contract` scenarios.

use super::*;

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
    let mut output = json!({"contract":"nola.tool_result.v1", "data":{"text":"done"},
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
        "../../fixtures/capability_activation_v1.json"
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
        serde_json::from_str(include_str!("../../fixtures/capability_activation_v1.json"))
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
        serde_json::from_str(include_str!("../../fixtures/capability_activation_v1.json"))
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
