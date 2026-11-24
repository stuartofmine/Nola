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
pub mod policy;

#[cfg(test)]
mod tests;

pub use catalog::ToolCatalog;
pub use compaction::{compact_history, CompactionReport};
pub use hooks::HookRunner;
pub use loop_guard::{semantic_observation_arguments, LoopGuard};
pub use loop_runner::{
    AgentEngine, AgentError, AgentEvent, AgentRunConfig, AgentRunResult, HttpModelCaller,
    ModelCaller,
};
pub use middleware::{ApprovalCallback, MiddlewareOutcome, ToolExecutionMiddleware};
pub use policy::EffectivePolicy;
