// SPDX-License-Identifier: Apache-2.0
//! Pure lease, execution, settlement, and recovery transitions for Host-effect delivery.
use nola_domain::{
    EffectDeliveryAttempt, EffectDeliveryAttemptMutation, EffectDeliveryAttemptState,
    EffectDeliveryManualResolution, EffectDeliveryReclaimPlan, EffectDeliveryRecoveryPlan,
    EffectDeliveryReplaySafety, EffectDeliverySettlement, MAX_EFFECT_DELIVERY_LEASE_EXTENSION_MS,
};
use EffectDeliveryAttemptState::*;

fn identity(value: &str) -> Result<(), String> {
    if value.trim().is_empty() || value.len() > 512 || value.chars().any(char::is_control) {
        return Err("invalid effect delivery identity".into());
    }
    Ok(())
}

fn reason(value: &str) -> Result<(), String> {
    if value.trim().is_empty() || value.len() > 4096 {
        return Err("invalid effect delivery interruption reason".into());
    }
    Ok(())
}

pub fn validate_effect_delivery_attempt(record: &EffectDeliveryAttempt) -> Result<(), String> {
    for value in [
        &record.id,
        &record.run_id,
        &record.effect_id,
        &record.request_id,
    ] {
        identity(value)?;
    }
    if let Some(value) = &record.predecessor_id {
        identity(value)?;
        if value == &record.id {
            return Err("effect delivery attempt cannot precede itself".into());
        }
    }
    for value in [
        &record.worker_generation,
        &record.lease_owner,
        &record.lease_token,
    ]
    .into_iter()
    .flatten()
    {
        identity(value)?;
    }
    if let Some(value) = &record.interruption_reason {
        reason(value)?;
    }
    if record.attempt_number == 0 || record.revision < 1 || record.created_at_ms < 0 {
        return Err("invalid effect delivery attempt number, revision, or creation time".into());
    }

    let lease_binding_count = [
        record.worker_generation.is_some(),
        record.lease_owner.is_some(),
        record.lease_token.is_some(),
        record.lease_expires_at_ms.is_some(),
        record.claimed_at_ms.is_some(),
        record.heartbeat_at_ms.is_some(),
    ]
    .into_iter()
    .filter(|bound| *bound)
    .count();
    if lease_binding_count != 0 && lease_binding_count != 6 {
        return Err("incomplete effect delivery lease binding".into());
    }
    let leased = lease_binding_count == 6;
    let rejected_before_execution =
        record.settlement == Some(EffectDeliverySettlement::RejectedBeforeExecution);
    let unclaimed_rejection = record.state == Settled && rejected_before_execution && !leased;
    if (record.state == Prepared || unclaimed_rejection) != !leased {
        return Err("effect delivery state and lease binding disagree".into());
    }
    if leased {
        let claimed = record.claimed_at_ms.unwrap();
        let heartbeat = record.heartbeat_at_ms.unwrap();
        let expires = record.lease_expires_at_ms.unwrap();
        if claimed < record.created_at_ms || heartbeat < claimed || expires <= heartbeat {
            return Err("invalid effect delivery lease timestamps".into());
        }
    }

    let execution_started = record.execution_started_at_ms.is_some();
    if ((matches!(record.state, ExecutionStarted | OutcomeUnknown)
        || (record.state == Settled && !rejected_before_execution))
        && !execution_started)
        || (matches!(record.state, Prepared | Leased) && execution_started)
        || (rejected_before_execution && execution_started)
    {
        return Err("effect delivery execution evidence mismatch".into());
    }
    if let Some(started) = record.execution_started_at_ms {
        if started < record.claimed_at_ms.unwrap_or(record.created_at_ms)
            || started > record.heartbeat_at_ms.unwrap_or(started)
        {
            return Err("invalid effect delivery execution timestamp".into());
        }
    }
    let terminal = record.state.is_terminal();
    if terminal != record.finished_at_ms.is_some() {
        return Err("effect delivery terminal timestamp mismatch".into());
    }
    if let Some(finished) = record.finished_at_ms {
        if finished
            < record
                .execution_started_at_ms
                .or(record.claimed_at_ms)
                .unwrap_or(record.created_at_ms)
        {
            return Err("effect delivery finish time moved backwards".into());
        }
    }
    if (record.state == Settled) != record.settlement.is_some()
        || (record.state == Interrupted) != record.recovery_plan.is_some()
        || (matches!(record.state, Interrupted | OutcomeUnknown) || rejected_before_execution)
            != record.interruption_reason.is_some()
    {
        return Err("effect delivery terminal evidence mismatch".into());
    }
    match &record.manual_resolution {
        Some(EffectDeliveryManualResolution::PendingPermissionRedisplay { permission_request_id, resolved_at_ms }) => {
            identity(permission_request_id)?;
            if record.state != Interrupted
                || record.recovery_plan != Some(EffectDeliveryRecoveryPlan::Retry)
                || record.replay_safety != EffectDeliveryReplaySafety::RequiresRecoveryDecision
                || !execution_started
                || *resolved_at_ms < record.finished_at_ms.unwrap_or(i64::MAX)
                || record.lease_expires_at_ms.is_none_or(|expiry| expiry > *resolved_at_ms)
            { return Err("invalid pending permission redisplay evidence".into()); }
        }
        Some(EffectDeliveryManualResolution::RiskRetryAuthorized {
            evidence,
            resolved_at_ms,
        }) => {
            if evidence.trim().is_empty() || evidence.len() > 16_384 {
                return Err("invalid bounded retry authorization".into());
            }
            if record.state != Interrupted
                || record.recovery_plan != Some(EffectDeliveryRecoveryPlan::Retry)
                || record.replay_safety != EffectDeliveryReplaySafety::RequiresRecoveryDecision
                || !execution_started
                || *resolved_at_ms < record.finished_at_ms.unwrap_or(i64::MAX)
            {
                return Err("invalid manual effect delivery resolution evidence".into());
            }
        }
        None => {}
    }
    if record.state == OutcomeUnknown
        && record.replay_safety != EffectDeliveryReplaySafety::RequiresRecoveryDecision
    {
        return Err("only a recovery-decision effect may have an unknown outcome".into());
    }
    if record.state == Interrupted {
        let expected = if execution_started {
            match record.replay_safety {
                EffectDeliveryReplaySafety::Idempotent => EffectDeliveryRecoveryPlan::Retry,
                EffectDeliveryReplaySafety::Recoverable => EffectDeliveryRecoveryPlan::Resume,
                EffectDeliveryReplaySafety::RequiresRecoveryDecision
                    if record.manual_resolution.is_some() =>
                {
                    EffectDeliveryRecoveryPlan::Retry
                }
                EffectDeliveryReplaySafety::RequiresRecoveryDecision => {
                    return Err(
                        "executed recovery-decision effect cannot be marked interrupted".into(),
                    )
                }
            }
        } else {
            EffectDeliveryRecoveryPlan::Retry
        };
        if record.recovery_plan != Some(expected) {
            return Err("effect delivery interruption lacks the required recovery plan".into());
        }
    }
    Ok(())
}

pub fn prepare_effect_delivery_attempt(
    id: &str,
    run_id: &str,
    effect_id: &str,
    request_id: &str,
    attempt_number: u32,
    replay_safety: EffectDeliveryReplaySafety,
    predecessor_id: Option<&str>,
    now_ms: i64,
) -> Result<EffectDeliveryAttemptMutation, String> {
    let record = EffectDeliveryAttempt {
        id: id.into(),
        run_id: run_id.into(),
        effect_id: effect_id.into(),
        request_id: request_id.into(),
        predecessor_id: predecessor_id.map(str::to_owned),
        attempt_number,
        replay_safety,
        state: Prepared,
        worker_generation: None,
        lease_owner: None,
        lease_token: None,
        lease_expires_at_ms: None,
        settlement: None,
        recovery_plan: None,
        manual_resolution: None,
        interruption_reason: None,
        created_at_ms: now_ms,
        claimed_at_ms: None,
        execution_started_at_ms: None,
        heartbeat_at_ms: None,
        finished_at_ms: None,
        revision: 1,
    };
    validate_effect_delivery_attempt(&record)?;
    Ok(EffectDeliveryAttemptMutation {
        expected_revision: None,
        record,
    })
}

fn next(
    current: &EffectDeliveryAttempt,
    now_ms: i64,
) -> Result<EffectDeliveryAttemptMutation, String> {
    validate_effect_delivery_attempt(current)?;
    if current.state.is_terminal() {
        return Err("terminal effect delivery attempt cannot change".into());
    }
    let floor = current
        .heartbeat_at_ms
        .or(current.claimed_at_ms)
        .unwrap_or(current.created_at_ms);
    if now_ms < floor {
        return Err("effect delivery time moved backwards".into());
    }
    let mut record = current.clone();
    record.revision = current
        .revision
        .checked_add(1)
        .ok_or("effect delivery revision exhausted")?;
    Ok(EffectDeliveryAttemptMutation {
        expected_revision: Some(current.revision),
        record,
    })
}

fn validate_lease_owner(
    current: &EffectDeliveryAttempt,
    worker_generation: &str,
    lease_owner: &str,
    lease_token: &str,
) -> Result<(), String> {
    for value in [worker_generation, lease_owner, lease_token] {
        identity(value)?;
    }
    if current.worker_generation.as_deref() != Some(worker_generation)
        || current.lease_owner.as_deref() != Some(lease_owner)
        || current.lease_token.as_deref() != Some(lease_token)
    {
        return Err("stale effect delivery lease owner, generation, or token".into());
    }
    Ok(())
}

fn validate_live_lease(current: &EffectDeliveryAttempt, now_ms: i64) -> Result<(), String> {
    if now_ms >= current.lease_expires_at_ms.unwrap_or(i64::MIN) {
        return Err("effect delivery lease expired".into());
    }
    Ok(())
}

fn validate_lease_extension(lease_expires_at_ms: i64, now_ms: i64) -> Result<(), String> {
    let extension_ms = lease_expires_at_ms
        .checked_sub(now_ms)
        .ok_or("effect delivery lease extension overflowed")?;
    if !(1..=MAX_EFFECT_DELIVERY_LEASE_EXTENSION_MS).contains(&extension_ms) {
        return Err("effect delivery lease extension exceeds its bounded maximum".into());
    }
    Ok(())
}

pub fn claim_effect_delivery_attempt(
    current: &EffectDeliveryAttempt,
    worker_generation: &str,
    lease_owner: &str,
    lease_token: &str,
    lease_expires_at_ms: i64,
    now_ms: i64,
) -> Result<EffectDeliveryAttemptMutation, String> {
    let mut mutation = next(current, now_ms)?;
    if current.state != Prepared {
        return Err("only a prepared effect delivery attempt may be claimed".into());
    }
    for value in [worker_generation, lease_owner, lease_token] {
        identity(value)?;
    }
    validate_lease_extension(lease_expires_at_ms, now_ms)?;
    mutation.record.state = Leased;
    mutation.record.worker_generation = Some(worker_generation.into());
    mutation.record.lease_owner = Some(lease_owner.into());
    mutation.record.lease_token = Some(lease_token.into());
    mutation.record.lease_expires_at_ms = Some(lease_expires_at_ms);
    mutation.record.claimed_at_ms = Some(now_ms);
    mutation.record.heartbeat_at_ms = Some(now_ms);
    validate_effect_delivery_attempt(&mutation.record)?;
    Ok(mutation)
}

pub fn start_effect_delivery_execution(
    current: &EffectDeliveryAttempt,
    worker_generation: &str,
    lease_owner: &str,
    lease_token: &str,
    now_ms: i64,
) -> Result<EffectDeliveryAttemptMutation, String> {
    validate_lease_owner(current, worker_generation, lease_owner, lease_token)?;
    validate_live_lease(current, now_ms)?;
    let mut mutation = next(current, now_ms)?;
    if current.state != Leased {
        return Err("only a leased effect delivery attempt may start execution".into());
    }
    mutation.record.state = ExecutionStarted;
    mutation.record.execution_started_at_ms = Some(now_ms);
    mutation.record.heartbeat_at_ms = Some(now_ms);
    validate_effect_delivery_attempt(&mutation.record)?;
    Ok(mutation)
}

pub fn heartbeat_effect_delivery_attempt(
    current: &EffectDeliveryAttempt,
    worker_generation: &str,
    lease_owner: &str,
    lease_token: &str,
    lease_expires_at_ms: i64,
    now_ms: i64,
) -> Result<EffectDeliveryAttemptMutation, String> {
    validate_lease_owner(current, worker_generation, lease_owner, lease_token)?;
    validate_live_lease(current, now_ms)?;
    let mut mutation = next(current, now_ms)?;
    if !matches!(current.state, Leased | ExecutionStarted) {
        return Err("only an active effect delivery lease may heartbeat".into());
    }
    validate_lease_extension(lease_expires_at_ms, now_ms)?;
    if lease_expires_at_ms <= current.lease_expires_at_ms.unwrap() {
        return Err("effect delivery heartbeat must strictly extend its lease".into());
    }
    mutation.record.heartbeat_at_ms = Some(now_ms);
    mutation.record.lease_expires_at_ms = Some(lease_expires_at_ms);
    validate_effect_delivery_attempt(&mutation.record)?;
    Ok(mutation)
}

pub fn settle_effect_delivery_attempt(
    current: &EffectDeliveryAttempt,
    worker_generation: &str,
    lease_owner: &str,
    lease_token: &str,
    settlement: EffectDeliverySettlement,
    now_ms: i64,
) -> Result<EffectDeliveryAttemptMutation, String> {
    validate_lease_owner(current, worker_generation, lease_owner, lease_token)?;
    validate_live_lease(current, now_ms)?;
    let mut mutation = next(current, now_ms)?;
    if current.state != ExecutionStarted
        || settlement == EffectDeliverySettlement::RejectedBeforeExecution
    {
        return Err("effect delivery cannot settle before execution starts".into());
    }
    mutation.record.state = Settled;
    mutation.record.settlement = Some(settlement);
    mutation.record.finished_at_ms = Some(now_ms);
    validate_effect_delivery_attempt(&mutation.record)?;
    Ok(mutation)
}

/// Application calls this only after a fenced cancellation control has been
/// accepted by Core for this exact target. The target's expired lease is not
/// execution authority; a stop confirmation is a separate control authority.
pub fn confirm_effect_delivery_target_stopped(
    current: &EffectDeliveryAttempt,
    now_ms: i64,
) -> Result<EffectDeliveryAttemptMutation, String> {
    validate_effect_delivery_attempt(current)?;
    if !matches!(current.state, Prepared | Leased | ExecutionStarted) {
        return Err("cancellation confirmation requires an active target delivery".into());
    }
    let mut mutation = next(current, now_ms)?;
    mutation.record.state = Settled;
    if current.execution_started_at_ms.is_some() {
        mutation.record.settlement = Some(EffectDeliverySettlement::Cancelled);
    } else {
        mutation.record.settlement = Some(EffectDeliverySettlement::RejectedBeforeExecution);
        mutation.record.interruption_reason = Some("cancelled_before_execution".into());
    }
    mutation.record.finished_at_ms = Some(now_ms);
    validate_effect_delivery_attempt(&mutation.record)?;
    Ok(mutation)
}

/// Records a lease owner's explicit refusal before any external execution began.
pub fn withdraw_unstarted_effect_for_permission_policy(
    current: &EffectDeliveryAttempt,
    now_ms: i64,
) -> Result<EffectDeliveryAttemptMutation, String> {
    validate_effect_delivery_attempt(current)?;
    if current.state != Leased || current.execution_started_at_ms.is_some() {
        return Err("permission policy cannot withdraw an admitted physical execution".into());
    }
    let mut mutation = next(current, now_ms)?;
    mutation.record.state = Settled;
    mutation.record.settlement = Some(EffectDeliverySettlement::RejectedBeforeExecution);
    mutation.record.interruption_reason = Some("permission_policy_changed".into());
    mutation.record.finished_at_ms = Some(now_ms);
    validate_effect_delivery_attempt(&mutation.record)?;
    Ok(mutation)
}

/// Application's serialized cancellation admission, not a worker stop claim.
/// The Store must additionally exclude *all* historical started attempts.
pub fn withdraw_unstarted_effect_for_cancellation(current: &EffectDeliveryAttempt, now_ms: i64)
    -> Result<EffectDeliveryAttemptMutation, String> {
    validate_effect_delivery_attempt(current)?;
    if !matches!(current.state, Prepared | Leased) || current.execution_started_at_ms.is_some() {
        return Err("cancellation cannot revoke an admitted physical execution".into());
    }
    let mut mutation = next(current, now_ms)?;
    mutation.record.state = Settled;
    mutation.record.settlement = Some(EffectDeliverySettlement::RejectedBeforeExecution);
    mutation.record.interruption_reason = Some("cancellation_not_started_admission".into());
    mutation.record.finished_at_ms = Some(now_ms);
    validate_effect_delivery_attempt(&mutation.record)?;
    Ok(mutation)
}

/// Records a lease owner's explicit refusal before any external execution began.
pub fn reject_effect_delivery_before_execution(
    current: &EffectDeliveryAttempt,
    worker_generation: &str,
    lease_owner: &str,
    lease_token: &str,
    rejection_reason: &str,
    now_ms: i64,
) -> Result<EffectDeliveryAttemptMutation, String> {
    validate_lease_owner(current, worker_generation, lease_owner, lease_token)?;
    validate_live_lease(current, now_ms)?;
    reason(rejection_reason)?;
    let mut mutation = next(current, now_ms)?;
    if current.state != Leased || current.execution_started_at_ms.is_some() {
        return Err("only a leased, unstarted delivery may be rejected before execution".into());
    }
    mutation.record.state = Settled;
    mutation.record.settlement = Some(EffectDeliverySettlement::RejectedBeforeExecution);
    mutation.record.interruption_reason = Some(rejection_reason.into());
    mutation.record.finished_at_ms = Some(now_ms);
    validate_effect_delivery_attempt(&mutation.record)?;
    Ok(mutation)
}

fn expected_recovery_plan(
    current: &EffectDeliveryAttempt,
) -> Result<EffectDeliveryRecoveryPlan, String> {
    if current.state == Leased {
        return Ok(EffectDeliveryRecoveryPlan::Retry);
    }
    if current.state != ExecutionStarted {
        return Err("only an active effect delivery attempt may be interrupted".into());
    }
    match current.replay_safety {
        EffectDeliveryReplaySafety::Idempotent => Ok(EffectDeliveryRecoveryPlan::Retry),
        EffectDeliveryReplaySafety::Recoverable => Ok(EffectDeliveryRecoveryPlan::Resume),
        EffectDeliveryReplaySafety::RequiresRecoveryDecision => {
            Err("executed recovery-decision effect must become outcome unknown".into())
        }
    }
}

pub fn interrupt_effect_delivery_attempt(
    current: &EffectDeliveryAttempt,
    worker_generation: &str,
    lease_owner: &str,
    lease_token: &str,
    recovery_plan: EffectDeliveryRecoveryPlan,
    interruption_reason: &str,
    now_ms: i64,
) -> Result<EffectDeliveryAttemptMutation, String> {
    validate_lease_owner(current, worker_generation, lease_owner, lease_token)?;
    reason(interruption_reason)?;
    let expected = expected_recovery_plan(current)?;
    if recovery_plan != expected {
        return Err(
            "effect delivery interruption recovery plan does not match replay safety".into(),
        );
    }
    let mut mutation = next(current, now_ms)?;
    mutation.record.state = Interrupted;
    mutation.record.recovery_plan = Some(recovery_plan);
    mutation.record.interruption_reason = Some(interruption_reason.into());
    mutation.record.finished_at_ms = Some(now_ms);
    validate_effect_delivery_attempt(&mutation.record)?;
    Ok(mutation)
}

pub fn mark_effect_delivery_outcome_unknown(
    current: &EffectDeliveryAttempt,
    worker_generation: &str,
    lease_owner: &str,
    lease_token: &str,
    interruption_reason: &str,
    now_ms: i64,
) -> Result<EffectDeliveryAttemptMutation, String> {
    validate_lease_owner(current, worker_generation, lease_owner, lease_token)?;
    reason(interruption_reason)?;
    if current.state != ExecutionStarted
        || current.replay_safety != EffectDeliveryReplaySafety::RequiresRecoveryDecision
    {
        return Err("only an executed recovery-decision effect may become outcome unknown".into());
    }
    let mut mutation = next(current, now_ms)?;
    mutation.record.state = OutcomeUnknown;
    mutation.record.interruption_reason = Some(interruption_reason.into());
    mutation.record.finished_at_ms = Some(now_ms);
    validate_effect_delivery_attempt(&mutation.record)?;
    Ok(mutation)
}

#[allow(clippy::too_many_arguments)]
/// Caller must prove the exact approval remains pending in its Store transaction.
/// No permission decision or business effect is produced by this transition.
pub fn authorize_pending_permission_redisplay(
    current: &EffectDeliveryAttempt,
    permission_request_id: &str,
    now_ms: i64,
) -> Result<EffectDeliveryAttemptMutation, String> {
    validate_effect_delivery_attempt(current)?;
    identity(permission_request_id)?;
    if current.state != ExecutionStarted
        || current.replay_safety != EffectDeliveryReplaySafety::RequiresRecoveryDecision
        || current.lease_expires_at_ms.is_none_or(|expiry| expiry > now_ms)
    { return Err("permission redisplay requires an expired executed delivery".into()); }
    let mut mutation = next(current, now_ms)?;
    mutation.record.state = Interrupted;
    mutation.record.recovery_plan = Some(EffectDeliveryRecoveryPlan::Retry);
    mutation.record.interruption_reason = Some("original approval remains pending after delivery expiry".into());
    mutation.record.finished_at_ms = Some(now_ms);
    mutation.record.manual_resolution = Some(EffectDeliveryManualResolution::PendingPermissionRedisplay {
        permission_request_id: permission_request_id.into(), resolved_at_ms: now_ms,
    });
    validate_effect_delivery_attempt(&mutation.record)?;
    Ok(mutation)
}

pub fn authorize_unknown_effect_retry(
    current: &EffectDeliveryAttempt,
    run_id: &str,
    effect_id: &str,
    request_id: &str,
    attempt_id: &str,
    worker_generation: &str,
    lease_owner: &str,
    lease_token: &str,
    expected_revision: i64,
    evidence: &str,
    resolved_at_ms: i64,
) -> Result<EffectDeliveryAttemptMutation, String> {
    validate_effect_delivery_attempt(current)?;
    for value in [
        run_id,
        effect_id,
        request_id,
        attempt_id,
        worker_generation,
        lease_owner,
        lease_token,
    ] {
        identity(value)?;
    }
    if evidence.trim().is_empty() || evidence.len() > 16_384 {
        return Err("invalid bounded retry authorization".into());
    }
    if current.state != OutcomeUnknown
        || current.replay_safety != EffectDeliveryReplaySafety::RequiresRecoveryDecision
        || current.run_id != run_id
        || current.effect_id != effect_id
        || current.request_id != request_id
        || current.id != attempt_id
        || current.worker_generation.as_deref() != Some(worker_generation)
        || current.lease_owner.as_deref() != Some(lease_owner)
        || current.lease_token.as_deref() != Some(lease_token)
        || current.revision != expected_revision
    {
        return Err("unknown effect retry authorization identity or revision mismatch".into());
    }
    if resolved_at_ms < current.finished_at_ms.unwrap_or(i64::MAX) {
        return Err("unknown effect retry resolution time moved backwards".into());
    }
    let mut record = current.clone();
    record.revision = current
        .revision
        .checked_add(1)
        .ok_or("effect delivery revision exhausted")?;
    record.state = Interrupted;
    record.recovery_plan = Some(EffectDeliveryRecoveryPlan::Retry);
    record.manual_resolution = Some(EffectDeliveryManualResolution::RiskRetryAuthorized {
        evidence: evidence.into(),
        resolved_at_ms,
    });
    validate_effect_delivery_attempt(&record)?;
    Ok(EffectDeliveryAttemptMutation {
        expected_revision: Some(expected_revision),
        record,
    })
}

#[allow(clippy::too_many_arguments)]
pub fn reclaim_expired_effect_delivery_attempt(
    current: &EffectDeliveryAttempt,
    current_worker_generation: &str,
    current_lease_owner: &str,
    current_lease_token: &str,
    new_id: &str,
    new_worker_generation: &str,
    new_lease_owner: &str,
    new_lease_token: &str,
    new_lease_expires_at_ms: i64,
    now_ms: i64,
) -> Result<EffectDeliveryReclaimPlan, String> {
    validate_lease_owner(
        current,
        current_worker_generation,
        current_lease_owner,
        current_lease_token,
    )?;
    if current.state != Leased || now_ms < current.lease_expires_at_ms.unwrap_or(i64::MAX) {
        return Err("only an expired, unstarted lease may be reclaimed automatically".into());
    }
    let interrupted = interrupt_effect_delivery_attempt(
        current,
        current_worker_generation,
        current_lease_owner,
        current_lease_token,
        EffectDeliveryRecoveryPlan::Retry,
        "lease expired before execution started",
        now_ms,
    )?;
    let successor = claim_effect_delivery_successor(
        current,
        new_id,
        &current.request_id,
        new_worker_generation,
        new_lease_owner,
        new_lease_token,
        new_lease_expires_at_ms,
        now_ms,
    )?;
    Ok(EffectDeliveryReclaimPlan {
        interrupted,
        successor,
    })
}

#[allow(clippy::too_many_arguments)]
fn claim_effect_delivery_successor(
    current: &EffectDeliveryAttempt,
    new_id: &str,
    committed_request_id: &str,
    new_worker_generation: &str,
    new_lease_owner: &str,
    new_lease_token: &str,
    new_lease_expires_at_ms: i64,
    now_ms: i64,
) -> Result<EffectDeliveryAttemptMutation, String> {
    if new_id == current.id
        || current.worker_generation.as_deref() == Some(new_worker_generation)
        || current.lease_owner.as_deref() == Some(new_lease_owner)
        || current.lease_token.as_deref() == Some(new_lease_token)
    {
        return Err(
            "delivery successor requires fresh attempt, generation, owner, and token identities"
                .into(),
        );
    }
    let attempt_number = current
        .attempt_number
        .checked_add(1)
        .ok_or("effect delivery attempt number exhausted")?;
    let prepared = prepare_effect_delivery_attempt(
        new_id,
        &current.run_id,
        &current.effect_id,
        committed_request_id,
        attempt_number,
        current.replay_safety,
        Some(&current.id),
        now_ms,
    )?;
    let claimed = claim_effect_delivery_attempt(
        &prepared.record,
        new_worker_generation,
        new_lease_owner,
        new_lease_token,
        new_lease_expires_at_ms,
        now_ms,
    )?;
    // Preparation and claim are one atomic insert; no intermediate row is published.
    Ok(EffectDeliveryAttemptMutation {
        expected_revision: None,
        record: claimed.record,
    })
}

/// Claims a fresh physical attempt from an explicit Retry/Resume recovery plan.
/// The terminal predecessor remains immutable and is not included in the returned mutation.
#[allow(clippy::too_many_arguments)]
pub fn claim_interrupted_effect_delivery_successor(
    current: &EffectDeliveryAttempt,
    new_id: &str,
    committed_request_id: &str,
    new_worker_generation: &str,
    new_lease_owner: &str,
    new_lease_token: &str,
    new_lease_expires_at_ms: i64,
    now_ms: i64,
) -> Result<EffectDeliveryAttemptMutation, String> {
    validate_effect_delivery_attempt(current)?;
    if current.state != Interrupted || current.recovery_plan.is_none() {
        return Err(
            "only an interrupted delivery with an explicit recovery plan may continue".into(),
        );
    }
    if now_ms < current.finished_at_ms.unwrap_or(current.created_at_ms) {
        return Err("effect delivery recovery time moved backwards".into());
    }
    claim_effect_delivery_successor(
        current,
        new_id,
        committed_request_id,
        new_worker_generation,
        new_lease_owner,
        new_lease_token,
        new_lease_expires_at_ms,
        now_ms,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prepared(safety: EffectDeliveryReplaySafety) -> EffectDeliveryAttempt {
        prepare_effect_delivery_attempt("a1", "run", "effect", "request-1", 1, safety, None, 100)
            .unwrap()
            .record
    }

    fn leased(safety: EffectDeliveryReplaySafety) -> EffectDeliveryAttempt {
        claim_effect_delivery_attempt(
            &prepared(safety),
            "generation-1",
            "worker-1",
            "token-1",
            200,
            110,
        )
        .unwrap()
        .record
    }

    fn executing(safety: EffectDeliveryReplaySafety) -> EffectDeliveryAttempt {
        start_effect_delivery_execution(&leased(safety), "generation-1", "worker-1", "token-1", 120)
            .unwrap()
            .record
    }

    #[test]
    fn confirmed_stop_settles_an_expired_target_without_fabricating_execution_or_rewriting_history()
    {
        for state in [Prepared, Leased, ExecutionStarted] {
            let source = match state {
                Prepared => prepared(EffectDeliveryReplaySafety::Recoverable),
                Leased => leased(EffectDeliveryReplaySafety::Recoverable),
                _ => executing(EffectDeliveryReplaySafety::Recoverable),
            };
            let started = state == ExecutionStarted;
            let result = confirm_effect_delivery_target_stopped(&source, 300).unwrap();
            assert_eq!(result.expected_revision, Some(source.revision));
            assert_eq!(result.record.state, Settled);
            assert_eq!(
                result.record.execution_started_at_ms,
                source.execution_started_at_ms
            );
            assert_eq!(result.record.lease_token, source.lease_token);
            assert_eq!(
                result.record.lease_expires_at_ms,
                source.lease_expires_at_ms
            );
            assert_eq!(
                result.record.settlement,
                Some(if started {
                    EffectDeliverySettlement::Cancelled
                } else {
                    EffectDeliverySettlement::RejectedBeforeExecution
                })
            );
            assert!(confirm_effect_delivery_target_stopped(&result.record, 301).is_err());
            assert!(confirm_effect_delivery_target_stopped(&source, 99).is_err());
        }
    }

    #[test]
    fn admission_claim_heartbeat_execution_and_settlement_are_cas_transitions() {
        let source = prepared(EffectDeliveryReplaySafety::Recoverable);
        let claim =
            claim_effect_delivery_attempt(&source, "generation-1", "worker-1", "token-1", 200, 110)
                .unwrap();
        assert_eq!(claim.expected_revision, Some(1));
        assert_eq!(claim.record.revision, 2);
        assert_eq!(source.state, Prepared);
        let heartbeat = heartbeat_effect_delivery_attempt(
            &claim.record,
            "generation-1",
            "worker-1",
            "token-1",
            250,
            120,
        )
        .unwrap();
        assert_eq!(heartbeat.expected_revision, Some(2));
        assert_eq!(heartbeat.record.heartbeat_at_ms, Some(120));
        let started = start_effect_delivery_execution(
            &heartbeat.record,
            "generation-1",
            "worker-1",
            "token-1",
            130,
        )
        .unwrap();
        assert_eq!(started.expected_revision, Some(3));
        let settled = settle_effect_delivery_attempt(
            &started.record,
            "generation-1",
            "worker-1",
            "token-1",
            EffectDeliverySettlement::Succeeded,
            140,
        )
        .unwrap();
        assert_eq!(settled.expected_revision, Some(4));
        assert_eq!(settled.record.state, Settled);
        assert_eq!(
            settled.record.settlement,
            Some(EffectDeliverySettlement::Succeeded)
        );
        assert!(heartbeat_effect_delivery_attempt(
            &settled.record,
            "generation-1",
            "worker-1",
            "token-1",
            300,
            150
        )
        .is_err());
    }

    #[test]
    fn claim_and_heartbeat_enforce_bounded_strict_extensions_without_overflow() {
        let source = prepared(EffectDeliveryReplaySafety::Idempotent);
        assert!(claim_effect_delivery_attempt(
            &source,
            "generation-1",
            "worker-1",
            "token-1",
            100 + MAX_EFFECT_DELIVERY_LEASE_EXTENSION_MS,
            100,
        )
        .is_ok());
        assert!(claim_effect_delivery_attempt(
            &source,
            "generation-1",
            "worker-1",
            "token-1",
            101 + MAX_EFFECT_DELIVERY_LEASE_EXTENSION_MS,
            100,
        )
        .is_err());

        let current = leased(EffectDeliveryReplaySafety::Idempotent);
        assert!(heartbeat_effect_delivery_attempt(
            &current,
            "generation-1",
            "worker-1",
            "token-1",
            200,
            120,
        )
        .is_err());
        assert!(heartbeat_effect_delivery_attempt(
            &current,
            "generation-1",
            "worker-1",
            "token-1",
            121 + MAX_EFFECT_DELIVERY_LEASE_EXTENSION_MS,
            120,
        )
        .is_err());

        let near_max = prepare_effect_delivery_attempt(
            "near-max",
            "run",
            "effect-max",
            "request-max",
            1,
            EffectDeliveryReplaySafety::Idempotent,
            None,
            100,
        )
        .unwrap();
        assert!(claim_effect_delivery_attempt(
            &near_max.record,
            "generation-max",
            "worker-max",
            "token-max",
            i64::MAX,
            i64::MAX - 100,
        )
        .is_ok());
        assert!(claim_effect_delivery_attempt(
            &source,
            "generation-1",
            "worker-1",
            "token-1",
            i64::MIN,
            i64::MAX,
        )
        .is_err());
    }

    #[test]
    fn every_worker_transition_rejects_a_stale_generation_owner_or_token_and_expiry() {
        let current = leased(EffectDeliveryReplaySafety::Idempotent);
        for (generation, owner, token) in [
            ("old-generation", "worker-1", "token-1"),
            ("generation-1", "old-worker", "token-1"),
            ("generation-1", "worker-1", "old-token"),
        ] {
            assert!(
                start_effect_delivery_execution(&current, generation, owner, token, 120).is_err()
            );
            assert!(heartbeat_effect_delivery_attempt(
                &current, generation, owner, token, 250, 120
            )
            .is_err());
            assert!(interrupt_effect_delivery_attempt(
                &current,
                generation,
                owner,
                token,
                EffectDeliveryRecoveryPlan::Retry,
                "lost",
                210
            )
            .is_err());
        }
        assert!(start_effect_delivery_execution(
            &current,
            "generation-1",
            "worker-1",
            "token-1",
            200
        )
        .is_err());
        assert!(heartbeat_effect_delivery_attempt(
            &current,
            "generation-1",
            "worker-1",
            "token-1",
            300,
            200
        )
        .is_err());
    }

    #[test]
    fn expired_unstarted_lease_reclaim_preserves_old_evidence_and_creates_a_new_attempt() {
        let old = leased(EffectDeliveryReplaySafety::RequiresRecoveryDecision);
        let plan = reclaim_expired_effect_delivery_attempt(
            &old,
            "generation-1",
            "worker-1",
            "token-1",
            "a2",
            "generation-2",
            "worker-2",
            "token-2",
            320,
            220,
        )
        .unwrap();
        assert_eq!(plan.interrupted.expected_revision, Some(2));
        assert_eq!(plan.interrupted.record.state, Interrupted);
        assert_eq!(
            plan.interrupted.record.recovery_plan,
            Some(EffectDeliveryRecoveryPlan::Retry)
        );
        assert_eq!(
            plan.interrupted.record.worker_generation.as_deref(),
            Some("generation-1")
        );
        assert_eq!(plan.successor.expected_revision, None);
        assert_eq!(plan.successor.record.id, "a2");
        assert_eq!(plan.successor.record.request_id, old.request_id);
        assert_eq!(plan.successor.record.predecessor_id.as_deref(), Some("a1"));
        assert_eq!(plan.successor.record.attempt_number, 2);
        assert_eq!(
            plan.successor.record.worker_generation.as_deref(),
            Some("generation-2")
        );
        assert_eq!(old.state, Leased);
        assert!(reclaim_expired_effect_delivery_attempt(
            &old,
            "generation-1",
            "worker-1",
            "token-1",
            "a2",
            "generation-2",
            "worker-2",
            "token-2",
            320,
            199
        )
        .is_err());
    }

    #[test]
    fn execution_started_recovery_is_fixed_by_replay_safety() {
        let idempotent = executing(EffectDeliveryReplaySafety::Idempotent);
        assert!(interrupt_effect_delivery_attempt(
            &idempotent,
            "generation-1",
            "worker-1",
            "token-1",
            EffectDeliveryRecoveryPlan::Resume,
            "disconnected",
            210
        )
        .is_err());
        let retry = interrupt_effect_delivery_attempt(
            &idempotent,
            "generation-1",
            "worker-1",
            "token-1",
            EffectDeliveryRecoveryPlan::Retry,
            "disconnected",
            210,
        )
        .unwrap();
        assert_eq!(retry.record.state, Interrupted);

        let recoverable = executing(EffectDeliveryReplaySafety::Recoverable);
        assert!(interrupt_effect_delivery_attempt(
            &recoverable,
            "generation-1",
            "worker-1",
            "token-1",
            EffectDeliveryRecoveryPlan::Retry,
            "disconnected",
            210
        )
        .is_err());
        let resume = interrupt_effect_delivery_attempt(
            &recoverable,
            "generation-1",
            "worker-1",
            "token-1",
            EffectDeliveryRecoveryPlan::Resume,
            "disconnected",
            210,
        )
        .unwrap();
        assert_eq!(
            resume.record.recovery_plan,
            Some(EffectDeliveryRecoveryPlan::Resume)
        );

        let unsafe_attempt = executing(EffectDeliveryReplaySafety::RequiresRecoveryDecision);
        for plan in [
            EffectDeliveryRecoveryPlan::Retry,
            EffectDeliveryRecoveryPlan::Resume,
        ] {
            assert!(interrupt_effect_delivery_attempt(
                &unsafe_attempt,
                "generation-1",
                "worker-1",
                "token-1",
                plan,
                "crashed",
                210
            )
            .is_err());
        }
        let unknown = mark_effect_delivery_outcome_unknown(
            &unsafe_attempt,
            "generation-1",
            "worker-1",
            "token-1",
            "crashed after start",
            210,
        )
        .unwrap();
        assert_eq!(unknown.record.state, OutcomeUnknown);
        assert_eq!(unknown.record.recovery_plan, None);
    }

    #[test]
    fn invalid_records_and_terminal_states_cannot_be_reopened_or_downgraded() {
        let mut partial = prepared(EffectDeliveryReplaySafety::Recoverable);
        partial.worker_generation = Some("generation".into());
        assert!(validate_effect_delivery_attempt(&partial).is_err());
        let settled = settle_effect_delivery_attempt(
            &executing(EffectDeliveryReplaySafety::Recoverable),
            "generation-1",
            "worker-1",
            "token-1",
            EffectDeliverySettlement::Failed,
            130,
        )
        .unwrap()
        .record;
        assert!(settle_effect_delivery_attempt(
            &settled,
            "generation-1",
            "worker-1",
            "token-1",
            EffectDeliverySettlement::Succeeded,
            140
        )
        .is_err());
        assert!(interrupt_effect_delivery_attempt(
            &settled,
            "generation-1",
            "worker-1",
            "token-1",
            EffectDeliveryRecoveryPlan::Resume,
            "late",
            140
        )
        .is_err());
        let mut overflow = leased(EffectDeliveryReplaySafety::Idempotent);
        overflow.revision = i64::MAX;
        assert!(start_effect_delivery_execution(
            &overflow,
            "generation-1",
            "worker-1",
            "token-1",
            120
        )
        .is_err());
    }

    #[test]
    fn explicit_interruption_plan_claims_a_fresh_successor_without_mutating_the_terminal_attempt() {
        for (safety, recovery_plan) in [
            (
                EffectDeliveryReplaySafety::Idempotent,
                EffectDeliveryRecoveryPlan::Retry,
            ),
            (
                EffectDeliveryReplaySafety::Recoverable,
                EffectDeliveryRecoveryPlan::Resume,
            ),
        ] {
            let current = executing(safety);
            let interrupted = interrupt_effect_delivery_attempt(
                &current,
                "generation-1",
                "worker-1",
                "token-1",
                recovery_plan,
                "worker disconnected",
                210,
            )
            .unwrap()
            .record;
            let predecessor = interrupted.clone();
            let successor = claim_interrupted_effect_delivery_successor(
                &interrupted,
                "a2",
                "request-2",
                "generation-2",
                "worker-2",
                "token-2",
                330,
                220,
            )
            .unwrap();
            assert_eq!(successor.expected_revision, None);
            assert_eq!(successor.record.state, Leased);
            assert_eq!(successor.record.run_id, predecessor.run_id);
            assert_eq!(successor.record.effect_id, predecessor.effect_id);
            assert_eq!(successor.record.request_id, "request-2");
            assert_ne!(successor.record.request_id, predecessor.request_id);
            assert_eq!(successor.record.replay_safety, predecessor.replay_safety);
            assert_eq!(successor.record.predecessor_id.as_deref(), Some("a1"));
            assert_eq!(successor.record.attempt_number, 2);
            assert_eq!(
                successor.record.worker_generation.as_deref(),
                Some("generation-2")
            );
            assert_eq!(interrupted, predecessor);
            assert!(claim_interrupted_effect_delivery_successor(
                &interrupted,
                "a3",
                "",
                "generation-3",
                "worker-3",
                "token-3",
                340,
                220,
            )
            .is_err());

            for (id, generation, owner, token) in [
                ("a1", "generation-2", "worker-2", "token-2"),
                ("a2", "generation-1", "worker-2", "token-2"),
                ("a2", "generation-2", "worker-1", "token-2"),
                ("a2", "generation-2", "worker-2", "token-1"),
            ] {
                assert!(claim_interrupted_effect_delivery_successor(
                    &interrupted,
                    id,
                    "request-2",
                    generation,
                    owner,
                    token,
                    330,
                    220,
                )
                .is_err());
            }
        }
    }

    #[test]
    fn settled_and_outcome_unknown_attempts_cannot_create_recovery_successors() {
        let settled = settle_effect_delivery_attempt(
            &executing(EffectDeliveryReplaySafety::Recoverable),
            "generation-1",
            "worker-1",
            "token-1",
            EffectDeliverySettlement::Succeeded,
            130,
        )
        .unwrap()
        .record;
        let unknown = mark_effect_delivery_outcome_unknown(
            &executing(EffectDeliveryReplaySafety::RequiresRecoveryDecision),
            "generation-1",
            "worker-1",
            "token-1",
            "worker disappeared",
            210,
        )
        .unwrap()
        .record;
        for terminal in [&settled, &unknown] {
            assert!(claim_interrupted_effect_delivery_successor(
                terminal,
                "a2",
                "request-2",
                "generation-2",
                "worker-2",
                "token-2",
                330,
                220,
            )
            .is_err());
        }
    }

    #[test]
    fn explicit_risk_retry_is_bound_to_the_exact_unknown_attempt() {
        let executing = executing(EffectDeliveryReplaySafety::RequiresRecoveryDecision);
        let unknown = mark_effect_delivery_outcome_unknown(
            &executing,
            "generation-1",
            "worker-1",
            "token-1",
            "transport outcome unavailable",
            130,
        )
        .unwrap()
        .record;
        let authorized = authorize_unknown_effect_retry(
            &unknown,
            "run",
            "effect",
            "request-1",
            "a1",
            "generation-1",
            "worker-1",
            "token-1",
            unknown.revision,
            "operator verified the external system did not apply the operation",
            140,
        )
        .unwrap();
        assert_eq!(authorized.expected_revision, Some(unknown.revision));
        assert_eq!(authorized.record.state, Interrupted);
        assert_eq!(
            authorized.record.recovery_plan,
            Some(EffectDeliveryRecoveryPlan::Retry)
        );
        assert!(matches!(
            authorized.record.manual_resolution,
            Some(EffectDeliveryManualResolution::RiskRetryAuthorized {
                resolved_at_ms: 140,
                ..
            })
        ));
        for result in [
            authorize_unknown_effect_retry(
                &unknown,
                "wrong-run",
                "effect",
                "request-1",
                "a1",
                "generation-1",
                "worker-1",
                "token-1",
                unknown.revision,
                "verified not applied",
                140,
            ),
            authorize_unknown_effect_retry(
                &unknown,
                "run",
                "effect",
                "request-1",
                "a1",
                "generation-1",
                "worker-1",
                "stale-token",
                unknown.revision,
                "verified not applied",
                140,
            ),
            authorize_unknown_effect_retry(
                &unknown,
                "run",
                "effect",
                "request-1",
                "a1",
                "generation-1",
                "worker-1",
                "token-1",
                unknown.revision + 1,
                "verified not applied",
                140,
            ),
            authorize_unknown_effect_retry(
                &unknown,
                "run",
                "effect",
                "request-1",
                "a1",
                "generation-1",
                "worker-1",
                "token-1",
                unknown.revision,
                " ",
                140,
            ),
            authorize_unknown_effect_retry(
                &unknown,
                "run",
                "effect",
                "request-1",
                "a1",
                "generation-1",
                "worker-1",
                "token-1",
                unknown.revision,
                "verified not applied",
                129,
            ),
        ] {
            assert!(result.is_err());
        }
    }

    #[test]
    fn pre_execution_rejection_is_typed_owned_and_does_not_forge_execution_evidence() {
        let source = leased(EffectDeliveryReplaySafety::RequiresRecoveryDecision);
        for (generation, owner, token) in [
            ("old-generation", "worker-1", "token-1"),
            ("generation-1", "old-worker", "token-1"),
            ("generation-1", "worker-1", "old-token"),
        ] {
            assert!(reject_effect_delivery_before_execution(
                &source,
                generation,
                owner,
                token,
                "deadline elapsed",
                120,
            )
            .is_err());
        }
        assert!(reject_effect_delivery_before_execution(
            &source,
            "generation-1",
            "worker-1",
            "token-1",
            "deadline elapsed",
            200,
        )
        .is_err());
        let rejected = reject_effect_delivery_before_execution(
            &source,
            "generation-1",
            "worker-1",
            "token-1",
            "deadline elapsed before dispatch",
            120,
        )
        .unwrap();
        assert_eq!(rejected.expected_revision, Some(source.revision));
        assert_eq!(rejected.record.state, Settled);
        assert_eq!(
            rejected.record.settlement,
            Some(EffectDeliverySettlement::RejectedBeforeExecution)
        );
        assert_eq!(rejected.record.execution_started_at_ms, None);
        assert_eq!(
            rejected.record.interruption_reason.as_deref(),
            Some("deadline elapsed before dispatch")
        );
        assert!(reject_effect_delivery_before_execution(
            &executing(EffectDeliveryReplaySafety::Idempotent),
            "generation-1",
            "worker-1",
            "token-1",
            "too late",
            130,
        )
        .is_err());
        for settlement in [
            EffectDeliverySettlement::Succeeded,
            EffectDeliverySettlement::Failed,
            EffectDeliverySettlement::Cancelled,
        ] {
            let mut forged = rejected.record.clone();
            forged.settlement = Some(settlement);
            forged.interruption_reason = None;
            assert!(validate_effect_delivery_attempt(&forged).is_err());
        }
    }

    #[test]
    fn policy_withdrawal_fences_unstarted_workers_and_preserves_started_execution() {
        let source = leased(EffectDeliveryReplaySafety::RequiresRecoveryDecision);
        let withdrawn = withdraw_unstarted_effect_for_permission_policy(&source, 120).unwrap();
        assert_eq!(withdrawn.expected_revision, Some(source.revision));
        assert_eq!(
            withdrawn.record.settlement,
            Some(EffectDeliverySettlement::RejectedBeforeExecution)
        );
        assert!(withdrawn.record.execution_started_at_ms.is_none());
        assert!(start_effect_delivery_execution(
            &withdrawn.record,
            "generation-1",
            "worker-1",
            "token-1",
            121
        )
        .is_err());
        assert!(withdraw_unstarted_effect_for_permission_policy(
            &executing(EffectDeliveryReplaySafety::Idempotent),
            120
        )
        .is_err());
    }
}
