#[path = "support/model_identity.rs"]
mod model_identity;
// SPDX-License-Identifier: Apache-2.0

use nola_core::agent::host::*;
use nola_core::agent::recovery::{CheckpointReason, CoreCheckpoint};
use nola_core::agent::*;
use serde_json::json;
use std::collections::BTreeMap;

fn capability(
    id: &str,
    route: HostCapabilityRoute,
    permission_domain: Option<&str>,
) -> HostCapabilityDescriptor {
    HostCapabilityDescriptor {
        id: id.to_string(),
        title: id.to_string(),
        route,
        availability: CapabilityAvailability::Available,
        exposure: HostCapabilityExposure::Visible,
        permission_domain: permission_domain.map(str::to_string),
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

fn config() -> AgentCoreConfig {
    AgentCoreConfig {
        turn_identity: model_identity::host_identity(),
        session_id: Some(SessionId::from_string("usage-contract".into())),
        model: Some("test-model".into()),
        host_catalog: Some(HostCapabilityCatalog {
            host_id: HostId("usage-host".into()),
            revision: 1,
            availability_epoch: 1,
            lease: Some(HostLease {
                lease_id: "usage-lease".into(),
                issued_at_ms: 1,
                last_heartbeat_ms: 1,
                expires_at_ms: u64::MAX,
            }),
            capabilities: BTreeMap::from([(
                "model.stream".into(),
                capability("model.stream", HostCapabilityRoute::Model, None),
            )]),
        }),
        ..Default::default()
    }
}

fn start(core: &mut AgentCore, id: &str) -> HostRequestEnvelope {
    let transition = core
        .submit(AgentSubmission::UserInput {
            items: vec![AgentItem::user_message(
                ItemId::from_string(id.into()),
                "Continue the current canonical history",
            )],
            turn_context_items: vec![],
            turn_context_platform: Default::default(),
            turn_context_components: vec![],
            submission_id: Some(format!("submit-{id}")),
            submitted_at_epoch_ms: Some(1000),
            delivery: AgentInputDelivery::CurrentTurn,
        })
        .unwrap();
    transition
        .host_requests
        .into_iter()
        .find(|request| matches!(request.payload, HostRequest::ModelStream { .. }))
        .unwrap()
}

fn complete(core: &mut AgentCore, request: &HostRequestEnvelope, id: &str, usage: TokenUsage) {
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
            sequence: 1,
            payload: HostResponse::ModelCompleted {
                items: vec![serde_json::to_value(AgentItem::assistant_message(
                    ItemId::from_string(id.into()),
                    "Completed this round.",
                ))
                .unwrap()],
                tool_calls: vec![],
                usage: serde_json::to_value(usage).unwrap(),
            },
        },
    })
    .unwrap();
}

fn complete_request_estimate(request: &HostRequestEnvelope) -> usize {
    let HostRequest::ModelStream { messages, tools, settings, .. } = &request.payload else {
        panic!("expected model request")
    };
    let plan: ModelInvocationPlan = serde_json::from_value(settings["model_invocation_plan"].clone()).unwrap();
    assert_eq!(plan.instruction_profile, ModelInstructionProfile::Generic,
        "The deterministic fixture has no Host-only instruction overlay");
    let items: Vec<AgentItem> = serde_json::from_value(json!(messages)).unwrap();
    let schema_tokens = serde_json::to_vec(tools).unwrap().len().saturating_add(3) / 4;
    assert_eq!(settings["step_context"]["tool_projection"]["schema_token_estimate"], json!(schema_tokens));
    items.iter().fold(schema_tokens, |sum, item| sum.saturating_add(item.estimated_tokens()))
}

fn reopen(core: &AgentCore) -> AgentCore {
    let checkpoint = core.checkpoint(CheckpointReason::Manual);
    let bytes = serde_json::to_vec(&checkpoint).unwrap();
    let decoded: CoreCheckpoint<AgentThreadState> = serde_json::from_slice(&bytes).unwrap();
    AgentCore::restore(config(), decoded).unwrap()
}

#[test]
fn model_usage_survives_cold_restore_without_becoming_rebased_context_pressure() {
    let mut core = AgentCore::try_new(config()).unwrap();
    let first = start(&mut core, "first-user");
    let first_estimate = complete_request_estimate(&first);
    assert_eq!(core.snapshot().context_usage.used_tokens, first_estimate);
    complete(
        &mut core,
        &first,
        "first-answer",
        TokenUsage {
            input_tokens: Some(8000),
            cached_input_tokens: Some(7000),
            cache_write_input_tokens: Some(600),
            output_tokens: Some(120),
            reasoning_tokens: Some(30),
            total_tokens: Some(0),
            ..Default::default()
        },
    );
    let provider = core.snapshot().context_usage;
    assert_eq!(provider.used_tokens, 8000);
    assert!(!provider.usage_is_estimated);
    let first_checkpoint = core.checkpoint(CheckpointReason::Manual);
    let usage = &first_checkpoint
        .state
        .conversation
        .context_window
        .cumulative_usage;
    assert_eq!(
        (
            usage.input_tokens,
            usage.cached_input_tokens,
            usage.cache_write_input_tokens,
            usage.output_tokens,
            usage.reasoning_tokens,
            usage.total_tokens
        ),
        (Some(8000), Some(7000), Some(600), Some(120), Some(30), Some(0))
    );
    core = reopen(&core);
    assert_eq!(core.snapshot().context_usage, provider);
    assert_eq!(
        serde_json::to_value(
            &core
                .checkpoint(CheckpointReason::Manual)
                .state
                .conversation
                .context_window
                .cumulative_usage
        )
        .unwrap(),
        serde_json::to_value(usage).unwrap()
    );

    let canonical = vec![AgentItem::user_message(
        ItemId::from_string("retained".into()),
        "Retained canonical context only",
    )];
    let expected_rebased = canonical
        .iter()
        .map(AgentItem::estimated_tokens)
        .sum::<usize>();
    core.submit(AgentSubmission::RebaseConversation {
        items: canonical.clone(),
        item_ownership: Default::default(),
    })
    .unwrap();
    assert_eq!(core.snapshot().context_usage.used_tokens, expected_rebased);
    assert!(core.snapshot().context_usage.usage_is_estimated);
    core = reopen(&core);
    assert_eq!(core.snapshot().context_usage.used_tokens, expected_rebased);
    let second = start(&mut core, "second-user");
    let before_zero = core.snapshot().context_usage;
    assert!(before_zero.usage_is_estimated);
    let request_estimate = complete_request_estimate(&second);
    let calibrated_estimate = ((request_estimate as u128) * 8000)
        .div_ceil(first_estimate as u128) as usize;
    assert_eq!(before_zero.used_tokens, request_estimate.max(calibrated_estimate),
        "The next request includes current runtime context and exact tool schemas, calibrated against the prior provider measurement");
    assert!(request_estimate > expected_rebased);
    complete(
        &mut core,
        &second,
        "second-answer",
        TokenUsage {
            input_tokens: Some(0),
            cached_input_tokens: Some(9000),
            cache_write_input_tokens: Some(400),
            output_tokens: Some(40),
            reasoning_tokens: Some(5),
            total_tokens: Some(0),
            ..Default::default()
        },
    );
    let after_zero = core.snapshot().context_usage;
    assert_eq!(after_zero.used_tokens, 0, "A reported zero is a measured value, not missing usage");
    assert!(!after_zero.usage_is_estimated);
    core = reopen(&core);
    let current = core.snapshot().context_usage;
    assert!(!current.usage_is_estimated);
    assert_eq!(current, after_zero);
    assert_eq!(current.used_tokens, 0);
    assert_eq!(current.cached_input_tokens, 16000);
    assert_eq!(current.cache_write_input_tokens, 1000);
    assert!(current.cached_input_tokens_reported && current.cache_write_input_tokens_reported);
    let checkpoint = core.checkpoint(CheckpointReason::Manual);
    let cumulative = &checkpoint
        .state
        .conversation
        .context_window
        .cumulative_usage;
    assert_eq!(
        (
            cumulative.input_tokens,
            cumulative.cached_input_tokens,
            cumulative.cache_write_input_tokens,
            cumulative.output_tokens,
            cumulative.reasoning_tokens,
            cumulative.total_tokens
        ),
        (Some(8000), Some(16000), Some(1000), Some(160), Some(35), Some(0))
    );
    assert!(checkpoint
        .state
        .conversation
        .model_context
        .starts_with(&canonical));
}
