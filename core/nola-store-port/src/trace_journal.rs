// SPDX-License-Identifier: Apache-2.0
//! Persistence facts independent of the concrete database.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppendTraceJournalEventInput {
    pub event_id: String,
    pub trace_id: String,
    pub run_id: String,
    pub conversation_id: String,
    pub message_id: Option<String>,
    pub turn_id: String,
    pub sequence: i64,
    pub kind: String,
    pub status: String,
    pub visibility: String,
    pub span_id: Option<String>,
    pub parent_span_id: Option<String>,
    pub emitted_at: String,
    pub event_json: String,
    pub envelope_json: Option<String>,
    pub run_status: Option<String>,
    pub terminal_reason: Option<String>,
    pub is_terminal: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredTraceJournalEvent {
    pub event_id: String,
    pub trace_id: String,
    pub conversation_id: String,
    pub turn_id: String,
    pub sequence: i64,
    pub kind: String,
    pub status: String,
    pub visibility: String,
    pub span_id: Option<String>,
    pub parent_span_id: Option<String>,
    pub emitted_at: String,
    pub event_json: String,
    pub envelope_json: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredTraceJournalRun {
    pub trace_id: String,
    pub run_id: String,
    pub conversation_id: String,
    pub message_id: Option<String>,
    pub turn_id: String,
    pub status: String,
    pub last_sequence: i64,
    pub terminal_reason: Option<String>,
    pub started_at: String,
    pub updated_at: String,
    pub finished_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredExactTraceReplay {
    pub trace_id: Option<String>,
    pub through_sequence: i64,
    pub next_sequence: i64,
    pub records: Vec<StoredTraceJournalEvent>,
}

/// Mutations participate in the caller's existing receipt Unit of Work.
pub trait TraceJournalRepository {
    type Error: std::fmt::Display;
    fn append_trace_journal_event(
        &mut self,
        input: &AppendTraceJournalEventInput,
    ) -> Result<bool, Self::Error>;
    fn trace_journal_events_after(
        &self,
        trace_id: &str,
        sequence: i64,
    ) -> Result<Vec<StoredTraceJournalEvent>, Self::Error>;
    /// Only traces bound to an admitted Run's exact conversation/root owner.
    /// Unbound diagnostic journals remain available through trace ID reads.
    fn trace_journal_runs_for_conversation(
        &self,
        conversation_id: &str,
    ) -> Result<Vec<StoredTraceJournalRun>, Self::Error>;
    fn exact_run_trace_replay(&self, conversation_id: &str, run_id: &str, assistant_message_id: &str,
        after_sequence: i64, through_sequence: Option<i64>, limit: usize,
    ) -> Result<StoredExactTraceReplay, Self::Error>;
    fn latest_trace_journal_id(&self, conversation_id: &str)
        -> Result<Option<String>, Self::Error>;
}
