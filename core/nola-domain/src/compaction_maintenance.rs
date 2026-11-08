// SPDX-License-Identifier: Apache-2.0
//! Facts for one maintenance response; provider transport remains a Host concern.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompactionMaintenanceFacts {
    pub summary: String,
    pub target_tokens: usize,
    pub output_limited: bool,
    pub has_tool_calls: bool,
    pub completed: bool,
    pub cancelled: bool,
    pub continuation_count: usize,
    pub rewrite_count: usize,
    pub can_continue: bool,
    pub can_rewrite: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum CompactionMaintenanceDecision {
    Accept,
    Continue { instructions: String },
    Rewrite { instructions: String },
    Reject { code: String },
}
