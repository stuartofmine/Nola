// SPDX-License-Identifier: Apache-2.0
//! Partition 6 of `agent_core_host_contract` scenarios.

use super::*;

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
