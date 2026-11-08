// SPDX-License-Identifier: Apache-2.0
//! Exact accepted/applied/withdrawn indices, written with their originating facts.
//! No body copies and no conversion of Turn-local sequence to Run watermark.
use crate::{params, Connection, OptionalExtension, StoreError, StoreResult};
use serde::Deserialize;
use nola_domain::core_event::CoreEventSource;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
#[cfg(test)]
#[path = "input_facts_tests.rs"]
pub(crate) mod tests;

fn invalid() -> StoreError {
    StoreError::Invariant("invalid input provenance fact".into())
}
fn conflict() -> StoreError {
    StoreError::Conflict("input provenance identity or lifecycle conflict".into())
}
fn positive(value: u64) -> StoreResult<i64> {
    let value = i64::try_from(value).map_err(|_| invalid())?;
    if value == 0 {
        return Err(invalid());
    }
    Ok(value)
}
#[derive(Deserialize)]
struct Admission {
    admission_id: String,
    submission_id: Option<String>,
    accepted_sequence: u64,
    origin: String,
    delivery: String,
    anchor_turn_id: Option<String>,
    payload_digest: String,
    item_ids: Vec<String>,
}
#[derive(Deserialize)]
struct Application {
    admission_id: String,
    destination_turn_id: String,
    applied_sequence: u64,
}

pub(super) fn index_event(
    connection: &Connection,
    event_id: &str,
) -> StoreResult<()> {
    if connection.is_autocommit() {
        return Err(invalid());
    }
    let (run, conversation, kind, json): (String, String, String, String) = connection.query_row(
        "SELECT e.run_id,r.conversation_id,e.event_kind,e.payload_json FROM run_events e
         JOIN runs r ON r.id=e.run_id WHERE e.event_id=?1",
        [event_id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
    )?;
    if !matches!(
        kind.as_str(),
        "input_accepted" | "input_applied" | "input_withdrawn"
    ) {
        return Ok(());
    }
    let payload: Value = serde_json::from_str(&json)?;
    let mut source_metadata = payload.clone();
    if let Some(object) = source_metadata.as_object_mut() {
        object.remove("items");
    }
    let source_digest = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&source_metadata)?)
    );
    if payload.get("type").and_then(Value::as_str) != Some(kind.as_str()) {
        return Err(invalid());
    }
    let envelope: CoreEventSource =
        serde_json::from_value(payload.get("core_envelope").cloned().ok_or_else(invalid)?)?;
    crate::require_id("Core input session", &envelope.session_id)?;
    let cursor = positive(envelope.cursor.sequence)?;
    let revision = i64::try_from(envelope.revision).map_err(|_| invalid())?;
    let provenance = "current_frontier";
    {
        let valid: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM core_delivery_witnesses
            WHERE conversation_id=?1 AND json_extract(witness_json,'$.session_id')=?2
            AND json_extract(witness_json,'$.state.session_id')=?2 AND event_cursor>=?3
            AND json_extract(witness_json,'$.core_revision')>=?4)",
            params![conversation, envelope.session_id, cursor, revision],
            |r| r.get(0),
        )?;
        if !valid {
            return Err(invalid());
        }
    }
    // A Core session is one conversation identity even across independent Runs.
    let crossed: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM input_admissions
        WHERE core_session_id=?1 AND conversation_id<>?2)",
        params![envelope.session_id, conversation],
        |r| r.get(0),
    )?;
    if crossed {
        return Err(conflict());
    }
    match kind.as_str() {
        "input_accepted" => {
            let admission: Admission =
                serde_json::from_value(payload.get("admission").cloned().ok_or_else(invalid)?)?;
            crate::require_id("input admission", &admission.admission_id)?;
            if let Some(id) = &admission.submission_id {
                crate::require_id("input submission", id)?;
            }
            if let Some(id) = &admission.anchor_turn_id {
                crate::require_id("input anchor", id)?;
            }
            let mut items = HashSet::new();
            for id in &admission.item_ids {
                crate::require_id("input item", id)?;
                if !items.insert(id) {
                    return Err(invalid());
                }
            }
            if !matches!(admission.delivery.as_str(), "current_turn" | "next_turn")
                || !matches!(
                    admission.origin.as_str(),
                    "unknown"
                        | "interactive_user"
                        | "goal_interactive_user"
                        | "scheduled_action"
                        | "replay"
                        | "system_recovery"
                        | "goal_continuation"
                        | "goal_audit"
                        | "delegated_agent"
                        | "maintenance"
                )
                || admission.payload_digest.len() != 64
                || !admission
                    .payload_digest
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit())
            {
                return Err(invalid());
            }
            let old: Option<(String, String)> = connection
                .query_row(
                    "SELECT source_event_id,source_digest FROM input_admissions
                WHERE core_session_id=?1 AND admission_id=?2",
                    params![envelope.session_id, admission.admission_id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            if let Some(old) = old {
                return if old == (event_id.to_owned(), source_digest.clone()) {
                    Ok(())
                } else {
                    Err(conflict())
                };
            }
            let sequence = positive(admission.accepted_sequence)?;
            let previous: Option<i64> = connection.query_row(
                "SELECT MAX(accepted_sequence) FROM input_admissions WHERE core_session_id=?1",
                [&envelope.session_id],
                |r| r.get(0),
            )?;
            // Historical gaps are not fabricated. New facts must extend the
            // actual last indexed sequence; no sequence is a completeness grant.
            if previous.unwrap_or(0).checked_add(1) != Some(sequence) {
                return Err(conflict());
            }
            connection.execute("INSERT INTO input_admissions VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)",
                params![envelope.session_id,admission.admission_id,conversation,admission.submission_id,sequence,event_id,run,revision,cursor,admission.delivery,admission.anchor_turn_id,admission.origin,admission.payload_digest,source_digest,provenance])?;
        }
        "input_applied" => {
            let turn = payload
                .get("turn_id")
                .and_then(Value::as_str)
                .ok_or_else(invalid)?;
            if envelope.turn_id.as_deref() != Some(turn) {
                return Err(invalid());
            }
            let applications: Vec<Application> =
                serde_json::from_value(payload.get("applications").cloned().ok_or_else(invalid)?)?;
            if applications.is_empty() {
                return Err(invalid());
            }
            let owned: bool = connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM assistant_turn_message_bindings
                WHERE conversation_id=?1 AND run_id=?2 AND turn_id=?3)
                OR EXISTS(SELECT 1 FROM runs r JOIN run_events e ON e.run_id=r.id
                WHERE r.id=?2 AND r.conversation_id=?1 AND r.kind='maintenance'
                AND e.event_kind='turn_started' AND json_extract(e.payload_json,'$.turn_id')=?3
                AND json_extract(e.payload_json,'$.core_envelope.session_id')=?4)",
                params![conversation, run, turn, envelope.session_id],
                |r| r.get(0),
            )?;
            if !owned {
                return Err(conflict());
            }
            let mut distinct = HashSet::new();
            for application in applications {
                if !distinct.insert(application.admission_id.clone()) {
                    return Err(invalid());
                }
                if application.destination_turn_id != turn {
                    return Err(invalid());
                }
                let accepted: Option<(String, i64,String,Option<String>,i64)> = connection
                    .query_row(
                        "SELECT conversation_id,core_cursor,delivery,anchor_turn_id,core_revision FROM input_admissions
                    WHERE core_session_id=?1 AND admission_id=?2",
                        params![envelope.session_id, application.admission_id],
                        |r| Ok((r.get(0)?, r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)),
                    )
                    .optional()?;
                if !accepted.is_some_and(|(c, at, delivery, anchor, accepted_revision)| {
                    c == conversation
                        && at < cursor
                        && accepted_revision <= revision
                        && match (delivery.as_str(), anchor.as_deref()) {
                            ("current_turn", Some(anchor)) => anchor == turn,
                            ("next_turn", Some(anchor)) => anchor != turn,
                            (_, None) => true,
                            _ => false,
                        }
                }) {
                    return Err(conflict());
                }
                let withdrawn:bool=connection.query_row("SELECT EXISTS(SELECT 1 FROM input_dispositions WHERE core_session_id=?1 AND admission_id=?2)",params![envelope.session_id,application.admission_id],|r|r.get(0))?;
                if withdrawn {
                    return Err(conflict());
                }
                let old:Option<(String,String)>=connection.query_row("SELECT source_event_id,source_digest FROM input_turn_applications WHERE core_session_id=?1 AND admission_id=?2",params![envelope.session_id,application.admission_id],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
                if let Some(old) = old {
                    if old == (event_id.to_owned(), source_digest.clone()) {
                        continue;
                    }
                    return Err(conflict());
                }
                let sequence = positive(application.applied_sequence)?;
                let previous:i64=connection.query_row("SELECT COALESCE(MAX(turn_applied_sequence),0) FROM input_turn_applications WHERE core_session_id=?1 AND destination_turn_id=?2",params![envelope.session_id,turn],|r|r.get(0))?;
                if previous.checked_add(1) != Some(sequence) {
                    return Err(conflict());
                }
                connection.execute(
                    "INSERT INTO input_turn_applications VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                    params![
                        envelope.session_id,
                        application.admission_id,
                        turn,
                        sequence,
                        source_digest,
                        run,
                        event_id,
                        revision,
                        cursor,
                        provenance
                    ],
                )?;
            }
            crate::run_input_frontier::append_event(connection, event_id)?;
        }
        "input_withdrawn" => {
            let ids: Vec<String> =
                serde_json::from_value(payload.get("admission_ids").cloned().ok_or_else(invalid)?)?;
            let mut distinct = HashSet::new();
            for id in ids {
                if !distinct.insert(id.clone()) {
                    return Err(invalid());
                }
                let accepted:bool=connection.query_row("SELECT EXISTS(SELECT 1 FROM input_admissions WHERE core_session_id=?1 AND admission_id=?2 AND conversation_id=?3 AND core_cursor<?4)",params![envelope.session_id,id,conversation,cursor],|r|r.get(0))?;
                let applied:bool=connection.query_row("SELECT EXISTS(SELECT 1 FROM input_turn_applications WHERE core_session_id=?1 AND admission_id=?2)",params![envelope.session_id,id],|r|r.get(0))?;
                if !accepted || applied {
                    return Err(conflict());
                }
                let old:Option<(String,String)>=connection.query_row("SELECT source_event_id,source_digest FROM input_dispositions WHERE core_session_id=?1 AND admission_id=?2",params![envelope.session_id,id],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
                if let Some(old) = old {
                    if old == (event_id.to_owned(), source_digest.clone()) {
                        continue;
                    }
                    return Err(conflict());
                }
                connection.execute(
                    "INSERT INTO input_dispositions VALUES(?1,?2,?3,?4,?5,?6)",
                    params![
                        envelope.session_id,
                        id,
                        source_digest,
                        event_id,
                        revision,
                        cursor
                    ],
                )?;
            }
        }
        _ => unreachable!(),
    }
    Ok(())
}
