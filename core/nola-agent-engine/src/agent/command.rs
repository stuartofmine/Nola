// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Internal reducer vocabulary; never part of the platform wire protocol.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::host::{HostCapabilityCatalog, HostResponseEnvelope, HostToolOutcome};
use super::permissions::PermissionUserDecision;
use super::protocol::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum AgentCommand {
    UserDelegation { agent_type: String, prompt: String, submission_id: String, submitted_at_epoch_ms: i64 },
    ApplyPermissionPolicy { policy: nola_domain::ActivePermissionPolicy },
    ResolveUnknownEffects { decision: nola_domain::unknown_effect_recovery::UnknownEffectRecoveryDecision },
    RespondToUserInputWait { input: nola_domain::user_input_wait::RespondToUserInputWait },
    PromoteQueuedInput {
        admission_id: String,
        expected_previous_turn_id: String,
        successor_turn_id: String,
        turn_context_components: Vec<crate::context_engine::ContextComponent>,
    },
    RebaseConversation {
        items: Vec<AgentItem>,
        item_ownership: std::collections::BTreeMap<String, super::session::AgentItemOwnership>,
    },
    RevertConversation {
        through_message_id: String,
        keep_message: bool,
    },
    HostResponded {
        response: HostResponseEnvelope,
    },
    CodeModeToolRequested {
        turn_id: TurnId,
        request: nola_domain::code_mode::CodeNestedCall,
    },
    CodeModeOutput {
        turn_id: TurnId,
        effect_id: String,
        frame: nola_domain::code_mode::CodeOutputFrame,
    },
    HostProgress {
        turn_id: Option<TurnId>,
        effect_id: String,
        title: String,
        #[serde(default)]
        detail: Option<String>,
        #[serde(default)]
        metadata: Value,
    },
    LifecycleHookUpdated {
        turn_id: Option<TurnId>,
        projection: crate::lifecycle_hooks::LifecycleHookExecutionProjection,
    },
    ToolCallProgress {
        turn_id: TurnId,
        call_id: CallId,
        channel: super::host::ToolProgressChannel,
        #[serde(default)]
        title: Option<String>,
        #[serde(default)]
        detail: Option<String>,
        #[serde(default)]
        delta: Option<String>,
    },
    MemoryRuntime {
        envelope: super::memory_runtime::MemoryRuntimeCommandEnvelope,
    },
    GoalRuntime {
        envelope: super::goal::GoalRuntimeCommandEnvelope,
    },
    MemoryWorkerCompleted {
        operation_id: String,
        job_id: String,
        ownership_token: String,
        now_epoch_ms: i64,
        completion_watermark: String,
        output: Value,
    },
    UpdateHostCatalog {
        catalog: HostCapabilityCatalog,
    },
    HostHeartbeat {
        host_id: String,
        availability_epoch: u64,
        lease_id: String,
        observed_at_ms: u64,
        extend_until_ms: u64,
    },
    UpdateSessionConfiguration {
        patch: AgentSessionConfigurationPatch,
    },
    HostDisconnected {
        host_id: String,
        availability_epoch: u64,
        reason: String,
    },
    SubmitInput {
        origin: AgentInputOrigin,
        items: Vec<AgentItem>,
        /// Model-visible context frozen for the Turn but never appended to the
        /// conversation transcript.
        #[serde(default)]
        turn_context_items: Vec<AgentItem>,
        #[serde(default)]
        turn_context_platform: crate::context_engine::ContextPlatform,
        #[serde(default)]
        turn_context_components: Vec<crate::context_engine::ContextComponent>,
        #[serde(default)]
        submission_id: Option<String>,
        #[serde(default)]
        submitted_at_epoch_ms: Option<i64>,
        #[serde(default = "default_input_delivery")]
        delivery: AgentInputDelivery,
    },
    ModelDelta {
        turn_id: TurnId,
        item_id: ItemId,
        kind: AgentItemDeltaKind,
        delta: String,
        #[serde(default)]
        presentation_committed: bool,
    },
    ModelToolCallDelta {
        turn_id: TurnId,
        part_id: String,
        call_id: CallId,
        capability_id: String,
        arguments_delta: String,
    },
    ModelContinuationUpdated {
        turn_id: TurnId,
        continuation: super::protocol::ModelProviderContinuationState,
    },
    ModelStreamRestarted {
        turn_id: TurnId,
    },
    DelegatedAgentSessionStarted {
        turn_id: TurnId,
        backend_id: String,
        backend_session_id: String,
    },
    DelegatedAgentTextDelta {
        turn_id: TurnId,
        target_effect_id: String,
        item_id: ItemId,
        delta: String,
    },
    DelegatedAgentToolStarted {
        turn_id: TurnId,
        target_effect_id: String,
        call_id: CallId,
        capability_id: String,
        input: Value,
    },
    DelegatedAgentToolCompleted {
        turn_id: TurnId,
        call_id: CallId,
        capability_id: String,
        output: Value,
        outcome: HostToolOutcome,
        full_output_ref: Option<String>,
        artifacts: Vec<super::host::HostArtifact>,
    },
    DelegatedAgentPermissionRequested {
        occurrence: nola_host_protocol::DelegatedPermissionOccurrence,
        turn_id: TurnId,
        target_effect_id: String,
        request_id: PermissionRequestId,
        call_id: CallId,
        capability_id: String,
        resource: String,
        reason: String,
        input: Value,
    },
    DelegatedAgentUsage {
        turn_id: TurnId,
        used_tokens: u64,
        context_window_tokens: u64,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
    },
    ModelCompleted {
        turn_id: TurnId,
        #[serde(default)]
        items: Vec<AgentItem>,
        #[serde(default)]
        tool_calls: Vec<ModelToolCall>,
        #[serde(default)]
        usage: TokenUsage,
        #[serde(default)]
        output_limited: bool,
        /// Host-observed duration of the completed model effect. This keeps
        /// Skill elapsed usage deterministic without reading a clock in Core.
        #[serde(default)]
        effect_elapsed_millis: u64,
    },
    ModelFailed {
        turn_id: TurnId,
        error: String,
    },
    RecoverContextLimit {
        turn_id: TurnId,
        error: String,
    },
    PermissionResolved {
        request_id: PermissionRequestId,
        decision: PermissionUserDecision,
        #[serde(default)]
        grant: Option<Value>,
    },
    PermissionIntervened {
        request_id: PermissionRequestId,
        intervention: super::permissions::PermissionUserIntervention,
    },
    ToolCompleted {
        turn_id: TurnId,
        call_id: CallId,
        capability_id: String,
        /// Original accepted Host observation, before Skill/business interpretation.
        evidence: Option<nola_domain::execution_evidence::ExecutionEvidenceObservation>,
        #[serde(default)]
        output: Value,
        outcome: HostToolOutcome,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        activation: Option<super::host::HostCapabilityActivation>,
        #[serde(default)]
        full_output_ref: Option<String>,
        #[serde(default)]
        artifacts: Vec<super::host::HostArtifact>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        skill_invocation: Option<super::capability::MaterializedSkillInvocation>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        skill_runtime_update: Option<super::capability::MaterializedSkillRuntimeUpdate>,
        /// Host-observed duration of the completed tool effect.
        #[serde(default)]
        effect_elapsed_millis: u64,
    },
    TaskPromoted {
        effect_id: String,
        call_id: CallId,
        task: super::task::TaskHandle,
    },
    TaskUpdated {
        task: super::task::TaskHandle,
    },
    ReconcileTaskTerminal {
        fact: super::task::TaskTerminalReconciliation,
    },
    UnresolvedTaskGuidance,
    ReconcileTaskGuidance {
        fact: super::task::TaskGuidanceResolution,
    },
    ApplyTaskGuidanceInput {
        message_id: String,
        expected_run_id: String,
        content: String,
    },
    ObserveTaskGuidanceDelivery {
        expected_run_id: String,
    },
    UpdateTask {
        task_id: String,
        state: super::task::TaskState,
        progress: Option<f64>,
        current_step: Option<String>,
        result_ref: Option<String>,
        cancel_requested: bool,
        ownership_token: String,
        completion_watermark: Option<i64>,
    },
    RequestTaskCancellation {
        task_id: String,
    },
    CreateMemory {
        record: crate::memory::MemoryRecord,
    },
    UpdateMemory {
        record: crate::memory::MemoryRecord,
    },
    DeleteMemory {
        memory_id: String,
    },
    SkillRuntime {
        envelope: super::capability::SkillRuntimeCommandEnvelope,
    },
    RequestCompaction {
        operation_id: String,
    },
    CompactionCompleted {
        summary: String,
        #[serde(default)]
        usage: TokenUsage,
    },
    CompactionFailed {
        error: String,
    },
    HostCancellationConfirmed { fact:nola_domain::core_requirements::CoreRuntimeDecisionFactKind },
    Cancel {
        goal_control: Option<super::goal::GoalRuntimeCommandEnvelope>,
        cancellation_id: String,
        #[serde(default)]
        reason: Option<String>,
    },
    CloseSession,
    SessionClosed,
    Reset,
}
