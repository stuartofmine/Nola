// SPDX-License-Identifier: Apache-2.0
//! Original archive metadata and byte ranges, independent of presentation previews.
use serde::{Deserialize, Serialize};

pub use crate::canonical_text::PartBodyField as ExportBodyField;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ExportConversation {
    pub id: String,
    pub title: String,
    pub revision: i64,
    pub configuration_revision: i64,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub settings_json: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ExportPart {
    pub has_derivation: bool,
    pub id: String,
    pub kind: String,
    pub ordinal: i64,
    pub state: String,
    pub content_digest: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub content_text_utf8_bytes: u64,
    /// None preserves SQL NULL; Some(0) is an existing empty JSON string.
    pub content_json_utf8_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ExportMessage {
    pub id: String,
    pub run_id: Option<String>,
    pub role: String,
    pub status: String,
    pub ordinal: i64,
    pub created_at_ms: i64,
    pub completed_at_ms: Option<i64>,
    pub parts: Vec<ExportPart>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ExportPage {
    pub conversation: ExportConversation,
    /// At most one original Message, without either body field.
    pub messages: Vec<ExportMessage>,
    pub next_ordinal: Option<i64>,
    pub has_more: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ExportRange {
    pub conversation_id: String,
    pub message_id: String,
    pub part_id: String,
    pub field: ExportBodyField,
    pub revision: i64,
    pub offset_bytes: u64,
    pub next_offset_bytes: u64,
    pub total_utf8_bytes: u64,
    pub text: String,
    pub end_of_content: bool,
}

pub use crate::{
    CanonicalPartOwner, PartDerivation, PartDerivationProvenance, StoredPartDerivation,
};
