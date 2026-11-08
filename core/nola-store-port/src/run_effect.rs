// SPDX-License-Identifier: Apache-2.0
//! Canonical Run recovery and atomic Host-effect transition persistence.
use crate::memory::StoredMemoryUsageEntry;
use nola_domain::{PartMutation, TransitionMessageInput};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OutboxEffectInput {
    #[serde(default)]
    pub model_attempt_id: Option<String>,
    #[serde(default)]
    pub delegated_invocation_id: Option<String>,
    pub effect_id: String,
    pub request_id: String,
    pub effect_kind: String,
    pub write: OutboxEffectWrite,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum OutboxEffectWrite {
    Declare { payload_json: String },
    Rebind {
        expected_request_id: String,
        expected_attempt_id: Option<String>,
        binding: crate::host_replay::CoreHostBindingOverlay,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredOutboxEffectIdentity {
    pub effect_id: String,
    pub run_id: String,
    pub request_id: String,
    pub effect_kind: String,
    pub status: String,
    pub binding: crate::host_replay::CoreHostBindingOverlay,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HostEventInput {
    pub event_id: String,
    pub effect_id: String,
    pub request_id: String,
    pub sequence: i64,
    pub payload_json: String,
    /// Exact normalized response consumed by Core. None denotes an external
    /// checkpoint-only commit that cannot be replayed as a reducer input.
    #[serde(default)]
    pub reduction_json: Option<String>,
    pub received_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RunEventInput {
    pub event_id: String,
    pub sequence: i64,
    pub event_kind: String,
    pub payload_json: String,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CheckpointWrite {
    Baseline { checkpoint_json: String },
    Replay { delivery_witness_json: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommitTransitionInput {
    pub transition_id: String,
    pub run_id: String,
    pub expected_checkpoint_revision: i64,
    pub checkpoint_revision: i64,
    pub checkpoint_event_cursor: i64,
    pub checkpoint: CheckpointWrite,
    pub host_event: Option<HostEventInput>,
    /// Exact non-Host reducer input, ordered with Host responses in this commit.
    pub submission_reduction_json: Option<String>,
    pub events: Vec<RunEventInput>,
    /// Messages materialized by Core at a safe boundary, such as a promoted
    /// NextTurn. Existing identities are validated and left unchanged.
    #[serde(default)]
    pub ensured_messages: Vec<TransitionMessageInput>,
    /// Explicit immutable ownership for Parts that do not belong to the Run's
    /// initially opened assistant Message.
    #[serde(default)]
    pub part_message_owners: BTreeMap<String, String>,
    /// Accepted typed Core ownership evidence, committed with canonical membership.
    pub assistant_turn_bindings: Vec<nola_domain::AssistantTurnBinding>,
    pub part_mutations: Vec<PartMutation>,
    pub outbox_effects: Vec<OutboxEffectInput>,
    #[serde(default)]
    pub model_attempt_mutations: Vec<nola_domain::ModelAttemptMutation>,
    #[serde(default)]
    pub delegated_invocation_mutations: Vec<nola_domain::DelegatedInvocationMutation>,
    #[serde(default)]
    pub delegated_permission_mutations: Vec<nola_domain::DelegatedPermissionDeliveryMutation>,
    #[serde(default)]
    pub effect_delivery_attempt_mutations: Vec<nola_domain::EffectDeliveryAttemptMutation>,
    #[serde(default)]
    pub acknowledged_effect_ids: Vec<String>,
    #[serde(default)]
    pub memory_usage_entries: Vec<StoredMemoryUsageEntry>,
    pub run_status: Option<String>,
    pub assistant_status: Option<String>,
    /// Defaults to the initially opened assistant Message when absent.
    #[serde(default)]
    pub assistant_status_message_id: Option<String>,
    pub committed_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommitTransitionResult {
    pub applied: bool,
    pub checkpoint_revision: i64,
    pub next_event_sequence: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredCheckpoint {
    pub conversation_id: String,
    pub revision: i64,
    pub event_cursor: i64,
    pub checkpoint_json: String,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredCheckpointReplayBase {
    pub run_id: String,
    pub checkpoint_revision: i64,
    pub through_ordinal: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredRecoveryCheckpoint {
    pub checkpoint: StoredCheckpoint,
    pub replay_base: Option<StoredCheckpointReplayBase>,
    pub replay_inputs: Vec<StoredCoreReplayInput>,
    /// Digest-verified pending model inputs, read in the same Store snapshot.
    /// Reducer replay must not replace a committed prompt with today's renderer.
    pub frozen_model_requests: Vec<String>,
}
impl From<StoredCheckpoint> for StoredRecoveryCheckpoint {
    fn from(checkpoint: StoredCheckpoint) -> Self {
        Self { checkpoint, replay_base: None, replay_inputs: Vec::new(), frozen_model_requests: Vec::new() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredCoreReplayInput {
    pub kind: ReplayInputKind,
    pub ordinal: i64,
    pub reduction_json: String,
    pub received_at_ms: i64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ReplayInputKind {
    HostResponse,
    Submission,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SubmissionReplayReduction {
    pub protocol_version: u16,
    pub session_id: String,
    pub decision_time_ms: i64,
    pub submission_json: String,
    /// Actual pre-reduction rebindings, without frozen request payloads.
    pub bindings: Vec<crate::host_replay::CoreHostBindingOverlay>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredActiveRun {
    pub run_id: String,
    pub kind: String,
    pub assistant_message_id: Option<String>,
    pub assistant_part_id: Option<String>,
    pub next_event_sequence: i64,
    pub invocation_config_digest: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredRunInvocationConfig {
    pub run_id: String,
    pub invocation_config_json: String,
    pub invocation_config_digest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredOutboxEffect {
    pub effect_id: String,
    pub run_id: String,
    pub request_id: String,
    pub effect_kind: String,
    pub payload_json: String,
    pub status: String,
    pub created_at_ms: i64,
    pub acknowledged_at_ms: Option<i64>,
}

/// All fields belong to the same read snapshot (or the caller's existing UoW).
/// Empty pending effects do not require an extant Run/checkpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommittedHostDeliverySnapshot<C = StoredCheckpoint> {
    pub conversation_id: Option<String>,
    pub checkpoint: Option<C>,
    pub effects: Vec<StoredOutboxEffect>,
}

/// Delivery admission only; never a Core restore checkpoint. Contains session
/// identity and effect identity/cursor/replay metadata, without request bodies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredDeliveryCheckpointWitness {
    pub conversation_id: String,
    pub witness_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredEffectHeartbeatSnapshot {
    pub effect_id: String,
    pub run_id: String,
    pub request_id: String,
    pub checkpoint: Option<StoredDeliveryCheckpointWitness>,
}

/// A repository instance participates in its caller's existing command UoW.
/// `commit_transition` atomically validates revisions/identities and commits
/// checkpoint, Run events, canonical Parts and effect settlement together.
/// Implementations must use a savepoint inside an outer receipt transaction;
/// neither reads nor transition commits may open a separate writer connection
/// or commit/rollback that outer transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommittedHostResponseReceipt {
    pub checkpoint_revision: i64,
    pub next_event_sequence: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedCancellationNotStarted {
    pub proof: nola_domain::cancellation_not_started::CancellationTargetNotStarted,
    pub mutations: Vec<nola_domain::EffectDeliveryAttemptMutation>,
}

pub trait RunEffectRepository {
    type Error: std::fmt::Display;
    /// Runs only inside the original Application write UoW. A positive fact
    /// atomically revokes every unstarted lease; unknown history yields None,
    /// never inferred authority from a Host's absent local registry.
    fn prepare_unstarted_cancellation_target(&self, run_id: &str,
        cancellation: &nola_domain::host_control::HostEffectControl,
        target: &nola_domain::host_control::HostEffectControl,
        checkpoint_revision: i64, at_ms: i64)
        -> Result<Option<PreparedCancellationNotStarted>, Self::Error>;
    /// Settlement inventory in the caller's snapshot. Never materializes
    /// request bodies; dispatch and cold recovery retain their full validation.
    fn pending_outbox_effect_ids(&self, run_id: &str) -> Result<Vec<String>, Self::Error>;
    /// Exact original terminal envelope plus its committed transition. This
    /// proves prior acceptance only, never current execution authority.
    fn committed_terminal_host_response(&self, conversation_id: &str, run_id: &str,
        event_id: &str, response_json: &str) -> Result<Option<CommittedHostResponseReceipt>, Self::Error>;
    fn outbox_effect_identity(&self, effect_id: &str) -> Result<Option<StoredOutboxEffectIdentity>, Self::Error>;
    /// Already-reduced transient responses, committed with their next durable
    /// boundary. Requires the caller's write UoW; never acknowledges by itself.
    fn append_host_event_history(&mut self, run_id: &str, events: &[HostEventInput]) -> Result<(), Self::Error>;
    /// Baseline and its Run-specific replay prefix from the same read snapshot.
    fn load_recovery_checkpoint(&self, conversation_id: &str) -> Result<Option<StoredRecoveryCheckpoint>, Self::Error>;
    /// Optional bounded memo of a pure recovery read. An adapter must reject
    /// uncommitted writes and invalidate on every local or external DB change.
    /// This is never an execution checkpoint or a source of delivery authority.
    fn cached_read_checkpoint(&self, _conversation_id: &str) -> Result<Option<StoredCheckpoint>, Self::Error> { Ok(None) }
    fn cache_read_checkpoint(&self, _checkpoint: &StoredCheckpoint) -> Result<(), Self::Error> { Ok(()) }
    /// Bounded incremental-write eligibility from current durable metadata only.
    fn checkpoint_replay_ready(&self, conversation_id: &str, run_id: &str, revision: i64,
        additional_reduction_bytes: usize) -> Result<bool, Self::Error>;
    fn checkpoint_replay_pending(&self, conversation_id: &str) -> Result<bool, Self::Error>;
    /// Filter exact ledger replays in the caller's transaction. A reused ID
    /// with different content is a conflict, even if its memory was deleted.
    fn unrecorded_memory_usage_entries(
        &self,
        entries: &[StoredMemoryUsageEntry],
    ) -> Result<Vec<StoredMemoryUsageEntry>, Self::Error>;
    /// Original field state, not a presentation DTO or public journal handle.
    /// Reject a known Part whose persisted owner differs from this owner.
    fn load_canonical_text_state(
        &self,
        owner: &nola_domain::CanonicalStreamOwner,
    ) -> Result<nola_domain::CanonicalTextState, Self::Error>;
    fn open_delegated_permissions_for_target(
        &self,
        effect_id: &str,
    ) -> Result<Vec<nola_domain::DelegatedPermissionDeliveryRecord>, Self::Error>;
    fn delegated_permission(
        &self,
        id: &str,
    ) -> Result<Option<nola_domain::DelegatedPermissionDeliveryRecord>, Self::Error>;
    fn delegated_permission_for_occurrence(
        &self,
        invocation_id: &str,
        occurrence_id: &str,
    ) -> Result<Option<nola_domain::DelegatedPermissionDeliveryRecord>, Self::Error>;
    fn delegated_permission_for_control(
        &self,
        effect_id: &str,
    ) -> Result<Option<nola_domain::DelegatedPermissionDeliveryRecord>, Self::Error>;
    fn delegated_invocation(
        &self,
        id: &str,
    ) -> Result<Option<nola_domain::DelegatedInvocationRecord>, Self::Error>;
    fn delegated_invocation_for_effect(
        &self,
        effect_id: &str,
    ) -> Result<Option<nola_domain::DelegatedInvocationRecord>, Self::Error>;
    fn model_attempt(
        &self,
        id: &str,
    ) -> Result<Option<nola_domain::ModelAttemptRecord>, Self::Error>;
    fn active_model_attempt_for_effect(
        &self,
        effect_id: &str,
    ) -> Result<Option<nola_domain::ModelAttemptRecord>, Self::Error>;
    fn checkpoint_revision(&self, conversation_id: &str) -> Result<Option<i64>, Self::Error>;
    fn load_checkpoint(
        &self,
        conversation_id: &str,
    ) -> Result<Option<StoredCheckpoint>, Self::Error>;
    fn active_run(&self, conversation_id: &str) -> Result<Option<StoredActiveRun>, Self::Error>;
    fn load_run_invocation_config(
        &self,
        run_id: &str,
    ) -> Result<Option<StoredRunInvocationConfig>, Self::Error>;
    fn outbox_effect(&self, effect_id: &str) -> Result<Option<StoredOutboxEffect>, Self::Error>;
    fn load_committed_host_delivery(
        &self,
        run_id: &str,
    ) -> Result<CommittedHostDeliverySnapshot, Self::Error>;
    fn load_committed_host_delivery_witness(
        &self,
        run_id: &str,
    ) -> Result<CommittedHostDeliverySnapshot<StoredDeliveryCheckpointWitness>, Self::Error> {
        self.load_selected_host_delivery_witness(run_id, None)
    }
    fn load_selected_host_delivery_witness(
        &self, run_id: &str, effect_ids: Option<&std::collections::HashSet<String>>,
    ) -> Result<CommittedHostDeliverySnapshot<StoredDeliveryCheckpointWitness>, Self::Error>;
    /// One pending effect and its exact committed witness, no outbox bodies.
    fn load_effect_heartbeat_snapshot(
        &self, run_id: &str, effect_id: &str,
    ) -> Result<Option<StoredEffectHeartbeatSnapshot>, Self::Error>;
    fn commit_transition(
        &mut self,
        input: &CommitTransitionInput,
    ) -> Result<CommitTransitionResult, Self::Error>;
}
