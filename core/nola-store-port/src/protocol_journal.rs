// SPDX-License-Identifier: Apache-2.0
//! Durable receipt and projection journal contracts shared by Application adapters.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BeginCommandReceiptInput {
    pub client_id: String,
    pub request_id: String,
    pub command_type: String,
    pub command_json: String,
    pub expected_revision: Option<i64>,
    pub received_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredCommandReceipt {
    pub client_id: String,
    pub request_id: String,
    pub command_type: String,
    pub command_json: String,
    pub expected_revision: Option<i64>,
    pub status: String,
    pub committed_revision: Option<i64>,
    pub result_json: Option<String>,
    pub error_json: Option<String>,
    pub conflict_json: Option<String>,
    pub received_at: String,
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum BeginCommandReceiptResult {
    Started(StoredCommandReceipt),
    Existing(StoredCommandReceipt),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum CompleteCommandReceiptOutcome {
    Committed {
        committed_revision: Option<i64>,
        result_json: String,
    },
    Rejected {
        error_json: String,
    },
    Conflict {
        conflict_json: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectionScopeKey {
    pub scope_type: String,
    pub scope_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppendProjectionEventInput {
    pub event_id: String,
    pub scope: ProjectionScopeKey,
    pub expected_after_sequence: u64,
    pub payload_json: String,
    pub occurred_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredProjectionEvent {
    pub event_id: String,
    pub scope: ProjectionScopeKey,
    pub sequence: u64,
    pub payload_json: String,
    pub occurred_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredProjectionCursor {
    pub scope: ProjectionScopeKey,
    pub current_sequence: u64,
    pub revision: u64,
    pub updated_at: String,
}

/// Mutations join the caller's command unit of work; no repository opens an
/// independent connection. Receipt identity includes the exact admitted command.
pub trait CommandReceiptRepository {
    type Error: std::fmt::Display;
    fn begin_command_receipt(
        &mut self,
        input: &BeginCommandReceiptInput,
    ) -> Result<BeginCommandReceiptResult, ReceiptAdmissionError<Self::Error>>;
    fn command_receipt(
        &self,
        client_id: &str,
        request_id: &str,
    ) -> Result<Option<StoredCommandReceipt>, Self::Error>;
    fn complete_command_receipt(
        &mut self,
        client_id: &str,
        request_id: &str,
        completed_at: &str,
        outcome: &CompleteCommandReceiptOutcome,
    ) -> Result<StoredCommandReceipt, Self::Error>;
}

/// A reused request identity is a terminal protocol conflict, not an I/O retry.
#[derive(Debug)]
pub enum ReceiptAdmissionError<E> {
    IdentityConflict(String),
    Persistence(E),
}

impl<E: std::fmt::Display> std::fmt::Display for ReceiptAdmissionError<E> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::IdentityConflict(message) => formatter.write_str(message),
            Self::Persistence(error) => error.fmt(formatter),
        }
    }
}

pub enum CommandRevisionEntity<'a> {
    ModelProfile(&'a str),
    Conversation(&'a str),
}

/// Owns the transaction shared by every repository participating in a command.
/// A mutation savepoint must not commit the enclosing receipt transaction.
pub trait CommandTransactionRepository {
    type Error: std::fmt::Display;
    /// Transaction ownership is observed on this same canonical connection.
    /// Joining an enclosing receipt must never start or commit another UoW.
    fn has_write_unit_of_work(&self) -> bool;
    fn begin_write_unit_of_work(&mut self) -> Result<(), Self::Error>;
    fn commit_write_unit_of_work(&mut self) -> Result<(), Self::Error>;
    fn rollback_write_unit_of_work(&mut self) -> Result<(), Self::Error>;
    fn begin_command_mutations(&mut self) -> Result<(), Self::Error>;
    fn commit_command_mutations(&mut self) -> Result<(), Self::Error>;
    fn rollback_command_mutations(&mut self) -> Result<(), Self::Error>;
    fn command_revision(
        &self,
        entity: CommandRevisionEntity<'_>,
    ) -> Result<Option<i64>, Self::Error>;
}

/// Append is a compare-and-swap against the scope cursor. Identity replay must
/// return the original event; an altered replay or stale cursor must not mutate
/// the journal. All events and their business command share one unit of work.
pub trait ProjectionJournalRepository {
    type Error: std::fmt::Display;
    fn append_projection_event(
        &mut self,
        input: &AppendProjectionEventInput,
    ) -> Result<StoredProjectionEvent, Self::Error>;
    fn projection_cursor(
        &self,
        scope: &ProjectionScopeKey,
    ) -> Result<Option<StoredProjectionCursor>, Self::Error>;
    fn projection_events_after(
        &self,
        scope: &ProjectionScopeKey,
        after_sequence: u64,
        limit: usize,
    ) -> Result<Vec<StoredProjectionEvent>, Self::Error>;
}

/// Composition port lending receipt admission and transaction control through
/// one object so both capabilities necessarily act on the same Unit of Work.
pub trait CommandProtocolRepositoryProvider {
    type Error: std::fmt::Display;
    type Repository<'a>: CommandReceiptRepository<Error = Self::Error>
        + CommandTransactionRepository<Error = Self::Error>
    where
        Self: 'a;

    fn command_protocol_repository(&mut self) -> Self::Repository<'_>;
}
