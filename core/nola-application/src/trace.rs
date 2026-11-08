// SPDX-License-Identifier: Apache-2.0
//! Application-owned trace read models.

use nola_agent_engine::agent_trace::AgentTraceEvent;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceRecord {
    pub event: AgentTraceEvent,
    pub envelope: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceRun {
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

impl TryFrom<nola_store_port::trace_journal::StoredTraceJournalEvent> for TraceRecord {
    type Error = String;

    fn try_from(
        record: nola_store_port::trace_journal::StoredTraceJournalEvent,
    ) -> Result<Self, Self::Error> {
        let event = serde_json::from_str(&record.event_json)
            .map_err(|error| format!("decode persisted trace event failed: {error}"))?;
        let envelope = record
            .envelope_json
            .map(|value| {
                serde_json::from_str(&value)
                    .map_err(|error| format!("decode persisted trace envelope failed: {error}"))
            })
            .transpose()?;
        Ok(Self { event, envelope })
    }
}

impl From<nola_store_port::trace_journal::StoredTraceJournalRun> for TraceRun {
    fn from(run: nola_store_port::trace_journal::StoredTraceJournalRun) -> Self {
        Self {
            trace_id: run.trace_id,
            run_id: run.run_id,
            conversation_id: run.conversation_id,
            message_id: run.message_id,
            turn_id: run.turn_id,
            status: run.status,
            last_sequence: run.last_sequence,
            terminal_reason: run.terminal_reason,
            started_at: run.started_at,
            updated_at: run.updated_at,
            finished_at: run.finished_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::TraceRun;

    #[test]
    fn run_wire_contract_is_a_projection_not_a_journal_row() {
        let value = serde_json::to_value(TraceRun {
            trace_id: "trace-1".into(),
            run_id: "run-1".into(),
            conversation_id: "conversation-1".into(),
            message_id: None,
            turn_id: "turn-1".into(),
            status: "running".into(),
            last_sequence: 1,
            terminal_reason: None,
            started_at: "2026-01-01T00:00:00Z".into(),
            updated_at: "2026-01-01T00:00:01Z".into(),
            finished_at: None,
        })
        .unwrap();

        assert!(value.get("event_json").is_none());
        assert!(value.get("envelope_json").is_none());
    }
}
