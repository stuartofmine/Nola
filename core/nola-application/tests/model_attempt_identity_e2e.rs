// SPDX-License-Identifier: Apache-2.0
//! Real Application admission rejects stale Attempt output across cold recovery.
mod support;
use nola_application::{
    begin_assistant_turn, claim_committed_host_requests, load_committed_host_requests,
    reduce_and_accept_host_response, ApplicationCommand, BeginAssistantTurn,
    ClaimCommittedHostRequests, ReduceAndAcceptHostResponse,
};
use nola_agent_engine::agent::host::*;
use nola_agent_engine::agent::protocol::*;
use nola_agent_engine::agent::recovery::CoreCheckpoint;
use nola_agent_engine::agent::{AgentCore, AgentThreadState};
use nola_store::NolaStore;
use nola_store_port::run_effect::RunEffectRepository;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

fn runtime_capability(id: &str, route: HostCapabilityRoute) -> HostCapabilityDescriptor {
    HostCapabilityDescriptor {
        id: id.to_string(),
        title: id.to_string(),
        route,
        availability: CapabilityAvailability::Available,
        exposure: HostCapabilityExposure::Internal,
        permission_domain: None,
        input_schema: serde_json::json!({"type": "object"}),
        input_normalization: Default::default(),
        output_policy: HostOutputPolicy {
            preview_char_limit: 4_000,
            persist_full_output: true,
            allow_artifacts: true,
        },
        supports_streaming: id == "model.stream",
        supports_argument_streaming: false,
        parallelism: HostCapabilityParallelism::Serial,
        cancellation: HostCancellationMode::Cooperative,
        replay_safety: HostReplaySafety::Recoverable,
    }
}

fn runtime_config() -> AgentCoreConfig {
    let mut capabilities = BTreeMap::new();
    capabilities.insert(
        "model.stream".to_string(),
        runtime_capability("model.stream", HostCapabilityRoute::Model),
    );
    support::goal_execution::with_execution_binding(AgentCoreConfig {
        turn_identity: support::model_identity::identity(),
        session_id: Some(SessionId::from_string(
            "completion-conversation".to_string(),
        )),
        model: Some("test-model".to_string()),
        host_catalog: Some(HostCapabilityCatalog {
            host_id: HostId("recovery-host".to_string()),
            revision: 1,
            availability_epoch: 1,
            lease: None,
            capabilities,
        }),
        ..AgentCoreConfig::default()
    })
}

fn response_for_request(
    request: &HostRequestEnvelope,
    sequence: u64,
    payload: HostResponse,
) -> HostResponseEnvelope {
    HostResponseEnvelope {
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
    }
}

fn response_command(
    request: &HostRequestEnvelope,
    run: &str,
    digest: &str,
    payload: HostResponse,
) -> ApplicationCommand {
    ApplicationCommand::ReduceAndAcceptHostResponse {
        input: ReduceAndAcceptHostResponse {
            conversation_id: "completion-conversation".into(),
            run_id: run.into(),
            invocation_config_digest: digest.into(),
            response: response_for_request(
                request,
                request.resume_after_sequence.unwrap_or(0) + 1,
                payload,
            ),
            received_at_ms: 2000,
        },
    }
}

fn claim_request(database: &std::path::Path, run: &str, now_ms: i64) -> HostRequestEnvelope {
    let mut store = NolaStore::open(database).unwrap();
    let claimed = claim_committed_host_requests(
        &mut store,
        ClaimCommittedHostRequests {
            run_id: run.into(),
            worker_generation: format!("generation-{now_ms}"),
            lease_owner: format!("runtime-{now_ms}"),
            claimed_at_ms: now_ms,
            lease_expires_at_ms: now_ms + 60_000,
            limit: 1,
        },
    )
    .unwrap();
    claimed.requests.into_iter().next().unwrap()
}

fn claim_and_start(
    database: &std::path::Path,
    run: &str,
    digest: &str,
    now_ms: i64,
) -> HostRequestEnvelope {
    let mut request = claim_request(database, run, now_ms);
    let mut store = NolaStore::open(database).unwrap();
    reduce_and_accept_host_response(
        &mut store,
        ReduceAndAcceptHostResponse {
            conversation_id: "completion-conversation".into(),
            run_id: run.into(),
            invocation_config_digest: digest.into(),
            response: response_for_request(&request, 1, HostResponse::ExecutionStarted),
            received_at_ms: now_ms + 1,
        },
    )
    .unwrap();
    request.resume_after_sequence = Some(1);
    request
}

fn pending_with_delivery(
    database: &std::path::Path,
    run: &str,
    delivery: Option<EffectDeliveryLease>,
) -> HostRequestEnvelope {
    let mut request = pending(database, run).remove(0);
    request.delivery = delivery;
    request
}

fn canonical_rows(db: &rusqlite::Connection) -> Vec<Vec<String>> {
    ["SELECT checkpoint_json FROM core_checkpoint_bodies ORDER BY conversation_id",
     "SELECT id,role,status,coalesce(run_id,''),ordinal FROM messages ORDER BY id",
     "SELECT id,kind,state,coalesce(content_text,''),coalesce(content_json,'') FROM message_parts ORDER BY id",
     "SELECT effect_id,status,json_array(binding_json,payload_json) FROM effect_outbox JOIN effect_outbox_payloads USING(effect_id) ORDER BY effect_id",
     "SELECT * FROM core_checkpoints ORDER BY conversation_id",
     "SELECT * FROM core_delivery_witnesses ORDER BY conversation_id",
     "SELECT * FROM core_checkpoint_replay_bases ORDER BY conversation_id",
     "SELECT * FROM host_event_inbox ORDER BY event_id",
     "SELECT * FROM host_event_replay_heads ORDER BY run_id",
     "SELECT * FROM host_event_replay_order ORDER BY event_id",
     "SELECT * FROM run_events ORDER BY event_id"]
        .into_iter().map(|sql| {
            let mut statement = db.prepare(sql).unwrap();
            let count = statement.column_count();
            statement.query_map([], |row| {
                let contents = (0..count).map(|index| format!("{:?}", row.get_ref(index).unwrap())).collect::<Vec<_>>().join("|");
                Ok(format!("{:x}", Sha256::digest(contents.as_bytes())))
            }).unwrap().collect::<Result<Vec<_>, _>>().unwrap()
        }).collect()
}

fn user_input(id: &str, delivery: AgentInputDelivery) -> AgentSubmission {
    AgentSubmission::UserInput {
        items: vec![AgentItem::user_message(ItemId::from_string(id.into()), id)],
        turn_context_items: vec![],
        turn_context_platform: Default::default(),
        turn_context_components: vec![],
        submission_id: Some(format!("submit-{id}")),
        submitted_at_epoch_ms: Some(1000),
        delivery,
    }
}

fn pending(database: &std::path::Path, run: &str) -> Vec<HostRequestEnvelope> {
    load_committed_host_requests(&mut NolaStore::open(database).unwrap(), run)
        .unwrap()
        .requests
}

fn assert_attempt_checkpoint_binding(
    database: &std::path::Path,
    run: &str,
    effect_id: &str,
    attempt_id: &str,
    started: bool,
) -> (
    CoreCheckpoint<AgentThreadState>,
    nola_domain::ModelAttemptRecord,
) {
    let store = NolaStore::open(database).unwrap();
    let checkpoint = current_checkpoint(database);
    let core_request = checkpoint
        .state
        .host
        .pending_effects
        .get(effect_id)
        .expect("admission baseline must retain its exact pending effect")
        .validated_request()
        .unwrap();
    let effect = store.outbox_effect(effect_id).unwrap().unwrap();
    assert_eq!(effect.run_id, run);
    let outbox_request: HostRequestEnvelope = serde_json::from_str(&effect.payload_json).unwrap();
    let ledger = store
        .active_model_attempt_for_effect(effect_id)
        .unwrap()
        .unwrap();
    assert_eq!(ledger.run_id, run);
    assert_eq!(ledger.id, attempt_id);
    assert_eq!(ledger.started_at_ms.is_some(), started);
    for request in [&core_request, &outbox_request] {
        assert_eq!(request.attempt_id.as_deref(), Some(attempt_id));
        assert_eq!(
            request.attempt_started, started,
            "checkpoint and outbox must describe the same ledger admission"
        );
    }
    (checkpoint, ledger)
}

fn current_checkpoint(database: &std::path::Path) -> CoreCheckpoint<AgentThreadState> {
    let mut runtime = support::open_runtime(database).unwrap();
    let current: nola_application::ConversationCheckpoint = serde_json::from_value(
        runtime
            .execute(ApplicationCommand::LoadCheckpoint {
                conversation_id: "completion-conversation".into(),
            })
            .unwrap(),
    )
    .unwrap();
    serde_json::from_str(&current.checkpoint_json).unwrap()
}

#[test]
fn warm_sampling_admission_and_resampling_replay_from_their_own_baselines() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("warm-admission-replay.db");
    let mut store = NolaStore::open(&database).unwrap();
    store
        .create_conversation(&nola_store::CreateConversationInput {
            id: "completion-conversation".into(),
            title: "Replay Goal".into(),
            revision: 1,
            settings_json: Some(json!({"project_root":"/workspace/replay-goal"}).to_string()),
            created_at_ms: 900,
            updated_at_ms: 900,
        })
        .unwrap();
    support::goal_execution::admit_start(
        &mut store,
        nola_application::ReduceAndCommitIdleSubmission {
            conversation_id: "completion-conversation".into(),
            core_config: runtime_config(),
            submission: AgentSubmission::GoalRuntime {
                envelope: nola_agent_engine::agent::GoalRuntimeCommandEnvelope {
                    operation_id: "start-replay-goal".into(),
                    command: nola_agent_engine::agent::GoalRuntimeCommand::Start {
                        id: "replay-goal".into(),
                        objective: "Retain Goal through real Host replay".into(),
                        constraints: vec![],
                        requirements: vec![nola_agent_engine::agent::GoalRequirement {
                            acceptance_policy: nola_agent_engine::agent::goal::GoalAcceptancePolicy::ModelJudgment,
                            semantic_revision: 1, id: "cold-replay".into(),
                            description: "Recover exact Goal after Host replay".into(),
                            dependencies: vec![],
                            status: nola_agent_engine::agent::GoalRecordStatus::Pending,
                            evidence_refs: vec![],
                        }],
                        now_epoch_ms: 900,
                    },
                },
            },
            committed_at_ms: 900,
        },
    )
    .unwrap();
    let frontier = support::admission_frontier::read(&store, "completion-conversation");
    let begun = begin_assistant_turn(
        &mut store,
        BeginAssistantTurn {
            expected_configuration_revision: frontier.0,
            expected_parent_message_id: frontier.1,
            conversation_id: "completion-conversation".into(),
            conversation_title: "Completion".into(),
            user_message_id: "initial".into(),
            user_part_id: "initial:body".into(),
            user_text: "initial".into(),
            user_content_json: None,
            assistant_message_id: "submit-initial:assistant".into(),
            context_checkpoint_id: None,
            core_config: runtime_config(),
            submission: user_input("initial", AgentInputDelivery::CurrentTurn),
            created_at_ms: 1_000,
        },
    )
    .unwrap();
    let run = begun.begin.run_id;
    let digest = begun.invocation_config_digest;
    drop(store);
    let first = claim_and_start(&database, &run, &digest, 1_100);
    let initial_id = first.attempt_id.clone().unwrap();
    let effect_id = first.effect_id.0.clone();
    let mut runtime = support::open_runtime(&database).unwrap();
    // An ordinary progress title warms the Core but cannot authorize sampling.
    let progress = response_command(
        &first,
        &run,
        &digest,
        HostResponse::ToolProgress {
            title: "model_attempt_started".into(),
            detail: Some("not a typed admission".into()),
            metadata: serde_json::Value::Null,
        },
    );
    runtime
        .execute_received_command(
            nola_application::ExecuteReceivedCommand {
                client_id: "warm-admission-client".into(),
                request_id: "warm-not-admission".into(),
                command_type: "reduce_and_accept_host_response".into(),
                command_json: serde_json::to_string(&progress).unwrap(),
                expected_revision: None,
                received_at: "2000".into(),
            },
            progress,
        )
        .unwrap();
    let (_, prepared) =
        assert_attempt_checkpoint_binding(&database, &run, &effect_id, &initial_id, false);
    assert_eq!(prepared.state, nola_domain::ModelAttemptState::Prepared);

    for (index, attempt_id) in [initial_id.as_str(), "warm-replacement-attempt"]
        .into_iter()
        .enumerate()
    {
        let request = pending_with_delivery(&database, &run, first.delivery.clone());
        let mut admission = response_command(
            &request,
            &run,
            &digest,
            HostResponse::ModelAttemptStarted {
                provider: "fixture-provider".into(),
                model: "test-model".into(),
                reason: if index == 0 { "initial" } else { "retry" }.into(),
            },
        );
        let admission_time = 2_100 + i64::try_from(index).unwrap() * 200;
        if let ApplicationCommand::ReduceAndAcceptHostResponse { input } = &mut admission {
            input.response.attempt_id = Some(attempt_id.into());
            input.received_at_ms = admission_time;
        }
        runtime.execute(admission).unwrap();
        let (baseline, ledger) =
            assert_attempt_checkpoint_binding(&database, &run, &effect_id, attempt_id, true);
        assert_eq!(ledger.state, nola_domain::ModelAttemptState::Running);
        let request = pending_with_delivery(&database, &run, first.delivery.clone());
        let mut delta = response_command(
            &request,
            &run,
            &digest,
            HostResponse::ModelDelta {
                channel: ModelDeltaChannel::Text,
                part_id: ledger.text_part_id,
                delta: format!("sampling {index}"),
            },
        );
        let observed_at_ms = admission_time + 100;
        let response = if let ApplicationCommand::ReduceAndAcceptHostResponse { input } = &mut delta
        {
            input.received_at_ms = observed_at_ms;
            input.response.clone()
        } else {
            unreachable!()
        };
        let mut expected = AgentCore::restore(runtime_config(), baseline).unwrap();
        // A real receipt boundary durably records the small delta as a replay
        // tail, rather than manufacturing history or rebinding a latest row.
        let command_json = serde_json::to_string(&delta).unwrap();
        let receipt = nola_application::ExecuteReceivedCommand {
            client_id: "warm-admission-client".into(),
            request_id: format!("sampling-{index}-delta"),
            command_type: "reduce_and_accept_host_response".into(),
            command_json,
            expected_revision: None,
            received_at: observed_at_ms.to_string(),
        };
        let accepted = if index == 0 {
            // Keep this owner hot through the subsequent resampling admission.
            runtime.execute_received_command(receipt, delta).unwrap()
        } else {
            // The replacement's first delta also has to be deterministic when
            // a different production owner binds it from the durable outbox.
            let mut cold_producer = support::open_runtime(&database).unwrap();
            cold_producer
                .execute_received_command(receipt, delta)
                .unwrap()
        };
        assert!(matches!(
            accepted.outcome,
            nola_application::ApplicationCommandReceiptOutcome::Committed { .. }
        ));
        let store = NolaStore::open(&database).unwrap();
        assert!(
            store
                .checkpoint_replay_pending("completion-conversation")
                .unwrap(),
            "the cold reader must consume a real durable replay tail"
        );
        let recovery = nola_store_port::run_effect::RunEffectRepository::load_recovery_checkpoint(
            &store,
            "completion-conversation",
        )
        .unwrap()
        .unwrap();
        let tail = recovery
            .replay_inputs
            .last()
            .expect("actual committed delta replay fact");
        let facts: nola_store_port::host_replay::HostReplayReduction =
            serde_json::from_str(&tail.reduction_json).unwrap();
        let nola_store_port::host_replay::HostReplayReduction::Replayable {
            normalized_response,
            binding,
            decision_time_ms,
            ..
        } = facts
        else {
            panic!("new delta must contain complete replay evidence")
        };
        let normalized: HostResponseEnvelope =
            serde_json::from_str(normalized_response.get()).unwrap();
        assert_eq!(normalized, response);
        assert_eq!(decision_time_ms, observed_at_ms);
        if let Some(binding) = binding {
            let mut frozen = expected
                .committed_host_request_resume_snapshot(&response.effect_id.0)
                .unwrap()
                .unwrap();
            binding.apply_to(&mut frozen).unwrap();
            expected.bind_committed_host_effect(&frozen).unwrap();
        }
        expected
            .submit_at(
                AgentSubmission::HostResponse {
                    response: normalized,
                },
                u64::try_from(decision_time_ms).unwrap(),
            )
            .unwrap();
        drop(store);
        let mut resumed_store = NolaStore::open(&database).unwrap();
        let goal = nola_application::execute_application_command(
            &mut resumed_store,
            ApplicationCommand::FindResumableGoalSession {
                project_root: "/workspace/replay-goal".into(),
            },
        )
        .unwrap();
        let goal: nola_application::SessionProjection = serde_json::from_value(goal).unwrap();
        assert_eq!(goal.conversation_id, "completion-conversation");
        assert_eq!(goal.goal_projection.active.unwrap().id, "replay-goal");
        drop(resumed_store);
        let mut cold = support::open_runtime(&database).unwrap();
        let current: nola_application::ConversationCheckpoint = serde_json::from_value(
            cold.execute(ApplicationCommand::LoadCheckpoint {
                conversation_id: "completion-conversation".into(),
            })
            .unwrap(),
        )
        .unwrap();
        let recovered: CoreCheckpoint<AgentThreadState> =
            serde_json::from_str(&current.checkpoint_json).unwrap();
        assert_eq!(serde_json::to_value(expected.checkpoint(recovered.reason)).unwrap(),
            serde_json::to_value(&recovered).unwrap(),
            "typed admission, checkpoint and response-only replay must reproduce the complete Core state");
    }
}

#[test]
fn physical_usage_commits_before_validation_failure_and_survives_cold_restore() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("rejected-output-usage.db");
    let mut store = NolaStore::open(&database).unwrap();
    let begun = begin_assistant_turn(&mut store, BeginAssistantTurn {
        expected_configuration_revision: None,
        expected_parent_message_id: None,
        conversation_id: "completion-conversation".into(), conversation_title: "Usage".into(),
        user_message_id: "initial".into(), user_part_id: "initial:body".into(),
        user_text: "Preserve accounting even when output is rejected".into(), user_content_json: None,
        assistant_message_id: "submit-initial:assistant".into(), context_checkpoint_id: None,
        core_config: runtime_config(), submission: user_input("initial", AgentInputDelivery::CurrentTurn),
        created_at_ms: 1000,
    }).unwrap();
    let run = begun.begin.run_id;
    let digest = begun.invocation_config_digest;
    drop(store);
    let first = claim_and_start(&database, &run, &digest, 1100);
    let usage = json!({"input_tokens":70014,"output_tokens":8000,
        "cache_usage":nola_domain::PromptCacheUsage::measured(Some(70014),Some(0),None,true)});
    for payload in [
        HostResponse::ModelAttemptStarted { provider:"fixture".into(),model:"model".into(),reason:"initial".into() },
        HostResponse::ModelUsageReported { usage:usage.clone() },
        HostResponse::Failed { error:HostError { code:"invalid_compaction_summary".into(),
            message:"Summary exhausted its output budget".into(),retryable:true,http_status:None } },
    ] {
        let current = pending_with_delivery(&database, &run, first.delivery.clone());
        // Each response is accepted by a fresh process-equivalent runtime.
        let mut runtime = support::open_runtime(&database).unwrap();
        runtime.execute(response_command(&current, &run, &digest, payload)).unwrap();
    }
    let store = NolaStore::open(&database).unwrap();
    let attempt = store.active_model_attempt_for_effect(&first.effect_id.0).unwrap().unwrap();
    assert_eq!(attempt.state, nola_domain::ModelAttemptState::Failed);
    assert_eq!(attempt.usage, usage);
    let total = store.conversation_cache_usage("completion-conversation").unwrap().unwrap();
    assert_eq!(total.reported_requests, 1);
    assert_eq!(total.totals.input_tokens, Some(70014));
    assert_eq!(total.totals.hit_rate_bps, Some(0));
}

#[test]
fn output_limited_attempt_settles_before_bounded_continuation_is_published() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("output-limit-attempt.db");
    let mut store = NolaStore::open(&database).unwrap();
    let begun = begin_assistant_turn(
        &mut store,
        BeginAssistantTurn {
            expected_configuration_revision: None,
            expected_parent_message_id: None,
            conversation_id: "completion-conversation".into(),
            conversation_title: "Completion".into(),
            user_message_id: "initial".into(),
            user_part_id: "initial:body".into(),
            user_text: "Return a response longer than one provider output window".into(),
            user_content_json: None,
            assistant_message_id: "submit-initial:assistant".into(),
            context_checkpoint_id: None,
            core_config: runtime_config(),
            submission: user_input("initial", AgentInputDelivery::CurrentTurn),
            created_at_ms: 1000,
        },
    )
    .unwrap();
    let run = begun.begin.run_id;
    let digest = begun.invocation_config_digest;
    drop(store);

    let first = claim_and_start(&database, &run, &digest, 1_100);
    let first_attempt = first.attempt_id.clone().expect("prepared Attempt");
    let mut runtime = support::open_runtime(&database).unwrap();
    runtime
        .execute(response_command(
            &first,
            &run,
            &digest,
            HostResponse::ModelAttemptStarted {
                provider: "fixture-provider".into(),
                model: "test-model".into(),
                reason: "initial".into(),
            },
        ))
        .unwrap();
    assert_attempt_checkpoint_binding(&database, &run, &first.effect_id.0, &first_attempt, true);
    drop(runtime);

    let current = pending_with_delivery(&database, &run, first.delivery.clone());
    let mut runtime = support::open_runtime(&database).unwrap();
    runtime
        .execute(response_command(
            &current,
            &run,
            &digest,
            HostResponse::ModelOutputLimited {
                items: vec![serde_json::to_value(AgentItem::assistant_message(
                    ItemId::from_string("partial-output".into()),
                    "BEGIN\n```cpp\nstd::vector<std",
                ))
                .unwrap()],
                tool_calls: vec![],
                usage: json!({"output_tokens": 128}),
            },
        ))
        .unwrap();
    drop(runtime);

    let continuation = pending(&database, &run);
    assert_eq!(
        continuation.len(),
        1,
        "Core must publish one bounded continuation"
    );
    let continuation_attempt = continuation[0]
        .attempt_id
        .as_deref()
        .expect("continuation has a prepared Attempt");
    assert_ne!(continuation_attempt, first_attempt);
    let db = rusqlite::Connection::open(&database).unwrap();
    assert_eq!(
        db.query_row::<String, _, _>(
            "SELECT state FROM run_attempts WHERE id=?1",
            [&first_attempt],
            |row| row.get(0),
        )
        .unwrap(),
        "succeeded"
    );
    assert_eq!(
        db.query_row::<String, _, _>(
            "SELECT state FROM run_attempts WHERE id=?1",
            [continuation_attempt],
            |row| row.get(0),
        )
        .unwrap(),
        "prepared"
    );
    assert_eq!(
        db.query_row::<String, _, _>(
            "SELECT active_attempt_id FROM runs WHERE id=?1",
            [&run],
            |row| row.get(0),
        )
        .unwrap(),
        continuation_attempt
    );
}

#[test]
fn retired_attempt_delta_is_rejected_after_new_sampling_admission_and_cold_restore() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("attempt.db");
    let mut store = NolaStore::open(&database).unwrap();
    let begun = begin_assistant_turn(
        &mut store,
        BeginAssistantTurn {
            expected_configuration_revision: None,
            expected_parent_message_id: None,
            conversation_id: "completion-conversation".into(),
            conversation_title: "Completion".into(),
            user_message_id: "initial".into(),
            user_part_id: "initial:body".into(),
            user_text: "initial".into(),
            user_content_json: None,
            assistant_message_id: "submit-initial:assistant".into(),
            context_checkpoint_id: None,
            core_config: runtime_config(),
            submission: user_input("initial", AgentInputDelivery::CurrentTurn),
            created_at_ms: 1000,
        },
    )
    .unwrap();
    let run = begun.begin.run_id;
    let digest = begun.invocation_config_digest;
    drop(store);
    let first = claim_and_start(&database, &run, &digest, 1_100);
    let initial_id = first.attempt_id.clone().expect("durably prepared Attempt");
    let mut runtime = support::open_runtime(&database).unwrap();
    runtime
        .execute(response_command(
            &first,
            &run,
            &digest,
            HostResponse::ModelAttemptStarted {
                provider: "fixture-provider".into(),
                model: "test-model".into(),
                reason: "initial".into(),
            },
        ))
        .unwrap();
    assert_attempt_checkpoint_binding(&database, &run, &first.effect_id.0, &initial_id, true);
    drop(runtime);
    let current = pending_with_delivery(&database, &run, first.delivery.clone());
    let next_id = "replacement-attempt";
    let mut restart = response_command(
        &current,
        &run,
        &digest,
        HostResponse::ModelAttemptStarted {
            provider: "fixture-provider".into(),
            model: "test-model".into(),
            reason: "retry".into(),
        },
    );
    if let ApplicationCommand::ReduceAndAcceptHostResponse { input } = &mut restart {
        input.response.attempt_id = Some(next_id.into());
        input.response.sequence = 3;
    }
    let mut runtime = support::open_runtime(&database).unwrap();
    runtime.execute(restart).unwrap();
    assert_attempt_checkpoint_binding(&database, &run, &first.effect_id.0, next_id, true);
    drop(runtime);
    let current = pending_with_delivery(&database, &run, first.delivery.clone());
    assert_eq!(current.attempt_id.as_deref(), Some(next_id));
    let db = rusqlite::Connection::open(&database).unwrap();
    let before = canonical_rows(&db);
    let ledger = || {
        db.query_row::<String,_,_>("SELECT group_concat(record_json,'|') FROM (SELECT record_json FROM run_attempts ORDER BY id)",[],|r|r.get(0)).unwrap()
    };
    let before_ledger = ledger();
    let HostRequest::ModelStream {
        text_part_id: old_part,
        ..
    } = &first.payload
    else {
        panic!("model")
    };
    let mut late = response_command(
        &current,
        &run,
        &digest,
        HostResponse::ModelDelta {
            channel: ModelDeltaChannel::Text,
            part_id: old_part.clone(),
            delta: "OLD_LATE_ATTEMPT".into(),
        },
    );
    if let ApplicationCommand::ReduceAndAcceptHostResponse { input } = &mut late {
        input.response.attempt_id = Some(initial_id);
        input.response.sequence = 4;
    }
    let mut runtime = support::open_runtime(&database).unwrap();
    let error = runtime.execute(late).unwrap_err();
    assert!(error.contains("retired Attempt"), "{error}");
    drop(runtime);
    assert_eq!(canonical_rows(&db), before);
    assert_eq!(ledger(), before_ledger);
    // Rejection must not consume sequence 4 or poison the current Attempt.
    let mut valid = response_command(
        &current,
        &run,
        &digest,
        HostResponse::ModelCompleted {
            items: vec![serde_json::to_value(AgentItem::assistant_message(
                ItemId::from_string("clean-result".into()),
                "Clean answer",
            ))
            .unwrap()],
            tool_calls: vec![],
            usage: json!({}),
        },
    );
    if let ApplicationCommand::ReduceAndAcceptHostResponse { input } = &mut valid {
        input.response.sequence = 4;
    }
    let mut runtime = support::open_runtime(&database).unwrap();
    runtime.execute(valid).unwrap();
    drop(runtime);
    assert_eq!(
        db.query_row::<String, _, _>(
            "SELECT state FROM run_attempts WHERE id=?1",
            [next_id],
            |r| r.get(0)
        )
        .unwrap(),
        "succeeded"
    );
    assert!(!serde_json::to_string(&current_checkpoint(&database))
        .unwrap()
        .contains("OLD_LATE_ATTEMPT"));
}

#[test]
fn attempt_admission_receipt_final_write_rolls_back_then_cold_replay_is_exact() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("attempt-receipt.db");
    let mut store = NolaStore::open(&database).unwrap();
    let begun = begin_assistant_turn(
        &mut store,
        BeginAssistantTurn {
            expected_configuration_revision: None,
            expected_parent_message_id: None,
            conversation_id: "completion-conversation".into(),
            conversation_title: "Completion".into(),
            user_message_id: "initial".into(),
            user_part_id: "initial:body".into(),
            user_text: "initial".into(),
            user_content_json: None,
            assistant_message_id: "submit-initial:assistant".into(),
            context_checkpoint_id: None,
            core_config: runtime_config(),
            submission: user_input("initial", AgentInputDelivery::CurrentTurn),
            created_at_ms: 1000,
        },
    )
    .unwrap();
    let run = begun.begin.run_id;
    let digest = begun.invocation_config_digest;
    drop(store);
    let first = claim_and_start(&database, &run, &digest, 1_100);

    let attempt_id = first.attempt_id.as_deref().expect("prepared Attempt owner");
    let db = rusqlite::Connection::open(&database).unwrap();
    assert_eq!(
        db.query_row::<String, _, _>(
            "SELECT state FROM run_attempts WHERE id=?1",
            [attempt_id],
            |r| r.get(0)
        )
        .unwrap(),
        "prepared"
    );
    let rows = || {
        let mut result = canonical_rows(&db);
        for table in [
            "runs",
            "run_attempts",
            "run_attempt_deliveries",
            "host_event_inbox",
            "run_events",
            "assistant_journal_heads",
            "assistant_journal_records",
            "assistant_journal_parts",
            "assistant_journal_chunks",
            "assistant_journal_publications",
            "assistant_journal_publication_frames",
        ] {
            let mut query = db
                .prepare(&format!("SELECT * FROM {table} ORDER BY 1"))
                .unwrap();
            let columns = query.column_count();
            result.push(
                query
                    .query_map([], |row| {
                        Ok((0..columns)
                            .map(|i| format!("{:?}", row.get_ref(i).unwrap()))
                            .collect::<Vec<_>>()
                            .join("|"))
                    })
                    .unwrap()
                    .collect::<Result<Vec<_>, _>>()
                    .unwrap(),
            );
        }
        result
    };
    let before = rows();
    let command = response_command(
        &first,
        &run,
        &digest,
        HostResponse::ModelAttemptStarted {
            provider: "fixture-provider".into(),
            model: "test-model".into(),
            reason: "initial".into(),
        },
    );
    let command_json = serde_json::to_string(&command).unwrap();
    let receipt = || nola_application::ExecuteReceivedCommand {
        client_id: "attempt-receipt-client".into(),
        request_id: "initial-admission".into(),
        command_type: "reduce_and_accept_host_response".into(),
        command_json: command_json.clone(),
        expected_revision: None,
        received_at: "2000".into(),
    };
    // Fault only at final receipt publication, after the Attempt mutation is visible
    // inside this same transaction. No inbox or ledger trigger substitutes for it.
    db.execute_batch("CREATE TRIGGER fail_attempt_receipt BEFORE UPDATE ON command_receipts WHEN NEW.status='committed' BEGIN SELECT CASE WHEN EXISTS(SELECT 1 FROM run_attempts WHERE state='running') THEN RAISE(ABORT,'attempt receipt final fault') ELSE RAISE(ABORT,'attempt missing before receipt') END; END").unwrap();
    let mut runtime = support::open_runtime(&database).unwrap();
    let error = runtime
        .execute_received_command(receipt(), command)
        .unwrap_err();
    assert!(
        error.message.contains("attempt receipt final fault"),
        "{error}"
    );
    assert_eq!(
        rows(),
        before,
        "Receipt failure must undo Attempt, active owner, checkpoint, Parts and effect publication"
    );
    assert_attempt_checkpoint_binding(&database, &run, &first.effect_id.0, attempt_id, false);
    assert_eq!(
        db.query_row::<i64, _, _>(
            "SELECT count(*) FROM command_receipts WHERE status='committed'",
            [],
            |r| r.get(0)
        )
        .unwrap(),
        0
    );
    db.execute_batch("DROP TRIGGER fail_attempt_receipt")
        .unwrap();
    let accepted = runtime
        .execute_received_command(receipt(), serde_json::from_str(&command_json).unwrap())
        .unwrap();
    assert!(matches!(
        accepted.outcome,
        nola_application::ApplicationCommandReceiptOutcome::Committed { .. }
    ));
    assert_eq!(
        db.query_row::<(String, i64), _, _>(
            "SELECT state,revision FROM run_attempts WHERE id=?1",
            [attempt_id],
            |r| Ok((r.get(0)?, r.get(1)?))
        )
        .unwrap(),
        ("running".into(), 2)
    );
    assert_eq!(
        db.query_row::<String, _, _>(
            "SELECT active_attempt_id FROM runs WHERE id=?1",
            [&run],
            |r| r.get(0)
        )
        .unwrap(),
        attempt_id
    );
    assert_attempt_checkpoint_binding(&database, &run, &first.effect_id.0, attempt_id, true);
    let committed = rows();
    drop(runtime);
    let mut runtime = support::open_runtime(&database).unwrap();
    assert_eq!(rows(), committed);
    let replay = runtime
        .execute_received_command(receipt(), serde_json::from_str(&command_json).unwrap())
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.outcome, accepted.outcome);
    assert_eq!(
        rows(),
        committed,
        "Cold receipt replay must not advance revision or create another Attempt"
    );
    assert_eq!(
        db.query_row::<i64, _, _>("SELECT count(*) FROM run_attempts", [], |r| r.get(0))
            .unwrap(),
        1
    );
}

#[test]
fn delegated_acceptance_and_worker_interruption_survive_cold_recovery_without_redispatch() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("delegated.db");
    let mut store = NolaStore::open(&database).unwrap();
    let mut config = runtime_config();
    config.host_catalog.as_mut().unwrap().capabilities.insert(
        "agent.delegate".into(),
        runtime_capability("agent.delegate", HostCapabilityRoute::Model),
    );
    config.turn_identity.execution_backend =
        nola_domain::execution_backend::ExecutionBackendSelection {
            backend_id: "codex".into(),
            ..Default::default()
        };
    config.turn_identity.prompt.stable_prefix_digest = "a".repeat(64);
    let begun = begin_assistant_turn(
        &mut store,
        BeginAssistantTurn {
            expected_configuration_revision: None,
            expected_parent_message_id: None,
            conversation_id: "completion-conversation".into(),
            conversation_title: "Delegated recovery".into(),
            user_message_id: "initial".into(),
            user_part_id: "initial:body".into(),
            user_text: "initial".into(),
            user_content_json: None,
            assistant_message_id: "submit-initial:assistant".into(),
            context_checkpoint_id: None,
            core_config: config,
            submission: user_input("initial", AgentInputDelivery::CurrentTurn),
            created_at_ms: 1000,
        },
    )
    .unwrap();
    let run = begun.begin.run_id;
    let digest = begun.invocation_config_digest;
    drop(store);
    let original = claim_and_start(&database, &run, &digest, 1_100);
    assert!(original.attempt_id.is_none());
    let invocation_id = original
        .delegated_invocation_id
        .clone()
        .expect("independent prepared invocation");
    assert_ne!(invocation_id, original.request_id);
    assert_eq!(original.resume_after_sequence, Some(1));
    let accepted = original.clone();
    assert_eq!(accepted.resume_after_sequence, Some(1));
    assert_eq!(accepted.payload, original.payload);
    let emit = |sequence, payload| ApplicationCommand::ReduceAndAcceptHostResponse {
        input: ReduceAndAcceptHostResponse {
            conversation_id: "completion-conversation".into(),
            run_id: run.clone(),
            invocation_config_digest: digest.clone(),
            response: response_for_request(&accepted, sequence, payload),
            received_at_ms: 2500,
        },
    };
    let mut runtime = support::open_runtime(&database).unwrap();
    runtime
        .execute(emit(
            2,
            HostResponse::DelegatedAgentEvent {
                event: nola_host_protocol::DelegatedAgentEventV1::SessionStarted {
                    backend_id: "codex".into(),
                    backend_session_id: "canonical-session".into(),
                },
            },
        ))
        .unwrap();
    let before_rejected = canonical_rows(&rusqlite::Connection::open(&database).unwrap());
    assert!(runtime
        .execute(emit(
            3,
            HostResponse::DelegatedAgentEvent {
                event: nola_host_protocol::DelegatedAgentEventV1::SessionStarted {
                    backend_id: "codex".into(),
                    backend_session_id: "replacement-session".into(),
                }
            }
        ))
        .is_err());
    assert!(runtime
        .execute(emit(
            3,
            HostResponse::DelegatedAgentEvent {
                event: nola_host_protocol::DelegatedAgentEventV1::TextDelta {
                    part_id: "foreign-invocation-part".into(),
                    delta: "rejected".into(),
                }
            }
        ))
        .is_err());
    assert_eq!(
        canonical_rows(&rusqlite::Connection::open(&database).unwrap()),
        before_rejected
    );
    let HostRequest::DelegatedAgent { request: delegated } = &original.payload else {
        unreachable!()
    };
    runtime
        .execute(emit(
            3,
            HostResponse::DelegatedAgentEvent {
                event: nola_host_protocol::DelegatedAgentEventV1::TextDelta {
                    part_id: delegated.text_part_id.clone(),
                    delta: "canonical delegated partial".into(),
                },
            },
        ))
        .unwrap();
    drop(runtime);
    let store = NolaStore::open(&database).unwrap();
    let ledger = store.delegated_invocation(&invocation_id).unwrap().unwrap();
    assert_eq!(
        ledger.backend_session_id.as_deref(),
        Some("canonical-session")
    );
    assert_eq!(ledger.state, nola_domain::DelegatedInvocationState::Running);
    let window = nola_application::load_conversation_projection_window_v1(
        &store,
        "completion-conversation",
        None,
        20,
        32768,
    )
    .unwrap();
    let owner = window
        .stream_owners
        .unwrap()
        .into_iter()
        .find(|owner| owner.part_id.0 == delegated.text_part_id)
        .unwrap();
    assert_eq!(
        owner.execution,
        nola_domain::ProjectionExecutionOwner::DelegatedInvocation {
            invocation_id: invocation_id.clone()
        }
    );
    drop(store);
    let mut runtime = support::open_runtime(&database).unwrap();
    runtime.execute(ApplicationCommand::ReduceAndAcceptHostResponse {
        input: ReduceAndAcceptHostResponse {
            conversation_id: "completion-conversation".into(),
            run_id: run.clone(),
            invocation_config_digest: digest,
            response: response_for_request(&accepted, 4, HostResponse::DelegatedExecutionInterrupted {
                reason: nola_host_protocol::DelegatedExecutionInterruptionReason::WorkerUnavailable,
            }),
            received_at_ms: 3000,
        },
    }).unwrap();
    drop(runtime);
    let mut runtime = support::open_runtime(&database).unwrap();
    runtime
        .execute(ApplicationCommand::RecoverOrphanedRuns {
            conversation_id: Some("completion-conversation".into()),
            reason: "server_restarted".into(),
            interrupted_at: "after".into(),
            interrupted_at_ms: 4000,
        })
        .unwrap();
    drop(runtime);
    let requests = pending(&database, &run);
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].request_id, original.request_id);
    assert_eq!(requests[0].effect_id, original.effect_id);
    assert_eq!(requests[0].payload, original.payload);
    assert_eq!(requests[0].resume_after_sequence, Some(4));
    let db = rusqlite::Connection::open(&database).unwrap();
    let checkpoint = serde_json::to_string(&current_checkpoint(&database)).unwrap();
    assert!(checkpoint.contains("delegated_interrupted"));
    assert!(checkpoint.contains("worker_unavailable"));
    let attempt_count: i64 = db
        .query_row("SELECT count(*) FROM run_attempts", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        attempt_count, 0,
        "An opaque delegated invocation is not a model sampling Attempt"
    );
}

fn delegated_config() -> AgentCoreConfig {
    let mut config = runtime_config();
    config.host_catalog.as_mut().unwrap().capabilities.insert(
        "agent.delegate".into(),
        runtime_capability("agent.delegate", HostCapabilityRoute::Model),
    );
    config.turn_identity.execution_backend =
        nola_domain::execution_backend::ExecutionBackendSelection {
            backend_id: "codex".into(),
            ..Default::default()
        };
    config.turn_identity.prompt.stable_prefix_digest = "a".repeat(64);
    config
}

#[test]
fn delegated_admission_receipt_final_write_rolls_back_then_cold_replay_is_exact() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("attempt-receipt.db");
    let mut store = NolaStore::open(&database).unwrap();
    let begun = begin_assistant_turn(
        &mut store,
        BeginAssistantTurn {
            expected_configuration_revision: None,
            expected_parent_message_id: None,
            conversation_id: "completion-conversation".into(),
            conversation_title: "Completion".into(),
            user_message_id: "initial".into(),
            user_part_id: "initial:body".into(),
            user_text: "initial".into(),
            user_content_json: None,
            assistant_message_id: "submit-initial:assistant".into(),
            context_checkpoint_id: None,
            core_config: delegated_config(),
            submission: user_input("initial", AgentInputDelivery::CurrentTurn),
            created_at_ms: 1000,
        },
    )
    .unwrap();
    let run = begun.begin.run_id;
    let digest = begun.invocation_config_digest;
    drop(store);
    let first = claim_request(&database, &run, 1_100);

    let attempt_id = first
        .delegated_invocation_id
        .as_deref()
        .expect("prepared Delegated invocation owner");
    let db = rusqlite::Connection::open(&database).unwrap();
    assert_eq!(
        db.query_row::<String, _, _>(
            "SELECT state FROM delegated_invocations WHERE id=?1",
            [attempt_id],
            |r| r.get(0)
        )
        .unwrap(),
        "prepared"
    );
    let rows = || {
        let mut result = canonical_rows(&db);
        for table in [
            "runs",
            "delegated_invocations",
            "delegated_invocation_deliveries",
            "host_event_inbox",
            "run_events",
        ] {
            let mut query = db
                .prepare(&format!("SELECT * FROM {table} ORDER BY 1"))
                .unwrap();
            let columns = query.column_count();
            result.push(
                query
                    .query_map([], |row| {
                        Ok((0..columns)
                            .map(|i| format!("{:?}", row.get_ref(i).unwrap()))
                            .collect::<Vec<_>>()
                            .join("|"))
                    })
                    .unwrap()
                    .collect::<Result<Vec<_>, _>>()
                    .unwrap(),
            );
        }
        result
    };
    let before = rows();
    let command = response_command(&first, &run, &digest, HostResponse::ExecutionStarted);
    let command_json = serde_json::to_string(&command).unwrap();
    let receipt = || nola_application::ExecuteReceivedCommand {
        client_id: "attempt-receipt-client".into(),
        request_id: "initial-admission".into(),
        command_type: "reduce_and_accept_host_response".into(),
        command_json: command_json.clone(),
        expected_revision: None,
        received_at: "2000".into(),
    };
    // Fault only at final receipt publication, after the Delegated invocation mutation is visible
    // inside this same transaction. No inbox or ledger trigger substitutes for it.
    db.execute_batch("CREATE TRIGGER fail_attempt_receipt BEFORE UPDATE ON command_receipts WHEN NEW.status='committed' BEGIN SELECT CASE WHEN EXISTS(SELECT 1 FROM delegated_invocations WHERE state='running') THEN RAISE(ABORT,'delegated receipt final fault') ELSE RAISE(ABORT,'attempt missing before receipt') END; END").unwrap();
    let mut runtime = support::open_runtime(&database).unwrap();
    let error = runtime
        .execute_received_command(receipt(), command)
        .unwrap_err();
    assert!(
        error.message.contains("delegated receipt final fault"),
        "{error}"
    );
    assert_eq!(
        rows(),
        before,
        "Receipt failure must undo Delegated invocation, active owner, checkpoint, Parts and effect publication"
    );
    assert_eq!(
        db.query_row::<i64, _, _>(
            "SELECT count(*) FROM command_receipts WHERE status='committed'",
            [],
            |r| r.get(0)
        )
        .unwrap(),
        0
    );
    db.execute_batch("DROP TRIGGER fail_attempt_receipt")
        .unwrap();
    let accepted = runtime
        .execute_received_command(receipt(), serde_json::from_str(&command_json).unwrap())
        .unwrap();
    assert!(matches!(
        accepted.outcome,
        nola_application::ApplicationCommandReceiptOutcome::Committed { .. }
    ));
    assert_eq!(
        db.query_row::<(String, i64), _, _>(
            "SELECT state,revision FROM delegated_invocations WHERE id=?1",
            [attempt_id],
            |r| Ok((r.get(0)?, r.get(1)?))
        )
        .unwrap(),
        ("running".into(), 2)
    );
    assert_eq!(
        db.query_row::<i64, _, _>("SELECT count(*) FROM run_attempts", [], |row| row.get(0))
            .unwrap(),
        0
    );
    let committed = rows();
    drop(runtime);
    let mut runtime = support::open_runtime(&database).unwrap();
    assert_eq!(rows(), committed);
    let replay = runtime
        .execute_received_command(receipt(), serde_json::from_str(&command_json).unwrap())
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.outcome, accepted.outcome);
    assert_eq!(
        rows(),
        committed,
        "Cold receipt replay must not advance revision or create another Delegated invocation"
    );
    assert_eq!(
        db.query_row::<i64, _, _>("SELECT count(*) FROM delegated_invocations", [], |r| r
            .get(0))
            .unwrap(),
        1
    );
}
