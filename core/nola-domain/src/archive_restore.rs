// SPDX-License-Identifier: Apache-2.0
//! Staged archive replacement. Original message/Part identities are preserved.
use crate::{CanonicalBodyReference, MessagePartInput, PartDerivation};
use crate::tool_source::ToolSourceClaim;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BeginArchiveRestore {
    pub restore_id: String,
    pub expected_directory_revision: u64,
    pub created_at_ms: i64,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ArchiveConversationProfile {
    pub id: String,
    pub title: String,
    pub settings_json: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ArchiveMessage {
    pub id: String,
    pub run_id: Option<String>,
    pub role: String,
    pub status: String,
    pub ordinal: i64,
    pub created_at_ms: i64,
    pub completed_at_ms: Option<i64>,
    pub parts: Vec<MessagePartInput>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ArchivePartUpload {
    pub upload_id: String,
    pub message_id: String,
    pub part_id: String,
    pub field: crate::PartBodyField,
    pub expected_utf8_bytes: u64,
}
/// Client intent uses sealed upload bindings, never supplied canonical references.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ArchiveRestoreManifestEntry {
    ToolSource {
        claim: ToolSourceClaim,
        upload_id: String,
    },
    Profile {
        profile: ArchiveConversationProfile,
    },
    PartDerivation {
        record: PartDerivation,
    },
    Message {
        conversation_id: String,
        message: ArchiveMessage,
        uploads: Vec<ArchivePartUpload>,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StageArchiveManifest {
    pub restore_id: String,
    pub sequence: u64,
    pub entry: ArchiveRestoreManifestEntry,
}
/// Internal Store/replica fact; Application resolves immutable owner-bound refs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ArchiveRestoreEntry {
    ToolSource {
        claim: ToolSourceClaim,
    },
    Profile {
        profile: ArchiveConversationProfile,
    },
    PartDerivation {
        record: PartDerivation,
    },
    Message {
        conversation_id: String,
        message: ArchiveMessage,
        references: Vec<CanonicalBodyReference>,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ArchiveRestoreProgress {
    pub restore_id: String,
    pub next_sequence: u64,
    pub manifest_digest: String,
    pub completed: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CommitArchiveRestore {
    pub restore_id: String,
    pub expected_directory_revision: u64,
    pub expected_entry_count: u64,
    pub manifest_digest: String,
    pub committed_at_ms: i64,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ArchiveRestoreResult {
    pub restore_id: String,
    pub conversation_count: u64,
    pub message_count: u64,
    pub directory_revision: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ArchiveRestoreTarget {
    pub conversation_id: String,
    pub base_revision: Option<i64>,
    pub base_content_revision: Option<i64>,
    pub resulting_revision: i64,
    pub deleted: bool,
}
