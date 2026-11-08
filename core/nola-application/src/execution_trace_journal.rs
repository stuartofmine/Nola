// SPDX-License-Identifier: Apache-2.0
//! Execution traces belong to accepted conversation transitions, not renderers.

use nola_agent_engine::agent_trace::{AgentTraceEvent, AgentTraceKind};
use nola_store_port::trace_journal::{AppendTraceJournalEventInput, TraceJournalRepository};

pub(crate) fn record<R: TraceJournalRepository + ?Sized>(
    store: &mut R,
    event: &AgentTraceEvent,
    envelope: Option<&serde_json::Value>,
) -> Result<bool, String> {
    fn label(value: impl serde::Serialize) -> Result<String, String> {
        serde_json::to_value(value).map_err(|e| e.to_string())?
            .as_str().map(str::to_owned).ok_or_else(|| "invalid trace enum representation".into())
    }
    let status = label(event.status)?;
    let is_terminal = matches!(event.kind,
        AgentTraceKind::RunCompleted | AgentTraceKind::RunFailed | AgentTraceKind::RunCancelled);
    let run_status = if is_terminal { Some(status.clone()) } else {
        match event.kind {
            AgentTraceKind::ToolPermissionRequested => Some("waiting".into()),
            AgentTraceKind::ToolPermissionResolved => Some("running".into()),
            _ => None,
        }
    };
    store.append_trace_journal_event(&AppendTraceJournalEventInput {
        event_id: event.event_id.clone(), trace_id: event.trace_id.clone(),
        run_id: event.run_id.clone(), conversation_id: event.conversation_id.clone(),
        message_id: event.message_id.clone(), turn_id: event.turn_id.clone(),
        sequence: i64::try_from(event.sequence).map_err(|_| "trace sequence exceeds i64")?,
        kind: label(event.kind)?, status, visibility: label(event.visibility)?,
        span_id: event.span_id.clone(), parent_span_id: event.parent_span_id.clone(),
        emitted_at: event.emitted_at.clone(),
        event_json: serde_json::to_string(event).map_err(|e| e.to_string())?,
        envelope_json: envelope.map(serde_json::to_string).transpose().map_err(|e| e.to_string())?,
        run_status, terminal_reason: is_terminal.then(|| event.payload.reason.clone()).flatten(),
        is_terminal,
    }).map_err(|e| e.to_string())
}

/// Bind Core-local trace identity to the canonical conversation at acceptance.
pub(crate) fn record_accepted<R: TraceJournalRepository + ?Sized>(
    store: &mut R, conversation_id: &str, traces: &[AgentTraceEvent],
) -> Result<(), String> {
    for trace in traces {
        let mut trace = trace.clone();
        trace.conversation_id = conversation_id.to_owned();
        record(store, &trace, None)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nola_agent_engine::agent_trace::{AgentTracePayload, AgentTraceStatus, AgentTraceVisibility};

    fn fixture() -> AgentTraceEvent {
        AgentTraceEvent {
            schema_version: 1, trace_id: "trace-a".into(), run_id: "run-a".into(),
            conversation_id: "core-session-a".into(), message_id: Some("message-a".into()),
            turn_id: "turn-a".into(), event_id: "event-a".into(), sequence: 1,
            emitted_at: "2026-10-15T00:00:00Z".into(), kind: AgentTraceKind::RunStarted,
            span_id: None, parent_span_id: None, status: AgentTraceStatus::Running,
            visibility: AgentTraceVisibility::Summary, payload: AgentTracePayload::default(),
        }
    }

    #[test]
    fn accepted_trace_is_owned_durable_and_idempotent_without_a_renderer() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("conversation.sqlite");
        let event = fixture();
        {
            let mut store = nola_store::NolaStore::open(&path).unwrap();
            record_accepted(&mut store, "conversation-a", &[event.clone()]).unwrap();
            record_accepted(&mut store, "conversation-a", &[event]).unwrap();
        }
        let store = nola_store::NolaStore::open(&path).unwrap();
        let events = store.trace_journal_events_after("trace-a", 0).unwrap();
        assert_eq!(events.len(), 1);
        let stored: AgentTraceEvent = serde_json::from_str(&events[0].event_json).unwrap();
        assert_eq!(stored.conversation_id, "conversation-a");
        assert!(store.trace_journal_runs_for_conversation("conversation-b").unwrap().is_empty());
    }

    #[test]
    fn rejected_acceptance_rolls_back_its_trace() {
        let root = tempfile::tempdir().unwrap();
        let mut store = nola_store::NolaStore::open(root.path().join("rollback.sqlite")).unwrap();
        let result: Result<(), String> = crate::assistant_publication_transaction::with_publication_transaction(
            &mut store, |store| {
                record_accepted(store, "conversation-a", &[fixture()])?;
                Err("rejected transition".into())
            },
        );
        assert!(result.is_err());
        assert!(store.trace_journal_events_after("trace-a", 0).unwrap().is_empty());
    }
}
