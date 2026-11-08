// SPDX-License-Identifier: Apache-2.0

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use super::host::HostCapabilityOrigin;
pub use nola_domain::{
    StepStatus, ToolPartState,
    ArtifactReference, CitationRange, SourceEvidence, SourceLocation, TextReviewState,
    ToolPermissionDisplay as ToolPermissionPart,
};

/// UI projections must remain cheap and stack-safe on every native client.
/// The canonical transcript and `full_output_ref` retain the complete tool
/// result; only recursively nested dynamic values in the presentation copy
/// are replaced with an explicit structural marker.
const MAX_PROJECTED_JSON_NESTING_DEPTH: usize = 48;

fn bounded_projection_json(value: &serde_json::Value) -> serde_json::Value {
    fn copy(value: &serde_json::Value, depth: usize) -> serde_json::Value {
        match value {
            serde_json::Value::Array(_) | serde_json::Value::Object(_)
                if depth >= MAX_PROJECTED_JSON_NESTING_DEPTH =>
            {
                serde_json::json!({
                    "_nola_projection": "truncated",
                    "reason": "maximum_nesting_depth"
                })
            }
            serde_json::Value::Array(values) => serde_json::Value::Array(
                values.iter().map(|value| copy(value, depth + 1)).collect(),
            ),
            serde_json::Value::Object(values) => serde_json::Value::Object(
                values
                    .iter()
                    .map(|(key, value)| (key.clone(), copy(value, depth + 1)))
                    .collect(),
            ),
            _ => value.clone(),
        }
    }

    copy(value, 0)
}

/// Stable UI-facing projection of an Agent response.
///
/// Provider chunks are never rendered directly. Core first applies them to a
/// stable part, and clients render the resulting projection by `part_id`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ConversationProjection {
    /// Core-applied input frontier; renderers never choose or move message owners.
    pub active_message_segments: BTreeMap<String, String>,
    pub activity_message_owners: BTreeMap<String, String>,
    pub revision: u64,
    pub order: Vec<String>,
    pub parts: BTreeMap<String, AgentPart>,
}

/// Immutable ownership carried by every projected Part.
///
/// `id` identifies the Part itself. `message_id` groups text, reasoning,
/// tools, artifacts, steps, and compaction metadata into one assistant
/// message, while `turn_id` keeps that message attached to its originating
/// Turn. Hosts can therefore restore a multi-Turn UI from the projection
/// snapshot alone instead of inferring ownership from event arrival order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartOwnership {
    pub message_id: String,
    pub turn_id: String,
}

impl PartOwnership {
    pub fn for_turn(turn_id: impl Into<String>) -> Self {
        let turn_id = turn_id.into();
        Self {
            message_id: format!("{turn_id}:assistant"),
            turn_id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentPart {
    Text(TextPart),
    Reasoning(ReasoningPart),
    Tool(ToolPart),
    Source(SourcePart),
    Citation(CitationPart),
    Artifact(ArtifactPart),
    Step(StepPart),
    Compaction(CompactionPart),
}

impl AgentPart {
    pub fn id(&self) -> &str {
        match self {
            Self::Text(part) => &part.id,
            Self::Reasoning(part) => &part.id,
            Self::Tool(part) => &part.id,
            Self::Source(part) => &part.id,
            Self::Citation(part) => &part.id,
            Self::Artifact(part) => &part.id,
            Self::Step(part) => &part.id,
            Self::Compaction(part) => &part.id,
        }
    }

    pub fn ownership(&self) -> &PartOwnership {
        match self {
            Self::Text(part) => &part.ownership,
            Self::Reasoning(part) => &part.ownership,
            Self::Tool(part) => &part.ownership,
            Self::Source(part) => &part.ownership,
            Self::Citation(part) => &part.ownership,
            Self::Artifact(part) => &part.ownership,
            Self::Step(part) => &part.ownership,
            Self::Compaction(part) => &part.ownership,
        }
    }

    /// Returns the same projected Part under a Host-owned canonical Message.
    /// The Turn identity and Part identity remain unchanged.
    pub fn with_message_id(mut self, message_id: impl Into<String>) -> Self {
        let message_id = message_id.into();
        match &mut self {
            Self::Text(part) => part.ownership.message_id = message_id.clone(),
            Self::Reasoning(part) => part.ownership.message_id = message_id.clone(),
            Self::Tool(part) => part.ownership.message_id = message_id.clone(),
            Self::Source(part) => part.ownership.message_id = message_id.clone(),
            Self::Citation(part) => part.ownership.message_id = message_id.clone(),
            Self::Artifact(part) => part.ownership.message_id = message_id.clone(),
            Self::Step(part) => part.ownership.message_id = message_id.clone(),
            Self::Compaction(part) => part.ownership.message_id = message_id,
        }
        self
    }

    pub fn message_id(&self) -> &str {
        &self.ownership().message_id
    }

    pub fn turn_id(&self) -> &str {
        &self.ownership().turn_id
    }

    pub fn is_terminal(&self) -> bool {
        match self {
            Self::Text(part) => part.is_complete && part.review != Some(TextReviewState::Pending),
            Self::Reasoning(part) => part.is_complete,
            Self::Tool(part) => part.state.is_terminal(),
            Self::Source(_)
            | Self::Citation(_)
            | Self::Artifact(_)
            | Self::Step(_)
            | Self::Compaction(_) => true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextPart {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review: Option<TextReviewState>,
    pub id: String,
    #[serde(flatten)]
    pub ownership: PartOwnership,
    pub text: String,
    pub is_complete: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReasoningPart {
    pub id: String,
    #[serde(flatten)]
    pub ownership: PartOwnership,
    pub title: String,
    pub detail: String,
    pub is_complete: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolPart {
    pub execution_source: nola_domain::code_mode::CodeItemSource,
    pub id: String,
    #[serde(flatten)]
    pub ownership: PartOwnership,
    pub call_id: String,
    pub capability_id: String,
    pub title: String,
    /// Human-readable outcome summary owned by the Host result contract.
    /// Clients must not reconstruct this value from serialized tool output.
    #[serde(default)]
    pub summary: Option<String>,
    /// Immutable Host-declared provenance captured with conversation history.
    #[serde(default)]
    pub origin: HostCapabilityOrigin,
    pub state: ToolPartState,
    pub input: serde_json::Value,
    #[serde(default)]
    pub raw_input: String,
    #[serde(default)]
    pub permission: Option<ToolPermissionPart>,
    pub output_preview: Option<String>,
    /// Host-bounded structured result. Clients render this directly instead
    /// of reverse-parsing `output_preview`; large payloads retain their
    /// `full_output_ref` beside this value.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present_json_value"
    )]
    pub structured_output: Option<serde_json::Value>,
    pub full_output_ref: Option<String>,
    pub artifacts: Vec<ArtifactReference>,
    pub started_at_ms: Option<u64>,
    pub finished_at_ms: Option<u64>,
}

fn present_json_value<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<serde_json::Value>, D::Error> {
    serde_json::Value::deserialize(deserializer).map(Some)
}

/// A durable piece of evidence identified independently from the tool call
/// that discovered it. Repeated citations to the same normalized URI reuse
/// the same `source_id`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourcePart {
    pub id: String,
    #[serde(flatten)]
    pub ownership: PartOwnership,
    pub source_id: String,
    pub title: String,
    pub uri: String,
    #[serde(default)]
    pub mime_type: Option<String>,
    /// Optional typed, redacted evidence carried by a source. Clients may
    /// expose provenance without loading or parsing the underlying content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<SourceEvidence>,
}

/// One concrete citation of a Source by a tool result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CitationPart {
    pub id: String,
    #[serde(flatten)]
    pub ownership: PartOwnership,
    pub citation_id: String,
    pub source_id: String,
    pub title: String,
    pub uri: String,
    #[serde(default)]
    pub mime_type: Option<String>,
    #[serde(default)]
    pub location: Option<SourceLocation>,
    #[serde(default)]
    pub range: Option<CitationRange>,
    pub origin_tool_call_id: String,
}


#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactPart {
    pub id: String,
    #[serde(flatten)]
    pub ownership: PartOwnership,
    pub artifact: ArtifactReference,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepPart {
    pub id: String,
    #[serde(flatten)]
    pub ownership: PartOwnership,
    pub title: String,
    pub status: StepStatus,
}


#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompactionPart {
    pub id: String,
    #[serde(flatten)]
    pub ownership: PartOwnership,
    pub summary: String,
    pub retained_item_ids: Vec<String>,
    /// SHA-256 over the ordered canonical source items summarized here.
    #[serde(default)]
    pub source_fingerprint: String,
    pub original_token_count: u64,
    pub compacted_token_count: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PartUpdate {
    TextReviewChanged {
        part_id: String,
        state: TextReviewState,
    },
    Created {
        part: AgentPart,
    },
    TextDelta {
        part_id: String,
        delta: String,
    },
    /// Appends a Core-approved semantic presentation unit. The Part remains
    /// open for later units; clients may render the cumulative snapshot.
    TextCommittedDelta {
        part_id: String,
        delta: String,
    },
    TextReplaced {
        part_id: String,
        text: String,
        is_complete: bool,
    },
    ReasoningDelta {
        part_id: String,
        title: Option<String>,
        detail_delta: String,
    },
    ReasoningReplaced {
        part_id: String,
        title: String,
        detail: String,
        is_complete: bool,
    },
    ToolInputDelta {
        part_id: String,
        delta: String,
    },
    ToolInputReplaced {
        part_id: String,
        input: serde_json::Value,
        raw_input: String,
        state: ToolPartState,
    },
    ToolInputResolved {
        part_id: String,
        input: serde_json::Value,
    },
    ToolPermission {
        part_id: String,
        permission: ToolPermissionPart,
    },
    /// Updates a running tool in place. `detail` is a replace operation while
    /// `output_delta` appends to a bounded UI-only preview.
    ToolProgress {
        part_id: String,
        title: Option<String>,
        detail: Option<String>,
        output_delta: Option<String>,
    },
    ToolState {
        part_id: String,
        state: ToolPartState,
        title: Option<String>,
        summary: Option<String>,
        output_preview: Option<String>,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "present_json_value"
        )]
        structured_output: Option<serde_json::Value>,
        full_output_ref: Option<String>,
        artifacts: Vec<ArtifactReference>,
        timestamp_ms: Option<u64>,
    },
    Completed {
        part_id: String,
    },
}

impl PartUpdate {
    pub fn part_id(&self) -> &str {
        match self {
            Self::Created { part } => part.id(),
            Self::TextDelta { part_id, .. }
            | Self::TextReviewChanged { part_id, .. }
            | Self::TextCommittedDelta { part_id, .. }
            | Self::TextReplaced { part_id, .. }
            | Self::ReasoningDelta { part_id, .. }
            | Self::ReasoningReplaced { part_id, .. }
            | Self::ToolInputDelta { part_id, .. }
            | Self::ToolInputReplaced { part_id, .. }
            | Self::ToolInputResolved { part_id, .. }
            | Self::ToolPermission { part_id, .. }
            | Self::ToolProgress { part_id, .. }
            | Self::ToolState { part_id, .. }
            | Self::Completed { part_id } => part_id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectionError {
    DuplicatePart(String),
    MissingPart(String),
    PartKindMismatch(String),
    TerminalPart(String),
}

impl ConversationProjection {
    /// Applies one normalized Core update while preserving node identity.
    ///
    /// The boolean result tells presenters whether visible content changed.
    /// Cursor-only events may advance elsewhere without forcing a UI publish.
    pub fn apply(&mut self, update: PartUpdate) -> Result<bool, ProjectionError> {
        let changed = match update {
            PartUpdate::TextReviewChanged { part_id, state } => match self.part_mut(&part_id)? {
                AgentPart::Text(part) if part.review == Some(state) && part.is_complete => false,
                AgentPart::Text(part) if part.review == Some(TextReviewState::Pending) => {
                    part.review = Some(state);
                    part.is_complete = true;
                    true
                }
                AgentPart::Text(_) => return Err(ProjectionError::TerminalPart(part_id)),
                _ => return Err(ProjectionError::PartKindMismatch(part_id)),
            },
            PartUpdate::Created { mut part } => {
                if let AgentPart::Tool(tool) = &mut part {
                    tool.input = bounded_projection_json(&tool.input);
                    tool.structured_output =
                        tool.structured_output.as_ref().map(bounded_projection_json);
                }
                let part_id = part.id().to_owned();
                if let Some(existing) = self.parts.get(&part_id) {
                    if existing == &part {
                        return Ok(false);
                    }
                    return Err(ProjectionError::DuplicatePart(part_id));
                }
                self.order.push(part_id.clone());
                self.parts.insert(part_id, part);
                true
            }
            PartUpdate::TextDelta { part_id, delta } => {
                if delta.is_empty() {
                    false
                } else {
                    let part = self.part_mut(&part_id)?;
                    match part {
                        AgentPart::Text(part) if !part.is_complete => {
                            part.text.push_str(&delta);
                            true
                        }
                        // A provider or Host transport may flush an already
                        // buffered text chunk after the authoritative
                        // completion event. The projection is presentation
                        // state: keep the completed body immutable and treat
                        // the late chunk as an idempotent no-op instead of
                        // failing the whole Agent turn.
                        AgentPart::Text(_) => false,
                        _ => return Err(ProjectionError::PartKindMismatch(part_id)),
                    }
                }
            }
            PartUpdate::TextCommittedDelta { part_id, delta } => {
                if delta.is_empty() {
                    false
                } else {
                    let part = self.part_mut(&part_id)?;
                    match part {
                        AgentPart::Text(part) if !part.is_complete => {
                            part.text.push_str(&delta);
                            true
                        }
                        AgentPart::Text(_) => false,
                        _ => return Err(ProjectionError::PartKindMismatch(part_id)),
                    }
                }
            }
            PartUpdate::TextReplaced {
                part_id,
                text,
                is_complete,
            } => {
                let part = self.part_mut(&part_id)?;
                match part {
                    AgentPart::Text(part) => {
                        let changed = part.text != text || part.is_complete != is_complete;
                        part.text = text;
                        part.is_complete = is_complete;
                        changed
                    }
                    _ => return Err(ProjectionError::PartKindMismatch(part_id)),
                }
            }
            PartUpdate::ReasoningDelta {
                part_id,
                title,
                detail_delta,
            } => {
                let part = self.part_mut(&part_id)?;
                match part {
                    AgentPart::Reasoning(part) if !part.is_complete => {
                        let mut changed = false;
                        if let Some(title) = title {
                            if title != part.title {
                                part.title = title;
                                changed = true;
                            }
                        }
                        if !detail_delta.is_empty() {
                            part.detail.push_str(&detail_delta);
                            changed = true;
                        }
                        changed
                    }
                    // Streaming transports may deliver a buffered reasoning
                    // fragment after the authoritative completion event.
                    // Completed presentation state is immutable; ignore the
                    // stale fragment instead of failing the entire turn.
                    AgentPart::Reasoning(_) => false,
                    _ => return Err(ProjectionError::PartKindMismatch(part_id)),
                }
            }
            PartUpdate::ReasoningReplaced {
                part_id,
                title,
                detail,
                is_complete,
            } => {
                let part = self.part_mut(&part_id)?;
                match part {
                    AgentPart::Reasoning(part) => {
                        let changed = part.title != title
                            || part.detail != detail
                            || part.is_complete != is_complete;
                        part.title = title;
                        part.detail = detail;
                        part.is_complete = is_complete;
                        changed
                    }
                    _ => return Err(ProjectionError::PartKindMismatch(part_id)),
                }
            }
            PartUpdate::ToolInputDelta { part_id, delta } => {
                if delta.is_empty() {
                    false
                } else {
                    let part = self.part_mut(&part_id)?;
                    match part {
                        AgentPart::Tool(part) if !part.state.is_terminal() => {
                            part.raw_input.push_str(&delta);
                            true
                        }
                        AgentPart::Tool(_) => false,
                        _ => return Err(ProjectionError::PartKindMismatch(part_id)),
                    }
                }
            }
            PartUpdate::ToolInputReplaced {
                part_id,
                mut input,
                raw_input,
                state,
            } => {
                input = bounded_projection_json(&input);
                let part = self.part_mut(&part_id)?;
                match part {
                    AgentPart::Tool(part) => {
                        let changed = part.input != input
                            || part.raw_input != raw_input
                            || part.state != state;
                        part.input = input;
                        part.raw_input = raw_input;
                        part.state = state;
                        changed
                    }
                    _ => return Err(ProjectionError::PartKindMismatch(part_id)),
                }
            }
            PartUpdate::ToolInputResolved { part_id, mut input } => {
                input = bounded_projection_json(&input);
                let part = self.part_mut(&part_id)?;
                match part {
                    AgentPart::Tool(part) => {
                        let changed = part.input != input;
                        part.input = input;
                        if part.raw_input.is_empty() {
                            part.raw_input = part.input.to_string();
                        }
                        changed
                    }
                    _ => return Err(ProjectionError::PartKindMismatch(part_id)),
                }
            }
            PartUpdate::ToolPermission {
                part_id,
                permission,
            } => {
                let part = self.part_mut(&part_id)?;
                match part {
                    AgentPart::Tool(part) => {
                        let is_request = permission.decision.is_none();
                        let changed = part.permission.as_ref() != Some(&permission)
                            || (is_request
                                && !part.state.is_terminal()
                                && part.state != ToolPartState::WaitingForPermission);
                        part.permission = Some(permission);
                        if is_request && !part.state.is_terminal() {
                            part.state = ToolPartState::WaitingForPermission;
                        }
                        changed
                    }
                    _ => return Err(ProjectionError::PartKindMismatch(part_id)),
                }
            }
            PartUpdate::ToolProgress {
                part_id,
                title,
                detail,
                output_delta,
            } => {
                let part = self.part_mut(&part_id)?;
                match part {
                    AgentPart::Tool(part) if !part.state.is_terminal() => {
                        let mut changed = part.state != ToolPartState::Running;
                        part.state = ToolPartState::Running;
                        if let Some(title) = title.filter(|value| !value.is_empty()) {
                            if part.title != title {
                                part.title = title;
                                changed = true;
                            }
                        }
                        if let Some(delta) = output_delta.filter(|value| !value.is_empty()) {
                            let mut preview = part.output_preview.take().unwrap_or_default();
                            preview.push_str(&delta);
                            const MAX_TOOL_PREVIEW_CHARS: usize = 16_384;
                            if preview.chars().count() > MAX_TOOL_PREVIEW_CHARS {
                                preview = preview
                                    .chars()
                                    .rev()
                                    .take(MAX_TOOL_PREVIEW_CHARS)
                                    .collect::<String>()
                                    .chars()
                                    .rev()
                                    .collect();
                            }
                            part.output_preview = Some(preview);
                            changed = true;
                        } else if let Some(detail) = detail.filter(|value| !value.is_empty()) {
                            if part.output_preview.as_deref() != Some(detail.as_str()) {
                                part.output_preview = Some(detail);
                                changed = true;
                            }
                        }
                        changed
                    }
                    AgentPart::Tool(_) => false,
                    _ => return Err(ProjectionError::PartKindMismatch(part_id)),
                }
            }
            PartUpdate::ToolState {
                part_id,
                state,
                title,
                summary,
                output_preview,
                mut structured_output,
                full_output_ref,
                artifacts,
                timestamp_ms,
            } => {
                structured_output = structured_output.as_ref().map(bounded_projection_json);
                let part = self.part_mut(&part_id)?;
                match part {
                    AgentPart::Tool(part) if !part.state.is_terminal() => {
                        part.state = state;
                        if let Some(title) = title {
                            part.title = title;
                        }
                        if summary.is_some() {
                            part.summary = summary;
                        }
                        if output_preview.is_some() {
                            part.output_preview = output_preview;
                        }
                        if structured_output.is_some() {
                            part.structured_output = structured_output;
                        }
                        if full_output_ref.is_some() {
                            part.full_output_ref = full_output_ref;
                        }
                        if !artifacts.is_empty() {
                            part.artifacts = artifacts;
                        }
                        if part.started_at_ms.is_none() && matches!(state, ToolPartState::Running) {
                            part.started_at_ms = timestamp_ms;
                        }
                        if state.is_terminal() {
                            part.finished_at_ms = timestamp_ms;
                        }
                        true
                    }
                    AgentPart::Tool(part) if part.state == state => false,
                    AgentPart::Tool(_) => false,
                    _ => return Err(ProjectionError::PartKindMismatch(part_id)),
                }
            }
            PartUpdate::Completed { part_id } => {
                let part = self.part_mut(&part_id)?;
                match part {
                    AgentPart::Text(part) => {
                        let changed = !part.is_complete;
                        part.is_complete = true;
                        changed
                    }
                    AgentPart::Reasoning(part) => {
                        let changed = !part.is_complete;
                        part.is_complete = true;
                        changed
                    }
                    AgentPart::Tool(part) if !part.state.is_terminal() => {
                        part.state = ToolPartState::Completed;
                        true
                    }
                    _ => false,
                }
            }
        };

        if changed {
            self.revision = self.revision.saturating_add(1);
        }
        Ok(changed)
    }

    fn part_mut(&mut self, part_id: &str) -> Result<&mut AgentPart, ProjectionError> {
        self.parts
            .get_mut(part_id)
            .ok_or_else(|| ProjectionError::MissingPart(part_id.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_result_absence_null_and_empty_values_survive_part_and_update_wire() {
        let base = serde_json::json!({
            "kind": "tool", "id": "p", "message_id": "m", "turn_id": "t",
            "execution_source": {"kind":"model"},
            "call_id": "call", "capability_id": "cap", "title": "标题",
            "summary": "", "origin": {"kind": "builtin"}, "state": "running",
            "input": {}, "raw_input": "private partial", "permission": null,
            "output_preview": "independent preview", "full_output_ref": "private path",
            "artifacts": [], "started_at_ms": 1, "finished_at_ms": null
        });
        let update_base = serde_json::json!({
            "type": "tool_state", "part_id": "p", "state": "running",
            "title": null, "summary": "", "output_preview": "independent preview",
            "full_output_ref": "private path", "artifacts": [], "timestamp_ms": 1
        });
        for value in [
            None,
            Some(serde_json::Value::Null),
            Some(serde_json::json!("")),
            Some(serde_json::json!([])),
            Some(serde_json::json!({})),
        ] {
            let mut wire = base.clone();
            let mut update_wire = update_base.clone();
            if let Some(value) = &value {
                wire["structured_output"] = value.clone();
                update_wire["structured_output"] = value.clone();
            }
            let part: AgentPart = serde_json::from_value(wire).unwrap();
            let AgentPart::Tool(tool) = &part else {
                panic!("tool")
            };
            assert_eq!(tool.structured_output, value);
            assert_eq!(tool.summary.as_deref(), Some(""));
            assert_eq!(tool.output_preview.as_deref(), Some("independent preview"));
            let encoded = serde_json::to_value(&part).unwrap();
            assert_eq!(encoded.get("structured_output"), value.as_ref());
            assert_eq!(serde_json::from_value::<AgentPart>(encoded).unwrap(), part);

            let update: PartUpdate = serde_json::from_value(update_wire).unwrap();
            let PartUpdate::ToolState {
                structured_output, ..
            } = &update
            else {
                panic!("state")
            };
            assert_eq!(structured_output, &value);
            let encoded = serde_json::to_value(&update).unwrap();
            assert_eq!(encoded.get("structured_output"), value.as_ref());
            assert_eq!(
                serde_json::from_value::<PartUpdate>(encoded).unwrap(),
                update
            );
        }
    }

    #[test]
    fn projected_dynamic_json_marks_overly_deep_subtrees_without_changing_source() {
        let mut source = serde_json::json!(true);
        for _ in 0..(MAX_PROJECTED_JSON_NESTING_DEPTH + 8) {
            source = serde_json::json!({"nested": source});
        }

        let projected = bounded_projection_json(&source);
        let mut cursor = &projected;
        for _ in 0..MAX_PROJECTED_JSON_NESTING_DEPTH {
            cursor = cursor
                .get("nested")
                .expect("projection should retain values before the depth boundary");
        }

        assert_eq!(cursor["_nola_projection"], "truncated");
        assert_eq!(cursor["reason"], "maximum_nesting_depth");
        assert!(source
            .pointer(&"/nested".repeat(MAX_PROJECTED_JSON_NESTING_DEPTH + 8))
            .is_some());
    }

    #[test]
    fn late_text_delta_after_completion_is_an_idempotent_no_op() {
        let mut projection = ConversationProjection::default();
        projection
            .apply(PartUpdate::Created {
                part: AgentPart::Text(TextPart {
                    review: None,
                    id: "assistant-text".to_string(),
                    ownership: PartOwnership::for_turn("turn-1"),
                    text: "final answer".to_string(),
                    is_complete: false,
                }),
            })
            .expect("create text part");
        projection
            .apply(PartUpdate::Completed {
                part_id: "assistant-text".to_string(),
            })
            .expect("complete text part");
        let revision = projection.revision;

        assert!(!projection
            .apply(PartUpdate::TextDelta {
                part_id: "assistant-text".to_string(),
                delta: " stale transport tail".to_string(),
            })
            .expect("ignore late text delta"));
        assert_eq!(projection.revision, revision);
        assert_eq!(
            projection.parts.get("assistant-text"),
            Some(&AgentPart::Text(TextPart {
                review: None,
                id: "assistant-text".to_string(),
                ownership: PartOwnership::for_turn("turn-1"),
                text: "final answer".to_string(),
                is_complete: true,
            }))
        );
    }

    #[test]
    fn late_reasoning_and_tool_updates_do_not_fail_a_completed_turn() {
        let mut projection = ConversationProjection::default();
        projection
            .apply(PartUpdate::Created {
                part: AgentPart::Reasoning(ReasoningPart {
                    id: "reasoning".to_string(),
                    ownership: PartOwnership::for_turn("turn-1"),
                    title: "Thinking".to_string(),
                    detail: "complete".to_string(),
                    is_complete: true,
                }),
            })
            .expect("create completed reasoning");
        projection
            .apply(PartUpdate::Created {
                part: AgentPart::Tool(ToolPart {
                    execution_source: Default::default(),
                    id: "tool".to_string(),
                    ownership: PartOwnership::for_turn("turn-1"),
                    call_id: "call".to_string(),
                    capability_id: "write_file".to_string(),
                    title: "Write file".to_string(),
                    summary: Some("Saved answer.py".to_string()),
                    origin: Default::default(),
                    state: ToolPartState::Completed,
                    input: serde_json::json!({}),
                    raw_input: String::new(),
                    permission: None,
                    output_preview: Some("saved".to_string()),
                    structured_output: Some(serde_json::json!({"saved": true})),
                    full_output_ref: None,
                    artifacts: Vec::new(),
                    started_at_ms: Some(1),
                    finished_at_ms: Some(2),
                }),
            })
            .expect("create completed tool");

        assert!(!projection
            .apply(PartUpdate::ReasoningDelta {
                part_id: "reasoning".to_string(),
                title: None,
                detail_delta: " stale".to_string(),
            })
            .expect("ignore late reasoning"));
        assert!(!projection
            .apply(PartUpdate::ToolInputDelta {
                part_id: "tool".to_string(),
                delta: "stale".to_string(),
            })
            .expect("ignore late tool input fragment"));
        assert!(projection
            .apply(PartUpdate::ToolInputResolved {
                part_id: "tool".to_string(),
                input: serde_json::json!({"path": "answer.py"}),
            })
            .expect("accept authoritative resolved tool input"));
        assert!(!projection
            .apply(PartUpdate::ToolState {
                part_id: "tool".to_string(),
                state: ToolPartState::Running,
                title: None,
                summary: None,
                output_preview: None,
                structured_output: None,
                full_output_ref: None,
                artifacts: Vec::new(),
                timestamp_ms: Some(3),
            })
            .expect("ignore stale nonterminal tool state"));
    }

    #[test]
    fn serialized_projection_restores_multi_turn_message_ownership_without_events() {
        let mut projection = ConversationProjection::default();
        let turn_one = PartOwnership::for_turn("conversation:turn:1");
        let turn_two = PartOwnership::for_turn("conversation:turn:2");
        let parts = vec![
            AgentPart::Text(TextPart {
                review: None,
                id: "text-1".to_string(),
                ownership: turn_one.clone(),
                text: "first".to_string(),
                is_complete: true,
            }),
            AgentPart::Reasoning(ReasoningPart {
                id: "reasoning-1".to_string(),
                ownership: turn_one.clone(),
                title: "Thinking".to_string(),
                detail: "inspect".to_string(),
                is_complete: true,
            }),
            AgentPart::Tool(ToolPart {
                execution_source: Default::default(),
                id: "tool-1".to_string(),
                ownership: turn_one.clone(),
                call_id: "call-1".to_string(),
                capability_id: "read_file".to_string(),
                title: "Read file".to_string(),
                summary: Some("Read one.txt".to_string()),
                origin: Default::default(),
                state: ToolPartState::Completed,
                input: serde_json::json!({"path": "one.txt"}),
                raw_input: String::new(),
                permission: None,
                output_preview: Some("one".to_string()),
                structured_output: None,
                full_output_ref: None,
                artifacts: Vec::new(),
                started_at_ms: Some(1),
                finished_at_ms: Some(2),
            }),
            AgentPart::Text(TextPart {
                review: None,
                id: "text-2".to_string(),
                ownership: turn_two.clone(),
                text: "second".to_string(),
                is_complete: true,
            }),
            AgentPart::Reasoning(ReasoningPart {
                id: "reasoning-2".to_string(),
                ownership: turn_two.clone(),
                title: "Thinking".to_string(),
                detail: "compare".to_string(),
                is_complete: true,
            }),
        ];
        for part in parts {
            projection
                .apply(PartUpdate::Created { part })
                .expect("create owned part");
        }

        let json = serde_json::to_value(&projection).expect("serialize projection");
        assert_eq!(json["parts"]["tool-1"]["message_id"], turn_one.message_id);
        assert_eq!(json["parts"]["tool-1"]["turn_id"], turn_one.turn_id);
        let restored: ConversationProjection =
            serde_json::from_value(json).expect("restore projection without event history");

        let mut groups: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
        for part_id in &restored.order {
            let part = restored.parts.get(part_id).expect("ordered part exists");
            groups
                .entry((part.turn_id().to_string(), part.message_id().to_string()))
                .or_default()
                .push(part.id().to_string());
        }
        assert_eq!(groups.len(), 2);
        assert_eq!(
            groups.get(&(turn_one.turn_id, turn_one.message_id)),
            Some(&vec![
                "text-1".to_string(),
                "reasoning-1".to_string(),
                "tool-1".to_string()
            ])
        );
        assert_eq!(
            groups.get(&(turn_two.turn_id, turn_two.message_id)),
            Some(&vec!["text-2".to_string(), "reasoning-2".to_string()])
        );
    }

    #[test]
    fn source_and_citation_parts_have_stable_json_contract() {
        let ownership = PartOwnership::for_turn("conversation:turn:source");
        let source = AgentPart::Source(SourcePart {
            id: "source_abc".to_string(),
            ownership: ownership.clone(),
            source_id: "source_abc".to_string(),
            title: "Evidence".to_string(),
            uri: "https://example.com/evidence".to_string(),
            mime_type: Some("text/html".to_string()),
            evidence: None,
        });
        let citation = AgentPart::Citation(CitationPart {
            id: "citation_def".to_string(),
            ownership,
            citation_id: "citation_def".to_string(),
            source_id: "source_abc".to_string(),
            title: "Evidence".to_string(),
            uri: "https://example.com/evidence".to_string(),
            mime_type: Some("text/html".to_string()),
            location: Some(SourceLocation {
                page: Some(2),
                line_start: Some(10),
                line_end: Some(14),
                section: Some("Results".to_string()),
            }),
            range: Some(CitationRange { start: 4, end: 12 }),
            origin_tool_call_id: "call-search".to_string(),
        });
        let source_json = serde_json::to_value(&source).expect("serialize Source Part");
        let citation_json = serde_json::to_value(&citation).expect("serialize Citation Part");

        assert_eq!(source_json["kind"], "source");
        assert_eq!(source_json["source_id"], "source_abc");
        assert_eq!(citation_json["kind"], "citation");
        assert_eq!(citation_json["source_id"], source_json["source_id"]);
        assert_eq!(citation_json["origin_tool_call_id"], "call-search");
        assert_eq!(citation_json["location"]["line_start"], 10);
        assert_eq!(citation_json["range"]["end"], 12);
        assert_eq!(
            serde_json::from_value::<AgentPart>(source_json).expect("restore Source Part"),
            source
        );
        assert_eq!(
            serde_json::from_value::<AgentPart>(citation_json).expect("restore Citation Part"),
            citation
        );
    }
}
