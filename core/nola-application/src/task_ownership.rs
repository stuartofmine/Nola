// SPDX-License-Identifier: Apache-2.0
//! Single durable Task authority: no parent model checkpoint authorizes work.
use crate::*;
use nola_agent_engine::agent::{
    task::{
        reduce_task_authority, TaskAuthorityCommand, TaskDurability, TaskHandle, TaskLeaseRecovery,
        TaskLineage,
    },
    SessionId, TaskKind, TaskState, TurnId,
};
use nola_store_port::runtime_task::RuntimeTaskRepository;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoverExpiredAgentTask {
    pub owner_conversation_id: String,
    pub lease: RuntimeTaskLeaseWitness,
    pub error: String,
    pub retry_at_ms: i64,
    pub now_ms: i64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimAgentTaskFair {
    pub owner_conversation_id: String,
    pub task_id: String,
    pub worker_id: String,
    pub worker_generation: String,
    pub lease_token: String,
    pub now_ms: i64,
    pub lease_until_ms: i64,
    pub max_running_global: u32,
    pub max_running_project: u32,
}

pub(crate) fn authority<R: nola_store_port::runtime_task::TaskAuthorityReadRepository + ?Sized>(
    store: &R,
    task: &nola_store_port::runtime_task::StoredRuntimeTask,
) -> Result<nola_domain::agent_task_admission::AgentTaskAdmission, String> {
    use nola_domain::agent_task_admission::RuntimeAgentTaskAuthorityPhase;
    let leaf = store
        .read_runtime_agent_task_authority(&task.id)
        .map_err(|e| e.to_string())?
        .ok_or("agent Task has no prepared/admitted authority")?;
    if leaf.phase != RuntimeAgentTaskAuthorityPhase::Admitted {
        return Err("agent Task is prepared, not admitted".into());
    }
    let admission = leaf.admission.ok_or("admitted Task has no source fact")?;
    admission.validate()?;
    if task.kind != "agent"
        || task.owner_session_id.as_deref() != Some(admission.parent_conversation_id.as_str())
        || task.child_session_id.as_deref() != Some(admission.child_conversation_id.as_str())
        || task.parent_tool_call_id.as_deref() != Some(admission.parent_tool_call_id.as_str())
        || admission
            .input_watermark
            .is_some_and(|initial| task.input_watermark.is_none_or(|current| current < initial))
    {
        return Err("Task authority immutable ownership or watermark differs".into());
    }
    Ok(admission)
}

pub(crate) fn core_task(
    task: &nola_store_port::runtime_task::StoredRuntimeTask,
    admission: &nola_domain::agent_task_admission::AgentTaskAdmission,
) -> Result<TaskHandle, String> {
    let state = match task.status.as_str() {
        "queued" => TaskState::Queued,
        "running" => TaskState::Running,
        "retry_waiting" => TaskState::RetryWaiting,
        "waiting_permission" => TaskState::Waiting,
        "suspended" | "awaiting_reconciliation" => TaskState::Suspended,
        "sealed" => return Err("sealed Task ownership must be projected from its exact terminal receipt".into()),
        "failed" => TaskState::Failed,
        "cancelled" => TaskState::Cancelled,
        _ => return Err("unknown durable Task state".into()),
    };
    Ok(TaskHandle {
        id: task.id.clone(),
        owner_session_id: SessionId::from_string(admission.parent_core_session_id.clone()),
        owner_turn_id: TurnId::from_string(admission.parent_turn_id.clone()),
        kind: TaskKind::Agent,
        state,
        progress: None,
        current_step: task.progress_summary.clone(),
        result_ref: task.output_path.clone(),
        cancel_requested: task.cancel_requested,
        cancel_requested_at_epoch_ms: task.cancel_requested_at_ms,
        lineage: TaskLineage {
            resumed_from_task_id: serde_json::from_str::<serde_json::Value>(&task.metadata_json)
                .ok().and_then(|value| value.get("resumed_from_task_id").and_then(|id| id.as_str()).map(str::to_owned)),
            parent_task_id: task.parent_task_id.clone(),
            child_session_id: Some(SessionId::from_string(
                admission.child_conversation_id.clone(),
            )),
            parent_call_id: Some(nola_agent_engine::agent::CallId::from_string(
                admission.parent_tool_call_id.clone(),
            )),
            ..Default::default()
        },
        durability: TaskDurability {
            worker_id: task.worker_id.clone(),
            ownership_token: task.ownership_token.clone(),
            lease_until_epoch_ms: task.lease_until_ms,
            last_heartbeat_epoch_ms: task.heartbeat_at_ms,
            retry_count: u32::try_from(task.retry_count)
                .map_err(|_| "Task retry counter overflow")?,
            retry_at_epoch_ms: task.retry_at_ms,
            last_error: task.last_error.clone(),
            input_watermark: task.input_watermark,
            completion_watermark: task.completion_watermark,
            ..Default::default()
        },
    })
}

pub(crate) fn recover<R: RuntimeTaskRepository + ?Sized>(
    store: &mut R,
    input: RecoverExpiredAgentTask,
) -> Result<bool, String> {
    let Some(old) = store
        .runtime_task_current_attempt(&input.lease.task_id)
        .map_err(|e| e.to_string())?
    else {
        return Ok(false);
    };
    if RuntimeTaskLeaseWitness::from(&old) != input.lease
        || old.state != "leased"
        || old.lease_expires_at_ms > input.now_ms
    {
        return Ok(false);
    }
    let task = store
        .runtime_task(&old.task_id)
        .map_err(|e| e.to_string())?
        .ok_or("expired Task is missing")?;
    let admission = authority(store, &task)?;
    if admission.parent_conversation_id != input.owner_conversation_id {
        return Err("expired Task parent identity differs".into());
    }
    if task.status != "running"
        || task.worker_id.as_deref() != Some(old.worker_id.as_str())
        || task.ownership_token.as_deref() != Some(old.lease_token.as_str())
        || task.lease_until_ms != Some(old.lease_expires_at_ms)
        || task.updated_at_ms > input.now_ms
        || store
            .pending_runtime_task_permission_suspension(&task.id)
            .map_err(|e| e.to_string())?
            .is_some()
    {
        return Ok(false);
    }
    if input.error.trim().is_empty() || input.retry_at_ms < input.now_ms {
        return Err("invalid expiry recovery clock/error".into());
    }
    if let Some(binding) = store
        .runtime_task_run_binding(&task.id)
        .map_err(|e| e.to_string())?
    {
        if store
            .reconcile_runtime_task_bound_run_terminal(&task.id, &binding.run_id, input.now_ms)
            .map_err(|e| e.to_string())?
        {
            return Ok(false);
        }
    }
    if !task.cancel_requested {
        let fact = TaskLeaseRecovery {
            recovery_id: format!("lease-recovery:{}:{}", old.id, old.revision),
            task_id: task.id.clone(),
            parent_session_id: SessionId::from_string(admission.parent_core_session_id.clone()),
            parent_turn_id: TurnId::from_string(admission.parent_turn_id.clone()),
            child_session_id: SessionId::from_string(admission.child_conversation_id.clone()),
            expired_worker_id: old.worker_id,
            expired_ownership_token: old.lease_token,
            expired_at_epoch_ms: old.lease_expires_at_ms,
            input_watermark: task.input_watermark,
            retry_count: u32::try_from(
                task.retry_count
                    .checked_add(1)
                    .ok_or("Task retry overflow")?,
            )
            .map_err(|_| "Task retry overflow")?,
            retry_at_epoch_ms: input.retry_at_ms,
            recovered_at_epoch_ms: input.now_ms,
            error: input.error.clone(),
        };
        reduce_task_authority(
            core_task(&task, &admission)?,
            TaskAuthorityCommand::RecoverExpired { fact },
        )?;
    }
    store
        .recover_expired_runtime_task(&task.id, &input.error, input.retry_at_ms, input.now_ms)
        .map_err(|e| e.to_string())
}

pub(crate) fn claim<R: RuntimeTaskRepository + ?Sized>(
    store: &mut R,
    input: ClaimAgentTaskFair,
) -> Result<Option<RuntimeTaskAttempt>, String> {
    let task = store
        .runtime_task(&input.task_id)
        .map_err(|e| e.to_string())?
        .ok_or("claim Task is missing")?;
    let admission = authority(store, &task)?;
    if admission.parent_conversation_id != input.owner_conversation_id {
        return Err("claim Task parent identity differs".into());
    }
    if task.cancel_requested {
        return Ok(None);
    }
    reduce_task_authority(
        core_task(&task, &admission)?,
        TaskAuthorityCommand::Claim {
            worker_id: input.worker_id.clone(),
            ownership_token: input.lease_token.clone(),
            now_epoch_ms: input.now_ms,
            lease_until_epoch_ms: input.lease_until_ms,
            input_watermark: task.input_watermark,
        },
    )?;
    store
        .claim_runtime_task_fair(
            &input.task_id,
            &input.worker_id,
            &input.worker_generation,
            &input.lease_token,
            input.now_ms,
            input.lease_until_ms,
            input.max_running_global,
            input.max_running_project,
        )
        .map_err(|e| e.to_string())
}
