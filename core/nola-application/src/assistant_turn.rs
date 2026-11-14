// SPDX-License-Identifier: Apache-2.0
//! Canonical assistant-turn admission and initial Host effect staging.

use nola_agent_engine::agent::host::HostRequest;
use nola_agent_engine::agent::protocol::{
    AgentCoreConfig, AgentEvent, AgentSubmission,
};
use nola_agent_engine::agent::recovery::{CoreCheckpoint};
use nola_domain::core_event::{CoreEventEnvelope};
use nola_agent_engine::agent::{AgentCore, AgentThreadState};
use nola_agent_engine::agent_trace::AgentTraceEvent;
use nola_store_port::run_creation::{
    BeginInternalTurnInput, BeginSuccessorTurnInput, BeginTurnInput, RunCreationRepository,
};
use serde::{Deserialize, Serialize};

use crate::canonical_part_projection::{CanonicalPartProjection, CanonicalPartProjectionPlanner};
use crate::{
    canonical_branch, decision_time_ms, delegated_invocation, delegated_permission,
    ensure_applied_user_messages, external_outbox_requests, input_origin_uses_internal_turn,
    invocation_config_snapshot, model_attempt, unrecorded_checkpoint_usage,
    TurnOpened, HostExecutionState,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BeginAssistantTurn {
    pub conversation_id: String,
    pub expected_configuration_revision: Option<i64>,
    pub expected_parent_message_id: Option<String>,
    pub conversation_title: String,
    pub user_message_id: String,
    pub user_part_id: String,
    pub user_text: String,
    pub user_content_json: Option<String>,
    pub assistant_message_id: String,
    pub context_checkpoint_id: Option<String>,
    pub core_config: AgentCoreConfig,
    pub submission: AgentSubmission,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcceptedAssistantTurn {
    pub begin: TurnOpened,
    pub assistant_publications: Vec<nola_domain::assistant_journal::AssistantJournalCursor>,
    pub invocation_config_digest: String,
    pub execution: HostExecutionState,
    pub events: Vec<CoreEventEnvelope<AgentEvent>>,
    pub traces: Vec<AgentTraceEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromoteQueuedAssistantTurn {
    pub conversation_id: String,
    pub created_at_ms: i64,
}

/// Host supplies execution configuration; user content comes exclusively from
/// the leased durable queue. Admission and queue acknowledgement commit together.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BeginQueuedUserTurn {
    pub conversation_id: String,
    pub command_id: String,
    pub lease_owner: String,
    pub delivery_attempt: i64,
    pub expected_configuration_revision: i64,
    pub expected_parent_message_id: Option<String>,
    pub core_config: AgentCoreConfig,
    pub turn_context_items: Vec<nola_agent_engine::agent::AgentItem>,
    pub turn_context_platform: nola_agent_engine::context_engine::ContextPlatform,
    pub turn_context_components: Vec<nola_agent_engine::context_engine::ContextComponent>,
    pub now_ms: i64,
}

pub fn begin_queued_user_turn<R: RunCreationRepository
    + nola_store_port::conversation_history::CanonicalHistoryRepository
    + crate::repositories::AssistantAdmissionRepositories
    + nola_store_port::runtime_task::RuntimeTaskRepository
    + nola_store_port::agent_queue::AgentQueueRepository + ?Sized>(
    store: &mut R, input: BeginQueuedUserTurn,
) -> Result<AcceptedAssistantTurn, String> {
    crate::assistant_publication_transaction::with_publication_transaction(store, |store| {
        let queued = store.peek_agent_command(&input.conversation_id).map_err(|error| error.to_string())?
            .ok_or("queued user input is absent")?;
        if queued.command_id != input.command_id
            || queued.lease_owner.as_deref() != Some(input.lease_owner.as_str())
            || input.delivery_attempt <= 0 || queued.attempt_count != input.delivery_attempt
            || queued.lease_until_ms.is_none_or(|until| until <= input.now_ms) {
            return Err("queued user input lease is stale or is not the queue head".into());
        }
        let envelope: nola_agent_engine::agent::host::AgentBoundaryInput =
            serde_json::from_str(&queued.payload_json).map_err(|error| error.to_string())?;
        let nola_agent_engine::agent::host::AgentBoundaryInput::Submission { mut submission } = envelope else {
            return Err("queue head is not a user submission".into());
        };
        let AgentSubmission::UserInput { items, submission_id, submitted_at_epoch_ms,
            turn_context_items, turn_context_platform, turn_context_components, .. } = &mut submission else {
            return Err("queue head requires another admission owner".into());
        };
        if submission_id.as_deref() != Some(input.command_id.as_str()) {
            return Err("queued user input identity mismatch".into());
        }
        *turn_context_items = input.turn_context_items;
        *turn_context_platform = input.turn_context_platform;
        *turn_context_components = input.turn_context_components;
        let submitted_at = submitted_at_epoch_ms.ok_or("queued user input has no timestamp")?;
        let mut messages = Vec::new();
        ensure_applied_user_messages(&mut messages, items, &input.command_id, submitted_at)?;
        if messages.len() != 1 { return Err("queued user input requires exactly one user message".into()); }
        let user = &messages[0];
        let part = user.initial_parts.first().ok_or("queued user input has no content")?;
        let (accepted, _) = admit_with_core(store, BeginAssistantTurn {
            conversation_id: input.conversation_id.clone(),
            expected_configuration_revision: Some(input.expected_configuration_revision),
            expected_parent_message_id: input.expected_parent_message_id,
            conversation_title: input.conversation_id.clone(),
            user_message_id: user.id.clone(), user_part_id: part.id.clone(),
            user_text: part.content_text.clone(), user_content_json: part.content_json.clone(),
            assistant_message_id: format!("{}:assistant", input.command_id),
            context_checkpoint_id: None, core_config: input.core_config,
            submission, created_at_ms: input.now_ms,
        }, None)?;
        if !store.acknowledge_agent_command(&input.conversation_id, &input.command_id, &input.lease_owner)
            .map_err(|error| error.to_string())? {
            return Err("queued user input acknowledgement lost its lease".into());
        }
        Ok(accepted)
    })
}

struct SuccessorAdmission {
    previous_run_id: String,
    expected_checkpoint_revision: i64,
    admission_id: String,
    originating_submission_id: String,
}

pub fn promote_queued_assistant_turn<R: RunCreationRepository
    + nola_store_port::conversation_history::CanonicalHistoryRepository
    + crate::repositories::AssistantAdmissionRepositories
    + nola_store_port::runtime_task::RuntimeTaskRepository + ?Sized>(
    store: &mut R, command: PromoteQueuedAssistantTurn,
) -> Result<Option<AcceptedAssistantTurn>, String> {
    promote_with_core(store, command).map(|result| result.map(|(accepted, _)| accepted))
}

pub(crate) fn promote_with_core<R: RunCreationRepository
    + nola_store_port::conversation_history::CanonicalHistoryRepository
    + crate::repositories::AssistantAdmissionRepositories
    + nola_store_port::runtime_task::RuntimeTaskRepository + ?Sized>(
    store: &mut R, command: PromoteQueuedAssistantTurn,
) -> Result<Option<(AcceptedAssistantTurn, AgentCore)>, String> {
    crate::assistant_publication_transaction::with_publication_transaction(store, |store| {
        if store.active_run(&command.conversation_id).map_err(|e| e.to_string())?.is_some() {
            return Ok(None);
        }
        let Some(stored) = crate::load_current_checkpoint(store, &command.conversation_id)? else { return Ok(None) };
        let checkpoint: CoreCheckpoint<AgentThreadState> = serde_json::from_str(&stored.checkpoint_json)
            .map_err(|e| format!("decode successor checkpoint failed: {e}"))?;
        checkpoint.validate_state_identity()?;
        let Some(batch) = checkpoint.state.execution.input_queue.pending_successor() else { return Ok(None) };
        if store.is_runtime_task_child_session(&command.conversation_id).map_err(|e| e.to_string())? {
            return Err("task child successor requires independent task lease admission".into());
        }
        let previous = checkpoint.state.execution.active_turn.as_ref().ok_or("pending successor has no predecessor")?;
        if !previous.status.is_terminal() || !checkpoint.state.host.pending_effects.is_quiescent() {
            return Err("pending successor predecessor has not settled".into());
        }
        let submission_id = batch.admission.submission_id.clone().ok_or("successor has no originating submission identity")?;
        let source_run = batch.admission.anchor_turn_id.as_ref().ok_or("successor has no source Run")?;
        let frozen = store.load_run_invocation_config(source_run).map_err(|e| e.to_string())?
            .ok_or("successor source Run has no frozen configuration")?;
        let config = crate::load_run_invocation_config(store, source_run, &frozen.invocation_config_digest)?;
        let mut messages = Vec::new();
        ensure_applied_user_messages(&mut messages, &batch.items, &submission_id, command.created_at_ms)?;
        let user = messages.first().ok_or("queued successor has no visible user input")?;
        let part = user.initial_parts.first().ok_or("queued successor has no user content")?;
        let input = BeginAssistantTurn {
            expected_configuration_revision: None,
            expected_parent_message_id: None,
            conversation_id: command.conversation_id.clone(), conversation_title: command.conversation_id,
            user_message_id: user.id.clone(), user_part_id: part.id.clone(),
            user_text: part.content_text.clone(), user_content_json: part.content_json.clone(),
            assistant_message_id: format!("{submission_id}:assistant"), context_checkpoint_id: None,
            core_config: config, created_at_ms: command.created_at_ms,
            submission: AgentSubmission::PromoteQueuedInput {
                turn_context_components: Vec::new(),
                admission_id: batch.admission.admission_id.clone(),
                expected_previous_turn_id: previous.turn_id.as_str().into(), successor_turn_id: submission_id.clone(),
            },
        };
        let successor = SuccessorAdmission { previous_run_id: previous.turn_id.as_str().into(),
            expected_checkpoint_revision: stored.revision, admission_id: batch.admission.admission_id.clone(),
            originating_submission_id: submission_id };
        begin_assistant_turn_in_transaction(store, input, Some(successor)).map(Some)
    })
}

pub fn begin_assistant_turn<
    R: RunCreationRepository
        + nola_store_port::conversation_history::CanonicalHistoryRepository
        + crate::repositories::AssistantAdmissionRepositories
        + ?Sized,
>(
    store: &mut R,
    command: BeginAssistantTurn,
) -> Result<AcceptedAssistantTurn, String> {
    if command.core_config.automation_execution_spec.is_some() {
        return Err("automation execution requires explicit BeginAutomationAssistantTurn admission".into());
    }
    crate::assistant_publication_transaction::with_publication_transaction(store, |store|
        begin_assistant_turn_in_transaction(store, command, None).map(|(accepted, _)| accepted))
}

/// Shared command admission gate. Stateful dispatch keeps the accepted Core;
/// stateless dispatch drops it. Neither path reconstructs it from its reply.
pub(crate) fn admit_with_core<R: RunCreationRepository
    + nola_store_port::conversation_history::CanonicalHistoryRepository
    + crate::repositories::AssistantAdmissionRepositories
    + nola_store_port::runtime_task::RuntimeTaskRepository + ?Sized>(
    store: &mut R,
    mut command: BeginAssistantTurn,
    lease: Option<crate::RuntimeTaskLeaseWitness>,
) -> Result<(AcceptedAssistantTurn, AgentCore), String> {
    if command.core_config.automation_execution_spec.is_some() {
        return Err("automation execution requires explicit BeginAutomationAssistantTurn admission".into());
    }
    let child = command.conversation_id.clone();
    let at = command.created_at_ms;
    crate::assistant_publication_transaction::with_publication_transaction(store, |store| {
        if let Some(lease) = &lease {
            crate::expert_peer_task::decorate_task_admission(store,lease,&mut command)?;
            store.validate_runtime_task_turn_admission(lease, &child, at)
                .map_err(|error| error.to_string())?;
        } else if store.is_runtime_task_child_session(&child).map_err(|error| error.to_string())? {
            return Err("task child requires explicit BeginTaskAssistantTurn lease admission".into());
        }
        let (accepted, core) = begin_assistant_turn_in_transaction(store, command, None)?;
        if let Some(lease) = &lease {
            store.bind_runtime_task_run(lease, &child, &accepted.begin.run_id, at)
                .map_err(|error| error.to_string())?;
            // Parent subscribers observe their own durable head through the
            // shared projection wakeup. Never send a parent hint to this child stream.
            crate::assistant_journal_producer::publish_task_run_binding(
                store,&lease.task_id,&accepted.begin.run_id)?;
        }
        Ok((accepted, core))
    })
}

pub(crate) fn begin_automation_turn_in_transaction<R: RunCreationRepository
    + nola_store_port::conversation_history::CanonicalHistoryRepository
    + crate::repositories::AssistantAdmissionRepositories
    + nola_store_port::runtime_task::RuntimeTaskRepository>(
    store: &mut R, command: BeginAssistantTurn,
) -> Result<(AcceptedAssistantTurn, AgentCore), String> {
    if store.is_runtime_task_child_session(&command.conversation_id).map_err(|error|error.to_string())? {
        return Err("automation cannot claim a task child conversation".into());
    }
    begin_assistant_turn_in_transaction(store, command, None)
}

fn begin_assistant_turn_in_transaction<R: RunCreationRepository
    + nola_store_port::conversation_history::CanonicalHistoryRepository
    + crate::repositories::AssistantAdmissionRepositories + ?Sized>(
    store: &mut R,
    mut command: BeginAssistantTurn,
    successor: Option<SuccessorAdmission>,
) -> Result<(AcceptedAssistantTurn, AgentCore), String> {
    crate::main_expert_selection::admit(store, &mut command)?;
    if let AgentSubmission::AutomatedInput {
        origin: nola_agent_engine::agent::AgentInputOrigin::GoalContinuation,
        items, submission_id, ..
    } = &command.submission {
        if items.len() != 1 {
            return Err("Goal continuation admission requires its formal trigger item".into());
        }
        let trigger = nola_agent_engine::agent::protocol::InternalTurnTrigger::from_item(&items[0])?;
        if trigger.origin != nola_agent_engine::agent::AgentInputOrigin::GoalContinuation
            || trigger.conversation_id.as_deref() != Some(command.conversation_id.as_str())
            || submission_id.as_ref().is_none_or(|id| trigger.trigger_id != format!("{id}:internal-trigger"))
        {
            return Err("Goal continuation trigger differs from its canonical owner or submission".into());
        }
    }
    if matches!(&command.submission, AgentSubmission::GoalUserInput { .. }
        | AgentSubmission::AutomatedInput { origin:nola_agent_engine::agent::AgentInputOrigin::GoalContinuation, .. }) {
        crate::goal_execution::validate_goal_round(store,&command.conversation_id,&command.core_config,command.created_at_ms)?;
    }
    let (invocation_config_json, invocation_config_digest) =
        invocation_config_snapshot(&command.core_config)?;
    let internal_execution = match &command.submission {
        AgentSubmission::AutomatedInput { origin, .. } => input_origin_uses_internal_turn(*origin),
        _ => false,
    };
    match &command.submission {
        AgentSubmission::UserDelegation { submission_id, .. } if !submission_id.trim().is_empty() => {}
        AgentSubmission::PromoteQueuedInput { .. } if successor.is_some() => {}
        AgentSubmission::UserInput {
            submission_id: Some(submission_id),
            ..
        }
        | AgentSubmission::GoalUserInput {
            submission_id: Some(submission_id),
            ..
        }
        | AgentSubmission::AutomatedInput {
            submission_id: Some(submission_id),
            ..
        } if !submission_id.trim().is_empty() => {}
        AgentSubmission::UserInput { .. }
        | AgentSubmission::GoalUserInput { .. }
        | AgentSubmission::AutomatedInput { .. } => {
            return Err("assistant turn requires a stable submission_id".to_string())
        }
        _ => return Err("assistant turn requires user or automated input".to_string()),
    }
    if store
        .active_run(&command.conversation_id)
        .map_err(|error| format!("load active Run failed: {error}"))?
        .is_some()
    {
        return Err("assistant turn cannot overlap an active Run".to_string());
    }
    let internal_target = if internal_execution {
        Some(
            store
                .internal_assistant_target(&command.conversation_id)
                .map_err(|error| format!("load exact internal assistant target failed: {error}"))?
                .ok_or("internal assistant turn requires an existing canonical target")?,
        )
    } else {
        None
    };
    let admission_message_id = internal_target
        .as_ref()
        .map(|target| target.assistant_message_id.clone())
        .unwrap_or_else(|| command.assistant_message_id.clone());
    let stored = crate::load_current_checkpoint(store, &command.conversation_id)?;
    let expected_checkpoint_revision = stored.as_ref().map(|checkpoint| checkpoint.revision);
    let needs_canonical_context = stored.is_none();
    let context_prompt_limit = command
        .core_config
        .context_window
        .clone()
        .normalized()
        .prompt_token_limit();
    let mut core = match stored {
        Some(stored) => {
            let value = serde_json::from_str(&stored.checkpoint_json)
                .map_err(|error| format!("decode Core checkpoint failed: {error}"))?;
            let checkpoint = if successor.is_some() {
                serde_json::from_value(value).map_err(|e| format!("decode successor Core checkpoint failed: {e}"))?
            } else { CoreCheckpoint::<AgentThreadState>::decode_history_for_new_turn(value)? };
            AgentCore::restore(command.core_config, checkpoint)
                .map_err(|error| format!("restore Core for assistant turn failed: {error}"))?
        }
        None => AgentCore::try_new(command.core_config)
            .map_err(|error| format!("initialize Core for assistant turn failed: {error}"))?,
    };
    if needs_canonical_context {
        canonical_branch::restore_canonical_context(
            store,
            &command.conversation_id,
            &mut core,
            context_prompt_limit,
        )?;
    }
    if matches!(&command.submission, AgentSubmission::UserInput { .. } | AgentSubmission::GoalUserInput { .. } | AgentSubmission::PromoteQueuedInput { .. }) {
        let surfaces = store.ui_surfaces_for_context(&command.conversation_id)
            .map_err(|error| format!("read admitted UI context failed: {error}"))?;
        crate::generative_ui::attach_client_data_model(&mut command.submission, &surfaces)?;
    }
    core.bind_next_trace_root(&admission_message_id)?;
    let transition = core
        .reduce_at(command.submission, decision_time_ms(command.created_at_ms)?)
        .map_err(|error| format!("reduce assistant turn submission failed: {error}"))?;
    let execution = core.host_execution_state();
    let checkpoint = core.checkpoint_view(transition.durability.as_ref()
        .map(|intent| intent.checkpoint_reason)
        .unwrap_or(nola_agent_engine::agent::recovery::CheckpointReason::Manual));
    let memory_usage_entries =
        unrecorded_checkpoint_usage(store, &checkpoint)?;
    let run_id = execution
        .control
        .active_turn_id
        .as_ref()
        .map(|turn| turn.0.clone())
        .ok_or_else(|| "assistant turn transition has no active turn".to_string())?;
    let assistant_part_id = transition
        .host_requests
        .iter()
        .find_map(|request| match &request.payload {
            HostRequest::ModelStream { text_part_id, .. } => Some(text_part_id.clone()),
            _ => None,
        })
        .unwrap_or_else(|| format!("{run_id}:assistant:text"));
    let accepted_turn_id = execution
        .control
        .active_turn_id
        .as_ref()
        .ok_or("assistant turn transition has no accepted Turn owner")?
        .as_str()
        .to_owned();
    let mut assistant_turn_bindings = vec![nola_domain::AssistantTurnBinding::Admission {
        turn_id: accepted_turn_id.clone(),
        assistant_message_id: admission_message_id.clone(),
    }];
    for event in &transition.event_envelopes {
        if let AgentEvent::AssistantActivityBound { turn_id, activity_id, activity_kind, .. } = &event.event {
            assistant_turn_bindings.push(nola_domain::AssistantTurnBinding::Activity {
                turn_id:turn_id.as_str().into(), assistant_message_id:admission_message_id.clone(),
                activity_id:activity_id.clone(),activity_kind:activity_kind.clone(),
            });
        }
        if let AgentEvent::AssistantEffectBound { turn_id, effect_id, .. } = &event.event {
            assistant_turn_bindings.push(nola_domain::AssistantTurnBinding::Effect {
                turn_id: turn_id.as_str().into(), assistant_message_id: admission_message_id.clone(), effect_id: effect_id.clone(),
            });
        }
    }
    if !internal_execution {
        // Ordinary admission physically creates this placeholder. Internal
        // resumption reuses a message; only an actual initial PartUpdated
        // materializes a new Part here. A placeholder ID alone does not prove
        // creation; without that event a later accepted Host update creates it.
        assistant_turn_bindings.push(nola_domain::AssistantTurnBinding::Part {
            turn_id: accepted_turn_id.clone(),
            assistant_message_id: admission_message_id.clone(),
            part_id: assistant_part_id.clone(),
        });
    }
    let initial_assistant_part_mutations = initial_assistant_part_mutations(
        store,
        &command.conversation_id,
        &run_id,
        &accepted_turn_id,
        &admission_message_id,
        &transition.event_envelopes,
        command.created_at_ms,
        &mut assistant_turn_bindings,
    )?;
    let checkpoint_revision = checkpoint.revision as i64;
    let checkpoint_event_cursor = checkpoint.last_event_cursor.sequence as i64;
    let checkpoint_json = serde_json::to_string(&checkpoint)
        .map_err(|error| format!("encode assistant turn checkpoint failed: {error}"))?;
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
    let immediate_terminal = outbox_effects.is_empty()
        && checkpoint.state.host.pending_effects.is_quiescent()
        && transition.event_envelopes.iter().any(|event| matches!(
            &event.event, AgentEvent::TurnCompleted { turn_id, .. } if turn_id.as_str() == run_id
        ))
        && transition.event_envelopes.iter().any(|event| matches!(
            &event.event, AgentEvent::RunResultDecided { .. }
        ));
    if outbox_effects.is_empty() && !immediate_terminal {
        return Err("staged Core transition has no external Host effect".to_string());
    }
    let mut applied_user_messages = Vec::new();
    for envelope in &transition.event_envelopes {
        if let AgentEvent::InputApplied { items, .. } = &envelope.event {
            ensure_applied_user_messages(
                &mut applied_user_messages,
                items,
                &run_id,
                command.created_at_ms,
            )?;
        }
    }
    let additional_user_parts = applied_user_messages
        .iter()
        .find(|message| message.id == command.user_message_id)
        .map(|message| {
            message
                .initial_parts
                .iter().cloned()
                .filter(|part| {
                    part.id != command.user_part_id && !(part.kind == "text" && part.ordinal == 1)
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let journal_admission_part_id = assistant_part_id.clone();
    let (begin, journal_parts, journal_attempts) = if internal_execution {
        let target = internal_target.ok_or("missing frozen internal assistant target")?;
        let input = BeginInternalTurnInput {
            initial_events: crate::initial_run_events::plan(&transition.event_envelopes)?,
            assistant_turn_bindings,
            initial_assistant_part_mutations,
            expected_previous_run_id: target.run_id,
            expected_checkpoint_revision,
            conversation_id: command.conversation_id,
            assistant_message_id: admission_message_id,
            assistant_part_id,
            run_id,
            context_checkpoint_id: command.context_checkpoint_id,
            checkpoint_revision,
            checkpoint_event_cursor,
            checkpoint_json,
            invocation_config_json,
            invocation_config_digest: invocation_config_digest.clone(),
            outbox_effects,
            model_attempt_mutations,
            delegated_invocation_mutations,
            delegated_permission_mutations,
            memory_usage_entries,
            created_at_ms: command.created_at_ms,
        };
        let begin = store.begin_internal_turn(&input)
            .map_err(|error| format!("begin canonical Core turn failed: {error}"))?;
        (begin, input.initial_assistant_part_mutations, input.model_attempt_mutations)
    } else {
        let input = BeginTurnInput {
            user_part_kind: applied_user_messages.iter().find(|message| message.id == command.user_message_id)
                .and_then(|message| message.initial_parts.iter().find(|part| part.id == command.user_part_id))
                .map(|part| part.kind.clone()).unwrap_or_else(|| "text".into()),
            expected_configuration_revision: command.expected_configuration_revision,
            expected_parent_message_id: command.expected_parent_message_id,
            initial_events: crate::initial_run_events::plan(&transition.event_envelopes)?,
            assistant_turn_bindings,
            initial_assistant_part_mutations,
            conversation_id: command.conversation_id,
            conversation_title: command.conversation_title,
            user_message_id: command.user_message_id,
            user_part_id: command.user_part_id,
            user_text: command.user_text,
            user_content_json: command.user_content_json,
            additional_user_parts,
            assistant_message_id: command.assistant_message_id,
            assistant_part_id,
            run_id,
            context_checkpoint_id: command.context_checkpoint_id,
            checkpoint_revision,
            checkpoint_event_cursor,
            checkpoint_json,
            invocation_config_json,
            invocation_config_digest: invocation_config_digest.clone(),
            outbox_effects,
            model_attempt_mutations,
            delegated_invocation_mutations,
            delegated_permission_mutations,
            memory_usage_entries,
            created_at_ms: command.created_at_ms,
        };
        let begin = if let Some(successor) = successor {
            store.begin_successor_turn(&BeginSuccessorTurnInput {
                previous_run_id: successor.previous_run_id,
                expected_checkpoint_revision: successor.expected_checkpoint_revision,
                admission_id: successor.admission_id,
                originating_submission_id: successor.originating_submission_id,
                turn: input.clone(), ensured_user_messages: applied_user_messages,
            }).map_err(|error| format!("begin successor Run failed: {error}"))?
        } else {
            store.begin_turn(&input).map_err(|error| format!("begin canonical Core turn failed: {error}"))?
        };
        (begin, input.initial_assistant_part_mutations, input.model_attempt_mutations)
    };
    // This is a Store publication transition, not a fabricated Host response
    // or an additional Core reduction. Store and Core revision clocks differ.
    let terminal_transition_id = if immediate_terminal {
        use nola_store_port::run_effect::{CheckpointWrite, CommitTransitionInput};
        let transition_id = format!("{}:admission-terminal", begin.run_id);
        store.commit_transition(&CommitTransitionInput {
            transition_id: transition_id.clone(), run_id: begin.run_id.clone(),
            expected_checkpoint_revision: checkpoint_revision,
            checkpoint_revision: checkpoint_revision.checked_add(1).ok_or("Store revision overflow")?,
            checkpoint_event_cursor,
            checkpoint: CheckpointWrite::Baseline { checkpoint_json: serde_json::to_string(&checkpoint).map_err(|e|e.to_string())? },
            host_event: None, submission_reduction_json: None, events: vec![],
            ensured_messages: vec![], part_message_owners: Default::default(),
            assistant_turn_bindings: vec![], part_mutations: vec![], outbox_effects: vec![],
            model_attempt_mutations: vec![], delegated_invocation_mutations: vec![],
            delegated_permission_mutations: vec![], effect_delivery_attempt_mutations: vec![],
            acknowledged_effect_ids: vec![], memory_usage_entries: vec![],
            run_status: Some("sealed".into()), assistant_status: Some("completed".into()),
            assistant_status_message_id: Some(begin.assistant_message_id.clone()),
            committed_at_ms: command.created_at_ms,
        }).map_err(|e| format!("commit immediate terminal admission failed: {e}"))?;
        Some(transition_id)
    } else { None };
    let mut changes = crate::assistant_journal_changes::collect_assistant_journal_changes(
        crate::assistant_journal_changes::AssistantJournalChangeScope {
            conversation_id: &begin.conversation_id, core_session_id: &checkpoint.session_id,
            source_run_id: Some(&begin.run_id), maintenance: false,
        }, &transition.event_envelopes, &checkpoint.state.skill_runtime, &journal_attempts, &journal_parts,
    )?;
    // Normal admission owns an empty canonical text slot, but must not reserve
    // a visible transcript position before the provider emits its first item.
    if !internal_execution {
        changes.changes.insert(0, crate::assistant_journal_changes::AssistantJournalChange::AdmissionText {
            part_id: &journal_admission_part_id,
        });
    }
    let publication = crate::assistant_journal_producer::publish_changed_assistant_journal(
        store, match &terminal_transition_id {
            Some(transition_id) => nola_store_port::assistant_journal::AssistantSourceCommitStamp::Transition { transition_id: transition_id.clone() },
            None => nola_store_port::assistant_journal::AssistantSourceCommitStamp::Admission { run_id: begin.run_id.clone() },
        },
        &begin.conversation_id, &checkpoint.session_id, Some(&begin.run_id), &changes,
    )?;
    let mut assistant_publications =
        crate::assistant_journal_producer::publication_cursors(publication);
    if let Some(transition_id) = &terminal_transition_id {
        store.seal_assistant_run_terminal(transition_id).map_err(|e| e.to_string())?
            .ok_or("immediate terminal admission has no canonical Run seal")?;
    }
    if !internal_execution && assistant_publications.is_empty() {
        // Notify the admitted exact owner at its valid genesis frontier. The
        // consumer can attach immediately without a fabricated Text declaration
        // stealing presentation order from reasoning or tool activity.
        let identity = nola_domain::assistant_journal::AssistantJournalIdentity {
            source_store_epoch: store.assistant_source_epoch().map_err(|e| e.to_string())?,
            conversation_id: begin.conversation_id.clone(),
            run_id: begin.run_id.clone(),
            assistant_message_id: begin.assistant_message_id.clone(),
        };
        assistant_publications.push(store.assistant_journal_head(&identity)
            .map_err(|e| e.to_string())?);
    }
    drop(changes);
    crate::execution_trace_journal::record_accepted(store, &begin.conversation_id, &transition.trace_events)?;
    drop(checkpoint);
    Ok((AcceptedAssistantTurn {
        assistant_publications,
        begin: begin.into(),
        invocation_config_digest,
        execution,
        events: transition.event_envelopes,
        traces: transition.trace_events,
    }, core))
}

/// Initial events are the source, not the cumulative checkpoint projection.
/// The existing stream planner preserves withholding/review and append-only
/// body semantics exactly as it does at later durable Host boundaries.
fn initial_assistant_part_mutations<R: RunCreationRepository + ?Sized>(
    store: &R,
    conversation_id: &str,
    run_id: &str,
    turn_id: &str,
    admitted_message_id: &str,
    events: &[CoreEventEnvelope<AgentEvent>],
    at: i64,
    bindings: &mut Vec<nola_domain::AssistantTurnBinding>,
) -> Result<Vec<nola_domain::PartMutation>, String> {
    let mut planner = CanonicalPartProjectionPlanner::default();
    let mut mutations = Vec::new();
    for event in events {
        let AgentEvent::PartUpdated {
            turn_id: declared_turn,
            projection_revision,
            update,
            part,
        } = &event.event
        else {
            continue;
        };
        let view = CanonicalPartProjection::new(part, run_id, Some(admitted_message_id))?;
        if declared_turn.as_str() != turn_id
            || part.turn_id() != turn_id
            || view.message_id() != admitted_message_id
        {
            return Err("initial Core Part is outside the exact admitted assistant owner".into());
        }
        let owner = nola_domain::CanonicalStreamOwner {
            conversation_id: conversation_id.into(),
            run_id: run_id.into(),
            message_id: admitted_message_id.into(),
            part_id: part.id().into(),
            field: nola_domain::PartBodyField::ContentText,
        };
        mutations.extend(
            planner.plan_view(
                &owner,
                update,
                &view,
                i64::try_from(*projection_revision)
                    .map_err(|_| "initial Part ordinal exceeds range")?,
                at,
                |owner| {
                    store
                        .load_canonical_text_state(owner)
                        .map_err(|error| error.to_string())
                },
            )?,
        );
        let binding = nola_domain::AssistantTurnBinding::Part {
            turn_id: turn_id.into(),
            assistant_message_id: admitted_message_id.into(),
            part_id: part.id().into(),
        };
        if !bindings.contains(&binding) {
            bindings.push(binding);
        }
    }
    Ok(mutations)
}

#[cfg(test)]
mod initial_part_tests {
    use super::*;
    use nola_domain::{AssistantTurnBinding, PartMutation};
    use nola_agent_engine::agent::parts::{
        AgentPart, PartOwnership, PartUpdate, ReasoningPart, StepPart, StepStatus,
    };
    use nola_agent_engine::agent::protocol::TurnId;
    use nola_domain::core_event::EventCursor;

    fn event(part: AgentPart, update: PartUpdate, sequence: u64) -> CoreEventEnvelope<AgentEvent> {
        CoreEventEnvelope {
            session_id: "separate-core-session".into(),
            turn_id: Some("run".into()),
            cursor: EventCursor { sequence },
            revision: sequence,
            event: AgentEvent::PartUpdated {
                turn_id: TurnId::from_string("run".into()),
                projection_revision: sequence,
                update,
                part,
            },
        }
    }

    #[test]
    fn initial_rich_parts_keep_event_order_and_actual_message_owner() {
        let store = nola_store::NolaStore::open(":memory:").unwrap();
        let reasoning = AgentPart::Reasoning(ReasoningPart {
            id: "reasoning".into(),
            ownership: PartOwnership::for_turn("run"),
            title: "Reasoning".into(),
            detail: "first".into(),
            is_complete: false,
        });
        let step = AgentPart::Step(StepPart {
            id: "step".into(),
            ownership: PartOwnership::for_turn("run"),
            title: "Started".into(),
            status: StepStatus::Started,
        });
        let mut updated = reasoning.clone();
        let AgentPart::Reasoning(value) = &mut updated else {
            unreachable!()
        };
        value.detail.push_str(" second");
        let events = [
            event(
                reasoning.clone(),
                PartUpdate::Created { part: reasoning },
                1,
            ),
            event(step.clone(), PartUpdate::Created { part: step }, 2),
            event(
                updated,
                PartUpdate::ReasoningDelta {
                    part_id: "reasoning".into(),
                    title: None,
                    detail_delta: " second".into(),
                },
                3,
            ),
        ];
        let mut bindings = vec![AssistantTurnBinding::Admission {
            turn_id: "run".into(),
            assistant_message_id: "actual-slot".into(),
        }];
        let mutations = initial_assistant_part_mutations(
            &store,
            "canonical-conversation",
            "run",
            "run",
            "actual-slot",
            &events,
            10,
            &mut bindings,
        )
        .unwrap();
        assert_eq!(mutations.len(), 3);
        for (mutation, expected_id) in mutations.iter().zip(["reasoning", "step", "reasoning"]) {
            let PartMutation::Upsert {
                part_id,
                content_json,
                ..
            } = mutation
            else {
                panic!("rich Part uses the canonical mutation contract")
            };
            assert_eq!(part_id, expected_id);
            let part: AgentPart = serde_json::from_str(content_json.as_deref().unwrap()).unwrap();
            assert_eq!(part.message_id(), "actual-slot");
            assert_eq!(part.turn_id(), "run");
        }
        assert_eq!(
            bindings.len(),
            3,
            "repeated Part updates must not create duplicate binding"
        );
        let PartMutation::Upsert { content_text, .. } = &mutations[2] else {
            unreachable!()
        };
        assert_eq!(content_text, "first second");
    }

    #[test]
    fn initial_part_from_other_turn_cannot_be_rebound_to_admission() {
        let store = nola_store::NolaStore::open(":memory:").unwrap();
        let part = AgentPart::Step(StepPart {
            id: "foreign".into(),
            ownership: PartOwnership::for_turn("other-turn"),
            title: "Foreign".into(),
            status: StepStatus::Started,
        });
        let events = [event(part.clone(), PartUpdate::Created { part }, 1)];
        let mut bindings = Vec::new();
        assert!(initial_assistant_part_mutations(
            &store,
            "canonical-conversation",
            "run",
            "run",
            "actual-slot",
            &events,
            10,
            &mut bindings,
        )
        .is_err());
        assert!(bindings.is_empty());
    }
}
