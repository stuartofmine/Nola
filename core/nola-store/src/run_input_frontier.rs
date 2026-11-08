// SPDX-License-Identifier: Apache-2.0
//! Run-wide applied-input prefixes derived only from exact committed sources.
//! Append verifies the existing chain head and this batch, not the full history.
use crate::{params, Connection, OptionalExtension, StoreError, StoreResult};
use serde::Serialize;
use nola_domain::core_event::CoreEventSource;
use serde_json::Value;
use sha2::{Digest, Sha256};

#[cfg(test)]
#[path = "run_input_frontier_tests.rs"]
mod tests;

fn invalid() -> StoreError {
    StoreError::Invariant("Run input prefix differs from its exact committed sources".into())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct Fact {
    run_id: String,
    core_session_id: String,
    admission_id: String,
    destination_turn_id: String,
    turn_applied_sequence: i64,
    source_event_id: String,
    core_revision: i64,
    core_cursor: i64,
    event_application_index: i64,
    source_digest: String,
    admission_source_event_id: String,
    admission_source_digest: String,
    admission_payload_digest: String,
}


#[derive(Debug)]
pub(super) struct RunInputPrefix {
    pub(super) run_sequence: i64,
    pub(super) core_cursor: i64,
    pub(super) chain_digest: String,
    /// Current-format source provenance, not a completeness certificate.
    pub(super) provenance: String,
}

pub(crate) struct AuditedRunInput {
    pub commitment: nola_domain::run_input_commitment::RunInputCommitment,
    pub applied: Vec<nola_domain::core_requirements::CoreAppliedInputRef>,
    pub originating_submission_id: String,
    pub item_ids: std::collections::BTreeSet<String>,
}

/// Full bidirectional audit at an already authenticated decision envelope.
/// Unlike read_prefix, this enumerates original events as well as derived rows:
/// an omitted original application cannot masquerade as an empty/short prefix.
pub(crate) fn audit_for_decision(
    c: &Connection, conversation: &str, run: &str, session: &str,
    revision: u64, cursor: u64,
) -> StoreResult<AuditedRunInput> {
    use nola_domain::assistant_journal::AssistantDigest;
    use nola_domain::assistant_journal_operations::JournalNumber;
    use nola_domain::core_requirements::CoreAppliedInputRef;
    use nola_domain::run_input_commitment::RunInputCommitment;
    let mut statement = c.prepare(
        "SELECT event_id FROM run_events WHERE run_id=?1 AND event_kind='input_applied' ORDER BY sequence"
    )?;
    let ids = statement.query_map([run], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    let mut facts = Vec::new();
    for id in ids {
        let original = event_source(c, &id)?;
        if original.conversation != conversation || original.envelope.session_id != session {
            return Err(invalid());
        }
        if original.envelope.cursor.sequence > cursor { continue; }
        if original.envelope.revision > revision { return Err(invalid()); }
        let applications = original.event["applications"].as_array().ok_or_else(invalid)?;
        if applications.is_empty() { return Err(invalid()); }
        for index in 0..applications.len() {
            facts.push(source_from_event(c, &id, i64::try_from(index).map_err(|_| invalid())?, &original)?);
        }
    }
    facts.sort_by_key(|(fact, _)| (fact.core_cursor, fact.event_application_index));
    let through = i64::try_from(cursor).map_err(|_| invalid())?;
    for table in ["run_input_applications", "input_turn_applications"] {
        let column = if table == "run_input_applications" { "run_id" } else { "source_run_id" };
        let count: i64 = c.query_row(
            &format!("SELECT COUNT(*) FROM {table} WHERE {column}=?1 AND core_cursor<=?2"),
            params![run, through], |r| r.get(0),
        )?;
        if count != facts.len() as i64 { return Err(invalid()); }
    }
    let mut previous = String::new();
    let mut previous_position = None;
    let mut applied = Vec::new();
    let mut originating_submission_id = None;
    let mut item_ids = std::collections::BTreeSet::new();
    for (index, (fact, provenance)) in facts.iter().enumerate() {
        let sequence = i64::try_from(index + 1).map_err(|_| invalid())?;
        if previous_position.is_some_and(|(rev, pos)| rev > fact.core_revision
            || pos >= (fact.core_cursor, fact.event_application_index)) {
            return Err(invalid());
        }
        let (stored, stored_provenance, stored_digest) = read_row(c, run, sequence)?;
        let expected_digest = chain(fact, sequence, &previous, provenance)?;
        if stored != *fact || stored_provenance != *provenance || stored_digest != expected_digest {
            return Err(invalid());
        }
        previous = expected_digest;
        previous_position = Some((fact.core_revision, (fact.core_cursor, fact.event_application_index)));
        applied.push(CoreAppliedInputRef {
            admission_id: fact.admission_id.clone(), turn_id: fact.destination_turn_id.clone(),
            applied_sequence: JournalNumber(fact.turn_applied_sequence as u64),
        });
        let accepted: String = c.query_row("SELECT payload_json FROM run_events WHERE event_id=?1",
            [&fact.admission_source_event_id], |r| r.get(0))?;
        let accepted: Value = serde_json::from_str(&accepted)?;
        for item in accepted["admission"]["item_ids"].as_array().ok_or_else(invalid)? {
            item_ids.insert(item.as_str().ok_or_else(invalid)?.to_owned());
        }
        if index == 0 {
            let origin: Option<String> = c.query_row(
                "SELECT submission_id FROM input_admissions WHERE core_session_id=?1 AND admission_id=?2",
                params![session, fact.admission_id], |r| r.get(0),
            )?;
            originating_submission_id = origin;
        }
    }
    let origin = originating_submission_id.ok_or_else(|| StoreError::Invariant(
        "Run decision has no original applied submission".into()
    ))?;
    crate::require_id("original Run submission", &origin)?;
    let commitment = RunInputCommitment {
        watermark: JournalNumber(facts.len() as u64),
        applications_digest: if facts.is_empty() { RunInputCommitment::empty_applications_digest() }
            else { AssistantDigest::parse_hex(&previous).map_err(|_| invalid())? },
    };
    commitment.validate().map_err(|_| invalid())?;
    Ok(AuditedRunInput { commitment, applied, originating_submission_id: origin, item_ids })
}

fn metadata_digest(value: &Value) -> StoreResult<String> {
    let mut metadata = value.clone();
    metadata
        .as_object_mut()
        .ok_or_else(invalid)?
        .remove("items");
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&metadata)?)
    ))
}

struct EventSource {
    run: String,
    conversation: String,
    event: Value,
    envelope: CoreEventSource,
    digest: String,
}

fn event_source(c: &Connection, event_id: &str) -> StoreResult<EventSource> {
    let (run, conversation, kind, json): (String, String, String, String) = c.query_row(
        "SELECT e.run_id,r.conversation_id,e.event_kind,e.payload_json FROM run_events e
         JOIN runs r ON r.id=e.run_id WHERE e.event_id=?1",
        [event_id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
    )?;
    let event: Value = serde_json::from_str(&json)?;
    if kind != "input_applied" || event["type"] != kind {
        return Err(invalid());
    }
    let envelope: CoreEventSource = serde_json::from_value(event["core_envelope"].clone())?;
    let digest = metadata_digest(&event)?;
    Ok(EventSource {
        run,
        conversation,
        event,
        envelope,
        digest,
    })
}

fn source(c: &Connection, event_id: &str, index: i64) -> StoreResult<(Fact, String)> {
    let source = event_source(c, event_id)?;
    source_from_event(c, event_id, index, &source)
}

fn source_from_event(
    c: &Connection,
    event_id: &str,
    index: i64,
    source: &EventSource,
) -> StoreResult<(Fact, String)> {
    let EventSource {
        run,
        conversation,
        event,
        envelope,
        digest,
    } = source;
    let turn = event["turn_id"].as_str().ok_or_else(invalid)?;
    let application = event["applications"]
        .as_array()
        .ok_or_else(invalid)?
        .get(usize::try_from(index).map_err(|_| invalid())?)
        .ok_or_else(invalid)?;
    let admission = application["admission_id"].as_str().ok_or_else(invalid)?;
    let local = application["applied_sequence"]
        .as_i64()
        .ok_or_else(invalid)?;
    if envelope.turn_id.as_deref() != Some(turn)
        || application["destination_turn_id"] != turn
        || local <= 0
    {
        return Err(invalid());
    }
    crate::require_id("Run input Core session", &envelope.session_id)?;
    let revision = i64::try_from(envelope.revision).map_err(|_| invalid())?;
    let cursor = i64::try_from(envelope.cursor.sequence).map_err(|_| invalid())?;
    if cursor <= 0 {
        return Err(invalid());
    }
    let (
        app_turn,
        app_local,
        app_event,
        app_run,
        app_revision,
        app_cursor,
        app_digest,
        app_provenance,
    ): (String, i64, String, String, i64, i64, String, String) = c.query_row(
        "SELECT destination_turn_id,turn_applied_sequence,source_event_id,source_run_id,
            core_revision,core_cursor,source_digest,provenance FROM input_turn_applications
         WHERE core_session_id=?1 AND admission_id=?2",
        params![envelope.session_id, admission],
        |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
                r.get(7)?,
            ))
        },
    )?;
    if (
        app_turn.as_str(),
        app_local,
        app_event.as_str(),
        app_run.as_str(),
        app_revision,
        app_cursor,
        app_digest.as_str(),
    ) != (
        turn,
        local,
        event_id,
        run.as_str(),
        revision,
        cursor,
        digest.as_str(),
    ) {
        return Err(invalid());
    }
    let owned:bool = c.query_row("SELECT EXISTS(SELECT 1 FROM assistant_turn_message_bindings
        WHERE conversation_id=?1 AND run_id=?2 AND turn_id=?3)
        OR EXISTS(SELECT 1 FROM runs r JOIN run_events e ON e.run_id=r.id
        WHERE r.id=?2 AND r.conversation_id=?1 AND r.kind='maintenance' AND e.event_kind='turn_started'
        AND json_extract(e.payload_json,'$.turn_id')=?3
        AND json_extract(e.payload_json,'$.core_envelope.session_id')=?4)",
        params![conversation,run,turn,envelope.session_id],|r|r.get(0))?;
    if !owned {
        return Err(invalid());
    }
    let (
        admission_conversation,
        admission_event,
        admission_digest,
        payload_digest,
        admission_provenance,
    ): (String, String, String, String, String) = c.query_row(
        "SELECT conversation_id,source_event_id,
        source_digest,payload_digest,provenance FROM input_admissions
        WHERE core_session_id=?1 AND admission_id=?2",
        params![envelope.session_id, admission],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
    )?;
    let (admission_kind,admission_json,source_conversation):(String,String,String) = c.query_row(
        "SELECT e.event_kind,e.payload_json,r.conversation_id FROM run_events e JOIN runs r ON r.id=e.run_id
         JOIN input_admissions a ON a.source_event_id=e.event_id AND a.source_run_id=e.run_id
          AND a.core_session_id=?2 AND a.admission_id=?3
          AND a.core_cursor=json_extract(e.payload_json,'$.core_envelope.cursor.sequence')
          AND a.core_revision=json_extract(e.payload_json,'$.core_envelope.revision')
         WHERE e.event_id=?1", params![admission_event,envelope.session_id,admission], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
    let accepted: Value = serde_json::from_str(&admission_json)?;
    let accepted_envelope: CoreEventSource = serde_json::from_value(accepted["core_envelope"].clone())?;
    if admission_conversation != *conversation
        || source_conversation != *conversation
        || admission_kind != "input_accepted"
        || accepted["type"] != "input_accepted"
        || accepted_envelope.session_id != envelope.session_id
        || accepted_envelope.cursor.sequence >= envelope.cursor.sequence
        || accepted_envelope.revision > envelope.revision
        || accepted["admission"]["admission_id"] != admission
        || accepted["admission"]["payload_digest"] != payload_digest
        || metadata_digest(&accepted)? != admission_digest
    {
        return Err(invalid());
    }
    let provenance = combine_provenance(&app_provenance, &admission_provenance)?;
    Ok((
        Fact {
            run_id: run.clone(),
            core_session_id: envelope.session_id.clone(),
            admission_id: admission.into(),
            destination_turn_id: turn.into(),
            turn_applied_sequence: local,
            source_event_id: event_id.into(),
            core_revision: revision,
            core_cursor: cursor,
            event_application_index: index,
            source_digest: digest.clone(),
            admission_source_event_id: admission_event,
            admission_source_digest: admission_digest,
            admission_payload_digest: payload_digest,
        },
        provenance,
    ))
}

fn combine_provenance(a: &str, b: &str) -> StoreResult<String> {
    for value in [a, b] {
        if value != "current_frontier" {
            return Err(invalid());
        }
    }
    Ok("current_frontier".into())
}

fn chain(fact: &Fact, sequence: i64, previous: &str, provenance: &str) -> StoreResult<String> {
    let bytes = serde_json::to_vec(&(sequence, previous, provenance, fact))?;
    let mut hash = Sha256::new();
    hash.update(b"NOLA-RUN-INPUT-PREFIX-V1");
    hash.update((bytes.len() as u64).to_be_bytes());
    hash.update(bytes);
    Ok(format!("{:x}", hash.finalize()))
}

fn read_row(c: &Connection, run: &str, sequence: i64) -> StoreResult<(Fact, String, String)> {
    c.query_row(
        "SELECT core_session_id,admission_id,destination_turn_id,turn_applied_sequence,
        source_event_id,core_revision,core_cursor,event_application_index,source_digest,
        admission_source_digest,admission_payload_digest,provenance,chain_digest,admission_source_event_id
        FROM run_input_applications WHERE run_id=?1 AND run_sequence=?2",
        params![run, sequence],
        |r| {
            Ok((
                Fact {
                    run_id: run.into(),
                    core_session_id: r.get(0)?,
                    admission_id: r.get(1)?,
                    destination_turn_id: r.get(2)?,
                    turn_applied_sequence: r.get(3)?,
                    source_event_id: r.get(4)?,
                    core_revision: r.get(5)?,
                    core_cursor: r.get(6)?,
                    event_application_index: r.get(7)?,
                    source_digest: r.get(8)?,
                    admission_source_event_id: r.get(13)?,
                    admission_source_digest: r.get(9)?,
                    admission_payload_digest: r.get(10)?,
                },
                r.get(11)?,
                r.get(12)?,
            ))
        },
    )
    .map_err(StoreError::from)
}

/// Validates a committed chain head against original sources. This bounded
/// reader is not a full historical audit and does not authorize a result seal.
pub(super) fn read_prefix(
    c: &Connection,
    run: &str,
    through_cursor: u64,
) -> StoreResult<Option<RunInputPrefix>> {
    let through = i64::try_from(through_cursor).map_err(|_| invalid())?;
    let sequence: Option<i64> = c
        .query_row(
            "SELECT run_sequence FROM run_input_applications
        WHERE run_id=?1 AND core_cursor<=?2 ORDER BY run_sequence DESC LIMIT 1",
            params![run, through],
            |r| r.get(0),
        )
        .optional()?;
    let Some(sequence) = sequence else {
        return Ok(None);
    };
    let (fact, provenance, digest) = read_row(c, run, sequence)?;
    let (original, source_provenance) =
        source(c, &fact.source_event_id, fact.event_application_index)?;
    if original != fact {
        return Err(invalid());
    }
    let (previous, previous_provenance) = if sequence == 1 {
        (String::new(), "current_frontier".into())
    } else {
        let (_, p, d) = read_row(c, run, sequence - 1)?;
        (d, p)
    };
    if combine_provenance(&previous_provenance, &source_provenance)? != provenance
        || chain(&fact, sequence, &previous, &provenance)? != digest
    {
        return Err(invalid());
    }
    Ok(Some(RunInputPrefix {
        run_sequence: sequence,
        core_cursor: fact.core_cursor,
        chain_digest: digest,
        provenance,
    }))
}

pub(super) fn append_event(c: &Connection, event_id: &str) -> StoreResult<()> {
    if c.is_autocommit() {
        return Err(invalid());
    }
    // Parse and hash the batch once, regardless of its number of applications.
    let batch = event_source(c, event_id)?;
    let count = i64::try_from(
        batch.event["applications"]
            .as_array()
            .ok_or_else(invalid)?
            .len(),
    )
    .map_err(|_| invalid())?;
    if count <= 0 {
        return Err(invalid());
    }
    let mut head = read_prefix(c, &batch.run, i64::MAX as u64)?;
    for index in 0..count {
        let (fact, source_provenance) = source_from_event(c, event_id, index, &batch)?;
        let old: Option<(String, i64)> = c
            .query_row(
                "SELECT run_id,run_sequence FROM run_input_applications
            WHERE core_session_id=?1 AND admission_id=?2",
                params![fact.core_session_id, fact.admission_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((run, sequence)) = old {
            let (existing, provenance, digest) = read_row(c, &run, sequence)?;
            if existing != fact {
                return Err(invalid());
            }
            // Replay validates the corresponding committed prefix, without
            // consuming a new Run sequence or upgrading historical provenance.
            let (previous, previous_provenance) = if sequence == 1 {
                (String::new(), "current_frontier".into())
            } else {
                let (_, p, d) = read_row(c, &run, sequence - 1)?;
                (d, p)
            };
            if combine_provenance(&previous_provenance, &source_provenance)? != provenance
                || chain(&fact, sequence, &previous, &provenance)? != digest
            {
                return Err(invalid());
            }
            continue;
        }
        let (sequence, previous, provenance) = if let Some(head) = head.as_ref() {
            let (old, _, _) = read_row(c, &fact.run_id, head.run_sequence)?;
            if old.core_session_id != fact.core_session_id
                || old.core_revision > fact.core_revision
                || (head.core_cursor, old.event_application_index) >= (fact.core_cursor, index)
                || (head.core_cursor == fact.core_cursor
                    && old.source_event_id != fact.source_event_id)
            {
                return Err(invalid());
            }
            (
                head.run_sequence.checked_add(1).ok_or_else(invalid)?,
                head.chain_digest.clone(),
                combine_provenance(&head.provenance, &source_provenance)?,
            )
        } else {
            (1, String::new(), source_provenance)
        };
        let digest = chain(&fact, sequence, &previous, &provenance)?;
        c.execute("INSERT INTO run_input_applications VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)",
            params![fact.run_id,sequence,fact.core_session_id,fact.admission_id,fact.destination_turn_id,
                fact.turn_applied_sequence,fact.source_event_id,fact.core_revision,fact.core_cursor,index,
                fact.source_digest,fact.admission_source_digest,fact.admission_payload_digest,provenance,digest,
                fact.admission_source_event_id])?;
        head = Some(RunInputPrefix {
            run_sequence: sequence,
            core_cursor: fact.core_cursor,
            chain_digest: digest,
            provenance,
        });
    }
    Ok(())
}
