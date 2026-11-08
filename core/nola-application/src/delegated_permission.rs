// SPDX-License-Identifier: Apache-2.0
//! Canonical permission occurrence, committed control intent and transport evidence.
use crate::*;
use nola_domain::{
    DelegatedPermissionDecision as Decision, DelegatedPermissionDeliveryMutation as Mutation,
    DelegatedPermissionDeliveryRecord as Record, DelegatedPermissionDeliveryState as State,
};
use nola_host_protocol::{
    DelegatedAgentEventV1 as Event, DelegatedPermissionDeliveryTarget as Target,
    HostPermissionAuthorizationDecision as Authorization,
};

pub(crate) fn is_control(request: &HostRequestEnvelope) -> bool {
    matches!(
        request.payload,
        HostRequest::ResolveDelegatedPermission { .. }
    )
}
fn digest(input: &Value) -> Result<String, String> {
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(input).map_err(|e| e.to_string())?)
    ))
}
fn target(record: &Record) -> Target {
    Target {
        delivery_id: record.id.clone(),
        invocation_id: record.invocation_id.clone(),
        worker_generation: record.worker_generation.clone(),
        occurrence_id: record.occurrence_id.clone(),
        call_id: record.call_id.clone(),
        request_digest: record.request_digest.clone(),
    }
}
fn decision(resolution: &nola_host_protocol::HostPermissionResolution) -> Result<Decision, String> {
    match resolution {
        nola_host_protocol::HostPermissionResolution::Authorization { decision, .. } => {
            Ok(match decision {
                Authorization::AllowOnce => Decision::AllowOnce,
                Authorization::AllowConversation => Decision::AllowConversation,
                Authorization::Deny => Decision::Deny,
            })
        }
        _ => Err("permission guidance is not a delegated authorization".into()),
    }
}
fn validate_record(record: &Record, run: &str) -> Result<(), String> {
    nola_kernel::validate_delegated_permission(record)?;
    if record.run_id != run || record.request_digest != digest(&record.request_json)? {
        return Err("delegated permission record integrity mismatch".into());
    }
    Ok(())
}

pub(crate) fn response_mutations<R: RunEffectRepository + ?Sized>(
    store: &R,
    run: &str,
    response: &HostResponseEnvelope,
    now: i64,
) -> Result<Vec<Mutation>, String> {
    if let HostResponse::DelegatedAgentEvent {
        event:
            Event::PermissionRequested {
                occurrence,
                request_id,
                call_id,
                input,
                ..
            },
    } = &response.payload
    {
        let invocation = store
            .delegated_invocation_for_effect(&response.effect_id.0)
            .map_err(|e| e.to_string())?
            .ok_or("permission occurrence has no invocation")?;
        if invocation.run_id != run {
            return Err("permission occurrence belongs to another Run".into());
        }
        if store
            .delegated_permission_for_occurrence(&invocation.id, &occurrence.occurrence_id)
            .map_err(|e| e.to_string())?
            .is_some()
        {
            return Err("permission occurrence already committed".into());
        }
        let record = Record {
            id: uuid::Uuid::new_v4().to_string(),
            run_id: run.into(),
            invocation_id: invocation.id,
            target_effect_id: response.effect_id.0.clone(),
            worker_generation: occurrence.worker_generation.clone(),
            occurrence_id: occurrence.occurrence_id.clone(),
            backend_id: invocation.backend_id,
            backend_session_id: invocation
                .backend_session_id
                .ok_or("permission occurrence precedes canonical backend session")?,
            rpc_request_id: request_id.clone(),
            call_id: call_id.clone(),
            request_json: input.clone(),
            request_digest: digest(input)?,
            control_effect_id: None,
            control_request_id: None,
            decision: None,
            state: State::AwaitingDecision,
            created_at_ms: now,
            claimed_at_ms: None,
            written_at_ms: None,
            revision: 1,
        };
        return Ok(vec![nola_kernel::prepare_delegated_permission(record)?]);
    }
    if matches!(response.payload, HostResponse::ExecutionStarted) {
        return Ok(vec![]);
    }
    let Some(record) = store
        .delegated_permission_for_control(&response.effect_id.0)
        .map_err(|e| e.to_string())?
    else {
        return Ok(vec![]);
    };
    validate_record(&record, run)?;
    let mutation = match &response.payload {
        HostResponse::DelegatedPermissionClaimed { .. } => {
            nola_kernel::claim_delegated_permission(&record, now)?
        }
        HostResponse::DelegatedPermissionTransportWritten { .. } => {
            nola_kernel::write_delegated_permission(&record, now)?
        }
        HostResponse::DelegatedPermissionInterrupted { .. }
        | HostResponse::ExecutionRejected { .. }
        | HostResponse::Failed { .. } => {
            if matches!(
                record.state,
                State::Interrupted | State::OutcomeUnknown | State::TransportWritten
            ) {
                return Ok(vec![]);
            }
            nola_kernel::interrupt_delegated_permission(&record, now)?
        }
        _ => return Err("permission control requires explicit delivery evidence".into()),
    };
    Ok(vec![mutation])
}

pub(crate) fn prepare_effects<R: RunEffectRepository + ?Sized>(
    store: &R,
    run: &str,
    effects: &mut [OutboxEffectInput],
    changes: &mut Vec<Mutation>,
    now: i64,
) -> Result<(), String> {
    for effect in effects {
        let nola_store_port::run_effect::OutboxEffectWrite::Declare { payload_json } =
            &mut effect.write
        else {
            continue;
        };
        let mut envelope: HostRequestEnvelope =
            serde_json::from_str(payload_json).map_err(|e| e.to_string())?;
        let HostRequest::ResolveDelegatedPermission {
            target_effect_id,
            backend_id,
            backend_session_id,
            permission_request_id,
            occurrence,
            resolution,
            delivery,
            ..
        } = &mut envelope.payload
        else {
            continue;
        };
        let invocation = store
            .delegated_invocation_for_effect(&target_effect_id.0)
            .map_err(|e| e.to_string())?
            .ok_or("permission control target has no invocation")?;
        let record = changes
            .iter()
            .rev()
            .find(|change| {
                change.record.invocation_id == invocation.id
                    && change.record.occurrence_id == occurrence.occurrence_id
            })
            .map(|change| change.record.clone())
            .or(store
                .delegated_permission_for_occurrence(&invocation.id, &occurrence.occurrence_id)
                .map_err(|e| e.to_string())?)
            .ok_or("permission control has no canonical occurrence")?;
        validate_record(&record, run)?;
        if invocation.run_id != run
            || record.target_effect_id != target_effect_id.0
            || record.backend_id != *backend_id
            || backend_session_id.as_deref() != Some(record.backend_session_id.as_str())
            || record.rpc_request_id != *permission_request_id
            || record.worker_generation != occurrence.worker_generation
        {
            return Err("permission control target differs from canonical occurrence".into());
        }
        let decision = decision(resolution)?;
        let prepared = if record.state == State::AwaitingDecision {
            let change = nola_kernel::authorize_delegated_permission(
                &record,
                effect.effect_id.clone(),
                effect.request_id.clone(),
                decision,
                now,
            )?;
            let next = change.record.clone();
            changes.push(change);
            next
        } else {
            if record.control_effect_id.as_deref() != Some(effect.effect_id.as_str())
                || record.control_request_id.as_deref() != Some(effect.request_id.as_str())
                || record.decision != Some(decision)
            {
                return Err(
                    "permission occurrence already has a different control decision".into(),
                );
            }
            record
        };
        let expected = target(&prepared);
        if delivery
            .as_ref()
            .is_some_and(|previous| previous != &expected)
        {
            return Err("permission control delivery identity changed".into());
        }
        *delivery = Some(expected);
        *payload_json = serde_json::to_string(&envelope).map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub(crate) fn validate_delivery_record(
    record: &Record,
    run: &str,
    request: &HostRequestEnvelope,
) -> Result<(), String> {
    validate_delivery_control(
        record,
        run,
        &nola_domain::host_control::HostEffectControl::from_request(request)?,
    )
}

fn validate_delivery_control(
    record: &Record,
    run: &str,
    request: &nola_domain::host_control::HostEffectControl,
) -> Result<(), String> {
    validate_record(record, run)?;
    let nola_domain::host_control::HostResponsePolicy::ResolveDelegatedPermission {
        target_effect_id,
        backend_id,
        backend_session_id,
        permission_request_id,
        occurrence,
        resolution,
        delivery,
        ..
    } = &request.payload.policy
    else {
        return Err("permission record bound to non-control request".into());
    };
    if record.control_effect_id.as_deref() != Some(request.effect_id.0.as_str())
        || record.control_request_id.as_deref() != Some(request.request_id.as_str())
        || record.target_effect_id != target_effect_id.0
        || record.backend_id != *backend_id
        || backend_session_id.as_deref() != Some(record.backend_session_id.as_str())
        || record.rpc_request_id != *permission_request_id
        || occurrence.worker_generation != record.worker_generation
        || occurrence.occurrence_id != record.occurrence_id
        || record.decision != Some(decision(resolution)?)
        || delivery.as_ref() != Some(&target(record))
    {
        return Err("permission delivery differs from canonical intent".into());
    }
    Ok(())
}

pub(crate) fn bind_response_control<R: RunEffectRepository + ?Sized>(
    store: &R,
    run: &str,
    request: &mut nola_domain::host_control::HostEffectControl,
) -> Result<(), String> {
    if !matches!(
        request.payload.policy,
        nola_domain::host_control::HostResponsePolicy::ResolveDelegatedPermission { .. }
    ) {
        return Ok(());
    }
    let record = store
        .delegated_permission_for_control(&request.effect_id.0)
        .map_err(|e| e.to_string())?
        .ok_or("permission response has no canonical delivery")?;
    let mut next = request.clone();
    if let nola_domain::host_control::HostResponsePolicy::ResolveDelegatedPermission {
        delivery,
        ..
    } = &mut next.payload.policy
    {
        let admitted = target(&record);
        if delivery
            .as_ref()
            .is_some_and(|previous| previous != &admitted)
        {
            return Err("permission delivery identity changed".into());
        }
        *delivery = Some(admitted);
    }
    validate_delivery_control(&record, run, &next)?;
    next.validate_bound()?;
    *request = next;
    Ok(())
}

pub(crate) fn validate_response<R: RunEffectRepository + ?Sized>(
    store: &R,
    run: &str,
    request: &nola_domain::host_control::HostEffectControl,
    response: &HostResponseEnvelope,
) -> Result<(), String> {
    if !matches!(
        request.payload.policy,
        nola_domain::host_control::HostResponsePolicy::ResolveDelegatedPermission { .. }
    ) {
        return Ok(());
    }
    if matches!(response.payload, HostResponse::ExecutionStarted) {
        return Ok(());
    }
    let record = store
        .delegated_permission_for_control(&request.effect_id.0)
        .map_err(|e| e.to_string())?
        .ok_or("permission response has no canonical delivery")?;
    validate_delivery_control(&record, run, request)?;
    if matches!(response.payload, HostResponse::ExecutionRejected { .. }) {
        return if record.state == State::Prepared {
            Ok(())
        } else {
            Err("permission control may be rejected only before delivery starts".into())
        };
    }
    let id = match &response.payload {
        HostResponse::DelegatedPermissionClaimed { delivery_id }
        | HostResponse::DelegatedPermissionTransportWritten { delivery_id }
        | HostResponse::DelegatedPermissionInterrupted { delivery_id } => delivery_id,
        _ => return Err("permission control requires typed evidence response".into()),
    };
    if id != &record.id {
        return Err("permission response delivery identity mismatch".into());
    }
    if matches!(
        response.payload,
        HostResponse::DelegatedPermissionClaimed { .. }
    ) {
        let invocation = store
            .delegated_invocation(&record.invocation_id)
            .map_err(|e| e.to_string())?
            .ok_or("permission target invocation missing")?;
        if invocation.run_id != run
            || invocation.backend_session_id.as_deref() != Some(record.backend_session_id.as_str())
            || invocation.state != nola_domain::DelegatedInvocationState::WaitingPermission
            || record.state != State::Prepared
        {
            return Err("permission delivery cannot be claimed in its current state".into());
        }
    }
    if matches!(
        response.payload,
        HostResponse::DelegatedPermissionTransportWritten { .. }
    ) && record.state != State::Claimed
    {
        return Err("permission write evidence requires committed claim".into());
    }
    Ok(())
}

pub(crate) fn bind_delivery<R: RunEffectRepository + ?Sized>(
    store: &R,
    run: &str,
    request: &HostRequestEnvelope,
) -> Result<(), String> {
    if !is_control(request) {
        return Ok(());
    }
    let record = store
        .delegated_permission_for_control(&request.effect_id.0)
        .map_err(|e| e.to_string())?
        .ok_or("permission delivery has no canonical ledger")?;
    validate_delivery_record(&record, run, request)
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
                "delegated-permission:{}:{}",
                change.record.id, change.record.revision
            ),
            sequence: *sequence,
            event_kind: "delegated_permission_changed".into(),
            payload_json: serde_json::to_string(&change.record).map_err(|e| e.to_string())?,
            created_at_ms: now,
        });
        *sequence = sequence.checked_add(1).ok_or("Run sequence exhausted")?;
    }
    Ok(())
}

pub(crate) fn parent_mutations<R: RunEffectRepository + ?Sized>(
    store: &R,
    response: &HostResponseEnvelope,
    now: i64,
) -> Result<Vec<nola_domain::DelegatedInvocationMutation>, String> {
    if !matches!(
        response.payload,
        HostResponse::DelegatedPermissionTransportWritten { .. }
    ) {
        return Ok(vec![]);
    }
    let permission = store
        .delegated_permission_for_control(&response.effect_id.0)
        .map_err(|e| e.to_string())?
        .ok_or("permission write has no delivery")?;
    let invocation = store
        .delegated_invocation(&permission.invocation_id)
        .map_err(|e| e.to_string())?
        .ok_or("permission write target missing")?;
    Ok(vec![nola_kernel::transition_delegated_invocation(
        &invocation,
        nola_domain::DelegatedInvocationState::Running,
        None,
        None,
        None,
        now,
    )?])
}

pub(crate) fn settle_effects<R: RunEffectRepository + ?Sized>(
    store: &R,
    effects: &[String],
    now: i64,
) -> Result<Vec<Mutation>, String> {
    let mut records = std::collections::BTreeMap::new();
    for effect in effects {
        if let Some(record) = store
            .delegated_permission_for_control(effect)
            .map_err(|e| e.to_string())?
        {
            records.insert(record.id.clone(), record);
        }
        for record in store
            .open_delegated_permissions_for_target(effect)
            .map_err(|e| e.to_string())?
        {
            records.insert(record.id.clone(), record);
        }
    }
    records
        .into_values()
        .filter(|record| {
            matches!(
                record.state,
                State::AwaitingDecision | State::Prepared | State::Claimed
            )
        })
        .map(|record| nola_kernel::interrupt_delegated_permission(&record, now))
        .collect()
}

pub(crate) fn should_dispatch<R: RunEffectRepository + ?Sized>(
    store: &R,
    request: &HostRequestEnvelope,
) -> Result<bool, String> {
    if !is_control(request) {
        return Ok(true);
    }
    let record = store
        .delegated_permission_for_control(&request.effect_id.0)
        .map_err(|e| e.to_string())?
        .ok_or("permission delivery missing")?;
    // Claimed may already have crossed the external write boundary. Only the
    // current worker can submit further evidence; cold loading cannot resend it.
    Ok(record.state == State::Prepared)
}

pub(crate) fn validate_recovery_record(
    delivery: &Record,
    facts: &nola_store_port::store_maintenance::RunRecoveryRecord,
) -> Result<(), String> {
    let run = facts
        .run
        .as_ref()
        .ok_or("permission evidence has no canonical Run")?;
    validate_record(delivery, &run.run_id)?;
    let owner = facts
        .delegated_invocations
        .iter()
        .find(|invocation| invocation.id == delivery.invocation_id)
        .ok_or("permission recovery has no canonical invocation owner")?;
    if owner.run_id != run.run_id
        || owner.effect_id != delivery.target_effect_id
        || owner.backend_id != delivery.backend_id
        || owner.backend_session_id.as_deref() != Some(delivery.backend_session_id.as_str())
    {
        return Err("permission recovery target identity mismatch".into());
    }
    Ok(())
}
