// SPDX-License-Identifier: Apache-2.0
//! Durable, fenced Host-effect delivery attempts.
use super::*;
use nola_domain::{
    EffectDeliveryAttempt, EffectDeliveryAttemptMutation, EffectDeliveryAttemptState,
};
use nola_store_port::effect_delivery::{
    EffectDeliveryCandidate, EffectDeliveryCandidateCursor, EffectDeliveryRepository,
    InterruptedEffectDeliveryCursor, RuntimeEffectWorkReason, RuntimeEffectWorkRunHead,
    RuntimeEffectWorkSchedule,
};
use std::collections::BTreeSet;

const MAX_EFFECT_DELIVERY_BATCH: usize = 64;
fn validate_user_retry_source(connection: &Connection, current: &EffectDeliveryAttempt, record: &EffectDeliveryAttempt) -> StoreResult<()> {
        let Some(nola_domain::EffectDeliveryManualResolution::RiskRetryAuthorized { evidence, .. }) = &record.manual_resolution else {
            return Err(StoreError::Invariant("retry authorization lacks explicit user decision".into()));
        };
        let decision:nola_domain::unknown_effect_recovery::UnknownEffectRecoveryDecision=serde_json::from_str(evidence)?;
        let original:Option<String>=connection.query_row("SELECT json_extract(e.payload_json,'$.fact.kind.decision') FROM run_requirement_sources s JOIN run_events e ON e.event_id=s.source_event_id
            WHERE s.run_id=?1 AND s.kind='runtime_decision_fact_observed' AND s.source_transition_id=?2
            AND json_extract(e.payload_json,'$.fact.kind.kind')='unknown_effect_recovery'",
            params![record.run_id,decision.resolution_id],|row|row.get(0)).optional()?;
        let original=original.map(|raw|serde_json::from_str::<nola_domain::unknown_effect_recovery::UnknownEffectRecoveryDecision>(&raw)).transpose()?;
        if original.as_ref()!=Some(&decision) || decision.disposition!=nola_domain::unknown_effect_recovery::UnknownEffectRecoveryDisposition::AuthorizeRiskRetry
            || !decision.targets.iter().any(|target|target.attempt_id==current.id && target.expected_attempt_revision==current.revision) {
            return Err(StoreError::Invariant("retry authorization lacks original Core user recovery decision".into()));
        }
    Ok(())
}
fn validate_permission_redisplay_source(connection: &Connection, current: &EffectDeliveryAttempt, record: &EffectDeliveryAttempt) -> StoreResult<()> {
    let Some(nola_domain::EffectDeliveryManualResolution::PendingPermissionRedisplay { permission_request_id, resolved_at_ms }) = &record.manual_resolution else {
        return Err(StoreError::Invariant("missing Core permission redisplay proof".into()));
    };
    if current.state != EffectDeliveryAttemptState::ExecutionStarted
        || current.lease_expires_at_ms.is_none_or(|expiry| expiry > *resolved_at_ms)
    { return Err(StoreError::Conflict("permission delivery is not expired".into())); }
    let source: Option<String> = connection.query_row(
        "SELECT w.witness_json FROM effect_outbox e
         JOIN runs r ON r.id=e.run_id JOIN core_checkpoints c ON c.conversation_id=r.conversation_id
         JOIN core_delivery_witnesses w ON w.conversation_id=c.conversation_id AND w.checkpoint_revision=c.revision AND w.event_cursor=c.event_cursor
         WHERE e.effect_id=?1 AND e.run_id=?2 AND e.request_id=?3 AND e.status='pending'
           AND r.completed_at_ms IS NULL AND r.status NOT IN ('sealed','interrupted')",
        params![current.effect_id,current.run_id,current.request_id], |row| row.get(0)).optional()?;
    let witness = source.ok_or_else(|| StoreError::Invariant("permission redisplay lost current Run/outbox/checkpoint".into()))?;
    let original = crate::outbox_payload::read(connection, &current.effect_id)?
        .ok_or_else(|| StoreError::Invariant("permission redisplay lost immutable request".into()))?;
    let request: serde_json::Value = serde_json::from_str(&original.payload_json)?;
    let witness: serde_json::Value = serde_json::from_str(&witness)?;
    let effect = &witness["state"]["host"]["pending_host_effects"]["effects"][&current.effect_id];
    let pending = witness["state"]["execution"]["permissions"]["pending"].as_array();
    if request["payload"]["type"] != "request_permission"
        || request["payload"]["permission_request_id"] != permission_request_id.as_str()
        || request["effect_id"] != current.effect_id || request["request_id"] != current.request_id
        || effect["permission_request_id"] != permission_request_id.as_str()
        || effect["effect_id"] != current.effect_id || effect["request_id"] != current.request_id
        || effect["session_id"] != request["session_id"] || witness["session_id"] != request["session_id"]
        || effect["turn_id"] != request["turn_id"] || effect["host_id"] != request["host_id"]
        || effect["catalog_revision"] != request["catalog_revision"]
        || effect["availability_epoch"] != request["availability_epoch"]
        || !pending.is_some_and(|items| items.iter().any(|ask| ask["request_id"] == permission_request_id.as_str()
            && ask["turn_id"] == request["turn_id"]))
    { return Err(StoreError::Invariant("permission redisplay does not match original pending approval".into())); }
    Ok(())
}

pub(super) const MAX_RECOVERY_EFFECT_DELIVERIES: usize = 256;

fn decode(raw: String) -> StoreResult<EffectDeliveryAttempt> {
    let record: EffectDeliveryAttempt = serde_json::from_str(&raw).map_err(|error| {
        StoreError::Invariant(format!("invalid effect delivery attempt: {error}"))
    })?;
    nola_kernel::validate_effect_delivery_attempt(&record).map_err(StoreError::Invariant)?;
    Ok(record)
}

fn read(connection: &Connection, id: &str) -> StoreResult<Option<EffectDeliveryAttempt>> {
    connection
        .query_row(
            "SELECT record_json FROM effect_delivery_attempts WHERE id=?1",
            params![id],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .map(decode)
        .transpose()
}

fn state_name(state: EffectDeliveryAttemptState) -> StoreResult<String> {
    serde_json::to_value(state)
        .map_err(StoreError::Json)?
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| StoreError::Invariant("effect delivery state must encode as text".into()))
}

fn replay_safety_name(record: &EffectDeliveryAttempt) -> StoreResult<String> {
    serde_json::to_value(record.replay_safety)
        .map_err(StoreError::Json)?
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| StoreError::Invariant("effect replay safety must encode as text".into()))
}

fn recovery_plan_name(record: &EffectDeliveryAttempt) -> StoreResult<Option<String>> {
    record
        .recovery_plan
        .map(serde_json::to_value)
        .transpose()
        .map_err(StoreError::Json)?
        .map(|value| {
            value.as_str().map(str::to_owned).ok_or_else(|| {
                StoreError::Invariant("effect recovery plan must encode as text".into())
            })
        })
        .transpose()
}

fn validate_transition(
    old: &EffectDeliveryAttempt,
    new: &EffectDeliveryAttempt,
) -> StoreResult<()> {
    if old.id != new.id
        || old.run_id != new.run_id
        || old.effect_id != new.effect_id
        || old.request_id != new.request_id
        || old.predecessor_id != new.predecessor_id
        || old.attempt_number != new.attempt_number
        || old.replay_safety != new.replay_safety
        || old.created_at_ms != new.created_at_ms
    {
        return Err(StoreError::Conflict(
            "effect delivery immutable identity changed".into(),
        ));
    }
    let permitted = matches!(
        (old.state, new.state),
        (
            EffectDeliveryAttemptState::Prepared,
            EffectDeliveryAttemptState::Leased
        ) | (
            EffectDeliveryAttemptState::Prepared,
            EffectDeliveryAttemptState::Settled
        ) | (
            EffectDeliveryAttemptState::Leased,
            EffectDeliveryAttemptState::Leased
        ) | (
            EffectDeliveryAttemptState::Leased,
            EffectDeliveryAttemptState::ExecutionStarted
        ) | (
            EffectDeliveryAttemptState::Leased,
            EffectDeliveryAttemptState::Settled
        ) | (
            EffectDeliveryAttemptState::Leased,
            EffectDeliveryAttemptState::Interrupted
        ) | (
            EffectDeliveryAttemptState::ExecutionStarted,
            EffectDeliveryAttemptState::ExecutionStarted
        ) | (
            EffectDeliveryAttemptState::ExecutionStarted,
            EffectDeliveryAttemptState::Settled
        ) | (
            EffectDeliveryAttemptState::ExecutionStarted,
            EffectDeliveryAttemptState::Interrupted
        ) | (
            EffectDeliveryAttemptState::ExecutionStarted,
            EffectDeliveryAttemptState::OutcomeUnknown
        ) | (
            EffectDeliveryAttemptState::OutcomeUnknown,
            EffectDeliveryAttemptState::Interrupted
        )
    );
    if !permitted {
        return Err(StoreError::Conflict(
            "invalid effect delivery state transition".into(),
        ));
    }
    if old.state == EffectDeliveryAttemptState::Prepared
        && new.state == EffectDeliveryAttemptState::Settled
        && (new.settlement != Some(nola_domain::EffectDeliverySettlement::RejectedBeforeExecution)
            || new.worker_generation.is_some())
    {
        return Err(StoreError::Conflict("unclaimed delivery can only be rejected without fabricating a lease".into()));
    }
    if old.worker_generation.is_some()
        && (old.worker_generation != new.worker_generation
            || old.lease_owner != new.lease_owner
            || old.lease_token != new.lease_token
            || old.claimed_at_ms != new.claimed_at_ms)
    {
        return Err(StoreError::Conflict(
            "effect delivery lease identity changed".into(),
        ));
    }
    if old.execution_started_at_ms.is_some()
        && old.execution_started_at_ms != new.execution_started_at_ms
    {
        return Err(StoreError::Conflict(
            "effect delivery execution evidence changed".into(),
        ));
    }
    if old.settlement.is_some() && old.settlement != new.settlement
        || old.recovery_plan.is_some() && old.recovery_plan != new.recovery_plan
        || old.interruption_reason.is_some() && old.interruption_reason != new.interruption_reason
        || old.finished_at_ms.is_some() && old.finished_at_ms != new.finished_at_ms
    {
        return Err(StoreError::Conflict(
            "effect delivery terminal evidence changed".into(),
        ));
    }
    Ok(())
}

fn validate_insert(connection: &Connection, record: &EffectDeliveryAttempt) -> StoreResult<()> {
    if let Some(predecessor_id) = &record.predecessor_id {
        let predecessor = read(connection, predecessor_id)?
            .ok_or_else(|| StoreError::Conflict("effect delivery predecessor missing".into()))?;
        if predecessor.run_id != record.run_id
            || predecessor.effect_id != record.effect_id
            || predecessor.replay_safety != record.replay_safety
            || predecessor.attempt_number.checked_add(1) != Some(record.attempt_number)
            || predecessor.state != EffectDeliveryAttemptState::Interrupted
        {
            return Err(StoreError::Conflict(
                "effect delivery predecessor chain mismatch".into(),
            ));
        }
        let owner: Option<(String, String, String)> = connection
            .query_row(
                "SELECT run_id,request_id,status FROM effect_outbox WHERE effect_id=?1",
                params![record.effect_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        if owner.as_ref()
            != Some(&(
                record.run_id.clone(),
                record.request_id.clone(),
                "pending".to_string(),
            ))
        {
            return Err(StoreError::Conflict(
                "effect delivery successor does not own the current pending outbox request".into(),
            ));
        }
    } else {
        if record.attempt_number != 1 {
            return Err(StoreError::Conflict(
                "initial effect delivery attempt number must be one".into(),
            ));
        }
        let owner: Option<(String, String, String)> = connection
            .query_row(
                "SELECT run_id,request_id,status FROM effect_outbox WHERE effect_id=?1",
                params![record.effect_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        if owner.as_ref()
            != Some(&(
                record.run_id.clone(),
                record.request_id.clone(),
                "pending".to_string(),
            ))
        {
            return Err(StoreError::Conflict(
                "effect delivery does not own a pending outbox effect".into(),
            ));
        }
        let exists: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM effect_delivery_attempts WHERE effect_id=?1)",
            params![record.effect_id],
            |row| row.get(0),
        )?;
        if exists {
            return Err(StoreError::Conflict(
                "effect delivery initial attempt already exists".into(),
            ));
        }
    }
    Ok(())
}

fn append_event(connection: &Connection, record: &EffectDeliveryAttempt) -> StoreResult<()> {
    let sequence: i64 = connection.query_row(
        "SELECT next_event_sequence FROM runs WHERE id=?1",
        params![record.run_id],
        |row| row.get(0),
    )?;
    let payload = serde_json::to_string(record).map_err(StoreError::Json)?;
    let event_time_ms = match &record.manual_resolution {
        Some(nola_domain::EffectDeliveryManualResolution::RiskRetryAuthorized {
            resolved_at_ms,
            ..
        }) => *resolved_at_ms,
        Some(nola_domain::EffectDeliveryManualResolution::PendingPermissionRedisplay { resolved_at_ms, .. }) => *resolved_at_ms,
        None => record.heartbeat_at_ms.unwrap_or(record.created_at_ms),
    };
    connection.execute(
        "INSERT INTO run_events(event_id,run_id,sequence,event_kind,payload_json,created_at_ms)
         VALUES(?1,?2,?3,'effect_delivery_attempt_changed',?4,?5)",
        params![
            format!("effect-delivery:{}:{}", record.id, record.revision),
            record.run_id,
            sequence,
            payload,
            event_time_ms,
        ],
    )?;
    connection.execute(
        "UPDATE runs SET next_event_sequence=?2 WHERE id=?1",
        params![
            record.run_id,
            sequence
                .checked_add(1)
                .ok_or_else(|| { StoreError::Invariant("Run event sequence overflow".into()) })?
        ],
    )?;
    Ok(())
}

pub(super) fn apply(
    connection: &Connection,
    mutations: &[EffectDeliveryAttemptMutation],
) -> StoreResult<Vec<EffectDeliveryAttempt>> {
    apply_with_authority(connection,mutations,false,false,false)
}

pub(super) fn apply_cancellation_admission(connection: &Connection, mutations: &[EffectDeliveryAttemptMutation])
    -> StoreResult<Vec<EffectDeliveryAttempt>> {
    apply_with_authority(connection,mutations,false,true,false)
}

pub(super) fn apply_recovery(connection: &Connection, mutations: &[EffectDeliveryAttemptMutation])
    -> StoreResult<Vec<EffectDeliveryAttempt>> {
    apply_with_authority(connection, mutations, false, false, true)
}

fn apply_with_authority(
    connection: &Connection, mutations: &[EffectDeliveryAttemptMutation], policy_transition: bool, cancellation_admission: bool, permission_recovery: bool,
) -> StoreResult<Vec<EffectDeliveryAttempt>> {
    if mutations.is_empty() {
        return Ok(Vec::new());
    }
    if mutations.len() > MAX_EFFECT_DELIVERY_BATCH {
        return Err(StoreError::Invariant(format!(
            "effect delivery mutation batch may contain at most {MAX_EFFECT_DELIVERY_BATCH} records"
        )));
    }
    let mut identities = BTreeSet::new();
    for mutation in mutations {
        if !identities.insert(mutation.record.id.as_str()) {
            return Err(StoreError::Conflict(
                "effect delivery mutation batch contains duplicate attempt identity".into(),
            ));
        }
        let record = &mutation.record;
        if record.interruption_reason.as_deref() == Some("permission_policy_changed") && !policy_transition {
            return Err(StoreError::Invariant("permission withdrawal requires its canonical policy transition".into()));
        }
        if record.interruption_reason.as_deref() == Some("cancellation_not_started_admission") && !cancellation_admission {
            return Err(StoreError::Invariant("cancellation revocation requires its canonical proof transaction".into()));
        }
        nola_kernel::validate_effect_delivery_attempt(record).map_err(StoreError::Invariant)?;
        if !record.state.is_terminal() {
            let sealed:bool=connection.query_row("SELECT status='sealed' FROM runs WHERE id=?1",
                [&record.run_id],|r|r.get(0))?;
            if sealed { return Err(StoreError::Conflict("sealed Run cannot dispatch or renew an effect".into())); }
        }
        let raw = serde_json::to_string(record).map_err(StoreError::Json)?;
        if raw.len() > 65_536 {
            return Err(StoreError::Invariant(
                "effect delivery attempt exceeds 64KiB".into(),
            ));
        }
        let state = state_name(record.state)?;
        let replay_safety = replay_safety_name(record)?;
        let recovery_plan = recovery_plan_name(record)?;
        match mutation.expected_revision {
            None => {
                if let Some(old) = read(connection, &record.id)? {
                    if old == *record {
                        continue;
                    }
                    return Err(StoreError::Conflict(
                        "effect delivery attempt identity already exists".into(),
                    ));
                }
                validate_insert(connection, record)?;
                connection.execute(
                    "INSERT INTO effect_delivery_attempts(
                        id,run_id,effect_id,request_id,predecessor_id,attempt_number,
                        replay_safety,state,recovery_plan,worker_generation,lease_owner,lease_token,
                        lease_expires_at_ms,revision,record_json
                     ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)",
                    params![
                        record.id,
                        record.run_id,
                        record.effect_id,
                        record.request_id,
                        record.predecessor_id,
                        record.attempt_number,
                        replay_safety,
                        state,
                        recovery_plan,
                        record.worker_generation,
                        record.lease_owner,
                        record.lease_token,
                        record.lease_expires_at_ms,
                        record.revision,
                        raw,
                    ],
                )?;
                append_event(connection, record)?;
            }
            Some(expected) => {
                let old = read(connection, &record.id)?.ok_or_else(|| {
                    StoreError::Conflict("effect delivery attempt missing".into())
                })?;
                if old.revision != expected || expected.checked_add(1) != Some(record.revision) {
                    return Err(StoreError::Conflict(
                        "effect delivery attempt revision changed".into(),
                    ));
                }
                validate_transition(&old, record)?;
                if matches!(record.manual_resolution, Some(nola_domain::EffectDeliveryManualResolution::PendingPermissionRedisplay { .. })) {
                    if !permission_recovery { return Err(StoreError::Invariant("permission redisplay requires same-transaction recovery authority".into())); }
                    validate_permission_redisplay_source(connection, &old, record)?;
                } else if old.state==EffectDeliveryAttemptState::OutcomeUnknown {
                    validate_user_retry_source(connection,&old,record)?;
                }
                let updated = connection.execute(
                    "UPDATE effect_delivery_attempts SET
                        state=?2,recovery_plan=?3,worker_generation=?4,lease_owner=?5,lease_token=?6,
                        lease_expires_at_ms=?7,revision=?8,record_json=?9
                     WHERE id=?1 AND revision=?10",
                    params![
                        record.id,
                        state,
                        recovery_plan,
                        record.worker_generation,
                        record.lease_owner,
                        record.lease_token,
                        record.lease_expires_at_ms,
                        record.revision,
                        raw,
                        expected,
                    ],
                )?;
                if updated != 1 {
                    return Err(StoreError::Conflict(
                        "effect delivery attempt revision changed".into(),
                    ));
                }
                append_event(connection, record)?;
            }
        }
    }
    Ok(mutations
        .iter()
        .map(|mutation| mutation.record.clone())
        .collect())
}

/// The caller has validated the exact committed policy command and removed
/// these Core effects. This write shares the checkpoint/outbox transaction.
pub(super) fn withdraw_for_permission_policy(
    connection: &Connection, effect_id: &str, now_ms: i64,
) -> StoreResult<()> {
    let raw: Option<String> = connection.query_row(
        "SELECT record_json FROM effect_delivery_attempts WHERE effect_id=?1
         AND state IN ('prepared','leased','execution_started')", [effect_id], |row| row.get(0),
    ).optional()?;
    let Some(raw) = raw else { return Ok(()); };
    let current = decode(raw)?;
    let mutation = nola_kernel::withdraw_unstarted_effect_for_permission_policy(&current,
        now_ms.max(current.heartbeat_at_ms.unwrap_or(current.created_at_ms)))
        .map_err(StoreError::Conflict)?;
    apply_with_authority(connection, &[mutation],true,false,false)?;
    Ok(())
}

pub(super) fn recovery_records(
    connection: &Connection,
    run_id: &str,
) -> StoreResult<Vec<EffectDeliveryAttempt>> {
    let mut statement = connection.prepare(
        // A resumed attempt permanently supersedes its interrupted predecessor.
        // Only chain tails own current recovery work; ancestors retain their
        // immutable request IDs and must not be matched against the rebound outbox.
        "SELECT attempt.record_json FROM effect_delivery_attempts attempt
         WHERE attempt.run_id=?1
         AND attempt.state IN ('prepared','leased','execution_started','interrupted','outcome_unknown')
         AND NOT EXISTS (SELECT 1 FROM effect_delivery_attempts successor
                         WHERE successor.predecessor_id=attempt.id)
         ORDER BY attempt.state,attempt.id LIMIT ?2",
    )?;
    let raw = statement
        .query_map(
            params![run_id, (MAX_RECOVERY_EFFECT_DELIVERIES + 1) as i64],
            |row| row.get::<_, String>(0),
        )?
        .collect::<Result<Vec<_>, _>>()?;
    if raw.len() > MAX_RECOVERY_EFFECT_DELIVERIES {
        return Err(StoreError::Invariant(
            "Run exceeds bounded effect delivery recovery window".into(),
        ));
    }
    raw.into_iter().map(decode).collect()
}

impl NolaStore {
    pub fn runtime_effect_work_schedule(
        &self,
        conversation_ids: &[String],
        now_ms: i64,
        limit: usize,
    ) -> StoreResult<RuntimeEffectWorkSchedule> {
        if conversation_ids.is_empty()
            || conversation_ids.len() > MAX_EFFECT_DELIVERY_BATCH
            || now_ms < 0
            || limit == 0
            || limit > MAX_EFFECT_DELIVERY_BATCH
        {
            return Err(StoreError::Invariant(format!(
                "runtime effect work schedule requires 1..={MAX_EFFECT_DELIVERY_BATCH} conversations, a nonnegative clock and limit 1..={MAX_EFFECT_DELIVERY_BATCH}"
            )));
        }
        let mut unique = BTreeSet::new();
        for conversation_id in conversation_ids {
            require_id("runtime_effect_work.conversation_id", conversation_id)?;
            if !unique.insert(conversation_id.as_str()) {
                return Err(StoreError::Invariant(
                    "runtime effect work schedule contains duplicate conversations".into(),
                ));
            }
        }
        let scope_values = (0..conversation_ids.len())
            .map(|index| format!("(?{})", index + 2))
            .collect::<Vec<_>>()
            .join(",");
        let limit_parameter = conversation_ids.len() + 2;
        let schedule_sql = format!(
            "WITH scope(conversation_id) AS (VALUES {scope_values}), runnable AS (
               SELECT run.conversation_id,effect.run_id,effect.effect_id,effect.request_id,
                      NULL AS attempt_id,'pending' AS reason,
                      MIN(effect.created_at_ms,?1) AS ready_at_ms,
                      effect.created_at_ms,1 AS due_rank
               FROM effect_outbox effect
               JOIN runs run ON run.id=effect.run_id
               JOIN scope ON scope.conversation_id=run.conversation_id
               WHERE effect.status='pending'
                 AND NOT EXISTS(
                   SELECT 1 FROM effect_delivery_attempts attempt
                   WHERE attempt.effect_id=effect.effect_id
                 )
               UNION ALL
               SELECT run.conversation_id,effect.run_id,effect.effect_id,effect.request_id,
                      attempt.id,attempt.recovery_plan AS reason,
                      MIN(CAST(COALESCE(
                            json_extract(attempt.record_json,'$.finished_at_ms'),
                            json_extract(attempt.record_json,'$.created_at_ms')) AS INTEGER),?1)
                        AS ready_at_ms,
                      effect.created_at_ms,1 AS due_rank
               FROM effect_outbox effect
               JOIN effect_delivery_attempts attempt ON attempt.effect_id=effect.effect_id
               JOIN runs run ON run.id=effect.run_id
               JOIN scope ON scope.conversation_id=run.conversation_id
               WHERE effect.status='pending'
                 AND attempt.run_id=effect.run_id
                 AND attempt.state='interrupted'
                 AND attempt.recovery_plan IN ('retry','resume')
                 AND ((attempt.recovery_plan='retry' AND effect.request_id=attempt.request_id)
                   OR (attempt.recovery_plan='resume' AND effect.request_id<>attempt.request_id))
                 AND NOT EXISTS(
                   SELECT 1 FROM effect_delivery_attempts successor
                   WHERE successor.predecessor_id=attempt.id
                 )
                 AND NOT EXISTS(
                   SELECT 1 FROM effect_delivery_attempts active
                   WHERE active.effect_id=attempt.effect_id
                     AND active.state IN ('prepared','leased','execution_started')
                 )
               UNION ALL
               SELECT run.conversation_id,effect.run_id,effect.effect_id,effect.request_id,
                      attempt.id,'recover_expired_lease' AS reason,
                      attempt.lease_expires_at_ms AS ready_at_ms,
                      effect.created_at_ms,0 AS due_rank
               FROM effect_delivery_attempts attempt
               JOIN effect_outbox effect ON effect.effect_id=attempt.effect_id
               JOIN runs run ON run.id=effect.run_id
               JOIN scope ON scope.conversation_id=run.conversation_id
               WHERE effect.status='pending' AND effect.run_id=attempt.run_id
                 AND attempt.state IN ('leased','execution_started')
                 AND attempt.lease_expires_at_ms IS NOT NULL
                 AND attempt.lease_expires_at_ms<=?1
             ), ranked AS (
               SELECT conversation_id,run_id,effect_id,request_id,attempt_id,reason,
                      ready_at_ms,created_at_ms,due_rank,
                      ROW_NUMBER() OVER (
                        PARTITION BY run_id
                        ORDER BY due_rank,ready_at_ms,created_at_ms,run_id,effect_id
                      ) AS run_rank
               FROM runnable
             )
             SELECT conversation_id,run_id,effect_id,request_id,attempt_id,reason,
                    ready_at_ms,created_at_ms
             FROM ranked WHERE run_rank=1
             ORDER BY due_rank,ready_at_ms,created_at_ms,run_id,effect_id
             LIMIT ?{limit_parameter}"
        );
        let mut schedule_parameters =
            Vec::<rusqlite::types::Value>::with_capacity(conversation_ids.len() + 2);
        schedule_parameters.push(now_ms.into());
        schedule_parameters.extend(conversation_ids.iter().cloned().map(Into::into));
        schedule_parameters.push((limit as i64).into());
        let mut statement = self.connection.prepare(&schedule_sql)?;
        let runs = statement
            .query_map(
                rusqlite::params_from_iter(schedule_parameters.iter()),
                |row| {
                    let reason = match row.get::<_, String>(5)?.as_str() {
                        "pending" => RuntimeEffectWorkReason::Pending,
                        "retry" => RuntimeEffectWorkReason::Retry,
                        "resume" => RuntimeEffectWorkReason::Resume,
                        "recover_expired_lease" => RuntimeEffectWorkReason::RecoverExpiredLease,
                        _ => unreachable!("query emits only typed work reasons"),
                    };
                    Ok(RuntimeEffectWorkRunHead {
                        conversation_id: row.get(0)?,
                        run_id: row.get(1)?,
                        effect_id: row.get(2)?,
                        request_id: row.get(3)?,
                        attempt_id: row.get(4)?,
                        reason,
                        ready_at_ms: row.get(6)?,
                        created_at_ms: row.get(7)?,
                    })
                },
            )?
            .collect::<Result<Vec<_>, _>>()?;
        let wake_sql = format!(
            "WITH scope(conversation_id) AS (VALUES {scope_values})
             SELECT MIN(attempt.lease_expires_at_ms)
             FROM effect_delivery_attempts attempt
             JOIN effect_outbox effect ON effect.effect_id=attempt.effect_id
             JOIN runs run ON run.id=effect.run_id
             JOIN scope ON scope.conversation_id=run.conversation_id
             WHERE effect.status='pending' AND effect.run_id=attempt.run_id
               AND attempt.state IN ('leased','execution_started')
               AND attempt.lease_expires_at_ms>?1"
        );
        let mut wake_parameters =
            Vec::<rusqlite::types::Value>::with_capacity(conversation_ids.len() + 1);
        wake_parameters.push(now_ms.into());
        wake_parameters.extend(conversation_ids.iter().cloned().map(Into::into));
        let next_wake_at_ms = self.connection.query_row(
            &wake_sql,
            rusqlite::params_from_iter(wake_parameters.iter()),
            |row| row.get(0),
        )?;
        Ok(RuntimeEffectWorkSchedule {
            runs,
            next_wake_at_ms,
        })
    }

    pub fn pending_effect_delivery_candidates(
        &self,
        run_id: &str,
        after: Option<&EffectDeliveryCandidateCursor>,
        limit: usize,
    ) -> StoreResult<Vec<EffectDeliveryCandidate>> {
        require_id("effect_delivery.run_id", run_id)?;
        if limit == 0 || limit > MAX_EFFECT_DELIVERY_BATCH {
            return Err(StoreError::Invariant(format!(
                "effect delivery candidate limit must be 1..={MAX_EFFECT_DELIVERY_BATCH}"
            )));
        }
        if let Some(cursor) = after {
            require_id("effect_delivery.cursor.effect_id", &cursor.effect_id)?;
        }
        let (after_time, after_id) = after
            .map(|cursor| (cursor.created_at_ms, cursor.effect_id.as_str()))
            .unwrap_or((i64::MIN, ""));
        let mut statement = self.connection.prepare(
            "SELECT e.effect_id,e.run_id,e.request_id,e.created_at_ms
             FROM effect_outbox e
             WHERE e.run_id=?1 AND e.status='pending'
               AND (e.created_at_ms>?2 OR (e.created_at_ms=?2 AND e.effect_id>?3))
               AND NOT EXISTS(
                    SELECT 1 FROM effect_delivery_attempts a WHERE a.effect_id=e.effect_id
               )
             ORDER BY e.created_at_ms,e.effect_id LIMIT ?4",
        )?;
        let rows =
            statement.query_map(params![run_id, after_time, after_id, limit as i64], |row| {
                Ok(EffectDeliveryCandidate {
                    effect_id: row.get(0)?,
                    run_id: row.get(1)?,
                    request_id: row.get(2)?,
                    created_at_ms: row.get(3)?,
                })
            })?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(StoreError::from)
    }

    pub fn effect_delivery_attempt(
        &self,
        attempt_id: &str,
    ) -> StoreResult<Option<EffectDeliveryAttempt>> {
        require_id("effect_delivery.attempt_id", attempt_id)?;
        read(&self.connection, attempt_id)
    }

    pub fn active_effect_delivery_attempt(
        &self,
        effect_id: &str,
    ) -> StoreResult<Option<EffectDeliveryAttempt>> {
        require_id("effect_delivery.effect_id", effect_id)?;
        let mut statement = self.connection.prepare(
            "SELECT record_json FROM effect_delivery_attempts
             WHERE effect_id=?1 AND state IN ('prepared','leased','execution_started') LIMIT 2",
        )?;
        let raw = statement
            .query_map(params![effect_id], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        if raw.len() > 1 {
            return Err(StoreError::Invariant(
                "effect has multiple active delivery attempts".into(),
            ));
        }
        raw.into_iter().next().map(decode).transpose()
    }

    pub fn interrupted_effect_delivery_candidates(
        &self,
        run_id: &str,
        after: Option<&InterruptedEffectDeliveryCursor>,
        limit: usize,
        include_unrebound: bool,
    ) -> StoreResult<Vec<EffectDeliveryAttempt>> {
        require_id("effect_delivery.run_id", run_id)?;
        if limit == 0 || limit > MAX_EFFECT_DELIVERY_BATCH {
            return Err(StoreError::Invariant(format!(
                "interrupted effect delivery candidate limit must be 1..={MAX_EFFECT_DELIVERY_BATCH}"
            )));
        }
        if let Some(cursor) = after {
            require_id("effect_delivery.cursor.attempt_id", &cursor.attempt_id)?;
        }
        let after_id = after.map(|cursor| cursor.attempt_id.as_str()).unwrap_or("");
        let mut statement = self.connection.prepare(
            "SELECT attempt.record_json
             FROM effect_delivery_attempts attempt
             JOIN effect_outbox effect ON effect.effect_id=attempt.effect_id
             WHERE attempt.run_id=?1
               AND attempt.state='interrupted'
               AND attempt.recovery_plan IN ('retry','resume')
               AND attempt.id>?2
               AND effect.run_id=attempt.run_id
               AND effect.status='pending'
               AND (
                    (attempt.recovery_plan='retry' AND effect.request_id=attempt.request_id)
                    OR
                    (attempt.recovery_plan='resume' AND (?4 OR effect.request_id<>attempt.request_id))
               )
               AND NOT EXISTS(
                    SELECT 1 FROM effect_delivery_attempts successor
                    WHERE successor.predecessor_id=attempt.id
               )
               AND NOT EXISTS(
                    SELECT 1 FROM effect_delivery_attempts active
                    WHERE active.effect_id=attempt.effect_id
                      AND active.state IN ('prepared','leased','execution_started')
               )
             ORDER BY attempt.id LIMIT ?3",
        )?;
        let raw = statement
            .query_map(params![run_id, after_id, limit as i64, include_unrebound], |row| {
                row.get::<_, String>(0)
            })?
            .collect::<Result<Vec<_>, _>>()?;
        raw.into_iter().map(decode).collect()
    }

    pub fn claim_effect_delivery_attempts(
        &mut self,
        claims: &[EffectDeliveryAttemptMutation],
    ) -> StoreResult<Vec<EffectDeliveryAttempt>> {
        if claims.is_empty() {
            return Err(StoreError::Invariant(
                "effect delivery claim batch must not be empty".into(),
            ));
        }
        for claim in claims {
            if claim.expected_revision.is_some()
                || claim.record.predecessor_id.is_some()
                || claim.record.attempt_number != 1
                || claim.record.state != EffectDeliveryAttemptState::Leased
            {
                return Err(StoreError::Invariant(
                    "initial claim must insert attempt one directly in Leased state".into(),
                ));
            }
        }
        let transaction = self.write_transaction()?;
        let records = apply(&transaction, claims)?;
        transaction.commit()?;
        Ok(records)
    }

    pub fn commit_effect_delivery_attempt_mutations(
        &mut self,
        mutations: &[EffectDeliveryAttemptMutation],
    ) -> StoreResult<Vec<EffectDeliveryAttempt>> {
        if mutations.is_empty() {
            return Err(StoreError::Invariant(
                "effect delivery mutation batch must not be empty".into(),
            ));
        }
        let transaction = self.write_transaction()?;
        let records = apply(&transaction, mutations)?;
        transaction.commit()?;
        Ok(records)
    }

    pub fn heartbeat_effect_delivery_attempt(
        &mut self,
        heartbeat: &EffectDeliveryAttemptMutation,
    ) -> StoreResult<EffectDeliveryAttempt> {
        let expected = heartbeat.expected_revision.ok_or_else(|| {
            StoreError::Invariant("effect delivery heartbeat requires a revision CAS".into())
        })?;
        let current = self
            .effect_delivery_attempt(&heartbeat.record.id)?
            .ok_or_else(|| StoreError::Conflict("effect delivery attempt missing".into()))?;
        if current.revision != expected
            || current.state != heartbeat.record.state
            || !matches!(
                current.state,
                EffectDeliveryAttemptState::Leased | EffectDeliveryAttemptState::ExecutionStarted
            )
            || heartbeat.record.heartbeat_at_ms <= current.heartbeat_at_ms
            || heartbeat.record.lease_expires_at_ms <= current.lease_expires_at_ms
        {
            return Err(StoreError::Conflict(
                "effect delivery heartbeat is not a same-state lease extension".into(),
            ));
        }
        let transaction = self.write_transaction()?;
        let mut records = apply(&transaction, std::slice::from_ref(heartbeat))?;
        transaction.commit()?;
        records.pop().ok_or_else(|| {
            StoreError::Invariant("effect delivery heartbeat was not applied".into())
        })
    }

    pub fn authorize_unknown_effect_retry(
        &mut self,
        authorization: &EffectDeliveryAttemptMutation,
    ) -> StoreResult<EffectDeliveryAttempt> {
        let expected = authorization.expected_revision.ok_or_else(|| {
            StoreError::Invariant(
                "unknown effect retry authorization requires a revision CAS".into(),
            )
        })?;
        if authorization.record.state != EffectDeliveryAttemptState::Interrupted
            || authorization.record.manual_resolution.is_none()
            || authorization.record.recovery_plan
                != Some(nola_domain::EffectDeliveryRecoveryPlan::Retry)
        {
            return Err(StoreError::Invariant(
                "unknown effect retry authorization requires a typed Retry resolution".into(),
            ));
        }
        let transaction = self.write_transaction()?;
        let current = read(&transaction, &authorization.record.id)?
            .ok_or_else(|| StoreError::Conflict("effect delivery attempt missing".into()))?;
        validate_user_retry_source(&transaction, &current, &authorization.record)?;
        if current.revision != expected
            || current.state != EffectDeliveryAttemptState::OutcomeUnknown
            || current.run_id != authorization.record.run_id
            || current.effect_id != authorization.record.effect_id
            || current.request_id != authorization.record.request_id
            || current.worker_generation != authorization.record.worker_generation
            || current.lease_owner != authorization.record.lease_owner
            || current.lease_token != authorization.record.lease_token
        {
            return Err(StoreError::Conflict(
                "unknown effect retry authorization lost its exact attempt fence".into(),
            ));
        }
        let owns_pending: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM effect_outbox
             WHERE effect_id=?1 AND run_id=?2 AND request_id=?3 AND status='pending')",
            params![current.effect_id, current.run_id, current.request_id],
            |row| row.get(0),
        )?;
        if !owns_pending {
            return Err(StoreError::Conflict(
                "unknown effect retry authorization lost its pending outbox request".into(),
            ));
        }
        let mut records = apply(&transaction, std::slice::from_ref(authorization))?;
        transaction.commit()?;
        records.pop().ok_or_else(|| {
            StoreError::Invariant("unknown effect retry authorization was not applied".into())
        })
    }
}

impl EffectDeliveryRepository for NolaStore {
    type Error = StoreError;
    fn unknown_effect_recovery_receipt(&self, id: &str) -> StoreResult<Option<(String,String)>> {
        self.connection.query_row("SELECT request_digest,receipt_json FROM unknown_effect_recovery_receipts WHERE resolution_id=?1",
            [id], |row|Ok((row.get(0)?,row.get(1)?))).optional().map_err(StoreError::from)
    }
    fn record_unknown_effect_recovery_receipt(&mut self, id: &str, digest: &str, receipt: &str) -> StoreResult<()> {
        if self.connection.is_autocommit() { return Err(StoreError::Invariant("recovery receipt requires original decision transaction".into())); }
        self.connection.execute("INSERT INTO unknown_effect_recovery_receipts VALUES(?1,?2,?3)",params![id,digest,receipt])?;
        Ok(())
    }

    fn runtime_effect_work_schedule(
        &self,
        conversation_ids: &[String],
        now_ms: i64,
        limit: usize,
    ) -> Result<RuntimeEffectWorkSchedule, Self::Error> {
        NolaStore::runtime_effect_work_schedule(self, conversation_ids, now_ms, limit)
    }

    fn pending_effect_delivery_candidates(
        &self,
        run_id: &str,
        after: Option<&EffectDeliveryCandidateCursor>,
        limit: usize,
    ) -> Result<Vec<EffectDeliveryCandidate>, Self::Error> {
        NolaStore::pending_effect_delivery_candidates(self, run_id, after, limit)
    }

    fn effect_delivery_attempt(
        &self,
        attempt_id: &str,
    ) -> Result<Option<EffectDeliveryAttempt>, Self::Error> {
        NolaStore::effect_delivery_attempt(self, attempt_id)
    }

    fn active_effect_delivery_attempt(
        &self,
        effect_id: &str,
    ) -> Result<Option<EffectDeliveryAttempt>, Self::Error> {
        NolaStore::active_effect_delivery_attempt(self, effect_id)
    }

    fn interrupted_effect_delivery_candidates(
        &self,
        run_id: &str,
        after: Option<&InterruptedEffectDeliveryCursor>,
        limit: usize,
        include_unrebound: bool,
    ) -> Result<Vec<EffectDeliveryAttempt>, Self::Error> {
        NolaStore::interrupted_effect_delivery_candidates(self, run_id, after, limit, include_unrebound)
    }

    fn claim_effect_delivery_attempts(
        &mut self,
        claims: &[EffectDeliveryAttemptMutation],
    ) -> Result<Vec<EffectDeliveryAttempt>, Self::Error> {
        NolaStore::claim_effect_delivery_attempts(self, claims)
    }

    fn heartbeat_effect_delivery_attempt(
        &mut self,
        heartbeat: &EffectDeliveryAttemptMutation,
    ) -> Result<EffectDeliveryAttempt, Self::Error> {
        NolaStore::heartbeat_effect_delivery_attempt(self, heartbeat)
    }

    fn authorize_unknown_effect_retry(
        &mut self,
        authorization: &EffectDeliveryAttemptMutation,
    ) -> Result<EffectDeliveryAttempt, Self::Error> {
        NolaStore::authorize_unknown_effect_retry(self, authorization)
    }

    fn commit_effect_delivery_attempt_mutations(
        &mut self,
        mutations: &[EffectDeliveryAttemptMutation],
    ) -> Result<Vec<EffectDeliveryAttempt>, Self::Error> {
        NolaStore::commit_effect_delivery_attempt_mutations(self, mutations)
    }
}
