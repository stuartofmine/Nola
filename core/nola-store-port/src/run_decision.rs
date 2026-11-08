// SPDX-License-Identifier: Apache-2.0
//! Exact committed Core decision evidence; not a sealed business result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunDecisionQuery {
    pub conversation_id: String,
    pub run_id: String,
    pub turn_id: String,
    pub core_revision: u64,
    pub core_event_cursor: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredRunDecision {
    pub source_transition_id: String,
    /// Store checkpoint revision, deliberately distinct from Core revision.
    pub checkpoint_revision: u64,
    pub core_session_id: String,
    pub event_json: String,
}

pub trait RunDecisionRepository {
    type Error: std::fmt::Display;
    /// None means the exact event does not exist or is not RunResultDecided.
    /// A found event with missing/mismatched ownership/frontier is an error.
    /// Does not infer outcome, read the latest checkpoint, or grant evidence credit.
    fn load_run_decision(&self, query: &RunDecisionQuery)
        -> Result<Option<StoredRunDecision>, Self::Error>;
}
