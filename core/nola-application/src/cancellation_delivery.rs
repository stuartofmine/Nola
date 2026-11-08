// SPDX-License-Identifier: Apache-2.0
//! Cancellation-only delivery admission. This does not infer OS liveness.
use nola_domain::EffectDeliveryAttempt;
use nola_agent_engine::agent::{
    host::{HostRequest, HostRequestEnvelope},
    recovery::CoreCheckpoint,
    AgentThreadState, AgentTurnStatus,
};
use nola_store_port::{
    cancellation_delivery::CancellationDeliveryRepository,
    effect_delivery::EffectDeliveryRepository, run_effect::RunEffectRepository,
    workspace_ownership::StoredSessionRunLease,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CancellationRunIdentity {
    pub conversation_id: String,
    pub expected_run_id: String,
    pub assistant_message_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CancellationTargetFact {
    /// Derived only from the exact Turn's frozen Host descriptor, never from tool names.
    #[serde(default)]
    pub process_bound: bool,
    pub cancellation: HostRequestEnvelope,
    pub cancellation_attempt: Option<EffectDeliveryAttempt>,
    pub target: Option<HostRequestEnvelope>,
    pub latest_attempt: Option<EffectDeliveryAttempt>,
    pub prior_attempts: Vec<EffectDeliveryAttempt>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunCancellationDeliveryFacts {
    pub identity: CancellationRunIdentity,
    pub checkpoint_revision: i64,
    pub session_lease: Option<StoredSessionRunLease>,
    pub targets: Vec<CancellationTargetFact>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimCancellationHostRequests {
    pub identity: CancellationRunIdentity,
    /// Exact persisted facts whose target owners the trusted Host observed dead.
    /// An unstarted, expired cancellation delivery can instead be fenced and
    /// reclaimed; its worker process need not exit its interactive shell.
    /// Never a caller-provided "quiescent" boolean or a registry-empty claim.
    pub observed: RunCancellationDeliveryFacts,
    pub worker_generation: String,
    pub lease_owner: String,
    pub claimed_at_ms: i64,
    pub lease_expires_at_ms: i64,
    pub limit: usize,
}

/// Expiry is sufficient only for an unstarted cancellation control delivery.
/// It never establishes quiescence of the target operation. The claim compares
/// all observed facts and replaces the delivery fence in the same transaction.
pub fn cancellation_delivery_reclaimable_by_expiry(
    attempt: &EffectDeliveryAttempt,
    now_ms: i64,
) -> bool {
    attempt.state == nola_domain::EffectDeliveryAttemptState::Leased
        && attempt.execution_started_at_ms.is_none()
        && attempt.lease_expires_at_ms.is_some_and(|expiry| expiry <= now_ms)
}

/// A Core snapshot selected by the Runtime Supervisor for this exact Store revision.
pub(crate) struct CancellationCheckpoint {
    pub(crate) conversation_id: String,
    pub(crate) revision: i64,
    pub(crate) invocation_config_digest: String,
    pub(crate) checkpoint: CoreCheckpoint<AgentThreadState>,
}

pub fn load_run_cancellation_delivery_facts<R: CancellationDeliveryRepository + ?Sized>(
    store: &R,
    identity: &CancellationRunIdentity,
) -> Result<RunCancellationDeliveryFacts, String> {
    if [
        &identity.conversation_id,
        &identity.expected_run_id,
        &identity.assistant_message_id,
    ]
    .iter()
    .any(|id| id.trim().is_empty())
    {
        return Err("cancellation requires exact Run identity".into());
    }
    load_run_cancellation_delivery_facts_with_checkpoint(store, identity, None)
}

pub(crate) fn load_run_cancellation_delivery_facts_with_checkpoint<R: CancellationDeliveryRepository + ?Sized>(
    store: &R, identity: &CancellationRunIdentity, current: Option<&CancellationCheckpoint>,
) -> Result<RunCancellationDeliveryFacts, String> {
    let snapshot = store
        .cancellation_delivery_snapshot(&identity.expected_run_id)
        .map_err(|e| e.to_string())?
        .ok_or("cancellation Run is missing")?;
    if snapshot.run.conversation_id != identity.conversation_id
        || snapshot.run.assistant_message_id.as_deref()
            != Some(identity.assistant_message_id.as_str())
        || !snapshot.run.assistant_owner_valid
        || snapshot.run.source_host_id.is_some()
        || snapshot.run.completed_at_ms.is_some()
    {
        return Err("cancellation Run ownership or active state changed".into());
    }
    let local = snapshot
        .delivery
        .checkpoint
        .ok_or("cancellation checkpoint is missing")?;
    if local.recovery.replay_base.as_ref().is_some_and(|base| base.run_id != identity.expected_run_id) {
        return Err("cancellation checkpoint belongs to another Run".into());
    }
    let (stored, checkpoint) = if let Some(current) = current {
        use sha2::{Digest, Sha256};
        // A full baseline needs no replay configuration; the Store includes
        // frozen invocation material only when a replay tail needs it.
        if let Some(invocation) = local.invocation.as_ref() {
            if invocation.run_id != identity.expected_run_id
                || invocation.invocation_config_digest != current.invocation_config_digest
                || format!("{:x}", Sha256::digest(invocation.invocation_config_json.as_bytes())) != current.invocation_config_digest {
                return Err("cancellation hot checkpoint frozen invocation changed".into());
            }
        } else if !local.recovery.replay_inputs.is_empty() {
            return Err("cancellation hot checkpoint replay has no frozen invocation".into());
        }
        if current.conversation_id != identity.conversation_id
            || current.revision != local.recovery.checkpoint.revision
            || i64::try_from(current.checkpoint.last_event_cursor.sequence).ok()
                != Some(local.recovery.checkpoint.event_cursor) {
            return Err("cancellation hot checkpoint no longer matches durable frontier".into());
        }
        (local.recovery.checkpoint, current.checkpoint.clone())
    } else {
        let stored = crate::session_projection::materialize_local_recovery(local)?;
        let checkpoint = serde_json::from_str(&stored.checkpoint_json).map_err(|e| e.to_string())?;
        (stored, checkpoint)
    };
    checkpoint.validate_state_identity()?;
    if snapshot.delivery.conversation_id.as_deref() != Some(identity.conversation_id.as_str())
        || stored.conversation_id != identity.conversation_id
        || checkpoint.session_id.trim().is_empty()
        || checkpoint.state.session_id.as_str() != checkpoint.session_id
        || i64::try_from(checkpoint.last_event_cursor.sequence).ok() != Some(stored.event_cursor)
    {
        return Err("cancellation checkpoint ownership, session or cursor mismatch".into());
    }
    if checkpoint
        .state
        .execution
        .active_turn
        .as_ref()
        .map(|turn| turn.status)
        != Some(AgentTurnStatus::Cancelling)
    {
        return Err("cancellation delivery requires committed Interrupt".into());
    }
    let mut targets = Vec::new();
    for effect in snapshot
        .delivery
        .effects
        .iter()
        .filter(|effect| effect.status == "pending")
    {
        let request: HostRequestEnvelope =
            serde_json::from_str(&effect.payload_json).map_err(|e| e.to_string())?;
        if request.session_id != checkpoint.session_id || effect.run_id != identity.expected_run_id
            || request.effect_id.0 != effect.effect_id || request.request_id != effect.request_id
        {
            return Err("cancellation outbox identity mismatch".into());
        }
        if !matches!(&request.payload, HostRequest::Cancel { .. } | HostRequest::CancelTask { .. }) {
            continue;
        }
        let mut frozen = checkpoint.state.host.pending_effects.get(&effect.effect_id)
            .ok_or("cancellation request is absent from recovered Core")?.validated_request()?;
        // Application admits only the no-start proof here. Never copy durable
        // envelope identity over Core's frozen witness before comparing it.
        let mut proof_binding = nola_domain::host_control::CoreHostBindingOverlay::from_request(&frozen);
        proof_binding.payload_binding =
            nola_domain::host_control::CoreHostBindingOverlay::from_request(&request).payload_binding;
        proof_binding.apply_to(&mut frozen)?;
        if frozen.request_id != request.request_id || frozen.session_id != request.session_id
            || frozen.turn_id != request.turn_id || frozen.host_id != request.host_id
            || frozen.lease_id != request.lease_id || frozen.catalog_revision != request.catalog_revision
            || frozen.availability_epoch != request.availability_epoch
            || frozen.delegated_invocation_id != request.delegated_invocation_id
            || frozen.attempt_id != request.attempt_id
            || frozen.attempt_started != request.attempt_started
            || frozen.created_at_ms != request.created_at_ms
            || frozen.deadline_ms != request.deadline_ms
            || frozen.batch != request.batch
            || frozen.payload != request.payload
        {
            return Err("cancellation request differs from its frozen Core identity or payload".into());
        }
        let target = match &request.payload {
            HostRequest::Cancel { target_effect_id, .. } => {
                let pending = checkpoint
                    .state
                    .host
                    .pending_effects
                    .get(&target_effect_id.0)
                    .ok_or("cancellation target is absent from checkpoint")?;
                Some(pending.validated_request()?)
            }
            HostRequest::CancelTask { .. } => None,
            _ => continue,
        };
        if target
            .as_ref()
            .is_some_and(|target| target.session_id != checkpoint.session_id)
        {
            return Err("cancellation target Core session mismatch".into());
        }
        let latest_attempt = target
            .as_ref()
            .and_then(|target| {
                snapshot
                    .attempts
                    .iter()
                    .find(|attempt| attempt.effect_id == target.effect_id.0)
            })
            .cloned();
        let prior_attempts = target.as_ref().map(|target| snapshot.attempts.iter()
            .filter(|attempt| attempt.effect_id == target.effect_id.0
                && latest_attempt.as_ref().is_none_or(|latest| latest.id != attempt.id))
            .cloned().collect::<Vec<_>>()).unwrap_or_default();
        if let Some(target) = &target {
            let attempts = latest_attempt.iter().chain(prior_attempts.iter()).collect::<Vec<_>>();
            for attempt in &attempts {
                nola_kernel::validate_effect_delivery_attempt(attempt)?;
                if attempt.run_id != identity.expected_run_id {
                    return Err("cancellation target attempt Run mismatch".into());
                }
            }
            // A presentation can retain an older transport lease after a successor
            // attempt. Prove that exact binding against history; the Host must
            // observe EVERY physical owner dead, not only the newest owner.
            if !attempts.is_empty() && !attempts.iter().any(|attempt| {
                attempt.request_id == target.request_id || target.delivery.as_ref().is_some_and(|delivery| {
                    delivery.attempt_id == attempt.id
                        && Some(delivery.worker_generation.as_str()) == attempt.worker_generation.as_deref()
                        && Some(delivery.lease_owner.as_str()) == attempt.lease_owner.as_deref()
                        && Some(delivery.lease_token.as_str()) == attempt.lease_token.as_deref()
                })
            }) {
                return Err("cancellation target attempt ownership mismatch".into());
            }
        }
        let cancellation_attempt = snapshot
            .attempts
            .iter()
            .find(|attempt| attempt.effect_id == request.effect_id.0)
            .cloned();
        if cancellation_attempt.as_ref().is_some_and(|attempt|
            attempt.run_id != identity.expected_run_id || attempt.request_id != request.request_id)
        {
            return Err("cancellation attempt ownership mismatch".into());
        }
        let process_bound = target.as_ref().is_some_and(|target| {
            let HostRequest::ExecuteTool { capability_id, .. } = &target.payload else { return false; };
            checkpoint.state.execution.active_turn.as_ref().is_some_and(|turn| {
                let projection = &turn.context.capability_projection;
                projection.host_id == target.host_id.0
                    && projection.descriptor(capability_id).is_some_and(|descriptor|
                        descriptor.route == nola_agent_engine::agent::host::HostCapabilityRoute::Tool
                        && descriptor.cancellation == nola_agent_engine::agent::host::HostCancellationMode::ProcessBound
                        && descriptor.replay_safety == nola_agent_engine::agent::host::HostReplaySafety::Idempotent)
            })
        });
        targets.push(CancellationTargetFact {
            process_bound,
            cancellation: request,
            cancellation_attempt,
            target,
            latest_attempt,
            prior_attempts,
        });
    }
    targets.sort_by(|a, b| a.cancellation.effect_id.0.cmp(&b.cancellation.effect_id.0));
    Ok(RunCancellationDeliveryFacts {
        identity: identity.clone(),
        checkpoint_revision: stored.revision,
        session_lease: snapshot.session_lease,
        targets,
    })
}

/// Parent-process death does not prove external tool, delegated child or MCP
/// side effects have stopped. Keep those targets pending for reconciliation.
pub fn supports_dead_owner_confirmation(fact: &CancellationTargetFact) -> bool {
    let Some(target) = &fact.target else {
        return matches!(fact.cancellation.payload, HostRequest::CancelTask { .. });
    };
    if matches!(&fact.cancellation.payload,
        HostRequest::Cancel { target_not_started: Some(proof), .. }
            if proof.matches_control(&fact.cancellation.host_id.0,
                &fact.cancellation.session_id, fact.cancellation.turn_id.as_deref(),
                &fact.cancellation.effect_id.0, &fact.cancellation.request_id, &target.effect_id.0)) {
        return true;
    }
    fact.process_bound || matches!(
        target.payload,
        HostRequest::ModelStream { .. } | HostRequest::RequestPermission { .. }
    )
}

pub fn claim_cancellation_host_requests<R>(
    store: &mut R,
    input: ClaimCancellationHostRequests,
) -> Result<crate::CommittedHostRequests, String>
where
    R: CancellationDeliveryRepository + RunEffectRepository + EffectDeliveryRepository + ?Sized,
{
    claim_cancellation_host_requests_with_checkpoint(store, input, None)
}

pub(crate) fn claim_cancellation_host_requests_with_checkpoint<R>(
    store: &mut R, input: ClaimCancellationHostRequests, checkpoint: Option<&CancellationCheckpoint>,
) -> Result<crate::CommittedHostRequests, String>
where R: CancellationDeliveryRepository + RunEffectRepository + EffectDeliveryRepository + ?Sized,
{
    store
        .require_cancellation_write_unit_of_work()
        .map_err(|e| e.to_string())?;
    let current = load_run_cancellation_delivery_facts_with_checkpoint(store, &input.identity, checkpoint)?;
    if current != input.observed {
        return Err("cancellation ownership changed after observation".into());
    }
    if let Some(observed) = &current.session_lease {
        if !store
            .release_observed_cancellation_session_lease(observed)
            .map_err(|e| e.to_string())?
        {
            return Err("cancellation session owner changed during claim".into());
        }
    }
    // The Host proved target quiescence independently. Reclaim only cancellation
    // deliveries: either the owner is dead or its unstarted lease has expired.
    // The exact snapshot comparison above fences a concurrent start/heartbeat.
    let mut interruptions = Vec::new();
    for fact in &current.targets {
        if let Some(attempt) = &fact.cancellation_attempt {
            use nola_domain::EffectDeliveryAttemptState as State;
            if matches!(attempt.state, State::Leased | State::ExecutionStarted) {
                interruptions.push(nola_kernel::interrupt_effect_delivery_attempt(
                    attempt,
                    attempt
                        .worker_generation
                        .as_deref()
                        .ok_or("cancel worker generation missing")?,
                    attempt
                        .lease_owner
                        .as_deref()
                        .ok_or("cancel worker identity missing")?,
                    attempt
                        .lease_token
                        .as_deref()
                        .ok_or("cancel worker token missing")?,
                    nola_domain::EffectDeliveryRecoveryPlan::Retry,
                    if cancellation_delivery_reclaimable_by_expiry(attempt, input.claimed_at_ms) {
                        "unstarted cancellation delivery lease expired"
                    } else {
                        "exact cancellation delivery owner observed dead"
                    },
                    input.claimed_at_ms,
                )?);
            }
        }
    }
    if !interruptions.is_empty() {
        store
            .commit_effect_delivery_attempt_mutations(&interruptions)
            .map_err(|e| e.to_string())?;
    }
    let allowed = current
        .targets
        .iter()
        .filter(|fact| supports_dead_owner_confirmation(fact))
        .map(|fact| fact.cancellation.effect_id.0.clone())
        .collect();
    crate::effect_delivery::claim_committed_host_requests_selected(
        store,
        crate::ClaimCommittedHostRequests {
            run_id: input.identity.expected_run_id,
            worker_generation: input.worker_generation,
            lease_owner: input.lease_owner,
            claimed_at_ms: input.claimed_at_ms,
            lease_expires_at_ms: input.lease_expires_at_ms,
            limit: input.limit,
        },
        Some(&allowed),
    )
}
