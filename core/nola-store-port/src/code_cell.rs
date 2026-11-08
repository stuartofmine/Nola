// SPDX-License-Identifier: Apache-2.0
//! Read-only cell observations, committed with the owning Run transaction.
//! These records never carry source, nested arguments, or execution authority.
use nola_domain::code_mode::{CodeCellIdentity, CodeCellView};

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodeCellRecord {
    pub store_epoch: String,
    pub conversation_id: String,
    pub run_id: String,
    pub parent_part_id: String,
    pub core_revision: u64,
    pub cell: CodeCellIdentity,
    /// Complete committed observation, not a model-visible page. Readers
    /// select bounded pages without loading unrelated cells or JS state.
    pub observation: CodeCellView,
}

impl CodeCellRecord {
    pub fn validate(&self) -> Result<(), String> {
        self.cell.validate()?;
        if [&self.store_epoch, &self.conversation_id, &self.run_id, &self.parent_part_id]
            .iter().any(|id| id.trim().is_empty()) || self.core_revision == 0 {
            return Err("invalid code cell record owner".into());
        }
        let view = &self.observation;
        if view.cell_id != self.cell.cell_id || view.parent_call_id != self.cell.parent_call_id
            || view.has_more_output || view.next_output_sequence != view.output.len() as u64
            || view.output.iter().enumerate().any(|(index, chunk)| chunk.sequence != index as u64 + 1) {
            return Err("incomplete code cell observation record".into());
        }
        view.validate_state()?;
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodeCellQuery {
    pub store_epoch: String,
    pub conversation_id: String,
    pub cell_id: String,
    pub after_output_sequence: u64,
}

pub trait CodeCellObservationRepository {
    type Error: std::fmt::Display;
    /// Caller has already authenticated the conversation. This is observation
    /// only: a historical ID cannot authorize termination or source replay.
    fn read_code_cell(&self, query: &CodeCellQuery) -> Result<Option<CodeCellView>, Self::Error>;
    /// Must participate in the current assistant publication transaction.
    /// A stale revision or conflicting owner is rejected, never overwritten.
    fn publish_code_cell(&mut self, record: &CodeCellRecord) -> Result<(), Self::Error>;
}
