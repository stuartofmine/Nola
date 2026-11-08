// SPDX-License-Identifier: Apache-2.0
//! Shared public Part facts. These are display evidence, never execution authority.
//!
//! Core uses these same values in its private execution state. Application selects
//! their public fields, and presenters/journals retain their independent semantics.
//! No provider payload, memory body, credential or private output locator belongs here.

use crate::{MessageId, MessagePartId, ToolCallId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextReviewState {
    Pending,
    Approved,
    Rejected,
    Interrupted,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostCapabilityOrigin {
    #[serde(default)]
    pub kind: HostCapabilityOriginKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostCapabilityOriginKind {
    #[default]
    Unknown,
    Builtin,
    Skill,
    Mcp,
    Device,
    Extension,
    Internal,
}

/// Read-only linkage to an approval in the execution ledger. This is not an
/// approval request, grant, scoped token, lease or permission to execute a tool.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolPermissionDisplay {
    pub request_id: String,
    pub permission_domain: String,
    pub rationale: String,
    #[serde(default)]
    pub decision: Option<String>,
}

/// Owner-bound indication that the canonical tool has a separately retained
/// result. A private Host path is not a public read handle. Any future read
/// operation must revalidate this exact owner and current disclosure policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolOutputReference {
    pub message_id: MessageId,
    pub part_id: MessagePartId,
    pub tool_call_id: ToolCallId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactReference {
    /// Opaque read-only display fact. Empty/non-URL values do not authorize a
    /// link, local read or tool execution; readers validate their own handles.
    pub uri: String,
    pub name: String,
    pub mime_type: Option<String>,
    pub operation: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceLocation {
    #[serde(default)]
    pub page: Option<u64>,
    #[serde(default)]
    pub line_start: Option<u64>,
    #[serde(default)]
    pub line_end: Option<u64>,
    #[serde(default)]
    pub section: Option<String>,
}

// Location coordinates are independent source facts, not an execution range:
// preserve missing endpoints, zero, reversed observations and Some(empty).
// Public URI strings are likewise opaque UTF-8 facts, including empty/blank
// values. Neither ordinary projections nor journal metadata add URI schemes,
// nonempty requirements or a line-order constraint not present in Core.

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CitationRange {
    pub start: u64,
    pub end: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicCitationRangeError {
    Incomplete,
    Reversed,
}

impl CitationRange {
    /// Shared public byte-range domain. Unlike independent location endpoints,
    /// a byte range is paired and ordered. No normalization or inferred bound.
    pub fn from_public_bounds(
        start: Option<u64>,
        end: Option<u64>,
    ) -> Result<Option<Self>, PublicCitationRangeError> {
        match (start, end) {
            (None, None) => Ok(None),
            (Some(start), Some(end)) if start <= end => Ok(Some(Self { start, end })),
            (Some(_), Some(_)) => Err(PublicCitationRangeError::Reversed),
            _ => Err(PublicCitationRangeError::Incomplete),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryScope {
    Turn,
    Conversation,
    #[default]
    Global,
}

/// Explicit ownership, not a namespace inferred from user language.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(tag = "kind", content = "name", rename_all = "snake_case")]
pub enum MemoryNamespace {
    #[default]
    Personal,
    Workspace,
    Conversation,
    Custom(String),
}

impl<'de> Deserialize<'de> for MemoryNamespace {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // Serde's tagged unit variants can otherwise ignore extra fields.
        // Use actual strict empty structs for the three fieldless namespaces.
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct EmptyNamespace {}
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct CustomNamespace {
            name: String,
        }
        #[derive(Deserialize)]
        #[serde(tag = "kind", rename_all = "snake_case")]
        enum Wire {
            Personal(EmptyNamespace),
            Workspace(EmptyNamespace),
            Conversation(EmptyNamespace),
            Custom(CustomNamespace),
        }
        Ok(match Wire::deserialize(deserializer)? {
            Wire::Personal(_) => Self::Personal,
            Wire::Workspace(_) => Self::Workspace,
            Wire::Conversation(_) => Self::Conversation,
            Wire::Custom(value) => Self::Custom(value.name),
        })
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryProvenanceKind {
    #[default]
    ExplicitUser,
    AgentInference,
    ToolResult,
    Imported,
    Synced,
    Other,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemorySourceRole {
    User,
    Assistant,
    Tool,
    System,
    #[default]
    Other,
}

/// Evidence of a component included in a request, without memory body/summary.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryUsage {
    pub record_id: String,
    pub component_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<MemoryScope>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<MemoryNamespace>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance_kind: Option<MemoryProvenanceKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_role: Option<MemorySourceRole>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_time: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range_start: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range_end: Option<String>,
    #[serde(default)]
    pub requires_current_state_verification: bool,
    #[serde(default)]
    pub truncated: bool,
    #[serde(default)]
    pub supersedes: Vec<String>,
    #[serde(default)]
    pub usage_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used_at_epoch_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SourceEvidence {
    Memory { usage: MemoryUsage },
}

#[cfg(test)]
#[path = "public_part_semantics_tests.rs"]
mod tests;
