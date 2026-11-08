// SPDX-License-Identifier: Apache-2.0
//! Real Host response entrypoint coverage for ordinary requirement decisions.
use super::*;
use nola_domain::run_result::RunResultValue;
use nola_domain::TextReviewState;

#[test]
fn compact_result_protocol_rejects_unobserved_evidence_without_side_effects() {
    let mut core = AgentCore::try_new(core_config(vec![])).unwrap();
    let start = core.submit(user_input("compact-result", "Finish the task",
        AgentInputDelivery::CurrentTurn)).unwrap();
    let response = empty_model_completion(vec![named_tool_call("invented-evidence", "requirements_update",
        json!({"outcome":"completed","answer":"Done", "assessments":[{
            "requirement":"r1","verdict":"satisfied","rationale":"unsupported",
            "evidence":["e999"]}]}))]);
    let rejected = submit_response(&mut core, &model_request(&start.host_requests), 1, response);
    assert!(!rejected.event_envelopes.iter().any(|event| matches!(event.event,
        AgentEvent::RunResultDecided { .. })));
    let checkpoint = core.checkpoint(CheckpointReason::Manual);
    assert!(checkpoint.state.execution.requirements.assessments.is_empty());
    assert!(serde_json::to_string(&checkpoint.state.execution.requirements.control_recovery)
        .unwrap().contains("unknown current reference e999"));
}

#[test]
fn model_control_binding_is_frozen_and_stale_scope_is_rejected_after_restore() {
    let config = core_config(vec![]);
    let mut core = AgentCore::try_new(config.clone()).unwrap();
    let start = core.submit(user_input("binding-input", "Explain simply",
        AgentInputDelivery::CurrentTurn)).unwrap();
    let request = model_request(&start.host_requests);
    let mut checkpoint = checkpoint_after(&core, &start);
    let binding = checkpoint.state.execution.active_turn.as_mut().unwrap()
        .last_model_requirement_binding.as_mut().unwrap();
    binding.expected_input_frontier_token = "stale-request-frontier".into();
    core = AgentCore::restore(config, checkpoint).unwrap();
    let rejected = submit_response(&mut core, &request, 1,
        empty_model_completion(vec![named_tool_call("stale-control", "requirements_update",
            json!({"outcome":"answered",
                "answer":"Must not publish from stale context."}))]));
    assert!(!rejected.event_envelopes.iter().any(|event| matches!(event.event,
        AgentEvent::RunResultDecided { .. })));
    let checkpoint = core.checkpoint(CheckpointReason::Manual);
    let recovery = serde_json::to_value(checkpoint.state.execution.requirements.control_recovery).unwrap();
    assert!(recovery.to_string().contains("scope_conflict"));
}

#[test]
fn repeated_valid_execution_review_concludes_incomplete_not_protocol_failure() {
    let config = source_core_config();
    let mut core = AgentCore::try_new(config.clone()).unwrap();
    let started = core
        .submit(user_input(
            "review-budget-input",
            "Finish the requested work",
            AgentInputDelivery::CurrentTurn,
        ))
        .unwrap();
    let mut request = model_request(&started.host_requests);
    for round in 0..3 {
        let candidate = submit_response(
            &mut core,
            &request,
            1,
            model_requested_review(&format!("review-{round}"), "Unverified candidate."),
        );
        let review = model_request(&candidate.host_requests);
        // Cold recovery retains the explicit review and its bounded recovery count.
        let mut checkpoint = core.checkpoint(CheckpointReason::Manual);
        if round == 2 {
            checkpoint.state.execution.active_turn.as_mut().unwrap().engineering.validation_required = true;
        }
        core = AgentCore::restore(config.clone(), checkpoint).unwrap();
        let report = r#"{"decision":"execute","outcome":"incomplete","constraints":["finish the work"],"items":[],"violations":["Required implementation is missing"]}"#;
        let reduced = submit_response(
            &mut core,
            &review,
            1,
            visible_model_completion(&format!("review-report-{round}"), report),
        );
        assert!(!reduced
            .event_envelopes
            .iter()
            .map(|envelope| &envelope.event)
            .any(|event| matches!(event, AgentEvent::TurnFailed { .. })));
        if round < 2 {
            request = model_request(&reduced.host_requests);
        } else {
            let decision = reduced
                .event_envelopes
                .iter()
                .map(|envelope| &envelope.event)
                .find_map(|event| match event {
                    AgentEvent::RunResultDecided { decision, .. } => Some(decision),
                    _ => None,
                })
                .expect("bounded review must conclude with a business result");
            assert!(
                matches!(&decision.value, RunResultValue::Incomplete { reason, .. }
                if reason.code == "review.execution_recovery_budget"
                    && reason.detail.as_deref().is_some_and(|detail| detail.contains("Required implementation is missing")))
            );
            assert!(reduced
                .event_envelopes
                .iter()
                .map(|envelope| &envelope.event)
                .any(|event| matches!(event, AgentEvent::TurnCompleted { .. })));
            assert!(!reduced.event_envelopes.iter().any(|envelope| matches!(&envelope.event,
                AgentEvent::PartUpdated { part: AgentPart::Text(text), .. }
                    if text.is_complete && matches!(text.review, None | Some(TextReviewState::Approved))
                        && text.text == "Unverified candidate.")));
        }
    }
}

#[test]
fn host_goal_complete_cannot_bypass_core_or_accept_owning_run() {
    let mut config = core_config(vec![]);
    config.host_catalog.as_mut().unwrap().capabilities.insert(
        "goal_complete".into(),
        capability("goal_complete", HostCapabilityRoute::Tool, None),
    );
    config.tools.push(json!({"type":"function","function":{"name":"goal_complete","parameters":{"type":"object"}}}));
    let mut core = AgentCore::try_new(config).unwrap();
    let started = core
        .submit(user_input(
            "goal-input",
            "Report the goal state",
            AgentInputDelivery::CurrentTurn,
        ))
        .unwrap();
    let requested = submit_response(
        &mut core,
        &model_request(&started.host_requests),
        1,
        empty_model_completion(vec![named_tool_call(
            "goal-call",
            "goal_complete",
            json!({}),
        )]),
    );
    // goal_complete is Core-owned, not an arbitrary Host tool. Registering a
    // same-named Host descriptor must not execute it or forge Goal/Run success.
    assert!(!requested.host_requests.iter().any(|request| matches!(
        request.payload, HostRequest::ExecuteTool { .. }
    )));
    assert!(!requested.event_envelopes.iter().any(|envelope| matches!(
        &envelope.event,
        AgentEvent::RunResultDecided { .. } | AgentEvent::TurnCompleted { .. }
    )));
    let checkpoint = core.checkpoint(CheckpointReason::Manual);
    assert!(checkpoint.state.goal_projection.active.is_none());
    assert!(checkpoint.state.execution.requirements.decision.is_none());
    model_request(&requested.host_requests);
}

#[test]
fn real_user_and_permission_waits_do_not_publish_any_result() {
    for question in [false, true] {
        let mut core = AgentCore::try_new(if question {
            durable_question_config()
        } else {
            engineering_core_config()
        })
        .unwrap();
        let started = core
            .submit(user_input(
                "wait-source",
                "Perform the requested work",
                AgentInputDelivery::CurrentTurn,
            ))
            .unwrap();
        let call = if question {
            named_tool_call(
                "question",
                "request_user_input",
                durable_question_arguments("scope"),
            )
        } else {
            tool_call("protected")
        };
        let waiting = submit_response(
            &mut core,
            &model_request(&started.host_requests),
            1,
            empty_model_completion(vec![call]),
        );
        assert!(!waiting.event_envelopes.iter().any(|envelope| matches!(
            &envelope.event,
            AgentEvent::RunResultDecided { .. }
                | AgentEvent::TurnCompleted { .. }
                | AgentEvent::TurnFailed { .. }
        )));
        assert_eq!(
            waiting.snapshot.turn_status,
            if question {
                nola_domain::AgentTurnStatus::AwaitingInput
            } else {
                nola_domain::AgentTurnStatus::AwaitingPermission
            }
        );
        assert!(checkpoint_after(&core, &waiting)
            .state
            .execution
            .requirements
            .decision
            .is_none());
    }
}

#[test]
fn explicit_cancel_retains_original_command_identity_after_host_confirmation() {
    let mut core = AgentCore::try_new(core_config(vec![])).unwrap();
    let started = core
        .submit(user_input(
            "cancel-input",
            "Explain",
            AgentInputDelivery::CurrentTurn,
        ))
        .unwrap();
    let model = model_request(&started.host_requests);
    let interrupted = core
        .submit(AgentSubmission::Interrupt {
            goal_control: None, cancellation_id: "original-command".into(),
            reason: Some("stop".into()),
        })
        .unwrap();
    let completed = confirm_cancellation(&mut core, &interrupted, &model.effect_id);
    assert!(completed.event_envelopes.iter().any(|envelope|matches!(&envelope.event,AgentEvent::RunResultDecided{decision,..}
        if matches!(&decision.value,RunResultValue::Cancelled{cancellation_id,..} if cancellation_id=="original-command"))));
}

#[test]
fn ordinary_text_stop_emits_answered_not_action_acceptance_without_extra_sampling() {
    let mut core = AgentCore::try_new(core_config(vec![])).unwrap();
    let start = core
        .submit(user_input(
            "ordinary-input",
            "你好",
            AgentInputDelivery::CurrentTurn,
        ))
        .unwrap();
    assert!(start
        .event_envelopes
        .iter()
        .map(|envelope| &envelope.event)
        .any(|event| matches!(event, AgentEvent::RequirementScopeChanged { .. })));
    let request = model_request(&start.host_requests);
    let completed = submit_response(
        &mut core,
        &request,
        1,
        visible_model_completion("ordinary-answer", "你好"),
    );
    assert!(completed.event_envelopes.iter().any(|envelope|matches!(&envelope.event,AgentEvent::RunResultDecided{decision,..}
        if matches!(&decision.value,RunResultValue::Answered{answer_part_refs} if answer_part_refs==&vec!["ordinary-answer".to_string()]))));
    assert!(!completed
        .host_requests
        .iter()
        .any(|request| matches!(request.payload, HostRequest::ModelStream { .. })));
}

#[test]
fn mixed_requirement_control_and_host_call_has_no_dispatch_or_partial_scope() {
    let mut core = AgentCore::try_new(core_config(vec![])).unwrap();
    let start = core
        .submit(user_input(
            "mixed-input",
            "Inspect current directory",
            AgentInputDelivery::CurrentTurn,
        ))
        .unwrap();
    let before = checkpoint_after(&core, &start)
        .state
        .execution
        .requirements
        .scope
        .clone();
    let request = model_request(&start.host_requests);
    let rejected = submit_response(
        &mut core,
        &request,
        1,
        empty_model_completion(vec![
            named_tool_call("control", "requirements_update", json!({})),
            named_tool_call("host", "read_file", json!({"path":"."})),
        ]),
    );
    assert!(!rejected.event_envelopes.iter().any(|envelope| matches!(
        &envelope.event,
        AgentEvent::RequirementScopeChanged { .. }
            | AgentEvent::RequirementAssessed { .. }
            | AgentEvent::RunResultDecided { .. }
    )));
    assert!(!rejected
        .host_requests
        .iter()
        .any(|request| matches!(request.payload, HostRequest::ExecuteTool { .. })));
    assert_eq!(
        before,
        checkpoint_after(&core, &rejected)
            .state
            .execution
            .requirements
            .scope
    );
}

#[test]
fn control_answer_is_published_once_with_real_part_and_model_source() {
    let mut core = AgentCore::try_new(core_config(vec![])).unwrap();
    let start = core
        .submit(user_input(
            "control-input",
            "Explain simply",
            AgentInputDelivery::CurrentTurn,
        ))
        .unwrap();
    let arguments = json!({
        "outcome":"answered","answer":"A short explanation."});
    let request = model_request(&start.host_requests);
    let completed = submit_response(
        &mut core,
        &request,
        1,
        empty_model_completion(vec![named_tool_call(
            "final-control",
            "requirements_update",
            arguments,
        )]),
    );
    let decision = completed
        .event_envelopes
        .iter()
        .map(|envelope| &envelope.event)
        .find_map(|event| match event {
            AgentEvent::RunResultDecided { decision, .. } => Some(decision),
            _ => None,
        })
        .unwrap();
    let RunResultValue::Answered { answer_part_refs } = &decision.value else {
        panic!("expected answered")
    };
    assert_eq!(answer_part_refs.len(), 1);
    assert!(completed.event_envelopes.iter().any(|envelope| matches!(&envelope.event,
        AgentEvent::PartUpdated { part: AgentPart::Text(text), .. }
            if text.id == answer_part_refs[0] && text.is_complete
                && text.text == "A short explanation.")));
    assert!(
        matches!(&decision.source,nola_domain::core_requirements::CoreDecisionSource::Model{request_id,call_id:Some(call),..} if request_id==&request.request_id && call=="final-control")
    );
    assert!(!completed
        .host_requests
        .iter()
        .any(|request| matches!(request.payload, HostRequest::ModelStream { .. })));
}

#[test]
fn rejected_requirement_control_survives_restore_until_corrected() {
    for malformed in ["scope", "answer", "mixed"] {
        for correct in [false, true] {
            let config = core_config(vec![]);
            let mut core = AgentCore::try_new(config.clone()).unwrap();
            let start = core.submit(user_input("input", "Explain simply",
                AgentInputDelivery::CurrentTurn)).unwrap();
            let valid = json!({
                "outcome":"answered",
                "answer":"Explanation."});
            let mut invalid = valid.clone();
            if malformed == "scope" { invalid["scope_id"] = json!("wrong-scope"); }
            if malformed == "answer" { invalid.as_object_mut().unwrap().remove("answer"); }
            let mut calls = vec![named_tool_call("rejected-control", "requirements_update", invalid)];
            if malformed == "mixed" {
                calls.push(named_tool_call("not-dispatched", "read_file", json!({"path":"."})));
            }
            let rejected = submit_response(&mut core, &model_request(&start.host_requests), 1,
                empty_model_completion(calls));
            assert!(!rejected.host_requests.iter().any(|request|
                matches!(request.payload, HostRequest::ExecuteTool { .. })));
            let request = model_request(&rejected.host_requests);
            let checkpoint = core.checkpoint(CheckpointReason::Manual);
            let encoded = serde_json::to_value(&checkpoint).unwrap();
            assert_eq!(encoded["state"]["execution"]["requirements"]["control_recovery"]["kind"], "rejected");
            core = AgentCore::restore(config, serde_json::from_value(encoded).unwrap()).unwrap();
            let response = if correct {
                empty_model_completion(vec![named_tool_call("corrected-control", "requirements_update", valid)])
            } else {
                visible_model_completion("plain-answer", "Explanation.")
            };
            let finished = submit_response(&mut core, &request, 1, response);
            let decision = finished.event_envelopes.iter().find_map(|envelope| match &envelope.event {
                AgentEvent::RunResultDecided { decision, .. } => Some(decision), _ => None,
            }).expect("normal completion must commit its result");
            if correct {
                assert!(matches!(decision.value, RunResultValue::Answered { .. }));
            } else {
                assert!(matches!(&decision.value, RunResultValue::Incomplete { reason, .. }
                    if reason.code == "protocol.recovery_pending"));
                assert!(matches!(&decision.source,
                    nola_domain::core_requirements::CoreDecisionSource::Runtime { source_refs, .. }
                    if !source_refs.is_empty()));
            }
            assert!(!finished.host_requests.iter().any(|request|
                matches!(request.payload, HostRequest::ModelStream { .. })));
        }
    }
}
