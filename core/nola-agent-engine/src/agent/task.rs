// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Platform-neutral durable lifecycle for Host-promoted background work.
// - Boundary: Core records explicit Host promotion; it never infers background work.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use super::protocol::{CallId, SessionId, TurnId};

pub const MAX_TASK_GUIDANCE_QUEUE: usize = 32;
pub const MAX_TASK_GUIDANCE_CHARS: usize = 8_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    Agent,
    Tool,
    Skill,
    DataRefresh,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    Queued,
    Running,
    Waiting,
    Suspended,
    RetryWaiting,
    Completed,
    Sealed,
    Failed,
    Cancelled,
}

impl TaskState {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Sealed | Self::Failed | Self::Cancelled)
    }

    fn can_transition_to(self, next: Self) -> bool {
        if self.is_terminal() {
            return false;
        }
        match self {
            Self::Queued => matches!(
                next,
                Self::Queued
                    | Self::Running
                    | Self::Waiting
                    | Self::Suspended
                    | Self::RetryWaiting
                    | Self::Completed
                    | Self::Failed
                    | Self::Cancelled
            ),
            Self::Running => matches!(
                next,
                Self::Running
                    | Self::Waiting
                    | Self::Suspended
                    | Self::RetryWaiting
                    | Self::Completed
                    | Self::Failed
                    | Self::Cancelled
            ),
            Self::Waiting => matches!(
                next,
                Self::Waiting
                    | Self::Running
                    | Self::Suspended
                    | Self::RetryWaiting
                    | Self::Completed
                    | Self::Failed
                    | Self::Cancelled
            ),
            Self::Suspended => matches!(
                next,
                Self::Suspended | Self::Running | Self::Failed | Self::Cancelled
            ),
            Self::RetryWaiting => matches!(
                next,
                Self::RetryWaiting | Self::Running | Self::Failed | Self::Cancelled
            ),
            Self::Completed | Self::Sealed | Self::Failed | Self::Cancelled => false,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskDurability {
    pub run_result: Option<nola_domain::run_result::RunResult>,
    pub result_digest: Option<nola_domain::assistant_journal::AssistantDigest>,
    #[serde(default)]
    pub terminal_receipt_id: Option<String>,
    #[serde(default)]
    pub worker_id: Option<String>,
    #[serde(default)]
    pub ownership_token: Option<String>,
    #[serde(default)]
    pub lease_until_epoch_ms: Option<i64>,
    #[serde(default)]
    pub last_heartbeat_epoch_ms: Option<i64>,
    #[serde(default)]
    pub retry_count: u32,
    #[serde(default)]
    pub retry_at_epoch_ms: Option<i64>,
    #[serde(default)]
    pub last_error: Option<String>,
    #[serde(default)]
    pub input_watermark: Option<i64>,
    #[serde(default)]
    pub completion_watermark: Option<i64>,
}

/// Application resolves this fact from an immutable Store receipt at every
/// reducer boundary. It is not a worker's assertion of completion.
pub type TaskTerminalReconciliation = nola_domain::agent_task_admission::AgentTaskTerminalProjection;

/// Application-authored retirement of one expired Runtime execution. Core's
/// presentation lease is not the source of expiry authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskLeaseRecovery {
    pub recovery_id: String,
    pub task_id: String,
    pub parent_session_id: SessionId,
    pub parent_turn_id: TurnId,
    pub child_session_id: SessionId,
    pub expired_worker_id: String,
    pub expired_ownership_token: String,
    pub expired_at_epoch_ms: i64,
    pub input_watermark: Option<i64>,
    pub retry_count: u32,
    pub retry_at_epoch_ms: i64,
    pub recovered_at_epoch_ms: i64,
    pub error: String,
}

/// A single durable Task aggregate, independent from its parent's model
/// checkpoint. Application reconstructs it from the authoritative Task row,
/// immutable admission and exact Attempt inside the same transaction.
pub enum TaskAuthorityCommand {
    Claim { worker_id:String, ownership_token:String, now_epoch_ms:i64, lease_until_epoch_ms:i64, input_watermark:Option<i64> },
    RecoverExpired { fact:TaskLeaseRecovery },
    ResumeAfterPermission { previous_ownership_token:String,worker_id:String,ownership_token:String,now_epoch_ms:i64,lease_until_epoch_ms:i64,input_watermark:Option<i64> },
    RecoverAfterPermission { previous_ownership_tokens:Vec<String>,worker_id:String,ownership_token:String,now_epoch_ms:i64,lease_until_epoch_ms:i64,input_watermark:Option<i64> },
    Heartbeat {ownership_token:String,now_epoch_ms:i64,lease_until_epoch_ms:i64},
    Complete {ownership_token:String,completion_watermark:Option<i64>,result_ref:Option<String>},
}

pub fn reduce_task_authority(current:TaskHandle, command:TaskAuthorityCommand) -> Result<TaskHandle,String> {
    let id=current.id.clone();
    current.validate()?;
    let mut projection=TaskProjection::default();
    // Birth/lineage were admitted separately. This is an already existing
    // single aggregate, not a second promotion requiring sibling records.
    projection.order.push(id.clone());
    projection.tasks.insert(id.clone(),current);
    match command {
        TaskAuthorityCommand::Claim {worker_id,ownership_token,now_epoch_ms,lease_until_epoch_ms,input_watermark} =>
            projection.claim(&id,worker_id,ownership_token,now_epoch_ms,lease_until_epoch_ms,input_watermark),
        TaskAuthorityCommand::RecoverExpired {fact} => projection.reconcile_lease_recovery(fact),
        TaskAuthorityCommand::ResumeAfterPermission {previous_ownership_token,worker_id,ownership_token,now_epoch_ms,lease_until_epoch_ms,input_watermark} =>
            projection.resume_after_permission(&id,&previous_ownership_token,worker_id,ownership_token,now_epoch_ms,lease_until_epoch_ms,input_watermark),
        TaskAuthorityCommand::RecoverAfterPermission {previous_ownership_tokens,worker_id,ownership_token,now_epoch_ms,lease_until_epoch_ms,input_watermark} =>
            projection.recover_after_permission(&id,&previous_ownership_tokens,worker_id,ownership_token,now_epoch_ms,lease_until_epoch_ms,input_watermark),
        TaskAuthorityCommand::Heartbeat {ownership_token,now_epoch_ms,lease_until_epoch_ms} =>
            projection.heartbeat(&id,&ownership_token,now_epoch_ms,lease_until_epoch_ms),
        TaskAuthorityCommand::Complete {ownership_token,completion_watermark,result_ref} => {
            let next=projection.next_handle_attested(&id,TaskState::Completed,Some(1.0),Some("complete".into()),result_ref,false,
                Some(&ownership_token),completion_watermark)?;
            projection.replace(next)
        }
    }
}

/// Stable relationships needed to resume a promoted Task as an execution
/// unit, not merely restore its display status. Scope identifiers remain
/// opaque to Core; Core owns their durable association with the Task.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskLineage {
    /// Previous execution sharing the child session, never an ancestry edge.
    pub resumed_from_task_id: Option<String>,
    #[serde(default)]
    pub parent_task_id: Option<String>,
    #[serde(default)]
    pub child_session_id: Option<SessionId>,
    #[serde(default)]
    pub parent_call_id: Option<CallId>,
    #[serde(default)]
    pub permission_scope_id: Option<String>,
    #[serde(default)]
    pub cancellation_scope_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskGuidanceMessage {
    pub sequence: u64,
    pub message_id: String,
    pub parent_session_id: SessionId,
    pub parent_turn_id: TurnId,
    pub input_watermark: Option<i64>,
    pub content: String,
    pub enqueued_at_epoch_ms: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskGuidanceQueue {
    #[serde(default)]
    pub results: BTreeMap<String, TaskGuidanceResolution>,
    #[serde(default)]
    pub settled_sequences: std::collections::BTreeSet<u64>,
    #[serde(default)]
    pub last_sequence: u64,
    #[serde(default)]
    pub acknowledged_sequence: u64,
    #[serde(default)]
    pub pending: Vec<TaskGuidanceMessage>,
    /// Bounded idempotency memory for acknowledgements whose messages have
    /// already been pruned from `pending`.
    #[serde(default)]
    pub acknowledged_message_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskGuidanceResolution {
    pub task_id: String,
    pub message_id: String,
    pub parent_session_id: SessionId,
    pub parent_turn_id: TurnId,
    pub input_watermark: Option<i64>,
    pub content: String,
    pub submitted_at_ms: i64,
    pub status: String,
    pub reason: Option<String>,
}

impl TaskLineage {
    fn validate(&self, task_id: &str) -> Result<(), String> {
        if self.parent_task_id.as_deref() == Some(task_id) {
            return Err("Task cannot be its own parent".to_string());
        }
        if self.resumed_from_task_id.as_deref() == Some(task_id) {
            return Err("Task cannot resume itself".into());
        }
        if self.resumed_from_task_id.is_some() && self.child_session_id.is_none() {
            return Err("Resumed Task requires a child session".into());
        }
        for (label, value) in [
            ("resumed Task id", self.resumed_from_task_id.as_deref()),
            ("parent Task id", self.parent_task_id.as_deref()),
            ("permission scope id", self.permission_scope_id.as_deref()),
            (
                "cancellation scope id",
                self.cancellation_scope_id.as_deref(),
            ),
        ] {
            if value.is_some_and(|value| value.trim().is_empty()) {
                return Err(format!("Task {label} must not be empty"));
            }
        }
        Ok(())
    }
}

impl TaskDurability {
    fn validate(&self) -> Result<(), String> {
        let has_owner = self.worker_id.is_some() || self.ownership_token.is_some();
        if has_owner && (self.worker_id.is_none() || self.ownership_token.is_none()) {
            return Err("Task lease must include both worker id and ownership token".to_string());
        }
        if self
            .worker_id
            .as_ref()
            .is_some_and(|value| value.trim().is_empty())
            || self
                .ownership_token
                .as_ref()
                .is_some_and(|value| value.trim().is_empty())
        {
            return Err("Task lease identifiers must not be empty".to_string());
        }
        if let (Some(heartbeat), Some(lease_until)) =
            (self.last_heartbeat_epoch_ms, self.lease_until_epoch_ms)
        {
            if heartbeat > lease_until {
                return Err("Task heartbeat must not be later than lease expiry".to_string());
            }
        }
        if let (Some(input), Some(completed)) = (self.input_watermark, self.completion_watermark) {
            if completed > input {
                return Err(
                    "Task completion watermark must not exceed its input snapshot".to_string(),
                );
            }
        }
        Ok(())
    }

    fn clear_lease(&mut self) {
        self.worker_id = None;
        self.ownership_token = None;
        self.lease_until_epoch_ms = None;
        self.last_heartbeat_epoch_ms = None;
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskHandle {
    pub id: String,
    pub owner_session_id: SessionId,
    pub owner_turn_id: TurnId,
    pub kind: TaskKind,
    pub state: TaskState,
    #[serde(default)]
    pub progress: Option<f64>,
    #[serde(default)]
    pub current_step: Option<String>,
    #[serde(default)]
    pub result_ref: Option<String>,
    #[serde(default)]
    pub cancel_requested: bool,
    #[serde(default)]
    pub cancel_requested_at_epoch_ms: Option<i64>,
    #[serde(default)]
    pub lineage: TaskLineage,
    #[serde(default)]
    pub durability: TaskDurability,
}

impl TaskHandle {
    fn validate(&self) -> Result<(), String> {
        if self.id.trim().is_empty() {
            return Err("Task id must not be empty".to_string());
        }
        if self
            .progress
            .is_some_and(|progress| !progress.is_finite() || !(0.0..=1.0).contains(&progress))
        {
            return Err("Task progress must be between 0 and 1".to_string());
        }
        if self
            .current_step
            .as_ref()
            .is_some_and(|step| step.trim().is_empty())
        {
            return Err("Task current step must not be empty".to_string());
        }
        if self
            .result_ref
            .as_ref()
            .is_some_and(|result_ref| result_ref.trim().is_empty())
        {
            return Err("Task result reference must not be empty".to_string());
        }
        self.lineage.validate(&self.id)?;
        self.durability.validate()?;
        match (&self.durability.run_result, &self.durability.result_digest) {
            (Some(result), Some(digest)) if self.kind == TaskKind::Agent && self.state == TaskState::Sealed => {
                result.validate()?;
                if result.computed_digest()? != *digest
                    || self.lineage.child_session_id.as_ref().map(|id| id.as_str()) != Some(result.conversation_id.as_str())
                    || self.durability.terminal_receipt_id.is_none() {
                    return Err("Task Run result does not match its sealed receipt".into());
                }
            }
            (None, None) if self.state != TaskState::Sealed => {}
            _ => return Err("sealed Task requires its exact Run result and digest".into()),
        }
        if self.kind == TaskKind::Agent && self.state == TaskState::Completed {
            return Err("agent Task cannot infer business completion from execution lifecycle".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TaskProjection {
    pub revision: u64,
    pub order: Vec<String>,
    pub tasks: BTreeMap<String, TaskHandle>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub guidance: BTreeMap<String, TaskGuidanceQueue>,
}

impl TaskProjection {
    /// Only an exact current-Run task admission can establish an external wait.
    /// Unknown, foreign and terminal IDs remain ordinary guarded tool calls.
    pub(crate) fn is_live_wait_call(
        &self,
        call: &super::protocol::ModelToolCall,
        session_id: &SessionId,
        turn_id: &TurnId,
    ) -> bool {
        if call.name != "wait_task" { return false; }
        let Some(id) = call.arguments.get("task_id").and_then(serde_json::Value::as_str) else {
            return false;
        };
        self.tasks.get(id).is_some_and(|task|
            task.owner_session_id == *session_id && task.owner_turn_id == *turn_id
                && !task.state.is_terminal() && !task.cancel_requested)
    }

    pub fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }

    pub(crate) fn promote(&mut self, task: TaskHandle) -> Result<TaskHandle, String> {
        task.validate()?;
        if task.state.is_terminal() {
            return Err("A promoted Task must be queued, running, or waiting".to_string());
        }
        if self.tasks.contains_key(&task.id) {
            return Err("Task id is already registered".to_string());
        }
        if let Some(parent_task_id) = task.lineage.parent_task_id.as_deref() {
            if !self.tasks.contains_key(parent_task_id) {
                return Err("Task parent is not registered".to_string());
            }
        }
        if let Some(child_session_id) = task.lineage.child_session_id.as_ref() {
            if let Some(previous) = task.lineage.resumed_from_task_id.as_ref() {
                let previous = self.tasks.get(previous).ok_or("Resumed Task is not registered")?;
                if !previous.state.is_terminal() || previous.owner_session_id != task.owner_session_id
                    || previous.kind != task.kind || previous.lineage.child_session_id.as_ref() != Some(child_session_id)
                    || previous.lineage.parent_task_id != task.lineage.parent_task_id {
                    return Err("Task resume requires a terminal predecessor with the same session and parent".into());
                }
            }
            let session_is_already_bound = self.tasks.values().any(|existing| {
                !existing.state.is_terminal()
                    && existing.lineage.child_session_id.as_ref() == Some(child_session_id)
            });
            if session_is_already_bound {
                return Err("Child session is already bound to a live Task".to_string());
            }
        }
        self.revision = self.revision.saturating_add(1);
        self.order.push(task.id.clone());
        self.tasks.insert(task.id.clone(), task.clone());
        Ok(task)
    }

    #[cfg(test)]
    pub(crate) fn next_handle(
        &self,
        task_id: &str,
        state: TaskState,
        progress: Option<f64>,
        current_step: Option<String>,
        result_ref: Option<String>,
        cancel_requested: bool,
    ) -> Result<TaskHandle, String> {
        self.next_handle_attested(
            task_id,
            state,
            progress,
            current_step,
            result_ref,
            cancel_requested,
            None,
            None,
        )
    }

    pub(crate) fn next_handle_attested(
        &self,
        task_id: &str,
        state: TaskState,
        progress: Option<f64>,
        current_step: Option<String>,
        result_ref: Option<String>,
        cancel_requested: bool,
        ownership_token: Option<&str>,
        completion_watermark: Option<i64>,
    ) -> Result<TaskHandle, String> {
        let current = self
            .tasks
            .get(task_id)
            .ok_or_else(|| "Task update references an unknown Task".to_string())?;
        if state == TaskState::Sealed || (current.kind == TaskKind::Agent && state == TaskState::Completed) {
            return Err("agent Task settlement requires the canonical Run receipt".into());
        }
        if !current.state.can_transition_to(state) {
            return Err(format!(
                "Task cannot transition from {:?} to {:?}",
                current.state, state
            ));
        }
        if let (Some(previous), Some(next)) = (current.progress, progress) {
            if next < previous {
                return Err("Task progress must not move backwards".to_string());
            }
        }
        if current.cancel_requested && !cancel_requested {
            return Err("Task cancellation request must not be cleared".to_string());
        }
        if current.durability.ownership_token.is_some()
            && current.durability.ownership_token.as_deref() != ownership_token
        {
            return Err("Task update does not own the running Task".to_string());
        }
        if !state.is_terminal() && completion_watermark.is_some() {
            return Err("Only a terminal Task may commit a completion watermark".to_string());
        }
        if state == TaskState::Completed {
            if self
                .guidance
                .get(task_id)
                .is_some_and(|queue| !queue.pending.is_empty())
            {
                return Err(
                    "Terminal Task must acknowledge all queued guidance before completion"
                        .to_string(),
                );
            }
        }
        if state.is_terminal() {
            match (current.durability.input_watermark, completion_watermark) {
                (Some(input), Some(completed)) if input == completed => {}
                (Some(_), _) => {
                    return Err("Terminal Task must commit its exact input watermark".to_string())
                }
                (None, Some(_)) => {
                    return Err(
                        "Task without an input snapshot cannot commit a completion watermark"
                            .to_string(),
                    )
                }
                (None, None) => {}
            }
        }
        let mut next = TaskHandle {
            id: current.id.clone(),
            owner_session_id: current.owner_session_id.clone(),
            owner_turn_id: current.owner_turn_id.clone(),
            kind: current.kind,
            state,
            progress,
            current_step,
            result_ref,
            cancel_requested,
            cancel_requested_at_epoch_ms: current.cancel_requested_at_epoch_ms,
            lineage: current.lineage.clone(),
            durability: current.durability.clone(),
        };
        if state.is_terminal() {
            next.durability.completion_watermark = completion_watermark;
            next.durability.clear_lease();
            next.durability.retry_at_epoch_ms = None;
        }
        next.validate()?;
        Ok(next)
    }

    pub(crate) fn replace(&mut self, task: TaskHandle) -> Result<TaskHandle, String> {
        task.validate()?;
        if !self.tasks.contains_key(&task.id) {
            return Err("Task update references an unknown Task".to_string());
        }
        self.revision = self.revision.saturating_add(1);
        self.tasks.insert(task.id.clone(), task.clone());
        Ok(task)
    }

    pub(crate) fn request_cancellation(&mut self, task_id: &str) -> Result<TaskHandle, String> {
        self.request_cancellation_at(task_id, None)
    }

    /// Requests cancellation for a Task and every live descendant. Children
    /// are returned before their parent so Hosts stop leaf executions before
    /// tearing down the scopes that own them.
    pub(crate) fn request_cancellation_tree(
        &mut self,
        task_id: &str,
    ) -> Result<Vec<(u64, TaskHandle)>, String> {
        let root = self
            .tasks
            .get(task_id)
            .ok_or_else(|| "Task cancellation references an unknown Task".to_string())?;
        if root.state.is_terminal() {
            return Err("A terminal Task cannot be cancelled".to_string());
        }
        if root.cancel_requested {
            return Err("Task cancellation was already requested".to_string());
        }

        fn visit(projection: &TaskProjection, parent_id: &str, ordered: &mut Vec<String>) {
            for child_id in &projection.order {
                let Some(child) = projection.tasks.get(child_id) else {
                    continue;
                };
                if child.lineage.parent_task_id.as_deref() != Some(parent_id) {
                    continue;
                }
                visit(projection, child_id, ordered);
                if !child.state.is_terminal() && !child.cancel_requested {
                    ordered.push(child_id.clone());
                }
            }
        }

        let mut ordered = Vec::new();
        visit(self, task_id, &mut ordered);
        ordered.push(task_id.to_string());

        let mut updated = Vec::with_capacity(ordered.len());
        for id in ordered {
            let task = self.request_cancellation(&id)?;
            updated.push((self.revision, task));
        }
        Ok(updated)
    }

    pub(crate) fn request_cancellation_at(
        &mut self,
        task_id: &str,
        requested_at_epoch_ms: Option<i64>,
    ) -> Result<TaskHandle, String> {
        let current = self
            .tasks
            .get(task_id)
            .ok_or_else(|| "Task cancellation references an unknown Task".to_string())?;
        if current.state.is_terminal() {
            return Err("A terminal Task cannot be cancelled".to_string());
        }
        if current.cancel_requested {
            return Err("Task cancellation was already requested".to_string());
        }
        let mut task = current.clone();
        task.cancel_requested = true;
        task.cancel_requested_at_epoch_ms = requested_at_epoch_ms;
        self.revision = self.revision.saturating_add(1);
        self.tasks.insert(task.id.clone(), task.clone());
        Ok(task)
    }

    pub(crate) fn reconcile_guidance(
        &mut self,
        fact: &TaskGuidanceResolution,
    ) -> Result<TaskHandle, String> {
        let mut task = self
            .tasks
            .get(&fact.task_id)
            .ok_or("guidance result references unknown task")?
            .clone();
        if task.kind != TaskKind::Agent
            || task.owner_session_id != fact.parent_session_id
            || task.owner_turn_id != fact.parent_turn_id
            || fact.input_watermark.is_some_and(|watermark| watermark < 0)
        {
            return Err("guidance result does not match canonical parent task identity".into());
        }
        if fact.message_id.trim().is_empty() || fact.content.trim().is_empty()
            || fact.content.chars().count() > MAX_TASK_GUIDANCE_CHARS {
            return Err("guidance projection contains invalid message identity or content".into());
        }
        if fact.status == "accepted" {
            if let Some(final_result) = self
                .guidance
                .get(&fact.task_id)
                .and_then(|queue| queue.results.get(&fact.message_id))
            {
                return if final_result.content == fact.content
                    && final_result.parent_session_id == fact.parent_session_id
                    && final_result.parent_turn_id == fact.parent_turn_id
                    && final_result.input_watermark == fact.input_watermark
                    && final_result.submitted_at_ms == fact.submitted_at_ms
                {
                    Ok(task)
                } else {
                    Err("replayed guidance admission conflicts with its final result".into())
                };
            }
            // Admission is decided once by the durable Task owner. This is a
            // display projection, not another terminal/cancellation/capacity gate.
            let queue = self.guidance.entry(fact.task_id.clone()).or_default();
            if let Some(existing) = queue.pending.iter().find(|item| item.message_id == fact.message_id) {
                return if existing.content == fact.content
                    && existing.parent_session_id == fact.parent_session_id
                    && existing.parent_turn_id == fact.parent_turn_id
                    && existing.input_watermark == fact.input_watermark
                    && existing.enqueued_at_epoch_ms == fact.submitted_at_ms {
                    Ok(task)
                } else { Err("guidance projection conflicts with accepted message".into()) };
            }
            let sequence = queue.last_sequence.checked_add(1).ok_or("guidance projection sequence exhausted")?;
            queue.last_sequence = sequence;
            queue.pending.push(TaskGuidanceMessage { sequence, message_id: fact.message_id.clone(),
                parent_session_id: fact.parent_session_id.clone(), parent_turn_id: fact.parent_turn_id.clone(),
                input_watermark: fact.input_watermark,
                content: fact.content.clone(), enqueued_at_epoch_ms: fact.submitted_at_ms });
            task.durability.input_watermark = task.durability.input_watermark.max(fact.input_watermark);
            self.tasks.insert(task.id.clone(), task.clone());
            self.revision = self.revision.saturating_add(1);
            return Ok(task);
        }
        if !matches!(fact.status.as_str(), "delivered" | "rejected")
            || (fact.status == "rejected" && fact.reason.is_none())
        {
            return Err("guidance result is not a final delivery outcome".into());
        }
        let queue = self.guidance.entry(fact.task_id.clone()).or_default();
        if let Some(existing) = queue.results.get(&fact.message_id) {
            return if existing == fact {
                Ok(task)
            } else {
                Err("guidance result conflicts with committed outcome".into())
            };
        }
        let pending = queue
            .pending
            .iter()
            .find(|message| message.message_id == fact.message_id);
        if let Some(message) = pending {
            if message.content != fact.content
                || message.parent_session_id != fact.parent_session_id
                || message.parent_turn_id != fact.parent_turn_id
                || message.input_watermark != fact.input_watermark
                || message.enqueued_at_epoch_ms != fact.submitted_at_ms
            {
                return Err("guidance result payload differs from accepted message".into());
            }
            queue.settled_sequences.insert(message.sequence);
        }
        queue
            .pending
            .retain(|message| message.message_id != fact.message_id);
        while queue
            .settled_sequences
            .remove(&queue.acknowledged_sequence.saturating_add(1))
        {
            queue.acknowledged_sequence = queue.acknowledged_sequence.saturating_add(1);
        }
        queue.results.insert(fact.message_id.clone(), fact.clone());
        task.durability.input_watermark = task.durability.input_watermark.max(fact.input_watermark);
        self.tasks.insert(task.id.clone(), task.clone());
        self.revision = self.revision.saturating_add(1);
        Ok(task)
    }

    pub(crate) fn claim(
        &mut self,
        task_id: &str,
        worker_id: String,
        ownership_token: String,
        now_epoch_ms: i64,
        lease_until_epoch_ms: i64,
        input_watermark: Option<i64>,
    ) -> Result<TaskHandle, String> {
        let current = self
            .tasks
            .get(task_id)
            .ok_or_else(|| "Task claim references an unknown Task".to_string())?;
        if !matches!(current.state, TaskState::Queued | TaskState::RetryWaiting) {
            return Err("Only queued or retry-waiting Tasks may be claimed".to_string());
        }
        if current.state == TaskState::RetryWaiting
            && current
                .durability
                .retry_at_epoch_ms
                .is_some_and(|retry_at| retry_at > now_epoch_ms)
        {
            return Err("Task retry delay has not elapsed".to_string());
        }
        if current
            .durability
            .lease_until_epoch_ms
            .is_some_and(|lease_until| lease_until > now_epoch_ms)
        {
            return Err("Task already has a live lease".to_string());
        }
        if lease_until_epoch_ms <= now_epoch_ms {
            return Err("Task lease expiry must be later than claim time".to_string());
        }
        if current.durability.input_watermark.is_some()
            && current.durability.input_watermark != input_watermark
        {
            return Err("Task input watermark cannot change between attempts".to_string());
        }
        let mut task = current.clone();
        task.state = TaskState::Running;
        task.durability.worker_id = Some(worker_id);
        task.durability.ownership_token = Some(ownership_token);
        task.durability.last_heartbeat_epoch_ms = Some(now_epoch_ms);
        task.durability.lease_until_epoch_ms = Some(lease_until_epoch_ms);
        task.durability.retry_at_epoch_ms = None;
        task.durability.input_watermark = input_watermark;
        self.replace(task)
    }

    pub(crate) fn reconcile_lease_recovery(
        &mut self,
        fact: TaskLeaseRecovery,
    ) -> Result<TaskHandle, String> {
        let current = self
            .tasks
            .get(&fact.task_id)
            .ok_or("expired execution references unknown Task")?;
        if fact.recovery_id.trim().is_empty()
            || fact.error.trim().is_empty()
            || fact.expired_worker_id.trim().is_empty()
            || fact.expired_ownership_token.trim().is_empty()
            || fact.expired_at_epoch_ms > fact.recovered_at_epoch_ms
            || fact.retry_at_epoch_ms < fact.recovered_at_epoch_ms
            || current.kind != TaskKind::Agent
            || current.cancel_requested
            || current.owner_session_id != fact.parent_session_id
            || current.owner_turn_id != fact.parent_turn_id
            || current.lineage.child_session_id.as_ref() != Some(&fact.child_session_id)
            || current
                .durability
                .input_watermark
                .is_some_and(|watermark| Some(watermark) != fact.input_watermark)
            || current
                .durability
                .last_heartbeat_epoch_ms
                .is_some_and(|at| at > fact.recovered_at_epoch_ms)
        {
            return Err("expired Task execution identity or clock differs".into());
        }
        match current.state {
            TaskState::Queued
                if current.durability.ownership_token.is_none()
                    && current.durability.worker_id.is_none()
                    && current.durability.lease_until_epoch_ms.is_none()
                    && current.durability.last_heartbeat_epoch_ms.is_none() => {}
            TaskState::Running
                if current.durability.ownership_token.as_deref()
                    == Some(fact.expired_ownership_token.as_str())
                    && current.durability.worker_id.as_deref()
                        == Some(fact.expired_worker_id.as_str()) => {}
            _ => return Err("expired Task execution no longer owns its parent".into()),
        }
        if fact.retry_count <= current.durability.retry_count {
            return Err("expired Task retry count did not advance".into());
        }
        let mut task = current.clone();
        task.state = TaskState::RetryWaiting;
        task.current_step = None;
        task.durability.clear_lease();
        task.durability.input_watermark = fact.input_watermark;
        task.durability.retry_count = fact.retry_count;
        task.durability.retry_at_epoch_ms = Some(fact.retry_at_epoch_ms);
        task.durability.last_error = Some(fact.error);
        self.replace(task)
    }

    pub(crate) fn resume_after_permission(
        &mut self,
        task_id: &str,
        previous_ownership_token: &str,
        worker_id: String,
        ownership_token: String,
        now_epoch_ms: i64,
        lease_until_epoch_ms: i64,
        input_watermark: Option<i64>,
    ) -> Result<TaskHandle, String> {
        self.handoff_after_permission(
            task_id,
            &[previous_ownership_token],
            false,
            worker_id,
            ownership_token,
            now_epoch_ms,
            lease_until_epoch_ms,
            input_watermark,
        )
    }

    pub fn reconcile_terminal(
        &mut self,
        fact: &TaskTerminalReconciliation,
        owner: &SessionId,
    ) -> Result<TaskHandle, String> {
        fact.validate()?;
        if fact.guidance_results.len() > MAX_TASK_GUIDANCE_QUEUE
            || fact.guidance_results.iter().any(|result| result.content.chars().count() > MAX_TASK_GUIDANCE_CHARS) {
            return Err("terminal Task guidance exceeds its bounded pending queue".into());
        }
        let mut staged = self.clone();
        for result in &fact.guidance_results {
            staged.reconcile_guidance(&TaskGuidanceResolution {
                task_id: result.task_id.clone(), message_id: result.message_id.clone(),
                parent_session_id: SessionId::from_string(result.parent_session_id.clone()),
                parent_turn_id: TurnId::from_string(result.parent_turn_id.clone()),
                input_watermark: result.input_watermark, content: result.content.clone(),
                submitted_at_ms: result.submitted_at_ms, status: result.status.clone(), reason: result.reason.clone(),
            })?;
        }
        let task = staged.reconcile_terminal_inner(fact, owner)?;
        *self = staged;
        Ok(task)
    }
    fn reconcile_terminal_inner(
        &mut self,
        fact: &TaskTerminalReconciliation,
        owner: &SessionId,
    ) -> Result<TaskHandle, String> {
        let current = self
            .tasks
            .get(&fact.admission.task_id)
            .ok_or("terminal receipt references unknown Task")?;
        let state = match fact.status.as_str() {
            "sealed" => TaskState::Sealed, "failed" => TaskState::Failed, "cancelled" => TaskState::Cancelled,
            _ => return Err("Task projection is not terminal".into()),
        };
        if fact.receipt_id.trim().is_empty()
            || current.kind != TaskKind::Agent
            || &current.owner_session_id != owner
            || current.owner_session_id.as_str() != fact.admission.parent_core_session_id
            || current.owner_turn_id.as_str() != fact.admission.parent_turn_id
            || current.lineage.child_session_id.as_ref().map(|id| id.as_str()) != Some(fact.admission.child_conversation_id.as_str())
            || current.lineage.parent_call_id.as_ref().map(|id| id.as_str()) != Some(fact.admission.parent_tool_call_id.as_str())
            || current.durability.input_watermark > fact.input_watermark
            || fact.completion_watermark != fact.input_watermark
        {
            return Err("terminal receipt does not match Task identity".into());
        }
        if current.state.is_terminal() {
            return if current.durability.terminal_receipt_id.as_deref()
                == Some(fact.receipt_id.as_str())
                && current.state == state
                && current.result_ref == fact.result_ref
                && current.durability.run_result == fact.run_result
                && current.durability.result_digest == fact.result_digest
                && current.durability.completion_watermark == fact.completion_watermark
            {
                Ok(current.clone())
            } else {
                Err("terminal receipt conflicts with committed Task result".into())
            };
        }
        if self
                .guidance
                .get(&fact.admission.task_id)
                .is_some_and(|queue| !queue.pending.is_empty())
        {
            return Err("terminal receipt cannot skip unacknowledged Task guidance".into());
        }
        // Execution leases live exclusively in the Runtime Task aggregate.
        // Application has authenticated this fact against the source admission;
        // the parent projects it without copying historical worker authority.
        let mut task = current.clone();
        task.state = state;
        // A terminal receipt retires the live step. Keeping the previous
        // "queued"/"working" text makes every client display contradictory
        // ongoing work next to the terminal result. Historical progress stays
        // in the journal; this is the current Task projection.
        task.current_step = None;
        task.result_ref = fact.result_ref.clone();
        task.durability.run_result = fact.run_result.clone();
        task.durability.result_digest = fact.result_digest;
        // Retain observed progress. Sealing, including Answered, does not
        // manufacture 100% acceptance of the Task's delivery requirements.
        task.durability.completion_watermark = fact.completion_watermark;
        task.durability.input_watermark = fact.input_watermark;
        task.durability.terminal_receipt_id = Some(fact.receipt_id.clone());
        task.durability.clear_lease();
        task.durability.retry_at_epoch_ms = None;
        self.replace(task)
    }

    pub(crate) fn recover_after_permission(
        &mut self,
        task_id: &str,
        previous_ownership_tokens: &[String],
        worker_id: String,
        ownership_token: String,
        now_epoch_ms: i64,
        lease_until_epoch_ms: i64,
        input_watermark: Option<i64>,
    ) -> Result<TaskHandle, String> {
        let tokens = previous_ownership_tokens
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        self.handoff_after_permission(
            task_id,
            &tokens,
            true,
            worker_id,
            ownership_token,
            now_epoch_ms,
            lease_until_epoch_ms,
            input_watermark,
        )
    }

    fn handoff_after_permission(
        &mut self,
        task_id: &str,
        previous_ownership_tokens: &[&str],
        allow_running: bool,
        worker_id: String,
        ownership_token: String,
        now_epoch_ms: i64,
        lease_until_epoch_ms: i64,
        input_watermark: Option<i64>,
    ) -> Result<TaskHandle, String> {
        let current = self
            .tasks
            .get(task_id)
            .ok_or_else(|| "Permission resume references an unknown Task".to_string())?;
        let valid_state = matches!(current.state, TaskState::Waiting | TaskState::Suspended)
            || (allow_running && current.state == TaskState::Running);
        if !valid_state
            || current.cancel_requested
            || previous_ownership_tokens.is_empty()
            || previous_ownership_tokens
                .iter()
                .any(|token| token.trim().is_empty() || token.trim() != *token)
            || previous_ownership_tokens
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len()
                != previous_ownership_tokens.len()
            || !current
                .durability
                .ownership_token
                .as_deref()
                .is_some_and(|current| previous_ownership_tokens.contains(&current))
            || ownership_token.trim().is_empty()
            || previous_ownership_tokens.contains(&ownership_token.as_str())
            || worker_id.trim().is_empty()
            || lease_until_epoch_ms <= now_epoch_ms
            || current
                .durability
                .last_heartbeat_epoch_ms
                .is_some_and(|last| last > now_epoch_ms)
            || current.durability.input_watermark != input_watermark
        {
            return Err("Permission resume does not match the suspended Task authority".into());
        }
        let mut task = current.clone();
        task.state = TaskState::Running;
        task.durability.worker_id = Some(worker_id);
        task.durability.ownership_token = Some(ownership_token);
        task.durability.last_heartbeat_epoch_ms = Some(now_epoch_ms);
        task.durability.lease_until_epoch_ms = Some(lease_until_epoch_ms);
        self.replace(task)
    }

    pub(crate) fn heartbeat(
        &mut self,
        task_id: &str,
        ownership_token: &str,
        now_epoch_ms: i64,
        lease_until_epoch_ms: i64,
    ) -> Result<TaskHandle, String> {
        let current = self
            .tasks
            .get(task_id)
            .ok_or_else(|| "Task heartbeat references an unknown Task".to_string())?;
        if !matches!(current.state, TaskState::Running | TaskState::Waiting)
            || current.durability.ownership_token.as_deref() != Some(ownership_token)
        {
            return Err("Task heartbeat does not own the live Task".to_string());
        }
        if lease_until_epoch_ms <= now_epoch_ms {
            return Err("Task heartbeat must extend the lease into the future".to_string());
        }
        if current
            .durability
            .last_heartbeat_epoch_ms
            .is_some_and(|last| now_epoch_ms < last)
        {
            return Err("Task heartbeat time must not move backwards".to_string());
        }
        if current
            .durability
            .lease_until_epoch_ms
            .is_some_and(|current_expiry| lease_until_epoch_ms < current_expiry)
        {
            return Err("Task heartbeat must not shorten the active lease".to_string());
        }
        let mut task = current.clone();
        task.durability.last_heartbeat_epoch_ms = Some(now_epoch_ms);
        task.durability.lease_until_epoch_ms = Some(lease_until_epoch_ms);
        self.replace(task)
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authoritative_guidance_projects_without_a_second_admission_decision() {
        let mut projection=TaskProjection::default();
        let mut record=task("agent",TaskState::Failed);
        record.kind=TaskKind::Agent;
        record.cancel_requested=true;
        projection.tasks.insert(record.id.clone(),record);
        let mut fact=TaskGuidanceResolution {task_id:"agent".into(),message_id:"message".into(),
            parent_session_id:SessionId::from_string("session".into()),parent_turn_id:TurnId::from_string("turn".into()),
            input_watermark:None,content:"  原样内容  ".into(),submitted_at_ms:10,status:"delivered".into(),reason:None};
        // Delivery can commit before the parent projects acceptance, and before
        // the parent drains a previously committed terminal display update.
        projection.reconcile_guidance(&fact).unwrap();
        let settled=projection.clone();
        projection.reconcile_guidance(&fact).unwrap();
        assert_eq!(projection,settled);
        fact.status="accepted".into();
        projection.reconcile_guidance(&fact).unwrap();
        assert_eq!(projection,settled);
        fact.submitted_at_ms=11;
        assert!(projection.reconcile_guidance(&fact).is_err());
        assert_eq!(projection,settled);
        fact.submitted_at_ms=10;
        fact.content="原样内容".into();
        assert!(projection.reconcile_guidance(&fact).is_err());
        assert_eq!(projection,settled);
        fact.content="  原样内容  ".into();
        fact.message_id="accepted".into();
        fact.input_watermark=Some(7);
        projection.reconcile_guidance(&fact).unwrap();
        assert_eq!(projection.guidance["agent"].pending[0].content,"  原样内容  ");
        let mut newer=fact.clone(); newer.message_id="newer".into(); newer.input_watermark=Some(8);
        projection.reconcile_guidance(&newer).unwrap();
        fact.status="delivered".into();
        let before=projection.clone();
        let mut wrong=fact.clone();wrong.input_watermark=Some(6);
        assert!(projection.reconcile_guidance(&wrong).is_err());
        assert_eq!(projection,before);
        projection.reconcile_guidance(&fact).unwrap();
        assert_eq!(projection.tasks["agent"].durability.input_watermark,Some(8));
        assert_eq!(projection.guidance["agent"].results["accepted"].input_watermark,Some(7));
    }

    fn task(id: &str, state: TaskState) -> TaskHandle {
        TaskHandle {
            id: id.to_string(),
            owner_session_id: SessionId::from_string("session".to_string()),
            owner_turn_id: TurnId::from_string("turn".to_string()),
            kind: TaskKind::DataRefresh,
            state,
            progress: Some(0.0),
            current_step: Some("queued".to_string()),
            result_ref: None,
            cancel_requested: false,
            cancel_requested_at_epoch_ms: None,
            lineage: TaskLineage::default(),
            durability: TaskDurability::default(),
        }
    }

    #[test]
    fn external_wait_requires_live_task_owned_by_exact_run() {
        let mut projection = TaskProjection::default();
        let mut record = task("child", TaskState::Waiting);
        record.kind = TaskKind::Agent;
        record.lineage.child_session_id = Some(SessionId::from_string("child-session".into()));
        projection.promote(record.clone()).unwrap();
        let call: crate::agent::protocol::ModelToolCall = serde_json::from_value(
            serde_json::json!({"name":"wait_task","arguments":{"task_id":"child","timeout_sec":1}})
        ).unwrap();
        let accepts = |projection: &TaskProjection| projection.is_live_wait_call(
            &call, &record.owner_session_id, &record.owner_turn_id);
        for _ in 0..10 { assert!(accepts(&projection)); }
        assert!(!projection.is_live_wait_call(&call, &SessionId::from_string("foreign".into()), &record.owner_turn_id));
        assert!(!projection.is_live_wait_call(&call, &record.owner_session_id, &TurnId::from_string("next-run".into())));
        let mut unknown = call.clone();
        unknown.arguments["task_id"] = serde_json::json!("unknown");
        assert!(!projection.is_live_wait_call(&unknown, &record.owner_session_id, &record.owner_turn_id));
        unknown = call.clone(); unknown.name = "read_task".into();
        assert!(!projection.is_live_wait_call(&unknown, &record.owner_session_id, &record.owner_turn_id));
        projection.tasks.get_mut("child").unwrap().cancel_requested = true;
        assert!(!accepts(&projection));
        projection.tasks.get_mut("child").unwrap().cancel_requested = false;
        for state in [TaskState::Completed, TaskState::Sealed, TaskState::Failed, TaskState::Cancelled] {
            projection.tasks.get_mut("child").unwrap().state = state;
            assert!(!accepts(&projection));
        }
    }

    #[test]
    fn runtime_expiry_fact_retires_only_its_exact_parent_authority() {
        for initially_claimed in [false, true] {
            let mut projection = TaskProjection::default();
            let mut record = task("agent", TaskState::Queued);
            record.kind = TaskKind::Agent;
            record.lineage.child_session_id = Some(SessionId::from_string("child".into()));
            projection.promote(record).unwrap();
            if initially_claimed {
                projection
                    .claim("agent", "worker".into(), "old".into(), 100, 200, Some(7))
                    .unwrap();
            }
            let fact = TaskLeaseRecovery {
                recovery_id: "recovery-original-attempt".into(),
                task_id: "agent".into(),
                parent_session_id: SessionId::from_string("session".into()),
                parent_turn_id: TurnId::from_string("turn".into()),
                child_session_id: SessionId::from_string("child".into()),
                expired_worker_id: "worker".into(),
                expired_ownership_token: "old".into(),
                expired_at_epoch_ms: 200,
                input_watermark: Some(7),
                retry_count: 1,
                retry_at_epoch_ms: 300,
                recovered_at_epoch_ms: 300,
                error: "runtime lease expired".into(),
            };
            for invalid in ["parent", "turn", "child", "clock", "count"] {
                let mut wrong = fact.clone();
                match invalid {
                    "parent" => wrong.parent_session_id = SessionId::from_string("foreign".into()),
                    "turn" => wrong.parent_turn_id = TurnId::from_string("foreign".into()),
                    "child" => wrong.child_session_id = SessionId::from_string("foreign".into()),
                    "clock" => wrong.expired_at_epoch_ms = 301,
                    "count" => wrong.retry_count = 0,
                    _ => unreachable!(),
                }
                let before = serde_json::to_value(&projection).unwrap();
                assert!(projection.reconcile_lease_recovery(wrong).is_err());
                assert_eq!(
                    serde_json::to_value(&projection).unwrap(),
                    before,
                    "failed retirement never mutates {invalid}"
                );
            }
            let recovered = projection.reconcile_lease_recovery(fact).unwrap();
            assert_eq!(recovered.state, TaskState::RetryWaiting);
            assert!(recovered.durability.ownership_token.is_none());
            assert_eq!(recovered.durability.retry_count, 1);
            assert_eq!(recovered.durability.retry_at_epoch_ms, Some(300));
            assert!(projection
                .claim("agent", "early-worker".into(), "early".into(), 299, 400, Some(7))
                .is_err());
            projection
                .claim(
                    "agent",
                    "next-worker".into(),
                    "next".into(),
                    300,
                    400,
                    Some(7),
                )
                .unwrap();
            assert!(
                projection
                    .claim(
                        "agent",
                        "old-worker".into(),
                        "old".into(),
                        301,
                        401,
                        Some(7)
                    )
                    .is_err(),
                "Running claim exclusion remains strict"
            );
        }
    }

    #[test]
    fn lineage_rejects_cycles_and_survives_state_updates() {
        let mut invalid = task("task-1", TaskState::Running);
        invalid.lineage.parent_task_id = Some("task-1".to_string());
        assert_eq!(
            invalid.validate(),
            Err("Task cannot be its own parent".to_string())
        );

        let mut projection = TaskProjection::default();
        projection
            .promote(task("task-parent", TaskState::Running))
            .expect("promote parent Task");
        let mut promoted = task("task-2", TaskState::Running);
        promoted.lineage = TaskLineage {
            resumed_from_task_id: None,
            parent_task_id: Some("task-parent".to_string()),
            child_session_id: Some(SessionId::from_string("child-session".to_string())),
            parent_call_id: Some(CallId::from_string("call-parent".to_string())),
            permission_scope_id: Some("permission-session".to_string()),
            cancellation_scope_id: Some("cancel-child".to_string()),
        };
        projection.promote(promoted.clone()).expect("promote Task");
        let updated = projection
            .next_handle(
                "task-2",
                TaskState::Waiting,
                Some(0.5),
                Some("waiting for permission".to_string()),
                None,
                false,
            )
            .expect("update Task");
        assert_eq!(updated.lineage, promoted.lineage);
    }

    #[test]
    fn resumed_execution_is_not_a_child_of_its_predecessor() {
        let mut projection = TaskProjection::default();
        let mut previous = task("previous", TaskState::Completed);
        previous.lineage.child_session_id = Some(SessionId::from_string("shared-child".into()));
        projection.tasks.insert(previous.id.clone(), previous.clone());
        let mut resumed = task("resumed", TaskState::Queued);
        resumed.lineage.resumed_from_task_id = Some(previous.id.clone());
        resumed.lineage.child_session_id = previous.lineage.child_session_id.clone();
        assert!(projection.promote(resumed.clone()).is_ok());
        assert!(projection.tasks["resumed"].lineage.parent_task_id.is_none());
        let mut invalid = TaskProjection::default();
        previous.state = TaskState::Running;
        invalid.tasks.insert(previous.id.clone(), previous);
        assert!(invalid.promote(resumed).is_err());
    }

    #[test]
    fn projection_requires_registered_parents_and_unique_live_child_sessions() {
        let mut projection = TaskProjection::default();
        let mut orphan = task("orphan", TaskState::Queued);
        orphan.lineage.parent_task_id = Some("missing".to_string());
        assert_eq!(
            projection.promote(orphan),
            Err("Task parent is not registered".to_string())
        );

        projection
            .promote(task("parent", TaskState::Running))
            .expect("promote parent");
        let child_session = SessionId::from_string("child-session".to_string());
        let mut first = task("child-1", TaskState::Running);
        first.lineage.parent_task_id = Some("parent".to_string());
        first.lineage.child_session_id = Some(child_session.clone());
        projection.promote(first).expect("promote first child");

        let mut duplicate = task("child-2", TaskState::Running);
        duplicate.lineage.parent_task_id = Some("parent".to_string());
        duplicate.lineage.child_session_id = Some(child_session);
        assert_eq!(
            projection.promote(duplicate),
            Err("Child session is already bound to a live Task".to_string())
        );
    }

    #[test]
    fn cancellation_propagates_to_descendants_before_their_parent() {
        let mut projection = TaskProjection::default();
        projection
            .promote(task("parent", TaskState::Running))
            .expect("promote parent");

        let mut child = task("child", TaskState::Running);
        child.lineage.parent_task_id = Some("parent".to_string());
        projection.promote(child).expect("promote child");

        let mut grandchild = task("grandchild", TaskState::Waiting);
        grandchild.lineage.parent_task_id = Some("child".to_string());
        projection.promote(grandchild).expect("promote grandchild");

        let updates = projection
            .request_cancellation_tree("parent")
            .expect("cancel Task tree");
        assert_eq!(
            updates
                .iter()
                .map(|(_, task)| task.id.as_str())
                .collect::<Vec<_>>(),
            vec!["grandchild", "child", "parent"]
        );
        assert!(updates.iter().all(|(_, task)| task.cancel_requested));
        assert!(updates.windows(2).all(|pair| pair[0].0 < pair[1].0));
    }

    #[test]
    fn lifecycle_supports_waiting_resume_and_each_terminal_state() {
        for terminal in [
            TaskState::Completed,
            TaskState::Failed,
            TaskState::Cancelled,
        ] {
            let mut projection = TaskProjection::default();
            projection
                .promote(task("refresh", TaskState::Queued))
                .unwrap();
            let running = projection
                .next_handle(
                    "refresh",
                    TaskState::Running,
                    Some(0.2),
                    Some("fetching".to_string()),
                    None,
                    false,
                )
                .unwrap();
            projection.replace(running).unwrap();
            let waiting = projection
                .next_handle(
                    "refresh",
                    TaskState::Waiting,
                    Some(0.4),
                    Some("waiting for network".to_string()),
                    None,
                    true,
                )
                .unwrap();
            projection.replace(waiting).unwrap();
            let resumed = projection
                .next_handle(
                    "refresh",
                    TaskState::Running,
                    Some(0.6),
                    Some("parsing".to_string()),
                    None,
                    true,
                )
                .unwrap();
            projection.replace(resumed).unwrap();
            let terminal_handle = projection
                .next_handle(
                    "refresh",
                    terminal,
                    Some(1.0),
                    Some("finished".to_string()),
                    Some("result://refresh".to_string()),
                    true,
                )
                .unwrap();
            projection.replace(terminal_handle).unwrap();
            assert_eq!(projection.tasks["refresh"].state, terminal);
            assert!(projection
                .next_handle(
                    "refresh",
                    terminal,
                    Some(1.0),
                    Some("duplicate".to_string()),
                    None,
                    true,
                )
                .is_err());
        }
    }

    #[test]
    fn lifecycle_rejects_implicit_terminal_promotion_and_regressions() {
        let mut projection = TaskProjection::default();
        assert!(projection
            .promote(task("terminal", TaskState::Completed))
            .is_err());
        projection
            .promote(task("active", TaskState::Running))
            .unwrap();
        assert!(projection
            .next_handle(
                "active",
                TaskState::Queued,
                Some(0.1),
                Some("backwards".to_string()),
                None,
                false,
            )
            .is_err());
        let progressed = projection
            .next_handle(
                "active",
                TaskState::Running,
                Some(0.5),
                Some("working".to_string()),
                None,
                true,
            )
            .unwrap();
        projection.replace(progressed).unwrap();
        assert!(projection
            .next_handle(
                "active",
                TaskState::Running,
                Some(0.4),
                Some("regressed".to_string()),
                None,
                true,
            )
            .is_err());
        assert!(projection
            .next_handle(
                "active",
                TaskState::Waiting,
                Some(0.6),
                Some("waiting".to_string()),
                None,
                false,
            )
            .is_err());
    }

    #[test]
    fn durable_claim_requires_owner_and_heartbeat_token() {
        let mut projection = TaskProjection::default();
        projection
            .promote(task("memory", TaskState::Queued))
            .unwrap();
        let claimed = projection
            .claim(
                "memory",
                "worker-a".to_string(),
                "token-a".to_string(),
                100,
                200,
                Some(7),
            )
            .unwrap();
        assert_eq!(claimed.state, TaskState::Running);
        assert!(projection.heartbeat("memory", "token-b", 150, 250).is_err());
        let heartbeat = projection.heartbeat("memory", "token-a", 150, 250).unwrap();
        assert_eq!(heartbeat.durability.last_heartbeat_epoch_ms, Some(150));
        assert_eq!(heartbeat.durability.lease_until_epoch_ms, Some(250));
        assert_eq!(heartbeat.durability.input_watermark, Some(7));
        assert!(projection.heartbeat("memory", "token-a", 149, 260).is_err());
        assert!(projection.heartbeat("memory", "token-a", 160, 249).is_err());

        let waiting = projection
            .next_handle_attested(
                "memory",
                TaskState::Waiting,
                None,
                Some("waiting for permission".to_string()),
                None,
                false,
                Some("token-a"),
                None,
            )
            .unwrap();
        projection.replace(waiting).unwrap();
        let waiting_heartbeat = projection.heartbeat("memory", "token-a", 170, 270).unwrap();
        assert_eq!(waiting_heartbeat.state, TaskState::Waiting);
        assert_eq!(
            waiting_heartbeat.durability.last_heartbeat_epoch_ms,
            Some(170)
        );
    }

    #[test]
    fn permission_resume_hands_off_authority_after_pause_without_reviving_old_token() {
        let mut projection = TaskProjection::default();
        projection
            .promote(task("permission", TaskState::Queued))
            .unwrap();
        projection
            .claim(
                "permission",
                "worker-a".into(),
                "old-token".into(),
                100,
                200,
                Some(7),
            )
            .unwrap();
        let waiting = projection
            .next_handle_attested(
                "permission",
                TaskState::Waiting,
                None,
                Some("permission".into()),
                None,
                false,
                Some("old-token"),
                None,
            )
            .unwrap();
        projection.replace(waiting).unwrap();
        let original = projection.clone();
        for (old, new, watermark, now) in [
            ("foreign", "new-token", Some(7), 300),
            ("old-token", "old-token", Some(7), 300),
            ("old-token", "new-token", Some(8), 300),
            ("old-token", "new-token", Some(7), 99),
        ] {
            assert!(projection
                .resume_after_permission(
                    "permission",
                    old,
                    "worker-b".into(),
                    new.into(),
                    now,
                    500,
                    watermark
                )
                .is_err());
            assert_eq!(projection, original);
        }
        let resumed = projection
            .resume_after_permission(
                "permission",
                "old-token",
                "worker-b".into(),
                "new-token".into(),
                300,
                500,
                Some(7),
            )
            .unwrap();
        assert_eq!(resumed.state, TaskState::Running);
        assert!(projection
            .heartbeat("permission", "old-token", 310, 510)
            .is_err());
        assert!(projection
            .heartbeat("permission", "new-token", 310, 510)
            .is_ok());
        assert!(projection
            .resume_after_permission(
                "permission",
                "old-token",
                "worker-c".into(),
                "third-token".into(),
                320,
                520,
                Some(7)
            )
            .is_err());
    }

    #[test]
    fn terminal_transition_clears_lease_but_keeps_watermarks() {
        let mut projection = TaskProjection::default();
        let mut handle = task("consolidate", TaskState::Queued);
        handle.durability.input_watermark = Some(40);
        projection.promote(handle).unwrap();
        projection
            .claim(
                "consolidate",
                "worker".to_string(),
                "token".to_string(),
                10,
                20,
                Some(40),
            )
            .unwrap();
        let completed = projection
            .next_handle_attested(
                "consolidate",
                TaskState::Completed,
                Some(1.0),
                Some("finished".to_string()),
                Some("memory://snapshot/40".to_string()),
                false,
                Some("token"),
                Some(40),
            )
            .unwrap();
        let completed = projection.replace(completed).unwrap();
        assert!(completed.durability.ownership_token.is_none());
        assert_eq!(completed.durability.input_watermark, Some(40));
        assert_eq!(completed.durability.completion_watermark, Some(40));
    }

    #[test]
    fn accepted_guidance_is_idempotent_and_settled_before_completion() {
        let mut projection = TaskProjection::default();
        let mut handle = task("agent", TaskState::Queued);
        handle.kind = TaskKind::Agent;
        handle.durability.input_watermark = Some(10);
        projection.promote(handle).unwrap();
        projection
            .claim(
                "agent",
                "worker".to_string(),
                "token".to_string(),
                100,
                200,
                Some(10),
            )
            .unwrap();

        let mut fact = TaskGuidanceResolution {
            task_id: "agent".into(),
            message_id: "message-1".into(),
            parent_session_id: SessionId::from_string("session".into()),
            parent_turn_id: TurnId::from_string("turn".into()),
            input_watermark: Some(10),
            content: "Inspect the failing contract first.".into(),
            submitted_at_ms: 120,
            status: "accepted".into(),
            reason: None,
        };
        projection.reconcile_guidance(&fact).unwrap();
        assert_eq!(projection.guidance["agent"].last_sequence, 1);
        assert_eq!(projection.guidance["agent"].pending.len(), 1);
        let revision = projection.revision;
        projection.reconcile_guidance(&fact).unwrap();
        assert_eq!(projection.revision, revision);
        assert!(projection
            .next_handle_attested(
                "agent",
                TaskState::Completed,
                Some(1.0),
                Some("finished".to_string()),
                Some("task://result".to_string()),
                false,
                Some("token"),
                Some(10),
            )
            .is_err());
        let mut wrong = fact.clone();
        wrong.parent_turn_id = TurnId::from_string("wrong-turn".into());
        assert!(projection.reconcile_guidance(&wrong).is_err());
        fact.status = "delivered".into();
        projection.reconcile_guidance(&fact).unwrap();
        let acknowledged = &projection.guidance["agent"];
        assert_eq!(acknowledged.acknowledged_sequence, 1);
        assert!(acknowledged.pending.is_empty());
        assert!(projection
            .next_handle_attested(
                "agent",
                TaskState::Completed,
                Some(1.0),
                Some("finished".to_string()),
                Some("task://result".to_string()),
                false,
                Some("token"),
                Some(10),
            )
            .is_err(), "acknowledged guidance is not a substitute for the canonical Run receipt");
    }
}
