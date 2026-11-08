// SPDX-License-Identifier: Apache-2.0
//! Atomic local and remote transition commits with run events and effects.

use super::*;
use nola_store_port::run_effect::CheckpointWrite;

fn validate_run_lifecycle(status: &str) -> StoreResult<()> {
    match status {
        "queued" | "running" | "waiting_input" | "sealed" => Ok(()),
        _ => Err(StoreError::Invariant(format!(
            "invalid Run lifecycle {status:?}; message closure and business results are not Run lifecycle states"
        ))),
    }
}

#[cfg(test)]
mod run_lifecycle_contract_tests {
    use super::validate_run_lifecycle;

    #[test]
    fn rejects_message_states_business_outcomes_and_unknown_values() {
        for status in ["queued", "running", "waiting_input", "sealed"] {
            assert!(validate_run_lifecycle(status).is_ok(), "{status}");
        }
        for status in ["", "streaming", "completed", "failed", "cancelled",
            "interrupted", "answered", "action_completed", "incomplete", "blocked", "unknown"] {
            assert!(validate_run_lifecycle(status).is_err(), "{status}");
        }
    }
}

#[derive(PartialEq)]
struct OmittedReplayPayload;
impl<'de> serde::Deserialize<'de> for OmittedReplayPayload {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        <serde::de::IgnoredAny as serde::Deserialize>::deserialize(deserializer)?;
        Ok(Self)
    }
}

fn validate_reduction_identity(
    event: &nola_store_port::run_effect::HostEventInput,
) -> StoreResult<()> {
    let Some(reduction) = &event.reduction_json else {
        return Ok(());
    };
    type Identity = nola_domain::host_effect::HostResponseEnvelope<OmittedReplayPayload>;
    let raw: Identity = serde_json::from_str(&event.payload_json).map_err(|error| {
        StoreError::Invariant(format!("invalid replay source envelope: {error}"))
    })?;
    let facts: nola_store_port::host_replay::HostReplayReduction = serde_json::from_str(reduction)
        .map_err(|error| StoreError::Invariant(format!("invalid typed replay facts: {error}")))?;
    facts.validate_clock(event.received_at_ms)
        .map_err(|error| StoreError::Conflict(error.into()))?;
    if !matches!(&facts, nola_store_port::host_replay::HostReplayReduction::Replayable { .. }) {
        return Err(StoreError::Conflict("only complete replay facts may be newly committed".into()));
    }
    let reduced: Identity = serde_json::from_str(facts.normalized_response().get()).map_err(|error| {
        StoreError::Invariant(format!("invalid replay reduction envelope: {error}"))
    })?;
    if raw != reduced
        || raw.effect_id.0 != event.effect_id
        || raw.request_id != event.request_id
        || i64::try_from(raw.sequence).ok() != Some(event.sequence)
        || raw.session_id.is_empty()
        || event.received_at_ms < 0
    {
        return Err(StoreError::Conflict(
            "Core replay input changed its Host identity or observation time".into(),
        ));
    }
    Ok(())
}

#[test]
fn normalized_replay_may_change_payload_but_not_delivery_identity() {
    let facts = |response: &serde_json::Value| serde_json::to_string(
        &nola_store_port::host_replay::HostReplayReduction::Replayable {
            protocol_version:1,decision_time_ms:123,binding:None,
            normalized_response:serde_json::value::RawValue::from_string(response.to_string()).unwrap(),
        }).unwrap();
    let raw = serde_json::json!({
        "request_id":"request", "effect_id":"effect", "host_id":"host", "lease_id":"lease",
        "session_id":"session", "turn_id":"turn", "catalog_revision":u64::MAX,
        "availability_epoch":u64::MAX, "sequence":7, "attempt_id":"attempt",
        "delegated_invocation_id":"invocation", "payload":{"type":"execution_started"}
    });
    let mut reduced = raw.clone();
    reduced["payload"] = serde_json::json!({"type":"accepted"});
    let event = nola_store_port::run_effect::HostEventInput {
        event_id: "event".into(),
        effect_id: "effect".into(),
        request_id: "request".into(),
        sequence: 7,
        payload_json: raw.to_string(),
        reduction_json: Some(facts(&reduced)),
        received_at_ms: 123,
    };
    validate_reduction_identity(&event).unwrap();
    for (field, value) in [
        ("request_id", serde_json::json!("other")),
        ("effect_id", serde_json::json!("other")),
        ("session_id", serde_json::json!("other")),
        ("turn_id", serde_json::Value::Null),
        ("host_id", serde_json::json!("other")),
        ("lease_id", serde_json::Value::Null),
        ("attempt_id", serde_json::Value::Null),
        ("delegated_invocation_id", serde_json::Value::Null),
        ("catalog_revision", serde_json::json!(1)),
        ("availability_epoch", serde_json::json!(1)),
        ("sequence", serde_json::json!(8)),
    ] {
        let mut changed = reduced.clone();
        changed[field] = value;
        let mut invalid = event.clone();
        invalid.reduction_json = Some(facts(&changed));
        assert!(validate_reduction_identity(&invalid).is_err(), "{field}");
    }
    let mut invalid = event;
    invalid.received_at_ms = -1;
    assert!(validate_reduction_identity(&invalid).is_err());
}

pub(crate) fn matching_host_event_replay(
    connection: &Connection,
    run_id: &str,
    event: &nola_store_port::run_effect::HostEventInput,
) -> StoreResult<bool> {
    require_id("host_event.event_id", &event.event_id)?;
    require_id("host_event.effect_id", &event.effect_id)?;
    require_id("host_event.request_id", &event.request_id)?;
    validate_json("host_event.payload_json", &event.payload_json)?;
    validate_reduction_identity(event)?;
    let mut query = connection.prepare(
        "SELECT event_id,run_id,effect_id,request_id,sequence,payload_json,reduction_json FROM host_event_inbox
         WHERE event_id=?1 OR (run_id=?2 AND effect_id=?3 AND sequence=?4)")?;
    let mut rows = query.query(params![
        event.event_id,
        run_id,
        event.effect_id,
        event.sequence
    ])?;
    let mut found = false;
    while let Some(row) = rows.next()? {
        if row.get::<_, String>(0)? != event.event_id
            || row.get::<_, String>(1)? != run_id
            || row.get::<_, String>(2)? != event.effect_id
            || row.get::<_, String>(3)? != event.request_id
            || row.get::<_, i64>(4)? != event.sequence
            || row.get::<_, String>(5)? != event.payload_json
            || row.get::<_, Option<String>>(6)? != event.reduction_json
        {
            return Err(StoreError::Conflict(
                "Host event replay changed identity or payload".into(),
            ));
        }
        found = true;
    }
    // A retry may arrive later, but never overwrites the first committed clock.
    Ok(found)
}

impl NolaStore {
    pub fn commit_transition(
        &mut self,
        input: &CommitTransitionInput,
    ) -> StoreResult<CommitTransitionResult> {
        if input.run_status.as_deref()==Some("sealed") && !self.has_write_unit_of_work() {
            return Err(StoreError::Invariant("terminal transition requires an outer result publication unit of work".into()));
        }
        require_id("transition_id", &input.transition_id)?;
        require_id("run_id", &input.run_id)?;
        let submission = input.submission_reduction_json.as_ref().map(|raw| {
            let facts: nola_store_port::run_effect::SubmissionReplayReduction = serde_json::from_str(raw)?;
            require_id("submission.session_id", &facts.session_id)?;
            let value: serde_json::Value = serde_json::from_str(&facts.submission_json)?;
            if input.host_event.is_some() || facts.protocol_version != 1
                || facts.decision_time_ms != input.committed_at_ms || facts.decision_time_ms < 0
                || facts.bindings.iter().any(|binding| binding.session_id != facts.session_id)
                || value.get("type").and_then(|value| value.as_str()).is_none()
                || value.get("type").and_then(|value| value.as_str()) == Some("host_response")
            {
                return Err(StoreError::Invariant("invalid non-Host reducer input".into()));
            }
            Ok(facts)
        }).transpose()?;
        match &input.checkpoint {
            CheckpointWrite::Baseline { checkpoint_json } => {
                validate_json("checkpoint_json", checkpoint_json)?
            }
            CheckpointWrite::Replay {
                delivery_witness_json,
            } => {
                validate_json("delivery_witness_json", delivery_witness_json)?;
                if input
                    .host_event
                    .as_ref()
                    .and_then(|event| event.reduction_json.as_ref())
                    .is_none() && submission.is_none()
                {
                    return Err(StoreError::Invariant(
                        "incremental checkpoint requires a recorded reducer input".into(),
                    ));
                }
            }
        }
        if input.checkpoint_revision <= input.expected_checkpoint_revision {
            return Err(StoreError::Invariant(
                "checkpoint revision must advance".to_string(),
            ));
        }

        let transaction = self.write_transaction()?;
        let host_replay = input
            .host_event
            .as_ref()
            .map(|event| matching_host_event_replay(&transaction, &input.run_id, event))
            .transpose()?
            .unwrap_or(false);
        if let Some(existing) = transition_result(&transaction, &input.transition_id)? {
            let recorded: Option<String> = transaction.query_row(
                "SELECT submission_json FROM host_event_replay_order WHERE submission_id=?1 AND run_id=?2",
                params![input.transition_id,input.run_id], |row| row.get(0)).optional()?;
            if recorded.as_deref() != input.submission_reduction_json.as_deref() {
                return Err(StoreError::Conflict("transition replay changed its reducer input".into()));
            }
            let owner: String = transaction.query_row(
                "SELECT run_id FROM committed_transitions WHERE transition_id=?1",
                [&input.transition_id],
                |row| row.get(0),
            )?;
            if owner != input.run_id || (input.host_event.is_some() && !host_replay) {
                return Err(StoreError::Conflict(
                    "transition replay changed its Run or Host event".into(),
                ));
            }
            transaction.commit()?;
            return Ok(existing);
        }

        let (conversation_id, run_kind, assistant_message_id, mut next_event_sequence): (
            String,
            String,
            Option<String>,
            i64,
        ) = transaction
            .query_row(
                "SELECT conversation_id, kind, assistant_message_id, next_event_sequence
                     FROM runs WHERE id = ?1",
                params![input.run_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?
            .ok_or_else(|| StoreError::Invariant(format!("unknown run {}", input.run_id)))?;

        if let Some(submission) = submission {
            let owns_session: bool = transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM core_delivery_witnesses WHERE conversation_id=?1
                 AND checkpoint_revision=?2 AND json_extract(witness_json,'$.session_id')=?3)",
                params![conversation_id,input.expected_checkpoint_revision,submission.session_id], |row| row.get(0))?;
            if !owns_session {
                return Err(StoreError::Conflict("submission replay has no exact session/revision owner".into()));
            }
            let raw = input.submission_reduction_json.as_ref().expect("validated submission");
            transaction.execute(
                "INSERT INTO host_event_replay_heads(run_id,ordinal,reduction_bytes) VALUES(?1,1,?2)
                 ON CONFLICT(run_id) DO UPDATE SET ordinal=ordinal+1,reduction_bytes=reduction_bytes+excluded.reduction_bytes",
                params![input.run_id,raw.len() as i64])?;
            transaction.execute(
                "INSERT INTO host_event_replay_order(run_id,ordinal,submission_id,submission_json,decision_time_ms)
                 SELECT run_id,ordinal,?2,?3,?4 FROM host_event_replay_heads WHERE run_id=?1",
                params![input.run_id,input.transition_id,raw,submission.decision_time_ms])?;
        }

        if let Some(host_event) = &input.host_event {
            if host_replay {
                let checkpoint_revision = transaction.query_row(
                    "SELECT revision FROM core_checkpoints WHERE conversation_id=?1",
                    [&conversation_id],
                    |row| row.get(0),
                )?;
                transaction.commit()?;
                return Ok(CommitTransitionResult {
                    applied: false,
                    checkpoint_revision,
                    next_event_sequence,
                });
            }
            transaction.execute(
                "INSERT INTO host_event_inbox(
                    event_id, run_id, effect_id, request_id, sequence, payload_json, received_at_ms, reduction_json
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    host_event.event_id,
                    input.run_id,
                    host_event.effect_id,
                    host_event.request_id,
                    host_event.sequence,
                    host_event.payload_json,
                    host_event.received_at_ms,
                    host_event.reduction_json
                ],
            )?;
        }

        let stored_revision: i64 = transaction
            .query_row(
                "SELECT revision FROM core_checkpoints WHERE conversation_id = ?1",
                params![conversation_id],
                |row| row.get(0),
            )
            .optional()?
            .ok_or_else(|| StoreError::Invariant("run has no Core checkpoint".to_string()))?;
        if stored_revision != input.expected_checkpoint_revision {
            return Err(StoreError::Conflict(format!(
                "checkpoint revision is {stored_revision}, expected {}",
                input.expected_checkpoint_revision
            )));
        }

        crate::cancellation_not_started::validate(&transaction, input)?;
        permission_source::validate_settlements(&transaction, input)?;
        for event in &input.events {
            require_id("run_event.event_id", &event.event_id)?;
            validate_json("run_event.payload_json", &event.payload_json)?;
            if event.sequence != next_event_sequence {
                return Err(StoreError::Conflict(format!(
                    "run event sequence is {}, expected {next_event_sequence}",
                    event.sequence
                )));
            }
            transaction.execute(
                "INSERT INTO run_events(event_id, run_id, sequence, event_kind, payload_json, created_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    event.event_id,
                    input.run_id,
                    event.sequence,
                    event.event_kind,
                    event.payload_json,
                    event.created_at_ms
                ],
            )?;
            next_event_sequence += 1;
        }

        if run_kind == "maintenance"
            && (!input.ensured_messages.is_empty()
                || !input.part_message_owners.is_empty()
                || !input.assistant_turn_bindings.is_empty()
                || !input.part_mutations.is_empty()
                || input.assistant_status.is_some())
        {
            return Err(StoreError::Invariant(
                "maintenance Run cannot mutate conversation message state".to_string(),
            ));
        }
        let replica_messages = conversation_transition_sync::prepare_messages(
            &transaction,
            &conversation_id,
            &input.ensured_messages,
        )?;
        let mut replica_changes = conversation_transition_sync::PreparedChanges::default();
        for message in &input.ensured_messages {
            if message.run_id.as_deref() != Some(input.run_id.as_str()) {
                return Err(StoreError::Invariant(
                    "transition Message must belong to the exact Run".into(),
                ));
            }
            ensure_transition_message(&transaction, &conversation_id, message)?;
        }
        for mutation in &input.part_mutations {
            let part_id = part_mutation_id(mutation);
            let owner = input
                .part_message_owners
                .get(part_id)
                .map(String::as_str)
                .or(assistant_message_id.as_deref())
                .ok_or_else(|| {
                    StoreError::Invariant(
                        "conversation Part mutation has no assistant owner".to_string(),
                    )
                })?;
            let owner_is_canonical: bool = transaction.query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM messages
                    WHERE id = ?1 AND conversation_id = ?2 AND role = 'assistant'
                        AND run_id = ?3
                 )",
                params![owner, conversation_id, input.run_id],
                |row| row.get(0),
            )?;
            if !owner_is_canonical {
                return Err(StoreError::Invariant(format!(
                    "Part {part_id} references non-canonical assistant Message {owner}"
                )));
            }
            assistant_turn_ownership::validate_part_write_owner(
                &transaction, &conversation_id, &input.run_id, owner, part_id)?;
            replica_changes.record_part_before_apply(
                &transaction,
                &conversation_id,
                owner,
                mutation,
            )?;
            apply_part_mutation(&transaction, owner, mutation)?;
        }

        assistant_turn_ownership::apply(
            &transaction,
            &conversation_id,
            &input.run_id,
            assistant_message_id.as_deref(),
            false,
            &input.assistant_turn_bindings,
        )?;

        if let Some(status) = &input.run_status {
            validate_run_lifecycle(status)?;
            let terminal = status == "sealed";
            transaction.execute(
                "UPDATE runs SET status = ?2, next_event_sequence = ?3, updated_at_ms = ?4,
                    completed_at_ms = CASE WHEN ?5 THEN ?4 ELSE completed_at_ms END
                 WHERE id = ?1",
                params![
                    input.run_id,
                    status,
                    next_event_sequence,
                    input.committed_at_ms,
                    terminal
                ],
            )?;
        } else {
            transaction.execute(
                "UPDATE runs SET next_event_sequence = ?2, updated_at_ms = ?3 WHERE id = ?1",
                params![input.run_id, next_event_sequence, input.committed_at_ms],
            )?;
        }

        if let Some(status) = &input.assistant_status {
            let assistant_message_id = input
                .assistant_status_message_id
                .as_deref()
                .or(assistant_message_id.as_deref())
                .ok_or_else(|| {
                    StoreError::Invariant("assistant status has no message owner".to_string())
                })?;
            let terminal = is_message_terminal(status);
            let mut targets = vec![assistant_message_id.to_owned()];
            if terminal {
                let mut query = transaction.prepare(
                    "SELECT segment.message_id FROM assistant_message_segments segment
                     JOIN assistant_turn_message_bindings root ON root.run_id=segment.run_id AND root.turn_id=segment.turn_id
                     WHERE segment.run_id=?1 AND root.message_id=?2 ORDER BY segment.source_sequence")?;
                let segments = query.query_map(params![input.run_id,assistant_message_id], |row|row.get::<_,String>(0))?
                    .collect::<Result<Vec<_>,_>>()?;
                targets.extend(segments);
                targets.sort(); targets.dedup();
            }
            for target_message_id in &targets {
                replica_changes.record_status_before_apply(
                    &transaction,
                    &conversation_id,
                    target_message_id,
                    status,
                    input.committed_at_ms,
                )?;
                let updated = transaction.execute(
                    "UPDATE messages SET status = ?2,
                        completed_at_ms = CASE WHEN ?3 THEN ?4 ELSE completed_at_ms END
                     WHERE id = ?1 AND conversation_id = ?5 AND run_id = ?6
                        AND role = 'assistant'",
                    params![
                        target_message_id,
                        status,
                        terminal,
                        input.committed_at_ms,
                        conversation_id,
                        input.run_id
                    ],
                )?;
                if updated != 1 {
                    return Err(StoreError::Invariant(
                        "assistant status must reference an assistant owned by the exact Run".into(),
                    ));
                }
            }

        }

        let replay_base = if let CheckpointWrite::Replay {
            delivery_witness_json,
        } = &input.checkpoint
        {
            let base: Option<(i64, i64, i64)> = transaction.query_row(
                "SELECT checkpoint_revision,through_ordinal,through_reduction_bytes FROM core_checkpoint_replay_bases WHERE conversation_id=?1 AND run_id=?2",
                params![conversation_id,input.run_id], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
            ).optional()?;
            let base = base.ok_or_else(|| {
                StoreError::Conflict("incremental checkpoint has no exact Run baseline".into())
            })?;
            let (head, count, missing, head_bytes): (i64, i64, i64, i64) = transaction.query_row(
                "SELECT COALESCE((SELECT ordinal FROM host_event_replay_heads WHERE run_id=?1),0),
                 count(*),COALESCE(sum(COALESCE(o.submission_json,h.reduction_json) IS NULL),0),
                 COALESCE((SELECT reduction_bytes FROM host_event_replay_heads WHERE run_id=?1),0)
                 FROM host_event_replay_order o LEFT JOIN host_event_inbox h ON h.event_id=o.event_id AND h.run_id=o.run_id
                 WHERE o.run_id=?1 AND o.ordinal>?2",
                params![input.run_id,base.1], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)),
            )?;
            if base.0 > input.expected_checkpoint_revision
                || head.checked_sub(base.1) != Some(count)
                || count == 0
                || missing != 0
                || count > nola_store_port::host_replay::HOST_REPLAY_MAX_RECORDS as i64
                || head_bytes.checked_sub(base.2).is_none_or(|bytes| bytes < 0
                    || bytes > nola_store_port::host_replay::HOST_REPLAY_MAX_BYTES as i64)
            {
                return Err(StoreError::Conflict(
                    "incremental checkpoint has an incomplete reducer history".into(),
                ));
            }
            let valid_session: bool = transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM core_delivery_witnesses WHERE conversation_id=?1
                 AND checkpoint_revision=?2 AND json_extract(witness_json,'$.session_id')=json_extract(?3,'$.session_id')
                 AND json_extract(?3,'$.session_id')=json_extract(?3,'$.state.session_id'))",
                params![conversation_id,input.expected_checkpoint_revision,delivery_witness_json], |row| row.get(0),
            )?;
            if !valid_session {
                return Err(StoreError::Conflict(
                    "incremental delivery witness changed Core session ownership".into(),
                ));
            }
            Some(base)
        } else {
            None
        };
        match &input.checkpoint {
            CheckpointWrite::Baseline { checkpoint_json } => {
                checkpoint_store::write_baseline(&transaction, &conversation_id,
                    input.checkpoint_revision, input.checkpoint_event_cursor,
                    checkpoint_json, input.committed_at_ms)?;
            }
            CheckpointWrite::Replay {
                delivery_witness_json,
            } => {
                transaction.execute(
                "UPDATE core_checkpoints SET revision=?2,event_cursor=?3,updated_at_ms=?4 WHERE conversation_id=?1 AND revision=?5",
                params![conversation_id,input.checkpoint_revision,input.checkpoint_event_cursor,input.committed_at_ms,input.expected_checkpoint_revision],
            )?;
                transaction.execute(
                "UPDATE core_delivery_witnesses SET checkpoint_revision=?2,event_cursor=?3,witness_json=?4 WHERE conversation_id=?1",
                params![conversation_id,input.checkpoint_revision,input.checkpoint_event_cursor,delivery_witness_json],
            )?;
            }
        }
        apply_memory_usage_entries(&transaction, &input.memory_usage_entries)?;
        model_attempts::apply(&transaction, &input.run_id, &input.model_attempt_mutations)?;
        delegated_invocations::apply(
            &transaction,
            &input.run_id,
            &input.delegated_invocation_mutations,
        )?;
        delegated_permissions::apply(
            &transaction,
            &input.run_id,
            &input.delegated_permission_mutations,
        )?;
        effect_deliveries::apply_cancellation_admission(&transaction, &input.effect_delivery_attempt_mutations)?;
        permission_source::withdraw_policy_deliveries(&transaction, input)?;
        // Delivery fencing appends its own canonical facts after Core events.
        // The receipt must expose that committed frontier to the hot Run.
        let next_event_sequence: i64 = transaction.query_row(
            "SELECT next_event_sequence FROM runs WHERE id=?1", [&input.run_id], |row|row.get(0),
        )?;
        insert_outbox_effects(
            &transaction,
            &input.run_id,
            &input.outbox_effects,
            input.committed_at_ms,
        )?;
        for effect_id in &input.acknowledged_effect_ids {
            require_id("acknowledged_effect_id", effect_id)?;
            let updated = transaction.execute(
                "UPDATE effect_outbox
                 SET status = 'acknowledged', acknowledged_at_ms = ?3
                 WHERE effect_id = ?1 AND run_id = ?2 AND status = 'pending'",
                params![effect_id, input.run_id, input.committed_at_ms],
            )?;
            if updated != 1 {
                let already_acknowledged: bool = transaction.query_row(
                    "SELECT EXISTS(
                        SELECT 1 FROM effect_outbox
                        WHERE effect_id = ?1 AND run_id = ?2 AND status = 'acknowledged'
                    )",
                    params![effect_id, input.run_id],
                    |row| row.get(0),
                )?;
                if !already_acknowledged {
                    return Err(StoreError::Conflict(format!(
                        "effect {effect_id} does not belong to run {}",
                        input.run_id
                    )));
                }
            }
        }
        transaction.execute(
            "UPDATE conversations SET revision = revision + 1, updated_at_ms = ?2 WHERE id = ?1",
            params![conversation_id, input.committed_at_ms],
        )?;
        let conversation_revision: i64 = transaction.query_row(
            "SELECT revision FROM conversations WHERE id=?1",
            params![conversation_id],
            |row| row.get(0),
        )?;
        // Publish actual canonical changes, never the checkpoint, Host requests,
        // or an unrelated historical transcript. Every content mutation advances
        // the publication base so intermediate Parts cannot disappear at finish.
        conversation_transition_sync::enqueue(
            &transaction,
            &conversation_id,
            conversation_revision,
            input,
            replica_messages,
            replica_changes,
        )?;
        transaction.execute(
            "INSERT INTO committed_transitions(
                transition_id, run_id, checkpoint_revision, next_event_sequence, committed_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                input.transition_id,
                input.run_id,
                input.checkpoint_revision,
                next_event_sequence,
                input.committed_at_ms
            ],
        )?;
        if let Some((baseline_revision, through_ordinal, through_reduction_bytes)) = replay_base {
            // The receipt trigger binds full baseline writes. An incremental
            // commit retains its older baseline prefix under the same UoW.
            transaction.execute(
                "UPDATE core_checkpoint_replay_bases SET checkpoint_revision=?2,through_ordinal=?3,through_reduction_bytes=?5 WHERE conversation_id=?1 AND run_id=?4",
                params![conversation_id,baseline_revision,through_ordinal,input.run_id,through_reduction_bytes],
            )?;
        }
        let wait_response=crate::user_input_wait::response_from_reduction(input.submission_reduction_json.as_deref())?;
        for event in &input.events {
            if event.event_kind == "execution_evidence_observed" {
                crate::execution_evidence::index_event(&transaction, &event.event_id, input.host_event.as_ref())?;
            }
            crate::input_facts::index_event(&transaction, &event.event_id)?;
            crate::user_input_wait::index_event(&transaction, &event.event_id,wait_response.as_ref())?;
            crate::run_requirements::index_event(&transaction, &event.event_id, input.host_event.as_ref())?;
        }
        crate::user_input_wait::ensure_not_sealed_with_open_waits(&transaction,&input.run_id)?;
        crate::runtime_task_user_input_wait::synchronize(&transaction,&input.run_id,input.committed_at_ms)?;
        crate::run_message_admission::publish(&transaction, &conversation_id, &input.run_id)?;
        transaction.commit()?;

        if self.has_write_unit_of_work() {
            self.applied_transition_stamps
                .insert(input.transition_id.clone());
        }
        self.observe_committed_host_input(input.host_event.as_ref());
        Ok(CommitTransitionResult {
            applied: true,
            checkpoint_revision: input.checkpoint_revision,
            next_event_sequence,
        })
    }
}
