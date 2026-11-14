// SPDX-License-Identifier: Apache-2.0
//! Consume a private peer plan at the real Begin transaction, without a user manifest.
use super::*;
pub(super) fn record_peer_send(connection: &Connection, input: &BeginTurnInput) -> StoreResult<()> {
    let key = format!("expert_peer_private/v1/exchange/{}", input.run_id);
    let raw: Option<String> = connection
        .query_row(
            "SELECT value_json FROM product_settings WHERE key=?1",
            [&key],
            |r| r.get(0),
        )
        .optional()?;
    let Some(raw) = raw else { return Ok(()) };
    if raw.len() > EXPERT_TEMPLATE_CAPTURE_MAX_BYTES {
        return Err(StoreError::Invariant(
            "expert_peer_capture_exceeds_budget".into(),
        ));
    }
    let mut record: serde_json::Value = serde_json::from_str(&raw)?;
    let view = &record["material"]["view"];
    let mut entry: ExpertMessageEntry =
        serde_json::from_value(record["material"]["entry"].clone())?;
    let definitions: Vec<ExpertPromptDefinition> =
        serde_json::from_value(record["material"]["prompt_definitions"].clone())?;
    let room = view["room_id"]
        .as_str()
        .ok_or_else(|| StoreError::Invariant("expert_peer_room_missing".into()))?
        .to_owned();
    if view["version"] != 1
        || view["exchange_id"] != input.run_id
        || entry.child_submission_id != input.run_id
        || entry.participant.context_conversation_id != input.conversation_id
        || view["target"]["participant_id"] != entry.participant.participant_id
        || view["target"]["context_conversation_id"] != input.conversation_id
        || entry.child_user_message_id != format!("{}:user", input.run_id)
        || entry.child_assistant_message_id != format!("{}:assistant", input.run_id)
    {
        return Err(StoreError::Conflict(
            "expert_peer_actual_owner_changed".into(),
        ));
    }
    let text = view["text"]
        .as_str()
        .ok_or_else(|| StoreError::Invariant("expert_peer_text_missing".into()))?;
    let (witness, instructions, config) =
        validate_consumed_entry(connection, input, &entry, &definitions, text, false)?;
    bind_consumed_receipt(connection, input, &entry, &room, None, &config)?;
    entry.model_witness = Some(witness);
    entry.instruction_witness = Some(instructions);
    record["material"]["entry"] = serde_json::to_value(entry)?;
    connection.execute("UPDATE product_settings SET value_json=?2,revision=revision+1,updated_at_ms=?3 WHERE key=?1",
        params![key,serde_json::to_string(&record)?,input.created_at_ms])?;
    Ok(())
}
