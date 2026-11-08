// SPDX-License-Identifier: Apache-2.0
//! Canonical platform-neutral effect identities and request/response contracts.
//! Shared by persistence, reducers and transport; no Host execution or I/O.
use crate::PermissionInstruction;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const DELEGATED_AGENT_CONTRACT_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DelegatedAgentContextMode {
    Bootstrap,
    Resume,
}

/// Declares the semantic shape expected from one model transaction. This is
/// request policy owned by Core, not a provider setting inferred by a Host.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelResponseMode {
    #[default]
    Conversational,
    /// Core has narrowed this model step to one or more internal protocol
    /// tools and requires a native tool call. Hosts translate this semantic
    /// contract to the provider's required-tool setting without choosing a
    /// capability from user wording.
    RequiredTool,
    StructuredAudit,
}

/// Terminal semantic outcome of one Host tool invocation.
///
/// This is deliberately not a boolean: cancellation is neither success nor
/// failure and must survive the Host -> Core -> Part projection boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostToolOutcome {
    Succeeded,
    /// The foreground call durably admitted work, but its effects have not
    /// completed. This is a receipt, never mutation or validation evidence.
    Deferred,
    Failed,
    Cancelled,
}

impl HostToolOutcome {
    pub fn is_error(self) -> bool {
        !matches!(self, Self::Succeeded | Self::Deferred)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DelegatedAgentContextV1 {
    pub mode: DelegatedAgentContextMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_anchor_item_id: Option<String>,
    pub target_anchor_item_id: String,
    pub identity_digest: String,
    pub messages: Vec<Value>,
}

/// Versioned payload for a Host-owned Agent executor such as Codex, Claude
/// Code, or OpenCode. This is an execution request, not a persistence model:
/// it intentionally contains no Message/Part/Run rows or UI presentation data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DelegatedAgentRequestV1 {
    pub contract_version: u32,
    pub backend_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    pub text_part_id: String,
    pub reasoning_part_id: String,
    pub context: DelegatedAgentContextV1,
    #[serde(default)]
    pub tools: Vec<Value>,
    #[serde(default)]
    pub settings: Value,
}

impl DelegatedAgentRequestV1 {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.contract_version != DELEGATED_AGENT_CONTRACT_VERSION {
            return Err("unsupported delegated Agent contract version");
        }
        if self.backend_id.trim().is_empty()
            || self.backend_id == "nola"
            || self.text_part_id.trim().is_empty()
            || self.reasoning_part_id.trim().is_empty()
        {
            return Err("delegated Agent request identity is incomplete");
        }
        if self.context.messages.is_empty() {
            return Err("delegated Agent request has no model-visible messages");
        }
        if self.context.target_anchor_item_id.trim().is_empty() {
            return Err("delegated Agent request has no target context anchor");
        }
        if self.context.identity_digest.trim().is_empty() {
            return Err("delegated Agent request has no frozen context identity");
        }
        match self.context.mode {
            DelegatedAgentContextMode::Bootstrap if self.context.base_anchor_item_id.is_some() => {
                return Err("bootstrap delegated context cannot have a base anchor");
            }
            DelegatedAgentContextMode::Resume
                if self.backend_session_id.is_none()
                    || self
                        .context
                        .base_anchor_item_id
                        .as_deref()
                        .is_none_or(|value| value.trim().is_empty()) =>
            {
                return Err("resumed delegated context requires a session and base anchor");
            }
            _ => {}
        }
        if self
            .backend_session_id
            .as_deref()
            .is_some_and(|value| value.trim().is_empty())
            || self
                .model_id
                .as_deref()
                .is_some_and(|value| value.trim().is_empty())
        {
            return Err("delegated Agent request contains an empty optional identity");
        }
        Ok(())
    }
}

/// Facts observed from a delegated Agent. A Host adapter emits these updates;
/// only Agent Core may reduce them into the durable conversation projection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum DelegatedAgentEventV1 {
    SessionStarted {
        backend_id: String,
        backend_session_id: String,
    },
    TextDelta {
        part_id: String,
        delta: String,
    },
    ReasoningDelta {
        part_id: String,
        delta: String,
    },
    ToolStarted {
        call_id: String,
        capability_id: String,
        #[serde(default)]
        input: Value,
    },
    ToolProgress {
        call_id: String,
        channel: ToolProgressChannel,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        delta: Option<String>,
        #[serde(default)]
        metadata: Value,
    },
    ToolCompleted {
        call_id: String,
        capability_id: String,
        #[serde(default)]
        output: Value,
        outcome: HostToolOutcome,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        full_output_ref: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        artifacts: Vec<HostArtifact>,
    },
    PermissionRequested {
        occurrence: DelegatedPermissionOccurrence,
        request_id: String,
        call_id: String,
        capability_id: String,
        resource: String,
        reason: String,
        #[serde(default)]
        input: Value,
    },
    Usage {
        used_tokens: u64,
        context_window_tokens: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        input_tokens: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        output_tokens: Option<u64>,
    },
    Accounting { snapshot: crate::delegated_invocation::ExternalUsageSnapshot },
    Completed {
        #[serde(default)]
        items: Vec<Value>,
        #[serde(default)]
        usage: Value,
    },
    Failed {
        error: HostError,
    },
    CancellationConfirmed,
}

impl DelegatedAgentEventV1 {
    pub fn validate(&self) -> Result<(), &'static str> {
        let nonempty = |value: &str| !value.trim().is_empty();
        match self {
            Self::SessionStarted {
                backend_id,
                backend_session_id,
            } if nonempty(backend_id) && nonempty(backend_session_id) => Ok(()),
            Self::TextDelta { part_id, delta } | Self::ReasoningDelta { part_id, delta }
                if nonempty(part_id) && !delta.is_empty() =>
            {
                Ok(())
            }
            Self::ToolStarted {
                call_id,
                capability_id,
                ..
            }
            | Self::ToolCompleted {
                call_id,
                capability_id,
                ..
            } if nonempty(call_id) && nonempty(capability_id) => Ok(()),
            Self::ToolProgress { call_id, .. } if nonempty(call_id) => Ok(()),
            Self::PermissionRequested {
                occurrence,
                request_id,
                call_id,
                capability_id,
                resource,
                ..
            } if nonempty(&occurrence.occurrence_id)
                && nonempty(&occurrence.worker_generation)
                && nonempty(request_id)
                && nonempty(call_id)
                && nonempty(capability_id)
                && nonempty(resource) =>
            {
                Ok(())
            }
            Self::Usage {
                context_window_tokens,
                used_tokens,
                ..
            } if *context_window_tokens > 0 && used_tokens <= context_window_tokens => Ok(()),
            Self::Accounting { snapshot } => snapshot.validate(),
            Self::Completed { .. } | Self::CancellationConfirmed => Ok(()),
            Self::Failed { error } if nonempty(&error.code) && nonempty(&error.message) => Ok(()),
            _ => Err("delegated Agent event is incomplete or inconsistent"),
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Completed { .. } | Self::Failed { .. } | Self::CancellationConfirmed
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct HostId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct HostEffectId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostLease {
    pub lease_id: String,
    pub issued_at_ms: u64,
    pub last_heartbeat_ms: u64,
    pub expires_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostRequestBatchIdentity {
    pub batch_id: String,
    pub index: usize,
    pub size: usize,
}

/// Application-owned fencing identity for one physical delivery of a logical
/// Host effect. Hosts echo this value unchanged on every response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectDeliveryLease {
    pub attempt_id: String,
    pub worker_generation: String,
    pub lease_owner: String,
    pub lease_token: String,
    pub lease_expires_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HostRequestEnvelope<P> {
    #[serde(default)]
    pub attempt_started: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delegated_invocation_id: Option<String>,
    pub request_id: String,
    pub effect_id: HostEffectId,
    pub host_id: HostId,
    #[serde(default)]
    pub lease_id: Option<String>,
    pub session_id: String,
    pub turn_id: Option<String>,
    pub catalog_revision: u64,
    pub availability_epoch: u64,
    pub created_at_ms: u64,
    pub deadline_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub batch: Option<HostRequestBatchIdentity>,
    #[serde(default)]
    pub resume_after_sequence: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery: Option<EffectDeliveryLease>,
    pub payload: P,
}

impl<P> HostRequestEnvelope<P> {
    pub fn is_expired_at(&self, now_ms: u64) -> bool {
        self.deadline_ms.is_some_and(|deadline| now_ms >= deadline)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HostResponseEnvelope<P> {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delegated_invocation_id: Option<String>,
    pub request_id: String,
    pub effect_id: HostEffectId,
    pub host_id: HostId,
    #[serde(default)]
    pub lease_id: Option<String>,
    pub session_id: String,
    pub turn_id: Option<String>,
    pub catalog_revision: u64,
    pub availability_epoch: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery: Option<EffectDeliveryLease>,
    pub sequence: u64,
    pub payload: P,
}

/// Stable Host request payload. Semantic snapshots owned by Agent Core are
/// generic parameters so a Host can implement this protocol without linking
/// the reducer that produced the request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HostRequest<
    TPermissionEvaluation = Value,
    TCapabilityError = Value,
    TLifecycleEvent = Value,
> {
    StartCodeCell {
        identity: crate::code_mode::CodeCellIdentity,
        limits: crate::code_mode::CodeCellLimits,
        initial_wait_ms: u32,
        source: String,
        tools: Vec<crate::code_mode::CodeToolDescription>,
    },
    ResolveCodeTool {
        result: crate::code_mode::CodeToolResult,
    },
    PollCodeCell {
        cell: crate::code_mode::CodeCellIdentity,
    },
    ObserveCodeCell {
        call_id: String,
        observation: crate::code_mode::CodeCellObservation,
    },
    /// Application-owned history observation. It never starts or controls a
    /// worker; the accepted Run binds the canonical conversation, not input.
    ReadCodeCellHistory {
        call_id: String,
        cell_id: String,
        after_output_sequence: u64,
    },
    ModelStream {
        model_id: String,
        text_part_id: String,
        reasoning_part_id: String,
        response_mode: ModelResponseMode,
        messages: Vec<Value>,
        tools: Vec<Value>,
        settings: Value,
    },
    DelegatedAgent {
        request: DelegatedAgentRequestV1,
    },
    ResolvePermissionResources {
        call_id: String,
        capability_id: String,
        input: Value,
        request: crate::permission_resource_plan::PermissionResolutionRequest,
    },
    ExecuteTool {
        /// Core-admitted policy for this exact effect, never model arguments.
        permission_profile: crate::PermissionProfile,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        permission_instructions: Vec<PermissionInstruction>,
        capability_id: String,
        call_id: String,
        input: Value,
        #[serde(default)]
        grant: Option<Value>,
    },
    RequestPermission {
        allow_conversation: bool,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        permission_instructions: Vec<PermissionInstruction>,
        permission_request_id: String,
        capability_id: String,
        resource: String,
        reason: String,
        #[serde(default)]
        input: Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        evaluation: Option<TPermissionEvaluation>,
    },
    ResolveDelegatedPermission {
        occurrence: DelegatedPermissionOccurrence,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        delivery: Option<DelegatedPermissionDeliveryTarget>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        permission_instructions: Vec<PermissionInstruction>,
        target_effect_id: HostEffectId,
        backend_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        backend_session_id: Option<String>,
        permission_request_id: String,
        resolution: HostPermissionResolution,
    },
    FetchContext {
        source_id: String,
        query: Value,
        freshness_deadline_ms: Option<u64>,
    },
    Compact {
        /// Admitted session/model window; maintenance must not guess capacity.
        context_window_tokens: u64,
        strategy: String,
        instructions: String,
        messages: Vec<Value>,
        target_tokens: u64,
    },
    Cancel {
        target_effect_id: HostEffectId,
        #[serde(deserialize_with = "required_cancel_not_started")]
        target_not_started: Option<crate::cancellation_not_started::CancellationTargetNotStarted>,
    },
    CancelTask {
        task_id: String,
    },
    RefreshCapabilityCatalog {
        run_id: String,
        catalog_revision: u64,
        errors: Vec<TCapabilityError>,
    },
    RunLifecycleHook {
        event: TLifecycleEvent,
    },
}

fn required_cancel_not_started<'de, D: serde::Deserializer<'de>>(deserializer: D)
    -> Result<Option<crate::cancellation_not_started::CancellationTargetNotStarted>, D::Error> {
    Option::deserialize(deserializer)
}

impl<TPermissionEvaluation, TCapabilityError, TLifecycleEvent>
    HostRequest<TPermissionEvaluation, TCapabilityError, TLifecycleEvent>
{
    /// Validates the response family before a reducer applies it.
    pub fn accepts_response<
        TContinuation,
        TTaskKind,
        TTaskState,
        TTaskLineage,
        TCapabilityCatalog,
        TLifecycleProjection,
    >(
        &self,
        response: &HostResponse<
            TContinuation,
            TTaskKind,
            TTaskState,
            TTaskLineage,
            TCapabilityCatalog,
            TLifecycleProjection,
        >,
    ) -> bool {
        crate::host_control::HostResponsePolicy::from_request(self).accepts_response(response)
    }
}

/// An unavailable permission transport is not a user authorization decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionTransportInterruptionReason {
    Timeout,
    Disconnected,
    Unavailable,
}

/// Stable Host response payload. Reducer-owned snapshots remain generic and
/// therefore cannot create a reverse protocol-to-Core dependency.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HostResponse<
    TContinuation = Value,
    TTaskKind = Value,
    TTaskState = Value,
    TTaskLineage = Value,
    TCapabilityCatalog = Value,
    TLifecycleProjection = Value,
> {
    Accepted,
    /// The Host is about to cross the physical side-effect boundary. It must
    /// wait for Application to commit this response before starting execution.
    ExecutionStarted,
    /// The Host refused a still-unstarted fenced delivery. Application must
    /// durably settle the lease before Core observes the logical failure.
    ExecutionRejected {
        code: String,
        message: String,
        retryable: bool,
    },
    DelegatedAgentEvent {
        event: DelegatedAgentEventV1,
    },
    /// The admitted external operation has no attached worker. Session identity
    /// alone does not authorize resubmitting its original input.
    DelegatedExecutionInterrupted {
        reason: DelegatedExecutionInterruptionReason,
    },
    ModelDelta {
        channel: ModelDeltaChannel,
        part_id: String,
        delta: String,
    },
    /// A semantic text unit that Core may expose immediately. Hosts emit this
    /// only after the model transaction is known not to contain a tool call.
    ModelCommittedDelta {
        part_id: String,
        delta: String,
    },
    ModelToolCallDelta {
        part_id: String,
        call_id: String,
        capability_id: String,
        arguments_delta: String,
    },
    ModelContinuation {
        continuation: TContinuation,
    },
    /// Requests durable admission for a resolved physical model sampling.
    /// Rejected admission must prevent the provider request from starting.
    /// Physical request accounting, independent of whether its output is usable.
    ModelUsageReported {
        usage: serde_json::Value,
    },
    ModelAttemptStarted {
        provider: String,
        model: String,
        reason: String,
    },
    ToolProgress {
        title: String,
        detail: Option<String>,
        metadata: Value,
    },
    /// Nonterminal request from an already admitted code cell. It is not a
    /// tool result and must be separately admitted by Core.
    CodeModeToolRequested {
        request: crate::code_mode::CodeNestedCall,
    },
    CodeModeOutput {
        frame: crate::code_mode::CodeOutputFrame,
    },
    /// Foreground observation ended; the original effect remains live.
    CodeModeYielded {
        cell: crate::code_mode::CodeCellIdentity,
        output_cursor: u64,
    },
    /// Observation temporarily released the Host to execute nested work.
    /// Unlike exec yield, this must not complete the model's wait call.
    CodeModeObservationSuspended {
        cell: crate::code_mode::CodeCellIdentity,
        call_id: String,
    },
    ToolCallProgress {
        call_id: String,
        channel: ToolProgressChannel,
        #[serde(default)]
        title: Option<String>,
        #[serde(default)]
        detail: Option<String>,
        #[serde(default)]
        delta: Option<String>,
        #[serde(default)]
        metadata: Value,
    },
    TaskPromoted {
        task_id: String,
        #[serde(default)]
        input_watermark: Option<i64>,
        kind: TTaskKind,
        state: TTaskState,
        #[serde(default)]
        progress: Option<f64>,
        #[serde(default)]
        current_step: Option<String>,
        #[serde(default)]
        lineage: TTaskLineage,
    },
    TaskUpdated {
        task_id: String,
        state: TTaskState,
        #[serde(default)]
        progress: Option<f64>,
        #[serde(default)]
        current_step: Option<String>,
        #[serde(default)]
        result_ref: Option<String>,
        #[serde(default)]
        cancel_requested: bool,
        #[serde(default)]
        ownership_token: Option<String>,
        #[serde(default)]
        completion_watermark: Option<i64>,
    },
    DelegatedPermissionClaimed {
        delivery_id: String,
    },
    DelegatedPermissionTransportWritten {
        delivery_id: String,
    },
    DelegatedPermissionInterrupted {
        delivery_id: String,
    },
    PermissionTransportInterrupted {
        reason: PermissionTransportInterruptionReason,
    },
    PermissionResolved {
        request_id: String,
        resolution: HostPermissionResolution,
    },
    PermissionResourcesResolved {
        call_id: String,
        facts: crate::permission_resource_plan::PermissionResolutionFacts,
    },
    ContextFetched {
        components: Vec<Value>,
    },
    CapabilityCatalogRefreshed {
        catalog: TCapabilityCatalog,
    },
    ModelCompleted {
        items: Vec<Value>,
        tool_calls: Vec<Value>,
        usage: Value,
    },
    /// Valid partial output whose provider explicitly reported an output-token
    /// limit. Core decides whether a bounded semantic continuation is useful.
    ModelOutputLimited {
        items: Vec<Value>,
        tool_calls: Vec<Value>,
        usage: Value,
    },
    LifecycleHookUpdated {
        projection: TLifecycleProjection,
    },
    ToolCompleted {
        call_id: String,
        output: Value,
        outcome: HostToolOutcome,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        activation: Option<HostCapabilityActivation>,
        full_output_ref: Option<String>,
        artifacts: Vec<HostArtifact>,
    },
    CompactionCompleted {
        summary: String,
        usage: Value,
    },
    Completed {
        output: Value,
        usage: Option<Value>,
        full_output_ref: Option<String>,
        artifacts: Vec<HostArtifact>,
    },
    Failed {
        error: HostError,
    },
    CancellationConfirmed {
        target_effect_id: HostEffectId,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DelegatedExecutionInterruptionReason {
    WorkerUnavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostPermissionAuthorizationDecision {
    AllowOnce,
    AllowConversation,
    Deny,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, tag = "type", rename_all = "snake_case")]
pub enum HostPermissionIntervention {
    ProvideInstruction { instruction: String },
    ReturnToComposer,
}

/// Canonical permission resolution exchanged with a Host.
///
/// Authorization and user intervention are mutually exclusive in the wire
/// type. This prevents invalid combinations such as a denial carrying a
/// conversation grant or an allow decision carrying composer instructions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, tag = "type", rename_all = "snake_case")]
pub enum HostPermissionResolution {
    Authorization {
        decision: HostPermissionAuthorizationDecision,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        grant: Option<Value>,
    },
    Intervention {
        intervention: HostPermissionIntervention,
    },
}

impl HostPermissionResolution {
    pub fn allow_once() -> Self {
        Self::Authorization {
            decision: HostPermissionAuthorizationDecision::AllowOnce,
            grant: None,
        }
    }

    pub fn allow_conversation(grant: Option<Value>) -> Self {
        Self::Authorization {
            decision: HostPermissionAuthorizationDecision::AllowConversation,
            grant,
        }
    }

    pub fn deny() -> Self {
        Self::Authorization {
            decision: HostPermissionAuthorizationDecision::Deny,
            grant: None,
        }
    }

    pub fn provide_instruction(instruction: impl Into<String>) -> Self {
        Self::Intervention {
            intervention: HostPermissionIntervention::ProvideInstruction {
                instruction: instruction.into(),
            },
        }
    }

    pub fn return_to_composer() -> Self {
        Self::Intervention {
            intervention: HostPermissionIntervention::ReturnToComposer,
        }
    }

    pub fn is_allowed(&self) -> bool {
        matches!(
            self,
            Self::Authorization {
                decision: HostPermissionAuthorizationDecision::AllowOnce
                    | HostPermissionAuthorizationDecision::AllowConversation,
                ..
            }
        )
    }
}

/// Minimal semantic hook needed for terminal response classification.
pub trait HostTaskState {
    fn is_terminal(&self) -> bool;
}

impl<
        TContinuation,
        TTaskKind,
        TTaskState: HostTaskState,
        TTaskLineage,
        TCapabilityCatalog,
        TLifecycleProjection,
    >
    HostResponse<
        TContinuation,
        TTaskKind,
        TTaskState,
        TTaskLineage,
        TCapabilityCatalog,
        TLifecycleProjection,
    >
{
    pub fn is_terminal(&self) -> bool {
        match self {
            // Promotion hands execution ownership from the foreground tool
            // request to the durable Task protocol. The Task may still be
            // running, but the original Host request has been fully handled;
            // later lifecycle changes arrive as Task submissions rather than
            // additional responses to this request.
            Self::TaskPromoted { .. } => true,
            Self::TaskUpdated { state, .. } => state.is_terminal(),
            Self::DelegatedAgentEvent { event } => event.is_terminal(),
            Self::DelegatedPermissionTransportWritten { .. }
            | Self::ExecutionRejected { .. }
            | Self::PermissionResolved { .. }
            | Self::ContextFetched { .. }
            | Self::PermissionResourcesResolved { .. }
            | Self::CapabilityCatalogRefreshed { .. }
            | Self::ModelCompleted { .. }
            | Self::ModelOutputLimited { .. }
            | Self::ToolCompleted { .. }
            | Self::CompactionCompleted { .. }
            | Self::Completed { .. }
            | Self::Failed { .. }
            | Self::CancellationConfirmed { .. } => true,
            _ => false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelDeltaChannel {
    Text,
    Reasoning,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolProgressChannel {
    Status,
    Stdout,
    Stderr,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostCapabilityActivation {
    pub source: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capability_ids: Vec<String>,
    /// Exact typed descriptors for every selected capability, including those
    /// already advertised at Turn start. Core never fills missing descriptors
    /// from historical catalog state and installs the batch only after all
    /// entries validate.
    /// Hosts keep their complete discovery registry process-local and attach
    /// only the selected subset so Core can validate, execute, and checkpoint
    /// the activated capability without persisting the full optional catalog.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capability_descriptors: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostArtifact {
    pub uri: String,
    pub name: String,
    pub mime_type: Option<String>,
    pub operation: Option<String>,
}

impl HostArtifact {
    /// Validates the canonical resource identity crossing the Host boundary.
    ///
    /// Hosts own platform path resolution and must submit an absolute,
    /// normalized URI. Core stores that identity verbatim; it never guesses
    /// whether an arbitrary string was intended to be a filesystem path.
    pub fn validate(&self) -> Result<(), String> {
        if self.uri.trim() != self.uri || self.uri.is_empty() {
            return Err("Host artifact URI must be a non-empty canonical absolute URI".to_string());
        }
        let parsed = url::Url::parse(&self.uri)
            .map_err(|_| "Host artifact URI must be a canonical absolute URI".to_string())?;
        if parsed.as_str() != self.uri {
            return Err("Host artifact URI must be normalized and percent-encoded".to_string());
        }
        if parsed.scheme() == "file" && parsed.to_file_path().is_err() {
            return Err("Host file artifact URI must identify an absolute local path".to_string());
        }
        if self.name.trim().is_empty() {
            return Err("Host artifact name must not be empty".to_string());
        }
        if self
            .mime_type
            .as_deref()
            .is_some_and(|mime_type| mime_type.trim().is_empty())
        {
            return Err("Host artifact MIME type must not be empty when present".to_string());
        }
        if self
            .operation
            .as_deref()
            .is_some_and(|operation| operation.trim().is_empty())
        {
            return Err("Host artifact operation must not be empty when present".to_string());
        }
        Ok(())
    }
}

pub fn validate_host_artifacts(artifacts: &[HostArtifact]) -> Result<(), String> {
    for artifact in artifacts {
        artifact.validate()?;
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http_status: Option<u16>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn generic_wire_envelopes_round_trip_without_core_types() {
        let request = HostRequestEnvelope {
            attempt_started: false,
            delegated_invocation_id: None,
            attempt_id: None,
            request_id: "request-1".to_string(),
            effect_id: HostEffectId("effect-1".to_string()),
            host_id: HostId("mac".to_string()),
            lease_id: Some("lease-1".to_string()),
            session_id: "session-1".to_string(),
            turn_id: Some("turn-1".to_string()),
            catalog_revision: 3,
            availability_epoch: 5,
            created_at_ms: 10,
            deadline_ms: Some(20),
            batch: Some(HostRequestBatchIdentity {
                batch_id: "batch-1".to_string(),
                index: 0,
                size: 2,
            }),
            resume_after_sequence: Some(7),
            delivery: Some(EffectDeliveryLease {
                attempt_id: "delivery-1".to_string(),
                worker_generation: "generation-1".to_string(),
                lease_owner: "runtime-1".to_string(),
                lease_token: "token-1".to_string(),
                lease_expires_at_ms: 30,
            }),
            payload: json!({"type": "model_stream"}),
        };

        let encoded = serde_json::to_string(&request).expect("encode request");
        let decoded: HostRequestEnvelope<serde_json::Value> =
            serde_json::from_str(&encoded).expect("decode request");
        assert_eq!(decoded, request);
        assert_eq!(
            decoded
                .delivery
                .as_ref()
                .map(|lease| lease.attempt_id.as_str()),
            Some("delivery-1")
        );
        assert!(!decoded.is_expired_at(19));
        assert!(decoded.is_expired_at(20));
    }

    #[test]
    fn permission_resolution_must_name_the_requested_permission() {
        let request: HostRequest<Value, Value, Value> = HostRequest::RequestPermission {
            allow_conversation: true,
            permission_instructions: Vec::new(),
            permission_request_id: "permission-current".into(),
            capability_id: "files.write".into(),
            resource: "/workspace/file".into(),
            reason: "write requested file".into(),
            input: Value::Null,
            evaluation: None,
        };
        for resolution in [
            HostPermissionResolution::Intervention {
                intervention: HostPermissionIntervention::ReturnToComposer,
            },
            HostPermissionResolution::Intervention {
                intervention: HostPermissionIntervention::ProvideInstruction {
                    instruction: "use another file".into(),
                },
            },
            HostPermissionResolution::Authorization {
                decision: HostPermissionAuthorizationDecision::AllowOnce,
                grant: None,
            },
        ] {
            for (request_id, accepted) in [
                ("permission-current", true),
                ("permission-old", false),
                ("", false),
            ] {
                let response: HostResponse<Value, Value, Value, Value, Value, Value> =
                    HostResponse::PermissionResolved {
                        request_id: request_id.into(),
                        resolution: resolution.clone(),
                    };
                assert_eq!(request.accepts_response(&response), accepted);
            }
        }
    }

    #[test]
    fn execution_rejection_is_a_typed_terminal_response() {
        type Response = HostResponse<Value, Value, TestState, Value, Value, Value>;
        #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
        enum TestState {
            Running,
        }
        impl HostTaskState for TestState {
            fn is_terminal(&self) -> bool {
                false
            }
        }

        let response: Response = HostResponse::ExecutionRejected {
            code: "deadline_exceeded".into(),
            message: "expired before execution".into(),
            retryable: false,
        };
        let wire = serde_json::to_value(&response).unwrap();
        assert_eq!(wire["type"], "execution_rejected");
        assert_eq!(serde_json::from_value::<Response>(wire).unwrap(), response);
        assert!(response.is_terminal());
    }

    #[test]
    fn concrete_payload_round_trips_without_core_types() {
        type Request = HostRequest<Value, Value, Value>;
        type Response = HostResponse<Value, Value, TestTaskState, Value, Value, Value>;

        #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
        enum TestTaskState {
            Running,
            Completed,
        }
        impl HostTaskState for TestTaskState {
            fn is_terminal(&self) -> bool {
                matches!(self, Self::Completed)
            }
        }

        let request = Request::ExecuteTool {
            permission_profile: crate::PermissionProfile::Cautious,
            capability_id: "read_file".to_string(),
            call_id: "call-1".to_string(),
            input: serde_json::json!({"path":"README.md"}),
            grant: None,
            permission_instructions: Vec::new(),
        };
        let mut missing_policy = serde_json::to_value(&request).unwrap();
        missing_policy.as_object_mut().unwrap().remove("permission_profile");
        assert!(serde_json::from_value::<Request>(missing_policy).is_err(),
            "an executable effect must not infer a missing admitted policy");
        let response = Response::ToolCompleted {
            call_id: "call-1".to_string(),
            output: serde_json::json!({"text":"ok"}),
            outcome: HostToolOutcome::Succeeded,
            activation: None,
            full_output_ref: None,
            artifacts: Vec::new(),
        };
        assert!(request.accepts_response(&response));
        assert!(response.is_terminal());
        let mut foreign_completion = response.clone();
        if let Response::ToolCompleted { call_id, .. } = &mut foreign_completion {
            *call_id = "another-call".into();
        }
        assert!(!request.accepts_response(&foreign_completion));
        for (call_id, accepted) in [("call-1", true), ("another-call", false), ("", false)] {
            let progress: Response = serde_json::from_value(json!({
                "type": "tool_call_progress", "call_id": call_id,
                "channel": "stdout", "delta": "progress"
            }))
            .unwrap();
            assert_eq!(request.accepts_response(&progress), accepted);
        }
        let promoted = Response::TaskPromoted {
            input_watermark: None,
            task_id: "task-1".to_string(),
            kind: Value::String("agent".to_string()),
            state: TestTaskState::Running,
            progress: Some(0.0),
            current_step: Some("queued".to_string()),
            lineage: Value::Null,
        };
        assert!(request.accepts_response(&promoted));
        assert!(
            promoted.is_terminal(),
            "promotion must settle the foreground Host request even while the Task is live"
        );
        let encoded = serde_json::to_string(&request).expect("encode request");
        let decoded: Request = serde_json::from_str(&encoded).expect("decode request");
        assert_eq!(decoded, request);
    }

    #[test]
    fn delegated_permission_delivery_requires_typed_transport_evidence() {
        struct Task;
        impl HostTaskState for Task {
            fn is_terminal(&self) -> bool {
                false
            }
        }
        type Response = HostResponse<Value, Value, Task>;
        let request: HostRequest = serde_json::from_value(json!({
            "type":"resolve_delegated_permission", "target_effect_id":"effect",
            "backend_id":"codex", "permission_request_id":"rpc",
            "occurrence":{"occurrence_id":"occurrence", "worker_generation":"generation"},
            "resolution":{"type":"authorization", "decision":"allow_once"}
        }))
        .unwrap();
        let claimed: Response = HostResponse::DelegatedPermissionClaimed {
            delivery_id: "delivery".into(),
        };
        let written: Response = HostResponse::DelegatedPermissionTransportWritten {
            delivery_id: "delivery".into(),
        };
        let interrupted: Response = HostResponse::DelegatedPermissionInterrupted {
            delivery_id: "delivery".into(),
        };
        assert!(request.accepts_response(&claimed));
        assert!(request.accepts_response(&written));
        assert!(request.accepts_response(&interrupted));
        assert!(!claimed.is_terminal());
        assert!(!interrupted.is_terminal());
        assert!(written.is_terminal());
        let legacy: Response = HostResponse::Completed {
            output: json!({"permission_resolution_delivered":true}),
            usage: None,
            full_output_ref: None,
            artifacts: vec![],
        };
        assert!(!request.accepts_response(&legacy));
        let mut event = json!({"type":"permission_requested", "request_id":"rpc", "call_id":"call", "capability_id":"shell", "resource":"resource", "reason":"fixture", "input":{}});
        assert!(serde_json::from_value::<DelegatedAgentEventV1>(event.clone()).is_err());
        event["occurrence"] = json!({"occurrence_id":"", "worker_generation":"generation"});
        assert!(serde_json::from_value::<DelegatedAgentEventV1>(event)
            .unwrap()
            .validate()
            .is_err());
    }

    #[test]
    fn delegated_agent_contract_contains_execution_facts_not_store_rows() {
        let request = DelegatedAgentRequestV1 {
            contract_version: DELEGATED_AGENT_CONTRACT_VERSION,
            backend_id: "codex".to_string(),
            backend_session_id: Some("thread-1".to_string()),
            model_id: Some("gpt-5.6-codex".to_string()),
            text_part_id: "assistant-text-1".to_string(),
            reasoning_part_id: "assistant-reasoning-1".to_string(),
            context: DelegatedAgentContextV1 {
                mode: DelegatedAgentContextMode::Resume,
                base_anchor_item_id: Some("user-previous".to_string()),
                target_anchor_item_id: "assistant-text-1".to_string(),
                identity_digest: "prompt-contract-v1".to_string(),
                messages: vec![json!({"role":"user","content":"Inspect the project"})],
            },
            tools: vec![json!({"name":"read_file"})],
            settings: json!({"workspace":"/workspace"}),
        };
        assert_eq!(request.validate(), Ok(()));
        let encoded = serde_json::to_value(&request).expect("encode delegated request");
        assert!(encoded.get("message_id").is_none());
        assert!(encoded.get("run_id").is_none());
        assert!(encoded.get("parts").is_none());

        let tool = DelegatedAgentEventV1::ToolCompleted {
            call_id: "call-1".to_string(),
            capability_id: "read_file".to_string(),
            output: json!({"text":"contents"}),
            outcome: HostToolOutcome::Succeeded,
            full_output_ref: None,
            artifacts: vec![HostArtifact {
                uri: "file:///workspace/README.md".to_string(),
                name: "README.md".to_string(),
                mime_type: Some("text/markdown".to_string()),
                operation: Some("read".to_string()),
            }],
        };
        assert_eq!(tool.validate(), Ok(()));
    }

    #[test]
    fn artifact_contract_accepts_encoded_absolute_uris_and_rejects_raw_paths() {
        let artifact = HostArtifact {
            uri: "file:///tmp/Nola%20%E6%8A%A5%E5%91%8A.docx".to_string(),
            name: "Nola 报告.docx".to_string(),
            mime_type: Some(
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
                    .to_string(),
            ),
            operation: Some("create".to_string()),
        };
        assert_eq!(artifact.validate(), Ok(()));

        let mut raw_path = artifact.clone();
        raw_path.uri = "/tmp/Nola 报告.docx".to_string();
        assert_eq!(
            raw_path.validate(),
            Err("Host artifact URI must be a canonical absolute URI".to_string())
        );

        let mut unencoded = artifact.clone();
        unencoded.uri = "file:///tmp/Nola 报告.docx".to_string();
        assert_eq!(
            unencoded.validate(),
            Err("Host artifact URI must be normalized and percent-encoded".to_string())
        );

        let mut unnamed = artifact;
        unnamed.name = "  ".to_string();
        assert_eq!(
            unnamed.validate(),
            Err("Host artifact name must not be empty".to_string())
        );
    }
}

#[cfg(test)]
mod permission_instruction_budget_tests {
    use super::*;
    fn entry(id_bytes: usize, instruction_bytes: usize) -> PermissionInstruction {
        PermissionInstruction {
            permission_request_id: "r".repeat(id_bytes),
            instruction: "x".repeat(instruction_bytes),
        }
    }
    #[test]
    fn append_limits_count_utf8_and_associated_request_ids_exactly() {
        assert!(PermissionInstruction::validate_append(&[], "r", &"x".repeat(4096)).is_ok());
        assert!(PermissionInstruction::validate_append(&[], "r", &"x".repeat(4097)).is_err());
        assert!(PermissionInstruction::validate_append(&[], "r", &"字".repeat(1366)).is_err());
        assert!(
            PermissionInstruction::validate_append(&vec![entry(1, 1); 15], "r", "last").is_ok()
        );
        assert!(
            PermissionInstruction::validate_append(&vec![entry(1, 1); 16], "r", "extra").is_err()
        );
        let history = vec![entry(1, 4095); 3]; // 12288 bytes, including original request IDs.
        assert!(PermissionInstruction::validate_append(&history, "r", &"x".repeat(4095)).is_ok());
        assert!(PermissionInstruction::validate_append(&history, "rr", &"x".repeat(4095)).is_err());
        assert!(PermissionInstruction::validate_append(&history, "r", &"x".repeat(4096)).is_err());
    }
    #[test]
    fn whitespace_and_rejected_append_do_not_rewrite_prior_guidance() {
        let history = vec![entry(1, 32)];
        let before = history.clone();
        assert!(PermissionInstruction::validate_append(&history, "r", " \n\t ").is_err());
        assert!(PermissionInstruction::validate_append(&history, "r", &"x".repeat(4097)).is_err());
        assert_eq!(history, before);
        assert!(
            PermissionInstruction::validate_append(
                &history,
                "r",
                &format!("  {} \n", "x".repeat(4096))
            )
            .is_ok(),
            "same trimming contract as Core"
        );
    }
    #[test]
    fn instruction_record_never_serializes_as_authorization_or_business_input() {
        let record = entry(3, 5);
        let value = serde_json::to_value(&record).unwrap();
        assert_eq!(value.as_object().unwrap().len(), 2);
        assert!(value.get("grant").is_none());
        assert!(value.get("input").is_none());
        assert!(serde_json::from_value::<PermissionInstruction>(
            serde_json::json!({"permission_request_id":"r","instruction":"help","grant":true})
        )
        .is_err());
        let resolution = HostPermissionResolution::provide_instruction("help");
        assert!(matches!(
            resolution,
            HostPermissionResolution::Intervention { .. }
        ));
    }
}

#[cfg(test)]
mod model_attempt_wire_cutover_tests {
    #[test]
    fn unowned_stream_reset_is_not_a_model_sampling_admission() {
        assert!(serde_json::from_value::<super::HostResponse>(
            serde_json::json!({"type": "model_stream_restarted"})
        )
        .is_err());
        assert!(serde_json::from_value::<super::HostResponse>(
            serde_json::json!({"type": "model_attempt_started", "provider": "target", "model": "model", "reason": "initial"})
        ).is_ok());
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DelegatedPermissionOccurrence {
    pub occurrence_id: String,
    pub worker_generation: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DelegatedPermissionDeliveryTarget {
    pub delivery_id: String,
    pub invocation_id: String,
    pub worker_generation: String,
    pub occurrence_id: String,
    pub call_id: String,
    pub request_digest: String,
}
