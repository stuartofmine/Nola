// SPDX-License-Identifier: Apache-2.0
//! Borrowed classification shared by ordinary projection and journal planning.
//! This is not a persistence DTO: body/JSON fields stay borrowed, and private
//! raw tool input and output storage paths never enter the public view.

use nola_domain::{
    ArtifactReference, CitationRange, HostCapabilityOrigin, SourceEvidence, SourceLocation,
    TextReviewState, ToolPermissionDisplay,
};
use nola_agent_engine::agent::parts::{
    AgentPart, ArtifactPart, CitationPart, CompactionPart, PartOwnership, ReasoningPart,
    SourcePart, StepPart, StepStatus, TextPart, ToolPart, ToolPartState,
};

use crate::canonical_part_projection::CanonicalPartProjection;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PublicPartIdentity<'a> {
    pub part_id: &'a str,
    pub message_id: &'a str,
    pub turn_id: &'a str,
}

impl<'a> PublicPartIdentity<'a> {
    fn new(id: &'a str, ownership: &'a PartOwnership) -> Self {
        let PartOwnership {
            message_id,
            turn_id,
        } = ownership;
        Self {
            part_id: id,
            message_id,
            turn_id,
        }
    }
}

#[derive(Debug)]
pub(crate) enum PublicPart<'a> {
    Text(PublicTextPart<'a>),
    Reasoning(PublicReasoningPart<'a>),
    Tool(PublicToolPart<'a>),
    Source(PublicSourcePart<'a>),
    Citation(PublicCitationPart<'a>),
    Attachment(PublicAttachmentPart<'a>),
    Step(PublicStepPart<'a>),
    Compaction(PublicCompactionPart<'a>),
}

#[derive(Debug)]
pub(crate) struct PublicTextPart<'a> {
    pub identity: PublicPartIdentity<'a>,
    pub text: &'a str,
    pub review: Option<TextReviewState>,
    pub is_complete: bool,
}

#[derive(Debug)]
pub(crate) struct PublicReasoningPart<'a> {
    pub identity: PublicPartIdentity<'a>,
    pub title: &'a str,
    pub detail: &'a str,
    pub is_complete: bool,
}

#[derive(Debug)]
pub(crate) struct PublicToolPart<'a> {
    pub execution_source: &'a nola_domain::code_mode::CodeItemSource,
    pub identity: PublicPartIdentity<'a>,
    pub call_id: &'a str,
    pub capability_id: &'a str,
    pub title: &'a str,
    pub summary: Option<&'a str>,
    pub origin: &'a HostCapabilityOrigin,
    pub state: ToolPartState,
    pub input: &'a serde_json::Value,
    pub permission: Option<&'a ToolPermissionDisplay>,
    pub output_preview: Option<&'a str>,
    pub structured_output: Option<&'a serde_json::Value>,
    /// Presence only. A public consumer must use its separately resolved
    /// canonical owner to construct a typed query identity, never this path.
    pub has_full_output: bool,
    pub artifacts: &'a [ArtifactReference],
    pub started_at_ms: Option<u64>,
    pub finished_at_ms: Option<u64>,
}

#[derive(Debug)]
pub(crate) struct PublicSourcePart<'a> {
    pub identity: PublicPartIdentity<'a>,
    pub source_id: &'a str,
    pub title: &'a str,
    pub uri: &'a str,
    pub mime_type: Option<&'a str>,
    pub evidence: Option<&'a SourceEvidence>,
}

#[derive(Debug)]
pub(crate) struct PublicCitationPart<'a> {
    pub identity: PublicPartIdentity<'a>,
    pub citation_id: &'a str,
    pub source_id: &'a str,
    pub title: &'a str,
    pub uri: &'a str,
    pub mime_type: Option<&'a str>,
    pub location: Option<&'a SourceLocation>,
    pub range: Option<&'a CitationRange>,
    pub origin_tool_call_id: &'a str,
}

#[derive(Debug)]
pub(crate) struct PublicAttachmentPart<'a> {
    pub identity: PublicPartIdentity<'a>,
    pub name: &'a str,
    pub mime_type: Option<&'a str>,
    pub uri: &'a str,
    pub operation: Option<&'a str>,
}

#[derive(Debug)]
pub(crate) struct PublicStepPart<'a> {
    pub identity: PublicPartIdentity<'a>,
    pub title: &'a str,
    pub status: StepStatus,
}

#[derive(Debug)]
pub(crate) struct PublicCompactionPart<'a> {
    pub identity: PublicPartIdentity<'a>,
    pub summary: &'a str,
    pub retained_item_ids: &'a [String],
    pub source_fingerprint: &'a str,
    pub original_token_count: u64,
    pub compacted_token_count: u64,
}

impl<'a> PublicPart<'a> {
    /// Classify an already resolved canonical view without materializing a
    /// remapped Core Part. This changes only the borrowed Message identity;
    /// Store publication still validates the exact persisted owner separately.
    pub(crate) fn from_canonical(view: &CanonicalPartProjection<'a>) -> Self {
        let mut part = Self::from_core(view.part());
        let identity = match &mut part {
            Self::Text(part) => &mut part.identity,
            Self::Reasoning(part) => &mut part.identity,
            Self::Tool(part) => &mut part.identity,
            Self::Source(part) => &mut part.identity,
            Self::Citation(part) => &mut part.identity,
            Self::Attachment(part) => &mut part.identity,
            Self::Step(part) => &mut part.identity,
            Self::Compaction(part) => &mut part.identity,
        };
        identity.message_id = view.message_id();
        part
    }

    pub(crate) fn from_core(part: &'a AgentPart) -> Self {
        match part {
            AgentPart::Text(TextPart {
                review,
                id,
                ownership,
                text,
                is_complete,
            }) => Self::Text(PublicTextPart {
                identity: PublicPartIdentity::new(id, ownership),
                text,
                review: *review,
                is_complete: *is_complete,
            }),
            AgentPart::Reasoning(ReasoningPart {
                id,
                ownership,
                title,
                detail,
                is_complete,
            }) => Self::Reasoning(PublicReasoningPart {
                identity: PublicPartIdentity::new(id, ownership),
                title,
                detail,
                is_complete: *is_complete,
            }),
            AgentPart::Tool(ToolPart {
                execution_source,
                id,
                ownership,
                call_id,
                capability_id,
                title,
                summary,
                origin,
                state,
                input,
                raw_input: _,
                permission,
                output_preview,
                structured_output,
                full_output_ref,
                artifacts,
                started_at_ms,
                finished_at_ms,
            }) => Self::Tool(PublicToolPart {
                execution_source,
                identity: PublicPartIdentity::new(id, ownership),
                call_id,
                capability_id,
                title,
                summary: summary.as_deref(),
                origin,
                state: *state,
                input,
                permission: permission.as_ref(),
                output_preview: output_preview.as_deref(),
                structured_output: structured_output.as_ref(),
                has_full_output: full_output_ref.is_some(),
                artifacts,
                started_at_ms: *started_at_ms,
                finished_at_ms: *finished_at_ms,
            }),
            AgentPart::Source(SourcePart {
                id,
                ownership,
                source_id,
                title,
                uri,
                mime_type,
                evidence,
            }) => Self::Source(PublicSourcePart {
                identity: PublicPartIdentity::new(id, ownership),
                source_id,
                title,
                uri,
                mime_type: mime_type.as_deref(),
                evidence: evidence.as_ref(),
            }),
            AgentPart::Citation(CitationPart {
                id,
                ownership,
                citation_id,
                source_id,
                title,
                uri,
                mime_type,
                location,
                range,
                origin_tool_call_id,
            }) => Self::Citation(PublicCitationPart {
                identity: PublicPartIdentity::new(id, ownership),
                citation_id,
                source_id,
                title,
                uri,
                mime_type: mime_type.as_deref(),
                location: location.as_ref(),
                range: range.as_ref(),
                origin_tool_call_id,
            }),
            AgentPart::Artifact(ArtifactPart {
                id,
                ownership,
                artifact,
            }) => {
                let ArtifactReference {
                    uri,
                    name,
                    mime_type,
                    operation,
                } = artifact;
                Self::Attachment(PublicAttachmentPart {
                    identity: PublicPartIdentity::new(id, ownership),
                    name,
                    mime_type: mime_type.as_deref(),
                    uri,
                    operation: operation.as_deref(),
                })
            }
            AgentPart::Step(StepPart {
                id,
                ownership,
                title,
                status,
            }) => Self::Step(PublicStepPart {
                identity: PublicPartIdentity::new(id, ownership),
                title,
                status: *status,
            }),
            AgentPart::Compaction(CompactionPart {
                id,
                ownership,
                summary,
                retained_item_ids,
                source_fingerprint,
                original_token_count,
                compacted_token_count,
            }) => Self::Compaction(PublicCompactionPart {
                identity: PublicPartIdentity::new(id, ownership),
                summary,
                retained_item_ids,
                source_fingerprint,
                original_token_count: *original_token_count,
                compacted_token_count: *compacted_token_count,
            }),
        }
    }

    pub(crate) fn identity(&self) -> PublicPartIdentity<'a> {
        match self {
            Self::Text(part) => part.identity,
            Self::Reasoning(part) => part.identity,
            Self::Tool(part) => part.identity,
            Self::Source(part) => part.identity,
            Self::Citation(part) => part.identity,
            Self::Attachment(part) => part.identity,
            Self::Step(part) => part.identity,
            Self::Compaction(part) => part.identity,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn borrowed_mapping_keeps_large_text_and_json_addresses() {
        let text = AgentPart::Text(TextPart {
            review: Some(TextReviewState::Pending),
            id: "text".into(),
            ownership: PartOwnership::for_turn("turn"),
            text: "原始🙂".repeat(256_000),
            is_complete: false,
        });
        let AgentPart::Text(original) = &text else {
            unreachable!()
        };
        let PublicPart::Text(mapped) = PublicPart::from_core(&text) else {
            panic!("expected borrowed Text")
        };
        assert_eq!(mapped.text.as_ptr(), original.text.as_ptr());
        assert_eq!(mapped.identity.part_id.as_ptr(), original.id.as_ptr());
        assert_eq!(mapped.review, Some(TextReviewState::Pending));

        let tool = AgentPart::Tool(ToolPart {
            execution_source: Default::default(),
            id: "tool".into(),
            ownership: PartOwnership::for_turn("turn"),
            call_id: "call".into(),
            capability_id: "read_file".into(),
            title: "title".into(),
            summary: Some(String::new()),
            origin: Default::default(),
            state: ToolPartState::Completed,
            input: serde_json::json!({"payload": "输入🙂".repeat(256_000)}),
            raw_input: "PRIVATE_RAW_INPUT".into(),
            permission: None,
            output_preview: Some(String::new()),
            structured_output: Some(serde_json::Value::Null),
            full_output_ref: Some("/PRIVATE_OUTPUT_PATH".into()),
            artifacts: vec![],
            started_at_ms: Some(0),
            finished_at_ms: None,
        });
        let AgentPart::Tool(original) = &tool else {
            unreachable!()
        };
        let public = PublicPart::from_core(&tool);
        let PublicPart::Tool(mapped) = &public else {
            panic!("expected borrowed Tool")
        };
        assert!(std::ptr::eq(mapped.input, &original.input));
        assert!(std::ptr::eq(
            mapped.structured_output.unwrap(),
            original.structured_output.as_ref().unwrap()
        ));
        assert_eq!(mapped.summary, Some(""));
        assert_eq!(mapped.output_preview, Some(""));
        assert!(mapped.has_full_output);
        assert_eq!(mapped.started_at_ms, Some(0));
        assert_eq!(public.identity().message_id, "turn:assistant");
        let diagnostic = format!("{public:?}");
        assert!(!diagnostic.contains("PRIVATE_RAW_INPUT"));
        assert!(!diagnostic.contains("PRIVATE_OUTPUT_PATH"));
    }
}
