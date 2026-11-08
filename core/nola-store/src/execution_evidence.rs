// SPDX-License-Identifier: Apache-2.0
//! Derived evidence pointers validated against the exact Core and Host inputs.
use crate::{params, Connection, OptionalExtension, StoreError, StoreResult};
use nola_domain::core_event::CoreEventSource;
use nola_domain::execution_evidence::{ExecutionEvidenceObservation, ExecutionReceipt};
use nola_store_port::execution_evidence::{
    ExecutionEvidenceQuery, ExecutionEvidenceRepository, StoredExecutionEvidence,
};
use nola_store_port::run_effect::HostEventInput;
use serde_json::Value;
use sha2::{Digest, Sha256};

fn invalid() -> StoreError {
    StoreError::Invariant("execution evidence source differs from committed Core/Host facts".into())
}
fn digest(value: &Value) -> StoreResult<String> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(value)?)))
}

/// Promotion is a handoff, not a completed execution. Its TaskUpdated source is
/// already in this transaction; child Run/Task bindings may be published later.
fn validate_task_promotion(
    c: &Connection,
    run: &str,
    envelope: &CoreEventSource,
    evidence: &ExecutionEvidenceObservation,
    payload: &Value,
) -> StoreResult<()> {
    if payload["type"] != "task_promoted"
        || evidence.outcome != nola_domain::host_effect::HostToolOutcome::Deferred
        || evidence.receipt.is_some()
        || !payload["task_id"].as_str().is_some_and(|id| !id.trim().is_empty())
        || !matches!(payload["state"].as_str(), Some("queued" | "running" | "waiting" | "suspended" | "retry_waiting"))
        || !matches!(payload["kind"].as_str(), Some("agent" | "tool" | "skill" | "data_refresh"))
    {
        return Err(invalid());
    }
    let mut lineage = payload["lineage"].clone();
    if !lineage.is_object()
        || (!lineage["parent_call_id"].is_null() && lineage["parent_call_id"] != evidence.call_id)
    {
        return Err(invalid());
    }
    lineage["parent_call_id"] = Value::String(evidence.call_id.clone());
    let expected = serde_json::json!({
        "id": payload["task_id"], "owner_session_id": envelope.session_id,
        "owner_turn_id": envelope.turn_id, "kind": payload["kind"], "state": payload["state"],
        "progress": payload["progress"], "current_step": payload["current_step"],
        "result_ref": null, "cancel_requested": false, "cancel_requested_at_epoch_ms": null,
        "lineage": lineage,
        "durability": {"run_result":null,"result_digest":null,"terminal_receipt_id":null,
            "worker_id":null,"ownership_token":null,"lease_until_epoch_ms":null,
            "last_heartbeat_epoch_ms":null,"retry_count":0,"retry_at_epoch_ms":null,
            "last_error":null,"input_watermark":payload["input_watermark"],"completion_watermark":null}
    });
    let mut statement = c.prepare(
        "SELECT event_id,payload_json FROM run_events WHERE run_id=?1 AND event_kind='task_updated'
         AND json_extract(payload_json,'$.core_envelope.revision')=?2
         AND json_extract(payload_json,'$.core_envelope.cursor.sequence')<?3
         AND json_extract(payload_json,'$.task.id')=?4",
    )?;
    let rows = statement.query_map(params![run, envelope.revision, envelope.cursor.sequence, payload["task_id"].as_str()],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?;
    let mut matches = 0;
    for row in rows {
        let (event_id, raw) = row?;
        let event: Value = serde_json::from_str(&raw)?;
        let source: CoreEventSource = serde_json::from_value(event["core_envelope"].clone())?;
        if event_id != format!("core:{run}:{}", source.cursor.sequence)
            || event["type"] != "task_updated"
            || source.session_id != envelope.session_id
            || source.turn_id != envelope.turn_id
            || source.revision != envelope.revision
            || source.cursor.sequence == 0
            || source.cursor.sequence >= envelope.cursor.sequence
            || !event["task_projection_revision"].as_u64().is_some_and(|revision| revision > 0)
            || event["task"] != expected
        {
            return Err(invalid());
        }
        matches += 1;
    }
    if matches != 1 { return Err(invalid()); }
    Ok(())
}

pub(crate) fn read_source(
    c: &Connection,
    event_id: &str,
    host_id: &str,
    validate_dispatch: bool,
) -> StoreResult<(StoredExecutionEvidence, String, String)> {
    let (run,conversation,kind,json):(String,String,String,String)=c.query_row(
        "SELECT e.run_id,r.conversation_id,e.event_kind,e.payload_json FROM run_events e JOIN runs r ON r.id=e.run_id WHERE e.event_id=?1",
        [event_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?;
    let event: Value = serde_json::from_str(&json)?;
    if kind != "execution_evidence_observed" || event["type"] != kind {
        return Err(invalid());
    }
    let envelope: CoreEventSource =
        serde_json::from_value(event.get("core_envelope").cloned().ok_or_else(|| {
            StoreError::Invariant(
                "execution evidence is missing its committed Core envelope".into(),
            )
        })?)?;
    let turn = event["turn_id"].as_str().ok_or_else(invalid)?.to_owned();
    let evidence: ExecutionEvidenceObservation =
        serde_json::from_value(event.get("evidence").cloned().ok_or_else(invalid)?)?;
    evidence.validate().map_err(|_| invalid())?;
    if event_id != format!("core:{run}:{}", envelope.cursor.sequence)
        || envelope.turn_id.as_deref() != Some(&turn)
        || envelope.revision == 0
        || envelope.cursor.sequence == 0
        || i64::try_from(envelope.revision).is_err()
        || i64::try_from(envelope.cursor.sequence).is_err()
    {
        return Err(invalid());
    }
    let owned: bool = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM assistant_turn_message_bindings
        WHERE conversation_id=?1 AND run_id=?2 AND turn_id=?3)",
        params![conversation, run, turn],
        |r| r.get(0),
    )?;
    if !owned {
        return Err(invalid());
    }
    let (host_run,effect,request,sequence,reduction):(String,String,String,u64,String)=c.query_row(
        "SELECT run_id,effect_id,request_id,sequence,reduction_json FROM host_event_inbox WHERE event_id=?1",
        [host_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)))?;
    let reduction_value: Value = serde_json::from_str(&reduction)?;
    let facts: nola_store_port::host_replay::HostReplayReduction =
        serde_json::from_str(&reduction)?;
    let response: nola_domain::host_effect::HostResponseEnvelope<Value> =
        serde_json::from_str(facts.normalized_response().get())?;
    let source = &evidence.source;
    if host_run != run
        || source.effect_id != effect
        || source.request_id != request
        || source.response_sequence.0 != sequence
        || response.effect_id.0 != effect
        || response.request_id != request
        || response.sequence != sequence
        || response.session_id != envelope.session_id
        || response.turn_id.as_deref() != Some(&turn)
        || response.host_id.0 != source.host_id
        || response.attempt_id != source.attempt_id
        || response.catalog_revision != source.catalog_revision.0
        || response.availability_epoch != source.availability_epoch.0
    {
        return Err(invalid());
    }
    match response.payload["type"].as_str() {
        Some("task_promoted") => validate_task_promotion(c, &run, &envelope, &evidence, &response.payload)?,
        Some("tool_completed") => {
            if response.payload["call_id"] != evidence.call_id
                || response.payload["outcome"] != serde_json::to_value(evidence.outcome)? {
                return Err(invalid());
            }
            let output = &response.payload["output"];
            let receipt: Option<ExecutionReceipt> = match output.get("contract") {
                Some(Value::String(contract)) if contract == "nola.tool_result.v2" => {
                    // Explicit null means the Host supplied no receipt. Missing
                    // fields remain invalid; never synthesize a receipt here.
                    serde_json::from_value(output.get("execution").cloned().ok_or_else(invalid)?)?
                }
                Some(Value::String(contract)) if contract.starts_with("nola.tool_result.") => {
                    return Err(invalid());
                }
                // Unrelated business contracts do not grant typed receipts.
                _ => None,
            };
            if receipt != evidence.receipt { return Err(invalid()); }
        }
        _ => return Err(invalid()),
    }
    // Dispatch rows are pruned after acknowledgment. Verify their canonical
    // tool binding when creating the index, then retain that binding via the
    // immutable source digest; later reads never require recreating outbox work.
    if validate_dispatch {
        let outbox = crate::outbox_payload::read(c, &effect)?.ok_or_else(invalid)?;
        let requested: Value = serde_json::from_str(&outbox.payload_json)?;
        let payload = &requested["payload"];
        let tool_matches = match payload["type"].as_str() {
            Some("execute_tool") => payload["call_id"] == evidence.call_id
                && payload["capability_id"] == evidence.tool_name,
            Some("start_code_cell") => {
                let cell: nola_domain::code_mode::CodeCellIdentity =
                    serde_json::from_value(payload["identity"].clone())?;
                cell.validate().map_err(StoreError::Invariant)?;
                cell.parent_call_id == evidence.call_id
                    && evidence.tool_name == "code_mode_exec"
                    && cell.host_id == response.host_id.0
                    && cell.conversation_id == response.session_id
                    && response.turn_id.as_deref() == Some(cell.turn_id.as_str())
            }
            Some("observe_code_cell") | Some("read_code_cell_history") =>
                payload["call_id"] == evidence.call_id && evidence.tool_name == "code_mode_wait",
            _ => false,
        };
        if outbox.run_id != run
            || outbox.request_id != request
            || !tool_matches
        {
            return Err(StoreError::Invariant(
                "execution evidence differs from canonical dispatched tool".into(),
            ));
        }
    }
    // Delivery attempt identity is distinct from the optional model attempt ID.
    // The original response owns lease tokens; the derived index copies none.
    if let Some(delivery) = &response.delivery {
        let record:String=c.query_row("SELECT record_json FROM effect_delivery_attempts WHERE id=?1 AND run_id=?2 AND effect_id=?3 AND request_id=?4",
            params![delivery.attempt_id,run,effect,request],|r|r.get(0))?;
        let attempt: nola_domain::effect_delivery_attempt::EffectDeliveryAttempt =
            serde_json::from_str(&record)?;
        if attempt.worker_generation.as_deref() != Some(&delivery.worker_generation)
            || attempt.lease_owner.as_deref() != Some(&delivery.lease_owner)
            || attempt.lease_token.as_deref() != Some(&delivery.lease_token)
        {
            return Err(invalid());
        }
    }
    Ok((
        StoredExecutionEvidence {
            conversation_id: conversation,
            run_id: run,
            core_session_id: envelope.session_id,
            turn_id: turn,
            core_revision: envelope.revision,
            core_event_cursor: envelope.cursor.sequence,
            source_event_id: event_id.into(),
            host_event_id: host_id.into(),
            observation: evidence,
        },
        digest(&event)?,
        digest(&reduction_value)?,
    ))
}

pub(super) fn index_event(
    c: &Connection,
    event_id: &str,
    host: Option<&HostEventInput>,
) -> StoreResult<()> {
    if c.is_autocommit() {
        return Err(invalid());
    }
    let kind: String = c.query_row(
        "SELECT event_kind FROM run_events WHERE event_id=?1",
        [event_id],
        |r| r.get(0),
    )?;
    if kind != "execution_evidence_observed" {
        return Ok(());
    }
    let host = host.ok_or_else(invalid)?;
    // A lower-index batch response can flush earlier, durably buffered Host
    // responses. Each observation references its own original inbox entry,
    // not necessarily the Host input which caused this reduction to flush.
    // The current Host input must still belong to the same owning Run.
    let source_host: String = c.query_row(
        "SELECT h.event_id FROM run_events e
        JOIN host_event_inbox current ON current.event_id=?2 AND current.run_id=e.run_id
        JOIN host_event_inbox h ON h.run_id=e.run_id
          AND h.effect_id=json_extract(e.payload_json,'$.evidence.source.effect_id')
          AND h.request_id=json_extract(e.payload_json,'$.evidence.source.request_id')
          AND h.sequence=json_extract(e.payload_json,'$.evidence.source.response_sequence')
        WHERE e.event_id=?1",
        params![event_id, host.event_id],
        |r| r.get(0),
    ).optional()?.ok_or_else(|| StoreError::Invariant(
        "execution evidence has no matching original Host receipt".into()))?;
    let (stored, source_digest, host_digest) = read_source(c, event_id, &source_host, true)?;
    // Buffered batch members are separately reduced, advancing Core revision
    // each time. Their envelopes may precede the final committed witness's
    // revision even though all are committed in this one Store transaction.
    let current: bool = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM core_delivery_witnesses
        WHERE conversation_id=?1 AND json_extract(witness_json,'$.session_id')=?2
        AND json_extract(witness_json,'$.state.session_id')=?2 AND event_cursor>=?3
        AND json_extract(witness_json,'$.core_revision')>=?4)",
        params![
            stored.conversation_id,
            stored.core_session_id,
            stored.core_event_cursor,
            stored.core_revision
        ],
        |r| r.get(0),
    )?;
    if !current {
        return Err(invalid());
    }
    let old: Option<(String, String, String, String)> = c
        .query_row(
            "SELECT source_event_id,source_digest,host_event_id,host_reduction_digest
        FROM execution_evidence_sources WHERE run_id=?1 AND evidence_id=?2",
            params![stored.run_id, stored.observation.evidence_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?;
    if let Some(old) = old {
        return if old == (event_id.into(), source_digest, source_host, host_digest) {
            Ok(())
        } else {
            Err(invalid())
        };
    }
    c.execute(
        "INSERT INTO execution_evidence_sources VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
        params![
            stored.observation.evidence_id,
            stored.run_id,
            stored.conversation_id,
            stored.core_session_id,
            stored.turn_id,
            stored.observation.call_id,
            stored.core_revision,
            stored.core_event_cursor,
            event_id,
            source_digest,
            source_host,
            host_digest
        ],
    )?;
    Ok(())
}

impl ExecutionEvidenceRepository for crate::NolaStore {
    type Error = StoreError;
    fn load_execution_evidence(
        &self,
        query: &ExecutionEvidenceQuery,
    ) -> StoreResult<Option<StoredExecutionEvidence>> {
        for (name, value) in [
            ("evidence conversation", &query.conversation_id),
            ("evidence run", &query.run_id),
            ("evidence id", &query.evidence_id),
        ] {
            crate::require_id(name, value)?;
        }
        let _snapshot = crate::read_snapshot::pin(&self.connection)?;
        let row:Option<(String,String,String,String,String,String,String,u64,u64,String)>=self.connection.query_row(
            "SELECT source_event_id,source_digest,host_event_id,host_reduction_digest,conversation_id,core_session_id,turn_id,core_revision,core_cursor,call_id
             FROM execution_evidence_sources WHERE run_id=?1 AND evidence_id=?2",params![query.run_id,query.evidence_id],
            |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?,r.get(9)?))).optional()?;
        let Some((
            event,
            expected,
            host,
            expected_host,
            conversation,
            session,
            turn,
            revision,
            cursor,
            call,
        )) = row
        else {
            return Ok(None);
        };
        let (stored, actual, actual_host) = read_source(&self.connection, &event, &host, false)?;
        if actual != expected
            || actual_host != expected_host
            || conversation != query.conversation_id
            || stored.conversation_id != conversation
            || stored.run_id != query.run_id
            || stored.observation.evidence_id != query.evidence_id
            || stored.core_session_id != session
            || stored.turn_id != turn
            || stored.core_revision != revision
            || stored.core_event_cursor != cursor
            || stored.observation.call_id != call
        {
            return Err(invalid());
        }
        Ok(Some(stored))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_envelope_rejects_unknown_frontier_fields() {
        let exact = serde_json::json!({"session_id":"session","turn_id":"turn","revision":2,"cursor":{"sequence":3}});
        assert!(serde_json::from_value::<CoreEventSource>(exact.clone()).is_ok());
        let mut extra = exact.clone();
        extra["future_revision"] = serde_json::json!(7);
        assert!(serde_json::from_value::<CoreEventSource>(extra).is_err());
        let mut extra = exact;
        extra["cursor"]["ordinal"] = serde_json::json!(3);
        assert!(serde_json::from_value::<CoreEventSource>(extra).is_err());
    }

    #[test]
    fn evidence_event_without_current_host_input_is_rejected_and_rolled_back() {
        let mut store = crate::NolaStore::open(":memory:").unwrap();
        store
            .begin_turn(&crate::input_facts::tests::input(vec![]))
            .unwrap();
        store.begin_write_unit_of_work().unwrap();
        store.connection.execute("INSERT INTO run_events(event_id,run_id,sequence,event_kind,payload_json,created_at_ms)
            VALUES('forged','run',999,'execution_evidence_observed','{}',10)",[]).unwrap();
        assert!(index_event(&store.connection, "forged", None).is_err());
        store.rollback_write_unit_of_work().unwrap();
        let events: i64 = store
            .connection
            .query_row(
                "SELECT count(*) FROM run_events WHERE event_id='forged'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let evidence: i64 = store
            .connection
            .query_row("SELECT count(*) FROM execution_evidence_sources", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!((events, evidence), (0, 0));
    }
}
