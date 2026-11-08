// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Defines the only public entry point into the Agent Core.
// - Boundary: Hosts submit typed input and execute returned effects; reducer internals stay private.

use super::host::HostRequestEnvelope;
use super::protocol::{
    AgentCoreConfig, AgentCoreSnapshot, AgentReduction, AgentSubmission, AgentTransition,
};
use super::recovery::{CheckpointReason, CoreCheckpoint};
use super::runtime::AgentRuntime;
use super::session::AgentThreadState;

/// Platform-neutral Agent kernel.
///
/// CLI, Web, FFI, and native Hosts all use this boundary:
///
/// 1. submit one typed input;
/// 2. receive canonical events, trace, and Host effects;
/// 3. execute effects outside Core;
/// 4. submit correlated Host responses.
///
/// Reducer state, active Turns, and scheduling internals are intentionally not
/// exposed. This prevents platform clients from growing a second Agent loop.
#[derive(Debug)]
pub struct AgentCore {
    // The owned Run candidate has a stable heap location. Application moves
    // this handle across replay/transaction layers; it must not copy a whole
    // session into every suspended caller's stack frame.
    reducer: Box<AgentRuntime>,
}

#[cfg(test)]
#[test]
fn owned_core_candidate_is_a_heap_handle_not_an_inline_session() {
    assert_eq!(std::mem::size_of::<AgentCore>(), std::mem::size_of::<usize>());
}

impl AgentCore {
    pub fn try_new(config: AgentCoreConfig) -> Result<Self, String> {
        Ok(Self {
            reducer: Box::new(AgentRuntime::try_new(config)?),
        })
    }

    pub fn restore(
        config: AgentCoreConfig,
        checkpoint: CoreCheckpoint<AgentThreadState>,
    ) -> Result<Self, String> {
        Ok(Self {
            reducer: Box::new(AgentRuntime::restore(config, checkpoint)?),
        })
    }

    pub fn bind_next_trace_root(&mut self, message_id: &str) -> Result<(), String> {
        self.reducer.bind_next_trace_root(message_id)
    }

    pub fn submit(&mut self, submission: AgentSubmission) -> Result<AgentTransition, String> {
        self.submit_at(submission, observation_time_ms())
    }

    /// Deterministic entry used by Application's admitted command. Every Effect
    /// created by this reduction uses the same externally observed time.
    pub fn submit_at(
        &mut self,
        submission: AgentSubmission,
        observed_at_ms: u64,
    ) -> Result<AgentTransition, String> {
        submission.validate()?;
        self.reducer.submit(submission, observed_at_ms)
    }

    pub fn snapshot(&self) -> AgentCoreSnapshot {
        self.reducer.snapshot()
    }

    /// Reduce admitted input without constructing the cumulative UI view.
    pub fn reduce_at(
        &mut self,
        submission: AgentSubmission,
        observed_at_ms: u64,
    ) -> Result<AgentReduction, String> {
        self.reducer.reduce(submission, observed_at_ms)
    }

    /// Application owns this candidate exclusively. Errors consume it instead
    /// of cloning the complete session for rollback on every stream delta.
    /// The returned candidate is not durable until the caller commits storage.
    pub fn reduce_owned_at(
        self,
        submission: AgentSubmission,
        observed_at_ms: u64,
    ) -> Result<(Self, AgentReduction), String> {
        let (reducer, reduction) = self.reducer.reduce_owned(submission, observed_at_ms)?;
        Ok((Self { reducer }, reduction))
    }

    /// Control-flow query without constructing the complete presentation model.
    pub fn turn_status(&self) -> super::turn::AgentTurnStatus {
        self.reducer.turn_status()
    }

    pub fn host_effect_is_cancelling(&self, effect_id: &str) -> bool {
        self.reducer.host_effect_is_cancelling(effect_id)
    }

    pub fn pending_host_requests(&self) -> Result<Vec<HostRequestEnvelope>, String> {
        self.reducer.pending_host_requests()
    }

    pub fn committed_host_request_resume_snapshot(
        &self,
        effect_id: &str,
    ) -> Result<Option<HostRequestEnvelope>, String> {
        self.reducer
            .committed_host_request_resume_snapshot(effect_id)
    }

    /// Committed scalar control only; never copies an assistant projection or
    /// any frozen prompt, permission input, memory or Skill content.
    pub fn control_snapshot(&self) -> super::protocol::AgentControlSnapshot {
        self.reducer.control_snapshot()
    }

    pub fn host_execution_state(&self) -> super::HostExecutionState {
        self.reducer.host_execution_state()
    }

    /// Returns a persistence-safe checkpoint after any accepted submission,
    /// including nonterminal streaming responses that do not emit a dedicated
    /// checkpoint effect.
    pub fn checkpoint(&self, reason: CheckpointReason) -> CoreCheckpoint<AgentThreadState> {
        self.reducer.checkpoint(reason)
    }

    /// Borrow live recovery state for synchronous persistence. The borrow
    /// prevents reduction/adoption while the checkpoint is being encoded.
    pub fn checkpoint_view(
        &self,
        reason: CheckpointReason,
    ) -> CoreCheckpoint<std::borrow::Cow<'_, AgentThreadState>> {
        self.reducer.checkpoint_view(reason)
    }

    /// Consume a candidate that will not be retained as a hot runtime. Frozen
    /// requests and transcript move into recovery state without a full clone.
    pub fn into_checkpoint(self, reason: CheckpointReason) -> CoreCheckpoint<AgentThreadState> {
        self.reducer.into_checkpoint(reason)
    }

    /// Rebinds the exact request read from the committed Application outbox.
    ///
    /// Unlike `resume_committed_host_effect`, this does not mint a replacement
    /// request identity. Application uses it when the Host has already
    /// executed the committed request and returned a correlated response.
    /// Recovery-only materialization from Application's digest-verified outbox.
    pub fn restore_committed_model_input(&mut self, request: &HostRequestEnvelope) -> Result<(), String> {
        self.reducer.restore_committed_model_input(request)
    }

    pub fn bind_committed_host_effect(
        &mut self,
        request: &HostRequestEnvelope,
    ) -> Result<(), String> {
        self.reducer.bind_committed_host_effect(request)
    }

    pub fn committed_host_effect_control(
        &self,
        effect_id: &str,
    ) -> Result<Option<nola_domain::host_control::HostEffectControl>, String> {
        self.reducer.committed_host_effect_control(effect_id)
    }

    pub fn bind_committed_host_response_control(&mut self, control: &nola_domain::host_control::HostEffectControl) -> Result<(), String> {
        self.reducer.bind_committed_host_response_control(control)
    }

    pub fn bind_committed_host_control(
        &mut self,
        control: &nola_domain::host_control::HostEffectControl,
    ) -> Result<(), String> {
        self.reducer.bind_committed_host_control(control)
    }

    /// Rebinds a redacted pending effect from the committed Application
    /// outbox and returns a fresh replay transition with renewed delivery
    /// identity and deadline.
    pub fn resume_committed_host_effect(
        &mut self,
        request: &HostRequestEnvelope,
    ) -> Result<AgentTransition, String> {
        self.resume_committed_host_effect_at(request, observation_time_ms())
    }

    pub fn resume_committed_host_effect_at(
        &mut self,
        request: &HostRequestEnvelope,
        observed_at_ms: u64,
    ) -> Result<AgentTransition, String> {
        let reduction = self
            .reducer
            .resume_committed_host_effect(request, observed_at_ms)?;
        Ok(reduction.with_snapshot(self.snapshot()))
    }

    /// Creates an independent checkpoint through one completed durable Item.
    /// The parent runtime is not mutated; Hosts persist and restore the result
    /// under `new_session_id`.
    pub fn branch_checkpoint(
        &self,
        new_session_id: &str,
        through_item_id: &str,
    ) -> Result<CoreCheckpoint<AgentThreadState>, String> {
        self.reducer
            .branch_checkpoint(new_session_id, through_item_id)
    }

    /// Creates a child checkpoint from durable state when no parent Runtime is
    /// resident. `through_message_id` is Core ownership identity, not display
    /// text or a client array index.
    pub fn branch_checkpoint_from_checkpoint(
        checkpoint: &CoreCheckpoint<AgentThreadState>,
        new_session_id: &str,
        through_message_id: &str,
    ) -> Result<CoreCheckpoint<AgentThreadState>, String> {
        AgentRuntime::branch_checkpoint_from_checkpoint(
            checkpoint,
            new_session_id,
            through_message_id,
        )
    }

    pub fn memory_worker_plan(
        &self,
        job_id: &str,
    ) -> Result<super::memory_worker::MemoryWorkerPlan, String> {
        self.reducer.memory_worker_plan(job_id)
    }
}

// Convenience facade for standalone Core callers. Application supplies its
// admitted timestamp through submit_at; reducer modules never read this clock.
fn observation_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

#[cfg(test)]
mod control_read_tests {
    use super::*;

    fn config() -> AgentCoreConfig {
        AgentCoreConfig {
            turn_identity: crate::agent::model_capability::test_host_identity(),
            session_id: Some(super::super::protocol::SessionId::from_string(
                "control-read-fixture".into(),
            )),
            ..Default::default()
        }
    }

    #[test]
    fn snapshot_free_reduction_preserves_execution_and_replay() {
        let mut projected = AgentCore::try_new(config()).unwrap();
        let checkpoint = projected.checkpoint(CheckpointReason::Manual);
        let mut owned = AgentCore::restore(config(), checkpoint.clone()).unwrap();
        let mut incremental = AgentCore::restore(config(), checkpoint).unwrap();
        for now in [1, 2] {
            let submission = AgentSubmission::CloseSession {
                submission_id: Some("snapshot-free-close".into()),
            };
            let full = projected.submit_at(submission.clone(), now).unwrap();
            let (next_owned, owned_delta) = owned.reduce_owned_at(submission.clone(), now).unwrap();
            owned = next_owned;
            let delta = incremental.reduce_at(submission, now).unwrap();
            let materialized = delta.with_snapshot(incremental.snapshot());
            let expected = serde_json::to_value(full).unwrap();
            assert_eq!(expected, serde_json::to_value(materialized).unwrap());
            assert_eq!(
                expected,
                serde_json::to_value(owned_delta.with_snapshot(owned.snapshot())).unwrap()
            );
            assert_eq!(
                serde_json::to_value(projected.checkpoint(CheckpointReason::Manual)).unwrap(),
                serde_json::to_value(incremental.checkpoint(CheckpointReason::Manual)).unwrap()
            );
            assert_eq!(
                serde_json::to_value(projected.checkpoint(CheckpointReason::Manual)).unwrap(),
                serde_json::to_value(owned.checkpoint(CheckpointReason::Manual)).unwrap()
            );
        }
        assert_eq!(
            serde_json::to_value(projected.checkpoint(CheckpointReason::Manual)).unwrap(),
            serde_json::to_value(owned.into_checkpoint(CheckpointReason::Manual)).unwrap()
        );
    }

    #[test]
    fn rejected_reduction_preserves_borrowed_core_and_consumes_owned_candidate() {
        let mut core = AgentCore::try_new(config()).unwrap();
        let checkpoint = core.checkpoint(CheckpointReason::Manual);
        let invalid = AgentSubmission::ResumeHostEffect {
            effect_id: "missing-effect".into(),
        };
        assert!(core.reduce_at(invalid.clone(), 42).is_err());
        assert_eq!(
            serde_json::to_value(&checkpoint).unwrap(),
            serde_json::to_value(core.checkpoint(CheckpointReason::Manual)).unwrap()
        );
        assert!(core.reduce_owned_at(invalid, 42).is_err());
        // Recovery is a new candidate from canonical facts, never the failed one.
        let restored = AgentCore::restore(config(), checkpoint).unwrap();
        assert_eq!(
            restored.turn_status(),
            super::super::turn::AgentTurnStatus::Idle
        );
    }

    #[test]
    fn narrow_control_reads_match_projection_without_changing_revision() {
        let mut core = AgentCore::try_new(config()).unwrap();
        for close in [false, true] {
            if close {
                core.submit_at(
                    AgentSubmission::CloseSession {
                        submission_id: Some("close-control-read-fixture".into()),
                    },
                    1,
                )
                .unwrap();
            }
            let before = core.snapshot();
            assert_eq!(core.turn_status(), before.turn_status);
            assert!(!core.host_effect_is_cancelling("unknown-effect"));
            assert_eq!(core.snapshot().revision, before.revision);
        }
    }
}
