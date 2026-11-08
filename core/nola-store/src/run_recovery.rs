// SPDX-License-Identifier: Apache-2.0
//! Same-transaction storage facts and application-planned recovery writes.
use super::*;
use nola_store_port::store_maintenance::{RecoveryRunRecord, RunRecoveryPlan, RunRecoveryRecord};

pub(super) fn with_plan<F>(
    store: &mut NolaStore,
    conversation: Option<&str>,
    reason: &str,
    at: &str,
    at_ms: i64,
    mut planner: F,
) -> StoreResult<usize>
where
    F: FnMut(&RunRecoveryRecord) -> Result<RunRecoveryPlan, String>,
{
    if let Some(id) = conversation {
        require_id("recovery.conversation_id", id)?;
    }
    require_id("recovery.reason", reason)?;
    require_id("recovery.at", at)?;
    if at_ms <= 0 {
        return Err(StoreError::Invariant(
            "recovery timestamp must be positive".into(),
        ));
    }
    let transaction = store.write_transaction()?;
    let mut after = String::new();
    let mut changed = 0;
    loop {
        // Keyset iteration retains at most one conversation's facts. A preserved
        // candidate still advances the key; no writes escape this transaction.
        let id: Option<String> = transaction
            .query_row(
                "SELECT conversation_id FROM (
                SELECT conversation_id FROM runs WHERE completed_at_ms IS NULL
                    AND status NOT IN ('sealed','interrupted')
                UNION SELECT conversation_id FROM trace_journal_runs WHERE finished_at IS NULL
             ) WHERE conversation_id>?1 AND (?2 IS NULL OR conversation_id=?2)
             ORDER BY conversation_id LIMIT 1",
                params![after, conversation],
                |r| r.get(0),
            )
            .optional()?;
        let Some(id) = id else {
            break;
        };
        after = id.clone();
        let record = read_record(&transaction, id)?;
        let plan = planner(&record).map_err(StoreError::Invariant)?;
        if let Some(lease) = &plan.release_session_lease {
            if record.session_lease.as_ref() != Some(lease) || lease.session_id != record.conversation_id {
                return Err(StoreError::Conflict("recovery session lease changed".into()));
            }
            let released = transaction.execute(
                "DELETE FROM session_run_leases WHERE session_id=?1 AND owner=?2 AND pid=?3 AND process_start=?4 AND acquired_at_ms=?5",
                params![lease.session_id, lease.owner, i64::from(lease.pid), lease.process_start, lease.acquired_at_ms],
            )?;
            if released != 1 { return Err(StoreError::Conflict("recovery session lease changed".into())); }
            changed += released;
        }
        if !plan.delegated_permission_mutations.is_empty() {
            let run = record.run.as_ref().ok_or_else(|| {
                StoreError::Invariant("permission recovery requires a Run".into())
            })?;
            crate::delegated_permissions::apply(
                &transaction,
                &run.run_id,
                &plan.delegated_permission_mutations,
            )?;
            crate::delegated_permissions::append_initial_events(
                &transaction,
                &run.run_id,
                &plan.delegated_permission_mutations,
                at_ms,
            )?;
            changed += plan.delegated_permission_mutations.len();
        }
        if !plan.effect_delivery_attempt_mutations.is_empty() {
            crate::effect_deliveries::apply_recovery(&transaction, &plan.effect_delivery_attempt_mutations)?;
            changed += plan.effect_delivery_attempt_mutations.len();
        }
        if !plan.preserve_all_traces {
            for trace in &record.traces {
                if plan.preserve_trace_turn_id.as_deref() == Some(trace.turn_id.as_str()) {
                    continue;
                }
                changed += transaction.execute(
                    "UPDATE trace_journal_runs SET status='interrupted',
                    terminal_reason=COALESCE(terminal_reason,?2),updated_at=?3,finished_at=?3
                    WHERE trace_id=?1 AND finished_at IS NULL",
                    params![trace.trace_id, reason, at],
                )?;
            }
        }
    }
    transaction.commit()?;
    Ok(changed)
}

fn read_record(connection: &Connection, conversation_id: String) -> StoreResult<RunRecoveryRecord> {
    let session_lease = connection.query_row(
        "SELECT session_id,owner,pid,process_start,acquired_at_ms FROM session_run_leases WHERE session_id=?1",
        params![conversation_id],
        session_run_lease_row,
    ).optional()?;
    let has_session_lease = session_lease.is_some();
    let mut statement = connection.prepare("SELECT r.id,r.status,p.config_json,r.invocation_config_digest,
        (SELECT CASE WHEN json_valid(payload_json) THEN CASE WHEN json_type(payload_json,'$.turn_id')='text' THEN json_extract(payload_json,'$.turn_id') ELSE '' END ELSE '' END FROM run_events WHERE run_id=r.id AND event_kind='turn_started' ORDER BY sequence DESC LIMIT 1), r.active_attempt_id
        FROM runs r LEFT JOIN run_invocation_payloads p ON p.run_id=r.id
        WHERE r.conversation_id=?1 AND r.completed_at_ms IS NULL
        AND r.status NOT IN ('sealed','interrupted') ORDER BY r.created_at_ms,r.id LIMIT 2")?;
    let mut rows = statement.query(params![conversation_id])?;
    let run = if let Some(row) = rows.next()? {
        Some(RecoveryRunRecord {
            run_id: row.get(0)?,
            active_attempt_id: row.get(5)?,
            status: row.get(1)?,
            invocation_config_json: row.get(2)?,
            invocation_config_digest: row.get(3)?,
            latest_started_turn_id: row.get(4)?,
        })
    } else {
        None
    };
    if rows.next()?.is_some() {
        return Err(StoreError::Invariant(
            "multiple active recovery Runs for conversation".into(),
        ));
    }
    let checkpoint = crate::run_effect_port::read_recovery_checkpoint(connection, &conversation_id)?;
    let effects = if let Some(run) = &run {
        let mut effects=crate::outbox_payload::pending(connection,&run.run_id)?;
        effects.sort_by(|a,b| (a.created_at_ms,&a.effect_id).cmp(&(b.created_at_ms,&b.effect_id)));
        effects
    } else {
        vec![]
    };
    let mut q=connection.prepare(&format!("SELECT {TRACE_JOURNAL_RUN_COLUMNS} FROM trace_journal_runs WHERE conversation_id=?1 AND finished_at IS NULL ORDER BY started_at,trace_id"))?;
    let traces = q
        .query_map(params![conversation_id], trace_journal_run_row)?
        .collect::<Result<Vec<_>, _>>()?;
    let mut model_attempts = Vec::new();
    for effect in &effects {
        if let Some(attempt) = crate::model_attempts::tail(connection, &effect.effect_id)? {
            model_attempts.push(attempt);
        }
    }
    let delegated_invocations = run
        .as_ref()
        .map(|r| crate::delegated_invocations::recovery_records(connection, &r.run_id))
        .transpose()?
        .unwrap_or_default();
    let delegated_permissions = run
        .as_ref()
        .map(|r| crate::delegated_permissions::recovery_records(connection, &r.run_id))
        .transpose()?
        .unwrap_or_default();
    let effect_delivery_attempts = run
        .as_ref()
        .map(|run| crate::effect_deliveries::recovery_records(connection, &run.run_id))
        .transpose()?
        .unwrap_or_default();
    let open_user_input_waits = run.as_ref()
        .map(|run| crate::user_input_wait::open_waits(connection, &conversation_id, Some(&run.run_id)))
        .transpose()?.unwrap_or_default();
    Ok(RunRecoveryRecord {
        open_user_input_waits,
        conversation_id,
        has_session_lease,
        session_lease,
        run,
        checkpoint,
        effects,
        model_attempts,
        delegated_permissions,
        effect_delivery_attempts,
        delegated_invocations,
        traces,
    })
}
