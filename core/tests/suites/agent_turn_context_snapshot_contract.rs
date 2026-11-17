use std::collections::BTreeMap;

use nola_core::agent::host::{
    CapabilityAvailability, HostCancellationMode, HostCapabilityCatalog, HostCapabilityDescriptor,
    HostCapabilityExposure, HostCapabilityParallelism, HostCapabilityRoute, HostId,
    HostOutputPolicy, HostReplaySafety, HostRequest, HostRequestEnvelope, HostResponse,
    HostResponseEnvelope,
};
use nola_core::agent::recovery::{CheckpointReason, CoreCheckpoint};
use nola_domain::core_event::{CoreEventEnvelope};
use nola_core::agent::{
    compose_agent_prompt, AgentCore, AgentCoreConfig, AgentInputDelivery, AgentItem,
    AgentPromptFragment, AgentPromptLayer, AgentPromptTrust, AgentSubmission,
    AgentTurnContextSnapshot, AgentTurnIdentity, AgentWorkspaceIdentity, ItemId, SessionId,
};
use nola_core::agent::{AgentInstructionIdentity, AgentInstructionKind, AgentInstructionManifest};
use nola_core::agent::{AgentPromptCompositionIdentity, AgentThreadState};
use nola_core::agent::{ModelCapabilityProfile, ModelInvocationPlan};
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

fn catalog(revision: u64) -> HostCapabilityCatalog {
    let mut capabilities = BTreeMap::new();
    for (id, route) in [("model.stream", HostCapabilityRoute::Model)] {
        capabilities.insert(id.to_string(), capability(id, route));
    }
    HostCapabilityCatalog {
        host_id: HostId("turn-context-host".to_string()),
        revision,
        availability_epoch: 1,
        lease: None,
        capabilities,
    }
}

fn turn_identity() -> AgentTurnIdentity {
    let composition = compose_agent_prompt([
        AgentPromptFragment {
            id: "identity-v1".to_string(),
            layer: AgentPromptLayer::Identity,
            trust: AgentPromptTrust::HostFact,
            cacheable: true,
            content: "Fixture Host identity".to_string(),
        },
        AgentPromptFragment {
            id: "tool-policy-v3".to_string(),
            layer: AgentPromptLayer::ExecutionPolicy,
            trust: AgentPromptTrust::HostInstruction,
            cacheable: true,
            content: "Fixture Host workflow".to_string(),
        },
    ])
    .expect("compose fixture prompt identity");
    AgentTurnIdentity {
        delegation: None, fresh_resend_policy: None, host_dependencies: None,
        remote_tool_plan: None,
        host_recovery:Default::default(),plugin_revisions:Default::default(),
        supports_reviewed_text_parts: false,
        host_invocation: None,
        locale: Some("zh-Hans".to_string()),
        workspace: Some(AgentWorkspaceIdentity {
            workspace_id: "workspace-alpha".to_string(),
            root_digest: "sha256:workspace-root-digest".to_string(),
            summary: Some("Alpha workspace".to_string()),
        }),
        response_style_id: Some("balanced".to_string()),
        model_capability_id: Some("reasoning-tools-v2".to_string()),
        model_capability_revision: 7,
        skill_catalog_revision: 11,
        skill_invocation_revision: 3,
        memory_policy_revision: 5,
        permission_policy_revision: 9,
        prompt: AgentPromptCompositionIdentity {
            stable_prefix_digest: composition.stable_prefix_sha256,
            fragment_ids: composition.fragment_ids,
            stable_fragment_digests: composition.stable_fragment_sha256,
        },
        instruction_manifest: AgentInstructionManifest {
            active_scope_uri: "/workspace-alpha".to_string(),
            sources: vec![AgentInstructionIdentity {
                source_id: "repo-guidance".to_string(),
                kind: AgentInstructionKind::UserInstruction,
                scope_uri: "/workspace-alpha".to_string(),
                scope_depth: 1,
                origin_uri: "/workspace-alpha/AGENTS.md".to_string(),
                content_digest: "sha256:repo-guidance".to_string(),
                observed_at: Some("2026-08-06T00:00:00Z".to_string()),
                selection_reason: "active_workspace_scope".to_string(),
                cacheable: false,
            }],
            digest: "sha256:instruction-manifest".to_string(),
        },
        model_capability: ModelCapabilityProfile {
            profile_id: "fixture-capability".to_string(),
            revision: 2,
            instruction_profile: nola_core::agent::ModelInstructionProfile::OpenAi,
            native_tool_calls: true,
            parallel_tool_calls: true,
            reasoning: true,
            reasoning_continuity: true,
            prompt_cache: true,
            vision_input: true,
            native_pdf_input: false,
            context_window_tokens: Some(128_000),
            max_output_tokens: Some(8_192),
            request_continuation: true,
            max_request_continuations: 2,
            stream_text: true,
            stream_reasoning: true,
            stream_tool_calls: true,
            direct_tool_capability_ids: Vec::new(),
        },
        execution_backend: Default::default(),
    }
}

fn config() -> AgentCoreConfig {
    AgentCoreConfig {
        session_id: Some(SessionId::from_string("turn-context-session".to_string())),
        model: Some("fixture-model".to_string()),
        host_catalog: Some(catalog(1)),
        turn_identity: turn_identity(),
        ..AgentCoreConfig::default()
    }
}

fn start_turn(core: &mut AgentCore) -> nola_core::agent::AgentTransition {
    core.submit(AgentSubmission::UserInput {
        items: vec![AgentItem::user_message(
            ItemId::from_string("turn-context-user".to_string()),
            "Verify the immutable Turn context.",
        )],
        turn_context_items: Vec::new(),
        turn_context_platform: Default::default(),
        turn_context_components: Vec::new(),
        submission_id: Some("turn-context-submission".to_string()),
        submitted_at_epoch_ms: None,
        delivery: AgentInputDelivery::CurrentTurn,
    })
    .expect("start immutable-context Turn")
}

fn model_request(requests: &[HostRequestEnvelope]) -> HostRequestEnvelope {
    requests
        .iter()
        .find(|request| matches!(request.payload, HostRequest::ModelStream { .. }))
        .cloned()
        .expect("model request")
}

fn model_snapshot(request: &HostRequestEnvelope) -> AgentTurnContextSnapshot {
    let HostRequest::ModelStream { settings, .. } = &request.payload else {
        panic!("expected model request")
    };
    serde_json::from_value(settings["step_context"]["turn_context"].clone())
        .expect("typed Turn snapshot in model settings")
}

fn model_settings(request: &HostRequestEnvelope) -> &serde_json::Value {
    let HostRequest::ModelStream { settings, .. } = &request.payload else {
        panic!("expected model request")
    };
    settings
}

fn checkpoint(core: &AgentCore) -> CoreCheckpoint<AgentThreadState> {
    core.checkpoint(CheckpointReason::Manual)
}

fn response_for(
    request: &HostRequestEnvelope,
    sequence: u64,
    payload: HostResponse,
) -> HostResponseEnvelope {
    HostResponseEnvelope {
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
    }
}

fn public_events(events: &[CoreEventEnvelope<nola_core::agent::AgentEvent>]) -> Vec<Value> {
    events
        .iter()
        .map(|event| serde_json::to_value(&event.event).expect("serialize public event"))
        .collect()
}

#[test]
fn accepted_model_completion_keeps_request_and_attempt_cache_provenance_after_effect_removal() {
    let runtime_config = config();
    let mut core = AgentCore::try_new(runtime_config.clone()).unwrap();
    let started = start_turn(&mut core);
    let request = model_request(&started.host_requests);
    let HostRequest::ModelStream { text_part_id, .. } = &request.payload else { unreachable!() };
    let cache = nola_domain::PromptCacheUsage::measured(Some(1000), Some(750), None, false);
    let diagnostic = nola_domain::PromptCacheDiagnostic {
        cache_usage: Some(cache), schema_version: 1,
        status: nola_domain::PromptCacheDiagnosticStatus::Hit,
        identity_stable: true, identity_sha256: Some("a".repeat(64)),
        reasons: vec![nola_domain::PromptCacheDiagnosticReason::ProviderReportedCacheRead],
        fragment_changes: Vec::new(), cache_read_input_tokens: Some(750), cache_write_input_tokens: None,
    };
    let usage = nola_core::agent::TokenUsage {
        input_tokens: Some(1000), output_tokens: Some(10), total_tokens: Some(1010),
        cached_input_tokens: Some(750),         cache_usage: Some(cache), prompt_cache_diagnostic: Some(diagnostic.clone()),
        ..Default::default()
    };
    let mut response = response_for(&request, 1, HostResponse::ModelCompleted {
        items: vec![json!({"type":"message", "id":text_part_id, "role":"assistant", "content":"Verified answer."})],
        tool_calls: Vec::new(), usage: serde_json::to_value(usage).unwrap(),
    });
    response.attempt_id = Some("physical-cache-attempt".into());
    let completed = core.submit(AgentSubmission::HostResponse { response }).unwrap();
    let expected = nola_domain::ContextUsageMeasurement {
        request_id: request.request_id.clone(), attempt_id: Some("physical-cache-attempt".into()), context_generation: Some(0),
    };
    let events = public_events(&completed.event_envelopes);
    let measured = events.iter().find(|event| event["type"] == "context_updated"
        && event["prompt_cache_measurement"]["request_id"] == request.request_id).expect("correlated cache context event");
    // Passive transcript refresh retains the Provider measurement until a new
    // model projection actually replaces it with an estimate.
    assert_eq!(measured["measurement"], serde_json::to_value(&expected).unwrap());
    assert_eq!(measured["usage_is_estimated"], false);
    assert_eq!(measured["prompt_cache_measurement"], serde_json::to_value(&expected).unwrap());
    assert_eq!(measured["prompt_cache_diagnostic"], serde_json::to_value(&diagnostic).unwrap());
    let saved = checkpoint(&core);
    assert!(!saved.state.host.pending_effects.iter().any(|effect| effect.effect_id == request.effect_id.0));
    let restored = AgentCore::restore(runtime_config, saved).unwrap();
    let cold = checkpoint(&restored).state.conversation.context_window;
    assert_eq!(cold.measurement, Some(expected.clone()));
    assert!(!cold.usage_snapshot().usage_is_estimated);
    assert_eq!(cold.prompt_cache_measurement, Some(expected));
    assert_eq!(cold.cumulative_usage.prompt_cache_diagnostic, Some(diagnostic));
}

#[test]
fn active_turn_keeps_frozen_identity_when_host_catalog_advances() {
    let mut core = AgentCore::try_new(config()).expect("create Core");
    let started = start_turn(&mut core);
    let first_model = model_request(&started.host_requests);
    let frozen = model_snapshot(&first_model);
    let model_prefix_digest = model_settings(&first_model)["step_context"]["model_prefix_digest"]
        .as_str()
        .expect("Core-owned model prefix digest in Host settings");
    assert_eq!(model_prefix_digest.len(), 64);
    assert!(model_prefix_digest
        .bytes()
        .all(|byte| byte.is_ascii_hexdigit()));
    assert_ne!(model_prefix_digest, frozen.stable_prefix_digest);
    let request_snapshot_digest = model_settings(&first_model)["step_context"]
        ["request_snapshot_digest"]
        .as_str()
        .expect("Core-owned request snapshot digest in Host settings");
    assert_eq!(request_snapshot_digest.len(), 64);
    assert!(request_snapshot_digest
        .bytes()
        .all(|byte| byte.is_ascii_hexdigit()));
    assert_ne!(request_snapshot_digest, model_prefix_digest);
    let invocation_plan: ModelInvocationPlan =
        serde_json::from_value(model_settings(&first_model)["model_invocation_plan"].clone())
            .expect("Core-owned model invocation plan in Host settings");
    assert_eq!(invocation_plan.profile_id, "fixture-capability");
    assert_eq!(
        invocation_plan.instruction_profile,
        nola_core::agent::ModelInstructionProfile::OpenAi
    );
    let HostRequest::ModelStream { tools, .. } = &first_model.payload else {
        panic!("expected model stream")
    };
    assert_eq!(tools.iter().map(|tool| tool["function"]["name"].as_str().unwrap())
        .collect::<Vec<_>>(), ["requirements_update", "task_scope_update"],
        "The current requirement scope exposes exactly its Core control tools, not any Host tool");
    assert!(invocation_plan.native_tool_calls);
    assert!(invocation_plan.parallel_tool_calls);
    assert!(invocation_plan.reasoning_continuity);
    assert!(invocation_plan.prompt_cache);
    assert_eq!(invocation_plan.max_request_continuations, 2);
    assert_eq!(invocation_plan.semantic_digest.len(), 64);
    let HostRequest::ModelStream { messages, .. } = &first_model.payload else {
        panic!("expected model stream")
    };
    assert!(messages.iter().any(|message| {
        message
            .get("content")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|content| content.contains("<model_compatibility profile=\"openai\">"))
    }));
    assert_eq!(frozen.tool_catalog_revision, 1);
    assert_eq!(frozen.model_id.as_deref(), Some("fixture-model"));
    assert_eq!(frozen.locale.as_deref(), Some("zh-Hans"));
    assert_eq!(frozen.skill_catalog_revision, 11);
    assert_eq!(frozen.skill_invocation_revision, 3);
    assert_eq!(frozen.memory_policy_revision, 5);
    assert_eq!(frozen.permission_policy_revision, 9);
    assert_eq!(
        frozen.prompt_fragment_ids.first().map(String::as_str),
        Some("identity-v1")
    );
    assert!(frozen
        .prompt_fragment_ids
        .iter()
        .any(|fragment_id| fragment_id == "tool-policy-v3"));
    assert_eq!(
        frozen.instruction_manifest,
        turn_identity().instruction_manifest
    );
    assert_eq!(frozen.model_capability, turn_identity().model_capability);

    core.submit(AgentSubmission::UpdateHostCatalog {
        catalog: catalog(2),
    })
    .expect("advance live Host catalog");

    let continued = core
        .submit(AgentSubmission::HostResponse {
            response: response_for(
                &first_model,
                1,
                HostResponse::ModelCompleted {
                    items: vec![json!({
                        "type": "reasoning",
                        "id": "reasoning-only",
                        "summary": "Need one continuation",
                        "content": "Need one continuation"
                    })],
                    tool_calls: Vec::new(),
                    usage: json!({"input_tokens": 10, "output_tokens": 2, "total_tokens": 12}),
                },
            ),
        })
        .expect("request bounded reasoning continuation");
    let second_model = model_request(&continued.host_requests);
    assert_eq!(
        second_model.catalog_revision, 1,
        "all model rounds in one Turn use the frozen Host revision"
    );
    assert_eq!(
        model_snapshot(&second_model),
        frozen,
        "later model rounds must use the Turn-start snapshot"
    );

    let mut changed_identity = turn_identity();
    changed_identity.locale = Some("en".to_string());
    let error = core
        .submit(AgentSubmission::UpdateSessionConfiguration {
            patch: nola_core::agent::AgentSessionConfigurationPatch {
                turn_identity: Some(changed_identity),
                ..Default::default()
            },
        })
        .expect_err("active Turn must reject mutable identity updates");
    assert!(error.contains("only change while the thread is idle"));
}

#[test]
fn checkpoint_restore_replays_the_same_turn_and_model_snapshot() {
    let runtime_config = config();
    let mut original = AgentCore::try_new(runtime_config.clone()).expect("create Core");
    let started = start_turn(&mut original);
    let original_model = model_request(&started.host_requests);
    let original_snapshot = model_snapshot(&original_model);
    let checkpoint = checkpoint(&original);
    let frozen_in_checkpoint = checkpoint
        .state
        .execution
        .active_turn
        .as_ref()
        .expect("active Turn in replay-safe checkpoint")
        .context
        .runtime_snapshot
        .clone();
    assert_eq!(frozen_in_checkpoint, original_snapshot);

    let serialized = serde_json::to_string(&checkpoint).expect("serialize checkpoint");
    assert!(!serialized.contains("Fixture Host identity"));
    assert!(!serialized.contains("Fixture Host workflow"));
    assert!(!serialized.contains("/Users/"));

    let mut restored = AgentCore::restore(runtime_config, checkpoint).expect("restore Core");
    let resumed = restored
        .submit(AgentSubmission::ResumeHostEffect {
            effect_id: original_model.effect_id.0.clone(),
        })
        .expect("explicitly resume pending model effect");
    let restored_model = model_request(&resumed.host_requests);
    assert_eq!(restored_model.effect_id, original_model.effect_id);
    assert_eq!(model_snapshot(&restored_model), original_snapshot);

    let delta = HostResponse::ModelDelta {
        channel: nola_core::agent::host::ModelDeltaChannel::Text,
        part_id: match &original_model.payload {
            HostRequest::ModelStream { text_part_id, .. } => text_part_id.clone(),
            _ => unreachable!(),
        },
        delta: "Replay-stable text".to_string(),
    };
    let original_transition = original
        .submit(AgentSubmission::HostResponse {
            response: response_for(&original_model, 1, delta.clone()),
        })
        .expect("original model delta");
    let restored_transition = restored
        .submit(AgentSubmission::HostResponse {
            response: response_for(&restored_model, 1, delta),
        })
        .expect("replayed model delta");
    assert_eq!(
        public_events(&original_transition.event_envelopes),
        public_events(&restored_transition.event_envelopes),
        "same checkpoint and correlated response must produce identical public events"
    );
}
