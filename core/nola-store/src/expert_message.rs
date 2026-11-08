// SPDX-License-Identifier: Apache-2.0
//! Private complete-set witnesses. No generic setting/sync access and no subset repair.
use super::*;
use nola_domain::{
    edited_resend::{FrozenResendModel, MessageSendWitness, EDITED_RESEND_PREFIX},
    expert_message::*,
};
use nola_store_port::{
    expert_message::ExpertMessageRepository,
    run_creation::BeginTurnInput,
};
use serde::{de::DeserializeOwned, Serialize};
fn key(kind: &str, owner: impl Serialize) -> StoreResult<String> {
    Ok(format!(
        "{EDITED_RESEND_PREFIX}expert/{kind}/{:x}",
        Sha256::digest(serde_json::to_vec(&owner)?)
    ))
}
fn read<T: DeserializeOwned>(connection: &Connection, key: &str) -> StoreResult<Option<T>> {
    let raw: Option<String> = connection
        .query_row(
            "SELECT value_json FROM product_settings WHERE key=?1",
            [key],
            |r| r.get(0),
        )
        .optional()?;
    raw.map(|raw| serde_json::from_str(&raw).map_err(StoreError::from))
        .transpose()
}
fn read_capture_fact(
    connection: &Connection,
    conversation: &str,
    submission: &str,
) -> StoreResult<Option<serde_json::Value>> {
    let raw: Option<String> = connection
        .query_row(
            "SELECT value_json FROM product_settings WHERE key=?1",
            [key("template_capture", (conversation, submission))?],
            |row| row.get(0),
        )
        .optional()?;
    raw.map(|raw| {
        if raw.len() > EXPERT_TEMPLATE_CAPTURE_MAX_BYTES {
            return Err(StoreError::Invariant(
                "expert_template_capture_fact_exceeds_budget".into(),
            ));
        }
        serde_json::from_str(&raw).map_err(StoreError::from)
    })
    .transpose()
}
fn immutable(
    connection: &Connection,
    key: &str,
    value: &impl Serialize,
    at: i64,
) -> StoreResult<()> {
    if connection.is_autocommit() {
        return Err(StoreError::Invariant(
            "expert_message_requires_transaction".into(),
        ));
    }
    let raw = serde_json::to_string(value)?;
    let prior: Option<String> = connection
        .query_row(
            "SELECT value_json FROM product_settings WHERE key=?1",
            [key],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(prior) = prior {
        return if prior == raw {
            Ok(())
        } else {
            Err(StoreError::Conflict(
                "expert_message_immutable_conflict".into(),
            ))
        };
    }
    connection.execute(
        "INSERT INTO product_settings(key,value_json,revision,updated_at_ms) VALUES(?1,?2,1,?3)",
        params![key, raw, at],
    )?;
    Ok(())
}
fn reservation_pending(
    connection: &Connection,
    reservation: &ExpertContextReservation,
) -> StoreResult<bool> {
    let raw: Option<String> = connection
        .query_row(
            "SELECT value_json FROM product_settings WHERE key=?1",
            [format!(
                "agent_room_submission/{}/{}",
                reservation.room_id, reservation.child_submission_id
            )],
            |r| r.get(0),
        )
        .optional()?;
    let value: serde_json::Value = serde_json::from_str(&raw.ok_or_else(|| {
        StoreError::Invariant("expert_context_reservation_receipt_missing".into())
    })?)?;
    if value.pointer("/input/room_id").and_then(|v| v.as_str())
        != Some(reservation.room_id.as_str())
        || value
            .pointer("/input/submission_id")
            .and_then(|v| v.as_str())
            != Some(reservation.child_submission_id.as_str())
        || value
            .get("context_conversation_id")
            .and_then(|v| v.as_str())
            != Some(reservation.context_conversation_id.as_str())
    {
        return Err(StoreError::Invariant(
            "expert_context_reservation_receipt_owner_changed".into(),
        ));
    }
    match value.get("state").and_then(|v| v.as_str()) {
        Some("selected" | "executing") => Ok(true),
        Some("completed" | "incomplete" | "blocked" | "failed" | "cancelled") => Ok(false),
        _ => Err(StoreError::Invariant(
            "expert_context_reservation_invalid_receipt_state".into(),
        )),
    }
}
fn reserve(
    connection: &Connection,
    value: &ExpertMessageManifest,
    entry: &ExpertMessageEntry,
) -> StoreResult<()> {
    let key = key(
        "context_reservation",
        &entry.participant.context_conversation_id,
    )?;
    let reservation = ExpertContextReservation {
        room_id: value.input.room_id.clone(),
        parent_conversation_id: value.input.conversation_id.clone(),
        parent_message_id: value.input.user_message_id.clone(),
        context_conversation_id: entry.participant.context_conversation_id.clone(),
        child_submission_id: entry.child_submission_id.clone(),
    };
    if let Some(prior) = read::<ExpertContextReservation>(connection, &key)? {
        if prior == reservation {
            return Ok(());
        }
        if reservation_pending(connection, &prior)? {
            return Err(StoreError::Conflict(
                "expert_context_reserved_by_another_parent".into(),
            ));
        }
        connection.execute("UPDATE product_settings SET value_json=?2,revision=revision+1,updated_at_ms=?3 WHERE key=?1",
            params![key,serde_json::to_string(&reservation)?,value.input.created_at_ms])?;
        Ok(())
    } else {
        immutable(connection, &key, &reservation, value.input.created_at_ms)
    }
}

impl ExpertMessageRepository for NolaStore {
    type Error = StoreError;
    fn expert_message_manifest(
        &self,
        conversation: &str,
        message: &str,
    ) -> StoreResult<Option<ExpertMessageManifest>> {
        let value: Option<ExpertMessageManifest> =
            read(&self.connection, &key("source", (conversation, message))?)?;
        if let Some(value) = &value {
            value.validate().map_err(StoreError::Invariant)?;
            if value.input.conversation_id != conversation || value.input.user_message_id != message
            {
                return Err(StoreError::Invariant(
                    "expert_message_source_owner_mismatch".into(),
                ));
            }
        }
        Ok(value)
    }
    fn expert_message_by_submission(
        &self,
        conversation: &str,
        submission: &str,
    ) -> StoreResult<Option<ExpertMessageManifest>> {
        let Some(message): Option<String> = read(
            &self.connection,
            &key("submission", (conversation, submission))?,
        )?
        else {
            return Ok(None);
        };
        let value = self
            .expert_message_manifest(conversation, &message)?
            .ok_or_else(|| {
                StoreError::Invariant("expert_message_submission_source_missing".into())
            })?;
        if value.input.submission_id != submission {
            return Err(StoreError::Invariant(
                "expert_message_submission_owner_mismatch".into(),
            ));
        }
        Ok(Some(value))
    }
    fn expert_message_for_child(
        &self,
        context: &str,
        run: &str,
    ) -> StoreResult<Option<ExpertMessageManifest>> {
        let Some((conversation, message)): Option<(String, String)> =
            read(&self.connection, &key("child", (context, run))?)?
        else {
            return Ok(None);
        };
        let manifest = self
            .expert_message_manifest(&conversation, &message)?
            .ok_or_else(|| StoreError::Invariant("expert_message_child_manifest_missing".into()))?;
        if manifest
            .entries
            .iter()
            .filter(|entry| {
                entry.participant.context_conversation_id == context
                    && entry.child_submission_id == run
            })
            .count()
            != 1
        {
            return Err(StoreError::Invariant(
                "expert_message_child_owner_changed".into(),
            ));
        }
        Ok(Some(manifest))
    }
    fn expert_template_capture_fact(
        &self,
        conversation: &str,
        submission: &str,
    ) -> StoreResult<Option<String>> {
        let value = read_capture_fact(&self.connection, conversation, submission)?;
        value
            .map(|value| serde_json::to_string(&value).map_err(StoreError::from))
            .transpose()
    }
    fn put_expert_template_capture_fact(
        &mut self,
        conversation: &str,
        submission: &str,
        fact_json: &str,
        captured_at_ms: i64,
    ) -> StoreResult<()> {
        if fact_json.len() > EXPERT_TEMPLATE_CAPTURE_MAX_BYTES || captured_at_ms < 0 {
            return Err(StoreError::Invariant(
                "expert_template_capture_fact_exceeds_budget".into(),
            ));
        }
        let value: serde_json::Value = serde_json::from_str(fact_json)?;
        if value.get("version").and_then(|v| v.as_u64()) != Some(1)
            || value.get("conversation_id").and_then(|v| v.as_str()) != Some(conversation)
            || value.get("submission_id").and_then(|v| v.as_str()) != Some(submission)
            || value.get("captured_at_ms").and_then(|v| v.as_i64()) != Some(captured_at_ms)
        {
            return Err(StoreError::Invariant(
                "expert_template_capture_fact_owner_changed".into(),
            ));
        }
        immutable(
            &self.connection,
            &key("template_capture", (conversation, submission))?,
            &value,
            captured_at_ms,
        )
    }
    fn expert_context_reservation(
        &self,
        context: &str,
    ) -> StoreResult<Option<ExpertContextReservation>> {
        let value: Option<ExpertContextReservation> =
            read(&self.connection, &key("context_reservation", context)?)?;
        if value
            .as_ref()
            .is_some_and(|v| v.context_conversation_id != context)
        {
            return Err(StoreError::Invariant(
                "expert_context_reservation_owner_changed".into(),
            ));
        }
        Ok(value)
    }
    fn put_expert_message_manifest(&mut self, value: &ExpertMessageManifest) -> StoreResult<()> {
        value.validate().map_err(StoreError::Invariant)?;
        if value
            .entries
            .iter()
            .any(|e| e.model_witness.is_some() || e.instruction_witness.is_some())
        {
            return Err(StoreError::Conflict(
                "expert_message_model_witness_requires_actual_run".into(),
            ));
        }
        let owner = (&value.input.conversation_id, &value.input.user_message_id);
        let source = key("source", owner)?;
        immutable(&self.connection, &source, value, value.input.created_at_ms)?;
        immutable(
            &self.connection,
            &key(
                "submission",
                (&value.input.conversation_id, &value.input.submission_id),
            )?,
            &value.input.user_message_id,
            value.input.created_at_ms,
        )?;
        for entry in &value.entries {
            reserve(&self.connection, value, entry)?;
            immutable(
                &self.connection,
                &key(
                    "child",
                    (
                        &entry.participant.context_conversation_id,
                        &entry.child_submission_id,
                    ),
                )?,
                &owner,
                value.input.created_at_ms,
            )?;
        }
        Ok(())
    }
    fn prepared_expert_resend(&self, target: &str) -> StoreResult<Option<PreparedExpertResend>> {
        let value: Option<PreparedExpertResend> =
            read(&self.connection, &key("prepared", target)?)?;
        if value
            .as_ref()
            .is_some_and(|v| v.input.target_conversation_id != target)
        {
            return Err(StoreError::Invariant(
                "expert_resend_target_owner_mismatch".into(),
            ));
        }
        Ok(value)
    }
    fn prepared_expert_resend_by_operation(
        &self,
        operation: &str,
    ) -> StoreResult<Option<PreparedExpertResend>> {
        let Some(target): Option<String> = read(&self.connection, &key("operation", operation)?)?
        else {
            return Ok(None);
        };
        let value = self.prepared_expert_resend(&target)?.ok_or_else(|| {
            StoreError::Invariant("expert_resend_operation_target_missing".into())
        })?;
        if value.input.operation_id != operation {
            return Err(StoreError::Invariant(
                "expert_resend_operation_owner_mismatch".into(),
            ));
        }
        Ok(Some(value))
    }
    fn expert_resend_child_instructions(
        &self,
        target: &str,
    ) -> StoreResult<Option<Vec<ExpertPromptDefinition>>> {
        child_instructions(&self.connection, target)
    }
    fn put_prepared_expert_resend(&mut self, value: &PreparedExpertResend) -> StoreResult<()> {
        immutable(
            &self.connection,
            &key("prepared", &value.input.target_conversation_id)?,
            value,
            value.input.created_at_ms,
        )?;
        immutable(
            &self.connection,
            &key("operation", &value.input.operation_id)?,
            &value.input.target_conversation_id,
            value.input.created_at_ms,
        )
    }
}
/// Read only the engine-free instruction projection of the private capture
/// fact. Authorization decisions are made by Application at capture, not here.
fn captured_instruction_sets(
    connection: &Connection,
    manifest: &ExpertMessageManifest,
) -> StoreResult<Vec<Vec<ExpertPromptDefinition>>> {
    let fact = read_capture_fact(
        connection,
        &manifest.input.conversation_id,
        &manifest.input.submission_id,
    )?
    .ok_or_else(|| StoreError::Conflict("expert_template_capture_witness_missing".into()))?;
    let digest = format!("{:x}", Sha256::digest(serde_json::to_vec(&manifest.input)?));
    let captured = fact
        .get("entries")
        .and_then(|v| v.as_array())
        .ok_or_else(|| StoreError::Invariant("expert_template_capture_entries_missing".into()))?;
    if fact.get("version").and_then(|v| v.as_u64()) != Some(1)
        || fact.get("conversation_id").and_then(|v| v.as_str())
            != Some(manifest.input.conversation_id.as_str())
        || fact.get("submission_id").and_then(|v| v.as_str())
            != Some(manifest.input.submission_id.as_str())
        || fact.get("message_id").and_then(|v| v.as_str())
            != Some(manifest.input.user_message_id.as_str())
        || fact.get("intent_sha256").and_then(|v| v.as_str()) != Some(digest.as_str())
        || captured.len() != manifest.entries.len()
    {
        return Err(StoreError::Invariant(
            "expert_template_capture_manifest_changed".into(),
        ));
    }
    manifest
        .entries
        .iter()
        .zip(captured)
        .map(|(entry, fact)| {
            if fact.get("participant_id").and_then(|v| v.as_str())
                != Some(entry.participant.participant_id.as_str())
                || fact.get("context_conversation_id").and_then(|v| v.as_str())
                    != Some(entry.participant.context_conversation_id.as_str())
                || fact
                    .get("context_configuration_revision")
                    .and_then(|v| v.as_i64())
                    != Some(entry.context_configuration_revision)
                || fact.get("child_submission_id").and_then(|v| v.as_str())
                    != Some(entry.child_submission_id.as_str())
                || fact.get("definition_sha256").and_then(|v| v.as_str())
                    != entry
                        .expected_prompt_definition
                        .as_ref()
                        .map(|v| v.content_sha256.as_str())
            {
                return Err(StoreError::Invariant(
                    "expert_template_capture_entry_changed".into(),
                ));
            }
            let registered: Vec<ExpertPromptDefinition> =
                serde_json::from_value(fact.get("host_user_instructions").cloned().ok_or_else(
                    || StoreError::Invariant("expert_host_instruction_registration_missing".into()),
                )?)?;
            if registered.len() > 8 || registered.iter().any(|v| v.definition_revision.is_some()) {
                return Err(StoreError::Invariant(
                    "expert_host_instruction_registration_invalid".into(),
                ));
            }
            let mut expected = entry
                .expected_prompt_definition
                .iter()
                .cloned()
                .collect::<Vec<_>>();
            expected.extend(registered);
            let mut ids = std::collections::BTreeSet::new();
            if expected.iter().any(|value| {
                value.component_id.trim().is_empty()
                    || value.component_id.len() > 512
                    || !ids.insert(&value.component_id)
                    || value.content.len() > 262_144
                    || value.content.contains('\0')
                    || value.content_sha256
                        != format!("{:x}", Sha256::digest(value.content.as_bytes()))
            }) {
                return Err(StoreError::Invariant(
                    "expert_template_capture_instruction_set_invalid".into(),
                ));
            }
            Ok(expected)
        })
        .collect()
}

/// Runs in the exact BeginTurn transaction after the actual invocation is stored.
/// No renderer command can manufacture or patch one successful subset.
pub(crate) fn record_child_send(
    connection: &Connection,
    input: &BeginTurnInput,
) -> StoreResult<()> {
    if let Some(reservation) = read::<ExpertContextReservation>(
        connection,
        &key("context_reservation", &input.conversation_id)?,
    )? {
        if reservation.child_submission_id != input.run_id
            && reservation_pending(connection, &reservation)?
        {
            return Err(StoreError::Conflict(
                "expert_context_reserved_by_another_parent".into(),
            ));
        }
    }
    let Some((conversation, message)): Option<(String, String)> = read(
        connection,
        &key("child", (&input.conversation_id, &input.run_id))?,
    )?
    else {
        return Ok(());
    };
    let source_key = key("source", (&conversation, &message))?;
    let mut manifest: ExpertMessageManifest = read(connection, &source_key)?
        .ok_or_else(|| StoreError::Invariant("expert_message_child_manifest_missing".into()))?;
    manifest.validate().map_err(StoreError::Invariant)?;
    let captured_instructions = captured_instruction_sets(connection, &manifest)?;
    let entry_index = manifest
        .entries
        .iter()
        .position(|entry| {
            entry.participant.context_conversation_id == input.conversation_id
                && entry.child_submission_id == input.run_id
        })
        .ok_or_else(|| {
            StoreError::Invariant("expert_message_child_not_in_complete_manifest".into())
        })?;
    let expected_instructions = &captured_instructions[entry_index];
    let entry = &mut manifest.entries[entry_index];
    let protected_resend: Option<nola_domain::edited_resend::PreparedEditedResend> = read(
        connection,
        &format!(
            "{EDITED_RESEND_PREFIX}prepared/{:x}",
            Sha256::digest(serde_json::to_vec(&input.conversation_id)?)
        ),
    )?;
    let fresh_resend = protected_resend
        .as_ref()
        .is_some_and(|p| p.run_id == input.run_id && p.user_message_id == input.user_message_id);
    if input.user_message_id != entry.child_user_message_id
        || input.assistant_message_id != entry.child_assistant_message_id
        || input.user_text != manifest.input.text
        || input.user_part_kind != "text"
        || input.user_content_json.is_some()
        || !input.additional_user_parts.is_empty()
        || (!fresh_resend
            && input.expected_configuration_revision != Some(entry.context_configuration_revision))
    {
        return Err(StoreError::Conflict(
            "expert_message_actual_child_differs_from_plan".into(),
        ));
    }
    let config: serde_json::Value = serde_json::from_str(&input.invocation_config_json)?;
    if format!(
        "{:x}",
        Sha256::digest(input.invocation_config_json.as_bytes())
    ) != input.invocation_config_digest
    {
        return Err(StoreError::Invariant(
            "expert_message_actual_model_digest_changed".into(),
        ));
    }
    let model = FrozenResendModel::from_invocation(&config).ok_or_else(|| {
        StoreError::Conflict("expert_message_actual_model_witness_missing".into())
    })?;
    let profile:Option<(String,String,String,String,String,i64,bool,Option<i64>)>=connection.query_row(
        "SELECT provider,model_name,runtime,source_json,parameters_json,revision,enabled,deleted_at_ms FROM model_profiles WHERE id=?1",
        [&entry.model_declaration.profile_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?))).optional()?;
    let Some((provider, name, runtime, source, parameters, revision, enabled, deleted)) = profile
    else {
        return Err(StoreError::Conflict(
            "expert_message_declared_profile_missing".into(),
        ));
    };
    if provider != entry.model_declaration.provider
        || name != entry.model_declaration.model_name
        || runtime != entry.model_declaration.runtime
        || revision != entry.model_declaration.profile_revision
        || !enabled
        || deleted.is_some()
        || serde_json::from_str::<serde_json::Value>(&parameters)?
            != entry.model_declaration.parameters
    {
        return Err(StoreError::Conflict(
            "expert_message_declared_profile_revision_changed".into(),
        ));
    }
    let source: serde_json::Value = serde_json::from_str(&source)?;
    validate_expert_model_route(
        &entry.model_declaration,
        &source,
        &entry.defaults,
        &model.primary,
    )
    .map_err(StoreError::Conflict)?;
    let checkpoint: serde_json::Value = serde_json::from_str(&input.checkpoint_json)?;
    let turn = checkpoint
        .pointer("/state/execution/active_turn")
        .ok_or_else(|| StoreError::Invariant("expert_message_actual_turn_missing".into()))?;
    if turn.get("turn_id").and_then(|v| v.as_str()) != Some(input.run_id.as_str()) {
        return Err(StoreError::Conflict(
            "expert_message_actual_instruction_owner_changed".into(),
        ));
    }
    let context = turn
        .get("context")
        .ok_or_else(|| StoreError::Invariant("expert_message_actual_context_missing".into()))?;
    // Select the closed protocol discriminator in the actual Core-produced checkpoint;
    // never inspect instruction text or deserialize the old executable runtime in Store.
    let components = context
        .get("components")
        .and_then(|v| v.as_array())
        .ok_or_else(|| {
            StoreError::Invariant("expert_message_typed_instruction_facts_missing".into())
        })?;
    let mut definitions = vec![];
    let mut ids = std::collections::BTreeSet::new();
    for component in components
        .iter()
        .filter(|c| c.get("kind").and_then(|v| v.as_str()) == Some("prompt_template"))
    {
        let id = component
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| StoreError::Conflict("expert_message_instruction_id_missing".into()))?
            .to_owned();
        let content = component
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                StoreError::Conflict("expert_message_instruction_definition_must_be_text".into())
            })?
            .to_owned();
        if content.len() > 262_144 || !ids.insert(id.clone()) {
            return Err(StoreError::Conflict(
                "expert_message_invalid_instruction_definition".into(),
            ));
        }
        definitions.push(ExpertPromptDefinition {
            component_id: id,
            content_sha256: format!("{:x}", Sha256::digest(content.as_bytes())),
            content,
            definition_revision: None,
            platform: context.get("platform").cloned().ok_or_else(|| {
                StoreError::Invariant("expert_message_instruction_platform_missing".into())
            })?,
        });
    }
    if let Some(expected) = &entry.expected_prompt_definition {
        let actual = definitions
            .iter_mut()
            .find(|d| d.component_id == expected.component_id)
            .ok_or_else(|| {
                StoreError::Conflict("expert_message_original_template_definition_missing".into())
            })?;
        if actual.content != expected.content || actual.content_sha256 != expected.content_sha256 {
            return Err(StoreError::Conflict(
                "expert_message_original_template_definition_changed".into(),
            ));
        }
        actual.definition_revision = expected.definition_revision;
    }
    if let Some(id) = entry
        .defaults
        .conversation_configuration()
        .get("selectedPromptTemplateID")
        .and_then(|v| v.as_str())
    {
        if !definitions
            .iter()
            .any(|d| d.component_id == format!("prompt_template:{id}"))
        {
            return Err(StoreError::Conflict(
                "expert_message_selected_template_has_no_typed_body".into(),
            ));
        }
    }
    if definitions.len() != expected_instructions.len()
        || expected_instructions.iter().any(|expected| {
            definitions
                .iter()
                .filter(|actual| {
                    actual.component_id == expected.component_id
                        && actual.content == expected.content
                        && actual.content_sha256 == expected.content_sha256
                        && actual.definition_revision == expected.definition_revision
                })
                .count()
                != 1
        })
    {
        return Err(StoreError::Conflict(
            "expert_message_actual_instruction_set_differs_from_capture".into(),
        ));
    }
    let witness = MessageSendWitness {
        conversation_id: input.conversation_id.clone(),
        message_id: input.user_message_id.clone(),
        run_id: input.run_id.clone(),
        assistant_message_id: input.assistant_message_id.clone(),
        body_sha256: format!("{:x}", Sha256::digest(input.user_text.as_bytes())),
        invocation_config_sha256: input.invocation_config_digest.clone(),
        model,
    };
    let instructions = ExpertInstructionWitness {
        checkpoint_sha256: format!("{:x}", Sha256::digest(input.checkpoint_json.as_bytes())),
        component_payload_digest: context
            .get("component_payload_digest")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                StoreError::Invariant("expert_message_instruction_payload_digest_missing".into())
            })?
            .into(),
        prompt_definitions: definitions,
    };
    if entry.model_witness.as_ref().is_some_and(|v| v != &witness)
        || entry
            .instruction_witness
            .as_ref()
            .is_some_and(|v| v != &instructions)
    {
        return Err(StoreError::Conflict(
            "expert_message_child_witness_conflict".into(),
        ));
    }
    // Real Begin admission is the shared execution ownership boundary, including
    // trusted Host adapters which enter through the ordinary prepared factory.
    let receipt_key = format!(
        "agent_room_submission/{}/{}",
        manifest.input.room_id, input.run_id
    );
    let raw: Option<String> = connection
        .query_row(
            "SELECT value_json FROM product_settings WHERE key=?1",
            [&receipt_key],
            |r| r.get(0),
        )
        .optional()?;
    let mut receipt: serde_json::Value =
        serde_json::from_str(&raw.ok_or_else(|| {
            StoreError::Invariant("expert_message_actual_receipt_missing".into())
        })?)?;
    if receipt.pointer("/input/room_id").and_then(|v| v.as_str())
        != Some(manifest.input.room_id.as_str())
        || receipt
            .pointer("/input/submission_id")
            .and_then(|v| v.as_str())
            != Some(input.run_id.as_str())
        || receipt
            .get("selected_participant_id")
            .and_then(|v| v.as_str())
            != Some(entry.participant.participant_id.as_str())
        || receipt
            .pointer("/input/participant_id")
            .and_then(|v| v.as_str())
            != Some(entry.participant.participant_id.as_str())
        || receipt
            .pointer("/input/conversation_id")
            .and_then(|v| v.as_str())
            != Some(manifest.input.conversation_id.as_str())
        || receipt
            .pointer("/input/parent_message_id")
            .and_then(|v| v.as_str())
            != Some(manifest.input.user_message_id.as_str())
        || receipt
            .get("context_conversation_id")
            .and_then(|v| v.as_str())
            != Some(input.conversation_id.as_str())
        || !matches!(
            receipt.get("state").and_then(|v| v.as_str()),
            Some("selected" | "executing")
        )
        || receipt
            .get("execution_id")
            .filter(|v| !v.is_null())
            .is_some_and(|v| v.as_str() != Some(input.run_id.as_str()))
    {
        return Err(StoreError::Conflict(
            "expert_message_actual_receipt_owner_changed".into(),
        ));
    }
    let old_intent: Option<String> = connection
        .query_row(
            "SELECT value_json FROM product_settings WHERE key=?1",
            [format!(
                "agent_room_execution/{}/{}",
                manifest.input.room_id, input.run_id
            )],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(intent) = old_intent {
        let intent: serde_json::Value = serde_json::from_str(&intent)?;
        let envelope = intent.get("envelope").ok_or_else(|| {
            StoreError::Invariant("expert_message_intent_envelope_missing".into())
        })?;
        let exact_native = envelope.get("kind").and_then(|v| v.as_str()) == Some("native")
            && (envelope
                .get("conversation_configuration_revision")
                .and_then(|v| v.as_i64())
                == input.expected_configuration_revision
                && envelope.get("model_profile_id").and_then(|v| v.as_str())
                    == Some(entry.model_declaration.profile_id.as_str())
                && envelope
                    .get("model_profile_revision")
                    .and_then(|v| v.as_i64())
                    == Some(entry.model_declaration.profile_revision)
                && envelope
                    .get("expected_parent_message_id")
                    .and_then(|v| v.as_str())
                    == input.expected_parent_message_id.as_deref()
                && envelope.get("text").and_then(|v| v.as_str()) == Some(input.user_text.as_str()));
        let exact_host = if envelope.get("kind").and_then(|v| v.as_str()) == Some("host") {
            let request = envelope.get("request").ok_or_else(|| {
                StoreError::Invariant("expert_message_host_request_missing".into())
            })?;
            let mut semantic = request.clone();
            let digest = semantic
                .as_object_mut()
                .ok_or_else(|| StoreError::Invariant("expert_message_host_request_invalid".into()))?
                .remove("request_digest")
                .and_then(|v| v.as_str().map(str::to_owned))
                .ok_or_else(|| {
                    StoreError::Invariant("expert_message_host_request_digest_missing".into())
                })?;
            let identity = nola_domain::remote_submission::RemoteSubmissionIdentity {
                source_host_id: request
                    .get("expected_host_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .into(),
                client_id: request
                    .get("client_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .into(),
                conversation_id: input.conversation_id.clone(),
                submission_id: input.run_id.clone(),
                user_message_id: input.user_message_id.clone(),
                assistant_message_id: input.assistant_message_id.clone(),
            };
            identity.validate().map_err(StoreError::Conflict)?;
            let local:Option<(String,String)>=connection.query_row("SELECT receipt_json,request_json FROM source_submission_receipts WHERE submission_id=?1",[&input.run_id],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
            let Some((receipt, original_request)) = local else {
                return Err(StoreError::Conflict(
                    "expert_message_actual_source_request_witness_missing".into(),
                ));
            };
            let receipt: nola_domain::remote_submission::SourceSubmissionReceipt =
                serde_json::from_str(&receipt)?;
            receipt.validate().map_err(StoreError::Conflict)?;
            let original: serde_json::Value = serde_json::from_str(&original_request)?;
            receipt.identity == identity
                && receipt.request_digest == digest
                && receipt.phase == nola_domain::remote_submission::RemoteSubmissionPhase::Prepared
                && original == semantic
                && nola_domain::remote_submission::submission_request_digest(&semantic)
                    .map_err(StoreError::Conflict)?
                    == digest
                && request.get("conversationId").and_then(|v| v.as_str())
                    == Some(input.conversation_id.as_str())
                && request.get("submission_id").and_then(|v| v.as_str())
                    == Some(input.run_id.as_str())
                && request.get("user_message_id").and_then(|v| v.as_str())
                    == Some(input.user_message_id.as_str())
                && request.get("assistant_message_id").and_then(|v| v.as_str())
                    == Some(input.assistant_message_id.as_str())
                && request
                    .get("expected_configuration_revision")
                    .and_then(|v| v.as_i64())
                    == input.expected_configuration_revision
                && request
                    .get("expected_parent_message_id")
                    .and_then(|v| v.as_str())
                    == input.expected_parent_message_id.as_deref()
                && request.get("messages")
                    == Some(&serde_json::json!([{"role":"user","content":input.user_text}]))
                && request
                    .pointer("/model_intent/profile_id")
                    .and_then(|v| v.as_str())
                    == Some(entry.model_declaration.profile_id.as_str())
                && request
                    .pointer("/model_intent/expected_revision")
                    .and_then(|v| v.as_i64())
                    == Some(entry.model_declaration.profile_revision)
                && config.pointer("/turn_identity/remote_tool_plan/reference")
                    == request.get("tool_plan_ref")
        } else {
            false
        };
        if intent.get("execution_id").and_then(|v| v.as_str()) != Some(input.run_id.as_str())
            || !(exact_native || exact_host)
        {
            return Err(StoreError::Conflict(
                "expert_message_actual_intent_owner_changed".into(),
            ));
        }
    }

    if receipt.get("state").and_then(|v| v.as_str()) != Some("executing")
        || receipt.get("execution_id").and_then(|v| v.as_str()) != Some(input.run_id.as_str())
    {
        let revision = receipt
            .get("revision")
            .and_then(|v| v.as_i64())
            .and_then(|v| v.checked_add(1))
            .ok_or_else(|| {
                StoreError::Invariant("expert_message_receipt_revision_invalid".into())
            })?;
        receipt["state"] = serde_json::json!("executing");
        receipt["execution_id"] = serde_json::json!(input.run_id);
        receipt["revision"] = serde_json::json!(revision);
        connection.execute("UPDATE product_settings SET value_json=?2,revision=revision+1,updated_at_ms=?3 WHERE key=?1",params![receipt_key,serde_json::to_string(&receipt)?,input.created_at_ms])?;
    }
    entry.model_witness = Some(witness);
    entry.instruction_witness = Some(instructions);
    connection.execute("UPDATE product_settings SET value_json=?2,revision=revision+1,updated_at_ms=?3 WHERE key=?1",params![source_key,serde_json::to_string(&manifest)?,input.created_at_ms])?;
    Ok(())
}

fn child_instructions(
    connection: &Connection,
    target: &str,
) -> StoreResult<Option<Vec<ExpertPromptDefinition>>> {
    // A fresh child has exactly one complete-parent manifest entry; no scan of successful Runs.
    let settings: Option<String> = connection
        .query_row(
            "SELECT settings_json FROM conversation_settings WHERE conversation_id=?1",
            [target],
            |r| r.get(0),
        )
        .optional()?;
    let Some(settings) = settings else {
        return Ok(None);
    };
    let settings: serde_json::Value = serde_json::from_str(&settings)?;
    let Some(owner) = settings.get(nola_domain::agent_room::CONTEXT_OWNER_FIELD) else {
        return Ok(None);
    };
    let owner: nola_domain::agent_room::ContextOwner = serde_json::from_value(owner.clone())?;
    let parent:Option<String>=connection.query_row("SELECT json_extract(value_json,'$.conversation_id') FROM product_settings WHERE key=?1",[format!("agent_chat_room_conversation/{}",owner.room_id)],|r|r.get(0)).optional()?;
    let Some(parent) = parent else {
        return Ok(None);
    };
    let Some(prepared): Option<PreparedExpertResend> =
        read(connection, &key("prepared", &parent)?)?
    else {
        return Ok(None);
    };
    let mut entries = prepared.entries.iter().filter(|e| {
        e.participant.context_conversation_id == target
            && e.participant.participant_id == owner.participant_id
            && e.participant.agent_id == owner.agent_id
    });
    let entry = entries
        .next()
        .ok_or_else(|| StoreError::Invariant("expert_resend_child_not_in_complete_plan".into()))?;
    if entries.next().is_some() {
        return Err(StoreError::Invariant(
            "expert_resend_duplicate_child_owner".into(),
        ));
    }
    Ok(Some(
        entry
            .original
            .instruction_witness
            .as_ref()
            .ok_or_else(|| {
                StoreError::Invariant("expert_resend_original_instruction_witness_missing".into())
            })?
            .prompt_definitions
            .clone(),
    ))
}

/// Shared consume check for both ordinary and expert edited children. This
/// compares typed instruction data only and never restores old Host context.
pub(crate) fn validate_prepared_instructions(
    connection: &Connection,
    input: &BeginTurnInput,
) -> StoreResult<()> {
    let expected = child_instructions(connection, &input.conversation_id)?.unwrap_or_default();
    let checkpoint: serde_json::Value = serde_json::from_str(&input.checkpoint_json)?;
    let components = checkpoint
        .pointer("/state/execution/active_turn/context/components")
        .and_then(|v| v.as_array())
        .ok_or_else(|| {
            StoreError::Conflict("edited_resend_fresh_instruction_facts_required".into())
        })?;
    let digest = nola_domain::edited_resend_context::validate_components(
        &expected,
        &serde_json::Value::Array(components.clone()),
    )
    .map_err(StoreError::Conflict)?;
    let config: serde_json::Value = serde_json::from_str(&input.invocation_config_json)?;
    if config
        .pointer("/turn_identity/fresh_resend_policy/context_components_sha256")
        .and_then(|v| v.as_str())
        != Some(digest.as_str())
    {
        return Err(StoreError::Conflict(
            "edited_resend_actual_context_differs_from_host_attestation".into(),
        ));
    }
    let prepared: nola_domain::edited_resend::PreparedEditedResend = read(
        connection,
        &super::edited_resend::key("prepared", &input.conversation_id)?,
    )?
    .ok_or_else(|| StoreError::Conflict("edited_resend_preparation_missing".into()))?;
    nola_domain::edited_resend_context::validate_items(
        &prepared,
        checkpoint
            .pointer("/state/conversation/transcript")
            .ok_or_else(|| StoreError::Conflict("edited_resend_transcript_missing".into()))?,
        &serde_json::json!([]),
    )
    .map_err(StoreError::Conflict)?;
    // Materialized context.items is derived from components. Raw context input
    // is verified through Core's existing accepted-input payload digest.
    let admissions = checkpoint
        .pointer("/state/execution/input_queue/admissions")
        .and_then(|v| v.as_object())
        .ok_or_else(|| StoreError::Conflict("edited_resend_input_admission_missing".into()))?;
    let admission = admissions
        .values()
        .next()
        .filter(|_| admissions.len() == 1)
        .ok_or_else(|| StoreError::Conflict("edited_resend_input_admission_not_unique".into()))?;
    if admission.get("submission_id").and_then(|v| v.as_str()) != Some(prepared.run_id.as_str())
        || admission.get("origin").and_then(|v| v.as_str()) != Some("interactive_user")
        || admission.get("delivery").and_then(|v| v.as_str()) != Some("current_turn")
        || admission.get("payload_digest")
            != config.pointer("/turn_identity/fresh_resend_policy/input_payload_sha256")
    {
        return Err(StoreError::Conflict(
            "edited_resend_actual_submission_differs_from_host_attestation".into(),
        ));
    }
    Ok(())
}
