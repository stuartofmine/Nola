// SPDX-License-Identifier: Apache-2.0
//! Durable delegated guidance uses the existing parent/child FIFO and Run UoW.
use crate::*;
use nola_domain::agent_task_admission::{AgentTaskGuidanceProjection, AgentTaskTerminalGuidance};
use nola_agent_engine::agent::task::TaskGuidanceResolution;
use nola_agent_engine::agent::SessionId;
use nola_store_port::{
    agent_queue::{AgentQueueRepository, EnqueueAgentCommandInput},
    remote_session::CanonicalRunTerminalRepository,
    run_effect::RunEffectRepository,
    runtime_task::RuntimeTaskRepository,
};

/// Read the original immutable guidance receipt, even after its child Run has
/// finished or the current input watermark has advanced. Absence is distinct
/// from failure to validate the requesting parent incarnation.
pub(crate) fn receipt<R: RuntimeTaskRepository + ?Sized>(
    store: &R, task_id: &str, message_id: &str,
    parent_conversation_id: &str, parent_run_id: &str,
) -> Result<Option<RuntimeTaskGuidance>, String> {
    if [task_id, message_id, parent_conversation_id, parent_run_id].iter()
        .any(|id| id.trim().is_empty() || id.len() > 1024) {
        return Err("guidance receipt requires bounded exact identities".into());
    }
    let authority = store.runtime_agent_task_authority(task_id).map_err(|e| e.to_string())?
        .ok_or("guidance Task has no authority")?;
    authority.validate()?;
    let admission = authority.admission.ok_or("guidance Task is not admitted")?;
    if admission.task_id != task_id || admission.parent_conversation_id != parent_conversation_id
        || admission.parent_run_id != parent_run_id {
        return Err("guidance receipt belongs to a different parent incarnation".into());
    }
    let record = store.runtime_task_guidance(task_id, message_id).map_err(|e| e.to_string())?;
    if let Some(record) = &record {
        if record.task_id != task_id || record.message_id != message_id
            || record.owner_session_id != admission.parent_conversation_id
            || record.parent_core_session_id != admission.parent_core_session_id
            || record.parent_turn_id != admission.parent_turn_id
            || record.child_session_id != admission.child_conversation_id
            || record.input_watermark < admission.input_watermark {
            return Err("guidance receipt differs from immutable Task admission".into());
        }
    }
    Ok(record)
}

/// Availability is derived from the same immutable Task/Run facts used by
/// submission. A Task kind or worker lease alone never admits guidance.
pub(crate) fn available<R: nola_store_port::runtime_task::TaskAuthorityReadRepository + RunEffectRepository + CanonicalRunTerminalRepository + ?Sized>(
    store: &R, task_id: &str, core_session: &str, turn: &str,
) -> Result<bool, String> {
    let Some(task) = store.read_runtime_task(task_id).map_err(|e| e.to_string())? else { return Ok(false) };
    if task.kind != "agent" { return Ok(false) }
    let Some(binding) = store.read_runtime_task_run_binding(task_id).map_err(|e|e.to_string())? else { return Ok(false) };
    let admission = crate::task_ownership::authority(store, &task)?;
    if admission.parent_core_session_id != core_session || admission.parent_turn_id != turn {
        return Err("guidance capability belongs to another Task owner".into());
    }
    if binding.owner_session_id != admission.parent_conversation_id
        || binding.child_session_id != admission.child_conversation_id
        || binding.input_watermark > task.input_watermark {
        return Err("guidance capability has a foreign Task Run binding".into());
    }

    if store.read_canonical_run_terminal(&admission.parent_conversation_id, &admission.parent_run_id)
        .map_err(|e|e.to_string())?.is_some() { return Ok(false) }
    Ok(unavailable_target(store, task_id, &admission.parent_conversation_id,
        &binding.child_session_id, &binding.run_id, task.input_watermark)?.is_none())
}

pub(crate) fn export_guidance<R: RuntimeTaskRepository + ?Sized>(
    store: &R,
    task_id: &str,
    message_id: &str,
    settlement: bool,
) -> Result<AgentTaskGuidanceProjection, String> {
    let record = load(store, task_id, message_id)?;
    let authority = store
        .runtime_agent_task_authority(task_id)
        .map_err(|e| e.to_string())?
        .ok_or("guidance has no Task authority")?;
    authority.validate()?;
    let admission = authority.admission.ok_or("guidance Task is not admitted")?;
    let task = store
        .runtime_task(task_id)
        .map_err(|e| e.to_string())?
        .ok_or("guidance Task is missing")?;
    let binding = store
        .runtime_task_run_binding(task_id)
        .map_err(|e| e.to_string())?
        .ok_or("guidance Task has no Run binding")?;
    if task.kind != "agent"
        || task.owner_session_id.as_deref() != Some(admission.parent_conversation_id.as_str())
        || task.child_session_id.as_deref() != Some(admission.child_conversation_id.as_str())
        || task.parent_tool_call_id.as_deref() != Some(admission.parent_tool_call_id.as_str())
        || record.owner_session_id != admission.parent_conversation_id
        || record.child_session_id != admission.child_conversation_id
        || record.input_watermark > task.input_watermark
        || binding.run_id != record.run_id
        || binding.owner_session_id != record.owner_session_id
        || binding.child_session_id != record.child_session_id
        || binding.input_watermark > record.input_watermark
    {
        return Err("guidance outcome does not match authoritative Task/Run identity".into());
    }
    let status = if !settlement && record.accepted_at_ms.is_some() {
        "accepted"
    } else {
        record.status.as_str()
    };
    let projection = AgentTaskGuidanceProjection {
        admission,
        authority_revision: authority.authority_revision,
        settlement,
        guidance: AgentTaskTerminalGuidance {
            task_id: record.task_id,
            message_id: record.message_id,
            parent_session_id: record.parent_core_session_id,
            parent_turn_id: record.parent_turn_id,
            input_watermark: record.input_watermark,
            content: record.content,
            submitted_at_ms: record.submitted_at_ms,
            status: status.into(),
            reason: if status == "accepted" {
                None
            } else {
                record.reason
            },
        },
    };
    projection.validate()?;
    Ok(projection)
}

pub(crate) enum PreparedTaskGuidanceImport {
    Active(ReduceAndCommitActiveSubmission),
    Idle(ReduceAndCommitIdleSubmission),
}

pub(crate) fn prepare_guidance_import<R: RuntimeTaskRepository + RunEffectRepository + CanonicalRunTerminalRepository + ?Sized>(
    store: &R,
    projection: &AgentTaskGuidanceProjection,
) -> Result<PreparedTaskGuidanceImport, String> {
    projection.validate()?;
    let admission = &projection.admission;
    let original = crate::agent_task_admission::export(
        store,
        &admission.parent_conversation_id,
        &admission.parent_run_id,
        &admission.task_id,
        admission.core_event_cursor,
    )?;
    if original != *admission {
        return Err("guidance projection differs from original source admission".into());
    }
    let stored = crate::load_current_checkpoint(store, &admission.parent_conversation_id)?
        .ok_or("guidance source checkpoint missing")?;
    let checkpoint: CoreCheckpoint<AgentThreadState> =
        serde_json::from_str(&stored.checkpoint_json).map_err(|e| e.to_string())?;
    let state = &checkpoint.state;
    let task = state
        .task_projection
        .tasks
        .get(&admission.task_id)
        .ok_or("guidance source Task missing")?;
    if state.session_id.as_str() != admission.parent_core_session_id
        || task.owner_session_id.as_str() != admission.parent_core_session_id
        || task.owner_turn_id.as_str() != admission.parent_turn_id
        || task.lineage.child_session_id.as_ref().map(|id| id.as_str())
            != Some(admission.child_conversation_id.as_str())
        || task.lineage.parent_call_id.as_ref().map(|id| id.as_str())
            != Some(admission.parent_tool_call_id.as_str())
        || task.durability.input_watermark < admission.input_watermark
    {
        return Err("guidance projection targets a foreign source Task incarnation".into());
    }
    let item = &projection.guidance;
    let fact = TaskGuidanceResolution {
        task_id: item.task_id.clone(),
        message_id: item.message_id.clone(),
        parent_session_id: SessionId::from_string(item.parent_session_id.clone()),
        parent_turn_id: nola_agent_engine::agent::TurnId::from_string(
            item.parent_turn_id.clone(),
        ),
        input_watermark: item.input_watermark,
        content: item.content.clone(),
        submitted_at_ms: item.submitted_at_ms,
        status: item.status.clone(),
        reason: item.reason.clone(),
    };
    let operation_id = format!(
        "task-guidance-{}:{}:{}",
        if projection.settlement {
            "result"
        } else {
            "admit"
        },
        item.task_id,
        item.message_id
    );
    let submission = AgentSubmission::ReconcileTaskGuidance {
        operation_id: operation_id.clone(),
        fact,
    };
    let at = item.submitted_at_ms.max(stored.updated_at_ms);
    if let Some(active) = store
        .active_run(&admission.parent_conversation_id)
        .map_err(|e| e.to_string())?
    {
        Ok(PreparedTaskGuidanceImport::Active(
            ReduceAndCommitActiveSubmission {
                conversation_id: admission.parent_conversation_id.clone(),
                run_id: active.run_id,
                transition_id: operation_id,
                invocation_config_digest: active
                    .invocation_config_digest
                    .ok_or("guidance source active Run lacks invocation digest")?,
                submission,
                committed_at_ms: at,
            },
        ))
    } else {
        if !state.permits_new_turn_execution_configuration() {
            return Err("guidance source has unsettled execution without active Run".into());
        }
        Ok(PreparedTaskGuidanceImport::Idle(
            ReduceAndCommitIdleSubmission {
                conversation_id: admission.parent_conversation_id.clone(),
                core_config: AgentCoreConfig {
                    automation_execution_spec: None,
                    session_id: Some(state.session_id.clone()),
                    model: state.configuration.model.clone(),
                    max_model_iterations: state.configuration.max_model_iterations,
                    max_strategy_recovery_attempts: state.configuration.max_strategy_recovery_attempts,
                    initial_items: Vec::new(),
                    tools: state.configuration.tools.clone(),
                    context_window: state.configuration.context_window.clone(),
                    host_catalog: Some(state.host.catalog.clone()),
                    permission_rules: state.execution.permission_rules.clone(),
                    memory_policy: state.configuration.memory_policy.clone(),
                    turn_identity: state.configuration.turn_identity.clone(),
                },
                submission,
                committed_at_ms: at,
            },
        ))
    }
}

pub(crate) trait GuidanceRepositories:
    RuntimeTaskRepository + RunEffectRepository + CanonicalRunTerminalRepository + AgentQueueRepository
{
}
impl<
        T: RuntimeTaskRepository
            + RunEffectRepository
            + CanonicalRunTerminalRepository
            + AgentQueueRepository
            + ?Sized,
    > GuidanceRepositories for T
{
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskGuidanceApplyResult {
    pub guidance: RuntimeTaskGuidance,
    pub accepted: Option<Box<AcceptedActiveSubmission>>,
    pub recovered: Option<Box<AcceptedIdleSubmission>>,
}

fn load<R: RuntimeTaskRepository + ?Sized>(
    store: &R,
    task: &str,
    message: &str,
) -> Result<RuntimeTaskGuidance, String> {
    store
        .runtime_task_guidance(task, message)
        .map_err(|e| e.to_string())?
        .ok_or("guidance identity is not durable".into())
}

fn enqueue<R: AgentQueueRepository + ?Sized>(
    store: &mut R,
    owner: &str,
    input: nola_agent_engine::agent::host::AgentBoundaryInput,
    at: i64,
) -> Result<(), String> {
    use nola_agent_engine::agent::host::AgentBoundaryInput;
    let command_id = match &input {
        AgentBoundaryInput::TaskGuidanceAvailable { task_id, message_id, settlement } =>
            format!("task-guidance-{}:{task_id}:{message_id}", if *settlement { "result" } else { "admit" }),
        AgentBoundaryInput::Submission { submission: AgentSubmission::DeliverTaskGuidance { operation_id, .. } } => operation_id.clone(),
        _ => return Err("guidance command lacks stable identity".into()),
    };
    store
        .enqueue_agent_command(&EnqueueAgentCommandInput {
            session_id: owner.into(),
            command_id,
            payload_json: serde_json::to_string(&input).map_err(|e| e.to_string())?,
            created_at_ms: at,
        })
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn parent_command(record: &RuntimeTaskGuidance, settlement: bool) -> nola_agent_engine::agent::host::AgentBoundaryInput {
    nola_agent_engine::agent::host::AgentBoundaryInput::TaskGuidanceAvailable {
        task_id: record.task_id.clone(),
        message_id: record.message_id.clone(),
        settlement,
    }
}

fn final_notification<R: AgentQueueRepository + ?Sized>(
    store: &mut R,
    record: &RuntimeTaskGuidance,
) -> Result<(), String> {
    enqueue(
        store,
        &record.owner_session_id,
        parent_command(record, true),
        record.resolved_at_ms.unwrap_or(record.submitted_at_ms),
    )
}

fn unavailable<R: RuntimeTaskRepository + RunEffectRepository + CanonicalRunTerminalRepository + ?Sized>(
    store: &R, record: &RuntimeTaskGuidance,
) -> Result<Option<&'static str>, String> {
    unavailable_target(store, &record.task_id, &record.owner_session_id,
        &record.child_session_id, &record.run_id, record.input_watermark)
}

fn unavailable_target<R: nola_store_port::runtime_task::TaskAuthorityReadRepository + RunEffectRepository + CanonicalRunTerminalRepository + ?Sized>(
    store: &R, task_id: &str, owner_session_id: &str, child_session_id: &str,
    run_id: &str, input_watermark: Option<i64>,
) -> Result<Option<&'static str>, String> {
    let task = store
        .read_runtime_task(task_id)
        .map_err(|e| e.to_string())?
        .ok_or("guidance task is missing")?;
    let binding = store
        .read_runtime_task_run_binding(task_id)
        .map_err(|e| e.to_string())?
        .ok_or("guidance task Run binding is missing")?;
    if task.owner_session_id.as_deref() != Some(owner_session_id)
        || task.child_session_id.as_deref() != Some(child_session_id)
        || task.input_watermark < input_watermark
        || binding.run_id != run_id
        || binding.child_session_id != child_session_id
        || binding.owner_session_id != owner_session_id
        || binding.input_watermark > input_watermark
    {
        return Err("guidance immutable task/Run identity changed".into());
    }
    if task.input_watermark != input_watermark {
        return Ok(Some("task_input_changed_before_delivery"));
    }
    if matches!(task.status.as_str(), "sealed" | "failed" | "cancelled") {
        return Ok(Some("task_terminal_before_delivery"));
    }
    if task.cancel_requested {
        return Ok(Some("task_cancelled_before_delivery"));
    }
    if store
        .read_canonical_run_terminal(child_session_id, run_id)
        .map_err(|e| e.to_string())?
        .is_some()
    {
        return Ok(Some("bound_run_terminal_before_delivery"));
    }
    if store
        .active_run(child_session_id)
        .map_err(|e| e.to_string())?
        .as_ref()
        .map(|run| run.run_id.as_str())
        != Some(run_id)
    {
        return Ok(Some("bound_run_unavailable_before_delivery"));
    }
    Ok(None)
}

pub(crate) fn submit<R: GuidanceRepositories + ?Sized>(
    store: &mut R,
    task_id: String,
    message_id: String,
    parent_turn_id: String,
    content: String,
    at: i64,
) -> Result<RuntimeTaskGuidance, String> {
    if message_id.trim().is_empty()
        || content.trim().is_empty()
        || content.chars().count() > nola_agent_engine::agent::task::MAX_TASK_GUIDANCE_CHARS
    {
        return Err("guidance requires a stable ID and bounded nonempty content".into());
    }
    let task = store
        .runtime_task(&task_id)
        .map_err(|e| e.to_string())?
        .ok_or("guidance task is missing")?;
    let owner = task
        .owner_session_id
        .clone()
        .ok_or("guidance task has no parent")?;
    // Parent display state is not execution authority, and its Store may have
    // been ephemeral. Use the immutable admission and current Task/Run instead
    // of requiring a second copy of the parent's complete checkpoint here.
    let authority = store
        .runtime_agent_task_authority(&task_id)
        .map_err(|e| e.to_string())?
        .ok_or("guidance task has no admission authority")?;
    authority.validate()?;
    let admission = authority.admission.ok_or("guidance task is not admitted")?;
    if let Some(existing) = receipt(store, &task_id, &message_id, &owner, &admission.parent_run_id)? {
        if existing.parent_turn_id != parent_turn_id || existing.content != content || existing.submitted_at_ms != at {
            return Err("guidance identity was already submitted with different immutable input".into());
        }
        return Ok(existing);
    }
    let binding = store.runtime_task_run_binding(&task_id).map_err(|e| e.to_string())?
        .ok_or("guidance task has not started a bound Run")?;
    if task.kind != "agent"
        || admission.task_id != task_id
        || admission.parent_turn_id != parent_turn_id
        || admission.parent_conversation_id != owner
        || task.child_session_id.as_deref() != Some(admission.child_conversation_id.as_str())
        || task.parent_tool_call_id.as_deref() != Some(admission.parent_tool_call_id.as_str())
        || binding.owner_session_id != owner
        || binding.child_session_id != admission.child_conversation_id
        || task.input_watermark < admission.input_watermark
        || binding.input_watermark > task.input_watermark
    {
        return Err("guidance does not belong to the immutable parent Task input".into());
    }
    let record = RuntimeTaskGuidance {
        task_id,
        message_id,
        owner_session_id: owner,
        parent_core_session_id: admission.parent_core_session_id,
        parent_turn_id,
        child_session_id: binding.child_session_id,
        run_id: binding.run_id,
        input_watermark: task.input_watermark,
        content,
        submitted_at_ms: at,
        status: "submitted".into(),
        accepted_at_ms: None,
        resolved_at_ms: None,
        reason: None,
        delivery_transition_id: None,
    };
    let mut record = store
        .insert_runtime_task_guidance(&record)
        .map_err(|e| e.to_string())?;
    if record.status == "submitted" {
        let mut reason = unavailable(store, &record)?;
        if reason.is_none()
            && store
                .outstanding_runtime_task_guidance_count(&record.task_id)
                .map_err(|e| e.to_string())?
                > nola_agent_engine::agent::task::MAX_TASK_GUIDANCE_QUEUE as u64
        {
            reason = Some("task_guidance_queue_full");
        }
        record = store
            .resolve_runtime_task_guidance(
                &record.task_id,
                &record.message_id,
                if reason.is_some() {
                    "rejected"
                } else {
                    "accepted"
                },
                reason,
                None,
                at,
            )
            .map_err(|e| e.to_string())?;
        if record.status == "accepted" {
            enqueue(
                store,
                &record.child_session_id,
                nola_agent_engine::agent::host::AgentBoundaryInput::Submission { submission: AgentSubmission::DeliverTaskGuidance {
                    operation_id: format!(
                        "task-guidance-deliver:{}:{}",
                        record.task_id, record.message_id
                    ),
                    task_id: record.task_id.clone(),
                    message_id: record.message_id.clone(),
                    expected_run_id: record.run_id.clone(),
                } },
                record.accepted_at_ms.unwrap_or(at),
            )?;
        }
    }
    enqueue(
        store,
        &record.owner_session_id,
        parent_command(&record, false),
        record.submitted_at_ms,
    )?;
    Ok(record)
}

pub(crate) fn prepare_command<R: GuidanceRepositories + ?Sized>(
    store: &mut R,
    command: &mut ApplicationCommand,
) -> Result<(), String> {
    if matches!(command,ApplicationCommand::ReduceAndCommitIdleSubmission {input}
        if matches!(input.submission,AgentSubmission::ApplyTaskGuidanceInput {..}|AgentSubmission::ObserveTaskGuidanceDelivery {..}))
    {
        return Err("guidance delivery cannot start or use an idle Run".into());
    }
    let (owner, submission) = match command {
        ApplicationCommand::ReduceAndCommitActiveSubmission { input } => {
            (input.conversation_id.as_str(), &mut input.submission)
        }
        ApplicationCommand::ReduceAndCommitIdleSubmission { input } => {
            (input.conversation_id.as_str(), &mut input.submission)
        }
        _ => return Ok(()),
    };
    validate_submission(store, owner, submission)
}

pub(crate) fn validate_submission<R: RuntimeTaskRepository + ?Sized>(
    store: &R,
    owner: &str,
    submission: &AgentSubmission,
) -> Result<(), String> {
    match submission {
        AgentSubmission::DeliverTaskGuidance { .. } => {
            return Err("child guidance FIFO marker requires ApplyTaskGuidance".into())
        }
        AgentSubmission::ObserveTaskGuidanceDelivery {
            task_id,
            message_id,
            expected_run_id,
            ..
        } => {
            let record = load(store, task_id, message_id)?;
            if record.status != "delivered"
                || record.child_session_id != owner
                || record.run_id != *expected_run_id
            {
                return Err(
                    "guidance observation requires a delivered record for the exact Run".into(),
                );
            }
        }
        AgentSubmission::ApplyTaskGuidanceInput {
            operation_id,
            task_id,
            message_id,
            expected_run_id,
            content,
        } => {
            let record = load(store, task_id, message_id)?;
            if record.status != "delivered"
                || record.child_session_id != owner
                || record.run_id != *expected_run_id
                || record.content != *content
                || *operation_id != format!("task-guidance-input:{}:{}", task_id, message_id)
            {
                return Err("guidance input lacks its exact committed delivery intent".into());
            }
        }
        AgentSubmission::ReconcileTaskGuidance { .. } => {
            return Err("guidance facts require trusted source-validated import".into())
        }
        _ => {}
    }
    Ok(())
}

pub(crate) struct PreparedApply {
    pub guidance: RuntimeTaskGuidance,
    pub submission: Option<ReduceAndCommitActiveSubmission>,
    pub recovered: Option<Box<AcceptedIdleSubmission>>,
}

pub(crate) fn prepare_apply<R: GuidanceRepositories + ?Sized>(
    store: &mut R,
    task: &str,
    message: &str,
    lease: Option<&RuntimeTaskLeaseWitness>,
    run: &str,
    at: i64,
) -> Result<PreparedApply, String> {
    let mut record = load(store, task, message)?;
    if record.run_id != run {
        return Err("guidance delivery references a different bound Run".into());
    }
    let unavailable = unavailable(store, &record)?;
    if !matches!(record.status.as_str(), "delivered" | "rejected") {
        if let Some(reason) = unavailable {
            record = store
                .resolve_runtime_task_guidance(task, message, "rejected", Some(reason), None, at)
                .map_err(|e| e.to_string())?;
            final_notification(store, &record)?;
        }
    }
    let active = store
        .active_run(&record.child_session_id)
        .map_err(|e| e.to_string())?;
    if record.status == "rejected"
        || (record.status == "delivered"
            && active.as_ref().map(|run| run.run_id.as_str()) != Some(run))
    {
        let recovered = if active.is_none()
            && store
                .read_canonical_run_terminal(&record.child_session_id, run)
                .map_err(|e| e.to_string())?
                .is_some()
        {
            Some(recover_snapshot(store, &record)?)
        } else {
            None
        };
        return Ok(PreparedApply {
            guidance: record,
            submission: None,
            recovered,
        });
    }
    let replay = record.status == "delivered";
    if !replay {
        if record.status != "accepted" {
            return Err("guidance has not been accepted by its canonical parent".into());
        }
        let lease = lease.ok_or("live guidance delivery requires an exact worker lease")?;
        let task = store
            .runtime_task(task)
            .map_err(|e| e.to_string())?
            .ok_or("guidance task missing")?;
        let attempt = store
            .runtime_task_attempt(&lease.attempt_id)
            .map_err(|e| e.to_string())?
            .ok_or("guidance attempt missing")?;
        if task.status != "running"
            || task.cancel_requested
            || task.id != lease.task_id
            || attempt.task_id != task.id
            || attempt.state != "leased"
            || RuntimeTaskLeaseWitness::from(&attempt) != *lease
            || task.worker_id.as_deref() != Some(attempt.worker_id.as_str())
            || task.ownership_token.as_deref() != Some(lease.lease_token.as_str())
            || task.lease_until_ms != Some(lease.current_expiry_ms)
            || lease.current_expiry_ms <= at
            || attempt.claimed_at_ms > at
            || task.updated_at_ms > at
        {
            return Err("guidance delivery worker lease is stale or foreign".into());
        }
        record = store
            .resolve_runtime_task_guidance(
                &record.task_id,
                message,
                "delivered",
                None,
                Some(&format!(
                    "task-guidance-delivery:{}:{}",
                    record.task_id, message
                )),
                at,
            )
            .map_err(|e| e.to_string())?;
    }
    let active = active
        .filter(|active| active.run_id == run)
        .ok_or("guidance has no exact active Run")?;
    let transition = if replay {
        format!(
            "task-guidance-replay:{}:{}:{}",
            record.task_id,
            message,
            uuid::Uuid::new_v4()
        )
    } else {
        record
            .delivery_transition_id
            .clone()
            .ok_or("guidance lacks delivery transition")?
    };
    let submission = if replay {
        AgentSubmission::ObserveTaskGuidanceDelivery {
            operation_id: transition.clone(),
            task_id: record.task_id.clone(),
            message_id: message.into(),
            expected_run_id: run.into(),
        }
    } else {
        AgentSubmission::ApplyTaskGuidanceInput {
            operation_id: format!("task-guidance-input:{}:{}", record.task_id, message),
            task_id: record.task_id.clone(),
            message_id: message.into(),
            expected_run_id: run.into(),
            content: record.content.clone(),
        }
    };
    let input = ReduceAndCommitActiveSubmission {
        conversation_id: record.child_session_id.clone(),
        run_id: run.into(),
        transition_id: transition,
        invocation_config_digest: active
            .invocation_config_digest
            .ok_or("guidance Run invocation digest missing")?,
        submission,
        committed_at_ms: at,
    };
    Ok(PreparedApply {
        guidance: record,
        submission: Some(input),
        recovered: None,
    })
}

fn recover_snapshot<R: GuidanceRepositories + ?Sized>(
    store: &R,
    record: &RuntimeTaskGuidance,
) -> Result<Box<AcceptedIdleSubmission>, String> {
    let stored = crate::load_current_checkpoint(store, &record.child_session_id)?
        .ok_or("guidance terminal checkpoint missing")?;
    let checkpoint: CoreCheckpoint<AgentThreadState> =
        serde_json::from_str(&stored.checkpoint_json).map_err(|e| e.to_string())?;
    let terminal = store
        .read_canonical_run_terminal(&record.child_session_id, &record.run_id)
        .map_err(|e| e.to_string())?
        .ok_or("guidance Run is not terminal")?;
    // This mapping checks lifecycle identity only; business acceptance remains
    // the exact RunResult and is never inferred from Turn::Completed.
    let expected_status = match &terminal.result.value {
        nola_domain::run_result::RunResultValue::Answered { .. }
        | nola_domain::run_result::RunResultValue::ActionCompleted { .. }
        | nola_domain::run_result::RunResultValue::DelegationAccepted { .. }
        | nola_domain::run_result::RunResultValue::Incomplete { .. }
        | nola_domain::run_result::RunResultValue::Blocked { .. } => {
            nola_agent_engine::agent::AgentTurnStatus::Completed
        }
        nola_domain::run_result::RunResultValue::Failed { .. } => {
            nola_agent_engine::agent::AgentTurnStatus::Failed
        }
        nola_domain::run_result::RunResultValue::Cancelled { .. } => {
            nola_agent_engine::agent::AgentTurnStatus::Cancelled
        }
    };
    if stored.conversation_id != record.child_session_id
        || checkpoint.session_id.is_empty()
        || checkpoint.state.session_id.as_str() != checkpoint.session_id
        || checkpoint
            .state
            .execution
            .active_turn
            .as_ref()
            .map(|turn| (turn.turn_id.as_str(), turn.status))
            != Some((record.run_id.as_str(), expected_status))
    {
        return Err("guidance terminal checkpoint belongs to another Run".into());
    }
    let invocation = store
        .load_run_invocation_config(&record.run_id)
        .map_err(|e| e.to_string())?
        .ok_or("guidance Run configuration missing")?;
    use sha2::{Digest, Sha256};
    if format!(
        "{:x}",
        Sha256::digest(invocation.invocation_config_json.as_bytes())
    ) != invocation.invocation_config_digest
    {
        return Err("guidance terminal Run configuration digest mismatch".into());
    }
    let mut config: AgentCoreConfig =
        serde_json::from_str(&invocation.invocation_config_json).map_err(|e| e.to_string())?;
    if config
        .session_id
        .as_ref()
        .is_some_and(|id| id.as_str() != checkpoint.session_id)
    {
        return Err("guidance terminal Core session identity mismatch".into());
    }
    crate::delegated_invocation::bind_stored_restore_configuration(
        store,
        &mut config,
        &checkpoint,
        &record.run_id,
    )?;
    let core = AgentCore::restore(config, checkpoint).map_err(|e| e.to_string())?;
    Ok(Box::new(AcceptedIdleSubmission {
        assistant_publications: vec![],
        execution: core.host_execution_state(),
        events: vec![],
        traces: vec![],
    }))
}

pub(crate) fn finish_apply<R: AgentQueueRepository + ?Sized>(
    store: &mut R,
    record: &RuntimeTaskGuidance,
) -> Result<(), String> {
    final_notification(store, record)
}

pub(crate) fn settle_ended_run<R: GuidanceRepositories + ?Sized>(
    store: &mut R,
    task: &str,
    at: i64,
) -> Result<(), String> {
    for record in store
        .runtime_task_guidance_records(task)
        .map_err(|e| e.to_string())?
    {
        if matches!(record.status.as_str(), "submitted" | "accepted") {
            if let Some(reason) = unavailable(store, &record)? {
                let result = store
                    .resolve_runtime_task_guidance(
                        task,
                        &record.message_id,
                        "rejected",
                        Some(reason),
                        None,
                        at,
                    )
                    .map_err(|e| e.to_string())?;
                final_notification(store, &result)?;
            }
        }
    }
    Ok(())
}
