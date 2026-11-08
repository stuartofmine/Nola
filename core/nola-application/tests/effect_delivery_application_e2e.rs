// SPDX-License-Identifier: Apache-2.0
//! Real Application + SQLite coverage for Host-effect delivery fencing.

mod support;

use nola_application::{
    begin_assistant_turn, claim_committed_host_requests, heartbeat_effect_delivery,
    load_committed_host_requests, reduce_and_accept_host_response,
    reduce_and_commit_active_submission, ApplicationCommand, ResolveUnknownEffectRecovery, UnknownEffectRecoveryChoice,
    BeginAssistantTurn, ClaimCommittedHostRequests, ExecuteReceivedCommand,
    HeartbeatEffectDelivery, ReduceAndAcceptHostResponse, ReduceAndCommitActiveSubmission,
};
use nola_agent_engine::agent::host::*;
use nola_agent_engine::agent::protocol::*;
use nola_domain::{EffectDeliveryAttemptState, EffectDeliverySettlement};
use nola_store::NolaStore;
use std::collections::BTreeMap;

fn model_capability() -> HostCapabilityDescriptor {
    HostCapabilityDescriptor {
        id: "model.stream".into(),
        title: "model.stream".into(),
        route: HostCapabilityRoute::Model,
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
        supports_streaming: true,
        supports_argument_streaming: false,
        parallelism: HostCapabilityParallelism::Serial,
        cancellation: HostCancellationMode::Cooperative,
        replay_safety: HostReplaySafety::Recoverable,
    }
}

fn heartbeat(
    run_id: &str,
    lease: &EffectDeliveryLease,
    heartbeat_at_ms: i64,
    requested_lease_expires_at_ms: i64,
) -> HeartbeatEffectDelivery {
    HeartbeatEffectDelivery {
        run_id: run_id.into(),
        attempt_id: lease.attempt_id.clone(),
        worker_generation: lease.worker_generation.clone(),
        lease_owner: lease.lease_owner.clone(),
        lease_token: lease.lease_token.clone(),
        current_lease_expires_at_ms: lease.lease_expires_at_ms,
        heartbeat_at_ms,
        requested_lease_expires_at_ms,
    }
}

fn recovery_host_binding(model: &str) -> nola_host_protocol::HostInvocationBinding {
    use nola_domain::generation::{resolve_output_limit, OutputTokenLimits, OutputTokenPreference};
    use nola_host_protocol::*;
    let route = HostModelRouteBinding::Http(nola_host_protocol::HostHttpModelRouteBinding {
        siwc_account: None,
        protocol: nola_domain::generation::ModelProtocol::ChatCompletions,
        output_token_field: nola_host_protocol::ModelOutputTokenField::MaxTokens,
        provider: Some("fixture".into()), endpoint: "https://fixture.invalid/v1".into(),
        endpoint_query_sha256: None, model: model.into(), credential_sha256: "a".repeat(64),
        credential_source: nola_host_protocol::HostSecretSource::Ephemeral,
        endpoint_query_source: nola_host_protocol::HostSecretSource::None,
    });
    let preference = OutputTokenPreference::Default;
    let resolved = resolve_output_limit(preference, OutputTokenLimits { known_output_cap: None }).unwrap();
    let binding = HostInvocationBinding {
        mac_launch: None,
        version: HOST_INVOCATION_BINDING_VERSION, primary: route.clone(), fallbacks: vec![],
        generation: HostGenerationBinding {
            max_retries: 2, parallel_tool_calls: None,
            reasoning_selection: None, sampling_override: false,
            reasoning_tier: Default::default(),
            generation_options: nola_domain::generation::GenerationOptions::default().try_into().unwrap(),
            temperature: serde_json::Number::from_f64(0.7).unwrap(),
            top_p: serde_json::Number::from_f64(0.9).unwrap(), max_output_tokens: resolved.tokens,
            thinking_enabled: false, reasoning_summary_supported: None, prompt_cache: false, prompt_cache_key_sha256: None,
            prompt_cache_key_prefix_sha256: None, prompt_cache_retention: None,
        },
        output_admission: HostOutputAdmission {
            version: HOST_OUTPUT_ADMISSION_VERSION, preference, resolved,
            configured_context_window: None,
            routes: vec![HostRouteTokenLimits { route, max_input_tokens: None,
                max_output_tokens: None, max_total_tokens: None }],
        },
        execution_policy_sha256: "b".repeat(64),
    };
    binding.validate().unwrap();
    binding
}

fn core_config() -> AgentCoreConfig {
    let mut capabilities = BTreeMap::new();
    capabilities.insert("model.stream".into(), model_capability());
    AgentCoreConfig {
        turn_identity: support::model_identity::identity(),
        session_id: Some(SessionId::from_string("delivery-conversation".into())),
        model: Some("delivery-model".into()),
        host_catalog: Some(HostCapabilityCatalog {
            host_id: HostId("delivery-host".into()),
            revision: 1,
            availability_epoch: 1,
            lease: None,
            capabilities,
        }),
        ..AgentCoreConfig::default()
    }
}

fn user_input() -> AgentSubmission {
    AgentSubmission::UserInput {
        items: vec![AgentItem::user_message(
            ItemId::from_string("delivery-input".into()),
            "执行一次模型请求",
        )],
        turn_context_items: Vec::new(),
        turn_context_platform: Default::default(),
        turn_context_components: Vec::new(),
        submission_id: Some("delivery-submission".into()),
        submitted_at_epoch_ms: Some(1_000),
        delivery: AgentInputDelivery::CurrentTurn,
    }
}

fn response(
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

#[test]
fn immediate_context_blocker_is_committed_without_a_host_effect() {
    immediate_context_blocker(false);
}

#[test]
fn immediate_context_blocker_rolls_back_if_terminal_sealing_fails() {
    immediate_context_blocker(true);
}

fn immediate_context_blocker(inject_seal_failure: bool) {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("immediate-blocker.db");
    let store = NolaStore::open(&database).unwrap();
    if inject_seal_failure {
        rusqlite::Connection::open(&database).unwrap().execute_batch(
            "CREATE TRIGGER reject_test_terminal BEFORE INSERT ON assistant_run_terminal_seals
             BEGIN SELECT RAISE(ABORT, 'injected terminal seal failure'); END;"
        ).unwrap();
    }
    let mut config = core_config();
    config.context_window.context_window_tokens = 4096;
    config.context_window.reserved_output_tokens = 512;
    let text = "required-input ".repeat(2000);
    let mut submission = user_input();
    if let AgentSubmission::UserInput { items, .. } = &mut submission {
        *items = vec![AgentItem::user_message(ItemId::from_string("delivery-input".into()), text.clone())];
    }
    let command = BeginAssistantTurn {
        expected_configuration_revision: None,
        expected_parent_message_id: None,
        conversation_id: "delivery-conversation".into(), conversation_title: "Budget".into(),
        user_message_id: "delivery-user".into(), user_part_id: "delivery-user-part".into(),
        user_text: text.clone(), user_content_json: None,
        assistant_message_id: "delivery-assistant".into(), context_checkpoint_id: None,
        core_config: config, submission, created_at_ms: 1000,
    };
    let mut runtime = support::open_runtime(&database).unwrap();
    let begun = runtime.execute(ApplicationCommand::BeginAssistantTurn { input: command })
        .and_then(|value| serde_json::from_value::<nola_application::AcceptedAssistantTurn>(value)
            .map_err(|error| error.to_string()));
    drop(runtime);
    if inject_seal_failure {
        assert!(begun.unwrap_err().contains("injected terminal seal failure"));
        drop(store);
        let db = rusqlite::Connection::open(&database).unwrap();
        for table in ["runs", "messages", "core_checkpoints", "committed_transitions", "run_results", "assistant_run_terminal_seals"] {
            let count: i64 = db.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row|row.get(0)).unwrap();
            assert_eq!(count, 0, "partial admission leaked into {table}");
        }
        return;
    }
    let begun = begun.unwrap();
    assert!(load_committed_host_requests(&store, &begun.begin.run_id).unwrap().requests.is_empty());
    drop(store);
    let reopened = NolaStore::open(&database).unwrap();
    use nola_store_port::assistant_journal::AssistantJournalRepository;
    let seal = reopened.local_assistant_run_terminal_seal("delivery-conversation", &begun.begin.run_id).unwrap();
    assert!(seal.is_some());
    let db = rusqlite::Connection::open(&database).unwrap();
    let (status, kind): (String, String) = db.query_row(
        "SELECT r.status,json_extract(rr.result_json,'$.value.kind') FROM runs r JOIN run_results rr ON rr.run_id=r.id WHERE r.id=?1",
        [&begun.begin.run_id], |r| Ok((r.get(0)?, r.get(1)?)),
    ).unwrap();
    assert_eq!((status.as_str(), kind.as_str()), ("sealed", "blocked"));
    let original: String = db.query_row("SELECT content_text FROM message_parts WHERE id='delivery-user-part'", [], |r|r.get(0)).unwrap();
    assert_eq!(original, text);
}

#[test]
fn pre_execution_rejection_is_durable_and_cannot_be_started_later() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("rejected-delivery.db");
    let mut store = NolaStore::open(&database).unwrap();
    let begun = begin_assistant_turn(&mut store, BeginAssistantTurn {
        expected_configuration_revision: None,
        expected_parent_message_id: None,
        conversation_id: "delivery-conversation".into(), conversation_title: "Rejected".into(),
        user_message_id: "delivery-user".into(), user_part_id: "delivery-user-part".into(),
        user_text: "Execute once".into(), user_content_json: None,
        assistant_message_id: "delivery-assistant".into(), context_checkpoint_id: None,
        core_config: core_config(), submission: user_input(), created_at_ms: 1_000,
    }).unwrap();
    let request = claim_committed_host_requests(&mut store, ClaimCommittedHostRequests {
        run_id: begun.begin.run_id.clone(), worker_generation: "generation-1".into(),
        lease_owner: "runtime-1".into(), claimed_at_ms: 1_100,
        lease_expires_at_ms: 2_000, limit: 1,
    }).unwrap().requests.into_iter().next().unwrap();
    let lease = request.delivery.clone().unwrap();
    reduce_and_accept_host_response(&mut store, ReduceAndAcceptHostResponse {
        conversation_id: "delivery-conversation".into(), run_id: begun.begin.run_id.clone(),
        invocation_config_digest: begun.invocation_config_digest.clone(),
        response: response(&request, 1, HostResponse::ExecutionRejected {
            code: "memory_authority_unavailable".into(), message: "Host authority revoked".into(),
            retryable: false,
        }), received_at_ms: 1_200,
    }).unwrap();
    let settled = store.effect_delivery_attempt(&lease.attempt_id).unwrap().unwrap();
    assert_eq!(settled.state, EffectDeliveryAttemptState::Settled);
    assert_eq!(settled.settlement, Some(EffectDeliverySettlement::RejectedBeforeExecution));
    let attempt = store.model_attempt(request.attempt_id.as_deref().unwrap()).unwrap().unwrap();
    assert_eq!(attempt.failure.as_ref().unwrap().code, "memory_authority_unavailable");
    drop(store);
    let mut reopened = NolaStore::open(&database).unwrap();
    assert_eq!(reopened.effect_delivery_attempt(&lease.attempt_id).unwrap().unwrap(), settled);
    assert!(reduce_and_accept_host_response(&mut reopened, ReduceAndAcceptHostResponse {
        conversation_id: "delivery-conversation".into(), run_id: begun.begin.run_id,
        invocation_config_digest: begun.invocation_config_digest,
        response: response(&request, 2, HostResponse::ExecutionStarted), received_at_ms: 1_300,
    }).is_err());
    assert_eq!(reopened.effect_delivery_attempt(&lease.attempt_id).unwrap().unwrap(), settled);
}

#[test]
fn execution_start_is_committed_before_progress_and_blocks_a_second_claim() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("effect-delivery-application.db");
    let mut store = NolaStore::open(&database).unwrap();
    let begun = begin_assistant_turn(
        &mut store,
        BeginAssistantTurn {
            expected_configuration_revision: None,
            expected_parent_message_id: None,
            conversation_id: "delivery-conversation".into(),
            conversation_title: "Delivery".into(),
            user_message_id: "delivery-user".into(),
            user_part_id: "delivery-user-part".into(),
            user_text: "执行一次模型请求".into(),
            user_content_json: None,
            assistant_message_id: "delivery-assistant".into(),
            context_checkpoint_id: None,
            core_config: core_config(),
            submission: user_input(),
            created_at_ms: 1_000,
        },
    )
    .unwrap();

    // Even a damaged/migrated database with no active or historical attempt
    // must not make a schema-20 pending effect accept an unfenced response.
    let raw_request = load_committed_host_requests(&store, &begun.begin.run_id)
        .unwrap()
        .requests
        .into_iter()
        .next()
        .unwrap();
    rusqlite::Connection::open(&database)
        .unwrap()
        .execute(
            "DELETE FROM effect_delivery_attempts WHERE effect_id=?1",
            [&raw_request.effect_id.0],
        )
        .unwrap();
    let error = reduce_and_accept_host_response(
        &mut store,
        ReduceAndAcceptHostResponse {
            conversation_id: "delivery-conversation".into(),
            run_id: begun.begin.run_id.clone(),
            invocation_config_digest: begun.invocation_config_digest.clone(),
            response: response(&raw_request, 1, HostResponse::ExecutionStarted),
            received_at_ms: 1_050,
        },
    )
    .unwrap_err();
    assert!(error.contains("required effect-delivery lease"), "{error}");

    let claimed = claim_committed_host_requests(
        &mut store,
        ClaimCommittedHostRequests {
            run_id: begun.begin.run_id.clone(),
            worker_generation: "generation-1".into(),
            lease_owner: "runtime-1".into(),
            claimed_at_ms: 1_100,
            lease_expires_at_ms: 2_000,
            limit: 1,
        },
    )
    .unwrap();
    assert_eq!(claimed.requests.len(), 1);
    let request = claimed.requests.into_iter().next().unwrap();
    let lease = request.delivery.clone().expect("Application-owned lease");

    reduce_and_accept_host_response(
        &mut store,
        ReduceAndAcceptHostResponse {
            conversation_id: "delivery-conversation".into(),
            run_id: begun.begin.run_id.clone(),
            invocation_config_digest: begun.invocation_config_digest.clone(),
            response: response(&request, 1, HostResponse::ExecutionStarted),
            received_at_ms: 1_200,
        },
    )
    .unwrap();
    let started = store
        .effect_delivery_attempt(&lease.attempt_id)
        .unwrap()
        .unwrap();
    assert_eq!(started.state, EffectDeliveryAttemptState::ExecutionStarted);

    let second = claim_committed_host_requests(
        &mut store,
        ClaimCommittedHostRequests {
            run_id: begun.begin.run_id.clone(),
            worker_generation: "generation-2".into(),
            lease_owner: "runtime-2".into(),
            claimed_at_ms: 1_300,
            lease_expires_at_ms: 2_100,
            limit: 1,
        },
    )
    .unwrap();
    assert!(second.requests.is_empty());

    let mut stale = response(
        &request,
        2,
        HostResponse::ToolProgress {
            title: "stale".into(),
            detail: None,
            metadata: serde_json::Value::Null,
        },
    );
    stale.delivery.as_mut().unwrap().lease_token = "wrong-token".into();
    let error = reduce_and_accept_host_response(
        &mut store,
        ReduceAndAcceptHostResponse {
            conversation_id: "delivery-conversation".into(),
            run_id: begun.begin.run_id.clone(),
            invocation_config_digest: begun.invocation_config_digest.clone(),
            response: stale,
            received_at_ms: 1_400,
        },
    )
    .unwrap_err();
    assert!(error.contains("stale or foreign"), "{error}");
    assert_eq!(
        store
            .effect_delivery_attempt(&lease.attempt_id)
            .unwrap()
            .unwrap(),
        started
    );

    // The rejected sequence was not consumed; the legitimate owner can use it.
    reduce_and_accept_host_response(
        &mut store,
        ReduceAndAcceptHostResponse {
            conversation_id: "delivery-conversation".into(),
            run_id: begun.begin.run_id.clone(),
            invocation_config_digest: begun.invocation_config_digest.clone(),
            response: response(
                &request,
                2,
                HostResponse::ToolProgress {
                    title: "running".into(),
                    detail: Some("one owner".into()),
                    metadata: serde_json::Value::Null,
                },
            ),
            received_at_ms: 1_500,
        },
    )
    .unwrap();

    reduce_and_accept_host_response(
        &mut store,
        ReduceAndAcceptHostResponse {
            conversation_id: "delivery-conversation".into(),
            run_id: request.turn_id.clone().unwrap(),
            invocation_config_digest: begun.invocation_config_digest.clone(),
            response: response(
                &request,
                3,
                HostResponse::Failed {
                    error: HostError {
                        code: "invalid_model_request".into(),
                        message: "The provider rejected the model request.".into(),
                        retryable: false,
                        http_status: Some(400),
                    },
                },
            ),
            received_at_ms: 1_600,
        },
    )
    .unwrap();
    let model_attempt = store.model_attempt(request.attempt_id.as_deref().unwrap())
        .unwrap().unwrap();
    assert_eq!(model_attempt.failure.unwrap().http_status, Some(400));
    let settled = store
        .effect_delivery_attempt(&lease.attempt_id)
        .unwrap()
        .unwrap();
    assert_eq!(settled.state, EffectDeliveryAttemptState::Settled);
    assert_eq!(settled.settlement, Some(EffectDeliverySettlement::Failed));
}

#[test]
fn heartbeat_fences_identity_expiry_and_subsequent_host_responses() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("effect-delivery-heartbeat.db");
    let mut store = NolaStore::open(&database).unwrap();
    let begun = begin_assistant_turn(
        &mut store,
        BeginAssistantTurn {
            expected_configuration_revision: None,
            expected_parent_message_id: None,
            conversation_id: "delivery-conversation".into(),
            conversation_title: "Delivery heartbeat".into(),
            user_message_id: "delivery-heartbeat-user".into(),
            user_part_id: "delivery-heartbeat-user-part".into(),
            user_text: "续租一次模型请求".into(),
            user_content_json: None,
            assistant_message_id: "delivery-heartbeat-assistant".into(),
            context_checkpoint_id: None,
            core_config: core_config(),
            submission: user_input(),
            created_at_ms: 1_000,
        },
    )
    .unwrap();
    let request = claim_committed_host_requests(
        &mut store,
        ClaimCommittedHostRequests {
            run_id: begun.begin.run_id.clone(),
            worker_generation: "heartbeat-generation".into(),
            lease_owner: "heartbeat-runtime".into(),
            claimed_at_ms: 1_100,
            lease_expires_at_ms: 2_000,
            limit: 1,
        },
    )
    .unwrap()
    .requests
    .into_iter()
    .next()
    .unwrap();
    let old_lease = request.delivery.clone().unwrap();

    let mut missing_attempt = heartbeat(&begun.begin.run_id, &old_lease, 1_200, 3_000);
    missing_attempt.attempt_id = "missing-attempt".into();
    assert!(heartbeat_effect_delivery(&mut store, missing_attempt)
        .unwrap_err()
        .contains("unknown attempt"));
    let mut missing = heartbeat(&begun.begin.run_id, &old_lease, 1_200, 3_000);
    missing.lease_token.clear();
    assert!(heartbeat_effect_delivery(&mut store, missing)
        .unwrap_err()
        .contains("invalid"));
    let mut foreign = heartbeat(&begun.begin.run_id, &old_lease, 1_200, 3_000);
    foreign.lease_owner = "another-runtime".into();
    assert!(heartbeat_effect_delivery(&mut store, foreign)
        .unwrap_err()
        .contains("stale or foreign"));
    let mut stale_expiry = heartbeat(&begun.begin.run_id, &old_lease, 1_200, 3_000);
    stale_expiry.current_lease_expires_at_ms = 1_999;
    assert!(heartbeat_effect_delivery(&mut store, stale_expiry)
        .unwrap_err()
        .contains("stale or foreign"));

    let renewed = heartbeat_effect_delivery(
        &mut store,
        heartbeat(&begun.begin.run_id, &old_lease, 1_200, 3_000),
    )
    .unwrap();
    assert_eq!(renewed.lease_expires_at_ms, 3_000);
    let persisted = store
        .effect_delivery_attempt(&renewed.attempt_id)
        .unwrap()
        .unwrap();
    assert_eq!(persisted.lease_expires_at_ms, Some(3_000));
    assert_eq!(persisted.heartbeat_at_ms, Some(1_200));

    let old_response = response(&request, 1, HostResponse::ExecutionStarted);
    let error = reduce_and_accept_host_response(
        &mut store,
        ReduceAndAcceptHostResponse {
            conversation_id: "delivery-conversation".into(),
            run_id: begun.begin.run_id.clone(),
            invocation_config_digest: begun.invocation_config_digest.clone(),
            response: old_response,
            received_at_ms: 1_300,
        },
    )
    .unwrap_err();
    assert!(error.contains("stale or foreign"), "{error}");

    let mut renewed_request = request.clone();
    renewed_request.delivery = Some(renewed.clone());
    reduce_and_accept_host_response(
        &mut store,
        ReduceAndAcceptHostResponse {
            conversation_id: "delivery-conversation".into(),
            run_id: begun.begin.run_id.clone(),
            invocation_config_digest: begun.invocation_config_digest.clone(),
            response: response(&renewed_request, 1, HostResponse::ExecutionStarted),
            received_at_ms: 1_300,
        },
    )
    .unwrap();

    assert!(heartbeat_effect_delivery(
        &mut store,
        heartbeat(&begun.begin.run_id, &renewed, 3_000, 4_000),
    )
    .unwrap_err()
    .contains("expired"));

    reduce_and_accept_host_response(
        &mut store,
        ReduceAndAcceptHostResponse {
            conversation_id: "delivery-conversation".into(),
            run_id: begun.begin.run_id.clone(),
            invocation_config_digest: begun.invocation_config_digest.clone(),
            response: response(
                &renewed_request,
                2,
                HostResponse::Failed {
                    error: HostError {
                        code: "fixture".into(),
                        message: "done".into(),
                        retryable: false,
                        http_status: None,
                    },
                },
            ),
            received_at_ms: 1_400,
        },
    )
    .unwrap();
    assert!(heartbeat_effect_delivery(
        &mut store,
        heartbeat(&begun.begin.run_id, &renewed, 1_500, 4_000),
    )
    .unwrap_err()
    .contains("not active"));
}

#[test]
fn heartbeat_and_its_command_receipt_commit_or_roll_back_together() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory
        .path()
        .join("effect-delivery-heartbeat-receipt.db");
    let mut store = NolaStore::open(&database).unwrap();
    let begun = begin_assistant_turn(
        &mut store,
        BeginAssistantTurn {
            expected_configuration_revision: None,
            expected_parent_message_id: None,
            conversation_id: "delivery-conversation".into(),
            conversation_title: "Delivery heartbeat receipt".into(),
            user_message_id: "delivery-heartbeat-receipt-user".into(),
            user_part_id: "delivery-heartbeat-receipt-user-part".into(),
            user_text: "事务续租一次模型请求".into(),
            user_content_json: None,
            assistant_message_id: "delivery-heartbeat-receipt-assistant".into(),
            context_checkpoint_id: None,
            core_config: core_config(),
            submission: user_input(),
            created_at_ms: 1_000,
        },
    )
    .unwrap();
    let request = claim_committed_host_requests(
        &mut store,
        ClaimCommittedHostRequests {
            run_id: begun.begin.run_id.clone(),
            worker_generation: "receipt-generation".into(),
            lease_owner: "receipt-runtime".into(),
            claimed_at_ms: 1_100,
            lease_expires_at_ms: 2_000,
            limit: 1,
        },
    )
    .unwrap()
    .requests
    .into_iter()
    .next()
    .unwrap();
    let lease = request.delivery.clone().unwrap();
    let before = store
        .effect_delivery_attempt(&lease.attempt_id)
        .unwrap()
        .unwrap();
    drop(store);

    let command = ApplicationCommand::HeartbeatEffectDelivery {
        input: heartbeat(&begun.begin.run_id, &lease, 1_200, 3_000),
    };
    let command_json = serde_json::to_string(&command).unwrap();
    let receipt = || nola_application::ExecuteReceivedCommand {
        client_id: "heartbeat-client".into(),
        request_id: "heartbeat-request".into(),
        command_type: "heartbeat_effect_delivery".into(),
        command_json: command_json.clone(),
        expected_revision: None,
        received_at: "1200".into(),
    };
    let inspect = rusqlite::Connection::open(&database).unwrap();
    inspect.execute_batch("CREATE TRIGGER fail_heartbeat_receipt BEFORE UPDATE ON command_receipts WHEN NEW.status='committed' BEGIN SELECT CASE WHEN EXISTS(SELECT 1 FROM effect_delivery_attempts WHERE id IS NOT NULL AND state='leased' AND json_extract(record_json,'$.lease_expires_at_ms')=3000) THEN RAISE(ABORT,'heartbeat receipt fault') ELSE RAISE(ABORT,'heartbeat missing before receipt') END; END").unwrap();
    let mut runtime = support::open_runtime(&database).unwrap();
    let error = runtime
        .execute_received_command(receipt(), serde_json::from_str(&command_json).unwrap())
        .unwrap_err();
    assert!(error.message.contains("heartbeat receipt fault"), "{error}");
    assert_eq!(
        NolaStore::open(&database)
            .unwrap()
            .effect_delivery_attempt(&lease.attempt_id)
            .unwrap()
            .unwrap(),
        before
    );

    inspect
        .execute_batch("DROP TRIGGER fail_heartbeat_receipt")
        .unwrap();
    let committed = runtime
        .execute_received_command(receipt(), serde_json::from_str(&command_json).unwrap())
        .unwrap();
    let result = match committed.outcome {
        nola_application::ApplicationCommandReceiptOutcome::Committed { result, .. } => result,
        outcome => panic!("unexpected heartbeat receipt outcome: {outcome:?}"),
    };
    let acknowledged: EffectDeliveryLease = serde_json::from_value(result).unwrap();
    assert_eq!(acknowledged.lease_expires_at_ms, 3_000);
    let replayed = runtime
        .execute_received_command(receipt(), serde_json::from_str(&command_json).unwrap())
        .unwrap();
    assert!(replayed.replayed);
    assert_eq!(
        NolaStore::open(&database)
            .unwrap()
            .effect_delivery_attempt(&lease.attempt_id)
            .unwrap()
            .unwrap()
            .lease_expires_at_ms,
        Some(3_000)
    );
}

#[test]
fn prepared_model_attempt_survives_rebind_without_sampling_mutation() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("prepared-rebind.db");
    let mut store = NolaStore::open(&database).unwrap();
    let begun = begin_assistant_turn(&mut store, BeginAssistantTurn {
        expected_configuration_revision: None,
        expected_parent_message_id: None,
        conversation_id: "delivery-conversation".into(), conversation_title: "Prepared recovery".into(),
        user_message_id: "prepared-user".into(), user_part_id: "prepared-user-part".into(),
        user_text: "Continue after the answer".into(), user_content_json: None,
        assistant_message_id: "prepared-assistant".into(), context_checkpoint_id: None,
        core_config: core_config(), submission: user_input(), created_at_ms: 1000,
    }).unwrap();
    let original = load_committed_host_requests(&store, &begun.begin.run_id).unwrap()
        .requests.into_iter().next().unwrap();
    assert!(original.attempt_id.is_some());
    assert!(!original.attempt_started);
    drop(store);
    let mut store = NolaStore::open(&database).unwrap();
    reduce_and_commit_active_submission(&mut store, ReduceAndCommitActiveSubmission {
        conversation_id: "delivery-conversation".into(), run_id: begun.begin.run_id.clone(),
        transition_id: "prepared-rebind".into(), invocation_config_digest: begun.invocation_config_digest,
        submission: AgentSubmission::ResumeHostEffect { effect_id: original.effect_id.0.clone() },
        committed_at_ms: 2000,
    }).unwrap();
    let rebound = load_committed_host_requests(&store, &begun.begin.run_id).unwrap()
        .requests.into_iter().next().unwrap();
    assert_ne!(rebound.request_id, original.request_id);
    assert_eq!(rebound.effect_id, original.effect_id);
    assert_eq!(rebound.attempt_id, original.attempt_id);
    assert!(!rebound.attempt_started);
    let claimed = claim_committed_host_requests(&mut store, ClaimCommittedHostRequests {
        run_id: begun.begin.run_id, worker_generation: "rebound-worker".into(),
        lease_owner: "rebound-owner".into(), claimed_at_ms: 2100, lease_expires_at_ms: 4000, limit: 1,
    }).unwrap();
    assert_eq!(claimed.requests.len(), 1);
    assert_eq!(claimed.requests[0].attempt_id, original.attempt_id);
}

#[test]
fn cold_application_recovery_records_one_resume_plan_from_the_checkpoint_cursor() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("effect-delivery-recovery.db");
    let mut store = NolaStore::open(&database).unwrap();
    let binding = recovery_host_binding("delivery-model");
    let mut frozen_config = core_config();
    frozen_config.turn_identity.host_invocation = Some(binding.clone());
    let begun = begin_assistant_turn(
        &mut store,
        BeginAssistantTurn {
            expected_configuration_revision: None,
            expected_parent_message_id: None,
            conversation_id: "delivery-conversation".into(),
            conversation_title: "Delivery recovery".into(),
            user_message_id: "delivery-recovery-user".into(),
            user_part_id: "delivery-recovery-user-part".into(),
            user_text: "恢复一次模型请求".into(),
            user_content_json: None,
            assistant_message_id: "delivery-recovery-assistant".into(),
            context_checkpoint_id: None,
            core_config: frozen_config,
            submission: user_input(),
            created_at_ms: 1_000,
        },
    )
    .unwrap();
    let claimed = claim_committed_host_requests(
        &mut store,
        ClaimCommittedHostRequests {
            run_id: begun.begin.run_id.clone(),
            worker_generation: "recovery-generation".into(),
            lease_owner: "recovery-runtime".into(),
            claimed_at_ms: 1_100,
            lease_expires_at_ms: 2_000,
            limit: 1,
        },
    )
    .unwrap();
    let request = claimed.requests.into_iter().next().unwrap();
    let lease = request.delivery.clone().unwrap();
    reduce_and_accept_host_response(
        &mut store,
        ReduceAndAcceptHostResponse {
            conversation_id: "delivery-conversation".into(),
            run_id: begun.begin.run_id.clone(),
            invocation_config_digest: begun.invocation_config_digest.clone(),
            response: response(&request, 1, HostResponse::ExecutionStarted),
            received_at_ms: 1_200,
        },
    )
    .unwrap();
    drop(store);

    let recovery = || ApplicationCommand::RecoverOrphanedRuns {
        conversation_id: Some("delivery-conversation".into()),
        reason: "runtime restarted".into(),
        interrupted_at: "3000".into(),
        interrupted_at_ms: 3_000,
    };
    let mut runtime = support::open_runtime(&database).unwrap();
    let ownership = ApplicationCommand::LoadRunDeliveryOwnership { run_id: begun.begin.run_id.clone() };
    assert!(ownership.is_read_only_query());
    let observed: Vec<EffectDeliveryLease> = serde_json::from_value(runtime.execute(ownership).unwrap()).unwrap();
    assert_eq!(observed, vec![lease.clone()]);
    let mut stale = observed.clone();
    stale[0].lease_token = "stale-owner-token".into();
    assert!(runtime.execute(ApplicationCommand::RecoverObservedOrphanedRun {
        host_invocation: binding.clone(), observed_session_lease: None,
        expected_run_id: begun.begin.run_id.clone(),
        conversation_id: "delivery-conversation".into(), observed_deliveries: stale,
        interrupted_at: "1500".into(), interrupted_at_ms: 1_500,
    }).unwrap_err().contains("ownership changed"));
    assert_eq!(runtime.execute(ApplicationCommand::RecoverObservedOrphanedRun {
        host_invocation: binding, observed_session_lease: None,
        expected_run_id: begun.begin.run_id.clone(),
        conversation_id: "delivery-conversation".into(), observed_deliveries: observed,
        interrupted_at: "1500".into(), interrupted_at_ms: 1_500,
    }).unwrap(), serde_json::json!(1));
    drop(runtime);

    let reopened = NolaStore::open(&database).unwrap();
    let interrupted = reopened
        .effect_delivery_attempt(&lease.attempt_id)
        .unwrap()
        .unwrap();
    assert_eq!(interrupted.state, EffectDeliveryAttemptState::Interrupted);
    assert_eq!(
        interrupted.recovery_plan,
        Some(nola_domain::EffectDeliveryRecoveryPlan::Resume)
    );
    assert!(reopened
        .pending_effect_delivery_candidates(&begun.begin.run_id, None, 1)
        .unwrap()
        .is_empty());
    drop(reopened);

    let mut runtime = support::open_runtime(&database).unwrap();
    assert_eq!(runtime.execute(recovery()).unwrap(), serde_json::json!(0));
    drop(runtime);

    let mut store = NolaStore::open(&database).unwrap();
    let rebound = reduce_and_commit_active_submission(
        &mut store,
        ReduceAndCommitActiveSubmission {
            conversation_id: "delivery-conversation".into(),
            run_id: begun.begin.run_id.clone(),
            transition_id: "resume-effect-with-fresh-request".into(),
            invocation_config_digest: begun.invocation_config_digest.clone(),
            submission: AgentSubmission::ResumeHostEffect {
                effect_id: request.effect_id.0.clone(),
            },
            committed_at_ms: 4_000,
        },
    )
    .unwrap();
    assert!(rebound.commit.applied);
    let rebound_requests = nola_application::load_committed_host_requests(&store, &begun.begin.run_id).unwrap().requests;
    assert_eq!(rebound_requests.len(), 1);
    assert_ne!(rebound_requests[0].request_id, request.request_id);
    let rebound_request_id = rebound_requests[0].request_id.clone();
    let resumed = claim_committed_host_requests(
        &mut store,
        ClaimCommittedHostRequests {
            run_id: begun.begin.run_id.clone(),
            worker_generation: "recovery-generation-2".into(),
            lease_owner: "recovery-runtime-2".into(),
            claimed_at_ms: 4_100,
            lease_expires_at_ms: 5_000,
            limit: 1,
        },
    )
    .unwrap();
    assert_eq!(resumed.requests.len(), 1);
    let successor_lease = resumed.requests[0].delivery.as_ref().unwrap();
    assert_ne!(successor_lease.attempt_id, lease.attempt_id);
    let successor = store
        .effect_delivery_attempt(&successor_lease.attempt_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        successor.predecessor_id.as_deref(),
        Some(lease.attempt_id.as_str())
    );
    assert_eq!(successor.request_id, rebound_request_id);
    assert_eq!(successor.attempt_number, 2);
    assert_eq!(successor.state, EffectDeliveryAttemptState::Leased);
}

#[test]
fn retry_is_claimed_by_the_next_generation_but_outcome_unknown_is_not() {
    for (suffix, replay_safety, start, expected_plan, should_reclaim) in [
        (
            "retry",
            HostReplaySafety::Idempotent,
            false,
            Some(nola_domain::EffectDeliveryRecoveryPlan::Retry),
            true,
        ),
        (
            "unknown",
            HostReplaySafety::RequiresRecoveryDecision,
            true,
            None,
            false,
        ),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let database = directory
            .path()
            .join(format!("effect-delivery-{suffix}.db"));
        let mut config = core_config();
        config
            .host_catalog
            .as_mut()
            .unwrap()
            .capabilities
            .get_mut("model.stream")
            .unwrap()
            .replay_safety = replay_safety;
        let mut store = NolaStore::open(&database).unwrap();
        let begun = begin_assistant_turn(
            &mut store,
            BeginAssistantTurn {
                expected_configuration_revision: None,
                expected_parent_message_id: None,
                conversation_id: "delivery-conversation".into(),
                conversation_title: "Delivery recovery policy".into(),
                user_message_id: format!("delivery-{suffix}-user"),
                user_part_id: format!("delivery-{suffix}-part"),
                user_text: "恢复模型请求".into(),
                user_content_json: None,
                assistant_message_id: format!("delivery-{suffix}-assistant"),
                context_checkpoint_id: None,
                core_config: config,
                submission: user_input(),
                created_at_ms: 1_000,
            },
        )
        .unwrap();
        let claimed = claim_committed_host_requests(
            &mut store,
            ClaimCommittedHostRequests {
                run_id: begun.begin.run_id.clone(),
                worker_generation: "generation-1".into(),
                lease_owner: "runtime-1".into(),
                claimed_at_ms: 1_100,
                lease_expires_at_ms: 2_000,
                limit: 1,
            },
        )
        .unwrap();
        let request = claimed.requests.into_iter().next().unwrap();
        let predecessor_id = request.delivery.as_ref().unwrap().attempt_id.clone();
        if start {
            reduce_and_accept_host_response(
                &mut store,
                ReduceAndAcceptHostResponse {
                    conversation_id: "delivery-conversation".into(),
                    run_id: begun.begin.run_id.clone(),
                    invocation_config_digest: begun.invocation_config_digest.clone(),
                    response: response(&request, 1, HostResponse::ExecutionStarted),
                    received_at_ms: 1_200,
                },
            )
            .unwrap();
        }
        drop(store);

        let mut runtime = support::open_runtime(&database).unwrap();
        assert_eq!(
            runtime
                .execute(ApplicationCommand::RecoverOrphanedRuns {
                    conversation_id: Some("delivery-conversation".into()),
                    reason: "runtime restarted".into(),
                    interrupted_at: "3000".into(),
                    interrupted_at_ms: 3_000,
                })
                .unwrap(),
            serde_json::json!(1)
        );
        drop(runtime);

        let mut store = NolaStore::open(&database).unwrap();
        let predecessor = store
            .effect_delivery_attempt(&predecessor_id)
            .unwrap()
            .unwrap();
        assert_eq!(predecessor.recovery_plan, expected_plan);
        assert_eq!(
            predecessor.state,
            if should_reclaim {
                EffectDeliveryAttemptState::Interrupted
            } else {
                EffectDeliveryAttemptState::OutcomeUnknown
            }
        );
        let next = claim_committed_host_requests(
            &mut store,
            ClaimCommittedHostRequests {
                run_id: begun.begin.run_id.clone(),
                worker_generation: "generation-2".into(),
                lease_owner: "runtime-2".into(),
                claimed_at_ms: 3_100,
                lease_expires_at_ms: 4_000,
                limit: 1,
            },
        )
        .unwrap();
        assert_eq!(next.requests.len(), usize::from(should_reclaim));
        if should_reclaim {
            let successor_id = &next.requests[0].delivery.as_ref().unwrap().attempt_id;
            let successor = store
                .effect_delivery_attempt(successor_id)
                .unwrap()
                .unwrap();
            assert_eq!(
                successor.predecessor_id.as_deref(),
                Some(predecessor_id.as_str())
            );
            assert_eq!(successor.worker_generation.as_deref(), Some("generation-2"));
        }
    }
}

#[test]
fn unknown_effect_user_decisions_commit_atomically_and_replay_without_reexecution() {
    for preserve_unknown in [false, true] {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("unknown-effect-manual-retry.db");
    let mut config = core_config();
    config.turn_identity.host_invocation = Some(recovery_host_binding("delivery-model"));
    config
        .host_catalog
        .as_mut()
        .unwrap()
        .capabilities
        .get_mut("model.stream")
        .unwrap()
        .replay_safety = HostReplaySafety::RequiresRecoveryDecision;
    let mut store = NolaStore::open(&database).unwrap();
    let begun = begin_assistant_turn(
        &mut store,
        BeginAssistantTurn {
            expected_configuration_revision: None,
            expected_parent_message_id: None,
            conversation_id: "delivery-conversation".into(),
            conversation_title: "Manual retry evidence".into(),
            user_message_id: "manual-retry-user".into(),
            user_part_id: "manual-retry-user-part".into(),
            user_text: "execute once".into(),
            user_content_json: None,
            assistant_message_id: "manual-retry-assistant".into(),
            context_checkpoint_id: None,
            core_config: config,
            submission: user_input(),
            created_at_ms: 1_000,
        },
    )
    .unwrap();
    let request = claim_committed_host_requests(
        &mut store,
        ClaimCommittedHostRequests {
            run_id: begun.begin.run_id.clone(),
            worker_generation: "manual-generation-1".into(),
            lease_owner: "manual-runtime-1".into(),
            claimed_at_ms: 1_100,
            lease_expires_at_ms: 2_000,
            limit: 1,
        },
    )
    .unwrap()
    .requests
    .remove(0);
    reduce_and_accept_host_response(
        &mut store,
        ReduceAndAcceptHostResponse {
            conversation_id: "delivery-conversation".into(),
            run_id: begun.begin.run_id.clone(),
            invocation_config_digest: begun.invocation_config_digest.clone(),
            response: response(&request, 1, HostResponse::ExecutionStarted),
            received_at_ms: 1_200,
        },
    )
    .unwrap();
    let lease = request.delivery.as_ref().unwrap().clone();
    drop(store);
    let mut runtime = support::open_runtime(&database).unwrap();
    // A live, started model request is independently pending, not an unknown
    // outcome. Cold clients must be allowed to enter exact Run recovery.
    assert!(runtime.execute(ApplicationCommand::InspectUnknownEffectRecovery {
        conversation_id: "delivery-conversation".into(),
        expected_run_id: begun.begin.run_id.clone(),
        assistant_message_id: "manual-retry-assistant".into(),
    }).unwrap().is_null());
    runtime
        .execute(ApplicationCommand::RecoverOrphanedRuns {
            conversation_id: Some("delivery-conversation".into()),
            reason: "runtime disappeared after execution".into(),
            interrupted_at: "3000".into(),
            interrupted_at_ms: 3_000,
        })
        .unwrap();
    drop(runtime);
    let inspect = NolaStore::open(&database).unwrap();
    let unknown = inspect
        .effect_delivery_attempt(&lease.attempt_id)
        .unwrap()
        .unwrap();
    drop(inspect);
    let mut runtime = support::open_runtime(&database).unwrap();
    // SQL column identity and the serialized attempt are one contract. A
    // damaged record must not manufacture fencing for a privileged decision.
    let db = rusqlite::Connection::open(&database).unwrap();
    let owner: String = db.query_row("SELECT assistant_message_id FROM runs WHERE id=?1",
        [&begun.begin.run_id], |row| row.get(0)).unwrap();
    let inspect_command = || ApplicationCommand::InspectUnknownEffectRecovery {
        conversation_id: "delivery-conversation".into(), expected_run_id: begun.begin.run_id.clone(),
        assistant_message_id: owner.clone(),
    };
    let facts: Option<nola_protocol::UnknownEffectRecoveryInspection> =
        serde_json::from_value(runtime.execute(inspect_command()).unwrap()).unwrap();
    assert_eq!(facts.unwrap().targets[0].attempt_id, unknown.id);
    let original = serde_json::to_string(&unknown).unwrap();
    for corrupt in [
        { let mut value = serde_json::to_value(&unknown).unwrap(); value["execution_started_at_ms"] = serde_json::Value::Null; value },
        { let mut value = serde_json::to_value(&unknown).unwrap(); value["revision"] = serde_json::json!(unknown.revision + 1); value },
        { let mut value = serde_json::to_value(&unknown).unwrap(); value["lease_token"] = serde_json::json!("forged-token"); value },
    ] {
        db.execute("UPDATE effect_delivery_attempts SET record_json=?2 WHERE id=?1",
            rusqlite::params![unknown.id, corrupt.to_string()]).unwrap();
        assert!(runtime.execute(inspect_command()).is_err());
        db.execute("UPDATE effect_delivery_attempts SET record_json=?2 WHERE id=?1",
            rusqlite::params![unknown.id, original]).unwrap();
    }
    let command = ApplicationCommand::ResolveUnknownEffectRecovery {
        input: ResolveUnknownEffectRecovery {
            conversation_id: "delivery-conversation".into(),
            run_id: begun.begin.run_id.clone(),
            invocation_config_digest: begun.invocation_config_digest.clone(),
            resolution_id: "manual-resolution-decision".into(),
            targets: vec![nola_domain::unknown_effect_recovery::UnknownEffectRecoveryTarget {
            effect_id: request.effect_id.0.clone(),
            request_id: request.request_id.clone(),
            attempt_id: lease.attempt_id.clone(),
            worker_generation: lease.worker_generation.clone(),
            lease_owner: lease.lease_owner.clone(),
            lease_token: lease.lease_token.clone(),
            expected_attempt_revision: unknown.revision,
            }],
            choice: if preserve_unknown {
                UnknownEffectRecoveryChoice::KeepUnknownBlocked { reason: "Keep the uncertain operation unresolved; do not retry it".into() }
            } else {
                UnknownEffectRecoveryChoice::AuthorizeRiskRetry { reason: "operator explicitly authorizes retry despite uncertain external outcome".into() }
            },
            resolved_at_ms: 3_100,
        },
    };
    let command_json = serde_json::to_string(&command).unwrap();
    let received = || ExecuteReceivedCommand {
        client_id: "manual-resolution-client".into(),
        request_id: "manual-resolution-command".into(),
        command_type: "resolve_unknown_effect_recovery".into(),
        command_json: command_json.clone(),
        expected_revision: None,
        received_at: "2026-09-09T00:00:00Z".into(),
    };
    let fault = rusqlite::Connection::open(&database).unwrap();
    fault
        .execute_batch(
            "CREATE TRIGGER fail_unknown_retry_receipt BEFORE UPDATE ON command_receipts
         WHEN NEW.status='committed'
         BEGIN SELECT RAISE(ABORT,'unknown retry receipt fault'); END;",
        )
        .unwrap();
    assert!(runtime
        .execute_received_command(received(), serde_json::from_str(&command_json).unwrap(),)
        .is_err());
    let inspect = NolaStore::open(&database).unwrap();
    assert_eq!(
        inspect
            .effect_delivery_attempt(&lease.attempt_id)
            .unwrap()
            .unwrap()
            .state,
        EffectDeliveryAttemptState::OutcomeUnknown
    );
    drop(inspect);
    fault
        .execute_batch("DROP TRIGGER fail_unknown_retry_receipt")
        .unwrap();
    let committed = runtime
        .execute_received_command(received(), serde_json::from_str(&command_json).unwrap())
        .unwrap();
    let replay = runtime
        .execute_received_command(received(), serde_json::from_str(&command_json).unwrap())
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.outcome, committed.outcome);
    drop(runtime);

    let mut store = NolaStore::open(&database).unwrap();
    let authorized = store
        .effect_delivery_attempt(&lease.attempt_id)
        .unwrap()
        .unwrap();
    if preserve_unknown {
        assert_eq!(authorized.state, EffectDeliveryAttemptState::OutcomeUnknown);
        let result: String = fault.query_row("SELECT result_json FROM run_results WHERE run_id=?1",[&begun.begin.run_id],|row|row.get(0)).unwrap();
        assert_eq!(serde_json::from_str::<serde_json::Value>(&result).unwrap()["value"]["kind"],"blocked");
        let status:String=fault.query_row("SELECT status FROM runs WHERE id=?1",[&begun.begin.run_id],|row|row.get(0)).unwrap();
        assert_eq!(status,"sealed");
        continue;
    }
    assert_eq!(authorized.state, EffectDeliveryAttemptState::Interrupted);
    let first = claim_committed_host_requests(
        &mut store,
        ClaimCommittedHostRequests {
            run_id: begun.begin.run_id.clone(),
            worker_generation: "manual-generation-2".into(),
            lease_owner: "manual-runtime-2".into(),
            claimed_at_ms: 3_200,
            lease_expires_at_ms: 4_000,
            limit: 1,
        },
    )
    .unwrap();
    assert_eq!(first.requests.len(), 1);
    assert_ne!(
        first.requests[0].delivery.as_ref().unwrap().attempt_id,
        lease.attempt_id
    );
    assert!(heartbeat_effect_delivery(
        &mut store,
        heartbeat(&begun.begin.run_id, &lease, 3_300, 4_100,)
    )
    .is_err());
    assert!(claim_committed_host_requests(
        &mut store,
        ClaimCommittedHostRequests {
            run_id: begun.begin.run_id,
            worker_generation: "manual-generation-3".into(),
            lease_owner: "manual-runtime-3".into(),
            claimed_at_ms: 3_300,
            lease_expires_at_ms: 4_100,
            limit: 1,
        }
    )
    .unwrap()
    .requests
    .is_empty());
    }
}

#[test]
fn cold_recovery_authority_uses_frozen_sqlite_policy_and_exact_owner() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("recovery-authority.db");
    let mut store = NolaStore::open(&database).unwrap();
    let mut config = core_config();
    config.turn_identity.host_invocation = Some(recovery_host_binding("delivery-model"));
    let begun = begin_assistant_turn(&mut store, BeginAssistantTurn {
        expected_configuration_revision:None, expected_parent_message_id:None,
        conversation_id:"authority-conversation".into(), conversation_title:"Recovery".into(),
        user_message_id:"authority-user".into(),user_part_id:"authority-user-part".into(),
        user_text:"Recover".into(),user_content_json:None,assistant_message_id:"authority-assistant".into(),
        context_checkpoint_id:None,core_config:config.clone(),submission:user_input(),created_at_ms:1000,
    }).unwrap();
    drop(store);
    let store = NolaStore::open(&database).unwrap();
    let proof = nola_application::load_exact_run_invocation(&store,"authority-conversation".into(),
        begun.begin.run_id.clone(),"authority-assistant".into()).unwrap().unwrap();
    let current = nola_application::RunRecoveryAuthority {
        host_catalog:config.host_catalog,tools:config.tools,permission_rules:config.permission_rules,
        context_window:config.context_window,host_invocation:config.turn_identity.host_invocation.unwrap(),
    };
    let verify = |current, run: String, digest: String| nola_application::validate_run_recovery_authority(
        &store,"authority-conversation".into(),run,"authority-assistant".into(),digest,current);
    let receipt=verify(current.clone(),proof.run_id.clone(),proof.invocation_config_digest.clone()).unwrap();
    assert_eq!(receipt.invocation_config_digest,proof.invocation_config_digest);
    assert!(verify(current.clone(),"other-run".into(),proof.invocation_config_digest.clone()).is_err());
    assert!(verify(current.clone(),proof.run_id.clone(),"0".repeat(64)).is_err());
    let mut changed=current; changed.context_window.reserved_output_tokens+=1;
    assert!(verify(changed,proof.run_id.clone(),proof.invocation_config_digest.clone()).is_err());
    let after=nola_application::load_exact_run_invocation(&store,"authority-conversation".into(),
        proof.run_id,"authority-assistant".into()).unwrap().unwrap();
    assert_eq!(after.invocation_config_digest,proof.invocation_config_digest);
    assert_eq!(after.host_invocation,proof.host_invocation);
}

#[test]
fn dead_worker_observation_recovers_before_expiry_and_fences_renewal() {
    use nola_application::run_process_observation::{RunProcessObservation, RunProcessObservationPlan};
    use nola_host_protocol::{HostProcessObservation, HostWorkerProcessIdentity};
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("dead-worker.db");
    let mut store = NolaStore::open(&database).unwrap();
    let binding = recovery_host_binding("delivery-model");
    let mut config = core_config();
    config.turn_identity.host_invocation = Some(binding.clone());
    let begun = begin_assistant_turn(&mut store, BeginAssistantTurn {
        expected_configuration_revision: None, expected_parent_message_id: None,
        conversation_id: "delivery-conversation".into(), conversation_title: "Exact owner".into(),
        user_message_id: "dead-user".into(), user_part_id: "dead-part".into(), user_text: "Resume".into(),
        user_content_json: None, assistant_message_id: "dead-assistant".into(), context_checkpoint_id: None,
        core_config: config, submission: user_input(), created_at_ms: 1000,
    }).unwrap();
    let owner = HostWorkerProcessIdentity { pid: 1234, boot_id: "boot-a".into(), start_id: "start-a".into(), nonce: "worker-a".into() };
    let identity = owner.lease_identity().unwrap();
    let request = claim_committed_host_requests(&mut store, ClaimCommittedHostRequests {
        run_id: begun.begin.run_id.clone(), worker_generation: identity.worker_generation,
        lease_owner: identity.lease_owner, claimed_at_ms: 1100, lease_expires_at_ms: 241100, limit: 1,
    }).unwrap().requests.remove(0);
    reduce_and_accept_host_response(&mut store, ReduceAndAcceptHostResponse {
        conversation_id: "delivery-conversation".into(), run_id: begun.begin.run_id.clone(),
        invocation_config_digest: begun.invocation_config_digest.clone(),
        response: response(&request, 1, HostResponse::ExecutionStarted), received_at_ms: 1200,
    }).unwrap();
    drop(store);
    let mut runtime = support::open_runtime(&database).unwrap();
    let inspect = || ApplicationCommand::InspectRunProcessOwners {
        conversation_id: "delivery-conversation".into(), expected_run_id: begun.begin.run_id.clone(),
    };
    assert!(inspect().is_read_only_query());
    let plan: RunProcessObservationPlan = serde_json::from_value(runtime.execute(inspect()).unwrap()).unwrap();
    assert_eq!(plan.process_owners, vec![owner.clone()]);
    let validate = |plan: RunProcessObservationPlan, observation| ApplicationCommand::ValidateRunProcessObservations {
        plan, observations: vec![RunProcessObservation { owner: owner.clone(), observation }],
    };
    assert_eq!(runtime.execute(validate(plan.clone(), HostProcessObservation::Running {
        pid: owner.pid, boot_id: owner.boot_id.clone(), start_id: owner.start_id.clone(),
    })).unwrap(), serde_json::json!(false));
    assert_eq!(runtime.execute(validate(plan.clone(), HostProcessObservation::Unknown { pid: owner.pid })).unwrap(), serde_json::json!(false));
    assert!(runtime.execute(ApplicationCommand::ValidateRunProcessObservations { plan: plan.clone(), observations: vec![] }).is_err());
    let mut unobservable = plan.clone();
    unobservable.has_unobservable_owner = true;
    assert!(!nola_application::run_process_observation::validate_owner_observations(&unobservable,
        &[RunProcessObservation { owner: owner.clone(), observation: HostProcessObservation::Gone { pid: owner.pid, boot_id: owner.boot_id.clone() } }]).unwrap());
    let mut incomplete = plan.clone();
    incomplete.process_owners.push(HostWorkerProcessIdentity { pid: 1235, ..owner.clone() });
    assert!(nola_application::run_process_observation::validate_owner_observations(&incomplete,
        &[RunProcessObservation { owner: owner.clone(), observation: HostProcessObservation::Gone { pid: owner.pid, boot_id: owner.boot_id.clone() } }]).is_err());
    assert!(runtime.execute(validate(plan.clone(), HostProcessObservation::Gone {
        pid: owner.pid + 1, boot_id: owner.boot_id.clone(),
    })).is_err());
    let reused_pid = HostProcessObservation::Running { pid: owner.pid, boot_id: owner.boot_id.clone(), start_id: "new-start".into() };
    assert_eq!(runtime.execute(validate(plan.clone(), reused_pid.clone())).unwrap(), serde_json::json!(true));
    let _: serde_json::Value = runtime.execute(ApplicationCommand::HeartbeatEffectDelivery {
        input: heartbeat(&begun.begin.run_id, &plan.deliveries[0], 1300, 242000),
    }).unwrap();
    assert!(runtime.execute(validate(plan.clone(), reused_pid.clone())).unwrap_err().contains("changed"));
    assert!(runtime.execute(ApplicationCommand::RecoverObservedOrphanedRun {
        conversation_id: plan.conversation_id.clone(), expected_run_id: plan.run_id.clone(), host_invocation: binding.clone(),
        observed_session_lease: None, observed_deliveries: plan.deliveries, interrupted_at: "1400".into(), interrupted_at_ms: 1400,
    }).unwrap_err().contains("ownership changed"));
    let current: RunProcessObservationPlan = serde_json::from_value(runtime.execute(inspect()).unwrap()).unwrap();
    assert_eq!(runtime.execute(validate(current.clone(), reused_pid)).unwrap(), serde_json::json!(true));
    assert_eq!(runtime.execute(ApplicationCommand::RecoverObservedOrphanedRun {
        conversation_id: current.conversation_id, expected_run_id: current.run_id, host_invocation: binding,
        observed_session_lease: None, observed_deliveries: current.deliveries, interrupted_at: "1400".into(), interrupted_at_ms: 1400,
    }).unwrap(), serde_json::json!(1));
}
