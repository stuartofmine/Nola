// SPDX-License-Identifier: Apache-2.0
//! One-shot, revision-consistent recovery read model.

use nola_store_port::{
    consistent_read::ConsistentReadRepository, resume_snapshot::ResumeSnapshotRepository,
};
use serde::{Deserialize, Serialize};

use crate::{
    load_session_projection, runtime_task::runtime_task, work_step_transaction::transaction,
    ActiveRun, PermissionGrant, PermissionRequest, RuntimeTask, SessionProjection,
    WorkStepTransaction,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResumeMailboxCursor {
    pub task_id: String,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcknowledgeResumeSnapshot {
    pub conversation_id: String,
    pub checkpoint_revision: i64,
    pub mailbox_cursor: Vec<ResumeMailboxCursor>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResumeAction {
    ResumeGoal,
    AwaitPermission,
    ReconcileSideEffects,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SupervisorTaskDisposition {
    Monitor,
    AwaitPermission,
    RetryScheduled,
    DeliverEvidence,
    Settled,
    RequiresIntervention,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SupervisorTaskReconciliation {
    pub task_id: String,
    pub status: String,
    pub disposition: SupervisorTaskDisposition,
    pub unread: bool,
    pub lease_expired: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SupervisorSideEffectDisposition {
    ReadyToExecute,
    CommitObservedResult,
    RequiresIntervention,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SupervisorSideEffectReconciliation {
    pub transaction_id: String,
    pub phase: crate::WorkStepTransactionPhase,
    pub disposition: SupervisorSideEffectDisposition,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SupervisorReconciliationReport {
    pub tasks: Vec<SupervisorTaskReconciliation>,
    pub side_effects: Vec<SupervisorSideEffectReconciliation>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResumeSnapshot {
    pub snapshot_version: u32,
    pub requested_at_ms: i64,
    pub projection: SessionProjection,
    pub active_run: Option<ActiveRun>,
    pub runtime_tasks: Vec<RuntimeTask>,
    pub mailbox_tasks: Vec<RuntimeTask>,
    pub mailbox_cursor: Vec<ResumeMailboxCursor>,
    pub effective_permission_grants: Vec<PermissionGrant>,
    pub pending_permission_requests: Vec<PermissionRequest>,
    pub incomplete_work_step_transactions: Vec<WorkStepTransaction>,
    pub supervisor_reconciliation: SupervisorReconciliationReport,
    pub action: ResumeAction,
}

pub fn load_resume_snapshot<S: ResumeSnapshotRepository + ConsistentReadRepository + nola_store_port::run_effect::RunEffectRepository + ?Sized>(
    store: &mut S,
    project_root: &str,
    host_id: Option<&str>,
    requested_at_ms: i64,
) -> Result<Option<ResumeSnapshot>, String> {
    let project_root = project_root.trim();
    if project_root.is_empty() {
        return Err("resume snapshot requires project_root".to_string());
    }
    store.with_consistent_read(|store| {
        let Some(projection) = find_resumable_projection(store, project_root)? else {
            return Ok(None);
        };
        let conversation_id = projection.conversation_id.as_str();
        let facts = store
            .load_resume_facts(conversation_id, project_root, host_id)
            .map_err(|error| format!("load consistent resume facts failed: {error}"))?;
        let active_run = facts.active_run.map(ActiveRun::from);
        let runtime_tasks = facts
            .runtime_tasks
            .into_iter()
            .map(runtime_task)
            .filter(|task| {
                task.owner_session_id
                    .as_deref()
                    .is_none_or(|owner| owner == conversation_id)
            })
            .collect::<Vec<_>>();
        let mailbox_tasks = runtime_tasks
            .iter()
            .filter(|task| task.unread && !is_live_task_status(&task.status))
            .cloned()
            .collect::<Vec<_>>();
        let mailbox_cursor = mailbox_tasks
            .iter()
            .map(|task| ResumeMailboxCursor {
                task_id: task.id.clone(),
                updated_at_ms: task.updated_at_ms,
            })
            .collect();
        let effective_permission_grants = facts
            .permission_grants
            .into_iter()
            .map(PermissionGrant::from_stored)
            .collect::<Result<Vec<_>, _>>()?;
        let pending_permission_requests = facts
            .pending_permissions
            .into_iter()
            .map(PermissionRequest::from_stored)
            .collect::<Result<Vec<_>, _>>()?;
        let incomplete_work_step_transactions = facts
            .incomplete_effects
            .into_iter()
            .map(transaction)
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .filter(|item| item.root == project_root)
            .collect::<Vec<_>>();
        let supervisor_reconciliation = SupervisorReconciliationReport {
            tasks: runtime_tasks
                .iter()
                .map(|task| reconcile_task(task, requested_at_ms))
                .collect(),
            side_effects: incomplete_work_step_transactions
                .iter()
                .map(reconcile_side_effect)
                .collect(),
        };
        let action = if !incomplete_work_step_transactions.is_empty() {
            ResumeAction::ReconcileSideEffects
        } else if !pending_permission_requests.is_empty() {
            ResumeAction::AwaitPermission
        } else {
            ResumeAction::ResumeGoal
        };
        Ok(Some(ResumeSnapshot {
            snapshot_version: 1,
            requested_at_ms,
            projection,
            active_run,
            runtime_tasks,
            mailbox_tasks,
            mailbox_cursor,
            effective_permission_grants,
            pending_permission_requests,
            incomplete_work_step_transactions,
            supervisor_reconciliation,
            action,
        }))
    })
}

fn reconcile_task(task: &RuntimeTask, requested_at_ms: i64) -> SupervisorTaskReconciliation {
    let lease_expired = task
        .lease_until_ms
        .is_some_and(|lease_until_ms| lease_until_ms <= requested_at_ms);
    let disposition = if lease_expired && is_live_task_status(&task.status) {
        SupervisorTaskDisposition::RequiresIntervention
    } else {
        match task.status.as_str() {
            "queued" | "running" => SupervisorTaskDisposition::Monitor,
            "waiting_permission" => SupervisorTaskDisposition::AwaitPermission,
            "retry_waiting" => SupervisorTaskDisposition::RetryScheduled,
            "sealed" | "completed" | "failed" | "cancelled" if task.unread => {
                SupervisorTaskDisposition::DeliverEvidence
            }
            "sealed" | "completed" | "failed" | "cancelled" => SupervisorTaskDisposition::Settled,
            _ => SupervisorTaskDisposition::RequiresIntervention,
        }
    };
    SupervisorTaskReconciliation {
        task_id: task.id.clone(),
        status: task.status.clone(),
        disposition,
        unread: task.unread,
        lease_expired,
    }
}

fn reconcile_side_effect(transaction: &WorkStepTransaction) -> SupervisorSideEffectReconciliation {
    use crate::WorkStepTransactionPhase;

    let disposition = match transaction.phase {
        WorkStepTransactionPhase::Prepared => SupervisorSideEffectDisposition::ReadyToExecute,
        WorkStepTransactionPhase::EffectObserved => {
            SupervisorSideEffectDisposition::CommitObservedResult
        }
        WorkStepTransactionPhase::Checkpointed
        | WorkStepTransactionPhase::Committed
        | WorkStepTransactionPhase::Failed => SupervisorSideEffectDisposition::RequiresIntervention,
    };
    SupervisorSideEffectReconciliation {
        transaction_id: transaction.id.clone(),
        phase: transaction.phase.clone(),
        disposition,
    }
}

pub fn acknowledge_resume_snapshot<S: ResumeSnapshotRepository + ?Sized>(
    store: &mut S,
    input: AcknowledgeResumeSnapshot,
) -> Result<(), String> {
    store
        .acknowledge_runtime_task_reads(
            &nola_store_port::resume_snapshot::AcknowledgeRuntimeTaskReadsInput {
                conversation_id: input.conversation_id,
                minimum_checkpoint_revision: input.checkpoint_revision,
                tasks: input
                    .mailbox_cursor
                    .into_iter()
                    .map(
                        |cursor| nola_store_port::resume_snapshot::RuntimeTaskReadCursor {
                            task_id: cursor.task_id,
                            updated_at_ms: cursor.updated_at_ms,
                        },
                    )
                    .collect(),
            },
        )
        .map_err(|error| error.to_string())
}

pub(crate) fn find_resumable_projection<S: ResumeSnapshotRepository + nola_store_port::run_effect::RunEffectRepository + ?Sized>(
    store: &S,
    project_root: &str,
) -> Result<Option<SessionProjection>, String> {
    for conversation_id in store
        .resumable_conversation_ids(project_root)
        .map_err(|error| error.to_string())?
    {
        let Some(projection) = load_session_projection(store, &conversation_id)? else {
            continue;
        };
        if projection
            .goal_projection
            .active
            .as_ref()
            .is_some_and(|goal| !goal.status.is_terminal())
        {
            return Ok(Some(projection));
        }
    }
    Ok(None)
}

fn is_live_task_status(status: &str) -> bool {
    matches!(
        status,
        "queued" | "running" | "retry_waiting" | "waiting_permission"
    )
}
