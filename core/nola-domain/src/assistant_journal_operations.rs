// SPDX-License-Identifier: Apache-2.0
//! Typed, bounded assistant journal operations. No Store or runtime IO.
//! Content references require an owning Store's manifest validation; a digest
//! alone is never interpreted as verified or publicly visible content.
use super::assistant_journal::{
    AssistantDigest, AssistantJournalIdentity, AssistantJournalRecordHeaderV1,
    ASSISTANT_CHUNK_MAX_BYTES, ASSISTANT_JOURNAL_VERSION,
};
use crate::{
    ActivityState, HostCapabilityOriginKind, MessageId, MessagePartId, TextReviewState, ToolCallId,
    ToolOutputReference,
};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_OPERATION_BATCH_BYTES: usize = 128 * 1024;
pub const MAX_BATCH_OPERATIONS: usize = 128;
pub const MAX_METADATA_STRING_BYTES: usize = 4096;
pub const MAX_PART_CONTENT_FIELDS: usize = 16;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OperationError {
    InvalidWire,
    InvalidVersion,
    Budget,
    InvalidIdentity,
    InvalidMetadata,
    InvalidContent,
    MissingPart,
    ExistingPart,
    OwnerMismatch,
    HeadConflict,
    GenerationConflict,
    ContentConflict,
    InvalidSequence,
    ManifestUnavailable,
    HeaderMismatch,
    RetiredPart,
}
impl std::fmt::Display for OperationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for OperationError {}
type Result<T> = std::result::Result<T, OperationError>;

/// Lossless across JavaScript/Swift/Rust: JSON decimal string, binary u64 BE.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct JournalNumber(pub u64);
impl Serialize for JournalNumber {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0.to_string())
    }
}
impl<'de> Deserialize<'de> for JournalNumber {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        if value.is_empty()
            || value.len() > 20
            || !value.bytes().all(|b| b.is_ascii_digit())
            || value.len() > 1 && value.starts_with('0')
        {
            return Err(serde::de::Error::custom(
                "expected canonical u64 decimal string",
            ));
        }
        value
            .parse::<u64>()
            .map(Self)
            .map_err(serde::de::Error::custom)
    }
}

// Fixed-order binary encoding. All lengths u32 BE, counters u64 BE, options
// 0/1, booleans 0/1, enums explicit u8 tags. No JSON formatting enters hashes.
trait Binary: Sized {
    fn write(&self, out: &mut Vec<u8>) -> Result<()>;
    fn read(reader: &mut Reader<'_>) -> Result<Self>;
}
struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}
impl<'a> Reader<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8]> {
        let end = self
            .offset
            .checked_add(count)
            .ok_or(OperationError::InvalidWire)?;
        let value = self
            .bytes
            .get(self.offset..end)
            .ok_or(OperationError::InvalidWire)?;
        self.offset = end;
        Ok(value)
    }
    fn length(&mut self, limit: usize) -> Result<usize> {
        let n = u32::from_be_bytes(self.take(4)?.try_into().unwrap()) as usize;
        if n > limit {
            return Err(OperationError::Budget);
        }
        Ok(n)
    }
}
impl Binary for JournalNumber {
    fn write(&self, out: &mut Vec<u8>) -> Result<()> {
        out.extend_from_slice(&self.0.to_be_bytes());
        Ok(())
    }
    fn read(r: &mut Reader<'_>) -> Result<Self> {
        Ok(Self(u64::from_be_bytes(r.take(8)?.try_into().unwrap())))
    }
}
impl Binary for bool {
    fn write(&self, out: &mut Vec<u8>) -> Result<()> {
        out.push(u8::from(*self));
        Ok(())
    }
    fn read(r: &mut Reader<'_>) -> Result<Self> {
        match r.take(1)?[0] {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(OperationError::InvalidWire),
        }
    }
}
impl Binary for String {
    fn write(&self, out: &mut Vec<u8>) -> Result<()> {
        if self.len() > MAX_METADATA_STRING_BYTES {
            return Err(OperationError::Budget);
        }
        out.extend_from_slice(&(self.len() as u32).to_be_bytes());
        out.extend_from_slice(self.as_bytes());
        Ok(())
    }
    fn read(r: &mut Reader<'_>) -> Result<Self> {
        let n = r.length(MAX_METADATA_STRING_BYTES)?;
        std::str::from_utf8(r.take(n)?)
            .map(str::to_owned)
            .map_err(|_| OperationError::InvalidWire)
    }
}
impl Binary for AssistantDigest {
    fn write(&self, out: &mut Vec<u8>) -> Result<()> {
        for pair in self.to_hex().as_bytes().chunks_exact(2) {
            let nibble = |b: u8| if b <= b'9' { b - b'0' } else { b - b'a' + 10 };
            out.push(nibble(pair[0]) << 4 | nibble(pair[1]));
        }
        Ok(())
    }
    fn read(r: &mut Reader<'_>) -> Result<Self> {
        Self::parse_hex(
            &r.take(32)?
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>(),
        )
        .map_err(|_| OperationError::InvalidWire)
    }
}
impl Binary for crate::generative_ui::UiSurfaceRevisionRef {
    fn write(&self, out: &mut Vec<u8>) -> Result<()> {
        self.validate().map_err(|_| OperationError::InvalidMetadata)?;
        let owner = &self.identity.owner;
        for value in [&owner.authority_id, &owner.conversation_id, &owner.participant_id,
            &owner.context_id, &owner.originating_run_id, &owner.message_id, &owner.part_id,
            &self.identity.surface_id] { value.write(out)?; }
        JournalNumber(self.identity.generation).write(out)?;
        JournalNumber(self.revision).write(out)
    }
    fn read(r: &mut Reader<'_>) -> Result<Self> {
        use crate::generative_ui::{UiSurfaceOwner, UiSurfaceIdentity};
        let value = Self { identity: UiSurfaceIdentity { owner: UiSurfaceOwner {
            authority_id: String::read(r)?, conversation_id: String::read(r)?, participant_id: String::read(r)?,
            context_id: String::read(r)?, originating_run_id: String::read(r)?, message_id: String::read(r)?, part_id: String::read(r)?,
        }, surface_id: String::read(r)?, generation: JournalNumber::read(r)?.0 }, revision: JournalNumber::read(r)?.0 };
        value.validate().map_err(|_| OperationError::InvalidMetadata)?;
        Ok(value)
    }
}
impl Binary for crate::code_mode::CodeItemSource {
    fn write(&self, out: &mut Vec<u8>) -> Result<()> {
        match self {
            Self::Model => out.push(0),
            Self::CodeCell { cell } => {
                cell.validate().map_err(|_| OperationError::InvalidContent)?;
                out.push(1);
                for value in [&cell.host_id, &cell.conversation_id, &cell.turn_id,
                    &cell.parent_call_id, &cell.cell_id, &cell.generation] {
                    value.write(out)?;
                }
            }
        }
        Ok(())
    }
    fn read(r: &mut Reader<'_>) -> Result<Self> {
        match r.take(1)?[0] {
            0 => Ok(Self::Model),
            1 => {
                let cell = crate::code_mode::CodeCellIdentity {
                    host_id: String::read(r)?, conversation_id: String::read(r)?,
                    turn_id: String::read(r)?, parent_call_id: String::read(r)?,
                    cell_id: String::read(r)?, generation: String::read(r)?,
                };
                cell.validate().map_err(|_| OperationError::InvalidContent)?;
                Ok(Self::CodeCell { cell })
            }
            _ => Err(OperationError::InvalidWire),
        }
    }
}

fn validate_tool_identity(previous: &JournalPartMetadata, next: &JournalPartMetadata) -> Result<()> {
    if let JournalPartMetadata::UiSurface(old) = previous {
        let JournalPartMetadata::UiSurface(new) = next else { return Err(OperationError::OwnerMismatch); };
        if old.surface.identity != new.surface.identity || new.surface.revision <= old.surface.revision {
            return Err(OperationError::OwnerMismatch);
        }
    }
    if let JournalPartMetadata::Tool(old) = previous {
        let JournalPartMetadata::Tool(new) = next else { return Err(OperationError::OwnerMismatch); };
        if old.tool_call_id != new.tool_call_id || old.capability_id != new.capability_id
            || old.execution_source != new.execution_source {
            return Err(OperationError::OwnerMismatch);
        }
    }
    Ok(())
}
impl<T: Binary> Binary for Option<T> {
    fn write(&self, out: &mut Vec<u8>) -> Result<()> {
        match self {
            None => out.push(0),
            Some(v) => {
                out.push(1);
                v.write(out)?;
            }
        }
        Ok(())
    }
    fn read(r: &mut Reader<'_>) -> Result<Self> {
        match r.take(1)?[0] {
            0 => Ok(None),
            1 => Ok(Some(T::read(r)?)),
            _ => Err(OperationError::InvalidWire),
        }
    }
}
impl<T: Binary> Binary for Vec<T> {
    fn write(&self, out: &mut Vec<u8>) -> Result<()> {
        if self.len() > MAX_BATCH_OPERATIONS {
            return Err(OperationError::Budget);
        }
        out.extend_from_slice(&(self.len() as u32).to_be_bytes());
        for item in self {
            item.write(out)?;
            if out.len() > MAX_OPERATION_BATCH_BYTES {
                return Err(OperationError::Budget);
            }
        }
        Ok(())
    }
    fn read(r: &mut Reader<'_>) -> Result<Self> {
        let count = r.length(MAX_BATCH_OPERATIONS)?;
        (0..count).map(|_| T::read(r)).collect()
    }
}
macro_rules! wire_struct {
    ($name:ident { $($field:ident : $ty:ty),* $(,)? }) => {
        #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(deny_unknown_fields)]
        pub struct $name { $(pub $field: $ty),* }
        impl Binary for $name {
            fn write(&self, out: &mut Vec<u8>) -> Result<()> { $(self.$field.write(out)?;)* Ok(()) }
            fn read(r: &mut Reader<'_>) -> Result<Self> { Ok(Self { $($field: <$ty>::read(r)?),* }) }
        }
    }
}
macro_rules! wire_enum {
    ($name:ident { $($variant:ident = $tag:literal),* $(,)? }) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum $name { $($variant),* }
        impl Binary for $name {
            fn write(&self, out: &mut Vec<u8>) -> Result<()> { out.push(match self { $(Self::$variant => $tag),* }); Ok(()) }
            fn read(r: &mut Reader<'_>) -> Result<Self> { match r.take(1)?[0] { $($tag => Ok(Self::$variant)),*, _ => Err(OperationError::InvalidWire) } }
        }
    }
}
impl Binary for AssistantJournalIdentity {
    fn write(&self, out: &mut Vec<u8>) -> Result<()> {
        self.source_store_epoch.write(out)?;
        self.conversation_id.write(out)?;
        self.run_id.write(out)?;
        self.assistant_message_id.write(out)
    }
    fn read(r: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            source_store_epoch: String::read(r)?,
            conversation_id: String::read(r)?,
            run_id: String::read(r)?,
            assistant_message_id: String::read(r)?,
        })
    }
}
impl Binary for ActivityState {
    fn write(&self, out: &mut Vec<u8>) -> Result<()> {
        out.push(match self {
            Self::Pending => 0,
            Self::Running => 1,
            Self::WaitingForPermission => 2,
            Self::Completed => 3,
            Self::Cancelled => 4,
            Self::Failed => 5,
            Self::WaitingForInput => 6,
            Self::Sealed => 7,
        });
        Ok(())
    }
    fn read(r: &mut Reader<'_>) -> Result<Self> {
        match r.take(1)?[0] {
            0 => Ok(Self::Pending),
            1 => Ok(Self::Running),
            2 => Ok(Self::WaitingForPermission),
            3 => Ok(Self::Completed),
            4 => Ok(Self::Cancelled),
            5 => Ok(Self::Failed),
            6 => Ok(Self::WaitingForInput),
            7 => Ok(Self::Sealed),
            _ => Err(OperationError::InvalidWire),
        }
    }
}

impl Binary for TextReviewState {
    fn write(&self, out: &mut Vec<u8>) -> Result<()> {
        out.push(match self {
            Self::Pending => 0,
            Self::Approved => 1,
            Self::Rejected => 2,
            Self::Interrupted => 3,
        });
        Ok(())
    }
    fn read(r: &mut Reader<'_>) -> Result<Self> {
        match r.take(1)?[0] {
            0 => Ok(Self::Pending),
            1 => Ok(Self::Approved),
            2 => Ok(Self::Rejected),
            3 => Ok(Self::Interrupted),
            _ => Err(OperationError::InvalidWire),
        }
    }
}
impl Binary for HostCapabilityOriginKind {
    fn write(&self, out: &mut Vec<u8>) -> Result<()> {
        out.push(match self {
            Self::Unknown => 0,
            Self::Builtin => 1,
            Self::Skill => 2,
            Self::Mcp => 3,
            Self::Device => 4,
            Self::Extension => 5,
            Self::Internal => 6,
        });
        Ok(())
    }
    fn read(r: &mut Reader<'_>) -> Result<Self> {
        match r.take(1)?[0] {
            0 => Ok(Self::Unknown),
            1 => Ok(Self::Builtin),
            2 => Ok(Self::Skill),
            3 => Ok(Self::Mcp),
            4 => Ok(Self::Device),
            5 => Ok(Self::Extension),
            6 => Ok(Self::Internal),
            _ => Err(OperationError::InvalidWire),
        }
    }
}
impl Binary for ToolOutputReference {
    fn write(&self, out: &mut Vec<u8>) -> Result<()> {
        self.message_id.0.write(out)?;
        self.part_id.0.write(out)?;
        self.tool_call_id.0.write(out)
    }
    fn read(r: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            message_id: MessageId(String::read(r)?),
            part_id: MessagePartId(String::read(r)?),
            tool_call_id: ToolCallId(String::read(r)?),
        })
    }
}

wire_enum!(JournalContentField { Markdown=0, ReasoningSummary=1, ToolInput=2, ToolOutput=3,
    SourceSnippet=4, TaskCurrentStep=5, ResultReference=6, FailureMessage=7, HistoricalPreview=8,
    ToolOutputText=9, Title=10, Uri=11, MimeType=12, Name=13, ToolResources=14,
    ToolSummary=15, PermissionDomain=16, PermissionRationale=17, PermissionDecision=18,
    OriginProviderId=19, OriginProviderTitle=20, OriginNamespace=21, ToolArtifacts=22,
    SourceEvidence=23, CitationSection=24, Operation=25 });
wire_enum!(JournalVisibility { Public=0, Withheld=1 });
wire_enum!(JournalSection { Transcript=0, Tasks=1, Skills=2, ModelAttempts=3 });
wire_enum!(JournalSkillMode { Inline=0, Fork=1 });
wire_enum!(JournalHistoricalSchema { AgentPart=0, MessagePartView=1 });
wire_struct!(JournalOrderKey {
    section: JournalSection,
    position: JournalNumber
});
wire_struct!(JournalChunkRef {
    digest: AssistantDigest,
    utf8_bytes: JournalNumber
});
// content_root is an immutable manifest locator, not a complete chunk list.
wire_struct!(JournalContentRef {
    field: JournalContentField,
    content_generation: JournalNumber,
    utf8_bytes: JournalNumber,
    chunk_count: JournalNumber,
    content_root: AssistantDigest
});
wire_struct!(JournalRange {
    start: JournalNumber,
    end: JournalNumber
});
/// Exact finite binary64 fraction in [0,1], including subnormals and signed
/// zero. A u64 rational cannot represent every actual task progress value.
/// JSON carries the bits as a canonical decimal string; hashes use u64 BE,
/// never platform-specific floating-point formatting or a rounded percentage.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct JournalProgress {
    fraction_bits: JournalNumber,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct JournalProgressWire {
    fraction_bits: JournalNumber,
}
impl JournalProgress {
    pub fn from_fraction(fraction: f64) -> Result<Self> {
        if !fraction.is_finite() || !(0.0..=1.0).contains(&fraction) {
            return Err(OperationError::InvalidMetadata);
        }
        Ok(Self {
            fraction_bits: JournalNumber(fraction.to_bits()),
        })
    }

    pub fn fraction(self) -> f64 {
        f64::from_bits(self.fraction_bits.0)
    }

    fn from_bits(bits: JournalNumber) -> Result<Self> {
        Self::from_fraction(f64::from_bits(bits.0))
    }
}
impl<'de> Deserialize<'de> for JournalProgress {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        let wire = JournalProgressWire::deserialize(deserializer)?;
        Self::from_bits(wire.fraction_bits).map_err(serde::de::Error::custom)
    }
}
impl Binary for JournalProgress {
    fn write(&self, out: &mut Vec<u8>) -> Result<()> {
        self.fraction_bits.write(out)
    }

    fn read(reader: &mut Reader<'_>) -> Result<Self> {
        Self::from_bits(JournalNumber::read(reader)?)
    }
}
wire_struct!(JournalAffectedResource {
    resource_type: String,
    uri: String,
    operation: String
});
/// Text metadata is either bounded inline UTF-8 or a typed field in this Part.
/// The reference is validated against the owning head, never an arbitrary URI.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum JournalTextValue {
    Inline(String),
    Field(JournalContentField),
}
impl From<String> for JournalTextValue {
    fn from(value: String) -> Self {
        Self::Inline(value)
    }
}
impl From<&str> for JournalTextValue {
    fn from(value: &str) -> Self {
        Self::Inline(value.to_owned())
    }
}
impl Binary for JournalTextValue {
    fn write(&self, out: &mut Vec<u8>) -> Result<()> {
        match self {
            Self::Inline(value) => {
                out.push(0);
                value.write(out)?;
            }
            Self::Field(field) => {
                out.push(1);
                field.write(out)?;
            }
        }
        Ok(())
    }
    fn read(r: &mut Reader<'_>) -> Result<Self> {
        match r.take(1)?[0] {
            0 => Ok(Self::Inline(String::read(r)?)),
            1 => Ok(Self::Field(JournalContentField::read(r)?)),
            _ => Err(OperationError::InvalidWire),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum JournalResourceList {
    Inline(Vec<JournalAffectedResource>),
    Field(JournalContentField),
}
impl From<Vec<JournalAffectedResource>> for JournalResourceList {
    fn from(value: Vec<JournalAffectedResource>) -> Self {
        Self::Inline(value)
    }
}
impl Binary for JournalResourceList {
    fn write(&self, out: &mut Vec<u8>) -> Result<()> {
        match self {
            Self::Inline(value) => {
                out.push(0);
                value.write(out)?;
            }
            Self::Field(field) => {
                out.push(1);
                field.write(out)?;
            }
        }
        Ok(())
    }
    fn read(r: &mut Reader<'_>) -> Result<Self> {
        match r.take(1)?[0] {
            0 => Ok(Self::Inline(Vec::<JournalAffectedResource>::read(r)?)),
            1 => Ok(Self::Field(JournalContentField::read(r)?)),
            _ => Err(OperationError::InvalidWire),
        }
    }
}
wire_struct!(JournalTextMetadata {
    is_final: bool,
    visibility: JournalVisibility,
    review: Option<TextReviewState>
});
wire_struct!(JournalReasoningMetadata {
    title: JournalTextValue,
    state: ActivityState
});
wire_struct!(JournalStepMetadata {
    title: JournalTextValue,
    state: ActivityState
});
// Read-only provenance and permission display, never an execution grant.
wire_struct!(JournalToolOrigin { kind: HostCapabilityOriginKind,
    provider_id: Option<JournalTextValue>, provider_title: Option<JournalTextValue>, namespace: Option<JournalTextValue> });
wire_struct!(JournalToolPermission { request_id: String, permission_domain: JournalTextValue,
    rationale: JournalTextValue, decision: Option<JournalTextValue> });
wire_struct!(JournalToolMetadata { tool_call_id: String, capability_id: String, title: JournalTextValue,
    execution_source: crate::code_mode::CodeItemSource,
    summary: Option<JournalTextValue>, origin: JournalToolOrigin, permission: Option<JournalToolPermission>,
    state: ActivityState, display_key: Option<String>, affected_resources: JournalResourceList,
    artifacts: JournalContentField, full_output: Option<ToolOutputReference>,
    started_at_ms: Option<JournalNumber>, finished_at_ms: Option<JournalNumber> });
wire_struct!(JournalSourceMetadata { source_id: String, title: JournalTextValue, uri: JournalTextValue,
    mime_type: Option<JournalTextValue>, evidence: Option<JournalContentField> });
wire_struct!(JournalCitationLocation { page: Option<JournalNumber>, line_start: Option<JournalNumber>,
    line_end: Option<JournalNumber>, section: Option<JournalTextValue> });
wire_struct!(JournalCitationMetadata { citation_id: String, source_id: String, title: JournalTextValue,
    uri: JournalTextValue, mime_type: Option<JournalTextValue>, origin_tool_call_id: String,
    location: Option<JournalCitationLocation>, byte_range: Option<JournalRange> });
wire_struct!(JournalAttachmentMetadata { attachment_id: String, name: JournalTextValue,
    mime_type: Option<JournalTextValue>, uri: JournalTextValue, operation: Option<JournalTextValue> });
wire_struct!(JournalTaskMetadata { task_id: String, display_key: String, state: ActivityState,
    progress: Option<JournalProgress>, cancel_requested: bool });
// A distinct wire tag leaves historical Task records read-only; capability
// claims are explicit facts and cannot be inferred from missing fields.
wire_struct!(JournalTaskControlMetadata { task: JournalTaskMetadata,
    supports_guidance: bool, supports_cancellation: bool });
wire_struct!(JournalSkillMetadata {
    run_id: String,
    skill_id: String,
    display_name: JournalTextValue,
    mode: JournalSkillMode,
    state: ActivityState,
    model_turns: JournalNumber,
    tool_calls: JournalNumber,
    elapsed_millis: JournalNumber
});
wire_struct!(JournalModelAttemptMetadata { attempt_id: String, state: ActivityState, provider: Option<String>, model: Option<String>,
    failure_code: String, retryable: bool, http_status: Option<JournalNumber>, fallback_attempt_id: Option<String>,
    fallback_provider: Option<String>, fallback_model: Option<String> });
wire_struct!(JournalErrorMetadata {
    code: String,
    display_key: String,
    recoverable: bool
});
wire_struct!(JournalUiSurfaceMetadata { surface: crate::generative_ui::UiSurfaceRevisionRef, summary: String });
wire_struct!(JournalHistoricalMetadata {
    schema: JournalHistoricalSchema,
    canonical_kind: String,
    canonical_revision: JournalNumber
});

macro_rules! payload_enum {
    ($name:ident { $($variant:ident($ty:ty) = $tag:literal),* $(,)? }) => {
        #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(tag="kind", content="data", rename_all="snake_case", deny_unknown_fields)]
        pub enum $name { $($variant($ty)),* }
        impl Binary for $name {
            fn write(&self, out: &mut Vec<u8>) -> Result<()> { match self { $(Self::$variant(value) => {out.push($tag);value.write(out)?;}),* } Ok(()) }
            fn read(r: &mut Reader<'_>) -> Result<Self> { match r.take(1)?[0] { $($tag => Ok(Self::$variant(<$ty>::read(r)?))),*, _ => Err(OperationError::InvalidWire) } }
        }
    }
}
payload_enum!(JournalPartMetadata {
    Text(JournalTextMetadata)=0, Reasoning(JournalReasoningMetadata)=1, Tool(JournalToolMetadata)=2,
    Source(JournalSourceMetadata)=3, Citation(JournalCitationMetadata)=4, Attachment(JournalAttachmentMetadata)=5,
    Task(JournalTaskMetadata)=6, Skill(JournalSkillMetadata)=7, ModelAttempt(JournalModelAttemptMetadata)=8,
    Error(JournalErrorMetadata)=9, Historical(JournalHistoricalMetadata)=10,
    Step(JournalStepMetadata)=11, TaskControl(JournalTaskControlMetadata)=12, UiSurface(JournalUiSurfaceMetadata)=13
});
wire_struct!(JournalPartHead { identity: AssistantJournalIdentity, part_id: String,
    generation: JournalNumber, revision: JournalNumber, metadata_revision: JournalNumber,
    order_key: JournalOrderKey, metadata: JournalPartMetadata, bodies: Vec<JournalContentRef> });
wire_struct!(DeclareJournalPart {
    part: JournalPartHead
});
wire_struct!(AppendJournalText { part_id: String, expected_generation: JournalNumber, expected_revision: JournalNumber,
    expected: JournalContentRef, chunks: Vec<JournalChunkRef>, resulting: JournalContentRef });
wire_struct!(ReplaceJournalPart {
    part_id: String,
    expected_generation: JournalNumber,
    expected_revision: JournalNumber,
    part: JournalPartHead
});
wire_struct!(UpdateJournalMetadata {
    part_id: String,
    expected_generation: JournalNumber,
    expected_revision: JournalNumber,
    metadata: JournalPartMetadata
});
wire_struct!(RemoveJournalPart {
    part_id: String,
    expected_generation: JournalNumber,
    expected_revision: JournalNumber
});
// Field versions use the new head revision, not the Part replacement generation.
// A deletion leaves the head clock intact, preventing old manifests from revival.
wire_struct!(ReplaceJournalContent {
    part_id: String,
    expected_generation: JournalNumber,
    expected_revision: JournalNumber,
    field: JournalContentField,
    expected: Option<JournalContentRef>,
    resulting: Option<JournalContentRef>
});
payload_enum!(AssistantJournalOperationV1 { Declare(DeclareJournalPart)=0, Append(AppendJournalText)=1,
    Replace(ReplaceJournalPart)=2, Update(UpdateJournalMetadata)=3, Remove(RemoveJournalPart)=4,
    ReplaceContent(ReplaceJournalContent)=5 });

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssistantOperationBatchV1 {
    pub protocol_version: u16,
    pub identity: AssistantJournalIdentity,
    pub operations: Vec<AssistantJournalOperationV1>,
}
fn hash(bytes: &[u8]) -> AssistantDigest {
    AssistantDigest::parse_hex(&format!("{:x}", Sha256::digest(bytes))).expect("sha256 hex")
}
fn tagged(kind: u8) -> Vec<u8> {
    let mut out = b"NOLA-AJ".to_vec();
    out.extend_from_slice(&ASSISTANT_JOURNAL_VERSION.to_be_bytes());
    out.push(kind);
    out
}
impl AssistantOperationBatchV1 {
    pub fn canonical_bytes(&self) -> Result<Vec<u8>> {
        if self.protocol_version != ASSISTANT_JOURNAL_VERSION {
            return Err(OperationError::InvalidVersion);
        }
        self.identity
            .validate()
            .map_err(|_| OperationError::InvalidIdentity)?;
        if self.operations.is_empty() || self.operations.len() > MAX_BATCH_OPERATIONS {
            return Err(OperationError::Budget);
        }
        for operation in &self.operations {
            validate_operation_shape(operation, &self.identity)?;
        }
        let mut out = tagged(3);
        self.identity.write(&mut out)?;
        self.operations.write(&mut out)?;
        if out.len() > MAX_OPERATION_BATCH_BYTES {
            return Err(OperationError::Budget);
        }
        Ok(out)
    }
    pub fn decode_canonical(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > MAX_OPERATION_BATCH_BYTES {
            return Err(OperationError::Budget);
        }
        let mut r = Reader { bytes, offset: 0 };
        if r.take(10)? != tagged(3).as_slice() {
            return Err(OperationError::InvalidVersion);
        }
        let result = Self {
            protocol_version: ASSISTANT_JOURNAL_VERSION,
            identity: AssistantJournalIdentity::read(&mut r)?,
            operations: Vec::read(&mut r)?,
        };
        if r.offset != bytes.len() || result.canonical_bytes()?.as_slice() != bytes {
            return Err(OperationError::InvalidWire);
        }
        Ok(result)
    }
    pub fn digest(&self) -> Result<AssistantDigest> {
        Ok(hash(&self.canonical_bytes()?))
    }
    pub fn seal(
        &self,
        sequence: u64,
        previous: AssistantDigest,
        operation_id: String,
        source_commit_revision: u64,
    ) -> Result<AssistantJournalRecordHeaderV1> {
        AssistantJournalRecordHeaderV1::seal(
            self.identity.clone(),
            sequence,
            previous,
            operation_id,
            source_commit_revision,
            self.digest()?,
        )
        .map_err(|_| OperationError::HeaderMismatch)
    }
    pub fn verify_header(&self, header: &AssistantJournalRecordHeaderV1) -> Result<()> {
        header
            .verify()
            .map_err(|_| OperationError::HeaderMismatch)?;
        if header.identity != self.identity || header.operation_batch_digest != self.digest()? {
            return Err(OperationError::HeaderMismatch);
        }
        Ok(())
    }
}

impl AssistantJournalRecordHeaderV1 {
    pub fn seal_from_batch(
        batch: &AssistantOperationBatchV1,
        sequence: u64,
        previous: AssistantDigest,
        operation_id: String,
        source_commit_revision: u64,
    ) -> Result<Self> {
        batch.seal(sequence, previous, operation_id, source_commit_revision)
    }

    /// Validates typed bytes and binding, not persisted ownership or manifests.
    /// The source/target Store must also run validate_assistant_batch in its UoW.
    pub fn verify_with_batch(&self, batch: &AssistantOperationBatchV1) -> Result<()> {
        batch.verify_header(self)
    }
}

pub fn empty_journal_content(
    identity: &AssistantJournalIdentity,
    part_id: &str,
    field: JournalContentField,
    generation: JournalNumber,
) -> Result<JournalContentRef> {
    identity
        .validate()
        .map_err(|_| OperationError::InvalidIdentity)?;
    valid_id(part_id)?;
    if generation.0 == 0 {
        return Err(OperationError::InvalidSequence);
    }
    let mut bytes = tagged(4);
    identity.write(&mut bytes)?;
    part_id.to_owned().write(&mut bytes)?;
    field.write(&mut bytes)?;
    generation.write(&mut bytes)?;
    Ok(JournalContentRef {
        field,
        content_generation: generation,
        utf8_bytes: JournalNumber(0),
        chunk_count: JournalNumber(0),
        content_root: hash(&bytes),
    })
}
pub fn append_journal_content(
    previous: &JournalContentRef,
    chunks: &[JournalChunkRef],
) -> Result<JournalContentRef> {
    if chunks.is_empty() || chunks.len() > MAX_BATCH_OPERATIONS {
        return Err(OperationError::Budget);
    }
    let mut result = previous.clone();
    for chunk in chunks {
        if chunk.utf8_bytes.0 == 0 || chunk.utf8_bytes.0 > ASSISTANT_CHUNK_MAX_BYTES as u64 {
            return Err(OperationError::InvalidContent);
        }
        let mut bytes = tagged(5);
        result.content_root.write(&mut bytes)?;
        chunk.digest.write(&mut bytes)?;
        chunk.utf8_bytes.write(&mut bytes)?;
        result.content_root = hash(&bytes);
        result.utf8_bytes = JournalNumber(
            result
                .utf8_bytes
                .0
                .checked_add(chunk.utf8_bytes.0)
                .ok_or(OperationError::InvalidContent)?,
        );
        result.chunk_count = JournalNumber(
            result
                .chunk_count
                .0
                .checked_add(1)
                .ok_or(OperationError::InvalidContent)?,
        );
    }
    Ok(result)
}

/// Store must perform these reads under its eventual CAS/write transaction.
/// All methods are narrowly keyed; never load an entire assistant to implement them.
pub trait AssistantJournalStateReader {
    /// None means never declared. A persisted tombstone MUST return RetiredPart,
    /// not None; retired Part IDs cannot be resurrected with generation one.
    fn load_part_head(&self, part_id: &str) -> Result<Option<JournalPartHead>>;
    fn verify_content_reference(
        &self,
        identity: &AssistantJournalIdentity,
        part_id: &str,
        content: &JournalContentRef,
    ) -> Result<()>;
    fn verify_chunk_reference(&self, chunk: &JournalChunkRef) -> Result<()>;
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JournalHeadChange {
    pub part_id: String,
    /// Exact old head for Store CAS (contains refs, never body bytes).
    pub expected: Option<JournalPartHead>,
    pub replacement: Option<JournalPartHead>,
    /// Last live head retained to persist a tombstone, including declare+remove
    /// in one batch. Never treat replacement=None as "the ID never existed".
    pub removed_head: Option<JournalPartHead>,
}

fn valid_id(value: &str) -> Result<()> {
    if value.trim().is_empty() || value.len() > 1024 || value.chars().any(char::is_control) {
        return Err(OperationError::InvalidIdentity);
    }
    Ok(())
}
fn increment(n: JournalNumber) -> Result<JournalNumber> {
    n.0.checked_add(1)
        .map(JournalNumber)
        .ok_or(OperationError::InvalidSequence)
}
fn check_expected(
    head: &JournalPartHead,
    generation: JournalNumber,
    revision: JournalNumber,
) -> Result<()> {
    if head.generation != generation {
        return Err(OperationError::GenerationConflict);
    }
    if head.revision != revision {
        return Err(OperationError::HeadConflict);
    }
    Ok(())
}
fn allowed_field(metadata: &JournalPartMetadata, field: JournalContentField) -> bool {
    use JournalContentField as F;
    match metadata {
        JournalPartMetadata::Text(m) => {
            m.visibility == JournalVisibility::Public && field == F::Markdown
        }
        JournalPartMetadata::Reasoning(_) => matches!(field, F::ReasoningSummary | F::Title),
        JournalPartMetadata::Step(_) => field == F::Title,
        JournalPartMetadata::Tool(_) => matches!(
            field,
            F::ToolInput
                | F::ToolOutput
                | F::ToolOutputText
                | F::ToolResources
                | F::Title
                | F::ToolSummary
                | F::PermissionDomain
                | F::PermissionRationale
                | F::PermissionDecision
                | F::OriginProviderId
                | F::OriginProviderTitle
                | F::OriginNamespace
                | F::ToolArtifacts
        ),
        JournalPartMetadata::Source(_) => {
            matches!(
                field,
                F::SourceSnippet | F::Title | F::Uri | F::MimeType | F::SourceEvidence
            )
        }
        JournalPartMetadata::Task(_) | JournalPartMetadata::TaskControl(_) => matches!(field, F::TaskCurrentStep | F::ResultReference),
        JournalPartMetadata::Skill(_) => {
            matches!(field, F::Title | F::ResultReference | F::FailureMessage)
        }
        JournalPartMetadata::ModelAttempt(_) | JournalPartMetadata::Error(_) => {
            field == F::FailureMessage
        }
        JournalPartMetadata::UiSurface(_) => false,
        JournalPartMetadata::Historical(_) => field == F::HistoricalPreview,
        JournalPartMetadata::Citation(_) => {
            matches!(field, F::Title | F::Uri | F::MimeType | F::CitationSection)
        }
        JournalPartMetadata::Attachment(_) => {
            matches!(field, F::Name | F::Uri | F::MimeType | F::Operation)
        }
    }
}
fn validate_metadata(metadata: &JournalPartMetadata) -> Result<()> {
    let ids: Vec<&str> = match metadata {
        JournalPartMetadata::Text(m) => {
            let public = m.review.is_none() || m.review == Some(TextReviewState::Approved);
            if public != (m.visibility == JournalVisibility::Public) {
                return Err(OperationError::InvalidMetadata);
            }
            vec![]
        }
        JournalPartMetadata::Reasoning(_) | JournalPartMetadata::Step(_) => vec![],
        JournalPartMetadata::Tool(m) => {
            if m.artifacts != JournalContentField::ToolArtifacts {
                return Err(OperationError::InvalidMetadata);
            }
            let mut ids = vec![m.tool_call_id.as_str(), m.capability_id.as_str()];
            if let Some(permission) = &m.permission {
                ids.push(&permission.request_id);
            }
            if let Some(output) = &m.full_output {
                ids.extend([
                    output.message_id.0.as_str(),
                    output.part_id.0.as_str(),
                    output.tool_call_id.0.as_str(),
                ]);
            }
            ids
        }
        JournalPartMetadata::Source(m) => {
            if m.evidence
                .is_some_and(|field| field != JournalContentField::SourceEvidence)
            {
                return Err(OperationError::InvalidMetadata);
            }
            vec![&m.source_id]
        }
        JournalPartMetadata::Citation(m) => {
            crate::CitationRange::from_public_bounds(
                m.byte_range.as_ref().map(|range| range.start.0),
                m.byte_range.as_ref().map(|range| range.end.0),
            )
            .map_err(|_| OperationError::InvalidMetadata)?;
            vec![&m.citation_id, &m.source_id, &m.origin_tool_call_id]
        }
        JournalPartMetadata::Attachment(m) => vec![&m.attachment_id],
        JournalPartMetadata::Task(m) => {
            vec![&m.task_id, &m.display_key]
        }
        JournalPartMetadata::TaskControl(m) => {
            vec![&m.task.task_id, &m.task.display_key]
        }
        JournalPartMetadata::Skill(m) => vec![&m.run_id, &m.skill_id],
        JournalPartMetadata::ModelAttempt(m) => {
            if m.http_status.is_some_and(|s| !(100..=599).contains(&s.0)) {
                return Err(OperationError::InvalidMetadata);
            }
            if m.fallback_attempt_id.is_none()
                && (m.fallback_provider.is_some() || m.fallback_model.is_some())
            {
                return Err(OperationError::InvalidMetadata);
            }
            vec![&m.attempt_id, &m.failure_code]
        }
        JournalPartMetadata::UiSurface(m) => {
            m.surface.validate().map_err(|_| OperationError::InvalidMetadata)?;
            if m.summary.len() > crate::generative_ui::UI_MAX_SUMMARY_BYTES { return Err(OperationError::InvalidMetadata); }
            vec![]
        }
        JournalPartMetadata::Error(m) => vec![&m.code, &m.display_key],
        JournalPartMetadata::Historical(m) => {
            if m.canonical_revision.0 == 0 {
                return Err(OperationError::InvalidMetadata);
            }
            vec![&m.canonical_kind]
        }
    };
    for id in ids {
        valid_id(id)?;
    }
    for (expected, value) in metadata_text_values(metadata) {
        match value {
            JournalTextValue::Field(field) if *field != expected => {
                return Err(OperationError::InvalidMetadata)
            }
            _ => {}
        }
    }
    if matches!(metadata, JournalPartMetadata::Tool(JournalToolMetadata { affected_resources: JournalResourceList::Field(field), .. }) if *field != JournalContentField::ToolResources)
    {
        return Err(OperationError::InvalidMetadata);
    }
    Ok(())
}
fn metadata_text_values(
    metadata: &JournalPartMetadata,
) -> Vec<(JournalContentField, &JournalTextValue)> {
    use JournalContentField as F;
    match metadata {
        JournalPartMetadata::Step(m) => vec![(F::Title, &m.title)],
        JournalPartMetadata::Skill(m) => vec![(F::Title, &m.display_name)],
        JournalPartMetadata::Reasoning(m) => vec![(F::Title, &m.title)],
        JournalPartMetadata::Tool(m) => {
            let mut values = vec![(F::Title, &m.title)];
            for (field, value) in [
                (F::ToolSummary, m.summary.as_ref()),
                (F::OriginProviderId, m.origin.provider_id.as_ref()),
                (F::OriginProviderTitle, m.origin.provider_title.as_ref()),
                (F::OriginNamespace, m.origin.namespace.as_ref()),
            ] {
                if let Some(value) = value {
                    values.push((field, value));
                }
            }
            if let Some(permission) = &m.permission {
                values.extend([
                    (F::PermissionDomain, &permission.permission_domain),
                    (F::PermissionRationale, &permission.rationale),
                ]);
                if let Some(value) = &permission.decision {
                    values.push((F::PermissionDecision, value));
                }
            }
            values
        }
        JournalPartMetadata::Source(m) => [
            (F::Title, Some(&m.title)),
            (F::Uri, Some(&m.uri)),
            (F::MimeType, m.mime_type.as_ref()),
        ]
        .into_iter()
        .filter_map(|(field, v)| v.map(|v| (field, v)))
        .collect(),
        JournalPartMetadata::Citation(m) => [
            (F::Title, Some(&m.title)),
            (F::Uri, Some(&m.uri)),
            (F::MimeType, m.mime_type.as_ref()),
            (
                F::CitationSection,
                m.location
                    .as_ref()
                    .and_then(|location| location.section.as_ref()),
            ),
        ]
        .into_iter()
        .filter_map(|(field, value)| value.map(|value| (field, value)))
        .collect(),
        JournalPartMetadata::Attachment(m) => [
            (F::Name, Some(&m.name)),
            (F::Uri, Some(&m.uri)),
            (F::MimeType, m.mime_type.as_ref()),
            (F::Operation, m.operation.as_ref()),
        ]
        .into_iter()
        .filter_map(|(field, v)| v.map(|v| (field, v)))
        .collect(),
        _ => vec![],
    }
}
fn validate_head(head: &JournalPartHead, identity: &AssistantJournalIdentity) -> Result<()> {
    if &head.identity != identity {
        return Err(OperationError::OwnerMismatch);
    }
    valid_id(&head.part_id)?;
    validate_metadata(&head.metadata)?;
    if let JournalPartMetadata::UiSurface(metadata) = &head.metadata {
        let owner = &metadata.surface.identity.owner;
        if owner.authority_id != identity.source_store_epoch || owner.conversation_id != identity.conversation_id
            || owner.originating_run_id != identity.run_id || owner.message_id != identity.assistant_message_id {
            return Err(OperationError::OwnerMismatch);
        }
    }
    if head.generation.0 == 0
        || head.revision.0 == 0
        || head.metadata_revision.0 == 0
        || head.metadata_revision.0 > head.revision.0
    {
        return Err(OperationError::InvalidSequence);
    }
    if head.bodies.len() > MAX_PART_CONTENT_FIELDS {
        return Err(OperationError::Budget);
    }
    let mut seen = BTreeSet::new();
    for body in &head.bodies {
        if !seen.insert(body.field)
            || !allowed_field(&head.metadata, body.field)
            || body.content_generation.0 == 0
            || body.content_generation.0 > head.revision.0
        {
            return Err(OperationError::InvalidContent);
        }
        if (body.utf8_bytes.0 == 0) != (body.chunk_count.0 == 0)
            || body.chunk_count.0 > body.utf8_bytes.0
        {
            return Err(OperationError::InvalidContent);
        }
        if body.utf8_bytes.0
            > body
                .chunk_count
                .0
                .saturating_mul(ASSISTANT_CHUNK_MAX_BYTES as u64)
        {
            return Err(OperationError::InvalidContent);
        }
        if body.chunk_count.0 == 0
            && *body
                != empty_journal_content(
                    identity,
                    &head.part_id,
                    body.field,
                    body.content_generation,
                )?
        {
            return Err(OperationError::ContentConflict);
        }
    }
    for (_, value) in metadata_text_values(&head.metadata) {
        if let JournalTextValue::Field(field) = value {
            if !head.bodies.iter().any(|body| body.field == *field) {
                return Err(OperationError::InvalidContent);
            }
        }
    }
    if let JournalPartMetadata::Tool(metadata) = &head.metadata {
        if let crate::code_mode::CodeItemSource::CodeCell { cell } = &metadata.execution_source {
            cell.validate().map_err(|_| OperationError::InvalidContent)?;
        }
        if let Some(output) = &metadata.full_output {
            if output.message_id.0 != identity.assistant_message_id
                || output.part_id.0 != head.part_id
                || output.tool_call_id.0 != metadata.tool_call_id
            {
                return Err(OperationError::OwnerMismatch);
            }
        }
        if !head
            .bodies
            .iter()
            .any(|body| body.field == metadata.artifacts)
        {
            return Err(OperationError::InvalidContent);
        }
        if matches!(metadata.affected_resources, JournalResourceList::Field(_))
            && !head
                .bodies
                .iter()
                .any(|c| c.field == JournalContentField::ToolResources)
        {
            return Err(OperationError::InvalidContent);
        }
    }
    if let JournalPartMetadata::Source(metadata) = &head.metadata {
        if metadata
            .evidence
            .is_some_and(|field| !head.bodies.iter().any(|body| body.field == field))
        {
            return Err(OperationError::InvalidContent);
        }
    }
    // An Attempt card exists only for a concrete public failure. Its text is
    // independent of bounded failure metadata, including an explicitly empty
    // message. A missing field must not masquerade as a complete failure.
    if matches!(head.metadata, JournalPartMetadata::ModelAttempt(_))
        && !head
            .bodies
            .iter()
            .any(|body| body.field == JournalContentField::FailureMessage)
    {
        return Err(OperationError::InvalidContent);
    }
    Ok(())
}

fn validate_operation_shape(
    operation: &AssistantJournalOperationV1,
    identity: &AssistantJournalIdentity,
) -> Result<()> {
    match operation {
        AssistantJournalOperationV1::Declare(v) => validate_head(&v.part, identity),
        AssistantJournalOperationV1::Replace(v) => {
            valid_id(&v.part_id)?;
            if v.part_id != v.part.part_id
                || v.expected_generation.0 == 0
                || v.expected_revision.0 == 0
            {
                return Err(OperationError::InvalidSequence);
            }
            validate_head(&v.part, identity)
        }
        AssistantJournalOperationV1::Append(v) => {
            valid_id(&v.part_id)?;
            if v.expected_generation.0 == 0
                || v.expected_revision.0 == 0
                || v.expected.content_generation.0 == 0
                || v.expected.content_generation.0 > v.expected_revision.0
                || v.expected.chunk_count.0 > v.expected.utf8_bytes.0
                || (v.expected.chunk_count.0 == 0) != (v.expected.utf8_bytes.0 == 0)
                || append_journal_content(&v.expected, &v.chunks)? != v.resulting
            {
                return Err(OperationError::InvalidContent);
            }
            Ok(())
        }
        AssistantJournalOperationV1::Update(v) => {
            valid_id(&v.part_id)?;
            if v.expected_generation.0 == 0 || v.expected_revision.0 == 0 {
                return Err(OperationError::InvalidSequence);
            }
            validate_metadata(&v.metadata)
        }
        AssistantJournalOperationV1::Remove(v) => {
            valid_id(&v.part_id)?;
            if v.expected_generation.0 == 0 || v.expected_revision.0 == 0 {
                return Err(OperationError::InvalidSequence);
            }
            Ok(())
        }
        AssistantJournalOperationV1::ReplaceContent(v) => {
            valid_id(&v.part_id)?;
            if v.expected_generation.0 == 0
                || v.expected_revision.0 == 0
                || (v.expected.is_none() && v.resulting.is_none())
                || v.expected.as_ref().is_some_and(|c| c.field != v.field)
                || v.resulting.as_ref().is_some_and(|c| {
                    c.field != v.field
                        || Some(c.content_generation.0) != v.expected_revision.0.checked_add(1)
                })
            {
                return Err(OperationError::InvalidContent);
            }
            Ok(())
        }
    }
}

/// Work/memory scale with this batch's affected heads, not all historical Parts.
/// Returned changes are plans only; no cursor is advanced or content published.
pub fn validate_assistant_batch<R: AssistantJournalStateReader>(
    reader: &R,
    batch: &AssistantOperationBatchV1,
) -> Result<Vec<JournalHeadChange>> {
    batch.canonical_bytes()?;
    let mut changes: BTreeMap<String, JournalHeadChange> = BTreeMap::new();
    let mut retired = BTreeSet::new();
    for op in &batch.operations {
        let id = match op {
            AssistantJournalOperationV1::Declare(v) => &v.part.part_id,
            AssistantJournalOperationV1::Append(v) => &v.part_id,
            AssistantJournalOperationV1::Replace(v) => &v.part_id,
            AssistantJournalOperationV1::Update(v) => &v.part_id,
            AssistantJournalOperationV1::Remove(v) => &v.part_id,
            AssistantJournalOperationV1::ReplaceContent(v) => &v.part_id,
        };
        valid_id(id)?;
        if !changes.contains_key(id) {
            let old = reader.load_part_head(id)?;
            if let Some(head) = &old {
                validate_head(head, &batch.identity)?;
                if head.part_id != *id {
                    return Err(OperationError::OwnerMismatch);
                }
            }
            changes.insert(
                id.clone(),
                JournalHeadChange {
                    part_id: id.clone(),
                    expected: old.clone(),
                    replacement: old,
                    removed_head: None,
                },
            );
        }
        let entry = changes.get_mut(id).expect("inserted affected head");
        match op {
            AssistantJournalOperationV1::Declare(v) => {
                if retired.contains(id) {
                    return Err(OperationError::RetiredPart);
                }
                if entry.replacement.is_some() || entry.expected.is_some() {
                    return Err(OperationError::ExistingPart);
                }
                if v.part.generation.0 != 1
                    || v.part.revision.0 != 1
                    || v.part.metadata_revision.0 != 1
                {
                    return Err(OperationError::InvalidSequence);
                }
                validate_head(&v.part, &batch.identity)?;
                for body in &v.part.bodies {
                    if body.content_generation.0 != 1 {
                        return Err(OperationError::GenerationConflict);
                    }
                    reader.verify_content_reference(&batch.identity, id, body)?;
                }
                entry.replacement = Some(v.part.clone());
            }
            AssistantJournalOperationV1::Append(v) => {
                let head = entry
                    .replacement
                    .as_mut()
                    .ok_or(OperationError::MissingPart)?;
                check_expected(head, v.expected_generation, v.expected_revision)?;
                if matches!(&head.metadata,JournalPartMetadata::Text(m) if m.is_final)
                    || matches!(&head.metadata,JournalPartMetadata::Reasoning(m) if matches!(m.state,ActivityState::Completed|ActivityState::Cancelled|ActivityState::Failed))
                    || matches!(&head.metadata,JournalPartMetadata::Tool(m) if matches!(m.state,ActivityState::Completed|ActivityState::Cancelled|ActivityState::Failed))
                {
                    return Err(OperationError::InvalidContent);
                }
                if !matches!(
                    v.expected.field,
                    JournalContentField::Markdown
                        | JournalContentField::ReasoningSummary
                        | JournalContentField::ToolOutputText
                        | JournalContentField::Title
                ) {
                    return Err(OperationError::InvalidContent);
                }
                if v.expected.field == JournalContentField::Title
                    && !matches!(head.metadata, JournalPartMetadata::Reasoning(_))
                {
                    return Err(OperationError::InvalidContent);
                }
                let current = head
                    .bodies
                    .iter_mut()
                    .find(|b| b.field == v.expected.field)
                    .ok_or(OperationError::InvalidContent)?;
                if current != &v.expected
                    || append_journal_content(current, &v.chunks)? != v.resulting
                {
                    return Err(OperationError::ContentConflict);
                }
                for chunk in &v.chunks {
                    reader.verify_chunk_reference(chunk)?;
                }
                *current = v.resulting.clone();
                head.revision = increment(head.revision)?;
                validate_head(head, &batch.identity)?;
            }
            AssistantJournalOperationV1::Replace(v) => {
                let old = entry
                    .replacement
                    .as_ref()
                    .ok_or(OperationError::MissingPart)?;
                check_expected(old, v.expected_generation, v.expected_revision)?;
                validate_tool_identity(&old.metadata, &v.part.metadata)?;
                if v.part.part_id != *id
                    || v.part.generation != increment(old.generation)?
                    || v.part.revision != increment(old.revision)?
                    || v.part.metadata_revision.0 != 1
                    || v.part.order_key != old.order_key
                {
                    return Err(OperationError::GenerationConflict);
                }
                validate_head(&v.part, &batch.identity)?;
                for body in &v.part.bodies {
                    if !old.bodies.iter().any(|existing| existing == body)
                        && body.content_generation != v.part.revision
                    {
                        return Err(OperationError::GenerationConflict);
                    }
                    reader.verify_content_reference(&batch.identity, id, body)?;
                }
                entry.replacement = Some(v.part.clone());
            }
            AssistantJournalOperationV1::Update(v) => {
                let head = entry
                    .replacement
                    .as_mut()
                    .ok_or(OperationError::MissingPart)?;
                check_expected(head, v.expected_generation, v.expected_revision)?;
                if std::mem::discriminant(&head.metadata) != std::mem::discriminant(&v.metadata) {
                    return Err(OperationError::InvalidMetadata);
                }
                validate_tool_identity(&head.metadata, &v.metadata)?;
                head.metadata = v.metadata.clone();
                head.metadata_revision = increment(head.metadata_revision)?;
                head.revision = increment(head.revision)?;
                validate_head(head, &batch.identity)?;
            }
            AssistantJournalOperationV1::Remove(v) => {
                let old = entry
                    .replacement
                    .as_ref()
                    .ok_or(OperationError::MissingPart)?;
                check_expected(old, v.expected_generation, v.expected_revision)?;
                entry.removed_head = Some(old.clone());
                entry.replacement = None;
                retired.insert(id.clone());
            }
            AssistantJournalOperationV1::ReplaceContent(v) => {
                let head = entry
                    .replacement
                    .as_mut()
                    .ok_or(OperationError::MissingPart)?;
                check_expected(head, v.expected_generation, v.expected_revision)?;
                let current = head.bodies.iter().find(|c| c.field == v.field);
                if current != v.expected.as_ref() {
                    return Err(OperationError::ContentConflict);
                }
                if let Some(resulting) = &v.resulting {
                    reader.verify_content_reference(&batch.identity, id, resulting)?;
                }
                head.bodies.retain(|c| c.field != v.field);
                if let Some(resulting) = &v.resulting {
                    head.bodies.push(resulting.clone());
                }
                head.revision = increment(head.revision)?;
                validate_head(head, &batch.identity)?;
            }
        }
    }
    Ok(changes.into_values().collect())
}

#[cfg(test)]
#[path = "assistant_journal_operations_tests.rs"]
mod tests;
