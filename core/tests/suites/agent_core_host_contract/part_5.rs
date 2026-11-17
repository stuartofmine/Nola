// SPDX-License-Identifier: Apache-2.0
//! Partition 5 of `agent_core_host_contract` scenarios.

use super::*;

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
    let trace: serde_json::Value = serde_json::from_str(include_str!("../../fixtures/mac_compaction_20261010.json")).unwrap();
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
fn first_summary_keeps_old_blocks_when_target_fits_and_consolidates_before_request_when_needed() {
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
    assert!(pending.handoff);
    assert!(retained_blocks.iter().all(|item| pending.source_item_ids.contains(item.id())));
    let HostRequest::Compact { instructions, .. } = &compact.payload else { unreachable!() };
    assert!(instructions.contains("50% reduction of the COMPLETE model context"));
    let completed = submit_response(&mut core, &compact, 1, HostResponse::CompactionCompleted {
        summary: "Offline operation is required. Additional constraints recorded; new work remains pending.".into(), usage: serde_json::json!({}),
    });
    assert!(!completed.host_requests.iter().any(|request| matches!(request.payload, HostRequest::Compact { .. })));
    let final_checkpoint = core.checkpoint(CheckpointReason::Manual);
    let blocks = final_checkpoint.state.conversation.model_context.iter()
        .filter(|item| matches!(item, AgentItem::Compaction { .. })).collect::<Vec<_>>();
    assert_eq!(blocks.len(), 1);
    assert!(matches!(blocks[0], AgentItem::Compaction { source_item_ids, .. }
        if retained_blocks.iter().all(|item| source_item_ids.contains(item.id()))));
    assert!(final_checkpoint.state.conversation.transcript.iter().any(|item| item == &previous));
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
    let config = core_config(initial_items);
    let mut core = AgentCore::try_new(config.clone()).expect("create Core");

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

    let HostRequest::Compact { instructions, .. } = &compact.payload else { unreachable!() };
    assert!(instructions.contains("50% reduction of the COMPLETE model context"));
    assert!(instructions.contains(&format!("at most {} tokens", requested.snapshot.context_used_tokens / 2)));
    let checkpoint = checkpoint_after(&core, &requested);
    assert!(checkpoint.state.conversation.context_window.pending_compaction.as_ref().unwrap().handoff);
    // Cold restore continues the first planned summary; there is no second
    // default summary round even when immutable content prevents a 50% gain.
    core = AgentCore::restore(config, checkpoint).unwrap();
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
    assert!(!completed.host_requests.iter().any(|request| matches!(request.payload,
        HostRequest::Compact { .. } | HostRequest::ModelStream { .. })));
    assert_eq!(completed.snapshot.turn_status, nola_core::agent::AgentTurnStatus::Completed);
    let final_state = core.checkpoint(CheckpointReason::Manual);
    let history_tokens = final_state.state.conversation.model_context.iter()
        .fold(0usize, |sum,item| sum.saturating_add(item.estimated_tokens()));
    assert!(completed.snapshot.context_used_tokens > history_tokens,
        "sealing must not falsely erase the fixed request prefix");
    assert_eq!(final_state.state.conversation.transcript.iter()
        .filter(|item| matches!(item, AgentItem::Compaction { .. })).count(), 1);
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
