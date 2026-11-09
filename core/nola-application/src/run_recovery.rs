// SPDX-License-Identifier: Apache-2.0
//! Startup reconciliation over one repository-owned write transaction.
//! Parsing/validation belongs to Application; Kernel owns the recovery policy.
use nola_agent_engine::agent::host::{HostReplaySafety, HostRequest, HostRequestEnvelope};
use nola_agent_engine::agent::recovery::CoreCheckpoint;
use nola_agent_engine::agent::{AgentCore, AgentCoreConfig, AgentThreadState};
use nola_agent_engine::AgentReducerPort;
use nola_domain::{
    AgentTurnStatus, EffectDeliveryAttemptMutation, EffectDeliveryAttemptState,
    EffectDeliveryRecoveryPlan, EffectDeliveryReplaySafety, RunRecoveryAction, RunRecoveryFacts,
    TraceRecoveryFacts,
};
use nola_store_port::store_maintenance::{
    RunRecoveryPlan, RunRecoveryRecord, StoreMaintenanceRepository,
};
use sha2::{Digest, Sha256};

pub(crate) struct RecoveryOutcome {
    pub changed: usize,
}

pub(crate) fn recover<R: StoreMaintenanceRepository + ?Sized>(
    store: &mut R,
    conversation: Option<&str>,
    reason: &str,
    at: &str,
    at_ms: i64,
) -> Result<RecoveryOutcome, String> {
    recover_with_observation(
        store,
        conversation,
        reason,
        at,
        at_ms,
        None,
        None,
        None,
        None,
        false,
    )
}

pub(crate) fn recover_observed<R: StoreMaintenanceRepository + ?Sized>(
    store: &mut R,
    conversation: &str,
    expected_run_id: &str,
    host_invocation: &nola_host_protocol::HostInvocationBinding,
    session_lease: Option<&nola_store_port::workspace_ownership::StoredSessionRunLease>,
    deliveries: &[nola_agent_engine::agent::host::EffectDeliveryLease],
    at: &str,
    at_ms: i64,
) -> Result<RecoveryOutcome, String> {
    if expected_run_id.trim().is_empty() {
        return Err("orphan recovery requires an expected Run identity".into());
    }
    if (deliveries.is_empty() && session_lease.is_none()) || deliveries.len() > 256 {
        return Err("orphan recovery requires bounded observed delivery identities".into());
    }
    host_invocation.validate().map_err(str::to_string)?;
    recover_with_observation(
        store,
        Some(conversation),
        "observed_owner_process_terminated",
        at,
        at_ms,
        Some(deliveries),
        Some(expected_run_id),
        Some(host_invocation),
        session_lease,
        false,
    )
}

/// A ready schedule is an observation, not a delivery-state mutation. Reconcile
/// only this admitted Run's currently expired leases before a successor claim.
/// Unlike observed process death, expiry never authorizes interrupting a renewed
/// lease, even when its attempt/token identity has stayed the same.
pub(crate) fn recover_expired<R: StoreMaintenanceRepository + ?Sized>(
    store: &mut R,
    conversation: &str,
    expected_run_id: &str,
    host_invocation: &nola_host_protocol::HostInvocationBinding,
    at: &str,
    at_ms: i64,
) -> Result<RecoveryOutcome, String> {
    if conversation.trim().is_empty() || expected_run_id.trim().is_empty() || at_ms < 0 {
        return Err(
            "expired delivery recovery requires exact Run identity and a valid clock".into(),
        );
    }
    host_invocation.validate().map_err(str::to_string)?;
    recover_with_observation(
        store,
        Some(conversation),
        "recover_expired_delivery_lease",
        at,
        at_ms,
        None,
        Some(expected_run_id),
        Some(host_invocation),
        None,
        true,
    )
}

fn recover_with_observation<R: StoreMaintenanceRepository + ?Sized>(
    store: &mut R,
    conversation: Option<&str>,
    reason: &str,
    at: &str,
    at_ms: i64,
    observed: Option<&[nola_agent_engine::agent::host::EffectDeliveryLease]>,
    expected_run_id: Option<&str>,
    host_invocation: Option<&nola_host_protocol::HostInvocationBinding>,
    observed_session_lease: Option<&nola_store_port::workspace_ownership::StoredSessionRunLease>,
    expired_only: bool,
) -> Result<RecoveryOutcome, String> {
    let mut inspected_expected = false;
    let changed = store
        .with_run_recovery_plan(conversation, reason, at, at_ms, |record| {
            let mut verified_configuration = None;
            if let Some(expected) = expected_run_id {
                if record.run.as_ref().map(|run| run.run_id.as_str()) != Some(expected) {
                    return Err(
                        "orphan recovery expected Run identity changed; observe again".into(),
                    );
                }
                inspected_expected = true;
                let config = verified_record_configuration(record)?;
                config.turn_identity.host_invocation.as_ref()
                    .ok_or("Host invocation unavailable: missing frozen binding")?
                    .require_exact(host_invocation.ok_or("Host invocation unavailable: missing recovery binding")?)
                    .map_err(str::to_string)?;
                verified_configuration = Some(config);
                if record.session_lease.as_ref() != observed_session_lease {
                    return Err(
                        "orphan recovery session lease ownership changed; observe again".into(),
                    );
                }
            }
            if expired_only {
                if record.has_session_lease || record.session_lease.is_some() {
                    return Err(
                        "expired delivery recovery cannot replace a session lease owner".into(),
                    );
                }
                // The Store's current expiry is authoritative. A schedule read
                // before a heartbeat cannot authorize interrupting that renewal.
                if !record.effect_delivery_attempts.iter().any(|attempt| {
                    matches!(
                        attempt.state,
                        EffectDeliveryAttemptState::Leased
                            | EffectDeliveryAttemptState::ExecutionStarted
                    ) && attempt
                        .lease_expires_at_ms
                        .is_some_and(|expiry| expiry <= at_ms)
                }) {
                    return Ok(RunRecoveryPlan {
                        release_session_lease: None,
                        effect_delivery_attempt_mutations: Vec::new(),
                        delegated_permission_mutations: Vec::new(),
                        preserve_all_traces: true,
                        preserve_trace_turn_id: None,
                    });
                }
            }
            // Facts change only in the plan. Store applies the CAS release and
            // all subsequent mutations together, or rolls everything back.
            let mut effective_record;
            let record = if observed_session_lease.is_some() {
                effective_record = record.clone();
                effective_record.has_session_lease = false;
                effective_record.session_lease = None;
                &effective_record
            } else {
                record
            };
            // Only session-lease observations above differ from the original
            // record. Reuse its verified immutable configuration and materialize
            // the checkpoint once for all planners in this transaction.
            let checkpoint = RecoveryCheckpoint {
                record, configuration: verified_configuration,
                value: std::cell::OnceCell::new(),
            };
            if let Some(observed) = observed {
                // OS liveness is observed by Host. Application fences that
                // observation against the current identities in this write
                // transaction, never against a stale checkpoint alone.
                if record.has_session_lease
                    || observed.iter().any(|lease| {
                        !record.effect_delivery_attempts.iter().any(|attempt| {
                            attempt.id == lease.attempt_id
                                && attempt.worker_generation.as_deref()
                                    == Some(lease.worker_generation.as_str())
                                && attempt.lease_owner.as_deref()
                                    == Some(lease.lease_owner.as_str())
                                && attempt.lease_token.as_deref()
                                    == Some(lease.lease_token.as_str())
                                && attempt.lease_expires_at_ms == Some(lease.lease_expires_at_ms)
                        })
                    })
                    || record.effect_delivery_attempts.iter().any(|attempt| {
                        !attempt.state.is_terminal()
                            && !observed.iter().any(|lease| lease.attempt_id == attempt.id)
                    })
                {
                    return Err("orphan recovery delivery ownership changed; observe again".into());
                }
            }
            // Expiry proves only individual physical leases stale; it is not
            // evidence of whole-Run/process death or a terminal transition.
            let mut plan = if expired_only {
                RunRecoveryPlan {
                    release_session_lease: None,
                    effect_delivery_attempt_mutations: Vec::new(),
                    delegated_permission_mutations: Vec::new(),
                    preserve_all_traces: true,
                    preserve_trace_turn_id: None,
                }
            } else {
                plan_record(record, &checkpoint)?
            };
            plan.release_session_lease = observed_session_lease.cloned();
            if !record.has_session_lease {
                for delivery in record
                    .delegated_permissions
                    .iter()
                    .filter(|_| !expired_only)
                {
                    crate::delegated_permission::validate_recovery_record(delivery, record)?;
                    if matches!(
                        delivery.state,
                        nola_domain::DelegatedPermissionDeliveryState::AwaitingDecision
                            | nola_domain::DelegatedPermissionDeliveryState::Prepared
                            | nola_domain::DelegatedPermissionDeliveryState::Claimed
                    ) {
                        plan.delegated_permission_mutations.push(
                            nola_kernel::interrupt_delegated_permission(delivery, at_ms)?,
                        );
                    }
                }
                plan.effect_delivery_attempt_mutations =
                    plan_effect_delivery_recovery(record, at_ms, observed.is_some(), expired_only, &checkpoint)?;
            }
            Ok(plan)
        })
        .map_err(|error| error.to_string())?;
    if expected_run_id.is_some() && !inspected_expected {
        return Err("orphan recovery expected Run is absent; inspect again".into());
    }
    Ok(RecoveryOutcome { changed })
}

fn delivery_replay_safety(value: HostReplaySafety) -> EffectDeliveryReplaySafety {
    match value {
        HostReplaySafety::Idempotent => EffectDeliveryReplaySafety::Idempotent,
        HostReplaySafety::Recoverable => EffectDeliveryReplaySafety::Recoverable,
        HostReplaySafety::RequiresRecoveryDecision => {
            EffectDeliveryReplaySafety::RequiresRecoveryDecision
        }
    }
}

fn delivery_lease_identity(
    attempt: &nola_domain::EffectDeliveryAttempt,
) -> Result<(&str, &str, &str), String> {
    Ok((
        attempt
            .worker_generation
            .as_deref()
            .ok_or("effect delivery recovery has no worker generation")?,
        attempt
            .lease_owner
            .as_deref()
            .ok_or("effect delivery recovery has no lease owner")?,
        attempt
            .lease_token
            .as_deref()
            .ok_or("effect delivery recovery has no lease token")?,
    ))
}

fn plan_effect_delivery_recovery(
    record: &RunRecoveryRecord,
    at_ms: i64,
    observed_owner_death: bool,
    expired_only: bool,
    checkpoint: &RecoveryCheckpoint<'_>,
) -> Result<Vec<EffectDeliveryAttemptMutation>, String> {
    if record.effect_delivery_attempts.is_empty() {
        return Ok(Vec::new());
    }
    let run = record
        .run
        .as_ref()
        .ok_or("effect delivery recovery has no canonical Run")?;
    let checkpoint = checkpoint.get()?;
    let journal = &checkpoint.state.host.pending_effects;
    let mut active_effects = std::collections::BTreeSet::new();
    let mut mutations = Vec::new();

    for attempt in &record.effect_delivery_attempts {
        nola_kernel::validate_effect_delivery_attempt(attempt)?;
        if attempt.run_id != run.run_id {
            return Err("effect delivery recovery Run identity mismatch".into());
        }
        if matches!(
            attempt.state,
            EffectDeliveryAttemptState::Prepared
                | EffectDeliveryAttemptState::Leased
                | EffectDeliveryAttemptState::ExecutionStarted
        ) && !active_effects.insert(attempt.effect_id.as_str())
        {
            return Err("effect delivery recovery has multiple active attempts".into());
        }

        // Terminal evidence is immutable. It remains in the recovery snapshot
        // so repeated startup can observe the decision without replaying it.
        if attempt.state.is_terminal() {
            continue;
        }
        if expired_only
            && !(matches!(
                attempt.state,
                EffectDeliveryAttemptState::Leased | EffectDeliveryAttemptState::ExecutionStarted
            ) && attempt
                .lease_expires_at_ms
                .is_some_and(|expiry| expiry <= at_ms))
        {
            continue;
        }

        // A committed cancellation withdraws its business target from
        // executable outbox work immediately, not from the physical ledger.
        // Keep that target for the exact cancellation-only reconciliation;
        // neither expiry nor startup may schedule its business action again.
        if let Some(target) = journal.get(&attempt.effect_id).filter(|target| {
            target.state == nola_agent_engine::agent::recovery::PendingHostEffectState::Cancelling
        }) {
            let request = target.validated_request()?;
            let control_id = target
                .cancel_effect_id
                .as_deref()
                .ok_or("cancelling delivery has no exact control identity")?;
            let control = journal
                .get(control_id)
                .ok_or("cancelling delivery control is absent from checkpoint")?;
            let frozen_control = control.validated_request()?;
            let matching = record
                .effects
                .iter()
                .filter(|effect| {
                    effect.effect_id == control_id && effect.request_id == frozen_control.request_id
                })
                .collect::<Vec<_>>();
            if matching.len() != 1
                || matching[0].run_id != run.run_id
                || matching[0].status != "pending"
                || request.request_id != attempt.request_id
                || request.session_id != checkpoint.session_id
                || request.turn_id != frozen_control.turn_id
                || frozen_control.session_id != checkpoint.session_id
                || delivery_replay_safety(target.replay_safety) != attempt.replay_safety
                || !matches!(&frozen_control.payload, HostRequest::Cancel { target_effect_id, .. } if target_effect_id.0 == attempt.effect_id)
            {
                return Err(
                    "cancelling delivery does not own an exact pending control/target pair".into(),
                );
            }
            let persisted_control: HostRequestEnvelope =
                serde_json::from_str(&matching[0].payload_json)
                    .map_err(|_| "cancelling delivery control request is invalid")?;
            if frozen_control != persisted_control {
                return Err("cancelling delivery control differs from its frozen request".into());
            }
            continue;
        }

        let effects = record
            .effects
            .iter()
            .filter(|effect| {
                effect.effect_id == attempt.effect_id
            })
            .collect::<Vec<_>>();
        if effects.len() != 1 {
            return Err("effect delivery recovery has no unique pending request".into());
        }
        let effect = effects[0];
        if effect.run_id != run.run_id || effect.status != "pending" {
            return Err("effect delivery recovery request ownership mismatch".into());
        }
        let request: HostRequestEnvelope = serde_json::from_str(&effect.payload_json)
            .map_err(|_| "effect delivery recovery request is invalid")?;
        let pending = journal
            .get(&attempt.effect_id)
            .ok_or("effect delivery recovery request is absent from checkpoint")?;
        // Core may rebind a transport request while the same physical delivery
        // is still recorded. The current immutable action + exact delivery lease
        // is the ownership proof, not the superseded transport request ID.
        let same_delivery = request.delivery.as_ref().is_some_and(|delivery| {
            delivery.attempt_id == attempt.id
                && Some(delivery.worker_generation.as_str()) == attempt.worker_generation.as_deref()
                && Some(delivery.lease_owner.as_str()) == attempt.lease_owner.as_deref()
                && Some(delivery.lease_token.as_str()) == attempt.lease_token.as_deref()
        });
        if request.effect_id.0 != attempt.effect_id
            || request.request_id != effect.request_id
            || (request.request_id != attempt.request_id && !same_delivery)
            || pending.effect_id != attempt.effect_id
            || pending.request_id != request.request_id
            || delivery_replay_safety(pending.replay_safety) != attempt.replay_safety
        {
            return Err("effect delivery recovery attempt/request identity mismatch".into());
        }

        let (generation, owner, token) = delivery_lease_identity(attempt)?;
        match attempt.state {
            EffectDeliveryAttemptState::Prepared => {
                return Err("prepared effect delivery has no durable lease claim".into())
            }
            EffectDeliveryAttemptState::Leased => {
                if observed_owner_death || at_ms >= attempt.lease_expires_at_ms.unwrap_or(i64::MAX)
                {
                    mutations.push(nola_kernel::interrupt_effect_delivery_attempt(
                        attempt,
                        generation,
                        owner,
                        token,
                        EffectDeliveryRecoveryPlan::Retry,
                        if observed_owner_death {
                            "worker disappeared before execution started"
                        } else {
                            "worker lease expired before execution started"
                        },
                        at_ms,
                    )?);
                }
            }
            EffectDeliveryAttemptState::ExecutionStarted => match attempt.replay_safety {
                EffectDeliveryReplaySafety::Idempotent => {
                    mutations.push(nola_kernel::interrupt_effect_delivery_attempt(
                        attempt,
                        generation,
                        owner,
                        token,
                        EffectDeliveryRecoveryPlan::Retry,
                        "worker disappeared after idempotent execution started",
                        at_ms,
                    )?);
                }
                EffectDeliveryReplaySafety::Recoverable => {
                    // The stored outbox is the immutable request body. Core's
                    // checkpoint owns the accepted response cursor, and the
                    // executable request is materialized by overlaying that
                    // cursor exactly as the normal load path does.
                    let mut recovery_request = request.clone();
                    recovery_request.resume_after_sequence = pending.last_response_sequence;
                    // Zero is the canonical cursor when execution crossed the
                    // start-ACK barrier but the provider produced no response
                    // frame before the worker disappeared.
                    recovery_request
                        .resume_after_sequence
                        .ok_or("recoverable effect delivery has no canonical resume cursor")?;
                    mutations.push(nola_kernel::interrupt_effect_delivery_attempt(
                        attempt,
                        generation,
                        owner,
                        token,
                        EffectDeliveryRecoveryPlan::Resume,
                        "worker disappeared after recoverable execution started",
                        at_ms,
                    )?);
                }
                EffectDeliveryReplaySafety::RequiresRecoveryDecision => {
                    if let nola_agent_engine::agent::host::HostRequest::RequestPermission { permission_request_id, .. } = &request.payload {
                        if attempt.lease_expires_at_ms.is_some_and(|expiry| expiry <= at_ms) {
                            let awaiting = checkpoint.state.execution.permissions.pending.iter().any(|ask|
                                ask.request_id.as_str() == permission_request_id
                                    && Some(ask.turn_id.as_str()) == request.turn_id.as_deref());
                            if !awaiting { return Err("permission redisplay has no original pending approval".into()); }
                            mutations.push(nola_kernel::authorize_pending_permission_redisplay(
                                attempt, permission_request_id, at_ms)?);
                            continue;
                        }
                    }
                    mutations.push(nola_kernel::mark_effect_delivery_outcome_unknown(
                        attempt,
                        generation,
                        owner,
                        token,
                        "worker disappeared after non-replayable execution started",
                        at_ms,
                    )?);
                }
            },
            EffectDeliveryAttemptState::Settled
            | EffectDeliveryAttemptState::Interrupted
            | EffectDeliveryAttemptState::OutcomeUnknown => unreachable!("terminal state"),
        }
    }
    Ok(mutations)
}

fn plan_record(record: &RunRecoveryRecord, checkpoint: &RecoveryCheckpoint<'_>) -> Result<RunRecoveryPlan, String> {
    let external_run = record.run.as_ref().is_some_and(|run| {
        run.invocation_config_json.is_none()
            && run.invocation_config_digest.is_none()
            && record.effects.is_empty()
    });
    let mut facts = RunRecoveryFacts {
        has_session_lease: record.has_session_lease,
        has_canonical_run: record.run.is_some(),
        external_run,
        checkpoint_verified: false,
        turn_status: None,
        pending_effect_count: record.effects.len(),
        open_user_input_wait_count: record.open_user_input_waits.len(),
    };
    let mut turn_id = None;
    // Do not parse, normalize or repair another live owner's state. Remote
    // Runs remain owned by their remote execution and have no local outbox.
    if record.run.is_some() && !record.has_session_lease && !external_run {
        let (id, status) = verified_current_turn(record, checkpoint).map_err(|reason| {
            format!(
                "Run recovery rejected for {}: {reason}",
                record.run.as_ref().unwrap().run_id
            )
        })?;
        turn_id = Some(id);
        facts.turn_status = Some(status);
        facts.checkpoint_verified = true;
    }
    let action = nola_kernel::plan_run_recovery(&facts);
    let preserve_all_traces = nola_kernel::preserve_recovery_trace(&TraceRecoveryFacts {
        has_session_lease: record.has_session_lease,
        external_run,
        matches_preserved_current_turn: false,
    });
    match action {
        RunRecoveryAction::Reject(reason) => Err(format!("Run recovery rejected: {reason:?}")),
        RunRecoveryAction::Preserve => Ok(RunRecoveryPlan {
            release_session_lease: None,
            effect_delivery_attempt_mutations: Vec::new(),
            delegated_permission_mutations: Vec::new(),
            preserve_all_traces,
            preserve_trace_turn_id: turn_id,
        }),
    }
}

// Local to one recovery-plan record, never retained across a transaction or
// a changed Store snapshot. All planners inspect the same verified Core state.
struct RecoveryCheckpoint<'a> {
    record: &'a RunRecoveryRecord,
    configuration: Option<AgentCoreConfig>,
    value: std::cell::OnceCell<Result<CoreCheckpoint<AgentThreadState>, String>>,
}
impl RecoveryCheckpoint<'_> {
    fn get(&self) -> Result<&CoreCheckpoint<AgentThreadState>, String> {
        self.value.get_or_init(|| recovered_record_checkpoint(self.record, self.configuration.as_ref()))
            .as_ref().map_err(Clone::clone)
    }
}

fn verified_record_configuration(record: &RunRecoveryRecord) -> Result<AgentCoreConfig, String> {
    let run = record.run.as_ref().ok_or("missing canonical Run")?;
    let config_json = run
        .invocation_config_json
        .as_ref()
        .ok_or("missing invocation configuration")?;
    let digest = run
        .invocation_config_digest
        .as_ref()
        .ok_or("missing invocation digest")?;
    if &format!("{:x}", Sha256::digest(config_json.as_bytes())) != digest {
        return Err("invocation configuration integrity mismatch".into());
    }
    serde_json::from_str(config_json).map_err(|_| "invalid invocation configuration".into())
}


fn recovered_record_checkpoint(
    record: &RunRecoveryRecord,
    configuration: Option<&AgentCoreConfig>,
) -> Result<CoreCheckpoint<AgentThreadState>, String> {
    let run = record.run.as_ref().ok_or("missing canonical Run")?;
    let recovery = record.checkpoint.as_ref().ok_or("missing checkpoint")?;
    let stored = &recovery.checkpoint;
    if recovery
        .replay_base
        .as_ref()
        .is_some_and(|base| base.run_id != run.run_id)
    {
        return Err("recovery checkpoint belongs to another Run".into());
    }
    let mut config = match configuration {
        Some(config) => config.clone(),
        None => verified_record_configuration(record)?,
    };
    let checkpoint: CoreCheckpoint<AgentThreadState> =
        serde_json::from_str(&stored.checkpoint_json).map_err(|_| "invalid checkpoint")?;
    checkpoint.validate_state_identity()?;
    crate::delegated_invocation::bind_restore_configuration(
        &mut config,
        &checkpoint,
        &run.run_id,
        &record.delegated_invocations,
    )?;
    let core = AgentCore::recover(config, checkpoint)
        .map_err(|error| format!("Core rejected recovery checkpoint: {error}"))?;
    let core = crate::replay_recovery_inputs(core, recovery)?;
    let checkpoint =
        core.recovery_checkpoint(nola_agent_engine::agent::recovery::CheckpointReason::Manual);
    if record.conversation_id.trim().is_empty()
        || stored.conversation_id != record.conversation_id
        || checkpoint.session_id.trim().is_empty()
        || checkpoint.state.session_id.as_str() != checkpoint.session_id
        || i64::try_from(checkpoint.last_event_cursor.sequence).ok() != Some(stored.event_cursor)
    {
        return Err("checkpoint owner or revision mismatch".into());
    }
    Ok(checkpoint)
}

fn verified_current_turn(record: &RunRecoveryRecord, checkpoint: &RecoveryCheckpoint<'_>) -> Result<(String, AgentTurnStatus), String> {
    let run = record.run.as_ref().ok_or("missing canonical Run")?;
    let checkpoint = checkpoint.get()?;
    let turn = checkpoint
        .state
        .execution
        .active_turn
        .as_ref()
        .ok_or("missing current Core Turn")?;
    let turn_id = turn.turn_id.as_str().to_owned();
    // BeginAssistantTurn uses its accepted first Core Turn ID as the Run ID.
    // Later Turns are registered by their committed TurnStarted event.
    let owned_current_turn = match run.latest_started_turn_id.as_deref() {
        Some(latest) => latest == turn_id,
        None => run.run_id == turn_id,
    };
    if !owned_current_turn {
        return Err("current Core Turn is not the Run's latest started Turn".into());
    }
    let status = turn.status;
    let open_waits = checkpoint
        .state
        .execution
        .user_input_waits
        .values()
        .filter(|wait| wait.is_open())
        .collect::<Vec<_>>();
    if open_waits.len() != record.open_user_input_waits.len() {
        return Err("checkpoint/Store open user wait set mismatch".into());
    }
    let mut wait_ids = std::collections::BTreeSet::new();
    for stored in &record.open_user_input_waits {
        stored.wait.validate()?;
        if stored.conversation_id != record.conversation_id
            || stored.run_id != run.run_id
            || stored.wait.core_session_id != checkpoint.session_id
            || stored.wait.turn_id != turn_id
            || !stored.wait.is_open()
            || !wait_ids.insert(&stored.wait.wait_id)
            || !open_waits.iter().any(|wait| *wait == &stored.wait)
        {
            return Err("checkpoint/Store open user wait ownership mismatch".into());
        }
    }
    if !open_waits.is_empty() && status.is_terminal() {
        return Err("terminal checkpoint retains open user waits".into());
    }
    for invocation in &record.delegated_invocations {
        if !invocation.state.is_terminal() && !record.effects.iter().any(|effect| {
            if effect.effect_id == invocation.effect_id { return true }
            let Ok(request) = serde_json::from_str::<HostRequestEnvelope>(&effect.payload_json) else { return false };
            invocation.state == nola_domain::DelegatedInvocationState::Cancelling
                && matches!(request.payload, HostRequest::Cancel { ref target_effect_id, .. } if target_effect_id.0 == invocation.effect_id)
        }) {
            return Err("active delegated invocation has no pending delivery".into());
        }
    }
    match run.active_attempt_id.as_deref() {
        Some(id) => {
            let matching = record
                .model_attempts
                .iter()
                .filter(|attempt| attempt.id == id)
                .collect::<Vec<_>>();
            if matching.len() != 1
                || matching[0].run_id != run.run_id
                || matching[0].state.is_terminal()
                || !record
                    .effects
                    .iter()
                    .any(|effect| effect.effect_id == matching[0].effect_id)
            {
                return Err("active model Attempt has no matching pending effect".into());
            }
        }
        None if record
            .model_attempts
            .iter()
            .any(|attempt| !attempt.state.is_terminal()) =>
        {
            return Err("pending model Attempt is not active in its Run".into());
        }
        None => {}
    }
    let journal = &checkpoint.state.host.pending_effects;
    let outbox_effect_ids = record
        .effects
        .iter()
        .map(|effect| effect.effect_id.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    // Core can retain a non-executable journal target for two explicit
    // protocols: durable Task promotion and two-phase Host cancellation.
    // Every other journal/outbox divergence remains corruption.
    for pending in journal.iter() {
        if outbox_effect_ids.contains(pending.effect_id.as_str()) {
            continue;
        }
        // Interrupting a Turn retires the original executable request and
        // durably enqueues an idempotent host.cancel request. Core keeps the
        // retired target in `cancelling` until that control request is
        // acknowledged, so the target is intentionally absent from the
        // executable outbox while the Turn remains nonterminal.
        let has_pending_cancel = pending
            .cancel_effect_id
            .as_deref()
            .is_some_and(|cancel_id| {
                record.effects.iter().any(|effect| {
                    if effect.effect_id != cancel_id {
                        return false;
                    }
                    serde_json::from_str::<HostRequestEnvelope>(&effect.payload_json).is_ok_and(
                        |request| {
                            matches!(
                                request.payload,
                                HostRequest::Cancel { ref target_effect_id, .. }
                                    if target_effect_id.0 == pending.effect_id
                            )
                        },
                    )
                })
            });
        if has_pending_cancel {
            continue;
        }
        let Some(task_id) = pending.task_id.as_deref() else {
            return Err("checkpoint/outbox effect set mismatch".into());
        };
        let Some(task) = checkpoint.state.task_projection.tasks.get(task_id) else {
            return Err("promoted Host effect has no matching Task projection".into());
        };
        if task.owner_turn_id.as_str() != turn_id {
            return Err("promoted Task belongs to another Core Turn".into());
        }
    }
    if outbox_effect_ids
        .iter()
        .any(|effect_id| journal.get(*effect_id).is_none())
    {
        return Err("checkpoint/outbox effect set mismatch".into());
    }
    let mut ids = std::collections::BTreeSet::new();
    for effect in &record.effects {
        if effect.run_id != run.run_id || !ids.insert(&effect.effect_id) {
            return Err("pending effect Run identity mismatch".into());
        }
        let request: HostRequestEnvelope = serde_json::from_str(&effect.payload_json)
            .map_err(|_| "invalid pending Host request")?;
        let entry = journal
            .get(&effect.effect_id)
            .ok_or("pending effect absent from checkpoint")?;
        if request.effect_id.0 != effect.effect_id
            || request.request_id != effect.request_id
            || request.session_id != checkpoint.session_id
            || entry.effect_id != effect.effect_id
            || entry.request_id != request.request_id
            || entry.host_id != request.host_id.0
            || entry.lease_id != request.lease_id
            || entry.session_id != request.session_id
            || entry.turn_id != request.turn_id
            || entry.catalog_revision != request.catalog_revision
            || entry.availability_epoch != request.availability_epoch
        {
            return Err("pending Host request/checkpoint identity mismatch".into());
        }
        let mut checkpoint_request: HostRequestEnvelope = serde_json::from_value(
            entry
                .request
                .clone()
                .ok_or("checkpoint effect has no request")?,
        )
        .map_err(|_| "invalid checkpoint Host request")?;
        let mut durable_request = request.clone();
        // Delivery overlays its cursor from the durable journal, not the
        // initial outbox wire copy. The application also overlays the
        // delivery lease when a worker claims the effect; that fencing
        // identity belongs to the delivery-attempt ledger, not Core's
        // checkpoint payload. Everything else must be identical.
        checkpoint_request.resume_after_sequence = None;
        durable_request.resume_after_sequence = None;
        checkpoint_request.delivery = None;
        durable_request.delivery = None;
        if crate::model_attempt::is_model_request(&request) {
            let attempts = record
                .model_attempts
                .iter()
                .filter(|attempt| attempt.effect_id == effect.effect_id)
                .collect::<Vec<_>>();
            if attempts.len() != 1 {
                return Err("model recovery requires one current Attempt per effect".into());
            }
            let attempt = attempts[0];
            nola_kernel::validate_model_attempt(attempt)?;
            if attempt.run_id != run.run_id
                || attempt.state.is_terminal()
                || request.attempt_id.as_deref() != Some(attempt.id.as_str())
                || request.attempt_started != attempt.started_at_ms.is_some()
            {
                return Err("pending model Attempt ledger identity mismatch".into());
            }
            // Attempt admission belongs to Application. The Core checkpoint
            // retains reducer delivery state; compare its remaining payload
            // only after independently validating the canonical ledger owner.
            checkpoint_request.attempt_started = false;
            durable_request.attempt_started = false;
            checkpoint_request.attempt_id = None;
            durable_request.attempt_id = None;
        }
        if crate::delegated_invocation::is_delegated(&request) {
            let invocations = record
                .delegated_invocations
                .iter()
                .filter(|item| item.effect_id == effect.effect_id)
                .collect::<Vec<_>>();
            if invocations.len() != 1 {
                return Err("delegated recovery requires exactly one canonical invocation".into());
            }
            let invocation = invocations[0];
            nola_kernel::validate_delegated_invocation(invocation)?;
            if invocation.run_id != run.run_id
                || invocation.state.is_terminal()
                || request.delegated_invocation_id.as_deref() != Some(invocation.id.as_str())
            {
                return Err("pending delegated invocation identity mismatch".into());
            }
            checkpoint_request.delegated_invocation_id = None;
            durable_request.delegated_invocation_id = None;
        }
        if crate::delegated_permission::is_control(&request) {
            let deliveries = record
                .delegated_permissions
                .iter()
                .filter(|delivery| {
                    delivery.control_effect_id.as_deref() == Some(effect.effect_id.as_str())
                })
                .collect::<Vec<_>>();
            if deliveries.len() != 1 {
                return Err("permission control recovery requires one canonical delivery".into());
            }
            crate::delegated_permission::validate_delivery_record(
                deliveries[0],
                &run.run_id,
                &request,
            )?;
            if let HostRequest::ResolveDelegatedPermission { delivery, .. } =
                &mut checkpoint_request.payload
            {
                *delivery = None;
            }
            if let HostRequest::ResolveDelegatedPermission { delivery, .. } =
                &mut durable_request.payload
            {
                *delivery = None;
            }
        }
        if let HostRequest::Cancel { target_not_started: Some(proof), .. } = &durable_request.payload {
            if proof.conversation_id != record.conversation_id || proof.run_id != run.run_id
                || proof.checkpoint_revision > checkpoint.revision as i64 {
                return Err("pending cancellation NotStarted owner mismatch".into());
            }
            // Only the admitted proof is absent from Core's frozen request.
            // Preserve every identity field for the comparison below.
            let mut proof_binding = nola_domain::host_control::CoreHostBindingOverlay::from_request(&checkpoint_request);
            proof_binding.payload_binding =
                nola_domain::host_control::CoreHostBindingOverlay::from_request(&durable_request).payload_binding;
            proof_binding.apply_to(&mut checkpoint_request)?;
        }
        if serde_json::to_value(checkpoint_request).map_err(|e| e.to_string())?
            != serde_json::to_value(durable_request).map_err(|e| e.to_string())?
        {
            return Err("pending Host request payload mismatch".into());
        }
        if let HostRequest::RequestPermission {
            allow_conversation,
            permission_request_id,
            capability_id,
            resource,
            input,
            permission_instructions,
            reason,
            evaluation,
        } = &request.payload
        {
            let matches = checkpoint
                .state
                .execution
                .permissions
                .pending
                .iter()
                .any(|pending| {
                    let mut arguments = pending.arguments.clone();
                    if let Some(context) = &pending.invocation_context {
                        let Some(object) = arguments.as_object_mut() else {
                            return false;
                        };
                        let Ok(value) = serde_json::to_value(context) else {
                            return false;
                        };
                        object.insert("_nolaExecution".into(), value);
                    }
                    pending.request_id.as_str() == permission_request_id
                        && pending.allow_conversation == *allow_conversation
                        && Some(pending.turn_id.as_str()) == request.turn_id.as_deref()
                        && Some(pending.call_id.as_str()) == entry.call_id.as_deref()
                        && &pending.capability_id == capability_id
                        && &pending.resource == resource
                        && &arguments == input
                        && &pending.permission_instructions == permission_instructions
                        && &pending.rationale == reason
                        && Some(&pending.evaluation) == evaluation.as_ref()
                });
            if !matches {
                return Err("permission checkpoint/request mismatch".into());
            }
        }
    }
    Ok((turn_id, status))
}
