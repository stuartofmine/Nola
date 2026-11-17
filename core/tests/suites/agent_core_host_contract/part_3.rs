// SPDX-License-Identifier: Apache-2.0
//! Partition 3 of `agent_core_host_contract` scenarios.

use super::*;

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
