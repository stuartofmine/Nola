// SPDX-License-Identifier: Apache-2.0
//! Persistent cancellation intent never stands in for Host cancellation proof.
mod support;
use nola_agent_engine::agent::{host::*, protocol::*, AgentThreadState};
use nola_application::*;
use nola_store::NolaStore;

fn admission_input() -> BeginAssistantTurn {
    let capability = HostCapabilityDescriptor {
        id: "model.stream".into(),
        title: "Fixture model".into(),
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
    BeginAssistantTurn {
        expected_configuration_revision: None,
        expected_parent_message_id: None,
        conversation_id: "conversation".into(),
        conversation_title: "Cancel".into(),
        user_message_id: "user".into(),
        user_part_id: "user-part".into(),
        user_text: "Work".into(),
        user_content_json: None,
        assistant_message_id: "assistant".into(),
        context_checkpoint_id: None,
        core_config: AgentCoreConfig {
            turn_identity: support::model_identity::identity(),
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
    }
}

#[test]
fn exact_expired_recovery_respects_heartbeat_and_reclaims_original_run_after_cold_open() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("expired-delivery.db");
    let route = serde_json::json!({"kind":"http", "protocol":"chat_completions", "output_token_field":"max_tokens",
        "provider":"fixture", "endpoint":"https://fixture.invalid/v1", "model":"fixture",
        "endpoint_query_sha256":null, "endpoint_query_source":{"kind":"none"},
        "credential_sha256":"a".repeat(64), "credential_source":{"kind":"ephemeral"}});
    let binding: nola_host_protocol::HostInvocationBinding = serde_json::from_value(serde_json::json!({
        "version":nola_host_protocol::HOST_INVOCATION_BINDING_VERSION, "primary":route.clone(), "fallbacks":[], "execution_policy_sha256":"b".repeat(64),
        "generation":{"temperature":0,"top_p":1,"max_output_tokens":1024,"max_retries":0,
            "parallel_tool_calls":false,"reasoning_selection":null,"sampling_override":false,
            "reasoning_tier":"balanced",
            "generation_options":{},"thinking_enabled":false,"prompt_cache":false,
            "prompt_cache_key_sha256":null,"prompt_cache_key_prefix_sha256":null,"prompt_cache_retention":null},
        "output_admission":{"version":nola_host_protocol::HOST_OUTPUT_ADMISSION_VERSION,"routes":[{"route":route,"max_input_tokens":null,
            "max_output_tokens":null,"max_total_tokens":null}],"preference":{"explicit":1024},
            "resolved":{"tokens":1024,"source":"explicit_preference"}}
    })).unwrap();
    binding.validate().unwrap();
    let mut input = admission_input();
    input.core_config.turn_identity.host_invocation = Some(binding.clone());
    input.core_config.context_window = serde_json::from_value(serde_json::json!({
        "context_window_tokens":8192,"reserved_output_tokens":1024}))
    .unwrap();
    let (run, original) = {
        let mut store = NolaStore::open(&path).unwrap();
        let begun = begin_assistant_turn(&mut store, input).unwrap();
        let claimed = claim_committed_host_requests(
            &mut store,
            ClaimCommittedHostRequests {
                run_id: begun.begin.run_id.clone(),
                worker_generation: "old-generation".into(),
                lease_owner: "old-owner".into(),
                claimed_at_ms: 1100,
                lease_expires_at_ms: 1500,
                limit: 1,
            },
        )
        .unwrap();
        (
            begun.begin.run_id,
            claimed
                .requests
                .into_iter()
                .next()
                .unwrap()
                .delivery
                .unwrap(),
        )
    };
    let mut runtime = support::open_runtime(&path).unwrap();
    let before = runtime
        .execute(ApplicationCommand::LoadExactRunInvocation {
            conversation_id: "conversation".into(),
            expected_run_id: run.clone(),
            assistant_message_id: "assistant".into(),
        })
        .unwrap();
    runtime
        .execute(ApplicationCommand::HeartbeatEffectDelivery {
            input: HeartbeatEffectDelivery {
                run_id: run.clone(),
                attempt_id: original.attempt_id.clone(),
                worker_generation: original.worker_generation.clone(),
                lease_owner: original.lease_owner.clone(),
                lease_token: original.lease_token.clone(),
                current_lease_expires_at_ms: 1500,
                heartbeat_at_ms: 1400,
                requested_lease_expires_at_ms: 2500,
            },
        })
        .unwrap();
    let command = |expected: &str, host: nola_host_protocol::HostInvocationBinding, at: i64| {
        ApplicationCommand::RecoverExpiredRunDeliveries {
            conversation_id: "conversation".into(),
            expected_run_id: expected.into(),
            host_invocation: host,
            interrupted_at: at.to_string(),
            interrupted_at_ms: at,
        }
    };
    assert!(!command(&run, binding.clone(), 1600).is_read_only_query());
    runtime
        .execute(command(&run, binding.clone(), 1600))
        .unwrap();
    let renewed = runtime
        .execute(ApplicationCommand::LoadRunDeliveryOwnership {
            run_id: run.clone(),
        })
        .unwrap();
    let leases: Vec<EffectDeliveryLease> = serde_json::from_value(renewed.clone()).unwrap();
    assert_eq!(leases.len(), 1);
    assert_eq!(leases[0].lease_token, original.lease_token);
    assert_eq!(leases[0].lease_expires_at_ms, 2500);
    assert!(runtime
        .execute(command("replacement-run", binding.clone(), 2600))
        .is_err());
    let mut unauthorized = binding.clone();
    unauthorized.execution_policy_sha256 = "c".repeat(64);
    assert!(runtime.execute(command(&run, unauthorized, 2600)).is_err());
    assert_eq!(
        runtime
            .execute(ApplicationCommand::LoadRunDeliveryOwnership {
                run_id: run.clone()
            })
            .unwrap(),
        renewed
    );
    drop(runtime);
    let mut runtime = support::open_runtime(&path).unwrap();
    runtime.execute(command(&run, binding, 2600)).unwrap();
    assert_eq!(
        runtime
            .execute(ApplicationCommand::LoadExactRunInvocation {
                conversation_id: "conversation".into(),
                expected_run_id: run.clone(),
                assistant_message_id: "assistant".into(),
            })
            .unwrap(),
        before,
        "Physical expiry reconciliation must not rewrite the frozen Run/checkpoint witness"
    );
    let claimed: CommittedHostRequests = serde_json::from_value(
        runtime
            .execute(ApplicationCommand::ClaimCommittedHostRequests {
                input: ClaimCommittedHostRequests {
                    run_id: run.clone(),
                    worker_generation: "new-generation".into(),
                    lease_owner: "new-owner".into(),
                    claimed_at_ms: 2700,
                    lease_expires_at_ms: 5000,
                    limit: 1,
                },
            })
            .unwrap(),
    )
    .unwrap();
    assert_eq!(claimed.requests.len(), 1);
    let successor = claimed.requests[0].delivery.as_ref().unwrap();
    assert_ne!(successor.attempt_id, original.attempt_id);
    assert_ne!(successor.lease_token, original.lease_token);
    assert_eq!(successor.worker_generation, "new-generation");
}

fn fixture(path: &std::path::Path) -> String {
    let mut store = NolaStore::open(path).unwrap();
    let begun = begin_assistant_turn(&mut store, admission_input()).unwrap();
    // Deliberately unknown (not claimed dead) physical ownership. Intent is
    // legal, but it must not release ownership or acknowledge cancellation.
    claim_committed_host_requests(
        &mut store,
        ClaimCommittedHostRequests {
            run_id: begun.begin.run_id.clone(),
            worker_generation: "unknown-generation".into(),
            lease_owner: "unknown-owner".into(),
            claimed_at_ms: 1100,
            lease_expires_at_ms: 100_000,
            limit: 1,
        },
    )
    .unwrap();
    begun.begin.run_id
}

fn command(run: &str) -> ApplicationCommand {
    ApplicationCommand::RequestRunCancellation {
        input: RequestRunCancellation {
            conversation_id: "conversation".into(),
            expected_run_id: run.into(),
            assistant_message_id: "assistant".into(),
            cancellation_id: "cancel-1".into(),
            reason: Some("User stopped recovery".into()),
            requested_at_ms: 1200,
        },
    }
}

fn received(command: &ApplicationCommand) -> ExecuteReceivedCommand {
    ExecuteReceivedCommand {
        client_id: "cancel-client".into(),
        request_id: "cancel-1".into(),
        command_type: "request_run_cancellation".into(),
        command_json: serde_json::to_string(command).unwrap(),
        expected_revision: None,
        received_at: "1200".into(),
    }
}

#[test]
fn unknown_owner_cancellation_is_persisted_without_terminal_and_receipt_replays() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("cancel.db");
    // A lease alone is now revocable before execution. This case requires an
    // actual accepted start, whose physical owner cannot be presumed stopped.
    let mut store = NolaStore::open(&path).unwrap();
    let begun = begin_assistant_turn(&mut store, admission_input()).unwrap();
    let run = begun.begin.run_id.clone();
    let target = claim_committed_host_requests(&mut store, ClaimCommittedHostRequests {
        run_id: run.clone(),
        worker_generation: "unknown-generation".into(),
        lease_owner: "unknown-owner".into(),
        claimed_at_ms: 1100,
        lease_expires_at_ms: 100_000,
        limit: 1,
    }).unwrap().requests.remove(0);
    drop(store);
    let mut runtime = support::open_runtime(&path).unwrap();
    runtime.execute(ApplicationCommand::ReduceAndAcceptHostResponse {
        input: ReduceAndAcceptHostResponse {
            conversation_id: "conversation".into(),
            run_id: run.clone(),
            invocation_config_digest: begun.invocation_config_digest,
            received_at_ms: 1101,
            response: HostResponseEnvelope {
                request_id: target.request_id.clone(),
                effect_id: target.effect_id.clone(),
                attempt_id: target.attempt_id.clone(),
                delegated_invocation_id: target.delegated_invocation_id.clone(),
                host_id: target.host_id.clone(),
                lease_id: target.lease_id.clone(),
                session_id: target.session_id.clone(),
                turn_id: target.turn_id.clone(),
                catalog_revision: target.catalog_revision,
                availability_epoch: target.availability_epoch,
                delivery: target.delivery.clone(),
                sequence: 1,
                payload: HostResponse::ExecutionStarted,
            },
        },
    }).unwrap();
    let cmd = command(&run);
    assert!(!cmd.is_read_only_query());
    let first = runtime
        .execute_received_command(received(&cmd), cmd)
        .unwrap();
    assert!(!first.replayed);
    let ApplicationCommandReceiptOutcome::Committed { result, .. } = &first.outcome else {
        panic!("{first:?}")
    };
    let receipt: RunCancellationReceipt = serde_json::from_value(result.clone()).unwrap();
    assert_eq!(
        receipt.disposition,
        RunCancellationDisposition::CancellationCommitted
    );
    assert_eq!(receipt.terminal_result, None);
    let db = rusqlite::Connection::open(&path).unwrap();
    let count: i64 = db
        .query_row("SELECT count(*) FROM run_events", [], |r| r.get(0))
        .unwrap();
    let second = runtime
        .execute_received_command(received(&command(&run)), command(&run))
        .unwrap();
    assert!(second.replayed);
    assert_eq!(first.outcome, second.outcome);
    assert_eq!(
        db.query_row("SELECT count(*) FROM run_events", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        count
    );
    let status: Option<i64> = db
        .query_row(
            "SELECT completed_at_ms FROM runs WHERE id=?1",
            [&run],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(status, None);
    assert_eq!(db.query_row(
        "SELECT count(*) FROM effect_delivery_attempts WHERE lease_owner='unknown-owner'
         AND state='execution_started' AND json_extract(record_json,'$.execution_started_at_ms')=1101",
        [], |r| r.get::<_, i64>(0),
    ).unwrap(), 1);
    let ownership: Vec<EffectDeliveryLease> = serde_json::from_value(runtime.execute(
        ApplicationCommand::LoadRunDeliveryOwnership { run_id: run.clone() },
    ).unwrap()).unwrap();
    assert_eq!(ownership.len(), 1);
    assert_eq!(ownership[0], *target.delivery.as_ref().unwrap());
    let store = NolaStore::open(&path).unwrap();
    assert!(load_committed_host_requests(&store, &run)
        .unwrap()
        .requests
        .iter()
        .any(|r| matches!(&r.payload, HostRequest::Cancel {
            target_effect_id, target_not_started: None,
        } if target_effect_id == &target.effect_id)));
    let repeat: RunCancellationReceipt =
        serde_json::from_value(runtime.execute(command(&run)).unwrap()).unwrap();
    assert_eq!(
        repeat.disposition,
        RunCancellationDisposition::AlreadyCancelling
    );
}

#[test]
fn cancellation_wrong_assistant_or_run_never_changes_checkpoint() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("identity.db");
    let run = fixture(&path);
    let mut runtime = support::open_runtime(&path).unwrap();
    let db = rusqlite::Connection::open(&path).unwrap();
    let before: String = db
        .query_row(
            "SELECT checkpoint_json FROM core_checkpoint_bodies",
            [],
            |r| r.get(0),
        )
        .unwrap();
    for wrong_run in [true, false] {
        let ApplicationCommand::RequestRunCancellation { mut input } = command(&run) else {
            unreachable!()
        };
        if wrong_run {
            input.expected_run_id = "different-run".into();
        } else {
            input.assistant_message_id = "different-assistant".into();
        }
        assert!(runtime
            .execute(ApplicationCommand::RequestRunCancellation { input })
            .is_err());
        assert_eq!(
            db.query_row(
                "SELECT checkpoint_json FROM core_checkpoint_bodies",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            before
        );
    }
}

#[test]
fn host_confirmed_cancellation_finishes_without_scheduling_lifecycle_or_user_work() {
    use nola_store_port::assistant_journal::AssistantJournalRepository;
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("cancel-terminal.db");
    let run = fixture(&path);
    let admitted = NolaStore::open(&path).unwrap();
    let identity = nola_domain::assistant_journal::AssistantJournalIdentity {
        source_store_epoch: admitted.assistant_source_epoch().unwrap(),
        conversation_id: "conversation".into(),
        run_id: run.clone(),
        assistant_message_id: "assistant".into(),
    };
    assert_eq!(
        admitted.assistant_journal_head(&identity).unwrap().sequence,
        0,
        "zero-token admission must not reserve a visible answer ahead of reasoning"
    );
    drop(admitted);
    let mut runtime = support::open_runtime(&path).unwrap();
    runtime
        .execute_received_command(received(&command(&run)), command(&run))
        .unwrap();
    let requests: CommittedHostRequests = serde_json::from_value(
        runtime
            .execute(ApplicationCommand::ClaimCommittedHostRequests {
                input: ClaimCommittedHostRequests {
                    run_id: run.clone(),
                    worker_generation: "isolated-cancel-host".into(),
                    lease_owner: "isolated-cancel-host".into(),
                    claimed_at_ms: 1300,
                    lease_expires_at_ms: 10_000,
                    limit: 64,
                },
            })
            .unwrap(),
    )
    .unwrap();
    assert_eq!(requests.requests.len(), 1);
    let request = &requests.requests[0];
    let HostRequest::Cancel { target_effect_id, .. } = &request.payload else {
        panic!(
            "cancellation may not dispatch hooks, models or tools: {:?}",
            request.payload
        )
    };
    let store = NolaStore::open(&path).unwrap();
    let digest =
        nola_store_port::run_effect::RunEffectRepository::load_run_invocation_config(&store, &run)
            .unwrap()
            .unwrap()
            .invocation_config_digest;
    for (sequence, payload) in [
        (1, HostResponse::ExecutionStarted),
        (
            2,
            HostResponse::CancellationConfirmed {
                target_effect_id: target_effect_id.clone(),
            },
        ),
    ] {
        let accepted: AcceptedHostResponse = serde_json::from_value(
            runtime
                .execute(ApplicationCommand::ReduceAndAcceptHostResponse {
                    input: ReduceAndAcceptHostResponse {
                        conversation_id: "conversation".into(),
                        run_id: run.clone(),
                        invocation_config_digest: digest.clone(),
                        received_at_ms: 1300 + sequence as i64,
                        response: HostResponseEnvelope {
                            request_id: request.request_id.clone(),
                            attempt_id: request.attempt_id.clone(),
                            delegated_invocation_id: request.delegated_invocation_id.clone(),
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
                    },
                })
                .unwrap(),
        )
        .unwrap();
        if sequence == 2 {
            assert_eq!(
                accepted.control.unwrap().turn_status,
                nola_domain::AgentTurnStatus::Cancelled
            );
            assert!(
                load_committed_host_requests(&store, &run)
                    .unwrap()
                    .requests
                    .is_empty(),
                "no lifecycle completion may be required after cancellation"
            );
        }
    }
    assert!(load_committed_host_requests(&store, &run)
        .unwrap()
        .requests
        .is_empty());
    assert!(
        store.assistant_journal_head(&identity).unwrap().sequence > 0,
        "zero-token cancellation must materialize an exact terminal frontier"
    );
    let seal = store
        .local_assistant_run_terminal_seal("conversation", &run)
        .unwrap()
        .expect("deferred text declaration must never weaken terminal seal coverage");
    assert!(
        matches!(
            seal.result.value,
            nola_domain::run_result::RunResultValue::Cancelled { .. }
        ),
        "a sealed lifecycle must retain the host-confirmed cancellation result"
    );
    let terminal = runtime
        .execute(ApplicationCommand::CanonicalRunTerminal {
            conversation_id: "conversation".into(),
            run_id: run,
        })
        .unwrap();
    assert_eq!(terminal["status"], "sealed");
}

#[test]
fn cancellation_receipt_failure_rolls_back_core_intent_and_outbox() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("rollback.db");
    let run = fixture(&path);
    let db = rusqlite::Connection::open(&path).unwrap();
    let before: String = db
        .query_row(
            "SELECT checkpoint_json FROM core_checkpoint_bodies",
            [],
            |r| r.get(0),
        )
        .unwrap();
    db.execute_batch("CREATE TRIGGER fail_cancel_receipt BEFORE UPDATE ON command_receipts WHEN NEW.status='committed' BEGIN SELECT RAISE(ABORT,'cancel receipt fault'); END;").unwrap();
    let mut runtime = support::open_runtime(&path).unwrap();
    assert!(runtime
        .execute_received_command(received(&command(&run)), command(&run))
        .unwrap_err()
        .message
        .contains("cancel receipt fault"));
    assert_eq!(
        db.query_row(
            "SELECT checkpoint_json FROM core_checkpoint_bodies",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        before
    );
    let store = NolaStore::open(&path).unwrap();
    assert!(!load_committed_host_requests(&store, &run)
        .unwrap()
        .requests
        .iter()
        .any(|r| matches!(r.payload, HostRequest::Cancel { .. })));
    db.execute_batch("DROP TRIGGER fail_cancel_receipt")
        .unwrap();
    let result = runtime
        .execute_received_command(received(&command(&run)), command(&run))
        .unwrap();
    assert!(matches!(
        result.outcome,
        ApplicationCommandReceiptOutcome::Committed { .. }
    ));
}

#[test]
fn cold_cancel_recovery_keeps_target_ownership_and_atomically_settles_its_expired_delivery() {
    use nola_store_port::assistant_journal::AssistantJournalRepository;
    for state in ["prepared", "leased", "started"] {
        let started = state == "started";
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("physical-cancellation.db");
        let mut store = NolaStore::open(&path).unwrap();
        let begun = begin_assistant_turn(&mut store, admission_input()).unwrap();
        let run = begun.begin.run_id.clone();
        let target = if state == "prepared" {
            let request = load_committed_host_requests(&store, &run)
                .unwrap()
                .requests
                .remove(0);
            let prepared = nola_kernel::prepare_effect_delivery_attempt(
                "prepared-target",
                &run,
                &request.effect_id.0,
                &request.request_id,
                1,
                nola_domain::EffectDeliveryReplaySafety::Recoverable,
                None,
                1100,
            )
            .unwrap();
            store
                .commit_effect_delivery_attempt_mutations(&[prepared])
                .unwrap();
            request
        } else {
            claim_committed_host_requests(
                &mut store,
                ClaimCommittedHostRequests {
                    run_id: run.clone(),
                    worker_generation: "original-generation".into(),
                    lease_owner: "original-owner".into(),
                    claimed_at_ms: 1100,
                    lease_expires_at_ms: 1500,
                    limit: 1,
                },
            )
            .unwrap()
            .requests
            .remove(0)
        };
        let target_attempt_id = target
            .delivery
            .as_ref()
            .map(|delivery| delivery.attempt_id.clone())
            .unwrap_or_else(|| "prepared-target".into());
        drop(store);
        let response = |request: &HostRequestEnvelope, sequence, received_at_ms, payload| {
            ApplicationCommand::ReduceAndAcceptHostResponse {
                input: ReduceAndAcceptHostResponse {
                    conversation_id: "conversation".into(),
                    run_id: run.clone(),
                    invocation_config_digest: begun.invocation_config_digest.clone(),
                    received_at_ms,
                    response: HostResponseEnvelope {
                        request_id: request.request_id.clone(),
                        effect_id: request.effect_id.clone(),
                        attempt_id: request.attempt_id.clone(),
                        delegated_invocation_id: request.delegated_invocation_id.clone(),
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
                },
            }
        };
        let mut runtime = support::open_runtime(&path).unwrap();
        if started {
            runtime
                .execute(response(&target, 1, 1101, HostResponse::ExecutionStarted))
                .unwrap();
        }
        runtime.execute(command(&run)).unwrap();
        drop(runtime);
        let store = NolaStore::open(&path).unwrap();
        let after_intent = store.effect_delivery_attempt(&target_attempt_id).unwrap().unwrap();
        assert_eq!(after_intent.execution_started_at_ms, started.then_some(1101));
        assert_eq!(after_intent.lease_owner.is_some(), state != "prepared");
        if started {
            assert_eq!(after_intent.state, nola_domain::EffectDeliveryAttemptState::ExecutionStarted);
            assert_eq!(after_intent.settlement, None);
        } else {
            assert_eq!(after_intent.state, nola_domain::EffectDeliveryAttemptState::Settled);
            assert_eq!(after_intent.settlement,
                Some(nola_domain::EffectDeliverySettlement::RejectedBeforeExecution));
            assert_eq!(after_intent.finished_at_ms, Some(1200),
                "never-started work is revoked with the cancellation admission, not later confirmation");
        }
        drop(store);
        let mut runtime = support::open_runtime(&path).unwrap();
        let ownership = runtime
            .execute(ApplicationCommand::LoadRunDeliveryOwnership {
                run_id: run.clone(),
            })
            .unwrap();
        let ownership: Vec<EffectDeliveryLease> = serde_json::from_value(ownership).unwrap();
        assert_eq!(
            ownership.len(),
            usize::from(started),
            "only actually started work retains physical ownership after atomic no-start revocation"
        );
        if let Some(owned) = ownership.first() {
            assert_eq!(owned.attempt_id, target_attempt_id);
        }
        runtime
            .execute(ApplicationCommand::RecoverOrphanedRuns {
                conversation_id: Some("conversation".into()),
                reason: "cold_cancel_recovery".into(),
                interrupted_at: "2000".into(),
                interrupted_at_ms: 2000,
            })
            .unwrap();
        let control: CommittedHostRequests = serde_json::from_value(
            runtime
                .execute(ApplicationCommand::ClaimCommittedHostRequests {
                    input: ClaimCommittedHostRequests {
                        run_id: run.clone(),
                        worker_generation: "control-generation".into(),
                        lease_owner: "control-owner".into(),
                        claimed_at_ms: 2100,
                        lease_expires_at_ms: 3000,
                        limit: 64,
                    },
                })
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            control.requests.len(),
            1,
            "recovery may dispatch only the original cancellation, never the target again"
        );
        let control = &control.requests[0];
        let HostRequest::Cancel { target_effect_id, target_not_started } = &control.payload else {
            panic!("cold recovery must retain the original Cancel control")
        };
        assert_eq!(target_effect_id, &target.effect_id);
        if started {
            assert!(target_not_started.is_none(), "a real start never earns a no-start proof");
        } else {
            let proof = target_not_started.as_ref().expect("atomically revoked target has its exact proof");
            proof.validate().unwrap();
            assert!(proof.matches_control(&control.host_id.0, &control.session_id,
                control.turn_id.as_deref(), &control.effect_id.0, &control.request_id, &target.effect_id.0));
            assert_eq!(proof.target_request_id, target.request_id);
            assert_eq!(proof.revoked_attempts.len(), 1);
            assert_eq!(proof.revoked_attempts[0].attempt_id, target_attempt_id);
            assert_eq!(proof.revoked_attempts[0].revision, after_intent.revision);
        }
        runtime
            .execute(response(control, 1, 2101, HostResponse::ExecutionStarted))
            .unwrap();
        let db = rusqlite::Connection::open(&path).unwrap();
        let before = || {
            db.query_row("SELECT group_concat(record_json,'|') FROM (SELECT record_json FROM effect_delivery_attempts ORDER BY id)",
            [], |row| row.get::<_,String>(0)).unwrap()
        };
        let unchanged = before();
        assert!(runtime
            .execute(response(
                control,
                2,
                2102,
                HostResponse::CancellationConfirmed {
                    target_effect_id: HostEffectId("foreign-target".into()),
                }
            ))
            .is_err());
        assert_eq!(
            before(),
            unchanged,
            "wrong target cannot settle either physical ledger entry"
        );
        let confirm = || {
            response(
                control,
                2,
                2102,
                HostResponse::CancellationConfirmed {
                    target_effect_id: target.effect_id.clone(),
                },
            )
        };
        db.execute_batch("CREATE TRIGGER fail_cancel_settlement BEFORE UPDATE OF state ON effect_delivery_attempts WHEN NEW.state='settled' BEGIN SELECT RAISE(ABORT,'cancel settlement fault'); END;").unwrap();
        assert!(runtime.execute(confirm()).is_err());
        assert_eq!(
            before(),
            unchanged,
            "target/control/checkpoint/Run seal must roll back together"
        );
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM assistant_run_terminal_seals",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        db.execute_batch("DROP TRIGGER fail_cancel_settlement")
            .unwrap();
        runtime.execute(confirm()).unwrap();
        drop(runtime);
        let store = NolaStore::open(&path).unwrap();
        let settled = store
            .effect_delivery_attempt(&target_attempt_id)
            .unwrap()
            .unwrap();
        assert_eq!(
            settled.state,
            nola_domain::EffectDeliveryAttemptState::Settled
        );
        assert_eq!(settled.execution_started_at_ms.is_some(), started);
        assert_eq!(
            settled.lease_owner.is_some(),
            state != "prepared",
            "cancelling a prepared target must not fabricate physical ownership"
        );
        assert_eq!(
            settled.settlement,
            Some(if started {
                nola_domain::EffectDeliverySettlement::Cancelled
            } else {
                nola_domain::EffectDeliverySettlement::RejectedBeforeExecution
            })
        );
        for request in [&target, control] {
            assert!(store
                .active_effect_delivery_attempt(&request.effect_id.0)
                .unwrap()
                .is_none());
        }
        let seal = store
            .local_assistant_run_terminal_seal("conversation", &run)
            .unwrap()
            .unwrap();
        assert!(matches!(
            seal.result.value,
            nola_domain::run_result::RunResultValue::Cancelled { .. }
        ));
        assert!(load_committed_host_requests(&store, &run)
            .unwrap()
            .requests
            .is_empty());
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM effect_delivery_attempts", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
            2
        );
    }
}

#[test]
fn other_runtime_cancellation_fences_a_live_hosts_old_hot_checkpoint() {
    two_runtime_fixture(false, false);
}

#[test]
fn committed_terminal_wins_a_later_cancellation_request() {
    two_runtime_fixture(true, false);
}

#[test]
fn hot_control_queries_avoid_body_reads_and_cancellation_materializes_baseline() {
    two_runtime_fixture(false, true);
}

#[test]
fn cold_task_projection_is_bound_to_committed_checkpoint_without_transcript_fields() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("cold-task-projection.db");
    let mut runtime = support::open_runtime(&path).unwrap();
    let begun: AcceptedAssistantTurn = serde_json::from_value(
        runtime
            .execute(ApplicationCommand::BeginAssistantTurn {
                input: admission_input(),
            })
            .unwrap(),
    )
    .unwrap();
    drop(runtime);
    let mut cold = support::open_runtime(&path).unwrap();
    let result = cold
        .execute(ApplicationCommand::LoadConversationTaskProjectionV1 {
            conversation_id: "conversation".into(),
        })
        .unwrap();
    let observer = NolaStore::open(&path).unwrap();
    let checkpoint = observer.load_checkpoint("conversation").unwrap().unwrap();
    assert_eq!(result["checkpoint_revision"], checkpoint.revision);
    assert_eq!(result["checkpoint_event_cursor"], checkpoint.event_cursor);
    assert_eq!(
        result["core_session_id"],
        serde_json::to_value(&begun.execution.control.session_id).unwrap()
    );
    assert_eq!(result.as_object().unwrap().len(), 4);
    assert!(result["task_projection"]["tasks"].is_object());
    assert!(ApplicationCommand::LoadConversationTaskProjectionV1 {
        conversation_id: "conversation".into(),
    }
    .is_read_only_query());
    assert!(cold
        .execute(ApplicationCommand::LoadConversationTaskProjectionV1 {
            conversation_id: "missing".into(),
        })
        .is_err());
}

#[test]
fn admission_retains_core_for_first_host_response_without_checkpoint_body_read() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("hot-admission.db");
    let mut runtime = support::open_runtime(&path).unwrap();
    let begun: AcceptedAssistantTurn = serde_json::from_value(
        runtime
            .execute(ApplicationCommand::BeginAssistantTurn {
                input: admission_input(),
            })
            .unwrap(),
    )
    .unwrap();
    let guard = rusqlite::Connection::open(&path).unwrap();
    let initial_base: (String, i64, i64) = guard.query_row(
        "SELECT run_id,through_ordinal,through_reduction_bytes FROM core_checkpoint_replay_bases WHERE conversation_id='conversation'",
        [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).unwrap();
    assert_eq!(initial_base, (begun.begin.run_id.clone(), 0, 0));
    guard
        .execute_batch(
            "ALTER TABLE core_checkpoint_bodies RENAME TO guarded_checkpoint_bodies;
        CREATE VIEW core_checkpoint_bodies AS SELECT conversation_id,
            json_extract('forbidden-checkpoint-body-read','$') AS checkpoint_json
            FROM guarded_checkpoint_bodies;",
        )
        .unwrap();
    let observer = NolaStore::open(&path).unwrap();
    assert!(observer.load_checkpoint("conversation").is_err());
    let execution = runtime
        .execute(ApplicationCommand::LoadHostExecutionState {
            conversation_id: "conversation".into(),
        })
        .unwrap();
    assert_eq!(execution, serde_json::to_value(&begun.execution).unwrap());
    let claimed: CommittedHostRequests = serde_json::from_value(
        runtime
            .execute(ApplicationCommand::ClaimCommittedHostRequests {
                input: ClaimCommittedHostRequests {
                    run_id: begun.begin.run_id.clone(),
                    worker_generation: "host".into(),
                    lease_owner: "host".into(),
                    claimed_at_ms: 1100,
                    lease_expires_at_ms: 10_000,
                    limit: 1,
                },
            })
            .unwrap(),
    )
    .unwrap();
    let request = claimed.requests.into_iter().next().unwrap();
    let accepted: AcceptedHostResponse = serde_json::from_value(
        runtime
            .execute(ApplicationCommand::ReduceAndAcceptHostResponse {
                input: ReduceAndAcceptHostResponse {
                    conversation_id: "conversation".into(),
                    run_id: begun.begin.run_id.clone(),
                    invocation_config_digest: begun.invocation_config_digest,
                    received_at_ms: 1200,
                    response: HostResponseEnvelope {
                        request_id: request.request_id,
                        attempt_id: request.attempt_id,
                        delegated_invocation_id: request.delegated_invocation_id,
                        effect_id: request.effect_id,
                        host_id: request.host_id,
                        lease_id: request.lease_id,
                        session_id: request.session_id,
                        turn_id: request.turn_id,
                        catalog_revision: request.catalog_revision,
                        availability_epoch: request.availability_epoch,
                        delivery: request.delivery,
                        sequence: 1,
                        payload: HostResponse::ExecutionStarted,
                    },
                },
            })
            .unwrap(),
    )
    .unwrap();
    assert!(accepted.commit.applied);
    assert!(
        accepted.control.is_none(),
        "incremental replies do not republish control"
    );
    let hot = runtime
        .execute(ApplicationCommand::LoadHostExecutionState {
            conversation_id: "conversation".into(),
        })
        .unwrap();
    guard
        .execute_batch(
            "DROP VIEW core_checkpoint_bodies;
        ALTER TABLE guarded_checkpoint_bodies RENAME TO core_checkpoint_bodies;",
        )
        .unwrap();
    drop(runtime);
    let mut reopened = support::open_runtime(&path).unwrap();
    let cold = reopened
        .execute(ApplicationCommand::LoadHostExecutionState {
            conversation_id: "conversation".into(),
        })
        .unwrap();
    assert_eq!(
        cold, hot,
        "cold replay must reconstruct the same selective execution state"
    );
}

#[test]
fn failed_admission_receipt_discards_the_retained_core() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("failed-hot-admission.db");
    let mut runtime = support::open_runtime(&path).unwrap();
    let guard = rusqlite::Connection::open(&path).unwrap();
    guard
        .execute_batch(
            "CREATE TRIGGER fail_admission_receipt BEFORE UPDATE ON command_receipts
        WHEN NEW.status='committed' BEGIN SELECT RAISE(ABORT,'admission receipt fault'); END;",
        )
        .unwrap();
    let command = ApplicationCommand::BeginAssistantTurn {
        input: admission_input(),
    };
    let receipt = ExecuteReceivedCommand {
        client_id: "admission-host".into(),
        request_id: "admission-1".into(),
        command_type: "begin_assistant_turn".into(),
        command_json: serde_json::to_string(&command).unwrap(),
        expected_revision: None,
        received_at: "1000".into(),
    };
    assert!(runtime
        .execute_received_command(receipt, command)
        .unwrap_err()
        .message
        .contains("admission receipt fault"));
    let observer = NolaStore::open(&path).unwrap();
    assert!(observer.active_run("conversation").unwrap().is_none());
    assert!(observer.load_checkpoint("conversation").unwrap().is_none());
    assert!(
        runtime
            .execute(ApplicationCommand::LoadHostExecutionState {
                conversation_id: "conversation".into(),
            })
            .is_err(),
        "rolled-back admission must not expose an execution overlay"
    );
    guard
        .execute_batch("DROP TRIGGER fail_admission_receipt")
        .unwrap();
    let accepted = runtime
        .execute(ApplicationCommand::BeginAssistantTurn {
            input: admission_input(),
        })
        .unwrap();
    assert_eq!(accepted["begin"]["run_id"], "run");
}

fn two_runtime_fixture(terminal_first: bool, forbid_checkpoint_body: bool) {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("two-hosts.db");
    let run = fixture(&path);
    let store = NolaStore::open(&path).unwrap();
    let mut request = load_committed_host_requests(&store, &run)
        .unwrap()
        .requests
        .into_iter()
        .next()
        .unwrap();
    let mut live_host = support::open_runtime(&path).unwrap();
    let leases: Vec<EffectDeliveryLease> = serde_json::from_value(
        live_host
            .execute(ApplicationCommand::LoadRunDeliveryOwnership {
                run_id: run.clone(),
            })
            .unwrap(),
    )
    .unwrap();
    request.delivery = Some(leases.into_iter().next().unwrap());
    let digest =
        nola_store_port::run_effect::RunEffectRepository::load_run_invocation_config(&store, &run)
            .unwrap()
            .unwrap()
            .invocation_config_digest;
    let response = |sequence, payload| ApplicationCommand::ReduceAndAcceptHostResponse {
        input: ReduceAndAcceptHostResponse {
            conversation_id: "conversation".into(),
            run_id: run.clone(),
            invocation_config_digest: digest.clone(),
            received_at_ms: 1100 + sequence as i64,
            response: HostResponseEnvelope {
                request_id: request.request_id.clone(),
                attempt_id: request.attempt_id.clone(),
                delegated_invocation_id: request.delegated_invocation_id.clone(),
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
        },
    };
    live_host
        .execute(response(1, HostResponse::ExecutionStarted))
        .unwrap();
    let mut control = support::open_runtime(&path).unwrap();
    if terminal_first {
        live_host
            .execute(response(
                2,
                HostResponse::Failed {
                    error: HostError {
                        code: "provider_failure".into(),
                        message: "Fixture provider finished with an error".into(),
                        retryable: false,
                        http_status: Some(400),
                    },
                },
            ))
            .unwrap();
        let before = nola_store_port::run_effect::RunEffectRepository::load_checkpoint(
            &store,
            "conversation",
        )
        .unwrap()
        .unwrap()
        .checkpoint_json;
        let receipt = control
            .execute_received_command(received(&command(&run)), command(&run))
            .unwrap();
        let ApplicationCommandReceiptOutcome::Committed { result, .. } = receipt.outcome else {
            panic!("terminal cancellation must acknowledge existing evidence")
        };
        let result: RunCancellationReceipt = serde_json::from_value(result).unwrap();
        assert_eq!(
            result.disposition,
            RunCancellationDisposition::AlreadyTerminal
        );
        assert!(matches!(
            result.terminal_result.map(|result| result.value),
            Some(nola_domain::run_result::RunResultValue::Failed { .. })
        ));
        assert_eq!(
            nola_store_port::run_effect::RunEffectRepository::load_checkpoint(
                &store,
                "conversation"
            )
            .unwrap()
            .unwrap()
            .checkpoint_json,
            before
        );
        let cold_execution = control
            .execute(ApplicationCommand::LoadHostExecutionState {
                conversation_id: "conversation".into(),
            })
            .unwrap();
        let guard = rusqlite::Connection::open(&path).unwrap();
        guard
            .execute_batch(
                "ALTER TABLE core_checkpoint_bodies RENAME TO guarded_checkpoint_bodies;
            CREATE VIEW core_checkpoint_bodies AS SELECT conversation_id,
                json_extract('forbidden-checkpoint-body-read','$') AS checkpoint_json
                FROM guarded_checkpoint_bodies;",
            )
            .unwrap();
        assert!(store.load_checkpoint("conversation").is_err());
        let terminal_execution = live_host
            .execute(ApplicationCommand::LoadHostExecutionState {
                conversation_id: "conversation".into(),
            })
            .unwrap();
        assert_eq!(
            terminal_execution, cold_execution,
            "terminal adoption must use the committed selective view without reading the body"
        );
        let terminal_permissions = live_host
            .execute(ApplicationCommand::LoadConversationPermissionProjectionV1 {
                conversation_id: "conversation".into(),
                run_id: run.clone(),
                scope: PermissionProjectionReadScope::CurrentRun,
            })
            .unwrap();
        assert_eq!(terminal_permissions["run_id"], run);
        guard
            .execute_batch(
                "DROP VIEW core_checkpoint_bodies;
            ALTER TABLE guarded_checkpoint_bodies RENAME TO core_checkpoint_bodies;",
            )
            .unwrap();
        return;
    }
    if forbid_checkpoint_body {
        let cold_permissions = control
            .execute(ApplicationCommand::LoadConversationPermissionProjectionV1 {
                conversation_id: "conversation".into(),
                run_id: run.clone(),
                scope: PermissionProjectionReadScope::CurrentRun,
            })
            .unwrap();
        let cold_execution = control
            .execute(ApplicationCommand::LoadHostExecutionState {
                conversation_id: "conversation".into(),
            })
            .unwrap();
        let baseline = store
            .load_checkpoint("conversation")
            .unwrap()
            .unwrap()
            .checkpoint_json;
        let guard = rusqlite::Connection::open(&path).unwrap();
        guard
            .execute_batch(
                "ALTER TABLE core_checkpoint_bodies RENAME TO guarded_checkpoint_bodies;
            CREATE VIEW core_checkpoint_bodies AS SELECT conversation_id,
                json_extract('forbidden-checkpoint-body-read','$') AS checkpoint_json
                FROM guarded_checkpoint_bodies;",
            )
            .unwrap();
        assert!(
            store.load_checkpoint("conversation").is_err(),
            "fixture must reject full-body reads"
        );
        let execution = live_host
            .execute(ApplicationCommand::LoadHostExecutionState {
                conversation_id: "conversation".into(),
            })
            .unwrap();
        assert_eq!(execution, cold_execution);
        for forbidden in [
            "checkpoint",
            "snapshot",
            "projection",
            "transcript",
            "model_context",
        ] {
            assert!(
                execution.get(forbidden).is_none(),
                "execution view must not carry {forbidden}"
            );
        }
        let permission_query = || ApplicationCommand::LoadConversationPermissionProjectionV1 {
            conversation_id: "conversation".into(),
            run_id: run.clone(),
            scope: PermissionProjectionReadScope::CurrentRun,
        };
        let permissions = live_host.execute(permission_query()).unwrap();
        assert_eq!(
            permissions, cold_permissions,
            "retained and cold-recovered permission frontiers must agree"
        );
        assert_eq!(permissions["run_id"], run);
        assert_eq!(permissions["permissions"]["pending"], serde_json::json!([]));
        // Same wire command and validation, now bound to the control witness.
        let controlled = live_host
            .execute(ApplicationCommand::LoadConversationPermissionProjectionV1 {
                conversation_id: "conversation".into(),
                run_id: run.clone(),
                scope: PermissionProjectionReadScope::Control {
                    session_id: permissions["session_id"].as_str().unwrap().into(),
                    active_turn_id: permissions["active_turn_id"].as_str().map(str::to_owned),
                    event_cursor: permissions["event_cursor"].as_u64().unwrap(),
                    permission_projection_revision: permissions["permission_projection_revision"]
                        .as_u64()
                        .unwrap(),
                },
            })
            .unwrap();
        assert_eq!(controlled, permissions);
        assert!(live_host
            .execute(ApplicationCommand::LoadConversationPermissionProjectionV1 {
                conversation_id: "conversation".into(),
                run_id: run.clone(),
                scope: PermissionProjectionReadScope::Control {
                    session_id: permissions["session_id"].as_str().unwrap().into(),
                    active_turn_id: permissions["active_turn_id"].as_str().map(str::to_owned),
                    event_cursor: permissions["event_cursor"].as_u64().unwrap(),
                    permission_projection_revision: permissions["permission_projection_revision"]
                        .as_u64()
                        .unwrap()
                        + 1,
                },
            })
            .unwrap_err()
            .contains("stale or foreign"));
        // Queries above must remain selective. Interrupt is now an intentional
        // durable baseline boundary, so restore its writable body table.
        guard.execute_batch("DROP VIEW core_checkpoint_bodies;
            ALTER TABLE guarded_checkpoint_bodies RENAME TO core_checkpoint_bodies;").unwrap();
        let receipt = live_host
            .execute_received_command(received(&command(&run)), command(&run))
            .unwrap();
        assert!(
            matches!(
                receipt.outcome,
                ApplicationCommandReceiptOutcome::Committed { .. }
            ),
            "hot cancellation must commit its durable baseline"
        );
        assert_ne!(
            store
                .load_checkpoint("conversation")
                .unwrap()
                .unwrap()
                .checkpoint_json,
            baseline
        );
    } else {
        control
            .execute_received_command(received(&command(&run)), command(&run))
            .unwrap();
    }
    // The live host still owns its pre-cancellation in-memory Core. Its next
    // response must reload the new durable revision, not resurrect that Core.
    let late: AcceptedHostResponse = serde_json::from_value(
        live_host
            .execute(response(2, HostResponse::Accepted))
            .unwrap(),
    )
    .unwrap();
    assert!(!late.commit.applied);
    assert!(late.events.is_empty());
    assert!(load_committed_host_requests(&store, &run)
        .unwrap()
        .requests
        .iter()
        .all(|r| matches!(r.payload, HostRequest::Cancel { .. })));
    let repeat: AcceptedHostResponse = serde_json::from_value(
        live_host
            .execute(response(3, HostResponse::Accepted))
            .unwrap(),
    )
    .unwrap();
    assert!(!repeat.commit.applied);
    assert!(repeat.events.is_empty());
    let attempt_id = request.attempt_id.as_deref().expect("model target has an Attempt");
    let cancelled_attempt = store.model_attempt(attempt_id).unwrap().unwrap();
    assert_eq!(cancelled_attempt.state, nola_domain::ModelAttemptState::Cancelled);
    for (sequence, payload) in [
        (4, HostResponse::Failed { error: HostError {
            code: "cancelled".into(), message: "local stream returned".into(),
            retryable: false, http_status: None,
        }}),
        (5, HostResponse::ModelDelta { channel: ModelDeltaChannel::Text,
            part_id: cancelled_attempt.text_part_id.clone(), delta: "MUST_NOT_REENTER_CONTENT".into() }),
    ] {
        // Cold and hot consumers must both acknowledge only the retired
        // transport response, without rewriting Cancelled as Failed or adding
        // late text. The separate Cancel still owns canonical termination.
        let mut cold = support::open_runtime(&path).unwrap();
        let accepted: AcceptedHostResponse = serde_json::from_value(
            cold.execute(response(sequence, payload)).unwrap()).unwrap();
        assert!(!accepted.commit.applied);
        assert!(accepted.events.is_empty());
        assert_eq!(store.model_attempt(attempt_id).unwrap().unwrap(), cancelled_attempt);
        assert!(store.active_run("conversation").unwrap().is_some());
    }
    for wrong_part in [false, true] {
        let mut invalid = response(6, HostResponse::ModelDelta { channel: ModelDeltaChannel::Text,
            part_id: if wrong_part { "foreign-part".into() } else { cancelled_attempt.text_part_id.clone() },
            delta: "FOREIGN_CONTENT".into() });
        if !wrong_part {
            let ApplicationCommand::ReduceAndAcceptHostResponse { input } = &mut invalid else { unreachable!() };
            input.response.attempt_id = Some("foreign-attempt".into());
        }
        let mut cold = support::open_runtime(&path).unwrap();
        assert!(cold.execute(invalid).is_err());
        assert_eq!(store.model_attempt(attempt_id).unwrap().unwrap(), cancelled_attempt);
    }
    let checkpoint = support::current_checkpoint(&path, "conversation");
    let checkpoint: nola_agent_engine::agent::recovery::CoreCheckpoint<AgentThreadState> =
        serde_json::from_str(&checkpoint.checkpoint_json).unwrap();
    assert_eq!(
        checkpoint.state.execution.active_turn.unwrap().status,
        nola_domain::AgentTurnStatus::Cancelling
    );
    assert!(load_committed_host_requests(&store, &run)
        .unwrap()
        .requests
        .iter()
        .any(|r| matches!(r.payload, HostRequest::Cancel { .. })));
    assert_eq!(
        nola_store_port::run_effect::RunEffectRepository::active_run(&store, "conversation")
            .unwrap()
            .unwrap()
            .run_id,
        run
    );
}

#[test]
fn typed_process_observations_validate_cold_cancellation_without_fabricating_owner_death() {
    use nola_application::cancellation_delivery::CancellationRunIdentity;
    use nola_application::cancellation_process_observation::*;
    use nola_host_protocol::{HostWorkerProcessIdentity, HostProcessObservation};
    let temp=tempfile::tempdir().unwrap();let path=temp.path().join("typed-process-cancel.db");
    let mut store=NolaStore::open(&path).unwrap();
    let begun=begin_assistant_turn(&mut store,admission_input()).unwrap();let run=begun.begin.run_id.clone();
    let old=HostWorkerProcessIdentity{pid:100,boot_id:"boot".into(),start_id:"1000".into(),nonce:"old".into()};
    let old_lease=old.lease_identity().unwrap();
    let target=claim_committed_host_requests(&mut store,ClaimCommittedHostRequests{run_id:run.clone(),
        worker_generation:old_lease.worker_generation,lease_owner:old_lease.lease_owner,
        claimed_at_ms:1100,lease_expires_at_ms:1500,limit:1}).unwrap().requests.remove(0);
    let response=|request:&HostRequestEnvelope,sequence,received_at_ms,payload| ApplicationCommand::ReduceAndAcceptHostResponse {
        input:ReduceAndAcceptHostResponse{conversation_id:"conversation".into(),run_id:run.clone(),
            invocation_config_digest:begun.invocation_config_digest.clone(),received_at_ms,
            response:HostResponseEnvelope{request_id:request.request_id.clone(),effect_id:request.effect_id.clone(),
                attempt_id:request.attempt_id.clone(),delegated_invocation_id:request.delegated_invocation_id.clone(),
                host_id:request.host_id.clone(),lease_id:request.lease_id.clone(),session_id:request.session_id.clone(),
                turn_id:request.turn_id.clone(),catalog_revision:request.catalog_revision,availability_epoch:request.availability_epoch,
                delivery:request.delivery.clone(),sequence,payload}}};
    drop(store);
    let mut runtime=support::open_runtime(&path).unwrap();
    runtime.execute(response(&target,1,1101,HostResponse::ExecutionStarted)).unwrap();
    runtime.execute(command(&run)).unwrap();drop(runtime);
    let mut store=NolaStore::open(&path).unwrap();
    let current=HostWorkerProcessIdentity{pid:101,boot_id:"boot".into(),start_id:"2000".into(),nonce:"new".into()};
    let lease=current.lease_identity().unwrap();
    let control=claim_committed_host_requests(&mut store,ClaimCommittedHostRequests{run_id:run.clone(),
        worker_generation:lease.worker_generation,lease_owner:lease.lease_owner,claimed_at_ms:2100,
        lease_expires_at_ms:5000,limit:1}).unwrap().requests.remove(0);
    let plan=inspect_cancellation_process_owners(&store,CancellationRunIdentity{conversation_id:"conversation".into(),
        expected_run_id:run.clone(),assistant_message_id:"assistant".into()},control.clone()).unwrap();
    assert_eq!(plan.target_owners,vec![old.clone()]);
    let observer=HostProcessObservation::Running{pid:101,boot_id:"boot".into(),start_id:"2000".into()};
    for observation in [HostProcessObservation::Unknown{pid:100},
        HostProcessObservation::Running{pid:100,boot_id:"boot".into(),start_id:"1000".into()},
        HostProcessObservation::Gone{pid:999,boot_id:"boot".into()}] {
        assert!(validate_cancellation_process_observations(&store,plan.clone(),observer.clone(),
            vec![CancellationProcessObservation{owner:old.clone(),observation}]).is_err());
    }
    let proven=validate_cancellation_process_observations(&store,plan.clone(),observer.clone(),
        vec![CancellationProcessObservation{owner:old,observation:HostProcessObservation::Gone{pid:100,boot_id:"boot".into()}}]).unwrap();
    assert_eq!(proven.effect_id,target.effect_id);
    drop(store);
    let mut runtime=support::open_runtime(&path).unwrap();
    runtime.execute(response(&control,1,2200,HostResponse::ExecutionStarted)).unwrap();
    runtime.execute(response(&control,2,2201,HostResponse::CancellationConfirmed{target_effect_id:target.effect_id})).unwrap();
    drop(runtime);
    let store=NolaStore::open(&path).unwrap();
    assert!(inspect_cancellation_process_owners(&store,plan.observed.identity,control).is_err());
}
