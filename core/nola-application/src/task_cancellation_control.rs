// SPDX-License-Identifier: Apache-2.0
//! Prompt task stop intent from the committed parent control outbox. This is
//! not proof of child termination; its worker still seals the exact child Run.
use nola_agent_engine::agent::host::{HostRequest, HostRequestEnvelope};
use crate::repositories::AgentRuntimeCommandRepositories;

pub(crate) fn claim<S: AgentRuntimeCommandRepositories>(
    store: &mut S, input: crate::ClaimCommittedHostRequests,
) -> Result<crate::CommittedHostRequests, String> {
    let pending = crate::load_committed_host_requests(store, &input.run_id)?;
    let allowed = pending.requests.iter().filter(|request|
        matches!(request.payload, HostRequest::CancelTask { .. }))
        .map(|request| request.effect_id.0.clone()).collect();
    crate::effect_delivery::claim_committed_host_requests_selected(store, input, Some(&allowed))
}

pub(crate) fn execute<S: AgentRuntimeCommandRepositories>(
    store: &mut S, request: &HostRequestEnvelope, now_ms: i64,
) -> Result<nola_domain::agent_task_admission::TaskCancellationExecution, String> {
    let HostRequest::CancelTask { task_id } = &request.payload else {
        return Err("task cancellation requires its exact committed control".into());
    };
    let (run_id,conversation_id,attempt)=crate::started_host_effect::validate(store,request,"cancel_task",now_ms)?;
    let lease=request.delivery.as_ref().ok_or("task cancellation requires claimed delivery")?;
    use nola_domain::agent_task_admission::{AgentTaskCancellationAuthorization, TaskCancellationExecution};
    let Some(task) = store.runtime_task(task_id).map_err(|e|e.to_string())? else {
        let (source_run, cursor) = store.source_agent_task_admission_location(&conversation_id, task_id)
            .map_err(|e|e.to_string())?.ok_or("task cancellation lacks original source admission")?;
        if source_run != run_id { return Err("task cancellation source Run changed".into()); }
        let admission = crate::agent_task_admission::export(store, &conversation_id, &source_run, task_id, cursor)?;
        if admission.parent_core_session_id != request.session_id
            || Some(admission.parent_turn_id.as_str()) != request.turn_id.as_deref() {
            return Err("task cancellation source owner differs from admission".into());
        }
        let deadline_ms = lease.lease_expires_at_ms.min(request.deadline_ms
            .and_then(|value|i64::try_from(value).ok()).unwrap_or(i64::MAX));
        let authorization = AgentTaskCancellationAuthorization {
            admission, source_host_id: request.host_id.0.clone(), control_attempt: attempt, deadline_ms,
        };
        authorization.validate()?;
        return Ok(TaskCancellationExecution::Transfer { authorization });
    };
    if task.owner_session_id.as_deref() != Some(conversation_id.as_str()) {
        return Err("task cancellation target belongs to another parent".into());
    }
    // Immutable source admission, when present, must name the very parent Run
    // that committed this control; an unrelated current Run cannot take over.
    if let Some(authority) = store.runtime_agent_task_authority(task_id).map_err(|e|e.to_string())? {
        authority.validate()?;
        if let Some(admission) = authority.admission {
            if admission.parent_conversation_id != conversation_id
                || admission.parent_core_session_id != request.session_id
                || admission.parent_run_id != run_id
                || Some(admission.parent_turn_id.as_str()) != request.turn_id.as_deref() {
                return Err("task cancellation source Run differs from its admission".into());
            }
        }
    }
    store.request_runtime_task_cancel(task_id, now_ms).map_err(|e|e.to_string())?;
    let persisted = store.runtime_task(task_id).map_err(|e|e.to_string())?.ok_or("task cancellation target disappeared")?;
    Ok(TaskCancellationExecution::Applied { cancel_requested: persisted.cancel_requested })
}

pub(crate) fn import<S: AgentRuntimeCommandRepositories>(store: &mut S,
    authorization: nola_domain::agent_task_admission::AgentTaskCancellationAuthorization,
    now_ms: i64,
) -> Result<bool, String> {
    authorization.validate()?;
    if now_ms < authorization.control_attempt.execution_started_at_ms.unwrap_or(i64::MAX)
        || now_ms >= authorization.deadline_ms {
        return Err("task cancellation transfer is outside its exact delivery lifetime".into());
    }
    let admission = &authorization.admission;
    let authority = store.runtime_agent_task_authority(&admission.task_id).map_err(|e|e.to_string())?
        .ok_or("task cancellation destination has no admission")?;
    authority.validate()?;
    if authority.admission.as_ref() != Some(admission) {
        return Err("task cancellation destination differs from source admission".into());
    }
    let task = store.runtime_task(&admission.task_id).map_err(|e|e.to_string())?
        .ok_or("task cancellation destination task is missing")?;
    if task.owner_session_id.as_deref() != Some(admission.parent_conversation_id.as_str())
        || task.child_session_id.as_deref() != Some(admission.child_conversation_id.as_str()) {
        return Err("task cancellation destination owner differs from admission".into());
    }
    store.request_runtime_task_cancel(&admission.task_id, now_ms).map_err(|e|e.to_string())?;
    Ok(store.runtime_task(&admission.task_id).map_err(|e|e.to_string())?
        .ok_or("task cancellation destination disappeared")?.cancel_requested)
}
