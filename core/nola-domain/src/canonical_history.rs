// SPDX-License-Identifier: Apache-2.0
//! Local-user canonical history authority and bounded original-text evidence.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CanonicalHistoryScope {
    CurrentConversation,
    LocalUserHistory,
    Conversations { conversation_ids: Vec<String> },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CanonicalHistoryQuery {
    Search {
        requester_conversation_id: String,
        scope: CanonicalHistoryScope,
        query: String,
        limit: usize,
        after_conversation_id: Option<String>,
    },
    Read {
        requester_conversation_id: String,
        conversation_id: String,
        message_id: String,
        part_id: String,
        expected_conversation_revision: Option<i64>,
        offset_bytes: u64,
        limit_bytes: usize,
    },
}

impl CanonicalHistoryQuery {
    pub fn requester(&self) -> &str {
        match self { Self::Search { requester_conversation_id, .. } | Self::Read { requester_conversation_id, .. } => requester_conversation_id }
    }
    pub fn validate(&self) -> Result<(), String> {
        fn identity(value: &str) -> Result<(), String> {
            if value.is_empty() || value.trim() != value || value.len() > 128 || value.chars().any(char::is_control) {
                return Err("invalid canonical history identity".into());
            }
            Ok(())
        }
        identity(self.requester())?;
        match self {
            Self::Search { scope, query, limit, after_conversation_id, .. } => {
                if query.trim().is_empty() || query.chars().count() > 500 || !(1..=20).contains(limit) {
                    return Err("canonical history search requires a bounded query and 1...20 matches".into());
                }
                if let Some(cursor) = after_conversation_id { identity(cursor)?; }
                if let CanonicalHistoryScope::Conversations { conversation_ids } = scope {
                    if !(1..=32).contains(&conversation_ids.len()) { return Err("canonical history scope requires 1...32 conversations".into()); }
                    let mut unique = std::collections::BTreeSet::new();
                    for id in conversation_ids { identity(id)?; if !unique.insert(id) { return Err("duplicate history scope identity".into()); } }
                }
            }
            Self::Read { conversation_id, message_id, part_id, expected_conversation_revision, offset_bytes, limit_bytes, .. } => {
                for id in [conversation_id, message_id, part_id] { identity(id)?; }
                if *offset_bytes >= i64::MAX as u64 || !(4..=32_768).contains(limit_bytes) {
                    return Err("invalid canonical history byte range".into());
                }
                if expected_conversation_revision.is_some_and(|revision| revision < 0) { return Err("invalid canonical history source revision".into()); }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CanonicalHistorySource {
    pub conversation_id: String,
    pub conversation_title: String,
    pub conversation_revision: i64,
    pub message_id: String,
    pub part_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CanonicalHistoryHit {
    pub source: CanonicalHistorySource,
    pub role: String,
    pub message_ordinal: i64,
    pub snippet: String,
    pub start_character: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CanonicalHistoryResponse {
    Search {
        requester_conversation_id: String,
        hits: Vec<CanonicalHistoryHit>,
        searched_conversation_ids: Vec<String>,
        /// Continue after this conversation; remaining history was not searched.
        next_conversation_cursor: Option<String>,
        /// Search selects bounded matches; refine the query for additional matches.
        results_limited: bool,
    },
    Read {
        requester_conversation_id: String,
        source: CanonicalHistorySource,
        offset_bytes: u64,
        next_offset_bytes: u64,
        total_utf8_bytes: u64,
        text: String,
        end_of_content: bool,
    },
}

impl CanonicalHistoryResponse {
    pub fn validate_for(&self, input: &CanonicalHistoryQuery) -> Result<(), String> {
        input.validate()?;
        match (self, input) {
            (Self::Search { requester_conversation_id, hits, searched_conversation_ids, next_conversation_cursor, .. },
             CanonicalHistoryQuery::Search { scope, limit, .. }) => {
                if requester_conversation_id != input.requester() || hits.len() > *limit || searched_conversation_ids.len() > 16
                    || next_conversation_cursor.as_ref().is_some_and(|cursor| searched_conversation_ids.last() != Some(cursor)) {
                    return Err("canonical history search response differs from requested owner or bounds".into());
                }
                for hit in hits {
                    let allowed = match scope {
                        CanonicalHistoryScope::CurrentConversation => hit.source.conversation_id == input.requester(),
                        CanonicalHistoryScope::LocalUserHistory => true,
                        CanonicalHistoryScope::Conversations { conversation_ids } => conversation_ids.contains(&hit.source.conversation_id),
                    };
                    if !allowed || !searched_conversation_ids.contains(&hit.source.conversation_id)
                        || hit.snippet.chars().count() > 1_600 || hit.start_character < 0 {
                        return Err("canonical history search source exceeds its selected scope or excerpt bounds".into());
                    }
                }
            }
            (Self::Read { requester_conversation_id, source, offset_bytes, next_offset_bytes, total_utf8_bytes, text, end_of_content },
             CanonicalHistoryQuery::Read { conversation_id, message_id, part_id, expected_conversation_revision, offset_bytes: requested_offset, limit_bytes, .. }) => {
                if requester_conversation_id != input.requester() || &source.conversation_id != conversation_id
                    || &source.message_id != message_id || &source.part_id != part_id || offset_bytes != requested_offset
                    || expected_conversation_revision.is_some_and(|revision| source.conversation_revision != revision)
                    || text.len() > *limit_bytes || next_offset_bytes < offset_bytes
                    || next_offset_bytes - offset_bytes != text.len() as u64 || next_offset_bytes > total_utf8_bytes
                    || *end_of_content != (next_offset_bytes == total_utf8_bytes) || (text.is_empty() && !*end_of_content) {
                    return Err("canonical history original-text response differs from exact source or byte range".into());
                }
            }
            _ => return Err("canonical history response kind differs from query".into()),
        }
        Ok(())
    }
}
