// SPDX-License-Identifier: Apache-2.0
//! Durable background-task lifecycle use-case boundary.

#[allow(unused_imports)]
pub use nola_store_port::runtime_task::{
    AcknowledgeRuntimeTaskResult, RuntimeTaskNotificationAcknowledgement, RuntimeTaskNotificationDelivery, CancelRuntimeTaskPermissionSuspension,
    PermissionContinuationGrantSelector, RecoverRuntimeTaskPermissionContinuationFair,
    RenewRuntimeTaskLease, ResumeRuntimeTaskAfterPermissionFair, RuntimeTaskAttempt,
    RuntimeTaskCommandReconciliation, RuntimeTaskGuidance, RuntimeTaskLeaseWitness, RuntimeTaskFinalProgress,
    RuntimeTaskPermissionContinuationGrant, RuntimeTaskPermissionResume,
    RuntimeTaskPermissionSuspension, RuntimeTaskPermissionBudgetMutation, RuntimeTaskProgressMutation, RuntimeTaskRepository,
    RuntimeTaskRunBinding, RuntimeTaskTerminalReceipt, StoredRuntimeTask, StoredRuntimeTaskEvent,
    SuspendRuntimeTaskForCommandReconciliation, SuspendRuntimeTaskForPermission,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub use nola_domain::runtime_task_scheduling::{
    RuntimeTaskPriority, RuntimeTaskResourceBudget, RuntimeTaskResourceUsage,
    RuntimeTaskScheduling, SCHEDULING_METADATA_KEY,
};

/// Immutable request used to create a durable task.
///
/// Runtime-owned state is intentionally absent. Clients cannot create a task
/// that already appears claimed, completed, cancelled, retried, or partially
/// progressed; those transitions are separate Application commands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateRuntimeTask {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub project_root: String,
    pub owner_session_id: Option<String>,
    pub parent_task_id: Option<String>,
    pub parent_tool_call_id: Option<String>,
    pub child_session_id: Option<String>,
    pub agent_type: Option<String>,
    pub prompt: Option<String>,
    pub initial_progress_summary: Option<String>,
    pub initial_activity: Option<String>,
    pub output_path: Option<String>,
    pub initially_unread: bool,
    pub input_watermark: Option<i64>,
    pub metadata: Value,
    pub scheduling: RuntimeTaskScheduling,
    pub created_at_ms: i64,
}

impl CreateRuntimeTask {
    pub fn validate(&self) -> Result<(), String> {
        for (label, value) in [
            ("id", self.id.as_str()),
            ("kind", self.kind.as_str()),
            ("title", self.title.as_str()),
            ("project_root", self.project_root.as_str()),
        ] {
            if value.trim().is_empty() {
                return Err(format!("runtime task {label} must not be empty"));
            }
        }
        if self.created_at_ms < 0 {
            return Err("runtime task created_at_ms must be non-negative".into());
        }
        if !self.metadata.is_object() {
            return Err("runtime task metadata must be a JSON object".into());
        }
        if self.metadata.get(nola_domain::terminal_input::TERMINAL_INPUT_RECEIPTS_METADATA_KEY).is_some()
            || self.metadata.get(nola_domain::terminal_input::TERMINAL_INPUT_QUEUE_METADATA_KEY).is_some() {
            return Err("new tasks cannot supply terminal input delivery receipts".into());
        }
        self.scheduling.validate()
    }

    pub(crate) fn into_task(self) -> Result<RuntimeTask, String> {
        self.validate()?;
        let mut task = RuntimeTask {
            id: self.id,
            kind: self.kind,
            status: "queued".to_string(),
            title: self.title,
            project_root: self.project_root,
            owner_session_id: self.owner_session_id,
            parent_task_id: self.parent_task_id,
            parent_tool_call_id: self.parent_tool_call_id,
            child_session_id: self.child_session_id,
            agent_type: self.agent_type,
            prompt: self.prompt,
            progress_summary: self.initial_progress_summary,
            last_activity: self.initial_activity,
            result_summary: None,
            error: None,
            output_path: self.output_path,
            unread: self.initially_unread,
            cancel_requested: false,
            cancel_requested_at_ms: None,
            worker_id: None,
            ownership_token: None,
            lease_until_ms: None,
            heartbeat_at_ms: None,
            retry_count: 0,
            retry_at_ms: None,
            last_error: None,
            input_watermark: self.input_watermark,
            completion_watermark: None,
            created_at_ms: self.created_at_ms,
            started_at_ms: None,
            ended_at_ms: None,
            updated_at_ms: self.created_at_ms,
            metadata_json: serde_json::to_string(&self.metadata)
                .map_err(|error| format!("encode runtime task metadata failed: {error}"))?,
        };
        task.set_scheduling(self.scheduling)?;
        Ok(task)
    }
}

impl TryFrom<RuntimeTask> for CreateRuntimeTask {
    type Error = String;

    fn try_from(task: RuntimeTask) -> Result<Self, Self::Error> {
        if task.status != "queued" {
            return Err("runtime task creation requires queued status".into());
        }
        if task.cancel_requested
            || task.cancel_requested_at_ms.is_some()
            || task.worker_id.is_some()
            || task.ownership_token.is_some()
            || task.lease_until_ms.is_some()
            || task.heartbeat_at_ms.is_some()
            || task.retry_count != 0
            || task.retry_at_ms.is_some()
            || task.last_error.is_some()
            || task.completion_watermark.is_some()
            || task.started_at_ms.is_some()
            || task.ended_at_ms.is_some()
            || task.result_summary.is_some()
            || task.error.is_some()
        {
            return Err("runtime task creation contains runtime-owned state".into());
        }
        if task.updated_at_ms != task.created_at_ms {
            return Err("runtime task creation timestamps must match".into());
        }
        let scheduling = task
            .scheduling()?
            .ok_or_else(|| "runtime task creation requires scheduling metadata".to_string())?;
        let mut metadata: Value = serde_json::from_str(&task.metadata_json)
            .map_err(|error| format!("invalid runtime task metadata: {error}"))?;
        let metadata_object = metadata
            .as_object_mut()
            .ok_or_else(|| "runtime task metadata must be a JSON object".to_string())?;
        metadata_object.remove(SCHEDULING_METADATA_KEY);
        let input = Self {
            id: task.id,
            kind: task.kind,
            title: task.title,
            project_root: task.project_root,
            owner_session_id: task.owner_session_id,
            parent_task_id: task.parent_task_id,
            parent_tool_call_id: task.parent_tool_call_id,
            child_session_id: task.child_session_id,
            agent_type: task.agent_type,
            prompt: task.prompt,
            initial_progress_summary: task.progress_summary,
            initial_activity: task.last_activity,
            output_path: task.output_path,
            initially_unread: task.unread,
            input_watermark: task.input_watermark,
            metadata,
            scheduling,
            created_at_ms: task.created_at_ms,
        };
        input.validate()?;
        Ok(input)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeTask {
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

impl RuntimeTask {
    pub fn scheduling(&self) -> Result<Option<RuntimeTaskScheduling>, String> {
        let metadata: Value = serde_json::from_str(&self.metadata_json)
            .map_err(|error| format!("invalid runtime task metadata: {error}"))?;
        let object = metadata
            .as_object()
            .ok_or_else(|| "runtime task metadata must be a JSON object".to_string())?;
        let Some(value) = object.get(SCHEDULING_METADATA_KEY) else {
            return Ok(None);
        };
        let scheduling: RuntimeTaskScheduling = serde_json::from_value(value.clone())
            .map_err(|error| format!("invalid runtime task scheduling metadata: {error}"))?;
        scheduling.validate()?;
        Ok(Some(scheduling))
    }

    pub fn set_scheduling(&mut self, scheduling: RuntimeTaskScheduling) -> Result<(), String> {
        scheduling.validate()?;
        let mut metadata: Value = serde_json::from_str(&self.metadata_json)
            .map_err(|error| format!("invalid runtime task metadata: {error}"))?;
        let object = metadata
            .as_object_mut()
            .ok_or_else(|| "runtime task metadata must be a JSON object".to_string())?;
        object.insert(
            SCHEDULING_METADATA_KEY.into(),
            serde_json::to_value(scheduling)
                .map_err(|error| format!("encode runtime task scheduling failed: {error}"))?,
        );
        self.metadata_json = metadata.to_string();
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeTaskEvent {
    pub id: i64,
    pub task_id: String,
    pub kind: String,
    pub status: String,
    pub summary: Option<String>,
    pub detail_json: String,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RuntimeTaskProgressAck {
    pub task: RuntimeTask,
    pub attempt: RuntimeTaskAttempt,
}

/// Final worker facts committed with the terminal receipt, never as a separate
/// best-effort progress command. Agent Runs seal through their own authority.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RuntimeTaskFinalObservation {
    pub output_bytes: Option<u64>,
    pub resource_usage: Option<RuntimeTaskResourceUsage>,
    pub metadata_patch: BTreeMap<String, Option<Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum RuntimeTaskCommand {
    AdvanceTerminalInput { lease: RuntimeTaskLeaseWitness,
        previous: nola_domain::terminal_input::TerminalInputReceipt,
        next: nola_domain::terminal_input::TerminalInputReceipt, now_ms: i64 },
    SubmitTerminalInput { input: nola_domain::terminal_input::TerminalInputRequest, now_ms: i64 },
    Create {
        input: CreateRuntimeTask,
    },
    Get {
        id: String,
    },
    RunBinding {
        task_id: String,
    },
    IsChildSession {
        child_session_id: String,
    },
    TerminalReceipt {
        task_id: String,
    },
    PendingTaskGuidance {
        task_id: String,
        expected_run_id: String,
    },
    ReconcileBoundRunTerminal {
        task_id: String,
        expected_run_id: String,
        now_ms: i64,
    },
    ListProject {
        project_root: String,
    },
    ListAll,
    Claim {
        task_id: String,
        worker_id: String,
        worker_generation: String,
        lease_token: String,
        now_ms: i64,
        lease_until_ms: i64,
    },
    ClaimFair {
        task_id: String,
        worker_id: String,
        worker_generation: String,
        lease_token: String,
        now_ms: i64,
        lease_until_ms: i64,
        max_running_global: u32,
        max_running_project: u32,
    },
    SuspendForPermission {
        input: SuspendRuntimeTaskForPermission,
    },
    SuspendForCommandReconciliation {
        input: SuspendRuntimeTaskForCommandReconciliation,
    },
    PendingCommandReconciliation {
        task_id: String,
    },
    ResumeAfterPermissionFair {
        input: ResumeRuntimeTaskAfterPermissionFair,
    },
    RecoverPermissionContinuationFair {
        input: RecoverRuntimeTaskPermissionContinuationFair,
    },
    PermissionContinuationGrant {
        task_id: String,
        suspension_id: String,
        selector: PermissionContinuationGrantSelector,
    },
    CancelPermissionSuspension {
        input: CancelRuntimeTaskPermissionSuspension,
    },
    PendingPermissionSuspension {
        task_id: String,
    },
    ObservePermissionWaitBudget {
        task_id: String,
        suspension_id: String,
        expected_suspension_revision: i64,
        now_ms: i64,
    },
    LatestPermissionSuspension {
        task_id: String,
    },
    Attempt {
        attempt_id: String,
    },
    CurrentAttempt {
        task_id: String,
    },
    AgentAuthority {
        task_id: String,
    },
    PreparedAgentTaskIds {
        owner_conversation_id: String,
        after_id: String,
        limit: usize,
    },
    SourceAgentTaskAdmissionLocation {
        owner_conversation_id: String,
        task_id: String,
    },
    PermissionResume {
        task_id: String,
        suspension_id: String,
        permission_request_id: String,
        continuation_id: String,
    },
    RenewLease {
        task_id: String,
        attempt_id: String,
        expected_attempt_revision: i64,
        worker_generation: String,
        lease_token: String,
        current_expiry_ms: i64,
        now_ms: i64,
        requested_expiry_ms: i64,
    },
    ReportProgress {
        task_id: String,
        attempt_id: String,
        expected_attempt_revision: i64,
        worker_generation: String,
        lease_token: String,
        current_expiry_ms: i64,
        progress_summary: Option<String>,
        last_activity: Option<String>,
        output_bytes: Option<u64>,
        resource_usage: Option<RuntimeTaskResourceUsage>,
        metadata_patch: BTreeMap<String, Option<Value>>,
        now_ms: i64,
    },
    ReportActivity {
        lease: RuntimeTaskLeaseWitness,
        summary: String,
        activity: String,
        detail_json: String,
        now_ms: i64,
    },
    Retry {
        lease: RuntimeTaskLeaseWitness,
        retry_at_ms: i64,
        error: String,
        now_ms: i64,
    },
    FinishDetailedWithEvent {
        lease: RuntimeTaskLeaseWitness,
        status: String,
        completion_watermark: Option<i64>,
        progress_summary: Option<String>,
        result_summary: Option<String>,
        error: Option<String>,
        output_path: Option<String>,
        unread: bool,
        event_kind: String,
        event_status: String,
        event_summary: Option<String>,
        event_detail_json: String,
        final_observation: Option<RuntimeTaskFinalObservation>,
        now_ms: i64,
    },
    NotificationDelivered { delivery: RuntimeTaskNotificationDelivery },
    AcknowledgeDelivery { delivery: RuntimeTaskNotificationDelivery },
    AcknowledgeModelDelivery {
        task_id: String,
    },
    AcknowledgeResult {
        input: AcknowledgeRuntimeTaskResult,
    },
    ReadOwnedCancellation { input: crate::owned_task_cancellation::OwnedTaskCancellation },
    RequestOwnedCancellation { input: crate::owned_task_cancellation::OwnedTaskCancellation, now_ms: i64 },
    RequestCancel {
        task_id: String,
        now_ms: i64,
    },
    FailExpired {
        task_id: String,
        error: String,
        now_ms: i64,
    },
    FailExpiredWithEvent {
        task_id: String,
        error: String,
        event_kind: String,
        event_status: String,
        event_summary: Option<String>,
        event_detail_json: String,
        now_ms: i64,
    },
    RecoverExpired {
        task_id: String,
        error: String,
        retry_at_ms: i64,
        now_ms: i64,
    },
    AppendEvent {
        task_id: String,
        kind: String,
        status: String,
        summary: Option<String>,
        detail_json: String,
        created_at_ms: i64,
    },
    LatestEventCursor,
    AttemptRetiredForUserInput { task_id: String, attempt_id: String },
    /// Observed exit of the exact current worker; not a Task/Run terminal.
    CurrentWorkerExit { task_id: String },
    ListEvents {
        task_id: String,
        limit: usize,
    },
    EventsAfterSessionCursor {
        owner_session_id: String,
        after_cursor: i64,
        limit: usize,
    },
}

pub(crate) fn execute_runtime_task_command<R: RuntimeTaskRepository + ?Sized>(
    store: &mut R,
    command: RuntimeTaskCommand,
) -> Result<Value, String> {
    match command {
        RuntimeTaskCommand::AdvanceTerminalInput { lease, previous, next, now_ms } => {
            encode(report_runtime_task_progress(store,&lease.task_id,&lease.attempt_id,lease.expected_attempt_revision,
                &lease.worker_generation,&lease.lease_token,lease.current_expiry_ms,None,None,None,None,
                BTreeMap::new(),now_ms,None,Some((previous,next)))?)
        },
        RuntimeTaskCommand::SubmitTerminalInput { input, now_ms } => {
            input.prepared_receipt()?;
            if now_ms < 0 { return Err("invalid terminal input submission time".into()); }
            encode(store.submit_terminal_input(&input, now_ms).map_err(|error| error.to_string())?)
        },
        RuntimeTaskCommand::NotificationDelivered { delivery } => encode(store.runtime_task_notification_delivered(&delivery).map_err(|e| e.to_string())?),
        RuntimeTaskCommand::AcknowledgeDelivery { delivery } => encode(store.acknowledge_runtime_task_delivery(&delivery).map_err(|e| e.to_string())?),
        RuntimeTaskCommand::PreparedAgentTaskIds { owner_conversation_id, after_id, limit } =>
            encode(store.prepared_agent_task_ids(&owner_conversation_id, &after_id, limit).map_err(|e| e.to_string())?),
        RuntimeTaskCommand::SourceAgentTaskAdmissionLocation { owner_conversation_id, task_id } =>
            encode(store.source_agent_task_admission_location(&owner_conversation_id, &task_id).map_err(|e| e.to_string())?),
        RuntimeTaskCommand::PendingTaskGuidance {
            task_id,
            expected_run_id,
        } => encode(
            store
                .pending_runtime_task_guidance(&task_id, &expected_run_id)
                .map_err(|e| e.to_string())?,
        ),
        RuntimeTaskCommand::TerminalReceipt { task_id } => encode(
            store
                .runtime_task_terminal_receipt(&task_id)
                .map_err(|error| error.to_string())?,
        ),
        RuntimeTaskCommand::RunBinding { task_id } => encode(
            store
                .runtime_task_run_binding(&task_id)
                .map_err(|error| error.to_string())?,
        ),
        RuntimeTaskCommand::IsChildSession { child_session_id } => encode(
            store
                .is_runtime_task_child_session(&child_session_id)
                .map_err(|error| error.to_string())?,
        ),
        RuntimeTaskCommand::ReconcileBoundRunTerminal {
            task_id,
            expected_run_id,
            now_ms,
        } => encode(
            store
                .reconcile_runtime_task_bound_run_terminal(&task_id, &expected_run_id, now_ms)
                .map_err(|error| error.to_string())?,
        ),
        RuntimeTaskCommand::Create { input } => {
            if input.metadata.get("delegation_origin").is_some() { return Err("Delegation origin requires exact Core preparation".into()); }
            input.validate()?;
            let task = input.into_task()?;
            store
                .create_runtime_task_with_event(&stored_task(task.clone()))
                .map_err(|error| format!("create runtime task failed: {error}"))?;
            encode(task)
        }
        RuntimeTaskCommand::Get { id } => encode(
            store
                .runtime_task(&id)
                .map_err(|error| format!("load runtime task failed: {error}"))?
                .map(runtime_task),
        ),
        RuntimeTaskCommand::ListProject { project_root } => encode(
            store
                .list_runtime_tasks(&project_root)
                .map_err(|error| format!("list runtime tasks failed: {error}"))?
                .into_iter()
                .map(runtime_task)
                .collect::<Vec<_>>(),
        ),
        RuntimeTaskCommand::ListAll => encode(
            store
                .list_all_runtime_tasks()
                .map_err(|error| format!("list all runtime tasks failed: {error}"))?
                .into_iter()
                .map(runtime_task)
                .collect::<Vec<_>>(),
        ),
        RuntimeTaskCommand::Claim {
            task_id,
            worker_id,
            worker_generation,
            lease_token,
            now_ms,
            lease_until_ms,
        } => {
            validate_runtime_task_claim_identity(&worker_id, &worker_generation, &lease_token)?;
            Ok(json!(store
                .claim_runtime_task(
                    &task_id,
                    &worker_id,
                    &worker_generation,
                    &lease_token,
                    now_ms,
                    lease_until_ms
                )
                .map_err(|error| format!(
                    "claim runtime task failed: {error}"
                ))?))
        }
        RuntimeTaskCommand::ClaimFair {
            task_id,
            worker_id,
            worker_generation,
            lease_token,
            now_ms,
            lease_until_ms,
            max_running_global,
            max_running_project,
        } => {
            validate_runtime_task_claim_identity(&worker_id, &worker_generation, &lease_token)?;
            Ok(json!(store
                .claim_runtime_task_fair(
                    &task_id,
                    &worker_id,
                    &worker_generation,
                    &lease_token,
                    now_ms,
                    lease_until_ms,
                    max_running_global,
                    max_running_project,
                )
                .map_err(|error| format!(
                    "fair claim runtime task failed: {error}"
                ))?))
        }
        RuntimeTaskCommand::SuspendForPermission { input } => encode(
            store
                .suspend_runtime_task_for_permission(&input)
                .map_err(|e| format!("suspend runtime task for permission failed: {e}"))?,
        ),
        RuntimeTaskCommand::ObservePermissionWaitBudget {
            task_id, suspension_id, expected_suspension_revision, now_ms,
        } => encode(observe_permission_wait_budget(store, &task_id, &suspension_id,
            expected_suspension_revision, now_ms)?),
        RuntimeTaskCommand::SuspendForCommandReconciliation { input } => encode(
            store
                .suspend_runtime_task_for_command_reconciliation(&input)
                .map_err(|error| {
                    format!("suspend runtime task for command reconciliation failed: {error}")
                })?,
        ),
        RuntimeTaskCommand::PendingCommandReconciliation { task_id } => encode(
            store
                .pending_runtime_task_command_reconciliation(&task_id)
                .map_err(|error| {
                    format!("load pending runtime task command reconciliation failed: {error}")
                })?,
        ),
        RuntimeTaskCommand::ResumeAfterPermissionFair { input } => encode(
            store
                .resume_runtime_task_after_permission_fair(&input)
                .map_err(|e| format!("resume runtime task after permission failed: {e}"))?,
        ),
        RuntimeTaskCommand::RecoverPermissionContinuationFair { input } => encode(
            store
                .recover_runtime_task_permission_continuation_fair(&input)
                .map_err(|e| format!("recover exact permission continuation failed: {e}"))?,
        ),
        RuntimeTaskCommand::PermissionContinuationGrant {
            task_id,
            suspension_id,
            selector,
        } => encode(
            store
                .runtime_task_permission_continuation_grant(&task_id, &suspension_id, &selector)
                .map_err(|e| format!("read permission continuation grant failed: {e}"))?,
        ),
        RuntimeTaskCommand::PermissionResume {
            task_id,
            suspension_id,
            permission_request_id,
            continuation_id,
        } => encode(
            store
                .runtime_task_permission_resume(
                    &task_id,
                    &suspension_id,
                    &permission_request_id,
                    &continuation_id,
                )
                .map_err(|e| format!("read exact permission resume failed: {e}"))?,
        ),
        RuntimeTaskCommand::CancelPermissionSuspension { input } => encode(
            store
                .cancel_runtime_task_permission_suspension(&input)
                .map_err(|e| format!("cancel runtime task permission suspension failed: {e}"))?,
        ),
        RuntimeTaskCommand::PendingPermissionSuspension { task_id } => encode(
            store
                .pending_runtime_task_permission_suspension(&task_id)
                .map_err(|e| {
                    format!("load pending runtime task permission suspension failed: {e}")
                })?,
        ),
        RuntimeTaskCommand::LatestPermissionSuspension { task_id } => encode(
            store
                .latest_runtime_task_permission_suspension(&task_id)
                .map_err(|e| format!("load latest permission suspension failed: {e}"))?,
        ),
        RuntimeTaskCommand::CurrentAttempt { task_id } => encode(
            store
                .runtime_task_current_attempt(&task_id)
                .map_err(|error| error.to_string())?,
        ),
        RuntimeTaskCommand::AgentAuthority { task_id } => encode(
            store
                .runtime_agent_task_authority(&task_id)
                .map_err(|error| error.to_string())?,
        ),
        RuntimeTaskCommand::Attempt { attempt_id } => encode(
            store
                .runtime_task_attempt(&attempt_id)
                .map_err(|e| format!("load exact runtime task attempt failed: {e}"))?,
        ),
        RuntimeTaskCommand::RenewLease {
            task_id,
            attempt_id,
            expected_attempt_revision,
            worker_generation,
            lease_token,
            current_expiry_ms,
            now_ms,
            requested_expiry_ms,
        } => encode(renew_runtime_task_lease(
            store,
            RenewRuntimeTaskLease {
                task_id,
                attempt_id,
                expected_attempt_revision,
                worker_generation,
                lease_token,
                current_expiry_ms,
                now_ms,
                requested_expiry_ms,
            },
        )?),
        RuntimeTaskCommand::ReportProgress {
            task_id,
            attempt_id,
            expected_attempt_revision,
            worker_generation,
            lease_token,
            current_expiry_ms,
            progress_summary,
            last_activity,
            output_bytes,
            resource_usage,
            metadata_patch,
            now_ms,
        } => encode(report_runtime_task_progress(
            store,
            &task_id,
            &attempt_id,
            expected_attempt_revision,
            &worker_generation,
            &lease_token,
            current_expiry_ms,
            progress_summary,
            last_activity,
            output_bytes,
            resource_usage,
            metadata_patch,
            now_ms,
            None,
            None,
        )?),
        RuntimeTaskCommand::ReportActivity { lease, summary, activity, detail_json, now_ms } => {
            if summary.trim().is_empty() || summary.chars().count()>4096 {
                return Err("runtime activity summary must contain 1 to 4096 characters".into());
            }
            serde_json::from_str::<Value>(&detail_json).map_err(|error|format!("invalid runtime activity detail: {error}"))?;
            encode(report_runtime_task_progress(store,&lease.task_id,&lease.attempt_id,lease.expected_attempt_revision,
                &lease.worker_generation,&lease.lease_token,lease.current_expiry_ms,
                Some(summary.clone()),Some(activity),None,None,BTreeMap::new(),now_ms,
                Some(nola_store_port::runtime_task::RuntimeTaskProgressActivity {summary,detail_json}),None)?)
        },
        RuntimeTaskCommand::Retry {
            lease,
            retry_at_ms,
            error,
            now_ms,
        } => Ok(json!(store
            .retry_runtime_task(&lease, retry_at_ms, &error, now_ms)
            .map_err(|store_error| format!(
                "retry runtime task failed: {store_error}"
            ))?)),
        RuntimeTaskCommand::FinishDetailedWithEvent {
            lease,
            status,
            completion_watermark,
            progress_summary,
            result_summary,
            error,
            output_path,
            unread,
            event_kind,
            event_status,
            event_summary,
            event_detail_json,
            final_observation,
            now_ms,
        } => {
            let final_progress = final_observation.map(|observation| {
                let mut task = runtime_task(store.runtime_task(&lease.task_id)
                    .map_err(|error| format!("load final observation task failed: {error}"))?
                    .ok_or("final observation task is absent")?);
                if task.status != "running" || task.ownership_token.as_deref() != Some(&lease.lease_token)
                    || task.lease_until_ms != Some(lease.current_expiry_ms)
                    || lease.current_expiry_ms <= now_ms || task.updated_at_ms > now_ms {
                    return Err("final observation task ownership is stale".to_string());
                }
                let expected_metadata_json = task.metadata_json.clone();
                apply_runtime_task_observation(&mut task, observation.output_bytes,
                    observation.resource_usage, observation.metadata_patch, now_ms)?;
                Ok(RuntimeTaskFinalProgress {
                    expected_updated_at_ms: task.updated_at_ms,
                    expected_metadata_json,
                    metadata_json: task.metadata_json,
                })
            }).transpose()?;
            Ok(json!(store
            .finish_runtime_task_detailed_with_event(
                &lease,
                &status,
                completion_watermark,
                progress_summary.as_deref(),
                result_summary.as_deref(),
                error.as_deref(),
                output_path.as_deref(),
                unread,
                &event_kind,
                &event_status,
                event_summary.as_deref(),
                &event_detail_json,
                final_progress.as_ref(),
                now_ms,
            )
            .map_err(|error| format!(
                "finish detailed runtime task with event failed: {error}"
            ))?))
        },
        RuntimeTaskCommand::AcknowledgeModelDelivery { task_id } => Ok(json!(store
            .acknowledge_runtime_task_model_delivery(&task_id)
            .map_err(|error| format!(
                "acknowledge runtime task model delivery failed: {error}"
            ))?)),
        RuntimeTaskCommand::AcknowledgeResult { input } => Ok(json!(store
            .acknowledge_runtime_task_result(&input)
            .map_err(|error| format!("acknowledge exact runtime task result failed: {error}"))?)),
        RuntimeTaskCommand::ReadOwnedCancellation { input } =>
            Ok(json!(crate::owned_task_cancellation::read(store, &input)?)),
        RuntimeTaskCommand::RequestOwnedCancellation { input, now_ms } =>
            Ok(json!(crate::owned_task_cancellation::request(store, &input, now_ms)?)),
        RuntimeTaskCommand::RequestCancel { task_id, now_ms } => Ok(json!(store
            .request_runtime_task_cancel(&task_id, now_ms)
            .map_err(|error| format!("request runtime task cancellation failed: {error}"))?)),
        RuntimeTaskCommand::FailExpired {
            task_id,
            error,
            now_ms,
        } => Ok(json!(store
            .fail_expired_runtime_task(&task_id, &error, now_ms)
            .map_err(|store_error| format!(
                "fail expired runtime task failed: {store_error}"
            ))?)),
        RuntimeTaskCommand::FailExpiredWithEvent {
            task_id,
            error,
            event_kind,
            event_status,
            event_summary,
            event_detail_json,
            now_ms,
        } => Ok(json!(store
            .fail_expired_runtime_task_with_event(
                &task_id,
                &error,
                &event_kind,
                &event_status,
                event_summary.as_deref(),
                &event_detail_json,
                now_ms,
            )
            .map_err(|store_error| format!(
                "fail expired runtime task with event failed: {store_error}"
            ))?)),
        RuntimeTaskCommand::RecoverExpired {
            task_id,
            error,
            retry_at_ms,
            now_ms,
        } => Ok(json!(store
            .recover_expired_runtime_task(&task_id, &error, retry_at_ms, now_ms)
            .map_err(|store_error| format!(
                "recover expired runtime task failed: {store_error}"
            ))?)),
        RuntimeTaskCommand::AppendEvent {
            task_id,
            kind,
            status,
            summary,
            detail_json,
            created_at_ms,
        } => Ok(json!(store
            .append_runtime_task_event(
                &task_id,
                &kind,
                &status,
                summary.as_deref(),
                &detail_json,
                created_at_ms
            )
            .map_err(|error| format!(
                "append runtime task event failed: {error}"
            ))?)),
        RuntimeTaskCommand::AttemptRetiredForUserInput { task_id, attempt_id } => encode(
            store.runtime_task_attempt_retired_for_user_input(&task_id, &attempt_id).map_err(|e| e.to_string())?),
        RuntimeTaskCommand::LatestEventCursor => Ok(json!(store
            .latest_runtime_task_event_id()
            .map_err(|error| format!("load latest runtime task event cursor failed: {error}"))?)),
        RuntimeTaskCommand::CurrentWorkerExit { task_id } => {
            let attempt = store.runtime_task_current_attempt(&task_id)
                .map_err(|error| format!("load current worker attempt failed: {error}"))?;
            let exit = if let Some(attempt) = attempt {
                store.runtime_task_events(&task_id, 8)
                    .map_err(|error| format!("load worker exit observation failed: {error}"))?
                    .into_iter().find(|event| current_worker_exit(&attempt.id, event))
                    .map(|event| event.summary.unwrap_or_else(|| "worker exited".into()))
            } else { None };
            encode(exit)
        },
        RuntimeTaskCommand::ListEvents { task_id, limit } => encode(
            store
                .runtime_task_events(&task_id, limit)
                .map_err(|error| format!("load runtime task events failed: {error}"))?
                .into_iter()
                .map(runtime_task_event)
                .collect::<Vec<_>>(),
        ),
        RuntimeTaskCommand::EventsAfterSessionCursor {
            owner_session_id,
            after_cursor,
            limit,
        } => encode(
            store
                .runtime_task_events_after_session_cursor(&owner_session_id, after_cursor, limit)
                .map_err(|error| format!("load runtime task event cursor failed: {error}"))?
                .into_iter()
                .map(runtime_task_event)
                .collect::<Vec<_>>(),
        ),
    }
}

fn current_worker_exit(attempt_id: &str, event: &nola_store_port::runtime_task::StoredRuntimeTaskEvent) -> bool {
    event.kind == "worker_process_exited" && serde_json::from_str::<Value>(&event.detail_json)
        .ok().is_some_and(|detail| detail["attempt_id"].as_str() == Some(attempt_id))
}

#[test]
fn worker_exit_observation_does_not_follow_a_replacement_attempt() {
    let event = nola_store_port::runtime_task::StoredRuntimeTaskEvent {
        id: 1, task_id: "task".into(), kind: "worker_process_exited".into(),
        status: "suspended".into(), summary: Some("worker failed".into()),
        detail_json: r#"{"attempt_id":"old-worker"}"#.into(), created_at_ms: 100,
    };
    assert!(current_worker_exit("old-worker", &event));
    assert!(!current_worker_exit("new-worker", &event));
}

fn validate_runtime_task_claim_identity(
    worker_id: &str,
    worker_generation: &str,
    lease_token: &str,
) -> Result<(), String> {
    for (label, value) in [
        ("worker_id", worker_id),
        ("worker_generation", worker_generation),
        ("lease_token", lease_token),
    ] {
        if value.trim().is_empty()
            || value.trim() != value
            || value.chars().any(char::is_control)
            || value.chars().count() > 128
        {
            return Err(format!(
                "runtime task claim {label} is not a canonical identity"
            ));
        }
    }
    Ok(())
}

fn renew_runtime_task_lease<R: RuntimeTaskRepository + ?Sized>(
    store: &mut R,
    renewal: RenewRuntimeTaskLease,
) -> Result<RuntimeTaskAttempt, String> {
    validate_runtime_task_claim_identity(
        &renewal.attempt_id,
        &renewal.worker_generation,
        &renewal.lease_token,
    )?;
    if renewal.expected_attempt_revision < 1 {
        return Err("runtime task lease renewal revision must be positive".into());
    }
    if renewal.current_expiry_ms <= renewal.now_ms {
        return Err("runtime task lease has expired".into());
    }
    if renewal.requested_expiry_ms <= renewal.current_expiry_ms {
        return Err("runtime task lease renewal must strictly extend expiry".into());
    }
    if renewal
        .requested_expiry_ms
        .checked_sub(renewal.now_ms)
        .is_none_or(|extension| extension > 10 * 60 * 1_000)
    {
        return Err("runtime task lease renewal exceeds the 10 minute bound".into());
    }
    let attempt = store
        .runtime_task_attempt(&renewal.attempt_id)
        .map_err(|error| format!("load runtime task attempt for renewal failed: {error}"))?
        .ok_or_else(|| "runtime task lease attempt does not exist".to_string())?;
    if attempt.task_id != renewal.task_id
        || attempt.worker_generation != renewal.worker_generation
        || attempt.lease_token != renewal.lease_token
        || attempt.lease_expires_at_ms != renewal.current_expiry_ms
        || attempt.revision != renewal.expected_attempt_revision
        || attempt.state != "leased"
    {
        return Err("runtime task lease renewal identity is stale or foreign".into());
    }
    store
        .renew_runtime_task_lease(&renewal)
        .map_err(|error| format!("renew runtime task lease failed: {error}"))?
        .ok_or_else(|| "runtime task lease renewal lost its compare-and-swap race".to_string())
}

#[allow(clippy::too_many_arguments)]
fn report_runtime_task_progress<R: RuntimeTaskRepository + ?Sized>(
    store: &mut R,
    task_id: &str,
    attempt_id: &str,
    expected_attempt_revision: i64,
    worker_generation: &str,
    lease_token: &str,
    current_expiry_ms: i64,
    progress_summary: Option<String>,
    last_activity: Option<String>,
    output_bytes: Option<u64>,
    resource_usage: Option<RuntimeTaskResourceUsage>,
    metadata_patch: BTreeMap<String, Option<Value>>,
    now_ms: i64,
    activity: Option<nola_store_port::runtime_task::RuntimeTaskProgressActivity>,
    terminal_transition: Option<(nola_domain::terminal_input::TerminalInputReceipt,
        nola_domain::terminal_input::TerminalInputReceipt)>,
) -> Result<RuntimeTaskProgressAck, String> {
    validate_runtime_task_claim_identity(attempt_id, worker_generation, lease_token)?;
    if expected_attempt_revision < 1 {
        return Err("runtime progress attempt revision must be positive".into());
    }
    if current_expiry_ms <= now_ms {
        return Err("runtime progress lease has expired".into());
    }
    let attempt = store
        .runtime_task_attempt(attempt_id)
        .map_err(|error| format!("load runtime task attempt for progress failed: {error}"))?
        .ok_or_else(|| "runtime progress attempt does not exist".to_string())?;
    if attempt.task_id != task_id
        || attempt.revision != expected_attempt_revision
        || attempt.worker_generation != worker_generation
        || attempt.lease_token != lease_token
        || attempt.lease_expires_at_ms != current_expiry_ms
        || attempt.state != "leased"
    {
        return Err("runtime progress attempt identity is stale or foreign".into());
    }
    if progress_summary
        .as_deref()
        .is_some_and(|value| value.trim().is_empty() || value.chars().count() > 4_096)
    {
        return Err("runtime progress summary must contain 1 to 4096 characters".into());
    }
    if last_activity
        .as_deref()
        .is_some_and(|value| value.chars().count() > 1_024)
    {
        return Err("runtime task activity exceeds 1024 characters".into());
    }
    // A clock-only observation is meaningful: elapsed wall time may exhaust
    // the durable budget even when no output or resource sample changed.
    // It still requires the same active-attempt CAS as every progress write.
    let stored = store
        .runtime_task(task_id)
        .map_err(|error| format!("load runtime task for progress failed: {error}"))?
        .ok_or_else(|| format!("runtime task {task_id} does not exist"))?;
    let mut task = runtime_task(stored);
    let expected_metadata_json = task.metadata_json.clone();
    if task.status != "running" {
        return Err(format!(
            "runtime task {task_id} cannot report progress from state {}",
            task.status
        ));
    }
    if task.ownership_token.as_deref() != Some(lease_token) {
        return Err(format!("runtime task {task_id} lease token mismatch"));
    }
    let lease_until_ms = task
        .lease_until_ms
        .ok_or_else(|| format!("runtime task {task_id} has no active lease"))?;
    if lease_until_ms <= now_ms {
        return Err(format!("runtime task {task_id} lease has expired"));
    }
    if lease_until_ms != current_expiry_ms {
        return Err(format!("runtime task {task_id} lease projection is stale"));
    }
    if now_ms < task.updated_at_ms {
        return Err(format!(
            "runtime task {task_id} progress timestamp moved backwards"
        ));
    }

    apply_runtime_task_observation(&mut task, output_bytes, resource_usage, metadata_patch, now_ms)?;
    if let Some((previous, next)) = terminal_transition {
        use nola_domain::terminal_input::TERMINAL_INPUT_RECEIPTS_METADATA_KEY as KEY;
        next.follows(Some(&previous))?;
        if previous == next { return Err("terminal transition must advance, not replay a write witness".into()); }
        let mut metadata: Value = serde_json::from_str(&task.metadata_json).map_err(|error|error.to_string())?;
        let key = &previous.identity.request_id;
        let current: nola_domain::terminal_input::TerminalInputReceipt = serde_json::from_value(
            metadata.get(KEY).and_then(|receipts|receipts.get(key)).cloned().ok_or("terminal receipt not found")?)
            .map_err(|error|error.to_string())?;
        if current != previous { return Err("terminal receipt changed before write; do not replay".into()); }
        if next.phase == nola_domain::terminal_input::TerminalInputPhase::Delivered {
            // The receipt retains identity/digest for deduplication; input
            // bytes are needed only until delivery has been acknowledged.
            let queue = metadata.get_mut(nola_domain::terminal_input::TERMINAL_INPUT_QUEUE_METADATA_KEY)
                .and_then(Value::as_object_mut).ok_or("terminal input queue not found")?;
            if queue.remove(key).is_none() { return Err("terminal input payload not found".into()); }
        }
        metadata[KEY][key] = serde_json::to_value(next).map_err(|error|error.to_string())?;
        task.metadata_json = metadata.to_string();
    }
    if let Some(progress_summary) = progress_summary {
        task.progress_summary = Some(progress_summary.trim().to_string());
    }
    if let Some(last_activity) = last_activity {
        task.last_activity = Some(last_activity);
    }
    let mutation = RuntimeTaskProgressMutation {
        task_id: task_id.into(),
        attempt_id: attempt_id.into(),
        expected_attempt_revision,
        worker_generation: worker_generation.into(),
        lease_token: lease_token.into(),
        current_expiry_ms,
        expected_updated_at_ms: task.updated_at_ms,
        expected_metadata_json,
        progress_summary: task.progress_summary.clone(),
        last_activity: task.last_activity.clone(),
        activity,
        metadata_json: task.metadata_json.clone(),
        now_ms,
    };
    let attempt = store
        .report_runtime_task_progress(&mutation)
        .map_err(|error| format!("report runtime task progress failed: {error}"))?
        .ok_or_else(|| format!("runtime task {task_id} progress lost its active attempt CAS"))?;
    task.updated_at_ms = now_ms;
    Ok(RuntimeTaskProgressAck { task, attempt })
}

fn apply_runtime_task_observation(
    task: &mut RuntimeTask,
    output_bytes: Option<u64>,
    resource_usage: Option<RuntimeTaskResourceUsage>,
    metadata_patch: BTreeMap<String, Option<Value>>,
    now_ms: i64,
) -> Result<(), String> {
    if metadata_patch.len() > 64 {
        return Err("runtime progress metadata patch exceeds 64 entries".into());
    }
    if metadata_patch.keys().any(|key| {
        key.trim().is_empty()
            || key.chars().count() > 128
            || matches!(key.as_str(), "delegation_origin" | "delegated_invocation")
            || key.as_str() == SCHEDULING_METADATA_KEY
            || key.as_str() == "_nola_command_reconciliation"
            || key.as_str() == nola_domain::terminal_input::TERMINAL_INPUT_QUEUE_METADATA_KEY
            || key.as_str() == nola_domain::terminal_input::TERMINAL_INPUT_RECEIPTS_METADATA_KEY
    }) {
        return Err("runtime progress metadata patch contains an invalid or reserved key".into());
    }
    let patch_bytes = serde_json::to_vec(&metadata_patch)
        .map_err(|error| format!("encode runtime progress metadata patch failed: {error}"))?;
    if patch_bytes.len() > 256 * 1_024 {
        return Err("runtime progress metadata patch exceeds 256 KiB".into());
    }
    let task_id = task.id.clone();
    let mut scheduling = task.scheduling()?
        .ok_or_else(|| format!("runtime task {task_id} has no scheduling policy"))?;
    if let Some(output_bytes) = output_bytes {
        if output_bytes < scheduling.output_bytes {
            return Err(format!(
                "runtime task {task_id} output byte count moved backwards"
            ));
        }
        scheduling.output_bytes = output_bytes;
    }
    if let Some(resource_usage) = resource_usage {
        if resource_usage.cpu_time_ms < scheduling.resource_usage.cpu_time_ms
            || resource_usage.peak_memory_bytes < scheduling.resource_usage.peak_memory_bytes
            || resource_usage.peak_processes < scheduling.resource_usage.peak_processes
        {
            return Err(format!(
                "runtime task {task_id} resource usage moved backwards"
            ));
        }
        scheduling.resource_usage = resource_usage;
    }
    scheduling.budget_exhaustion = scheduling
        .execution_budget_failure(task.started_at_ms, now_ms)?
        .map(|failure| failure.resource.code().to_string());
    task.set_scheduling(scheduling)?;

    let mut metadata: Value = serde_json::from_str(&task.metadata_json)
        .map_err(|error| format!("invalid runtime task metadata: {error}"))?;
    let object = metadata
        .as_object_mut()
        .ok_or_else(|| "runtime task metadata must be a JSON object".to_string())?;
    for (key, value) in metadata_patch {
        match value {
            Some(value) => {
                object.insert(key, value);
            }
            None => {
                object.remove(&key);
            }
        }
    }
    task.metadata_json = serde_json::to_string(&metadata)
        .map_err(|error| format!("encode runtime task metadata failed: {error}"))?;
    Ok(())
}

fn observe_permission_wait_budget<R: RuntimeTaskRepository + ?Sized>(
    store: &mut R,
    task_id: &str,
    suspension_id: &str,
    expected_suspension_revision: i64,
    now_ms: i64,
) -> Result<RuntimeTask, String> {
    if task_id.trim().is_empty() || suspension_id.trim().is_empty()
        || expected_suspension_revision < 1 || now_ms < 0 {
        return Err("permission budget observation identity is invalid".into());
    }
    let mut task = runtime_task(store.runtime_task(task_id)
        .map_err(|error| format!("load permission budget task failed: {error}"))?
        .ok_or("permission budget task is absent")?);
    let suspension = store.runtime_task_permission_suspension(suspension_id)
        .map_err(|error| format!("load permission budget suspension failed: {error}"))?
        .ok_or("permission budget suspension is absent")?;
    let attempt = store.runtime_task_attempt(&suspension.attempt_id)
        .map_err(|error| format!("load permission budget attempt failed: {error}"))?
        .ok_or("permission budget attempt is absent")?;
    if task.status != "waiting_permission" || task.cancel_requested
        || task.worker_id.is_some() || task.ownership_token.is_some() || task.lease_until_ms.is_some()
        || suspension.task_id != task_id || suspension.state != "pending"
        || suspension.revision != expected_suspension_revision
        || attempt.task_id != task_id || attempt.state != "permission_suspended" {
        return Err("permission budget observation has stale or foreign wait identity".into());
    }
    if now_ms < task.updated_at_ms || now_ms < suspension.suspended_at_ms {
        return Err("permission budget observation timestamp moved backwards".into());
    }
    let mut scheduling = task.scheduling()?.ok_or("permission budget task has no scheduling policy")?;
    let failure = scheduling.execution_budget_failure(task.started_at_ms, now_ms)?;
    if failure.is_none() || scheduling.budget_exhaustion.is_some() {
        // Read-only observations do not create archive generations or duplicate
        // budget events. The subsequent cancellation path revalidates the Run.
        return Ok(task);
    }
    let expected_metadata_json = task.metadata_json.clone();
    scheduling.budget_exhaustion = failure.map(|failure| failure.resource.code().to_owned());
    task.set_scheduling(scheduling)?;
    let changed = store.observe_runtime_task_permission_budget(&RuntimeTaskPermissionBudgetMutation {
        task_id: task_id.into(), suspension_id: suspension_id.into(), expected_suspension_revision,
        attempt_id: attempt.id, expected_attempt_revision: attempt.revision,
        expected_updated_at_ms: task.updated_at_ms, expected_started_at_ms: task.started_at_ms,
        expected_metadata_json, metadata_json: task.metadata_json.clone(), now_ms,
    }).map_err(|error| format!("commit permission budget observation failed: {error}"))?;
    if !changed { return Err("permission budget observation lost its wait CAS".into()); }
    task.updated_at_ms = now_ms;
    Ok(task)
}

fn encode<T: Serialize>(value: T) -> Result<Value, String> {
    serde_json::to_value(value)
        .map_err(|error| format!("encode runtime task result failed: {error}"))
}

pub(crate) fn stored_task(task: RuntimeTask) -> StoredRuntimeTask {
    StoredRuntimeTask {
        id: task.id,
        kind: task.kind,
        status: task.status,
        title: task.title,
        project_root: task.project_root,
        owner_session_id: task.owner_session_id,
        parent_task_id: task.parent_task_id,
        parent_tool_call_id: task.parent_tool_call_id,
        child_session_id: task.child_session_id,
        agent_type: task.agent_type,
        prompt: task.prompt,
        progress_summary: task.progress_summary,
        last_activity: task.last_activity,
        result_summary: task.result_summary,
        error: task.error,
        output_path: task.output_path,
        unread: task.unread,
        cancel_requested: task.cancel_requested,
        cancel_requested_at_ms: task.cancel_requested_at_ms,
        worker_id: task.worker_id,
        ownership_token: task.ownership_token,
        lease_until_ms: task.lease_until_ms,
        heartbeat_at_ms: task.heartbeat_at_ms,
        retry_count: task.retry_count,
        retry_at_ms: task.retry_at_ms,
        last_error: task.last_error,
        input_watermark: task.input_watermark,
        completion_watermark: task.completion_watermark,
        created_at_ms: task.created_at_ms,
        started_at_ms: task.started_at_ms,
        ended_at_ms: task.ended_at_ms,
        updated_at_ms: task.updated_at_ms,
        metadata_json: task.metadata_json,
    }
}

pub(crate) fn runtime_task(task: StoredRuntimeTask) -> RuntimeTask {
    RuntimeTask {
        id: task.id,
        kind: task.kind,
        status: task.status,
        title: task.title,
        project_root: task.project_root,
        owner_session_id: task.owner_session_id,
        parent_task_id: task.parent_task_id,
        parent_tool_call_id: task.parent_tool_call_id,
        child_session_id: task.child_session_id,
        agent_type: task.agent_type,
        prompt: task.prompt,
        progress_summary: task.progress_summary,
        last_activity: task.last_activity,
        result_summary: task.result_summary,
        error: task.error,
        output_path: task.output_path,
        unread: task.unread,
        cancel_requested: task.cancel_requested,
        cancel_requested_at_ms: task.cancel_requested_at_ms,
        worker_id: task.worker_id,
        ownership_token: task.ownership_token,
        lease_until_ms: task.lease_until_ms,
        heartbeat_at_ms: task.heartbeat_at_ms,
        retry_count: task.retry_count,
        retry_at_ms: task.retry_at_ms,
        last_error: task.last_error,
        input_watermark: task.input_watermark,
        completion_watermark: task.completion_watermark,
        created_at_ms: task.created_at_ms,
        started_at_ms: task.started_at_ms,
        ended_at_ms: task.ended_at_ms,
        updated_at_ms: task.updated_at_ms,
        metadata_json: task.metadata_json,
    }
}

fn runtime_task_event(event: StoredRuntimeTaskEvent) -> RuntimeTaskEvent {
    RuntimeTaskEvent {
        id: event.id,
        task_id: event.task_id,
        kind: event.kind,
        status: event.status,
        summary: event.summary,
        detail_json: event.detail_json,
        created_at_ms: event.created_at_ms,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(metadata_json: &str) -> RuntimeTask {
        RuntimeTask {
            id: "task-budget".into(),
            kind: "agent".into(),
            status: "queued".into(),
            title: "Budgeted task".into(),
            project_root: "/workspace".into(),
            owner_session_id: None,
            parent_task_id: None,
            parent_tool_call_id: None,
            child_session_id: None,
            agent_type: None,
            prompt: None,
            progress_summary: None,
            last_activity: None,
            result_summary: None,
            error: None,
            output_path: None,
            unread: false,
            cancel_requested: false,
            cancel_requested_at_ms: None,
            worker_id: None,
            ownership_token: None,
            lease_until_ms: None,
            heartbeat_at_ms: None,
            retry_count: 0,
            retry_at_ms: None,
            last_error: None,
            input_watermark: None,
            completion_watermark: None,
            created_at_ms: 1,
            started_at_ms: None,
            ended_at_ms: None,
            updated_at_ms: 1,
            metadata_json: metadata_json.into(),
        }
    }

    #[test]
    fn scheduling_round_trips_without_destroying_host_metadata() {
        let mut task = task(r#"{"host":"cli"}"#);
        let expected = RuntimeTaskScheduling {
            priority: RuntimeTaskPriority::High,
            queued_at_ms: 10,
            resource_budget: RuntimeTaskResourceBudget {
                max_wall_time_ms: Some(2_000),
                max_output_bytes: Some(4_096),
                max_cpu_time_ms: Some(1_500),
                max_memory_bytes: Some(64 * 1024 * 1024),
                max_processes: Some(4),
            },
            output_bytes: 12,
            resource_usage: RuntimeTaskResourceUsage {
                cpu_time_ms: 321,
                peak_memory_bytes: 8 * 1024 * 1024,
                peak_processes: 3,
            },
            budget_exhaustion: None,
        };
        task.set_scheduling(expected.clone()).unwrap();
        assert_eq!(task.scheduling().unwrap(), Some(expected));
        assert_eq!(
            serde_json::from_str::<Value>(&task.metadata_json).unwrap()["host"],
            "cli"
        );
    }

    #[test]
    fn scheduling_distinguishes_absent_policy_from_invalid_metadata_container() {
        assert_eq!(task("{}").scheduling().unwrap(), None);
        for metadata in ["null", "[]", "42", "\"text\""] {
            let mut invalid = task(metadata);
            assert_eq!(invalid.scheduling().unwrap_err(), "runtime task metadata must be a JSON object");
            let scheduling = RuntimeTaskScheduling {
                priority: RuntimeTaskPriority::Normal,
                queued_at_ms: 1,
                resource_budget: RuntimeTaskResourceBudget {
                    max_wall_time_ms: Some(1_000),
                    max_output_bytes: Some(1_024),
                    max_cpu_time_ms: None,
                    max_memory_bytes: None,
                    max_processes: None,
                },
                output_bytes: 0,
                resource_usage: RuntimeTaskResourceUsage::default(),
                budget_exhaustion: None,
            };
            assert_eq!(invalid.set_scheduling(scheduling).unwrap_err(), "runtime task metadata must be a JSON object");
        }
    }

    #[test]
    fn malformed_or_zero_budget_is_rejected_at_the_application_boundary() {
        let malformed = task(r#"{"_nola_scheduler":{"priority":"urgent"}}"#);
        assert!(malformed.scheduling().is_err());
        let mut zero = task("{}");
        assert!(zero
            .set_scheduling(RuntimeTaskScheduling {
                priority: RuntimeTaskPriority::Normal,
                queued_at_ms: 1,
                resource_budget: RuntimeTaskResourceBudget {
                    max_wall_time_ms: Some(0),
                    max_output_bytes: Some(1),
                    max_cpu_time_ms: Some(1),
                    max_memory_bytes: Some(1),
                    max_processes: Some(1),
                },
                output_bytes: 0,
                resource_usage: RuntimeTaskResourceUsage::default(),
                budget_exhaustion: None,
            })
            .is_err());

        for resource_budget in [
            RuntimeTaskResourceBudget {
                max_cpu_time_ms: Some(0),
                ..Default::default()
            },
            RuntimeTaskResourceBudget {
                max_memory_bytes: Some(0),
                ..Default::default()
            },
            RuntimeTaskResourceBudget {
                max_processes: Some(0),
                ..Default::default()
            },
        ] {
            assert!(zero
                .set_scheduling(RuntimeTaskScheduling {
                    priority: RuntimeTaskPriority::Normal,
                    queued_at_ms: 1,
                    resource_budget,
                    output_bytes: 0,
                    resource_usage: RuntimeTaskResourceUsage::default(),
                    budget_exhaustion: None,
                })
                .is_err());
        }
    }
    #[derive(Default)]
    struct MemoryTasks {
        task: Option<StoredRuntimeTask>,
        attempt: Option<RuntimeTaskAttempt>,
        suspension: Option<RuntimeTaskPermissionSuspension>,
        fail_write: bool,
        reject_progress_cas: bool,
    }

    fn permission_budget_fixture() -> MemoryTasks {
        let mut waiting = task("{}");
        waiting.status = "waiting_permission".into();
        waiting.started_at_ms = Some(10);
        waiting.updated_at_ms = 20;
        waiting.set_scheduling(RuntimeTaskScheduling {
            priority: RuntimeTaskPriority::Normal, queued_at_ms: 1,
            resource_budget: RuntimeTaskResourceBudget { max_wall_time_ms: Some(100), ..Default::default() },
            output_bytes: 0, resource_usage: Default::default(), budget_exhaustion: None,
        }).unwrap();
        MemoryTasks {
            task: Some(stored_task(waiting)),
            attempt: Some(RuntimeTaskAttempt {
                id: "attempt".into(), task_id: "task-budget".into(), attempt_number: 1,
                worker_id: "worker".into(), worker_generation: "generation".into(),
                lease_token: "retired-token".into(), lease_expires_at_ms: 30,
                state: "permission_suspended".into(), revision: 2, claimed_at_ms: 10,
            }),
            suspension: Some(RuntimeTaskPermissionSuspension {
                id: "suspension".into(), task_id: "task-budget".into(), attempt_id: "attempt".into(),
                permission_request_id: "permission".into(), continuation_id: "continuation".into(),
                continuation_json: "{}".into(), state: "pending".into(), revision: 1,
                suspended_at_ms: 20, resumed_at_ms: None,
            }), ..Default::default()
        }
    }

    #[test]
    fn permission_wait_budget_uses_original_start_without_execution_lease() {
        let mut repo = permission_budget_fixture();
        repo.fail_write = true;
        let early = observe_permission_wait_budget(&mut repo, "task-budget", "suspension", 1, 109).unwrap();
        assert!(early.scheduling().unwrap().unwrap().budget_exhaustion.is_none());
        repo.fail_write = false;
        let attempt = repo.attempt.clone();
        let suspension = repo.suspension.clone();
        let result = observe_permission_wait_budget(&mut repo, "task-budget", "suspension", 1, 110).unwrap();
        assert_eq!(result.scheduling().unwrap().unwrap().budget_exhaustion.as_deref(), Some("wall_time_ms"));
        assert_eq!(result.status, "waiting_permission");
        assert!(result.ownership_token.is_none());
        assert_eq!(repo.attempt, attempt);
        assert_eq!(repo.suspension, suspension);
        repo.fail_write = true;
        assert_eq!(observe_permission_wait_budget(&mut repo, "task-budget", "suspension", 1, 120).unwrap(), result);
    }

    #[test]
    fn permission_wait_budget_rejects_stale_identity_and_observation_errors() {
        for case in 0..8 {
            let mut repo = permission_budget_fixture();
            match case {
                0 => repo.task.as_mut().unwrap().cancel_requested = true,
                1 => repo.task.as_mut().unwrap().status = "running".into(),
                2 => repo.suspension.as_mut().unwrap().revision = 2,
                3 => repo.attempt.as_mut().unwrap().state = "leased".into(),
                4 => repo.task.as_mut().unwrap().metadata_json = "{}".into(),
                5 => repo.reject_progress_cas = true,
                6 => repo.fail_write = true,
                _ => repo.task.as_mut().unwrap().updated_at_ms = 111,
            }
            let before = repo.task.clone();
            assert!(observe_permission_wait_budget(&mut repo, "task-budget", "suspension", 1, 110).is_err(), "case {case}");
            assert_eq!(repo.task, before, "case {case} must not synthesize budget state");
        }
    }
    #[allow(unused_variables)]
    impl RuntimeTaskRepository for MemoryTasks {
        fn submit_terminal_input(&mut self, _input: &nola_domain::terminal_input::TerminalInputRequest,
            _now_ms: i64) -> Result<nola_domain::terminal_input::TerminalInputReceipt, Self::Error> {
            Err("terminal input is not part of this lifecycle fixture".into())
        }
        fn outstanding_runtime_task_guidance_count(&self, _: &str) -> Result<u64, Self::Error> {
            Err("fixture guidance authority unavailable".into())
        }
        fn runtime_task_guidance_records(
            &self,
            _: &str,
        ) -> Result<Vec<RuntimeTaskGuidance>, Self::Error> {
            Ok(vec![])
        }
        fn runtime_task_guidance(
            &self,
            _: &str,
            _: &str,
        ) -> Result<Option<RuntimeTaskGuidance>, Self::Error> {
            Ok(None)
        }
        fn pending_runtime_task_guidance(
            &self,
            _: &str,
            _: &str,
        ) -> Result<Vec<RuntimeTaskGuidance>, Self::Error> {
            Ok(vec![])
        }
        fn insert_runtime_task_guidance(
            &mut self,
            _: &RuntimeTaskGuidance,
        ) -> Result<RuntimeTaskGuidance, Self::Error> {
            Err("guidance unavailable".into())
        }
        fn resolve_runtime_task_guidance(
            &mut self,
            _: &str,
            _: &str,
            _: &str,
            _: Option<&str>,
            _: Option<&str>,
            _: i64,
        ) -> Result<RuntimeTaskGuidance, Self::Error> {
            Err("guidance unavailable".into())
        }
        type Error = String;
        fn runtime_task_run_binding(
            &self,
            _: &str,
        ) -> Result<Option<RuntimeTaskRunBinding>, Self::Error> {
            Ok(None)
        }
        fn runtime_task_terminal_receipt(
            &self,
            _: &str,
        ) -> Result<Option<RuntimeTaskTerminalReceipt>, Self::Error> {
            Ok(None)
        }
        fn validate_runtime_task_turn_admission(
            &self,
            _: &RuntimeTaskLeaseWitness,
            _: &str,
            _: i64,
        ) -> Result<(), Self::Error> {
            Err("fixture operation is deliberately unavailable".into())
        }
        fn bind_runtime_task_run(
            &mut self,
            _: &RuntimeTaskLeaseWitness,
            _: &str,
            _: &str,
            _: i64,
        ) -> Result<RuntimeTaskRunBinding, Self::Error> {
            Err("fixture operation is deliberately unavailable".into())
        }
        fn is_runtime_task_child_session(&self, _: &str) -> Result<bool, Self::Error> {
            Ok(false)
        }
        fn reconcile_runtime_task_bound_run_terminal(
            &mut self,
            _: &str,
            _: &str,
            _: i64,
        ) -> Result<bool, Self::Error> {
            Err("fixture operation is deliberately unavailable".into())
        }
        fn suspend_runtime_task_for_command_reconciliation(
            &mut self,
            _: &SuspendRuntimeTaskForCommandReconciliation,
        ) -> Result<Option<RuntimeTaskCommandReconciliation>, Self::Error> {
            Err("fixture operation is deliberately unavailable".into())
        }
        fn pending_runtime_task_command_reconciliation(
            &self,
            _: &str,
        ) -> Result<Option<RuntimeTaskCommandReconciliation>, Self::Error> {
            Err("fixture operation is deliberately unavailable".into())
        }
        fn acknowledge_runtime_task_model_delivery(
            &mut self,
            task_id: &str,
        ) -> Result<bool, Self::Error> {
            Err("fixture operation is deliberately unavailable".into())
        }
        fn acknowledge_runtime_task_result(
            &mut self,
            _: &AcknowledgeRuntimeTaskResult,
        ) -> Result<bool, Self::Error> {
            Err("fixture operation is deliberately unavailable".into())
        }
        fn runtime_task_notification_delivered(&self, _: &RuntimeTaskNotificationDelivery) -> Result<bool, Self::Error> {
            Err("fixture operation is deliberately unavailable".into())
        }
        fn acknowledge_runtime_task_delivery(&mut self, _: &RuntimeTaskNotificationDelivery) -> Result<RuntimeTaskNotificationAcknowledgement, Self::Error> {
            Err("fixture operation is deliberately unavailable".into())
        }
        fn append_runtime_task_event(
            &mut self,
            task_id: &str,
            kind: &str,
            status: &str,
            summary: Option<&str>,
            detail_json: &str,
            created_at_ms: i64,
        ) -> Result<i64, Self::Error> {
            Err("fixture operation is deliberately unavailable".into())
        }
        fn claim_runtime_task(
            &mut self,
            task_id: &str,
            worker_id: &str,
            worker_generation: &str,
            ownership_token: &str,
            now_ms: i64,
            lease_until_ms: i64,
        ) -> Result<Option<nola_store_port::runtime_task::RuntimeTaskAttempt>, Self::Error>
        {
            let Some(task) = self.task.as_mut() else {
                return Ok(None);
            };
            if task.id != task_id || task.status != "queued" || lease_until_ms <= now_ms {
                return Ok(None);
            }
            task.status = "running".into();
            task.worker_id = Some(worker_id.into());
            task.ownership_token = Some(ownership_token.into());
            task.lease_until_ms = Some(lease_until_ms);
            task.heartbeat_at_ms = Some(now_ms);
            task.started_at_ms = Some(now_ms);
            task.updated_at_ms = now_ms;
            let attempt = RuntimeTaskAttempt {
                id: "attempt-1".into(),
                task_id: task_id.into(),
                attempt_number: 1,
                worker_id: worker_id.into(),
                worker_generation: worker_generation.into(),
                lease_token: ownership_token.into(),
                lease_expires_at_ms: lease_until_ms,
                state: "leased".into(),
                revision: 1,
                claimed_at_ms: now_ms,
            };
            self.attempt = Some(attempt.clone());
            Ok(Some(attempt))
        }
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
        ) -> Result<Option<nola_store_port::runtime_task::RuntimeTaskAttempt>, Self::Error>
        {
            Err("fixture operation is deliberately unavailable".into())
        }
        fn create_runtime_task_with_event(
            &mut self,
            task: &StoredRuntimeTask,
        ) -> Result<(), Self::Error> {
            if self.fail_write {
                return Err("write unavailable".into());
            }
            if self.task.is_some() {
                return Err("duplicate task".into());
            }
            self.task = Some(task.clone());
            Ok(())
        }
        fn fail_expired_runtime_task(
            &mut self,
            task_id: &str,
            error: &str,
            now_ms: i64,
        ) -> Result<bool, Self::Error> {
            Err("fixture operation is deliberately unavailable".into())
        }
        fn fail_expired_runtime_task_with_event(
            &mut self,
            task_id: &str,
            error: &str,
            event_kind: &str,
            event_status: &str,
            event_summary: Option<&str>,
            event_detail_json: &str,
            now_ms: i64,
        ) -> Result<bool, Self::Error> {
            Err("fixture operation is deliberately unavailable".into())
        }
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
        ) -> Result<bool, Self::Error> {
            Err("fixture operation is deliberately unavailable".into())
        }
        fn renew_runtime_task_lease(
            &mut self,
            renewal: &RenewRuntimeTaskLease,
        ) -> Result<Option<RuntimeTaskAttempt>, Self::Error> {
            let Some(attempt) = self.attempt.as_mut() else {
                return Ok(None);
            };
            let Some(task) = self.task.as_mut() else {
                return Ok(None);
            };
            if attempt.id != renewal.attempt_id
                || attempt.task_id != renewal.task_id
                || attempt.revision != renewal.expected_attempt_revision
                || attempt.worker_generation != renewal.worker_generation
                || attempt.lease_token != renewal.lease_token
                || attempt.lease_expires_at_ms != renewal.current_expiry_ms
                || attempt.state != "leased"
                || task.status != "running"
                || task.lease_until_ms != Some(renewal.current_expiry_ms)
            {
                return Ok(None);
            }
            attempt.revision += 1;
            attempt.lease_expires_at_ms = renewal.requested_expiry_ms;
            task.lease_until_ms = Some(renewal.requested_expiry_ms);
            task.heartbeat_at_ms = Some(renewal.now_ms);
            task.updated_at_ms = renewal.now_ms;
            Ok(Some(attempt.clone()))
        }
        fn latest_runtime_task_event_id(&self) -> Result<i64, Self::Error> {
            Err("fixture operation is deliberately unavailable".into())
        }
        fn runtime_task_for_delegation(
            &self, _: &str, _: &str, _: &str,
        ) -> Result<Option<StoredRuntimeTask>, Self::Error> {
            Err("fixture operation is deliberately unavailable".into())
        }
        fn list_all_runtime_tasks(&self) -> Result<Vec<StoredRuntimeTask>, Self::Error> {
            Err("fixture operation is deliberately unavailable".into())
        }
        fn list_runtime_tasks(
            &self,
            project_root: &str,
        ) -> Result<Vec<StoredRuntimeTask>, Self::Error> {
            Err("fixture operation is deliberately unavailable".into())
        }
        fn runtime_agent_task_authority(
            &self,
            _task_id: &str,
        ) -> Result<Option<nola_store_port::runtime_task::RuntimeAgentTaskAuthority>, Self::Error>
        {
            Err("fixture operation is deliberately unavailable".into())
        }
        fn admit_runtime_agent_task(
            &mut self,
            _admission: &nola_store_port::runtime_task::AgentTaskAdmission,
        ) -> Result<nola_store_port::runtime_task::RuntimeAgentTaskAuthority, Self::Error> {
            Err("fixture operation is deliberately unavailable".into())
        }
        fn source_agent_task_admission_evidence(
            &self,
            _conversation_id: &str,
            _run_id: &str,
            _task_id: &str,
            _event_cursor: u64,
        ) -> Result<
            Option<nola_store_port::runtime_task::StoredAgentTaskAdmissionEvidence>,
            Self::Error,
        > {
            Err("fixture operation is deliberately unavailable".into())
        }
        fn prepared_agent_task_ids(&self, _: &str, _: &str, _: usize) -> Result<Vec<String>, Self::Error> {
            Err("fixture operation is deliberately unavailable".into())
        }
        fn source_agent_task_admission_location(&self, _: &str, _: &str) -> Result<Option<(String, u64)>, Self::Error> {
            Err("fixture operation is deliberately unavailable".into())
        }
        fn recover_expired_runtime_task(
            &mut self,
            task_id: &str,
            error: &str,
            retry_at_ms: i64,
            now_ms: i64,
        ) -> Result<bool, Self::Error> {
            Err("fixture operation is deliberately unavailable".into())
        }
        fn request_runtime_task_cancel(
            &mut self,
            task_id: &str,
            now_ms: i64,
        ) -> Result<bool, Self::Error> {
            Err("fixture operation is deliberately unavailable".into())
        }
        fn report_runtime_task_progress(
            &mut self,
            mutation: &RuntimeTaskProgressMutation,
        ) -> Result<Option<RuntimeTaskAttempt>, Self::Error> {
            if self.fail_write {
                return Err("write unavailable".into());
            }
            if self.reject_progress_cas {
                return Ok(None);
            }
            let Some(task) = self.task.as_mut() else {
                return Ok(None);
            };
            let Some(attempt) = self.attempt.as_mut() else {
                return Ok(None);
            };
            if task.id != mutation.task_id
                || task.status != "running"
                || task.ownership_token.as_deref() != Some(&mutation.lease_token)
                || task.updated_at_ms != mutation.expected_updated_at_ms
                || task.lease_until_ms != Some(mutation.current_expiry_ms)
                || task
                    .lease_until_ms
                    .is_none_or(|expiry| expiry <= mutation.now_ms)
                || task.metadata_json != mutation.expected_metadata_json
                || task.cancel_requested
                || attempt.id != mutation.attempt_id
                || attempt.revision != mutation.expected_attempt_revision
                || attempt.worker_generation != mutation.worker_generation
                || attempt.lease_token != mutation.lease_token
            {
                return Ok(None);
            }
            task.progress_summary = mutation.progress_summary.clone();
            task.last_activity = mutation.last_activity.clone();
            task.metadata_json = mutation.metadata_json.clone();
            task.updated_at_ms = mutation.now_ms;
            attempt.revision += 1;
            Ok(Some(attempt.clone()))
        }
        fn observe_runtime_task_permission_budget(
            &mut self,
            mutation: &RuntimeTaskPermissionBudgetMutation,
        ) -> Result<bool, Self::Error> {
            if self.fail_write { return Err("write unavailable".into()); }
            if self.reject_progress_cas { return Ok(false); }
            let task = self.task.as_mut().ok_or("fixture task absent")?;
            if task.metadata_json != mutation.expected_metadata_json
                || task.updated_at_ms != mutation.expected_updated_at_ms {
                return Ok(false);
            }
            task.metadata_json = mutation.metadata_json.clone();
            task.updated_at_ms = mutation.now_ms;
            Ok(true)
        }
        fn resume_runtime_task_after_permission_fair(
            &mut self,
            _: &ResumeRuntimeTaskAfterPermissionFair,
        ) -> Result<Option<RuntimeTaskAttempt>, Self::Error> {
            Err("fixture operation is deliberately unavailable".into())
        }
        fn runtime_task_permission_suspension(
            &self,
            id: &str,
        ) -> Result<Option<RuntimeTaskPermissionSuspension>, Self::Error> {
            Ok(self.suspension.clone().filter(|suspension| suspension.id == id))
        }
        fn pending_runtime_task_permission_suspension(
            &self,
            _: &str,
        ) -> Result<Option<RuntimeTaskPermissionSuspension>, Self::Error> {
            Ok(None)
        }
        fn latest_runtime_task_permission_suspension(
            &self,
            _: &str,
        ) -> Result<Option<RuntimeTaskPermissionSuspension>, Self::Error> {
            Ok(None)
        }
        fn runtime_task_permission_resume(
            &self,
            _: &str,
            _: &str,
            _: &str,
            _: &str,
        ) -> Result<Option<RuntimeTaskPermissionResume>, Self::Error> {
            Err("fixture operation is deliberately unavailable".into())
        }
        fn runtime_task_permission_continuation_grant(
            &self,
            _: &str,
            _: &str,
            _: &PermissionContinuationGrantSelector,
        ) -> Result<Option<RuntimeTaskPermissionContinuationGrant>, Self::Error> {
            Err("fixture operation is deliberately unavailable".into())
        }
        fn recover_runtime_task_permission_continuation_fair(
            &mut self,
            _: &RecoverRuntimeTaskPermissionContinuationFair,
        ) -> Result<Option<RuntimeTaskAttempt>, Self::Error> {
            Err("fixture operation is deliberately unavailable".into())
        }
        fn cancel_runtime_task_permission_suspension(
            &mut self,
            _: &CancelRuntimeTaskPermissionSuspension,
        ) -> Result<Option<RuntimeTaskPermissionSuspension>, Self::Error> {
            Err("fixture operation is deliberately unavailable".into())
        }
        fn suspend_runtime_task_for_permission(
            &mut self,
            _: &SuspendRuntimeTaskForPermission,
        ) -> Result<Option<RuntimeTaskPermissionSuspension>, Self::Error> {
            Err("fixture operation is deliberately unavailable".into())
        }
        fn retry_runtime_task(
            &mut self,
            lease: &RuntimeTaskLeaseWitness,
            retry_at_ms: i64,
            error: &str,
            now_ms: i64,
        ) -> Result<bool, Self::Error> {
            Err("fixture operation is deliberately unavailable".into())
        }
        fn runtime_task(&self, id: &str) -> Result<Option<StoredRuntimeTask>, Self::Error> {
            Ok(self.task.as_ref().filter(|t| t.id == id).cloned())
        }
        fn runtime_task_attempt(
            &self,
            id: &str,
        ) -> Result<Option<nola_store_port::runtime_task::RuntimeTaskAttempt>, Self::Error>
        {
            Ok(self
                .attempt
                .as_ref()
                .filter(|attempt| attempt.id == id)
                .cloned())
        }
        fn runtime_task_current_attempt(
            &self,
            task_id: &str,
        ) -> Result<Option<nola_store_port::runtime_task::RuntimeTaskAttempt>, Self::Error>
        {
            Ok(self
                .attempt
                .as_ref()
                .filter(|attempt| {
                    attempt.task_id == task_id
                        && attempt.state == "leased"
                        && self.task.as_ref().is_some_and(|task| {
                            task.id == task_id
                                && task.worker_id.as_deref() == Some(attempt.worker_id.as_str())
                                && task.ownership_token.as_deref()
                                    == Some(attempt.lease_token.as_str())
                                && task.lease_until_ms == Some(attempt.lease_expires_at_ms)
                        })
                })
                .cloned())
        }
        fn runtime_task_attempt_retired_for_user_input(&self, _task_id: &str, _attempt_id: &str) -> Result<bool, Self::Error> { Ok(false) }

        fn runtime_task_events(
            &self,
            task_id: &str,
            limit: usize,
        ) -> Result<Vec<StoredRuntimeTaskEvent>, Self::Error> {
            Err("fixture operation is deliberately unavailable".into())
        }
        fn runtime_task_events_after_session_cursor(
            &self,
            owner_session_id: &str,
            after_cursor: i64,
            limit: usize,
        ) -> Result<Vec<StoredRuntimeTaskEvent>, Self::Error> {
            Err("fixture operation is deliberately unavailable".into())
        }
    }

    #[test]
    fn commands_and_progress_use_a_non_sqlite_repository_without_bypassing_validation() {
        let mut input = task(r#"{"host":"fixture"}"#);
        input
            .set_scheduling(RuntimeTaskScheduling {
                priority: RuntimeTaskPriority::High,
                queued_at_ms: 1,
                resource_budget: RuntimeTaskResourceBudget {
                    max_output_bytes: Some(10),
                    ..Default::default()
                },
                output_bytes: 0,
                resource_usage: Default::default(),
                budget_exhaustion: None,
            })
            .unwrap();
        let input = CreateRuntimeTask::try_from(input).unwrap();
        let mut repo = MemoryTasks {
            fail_write: true,
            ..Default::default()
        };
        assert!(execute_runtime_task_command(
            &mut repo,
            RuntimeTaskCommand::Create {
                input: input.clone()
            }
        )
        .unwrap_err()
        .contains("write unavailable"));
        assert!(repo.task.is_none());
        repo.fail_write = false;
        execute_runtime_task_command(&mut repo, RuntimeTaskCommand::Create { input }).unwrap();
        assert!(execute_runtime_task_command(
            &mut repo,
            RuntimeTaskCommand::Claim {
                task_id: "task-budget".into(),
                worker_id: "worker-generation-1".into(),
                worker_generation: " bad-generation".into(),
                lease_token: "owner".into(),
                now_ms: 1,
                lease_until_ms: 100,
            },
        )
        .unwrap_err()
        .contains("worker_generation"));
        assert_eq!(repo.task.as_ref().unwrap().status, "queued");
        let claimed = execute_runtime_task_command(
            &mut repo,
            RuntimeTaskCommand::Claim {
                task_id: "task-budget".into(),
                worker_id: "worker-generation-1".into(),
                worker_generation: "generation-1".into(),
                lease_token: "owner".into(),
                now_ms: 1,
                lease_until_ms: 100,
            },
        )
        .unwrap();
        let claimed: nola_store_port::runtime_task::RuntimeTaskAttempt =
            serde_json::from_value(claimed).unwrap();
        assert_eq!(claimed.id, "attempt-1");
        assert_eq!(claimed.attempt_number, 1);
        assert_eq!(claimed.revision, 1);
        assert!(execute_runtime_task_command(
            &mut repo,
            RuntimeTaskCommand::RenewLease {
                task_id: claimed.task_id.clone(),
                attempt_id: claimed.id.clone(),
                expected_attempt_revision: claimed.revision,
                worker_generation: claimed.worker_generation.clone(),
                lease_token: claimed.lease_token.clone(),
                current_expiry_ms: claimed.lease_expires_at_ms,
                now_ms: 2,
                requested_expiry_ms: claimed.lease_expires_at_ms,
            },
        )
        .unwrap_err()
        .contains("strictly extend"));
        let renewed = execute_runtime_task_command(
            &mut repo,
            RuntimeTaskCommand::RenewLease {
                task_id: claimed.task_id.clone(),
                attempt_id: claimed.id.clone(),
                expected_attempt_revision: claimed.revision,
                worker_generation: claimed.worker_generation.clone(),
                lease_token: claimed.lease_token.clone(),
                current_expiry_ms: claimed.lease_expires_at_ms,
                now_ms: 2,
                requested_expiry_ms: 101,
            },
        )
        .unwrap();
        let renewed: RuntimeTaskAttempt = serde_json::from_value(renewed).unwrap();
        assert_eq!(renewed.revision, 2);
        assert_eq!(renewed.lease_expires_at_ms, 101);
        let report = |attempt: &RuntimeTaskAttempt, lease_token: &str, bytes| {
            RuntimeTaskCommand::ReportProgress {
                task_id: "task-budget".into(),
                attempt_id: attempt.id.clone(),
                expected_attempt_revision: attempt.revision,
                worker_generation: attempt.worker_generation.clone(),
                lease_token: lease_token.into(),
                current_expiry_ms: attempt.lease_expires_at_ms,
                progress_summary: Some("working".into()),
                last_activity: None,
                output_bytes: Some(bytes),
                resource_usage: None,
                metadata_patch: BTreeMap::new(),
                now_ms: 2,
            }
        };
        let before = repo.task.clone();
        assert!(
            execute_runtime_task_command(&mut repo, report(&renewed, "wrong", 11))
                .unwrap_err()
                .contains("stale or foreign")
        );
        assert_eq!(repo.task, before);
        repo.fail_write = true;
        assert!(
            execute_runtime_task_command(&mut repo, report(&renewed, "owner", 11))
                .unwrap_err()
                .contains("write unavailable")
        );
        assert_eq!(repo.task, before);
        repo.fail_write = false;
        let progress_ack =
            execute_runtime_task_command(&mut repo, report(&renewed, "owner", 11)).unwrap();
        let progress_ack: RuntimeTaskProgressAck = serde_json::from_value(progress_ack).unwrap();
        assert_eq!(progress_ack.attempt.revision, renewed.revision + 1);
        let read = execute_runtime_task_command(
            &mut repo,
            RuntimeTaskCommand::Get {
                id: "task-budget".into(),
            },
        )
        .unwrap();
        let read: RuntimeTask = serde_json::from_value(read).unwrap();
        assert_eq!(
            read.scheduling()
                .unwrap()
                .unwrap()
                .budget_exhaustion
                .as_deref(),
            Some("output_bytes")
        );
        assert_eq!(
            serde_json::from_str::<Value>(&read.metadata_json).unwrap()["host"],
            "fixture"
        );
        let after = repo.task.clone();
        assert!(execute_runtime_task_command(
            &mut repo,
            report(&progress_ack.attempt, "owner", 10)
        )
        .unwrap_err()
        .contains("moved backwards"));
        assert_eq!(repo.task, after);

        repo.reject_progress_cas = true;
        assert!(execute_runtime_task_command(
            &mut repo,
            report(&progress_ack.attempt, "owner", 12)
        )
        .unwrap_err()
        .contains("lost its active attempt CAS"));
        assert_eq!(repo.task, after);
        repo.reject_progress_cas = false;
        repo.task.as_mut().unwrap().lease_until_ms = Some(2);
        repo.attempt.as_mut().unwrap().lease_expires_at_ms = 2;
        let expired_attempt = repo.attempt.clone().unwrap();
        assert!(
            execute_runtime_task_command(&mut repo, report(&expired_attempt, "owner", 12))
                .unwrap_err()
                .contains("lease has expired")
        );
        assert_eq!(
            repo.task,
            after.map(|mut task| {
                task.lease_until_ms = Some(2);
                task
            })
        );
    }

    #[test]
    fn task_creation_cannot_supply_terminal_delivery_history() {
        let mut original = task("{}");
        original.kind = "shell_process".into();
        original.set_scheduling(RuntimeTaskScheduling {
            priority: RuntimeTaskPriority::Normal,
            queued_at_ms: original.created_at_ms,
            resource_budget: RuntimeTaskResourceBudget::default(),
            output_bytes: 0,
            resource_usage: RuntimeTaskResourceUsage::default(),
            budget_exhaustion: None,
        }).unwrap();
        let mut input = CreateRuntimeTask::try_from(original).unwrap();
        input.metadata[nola_domain::terminal_input::TERMINAL_INPUT_RECEIPTS_METADATA_KEY] = json!({});
        assert!(input.validate().unwrap_err().contains("delivery receipts"));
    }

    #[test]
    fn generic_progress_cannot_replace_or_erase_terminal_input_state() {
        use nola_domain::terminal_input::{
            TERMINAL_INPUT_QUEUE_METADATA_KEY, TERMINAL_INPUT_RECEIPTS_METADATA_KEY,
        };
        for key in [TERMINAL_INPUT_QUEUE_METADATA_KEY, TERMINAL_INPUT_RECEIPTS_METADATA_KEY] {
            for value in [None, Some(json!({})), Some(json!({"request": "forged"}))] {
                let mut current = task("{}");
                let before = current.clone();
                let error = apply_runtime_task_observation(
                    &mut current, None, None,
                    BTreeMap::from([(key.to_owned(), value)]), 42,
                ).unwrap_err();
                assert!(error.contains("reserved key"));
                assert_eq!(current, before);
            }
        }
    }

    #[test]
    fn task_receipt_failure_rolls_back_event_and_claim_then_reopens_exactly() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("task-port.db");
        let mut runtime = crate::ApplicationRuntime::open(&path).unwrap();
        let inspect = rusqlite::Connection::open(&path).unwrap();
        let mut input = task("{}");
        // This test exercises generic receipt atomicity, not Core agent admission.
        input.kind = "shell_process".into();
        input
            .set_scheduling(RuntimeTaskScheduling {
                priority: RuntimeTaskPriority::High,
                queued_at_ms: 1,
                resource_budget: Default::default(),
                output_bytes: 0,
                resource_usage: Default::default(),
                budget_exhaustion: None,
            })
            .unwrap();
        let input = CreateRuntimeTask::try_from(input).unwrap();
        let command = serde_json::to_string(&crate::ApplicationCommand::RuntimeTask {
            command: RuntimeTaskCommand::Create { input },
        })
        .unwrap();
        let receipt = |id: &str, command: &str| crate::ExecuteReceivedCommand {
            client_id: "task-client".into(),
            request_id: id.into(),
            command_type: "runtime_task".into(),
            command_json: command.into(),
            expected_revision: None,
            received_at: "1".into(),
        };
        inspect.execute_batch("CREATE TRIGGER fail_task_receipt BEFORE UPDATE ON command_receipts WHEN NEW.status='committed' BEGIN SELECT RAISE(ABORT,'task receipt fault'); END;").unwrap();
        assert!(runtime
            .execute_received_command(
                receipt("create", &command),
                serde_json::from_str(&command).unwrap()
            )
            .unwrap_err()
            .to_string()
            .contains("task receipt fault"));
        assert!(runtime.store.runtime_task("task-budget").unwrap().is_none());
        assert_eq!(runtime.store.latest_runtime_task_event_id().unwrap(), 0);
        inspect
            .execute_batch("DROP TRIGGER fail_task_receipt;")
            .unwrap();
        runtime
            .execute_received_command(
                receipt("create", &command),
                serde_json::from_str(&command).unwrap(),
            )
            .unwrap();
        let cursor = runtime.store.latest_runtime_task_event_id().unwrap();
        assert!(cursor > 0);
        let claim = serde_json::to_string(&crate::ApplicationCommand::RuntimeTask {
            command: RuntimeTaskCommand::ClaimFair {
                task_id: "task-budget".into(),
                worker_id: "worker".into(),
                worker_generation: "generation".into(),
                lease_token: "owner".into(),
                now_ms: 2,
                lease_until_ms: 100,
                max_running_global: 1,
                max_running_project: 1,
            },
        })
        .unwrap();
        let mut missing_generation: Value = serde_json::from_str(&claim).unwrap();
        missing_generation["command"]
            .as_object_mut()
            .unwrap()
            .remove("worker_generation");
        assert!(serde_json::from_value::<crate::ApplicationCommand>(missing_generation).is_err());
        inspect.execute_batch("CREATE TRIGGER fail_task_receipt BEFORE UPDATE ON command_receipts WHEN NEW.status='committed' BEGIN SELECT RAISE(ABORT,'task receipt fault'); END;").unwrap();
        assert!(runtime
            .execute_received_command(
                receipt("claim", &claim),
                serde_json::from_str(&claim).unwrap()
            )
            .is_err());
        assert_eq!(
            runtime
                .store
                .runtime_task("task-budget")
                .unwrap()
                .unwrap()
                .status,
            "queued"
        );
        let attempts_after_rollback: i64 = inspect
            .query_row("SELECT COUNT(*) FROM runtime_task_attempts", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(attempts_after_rollback, 0);
        inspect
            .execute_batch("DROP TRIGGER fail_task_receipt;")
            .unwrap();
        let committed = runtime
            .execute_received_command(
                receipt("claim", &claim),
                serde_json::from_str(&claim).unwrap(),
            )
            .unwrap();
        drop(runtime);
        let mut reopened = crate::ApplicationRuntime::open(&path).unwrap();
        let replay = reopened
            .execute_received_command(
                receipt("claim", &claim),
                serde_json::from_str(&claim).unwrap(),
            )
            .unwrap();
        assert!(replay.replayed);
        assert_eq!(replay.outcome, committed.outcome);
        assert_eq!(
            reopened
                .store
                .runtime_task("task-budget")
                .unwrap()
                .unwrap()
                .ownership_token
                .as_deref(),
            Some("owner")
        );
        assert_eq!(
            reopened.store.latest_runtime_task_event_id().unwrap(),
            cursor
        );
        let claimed: RuntimeTaskAttempt = match &committed.outcome {
            crate::ApplicationCommandReceiptOutcome::Committed { result, .. } => {
                serde_json::from_value(result.clone()).unwrap()
            }
            outcome => panic!("unexpected claim outcome: {outcome:?}"),
        };
        let before_task = reopened.store.runtime_task("task-budget").unwrap().unwrap();
        let progress = serde_json::to_string(&crate::ApplicationCommand::RuntimeTask {
            command: RuntimeTaskCommand::ReportProgress {
                task_id: claimed.task_id.clone(),
                attempt_id: claimed.id.clone(),
                expected_attempt_revision: claimed.revision,
                worker_generation: claimed.worker_generation.clone(),
                lease_token: claimed.lease_token.clone(),
                current_expiry_ms: claimed.lease_expires_at_ms,
                progress_summary: Some("must roll back".into()),
                last_activity: Some("working".into()),
                output_bytes: None,
                resource_usage: None,
                metadata_patch: BTreeMap::new(),
                now_ms: 3,
            },
        })
        .unwrap();
        inspect.execute_batch("CREATE TRIGGER fail_task_progress_receipt BEFORE UPDATE ON command_receipts WHEN NEW.status='committed' BEGIN SELECT RAISE(ABORT,'task progress receipt fault'); END;").unwrap();
        assert!(reopened
            .execute_received_command(
                receipt("progress", &progress),
                serde_json::from_str(&progress).unwrap(),
            )
            .unwrap_err()
            .to_string()
            .contains("task progress receipt fault"));
        assert_eq!(
            reopened.store.runtime_task("task-budget").unwrap().unwrap(),
            before_task
        );
        assert_eq!(
            reopened
                .store
                .runtime_task_attempt(&claimed.id)
                .unwrap()
                .unwrap()
                .revision,
            claimed.revision
        );
        inspect
            .execute_batch("DROP TRIGGER fail_task_progress_receipt;")
            .unwrap();
    }

    #[test]
    fn terminal_commands_require_full_witness_and_receipt_failure_restores_authority() {
        for retry in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("task-terminal-receipt.db");
            let mut runtime = crate::ApplicationRuntime::open(&path).unwrap();
            let mut input = task("{}");
            // Worker witness and receipt rollback apply to ordinary tasks too;
            // genuine agent admission is covered by TaskPromoted E2E fixtures.
            input.kind = "shell_process".into();
            input
                .set_scheduling(RuntimeTaskScheduling {
                    priority: RuntimeTaskPriority::Normal,
                    queued_at_ms: 1,
                    resource_budget: Default::default(),
                    output_bytes: 0,
                    resource_usage: Default::default(),
                    budget_exhaustion: None,
                })
                .unwrap();
            runtime
                .execute(crate::ApplicationCommand::RuntimeTask {
                    command: RuntimeTaskCommand::Create {
                        input: CreateRuntimeTask::try_from(input).unwrap(),
                    },
                })
                .unwrap();
            let claimed: RuntimeTaskAttempt = serde_json::from_value(
                runtime
                    .execute(crate::ApplicationCommand::RuntimeTask {
                        command: RuntimeTaskCommand::Claim {
                            task_id: "task-budget".into(),
                            worker_id: "worker".into(),
                            worker_generation: "generation".into(),
                            lease_token: "token".into(),
                            now_ms: 2,
                            lease_until_ms: 100,
                        },
                    })
                    .unwrap(),
            )
            .unwrap();
            let lease = RuntimeTaskLeaseWitness::from(&claimed);
            let command = crate::ApplicationCommand::RuntimeTask {
                command: if retry {
                    RuntimeTaskCommand::Retry {
                        lease: lease.clone(),
                        retry_at_ms: 10,
                        error: "temporary".into(),
                        now_ms: 3,
                    }
                } else {
                    RuntimeTaskCommand::FinishDetailedWithEvent {
                        lease: lease.clone(),
                        status: "completed".into(),
                        completion_watermark: None,
                        progress_summary: Some("completed".into()),
                        result_summary: Some("owned result".into()),
                        error: None,
                        output_path: None,
                        unread: true,
                        event_kind: "finished".into(),
                        event_status: "completed".into(),
                        event_summary: None,
                        event_detail_json: "{}".into(),
                        final_observation: None,
                        now_ms: 3,
                    }
                },
            };
            let wire = serde_json::to_value(&command).unwrap();
            for field in [
                "task_id",
                "attempt_id",
                "expected_attempt_revision",
                "worker_generation",
                "lease_token",
                "current_expiry_ms",
            ] {
                let mut missing = wire.clone();
                missing["command"]["lease"]
                    .as_object_mut()
                    .unwrap()
                    .remove(field);
                assert!(serde_json::from_value::<crate::ApplicationCommand>(missing).is_err());
            }
            let mut legacy = wire.clone();
            legacy["command"].as_object_mut().unwrap().remove("lease");
            legacy["command"]["task_id"] = json!("task-budget");
            legacy["command"]["ownership_token"] = json!("token");
            assert!(serde_json::from_value::<crate::ApplicationCommand>(legacy).is_err());
            let mut absent = wire.clone();
            absent["command"]["lease"] = Value::Null;
            assert!(serde_json::from_value::<crate::ApplicationCommand>(absent).is_err());
            let command_json = serde_json::to_string(&command).unwrap();
            let receipt = || crate::ExecuteReceivedCommand {
                client_id: "terminal-client".into(),
                request_id: "terminal-request".into(),
                command_type: "runtime_task".into(),
                command_json: command_json.clone(),
                expected_revision: None,
                received_at: "3".into(),
            };
            let before = runtime.store.runtime_task("task-budget").unwrap();
            let cursor = runtime.store.latest_runtime_task_event_id().unwrap();
            let inspect = rusqlite::Connection::open(&path).unwrap();
            inspect.execute_batch("CREATE TRIGGER fail_terminal_receipt BEFORE UPDATE ON command_receipts WHEN NEW.status='committed' BEGIN SELECT RAISE(ABORT,'terminal receipt fault'); END;").unwrap();
            assert!(runtime
                .execute_received_command(receipt(), serde_json::from_str(&command_json).unwrap())
                .unwrap_err()
                .to_string()
                .contains("terminal receipt fault"));
            assert_eq!(runtime.store.runtime_task("task-budget").unwrap(), before);
            assert_eq!(
                runtime.store.runtime_task_attempt(&claimed.id).unwrap(),
                Some(claimed.clone())
            );
            assert_eq!(
                runtime.store.latest_runtime_task_event_id().unwrap(),
                cursor
            );
            drop(runtime);
            let mut runtime = crate::ApplicationRuntime::open(&path).unwrap();
            assert_eq!(runtime.store.runtime_task("task-budget").unwrap(), before);
            assert_eq!(
                runtime.store.runtime_task_attempt(&claimed.id).unwrap(),
                Some(claimed.clone())
            );
            inspect
                .execute_batch("DROP TRIGGER fail_terminal_receipt")
                .unwrap();
            let committed = runtime
                .execute_received_command(receipt(), serde_json::from_str(&command_json).unwrap())
                .unwrap();
            assert!(!committed.replayed);
            assert!(matches!(
                committed.outcome,
                crate::ApplicationCommandReceiptOutcome::Committed { .. }
            ));
            assert_eq!(
                runtime
                    .store
                    .runtime_task_attempt(&claimed.id)
                    .unwrap()
                    .unwrap()
                    .revision,
                claimed.revision + 1
            );
            assert_eq!(
                runtime
                    .store
                    .runtime_task_attempt(&claimed.id)
                    .unwrap()
                    .unwrap()
                    .state,
                "superseded"
            );
            let committed_cursor = runtime.store.latest_runtime_task_event_id().unwrap();
            assert!(committed_cursor > cursor);
            drop(runtime);
            let mut runtime = crate::ApplicationRuntime::open(&path).unwrap();
            let replay = runtime
                .execute_received_command(receipt(), serde_json::from_str(&command_json).unwrap())
                .unwrap();
            assert!(replay.replayed);
            assert_eq!(replay.outcome, committed.outcome);
            assert_eq!(
                runtime.store.latest_runtime_task_event_id().unwrap(),
                committed_cursor
            );
            assert_eq!(
                runtime
                    .store
                    .runtime_task_attempt(&claimed.id)
                    .unwrap()
                    .unwrap()
                    .revision,
                claimed.revision + 1
            );
        }
    }
}
