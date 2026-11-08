// SPDX-License-Identifier: Apache-2.0
//! History import commands shared by every client transport.
use crate::TranscriptMessageInput;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConversationTranscriptImportSource {
    BackupRestore,
    RemoteSync,
    ExternalArchive,
}

/// Imports an already-finalized transcript from an explicit external source.
///
/// This is intentionally not a generic message append command. Assistant
/// messages remain owned by a Run and enter through `BeginAssistantTurn` plus
/// Host responses. Import is only for immutable, completed history whose
/// provenance is known; interactive user-only admission has its own command.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ImportConversationTranscript {
    pub import_id: String,
    pub source: ConversationTranscriptImportSource,
    pub conversation_id: String,
    pub expected_message_count: usize,
    pub messages: Vec<TranscriptMessageInput>,
    pub imported_at_ms: i64,
}

/// Atomically append the user's canonical message without starting an
/// assistant Run. `message_id` is a stable client idempotency key; retries
/// with the same identity and exact text return the same admission receipt.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AdmitConversationUserMessage {
    pub conversation_id: String,
    pub message_id: String,
    pub text: String,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum TranscriptCommand {
    AdmitConversationUserMessage {
        input: AdmitConversationUserMessage,
    },
    ImportConversationTranscript {
        input: ImportConversationTranscript,
    },
    PublishUploadedTranscript {
        input: crate::uploaded_transcript::UploadedTranscriptManifest,
    },
}

impl crate::CommandMetadata for TranscriptCommand {
    fn wire_command_name(&self) -> &'static str {
        match self {
            Self::AdmitConversationUserMessage { .. } => "admit_conversation_user_message",
            Self::ImportConversationTranscript { .. } => "import_conversation_transcript",
            Self::PublishUploadedTranscript { .. } => "publish_uploaded_transcript",
        }
    }
}
