// SPDX-License-Identifier: Apache-2.0
//! User cancellation addresses immutable delegated-task ownership, never the
//! conversation's latest Run. The existing worker stop lifecycle stays owner.
use nola_store_port::runtime_task::RuntimeTaskRepository;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnedTaskCancellation {
    pub parent_conversation_id: String,
    pub parent_run_id: String,
    pub task_id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnedTaskCancellationReceipt {
    pub parent_conversation_id: String,
    pub parent_run_id: String,
    pub task_id: String,
    pub cancel_requested: bool,
    pub terminal: bool,
    pub state: String,
}

pub(crate) fn read<R: RuntimeTaskRepository + ?Sized>(store: &R, input: &OwnedTaskCancellation)
    -> Result<OwnedTaskCancellationReceipt, String> {
    if [&input.parent_conversation_id, &input.parent_run_id, &input.task_id].iter()
        .any(|id| id.is_empty() || id.trim() != *id || id.len() > 512 || id.chars().any(char::is_control)) {
        return Err("task cancellation requires exact bounded ownership".into());
    }
    let task = store.runtime_task(&input.task_id).map_err(|e| e.to_string())?
        .ok_or("task cancellation target is unavailable")?;
    let admission = crate::task_ownership::authority(store, &task)?;
    if admission.parent_conversation_id != input.parent_conversation_id
        || admission.parent_run_id != input.parent_run_id || admission.task_id != input.task_id {
        return Err("task cancellation belongs to another parent incarnation".into());
    }
    let terminal = match task.status.as_str() {
        "sealed" | "failed" | "cancelled" => true,
        "queued" | "running" | "retry_waiting" | "waiting_permission" | "suspended" | "awaiting_reconciliation" => false,
        _ => return Err("task cancellation target has an invalid durable state".into()),
    };
    Ok(OwnedTaskCancellationReceipt {parent_conversation_id: input.parent_conversation_id.clone(),
        parent_run_id: input.parent_run_id.clone(), task_id: input.task_id.clone(),
        cancel_requested: task.cancel_requested, terminal, state: task.status})
}

pub(crate) fn request<R: RuntimeTaskRepository + ?Sized>(store: &mut R, input: &OwnedTaskCancellation, now_ms: i64)
    -> Result<OwnedTaskCancellationReceipt, String> {
    let before = read(store, input)?;
    if now_ms < 0 { return Err("invalid task cancellation time".into()); }
    if !before.terminal && !before.cancel_requested {
        store.request_runtime_task_cancel(&input.task_id, now_ms).map_err(|e| e.to_string())?;
    }
    // The returned fact is the persisted cancellation request, not the change
    // count and not a claim that the child worker/Run has already stopped.
    read(store, input)
}

/// Parent-side user recovery cannot borrow another Task or today's parent Run.
/// The child remains read-only; only this explicit stop decision is available.
pub(crate) fn recovery_binding<R: crate::repositories::RuntimeExecutionRepositories>(
    store: &R, owner: &OwnedTaskCancellation,
) -> Result<nola_domain::task_execution::RuntimeTaskRunBinding,String> {
    let receipt=read(store,owner)?;
    if !receipt.cancel_requested { return Err("Task has no committed cancellation request".into()); }
    let task=store.runtime_task(&owner.task_id).map_err(|e|e.to_string())?.ok_or("Task missing")?;
    let binding=store.runtime_task_run_binding(&owner.task_id).map_err(|e|e.to_string())?.ok_or("Task has no admitted child Run")?;
    if binding.task_id!=owner.task_id || binding.owner_session_id!=owner.parent_conversation_id
        || task.child_session_id.as_deref()!=Some(binding.child_session_id.as_str()) {
        return Err("Task child binding changed".into());
    }
    Ok(binding)
}
