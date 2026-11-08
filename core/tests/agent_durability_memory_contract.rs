// SPDX-License-Identifier: Apache-2.0
//! Durability boundaries must stay out of the Host effect journal and grow linearly.
#[path = "support/model_identity.rs"]
mod model_identity;

use std::collections::BTreeMap;

use nola_core::agent::host::{
    CapabilityAvailability, HostCancellationMode, HostCapabilityCatalog, HostCapabilityDescriptor,
    HostCapabilityExposure, HostCapabilityParallelism, HostCapabilityRoute, HostId,
    HostOutputPolicy, HostReplaySafety, HostRequest, HostResponse, HostResponseEnvelope,
    ModelDeltaChannel,
};
use nola_core::agent::recovery::CheckpointReason;
use nola_core::agent::{
    AgentCore, AgentCoreConfig, AgentInputDelivery, AgentItem, AgentSubmission, ItemId, SessionId,
};
use serde_json::json;

fn config() -> AgentCoreConfig {
    let capability = HostCapabilityDescriptor {
        id: "model.stream".to_string(),
        title: "Model stream".to_string(),
        route: HostCapabilityRoute::Model,
        availability: CapabilityAvailability::Available,
        exposure: HostCapabilityExposure::Internal,
        permission_domain: None,
        input_schema: json!({"type": "object"}),
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
    };
    AgentCoreConfig {
        turn_identity: model_identity::host_identity(),
        session_id: Some(SessionId::from_string("durability-memory".to_string())),
        model: Some("fixture-model".to_string()),
        host_catalog: Some(HostCapabilityCatalog {
            host_id: HostId("durability-host".to_string()),
            revision: 1,
            availability_epoch: 1,
            lease: None,
            capabilities: BTreeMap::from([("model.stream".to_string(), capability)]),
        }),
        ..AgentCoreConfig::default()
    }
}

#[test]
fn ten_thousand_stream_deltas_keep_checkpoint_growth_linear_and_payload_free() {
    let mut core = AgentCore::try_new(config()).expect("create Core");
    let started = core
        .submit(AgentSubmission::UserInput {
            items: vec![AgentItem::user_message(
                ItemId::from_string("durability-user".to_string()),
                "stream a long response",
            )],
            turn_context_items: Vec::new(),
            turn_context_platform: Default::default(),
            turn_context_components: Vec::new(),
            submission_id: Some("durability-submission".to_string()),
            submitted_at_epoch_ms: Some(1_000),
            delivery: AgentInputDelivery::CurrentTurn,
        })
        .expect("start Turn");
    assert!(started.durability.is_some());
    let model = started
        .host_requests
        .iter()
        .find(|request| matches!(request.payload, HostRequest::ModelStream { .. }))
        .cloned()
        .expect("model request");
    let text_part_id = match &model.payload {
        HostRequest::ModelStream { text_part_id, .. } => text_part_id.clone(),
        _ => unreachable!(),
    };
    let serialized_transition = serde_json::to_string(&started).expect("serialize transition");
    assert!(!serialized_transition.contains("persist_checkpoint"));
    assert!(!serialized_transition.contains("journal.checkpoint"));

    let baseline = serde_json::to_vec(&core.checkpoint(CheckpointReason::Manual))
        .expect("serialize baseline")
        .len();
    let mut first_thousand = 0;
    let mut last_checkpoint = baseline;

    for sequence in 1_u64..=10_000 {
        let transition = core
            .submit(AgentSubmission::HostResponse {
                response: HostResponseEnvelope {
                    delegated_invocation_id: model.delegated_invocation_id.clone(),
                    attempt_id: None,
                    request_id: model.request_id.clone(),
                    effect_id: model.effect_id.clone(),
                    host_id: model.host_id.clone(),
                    lease_id: model.lease_id.clone(),
                    session_id: model.session_id.clone(),
                    turn_id: model.turn_id.clone(),
                    catalog_revision: model.catalog_revision,
                    availability_epoch: model.availability_epoch,
                    delivery: model.delivery.clone(),
                    sequence,
                    payload: HostResponse::ModelDelta {
                        channel: ModelDeltaChannel::Text,
                        part_id: text_part_id.clone(),
                        delta: format!("segment-{sequence} "),
                    },
                },
            })
            .expect("accept stream delta");
        assert!(transition.durability.is_none());
        assert!(transition.host_requests.is_empty());

        if sequence % 64 == 0 || sequence == 1_000 || sequence == 10_000 {
            last_checkpoint = serde_json::to_vec(&core.checkpoint(CheckpointReason::Manual))
                .expect("serialize bounded checkpoint")
                .len();
            if sequence == 1_000 {
                first_thousand = last_checkpoint;
            }
        }
    }

    let first_growth = first_thousand.saturating_sub(baseline).max(1);
    let total_growth = last_checkpoint.saturating_sub(baseline);
    assert!(
        total_growth <= first_growth.saturating_mul(14),
        "checkpoint growth must remain linear: baseline={baseline}, first={first_thousand}, last={last_checkpoint}"
    );
    assert_eq!(
        core.checkpoint(CheckpointReason::Manual)
            .state
            .host
            .pending_effects
            .len(),
        1,
        "the journal must contain only the in-flight model effect"
    );

    let completed = core
        .submit(AgentSubmission::HostResponse {
            response: HostResponseEnvelope {
                delegated_invocation_id: model.delegated_invocation_id.clone(),
                attempt_id: None,
                request_id: model.request_id,
                effect_id: model.effect_id,
                host_id: model.host_id,
                lease_id: model.lease_id,
                session_id: model.session_id,
                turn_id: model.turn_id,
                catalog_revision: model.catalog_revision,
                availability_epoch: model.availability_epoch,
                delivery: model.delivery.clone(),
                sequence: 10_001,
                payload: HostResponse::ModelCompleted {
                    items: Vec::new(),
                    tool_calls: Vec::new(),
                    usage: json!({}),
                },
            },
        })
        .expect("complete stream");
    assert!(completed.snapshot.turn_status.is_terminal());
    assert!(completed.durability.is_some());
    assert!(core.pending_host_requests().unwrap().is_empty());
    let final_checkpoint = core.checkpoint(CheckpointReason::TurnCompleted);
    assert_eq!(final_checkpoint.state.host.pending_effects.len(), 0);
}
