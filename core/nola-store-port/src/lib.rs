// SPDX-License-Identifier: Apache-2.0
//! Persistence contracts without a database or Application implementation dependency.
use serde::{Deserialize, Serialize};
pub mod activity;
pub mod automation_facts;
pub mod room_presentation;
pub mod assistant_journal;
pub mod run_message_admission;
pub mod assistant_ownership;
pub mod cancellation_delivery;
pub mod canonical_body;
pub mod canonical_branch;
pub mod edited_resend;
pub mod content_upload;
pub mod context;
pub mod conversation_deletion;
pub mod conversation_profile;
pub mod conversation_resource;
pub mod conversation_revert;
pub mod effect_delivery;
pub mod host_replay;
pub mod mcp;
pub mod plugin;
pub mod memory;
pub mod model;
pub mod run_creation;
pub mod run_effect;
pub mod run_decision;
pub mod run_recovery_inspection;
pub mod skill;
pub mod sop_run;
pub mod transcript_export;
pub mod tool_source;
pub mod transcript_preparation;
pub mod transcript_publication;
pub mod transcript_write;

/// Metadata for a canonical text Part whose window preview is incomplete.
/// Byte lengths describe the original UTF-8 body, never encoded JSON or a summary.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(try_from = "DeferredTextPartWire")]
pub struct DeferredTextPart {
    pub message_id: String,
    pub part_id: String,
    pub total_utf8_bytes: u64,
    pub preview_utf8_bytes: u64,
    /// Exact Part lifecycle only; not read authorization or a snapshot version.
    pub is_final: bool,
    /// Store's current range eligibility for this exact Part. Every read still
    /// checks ownership, visibility and backing; this is not a version handle.
    pub can_read_original: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DeferredTextPartWire {
    message_id: String,
    part_id: String,
    total_utf8_bytes: u64,
    preview_utf8_bytes: u64,
    is_final: bool,
    can_read_original: bool,
}

impl TryFrom<DeferredTextPartWire> for DeferredTextPart {
    type Error = &'static str;

    fn try_from(value: DeferredTextPartWire) -> Result<Self, Self::Error> {
        Self::new(
            value.message_id,
            value.part_id,
            value.total_utf8_bytes,
            value.preview_utf8_bytes,
            value.is_final,
            value.can_read_original,
        )
    }
}

impl DeferredTextPart {
    pub fn new(
        message_id: String,
        part_id: String,
        total_utf8_bytes: u64,
        preview_utf8_bytes: u64,
        is_final: bool,
        can_read_original: bool,
    ) -> Result<Self, &'static str> {
        if message_id.trim().is_empty() || part_id.trim().is_empty() {
            return Err("deferred text requires canonical message and Part identities");
        }
        if total_utf8_bytes == 0 || preview_utf8_bytes >= total_utf8_bytes {
            return Err("deferred text preview must be shorter than its original body");
        }
        if can_read_original && !is_final {
            return Err("an unfinished text Part cannot expose unversioned original ranges");
        }
        Ok(Self {
            message_id,
            part_id,
            total_utf8_bytes,
            preview_utf8_bytes,
            is_final,
            can_read_original,
        })
    }
}

#[cfg(test)]
mod deferred_text_tests {
    use super::DeferredTextPart;
    use serde_json::json;

    #[test]
    fn large_original_metadata_is_independent_of_preview_size_and_storage() {
        for preview in [0, 4, 32768] {
            let value =
                DeferredTextPart::new("m".into(), "p".into(), 10_000_000, preview, true, true)
                    .unwrap();
            assert_eq!(value.total_utf8_bytes, 10_000_000);
            assert_eq!(value.preview_utf8_bytes, preview);
        }
    }

    #[test]
    fn complete_empty_or_unowned_parts_are_not_deferred() {
        for (message, part, total, preview) in [
            ("m", "p", 0, 0),
            ("m", "p", 4, 4),
            ("m", "p", 4, 5),
            (" ", "p", 4, 0),
            ("m", "", 4, 0),
        ] {
            assert!(
                DeferredTextPart::new(message.into(), part.into(), total, preview, true, true)
                    .is_err()
            );
        }
    }

    #[test]
    fn wire_decode_enforces_the_same_identity_and_length_contract() {
        let valid = json!({"message_id":"m", "part_id":"p",
            "total_utf8_bytes":10_000_000, "preview_utf8_bytes":32768, "is_final":true,
            "can_read_original":true});
        let decoded: DeferredTextPart = serde_json::from_value(valid.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), valid);
        let mut streaming = valid.clone();
        streaming["is_final"] = json!(false);
        streaming["can_read_original"] = json!(false);
        let decoded: DeferredTextPart = serde_json::from_value(streaming.clone()).unwrap();
        assert!(!decoded.is_final);
        assert_eq!(serde_json::to_value(decoded).unwrap(), streaming);
        for (key, value) in [
            ("message_id", json!(" ")),
            ("part_id", json!("")),
            ("total_utf8_bytes", json!(0)),
            ("total_utf8_bytes", json!(32768)),
            ("preview_utf8_bytes", json!(10_000_001)),
            ("preview_utf8_bytes", json!(-1)),
            ("preview_utf8_bytes", json!("32768")),
            ("is_final", json!(null)),
            ("is_final", json!(1)),
            ("is_final", json!("true")),
            ("is_final", json!(false)),
            ("can_read_original", json!(null)),
            ("can_read_original", json!(1)),
            ("can_read_original", json!("true")),
            ("path", json!("/tmp/body")),
        ] {
            let mut invalid = valid.clone();
            invalid[key] = value;
            assert!(
                serde_json::from_value::<DeferredTextPart>(invalid).is_err(),
                "{key}"
            );
        }
        for missing in [
            "message_id",
            "part_id",
            "total_utf8_bytes",
            "preview_utf8_bytes",
            "is_final",
            "can_read_original",
        ] {
            let mut invalid = valid.clone();
            invalid.as_object_mut().unwrap().remove(missing);
            assert!(
                serde_json::from_value::<DeferredTextPart>(invalid).is_err(),
                "{missing}"
            );
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredTranscriptRange {
    pub conversation_id: String,
    pub message_id: String,
    pub part_id: String,
    pub offset_bytes: u64,
    pub next_offset_bytes: u64,
    pub total_utf8_bytes: u64,
    pub text: String,
    pub end_of_content: bool,
}

#[derive(Debug)]
pub struct StoredTranscriptMatch {
    pub message_id: String,
    pub part_id: String,
    pub role: String,
    pub message_ordinal: i64,
    pub snippet: String,
    pub start_character: i64,
}

/// Reads canonical original text, not summaries or presentation caches.
/// Implementations must enforce conversation ownership and visible text scope,
/// return UTF-8-aligned ranges of at most `limit_bytes` (4..=32768), and
/// bound search to `limit` (1..=20) excerpts of at most 1600 characters each.
/// Search uses literal queries, with results in canonical message/Part order.
/// This port grants no write or transaction access.
pub trait TranscriptReadRepository {
    type Error: std::fmt::Display;

    fn query_canonical_history(
        &self,
        input: &nola_domain::canonical_history::CanonicalHistoryQuery,
    ) -> Result<nola_domain::canonical_history::CanonicalHistoryResponse, Self::Error>;

    fn read_conversation_text_range(
        &self,
        conversation_id: &str,
        message_id: &str,
        part_id: &str,
        offset_bytes: u64,
        limit_bytes: usize,
    ) -> Result<StoredTranscriptRange, Self::Error>;

    fn search_transcript_text(
        &self,
        conversation_id: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<StoredTranscriptMatch>, Self::Error>;
}

pub mod data_source;

pub mod document_source;

pub mod archive_restore;

pub mod replica_sync;

pub mod protocol_journal;

pub mod part_derivation;

pub mod historical_part_projection;

pub mod conversation_history;

pub mod permission;

pub mod automation;

pub mod runtime_task;

pub mod workspace_ownership;

pub mod conversation_plan;
pub mod work_step_transaction;

pub mod agent_queue;

pub mod trace_journal;

pub mod product_setting;

pub mod remote_session;

pub mod consistent_read;
pub mod resume_snapshot;

pub mod conversation_query;

pub mod store_maintenance;
pub mod user_input_wait;
pub mod execution_evidence;
pub mod code_cell;
pub mod auxiliary_invocation;
pub mod prepared_host_tool_plan;

pub mod generative_ui;

pub mod goal_execution;

pub mod expert_message;
