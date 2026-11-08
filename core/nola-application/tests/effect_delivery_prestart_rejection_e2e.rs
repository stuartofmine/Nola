// SPDX-License-Identifier: Apache-2.0
//! Durable pre-execution rejection through the real Application + SQLite boundary.

#[path = "support/model_identity.rs"]
mod fixture_model_identity;
use nola_application::{
    begin_assistant_turn, claim_committed_host_requests, reduce_and_accept_host_response,
    BeginAssistantTurn, ClaimCommittedHostRequests, ReduceAndAcceptHostResponse,
};
use nola_agent_engine::agent::host::*;
use nola_agent_engine::agent::protocol::*;
use nola_domain::{EffectDeliveryAttemptState, EffectDeliverySettlement};
use nola_store::NolaStore;
use std::collections::BTreeMap;

fn config() -> AgentCoreConfig {
    let mut capabilities = BTreeMap::new();
    capabilities.insert(
        "model.stream".into(),
        HostCapabilityDescriptor {
            id: "model.stream".into(),
            title: "model.stream".into(),
            route: HostCapabilityRoute::Model,
            availability: CapabilityAvailability::Available,
            exposure: HostCapabilityExposure::Internal,
            permission_domain: None,
            input_schema: serde_json::json!({"type":"object"}),
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
        },
    );
    AgentCoreConfig {
        turn_identity: fixture_model_identity::identity(),
        session_id: Some(SessionId::from_string("prestart-rejection".into())),
        model: Some("fixture-model".into()),
        host_catalog: Some(HostCapabilityCatalog {
            host_id: HostId("fixture-host".into()),
            revision: 1,
            availability_epoch: 1,
            lease: None,
            capabilities,
        }),
        ..AgentCoreConfig::default()
    }
}

fn response(request: &HostRequestEnvelope, payload: HostResponse) -> HostResponseEnvelope {
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
        sequence: 0,
        payload,
    }
}

#[test]
fn leased_rejection_settles_attempt_and_persists_typed_event_atomically() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("prestart-rejection.db");
    let mut store = NolaStore::open(&database).unwrap();
    let begun = begin_assistant_turn(
        &mut store,
        BeginAssistantTurn {
            expected_configuration_revision: None,
            expected_parent_message_id: None,
            conversation_id: "prestart-rejection".into(),
            conversation_title: "Pre-start rejection".into(),
            user_message_id: "user".into(),
            user_part_id: "user-part".into(),
            user_text: "执行请求".into(),
            user_content_json: None,
            assistant_message_id: "assistant".into(),
            context_checkpoint_id: None,
            core_config: config(),
            submission: AgentSubmission::UserInput {
                items: vec![AgentItem::user_message(
                    ItemId::from_string("input".into()),
                    "执行请求",
                )],
                turn_context_items: Vec::new(),
                turn_context_platform: Default::default(),
                turn_context_components: Vec::new(),
                submission_id: Some("submission".into()),
                submitted_at_epoch_ms: Some(1_000),
                delivery: AgentInputDelivery::CurrentTurn,
            },
            created_at_ms: 1_000,
        },
    )
    .unwrap();
    let claimed = claim_committed_host_requests(
        &mut store,
        ClaimCommittedHostRequests {
            run_id: begun.begin.run_id.clone(),
            worker_generation: "generation".into(),
            lease_owner: "runtime".into(),
            claimed_at_ms: 1_100,
            lease_expires_at_ms: 2_000,
            limit: 1,
        },
    )
    .unwrap();
    let request = claimed.requests.into_iter().next().unwrap();
    let attempt_id = request.delivery.as_ref().unwrap().attempt_id.clone();

    let accepted = reduce_and_accept_host_response(
        &mut store,
        ReduceAndAcceptHostResponse {
            conversation_id: "prestart-rejection".into(),
            run_id: begun.begin.run_id.clone(),
            invocation_config_digest: begun.invocation_config_digest,
            response: response(
                &request,
                HostResponse::ExecutionRejected {
                    code: "deadline_exceeded".into(),
                    message: "expired before execution".into(),
                    retryable: false,
                },
            ),
            received_at_ms: 1_200,
        },
    )
    .unwrap();

    assert!(accepted.control.unwrap().turn_status.is_terminal());
    let attempt = store.effect_delivery_attempt(&attempt_id).unwrap().unwrap();
    assert_eq!(attempt.state, EffectDeliveryAttemptState::Settled);
    assert_eq!(
        attempt.settlement,
        Some(EffectDeliverySettlement::RejectedBeforeExecution)
    );
    assert_eq!(
        attempt.interruption_reason.as_deref(),
        Some("deadline_exceeded: expired before execution")
    );

    let connection = rusqlite::Connection::open(database).unwrap();
    let payload: String = connection
        .query_row(
            "SELECT payload_json FROM host_event_inbox WHERE run_id=?1",
            [&begun.begin.run_id],
            |row| row.get(0),
        )
        .unwrap();
    let payload: serde_json::Value = serde_json::from_str(&payload).unwrap();
    assert_eq!(payload["payload"]["type"], "execution_rejected");
    assert_eq!(payload["payload"]["code"], "deadline_exceeded");
}
