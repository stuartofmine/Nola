// SPDX-License-Identifier: Apache-2.0
//! Autonomous Goal Execution Worker (`GoalWorker`):
//! Executes a persisted `GoalRecord` via `AgentEngine`, updates live status
//! (`Pending -> Running -> Completed/Failed`), and persists progress checkpoints.

use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use nola_engine::{
    AgentEngine, AgentRunConfig, EffectivePolicy, ForceToolSet, HookRunner, ModelCaller,
};
use nola_tools::{path_within_workspace, SandboxMode, ToolContext, ToolRegistry};
use nola_types::{
    now_ms, FrozenSystemBlock, GoalRecord, GoalStatus, HostCapabilities, ModelRouteConfig,
};

use crate::goals_sops::KIND_GOAL;
use crate::ModulesSuite;

pub struct GoalWorker;

impl GoalWorker {
    /// Execute a persisted goal end-to-end and checkpoint its final status & progress note.
    pub fn execute_goal(
        suite: &Arc<ModulesSuite>,
        caller: Arc<dyn ModelCaller>,
        route: ModelRouteConfig,
        policy: EffectivePolicy,
        goal_id: &str,
        max_steps: usize,
        cancel: Arc<AtomicBool>,
    ) -> Result<GoalRecord, String> {
        let mut goal = suite
            .get_record::<GoalRecord>(KIND_GOAL, goal_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("goal '{goal_id}' not found"))?;

        if goal.status == GoalStatus::Running {
            return Err(format!("goal '{goal_id}' is already running"));
        }

        let goal_ws = Path::new(&goal.workspace_root);
        if !path_within_workspace(&suite.workspace_root, goal_ws) {
            return Err(format!(
                "goal workspace '{}' escapes active workspace root '{}'",
                goal.workspace_root,
                suite.workspace_root.display()
            ));
        }

        goal.status = GoalStatus::Running;
        goal.updated_at_ms = now_ms();
        suite
            .upsert_record(KIND_GOAL, &goal.id, &goal)
            .map_err(|e| e.to_string())?;

        let mut registry = ToolRegistry::with_builtin_tools();
        suite.register_tools(&mut registry);
        let engine = AgentEngine::new(suite.store.clone(), Arc::new(registry), caller);

        let mut ctx = ToolContext::new(goal_ws);
        ctx.permission_profile = policy.profile;
        ctx.network_allowed = policy.network_allowed;
        ctx.sandbox_mode = SandboxMode::from_env();

        let hooks = HookRunner::load(goal_ws, suite.user_home.as_deref(), policy.workspace_trusted);
        let run_cfg = AgentRunConfig {
            session_id: Some(format!("goal_{}", goal.id)),
            resume: true,
            route,
            frozen_system_blocks: vec![FrozenSystemBlock {
                id: "goal_worker".into(),
                text: format!(
                    "You are executing an autonomous Nola Goal.\n\
                     Title: {}\n\
                     Workspace: {}\n\
                     Verify all changes with real commands/tests before declaring completion.",
                    goal.title, goal.workspace_root
                ),
            }],
            host_capabilities: HostCapabilities {
                interactive_user_input: false,
                network_allowed: policy.network_allowed,
            },
            policy,
            hooks,
            approval: None,
            max_steps,
            force_tool_set: ForceToolSet::Auto,
            stream: false,
            json_schema: None,
            cancel,
            auto_recall_memory: true,
            inbox: None,
        };

        match engine.run(&ctx, &goal.objective, run_cfg, |_| {}) {
            Ok(res) => {
                goal.status = if res.cancelled {
                    GoalStatus::Paused
                } else {
                    GoalStatus::Completed
                };
                let summary: String = res.final_text.chars().take(400).collect();
                goal.progress_notes
                    .push(format!("[steps={}] {summary}", res.steps));
                goal.updated_at_ms = now_ms();
                let _ = suite.upsert_record(KIND_GOAL, &goal.id, &goal);
                Ok(goal)
            }
            Err(e) => {
                goal.status = GoalStatus::Failed;
                goal.progress_notes.push(format!("[error] {e}"));
                goal.updated_at_ms = now_ms();
                let _ = suite.upsert_record(KIND_GOAL, &goal.id, &goal);
                Err(e.to_string())
            }
        }
    }
}
