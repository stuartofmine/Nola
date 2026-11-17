// SPDX-License-Identifier: Apache-2.0
//! Partition 2 of `agent_core_host_contract` scenarios.

use super::*;

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
