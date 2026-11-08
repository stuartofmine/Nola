// SPDX-License-Identifier: Apache-2.0
//! Compaction lifecycle facts, independent of transcript representation.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompactionRetryState {
    pub source_fingerprint: Option<String>,
    pub failed_attempts: u8,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CompactionLifecycleState {
    pub generation: u64,
    pub in_progress: bool,
    pub consecutive_failures: u8,
    pub automatic_blocked: bool,
    pub retry: CompactionRetryState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BeginCompactionError {
    AlreadyActive,
    ArchiveRetryLimit,
}
impl std::fmt::Display for BeginCompactionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::AlreadyActive => "context compaction is already active",
            Self::ArchiveRetryLimit => "the same context archive failed to compact three times",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinishedCompactionTransition {
    pub state: CompactionLifecycleState,
    pub made_progress: bool,
}
