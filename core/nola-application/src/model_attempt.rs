// SPDX-License-Identifier: Apache-2.0
//! Application-owned model sampling admission and atomic ledger planning.

use crate::*;
use nola_domain::{
    ModelAttemptFailure, ModelAttemptMutation, ModelAttemptRecord, ModelAttemptState,
};

fn bounded_failure_message(value: &str) -> String {
    let compact = value.split_whitespace().collect::<Vec<_>>().join(" ");
    compact.chars().take(1_024).collect()
}

fn progress_failure(metadata: &serde_json::Value) -> Option<ModelAttemptFailure> {
    let object = metadata.as_object()?;
    let code = object.get("failure_code")?.as_str()?.trim();
    let message = object.get("failure_message")?.as_str()?.trim();
    if code.is_empty() || message.is_empty() {
        return None;
    }
    Some(ModelAttemptFailure {
        code: code.chars().take(128).collect(),
        message: bounded_failure_message(message),
        retryable: object
            .get("retryable")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true),
        http_status: object
            .get("provider_status")
            .and_then(serde_json::Value::as_u64)
            .and_then(|status| u16::try_from(status).ok()),
    })
}

#[cfg(test)]
mod provider_failure_contract_tests {
    use super::progress_failure;

    #[test]
    fn retry_attempt_retains_safe_http_status_without_provider_body() {
        let failure = progress_failure(&serde_json::json!({
            "failure_code": "invalid_model_request",
            "failure_message": "The provider rejected the model request.",
            "retryable": false,
            "provider_status": 400,
            "diagnostic": "private provider response body",
        }))
        .unwrap();
        assert_eq!(failure.http_status, Some(400));
        assert_eq!(failure.code, "invalid_model_request");
        assert!(!serde_json::to_string(&failure)
            .unwrap()
            .contains("private provider"));
    }
}

pub(crate) fn is_model_request(request: &HostRequestEnvelope) -> bool {
    matches!(
        request.payload,
        HostRequest::ModelStream { .. } | HostRequest::Compact { .. }
    )
}

pub(crate) fn prepare_effects<R: RunEffectRepository + ?Sized>(
    store: &R,
    run_id: &str,
    effects: &mut [OutboxEffectInput],
    now: i64,
) -> Result<Vec<ModelAttemptMutation>, String> {
    let mut mutations = Vec::new();
    for effect in effects {
        let nola_store_port::run_effect::OutboxEffectWrite::Declare { payload_json } =
            &mut effect.write
        else {
            // Core recovery regenerates transport bindings, not Application's
            // sampling identity. Preserve the existing admission even when no
            // new sampling mutation occurs (e.g. resuming an answered wait).
            if let nola_store_port::run_effect::OutboxEffectWrite::Rebind { binding, .. } = &mut effect.write {
                if let Some(record) = store.active_model_attempt_for_effect(&effect.effect_id)
                    .map_err(|error| format!("load rebound model Attempt: {error}"))? {
                    nola_kernel::validate_model_attempt(&record)?;
                    if record.run_id != run_id || record.effect_id != effect.effect_id || record.state.is_terminal() {
                        return Err("rebound model Attempt belongs to another Run/effect or is terminal".into());
                    }
                    effect.model_attempt_id = Some(record.id.clone());
                    binding.attempt_started = record.started_at_ms.is_some();
                    binding.attempt_id = Some(record.id);
                }
            }
            continue;
        };
        let mut request: HostRequestEnvelope = serde_json::from_str(payload_json)
            .map_err(|error| format!("decode model admission request: {error}"))?;
        if !is_model_request(&request) {
            continue;
        }
        let record = match store
            .active_model_attempt_for_effect(&effect.effect_id)
            .map_err(|error| format!("load model Attempt: {error}"))?
        {
            Some(record) => {
                if record.run_id != run_id || record.effect_id != effect.effect_id {
                    return Err("model Attempt belongs to another Run/effect".into());
                }
                record
            }
            None => {
                let id = uuid::Uuid::new_v4().to_string();
                let (text, reasoning) = match &request.payload {
                    HostRequest::ModelStream {
                        text_part_id,
                        reasoning_part_id,
                        ..
                    } => (text_part_id.clone(), reasoning_part_id.clone()),
                    _ => (format!("{id}:text"), format!("{id}:reasoning")),
                };
                let record = nola_kernel::prepare_model_attempt(
                    run_id,
                    &effect.effect_id,
                    &effect.effect_id,
                    &id,
                    &text,
                    &reasoning,
                    now,
                )?;
                mutations.push(ModelAttemptMutation {
                    expected_revision: None,
                    record: record.clone(),
                });
                record
            }
        };
        effect.model_attempt_id = Some(record.id.clone());
        request.attempt_started = record.started_at_ms.is_some();
        request.attempt_id = Some(record.id);
        *payload_json = serde_json::to_string(&request).map_err(|error| error.to_string())?;
    }
    Ok(mutations)
}

pub(crate) fn validate_response<R: RunEffectRepository + ?Sized>(
    store: &R,
    run_id: &str,
    request: &nola_domain::host_control::HostEffectControl,
    response: &HostResponseEnvelope,
) -> Result<Option<ModelAttemptRecord>, String> {
    if !matches!(
        request.payload.policy,
        nola_domain::host_control::HostResponsePolicy::ModelStream { .. }
            | nola_domain::host_control::HostResponsePolicy::Compact { .. }
    ) {
        if response.attempt_id.is_some() {
            return Err("non-model response carries a model Attempt".into());
        }
        return Ok(None);
    }
    let current = store
        .active_model_attempt_for_effect(&request.effect_id.0)
        .map_err(|error| format!("load current model Attempt: {error}"))?
        .ok_or("model effect has no canonical Attempt")?;
    if current.run_id != run_id || current.effect_id != request.effect_id.0 {
        return Err("model response Attempt Run/effect mismatch".into());
    }
    let proposed = response
        .attempt_id
        .as_deref()
        .ok_or("model response has no Attempt identity")?;
    if matches!(response.payload, HostResponse::ModelAttemptStarted { .. }) {
        if current.state == ModelAttemptState::Prepared && proposed != current.id {
            return Err("initial sampling must activate the prepared Attempt".into());
        }
    } else {
        if proposed != current.id {
            return Err("model response belongs to a retired Attempt".into());
        }
        // Host setup/progress can precede sampling. Model content and completion
        // require the durable admission that authorizes the physical request.
        if matches!(
            response.payload,
            HostResponse::ModelDelta { .. }
                | HostResponse::ModelUsageReported { .. }
                | HostResponse::ModelCommittedDelta { .. }
                | HostResponse::ModelToolCallDelta { .. }
                | HostResponse::ModelContinuation { .. }
                | HostResponse::ModelCompleted { .. }
                | HostResponse::CompactionCompleted { .. }
        ) && current.state != ModelAttemptState::Running
        {
            return Err("model output arrived before durable sampling admission".into());
        }
        match &response.payload {
            HostResponse::ModelDelta {
                channel, part_id, ..
            } => {
                let expected = match channel {
                    nola_agent_engine::agent::host::ModelDeltaChannel::Text => {
                        &current.text_part_id
                    }
                    nola_agent_engine::agent::host::ModelDeltaChannel::Reasoning => {
                        &current.reasoning_part_id
                    }
                };
                if part_id != expected {
                    return Err("model delta Part belongs to another Attempt".into());
                }
            }
            HostResponse::ModelCommittedDelta { part_id, .. }
                if part_id != &current.text_part_id =>
            {
                return Err("committed model Part belongs to another Attempt".into())
            }
            _ => {}
        }
    }
    Ok(Some(current))
}

/// Identity-only admission for a response to an exact cancelled/retired target.
/// The caller proves the live Cancel relationship; this never mutates a ledger.
pub(crate) fn validate_retired_response<R: RunEffectRepository + ?Sized>(
    store: &R,
    run_id: &str,
    request: &nola_domain::host_control::HostEffectControl,
    response: &HostResponseEnvelope,
) -> Result<(), String> {
    if !matches!(request.payload.policy,
        nola_domain::host_control::HostResponsePolicy::ModelStream { .. }
            | nola_domain::host_control::HostResponsePolicy::Compact { .. }) {
        return if request.attempt_id.is_none() && response.attempt_id.is_none() { Ok(()) }
            else { Err("non-model retired response carries a model Attempt".into()) };
    }
    let current = store.active_model_attempt_for_effect(&request.effect_id.0)
        .map_err(|error| format!("load retired model Attempt: {error}"))?
        .ok_or("retired model effect has no canonical Attempt")?;
    validate_retired_model_response(&current, run_id, request, response)
}

fn validate_retired_model_response(
    current: &ModelAttemptRecord,
    run_id: &str,
    request: &nola_domain::host_control::HostEffectControl,
    response: &HostResponseEnvelope,
) -> Result<(), String> {
    nola_kernel::validate_model_attempt(current)?;
    if current.run_id != run_id || current.effect_id != request.effect_id.0
        || current.state != ModelAttemptState::Cancelled
        || request.attempt_id.as_deref() != Some(current.id.as_str())
        || response.attempt_id.as_deref() != Some(current.id.as_str()) {
        return Err("retired response differs from its cancelled model Attempt".into());
    }
    match &response.payload {
        HostResponse::ModelDelta { channel, part_id, .. } => {
            let expected = match channel {
                nola_agent_engine::agent::host::ModelDeltaChannel::Text => &current.text_part_id,
                nola_agent_engine::agent::host::ModelDeltaChannel::Reasoning => &current.reasoning_part_id,
            };
            if part_id != expected { return Err("retired delta Part belongs to another Attempt".into()); }
        }
        HostResponse::ModelCommittedDelta { part_id, .. } if part_id != &current.text_part_id => {
            return Err("retired committed Part belongs to another Attempt".into());
        }
        HostResponse::ExecutionStarted | HostResponse::ModelAttemptStarted { .. } => {
            return Err("retired model Attempt cannot restart".into());
        }
        _ => {}
    }
    Ok(())
}

pub(crate) fn response_mutations<R: RunEffectRepository + ?Sized>(
    store: &R,
    run_id: &str,
    response: &HostResponseEnvelope,
    now: i64,
) -> Result<Vec<ModelAttemptMutation>, String> {
    let Some(current) = store
        .active_model_attempt_for_effect(&response.effect_id.0)
        .map_err(|error| format!("load accepted model Attempt: {error}"))?
    else {
        return Ok(Vec::new());
    };
    if current.run_id != run_id {
        return Err("accepted model Attempt Run mismatch".into());
    }
    match &response.payload {
        HostResponse::ModelUsageReported { usage } => Ok(vec![nola_kernel::record_model_attempt_usage(
            &current, usage.clone(), now,
        )?]),
        HostResponse::ModelAttemptStarted {
            provider,
            model,
            reason,
        } => nola_kernel::start_model_attempt(
            &current,
            response
                .attempt_id
                .as_deref()
                .ok_or("sampling admission has no Attempt ID")?,
            provider,
            model,
            reason,
            now,
        ),
        HostResponse::ModelCompleted { usage, .. }
        | HostResponse::ModelOutputLimited { usage, .. }
        | HostResponse::CompactionCompleted { usage, .. } => {
            Ok(vec![nola_kernel::finish_model_attempt(
                &current,
                ModelAttemptState::Succeeded,
                None,
                // ModelCompleted may carry aggregate logical-turn usage after
                // recovery calls. Keep the exact physical snapshot if reported.
                if current.usage.as_object().is_some_and(|fields| !fields.is_empty()) {
                    current.usage.clone()
                } else { usage.clone() },
                now,
            )?])
        }
        HostResponse::Failed { error } => Ok(vec![nola_kernel::finish_model_attempt(
            &current,
            ModelAttemptState::Failed,
            Some(ModelAttemptFailure {
                code: error.code.clone(),
                message: bounded_failure_message(&error.message),
                retryable: error.retryable,
                http_status: error.http_status,
            }),
            serde_json::json!({}),
            now,
        )?]),
        HostResponse::ExecutionRejected {
            code,
            message,
            retryable,
        } => Ok(vec![nola_kernel::finish_model_attempt(
            &current,
            ModelAttemptState::Failed,
            Some(ModelAttemptFailure {
                code: code.clone(),
                message: bounded_failure_message(message),
                retryable: *retryable,
                http_status: None,
            }),
            serde_json::json!({}),
            now,
        )?]),
        HostResponse::CancellationConfirmed { .. } => Ok(vec![nola_kernel::finish_model_attempt(
            &current,
            ModelAttemptState::Cancelled,
            None,
            serde_json::json!({}),
            now,
        )?]),
        HostResponse::ToolProgress {
            title, metadata, ..
        } if matches!(title.as_str(), "model_retry" | "compaction_retry") => {
            let Some(failure) = progress_failure(metadata) else {
                return Ok(Vec::new());
            };
            Ok(vec![nola_kernel::record_model_attempt_failure(
                &current, failure, now,
            )?])
        }
        _ => Ok(Vec::new()),
    }
}

pub(crate) fn bind_mutated_effects(
    effects: &mut [OutboxEffectInput],
    mutations: &[ModelAttemptMutation],
) -> Result<(), String> {
    for effect in effects {
        if let Some(mutation) = mutations
            .iter()
            .rev()
            .find(|m| m.record.effect_id == effect.effect_id)
        {
            effect.model_attempt_id = Some(mutation.record.id.clone());
            match &mut effect.write {
                nola_store_port::run_effect::OutboxEffectWrite::Declare { payload_json } => {
                    let mut request: HostRequestEnvelope =
                        serde_json::from_str(payload_json).map_err(|e| e.to_string())?;
                    request.attempt_started = mutation.record.started_at_ms.is_some();
                    request.attempt_id = Some(mutation.record.id.clone());
                    *payload_json = serde_json::to_string(&request).map_err(|e| e.to_string())?;
                }
                nola_store_port::run_effect::OutboxEffectWrite::Rebind { binding, .. } => {
                    binding.attempt_started = mutation.record.started_at_ms.is_some();
                    binding.attempt_id = Some(mutation.record.id.clone());
                }
            }
        }
    }
    Ok(())
}

pub(crate) fn bind_delivery<R: RunEffectRepository + ?Sized>(
    store: &R,
    run_id: &str,
    request: &mut HostRequestEnvelope,
) -> Result<(), String> {
    if !is_model_request(request) {
        return Ok(());
    }
    let current = store
        .active_model_attempt_for_effect(&request.effect_id.0)
        .map_err(|error| format!("load executable Attempt: {error}"))?
        .ok_or("committed model effect has no Attempt ledger")?;
    nola_kernel::validate_model_attempt(&current)?;
    if current.run_id != run_id
        || current.effect_id != request.effect_id.0
        || current.state.is_terminal()
    {
        return Err("committed model effect has no executable Attempt".into());
    }
    if request.attempt_id.as_deref() != Some(current.id.as_str())
        || request.attempt_started != current.started_at_ms.is_some()
    {
        return Err("outbox delivery differs from its canonical Attempt".into());
    }
    request.attempt_started = current.started_at_ms.is_some();
    request.attempt_id = Some(current.id);
    Ok(())
}

pub(crate) fn settle_effects<R: RunEffectRepository + ?Sized>(
    store: &R,
    effect_ids: &[String],
    now: i64,
) -> Result<Vec<ModelAttemptMutation>, String> {
    let mut mutations = Vec::new();
    for effect in effect_ids {
        if let Some(current) = store
            .active_model_attempt_for_effect(effect)
            .map_err(|error| format!("load settled model Attempt: {error}"))?
        {
            if !current.state.is_terminal() {
                mutations.push(nola_kernel::finish_model_attempt(
                    &current,
                    ModelAttemptState::Cancelled,
                    Some(ModelAttemptFailure {
                        code: "model_effect_retired".into(),
                        message: "Model effect retired by committed submission".into(),
                        retryable: false,
                        http_status: None,
                    }),
                    current.usage.clone(),
                    now,
                )?);
            }
        }
    }
    Ok(mutations)
}

pub(crate) fn append_events(
    events: &mut Vec<RunEventInput>,
    sequence: &mut i64,
    mutations: &[ModelAttemptMutation],
    now: i64,
) -> Result<(), String> {
    for mutation in mutations {
        events.push(RunEventInput {
            event_id: format!(
                "model-attempt:{}:{}",
                mutation.record.id, mutation.record.revision
            ),
            sequence: *sequence,
            event_kind: "model_attempt_changed".into(),
            payload_json: serde_json::to_string(&mutation.record)
                .map_err(|error| error.to_string())?,
            created_at_ms: now,
        });
        *sequence = sequence
            .checked_add(1)
            .ok_or("Run event sequence exhausted")?;
    }
    Ok(())
}
