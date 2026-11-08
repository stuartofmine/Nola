// SPDX-License-Identifier: Apache-2.0
//! Writer-fenced cancellation admission, not inferred process liveness.
use crate::*;
use nola_domain::{EffectDeliveryAttempt, EffectDeliveryAttemptState as State, EffectDeliverySettlement};
use nola_domain::host_control::HostEffectControl;
use nola_domain::cancellation_not_started::{CancellationTargetNotStarted, CancellationRevokedAttempt};
use nola_store_port::run_effect::{PreparedCancellationNotStarted, CommitTransitionInput, OutboxEffectWrite};

pub(super) fn prepare(c: &Connection, run: &str, cancellation: &HostEffectControl,
    target: &HostEffectControl, revision: i64, at: i64) -> StoreResult<Option<PreparedCancellationNotStarted>> {
    if c.is_autocommit() { return Err(StoreError::Invariant("NotStarted admission requires Application write UoW".into())); }
    let header: Option<(String,String,String)> = c.query_row(
        "SELECT run_id,request_id,status FROM effect_outbox WHERE effect_id=?1", [&target.effect_id.0],
        |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional()?;
    let Some((owner, request_id, status)) = header else { return Ok(None); };
    if owner != run || request_id != target.request_id {
        return Err(StoreError::Conflict("NotStarted target publication has another owner".into()));
    }
    // A promoted/retired target may legitimately have an acknowledged or
    // pruned publication. That is not a new no-start proof and must not break
    // the independent owned-worker cancellation path.
    if status != "pending" { return Ok(None); }
    // Started or uncertain execution can never receive a NotStarted proof.
    // Decide this before comparing a regenerated reducer payload: the ordinary
    // owned cancellation path still needs to publish its stop intent and wait
    // for actual execution reconciliation, not fail an optional optimization.
    // Read every historical record, not the latest active attempt. A bound on
    // the result never authorizes dropping an older executed predecessor.
    let total: i64 = c.query_row("SELECT COUNT(*) FROM effect_delivery_attempts WHERE effect_id=?1",
        [&target.effect_id.0], |row| row.get(0))?;
    if total > 256 { return Ok(None); }
    let raw = c.prepare("SELECT record_json FROM effect_delivery_attempts WHERE effect_id=?1 ORDER BY attempt_number,id")?
        .query_map([&target.effect_id.0], |row| row.get::<_,String>(0))?.collect::<Result<Vec<_>,_>>()?;
    let mut attempts = Vec::new();
    for raw in raw {
        let record: EffectDeliveryAttempt = serde_json::from_str(&raw)?;
        nola_kernel::validate_effect_delivery_attempt(&record).map_err(StoreError::Invariant)?;
        if record.run_id != run || record.effect_id != target.effect_id.0 {
            return Err(StoreError::Conflict("NotStarted history belongs to another run or effect".into()));
        }
        // Cold recovery can reissue this effect with a new Host request ID.
        // Its predecessor remains relevant execution history, not foreign data.
        // A request mismatch supplies no exact no-start proof; retain the
        // ordinary cancellation/reconciliation path instead of rejecting Stop.
        if record.request_id != target.request_id { return Ok(None); }
        if record.execution_started_at_ms.is_some() || !matches!(record.state, State::Prepared | State::Leased)
            && !(record.state == State::Settled && record.settlement == Some(EffectDeliverySettlement::RejectedBeforeExecution)) {
            return Ok(None);
        }
        attempts.push(record);
    }
    let Some(source) = crate::outbox_payload::read(c, &target.effect_id.0)? else {
        return Err(StoreError::Conflict("NotStarted target has no original publication".into()));
    };
    let original: nola_domain::host_effect::HostRequestEnvelope<nola_domain::host_effect::HostRequest> =
        serde_json::from_str(&source.payload_json)?;
    let original_control = HostEffectControl::from_request(&original).map_err(StoreError::Invariant)?;
    let (conversation, current_revision): (String, i64) = c.query_row(
        "SELECT r.conversation_id,c.revision FROM runs r JOIN core_checkpoints c ON c.conversation_id=r.conversation_id
         WHERE r.id=?1 AND r.completed_at_ms IS NULL", [run], |row| Ok((row.get(0)?, row.get(1)?)))?;
    let checks = [
        (source.run_id == run, "source_run"),
        (source.status == "pending", "source_status"),
        (source.request_id == target.request_id, "source_request"),
        (original_control.payload.payload_identity == target.payload.payload_identity, "payload"),
        (original.host_id == target.host_id, "target_host"),
        (original.session_id == target.session_id, "target_session"),
        (original.turn_id == target.turn_id, "target_turn"),
        (original.catalog_revision == target.catalog_revision, "catalog_revision"),
        (original.availability_epoch == target.availability_epoch, "availability_epoch"),
        (cancellation.host_id == target.host_id, "cancellation_host"),
        (cancellation.session_id == target.session_id, "cancellation_session"),
        (cancellation.turn_id == target.turn_id, "cancellation_turn"),
        (target.turn_id.as_deref() == Some(run), "run_turn"),
        (current_revision.checked_add(1) == Some(revision), "checkpoint_revision"),
    ];
    let changed = checks.iter().filter_map(|(matches, field)| (!matches).then_some(*field)).collect::<Vec<_>>();
    if !changed.is_empty() {
        // Field names only: no payloads, tokens or user-provided identities.
        return Err(StoreError::Conflict(format!("NotStarted target/source identity changed: {}", changed.join(", "))));
    }
    let nola_domain::host_control::HostResponsePolicy::Cancel {target_effect_id,..} = &cancellation.payload.policy
        else { return Err(StoreError::Invariant("NotStarted requires a Cancel control".into())); };
    if target_effect_id != &target.effect_id { return Err(StoreError::Conflict("NotStarted control targets another effect".into())); }
    let mut mutations = Vec::new();
    let mut revoked_attempts = Vec::new();
    for attempt in attempts {
        if matches!(attempt.state, State::Prepared | State::Leased) {
            let mutation = nola_kernel::withdraw_unstarted_effect_for_cancellation(&attempt, at).map_err(StoreError::Invariant)?;
            revoked_attempts.push(CancellationRevokedAttempt {attempt_id:attempt.id,revision:mutation.record.revision});
            mutations.push(mutation);
        } else {
            revoked_attempts.push(CancellationRevokedAttempt {attempt_id:attempt.id,revision:attempt.revision});
        }
    }
    let proof = CancellationTargetNotStarted {conversation_id:conversation,run_id:run.into(),
        core_session_id:target.session_id.clone(),host_id:target.host_id.0.clone(),
        cancellation_effect_id:cancellation.effect_id.0.clone(),cancellation_request_id:cancellation.request_id.clone(),
        target_effect_id:target.effect_id.0.clone(),target_request_id:target.request_id.clone(),
        target_payload_identity:target.payload.payload_identity.clone(),checkpoint_revision:revision,revoked_attempts};
    proof.validate().map_err(StoreError::Invariant)?;
    Ok(Some(PreparedCancellationNotStarted {proof,mutations}))
}

/// Validate before writing the checkpoint or applying revocations. Claim/start
/// and this guard serialize under the same Store writer transaction.
pub(super) fn validate(c: &Connection, input: &CommitTransitionInput) -> StoreResult<()> {
    let mut admitted = Vec::new();
    for effect in &input.outbox_effects {
        let OutboxEffectWrite::Declare {payload_json} = &effect.write else {continue};
        let cancellation: nola_domain::host_effect::HostRequestEnvelope<nola_domain::host_effect::HostRequest> = serde_json::from_str(payload_json)?;
        let nola_domain::host_effect::HostRequest::Cancel {target_effect_id,target_not_started:Some(proof)} = &cancellation.payload else {continue};
        if !input.acknowledged_effect_ids.contains(&target_effect_id.0) || proof.checkpoint_revision != input.checkpoint_revision {
            return Err(StoreError::Conflict("NotStarted admission did not retire its target atomically".into()));
        }
        let original = crate::outbox_payload::read(c, &target_effect_id.0)?
            .ok_or_else(|| StoreError::Conflict("NotStarted original target missing".into()))?;
        let target: nola_domain::host_effect::HostRequestEnvelope<nola_domain::host_effect::HostRequest> = serde_json::from_str(&original.payload_json)?;
        let expected = prepare(c,&input.run_id,&HostEffectControl::from_request(&cancellation).map_err(StoreError::Invariant)?,
            &HostEffectControl::from_request(&target).map_err(StoreError::Invariant)?,input.checkpoint_revision,input.committed_at_ms)?
            .ok_or_else(|| StoreError::Conflict("NotStarted history does not prove no execution".into()))?;
        if expected.proof != *proof || expected.mutations.iter().any(|mutation| !input.effect_delivery_attempt_mutations.contains(mutation))
            || input.effect_delivery_attempt_mutations.iter().any(|mutation|
                mutation.record.effect_id == target_effect_id.0 && !expected.mutations.contains(mutation)) {
            return Err(StoreError::Conflict("NotStarted proof or revocation mutations changed".into()));
        }
        admitted.extend(expected.mutations);
    }
    validate_revocation_membership(&input.effect_delivery_attempt_mutations, &admitted)
}

fn validate_revocation_membership(
    submitted: &[nola_domain::EffectDeliveryAttemptMutation],
    admitted: &[nola_domain::EffectDeliveryAttemptMutation],
) -> StoreResult<()> {
    if submitted.iter().any(|mutation|
        mutation.record.interruption_reason.as_deref() == Some("cancellation_not_started_admission")
            && !admitted.contains(mutation)) {
        return Err(StoreError::Conflict("cancellation revocation has no exact committed NotStarted proof".into()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn revocation(id: &str) -> nola_domain::EffectDeliveryAttemptMutation {
        let prepared = nola_kernel::prepare_effect_delivery_attempt(
            id, "run", &format!("effect-{id}"), &format!("request-{id}"), 1,
            nola_domain::EffectDeliveryReplaySafety::Recoverable, None, 1000,
        ).unwrap();
        nola_kernel::withdraw_unstarted_effect_for_cancellation(&prepared.record, 1100).unwrap()
    }

    #[test]
    fn started_history_skips_optional_proof_without_revocation_or_source_rewrite() {
        use nola_domain::host_effect::{HostId, HostEffectId, HostRequest, HostRequestEnvelope};
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch("CREATE TABLE effect_outbox(effect_id TEXT,run_id TEXT,request_id TEXT,status TEXT);
            CREATE TABLE effect_delivery_attempts(id TEXT,effect_id TEXT,attempt_number INTEGER,record_json TEXT);
            INSERT INTO effect_outbox VALUES('effect','run','request','pending'); BEGIN IMMEDIATE;").unwrap();
        let envelope = |id: &str, request: &str, target: &str| HostRequestEnvelope {
            attempt_started:false,attempt_id:None,delegated_invocation_id:None,
            effect_id:HostEffectId(id.into()),request_id:request.into(),host_id:HostId("host".into()),
            lease_id:None,session_id:"session".into(),turn_id:Some("run".into()),
            catalog_revision:1,availability_epoch:1,created_at_ms:1000,deadline_ms:None,
            batch:None,resume_after_sequence:None,delivery:None,
            payload:HostRequest::<serde_json::Value>::Cancel {target_effect_id:HostEffectId(target.into()),target_not_started:None},
        };
        let target = HostEffectControl::from_request(&envelope("effect","request","earlier")).unwrap();
        let cancel = HostEffectControl::from_request(&envelope("cancel","cancel-request","effect")).unwrap();
        let prepared = nola_kernel::prepare_effect_delivery_attempt("attempt","run","effect","request",1,
            nola_domain::EffectDeliveryReplaySafety::Recoverable,None,1000).unwrap();
        let leased = nola_kernel::claim_effect_delivery_attempt(&prepared.record,"worker","owner","token",3000,1100).unwrap();
        let started = nola_kernel::start_effect_delivery_execution(&leased.record,"worker","owner","token",1200).unwrap();
        let raw = serde_json::to_string(&started.record).unwrap();
        c.execute("INSERT INTO effect_delivery_attempts VALUES('attempt','effect',1,?1)",[&raw]).unwrap();
        assert!(prepare(&c,"run",&cancel,&target,2,1300).unwrap().is_none());
        assert_eq!(c.query_row("SELECT record_json FROM effect_delivery_attempts",[],|r|r.get::<_,String>(0)).unwrap(),raw);
        assert!(prepare(&c,"foreign-run",&cancel,&target,2,1300).is_err());
        // An unstarted delivery still needs the original exact source: no
        // missing-source or changed-payload fallback may manufacture a proof.
        c.execute("UPDATE effect_delivery_attempts SET record_json=?1",[serde_json::to_string(&prepared.record).unwrap()]).unwrap();
        assert!(prepare(&c,"run",&cancel,&target,2,1300).is_err());
    }

    #[test]
    fn cancellation_revocation_requires_a_validated_proof_membership() {
        let submitted = revocation("unproved");
        assert!(matches!(validate_revocation_membership(&[submitted], &[]),
            Err(StoreError::Conflict(message)) if message.contains("no exact committed NotStarted proof")));
        assert!(validate_revocation_membership(&[], &[]).is_ok());
    }

    #[test]
    fn a_proved_revocation_cannot_authorize_an_extra_or_modified_mutation() {
        let admitted = revocation("admitted");
        assert!(validate_revocation_membership(std::slice::from_ref(&admitted),
            std::slice::from_ref(&admitted)).is_ok());
        let extra = revocation("foreign");
        assert!(validate_revocation_membership(&[admitted.clone(), extra],
            std::slice::from_ref(&admitted)).is_err());
        // Matching IDs alone are insufficient: the complete CAS mutation,
        // including the expected predecessor revision, belongs to the proof.
        let mut modified = admitted.clone();
        modified.expected_revision = Some(99);
        assert!(validate_revocation_membership(&[modified], &[admitted]).is_err());
    }
}
