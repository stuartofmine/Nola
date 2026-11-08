// SPDX-License-Identifier: Apache-2.0
//! Private capture facts bind instruction bytes, never reusable resource grants.
use super::*;
use crate::expert_template_capture::{
    self as policy, ExpertTemplateCaptureEntry, ExpertTemplateCaptureFact,
    ExpertTemplateCaptureOrigin, HostExpertTemplateAuthority,
};
use nola_agent_engine::agent::resource_access::{
    AgentResourceAccessReason, AgentResourceKind, AgentResourcePermission,
    AgentResourcePrincipalKind,
};
use std::collections::BTreeSet;

fn identity(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 512 && !value.chars().any(char::is_control)
}
fn sha(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|v| v.is_ascii_digit() || (b'a'..=b'f').contains(&v))
}
fn selected(entry: &ExpertMessageEntry) -> Result<Option<String>, String> {
    match entry
        .defaults
        .conversation_configuration()
        .get("selectedPromptTemplateID")
    {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(value)) => Ok(Some(value.clone())),
        _ => Err("expert_template_invalid_frozen_selection".into()),
    }
}
pub(super) fn definitions(
    entry: &ExpertMessageEntry,
    fact: &ExpertTemplateCaptureEntry,
) -> Result<Vec<ExpertPromptDefinition>, String> {
    policy::validate_host_user_instructions(&fact.host_user_instructions)?;
    let mut result = entry
        .expected_prompt_definition
        .iter()
        .cloned()
        .collect::<Vec<_>>();
    result.extend(fact.host_user_instructions.clone());
    let mut ids = BTreeSet::new();
    for definition in &result {
        if !identity(&definition.component_id)
            || !ids.insert(&definition.component_id)
            || definition.content.len() > 262_144
            || definition.content.contains('\0')
            || definition.content_sha256
                != format!("{:x}", Sha256::digest(definition.content.as_bytes()))
        {
            return Err("expert_template_instruction_set_invalid_or_overlapping".into());
        }
    }
    result.sort_by(|a, b| a.component_id.cmp(&b.component_id));
    Ok(result)
}
pub(super) fn same_definitions(
    expected: &[ExpertPromptDefinition],
    actual: &[ExpertPromptDefinition],
) -> bool {
    if expected.len() != actual.len() {
        return false;
    }
    expected.iter().all(|wanted| {
        actual
            .iter()
            .filter(|value| {
                value.component_id == wanted.component_id
                    && value.content == wanted.content
                    && value.content_sha256 == wanted.content_sha256
                    && value.definition_revision == wanted.definition_revision
            })
            .count()
            == 1
    })
}
fn base(fact: &ExpertTemplateCaptureFact) -> Result<(), String> {
    HostExpertTemplateAuthority {
        actor: fact.actor.clone(),
        request_host_id: fact.request_host_id.clone(),
    }
    .validate()?;
    if fact.version != 1
        || !identity(&fact.conversation_id)
        || !identity(&fact.submission_id)
        || !identity(&fact.message_id)
        || !sha(&fact.intent_sha256)
        || fact.captured_at_ms < 0
        || fact.entries.is_empty()
        || fact.entries.len() > 32
    {
        return Err("expert_template_capture_fact_invalid".into());
    }
    let mut participants = BTreeSet::new();
    let mut contexts = BTreeSet::new();
    let mut children = BTreeSet::new();
    for entry in &fact.entries {
        if !identity(&entry.participant_id)
            || !participants.insert(&entry.participant_id)
            || !identity(&entry.context_conversation_id)
            || !contexts.insert(&entry.context_conversation_id)
            || !identity(&entry.child_submission_id)
            || !children.insert(&entry.child_submission_id)
            || entry.context_configuration_revision < 1
            || entry.definition_sha256.as_deref().is_some_and(|v| !sha(v))
        {
            return Err("expert_template_capture_entry_invalid".into());
        }
        policy::validate_host_user_instructions(&entry.host_user_instructions)?;
    }
    Ok(())
}
pub(super) fn read<S: ExpertMessageRepository + ?Sized>(
    store: &S,
    conversation: &str,
    submission: &str,
) -> Result<ExpertTemplateCaptureFact, String> {
    let raw = store
        .expert_template_capture_fact(conversation, submission)
        .map_err(|e| e.to_string())?
        .ok_or("expert_template_capture_witness_missing")?;
    if raw.len() > EXPERT_TEMPLATE_CAPTURE_MAX_BYTES {
        return Err("expert_template_capture_fact_exceeds_budget".into());
    }
    let fact: ExpertTemplateCaptureFact =
        serde_json::from_str(&raw).map_err(|_| "expert_template_capture_fact_invalid")?;
    base(&fact)?;
    if fact.conversation_id != conversation || fact.submission_id != submission {
        return Err("expert_template_capture_fact_owner_changed".into());
    }
    Ok(fact)
}
fn direct_origin(
    fact: &ExpertTemplateCaptureFact,
    captured: &ExpertTemplateCaptureEntry,
    entry: &ExpertMessageEntry,
) -> Result<(), String> {
    let definition = entry.expected_prompt_definition.as_ref();
    if captured.definition_sha256.as_deref() != definition.map(|v| v.content_sha256.as_str()) {
        return Err("expert_template_capture_definition_changed".into());
    }
    match &captured.origin {
        ExpertTemplateCaptureOrigin::NoTemplate => {
            if definition.is_some() || selected(entry)?.is_some() {
                return Err("expert_template_no_template_changed".into());
            }
        }
        ExpertTemplateCaptureOrigin::Custom {
            catalog_revision,
            resource,
            decision,
        } => {
            let definition = definition.ok_or("expert_template_custom_definition_missing")?;
            let selected = selected(entry)?.ok_or("expert_template_custom_selection_missing")?;
            let id = uuid::Uuid::parse_str(&selected)
                .map_err(|_| "expert_template_custom_selection_invalid")?;
            if *catalog_revision < 1
                || definition.definition_revision != Some(*catalog_revision as u64)
                || definition.component_id != format!("prompt_template:{selected}")
                || resource.kind != AgentResourceKind::PromptTemplate
                || resource.owner.kind == AgentResourcePrincipalKind::Anyone
                || !identity(&resource.owner.id)
                || resource
                    .host_id
                    .as_deref()
                    .is_some_and(|host| !identity(host))
                || resource.revision == 0
                || uuid::Uuid::parse_str(&resource.resource_id).ok() != Some(id)
                || resource.content_digest.as_deref()
                    != Some(format!("sha256:{}", definition.content_sha256).as_str())
                || decision.schema_version != 1
                || !decision.allowed
                || decision.tool_execution_authorized
                || decision.resource_kind != AgentResourceKind::PromptTemplate
                || decision.permission != AgentResourcePermission::ReadContent
                || decision.resource_id != resource.resource_id
            {
                return Err("expert_template_custom_capture_binding_changed".into());
            }
            match decision.reason {
                AgentResourceAccessReason::Owner if fact.actor == resource.owner => {}
                AgentResourceAccessReason::ExplicitGrant
                    if decision.matched_grant_id.as_deref().is_some_and(identity) => {}
                _ => return Err("expert_template_capture_decision_invalid".into()),
            }
        }
        ExpertTemplateCaptureOrigin::Builtin {
            provider_id,
            template_id,
        } => {
            let definition = definition.ok_or("expert_template_builtin_definition_missing")?;
            if !identity(provider_id)
                || uuid::Uuid::parse_str(template_id).is_err()
                || selected(entry)?.as_deref() != Some(template_id.as_str())
                || definition.component_id != format!("prompt_template:{template_id}")
                || definition.definition_revision.is_some()
            {
                return Err("expert_template_builtin_capture_binding_changed".into());
            }
        }
        ExpertTemplateCaptureOrigin::Retained { .. } => {
            return Err("expert_template_retained_capture_requires_root".into())
        }
    }
    Ok(())
}
pub(super) fn validate<S: ExpertMessageRepository + ?Sized>(
    store: &S,
    manifest: &ExpertMessageManifest,
    fact: &ExpertTemplateCaptureFact,
) -> Result<(), String> {
    manifest.validate()?;
    base(fact)?;
    if fact.conversation_id != manifest.input.conversation_id
        || fact.submission_id != manifest.input.submission_id
        || fact.message_id != manifest.input.user_message_id
        || fact.intent_sha256 != policy::payload_sha256(&manifest.input)?
        || fact.entries.len() != manifest.entries.len()
    {
        return Err("expert_template_capture_manifest_changed".into());
    }
    for (entry, captured) in manifest.entries.iter().zip(&fact.entries) {
        if captured.participant_id != entry.participant.participant_id
            || captured.context_conversation_id != entry.participant.context_conversation_id
            || captured.context_configuration_revision != entry.context_configuration_revision
            || captured.child_submission_id != entry.child_submission_id
        {
            return Err("expert_template_capture_participant_changed".into());
        }
        let expected = definitions(entry, captured)?;
        if let Some(witness) = &entry.instruction_witness {
            if !same_definitions(&expected, &witness.prompt_definitions) {
                return Err("expert_template_actual_instruction_set_changed".into());
            }
        }
        if let ExpertTemplateCaptureOrigin::Retained {
            source_conversation_id,
            source_submission_id,
            source_participant_id,
            source_capture_sha256,
        } = &captured.origin
        {
            let original = read(store, source_conversation_id, source_submission_id)?;
            if policy::payload_sha256(&original)? != *source_capture_sha256
                || original.actor != fact.actor
                || original.request_host_id != fact.request_host_id
            {
                return Err("expert_template_retained_capture_owner_changed".into());
            }
            let original_entry = original
                .entries
                .iter()
                .find(|value| value.participant_id == *source_participant_id)
                .ok_or("expert_template_retained_capture_entry_missing")?;
            if original_entry.definition_sha256 != captured.definition_sha256
                || original_entry.host_user_instructions != captured.host_user_instructions
            {
                return Err("expert_template_retained_definition_changed".into());
            }
            // Current producers flatten to the original capture, not a chain of
            // freshly claimed resource decisions or unbounded history walks.
            direct_origin(&original, original_entry, entry)?;
        } else {
            direct_origin(fact, captured, entry)?;
        }
    }
    Ok(())
}
pub(super) fn authorized<S: ExpertMessageRepository + ConversationProjectionRepository + ?Sized>(
    store: &S,
    manifest: &ExpertMessageManifest,
    authority: &HostExpertTemplateAuthority,
) -> Result<ExpertTemplateCaptureFact, String> {
    authority.validate()?;
    if store
        .conversation_source_owner(&manifest.input.conversation_id)
        .map_err(|e| e.to_string())?
        != ConversationSourceOwner::Local
    {
        return Err("expert_template_source_not_local".into());
    }
    let fact = read(
        store,
        &manifest.input.conversation_id,
        &manifest.input.submission_id,
    )?;
    if fact.actor != authority.actor || fact.request_host_id != authority.request_host_id {
        return Err("expert_template_source_access_denied".into());
    }
    validate(store, manifest, &fact)?;
    Ok(fact)
}
pub(super) fn persist<S: ExpertMessageRepository + ?Sized>(
    store: &mut S,
    fact: &ExpertTemplateCaptureFact,
) -> Result<(), String> {
    base(fact)?;
    let raw = serde_json::to_string(fact).map_err(|e| e.to_string())?;
    if raw.len() > EXPERT_TEMPLATE_CAPTURE_MAX_BYTES {
        return Err("expert_template_capture_fact_exceeds_budget".into());
    }
    store
        .put_expert_template_capture_fact(
            &fact.conversation_id,
            &fact.submission_id,
            &raw,
            fact.captured_at_ms,
        )
        .map_err(|e| e.to_string())
}
pub(super) fn retained_origin(
    fact: &ExpertTemplateCaptureFact,
    entry: &ExpertTemplateCaptureEntry,
) -> Result<ExpertTemplateCaptureOrigin, String> {
    Ok(match &entry.origin {
        ExpertTemplateCaptureOrigin::Retained { .. } => entry.origin.clone(),
        _ => ExpertTemplateCaptureOrigin::Retained {
            source_conversation_id: fact.conversation_id.clone(),
            source_submission_id: fact.submission_id.clone(),
            source_participant_id: entry.participant_id.clone(),
            source_capture_sha256: policy::payload_sha256(fact)?,
        },
    })
}
