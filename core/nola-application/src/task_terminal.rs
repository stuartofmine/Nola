// SPDX-License-Identifier: Apache-2.0
//! Receipt-derived terminal delivery. Queued payloads are revalidated before
//! reduction, including commands submitted through the generic inbox API.
use crate::{ApplicationCommand, RuntimeTaskTerminalReceipt};
use nola_domain::agent_task_admission::{AgentTaskTerminalProjection, AgentTaskTerminalGuidance};
pub use nola_domain::agent_task_admission::TaskTerminalReconciliationApplied;
use nola_agent_engine::agent::AgentSubmission;
use nola_store_port::remote_session::RemoteSessionRepository;
use nola_store_port::run_effect::RunEffectRepository;
use nola_store_port::runtime_task::RuntimeTaskRepository;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskTerminalReconciliationEnqueued {
    pub conversation_id: String,
    pub command_id: String,
    pub receipt_id: String,
}

fn load<R: RuntimeTaskRepository + ?Sized>(
    store: &R,
    task_id: &str,
    receipt_id: &str,
) -> Result<RuntimeTaskTerminalReceipt, String> {
    let receipt=store.runtime_task_terminal_receipt(task_id).map_err(|e|e.to_string())?
        .ok_or("terminal task has no durable receipt; historical execution authority cannot be inferred")?;
    let task = store
        .runtime_task(task_id)
        .map_err(|e| e.to_string())?
        .ok_or("terminal task is missing")?;
    if receipt.receipt_id != receipt_id
        || receipt.task_id != task_id
        || task.kind != "agent"
        || receipt.owner_session_id.trim().is_empty()
        || task.owner_session_id.as_deref() != Some(receipt.owner_session_id.as_str())
        || task.child_session_id != receipt.child_session_id
        || task.input_watermark != receipt.input_watermark
        || task.status != receipt.status
        || task.completion_watermark != receipt.completion_watermark
        || task.ended_at_ms != Some(receipt.ended_at_ms)
    {
        return Err("terminal receipt does not match immutable task identity and result".into());
    }
    Ok(receipt)
}


pub(crate) fn export_projection<R: RuntimeTaskRepository + ?Sized>(
    store: &R, task_id: &str, receipt_id: &str, guidance_message_ids: &[String],
) -> Result<AgentTaskTerminalProjection, String> {
    use nola_agent_engine::agent::task::{MAX_TASK_GUIDANCE_QUEUE,MAX_TASK_GUIDANCE_CHARS};
    let requested: std::collections::BTreeSet<_> = guidance_message_ids.iter().collect();
    if requested.len() != guidance_message_ids.len() || requested.len() > MAX_TASK_GUIDANCE_QUEUE {
        return Err("terminal export requires a bounded unique pending guidance selection".into());
    }
    let receipt = load(store, task_id, receipt_id)?;
    let authority = store.runtime_agent_task_authority(task_id).map_err(|e|e.to_string())?
        .ok_or("terminal Task has no Core admission authority")?;
    authority.validate()?;
    let admission = authority.admission.ok_or("prepared Task has no terminal parent projection")?;
    if admission.parent_conversation_id != receipt.owner_session_id
        || Some(admission.child_conversation_id.as_str()) != receipt.child_session_id.as_deref()
        || admission.input_watermark > receipt.input_watermark {
        return Err("terminal receipt does not own the admitted Task".into());
    }
    let mut guidance_results = Vec::with_capacity(requested.len());
    for message_id in requested {
        let record = receipt.guidance.iter().find(|record| &record.message_id == message_id)
            .ok_or("requested pending guidance has no settled terminal receipt")?;
        if record.content.chars().count() > MAX_TASK_GUIDANCE_CHARS {
            return Err("terminal guidance content exceeds admission limit".into());
        }
        guidance_results.push(AgentTaskTerminalGuidance {
            task_id: record.task_id.clone(), message_id: record.message_id.clone(),
            parent_session_id: record.parent_core_session_id.clone(), parent_turn_id: record.parent_turn_id.clone(),
            input_watermark: record.input_watermark, content: record.content.clone(), submitted_at_ms: record.submitted_at_ms,
            status: record.status.clone(), reason: record.reason.clone(),
        });
    }
    let projection = AgentTaskTerminalProjection {
        protocol_version: 2, admission, authority_revision: authority.authority_revision,
        receipt_id: receipt.receipt_id, status: receipt.status, input_watermark: receipt.input_watermark,
        completion_watermark: receipt.completion_watermark, result_ref: receipt.result_ref,
        run_result: receipt.journal_result.as_ref().map(|result| result.seal.result.clone()),
        result_digest: receipt.journal_result.as_ref().map(|result| result.seal.result_digest),
        ended_at_ms: receipt.ended_at_ms, guidance_results,
    };
    projection.validate()?;
    Ok(projection)
}

pub(crate) enum PreparedTaskTerminalImport {
    Active(crate::ReduceAndCommitActiveSubmission),
    Idle(crate::ReduceAndCommitIdleSubmission),
    AlreadyApplied(TaskTerminalReconciliationApplied),
}

/// Local Hosts consume the same receipt-derived import as remote Hosts. The
/// caller supplies no result, Task state or projection and cannot skip a
/// different command at the head of the parent's durable inbox.
pub(crate) fn claim_local_notice<R: RuntimeTaskRepository + RunEffectRepository + nola_store_port::agent_queue::AgentQueueRepository + nola_store_port::remote_session::CanonicalRunTerminalRepository>(
    store: &mut R, conversation_id: &str, consumer_id: &str, now_ms: i64,
) -> Result<Option<(String, PreparedTaskTerminalImport)>, String> {
    use nola_agent_engine::agent::{host::AgentBoundaryInput, AgentThreadState, recovery::CoreCheckpoint};
    if consumer_id.trim().is_empty() { return Err("Task terminal consumer is missing".into()); }
    let Some(head) = store.peek_agent_command(conversation_id).map_err(|e| e.to_string())? else { return Ok(None); };
    if !matches!(serde_json::from_str::<AgentBoundaryInput>(&head.payload_json).map_err(|e| e.to_string())?,
        AgentBoundaryInput::TaskTerminalAvailable { .. } | AgentBoundaryInput::TaskGuidanceAvailable { .. }) { return Ok(None); }
    let Some(claimed) = store.claim_next_agent_command(conversation_id, consumer_id, now_ms,
        now_ms.checked_add(30_000).ok_or("Task terminal lease clock overflow")?).map_err(|e| e.to_string())?
        else { return Ok(None); };
    let notice: AgentBoundaryInput = serde_json::from_str(&claimed.payload_json).map_err(|e| e.to_string())?;
    if let AgentBoundaryInput::TaskGuidanceAvailable { task_id, message_id, settlement } = &notice {
        let projection = crate::task_guidance::export_guidance(store, task_id, message_id, *settlement)?;
        if projection.admission.parent_conversation_id != conversation_id { return Err("Task guidance belongs to another parent".into()); }
        let prepared = match crate::task_guidance::prepare_guidance_import(store, &projection)? {
            crate::task_guidance::PreparedTaskGuidanceImport::Active(input) => PreparedTaskTerminalImport::Active(input),
            crate::task_guidance::PreparedTaskGuidanceImport::Idle(input) => PreparedTaskTerminalImport::Idle(input),
        };
        return Ok(Some((claimed.command_id, prepared)));
    }
    let AgentBoundaryInput::TaskTerminalAvailable { task_id, receipt_id } = notice else {
        return Err("Task terminal queue head changed".into());
    };
    let receipt = load(store, &task_id, &receipt_id)?;
    if receipt.owner_session_id != conversation_id { return Err("Task terminal belongs to another parent".into()); }
    let stored = crate::load_current_checkpoint(store, conversation_id)?.ok_or("Task terminal parent checkpoint is missing")?;
    let checkpoint: CoreCheckpoint<AgentThreadState> = serde_json::from_str(&stored.checkpoint_json).map_err(|e| e.to_string())?;
    let pending = checkpoint.state.task_projection.guidance.get(&task_id)
        .map(|queue| queue.pending.iter().map(|message| message.message_id.clone()).collect::<Vec<_>>()).unwrap_or_default();
    let projection = export_projection(store, &task_id, &receipt_id, &pending)?;
    Ok(Some((claimed.command_id, prepare_import(store, &projection)?)))
}

pub(crate) fn prepare_import<R: RuntimeTaskRepository + RunEffectRepository + ?Sized>(
    store: &R, projection: &AgentTaskTerminalProjection,
) -> Result<PreparedTaskTerminalImport, String> {
    use nola_agent_engine::agent::{AgentCoreConfig,AgentThreadState,recovery::CoreCheckpoint};
    use nola_agent_engine::agent::task::{MAX_TASK_GUIDANCE_QUEUE,MAX_TASK_GUIDANCE_CHARS};
    projection.validate()?;
    let admission = &projection.admission;
    let original = crate::agent_task_admission::export(store, &admission.parent_conversation_id,
        &admission.parent_run_id, &admission.task_id, admission.core_event_cursor)?;
    if original != *admission { return Err("terminal projection does not match the original source Task admission".into()); }
    let stored = crate::load_current_checkpoint(store, &admission.parent_conversation_id)?
        .ok_or("Task terminal projection source checkpoint is unavailable")?;
    let checkpoint: CoreCheckpoint<AgentThreadState> = serde_json::from_str(&stored.checkpoint_json).map_err(|e|e.to_string())?;
    let state = &checkpoint.state;
    let task = state.task_projection.tasks.get(&admission.task_id).ok_or("terminal source Task no longer exists")?;
    if state.session_id.as_str() != admission.parent_core_session_id
        || task.owner_session_id.as_str() != admission.parent_core_session_id
        || task.owner_turn_id.as_str() != admission.parent_turn_id
        || task.lineage.child_session_id.as_ref().map(|id|id.as_str()) != Some(admission.child_conversation_id.as_str())
        || task.lineage.parent_call_id.as_ref().map(|id|id.as_str()) != Some(admission.parent_tool_call_id.as_str())
        || task.durability.input_watermark > projection.input_watermark {
        return Err("terminal projection targets a foreign source Task incarnation".into());
    }
    if projection.guidance_results.len() > MAX_TASK_GUIDANCE_QUEUE
        || projection.guidance_results.iter().any(|item|item.content.chars().count() > MAX_TASK_GUIDANCE_CHARS) {
        return Err("terminal guidance exceeds the source pending queue limit".into());
    }
    if let Some(queue) = state.task_projection.guidance.get(&admission.task_id) {
        for pending in &queue.pending {
            if !projection.guidance_results.iter().any(|result| result.message_id == pending.message_id
                && result.content == pending.content && result.parent_session_id == pending.parent_session_id.as_str()
                && result.input_watermark == pending.input_watermark
                && result.parent_turn_id == pending.parent_turn_id.as_str() && result.submitted_at_ms == pending.enqueued_at_epoch_ms) {
                return Err("terminal projection omitted or changed pending source guidance".into());
            }
        }
        for result in &projection.guidance_results {
            if !queue.pending.iter().any(|pending| pending.message_id == result.message_id)
                && !queue.results.contains_key(&result.message_id) {
                return Err("terminal projection includes unknown source guidance".into());
            }
        }
    } else if !projection.guidance_results.is_empty() {
        return Err("terminal projection invents source guidance".into());
    }
    // Run the pure narrow reducer before selecting a publication route. This
    // also proves replay equality and exact guidance content/status semantics.
    let mut task_projection = state.task_projection.clone();
    task_projection.reconcile_terminal(projection, &state.session_id)?;
    if task.state.is_terminal() {
        return Ok(PreparedTaskTerminalImport::AlreadyApplied(TaskTerminalReconciliationApplied {
            conversation_id: admission.parent_conversation_id.clone(), task_id: admission.task_id.clone(),
            receipt_id: projection.receipt_id.clone(), core_revision: checkpoint.revision,
        }));
    }
    let operation_id = format!("terminal:{}", projection.receipt_id);
    let submission = AgentSubmission::ReconcileTaskTerminal { operation_id: operation_id.clone(), fact: projection.clone() };
    if let Some(active) = store.active_run(&admission.parent_conversation_id).map_err(|e|e.to_string())? {
        Ok(PreparedTaskTerminalImport::Active(crate::ReduceAndCommitActiveSubmission {
            conversation_id: admission.parent_conversation_id.clone(), run_id: active.run_id,
            transition_id: operation_id, invocation_config_digest: active.invocation_config_digest.ok_or("active parent Run has no invocation digest")?,
            submission, committed_at_ms: projection.ended_at_ms.max(stored.updated_at_ms),
        }))
    } else {
        if !state.permits_new_turn_execution_configuration() { return Err("source parent has unsettled execution without an active Run".into()); }
        Ok(PreparedTaskTerminalImport::Idle(crate::ReduceAndCommitIdleSubmission {
            conversation_id: admission.parent_conversation_id.clone(), core_config: AgentCoreConfig {
                automation_execution_spec: None,
                session_id: Some(state.session_id.clone()), model: state.configuration.model.clone(),
                max_model_iterations: state.configuration.max_model_iterations, initial_items: Vec::new(),
                max_strategy_recovery_attempts: state.configuration.max_strategy_recovery_attempts,
                tools: state.configuration.tools.clone(), context_window: state.configuration.context_window.clone(),
                host_catalog: Some(state.host.catalog.clone()), permission_rules: state.execution.permission_rules.clone(),
                memory_policy: state.configuration.memory_policy.clone(), turn_identity: state.configuration.turn_identity.clone(),
            }, submission, committed_at_ms: projection.ended_at_ms.max(stored.updated_at_ms),
        }))
    }
}

pub(crate) fn enqueue<R: RuntimeTaskRepository + nola_store_port::agent_queue::AgentQueueRepository + ?Sized>(
    store: &mut R,
    task_id: &str,
    receipt_id: &str,
) -> Result<TaskTerminalReconciliationEnqueued, String> {
    let receipt = load(store, task_id, receipt_id)?;
    let authority = store.runtime_agent_task_authority(task_id).map_err(|error| error.to_string())?
        .ok_or("terminal notification has no admitted Task authority")?;
    authority.validate()?;
    let admission = authority.admission.ok_or("prepared Task has no source terminal projection")?;
    if admission.parent_conversation_id != receipt.owner_session_id
        || Some(admission.child_conversation_id.as_str()) != receipt.child_session_id.as_deref()
        || admission.input_watermark > receipt.input_watermark {
        return Err("terminal notification differs from its admitted Task owner".into());
    }
    let command_id = format!("terminal:{}", receipt.receipt_id);
    let input = nola_store_port::agent_queue::EnqueueAgentCommandInput {
        session_id: receipt.owner_session_id.clone(),
        command_id: command_id.clone(),
        payload_json: serde_json::to_string(&nola_agent_engine::agent::host::AgentBoundaryInput::TaskTerminalAvailable {
            task_id: task_id.into(), receipt_id: receipt.receipt_id.clone(),
        }).map_err(|e| e.to_string())?,
        created_at_ms: receipt.recorded_at_ms,
    };
    store
        .enqueue_agent_command(&input)
        .map_err(|e| e.to_string())?;
    Ok(TaskTerminalReconciliationEnqueued {
        conversation_id: receipt.owner_session_id,
        command_id,
        receipt_id: receipt.receipt_id,
    })
}

fn validate_submission<
    R: RuntimeTaskRepository + RemoteSessionRepository + RunEffectRepository + ?Sized,
>(
    store: &R,
    owner: &str,
    submitted: &AgentSubmission,
) -> Result<(), String> {
    crate::task_guidance::validate_submission(store, owner, submitted)?;
    match submitted {
        AgentSubmission::HostResponse { response } => {
            validate_host_response(store, owner, &response.payload)?
        }
        AgentSubmission::ReconcileTaskTerminal { .. } => return Err("terminal Task facts require trusted source-validated import".into()),
        AgentSubmission::UpdateTask { task_id, state, .. } if state.is_terminal() => {
            if is_agent_task(store, owner, task_id)?
            {
                return Err("durable agent terminal update requires a terminal receipt".into());
            }
        }
        _ => {}
    }
    Ok(())
}

pub(crate) fn validate_terminal_mutation<
    R: crate::repositories::AgentRuntimeCommandRepositories,
>(
    store: &mut R,
    command: &crate::RuntimeTaskCommand,
) -> Result<(), String> {
    if let crate::RuntimeTaskCommand::FinishDetailedWithEvent { lease, .. } = command {
        if store
            .runtime_task_run_binding(&lease.task_id)
            .map_err(|error| error.to_string())?
            .is_some()
        {
            return Err(
                "bound agent task must settle from its canonical Run seal, not a worker summary"
                    .into(),
            );
        }
    }
    match command {
        crate::RuntimeTaskCommand::FinishDetailedWithEvent { lease, now_ms, .. } => {
            crate::task_guidance::settle_ended_run(store, &lease.task_id, *now_ms)?
        }
        crate::RuntimeTaskCommand::ReconcileBoundRunTerminal {
            task_id, now_ms, ..
        } => crate::task_guidance::settle_ended_run(store, task_id, *now_ms)?,
        _ => {}
    }
    let task_id = match command {
        crate::RuntimeTaskCommand::FinishDetailedWithEvent { lease, status, .. }
            if status == "completed" =>
        {
            lease.task_id.as_str()
        }
        crate::RuntimeTaskCommand::ReconcileBoundRunTerminal {
            task_id,
            expected_run_id,
            ..
        } => {
            let Some(binding) = store
                .runtime_task_run_binding(task_id)
                .map_err(|e| e.to_string())?
            else {
                return Ok(());
            };
            if binding.run_id != *expected_run_id {
                return Ok(());
            }
            let terminal = store
                .canonical_run_terminal(&binding.child_session_id, expected_run_id)
                .map_err(|e| e.to_string())?;
            if terminal.is_none() {
                return Ok(());
            }
            task_id.as_str()
        }
        _ => return Ok(()),
    };
    let Some(task) = store.runtime_task(task_id).map_err(|e| e.to_string())? else {
        return Ok(());
    };
    if task.kind != "agent" {
        return Ok(());
    }
    let authority = store.runtime_agent_task_authority(task_id).map_err(|e|e.to_string())?
        .ok_or("completed Task has no admitted authority")?;
    authority.validate()?;
    let admission = authority.admission.ok_or("prepared Task cannot complete execution")?;
    if admission.task_id != task.id
        || task.owner_session_id.as_deref() != Some(admission.parent_conversation_id.as_str())
        || task.child_session_id.as_deref() != Some(admission.child_conversation_id.as_str())
        || task.parent_tool_call_id.as_deref() != Some(admission.parent_tool_call_id.as_str())
        || task.input_watermark < admission.input_watermark {
        return Err("completed Task no longer matches its immutable admission".into());
    }
    if store.outstanding_runtime_task_guidance_count(task_id).map_err(|e|e.to_string())? != 0 {
        return Err("completed Task must settle all admitted guidance before terminal commit".into());
    }
    Ok(())
}

fn is_agent_task<R: RuntimeTaskRepository + RunEffectRepository + ?Sized>(
    store: &R, owner: &str, task_id: &str,
) -> Result<bool, String> {
    if store.runtime_task(task_id).map_err(|error| error.to_string())?
        .is_some_and(|task| task.kind == "agent") { return Ok(true); }
    // Source identity is sufficient to reject an untrusted mutation. It does
    // not confer worker authority or require a global Task row in this Store.
    let Some(stored) = crate::load_current_checkpoint(store, owner)? else { return Ok(false); };
    let checkpoint: nola_agent_engine::agent::recovery::CoreCheckpoint<
        nola_agent_engine::agent::AgentThreadState,
    > = serde_json::from_str(&stored.checkpoint_json).map_err(|error| error.to_string())?;
    Ok(checkpoint.state.task_projection.tasks.get(task_id)
        .is_some_and(|task| task.kind == nola_agent_engine::agent::TaskKind::Agent))
}

fn validate_host_response<R: RuntimeTaskRepository + RunEffectRepository + ?Sized>(
    store: &R,
    owner: &str,
    response: &nola_agent_engine::agent::host::HostResponse,
) -> Result<(), String> {
    if let nola_agent_engine::agent::host::HostResponse::TaskUpdated {
        task_id,
        state,
        ..
    } = response
    {
        if state.is_terminal()
            && is_agent_task(store, owner, task_id)?
        {
            return Err("durable agent terminal Host response requires a terminal receipt".into());
        }
    }
    Ok(())
}

pub(crate) fn validate_generic_enqueue<
    R: RuntimeTaskRepository + RemoteSessionRepository + RunEffectRepository + ?Sized,
>(
    store: &R,
    input: &crate::AgentCommand,
) -> Result<(), String> {
    if input.command_id.starts_with("terminal:") {
        return Err("terminal command identities are reserved for receipt delivery".into());
    }
    if input.command_id.starts_with("permission-policy:") {
        return Err("permission policy identities require committed configuration admission".into());
    }
    if input.command_id.starts_with("task-guidance-") {
        return Err("guidance FIFO identities require typed admission".into());
    }
    {
        let submitted = serde_json::from_str::<AgentSubmission>(&input.payload_json)
            .map_err(|error| format!("Agent command requires a valid typed submission: {error}"))?;
        if matches!(submitted, AgentSubmission::ApplyPermissionPolicy { .. }) {
            return Err("permission policy requires committed configuration notification".into());
        }
        if matches!(
            submitted,
            AgentSubmission::DeliverTaskGuidance { .. }
                | AgentSubmission::ReconcileTaskGuidance { .. }
                | AgentSubmission::ApplyTaskGuidanceInput { .. }
                | AgentSubmission::ObserveTaskGuidanceDelivery { .. }
        ) {
            return Err("guidance FIFO requires typed admission".into());
        }
        if matches!(submitted, AgentSubmission::ReconcileTaskTerminal { .. }) {
            return Err("terminal reconciliation must use the receipt-only enqueue command".into());
        }
        validate_submission(store, &input.session_id, &submitted)?;
    }
    Ok(())
}

pub(crate) fn validate_command<
    R: RuntimeTaskRepository + RemoteSessionRepository + RunEffectRepository + ?Sized,
>(
    store: &R,
    command: &ApplicationCommand,
) -> Result<(), String> {
    match command {
        ApplicationCommand::ReduceAndAcceptHostResponse { input } => {
            validate_host_response(store, &input.conversation_id, &input.response.payload)
        }
        ApplicationCommand::ReduceAndCommitActiveSubmission { input } => {
            validate_submission(store, &input.conversation_id, &input.submission)
        }
        ApplicationCommand::ReduceAndCommitIdleSubmission { input } => {
            validate_submission(store, &input.conversation_id, &input.submission)
        }
        ApplicationCommand::EnqueueAgentCommand { input } => validate_generic_enqueue(store, input),
        _ => Ok(()),
    }
}
