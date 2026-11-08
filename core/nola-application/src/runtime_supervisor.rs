// SPDX-License-Identifier: Apache-2.0
//! Ownership and lifecycle for hot Agent Core executions.
//!
//! The durable Store remains authoritative. This supervisor only retains a
//! bounded in-memory acceleration overlay while the matching checkpoint
//! revision is still current.

use std::collections::HashMap;

use nola_agent_engine::agent::host::HostRequestEnvelope;
use nola_agent_engine::agent::protocol::AgentEvent;
use nola_domain::core_event::CoreEventEnvelope;
use nola_agent_engine::agent::AgentCore;

use crate::CommittedHostRequests;

fn overlay_request_cursor(
    committed: &mut HostRequestEnvelope,
    mut hot: nola_domain::host_control::HostEffectControl,
) -> Result<(), String> {
    // This is an explicit dispatch materialization, not a streamed reduction.
    // Validate the newly loaded body once against the hot immutable identity.
    let committed_control = nola_domain::host_control::HostEffectControl::from_request(committed)?;
    let cursor = committed
        .resume_after_sequence
        .max(hot.resume_after_sequence);
    // Only these fields are supplied independently by the attempt ledger or
    // hot reducer. Normalize the owned temporary, not clones of both bodies.
    hot.resume_after_sequence = committed.resume_after_sequence;
    hot.attempt_id.clone_from(&committed.attempt_id);
    hot.attempt_started = committed.attempt_started;
    // The durable effect declaration is not a delivery claim. A reducer may
    // retain the lease of an already validated response, whereas this read
    // returns an unclaimed declaration (or an independently claimed successor).
    // Never copy that hot lease into the returned request. Delivery authority
    // is established separately by the delivery ledger at claim/response time;
    // a per-effect response cursor cannot grant, renew or replace that lease.
    hot.delivery.clone_from(&committed.delivery);
    // Cancellation admission enriches the durable outbox with a revocation
    // proof after the reducer creates its immutable control. Reuse the Core
    // binding validator: only the same control may acquire that proof, and an
    // existing proof cannot be replaced. All other identity fields remain hot
    // until the strict comparison below.
    let mut admitted_binding = hot.binding();
    admitted_binding.payload_binding = committed_control.binding().payload_binding;
    hot.apply_binding(&admitted_binding)?;
    if hot != committed_control {
        // Field names identify a broken binding without exposing tool inputs or leases.
        let hot_fields = serde_json::to_value(&hot).map_err(|error| error.to_string())?;
        let committed_fields = serde_json::to_value(&committed_control).map_err(|error| error.to_string())?;
        let changed = hot_fields.as_object().ok_or("invalid hot control")?.iter()
            .filter(|(key, value)| committed_fields.get(*key) != Some(*value))
            .map(|(key, _)| key.as_str()).collect::<Vec<_>>().join(", ");
        return Err(format!("hot cursor belongs to a different committed Host delivery (fields: {changed})"));
    }
    committed.resume_after_sequence = cursor;
    Ok(())
}

pub(crate) struct RuntimeSupervisor {
    active_runs: HashMap<String, HotActiveRun>,
    // Keep only the most recent terminal execution view, never its Core or
    // transcript. This bridges final delivery to Host adoption after commit.
    terminal_execution: Option<(String, String, i64, crate::HostExecutionState)>,
    unit_of_work: Option<RuntimeUnitOfWork>,
    poison_reason: Option<String>,
    rollback_capsules: Vec<HotRollbackCapsule>,
}

/// Only bounded input facts are copied on the hot path, never the transcript
/// or accumulated PartUpdated snapshots. Rebuild is paid only on rejection.
pub(crate) struct HotRollbackCapsule {
    pub(crate) run_id: String,
    pub(crate) conversation_id: String,
    pub(crate) checkpoint_revision: i64,
    pub(crate) next_event_sequence: i64,
    pub(crate) invocation_config_digest: String,
    pub(crate) core_revision: u64,
    pub(crate) event_cursor: u64,
    pub(crate) deferred_host_events: Vec<nola_store_port::run_effect::HostEventInput>,
    pub(crate) deferred_traces: Vec<nola_agent_engine::agent_trace::AgentTraceEvent>,
    pub(crate) deferred_bytes: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RuntimeUnitOfWorkPurpose {
    ReceiptAck,
    DurableFlush,
}

struct RuntimeUnitOfWork {
    purpose: RuntimeUnitOfWorkPurpose,
    retained: Vec<RetainHotRun>,
}

pub(crate) struct HotActiveRun {
    pub(crate) conversation_id: String,
    pub(crate) core: AgentCore,
    pub(crate) checkpoint_revision: i64,
    pub(crate) next_event_sequence: i64,
    pub(crate) invocation_config_digest: String,
    pub(crate) deferred_events: Vec<CoreEventEnvelope<AgentEvent>>,
    pub(crate) deferred_traces: Vec<nola_agent_engine::agent_trace::AgentTraceEvent>,
    pub(crate) deferred_host_events: Vec<nola_store_port::run_effect::HostEventInput>,
    pub(crate) deferred_bytes: usize,
}

pub(crate) struct RetainHotRun {
    pub(crate) run_id: String,
    pub(crate) conversation_id: String,
    pub(crate) core: AgentCore,
    pub(crate) terminal: bool,
    pub(crate) checkpoint_revision: i64,
    pub(crate) next_event_sequence: i64,
    pub(crate) invocation_config_digest: String,
    pub(crate) deferred_events: Vec<CoreEventEnvelope<AgentEvent>>,
    pub(crate) deferred_traces: Vec<nola_agent_engine::agent_trace::AgentTraceEvent>,
    pub(crate) deferred_host_events: Vec<nola_store_port::run_effect::HostEventInput>,
    pub(crate) deferred_bytes: usize,
}

impl RuntimeSupervisor {
    pub(crate) fn new() -> Self {
        Self {
            active_runs: HashMap::new(),
            terminal_execution: None,
            unit_of_work: None,
            poison_reason: None,
            rollback_capsules: Vec::new(),
        }
    }

    pub(crate) fn ensure_usable(&self) -> Result<(), String> {
        match &self.poison_reason {
            Some(reason) => Err(format!(
                "Runtime Supervisor is unusable; reopen required: {reason}"
            )),
            None => Ok(()),
        }
    }

    /// A failed commit acknowledgement cannot establish which checkpoint is
    /// canonical. Discard every overlay and require a fresh Store/runtime owner.
    pub(crate) fn poison(&mut self, reason: String) {
        self.poison_reason.get_or_insert(reason);
        self.active_runs.clear();
        self.terminal_execution = None;
        self.unit_of_work = None;
        self.rollback_capsules.clear();
    }

    pub(crate) fn begin_unit_of_work(&mut self) -> Result<(), String> {
        self.begin_unit_of_work_with_purpose(RuntimeUnitOfWorkPurpose::ReceiptAck)
    }

    pub(crate) fn begin_durable_flush(&mut self) -> Result<(), String> {
        self.begin_unit_of_work_with_purpose(RuntimeUnitOfWorkPurpose::DurableFlush)
    }

    fn begin_unit_of_work_with_purpose(
        &mut self,
        purpose: RuntimeUnitOfWorkPurpose,
    ) -> Result<(), String> {
        self.ensure_usable()?;
        if self.unit_of_work.is_some() {
            return Err("Runtime Supervisor unit of work is already active".to_string());
        }
        self.unit_of_work = Some(RuntimeUnitOfWork {
            purpose,
            retained: Vec::new(),
        });
        Ok(())
    }

    pub(crate) fn commit_unit_of_work(&mut self) -> Result<(), String> {
        self.ensure_usable()?;
        let Some(staged) = self.unit_of_work.take() else {
            let reason = "Runtime Supervisor unit of work is not active".to_string();
            self.poison(reason.clone());
            return Err(reason);
        };
        for run in staged.retained {
            self.apply_retain(run);
        }
        self.rollback_capsules.clear();
        Ok(())
    }

    pub(crate) fn rollback_unit_of_work(&mut self) {
        self.unit_of_work = None;
    }

    pub(crate) fn commit_hot_command(&mut self) {
        self.rollback_capsules.clear();
    }

    pub(crate) fn take_rollback_capsules(&mut self) -> Vec<HotRollbackCapsule> {
        std::mem::take(&mut self.rollback_capsules)
    }

    pub(crate) fn has_unit_of_work(&self) -> bool {
        self.unit_of_work.is_some()
    }

    pub(crate) fn requires_durable_ack(&self) -> bool {
        self.unit_of_work
            .as_ref()
            .is_some_and(|staged| staged.purpose == RuntimeUnitOfWorkPurpose::ReceiptAck)
    }

    pub(crate) fn deferred_bytes(
        &self,
        run_id: &str,
        conversation_id: &str,
        checkpoint_revision: Option<i64>,
    ) -> usize {
        self.active_runs
            .get(run_id)
            .filter(|hot| {
                hot.conversation_id == conversation_id
                    && checkpoint_revision == Some(hot.checkpoint_revision)
            })
            .map_or(0, |hot| hot.deferred_bytes)
    }

    /// Application supplies the revision observed in its command transaction.
    /// The hot overlay neither opens persistence nor chooses a read snapshot.
    pub(crate) fn current_control(
        &self,
        run_id: &str,
        conversation_id: &str,
        checkpoint_revision: Option<i64>,
    ) -> Option<nola_agent_engine::agent::protocol::AgentControlSnapshot> {
        if self.poison_reason.is_some() { return None; }
        self.active_runs.get(run_id).filter(|hot| {
            hot.conversation_id == conversation_id
                && checkpoint_revision == Some(hot.checkpoint_revision)
        }).map(|hot| hot.core.control_snapshot())
    }

    /// Export permissions only when the retained Core has no unpublished tail.
    pub(crate) fn current_permissions(
        &self,
        run_id: &str,
        conversation_id: &str,
        checkpoint_revision: Option<i64>,
    ) -> Result<Option<nola_store_port::conversation_query::CurrentPermissionState>, String> {
        if self.poison_reason.is_some() { return Ok(None); }
        let Some(hot) = self.active_runs.get(run_id).filter(|hot| {
            hot.conversation_id == conversation_id
                && checkpoint_revision == Some(hot.checkpoint_revision)
                && hot.deferred_events.is_empty() && hot.deferred_host_events.is_empty() && hot.deferred_traces.is_empty()
        }) else {
            let Some((_, _, revision, state)) = self.terminal_execution.as_ref().filter(|(id, run, revision, _)|
                id == conversation_id && checkpoint_revision == Some(*revision)
                    && run == run_id) else { return Ok(None); };
            return Ok(Some(nola_store_port::conversation_query::CurrentPermissionState {
                active_permission_policy: state.control.active_permission_policy.clone(),
                conversation_id: conversation_id.to_owned(), revision: *revision,
                event_cursor: i64::try_from(state.control.last_event_cursor.sequence).map_err(|e| e.to_string())?,
                session_id: state.control.session_id.as_str().to_owned(),
                state_session_id: state.control.session_id.as_str().to_owned(),
                active_turn_id: state.control.active_turn_id.as_ref().map(|id| id.as_str().to_owned()),
                permissions: serde_json::json!({"revision": state.permissions.revision, "pending": state.permissions.pending}),
            }));
        };
        let checkpoint = hot.core.checkpoint_view(crate::CheckpointReason::Manual);
        Ok(Some(nola_store_port::conversation_query::CurrentPermissionState {
            active_permission_policy: checkpoint.state.execution.active_permission_policy.clone(),
            conversation_id: conversation_id.to_owned(),
            revision: hot.checkpoint_revision,
            event_cursor: i64::try_from(checkpoint.last_event_cursor.sequence).map_err(|e| e.to_string())?,
            session_id: checkpoint.session_id.clone(),
            state_session_id: checkpoint.state.session_id.as_str().to_owned(),
            active_turn_id: checkpoint.state.execution.active_turn.as_ref().map(|turn| turn.turn_id.0.clone()),
            permissions: serde_json::json!({
                "revision": checkpoint.state.execution.permissions.revision,
                "pending": checkpoint.state.execution.permissions.pending,
            }),
        }))
    }

    /// Take only the state belonging to the caller's exact Store snapshot.
    pub(crate) fn take_current(
        &mut self,
        run_id: &str,
        conversation_id: &str,
        checkpoint_revision: Option<i64>,
    ) -> Option<HotActiveRun> {
        if self.poison_reason.is_some() {
            return None;
        }
        let hot = self.active_runs.get(run_id)?;
        if hot.conversation_id != conversation_id
            || checkpoint_revision != Some(hot.checkpoint_revision) {
            return None;
        }
        if !self.rollback_capsules.iter().any(|capsule| capsule.run_id == run_id) {
            let checkpoint = hot.core.checkpoint_view(crate::CheckpointReason::Manual);
            self.rollback_capsules.push(HotRollbackCapsule {
                run_id: run_id.to_owned(), conversation_id: hot.conversation_id.clone(),
                checkpoint_revision: hot.checkpoint_revision,
                next_event_sequence: hot.next_event_sequence,
                invocation_config_digest: hot.invocation_config_digest.clone(),
                core_revision: checkpoint.revision,
                event_cursor: checkpoint.last_event_cursor.sequence,
                deferred_host_events: hot.deferred_host_events.clone(),
                deferred_traces: hot.deferred_traces.clone(),
                deferred_bytes: hot.deferred_bytes,
            });
        }
        self.active_runs.remove(run_id)
    }

    pub(crate) fn retain(&mut self, retained: RetainHotRun) {
        if self.poison_reason.is_some() {
            return;
        }
        if let Some(staged) = self.unit_of_work.as_mut() {
            staged.retained.push(retained);
            return;
        }
        self.apply_retain(retained);
    }

    /// A recovered baseline contains only already committed facts. Adopt it
    /// before a command mutates Core so that the normal take/rollback capsule
    /// owns subsequent speculative changes in this unit of work.
    pub(crate) fn retain_recovered(&mut self, retained: RetainHotRun) {
        debug_assert!(retained.deferred_events.is_empty() && retained.deferred_host_events.is_empty()
            && retained.deferred_traces.is_empty());
        if self.poison_reason.is_none() { self.apply_retain(retained); }
    }

    pub(crate) fn cancellation_checkpoint(&self, run: &str, conversation: &str, revision: i64)
        -> Option<crate::cancellation_delivery::CancellationCheckpoint> {
        if self.poison_reason.is_some() { return None; }
        let hot = self.active_runs.get(run).filter(|hot| hot.conversation_id == conversation
            && hot.checkpoint_revision == revision && hot.deferred_events.is_empty()
            && hot.deferred_host_events.is_empty() && hot.deferred_traces.is_empty())?;
        Some(crate::cancellation_delivery::CancellationCheckpoint {
            conversation_id: conversation.into(), revision,
            invocation_config_digest: hot.invocation_config_digest.clone(),
            checkpoint: hot.core.checkpoint(crate::CheckpointReason::Manual),
        })
    }

    fn apply_retain(&mut self, retained: RetainHotRun) {
        if self.terminal_execution.as_ref().is_some_and(|(id, _, _, _)| id == &retained.conversation_id) {
            self.terminal_execution = None;
        }
        if retained.terminal {
            if retained.deferred_events.is_empty() && retained.deferred_host_events.is_empty() && retained.deferred_traces.is_empty() {
                self.terminal_execution = Some((retained.conversation_id.clone(), retained.run_id.clone(), retained.checkpoint_revision,
                    retained.core.host_execution_state()));
            }
            self.active_runs.remove(&retained.run_id);
            self.active_runs
                .retain(|_, run| run.conversation_id != retained.conversation_id);
            return;
        }
        self.active_runs.retain(|existing_run_id, run| {
            existing_run_id == &retained.run_id || run.conversation_id != retained.conversation_id
        });
        self.active_runs.insert(
            retained.run_id,
            HotActiveRun {
                conversation_id: retained.conversation_id,
                core: retained.core,
                checkpoint_revision: retained.checkpoint_revision,
                next_event_sequence: retained.next_event_sequence,
                invocation_config_digest: retained.invocation_config_digest,
                deferred_events: retained.deferred_events,
                deferred_traces: retained.deferred_traces,
                deferred_host_events: retained.deferred_host_events,
                deferred_bytes: retained.deferred_bytes,
            },
        );
    }

    /// A narrow view of the same retained Core, never a separate Goal cache.
    /// A pending tail must be materialized by the canonical read path instead.
    pub(crate) fn current_goal(&self, conversation_id: &str, revision: i64)
        -> Option<nola_agent_engine::agent::goal::GoalProjection> {
        if self.poison_reason.is_some() { return None; }
        let mut matching = self.active_runs.values().filter(|hot|
            hot.conversation_id == conversation_id && hot.checkpoint_revision == revision
                && hot.deferred_events.is_empty() && hot.deferred_host_events.is_empty()
                && hot.deferred_traces.is_empty());
        let hot = matching.next()?;
        if matching.next().is_some() { return None; }
        Some(hot.core.checkpoint_view(crate::CheckpointReason::Manual).state.goal_projection.clone())
    }

    pub(crate) fn execution_state(&self, conversation_id: &str, revision: i64) -> Option<crate::HostExecutionState> {
        if self.poison_reason.is_some() { return None; }
        if let Some(hot) = self.active_runs.values().find(|hot| hot.conversation_id == conversation_id
            && hot.checkpoint_revision == revision && hot.deferred_events.is_empty()
            && hot.deferred_host_events.is_empty() && hot.deferred_traces.is_empty()) {
            return Some(hot.core.host_execution_state());
        }
        self.terminal_execution.as_ref().filter(|(id, _, current, _)| id == conversation_id && *current == revision)
            .map(|(_, _, _, state)| state.clone())
    }

    pub(crate) fn overlay_resume_cursors<R: nola_store_port::run_effect::RunEffectRepository + ?Sized>(
        &self,
        store: &R,
        run_id: &str,
        committed: &mut CommittedHostRequests,
    ) -> Result<(), String> {
        self.ensure_usable()?;
        let Some(hot) = self.active_runs.get(run_id) else {
            return Ok(());
        };
        for request in &mut committed.requests {
            if let Some(mut hot_request) = hot
                .core
                .committed_host_effect_control(&request.effect_id.0)?
            {
                // Delegated invocation IDs are attached by Application at the
                // durable declaration boundary, after Core creates its control.
                // Bind through the same exact ledger validator used for response
                // admission before comparing the remaining immutable identity.
                crate::delegated_invocation::bind_response_control(store, run_id, &mut hot_request)?;
                // Application owns Attempt admission; Core owns only the hot
                // response cursor. Never overwrite ledger-owned delivery facts
                // with the reducer's retained pre-admission request.
                overlay_request_cursor(request, hot_request)?;
            }
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn active_run_count(&self) -> usize {
        self.active_runs.len()
    }

    #[cfg(test)]
    pub(crate) fn contains_run(&self, run_id: &str) -> bool {
        self.active_runs.contains_key(run_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nola_agent_engine::agent::AgentCoreConfig;

    #[test]
    fn delegated_cursor_requires_bound_invocation_and_still_rejects_foreign_identity() {
        let mut request: HostRequestEnvelope = serde_json::from_value(serde_json::json!({
            "effect_id":"effect", "request_id":"request", "session_id":"session",
            "turn_id":"turn", "host_id":"host", "lease_id":null,
            "catalog_revision":1,"availability_epoch":1,"created_at_ms":1,"deadline_ms":null,
            "payload":{"type":"delegated_agent","request":{
                "contract_version":1,"backend_id":"codex","text_part_id":"text","reasoning_part_id":"reasoning",
                "context":{"mode":"bootstrap","target_anchor_item_id":"user","identity_digest":"a".repeat(64),
                    "messages":[{"role":"user","content":"Inspect"}]},"tools":[],"settings":{}}}
        })).unwrap();
        let mut hot = nola_domain::host_control::HostEffectControl::from_request(&request).unwrap();
        request.delegated_invocation_id = Some("canonical-invocation".into());
        let before = request.clone();
        assert!(overlay_request_cursor(&mut request, hot.clone()).is_err());
        assert_eq!(request, before);
        // This exact binding is supplied only after the Application validates
        // the durable delegated invocation ledger for this effect and Run.
        hot.apply_binding(&nola_domain::host_control::CoreHostBindingOverlay::from_request(&request)).unwrap();
        hot.resume_after_sequence = Some(3);
        overlay_request_cursor(&mut request, hot.clone()).unwrap();
        assert_eq!(request.resume_after_sequence, Some(3));
        assert_eq!(request.delegated_invocation_id.as_deref(), Some("canonical-invocation"));
        let before = request.clone();
        hot.delegated_invocation_id = Some("foreign-invocation".into());
        assert!(hot.apply_binding(&nola_domain::host_control::CoreHostBindingOverlay::from_request(&request)).is_err());
        assert!(overlay_request_cursor(&mut request, hot).is_err());
        assert_eq!(request, before);
    }

    #[test]
    fn cursor_overlay_preserves_ledger_facts_and_rejects_changed_request_without_mutation() {
        let raw = serde_json::json!({
            "effect_id":"effect", "request_id":"request", "session_id":"session",
            "turn_id":"turn", "host_id":"host", "lease_id":"lease",
            "catalog_revision":1, "availability_epoch":2, "created_at_ms":1,
            "deadline_ms":null, "resume_after_sequence":4,
            "attempt_id":"committed-attempt", "attempt_started":true,
            "payload":{"type":"cancel", "target_effect_id":"target","target_not_started":null}
        });
        let mut committed: HostRequestEnvelope = serde_json::from_value(raw.clone()).unwrap();
        let mut hot = committed.clone();
        hot.attempt_id = None;
        hot.attempt_started = false;
        hot.resume_after_sequence = Some(8);
        overlay_request_cursor(
            &mut committed,
            nola_domain::host_control::HostEffectControl::from_request(&hot).unwrap(),
        )
        .unwrap();
        assert_eq!(committed.resume_after_sequence, Some(8));
        assert_eq!(committed.attempt_id.as_deref(), Some("committed-attempt"));
        assert!(committed.attempt_started);
        for (field, foreign) in [
            ("session_id", serde_json::json!("foreign")),
            ("request_id", serde_json::json!("foreign")),
            ("lease_id", serde_json::Value::Null),
            ("availability_epoch", serde_json::json!(3)),
            (
                "payload",
                serde_json::json!({"type":"cancel", "target_effect_id":"foreign","target_not_started":null}),
            ),
        ] {
            let before = committed.clone();
            let mut changed = raw.clone();
            changed[field] = foreign;
            changed["resume_after_sequence"] = serde_json::json!(100);
            assert!(
                overlay_request_cursor(
                    &mut committed,
                    nola_domain::host_control::HostEffectControl::from_request(
                        &serde_json::from_value::<HostRequestEnvelope>(changed).unwrap()
                    )
                    .unwrap()
                )
                .is_err(),
                "{field}"
            );
            assert_eq!(committed, before, "{field}");
        }
        let stale: HostRequestEnvelope = serde_json::from_value(raw).unwrap();
        overlay_request_cursor(
            &mut committed,
            nola_domain::host_control::HostEffectControl::from_request(&stale).unwrap(),
        )
        .unwrap();
        assert_eq!(committed.resume_after_sequence, Some(8));
    }

    #[test]
    fn cursor_overlay_adopts_exact_durable_cancellation_proof_only() {
        let raw = serde_json::json!({
            "effect_id":"cancel", "request_id":"cancel-request", "session_id":"session",
            "turn_id":"run", "host_id":"host", "lease_id":null,
            "catalog_revision":1, "availability_epoch":2, "created_at_ms":1,
            "deadline_ms":null, "resume_after_sequence":null,
            "payload":{"type":"cancel", "target_effect_id":"target","target_not_started":null}
        });
        let hot_request: HostRequestEnvelope = serde_json::from_value(raw.clone()).unwrap();
        let hot = nola_domain::host_control::HostEffectControl::from_request(&hot_request).unwrap();
        let mut admitted = raw;
        admitted["payload"]["target_not_started"] = serde_json::json!({
            "conversation_id":"session", "run_id":"run", "core_session_id":"session",
            "host_id":"host", "cancellation_effect_id":"cancel", "cancellation_request_id":"cancel-request",
            "target_effect_id":"target", "target_request_id":"target-request",
            "target_payload_identity":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "checkpoint_revision":4,
            "revoked_attempts":[{"attempt_id":"attempt","revision":2}]
        });
        let mut committed: HostRequestEnvelope = serde_json::from_value(admitted.clone()).unwrap();
        overlay_request_cursor(&mut committed, hot.clone()).unwrap();
        let admitted_hot = nola_domain::host_control::HostEffectControl::from_request(&committed).unwrap();
        let mut replacement = admitted.clone();
        replacement["payload"]["target_not_started"]["checkpoint_revision"] = serde_json::json!(5);
        let mut replacement: HostRequestEnvelope = serde_json::from_value(replacement).unwrap();
        let before = replacement.clone();
        assert!(overlay_request_cursor(&mut replacement, admitted_hot).is_err());
        assert_eq!(replacement, before);
        admitted["payload"]["target_not_started"]["target_effect_id"] = serde_json::json!("foreign");
        let mut foreign: HostRequestEnvelope = serde_json::from_value(admitted).unwrap();
        let before = foreign.clone();
        assert!(overlay_request_cursor(&mut foreign, hot).is_err());
        assert_eq!(foreign, before);
    }

    #[test]
    fn cursor_read_never_restores_or_replaces_a_delivery_lease() {
        let mut committed: HostRequestEnvelope = serde_json::from_value(serde_json::json!({
            "effect_id":"effect", "request_id":"request", "session_id":"session",
            "turn_id":"turn", "host_id":"host", "lease_id":"catalog-lease",
            "catalog_revision":1, "availability_epoch":2, "created_at_ms":1,
            "deadline_ms":null, "resume_after_sequence":4,
            "payload":{"type":"cancel", "target_effect_id":"target","target_not_started":null}
        }))
        .unwrap();
        let old_delivery = serde_json::from_value(serde_json::json!({
            "attempt_id":"old-delivery", "worker_generation":"old-worker",
            "lease_owner":"old-owner", "lease_token":"old-token", "lease_expires_at_ms":10
        }))
        .unwrap();
        let mut hot = committed.clone();
        hot.delivery = Some(old_delivery);
        hot.resume_after_sequence = Some(8);
        overlay_request_cursor(
            &mut committed,
            nola_domain::host_control::HostEffectControl::from_request(&hot).unwrap(),
        )
        .unwrap();
        assert_eq!(committed.resume_after_sequence, Some(8));
        assert!(
            committed.delivery.is_none(),
            "a cursor read cannot claim execution"
        );

        let successor = serde_json::from_value(serde_json::json!({
            "attempt_id":"successor", "worker_generation":"new-worker",
            "lease_owner":"new-owner", "lease_token":"new-token", "lease_expires_at_ms":100
        }))
        .unwrap();
        committed.delivery = Some(successor);
        let before = committed.clone();
        overlay_request_cursor(
            &mut committed,
            nola_domain::host_control::HostEffectControl::from_request(&hot).unwrap(),
        )
        .unwrap();
        assert_eq!(
            committed, before,
            "old hot lease cannot replace the ledger claim"
        );

        hot.request_id = "foreign-request".into();
        hot.resume_after_sequence = Some(100);
        assert!(overlay_request_cursor(
            &mut committed,
            nola_domain::host_control::HostEffectControl::from_request(&hot).unwrap()
        )
        .is_err());
        assert_eq!(
            committed, before,
            "lease normalization must not normalize request identity"
        );
    }

    fn retained_run() -> RetainHotRun {
        RetainHotRun {
            run_id: "run".into(),
            conversation_id: "conversation".into(),
            core: AgentCore::try_new(AgentCoreConfig {
                session_id: Some(nola_agent_engine::agent::SessionId::from_string(
                    "conversation".into(),
                )),
                ..crate::model_fixture::config()
            })
            .unwrap(),
            terminal: false,
            checkpoint_revision: 7,
            next_event_sequence: 12,
            invocation_config_digest: "config".into(),
            deferred_events: Vec::new(),
            deferred_traces: Vec::new(),
            deferred_host_events: Vec::new(),
            deferred_bytes: 0,
        }
    }

    fn retain(supervisor: &mut RuntimeSupervisor) {
        supervisor.retain(retained_run());
    }

    #[test]
    fn narrow_goal_read_requires_exact_committed_owner_and_no_unpublished_tail() {
        let mut supervisor = RuntimeSupervisor::new();
        let retained = retained_run();
        let expected = retained.core.checkpoint_view(crate::CheckpointReason::Manual)
            .state.goal_projection.clone();
        supervisor.retain(retained);
        assert_eq!(supervisor.current_goal("conversation", 7), Some(expected));
        assert!(supervisor.current_permissions("run", "conversation", Some(7)).unwrap().is_some());
        assert!(supervisor.current_permissions("foreign", "conversation", Some(7)).unwrap().is_none());
        assert!(supervisor.current_permissions("run", "foreign", Some(7)).unwrap().is_none());
        assert!(supervisor.current_permissions("run", "conversation", Some(8)).unwrap().is_none());
        assert!(supervisor.cancellation_checkpoint("run", "conversation", 7).is_some());
        assert!(supervisor.cancellation_checkpoint("foreign", "conversation", 7).is_none());
        assert!(supervisor.cancellation_checkpoint("run", "foreign", 7).is_none());
        assert!(supervisor.cancellation_checkpoint("run", "conversation", 8).is_none());
        assert!(supervisor.current_goal("foreign", 7).is_none());
        assert!(supervisor.current_goal("conversation", 6).is_none());
        assert!(supervisor.current_goal("conversation", 8).is_none());
        supervisor.active_runs.get_mut("run").unwrap().deferred_host_events.push(
            nola_store_port::run_effect::HostEventInput {
                event_id: "unpublished".into(), effect_id: "effect".into(),
                request_id: "request".into(), sequence: 1, payload_json: "{}".into(),
                reduction_json: None, received_at_ms: 1,
            });
        assert!(supervisor.current_goal("conversation", 7).is_none());
        assert!(supervisor.current_permissions("run", "conversation", Some(7)).unwrap().is_none());
        assert!(supervisor.cancellation_checkpoint("run", "conversation", 7).is_none());
        assert!(supervisor.contains_run("run"), "a read cannot evict pending work");
    }

    #[test]
    fn hot_state_requires_the_owner_and_durable_revision_supplied_by_application() {
        let mut supervisor = RuntimeSupervisor::new();
        for (conversation, revision) in [
            ("conversation", None),
            ("conversation", Some(6)),
            ("conversation", Some(8)),
            ("foreign", Some(7)),
        ] {
            retain(&mut supervisor);
            assert!(supervisor
                .take_current("run", conversation, revision)
                .is_none());
            assert!(supervisor.contains_run("run"), "a mismatched reader must not evict its owner");
        }
        retain(&mut supervisor);
        let hot = supervisor
            .take_current("run", "conversation", Some(7))
            .unwrap();
        assert_eq!(hot.next_event_sequence, 12);
        assert_eq!(hot.invocation_config_digest, "config");
    }

    #[test]
    fn rolled_back_command_cannot_publish_its_hot_overlay() {
        let mut supervisor = RuntimeSupervisor::new();
        supervisor.begin_unit_of_work().unwrap();
        retain(&mut supervisor);
        assert!(!supervisor.contains_run("run"));
        supervisor.rollback_unit_of_work();
        assert!(supervisor
            .take_current("run", "conversation", Some(7))
            .is_none());
        supervisor.begin_unit_of_work().unwrap();
        retain(&mut supervisor);
        supervisor.commit_unit_of_work().unwrap();
        assert!(supervisor
            .take_current("run", "conversation", Some(7))
            .is_some());
    }

    #[test]
    fn only_receipt_units_of_work_require_a_durable_acknowledgement() {
        let mut supervisor = RuntimeSupervisor::new();
        assert!(!supervisor.has_unit_of_work());
        assert!(!supervisor.requires_durable_ack());

        supervisor.begin_unit_of_work().unwrap();
        assert!(supervisor.has_unit_of_work());
        assert!(supervisor.requires_durable_ack());
        assert!(supervisor.begin_durable_flush().is_err());
        assert!(supervisor.requires_durable_ack());
        supervisor.rollback_unit_of_work();
        assert!(!supervisor.has_unit_of_work());
        assert!(!supervisor.requires_durable_ack());

        supervisor.begin_durable_flush().unwrap();
        assert!(supervisor.has_unit_of_work());
        assert!(!supervisor.requires_durable_ack());
        assert!(supervisor.begin_unit_of_work().is_err());
        assert!(!supervisor.requires_durable_ack());
        supervisor.commit_unit_of_work().unwrap();
        assert!(!supervisor.has_unit_of_work());
    }

    #[test]
    fn durable_flush_retains_are_adopted_only_after_commit() {
        let mut supervisor = RuntimeSupervisor::new();
        retain(&mut supervisor);
        supervisor.begin_durable_flush().unwrap();
        let mut successor = retained_run();
        successor.run_id = "successor".into();
        successor.checkpoint_revision = 8;
        supervisor.retain(successor);
        assert!(supervisor.contains_run("run"));
        assert!(!supervisor.contains_run("successor"));
        supervisor.rollback_unit_of_work();
        assert!(supervisor.contains_run("run"));
        assert!(!supervisor.contains_run("successor"));

        supervisor.begin_durable_flush().unwrap();
        let mut successor = retained_run();
        successor.run_id = "successor".into();
        successor.checkpoint_revision = 8;
        supervisor.retain(successor);
        supervisor.commit_unit_of_work().unwrap();
        // The committed successor replaces the same conversation's old cache;
        // recovery does not need a second imperative invalidation API.
        assert!(!supervisor.contains_run("run"));
        assert_eq!(
            supervisor
                .take_current("successor", "conversation", Some(8))
                .unwrap()
                .checkpoint_revision,
            8
        );
    }

    #[test]
    fn deferred_byte_budget_requires_exact_run_owner_and_revision() {
        let mut supervisor = RuntimeSupervisor::new();
        let mut retained = retained_run();
        retained.deferred_bytes = 81;
        supervisor.retain(retained);
        for (run, conversation, revision) in [
            ("unknown", "conversation", Some(7)),
            ("run", "foreign", Some(7)),
            ("run", "conversation", None),
            ("run", "conversation", Some(6)),
            ("run", "conversation", Some(8)),
        ] {
            assert_eq!(supervisor.deferred_bytes(run, conversation, revision), 0);
        }
        assert_eq!(
            supervisor.deferred_bytes("run", "conversation", Some(7)),
            81
        );
        assert!(supervisor.contains_run("run"));
    }

    #[test]
    fn poison_discards_all_overlays_and_permanently_refuses_reuse() {
        let mut supervisor = RuntimeSupervisor::new();
        retain(&mut supervisor);
        supervisor.begin_unit_of_work().unwrap();
        retain(&mut supervisor);
        supervisor.poison("commit outcome unknown".into());
        assert_eq!(supervisor.active_run_count(), 0);
        assert!(!supervisor.has_unit_of_work());
        assert!(!supervisor.requires_durable_ack());
        assert!(supervisor
            .ensure_usable()
            .unwrap_err()
            .contains("commit outcome unknown"));
        assert!(supervisor.begin_unit_of_work().is_err());
        assert!(supervisor.begin_durable_flush().is_err());
        assert!(supervisor.commit_unit_of_work().is_err());

        supervisor.rollback_unit_of_work();
        retain(&mut supervisor);
        supervisor.poison("later rollback failure".into());
        assert_eq!(supervisor.active_run_count(), 0);
        assert!(supervisor
            .take_current("run", "conversation", Some(7))
            .is_none());
        assert!(supervisor
            .ensure_usable()
            .unwrap_err()
            .contains("commit outcome unknown"));
    }

    #[test]
    fn failed_adoption_poison_prevents_existing_hot_state_from_being_reused() {
        let mut supervisor = RuntimeSupervisor::new();
        retain(&mut supervisor);
        assert!(supervisor.commit_unit_of_work().is_err());
        assert_eq!(supervisor.active_run_count(), 0);
        assert!(supervisor.ensure_usable().is_err());
        assert!(supervisor.begin_durable_flush().is_err());
    }
}
