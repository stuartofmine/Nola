// SPDX-License-Identifier: Apache-2.0
//! Owned checkpoint boundaries remain wire-transparent and bounded on the stack.
use super::*;
use nola_host_protocol::*;

fn host_binding() -> HostInvocationBinding {
    let route = HostModelRouteBinding::Http(HostHttpModelRouteBinding {
        siwc_account: None,
        protocol: nola_domain::generation::ModelProtocol::ChatCompletions,
        output_token_field: ModelOutputTokenField::MaxTokens,
        provider: Some("stack-fixture".into()),
        endpoint: "https://fixture.invalid/v1".into(),
        endpoint_query_sha256: None,
        endpoint_query_source: HostSecretSource::None,
        model: "stack-model".into(),
        credential_sha256: EMPTY_CREDENTIAL_SHA256.into(),
        credential_source: HostSecretSource::None,
    });
    let output_admission = HostOutputAdmission::admit(
        vec![HostRouteTokenLimits {
            route: route.clone(),
            max_input_tokens: None,
            max_output_tokens: None,
            max_total_tokens: None,
        }],
        nola_domain::generation::OutputTokenPreference::Default,
    )
    .unwrap();
    let binding = HostInvocationBinding {
        mac_launch: None,
        version: HOST_INVOCATION_BINDING_VERSION,
        primary: route,
        fallbacks: vec![],
        generation: HostGenerationBinding {
            max_retries: 2,
            parallel_tool_calls: None,
            reasoning_selection: None,
            reasoning_tier: Default::default(),
            sampling_override: false,
            generation_options: nola_domain::generation::GenerationOptions::default()
                .try_into()
                .unwrap(),
            temperature: serde_json::Number::from_f64(0.7).unwrap(),
            top_p: serde_json::Number::from_f64(0.9).unwrap(),
            max_output_tokens: output_admission.resolved.tokens,
            thinking_enabled: false, reasoning_summary_supported: None,
            prompt_cache: false,
            prompt_cache_key_sha256: None,
            prompt_cache_key_prefix_sha256: None,
            prompt_cache_retention: None,
        },
        output_admission,
        execution_policy_sha256: "b".repeat(64),
    };
    binding.validate().unwrap();
    binding
}

fn active_runtime() -> AgentRuntime {
    let mut runtime = AgentRuntime::try_new(AgentCoreConfig {
        session_id: Some(SessionId::from_string("heap-owned-checkpoint".into())),
        model: Some("stack-model".into()),
        tools: vec![json!({"type":"function","function":{"name":"frozen_tool","parameters":{"type":"object"}}})],
        turn_identity: super::super::protocol::AgentTurnIdentity {
            host_invocation: Some(host_binding()),
            ..crate::agent::model_capability::test_host_identity()
        },
        ..Default::default()
    }).unwrap();
    let context = Box::new(AgentTurnContext {
        input_origin: Default::default(),
        configuration: runtime.session.configuration.clone(),
        platform: Default::default(),
        components: vec![],
        component_metrics: vec![],
        component_payload_digest: String::new(),
        items: vec![AgentItem::user_message(
            ItemId::from_string("frozen-input".into()),
            "frozen context🙂".repeat(8_000),
        )],
        host_id: runtime.session.host.catalog.host_id.0.clone(),
        host_catalog_revision: runtime.session.host.catalog.revision,
        host_availability_epoch: runtime.session.host.catalog.availability_epoch,
        capability_projection: Default::default(),
        runtime_snapshot: Default::default(),
    });
    runtime.session.execution.active_turn = Some(Box::new(AgentTurnState::new(
        TurnId::from_string("owned-turn".into()),
        context,
    )));
    runtime
}

#[test]
fn moved_state_keeps_independent_heap_owned_frozen_boundaries() {
    let runtime = active_runtime();
    let state = runtime.session;
    let configuration = state.configuration.as_ref() as *const AgentSessionConfiguration;
    let turn = state.execution.active_turn.as_deref().unwrap() as *const AgentTurnState;
    let context = state
        .execution
        .active_turn
        .as_ref()
        .unwrap()
        .context
        .as_ref() as *const AgentTurnContext;
    let frozen_configuration = state
        .execution
        .active_turn
        .as_ref()
        .unwrap()
        .context
        .configuration
        .as_ref() as *const AgentSessionConfiguration;
    let mut moved = std::hint::black_box(state);
    assert_eq!(moved.configuration.as_ref() as *const _, configuration);
    assert_eq!(
        moved.execution.active_turn.as_deref().unwrap() as *const _,
        turn
    );
    assert_eq!(
        moved
            .execution
            .active_turn
            .as_ref()
            .unwrap()
            .context
            .as_ref() as *const _,
        context
    );
    assert_eq!(
        moved
            .execution
            .active_turn
            .as_ref()
            .unwrap()
            .context
            .configuration
            .as_ref() as *const _,
        frozen_configuration
    );
    let cloned = moved.clone();
    assert_ne!(cloned.configuration.as_ref() as *const _, configuration);
    assert_ne!(
        cloned.execution.active_turn.as_deref().unwrap() as *const _,
        turn
    );
    moved.configuration.tools.clear();
    assert_eq!(cloned.configuration.tools.len(), 1);
    assert_eq!(
        moved
            .execution
            .active_turn
            .as_ref()
            .unwrap()
            .context
            .configuration
            .tools
            .len(),
        1,
        "current configuration mutation must not replace a frozen Turn's owner"
    );
    assert_eq!(
        std::mem::size_of::<Option<Box<AgentTurnState>>>(),
        std::mem::size_of::<usize>()
    );
}

#[test]
fn frozen_host_checkpoint_keeps_inline_json_and_repeatedly_cold_restores_on_bounded_stack() {
    let runtime = active_runtime();
    let config = runtime.config.clone();
    let checkpoint = runtime.checkpoint(CheckpointReason::PermissionTransportInterrupted);
    checkpoint.validate_state_identity().unwrap();
    let wire = serde_json::to_string(&checkpoint).unwrap();
    let expected = serde_json::to_value(&checkpoint).unwrap();
    assert_eq!(expected["schema_version"], AGENT_CHECKPOINT_SCHEMA_VERSION);
    assert!(expected["state"]["configuration"].is_object());
    let context = &expected["state"]["execution"]["active_turn"]["context"];
    assert!(context.is_object());
    assert!(context["configuration"].is_object());
    // The old inline DTO remains directly decodable; no wrapper or schema
    // migration can hide a lost frozen field.
    let inline: AgentSessionConfiguration =
        serde_json::from_value(context["configuration"].clone()).unwrap();
    assert_eq!(
        serde_json::to_value(inline).unwrap(),
        context["configuration"]
    );
    let restored = std::thread::Builder::new()
        .name("owned-checkpoint-small-stack".into())
        .stack_size(1024 * 1024)
        .spawn(move || {
            let mut wire = wire;
            for _ in 0..3 {
                let decoded: CoreCheckpoint<AgentThreadState> =
                    serde_json::from_str(&wire).unwrap();
                decoded.validate_state_identity().unwrap();
                let runtime = AgentRuntime::restore(config.clone(), decoded).unwrap();
                let checkpoint =
                    runtime.into_checkpoint(CheckpointReason::PermissionTransportInterrupted);
                assert_eq!(serde_json::to_value(&checkpoint).unwrap(), expected);
                wire = serde_json::to_string(&checkpoint).unwrap();
            }
            serde_json::from_str::<Value>(&wire).unwrap()
        })
        .unwrap()
        .join()
        .unwrap();
    assert!(
        restored["state"]["execution"]["active_turn"]["context"]["configuration"]["turn_identity"]
            ["host_invocation"]
            .is_object()
    );
}
