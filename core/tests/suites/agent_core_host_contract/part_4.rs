// SPDX-License-Identifier: Apache-2.0
//! Partition 4 of `agent_core_host_contract` scenarios.

use super::*;

#[test]
fn conversation_grant_cannot_expand_to_a_different_pending_resource() {
    exercise_pending_conversation_grant(false);
}

#[test]
fn conversation_grant_reuse_retains_original_decision_source() {
    exercise_pending_conversation_grant(true);
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
