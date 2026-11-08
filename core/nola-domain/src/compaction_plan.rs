// SPDX-License-Identifier: Apache-2.0
//! Body-free facts and indices for deterministic history compaction.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CompactionItemKind {
    PersistentInstruction,
    UserMessage,
    ToolCall { call_id: String },
    ToolResult { call_id: String },
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompactionItemFact {
    pub estimated_tokens: usize,
    pub kind: CompactionItemKind,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompactionIndexPlan {
    pub summarize_indices: Vec<usize>,
    pub kept_indices: Vec<usize>,
}
