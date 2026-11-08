// SPDX-License-Identifier: Apache-2.0
//! Pure permission delivery transitions. A claim must commit before writing to a worker.
use nola_domain::{
    DelegatedPermissionDecision, DelegatedPermissionDeliveryMutation,
    DelegatedPermissionDeliveryRecord, DelegatedPermissionDeliveryState,
};
use DelegatedPermissionDeliveryState::*;

fn identity(value: &str) -> Result<(), String> {
    if value.trim().is_empty() || value.len() > 512 || value.chars().any(char::is_control) {
        return Err("invalid delegated permission identity".into());
    }
    Ok(())
}

pub fn validate_delegated_permission(
    record: &DelegatedPermissionDeliveryRecord,
) -> Result<(), String> {
    for value in [
        &record.id,
        &record.run_id,
        &record.invocation_id,
        &record.target_effect_id,
        &record.worker_generation,
        &record.occurrence_id,
        &record.backend_id,
        &record.backend_session_id,
        &record.rpc_request_id,
        &record.call_id,
        &record.request_digest,
    ] {
        identity(value)?;
    }
    for value in [&record.control_effect_id, &record.control_request_id]
        .into_iter()
        .flatten()
    {
        identity(value)?;
    }
    if record.request_json.is_null() {
        return Err("delegated permission request must preserve the original payload".into());
    }
    if record.revision < 1
        || record.created_at_ms < 0
        || record
            .claimed_at_ms
            .is_some_and(|time| time < record.created_at_ms)
        || record
            .written_at_ms
            .is_some_and(|time| time < record.claimed_at_ms.unwrap_or(record.created_at_ms))
    {
        return Err("invalid delegated permission revision or timestamps".into());
    }
    let authorized = record.decision.is_some();
    if record.control_effect_id.is_some() != authorized
        || record.control_request_id.is_some() != authorized
        || (record.state == AwaitingDecision && authorized)
        || (matches!(
            record.state,
            Prepared | Claimed | TransportWritten | OutcomeUnknown
        ) && !authorized)
    {
        return Err("delegated permission authorization binding mismatch".into());
    }
    if record.claimed_at_ms.is_some()
        != matches!(record.state, Claimed | TransportWritten | OutcomeUnknown)
        || record.written_at_ms.is_some() != (record.state == TransportWritten)
    {
        return Err("delegated permission transport evidence mismatch".into());
    }
    Ok(())
}

pub fn prepare_delegated_permission(
    record: DelegatedPermissionDeliveryRecord,
) -> Result<DelegatedPermissionDeliveryMutation, String> {
    validate_delegated_permission(&record)?;
    if record.state != AwaitingDecision || record.revision != 1 {
        return Err("new delegated permission must await a decision at revision one".into());
    }
    Ok(DelegatedPermissionDeliveryMutation {
        expected_revision: None,
        record,
    })
}

fn next(
    current: &DelegatedPermissionDeliveryRecord,
    now_ms: i64,
) -> Result<DelegatedPermissionDeliveryMutation, String> {
    validate_delegated_permission(current)?;
    if now_ms
        < current
            .written_at_ms
            .or(current.claimed_at_ms)
            .unwrap_or(current.created_at_ms)
    {
        return Err("delegated permission time moved backwards".into());
    }
    let mut record = current.clone();
    record.revision = current
        .revision
        .checked_add(1)
        .ok_or("delegated permission revision exhausted")?;
    Ok(DelegatedPermissionDeliveryMutation {
        expected_revision: Some(current.revision),
        record,
    })
}

pub fn authorize_delegated_permission(
    current: &DelegatedPermissionDeliveryRecord,
    control_effect_id: String,
    control_request_id: String,
    decision: DelegatedPermissionDecision,
    now_ms: i64,
) -> Result<DelegatedPermissionDeliveryMutation, String> {
    let mut mutation = next(current, now_ms)?;
    if current.state != AwaitingDecision {
        return Err("delegated permission decision is already fixed".into());
    }
    mutation.record.control_effect_id = Some(control_effect_id);
    mutation.record.control_request_id = Some(control_request_id);
    mutation.record.decision = Some(decision);
    mutation.record.state = Prepared;
    validate_delegated_permission(&mutation.record)?;
    Ok(mutation)
}

pub fn claim_delegated_permission(
    current: &DelegatedPermissionDeliveryRecord,
    now_ms: i64,
) -> Result<DelegatedPermissionDeliveryMutation, String> {
    let mut mutation = next(current, now_ms)?;
    if current.state != Prepared {
        return Err("only prepared delegated permission may claim transport".into());
    }
    mutation.record.state = Claimed;
    mutation.record.claimed_at_ms = Some(now_ms);
    validate_delegated_permission(&mutation.record)?;
    Ok(mutation)
}

/// This records a completed transport write, not acknowledgement of backend execution.
pub fn write_delegated_permission(
    current: &DelegatedPermissionDeliveryRecord,
    now_ms: i64,
) -> Result<DelegatedPermissionDeliveryMutation, String> {
    let mut mutation = next(current, now_ms)?;
    if current.state != Claimed {
        return Err("delegated permission transport requires a committed claim".into());
    }
    mutation.record.state = TransportWritten;
    mutation.record.written_at_ms = Some(now_ms);
    validate_delegated_permission(&mutation.record)?;
    Ok(mutation)
}

/// Loss after claim cannot prove that the external write did not occur.
pub fn interrupt_delegated_permission(
    current: &DelegatedPermissionDeliveryRecord,
    now_ms: i64,
) -> Result<DelegatedPermissionDeliveryMutation, String> {
    let mut mutation = next(current, now_ms)?;
    mutation.record.state = match current.state {
        AwaitingDecision | Prepared => Interrupted,
        Claimed => OutcomeUnknown,
        TransportWritten | OutcomeUnknown | Interrupted => {
            return Err(
                "delegated permission transport evidence cannot be retried or downgraded".into(),
            );
        }
    };
    validate_delegated_permission(&mutation.record)?;
    Ok(mutation)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn occurrence() -> DelegatedPermissionDeliveryRecord {
        DelegatedPermissionDeliveryRecord {
            id: "delivery".into(),
            run_id: "run".into(),
            invocation_id: "invocation".into(),
            target_effect_id: "target".into(),
            worker_generation: "generation".into(),
            occurrence_id: "occurrence".into(),
            backend_id: "backend".into(),
            backend_session_id: "session".into(),
            rpc_request_id: "rpc-1".into(),
            call_id: "call-1".into(),
            request_json: serde_json::json!({"input": "保留原始请求"}),
            request_digest: "digest".into(),
            control_effect_id: None,
            control_request_id: None,
            decision: None,
            state: AwaitingDecision,
            created_at_ms: 100,
            claimed_at_ms: None,
            written_at_ms: None,
            revision: 1,
        }
    }
    fn authorize(current: &DelegatedPermissionDeliveryRecord) -> DelegatedPermissionDeliveryRecord {
        authorize_delegated_permission(
            current,
            "control".into(),
            "request".into(),
            DelegatedPermissionDecision::AllowOnce,
            110,
        )
        .unwrap()
        .record
    }

    #[test]
    fn write_requires_authorization_and_claim_and_preserves_the_original_occurrence() {
        let source = occurrence();
        let inserted = prepare_delegated_permission(source.clone()).unwrap();
        assert_eq!(inserted.expected_revision, None);
        assert!(claim_delegated_permission(&source, 110).is_err());
        assert!(write_delegated_permission(&source, 110).is_err());
        let authorized = authorize(&source);
        assert!(write_delegated_permission(&authorized, 120).is_err());
        let claimed = claim_delegated_permission(&authorized, 120).unwrap();
        assert_eq!(claimed.expected_revision, Some(2));
        assert_eq!(claimed.record.revision, 3);
        assert_eq!(claimed.record.claimed_at_ms, Some(120));
        let written = write_delegated_permission(&claimed.record, 130).unwrap();
        assert_eq!(written.expected_revision, Some(3));
        assert_eq!(written.record.written_at_ms, Some(130));
        let mut expected = source.clone();
        expected.control_effect_id = Some("control".into());
        expected.control_request_id = Some("request".into());
        expected.decision = Some(DelegatedPermissionDecision::AllowOnce);
        expected.state = TransportWritten;
        expected.claimed_at_ms = Some(120);
        expected.written_at_ms = Some(130);
        expected.revision = 4;
        assert_eq!(written.record, expected);
        assert_eq!(source, occurrence());
    }

    #[test]
    fn recovery_never_converts_a_claim_to_safe_retry_or_denial() {
        let waiting = occurrence();
        let prepared = authorize(&waiting);
        for source in [&waiting, &prepared] {
            let recovered = interrupt_delegated_permission(source, 120).unwrap();
            assert_eq!(recovered.record.state, Interrupted);
            assert_eq!(recovered.record.decision, source.decision);
            assert_eq!(recovered.record.claimed_at_ms, None);
        }
        let claimed = claim_delegated_permission(&prepared, 120).unwrap().record;
        let unknown = interrupt_delegated_permission(&claimed, 130)
            .unwrap()
            .record;
        assert_eq!(unknown.state, OutcomeUnknown);
        assert_eq!(unknown.claimed_at_ms, Some(120));
        assert_eq!(unknown.written_at_ms, None);
        assert_eq!(unknown.decision, prepared.decision);
        let written = write_delegated_permission(&claimed, 130).unwrap().record;
        let interrupted = interrupt_delegated_permission(&prepared, 130)
            .unwrap()
            .record;
        for source in [&unknown, &written, &interrupted] {
            assert!(claim_delegated_permission(source, 140).is_err());
            assert!(write_delegated_permission(source, 140).is_err());
            assert!(interrupt_delegated_permission(source, 140).is_err());
        }
        assert!(claim_delegated_permission(&claimed, 130).is_err());
    }

    #[test]
    fn authorization_is_immutable_for_every_decision_and_state_after_preparation() {
        for decision in [
            DelegatedPermissionDecision::AllowOnce,
            DelegatedPermissionDecision::AllowConversation,
            DelegatedPermissionDecision::Deny,
        ] {
            let prepared = authorize_delegated_permission(
                &occurrence(),
                "control".into(),
                "request".into(),
                decision,
                110,
            )
            .unwrap()
            .record;
            let claimed = claim_delegated_permission(&prepared, 120).unwrap().record;
            let written = write_delegated_permission(&claimed, 130).unwrap().record;
            let unknown = interrupt_delegated_permission(&claimed, 130)
                .unwrap()
                .record;
            let interrupted = interrupt_delegated_permission(&prepared, 130)
                .unwrap()
                .record;
            for source in [&prepared, &claimed, &written, &unknown, &interrupted] {
                assert!(authorize_delegated_permission(
                    source,
                    "control".into(),
                    "request".into(),
                    decision,
                    140
                )
                .is_err());
                assert!(authorize_delegated_permission(
                    source,
                    "replacement".into(),
                    "new-request".into(),
                    DelegatedPermissionDecision::Deny,
                    140
                )
                .is_err());
            }
        }
    }

    #[test]
    fn malformed_evidence_and_clock_or_revision_regressions_are_rejected() {
        let source = occurrence();
        assert!(authorize_delegated_permission(
            &source,
            "control".into(),
            "request".into(),
            DelegatedPermissionDecision::Deny,
            99
        )
        .is_err());
        let mut invalid = source.clone();
        invalid.worker_generation.clear();
        assert!(validate_delegated_permission(&invalid).is_err());
        invalid = source.clone();
        invalid.control_request_id = Some("unbound".into());
        assert!(validate_delegated_permission(&invalid).is_err());
        invalid = source.clone();
        invalid.request_json = serde_json::Value::Null;
        assert!(validate_delegated_permission(&invalid).is_err());
        invalid = source.clone();
        invalid.revision = 2;
        assert!(prepare_delegated_permission(invalid).is_err());
        let prepared = authorize(&source);
        assert!(prepare_delegated_permission(prepared.clone()).is_err());
        invalid = prepared.clone();
        invalid.state = TransportWritten;
        invalid.written_at_ms = Some(130);
        assert!(validate_delegated_permission(&invalid).is_err());
        invalid = prepared.clone();
        invalid.revision = i64::MAX;
        assert!(claim_delegated_permission(&invalid, 120).is_err());
        let claimed = claim_delegated_permission(&prepared, 120).unwrap().record;
        assert!(write_delegated_permission(&claimed, 119).is_err());
        assert!(interrupt_delegated_permission(&claimed, 119).is_err());
        invalid = claimed;
        invalid.state = Interrupted;
        assert!(validate_delegated_permission(&invalid).is_err());
    }

    #[test]
    fn serialized_evidence_is_strict_and_preserves_raw_request() {
        let prepared = authorize(&occurrence());
        let claimed = claim_delegated_permission(&prepared, 120).unwrap();
        let mut encoded = serde_json::to_value(&claimed).unwrap();
        assert_eq!(encoded["record"]["state"], "claimed");
        assert_eq!(encoded["record"]["decision"], "allow_once");
        assert_eq!(
            serde_json::from_value::<DelegatedPermissionDeliveryMutation>(encoded.clone()).unwrap(),
            claimed
        );
        encoded["record"]["assumed_delivered"] = serde_json::json!(true);
        assert!(serde_json::from_value::<DelegatedPermissionDeliveryMutation>(encoded).is_err());
    }
}
