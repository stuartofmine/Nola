// SPDX-License-Identifier: Apache-2.0
//! Private typed resend facts use the existing product_settings table, with no
//! schema conversion, generic setting writes, or replica authority.
use super::*;
use nola_domain::edited_resend::*;
use nola_store_port::edited_resend::{
    EditedResendHostMaterial, EditedResendRepository, ResendModelCredentialReference,
};
use nola_store_port::run_creation::BeginTurnInput;
use serde::{de::DeserializeOwned, Serialize};

pub(crate) fn key(kind: &str, identity: impl Serialize) -> StoreResult<String> {
    Ok(format!(
        "{EDITED_RESEND_PREFIX}{kind}/{:x}",
        Sha256::digest(serde_json::to_vec(&identity)?)
    ))
}
pub(crate) fn read<T: DeserializeOwned>(connection: &Connection, key: &str) -> StoreResult<Option<T>> {
    let raw: Option<String> = connection
        .query_row(
            "SELECT value_json FROM product_settings WHERE key=?1",
            [key],
            |row| row.get(0),
        )
        .optional()?;
    raw.map(|raw| serde_json::from_str(&raw).map_err(StoreError::from))
        .transpose()
}
pub(crate) fn write_immutable(
    connection: &Connection,
    key: &str,
    value: &impl Serialize,
    at: i64,
) -> StoreResult<()> {
    let raw = serde_json::to_string(value)?;
    let previous: Option<String> = connection
        .query_row(
            "SELECT value_json FROM product_settings WHERE key=?1",
            [key],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(previous) = previous {
        if previous != raw {
            return Err(StoreError::Conflict(
                "edited_resend_immutable_record_conflict".into(),
            ));
        }
        return Ok(());
    }
    connection.execute(
        "INSERT INTO product_settings(key,value_json,revision,updated_at_ms) VALUES(?1,?2,1,?3)",
        params![key, raw, at],
    )?;
    // Like local Run invocation payloads, these are source-local execution
    // witnesses. Generic product-setting sync must not manufacture them.
    Ok(())
}

/// Runs inside begin_turn's existing transaction. Missing proof disables this
/// optional product capability; it does not rewrite old sends or their data.
pub(crate) fn record_plain_send(
    connection: &Connection,
    input: &BeginTurnInput,
) -> StoreResult<()> {
    if input.user_part_kind != "text"
        || input.user_content_json.is_some()
        || !input.additional_user_parts.is_empty()
    {
        return Ok(());
    }
    if format!(
        "{:x}",
        Sha256::digest(input.invocation_config_json.as_bytes())
    ) != input.invocation_config_digest
    {
        return Ok(());
    }
    let config: serde_json::Value = serde_json::from_str(&input.invocation_config_json)?;
    let Some(model) = FrozenResendModel::from_invocation(&config) else {
        return Ok(());
    };
    let source = MessageSendWitness {
        conversation_id: input.conversation_id.clone(),
        message_id: input.user_message_id.clone(),
        run_id: input.run_id.clone(),
        assistant_message_id: input.assistant_message_id.clone(),
        body_sha256: format!("{:x}", Sha256::digest(input.user_text.as_bytes())),
        invocation_config_sha256: input.invocation_config_digest.clone(),
        model,
    };
    write_immutable(
        connection,
        &key("source", (&source.conversation_id, &source.message_id))?,
        &source,
        input.created_at_ms,
    )?;
    let reference: Option<ResendModelCredentialReference> = config
        .pointer("/turn_identity/host_recovery")
        .filter(|value| !value.is_null())
        .map(|value| serde_json::from_value(value.clone()))
        .transpose()?;
    write_immutable(
        connection,
        &key(
            "model_credential",
            (&source.conversation_id, &source.message_id),
        )?,
        &reference,
        input.created_at_ms,
    )
}

/// A caller cannot bypass the prepared model or turn it into an old-Run replay
/// by invoking ordinary BeginAssistantTurn with today's selected configuration.
pub(crate) fn validate_prepared_admission(
    connection: &Connection,
    input: &BeginTurnInput,
) -> StoreResult<()> {
    let Some(prepared): Option<PreparedEditedResend> =
        read(connection, &key("prepared", &input.conversation_id)?)?
    else {
        return Ok(());
    };
    if prepared.admitted_run_id.is_some() {
        return Ok(());
    }
    let config: serde_json::Value = serde_json::from_str(&input.invocation_config_json)?;
    let fresh = config
        .get("initial_items")
        .and_then(|v| v.as_array())
        .is_some_and(|v| v.is_empty())
        && config.get("session_id").and_then(|v| v.as_str())
            == Some(input.conversation_id.as_str())
        && config
            .get("automation_execution_spec")
            .is_none_or(|v| v.is_null())
        && config
            .pointer("/turn_identity/remote_tool_plan")
            .is_none_or(|plan| {
                plan.is_null()
                    || plan
                        .pointer("/reference/session_id")
                        .and_then(|id| id.as_str())
                        == Some(input.conversation_id.as_str())
            });
    let policy: FreshResendPolicyWitness = serde_json::from_value(
        config
            .pointer("/turn_identity/fresh_resend_policy")
            .cloned()
            .ok_or_else(|| {
                StoreError::Conflict("edited_resend_fresh_policy_witness_required".into())
            })?,
    )?;
    let catalog = config
        .get("host_catalog")
        .ok_or_else(|| StoreError::Conflict("edited_resend_fresh_catalog_required".into()))?;
    let rules = config
        .get("permission_rules")
        .ok_or_else(|| StoreError::Conflict("edited_resend_fresh_rules_required".into()))?;
    let scoped_rules = rules
        .get("rules")
        .and_then(|v| v.as_array())
        .ok_or_else(|| StoreError::Conflict("edited_resend_invalid_fresh_rules".into()))?;
    let explicit_old_grants = scoped_rules.iter().any(|rule| {
        matches!(
            rule.get("source").and_then(|v| v.as_str()),
            Some("conversation" | "turn")
        )
    });
    let digest = |value: &serde_json::Value| {
        canonical_resend_value_sha256(value).map_err(StoreError::Invariant)
    };
    let policy_matches = policy.operation_id == prepared.input.operation_id
        && policy.conversation_id == input.conversation_id
        && policy.excluded_source_conversation_id == prepared.input.source_conversation_id
        && catalog.get("host_id").and_then(|v| v.as_str()) == Some(policy.host_id.as_str())
        && catalog.get("revision").and_then(|v| v.as_u64()) == Some(policy.catalog_revision)
        && digest(catalog)? == policy.catalog_sha256
        && digest(rules)? == policy.permission_rules_sha256
        && config
            .pointer("/turn_identity/host_invocation/execution_policy_sha256")
            .and_then(|v| v.as_str())
            == Some(policy.execution_policy_sha256.as_str());
    if prepared.input.target_conversation_id != input.conversation_id
        || prepared.run_id != input.run_id
        || prepared.user_message_id != input.user_message_id
        || prepared.assistant_message_id != input.assistant_message_id
        || prepared.input.edited_text != input.user_text
        || prepared
            .admitted_run_id
            .as_ref()
            .is_some_and(|id| id != &input.run_id)
        || input.expected_parent_message_id.is_some()
        || input.context_checkpoint_id.is_some()
        || input.user_part_kind != "text"
        || input.user_content_json.is_some()
        || !input.additional_user_parts.is_empty()
        || format!(
            "{:x}",
            Sha256::digest(input.invocation_config_json.as_bytes())
        ) != input.invocation_config_digest
        || FrozenResendModel::from_invocation(&config).as_ref() != Some(&prepared.source.model)
        || !fresh
        || !policy_matches
        || explicit_old_grants
    {
        return Err(StoreError::Conflict(
            "edited_resend_admission_differs_from_prepared_intent".into(),
        ));
    }
    crate::expert_message::validate_prepared_instructions(connection,input)?;
    let previous: i64 = connection.query_row(
        "SELECT COUNT(*) FROM messages WHERE conversation_id=?1",
        [&input.conversation_id],
        |row| row.get(0),
    )?;
    if previous != 0 {
        return Err(StoreError::Conflict(
            "edited_resend_target_is_not_empty".into(),
        ));
    }
    Ok(())
}

pub(crate) fn mark_prepared_admitted(
    connection: &Connection,
    input: &BeginTurnInput,
) -> StoreResult<()> {
    let prepared_key = key("prepared", &input.conversation_id)?;
    let Some(mut value): Option<PreparedEditedResend> = read(connection, &prepared_key)? else {
        return Ok(());
    };
    if value.admitted_run_id.is_some() {
        return Ok(());
    }
    if value.run_id != input.run_id {
        return Err(StoreError::Conflict(
            "edited_resend_admission_owner_mismatch".into(),
        ));
    }
    value.admitted_run_id = Some(input.run_id.clone());
    connection.execute("UPDATE product_settings SET value_json=?2,revision=revision+1,updated_at_ms=?3 WHERE key=?1",params![prepared_key,serde_json::to_string(&value)?,input.created_at_ms])?;
    Ok(())
}

impl EditedResendRepository for NolaStore {
    type Error = StoreError;
    fn message_send_witness(
        &self,
        conversation: &str,
        message: &str,
    ) -> StoreResult<Option<MessageSendWitness>> {
        // An associated Room can contain additional original recipients. Until
        // admission owns a complete expert-set witness, never return a main
        // model-only snapshot as if it proved the entire original send.
        let room_association: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM product_settings WHERE key=?1)",
            [format!("agent_chat_conversation_room/{conversation}")],
            |row| row.get(0),
        )?;
        if room_association {
            return Err(StoreError::Conflict(
                "edited_resend_expert_snapshot_unavailable".into(),
            ));
        }
        let result: Option<MessageSendWitness> =
            read(&self.connection, &key("source", (conversation, message))?)?;
        if result.as_ref().is_some_and(|value| {
            value.conversation_id != conversation || value.message_id != message
        }) {
            return Err(StoreError::Invariant(
                "edited_resend_source_identity_mismatch".into(),
            ));
        }
        Ok(result)
    }
    fn prepared_edited_resend_by_operation(
        &self,
        operation: &str,
    ) -> StoreResult<Option<PreparedEditedResend>> {
        let Some(target): Option<String> = read(&self.connection, &key("operation", operation)?)?
        else {
            return Ok(None);
        };
        let result = self.prepared_edited_resend(&target)?.ok_or_else(|| {
            StoreError::Invariant("edited_resend_operation_target_missing".into())
        })?;
        if result.input.operation_id != operation {
            return Err(StoreError::Invariant(
                "edited_resend_operation_identity_mismatch".into(),
            ));
        }
        Ok(Some(result))
    }
    fn prepared_edited_resend(
        &self,
        conversation: &str,
    ) -> StoreResult<Option<PreparedEditedResend>> {
        let result: Option<PreparedEditedResend> =
            read(&self.connection, &key("prepared", conversation)?)?;
        if result
            .as_ref()
            .is_some_and(|value| value.input.target_conversation_id != conversation)
        {
            return Err(StoreError::Invariant(
                "edited_resend_target_identity_mismatch".into(),
            ));
        }
        Ok(result)
    }
    fn edited_resend_host_material(
        &self,
        target: &str,
    ) -> StoreResult<Option<EditedResendHostMaterial>> {
        let _snapshot = crate::read_snapshot::pin(&self.connection)?;
        let Some(prepared) = self.prepared_edited_resend(target)? else {
            return Ok(None);
        };
        let source = &prepared.source;
        let owned:bool=self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM runs r JOIN messages u ON u.id=r.user_message_id AND u.conversation_id=r.conversation_id AND u.run_id=r.id AND u.role='user' JOIN messages a ON a.id=r.assistant_message_id AND a.conversation_id=r.conversation_id AND a.run_id=r.id AND a.role='assistant' WHERE r.id=?1 AND r.conversation_id=?2 AND u.id=?3 AND a.id=?4 AND NOT EXISTS(SELECT 1 FROM remote_run_sources s WHERE s.run_id=r.id))",
            params![source.run_id,source.conversation_id,source.message_id,source.assistant_message_id],|row|row.get(0))?;
        if !owned {
            return Err(StoreError::Conflict(
                "edited_resend_private_source_owner_changed".into(),
            ));
        }
        let raw: String = self.connection.query_row(
            "SELECT config_json FROM run_invocation_payloads WHERE run_id=?1",
            [&source.run_id],
            |row| row.get(0),
        )?;
        if format!("{:x}", Sha256::digest(raw.as_bytes())) != source.invocation_config_sha256 {
            return Err(StoreError::Invariant(
                "edited_resend_private_source_digest_changed".into(),
            ));
        }
        let config: serde_json::Value = serde_json::from_str(&raw)?;
        if FrozenResendModel::from_invocation(&config).as_ref() != Some(&source.model) {
            return Err(StoreError::Invariant(
                "edited_resend_private_source_model_changed".into(),
            ));
        }
        let stored: Option<Option<ResendModelCredentialReference>> = read(
            &self.connection,
            &key(
                "model_credential",
                (&source.conversation_id, &source.message_id),
            )?,
        )?;
        let reference = stored.ok_or_else(|| {
            StoreError::Conflict("edited_resend_credential_witness_unavailable".into())
        })?;
        let actual: Option<ResendModelCredentialReference> = config
            .pointer("/turn_identity/host_recovery")
            .filter(|value| !value.is_null())
            .map(|value| serde_json::from_value(value.clone()))
            .transpose()?;
        if actual != reference {
            return Err(StoreError::Invariant(
                "edited_resend_credential_source_changed".into(),
            ));
        }
        let expert_child = nola_store_port::expert_message::ExpertMessageRepository::expert_message_for_child(self, target, &prepared.run_id)?.is_some();
        Ok(Some(EditedResendHostMaterial {
            prepared,
            model_credential_reference: reference,
            expert_child,
        }))
    }
    fn put_prepared_edited_resend(&mut self, value: &PreparedEditedResend) -> StoreResult<()> {
        if !self.has_write_unit_of_work() || value.admitted_run_id.is_some() {
            return Err(StoreError::Invariant(
                "edited_resend_preparation_requires_transaction".into(),
            ));
        }
        write_immutable(
            &self.connection,
            &key("prepared", &value.input.target_conversation_id)?,
            value,
            value.input.created_at_ms,
        )?;
        write_immutable(
            &self.connection,
            &key("operation", &value.input.operation_id)?,
            &value.input.target_conversation_id,
            value.input.created_at_ms,
        )
    }
}
