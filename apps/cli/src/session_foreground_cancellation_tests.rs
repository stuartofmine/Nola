// SPDX-License-Identifier: Apache-2.0
//! Real Application cancellation admission; no mocked ACK or terminal seal.
use super::*;
use nola_core::agent::host::*;
#[path = "../../../core/nola-application/tests/support/mod.rs"]
mod support;

fn frozen_host_binding() -> nola_host_protocol::HostInvocationBinding {
    use nola_host_protocol::*;
    let route = HostModelRouteBinding::Http(HostHttpModelRouteBinding {
        siwc_account: None,
        protocol: nola_domain::generation::ModelProtocol::ChatCompletions,
        output_token_field: ModelOutputTokenField::MaxTokens,
        provider: Some("fixture".into()), endpoint: "https://fixture.invalid/v1".into(),
        endpoint_query_sha256: None, model: "fixture".into(),
        credential_sha256: "a".repeat(64), credential_source: HostSecretSource::Ephemeral,
        endpoint_query_source: HostSecretSource::None,
    });
    let output_admission = HostOutputAdmission::admit(vec![HostRouteTokenLimits {
        route: route.clone(), max_input_tokens: None, max_output_tokens: None, max_total_tokens: None,
    }], nola_domain::generation::OutputTokenPreference::Default).unwrap();
    let binding = HostInvocationBinding {
        mac_launch: None, version: HOST_INVOCATION_BINDING_VERSION,
        primary: route, fallbacks: Vec::new(),
        generation: HostGenerationBinding {
            max_retries: 0, parallel_tool_calls: None, reasoning_selection: None,
            reasoning_tier: Default::default(), sampling_override: false,
            generation_options: nola_domain::generation::GenerationOptions::default().try_into().unwrap(),
            temperature: serde_json::Number::from_f64(0.7).unwrap(),
            top_p: serde_json::Number::from_f64(0.9).unwrap(),
            max_output_tokens: output_admission.resolved.tokens,
            thinking_enabled: false, reasoning_summary_supported: None, prompt_cache: false, prompt_cache_key_sha256: None,
            prompt_cache_key_prefix_sha256: None, prompt_cache_retention: None,
        },
        output_admission, execution_policy_sha256: "b".repeat(64),
    };
    binding.validate().unwrap();
    binding
}

fn source(path: &std::path::Path) -> SessionStore {
    source_with_config(path, |_| {})
}

pub(super) fn source_with_config(path: &std::path::Path, configure: impl FnOnce(&mut AgentCoreConfig)) -> SessionStore {
    let mut application = support::open_runtime(path).unwrap();
    // No model adapter/worker is ever constructed by this fixture. Its only
    // target is a real prepared model effect with no physical execution.
    let model = HostCapabilityDescriptor {
        id: "model.stream".into(), title: "Fixture model".into(),
        route: HostCapabilityRoute::Model, availability: CapabilityAvailability::Available,
        exposure: HostCapabilityExposure::Internal, permission_domain: None,
        input_schema: serde_json::json!({"type":"object"}), input_normalization: Default::default(),
        output_policy: HostOutputPolicy { preview_char_limit: 4000, persist_full_output: true, allow_artifacts: true },
        supports_streaming: true, supports_argument_streaming: false,
        parallelism: HostCapabilityParallelism::Serial, cancellation: HostCancellationMode::Cooperative,
        replay_safety: HostReplaySafety::Recoverable,
    };
    let mut turn_identity = support::model_identity::identity();
    // Freeze the same explicit fixture binding in the admitted Run and its
    // checkpoint, so active reads exercise strict recovery rather than an
    // unbound synthetic config. No adapter or network request is constructed.
    turn_identity.host_invocation = Some(frozen_host_binding());
    let mut config = AgentCoreConfig {
        session_id: Some(nola_core::agent::SessionId::from_string("cancel-core".into())),
        model: Some("fixture".into()), turn_identity,
        host_catalog: Some(HostCapabilityCatalog {
            host_id: HostId("fixture-host".into()), revision: 1, availability_epoch: 1,
            lease: None, capabilities: [("model.stream".into(), model)].into_iter().collect(),
        }),
        ..Default::default()
    };
    configure(&mut config);
    application.execute(ApplicationCommand::BeginAssistantTurn {
        input: BeginAssistantTurn {
            expected_configuration_revision: None, expected_parent_message_id: None,
            conversation_id: "parent".into(), conversation_title: "Cancellation".into(),
            user_message_id: "user".into(), user_part_id: "user:body".into(),
            user_text: "Fixture question".into(), user_content_json: None,
            assistant_message_id: "assistant".into(), context_checkpoint_id: None,
            core_config: config,
            submission: nola_core::agent::AgentSubmission::UserInput {
                items: vec![nola_core::agent::AgentItem::user_message(
                    nola_core::agent::ItemId::from_string("user".into()), "Fixture question")],
                turn_context_items: Vec::new(), turn_context_platform: Default::default(),
                turn_context_components: Vec::new(), submission_id: Some("cancel-run".into()),
                submitted_at_epoch_ms: Some(1000), delivery: nola_core::agent::AgentInputDelivery::CurrentTurn,
            },
            created_at_ms: 1000,
        },
    }).unwrap();
    let mut port = CliApplicationPort::InProcess(application);
    let active = port.active_run("parent").unwrap().unwrap();
    SessionStore {
        id: "parent".into(), initial_configuration_revision: None,
        application_port: port,
        active_canonical_turn: Some(ActiveCanonicalTurn {
            run_id: active.run_id, kind: active.kind,
            next_event_sequence: active.next_event_sequence,
            invocation_config_digest: active.invocation_config_digest,
        }),
        task_execution: None, assistant_journal_notifications: Arc::default(),
        task_result_dispatches: Default::default(),
    }
}

fn interrupt(id: &str) -> AgentBoundaryInput {
    AgentBoundaryInput::Submission {
        submission: nola_core::agent::AgentSubmission::Interrupt {
            goal_control: None, cancellation_id: id.into(), reason: Some("cancelled by user".into()),
        },
    }
}

fn checkpoint(source: &mut SessionStore) -> Value {
    serde_json::to_value(source.load_recovery_core_checkpoint().unwrap()).unwrap()
}

#[test]
fn lost_foreground_cancel_ack_reuses_the_formal_admission_without_a_second_interrupt() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("cancel.sqlite");
    let mut source = source(&path);
    let input = interrupt("same-execution-and-run");
    let first = source.accept_boundary_input(input.clone()).unwrap();
    assert_eq!(first.control.unwrap().turn_status, nola_core::agent::AgentTurnStatus::Cancelling);
    let committed = checkpoint(&mut source);
    source.application_port = CliApplicationPort::InProcess(support::open_runtime(&path).unwrap());
    // The transport lost the first ACK. Application, not a local flag or an
    // error-string match, recognizes this exact Run's existing cancellation.
    let replay = source.accept_boundary_input(input).unwrap();
    assert_eq!(replay.control.unwrap().turn_status, nola_core::agent::AgentTurnStatus::Cancelling);
    assert_eq!(checkpoint(&mut source), committed);
    let run_id = source.active_core_run_id().unwrap().to_owned();
    assert!(source.load_assistant_run_terminal_seal(&run_id).unwrap().is_none(),
        "admitting/replaying stop intent must never fabricate physical stop or seal");
    let error = source.reduce_active_core_submission("raw-duplicate".into(),
        nola_core::agent::AgentSubmission::Interrupt {
            goal_control: None, cancellation_id: "same-execution-and-run".into(), reason: None,
        }).unwrap_err();
    assert!(error.contains("already cancelling"), "Core's strict rejection remains: {error}");
    assert_eq!(checkpoint(&mut source), committed);
}

#[test]
fn invalid_cancel_identity_preserves_the_original_rejection_and_canonical_state() {
    let root = tempfile::tempdir().unwrap();
    let mut source = source(&root.path().join("invalid.sqlite"));
    let before = checkpoint(&mut source);
    for _ in 0..2 {
        let error = source.accept_boundary_input(interrupt("")).unwrap_err();
        assert!(error.contains("bounded exact identities"), "{error}");
        assert_eq!(checkpoint(&mut source), before);
    }
}

#[test]
fn retained_foreign_run_cannot_redirect_cancel_to_the_current_run() {
    let root = tempfile::tempdir().unwrap();
    let mut source = source(&root.path().join("foreign.sqlite"));
    let before = checkpoint(&mut source);
    let original = source.active_canonical_turn.clone();
    source.active_canonical_turn.as_mut().unwrap().run_id = "different-original-run".into();
    assert!(source.accept_boundary_input(interrupt("old-token"))
        .unwrap_err().contains("successor Run"));
    source.active_canonical_turn = original;
    assert_eq!(checkpoint(&mut source), before);
}

fn seal_the_never_executed_fixture(source: &mut SessionStore, cancellation_id: &str) {
    source.accept_boundary_input(interrupt(cancellation_id)).unwrap();
    let now = monotonic_epoch_millis();
    let controls = source.claim_committed_requests(RuntimeEffectDeliveryClaim {
        worker_generation: "fixture-cancel".into(), lease_owner: "fixture-cancel".into(),
        claimed_at_ms: now, lease_expires_at_ms: now + 60_000, limit: 64,
    }).unwrap();
    assert_eq!(controls.len(), 1, "the fixture created one model and never ran it");
    let request = &controls[0];
    let HostRequest::Cancel { target_effect_id, .. } = &request.payload else {
        panic!("only the formal Cancel may be delivered");
    };
    // This fixture never constructs a provider/worker and cannot have started
    // its prepared model. A real fenced Cancel delivery, not a fake seal row,
    // supplies that known NotStarted fact to Application.
    for (sequence, payload) in [
        (1, HostResponse::ExecutionStarted),
        (2, HostResponse::CancellationConfirmed { target_effect_id: target_effect_id.clone() }),
    ] {
        source.accept_raw_host_response(HostResponseEnvelope {
            request_id: request.request_id.clone(), effect_id: request.effect_id.clone(),
            host_id: request.host_id.clone(), session_id: request.session_id.clone(),
            turn_id: request.turn_id.clone(), attempt_id: request.attempt_id.clone(),
            delegated_invocation_id: request.delegated_invocation_id.clone(), lease_id: request.lease_id.clone(),
            catalog_revision: request.catalog_revision, availability_epoch: request.availability_epoch,
            sequence, delivery: request.delivery.clone(), payload,
        }).unwrap();
    }
}

#[test]
fn lost_cancel_ack_after_seal_cold_replays_terminal_control_without_reopening_the_run() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("terminal.sqlite");
    let mut source = source(&path);
    let retained = source.active_canonical_turn.clone();
    seal_the_never_executed_fixture(&mut source, "terminal-stop");
    let seal = source.load_assistant_run_terminal_seal("cancel-run").unwrap().unwrap();
    assert!(matches!(seal.result.value, nola_domain::run_result::RunResultValue::Cancelled { .. }));
    assert!(source.application_port.active_run("parent").unwrap().is_none());
    source.application_port = CliApplicationPort::InProcess(support::open_runtime(&path).unwrap());
    source.active_canonical_turn = retained.clone();
    let committed = checkpoint(&mut source);
    // Reading the terminal checkpoint retires SessionStore's active pointer;
    // the failed transport still owns the original retained Run, not a new one.
    source.active_canonical_turn = retained;
    let replay = source.accept_boundary_input(interrupt("terminal-stop")).unwrap();
    let control = replay.control.unwrap();
    assert_eq!(control.turn_status, nola_core::agent::AgentTurnStatus::Cancelled);
    assert_eq!(control.active_turn_id.as_ref().map(|turn| turn.as_str()), Some("cancel-run"));
    assert_eq!(source.load_assistant_run_terminal_seal("cancel-run").unwrap().unwrap(), seal);
    assert!(source.application_port.active_run("parent").unwrap().is_none());
    assert_eq!(checkpoint(&mut source), committed);
}

#[test]
fn sealed_cancel_retry_rejects_an_actually_admitted_successor_without_modifying_either_run() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("successor.sqlite");
    let mut source = source(&path);
    let retained = source.active_canonical_turn.clone();
    seal_the_never_executed_fixture(&mut source, "original-stop");
    let old_seal = source.load_assistant_run_terminal_seal("cancel-run").unwrap().unwrap();
    let config = source.load_host_execution_state().unwrap().configuration;
    let tail: Option<nola_application::ConversationTailSnapshot> = source.application_port.execute(
        ApplicationCommand::LoadConversationTailSnapshotV1 {
            conversation_id: "parent".into(), message_limit: 1,
            part_text_limit: 1, part_json_limit: 1,
        },
    ).unwrap();
    let tail = tail.unwrap().conversation;
    let now = monotonic_epoch_millis();
    source.application_port.begin_assistant_turn(BeginAssistantTurn {
        expected_configuration_revision: Some(tail.configuration_revision),
        expected_parent_message_id: tail.messages.last().map(|message| message.id.clone()),
        conversation_id: "parent".into(), conversation_title: "Successor".into(),
        user_message_id: "successor-user".into(), user_part_id: "successor-user:body".into(),
        user_text: "Independent successor".into(), user_content_json: None,
        assistant_message_id: "successor-assistant".into(), context_checkpoint_id: None,
        core_config: config,
        submission: nola_core::agent::AgentSubmission::UserInput {
            items: vec![nola_core::agent::AgentItem::user_message(
                nola_core::agent::ItemId::from_string("successor-user".into()), "Independent successor")],
            turn_context_items: Vec::new(), turn_context_platform: Default::default(),
            turn_context_components: Vec::new(), submission_id: Some("successor-run".into()),
            submitted_at_epoch_ms: Some(now), delivery: nola_core::agent::AgentInputDelivery::CurrentTurn,
        },
        created_at_ms: now,
    }).unwrap();
    let before = source.application_port.load_checkpoint("parent").unwrap().unwrap().checkpoint_json;
    source.active_canonical_turn = retained;
    assert!(source.accept_boundary_input(interrupt("original-stop"))
        .unwrap_err().contains("successor Run"));
    assert_eq!(source.application_port.active_run("parent").unwrap().unwrap().run_id, "successor-run");
    assert_eq!(source.application_port.load_checkpoint("parent").unwrap().unwrap().checkpoint_json, before);
    assert_eq!(source.load_assistant_run_terminal_seal("cancel-run").unwrap().unwrap(), old_seal);
}
