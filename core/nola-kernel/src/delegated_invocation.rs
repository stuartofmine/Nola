// SPDX-License-Identifier: Apache-2.0
//! Pure external-agent admission. Callers atomically commit returned mutations before execution.
use nola_domain::{
    DelegatedInvocationMutation, DelegatedInvocationRecord, DelegatedInvocationState,
};
use DelegatedInvocationState::*;

fn identity(value: &str) -> Result<(), String> {
    if value.trim().is_empty() || value.len() > 512 || value.chars().any(char::is_control) {
        return Err("invalid delegated invocation identity".into());
    }
    Ok(())
}

pub fn validate_delegated_invocation(record: &DelegatedInvocationRecord) -> Result<(), String> {
    for id in [
        &record.id,
        &record.run_id,
        &record.effect_id,
        &record.backend_id,
        &record.text_part_id,
        &record.reasoning_part_id,
    ] {
        identity(id)?;
    }
    if let Some(session) = &record.backend_session_id {
        identity(session)?;
    }
    if record.text_part_id == record.reasoning_part_id {
        return Err("delegated invocation Parts must be distinct".into());
    }
    if record.revision <= 0
        || record.created_at_ms < 0
        || record
            .started_at_ms
            .is_some_and(|time| time < record.created_at_ms)
        || record
            .completed_at_ms
            .is_some_and(|time| time < record.started_at_ms.unwrap_or(record.created_at_ms))
    {
        return Err("invalid delegated invocation revision or timestamps".into());
    }
    if record.state.is_terminal() != record.completed_at_ms.is_some() {
        return Err("delegated invocation terminal timestamp mismatch".into());
    }
    if (record.state == Prepared && record.started_at_ms.is_some())
        || (matches!(
            record.state,
            Running | WaitingPermission | Interrupted | Cancelling | Succeeded
        ) && record.started_at_ms.is_none())
    {
        return Err("delegated invocation admission timestamp mismatch".into());
    }
    if !record.usage.is_object() {
        return Err("delegated invocation usage must be an object".into());
    }
    if record
        .failure
        .as_ref()
        .is_some_and(|value| value.trim().is_empty() || value.len() > 4096)
        || (record.failure.is_some()
            && !matches!(record.state, Failed | Cancelled | Interrupted | Cancelling))
    {
        return Err("invalid delegated invocation failure".into());
    }
    Ok(())
}

pub fn prepare_delegated_invocation(
    run_id: &str,
    effect_id: &str,
    id: &str,
    backend_id: &str,
    backend_session_id: Option<&str>,
    text_part_id: &str,
    reasoning_part_id: &str,
    now: i64,
) -> Result<DelegatedInvocationRecord, String> {
    let record = DelegatedInvocationRecord {
        id: id.into(),
        run_id: run_id.into(),
        effect_id: effect_id.into(),
        backend_id: backend_id.into(),
        backend_session_id: backend_session_id.map(str::to_owned),
        text_part_id: text_part_id.into(),
        reasoning_part_id: reasoning_part_id.into(),
        state: Prepared,
        failure: None,
        usage: serde_json::json!({}),
        created_at_ms: now,
        started_at_ms: None,
        completed_at_ms: None,
        revision: 1,
    };
    validate_delegated_invocation(&record)?;
    Ok(record)
}

/// A missing session preserves the existing binding. A supplied session binds once.
/// Interrupted invocations require a separate recovery-evidence protocol before they can run again.
pub fn transition_delegated_invocation(
    current: &DelegatedInvocationRecord,
    state: DelegatedInvocationState,
    backend_session_id: Option<&str>,
    failure: Option<String>,
    usage: Option<serde_json::Value>,
    now: i64,
) -> Result<DelegatedInvocationMutation, String> {
    validate_delegated_invocation(current)?;
    if current.state.is_terminal() {
        return Err("terminal delegated invocation cannot change".into());
    }
    let allowed = match current.state {
        Prepared => matches!(state, Running | Failed | Cancelled),
        Running | WaitingPermission => matches!(
            state,
            Running | WaitingPermission | Interrupted | Cancelling | Succeeded | Failed | Cancelled
        ),
        Interrupted => matches!(state, Interrupted | Cancelling | Failed | Cancelled),
        Cancelling => matches!(state, Cancelling | Cancelled | Failed),
        Succeeded | Failed | Cancelled => false,
    };
    if !allowed {
        return Err("invalid delegated invocation transition; interrupted execution requires recovery evidence".into());
    }
    if now < current.started_at_ms.unwrap_or(current.created_at_ms) {
        return Err("delegated invocation time moved backwards".into());
    }
    let mut record = current.clone();
    record.revision = current
        .revision
        .checked_add(1)
        .ok_or("delegated invocation revision exhausted")?;
    if let Some(session) = backend_session_id {
        identity(session)?;
        if current
            .backend_session_id
            .as_deref()
            .is_some_and(|existing| existing != session)
        {
            return Err("delegated invocation backend session cannot change".into());
        }
        record.backend_session_id = Some(session.into());
    }
    record.state = state;
    if current.state == Prepared && state == Running {
        record.started_at_ms = Some(now);
    }
    record.completed_at_ms = state.is_terminal().then_some(now);
    record.failure = failure;
    if let Some(usage) = usage {
        record.usage = usage;
    }
    validate_delegated_invocation(&record)?;
    Ok(DelegatedInvocationMutation {
        expected_revision: Some(current.revision),
        record,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn prepared() -> DelegatedInvocationRecord {
        prepare_delegated_invocation(
            "run",
            "effect",
            "invocation",
            "codex",
            None,
            "text",
            "reasoning",
            100,
        )
        .unwrap()
    }
    fn step(
        current: &DelegatedInvocationRecord,
        state: DelegatedInvocationState,
    ) -> Result<DelegatedInvocationRecord, String> {
        transition_delegated_invocation(current, state, None, None, None, 120)
            .map(|mutation| mutation.record)
    }
    #[test]
    fn acceptance_is_required_before_output_and_permission() {
        let source = prepared();
        for state in [
            Prepared,
            WaitingPermission,
            Interrupted,
            Cancelling,
            Succeeded,
        ] {
            assert!(step(&source, state).is_err());
        }
        let accepted =
            transition_delegated_invocation(&source, Running, None, None, None, 110).unwrap();
        assert_eq!(accepted.expected_revision, Some(1));
        assert_eq!(accepted.record.revision, 2);
        assert_eq!(accepted.record.started_at_ms, Some(110));
        assert_eq!(accepted.record.id, source.id);
        assert_eq!(accepted.record.text_part_id, source.text_part_id);
        assert_eq!(source.state, Prepared);
        let waiting = step(&accepted.record, WaitingPermission).unwrap();
        let running = step(&waiting, Running).unwrap();
        assert_eq!(running.started_at_ms, Some(110));
    }
    #[test]
    fn session_binding_cannot_be_replaced_and_usage_survives_missing_update() {
        let running = step(&prepared(), Running).unwrap();
        let bound = transition_delegated_invocation(
            &running,
            Running,
            Some("session-a"),
            None,
            Some(serde_json::json!({"input_tokens": 7})),
            130,
        )
        .unwrap()
        .record;
        assert!(transition_delegated_invocation(
            &bound,
            Running,
            Some("session-b"),
            None,
            None,
            140
        )
        .is_err());
        let finished = transition_delegated_invocation(&bound, Succeeded, None, None, None, 140)
            .unwrap()
            .record;
        assert_eq!(finished.backend_session_id.as_deref(), Some("session-a"));
        assert_eq!(finished.usage, serde_json::json!({"input_tokens": 7}));
        assert_eq!(finished.completed_at_ms, Some(140));
    }
    #[test]
    fn interrupted_worker_cannot_resume_or_claim_success_without_evidence() {
        let interrupted = step(&step(&prepared(), Running).unwrap(), Interrupted).unwrap();
        for state in [Prepared, Running, WaitingPermission, Succeeded] {
            assert!(step(&interrupted, state).is_err());
        }
        assert!(step(&interrupted, Interrupted).is_ok());
        assert!(step(&interrupted, Failed).is_ok());
        assert!(step(&interrupted, Cancelled).is_ok());
    }
    #[test]
    fn cancellation_intent_remains_pending_until_worker_confirms_stop() {
        let running = transition_delegated_invocation(
            &prepared(),
            Running,
            Some("worker-session"),
            None,
            None,
            110,
        )
        .unwrap()
        .record;
        for source in [
            running.clone(),
            step(&running, WaitingPermission).unwrap(),
            step(&running, Interrupted).unwrap(),
        ] {
            let intent =
                transition_delegated_invocation(&source, Cancelling, None, None, None, 130)
                    .unwrap();
            assert_eq!(intent.expected_revision, Some(source.revision));
            assert!(!intent.record.state.is_terminal());
            assert_eq!(intent.record.completed_at_ms, None);
            assert_eq!(intent.record.started_at_ms, Some(110));
            assert_eq!(intent.record.backend_session_id, source.backend_session_id);
            assert_eq!(intent.record.text_part_id, source.text_part_id);
            assert_eq!(intent.record.reasoning_part_id, source.reasoning_part_id);
            for forbidden in [Prepared, Running, WaitingPermission, Interrupted, Succeeded] {
                assert!(transition_delegated_invocation(
                    &intent.record,
                    forbidden,
                    None,
                    None,
                    None,
                    140
                )
                .is_err());
            }
            let confirmed =
                transition_delegated_invocation(&intent.record, Cancelled, None, None, None, 140)
                    .unwrap()
                    .record;
            assert!(confirmed.state.is_terminal());
            assert_eq!(confirmed.completed_at_ms, Some(140));
        }
        let mut unadmitted = prepared();
        unadmitted.state = Cancelling;
        assert!(validate_delegated_invocation(&unadmitted).is_err());
        assert!(step(&prepared(), Cancelling).is_err());
        assert_eq!(step(&prepared(), Cancelled).unwrap().started_at_ms, None);
    }

    #[test]
    fn terminal_records_are_immutable_including_repeated_completion() {
        for state in [Succeeded, Failed, Cancelled] {
            let terminal = step(&step(&prepared(), Running).unwrap(), state).unwrap();
            for next in [
                Prepared,
                Running,
                WaitingPermission,
                Interrupted,
                Cancelling,
                Succeeded,
                Failed,
                Cancelled,
            ] {
                assert!(step(&terminal, next).is_err());
            }
        }
        for state in [Failed, Cancelled] {
            let unstarted = step(&prepared(), state).unwrap();
            assert_eq!(unstarted.started_at_ms, None);
            assert!(unstarted.completed_at_ms.is_some());
        }
    }
    #[test]
    fn invalid_records_clocks_and_revision_overflow_are_rejected() {
        let source = prepared();
        assert!(transition_delegated_invocation(&source, Running, None, None, None, 99).is_err());
        let mut overflow = source.clone();
        overflow.revision = i64::MAX;
        assert!(step(&overflow, Running).is_err());
        let mut invalid = source.clone();
        invalid.reasoning_part_id = invalid.text_part_id.clone();
        assert!(validate_delegated_invocation(&invalid).is_err());
        invalid = source.clone();
        invalid.usage = serde_json::Value::Null;
        assert!(validate_delegated_invocation(&invalid).is_err());
        invalid = source.clone();
        invalid.state = Running;
        assert!(validate_delegated_invocation(&invalid).is_err());
        invalid = source.clone();
        invalid.backend_session_id = Some("\n".into());
        assert!(validate_delegated_invocation(&invalid).is_err());
        assert!(transition_delegated_invocation(
            &source,
            Running,
            None,
            Some("failure on healthy execution".into()),
            None,
            120
        )
        .is_err());
    }
    #[test]
    fn resume_context_preserves_prebound_session_and_serialized_identity() {
        let record = prepare_delegated_invocation(
            "run",
            "effect",
            "invocation",
            "codex",
            Some("existing-session"),
            "text",
            "reasoning",
            100,
        )
        .unwrap();
        let running = step(&record, Running).unwrap();
        assert_eq!(running.backend_session_id, record.backend_session_id);
        let encoded = serde_json::to_value(&running).unwrap();
        assert_eq!(encoded["state"], "running");
        assert_eq!(
            serde_json::from_value::<DelegatedInvocationRecord>(encoded).unwrap(),
            running
        );
    }
}
