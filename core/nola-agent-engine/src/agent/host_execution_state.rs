// SPDX-License-Identifier: Apache-2.0
//! Selective live execution inputs; never constructs a recovery projection.
use super::{AgentCoreConfig, AgentThreadState};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostExecutionState {
    pub control: super::protocol::AgentControlSnapshot,
    pub configuration: AgentCoreConfig,
    pub permissions: super::permissions::PermissionProjection,
    pub pending_effects: Vec<HostPendingEffect>,
    pub pending_guidance_ids: BTreeMap<String, Vec<String>>,
    pub permits_new_turn_configuration: bool,
    pub has_active_goal: bool,
    pub has_running_goal: bool,
    pub cumulative_model_tokens: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostPendingEffect {
    pub effect_id: String,
    pub capability_id: String,
    /// Cancellation owns the original delivery; recovery must not restart it.
    pub recovery_rebind_allowed: bool,
}

impl HostExecutionState {
    pub(crate) fn from_state(state: &AgentThreadState, control: super::protocol::AgentControlSnapshot) -> Self {
        let mut view = Self {
            control,
            configuration: AgentCoreConfig {
                automation_execution_spec: None,
                session_id: Some(state.session_id.clone()),
                model: state.configuration.model.clone(),
                max_model_iterations: state.configuration.max_model_iterations,
                max_strategy_recovery_attempts: state.configuration.max_strategy_recovery_attempts,
                initial_items: Vec::new(),
                tools: state.configuration.tools.clone(),
                context_window: state.configuration.context_window.clone(),
                memory_policy: state.configuration.memory_policy.clone(),
                turn_identity: state.configuration.turn_identity.clone(),
                host_catalog: Some(state.host.catalog.clone()),
                permission_rules: state.execution.permission_rules.clone(),
            },
            permissions: state.execution.permissions.clone(),
            pending_effects: state.host.pending_effects.iter().map(|effect| HostPendingEffect {
                effect_id: effect.effect_id.clone(), capability_id: effect.capability_id.clone(),
                recovery_rebind_allowed: effect.state != super::recovery::PendingHostEffectState::Cancelling,
            }).collect(),
            pending_guidance_ids: state.task_projection.guidance.iter().map(|(task, queue)|
                (task.clone(), queue.pending.iter().map(|message| message.message_id.clone()).collect()))
                .collect(),
            permits_new_turn_configuration: state.permits_new_turn_execution_configuration(),
            has_active_goal: state.goal_projection.active.is_some(),
            has_running_goal: state.goal_projection.active.as_ref()
                .is_some_and(|goal| goal.status == super::GoalStatus::Running),
            cumulative_model_tokens: u64::try_from(state.conversation.context_window.cumulative_usage.budget_token_lower_bound())
                .unwrap_or(u64::MAX),
        };
        // Match the terminal durable view without projecting/cloning the
        // Thread. Native tool descriptors are supplied at successor admission.
        if view.permits_new_turn_configuration && view.control.active_turn_id.is_some()
            && view.control.turn_status.is_terminal() {
            view.configuration.tools.clear();
            if let Some(catalog) = &mut view.configuration.host_catalog {
                catalog.capabilities.retain(|_, capability| capability.route != super::host::HostCapabilityRoute::Tool);
            }
            view.control.tool_count = 0;
        }
        view
    }
}
