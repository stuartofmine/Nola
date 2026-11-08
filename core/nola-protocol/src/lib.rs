// SPDX-License-Identifier: Apache-2.0
//! Versioned wire contracts shared by Nola Application Service clients.

pub mod historical_projection;
pub use historical_projection::*;
pub mod archive_restore;
pub mod transcript_export;
pub use nola_domain::tool_source::{ToolSourceClaim, ToolSourceExportRange};
pub use archive_restore::*;
use nola_domain::ConversationProjectionMutation;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::error::Error;
use std::fmt;
pub use transcript_export::{
    CanonicalPartOwner, ExportBodyField, ExportCommand, ExportPage, ExportRange, PartDerivation,
    PartDerivationProvenance, StoredPartDerivation,
};

pub mod command_outcome;
pub use command_outcome::command_outcome_requires_owner_reopen;
pub mod command_metadata;
pub use command_metadata::CommandMetadata;
pub mod content_upload;
pub mod conversation_recall;
pub mod replica_sync;
pub mod transcript;
pub use conversation_recall::{
    ConversationRecallCommand, ConversationRecallHit, ConversationTextRange,
};
pub use replica_sync::*;
pub mod conversation_transcript;
pub mod uploaded_transcript;
pub use conversation_transcript::{
    AdmitConversationUserMessage, ConversationTranscriptImportSource, ImportConversationTranscript, TranscriptCommand,
};
pub use transcript::{TranscriptMessageInput, TranscriptPartInput};

// Reissued 1.0.0 establishes protocol 1 as the first supported release baseline.
// Future incompatible changes require explicit negotiation and compatibility tests.
// The protocol identity remains part of the local socket name, preventing reuse
// of an incompatible persistent service. Development-era formats are unsupported.
pub const APPLICATION_SERVICE_PROTOCOL_VERSION: u32 = 1;
pub const APPLICATION_SERVICE_MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;
pub const COMMAND_MAX_CONTROL_BYTES: usize = 256 * 1024;
pub const EVENT_MAX_CONTROL_BYTES: usize = 64 * 1024;
pub const MAX_PAGE_ITEMS: u32 = 512;

/// Trusted local Host transfer, carried by the existing permission-protected
/// service transport. The digest is integrity evidence, not authentication.
/// This is not a model/tool command or a general Application JSON admission.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HostTaskCancellationImport {
    pub authorization: nola_domain::agent_task_admission::AgentTaskCancellationAuthorization,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HostAgentTaskAdmissionImport {
    pub admission: nola_domain::agent_task_admission::AgentTaskAdmission,
}

pub use nola_domain::agent_task_admission::{
    AgentTaskAdmission, RuntimeAgentTaskAuthority, RuntimeAgentTaskAuthorityPhase,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HostAgentTaskTerminalProjectionImport {
    pub projection: nola_domain::agent_task_admission::AgentTaskTerminalProjection,
}

pub use nola_domain::agent_task_admission::{AgentTaskTerminalProjection, TaskTerminalReconciliationApplied};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HostAgentTaskGuidanceProjectionImport {
    pub projection: nola_domain::agent_task_admission::AgentTaskGuidanceProjection,
}
pub use nola_domain::agent_task_admission::{AgentTaskGuidanceProjection, TaskGuidanceProjectionApplied};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CanonicalReplicaChange {
    pub operation_id: String,
    pub entity_type: String,
    pub entity_id: String,
    pub revision: i64,
    pub deleted: bool,
    pub payload: Value,
    pub origin_device_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CanonicalReplicaPushRequest {
    pub changes: Vec<CanonicalReplicaChange>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CanonicalReplicaPushResult {
    pub operation_id: String,
    pub status: String,
    pub current_revision: i64,
    pub cursor: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CanonicalReplicaPushResponse {
    pub results: Vec<CanonicalReplicaPushResult>,
    pub server_time: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CanonicalReplicaCommittedChange {
    pub cursor: i64,
    pub operation_id: String,
    pub entity_type: String,
    pub entity_id: String,
    pub revision: i64,
    pub deleted: bool,
    pub payload: Value,
    pub origin_device_id: String,
    pub committed_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CanonicalReplicaPullResponse {
    pub changes: Vec<CanonicalReplicaCommittedChange>,
    pub next_cursor: i64,
    pub has_more: bool,
    pub server_time: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProtocolValidationCode {
    InvalidProtocolVersion,
    MissingValue,
    InvalidIdentifier,
    InvalidTypeTag,
    InvalidPayload,
    InvalidState,
    InvalidLimit,
    InvalidRange,
    DuplicateIdentity,
    ScopeMismatch,
    SequenceGap,
    FrameTooLarge,
    SerializationFailure,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProtocolValidationError {
    pub field: String,
    pub code: ProtocolValidationCode,
    pub message: String,
}

impl fmt::Display for ProtocolValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.field, self.message)
    }
}

impl Error for ProtocolValidationError {}

fn validation_error(
    field: impl Into<String>,
    code: ProtocolValidationCode,
    message: impl Into<String>,
) -> ProtocolValidationError {
    ProtocolValidationError {
        field: field.into(),
        code,
        message: message.into(),
    }
}

fn validate_protocol_version(protocol_version: u32) -> Result<(), ProtocolValidationError> {
    if protocol_version != APPLICATION_SERVICE_PROTOCOL_VERSION {
        return Err(validation_error(
            "protocol_version",
            ProtocolValidationCode::InvalidProtocolVersion,
            format!(
                "expected protocol version {}, received {protocol_version}",
                APPLICATION_SERVICE_PROTOCOL_VERSION
            ),
        ));
    }
    Ok(())
}

fn validate_required_text(field: &str, value: &str) -> Result<(), ProtocolValidationError> {
    if value.trim().is_empty() {
        return Err(validation_error(
            field,
            ProtocolValidationCode::MissingValue,
            "value must not be empty",
        ));
    }
    if value.chars().any(char::is_control) {
        return Err(validation_error(
            field,
            ProtocolValidationCode::InvalidIdentifier,
            "value must not contain control characters",
        ));
    }
    Ok(())
}

fn validate_type_tag(field: &str, value: &str) -> Result<(), ProtocolValidationError> {
    validate_required_text(field, value)?;
    let mut characters = value.chars();
    if !characters
        .next()
        .is_some_and(|character| character.is_ascii_lowercase())
        || !characters.all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_'
        })
    {
        return Err(validation_error(
            field,
            ProtocolValidationCode::InvalidTypeTag,
            "type tag must use lower snake_case",
        ));
    }
    Ok(())
}

fn validate_object(field: &str, value: &Value) -> Result<(), ProtocolValidationError> {
    if !value.is_object() {
        return Err(validation_error(
            field,
            ProtocolValidationCode::InvalidPayload,
            "payload must be a JSON object",
        ));
    }
    Ok(())
}

fn validate_serialized_size<T: Serialize>(
    field: &str,
    value: &T,
    maximum: usize,
) -> Result<(), ProtocolValidationError> {
    let size = serde_json::to_vec(value)
        .map_err(|error| {
            validation_error(
                field,
                ProtocolValidationCode::SerializationFailure,
                error.to_string(),
            )
        })?
        .len();
    if size > maximum {
        return Err(validation_error(
            field,
            ProtocolValidationCode::FrameTooLarge,
            format!("serialized frame is {size} bytes; maximum is {maximum}"),
        ));
    }
    Ok(())
}

fn validate_page_limit(field: &str, limit: u32) -> Result<(), ProtocolValidationError> {
    if limit == 0 || limit > MAX_PAGE_ITEMS {
        return Err(validation_error(
            field,
            ProtocolValidationCode::InvalidLimit,
            format!("limit must be between 1 and {MAX_PAGE_ITEMS}"),
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CommandPayload {
    #[serde(rename = "type")]
    pub command_type: String,
    pub arguments: Value,
}

impl CommandPayload {
    pub fn validate(&self) -> Result<(), ProtocolValidationError> {
        validate_type_tag("command.type", &self.command_type)?;
        validate_object("command.arguments", &self.arguments)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CommandEnvelope {
    pub protocol_version: u32,
    pub request_id: String,
    pub client_id: String,
    pub expected_revision: Option<u64>,
    pub command: CommandPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CommandReceiptLookupRequest {
    pub request_id: String,
}

impl CommandReceiptLookupRequest {
    pub fn validate(&self) -> Result<(), ProtocolValidationError> {
        validate_required_text("request_id", &self.request_id)
    }
}

impl CommandEnvelope {
    pub fn validate(&self) -> Result<(), ProtocolValidationError> {
        validate_protocol_version(self.protocol_version)?;
        validate_required_text("request_id", &self.request_id)?;
        validate_required_text("client_id", &self.client_id)?;
        self.command.validate()?;
        validate_serialized_size("command", self, APPLICATION_SERVICE_MAX_FRAME_BYTES)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ProtocolFailure {
    pub code: String,
    pub message: String,
    pub recoverable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

impl ProtocolFailure {
    pub fn validate(&self, field: &str) -> Result<(), ProtocolValidationError> {
        validate_type_tag(&format!("{field}.code"), &self.code)?;
        validate_required_text(&format!("{field}.message"), &self.message)?;
        if let Some(details) = &self.details {
            validate_object(&format!("{field}.details"), details)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RevisionConflict {
    pub scope_id: String,
    pub expected_revision: u64,
    pub actual_revision: u64,
}

impl RevisionConflict {
    pub fn validate(&self) -> Result<(), ProtocolValidationError> {
        validate_required_text("conflict.scope_id", &self.scope_id)?;
        if self.expected_revision == self.actual_revision {
            return Err(validation_error(
                "conflict.actual_revision",
                ProtocolValidationCode::InvalidState,
                "conflict revisions must differ",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum CommandReceiptOutcome {
    Accepted,
    Committed {
        completed_at: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        committed_revision: Option<u64>,
        result: Value,
    },
    Rejected {
        completed_at: String,
        error: ProtocolFailure,
    },
    Conflict {
        completed_at: String,
        conflict: RevisionConflict,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CommandReceipt {
    pub protocol_version: u32,
    pub request_id: String,
    pub client_id: String,
    pub received_at: String,
    #[serde(flatten)]
    pub outcome: CommandReceiptOutcome,
}

impl CommandReceipt {
    pub fn validate(&self) -> Result<(), ProtocolValidationError> {
        validate_protocol_version(self.protocol_version)?;
        validate_required_text("request_id", &self.request_id)?;
        validate_required_text("client_id", &self.client_id)?;
        validate_required_text("received_at", &self.received_at)?;
        match &self.outcome {
            CommandReceiptOutcome::Accepted => {}
            CommandReceiptOutcome::Committed {
                completed_at,
                committed_revision,
                ..
            } => {
                validate_required_text("completed_at", completed_at)?;
                if committed_revision == &Some(0) {
                    return Err(validation_error(
                        "committed_revision",
                        ProtocolValidationCode::InvalidRange,
                        "committed revision must be greater than zero when present",
                    ));
                }
            }
            CommandReceiptOutcome::Rejected {
                completed_at,
                error,
            } => {
                validate_required_text("completed_at", completed_at)?;
                error.validate("error")?;
            }
            CommandReceiptOutcome::Conflict {
                completed_at,
                conflict,
            } => {
                validate_required_text("completed_at", completed_at)?;
                conflict.validate()?;
            }
        }
        validate_serialized_size("receipt", self, APPLICATION_SERVICE_MAX_FRAME_BYTES)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProjectionScope {
    Conversations,
    Conversation { conversation_id: String },
    Run { run_id: String },
    Workspace { workspace_id: String },
}

impl ProjectionScope {
    pub fn validate(&self) -> Result<(), ProtocolValidationError> {
        match self {
            Self::Conversations => Ok(()),
            Self::Conversation { conversation_id } => {
                validate_required_text("scope.conversation_id", conversation_id)
            }
            Self::Run { run_id } => validate_required_text("scope.run_id", run_id),
            Self::Workspace { workspace_id } => {
                validate_required_text("scope.workspace_id", workspace_id)
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProjectionCursor {
    pub scope: ProjectionScope,
    pub sequence: u64,
}

impl ProjectionCursor {
    pub fn validate(&self) -> Result<(), ProtocolValidationError> {
        self.scope.validate()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ProjectionRecord {
    pub entity_type: String,
    pub entity_id: String,
    pub revision: u64,
    pub payload: Value,
}

impl ProjectionRecord {
    pub fn validate(&self, field: &str) -> Result<(), ProtocolValidationError> {
        validate_type_tag(&format!("{field}.entity_type"), &self.entity_type)?;
        validate_required_text(&format!("{field}.entity_id"), &self.entity_id)?;
        if self.revision == 0 {
            return Err(validation_error(
                format!("{field}.revision"),
                ProtocolValidationCode::InvalidRange,
                "revision must be greater than zero",
            ));
        }
        if self.payload.is_null() {
            return Err(validation_error(
                format!("{field}.payload"),
                ProtocolValidationCode::InvalidPayload,
                "payload must not be null",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ProjectionSnapshot {
    pub protocol_version: u32,
    pub scope: ProjectionScope,
    pub revision: u64,
    pub cursor: ProjectionCursor,
    pub generated_at: String,
    pub records: Vec<ProjectionRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<ProjectionPage>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProjectionPageRequest {
    pub offset: u64,
    pub limit: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_revision: Option<u64>,
}

impl ProjectionPageRequest {
    pub fn validate(&self) -> Result<(), ProtocolValidationError> {
        validate_page_limit("page.limit", self.limit)?;
        if self.expected_revision == Some(0) {
            return Err(validation_error(
                "page.expected_revision",
                ProtocolValidationCode::InvalidRange,
                "expected revision must be greater than zero when present",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProjectionPage {
    pub offset: u64,
    pub limit: u32,
    pub has_more: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProjectionSnapshotRequest {
    pub scope: ProjectionScope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<ProjectionPageRequest>,
}

impl ProjectionSnapshotRequest {
    pub fn validate(&self) -> Result<(), ProtocolValidationError> {
        self.scope.validate()?;
        if let Some(page) = &self.page {
            page.validate()?;
        }
        Ok(())
    }
}

impl ProjectionSnapshot {
    pub fn validate(&self) -> Result<(), ProtocolValidationError> {
        validate_protocol_version(self.protocol_version)?;
        self.scope.validate()?;
        self.cursor.validate()?;
        validate_required_text("generated_at", &self.generated_at)?;
        if self.cursor.scope != self.scope {
            return Err(validation_error(
                "cursor.scope",
                ProtocolValidationCode::ScopeMismatch,
                "snapshot cursor scope must match snapshot scope",
            ));
        }
        let mut identities = HashSet::new();
        for (index, record) in self.records.iter().enumerate() {
            record.validate(&format!("records[{index}]"))?;
            if !identities.insert((&record.entity_type, &record.entity_id)) {
                return Err(validation_error(
                    format!("records[{index}]"),
                    ProtocolValidationCode::DuplicateIdentity,
                    "snapshot contains duplicate entity identity",
                ));
            }
        }
        if let Some(page) = &self.page {
            validate_page_limit("page.limit", page.limit)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProjectionEventIdentity {
    pub event_id: String,
    pub conversation_id: Option<String>,
    pub run_id: Option<String>,
    pub attempt_id: Option<String>,
    pub delegated_invocation_id: Option<String>,
    pub message_id: Option<String>,
    pub part_id: Option<String>,
    pub call_id: Option<String>,
}

impl ProjectionEventIdentity {
    pub fn validate(&self) -> Result<(), ProtocolValidationError> {
        validate_required_text("identity.event_id", &self.event_id)?;
        for (field, value) in [
            ("identity.conversation_id", &self.conversation_id),
            ("identity.run_id", &self.run_id),
            ("identity.attempt_id", &self.attempt_id),
            (
                "identity.delegated_invocation_id",
                &self.delegated_invocation_id,
            ),
            ("identity.message_id", &self.message_id),
            ("identity.part_id", &self.part_id),
            ("identity.call_id", &self.call_id),
        ] {
            if let Some(value) = value {
                validate_required_text(field, value)?;
            }
        }
        if self.attempt_id.is_some() && self.delegated_invocation_id.is_some() {
            return Err(validation_error(
                "identity.delegated_invocation_id",
                ProtocolValidationCode::InvalidState,
                "model attempt and delegated invocation ownership are mutually exclusive",
            ));
        }
        if (self.attempt_id.is_some() || self.delegated_invocation_id.is_some())
            && self.run_id.is_none()
        {
            return Err(validation_error(
                "identity.attempt_id",
                ProtocolValidationCode::InvalidState,
                "attempt identity requires run identity",
            ));
        }
        if self.part_id.is_some() && self.message_id.is_none() {
            return Err(validation_error(
                "identity.part_id",
                ProtocolValidationCode::InvalidState,
                "part identity requires message identity",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProjectionMutation {
    Conversation {
        mutation: ConversationProjectionMutation,
    },
    Upsert {
        record: ProjectionRecord,
    },
    Remove {
        entity_type: String,
        entity_id: String,
        revision: u64,
    },
}

impl ProjectionMutation {
    fn validate(&self) -> Result<(), ProtocolValidationError> {
        match self {
            Self::Conversation { .. } => Ok(()),
            Self::Upsert { record } => record.validate("mutation.record"),
            Self::Remove {
                entity_type,
                entity_id,
                revision,
            } => {
                validate_type_tag("mutation.entity_type", entity_type)?;
                validate_required_text("mutation.entity_id", entity_id)?;
                if *revision == 0 {
                    return Err(validation_error(
                        "mutation.revision",
                        ProtocolValidationCode::InvalidRange,
                        "revision must be greater than zero",
                    ));
                }
                Ok(())
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ProjectionEvent {
    pub protocol_version: u32,
    pub scope: ProjectionScope,
    pub cursor: ProjectionCursor,
    pub identity: ProjectionEventIdentity,
    pub mutation: ProjectionMutation,
    pub occurred_at: String,
}

impl ProjectionEvent {
    pub fn validate(&self) -> Result<(), ProtocolValidationError> {
        validate_protocol_version(self.protocol_version)?;
        self.scope.validate()?;
        self.cursor.validate()?;
        self.identity.validate()?;
        self.mutation.validate()?;
        validate_required_text("occurred_at", &self.occurred_at)?;
        if self.cursor.scope != self.scope {
            return Err(validation_error(
                "cursor.scope",
                ProtocolValidationCode::ScopeMismatch,
                "event cursor scope must match event scope",
            ));
        }
        if self.cursor.sequence == 0 {
            return Err(validation_error(
                "cursor.sequence",
                ProtocolValidationCode::InvalidRange,
                "event sequence must be greater than zero",
            ));
        }
        match &self.scope {
            ProjectionScope::Conversation { conversation_id }
                if self.identity.conversation_id.as_ref() != Some(conversation_id) =>
            {
                return Err(validation_error(
                    "identity.conversation_id",
                    ProtocolValidationCode::ScopeMismatch,
                    "event conversation identity must match scope",
                ));
            }
            ProjectionScope::Run { run_id } if self.identity.run_id.as_ref() != Some(run_id) => {
                return Err(validation_error(
                    "identity.run_id",
                    ProtocolValidationCode::ScopeMismatch,
                    "event run identity must match scope",
                ));
            }
            _ => {}
        }
        if matches!(&self.mutation, ProjectionMutation::Conversation { .. })
            && !matches!(&self.scope, ProjectionScope::Conversation { .. })
        {
            return Err(validation_error(
                "mutation",
                ProtocolValidationCode::ScopeMismatch,
                "conversation mutations require a conversation scope",
            ));
        }
        if let ProjectionMutation::Conversation { mutation } = &self.mutation {
            let expected_message = mutation.message_id().map(|identity| &identity.0);
            let expected_part = mutation.part_id().map(|identity| &identity.0);
            if self.identity.message_id.as_ref() != expected_message
                || self.identity.part_id.as_ref() != expected_part
            {
                return Err(validation_error(
                    "identity",
                    ProtocolValidationCode::ScopeMismatch,
                    "conversation mutation identity must match its message and Part",
                ));
            }
        }
        validate_serialized_size("event", self, EVENT_MAX_CONTROL_BYTES)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EventSuffixRequest {
    pub scope: ProjectionScope,
    pub after: ProjectionCursor,
    pub limit: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wait_ms: Option<u32>,
}

/// A wake hint for windowed clients, never a delivery acknowledgement.
/// No event bodies or transcript content are materialized by this query.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ProjectionCursorWaitRequest {
    pub after: ProjectionCursor,
    pub wait_ms: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RuntimeEffectWorkWaitRequest {
    pub conversation_ids: Vec<String>,
    pub limit: u32,
    pub wait_ms: u32,
}

pub use nola_domain::assistant_journal::AssistantJournalCursor;

/// Wait for a source-owned journal head; notification is not content or admission.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AssistantJournalWaitRequest {
    pub after: nola_domain::assistant_journal::AssistantJournalCursor,
    pub wait_ms: u32,
}
impl AssistantJournalWaitRequest {
    pub fn validate(&self) -> Result<(), String> {
        self.after.identity.validate().map_err(|error| error.to_string())?;
        if self.wait_ms > 30_000 { return Err("journal wait exceeds 30 seconds".into()); }
        Ok(())
    }
}

mod unknown_effect_recovery;
pub use unknown_effect_recovery::{ResolveUnknownEffectRecovery, UnknownEffectRecoveryChoice, UnknownEffectRecoveryReply, UnknownEffectRecoveryInspection};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeEffectWorkReason {
    Pending,
    Retry,
    Resume,
    RecoverExpiredLease,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeEffectWorkRunHead {
    pub conversation_id: String,
    pub run_id: String,
    pub effect_id: String,
    pub request_id: String,
    pub attempt_id: Option<String>,
    pub reason: RuntimeEffectWorkReason,
    pub ready_at_ms: i64,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeEffectWorkSchedule {
    pub observed_at_ms: i64,
    pub runs: Vec<RuntimeEffectWorkRunHead>,
    pub next_wake_at_ms: Option<i64>,
}

impl RuntimeEffectWorkSchedule {
    pub fn validate(&self, limit: u32) -> Result<(), ProtocolValidationError> {
        if self.observed_at_ms < 0 {
            return Err(validation_error(
                "observed_at_ms",
                ProtocolValidationCode::InvalidRange,
                "Runtime effect-work observation time must be nonnegative",
            ));
        }
        if self.runs.len() > limit as usize || limit == 0 || limit > 64 {
            return Err(validation_error(
                "runs",
                ProtocolValidationCode::InvalidLimit,
                "Runtime effect-work schedule exceeds its requested limit",
            ));
        }
        if self
            .next_wake_at_ms
            .is_some_and(|wake| wake <= self.observed_at_ms)
        {
            return Err(validation_error(
                "next_wake_at_ms",
                ProtocolValidationCode::InvalidRange,
                "Runtime effect-work wake must be later than its observation",
            ));
        }
        let mut run_ids = HashSet::new();
        let mut previous: Option<(u8, i64, i64, &str, &str)> = None;
        for (index, head) in self.runs.iter().enumerate() {
            for (field, value) in [
                ("conversation_id", head.conversation_id.as_str()),
                ("run_id", head.run_id.as_str()),
                ("effect_id", head.effect_id.as_str()),
                ("request_id", head.request_id.as_str()),
            ] {
                validate_required_text(&format!("runs[{index}].{field}"), value)?;
            }
            if head.ready_at_ms < 0
                || head.created_at_ms < 0
                || head.ready_at_ms > self.observed_at_ms
            {
                return Err(validation_error(
                    format!("runs[{index}].ready_at_ms"),
                    ProtocolValidationCode::InvalidRange,
                    "Runtime effect-work head must be ready at its observation",
                ));
            }
            match (head.reason, head.attempt_id.as_deref()) {
                (RuntimeEffectWorkReason::Pending, None) => {}
                (RuntimeEffectWorkReason::Pending, Some(_)) => {
                    return Err(validation_error(
                        format!("runs[{index}].attempt_id"),
                        ProtocolValidationCode::InvalidState,
                        "pending Runtime work must not name a delivery attempt",
                    ))
                }
                (_, Some(attempt_id)) => {
                    validate_required_text(&format!("runs[{index}].attempt_id"), attempt_id)?
                }
                (_, None) => {
                    return Err(validation_error(
                        format!("runs[{index}].attempt_id"),
                        ProtocolValidationCode::InvalidState,
                        "recovery Runtime work must name its delivery attempt",
                    ))
                }
            }
            if !run_ids.insert(head.run_id.as_str()) {
                return Err(validation_error(
                    format!("runs[{index}].run_id"),
                    ProtocolValidationCode::DuplicateIdentity,
                    "Runtime effect-work schedule contains a duplicate Run",
                ));
            }
            let reason_rank = match head.reason {
                RuntimeEffectWorkReason::RecoverExpiredLease => 0,
                RuntimeEffectWorkReason::Pending
                | RuntimeEffectWorkReason::Retry
                | RuntimeEffectWorkReason::Resume => 1,
            };
            let key = (
                reason_rank,
                head.ready_at_ms,
                head.created_at_ms,
                head.run_id.as_str(),
                head.effect_id.as_str(),
            );
            if previous.is_some_and(|previous| previous > key) {
                return Err(validation_error(
                    format!("runs[{index}]"),
                    ProtocolValidationCode::InvalidState,
                    "Runtime effect-work schedule is not in stable order",
                ));
            }
            previous = Some(key);
        }
        Ok(())
    }

    pub fn validate_for_request(
        &self,
        request: &RuntimeEffectWorkWaitRequest,
    ) -> Result<(), ProtocolValidationError> {
        request.validate()?;
        self.validate(request.limit)?;
        let scope = request
            .conversation_ids
            .iter()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        if let Some((index, _)) = self
            .runs
            .iter()
            .enumerate()
            .find(|(_, head)| !scope.contains(head.conversation_id.as_str()))
        {
            return Err(validation_error(
                format!("runs[{index}].conversation_id"),
                ProtocolValidationCode::ScopeMismatch,
                "Runtime effect-work head is outside the requested conversation scope",
            ));
        }
        Ok(())
    }
}

impl RuntimeEffectWorkWaitRequest {
    pub fn validate(&self) -> Result<(), ProtocolValidationError> {
        if !(1..=64).contains(&self.conversation_ids.len()) {
            return Err(validation_error(
                "conversation_ids",
                ProtocolValidationCode::InvalidLimit,
                "Runtime effect-work scope must contain between 1 and 64 conversations",
            ));
        }
        let mut conversations = HashSet::new();
        for (index, conversation_id) in self.conversation_ids.iter().enumerate() {
            validate_required_text(&format!("conversation_ids[{index}]"), conversation_id)?;
            if !conversations.insert(conversation_id.as_str()) {
                return Err(validation_error(
                    format!("conversation_ids[{index}]"),
                    ProtocolValidationCode::DuplicateIdentity,
                    "Runtime effect-work scope contains a duplicate conversation",
                ));
            }
        }
        if !(1..=64).contains(&self.limit) {
            return Err(validation_error(
                "limit",
                ProtocolValidationCode::InvalidLimit,
                "Runtime effect-work limit must be between 1 and 64",
            ));
        }
        if self.wait_ms > 30_000 {
            return Err(validation_error(
                "wait_ms",
                ProtocolValidationCode::InvalidRange,
                "Runtime effect-work wait must not exceed 30000 milliseconds",
            ));
        }
        Ok(())
    }
}

impl ProjectionCursorWaitRequest {
    pub fn validate(&self) -> Result<(), ProtocolValidationError> {
        self.after.validate()?;
        if self.wait_ms > 30_000 {
            return Err(validation_error(
                "wait_ms",
                ProtocolValidationCode::InvalidRange,
                "projection wait must not exceed 30000 milliseconds",
            ));
        }
        Ok(())
    }
}

impl EventSuffixRequest {
    pub fn validate(&self) -> Result<(), ProtocolValidationError> {
        self.scope.validate()?;
        self.after.validate()?;
        validate_page_limit("limit", self.limit)?;
        if self.wait_ms.is_some_and(|wait_ms| wait_ms > 30_000) {
            return Err(validation_error(
                "wait_ms",
                ProtocolValidationCode::InvalidRange,
                "projection wait must not exceed 30000 milliseconds",
            ));
        }
        if self.after.scope != self.scope {
            return Err(validation_error(
                "after.scope",
                ProtocolValidationCode::ScopeMismatch,
                "cursor scope must match requested scope",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EventSuffix {
    pub scope: ProjectionScope,
    pub after: ProjectionCursor,
    pub events: Vec<ProjectionEvent>,
    pub next_cursor: ProjectionCursor,
    pub has_more: bool,
}

impl EventSuffix {
    pub fn validate(&self) -> Result<(), ProtocolValidationError> {
        self.scope.validate()?;
        self.after.validate()?;
        self.next_cursor.validate()?;
        if self.after.scope != self.scope || self.next_cursor.scope != self.scope {
            return Err(validation_error(
                "cursor.scope",
                ProtocolValidationCode::ScopeMismatch,
                "all suffix cursors must match suffix scope",
            ));
        }
        let mut expected_sequence = self.after.sequence.checked_add(1).ok_or_else(|| {
            validation_error(
                "after.sequence",
                ProtocolValidationCode::InvalidRange,
                "cursor sequence cannot advance",
            )
        })?;
        let mut event_ids = HashSet::new();
        for (index, event) in self.events.iter().enumerate() {
            event.validate()?;
            if event.scope != self.scope {
                return Err(validation_error(
                    format!("events[{index}].scope"),
                    ProtocolValidationCode::ScopeMismatch,
                    "event scope must match suffix scope",
                ));
            }
            if event.cursor.sequence != expected_sequence {
                return Err(validation_error(
                    format!("events[{index}].cursor.sequence"),
                    ProtocolValidationCode::SequenceGap,
                    format!(
                        "expected sequence {expected_sequence}, received {}",
                        event.cursor.sequence
                    ),
                ));
            }
            if !event_ids.insert(&event.identity.event_id) {
                return Err(validation_error(
                    format!("events[{index}].identity.event_id"),
                    ProtocolValidationCode::DuplicateIdentity,
                    "event suffix contains a duplicate event identity",
                ));
            }
            expected_sequence = expected_sequence.checked_add(1).ok_or_else(|| {
                validation_error(
                    format!("events[{index}].cursor.sequence"),
                    ProtocolValidationCode::InvalidRange,
                    "event sequence cannot advance",
                )
            })?;
        }
        let expected_next = expected_sequence - 1;
        if self.next_cursor.sequence != expected_next {
            return Err(validation_error(
                "next_cursor.sequence",
                ProtocolValidationCode::SequenceGap,
                format!(
                    "expected next cursor {expected_next}, received {}",
                    self.next_cursor.sequence
                ),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(transparent)]
pub struct PageCursor(pub String);

impl PageCursor {
    pub fn validate(&self, field: &str) -> Result<(), ProtocolValidationError> {
        validate_required_text(field, &self.0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConversationSummary {
    pub conversation_id: String,
    pub title: String,
    pub updated_at: String,
    pub last_message_preview: Option<String>,
    pub revision: u64,
}

impl ConversationSummary {
    fn validate(&self, field: &str) -> Result<(), ProtocolValidationError> {
        validate_required_text(&format!("{field}.conversation_id"), &self.conversation_id)?;
        validate_required_text(&format!("{field}.title"), &self.title)?;
        validate_required_text(&format!("{field}.updated_at"), &self.updated_at)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConversationSummaryPageRequest {
    pub page_cursor: Option<PageCursor>,
    pub limit: u32,
}

impl ConversationSummaryPageRequest {
    pub fn validate(&self) -> Result<(), ProtocolValidationError> {
        validate_page_limit("limit", self.limit)?;
        if let Some(cursor) = &self.page_cursor {
            cursor.validate("page_cursor")?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConversationSummaryPage {
    pub items: Vec<ConversationSummary>,
    pub next_cursor: Option<PageCursor>,
    pub has_more: bool,
}

impl ConversationSummaryPage {
    pub fn validate(&self) -> Result<(), ProtocolValidationError> {
        if self.items.len() > MAX_PAGE_ITEMS as usize {
            return Err(validation_error(
                "items",
                ProtocolValidationCode::InvalidLimit,
                format!("page contains more than {MAX_PAGE_ITEMS} items"),
            ));
        }
        let mut identities = HashSet::new();
        for (index, item) in self.items.iter().enumerate() {
            item.validate(&format!("items[{index}]"))?;
            if !identities.insert(&item.conversation_id) {
                return Err(validation_error(
                    format!("items[{index}].conversation_id"),
                    ProtocolValidationCode::DuplicateIdentity,
                    "page contains a duplicate conversation identity",
                ));
            }
        }
        match (&self.next_cursor, self.has_more) {
            (Some(cursor), true) => cursor.validate("next_cursor"),
            (None, false) => Ok(()),
            _ => Err(validation_error(
                "next_cursor",
                ProtocolValidationCode::InvalidState,
                "next cursor must be present exactly when more items are available",
            )),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ConversationWindowAnchor {
    Latest,
    Message { message_id: String },
    Sequence { sequence: u64 },
}

impl ConversationWindowAnchor {
    fn validate(&self) -> Result<(), ProtocolValidationError> {
        match self {
            Self::Latest | Self::Sequence { .. } => Ok(()),
            Self::Message { message_id } => validate_required_text("anchor.message_id", message_id),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConversationWindowRequest {
    pub conversation_id: String,
    pub anchor: ConversationWindowAnchor,
    pub before: u32,
    pub after: u32,
}

impl ConversationWindowRequest {
    pub fn validate(&self) -> Result<(), ProtocolValidationError> {
        validate_required_text("conversation_id", &self.conversation_id)?;
        self.anchor.validate()?;
        let size = self.before.checked_add(self.after).ok_or_else(|| {
            validation_error(
                "before",
                ProtocolValidationCode::InvalidRange,
                "window size overflowed",
            )
        })?;
        if size == 0 || size > MAX_PAGE_ITEMS {
            return Err(validation_error(
                "before",
                ProtocolValidationCode::InvalidLimit,
                format!("combined window size must be between 1 and {MAX_PAGE_ITEMS}"),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ConversationWindowItem {
    pub message_id: String,
    pub ordinal: u64,
    pub revision: u64,
    pub payload: Value,
}

impl ConversationWindowItem {
    fn validate(&self, field: &str) -> Result<(), ProtocolValidationError> {
        validate_required_text(&format!("{field}.message_id"), &self.message_id)?;
        if self.revision == 0 {
            return Err(validation_error(
                format!("{field}.revision"),
                ProtocolValidationCode::InvalidRange,
                "revision must be greater than zero",
            ));
        }
        validate_object(&format!("{field}.payload"), &self.payload)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ConversationWindow {
    pub conversation_id: String,
    pub anchor: ConversationWindowAnchor,
    pub items: Vec<ConversationWindowItem>,
    pub previous_cursor: Option<PageCursor>,
    pub next_cursor: Option<PageCursor>,
    pub has_more_before: bool,
    pub has_more_after: bool,
    pub projection_cursor: ProjectionCursor,
}

impl ConversationWindow {
    pub fn validate(&self) -> Result<(), ProtocolValidationError> {
        validate_required_text("conversation_id", &self.conversation_id)?;
        self.anchor.validate()?;
        self.projection_cursor.validate()?;
        if self.projection_cursor.scope
            != (ProjectionScope::Conversation {
                conversation_id: self.conversation_id.clone(),
            })
        {
            return Err(validation_error(
                "projection_cursor.scope",
                ProtocolValidationCode::ScopeMismatch,
                "window projection cursor must use the conversation scope",
            ));
        }
        if self.items.len() > MAX_PAGE_ITEMS as usize {
            return Err(validation_error(
                "items",
                ProtocolValidationCode::InvalidLimit,
                format!("window contains more than {MAX_PAGE_ITEMS} items"),
            ));
        }
        let mut identities = HashSet::new();
        let mut previous_ordinal = None;
        for (index, item) in self.items.iter().enumerate() {
            item.validate(&format!("items[{index}]"))?;
            if !identities.insert(&item.message_id) {
                return Err(validation_error(
                    format!("items[{index}].message_id"),
                    ProtocolValidationCode::DuplicateIdentity,
                    "window contains a duplicate message identity",
                ));
            }
            if previous_ordinal.is_some_and(|ordinal| item.ordinal <= ordinal) {
                return Err(validation_error(
                    format!("items[{index}].ordinal"),
                    ProtocolValidationCode::InvalidRange,
                    "window items must be in strictly increasing ordinal order",
                ));
            }
            previous_ordinal = Some(item.ordinal);
        }
        for (field, cursor, has_more) in [
            (
                "previous_cursor",
                &self.previous_cursor,
                self.has_more_before,
            ),
            ("next_cursor", &self.next_cursor, self.has_more_after),
        ] {
            match (cursor, has_more) {
                (Some(cursor), true) => cursor.validate(field)?,
                (None, false) => {}
                _ => {
                    return Err(validation_error(
                        field,
                        ProtocolValidationCode::InvalidState,
                        "page cursor must be present exactly when more items are available",
                    ));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn current_protocol_command_fixture_is_stable_and_valid() {
        let fixture = include_str!("../tests/fixtures/current/command.json");
        let envelope: CommandEnvelope = serde_json::from_str(fixture).unwrap();
        envelope.validate().unwrap();
        assert_eq!(envelope.command.command_type, "rename_conversation");
        assert_eq!(
            envelope.command.arguments["conversation_id"],
            "conversation-1"
        );
    }

    #[test]
    fn runtime_effect_work_schedule_command_fixture_is_read_only_shaped() {
        let fixture =
            include_str!("../tests/fixtures/current/runtime_effect_work_schedule_command.json");
        let envelope: CommandEnvelope = serde_json::from_str(fixture).unwrap();
        envelope.validate().unwrap();
        assert_eq!(
            envelope.command.command_type,
            "load_runtime_effect_work_schedule"
        );
        assert_eq!(
            envelope.command.arguments,
            serde_json::json!({
                "conversation_ids": ["conversation-1"],
                "now_ms": 1735689600000_i64,
                "limit": 16
            })
        );
        assert!(envelope.expected_revision.is_none());
    }

    #[test]
    fn unknown_effect_recovery_fixture_is_strict_and_requires_explicit_choice() {
        let fixture = include_str!(
            "../tests/fixtures/current/resolve_unknown_effect_recovery_command.json"
        );
        let envelope: CommandEnvelope = serde_json::from_str(fixture).unwrap();
        envelope.validate().unwrap();
        assert_eq!(
            envelope.command.command_type,
            "resolve_unknown_effect_recovery"
        );
        let input: ResolveUnknownEffectRecovery =
            serde_json::from_value(envelope.command.arguments["input"].clone()).unwrap();
        input.validate().unwrap();
        for mutation in [
            serde_json::json!({"choice": {"kind":"authorize_risk_retry","reason":" "}}),
            serde_json::json!({"targets": []}),
            serde_json::json!({"resolution_id": ""}),
            serde_json::json!({"unexpected": true}),
        ] {
            let mut value = serde_json::to_value(&input).unwrap();
            value
                .as_object_mut()
                .unwrap()
                .extend(mutation.as_object().unwrap().clone());
            let decoded = serde_json::from_value::<ResolveUnknownEffectRecovery>(value);
            assert!(decoded.is_err() || decoded.unwrap().validate().is_err());
        }
    }

    #[test]
    fn runtime_effect_work_schedule_orders_expired_recovery_before_other_ready_work() {
        let schedule = RuntimeEffectWorkSchedule {
            observed_at_ms: 100,
            runs: vec![
                RuntimeEffectWorkRunHead {
                    conversation_id: "conversation-recovery".into(),
                    run_id: "run-recovery".into(),
                    effect_id: "effect-recovery".into(),
                    request_id: "request-recovery".into(),
                    attempt_id: Some("attempt-recovery".into()),
                    reason: RuntimeEffectWorkReason::RecoverExpiredLease,
                    ready_at_ms: 90,
                    created_at_ms: 20,
                },
                RuntimeEffectWorkRunHead {
                    conversation_id: "conversation-pending".into(),
                    run_id: "run-pending".into(),
                    effect_id: "effect-pending".into(),
                    request_id: "request-pending".into(),
                    attempt_id: None,
                    reason: RuntimeEffectWorkReason::Pending,
                    ready_at_ms: 10,
                    created_at_ms: 10,
                },
            ],
            next_wake_at_ms: None,
        };
        schedule.validate(2).unwrap();
    }

    #[test]
    fn runtime_effect_work_wait_requires_a_unique_bounded_conversation_scope() {
        for conversation_ids in [
            Vec::<String>::new(),
            vec![" ".into()],
            vec!["same".into(), "same".into()],
            (0..65)
                .map(|index| format!("conversation-{index}"))
                .collect(),
        ] {
            assert!(RuntimeEffectWorkWaitRequest {
                conversation_ids,
                limit: 1,
                wait_ms: 1,
            }
            .validate()
            .is_err());
        }
    }

    #[test]
    fn runtime_effect_work_response_cannot_escape_the_requested_scope() {
        let request = RuntimeEffectWorkWaitRequest {
            conversation_ids: vec!["owned".into()],
            limit: 1,
            wait_ms: 1,
        };
        let schedule = RuntimeEffectWorkSchedule {
            observed_at_ms: 100,
            runs: vec![RuntimeEffectWorkRunHead {
                conversation_id: "foreign".into(),
                run_id: "run".into(),
                effect_id: "effect".into(),
                request_id: "request".into(),
                attempt_id: None,
                reason: RuntimeEffectWorkReason::Pending,
                ready_at_ms: 100,
                created_at_ms: 100,
            }],
            next_wake_at_ms: None,
        };
        assert!(schedule.validate_for_request(&request).is_err());
    }

    #[test]
    fn current_protocol_receipt_fixture_excludes_ambiguous_result_states() {
        let fixture = include_str!("../tests/fixtures/current/receipt.json");
        let receipt: CommandReceipt = serde_json::from_str(fixture).unwrap();
        receipt.validate().unwrap();
        assert!(matches!(
            receipt.outcome,
            CommandReceiptOutcome::Committed {
                committed_revision: Some(43),
                ..
            }
        ));
    }

    #[test]
    fn current_protocol_projection_fixtures_form_a_contiguous_suffix() {
        let snapshot: ProjectionSnapshot = serde_json::from_str(include_str!(
            "../tests/fixtures/current/projection_snapshot.json"
        ))
        .unwrap();
        snapshot.validate().unwrap();
        let suffix: EventSuffix = serde_json::from_str(include_str!(
            "../tests/fixtures/current/event_suffix.json"
        ))
        .unwrap();
        suffix.validate().unwrap();
        assert_eq!(snapshot.cursor.sequence, suffix.after.sequence);
        assert_eq!(suffix.next_cursor.sequence, 44);
    }

    #[test]
    fn current_protocol_window_fixture_is_ordered_and_bounded() {
        let window: ConversationWindow = serde_json::from_str(include_str!(
            "../tests/fixtures/current/conversation_window.json"
        ))
        .unwrap();
        window.validate().unwrap();
        assert_eq!(window.items.len(), 2);
    }

    #[test]
    fn projection_suffix_rejects_sequence_gaps() {
        let mut suffix: EventSuffix = serde_json::from_str(include_str!(
            "../tests/fixtures/current/event_suffix.json"
        ))
        .unwrap();
        suffix.events[1].cursor.sequence = 45;
        let error = suffix.validate().unwrap_err();
        assert_eq!(error.code, ProtocolValidationCode::SequenceGap);
    }

    #[test]
    fn current_protocol_rejects_unknown_fields() {
        let value = json!({
            "protocol_version": APPLICATION_SERVICE_PROTOCOL_VERSION,
            "request_id": "request-1",
            "client_id": "mac-1",
            "expected_revision": 42,
            "unexpected": true,
            "command": {"type": "rename_conversation", "arguments": {}}
        });
        assert!(serde_json::from_value::<CommandEnvelope>(value).is_err());
    }

    #[test]
    fn projection_suffix_rejects_old_version_at_every_event_position() {
        let valid: EventSuffix = serde_json::from_str(include_str!(
            "../tests/fixtures/current/event_suffix.json"
        ))
        .unwrap();
        valid.validate().unwrap();
        for index in 0..valid.events.len() {
            let mut suffix: EventSuffix = serde_json::from_str(include_str!(
                "../tests/fixtures/current/event_suffix.json"
            ))
            .unwrap();
            suffix.events[index].protocol_version = APPLICATION_SERVICE_PROTOCOL_VERSION - 1;
            assert_eq!(
                suffix.validate().unwrap_err().code,
                ProtocolValidationCode::InvalidProtocolVersion
            );
        }
    }

    #[test]
    fn canonical_replica_contract_matches_worker_camel_case_shape() {
        let request = CanonicalReplicaPushRequest {
            changes: vec![CanonicalReplicaChange {
                operation_id: "op-1".to_string(),
                entity_type: "product_setting".to_string(),
                entity_id: "assistant.memory.enabled".to_string(),
                revision: 2,
                deleted: false,
                payload: json!({"enabled": true}),
                origin_device_id: "mac-a".to_string(),
            }],
        };
        let encoded = serde_json::to_value(&request).unwrap();
        assert_eq!(encoded["changes"][0]["operationId"], "op-1");
        assert_eq!(encoded["changes"][0]["originDeviceId"], "mac-a");
        assert_eq!(
            serde_json::from_value::<CanonicalReplicaPushRequest>(encoded).unwrap(),
            request
        );
    }
}

pub use nola_domain::{CanonicalBranchIntent, CanonicalBranchResult, CanonicalBranchTarget};

#[cfg(test)]
mod delegated_projection_identity_tests {
    use super::*;

    #[test]
    fn delegated_owner_is_distinct_and_requires_run_scope() {
        let mut identity = ProjectionEventIdentity {
            event_id: "event".into(),
            conversation_id: Some("conversation".into()),
            run_id: Some("run".into()),
            attempt_id: None,
            delegated_invocation_id: Some("external".into()),
            message_id: Some("message".into()),
            part_id: Some("part".into()),
            call_id: None,
        };
        identity.validate().unwrap();
        identity.attempt_id = Some("external".into());
        assert!(identity.validate().is_err());
        identity.attempt_id = None;
        identity.run_id = None;
        assert!(identity.validate().is_err());
        identity.run_id = Some("run".into());
        identity.delegated_invocation_id = Some(" ".into());
        assert!(identity.validate().is_err());
    }
}
