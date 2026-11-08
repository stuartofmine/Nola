// SPDX-License-Identifier: MIT

//! Stable, UI-neutral conversation projection contracts.
//!
//! These types are the only semantic input that platform presenters should
//! consume. Provider deltas, Host events, and diagnostic traces must first be
//! reduced and committed by Core and the application layer.

use crate::{
    ArtifactReference, HostCapabilityOrigin, ModelAttemptFailure, SourceEvidence, SourceLocation,
    TextReviewState, ToolOutputReference, ToolPermissionDisplay,
};
use serde::{Deserialize, Serialize};

macro_rules! domain_id {
    ($name:ident) => {
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub String);
    };
}

domain_id!(ConversationId);
domain_id!(MessageId);
domain_id!(MessagePartId);
domain_id!(RunId);
domain_id!(RunAttemptId);
domain_id!(RunEventId);
domain_id!(ToolCallId);
domain_id!(CapabilityId);
domain_id!(AttachmentId);
domain_id!(ArtifactId);
domain_id!(EffectId);
domain_id!(PermissionGrantId);
domain_id!(ContextCheckpointId);
domain_id!(MemoryRecordId);
domain_id!(ClientId);
domain_id!(RequestId);

/// Durable conversation source identity, independent of active execution.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ConversationSourceOwner {
    Local,
    Remote { source_host_id: String, source_store_epoch: String },
    Unavailable,
}

/// Source-owned execution admission, independent from transcript liveness.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ConversationExecutionOwnership {
    User,
    TaskChild { parent_conversation_id: String, parent_run_id: String, task_id: String, child_conversation_id: String },
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationProjection {
    pub execution_ownership: ConversationExecutionOwnership,
    pub contract_version: u32,
    pub conversation_id: ConversationId,
    pub revision: u64,
    pub title: String,
    pub messages: Vec<MessageView>,
    pub active_run: Option<RunActivityView>,
    #[serde(default)]
    pub surface: ConversationSurfaceView,
    pub context_usage: ContextUsageView,
}

/// UI-neutral lifecycle semantics for rendering a conversation.
///
/// Platforms keep their native styling and animation, but they must not infer
/// transcript ownership independently from raw message and Part states. A
/// conversation has immutable committed history, at most one mutable live
/// assistant message, and at most one current activity Part. Terminal activity
/// Parts remain available in the canonical projection for an expandable audit
/// trail without becoming an unbounded primary transcript.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationSurfaceView {
    pub revision: u64,
    pub committed_message_ids: Vec<MessageId>,
    pub live_message_id: Option<MessageId>,
    pub current_activity_part_id: Option<MessagePartId>,
    pub archived_activity_part_ids: Vec<MessagePartId>,
    pub show_assistant_placeholder: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    User,
    Assistant,
    System,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageState {
    Pending,
    Running,
    Completed,
    Cancelled,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageView {
    pub message_id: MessageId,
    pub role: MessageRole,
    pub state: MessageState,
    pub revision: u64,
    pub created_at_ms: i64,
    pub parts: Vec<MessagePartView>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MessagePartView {
    /// Core-validated interaction: summary is display-only; envelope is lossless model context.
    UiAction {
        part_id: MessagePartId,
        revision: u64,
        surface: crate::generative_ui::UiSurfaceRevisionRef,
        envelope: serde_json::Value,
        summary: String,
    },
    /// A durable reference, never a copied component tree or executable action.
    UiSurface {
        part_id: MessagePartId,
        revision: u64,
        surface: crate::generative_ui::UiSurfaceRevisionRef,
        summary: String,
    },
    /// Canonical historical interpretation; never a live permission/tool request.
    Historical {
        part_id: MessagePartId,
        revision: u64,
        projection: Box<crate::HistoricalPartProjection>,
    },
    Text {
        part_id: MessagePartId,
        revision: u64,
        markdown: String,
        is_final: bool,
        review: Option<TextReviewState>,
    },
    Reasoning {
        part_id: MessagePartId,
        revision: u64,
        state: ActivityState,
        title: String,
        summary: Option<String>,
    },
    Tool {
        execution_source: crate::code_mode::CodeItemSource,
        part_id: MessagePartId,
        revision: u64,
        tool_call_id: ToolCallId,
        capability_id: CapabilityId,
        state: ActivityState,
        display_key: Option<String>,
        title: String,
        summary: Option<String>,
        origin: HostCapabilityOrigin,
        permission: Option<ToolPermissionDisplay>,
        input: StructuredValue,
        /// The structured result; a textual preview is an independent field.
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "present_structured_value"
        )]
        output: Option<StructuredValue>,
        output_preview: Option<String>,
        full_output: Option<ToolOutputReference>,
        artifacts: Vec<ArtifactReference>,
        started_at_ms: Option<u64>,
        finished_at_ms: Option<u64>,
        affected_resources: Vec<AffectedResourceView>,
    },
    Source {
        part_id: MessagePartId,
        revision: u64,
        source_id: String,
        title: String,
        uri: String,
        mime_type: Option<String>,
        snippet: Option<String>,
        evidence: Option<SourceEvidence>,
    },
    Citation {
        part_id: MessagePartId,
        revision: u64,
        citation_id: String,
        source_id: String,
        title: String,
        uri: String,
        mime_type: Option<String>,
        origin_tool_call_id: String,
        /// Retain absence versus an explicitly empty location. The flat fields
        /// below are derived conveniences, not a second source of authority.
        location: Option<SourceLocation>,
        section: Option<String>,
        page: Option<u64>,
        line_start: Option<u64>,
        line_end: Option<u64>,
        range_start: Option<u64>,
        range_end: Option<u64>,
    },
    Attachment {
        part_id: MessagePartId,
        revision: u64,
        attachment_id: AttachmentId,
        name: String,
        mime_type: Option<String>,
        uri: String,
        operation: Option<String>,
    },
    Task {
        part_id: MessagePartId,
        revision: u64,
        task_id: String,
        display_key: String,
        state: ActivityState,
        progress: Option<f64>,
        current_step: Option<String>,
        result_ref: Option<String>,
        cancel_requested: bool,
    },
    Skill {
        part_id: MessagePartId,
        revision: u64,
        run_id: String,
        skill_id: String,
        display_name: String,
        mode: String,
        state: ActivityState,
        model_turns: u32,
        tool_calls: u32,
        elapsed_millis: u64,
        result_ref: Option<String>,
        failure: Option<String>,
    },
    /// Bounded presentation of one failed physical model sampling attempt.
    /// The Attempt ledger remains canonical; Application derives this Part.
    ModelAttempt {
        part_id: MessagePartId,
        revision: u64,
        state: ActivityState,
        attempt_id: RunAttemptId,
        provider: Option<String>,
        model: Option<String>,
        failure: ModelAttemptFailure,
        fallback_attempt_id: Option<RunAttemptId>,
        fallback_provider: Option<String>,
        fallback_model: Option<String>,
    },
    Compaction {
        part_id: MessagePartId,
        revision: u64,
        summary: String,
        retained_item_ids: Vec<String>,
        #[serde(default)]
        source_fingerprint: String,
        original_token_count: u64,
        compacted_token_count: u64,
    },
    Error {
        part_id: MessagePartId,
        revision: u64,
        code: String,
        display_key: String,
        recoverable: bool,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PublicPartContractError {
    WithheldText,
    ToolOutputOwnerMismatch,
    InvalidToolExecutionSource,
    InvalidUiSurfaceReference,
    CitationLocationMismatch,
    CitationRangeIncomplete,
    CitationRangeReversed,
}

impl std::fmt::Display for PublicPartContractError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::WithheldText => "withheld review cannot carry public text",
            Self::ToolOutputOwnerMismatch => "tool output reference has a different public owner",
            Self::InvalidUiSurfaceReference => "UI surface reference has an invalid owner or revision",
            Self::InvalidToolExecutionSource => "tool execution provenance has an invalid cell identity",
            Self::CitationLocationMismatch => "citation location disagrees with derived fields",
            Self::CitationRangeIncomplete => "citation range must have both bounds or neither",
            Self::CitationRangeReversed => "citation byte range must be ordered",
        })
    }
}

impl std::error::Error for PublicPartContractError {}

impl MessagePartView {
    /// Validate public facts before an externally supplied projection can be
    /// committed. A display descriptor is not an authorization capability.
    pub fn validate_public_semantics(
        &self,
        message_id: &MessageId,
    ) -> Result<(), PublicPartContractError> {
        if let Self::UiAction { surface, revision, envelope, summary, .. } = self {
            if *revision == 0 || (crate::generative_ui::UiActionContent {
                surface: surface.clone(), envelope: envelope.clone(), summary: summary.clone(),
            }).validate().is_err() {
                return Err(PublicPartContractError::InvalidUiSurfaceReference);
            }
        }
        if let Self::UiSurface { surface, revision, summary, .. } = self {
            if *revision == 0 || surface.validate().is_err() || surface.identity.owner.message_id != message_id.0
                || summary.len() > crate::generative_ui::UI_MAX_SUMMARY_BYTES {
                return Err(PublicPartContractError::InvalidUiSurfaceReference);
            }
        }
        if let Self::Tool { execution_source: crate::code_mode::CodeItemSource::CodeCell { cell }, .. } = self {
            cell.validate().map_err(|_| PublicPartContractError::InvalidToolExecutionSource)?;
        }
        match self {
            Self::Text {
                markdown, review, ..
            } if !markdown.is_empty()
                && matches!(review, Some(state) if *state != TextReviewState::Approved) =>
            {
                Err(PublicPartContractError::WithheldText)
            }
            Self::Tool {
                part_id,
                tool_call_id,
                full_output: Some(reference),
                ..
            } if reference.message_id != *message_id
                || reference.part_id != *part_id
                || reference.tool_call_id != *tool_call_id =>
            {
                Err(PublicPartContractError::ToolOutputOwnerMismatch)
            }
            Self::Citation {
                location,
                page,
                line_start,
                line_end,
                section,
                range_start,
                range_end,
                ..
            } => {
                if *page != location.as_ref().and_then(|value| value.page)
                    || *line_start != location.as_ref().and_then(|value| value.line_start)
                    || *line_end != location.as_ref().and_then(|value| value.line_end)
                    || section.as_deref()
                        != location.as_ref().and_then(|value| value.section.as_deref())
                {
                    return Err(PublicPartContractError::CitationLocationMismatch);
                }
                crate::CitationRange::from_public_bounds(*range_start, *range_end)
                    .map(|_| ())
                    .map_err(|error| match error {
                        crate::PublicCitationRangeError::Incomplete => {
                            PublicPartContractError::CitationRangeIncomplete
                        }
                        crate::PublicCitationRangeError::Reversed => {
                            PublicPartContractError::CitationRangeReversed
                        }
                    })
            }
            _ => Ok(()),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivityState {
    Pending,
    Running,
    WaitingForPermission,
    WaitingForInput,
    Completed,
    Sealed,
    Cancelled,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum StructuredValue {
    Null,
    Bool(bool),
    Number(serde_json::Number),
    String(String),
    Array(Vec<StructuredValue>),
    Object(std::collections::BTreeMap<String, StructuredValue>),
}

/// Serde's ordinary Option decoder collapses a present JSON null into None.
/// Field absence is handled by `default`; a present field is a real result,
/// including null, empty string, empty array and empty object.
fn present_structured_value<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<StructuredValue>, D::Error> {
    StructuredValue::deserialize(deserializer).map(Some)
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AffectedResourceView {
    pub resource_type: String,
    pub uri: String,
    pub operation: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunActivityView {
    pub run_id: RunId,
    pub state: ActivityState,
    pub active_part_id: Option<MessagePartId>,
    pub display_key: Option<String>,
    pub progress: Option<f64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextUsageView {
    pub measurement: Option<crate::ContextUsageMeasurement>,
    pub prompt_cache_measurement: Option<crate::ContextUsageMeasurement>,
    pub prompt_cache_diagnostic: Option<crate::PromptCacheDiagnostic>,
    /// Token-weighted totals from the durable conversation request ledger.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_cache_usage: Option<crate::ConversationCacheUsage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_usage: Option<crate::PromptCacheUsage>,
    pub used_tokens: u64,
    pub context_window_tokens: u64,
    pub prompt_token_limit: u64,
    pub estimated: bool,
    pub measured_at_revision: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationSummaryView {
    pub conversation_id: ConversationId,
    pub title: String,
    pub revision: u64,
    pub message_count: u64,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub project_root: Option<String>,
    pub first_user_message: Option<String>,
    pub last_user_message_at_ms: Option<i64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationListProjectionEventIdentity {
    pub event_id: RunEventId,
    pub conversation_id: ConversationId,
    pub sequence: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ConversationListProjectionMutation {
    Upsert {
        conversation: ConversationSummaryView,
    },
    Remove {
        conversation_id: ConversationId,
        revision: u64,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationListProjectionEvent {
    pub identity: ConversationListProjectionEventIdentity,
    pub mutation: ConversationListProjectionMutation,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationProjectionEventIdentity {
    pub event_id: RunEventId,
    pub conversation_id: ConversationId,
    pub run_id: Option<RunId>,
    pub attempt_id: Option<RunAttemptId>,
    pub delegated_invocation_id: Option<String>,
    pub sequence: u64,
}

/// Physical model sampling and opaque external execution have distinct identities.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProjectionExecutionOwner {
    ModelAttempt { attempt_id: RunAttemptId },
    DelegatedInvocation { invocation_id: String },
}

impl ProjectionExecutionOwner {
    pub fn identity(&self) -> &str {
        match self {
            Self::ModelAttempt { attempt_id } => &attempt_id.0,
            Self::DelegatedInvocation { invocation_id } => invocation_id,
        }
    }
}

/// Canonical ownership required to restore a mutable text Part.
/// Application supplies these IDs; presenters must not infer them from text,
/// the latest active run, or event arrival order.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectionStreamOwner {
    pub message_id: MessageId,
    pub part_id: MessagePartId,
    pub run_id: RunId,
    pub execution: ProjectionExecutionOwner,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ConversationProjectionMutation {
    /// Bulk canonical changes invalidate a presenter window without broadcasting
    /// the transcript. Clients must restore an authoritative snapshot and cursor.
    InvalidateProjection {
        revision: u64,
    },
    SetTitle {
        title: String,
        revision: u64,
    },
    UpsertMessage {
        message: MessageView,
    },
    RemoveMessage {
        message_id: MessageId,
        revision: u64,
    },
    AppendTextPreview {
        message_id: MessageId,
        part_id: MessagePartId,
        revision: u64,
        delta: String,
    },
    CompleteTextPart {
        message_id: MessageId,
        part_id: MessagePartId,
        revision: u64,
        markdown: String,
    },
    SetActiveRun {
        active_run: Option<RunActivityView>,
    },
    ReplaceSurface {
        surface: ConversationSurfaceView,
    },
    SetContextUsage {
        context_usage: ContextUsageView,
    },
}

impl ConversationProjectionMutation {
    pub fn message_id(&self) -> Option<&MessageId> {
        match self {
            Self::InvalidateProjection { .. } => None,
            Self::SetTitle { .. } => None,
            Self::UpsertMessage { message } => Some(&message.message_id),
            Self::RemoveMessage { message_id, .. } => Some(message_id),
            Self::AppendTextPreview { message_id, .. }
            | Self::CompleteTextPart { message_id, .. } => Some(message_id),
            Self::SetActiveRun { .. }
            | Self::ReplaceSurface { .. }
            | Self::SetContextUsage { .. } => None,
        }
    }

    pub fn part_id(&self) -> Option<&MessagePartId> {
        match self {
            Self::InvalidateProjection { .. } => None,
            Self::AppendTextPreview { part_id, .. } | Self::CompleteTextPart { part_id, .. } => {
                Some(part_id)
            }
            Self::SetTitle { .. }
            | Self::UpsertMessage { .. }
            | Self::RemoveMessage { .. }
            | Self::SetActiveRun { .. }
            | Self::ReplaceSurface { .. }
            | Self::SetContextUsage { .. } => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationProjectionEvent {
    pub identity: ConversationProjectionEventIdentity,
    pub mutation: ConversationProjectionMutation,
}

#[cfg(test)]
mod tests {
    use super::ConversationProjection;

    fn projection_with_tool(tool_fields: &str) -> String {
        format!(
            r#"{{
                "execution_ownership": {{"kind": "user"}},
                "contract_version": 1,
                "conversation_id": "conversation-1",
                "revision": 3,
                "title": "Contract",
                "messages": [{{
                    "message_id": "message-1",
                    "role": "assistant",
                    "state": "running",
                    "revision": 2,
                    "created_at_ms": 1787356800000,
                    "parts": [{{
                        "kind": "tool",
                        "execution_source": {{"kind": "model"}},
                        "part_id": "part-1",
                        "revision": 1,
                        "tool_call_id": "call-1",
                        {tool_fields},
                        "state": "running",
                        "display_key": "tool.file.write",
                        "title": "write_file",
                        "origin": {{"kind": "builtin"}},
                        "input": {{"path": "/tmp/a.txt"}},
                        "output": null,
                        "artifacts": [],
                        "affected_resources": []
                    }}]
                }}],
                "active_run": null,
                "context_usage": {{
                    "used_tokens": 120,
                    "context_window_tokens": 65536,
                    "prompt_token_limit": 57344,
                    "estimated": false,
                    "measured_at_revision": 3
                }}
            }}"#
        )
    }

    #[test]
    fn tool_part_requires_canonical_capability_id() {
        let valid = projection_with_tool(r#""capability_id": "file.write""#);
        assert!(serde_json::from_str::<ConversationProjection>(&valid).is_ok());

        let missing = projection_with_tool(r#""summary": "write_file""#);
        assert!(serde_json::from_str::<ConversationProjection>(&missing).is_err());

        let alias = projection_with_tool(r#""capabilityId": "file.write""#);
        assert!(serde_json::from_str::<ConversationProjection>(&alias).is_err());
    }

    #[test]
    fn context_usage_rejects_client_specific_fields() {
        let valid = projection_with_tool(r#""capability_id": "file.write""#);
        let invalid = valid.replace(
            r#""measured_at_revision": 3"#,
            r#""measured_at_revision": 3, "message_count": 12"#,
        );
        assert!(serde_json::from_str::<ConversationProjection>(&invalid).is_err());
    }
}
