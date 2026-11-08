// SPDX-License-Identifier: Apache-2.0
//! Native transport sink: no terminal renderer, stdin, storage or business state.
use crate::runtime_output::{JournalPublicationObserver, RuntimeOutputPort};
use nola_core::agent::capability::SkillRuntimeEvent;
use nola_core::agent::protocol::AgentEvent;
use nola_core::agent::task::TaskHandle;
use nola_core::agent_trace::{attach_agent_trace, AgentTraceEvent};
use nola_domain::core_event::CoreEventEnvelope;
use serde_json::{json, Value};

pub(crate) struct NativeRuntimeOutput {
    json: bool,
    session_id: Option<String>,
    host_trace_binding: Option<(String, String)>,
    canonical_trace_observed: bool,
    next_trace: Option<AgentTraceEvent>,
    recovery_transport_suppressed: bool,
    terminal_transport_deferred: bool,
    deferred_terminal_transport: Vec<Value>,
    control: Option<crate::execution_context::ExecutionContext>,
    event_sink: Option<Box<dyn FnMut(Value) + Send>>,
}
impl NativeRuntimeOutput {
    pub(crate) fn new() -> Self {
        Self {
            json: true,
            session_id: None,
            host_trace_binding: None,
            canonical_trace_observed: false,
            next_trace: None,
            recovery_transport_suppressed: false,
            terminal_transport_deferred: false,
            deferred_terminal_transport: vec![],
            control: None,
            event_sink: None,
        }
    }
    pub(crate) fn with_event_sink(sink: impl FnMut(Value) + Send + 'static) -> Self {
        Self {
            event_sink: Some(Box::new(sink)),
            ..Self::new()
        }
    }
    fn deliver_result(&self, result: std::io::Result<()>) {
        if let Err(error) = result {
            eprintln!("native protocol delivery failed: {error}");
            if let Some(control) = &self.control {
                control.request();
            }
        }
    }
    fn emit_json_value(&mut self, value: Value) {
        if self.recovery_transport_suppressed {
            return;
        }
        if let Some(sink) = &mut self.event_sink {
            sink(value)
        } else {
            self.deliver_result(crate::native_host_output::emit(value));
        }
    }
    fn json_line(&mut self, mut value: Value) {
        if let Some(trace) = self.next_trace.take() {
            attach_agent_trace(&mut value, &trace);
        }
        let terminal = matches!(
            value.get("type").and_then(Value::as_str),
            Some("turn_completed" | "turn_cancelled" | "turn_failed" | "cancelled" | "error")
        ) || matches!(
            value.pointer("/trace/kind").and_then(Value::as_str),
            Some("run.completed" | "run.cancelled" | "run.failed")
        );
        if self.terminal_transport_deferred && terminal {
            self.deferred_terminal_transport.push(value);
        } else {
            self.emit_json_value(value);
        }
    }
}
impl RuntimeOutputPort for NativeRuntimeOutput {
    fn bind_execution_control(&mut self, context: crate::execution_context::ExecutionContext) {
        self.control = Some(context);
    }
    fn journal_observer(&self) -> Option<Box<dyn JournalPublicationObserver>> {
        None
    }
    fn is_json_transport(&self) -> bool {
        true
    }
    fn set_session_id(&mut self, id: Option<String>) {
        self.session_id = id;
    }
    fn set_host_turn_identity(
        &mut self,
        run_id: Option<String>,
        _assistant_message_id: Option<String>,
    ) {
        let binding = self.session_id.clone().zip(run_id);
        if self.host_trace_binding != binding { self.canonical_trace_observed = false; }
        self.host_trace_binding = binding;
        self.next_trace = None;
    }
    fn set_next_trace_message_id(&mut self, _message_id: Option<String>) {}
    fn begin_core_trace(&mut self, trace: &AgentTraceEvent) {
        self.canonical_trace_observed |= trace.kind != nola_core::agent_trace::AgentTraceKind::UsageSnapshot
            && self.host_trace_binding.as_ref().is_some_and(|(conversation, run)|
                trace.conversation_id == *conversation && trace.run_id == *run);
        self.next_trace = Some(trace.clone());
    }
    fn discard_core_state_snapshot_trace(&mut self) {
        if self.canonical_trace_observed && self.next_trace.as_ref().is_some_and(|trace| {
            self.host_trace_binding.as_ref().is_some_and(|(conversation, run)|
                trace.conversation_id == *conversation && trace.run_id == *run)
        }) {
            // An admitted Run's usage trace occupies a real sequence slot.
            // Publish it once, separately from the session footer projection.
            self.end_core_event_without_output();
        }
        self.next_trace = None;
    }
    fn end_core_event(&mut self) {
        if self.next_trace.is_some() {
            self.json_line(json!({"type":"agent_trace"}));
        }
    }
    fn end_core_event_without_output(&mut self) {
        self.end_core_event();
    }
    fn handoff_core_trace_to_host_event(&mut self) {
        if let Some(trace) = self.next_trace.take() {
            if self.event_sink.is_none()
                && !self.recovery_transport_suppressed
                && crate::tools::permission_trace_uses_direct_native_transport()
            {
                self.deliver_result(crate::native_host_output::reserve_challenge(
                    &trace.event_id,
                ));
            }
        }
    }
    fn status(&mut self, _phase: &str, _message: &str) {}
    fn done(&mut self, _reason: &str) {}
    fn error(&mut self, message: &str) {
        self.json_line(json!({"type":"error","detail":message,"reason":"failed"}));
    }
    fn defer_terminal_transport(&mut self) {
        self.terminal_transport_deferred = true;
    }
    fn discard_deferred_terminal_transport(&mut self) {
        self.terminal_transport_deferred = false;
        self.deferred_terminal_transport.clear();
    }
    fn flush_deferred_terminal_transport(&mut self) {
        self.terminal_transport_deferred = false;
        for frame in std::mem::take(&mut self.deferred_terminal_transport) {
            self.emit_json_value(frame);
        }
    }
    fn begin_recovery_transport_isolation(&mut self) {
        self.recovery_transport_suppressed = true;
    }
    fn end_recovery_transport_isolation(&mut self) {
        self.recovery_transport_suppressed = false;
        self.next_trace = None;
        self.host_trace_binding = None;
        self.canonical_trace_observed = false;
        self.discard_deferred_terminal_transport();
    }
    fn committed_assistant_journal(
        &mut self,
        through: nola_domain::assistant_journal::AssistantJournalCursor,
    ) {
        self.emit_json_value(json!({"type":"assistant_journal_committed_v1","session_id":through.identity.conversation_id,"source_host_id":"local","through":through}));
    }
    fn set_context_usage_without_breakdown(
        &mut self,
        used_tokens: usize,
        context_window_tokens: usize,
        prompt_token_limit: usize,
        message_count: Option<usize>,
        tool_count: usize,
        cache_usage: Option<nola_domain::PromptCacheUsage>,
    ) {
        if context_window_tokens == 0 {
            return;
        }
        let mut event = json!({"type":"context_usage","used_tokens":used_tokens,"context_window_tokens":context_window_tokens,
 "prompt_token_limit":prompt_token_limit,"limit_tokens":context_window_tokens,
 "reserved_output_tokens":context_window_tokens.saturating_sub(prompt_token_limit),
 "tool_count_scope":"host_catalog","breakdown_status":"unavailable",
 "tool_count":tool_count,"usage_percent":used_tokens as f64/context_window_tokens as f64*100.0,"cache_usage":cache_usage});
        if let Some(count) = message_count {
            event["message_count"] = json!(count);
        }
        self.json_line(event);
    }
    fn submission_admission(
        &mut self,
        conversation_id: &str,
        run_id: &str,
        assistant_message_id: &str,
        rejection: Option<&str>,
    ) {
        if rejection.is_none() {
            // Recovery isolation retired the previous presentation owner.
            // Rebind only from Application's committed admission receipt.
            self.set_session_id(Some(conversation_id.to_owned()));
            self.set_host_turn_identity(Some(run_id.to_owned()), Some(assistant_message_id.to_owned()));
        }
        if !self.json && self.event_sink.is_none() {
            return;
        }
        let mut frame = json!({
            "type": if rejection.is_some() { "submission_rejected" } else { "submission_admitted" },
            "conversation_id": conversation_id,
            "run_id": run_id,
            "assistant_message_id": assistant_message_id,
        });
        if let Some(message) = rejection {
            frame["code"] = json!("request_rejected_before_admission");
            frame["message"] = json!(message);
        }
        self.emit_json_value(frame);
    }
    fn core_turn_terminal(&mut self, envelope: &CoreEventEnvelope<AgentEvent>) {
        if !self.json {
            return;
        }
        let (event_type, reason) = match &envelope.event {
            AgentEvent::TurnCompleted { .. } => ("turn_completed", None),
            AgentEvent::TurnCancelled { reason, .. } => ("turn_cancelled", Some(reason.as_str())),
            AgentEvent::TurnFailed { error, .. } => ("turn_failed", Some(error.as_str())),
            _ => return,
        };
        self.json_line(json!({
            "protocol_version": 1,
            "type": event_type,
            "session_id": envelope.session_id,
            "turn_id": envelope.turn_id,
            "cursor": envelope.cursor.sequence,
            "revision": envelope.revision,
            "canonical_run_id": self.host_trace_binding.as_ref().map(|(_, run_id)| run_id.as_str()),
            "reason": reason,
            "event": envelope.event,
        }));
    }
    fn core_task_updated(
        &mut self,
        envelope: &CoreEventEnvelope<AgentEvent>,
        task_projection_revision: u64,
        task: &TaskHandle,
    ) {
        if !self.json {
            return;
        }
        self.json_line(json!({
            "protocol_version": 1,
            "type": "core_task_updated",
            "session_id": envelope.session_id,
            "turn_id": envelope.turn_id,
            "cursor": envelope.cursor.sequence,
            "revision": envelope.revision,
            "task_projection_revision": task_projection_revision,
            "task": task,
        }));
    }
    fn core_skill_runtime_updated(
        &mut self,
        envelope: &CoreEventEnvelope<AgentEvent>,
        skill_runtime_revision: u64,
        update: &SkillRuntimeEvent,
    ) {
        if !self.json {
            return;
        }
        self.json_line(json!({
            "protocol_version": 1,
            "type": "core_skill_runtime_updated",
            "session_id": envelope.session_id,
            "turn_id": envelope.turn_id,
            "cursor": envelope.cursor.sequence,
            "revision": envelope.revision,
            "skill_runtime_revision": skill_runtime_revision,
            "update": update,
        }));
    }
    fn core_capabilities_activated(
        &mut self,
        envelope: &CoreEventEnvelope<AgentEvent>,
        source: &str,
        capability_ids: &[String],
    ) {
        if !self.json {
            return;
        }
        self.json_line(json!({
            "protocol_version": 1,
            "type": "core_capabilities_activated",
            "session_id": envelope.session_id,
            "turn_id": envelope.turn_id,
            "cursor": envelope.cursor.sequence,
            "revision": envelope.revision,
            "source": source,
            "capability_ids": capability_ids,
        }));
    }
    fn core_permission_projection_updated(
        &mut self,
        session_id: &str,
        turn_id: Option<&str>,
        revision: u64,
        pending: &[nola_core::agent::permissions::PendingPermissionRequest],
        active_policy: Option<&nola_domain::ActivePermissionPolicy>,
    ) {
        if !self.json {
            return;
        }
        self.json_line(json!({
            "protocol_version": 1,
            "type": "core_permission_projection_updated",
            "session_id": session_id,
            "turn_id": turn_id,
            "permission_projection_revision": revision,
            "permission_projection": {"revision": revision, "pending": pending,"active_policy":active_policy},
        }));
    }
    fn committed_assistant_run_terminal(
        &mut self,
        seal: &nola_domain::assistant_run_terminal::AssistantRunTerminalSeal,
    ) {
        if !self.json || self.recovery_transport_suppressed {
            return;
        }
        self.emit_json_value(json!({
            "type": "assistant_run_terminal_committed_v1",
            "session_id": seal.identity.conversation_id,
            "source_host_id": "local",
            "run_id": seal.identity.run_id,
            "seal_digest": seal.seal_digest,
        }));
    }
    fn core_memory_runtime_projection_updated(
        &mut self,
        session_id: &str,
        turn_id: Option<&str>,
        projection: &nola_core::agent::MemoryRuntimeProjection,
    ) {
        if !self.json {
            return;
        }
        self.json_line(json!({
            "protocol_version": 1,
            "type": "core_memory_runtime_projection_updated",
            "session_id": session_id,
            "turn_id": turn_id,
            "memory_runtime_revision": projection.revision,
            "memory_runtime_projection": projection,
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    fn trace() -> AgentTraceEvent {
        nola_core::agent_trace::AgentTraceContext::with_identifier(
            "conversation",
            None,
            "run",
        )
        .project_host_event(
            &json!({"type":"tool_progress","title":"Working"}),
            "event-1",
            "2026-10-12T00:00:00.000Z",
        )
        .unwrap()
    }

    #[test]
    fn native_trace_sequence_survives_no_visible_output_and_status() {
        let frames = Arc::new(Mutex::new(Vec::new()));
        let sink = frames.clone();
        let mut output =
            NativeRuntimeOutput::with_event_sink(move |frame| sink.lock().unwrap().push(frame));
        let trace = trace();
        output.begin_core_trace(&trace);
        output.status("processing", "display only");
        output.end_core_event_without_output();
        output.end_core_event();
        let frames = frames.lock().unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0]["trace"]["sequence"], trace.sequence);
        assert_eq!(frames[0]["trace"]["event_id"], trace.event_id);
    }

    #[test]
    fn recovery_discards_old_transport_and_terminal_deferral_is_explicit() {
        let frames = Arc::new(Mutex::new(Vec::new()));
        let sink = frames.clone();
        let mut output =
            NativeRuntimeOutput::with_event_sink(move |frame| sink.lock().unwrap().push(frame));
        output.begin_recovery_transport_isolation();
        output.defer_terminal_transport();
        output.error("old Run");
        output.end_recovery_transport_isolation();
        output.flush_deferred_terminal_transport();
        assert!(frames.lock().unwrap().is_empty());
        output.defer_terminal_transport();
        output.error("new Run");
        assert!(frames.lock().unwrap().is_empty());
        output.flush_deferred_terminal_transport();
        assert_eq!(frames.lock().unwrap().len(), 1);
    }

    #[test]
    fn failed_delivery_cancels_only_bound_execution() {
        let bound = crate::execution_context::ExecutionContext::default();
        let other = crate::execution_context::ExecutionContext::default();
        let mut output = NativeRuntimeOutput::new();
        output.bind_execution_control(bound.clone());
        output.deliver_result(Err(std::io::Error::new(
            std::io::ErrorKind::BrokenPipe,
            "closed",
        )));
        assert!(bound.is_requested());
        assert!(!other.is_requested());
        assert!(output.journal_observer().is_none());
    }

    #[test]
    fn usage_snapshot_and_permission_handoff_do_not_emit_duplicate_traces() {
        let frames = Arc::new(Mutex::new(Vec::new()));
        let sink = frames.clone();
        let mut output =
            NativeRuntimeOutput::with_event_sink(move |frame| sink.lock().unwrap().push(frame));
        output.begin_core_trace(&trace());
        output.discard_core_state_snapshot_trace();
        output.set_context_usage_without_breakdown(1, 100, 90, None, 2, None);
        output.begin_core_trace(&trace());
        output.handoff_core_trace_to_host_event();
        output.end_core_event_without_output();
        let frames = frames.lock().unwrap();
        assert_eq!(frames.len(), 1);
        assert!(frames[0].get("trace").is_none());
        assert_eq!(frames[0]["breakdown_status"], "unavailable");
    }

    #[test]
    fn usage_trace_keeps_only_the_bound_runs_sequence_slot() {
        for owner in [None, Some("another-run"), Some("same-run")] {
            let frames = Arc::new(Mutex::new(Vec::new()));
            let sink = frames.clone();
            let mut output = NativeRuntimeOutput::with_event_sink(move |frame| sink.lock().unwrap().push(frame));
            let mut usage = trace();
            usage.kind = nola_core::agent_trace::AgentTraceKind::UsageSnapshot;
            output.set_session_id(Some(usage.conversation_id.clone()));
            output.set_host_turn_identity(owner.map(|value| if value == "same-run" { usage.run_id.clone() } else { value.into() }), None);
            output.begin_core_trace(&usage);
            output.discard_core_state_snapshot_trace();
            assert!(frames.lock().unwrap().is_empty(), "idle usage is not admission");
            let mut started = usage.clone();
            started.kind = nola_core::agent_trace::AgentTraceKind::RunStarted;
            output.begin_core_trace(&started);
            output.end_core_event();
            usage.sequence = 2;
            output.begin_core_trace(&usage);
            output.discard_core_state_snapshot_trace();
            output.set_context_usage_without_breakdown(1, 100, 90, None, 2, None);
            output.end_core_event();
            let frames = frames.lock().unwrap();
            assert_eq!(frames.iter().filter(|frame| frame.get("trace").is_some()).count(), 1 + usize::from(owner == Some("same-run")));
            assert!(frames.last().unwrap().get("trace").is_none());
        }
    }

    #[test]
    fn committed_admission_rebinds_trace_after_recovery_isolation() {
        let frames = Arc::new(Mutex::new(Vec::new()));
        let sink = frames.clone();
        let mut output = NativeRuntimeOutput::with_event_sink(move |frame| sink.lock().unwrap().push(frame));
        let mut usage = trace();
        output.set_session_id(Some(usage.conversation_id.clone()));
        output.begin_recovery_transport_isolation();
        output.set_host_turn_identity(Some(usage.run_id.clone()), usage.message_id.clone());
        output.end_recovery_transport_isolation();
        output.submission_admission(&usage.conversation_id, &usage.run_id, usage.message_id.as_deref().unwrap(), None);
        let mut started = usage.clone();
        started.kind = nola_core::agent_trace::AgentTraceKind::RunStarted;
        output.begin_core_trace(&started);
        output.end_core_event();
        usage.kind = nola_core::agent_trace::AgentTraceKind::UsageSnapshot;
        usage.sequence = 2;
        output.begin_core_trace(&usage);
        output.discard_core_state_snapshot_trace();
        output.set_context_usage_without_breakdown(1, 100, 90, None, 2, None);
        let frames = frames.lock().unwrap();
        let sequences: Vec<_> = frames.iter().filter_map(|frame| frame.get("trace")).map(|trace| trace["sequence"].as_u64().unwrap()).collect();
        assert_eq!(sequences, vec![1, 2]);
        assert!(frames.last().unwrap().get("trace").is_none());
    }
}
