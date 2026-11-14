// SPDX-License-Identifier: Apache-2.0
//! A user selection guides the single Main agent; it never submits expert work.
use crate::{assistant_turn::BeginAssistantTurn, repositories::AssistantAdmissionRepositories};
use nola_agent_engine::{agent::AgentSubmission, context_engine::ContextComponentKind};
use nola_domain::expert_task_target::{
    MainExpertSelectionContextV1, MAIN_EXPERT_SELECTION_COMPONENT_ID,
};
use serde_json::json;

pub(crate) fn admit<S: AssistantAdmissionRepositories + ?Sized>(
    store: &mut S,
    command: &mut BeginAssistantTurn,
) -> Result<(), String> {
    let turn_context_components = match &mut command.submission {
        AgentSubmission::UserInput {
            turn_context_components,
            ..
        }
        | AgentSubmission::GoalUserInput {
            turn_context_components,
            ..
        } => turn_context_components,
        AgentSubmission::AutomatedInput {
            turn_context_components,
            ..
        }
        | AgentSubmission::PromoteQueuedInput {
            turn_context_components,
            ..
        } => {
            if turn_context_components
                .iter()
                .any(|component| component.kind == ContextComponentKind::MainExpertSelection)
            {
                return Err("main_expert_selection_requires_fresh_user_input".into());
            }
            return Ok(());
        }
        _ => return Ok(()),
    };
    let indexes = turn_context_components
        .iter()
        .enumerate()
        .filter(|(_, c)| c.kind == ContextComponentKind::MainExpertSelection)
        .map(|(i, _)| i)
        .collect::<Vec<_>>();
    if indexes.is_empty() {
        return Ok(());
    }
    if indexes.len() != 1 {
        return Err("duplicate_main_expert_selection".into());
    }
    let component = &mut turn_context_components[indexes[0]];
    if component.id != MAIN_EXPERT_SELECTION_COMPONENT_ID || component.source.is_some() {
        return Err("main_expert_selection_invalid_provenance".into());
    }
    let mut input: MainExpertSelectionContextV1 = serde_json::from_value(component.content.clone())
        .map_err(|_| "main_expert_selection_invalid")?;
    if input.version != 1 || input.selected_experts.is_empty() || input.selected_experts.len() > 32
    {
        return Err("main_expert_selection_unsupported_or_empty".into());
    }
    if let Some(expected) = &input.expected_source_store_epoch {
        let epoch = store.assistant_source_epoch().map_err(|e| e.to_string())?;
        if expected.is_empty() || expected != &epoch {
            return Err("main_expert_selection_source_changed".into());
        }
    }
    if !command.core_config.turn_identity.model_capability.native_tool_calls {
        return Err("main_expert_selection_requires_tool_capable_main_model".into());
    }
    use nola_agent_engine::agent::host::{CapabilityAvailability,HostCapabilityRoute,HostCapabilityExposure};
    if !command.core_config.host_catalog.as_ref().and_then(|catalog|catalog.capabilities.get("delegate_expert"))
        .is_some_and(|capability|capability.id=="delegate_expert" && capability.route==HostCapabilityRoute::Tool
            && capability.availability==CapabilityAvailability::Available
            && matches!(capability.exposure,HostCapabilityExposure::Visible|HostCapabilityExposure::Deferred)) {
        return Err("main_expert_selection_requires_enabled_expert_dispatch".into());
    }
    let parent = &command.conversation_id;
    if store
        .conversation_summary(parent)
        .map_err(|e| e.to_string())?
        .is_none()
    {
        if command.expected_configuration_revision.is_some() || command.expected_parent_message_id.is_some(){
            return Err("main_expert_selection_new_conversation_precondition_changed".into());
        }
        let created=crate::conversation::create_conversation(
            &mut store.conversation_write_repository(),
            crate::conversation::CreateConversation {
                id: parent.clone(),
                title: command.conversation_title.clone(),
                settings_json: None,
                created_at_ms: command.created_at_ms,
                updated_at_ms: command.created_at_ms,
            },
        )?;
        if !created.created{return Err("main_expert_selection_conversation_creation_conflict".into());}
        command.expected_configuration_revision=created.initial_configuration_revision;
    }
    if store
        .read_room_context_owner(parent)
        .map_err(|e| e.to_string())?
        .is_some()
        || store
            .conversation_execution_ownership(parent)
            .map_err(|e| e.to_string())?
            != nola_domain::ConversationExecutionOwnership::User
        || store
            .conversation_source_owner(parent)
            .map_err(|e| e.to_string())?
            != nola_domain::ConversationSourceOwner::Local
    {
        return Err("main_expert_selection_requires_main_conversation".into());
    }
    let mut unique = std::collections::BTreeSet::new();
    for selected in &input.selected_experts {
        if selected.agent_revision < 1
            || !unique.insert(selected.agent_id.clone())
            || selected.participant_id.is_some() != selected.context_conversation_id.is_some()
        {
            return Err("main_expert_selection_invalid_reference".into());
        }
    }
    let mut association =
        crate::agent_room::ensure_for_conversation(store, parent.clone(), command.created_at_ms)?;
    let mut members = vec![];
    for selected in &mut input.selected_experts {
        let agent = nola_domain::persistent_agent::AgentId::try_from(selected.agent_id.clone())?;
        let existing = association
            .room
            .room
            .participants()
            .iter()
            .find(|p| p.active && p.agent_id == agent)
            .cloned();
        let member = if let Some(participant) = &selected.participant_id {
            existing
                .filter(|p| {
                    p.id == *participant
                        && p.agent_revision == selected.agent_revision
                        && Some(&p.context_conversation_id)
                            == selected.context_conversation_id.as_ref()
                })
                .ok_or("main_expert_selection_member_changed")?
        } else if let Some(existing) = existing {
            if existing.agent_revision != selected.agent_revision {
                return Err("main_expert_selection_requires_explicit_rejoin".into());
            }
            // An unpinned directory choice must still match the observed directory.
            crate::persistent_agent::capture_defaults(
                &store.product_setting_repository(),
                &agent,
                selected.agent_revision,
            )?;
            existing
        } else {
            association.room = crate::agent_room::join(
                store,
                crate::agent_room::JoinAgent {
                    room_id: association.room_id.clone(),
                    expected_room_revision: association.room.revision,
                    agent_id: agent.clone(),
                    expected_agent_revision: selected.agent_revision,
                    at_ms: command.created_at_ms,
                },
            )?;
            association
                .room
                .room
                .participants()
                .iter()
                .find(|p| p.active && p.agent_id == agent)
                .cloned()
                .ok_or("main_expert_selection_join_missing")?
        };
        selected.participant_id = Some(member.id.clone());
        selected.context_conversation_id = Some(member.context_conversation_id.clone());
        members.push(json!({"participant_id":member.id,"agent_id":member.agent_id,"agent_revision":member.agent_revision,
            "context_conversation_id":member.context_conversation_id,"display_name":member.display_name,"description":member.description}));
    }
    component.content = json!({"version":1,"selected_experts":input.selected_experts,"room_id":association.room_id,"members":members,
        "guidance":"These are the user's selected experts. You remain the only agent communicating with the user. When useful, delegate bounded work using delegate_expert and integrate canonical Task results into your reply. A selection does not force a tool call or grant permissions."});
    Ok(())
}
