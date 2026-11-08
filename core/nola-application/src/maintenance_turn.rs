// SPDX-License-Identifier: Apache-2.0
//! Memory maintenance queries and canonical maintenance-run admission.

use nola_agent_engine::agent::protocol::{AgentCoreConfig, AgentEvent, AgentSubmission};
use nola_agent_engine::agent::recovery::{CoreCheckpoint};
use nola_domain::core_event::{CoreEventEnvelope};
use nola_agent_engine::agent::{AgentCore, AgentThreadState};
use nola_agent_engine::agent_trace::AgentTraceEvent;
use nola_store_port::run_creation::{BeginMaintenanceRunInput, RunCreationRepository};
use nola_store_port::run_effect::RunEffectRepository;
use serde::{Deserialize, Serialize};

use crate::{
    decision_time_ms, delegated_invocation, delegated_permission, external_outbox_requests,
    invocation_config_snapshot, model_attempt, unrecorded_checkpoint_usage, HostExecutionState,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BeginMaintenanceTurn {
    pub conversation_id: String,
    pub context_checkpoint_id: Option<String>,
    pub core_config: AgentCoreConfig,
    pub submission: AgentSubmission,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcceptedMaintenanceTurn {
    pub run_id: String,
    pub invocation_config_digest: String,
    pub execution: HostExecutionState,
    pub events: Vec<CoreEventEnvelope<AgentEvent>>,
    pub traces: Vec<AgentTraceEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryClaimableMemoryJobs {
    pub conversation_id: String,
    pub now_epoch_ms: i64,
}

/// Reads Core-owned worker readiness from the canonical Application
/// checkpoint. Platform Hosts must not decode checkpoints or reproduce retry
/// policy in file sidecars.
pub fn query_claimable_memory_jobs<R: RunEffectRepository + ?Sized>(
    store: &R,
    command: QueryClaimableMemoryJobs,
) -> Result<Vec<nola_agent_engine::agent::MemoryJobHandle>, String> {
    let Some(stored) = crate::load_current_checkpoint(store, &command.conversation_id)? else {
        return Ok(Vec::new());
    };
    let checkpoint =
        serde_json::from_str::<CoreCheckpoint<AgentThreadState>>(&stored.checkpoint_json)
            .map_err(|error| format!("decode Core checkpoint failed: {error}"))?;
    Ok(checkpoint
        .state
        .memory_runtime
        .claimable_jobs(command.now_epoch_ms))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildMemoryWorkerPlan {
    pub conversation_id: String,
    pub job_id: String,
}

/// Builds model input from the same canonical checkpoint used for worker
/// claims. The returned plan contains frozen evidence selected by Core.
pub fn build_memory_worker_plan<R: RunEffectRepository + ?Sized>(
    store: &R,
    command: BuildMemoryWorkerPlan,
) -> Result<nola_agent_engine::agent::MemoryWorkerPlan, String> {
    let stored = crate::load_current_checkpoint(store, &command.conversation_id)?
        .ok_or_else(|| "Memory Worker plan has no canonical checkpoint".to_string())?;
    let checkpoint =
        serde_json::from_str::<CoreCheckpoint<AgentThreadState>>(&stored.checkpoint_json)
            .map_err(|error| format!("decode Core checkpoint failed: {error}"))?;
    nola_agent_engine::agent::plan_memory_worker_from_state(
        &checkpoint.state,
        &command.job_id,
    )
}

pub fn begin_maintenance_turn<R: RunCreationRepository + crate::repositories::AssistantPublicationRepositories + ?Sized>(
    store: &mut R,
    command: BeginMaintenanceTurn,
) -> Result<AcceptedMaintenanceTurn, String> {
    crate::assistant_publication_transaction::with_publication_transaction(store, |store|
        begin_maintenance_turn_in_transaction(store, command))
}

fn begin_maintenance_turn_in_transaction<R: RunCreationRepository + crate::repositories::AssistantPublicationRepositories + ?Sized>(
    store: &mut R,
    command: BeginMaintenanceTurn,
) -> Result<AcceptedMaintenanceTurn, String> {
    if !matches!(&command.submission, AgentSubmission::Compact { operation_id } if !operation_id.trim().is_empty())
        && !matches!(&command.submission, AgentSubmission::CloseSession { submission_id: Some(id) } if !id.trim().is_empty())
    {
        return Err(
            "maintenance Run admission requires an explicit compaction or session-close operation".to_string(),
        );
    }
    if store
        .active_run(&command.conversation_id)
        .map_err(|error| format!("load active Run failed: {error}"))?
        .is_some()
    {
        return Err("maintenance turn cannot overlap an active Run".to_string());
    }
    let (invocation_config_json, invocation_config_digest) =
        invocation_config_snapshot(&command.core_config)?;
    let stored = crate::load_current_checkpoint(store, &command.conversation_id)?;
    let mut core = match stored {
        Some(stored) => {
            let checkpoint = crate::decode_idle_checkpoint(&stored.checkpoint_json)?;
            if checkpoint.state.journal.session_closed {
                return Err("session is already closed; no new maintenance Run admitted".into());
            }
            if checkpoint.state.journal.session_closing {
                return Err("session close already admitted; recover its original Run instead".into());
            }
            AgentCore::restore(command.core_config, checkpoint)
                .map_err(|error| format!("restore Core for maintenance turn failed: {error}"))?
        }
        None => AgentCore::try_new(command.core_config)
            .map_err(|error| format!("initialize Core for maintenance turn failed: {error}"))?,
    };
    let transition = core
        .reduce_at(command.submission, decision_time_ms(command.created_at_ms)?)
        .map_err(|error| format!("reduce maintenance submission failed: {error}"))?;
    let execution = core.host_execution_state();
    let checkpoint = core.into_checkpoint(
        transition
            .durability
            .as_ref()
            .map(|intent| intent.checkpoint_reason)
            .unwrap_or(nola_agent_engine::agent::recovery::CheckpointReason::Manual),
    );
    let memory_usage_entries = unrecorded_checkpoint_usage(store, &checkpoint)?;
    let run_id = execution
        .control
        .active_turn_id
        .as_ref()
        .map(|turn| turn.0.clone())
        .ok_or_else(|| "maintenance transition has no active Run".to_string())?;
    validate_maintenance_checkpoint(&run_id, &checkpoint, &transition.event_envelopes)?;
    let mut outbox_effects = external_outbox_requests(store, &transition.host_requests)?;
    let model_attempt_mutations =
        model_attempt::prepare_effects(store, &run_id, &mut outbox_effects, command.created_at_ms)?;
    let delegated_invocation_mutations = delegated_invocation::prepare_effects(
        store,
        &run_id,
        &mut outbox_effects,
        command.created_at_ms,
    )?;
    let mut delegated_permission_mutations = Vec::new();
    delegated_permission::prepare_effects(
        store,
        &run_id,
        &mut outbox_effects,
        &mut delegated_permission_mutations,
        command.created_at_ms,
    )?;
    store
        .begin_maintenance_run(&BeginMaintenanceRunInput {
            initial_events: crate::initial_run_events::plan(&transition.event_envelopes)?,
            conversation_id: command.conversation_id.clone(),
            run_id: run_id.clone(),
            context_checkpoint_id: command.context_checkpoint_id,
            checkpoint_revision: checkpoint.revision as i64,
            checkpoint_event_cursor: checkpoint.last_event_cursor.sequence as i64,
            checkpoint_json: serde_json::to_string(&checkpoint)
                .map_err(|error| format!("encode maintenance checkpoint failed: {error}"))?,
            invocation_config_json,
            invocation_config_digest: invocation_config_digest.clone(),
            outbox_effects,
            model_attempt_mutations,
            delegated_invocation_mutations,
            delegated_permission_mutations,
            memory_usage_entries,
            created_at_ms: command.created_at_ms,
        })
        .map_err(|error| format!("begin canonical maintenance Run failed: {error}"))?;
    crate::execution_trace_journal::record_accepted(store, &command.conversation_id, &transition.trace_events)?;
    Ok(AcceptedMaintenanceTurn {
        run_id,
        invocation_config_digest,
        execution,
        events: transition.event_envelopes,
        traces: transition.trace_events,
    })
}

/// Maintenance settles its own operation, not the user's requirements. The
/// original compaction events and checkpoint are durable operation evidence;
/// no assistant message or ordinary RunResult may be manufactured for it.
pub(crate) fn validate_maintenance_checkpoint<S: std::borrow::Borrow<AgentThreadState>>(
    run_id: &str,
    checkpoint: &CoreCheckpoint<S>,
    events: &[CoreEventEnvelope<AgentEvent>],
) -> Result<(), String> {
    let state = checkpoint.state.borrow();
    let turn = state
        .execution
        .active_turn
        .as_ref()
        .ok_or("maintenance checkpoint has no owning Turn")?;
    if turn.turn_id.as_str() != run_id
        || turn.context.input_origin
            != nola_agent_engine::agent::protocol::AgentInputOrigin::Maintenance
        || state.execution.requirements.scope.is_some()
        || state.execution.requirements.decision.is_some()
    {
        return Err(
            "maintenance checkpoint cannot own ordinary user requirements or another Turn".into(),
        );
    }
    if turn.status == nola_domain::AgentTurnStatus::Completed {
        let observed=state.conversation.transcript.iter().any(|item|matches!(item,
            nola_agent_engine::agent::AgentItem::Compaction{id,summary,source_fingerprint,..}
                if !summary.trim().is_empty() && !source_fingerprint.is_empty()
                    && state.item_ownership.get(id.as_str()).is_some_and(|owner|owner.turn_id==run_id)));
        let explicit_noop = events.iter().any(|event| {
            matches!(&event.event,
            AgentEvent::CompactionCompleted { replaced_item_count: 0, source_item_count: 0,
                compaction_item_id: None, summary, .. } if summary.is_empty())
        });
        if !observed && !explicit_noop && !state.journal.session_closed {
            return Err("completed maintenance has no owned compaction or session-close result".into());
        }
    }
    Ok(())
}
