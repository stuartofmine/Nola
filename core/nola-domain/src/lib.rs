// SPDX-License-Identifier: Apache-2.0
//! Stable, platform-neutral product contracts.
//!
//! Persistence rows and transaction execution belong to Store. Canonical content
//! changes, identities and projections are shared business contracts owned here.

pub mod part_state;
pub use part_state::{StepStatus, ToolPartState};

pub mod delegation;
pub mod task_execution;
pub mod task_workspace;
pub mod model_runtime;
pub mod generation_override;
pub mod generative_ui;
pub mod generative_ui_tool;
pub mod execution_policy;
pub mod run_generation_intent;
pub mod requirement_control;
pub mod permission_settlement;
pub mod host_effect;
pub mod cancellation_not_started;
pub mod execution_evidence;
pub mod tool_evidence;
pub mod tool_source;
pub mod host_control;
pub mod agent_task_admission;
pub mod remote_submission;
pub mod plugin;
pub mod language_service;
pub mod code_mode;
pub mod persistent_agent;
pub mod model_connection;
pub mod agent_room;

mod turn_transition;
pub use turn_transition::*;

mod context_accounting;
pub use context_accounting::*;
mod prompt_cache_diagnostic;
pub use prompt_cache_diagnostic::*;

mod compaction_request;
pub use compaction_request::*;

pub mod projection;
pub mod memory_owner;
pub mod public_part_semantics;
pub use public_part_semantics::*;
pub mod transcript_export;
pub use projection::*;
mod run_recovery_inspection;
pub use run_recovery_inspection::*;
pub mod canonical_text;
pub mod canonical_history;
pub mod execution_constraints;
pub mod canonical_stream;
pub use canonical_stream::*;
pub mod assistant_journal;
pub mod assistant_run_terminal;
pub mod run_message_admission;
pub mod assistant_journal_operations;
pub mod assistant_journal_content;
mod assistant_turn_binding;
pub use assistant_turn_binding::*;
pub use canonical_text::*;
pub mod conversation_metadata;
pub use conversation_metadata::*;
pub mod content_mutation;
pub use content_mutation::*;

pub mod canonical_branch;
pub mod edited_resend;
pub use canonical_branch::*;

pub mod archive_restore;
pub use archive_restore::*;
pub mod part_semantics;
pub use part_semantics::*;

pub mod part_derivation;
pub use part_derivation::*;

pub mod historical_part_projection;
pub use historical_part_projection::*;

mod tool_batch;
pub use tool_batch::*;

mod permission_policy;
pub use permission_policy::*;

pub mod compaction_plan;

pub mod compaction_lifecycle;
pub use compaction_lifecycle::*;

pub mod context_budget;
pub use context_budget::*;

pub mod model_response_policy;
pub mod generation;
pub mod reasoning_tiers;
pub mod model_capabilities;

pub mod item_token_estimate;

pub mod model_completion;
pub use model_completion::*;

pub mod run_result;
pub mod run_requirements;
pub mod core_requirements;
pub mod run_input_commitment;

pub mod run_recovery;
pub use run_recovery::*;

pub mod model_attempt;
pub mod auxiliary_invocation;
pub use auxiliary_invocation::*;
pub use model_attempt::*;

pub mod delegated_invocation;
pub use delegated_invocation::*;
pub mod delegated_permission;
pub use delegated_permission::*;
pub mod effect_delivery_attempt;
pub mod unknown_effect_recovery;
pub use effect_delivery_attempt::*;
pub mod permission_instruction;
pub mod permission_closure;
pub mod permission_scope;
pub mod permission_resource_plan;
pub mod committed_run_context_usage;
pub mod runtime_task_scheduling;
pub mod runtime_task_shell_delivery;
pub mod terminal_input;
pub use permission_instruction::*;
pub mod user_input_wait;
pub mod core_event;
pub mod automation_execution;
pub mod conversation_configuration;
pub mod configuration_patch;
pub mod execution_backend;
pub mod data_source;
pub mod memory_projection;

pub mod compaction_maintenance;

pub mod goal_execution;

pub mod host_tool_plan;

pub mod expert_message;

pub mod edited_resend_context;
