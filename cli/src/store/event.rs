use super::{AgentTraceRunRecord, NolaStore};
use crate::application_port::CliApplicationPort;
use chrono::Local;
use nola_application::{ApplicationCommand, TraceRecord, TraceRun};
use nola_core::agent_trace::AgentTraceEvent;
use serde_json::Value;

impl NolaStore {
    pub fn interrupt_orphaned_runs_for_session(&self, session_id: &str) -> Result<usize, String> {
        let mut application = CliApplicationPort::persistent(self.path())?;
        let now = Local::now();
        application.execute(ApplicationCommand::RecoverOrphanedRuns {
            conversation_id: Some(session_id.to_string()),
            reason: "owner_process_terminated".to_string(),
            interrupted_at: now.to_rfc3339(),
            interrupted_at_ms: now.timestamp_millis(),
        })
    }

    pub fn record_trace_event(&self, event: &AgentTraceEvent) -> Result<bool, String> {
        self.record_trace_event_with_envelope(event, None)
    }

    pub fn record_trace_event_envelope(
        &self,
        event: &AgentTraceEvent,
        envelope: &Value,
    ) -> Result<bool, String> {
        if !envelope.is_object() {
            return Err("trace transport envelope must be a JSON object".to_string());
        }
        self.record_trace_event_with_envelope(event, Some(envelope.clone()))
    }

    fn record_trace_event_with_envelope(
        &self,
        event: &AgentTraceEvent,
        envelope: Option<Value>,
    ) -> Result<bool, String> {
        let mut application = CliApplicationPort::persistent(self.path())?;
        application.execute(ApplicationCommand::RecordAgentTrace {
            event: event.clone(),
            envelope,
        })
    }

    pub fn trace_events_after(
        &self,
        trace_id: &str,
        sequence: u64,
    ) -> Result<Vec<AgentTraceEvent>, String> {
        let mut application = CliApplicationPort::persistent(self.path())?;
        let events: Vec<TraceRecord> =
            application.execute(ApplicationCommand::TraceEventsAfter {
                trace_id: trace_id.to_string(),
                sequence: i64::try_from(sequence)
                    .map_err(|_| "trace sequence exceeds i64".to_string())?,
            })?;
        Ok(events.into_iter().map(|record| record.event).collect())
    }

    pub fn trace_event_envelopes_after(
        &self,
        trace_id: &str,
        sequence: u64,
    ) -> Result<Vec<Value>, String> {
        let mut application = CliApplicationPort::persistent(self.path())?;
        let events: Vec<TraceRecord> =
            application.execute(ApplicationCommand::TraceEventsAfter {
                trace_id: trace_id.to_string(),
                sequence: i64::try_from(sequence)
                    .map_err(|_| "trace sequence exceeds i64".to_string())?,
            })?;
        events
            .into_iter()
            .map(|record| {
                if let Some(envelope) = record.envelope {
                    return Ok(envelope);
                }
                let event_type = serde_json::to_value(record.event.kind)
                    .ok()
                    .and_then(|value| value.as_str().map(str::to_string))
                    .unwrap_or_else(|| "trace.replay".to_string());
                Ok(serde_json::json!({"type": event_type, "trace": record.event}))
            })
            .collect()
    }

    pub fn latest_trace_id(&self, session_id: &str) -> Result<Option<String>, String> {
        let mut application = CliApplicationPort::persistent(self.path())?;
        application.execute(ApplicationCommand::LatestTraceId {
            conversation_id: session_id.to_string(),
        })
    }

    pub fn trace_runs_for_session(
        &self,
        session_id: &str,
    ) -> Result<Vec<AgentTraceRunRecord>, String> {
        let mut application = CliApplicationPort::persistent(self.path())?;
        let runs: Vec<TraceRun> =
            application.execute(ApplicationCommand::TraceRunsForConversation {
                conversation_id: session_id.to_string(),
            })?;
        Ok(runs
            .into_iter()
            .map(|run| AgentTraceRunRecord {
                trace_id: run.trace_id,
                run_id: run.run_id,
                message_id: run.message_id,
                status: run.status,
                last_sequence: u64::try_from(run.last_sequence.max(0)).unwrap_or(0),
                terminal_reason: run.terminal_reason,
                started_at: run.started_at,
                finished_at: run.finished_at,
            })
            .collect())
    }
}
