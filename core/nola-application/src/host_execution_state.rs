// SPDX-License-Identifier: Apache-2.0
//! Committed Host execution inputs, without transcript or display projections.

use crate::*;

/// Explicit cold-view query. Task state is not part of streaming Host replies;
/// callers bind this projection to their exact Run recovery witness.
pub(crate) fn load_task_projection<R: RunEffectRepository + ?Sized>(
    store: &R,
    conversation_id: &str,
) -> Result<serde_json::Value, String> {
    let revision = store.checkpoint_revision(conversation_id).map_err(|e| e.to_string())?
        .ok_or("Task projection has no committed revision")?;
    let stored = crate::load_current_checkpoint(store, conversation_id)?
        .ok_or("Task projection has no committed checkpoint")?;
    let checkpoint: CoreCheckpoint<AgentThreadState> = serde_json::from_str(&stored.checkpoint_json)
        .map_err(|e| e.to_string())?;
    crate::assistant_owner::validate_checkpoint_identity(
        conversation_id, &stored.conversation_id, &checkpoint.session_id,
        checkpoint.state.session_id.as_str(),
    ).map_err(|e| e.to_string())?;
    checkpoint.validate_state_identity()?;
    if stored.revision != revision
        || i64::try_from(checkpoint.last_event_cursor.sequence).ok() != Some(stored.event_cursor)
        || store.checkpoint_revision(conversation_id).map_err(|e| e.to_string())? != Some(revision) {
        return Err("Task projection changed during read".into());
    }
    Ok(serde_json::json!({
        "core_session_id": checkpoint.session_id,
        "checkpoint_revision": revision,
        "checkpoint_event_cursor": stored.event_cursor,
        "task_projection": checkpoint.state.task_projection,
    }))
}

pub(crate) fn load<R: RunEffectRepository + ?Sized>(store: &R, conversation_id: &str)
    -> Result<HostExecutionState, String>
{
    let stored = crate::load_current_checkpoint(store, conversation_id)?
        .ok_or("Host execution state has no committed checkpoint")?;
    let checkpoint: CoreCheckpoint<AgentThreadState> = serde_json::from_str(&stored.checkpoint_json)
        .map_err(|error| error.to_string())?;
    crate::assistant_owner::validate_checkpoint_identity(conversation_id, &stored.conversation_id,
        &checkpoint.session_id, checkpoint.state.session_id.as_str()).map_err(|e| e.to_string())?;
    checkpoint.validate_state_identity()?;
    if i64::try_from(checkpoint.last_event_cursor.sequence).ok() != Some(stored.event_cursor) {
        return Err("Host execution checkpoint cursor changed".into());
    }
    let state = &checkpoint.state;
    let config = AgentCoreConfig {
        automation_execution_spec: None,
        session_id: Some(state.session_id.clone()), model: state.configuration.model.clone(),
        max_model_iterations: state.configuration.max_model_iterations, initial_items: Vec::new(),
        max_strategy_recovery_attempts: state.configuration.max_strategy_recovery_attempts,
        tools: state.configuration.tools.clone(), context_window: state.configuration.context_window.clone(),
        memory_policy: state.configuration.memory_policy.clone(), turn_identity: state.configuration.turn_identity.clone(),
        host_catalog: Some(state.host.catalog.clone()), permission_rules: state.execution.permission_rules.clone(),
    };
    let core = AgentCore::restore(config, checkpoint)?;
    Ok(core.host_execution_state())
}
