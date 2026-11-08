// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Defines transient signals used only while AgentRuntime reduces a submission.
// - Boundary: RuntimeEvent never crosses FFI, persistence, Trace, or Host transports.

use serde_json::Value;

use super::capability::SkillRuntimeEvent;
use super::context_window::CompactionReason;
use super::host::{HostArtifact, HostToolOutcome};
use super::parts::{AgentPart, PartUpdate};
use super::protocol::{
    AgentEvent, AgentInputDelivery, AgentItem, AgentItemDeltaKind, CallId, ItemId,
    PermissionRequestId, SessionId, TurnId,
};
use super::task::TaskHandle;
use super::turn::AgentEngineeringLoopState;
use crate::memory::MemoryUpdate;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ItemDiscardDisposition {
    Hidden,
    Process,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PermissionResolutionKind {
    AllowOnce,
    AllowConversation,
    ConversationRuleApplied,
    PolicyApplied,
    Deny,
    ProvideInstruction,
}

impl PermissionResolutionKind {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::AllowOnce => "allow_once",
            Self::AllowConversation => "allow_conversation",
            Self::ConversationRuleApplied => "conversation_rule_applied",
            Self::PolicyApplied => "policy_applied",
            Self::Deny => "deny",
            Self::ProvideInstruction => "provide_instruction",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) enum RuntimeEvent {
    PermissionSettled { receipt: super::permissions::PermissionReceipt },
    RuntimeDecisionFactObserved { turn_id: TurnId, fact: nola_domain::core_requirements::CoreRuntimeDecisionFact },
    RequirementScopeChanged { turn_id: TurnId, scope: nola_domain::core_requirements::CoreRequirementScope },
    RequirementAssessed { turn_id: TurnId, assessment: nola_domain::core_requirements::CoreRequirementAssessment },
    RunResultDecided { turn_id: TurnId, decision: nola_domain::core_requirements::CoreRunResultDecision },
    UserInputWaitChanged { wait: nola_domain::user_input_wait::UserInputWait },
    TextReviewChanged {
        turn_id: TurnId,
        item_id: ItemId,
        state: super::parts::TextReviewState,
    },
    SessionStarted {
        session_id: SessionId,
    },
    TurnStarted {
        session_id: SessionId,
        turn_id: TurnId,
        assistant_message_id: String,
    },
    TurnContextCaptured {
        turn_id: TurnId,
        model: Option<String>,
        tool_count: usize,
        stable_prefix_digest: String,
        prompt_fragment_ids: Vec<String>,
        context_item_count: usize,
        context_component_count: usize,
        context_payload_digest: String,
        context_window_tokens: usize,
        prompt_token_limit: usize,
        host_id: String,
        host_catalog_revision: u64,
        host_availability_epoch: u64,
        capability_projection: super::capability_projection::AgentTurnCapabilityProjection,
        memory_usage: Vec<crate::memory::MemoryUsage>,
    },
    /// Applied input starts a new presentation message, not a new Run/Turn.
    AssistantSegmentStarted { turn_id: TurnId, assistant_message_id: String, applications: Vec<super::InputApplication> },
    /// Immutable presentation owner of a newly declared physical effect.
    AssistantActivityBound { turn_id: TurnId, assistant_message_id: String, activity_id: String, activity_kind: String },
    AssistantEffectBound { turn_id: TurnId, assistant_message_id: String, effect_id: String },
    InputAccepted { admission: super::InputAdmission },
    InputWithdrawn { turn_id: TurnId, admission_ids: Vec<String>, reason: String },
    InputQueued {
        turn_id: TurnId,
        delivery: AgentInputDelivery,
        item_count: usize,
        items: Vec<AgentItem>,
    },
    InputApplied {
        turn_id: TurnId,
        applications: Vec<super::InputApplication>,
        items: Vec<AgentItem>,
    },
    PartUpdated {
        turn_id: TurnId,
        projection_revision: u64,
        update: PartUpdate,
        part: AgentPart,
    },
    ItemStarted {
        turn_id: TurnId,
        item_id: ItemId,
        kind: AgentItemDeltaKind,
    },
    ItemDelta {
        turn_id: TurnId,
        item_id: ItemId,
        kind: AgentItemDeltaKind,
        delta: String,
        presentation_committed: bool,
    },
    ItemDiscarded {
        turn_id: TurnId,
        item_id: ItemId,
        disposition: ItemDiscardDisposition,
    },
    ItemCompleted {
        turn_id: TurnId,
        item: AgentItem,
    },
    ToolInputDelta {
        turn_id: TurnId,
        part_id: String,
        call_id: CallId,
        capability_id: String,
        arguments_delta: String,
    },
    PermissionRequested {
        turn_id: TurnId,
        request_id: PermissionRequestId,
        call_id: CallId,
        tool_name: String,
        arguments: Value,
        permission_domain: String,
        rationale: String,
    },
    PermissionResolved {
        turn_id: TurnId,
        request_id: PermissionRequestId,
        call_id: CallId,
        decision: PermissionResolutionKind,
    },
    ToolStarted {
        turn_id: TurnId,
        call_id: CallId,
        tool_name: String,
        arguments: Value,
    },
    ToolCallProgress {
        turn_id: TurnId,
        call_id: CallId,
        title: Option<String>,
        detail: Option<String>,
        delta: Option<String>,
    },
    ToolCompleted {
        turn_id: TurnId,
        call_id: CallId,
        tool_name: String,
        output: Value,
        outcome: HostToolOutcome,
        full_output_ref: Option<String>,
        artifacts: Vec<HostArtifact>,
    },
    CapabilitiesActivated {
        turn_id: TurnId,
        source: String,
        capability_ids: Vec<String>,
    },
    EngineeringLoopUpdated {
        turn_id: TurnId,
        state: AgentEngineeringLoopState,
    },
    ExecutionEvidenceObserved {
        turn_id: TurnId,
        evidence: nola_domain::execution_evidence::ExecutionEvidenceObservation,
    },
    HostProgress {
        turn_id: Option<TurnId>,
        effect_id: String,
        title: String,
        detail: Option<String>,
        metadata: serde_json::Value,
    },
    LifecycleHookUpdated {
        turn_id: Option<TurnId>,
        projection: crate::lifecycle_hooks::LifecycleHookExecutionProjection,
    },
    TaskUpdated {
        task_projection_revision: u64,
        task: TaskHandle,
    },
    GoalUpdated {
        goal_projection_revision: u64,
        goal: super::goal::GoalProjection,
    },
    MemoryUpdated {
        memory_projection_revision: u64,
        update: MemoryUpdate,
    },
    SkillRuntimeUpdated {
        turn_id: Option<TurnId>,
        skill_runtime_revision: u64,
        update: SkillRuntimeEvent,
    },
    HostCatalogUpdated {
        host_id: String,
        revision: u64,
        availability_epoch: u64,
    },
    HostLeaseRenewed {
        host_id: String,
        availability_epoch: u64,
        lease_id: String,
        observed_at_ms: u64,
        expires_at_ms: u64,
    },
    SessionConfigurationUpdated {
        model: Option<String>,
        tool_count: usize,
        context_window_tokens: usize,
        prompt_token_limit: usize,
    },
    HostDisconnected {
        host_id: String,
        availability_epoch: u64,
        reason: String,
    },
    ContextUpdated {
        measurement: Option<nola_domain::ContextUsageMeasurement>,
        prompt_cache_measurement: Option<nola_domain::ContextUsageMeasurement>,
        used_tokens: usize,
        context_window_tokens: usize,
        usage_percent: f64,
        usage_is_estimated: bool,
        context_usage: super::context_window::ContextUsageSnapshot,
        prompt_cache_diagnostic: Option<crate::prompt_cache::PromptCacheDiagnostic>,
    },
    CompactionRequested {
        reason: CompactionReason,
        item_count: usize,
        estimated_tokens: usize,
        source_fingerprint: String,
        source_item_count: usize,
        original_token_count: usize,
    },
    CompactionCompleted {
        summary: String,
        replaced_item_count: usize,
        remaining_item_count: usize,
        reason: Option<CompactionReason>,
        compaction_item_id: Option<ItemId>,
        source_fingerprint: Option<String>,
        source_item_count: usize,
        original_token_count: usize,
    },
    CompactionFailed {
        error: String,
    },
    TurnCompleted {
        session_id: SessionId,
        turn_id: TurnId,
        outcome: Option<nola_domain::TaskCompletionOutcome>,
    },
    TurnCancelled {
        session_id: SessionId,
        turn_id: TurnId,
        reason: String,
    },
    TurnFailed {
        session_id: SessionId,
        turn_id: TurnId,
        error: String,
    },
}

impl RuntimeEvent {
    pub(crate) fn into_public(self) -> Option<AgentEvent> {
        Some(match self {
            Self::PermissionSettled { receipt } => AgentEvent::PermissionSettled { receipt },
            Self::RuntimeDecisionFactObserved { turn_id, fact } => AgentEvent::RuntimeDecisionFactObserved { turn_id, fact },
            Self::RequirementScopeChanged { turn_id, scope } => AgentEvent::RequirementScopeChanged { turn_id, scope },
            Self::RequirementAssessed { turn_id, assessment } => AgentEvent::RequirementAssessed { turn_id, assessment },
            Self::RunResultDecided { turn_id, decision } => AgentEvent::RunResultDecided { turn_id, decision },
            Self::UserInputWaitChanged { wait } => AgentEvent::UserInputWaitChanged { wait },
            Self::TextReviewChanged { .. } => return None,
            Self::SessionStarted { session_id } => AgentEvent::SessionStarted { session_id },
            Self::TurnStarted {
                session_id,
                turn_id,
                assistant_message_id,
            } => AgentEvent::TurnStarted {
                session_id,
                turn_id,
                assistant_message_id,
            },
            Self::TurnContextCaptured {
                turn_id,
                model,
                tool_count,
                stable_prefix_digest,
                prompt_fragment_ids,
                context_item_count,
                context_component_count,
                context_payload_digest,
                context_window_tokens,
                prompt_token_limit,
                host_id,
                host_catalog_revision,
                host_availability_epoch,
                capability_projection,
                memory_usage,
            } => AgentEvent::TurnContextCaptured {
                turn_id,
                model,
                tool_count,
                stable_prefix_digest,
                prompt_fragment_ids,
                context_item_count,
                context_component_count,
                context_payload_digest,
                context_window_tokens,
                prompt_token_limit,
                host_id,
                host_catalog_revision,
                host_availability_epoch,
                capability_projection,
                memory_usage,
            },
            Self::AssistantSegmentStarted { turn_id, assistant_message_id, applications } => AgentEvent::AssistantSegmentStarted { turn_id, assistant_message_id, applications },
            Self::AssistantActivityBound { turn_id, assistant_message_id, activity_id, activity_kind } => AgentEvent::AssistantActivityBound { turn_id, assistant_message_id, activity_id, activity_kind },
            Self::AssistantEffectBound { turn_id, assistant_message_id, effect_id } => AgentEvent::AssistantEffectBound { turn_id, assistant_message_id, effect_id },
            Self::InputAccepted { admission } => AgentEvent::InputAccepted { admission },
            Self::InputWithdrawn { turn_id, admission_ids, reason } => AgentEvent::InputWithdrawn { turn_id, admission_ids, reason },
            Self::InputQueued {
                turn_id,
                delivery,
                item_count,
                items,
            } => AgentEvent::InputQueued {
                turn_id,
                delivery,
                item_count,
                items,
            },
            Self::InputApplied { turn_id, applications, items } => AgentEvent::InputApplied { turn_id, applications, items },
            Self::PartUpdated {
                turn_id,
                projection_revision,
                update,
                part,
            } => AgentEvent::PartUpdated {
                turn_id,
                projection_revision,
                update,
                part,
            },
            Self::HostProgress {
                turn_id,
                effect_id,
                title,
                detail,
                metadata,
            } => AgentEvent::HostProgress {
                turn_id,
                effect_id,
                title,
                detail,
                metadata,
            },
            Self::LifecycleHookUpdated {
                turn_id,
                projection,
            } => AgentEvent::LifecycleHookUpdated {
                turn_id,
                projection,
            },
            Self::TaskUpdated {
                task_projection_revision,
                task,
            } => AgentEvent::TaskUpdated {
                task_projection_revision,
                task,
            },
            Self::GoalUpdated {
                goal_projection_revision,
                goal,
            } => AgentEvent::GoalUpdated {
                goal_projection_revision,
                goal,
            },
            Self::MemoryUpdated {
                memory_projection_revision,
                update,
            } => AgentEvent::MemoryUpdated {
                memory_projection_revision,
                update,
            },
            Self::SkillRuntimeUpdated {
                turn_id,
                skill_runtime_revision,
                update,
            } => AgentEvent::SkillRuntimeUpdated {
                turn_id,
                skill_runtime_revision,
                update,
            },
            Self::EngineeringLoopUpdated { turn_id, state } => {
                AgentEvent::EngineeringLoopUpdated { turn_id, state }
            }
            Self::ExecutionEvidenceObserved { turn_id, evidence } => {
                AgentEvent::ExecutionEvidenceObserved { turn_id, evidence }
            }
            Self::HostCatalogUpdated {
                host_id,
                revision,
                availability_epoch,
            } => AgentEvent::HostCatalogUpdated {
                host_id,
                revision,
                availability_epoch,
            },
            Self::HostLeaseRenewed {
                host_id,
                availability_epoch,
                lease_id,
                observed_at_ms,
                expires_at_ms,
            } => AgentEvent::HostLeaseRenewed {
                host_id,
                availability_epoch,
                lease_id,
                observed_at_ms,
                expires_at_ms,
            },
            Self::SessionConfigurationUpdated {
                model,
                tool_count,
                context_window_tokens,
                prompt_token_limit,
            } => AgentEvent::SessionConfigurationUpdated {
                model,
                tool_count,
                context_window_tokens,
                prompt_token_limit,
            },
            Self::HostDisconnected {
                host_id,
                availability_epoch,
                reason,
            } => AgentEvent::HostDisconnected {
                host_id,
                availability_epoch,
                reason,
            },
            Self::ContextUpdated {
                measurement,
                prompt_cache_measurement,
                used_tokens,
                context_window_tokens,
                usage_percent,
                usage_is_estimated,
                context_usage,
                prompt_cache_diagnostic,
            } => AgentEvent::ContextUpdated {
                measurement,
                prompt_cache_measurement,
                used_tokens,
                context_window_tokens,
                usage_percent,
                usage_is_estimated,
                context_usage,
                prompt_cache_diagnostic,
            },
            Self::CompactionRequested {
                reason,
                item_count,
                estimated_tokens,
                source_fingerprint,
                source_item_count,
                original_token_count,
            } => AgentEvent::CompactionRequested {
                reason,
                item_count,
                estimated_tokens,
                source_fingerprint,
                source_item_count,
                original_token_count,
            },
            Self::CompactionCompleted {
                summary,
                replaced_item_count,
                remaining_item_count,
                reason,
                compaction_item_id,
                source_fingerprint,
                source_item_count,
                original_token_count,
            } => AgentEvent::CompactionCompleted {
                summary,
                replaced_item_count,
                remaining_item_count,
                reason,
                compaction_item_id,
                source_fingerprint,
                source_item_count,
                original_token_count,
            },
            Self::CompactionFailed { error } => AgentEvent::CompactionFailed { error },
            Self::TurnCompleted {
                session_id,
                turn_id,
                outcome,
            } => AgentEvent::TurnCompleted {
                session_id,
                turn_id,
                outcome,
            },
            Self::TurnCancelled {
                session_id,
                turn_id,
                reason,
            } => AgentEvent::TurnCancelled {
                session_id,
                turn_id,
                reason,
            },
            Self::TurnFailed {
                session_id,
                turn_id,
                error,
            } => AgentEvent::TurnFailed {
                session_id,
                turn_id,
                error,
            },
            Self::CapabilitiesActivated {
                turn_id,
                source,
                capability_ids,
            } => AgentEvent::CapabilitiesActivated {
                turn_id,
                source,
                capability_ids,
            },
            Self::ItemStarted { .. }
            | Self::ItemDelta { .. }
            | Self::ItemDiscarded { .. }
            | Self::ItemCompleted { .. }
            | Self::ToolInputDelta { .. }
            | Self::PermissionRequested { .. }
            | Self::PermissionResolved { .. }
            | Self::ToolStarted { .. }
            | Self::ToolCallProgress { .. }
            | Self::ToolCompleted { .. } => return None,
        })
    }
}
