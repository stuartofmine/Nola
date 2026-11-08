// SPDX-License-Identifier: Apache-2.0
pub mod capability;
// File Overview:
// - Purpose: Owns the platform-neutral, effect-driven Agent Core v2.
// - Boundary: The core decides state transitions; hosts perform all I/O.

pub mod capability_projection;
pub mod code_mode;
pub mod generative_ui;
pub(crate) mod command;
pub mod compaction_planner;
pub mod context_normalizer;
pub mod context_window;
mod effect;
pub mod goal;
pub mod host;
mod host_execution_state;
pub use host_execution_state::{HostExecutionState, HostPendingEffect};
mod input_queue;
pub mod instruction_hierarchy;
mod kernel;
pub mod loop_guard;
pub mod memory_runtime;
pub mod memory_worker;
pub mod model_capability;
pub mod model_tool_batch;
pub mod parts;
pub mod permissions;
pub mod policy;
pub mod prompt_composer;
pub mod protocol;
pub mod recovery;
// Public agent requests use the Domain contract; the engine owns no copy.
pub use nola_domain::generation as reasoning;
pub mod resource_access;
mod runtime;
mod runtime_event;
mod session;
pub mod sop;
pub mod task;
pub mod tool_lifecycle;
pub mod tool_scheduler;
mod turn;

pub use crate::context_engine::{
    prepare_context_components, ContextComponent, ContextComponentMetrics,
    ContextComponentPreparation, ContextComponentSource, ContextPlatform,
};
pub use crate::memory::{
    select_memories, MemoryKind, MemoryNamespace, MemoryPolicy, MemoryProjection, MemoryProvenance,
    MemoryProvenanceKind, MemoryQualityAssessment, MemoryRecallMode, MemoryRecord, MemoryRetention,
    MemoryScope, MemorySelectionInput, MemorySelectionPlan, MemorySelectionReason,
    MemorySelectionRejectReason, MemorySourceRole, MemoryTemporalMetadata, MemoryTokenAccounting,
    MemoryUpdate, RejectedMemory, SelectedMemory,
};
pub use capability_projection::{
    AgentModelToolProjection, AgentTurnCapability, AgentTurnCapabilityProjection,
    AgentTurnDisabledCapability, TURN_CAPABILITY_PROJECTION_SCHEMA_VERSION,
};
pub use compaction_planner::{compaction_instructions, plan_compaction, CompactionPlan};
pub use context_window::{CompactionReason, ContextWindowConfig, ContextWindowState, UsageOrigin};
pub use goal::{
    GoalArtifact, GoalAuditFinding, GoalAuditReport, GoalAuditVerdict, GoalContractStatus,
    GoalEvidence, GoalExecutionReport, GoalFact, GoalProjection, GoalRecordKind, GoalRecordStatus,
    GoalRequirement, GoalRuntimeCommand, GoalRuntimeCommandEnvelope, GoalState, GoalStatus,
    GoalStopReason, GoalSubtaskContract, GoalCompletionClaim, GoalWorkEntry, GoalWorkKind, GoalWorkOrigin,
    GoalWorkProgress, GoalWorkState,
};
pub use host::ToolProgressChannel;
pub use input_queue::{AgentInputQueue, InputAdmission, InputApplication};
pub use instruction_hierarchy::{
    select_active_instructions, AgentInstructionIdentity, AgentInstructionKind,
    AgentInstructionManifest, AgentInstructionSelection, AgentInstructionSource,
    InstructionSelectionError,
};
pub use kernel::AgentCore;
pub use memory_runtime::{
    ExtractedMemory, MemoryCitation, MemoryConsolidationOutput, MemoryExtractionOutput,
    MemoryJobHandle, MemoryJobKind, MemoryJobOutput, MemoryJobState, MemoryRuntimeApplyResult,
    MemoryRuntimeCommand, MemoryRuntimeCommandEnvelope, MemoryRuntimeEvent,
    MemoryRuntimeProjection, MemorySourceSnapshot, MemoryUsageLedgerEntry, MemoryUsageStats,
};
pub use memory_worker::{
    plan_memory_worker, plan_memory_worker_from_state, MemoryWorkerPlan, MemoryWorkerPlanEnvelope,
    MemoryWorkerPlanInput, MemoryWorkerTranscriptItem, MEMORY_WORKER_PLAN_SCHEMA_VERSION,
};
pub use model_capability::{
    model_instruction_prefix_digest, model_instruction_profile_fragment,
    resolve_model_capability_profile, resolve_model_instruction_profile, ModelCapabilityProfile,
    ModelInstructionProfile, ModelInvocationPlan,
};
pub use permissions::{
    PendingPermissionRequest, PermissionContinuation, PermissionEvaluation,
    PermissionEvaluationReason, PermissionPolicyDecision, PermissionProjection, PermissionRule,
    PermissionRuleSet, PermissionRuleSource, PermissionSubject, PermissionUserDecision,
    PermissionUserIntervention,
};
pub use policy::{
    capability_for_tool_name, context_output_reservation_tokens,
    default_compaction_threshold_for_window, permission_name_for_tool,
    prompt_token_limit_for_window, reserved_output_tokens_for_window, tool_permission_domain,
    HostCapability, ABSOLUTE_MODEL_OUTPUT_TOKEN_CEILING, DEFAULT_MODEL_OUTPUT_TOKENS,
    SAFE_MODEL_OUTPUT_TOKEN_CEILING,
};
pub use prompt_composer::{
    builtin_agent_policy_fragments, compose_agent_prompt, compose_agent_prompt_extension,
    compose_host_agent_prompt, compose_host_agent_prompt_for_tier, compose_host_agent_prompt_extension, AgentPromptComposition,
    AgentPromptFragment, AgentPromptLayer, AgentPromptTrust, PromptCompositionError,
};
pub use protocol::{
    AgentControlSnapshot, AgentCoreConfig, AgentCoreSnapshot, AgentEvent, AgentInputDelivery,
    AgentInputOrigin, AgentItem, AgentItemDeltaKind, AgentPromptCompositionIdentity,
    AgentReduction, AgentRole, AgentSessionConfigurationPatch, AgentStepContextSnapshot,
    AgentSubmission, AgentTransition, AgentTurnArtifactContract, AgentTurnContextSnapshot,
    AgentTurnIdentity, AgentWorkspaceIdentity, CallId, ItemId, ModelProviderContinuationState,
    ModelRequest, PermissionRequestId, ProviderContinuationCompactionPolicy, SessionId, TokenUsage,
    ToolInvocationContext, TurnId, TURN_ARTIFACT_CONTRACT_COMPONENT_ID,
};
pub use resource_access::{
    evaluate_resource_access, AgentResourceAccessDecision, AgentResourceAccessGrant,
    AgentResourceAccessReason, AgentResourceAccessRequest, AgentResourceGrantRejection,
    AgentResourceGrantRejectionReason, AgentResourceIdentity, AgentResourceKind,
    AgentResourcePermission, AgentResourcePrincipal, AgentResourcePrincipalKind,
    RESOURCE_ACCESS_ABI_VERSION, RESOURCE_ACCESS_SCHEMA_VERSION,
};
pub use session::{
    AgentConversationState, AgentExecutionState, AgentHostState, AgentInvokedSkill,
    AgentItemOwnership, AgentJournalState, AgentSessionConfiguration, AgentSessionLineage,
    AgentSkillActivation, AgentThreadState,
};
pub use sop::{
    SopArtifactContract, SopCapabilityKind, SopCapabilityRequirement, SopCompensationAction,
    SopCompensationContract, SopCompensationRunRecord, SopCompensationRunStatus,
    SopCompletionContract, SopCondition, SopConditionDecision, SopDefinition, SopExecutionMode,
    SopRetryPolicy, SopRunEvent, SopRunRecord, SopRunStatus, SopRunTransitionError, SopStep,
    SopStepAttemptRecord, SopStepBudget, SopStepRunRecord, SopStepRunStatus, SopValidationError,
    SopVerificationContract, SopVerifierContract, SopVerifierKind, SopVerifierResult,
    SopWaitDisposition, SopWaitKind, SopWaitRecord, SOP_SCHEMA_VERSION,
};
pub use task::{
    TaskDurability, TaskGuidanceMessage, TaskGuidanceQueue, TaskHandle, TaskKind, TaskLineage,
    TaskProjection, TaskState, MAX_TASK_GUIDANCE_CHARS, MAX_TASK_GUIDANCE_QUEUE,
};
pub use tool_lifecycle::{ToolCallState, ToolCallStatus};
pub use tool_scheduler::{
    plan_next_tool_batch, ToolBatchPlan, ToolExecutionRuntime, ToolInterruptBehavior,
};
pub use turn::{
    AgentEngineeringLoopState, AgentEngineeringPhase, AgentTurnContext, AgentTurnState,
    AgentTurnStatus,
};

/// Canonical admission digest also used by trusted fresh-resend attestation.
pub use input_queue::input_digest as accepted_input_payload_digest;
