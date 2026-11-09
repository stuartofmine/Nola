// SPDX-License-Identifier: Apache-2.0
use super::*;
use nola_core::agent::{host::*, protocol::*};

#[test]
fn permission_wait_wakes_only_for_its_exact_committed_cancellation_even_with_live_owner() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("permission-wait.db");
    let identity = fixture(&path, Some(std::process::id()), true);
    let mut application = CliApplicationPort::persistent(&path).unwrap();
    assert!(!crate::web::permission_wait_stop_committed(&mut application,
        &identity.conversation_id, &identity.expected_run_id).unwrap());
    let _: nola_application::RunCancellationReceipt = application.execute(
        ApplicationCommand::RequestRunCancellation { input: nola_application::RequestRunCancellation {
            conversation_id: identity.conversation_id.clone(), expected_run_id: identity.expected_run_id.clone(),
            assistant_message_id: identity.assistant_message_id.clone(), cancellation_id: "cancel-permission".into(),
            reason: None, requested_at_ms: chrono::Utc::now().timestamp_millis(),
        }}).unwrap();
    assert!(crate::web::permission_wait_stop_committed(&mut application,
        &identity.conversation_id, &identity.expected_run_id).unwrap());
    assert!(!crate::web::permission_wait_stop_committed(&mut application,
        &identity.conversation_id, "another-run").unwrap());
    assert!(application.active_run(&identity.conversation_id).unwrap().is_some(),
        "waking a wait is not evidence that its live target stopped");
}

/// Explicit operator harness for an isolated real Host failure. It starts only
/// the Application service and cancel-only driver; never a model/tool worker.
#[test]
#[ignore = "requires explicit isolated NOLA_EXACT_CANCELLATION_COMMAND and NOLA_EXACT_CANCELLATION_DB"]
fn recorded_isolated_run_cancels_through_one_durable_service_receipt() {
    let path = std::env::var("NOLA_EXACT_CANCELLATION_DB").unwrap();
    assert!(path.contains("/nola-a2ui-real-"), "only an isolated real-test Store is permitted");
    let command_path = std::env::var("NOLA_EXACT_CANCELLATION_COMMAND").unwrap();
    let input: nola_application::RequestRunCancellation = serde_json::from_str(
        &std::fs::read_to_string(command_path).unwrap()).unwrap();
    let identity = CancellationRunIdentity { conversation_id: input.conversation_id.clone(),
        expected_run_id: input.expected_run_id.clone(), assistant_message_id: input.assistant_message_id.clone() };
    let client = nola_service_client::ApplicationServiceClient::for_database(&path)
        .with_request_timeout(std::time::Duration::from_secs(1200)).unwrap();
    let request_id = format!("isolated-exact-cancel:{}", input.cancellation_id);
    eprintln!("Submitting one exact cancellation receipt; cold replay may take several minutes");
    let receipt: nola_application::RunCancellationReceipt = match client.execute_typed_with_revision(
        &request_id, None, ApplicationCommand::RequestRunCancellation { input }) {
        Ok(receipt) => receipt,
        Err(error) if error.requires_reconciliation() => client.reconcile_command_result(&error).unwrap(),
        Err(error) => panic!("exact cancellation rejected: {error}"),
    };
    eprintln!("Exact cancellation receipt committed; draining cancellation-only effects");
    if receipt.terminal_result.is_none() {
        resume_pending(CliApplicationPort::Service(client.clone()), identity.clone(), &mut |_| {}).unwrap();
    }
    let mut application = CliApplicationPort::Service(client);
    let active = application.active_run(&identity.conversation_id).unwrap();
    assert!(active.is_none(), "cancel-only reconciliation must seal the exact original Run");
}

#[test]
fn resumed_host_checks_exact_control_fence_and_dead_target_owner() {
    for live in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("resumed-cancel.db");
        let identity = fixture(&path, Some(if live { std::process::id() } else { 2_000_000_000 }), true);
        let mut application = CliApplicationPort::persistent(&path).unwrap();
        let now = chrono::Utc::now().timestamp_millis();
        let _: nola_application::RunCancellationReceipt = application.execute(
            ApplicationCommand::RequestRunCancellation { input: nola_application::RequestRunCancellation {
                conversation_id: identity.conversation_id.clone(), expected_run_id: identity.expected_run_id.clone(),
                assistant_message_id: identity.assistant_message_id.clone(), cancellation_id: "cancel".into(),
                reason: None, requested_at_ms: now,
            }}).unwrap();
        let owner = crate::process_observation::test_worker_lease(std::process::id());
        let mut requests = application.claim_committed_host_requests(nola_application::ClaimCommittedHostRequests {
            run_id: identity.expected_run_id.clone(), worker_generation: owner.worker_generation.clone(),
            lease_owner: owner.lease_owner.clone(), claimed_at_ms: now,
            lease_expires_at_ms: now + 60_000, limit: 64,
        }).unwrap().requests;
        let request = requests.remove(0);
        let result = observe_dispatched_target(&mut application, &identity.conversation_id, &identity.expected_run_id, &request);
        if live { assert!(result.unwrap_err().contains("alive")); }
        else {
            let target = result.unwrap();
            assert!(matches!(request.payload, HostRequest::Cancel { ref target_effect_id, .. } if *target_effect_id == target.effect_id));
            for damage in ["request", "fence", "run"] {
                let mut changed = request.clone();
                if damage == "request" { changed.request_id.push_str("-wrong"); }
                if damage == "fence" { changed.delivery.as_mut().unwrap().lease_token.push_str("-wrong"); }
                assert!(observe_dispatched_target(&mut application, &identity.conversation_id,
                    if damage == "run" { "wrong" } else { &identity.expected_run_id }, &changed).is_err());
            }
        }
    }
}

#[test]
fn living_idle_cancel_worker_does_not_block_its_expired_unstarted_delivery() {
    for expired in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("idle-cancel-worker.db");
        let identity = fixture(&path, None, false);
        let mut application = CliApplicationPort::persistent(&path).unwrap();
        let now = chrono::Utc::now().timestamp_millis();
        let _: nola_application::RunCancellationReceipt = application.execute(
            ApplicationCommand::RequestRunCancellation {
                input: nola_application::RequestRunCancellation {
                    conversation_id: identity.conversation_id.clone(),
                    expected_run_id: identity.expected_run_id.clone(),
                    assistant_message_id: identity.assistant_message_id.clone(),
                    cancellation_id: "cancel".into(), reason: None, requested_at_ms: now - 120_000,
                },
            },
        ).unwrap();
        let observed: RunCancellationDeliveryFacts = application.execute(
            ApplicationCommand::LoadRunCancellationDeliveryFacts { input: identity.clone() },
        ).unwrap();
        let owner = crate::process_observation::test_worker_lease(std::process::id());
        let requests: nola_application::CommittedHostRequests = application.execute(
            ApplicationCommand::ClaimCancellationHostRequests {
                input: ClaimCancellationHostRequests {
                    identity: identity.clone(), observed,
                    worker_generation: owner.worker_generation.clone(),
                    lease_owner: owner.lease_owner.clone(),
                    claimed_at_ms: now - 60_000,
                    lease_expires_at_ms: if expired { now - 1 } else { now + 60_000 },
                    limit: 64,
                },
            },
        ).unwrap();
        assert!(!requests.requests.is_empty());
        let result = resume_pending(application, identity.clone(), &mut |_| {});
        if expired {
            result.unwrap();
            assert!(matches!(inspect(&path, &identity).observation,
                nola_domain::RunRecoveryObservation::Terminal { .. }));
        } else {
            assert!(result.unwrap_err().contains("alive"));
            assert!(matches!(inspect(&path, &identity).observation,
                nola_domain::RunRecoveryObservation::Active { .. }));
        }
    }
}

#[test]
#[ignore = "requires NOLA_RECOVERY_SOURCE_DB and NOLA_RECOVERY_CONVERSATION; copies source read-only"]
fn recorded_failed_run_cancels_to_a_real_terminal_seal_without_model_or_tools() {
    let source = std::env::var("NOLA_RECOVERY_SOURCE_DB").unwrap();
    let conversation = std::env::var("NOLA_RECOVERY_CONVERSATION").unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("recorded-cancellation.db");
    assert!(std::process::Command::new("sqlite3")
        .args(["-readonly", &source, &format!(".backup '{}'", path.display())])
        .status().unwrap().success());
    let db = rusqlite::Connection::open(&path).unwrap();
    let (run, assistant): (String, String) = db.query_row(
        "SELECT id,assistant_message_id FROM runs WHERE conversation_id=?1 AND status='running' ORDER BY created_at_ms DESC LIMIT 1",
        [&conversation], |row| Ok((row.get(0)?, row.get(1)?))).unwrap();
    drop(db);
    let identity = CancellationRunIdentity { conversation_id: conversation,
        expected_run_id: run, assistant_message_id: assistant };
    cancel(CliApplicationPort::persistent(&path).unwrap().with_service_receipts(),
        identity.clone(), "isolated-recorded-cancel", &mut |_| {}).unwrap();
    assert!(matches!(inspect(&path, &identity).observation,
        nola_domain::RunRecoveryObservation::Terminal {
            result: nola_domain::run_result::RunResult {
                value: nola_domain::run_result::RunResultValue::Cancelled { .. }, .. }, ..
        }));
}

#[test]
fn normal_cold_resume_finishes_committed_cancellation_only_after_owner_death() {
    let _environment = crate::store::test_env_lock();
    for live in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("cold-cancellation.db");
        let pid = if live { std::process::id() } else {
            let mut child = crate::process_observation::test_exited_child();
            let pid = child.id();
            child.wait().unwrap();
            pid
        };
        let identity = fixture(&path, Some(pid), true);
        let mut application = CliApplicationPort::persistent(&path).unwrap();
        let _: nola_application::RunCancellationReceipt = application.execute(ApplicationCommand::RequestRunCancellation {
            input: nola_application::RequestRunCancellation {
                conversation_id: identity.conversation_id.clone(), expected_run_id: identity.expected_run_id.clone(),
                assistant_message_id: identity.assistant_message_id.clone(), cancellation_id: "already-cancelling".into(),
                reason: Some("fixture".into()), requested_at_ms: chrono::Utc::now().timestamp_millis(),
            },
        }).unwrap();
        let previous = std::env::var_os("NOLA_DB_PATH");
        std::env::set_var("NOLA_DB_PATH", &path);
        let mut emitter = crate::native_runtime_output::NativeRuntimeOutput::with_event_sink(|_| {});
        let result = super::super::resume_cold_cancellation(&identity.conversation_id, &identity.expected_run_id, &mut emitter);
        match previous { Some(value) => std::env::set_var("NOLA_DB_PATH", value), None => std::env::remove_var("NOLA_DB_PATH") }
        if live {
            assert!(result.unwrap_err().contains("alive"));
            assert!(matches!(inspect(&path, &identity).observation, nola_domain::RunRecoveryObservation::Active { .. }));
        } else {
            assert!(result.unwrap());
            assert!(matches!(inspect(&path, &identity).observation, nola_domain::RunRecoveryObservation::Terminal { .. }));
        }
    }
}

#[test]
#[ignore = "requires read-only recorded Store; exercises ordinary cold-cancellation routing on a copy"]
fn recorded_cancelling_run_resumes_to_terminal_without_normal_host() {
    let _environment = crate::store::test_env_lock();
    let source = std::env::var("NOLA_RECOVERY_SOURCE_DB").unwrap();
    let conversation = std::env::var("NOLA_RECOVERY_CONVERSATION").unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("recorded-resume.db");
    assert!(std::process::Command::new("sqlite3").args(["-readonly", &source, &format!(".backup '{}'", path.display())]).status().unwrap().success());
    let mut application = CliApplicationPort::persistent(&path).unwrap();
    let active = application.active_run(&conversation).unwrap().unwrap();
    let identity = CancellationRunIdentity { conversation_id: conversation.clone(), expected_run_id: active.run_id.clone(),
        assistant_message_id: active.assistant_message_id.unwrap() };
    let previous = std::env::var_os("NOLA_DB_PATH");
    std::env::set_var("NOLA_DB_PATH", &path);
    let mut emitter = crate::native_runtime_output::NativeRuntimeOutput::with_event_sink(|_| {});
    let result = super::super::resume_cold_cancellation(&conversation, &active.run_id, &mut emitter);
    match previous { Some(value) => std::env::set_var("NOLA_DB_PATH", value), None => std::env::remove_var("NOLA_DB_PATH") }
    assert!(result.unwrap());
    assert!(super::super::active_core_agent_host(&conversation).unwrap().is_none());
    assert!(matches!(inspect(&path, &identity).observation, nola_domain::RunRecoveryObservation::Terminal {
        result: nola_domain::run_result::RunResult { value: nola_domain::run_result::RunResultValue::Cancelled { .. }, .. }, .. }));
}

fn fixture(path: &std::path::Path, owner: Option<u32>, started: bool) -> CancellationRunIdentity {
    let mut application = CliApplicationPort::persistent(path)
        .unwrap()
        .with_service_receipts();
    let capability = HostCapabilityDescriptor {
        id: "model.stream".into(),
        title: "Fixture".into(),
        route: HostCapabilityRoute::Model,
        availability: CapabilityAvailability::Available,
        exposure: HostCapabilityExposure::Internal,
        permission_domain: None,
        input_schema: serde_json::json!({"type":"object"}),
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
    let begun = application
        .begin_assistant_turn(nola_application::BeginAssistantTurn {
            expected_configuration_revision: None, expected_parent_message_id: None,
            conversation_id: "conversation".into(),
            conversation_title: "Cancel".into(),
            user_message_id: "user".into(),
            user_part_id: "user-part".into(),
            user_text: "Work".into(),
            user_content_json: None,
            assistant_message_id: "assistant".into(),
            context_checkpoint_id: None,
            core_config: AgentCoreConfig {
                turn_identity: crate::test_model_identity::identity(),
                session_id: Some(SessionId::from_string("conversation".into())),
                model: Some("fixture".into()),
                host_catalog: Some(HostCapabilityCatalog {
                    host_id: HostId("fixture".into()),
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
                    "Work",
                )],
                turn_context_items: vec![],
                turn_context_platform: Default::default(),
                turn_context_components: vec![],
                submission_id: Some("run".into()),
                submitted_at_epoch_ms: Some(1000),
                delivery: AgentInputDelivery::CurrentTurn,
            },
            created_at_ms: 1000,
        })
        .unwrap();
    if let Some(pid) = owner {
        let now = chrono::Utc::now().timestamp_millis();
        let owner = crate::process_observation::test_worker_lease(pid);
        let request = application
            .claim_committed_host_requests(nola_application::ClaimCommittedHostRequests {
                run_id: begun.begin.run_id.clone(),
                worker_generation: owner.worker_generation.clone(),
                lease_owner: owner.lease_owner.clone(),
                claimed_at_ms: now,
                lease_expires_at_ms: now + 60_000,
                limit: 1,
            })
            .unwrap()
            .requests
            .remove(0);
        if started {
            application
                .reduce_and_accept_host_response(nola_application::ReduceAndAcceptHostResponse {
                    conversation_id: "conversation".into(),
                    run_id: begun.begin.run_id.clone(),
                    invocation_config_digest: begun.invocation_config_digest,
                    response: HostResponseEnvelope {
                        attempt_id: request.attempt_id,
                        delegated_invocation_id: request.delegated_invocation_id,
                        request_id: request.request_id,
                        effect_id: request.effect_id,
                        host_id: request.host_id,
                        lease_id: request.lease_id,
                        session_id: request.session_id,
                        turn_id: request.turn_id,
                        catalog_revision: request.catalog_revision,
                        availability_epoch: request.availability_epoch,
                        delivery: request.delivery,
                        sequence: 0,
                        payload: HostResponse::ExecutionStarted,
                    },
                    received_at_ms: now,
                })
                .unwrap();
        }
    }
    CancellationRunIdentity {
        conversation_id: "conversation".into(),
        expected_run_id: begun.begin.run_id,
        assistant_message_id: "assistant".into(),
    }
}

fn inspect(
    path: &std::path::Path,
    identity: &CancellationRunIdentity,
) -> nola_domain::RunRecoveryInspection {
    CliApplicationPort::persistent(path)
        .unwrap()
        .execute(ApplicationCommand::InspectRunRecovery {
            conversation_id: identity.conversation_id.clone(),
            expected_run_id: identity.expected_run_id.clone(),
            assistant_message_id: identity.assistant_message_id.clone(),
        })
        .unwrap()
}

#[test]
fn never_started_model_cancels_without_provider_or_normal_host() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("cancel.db");
    let identity = fixture(&path, None, false);
    cancel(
        CliApplicationPort::persistent(&path)
            .unwrap()
            .with_service_receipts(),
        identity.clone(),
        "cancel",
        &mut |_| {},
    )
    .unwrap();
    assert!(matches!(
        inspect(&path, &identity).observation,
        nola_domain::RunRecoveryObservation::Terminal {
            result: nola_domain::run_result::RunResult { value: nola_domain::run_result::RunResultValue::Cancelled { .. }, .. },
            ..
        }
    ));
}

#[test]
fn real_exited_model_owner_can_be_cancelled_but_live_owner_stays_pending() {
    for live in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("owner.db");
        let pid = if live {
            std::process::id()
        } else {
            let mut child = crate::process_observation::test_exited_child();
            let pid = child.id();
            assert!(child.wait().unwrap().success());
            pid
        };
        let identity = fixture(&path, Some(pid), true);
        let result = cancel(
            CliApplicationPort::persistent(&path)
                .unwrap()
                .with_service_receipts(),
            identity.clone(),
            "cancel",
            &mut |_| {},
        );
        if live {
            assert!(result.unwrap_err().contains("alive"));
            assert!(matches!(
                inspect(&path, &identity).observation,
                nola_domain::RunRecoveryObservation::Active { .. }
            ));
        } else {
            result.unwrap();
            assert!(matches!(
                inspect(&path, &identity).observation,
                nola_domain::RunRecoveryObservation::Terminal {
                    result: nola_domain::run_result::RunResult { value: nola_domain::run_result::RunResultValue::Cancelled { .. }, .. },
                    ..
                }
            ));
        }
    }
}

#[test]
fn live_historical_owner_blocks_cancellation_even_when_latest_owner_exited() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("historical-owner.db");
    let mut child = crate::process_observation::test_exited_child();
    let pid = child.id(); child.wait().unwrap();
    let identity = fixture(&path, Some(pid), true);
    let mut application = CliApplicationPort::persistent(&path).unwrap();
    let _: nola_application::RunCancellationReceipt = application.execute(ApplicationCommand::RequestRunCancellation {
        input: nola_application::RequestRunCancellation {
            conversation_id: identity.conversation_id.clone(), expected_run_id: identity.expected_run_id.clone(),
            assistant_message_id: identity.assistant_message_id.clone(), cancellation_id: "history-proof".into(),
            reason: None, requested_at_ms: chrono::Utc::now().timestamp_millis(),
        },
    }).unwrap();
    let facts: RunCancellationDeliveryFacts = application.execute(ApplicationCommand::LoadRunCancellationDeliveryFacts { input: identity }).unwrap();
    let mut fact = facts.targets.into_iter().find(|fact| fact.latest_attempt.is_some()).unwrap();
    observe_target_owners(&fact).unwrap();
    let mut prior = fact.latest_attempt.clone().unwrap();
    let owner = crate::process_observation::test_worker_lease(std::process::id());
    prior.worker_generation = Some(owner.worker_generation.clone());
    prior.lease_owner = Some(owner.lease_owner.clone());
    fact.prior_attempts.push(prior);
    assert!(observe_target_owners(&fact).unwrap_err().contains("alive"));
}

#[test]
fn wrong_identity_never_commits_interrupt() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("wrong.db");
    let identity = fixture(&path, None, false);
    let mut wrong = identity.clone();
    wrong.assistant_message_id = "other".into();
    assert!(cancel(
        CliApplicationPort::persistent(&path)
            .unwrap()
            .with_service_receipts(),
        wrong,
        "cancel",
        &mut |_| {}
    )
    .is_err());
    let requests = CliApplicationPort::persistent(&path)
        .unwrap()
        .execute::<nola_application::CommittedHostRequests>(
            ApplicationCommand::LoadCommittedHostRequests {
                run_id: identity.expected_run_id,
            },
        )
        .unwrap();
    assert!(requests
        .requests
        .iter()
        .all(|request| !matches!(request.payload, HostRequest::Cancel { .. })));
}

#[test]
fn stale_claim_facts_cannot_lease_cancellation_or_data_plane_work() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("claim.db");
    let identity = fixture(&path, None, false);
    let mut application = CliApplicationPort::persistent(&path)
        .unwrap()
        .with_service_receipts();
    let _: nola_application::RunCancellationReceipt = application
        .execute(ApplicationCommand::RequestRunCancellation {
            input: nola_application::RequestRunCancellation {
                conversation_id: identity.conversation_id.clone(),
                expected_run_id: identity.expected_run_id.clone(),
                assistant_message_id: identity.assistant_message_id.clone(),
                cancellation_id: "cancel".into(),
                reason: None,
                requested_at_ms: chrono::Utc::now().timestamp_millis(),
            },
        })
        .unwrap();
    let read = |application: &mut CliApplicationPort| {
        application
            .execute::<RunCancellationDeliveryFacts>(
                ApplicationCommand::LoadRunCancellationDeliveryFacts {
                    input: identity.clone(),
                },
            )
            .unwrap()
    };
    let original = read(&mut application);
    assert!(!original.targets.is_empty());
    assert!(original
        .targets
        .iter()
        .all(|target| target.latest_attempt.is_none() && target.cancellation_attempt.is_none()));
    let mut stale = original.clone();
    stale.checkpoint_revision -= 1;
    let now = chrono::Utc::now().timestamp_millis();
    let claim = |observed| ApplicationCommand::ClaimCancellationHostRequests {
        input: ClaimCancellationHostRequests {
            identity: identity.clone(),
            observed,
            worker_generation: "fixture-worker".into(),
            lease_owner: "fixture-owner".into(),
            claimed_at_ms: now,
            lease_expires_at_ms: now + 60_000,
            limit: 64,
        },
    };
    assert!(application
        .execute::<nola_application::CommittedHostRequests>(claim(stale))
        .is_err());
    assert_eq!(
        read(&mut application),
        original,
        "rejected evidence must leave attempt ledger unchanged"
    );
    let requests: nola_application::CommittedHostRequests =
        application.execute(claim(original)).unwrap();
    assert!(!requests.requests.is_empty());
    assert!(requests.requests.iter().all(|request| matches!(
        request.payload,
        HostRequest::Cancel { .. } | HostRequest::CancelTask { .. }
    )));
    let after = read(&mut application);
    assert!(
        after
            .targets
            .iter()
            .all(|target| target.latest_attempt.is_none()),
        "original model work must never be leased by cancel-only admission"
    );
    assert!(after
        .targets
        .iter()
        .any(|target| target.cancellation_attempt.is_some()));
}

#[test]
fn started_external_target_requires_independent_shutdown_evidence() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("external.db");
    let mut child = crate::process_observation::test_exited_child();
    let dead_pid = child.id();
    child.wait().unwrap();
    let identity = fixture(&path, Some(dead_pid), true);
    let mut application = CliApplicationPort::persistent(&path)
        .unwrap()
        .with_service_receipts();
    let _: nola_application::RunCancellationReceipt = application
        .execute(ApplicationCommand::RequestRunCancellation {
            input: nola_application::RequestRunCancellation {
                conversation_id: identity.conversation_id.clone(),
                expected_run_id: identity.expected_run_id.clone(),
                assistant_message_id: identity.assistant_message_id.clone(),
                cancellation_id: "cancel".into(),
                reason: None,
                requested_at_ms: chrono::Utc::now().timestamp_millis(),
            },
        })
        .unwrap();
    let mut facts: RunCancellationDeliveryFacts = application
        .execute(ApplicationCommand::LoadRunCancellationDeliveryFacts { input: identity })
        .unwrap();
    let fact = facts.targets.first_mut().unwrap();
    assert!(fact
        .latest_attempt
        .as_ref()
        .unwrap()
        .execution_started_at_ms
        .is_some());
    assert!(nola_application::cancellation_delivery::supports_dead_owner_confirmation(fact));
    // Pure policy boundary, not a claim that an external process was stopped.
    fact.target.as_mut().unwrap().payload = HostRequest::ExecuteTool {
        permission_profile: nola_domain::PermissionProfile::Cautious,
        permission_instructions: vec![],
        capability_id: "shell".into(),
        call_id: "call".into(),
        input: serde_json::json!({}),
        grant: None,
    };
    assert!(!nola_application::cancellation_delivery::supports_dead_owner_confirmation(fact));
    assert!(observe_target_owners(fact).unwrap_err()
        .contains("independent cancellation reconciliation"));
    assert!(!matches!(inspect(&path, &facts.identity).observation,
        nola_domain::RunRecoveryObservation::Terminal { .. }),
        "An exited owner alone cannot seal an external target");
}

#[test]
fn cancelled_dead_session_owner_does_not_block_the_next_run_lease() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("session-owner.db");
    let mut child = crate::process_observation::test_exited_child();
    let dead_pid = child.id();
    assert!(child.wait().unwrap().success());
    let identity = fixture(&path, Some(dead_pid), true);
    let frozen = {
        let store = nola_store::NolaStore::open(&path).unwrap();
        nola_store_port::run_effect::RunEffectRepository::load_run_invocation_config(
            &store,
            &identity.expected_run_id,
        )
        .unwrap()
        .unwrap()
    };
    let mut application = CliApplicationPort::persistent(&path)
        .unwrap()
        .with_service_receipts();
    let old = nola_application::SessionRunLease {
        session_id: identity.conversation_id.clone(),
        owner: "original-session-owner".into(),
        pid: dead_pid,
        process_start: "isolated-exited-child".into(),
        acquired_at_ms: 1000,
    };
    assert!(application
        .execute::<bool>(ApplicationCommand::AcquireSessionRunLease { input: old })
        .unwrap());
    cancel(application, identity.clone(), "cancel", &mut |_| {}).unwrap();
    assert!(matches!(
        inspect(&path, &identity).observation,
        nola_domain::RunRecoveryObservation::Terminal {
            result: nola_domain::run_result::RunResult { value: nola_domain::run_result::RunResultValue::Cancelled { .. }, .. },
            ..
        }
    ));
    let mut next = CliApplicationPort::persistent(&path)
        .unwrap()
        .with_service_receipts();
    let acquired: bool = next
        .execute(ApplicationCommand::AcquireSessionRunLease {
            input: nola_application::SessionRunLease {
                session_id: identity.conversation_id.clone(),
                owner: "next-session-owner".into(),
                pid: std::process::id(),
                process_start: "current-test-process".into(),
                acquired_at_ms: chrono::Utc::now().timestamp_millis(),
            },
        })
        .unwrap();
    assert!(
        acquired,
        "completed cancellation must not leave its dead owner blocking the next Run lease"
    );
    let frontier = next.load_conversation(&identity.conversation_id).unwrap().unwrap();
    let next_run = next
        .begin_assistant_turn(nola_application::BeginAssistantTurn {
            expected_configuration_revision: Some(frontier.configuration_revision),
            expected_parent_message_id: frontier.messages.last().map(|message| message.id.clone()),
            conversation_id: identity.conversation_id.clone(),
            conversation_title: "Next Run".into(),
            user_message_id: "next-user".into(),
            user_part_id: "next-user:text".into(),
            user_text: "Next".into(),
            user_content_json: None,
            assistant_message_id: "next-assistant".into(),
            context_checkpoint_id: None,
            core_config: serde_json::from_str(&frozen.invocation_config_json).unwrap(),
            submission: AgentSubmission::UserInput {
                items: vec![AgentItem::user_message(
                    ItemId::from_string("next-user".into()),
                    "Next",
                )],
                turn_context_items: vec![],
                turn_context_platform: Default::default(),
                turn_context_components: vec![],
                submission_id: Some("next-run".into()),
                submitted_at_epoch_ms: Some(chrono::Utc::now().timestamp_millis()),
                delivery: AgentInputDelivery::CurrentTurn,
            },
            created_at_ms: chrono::Utc::now().timestamp_millis(),
        })
        .unwrap();
    assert_eq!(
        next.active_run(&identity.conversation_id)
            .unwrap()
            .unwrap()
            .run_id,
        next_run.begin.run_id
    );
    assert_ne!(next_run.begin.run_id, identity.expected_run_id);
}

#[test]
fn cancellation_claim_receipt_failure_rolls_back_owner_release_and_attempts() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("claim-rollback.db");
    let identity = fixture(&path, None, false);
    let mut application = CliApplicationPort::persistent(&path)
        .unwrap()
        .with_service_receipts();
    let mut child = crate::process_observation::test_exited_child();
    let pid = child.id();
    child.wait().unwrap();
    let lease = nola_application::SessionRunLease {
        session_id: identity.conversation_id.clone(),
        owner: "original".into(),
        pid,
        process_start: "exited-child".into(),
        acquired_at_ms: 1000,
    };
    assert!(application
        .execute::<bool>(ApplicationCommand::AcquireSessionRunLease { input: lease })
        .unwrap());
    let _: nola_application::RunCancellationReceipt = application
        .execute(ApplicationCommand::RequestRunCancellation {
            input: nola_application::RequestRunCancellation {
                conversation_id: identity.conversation_id.clone(),
                expected_run_id: identity.expected_run_id.clone(),
                assistant_message_id: identity.assistant_message_id.clone(),
                cancellation_id: "cancel".into(),
                reason: None,
                requested_at_ms: chrono::Utc::now().timestamp_millis(),
            },
        })
        .unwrap();
    let before: RunCancellationDeliveryFacts = application
        .execute(ApplicationCommand::LoadRunCancellationDeliveryFacts {
            input: identity.clone(),
        })
        .unwrap();
    let now = chrono::Utc::now().timestamp_millis();
    let input = ClaimCancellationHostRequests {
        identity: identity.clone(),
        observed: before.clone(),
        worker_generation: "fixture-worker".into(),
        lease_owner: "fixture-owner".into(),
        claimed_at_ms: now,
        lease_expires_at_ms: now + 60_000,
        limit: 64,
    };
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TRIGGER fail_cancel_claim_receipt BEFORE UPDATE ON command_receipts WHEN NEW.status='committed' AND NEW.command_type='claim_cancellation_host_requests' BEGIN SELECT RAISE(ABORT,'cancel claim receipt fault'); END;").unwrap();
    let error = application
        .execute::<nola_application::CommittedHostRequests>(
            ApplicationCommand::ClaimCancellationHostRequests {
                input: input.clone(),
            },
        )
        .unwrap_err();
    assert!(error.contains("cancel claim receipt fault"), "{error}");
    let after: RunCancellationDeliveryFacts = application
        .execute(ApplicationCommand::LoadRunCancellationDeliveryFacts {
            input: identity.clone(),
        })
        .unwrap();
    assert_eq!(before, after, "all five lease fields, target attempt, cancellation claims and checkpoint must roll back together");
    db.execute_batch("DROP TRIGGER fail_cancel_claim_receipt")
        .unwrap();
    let requests: nola_application::CommittedHostRequests = application
        .execute(ApplicationCommand::ClaimCancellationHostRequests { input })
        .unwrap();
    assert!(!requests.requests.is_empty());
    let committed: RunCancellationDeliveryFacts = application
        .execute(ApplicationCommand::LoadRunCancellationDeliveryFacts { input: identity })
        .unwrap();
    assert!(committed.session_lease.is_none());
    assert!(committed
        .targets
        .iter()
        .all(|target| target.latest_attempt.is_none()));
    assert!(committed
        .targets
        .iter()
        .any(|target| target.cancellation_attempt.is_some()));
}

#[test]
fn cancellation_claim_direct_apis_require_an_outer_write_unit_of_work() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("direct-claim.db");
    let identity = fixture(&path, None, false);
    let mut received = CliApplicationPort::persistent(&path)
        .unwrap()
        .with_service_receipts();
    let _: nola_application::RunCancellationReceipt = received
        .execute(ApplicationCommand::RequestRunCancellation {
            input: nola_application::RequestRunCancellation {
                conversation_id: identity.conversation_id.clone(),
                expected_run_id: identity.expected_run_id.clone(),
                assistant_message_id: identity.assistant_message_id.clone(),
                cancellation_id: "cancel".into(),
                reason: None,
                requested_at_ms: chrono::Utc::now().timestamp_millis(),
            },
        })
        .unwrap();
    let before: RunCancellationDeliveryFacts = received
        .execute(ApplicationCommand::LoadRunCancellationDeliveryFacts {
            input: identity.clone(),
        })
        .unwrap();
    let now = chrono::Utc::now().timestamp_millis();
    let claim = ClaimCancellationHostRequests {
        identity: identity.clone(),
        observed: before.clone(),
        worker_generation: "fixture-worker".into(),
        lease_owner: "fixture-owner".into(),
        claimed_at_ms: now,
        lease_expires_at_ms: now + 60_000,
        limit: 64,
    };
    // The CLI port now supplies the received-command transaction in either
    // transport mode. Exercise the raw Application API for this precondition.
    let mut direct = nola_application_bootstrap::open(&path).unwrap();
    let error = direct
        .execute(
            ApplicationCommand::ClaimCancellationHostRequests {
                input: claim.clone(),
            },
        )
        .unwrap_err();
    assert!(error.contains("write unit of work"), "{error}");
    let mut store = nola_store::NolaStore::open(&path).unwrap();
    let error = nola_application::cancellation_delivery::claim_cancellation_host_requests(
        &mut store,
        claim.clone(),
    )
    .unwrap_err();
    assert!(error.contains("write unit of work"), "{error}");
    let after: RunCancellationDeliveryFacts = received
        .execute(ApplicationCommand::LoadRunCancellationDeliveryFacts { input: identity })
        .unwrap();
    assert_eq!(before, after);
    let claimed: nola_application::CommittedHostRequests = received
        .execute(ApplicationCommand::ClaimCancellationHostRequests { input: claim })
        .unwrap();
    assert!(!claimed.requests.is_empty());
}

#[test]
fn cancellation_reads_only_current_target_after_more_than_256_historical_effects() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("long-history.db");
    let mut child = crate::process_observation::test_exited_child();
    let dead_pid = child.id();
    assert!(child.wait().unwrap().success());
    let identity = fixture(&path, Some(dead_pid), true);
    let mut application = CliApplicationPort::persistent(&path)
        .unwrap()
        .with_service_receipts();
    let requests: nola_application::CommittedHostRequests = application
        .execute(ApplicationCommand::LoadCommittedHostRequests {
            run_id: identity.expected_run_id.clone(),
        })
        .unwrap();
    let current_request = requests.requests.into_iter().next().unwrap();
    let mut store = nola_store::NolaStore::open(&path).unwrap();
    let current_attempt = store
        .active_effect_delivery_attempt(&current_request.effect_id.0)
        .unwrap()
        .unwrap();
    let db = rusqlite::Connection::open(&path).unwrap();
    let mut history = Vec::new();
    // Build a legal settled-history fixture, not fabricated cancellation proof.
    // The production Store validates every typed attempt before persistence;
    // outbox acknowledgement uses its normal storage operation.
    for index in 0..300 {
        let mut historical_request = current_request.clone();
        historical_request.effect_id = HostEffectId(format!("historical-effect-{index}"));
        historical_request.request_id = format!("historical-request-{index}");
        historical_request.delivery = None;
        use sha2::Digest;
        let original = serde_json::to_string(&historical_request).unwrap();
        let binding = serde_json::to_string(&nola_store_port::host_replay::CoreHostBindingOverlay::from_request(&historical_request)).unwrap();
        db.execute("INSERT INTO effect_outbox(effect_id,run_id,request_id,effect_kind,binding_json,payload_digest,status,created_at_ms) VALUES(?1,?2,?3,'model.stream',?4,?5,'pending',1000)",
            rusqlite::params![historical_request.effect_id.0, identity.expected_run_id,
                historical_request.request_id, binding, format!("{:x}",sha2::Sha256::digest(original.as_bytes()))]).unwrap();
        db.execute("INSERT INTO effect_outbox_payloads VALUES(?1,?2)", rusqlite::params![historical_request.effect_id.0,original]).unwrap();
        let mut settled = current_attempt.clone();
        settled.id = format!("historical-attempt-{index}");
        settled.effect_id = historical_request.effect_id.0;
        settled.request_id = historical_request.request_id;
        settled.state = nola_domain::EffectDeliveryAttemptState::Settled;
        settled.settlement = Some(nola_domain::EffectDeliverySettlement::Succeeded);
        settled.finished_at_ms = settled.execution_started_at_ms;
        settled.revision += 1;
        store
            .commit_effect_delivery_attempt_mutations(&[
                nola_domain::EffectDeliveryAttemptMutation {
                    expected_revision: None,
                    record: settled.clone(),
                },
            ])
            .unwrap();
        assert!(store
            .acknowledge_effect(&settled.effect_id, settled.finished_at_ms.unwrap())
            .unwrap());
        history.push(settled);
    }
    assert_eq!(
        db.query_row(
            "SELECT COUNT(DISTINCT effect_id) FROM effect_delivery_attempts WHERE run_id=?1",
            [&identity.expected_run_id],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        301
    );
    let _: nola_application::RunCancellationReceipt = application
        .execute(ApplicationCommand::RequestRunCancellation {
            input: nola_application::RequestRunCancellation {
                conversation_id: identity.conversation_id.clone(),
                expected_run_id: identity.expected_run_id.clone(),
                assistant_message_id: identity.assistant_message_id.clone(),
                cancellation_id: "long-cancel".into(),
                reason: None,
                requested_at_ms: chrono::Utc::now().timestamp_millis(),
            },
        })
        .unwrap();
    let observed: RunCancellationDeliveryFacts = application
        .execute(ApplicationCommand::LoadRunCancellationDeliveryFacts {
            input: identity.clone(),
        })
        .unwrap();
    assert_eq!(observed.targets.len(), 1);
    assert_eq!(
        observed.targets[0].target.as_ref().unwrap().effect_id,
        current_request.effect_id
    );
    assert_eq!(
        observed.targets[0].latest_attempt.as_ref(),
        Some(&current_attempt)
    );
    cancel(application, identity.clone(), "long-cancel", &mut |_| {}).unwrap();
    assert!(matches!(
        inspect(&path, &identity).observation,
        nola_domain::RunRecoveryObservation::Terminal {
            result: nola_domain::run_result::RunResult { value: nola_domain::run_result::RunResultValue::Cancelled { .. }, .. },
            ..
        }
    ));
    assert!(
        store
            .pending_effects(&identity.expected_run_id)
            .unwrap()
            .is_empty(),
        "cancel-only must not schedule a new model request"
    );
    for expected in history {
        assert_eq!(
            store
                .effect_delivery_attempt(&expected.id)
                .unwrap()
                .as_ref(),
            Some(&expected),
            "unrelated historical attempts must remain byte-equivalent typed records"
        );
    }
}

fn local_process_tool_fixture(path: &std::path::Path, owner: Option<u32>, started: bool) -> CancellationRunIdentity {
    let mut application = CliApplicationPort::persistent(path)
        .unwrap()
        .with_service_receipts();
    let capability = HostCapabilityDescriptor {
        id: "model.stream".into(),
        title: "Fixture".into(),
        route: HostCapabilityRoute::Model,
        availability: CapabilityAvailability::Available,
        exposure: HostCapabilityExposure::Internal,
        permission_domain: None,
        input_schema: serde_json::json!({"type":"object"}),
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
    let mut local = capability.clone();
    local.id = "fixture.local.read".into();
    local.route = HostCapabilityRoute::Tool;
    local.exposure = HostCapabilityExposure::Visible;
    local.cancellation = HostCancellationMode::ProcessBound;
    local.replay_safety = HostReplaySafety::Idempotent;
    let local_schema = serde_json::json!({"type":"function","function":{"name":local.id,"parameters":{"type":"object","properties":{}}}});
    let begun = application
        .begin_assistant_turn(nola_application::BeginAssistantTurn {
            expected_configuration_revision: None, expected_parent_message_id: None,
            conversation_id: "conversation".into(),
            conversation_title: "Cancel".into(),
            user_message_id: "user".into(),
            user_part_id: "user-part".into(),
            user_text: "Work".into(),
            user_content_json: None,
            assistant_message_id: "assistant".into(),
            context_checkpoint_id: None,
            core_config: AgentCoreConfig {
                turn_identity: crate::test_model_identity::identity(),
                session_id: Some(SessionId::from_string("conversation".into())),
                model: Some("fixture".into()),
                tools: vec![local_schema],
                permission_rules: nola_core::agent::permissions::PermissionRuleSet { rules: vec![nola_core::agent::permissions::PermissionRule {
                    capability_pattern:"fixture.local.read".into(), resource_pattern:"*".into(),
                    decision:nola_core::agent::permissions::PermissionPolicyDecision::Allow,
                    source:nola_core::agent::permissions::PermissionRuleSource::Host,
                }] },
                host_catalog: Some(HostCapabilityCatalog {
                    host_id: HostId("fixture".into()),
                    revision: 1,
                    availability_epoch: 1,
                    lease: None,
                    capabilities: [("model.stream".into(), capability), (local.id.clone(), local)].into_iter().collect(),
                }),
                ..Default::default()
            },
            submission: AgentSubmission::UserInput {
                items: vec![AgentItem::user_message(
                    ItemId::from_string("user".into()),
                    "Work",
                )],
                turn_context_items: vec![],
                turn_context_platform: Default::default(),
                turn_context_components: vec![],
                submission_id: Some("run".into()),
                submitted_at_epoch_ms: Some(1000),
                delivery: AgentInputDelivery::CurrentTurn,
            },
            created_at_ms: 1000,
        })
        .unwrap();
    if let Some(pid) = owner {
        let now = chrono::Utc::now().timestamp_millis();
        let owner = crate::process_observation::test_worker_lease(pid);
        let request = application
            .claim_committed_host_requests(nola_application::ClaimCommittedHostRequests {
                run_id: begun.begin.run_id.clone(),
                worker_generation: owner.worker_generation.clone(),
                lease_owner: owner.lease_owner.clone(),
                claimed_at_ms: now,
                lease_expires_at_ms: now + 60_000,
                limit: 1,
            })
            .unwrap()
            .requests
            .remove(0);
        let model_request = request.clone();
        if started {
            application
                .reduce_and_accept_host_response(nola_application::ReduceAndAcceptHostResponse {
                    conversation_id: "conversation".into(),
                    run_id: begun.begin.run_id.clone(),
                    invocation_config_digest: begun.invocation_config_digest.clone(),
                    response: HostResponseEnvelope {
                        attempt_id: request.attempt_id,
                        delegated_invocation_id: request.delegated_invocation_id,
                        request_id: request.request_id,
                        effect_id: request.effect_id,
                        host_id: request.host_id,
                        lease_id: request.lease_id,
                        session_id: request.session_id,
                        turn_id: request.turn_id,
                        catalog_revision: request.catalog_revision,
                        availability_epoch: request.availability_epoch,
                        delivery: request.delivery,
                        sequence: 0,
                        payload: HostResponse::ExecutionStarted,
                    },
                    received_at_ms: now,
                })
                .unwrap();
        }

        let request = model_request.clone();
        application.reduce_and_accept_host_response(nola_application::ReduceAndAcceptHostResponse {
            conversation_id: "conversation".into(), run_id: begun.begin.run_id.clone(),
            invocation_config_digest: begun.invocation_config_digest.clone(), received_at_ms: now + 1,
            response: HostResponseEnvelope {
                attempt_id: request.attempt_id, delegated_invocation_id: request.delegated_invocation_id,
                request_id: request.request_id, effect_id: request.effect_id, host_id: request.host_id,
                lease_id: request.lease_id, session_id: request.session_id, turn_id: request.turn_id,
                catalog_revision: request.catalog_revision, availability_epoch: request.availability_epoch,
                delivery: request.delivery, sequence: 1,
                payload: HostResponse::ModelAttemptStarted {provider:"fixture".into(),model:"fixture".into(),reason:"initial".into()},
            },
        }).unwrap();
        let request = model_request;
        application.reduce_and_accept_host_response(nola_application::ReduceAndAcceptHostResponse {
            conversation_id: "conversation".into(), run_id: begun.begin.run_id.clone(),
            invocation_config_digest: begun.invocation_config_digest.clone(), received_at_ms: now + 1,
            response: HostResponseEnvelope {
                attempt_id: request.attempt_id, delegated_invocation_id: request.delegated_invocation_id,
                request_id: request.request_id, effect_id: request.effect_id, host_id: request.host_id,
                lease_id: request.lease_id, session_id: request.session_id, turn_id: request.turn_id,
                catalog_revision: request.catalog_revision, availability_epoch: request.availability_epoch,
                delivery: request.delivery, sequence: 2,
                payload: HostResponse::ModelCompleted { items:vec![], tool_calls:vec![serde_json::json!({"call_id":"read","name":"fixture.local.read","arguments":{},"permission_scope":null,"permission_rationale":null})], usage:serde_json::json!({}) },
            },
        }).unwrap();
        let mut request = application.claim_committed_host_requests(nola_application::ClaimCommittedHostRequests {
            run_id: begun.begin.run_id.clone(), worker_generation: owner.worker_generation.clone(),
            lease_owner: owner.lease_owner.clone(), claimed_at_ms:now+2, lease_expires_at_ms:now+60_000, limit:1,
        }).unwrap().requests.remove(0);
        if let HostRequest::RequestPermission {permission_request_id,..} = request.payload.clone() {
            for (sequence,payload) in [(0,HostResponse::ExecutionStarted),(1,HostResponse::PermissionResolved {request_id:permission_request_id,resolution:HostPermissionResolution::allow_once()})] {
                application.reduce_and_accept_host_response(nola_application::ReduceAndAcceptHostResponse {
                    conversation_id:"conversation".into(),run_id:begun.begin.run_id.clone(),invocation_config_digest:begun.invocation_config_digest.clone(),received_at_ms:now+3,
                    response:HostResponseEnvelope {
                        attempt_id:request.attempt_id.clone(),delegated_invocation_id:request.delegated_invocation_id.clone(),
                        request_id:request.request_id.clone(),effect_id:request.effect_id.clone(),host_id:request.host_id.clone(),
                        lease_id:request.lease_id.clone(),session_id:request.session_id.clone(),turn_id:request.turn_id.clone(),
                        catalog_revision:request.catalog_revision,availability_epoch:request.availability_epoch,delivery:request.delivery.clone(),sequence,payload,
                    },
                }).unwrap();
            }
            request=application.claim_committed_host_requests(nola_application::ClaimCommittedHostRequests {
                run_id:begun.begin.run_id.clone(),worker_generation:owner.worker_generation.clone(),lease_owner:owner.lease_owner.clone(),claimed_at_ms:now+4,lease_expires_at_ms:now+60_000,limit:1,
            }).unwrap().requests.remove(0);
        }
        assert!(matches!(request.payload, HostRequest::ExecuteTool {..}), "unexpected next effect: {:?}",request.payload);
        application.reduce_and_accept_host_response(nola_application::ReduceAndAcceptHostResponse {
            conversation_id:"conversation".into(), run_id:begun.begin.run_id.clone(),
            invocation_config_digest:begun.invocation_config_digest.clone(), received_at_ms:now+3,
            response:HostResponseEnvelope {
                attempt_id: request.attempt_id, delegated_invocation_id:request.delegated_invocation_id,
                request_id:request.request_id, effect_id:request.effect_id, host_id:request.host_id,
                lease_id:request.lease_id, session_id:request.session_id, turn_id:request.turn_id,
                catalog_revision:request.catalog_revision, availability_epoch:request.availability_epoch,
                delivery:request.delivery, sequence:0, payload:HostResponse::ExecutionStarted,
            },
        }).unwrap();
    }
    CancellationRunIdentity {
        conversation_id: "conversation".into(),
        expected_run_id: begun.begin.run_id,
        assistant_message_id: "assistant".into(),
    }
}

#[test]
fn process_bound_local_tool_recovers_after_exit_but_live_owner_stays_pending() {
    for live in [false,true] {
        let directory=tempfile::tempdir().unwrap(); let path=directory.path().join("local-tool.db");
        let pid=if live {std::process::id()} else { let mut child=crate::process_observation::test_exited_child();let id=child.id();child.wait().unwrap();id };
        let identity=local_process_tool_fixture(&path,Some(pid),true);
        let result=cancel(CliApplicationPort::persistent(&path).unwrap().with_service_receipts(),identity.clone(),"cancel-local",&mut |_|{});
        if live {assert!(result.unwrap_err().contains("alive"));}
        else {result.unwrap();assert!(matches!(inspect(&path,&identity).observation,nola_domain::RunRecoveryObservation::Terminal {result:nola_domain::run_result::RunResult {value:nola_domain::run_result::RunResultValue::Cancelled {..},..},..}));}
    }
}
