// SPDX-License-Identifier: Apache-2.0
//! Atomic Goal admission and its immutable, non-secret execution template.
use nola_agent_engine::agent::{
    protocol::AgentCoreConfig, AgentSubmission, GoalRuntimeCommand, GoalRuntimeCommandEnvelope,
};
use nola_domain::goal_execution::{GoalExecutionBindingIdentity, GoalModelSelection};
use nola_store_port::goal_execution::{GoalExecutionRepository, StoredGoalExecutionBinding};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoalExecutionTemplate {
    pub model_selection: GoalModelSelection,
    /// The same typed execution facts frozen for Runs. No transcript is kept;
    /// credentials remain behind the existing Host-private recovery reference.
    pub core_config: AgentCoreConfig,
}
impl GoalExecutionTemplate {
    pub fn validate(&self) -> Result<(), String> {
        self.model_selection.validate()?;
        if self
            .core_config
            .session_id
            .as_ref()
            .is_none_or(|id| id.as_str().trim().is_empty())
        {
            return Err("Goal template requires a Core session identity".into());
        }
        if !self.core_config.initial_items.is_empty()
            || self.core_config.automation_execution_spec.is_some()
        {
            return Err("Goal template cannot capture transcript or automation ownership".into());
        }
        self.core_config.turn_identity.validate_model_capability()?;
        self.core_config
            .turn_identity
            .host_invocation
            .as_ref()
            .ok_or("Goal requires a frozen Host invocation")?
            .validate()
            .map_err(str::to_string)?;
        let reference = self
            .core_config
            .turn_identity
            .host_recovery
            .as_ref()
            .ok_or("Goal requires a durable Host recovery reference")?;
        if reference.adapter.trim().is_empty()
            || reference.adapter.len() > 512
            || reference.handle.trim().is_empty()
            || reference.handle.len() > 1024
            || reference.adapter.chars().any(char::is_control)
            || reference.handle.chars().any(char::is_control)
            || reference.sha256.len() != 64
            || !reference
                .sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err("invalid Goal Host recovery reference".into());
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredGoalExecutionPayload {
    identity: GoalExecutionBindingIdentity,
    template: GoalExecutionTemplate,
    created_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartGoalExecution {
    pub identity: GoalExecutionBindingIdentity,
    pub template: GoalExecutionTemplate,
    pub envelope: GoalRuntimeCommandEnvelope,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoalExecutionBinding {
    pub identity: GoalExecutionBindingIdentity,
    pub template: GoalExecutionTemplate,
    pub binding_sha256: String,
    pub created_at_ms: i64,
}

pub fn load_goal_execution_binding<R: GoalExecutionRepository + ?Sized>(
    store: &R,
    conversation_id: &str,
    goal_id: &str,
) -> Result<Option<GoalExecutionBinding>, String> {
    let Some(row) = store
        .load_goal_execution_binding(conversation_id, goal_id)
        .map_err(|e| e.to_string())?
    else {
        return Ok(None);
    };
    row.identity.validate()?;
    if row.identity.conversation_id != conversation_id
        || row.identity.goal_id != goal_id
        || format!("{:x}", Sha256::digest(row.binding_json.as_bytes())) != row.binding_sha256
    {
        return Err("Goal execution binding ownership or digest mismatch".into());
    }
    let payload: StoredGoalExecutionPayload = serde_json::from_str(&row.binding_json)
        .map_err(|_| "invalid stored Goal execution template")?;
    if payload.identity != row.identity || payload.created_at_ms != row.created_at_ms {
        return Err("Goal execution binding identity differs from its sealed payload".into());
    }
    let template = payload.template;
    template.validate()?;
    validate_source_host(&row.identity, &template)?;
    Ok(Some(GoalExecutionBinding {
        identity: row.identity,
        template,
        binding_sha256: row.binding_sha256,
        created_at_ms: row.created_at_ms,
    }))
}

pub fn start_goal_execution<R>(
    store: &mut R,
    input: StartGoalExecution,
) -> Result<crate::AcceptedIdleSubmission, String>
where
    R: GoalExecutionRepository
        + nola_store_port::memory::IdleCheckpointRepository
        + crate::repositories::AssistantAdmissionRepositories
        + ?Sized,
{
    input.identity.validate()?;
    input.template.validate()?;
    validate_source_host(&input.identity, &input.template)?;
    let now = match &input.envelope.command {
        GoalRuntimeCommand::Start {
            id, now_epoch_ms, ..
        } if id == &input.identity.goal_id
            && input.envelope.operation_id == input.identity.operation_id =>
        {
            *now_epoch_ms
        }
        _ => return Err("Goal binding admission requires the exact Start identity".into()),
    };
    let json = serde_json::to_string(&StoredGoalExecutionPayload {
        identity: input.identity.clone(),
        template: input.template.clone(),
        created_at_ms: now,
    })
    .map_err(|_| "cannot encode Goal execution template")?;
    let row = StoredGoalExecutionBinding {
        identity: input.identity.clone(),
        binding_sha256: format!("{:x}", Sha256::digest(json.as_bytes())),
        binding_json: json,
        created_at_ms: now,
    };
    crate::assistant_publication_transaction::with_publication_transaction(store, |store| {
        // Write first so a conflicting retry fails before touching the Goal;
        // any reduction failure rolls this row back with the same transaction.
        validate_model_selection(store, &input.template)?;
        store
            .insert_goal_execution_binding(&row)
            .map_err(|e| e.to_string())?;
        let accepted = crate::reduce_and_commit_idle_submission_in_transaction(
            store,
            crate::ReduceAndCommitIdleSubmission {
                conversation_id: input.identity.conversation_id.clone(),
                core_config: input.template.core_config,
                submission: AgentSubmission::GoalRuntime {
                    envelope: input.envelope,
                },
                committed_at_ms: now,
            },
        )?;
        let stored = crate::load_current_checkpoint(store, &input.identity.conversation_id)?
            .ok_or("Goal Start did not commit a checkpoint")?;
        let checkpoint: nola_agent_engine::agent::recovery::CoreCheckpoint<
            nola_agent_engine::agent::AgentThreadState,
        > = serde_json::from_str(&stored.checkpoint_json)
            .map_err(|_| "invalid Goal Start checkpoint")?;
        let projection = &checkpoint.state.goal_projection;
        if projection.active.as_ref().map(|goal| goal.id.as_str())
            != Some(input.identity.goal_id.as_str())
            || !projection
                .applied_operations
                .get(&input.identity.operation_id)
                .is_some_and(|operation| operation.rejection.is_none())
        {
            return Err("Goal Start was not accepted for the execution binding".into());
        }
        Ok(accepted)
    })
}

pub(crate) fn reject_unbound_goal_start(submission: &AgentSubmission) -> Result<(), String> {
    if matches!(submission, AgentSubmission::GoalRuntime { envelope } if matches!(envelope.command,GoalRuntimeCommand::Start { .. }))
    {
        return Err("Goal Start requires atomic execution binding admission".into());
    }
    Ok(())
}

/// Fresh Host reconstruction must retain the Goal's model and policy. Reuse the
/// Run validator's exact capability rules, including limited receipt renewal.
pub fn validate_goal_execution_authority<R: GoalExecutionRepository + ?Sized>(
    store: &R,
    conversation_id: &str,
    goal_id: &str,
    source_host_id: &str,
    expected_binding_sha256: &str,
    current: &crate::RunRecoveryAuthority,
    now_ms: u64,
) -> Result<GoalExecutionBinding, String> {
    let binding = load_goal_execution_binding(store, conversation_id, goal_id)?
        .ok_or("Goal execution binding is unavailable")?;
    if binding.identity.source_host_id != source_host_id
        || binding.binding_sha256 != expected_binding_sha256
    {
        return Err("Goal execution authority identity changed".into());
    }
    crate::run_recovery_authority::validate_facts(&binding.template.core_config, current, now_ms)?;
    Ok(binding)
}

fn validate_source_host(
    identity: &GoalExecutionBindingIdentity,
    template: &GoalExecutionTemplate,
) -> Result<(), String> {
    if template
        .core_config
        .host_catalog
        .as_ref()
        .map(|catalog| catalog.host_id.0.as_str())
        != Some(identity.source_host_id.as_str())
    {
        return Err("Goal source Host differs from its execution catalog".into());
    }
    Ok(())
}

fn validate_model_selection<R: nola_store_port::model::ModelRepositoryProvider + ?Sized>(
    store: &mut R,
    template: &GoalExecutionTemplate,
) -> Result<(), String> {
    use nola_store_port::model::ModelRepository;
    let GoalModelSelection::Profile {
        profile_id,
        revision,
    } = &template.model_selection
    else {
        return Ok(());
    };
    let profile = store
        .model_repository()
        .model_profile(profile_id)
        .map_err(|e| e.to_string())?
        .ok_or("Goal selected model profile is unavailable")?;
    if profile.revision != *revision || !profile.enabled || profile.deleted_at_ms.is_some() {
        return Err("Goal selected model profile changed, was disabled or removed".into());
    }
    let primary = &template
        .core_config
        .turn_identity
        .host_invocation
        .as_ref()
        .ok_or("Goal model binding is unavailable")?
        .primary;
    match primary {
        nola_host_protocol::HostModelRouteBinding::Http(route)
            if route.model != profile.model_name =>
        {
            return Err("Goal selected profile differs from the admitted model route".into())
        }
        nola_host_protocol::HostModelRouteBinding::Native(route) => {
            if route.profile_ref.as_ref().is_none_or(|reference| {
                reference.profile_id != *profile_id || reference.admitted_revision != *revision
            }) {
                return Err("Goal native profile differs from the admitted model route".into());
            }
        }
        _ => {}
    }
    let source: serde_json::Value =
        serde_json::from_str(&profile.source_json).map_err(|_| "invalid Goal model source")?;
    if !source.is_object()
        || source.get("endpointEnabled") == Some(&serde_json::Value::Bool(false))
        || source.get("modelEnabled") == Some(&serde_json::Value::Bool(false))
    {
        return Err("Goal model endpoint or model is disabled".into());
    }
    Ok(())
}

/// Execution admission is separate from inspection. An old binding remains
/// readable after revocation, but cannot launch another round.
pub fn prepare_goal_execution<R>(
    store: &mut R,
    conversation_id: &str,
    goal_id: &str,
) -> Result<GoalExecutionBinding, String>
where
    R: GoalExecutionRepository
        + nola_store_port::model::ModelRepositoryProvider
        + nola_store_port::run_effect::RunEffectRepository
        + ?Sized,
{
    let binding = load_goal_execution_binding(store, conversation_id, goal_id)?
        .ok_or("Goal execution binding is unavailable")?;
    let stored = crate::load_current_checkpoint(store, conversation_id)?
        .ok_or("Goal checkpoint is unavailable")?;
    let checkpoint: nola_agent_engine::agent::recovery::CoreCheckpoint<
        nola_agent_engine::agent::AgentThreadState,
    > = serde_json::from_str(&stored.checkpoint_json)
        .map_err(|_| "invalid Goal execution checkpoint")?;
    let goal = checkpoint
        .state
        .goal_projection
        .active
        .as_ref()
        .ok_or("Goal is unavailable")?;
    if goal.id != goal_id || goal.status.is_terminal() {
        return Err("Goal is no longer executable".into());
    }
    validate_model_selection(store, &binding.template)?;
    Ok(binding)
}

pub(crate) fn validate_goal_round<R>(
    store: &mut R,
    conversation_id: &str,
    current: &AgentCoreConfig,
    now_ms: i64,
) -> Result<(), String>
where
    R: GoalExecutionRepository
        + nola_store_port::model::ModelRepositoryProvider
        + nola_store_port::run_effect::RunEffectRepository
        + ?Sized,
{
    let stored = crate::load_current_checkpoint(store, conversation_id)?
        .ok_or("Goal round requires its admitted Goal")?;
    let checkpoint: nola_agent_engine::agent::recovery::CoreCheckpoint<
        nola_agent_engine::agent::AgentThreadState,
    > = serde_json::from_str(&stored.checkpoint_json)
        .map_err(|_| "invalid Goal round checkpoint")?;
    let goal = checkpoint
        .state
        .goal_projection
        .active
        .as_ref()
        .ok_or("Goal round requires its admitted Goal")?;
    let binding = prepare_goal_execution(store, conversation_id, &goal.id)?;
    let fresh = crate::RunRecoveryAuthority {
        host_catalog: current.host_catalog.clone(),
        tools: current.tools.clone(),
        permission_rules: current.permission_rules.clone(),
        context_window: current.context_window.clone(),
        host_invocation: current
            .turn_identity
            .host_invocation
            .clone()
            .ok_or("Goal round requires frozen Host identity")?,
    };
    let now = u64::try_from(now_ms).map_err(|_| "invalid Goal round time")?;
    let frozen = &binding.template.core_config;
    crate::run_recovery_authority::validate_facts(frozen, &fresh, now)?;
    let stable = |config: &AgentCoreConfig| {
        serde_json::to_value((
            &config.model,
            config.max_model_iterations,
            config.max_strategy_recovery_attempts,
            &config.memory_policy,
            &config.turn_identity.plugin_revisions,
            &config.turn_identity.workspace,
            &config.turn_identity.execution_backend,
            &config.turn_identity.model_capability,
            config.turn_identity.skill_catalog_revision,
            config.turn_identity.skill_invocation_revision,
            config.turn_identity.memory_policy_revision,
            config.turn_identity.permission_policy_revision,
        ))
        .map_err(|_| "invalid Goal execution policy")
    };
    if stable(frozen)? != stable(current)? {
        return Err("Goal execution policy differs from Start".into());
    }
    Ok(())
}
