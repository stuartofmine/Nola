// SPDX-License-Identifier: Apache-2.0
//! A real source Turn; shared with the cross-process service read regression.
#[path = "model_identity.rs"]
mod fixture_model_identity;
#[path = "admission_frontier.rs"]
mod fixture_admission_frontier;
use nola_application::*;
use nola_agent_engine::agent::{host::*, protocol::*};
use nola_domain::committed_run_context_usage::CommittedRunContextUsage;
use nola_domain::remote_submission::{
    RegisteredSourceSubmission, RemoteSubmissionIdentity, SourceSubmissionReceipt,
};
use nola_store::NolaStore;
use serde_json::json;

/// The observer brackets only admitted streaming deltas, excluding initial
/// immutable payload admission and the deliberately cold terminal baseline.
pub fn seed_stream_with_observer(
    store: &mut NolaStore,
    tokens: Option<usize>,
    config_padding: usize,
    deltas: usize,
    observer: impl FnMut(bool),
) -> (CommittedRunContextUsage, SourceSubmissionReceipt) {
    let identity = RemoteSubmissionIdentity {
        source_host_id: "usage-host".into(),
        client_id: "usage-client".into(),
        conversation_id: "usage".into(),
        submission_id: "usage-run".into(),
        user_message_id: "usage-user".into(),
        assistant_message_id: "usage-assistant".into(),
    };
    seed_stream_with_identity(store, tokens, config_padding, deltas, observer, identity)
}

pub fn seed_stream_with_identity(
    store: &mut NolaStore,
    tokens: Option<usize>,
    config_padding: usize,
    deltas: usize,
    mut observer: impl FnMut(bool),
    identity: RemoteSubmissionIdentity,
) -> (CommittedRunContextUsage, SourceSubmissionReceipt) {
    let registered: RegisteredSourceSubmission = serde_json::from_value(
        execute_application_command(
            store,
            ApplicationCommand::RegisterSourceSubmission {
                identity: identity.clone(),
                request: json!({"message":"Usage fixture"}),
            },
        )
        .unwrap(),
    )
    .unwrap();
    let config = AgentCoreConfig {
        turn_identity: fixture_model_identity::identity(),
        session_id: Some(SessionId::from_string("usage".into())),
        model: Some("fixture-model".into()),
        context_window: nola_agent_engine::agent::context_window::ContextWindowConfig {
            context_window_tokens: 65536,
            reserved_output_tokens: 8192,
            ..Default::default()
        },
        host_catalog: Some(HostCapabilityCatalog {
            host_id: HostId("usage-host".into()),
            revision: 1,
            availability_epoch: 1,
            lease: None,
            capabilities: std::collections::BTreeMap::from([(
                "model.stream".into(),
                HostCapabilityDescriptor {
                    id: "model.stream".into(),
                    title: "Model".into(),
                    route: HostCapabilityRoute::Model,
                    availability: CapabilityAvailability::Available,
                    exposure: HostCapabilityExposure::Internal,
                    permission_domain: None,
                    input_schema: json!({"type":"object","description":"x".repeat(config_padding)}),
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
                },
            )]),
        }),
        ..Default::default()
    };
    let frontier = fixture_admission_frontier::read(store, "usage");
    let begun = begin_assistant_turn(
        store,
        BeginAssistantTurn {
            expected_configuration_revision: frontier.0,
            expected_parent_message_id: frontier.1,
            conversation_id: "usage".into(),
            conversation_title: "Usage".into(),
            user_message_id: identity.user_message_id.clone(),
            user_part_id: format!("{}:text", identity.user_message_id),
            user_text: "Usage fixture".into(),
            user_content_json: None,
            assistant_message_id: identity.assistant_message_id.clone(),
            context_checkpoint_id: None,
            core_config: config,
            submission: AgentSubmission::UserInput {
                items: vec![AgentItem::user_message(
                    ItemId::from_string(identity.user_message_id.clone()),
                    "Usage fixture",
                )],
                turn_context_items: vec![],
                turn_context_platform: Default::default(),
                turn_context_components: vec![],
                submission_id: Some(identity.submission_id.clone()),
                submitted_at_epoch_ms: Some(1000),
                delivery: AgentInputDelivery::CurrentTurn,
            },
            created_at_ms: 1000,
        },
    )
    .unwrap();
    let claimed = claim_committed_host_requests(
        store,
        ClaimCommittedHostRequests {
            run_id: begun.begin.run_id.clone(),
            worker_generation: "usage-generation".into(),
            lease_owner: "usage-worker".into(),
            claimed_at_ms: 1001,
            lease_expires_at_ms: 61000,
            limit: 64,
        },
    )
    .unwrap();
    let mut request = claimed
        .requests
        .into_iter()
        .find(|r| matches!(r.payload, HostRequest::ModelStream { .. }))
        .unwrap();
    let mut responses = vec![
        HostResponse::ExecutionStarted,
        HostResponse::ModelAttemptStarted {
            provider: "fixture".into(),
            model: "fixture-model".into(),
            reason: "initial".into(),
        },
    ];
    let text_part_id = match &request.payload {
        HostRequest::ModelStream { text_part_id, .. } => text_part_id.clone(),
        _ => unreachable!(),
    };
    responses.extend((0..deltas).map(|_| HostResponse::ModelDelta {
        channel: ModelDeltaChannel::Text,
        part_id: text_part_id.clone(),
        delta: "bounded stream fragment".into(),
    }));
    if let Some(tokens) = tokens {
        responses.push(HostResponse::ModelCompleted {
            items: vec![serde_json::to_value(AgentItem::assistant_message(
                ItemId::from_string(format!("{}:model-result", identity.assistant_message_id)),
                "Done",
            ))
            .unwrap()],
            tool_calls: vec![],
            usage: json!({"input_tokens":tokens,"output_tokens":0,"total_tokens":tokens,
                "cache_usage": nola_domain::PromptCacheUsage::measured(Some(tokens as u64), Some((tokens/2) as u64), Some(0), false)}),
        });
    }
    for (sequence, payload) in responses.into_iter().enumerate() {
        if deltas > 0 && sequence == 2 {
            observer(true);
        }
        reduce_and_accept_host_response(
            store,
            ReduceAndAcceptHostResponse {
                conversation_id: "usage".into(),
                run_id: begun.begin.run_id.clone(),
                invocation_config_digest: begun.invocation_config_digest.clone(),
                received_at_ms: 1010 + sequence as i64,
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
                    sequence: sequence as u64 + 1,
                    payload,
                },
            },
        )
        .unwrap();
        if deltas > 0 && sequence == deltas + 1 {
            observer(false);
        }
        if sequence == 1 {
            let delivery = request.delivery.clone();
            request = load_committed_host_requests(store, &begun.begin.run_id)
                .unwrap()
                .requests
                .into_iter()
                .find(|r| r.effect_id == request.effect_id)
                .unwrap();
            request.delivery = delivery;
        }
    }
    let receipt: SourceSubmissionReceipt = serde_json::from_value(
        execute_application_command(
            store,
            ApplicationCommand::ReadSourceSubmissionReceipt {
                identity,
                request_digest: registered.receipt.request_digest,
            },
        )
        .unwrap(),
    )
    .unwrap();
    let record = store
        .local_run_context_usage(&receipt.admission.as_ref().unwrap().identity)
        .unwrap()
        .unwrap();
    (record, receipt)
}
