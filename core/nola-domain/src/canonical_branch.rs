// SPDX-License-Identifier: Apache-2.0
//! Branch intent carries identity and metadata; canonical history stays in Store.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanonicalBranchTarget {
    pub id: String,
    pub title: String,
    pub settings_json: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanonicalBranchIntent {
    pub operation_id: String,
    pub source_conversation_id: String,
    pub expected_source_revision: i64,
    pub through_message_id: String,
    pub conversation: CanonicalBranchTarget,
    pub committed_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CanonicalBranchResult {
    pub created: bool,
    pub message_count: usize,
    pub conversation_revision: i64,
    pub conversation_id: String,
    pub source_conversation_id: String,
    pub through_message_id: String,
}
