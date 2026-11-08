// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Defines the stable command, event, effect, and snapshot contract for Agent Core v2.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

use crate::agent_trace::AgentTraceEvent;

use super::command::AgentCommand;
use super::host::{HostCapabilityCatalog, HostRequestEnvelope, HostResponseEnvelope};
use super::parts::{AgentPart, ConversationProjection, PartUpdate};
use super::permissions::{PermissionRuleSet, PermissionUserDecision};
use super::recovery::{CheckpointReason};
use nola_domain::core_event::{CoreEventEnvelope, EventCursor};

macro_rules! opaque_id {
    ($name:ident) => {
        #[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub String);

        impl $name {
            pub fn from_string(value: String) -> Self {
                Self(value)
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
    };
}

opaque_id!(SessionId);
opaque_id!(TurnId);
opaque_id!(ItemId);
opaque_id!(CallId);
opaque_id!(PermissionRequestId);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRole {
    System,
    User,
    Assistant,
    Developer,
}

/// Provider-owned state required to continue a model protocol across requests.
///
/// Core deliberately treats `state` and `integrity_kind` as opaque values. A
/// Host may interpret only namespaces it owns; unknown namespaces must survive
/// checkpoints and replay without conversion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelProviderContinuationState {
    pub provider_namespace: String,
    pub model_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_id: Option<String>,
    #[serde(default)]
    pub state: Value,
    #[serde(default)]
    pub integrity_kind: String,
    #[serde(default)]
    pub compaction_policy: ProviderContinuationCompactionPolicy,
}

impl ModelProviderContinuationState {
    pub fn is_for_model(&self, model_id: Option<&str>) -> bool {
        model_id.is_none_or(|model_id| self.model_id == model_id)
    }

    pub fn survives_compaction(&self) -> bool {
        self.compaction_policy == ProviderContinuationCompactionPolicy::Preserve
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderContinuationCompactionPolicy {
    Preserve,
    #[default]
    ClearOnCompaction,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentItem {
    UiAction {
        id: ItemId,
        action: nola_domain::generative_ui::UiActionContent,
    },
    Message {
        id: ItemId,
        role: AgentRole,
        content: String,
    },
    Attachment {
        id: ItemId,
        message_id: ItemId,
        uri: String,
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mime_type: Option<String>,
        #[serde(default)]
        byte_count: u64,
    },
    Reasoning {
        id: ItemId,
        #[serde(default)]
        summary: String,
        #[serde(default)]
        content: String,
        /// Model-protocol metadata. Conversation UI projects only summary and
        /// content and must never expose this opaque provider state.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_continuation: Option<ModelProviderContinuationState>,
    },
    ToolCall {
        id: ItemId,
        call_id: CallId,
        name: String,
        #[serde(default)]
        arguments: Value,
    },
    ToolResult {
        id: ItemId,
        call_id: CallId,
        name: String,
        #[serde(default)]
        output: Value,
        #[serde(default)]
        is_error: bool,
        #[serde(default)]
        full_output_ref: Option<String>,
        #[serde(default)]
        artifacts: Vec<super::host::HostArtifact>,
    },
    Compaction {
        id: ItemId,
        summary: String,
        replaced_item_count: usize,
        /// Stable canonical item identities summarized by this checkpoint.
        /// The lossless transcript keeps those items after compaction, so a
        /// consumer can locate the exact source bodies instead of trusting
        /// the generated summary as an untraceable replacement.
        #[serde(default)]
        source_item_ids: Vec<ItemId>,
        /// SHA-256 of the canonical serialized source item sequence.
        #[serde(default)]
        source_fingerprint: String,
        /// Estimated model tokens in the source sequence before compaction.
        #[serde(default)]
        original_token_count: usize,
    },
}

impl AgentItem {
    pub fn is_user_input(&self) -> bool {
        matches!(self, Self::UiAction { .. } | Self::Message { role: AgentRole::User, .. })
    }

    pub fn id(&self) -> &ItemId {
        match self {
            Self::UiAction { id, .. }
            | Self::Message { id, .. }
            | Self::Attachment { id, .. }
            | Self::Reasoning { id, .. }
            | Self::ToolCall { id, .. }
            | Self::ToolResult { id, .. }
            | Self::Compaction { id, .. } => id,
        }
    }

    pub fn user_message(id: ItemId, content: impl Into<String>) -> Self {
        Self::Message {
            id,
            role: AgentRole::User,
            content: content.into(),
        }
    }

    pub fn assistant_message(id: ItemId, content: impl Into<String>) -> Self {
        Self::Message {
            id,
            role: AgentRole::Assistant,
            content: content.into(),
        }
    }

    pub fn estimated_tokens(&self) -> usize {
        use nola_domain::item_token_estimate::ItemTokenEstimateFacts;
        let facts = match self {
            Self::UiAction { action, .. } => return nola_kernel::item_token_estimate::estimated_item_tokens(
                ItemTokenEstimateFacts::NamedJson { name: "ui_action", value: &action.model_envelope() }),
            Self::Message { content, .. } => ItemTokenEstimateFacts::Text(content),
            Self::Attachment {
                uri,
                name,
                mime_type,
                ..
            } => ItemTokenEstimateFacts::Attachment {
                uri,
                name,
                mime_type: mime_type.as_deref(),
            },
            Self::Reasoning {
                summary, content, ..
            } => ItemTokenEstimateFacts::Reasoning { summary, content },
            Self::ToolCall {
                name, arguments, ..
            } => ItemTokenEstimateFacts::NamedJson {
                name,
                value: arguments,
            },
            Self::ToolResult { name, output, .. } => ItemTokenEstimateFacts::NamedJson {
                name,
                value: output,
            },
            Self::Compaction { summary, .. } => ItemTokenEstimateFacts::Text(summary),
        };
        nola_kernel::item_token_estimate::estimated_item_tokens(facts)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenUsage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_usage: Option<nola_domain::PromptCacheUsage>,
    #[serde(default)]
    pub input_tokens: Option<usize>,
    #[serde(default)]
    pub cached_input_tokens: Option<usize>,
    #[serde(default)]
    pub cache_write_input_tokens: Option<usize>,
    #[serde(default)]
    pub output_tokens: Option<usize>,
    #[serde(default)]
    pub reasoning_tokens: Option<usize>,
    #[serde(default)]
    pub total_tokens: Option<usize>,
    #[serde(default)]
    pub request_count: usize,
    #[serde(default)]
    pub known_token_lower_bound: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_cache_identity: Option<crate::prompt_cache::PromptCacheIdentity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_cache_diagnostic: Option<crate::prompt_cache::PromptCacheDiagnostic>,
}

impl TokenUsage {
    pub(crate) fn accounting_facts(&self) -> nola_domain::TokenAccounting {
        nola_domain::TokenAccounting {
            input_tokens: self.input_tokens,
            cached_input_tokens: self.cached_input_tokens,
            cache_write_input_tokens: self.cache_write_input_tokens,
            output_tokens: self.output_tokens,
            reasoning_tokens: self.reasoning_tokens,
            total_tokens: self.total_tokens,
            request_count: self.request_count,
            known_token_lower_bound: self.known_token_lower_bound,
        }
    }

    pub(crate) fn apply_accounting(&mut self, facts: nola_domain::TokenAccounting) {
        self.input_tokens = facts.input_tokens;
        self.cached_input_tokens = facts.cached_input_tokens;
        self.cache_write_input_tokens = facts.cache_write_input_tokens;
        self.output_tokens = facts.output_tokens;
        self.reasoning_tokens = facts.reasoning_tokens;
        self.total_tokens = facts.total_tokens;
        self.request_count = facts.request_count;
        self.known_token_lower_bound = facts.known_token_lower_bound;
    }

    pub fn budget_token_lower_bound(&self) -> usize {
        nola_kernel::provider_usage_budget_lower_bound(&self.accounting_facts())
    }

    pub fn accumulate(&mut self, usage: &Self) {
        let mut facts = self.accounting_facts();
        nola_kernel::accumulate_token_accounting(&mut facts, usage.accounting_facts());
        self.apply_accounting(facts);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelToolCall {
    #[serde(default)]
    pub call_id: CallId,
    pub name: String,
    #[serde(default)]
    pub arguments: Value,
    #[serde(default)]
    pub permission_scope: Option<String>,
    #[serde(default)]
    pub permission_rationale: Option<String>,
}

/// Model-selected execution lineage for a concrete tool invocation.
///
/// This metadata is removed from business arguments before validation and
/// execution. Core and Application use it only to correlate the call with a
/// durable SOP step; Hosts must validate the referenced run and step rather
/// than infer lineage from tool names, text, or currently active work.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ToolInvocationContext {
    pub sop_run_id: String,
    pub sop_step_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentItemDeltaKind {
    AssistantText,
    ReasoningSummary,
    ReasoningContent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentInputDelivery {
    CurrentTurn,
    NextTurn,
}

#[path = "internal_trigger.rs"]
mod internal_trigger;
pub use internal_trigger::{internal_turn_context_digest, GoalTriggerBinding, InternalTurnTrigger};

/// Provenance of the input that created a Turn.
///
/// Only `InteractiveUser` represents fresh user intent. Stored prompts,
/// replayed transcript content, recovery instructions, and delegated work must
/// never be interpreted as live authorization for a new side effect.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentInputOrigin {
    #[default]
    Unknown,
    InteractiveUser,
    /// Fresh user intent that explicitly starts or resumes a durable Goal.
    /// This remains a visible conversation Turn and carries live user
    /// authority, unlike an automated `GoalContinuation`.
    GoalInteractiveUser,
    ScheduledAction,
    Replay,
    SystemRecovery,
    GoalContinuation,
    /// Independent read-only verification for a durable Goal. This origin may
    /// report a typed Goal verdict without inheriting live user authority or
    /// joining the executor's conversation state.
    GoalAudit,
    DelegatedAgent,
    /// Explicit session maintenance requested by the user or Host. A
    /// maintenance Turn owns durable effects but must not invoke the normal
    /// assistant model loop.
    Maintenance,
}

impl AgentInputOrigin {
    pub fn carries_live_user_authority(self) -> bool {
        matches!(self, Self::InteractiveUser | Self::GoalInteractiveUser)
    }

    /// Conversation permission rules are explicit structured user decisions,
    /// not authority inferred from prompt text. A delegated child may inherit
    /// those rules when its Host copies the parent conversation scope into the
    /// child configuration. Turn-only grants remain local to the parent Turn.
    pub fn accepts_conversation_permission_grants(self) -> bool {
        matches!(
            self,
            Self::InteractiveUser | Self::GoalInteractiveUser | Self::DelegatedAgent
        )
    }

    pub fn is_automated(self) -> bool {
        matches!(
            self,
            Self::ScheduledAction
                | Self::Replay
                | Self::SystemRecovery
                | Self::GoalContinuation
                | Self::GoalAudit
                | Self::DelegatedAgent
        )
    }
}

/// Non-sensitive workspace identity captured for a Turn.
///
/// Hosts must not put a filesystem path, bookmark, document content, or secret
/// in this structure. `root_digest` is an irreversible identity digest and
/// `summary` is a short user-facing label only.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentWorkspaceIdentity {
    #[serde(default)]
    pub workspace_id: String,
    #[serde(default)]
    pub root_digest: String,
    #[serde(default)]
    pub summary: Option<String>,
}

/// Identity of the already-composed prompt. Prompt content is deliberately
/// excluded from checkpoints and model request metadata.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentPromptCompositionIdentity {
    #[serde(default)]
    pub stable_prefix_digest: String,
    #[serde(default)]
    pub fragment_ids: Vec<String>,
    /// Stable fragment ID to typed-fragment digest. Prompt text is never
    /// persisted; this is diagnostic metadata for locating cache breaks.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub stable_fragment_digests: BTreeMap<String, String>,
}

/// Host/session metadata that is safe to freeze into the next Turn.
///
/// Values are identifiers, revisions, digests, and summaries. Model windows
/// and the active tool catalog revision are derived by Core at Turn creation.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentTurnIdentity {
    /// New-owner reconstruction integrity for an explicitly prepared resend.
    /// This is a trusted Host statement, never an independent permission grant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fresh_resend_policy: Option<nola_domain::edited_resend::FreshResendPolicyWitness>,
    /// Frozen non-secret Host dependency snapshots (such as skill instructions).
    /// Recovery uses these facts, never today's enabled catalog. No authority
    /// to execute tools or access resources is granted by this material.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_dependencies: Option<Value>,
    /// Frozen effective child profiles and non-secret execution facts. None means this Host does not offer delegation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delegation: Option<nola_host_protocol::delegation::ValidatedDelegationParent>,
    /// Authenticated remote Host availability snapshot, never an operation grant.
    pub remote_tool_plan: Option<nola_domain::host_tool_plan::HostToolPlanSnapshot>,
    /// Enabled plugin installation revisions frozen for this Run. A cold Host
    /// may recreate connections, but must not substitute a different package.
    #[serde(default)]
    pub plugin_revisions: std::collections::BTreeMap<String, i64>,
    /// Host can render durable, explicitly labelled review candidates and
    /// preserve rejected revisions. Hosts without this capability stay private.
    #[serde(default)]
    pub supports_reviewed_text_parts: bool,
    /// Immutable non-secret Host route/configuration identity. Older Runs lack
    /// this proof and cannot be cold-replayed by a Host that requires it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_invocation: Option<nola_host_protocol::HostInvocationBinding>,
    /// Host-private immutable reconstruction material. This opaque reference
    /// grants no authority and contains no credentials or platform handles.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_recovery: Option<nola_host_protocol::HostRunRecoveryReference>,
    #[serde(default)]
    pub locale: Option<String>,
    #[serde(default)]
    pub workspace: Option<AgentWorkspaceIdentity>,
    #[serde(default)]
    pub response_style_id: Option<String>,
    #[serde(default)]
    pub model_capability_id: Option<String>,
    #[serde(default)]
    pub model_capability_revision: u64,
    #[serde(default)]
    pub skill_catalog_revision: u64,
    #[serde(default)]
    pub skill_invocation_revision: u64,
    #[serde(default)]
    pub memory_policy_revision: u64,
    #[serde(default)]
    pub permission_policy_revision: u64,
    #[serde(default)]
    pub prompt: AgentPromptCompositionIdentity,
    #[serde(default)]
    pub instruction_manifest: super::instruction_hierarchy::AgentInstructionManifest,
    #[serde(default)]
    pub model_capability: super::model_capability::ModelCapabilityProfile,
    /// Stable Host executor selection for this conversation. The backend ID
    /// is a protocol identity, never a localized display label.
    #[serde(default)]
    pub execution_backend: nola_domain::execution_backend::ExecutionBackendSelection,
}

impl AgentTurnIdentity {
    /// Hosts declare execution capabilities; a model name cannot grant them.
    pub fn validate_model_capability(&self) -> Result<(), String> {
        let profile = &self.model_capability;
        if profile.profile_id.trim().is_empty()
            || profile.profile_id.trim() != profile.profile_id
            || profile.revision == 0
        {
            return Err("Agent Host must declare a non-empty model capability profile and positive revision".into());
        }
        Ok(())
    }
}

/// Immutable, non-sensitive identity snapshot used by every model round in a
/// Turn and persisted in the Core checkpoint.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentTurnContextSnapshot {
    #[serde(default)]
    pub input_origin: AgentInputOrigin,
    #[serde(default)]
    pub locale: Option<String>,
    #[serde(default)]
    pub workspace: Option<AgentWorkspaceIdentity>,
    #[serde(default)]
    pub response_style_id: Option<String>,
    #[serde(default)]
    pub model_id: Option<String>,
    #[serde(default)]
    pub execution_backend: nola_domain::execution_backend::ExecutionBackendSelection,
    #[serde(default)]
    pub model_capability_id: Option<String>,
    #[serde(default)]
    pub model_capability_revision: u64,
    #[serde(default)]
    pub model_context_window_tokens: usize,
    #[serde(default)]
    pub model_prompt_token_limit: usize,
    #[serde(default)]
    pub tool_catalog_revision: u64,
    #[serde(default)]
    pub capability_projection_digest: String,
    #[serde(default)]
    pub capability_schema_digest: String,
    #[serde(default)]
    pub enabled_capability_ids: Vec<String>,
    #[serde(default)]
    pub skill_catalog_revision: u64,
    #[serde(default)]
    pub skill_invocation_revision: u64,
    #[serde(default)]
    pub memory_policy_revision: u64,
    #[serde(default)]
    pub permission_policy_revision: u64,
    #[serde(default)]
    pub stable_prefix_digest: String,
    #[serde(default)]
    pub prompt_fragment_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub stable_prompt_fragment_digests: BTreeMap<String, String>,
    #[serde(default)]
    pub instruction_manifest: super::instruction_hierarchy::AgentInstructionManifest,
    #[serde(default)]
    pub model_capability: super::model_capability::ModelCapabilityProfile,
}

impl AgentTurnContextSnapshot {
    pub(crate) fn capture(
        identity: &AgentTurnIdentity,
        input_origin: AgentInputOrigin,
        model: Option<String>,
        context_window: &super::context_window::ContextWindowConfig,
        tool_catalog_revision: u64,
        capability_projection: &super::capability_projection::AgentTurnCapabilityProjection,
    ) -> Self {
        Self {
            input_origin,
            locale: identity.locale.clone(),
            workspace: identity.workspace.clone(),
            response_style_id: identity.response_style_id.clone(),
            model_id: model,
            execution_backend: identity.execution_backend.clone(),
            model_capability_id: identity.model_capability_id.clone(),
            model_capability_revision: identity.model_capability_revision,
            model_context_window_tokens: context_window.context_window_tokens,
            model_prompt_token_limit: context_window.prompt_token_limit(),
            tool_catalog_revision,
            capability_projection_digest: capability_projection.projection_digest.clone(),
            capability_schema_digest: capability_projection.model_schema_digest.clone(),
            enabled_capability_ids: capability_projection.enabled_ids(),
            skill_catalog_revision: identity.skill_catalog_revision,
            skill_invocation_revision: identity.skill_invocation_revision,
            memory_policy_revision: identity.memory_policy_revision,
            permission_policy_revision: identity.permission_policy_revision,
            stable_prefix_digest: identity.prompt.stable_prefix_digest.clone(),
            prompt_fragment_ids: identity.prompt.fragment_ids.clone(),
            stable_prompt_fragment_digests: identity.prompt.stable_fragment_digests.clone(),
            instruction_manifest: identity.instruction_manifest.clone(),
            model_capability: identity.model_capability.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentCoreConfig {
    /// Admission-only actual Host configuration witness, not model metadata.
    /// Ordinary interactive Runs omit it; it never grants authority itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub automation_execution_spec: Option<nola_domain::automation_execution::AutomationExecutionSpec>,
    #[serde(default)]
    pub session_id: Option<SessionId>,
    #[serde(default)]
    pub model: Option<String>,
    /// Maximum tool-capable model requests in one Turn. Zero keeps execution
    /// unbounded. Exhaustion permits one final tool-free synthesis request.
    #[serde(default)]
    pub max_model_iterations: usize,
    /// Per-Turn model requests allowed for strategy recovery. Zero disables
    /// recovery sampling; this remains bounded when ordinary iterations are not.
    #[serde(default = "default_strategy_recovery_attempts")]
    pub max_strategy_recovery_attempts: u32,
    /// Existing persisted conversation imported when creating a new Core
    /// session. Crash/process recovery must use a checkpoint instead.
    #[serde(default)]
    pub initial_items: Vec<AgentItem>,
    #[serde(default)]
    pub tools: Vec<Value>,
    #[serde(default)]
    pub context_window: super::context_window::ContextWindowConfig,
    #[serde(default)]
    pub host_catalog: Option<HostCapabilityCatalog>,
    #[serde(default)]
    pub permission_rules: PermissionRuleSet,
    #[serde(default)]
    pub memory_policy: crate::memory::MemoryPolicy,
    /// Non-sensitive identity/revision metadata frozen when each Turn starts.
    #[serde(default)]
    pub turn_identity: AgentTurnIdentity,
}

fn default_strategy_recovery_attempts() -> u32 {
    3
}

impl Default for AgentCoreConfig {
    fn default() -> Self {
        Self {
            automation_execution_spec: None,
            session_id: None,
            model: None,
            max_model_iterations: 0,
            max_strategy_recovery_attempts: default_strategy_recovery_attempts(),
            initial_items: Vec::new(),
            tools: Vec::new(),
            context_window: super::context_window::ContextWindowConfig::default(),
            host_catalog: None,
            permission_rules: PermissionRuleSet::default(),
            memory_policy: crate::memory::MemoryPolicy::default(),
            turn_identity: AgentTurnIdentity::default(),
        }
    }
}

/// Explicit, checkpointed updates to one conversation's configuration.
///
/// Hosts must submit this patch while the thread is idle. Reconstructing a
/// runtime with different settings is intentionally rejected during restore.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentSessionConfigurationPatch {
    /// Missing keeps the current model; JSON `null` selects the Host default.
    #[serde(default)]
    pub model: Option<Option<String>>,
    /// Missing keeps the current limit. Zero selects unbounded execution.
    #[serde(default)]
    pub max_model_iterations: Option<usize>,
    /// Missing keeps the current recovery allowance; zero disables it.
    #[serde(default)]
    pub max_strategy_recovery_attempts: Option<u32>,
    /// Missing keeps the current tool schema; an empty array disables tools.
    #[serde(default)]
    pub tools: Option<Vec<Value>>,
    /// Missing keeps the current context-window contract.
    #[serde(default)]
    pub context_window: Option<super::context_window::ContextWindowConfig>,
    /// Missing keeps the current per-conversation memory behavior.
    #[serde(default)]
    pub memory_policy: Option<crate::memory::MemoryPolicy>,
    /// Missing keeps the current identity metadata. Updates are accepted only
    /// while idle and therefore affect the next Turn, never the active Turn.
    #[serde(default)]
    pub turn_identity: Option<AgentTurnIdentity>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelRequest {
    pub session_id: SessionId,
    pub turn_id: TurnId,
    pub request_id: ItemId,
    pub text_part_id: ItemId,
    pub reasoning_part_id: ItemId,
    #[serde(default)]
    pub model: Option<String>,
    /// Semantic response contract for this individual sampling step. Hosts
    /// apply provider mechanics but must not infer this mode from prompt text.
    #[serde(default)]
    pub response_mode: super::host::ModelResponseMode,
    /// Frozen identity/revision metadata shared by every model round in this
    /// Turn. It contains no prompt, workspace path, memory, or Skill content.
    #[serde(default)]
    pub turn_context_snapshot: AgentTurnContextSnapshot,
    /// Exact tool subset supplied for this individual model sampling step.
    /// It is derived only from the Turn's frozen capability registry.
    #[serde(default)]
    pub tool_projection: super::capability_projection::AgentModelToolProjection,
    pub items: Vec<AgentItem>,
    /// Original user-authored message identity from canonical conversation
    /// history. User-role runtime facts and review packets are not user input.
    /// Required on replay; Hosts must not infer origin from content or IDs.
    pub original_user_message_ids: std::collections::BTreeSet<String>,
    /// Core-planned suffix for a delegated executor that has committed the
    /// matching base anchor. Hosts never derive this subset themselves.
    #[serde(default)]
    pub delegated_incremental_items: Vec<AgentItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delegated_context_base_anchor_item_id: Option<String>,
    pub tools: Vec<Value>,
    pub prompt_token_limit: usize,
    pub context_window_tokens: usize,
    /// Keep candidate answer text out of the user-visible stream until Core
    /// can evaluate the completed model transaction. Used while an engineering
    /// validation gate is still open so a rejected completion never flashes in
    /// clients before the corrective model round.
    #[serde(default)]
    pub defer_text_presentation: bool,
}

/// Immutable identity of one physical model sampling step. A Turn may contain
/// several Steps, but a Host must never rebuild or reinterpret any of these
/// values from rendered prompt text.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentStepContextSnapshot {
    pub schema_version: u32,
    pub step_index: u64,
    pub turn_context: AgentTurnContextSnapshot,
    pub tool_projection: super::capability_projection::AgentModelToolProjection,
    pub invocation_policy_digest: String,
    pub stable_prefix_digest: String,
    pub model_prefix_digest: String,
    pub request_snapshot_digest: String,
}

pub(super) fn default_input_delivery() -> AgentInputDelivery {
    AgentInputDelivery::CurrentTurn
}

/// Versioned typed context component used by external Hosts to declare
/// durable outputs that must be created and validated before a Turn can end.
/// The declaration carries no permission grant and Core never derives it from
/// user text.
pub const TURN_ARTIFACT_CONTRACT_COMPONENT_ID: &str = "nola.turn.artifact-contract.v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentTurnArtifactContract {
    pub schema_version: u32,
    pub expected_artifact_refs: Vec<String>,
}

impl AgentTurnArtifactContract {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1 {
            return Err("turn artifact contract schema_version must be 1".to_string());
        }
        if self.expected_artifact_refs.len() > 256 {
            return Err(
                "turn artifact contract cannot contain more than 256 artifacts".to_string(),
            );
        }
        let mut seen = std::collections::BTreeSet::new();
        for reference in &self.expected_artifact_refs {
            let normalized = reference.trim();
            if normalized.is_empty() {
                return Err("turn artifact reference cannot be empty".to_string());
            }
            if normalized.chars().count() > 4_096 {
                return Err("turn artifact reference is too long".to_string());
            }
            if !seen.insert(normalized) {
                return Err(format!("duplicate turn artifact reference: {normalized}"));
            }
        }
        Ok(())
    }
}

/// Stable external input accepted by an Agent session.
///
/// Submissions are the only public mutation boundary. `AgentCommand` remains
/// the reducer's internal vocabulary and may evolve without forcing every
/// platform transport to understand internal state-machine details.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentSubmission {
    /// Accepted only through the source-validated Application policy command.
    ApplyPermissionPolicy { policy: nola_domain::ActivePermissionPolicy },
    RespondToUserInputWait { input: nola_domain::user_input_wait::RespondToUserInputWait },
    /// Application-only successor admission after the predecessor Run settles.
    PromoteQueuedInput {
        admission_id: String,
        expected_previous_turn_id: String,
        successor_turn_id: String,
        turn_context_components: Vec<crate::context_engine::ContextComponent>,
    },
    /// Reconciles Core with the Host's durable conversation history at an
    /// idle Turn boundary. Ephemeral tool transactions from a prior Turn are
    /// intentionally excluded by the Host instead of leaking into the next
    /// model request.
    RebaseConversation {
        items: Vec<AgentItem>,
        /// Core-authored ownership restored beside the durable transcript.
        item_ownership: std::collections::BTreeMap<String, super::session::AgentItemOwnership>,
    },
    /// Rewinds durable conversation and model context to a Core-owned message
    /// boundary. Hosts must pass the Core message identity rather than
    /// reconstructing model history from a UI transcript.
    RevertConversation {
        through_message_id: String,
        keep_message: bool,
        #[serde(default)]
        submission_id: Option<String>,
    },
    /// Explicit user command. Core admits one delegation tool call without a model round.
    UserDelegation {
        agent_type: String,
        prompt: String,
        submission_id: String,
        submitted_at_epoch_ms: i64,
    },
    UserInput {
        items: Vec<AgentItem>,
        /// Typed, model-visible context for the new Turn. Unlike `items`, these
        /// values are not conversation history and are not persisted as user
        /// or assistant messages.
        #[serde(default)]
        turn_context_items: Vec<AgentItem>,
        /// Platform used to rank same-platform and shared components.
        #[serde(default)]
        turn_context_platform: crate::context_engine::ContextPlatform,
        /// Raw typed components. Core validates authorization/freshness and
        /// materializes the exact model-visible snapshot.
        #[serde(default)]
        turn_context_components: Vec<crate::context_engine::ContextComponent>,
        #[serde(default)]
        submission_id: Option<String>,
        /// Host-observed wall-clock time for deterministic usage/audit records.
        /// Core never reads the system clock itself.
        #[serde(default)]
        submitted_at_epoch_ms: Option<i64>,
        #[serde(default = "default_input_delivery")]
        delivery: AgentInputDelivery,
    },
    /// Starts a visible Turn from fresh user intent that explicitly creates or
    /// resumes a durable Goal. This is not an automated continuation: the user
    /// message remains in conversation history and carries live authority,
    /// while Core also exposes Goal lifecycle control for the Turn.
    GoalUserInput {
        items: Vec<AgentItem>,
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
    /// Starts a non-interactive Turn from a typed automation/recovery source.
    /// This input cannot be queued into an active interactive Turn and never
    /// carries fresh user authority merely because its text was once authored
    /// by a user.
    AutomatedInput {
        origin: AgentInputOrigin,
        items: Vec<AgentItem>,
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
    },
    PermissionResponse {
        request_id: PermissionRequestId,
        decision: PermissionUserDecision,
        #[serde(default)]
        grant: Option<Value>,
    },
    HostResponse {
        response: HostResponseEnvelope,
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
    ResumeHostEffect {
        effect_id: String,
    },
    CloseSession {
        #[serde(default)]
        submission_id: Option<String>,
    },
    /// Atomically adopts a fresh permission-continuation grant after process
    /// loss. The Host supplies only Store-verified predecessor identities.
    ReconcileTaskTerminal {
        operation_id: String,
        fact: super::task::TaskTerminalReconciliation,
    },
    /// Commits progress or a terminal result from a claimed Task worker.
    /// This is an independent durable operation, not a delayed response to
    /// the tool call that originally promoted the Task.
    UpdateTask {
        operation_id: String,
        task_id: String,
        state: super::task::TaskState,
        #[serde(default)]
        progress: Option<f64>,
        #[serde(default)]
        current_step: Option<String>,
        #[serde(default)]
        result_ref: Option<String>,
        #[serde(default)]
        cancel_requested: bool,
        ownership_token: String,
        #[serde(default)]
        completion_watermark: Option<i64>,
    },
    /// Child delivery marker. Application resolves its persisted identity.
    DeliverTaskGuidance {
        operation_id: String,
        task_id: String,
        message_id: String,
        expected_run_id: String,
    },
    ReconcileTaskGuidance {
        operation_id: String,
        fact: super::task::TaskGuidanceResolution,
    },
    ApplyTaskGuidanceInput {
        operation_id: String,
        task_id: String,
        message_id: String,
        expected_run_id: String,
        content: String,
    },
    ObserveTaskGuidanceDelivery {
        operation_id: String,
        task_id: String,
        message_id: String,
        expected_run_id: String,
    },
    /// Acknowledges one stable guidance identity. Core resolves it to the
    /// monotonic sequence it assigned when the message was queued.
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
    /// Applies one idempotent extraction, consolidation, lease, retry,
    /// cancellation, or usage-ledger operation to the Session-owned runtime.
    MemoryRuntime {
        envelope: super::memory_runtime::MemoryRuntimeCommandEnvelope,
    },
    /// Applies one idempotent long-horizon Goal state transition. Executors
    /// report work; only ApplyAudit may advance records to completed.
    GoalRuntime {
        envelope: super::goal::GoalRuntimeCommandEnvelope,
    },
    ClaimMemoryWorker {
        operation_id: String,
        job_id: String,
        worker_id: String,
        ownership_token: String,
        now_epoch_ms: i64,
        lease_until_epoch_ms: i64,
    },
    HeartbeatMemoryWorker {
        operation_id: String,
        job_id: String,
        ownership_token: String,
        now_epoch_ms: i64,
        lease_until_epoch_ms: i64,
    },
    MemoryWorkerFailed {
        operation_id: String,
        job_id: String,
        ownership_token: String,
        now_epoch_ms: i64,
        #[serde(default)]
        retry_after_ms: Option<i64>,
        max_retry_count: u32,
        error: String,
    },
    /// Returns provider-produced structured memory output to Core. Hosts own
    /// provider I/O only; they must not deserialize `MemoryJobOutput` or
    /// construct the internal memory-runtime completion command themselves.
    MemoryWorkerCompleted {
        operation_id: String,
        job_id: String,
        ownership_token: String,
        now_epoch_ms: i64,
        completion_watermark: String,
        output: Value,
    },
    SkillRuntime {
        envelope: super::capability::SkillRuntimeCommandEnvelope,
    },
    Interrupt {
        goal_control: Option<super::goal::GoalRuntimeCommandEnvelope>,
        cancellation_id: String,
        #[serde(default)]
        reason: Option<String>,
    },
    /// Application-only explicit user recovery; generic submission routes reject it.
    ResolveUnknownEffects { decision: nola_domain::unknown_effect_recovery::UnknownEffectRecoveryDecision },
    Compact {
        operation_id: String,
    },
    Reset,
}

impl AgentSubmission {
    pub(crate) fn into_command(self) -> Result<AgentCommand, String> {
        let submission = self;
        let command = match submission {
            AgentSubmission::UserDelegation { agent_type, prompt, submission_id, submitted_at_epoch_ms } => {
                user_delegation_item(&agent_type, &prompt, &submission_id)?;
                if submitted_at_epoch_ms <= 0 { return Err("delegation command requires a positive observation time".into()); }
                AgentCommand::UserDelegation { agent_type, prompt, submission_id, submitted_at_epoch_ms }
            }
            AgentSubmission::ApplyPermissionPolicy { policy } => AgentCommand::ApplyPermissionPolicy { policy },
            AgentSubmission::ResolveUnknownEffects { decision } => AgentCommand::ResolveUnknownEffects { decision },
            AgentSubmission::RespondToUserInputWait { input } => AgentCommand::RespondToUserInputWait { input },
            AgentSubmission::PromoteQueuedInput { admission_id, expected_previous_turn_id, successor_turn_id, turn_context_components } =>
                AgentCommand::PromoteQueuedInput { admission_id, expected_previous_turn_id, successor_turn_id, turn_context_components },
            AgentSubmission::RebaseConversation {
                items,
                item_ownership,
            } => AgentCommand::RebaseConversation {
                items,
                item_ownership,
            },
            AgentSubmission::RevertConversation {
                through_message_id,
                keep_message,
                ..
            } => AgentCommand::RevertConversation {
                through_message_id,
                keep_message,
            },
            AgentSubmission::UserInput {
                items,
                turn_context_items,
                turn_context_platform,
                turn_context_components,
                submission_id,
                submitted_at_epoch_ms,
                delivery,
            } => AgentCommand::SubmitInput {
                items,
                turn_context_items,
                turn_context_platform,
                turn_context_components,
                submission_id,
                submitted_at_epoch_ms,
                delivery,
                origin: AgentInputOrigin::InteractiveUser,
            },
            AgentSubmission::GoalUserInput {
                items,
                turn_context_items,
                turn_context_platform,
                turn_context_components,
                submission_id,
                submitted_at_epoch_ms,
                delivery,
            } => AgentCommand::SubmitInput {
                items,
                turn_context_items,
                turn_context_platform,
                turn_context_components,
                submission_id,
                submitted_at_epoch_ms,
                delivery,
                origin: AgentInputOrigin::GoalInteractiveUser,
            },
            AgentSubmission::AutomatedInput {
                origin,
                items,
                turn_context_items,
                turn_context_platform,
                turn_context_components,
                submission_id,
                submitted_at_epoch_ms,
            } => AgentCommand::SubmitInput {
                items,
                turn_context_items,
                turn_context_platform,
                turn_context_components,
                submission_id,
                submitted_at_epoch_ms,
                delivery: AgentInputDelivery::CurrentTurn,
                origin,
            },
            AgentSubmission::PermissionResponse {
                request_id,
                decision,
                grant,
            } => AgentCommand::PermissionResolved {
                request_id,
                decision,
                grant,
            },
            AgentSubmission::HostResponse { response } => AgentCommand::HostResponded { response },
            AgentSubmission::UpdateHostCatalog { catalog } => {
                AgentCommand::UpdateHostCatalog { catalog }
            }
            AgentSubmission::HostHeartbeat {
                host_id,
                availability_epoch,
                lease_id,
                observed_at_ms,
                extend_until_ms,
            } => AgentCommand::HostHeartbeat {
                host_id,
                availability_epoch,
                lease_id,
                observed_at_ms,
                extend_until_ms,
            },
            AgentSubmission::UpdateSessionConfiguration { patch } => {
                AgentCommand::UpdateSessionConfiguration { patch }
            }
            AgentSubmission::HostDisconnected {
                host_id,
                availability_epoch,
                reason,
            } => AgentCommand::HostDisconnected {
                host_id,
                availability_epoch,
                reason,
            },
            AgentSubmission::ResumeHostEffect { .. } => {
                unreachable!("Host effect recovery is handled at the submission boundary")
            }
            AgentSubmission::CloseSession { .. } => AgentCommand::CloseSession,
            AgentSubmission::ReconcileTaskTerminal { fact, .. } => {
                AgentCommand::ReconcileTaskTerminal { fact }
            }
            AgentSubmission::DeliverTaskGuidance { .. } => AgentCommand::UnresolvedTaskGuidance,
            AgentSubmission::ReconcileTaskGuidance { fact, .. } => {
                AgentCommand::ReconcileTaskGuidance { fact }
            }
            AgentSubmission::ApplyTaskGuidanceInput {
                message_id,
                expected_run_id,
                content,
                ..
            } => AgentCommand::ApplyTaskGuidanceInput {
                message_id,
                expected_run_id,
                content,
            },
            AgentSubmission::ObserveTaskGuidanceDelivery {
                expected_run_id, ..
            } => AgentCommand::ObserveTaskGuidanceDelivery { expected_run_id },
            AgentSubmission::UpdateTask {
                task_id,
                state,
                progress,
                current_step,
                result_ref,
                cancel_requested,
                ownership_token,
                completion_watermark,
                ..
            } => AgentCommand::UpdateTask {
                task_id,
                state,
                progress,
                current_step,
                result_ref,
                cancel_requested,
                ownership_token,
                completion_watermark,
            },
            AgentSubmission::RequestTaskCancellation { task_id } => {
                AgentCommand::RequestTaskCancellation { task_id }
            }
            AgentSubmission::CreateMemory { record } => AgentCommand::CreateMemory { record },
            AgentSubmission::UpdateMemory { record } => AgentCommand::UpdateMemory { record },
            AgentSubmission::DeleteMemory { memory_id } => AgentCommand::DeleteMemory { memory_id },
            AgentSubmission::MemoryRuntime { envelope } => AgentCommand::MemoryRuntime { envelope },
            AgentSubmission::GoalRuntime { envelope } => AgentCommand::GoalRuntime { envelope },
            AgentSubmission::ClaimMemoryWorker {
                operation_id,
                job_id,
                worker_id,
                ownership_token,
                now_epoch_ms,
                lease_until_epoch_ms,
            } => AgentCommand::MemoryRuntime {
                envelope: super::memory_runtime::MemoryRuntimeCommandEnvelope {
                    operation_id,
                    command: super::memory_runtime::MemoryRuntimeCommand::Claim {
                        job_id,
                        worker_id,
                        ownership_token,
                        now_epoch_ms,
                        lease_until_epoch_ms,
                    },
                },
            },
            AgentSubmission::HeartbeatMemoryWorker {
                operation_id,
                job_id,
                ownership_token,
                now_epoch_ms,
                lease_until_epoch_ms,
            } => AgentCommand::MemoryRuntime {
                envelope: super::memory_runtime::MemoryRuntimeCommandEnvelope {
                    operation_id,
                    command: super::memory_runtime::MemoryRuntimeCommand::Heartbeat {
                        job_id,
                        ownership_token,
                        now_epoch_ms,
                        lease_until_epoch_ms,
                    },
                },
            },
            AgentSubmission::MemoryWorkerFailed {
                operation_id,
                job_id,
                ownership_token,
                now_epoch_ms,
                retry_after_ms,
                max_retry_count,
                error,
            } => AgentCommand::MemoryRuntime {
                envelope: super::memory_runtime::MemoryRuntimeCommandEnvelope {
                    operation_id,
                    command: super::memory_runtime::MemoryRuntimeCommand::ScheduleRetry {
                        job_id,
                        ownership_token,
                        now_epoch_ms,
                        retry_after_ms,
                        max_retry_count,
                        error,
                    },
                },
            },
            AgentSubmission::MemoryWorkerCompleted {
                operation_id,
                job_id,
                ownership_token,
                now_epoch_ms,
                completion_watermark,
                output,
            } => AgentCommand::MemoryWorkerCompleted {
                operation_id,
                job_id,
                ownership_token,
                now_epoch_ms,
                completion_watermark,
                output,
            },
            AgentSubmission::SkillRuntime { envelope } => AgentCommand::SkillRuntime { envelope },
            AgentSubmission::Interrupt { goal_control, cancellation_id, reason } => AgentCommand::Cancel { goal_control, cancellation_id, reason },
            AgentSubmission::Compact { operation_id } => {
                if operation_id.trim().is_empty() {
                    return Err("compact operation_id cannot be empty".to_string());
                }
                AgentCommand::RequestCompaction { operation_id }
            }
            AgentSubmission::Reset => AgentCommand::Reset,
        };
        Ok(command)
    }

    pub(crate) fn idempotency_key(&self) -> Option<&str> {
        match self {
            Self::UserDelegation { submission_id, .. } => Some(submission_id),
            Self::UserInput {
                submission_id: Some(submission_id),
                ..
            } if !submission_id.trim().is_empty() => Some(submission_id.trim()),
            Self::GoalUserInput {
                submission_id: Some(submission_id),
                ..
            } if !submission_id.trim().is_empty() => Some(submission_id.trim()),
            Self::AutomatedInput {
                submission_id: Some(submission_id),
                ..
            } if !submission_id.trim().is_empty() => Some(submission_id.trim()),
            Self::RevertConversation {
                submission_id: Some(submission_id),
                ..
            } if !submission_id.trim().is_empty() => Some(submission_id.trim()),
            Self::CloseSession {
                submission_id: Some(submission_id),
            } if !submission_id.trim().is_empty() => Some(submission_id.trim()),
            Self::SkillRuntime { envelope } if !envelope.operation_id.trim().is_empty() => {
                Some(&envelope.operation_id)
            }
            // Goal owns a durable content-bound semantic operation ledger.
            // The generic ID-only journal must neither short-circuit its
            // changed-body check nor collide with another submission family.
            Self::GoalRuntime { .. } => None,
            Self::MemoryWorkerCompleted { operation_id, .. }
            | Self::ClaimMemoryWorker { operation_id, .. }
            | Self::HeartbeatMemoryWorker { operation_id, .. }
            | Self::MemoryWorkerFailed { operation_id, .. }
                if !operation_id.trim().is_empty() =>
            {
                Some(operation_id)
            }
            Self::ReconcileTaskTerminal { operation_id, .. }
            | Self::DeliverTaskGuidance { operation_id, .. }
            | Self::ReconcileTaskGuidance { operation_id, .. }
            | Self::ApplyTaskGuidanceInput { operation_id, .. }
            | Self::ObserveTaskGuidanceDelivery { operation_id, .. }
            | Self::UpdateTask { operation_id, .. }
                if !operation_id.trim().is_empty() =>
            {
                Some(operation_id)
            }
            _ => None,
        }
    }

    pub(crate) fn validate(&self) -> Result<(), String> {
        if let Self::UserDelegation { agent_type, prompt, submission_id, submitted_at_epoch_ms } = self {
            user_delegation_item(agent_type, prompt, submission_id)?;
            if *submitted_at_epoch_ms <= 0 { return Err("delegation command requires a positive observation time".into()); }
        }
        if matches!(
            self,
            Self::UserInput {
                submission_id: Some(submission_id),
                ..
            }
                | Self::GoalUserInput {
                    submission_id: Some(submission_id),
                    ..
                }
                | Self::AutomatedInput {
                    submission_id: Some(submission_id),
                    ..
                }
                | Self::RevertConversation {
                    submission_id: Some(submission_id),
                    ..
                }
                | Self::CloseSession {
                    submission_id: Some(submission_id),
                } if submission_id.trim().is_empty()
        ) {
            return Err("submission_id must not be empty when provided".to_string());
        }
        if matches!(
            self,
            Self::RevertConversation {
                through_message_id,
                ..
            } if through_message_id.trim().is_empty()
        ) {
            return Err("conversation revert requires a Core message id".to_string());
        }
        if let Self::AutomatedInput { origin, .. } = self {
            if !origin.is_automated() {
                return Err(
                    "automated input requires a scheduled, replay, recovery, goal continuation, or delegated origin"
                        .to_string(),
                );
            }
        }
        if matches!(
            self,
            Self::ReconcileTaskTerminal { operation_id, .. }
                | Self::DeliverTaskGuidance { operation_id, .. }
                | Self::ReconcileTaskGuidance { operation_id, .. }
                | Self::ApplyTaskGuidanceInput { operation_id, .. }
                | Self::ObserveTaskGuidanceDelivery { operation_id, .. }
                | Self::UpdateTask { operation_id, .. }
                | Self::ClaimMemoryWorker { operation_id, .. }
                | Self::HeartbeatMemoryWorker { operation_id, .. }
                | Self::MemoryWorkerFailed { operation_id, .. }
                | Self::MemoryWorkerCompleted { operation_id, .. }
                if operation_id.trim().is_empty()
        ) {
            return Err("durable operation id must not be empty".to_string());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    /// Core receipt fact, not a Host choice or a display-derived settlement.
    PermissionSettled { receipt: super::permissions::PermissionReceipt },
    RuntimeDecisionFactObserved { turn_id: TurnId, fact: nola_domain::core_requirements::CoreRuntimeDecisionFact },
    RequirementScopeChanged { turn_id: TurnId, scope: nola_domain::core_requirements::CoreRequirementScope },
    RequirementAssessed { turn_id: TurnId, assessment: nola_domain::core_requirements::CoreRequirementAssessment },
    RunResultDecided { turn_id: TurnId, decision: nola_domain::core_requirements::CoreRunResultDecision },
    UserInputWaitChanged { wait: nola_domain::user_input_wait::UserInputWait },
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
        #[serde(default)]
        model: Option<String>,
        tool_count: usize,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        stable_prefix_digest: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        prompt_fragment_ids: Vec<String>,
        #[serde(default)]
        context_item_count: usize,
        #[serde(default)]
        context_component_count: usize,
        #[serde(default)]
        context_payload_digest: String,
        context_window_tokens: usize,
        prompt_token_limit: usize,
        host_id: String,
        host_catalog_revision: u64,
        host_availability_epoch: u64,
        #[serde(default)]
        capability_projection: super::capability_projection::AgentTurnCapabilityProjection,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        memory_usage: Vec<crate::memory::MemoryUsage>,
    },
    CapabilitiesActivated {
        turn_id: TurnId,
        source: String,
        capability_ids: Vec<String>,
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
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        items: Vec<AgentItem>,
    },
    InputApplied {
        turn_id: TurnId,
        applications: Vec<super::InputApplication>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        items: Vec<AgentItem>,
    },
    /// The only UI-facing Item/Part mutation.
    ///
    /// Runtime-internal model and tool events are normalized into this event
    /// before leaving Core. `part` is the resulting stable snapshot while
    /// `update` preserves the exact incremental operation.
    PartUpdated {
        turn_id: TurnId,
        projection_revision: u64,
        update: PartUpdate,
        part: AgentPart,
    },
    HostProgress {
        #[serde(default)]
        turn_id: Option<TurnId>,
        effect_id: String,
        title: String,
        #[serde(default)]
        detail: Option<String>,
        #[serde(default)]
        metadata: Value,
    },
    LifecycleHookUpdated {
        #[serde(default)]
        turn_id: Option<TurnId>,
        projection: crate::lifecycle_hooks::LifecycleHookExecutionProjection,
    },
    TaskUpdated {
        task_projection_revision: u64,
        task: super::task::TaskHandle,
    },
    GoalUpdated {
        goal_projection_revision: u64,
        goal: super::goal::GoalProjection,
    },
    MemoryUpdated {
        memory_projection_revision: u64,
        update: crate::memory::MemoryUpdate,
    },
    SkillRuntimeUpdated {
        #[serde(default)]
        turn_id: Option<TurnId>,
        skill_runtime_revision: u64,
        update: super::capability::SkillRuntimeEvent,
    },
    EngineeringLoopUpdated {
        turn_id: TurnId,
        state: super::turn::AgentEngineeringLoopState,
    },
    ExecutionEvidenceObserved {
        turn_id: TurnId,
        evidence: nola_domain::execution_evidence::ExecutionEvidenceObservation,
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
        #[serde(default)]
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
        #[serde(default)]
        context_usage: super::context_window::ContextUsageSnapshot,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        prompt_cache_diagnostic: Option<crate::prompt_cache::PromptCacheDiagnostic>,
    },
    CompactionRequested {
        reason: super::context_window::CompactionReason,
        item_count: usize,
        estimated_tokens: usize,
        #[serde(default)]
        source_fingerprint: String,
        #[serde(default)]
        source_item_count: usize,
        #[serde(default)]
        original_token_count: usize,
    },
    CompactionCompleted {
        summary: String,
        replaced_item_count: usize,
        remaining_item_count: usize,
        #[serde(default)]
        reason: Option<super::context_window::CompactionReason>,
        #[serde(default)]
        compaction_item_id: Option<ItemId>,
        #[serde(default)]
        source_fingerprint: Option<String>,
        #[serde(default)]
        source_item_count: usize,
        #[serde(default)]
        original_token_count: usize,
    },
    CompactionFailed {
        error: String,
    },
    TurnCompleted {
        session_id: SessionId,
        turn_id: TurnId,
        #[serde(default)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDurabilityIntent {
    /// Application owns persistence. Core only reports why the current
    /// transition should become durable; no checkpoint payload crosses the
    /// Host boundary or enters the pending-effect journal.
    pub checkpoint_reason: CheckpointReason,
}

/// Execution facts without a presentation snapshot. Streaming consumers use
/// this result directly; only boundary callers materialize the current view.
#[derive(Debug, Clone)]
pub struct AgentReduction {
    pub event_envelopes: Vec<CoreEventEnvelope<AgentEvent>>,
    pub trace_events: Vec<AgentTraceEvent>,
    pub host_requests: Vec<HostRequestEnvelope>,
    pub durability: Option<AgentDurabilityIntent>,
}

impl AgentReduction {
    pub fn with_snapshot(self, snapshot: AgentCoreSnapshot) -> AgentTransition {
        AgentTransition {
            event_envelopes: self.event_envelopes,
            trace_events: self.trace_events,
            host_requests: self.host_requests,
            durability: self.durability,
            snapshot,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTransition {
    #[serde(default)]
    pub event_envelopes: Vec<CoreEventEnvelope<AgentEvent>>,
    /// Canonical UI projection produced by Core from `event_envelopes`.
    ///
    /// Hosts execute effects; presenters consume these traces. Platforms must
    /// not reconstruct tool/reasoning/message lifecycle from provider chunks.
    #[serde(default)]
    pub trace_events: Vec<AgentTraceEvent>,
    #[serde(default)]
    pub host_requests: Vec<HostRequestEnvelope>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub durability: Option<AgentDurabilityIntent>,
    pub snapshot: AgentCoreSnapshot,
}

/// Bounded committed control, independent from every cumulative display/body
/// projection. Permission revision is a wakeup for an exact-owner read.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentControlSnapshot {
    pub active_permission_policy: Option<nola_domain::ActivePermissionPolicy>,
    pub open_user_input_waits: Vec<nola_domain::user_input_wait::UserInputWait>,
    pub session_id: SessionId,
    pub revision: u64,
    pub last_event_cursor: EventCursor,
    pub host_catalog_revision: u64,
    pub model: Option<String>,
    pub tool_count: usize,
    pub active_turn_id: Option<TurnId>,
    pub turn_status: super::turn::AgentTurnStatus,
    pub completion_outcome: Option<nola_domain::TaskCompletionOutcome>,
    pub history_item_count: usize,
    pub model_context_item_count: usize,
    pub queued_input_count: usize,
    pub active_tool_count: usize,
    pub context_window_tokens: usize,
    pub context_used_tokens: usize,
    pub context_usage_percent: f64,
    pub context_usage_is_estimated: bool,
    pub context_usage: super::context_window::ContextUsageSnapshot,
    pub prompt_cache_diagnostic: Option<crate::prompt_cache::PromptCacheDiagnostic>,
    pub compaction_in_progress: bool,
    pub consecutive_compaction_failures: u8,
    pub automatic_compaction_blocked: bool,
    pub pending_host_effect_count: usize,
    pub pending_recovery_effect_count: usize,
    pub session_closing: bool,
    pub session_closed: bool,
    pub permission_projection_revision: u64,
}

impl From<&AgentCoreSnapshot> for AgentControlSnapshot {
    fn from(snapshot: &AgentCoreSnapshot) -> Self {
        Self {
            open_user_input_waits: snapshot.open_user_input_waits.clone(),
            active_permission_policy: snapshot.active_permission_policy.clone(),
            session_id: snapshot.session_id.clone(),
            revision: snapshot.revision,
            last_event_cursor: snapshot.last_event_cursor,
            host_catalog_revision: snapshot.host_catalog_revision,
            model: snapshot.model.clone(),
            tool_count: snapshot.tool_count,
            active_turn_id: snapshot.active_turn_id.clone(),
            turn_status: snapshot.turn_status,
            completion_outcome: snapshot.completion_outcome,
            history_item_count: snapshot.history_item_count,
            model_context_item_count: snapshot.model_context_item_count,
            queued_input_count: snapshot.queued_input_count,
            active_tool_count: snapshot.active_tool_count,
            context_window_tokens: snapshot.context_window_tokens,
            context_used_tokens: snapshot.context_used_tokens,
            context_usage_percent: snapshot.context_usage_percent,
            context_usage_is_estimated: snapshot.context_usage_is_estimated,
            context_usage: snapshot.context_usage.clone(),
            prompt_cache_diagnostic: snapshot.prompt_cache_diagnostic.clone(),
            compaction_in_progress: snapshot.compaction_in_progress,
            consecutive_compaction_failures: snapshot.consecutive_compaction_failures,
            automatic_compaction_blocked: snapshot.automatic_compaction_blocked,
            pending_host_effect_count: snapshot.pending_host_effect_count,
            pending_recovery_effect_count: snapshot.pending_recovery_effect_count,
            session_closing: snapshot.session_closing,
            session_closed: snapshot.session_closed,
            permission_projection_revision: snapshot.permissions.revision,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentCoreSnapshot {
    pub active_permission_policy: Option<nola_domain::ActivePermissionPolicy>,
    pub open_user_input_waits: Vec<nola_domain::user_input_wait::UserInputWait>,
    pub session_id: SessionId,
    pub revision: u64,
    pub last_event_cursor: EventCursor,
    pub host_catalog_revision: u64,
    #[serde(default)]
    pub model: Option<String>,
    pub tool_count: usize,
    #[serde(default)]
    pub active_turn_id: Option<TurnId>,
    pub turn_status: super::turn::AgentTurnStatus,
    /// Outcome of reviewed task content, not merely model/Turn termination.
    #[serde(default)]
    pub completion_outcome: Option<nola_domain::TaskCompletionOutcome>,
    /// Canonical append-only transcript size.
    pub history_item_count: usize,
    /// Current model-visible context size after compaction.
    pub model_context_item_count: usize,
    pub queued_input_count: usize,
    pub active_tool_count: usize,
    #[serde(default)]
    pub engineering: super::turn::AgentEngineeringLoopState,
    pub context_window_tokens: usize,
    pub context_used_tokens: usize,
    pub context_usage_percent: f64,
    pub context_usage_is_estimated: bool,
    /// Authoritative context usage consumed by every Host.
    #[serde(default)]
    pub context_usage: super::context_window::ContextUsageSnapshot,
    /// Latest provider-backed prompt-cache diagnosis. It contains digests and
    /// bounded enum reasons only, never prompt or tool-schema content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_cache_diagnostic: Option<crate::prompt_cache::PromptCacheDiagnostic>,
    pub compaction_in_progress: bool,
    pub consecutive_compaction_failures: u8,
    pub automatic_compaction_blocked: bool,
    pub pending_host_effect_count: usize,
    pub pending_recovery_effect_count: usize,
    #[serde(default)]
    pub session_closing: bool,
    #[serde(default)]
    pub session_closed: bool,
    #[serde(default)]
    pub tasks: super::task::TaskProjection,
    /// Authoritative long-horizon objective and audited task state. Clients
    /// render this projection instead of reconstructing progress from chat.
    #[serde(default)]
    pub goal: super::goal::GoalProjection,
    #[serde(default)]
    pub memories: crate::memory::MemoryProjection,
    #[serde(default)]
    pub memory_runtime: super::memory_runtime::MemoryRuntimeProjection,
    /// Authoritative pending permission requests for reconnecting Hosts.
    #[serde(default)]
    pub permissions: super::permissions::PermissionProjection,
    /// Effective structured rules, including conversation grants. Remote
    /// product Hosts persist only durable sources and inject them into the
    /// next runtime instead of reconstructing grants from UI events.
    #[serde(default)]
    pub permission_rules: super::permissions::PermissionRuleSet,
    #[serde(default)]
    pub skill_runtime: super::capability::SkillRuntimeState,
    pub projection: ConversationProjection,
}

#[cfg(test)]
mod estimated_token_tests {
    use super::{AgentItem, ItemId};

    #[test]
    fn estimation_uses_unicode_characters_instead_of_utf8_bytes() {
        let ascii = AgentItem::user_message(ItemId::from_string("ascii".to_string()), "12345678");
        let unicode = AgentItem::user_message(ItemId::from_string("unicode".to_string()), "中文🙂");

        assert_eq!(ascii.estimated_tokens(), 6);
        assert_eq!(unicode.estimated_tokens(), 5);
        assert_ne!(unicode.estimated_tokens(), "中文🙂".len().div_ceil(4) + 4);
    }
}

/// The visible user message and its identity are shared with Application admission.
pub fn user_delegation_item(agent_type: &str, prompt: &str, submission_id: &str) -> Result<AgentItem, String> {
    if agent_type.is_empty() || agent_type.trim() != agent_type || agent_type.chars().any(char::is_whitespace)
        || prompt.trim().is_empty() || submission_id.is_empty() || submission_id.trim() != submission_id
        || submission_id.chars().any(char::is_control) {
        return Err("invalid explicit delegation command".into());
    }
    Ok(AgentItem::user_message(ItemId::from_string(format!("{submission_id}:user")),
        format!("/agent {agent_type} {prompt}")))
}

#[cfg(test)]
mod closed_host_configuration_contract {
    use super::*;
    #[test]
    fn current_host_config_rejects_unknown_fields_instead_of_dropping_them_before_attestation() {
        assert!(serde_json::from_value::<AgentCoreConfig>(serde_json::json!({"sessionId":"ignored-owner"})).is_err());
        assert!(serde_json::from_value::<AgentCoreConfig>(serde_json::json!({"turn_identity":{"unverified_model_alias":"wrong"}})).is_err());
        let config=AgentCoreConfig::default();
        let encoded=serde_json::to_value(&config).unwrap();
        let decoded:AgentCoreConfig=serde_json::from_value(encoded).unwrap();
        assert!(decoded.turn_identity.fresh_resend_policy.is_none());
    }
}
