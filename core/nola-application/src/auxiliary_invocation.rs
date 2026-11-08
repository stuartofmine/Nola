// SPDX-License-Identifier: Apache-2.0
//! Budget/cancellation admission for every physical auxiliary model request.
use nola_domain::*;
use nola_store_port::auxiliary_invocation::AuxiliaryInvocationRepository;

fn identity(value: &str) -> Result<(), String> {
    if value.trim().is_empty()
        || value.trim() != value
        || value.len() > 256
        || value.chars().any(char::is_control)
    {
        Err("invalid auxiliary identity".into())
    } else {
        Ok(())
    }
}

fn validate_usage(usage: &serde_json::Value) -> Result<(), String> {
    match usage.get("coverage").and_then(|value| value.as_str()) {
        Some("unknown") if usage.get("normalized").is_some_and(|value| value.is_null()) => Ok(()),
        Some("known") => {
            let normalized = usage
                .get("normalized")
                .ok_or("known usage requires normalized counters")?;
            let input = normalized
                .get("input_tokens")
                .and_then(|value| value.as_u64())
                .ok_or("invalid measured input tokens")?;
            normalized
                .get("output_tokens")
                .and_then(|value| value.as_u64())
                .ok_or("invalid measured output tokens")?;
            for key in ["cached_input_tokens", "cache_write_input_tokens"] {
                if let Some(value) = normalized.get(key).filter(|value| !value.is_null()) {
                    if value.as_u64().is_none_or(|count| count > input) {
                        return Err("invalid measured cache counters".into());
                    }
                }
            }
            if !usage
                .get("provider_raw")
                .is_some_and(|value| value.is_object())
            {
                return Err("known usage requires provider raw evidence".into());
            }
            Ok(())
        }
        _ => Err("usage must preserve unknown or measured coverage".into()),
    }
}

pub fn execute_auxiliary_invocation<R: AuxiliaryInvocationRepository>(
    store: &mut R,
    command: AuxiliaryInvocationCommand,
) -> Result<AuxiliaryInvocationReceipt, String> {
    use AuxiliaryInvocationCommand::*;
    let load = |store: &R, id: &str| -> Result<AuxiliaryOperation, String> {
        identity(id)?;
        store
            .auxiliary_operation(id)
            .map_err(|error| error.to_string())?
            .ok_or("auxiliary operation missing".into())
    };
    match command {
        Admit {
            operation_id,
            owner,
            scenario,
            configuration,
            at_ms,
        } => {
            identity(&operation_id)?;
            identity(&configuration.provider)?;
            if configuration
                .model
                .as_ref()
                .is_some_and(|model| model.trim().is_empty() || model.len() > 1024)
            {
                return Err("invalid auxiliary model identity".into());
            }
            let owner_id = match &owner {
                AuxiliaryInvocationOwner::Room { room_id } => room_id,
                AuxiliaryInvocationOwner::Conversation { conversation_id } => conversation_id,
                AuxiliaryInvocationOwner::Account { account_id } => account_id,
                AuxiliaryInvocationOwner::Task { task_id } => task_id,
                AuxiliaryInvocationOwner::SourceAuthority {
                    source_host_id,
                    authority_id,
                } => {
                    identity(source_host_id)?;
                    authority_id
                }
            };
            identity(owner_id)?;
            let digest = |value: &str| {
                value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
            };
            if at_ms < 0
                || !(1..=8).contains(&configuration.max_attempts)
                || configuration.maximum_duration_ms == 0
                || configuration.maximum_duration_ms > 3_600_000
                || !digest(&configuration.input_digest)
                || !digest(&configuration.configuration_digest)
            {
                return Err("invalid auxiliary invocation configuration or total budget".into());
            }
            if configuration.max_output_tokens_per_attempt == Some(0)
                || configuration.total_output_token_budget == Some(0)
                || configuration
                    .total_output_token_budget
                    .is_some_and(|total| {
                        configuration
                            .max_output_tokens_per_attempt
                            .is_none_or(|cap| cap > total)
                    })
            {
                return Err("invalid auxiliary total output token budget".into());
            }
            if let Some(operation) = store
                .auxiliary_operation(&operation_id)
                .map_err(|error| error.to_string())?
            {
                if operation.owner != owner
                    || operation.scenario != scenario
                    || operation.configuration != configuration
                {
                    return Err(
                        "auxiliary operation identity already has different frozen input".into(),
                    );
                }
                let attempt = store
                    .auxiliary_attempt_tail(&operation_id)
                    .map_err(|error| error.to_string())?;
                return Ok(AuxiliaryInvocationReceipt {
                    dispatch_permitted: false,
                    operation,
                    attempt,
                });
            }
            let operation = AuxiliaryOperation {
                id: operation_id,
                owner,
                scenario,
                configuration,
                state: AuxiliaryOperationState::Ready,
                attempts_started: 0,
                active_attempt_id: None,
                cancellation_requested_at_ms: None,
                created_at_ms: at_ms,
                revision: 1,
            };
            store
                .commit_auxiliary_invocation(&operation, None, None, None)
                .map_err(|error| error.to_string())?;
            Ok(AuxiliaryInvocationReceipt {
                dispatch_permitted: false,
                operation,
                attempt: None,
            })
        }
        StartAttempt {
            operation_id,
            attempt_id,
            expected_revision,
            at_ms,
        } => {
            identity(&attempt_id)?;
            let mut operation = load(store, &operation_id)?;
            if let Some(attempt) = store
                .auxiliary_attempt(&operation_id, &attempt_id)
                .map_err(|error| error.to_string())?
            {
                // The caller must not dispatch on a repeated start receipt.
                return Ok(AuxiliaryInvocationReceipt {
                    dispatch_permitted: false,
                    operation,
                    attempt: Some(attempt),
                });
            }
            if operation.revision != expected_revision
                || operation.state != AuxiliaryOperationState::Ready
                || operation.active_attempt_id.is_some()
                || operation.cancellation_requested_at_ms.is_some()
                || operation.attempts_started >= operation.configuration.max_attempts
                || at_ms < operation.created_at_ms
                || u64::try_from(at_ms - operation.created_at_ms).unwrap_or(u64::MAX)
                    > operation.configuration.maximum_duration_ms
            {
                return Err(
                    "auxiliary invocation cancelled, exhausted, expired or no longer ready".into(),
                );
            }
            let predecessor = store
                .auxiliary_attempt_tail(&operation_id)
                .map_err(|error| error.to_string())?;
            if predecessor.as_ref().is_some_and(|attempt| {
                !attempt.state.is_terminal()
                    || attempt
                        .completed_at_ms
                        .is_none_or(|completed| completed > at_ms)
            }) {
                return Err("auxiliary predecessor has no completed physical boundary".into());
            }
            if let Some(total) = operation.configuration.total_output_token_budget {
                let cap = operation
                    .configuration
                    .max_output_tokens_per_attempt
                    .ok_or("total output budget requires a per-Attempt reservation")?;
                // Unknown usage reserves the full admitted cap. A provider
                // failure cannot reset the operation's total token budget.
                if cap
                    .checked_mul(u64::from(operation.attempts_started) + 1)
                    .is_none_or(|reserved| reserved > total)
                {
                    return Err("auxiliary total output token budget exhausted".into());
                }
            }
            let attempt = AuxiliaryModelAttempt {
                id: attempt_id.clone(),
                operation_id,
                predecessor_id: predecessor.map(|attempt| attempt.id),
                state: ModelAttemptState::Running,
                provider: operation.configuration.provider.clone(),
                model: operation.configuration.model.clone(),
                usage: serde_json::json!({"coverage":"unknown","provider_raw":null,"normalized":null}),
                failure: None,
                output: None,
                created_at_ms: at_ms,
                completed_at_ms: None,
                revision: 1,
            };
            operation.state = AuxiliaryOperationState::Running;
            operation.active_attempt_id = Some(attempt_id);
            operation.attempts_started += 1;
            operation.revision += 1;
            store
                .commit_auxiliary_invocation(
                    &operation,
                    Some(expected_revision),
                    Some(&attempt),
                    None,
                )
                .map_err(|error| error.to_string())?;
            Ok(AuxiliaryInvocationReceipt {
                dispatch_permitted: true,
                operation,
                attempt: Some(attempt),
            })
        }
        RecordReceipt {
            operation_id,
            attempt_id,
            expected_attempt_revision,
            state,
            usage,
            failure,
            output,
            at_ms,
        } => {
            if !matches!(
                state,
                ModelAttemptState::Succeeded
                    | ModelAttemptState::Failed
                    | ModelAttemptState::Cancelled
            ) {
                return Err("invalid auxiliary terminal receipt or usage coverage".into());
            }
            validate_usage(&usage)?;
            let mut operation = load(store, &operation_id)?;
            let mut attempt = store
                .auxiliary_attempt(&operation_id, &attempt_id)
                .map_err(|error| error.to_string())?
                .ok_or("auxiliary Attempt missing or belongs to another operation")?;
            if output.is_some() && state != ModelAttemptState::Succeeded {
                return Err("only successful auxiliary receipts can publish output".into());
            }
            if attempt.state == state
                && attempt.usage == usage
                && attempt.failure == failure
                && attempt.output == output
            {
                return Ok(AuxiliaryInvocationReceipt {
                    dispatch_permitted: false,
                    operation,
                    attempt: Some(attempt),
                });
            }
            if attempt.revision != expected_attempt_revision
                || at_ms < attempt.created_at_ms
                || (attempt.state.is_terminal() && attempt.state != state)
            {
                return Err("auxiliary Attempt revision or terminal state changed".into());
            }
            if attempt
                .usage
                .get("coverage")
                .and_then(|value| value.as_str())
                == Some("known")
                && usage.get("coverage").and_then(|value| value.as_str()) != Some("known")
            {
                return Err("measured Attempt usage cannot become unknown".into());
            }
            if !attempt.state.is_terminal()
                && operation.active_attempt_id.as_deref() != Some(attempt_id.as_str())
            {
                return Err("auxiliary operation no longer owns this physical Attempt".into());
            }
            let old_revision = operation.revision;
            if attempt.output.is_some() && attempt.output != output {
                return Err("committed auxiliary result cannot be replaced".into());
            }
            let was_terminal = attempt.state.is_terminal();
            attempt.state = state;
            attempt.usage = usage;
            attempt.failure = failure;
            attempt.output = output;
            if !was_terminal {
                attempt.completed_at_ms = Some(at_ms);
            }
            attempt.revision += 1;
            if !was_terminal {
                operation.active_attempt_id = None;
                operation.state = if operation.cancellation_requested_at_ms.is_some()
                    || state == ModelAttemptState::Cancelled
                {
                    AuxiliaryOperationState::Cancelled
                } else if state == ModelAttemptState::Succeeded {
                    AuxiliaryOperationState::Succeeded
                } else if operation.attempts_started < operation.configuration.max_attempts {
                    AuxiliaryOperationState::Ready
                } else {
                    AuxiliaryOperationState::Failed
                };
            }
            operation.revision += 1;
            store
                .commit_auxiliary_invocation(
                    &operation,
                    Some(old_revision),
                    Some(&attempt),
                    Some(expected_attempt_revision),
                )
                .map_err(|error| error.to_string())?;
            Ok(AuxiliaryInvocationReceipt {
                dispatch_permitted: false,
                operation,
                attempt: Some(attempt),
            })
        }
        Cancel {
            operation_id,
            at_ms,
        } => {
            let mut operation = load(store, &operation_id)?;
            let old_revision = operation.revision;
            if operation.cancellation_requested_at_ms.is_none() {
                if at_ms < operation.created_at_ms {
                    return Err("invalid cancellation time".into());
                }
                operation.cancellation_requested_at_ms = Some(at_ms);
                if operation.active_attempt_id.is_none()
                    && matches!(
                        operation.state,
                        AuxiliaryOperationState::Ready | AuxiliaryOperationState::Running
                    )
                {
                    operation.state = AuxiliaryOperationState::Cancelled;
                }
                operation.revision += 1;
                store
                    .commit_auxiliary_invocation(&operation, Some(old_revision), None, None)
                    .map_err(|error| error.to_string())?;
            }
            let attempt = store
                .auxiliary_attempt_tail(&operation_id)
                .map_err(|error| error.to_string())?;
            Ok(AuxiliaryInvocationReceipt {
                dispatch_permitted: false,
                operation,
                attempt,
            })
        }
        Inspect { operation_id } => {
            let operation = load(store, &operation_id)?;
            let attempt = store
                .auxiliary_attempt_tail(&operation_id)
                .map_err(|error| error.to_string())?;
            Ok(AuxiliaryInvocationReceipt {
                dispatch_permitted: false,
                operation,
                attempt,
            })
        }
        ObserveSource {
            local_owner,
            observation,
        } => {
            validate_source_observation(&local_owner, &observation)?;
            store
                .commit_auxiliary_source_observation(&local_owner, &observation)
                .map_err(|error| error.to_string())?;
            Ok(observation.receipt)
        }
        InspectSource {
            local_owner,
            source_host_id,
            operation_id,
        } => {
            let observation = store
                .auxiliary_source_observation(&local_owner, &source_host_id, &operation_id)
                .map_err(|error| error.to_string())?
                .ok_or("auxiliary source observation missing")?;
            Ok(observation.receipt)
        }
    }
}

fn validate_source_observation(
    local_owner: &AuxiliaryInvocationOwner,
    source: &AuxiliarySourceObservation,
) -> Result<(), String> {
    if matches!(
        local_owner,
        AuxiliaryInvocationOwner::SourceAuthority { .. }
    ) {
        return Err("source observations require an explicit local product owner".into());
    }
    identity(&source.source_host_id)?;
    identity(&source.operation_id)?;
    identity(&source.attempt_id)?;
    let receipt = &source.receipt;
    let owner_host = match &receipt.operation.owner {
        AuxiliaryInvocationOwner::SourceAuthority {
            source_host_id,
            authority_id,
        } => {
            identity(authority_id)?;
            source_host_id
        }
        _ => return Err("remote auxiliary receipt has no source authority".into()),
    };
    if owner_host != &source.source_host_id
        || receipt.operation.id != source.operation_id
        || source.attempt_id != format!("{}:physical", source.operation_id)
        || receipt.dispatch_permitted
        || receipt.operation.configuration.max_attempts != 1
        || receipt.operation.attempts_started > 1
        || (receipt.operation.attempts_started == 0 && receipt.attempt.is_some())
        || (receipt.operation.attempts_started == 1
            && receipt.attempt.as_ref().is_none_or(|attempt| {
                attempt.id != source.attempt_id || attempt.operation_id != source.operation_id
            }))
    {
        return Err("auxiliary source receipt identity mismatch".into());
    }
    if let Some(attempt) = &receipt.attempt {
        if attempt.provider != receipt.operation.configuration.provider
            || attempt.model != receipt.operation.configuration.model
            || attempt.predecessor_id.is_some()
            || (attempt.state == ModelAttemptState::Running
                && (receipt.operation.state != AuxiliaryOperationState::Running
                    || receipt.operation.active_attempt_id.as_deref() != Some(attempt.id.as_str())))
            || (attempt.state.is_terminal() && receipt.operation.active_attempt_id.is_some())
        {
            return Err("auxiliary source physical Attempt binding mismatch".into());
        }
        validate_usage(&attempt.usage)?;
    }
    Ok(())
}
