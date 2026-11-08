// SPDX-License-Identifier: Apache-2.0
//! Fresh-owner edited resend preparation. No old history, grants, or effects.
use nola_domain::edited_resend::{FrozenResendModel, PrepareEditedResend, PreparedEditedResend};
use nola_domain::{ConversationSourceOwner, MessagePartView, MessageRole, MessageState};
use nola_store_port::conversation_history::ConversationProjectionRepository;
use nola_store_port::conversation_profile::ConversationWriteRepositoryProvider;
use nola_store_port::edited_resend::EditedResendRepository;
use nola_store_port::protocol_journal::{
    CommandProtocolRepositoryProvider, CommandTransactionRepository,
};
use nola_store_port::run_recovery_inspection::RunRecoveryInspectionRepository;
use sha2::{Digest, Sha256};

pub fn prepare<S>(store: &mut S, input: PrepareEditedResend) -> Result<PreparedEditedResend, String>
where
    S: EditedResendRepository
        + ConversationProjectionRepository
        + ConversationWriteRepositoryProvider
        + CommandProtocolRepositoryProvider
        + RunRecoveryInspectionRepository,
{
    validate_input(&input)?;
    store
        .command_protocol_repository()
        .begin_command_mutations()
        .map_err(|e| e.to_string())?;
    let result = prepare_inner(store, input, None);
    match result {
        Ok(result) => {
            store
                .command_protocol_repository()
                .commit_command_mutations()
                .map_err(|e| e.to_string())?;
            Ok(result)
        }
        Err(error) => {
            store
                .command_protocol_repository()
                .rollback_command_mutations()
                .map_err(|rollback| {
                    format!("{error}; edited resend rollback failed: {rollback}")
                })?;
            Err(error)
        }
    }
}

fn validate_input(input: &PrepareEditedResend) -> Result<(), String> {
    for id in [&input.operation_id, &input.target_conversation_id] {
        uuid::Uuid::parse_str(id).map_err(|_| "edited_resend_requires_stable_uuid_identities")?;
    }
    for id in [&input.source_conversation_id, &input.source_message_id] {
        if id.trim().is_empty()
            || id.trim() != id
            || id.len() > 512
            || id.chars().any(char::is_control)
        {
            return Err("edited_resend_invalid_source_identity".into());
        }
    }
    if input.source_conversation_id == input.target_conversation_id
        || input.expected_source_revision < 1
        || input.expected_source_message_revision < 1
        || input.created_at_ms < 0
        || input.edited_text.trim().is_empty()
        || input.edited_text.len() > 1_000_000
        || input.edited_text.contains('\0')
        || input.title.trim().is_empty()
        || input.title.len() > 512
        || input.title.chars().any(char::is_control)
    {
        return Err("edited_resend_invalid_input".into());
    }
    Ok(())
}

fn prepare_inner<S>(
    store: &mut S,
    input: PrepareEditedResend,
    verified_expert: Option<(
        nola_domain::edited_resend::MessageSendWitness,
        serde_json::Value,
    )>,
) -> Result<PreparedEditedResend, String>
where
    S: EditedResendRepository
        + ConversationProjectionRepository
        + ConversationWriteRepositoryProvider
        + RunRecoveryInspectionRepository,
{
    if let Some(previous) = store
        .prepared_edited_resend_by_operation(&input.operation_id)
        .map_err(|e| e.to_string())?
    {
        if previous.input != input {
            return Err("edited_resend_operation_conflict".into());
        }
        return Ok(previous);
    }
    if store
        .conversation_source_owner(&input.source_conversation_id)
        .map_err(|e| e.to_string())?
        != ConversationSourceOwner::Local
    {
        return Err("edited_resend_requires_exact_local_source".into());
    }
    if store
        .active_run(&input.source_conversation_id)
        .map_err(|e| e.to_string())?
        .is_some()
    {
        return Err("edited_resend_source_has_active_run".into());
    }
    let conversation = store
        .load_conversation(&input.source_conversation_id)
        .map_err(|e| e.to_string())?
        .ok_or("edited_resend_source_not_found")?;
    if conversation.revision != input.expected_source_revision {
        return Err("edited_resend_source_revision_changed".into());
    }
    let source_settings: serde_json::Value =
        serde_json::from_str(conversation.settings_json.as_deref().unwrap_or("{}"))
            .map_err(|_| "edited_resend_invalid_source_configuration")?;
    if verified_expert.is_none()
        && source_settings
            .get(nola_domain::agent_room::CONTEXT_OWNER_FIELD)
            .is_some_and(|value| !value.is_null())
    {
        return Err("edited_resend_expert_snapshot_unavailable".into());
    }
    let projection = crate::load_conversation_projection_v1(store, &input.source_conversation_id)
        .map_err(|e| e.to_string())?;
    let message = projection
        .messages
        .iter()
        .find(|m| m.message_id.0 == input.source_message_id)
        .ok_or("edited_resend_source_message_not_visible")?;
    if message.role != MessageRole::User
        || message.state != MessageState::Completed
        || message.revision != input.expected_source_message_revision
    {
        return Err("edited_resend_source_message_revision_or_kind_changed".into());
    }
    let [MessagePartView::Text {
        markdown,
        is_final: true,
        ..
    }] = message.parts.as_slice()
    else {
        return Err("edited_resend_source_send_snapshot_unavailable".into());
    };
    let stored = conversation
        .messages
        .iter()
        .find(|m| m.id == input.source_message_id)
        .ok_or("edited_resend_source_message_not_found")?;
    // Expert-only parents currently have no complete frozen expert-set witness.
    // A subset of Room child submissions is never treated as that snapshot.
    let run_id = stored
        .run_id
        .as_ref()
        .ok_or("edited_resend_expert_snapshot_unavailable")?;
    let source = if let Some((source, _)) = &verified_expert {
        source.clone()
    } else {
        store
            .message_send_witness(&input.source_conversation_id, &input.source_message_id)
            .map_err(|e| e.to_string())?
            .ok_or("edited_resend_source_send_snapshot_unavailable")?
    };
    if source.conversation_id != input.source_conversation_id
        || source.message_id != input.source_message_id
    {
        return Err("edited_resend_verified_source_owner_mismatch".into());
    }
    if stored.role != "user"
        || stored.status != "completed"
        || source.run_id != *run_id
        || source.body_sha256 != format!("{:x}", Sha256::digest(markdown.as_bytes()))
    {
        return Err("edited_resend_source_body_or_owner_changed".into());
    }
    let run = store
        .inspect_exact_run(run_id)
        .map_err(|e| e.to_string())?
        .ok_or("edited_resend_source_run_missing")?;
    if run.conversation_id != input.source_conversation_id
        || run.assistant_message_id.as_deref() != Some(source.assistant_message_id.as_str())
        || run.kind != "conversation"
        || !run.assistant_owner_valid
        || run.source_host_id.is_some()
        || run.status != "sealed"
        || run.terminal.is_none()
    {
        return Err("edited_resend_source_run_owner_unavailable".into());
    }
    let frozen = store
        .load_run_invocation_config(run_id)
        .map_err(|e| e.to_string())?
        .ok_or("edited_resend_source_send_snapshot_unavailable")?;
    let actual = format!(
        "{:x}",
        Sha256::digest(frozen.invocation_config_json.as_bytes())
    );
    let config: serde_json::Value = serde_json::from_str(&frozen.invocation_config_json)
        .map_err(|_| "edited_resend_invalid_frozen_config")?;
    if frozen.invocation_config_digest != source.invocation_config_sha256
        || actual != source.invocation_config_sha256
        || FrozenResendModel::from_invocation(&config).as_ref() != Some(&source.model)
    {
        return Err("edited_resend_source_snapshot_integrity_failure".into());
    }
    validate_model(&source.model)?;
    if store
        .load_conversation(&input.target_conversation_id)
        .map_err(|e| e.to_string())?
        .is_some()
    {
        return Err("edited_resend_target_already_exists".into());
    }
    let prepared = PreparedEditedResend {
        user_message_id: format!("{}:user", input.operation_id),
        assistant_message_id: format!("{}:assistant", input.operation_id),
        run_id: input.operation_id.clone(),
        input,
        source,
        admitted_run_id: None,
    };
    if prepared.run_id == prepared.source.run_id {
        return Err("edited_resend_requires_new_run_identity".into());
    }
    let created = crate::conversation::create_conversation(
        &mut store.conversation_write_repository(),
        crate::conversation::CreateConversation {
            id: prepared.input.target_conversation_id.clone(),
            title: prepared.input.title.clone(),
            settings_json: Some(
                verified_expert
                    .map(|(_, settings)| settings)
                    .unwrap_or_else(|| serde_json::json!({"permissionProfile":"cautious"}))
                    .to_string(),
            ),
            created_at_ms: prepared.input.created_at_ms,
            updated_at_ms: prepared.input.created_at_ms,
        },
    )?;
    if !created.created {
        return Err("edited_resend_target_already_exists".into());
    }
    store
        .put_prepared_edited_resend(&prepared)
        .map_err(|e| e.to_string())?;
    Ok(prepared)
}

/// Called only after the complete expert manifest proved this exact source entry.
/// All ordinary source body, revision, sealed Run and invocation checks stay shared.
pub(crate) fn prepare_verified_expert_child<S>(
    store: &mut S,
    input: PrepareEditedResend,
    source: nola_domain::edited_resend::MessageSendWitness,
    settings: serde_json::Value,
) -> Result<PreparedEditedResend, String>
where
    S: EditedResendRepository
        + ConversationProjectionRepository
        + ConversationWriteRepositoryProvider
        + RunRecoveryInspectionRepository,
{
    validate_input(&input)?;
    let fields = settings
        .as_object()
        .ok_or("expert_resend_invalid_context_settings")?;
    if fields.get("permissionProfile").and_then(|v| v.as_str()) != Some("cautious")
        || !fields.contains_key(nola_domain::agent_room::CONTEXT_OWNER_FIELD)
    {
        return Err("expert_resend_fresh_context_owner_required".into());
    }
    prepare_inner(store, input, Some((source, settings)))
}

fn validate_model(model: &FrozenResendModel) -> Result<(), String> {
    let profile: nola_agent_engine::agent::ModelCapabilityProfile =
        serde_json::from_value(model.model_capability.clone())
            .map_err(|_| "edited_resend_model_capability_unavailable")?;
    let identity = nola_agent_engine::agent::protocol::AgentTurnIdentity {
        model_capability: profile.clone(),
        ..Default::default()
    };
    identity
        .validate_model_capability()
        .map_err(|_| "edited_resend_model_capability_unavailable")?;
    if !profile.direct_tool_capability_ids.is_empty()
        || profile.context_window_tokens == Some(0)
        || profile.max_output_tokens == Some(0)
        || serde_json::to_value(&profile)
            .map_err(|_| "edited_resend_model_capability_unavailable")?
            != model.model_capability
    {
        return Err("edited_resend_model_capability_unavailable".into());
    }
    // Construct only for existing typed model validators. The zero policy hash
    // is not persisted or dispatched; each new Host must supply fresh policy.
    let binding: nola_host_protocol::HostInvocationBinding =
        serde_json::from_value(serde_json::json!({
            "version":model.version,"primary":model.primary,"fallbacks":model.fallbacks,
            "generation":model.generation,"output_admission":model.output_admission,
            "execution_policy_sha256":"0".repeat(64),
        }))
        .map_err(|_| "edited_resend_model_snapshot_unavailable")?;
    binding
        .validate()
        .map_err(|_| "edited_resend_model_snapshot_unavailable".into())
}

/// Called only after the trusted Host has rebuilt its target's catalog and
/// product/global/OS policy, with source-scoped grants explicitly excluded.
/// A hash binds that fact to admission; it is not itself an authority proof.
pub fn attest_fresh_policy<
    S: EditedResendRepository + nola_store_port::expert_message::ExpertMessageRepository,
>(
    store: &S,
    target: &str,
    config: &nola_agent_engine::agent::AgentCoreConfig,
    submission: &nola_agent_engine::agent::AgentSubmission,
) -> Result<nola_domain::edited_resend::FreshResendPolicyWitness, String> {
    use nola_domain::edited_resend::{canonical_resend_value_sha256, FreshResendPolicyWitness};
    let prepared = store
        .prepared_edited_resend(target)
        .map_err(|e| e.to_string())?
        .ok_or("edited_resend_preparation_not_found")?;
    if prepared.admitted_run_id.is_some()
        || config.session_id.as_ref().map(|id| id.as_str()) != Some(target)
        || !config.initial_items.is_empty()
        || config.automation_execution_spec.is_some()
        || config.permission_rules.rules.iter().any(|rule| {
            matches!(
                rule.source,
                nola_domain::PermissionRuleSource::Conversation
                    | nola_domain::PermissionRuleSource::Turn
            )
        })
    {
        return Err("edited_resend_policy_requires_fresh_target_configuration".into());
    }
    if config
        .turn_identity
        .remote_tool_plan
        .as_ref()
        .is_some_and(|plan| plan.reference.session_id != target)
    {
        return Err("edited_resend_tool_plan_belongs_to_another_owner".into());
    }
    let binding = config
        .turn_identity
        .host_invocation
        .as_ref()
        .ok_or("edited_resend_fresh_host_binding_required")?;
    binding.validate().map_err(str::to_string)?;
    let value =
        serde_json::to_value(config).map_err(|_| "edited_resend_invalid_target_configuration")?;
    if FrozenResendModel::from_invocation(&value).as_ref() != Some(&prepared.source.model) {
        return Err("edited_resend_model_differs_from_prepared_intent".into());
    }
    let nola_agent_engine::agent::AgentSubmission::UserInput {
        items,
        turn_context_items,
        turn_context_platform,
        turn_context_components,
        submission_id,
        delivery,
        ..
    } = submission
    else {
        return Err("edited_resend_requires_direct_user_submission".into());
    };
    if submission_id.as_deref() != Some(prepared.run_id.as_str())
        || *delivery != nola_agent_engine::agent::AgentInputDelivery::CurrentTurn
    {
        return Err("edited_resend_submission_owner_changed".into());
    }
    nola_domain::edited_resend_context::validate_items(
        &prepared,
        &serde_json::to_value(items).map_err(|e| e.to_string())?,
        &serde_json::to_value(turn_context_items).map_err(|e| e.to_string())?,
    )?;
    let definitions = store
        .expert_resend_child_instructions(target)
        .map_err(|e| e.to_string())?
        .unwrap_or_default();
    let context_components_sha256 = nola_domain::edited_resend_context::validate_components(
        &definitions,
        &serde_json::to_value(turn_context_components).map_err(|e| e.to_string())?,
    )?;
    let catalog = config
        .host_catalog
        .as_ref()
        .ok_or("edited_resend_fresh_catalog_required")?;
    let input_payload_sha256 = nola_agent_engine::agent::accepted_input_payload_digest(
        &serde_json::json!({
        "origin":"interactive_user","delivery":delivery,"items":items,
        "turn_context_items":turn_context_items,"platform":turn_context_platform,"components":turn_context_components}),
    );
    Ok(FreshResendPolicyWitness {
        context_components_sha256,
        input_payload_sha256,
        operation_id: prepared.input.operation_id,
        conversation_id: target.into(),
        excluded_source_conversation_id: prepared.input.source_conversation_id,
        host_id: catalog.host_id.0.clone(),
        catalog_revision: catalog.revision,
        catalog_sha256: canonical_resend_value_sha256(
            value
                .get("host_catalog")
                .ok_or("edited_resend_missing_catalog")?,
        )?,
        permission_rules_sha256: canonical_resend_value_sha256(
            value
                .get("permission_rules")
                .ok_or("edited_resend_missing_rules")?,
        )?,
        execution_policy_sha256: binding.execution_policy_sha256.clone(),
    })
}
