// SPDX-License-Identifier: Apache-2.0
//! Shared exact started-delivery authority; callers still validate the concrete payload.
use crate::repositories::AgentRuntimeCommandRepositories;
use nola_agent_engine::agent::host::HostRequestEnvelope;
pub(crate) fn validate<S: AgentRuntimeCommandRepositories>(
    store: &mut S,
    request: &HostRequestEnvelope,
    expected_kind: &str,
    now_ms: i64,
) -> Result<(String, String, nola_domain::EffectDeliveryAttempt), String> {
    validate_delivery(store, request, expected_kind, now_ms, false)
}

/// Showing a question does not begin the requested operation.
pub(crate) fn validate_permission_presentation<S: AgentRuntimeCommandRepositories>(
    store: &mut S, request: &HostRequestEnvelope, now_ms: i64,
) -> Result<(String, String, nola_domain::EffectDeliveryAttempt), String> {
    validate_delivery(store, request, "request_permission", now_ms, true)
}

fn validate_delivery<S: AgentRuntimeCommandRepositories>(
    store: &mut S, request: &HostRequestEnvelope, expected_kind: &str, now_ms: i64,
    presenting_permission: bool,
) -> Result<(String, String, nola_domain::EffectDeliveryAttempt), String> {
    let identity = store
        .outbox_effect_identity(&request.effect_id.0)
        .map_err(|e| e.to_string())?
        .ok_or("Host execution effect is not committed")?;
    if identity.status != "pending" || identity.effect_kind != expected_kind {
        return Err("Host execution effect is no longer pending".into());
    }
    let selected = std::collections::HashSet::from([request.effect_id.0.clone()]);
    let conversation_id = store
        .load_selected_host_delivery_witness(&identity.run_id, Some(&selected))
        .map_err(|e| e.to_string())?
        .conversation_id
        .ok_or("Host execution lacks conversation owner")?;
    let (pending, _) =
        crate::load_selected_committed_host_requests(store, &identity.run_id, Some(&selected))?;
    let frozen = pending
        .requests
        .into_iter()
        .find(|value| value.effect_id == request.effect_id)
        .ok_or("Host execution is absent from the current Core outbox")?;
    let mut submitted = request.clone();
    submitted.delivery = frozen.delivery.clone();
    submitted.resume_after_sequence = frozen.resume_after_sequence;
    submitted.attempt_started = frozen.attempt_started;
    if serde_json::to_value(&submitted).map_err(|e| e.to_string())?
        != serde_json::to_value(&frozen).map_err(|e| e.to_string())?
    {
        return Err("Host execution differs from the committed control".into());
    }
    let lease = request
        .delivery
        .as_ref()
        .ok_or("Host execution requires claimed delivery")?;
    let attempt = store
        .active_effect_delivery_attempt(&request.effect_id.0)
        .map_err(|e| e.to_string())?
        .ok_or("Host execution has no active delivery")?;
    if attempt.run_id != identity.run_id
        || attempt.request_id != request.request_id
        || attempt.id != lease.attempt_id
        || attempt.worker_generation.as_deref() != Some(lease.worker_generation.as_str())
        || attempt.lease_owner.as_deref() != Some(lease.lease_owner.as_str())
        || attempt.lease_token.as_deref() != Some(lease.lease_token.as_str())
        || attempt.lease_expires_at_ms != Some(lease.lease_expires_at_ms)
        || !(attempt.state == nola_domain::EffectDeliveryAttemptState::ExecutionStarted
            || (presenting_permission && attempt.state == nola_domain::EffectDeliveryAttemptState::Leased))
        || now_ms < 0
        || now_ms >= lease.lease_expires_at_ms
        || request.is_expired_at(now_ms as u64)
    {
        return Err("Host execution requires its current started delivery fence".into());
    }
    Ok((identity.run_id, conversation_id, attempt))
}
