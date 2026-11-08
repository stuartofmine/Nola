// SPDX-License-Identifier: Apache-2.0
//! Pure model-attempt admission. Callers atomically commit every returned mutation.
use nola_domain::{
    ModelAttemptFailure, ModelAttemptMutation, ModelAttemptRecord, ModelAttemptState,
};
use ModelAttemptState::*;

fn identity(value: &str) -> Result<(), String> {
    if value.trim().is_empty() || value.len() > 512 || value.chars().any(char::is_control) {
        return Err("invalid model attempt identity".into());
    }
    Ok(())
}

pub fn validate_model_attempt(record: &ModelAttemptRecord) -> Result<(), String> {
    for id in [
        &record.id,
        &record.run_id,
        &record.invocation_id,
        &record.effect_id,
        &record.text_part_id,
        &record.reasoning_part_id,
    ] {
        identity(id)?;
    }
    if record.text_part_id == record.reasoning_part_id {
        return Err("attempt Parts must be distinct".into());
    }
    if let Some(predecessor) = &record.predecessor_id {
        identity(predecessor)?;
        if predecessor == &record.id {
            return Err("attempt cannot precede itself".into());
        }
    }
    if record.revision <= 0
        || record.created_at_ms < 0
        || record
            .started_at_ms
            .is_some_and(|t| t < record.created_at_ms)
        || record
            .completed_at_ms
            .is_some_and(|t| t < record.started_at_ms.unwrap_or(record.created_at_ms))
    {
        return Err("invalid model attempt revision or timestamps".into());
    }
    if record.state.is_terminal() != record.completed_at_ms.is_some() {
        return Err("model attempt terminal timestamp mismatch".into());
    }
    if record.provider.is_some() != record.model.is_some() {
        return Err("incomplete provider binding".into());
    }
    if let Some(provider) = &record.provider {
        identity(provider)?;
    }
    if let Some(model) = &record.model {
        identity(model)?;
    }
    if record.started_at_ms.is_some() != record.provider.is_some()
        || (matches!(record.state, Running | Interrupted | Succeeded | Superseded)
            && record.started_at_ms.is_none())
        || (record.state == Prepared && record.started_at_ms.is_some())
    {
        return Err("model attempt state/provider binding mismatch".into());
    }
    if !record.usage.is_object() {
        return Err("model attempt usage must be an object".into());
    }
    if let Some(failure) = &record.failure {
        if failure.code.trim().is_empty()
            || failure.code.len() > 128
            || failure.message.trim().is_empty()
            || failure.message.len() > 1_024
            || failure.code.chars().any(char::is_control)
            || failure.message.chars().any(char::is_control)
        {
            return Err("invalid model attempt failure".into());
        }
    }
    Ok(())
}

fn next(current: &ModelAttemptRecord, now: i64) -> Result<ModelAttemptRecord, String> {
    validate_model_attempt(current)?;
    if current.state.is_terminal() {
        return Err("terminal model attempt cannot change".into());
    }
    if now < current.started_at_ms.unwrap_or(current.created_at_ms) {
        return Err("attempt time moved backwards".into());
    }
    let mut record = current.clone();
    record.revision = record
        .revision
        .checked_add(1)
        .ok_or("attempt revision exhausted")?;
    Ok(record)
}

pub fn prepare_model_attempt(
    run_id: &str,
    invocation_id: &str,
    effect_id: &str,
    id: &str,
    text_part_id: &str,
    reasoning_part_id: &str,
    now: i64,
) -> Result<ModelAttemptRecord, String> {
    let record = ModelAttemptRecord {
        id: id.into(),
        run_id: run_id.into(),
        invocation_id: invocation_id.into(),
        effect_id: effect_id.into(),
        text_part_id: text_part_id.into(),
        reasoning_part_id: reasoning_part_id.into(),
        predecessor_id: None,
        state: Prepared,
        provider: None,
        model: None,
        failure: None,
        usage: serde_json::json!({}),
        created_at_ms: now,
        started_at_ms: None,
        completed_at_ms: None,
        revision: 1,
    };
    validate_model_attempt(&record)?;
    Ok(record)
}

pub fn start_model_attempt(
    current: &ModelAttemptRecord,
    new_id: &str,
    provider: &str,
    model: &str,
    reason: &str,
    now: i64,
) -> Result<Vec<ModelAttemptMutation>, String> {
    identity(new_id)?;
    identity(provider)?;
    identity(model)?;
    if reason.trim().is_empty() || reason.len() > 4096 {
        return Err("invalid attempt admission reason".into());
    }
    let mut previous = next(current, now)?;
    if current.state == Prepared {
        if new_id != current.id {
            return Err("prepared admission must retain its identity".into());
        }
        previous.state = Running;
        previous.provider = Some(provider.into());
        previous.model = Some(model.into());
        previous.started_at_ms = Some(now);
        validate_model_attempt(&previous)?;
        return Ok(vec![ModelAttemptMutation {
            expected_revision: Some(current.revision),
            record: previous,
        }]);
    }
    if !matches!(current.state, Running | Interrupted) || new_id == current.id {
        return Err("resampling requires a fresh attempt identity".into());
    }
    previous.state = Superseded;
    previous.completed_at_ms = Some(now);
    if previous.failure.is_none() {
        previous.failure = Some(ModelAttemptFailure {
            code: "model_resampled".into(),
            message: reason.into(),
            retryable: true,
            http_status: None,
        });
    }
    let mut fresh = prepare_model_attempt(
        &current.run_id,
        &current.invocation_id,
        &current.effect_id,
        new_id,
        &format!("{new_id}:text"),
        &format!("{new_id}:reasoning"),
        now,
    )?;
    fresh.predecessor_id = Some(current.id.clone());
    fresh.state = Running;
    fresh.provider = Some(provider.into());
    fresh.model = Some(model.into());
    fresh.started_at_ms = Some(now);
    validate_model_attempt(&previous)?;
    validate_model_attempt(&fresh)?;
    Ok(vec![
        ModelAttemptMutation {
            expected_revision: Some(current.revision),
            record: previous,
        },
        ModelAttemptMutation {
            expected_revision: None,
            record: fresh,
        },
    ])
}

pub fn record_model_attempt_usage(
    current: &ModelAttemptRecord,
    usage: serde_json::Value,
    now: i64,
) -> Result<ModelAttemptMutation, String> {
    if current.state != Running {
        return Err("only a running model attempt can report usage".into());
    }
    let mut record = next(current, now)?;
    record.usage = usage;
    validate_model_attempt(&record)?;
    Ok(ModelAttemptMutation { expected_revision: Some(current.revision), record })
}

pub fn finish_model_attempt(
    current: &ModelAttemptRecord,
    state: ModelAttemptState,
    failure: Option<ModelAttemptFailure>,
    usage: serde_json::Value,
    now: i64,
) -> Result<ModelAttemptMutation, String> {
    let mut record = next(current, now)?;
    if !matches!(state, Succeeded | Failed | Cancelled | Interrupted)
        || (matches!(state, Succeeded | Interrupted) && current.state != Running)
    {
        return Err("invalid model attempt completion transition".into());
    }
    record.state = state;
    record.failure = failure;
    // Failure/cancellation often has no additional accounting. It must not
    // erase a physical request's already-committed provider usage.
    if usage.as_object().is_none_or(|fields| !fields.is_empty()) {
        record.usage = usage;
    }
    record.completed_at_ms = state.is_terminal().then_some(now);
    validate_model_attempt(&record)?;
    Ok(ModelAttemptMutation {
        expected_revision: Some(current.revision),
        record,
    })
}

/// Records the observed failure before a retry or fallback starts. The
/// Attempt remains active until the succeeding admission atomically closes it
/// and links a fresh successor.
pub fn record_model_attempt_failure(
    current: &ModelAttemptRecord,
    failure: ModelAttemptFailure,
    now: i64,
) -> Result<ModelAttemptMutation, String> {
    if current.state != Running {
        return Err("only a running model attempt can record a sampling failure".into());
    }
    let mut record = next(current, now)?;
    record.failure = Some(failure);
    validate_model_attempt(&record)?;
    Ok(ModelAttemptMutation {
        expected_revision: Some(current.revision),
        record,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn prepared() -> ModelAttemptRecord {
        prepare_model_attempt(
            "store-run",
            "logical-model",
            "effect",
            "a1",
            "original-text",
            "original-reasoning",
            100,
        )
        .unwrap()
    }
    fn running() -> ModelAttemptRecord {
        start_model_attempt(&prepared(), "a1", "provider-a", "model-a", "initial", 110)
            .unwrap()
            .remove(0)
            .record
    }

    #[test]
    fn prepared_admission_preserves_owner_and_compares_revision() {
        let source = prepared();
        let mutations =
            start_model_attempt(&source, "a1", "provider-a", "model-a", "initial", 110).unwrap();
        assert_eq!(mutations.len(), 1);
        assert_eq!(mutations[0].expected_revision, Some(1));
        let running = &mutations[0].record;
        assert_eq!(running.revision, 2);
        assert_eq!(running.text_part_id, "original-text");
        assert_eq!(running.reasoning_part_id, "original-reasoning");
        assert_eq!(running.state, Running);
        assert_eq!(source.state, Prepared);
        assert!(start_model_attempt(&source, "other", "p", "m", "initial", 110).is_err());
    }

    #[test]
    fn retry_and_cold_resampling_have_new_identity_and_empty_usage() {
        for interrupted in [false, true] {
            let mut old = running();
            old.usage = serde_json::json!({"input_tokens":70});
            if interrupted {
                old = finish_model_attempt(
                    &old,
                    Interrupted,
                    Some(ModelAttemptFailure {
                        code: "connection_lost".into(),
                        message: "Connection lost".into(),
                        retryable: true,
                        http_status: None,
                    }),
                    old.usage.clone(),
                    120,
                )
                .unwrap()
                .record;
            }
            let old: ModelAttemptRecord =
                serde_json::from_slice(&serde_json::to_vec(&old).unwrap()).unwrap();
            let changes =
                start_model_attempt(&old, "a2", "provider-b", "model-b", "fallback", 130).unwrap();
            assert_eq!(changes.len(), 2);
            assert_eq!(changes[0].expected_revision, Some(old.revision));
            assert_eq!(changes[0].record.state, Superseded);
            assert_eq!(changes[0].record.usage, old.usage);
            assert_eq!(changes[0].record.completed_at_ms, Some(130));
            let next = &changes[1].record;
            assert_eq!(changes[1].expected_revision, None);
            assert_eq!(next.predecessor_id.as_deref(), Some("a1"));
            assert_eq!(next.run_id, old.run_id);
            assert_eq!(next.invocation_id, old.invocation_id);
            assert_eq!(next.effect_id, old.effect_id);
            assert_eq!(next.text_part_id, "a2:text");
            assert_eq!(next.reasoning_part_id, "a2:reasoning");
            assert_eq!(next.usage, serde_json::json!({}));
            assert_eq!(next.revision, 1);
            assert_eq!(next.started_at_ms, Some(130));
        }
    }

    #[test]
    fn reported_usage_survives_failed_cancelled_and_superseded_attempts() {
        let usage = serde_json::json!({"input_tokens":70014,"output_tokens":8000,
            "cache_usage":{"input_tokens":70014,"read_tokens":0,"write_tokens":null,
                "hit_rate_bps":0,"is_compaction":true}});
        let measured = record_model_attempt_usage(&running(), usage.clone(), 115).unwrap().record;
        assert_eq!(measured.state, Running);
        for state in [Failed, Cancelled] {
            assert_eq!(finish_model_attempt(&measured, state, None, serde_json::json!({}), 120)
                .unwrap().record.usage, usage);
        }
        let replacement = start_model_attempt(&measured, "a2", "p", "m", "summary rejected", 120).unwrap();
        assert_eq!(replacement[0].record.usage, usage);
        assert_eq!(replacement[0].record.state, Superseded);
        assert_eq!(replacement[1].record.usage, serde_json::json!({}));
        assert!(record_model_attempt_usage(&prepared(), usage.clone(), 115).is_err());
        assert!(record_model_attempt_usage(&measured, serde_json::Value::Null, 120).is_err());
        let done = finish_model_attempt(&measured, Failed, None, serde_json::json!({}), 120).unwrap().record;
        assert!(record_model_attempt_usage(&done, usage, 125).is_err());
    }

    #[test]
    fn terminal_attempts_cannot_be_reactivated_or_finished_twice() {
        for state in [Succeeded, Failed, Cancelled, Superseded] {
            let done = if state == Superseded {
                start_model_attempt(&running(), "a2", "p", "m", "retry", 120)
                    .unwrap()
                    .remove(0)
                    .record
            } else {
                finish_model_attempt(&running(), state, None, serde_json::json!({}), 120)
                    .unwrap()
                    .record
            };
            assert!(start_model_attempt(&done, "a3", "p", "m", "retry", 130).is_err());
            assert!(finish_model_attempt(&done, Failed, None, serde_json::json!({}), 130).is_err());
            assert_eq!(done.completed_at_ms, Some(120));
        }
        assert!(
            finish_model_attempt(&prepared(), Succeeded, None, serde_json::json!({}), 120).is_err()
        );
        assert!(
            finish_model_attempt(&running(), Superseded, None, serde_json::json!({}), 120).is_err()
        );
        assert_eq!(
            finish_model_attempt(&prepared(), Cancelled, None, serde_json::json!({}), 120)
                .unwrap()
                .record
                .started_at_ms,
            None
        );
    }

    #[test]
    fn invalid_ids_times_payload_and_revision_fail_without_mutation() {
        let original = running();
        for (id, now) in [("", 120), ("a1", 120), ("a2", 99)] {
            assert!(start_model_attempt(&original, id, "p", "m", "retry", now).is_err());
        }
        assert!(
            finish_model_attempt(&original, Failed, None, serde_json::Value::Null, 120).is_err()
        );
        let mut exhausted = original.clone();
        exhausted.revision = i64::MAX;
        assert!(
            finish_model_attempt(&exhausted, Failed, None, serde_json::json!({}), 120).is_err()
        );
        let mut invalid = original.clone();
        invalid.predecessor_id = Some(invalid.id.clone());
        assert!(validate_model_attempt(&invalid).is_err());
        invalid = original.clone();
        invalid.reasoning_part_id = invalid.text_part_id.clone();
        assert!(validate_model_attempt(&invalid).is_err());
        assert_eq!(original, running());
    }
}
