// SPDX-License-Identifier: Apache-2.0
//! Real Application UoW task admission, immutable Run identity and terminal recovery.
mod support;
use nola_application::{
    ApplicationCommand, BeginAssistantTurn, RuntimeTaskCommand, RuntimeTaskLeaseWitness,
    RuntimeTaskRunBinding,
};
use nola_agent_engine::agent::host::*;
use nola_agent_engine::agent::{
    AgentCoreConfig, AgentInputDelivery, AgentItem, AgentSubmission, ItemId, SessionId,
};
use serde_json::json;

#[test]
fn automation_occurrence_uses_atomic_exact_admission_and_never_exit_code_result() {
    use nola_domain::automation_execution::*;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("automation-admission.sqlite");
    let mut store = nola_store::NolaStore::open(&path).unwrap();
    let definition = store.upsert_automation_definition(&nola_store::StoredAutomationDefinition {
        execution_spec: serde_json::from_value(serde_json::json!({"kind":"agent","spec":{"prompt":"Execute this task once","execution_scope":{"kind":"local_workspace","root":"/fixture"},"permission_profile":"cautious","model_selection":"use_current_default_at_admission","context_refs":[],"skill_refs":[]}})).unwrap(),
        id: "automation".into(), name: "Automation".into(), schedule_format: "fixture".into(),
        enabled: true, revision: 0, metadata_json: "{}".into(), created_at_ms: 800, updated_at_ms: 800, deleted_at_ms: None,
    }).unwrap();
    store.enqueue_automation_run(&nola_store::StoredAutomationRun {
        definition_revision: 1,
        id: "occurrence".into(), automation_id: definition.id.clone(), occurrence_key: "slot".into(), trigger: "schedule".into(),
        status: "queued".into(), attempt: 0, worker_id: None, lease_token: None, lease_until_ms: None,
        scheduled_for_ms: 900, started_at_ms: None, finished_at_ms: None, next_retry_at_ms: None,
        result_json: None, error_json: None, created_at_ms: 900, updated_at_ms: 900,
    }).unwrap();
    assert!(store.claim_automation_run("occurrence", "worker", "lease", 900, 2000).unwrap());
    drop(store);
    let lease = AutomationExecutionLease { occurrence_id: "occurrence".into(), definition_revision: definition.revision,
        worker_id: "worker".into(), lease_token: "lease".into() };
    let identity = AutomationExecutionIdentity::for_occurrence("occurrence");
    let mut input = begin();
    input.core_config.automation_execution_spec = Some(definition.execution_spec.agent().unwrap().clone());
    input.conversation_id = identity.conversation_id.clone();
    input.user_message_id = identity.user_message_id.clone();
    input.assistant_message_id = identity.assistant_message_id.clone();
    if let AgentSubmission::UserInput { submission_id, items, .. } = &mut input.submission {
        *submission_id = Some(identity.submission_id.clone());
        *items = vec![AgentItem::user_message(ItemId::from_string(identity.user_message_id.clone()), "Execute this task once")];
    }
    let mut runtime = support::open_runtime(&path).unwrap();
    let mut stale = lease.clone(); stale.definition_revision += 1;
    assert!(runtime.execute(ApplicationCommand::BeginAutomationAssistantTurn { lease: stale, input: input.clone() }).is_err());
    let before: AutomationExecutionObservation = serde_json::from_value(runtime.execute(
        ApplicationCommand::ObserveAutomationExecution { occurrence_id: "occurrence".into() }).unwrap()).unwrap();
    assert!(before.binding.is_none());
    let inspect = rusqlite::Connection::open(&path).unwrap();
    inspect.execute_batch("CREATE TRIGGER reject_automation_binding BEFORE INSERT ON automation_execution_bindings BEGIN SELECT RAISE(ABORT,'fixture binding failure'); END;").unwrap();
    assert!(runtime.execute(ApplicationCommand::BeginAutomationAssistantTurn { lease: lease.clone(), input: input.clone() }).is_err());
    let leaked: i64 = inspect.query_row("SELECT COUNT(*) FROM runs WHERE conversation_id=?1", [&identity.conversation_id], |row|row.get(0)).unwrap();
    assert_eq!(leaked, 0, "binding failure rolls back Run, message and initial effect admission");
    inspect.execute_batch("DROP TRIGGER reject_automation_binding;").unwrap();
    drop(runtime);
    let mut runtime = support::open_runtime(&path).unwrap();
    let accepted: nola_application::AcceptedAssistantTurn = serde_json::from_value(runtime.execute(
        ApplicationCommand::BeginAutomationAssistantTurn { lease: lease.clone(), input: input.clone() }).unwrap()).unwrap();
    let replay: nola_application::AcceptedAssistantTurn = serde_json::from_value(runtime.execute(
        ApplicationCommand::BeginAutomationAssistantTurn { lease: lease.clone(), input: input.clone() }).unwrap()).unwrap();
    assert_eq!(replay.begin.run_id, accepted.begin.run_id);
    // Admission lease expiry does not authorize another worker to dispatch
    // the occurrence again. The bound canonical Run now owns execution.
    let mut competing = nola_store::NolaStore::open(&path).unwrap();
    assert!(!competing.claim_automation_run("occurrence", "second-worker", "second-lease", 2001, 3000).unwrap());
    assert_eq!(competing.observe_automation_execution("occurrence").unwrap().binding.unwrap().run_id, accepted.begin.run_id);
    drop(competing);
    assert_eq!(runtime.execute(ApplicationCommand::ObserveAutomationWorker { lease: lease.clone(),
        observation: AutomationWorkerObservation::LaunchFailed { detail: "Host lost the admission ACK".into() }, observed_at_ms: 1080 }).unwrap(), json!(true));
    let after_unknown_ack: AutomationExecutionObservation = serde_json::from_value(runtime.execute(
        ApplicationCommand::ObserveAutomationExecution { occurrence_id: "occurrence".into() }).unwrap()).unwrap();
    assert_eq!(after_unknown_ack.phase, AutomationExecutionPhase::Admitted);
    assert_eq!(after_unknown_ack.binding.unwrap().run_id, accepted.begin.run_id);
    assert!(after_unknown_ack.terminal.is_none(), "A transport exception after admission cannot fabricate a terminal failure");
    let mut changed = input.clone(); changed.user_text.push_str(" changed");
    assert!(runtime.execute(ApplicationCommand::BeginAutomationAssistantTurn { lease: lease.clone(), input: changed }).is_err());
    assert!(runtime.execute(ApplicationCommand::BeginAssistantTurn { input }).is_err());
    assert_eq!(runtime.execute(ApplicationCommand::ObserveAutomationWorker { lease,
        observation: AutomationWorkerObservation::Exited { exit_code: 0 }, observed_at_ms: 1100 }).unwrap(), json!(true));
    assert_eq!(runtime.execute(ApplicationCommand::ReconcileAutomationExecution { occurrence_id: "occurrence".into(), reconciled_at_ms: 1200 }).unwrap(), json!(null));
    drop(runtime);
    let reopened = nola_store::NolaStore::open(&path).unwrap();
    let observed = reopened.observe_automation_execution("occurrence").unwrap();
    assert_eq!(observed.binding.unwrap().run_id, accepted.begin.run_id);
    assert!(observed.terminal.is_none());
    assert!(observed.receipt.is_none());
    assert!(!observed.notification_delivered);
    drop(reopened);
    let mut runtime = support::open_runtime(&path).unwrap();
    for round in 0..12 {
        let now = 1200 + round * 4;
        let claimed: nola_application::CommittedHostRequests = serde_json::from_value(runtime.execute(
            ApplicationCommand::ClaimCommittedHostRequests { input: nola_application::ClaimCommittedHostRequests {
                run_id: accepted.begin.run_id.clone(), worker_generation: "automation-generation".into(),
                lease_owner: "automation-worker".into(), claimed_at_ms: now, lease_expires_at_ms: 2500, limit: 1,
            }}).unwrap()).unwrap();
        let Some(request) = claimed.requests.into_iter().next() else { break; };
        let mut sequence = request.resume_after_sequence.unwrap_or(0);
        for (index, payload) in [HostResponse::ExecutionStarted, HostResponse::Failed { error: HostError {
            code: "fixture_failure".into(), message: "real Host terminal failure".into(), retryable: false, http_status: None,
        }}].into_iter().enumerate() {
            sequence += 1;
            runtime.execute(ApplicationCommand::ReduceAndAcceptHostResponse {
                input: nola_application::ReduceAndAcceptHostResponse {
                    conversation_id: identity.conversation_id.clone(), run_id: accepted.begin.run_id.clone(),
                    invocation_config_digest: accepted.invocation_config_digest.clone(), received_at_ms: now + index as i64 + 1,
                    response: HostResponseEnvelope {
                        attempt_id: request.attempt_id.clone(), delegated_invocation_id: request.delegated_invocation_id.clone(),
                        request_id: request.request_id.clone(), effect_id: request.effect_id.clone(), host_id: request.host_id.clone(),
                        lease_id: request.lease_id.clone(), session_id: request.session_id.clone(), turn_id: request.turn_id.clone(),
                        catalog_revision: request.catalog_revision, availability_epoch: request.availability_epoch,
                        delivery: request.delivery.clone(), sequence, payload,
                    },
                },
            }).unwrap();
        }
    }
    let receipt: Option<AutomationExecutionReceipt> = serde_json::from_value(runtime.execute(
        ApplicationCommand::ReconcileAutomationExecution { occurrence_id: "occurrence".into(), reconciled_at_ms: 3000 }).unwrap()).unwrap();
    let receipt = receipt.expect("real Host failure seals a result; process exit did not");
    assert!(matches!(receipt.terminal.result.value, nola_domain::run_result::RunResultValue::Failed { .. }));
    let replay: Option<AutomationExecutionReceipt> = serde_json::from_value(runtime.execute(
        ApplicationCommand::ReconcileAutomationExecution { occurrence_id: "occurrence".into(), reconciled_at_ms: 4000 }).unwrap()).unwrap();
    assert_eq!(replay, Some(receipt.clone()));
    assert_eq!(runtime.execute(ApplicationCommand::AckAutomationNotification {
        occurrence_id: "occurrence".into(), notification_id: receipt.notification_id.clone(), delivered_at_ms: 2999 }).unwrap(), json!(false));
    assert_eq!(runtime.execute(ApplicationCommand::ListPendingAutomationExecutions { after_occurrence_id: None, limit: 10 }).unwrap(), json!(["occurrence"]));
    assert_eq!(runtime.execute(ApplicationCommand::AckAutomationNotification {
        occurrence_id: "occurrence".into(), notification_id: receipt.notification_id, delivered_at_ms: 4000 }).unwrap(), json!(true));
    assert_eq!(runtime.execute(ApplicationCommand::ListPendingAutomationExecutions { after_occurrence_id: None, limit: 10 }).unwrap(), json!([]));
}

fn begin() -> BeginAssistantTurn {
    let capability = HostCapabilityDescriptor {
        id: "model.stream".into(),
        title: "fixture model".into(),
        route: HostCapabilityRoute::Model,
        availability: CapabilityAvailability::Available,
        exposure: HostCapabilityExposure::Internal,
        permission_domain: None,
        input_schema: json!({"type":"object"}),
        input_normalization: Default::default(),
        output_policy: HostOutputPolicy {
            preview_char_limit: 4000,
            persist_full_output: true,
            allow_artifacts: true,
        },
        supports_streaming: true,
        supports_argument_streaming: false,
        parallelism: HostCapabilityParallelism::Serial,
        cancellation: HostCancellationMode::Cooperative,
        replay_safety: HostReplaySafety::Recoverable,
    };
    BeginAssistantTurn {
        expected_configuration_revision: None,
        expected_parent_message_id: None,
        conversation_id: "child".into(),
        conversation_title: "Child task".into(),
        user_message_id: "user".into(),
        user_part_id: "user-body".into(),
        user_text: "Execute this task once".into(),
        user_content_json: None,
        assistant_message_id: "assistant".into(),
        context_checkpoint_id: None,
        core_config: AgentCoreConfig {
            turn_identity: support::model_identity::identity(),
            session_id: Some(SessionId::from_string("actual-child-core".into())),
            model: Some("fixture-model".into()),
            host_catalog: Some(HostCapabilityCatalog {
                host_id: HostId("fixture-host".into()),
                revision: 1,
                availability_epoch: 1,
                lease: None,
                capabilities: [("model.stream".into(), capability)].into_iter().collect(),
            }),
            ..Default::default()
        },
        submission: AgentSubmission::UserInput {
            items: vec![AgentItem::user_message(
                ItemId::from_string("user".into()),
                "Execute this task once",
            )],
            turn_context_items: vec![],
            turn_context_platform: Default::default(),
            turn_context_components: vec![],
            submission_id: Some("task-admission".into()),
            submitted_at_epoch_ms: Some(1000),
            delivery: AgentInputDelivery::CurrentTurn,
        },
        created_at_ms: 1000,
    }
}

fn seed(path: &std::path::Path) -> RuntimeTaskLeaseWitness {
    seed_with_budget(path, None)
}

fn seed_with_budget(
    path: &std::path::Path,
    budget: Option<nola_application::RuntimeTaskResourceBudget>,
) -> RuntimeTaskLeaseWitness {
    let mut runtime = support::open_runtime(path).unwrap();
    support::agent_task_admission::draft(&mut runtime, budget);
    let admission = support::agent_task_admission::source_fact(&mut runtime);
    let authority: nola_domain::agent_task_admission::RuntimeAgentTaskAuthority =
        serde_json::from_value(runtime.execute(ApplicationCommand::ImportHostAgentTaskAdmission {
            admission: admission.clone(),
        }).unwrap()).unwrap();
    assert_eq!(authority.admission.as_ref(), Some(&admission));
    drop(runtime);
    let mut store = nola_store::NolaStore::open(path).unwrap();
    let attempt = store.claim_runtime_task("task", "worker", "generation", "token", 900, 10000)
        .unwrap().expect("a real committed source admission permits the worker claim");
    RuntimeTaskLeaseWitness::from(&attempt)
}

fn read_binding(
    runtime: &mut nola_application::ApplicationRuntime<nola_store::NolaStore>,
) -> Option<RuntimeTaskRunBinding> {
    serde_json::from_value(
        runtime
            .execute(ApplicationCommand::RuntimeTask {
                command: RuntimeTaskCommand::RunBinding {
                    task_id: "task".into(),
                },
            })
            .unwrap(),
    )
    .unwrap()
}

#[test]
fn first_run_binding_cannot_adopt_preexisting_permission_authority() {
    for during_admission in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("permission-before-binding.sqlite");
        let lease = seed(&path);
        let inspect = rusqlite::Connection::open(&path).unwrap();
        if during_admission {
            // Exercise the binding transaction's second check, not only the
            // preflight snapshot: this row must roll back with the new Run.
            inspect.execute_batch("CREATE TRIGGER insert_prior_permission AFTER INSERT ON runs
                WHEN NEW.conversation_id='child' BEGIN
                INSERT INTO permission_requests(request_id,task_id,child_session_id,capability,
                    input_json,scope_rule,status,created_at_ms)
                VALUES('prior','task','child','file.write','{}','fixture','pending',950);
                END;").unwrap();
        } else {
            let mut store = nola_store::NolaStore::open(&path).unwrap();
            store.enqueue_permission_request(&nola_store_port::permission::StoredPermissionRequest {
                request_id: "prior".into(), task_id: "task".into(), child_session_id: "child".into(),
                capability: "file.write".into(), input_json: "{}".into(), scope_rule: "fixture".into(),
                status: "pending".into(), created_at_ms: 950, decision: None, custom_response: None,
                resolved_at_ms: None, core_permission_request_id: Some("unproven-core-id".into()),
                permission_instructions_json: "[]".into(), closure: None,
            }).unwrap();
        }
        let before = nola_store::NolaStore::open(&path).unwrap().runtime_task("task").unwrap().unwrap();
        let mut runtime = support::open_runtime(&path).unwrap();
        assert!(runtime.execute(ApplicationCommand::BeginTaskAssistantTurn {
            lease: lease.clone(), input: begin(),
        }).is_err());
        assert!(read_binding(&mut runtime).is_none());
        let store = nola_store::NolaStore::open(&path).unwrap();
        assert_eq!(store.runtime_task("task").unwrap().unwrap(), before);
        let runs: i64 = inspect.query_row("SELECT COUNT(*) FROM runs WHERE conversation_id='child'", [], |row| row.get(0)).unwrap();
        assert_eq!(runs, 0, "rejected binding must not leave a child Run or its execution authority");
        assert_eq!(store.permission_request("prior").unwrap().is_some(), !during_admission);
        if during_admission {
            inspect.execute_batch("DROP TRIGGER insert_prior_permission").unwrap();
            // The unchanged original lease remains usable after rollback.
            runtime.execute(ApplicationCommand::BeginTaskAssistantTurn { lease, input: begin() }).unwrap();
            assert!(read_binding(&mut runtime).is_some());
        }
    }
}

#[test]
fn retired_promotion_allows_terminal_parent_import_and_successor_run() {
    for child_status in ["completed", "cancelled"] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("handoff.sqlite");
        let lease = seed(&path);
        let mut runtime = support::open_runtime(&path).unwrap();
        runtime.execute(ApplicationCommand::BeginTaskAssistantTurn { lease, input: begin() }).unwrap();
        let binding = read_binding(&mut runtime).unwrap();
        finish_child(&mut runtime, &binding, child_status);
        let mut store = nola_store::NolaStore::open(&path).unwrap();
        let parent = store.active_run("parent").unwrap().unwrap();
        let digest = parent.invocation_config_digest.unwrap();
        let mut request = nola_application::load_committed_host_requests(&store, &parent.run_id).unwrap()
            .requests.into_iter().find(|request| matches!(request.payload, HostRequest::ModelStream { .. })).unwrap();
        support::admit_model_sampling_for_conversation(&mut store, "parent", &parent.run_id, &digest, &mut request, 1210);
        runtime = support::open_runtime(&path).unwrap();
        runtime.execute(ApplicationCommand::ReduceAndAcceptHostResponse { input: nola_application::ReduceAndAcceptHostResponse {
            conversation_id: "parent".into(), run_id: parent.run_id.clone(), invocation_config_digest: digest,
            received_at_ms: 1220, response: HostResponseEnvelope {
                attempt_id: request.attempt_id.clone(), delegated_invocation_id: request.delegated_invocation_id.clone(),
                request_id: request.request_id.clone(), effect_id: request.effect_id.clone(), host_id: request.host_id.clone(),
                lease_id: request.lease_id.clone(), session_id: request.session_id.clone(), turn_id: request.turn_id.clone(),
                catalog_revision: request.catalog_revision, availability_epoch: request.availability_epoch, delivery: request.delivery.clone(),
                sequence: request.resume_after_sequence.unwrap_or(0)+1,
                payload: HostResponse::ModelCompleted { items: vec![serde_json::to_value(AgentItem::assistant_message(
                    ItemId::from_string("parent-final".into()), "Foreground finished after durable handoff.")).unwrap()],tool_calls:vec![],usage:json!({}) },
            },
        }}).unwrap();
        assert!(store.active_run("parent").unwrap().is_none());
        let before = support::current_checkpoint(&path, "parent");
        let checkpoint: nola_agent_engine::agent::recovery::CoreCheckpoint<nola_agent_engine::agent::AgentThreadState> = serde_json::from_str(&before.checkpoint_json).unwrap();
        assert_eq!(checkpoint.state.host.pending_effects.len(), 0);
        assert!(checkpoint.state.permits_new_turn_execution_configuration());
        assert_eq!(runtime.execute(ApplicationCommand::RuntimeTask { command: RuntimeTaskCommand::ReconcileBoundRunTerminal {
            task_id:"task".into(),expected_run_id:binding.run_id.clone(),now_ms:1300,
        }}).unwrap(),json!(true));
        let receipt=store.runtime_task_terminal_receipt("task").unwrap().unwrap();
        let projection:nola_domain::agent_task_admission::AgentTaskTerminalProjection=serde_json::from_value(runtime.execute(
            ApplicationCommand::ExportAgentTaskTerminalProjection {task_id:"task".into(),expected_receipt_id:receipt.receipt_id.clone(),guidance_message_ids:vec![]}
        ).unwrap()).unwrap();
        // Cold idle import must succeed without reopening the completed Run or
        // resurrecting the retired foreground ExecuteTool request.
        runtime=support::open_runtime(&path).unwrap();
        for _ in 0..2 {
            runtime.execute(ApplicationCommand::ImportHostAgentTaskTerminalProjection {projection:projection.clone()}).unwrap();
        }
        assert!(store.active_run("parent").unwrap().is_none());
        let settled = support::current_checkpoint(&path,"parent");
        let settled:nola_agent_engine::agent::recovery::CoreCheckpoint<nola_agent_engine::agent::AgentThreadState>=serde_json::from_str(&settled.checkpoint_json).unwrap();
        assert_eq!(settled.state.host.pending_effects.len(), 0);
        assert!(settled.state.permits_new_turn_execution_configuration());
        assert!(settled.state.task_projection.tasks["task"].state.is_terminal());
        let mut next=begin();next.conversation_id="parent".into();next.user_message_id="next-user".into();
        let frontier = support::admission_frontier::read(&store, "parent");
        next.expected_configuration_revision = frontier.0;
        next.expected_parent_message_id = frontier.1;
        next.user_text="A genuinely new request".into();next.conversation_title="Parent".into();
        next.user_part_id="next-user-body".into();next.assistant_message_id="next-assistant".into();
        next.core_config.session_id=Some(SessionId::from_string("actual-parent-core".into()));next.created_at_ms=1500;
        next.submission=AgentSubmission::UserInput {items:vec![AgentItem::user_message(ItemId::from_string("next-user".into()),"A genuinely new request")],
            turn_context_items:vec![],turn_context_platform:Default::default(),turn_context_components:vec![],submission_id:Some("next-run".into()),
            submitted_at_epoch_ms:Some(1500),delivery:AgentInputDelivery::CurrentTurn};
        let accepted:nola_application::AcceptedAssistantTurn=serde_json::from_value(runtime.execute(ApplicationCommand::BeginAssistantTurn {input:next}).unwrap()).unwrap();
        assert_ne!(accepted.begin.run_id,parent.run_id);
        assert_eq!(store.runtime_task_run_binding("task").unwrap(),Some(binding));
    }
}

fn finish_child(
    runtime: &mut nola_application::ApplicationRuntime<nola_store::NolaStore>,
    bound: &RuntimeTaskRunBinding,
    status: &str,
) {
    finish_child_with_text(runtime, bound, status, "真实结果🙂");
}

fn finish_child_with_text(
    runtime: &mut nola_application::ApplicationRuntime<nola_store::NolaStore>,
    bound: &RuntimeTaskRunBinding,
    status: &str,
    text: &str,
) -> Vec<HostRequestEnvelope> {
    let mut observed = Vec::new();
    if status == "cancelled" {
        runtime
            .execute(ApplicationCommand::RequestRunCancellation {
                input: nola_application::RequestRunCancellation {
                    conversation_id: "child".into(),
                    expected_run_id: bound.run_id.clone(),
                    assistant_message_id: "assistant".into(),
                    cancellation_id: "fixture-cancel".into(),
                    reason: None,
                    requested_at_ms: 1100,
                },
            })
            .unwrap();
    }
    let active: Option<nola_application::ActiveRun> = serde_json::from_value(
        runtime
            .execute(ApplicationCommand::ActiveRun {
                conversation_id: "child".into(),
            })
            .unwrap(),
    )
    .unwrap();
    let Some(active) = active else {
        assert_eq!(status, "cancelled");
        return observed;
    };
    for round in 0..12 {
        let now = bound.bound_at_ms + 100 + round * 4;
        let claimed: nola_application::CommittedHostRequests = serde_json::from_value(
            runtime
                .execute(ApplicationCommand::ClaimCommittedHostRequests {
                    input: nola_application::ClaimCommittedHostRequests {
                        run_id: bound.run_id.clone(),
                        worker_generation: "fixture-generation".into(),
                        lease_owner: "fixture-host-worker".into(),
                        claimed_at_ms: now,
                        lease_expires_at_ms: 2000,
                        limit: 1,
                    },
                })
                .unwrap(),
        )
        .unwrap();
        let Some(request) = claimed.requests.into_iter().next() else {
            break;
        };
        observed.push(request.clone());
        let mut sequence = request.resume_after_sequence.unwrap_or(0);
        let responses = if let HostRequest::Cancel { target_effect_id, .. } = &request.payload {
            vec![
                HostResponse::ExecutionStarted,
                HostResponse::CancellationConfirmed {
                    target_effect_id: target_effect_id.clone(),
                },
            ]
        } else {
            let terminal = if status == "failed" {
                HostResponse::Failed {
                    error: HostError {
                        code: "fixture_failure".into(),
                        message: "Actual Host failure".into(),
                        retryable: false,
                        http_status: None,
                    },
                }
            } else {
                HostResponse::ModelCompleted {
                    items: vec![serde_json::to_value(AgentItem::assistant_message(
                        ItemId::from_string(format!("child-result:{}", bound.run_id)),
                        text,
                    ))
                    .unwrap()],
                    tool_calls: vec![],
                    usage: json!({}),
                }
            };
            vec![
                HostResponse::ExecutionStarted,
                HostResponse::ModelAttemptStarted {
                    provider: "fixture".into(),
                    model: "fixture-model".into(),
                    reason: "initial".into(),
                },
                terminal,
            ]
        };
        for (index, payload) in responses.into_iter().enumerate() {
            sequence += 1;
            runtime
                .execute(ApplicationCommand::ReduceAndAcceptHostResponse {
                    input: nola_application::ReduceAndAcceptHostResponse {
                        conversation_id: "child".into(),
                        run_id: bound.run_id.clone(),
                        invocation_config_digest: active.invocation_config_digest.clone().unwrap(),
                        response: HostResponseEnvelope {
                            attempt_id: request.attempt_id.clone(),
                            delegated_invocation_id: request.delegated_invocation_id.clone(),
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
                        },
                        received_at_ms: now + index as i64 + 1,
                    },
                })
                .unwrap();
        }
    }
    observed
}

fn task_guidance_metadata(path: &std::path::Path) -> (nola_domain::assistant_journal::AssistantJournalCursor, bool) {
    use nola_store_port::assistant_journal::AssistantJournalRepository;
    let store=nola_store::NolaStore::open(path).unwrap();
    let parent=store.active_run("parent").unwrap().unwrap();
    let identity=nola_domain::assistant_journal::AssistantJournalIdentity {
        source_store_epoch:store.assistant_source_epoch().unwrap(),conversation_id:"parent".into(),
        run_id:parent.run_id,assistant_message_id:parent.assistant_message_id.unwrap(),
    };
    let nola_store_port::assistant_journal::AssistantJournalPartState::Live(head)=store.assistant_journal_part_state(&identity,"task:task").unwrap() else { panic!("production Task card missing") };
    let nola_domain::assistant_journal_operations::JournalPartMetadata::TaskControl(metadata)=head.metadata else { panic!("Task controls missing") };
    (store.assistant_journal_head(&identity).unwrap(), metadata.supports_guidance)
}

#[test]
fn guidance_capability_changes_only_when_actual_child_binding_commits() {
    let root=tempfile::tempdir().unwrap(); let path=root.path().join("guidance-binding.sqlite");
    let lease=seed(&path);
    let (before,available)=task_guidance_metadata(&path);
    assert!(!available,"claimed worker without child binding cannot accept guidance");
    let mut runtime=support::open_runtime(&path).unwrap();
    let db=rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TRIGGER reject_binding_publication BEFORE INSERT ON assistant_journal_publications WHEN NEW.source_kind='task_run_binding' BEGIN SELECT RAISE(ABORT,'binding publication fault'); END;").unwrap();
    assert!(runtime.execute(ApplicationCommand::BeginTaskAssistantTurn {lease:lease.clone(),input:begin()}).is_err());
    assert!(read_binding(&mut runtime).is_none());
    assert_eq!(task_guidance_metadata(&path),(before.clone(),false),"binding and parent publication roll back together");
    db.execute_batch("DROP TRIGGER reject_binding_publication").unwrap();
    let accepted:nola_application::AcceptedAssistantTurn=serde_json::from_value(runtime.execute(
        ApplicationCommand::BeginTaskAssistantTurn {lease,input:begin()}).unwrap()).unwrap();
    let (after,available)=task_guidance_metadata(&path);
    assert!(available,"binding commit enables the existing parent Task card");
    {
        use nola_store_port::assistant_journal::AssistantJournalRepository;
        let store=nola_store::NolaStore::open(&path).unwrap();
        let nola_store_port::assistant_journal::AssistantJournalPartState::Live(head)=store.assistant_journal_part_state(&after.identity,"task:task").unwrap() else {panic!()};
        let nola_domain::assistant_journal_operations::JournalPartMetadata::TaskControl(metadata)=head.metadata else {panic!()};
        assert_eq!(metadata.task.state,nola_domain::ActivityState::Running,"actual bound execution cannot remain displayed as queued");
    }
    assert!(after.sequence>before.sequence,"parent subscriber has a new durable cursor");
    assert!(accepted.assistant_publications.iter().all(|cursor|
        cursor.identity.conversation_id == "child"),"child transport must not receive foreign parent hints");
    let binding=read_binding(&mut runtime).unwrap();
    use nola_store_port::assistant_journal::*;
    let mut store=nola_store::NolaStore::open(&path).unwrap();
    let page=store.read_assistant_journal_page(&AssistantJournalReadWindow {after:before.clone(),through:after.clone()},16,131072).unwrap();
    let AssistantJournalPage::Records {records,..}=page else { panic!("binding record missing") };
    assert_eq!(records.len(),1);
    let replay=CommitAssistantPublicationSet {
        stamp:AssistantSourceCommitStamp::TaskRunBinding {task_id:"task".into(),child_run_id:binding.run_id.clone()},
        groups:vec![AssistantPublicationGroup {expected:before,frames:vec![AssistantJournalFrame {batch:records[0].batch.clone(),supplied_chunks:vec![]}]}],
    };
    submit_guidance(&mut runtime,"state-must-not-regress",1050);
    pump_guidance_parent(&mut runtime,&path,1051);
    let nola_store_port::assistant_journal::AssistantJournalPartState::Live(head)=store.assistant_journal_part_state(&after.identity,"task:task").unwrap() else {panic!()};
    let nola_domain::assistant_journal_operations::JournalPartMetadata::TaskControl(metadata)=head.metadata else {panic!()};
    assert_eq!(metadata.task.state,nola_domain::ActivityState::Running,"a later parent guidance event must not restore stale queued display state");
    finish_child(&mut runtime,&binding,"completed");
    runtime.execute(ApplicationCommand::RuntimeTask {command:RuntimeTaskCommand::ReconcileBoundRunTerminal {
        task_id:"task".into(),expected_run_id:binding.run_id,now_ms:1300}}).unwrap();
    let receipt=store.runtime_task_terminal_receipt("task").unwrap().unwrap();
    runtime.execute(ApplicationCommand::EnqueueTaskTerminalReconciliation {task_id:"task".into(),expected_receipt_id:receipt.receipt_id}).unwrap();
    pump_guidance_parent(&mut runtime,&path,1400);
    let (terminal,available)=task_guidance_metadata(&path);
    assert!(!available,"terminal task never offers guidance");
    assert!(terminal.sequence>after.sequence);
    store.begin_write_unit_of_work().unwrap();
    assert!(matches!(store.commit_assistant_publication_set(&replay).unwrap(),AssistantPublicationCommit::Replay(_)));
    store.commit_write_unit_of_work().unwrap();
    assert_eq!(task_guidance_metadata(&path),(terminal,false),"old exact replay cannot overwrite newer terminal metadata");
    for foreign in ["task", "run", "metadata"] {
        let mut forged=replay.clone();
        match foreign {
            "task" => forged.stamp=AssistantSourceCommitStamp::TaskRunBinding {task_id:"foreign".into(),child_run_id:accepted.begin.run_id.clone()},
            "run" => forged.stamp=AssistantSourceCommitStamp::TaskRunBinding {task_id:"task".into(),child_run_id:"foreign".into()},
            _ => {
                let nola_domain::assistant_journal_operations::AssistantJournalOperationV1::Update(update)=&mut forged.groups[0].frames[0].batch.operations[0] else {panic!()};
                let nola_domain::assistant_journal_operations::JournalPartMetadata::TaskControl(metadata)=&mut update.metadata else {panic!()};
                metadata.task.display_key="forged".into();
            }
        }
        store.begin_write_unit_of_work().unwrap();
        assert!(store.commit_assistant_publication_set(&forged).is_err(),"accepted {foreign}");
        store.rollback_write_unit_of_work().unwrap();
    }
}

#[test]
fn explicit_task_admission_binds_the_actual_run_and_rejects_generic_or_stale_admission() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("admission.sqlite");
    let lease = seed(&path);
    let mut runtime = support::open_runtime(&path).unwrap();
    assert!(runtime
        .execute(ApplicationCommand::BeginAssistantTurn { input: begin() })
        .unwrap_err()
        .contains("explicit"));
    let mut stale = lease.clone();
    stale.expected_attempt_revision += 1;
    assert!(runtime
        .execute(ApplicationCommand::BeginTaskAssistantTurn {
            lease: stale,
            input: begin()
        })
        .is_err());
    assert!(read_binding(&mut runtime).is_none());
    let accepted: nola_application::AcceptedAssistantTurn = serde_json::from_value(
        runtime
            .execute(ApplicationCommand::BeginTaskAssistantTurn {
                lease: lease.clone(),
                input: begin(),
            })
            .unwrap(),
    )
    .unwrap();
    let bound = read_binding(&mut runtime).unwrap();
    assert_eq!(bound.run_id, accepted.begin.run_id);
    assert_eq!(bound.child_session_id, "child");
    assert_eq!(bound.owner_session_id, "parent");
    assert_eq!(bound.admission_attempt_id, lease.attempt_id);
    assert_eq!(bound.input_watermark, Some(7));
    assert!(runtime
        .execute(ApplicationCommand::BeginTaskAssistantTurn {
            lease,
            input: begin()
        })
        .is_err());
    drop(runtime);
    let mut runtime = support::open_runtime(&path).unwrap();
    assert_eq!(read_binding(&mut runtime), Some(bound));
}

#[test]
fn task_admission_requires_exact_admitted_identity_and_current_attempt() {
    for field in ["id", "kind", "owner_session_id", "child_session_id", "parent_tool_call_id", "input_watermark"] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("immutable-task-fence.sqlite");
        let lease = seed(&path);
        let store = nola_store::NolaStore::open(&path).unwrap();
        let snapshot = || (store.runtime_task("task").unwrap(), store.runtime_agent_task_authority("task").unwrap(),
            store.runtime_task_attempt(&lease.attempt_id).unwrap());
        let before = snapshot();
        let db = rusqlite::Connection::open(&path).unwrap();
        let value = if field == "input_watermark" { rusqlite::types::Value::Integer(6) }
            else { rusqlite::types::Value::Text("foreign".into()) };
        let error = db.execute(&format!("UPDATE durable_tasks SET {field}=?1 WHERE id='task'"), [value]).unwrap_err();
        assert_eq!(error.to_string(), "agent task identity is immutable", "{field}");
        assert_eq!(snapshot(), before, "{field} changed Task, admission, or attempt");
        assert!(store.runtime_task_run_binding("task").unwrap().is_none());
        let runs: i64 = db.query_row("SELECT count(*) FROM runs WHERE conversation_id='child'", [], |row| row.get(0)).unwrap();
        assert_eq!(runs, 0);
        let mut runtime = support::open_runtime(&path).unwrap();
        runtime.execute(ApplicationCommand::BeginTaskAssistantTurn { lease, input: begin() }).unwrap();
    }
    for fault in ["missing", "token", "generation", "attempt_revision", "expiry", "worker", "state", "cancelled"] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("dynamic-task-fence.sqlite");
        let lease = seed(&path);
        let mut candidate = lease.clone();
        let mut runtime = support::open_runtime(&path).unwrap();
        let db = rusqlite::Connection::open(&path).unwrap();
        let mutation = match fault {
            "missing" => { candidate.task_id = "missing".into(); None },
            "token" => { candidate.lease_token = "foreign".into(); None },
            "generation" => { candidate.worker_generation = "foreign".into(); None },
            "attempt_revision" => { candidate.expected_attempt_revision += 1; None },
            "expiry" => { candidate.current_expiry_ms += 1; None },
            "worker" => Some(("worker_id", rusqlite::types::Value::Text("foreign".into()))),
            "state" => Some(("status", rusqlite::types::Value::Text("waiting".into()))),
            "cancelled" => Some(("cancel_requested", rusqlite::types::Value::Integer(1))),
            _ => unreachable!(),
        };
        let original = mutation.as_ref().map(|(field, value)| {
            let original: rusqlite::types::Value = db.query_row(
                &format!("SELECT {field} FROM durable_tasks WHERE id='task'"), [], |row| row.get(0)
            ).unwrap();
            db.execute(&format!("UPDATE durable_tasks SET {field}=?1 WHERE id='task'"), [value]).unwrap();
            (*field, original)
        });
        let store = nola_store::NolaStore::open(&path).unwrap();
        let snapshot = || (store.runtime_task("task").unwrap(), store.runtime_agent_task_authority("task").unwrap(),
            store.runtime_task_attempt(&lease.attempt_id).unwrap());
        let before = snapshot();
        assert!(runtime.execute(ApplicationCommand::BeginTaskAssistantTurn {
            lease: candidate, input: begin(),
        }).is_err(), "accepted {fault}");
        assert_eq!(snapshot(), before, "{fault} changed authority on failed admission");
        assert!(read_binding(&mut runtime).is_none());
        let runs: i64 = db.query_row("SELECT COUNT(*) FROM runs WHERE conversation_id='child'", [], |row| row.get(0)).unwrap();
        assert_eq!(runs, 0, "{fault} published a child Run");
        if let Some((field, value)) = original {
            db.execute(&format!("UPDATE durable_tasks SET {field}=?1 WHERE id='task'"), [value]).unwrap();
        }
        runtime.execute(ApplicationCommand::BeginTaskAssistantTurn { lease, input: begin() }).unwrap();
    }
}

#[test]
fn binding_failure_rolls_back_the_entire_canonical_begin() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("rollback.sqlite");
    let lease = seed(&path);
    let mut runtime = support::open_runtime(&path).unwrap();
    let db = rusqlite::Connection::open(&path).unwrap();
    let tables = [
        "runs",
        "messages",
        "core_checkpoints",
        "effect_outbox",
        "runtime_task_run_bindings",
    ];
    let baseline: Vec<i64> = tables
        .iter()
        .map(|table| {
            db.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap()
        })
        .collect();
    db.execute_batch("CREATE TRIGGER fail_task_binding BEFORE INSERT ON runtime_task_run_bindings BEGIN SELECT RAISE(ABORT,'binding fault'); END;").unwrap();
    assert!(runtime
        .execute(ApplicationCommand::BeginTaskAssistantTurn {
            lease: lease.clone(),
            input: begin()
        })
        .is_err());
    for (table, before) in tables.into_iter().zip(baseline) {
        let count: i64 = db
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, before, "{table} leaked from failed task admission");
    }
    db.execute_batch("DROP TRIGGER fail_task_binding;").unwrap();
    runtime
        .execute(ApplicationCommand::BeginTaskAssistantTurn {
            lease,
            input: begin(),
        })
        .unwrap();
    assert!(read_binding(&mut runtime).is_some());
}

#[test]
fn measured_budget_failure_settles_task_without_rewriting_child_seal() {
    for child_status in ["completed", "cancelled"] {
        for resource in [
            "cpu_time_ms",
            "memory_bytes",
            "processes",
            "output_bytes",
            "wall_time_ms",
        ] {
            let root = tempfile::tempdir().unwrap();
            let path = root.path().join("budget-terminal.sqlite");
            let mut budget = nola_application::RuntimeTaskResourceBudget::default();
            match resource {
                "cpu_time_ms" => budget.max_cpu_time_ms = Some(10),
                "memory_bytes" => budget.max_memory_bytes = Some(10),
                "processes" => budget.max_processes = Some(10),
                "output_bytes" => budget.max_output_bytes = Some(10),
                "wall_time_ms" => budget.max_wall_time_ms = Some(100),
                _ => unreachable!(),
            }
            let lease = seed_with_budget(&path, Some(budget));
            let mut runtime = support::open_runtime(&path).unwrap();
            runtime
                .execute(ApplicationCommand::BeginTaskAssistantTurn {
                    lease: lease.clone(),
                    input: begin(),
                })
                .unwrap();
            let bound = read_binding(&mut runtime).unwrap();
            // The completed/output case deliberately races the final worker
            // report: the sealed journal itself must prove the lower bound.
            if !(resource == "output_bytes" && child_status == "completed") {
                runtime
                    .execute(ApplicationCommand::RuntimeTask {
                        command: RuntimeTaskCommand::ReportProgress {
                            task_id: lease.task_id.clone(),
                            attempt_id: lease.attempt_id.clone(),
                            expected_attempt_revision: lease.expected_attempt_revision,
                            worker_generation: lease.worker_generation.clone(),
                            lease_token: lease.lease_token.clone(),
                            current_expiry_ms: lease.current_expiry_ms,
                            progress_summary: Some("measured usage".into()),
                            last_activity: None,
                            output_bytes: Some(11),
                            resource_usage: Some(nola_application::RuntimeTaskResourceUsage {
                                cpu_time_ms: 11,
                                peak_memory_bytes: 11,
                                peak_processes: 11,
                            }),
                            metadata_patch: Default::default(),
                            now_ms: 1100,
                        },
                    })
                    .unwrap();
            }
            finish_child(&mut runtime, &bound, child_status);
            let reconcile = || ApplicationCommand::RuntimeTask {
                command: RuntimeTaskCommand::ReconcileBoundRunTerminal {
                    task_id: "task".into(),
                    expected_run_id: bound.run_id.clone(),
                    now_ms: 1200,
                },
            };
            let db = rusqlite::Connection::open(&path).unwrap();
            let store = nola_store::NolaStore::open(&path).unwrap();
            let before = serde_json::to_value(store.runtime_task("task").unwrap()).unwrap();
            db.execute_batch("CREATE TRIGGER reject_budget_receipt BEFORE INSERT ON runtime_task_terminal_receipts BEGIN SELECT RAISE(ABORT,'budget receipt fault'); END;").unwrap();
            assert!(runtime.execute(reconcile()).is_err());
            assert_eq!(
                serde_json::to_value(store.runtime_task("task").unwrap()).unwrap(),
                before
            );
            assert!(store
                .runtime_task_terminal_receipt("task")
                .unwrap()
                .is_none());
            db.execute_batch("DROP TRIGGER reject_budget_receipt;")
                .unwrap();
            assert_eq!(runtime.execute(reconcile()).unwrap(), json!(true));
            let receipt = store
                .runtime_task_terminal_receipt("task")
                .unwrap()
                .unwrap();
            assert_eq!(receipt.status, "sealed");
            let result = receipt.journal_result.as_ref().unwrap();
            let preview_limit = if resource == "output_bytes" { 10 } else { 4096 };
            assert!(result.preview.len() <= preview_limit);
            assert_eq!(
                result.preview_truncated,
                (result.preview.len() as u64) < result.public_utf8_bytes.0
            );
            assert_eq!(
                store
                    .runtime_task("task")
                    .unwrap()
                    .unwrap()
                    .result_summary
                    .as_deref(),
                Some(result.preview.as_str())
            );
            assert_eq!(
                serde_json::to_value(&result.seal.result.value).unwrap()["kind"],
                json!(if child_status == "completed" { "answered" } else { child_status })
            );
            assert_eq!(result.seal.identity.run_id, bound.run_id);
            let source = serde_json::to_value(&receipt.source).unwrap();
            assert_eq!(source["budget_failure"]["resource"], resource);
            assert_eq!(runtime.execute(reconcile()).unwrap(), json!(true));
            assert_eq!(
                store.runtime_task_terminal_receipt("task").unwrap(),
                Some(receipt)
            );
        }
    }
}

#[test]
fn exact_terminal_reconciliation_never_begins_a_replacement_run_and_is_idempotent() {
    for status in ["completed", "failed", "cancelled"] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("terminal.sqlite");
        let lease = seed(&path);
        let mut runtime = support::open_runtime(&path).unwrap();
        runtime
            .execute(ApplicationCommand::BeginTaskAssistantTurn {
                lease: lease.clone(),
                input: begin(),
            })
            .unwrap();
        let bound = read_binding(&mut runtime).unwrap();
        let db = rusqlite::Connection::open(&path).unwrap();
        finish_child(&mut runtime, &bound, status);
        assert_eq!(
            runtime
                .execute(ApplicationCommand::RuntimeTask {
                    command: RuntimeTaskCommand::ReconcileBoundRunTerminal {
                        task_id: "task".into(),
                        expected_run_id: "foreign".into(),
                        now_ms: 1200,
                    }
                })
                .unwrap(),
            json!(false)
        );
        for _ in 0..2 {
            assert_eq!(
                runtime
                    .execute(ApplicationCommand::RuntimeTask {
                        command: RuntimeTaskCommand::ReconcileBoundRunTerminal {
                            task_id: "task".into(),
                            expected_run_id: bound.run_id.clone(),
                            now_ms: 1200,
                        }
                    })
                    .unwrap(),
                json!(true)
            );
        }
        let mut store = nola_store::NolaStore::open(&path).unwrap();
        let task = store.runtime_task("task").unwrap().unwrap();
        assert_eq!(task.status, "sealed");
        let receipt = store
            .runtime_task_terminal_receipt("task")
            .unwrap()
            .unwrap();
        let result = receipt
            .journal_result
            .as_ref()
            .expect("canonical task terminal must bind its seal");
        assert_eq!(result.seal.identity.run_id, bound.run_id);
        assert_eq!(result.seal.identity.conversation_id, "child");
        assert_eq!(serde_json::to_value(&result.seal.result.value).unwrap()["kind"],
            json!(if status == "completed" { "answered" } else { status }));
        assert_eq!(result.seal.result_digest, result.seal.result.computed_digest().unwrap());
        if status == "completed" {
            assert_eq!(result.preview, "真实结果🙂");
            assert_eq!(result.public_utf8_bytes.0, "真实结果🙂".len() as u64);
            assert!(!result.preview_truncated);
            assert_eq!(task.result_summary.as_deref(), Some("真实结果🙂"));
        }
        assert_eq!(
            nola_store::NolaStore::open(&path)
                .unwrap()
                .runtime_task_terminal_receipt("task")
                .unwrap(),
            Some(receipt.clone())
        );
        let delivery_count: i64 = db
            .query_row(
                "SELECT count(*) FROM agent_commands WHERE command_id=?1",
                [format!("terminal:{}", receipt.receipt_id)],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            delivery_count, 1,
            "terminal receipt and parent delivery commit together exactly once"
        );
        assert_eq!(task.completion_watermark, Some(7));
        assert!(task.ownership_token.is_none());
        assert!(!store
            .retry_runtime_task(&lease, 1300, "stale execution", 1200)
            .unwrap());
        let events: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM durable_task_events WHERE kind='bound_run_terminal'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(events, 1);
        let runs: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM runs WHERE conversation_id='child'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(runs, 1);
        let parent_state = || {
            let mut source = support::open_runtime(&path).unwrap();
            let current: nola_application::ConversationCheckpoint = serde_json::from_value(source.execute(
                ApplicationCommand::LoadCheckpoint { conversation_id: "parent".into() }
            ).unwrap()).unwrap();
            let checkpoint: nola_agent_engine::agent::recovery::CoreCheckpoint<
                nola_agent_engine::agent::AgentThreadState,
            > = serde_json::from_str(&current.checkpoint_json).unwrap();
            checkpoint.state.task_projection.tasks["task"].state
        };
        assert_eq!(
            parent_state(),
            nola_agent_engine::agent::TaskState::Queued,
            "child seal/receipt is durable before the independent parent inbox reduction"
        );
        pump_guidance_parent(&mut runtime, &path, 1400);
        let expected = nola_agent_engine::agent::TaskState::Sealed;
        assert_eq!(parent_state(), expected);
    }
}

#[test]
fn terminal_delivery_failure_rolls_back_result_receipt_and_attempt_retirement() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("terminal-delivery-rollback.sqlite");
    let lease = seed(&path);
    let mut runtime = support::open_runtime(&path).unwrap();
    runtime
        .execute(ApplicationCommand::BeginTaskAssistantTurn {
            lease,
            input: begin(),
        })
        .unwrap();
    let bound = read_binding(&mut runtime).unwrap();
    finish_child(&mut runtime, &bound, "completed");
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TRIGGER fail_terminal_delivery BEFORE INSERT ON agent_commands
        WHEN NEW.command_id LIKE 'terminal:%' BEGIN SELECT RAISE(ABORT,'terminal delivery fault'); END;").unwrap();
    let reconcile = || ApplicationCommand::RuntimeTask {
        command: RuntimeTaskCommand::ReconcileBoundRunTerminal {
            task_id: "task".into(),
            expected_run_id: bound.run_id.clone(),
            now_ms: 1200,
        },
    };
    assert!(runtime
        .execute(reconcile())
        .unwrap_err()
        .contains("terminal delivery fault"));
    let store = nola_store::NolaStore::open(&path).unwrap();
    let task = store.runtime_task("task").unwrap().unwrap();
    assert_eq!(task.status, "running");
    assert!(task.result_summary.is_none());
    assert!(task.ownership_token.is_some());
    assert!(store
        .runtime_task_terminal_receipt("task")
        .unwrap()
        .is_none());
    db.execute_batch("DROP TRIGGER fail_terminal_delivery;")
        .unwrap();
    assert_eq!(runtime.execute(reconcile()).unwrap(), json!(true));
    assert_eq!(
        store.runtime_task("task").unwrap().unwrap().status,
        "sealed"
    );
}

#[test]
fn task_budget_bounds_unicode_preview_atomically_without_changing_child_evidence() {
    fn rows(db: &rusqlite::Connection, tables: &[&str]) -> Vec<Vec<Vec<rusqlite::types::Value>>> {
        tables
            .iter()
            .map(|table| {
                let mut statement = db
                    .prepare(&format!("SELECT * FROM {table} ORDER BY rowid"))
                    .unwrap();
                let width = statement.column_count();
                let rows = statement
                    .query_map([], |row| {
                        (0..width)
                            .map(|index| row.get(index))
                            .collect::<rusqlite::Result<Vec<_>>>()
                    })
                    .unwrap()
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .unwrap();
                rows
            })
            .collect()
    }
    for limit in [1u64, 2, 3, 4, 1024, 4096, 8192] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("bounded-unicode-task.sqlite");
        let lease = seed_with_budget(
            &path,
            Some(nola_application::RuntimeTaskResourceBudget {
                max_output_bytes: Some(limit),
                ..Default::default()
            }),
        );
        let mut runtime = support::open_runtime(&path).unwrap();
        runtime
            .execute(ApplicationCommand::BeginTaskAssistantTurn {
                lease,
                input: begin(),
            })
            .unwrap();
        let bound = read_binding(&mut runtime).unwrap();
        let text = "水".repeat(if limit == 1024 { 600 } else { 2000 });
        finish_child_with_text(&mut runtime, &bound, "completed", &text);
        drop(runtime);
        let mut runtime = support::open_runtime(&path).unwrap();
        let reconcile = || ApplicationCommand::RuntimeTask {
            command: RuntimeTaskCommand::ReconcileBoundRunTerminal {
                task_id: "task".into(),
                expected_run_id: bound.run_id.clone(),
                now_ms: 1200,
            },
        };
        let db = rusqlite::Connection::open(&path).unwrap();
        let evidence_tables = [
            "assistant_run_terminal_seals",
            "assistant_run_terminal_manifest",
            "assistant_journal_records",
            "assistant_journal_part_versions",
        ];
        let original_evidence = rows(&db, &evidence_tables);
        let mutable_tables = [
            "durable_tasks",
            "runtime_task_attempts",
            "durable_task_events",
            "permission_requests",
            "runtime_task_permission_suspensions",
            "runtime_task_terminal_receipts",
        ];
        let before = rows(&db, &mutable_tables);
        db.execute_batch("CREATE TRIGGER reject_preview_receipt BEFORE INSERT ON runtime_task_terminal_receipts BEGIN SELECT RAISE(ABORT,'preview receipt fault'); END;").unwrap();
        assert!(runtime.execute(reconcile()).is_err());
        assert_eq!(
            rows(&db, &mutable_tables),
            before,
            "failed preview transaction leaked for limit {limit}"
        );
        assert_eq!(rows(&db, &evidence_tables), original_evidence);
        db.execute_batch("DROP TRIGGER reject_preview_receipt;")
            .unwrap();
        assert_eq!(runtime.execute(reconcile()).unwrap(), json!(true));
        let store = nola_store::NolaStore::open(&path).unwrap();
        let receipt = store
            .runtime_task_terminal_receipt("task")
            .unwrap()
            .unwrap();
        let result = receipt.journal_result.as_ref().unwrap();
        let expected_limit = limit.min(4096) as usize;
        let mut end = expected_limit.min(text.len());
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        assert_eq!(result.preview, &text[..end]);
        assert_eq!(result.public_utf8_bytes.0, text.len() as u64);
        assert_eq!(result.preview_truncated, end < text.len());
        assert_eq!(
            serde_json::to_value(&result.seal.result.value).unwrap()["kind"],
            json!("answered")
        );
        let task = store.runtime_task("task").unwrap().unwrap();
        assert_eq!(
            task.result_summary.as_deref(),
            Some(result.preview.as_str())
        );
        assert_eq!(task.status, "sealed");
        let metadata: serde_json::Value = serde_json::from_str(&task.metadata_json).unwrap();
        assert_eq!(
            metadata
                .pointer("/_nola_scheduler/output_bytes")
                .and_then(|v| v.as_u64()),
            Some(text.len() as u64)
        );
        assert_eq!(rows(&db, &evidence_tables), original_evidence);
        let receipt_bytes: String = db
            .query_row(
                "SELECT receipt_json FROM runtime_task_terminal_receipts WHERE task_id='task'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let settled = rows(&db, &mutable_tables);
        drop(runtime);
        let mut runtime = support::open_runtime(&path).unwrap();
        assert_eq!(runtime.execute(reconcile()).unwrap(), json!(true));
        assert_eq!(
            store.runtime_task_terminal_receipt("task").unwrap(),
            Some(receipt)
        );
        let replay_bytes: String = db
            .query_row(
                "SELECT receipt_json FROM runtime_task_terminal_receipts WHERE task_id='task'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(replay_bytes, receipt_bytes);
        assert_eq!(rows(&db, &mutable_tables), settled);
        assert_eq!(rows(&db, &evidence_tables), original_evidence);
    }
}
#[test]
fn cold_task_result_preserves_unicode_byte_count_and_bounded_sealed_preview() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("unicode-task-result.sqlite");
    let lease = seed(&path);
    let mut runtime = support::open_runtime(&path).unwrap();
    runtime
        .execute(ApplicationCommand::BeginTaskAssistantTurn {
            lease,
            input: begin(),
        })
        .unwrap();
    let bound = read_binding(&mut runtime).unwrap();
    let text = "中🙂".repeat(2000);
    finish_child_with_text(&mut runtime, &bound, "completed", &text);
    drop(runtime);
    // No live worker progress, local trace, or output buffer survives reopen.
    let mut recovered = support::open_runtime(&path).unwrap();
    assert_eq!(
        recovered
            .execute(ApplicationCommand::RuntimeTask {
                command: RuntimeTaskCommand::ReconcileBoundRunTerminal {
                    task_id: "task".into(),
                    expected_run_id: bound.run_id.clone(),
                    now_ms: 1200,
                }
            })
            .unwrap(),
        json!(true)
    );
    let store = nola_store::NolaStore::open(&path).unwrap();
    let receipt = store
        .runtime_task_terminal_receipt("task")
        .unwrap()
        .unwrap();
    let result = receipt.journal_result.unwrap();
    assert_eq!(result.public_utf8_bytes.0, text.len() as u64);
    assert!(result.preview.len() <= 4096 && result.preview.len() > 4088);
    assert!(text.starts_with(&result.preview));
    assert!(result.preview_truncated);
    assert_eq!(result.seal.identity.run_id, bound.run_id);
    let task = store.runtime_task("task").unwrap().unwrap();
    let metadata: serde_json::Value = serde_json::from_str(&task.metadata_json).unwrap();
    assert!(
        metadata.get("terminal_usage_observed_at_ms").is_none(),
        "cold recovery cannot invent process observations"
    );
}

#[test]
fn unresolved_command_reconciliation_is_not_overwritten_by_run_terminal() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("unresolved.sqlite");
    let lease = seed(&path);
    let mut runtime = support::open_runtime(&path).unwrap();
    runtime
        .execute(ApplicationCommand::BeginTaskAssistantTurn {
            lease,
            input: begin(),
        })
        .unwrap();
    let bound = read_binding(&mut runtime).unwrap();
    // Exercise a real terminal receipt; a status label without a seal is
    // corrupt evidence and would reject before the task-state guard is tested.
    finish_child(&mut runtime, &bound, "completed");
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute(
        "UPDATE durable_tasks SET status='awaiting_reconciliation' WHERE id='task'",
        [],
    )
    .unwrap();
    assert_eq!(
        runtime
            .execute(ApplicationCommand::RuntimeTask {
                command: RuntimeTaskCommand::ReconcileBoundRunTerminal {
                    task_id: "task".into(),
                    expected_run_id: bound.run_id,
                    now_ms: 1200,
                }
            })
            .unwrap(),
        json!(false)
    );
    assert_eq!(
        nola_store::NolaStore::open(&path)
            .unwrap()
            .runtime_task("task")
            .unwrap()
            .unwrap()
            .status,
        "awaiting_reconciliation"
    );
}

#[test]
fn deleting_a_run_preserves_admission_and_cannot_authorize_a_second_execution() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("deleted.sqlite");
    let lease = seed(&path);
    let mut runtime = support::open_runtime(&path).unwrap();
    runtime
        .execute(ApplicationCommand::BeginTaskAssistantTurn {
            lease: lease.clone(),
            input: begin(),
        })
        .unwrap();
    let bound = read_binding(&mut runtime).unwrap();
    // Exercise the production lifecycle: an active Run cannot be removed.
    // Once terminal, its conversation-owned facts cascade, but admission
    // authority remains outside that lifecycle to prevent executing twice.
    let mut store = nola_store::NolaStore::open(&path).unwrap();
    assert!(store.delete_conversation("child").is_err());
    finish_child(&mut runtime, &bound, "completed");
    assert!(store.delete_conversation("child").unwrap());
    assert_eq!(read_binding(&mut runtime), Some(bound.clone()));
    let db = rusqlite::Connection::open(&path).unwrap();
    let remaining: i64 = db.query_row(
        "SELECT COUNT(*) FROM run_input_applications WHERE run_id=?1",
        [&bound.run_id], |row| row.get(0),
    ).unwrap();
    assert_eq!(remaining, 0);
    assert!(!db.prepare("PRAGMA foreign_key_check").unwrap().query([]).unwrap().next().unwrap().is_some());
    assert!(runtime
        .execute(ApplicationCommand::BeginTaskAssistantTurn {
            lease,
            input: begin()
        })
        .is_err());
    assert!(runtime
        .execute(ApplicationCommand::BeginAssistantTurn { input: begin() })
        .is_err());
}

#[test]
fn committed_bound_run_completion_wins_over_later_task_cancel_and_rejects_late_guidance() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("late-cancel.sqlite");
    let lease = seed(&path);
    let mut runtime = support::open_runtime(&path).unwrap();
    runtime
        .execute(ApplicationCommand::BeginTaskAssistantTurn {
            lease,
            input: begin(),
        })
        .unwrap();
    let bound = read_binding(&mut runtime).unwrap();
    finish_child(&mut runtime, &bound, "completed");
    let guidance: nola_application::RuntimeTaskGuidance = serde_json::from_value(
        runtime
            .execute(ApplicationCommand::SubmitTaskGuidance {
                task_id: "task".into(),
                message_id: "late-message".into(),
                parent_turn_id: nola_store::NolaStore::open(&path).unwrap()
                    .runtime_agent_task_authority("task").unwrap().unwrap().admission.unwrap().parent_turn_id,
                content: "new work after completion".into(),
                submitted_at_ms: 1150,
            })
            .unwrap(),
    )
    .unwrap();
    assert_eq!(guidance.status, "rejected");
    assert_eq!(
        guidance.reason.as_deref(),
        Some("bound_run_terminal_before_delivery")
    );
    let mut store = nola_store::NolaStore::open(&path).unwrap();
    assert!(store.request_runtime_task_cancel("task", 1200).unwrap());
    assert_eq!(
        runtime
            .execute(ApplicationCommand::RuntimeTask {
                command: RuntimeTaskCommand::ReconcileBoundRunTerminal {
                    task_id: "task".into(),
                    expected_run_id: bound.run_id,
                    now_ms: 1300,
                }
            })
            .unwrap(),
        json!(true)
    );
    let task = store.runtime_task("task").unwrap().unwrap();
    assert_eq!(task.status, "sealed");
    assert!(task.cancel_requested);
    let receipt = store
        .runtime_task_terminal_receipt("task")
        .unwrap()
        .unwrap();
    assert_eq!(receipt.status, "sealed");
    assert_eq!(receipt.completion_watermark, Some(7));
}

fn pump_guidance_parent(
    runtime: &mut nola_application::ApplicationRuntime<nola_store::NolaStore>,
    path: &std::path::Path,
    at: i64,
) {
    let mut store = nola_store::NolaStore::open(path).unwrap();
    while let Some(command) = store.peek_agent_command("parent").unwrap() {
        let notification: nola_agent_engine::agent::host::AgentBoundaryInput = serde_json::from_str(&command.payload_json).unwrap();
        assert!(matches!(notification,
            nola_agent_engine::agent::host::AgentBoundaryInput::TaskTerminalAvailable { .. }
            | nola_agent_engine::agent::host::AgentBoundaryInput::TaskGuidanceAvailable { .. }));
        assert!(!runtime.execute(ApplicationCommand::ApplyPendingTaskNotice {
            conversation_id:"parent".into(),consumer_id:"test-parent".into(),now_ms:at,
        }).unwrap().is_null());
    }
}

fn submit_guidance(
    runtime: &mut nola_application::ApplicationRuntime<nola_store::NolaStore>,
    id: &str,
    at: i64,
) {
    let authority: Option<nola_domain::agent_task_admission::RuntimeAgentTaskAuthority> = serde_json::from_value(
        runtime.execute(ApplicationCommand::RuntimeTask { command: RuntimeTaskCommand::AgentAuthority { task_id: "task".into() } }).unwrap(),
    ).unwrap();
    let parent_turn_id = authority.unwrap().admission.unwrap().parent_turn_id;
    runtime
        .execute(ApplicationCommand::SubmitTaskGuidance {
            task_id: "task".into(),
            message_id: id.into(),
            parent_turn_id,
            content: format!("Guidance {id}"),
            submitted_at_ms: at,
        })
        .unwrap();
}

fn complete_guidance_model_step(
    runtime: &mut nola_application::ApplicationRuntime<nola_store::NolaStore>,
    path: &std::path::Path,
    run: &str,
    at: i64,
    answer: &str,
) {
    let mut store = nola_store::NolaStore::open(path).unwrap();
    let active = store.active_run("child").unwrap().unwrap();
    let digest = active.invocation_config_digest.unwrap();
    let mut request = nola_application::load_committed_host_requests(&store, run)
        .unwrap()
        .requests
        .into_iter()
        .find(|request| matches!(request.payload, HostRequest::ModelStream { .. }))
        .unwrap();
    support::admit_model_sampling_for_conversation(
        &mut store,
        "child",
        run,
        &digest,
        &mut request,
        at,
    );
    *runtime = support::open_runtime(path).unwrap();
    let response = HostResponseEnvelope {
        attempt_id: request.attempt_id.clone(),
        delegated_invocation_id: request.delegated_invocation_id.clone(),
        request_id: request.request_id.clone(),
        effect_id: request.effect_id.clone(),
        host_id: request.host_id.clone(),
        lease_id: request.lease_id.clone(),
        session_id: request.session_id.clone(),
        turn_id: request.turn_id.clone(),
        catalog_revision: request.catalog_revision,
        availability_epoch: request.availability_epoch,
        delivery: request.delivery.clone(),
        sequence: request.resume_after_sequence.unwrap_or(0) + 1,
        payload: HostResponse::ModelCompleted {
            items: vec![serde_json::to_value(AgentItem::assistant_message(
                ItemId::from_string(match &request.payload { HostRequest::ModelStream { text_part_id, .. } => text_part_id.clone(), _ => unreachable!() }),
                answer,
            ))
            .unwrap()],
            tool_calls: vec![],
            usage: json!({}),
        },
    };
    runtime
        .execute(ApplicationCommand::ReduceAndAcceptHostResponse {
            input: nola_application::ReduceAndAcceptHostResponse {
                conversation_id: "child".into(),
                run_id: run.into(),
                invocation_config_digest: digest,
                response,
                received_at_ms: at,
            },
        })
        .unwrap();

}

fn complete_guidance_child(runtime: &mut nola_application::ApplicationRuntime<nola_store::NolaStore>,
    path: &std::path::Path, run: &str, at: i64) {
    complete_guidance_model_step(runtime,path,run,at,"Finished original task.");
    assert!(nola_store::NolaStore::open(path).unwrap().active_run("child").unwrap().is_none());
}

#[test]
fn delivered_guidance_replay_after_new_events_and_operation_eviction_never_injects_again() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("guidance-replay.sqlite");
    let lease = seed(&path);
    let mut runtime = support::open_runtime(&path).unwrap();
    runtime
        .execute(ApplicationCommand::BeginTaskAssistantTurn {
            lease: lease.clone(),
            input: begin(),
        })
        .unwrap();
    let run = read_binding(&mut runtime).unwrap().run_id;
    submit_guidance(&mut runtime, "g1", 1050);
    pump_guidance_parent(&mut runtime, &path, 1100);
    let apply = |lease, at| ApplicationCommand::ApplyTaskGuidance {
        task_id: "task".into(),
        message_id: "g1".into(),
        lease,
        expected_run_id: run.clone(),
        now_ms: at,
    };
    let first: nola_application::TaskGuidanceApplyResult =
        serde_json::from_value(runtime.execute(apply(Some(lease), 1150)).unwrap()).unwrap();
    assert_eq!(first.accepted.unwrap().control.queued_input_count, 1);
    let store = nola_store::NolaStore::open(&path).unwrap();
    let active = store.active_run("child").unwrap().unwrap();
    let newer: nola_application::AcceptedActiveSubmission = serde_json::from_value(
        runtime
            .execute(ApplicationCommand::ReduceAndCommitActiveSubmission {
                input: nola_application::ReduceAndCommitActiveSubmission {
                    conversation_id: "child".into(),
                    run_id: run.clone(),
                    transition_id: "later-input".into(),
                    invocation_config_digest: active.invocation_config_digest.unwrap(),
                    submission: AgentSubmission::UserInput {
                        items: vec![AgentItem::user_message(
                            ItemId::from_string("later-input".into()),
                            "Later input",
                        )],
                        turn_context_items: vec![],
                        turn_context_platform: Default::default(),
                        turn_context_components: vec![],
                        submission_id: Some("later-input".into()),
                        submitted_at_epoch_ms: Some(1200),
                        delivery: AgentInputDelivery::CurrentTurn,
                    },
                    committed_at_ms: 1200,
                },
            })
            .unwrap(),
    )
    .unwrap();
    assert_eq!(newer.control.queued_input_count, 2);
    drop(runtime);
    let db = rusqlite::Connection::open(&path).unwrap();
    let text = support::current_checkpoint(&path, "child").checkpoint_json;
    let mut checkpoint: nola_agent_engine::agent::recovery::CoreCheckpoint<
        nola_agent_engine::agent::AgentThreadState,
    > = serde_json::from_str(&text).unwrap();
    checkpoint.state.journal.accepted_submission_ids.clear();
    checkpoint.state.journal.accepted_submission_order.clear();
    db.execute(
        "UPDATE core_checkpoint_bodies SET checkpoint_json=?1 WHERE conversation_id='child'",
        [serde_json::to_string(&checkpoint).unwrap()],
    )
    .unwrap();
    // The fixture replaces the fully materialized frontier, not its older
    // physical baseline. Preserve the exact consumed prefix when evicting IDs.
    db.execute("INSERT INTO core_checkpoint_replay_bases
        SELECT c.conversation_id,r.id,c.revision,COALESCE(h.ordinal,0),COALESCE(h.reduction_bytes,0)
        FROM core_checkpoints c JOIN runs r ON r.conversation_id=c.conversation_id
        LEFT JOIN host_event_replay_heads h ON h.run_id=r.id
        WHERE c.conversation_id='child' AND r.id=?1", [&run]).unwrap();
    let mut runtime = support::open_runtime(&path).unwrap();
    let replay: nola_application::TaskGuidanceApplyResult =
        serde_json::from_value(runtime.execute(apply(None, 1300)).unwrap()).unwrap();
    let accepted = replay.accepted.unwrap();
    assert_eq!(accepted.control.queued_input_count, 2);
    assert!(accepted.events.iter().all(|event| !matches!(
        event.event,
        nola_agent_engine::agent::AgentEvent::InputQueued { .. }
    )));
    let count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM runs WHERE conversation_id='child'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
    pump_guidance_parent(&mut runtime, &path, 1350);
    let parent: nola_application::ConversationCheckpoint = serde_json::from_value(
        runtime.execute(ApplicationCommand::LoadCheckpoint {
            conversation_id: "parent".into(),
        }).unwrap(),
    ).unwrap();
    let parent: nola_agent_engine::agent::recovery::CoreCheckpoint<nola_agent_engine::agent::AgentThreadState> =
        serde_json::from_str(&parent.checkpoint_json).unwrap();
    assert!(parent.state.task_projection.guidance["task"]
        .pending
        .is_empty());
    assert_eq!(
        parent.state.task_projection.guidance["task"].results["g1"].status,
        "delivered"
    );
}

#[test]
fn unobserved_or_parent_observed_guidance_gets_explicit_rejection_when_original_run_finishes() {
    for parent_observed in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("guidance-terminal.sqlite");
        let lease = seed(&path);
        let mut runtime = support::open_runtime(&path).unwrap();
        runtime
            .execute(ApplicationCommand::BeginTaskAssistantTurn {
                lease: lease.clone(),
                input: begin(),
            })
            .unwrap();
        let run = read_binding(&mut runtime).unwrap().run_id;
        submit_guidance(&mut runtime, "late", 1050);
        let store = nola_store::NolaStore::open(&path).unwrap();
        assert_eq!(store.runtime_task_guidance("task", "late").unwrap().unwrap().status, "accepted");
        // The parent only observes acceptance. It does not grant execution authority.
        if parent_observed {
            pump_guidance_parent(&mut runtime, &path, 1100);
        }
        let before = store.runtime_task("task").unwrap().unwrap();
        let error = runtime.execute(ApplicationCommand::RuntimeTask {
            command: RuntimeTaskCommand::FinishDetailedWithEvent {
                final_observation: None,
                lease, status: "completed".into(), completion_watermark: Some(7),
                progress_summary: None, result_summary: Some("worker summary".into()),
                error: None, output_path: None, unread: false,
                event_kind: "completed".into(), event_status: "completed".into(),
                event_summary: None, event_detail_json: "{}".into(), now_ms: 1150,
            },
        }).unwrap_err();
        assert!(error.contains("canonical Run seal"), "{error}");
        assert_eq!(store.runtime_task("task").unwrap().unwrap(), before);
        assert!(store.runtime_task_terminal_receipt("task").unwrap().is_none());
        assert_eq!(store.runtime_task_guidance("task", "late").unwrap().unwrap().status, "accepted");
        complete_guidance_child(&mut runtime, &path, &run, 1200);
        assert_eq!(
            runtime
                .execute(ApplicationCommand::RuntimeTask {
                    command: RuntimeTaskCommand::ReconcileBoundRunTerminal {
                        task_id: "task".into(),
                        expected_run_id: run.clone(),
                        now_ms: 1300,
                    }
                })
                .unwrap(),
            json!(true)
        );
        let store = nola_store::NolaStore::open(&path).unwrap();
        let receipt = store
            .runtime_task_terminal_receipt("task")
            .unwrap()
            .unwrap();
        runtime
            .execute(ApplicationCommand::EnqueueTaskTerminalReconciliation {
                task_id: "task".into(),
                expected_receipt_id: receipt.receipt_id,
            })
            .unwrap();
        pump_guidance_parent(&mut runtime, &path, 1400);
        let guidance = store
            .runtime_task_guidance("task", "late")
            .unwrap()
            .unwrap();
        assert_eq!(guidance.status, "rejected");
        assert_eq!(
            guidance.reason.as_deref(),
            Some("bound_run_terminal_before_delivery")
        );
        let result: nola_application::TaskGuidanceApplyResult = serde_json::from_value(
            runtime
                .execute(ApplicationCommand::ApplyTaskGuidance {
                    task_id: "task".into(),
                    message_id: "late".into(),
                    lease: None,
                    expected_run_id: run,
                    now_ms: 1500,
                })
                .unwrap(),
        )
        .unwrap();
        assert!(result.accepted.is_none());
        assert!(result.recovered.is_some());
        let cp: nola_application::ConversationCheckpoint = serde_json::from_value(
            runtime.execute(ApplicationCommand::LoadCheckpoint {
                conversation_id: "parent".into(),
            }).unwrap(),
        ).unwrap();
        let cp: nola_agent_engine::agent::recovery::CoreCheckpoint<nola_agent_engine::agent::AgentThreadState> =
            serde_json::from_str(&cp.checkpoint_json).unwrap();
        assert_eq!(
            cp.state.task_projection.tasks["task"].state,
            nola_agent_engine::agent::TaskState::Sealed
        );
        assert!(cp.state.task_projection.guidance["task"].pending.is_empty());
        assert_eq!(
            cp.state.task_projection.guidance["task"].results["late"].status,
            "rejected"
        );
    }
}

#[test]
fn sealed_task_continuation_admits_a_new_run_and_preserves_original_binding() {
    exercise_sealed_task_continuation(None);
}

#[test]
fn task_continuation_rejects_invalid_or_competing_lineage_before_publishing_a_run() {
    for fault in ["missing_predecessor", "unbound_predecessor", "unsealed_predecessor", "different_project", "competing_continuation"] {
        exercise_sealed_task_continuation(Some(fault));
    }
}

fn exercise_sealed_task_continuation(fault: Option<&str>) {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("continuation.sqlite");
    let first_lease = seed(&path);
    let mut runtime = support::open_runtime(&path).unwrap();
    runtime.execute(ApplicationCommand::BeginTaskAssistantTurn { lease: first_lease, input: begin() }).unwrap();
    let first = read_binding(&mut runtime).unwrap();
    finish_child_with_text(&mut runtime, &first, "completed", "FIRST_RESULT_ORCHID");
    runtime.execute(ApplicationCommand::RuntimeTask { command: RuntimeTaskCommand::ReconcileBoundRunTerminal {
        task_id: "task".into(), expected_run_id: first.run_id.clone(), now_ms: 1200,
    }}).unwrap();
    let mut store = nola_store::NolaStore::open(&path).unwrap();
    let original = store.runtime_task("task").unwrap().unwrap();
    let receipt = store.runtime_task_terminal_receipt("task").unwrap().unwrap();
    let mut draft = original.clone();
    draft.id = "continuation".into();
    draft.status = "queued".into();
    draft.parent_tool_call_id = Some("continuation-call".into());
    draft.input_watermark = Some(1300);
    draft.completion_watermark = None;
    draft.created_at_ms = 1300;
    draft.updated_at_ms = 1300;
    draft.started_at_ms = None;
    draft.ended_at_ms = None;
    draft.worker_id = None;
    draft.ownership_token = None;
    draft.lease_until_ms = None;
    draft.heartbeat_at_ms = None;
    draft.result_summary = None;
    draft.error = None;
    draft.cancel_requested = false;
    draft.cancel_requested_at_ms = None;
    if fault == Some("different_project") { draft.project_root = "/another-project".into(); }
    store.create_runtime_task_with_event(&draft).unwrap();
    // Reconcile the actual predecessor into the parent before its next tool call.
    pump_guidance_parent(&mut runtime, &path, 1250);
    let mut admission = support::agent_task_admission::source_fact_for_continuation(
        &mut runtime, "continuation", "parent", "child", "continuation-call", Some(1300),
        support::model_identity::identity(), Some("task".into()));
    // Hostile transport facts still cannot establish a receiver-side history.
    if fault == Some("missing_predecessor") { admission.predecessor_task_id = None; admission = admission.seal().unwrap(); }
    if fault == Some("unbound_predecessor") { admission.predecessor_task_id = Some("missing".into()); admission = admission.seal().unwrap(); }
    runtime.execute(ApplicationCommand::ImportHostAgentTaskAdmission { admission }).unwrap();
    if fault == Some("unsealed_predecessor") {
        rusqlite::Connection::open(&path).unwrap().execute("UPDATE durable_tasks SET status='running' WHERE id='task'", []).unwrap();
    }
    if fault == Some("competing_continuation") {
        let mut competing = draft.clone();
        competing.id = "competing".into();
        competing.parent_tool_call_id = Some("competing-call".into());
        store.create_runtime_task_with_event(&competing).unwrap();
    }
    let attempt = store.claim_runtime_task("continuation", "next-worker", "next-generation", "next-token", 1350, 10000).unwrap().unwrap();
    let tail: nola_application::ConversationTailSnapshot = serde_json::from_value(runtime.execute(
        ApplicationCommand::LoadConversationTailSnapshotV1 { conversation_id: "child".into(),
            message_limit: 2, part_text_limit: 1024, part_json_limit: 1024 }
    ).unwrap()).unwrap();
    let mut input = begin();
    input.expected_configuration_revision = Some(tail.conversation.configuration_revision);
    input.expected_parent_message_id = tail.conversation.messages.last().map(|message|message.id.clone());
    input.user_message_id = "second-user".into();
    input.user_part_id = "second-user-body".into();
    input.assistant_message_id = "second-assistant".into();
    input.created_at_ms = 1400;
    input.submission = AgentSubmission::UserInput {
        items: vec![AgentItem::user_message(ItemId::from_string("second-user".into()), "Continue the prior task")],
        turn_context_items: vec![], turn_context_platform: Default::default(), turn_context_components: vec![],
        submission_id: Some("second-submission".into()), submitted_at_epoch_ms: Some(1400),
        delivery: AgentInputDelivery::CurrentTurn,
    };
    let outcome = runtime.execute(ApplicationCommand::BeginTaskAssistantTurn {
        lease: RuntimeTaskLeaseWitness::from(&attempt), input });
    if let Some(fault) = fault {
        assert!(outcome.is_err(), "admitted {fault}");
        assert!(store.runtime_task_run_binding("continuation").unwrap().is_none());
        assert_eq!(store.runtime_task_run_binding("task").unwrap().unwrap(), first);
        assert_eq!(store.runtime_task_terminal_receipt("task").unwrap().unwrap(), receipt);
        let db = rusqlite::Connection::open(&path).unwrap();
        assert_eq!(db.query_row::<i64, _, _>("SELECT COUNT(*) FROM runs WHERE conversation_id='child'", [], |row|row.get(0)).unwrap(), 1);
        return;
    }
    let accepted: nola_application::AcceptedAssistantTurn = serde_json::from_value(outcome.unwrap()).unwrap();
    let second = store.runtime_task_run_binding("continuation").unwrap().unwrap();
    assert_ne!(second.run_id, first.run_id);
    assert_eq!(second.run_id, accepted.begin.run_id);
    let second_requests = finish_child_with_text(&mut runtime, &second, "completed", "SECOND_RESULT_ORCHID");
    let model_requests: Vec<_> = second_requests.iter().filter(|request| matches!(request.payload, HostRequest::ModelStream { .. })).collect();
    assert!(!model_requests.is_empty());
    assert!(model_requests.iter().any(|request| serde_json::to_string(&request.payload).unwrap().contains("FIRST_RESULT_ORCHID")),
        "the next actual model invocation must include the first task's history");
    runtime.execute(ApplicationCommand::RuntimeTask { command: RuntimeTaskCommand::ReconcileBoundRunTerminal {
        task_id: "continuation".into(), expected_run_id: second.run_id.clone(), now_ms: 1700,
    }}).unwrap();
    assert_eq!(store.runtime_task_run_binding("task").unwrap().unwrap(), first);
    assert_eq!(store.runtime_task_terminal_receipt("task").unwrap().unwrap(), receipt);
    assert_eq!(store.runtime_task("continuation").unwrap().unwrap().status, "sealed");
    let db = rusqlite::Connection::open(&path).unwrap();
    assert_eq!(db.query_row::<i64, _, _>("SELECT COUNT(*) FROM runs WHERE conversation_id='child' AND status='sealed'", [], |row|row.get(0)).unwrap(), 2);
}

#[test]
fn guidance_receipt_survives_cold_terminal_and_retries_keep_original_input() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("guidance-receipt.sqlite");
    let lease = seed(&path);
    let mut runtime = support::open_runtime(&path).unwrap();
    runtime.execute(ApplicationCommand::BeginTaskAssistantTurn { lease, input: begin() }).unwrap();
    let run = read_binding(&mut runtime).unwrap().run_id;
    let authority: Option<nola_domain::agent_task_admission::RuntimeAgentTaskAuthority> = serde_json::from_value(
        runtime.execute(ApplicationCommand::RuntimeTask { command: RuntimeTaskCommand::AgentAuthority { task_id: "task".into() } }).unwrap()).unwrap();
    let admission = authority.unwrap().admission.unwrap();
    let query = |parent_run: &str| ApplicationCommand::LoadTaskGuidanceReceipt {
        task_id: "task".into(), message_id: "stable-guidance".into(),
        parent_conversation_id: admission.parent_conversation_id.clone(), parent_run_id: parent_run.into(),
    };
    assert!(runtime.execute(query(&admission.parent_run_id)).unwrap().is_null());
    assert!(runtime.execute(query("foreign-parent-run")).is_err());
    let submit = |content: &str, at| ApplicationCommand::SubmitTaskGuidance {
        task_id: "task".into(), message_id: "stable-guidance".into(),
        parent_turn_id: admission.parent_turn_id.clone(), content: content.into(), submitted_at_ms: at,
    };
    let first = runtime.execute(submit("Original instruction", 1050)).unwrap();
    assert_eq!(runtime.execute(query(&admission.parent_run_id)).unwrap(), first);
    assert!(runtime.execute(submit("Changed instruction", 1050)).is_err());
    assert!(runtime.execute(submit("Original instruction", 1051)).is_err());
    complete_guidance_child(&mut runtime, &path, &run, 1200);
    drop(runtime);
    let mut cold = support::open_runtime(&path).unwrap();
    let settled = cold.execute(query(&admission.parent_run_id)).unwrap();
    assert_eq!(settled["content"], "Original instruction");
    assert_eq!(settled["submitted_at_ms"], 1050);
    let observer = rusqlite::Connection::open(&path).unwrap();
    let queued_before: i64 = observer.query_row("SELECT count(*) FROM agent_commands", [], |row| row.get(0)).unwrap();
    for _ in 0..3 {
        assert_eq!(cold.execute(submit("Original instruction", 1050)).unwrap(), settled);
    }
    let queued_after: i64 = observer.query_row("SELECT count(*) FROM agent_commands", [], |row| row.get(0)).unwrap();
    assert_eq!(queued_after, queued_before, "receipt recovery must not enqueue another delivery");
    assert!(cold.execute(query("foreign-parent-run")).is_err());
    assert!(query(&admission.parent_run_id).is_read_only_query());
}

#[test]
fn task_child_readonly_window_preserves_exact_ownership() {
    use nola_store_port::conversation_history::ConversationProjectionRepository;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("task-child-owner.sqlite");
    let lease = seed(&path);
    let mut runtime = support::open_runtime(&path).unwrap();
    let admitted = runtime.execute(ApplicationCommand::BeginTaskAssistantTurn { lease, input: begin() }).unwrap();
    let _: nola_application::AcceptedAssistantTurn = serde_json::from_value(admitted).unwrap();
    let source = nola_store::NolaStore::open(&path).unwrap();
    let authority = source.runtime_agent_task_authority("task").unwrap().unwrap().admission.unwrap();
    let expected = nola_domain::ConversationExecutionOwnership::TaskChild {
        parent_conversation_id: authority.parent_conversation_id.clone(), parent_run_id: authority.parent_run_id.clone(),
        task_id: "task".into(), child_conversation_id: "child".into(),
    };
    assert_eq!(source.conversation_execution_ownership("child").unwrap(), expected);
    use nola_store_port::run_message_admission::RunMessageAdmissionRepository;
    let run = source.runtime_task_run_binding("task").unwrap().unwrap().run_id;
    let head = source.local_run_message_admission_head("child", &run).unwrap();
    let initial = nola_domain::run_message_admission::RunMessageAdmissionCursor::initial(head.identity.clone()).unwrap();
    let page = source.read_run_message_admissions(&initial, &head, 64, 262144).unwrap();
    let first_message = &page.records[0].message_id;
    let raw: String = page.records.iter().take_while(|record| &record.message_id == first_message).map(|record| record.text.as_str()).collect();
    let declaration: nola_domain::ReplicaMessageDeclaration = serde_json::from_str(&raw).unwrap();
    assert_eq!(declaration.execution_ownership, expected);

    let request = ApplicationCommand::LoadTaskChildConversationWindow {
        parent_conversation_id: authority.parent_conversation_id.clone(), parent_run_id: authority.parent_run_id.clone(),
        task_id: "task".into(), child_conversation_id: "child".into(), before_ordinal: None,
        limit: 20, content_byte_limit: 262144,
    };
    let request = serde_json::to_value(request).unwrap();
    let window = runtime.execute(serde_json::from_value(request.clone()).unwrap()).unwrap();
    assert_eq!(window["projection"]["execution_ownership"], serde_json::to_value(&expected).unwrap());
    for field in ["parent_conversation_id", "parent_run_id", "task_id", "child_conversation_id"] {
        let mut wrong = serde_json::to_value(&request).unwrap();
        wrong[field] = json!("foreign");
        assert!(runtime.execute(serde_json::from_value(wrong).unwrap()).is_err(), "{field}");
    }
}

#[test]
fn committed_task_control_claim_is_selective_and_failure_does_not_poison_outbox() {
    for reject in [false, true] {
        let directory=tempfile::tempdir().unwrap(); let path=directory.path().join("task-controls.sqlite");
        let mut runtime=support::open_runtime(&path).unwrap();
        support::agent_task_admission::draft(&mut runtime,None);
        let admission=support::agent_task_admission::source_fact(&mut runtime);
        runtime.execute(ApplicationCommand::ImportHostAgentTaskAdmission{admission:admission.clone()}).unwrap();
        let active:nola_application::ActiveRun=serde_json::from_value(runtime.execute(ApplicationCommand::ActiveRun{conversation_id:"parent".into()}).unwrap()).unwrap();
        let digest=active.invocation_config_digest.unwrap();
        runtime.execute(ApplicationCommand::ReduceAndCommitActiveSubmission{input:nola_application::ReduceAndCommitActiveSubmission{
            conversation_id:"parent".into(),run_id:active.run_id.clone(),invocation_config_digest:digest.clone(),
            transition_id:"request-task-stop".into(),submission:AgentSubmission::RequestTaskCancellation{task_id:"task".into()},committed_at_ms:1100,
        }}).unwrap();
        let claim=nola_application::ClaimCommittedHostRequests{run_id:active.run_id.clone(),worker_generation:"control-generation".into(),lease_owner:"control-owner".into(),claimed_at_ms:1200,lease_expires_at_ms:100000,limit:64};
        let selected:nola_application::CommittedHostRequests=serde_json::from_value(runtime.execute(ApplicationCommand::ClaimApplicationControlRequests{input:claim}).unwrap()).unwrap();
        assert_eq!(selected.requests.len(),1,"no unrelated model or task effect may be leased");
        let request=&selected.requests[0]; assert!(matches!(request.payload,HostRequest::CancelTask{..}));
        let response=|sequence,payload| ApplicationCommand::ReduceAndAcceptHostResponse{input:nola_application::ReduceAndAcceptHostResponse{
            conversation_id:"parent".into(),run_id:active.run_id.clone(),invocation_config_digest:digest.clone(),received_at_ms:if reject {40000} else {1300},
            response:HostResponseEnvelope{attempt_id:request.attempt_id.clone(),delegated_invocation_id:request.delegated_invocation_id.clone(),
                request_id:request.request_id.clone(),effect_id:request.effect_id.clone(),host_id:request.host_id.clone(),lease_id:request.lease_id.clone(),
                session_id:request.session_id.clone(),turn_id:request.turn_id.clone(),catalog_revision:request.catalog_revision,availability_epoch:request.availability_epoch,
                delivery:request.delivery.clone(),sequence,payload},
        }};
        if reject {
            assert!(request.is_expired_at(40000));
            assert!(runtime.execute(ApplicationCommand::ExecuteTaskCancellationForEffect{request:request.clone(),now_ms:40000}).is_err());
            runtime.execute(response(0,HostResponse::ExecutionRejected{code:"deadline_exceeded".into(),message:"control expired before start".into(),retryable:false})).unwrap();
        } else {
            assert!(runtime.execute(ApplicationCommand::ExecuteTaskCancellationForEffect{request:request.clone(),now_ms:1250}).is_err(),"must require committed execution start");
            runtime.execute(response(0,HostResponse::ExecutionStarted)).unwrap();
            let mut foreign=request.clone(); foreign.delivery.as_mut().unwrap().lease_token.push_str("-foreign");
            assert!(runtime.execute(ApplicationCommand::ExecuteTaskCancellationForEffect{request:foreign,now_ms:1300}).is_err());
            assert_eq!(runtime.execute(ApplicationCommand::ExecuteTaskCancellationForEffect{request:request.clone(),now_ms:1300}).unwrap(),json!({"kind":"applied","cancel_requested":true}));
            assert_eq!(runtime.execute(ApplicationCommand::ExecuteTaskCancellationForEffect{request:request.clone(),now_ms:1301}).unwrap(),json!({"kind":"applied","cancel_requested":true}),"same-store retry returns durable intent, not changed=false");
            runtime.execute(response(1,HostResponse::Completed{output:json!({"task_id":"task","cancel_requested":true}),usage:None,full_output_ref:None,artifacts:vec![]})).unwrap();
        }
        let store=nola_store::NolaStore::open(&path).unwrap();
        assert_eq!(store.runtime_task("task").unwrap().unwrap().cancel_requested,!reject);
        let inspect=rusqlite::Connection::open(&path).unwrap();
        let state:String=inspect.query_row("SELECT state FROM effect_delivery_attempts WHERE effect_id=?1",[&request.effect_id.0],|row|row.get(0)).unwrap();
        assert_eq!(state,"settled");
        let pending:nola_application::CommittedHostRequests=serde_json::from_value(runtime.execute(ApplicationCommand::LoadCommittedHostRequests{run_id:active.run_id}).unwrap()).unwrap();
        assert!(!pending.requests.iter().any(|value|value.effect_id==request.effect_id));
        let after:nola_application::CommittedHostRequests=serde_json::from_value(runtime.execute(ApplicationCommand::ClaimCommittedHostRequests{input:nola_application::ClaimCommittedHostRequests{
            run_id:admission.parent_run_id.clone(),worker_generation:"next-generation".into(),lease_owner:"next-owner".into(),
            claimed_at_ms:41000,lease_expires_at_ms:141000,limit:64,
        }}).unwrap()).unwrap();
        assert!(!after.requests.iter().any(|value|value.effect_id==request.effect_id),"settled failure cannot block following claims");
    }
}

#[test]
fn applied_guidance_segments_one_run_without_reordering_previous_answer_and_seals_all_owners() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("guidance-segments.sqlite");
    let lease = seed(&path);
    let mut runtime = support::open_runtime(&path).unwrap();
    runtime.execute(ApplicationCommand::BeginTaskAssistantTurn { lease:lease.clone(), input:begin() }).unwrap();
    let run = read_binding(&mut runtime).unwrap().run_id;
    submit_guidance(&mut runtime,"segment-guidance",1050);
    pump_guidance_parent(&mut runtime,&path,1100);
    runtime.execute(ApplicationCommand::ApplyTaskGuidance { task_id:"task".into(), message_id:"segment-guidance".into(),
        lease:Some(lease), expected_run_id:run.clone(), now_ms:1150 }).unwrap();
    let db = rusqlite::Connection::open(&path).unwrap();
    let count = || db.query_row("SELECT COUNT(*) FROM assistant_message_segments WHERE run_id=?1",[&run],|row|row.get::<_,i64>(0)).unwrap();
    assert_eq!(count(),0,"queued input alone has no presentation boundary");
    complete_guidance_model_step(&mut runtime,&path,&run,1200,"BEFORE_GUIDANCE");
    assert_eq!(count(),1);
    let messages: Vec<(String,String,i64)> = db.prepare("SELECT id,role,ordinal FROM messages WHERE conversation_id='child' ORDER BY ordinal").unwrap()
        .query_map([],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).unwrap().collect::<Result<_,_>>().unwrap();
    let user = messages.iter().position(|(id,_,_)|id=="task-guidance:segment-guidance").unwrap();
    assert_eq!(messages[user-1].1,"assistant"); assert_eq!(messages[user+1].1,"assistant");
    let successor = messages[user+1].0.clone();
    let original = messages[user-1].0.clone();
    assert_ne!(original,successor);
    drop(runtime);
    let mut runtime = support::open_runtime(&path).unwrap();
    for step in 0..8 {
        if nola_store::NolaStore::open(&path).unwrap().active_run("child").unwrap().is_none() { break; }
        complete_guidance_model_step(&mut runtime,&path,&run,1300+step*100,"AFTER_GUIDANCE");
    }
    assert!(nola_store::NolaStore::open(&path).unwrap().active_run("child").unwrap().is_none());
    assert_eq!(db.query_row("SELECT COUNT(*) FROM runs WHERE conversation_id='child'",[],|r|r.get::<_,i64>(0)).unwrap(),1);
    assert_eq!(db.query_row("SELECT COUNT(*) FROM messages WHERE run_id=?1 AND role='assistant' AND (status NOT IN ('completed','cancelled','failed') OR completed_at_ms IS NULL)",[&run],|r|r.get::<_,i64>(0)).unwrap(),0);
    let owners: Vec<String> = db.prepare("SELECT message_id FROM assistant_effect_message_bindings WHERE run_id=?1 ORDER BY effect_id").unwrap()
        .query_map([&run],|r|r.get(0)).unwrap().collect::<Result<_,_>>().unwrap();
    assert!(owners.contains(&original) && owners.contains(&successor),"every attempt has its original effect owner after cold reopen");
    let manifest:Vec<serde_json::Value> = db.prepare("SELECT entry_json FROM assistant_run_terminal_manifest WHERE run_id=?1 ORDER BY ordinal").unwrap()
        .query_map([&run],|r|r.get::<_,String>(0)).unwrap().map(|row|serde_json::from_str(&row.unwrap()).unwrap()).collect();
    for owner in [&original,&successor] {
        assert!(manifest.iter().any(|entry|entry.pointer("/through/identity/assistant_message_id").and_then(|v|v.as_str())==Some(owner.as_str())),
            "terminal manifest must contain every admitted assistant segment: {owner}");
    }
    assert_eq!(db.query_row("SELECT COUNT(*) FROM assistant_run_terminal_seals WHERE run_id=?1",[&run],|r|r.get::<_,i64>(0)).unwrap(),1);
}
