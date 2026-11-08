// SPDX-License-Identifier: Apache-2.0
//! Durable identity for a single physical model sampling attempt.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelAttemptState {
    Prepared,
    Running,
    Succeeded,
    Failed,
    Cancelled,
    Superseded,
    Interrupted,
}

/// Bounded, user-presentable failure recorded for one physical sampling attempt.
///
/// Provider response bodies and diagnostic identifiers stay in the trace
/// journal. Conversation projections consume this typed summary and never
/// reconstruct product state from transport logs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelAttemptFailure {
    pub code: String,
    pub message: String,
    pub retryable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http_status: Option<u16>,
}

impl ModelAttemptState {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed | Self::Cancelled | Self::Superseded
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelAttemptRecord {
    pub id: String,
    pub run_id: String,
    pub invocation_id: String,
    pub effect_id: String,
    pub text_part_id: String,
    pub reasoning_part_id: String,
    pub predecessor_id: Option<String>,
    pub state: ModelAttemptState,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub failure: Option<ModelAttemptFailure>,
    pub usage: serde_json::Value,
    pub created_at_ms: i64,
    pub started_at_ms: Option<i64>,
    pub completed_at_ms: Option<i64>,
    pub revision: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelAttemptMutation {
    /// None inserts a new identity; Some performs a revision compare-and-swap.
    pub expected_revision: Option<i64>,
    pub record: ModelAttemptRecord,
}
