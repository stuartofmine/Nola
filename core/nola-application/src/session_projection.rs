// SPDX-License-Identifier: Apache-2.0
//! Authoritative local and remote Agent session read models.

use std::collections::BTreeMap;

use nola_agent_engine::agent::capability::SkillRun;
use nola_agent_engine::agent::context_window::ContextUsageSnapshot;
use nola_agent_engine::agent::goal::GoalProjection;
use nola_agent_engine::agent::memory_runtime::MemoryRuntimeProjection;
use nola_agent_engine::agent::parts::ConversationProjection;
use nola_agent_engine::agent::permissions::{PermissionProjection, PermissionRuleSet};
use nola_agent_engine::agent::recovery::CoreCheckpoint;
use nola_agent_engine::agent::task::TaskProjection;
use nola_agent_engine::agent::AgentThreadState;
use nola_domain::memory_projection::MemoryProjection;
use nola_store_port::remote_session::{AuthoritativeSessionRecord, SessionProjectionRepository};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionSkillProjection {
    pub session_id: String,
    pub cursor: u64,
    pub revision: u64,
    pub catalog_revision: u64,
    pub order: Vec<String>,
    pub runs: BTreeMap<String, SkillRun>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionProjectionSource {
    LocalCheckpoint,
}

/// Stable, platform-neutral read model for one persisted Agent session.
///
/// This is derived from the canonical Core checkpoint in Nola Store. Clients
/// must not reconstruct these projections from Host traces, CLI output, or UI
/// archives. `store_revision` is the Store CAS revision; `core_revision` is
/// the reducer revision contained in the checkpoint.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionProjection {
    pub conversation_id: String,
    pub source: SessionProjectionSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_host_id: Option<String>,
    pub store_revision: i64,
    pub core_revision: u64,
    pub event_cursor: u64,
    pub capability_catalog_revision: u64,
    pub checkpoint_created_at_ms: u64,
    pub conversation_projection: ConversationProjection,
    pub task_projection: TaskProjection,
    pub goal_projection: GoalProjection,
    pub memory_projection: MemoryProjection,
    pub memory_runtime_projection: MemoryRuntimeProjection,
    pub skill_runtime_projection: SessionSkillProjection,
    pub permission_projection: PermissionProjection,
    pub permission_rule_set: PermissionRuleSet,
    pub context_usage: ContextUsageSnapshot,
}

/// Loads the authoritative UI/query projection without involving a Host.
/// Hosts execute effects; they are not a persistence or session-query API.
pub fn load_session_projection<R: SessionProjectionRepository + nola_store_port::run_effect::RunEffectRepository + ?Sized>(
    store: &R,
    conversation_id: &str,
) -> Result<Option<SessionProjection>, String> {
    match store
        .load_authoritative_session(conversation_id)
        .map_err(|error| format!("load authoritative session failed: {error}"))?
    {
        Some(AuthoritativeSessionRecord::Local(record)) => {
            if record.recovery.checkpoint.conversation_id != conversation_id {
                return Err("canonical checkpoint repository key mismatch".into());
            }
            // The repository has established local presentation ownership;
            // share the exact pinned materialization/cache with checkpoint reads.
            let stored = crate::load_current_checkpoint(store, conversation_id)?
                .ok_or("authoritative local checkpoint disappeared")?;
            decode_local_session_projection(&stored).map(Some)
        }
        None => Ok(None),
    }
}

pub(crate) fn materialize_local_recovery(
    record: nola_store_port::remote_session::StoredLocalSessionRecovery,
) -> Result<nola_store_port::run_effect::StoredCheckpoint, String> {
    if record.recovery.replay_inputs.is_empty() {
        return Ok(record.recovery.checkpoint);
    }
    use sha2::{Digest, Sha256};
    let base = record.recovery.replay_base.as_ref().ok_or("session replay has no Run owner")?;
    let invocation = record.invocation.as_ref().ok_or("session replay has no invocation configuration")?;
    if invocation.run_id != base.run_id
        || format!("{:x}", Sha256::digest(invocation.invocation_config_json.as_bytes()))
            != invocation.invocation_config_digest {
        return Err("session replay invocation identity or integrity mismatch".into());
    }
    let config = serde_json::from_str(&invocation.invocation_config_json)
        .map_err(|error| error.to_string())?;
    materialize_local_recovery_with_config(record, config)
}

/// Internal continuation of a verified read. The caller must have validated
/// the frozen bytes and their equality with this record's invocation. Do not
/// use this path for an independently loaded or client-supplied configuration.
pub(crate) fn materialize_local_recovery_with_config(
    mut record: nola_store_port::remote_session::StoredLocalSessionRecovery,
    mut config: nola_agent_engine::agent::protocol::AgentCoreConfig,
) -> Result<nola_store_port::run_effect::StoredCheckpoint, String> {
    if !record.recovery.replay_inputs.is_empty() {
        let base = record.recovery.replay_base.as_ref().ok_or("session replay has no Run owner")?;
        let invocation = record.invocation.as_ref().ok_or("session replay has no invocation configuration")?;
        if invocation.run_id != base.run_id {
            return Err("session replay invocation identity or integrity mismatch".into());
        }
        let checkpoint = serde_json::from_str::<CoreCheckpoint<AgentThreadState>>(
            &record.recovery.checkpoint.checkpoint_json,
        ).map_err(|error| error.to_string())?;
        crate::delegated_invocation::bind_restore_configuration(
            &mut config, &checkpoint, &base.run_id, &record.delegated_invocations,
        )?;
        let reason = checkpoint.reason;
        let core = nola_agent_engine::agent::AgentCore::restore(config, checkpoint)
            .map_err(|error| error.to_string())?;
        record.recovery.checkpoint.checkpoint_json =
            crate::materialize_replay_checkpoint_json(core, &record.recovery, reason)?;
    }
    Ok(record.recovery.checkpoint)
}

fn decode_local_session_projection(
    stored: &nola_store_port::run_effect::StoredCheckpoint,
) -> Result<SessionProjection, String> {
    let conversation_id = stored.conversation_id.as_str();
    let checkpoint =
        serde_json::from_str::<CoreCheckpoint<AgentThreadState>>(&stored.checkpoint_json)
            .map_err(|error| format!("decode canonical Core checkpoint failed: {error}"))?;
    if crate::assistant_owner::validate_checkpoint_identity(
        conversation_id,
        &stored.conversation_id,
        &checkpoint.session_id,
        checkpoint.state.session_id.as_str(),
    )
    .is_err()
    {
        return Err(format!(
            "canonical checkpoint identity mismatch: requested={conversation_id}, checkpoint={}, state={}",
            checkpoint.session_id,
            checkpoint.state.session_id.as_str()
        ));
    }
    if stored.event_cursor < 0
        || checkpoint.last_event_cursor.sequence != stored.event_cursor as u64
    {
        return Err(format!(
            "canonical checkpoint cursor mismatch: stored={}, checkpoint={}",
            stored.event_cursor, checkpoint.last_event_cursor.sequence
        ));
    }

    let state = checkpoint.state;
    for (task_id, task) in &state.task_projection.tasks {
        if task_id != &task.id || task.owner_session_id.as_str() != checkpoint.session_id {
            return Err(format!("canonical Task identity mismatch: {task_id}"));
        }
    }
    for (run_id, run) in &state.skill_runtime.runs {
        if run_id != &run.run_id || run.owner.session_id != checkpoint.session_id {
            return Err(format!("canonical Skill run identity mismatch: {run_id}"));
        }
    }
    let skill_order = state.skill_runtime.runs.keys().cloned().collect();
    let skill_runtime_projection = SessionSkillProjection {
        session_id: checkpoint.session_id.clone(),
        cursor: checkpoint.last_event_cursor.sequence,
        revision: state.skill_runtime.revision,
        catalog_revision: state.skill_runtime.catalog.revision,
        order: skill_order,
        runs: state.skill_runtime.runs,
    };
    Ok(SessionProjection {
        conversation_id: stored.conversation_id.clone(),
        source: SessionProjectionSource::LocalCheckpoint,
        source_host_id: None,
        store_revision: stored.revision,
        core_revision: checkpoint.revision,
        event_cursor: checkpoint.last_event_cursor.sequence,
        capability_catalog_revision: checkpoint.capability_catalog_revision,
        checkpoint_created_at_ms: checkpoint.created_at_ms,
        conversation_projection: state.conversation.projection,
        task_projection: state.task_projection,
        goal_projection: state.goal_projection,
        memory_projection: state.memory_projection,
        memory_runtime_projection: state.memory_runtime,
        skill_runtime_projection,
        permission_projection: state.execution.permissions,
        permission_rule_set: state.execution.permission_rules,
        context_usage: state.conversation.context_window.usage_snapshot(),
    })
}
