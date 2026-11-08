// SPDX-License-Identifier: Apache-2.0
//! Explicit facts and decisions for requesting context compaction.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompactionReason {
    Manual,
    ThresholdReached,
    ContextLimitRecovery,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompactionRequestFacts {
    pub reason: CompactionReason,
    pub compaction_in_progress: bool,
    pub context_is_empty: bool,
    pub automatic_compaction_count: Option<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactionRequestRejection {
    AlreadyActive,
    EmptyContext,
    AutomaticLimitReached,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmptyCompactionArchiveAction {
    ResumeModel,
    CompleteWithoutSummary,
    Reject,
}
