// SPDX-License-Identifier: Apache-2.0
//! Actual isolated Store admission/rollback; no model execution or UI claims.
mod support;
use nola_application::{agent_room as rooms, expert_message, persistent_agent as agents};
use nola_domain::{
    expert_message::*,
    persistent_agent::{AgentAvailability, AgentDefaults},
};
use nola_store::NolaStore;
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
    for id in ["first", "second", "third", "fourth", "fifth"] {
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

use nola_application::expert_peer::{PeerClarificationInput, PeerExchangeInput, PeerMaterial};

fn peer_fixture() -> (NolaStore, PeerExchangeInput) {
    let (store, input, _) = peer_fixture_with_accepted();
    (store, input)
}
fn peer_fixture_with_accepted() -> (NolaStore, PeerExchangeInput, AcceptedAssistantTurn) {
    let (mut store, initial) = setup();
    let target=initial.selected_participants[1].clone();
    let run=uuid::Uuid::new_v4().to_string();
    let started=begin_real_child(&mut store,"parent",&run,&initial.text,config("parent"),vec![]).unwrap();
    (store,PeerExchangeInput{source_context_id:"parent".into(),source_run_id:run,call_id:"peer-call-1".into(),
        target_participant_id:target.participant_id,kind:"request".into(),text:"Check this exact implementation".into(),in_reply_to:None},started)
}

fn peer_context(
    preview: &nola_application::expert_template_capture::ExpertTemplateCapturePreview,
) -> HostExpertTemplateCaptureContext {
    let a = authority();
    HostExpertTemplateCaptureContext {
        actor: a.actor,
        request_host_id: a.request_host_id,
        builtins: vec![],
        selections: preview
            .participants
            .iter()
            .map(|p| HostExpertTemplateSelection {
                context_conversation_id: p.participant.context_conversation_id.clone(),
                context_configuration_revision: p.context_configuration_revision,
                model_profile: nola_host_protocol::HostModelProfileReference {
                    profile_id: p.model_declaration.profile_id.clone(),
                    admitted_revision: p.model_declaration.profile_revision,
                },
                source: HostExpertTemplateSelectionSource::NoTemplate,
                template_id: None,
                host_user_instructions: vec![],
            })
            .collect(),
    }
}
fn peer_admit(store: &mut NolaStore, input: PeerExchangeInput) -> Result<PeerMaterial, String> {
    let context = peer_context(&nola_application::expert_peer::preview(
        store,
        &input,
        &authority(),
    )?);
    fixture_command(store, |store| {
        nola_application::expert_peer::admit(store, input, &context, 1001)
    })
}
#[test]
fn peer_admission_is_atomic_idempotent_and_does_not_fabricate_parent_user_messages() {
    let (mut store, input) = peer_fixture();
    let before = store.load_conversation("parent").unwrap().unwrap();
    let value = peer_admit(&mut store, input.clone()).unwrap();
    assert_eq!(
        value.view.source.context_conversation_id(),
        input.source_context_id
    );
    assert_eq!(
        value.view.target.participant_id,
        input.target_participant_id
    );
    assert_eq!(value.view.source_display_name, "Nola");
    assert_eq!(value.view.target_display_name, "second");
    assert_eq!(value.entry.child_submission_id, value.view.exchange_id);
    assert!(value.prompt_definitions.is_empty());
    assert_eq!(store.load_conversation("parent").unwrap().unwrap(), before);
    let receipt = rooms::submission(
        &store.product_setting_repository(),
        &value.view.room_id,
        &value.view.exchange_id,
    )
    .unwrap()
    .unwrap();
    assert!(receipt.input.parent_message_id.is_none());
    assert!(receipt.input.conversation_id.is_none());
    assert_eq!(
        receipt.selected_participant_id,
        Some(input.target_participant_id.clone())
    );
    let replay = peer_admit(&mut store, input.clone()).unwrap();
    assert_eq!(value, replay);
    let mut changed = input;
    changed.text = "different intent".into();
    assert!(peer_admit(&mut store, changed)
        .unwrap_err()
        .contains("call_conflict"));
    assert_eq!(
        rooms::submissions(
            &store.product_setting_repository(),
            &value.view.room_id,
            200,
            0
        )
        .unwrap()
        .len(),
        0
    );
    assert_eq!(
        rooms::submissions(
            &store.product_setting_repository(),
            &value.view.room_id,
            1,
            0
        )
        .unwrap()
        .len(),
        0
    );
    assert!(
        rooms::submissions(
            &store.product_setting_repository(),
            &value.view.room_id,
            1,
            1
        )
        .unwrap()
        .is_empty(),
        "Peer receipts are excluded before public pagination, never rendered as user prompts"
    );
    let public = nola_application::expert_peer::list_views(&store, "parent", 200, 0).unwrap();
    let json = serde_json::to_value(public).unwrap();
    assert_eq!(json["exchanges"][0]["exchange_id"], value.view.exchange_id);
    assert!(json["exchanges"][0].get("entry").is_none());
    assert!(json["exchanges"][0].get("prompt_definitions").is_none());
    let mapped = nola_application::expert_peer::view_for_child_run(
        &store,
        "parent",
        &value.view.target.context_conversation_id,
        &value.view.exchange_id,
    )
    .unwrap()
    .unwrap();
    assert_eq!(mapped, value.view);
    assert!(nola_application::expert_peer::view_for_child_run(
        &store,
        "parent",
        &input_source(&value),
        &value.view.exchange_id
    )
    .is_err());
}
fn input_source(value: &PeerMaterial) -> String {
    value.view.source.context_conversation_id().to_owned()
}
#[test]
fn peer_source_authority_busy_target_and_changed_capture_fail_without_partial_records() {
    let (mut store, input) = peer_fixture();
    let mut other = authority();
    other.actor.id = "another-user".into();
    other.actor.kind=nola_agent_engine::agent::resource_access::AgentResourcePrincipalKind::Anyone;
    assert!(nola_application::expert_peer::preview(&mut store, &input, &other).is_err());
    other=authority();other.actor.id="another-user".into();
    let mut inactive = input.clone();
    inactive.source_run_id = uuid::Uuid::new_v4().to_string();
    assert!(
        nola_application::expert_peer::preview(&mut store, &inactive, &authority())
            .unwrap_err()
            .contains("main_agent_run_mismatch")
    );
    let preview = nola_application::expert_peer::preview(&mut store, &input, &authority()).unwrap();
    let mut wrong = peer_context(&preview);
    wrong.selections[0].model_profile.admitted_revision += 1;
    assert!(
        fixture_command(&mut store, |store| nola_application::expert_peer::admit(
            store,
            input.clone(),
            &wrong,
            1001
        ))
        .unwrap_err()
        .contains("model_changed")
    );
    assert!(
        nola_application::expert_peer::list_views(&store, "parent", 200, 0)
            .unwrap()
            .exchanges
            .is_empty()
    );
    let value = peer_admit(&mut store, input.clone()).unwrap();
    let mut second = input;
    second.call_id = "new-call-while-target-selected".into();
    assert!(peer_admit(&mut store, second)
        .unwrap_err()
        .contains("target_busy"));
    assert_eq!(
        nola_application::expert_peer::list_views(&store, "parent", 200, 0)
            .unwrap()
            .exchanges
            .len(),
        1
    );
    assert!(nola_application::expert_peer::load(
        &mut store,
        &value.view.room_id,
        &value.view.exchange_id,
        &other
    )
    .is_err());
}
#[test]
fn expert_cannot_clarify_or_dispatch_another_expert() {
    let(mut store,input)=peer_fixture();let value=peer_admit(&mut store,input).unwrap();
    let target=&value.view.target.context_conversation_id;
    begin_real_child(&mut store,target,&value.view.exchange_id,&value.view.text,config(target),vec![]).unwrap();
    let association=rooms::for_conversation(&mut store,"parent").unwrap().unwrap();
    let other=association.room.room.participants().iter().find(|p|p.id!=value.view.target.participant_id).unwrap();
    let attempted=PeerExchangeInput{source_context_id:target.clone(),source_run_id:value.view.exchange_id.clone(),call_id:"forbidden-recursive".into(),
        target_participant_id:other.id.clone(),kind:"request".into(),text:"Delegate again".into(),in_reply_to:None};
    assert!(peer_admit(&mut store,attempted).unwrap_err().contains("only_main_agent"));
    assert!(fixture_command(&mut store,|s|nola_application::expert_peer::clarify(s,PeerClarificationInput{source_context_id:target.clone(),source_run_id:value.view.exchange_id.clone(),
        call_id:"forbidden-clarify".into(),exchange_id:value.view.exchange_id.clone(),text:"Ask Main".into()},&authority(),1002)).unwrap_err().contains("through_their_task"));
    assert!(nola_application::expert_peer::load(&mut store,&value.view.room_id,&value.view.exchange_id,&authority()).unwrap().view.clarifications.is_empty());
}
#[test]
fn peer_real_begin_enforces_frozen_model_and_records_actual_witnesses() {
    let (mut store, input) = peer_fixture();
    let value = peer_admit(&mut store, input).unwrap();
    let target = &value.view.target.context_conversation_id;
    let mut wrong = config(target);
    let mut binding =
        serde_json::to_value(wrong.turn_identity.host_invocation.take().unwrap()).unwrap();
    binding["primary"]["model"] = json!("different-model");
    wrong.turn_identity.host_invocation = Some(serde_json::from_value(binding).unwrap());
    assert!(begin_real_child(
        &mut store,
        target,
        &value.view.exchange_id,
        &value.view.text,
        wrong,
        vec![]
    )
    .is_err());
    assert!(store.active_run(target).unwrap().is_none());
    assert!(store
        .load_conversation(target)
        .unwrap()
        .unwrap()
        .messages
        .is_empty());
    begin_real_child(
        &mut store,
        target,
        &value.view.exchange_id,
        &value.view.text,
        config(target),
        vec![],
    )
    .unwrap();
    let actual = nola_application::expert_peer::load(
        &mut store,
        &value.view.room_id,
        &value.view.exchange_id,
        &authority(),
    )
    .unwrap();
    assert!(actual.entry.model_witness.is_some());
    assert!(actual.entry.instruction_witness.is_some());
    assert_eq!(
        actual.entry.model_witness.unwrap().run_id,
        value.view.exchange_id
    );
}
#[test]
fn human_admission_cannot_claim_a_peer_target_before_its_run_begins() {
    let (mut store, input) = peer_fixture();
    let value = peer_admit(&mut store, input).unwrap();
    let association = rooms::for_conversation(&mut store, "parent")
        .unwrap()
        .unwrap();
    let human = AdmitExpertConversationMessage {
        conversation_id: "parent".into(),
        expected_revision: store.load_conversation("parent").unwrap().unwrap().revision,
        room_id: value.view.room_id.clone(),
        expected_room_revision: association.room.revision,
        submission_id: uuid::Uuid::new_v4().to_string(),
        user_message_id: "new-human-message".into(),
        text: "Do not steal the selected peer context".into(),
        selected_participants: vec![value.view.target],
        created_at_ms: 1003,
    };
    let count = store
        .load_conversation("parent")
        .unwrap()
        .unwrap()
        .messages
        .len();
    assert!(admit(&mut store, human)
        .unwrap_err()
        .contains("parent_has_active_run"));
    assert_eq!(
        store
            .load_conversation("parent")
            .unwrap()
            .unwrap()
            .messages
            .len(),
        count
    );
}
#[test]
fn main_source_wire_cannot_be_forged_as_expert_or_unknown_kind() {
    use nola_domain::expert_task_target::ExpertTaskSourceV1;
    let wire=json!({"kind":"main_agent","context_conversation_id":"parent"});
    let source:ExpertTaskSourceV1=serde_json::from_value(wire.clone()).unwrap();assert!(source.is_main_agent());
    let mut forged=wire;forged["participant_id"]=json!("forged");assert!(serde_json::from_value::<ExpertTaskSourceV1>(forged).is_err());
    assert!(serde_json::from_value::<ExpertTaskSourceV1>(json!({"kind":"other","context_conversation_id":"parent"})).is_err());
    let(mut store,input)=peer_fixture();let value=peer_admit(&mut store,input).unwrap();
    let legacy=serde_json::to_value(&value.view.target).unwrap();assert!(serde_json::from_value::<ExpertTaskSourceV1>(legacy).unwrap().legacy_expert().is_some());
    assert!(value.view.source.is_main_agent());
}
#[test]
fn peer_private_host_commands_cannot_be_deserialized_or_read_through_generic_settings() {
    let (mut store, input) = peer_fixture();
    let value = peer_admit(&mut store, input).unwrap();
    for name in [
        "preview_expert_peer",
        "admit_expert_peer",
        "load_expert_peer",
        "load_expert_peer_for_submission",
        "clarify_expert_peer",
    ] {
        assert!(
            serde_json::from_value::<nola_application::ApplicationCommand>(
                json!({"type":name,"input":{}})
            )
            .is_err()
        );
    }
    let key = format!(
        "{}exchange/{}",
        nola_application::expert_peer::PRIVATE_PREFIX,
        value.view.exchange_id
    );
    assert!(nola_application::execute_application_command(
        &mut store,
        nola_application::ApplicationCommand::GetProductSetting { key: key.clone() }
    )
    .is_err());
    assert!(nola_application::execute_application_command(
        &mut store,
        nola_application::ApplicationCommand::PutProductSetting {
            key: key.clone(),
            value_json: "{}".into(),
            updated_at_ms: 1002
        }
    )
    .is_err());
    assert!(nola_application::execute_application_command(
        &mut store,
        nola_application::ApplicationCommand::DeleteProductSetting { key }
    )
    .is_err());
    let missing = uuid::Uuid::new_v4().to_string();
    assert!(nola_application::expert_peer::load_for_submission(
        &mut store,
        &value.view.room_id,
        &missing,
        &authority()
    )
    .unwrap()
    .is_none());
    let mut wrong = authority();
    wrong.actor.id = "wrong-user".into();
    assert!(nola_application::expert_peer::load_for_submission(
        &mut store,
        &value.view.room_id,
        &value.view.exchange_id,
        &wrong
    )
    .is_err());
}

fn finish_target(store: &mut NolaStore, owner: &str, accepted: AcceptedAssistantTurn) {
    use nola_application::{
        claim_committed_host_requests, reduce_and_accept_host_response, ClaimCommittedHostRequests,
        ReduceAndAcceptHostResponse,
    };
    let claimed = claim_committed_host_requests(
        store,
        ClaimCommittedHostRequests {
            run_id: accepted.begin.run_id.clone(),
            worker_generation: "peer-fixture".into(),
            lease_owner: "peer-owner".into(),
            claimed_at_ms: 1010,
            lease_expires_at_ms: 5000,
            limit: 64,
        },
    )
    .unwrap();
    let request = claimed
        .requests
        .iter()
        .find(|r| matches!(r.payload, HostRequest::ModelStream { .. }))
        .unwrap();
    for (index, payload) in [
        HostResponse::ExecutionStarted,
        HostResponse::ModelAttemptStarted {
            provider: "fixture".into(),
            model: "original-model".into(),
            reason: "initial".into(),
        },
        HostResponse::ModelCompleted {
            items: vec![serde_json::to_value(AgentItem::assistant_message(
                ItemId::from_string("peer-answer".into()),
                "Waiting for clarification",
            ))
            .unwrap()],
            tool_calls: vec![],
            usage: json!({}),
        },
    ]
    .into_iter()
    .enumerate()
    {
        reduce_and_accept_host_response(
            store,
            ReduceAndAcceptHostResponse {
                conversation_id: owner.into(),
                run_id: accepted.begin.run_id.clone(),
                invocation_config_digest: accepted.invocation_config_digest.clone(),
                received_at_ms: 1020 + index as i64,
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
                    sequence: index as u64 + 1,
                    payload,
                },
            },
        )
        .unwrap();
    }
    assert!(store.active_run(owner).unwrap().is_none());
}
#[test]
fn canonical_target_terminal_allows_original_source_clarification_followup_without_new_hop() {
    let (mut store, input) = peer_fixture();
    let value = peer_admit(&mut store, input.clone()).unwrap();
    let target = value.view.target.context_conversation_id.clone();
    let accepted = begin_real_child(
        &mut store,
        &target,
        &value.view.exchange_id,
        &value.view.text,
        config(&target),
        vec![],
    )
    .unwrap();
    finish_target(&mut store, &target, accepted);
    let follow = peer_admit(
        &mut store,
        PeerExchangeInput {
            call_id: "answer-detail".into(),
            text: "Keep the public signature unchanged".into(),
            in_reply_to: Some(value.view.exchange_id.clone()),
            ..input
        },
    )
    .unwrap();
    assert_eq!(follow.view.root_exchange_id, value.view.root_exchange_id);
    assert_eq!(follow.view.hop, value.view.hop);
    assert_eq!(
        follow.view.in_reply_to,
        Some(value.view.exchange_id.clone())
    );
    assert_ne!(follow.view.exchange_id, value.view.exchange_id);
    assert_eq!(follow.view.source, value.view.source);
    assert_eq!(follow.view.target, value.view.target);
}
#[test]
fn directory_exposes_only_bound_members_and_preserves_ordinary_parent_active_run_guard() {
    let (mut store, input) = peer_fixture();
    let directory = nola_application::expert_peer::directory(
        &mut store,
        &input.source_context_id,
        &input.source_run_id,
        &authority(),
    )
    .unwrap();
    assert_eq!(directory.members.len(), 5);
    assert_eq!(
        directory.source.context_conversation_id(),
        input.source_context_id
    );
    assert!(nola_application::expert_peer::directory(&mut store,"unrelated-parent",&input.source_run_id,&authority()).is_err());
    let association = rooms::for_conversation(&mut store, "parent")
        .unwrap()
        .unwrap();
    let target = association
        .room
        .room
        .participants()
        .iter()
        .find(|p| p.id == input.target_participant_id)
        .unwrap();
    let capture = AdmitExpertConversationMessage {
        conversation_id: "parent".into(),
        expected_revision: store.load_conversation("parent").unwrap().unwrap().revision,
        room_id: directory.room_id,
        expected_room_revision: directory.room_revision,
        submission_id: uuid::Uuid::new_v4().to_string(),
        user_message_id: "blocked-user-admission".into(),
        text: "Human send must remain blocked".into(),
        selected_participants: vec![ExpertParticipantReference {
            participant_id: target.id.clone(),
            agent_id: target.agent_id.clone(),
            agent_revision: target.agent_revision,
            context_conversation_id: target.context_conversation_id.clone(),
        }],
        created_at_ms: 2000,
    };
    assert!(
        expert_message::preview_capture(&mut store, &capture, &authority())
            .unwrap_err()
            .contains("parent_has_active_run")
    );
    assert!(
        nola_application::expert_peer::preview(&mut store, &input, &authority()).is_ok(),
        "Only peer capture may coexist with parent execution"
    );
}

fn history_configure_next(store: &mut NolaStore, agent: &str) {
    let directory = agents::load(&store.product_setting_repository()).unwrap();
    fixture_command(store, |store| {
        agents::configure(
            &mut store.product_setting_repository(),
            agents::ConfigureAgent {
                id: agent.to_string().try_into().unwrap(),
                expected_directory_revision: directory.revision,
                expected_agent_revision: Some(1),
                name: format!("Updated {agent}"),
                description: "Updated responsibility".into(),
                defaults: AgentDefaults::try_from(
                    json!({"modelSelectionID":"configured:model","language":"en"}),
                )
                .unwrap(),
                availability: AgentAvailability::Active,
                updated_at_ms: 20,
            },
        )
    })
    .unwrap();
}

#[test]
fn expert_child_pages_keep_retired_contexts_and_history_reads_never_create_runs() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("children.sqlite");
    let (mut store, input) = setup_at(&path);
    let old = input.selected_participants[0].clone();
    history_configure_next(&mut store, "first");
    let next = fixture_command(&mut store, |store| {
        rooms::rejoin(
            store,
            rooms::JoinAgent {
                room_id: input.room_id.clone(),
                expected_room_revision: input.expected_room_revision,
                agent_id: old.agent_id.clone(),
                expected_agent_revision: 2,
                at_ms: 21,
            },
        )
    })
    .unwrap();
    let fresh = next
        .room
        .participants()
        .iter()
        .find(|p| p.agent_id == old.agent_id)
        .unwrap();
    assert_ne!(fresh.context_conversation_id, old.context_conversation_id);
    let mut ids = std::collections::BTreeSet::new();
    let mut offset = 0;
    loop {
        let page = rooms::child_conversations(&mut store, "parent".into(), 1, offset).unwrap();
        assert_eq!(page.room_id.as_deref(), Some(input.room_id.as_str()));
        assert_eq!(page.children.len(), 1);
        let child = &page.children[0];
        assert!(ids.insert(child.conversation_id.clone()));
        if child.conversation_id == old.context_conversation_id {
            assert!(!child.is_current && !child.active);
            assert_eq!(child.agent_revision, None);
        }
        if !page.has_more {
            assert_eq!(page.next_offset, None);
            break;
        }
        assert_eq!(page.next_offset, Some(offset + 1));
        offset += 1;
    }
    assert_eq!(ids.len(), 6);
    assert!(
        ids.contains(&old.context_conversation_id) && ids.contains(&fresh.context_conversation_id)
    );
    let count = || {
        rusqlite::Connection::open(&path)
            .unwrap()
            .query_row::<i64, _, _>("SELECT COUNT(*) FROM runs", [], |r| r.get(0))
            .unwrap()
    };
    let before = count();
    let history = rooms::child_history(
        &mut store,
        "parent".into(),
        old.context_conversation_id.clone(),
        None,
        50,
        262144,
    )
    .unwrap();
    assert_eq!(history.child_conversation_id, old.context_conversation_id);
    assert!(history.window.projection.messages.is_empty());
    assert!(history.expert_exchanges.is_empty());
    assert_eq!(
        (before, count()),
        (0, 0),
        "Read must never create/resume a Run"
    );
}

#[test]
fn root_directory_filters_before_paging_and_keeps_published_all_directory_semantics() {
    use nola_application::ApplicationCommand;
    use nola_store_port::conversation_query::ConversationListQuery;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("roots.sqlite");
    let (mut store, _) = setup_at(&path);
    for (id, at) in [("second-parent", 30), ("task-child", 40)] {
        store
            .create_conversation(
                &nola_store_port::conversation_profile::CreateConversationInput {
                    id: id.into(),
                    title: id.into(),
                    revision: 1,
                    settings_json: Some("{}".into()),
                    created_at_ms: at,
                    updated_at_ms: at,
                },
            )
            .unwrap();
    }
    drop(store);
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute("INSERT INTO runtime_task_run_bindings(task_id,owner_session_id,child_session_id,run_id,admission_attempt_id,bound_at_ms) VALUES('fixture-task','parent','task-child','fixture-run','fixture-admission',40)",[]).unwrap();
    db.execute("UPDATE conversations SET updated_at_ms=100 WHERE id NOT IN ('parent','second-parent','task-child')",[]).unwrap();
    drop(db);
    let query = |offset| ConversationListQuery {
        conversation_id: None,
        project_root: None,
        non_empty: false,
        limit: 1,
        offset,
    };
    let mut runtime = support::open_runtime(&path).unwrap();
    let first = runtime
        .execute(ApplicationCommand::ListRootConversations { query: query(0) })
        .unwrap();
    assert_eq!(first.as_array().unwrap().len(), 1);
    assert_eq!(first[0]["id"], "second-parent");
    assert_eq!(
        runtime
            .execute(ApplicationCommand::ListRootConversations { query: query(1) })
            .unwrap()[0]["id"],
        "parent"
    );
    assert_eq!(
        runtime
            .execute(ApplicationCommand::ListRootConversations { query: query(2) })
            .unwrap(),
        json!([])
    );
    let all = runtime
        .execute(ApplicationCommand::ListConversations {
            query: ConversationListQuery {
                limit: 100,
                ..query(0)
            },
        })
        .unwrap();
    assert_eq!(
        all.as_array().unwrap().len(),
        8,
        "Published all-list retains expert and task contexts"
    );
    assert!(all
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v["id"] == "task-child"));
}

#[test]
fn child_history_rejects_sibling_room_parent_self_and_invalid_bounds() {
    let (mut store, input) = setup();
    let child = input.selected_participants[0]
        .context_conversation_id
        .clone();
    store
        .create_conversation(
            &nola_store_port::conversation_profile::CreateConversationInput {
                id: "other-parent".into(),
                title: "Other".into(),
                revision: 1,
                settings_json: Some("{}".into()),
                created_at_ms: 1,
                updated_at_ms: 1,
            },
        )
        .unwrap();
    fixture_command(&mut store, |store| {
        rooms::ensure_for_conversation(store, "other-parent".into(), 1)
    })
    .unwrap();
    assert!(rooms::child_history(
        &mut store,
        "other-parent".into(),
        child.clone(),
        None,
        50,
        262144
    )
    .is_err());
    assert!(
        rooms::child_history(&mut store, child.clone(), child.clone(), None, 50, 262144).is_err()
    );
    assert!(rooms::child_conversations(&mut store, child, 40, 0).is_err());
    for (limit, bytes, before) in [(0, 262144, None), (50, 0, None), (50, 262144, Some(0))] {
        assert!(rooms::child_history(
            &mut store,
            "parent".into(),
            input.selected_participants[0]
                .context_conversation_id
                .clone(),
            before,
            limit,
            bytes
        )
        .is_err());
    }
}

#[test]
fn public_submission_paging_hides_peers_but_rejoin_busy_scan_keeps_them() {
    let (mut store, input, started) = peer_fixture_with_accepted();
    let peer = peer_admit(&mut store, input).unwrap();
    let public = rooms::submissions(
        &store.product_setting_repository(),
        &peer.view.room_id,
        200,
        0,
    )
    .unwrap();
    assert!(public.is_empty());
    finish_target(&mut store, &peer.view.source.context_conversation_id(), started);
    history_configure_next(&mut store, "third");
    let room = rooms::load(&store.product_setting_repository(), &peer.view.room_id)
        .unwrap()
        .unwrap();
    let third = room
        .room
        .participants()
        .iter()
        .find(|p| p.agent_id.as_str() == "third")
        .unwrap();
    let error = fixture_command(&mut store, |store| {
        rooms::rejoin(
            store,
            rooms::JoinAgent {
                room_id: peer.view.room_id.clone(),
                expected_room_revision: room.revision,
                agent_id: third.agent_id.clone(),
                expected_agent_revision: 2,
                at_ms: 31,
            },
        )
    })
    .unwrap_err();
    assert!(
        error.contains("in_use_by_pending_submission"),
        "hidden Selected peer must still block Room replacement: {error}"
    );
    assert!(rooms::submissions(
        &store.product_setting_repository(),
        &peer.view.room_id,
        1,
        1
    )
    .unwrap()
    .is_empty());
    assert_eq!(
        rooms::submission(
            &store.product_setting_repository(),
            &peer.view.room_id,
            &peer.view.exchange_id
        )
        .unwrap()
        .unwrap()
        .state,
        rooms::RoomSubmissionState::Selected
    );
}

#[test]
fn child_history_provenance_is_scoped_to_exact_window_and_target() {
    let (mut store, input) = peer_fixture();
    let peer = peer_admit(&mut store, input).unwrap();
    let target = &peer.view.target.context_conversation_id;
    begin_real_child(
        &mut store,
        target,
        &peer.view.exchange_id,
        &peer.view.text,
        config(target),
        vec![],
    )
    .unwrap();
    let history = rooms::child_history(
        &mut store,
        "parent".into(),
        target.clone(),
        None,
        50,
        262144,
    )
    .unwrap();
    assert_eq!(history.expert_exchanges.len(), 1);
    assert_eq!(
        history.expert_exchanges[0].exchange_id,
        peer.view.exchange_id
    );
    assert_eq!(history.expert_exchanges[0].source_display_name, "Nola");
    assert!(history
        .window
        .projection
        .messages
        .iter()
        .any(|m| m.message_id.0 == format!("{}:user", peer.view.exchange_id)));
    assert!(rooms::child_history(&mut store,"parent".into(),peer.view.source.context_conversation_id().to_owned(),None,50,262144).is_err(),
        "The main conversation cannot masquerade as a child history");
}

fn expert_task_draft(value: &PeerMaterial) -> nola_store_port::runtime_task::StoredRuntimeTask {
    use nola_domain::expert_task_target::ExpertTaskTargetV1;
    let pin = ExpertTaskTargetV1 {
        version: 1,
        task_id: value.view.exchange_id.clone(),
        exchange_id: value.view.exchange_id.clone(),
        parent_conversation_id: value.view.parent_conversation_id.clone(),
        room_id: value.view.room_id.clone(),
        source: value.view.source.clone(),
        target: value.view.target.clone(),
        source_run_id: value.view.source_run_id.clone(),
        source_call_id: value.view.source_call_id.clone(),
    };
    serde_json::from_value(json!({"id":pin.task_id,"kind":"agent","status":"queued","title":"Prepared expert target",
        "project_root":"/fixture","owner_session_id":pin.source.context_conversation_id(),"child_session_id":pin.target.context_conversation_id,
        "parent_tool_call_id":pin.source_call_id,"unread":false,"cancel_requested":false,"retry_count":0,"input_watermark":7,
        "created_at_ms":1004,"updated_at_ms":1004,"metadata_json":json!({"expert_target":pin}).to_string()})).unwrap()
}
#[test]
fn prepared_expert_target_preserves_context_visibility_but_never_grants_worker_authority() {
    use nola_store_port::conversation_history::ConversationProjectionRepository;
    let (mut store, input) = peer_fixture();
    let value = peer_admit(&mut store, input).unwrap();
    let draft = expert_task_draft(&value);
    store.create_runtime_task_with_event(&draft).unwrap();
    let target = &value.view.target.context_conversation_id;
    assert!(store
        .validated_expert_task_target(&draft.id, target, false)
        .unwrap()
        .is_some());
    assert!(store
        .validated_expert_task_target(&draft.id, target, true)
        .is_err());
    assert_eq!(
        store
            .runtime_agent_task_authority(&draft.id)
            .unwrap()
            .unwrap()
            .phase,
        nola_domain::agent_task_admission::RuntimeAgentTaskAuthorityPhase::Prepared
    );
    assert!(store
        .claim_runtime_task(&draft.id, "worker", "generation", "token", 1010, 2000)
        .unwrap()
        .is_none());
    assert!(!store.is_runtime_task_child_session(target).unwrap());
    assert_eq!(
        store.conversation_execution_ownership(target).unwrap(),
        nola_domain::ConversationExecutionOwnership::User
    );
    assert!(store.expert_context_has_pending_task(target).unwrap());
    assert!(store.runtime_task_run_binding(&draft.id).unwrap().is_none());
    assert!(store.active_run(target).unwrap().is_none());
}
#[test]
fn metadata_only_unknown_version_or_foreign_peer_pin_cannot_authorize_expert_task() {
    for change in ["version", "private-record", "call", "target"] {
        let (mut store, input) = peer_fixture();
        let value = peer_admit(&mut store, input).unwrap();
        let mut draft = expert_task_draft(&value);
        let mut metadata: serde_json::Value = serde_json::from_str(&draft.metadata_json).unwrap();
        match change {
            "version" => metadata["expert_target"]["version"] = json!(99),
            "private-record" => {
                let id = uuid::Uuid::new_v4().to_string();
                draft.id = id.clone();
                metadata["expert_target"]["task_id"] = json!(id);
                metadata["expert_target"]["exchange_id"] = json!(draft.id);
            }
            "call" => {
                draft.parent_tool_call_id = Some("different-call".into());
                metadata["expert_target"]["source_call_id"] = json!("different-call");
            }
            "target" => {
                draft.child_session_id = Some("foreign-child".into());
                metadata["expert_target"]["target"]["context_conversation_id"] =
                    json!("foreign-child");
            }
            _ => unreachable!(),
        }
        draft.metadata_json = metadata.to_string();
        store.create_runtime_task_with_event(&draft).unwrap();
        assert!(
            store
                .validated_expert_task_target(
                    &draft.id,
                    draft.child_session_id.as_deref().unwrap(),
                    false
                )
                .is_err(),
            "{change}"
        );
        assert!(store
            .claim_runtime_task(&draft.id, "worker", "generation", "token", 1010, 2000)
            .unwrap()
            .is_none());
    }
}
#[test]
fn plain_task_without_expert_pin_keeps_original_prepared_authority_semantics() {
    let (mut store, input) = peer_fixture();
    let value = peer_admit(&mut store, input).unwrap();
    let mut draft = expert_task_draft(&value);
    draft.metadata_json = "{}".into();
    store.create_runtime_task_with_event(&draft).unwrap();
    let child = draft.child_session_id.as_deref().unwrap();
    assert!(store
        .validated_expert_task_target(&draft.id, child, false)
        .unwrap()
        .is_none());
    assert!(store.is_runtime_task_child_session(child).unwrap());
    assert!(store
        .claim_runtime_task(&draft.id, "worker", "generation", "token", 1010, 2000)
        .unwrap()
        .is_none());
}

/// Reproduce already-imported source ownership facts in a private database only.
/// This tests read ownership, not authentication or remote-import admission.
fn history_fixture_remote_source(path: &std::path::Path, owner: &str, host: &str, epoch: &str) {
    let db=rusqlite::Connection::open(path).unwrap();let run=format!("fixture-source-{owner}");
    db.execute("INSERT INTO runs(id,conversation_id,kind,status,next_event_sequence,created_at_ms,updated_at_ms) VALUES(?1,?2,'conversation','completed',0,50,50)",rusqlite::params![run,owner]).unwrap();
    db.execute("INSERT INTO remote_run_sources(run_id,source_host_id) VALUES(?1,?2)",rusqlite::params![run,host]).unwrap();
    db.execute("INSERT INTO remote_assistant_journal_sources(run_id,source_host_id,source_store_epoch,assistant_message_id) VALUES(?1,?2,?3,?4)",rusqlite::params![run,host,epoch,format!("{run}:assistant")]).unwrap();
    db.execute("INSERT INTO remote_conversation_execution_owners(run_id,source_host_id,source_store_epoch,ownership_json) VALUES(?1,?2,?3,'{\"kind\":\"user\"}')",rusqlite::params![run,host,epoch]).unwrap();
}

#[test]
fn child_history_and_directory_require_the_same_remote_host_and_epoch_as_parent() {
    use nola_store_port::conversation_history::ConversationProjectionRepository;
    for(host,epoch,allowed)in[("host-a","epoch-a",true),("host-b","epoch-a",false),("host-a","epoch-b",false)] {
        let dir=tempfile::tempdir().unwrap();let path=dir.path().join("remote-history.sqlite");let(store,input)=setup_at(&path);drop(store);
        let target=&input.selected_participants[0].context_conversation_id;
        history_fixture_remote_source(&path,"parent","host-a","epoch-a");
        for member in &input.selected_participants {
            let(is_host,is_epoch)=if member.context_conversation_id==*target{(host,epoch)}else{("host-a","epoch-a")};
            history_fixture_remote_source(&path,&member.context_conversation_id,is_host,is_epoch);
        }
        let mut store=NolaStore::open(&path).unwrap();
        assert_eq!(store.conversation_source_owner(target).unwrap(),nola_domain::ConversationSourceOwner::Remote{source_host_id:host.into(),source_store_epoch:epoch.into()});
        let children=rooms::child_conversations(&mut store,"parent".into(),40,0);
        let history=rooms::child_history(&mut store,"parent".into(),target.clone(),None,50,262144);
        if allowed {
            assert_eq!(children.unwrap().children.len(),5);assert_eq!(history.unwrap().source_owner,nola_domain::ConversationSourceOwner::Remote{source_host_id:"host-a".into(),source_store_epoch:"epoch-a".into()});
        }else{
            assert!(children.unwrap_err().contains("source_mismatch"));assert!(history.unwrap_err().contains("source_mismatch"));
        }
    }
}

#[path = "support/expert_peer_task_bridge.rs"]
mod task_bridge;

fn main_selection_config(owner:&str)->AgentCoreConfig {
    let mut cfg=config(owner);let schema=nola_domain::expert_task_target::tool_schema();cfg.tools.push(schema.clone());
    let mut descriptor=cfg.host_catalog.as_ref().unwrap().capabilities["model.stream"].clone();
    descriptor.id="delegate_expert".into();descriptor.route=HostCapabilityRoute::Tool;descriptor.exposure=HostCapabilityExposure::Visible;
    descriptor.input_schema=schema["function"]["parameters"].clone();descriptor.supports_streaming=false;
    cfg.host_catalog.as_mut().unwrap().capabilities.insert("delegate_expert".into(),descriptor);cfg
}
fn selection_component(value:serde_json::Value)->nola_agent_engine::context_engine::ContextComponent{
    nola_agent_engine::context_engine::ContextComponent{kind:nola_agent_engine::context_engine::ContextComponentKind::MainExpertSelection,
        id:nola_domain::expert_task_target::MAIN_EXPERT_SELECTION_COMPONENT_ID.into(),content:value,source:None}
}
#[test]
fn main_selection_pins_existing_snapshot_without_creating_an_expert_submission(){
    let(mut store,initial)=setup();let selected=&initial.selected_participants[1];
    history_configure_next(&mut store,"second"); // Existing joined revision remains an intentional snapshot.
    let run=uuid::Uuid::new_v4().to_string();
    let component=selection_component(json!({"version":1,"selected_experts":[selected]}));
    let accepted=begin_real_child(&mut store,"parent",&run,"Please check this",main_selection_config("parent"),vec![component]).unwrap();
    let actual=store.load_conversation("parent").unwrap().unwrap();
    assert_eq!(actual.messages.iter().filter(|message|message.role=="user").count(),1);
    assert!(store.active_run(&selected.context_conversation_id).unwrap().is_none());
    assert!(rooms::submissions(&store.product_setting_repository(),&initial.room_id,100,0).unwrap().is_empty());
    let requests=nola_application::claim_committed_host_requests(&mut store,nola_application::ClaimCommittedHostRequests{run_id:accepted.begin.run_id,
        worker_generation:"main-selected".into(),lease_owner:"main-selected".into(),claimed_at_ms:11,lease_expires_at_ms:100,limit:64}).unwrap();
    let model=requests.requests.iter().find(|r|matches!(r.payload,HostRequest::ModelStream{..})).unwrap();
    let wire=serde_json::to_string(&model.payload).unwrap();assert!(wire.contains("main_expert_selection"));assert!(wire.contains(&selected.participant_id));
}
#[test]
fn main_selection_rejects_stale_unpinned_or_forged_scope_atomically(){
    for variant in ["stale","scope","epoch","empty_epoch"] {
        let(mut store,initial)=setup();let selected=&initial.selected_participants[1];
        let mut entry=serde_json::to_value(selected).unwrap();
        let mut value=json!({"version":1,"selected_experts":[]});
        if variant=="stale" {history_configure_next(&mut store,"second");entry.as_object_mut().unwrap().remove("participant_id");entry.as_object_mut().unwrap().remove("context_conversation_id");}
        if variant=="scope" {entry["context_conversation_id"]=json!(initial.selected_participants[2].context_conversation_id);}
        if variant=="epoch" {value["expected_source_store_epoch"]=json!("different-source");}
        if variant=="empty_epoch" {value["expected_source_store_epoch"]=json!("");}
        value["selected_experts"]=json!([entry]);
        let before=store.load_conversation("parent").unwrap().unwrap();let run=uuid::Uuid::new_v4().to_string();
        assert!(begin_real_child(&mut store,"parent",&run,"Same input",main_selection_config("parent"),vec![selection_component(value)]).is_err(),"{variant}");
        assert_eq!(store.load_conversation("parent").unwrap().unwrap(),before);assert!(store.active_run("parent").unwrap().is_none());
    }
}
#[test]
fn published_host_admission_cannot_start_new_direct_expert_fanout(){
    let(mut store,initial)=setup();let context=host_context(&store,&initial);
    assert!(fixture_command(&mut store,|store|expert_message::replay_existing(store,initial.clone(),&context,1000)).unwrap_err().contains("requires_main_agent_turn"));
    let legacy=admit(&mut store,initial.clone()).unwrap();
    let recovered=fixture_command(&mut store,|store|expert_message::replay_existing(store,initial,&context,1001)).unwrap();assert_eq!(legacy,recovered);
}
#[test]
fn first_main_input_can_join_observed_expert_without_an_empty_preexisting_conversation(){
    let(mut store,initial)=setup();let selected=&initial.selected_participants[1];
    let parent=uuid::Uuid::new_v4().to_string();let run=uuid::Uuid::new_v4().to_string();
    assert!(store.load_conversation(&parent).unwrap().is_none());
    let component=selection_component(json!({"version":1,"selected_experts":[{"agent_id":selected.agent_id,"agent_revision":selected.agent_revision}]}));
    begin_real_child(&mut store,&parent,&run,"First input with expert",main_selection_config(&parent),vec![component]).unwrap();
    let room=rooms::for_conversation(&mut store,&parent).unwrap().unwrap();assert_eq!(room.room.room.participants().len(),1);
    assert_eq!(store.load_conversation(&parent).unwrap().unwrap().messages.iter().filter(|m|m.role=="user").count(),1);
    assert!(store.active_run(&room.room.room.participants()[0].context_conversation_id).unwrap().is_none());
}

#[test]
fn selected_experts_require_actual_main_tool_capability_and_available_dispatch() {
    for variant in ["model_without_tools","tools_disabled","hidden","unavailable"] {
        let(mut store,initial)=setup();let selected=&initial.selected_participants[1];
        let mut cfg=main_selection_config("parent");
        match variant {
            "model_without_tools"=>cfg.turn_identity.model_capability.native_tool_calls=false,
            "tools_disabled"=>{cfg.tools.clear();cfg.host_catalog.as_mut().unwrap().capabilities.remove("delegate_expert");},
            "hidden"=>cfg.host_catalog.as_mut().unwrap().capabilities.get_mut("delegate_expert").unwrap().exposure=HostCapabilityExposure::Hidden,
            "unavailable"=>cfg.host_catalog.as_mut().unwrap().capabilities.get_mut("delegate_expert").unwrap().availability=CapabilityAvailability::Unavailable,
            _=>unreachable!(),
        }
        let before=store.load_conversation("parent").unwrap().unwrap();
        let component=selection_component(json!({"version":1,"selected_experts":[selected]}));let run=uuid::Uuid::new_v4().to_string();
        let error=begin_real_child(&mut store,"parent",&run,"Explicit selected expert",cfg,vec![component]).unwrap_err();
        assert!(error.contains(if variant=="model_without_tools" {"requires_tool_capable_main_model"} else {"requires_enabled_expert_dispatch"}),"{variant}: {error}");
        assert_eq!(store.load_conversation("parent").unwrap().unwrap(),before);
        assert!(store.active_run("parent").unwrap().is_none());
    }
    let(mut store,_)=setup();let run=uuid::Uuid::new_v4().to_string();let mut cfg=config("parent");cfg.turn_identity.model_capability.native_tool_calls=false;
    assert!(begin_real_child(&mut store,"parent",&run,"Ordinary chat without selected experts",cfg,vec![]).is_ok());
}
