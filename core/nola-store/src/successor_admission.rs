// SPDX-License-Identifier: Apache-2.0
//! Exact, replayable promotion of a committed NextTurn admission.
use super::*;
use serde_json::Value;
use sha2::{Digest, Sha256};

fn invalid(message: &str) -> StoreError {
    StoreError::Conflict(message.into())
}
fn digest(input: &BeginSuccessorTurnInput) -> StoreResult<String> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(input)?)))
}
fn session(input: &BeginSuccessorTurnInput) -> StoreResult<String> {
    let checkpoint: Value = serde_json::from_str(&input.turn.checkpoint_json)?;
    let id = checkpoint["session_id"]
        .as_str()
        .filter(|id| !id.is_empty())
        .ok_or_else(|| invalid("successor has no Core session"))?;
    if checkpoint["state"]["session_id"].as_str() != Some(id) {
        return Err(invalid("successor Core session differs from state"));
    }
    Ok(id.into())
}

pub(super) fn validate(
    c: &Connection,
    input: &BeginSuccessorTurnInput,
) -> StoreResult<Option<BeginTurnResult>> {
    for (name, id) in [
        ("successor.previous_run", &input.previous_run_id),
        ("successor.admission", &input.admission_id),
        ("successor.submission", &input.originating_submission_id),
    ] {
        require_id(name, id)?;
    }
    let turn = &input.turn;
    let core_session = session(input)?;
    let prior: Option<(String,String)> = c.query_row(
        "SELECT request_digest,result_json FROM run_successor_admissions WHERE core_session_id=?1 AND admission_id=?2",
        params![core_session,input.admission_id], |row| Ok((row.get(0)?,row.get(1)?))).optional()?;
    if let Some((request_digest, result)) = prior {
        if request_digest != digest(input)? {
            return Err(invalid("successor admission replay changed its request"));
        }
        return Ok(Some(serde_json::from_str(&result)?));
    }
    let old: Option<(i64,String)> = c.query_row(
        "SELECT h.revision,b.checkpoint_json FROM core_checkpoints h JOIN core_checkpoint_bodies b USING(conversation_id) WHERE h.conversation_id=?1",
        [&turn.conversation_id], |row| Ok((row.get(0)?,row.get(1)?))).optional()?;
    let Some((revision, raw)) = old else {
        return Err(invalid("successor has no predecessor checkpoint"));
    };
    let checkpoint: Value = serde_json::from_str(&raw)?;
    let old_turn = &checkpoint["state"]["execution"]["active_turn"];
    if revision != input.expected_checkpoint_revision
        || turn.checkpoint_revision <= revision
        || checkpoint["session_id"].as_str() != Some(&core_session)
        || !matches!(
            old_turn["status"].as_str(),
            Some("completed" | "cancelled" | "failed")
        )
    {
        return Err(invalid(
            "successor checkpoint CAS or predecessor identity changed",
        ));
    }
    let old_core_turn = old_turn["turn_id"]
        .as_str()
        .ok_or_else(|| invalid("successor predecessor has no Core turn"))?;
    let owned:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM assistant_turn_message_bindings WHERE conversation_id=?1 AND run_id=?2 AND turn_id=?3)",
        params![turn.conversation_id,input.previous_run_id,old_core_turn],|row|row.get(0))?;
    if !owned {
        return Err(invalid(
            "successor predecessor Core turn belongs to another Run",
        ));
    }
    let settled: bool = c.query_row("SELECT EXISTS(SELECT 1 FROM runs r
        JOIN assistant_run_terminal_seals s ON s.run_id=r.id AND s.conversation_id=r.conversation_id
        WHERE r.id=?1 AND r.conversation_id=?2 AND r.kind='conversation'
        AND r.status IN ('sealed') AND r.completed_at_ms IS NOT NULL
        AND NOT EXISTS(SELECT 1 FROM remote_run_sources remote WHERE remote.run_id=r.id)
        AND NOT EXISTS(SELECT 1 FROM effect_outbox e WHERE e.run_id=r.id AND e.status<>'acknowledged'))
        AND NOT EXISTS(SELECT 1 FROM runs live WHERE live.conversation_id=?2 AND live.status NOT IN ('sealed'))",
        params![input.previous_run_id,turn.conversation_id], |row| row.get(0))?;
    if !settled {
        return Err(invalid(
            "successor requires sealed predecessor and settled effects without another active Run",
        ));
    }
    let candidate: Option<(String,Option<String>)> = c.query_row("SELECT a.admission_id,a.submission_id
        FROM input_admissions a WHERE a.core_session_id=?1 AND a.conversation_id=?2 AND a.delivery='next_turn'
        AND NOT EXISTS(SELECT 1 FROM input_turn_applications p WHERE p.core_session_id=a.core_session_id AND p.admission_id=a.admission_id)
        AND NOT EXISTS(SELECT 1 FROM input_dispositions d WHERE d.core_session_id=a.core_session_id AND d.admission_id=a.admission_id)
        ORDER BY a.accepted_sequence LIMIT 1", params![core_session,turn.conversation_id],
        |row| Ok((row.get(0)?,row.get(1)?))).optional()?;
    if candidate
        != Some((
            input.admission_id.clone(),
            Some(input.originating_submission_id.clone()),
        ))
    {
        return Err(invalid(
            "successor is not the exact earliest unapplied NextTurn admission",
        ));
    }
    let source_terminal: bool = c.query_row("SELECT EXISTS(SELECT 1 FROM input_admissions a JOIN runs r ON r.id=a.source_run_id
        JOIN assistant_run_terminal_seals s ON s.run_id=r.id WHERE a.core_session_id=?1 AND a.admission_id=?2
        AND r.conversation_id=?3 AND r.status IN ('sealed'))",
        params![core_session,input.admission_id,turn.conversation_id], |row| row.get(0))?;
    if !source_terminal {
        return Err(invalid("successor source admission Run is not sealed"));
    }
    let queued = checkpoint["state"]["execution"]["input_queue"]["next_turn"]
        .as_array()
        .and_then(|items| items.first())
        .ok_or_else(|| invalid("successor is absent from durable Core queue"))?;
    if queued["admission"]["admission_id"].as_str() != Some(&input.admission_id)
        || queued["admission"]["submission_id"].as_str() != Some(&input.originating_submission_id)
    {
        return Err(invalid("successor queue admission identity differs"));
    }
    let applications: Vec<Value> = turn
        .initial_events
        .iter()
        .filter(|event| event.event_kind == "input_applied")
        .map(|event| serde_json::from_str(&event.payload_json))
        .collect::<Result<_, _>>()?;
    if applications.len() != 1
        || applications[0]["applications"].as_array().map(Vec::len) != Some(1)
        || applications[0]["applications"][0]["admission_id"].as_str() != Some(&input.admission_id)
        || applications[0]["applications"][0]["destination_turn_id"].as_str() != Some(&turn.run_id)
        || turn
            .initial_events
            .iter()
            .any(|event| event.event_kind == "input_accepted")
    {
        return Err(invalid(
            "successor must apply exactly its original accepted batch without reacceptance",
        ));
    }
    validate_user_items(input, &queued["items"])?;
    Ok(None)
}

fn validate_user_items(input: &BeginSuccessorTurnInput, items: &Value) -> StoreResult<()> {
    let items = items
        .as_array()
        .ok_or_else(|| invalid("successor accepted items are missing"))?;
    let users: Vec<_> = items
        .iter()
        .filter(|item| item["type"] == "message" && item["role"] == "user")
        .collect();
    if users.is_empty() || users.len() != input.ensured_user_messages.len() {
        return Err(invalid(
            "successor user declarations do not cover accepted input",
        ));
    }
    for (user, message) in users.iter().zip(&input.ensured_user_messages) {
        if user["id"].as_str() != Some(&message.id)
            || message.role != "user"
            || message.status != "completed"
            || message.run_id.as_deref() != Some(&input.turn.run_id)
        {
            return Err(invalid("successor user declaration identity differs"));
        }
        let body = message
            .initial_parts
            .first()
            .ok_or_else(|| invalid("successor user body is missing"))?;
        if body.kind != "text"
            || body.state != "completed"
            || body.ordinal != 1
            || body.content_json.is_some()
            || user["content"].as_str() != Some(&body.content_text)
        {
            return Err(invalid("successor user body differs from accepted input"));
        }
        let attachments: Vec<_> = items
            .iter()
            .filter(|item| {
                item["type"] == "attachment" && item["message_id"].as_str() == Some(&message.id)
            })
            .collect();
        if message.initial_parts.len() != 1 + attachments.len() {
            return Err(invalid("successor attachment count differs"));
        }
        for (attachment, part) in attachments.iter().zip(message.initial_parts.iter().skip(1)) {
            let body: Value = serde_json::from_str(
                part.content_json
                    .as_deref()
                    .ok_or_else(|| invalid("successor attachment metadata missing"))?,
            )?;
            if part.kind != "attachment"
                || attachment["id"].as_str() != Some(&part.id)
                || part.state != "completed"
                || attachment["name"].as_str() != Some(&part.content_text)
                || body["kind"] != "artifact"
                || body["id"].as_str() != Some(&part.id)
                || body["message_id"].as_str() != Some(&message.id)
                || body["turn_id"].as_str() != Some(&input.turn.run_id)
                || body["artifact"]["uri"] != attachment["uri"]
                || body["artifact"]["name"] != attachment["name"]
                || body["artifact"]["mime_type"] != attachment["mime_type"]
            {
                return Err(invalid("successor attachment differs from accepted input"));
            }
        }
    }
    let first = &input.ensured_user_messages[0];
    if first.id != input.turn.user_message_id
        || first.initial_parts[0].id != input.turn.user_part_id
        || first.initial_parts[0].content_text != input.turn.user_text
        || first.initial_parts[0].content_json != input.turn.user_content_json
        || first.initial_parts[1..] != input.turn.additional_user_parts
    {
        return Err(invalid(
            "successor primary user seed differs from declaration",
        ));
    }
    Ok(())
}

pub(super) fn ensure_users(
    c: &Connection,
    input: &BeginSuccessorTurnInput,
) -> StoreResult<Vec<String>> {
    let source_run:String=c.query_row("SELECT source_run_id FROM input_admissions WHERE core_session_id=?1 AND admission_id=?2 AND conversation_id=?3",
        params![session(input)?,input.admission_id,input.turn.conversation_id],|row|row.get(0))?;
    let mut inserted = Vec::new();
    for message in &input.ensured_user_messages {
        let existing: Option<(String, String, Option<String>, String)> = c
            .query_row(
                "SELECT conversation_id,role,run_id,status FROM messages WHERE id=?1",
                [&message.id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?;
        if let Some((conversation, role, owner, status)) = existing {
            if conversation != input.turn.conversation_id
                || role != "user"
                || status != "completed"
                || owner.as_deref().is_some_and(|owner| owner != source_run)
            {
                return Err(invalid(
                    "successor existing user owner differs from its accepted source Run",
                ));
            }
            let count: i64 = c.query_row(
                "SELECT COUNT(*) FROM message_parts WHERE message_id=?1",
                [&message.id],
                |row| row.get(0),
            )?;
            if count != message.initial_parts.len() as i64 {
                return Err(invalid("successor existing user parts differ"));
            }
            for part in &message.initial_parts {
                let same:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM message_parts WHERE id=?1 AND message_id=?2 AND kind=?3 AND ordinal=?4 AND state=?5 AND content_text=?6)",
                    params![part.id,message.id,part.kind,part.ordinal,part.state,part.content_text],|row|row.get(0))?;
                if !same {
                    return Err(invalid("successor existing user content differs"));
                }
                let stored: Option<String> = c.query_row(
                    "SELECT content_json FROM message_parts WHERE id=?1",
                    [&part.id],
                    |row| row.get(0),
                )?;
                let actual = stored
                    .as_deref()
                    .map(serde_json::from_str::<Value>)
                    .transpose()?;
                let mut expected = part
                    .content_json
                    .as_deref()
                    .map(serde_json::from_str::<Value>)
                    .transpose()?;
                if part.kind == "attachment" {
                    // Preserve the original queued attachment owner, never
                    // rewrite an old Part to make it appear newly submitted.
                    if let Some(expected) = expected.as_mut() {
                        expected["turn_id"] = source_run.clone().into();
                    }
                }
                if actual != expected {
                    return Err(invalid("successor existing user metadata differs"));
                }
            }
        } else {
            ensure_transition_message(c, &input.turn.conversation_id, message)?;
            inserted.push(message.id.clone());
        }
    }
    Ok(inserted)
}

pub(super) fn record(
    c: &Connection,
    input: &BeginSuccessorTurnInput,
    result: &BeginTurnResult,
) -> StoreResult<()> {
    let core_session = session(input)?;
    let applied:bool=c.query_row("SELECT EXISTS(SELECT 1 FROM input_turn_applications WHERE core_session_id=?1 AND admission_id=?2 AND destination_turn_id=?3 AND source_run_id=?3)",
        params![core_session,input.admission_id,input.turn.run_id],|row|row.get(0))?;
    if !applied {
        return Err(invalid(
            "successor application fact was not committed to its new Run",
        ));
    }
    c.execute("INSERT INTO run_successor_admissions(core_session_id,admission_id,conversation_id,previous_run_id,run_id,originating_submission_id,request_digest,result_json) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
        params![core_session,input.admission_id,input.turn.conversation_id,input.previous_run_id,input.turn.run_id,input.originating_submission_id,digest(input)?,serde_json::to_string(result)?])?;
    Ok(())
}
