// SPDX-License-Identifier: Apache-2.0
//! Actual isolated Store admission/rollback; no model execution or UI claims.
mod support;
use nola_application::{agent_room as rooms, expert_message, persistent_agent as agents};
use nola_domain::{
    edited_resend::PrepareEditedResend,
    expert_message::*,
    persistent_agent::{AgentAvailability, AgentDefaults},
};
use nola_store::NolaStore;
use nola_store_port::expert_message::ExpertMessageRepository;
use serde_json::json;
fn setup() -> (NolaStore, AdmitExpertConversationMessage) {
    setup_at(":memory:")
}
fn setup_at(path: impl AsRef<std::path::Path>) -> (NolaStore, AdmitExpertConversationMessage) {
    let mut store = NolaStore::open(path).unwrap();
    store
        .upsert_model_profile(&nola_store_port::model::StoredModelProfile {
            id: "model".into(),
            display_name: "Model".into(),
            provider: "fixture".into(),
            model_name: "original-model".into(),
            runtime: "remote".into(),
            source_json: serde_json::json!({"baseURL":"https://fixture.invalid/v1","wireProtocol":"chat_completions","capabilities":["chat"],"executionCapabilities":{"toolCalls":true,"imageInput":false,"reasoning":false,"samplingParameters":true}}).to_string(),
            parameters_json: "{}".into(),
            selected: true,
            enabled: true,
            revision: 1,
            created_at_ms: 1,
            updated_at_ms: 1,
            deleted_at_ms: None,
        })
        .unwrap();
    store
        .create_conversation(
            &nola_store_port::conversation_profile::CreateConversationInput {
                id: "parent".into(),
                title: "Experts".into(),
                revision: 1,
                settings_json: Some("{}".into()),
                created_at_ms: 1,
                updated_at_ms: 1,
            },
        )
        .unwrap();
    store.begin_write_unit_of_work().unwrap();
    let association = rooms::ensure_for_conversation(&mut store, "parent".into(), 1).unwrap();
    let mut room = association.room;
    let mut directory = None;
    for id in ["first", "second"] {
        let configured = agents::configure(
            &mut store.product_setting_repository(),
            agents::ConfigureAgent {
                id: id.to_string().try_into().unwrap(),
                expected_directory_revision: directory,
                expected_agent_revision: None,
                name: id.into(),
                description: format!("Original {id} responsibility"),
                defaults: AgentDefaults::try_from(
                    json!({"modelSelectionID":"configured:model","language":"en"}),
                )
                .unwrap(),
                availability: AgentAvailability::Active,
                updated_at_ms: 1,
            },
        )
        .unwrap();
        directory = configured.revision;
        room = rooms::join(
            &mut store,
            rooms::JoinAgent {
                room_id: room.room.id().into(),
                expected_room_revision: room.revision,
                agent_id: id.to_string().try_into().unwrap(),
                expected_agent_revision: 1,
                at_ms: 1,
            },
        )
        .unwrap();
    }
    store.commit_write_unit_of_work().unwrap();
    let input = AdmitExpertConversationMessage {
        conversation_id: "parent".into(),
        expected_revision: store.load_conversation("parent").unwrap().unwrap().revision,
        room_id: room.room.id().into(),
        expected_room_revision: room.revision,
        submission_id: uuid::Uuid::new_v4().to_string(),
        user_message_id: "first-parent-user".into(),
        text: "Review this together".into(),
        selected_participants: room
            .room
            .participants()
            .iter()
            .map(|p| ExpertParticipantReference {
                participant_id: p.id.clone(),
                agent_id: p.agent_id.clone(),
                agent_revision: p.agent_revision,
                context_conversation_id: p.context_conversation_id.clone(),
            })
            .collect(),
        created_at_ms: 10,
    };
    (store, input)
}
#[test]
fn admission_freezes_complete_set_and_reuses_original_room_contexts_on_retry() {
    let (mut store, input) = setup();
    let accepted = admit(&mut store, input.clone()).unwrap();
    assert_eq!(accepted.entries.len(), 2);
    assert_eq!(
        accepted.input.selected_participants,
        input.selected_participants
    );
    assert_eq!(
        accepted
            .entries
            .iter()
            .map(|e| e.participant.context_conversation_id.clone())
            .collect::<Vec<_>>(),
        input
            .selected_participants
            .iter()
            .map(|r| r.context_conversation_id.clone())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        store
            .load_conversation("parent")
            .unwrap()
            .unwrap()
            .messages
            .len(),
        1
    );
    assert_eq!(
        rooms::submissions(&store.product_setting_repository(), &input.room_id, 50, 0)
            .unwrap()
            .len(),
        2
    );
    assert_eq!(admit(&mut store, input.clone()).unwrap(), accepted);
    assert_eq!(
        store
            .load_conversation("parent")
            .unwrap()
            .unwrap()
            .messages
            .len(),
        1
    );
    assert!(
        accepted.require_complete_source().is_err(),
        "Accepted delivery is not proof that any child model executed"
    );
    let target = uuid::Uuid::new_v4().to_string();
    let source = store.load_conversation("parent").unwrap().unwrap();
    let projection = nola_application::load_conversation_projection_v1(&store, "parent").unwrap();
    let edited = PrepareEditedResend {
        operation_id: uuid::Uuid::new_v4().to_string(),
        source_conversation_id: "parent".into(),
        source_message_id: input.user_message_id,
        expected_source_revision: source.revision,
        expected_source_message_revision: projection.messages[0].revision,
        edited_text: "Changed request".into(),
        target_conversation_id: target.clone(),
        title: "Edited experts".into(),
        created_at_ms: 20,
    };
    assert!(prepare(&mut store, edited)
        .unwrap_err()
        .contains("complete_source_witness"));
    assert!(store.load_conversation(&target).unwrap().is_none());
}
#[test]
fn stale_or_duplicate_member_rejects_whole_parent_and_all_children() {
    let (mut store, mut input) = setup();
    input.selected_participants[1].agent_revision += 1;
    assert!(admit(&mut store, input.clone()).is_err());
    assert!(store
        .load_conversation("parent")
        .unwrap()
        .unwrap()
        .messages
        .is_empty());
    assert!(store
        .expert_message_manifest("parent", &input.user_message_id)
        .unwrap()
        .is_none());
    assert!(
        rooms::submissions(&store.product_setting_repository(), &input.room_id, 50, 0)
            .unwrap()
            .is_empty()
    );
    input.selected_participants[1] = input.selected_participants[0].clone();
    assert!(admit(&mut store, input.clone()).is_err());
    assert!(store
        .load_conversation("parent")
        .unwrap()
        .unwrap()
        .messages
        .is_empty());
}
#[test]
fn ordinary_product_setting_writes_cannot_forge_private_expert_witnesses() {
    let (mut store, _) = setup();
    assert!(store
        .put_product_setting("edited_resend/expert/source/forged", "{}", 1)
        .is_err());
    assert!(store
        .product_setting("edited_resend/expert/source/forged")
        .is_err());
}

use nola_agent_engine::agent::{
    host::*, AgentCoreConfig, AgentInputDelivery, AgentItem, AgentSubmission, ItemId, SessionId,
};
use nola_application::{begin_assistant_turn, AcceptedAssistantTurn, BeginAssistantTurn};
// Application helpers execute inside the runtime's outer receipt transaction.
// Direct fixture calls preserve that production transaction contract.
fn fixture_command<T>(
    store: &mut NolaStore,
    body: impl FnOnce(&mut NolaStore) -> Result<T, String>,
) -> Result<T, String> {
    store
        .begin_write_unit_of_work()
        .map_err(|e| e.to_string())?;
    match body(store) {
        Ok(value) => {
            store
                .commit_write_unit_of_work()
                .map_err(|e| e.to_string())?;
            Ok(value)
        }
        Err(error) => {
            store
                .rollback_write_unit_of_work()
                .map_err(|e| format!("{error}; fixture rollback: {e}"))?;
            Err(error)
        }
    }
}
use nola_application::expert_template_capture::{
    HostExpertTemplateAuthority, HostExpertTemplateCaptureContext, HostExpertTemplateSelection,
    HostExpertTemplateSelectionSource,
};
fn authority() -> HostExpertTemplateAuthority {
    HostExpertTemplateAuthority {
        actor: nola_agent_engine::agent::resource_access::AgentResourcePrincipal {
            kind: nola_agent_engine::agent::resource_access::AgentResourcePrincipalKind::LocalUser,
            id: "capture-user".into(),
        },
        request_host_id: Some("capture-host".into()),
    }
}
fn host_context(
    store: &NolaStore,
    input: &AdmitExpertConversationMessage,
) -> HostExpertTemplateCaptureContext {
    let authority = authority();
    HostExpertTemplateCaptureContext {
        actor: authority.actor,
        request_host_id: authority.request_host_id,
        selections: input
            .selected_participants
            .iter()
            .map(|reference| {
                let context = store
                    .load_conversation(&reference.context_conversation_id)
                    .unwrap()
                    .unwrap();
                let settings: serde_json::Value =
                    serde_json::from_str(context.settings_json.as_deref().unwrap_or("{}")).unwrap();
                let selected = settings
                    .get("selectedPromptTemplateID")
                    .and_then(|v| v.as_str())
                    .map(str::to_owned);
                HostExpertTemplateSelection {
                    context_conversation_id: reference.context_conversation_id.clone(),
                    context_configuration_revision: context.configuration_revision,
                    model_profile: nola_host_protocol::HostModelProfileReference {
                        profile_id: "model".into(),
                        admitted_revision: store.model_profile("model").unwrap().unwrap().revision,
                    },
                    source: if selected.is_some() {
                        HostExpertTemplateSelectionSource::Explicit
                    } else {
                        HostExpertTemplateSelectionSource::NoTemplate
                    },
                    template_id: selected,
                    host_user_instructions: vec![],
                }
            })
            .collect(),
        builtins: vec![],
    }
}
fn admit_with_host(
    store: &mut NolaStore,
    input: AdmitExpertConversationMessage,
    context: &HostExpertTemplateCaptureContext,
) -> Result<ExpertMessageManifest, String> {
    fixture_command(store, |store| {
        expert_message::admit(store, input, context, 1000)
    })
}
fn admit(
    store: &mut NolaStore,
    input: AdmitExpertConversationMessage,
) -> Result<ExpertMessageManifest, String> {
    let context = host_context(store, &input);
    admit_with_host(store, input, &context)
}
fn prepare(
    store: &mut NolaStore,
    input: PrepareEditedResend,
) -> Result<PreparedExpertResend, String> {
    fixture_command(store, |store| {
        expert_message::prepare_resend(store, input, &authority(), 2000)
    })
}

fn config(owner: &str) -> AgentCoreConfig {
    let mut config = support::goal_execution::config();
    config.session_id = Some(SessionId::from_string(owner.into()));
    config.model = Some("original-model".into());
    config.turn_identity.model_capability = nola_agent_engine::agent::ModelCapabilityProfile {
        profile_id: "original-intrinsic".into(),
        revision: 7,
        native_tool_calls: true,
        context_window_tokens: Some(64_000),
        max_output_tokens: Some(16_384),
        stream_text: true,
        ..Default::default()
    };
    config.turn_identity.model_capability_id = Some("original-intrinsic".into());
    config.turn_identity.model_capability_revision = 7;
    config.context_window.context_window_tokens = 64_000;
    config.context_window.reserved_output_tokens = 16_384;
    config.context_window.prompt_token_limit = Some(48_000);
    config.context_window.auto_compact_tokens = Some(48_000);
    config.context_window.compaction_target_tokens = Some(24_000);
    config.tools.clear();
    config.turn_identity.remote_tool_plan = None;
    config.turn_identity.host_recovery = None;
    config.host_catalog = Some(HostCapabilityCatalog {
        host_id: HostId("host".into()),
        revision: 1,
        availability_epoch: 1,
        lease: None,
        capabilities: std::collections::BTreeMap::from([(
            "model.stream".into(),
            HostCapabilityDescriptor {
                id: "model.stream".into(),
                title: "Fixture model".into(),
                route: HostCapabilityRoute::Model,
                availability: CapabilityAvailability::Available,
                exposure: HostCapabilityExposure::Internal,
                permission_domain: None,
                input_schema: json!({"type":"object"}),
                input_normalization: Default::default(),
                output_policy: HostOutputPolicy {
                    preview_char_limit: 4000,
                    persist_full_output: true,
                    allow_artifacts: false,
                },
                supports_streaming: true,
                supports_argument_streaming: false,
                parallelism: HostCapabilityParallelism::Serial,
                cancellation: HostCancellationMode::Cooperative,
                replay_safety: HostReplaySafety::Recoverable,
            },
        )]),
    });
    config
}
fn begin_real_child(
    store: &mut NolaStore,
    owner: &str,
    run: &str,
    text: &str,
    config: AgentCoreConfig,
    components: Vec<nola_agent_engine::context_engine::ContextComponent>,
) -> Result<AcceptedAssistantTurn, String> {
    begin_using_submission(
        store,
        owner,
        run,
        text,
        config,
        resend_submission(run, text, components),
    )
}
fn begin_using_submission(
    store: &mut NolaStore,
    owner: &str,
    run: &str,
    text: &str,
    config: AgentCoreConfig,
    submission: AgentSubmission,
) -> Result<AcceptedAssistantTurn, String> {
    let frontier = support::admission_frontier::read(store, owner);
    begin_assistant_turn(
        store,
        BeginAssistantTurn {
            conversation_id: owner.into(),
            expected_configuration_revision: frontier.0,
            expected_parent_message_id: frontier.1,
            conversation_title: "Resend fixture".into(),
            user_message_id: format!("{run}:user"),
            user_part_id: format!("{run}:user:text"),
            user_text: text.into(),
            user_content_json: None,
            assistant_message_id: format!("{run}:assistant"),
            context_checkpoint_id: None,
            core_config: config,
            created_at_ms: 10,
            submission,
        },
    )
}
fn resend_submission(
    run: &str,
    text: &str,
    components: Vec<nola_agent_engine::context_engine::ContextComponent>,
) -> AgentSubmission {
    AgentSubmission::UserInput {
        items: vec![AgentItem::user_message(
            ItemId::from_string(format!("{run}:user")),
            text,
        )],
        turn_context_items: vec![],
        turn_context_platform: Default::default(),
        turn_context_components: components,
        submission_id: Some(run.into()),
        submitted_at_epoch_ms: Some(10),
        delivery: AgentInputDelivery::CurrentTurn,
    }
}

#[test]
fn selected_reservation_blocks_a_second_parent_before_any_child_run() {
    let (mut store, input) = setup();
    let manifest = admit(&mut store, input.clone()).unwrap();
    let mut second = input.clone();
    second.submission_id = uuid::Uuid::new_v4().to_string();
    second.user_message_id = "second-parent".into();
    second.expected_revision = store.load_conversation("parent").unwrap().unwrap().revision;
    assert!(admit(&mut store, second)
        .unwrap_err()
        .contains("reserved_by_another_parent"));
    assert_eq!(
        store
            .load_conversation("parent")
            .unwrap()
            .unwrap()
            .messages
            .len(),
        1
    );
    assert_eq!(
        store
            .expert_message_by_submission("parent", &input.submission_id)
            .unwrap()
            .unwrap(),
        manifest
    );
    assert_eq!(
        rooms::submissions(&store.product_setting_repository(), &input.room_id, 100, 0)
            .unwrap()
            .len(),
        2
    );
}
#[test]
fn actual_begin_atomically_binds_room_owner_and_partial_source_stays_incomplete() {
    let (mut store, input) = setup();
    let manifest = admit(&mut store, input.clone()).unwrap();
    let entry = &manifest.entries[0];
    let context = &entry.participant.context_conversation_id;
    let accepted = begin_real_child(
        &mut store,
        context,
        &entry.child_submission_id,
        &input.text,
        config(context),
        vec![],
    )
    .unwrap();
    assert_eq!(accepted.begin.run_id, entry.child_submission_id);
    let receipt = rooms::submission(
        &store.product_setting_repository(),
        &input.room_id,
        &entry.child_submission_id,
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        receipt.execution_id.as_deref(),
        Some(entry.child_submission_id.as_str())
    );
    assert_eq!(receipt.state, rooms::RoomSubmissionState::Executing);
    assert!(
        rooms::execution_intent(
            &store.product_setting_repository(),
            &input.room_id,
            &entry.child_submission_id
        )
        .unwrap()
        .is_none(),
        "Real admitted ownership does not manufacture a replayable Host request"
    );
    let observed = store
        .expert_message_manifest("parent", &input.user_message_id)
        .unwrap()
        .unwrap();
    assert!(observed.entries[0].model_witness.is_some());
    assert!(observed.entries[0].instruction_witness.is_some());
    assert!(observed.entries[1].model_witness.is_none());
    assert!(observed.require_complete_source().is_err());
    let claim = fixture_command(&mut store, |store| {
        rooms::claim_execution(
            store,
            &input.room_id,
            &entry.child_submission_id,
            receipt.revision,
            20,
        )
    })
    .unwrap();
    assert!(
        !claim.dispatch_permitted,
        "The actual Run owns all subsequent effect delivery"
    );
}
#[test]
fn a_reserved_context_rejects_foreign_begin_without_leaving_run_or_transcript() {
    let (mut store, input) = setup();
    let manifest = admit(&mut store, input.clone()).unwrap();
    let entry = &manifest.entries[0];
    let context = &entry.participant.context_conversation_id;
    let foreign = uuid::Uuid::new_v4().to_string();
    assert!(begin_real_child(
        &mut store,
        context,
        &foreign,
        &input.text,
        config(context),
        vec![]
    )
    .unwrap_err()
    .contains("reserved_by_another_parent"));
    assert!(store.active_run(context).unwrap().is_none());
    assert!(store
        .load_conversation(context)
        .unwrap()
        .unwrap()
        .messages
        .is_empty());
    assert!(store
        .expert_message_manifest("parent", &input.user_message_id)
        .unwrap()
        .unwrap()
        .entries[0]
        .model_witness
        .is_none());
    let receipt = rooms::submission(
        &store.product_setting_repository(),
        &input.room_id,
        &entry.child_submission_id,
    )
    .unwrap()
    .unwrap();
    assert_eq!(receipt.state, rooms::RoomSubmissionState::Selected);
    assert!(receipt.execution_id.is_none());
}
#[test]
fn native_factory_failure_retries_only_same_durable_intent_then_observes_actual_begin() {
    let (mut store, input) = setup();
    let manifest = admit(&mut store, input.clone()).unwrap();
    let entry = &manifest.entries[0];
    let receipt = rooms::submission(
        &store.product_setting_repository(),
        &input.room_id,
        &entry.child_submission_id,
    )
    .unwrap()
    .unwrap();
    let intent = rooms::RoomExecutionIntent {
        execution_id: entry.child_submission_id.clone(),
        envelope: rooms::RoomExecutionEnvelope::Native {
            platform: "android".into(),
            conversation_configuration_revision: entry.context_configuration_revision,
            model_profile_id: "model".into(),
            model_profile_revision: 1,
            expected_parent_message_id: None,
            text: input.text.clone(),
        },
    };
    let prepared = fixture_command(&mut store, |store| {
        rooms::prepare_execution(
            store,
            &input.room_id,
            &entry.child_submission_id,
            receipt.revision,
            intent.clone(),
            11,
        )
    })
    .unwrap();
    let first = fixture_command(&mut store, |store| {
        rooms::claim_execution(
            store,
            &input.room_id,
            &entry.child_submission_id,
            prepared.revision,
            12,
        )
    })
    .unwrap();
    let retry = fixture_command(&mut store, |store| {
        rooms::claim_execution(
            store,
            &input.room_id,
            &entry.child_submission_id,
            prepared.revision,
            13,
        )
    })
    .unwrap();
    assert!(first.dispatch_permitted && retry.dispatch_permitted);
    assert_eq!(first.submission.execution_id, retry.submission.execution_id);
    let mut changed = intent;
    changed.execution_id = uuid::Uuid::new_v4().to_string();
    assert!(
        fixture_command(&mut store, |store| rooms::prepare_execution(
            store,
            &input.room_id,
            &entry.child_submission_id,
            retry.submission.revision,
            changed,
            14
        ))
        .is_err()
    );
    let context = &entry.participant.context_conversation_id;
    begin_real_child(
        &mut store,
        context,
        &entry.child_submission_id,
        &input.text,
        config(context),
        vec![],
    )
    .unwrap();
    let observed = fixture_command(&mut store, |store| {
        rooms::claim_execution(
            store,
            &input.room_id,
            &entry.child_submission_id,
            retry.submission.revision,
            15,
        )
    })
    .unwrap();
    assert!(!observed.dispatch_permitted);
}

fn host_intent(
    manifest: &ExpertMessageManifest,
) -> (
    rooms::RoomExecutionIntent,
    nola_domain::host_tool_plan::HostToolPlanSnapshot,
) {
    let entry = &manifest.entries[0];
    let plan = nola_domain::host_tool_plan::HostToolPlanSnapshot {
        schema_version: 1,
        reference: nola_domain::host_tool_plan::HostToolPlanReference {
            manifest_id: uuid::Uuid::new_v4().to_string(),
            host_id: "host".into(),
            client_id: format!("room:{}", manifest.input.room_id),
            session_id: entry.participant.context_conversation_id.clone(),
            catalog_revision: "a".repeat(64),
        },
        issued_at_ms: 10,
        expires_at_ms: 300010,
        tools: vec![],
    };
    let mut request = rooms::RoomHostRequest {
        expected_host_id: "host".into(),
        client_id: plan.reference.client_id.clone(),
        conversation_id: entry.participant.context_conversation_id.clone(),
        submission_id: entry.child_submission_id.clone(),
        user_message_id: entry.child_user_message_id.clone(),
        assistant_message_id: entry.child_assistant_message_id.clone(),
        expected_configuration_revision: Some(entry.context_configuration_revision),
        expected_parent_message_id: None,
        execution_backend: "nola".into(),
        tool_plan_ref: plan.reference.clone(),
        messages: vec![rooms::RoomExecutionMessage {
            role: "user".into(),
            content: manifest.input.text.clone(),
        }],
        request_digest: String::new(),
        model_intent: serde_json::from_value(
            json!({"profile_id":"model","expected_revision":1,"tier":"balanced","overrides":{}}),
        )
        .unwrap(),
    };
    request.request_digest = nola_domain::remote_submission::submission_request_digest(
        &request.semantic_request().unwrap(),
    )
    .unwrap();
    (
        rooms::RoomExecutionIntent {
            execution_id: entry.child_submission_id.clone(),
            envelope: rooms::RoomExecutionEnvelope::Host {
                request,
                tool_plan: plan.clone(),
                model_profile_id: "model".into(),
                model_profile_revision: 1,
            },
        },
        plan,
    )
}
#[test]
fn host_cold_retry_requires_source_owned_plan_and_same_request_receipt() {
    use nola_store_port::prepared_host_tool_plan::PreparedHostToolPlanRepository;
    let (mut store, input) = setup();
    let manifest = admit(&mut store, input.clone()).unwrap();
    let entry = &manifest.entries[0];
    let receipt = rooms::submission(
        &store.product_setting_repository(),
        &input.room_id,
        &entry.child_submission_id,
    )
    .unwrap()
    .unwrap();
    let (intent, plan) = host_intent(&manifest);
    let prepared = fixture_command(&mut store, |store| {
        rooms::prepare_execution(
            store,
            &input.room_id,
            &entry.child_submission_id,
            receipt.revision,
            intent.clone(),
            11,
        )
    })
    .unwrap();
    assert!(fixture_command(&mut store, |store| rooms::claim_execution(
        store,
        &input.room_id,
        &entry.child_submission_id,
        prepared.revision,
        12
    ))
    .unwrap_err()
    .contains("source_tool_plan_unavailable"));
    store.persist_prepared_host_tool_plan(&plan, 10).unwrap();
    let claim = fixture_command(&mut store, |store| {
        rooms::claim_execution(
            store,
            &input.room_id,
            &entry.child_submission_id,
            prepared.revision,
            13,
        )
    })
    .unwrap();
    assert!(claim.dispatch_permitted);
    let rooms::RoomExecutionEnvelope::Host { request, .. } = intent.envelope else {
        unreachable!()
    };
    let identity = nola_domain::remote_submission::RemoteSubmissionIdentity {
        source_host_id: request.expected_host_id.clone(),
        client_id: request.client_id.clone(),
        conversation_id: request.conversation_id.clone(),
        submission_id: request.submission_id.clone(),
        user_message_id: request.user_message_id.clone(),
        assistant_message_id: request.assistant_message_id.clone(),
    };
    let registered = store
        .register_source_submission(&identity, &request.semantic_request().unwrap())
        .unwrap();
    assert!(registered.created);
    let retry = fixture_command(&mut store, |store| {
        rooms::claim_execution(
            store,
            &input.room_id,
            &entry.child_submission_id,
            claim.submission.revision,
            14,
        )
    })
    .unwrap();
    assert!(retry.dispatch_permitted);
    assert_eq!(retry.submission.execution_id, claim.submission.execution_id);
    let mut actual = config(&entry.participant.context_conversation_id);
    actual.turn_identity.remote_tool_plan = Some(plan);
    begin_real_child(
        &mut store,
        &entry.participant.context_conversation_id,
        &entry.child_submission_id,
        &input.text,
        actual,
        vec![],
    )
    .unwrap();
    let observed = fixture_command(&mut store, |store| {
        rooms::claim_execution(
            store,
            &input.room_id,
            &entry.child_submission_id,
            retry.submission.revision,
            15,
        )
    })
    .unwrap();
    assert!(!observed.dispatch_permitted);
    assert_eq!(
        rooms::submission(
            &store.product_setting_repository(),
            &input.room_id,
            &entry.child_submission_id
        )
        .unwrap()
        .unwrap()
        .execution_id,
        Some(entry.child_submission_id.clone())
    );
}
#[test]
fn same_named_http_model_cannot_change_original_endpoint_or_protocol() {
    let (mut store, input) = setup();
    let manifest = admit(&mut store, input.clone()).unwrap();
    let entry = &manifest.entries[0];
    let mut actual = config(&entry.participant.context_conversation_id);
    let mut binding =
        serde_json::to_value(actual.turn_identity.host_invocation.as_ref().unwrap()).unwrap();
    binding["primary"]["endpoint"] = json!("https://other.invalid/v1");
    binding["output_admission"]["routes"][0]["route"] = binding["primary"].clone();
    actual.turn_identity.host_invocation = Some(serde_json::from_value(binding).unwrap());
    assert!(begin_real_child(
        &mut store,
        &entry.participant.context_conversation_id,
        &entry.child_submission_id,
        &input.text,
        actual,
        vec![]
    )
    .is_err());
    assert!(store
        .active_run(&entry.participant.context_conversation_id)
        .unwrap()
        .is_none());
    let receipt = rooms::submission(
        &store.product_setting_repository(),
        &input.room_id,
        &entry.child_submission_id,
    )
    .unwrap()
    .unwrap();
    assert!(receipt.execution_id.is_none());
    assert_eq!(receipt.state, rooms::RoomSubmissionState::Selected);
}

include!("support/expert_template_capture_cases.rs");
include!("support/expert_host_operation_cases.rs");
include!("support/expert_capture_preview_cases.rs");
