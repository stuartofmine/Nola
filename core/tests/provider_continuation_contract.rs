#[path = "support/model_identity.rs"]
mod model_identity;
// SPDX-License-Identifier: Apache-2.0

use std::collections::BTreeMap;

use nola_core::agent::host::{
    CapabilityAvailability, HostCancellationMode, HostCapabilityCatalog, HostCapabilityDescriptor,
    HostCapabilityExposure, HostCapabilityParallelism, HostCapabilityRoute, HostError, HostId,
    HostLease, HostOutputPolicy, HostReplaySafety, HostRequest, HostRequestEnvelope, HostResponse,
    HostResponseEnvelope,
};
use nola_core::agent::protocol::{
    AgentSessionConfigurationPatch, ModelProviderContinuationState,
    ProviderContinuationCompactionPolicy,
};
use nola_core::agent::recovery::{CheckpointReason, CoreCheckpoint};
use nola_core::agent::{
    AgentCore, AgentCoreConfig, AgentInputDelivery, AgentItem, AgentRole, AgentSubmission,
    AgentThreadState, ContextPlatform, ItemId, ModelInvocationPlan, SessionId, TokenUsage,
};
use serde_json::{json, Value};

fn capability(id: &str, route: HostCapabilityRoute) -> HostCapabilityDescriptor {
    HostCapabilityDescriptor {
        id: id.to_string(),
        title: id.to_string(),
        route,
        availability: CapabilityAvailability::Available,
        exposure: HostCapabilityExposure::Visible,
        permission_domain: None,
        input_schema: json!({"type": "object"}),
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

fn catalog() -> HostCapabilityCatalog {
    let mut capabilities = BTreeMap::new();
    for (id, route) in [
        ("model.stream", HostCapabilityRoute::Model),
        ("context.compact", HostCapabilityRoute::Compaction),
    ] {
        capabilities.insert(id.to_string(), capability(id, route));
    }
    HostCapabilityCatalog {
        host_id: HostId("continuation-test-host".to_string()),
        revision: 1,
        availability_epoch: 1,
        lease: Some(HostLease {
            lease_id: "continuation-test-lease".to_string(),
            issued_at_ms: 1,
            last_heartbeat_ms: 1,
            expires_at_ms: u64::MAX,
        }),
        capabilities,
    }
}

fn config() -> AgentCoreConfig {
    AgentCoreConfig {
        turn_identity: model_identity::host_identity(),
        session_id: Some(SessionId::from_string(
            "provider-continuation-contract".to_string(),
        )),
        model: Some("test-model".to_string()),
        host_catalog: Some(catalog()),
        ..AgentCoreConfig::default()
    }
}

fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/provider_continuation_v1.json"))
        .expect("provider continuation fixture")
}

fn committed_continuation() -> ModelProviderContinuationState {
    serde_json::from_value(fixture()["committed"].clone()).expect("typed continuation")
}

fn user_input(id: &str, text: &str) -> AgentSubmission {
    AgentSubmission::UserInput {
        items: vec![AgentItem::Message {
            id: ItemId::from_string(id.to_string()),
            role: AgentRole::User,
            content: text.to_string(),
        }],
        turn_context_items: Vec::new(),
        turn_context_platform: ContextPlatform::Unknown,
        turn_context_components: Vec::new(),
        submission_id: Some(format!("submission-{id}")),
        submitted_at_epoch_ms: None,
        delivery: AgentInputDelivery::CurrentTurn,
    }
}

fn request_matching(
    requests: &[HostRequestEnvelope],
    predicate: impl Fn(&HostRequest) -> bool,
) -> HostRequestEnvelope {
    requests
        .iter()
        .find(|request| predicate(&request.payload))
        .cloned()
        .expect("expected Host request")
}

fn model_request(requests: &[HostRequestEnvelope]) -> HostRequestEnvelope {
    request_matching(requests, |request| {
        matches!(request, HostRequest::ModelStream { .. })
    })
}

fn submit_response(
    core: &mut AgentCore,
    request: &HostRequestEnvelope,
    sequence: u64,
    payload: HostResponse,
) -> nola_core::agent::AgentTransition {
    core.submit(AgentSubmission::HostResponse {
        response: HostResponseEnvelope {
            delegated_invocation_id: request.delegated_invocation_id.clone(),
            attempt_id: None,
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
    })
    .expect("Host response")
}

fn checkpoint(core: &AgentCore) -> CoreCheckpoint<AgentThreadState> {
    core.checkpoint(CheckpointReason::Manual)
}

fn complete_without_reasoning(
    core: &mut AgentCore,
    request: &HostRequestEnvelope,
    sequence: u64,
) -> nola_core::agent::AgentTransition {
    submit_response(
        core,
        request,
        sequence,
        HostResponse::ModelCompleted {
            items: vec![serde_json::to_value(AgentItem::Message {
                id: ItemId::from_string("assistant-complete".to_string()),
                role: AgentRole::Assistant,
                content: "answer".to_string(),
            })
            .expect("assistant item")],
            tool_calls: Vec::new(),
            usage: serde_json::to_value(TokenUsage::default()).expect("usage"),
        },
    )
}

fn stage(
    core: &mut AgentCore,
    request: &HostRequestEnvelope,
    continuation: ModelProviderContinuationState,
) -> nola_core::agent::AgentTransition {
    submit_response(
        core,
        request,
        1,
        HostResponse::ModelContinuation { continuation },
    )
}

#[test]
fn independent_review_cannot_replace_generation_continuation() {
    let runtime_config = config();
    let generation = committed_continuation();
    let mut core = AgentCore::try_new(runtime_config.clone()).expect("create Core");
    let started = core
        .submit(user_input("review-user", "answer the request"))
        .expect("start turn");
    let mut forced_review = checkpoint(&core);
    forced_review
        .state
        .execution
        .active_turn
        .as_mut()
        .expect("active turn")
        .final_answer_audit_required = true;
    let mut core = AgentCore::restore(runtime_config, forced_review).expect("restore Core");
    let model = model_request(&started.host_requests);
    stage(&mut core, &model, generation.clone());
    let candidate = complete_without_reasoning(&mut core, &model, 2);
    let review = model_request(&candidate.host_requests);
    assert!(matches!(review.payload, HostRequest::ModelStream {
        response_mode: nola_core::agent::host::ModelResponseMode::StructuredAudit,
        ..
    }));

    let mut reviewer = generation.clone();
    reviewer.response_id = Some("reviewer-response".into());
    reviewer.state = json!({"review_only": true});
    submit_response(
        &mut core,
        &review,
        1,
        HostResponse::ModelContinuation {
            continuation: reviewer.clone(),
        },
    );
    let reviewed = submit_response(
        &mut core,
        &review,
        2,
        HostResponse::ModelCompleted {
            items: vec![
                serde_json::to_value(AgentItem::Reasoning {
                    id: ItemId::from_string("reviewer-reasoning".to_string()),
                    summary: String::new(),
                    content: String::new(),
                    provider_continuation: Some(reviewer),
                })
                .expect("reasoning"),
                serde_json::to_value(AgentItem::Message {
                    id: ItemId::from_string("reviewer-report".to_string()),
                    role: AgentRole::Assistant,
                    content: r#"{"decision":"accept","outcome":"answered","constraints":[],"items":[{"label":"answer","eligibility":"eligible","evidence":"Candidate directly answers the request."}],"violations":[]}"#.into(),
                })
                .expect("review report"),
            ],
            tool_calls: Vec::new(),
            usage: serde_json::to_value(TokenUsage::default()).expect("usage"),
        },
    );
    assert_eq!(
        reviewed.snapshot.turn_status,
        nola_core::agent::AgentTurnStatus::Completed
    );
    assert_eq!(checkpoint(&core).state.conversation.provider_continuation, Some(generation));
}

#[test]
fn continuation_is_pending_until_model_completion_then_survives_restore() {
    let runtime_config = config();
    let expected = committed_continuation();
    let mut core = AgentCore::try_new(runtime_config.clone()).expect("create Core");
    let started = core
        .submit(user_input("user-1", "first turn"))
        .expect("start turn");
    let model = model_request(&started.host_requests);

    let pending = stage(&mut core, &model, expected.clone());
    assert!(
        pending.durability.is_none(),
        "pending provider state must not request a durable commit before completion"
    );

    let completed = complete_without_reasoning(&mut core, &model, 2);
    let durable = checkpoint(&core);
    assert_eq!(
        durable.state.conversation.provider_continuation,
        Some(expected.clone())
    );
    assert!(
        !serde_json::to_string(&completed.snapshot.projection)
            .expect("projection JSON")
            .contains("provider_namespace"),
        "opaque provider state must not enter the UI projection"
    );

    let mut restored = AgentCore::restore(runtime_config, durable).expect("restore Core");
    let resumed = restored
        .submit(user_input("user-2", "second turn"))
        .expect("resume turn");
    let request = model_request(&resumed.host_requests);
    let HostRequest::ModelStream { settings, .. } = request.payload else {
        unreachable!()
    };
    let plan: ModelInvocationPlan =
        serde_json::from_value(settings["model_invocation_plan"].clone())
            .expect("model invocation plan");
    assert_eq!(plan.provider_continuation, Some(expected));
}

#[test]
fn failed_and_context_limit_recovery_do_not_commit_pending_state() {
    for (code, retryable) in [("provider_error", true), ("context_length_exceeded", false)] {
        let mut core = AgentCore::try_new(config()).expect("create Core");
        let started = core
            .submit(user_input(&format!("user-{code}"), "turn"))
            .expect("start turn");
        let model = model_request(&started.host_requests);
        stage(&mut core, &model, committed_continuation());
        let _failed = submit_response(
            &mut core,
            &model,
            2,
            HostResponse::Failed {
                error: HostError {
                    code: code.to_string(),
                    message: "fixture failure".to_string(),
                    retryable,
                    http_status: None,
                },
            },
        );
        let durable = checkpoint(&core);
        assert_eq!(
            durable.state.conversation.provider_continuation, None,
            "{code} must discard staged provider state"
        );
    }
}

#[test]
fn changing_model_clears_committed_provider_state() {
    let mut core = AgentCore::try_new(config()).expect("create Core");
    let started = core
        .submit(user_input("user-model", "turn"))
        .expect("start turn");
    let model = model_request(&started.host_requests);
    stage(&mut core, &model, committed_continuation());
    complete_without_reasoning(&mut core, &model, 2);

    let _updated = core
        .submit(AgentSubmission::UpdateSessionConfiguration {
            patch: AgentSessionConfigurationPatch {
                model: Some(Some("other-model".to_string())),
                ..AgentSessionConfigurationPatch::default()
            },
        })
        .expect("change model");
    assert_eq!(
        checkpoint(&core).state.conversation.provider_continuation,
        None
    );
}

#[test]
fn unknown_namespace_and_integrity_kind_round_trip_without_interpretation() {
    let source = fixture()["unknown_provider"].clone();
    let continuation: ModelProviderContinuationState =
        serde_json::from_value(source.clone()).expect("unknown continuation");

    assert_eq!(continuation.provider_namespace, "vendor.example.future");
    assert_eq!(continuation.integrity_kind, "future-integrity-v9");
    assert_eq!(
        continuation.compaction_policy,
        ProviderContinuationCompactionPolicy::Preserve
    );
    assert_eq!(
        serde_json::to_value(continuation).expect("round-trip continuation"),
        source
    );
}
