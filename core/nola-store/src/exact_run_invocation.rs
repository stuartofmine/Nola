// SPDX-License-Identifier: Apache-2.0
//! Exact frozen configuration reads; no current model or credential lookup.
use super::*;
use nola_store_port::run_recovery_inspection::{
    StoredExactRunCoreCheckpoint, StoredExactRunInvocation, StoredInitialCoreAdmission,
};

pub(super) fn load(
    store: &NolaStore,
    conversation_id: &str,
    run_id: &str,
    assistant_message_id: &str,
) -> StoreResult<Option<StoredExactRunInvocation>> {
    for (name, value) in [
        ("conversation_id", conversation_id),
        ("run_id", run_id),
        ("assistant_message_id", assistant_message_id),
    ] {
        require_id(name, value)?;
    }
    let _snapshot = read_snapshot::pin(&store.connection)?;
    let record = store
        .connection
        .query_row(
            "SELECT r.conversation_id, r.assistant_message_id, r.kind, r.status,
                r.completed_at_ms, p.config_json, r.invocation_config_digest,
                EXISTS(SELECT 1 FROM messages m WHERE m.id=r.assistant_message_id
                  AND m.conversation_id=r.conversation_id AND m.run_id=r.id AND m.role='assistant'),
                EXISTS(SELECT 1 FROM remote_run_sources s WHERE s.run_id=r.id),
                c.conversation_id,
                (SELECT b.turn_id FROM assistant_turn_message_bindings b
                 WHERE b.run_id=r.id AND b.conversation_id=r.conversation_id
                   AND b.message_id=r.assistant_message_id AND b.admission=1),
                (SELECT CASE WHEN json_valid(e.payload_json) THEN CASE WHEN json_type(e.payload_json,'$.turn_id')='text'
                   THEN json_extract(e.payload_json,'$.turn_id') ELSE '' END ELSE '' END
                 FROM run_events e WHERE e.run_id=r.id AND e.event_kind='turn_started'
                 ORDER BY e.sequence DESC LIMIT 1),
                EXISTS(SELECT 1 FROM committed_transitions t WHERE t.run_id=r.id AND t.checkpoint_revision=c.revision),
                (SELECT CASE WHEN json_valid(e.payload_json) THEN CASE WHEN json_type(e.payload_json,'$.session_id')='text'
                   THEN json_extract(e.payload_json,'$.session_id') ELSE '' END ELSE '' END
                 FROM run_events e WHERE e.run_id=r.id AND e.event_kind='turn_started'
                 ORDER BY e.sequence DESC LIMIT 1),
                (r.completed_at_ms IS NOT NULL
                 OR EXISTS(SELECT 1 FROM assistant_turn_message_bindings b WHERE b.run_id=r.id)
                 OR EXISTS(SELECT 1 FROM committed_transitions t WHERE t.run_id=r.id)
                 OR EXISTS(SELECT 1 FROM effect_outbox o WHERE o.run_id=r.id))
         FROM runs r LEFT JOIN run_invocation_payloads p ON p.run_id=r.id
         LEFT JOIN core_checkpoints c ON c.conversation_id=r.conversation_id WHERE r.id=?1",
            params![run_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, bool>(7)?,
                    row.get::<_, bool>(8)?,
                    match row.get::<_, Option<String>>(9)? {
                        Some(_) => Some((row.get::<_, Option<String>>(10)?, row.get::<_, Option<String>>(11)?,
                            row.get::<_, bool>(12)?, row.get::<_, Option<String>>(13)?)),
                        None => None,
                    },
                    row.get::<_, bool>(14)?,
                ))
            },
        )
        .optional()?;
    let Some((
        stored_conversation,
        stored_assistant,
        kind,
        status,
        completed_at_ms,
        config,
        digest,
        owner_valid,
        remote,
        core_checkpoint,
        checkpoint_expected,
    )) = record
    else {
        return Ok(None);
    };
    if stored_conversation != conversation_id
        || stored_assistant.as_deref() != Some(assistant_message_id)
        || kind != "conversation"
        || !owner_valid
        || remote
    {
        return Err(StoreError::Conflict(
            "exact local Run invocation identity or ownership mismatch".into(),
        ));
    }
    let (Some(invocation_config_json), Some(invocation_config_digest)) = (config, digest) else {
        return Err(StoreError::Conflict(
            "exact Run has no frozen invocation configuration".into(),
        ));
    };
    if invocation_config_json.trim().is_empty() || invocation_config_digest.trim().is_empty() {
        return Err(StoreError::Conflict(
            "exact Run has an empty frozen invocation configuration".into(),
        ));
    }
    if checkpoint_expected && core_checkpoint.is_none() {
        return Err(StoreError::Conflict(
            "exact Run lost its committed Core checkpoint".into(),
        ));
    }
    let terminal = store.canonical_run_terminal(conversation_id, run_id)?;
    if (status == "sealed") != terminal.is_some()
        || completed_at_ms != terminal.as_ref().map(|terminal| terminal.completed_at_ms)
    {
        return Err(StoreError::Invariant("exact Run terminal header and result seal differ".into()));
    }
    let core_checkpoint = core_checkpoint
        .map(
            |(
                admission_turn_id,
                latest_started_turn_id,
                committed_by_run,
                latest_started_session_id,
            )| {
                let recovery = store
                    .load_local_session_recovery(conversation_id)?
                    .ok_or_else(|| {
                        StoreError::Conflict("exact Run checkpoint disappeared".into())
                    })?;
                if recovery
                    .recovery
                    .replay_base
                    .as_ref()
                    .is_some_and(|base| base.run_id != run_id)
                {
                    return Err(StoreError::Conflict(
                        "exact Run checkpoint replay owner mismatch".into(),
                    ));
                }
                Ok(StoredExactRunCoreCheckpoint {
                    recovered_checkpoint: nola_store_port::run_effect::RunEffectRepository::cached_read_checkpoint(
                        store, conversation_id,
                    )?,
                    recovery,
                    admission_turn_id,
                    latest_started_turn_id,
                    committed_by_run,
                    latest_started_session_id,
                    initial_admission: initial_admission(&store.connection, run_id)?,
                })
            },
        )
        .transpose()?;
    Ok(Some(StoredExactRunInvocation {
        conversation_id: stored_conversation,
        run_id: run_id.into(),
        assistant_message_id: assistant_message_id.into(),
        status,
        completed_at_ms,
        terminal,
        invocation_config_json,
        invocation_config_digest,
        core_checkpoint,
    }))
}

fn initial_admission(
    connection: &Connection,
    run_id: &str,
) -> StoreResult<Option<StoredInitialCoreAdmission>> {
    // Bound the read before materializing JSON. An existing malformed first
    // fact is an error, not permission to skip forward to a later valid event.
    let raw: Option<String> = connection.query_row(
        "SELECT CASE WHEN length(CAST(payload_json AS BLOB))<=16384 AND json_valid(payload_json)
             THEN payload_json ELSE '' END
         FROM run_events WHERE run_id=?1 AND event_kind='turn_started'
         ORDER BY sequence ASC LIMIT 1",
        [run_id], |row| row.get(0)).optional()?;
    let Some(raw) = raw else { return Ok(None) };
    #[derive(serde::Deserialize)]
    struct Cursor {
        sequence: u64,
    }
    #[derive(serde::Deserialize)]
    struct Envelope {
        session_id: String,
        turn_id: String,
        revision: u64,
        cursor: Cursor,
    }
    #[derive(serde::Deserialize)]
    struct Admission {
        #[serde(rename = "type")]
        kind: String,
        session_id: String,
        turn_id: String,
        core_envelope: Envelope,
    }
    let fact: Admission = serde_json::from_str(&raw).map_err(|_| {
        StoreError::Invariant("exact Run initial admission fact is malformed".into())
    })?;
    if fact.kind != "turn_started"
        || fact.turn_id != run_id
        || fact.core_envelope.turn_id != fact.turn_id
        || fact.core_envelope.session_id != fact.session_id
        || fact.session_id.trim().is_empty()
        || fact.core_envelope.cursor.sequence == 0
    {
        return Err(StoreError::Invariant(
            "exact Run initial admission identity mismatch".into(),
        ));
    }
    Ok(Some(StoredInitialCoreAdmission {
        session_id: fact.session_id,
        turn_id: fact.turn_id,
        revision: fact.core_envelope.revision,
        event_cursor: fact.core_envelope.cursor.sequence,
    }))
}

#[cfg(test)]
#[path = "exact_run_invocation_tests.rs"]
mod tests;
