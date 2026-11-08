// SPDX-License-Identifier: Apache-2.0
//! Application-owned Host-effect delivery admission and response fencing.

use nola_agent_engine::agent::host::{
    EffectDeliveryLease, HostReplaySafety, HostResponse, HostResponseEnvelope,
};
use nola_agent_engine::agent::permissions::PermissionContinuation;
use nola_domain::{
    EffectDeliveryAttempt, EffectDeliveryAttemptMutation, EffectDeliveryAttemptState,
    EffectDeliveryReplaySafety, EffectDeliverySettlement,
};
use nola_store_port::{effect_delivery::EffectDeliveryRepository, run_effect::RunEffectRepository};

use crate::CommittedHostRequests;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrepareRunEffectRecovery {
    pub conversation_id: String,
    pub run_id: String,
    pub invocation_config_digest: String,
    pub now_ms: i64,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedRunEffectRecovery {
    pub rebound_count: usize,
    /// Core-authored presentation facts for reissued permission requests.
    /// Hosts must not manufacture a replacement approval trace from arguments.
    pub traces: Vec<nola_agent_engine::agent_trace::AgentTraceEvent>,
}

/// Complete the Core half of an already-authorized Resume recovery before
/// scheduling a physical successor. Never replay the predecessor request or
/// turn an unknown outcome into a retry. Repeated calls skip rebound requests.
pub fn prepare_run_effect_recovery<R: crate::repositories::AgentRuntimeCommandRepositories + ?Sized>(
    store: &mut R,
    input: PrepareRunEffectRecovery,
) -> Result<PreparedRunEffectRecovery, String> {
    crate::assistant_publication_transaction::with_publication_transaction(store, |store| {
        let planned = plan_run_effect_recovery(store, &input)?;
        let mut prepared = PreparedRunEffectRecovery { rebound_count: planned.len(), traces: Vec::new() };
        for submission in planned {
            prepared.traces.extend(crate::reduce_and_commit_active_submission(store, submission)?.traces);
        }
        Ok(prepared)
    })
}

/// Shared authority decision for both standalone Store and hot Runtime callers.
/// The Runtime must execute these submissions through its normal transactional
/// reducer so deferred progress and rollback capsules retain their ownership.
pub(crate) fn plan_run_effect_recovery<R: EffectDeliveryRepository + RunEffectRepository + ?Sized>(
    store: &R,
    input: &PrepareRunEffectRecovery,
) -> Result<Vec<crate::ReduceAndCommitActiveSubmission>, String> {
    if input.now_ms < 0 { return Err("invalid effect recovery clock".into()); }
    let active = store.active_run(&input.conversation_id).map_err(|e| e.to_string())?
        .ok_or("effect recovery has no active Run")?;
    if active.run_id != input.run_id || active.invocation_config_digest.as_deref() != Some(input.invocation_config_digest.as_str()) {
        return Err("effect recovery does not match the frozen active Run".into());
    }
    let mut after = None;
    let mut planned = Vec::new();
    loop {
        let attempts = store.interrupted_effect_delivery_candidates(&input.run_id, after.as_ref(), 64, true)
            .map_err(|e| e.to_string())?;
        if attempts.is_empty() { break; }
        for attempt in &attempts {
            if attempt.recovery_plan != Some(nola_domain::EffectDeliveryRecoveryPlan::Resume) { continue; }
            let effect = store.outbox_effect_identity(&attempt.effect_id).map_err(|e| e.to_string())?
                .ok_or("recovery attempt has no outbox effect")?;
            if effect.run_id != input.run_id || effect.status != "pending" {
                return Err("recovery attempt lost its pending Run ownership".into());
            }
            if effect.request_id != attempt.request_id { continue; }
            planned.push(crate::ReduceAndCommitActiveSubmission {
                conversation_id: input.conversation_id.clone(), run_id: input.run_id.clone(),
                invocation_config_digest: input.invocation_config_digest.clone(),
                transition_id: format!("resume-effect:{}", attempt.id),
                submission: crate::AgentSubmission::ResumeHostEffect { effect_id: attempt.effect_id.clone().into() },
                committed_at_ms: input.now_ms,
            });
        }
        after = attempts.last().map(|attempt| nola_store_port::effect_delivery::InterruptedEffectDeliveryCursor {
            attempt_id: attempt.id.clone(),
        });
    }
    Ok(planned)
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoadRuntimeEffectWorkSchedule {
    pub conversation_ids: Vec<String>,
    pub now_ms: i64,
    pub limit: usize,
}

pub use nola_protocol::{
    RuntimeEffectWorkReason, RuntimeEffectWorkRunHead, RuntimeEffectWorkSchedule,
};

/// Read physical ownership from the attempt ledger, including workers that
/// crossed the start barrier but have not emitted a first Core response.
pub(crate) fn load_run_delivery_ownership<R>(
    store: &R,
    run_id: &str,
) -> Result<Vec<EffectDeliveryLease>, String>
where
    R: EffectDeliveryRepository + RunEffectRepository + ?Sized,
{
    let snapshot = store
        .load_committed_host_delivery_witness(run_id)
        .map_err(|error| format!("load delivery ownership: {error}"))?;
    let mut leases = Vec::new();
    let mut effect_ids = std::collections::BTreeSet::new();
    for effect in snapshot.effects {
        effect_ids.insert(effect.effect_id.clone());
        if effect.effect_kind == "cancel" {
            let request: nola_agent_engine::agent::host::HostRequestEnvelope =
                serde_json::from_str(&effect.payload_json)
                    .map_err(|_| "delivery cancellation request is invalid")?;
            if effect.run_id != run_id
                || request.effect_id.0 != effect.effect_id
                || request.request_id != effect.request_id
            {
                return Err("delivery cancellation request ownership mismatch".into());
            }
            let head = store
                .outbox_effect_identity(&effect.effect_id)
                .map_err(|error| format!("load cancellation ownership binding: {error}"))?
                .ok_or("delivery cancellation request has no committed binding")?;
            if head.run_id != run_id
                || head.status != "pending"
                || head.binding
                    != nola_store_port::host_replay::CoreHostBindingOverlay::from_request(&request)
            {
                return Err("delivery cancellation request binding mismatch".into());
            }
            if let nola_agent_engine::agent::host::HostRequest::Cancel { target_effect_id, .. } =
                request.payload
            {
                effect_ids.insert(target_effect_id.0);
            } else {
                return Err("delivery cancellation kind and payload disagree".into());
            }
        }
    }
    for effect_id in effect_ids {
        let Some(attempt) = store
            .active_effect_delivery_attempt(&effect_id)
            .map_err(|error| format!("load active delivery ownership: {error}"))?
        else {
            continue;
        };
        if attempt.run_id != run_id || attempt.effect_id != effect_id {
            return Err("delivery ownership Run mismatch".into());
        }
        if attempt.state == EffectDeliveryAttemptState::Prepared {
            // Preparation is not physical lease ownership. Do not manufacture
            // a dead-owner observation for work that was never claimed.
            continue;
        }
        leases.push(EffectDeliveryLease {
            attempt_id: attempt.id,
            worker_generation: attempt
                .worker_generation
                .ok_or("delivery owner has no generation")?,
            lease_owner: attempt
                .lease_owner
                .ok_or("delivery owner has no identity")?,
            lease_token: attempt.lease_token.ok_or("delivery owner has no token")?,
            lease_expires_at_ms: attempt
                .lease_expires_at_ms
                .ok_or("delivery owner has no expiry")?,
        });
        if leases.len() > 256 {
            return Err("delivery ownership exceeds bounded recovery window".into());
        }
    }
    Ok(leases)
}

/// A confirmed control settles its target independently of the target's old
/// execution lease. This is planned only AFTER Core accepted the exact control;
/// intent, expiry, dead PID observations and failed controls never call it.
pub(crate) fn confirmed_cancellation_mutations<R>(
    store: &R,
    run_id: &str,
    response: &HostResponseEnvelope,
    accepted_state: &nola_agent_engine::agent::AgentThreadState,
    received_at_ms: i64,
) -> Result<Vec<EffectDeliveryAttemptMutation>, String>
where
    R: RunEffectRepository + EffectDeliveryRepository + ?Sized,
{
    let HostResponse::CancellationConfirmed { target_effect_id } = &response.payload else {
        return Ok(Vec::new());
    };
    let control = store
        .outbox_effect(&response.effect_id.0)
        .map_err(|error| format!("load cancellation control: {error}"))?
        .ok_or("cancellation control has no committed request")?;
    let frozen: nola_agent_engine::agent::host::HostRequestEnvelope =
        serde_json::from_str(&control.payload_json)
            .map_err(|_| "invalid committed cancellation control")?;
    if control.run_id != run_id
        || control.status != "pending"
        || control.request_id != response.request_id
        || control.effect_id != response.effect_id.0
        || frozen.session_id != response.session_id
        || frozen.turn_id != response.turn_id
        || !matches!(&frozen.payload, nola_agent_engine::agent::host::HostRequest::Cancel { target_effect_id: target, .. } if target == target_effect_id)
        || accepted_state
            .host
            .pending_effects
            .get(&response.effect_id.0)
            .is_some()
        || accepted_state
            .host
            .pending_effects
            .get(&target_effect_id.0)
            .is_some()
    {
        return Err(
            "cancellation confirmation is not an accepted exact control/target pair".into(),
        );
    }
    let Some(target) = store
        .active_effect_delivery_attempt(&target_effect_id.0)
        .map_err(|error| format!("load cancellation target delivery: {error}"))?
    else {
        return Ok(Vec::new());
    };
    if response.delivery.is_none() {
        return Err("physical cancellation target requires a fenced control confirmation".into());
    }
    let target_request = store
        .outbox_effect_identity(&target_effect_id.0)
        .map_err(|error| format!("load cancellation target identity: {error}"))?
        .ok_or("cancellation target has no committed identity")?;
    if target.run_id != run_id
        || target.effect_id != target_effect_id.0
        || target_request.run_id != run_id
        || target_request.request_id != target.request_id
        || target_request.binding.session_id != response.session_id
        || target_request.binding.turn_id != response.turn_id
    {
        return Err("cancellation target delivery ownership mismatch".into());
    }
    Ok(vec![nola_kernel::confirm_effect_delivery_target_stopped(
        &target,
        logical_received_at_ms(Some(&target), received_at_ms),
    )?])
}

pub fn load_runtime_effect_work_schedule<R>(
    store: &R,
    query: LoadRuntimeEffectWorkSchedule,
) -> Result<RuntimeEffectWorkSchedule, String>
where
    R: EffectDeliveryRepository + ?Sized,
{
    let unique_conversations = query
        .conversation_ids
        .iter()
        .map(|id| id.as_str())
        .collect::<std::collections::HashSet<_>>();
    if query.now_ms < 0
        || !(1..=64).contains(&query.limit)
        || !(1..=64).contains(&query.conversation_ids.len())
        || query.conversation_ids.iter().any(|id| id.trim().is_empty())
        || unique_conversations.len() != query.conversation_ids.len()
    {
        return Err("invalid Runtime effect-work schedule query".into());
    }
    let schedule = store
        .runtime_effect_work_schedule(&query.conversation_ids, query.now_ms, query.limit)
        .map_err(|error| format!("load Runtime effect-work schedule failed: {error}"))?;
    if schedule.runs.len() > query.limit {
        return Err("Runtime effect-work schedule exceeded its requested limit".into());
    }
    let runs = schedule
        .runs
        .into_iter()
        .map(|head| RuntimeEffectWorkRunHead {
            run_id: head.run_id,
            conversation_id: head.conversation_id,
            effect_id: head.effect_id,
            request_id: head.request_id,
            attempt_id: head.attempt_id,
            reason: match head.reason {
                nola_store_port::effect_delivery::RuntimeEffectWorkReason::Pending => {
                    RuntimeEffectWorkReason::Pending
                }
                nola_store_port::effect_delivery::RuntimeEffectWorkReason::Retry => {
                    RuntimeEffectWorkReason::Retry
                }
                nola_store_port::effect_delivery::RuntimeEffectWorkReason::Resume => {
                    RuntimeEffectWorkReason::Resume
                }
                nola_store_port::effect_delivery::RuntimeEffectWorkReason::RecoverExpiredLease => {
                    RuntimeEffectWorkReason::RecoverExpiredLease
                }
            },
            ready_at_ms: head.ready_at_ms,
            created_at_ms: head.created_at_ms,
        })
        .collect();
    let schedule = RuntimeEffectWorkSchedule {
        observed_at_ms: query.now_ms,
        runs,
        next_wake_at_ms: schedule.next_wake_at_ms,
    };
    schedule
        .validate(query.limit as u32)
        .map_err(|error| format!("invalid Runtime effect-work schedule: {error}"))?;
    if schedule
        .runs
        .iter()
        .any(|head| !unique_conversations.contains(head.conversation_id.as_str()))
    {
        return Err("Runtime effect-work schedule escaped its conversation scope".into());
    }
    Ok(schedule)
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimCommittedHostRequests {
    pub run_id: String,
    pub worker_generation: String,
    pub lease_owner: String,
    pub claimed_at_ms: i64,
    pub lease_expires_at_ms: i64,
    pub limit: usize,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HeartbeatEffectDelivery {
    pub run_id: String,
    pub attempt_id: String,
    pub worker_generation: String,
    pub lease_owner: String,
    pub lease_token: String,
    pub current_lease_expires_at_ms: i64,
    pub heartbeat_at_ms: i64,
    pub requested_lease_expires_at_ms: i64,
}

fn domain_replay_safety(value: HostReplaySafety) -> EffectDeliveryReplaySafety {
    match value {
        HostReplaySafety::Idempotent => EffectDeliveryReplaySafety::Idempotent,
        HostReplaySafety::Recoverable => EffectDeliveryReplaySafety::Recoverable,
        HostReplaySafety::RequiresRecoveryDecision => {
            EffectDeliveryReplaySafety::RequiresRecoveryDecision
        }
    }
}

fn wire_lease(record: &EffectDeliveryAttempt) -> Result<EffectDeliveryLease, String> {
    Ok(EffectDeliveryLease {
        attempt_id: record.id.clone(),
        worker_generation: record
            .worker_generation
            .clone()
            .ok_or_else(|| "claimed Host effect has no worker generation".to_string())?,
        lease_owner: record
            .lease_owner
            .clone()
            .ok_or_else(|| "claimed Host effect has no lease owner".to_string())?,
        lease_token: record
            .lease_token
            .clone()
            .ok_or_else(|| "claimed Host effect has no lease token".to_string())?,
        lease_expires_at_ms: record
            .lease_expires_at_ms
            .ok_or_else(|| "claimed Host effect has no lease expiry".to_string())?,
    })
}

pub fn heartbeat_effect_delivery<R>(
    store: &mut R,
    command: HeartbeatEffectDelivery,
) -> Result<EffectDeliveryLease, String>
where
    R: RunEffectRepository + EffectDeliveryRepository + ?Sized,
{
    if command.run_id.trim().is_empty()
        || command.attempt_id.trim().is_empty()
        || command.worker_generation.trim().is_empty()
        || command.lease_owner.trim().is_empty()
        || command.lease_token.trim().is_empty()
        || command.current_lease_expires_at_ms < 0
        || command.heartbeat_at_ms < 0
        || command.requested_lease_expires_at_ms <= command.heartbeat_at_ms
    {
        return Err("invalid Host-effect delivery heartbeat".into());
    }
    let attempt = store
        .effect_delivery_attempt(&command.attempt_id)
        .map_err(|error| format!("load Host-effect delivery heartbeat attempt failed: {error}"))?
        .ok_or_else(|| "Host-effect delivery heartbeat names an unknown attempt".to_string())?;
    if attempt.run_id != command.run_id
        || attempt.worker_generation.as_deref() != Some(command.worker_generation.as_str())
        || attempt.lease_owner.as_deref() != Some(command.lease_owner.as_str())
        || attempt.lease_token.as_deref() != Some(command.lease_token.as_str())
        || attempt.lease_expires_at_ms != Some(command.current_lease_expires_at_ms)
    {
        return Err("stale or foreign Host-effect delivery heartbeat".into());
    }
    let active = store
        .active_effect_delivery_attempt(&attempt.effect_id)
        .map_err(|error| format!("load active Host-effect delivery heartbeat failed: {error}"))?
        .ok_or_else(|| "Host-effect delivery heartbeat attempt is not active".to_string())?;
    if active != attempt {
        return Err("Host-effect delivery heartbeat does not name the active attempt".into());
    }

    let snapshot = store
        .load_effect_heartbeat_snapshot(&command.run_id, &attempt.effect_id)
        .map_err(|error| format!("load Host-effect delivery heartbeat snapshot failed: {error}"))?
        .ok_or_else(|| "Host-effect delivery heartbeat has no pending outbox effect".to_string())?;
    if snapshot.run_id != attempt.run_id
        || snapshot.effect_id != attempt.effect_id
        || snapshot.request_id != attempt.request_id
    {
        return Err("Host-effect delivery heartbeat outbox identity mismatch".into());
    }
    let checkpoint = snapshot.checkpoint.ok_or_else(|| {
        "Host-effect delivery heartbeat has no canonical Core checkpoint".to_string()
    })?;
    let checkpoint: crate::CommittedDeliveryCheckpoint =
        serde_json::from_str(&checkpoint.witness_json).map_err(|error| {
            format!("decode Host-effect delivery heartbeat checkpoint failed: {error}")
        })?;
    if checkpoint.session_id != checkpoint.state.session_id {
        return Err("Host-effect delivery heartbeat checkpoint identity mismatch".into());
    }
    let pending = checkpoint
        .state
        .host
        .pending_effects
        .effects
        .get(&attempt.effect_id)
        .ok_or_else(|| {
            "Host-effect delivery heartbeat is absent from Core checkpoint".to_string()
        })?;
    if pending.effect_id != attempt.effect_id
        || pending.request_id != attempt.request_id
        || pending.session_id != checkpoint.session_id
        || domain_replay_safety(pending.replay_safety) != attempt.replay_safety
    {
        return Err("Host-effect delivery heartbeat replay identity mismatch".into());
    }

    let mutation = nola_kernel::heartbeat_effect_delivery_attempt(
        &attempt,
        &command.worker_generation,
        &command.lease_owner,
        &command.lease_token,
        command.requested_lease_expires_at_ms,
        command.heartbeat_at_ms,
    )?;
    // A delegated worker parks after it has emitted PermissionRequested so the
    // independently committed permission.ask effect can reach the interactive
    // Host. Its original delivery is still the fence for every later backend
    // event. Renew that parked delivery in the same transaction as the visible
    // permission wait; otherwise a user taking longer than one lease interval
    // leaves the warm worker holding an irrecoverably stale token.
    //
    // The relationship comes only from the canonical checkpoint. Requiring the
    // exact same worker generation and owner prevents a newly claimed
    // permission UI from keeping a disappeared predecessor alive.
    let mut mutations = vec![mutation];
    if let Some(permission_request_id) = pending.permission_request_id.as_ref() {
        let delegated_target = checkpoint
            .state
            .execution
            .permissions
            .pending
            .iter()
            .find(|pending| pending.request_id.as_str() == permission_request_id)
            .and_then(|pending| match &pending.continuation {
                PermissionContinuation::DelegatedAgent {
                    target_effect_id, ..
                } => Some(target_effect_id.as_str()),
                PermissionContinuation::CoreTool => None,
            });
        if let Some(target_effect_id) = delegated_target {
            let parent = store
                .active_effect_delivery_attempt(target_effect_id)
                .map_err(|error| {
                    format!("load parked delegated delivery heartbeat failed: {error}")
                })?
                .ok_or_else(|| {
                    "delegated permission wait has no active parent delivery".to_string()
                })?;
            if parent.run_id != command.run_id
                || parent.state != EffectDeliveryAttemptState::ExecutionStarted
                || parent.worker_generation.as_deref() != Some(command.worker_generation.as_str())
                || parent.lease_owner.as_deref() != Some(command.lease_owner.as_str())
            {
                return Err(
                    "delegated permission wait and parent delivery ownership differ".into(),
                );
            }
            let parent_token = parent
                .lease_token
                .as_deref()
                .ok_or("parked delegated delivery has no lease token")?;
            mutations.push(nola_kernel::heartbeat_effect_delivery_attempt(
                &parent,
                &command.worker_generation,
                &command.lease_owner,
                parent_token,
                command.requested_lease_expires_at_ms,
                command.heartbeat_at_ms,
            )?);
        }
    }
    let renewed = if mutations.len() == 1 {
        store
            .heartbeat_effect_delivery_attempt(&mutations[0])
            .map_err(|error| format!("commit Host-effect delivery heartbeat failed: {error}"))?
    } else {
        store
            .commit_effect_delivery_attempt_mutations(&mutations)
            .map_err(|error| {
                format!("commit linked Host-effect delivery heartbeats failed: {error}")
            })?
            .into_iter()
            .find(|record| record.id == command.attempt_id)
            .ok_or("linked Host-effect heartbeat omitted its requested attempt")?
    };
    wire_lease(&renewed)
}

pub fn claim_committed_host_requests<R>(
    store: &mut R,
    command: ClaimCommittedHostRequests,
) -> Result<CommittedHostRequests, String>
where
    R: RunEffectRepository + EffectDeliveryRepository + ?Sized,
{
    claim_committed_host_requests_selected(store, command, None)
}

pub(crate) fn claim_committed_host_requests_selected<R>(
    store: &mut R,
    command: ClaimCommittedHostRequests,
    allowed: Option<&std::collections::HashSet<String>>,
) -> Result<CommittedHostRequests, String>
where
    R: RunEffectRepository + EffectDeliveryRepository + ?Sized,
{
    if command.run_id.trim().is_empty()
        || command.worker_generation.trim().is_empty()
        || command.lease_owner.trim().is_empty()
        || !(1..=64).contains(&command.limit)
        || command.claimed_at_ms < 0
        || command.lease_expires_at_ms <= command.claimed_at_ms
    {
        return Err("invalid committed Host-effect claim".into());
    }

    let mut initial_candidates = Vec::new();
    let mut cursor = None;
    for _ in 0..if allowed.is_some() { 4 } else { 1 } {
        let page_size = if allowed.is_some() { 64 } else { command.limit };
        let page = store
            .pending_effect_delivery_candidates(&command.run_id, cursor.as_ref(), page_size)
            .map_err(|error| format!("load Host-effect delivery candidates failed: {error}"))?;
        cursor = page.last().map(|candidate| {
            nola_store_port::effect_delivery::EffectDeliveryCandidateCursor {
                created_at_ms: candidate.created_at_ms,
                effect_id: candidate.effect_id.clone(),
            }
        });
        let exhausted = page.len() < page_size;
        initial_candidates.extend(
            page.into_iter()
                .filter(|candidate| allowed.is_none_or(|ids| ids.contains(&candidate.effect_id))),
        );
        if exhausted || initial_candidates.len() >= command.limit {
            break;
        }
    }
    initial_candidates.truncate(command.limit);
    let remaining = command.limit.saturating_sub(initial_candidates.len());
    let interrupted_candidates =
        if remaining == 0 {
            Vec::new()
        } else {
            let mut selected = Vec::new();
            let mut cursor = None;
            for _ in 0..if allowed.is_some() { 4 } else { 1 } {
                let page = store
                    .interrupted_effect_delivery_candidates(
                        &command.run_id,
                        cursor.as_ref(),
                        if allowed.is_some() { 64 } else { remaining },
                        false,
                    )
                    .map_err(|error| {
                        format!("load interrupted Host-effect delivery candidates failed: {error}")
                    })?;
                cursor = page.last().map(|candidate| {
                    nola_store_port::effect_delivery::InterruptedEffectDeliveryCursor {
                        attempt_id: candidate.id.clone(),
                    }
                });
                let exhausted = page.len() < if allowed.is_some() { 64 } else { remaining };
                selected.extend(page.into_iter().filter(|candidate| {
                    allowed.is_none_or(|ids| ids.contains(&candidate.effect_id))
                }));
                if exhausted || selected.len() >= remaining {
                    break;
                }
            }
            selected.truncate(remaining);
            selected
        };
    if initial_candidates.is_empty() && interrupted_candidates.is_empty() {
        return Ok(CommittedHostRequests {
            effect_ids: Vec::new(),
            requests: Vec::new(),
        });
    }
    let selected = initial_candidates
        .iter()
        .map(|candidate| candidate.effect_id.clone())
        .chain(
            interrupted_candidates
                .iter()
                .map(|candidate| candidate.effect_id.clone()),
        )
        .collect::<std::collections::HashSet<_>>();
    let (committed, checkpoint) =
        crate::load_selected_committed_host_requests(store, &command.run_id, Some(&selected))?;
    let Some(checkpoint) = checkpoint else {
        return Ok(committed);
    };

    let mut requests = committed
        .requests
        .into_iter()
        .map(|request| (request.effect_id.0.clone(), request))
        .collect::<std::collections::HashMap<_, _>>();
    let mut claims = Vec::new();
    for candidate in initial_candidates {
        let Some(request) = requests.get(&candidate.effect_id) else {
            continue;
        };
        if candidate.request_id != request.request_id || candidate.run_id != command.run_id {
            return Err("Host-effect delivery candidate identity mismatch".into());
        }
        let pending = checkpoint
            .state
            .host
            .pending_effects
            .effects
            .get(&candidate.effect_id)
            .ok_or_else(|| "Host-effect candidate is absent from Core checkpoint".to_string())?;
        if !pending.matches_request(request) || pending.session_id != checkpoint.session_id {
            return Err("Host-effect candidate/current request mismatch".into());
        }
        let prepared = nola_kernel::prepare_effect_delivery_attempt(
            &uuid::Uuid::new_v4().to_string(),
            &command.run_id,
            &candidate.effect_id,
            &candidate.request_id,
            1,
            domain_replay_safety(pending.replay_safety),
            None,
            command.claimed_at_ms,
        )?;
        let claimed = nola_kernel::claim_effect_delivery_attempt(
            &prepared.record,
            &command.worker_generation,
            &command.lease_owner,
            &uuid::Uuid::new_v4().to_string(),
            command.lease_expires_at_ms,
            command.claimed_at_ms,
        )?;
        claims.push(EffectDeliveryAttemptMutation {
            expected_revision: None,
            record: claimed.record,
        });
    }
    for interrupted in interrupted_candidates {
        let Some(request) = requests.get(&interrupted.effect_id) else {
            continue;
        };
        if interrupted.run_id != command.run_id || request.effect_id.0 != interrupted.effect_id {
            return Err("interrupted Host-effect delivery candidate identity mismatch".into());
        }
        let pending = checkpoint
            .state
            .host
            .pending_effects
            .effects
            .get(&interrupted.effect_id)
            .ok_or_else(|| {
                "interrupted Host-effect candidate is absent from Core checkpoint".to_string()
            })?;
        if !pending.matches_request(request)
            || pending.session_id != checkpoint.session_id
            || domain_replay_safety(pending.replay_safety) != interrupted.replay_safety
        {
            return Err("interrupted Host-effect candidate/current request mismatch".into());
        }
        claims.push(nola_kernel::claim_interrupted_effect_delivery_successor(
            &interrupted,
            &uuid::Uuid::new_v4().to_string(),
            &request.request_id,
            &command.worker_generation,
            &command.lease_owner,
            &uuid::Uuid::new_v4().to_string(),
            command.lease_expires_at_ms,
            command.claimed_at_ms,
        )?);
    }
    if claims.is_empty() {
        return Ok(CommittedHostRequests {
            effect_ids: Vec::new(),
            requests: Vec::new(),
        });
    }
    let records = store
        .commit_effect_delivery_attempt_mutations(&claims)
        .map_err(|error| format!("claim committed Host effects failed: {error}"))?;
    let mut claimed_requests = Vec::with_capacity(records.len());
    for record in records {
        let mut request = requests
            .remove(&record.effect_id)
            .ok_or_else(|| "claimed Host effect has no validated request".to_string())?;
        request.delivery = Some(wire_lease(&record)?);
        claimed_requests.push(request);
    }
    // Claims are reconstructed from durable candidate order, which can
    // differ from Core's original batch after a crash or permission wait.
    nola_host_protocol::prioritize_code_cell_controls(&mut claimed_requests);
    let effect_ids = claimed_requests.iter().map(|request| request.effect_id.0.clone()).collect();
    Ok(CommittedHostRequests {
        effect_ids,
        requests: claimed_requests,
    })
}

pub(crate) fn validate_and_bind_response<R: EffectDeliveryRepository + ?Sized>(
    store: &R,
    run_id: &str,
    request: &mut nola_domain::host_control::HostEffectControl,
    response: &HostResponseEnvelope,
    received_at_ms: i64,
) -> Result<Option<EffectDeliveryAttempt>, String> {
    let attempt = validate_response_binding(store, run_id, request, response, received_at_ms)?;
    request.delivery = response.delivery.clone();
    Ok(attempt)
}

/// Validates the exact immutable request without copying its model context.
/// Callers that bind Core's request use the mutating wrapper above; a commit
/// within the same Application transaction only needs this proof.
pub(crate) fn validate_response_binding<R: EffectDeliveryRepository + ?Sized>(
    store: &R,
    run_id: &str,
    request: &nola_domain::host_control::HostEffectControl,
    response: &HostResponseEnvelope,
    received_at_ms: i64,
) -> Result<Option<EffectDeliveryAttempt>, String> {
    // A valid delivery lease does not authorize rebinding the response to a
    // different Core session or Host. Check the committed request as well as
    // the attempt before either reduction or direct transition acceptance.
    if request.session_id.is_empty()
        || response.session_id != request.session_id
        || response.host_id != request.host_id
        || response.effect_id != request.effect_id
        || response.request_id != request.request_id
        || response.lease_id != request.lease_id
        || response.catalog_revision != request.catalog_revision
        || response.availability_epoch != request.availability_epoch
        || (response.turn_id != request.turn_id
            // Core permits an omitted Turn echo only for the non-replaceable
            // in-process Host. A present foreign Turn is never normalized.
            && !(response.turn_id.is_none() && request.lease_id.is_none()))
    {
        return Err("Host response identity differs from its committed request".into());
    }
    if !request.payload.accepts_response(&response.payload) {
        return Err("Host response payload does not match its committed request".into());
    }
    let delivery = response
        .delivery
        .as_ref()
        .ok_or_else(|| "Host response omitted its required effect-delivery lease".to_string())?;
    let attempt = store
        .effect_delivery_attempt(&delivery.attempt_id)
        .map_err(|error| format!("load Host-effect delivery attempt failed: {error}"))?
        .ok_or_else(|| "Host response names an unknown delivery attempt".to_string())?;
    if attempt.run_id != run_id
        || attempt.effect_id != response.effect_id.0
        || attempt.request_id != response.request_id
        || attempt.worker_generation.as_deref() != Some(delivery.worker_generation.as_str())
        || attempt.lease_owner.as_deref() != Some(delivery.lease_owner.as_str())
        || attempt.lease_token.as_deref() != Some(delivery.lease_token.as_str())
        || attempt.lease_expires_at_ms != Some(delivery.lease_expires_at_ms)
    {
        return Err("stale or foreign Host-effect delivery response".into());
    }
    if received_at_ms >= delivery.lease_expires_at_ms {
        // Preserve timing evidence at the rejecting authority. A generic expiry
        // string cannot distinguish delayed admission from a clock step or a
        // missed renewal. Do not log lease tokens or the response payload.
        return Err(format!(
            "Host-effect delivery response arrived after lease expiry: effect_id={}, received_at_ms={}, lease_expires_at_ms={}, heartbeat_at_ms={:?}, overdue_ms={}",
            response.effect_id.0,
            received_at_ms,
            delivery.lease_expires_at_ms,
            attempt.heartbeat_at_ms,
            received_at_ms.saturating_sub(delivery.lease_expires_at_ms),
        ));
    }
    match &response.payload {
        HostResponse::ExecutionStarted if attempt.state != EffectDeliveryAttemptState::Leased => {
            return Err("Host-effect execution may start only from a leased attempt".into())
        }
        HostResponse::ExecutionStarted => {}
        HostResponse::ExecutionRejected { .. }
            if attempt.state != EffectDeliveryAttemptState::Leased =>
        {
            return Err("Host-effect execution rejection may settle only a leased attempt".into())
        }
        HostResponse::ExecutionRejected { .. } => {}
        _ if attempt.state != EffectDeliveryAttemptState::ExecutionStarted => {
            return Err(
                "Host response arrived before durable execution-start acknowledgement".into(),
            )
        }
        _ => {}
    }
    Ok(Some(attempt))
}

pub(crate) fn response_mutations(
    attempt: Option<&EffectDeliveryAttempt>,
    response: &HostResponseEnvelope,
    received_at_ms: i64,
) -> Result<Vec<EffectDeliveryAttemptMutation>, String> {
    let Some(attempt) = attempt else {
        return Ok(Vec::new());
    };
    let delivery = response
        .delivery
        .as_ref()
        .ok_or_else(|| "fenced Host response lost its delivery lease".to_string())?;
    let mutation = match &response.payload {
        HostResponse::ExecutionStarted => nola_kernel::start_effect_delivery_execution(
            attempt,
            &delivery.worker_generation,
            &delivery.lease_owner,
            &delivery.lease_token,
            received_at_ms,
        )?,
        HostResponse::ExecutionRejected { code, message, .. } => {
            let reason = format!("{code}: {message}");
            nola_kernel::reject_effect_delivery_before_execution(
                attempt,
                &delivery.worker_generation,
                &delivery.lease_owner,
                &delivery.lease_token,
                &reason,
                received_at_ms,
            )?
        }
        HostResponse::PermissionTransportInterrupted { .. }
        | HostResponse::DelegatedExecutionInterrupted { .. }
        | HostResponse::DelegatedPermissionInterrupted { .. } => {
            let reason = match &response.payload {
                HostResponse::PermissionTransportInterrupted { .. } => {
                    "permission transport interrupted"
                }
                HostResponse::DelegatedExecutionInterrupted { .. } => {
                    "delegated execution interrupted"
                }
                HostResponse::DelegatedPermissionInterrupted { .. } => {
                    "delegated permission transport interrupted"
                }
                _ => unreachable!(),
            };
            match attempt.replay_safety {
                EffectDeliveryReplaySafety::Idempotent => {
                    nola_kernel::interrupt_effect_delivery_attempt(
                        attempt,
                        &delivery.worker_generation,
                        &delivery.lease_owner,
                        &delivery.lease_token,
                        nola_domain::EffectDeliveryRecoveryPlan::Retry,
                        reason,
                        received_at_ms,
                    )?
                }
                EffectDeliveryReplaySafety::Recoverable => {
                    nola_kernel::interrupt_effect_delivery_attempt(
                        attempt,
                        &delivery.worker_generation,
                        &delivery.lease_owner,
                        &delivery.lease_token,
                        nola_domain::EffectDeliveryRecoveryPlan::Resume,
                        reason,
                        received_at_ms,
                    )?
                }
                EffectDeliveryReplaySafety::RequiresRecoveryDecision => {
                    nola_kernel::mark_effect_delivery_outcome_unknown(
                        attempt,
                        &delivery.worker_generation,
                        &delivery.lease_owner,
                        &delivery.lease_token,
                        reason,
                        received_at_ms,
                    )?
                }
            }
        }
        payload => {
            let Some(settlement) = terminal_settlement(payload)? else {
                return Ok(Vec::new());
            };
            nola_kernel::settle_effect_delivery_attempt(
                attempt,
                &delivery.worker_generation,
                &delivery.lease_owner,
                &delivery.lease_token,
                settlement,
                received_at_ms,
            )?
        }
    };
    Ok(vec![mutation])
}

pub(crate) fn logical_received_at_ms(
    attempt: Option<&EffectDeliveryAttempt>,
    observed_at_ms: i64,
) -> i64 {
    let Some(attempt) = attempt else {
        return observed_at_ms;
    };
    let delivery_floor = [
        Some(attempt.created_at_ms),
        attempt.claimed_at_ms,
        attempt.execution_started_at_ms,
        attempt.heartbeat_at_ms,
    ]
    .into_iter()
    .flatten()
    .max()
    .unwrap_or(attempt.created_at_ms);
    observed_at_ms.max(delivery_floor)
}

pub(crate) fn terminal_settlement(
    response: &HostResponse,
) -> Result<Option<EffectDeliverySettlement>, String> {
    use nola_agent_engine::agent::host::{DelegatedAgentEventV1, HostToolOutcome};
    use nola_agent_engine::agent::task::TaskState;

    let settlement = match response {
        HostResponse::Failed { .. } => Some(EffectDeliverySettlement::Failed),
        HostResponse::CancellationConfirmed { .. } => Some(EffectDeliverySettlement::Cancelled),
        HostResponse::DelegatedAgentEvent { event } => match event {
            DelegatedAgentEventV1::Completed { .. } => Some(EffectDeliverySettlement::Succeeded),
            DelegatedAgentEventV1::Failed { .. } => Some(EffectDeliverySettlement::Failed),
            DelegatedAgentEventV1::CancellationConfirmed => {
                Some(EffectDeliverySettlement::Cancelled)
            }
            _ => None,
        },
        HostResponse::ToolCompleted { outcome, .. } => Some(match outcome {
            HostToolOutcome::Succeeded | HostToolOutcome::Deferred => {
                EffectDeliverySettlement::Succeeded
            }
            HostToolOutcome::Failed => EffectDeliverySettlement::Failed,
            HostToolOutcome::Cancelled => EffectDeliverySettlement::Cancelled,
        }),
        HostResponse::TaskUpdated { state, .. } => match state {
            // Host delivery has settled. This does not accept the Task's or
            // parent's business requirements; only the sealed Run result can.
            TaskState::Sealed | TaskState::Completed => Some(EffectDeliverySettlement::Succeeded),
            TaskState::Failed => Some(EffectDeliverySettlement::Failed),
            TaskState::Cancelled => Some(EffectDeliverySettlement::Cancelled),
            _ => None,
        },
        HostResponse::TaskPromoted { .. }
        | HostResponse::DelegatedPermissionTransportWritten { .. }
        | HostResponse::PermissionResolved { .. }
        | HostResponse::ContextFetched { .. }
        | HostResponse::PermissionResourcesResolved { .. }
        | HostResponse::CapabilityCatalogRefreshed { .. }
        | HostResponse::ModelCompleted { .. }
        | HostResponse::ModelOutputLimited { .. }
        | HostResponse::CompactionCompleted { .. }
        | HostResponse::Completed { .. } => Some(EffectDeliverySettlement::Succeeded),
        response if response.is_terminal() => {
            return Err("terminal Host response has no explicit delivery settlement mapping".into())
        }
        _ => None,
    };
    Ok(settlement)
}

#[cfg(test)]
mod schedule_tests {
    use super::*;
    use nola_store_port::effect_delivery::{
        EffectDeliveryCandidate, EffectDeliveryCandidateCursor, InterruptedEffectDeliveryCursor,
    };
    use std::cell::Cell;

    struct ScheduleStore {
        calls: Cell<usize>,
        returned_runs: usize,
    }

    impl EffectDeliveryRepository for ScheduleStore {
        type Error = String;
        fn unknown_effect_recovery_receipt(
            &self,
            _: &str,
        ) -> Result<Option<(String, String)>, String> {
            Err("not supported by scheduler fixture".into())
        }
        fn record_unknown_effect_recovery_receipt(
            &mut self,
            _: &str,
            _: &str,
            _: &str,
        ) -> Result<(), String> {
            Err("not supported by scheduler fixture".into())
        }

        fn runtime_effect_work_schedule(
            &self,
            conversation_ids: &[String],
            now_ms: i64,
            limit: usize,
        ) -> Result<nola_store_port::effect_delivery::RuntimeEffectWorkSchedule, String> {
            self.calls.set(self.calls.get() + 1);
            assert_eq!(conversation_ids, ["conversation-0"]);
            assert_eq!(now_ms, 50);
            assert_eq!(limit, 2);
            Ok(nola_store_port::effect_delivery::RuntimeEffectWorkSchedule {
                runs: (0..self.returned_runs)
                    .map(|index| nola_store_port::effect_delivery::RuntimeEffectWorkRunHead {
                        conversation_id: format!("conversation-{index}"),
                        run_id: format!("run-{index}"),
                        effect_id: format!("effect-{index}"),
                        request_id: format!("request-{index}"),
                        attempt_id: Some(format!("attempt-{index}")),
                        reason: nola_store_port::effect_delivery::RuntimeEffectWorkReason::RecoverExpiredLease,
                        ready_at_ms: 40 + index as i64,
                        created_at_ms: 10 + index as i64,
                    })
                    .collect(),
                next_wake_at_ms: Some(75),
            })
        }

        fn pending_effect_delivery_candidates(
            &self,
            _: &str,
            _: Option<&EffectDeliveryCandidateCursor>,
            _: usize,
        ) -> Result<Vec<EffectDeliveryCandidate>, String> {
            panic!("schedule query attempted a claim scan")
        }
        fn effect_delivery_attempt(
            &self,
            _: &str,
        ) -> Result<Option<EffectDeliveryAttempt>, String> {
            panic!("schedule query loaded an individual attempt")
        }
        fn active_effect_delivery_attempt(
            &self,
            _: &str,
        ) -> Result<Option<EffectDeliveryAttempt>, String> {
            panic!("schedule query loaded an active attempt")
        }
        fn interrupted_effect_delivery_candidates(
            &self,
            _: &str,
            _: Option<&InterruptedEffectDeliveryCursor>,
            _: usize,
            _: bool,
        ) -> Result<Vec<EffectDeliveryAttempt>, String> {
            panic!("schedule query attempted an interrupted claim scan")
        }
        fn claim_effect_delivery_attempts(
            &mut self,
            _: &[EffectDeliveryAttemptMutation],
        ) -> Result<Vec<EffectDeliveryAttempt>, String> {
            panic!("schedule query attempted a write")
        }
        fn heartbeat_effect_delivery_attempt(
            &mut self,
            _: &EffectDeliveryAttemptMutation,
        ) -> Result<EffectDeliveryAttempt, String> {
            panic!("schedule query attempted a write")
        }
        fn authorize_unknown_effect_retry(
            &mut self,
            _: &EffectDeliveryAttemptMutation,
        ) -> Result<EffectDeliveryAttempt, String> {
            panic!("schedule query attempted a write")
        }
        fn commit_effect_delivery_attempt_mutations(
            &mut self,
            _: &[EffectDeliveryAttemptMutation],
        ) -> Result<Vec<EffectDeliveryAttempt>, String> {
            panic!("schedule query attempted a write")
        }
    }

    #[test]
    fn schedule_query_is_bounded_read_only_and_serializes_typed_identity() {
        let store = ScheduleStore {
            calls: Cell::new(0),
            returned_runs: 1,
        };
        let schedule = load_runtime_effect_work_schedule(
            &store,
            LoadRuntimeEffectWorkSchedule {
                conversation_ids: vec!["conversation-0".into()],
                now_ms: 50,
                limit: 2,
            },
        )
        .unwrap();
        assert_eq!(store.calls.get(), 1);
        assert_eq!(schedule.next_wake_at_ms, Some(75));
        assert_eq!(
            serde_json::to_value(schedule).unwrap(),
            serde_json::json!({
                "observed_at_ms": 50,
                "runs": [{
                    "conversation_id": "conversation-0",
                    "run_id": "run-0",
                    "effect_id": "effect-0",
                    "request_id": "request-0",
                    "attempt_id": "attempt-0",
                    "reason": "recover_expired_lease",
                    "ready_at_ms": 40,
                    "created_at_ms": 10
                }],
                "next_wake_at_ms": 75
            })
        );
    }

    #[test]
    fn schedule_query_rejects_invalid_inputs_before_repository_access() {
        for query in [
            LoadRuntimeEffectWorkSchedule {
                conversation_ids: vec!["conversation".into()],
                now_ms: -1,
                limit: 1,
            },
            LoadRuntimeEffectWorkSchedule {
                conversation_ids: vec!["conversation".into()],
                now_ms: 0,
                limit: 0,
            },
            LoadRuntimeEffectWorkSchedule {
                conversation_ids: vec!["conversation".into()],
                now_ms: 0,
                limit: 65,
            },
            LoadRuntimeEffectWorkSchedule {
                conversation_ids: Vec::new(),
                now_ms: 0,
                limit: 1,
            },
            LoadRuntimeEffectWorkSchedule {
                conversation_ids: vec![" ".into()],
                now_ms: 0,
                limit: 1,
            },
            LoadRuntimeEffectWorkSchedule {
                conversation_ids: vec!["same".into(), "same".into()],
                now_ms: 0,
                limit: 1,
            },
        ] {
            let store = ScheduleStore {
                calls: Cell::new(0),
                returned_runs: 0,
            };
            assert!(load_runtime_effect_work_schedule(&store, query).is_err());
            assert_eq!(store.calls.get(), 0);
        }
    }

    #[test]
    fn schedule_query_rejects_a_repository_that_breaks_the_limit_contract() {
        let store = ScheduleStore {
            calls: Cell::new(0),
            returned_runs: 3,
        };
        assert!(load_runtime_effect_work_schedule(
            &store,
            LoadRuntimeEffectWorkSchedule {
                conversation_ids: vec!["conversation-0".into()],
                now_ms: 50,
                limit: 2,
            }
        )
        .is_err());
    }
}

#[cfg(test)]
mod settlement_tests {
    use super::*;
    use nola_agent_engine::agent::host::{DelegatedAgentEventV1, HostError, HostToolOutcome};
    use nola_agent_engine::agent::task::TaskState;
    use serde_json::Value;

    fn delegated(event: DelegatedAgentEventV1) -> HostResponse {
        HostResponse::DelegatedAgentEvent { event }
    }

    fn tool(outcome: HostToolOutcome) -> HostResponse {
        HostResponse::ToolCompleted {
            call_id: "call".into(),
            output: Value::Null,
            outcome,
            activation: None,
            full_output_ref: None,
            artifacts: Vec::new(),
        }
    }

    fn task(state: TaskState) -> HostResponse {
        HostResponse::TaskUpdated {
            task_id: "task".into(),
            state,
            progress: None,
            current_step: None,
            result_ref: None,
            cancel_requested: false,
            ownership_token: None,
            completion_watermark: None,
        }
    }

    #[test]
    fn typed_terminal_failures_and_cancellations_never_settle_as_success() {
        let failure = HostError {
            code: "failed".into(),
            message: "failed".into(),
            retryable: false,
            http_status: None,
        };
        for response in [
            HostResponse::Failed {
                error: failure.clone(),
            },
            delegated(DelegatedAgentEventV1::Failed {
                error: failure.clone(),
            }),
            tool(HostToolOutcome::Failed),
            task(TaskState::Failed),
        ] {
            assert_eq!(
                terminal_settlement(&response).unwrap(),
                Some(EffectDeliverySettlement::Failed)
            );
        }
        for response in [
            HostResponse::CancellationConfirmed {
                target_effect_id: nola_agent_engine::agent::host::HostEffectId("effect".into()),
            },
            delegated(DelegatedAgentEventV1::CancellationConfirmed),
            tool(HostToolOutcome::Cancelled),
            task(TaskState::Cancelled),
        ] {
            assert_eq!(
                terminal_settlement(&response).unwrap(),
                Some(EffectDeliverySettlement::Cancelled)
            );
        }
    }

    #[test]
    fn typed_success_and_nonterminal_task_states_are_distinct() {
        assert_eq!(
            terminal_settlement(&delegated(DelegatedAgentEventV1::Completed {
                items: Vec::new(),
                usage: Value::Null,
            }))
            .unwrap(),
            Some(EffectDeliverySettlement::Succeeded)
        );
        assert_eq!(
            terminal_settlement(&tool(HostToolOutcome::Succeeded)).unwrap(),
            Some(EffectDeliverySettlement::Succeeded)
        );
        assert_eq!(
            terminal_settlement(&task(TaskState::Completed)).unwrap(),
            Some(EffectDeliverySettlement::Succeeded)
        );
        assert_eq!(
            terminal_settlement(&task(TaskState::Sealed)).unwrap(),
            Some(EffectDeliverySettlement::Succeeded)
        );
        assert_eq!(
            terminal_settlement(&task(TaskState::Running)).unwrap(),
            None
        );
    }

    #[test]
    fn response_time_uses_the_durable_heartbeat_floor_after_wall_clock_regression() {
        let prepared = nola_kernel::prepare_effect_delivery_attempt(
            "attempt",
            "run",
            "effect",
            "request",
            1,
            EffectDeliveryReplaySafety::Idempotent,
            None,
            100,
        )
        .unwrap();
        let claimed = nola_kernel::claim_effect_delivery_attempt(
            &prepared.record,
            "generation",
            "owner",
            "token",
            1_000,
            100,
        )
        .unwrap();
        let started = nola_kernel::start_effect_delivery_execution(
            &claimed.record,
            "generation",
            "owner",
            "token",
            110,
        )
        .unwrap();
        let before_heartbeat_received_at_ms = logical_received_at_ms(Some(&started.record), 109);
        let before_heartbeat_settled = nola_kernel::settle_effect_delivery_attempt(
            &started.record,
            "generation",
            "owner",
            "token",
            EffectDeliverySettlement::Succeeded,
            before_heartbeat_received_at_ms,
        )
        .unwrap();
        let heartbeat = nola_kernel::heartbeat_effect_delivery_attempt(
            &started.record,
            "generation",
            "owner",
            "token",
            1_100,
            200,
        )
        .unwrap();

        let received_at_ms = logical_received_at_ms(Some(&heartbeat.record), 199);
        let settled = nola_kernel::settle_effect_delivery_attempt(
            &heartbeat.record,
            "generation",
            "owner",
            "token",
            EffectDeliverySettlement::Succeeded,
            received_at_ms,
        )
        .unwrap();

        assert_eq!(before_heartbeat_received_at_ms, 110);
        assert_eq!(before_heartbeat_settled.record.finished_at_ms, Some(110));
        assert_eq!(received_at_ms, 200);
        assert_eq!(settled.record.finished_at_ms, Some(200));
    }
}
