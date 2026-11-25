// SPDX-License-Identifier: Apache-2.0
//! Lean Nola v2 Core Agent Harness:
//! - Immutable frozen system prefix + prefix-stable deferred tool activation
//! - 6-stage `ToolExecutionMiddleware` (Normalize -> Policy -> PreHooks -> Exec -> PostHooks -> Receipt)
//! - Workspace-trusted `.nola/settings*.json` & `.nola/hooks.json` with `--resume` persistence
//! - Two-tier compaction (Tier-1 zero-cost micro-compaction committing partial savings -> Tier-2 summary)
//! - Parallel read-only tool batches + clean loop guard + cancellation safety.

pub mod catalog;
pub mod compaction;
pub mod hooks;
pub mod loop_guard;
pub mod loop_runner;
pub mod middleware;
pub mod planning;
pub mod policy;
pub mod subagents;

#[cfg(test)]
mod tests;

pub use catalog::{ForceToolSet, ToolCatalog};
pub use compaction::{
    compact_history, compact_history_force, CompactionLoopBreaker, CompactionReport,
    CompactionSummaryMetadata,
};
pub use hooks::HookRunner;
pub use loop_guard::{
    investigation_identity, semantic_observation_arguments, InvestigationIdentity, LoopGuard,
    LoopGuardVerdict,
};
pub use loop_runner::{
    compact_session_now, load_session_usage, validate_json_output, workspace_memory_namespace,
    AgentEngine, AgentError, AgentEvent, AgentRunConfig, AgentRunResult, HttpModelCaller,
    LlmClientModelCaller, ModelCaller, ModifiedFileRecord, VerificationCheckRecord,
    VerificationSummary,
};
pub use middleware::{ApprovalCallback, MiddlewareOutcome, ToolExecutionMiddleware};
pub use planning::{
    approve_agent_plan, is_plan_mode, load_session_plan, load_session_todos,
    register_planning_tools, set_plan_mode, PlanningState, TodoItem, UserInputCallback,
};
pub use policy::{
    is_external_send, is_sensitive_path, load_active_permission_grants, persist_permission_grant,
    DurablePermissionGrant, EffectivePolicy,
};
pub use subagents::{
    register_subagent_tools, SubagentManager, SubagentSpawnOptions, SubagentTaskSnapshot,
    SubagentToolFactory, TaskStatus, WaitMode,
};
