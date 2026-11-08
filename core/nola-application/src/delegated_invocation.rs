// SPDX-License-Identifier: Apache-2.0
//! Admission of opaque external invocations, independent of model sampling.
use crate::*;
use nola_domain::{DelegatedInvocationMutation as Mutation, DelegatedInvocationState as State};
use nola_host_protocol::DelegatedAgentEventV1 as Event;

pub(crate) fn is_delegated(request: &HostRequestEnvelope) -> bool {
    matches!(request.payload, HostRequest::DelegatedAgent { .. })
}

pub(crate) fn prepare_effects<R: RunEffectRepository + ?Sized>(
    store: &R,
    run: &str,
    effects: &mut [OutboxEffectInput],
    now: i64,
) -> Result<Vec<Mutation>, String> {
    let mut changes = Vec::new();
    for effect in effects {
        let nola_store_port::run_effect::OutboxEffectWrite::Declare { payload_json } =
            &mut effect.write
        else {
            continue;
        };
        let mut envelope: HostRequestEnvelope =
            serde_json::from_str(payload_json).map_err(|e| e.to_string())?;
        let HostRequest::DelegatedAgent { request } = &envelope.payload else {
            continue;
        };
        let record = match store
            .delegated_invocation_for_effect(&effect.effect_id)
            .map_err(|e| e.to_string())?
        {
            Some(record) => {
                if record.run_id != run
                    || record.backend_id != request.backend_id
                    || record.text_part_id != request.text_part_id
                    || record.reasoning_part_id != request.reasoning_part_id
                    || record.state.is_terminal()
                {
                    return Err("delegated continuation differs from canonical invocation".into());
                }
                record
            }
            None => {
                let record = nola_kernel::prepare_delegated_invocation(
                    run,
                    &effect.effect_id,
                    &uuid::Uuid::new_v4().to_string(),
                    &request.backend_id,
                    request.backend_session_id.as_deref(),
                    &request.text_part_id,
                    &request.reasoning_part_id,
                    now,
                )?;
                changes.push(Mutation {
                    expected_revision: None,
                    record: record.clone(),
                });
                record
            }
        };
        envelope.delegated_invocation_id = Some(record.id.clone());
        effect.delegated_invocation_id = Some(record.id);
        *payload_json = serde_json::to_string(&envelope).map_err(|e| e.to_string())?;
    }
    Ok(changes)
}

pub(crate) fn bind_response_control<R: RunEffectRepository + ?Sized>(
    store: &R,
    run: &str,
    request: &mut nola_domain::host_control::HostEffectControl,
) -> Result<(), String> {
    if !matches!(
        request.payload.policy,
        nola_domain::host_control::HostResponsePolicy::DelegatedAgent { .. }
    ) {
        return Ok(());
    }
    let current = store
        .delegated_invocation_for_effect(&request.effect_id.0)
        .map_err(|e| e.to_string())?
        .ok_or("delegated effect has no invocation ledger")?;
    nola_kernel::validate_delegated_invocation(&current)?;
    if current.run_id != run
        || request
            .delegated_invocation_id
            .as_ref()
            .is_some_and(|id| id != &current.id)
    {
        return Err("delegated control authority changed".into());
    }
    request.delegated_invocation_id = Some(current.id);
    request.validate_bound()
}

pub(crate) fn validate_response<R: RunEffectRepository + ?Sized>(
    store: &R,
    run: &str,
    request: &nola_domain::host_control::HostEffectControl,
    response: &HostResponseEnvelope,
) -> Result<(), String> {
    if !matches!(
        request.payload.policy,
        nola_domain::host_control::HostResponsePolicy::DelegatedAgent { .. }
    ) {
        return if response.delegated_invocation_id.is_some() {
            Err("non-delegated response carries invocation identity".into())
        } else {
            Ok(())
        };
    }
    let current = store
        .delegated_invocation_for_effect(&request.effect_id.0)
        .map_err(|e| e.to_string())?
        .ok_or("delegated effect has no invocation ledger")?;
    nola_kernel::validate_delegated_invocation(&current)?;
    if current.run_id != run
        || current.state.is_terminal()
        || request.delegated_invocation_id.as_deref() != Some(current.id.as_str())
        || response.delegated_invocation_id != request.delegated_invocation_id
    {
        return Err("delegated response invocation identity mismatch".into());
    }
    match &response.payload {
        HostResponse::Accepted | HostResponse::ExecutionStarted
            if current.state != State::Prepared =>
        {
            return Err("delegated invocation already admitted".into())
        }
        HostResponse::ExecutionRejected { .. } if current.state != State::Prepared => {
            return Err("delegated invocation may be rejected only before admission".into())
        }
        HostResponse::DelegatedAgentEvent { event } => {
            if current.state == State::Prepared {
                return Err("delegated output precedes execution admission".into());
            }
            if matches!(current.state, State::Interrupted | State::Cancelling)
                && !matches!(event, Event::CancellationConfirmed | Event::Failed { .. })
            {
                return Err("interrupted invocation requires recovery evidence".into());
            }
            match event {
                Event::SessionStarted {
                    backend_id,
                    backend_session_id,
                } if backend_id != &current.backend_id
                    || current
                        .backend_session_id
                        .as_ref()
                        .is_some_and(|id| id != backend_session_id) =>
                {
                    return Err("delegated backend session identity changed".into())
                }
                Event::TextDelta { part_id, .. } if part_id != &current.text_part_id => {
                    return Err("delegated text belongs to another invocation".into())
                }
                Event::ReasoningDelta { part_id, .. } if part_id != &current.reasoning_part_id => {
                    return Err("delegated reasoning belongs to another invocation".into())
                }
                _ => {}
            }
        }
        _ => {}
    }
    Ok(())
}

pub(crate) fn response_mutations<R: RunEffectRepository + ?Sized>(
    store: &R,
    run: &str,
    response: &HostResponseEnvelope,
    now: i64,
) -> Result<Vec<Mutation>, String> {
    let target_effect = match &response.payload {
        HostResponse::CancellationConfirmed { target_effect_id } => target_effect_id.0.as_str(),
        _ => response.effect_id.0.as_str(),
    };
    let Some(record) = store
        .delegated_invocation_for_effect(target_effect)
        .map_err(|e| e.to_string())?
    else {
        return Ok(vec![]);
    };
    if record.state == State::Cancelled
        && matches!(response.payload, HostResponse::CancellationConfirmed { .. })
    {
        return Ok(vec![]);
    }

    if record.run_id != run {
        return Err("delegated invocation Run mismatch".into());
    }
    let mut session = None;
    let mut failure = None;
    let mut usage = None;
    let state = match &response.payload {
        HostResponse::Accepted | HostResponse::ExecutionStarted => State::Running,
        HostResponse::DelegatedExecutionInterrupted { .. } => State::Interrupted,
        HostResponse::Failed { error } => {
            failure = Some(error.message.clone());
            State::Failed
        }
        HostResponse::ExecutionRejected { message, .. } => {
            failure = Some(message.clone());
            State::Failed
        }
        HostResponse::CancellationConfirmed { .. } => State::Cancelled,
        HostResponse::DelegatedAgentEvent { event } => match event {
            Event::SessionStarted {
                backend_session_id, ..
            } => {
                session = Some(backend_session_id.as_str());
                State::Running
            }
            Event::PermissionRequested { .. } => State::WaitingPermission,
            Event::Completed { usage: value, .. } => {
                // Completion must not erase accounting committed earlier in the stream.
                if record.usage.get("accounting").is_none() { usage = Some(value.clone()); }
                State::Succeeded
            }
            Event::Failed { error } => {
                failure = Some(error.message.clone());
                State::Failed
            }
            Event::CancellationConfirmed => State::Cancelled,
            Event::Accounting { snapshot } => {
                if snapshot.backend_id != record.backend_id { return Err("external usage backend mismatch".into()); }
                snapshot.validate().map_err(str::to_string)?;
                if snapshot.basis == nola_domain::delegated_invocation::ExternalUsageBasis::SessionCumulative
                    && record.backend_session_id.as_deref() != Some(snapshot.scope_id.as_str()) {
                    return Err("external usage session mismatch".into());
                }
                if snapshot.basis != nola_domain::delegated_invocation::ExternalUsageBasis::SessionCumulative
                    && snapshot.scope_id != record.id {
                    return Err("external usage invocation mismatch".into());
                }
                // This is the latest cumulative snapshot for this invocation's
                // source, not another charge on duplicate provider events.
                usage = Some(serde_json::json!({"accounting":snapshot}));
                record.state
            }
            Event::Usage { .. } => {
                if record.usage.get("accounting").is_none() { usage = Some(serde_json::to_value(event).map_err(|e| e.to_string())?); }
                record.state
            }
            _ => State::Running,
        },
        _ => return Ok(vec![]),
    };
    // Body deltas retain ledger identity; only lifecycle/session/usage changes
    // need a new ledger revision in the same content transaction.
    if state == record.state && session.is_none() && failure.is_none() && usage.is_none() {
        return Ok(vec![]);
    }
    Ok(vec![nola_kernel::transition_delegated_invocation(
        &record, state, session, failure, usage, now,
    )?])
}

pub(crate) fn bind_delivery<R: RunEffectRepository + ?Sized>(
    store: &R,
    run: &str,
    request: &HostRequestEnvelope,
) -> Result<(), String> {
    if !is_delegated(request) {
        return Ok(());
    }
    let record = store
        .delegated_invocation_for_effect(&request.effect_id.0)
        .map_err(|e| e.to_string())?
        .ok_or("delegated delivery missing invocation")?;
    nola_kernel::validate_delegated_invocation(&record)?;
    if record.run_id != run
        || record.state.is_terminal()
        || request.delegated_invocation_id.as_deref() != Some(record.id.as_str())
    {
        return Err("delegated delivery differs from invocation ledger".into());
    }
    Ok(())
}

pub(crate) fn settle_effects<R: RunEffectRepository + ?Sized>(
    store: &R,
    effects: &[String],
    now: i64,
) -> Result<Vec<Mutation>, String> {
    let mut changes = Vec::new();
    for effect in effects {
        if let Some(record) = store
            .delegated_invocation_for_effect(effect)
            .map_err(|e| e.to_string())?
        {
            if !record.state.is_terminal() && record.state != State::Cancelling {
                let state = if record.started_at_ms.is_some() {
                    State::Cancelling
                } else {
                    State::Cancelled
                };
                changes.push(nola_kernel::transition_delegated_invocation(
                    &record, state, None, None, None, now,
                )?);
            }
        }
    }
    Ok(changes)
}

pub(crate) fn append_events(
    events: &mut Vec<RunEventInput>,
    sequence: &mut i64,
    changes: &[Mutation],
    now: i64,
) -> Result<(), String> {
    for change in changes {
        events.push(RunEventInput {
            event_id: format!(
                "delegated-invocation:{}:{}",
                change.record.id, change.record.revision
            ),
            sequence: *sequence,
            event_kind: "delegated_invocation_changed".into(),
            payload_json: serde_json::to_string(&change.record).map_err(|e| e.to_string())?,
            created_at_ms: now,
        });
        *sequence = sequence
            .checked_add(1)
            .ok_or("Run event sequence exhausted")?;
    }
    Ok(())
}

/// The frozen invocation config remains immutable. During the legacy Core
/// adapter restore, bind only a session observation proven by the independent
/// canonical invocation ledger; never accept arbitrary checkpoint config drift.
pub(crate) fn bind_restore_configuration(
    config: &mut AgentCoreConfig,
    checkpoint: &CoreCheckpoint<AgentThreadState>,
    run: &str,
    records: &[nola_domain::DelegatedInvocationRecord],
) -> Result<(), String> {
    let actual = &checkpoint
        .state
        .configuration
        .turn_identity
        .execution_backend;
    let expected = &mut config.turn_identity.execution_backend;
    if actual.backend_session_id == expected.backend_session_id {
        return Ok(());
    }
    if actual.backend_id != expected.backend_id {
        return Err("delegated restore backend differs from frozen configuration".into());
    }
    let Some(session) = actual.backend_session_id.as_deref() else {
        return Err("delegated restore discarded its session binding".into());
    };
    let proven = records.iter().any(|record| {
        record.run_id == run
            && record.backend_id == actual.backend_id
            && record.backend_session_id.as_deref() == Some(session)
    });
    if !proven {
        return Err("delegated restore session lacks canonical invocation evidence".into());
    }
    expected.backend_session_id = Some(session.to_owned());
    Ok(())
}

pub(crate) fn bind_stored_restore_configuration<R: RunEffectRepository + ?Sized>(
    store: &R,
    config: &mut AgentCoreConfig,
    checkpoint: &CoreCheckpoint<AgentThreadState>,
    run: &str,
) -> Result<(), String> {
    let mut records = Vec::new();
    for effect in checkpoint.state.host.pending_effects.iter() {
        for owner_effect in
            std::iter::once(effect.effect_id.as_str()).chain(effect.target_effect_id.as_deref())
        {
            if let Some(record) = store
                .delegated_invocation_for_effect(owner_effect)
                .map_err(|e| e.to_string())?
            {
                if !records
                    .iter()
                    .any(|existing: &nola_domain::DelegatedInvocationRecord| {
                        existing.id == record.id
                    })
                {
                    records.push(record);
                }
            }
        }
    }
    bind_restore_configuration(config, checkpoint, run, &records)
}
