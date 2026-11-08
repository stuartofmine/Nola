// SPDX-License-Identifier: Apache-2.0
//! Bounded, non-executable interpretation of canonical historical JSON.
use crate::{
    CanonicalPartOwner, SemanticIdentityAssessment, SemanticSourceKey, SemanticValue,
    StructuredValue,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoricalPartSchema {
    AgentPart,
    MessagePartView,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum HistoricalProjectionStatus {
    NotLoaded { reason: HistoricalNotLoadedReason },
    Supported,
    Deferred,
    Unsupported { code: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoricalPartProjection {
    pub owner: CanonicalPartOwner,
    pub canonical_revision: i64,
    pub source: Option<SemanticSourceKey>,
    pub schema: Option<HistoricalPartSchema>,
    pub canonical_kind: String,
    pub binding: Option<SemanticIdentityAssessment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canonical_text: Option<HistoricalTextPreview>,
    pub status: HistoricalProjectionStatus,
    /// Only the recognized schema's fixed field catalog is present. Missing,
    /// complete null and deferred original tokens remain distinct.
    pub fields: BTreeMap<String, SemanticValue<StructuredValue>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoricalNotLoadedReason {
    SelectionBudget,
}

/// Typed internal budget outcome; source errors remain errors.
#[derive(Debug, Clone, PartialEq)]
pub enum HistoricalPartRead {
    Absent,
    Available(Box<HistoricalPartProjection>),
    BudgetDeferred { canonical_kind: String },
}

/// Descriptive canonical text, independent of JSON interpretation or authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoricalTextPreview {
    pub text: String,
    pub total_utf8_bytes: u64,
}
