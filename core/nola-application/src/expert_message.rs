// SPDX-License-Identifier: Apache-2.0
//! One complete expert-set admission and fresh-owner resend preparation.
use nola_domain::{
    agent_room::{AgentRoom, ContextOwner, RoomParticipant, CONTEXT_OWNER_FIELD},
    edited_resend::PrepareEditedResend,
    expert_message::*,
    persistent_agent::AgentDefaults,
    ConversationSourceOwner, MessagePartView, MessageRole, MessageState,
};
use nola_store_port::{
    conversation_history::ConversationProjectionRepository,
    conversation_profile::ConversationWriteRepositoryProvider,
    conversation_query::ConversationQueryRepository,
    edited_resend::EditedResendRepository,
    expert_message::ExpertMessageRepository,
    product_setting::{ProductSettingRepository, ProductSettingRepositoryProvider},
    protocol_journal::{CommandProtocolRepositoryProvider, CommandTransactionRepository},
    run_recovery_inspection::RunRecoveryInspectionRepository,
};
use sha2::{Digest, Sha256};

fn transaction<S, T>(
    store: &mut S,
    body: impl FnOnce(&mut S) -> Result<T, String>,
) -> Result<T, String>
where
    S: CommandProtocolRepositoryProvider,
{
    store
        .command_protocol_repository()
        .begin_command_mutations()
        .map_err(|e| e.to_string())?;
    match body(store) {
        Ok(value) => {
            store
                .command_protocol_repository()
                .commit_command_mutations()
                .map_err(|e| e.to_string())?;
            Ok(value)
        }
        Err(error) => {
            store
                .command_protocol_repository()
                .rollback_command_mutations()
                .map_err(|e| format!("{error}; expert message rollback failed: {e}"))?;
            Err(error)
        }
    }
}
fn defaults(settings: &str) -> Result<AgentDefaults, String> {
    let value: serde_json::Value =
        serde_json::from_str(settings).map_err(|_| "expert_message_invalid_context_settings")?;
    let fields = value
        .as_object()
        .ok_or("expert_message_invalid_context_settings")?;
    // Context ownership is Core metadata, not a configuration override. All
    // remaining fields must satisfy the current configuration schema before
    // selecting the smaller frozen defaults set. Otherwise a misspelled field
    // could silently turn an explicit selection into an inherited default.
    let configuration: serde_json::Map<String, serde_json::Value> = fields
        .iter()
        .filter(|(key, _)| key.as_str() != CONTEXT_OWNER_FIELD)
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    nola_domain::configuration_patch::validate_patch(
        &serde_json::Value::Object(configuration).to_string(),
    )?;
    let allowed = [
        "modelSelectionID",
        "modelOverrides",
        "selectedPromptTemplateID",
        "expertReasoningEffort",
        "language",
        "promptResponseStyle",
        "contextLimitOverride",
        "memoryEnabledOverride",
        "retrievalEnabledOverride",
    ];
    AgentDefaults::try_from(serde_json::Value::Object(
        fields
            .iter()
            .filter(|(key, _)| allowed.contains(&key.as_str()))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect(),
    ))
}
pub(crate) mod capture;
pub(crate) mod preview;

#[cfg(test)]
mod context_configuration_tests {
    use super::defaults;

    #[test]
    fn context_metadata_is_not_a_default_but_current_configuration_is_validated() {
        let snapshot = defaults(r#"{"agentRoomOwner":{"room_id":"room"},"selectedPromptTemplateID":"template","isPinned":true}"#).unwrap();
        assert_eq!(snapshot.conversation_configuration(), serde_json::json!({
            "selectedPromptTemplateID": "template"
        }));
        assert!(defaults(r#"{"selectedPromptTemplateId":"template"}"#).is_err());
        assert!(defaults(r#"{"selectedPromptTemplateID":42}"#).is_err());
        assert!(defaults(r#"{"modelOverrides":{"configured:one":{"temperature":"warm"}}}"#).is_err());
    }
}
use crate::expert_template_capture::{
    self as template_policy, ExpertTemplateCaptureEntry, ExpertTemplateCaptureFact,
    HostExpertTemplateAuthority, HostExpertTemplateCaptureContext,
};
use nola_store_port::consistent_read::ConsistentReadRepository;
use nola_store_port::room_presentation::RoomPresentationRepository;
pub use preview::preview_capture;

/// The former direct fan-out is retained only for exact persisted receipt recovery.
/// New user turns belong to Main and may carry a typed expert selection.
pub fn replay_existing<S>(store:&mut S,input:AdmitExpertConversationMessage,host:&HostExpertTemplateCaptureContext,captured_at_ms:i64)->Result<ExpertMessageManifest,String>
where S: ExpertMessageRepository + ProductSettingRepositoryProvider + ConversationProjectionRepository
    + ConversationQueryRepository + ConversationWriteRepositoryProvider + RunRecoveryInspectionRepository
    + RoomPresentationRepository + CommandProtocolRepositoryProvider {
    if store.expert_message_by_submission(&input.conversation_id,&input.submission_id).map_err(|e|e.to_string())?.is_none(){
        return Err("expert_dispatch_requires_main_agent_turn".into());
    }
    admit(store,input,host,captured_at_ms)
}

pub fn admit<S>(
    store: &mut S,
    input: AdmitExpertConversationMessage,
    host: &HostExpertTemplateCaptureContext,
    captured_at_ms: i64,
) -> Result<ExpertMessageManifest, String>
where
    S: ExpertMessageRepository
        + ProductSettingRepositoryProvider
        + ConversationProjectionRepository
        + ConversationQueryRepository
        + ConversationWriteRepositoryProvider
        + CommandProtocolRepositoryProvider
        + RunRecoveryInspectionRepository
        + RoomPresentationRepository,
{
    input.validate()?;
    host.authority().validate()?;
    if captured_at_ms < 0 {
        return Err("invalid_expert_template_capture_time".into());
    }
    transaction(store, |store| {
        admit_inner(store, input, host, captured_at_ms)
    })
}
fn admit_inner<S>(
    store: &mut S,
    input: AdmitExpertConversationMessage,
    host: &HostExpertTemplateCaptureContext,
    captured_at_ms: i64,
) -> Result<ExpertMessageManifest, String>
where
    S: ExpertMessageRepository
        + ProductSettingRepositoryProvider
        + ConversationProjectionRepository
        + ConversationQueryRepository
        + ConversationWriteRepositoryProvider
        + RunRecoveryInspectionRepository
        + RoomPresentationRepository,
{
    if let Some(prior) = store
        .expert_message_by_submission(&input.conversation_id, &input.submission_id)
        .map_err(|e| e.to_string())?
    {
        return if prior.input == input {
            capture::authorized(store, &prior, &host.authority())?;
            Ok(prior)
        } else {
            Err("expert_message_submission_conflict".into())
        };
    }
    host.validate_input(&input)?;
    let resolved = preview::resolve(store, &input)?;
    let parent = resolved.parent;
    let catalog = template_policy::load_custom_catalog(&store.product_setting_repository())?;
    let mut captured_entries = Vec::with_capacity(input.selected_participants.len());
    let mut entries = Vec::with_capacity(input.selected_participants.len());
    for participant in resolved.participants {
        let preview = participant.preview;
        let reference = &preview.participant;
        require_context_available(
            store,
            &reference.context_conversation_id,
            input.created_at_ms,
        )?;
        let selection = host
            .selections
            .iter()
            .find(|selection| {
                selection.context_conversation_id == reference.context_conversation_id
            })
            .ok_or("expert_template_host_selection_missing")?;
        if selection.model_profile.profile_id != preview.model_declaration.profile_id
            || selection.model_profile.admitted_revision
                != preview.model_declaration.profile_revision
        {
            return Err("expert_template_capture_model_profile_changed".into());
        }
        let mut frozen = preview.defaults.conversation_configuration();
        let resolved = template_policy::resolve_selection(
            host,
            &reference.context_conversation_id,
            preview.context_configuration_revision,
            &frozen,
            catalog.as_ref(),
            captured_at_ms,
        )?;
        if let Some(id) = &resolved.template_id {
            frozen["selectedPromptTemplateID"] = serde_json::Value::String(id.clone());
        } else {
            frozen
                .as_object_mut()
                .ok_or("expert_message_invalid_context_settings")?
                .remove("selectedPromptTemplateID");
        }
        let snapshot = AgentDefaults::try_from(frozen)?;
        let expected_prompt_definition = resolved.definition;
        let child_submission_id = uuid::Uuid::new_v4().to_string();
        let host_instructions = host
            .selections
            .iter()
            .find(|selection| {
                selection.context_conversation_id == reference.context_conversation_id
            })
            .ok_or("expert_template_host_selection_missing")?
            .host_user_instructions
            .clone();
        captured_entries.push(ExpertTemplateCaptureEntry {
            participant_id: reference.participant_id.clone(),
            context_conversation_id: reference.context_conversation_id.clone(),
            context_configuration_revision: preview.context_configuration_revision,
            child_submission_id: child_submission_id.clone(),
            definition_sha256: expected_prompt_definition
                .as_ref()
                .map(|v| v.content_sha256.clone()),
            host_user_instructions: host_instructions,
            origin: resolved.origin,
        });
        entries.push(ExpertMessageEntry {
            participant: reference.clone(),
            display_name: participant.display_name,
            description: participant.description,
            defaults: snapshot,
            model_declaration: preview.model_declaration,
            context_configuration_revision: preview.context_configuration_revision,
            child_user_message_id: format!("{child_submission_id}:user"),
            child_assistant_message_id: format!("{child_submission_id}:assistant"),
            child_submission_id,
            expected_prompt_definition,
            model_witness: None,
            instruction_witness: None,
        });
    }
    let value = ExpertMessageManifest {
        version: 1,
        input,
        entries,
    };
    value.validate()?;
    let captured = ExpertTemplateCaptureFact {
        version: 1,
        conversation_id: value.input.conversation_id.clone(),
        submission_id: value.input.submission_id.clone(),
        message_id: value.input.user_message_id.clone(),
        intent_sha256: template_policy::payload_sha256(&value.input)?,
        actor: host.actor.clone(),
        request_host_id: host.request_host_id.clone(),
        captured_at_ms,
        entries: captured_entries,
    };
    capture::validate(store, &value, &captured)?;
    crate::conversation::admit_user_message(
        &mut store.conversation_write_repository(),
        &parent,
        nola_protocol::AdmitConversationUserMessage {
            conversation_id: value.input.conversation_id.clone(),
            message_id: value.input.user_message_id.clone(),
            text: value.input.text.clone(),
            created_at_ms: value.input.created_at_ms,
        },
    )?;
    store
        .put_expert_message_manifest(&value)
        .map_err(|e| e.to_string())?;
    capture::persist(store, &captured)?;
    for entry in &value.entries {
        crate::agent_room::submit_for_conversation(
            store,
            &value.input.conversation_id,
            crate::agent_room::SubmitRoomMessage {
                room_id: value.input.room_id.clone(),
                submission_id: entry.child_submission_id.clone(),
                expected_room_revision: value.input.expected_room_revision,
                text: value.input.text.clone(),
                participant_id: Some(entry.participant.participant_id.clone()),
                at_ms: value.input.created_at_ms,
                parent_message_id: Some(value.input.user_message_id.clone()),
                conversation_id: Some(value.input.conversation_id.clone()),
            },
        )?;
    }
    Ok(value)
}

/// Busy includes selected work before any Run exists. Only its exact canonical
/// terminal seal or an explicit pending cancellation/failure releases the slot.
pub(crate) fn require_context_available<S>(store: &mut S, context: &str, at_ms: i64) -> Result<(), String>
where
    S: ExpertMessageRepository + ProductSettingRepositoryProvider + RunRecoveryInspectionRepository + RoomPresentationRepository,
{
    if store.expert_context_has_pending_task(context).map_err(|e| e.to_string())? {
        return Err("expert_context_has_pending_task".into());
    }
    crate::expert_peer::require_peer_context_available(store,context,at_ms)?;
    let Some(reservation) = store
        .expert_context_reservation(context)
        .map_err(|e| e.to_string())?
    else {
        return Ok(());
    };
    let receipt = crate::agent_room::submission(
        &store.product_setting_repository(),
        &reservation.room_id,
        &reservation.child_submission_id,
    )?
    .ok_or("expert_context_reservation_receipt_missing")?;
    if receipt.context_conversation_id.as_deref() != Some(context)
        || receipt.input.parent_message_id.as_deref()
            != Some(reservation.parent_message_id.as_str())
        || receipt.input.conversation_id.as_deref()
            != Some(reservation.parent_conversation_id.as_str())
    {
        return Err("expert_context_reservation_owner_changed".into());
    }
    use crate::agent_room::RoomSubmissionState;
    if matches!(
        receipt.state,
        RoomSubmissionState::Completed
            | RoomSubmissionState::Incomplete
            | RoomSubmissionState::Blocked
            | RoomSubmissionState::Failed
            | RoomSubmissionState::Cancelled
    ) {
        return Ok(());
    }
    if let Some(run) = store
        .inspect_exact_run(&reservation.child_submission_id)
        .map_err(|e| e.to_string())?
    {
        if run.conversation_id != context
            || run.kind != "conversation"
            || !run.assistant_owner_valid
        {
            return Err("expert_context_reserved_run_owner_changed".into());
        }
        if run.status == "sealed" && run.terminal.is_some() {
            crate::agent_room::observe_execution(
                store,
                &reservation.room_id,
                &reservation.child_submission_id,
                &reservation.child_submission_id,
                at_ms,
            )?;
            return Ok(());
        }
    }
    Err("expert_context_reserved_by_another_parent".into())
}

/// A legacy expert resend may be recovered, but new edited input goes through Main.
pub fn replay_prepared_resend<S>(store:&mut S,input:PrepareEditedResend,authority:&HostExpertTemplateAuthority,captured_at_ms:i64)->Result<PreparedExpertResend,String>
where S: ExpertMessageRepository + EditedResendRepository + ProductSettingRepositoryProvider
    + ConversationProjectionRepository + ConversationQueryRepository + ConversationWriteRepositoryProvider
    + CommandProtocolRepositoryProvider + RunRecoveryInspectionRepository {
    if store.prepared_expert_resend_by_operation(&input.operation_id).map_err(|e|e.to_string())?.is_none(){
        return Err("expert_resend_requires_main_agent_turn".into());
    }
    prepare_resend(store,input,authority,captured_at_ms)
}

pub fn prepare_resend<S>(
    store: &mut S,
    input: PrepareEditedResend,
    authority: &HostExpertTemplateAuthority,
    captured_at_ms: i64,
) -> Result<PreparedExpertResend, String>
where
    S: ExpertMessageRepository
        + EditedResendRepository
        + ProductSettingRepositoryProvider
        + ConversationProjectionRepository
        + ConversationQueryRepository
        + ConversationWriteRepositoryProvider
        + CommandProtocolRepositoryProvider
        + RunRecoveryInspectionRepository,
{
    authority.validate()?;
    if captured_at_ms < 0 {
        return Err("invalid_expert_template_capture_time".into());
    }
    transaction(store, |store| {
        prepare_inner(store, input, authority, captured_at_ms)
    })
}
fn prepare_inner<S>(
    store: &mut S,
    input: PrepareEditedResend,
    authority: &HostExpertTemplateAuthority,
    captured_at_ms: i64,
) -> Result<PreparedExpertResend, String>
where
    S: ExpertMessageRepository
        + EditedResendRepository
        + ProductSettingRepositoryProvider
        + ConversationProjectionRepository
        + ConversationQueryRepository
        + ConversationWriteRepositoryProvider
        + RunRecoveryInspectionRepository,
{
    uuid::Uuid::parse_str(&input.operation_id)
        .map_err(|_| "expert_resend_requires_stable_operation")?;
    uuid::Uuid::parse_str(&input.target_conversation_id)
        .map_err(|_| "expert_resend_requires_fresh_target_uuid")?;
    let manifest = store
        .expert_message_manifest(&input.source_conversation_id, &input.source_message_id)
        .map_err(|e| e.to_string())?
        .ok_or("expert_message_complete_source_manifest_unavailable")?;
    let source_capture = capture::authorized(store, &manifest, authority)?;
    if let Some(prior) = store
        .prepared_expert_resend_by_operation(&input.operation_id)
        .map_err(|e| e.to_string())?
    {
        return if prior.input == input {
            let value = load_prepared_unchecked(store, &input.target_conversation_id)?
                .ok_or("expert_resend_prepared_target_missing")?;
            authorize_prepared(store, &value, authority)?;
            Ok(value)
        } else {
            Err("expert_resend_operation_conflict".into())
        };
    }
    if input.target_conversation_id == input.source_conversation_id
        || input.edited_text.trim().is_empty()
        || input.edited_text.len() > 1_000_000
        || input.edited_text.contains('\0')
        || input.created_at_ms < 0
        || input.title.trim().is_empty()
        || input.title.len() > 512
    {
        return Err("expert_resend_invalid_input".into());
    }
    manifest.require_complete_source()?;
    if store
        .conversation_source_owner(&input.source_conversation_id)
        .map_err(|e| e.to_string())?
        != ConversationSourceOwner::Local
        || store
            .active_run(&input.source_conversation_id)
            .map_err(|e| e.to_string())?
            .is_some()
    {
        return Err("expert_resend_source_owner_not_idle_local".into());
    }
    let source = store
        .load_conversation(&input.source_conversation_id)
        .map_err(|e| e.to_string())?
        .ok_or("expert_resend_source_missing")?;
    if source.revision != input.expected_source_revision {
        return Err("expert_resend_source_revision_changed".into());
    }
    let projection = crate::load_conversation_projection_v1(store, &input.source_conversation_id)
        .map_err(|e| e.to_string())?;
    let message = projection
        .messages
        .iter()
        .find(|m| m.message_id.0 == input.source_message_id)
        .ok_or("expert_resend_source_message_not_visible")?;
    if message.role != MessageRole::User
        || message.state != MessageState::Completed
        || message.revision != input.expected_source_message_revision
    {
        return Err("expert_resend_source_message_revision_changed".into());
    }
    let [MessagePartView::Text {
        markdown,
        is_final: true,
        ..
    }] = message.parts.as_slice()
    else {
        return Err("expert_resend_requires_plain_original_input".into());
    };
    if markdown != &manifest.input.text {
        return Err("expert_resend_original_body_changed".into());
    }
    let room_id = uuid::Uuid::new_v4().to_string();
    let parent_message_id = format!("{}:user", input.operation_id);
    let created = crate::conversation::create_conversation(
        &mut store.conversation_write_repository(),
        crate::conversation::CreateConversation {
            id: input.target_conversation_id.clone(),
            title: input.title.clone(),
            settings_json: Some(serde_json::json!({"permissionProfile":"cautious"}).to_string()),
            created_at_ms: input.created_at_ms,
            updated_at_ms: input.created_at_ms,
        },
    )?;
    if !created.created {
        return Err("expert_resend_target_already_exists".into());
    }
    let mut entries = vec![];
    let mut participants = vec![];
    for original in manifest.entries.iter().cloned() {
        let witness = original
            .model_witness
            .clone()
            .ok_or("expert_message_complete_model_witness_required")?;
        let child = store
            .load_conversation(&witness.conversation_id)
            .map_err(|e| e.to_string())?
            .ok_or("expert_resend_original_child_missing")?;
        let child_projection =
            crate::load_conversation_projection_v1(store, &witness.conversation_id)
                .map_err(|e| e.to_string())?;
        let child_message = child_projection
            .messages
            .iter()
            .find(|m| m.message_id.0 == witness.message_id)
            .ok_or("expert_resend_original_child_message_missing")?;
        let participant = ExpertParticipantReference {
            participant_id: uuid::Uuid::new_v4().to_string(),
            agent_id: original.participant.agent_id.clone(),
            agent_revision: original.participant.agent_revision,
            context_conversation_id: uuid::Uuid::new_v4().to_string().to_uppercase(),
        };
        let mut settings = original.defaults.conversation_configuration();
        settings["permissionProfile"] = serde_json::json!("cautious");
        settings[CONTEXT_OWNER_FIELD] = serde_json::to_value(ContextOwner {
            room_id: room_id.clone(),
            participant_id: participant.participant_id.clone(),
            agent_id: participant.agent_id.clone(),
        })
        .map_err(|e| e.to_string())?;
        let child_input = PrepareEditedResend {
            operation_id: uuid::Uuid::new_v4().to_string(),
            source_conversation_id: witness.conversation_id.clone(),
            source_message_id: witness.message_id.clone(),
            expected_source_revision: child.revision,
            expected_source_message_revision: child_message.revision,
            edited_text: input.edited_text.clone(),
            target_conversation_id: participant.context_conversation_id.clone(),
            title: original.display_name.clone(),
            created_at_ms: input.created_at_ms,
        };
        let prepared = crate::edited_resend::prepare_verified_expert_child(
            store,
            child_input,
            witness,
            settings,
        )?;
        participants.push(RoomParticipant {
            id: participant.participant_id.clone(),
            agent_id: participant.agent_id.clone(),
            agent_revision: participant.agent_revision,
            display_name: original.display_name.clone(),
            description: original.description.clone(),
            context_conversation_id: participant.context_conversation_id.clone(),
            active: true,
        });
        entries.push(PreparedExpertResendEntry {
            original,
            participant,
            prepared,
        });
    }
    let room = AgentRoom::new(room_id.clone(), input.title.clone(), participants)?;
    let association = crate::agent_room::install_resend_room(
        store,
        input.target_conversation_id.clone(),
        room,
        input.created_at_ms,
    )?;
    let parent = store
        .load_conversation(&input.target_conversation_id)
        .map_err(|e| e.to_string())?
        .ok_or("expert_resend_new_parent_missing")?;
    crate::conversation::admit_user_message(
        &mut store.conversation_write_repository(),
        &parent,
        nola_protocol::AdmitConversationUserMessage {
            conversation_id: input.target_conversation_id.clone(),
            message_id: parent_message_id.clone(),
            text: input.edited_text.clone(),
            created_at_ms: input.created_at_ms,
        },
    )?;
    let new_input = AdmitExpertConversationMessage {
        conversation_id: input.target_conversation_id.clone(),
        expected_revision: parent.revision,
        room_id: room_id.clone(),
        expected_room_revision: association.room.revision,
        submission_id: input.operation_id.clone(),
        user_message_id: parent_message_id.clone(),
        text: input.edited_text.clone(),
        selected_participants: entries.iter().map(|e| e.participant.clone()).collect(),
        created_at_ms: input.created_at_ms,
    };
    let new_manifest = ExpertMessageManifest {
        version: 1,
        input: new_input,
        entries: entries
            .iter()
            .map(|entry| ExpertMessageEntry {
                participant: entry.participant.clone(),
                display_name: entry.original.display_name.clone(),
                description: entry.original.description.clone(),
                defaults: entry.original.defaults.clone(),
                model_declaration: entry.original.model_declaration.clone(),
                context_configuration_revision: 1,
                child_submission_id: entry.prepared.run_id.clone(),
                child_user_message_id: entry.prepared.user_message_id.clone(),
                child_assistant_message_id: entry.prepared.assistant_message_id.clone(),
                expected_prompt_definition: entry.original.expected_prompt_definition.clone(),
                model_witness: None,
                instruction_witness: None,
            })
            .collect(),
    };
    store
        .put_expert_message_manifest(&new_manifest)
        .map_err(|e| e.to_string())?;
    let new_capture = ExpertTemplateCaptureFact {
        version: 1,
        conversation_id: new_manifest.input.conversation_id.clone(),
        submission_id: new_manifest.input.submission_id.clone(),
        message_id: new_manifest.input.user_message_id.clone(),
        intent_sha256: template_policy::payload_sha256(&new_manifest.input)?,
        actor: authority.actor.clone(),
        request_host_id: authority.request_host_id.clone(),
        captured_at_ms,
        entries: new_manifest
            .entries
            .iter()
            .zip(&source_capture.entries)
            .map(|(entry, original)| {
                Ok(ExpertTemplateCaptureEntry {
                    participant_id: entry.participant.participant_id.clone(),
                    context_conversation_id: entry.participant.context_conversation_id.clone(),
                    context_configuration_revision: entry.context_configuration_revision,
                    child_submission_id: entry.child_submission_id.clone(),
                    definition_sha256: entry
                        .expected_prompt_definition
                        .as_ref()
                        .map(|v| v.content_sha256.clone()),
                    host_user_instructions: original.host_user_instructions.clone(),
                    origin: capture::retained_origin(&source_capture, original)?,
                })
            })
            .collect::<Result<Vec<_>, String>>()?,
    };
    capture::validate(store, &new_manifest, &new_capture)?;
    capture::persist(store, &new_capture)?;
    for entry in &entries {
        crate::agent_room::submit_for_conversation(
            store,
            &input.target_conversation_id,
            crate::agent_room::SubmitRoomMessage {
                room_id: room_id.clone(),
                submission_id: entry.prepared.run_id.clone(),
                expected_room_revision: association.room.revision,
                text: input.edited_text.clone(),
                participant_id: Some(entry.participant.participant_id.clone()),
                at_ms: input.created_at_ms,
                parent_message_id: Some(parent_message_id.clone()),
                conversation_id: Some(input.target_conversation_id.clone()),
            },
        )?;
    }
    let prepared = PreparedExpertResend {
        input,
        source_submission_id: manifest.input.submission_id,
        room_id,
        parent_message_id,
        entries,
    };
    store
        .put_prepared_expert_resend(&prepared)
        .map_err(|e| e.to_string())?;
    Ok(prepared)
}
/// Every child admission remains owned by the existing ordinary consumer.
/// A partial admitted subset is returned explicitly, never labelled group completion.
fn load_prepared_unchecked<S: ExpertMessageRepository + EditedResendRepository>(
    store: &S,
    target: &str,
) -> Result<Option<PreparedExpertResend>, String> {
    let Some(mut value) = store
        .prepared_expert_resend(target)
        .map_err(|e| e.to_string())?
    else {
        return Ok(None);
    };
    for entry in &mut value.entries {
        let child = store
            .prepared_edited_resend(&entry.participant.context_conversation_id)
            .map_err(|e| e.to_string())?
            .ok_or("expert_resend_prepared_child_missing")?;
        if child.input != entry.prepared.input
            || child.source != entry.prepared.source
            || child.run_id != entry.prepared.run_id
        {
            return Err("expert_resend_child_preparation_owner_changed".into());
        }
        entry.prepared = child;
    }
    Ok(Some(value))
}

fn require_supported_missing_manifest<S>(
    store: &mut S,
    conversation_id: &str,
    message_id: &str,
) -> Result<(), String>
where
    S: ExpertMessageRepository
        + ConversationProjectionRepository
        + ProductSettingRepositoryProvider,
{
    let conversation = store
        .load_conversation(conversation_id)
        .map_err(|e| e.to_string())?
        .ok_or("edited_resend_source_not_found")?;
    let Some(message) = conversation.messages.iter().find(|m| m.id == message_id) else {
        return Ok(());
    };
    if message.role == "user" && message.status == "completed" && message.run_id.is_none() {
        let associated = store
            .product_setting_repository()
            .product_setting(&format!(
                "{}{}",
                crate::agent_room::CONVERSATION_ROOM_KEY_PREFIX,
                conversation_id
            ))
            .map_err(|e| e.to_string())?
            .is_some();
        let settings: serde_json::Value =
            serde_json::from_str(conversation.settings_json.as_deref().unwrap_or("{}"))
                .map_err(|_| "edited_resend_invalid_source_configuration")?;
        if associated
            || settings
                .get(CONTEXT_OWNER_FIELD)
                .is_some_and(|v| !v.is_null())
        {
            return Err("expert_message_complete_source_manifest_unavailable".into());
        }
    }
    Ok(())
}

fn authorize_prepared<S>(
    store: &S,
    value: &PreparedExpertResend,
    authority: &HostExpertTemplateAuthority,
) -> Result<(), String>
where
    S: ExpertMessageRepository + ConversationProjectionRepository,
{
    let manifest = store
        .expert_message_manifest(
            &value.input.target_conversation_id,
            &value.parent_message_id,
        )
        .map_err(|e| e.to_string())?
        .ok_or("expert_resend_prepared_capture_manifest_missing")?;
    capture::authorized(store, &manifest, authority)?;
    if manifest.input.room_id != value.room_id
        || manifest.input.submission_id != value.input.operation_id
        || manifest.input.text != value.input.edited_text
        || manifest.entries.len() != value.entries.len()
    {
        return Err("expert_resend_prepared_capture_owner_changed".into());
    }
    for (entry, prepared) in manifest.entries.iter().zip(&value.entries) {
        if entry.participant != prepared.participant
            || entry.child_submission_id != prepared.prepared.run_id
            || entry.child_user_message_id != prepared.prepared.user_message_id
            || entry.child_assistant_message_id != prepared.prepared.assistant_message_id
            || entry.expected_prompt_definition != prepared.original.expected_prompt_definition
        {
            return Err("expert_resend_prepared_capture_entry_changed".into());
        }
    }
    Ok(())
}

pub fn load_manifest_authorized<S>(
    store: &mut S,
    conversation: &str,
    message: &str,
    authority: &HostExpertTemplateAuthority,
) -> Result<Option<ExpertMessageManifest>, String>
where
    S: ExpertMessageRepository
        + ConversationProjectionRepository
        + ProductSettingRepositoryProvider
        + ConsistentReadRepository,
{
    authority.validate()?;
    let value = store.with_consistent_read(|reader| {
        let value = reader
            .expert_message_manifest(conversation, message)
            .map_err(|e| e.to_string())?;
        if let Some(manifest) = &value {
            capture::authorized(reader, manifest, authority)?;
        }
        Ok(value)
    })?;
    if value.is_none() {
        require_supported_missing_manifest(store, conversation, message)?;
    }
    Ok(value)
}

pub fn load_prepared_authorized<S>(
    store: &mut S,
    target: &str,
    authority: &HostExpertTemplateAuthority,
) -> Result<Option<PreparedExpertResend>, String>
where
    S: ExpertMessageRepository
        + EditedResendRepository
        + ConversationProjectionRepository
        + ConsistentReadRepository,
{
    authority.validate()?;
    store.with_consistent_read(|reader| {
        let value = load_prepared_unchecked(reader, target)?;
        if let Some(prepared) = &value {
            authorize_prepared(reader, prepared, authority)?;
        }
        Ok(value)
    })
}

pub fn load_child_instructions_authorized<S>(
    store: &mut S,
    context: &str,
    expected_run: &str,
    authority: &HostExpertTemplateAuthority,
) -> Result<Option<Vec<ExpertPromptDefinition>>, String>
where
    S: ExpertMessageRepository
        + ConversationProjectionRepository
        + ConversationQueryRepository
        + ConsistentReadRepository,
{
    authority.validate()?;
    if context.trim().is_empty() || expected_run.trim().is_empty() {
        return Err("expert_child_instruction_identity_missing".into());
    }
    store.with_consistent_read(|reader| {
        let Some(manifest) = reader
            .expert_message_for_child(context, expected_run)
            .map_err(|e| e.to_string())?
        else {
            if let Some(settings) = reader
                .load_conversation_settings(context)
                .map_err(|e| e.to_string())?
            {
                let settings: serde_json::Value =
                    serde_json::from_str(&settings).map_err(|_| "expert_child_context_invalid")?;
                if settings
                    .get(CONTEXT_OWNER_FIELD)
                    .is_some_and(|v| !v.is_null())
                {
                    return Err("expert_child_capture_manifest_missing".into());
                }
            }
            return Ok(None);
        };
        if reader
            .conversation_source_owner(context)
            .map_err(|e| e.to_string())?
            != ConversationSourceOwner::Local
        {
            return Err("expert_template_child_source_not_local".into());
        }
        let fact = capture::authorized(reader, &manifest, authority)?;
        let index = manifest
            .entries
            .iter()
            .position(|entry| {
                entry.participant.context_conversation_id == context
                    && entry.child_submission_id == expected_run
            })
            .ok_or("expert_child_instruction_owner_changed")?;
        let entry = &manifest.entries[index];
        let expected = capture::definitions(entry, &fact.entries[index])?;
        if let Some(actual) = &entry.instruction_witness {
            if !capture::same_definitions(&expected, &actual.prompt_definitions) {
                return Err("expert_child_instruction_set_changed".into());
            }
            return Ok(Some(actual.prompt_definitions.clone()));
        }
        if let Some(prepared) = reader
            .prepared_expert_resend(&manifest.input.conversation_id)
            .map_err(|e| e.to_string())?
        {
            authorize_prepared(reader, &prepared, authority)?;
            let original = prepared
                .entries
                .iter()
                .find(|value| {
                    value.participant == entry.participant && value.prepared.run_id == expected_run
                })
                .ok_or("expert_child_prepared_instruction_owner_changed")?;
            let actual = original
                .original
                .instruction_witness
                .as_ref()
                .ok_or("expert_child_prepared_instruction_witness_missing")?;
            if !capture::same_definitions(&expected, &actual.prompt_definitions) {
                return Err("expert_child_prepared_instruction_set_changed".into());
            }
            return Ok(Some(actual.prompt_definitions.clone()));
        }
        Ok(Some(expected))
    })
}
