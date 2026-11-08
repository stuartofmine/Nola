// SPDX-License-Identifier: Apache-2.0
//! Executes production Host recovery/commit helpers without any SQLite repository.
#[path = "support/model_identity.rs"]
mod fixture_model_identity;
use nola_application::{
    claim_committed_host_requests, load_committed_host_requests, reduce_and_accept_host_response,
    ClaimCommittedHostRequests, ReduceAndAcceptHostResponse,
};
use nola_agent_engine::agent::host::*;
use nola_agent_engine::agent::recovery::CheckpointReason;
use nola_agent_engine::agent::{
    AgentCore, AgentCoreConfig, AgentInputDelivery, AgentItem, AgentSubmission, ItemId, SessionId,
};
use nola_store_port::run_effect::*;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

#[path = "support/run_effect_publication.rs"]
mod publication;

fn effect_binding(
    input: &OutboxEffectInput,
) -> Result<nola_store_port::host_replay::CoreHostBindingOverlay, String> {
    match &input.write {
        OutboxEffectWrite::Declare { payload_json } => {
            let request: HostRequestEnvelope =
                serde_json::from_str(payload_json).map_err(|e| e.to_string())?;
            Ok(nola_store_port::host_replay::CoreHostBindingOverlay::from_request(&request))
        }
        OutboxEffectWrite::Rebind { binding, .. } => Ok(binding.clone()),
    }
}

fn effect_payload(
    input: &OutboxEffectInput,
    run_id: &str,
    previous: Option<&StoredOutboxEffect>,
) -> Result<String, String> {
    match &input.write {
        OutboxEffectWrite::Declare { payload_json } => {
            if previous.is_some() {
                return Err("duplicate effect declaration".into());
            }
            let request: HostRequestEnvelope =
                serde_json::from_str(payload_json).map_err(|e| e.to_string())?;
            if request.effect_id.0.as_str() != input.effect_id
                || request.request_id != input.request_id
            {
                return Err("effect declaration identity mismatch".into());
            }
            Ok(payload_json.clone())
        }
        OutboxEffectWrite::Rebind {
            expected_request_id,
            expected_attempt_id,
            binding,
        } => {
            let previous = previous.ok_or("missing effect rebind owner")?;
            let mut request: HostRequestEnvelope =
                serde_json::from_str(&previous.payload_json).map_err(|e| e.to_string())?;
            if previous.run_id != run_id
                || previous.status != "pending"
                || previous.effect_kind != input.effect_kind
                || previous.request_id != *expected_request_id
                || request.attempt_id != *expected_attempt_id
                || binding.request_id != input.request_id
            {
                return Err("effect rebind CAS mismatch".into());
            }
            binding.apply_to(&mut request)?;
            serde_json::to_string(&request).map_err(|e| e.to_string())
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct MemoryRun {
    journal: publication::MemoryJournal,
    transaction_backup: Option<Box<MemoryRun>>,
    core_turn_ids: std::collections::BTreeSet<String>,
    checkpoint: StoredCheckpoint,
    run: StoredActiveRun,
    config: StoredRunInvocationConfig,
    effects: Vec<StoredOutboxEffect>,
    commits: Vec<CommitTransitionInput>,
    fail_commit: bool,
    attempts: BTreeMap<String, nola_domain::ModelAttemptRecord>,
    delegated_invocations: BTreeMap<String, nola_domain::DelegatedInvocationRecord>,
    delegated_permissions: BTreeMap<String, nola_domain::DelegatedPermissionDeliveryRecord>,
    effect_deliveries: BTreeMap<String, nola_domain::EffectDeliveryAttempt>,
}

fn assert_retired_target_noop(repo: &mut MemoryRun, command: ReduceAndAcceptHostResponse) {
    // Includes the journal/terminal source, every ledger, complete transition
    // receipts, frozen config, outbox, checkpoint and transaction ownership.
    let before = repo.clone();
    let accepted = reduce_and_accept_host_response(repo, command).unwrap();
    assert!(!accepted.commit.applied);
    assert!(accepted.commit.assistant_publications.is_empty());
    assert_eq!(accepted.commit.checkpoint_revision, before.checkpoint.revision);
    assert_eq!(accepted.commit.next_event_sequence, before.run.next_event_sequence);
    assert!(accepted.events.is_empty());
    assert!(accepted.traces.is_empty());
    let control = accepted.control.unwrap();
    assert_eq!(control.turn_status, nola_domain::AgentTurnStatus::Cancelling);
    assert!(control.completion_outcome.is_none());
    assert_eq!(*repo, before, "retired target ACK must not commit any canonical mutation");
}

fn apply_effect_deliveries(
    records: &mut BTreeMap<String, nola_domain::EffectDeliveryAttempt>,
    mutations: &[nola_domain::EffectDeliveryAttemptMutation],
) -> Result<Vec<nola_domain::EffectDeliveryAttempt>, String> {
    let mut next = records.clone();
    let mut applied = Vec::new();
    for mutation in mutations {
        nola_kernel::validate_effect_delivery_attempt(&mutation.record)?;
        match (mutation.expected_revision, next.get(&mutation.record.id)) {
            (None, None) => {}
            (Some(expected), Some(previous))
                if previous.revision == expected && mutation.record.revision == expected + 1 => {}
            _ => return Err("fixture effect-delivery CAS conflict".into()),
        }
        next.insert(mutation.record.id.clone(), mutation.record.clone());
        applied.push(mutation.record.clone());
    }
    *records = next;
    Ok(applied)
}
fn apply_attempts(
    records: &mut BTreeMap<String, nola_domain::ModelAttemptRecord>,
    mutations: &[nola_domain::ModelAttemptMutation],
) -> Result<(), String> {
    let mut next = records.clone();
    for mutation in mutations {
        let record = &mutation.record;
        match (mutation.expected_revision, next.get(&record.id)) {
            (None, None) if record.revision == 1 => {}
            (Some(expected), Some(previous))
                if previous.revision == expected && record.revision == expected + 1 => {}
            _ => return Err("fixture Attempt CAS conflict".into()),
        }
        next.insert(record.id.clone(), record.clone());
    }
    *records = next;
    Ok(())
}
fn apply_delegated_invocations(
    records: &mut BTreeMap<String, nola_domain::DelegatedInvocationRecord>,
    mutations: &[nola_domain::DelegatedInvocationMutation],
    run_id: &str,
) -> Result<(), String> {
    let mut next = records.clone();
    for mutation in mutations {
        let record = &mutation.record;
        if record.run_id != run_id {
            return Err("fixture delegated Run mismatch".into());
        }
        match (mutation.expected_revision, next.get(&record.id)) {
            (None, None)
                if record.revision == 1
                    && !next.values().any(|r| r.effect_id == record.effect_id) => {}
            (Some(expected), Some(previous))
                if previous.revision == expected
                    && expected.checked_add(1) == Some(record.revision)
                    && !previous.state.is_terminal()
                    && previous.run_id == record.run_id
                    && previous.effect_id == record.effect_id
                    && previous.backend_id == record.backend_id
                    && previous.text_part_id == record.text_part_id
                    && previous.reasoning_part_id == record.reasoning_part_id
                    && previous.created_at_ms == record.created_at_ms
                    && (previous.backend_session_id.is_none()
                        || previous.backend_session_id == record.backend_session_id) => {}
            _ => return Err("fixture delegated CAS or ownership conflict".into()),
        }
        next.insert(record.id.clone(), record.clone());
    }
    *records = next;
    Ok(())
}
fn apply_permissions(
    records: &mut BTreeMap<String, nola_domain::DelegatedPermissionDeliveryRecord>,
    owners: &BTreeMap<String, nola_domain::DelegatedInvocationRecord>,
    mutations: &[nola_domain::DelegatedPermissionDeliveryMutation],
    run_id: &str,
) -> Result<(), String> {
    let mut next = records.clone();
    for mutation in mutations {
        let r = &mutation.record;
        let owner = owners
            .get(&r.invocation_id)
            .ok_or("permission invocation missing")?;
        if r.run_id != run_id
            || owner.run_id != run_id
            || owner.effect_id != r.target_effect_id
            || owner.backend_id != r.backend_id
            || owner.backend_session_id.as_deref() != Some(r.backend_session_id.as_str())
        {
            return Err("permission target mismatch".into());
        }
        if next.values().any(|p| {
            p.id != r.id
                && ((p.invocation_id == r.invocation_id && p.occurrence_id == r.occurrence_id)
                    || (r.control_effect_id.is_some()
                        && p.control_effect_id == r.control_effect_id)
                    || (r.control_request_id.is_some()
                        && p.control_request_id == r.control_request_id))
        }) {
            return Err("permission unique identity conflict".into());
        }
        match (mutation.expected_revision, next.get(&r.id)) {
            (None, None) if r.revision == 1 => {}
            (Some(expected), Some(old))
                if old.revision == expected && expected.checked_add(1) == Some(r.revision) =>
            {
                let mut identity = r.clone();
                identity.state = old.state;
                identity.revision = old.revision;
                identity.claimed_at_ms = old.claimed_at_ms;
                identity.written_at_ms = old.written_at_ms;
                if old.control_effect_id.is_none() {
                    identity.control_effect_id = None;
                    identity.control_request_id = None;
                    identity.decision = None;
                }
                if identity != *old
                    || (old.claimed_at_ms.is_some() && old.claimed_at_ms != r.claimed_at_ms)
                    || (old.written_at_ms.is_some() && old.written_at_ms != r.written_at_ms)
                {
                    return Err("permission immutable identity conflict".into());
                }
            }
            _ => return Err("permission CAS conflict".into()),
        }
        next.insert(r.id.clone(), r.clone());
    }
    *records = next;
    Ok(())
}
fn validate_delegated_deliveries(
    records: &BTreeMap<String, nola_domain::DelegatedInvocationRecord>,
    run_id: &str,
    effects: &[OutboxEffectInput],
) -> Result<(), String> {
    for effect in effects {
        let Some(id) = &effect.delegated_invocation_id else {
            continue;
        };
        let record = records
            .get(id)
            .ok_or("typed delegated outbox owner missing")?;
        let payload = effect_binding(effect)?;
        if effect.model_attempt_id.is_some()
            || payload.delegated_invocation_id.as_deref() != Some(id.as_str())
            || record.run_id != run_id
            || record.effect_id != effect.effect_id
        {
            return Err("typed delegated outbox ownership mismatch".into());
        }
    }
    Ok(())
}
impl RunEffectRepository for MemoryRun {
    type Error = String;
    fn prepare_unstarted_cancellation_target(&self, _: &str,
        _: &nola_domain::host_control::HostEffectControl, _: &nola_domain::host_control::HostEffectControl,
        _: i64, _: i64) -> Result<Option<PreparedCancellationNotStarted>,String> {
        // This port fixture lacks the independent durable source audit that
        // certifies a revocation. It must not manufacture a positive proof.
        Ok(None)
    }
    fn pending_outbox_effect_ids(&self, run_id: &str) -> Result<Vec<String>, String> {
        Ok(self.effects.iter().filter(|effect| effect.run_id == run_id && effect.status == "pending")
            .map(|effect| effect.effect_id.clone()).collect())
    }
    fn committed_terminal_host_response(&self, _: &str, _: &str, _: &str, _: &str) -> Result<Option<nola_store_port::run_effect::CommittedHostResponseReceipt>, String> {
        Ok(None) // This fixture has no durable Host inbox; SQLite tests own replay proof.
    }
    fn checkpoint_replay_pending(&self, _: &str) -> Result<bool, String> {
        Ok(false)
    }
    fn checkpoint_replay_ready(&self, _: &str, _: &str, _: i64, _: usize) -> Result<bool, String> {
        Ok(false)
    }
    fn load_recovery_checkpoint(
        &self,
        id: &str,
    ) -> Result<Option<nola_store_port::run_effect::StoredRecoveryCheckpoint>, String> {
        Ok(self.load_checkpoint(id)?.map(|checkpoint| {
            nola_store_port::run_effect::StoredRecoveryCheckpoint {
                checkpoint,
                replay_base: None,
                replay_inputs: Vec::new(),
                frozen_model_requests: Vec::new(),
            }
        }))
    }
    fn append_host_event_history(
        &mut self,
        _: &str,
        events: &[nola_store_port::run_effect::HostEventInput],
    ) -> Result<(), String> {
        if events.is_empty() {
            Ok(())
        } else {
            Err("this direct-command fixture has no hot response history".into())
        }
    }
    fn unrecorded_memory_usage_entries(
        &self,
        entries: &[nola_store_port::memory::StoredMemoryUsageEntry],
    ) -> Result<Vec<nola_store_port::memory::StoredMemoryUsageEntry>, String> {
        if entries.is_empty() {
            Ok(Vec::new())
        } else {
            Err("this fixture has no memory ledger".into())
        }
    }
    fn load_canonical_text_state(
        &self,
        owner: &nola_domain::CanonicalStreamOwner,
    ) -> Result<nola_domain::CanonicalTextState, String> {
        use nola_domain::{CanonicalTextState, PartMutation};
        owner
            .validate()
            .map_err(|e| format!("invalid fixture owner: {e:?}"))?;
        if owner.conversation_id != self.checkpoint.conversation_id
            || owner.run_id != self.run.run_id
        {
            return Err("fixture stream owner differs from its Run".into());
        }
        let mut result = CanonicalTextState::Missing;
        if self.run.assistant_part_id.as_deref() == Some(owner.part_id.as_str()) {
            if self.run.assistant_message_id.as_deref() != Some(owner.message_id.as_str()) {
                return Err("fixture initial Part owner differs".into());
            }
            result = CanonicalTextState::Existing {
                kind: "text".into(),
                state: "open".into(),
                total_utf8_bytes: 0,
            };
        }
        for commit in &self.commits {
            for mutation in &commit.part_mutations {
                let part = match mutation {
                    PartMutation::Upsert { part_id, .. }
                    | PartMutation::AppendText { part_id, .. }
                    | PartMutation::Close { part_id, .. }
                    | PartMutation::Metadata { part_id, .. } => part_id,
                    PartMutation::Stream { operation, .. } => &operation.owner().part_id,
                };
                if part != &owner.part_id {
                    continue;
                }
                let message = commit
                    .part_message_owners
                    .get(part)
                    .map(String::as_str)
                    .or(self.run.assistant_message_id.as_deref());
                if message != Some(owner.message_id.as_str()) {
                    return Err("fixture persisted Part owner differs".into());
                }
                match mutation {
                    PartMutation::Stream { operation, .. } => {
                        if operation.owner() != owner {
                            return Err("fixture stream owner differs".into());
                        }
                        let head = match &result {
                            CanonicalTextState::Stream(head) => Some(head),
                            _ => None,
                        };
                        result = CanonicalTextState::Stream(
                            nola_domain::apply_canonical_stream(head, operation)
                                .map_err(|e| format!("fixture stream transition: {e:?}"))?,
                        );
                    }
                    PartMutation::Upsert {
                        kind,
                        state,
                        content_text,
                        ..
                    } => {
                        result = CanonicalTextState::Existing {
                            kind: kind.clone(),
                            state: state.clone(),
                            total_utf8_bytes: content_text.len() as u64,
                        };
                    }
                    PartMutation::AppendText { text, .. } => {
                        if let CanonicalTextState::Existing {
                            total_utf8_bytes, ..
                        } = &mut result
                        {
                            *total_utf8_bytes += text.len() as u64;
                        } else {
                            return Err("fixture legacy append has no original field".into());
                        }
                    }
                    PartMutation::Close { state, .. } | PartMutation::Metadata { state, .. } => {
                        if let CanonicalTextState::Existing { state: old, .. } = &mut result {
                            *old = state.clone();
                        }
                    }
                }
            }
        }
        Ok(result)
    }
    fn delegated_permission(
        &self,
        id: &str,
    ) -> Result<Option<nola_domain::DelegatedPermissionDeliveryRecord>, String> {
        Ok(self.delegated_permissions.get(id).cloned())
    }
    fn delegated_permission_for_occurrence(
        &self,
        invocation_id: &str,
        occurrence_id: &str,
    ) -> Result<Option<nola_domain::DelegatedPermissionDeliveryRecord>, String> {
        Ok(self
            .delegated_permissions
            .values()
            .find(|r| r.invocation_id == invocation_id && r.occurrence_id == occurrence_id)
            .cloned())
    }
    fn delegated_permission_for_control(
        &self,
        effect_id: &str,
    ) -> Result<Option<nola_domain::DelegatedPermissionDeliveryRecord>, String> {
        Ok(self
            .delegated_permissions
            .values()
            .find(|r| r.control_effect_id.as_deref() == Some(effect_id))
            .cloned())
    }
    fn open_delegated_permissions_for_target(
        &self,
        effect_id: &str,
    ) -> Result<Vec<nola_domain::DelegatedPermissionDeliveryRecord>, String> {
        use nola_domain::DelegatedPermissionDeliveryState as S;
        Ok(self
            .delegated_permissions
            .values()
            .filter(|r| {
                r.target_effect_id == effect_id
                    && matches!(r.state, S::AwaitingDecision | S::Prepared | S::Claimed)
            })
            .cloned()
            .collect())
    }

    fn delegated_invocation(
        &self,
        id: &str,
    ) -> Result<Option<nola_domain::DelegatedInvocationRecord>, String> {
        Ok(self.delegated_invocations.get(id).cloned())
    }
    fn delegated_invocation_for_effect(
        &self,
        id: &str,
    ) -> Result<Option<nola_domain::DelegatedInvocationRecord>, String> {
        Ok(self
            .delegated_invocations
            .values()
            .find(|r| r.effect_id == id)
            .cloned())
    }
    fn model_attempt(&self, id: &str) -> Result<Option<nola_domain::ModelAttemptRecord>, String> {
        Ok(self.attempts.get(id).cloned())
    }
    fn active_model_attempt_for_effect(
        &self,
        id: &str,
    ) -> Result<Option<nola_domain::ModelAttemptRecord>, String> {
        Ok(self
            .attempts
            .values()
            .find(|record| {
                record.effect_id == id
                    && !self
                        .attempts
                        .values()
                        .any(|child| child.predecessor_id.as_deref() == Some(record.id.as_str()))
            })
            .cloned())
    }
    fn checkpoint_revision(&self, _: &str) -> Result<Option<i64>, String> {
        Ok(Some(self.checkpoint.revision))
    }
    fn load_checkpoint(&self, _: &str) -> Result<Option<StoredCheckpoint>, String> {
        Ok(Some(self.checkpoint.clone()))
    }
    fn active_run(&self, _: &str) -> Result<Option<StoredActiveRun>, String> {
        Ok(Some(self.run.clone()))
    }
    fn load_run_invocation_config(
        &self,
        _: &str,
    ) -> Result<Option<StoredRunInvocationConfig>, String> {
        Ok(Some(self.config.clone()))
    }
    fn outbox_effect(&self, id: &str) -> Result<Option<StoredOutboxEffect>, String> {
        Ok(self
            .effects
            .iter()
            .find(|effect| effect.effect_id == id)
            .cloned())
    }
    fn outbox_effect_identity(
        &self,
        id: &str,
    ) -> Result<Option<StoredOutboxEffectIdentity>, String> {
        self.effects
            .iter()
            .find(|effect| effect.effect_id == id)
            .map(|effect| {
                let request: HostRequestEnvelope =
                    serde_json::from_str(&effect.payload_json).map_err(|e| e.to_string())?;
                Ok(StoredOutboxEffectIdentity {
                    effect_id: effect.effect_id.clone(),
                    run_id: effect.run_id.clone(),
                    request_id: effect.request_id.clone(),
                    effect_kind: effect.effect_kind.clone(),
                    status: effect.status.clone(),
                    binding: nola_store_port::host_replay::CoreHostBindingOverlay::from_request(
                        &request,
                    ),
                })
            })
            .transpose()
    }
    fn load_committed_host_delivery(
        &self,
        _: &str,
    ) -> Result<CommittedHostDeliverySnapshot, String> {
        Ok(CommittedHostDeliverySnapshot {
            conversation_id: Some(self.checkpoint.conversation_id.clone()),
            checkpoint: Some(self.checkpoint.clone()),
            effects: self
                .effects
                .iter()
                .filter(|effect| effect.status == "pending")
                .cloned()
                .collect(),
        })
    }
    fn load_committed_host_delivery_witness(
        &self,
        id: &str,
    ) -> Result<
        CommittedHostDeliverySnapshot<nola_store_port::run_effect::StoredDeliveryCheckpointWitness>,
        String,
    > {
        let snapshot = self.load_committed_host_delivery(id)?;
        Ok(CommittedHostDeliverySnapshot {
            conversation_id: snapshot.conversation_id,
            effects: snapshot.effects,
            checkpoint: snapshot.checkpoint.map(|checkpoint| {
                nola_store_port::run_effect::StoredDeliveryCheckpointWitness {
                    conversation_id: checkpoint.conversation_id,
                    witness_json: checkpoint.checkpoint_json,
                }
            }),
        })
    }
    fn load_selected_host_delivery_witness(
        &self,
        id: &str,
        selected: Option<&std::collections::HashSet<String>>,
    ) -> Result<
        CommittedHostDeliverySnapshot<nola_store_port::run_effect::StoredDeliveryCheckpointWitness>,
        String,
    > {
        let mut snapshot = self.load_committed_host_delivery_witness(id)?;
        if let Some(selected) = selected {
            snapshot
                .effects
                .retain(|effect| selected.contains(&effect.effect_id));
        }
        Ok(snapshot)
    }
    fn load_effect_heartbeat_snapshot(
        &self,
        id: &str,
        effect_id: &str,
    ) -> Result<Option<nola_store_port::run_effect::StoredEffectHeartbeatSnapshot>, String> {
        let snapshot = self.load_committed_host_delivery_witness(id)?;
        let Some(effect) = snapshot
            .effects
            .iter()
            .find(|effect| effect.effect_id == effect_id && effect.run_id == id)
        else {
            return Ok(None);
        };
        let mut checkpoint = snapshot.checkpoint;
        if let Some(checkpoint) = checkpoint.as_mut() {
            let request: serde_json::Value =
                serde_json::from_str(&effect.payload_json).map_err(|e| e.to_string())?;
            let mut witness: serde_json::Value =
                serde_json::from_str(&checkpoint.witness_json).map_err(|e| e.to_string())?;
            if request["payload"]["type"] == "request_permission" {
                witness["state"]["host"]["pending_host_effects"]["effects"][effect_id]
                    ["permission_request_id"] = request["payload"]["permission_request_id"].clone();
            }
            checkpoint.witness_json = witness.to_string();
        }
        Ok(Some(
            nola_store_port::run_effect::StoredEffectHeartbeatSnapshot {
                effect_id: effect.effect_id.clone(),
                run_id: effect.run_id.clone(),
                request_id: effect.request_id.clone(),
                checkpoint,
            },
        ))
    }
    fn commit_transition(
        &mut self,
        input: &CommitTransitionInput,
    ) -> Result<CommitTransitionResult, String> {
        // Fixture provides one atomic operation; no alternate Store methods exist.
        if self.fail_commit {
            return Err("fixture commit unavailable".into());
        }
        if input.expected_checkpoint_revision != self.checkpoint.revision {
            return Err("fixture stale revision".into());
        }
        let mut attempts = self.attempts.clone();
        let mut delegated = self.delegated_invocations.clone();
        apply_attempts(&mut attempts, &input.model_attempt_mutations)?;
        apply_delegated_invocations(
            &mut delegated,
            &input.delegated_invocation_mutations,
            &input.run_id,
        )?;
        validate_delegated_deliveries(&delegated, &input.run_id, &input.outbox_effects)?;
        let mut permissions = self.delegated_permissions.clone();
        apply_permissions(
            &mut permissions,
            &delegated,
            &input.delegated_permission_mutations,
            &input.run_id,
        )?;
        let mut effect_deliveries = self.effect_deliveries.clone();
        apply_effect_deliveries(
            &mut effect_deliveries,
            &input.effect_delivery_attempt_mutations,
        )?;
        self.delegated_permissions = permissions;
        self.effect_deliveries = effect_deliveries;
        self.attempts = attempts;
        self.delegated_invocations = delegated;
        self.checkpoint.revision = input.checkpoint_revision;
        self.checkpoint.event_cursor = input.checkpoint_event_cursor;
        self.checkpoint.checkpoint_json = match &input.checkpoint {
            nola_store_port::run_effect::CheckpointWrite::Baseline { checkpoint_json } => {
                checkpoint_json.clone()
            }
            nola_store_port::run_effect::CheckpointWrite::Replay { .. } => {
                return Err("fixture does not implement incremental checkpoints".into())
            }
        };
        let checkpoint: nola_agent_engine::agent::recovery::CoreCheckpoint<
            nola_agent_engine::agent::AgentThreadState,
        > = serde_json::from_str(&self.checkpoint.checkpoint_json).map_err(|e| e.to_string())?;
        if let Some(turn) = checkpoint.state.execution.active_turn {
            self.core_turn_ids.insert(turn.turn_id.as_str().into());
        }
        self.run.next_event_sequence += input.events.len() as i64;
        for id in &input.acknowledged_effect_ids {
            self.effects
                .iter_mut()
                .find(|effect| &effect.effect_id == id)
                .unwrap()
                .status = "acknowledged".into();
        }
        for successor in &input.outbox_effects {
            let payload_json = effect_payload(
                successor,
                &input.run_id,
                self.effects
                    .iter()
                    .find(|effect| effect.effect_id == successor.effect_id),
            )?;
            if let Some(attempt_id) = &successor.model_attempt_id {
                let attempt = self
                    .attempts
                    .get(attempt_id)
                    .expect("typed outbox Attempt exists");
                let payload: HostRequestEnvelope = serde_json::from_str(&payload_json).unwrap();
                assert_eq!(payload.attempt_id.as_deref(), Some(attempt_id.as_str()));
                assert_eq!(attempt.run_id, input.run_id);
                assert_eq!(attempt.effect_id, successor.effect_id);
                assert_eq!(payload.attempt_started, attempt.started_at_ms.is_some());
            }
            let effect = StoredOutboxEffect {
                effect_id: successor.effect_id.clone(),
                run_id: input.run_id.clone(),
                request_id: successor.request_id.clone(),
                effect_kind: successor.effect_kind.clone(),
                payload_json,
                status: "pending".into(),
                created_at_ms: 1,
                acknowledged_at_ms: None,
            };
            if let Some(existing) = self
                .effects
                .iter_mut()
                .find(|e| e.effect_id == effect.effect_id)
            {
                *existing = effect;
            } else {
                self.effects.push(effect);
            }
        }
        self.commits.push(input.clone());
        Ok(CommitTransitionResult {
            applied: true,
            checkpoint_revision: self.checkpoint.revision,
            next_event_sequence: self.run.next_event_sequence,
        })
    }
}

fn runtime_capability(id: &str, route: HostCapabilityRoute) -> HostCapabilityDescriptor {
    HostCapabilityDescriptor {
        id: id.to_string(),
        title: id.to_string(),
        route,
        availability: CapabilityAvailability::Available,
        exposure: HostCapabilityExposure::Internal,
        permission_domain: None,
        input_schema: serde_json::json!({"type": "object"}),
        input_normalization: Default::default(),
        output_policy: HostOutputPolicy {
            preview_char_limit: 4_000,
            persist_full_output: true,
            allow_artifacts: true,
        },
        supports_streaming: id == "model.stream",
        supports_argument_streaming: false,
        parallelism: HostCapabilityParallelism::Serial,
        cancellation: HostCancellationMode::Cooperative,
        replay_safety: HostReplaySafety::Recoverable,
    }
}

fn runtime_config() -> AgentCoreConfig {
    let mut capabilities = BTreeMap::new();
    capabilities.insert(
        "model.stream".to_string(),
        runtime_capability("model.stream", HostCapabilityRoute::Model),
    );
    AgentCoreConfig {
        turn_identity: fixture_model_identity::identity(),
        session_id: Some(SessionId::from_string("recovery-conversation".to_string())),
        model: Some("test-model".to_string()),
        host_catalog: Some(HostCapabilityCatalog {
            host_id: HostId("recovery-host".to_string()),
            revision: 1,
            availability_epoch: 1,
            lease: None,
            capabilities,
        }),
        ..AgentCoreConfig::default()
    }
}

fn response_for_request(
    request: &HostRequestEnvelope,
    sequence: u64,
    payload: HostResponse,
) -> HostResponseEnvelope {
    HostResponseEnvelope {
        attempt_id: request.attempt_id.clone(),
        delegated_invocation_id: request.delegated_invocation_id.clone(),
        request_id: request.request_id.clone(),
        effect_id: request.effect_id.clone(),
        host_id: request.host_id.clone(),
        lease_id: request.lease_id.clone(),
        session_id: request.session_id.clone(),
        turn_id: request.turn_id.clone(),
        catalog_revision: request.catalog_revision,
        availability_epoch: request.availability_epoch,
        delivery: request.delivery.clone(),
        sequence,
        payload,
    }
}

fn seed() -> (MemoryRun, HostRequestEnvelope) {
    let config = runtime_config();
    let config_json = serde_json::to_string(&config).unwrap();
    let digest = format!("{:x}", Sha256::digest(config_json.as_bytes()));
    let mut core = AgentCore::try_new(config).unwrap();
    let transition = core
        .submit(AgentSubmission::UserInput {
            items: vec![AgentItem::user_message(
                ItemId::from_string("user".into()),
                "hello",
            )],
            turn_context_items: vec![],
            turn_context_platform: Default::default(),
            turn_context_components: vec![],
            submission_id: Some("run".into()),
            submitted_at_epoch_ms: Some(1000),
            delivery: AgentInputDelivery::CurrentTurn,
        })
        .unwrap();
    let mut request = transition
        .host_requests
        .into_iter()
        .find(|r| matches!(r.payload, HostRequest::ModelStream { .. }))
        .unwrap();
    let part_id = match &request.payload {
        HostRequest::ModelStream { text_part_id, .. } => text_part_id.clone(),
        _ => unreachable!(),
    };
    let run_id = request.turn_id.clone().unwrap();
    let (text, reasoning) = match &request.payload {
        HostRequest::ModelStream {
            text_part_id,
            reasoning_part_id,
            ..
        } => (text_part_id.clone(), reasoning_part_id.clone()),
        _ => unreachable!(),
    };
    let attempt = nola_domain::ModelAttemptRecord {
        id: "fixture-initial-attempt".into(),
        run_id: run_id.clone(),
        invocation_id: request.effect_id.0.clone(),
        effect_id: request.effect_id.0.clone(),
        text_part_id: text,
        reasoning_part_id: reasoning,
        predecessor_id: None,
        state: nola_domain::ModelAttemptState::Running,
        provider: Some("fixture".into()),
        model: Some("test-model".into()),
        failure: None,
        usage: serde_json::json!({}),
        created_at_ms: 1000,
        started_at_ms: Some(1000),
        completed_at_ms: None,
        revision: 1,
    };
    request.attempt_id = Some(attempt.id.clone());
    request.attempt_started = true;
    let checkpoint = core.checkpoint(CheckpointReason::Manual);
    let checkpoint = StoredCheckpoint {
        conversation_id: "recovery-conversation".into(),
        revision: 1,
        event_cursor: checkpoint.last_event_cursor.sequence as i64,
        checkpoint_json: serde_json::to_string(&checkpoint).unwrap(),
        updated_at_ms: 1000,
    };
    (
        MemoryRun {
            journal: Default::default(),
            transaction_backup: None,
            core_turn_ids: std::collections::BTreeSet::from([run_id.clone()]),
            checkpoint,
            run: StoredActiveRun {
                run_id: run_id.clone(),
                kind: "conversation".into(),
                assistant_message_id: Some(format!("{run_id}:assistant")),
                assistant_part_id: Some(part_id),
                next_event_sequence: 1,
                invocation_config_digest: Some(digest.clone()),
            },
            config: StoredRunInvocationConfig {
                run_id: run_id.clone(),
                invocation_config_json: config_json,
                invocation_config_digest: digest,
            },
            effects: vec![StoredOutboxEffect {
                effect_id: request.effect_id.0.clone(),
                run_id,
                request_id: request.request_id.clone(),
                effect_kind: "model_stream".into(),
                payload_json: serde_json::to_string(&request).unwrap(),
                status: "pending".into(),
                created_at_ms: 1000,
                acknowledged_at_ms: None,
            }],
            commits: vec![],
            fail_commit: false,
            attempts: BTreeMap::from([(attempt.id.clone(), attempt)]),
            delegated_invocations: BTreeMap::new(),
            delegated_permissions: BTreeMap::new(),
            effect_deliveries: BTreeMap::new(),
        },
        request,
    )
}
fn command(
    repo: &MemoryRun,
    request: &HostRequestEnvelope,
    sequence: u64,
    payload: HostResponse,
) -> ReduceAndAcceptHostResponse {
    ReduceAndAcceptHostResponse {
        conversation_id: repo.checkpoint.conversation_id.clone(),
        run_id: repo.run.run_id.clone(),
        invocation_config_digest: repo.config.invocation_config_digest.clone(),
        response: response_for_request(request, sequence, payload),
        received_at_ms: 2000,
    }
}

fn claim_request<R>(repo: &mut R, run_id: &str, now_ms: i64) -> HostRequestEnvelope
where
    R: RunEffectRepository<Error = String>
        + nola_store_port::effect_delivery::EffectDeliveryRepository<Error = String>,
{
    claim_committed_host_requests(
        repo,
        ClaimCommittedHostRequests {
            run_id: run_id.into(),
            worker_generation: format!("generation-{now_ms}"),
            lease_owner: format!("worker-{now_ms}"),
            claimed_at_ms: now_ms,
            lease_expires_at_ms: now_ms + 60_000,
            limit: 1,
        },
    )
    .unwrap()
    .requests
    .into_iter()
    .next()
    .unwrap()
}

fn claim_and_start<R>(
    repo: &mut R,
    run_id: &str,
    conversation_id: &str,
    digest: &str,
    now_ms: i64,
) -> HostRequestEnvelope
where
    R: RunEffectRepository<Error = String>
        + nola_application::repositories::AssistantPublicationRepositories
        + nola_store_port::effect_delivery::EffectDeliveryRepository<Error = String>,
{
    let mut request = claim_request(repo, run_id, now_ms);
    reduce_and_accept_host_response(
        repo,
        ReduceAndAcceptHostResponse {
            conversation_id: conversation_id.into(),
            run_id: run_id.into(),
            invocation_config_digest: digest.into(),
            response: response_for_request(&request, 1, HostResponse::ExecutionStarted),
            received_at_ms: now_ms + 1,
        },
    )
    .unwrap();
    request.resume_after_sequence = Some(1);
    request
}

#[test]
fn real_cold_host_reduction_uses_only_the_port_and_atomic_commit() {
    let (mut repo, _) = seed();
    let run_id = repo.run.run_id.clone();
    let conversation_id = repo.checkpoint.conversation_id.clone();
    let digest = repo.config.invocation_config_digest.clone();
    let request = claim_and_start(&mut repo, &run_id, &conversation_id, &digest, 1_100);
    let restored = load_committed_host_requests(&repo, &repo.run.run_id).unwrap();
    assert_eq!(restored.effect_ids, vec![request.effect_id.0.clone()]);
    let delta = command(
        &repo,
        &request,
        2,
        HostResponse::ModelDelta {
            channel: ModelDeltaChannel::Text,
            part_id: repo.run.assistant_part_id.clone().unwrap(),
            delta: "original".into(),
        },
    );
    repo.fail_commit = true;
    let before = repo.checkpoint.clone();
    let before_commits = repo.commits.len();
    assert!(reduce_and_accept_host_response(&mut repo, delta.clone())
        .unwrap_err()
        .contains("fixture commit unavailable"));
    assert_eq!(repo.checkpoint, before);
    assert_eq!(repo.commits.len(), before_commits);
    repo.fail_commit = false;
    // Publication is part of the same command, not best-effort presentation.
    // Failure after the canonical commit must restore checkpoint, outbox and
    // all prepared content, order reservations and visible journal heads.
    repo.journal.fail_publication = true;
    let journal_before = repo.journal.clone();
    let effects_before = serde_json::to_value(&repo.effects).unwrap();
    assert!(reduce_and_accept_host_response(&mut repo, delta.clone())
        .unwrap_err()
        .contains("fixture publication unavailable"));
    assert_eq!(repo.checkpoint, before);
    assert_eq!(repo.commits.len(), before_commits);
    assert_eq!(serde_json::to_value(&repo.effects).unwrap(), effects_before);
    assert_eq!(repo.journal, journal_before);
    repo.journal.fail_publication = false;
    let accepted = reduce_and_accept_host_response(&mut repo, delta).unwrap();
    assert!(accepted.commit.applied);
    assert!(repo.journal.record_count() > journal_before.record_count());
    let input = repo.commits.last().unwrap();
    assert_eq!(input.expected_checkpoint_revision, before.revision);
    assert!(input.host_event.is_some());
    assert!(!input.events.is_empty());
    assert!(!input.part_mutations.is_empty());
    assert!(input.acknowledged_effect_ids.is_empty());
    // A second call reconstructs Core from the Port checkpoint, never a retained hot Core.
    let mut cold = repo.clone();
    let delivery = load_committed_host_requests(&cold, &cold.run.run_id).unwrap();
    assert_eq!(delivery.requests[0].resume_after_sequence, Some(2));
    let terminal = command(
        &cold,
        &request,
        3,
        HostResponse::ModelCompleted {
            items: vec![],
            tool_calls: vec![],
            usage: serde_json::json!({}),
        },
    );
    let before = cold.clone();
    assert!(reduce_and_accept_host_response(&mut cold, terminal).unwrap_err()
        .contains("terminal result sealing requires the real SQLite fixture"));
    assert_eq!(cold.checkpoint, before.checkpoint);
    assert_eq!(cold.journal, before.journal);
    assert_eq!(serde_json::to_value(&cold.effects).unwrap(), serde_json::to_value(&before.effects).unwrap());
    // Real terminal ACK/seal atomicity now lives in turn_transaction_e2e;
    // the in-memory adapter deliberately cannot stand in for Store authority.
}

#[test]
fn malformed_delivery_identity_is_rejected_without_a_commit() {
    let (mut repo, _) = seed();
    repo.effects[0].run_id = "other-run".into();
    assert!(load_committed_host_requests(&repo, &repo.run.run_id)
        .unwrap_err()
        .contains("Run/session identity mismatch"));
    assert!(repo.commits.is_empty());
    let (mut repo, _) = seed();
    repo.effects[0].request_id = "changed-request".into();
    assert!(load_committed_host_requests(&repo, &repo.run.run_id)
        .unwrap_err()
        .contains("identity mismatch"));
    assert!(repo.commits.is_empty());
}

#[derive(Clone, Default)]
struct MemoryCreation {
    journal: publication::MemoryJournal,
    transaction_backup: Option<Box<MemoryCreation>>,
    opened: Option<MemoryRun>,
    models: admission_models::Models,
    goal_bindings: BTreeMap<(String,String), nola_store_port::goal_execution::StoredGoalExecutionBinding>,
    fail_creation: bool,
    // Observation counters are not transactional business state.
    history_reads: std::rc::Rc<std::cell::Cell<usize>>,
}
impl RunEffectRepository for MemoryCreation {
    type Error = String;
    fn prepare_unstarted_cancellation_target(&self, run: &str,
        cancellation: &nola_domain::host_control::HostEffectControl, target: &nola_domain::host_control::HostEffectControl,
        revision: i64, at: i64) -> Result<Option<PreparedCancellationNotStarted>,String> {
        self.opened.as_ref().ok_or("no Run")?.prepare_unstarted_cancellation_target(run,cancellation,target,revision,at)
    }
    fn pending_outbox_effect_ids(&self, run_id: &str) -> Result<Vec<String>, String> {
        self.opened.as_ref().ok_or("no Run")?.pending_outbox_effect_ids(run_id)
    }
    fn committed_terminal_host_response(&self, _: &str, _: &str, _: &str, _: &str) -> Result<Option<nola_store_port::run_effect::CommittedHostResponseReceipt>, String> {
        Ok(None)
    }
    fn checkpoint_replay_pending(&self, _: &str) -> Result<bool, String> {
        Ok(false)
    }
    fn checkpoint_replay_ready(&self, _: &str, _: &str, _: i64, _: usize) -> Result<bool, String> {
        Ok(false)
    }
    fn load_recovery_checkpoint(
        &self,
        id: &str,
    ) -> Result<Option<nola_store_port::run_effect::StoredRecoveryCheckpoint>, String> {
        match &self.opened {
            Some(run) => run.load_recovery_checkpoint(id),
            None => Ok(None),
        }
    }
    fn append_host_event_history(
        &mut self,
        run_id: &str,
        events: &[nola_store_port::run_effect::HostEventInput],
    ) -> Result<(), String> {
        self.opened
            .as_mut()
            .ok_or("no Run")?
            .append_host_event_history(run_id, events)
    }
    fn unrecorded_memory_usage_entries(
        &self,
        entries: &[nola_store_port::memory::StoredMemoryUsageEntry],
    ) -> Result<Vec<nola_store_port::memory::StoredMemoryUsageEntry>, String> {
        match &self.opened {
            Some(run) => run.unrecorded_memory_usage_entries(entries),
            None => Err("no opened Run ledger".into()),
        }
    }
    fn load_canonical_text_state(
        &self,
        owner: &nola_domain::CanonicalStreamOwner,
    ) -> Result<nola_domain::CanonicalTextState, String> {
        match &self.opened {
            Some(run) => run.load_canonical_text_state(owner),
            None => Ok(nola_domain::CanonicalTextState::Missing),
        }
    }
    fn delegated_permission(
        &self,
        id: &str,
    ) -> Result<Option<nola_domain::DelegatedPermissionDeliveryRecord>, String> {
        self.opened
            .as_ref()
            .map_or(Ok(None), |r| r.delegated_permission(id))
    }
    fn delegated_permission_for_occurrence(
        &self,
        invocation_id: &str,
        occurrence_id: &str,
    ) -> Result<Option<nola_domain::DelegatedPermissionDeliveryRecord>, String> {
        self.opened.as_ref().map_or(Ok(None), |r| {
            r.delegated_permission_for_occurrence(invocation_id, occurrence_id)
        })
    }
    fn delegated_permission_for_control(
        &self,
        effect_id: &str,
    ) -> Result<Option<nola_domain::DelegatedPermissionDeliveryRecord>, String> {
        self.opened
            .as_ref()
            .map_or(Ok(None), |r| r.delegated_permission_for_control(effect_id))
    }
    fn open_delegated_permissions_for_target(
        &self,
        effect_id: &str,
    ) -> Result<Vec<nola_domain::DelegatedPermissionDeliveryRecord>, String> {
        self.opened.as_ref().map_or(Ok(vec![]), |r| {
            r.open_delegated_permissions_for_target(effect_id)
        })
    }

    fn delegated_invocation(
        &self,
        id: &str,
    ) -> Result<Option<nola_domain::DelegatedInvocationRecord>, String> {
        self.opened
            .as_ref()
            .map_or(Ok(None), |r| r.delegated_invocation(id))
    }
    fn delegated_invocation_for_effect(
        &self,
        id: &str,
    ) -> Result<Option<nola_domain::DelegatedInvocationRecord>, String> {
        self.opened
            .as_ref()
            .map_or(Ok(None), |r| r.delegated_invocation_for_effect(id))
    }
    fn model_attempt(&self, id: &str) -> Result<Option<nola_domain::ModelAttemptRecord>, String> {
        self.opened
            .as_ref()
            .map_or(Ok(None), |r| r.model_attempt(id))
    }
    fn active_model_attempt_for_effect(
        &self,
        id: &str,
    ) -> Result<Option<nola_domain::ModelAttemptRecord>, String> {
        self.opened
            .as_ref()
            .map_or(Ok(None), |r| r.active_model_attempt_for_effect(id))
    }
    fn checkpoint_revision(&self, id: &str) -> Result<Option<i64>, String> {
        self.opened
            .as_ref()
            .map_or(Ok(None), |r| r.checkpoint_revision(id))
    }
    fn load_checkpoint(&self, id: &str) -> Result<Option<StoredCheckpoint>, String> {
        self.opened
            .as_ref()
            .map_or(Ok(None), |r| r.load_checkpoint(id))
    }
    fn active_run(&self, id: &str) -> Result<Option<StoredActiveRun>, String> {
        self.opened.as_ref().map_or(Ok(None), |r| r.active_run(id))
    }
    fn load_run_invocation_config(
        &self,
        id: &str,
    ) -> Result<Option<StoredRunInvocationConfig>, String> {
        self.opened
            .as_ref()
            .map_or(Ok(None), |r| r.load_run_invocation_config(id))
    }
    fn outbox_effect(&self, id: &str) -> Result<Option<StoredOutboxEffect>, String> {
        self.opened
            .as_ref()
            .map_or(Ok(None), |r| r.outbox_effect(id))
    }
    fn outbox_effect_identity(
        &self,
        id: &str,
    ) -> Result<Option<StoredOutboxEffectIdentity>, String> {
        self.opened
            .as_ref()
            .map_or(Ok(None), |r| r.outbox_effect_identity(id))
    }
    fn load_committed_host_delivery(
        &self,
        id: &str,
    ) -> Result<CommittedHostDeliverySnapshot, String> {
        self.opened
            .as_ref()
            .ok_or("no Run")?
            .load_committed_host_delivery(id)
    }
    fn load_committed_host_delivery_witness(
        &self,
        id: &str,
    ) -> Result<
        CommittedHostDeliverySnapshot<nola_store_port::run_effect::StoredDeliveryCheckpointWitness>,
        String,
    > {
        self.opened
            .as_ref()
            .ok_or("no Run")?
            .load_committed_host_delivery_witness(id)
    }
    fn load_selected_host_delivery_witness(
        &self,
        id: &str,
        selected: Option<&std::collections::HashSet<String>>,
    ) -> Result<
        CommittedHostDeliverySnapshot<nola_store_port::run_effect::StoredDeliveryCheckpointWitness>,
        String,
    > {
        self.opened
            .as_ref()
            .ok_or("no Run")?
            .load_selected_host_delivery_witness(id, selected)
    }
    fn load_effect_heartbeat_snapshot(
        &self,
        id: &str,
        effect_id: &str,
    ) -> Result<Option<nola_store_port::run_effect::StoredEffectHeartbeatSnapshot>, String> {
        self.opened
            .as_ref()
            .ok_or("no Run")?
            .load_effect_heartbeat_snapshot(id, effect_id)
    }
    fn commit_transition(
        &mut self,
        input: &CommitTransitionInput,
    ) -> Result<CommitTransitionResult, String> {
        self.opened
            .as_mut()
            .ok_or("no Run")?
            .commit_transition(input)
    }
}

impl nola_store_port::effect_delivery::EffectDeliveryRepository for MemoryRun {
    type Error = String;
    fn unknown_effect_recovery_receipt(&self, _: &str) -> Result<Option<(String,String)>,String> { Err("not supported by fixture".into()) }
    fn record_unknown_effect_recovery_receipt(&mut self, _: &str, _: &str, _: &str) -> Result<(),String> { Err("not supported by fixture".into()) }

    fn runtime_effect_work_schedule(
        &self,
        _: &[String],
        _: i64,
        _: usize,
    ) -> Result<nola_store_port::effect_delivery::RuntimeEffectWorkSchedule, String> {
        Ok(
            nola_store_port::effect_delivery::RuntimeEffectWorkSchedule {
                runs: Vec::new(),
                next_wake_at_ms: None,
            },
        )
    }

    fn pending_effect_delivery_candidates(
        &self,
        run_id: &str,
        _: Option<&nola_store_port::effect_delivery::EffectDeliveryCandidateCursor>,
        limit: usize,
    ) -> Result<Vec<nola_store_port::effect_delivery::EffectDeliveryCandidate>, String> {
        Ok(self
            .effects
            .iter()
            .filter(|effect| {
                effect.run_id == run_id
                    && effect.status == "pending"
                    && !self
                        .effect_deliveries
                        .values()
                        .any(|a| a.effect_id == effect.effect_id)
            })
            .take(limit)
            .map(
                |effect| nola_store_port::effect_delivery::EffectDeliveryCandidate {
                    effect_id: effect.effect_id.clone(),
                    run_id: effect.run_id.clone(),
                    request_id: effect.request_id.clone(),
                    created_at_ms: effect.created_at_ms,
                },
            )
            .collect())
    }

    fn effect_delivery_attempt(
        &self,
        id: &str,
    ) -> Result<Option<nola_domain::EffectDeliveryAttempt>, String> {
        Ok(self.effect_deliveries.get(id).cloned())
    }

    fn active_effect_delivery_attempt(
        &self,
        effect_id: &str,
    ) -> Result<Option<nola_domain::EffectDeliveryAttempt>, String> {
        Ok(self
            .effect_deliveries
            .values()
            .find(|attempt| attempt.effect_id == effect_id && !attempt.state.is_terminal())
            .cloned())
    }

    fn interrupted_effect_delivery_candidates(
        &self,
        run_id: &str,
        _: Option<&nola_store_port::effect_delivery::InterruptedEffectDeliveryCursor>,
        limit: usize,
        _include_unrebound: bool,
    ) -> Result<Vec<nola_domain::EffectDeliveryAttempt>, String> {
        Ok(self
            .effect_deliveries
            .values()
            .filter(|attempt| {
                attempt.run_id == run_id
                    && attempt.state == nola_domain::EffectDeliveryAttemptState::Interrupted
                    && attempt.recovery_plan.is_some()
                    && !self.effect_deliveries.values().any(|candidate| {
                        candidate.predecessor_id.as_deref() == Some(attempt.id.as_str())
                    })
            })
            .take(limit)
            .cloned()
            .collect())
    }

    fn claim_effect_delivery_attempts(
        &mut self,
        mutations: &[nola_domain::EffectDeliveryAttemptMutation],
    ) -> Result<Vec<nola_domain::EffectDeliveryAttempt>, String> {
        apply_effect_deliveries(&mut self.effect_deliveries, mutations)
    }

    fn heartbeat_effect_delivery_attempt(
        &mut self,
        mutation: &nola_domain::EffectDeliveryAttemptMutation,
    ) -> Result<nola_domain::EffectDeliveryAttempt, String> {
        apply_effect_deliveries(&mut self.effect_deliveries, std::slice::from_ref(mutation))?
            .into_iter()
            .next()
            .ok_or_else(|| "heartbeat produced no effect delivery attempt".to_string())
    }

    fn authorize_unknown_effect_retry(
        &mut self,
        mutation: &nola_domain::EffectDeliveryAttemptMutation,
    ) -> Result<nola_domain::EffectDeliveryAttempt, String> {
        apply_effect_deliveries(&mut self.effect_deliveries, std::slice::from_ref(mutation))?
            .into_iter()
            .next()
            .ok_or_else(|| "authorization produced no effect delivery attempt".to_string())
    }

    fn commit_effect_delivery_attempt_mutations(
        &mut self,
        mutations: &[nola_domain::EffectDeliveryAttemptMutation],
    ) -> Result<Vec<nola_domain::EffectDeliveryAttempt>, String> {
        apply_effect_deliveries(&mut self.effect_deliveries, mutations)
    }
}

impl nola_store_port::effect_delivery::EffectDeliveryRepository for MemoryCreation {
    type Error = String;
    fn unknown_effect_recovery_receipt(&self, _: &str) -> Result<Option<(String,String)>,String> { Err("not supported by fixture".into()) }
    fn record_unknown_effect_recovery_receipt(&mut self, _: &str, _: &str, _: &str) -> Result<(),String> { Err("not supported by fixture".into()) }

    fn runtime_effect_work_schedule(
        &self,
        conversation_ids: &[String],
        now_ms: i64,
        limit: usize,
    ) -> Result<nola_store_port::effect_delivery::RuntimeEffectWorkSchedule, String> {
        self.opened
            .as_ref()
            .ok_or("no Run")?
            .runtime_effect_work_schedule(conversation_ids, now_ms, limit)
    }

    fn pending_effect_delivery_candidates(
        &self,
        run_id: &str,
        after: Option<&nola_store_port::effect_delivery::EffectDeliveryCandidateCursor>,
        limit: usize,
    ) -> Result<Vec<nola_store_port::effect_delivery::EffectDeliveryCandidate>, String> {
        self.opened
            .as_ref()
            .ok_or("no Run")?
            .pending_effect_delivery_candidates(run_id, after, limit)
    }
    fn effect_delivery_attempt(
        &self,
        id: &str,
    ) -> Result<Option<nola_domain::EffectDeliveryAttempt>, String> {
        self.opened
            .as_ref()
            .map_or(Ok(None), |run| run.effect_delivery_attempt(id))
    }
    fn active_effect_delivery_attempt(
        &self,
        id: &str,
    ) -> Result<Option<nola_domain::EffectDeliveryAttempt>, String> {
        self.opened
            .as_ref()
            .map_or(Ok(None), |run| run.active_effect_delivery_attempt(id))
    }
    fn interrupted_effect_delivery_candidates(
        &self,
        run_id: &str,
        after: Option<&nola_store_port::effect_delivery::InterruptedEffectDeliveryCursor>,
        limit: usize,
        include_unrebound: bool,
    ) -> Result<Vec<nola_domain::EffectDeliveryAttempt>, String> {
        self.opened
            .as_ref()
            .ok_or("no Run")?
            .interrupted_effect_delivery_candidates(run_id, after, limit, include_unrebound)
    }
    fn claim_effect_delivery_attempts(
        &mut self,
        mutations: &[nola_domain::EffectDeliveryAttemptMutation],
    ) -> Result<Vec<nola_domain::EffectDeliveryAttempt>, String> {
        self.opened
            .as_mut()
            .ok_or("no Run")?
            .claim_effect_delivery_attempts(mutations)
    }
    fn heartbeat_effect_delivery_attempt(
        &mut self,
        mutation: &nola_domain::EffectDeliveryAttemptMutation,
    ) -> Result<nola_domain::EffectDeliveryAttempt, String> {
        self.opened
            .as_mut()
            .ok_or("no Run")?
            .heartbeat_effect_delivery_attempt(mutation)
    }
    fn authorize_unknown_effect_retry(
        &mut self,
        mutation: &nola_domain::EffectDeliveryAttemptMutation,
    ) -> Result<nola_domain::EffectDeliveryAttempt, String> {
        self.opened
            .as_mut()
            .ok_or("no Run")?
            .authorize_unknown_effect_retry(mutation)
    }
    fn commit_effect_delivery_attempt_mutations(
        &mut self,
        mutations: &[nola_domain::EffectDeliveryAttemptMutation],
    ) -> Result<Vec<nola_domain::EffectDeliveryAttempt>, String> {
        self.opened
            .as_mut()
            .ok_or("no Run")?
            .commit_effect_delivery_attempt_mutations(mutations)
    }
}

impl nola_store_port::historical_part_projection::HistoricalPartProjectionRepository
    for MemoryCreation
{
    type Error = String;
    fn read_historical_part_with_budget(
        &self,
        _: &nola_domain::CanonicalPartOwner,
        _: i64,
        _: usize,
    ) -> Result<nola_domain::HistoricalPartRead, String> {
        Err("empty fixture has no Parts".into())
    }
    fn read_historical_part(
        &self,
        _: &nola_domain::CanonicalPartOwner,
        _: i64,
        _: usize,
    ) -> Result<Option<nola_domain::HistoricalPartProjection>, String> {
        Err("empty fixture has no Parts".into())
    }
}
impl nola_store_port::tool_source::ToolSourceRepository for MemoryCreation {
    fn tool_sources_for_part(&self,_:&nola_domain::CanonicalPartOwner,_:i64)->Result<Vec<nola_store_port::tool_source::ToolSourceBinding>,String> {
        Err("empty fixture has no tool Parts".into())
    }
    fn tool_source_by_id(&self,_:&str,_:&str,_:i64)->Result<Option<nola_store_port::tool_source::ToolSourceBinding>,String> {
        Err("empty fixture has no tool sources".into())
    }
    fn read_tool_source_range(&self,_:&nola_store_port::tool_source::ToolSourceBinding,_:i64,_:u64,_:usize)->Result<nola_store_port::tool_source::ToolSourceRange,String> {
        Err("empty fixture has no tool sources".into())
    }
}
impl nola_store_port::conversation_history::CanonicalHistoryRepository for MemoryCreation {
    fn load_conversation_message_window(
        &self,
        _: &str,
        _: Option<i64>,
        _: usize,
        _: usize,
    ) -> Result<
        Option<nola_store_port::conversation_history::StoredConversationMessageWindow>,
        String,
    > {
        self.history_reads.set(self.history_reads.get() + 1);
        Ok(None)
    }
}
impl nola_store_port::run_creation::RunCreationRepository for MemoryCreation {
    fn ui_surfaces_for_context(
        &self,
        _: &str,
    ) -> Result<Vec<nola_domain::generative_ui::UiSurfaceState>, String> {
        Ok(Vec::new())
    }
    fn begin_successor_turn(
        &mut self,
        _: &nola_store_port::run_creation::BeginSuccessorTurnInput,
    ) -> Result<nola_store_port::run_creation::BeginTurnResult, String> {
        Err("fixture has no committed NextTurn admission".into())
    }
    fn internal_assistant_target(
        &self,
        _: &str,
    ) -> Result<Option<nola_store_port::assistant_ownership::AssistantOwner>, String> {
        Err("no existing user trigger".into())
    }
    fn begin_turn(
        &mut self,
        input: &nola_store_port::run_creation::BeginTurnInput,
    ) -> Result<nola_store_port::run_creation::BeginTurnResult, String> {
        if self.fail_creation {
            return Err("creation unavailable".into());
        }
        if self.opened.is_some() {
            return Err("already opened".into());
        }
        let (mut run, _) = seed();
        run.checkpoint = StoredCheckpoint {
            conversation_id: input.conversation_id.clone(),
            revision: input.checkpoint_revision,
            event_cursor: input.checkpoint_event_cursor,
            checkpoint_json: input.checkpoint_json.clone(),
            updated_at_ms: input.created_at_ms,
        };
        let checkpoint: nola_agent_engine::agent::recovery::CoreCheckpoint<
            nola_agent_engine::agent::AgentThreadState,
        > = serde_json::from_str(&input.checkpoint_json).map_err(|e| e.to_string())?;
        run.core_turn_ids = checkpoint
            .state
            .execution
            .active_turn
            .iter()
            .map(|turn| turn.turn_id.as_str().to_owned())
            .collect();
        run.run.run_id = input.run_id.clone();
        run.run.assistant_message_id = Some(input.assistant_message_id.clone());
        run.run.assistant_part_id = Some(input.assistant_part_id.clone());
        run.run.invocation_config_digest = Some(input.invocation_config_digest.clone());
        run.config = StoredRunInvocationConfig {
            run_id: input.run_id.clone(),
            invocation_config_json: input.invocation_config_json.clone(),
            invocation_config_digest: input.invocation_config_digest.clone(),
        };
        run.effects = input
            .outbox_effects
            .iter()
            .map(|e| {
                Ok(StoredOutboxEffect {
                    effect_id: e.effect_id.clone(),
                    run_id: input.run_id.clone(),
                    request_id: e.request_id.clone(),
                    effect_kind: e.effect_kind.clone(),
                    payload_json: effect_payload(e, &input.run_id, None)?,
                    status: "pending".into(),
                    created_at_ms: input.created_at_ms,
                    acknowledged_at_ms: None,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        run.attempts.clear();
        apply_attempts(&mut run.attempts, &input.model_attempt_mutations)?;
        run.delegated_invocations.clear();
        apply_delegated_invocations(
            &mut run.delegated_invocations,
            &input.delegated_invocation_mutations,
            &input.run_id,
        )?;
        run.delegated_permissions.clear();
        apply_permissions(
            &mut run.delegated_permissions,
            &run.delegated_invocations,
            &input.delegated_permission_mutations,
            &input.run_id,
        )?;
        validate_delegated_deliveries(
            &run.delegated_invocations,
            &input.run_id,
            &input.outbox_effects,
        )?;
        run.run.next_event_sequence = 1
            + input.model_attempt_mutations.len() as i64
            + input.delegated_invocation_mutations.len() as i64;
        self.opened = Some(run);
        Ok(nola_store_port::run_creation::BeginTurnResult {
            conversation_id: input.conversation_id.clone(),
            user_message_id: input.user_message_id.clone(),
            assistant_message_id: input.assistant_message_id.clone(),
            run_id: input.run_id.clone(),
            user_ordinal: 1,
            assistant_ordinal: 2,
        })
    }
    fn begin_internal_turn(
        &mut self,
        _: &nola_store_port::run_creation::BeginInternalTurnInput,
    ) -> Result<nola_store_port::run_creation::BeginTurnResult, String> {
        Err("no existing user trigger".into())
    }
    fn begin_maintenance_run(
        &mut self,
        _: &nola_store_port::run_creation::BeginMaintenanceRunInput,
    ) -> Result<nola_store_port::run_creation::BeginMaintenanceRunResult, String> {
        Err("no maintenance fixture".into())
    }
}

#[test]
fn creation_uses_ports_and_failed_admission_can_retry_then_cold_restore() {
    let mut repo = MemoryCreation {
        fail_creation: true,
        ..Default::default()
    };
    let command = || nola_application::BeginAssistantTurn {
        expected_configuration_revision: None,
        expected_parent_message_id: None,
        conversation_id: "recovery-conversation".into(),
        conversation_title: "Port".into(),
        user_message_id: "user".into(),
        user_part_id: "user:text".into(),
        user_text: "hello".into(),
        user_content_json: None,
        assistant_message_id: "created:assistant".into(),
        context_checkpoint_id: None,
        core_config: runtime_config(),
        submission: AgentSubmission::UserInput {
            items: vec![AgentItem::user_message(
                ItemId::from_string("user".into()),
                "hello",
            )],
            turn_context_items: vec![],
            turn_context_platform: Default::default(),
            turn_context_components: vec![],
            submission_id: Some("created".into()),
            submitted_at_epoch_ms: Some(1000),
            delivery: AgentInputDelivery::CurrentTurn,
        },
        created_at_ms: 1000,
    };
    assert!(nola_application::begin_assistant_turn(&mut repo, command())
        .unwrap_err()
        .contains("creation unavailable"));
    assert!(repo.opened.is_none());
    repo.fail_creation = false;
    let accepted = nola_application::begin_assistant_turn(&mut repo, command()).unwrap();
    assert_eq!(
        repo.history_reads.get(),
        2,
        "production empty canonical context path used on both attempts"
    );
    let request = load_committed_host_requests(&repo, &accepted.begin.run_id)
        .unwrap()
        .requests
        .remove(0);
    assert_eq!(
        request.created_at_ms, 1000,
        "Application owns the decision observation time"
    );
    assert_eq!(request.deadline_ms, Some(601000));
    let mut independent = MemoryCreation::default();
    let second = nola_application::begin_assistant_turn(&mut independent, command()).unwrap();
    let repeated = load_committed_host_requests(&independent, &second.begin.run_id)
        .unwrap()
        .requests
        .remove(0);
    assert_ne!(
        request.attempt_id, repeated.attempt_id,
        "independent admission owns a distinct physical Attempt"
    );
    for (repository, envelope) in [(&repo, &request), (&independent, &repeated)] {
        let attempt = repository
            .model_attempt(envelope.attempt_id.as_deref().unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(attempt.effect_id, envelope.effect_id.0);
        assert_eq!(attempt.state, nola_domain::ModelAttemptState::Prepared);
    }
    let mut first_facts = serde_json::to_value(&request).unwrap();
    let mut second_facts = serde_json::to_value(&repeated).unwrap();
    first_facts.as_object_mut().unwrap().remove("attempt_id");
    second_facts.as_object_mut().unwrap().remove("attempt_id");
    assert_eq!(
        first_facts, second_facts,
        "same decision facts retain identical Effect content and timestamps"
    );
    let run_id = accepted.begin.run_id.clone();
    let digest = accepted.invocation_config_digest.clone();
    let request = claim_and_start(&mut repo, &run_id, "recovery-conversation", &digest, 1_100);
    let start = crate::command(
        repo.opened.as_ref().unwrap(),
        &request,
        2,
        HostResponse::ModelAttemptStarted {
            provider: "fixture".into(),
            model: "test-model".into(),
            reason: "initial".into(),
        },
    );
    reduce_and_accept_host_response(&mut repo, start).unwrap();
    assert_eq!(
        repo.model_attempt(request.attempt_id.as_deref().unwrap())
            .unwrap()
            .unwrap()
            .state,
        nola_domain::ModelAttemptState::Running
    );
    let run = repo.opened.as_ref().unwrap();
    let mut input = super_command_for_created(run, &request);
    input.response.sequence = 3;
    reduce_and_accept_host_response(&mut repo, input).unwrap();
    assert_eq!(repo.opened.as_ref().unwrap().commits.len(), 3);
    assert!(nola_application::begin_assistant_turn(&mut repo, command())
        .unwrap_err()
        .contains("overlap"));
}
fn super_command_for_created(
    run: &MemoryRun,
    request: &HostRequestEnvelope,
) -> ReduceAndAcceptHostResponse {
    command(
        run,
        request,
        1,
        HostResponse::ModelDelta {
            channel: ModelDeltaChannel::Text,
            part_id: run.run.assistant_part_id.clone().unwrap(),
            delta: "created via port".into(),
        },
    )
}

#[test]
fn cold_effect_replay_uses_explicit_time_and_rejects_unsupported_clock_atomically() {
    let (repo, request) = seed();
    let checkpoint = serde_json::from_str(&repo.checkpoint.checkpoint_json).unwrap();
    let mut first = AgentCore::restore(runtime_config(), checkpoint).unwrap();
    let checkpoint = serde_json::from_str(&repo.checkpoint.checkpoint_json).unwrap();
    let mut second = AgentCore::restore(runtime_config(), checkpoint).unwrap();
    let a = first
        .resume_committed_host_effect_at(&request, 1234)
        .unwrap();
    let b = second
        .resume_committed_host_effect_at(&request, 1234)
        .unwrap();
    assert_eq!(
        serde_json::to_value(&a.host_requests).unwrap(),
        serde_json::to_value(&b.host_requests).unwrap()
    );
    assert_eq!(a.host_requests[0].created_at_ms, 1234);
    assert_eq!(a.host_requests[0].deadline_ms, Some(601234));
    assert_eq!(
        serde_json::to_value(first.checkpoint(CheckpointReason::Manual)).unwrap(),
        serde_json::to_value(second.checkpoint(CheckpointReason::Manual)).unwrap()
    );

    let maximum_time =
        u64::try_from(chrono::DateTime::<chrono::Utc>::MAX_UTC.timestamp_millis()).unwrap();
    let checkpoint = serde_json::from_str(&repo.checkpoint.checkpoint_json).unwrap();
    let mut boundary = AgentCore::restore(runtime_config(), checkpoint).unwrap();
    let replay = boundary
        .resume_committed_host_effect_at(&request, maximum_time)
        .unwrap();
    assert_eq!(replay.host_requests[0].created_at_ms, maximum_time);
    assert_eq!(
        replay.host_requests[0].deadline_ms,
        Some(maximum_time + 600_000)
    );

    let checkpoint = serde_json::from_str(&repo.checkpoint.checkpoint_json).unwrap();
    let mut rejected = AgentCore::restore(runtime_config(), checkpoint).unwrap();
    let before = serde_json::to_value(rejected.checkpoint(CheckpointReason::Manual)).unwrap();
    for invalid_time in [maximum_time + 1, i64::MAX as u64, u64::MAX] {
        let error = rejected
            .resume_committed_host_effect_at(&request, invalid_time)
            .unwrap_err();
        assert!(
            error.contains("observation timestamp is outside the supported range"),
            "{error}"
        );
        assert_eq!(
            serde_json::to_value(rejected.checkpoint(CheckpointReason::Manual)).unwrap(),
            before,
            "clock rejection must not rebind the request, consume IDs or advance the journal"
        );
    }
    let after_rejection = rejected
        .resume_committed_host_effect_at(&request, 1234)
        .unwrap();
    assert_eq!(
        serde_json::to_value(after_rejection).unwrap(),
        serde_json::to_value(a).unwrap()
    );
    assert_eq!(
        serde_json::to_value(rejected.checkpoint(CheckpointReason::Manual)).unwrap(),
        serde_json::to_value(first.checkpoint(CheckpointReason::Manual)).unwrap()
    );
}

#[test]
fn delegated_lifecycle_is_atomic_and_cold_restorable_without_sqlite() {
    use nola_domain::DelegatedInvocationState as State;
    use nola_host_protocol::DelegatedAgentEventV1 as Event;
    let mut config = runtime_config();
    config.host_catalog.as_mut().unwrap().capabilities.insert(
        "agent.delegate".into(),
        runtime_capability("agent.delegate", HostCapabilityRoute::Model),
    );
    config.turn_identity.execution_backend =
        nola_domain::execution_backend::ExecutionBackendSelection {
            backend_id: "codex".into(),
            ..Default::default()
        };
    config.turn_identity.prompt.stable_prefix_digest = "a".repeat(64);
    let begin = nola_application::BeginAssistantTurn {
        expected_configuration_revision: None,
        expected_parent_message_id: None,
        conversation_id: "recovery-conversation".into(),
        conversation_title: "Delegated port".into(),
        user_message_id: "user".into(),
        user_part_id: "user:text".into(),
        user_text: "hello".into(),
        user_content_json: None,
        assistant_message_id: "delegated:assistant".into(),
        context_checkpoint_id: None,
        core_config: config,
        submission: AgentSubmission::UserInput {
            items: vec![AgentItem::user_message(
                ItemId::from_string("user".into()),
                "hello",
            )],
            turn_context_items: vec![],
            turn_context_platform: Default::default(),
            turn_context_components: vec![],
            submission_id: Some("delegated-run".into()),
            submitted_at_epoch_ms: Some(1000),
            delivery: AgentInputDelivery::CurrentTurn,
        },
        created_at_ms: 1000,
    };
    let mut repo = MemoryCreation {
        fail_creation: true,
        ..Default::default()
    };
    assert!(nola_application::begin_assistant_turn(&mut repo, begin.clone()).is_err());
    assert!(repo.opened.is_none());
    repo.fail_creation = false;
    let opened = nola_application::begin_assistant_turn(&mut repo, begin).unwrap();
    let request = load_committed_host_requests(&repo, &opened.begin.run_id)
        .unwrap()
        .requests
        .remove(0);
    assert!(matches!(
        request.payload,
        HostRequest::DelegatedAgent { .. }
    ));
    assert!(request.attempt_id.is_none());
    let id = request.delegated_invocation_id.as_deref().unwrap();
    let initial = repo.delegated_invocation(id).unwrap().unwrap();
    assert_ne!(initial.id, request.request_id);
    assert_eq!(initial.state, State::Prepared);
    assert!(repo.opened.as_ref().unwrap().attempts.is_empty());
    let request = claim_request(&mut repo, &opened.begin.run_id, 1_100);
    let accepted = command(
        repo.opened.as_ref().unwrap(),
        &request,
        1,
        HostResponse::ExecutionStarted,
    );
    repo.opened.as_mut().unwrap().fail_commit = true;
    assert!(reduce_and_accept_host_response(&mut repo, accepted.clone()).is_err());
    assert_eq!(repo.delegated_invocation(id).unwrap(), Some(initial));
    repo.opened.as_mut().unwrap().fail_commit = false;
    reduce_and_accept_host_response(&mut repo, accepted).unwrap();
    assert_eq!(
        repo.delegated_invocation(id).unwrap().unwrap().state,
        State::Running
    );
    let start = command(
        repo.opened.as_ref().unwrap(),
        &request,
        2,
        HostResponse::DelegatedAgentEvent {
            event: Event::SessionStarted {
                backend_id: "codex".into(),
                backend_session_id: "actual-session".into(),
            },
        },
    );
    reduce_and_accept_host_response(&mut repo, start).unwrap();
    let mut cold = repo.opened.as_ref().unwrap().clone();
    let restored = load_committed_host_requests(&cold, &opened.begin.run_id)
        .unwrap()
        .requests
        .remove(0);
    assert_eq!(
        restored.delegated_invocation_id,
        request.delegated_invocation_id
    );
    assert_eq!(restored.resume_after_sequence, Some(2));
    let session = cold.delegated_invocation(id).unwrap().unwrap();
    assert_eq!(
        session.backend_session_id.as_deref(),
        Some("actual-session")
    );
    // Permission admission has its own durable identity in a non-SQLite repository.
    let mut permission_branch = cold.clone();
    let permission_response = command(
        &permission_branch,
        &request,
        3,
        HostResponse::DelegatedAgentEvent {
            event: Event::PermissionRequested {
                occurrence: nola_host_protocol::DelegatedPermissionOccurrence {
                    occurrence_id: "port-occurrence".into(),
                    worker_generation: "port-worker".into(),
                },
                request_id: "port-rpc".into(),
                call_id: "port-call".into(),
                capability_id: "filesystem.write".into(),
                resource: "/fixture/permission.txt".into(),
                reason: "Write fixture".into(),
                input: serde_json::json!({"path":"/fixture/permission.txt"}),
            },
        },
    );
    permission_branch.fail_commit = true;
    assert!(
        reduce_and_accept_host_response(&mut permission_branch, permission_response.clone())
            .is_err()
    );
    assert!(permission_branch.delegated_permissions.is_empty());
    permission_branch.fail_commit = false;
    reduce_and_accept_host_response(&mut permission_branch, permission_response).unwrap();
    let awaiting = permission_branch
        .delegated_permission_for_occurrence(id, "port-occurrence")
        .unwrap()
        .unwrap();
    assert_eq!(
        awaiting.state,
        nola_domain::DelegatedPermissionDeliveryState::AwaitingDecision
    );
    assert_eq!(awaiting.worker_generation, "port-worker");
    assert_eq!(
        permission_branch
            .open_delegated_permissions_for_target(&request.effect_id.0)
            .unwrap(),
        vec![awaiting.clone()]
    );
    let mut permission_cold = permission_branch.clone();
    nola_application::reduce_and_commit_active_submission(
        &mut permission_cold,
        nola_application::ReduceAndCommitActiveSubmission {
            conversation_id: permission_branch.checkpoint.conversation_id.clone(),
            run_id: permission_branch.run.run_id.clone(),
            transition_id: "permission-cancel".into(),
            invocation_config_digest: permission_branch.config.invocation_config_digest.clone(),
            submission: AgentSubmission::Interrupt {
                goal_control: None, cancellation_id: "cancel-before-decision".into(),
                reason: Some("cancel before decision".into()),
            },
            committed_at_ms: 2001,
        },
    )
    .unwrap();
    assert_eq!(
        permission_cold
            .delegated_permission(&awaiting.id)
            .unwrap()
            .unwrap()
            .state,
        nola_domain::DelegatedPermissionDeliveryState::Interrupted
    );
    assert!(permission_cold
        .open_delegated_permissions_for_target(&request.effect_id.0)
        .unwrap()
        .is_empty());
    // A separate cold branch exercises cancellation of the same admitted operation.
    let mut cancelled = cold.clone();
    let cancel_command = nola_application::ReduceAndCommitActiveSubmission {
        conversation_id: cancelled.checkpoint.conversation_id.clone(),
        run_id: cancelled.run.run_id.clone(),
        transition_id: "delegated-cancel".into(),
        invocation_config_digest: cancelled.config.invocation_config_digest.clone(),
        submission: AgentSubmission::Interrupt {
            goal_control: None, cancellation_id: "user-cancelled".into(),
            reason: Some("user cancelled".into()),
        },
        committed_at_ms: 2001,
    };
    nola_application::reduce_and_commit_active_submission(&mut cancelled, cancel_command).unwrap();
    let cancel_request = load_committed_host_requests(&cancelled, &opened.begin.run_id)
        .unwrap()
        .requests
        .into_iter()
        .find(|r| matches!(r.payload, HostRequest::Cancel { .. }))
        .expect("durable delegated cancellation");
    let mut target_first = cancelled.clone();
    let target_done = command(
        &target_first,
        &request,
        3,
        HostResponse::DelegatedAgentEvent {
            event: Event::CancellationConfirmed,
        },
    );
    let cancelling = target_first.delegated_invocation(id).unwrap().unwrap();
    assert_eq!(cancelling.state, State::Cancelling);
    assert!(cancelling.completed_at_ms.is_none());
    let before_late_target = target_first.checkpoint.clone();
    let cold_checkpoint: nola_agent_engine::agent::recovery::CoreCheckpoint<nola_agent_engine::agent::AgentThreadState> =
        serde_json::from_str(&before_late_target.checkpoint_json).unwrap();
    let cancelling_target = cold_checkpoint.state.host.pending_effects.get(&request.effect_id.0)
        .expect("cancellation retains its exact target until the Cancel effect settles");
    assert_eq!(cancelling_target.state, nola_agent_engine::agent::recovery::PendingHostEffectState::Cancelling);
    assert_eq!(cancelling_target.cancel_effect_id.as_deref(), Some(cancel_request.effect_id.0.as_str()));
    assert_retired_target_noop(&mut target_first, target_done.clone());
    assert_retired_target_noop(&mut target_first, target_done.clone());
    assert_eq!(target_first.checkpoint, before_late_target);
    assert_eq!(
        target_first.delegated_invocation(id).unwrap(),
        Some(cancelling)
    );
    for damage in ["completed", "foreign_invocation", "foreign_request", "foreign_host",
        "foreign_session", "foreign_turn", "foreign_catalog", "foreign_delivery", "model_attempt"] {
        let before = target_first.clone();
        let mut forged = target_done.clone();
        match damage {
            "completed" => forged.response.payload = HostResponse::DelegatedAgentEvent {
                event: Event::Completed {
                    items: vec![serde_json::to_value(AgentItem::assistant_message(
                        ItemId::from_string(session.text_part_id.clone()), "MUST_NOT_PUBLISH")).unwrap()],
                    usage: serde_json::json!({}),
                },
            },
            "foreign_invocation" => forged.response.delegated_invocation_id = Some("foreign".into()),
            "foreign_request" => forged.response.request_id = "foreign".into(),
            "foreign_host" => forged.response.host_id = HostId("foreign".into()),
            "foreign_session" => forged.response.session_id = "foreign".into(),
            "foreign_turn" => forged.response.turn_id = Some("foreign".into()),
            "foreign_catalog" => forged.response.catalog_revision += 1,
            "foreign_delivery" => forged.response.delivery.as_mut().unwrap().lease_token.push_str("-foreign"),
            "model_attempt" => forged.response.attempt_id = Some("foreign-model-attempt".into()),
            _ => unreachable!(),
        }
        assert!(reduce_and_accept_host_response(&mut target_first, forged).is_err(), "{damage}");
        assert_eq!(target_first, before, "{damage} changed canonical state");
    }
    let cleanup = claim_and_start(
        &mut target_first,
        &opened.begin.run_id,
        "recovery-conversation",
        &opened.invocation_config_digest,
        2_100,
    );
    let target = match &cleanup.payload {
        HostRequest::Cancel { target_effect_id, .. } => target_effect_id.clone(),
        _ => panic!("expected cancellation control"),
    };
    let mut done = command(
        &target_first,
        &cleanup,
        2,
        HostResponse::CancellationConfirmed {
            target_effect_id: target,
        },
    );
    done.received_at_ms = 2_200;
    let before_terminal = target_first.clone();
    assert!(reduce_and_accept_host_response(&mut target_first, done).unwrap_err()
        .contains("terminal result sealing requires the real SQLite fixture"));
    assert_eq!(target_first, before_terminal);
    let stopped = target_first.delegated_invocation(id).unwrap().unwrap();
    assert_eq!(stopped.state, State::Cancelling);
    assert!(stopped.completed_at_ms.is_none());
    let after_stop = target_first.checkpoint.clone();
    assert_retired_target_noop(&mut target_first, target_done);
    assert_eq!(target_first.checkpoint, after_stop);
    assert_eq!(
        target_first.delegated_invocation(id).unwrap(),
        Some(stopped)
    );
    assert!(!load_committed_host_requests(&target_first, &opened.begin.run_id).unwrap().requests.is_empty());
    let target_effect_id = match &cancel_request.payload {
        HostRequest::Cancel { target_effect_id, .. } => target_effect_id.clone(),
        _ => unreachable!(),
    };
    let cancel_request = claim_and_start(
        &mut cancelled,
        &opened.begin.run_id,
        "recovery-conversation",
        &opened.invocation_config_digest,
        2_100,
    );
    let mut confirmed = command(
        &cancelled,
        &cancel_request,
        2,
        HostResponse::CancellationConfirmed { target_effect_id },
    );
    confirmed.received_at_ms = 2_200;
    let before_terminal = cancelled.clone();
    assert!(reduce_and_accept_host_response(&mut cancelled, confirmed).unwrap_err()
        .contains("terminal result sealing requires the real SQLite fixture"));
    assert_eq!(cancelled, before_terminal);
    assert_eq!(
        cancelled.delegated_invocation(id).unwrap().unwrap().state,
        State::Cancelling
    );
    assert!(!load_committed_host_requests(&cancelled, &opened.begin.run_id).unwrap().requests.is_empty());
    let rebind = command(
        &cold,
        &request,
        3,
        HostResponse::DelegatedAgentEvent {
            event: Event::SessionStarted {
                backend_id: "codex".into(),
                backend_session_id: "replacement-session".into(),
            },
        },
    );
    let before = cold.checkpoint.clone();
    assert!(reduce_and_accept_host_response(&mut cold, rebind)
        .unwrap_err()
        .contains("session identity changed"));
    assert_eq!(cold.checkpoint, before);
    assert_eq!(
        cold.delegated_invocation(id).unwrap(),
        Some(session.clone())
    );
    let finish = command(
        &cold,
        &request,
        3,
        HostResponse::DelegatedAgentEvent {
            event: Event::Completed {
                items: vec![serde_json::to_value(AgentItem::assistant_message(
                    ItemId::from_string(session.text_part_id.clone()),
                    "delegated answer",
                ))
                .unwrap()],
                usage: serde_json::json!({}),
            },
        },
    );
    let before_terminal = cold.checkpoint.clone();
    assert!(reduce_and_accept_host_response(&mut cold, finish).unwrap_err()
        .contains("terminal result sealing requires the real SQLite fixture"));
    assert_eq!(cold.checkpoint, before_terminal);
    let finished = cold.delegated_invocation(id).unwrap().unwrap();
    assert_eq!(finished.state, session.state);
    assert_eq!(
        finished.backend_session_id.as_deref(),
        Some("actual-session")
    );
    let remaining = load_committed_host_requests(&cold, &opened.begin.run_id)
        .unwrap()
        .requests;
    assert!(!remaining.is_empty(), "uncommitted terminal cannot acknowledge its Host request");
    assert!(cold.attempts.is_empty());
}


#[path = "support/run_effect_admission_models.rs"]
mod admission_models;
impl nola_store_port::goal_execution::GoalExecutionRepository for MemoryCreation {
    type Error = String;
    fn load_goal_execution_binding(&self, conversation: &str, goal: &str) -> Result<Option<nola_store_port::goal_execution::StoredGoalExecutionBinding>,String> {
        Ok(self.goal_bindings.get(&(conversation.into(),goal.into())).cloned())
    }
    fn insert_goal_execution_binding(&mut self, binding: &nola_store_port::goal_execution::StoredGoalExecutionBinding) -> Result<(),String> {
        if self.transaction_backup.is_none() { return Err("Goal binding outside transaction".into()); }
        let key=(binding.identity.conversation_id.clone(),binding.identity.goal_id.clone());
        if let Some(existing)=self.goal_bindings.get(&key) {
            if existing != binding { return Err("Goal execution binding is immutable".into()); }
        } else { self.goal_bindings.insert(key,binding.clone()); }
        Ok(())
    }
}
impl nola_store_port::model::ModelRepositoryProvider for MemoryCreation {
    type Error = nola_store::StoreError;
    type Repository<'a> = admission_models::Models;
    fn model_repository(&mut self) -> Self::Repository<'_> { self.models.clone() }
}
