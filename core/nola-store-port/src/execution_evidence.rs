// SPDX-License-Identifier: Apache-2.0
//! Exact committed observations. Reading an observation grants no acceptance.
use nola_domain::execution_evidence::ExecutionEvidenceObservation;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionEvidenceQuery {
    pub conversation_id: String,
    pub run_id: String,
    pub evidence_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredExecutionEvidence {
    pub conversation_id: String,
    pub run_id: String,
    pub core_session_id: String,
    pub turn_id: String,
    pub core_revision: u64,
    pub core_event_cursor: u64,
    pub source_event_id: String,
    pub host_event_id: String,
    pub observation: ExecutionEvidenceObservation,
}

pub trait ExecutionEvidenceRepository {
    type Error: std::fmt::Display;
    /// None is missing evidence. Found evidence with mismatched ownership or
    /// corrupted source facts is an error, not an empty/successful observation.
    fn load_execution_evidence(
        &self,
        query: &ExecutionEvidenceQuery,
    ) -> Result<Option<StoredExecutionEvidence>, Self::Error>;
}
