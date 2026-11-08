// SPDX-License-Identifier: Apache-2.0
//! Capture exact permission provenance inside the suspension transaction.
use super::*;
use nola_domain::permission_closure::{CorePermissionOccurrence, CorePermissionSourceProof};
use nola_domain::assistant_journal::AssistantDigest;
use nola_domain::assistant_journal_operations::JournalNumber;
use nola_domain::host_effect::{HostRequest, HostRequestEnvelope};

/// Validate settlement against the previous committed permission, before the
/// transition replaces the witness or acknowledges its outbox effect.
pub(super) fn validate_settlements(
    connection: &Connection,
    input: &nola_store_port::run_effect::CommitTransitionInput,
) -> StoreResult<()> {
    let applied_policy = validate_policy_submission(connection, input)?;
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Settlement {
        #[serde(rename = "type")]
        kind: String,
        receipt: nola_domain::permission_settlement::PermissionReceipt,
        core_envelope: nola_domain::core_event::CoreEventSource,
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut revision_window = None;
    for event in input.events.iter().filter(|event| event.event_kind == "permission_settled") {
        // Store checkpoint revisions count durable commits; Core revisions also
        // count streamed reductions between commits. Compare the event with
        // its Core frontier, never with the Store's independent CAS counter.
        let (previous_revision, next_revision) = match revision_window {
            Some(window) => window,
            None => {
                let previous: u64 = connection.query_row(
                    "SELECT json_extract(w.witness_json,'$.core_revision')
                     FROM runs r JOIN core_checkpoints c ON c.conversation_id=r.conversation_id
                     JOIN core_delivery_witnesses w ON w.conversation_id=c.conversation_id
                       AND w.checkpoint_revision=c.revision AND w.event_cursor=c.event_cursor
                     WHERE r.id=?1 AND c.revision=?2",
                    params![input.run_id, input.expected_checkpoint_revision], |row| row.get(0),
                )?;
                let (raw, field) = match &input.checkpoint {
                    nola_store_port::run_effect::CheckpointWrite::Baseline { checkpoint_json } => (checkpoint_json, "revision"),
                    nola_store_port::run_effect::CheckpointWrite::Replay { delivery_witness_json } => (delivery_witness_json, "core_revision"),
                };
                let frontier: serde_json::Value = serde_json::from_str(raw)?;
                let next = frontier[field].as_u64().ok_or_else(||
                    StoreError::Conflict("permission settlement has no Core revision frontier".into()))?;
                revision_window = Some((previous, next));
                (previous, next)
            }
        };
        let fact: Settlement = serde_json::from_str(&event.payload_json)?;
        let receipt = &fact.receipt;
        if let nola_domain::permission_settlement::PermissionReceiptOutcome::PolicyApplied { policy } = &receipt.outcome {
            if applied_policy.as_ref() != Some(policy) {
                return Err(StoreError::Conflict("permission policy settlement has no matching committed policy source".into()));
            }
        }
        let source = &fact.core_envelope;
        if fact.kind != "permission_settled"
            || !seen.insert(receipt.request_id.clone())
            || source.turn_id.as_deref() != Some(receipt.turn_id.as_str())
            || source.revision <= previous_revision || source.revision > next_revision
            || source.cursor.sequence == 0
            || i64::try_from(source.cursor.sequence).ok().is_none_or(|cursor|
                cursor > input.checkpoint_event_cursor)
        {
            return Err(StoreError::Conflict(format!(
                "invalid permission settlement frontier: kind={}; source_turn={:?}; receipt_turn={}; source_revision={}; previous_core_revision={}; next_core_revision={}; source_cursor={}; checkpoint_cursor={}",
                fact.kind, source.turn_id, receipt.turn_id, source.revision,
                previous_revision, next_revision, source.cursor.sequence, input.checkpoint_event_cursor,
            )));
        }
        let effects: Vec<String> = connection.prepare(
            "SELECT e.effect_id FROM runs r
             JOIN core_checkpoints cp ON cp.conversation_id=r.conversation_id
             JOIN core_delivery_witnesses w ON w.conversation_id=cp.conversation_id
               AND w.checkpoint_revision=cp.revision AND w.event_cursor=cp.event_cursor
             JOIN json_each(w.witness_json,'$.state.host.pending_host_effects.effects') p
             JOIN json_each(w.witness_json,'$.state.execution.permissions.pending') q
             JOIN effect_outbox e ON e.effect_id=p.key AND e.run_id=r.id
             WHERE r.id=?1 AND cp.revision=?2 AND e.status='pending'
               AND json_extract(w.witness_json,'$.session_id')=?3
               AND json_extract(p.value,'$.permission_request_id')=?4
               AND json_extract(p.value,'$.request_id')=e.request_id
               AND json_extract(q.value,'$.request_id')=?4
               AND json_extract(q.value,'$.turn_id')=?5
               AND json_extract(q.value,'$.call_id')=?6
               AND cp.event_cursor<?7"
        )?.query_map(params![input.run_id,input.expected_checkpoint_revision,source.session_id,
            receipt.request_id,receipt.turn_id,receipt.call_id,source.cursor.sequence],
            |row| row.get(0))?.collect::<Result<_, _>>()?;
        if effects.len() != 1 {
            return Err(StoreError::Conflict("permission settlement lacks its exact original effect".into()));
        }
        let effect = crate::outbox_payload::read(connection, &effects[0])?
            .ok_or_else(|| StoreError::Conflict("permission settlement source disappeared".into()))?;
        let envelope: HostRequestEnvelope<HostRequest> = serde_json::from_str(&effect.payload_json)?;
        let HostRequest::RequestPermission { permission_request_id, capability_id, resource, .. } = envelope.payload else {
            return Err(StoreError::Conflict("settlement source is not a permission request".into()));
        };
        if envelope.session_id != source.session_id
            || envelope.turn_id.as_deref() != Some(receipt.turn_id.as_str())
            || permission_request_id != receipt.request_id
            || capability_id != receipt.capability_id || resource != receipt.resource
        {
            return Err(StoreError::Conflict("permission settlement differs from its original subject".into()));
        }
        close_settled_waits(connection, input, event, receipt, &effects[0])?;
    }
    Ok(())
}

fn validate_policy_submission(
    connection: &Connection,
    input: &nola_store_port::run_effect::CommitTransitionInput,
) -> StoreResult<Option<nola_domain::ActivePermissionPolicy>> {
    let Some(raw) = &input.submission_reduction_json else { return Ok(None); };
    let reduction: nola_store_port::run_effect::SubmissionReplayReduction = serde_json::from_str(raw)?;
    let submission: serde_json::Value = serde_json::from_str(&reduction.submission_json)?;
    if submission["type"] != "apply_permission_policy" { return Ok(None); }
    let policy: nola_domain::ActivePermissionPolicy = serde_json::from_value(submission["policy"].clone())?;
    policy.validate().map_err(StoreError::Invariant)?;
    if policy.run_id != input.run_id || input.transition_id != policy.command_id() {
        return Err(StoreError::Conflict("permission policy changed Run or command identity".into()));
    }
    let payload: Option<String> = connection.query_row(
        "SELECT q.payload_json FROM agent_commands q JOIN runs r ON r.conversation_id=q.session_id
         WHERE r.id=?1 AND q.command_id=?2 AND q.lease_owner IS NOT NULL AND q.lease_until_ms IS NOT NULL",
        params![input.run_id, policy.command_id()], |row| row.get(0),
    ).optional()?;
    let payload: serde_json::Value = serde_json::from_str(&payload.ok_or_else(||
        StoreError::Conflict("permission policy has no source-owned claimed notification".into()))?)?;
    if payload != serde_json::json!({"type":"permission_policy_available", "policy":policy}) {
        return Err(StoreError::Conflict("permission policy differs from configuration transaction notification".into()));
    }
    Ok(Some(policy))
}

/// Fence old leased deliveries before acknowledging their removed outbox rows.
/// ExecutionStarted and policy application serialize on the same writer.
pub(super) fn withdraw_policy_deliveries(
    connection: &Connection, input: &nola_store_port::run_effect::CommitTransitionInput,
) -> StoreResult<()> {
    if validate_policy_submission(connection, input)?.is_none() { return Ok(()); }
    for effect_id in &input.acknowledged_effect_ids {
        let effect = crate::outbox_payload::read(connection, effect_id)?
            .ok_or_else(||StoreError::Conflict("withdrawn permission effect disappeared".into()))?;
        let request: HostRequestEnvelope<HostRequest> = serde_json::from_str(&effect.payload_json)?;
        if matches!(request.payload, HostRequest::ExecuteTool { .. }) {
            crate::effect_deliveries::withdraw_for_permission_policy(connection, effect_id, input.committed_at_ms)?;
        }
    }
    Ok(())
}

fn close_settled_waits(
    connection: &Connection,
    input: &nola_store_port::run_effect::CommitTransitionInput,
    event: &nola_store_port::run_effect::RunEventInput,
    receipt: &nola_domain::permission_settlement::PermissionReceipt,
    effect_id: &str,
) -> StoreResult<()> {
    use nola_domain::permission_closure::{PermissionClosureSource, PermissionRequestClosure, PermissionClosureRequest};
    let requests = connection.prepare(&format!(
        "SELECT {PERMISSION_REQUEST_COLUMNS} FROM permission_requests
         WHERE core_permission_request_id=?1 AND task_id IN
           (SELECT task_id FROM runtime_task_run_bindings WHERE run_id=?2)"
    ))?.query_map(params![receipt.request_id,input.run_id], permission_request_row)?
        .collect::<Result<Vec<_>, _>>()?;
    for request in requests {
        if request.closure.is_some() {
            return Err(StoreError::Conflict("settlement cannot reopen a closed permission".into()));
        }
        let proof_json: String = connection.query_row(
            "SELECT proof_json FROM permission_request_sources WHERE request_id=?1",
            [&request.request_id], |row| row.get(0))?;
        let proof: CorePermissionSourceProof = serde_json::from_str(&proof_json)?;
        let current = capture(connection, &input.run_id, &request)?;
        if proof.permission.run_id != input.run_id || proof.permission.effect_id != effect_id
            || proof.permission.core_permission_request_id != receipt.request_id
            || request.capability != receipt.capability_id
            || proof.permission != current.permission
            || proof.host_request_id != current.host_request_id
            || proof.payload_digest != current.payload_digest
            || proof.checkpoint_revision > current.checkpoint_revision
            || proof.event_cursor > current.event_cursor
        {
            return Err(StoreError::Conflict("settlement differs from retained permission provenance".into()));
        }
        let closure = PermissionRequestClosure {
            closure_id: format!("permission-settled:{}:{}",request.request_id,event.event_id),
            request_id: request.request_id.clone(), task_id: request.task_id.clone(),
            child_session_id: request.child_session_id.clone(), closed_at_ms: input.committed_at_ms,
            source: PermissionClosureSource::CoreSettled {
                permission: proof.permission, event_id: event.event_id.clone(),
                event_cursor: JournalNumber(u64::try_from(event.sequence)
                    .map_err(|_| StoreError::Invariant("invalid settlement event sequence".into()))?),
            },
        };
        closure.validate_for(&PermissionClosureRequest {
            request_id: &request.request_id, task_id: &request.task_id,
            child_session_id: &request.child_session_id,
            core_permission_request_id: request.core_permission_request_id.as_deref(),
            created_at_ms: request.created_at_ms, resolved_at_ms: request.resolved_at_ms,
        }).map_err(|error| StoreError::Invariant(error.to_string()))?;
        if connection.execute("UPDATE permission_requests SET status='withdrawn',closure_json=?2
            WHERE request_id=?1 AND closure_json IS NULL",
            params![request.request_id,serde_json::to_string(&closure)?])? != 1 {
            return Err(StoreError::Conflict("settlement lost its permission request".into()));
        }
        // Stop replaying the old permission response. A running successor
        // keeps its current lease; an unclaimed suspension re-enters ordinary
        // fair scheduling of the SAME bound Run, without creating a decision.
        let pending: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM runtime_task_permission_suspensions WHERE permission_request_id=?1 AND state='pending')",
            [&request.request_id], |row| row.get(0))?;
        connection.execute("UPDATE runtime_task_permission_suspensions SET state='settled',revision=revision+1
            WHERE permission_request_id=?1 AND state IN ('pending','resumed')", [&request.request_id])?;
        if pending {
            connection.execute("UPDATE durable_tasks SET status='queued',updated_at_ms=?2
                WHERE id=?1 AND status='waiting_permission' AND cancel_requested=0
                  AND ownership_token IS NULL", params![request.task_id,input.committed_at_ms])?;
        }
    }
    Ok(())
}

/// A permission owns its provenance, independently of whether its worker suspends.
/// Only Store captures this value; public request commands cannot supply it.
pub(super) fn retain_bound_source(
    connection: &Connection, request: &StoredPermissionRequest,
) -> StoreResult<Option<CorePermissionSourceProof>> {
    if connection.is_autocommit() {
        return Err(StoreError::Invariant("permission source retention requires a write transaction".into()));
    }
    let binding: Option<(String, String)> = connection.query_row(
        "SELECT run_id,child_session_id FROM runtime_task_run_bindings WHERE task_id=?1",
        [&request.task_id], |row| Ok((row.get(0)?,row.get(1)?)),
    ).optional()?;
    let Some((run_id, child)) = binding else { return Ok(None); };
    if child != request.child_session_id {
        return Err(StoreError::Conflict("permission child differs from canonical Task Run binding".into()));
    }
    let proof = capture(connection, &run_id, request)?;
    let existing: Option<String> = connection.query_row(
        "SELECT proof_json FROM permission_request_sources WHERE request_id=?1",
        [&request.request_id], |row| row.get(0),
    ).optional()?;
    if let Some(existing) = existing {
        let existing: CorePermissionSourceProof = serde_json::from_str(&existing)?;
        if existing.permission != proof.permission || existing.host_request_id != proof.host_request_id
            || existing.payload_digest != proof.payload_digest
            || existing.checkpoint_revision > proof.checkpoint_revision || existing.event_cursor > proof.event_cursor {
            return Err(StoreError::Conflict("permission source changed after admission".into()));
        }
        return Ok(Some(existing));
    }
    connection.execute("INSERT INTO permission_request_sources(request_id,proof_json) VALUES(?1,?2)",
        params![request.request_id,serde_json::to_string(&proof)?])?;
    Ok(Some(proof))
}

pub(super) fn close_from_unbound_terminal(
    connection: &Connection,
    receipt: &RuntimeTaskTerminalReceipt,
) -> StoreResult<()> {
    use nola_domain::permission_closure::{PermissionRequestClosure, PermissionClosureSource, PermissionClosureRequest};
    if connection.is_autocommit() || !matches!(receipt.status.as_str(), "completed" | "failed" | "cancelled") || receipt.journal_result.is_some() {
        return Err(StoreError::Invariant("unbound closure requires the terminal Task receipt transaction".into()));
    }
    let retained: Option<String> = connection.query_row(
        "SELECT r.receipt_json FROM runtime_task_terminal_receipts r
         JOIN durable_tasks t ON t.id=r.task_id
         WHERE r.task_id=?1 AND r.receipt_id=?2 AND t.status=?4
           AND t.ended_at_ms=?3
           AND NOT EXISTS(SELECT 1 FROM runtime_task_run_bindings b WHERE b.task_id=t.id)",
        params![receipt.task_id,receipt.receipt_id,receipt.ended_at_ms,receipt.status], |row| row.get(0),
    ).optional()?;
    let retained: RuntimeTaskTerminalReceipt = serde_json::from_str(&retained
        .ok_or_else(|| StoreError::Conflict("unbound permission closure lacks exact terminal Task authority".into()))?)?;
    if retained != *receipt || matches!(receipt.source, RuntimeTaskTerminalSource::BoundRunTerminal { .. }) {
        return Err(StoreError::Conflict("unbound permission closure receipt differs".into()));
    }
    let requests = connection.prepare(&format!(
        "SELECT {PERMISSION_REQUEST_COLUMNS} FROM permission_requests WHERE task_id=?1 ORDER BY request_id"
    ))?.query_map([&receipt.task_id], permission_request_row)?.collect::<Result<Vec<_>, _>>()?;
    for request in requests {
        if request.closure.is_some() { continue; }
        if receipt.child_session_id.as_deref().is_some_and(|child| child != request.child_session_id) {
            return Err(StoreError::Conflict("unbound permission child differs from its Task receipt".into()));
        }
        let closure = PermissionRequestClosure {
            closure_id: format!("permission-close:{}:{}",request.request_id,receipt.receipt_id),
            request_id: request.request_id.clone(), task_id: receipt.task_id.clone(),
            child_session_id: request.child_session_id.clone(), closed_at_ms: receipt.recorded_at_ms,
            source: PermissionClosureSource::UnboundTaskTerminal { terminal_receipt_id: receipt.receipt_id.clone() },
        };
        closure.validate_for(&PermissionClosureRequest {
            request_id: &request.request_id, task_id: &request.task_id,
            child_session_id: &request.child_session_id,
            core_permission_request_id: request.core_permission_request_id.as_deref(),
            created_at_ms: request.created_at_ms, resolved_at_ms: request.resolved_at_ms,
        }).map_err(|error| StoreError::Invariant(error.to_string()))?;
        if connection.execute("UPDATE permission_requests SET status='withdrawn',closure_json=?2
            WHERE request_id=?1 AND closure_json IS NULL",
            params![request.request_id,serde_json::to_string(&closure)?])? != 1 {
            return Err(StoreError::Conflict("unbound permission closure lost its request".into()));
        }
    }
    crate::permission_transport::revoke_task_request_grants(connection, &receipt.task_id, receipt.recorded_at_ms)?;
    Ok(())
}

/// Called only by canonical Task/Run reconciliation after validating the seal
/// against its durable Run and manifest, inside that same owned transaction.
pub(super) fn close_from_run(
    connection: &Connection,
    task_id: &str,
    seal: &nola_domain::assistant_run_terminal::AssistantRunTerminalSeal,
    at: i64,
) -> StoreResult<()> {
    use nola_domain::permission_closure::{PermissionRequestClosure, PermissionClosureSource, PermissionClosureRequest};
    if connection.is_autocommit() {
        return Err(StoreError::Invariant("permission closure requires the Run reconciliation transaction".into()));
    }
    let requests = connection.prepare(&format!(
        "SELECT {PERMISSION_REQUEST_COLUMNS} FROM permission_requests WHERE task_id=?1 ORDER BY request_id"
    ))?.query_map([task_id], permission_request_row)?.collect::<Result<Vec<_>, _>>()?;
    for request in requests {
        if request.closure.is_some() { continue; }
        let source: String = connection.query_row(
            "SELECT proof_json FROM permission_request_sources WHERE request_id=?1",
            params![request.request_id], |row| row.get(0),
        ).optional()?.ok_or_else(|| StoreError::Invariant("permission closure lacks captured source".into()))?;
        let proof: CorePermissionSourceProof = serde_json::from_str(&source)?;
        if proof.permission.run_id != seal.identity.run_id
            || request.child_session_id != seal.result.conversation_id
            || proof.checkpoint_revision > seal.checkpoint_revision
            || proof.event_cursor > seal.event_cursor
            || proof.permission.effect_id.trim().is_empty() || proof.host_request_id.trim().is_empty()
        {
            return Err(StoreError::Conflict("permission closure source differs from canonical Run seal".into()));
        }
        let closure = PermissionRequestClosure {
            closure_id: format!("permission-close:{}:{}",request.request_id,seal.seal_digest.to_hex()),
            request_id: request.request_id.clone(), task_id: request.task_id.clone(),
            child_session_id: request.child_session_id.clone(), closed_at_ms: at,
            source: PermissionClosureSource::RunSealed {
                permission: proof.permission, assistant_message_id: seal.identity.assistant_message_id.clone(),
                seal_digest: seal.seal_digest,
            },
        };
        closure.validate_for(&PermissionClosureRequest {
            request_id: &request.request_id, task_id: &request.task_id,
            child_session_id: &request.child_session_id,
            core_permission_request_id: request.core_permission_request_id.as_deref(),
            created_at_ms: request.created_at_ms, resolved_at_ms: request.resolved_at_ms,
        }).map_err(|error| StoreError::Invariant(error.to_string()))?;
        // User decision, instruction and its timestamp remain untouched.
        if connection.execute("UPDATE permission_requests SET status='withdrawn',closure_json=?2
            WHERE request_id=?1 AND closure_json IS NULL",params![request.request_id,serde_json::to_string(&closure)?])? != 1 {
            return Err(StoreError::Conflict("permission closure lost its request".into()));
        }
    }
    Ok(())
}

pub(super) fn capture(
    connection: &Connection,
    run_id: &str,
    request: &StoredPermissionRequest,
) -> StoreResult<CorePermissionSourceProof> {
    if connection.is_autocommit() {
        return Err(StoreError::Invariant("permission provenance requires a write transaction".into()));
    }
    let core_id = request.core_permission_request_id.as_deref()
        .filter(|id| !id.trim().is_empty())
        .ok_or_else(|| StoreError::Invariant("bound permission requires its original Core request".into()))?;
    // The compact witness must be for the current committed checkpoint. Match
    // the occurrence there first, not just a caller-controlled request ID.
    let rows: Vec<(String, String, String, i64, i64)> = connection.prepare(
        "SELECT e.effect_id,json_extract(body.payload_json,'$.request_id'),e.payload_digest,cp.revision,cp.event_cursor
         FROM runs r JOIN core_checkpoints cp ON cp.conversation_id=r.conversation_id
         JOIN core_delivery_witnesses w ON w.conversation_id=cp.conversation_id
           AND w.checkpoint_revision=cp.revision AND w.event_cursor=cp.event_cursor
         JOIN json_each(w.witness_json,'$.state.host.pending_host_effects.effects') p
         JOIN effect_outbox e ON e.effect_id=p.key AND e.run_id=r.id
         JOIN effect_outbox_payloads body ON body.effect_id=e.effect_id
         WHERE r.id=?1 AND r.conversation_id=?2 AND e.status='pending'
           AND json_extract(p.value,'$.permission_request_id')=?3
           AND json_extract(p.value,'$.request_id')=e.request_id"
    )?.query_map(params![run_id,request.child_session_id,core_id], |row|
        Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?)))?
        .collect::<Result<_, _>>()?;
    if rows.len() != 1 {
        return Err(StoreError::Conflict("permission has no unique committed Core effect".into()));
    }
    let (effect_id, host_request_id, digest, revision, cursor) = &rows[0];
    // Provenance binds the immutable declaration's request ID. A legitimate
    // delivery rebind changes e.request_id, not that declaration or its digest.
    // The canonical reader still verifies those bytes and the current binding.
    let effect = crate::outbox_payload::read(connection, effect_id)?
        .ok_or_else(|| StoreError::Conflict("permission effect disappeared".into()))?;
    let envelope: HostRequestEnvelope<HostRequest> = serde_json::from_str(&effect.payload_json)?;
    let HostRequest::RequestPermission { permission_request_id, capability_id, input,
        permission_instructions, .. } = envelope.payload else {
        return Err(StoreError::Conflict("permission source is not a permission effect".into()));
    };
    let instructions: Vec<nola_domain::PermissionInstruction> =
        serde_json::from_str(&request.permission_instructions_json)?;
    let expected_input: serde_json::Value = serde_json::from_str(&request.input_json)?;
    if effect.run_id != run_id || envelope.session_id != request.child_session_id
        || permission_request_id != core_id || capability_id != request.capability
        || input != expected_input || permission_instructions != instructions
    {
        return Err(StoreError::Conflict("permission input differs from committed Core effect".into()));
    }
    Ok(CorePermissionSourceProof {
        permission: CorePermissionOccurrence { run_id: run_id.into(),
            core_permission_request_id: core_id.into(), effect_id: effect_id.clone() },
        host_request_id: host_request_id.clone(),
        payload_digest: AssistantDigest::parse_hex(digest)
            .map_err(|error| StoreError::Invariant(error.to_string()))?,
        checkpoint_revision: JournalNumber(u64::try_from(*revision)
            .map_err(|_| StoreError::Invariant("negative permission checkpoint revision".into()))?),
        event_cursor: JournalNumber(u64::try_from(*cursor)
            .map_err(|_| StoreError::Invariant("negative permission event cursor".into()))?),
    })
}

#[cfg(test)]
#[path = "permission_source_frontier_tests.rs"]
mod frontier_tests;
