// SPDX-License-Identifier: Apache-2.0
//! Persistence contracts; mutations participate in the caller's unit of work.
use crate::permission::StoredPermissionRequest;
pub use nola_domain::agent_task_admission::{
    AgentTaskAdmission, RuntimeAgentTaskAuthority, RuntimeAgentTaskAuthorityPhase,
};
pub use nola_domain::task_execution::{RuntimeTaskAttempt, RuntimeTaskLeaseWitness, RuntimeTaskPermissionSuspension, RuntimeTaskRunBinding};
use serde::{Deserialize, Serialize};

/// Exact presentation occurrence. This is neither model consumption nor task acceptance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeTaskNotificationDelivery {
    /// None is an explicitly unowned shell task, never an owner wildcard.
    #[serde(deserialize_with = "deserialize_notification_owner")]
    pub owner_conversation_id: Option<String>,
    pub task_id: String,
    pub occurrence_id: String,
    pub consumer_id: String,
    pub command_id: String,
}

/// Presentation settlement only; no variant authorizes or completes task work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeTaskNotificationAcknowledgement {
    Delivered,
    AlreadyDelivered,
    /// A proven occurrence of this owner was resolved before its display ACK.
    /// No delivery record is written for this outcome.
    Superseded,
}

/// Stable, unambiguous identity of one committed question version. Labels and
/// timestamps are intentionally excluded; a revised question is a new occurrence.
pub fn user_input_notification_occurrence_id(
    run_id: &str,
    wait_id: &str,
    revision: u64,
    response_schema_digest: &str,
) -> String {
    format!("user-input:{}", serde_json::json!([run_id, wait_id, revision, response_schema_digest]))
}

fn deserialize_notification_owner<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<String>, D::Error> {
    Option::<String>::deserialize(deserializer)
}

#[cfg(test)]
mod notification_contract_tests {
    use super::*;

    #[test]
    fn question_occurrence_binds_all_identity_fields_without_delimiter_aliases() {
        let original=user_input_notification_occurrence_id("run:part", "wait", 1, "schema");
        assert_ne!(original,user_input_notification_occurrence_id("run", "part:wait", 1, "schema"));
        assert_ne!(original,user_input_notification_occurrence_id("run:part", "wait", 2, "schema"));
        assert_ne!(original,user_input_notification_occurrence_id("run:part", "wait", 1, "other"));
        assert_eq!(original,user_input_notification_occurrence_id("run:part", "wait", 1, "schema"));
    }

    #[test]
    fn unowned_notification_requires_explicit_null_not_a_missing_owner() {
        let mut value = serde_json::json!({"task_id":"task","occurrence_id":"terminal:receipt",
            "consumer_id":"cli","command_id":"display"});
        assert!(serde_json::from_value::<RuntimeTaskNotificationDelivery>(value.clone()).is_err());
        value["owner_conversation_id"] = serde_json::Value::Null;
        let delivery: RuntimeTaskNotificationDelivery = serde_json::from_value(value).unwrap();
        assert_eq!(delivery.owner_conversation_id, None);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoredAgentTaskAdmissionEvidence {
    pub source_store_epoch: String,
    pub source_transition_id: String,
    pub source_host_event_id: String,
    pub source_checkpoint_revision: i64,
    pub core_event_json: String,
    pub host_response_json: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredRuntimeTask {
    pub id: String,
    pub kind: String,
    pub status: String,
    pub title: String,
    pub project_root: String,
    pub owner_session_id: Option<String>,
    pub parent_task_id: Option<String>,
    pub parent_tool_call_id: Option<String>,
    pub child_session_id: Option<String>,
    pub agent_type: Option<String>,
    pub prompt: Option<String>,
    pub progress_summary: Option<String>,
    pub last_activity: Option<String>,
    pub result_summary: Option<String>,
    pub error: Option<String>,
    pub output_path: Option<String>,
    pub unread: bool,
    pub cancel_requested: bool,
    pub cancel_requested_at_ms: Option<i64>,
    pub worker_id: Option<String>,
    pub ownership_token: Option<String>,
    pub lease_until_ms: Option<i64>,
    pub heartbeat_at_ms: Option<i64>,
    pub retry_count: i64,
    pub retry_at_ms: Option<i64>,
    pub last_error: Option<String>,
    pub input_watermark: Option<i64>,
    pub completion_watermark: Option<i64>,
    pub created_at_ms: i64,
    pub started_at_ms: Option<i64>,
    pub ended_at_ms: Option<i64>,
    pub updated_at_ms: i64,
    pub metadata_json: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredRuntimeTaskEvent {
    pub id: i64,
    pub task_id: String,
    pub kind: String,
    pub status: String,
    pub summary: Option<String>,
    pub detail_json: String,
    pub created_at_ms: i64,
}

/// Field-limited, compare-and-swap progress transition for the active lease.
///
/// The expected projection fields fence a concurrent heartbeat, cancellation,
/// notification acknowledgement, or lifecycle transition between the
/// Application read and this write. Canonical state and lease columns are not
/// supplied by the caller and therefore cannot be overwritten by progress.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RuntimeTaskProgressActivity {
    pub summary: String,
    pub detail_json: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeTaskProgressMutation {
    pub task_id: String,
    pub attempt_id: String,
    pub expected_attempt_revision: i64,
    pub worker_generation: String,
    pub lease_token: String,
    pub current_expiry_ms: i64,
    pub expected_updated_at_ms: i64,
    pub expected_metadata_json: String,
    pub progress_summary: Option<String>,
    pub last_activity: Option<String>,
    pub activity: Option<RuntimeTaskProgressActivity>,
    pub metadata_json: String,
    pub now_ms: i64,
}

/// Application-computed final observation committed with the worker terminal.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RuntimeTaskFinalProgress {
    pub expected_updated_at_ms: i64,
    pub expected_metadata_json: String,
    pub metadata_json: String,
}

/// Application-derived budget observation for one exact non-executing wait.
/// This grants no execution authority and never renews an attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeTaskPermissionBudgetMutation {
    pub task_id: String,
    pub suspension_id: String,
    pub expected_suspension_revision: i64,
    pub attempt_id: String,
    pub expected_attempt_revision: i64,
    pub expected_updated_at_ms: i64,
    pub expected_started_at_ms: Option<i64>,
    pub expected_metadata_json: String,
    pub metadata_json: String,
    pub now_ms: i64,
}

/// Store-authored immutable evidence of a terminal transition. Never supplied
/// by a worker or reconstructed from the parent's current ownership token.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RuntimeTaskTerminalReceipt {
    pub journal_result: Option<RuntimeTaskJournalResult>,
    #[serde(default)]
    pub guidance: Vec<RuntimeTaskGuidance>,
    pub receipt_id: String,
    pub task_id: String,
    pub owner_session_id: String,
    pub child_session_id: Option<String>,
    pub input_watermark: Option<i64>,
    pub completion_watermark: Option<i64>,
    pub status: String,
    pub result_ref: Option<String>,
    pub ended_at_ms: i64,
    pub recorded_at_ms: i64,
    pub source: RuntimeTaskTerminalSource,
    pub attempts: Vec<RuntimeTaskAttempt>,
}

/// Consume one exact model-delivered terminal result, not the entire resume
/// mailbox and not a claim that the UI has displayed it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AcknowledgeRuntimeTaskResult {
    pub owner_conversation_id: String,
    pub task_id: String,
    pub terminal_receipt_id: String,
    pub expected_updated_at_ms: i64,
}

/// Source-authored terminal result. Preview is bounded; references, visibility
/// and byte counts are pinned to the immutable terminal publication, not trace.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RuntimeTaskJournalResult {
    /// The exact child Run result is inside this verified seal. Task execution
    /// settlement and supervising resource failures do not rewrite that result
    /// or establish acceptance of the parent's requirements.
    pub seal: nola_domain::assistant_run_terminal::AssistantRunTerminalSeal,
    pub public_utf8_bytes: nola_domain::assistant_journal_operations::JournalNumber,
    pub preview: String,
    pub preview_truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum RuntimeTaskTerminalSource {
    WorkerFinish {
        lease: RuntimeTaskLeaseWitness,
    },
    LeaseExpired,
    Cancellation,
    BoundRunTerminal {
        binding: RuntimeTaskRunBinding,
        assistant_message_id: String,
        budget_failure: Option<nola_domain::runtime_task_scheduling::RuntimeTaskBudgetFailure>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RuntimeTaskGuidance {
    pub task_id: String,
    pub message_id: String,
    pub owner_session_id: String,
    pub parent_core_session_id: String,
    pub parent_turn_id: String,
    pub child_session_id: String,
    pub run_id: String,
    pub input_watermark: Option<i64>,
    pub content: String,
    pub submitted_at_ms: i64,
    pub status: String,
    pub accepted_at_ms: Option<i64>,
    pub resolved_at_ms: Option<i64>,
    pub reason: Option<String>,
    pub delivery_transition_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RenewRuntimeTaskLease {
    pub task_id: String,
    pub attempt_id: String,
    pub expected_attempt_revision: i64,
    pub worker_generation: String,
    pub lease_token: String,
    pub current_expiry_ms: i64,
    pub now_ms: i64,
    pub requested_expiry_ms: i64,
}

/// Immutable evidence of the grant committed for one permission continuation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RuntimeTaskPermissionResume {
    pub suspension: RuntimeTaskPermissionSuspension,
    pub granted_attempt: RuntimeTaskAttempt,
}

/// A verified historical grant and its ordered predecessors for one permission
/// occurrence. None of these historical attempts authorizes a new worker.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RuntimeTaskPermissionContinuationGrant {
    pub suspension: RuntimeTaskPermissionSuspension,
    pub ancestors: Vec<RuntimeTaskAttempt>,
    pub granted_attempt: RuntimeTaskAttempt,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PermissionContinuationGrantSelector {
    ExactAttempt {
        attempt_id: String,
    },
    /// Observation only. Recovery must CAS this expired predecessor into a new
    /// grant; a worker must never adopt the returned historical attempt.
    RecoveryCandidate,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RecoverRuntimeTaskPermissionContinuationFair {
    pub suspension_id: String,
    pub expected_suspension_revision: i64,
    pub permission_request_id: String,
    pub continuation_id: String,
    pub expected_child_session_id: String,
    pub expected_run_id: String,
    pub predecessor: RuntimeTaskLeaseWitness,
    pub grant_attempt_id: String,
    pub worker_id: String,
    pub worker_generation: String,
    pub lease_token: String,
    pub now_ms: i64,
    pub lease_until_ms: i64,
    pub max_running_global: u32,
    pub max_running_project: u32,
    pub max_recoveries: u32,
}

/// A command may have committed even when its acknowledgement was lost.
/// This is not a permission wait, a failed task, or authorization to retry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RuntimeTaskCommandReconciliation {
    pub task_id: String,
    pub attempt_id: String,
    pub client_id: String,
    pub request_id: String,
    pub failure_code: String,
    pub state: String,
    pub revision: i64,
    pub awaiting_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SuspendRuntimeTaskForCommandReconciliation {
    pub lease: RuntimeTaskLeaseWitness,
    pub client_id: String,
    pub request_id: String,
    pub failure_code: String,
    pub progress_summary: Option<String>,
    pub now_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SuspendRuntimeTaskForPermission {
    pub task_id: String,
    pub attempt_id: String,
    pub expected_attempt_revision: i64,
    pub worker_generation: String,
    pub lease_token: String,
    pub current_expiry_ms: i64,
    /// The request id is the permission occurrence identity. The Store must
    /// ensure this fact in the same transaction as suspension.
    pub permission_request: StoredPermissionRequest,
    pub continuation_id: String,
    pub continuation_json: String,
    pub progress_summary: String,
    pub last_activity: Option<String>,
    pub now_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResumeRuntimeTaskAfterPermissionFair {
    pub task_id: String,
    pub suspension_id: String,
    pub expected_suspension_revision: i64,
    pub permission_request_id: String,
    pub continuation_id: String,
    pub worker_id: String,
    pub worker_generation: String,
    pub lease_token: String,
    pub now_ms: i64,
    pub lease_until_ms: i64,
    pub max_running_global: u32,
    pub max_running_project: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CancelRuntimeTaskPermissionSuspension {
    pub task_id: String,
    pub suspension_id: String,
    pub expected_suspension_revision: i64,
    pub permission_request_id: String,
    pub continuation_id: String,
    pub requested_at_ms: i64,
}

/// Preserve atomic state/event and scheduling admission on the same repository.
/// No operation may commit an enclosing command receipt transaction.
#[allow(clippy::too_many_arguments)]
pub trait RuntimeTaskRepository {
    /// None means no supported Core-authored persistent expert target. It never grants worker authority.
    fn validated_expert_task_target(&self, _task_id:&str, _child:&str, _require_admitted:bool)
        -> Result<Option<nola_domain::expert_task_target::ExpertTaskTargetV1>,Self::Error> { Ok(None) }
    fn submit_terminal_input(&mut self, input: &nola_domain::terminal_input::TerminalInputRequest,
        now_ms: i64) -> Result<nola_domain::terminal_input::TerminalInputReceipt, Self::Error>;
    /// Bounded target-side recovery candidates; never a task projection.
    fn prepared_agent_task_ids(&self, owner: &str, after_id: &str, limit: usize)
        -> Result<Vec<String>, Self::Error>;
    /// Locate original committed evidence without materializing a checkpoint.
    fn source_agent_task_admission_location(&self, owner: &str, task_id: &str)
        -> Result<Option<(String, u64)>, Self::Error>;
    fn runtime_agent_task_authority(
        &self,
        task_id: &str,
    ) -> Result<Option<RuntimeAgentTaskAuthority>, Self::Error>;
    /// Internal trusted Host import. Digest validation is not source
    /// authorization: Application must consume its authenticated exporter.
    /// This method is not an ordinary caller-supplied JSON command boundary.
    fn admit_runtime_agent_task(
        &mut self,
        admission: &AgentTaskAdmission,
    ) -> Result<RuntimeAgentTaskAuthority, Self::Error>;
    fn source_agent_task_admission_evidence(
        &self,
        conversation_id: &str,
        run_id: &str,
        task_id: &str,
        event_cursor: u64,
    ) -> Result<Option<StoredAgentTaskAdmissionEvidence>, Self::Error>;
    fn runtime_task_guidance_records(
        &self,
        task_id: &str,
    ) -> Result<Vec<RuntimeTaskGuidance>, Self::Error>;
    /// Count outstanding guidance without decoding its accumulated history.
    fn outstanding_runtime_task_guidance_count(&self, task_id: &str) -> Result<u64, Self::Error>;
    fn runtime_task_guidance(
        &self,
        task_id: &str,
        message_id: &str,
    ) -> Result<Option<RuntimeTaskGuidance>, Self::Error>;
    fn pending_runtime_task_guidance(
        &self,
        task_id: &str,
        run_id: &str,
    ) -> Result<Vec<RuntimeTaskGuidance>, Self::Error>;
    fn insert_runtime_task_guidance(
        &mut self,
        guidance: &RuntimeTaskGuidance,
    ) -> Result<RuntimeTaskGuidance, Self::Error>;
    fn resolve_runtime_task_guidance(
        &mut self,
        task_id: &str,
        message_id: &str,
        status: &str,
        reason: Option<&str>,
        transition_id: Option<&str>,
        at_ms: i64,
    ) -> Result<RuntimeTaskGuidance, Self::Error>;
    fn runtime_task_terminal_receipt(
        &self,
        task_id: &str,
    ) -> Result<Option<RuntimeTaskTerminalReceipt>, Self::Error>;
    type Error: std::fmt::Display;
    fn runtime_task_run_binding(
        &self,
        task_id: &str,
    ) -> Result<Option<RuntimeTaskRunBinding>, Self::Error>;
    /// Exact persisted source Run binding; absence never invents Task lineage.
    fn runtime_task_binding_for_run(&self, _child: &str, _run: &str) -> Result<Option<RuntimeTaskRunBinding>, Self::Error> { Ok(None) }
    fn validate_runtime_task_turn_admission(
        &self,
        lease: &RuntimeTaskLeaseWitness,
        child_session_id: &str,
        now_ms: i64,
    ) -> Result<(), Self::Error>;
    fn bind_runtime_task_run(
        &mut self,
        lease: &RuntimeTaskLeaseWitness,
        child_session_id: &str,
        run_id: &str,
        now_ms: i64,
    ) -> Result<RuntimeTaskRunBinding, Self::Error>;
    fn is_runtime_task_child_session(&self, child_session_id: &str) -> Result<bool, Self::Error>;
    fn reconcile_runtime_task_bound_run_terminal(
        &mut self,
        task_id: &str,
        expected_run_id: &str,
        now_ms: i64,
    ) -> Result<bool, Self::Error>;
    fn suspend_runtime_task_for_command_reconciliation(
        &mut self,
        input: &SuspendRuntimeTaskForCommandReconciliation,
    ) -> Result<Option<RuntimeTaskCommandReconciliation>, Self::Error>;
    fn pending_runtime_task_command_reconciliation(
        &self,
        task_id: &str,
    ) -> Result<Option<RuntimeTaskCommandReconciliation>, Self::Error>;
    fn acknowledge_runtime_task_model_delivery(
        &mut self,
        task_id: &str,
    ) -> Result<bool, Self::Error>;
    fn acknowledge_runtime_task_result(
        &mut self,
        input: &AcknowledgeRuntimeTaskResult,
    ) -> Result<bool, Self::Error>;
    fn runtime_task_notification_delivered(&self, delivery: &RuntimeTaskNotificationDelivery)
        -> Result<bool, Self::Error>;
    fn acknowledge_runtime_task_delivery(&mut self, delivery: &RuntimeTaskNotificationDelivery)
        -> Result<RuntimeTaskNotificationAcknowledgement, Self::Error>;
    fn append_runtime_task_event(
        &mut self,
        task_id: &str,
        kind: &str,
        status: &str,
        summary: Option<&str>,
        detail_json: &str,
        created_at_ms: i64,
    ) -> Result<i64, Self::Error>;
    fn claim_runtime_task(
        &mut self,
        task_id: &str,
        worker_id: &str,
        worker_generation: &str,
        ownership_token: &str,
        now_ms: i64,
        lease_until_ms: i64,
    ) -> Result<Option<RuntimeTaskAttempt>, Self::Error>;
    fn claim_runtime_task_fair(
        &mut self,
        task_id: &str,
        worker_id: &str,
        worker_generation: &str,
        ownership_token: &str,
        now_ms: i64,
        lease_until_ms: i64,
        max_running_global: u32,
        max_running_project: u32,
    ) -> Result<Option<RuntimeTaskAttempt>, Self::Error>;
    fn create_runtime_task_with_event(
        &mut self,
        task: &StoredRuntimeTask,
    ) -> Result<(), Self::Error>;
    fn fail_expired_runtime_task(
        &mut self,
        task_id: &str,
        error: &str,
        now_ms: i64,
    ) -> Result<bool, Self::Error>;
    fn fail_expired_runtime_task_with_event(
        &mut self,
        task_id: &str,
        error: &str,
        event_kind: &str,
        event_status: &str,
        event_summary: Option<&str>,
        event_detail_json: &str,
        now_ms: i64,
    ) -> Result<bool, Self::Error>;
    fn finish_runtime_task_detailed_with_event(
        &mut self,
        lease: &RuntimeTaskLeaseWitness,
        status: &str,
        completion_watermark: Option<i64>,
        progress_summary: Option<&str>,
        result_summary: Option<&str>,
        error: Option<&str>,
        output_path: Option<&str>,
        unread: bool,
        event_kind: &str,
        event_status: &str,
        event_summary: Option<&str>,
        event_detail_json: &str,
        final_progress: Option<&RuntimeTaskFinalProgress>,
        now_ms: i64,
    ) -> Result<bool, Self::Error>;
    fn renew_runtime_task_lease(
        &mut self,
        renewal: &RenewRuntimeTaskLease,
    ) -> Result<Option<RuntimeTaskAttempt>, Self::Error>;
    fn latest_runtime_task_event_id(&self) -> Result<i64, Self::Error>;
    /// Locate one prepared/admitted delegation by its immutable parent effect.
    fn runtime_task_for_delegation(
        &self, owner_conversation_id: &str, parent_run_id: &str, parent_effect_id: &str,
    ) -> Result<Option<StoredRuntimeTask>, Self::Error>;
    fn list_all_runtime_tasks(&self) -> Result<Vec<StoredRuntimeTask>, Self::Error>;
    fn list_runtime_tasks(&self, project_root: &str)
        -> Result<Vec<StoredRuntimeTask>, Self::Error>;
    fn recover_expired_runtime_task(
        &mut self,
        task_id: &str,
        error: &str,
        retry_at_ms: i64,
        now_ms: i64,
    ) -> Result<bool, Self::Error>;
    fn request_runtime_task_cancel(
        &mut self,
        task_id: &str,
        now_ms: i64,
    ) -> Result<bool, Self::Error>;
    fn report_runtime_task_progress(
        &mut self,
        mutation: &RuntimeTaskProgressMutation,
    ) -> Result<Option<RuntimeTaskAttempt>, Self::Error>;
    fn observe_runtime_task_permission_budget(
        &mut self,
        mutation: &RuntimeTaskPermissionBudgetMutation,
    ) -> Result<bool, Self::Error>;
    fn resume_runtime_task_after_permission_fair(
        &mut self,
        input: &ResumeRuntimeTaskAfterPermissionFair,
    ) -> Result<Option<RuntimeTaskAttempt>, Self::Error>;
    fn runtime_task_permission_suspension(
        &self,
        id: &str,
    ) -> Result<Option<RuntimeTaskPermissionSuspension>, Self::Error>;
    fn pending_runtime_task_permission_suspension(
        &self,
        task_id: &str,
    ) -> Result<Option<RuntimeTaskPermissionSuspension>, Self::Error>;
    fn latest_runtime_task_permission_suspension(
        &self,
        task_id: &str,
    ) -> Result<Option<RuntimeTaskPermissionSuspension>, Self::Error>;
    fn runtime_task_permission_resume(
        &self,
        task_id: &str,
        suspension_id: &str,
        permission_request_id: &str,
        continuation_id: &str,
    ) -> Result<Option<RuntimeTaskPermissionResume>, Self::Error>;
    fn runtime_task_permission_continuation_grant(
        &self,
        task_id: &str,
        suspension_id: &str,
        selector: &PermissionContinuationGrantSelector,
    ) -> Result<Option<RuntimeTaskPermissionContinuationGrant>, Self::Error>;
    fn recover_runtime_task_permission_continuation_fair(
        &mut self,
        input: &RecoverRuntimeTaskPermissionContinuationFair,
    ) -> Result<Option<RuntimeTaskAttempt>, Self::Error>;
    fn cancel_runtime_task_permission_suspension(
        &mut self,
        input: &CancelRuntimeTaskPermissionSuspension,
    ) -> Result<Option<RuntimeTaskPermissionSuspension>, Self::Error>;
    fn retry_runtime_task(
        &mut self,
        lease: &RuntimeTaskLeaseWitness,
        retry_at_ms: i64,
        error: &str,
        now_ms: i64,
    ) -> Result<bool, Self::Error>;
    fn runtime_task(&self, id: &str) -> Result<Option<StoredRuntimeTask>, Self::Error>;
    fn runtime_task_attempt(&self, id: &str) -> Result<Option<RuntimeTaskAttempt>, Self::Error>;
    fn runtime_task_attempt_retired_for_user_input(&self, task_id: &str, attempt_id: &str) -> Result<bool, Self::Error>;
    /// Exact current lease, including an expired lease for recovery proof.
    /// Never substitutes the newest historical Attempt for current ownership.
    fn runtime_task_current_attempt(
        &self,
        task_id: &str,
    ) -> Result<Option<RuntimeTaskAttempt>, Self::Error>;
    fn runtime_task_events(
        &self,
        task_id: &str,
        limit: usize,
    ) -> Result<Vec<StoredRuntimeTaskEvent>, Self::Error>;
    fn runtime_task_events_after_session_cursor(
        &self,
        owner_session_id: &str,
        after_cursor: i64,
        limit: usize,
    ) -> Result<Vec<StoredRuntimeTaskEvent>, Self::Error>;
    fn suspend_runtime_task_for_permission(
        &mut self,
        input: &SuspendRuntimeTaskForPermission,
    ) -> Result<Option<RuntimeTaskPermissionSuspension>, Self::Error>;
}

/// Exact Task facts needed by public capability projection, without worker mutation authority.
pub trait TaskAuthorityReadRepository {
    type Error: std::fmt::Display;
    fn read_runtime_task(&self, id: &str) -> Result<Option<StoredRuntimeTask>, Self::Error>;
    fn read_runtime_task_run_binding(&self, id: &str) -> Result<Option<RuntimeTaskRunBinding>, Self::Error>;
    fn read_runtime_agent_task_authority(&self, id: &str) -> Result<Option<RuntimeAgentTaskAuthority>, Self::Error>;
    fn read_validated_expert_task_target(&self, _task_id:&str, _child:&str, _require_admitted:bool)
        -> Result<Option<nola_domain::expert_task_target::ExpertTaskTargetV1>,Self::Error> { Ok(None) }
}
impl<T: RuntimeTaskRepository + ?Sized> TaskAuthorityReadRepository for T {
    type Error = <T as RuntimeTaskRepository>::Error;
    fn read_validated_expert_task_target(&self, task_id:&str, child:&str, require_admitted:bool)
        -> Result<Option<nola_domain::expert_task_target::ExpertTaskTargetV1>,Self::Error> {
        self.validated_expert_task_target(task_id,child,require_admitted)
    }
    fn read_runtime_task(&self, id: &str) -> Result<Option<StoredRuntimeTask>, Self::Error> {
        self.runtime_task(id)
    }
    fn read_runtime_task_run_binding(&self, id: &str) -> Result<Option<RuntimeTaskRunBinding>, Self::Error> {
        self.runtime_task_run_binding(id)
    }
    fn read_runtime_agent_task_authority(&self, id: &str) -> Result<Option<RuntimeAgentTaskAuthority>, Self::Error> {
        self.runtime_agent_task_authority(id)
    }
}
