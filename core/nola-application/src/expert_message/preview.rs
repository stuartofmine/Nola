// SPDX-License-Identifier: Apache-2.0
//! One immutable model-resolution path for native capture preview and admission.
use super::*;
use crate::expert_template_capture::{
    ExpertTemplateCapturePreview, ExpertTemplateCapturePreviewEntry,
};
use nola_store_port::room_presentation::{RoomPresentationKey, RoomPresentationRepository};

pub(crate) struct CaptureInputs {
    pub parent: nola_store_port::conversation_history::StoredConversation,
    pub participants: Vec<CaptureParticipant>,
}
pub(crate) struct CaptureParticipant {
    pub preview: ExpertTemplateCapturePreviewEntry,
    pub display_name: String,
    pub description: String,
}
pub(crate) fn resolve<S>(store: &S, input: &AdmitExpertConversationMessage) -> Result<CaptureInputs, String>
where S: ExpertMessageRepository + ConversationProjectionRepository + RoomPresentationRepository {
    resolve_with_parent_policy(store, input, false)
}
pub(crate) fn resolve_peer<S>(store: &S, input: &AdmitExpertConversationMessage) -> Result<CaptureInputs, String>
where S: ExpertMessageRepository + ConversationProjectionRepository + RoomPresentationRepository {
    resolve_with_parent_policy(store, input, true)
}
fn resolve_with_parent_policy<S>(store: &S, input: &AdmitExpertConversationMessage, peer: bool) -> Result<CaptureInputs, String>
where S: ExpertMessageRepository + ConversationProjectionRepository + RoomPresentationRepository {

    if store
        .conversation_source_owner(&input.conversation_id)
        .map_err(|e| e.to_string())?
        != ConversationSourceOwner::Local
    {
        return Err("expert_message_requires_local_canonical_owner".into());
    }
    if store
        .conversation_execution_ownership(&input.conversation_id)
        .map_err(|e| e.to_string())?
        != nola_domain::ConversationExecutionOwnership::User
        || store
            .read_room_context_owner(&input.conversation_id)
            .map_err(|e| e.to_string())?
            .is_some()
    {
        return Err("expert_message_requires_ordinary_parent_owner".into());
    }
    if !peer && store
        .active_run(&input.conversation_id)
        .map_err(|e| e.to_string())?
        .is_some()
    {
        return Err("expert_message_parent_has_active_run".into());
    }
    let parent = store
        .load_conversation(&input.conversation_id)
        .map_err(|e| e.to_string())?
        .ok_or("expert_message_parent_not_found")?;
    if parent.revision != input.expected_revision {
        return Err("expert_message_parent_revision_changed".into());
    }
    let forward = store
        .read_room_presentation_setting(RoomPresentationKey::ConversationAssociation(
            &input.conversation_id,
        ))
        .map_err(|e| e.to_string())?
        .ok_or("expert_message_room_association_required")?;
    let reverse = store
        .read_room_presentation_setting(RoomPresentationKey::RoomAssociation(&input.room_id))
        .map_err(|e| e.to_string())?
        .ok_or("expert_message_room_reverse_association_required")?;
    let room = store
        .read_room_presentation_setting(RoomPresentationKey::Room(&input.room_id))
        .map_err(|e| e.to_string())?
        .ok_or("expert_message_room_missing")?;
    let association = crate::agent_room::validated_association(
        &input.conversation_id,
        &forward,
        &reverse,
        crate::agent_room::room_from_record(&input.room_id, &room)?,
    )?;
    if association.room_id != input.room_id
        || association.room.revision != input.expected_room_revision
    {
        return Err("expert_message_room_revision_changed".into());
    }
    let facts = store
        .model_resolution_facts(crate::model_selection::DEFAULT_CHAT_MODEL_KEY)
        .map_err(|e| e.to_string())?;
    let profiles = facts.profiles;
    let preference = facts.default_chat_preference;
    let candidates = profiles
        .iter()
        .filter(|p| p.deleted_at_ms.is_none())
        .map(|p| crate::model_selection::ChatModelCandidate {
            profile_id: p.id.clone(),
            allowed_for_chat: true,
            availability: if p.enabled {
                crate::model_selection::ModelAvailability::Available
            } else {
                crate::model_selection::ModelAvailability::Disabled
            },
        })
        .collect::<Vec<_>>();
    let mut participants = Vec::with_capacity(input.selected_participants.len());
    for reference in &input.selected_participants {
        let member = association.room.room.recipient(&reference.participant_id)?;
        if member.agent_id != reference.agent_id
            || member.agent_revision != reference.agent_revision
            || member.context_conversation_id != reference.context_conversation_id
        {
            return Err("expert_message_selected_participant_snapshot_changed".into());
        }
        let context = store
            .load_conversation(&reference.context_conversation_id)
            .map_err(|e| e.to_string())?
            .ok_or("expert_message_context_not_found")?;
        if store
            .conversation_source_owner(&context.id)
            .map_err(|e| e.to_string())?
            != ConversationSourceOwner::Local
        {
            return Err("expert_message_child_requires_local_canonical_owner".into());
        }
        if store
            .active_run(&context.id)
            .map_err(|e| e.to_string())?
            .is_some()
        {
            return Err("expert_message_selected_context_has_active_run".into());
        }
        let settings: serde_json::Value =
            serde_json::from_str(context.settings_json.as_deref().unwrap_or("{}"))
                .map_err(|_| "expert_message_invalid_context_settings")?;
        let owner: ContextOwner = serde_json::from_value(
            settings
                .get(CONTEXT_OWNER_FIELD)
                .cloned()
                .ok_or("expert_message_context_owner_missing")?,
        )
        .map_err(|_| "expert_message_invalid_context_owner")?;
        if owner.room_id != input.room_id
            || owner.participant_id != member.id
            || owner.agent_id != member.agent_id
        {
            return Err("expert_message_context_owner_changed".into());
        }
        let snapshot = defaults(context.settings_json.as_deref().unwrap_or("{}"))?;
        let frozen_configuration = snapshot.conversation_configuration();
        let original_selection = frozen_configuration
            .get("modelSelectionID")
            .and_then(|v| v.as_str());
        let intent = match original_selection {
            Some(selection) => {
                let id = selection
                    .strip_prefix("configured:")
                    .or_else(|| selection.strip_prefix("local:"))
                    .filter(|id| !id.is_empty())
                    .ok_or("expert_message_selected_model_identity_not_supported")?;
                crate::model_selection::ChatModelIntent::Explicit {
                    profile_id: id.into(),
                }
            }
            None => crate::model_selection::ChatModelIntent::Inherit,
        };
        let crate::model_selection::ChatModelResolution::Resolved { profile_id, .. } =
            crate::model_selection::resolve_chat_model(
                &intent,
                preference.as_ref().map(|v| v.value_json.as_str()),
                &candidates,
            )
        else {
            return Err("expert_message_original_model_declaration_unavailable".into());
        };
        let profile = profiles
            .iter()
            .find(|p| p.id == profile_id)
            .ok_or("expert_message_model_profile_missing")?;
        let mut frozen = snapshot.conversation_configuration();
        let selection = original_selection.map(str::to_owned).unwrap_or_else(|| {
            format!(
                "{}:{}",
                if profile.runtime == "remote" {
                    "configured"
                } else {
                    "local"
                },
                profile.id
            )
        });
        let client_model_presentation = settings
            .get("clientModelPresentation")
            .and_then(|models| models.get(&selection))
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        frozen["modelSelectionID"] = serde_json::Value::String(selection.clone());
        crate::expert_roles::apply_reasoning_preference(&mut frozen, &selection)?;
        if frozen.get("expertReasoningEffort").and_then(|value| value.as_str())
            .is_some_and(|effort| effort != "automatic") && profile.runtime != "remote" {
            return Err("expert_reasoning_effort_requires_declared_remote_model".into());
        }
        let effective_parameters = crate::effective_model_parameters::resolve_profile_parameters(
            profile, &frozen, &selection,
        )?;
        participants.push(CaptureParticipant {
            preview: ExpertTemplateCapturePreviewEntry {
                participant: reference.clone(),
                context_configuration_revision: context.configuration_revision,
                defaults: AgentDefaults::try_from(frozen)?,
                effective_parameters,
                client_model_presentation,
                model_declaration: ExpertModelDeclaration {
                    profile_id: profile.id.clone(),
                    profile_revision: profile.revision,
                    provider: profile.provider.clone(),
                    model_name: profile.model_name.clone(),
                    runtime: profile.runtime.clone(),
                    parameters: serde_json::from_str(&profile.parameters_json)
                        .map_err(|_| "expert_message_invalid_model_parameters")?,
                },
            },
            display_name: member.display_name.clone(),
            description: member.description.clone(),
        });
    }
    Ok(CaptureInputs {
        parent,
        participants,
    })
}

pub fn preview_capture<S>(
    store: &mut S,
    input: &AdmitExpertConversationMessage,
    authority: &HostExpertTemplateAuthority,
) -> Result<ExpertTemplateCapturePreview, String>
where
    S: ExpertMessageRepository
        + ConversationProjectionRepository
        + RoomPresentationRepository
        + ConsistentReadRepository,
{
    input.validate()?;
    authority.validate()?;
    store.with_consistent_read(|reader| {
        // Preview is not permission to use bodies or dispatch work. It has no
        // side effects, reservation reconciliation, receipt, or Run creation.
        let resolved = resolve(reader, input)?;
        Ok(ExpertTemplateCapturePreview {
            participants: resolved
                .participants
                .into_iter()
                .map(|entry| entry.preview)
                .collect(),
        })
    })
}
