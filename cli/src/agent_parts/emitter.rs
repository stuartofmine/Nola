use std::io::{self, IsTerminal, Write};
use std::sync::{Arc, Mutex};

use serde_json::{json, Map, Value};

use crate::i18n::{current_language, format_error, tr, ui_text, Language, Message, UiText};
use crate::interaction::ExecutionCancellation;
use crate::llm;
use crate::session::redact_sensitive_text;
use crate::store::NolaStore;
use crate::tools::{ToolOutcome, ToolRequest};
use nola_core::agent::capability::SkillRuntimeEvent;
use nola_core::agent::protocol::AgentEvent;
use nola_domain::core_event::CoreEventEnvelope;
use nola_core::agent::task::TaskHandle;
use nola_core::agent_trace::{attach_agent_trace, AgentTraceContext, AgentTraceEvent};
use unicode_width::UnicodeWidthChar;

impl crate::runtime_output::RuntimeOutputPort for Emitter {
 fn bind_execution_control(&mut self, context: crate::execution_context::ExecutionContext) {
 self.output_execution_context = Some(context.clone());
 if let Some(control) = crate::interaction::current_execution_cancellation().filter(|control| control.execution_id() == context.execution_id()) { self.attach_execution_cancellation(control); }
 }
fn submission_admission(&mut self, conversation_id: &str, run_id: &str,
        assistant_message_id: &str, rejection: Option<&str>) { Emitter::submission_admission(self, conversation_id, run_id, assistant_message_id, rejection) }
fn defer_terminal_transport(&mut self) { Emitter::defer_terminal_transport(self) }
fn begin_recovery_transport_isolation(&mut self) { Emitter::begin_recovery_transport_isolation(self) }
fn end_recovery_transport_isolation(&mut self) { Emitter::end_recovery_transport_isolation(self) }
fn flush_deferred_terminal_transport(&mut self) { Emitter::flush_deferred_terminal_transport(self) }
fn discard_deferred_terminal_transport(&mut self) { Emitter::discard_deferred_terminal_transport(self) }
fn set_session_id(&mut self, session_id: Option<String>) { Emitter::set_session_id(self, session_id) }
fn set_host_turn_identity(
        &mut self,
        run_id: Option<String>,
        assistant_message_id: Option<String>,
    ) { Emitter::set_host_turn_identity(self, run_id, assistant_message_id) }
fn set_next_trace_message_id(&mut self, message_id: Option<String>) { Emitter::set_next_trace_message_id(self, message_id) }
fn begin_core_trace(&mut self, trace: &AgentTraceEvent) { Emitter::begin_core_trace(self, trace) }
fn discard_core_state_snapshot_trace(&mut self) { Emitter::discard_core_state_snapshot_trace(self) }
fn core_turn_terminal(&mut self, envelope: &CoreEventEnvelope<AgentEvent>) { Emitter::core_turn_terminal(self, envelope) }
fn core_task_updated(
        &mut self,
        envelope: &CoreEventEnvelope<AgentEvent>,
        task_projection_revision: u64,
        task: &TaskHandle,
    ) { Emitter::core_task_updated(self, envelope, task_projection_revision, task) }
fn core_skill_runtime_updated(
        &mut self,
        envelope: &CoreEventEnvelope<AgentEvent>,
        skill_runtime_revision: u64,
        update: &SkillRuntimeEvent,
    ) { Emitter::core_skill_runtime_updated(self, envelope, skill_runtime_revision, update) }
fn core_capabilities_activated(
        &mut self,
        envelope: &CoreEventEnvelope<AgentEvent>,
        source: &str,
        capability_ids: &[String],
    ) { Emitter::core_capabilities_activated(self, envelope, source, capability_ids) }
fn core_permission_projection_updated(
        &mut self,
        session_id: &str,
        turn_id: Option<&str>,
        revision: u64,
        pending: &[nola_core::agent::permissions::PendingPermissionRequest],
        active_policy: Option<&nola_domain::ActivePermissionPolicy>,
    ) { Emitter::core_permission_projection_updated(self, session_id, turn_id, revision, pending, active_policy) }
fn committed_assistant_journal(
        &mut self,
        through: nola_domain::assistant_journal::AssistantJournalCursor,
    ) { Emitter::committed_assistant_journal(self, through) }
fn committed_assistant_run_terminal(
        &mut self,
        seal: &nola_domain::assistant_run_terminal::AssistantRunTerminalSeal,
    ) { Emitter::committed_assistant_run_terminal(self, seal) }
fn core_memory_runtime_projection_updated(
        &mut self,
        session_id: &str,
        turn_id: Option<&str>,
        projection: &nola_core::agent::MemoryRuntimeProjection,
    ) { Emitter::core_memory_runtime_projection_updated(self, session_id, turn_id, projection) }
fn end_core_event(&mut self) { Emitter::end_core_event(self) }
fn end_core_event_without_output(&mut self) { Emitter::end_core_event_without_output(self) }
fn handoff_core_trace_to_host_event(&mut self) { Emitter::handoff_core_trace_to_host_event(self) }
fn set_context_usage_without_breakdown(
        &mut self,
        used_tokens: usize,
        context_window_tokens: usize,
        prompt_token_limit: usize,
        message_count: Option<usize>,
        tool_count: usize,
        cache_usage: Option<nola_domain::PromptCacheUsage>,
    ) { Emitter::set_context_usage_without_breakdown(self, used_tokens, context_window_tokens, prompt_token_limit, message_count, tool_count, cache_usage) }
fn is_json_transport(&self) -> bool { Emitter::is_json_transport(self) }
fn status(&mut self, phase: &str, message: &str) { Emitter::status(self, phase, message) }
fn done(&mut self, reason: &str) { Emitter::done(self, reason) }
fn error(&mut self, message: &str) { Emitter::error(self, message) }
fn journal_observer(&self) -> Option<Box<dyn crate::runtime_output::JournalPublicationObserver>> { Emitter::journal_observer(self) }
}


const HUMAN_STATUS_LABEL_WIDTH: usize = 10;
const HUMAN_STATUS_DETAIL_MAX_WIDTH: usize = 140;
const STREAM_COMMIT_MAX_LINES: usize = 3;
const STREAM_COMMIT_MAX_CHARS: usize = 360;
const STREAM_COMMIT_SOFT_CHARS: usize = 240;
static TERMINAL_RENDER_LOCK: Mutex<()> = Mutex::new(());

fn emit_durable_terminal_line(line: &str) {
    if crate::terminal_ui::owns_terminal() {
        crate::terminal_ui::commit_history(line);
    } else {
        println!("{line}");
    }
}

fn is_terminal_transport_frame(value: &Value) -> bool {
    matches!(
        value.get("type").and_then(Value::as_str),
        Some("turn_completed" | "turn_cancelled" | "turn_failed" | "cancelled" | "error")
    ) || matches!(
        value.pointer("/trace/kind").and_then(Value::as_str),
        Some("run.completed" | "run.cancelled" | "run.failed")
    )
}

pub(crate) fn print_queued_follow_up(text: &str) {
    if !std::io::stdout().is_terminal() || text.trim().is_empty() {
        return;
    }
    let _render_guard = TERMINAL_RENDER_LOCK
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    let width = agent_terminal_columns().saturating_sub(6).max(1);
    let visible = truncate_display_value(text.trim(), width);
    let line = if terminal_color_enabled() {
        format!(
            "    {}›{} {}{}{}",
            crate::INTERACTIVE_ACCENT,
            crate::INTERACTIVE_RESET,
            crate::INTERACTIVE_VALUE,
            visible,
            crate::INTERACTIVE_RESET
        )
    } else {
        format!("    › {visible}")
    };
    crate::terminal_ui::queue_user_history_after_assistant(format!("\n{line}\n\n"));
}


#[cfg(test)]
mod native_host_trace_transport_tests {
    use super::{AgentTraceContext, Emitter};
    use crate::tools::{ToolOutcome, ToolRequest};
    use nola_core::agent::protocol::{AgentEvent, SessionId, TurnId};
    use nola_domain::core_event::{CoreEventEnvelope, EventCursor};
    use nola_core::agent::task::TaskHandle;
    use serde_json::{json, Value};
    use std::sync::{Arc, Mutex};

    #[test]
    fn host_startup_failure_and_cancellation_keep_the_canonical_run_identity() {
        for (detail, kind) in [
            ("fixture startup failed", "run.failed"),
            ("cancelled by user", "run.cancelled"),
        ] {
            let captured = Arc::new(Mutex::new(Vec::<Value>::new()));
            let sink = Arc::clone(&captured);
            let mut emitter = Emitter::with_event_sink(move |event| {
                sink.lock().unwrap().push(event);
            });
            emitter.session_id = Some("conversation-identity".into());
            emitter.set_host_turn_identity(
                Some("run-with-dashes".into()),
                Some("assistant-bound".into()),
            );
            emitter.error(detail);

            let events = captured.lock().unwrap();
            assert_eq!(events.len(), 1);
            let trace = &events[0]["trace"];
            assert_eq!(trace["run_id"], "run-with-dashes");
            assert_eq!(trace["turn_id"], "run-with-dashes");
            assert_eq!(trace["trace_id"], "trace_run_with_dashes");
            assert_eq!(trace["message_id"], "assistant-bound");
            assert_eq!(trace["kind"], kind);
            assert_eq!(trace["sequence"], 1);
        }
    }

    #[test]
    fn outer_transport_errors_do_not_create_a_second_core_run_or_terminal() {
        for event in [
            json!({"type":"error", "detail":"core failed"}),
            json!({"type":"cancelled", "reason":"cancelled"}),
            json!({"type":"done", "reason":"completed"}),
        ] {
            let captured = Arc::new(Mutex::new(Vec::<Value>::new()));
            let sink = Arc::clone(&captured);
            let mut emitter = Emitter::with_event_sink(move |event| {
                sink.lock().unwrap().push(event);
            });
            emitter.session_id = Some("conversation-identity".into());
            emitter.set_host_turn_identity(
                Some("run-with-dashes".into()),
                Some("assistant-bound".into()),
            );
            let mut context = AgentTraceContext::for_turn(
                "conversation-identity",
                Some("assistant-bound".into()),
                "run-with-dashes",
            );
            let trace = context.project_external_event(&event).unwrap();
            emitter.begin_core_trace(&trace);
            emitter.end_core_event_without_output();
            emitter.error("outer transport reported the same terminal failure");

            let events = captured.lock().unwrap();
            assert_eq!(events.len(), 2);
            assert_eq!(events[0]["trace"], serde_json::to_value(&trace).unwrap());
            assert!(events[1].get("trace").is_none());
            assert_eq!(events[1]["type"], "error");
            assert_eq!(
                events[1]["detail"],
                "outer transport reported the same terminal failure"
            );
            assert_eq!(
                events
                    .iter()
                    .filter(|event| event.get("trace").is_some())
                    .count(),
                1
            );
        }
    }

    #[test]
    fn state_snapshot_detachment_does_not_release_core_trace_ownership() {
        let captured = Arc::new(Mutex::new(Vec::<Value>::new()));
        let sink = Arc::clone(&captured);
        let mut emitter = Emitter::with_event_sink(move |event| {
            sink.lock().unwrap().push(event);
        });
        emitter.session_id = Some("conversation-identity".into());
        emitter.set_host_turn_identity(Some("first-run".into()), Some("first-assistant".into()));
        let mut context = AgentTraceContext::for_turn(
            "conversation-identity",
            Some("first-assistant".into()),
            "first-run",
        );
        let started = context
            .project_agent_event_at(
                &AgentEvent::TurnStarted {
                    session_id: SessionId::from_string("conversation-identity".into()),
                    turn_id: TurnId::from_string("first-run".into()),
                    assistant_message_id: "first-assistant".into(),
                },
                1,
                "2026-10-04T00:00:00.000Z",
            )
            .unwrap();
        emitter.begin_core_trace(&started);
        emitter.end_core_event_without_output();
        let usage = context
            .project_external_event(&json!({"type":"context_usage", "used_tokens":1}))
            .unwrap();
        emitter.begin_core_trace(&usage);
        emitter.discard_core_state_snapshot_trace();
        assert!(emitter.active_core_trace_identity.is_none());
        // Rebinding the same resumed Run must not release Core ownership or
        // reset its pre-Core diagnostic cursor.
        emitter.set_host_turn_identity(Some("first-run".into()), Some("first-assistant".into()));
        emitter.error("transport disconnected while Run was active");
        assert_eq!(captured.lock().unwrap()[1]["trace"]["kind"], "usage.snapshot");
        assert_eq!(captured.lock().unwrap()[1]["trace"]["sequence"], 2);
        assert!(captured.lock().unwrap()[2].get("trace").is_none());

        emitter.set_host_turn_identity(Some("second-run".into()), Some("second-assistant".into()));
        emitter.error("next Run failed before Core admission");
        let events = captured.lock().unwrap();
        assert_eq!(events[3]["trace"]["run_id"], "second-run");
        assert_eq!(events[3]["trace"]["message_id"], "second-assistant");
        assert_eq!(events[3]["trace"]["sequence"], 1);
    }

    #[test]
    fn idle_usage_before_admission_does_not_hide_the_bound_startup_failure() {
        let captured = Arc::new(Mutex::new(Vec::<Value>::new()));
        let sink = Arc::clone(&captured);
        let mut emitter = Emitter::with_event_sink(move |event| {
            sink.lock().unwrap().push(event);
        });
        emitter.session_id = Some("conversation-identity".into());
        emitter.set_host_turn_identity(Some("new-run".into()), Some("new-assistant".into()));
        let mut context = AgentTraceContext::for_turn(
            "conversation-identity",
            Some("new-assistant".into()),
            "new-run",
        );
        let usage = context
            .project_external_event(&json!({"type":"context_usage", "used_tokens":1}))
            .unwrap();
        emitter.begin_core_trace(&usage);
        emitter.discard_core_state_snapshot_trace();
        emitter.error("the Run failed before admission");
        let events = captured.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["trace"]["run_id"], "new-run");
        assert_eq!(events[0]["trace"]["message_id"], "new-assistant");
        assert_eq!(events[0]["trace"]["kind"], "run.failed");
        assert_eq!(events[0]["trace"]["sequence"], 1);
    }

    #[test]
    fn a_different_core_run_cannot_take_ownership_of_the_bound_startup_trace() {
        let mut emitter = Emitter::with_event_sink(|_| {});
        emitter.session_id = Some("conversation-identity".into());
        emitter.set_host_turn_identity(Some("new-run".into()), Some("new-assistant".into()));
        let mut context = AgentTraceContext::for_turn(
            "conversation-identity",
            Some("prior-assistant".into()),
            "prior-run",
        );
        let prior = context
            .project_external_event(&json!({"type":"error", "detail":"prior Run failed"}))
            .unwrap();
        emitter.begin_core_trace(&prior);
        assert!(!emitter.canonical_trace_observed);
    }

    #[test]
    fn canonical_failure_trace_is_preserved_when_error_formats_the_frame() {
        let captured = Arc::new(Mutex::new(Vec::<Value>::new()));
        let sink = Arc::clone(&captured);
        let mut emitter = Emitter::with_event_sink(move |event| {
            sink.lock().unwrap().push(event);
        });
        let mut context =
            AgentTraceContext::for_turn("conversation", Some("assistant".into()), "run-id");
        let trace = context
            .project_external_event(&json!({"type":"error", "detail":"canonical failure"}))
            .unwrap();
        emitter.begin_core_trace(&trace);
        emitter.error("canonical failure");
        let events = captured.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["trace"], serde_json::to_value(trace).unwrap());
    }

    #[test]
    fn delivery_failure_cancels_attached_run_without_thread_scope() {
        let cancellation = crate::interaction::ExecutionCancellation::default();
        let worker_cancellation = cancellation.clone();
        std::thread::spawn(move || {
            assert!(crate::interaction::current_execution_cancellation().is_none());
            let mut emitter = Emitter::new(true);
            emitter.attach_execution_cancellation(worker_cancellation.clone());
            emitter.handle_native_delivery_result(Ok(()));
            assert!(!worker_cancellation.is_requested());
            emitter.handle_native_delivery_result(Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "fixture disconnected",
            )));
        })
        .join()
        .unwrap();
        assert!(cancellation.is_requested());
    }

    #[test]
    fn delivery_failure_does_not_cancel_unrelated_ambient_run() {
        let attached = crate::interaction::ExecutionCancellation::default();
        let ambient = crate::interaction::ExecutionCancellation::default();
        let _scope = ambient.enter();
        let mut emitter = Emitter::new(true);
        emitter.attach_execution_cancellation(attached.clone());
        emitter.handle_native_delivery_result(Err(std::io::Error::new(
            std::io::ErrorKind::BrokenPipe,
            "fixture disconnected",
        )));
        assert!(attached.is_requested());
        assert!(!ambient.is_requested());
    }

    #[test]
    fn terminal_quiet_event_still_reaches_native_host_transport() {
        let captured = Arc::new(Mutex::new(Vec::<Value>::new()));
        let sink = Arc::clone(&captured);
        let mut emitter = Emitter::with_event_sink(move |event| {
            sink.lock().expect("capture event").push(event);
        });
        let mut context = AgentTraceContext::with_identifier(
            "conversation-transport-test".to_string(),
            None,
            "turn-transport-test".to_string(),
        );
        let trace = context
            .project_host_event(
                &json!({"type": "tool_progress", "title": "Working"}),
                "event-transport-test",
                "2026-08-01T00:00:00.000Z",
            )
            .unwrap();

        emitter.begin_core_trace(&trace);
        emitter.end_core_event_without_output();

        let events = captured.lock().expect("read captured events");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["type"], "agent_trace");
        assert_eq!(events[0]["trace"]["sequence"], trace.sequence);
        assert_eq!(events[0]["trace"]["event_id"], trace.event_id);
    }

    #[test]
    fn recovery_transport_isolation_drops_prior_run_frames_and_terminal_state() {
        let captured = Arc::new(Mutex::new(Vec::<Value>::new()));
        let sink = Arc::clone(&captured);
        let mut emitter = Emitter::with_event_sink(move |event| {
            sink.lock().expect("capture event").push(event);
        });

        emitter.begin_recovery_transport_isolation();
        emitter.error("prior Run was interrupted");
        emitter.end_recovery_transport_isolation();
        assert!(emitter.take_terminal_reason().is_none());

        emitter.json_line(json!({"type":"status", "state":"running"}));
        let events = captured.lock().expect("read captured events");
        assert_eq!(
            events.as_slice(),
            &[json!({"type":"status", "state":"running"})]
        );
    }

    #[test]
    fn terminal_notification_carries_only_exact_run_and_seal_identity() {
        let captured = Arc::new(Mutex::new(Vec::<Value>::new()));
        let sink = Arc::clone(&captured);
        let mut emitter = Emitter::with_event_sink(move |event| {
            sink.lock()
                .expect("capture terminal projection")
                .push(event);
        });
        let (_source, journal) = crate::source_journal::source_tests::completed_source(
            &[],
            "Actual sealed answer",
        );
        let seal = journal.seal.unwrap();
        seal.validate().unwrap();
        // A recomputed digest cannot turn genesis admission into a terminal fact.
        let mut forged = seal.clone();
        forged.admission = nola_domain::run_message_admission::RunMessageAdmissionCursor::initial(
            seal.identity.clone(),
        )
        .unwrap();
        forged.seal_digest = forged.computed_digest().unwrap();
        assert!(forged.validate().is_err());
        emitter.committed_assistant_run_terminal(&seal);

        let events = captured.lock().expect("read terminal projection");
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0],
            json!({"type":"assistant_run_terminal_committed_v1",
            "session_id":seal.identity.conversation_id, "source_host_id":"local", "run_id":seal.identity.run_id,
            "seal_digest":seal.seal_digest})
        );
        assert!(serde_json::to_vec(&events[0]).unwrap().len() < 512);
    }

    #[test]
    fn context_usage_is_an_untraced_state_projection() {
        let captured = Arc::new(Mutex::new(Vec::<Value>::new()));
        let sink = Arc::clone(&captured);
        let mut emitter = Emitter::with_event_sink(move |event| {
            sink.lock().expect("capture event").push(event);
        });
        let mut context = AgentTraceContext::with_identifier(
            "conversation-context-test".to_string(),
            None,
            "turn-context-test".to_string(),
        );
        let trace = context
            .project_host_event(
                &json!({
                    "type": "context_usage",
                    "used_tokens": 4096,
                    "context_window_tokens": 65536
                }),
                "event-context-test",
                "2026-08-04T00:00:00.000Z",
            )
            .unwrap();

        emitter.begin_core_trace(&trace);
        emitter.discard_core_state_snapshot_trace();
        emitter.set_context_usage_with_breakdown(
            4096,
            65536,
            65536,
            4096,
            0,
            3,
            2,
            0,
            0,
            4096,
            Value::Array(Vec::new()),
        );
        emitter.end_core_event();

        let events = captured.lock().expect("read captured events");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["type"], "context_usage");
        assert!(events[0].get("trace").is_none());
        assert!(emitter.next_trace.is_none());
        assert!(emitter.active_core_trace_identity.is_none());
    }

    #[test]
    fn context_total_does_not_fabricate_an_unknown_breakdown() {
        let captured = Arc::new(Mutex::new(Vec::<Value>::new()));
        let sink = Arc::clone(&captured);
        let mut emitter = Emitter::with_event_sink(move |event| {
            sink.lock().expect("capture event").push(event);
        });

        emitter.set_context_usage_without_breakdown(4096, 65536, 60000, None, 7, None);

        let events = captured.lock().expect("read captured events");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["used_tokens"], 4096);
        assert_eq!(events[0]["prompt_token_limit"], 60000);
        assert_eq!(events[0]["reserved_output_tokens"], 5536);
        assert_eq!(events[0]["breakdown_status"], "unavailable");
        assert_eq!(events[0]["tool_count"], 7);
        assert_eq!(events[0]["tool_count_scope"], "host_catalog");
        for unknown in [
            "message_tokens",
            "tool_schema_tokens",
            "component_tokens",
            "tool_result_tokens",
            "conversation_tokens",
            "component_metrics",
            "message_count",
        ] {
            assert!(
                events[0].get(unknown).is_none(),
                "{unknown} must stay unknown"
            );
        }
        drop(events);

        let cache = nola_domain::PromptCacheUsage::measured(Some(5000), Some(4000), Some(0), false);
        emitter.set_context_usage_without_breakdown(5000, 65536, 60000, Some(12), 8, Some(cache));
        assert_eq!(emitter.take_context_usage().unwrap().cache_usage, Some(cache));
        let events = captured.lock().expect("read known count event");
        assert_eq!(events[1]["message_count"], 12);
        assert_eq!(events[1]["breakdown_status"], "unavailable");
        assert_eq!(events[1]["cache_usage"]["hit_rate_bps"], 8000);
    }

    #[test]
    fn content_has_no_diagnostic_trace_to_emit() {
        let mut context = AgentTraceContext::for_turn(
            "conversation-content-test",
            Some("assistant-content-test".into()),
            "turn-content-test",
        );
        for kind in ["assistant_delta", "assistant_reset", "thinking_delta"] {
            assert!(context
                .project_host_event(
                    &json!({"type":kind,"delta":"Canonical body"}),
                    "event-content-test",
                    "2026-08-04T00:00:00Z"
                )
                .is_none());
        }
        let trace = context
            .project_external_event(&json!({"type":"status","phase":"processing"}))
            .unwrap();
        assert_eq!(trace.sequence, 1);
    }

    #[test]
    fn tool_completion_transport_emits_only_canonical_agent_trace() {
        let captured = Arc::new(Mutex::new(Vec::<Value>::new()));
        let sink = Arc::clone(&captured);
        let trace_context = Arc::new(Mutex::new(AgentTraceContext::with_identifier(
            "conversation-tool-test".to_string(),
            None,
            "turn-tool-test".to_string(),
        )));
        let mut emitter = Emitter::with_event_sink(move |event| {
            sink.lock().expect("capture event").push(event);
        })
        .with_trace_context(trace_context);
        let request = ToolRequest {
            id: "call-1".to_string(),
            name: "read_file".to_string(),
            args: serde_json::Map::new(),
        };
        let outcome = ToolOutcome {
            full_output_ref: None,
            data: None,
            execution: None,
            text: "contents".to_string(),
            display: "Read file".to_string(),
            error: false,
            artifacts: Vec::new(),
            activation: None,
        };

        emitter.tool_result(&request, &outcome);

        let events = captured.lock().expect("read captured events");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["type"], "agent_trace");
        assert_eq!(events[0]["trace"]["kind"], "tool.completed");
        assert_eq!(events[0]["trace"]["payload"]["call_id"], "call-1");
        assert_ne!(events[0]["type"], "tool_result");
    }

    #[test]
    fn core_task_event_uses_the_shared_lifecycle_contract() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/core_task_lifecycle_v1.json"
        ))
        .expect("decode shared Task fixture");
        let expected = &fixture["events"][0];
        let task: TaskHandle =
            serde_json::from_value(expected["task"].clone()).expect("decode fixture TaskHandle");
        let envelope = CoreEventEnvelope {
            session_id: expected["session_id"].as_str().unwrap().to_string(),
            turn_id: Some(expected["turn_id"].as_str().unwrap().to_string()),
            cursor: EventCursor {
                sequence: expected["cursor"].as_u64().unwrap(),
            },
            revision: expected["revision"].as_u64().unwrap(),
            event: AgentEvent::TaskUpdated {
                task_projection_revision: expected["task_projection_revision"].as_u64().unwrap(),
                task: task.clone(),
            },
        };
        assert_eq!(
            task.owner_session_id,
            SessionId::from_string("conversation-task-contract".to_string())
        );
        let captured = Arc::new(Mutex::new(Vec::<Value>::new()));
        let sink = Arc::clone(&captured);
        let mut emitter = Emitter::with_event_sink(move |event| {
            sink.lock().expect("capture Task event").push(event);
        });
        emitter.core_task_updated(
            &envelope,
            expected["task_projection_revision"].as_u64().unwrap(),
            &task,
        );
        let events = captured.lock().expect("read Task event");
        assert_eq!(events[0]["type"], "core_task_updated");
        assert_eq!(events[0]["task"], expected["task"]);
        assert_eq!(events[0]["task_projection_revision"], 3);
    }

    #[test]
    fn shared_capability_activation_contract_projects_to_cli_once() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../../core/tests/fixtures/capability_activation_v1.json"
        ))
        .expect("decode shared capability activation fixture");
        let envelope: CoreEventEnvelope<AgentEvent> =
            serde_json::from_value(fixture["core_envelope"].clone())
                .expect("decode Core capability activation envelope");
        let AgentEvent::CapabilitiesActivated {
            source,
            capability_ids,
            ..
        } = &envelope.event
        else {
            panic!("expected capabilities_activated event");
        };
        let captured = Arc::new(Mutex::new(Vec::<Value>::new()));
        let sink = Arc::clone(&captured);
        let mut emitter = Emitter::with_event_sink(move |event| {
            sink.lock().expect("capture CLI projection").push(event);
        });

        emitter.core_capabilities_activated(&envelope, source, capability_ids);

        let events = captured.lock().expect("read CLI projection");
        assert_eq!(events.as_slice(), &[fixture["cli_projection"].clone()]);
    }
}


pub struct Emitter {
    json: bool,
    printed_text: bool,
    event_sink: Option<Box<dyn FnMut(Value) + Send>>,
    markdown_text: String,
    quiet_status: bool,
    last_quiet_status: Option<String>,
    in_code_block: bool,
    code_block_lang: Option<String>,
    code_block_buffer: String,
    stream_text_buffer: String,
    stream_text_is_preview: bool,
    streaming_vertical_table: Option<StreamingVerticalTable>,
    output_ended_with_newline: bool,
    clean_before_first_text: bool,
    terminal_reason: Option<String>,
    session_id: Option<String>,
    tool_event_store: Option<NolaStore>,
    iteration_index: Option<usize>,
    last_tool_name: String,
    pending_tool_items: Vec<DisplayedToolItem>,
    activity_spinner: Option<EmitterActivitySpinner>,
    compaction_active: bool,
    review_active: bool,
    sticky_footer_base: Option<String>,
    sticky_footer_context: Option<nola_core::agent::context_window::ContextUsageSnapshot>,
    last_context_usage: Option<nola_core::agent::context_window::ContextUsageSnapshot>,
    sticky_footer_plan: Option<String>,
    sticky_footer_visible_lines: usize,
    sticky_footer_reserved_lines: usize,
    sticky_footer_render_key: Option<(String, u16, u16)>,
    sticky_footer_suspended: bool,
    execution_cancellation: Option<ExecutionCancellation>,
    transient_status_visible: bool,
    trace_context: Option<Arc<Mutex<AgentTraceContext>>>,
    next_trace: Option<AgentTraceEvent>,
    active_core_trace_identity: Option<(String, String, String, Option<String>, String)>,
    // Once Core owns a Run's trace, transport failures remain diagnostics.
    // They cannot create another lifecycle with an independent sequence.
    canonical_trace_observed: bool,
    host_trace_binding: Option<(String, String)>,
    recovery_transport_suppressed: bool,
    terminal_transport_deferred: bool,
    deferred_terminal_transport: Vec<Value>,
    output_execution_context: Option<crate::execution_context::ExecutionContext>,
    journal_inbox: std::sync::Arc<std::sync::Mutex<crate::terminal_journal_presentation::JournalInbox>>,
    journal_presentation: crate::terminal_journal_presentation::TerminalJournalPresentation,
}

#[derive(Debug, Clone)]
pub struct SessionPlanSummary {
    pub completed: usize,
    pub total: usize,
    pub current: Option<String>,
    pub steps: Vec<SessionPlanStep>,
}

#[derive(Debug, Clone)]
pub struct SessionPlanStep {
    pub id: String,
    pub content: String,
    pub active_form: String,
    pub status: String,
    pub priority: String,
    pub dependency_ids: Vec<String>,
    pub approval_status: String,
}

#[derive(Debug, Clone)]
struct DisplayedToolItem {
    name: String,
    args: Option<String>,
    detail: Option<String>,
    is_error: bool,
    is_read_activity: bool,
}

#[derive(Debug, Clone)]
struct StreamingVerticalTable {
    header: Vec<String>,
    columns: usize,
    emitted_rows: usize,
}

struct EmitterActivitySpinner {
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    rendered: std::sync::Arc<std::sync::atomic::AtomicBool>,
    detail: std::sync::Arc<std::sync::Mutex<String>>,
    handle: Option<std::thread::JoinHandle<()>>,
}

// A stage clock, not an estimate of task completion. Tool detail updates do
// not restart it; a lifecycle transition creates a new spinner/stage.
fn timed_activity_detail(detail: &str, elapsed: std::time::Duration) -> String {
    let seconds = elapsed.as_secs();
    let elapsed = if seconds < 60 {
        format!("{seconds}s")
    } else {
        format!("{}m {:02}s", seconds / 60, seconds % 60)
    };
    format!("{elapsed} · {detail}")
}

impl EmitterActivitySpinner {
    fn start(
        detail: &str,
        cancellation: Option<ExecutionCancellation>,
    ) -> Self {
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let thread_stop = std::sync::Arc::clone(&stop);
        let rendered = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let thread_rendered = std::sync::Arc::clone(&rendered);
        let detail = std::sync::Arc::new(std::sync::Mutex::new(detail.to_string()));
        let thread_detail = std::sync::Arc::clone(&detail);
        let reduced_motion = crate::terminal_ui::reduced_motion();
        let handle = std::thread::spawn(move || {
            let started = std::time::Instant::now();
            // Avoid painting operations that complete within one perceptual
            // frame. Drawing and immediately clearing them causes the brief
            // tool-call flash that leaks into some terminal scrollback.
            std::thread::park_timeout(std::time::Duration::from_millis(140));
            if thread_stop.load(std::sync::atomic::Ordering::Acquire) {
                return;
            }
            let current_detail = thread_detail
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .clone();
            render_emitter_activity_frame(
                if reduced_motion { "•" } else { EMITTER_ACTIVITY_SPINNER_FRAMES[0] },
                &timed_activity_detail(&current_detail, started.elapsed()),
                cancellation.as_ref(),
            );
            thread_rendered.store(true, std::sync::atomic::Ordering::Release);
            let mut frame = 1usize;
            while !thread_stop.load(std::sync::atomic::Ordering::Acquire) {
                std::thread::park_timeout(std::time::Duration::from_millis(if reduced_motion { 1000 } else { 110 }));
                if thread_stop.load(std::sync::atomic::Ordering::Acquire) {
                    break;
                }
                if crate::interaction::permission_prompt_active() {
                    continue;
                }
                let current_detail = thread_detail
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .clone();
                render_emitter_activity_frame(
                    if reduced_motion { "•" } else { EMITTER_ACTIVITY_SPINNER_FRAMES[frame % EMITTER_ACTIVITY_SPINNER_FRAMES.len()] },
                    &timed_activity_detail(&current_detail, started.elapsed()),
                    cancellation.as_ref(),
                );
                frame = frame.saturating_add(1);
            }
        });
        Self {
            stop,
            rendered,
            detail,
            handle: Some(handle),
        }
    }

    fn update_detail(&self, detail: &str) {
        let detail = detail.trim();
        if detail.is_empty() {
            return;
        }
        *self
            .detail
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = detail.to_string();
        if let Some(handle) = self.handle.as_ref() {
            handle.thread().unpark();
        }
    }

    fn stop(&mut self) -> bool {
        self.stop.store(true, std::sync::atomic::Ordering::Release);
        if let Some(handle) = self.handle.take() {
            handle.thread().unpark();
            let _ = handle.join();
        }
        self.rendered.load(std::sync::atomic::Ordering::Acquire)
    }
}

impl Drop for EmitterActivitySpinner {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

const EMITTER_ACTIVITY_SPINNER_FRAMES: [&str; 10] =
    ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

fn render_emitter_activity_frame(
    frame: &str,
    detail: &str,
    cancellation: Option<&ExecutionCancellation>,
) {
    let _render_guard = TERMINAL_RENDER_LOCK
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    if crate::interaction::permission_prompt_active() {
        return;
    }
    if crate::terminal_ui::owns_terminal() {
        if let Some(cancellation) = cancellation {
            crate::terminal_ui::sync_composer(cancellation);
        }
        crate::terminal_ui::set_activity("agent-activity", frame, detail);
        return;
    }
    render_execution_composer_row(cancellation);
    let detail = match cancellation.map(ExecutionCancellation::input_snapshot) {
        Some(snapshot) if snapshot.queued_count > 0 => format!(
            "{detail}  ·  {} {}",
            tr(Message::InteractiveExecutionQueuedPrefix)
                .trim_end_matches([':', '：'])
                .trim(),
            snapshot.queued_count
        ),
        _ => detail.to_string(),
    };
    // The status row is redrawn in place. Letting it reach the terminal edge
    // makes the terminal wrap before the next clear-line sequence, leaving a
    // stale fragment in scrollback and moving the composer cursor. Reserve
    // one column at the right edge in addition to the two-space/spinner prefix.
    let detail_width = agent_terminal_columns()
        .saturating_sub(5)
        .min(HUMAN_STATUS_DETAIL_MAX_WIDTH)
        .max(1);
    let detail = truncate_display_value(&detail, detail_width);
    if terminal_color_enabled() {
        print!(
            "\r\x1b[2K  \x1b[38;5;75m{frame}{CODE_COLOR_RESET} {CODE_COLOR_COMMENT}{detail}{CODE_COLOR_RESET}"
        );
    } else {
        print!("\r\x1b[2K  {frame} {detail}");
    }
    let _ = io::stdout().flush();
}

fn render_execution_composer_row(cancellation: Option<&ExecutionCancellation>) {
    let Some(cancellation) = cancellation.filter(|value| value.accepts_terminal_input()) else {
        return;
    };
    if crate::terminal_ui::owns_terminal() {
        crate::terminal_ui::sync_composer(cancellation);
        return;
    }
    let Ok((columns, rows)) = crossterm::terminal::size() else {
        return;
    };
    if columns == 0 || rows == 0 {
        return;
    }
    let content = execution_composer_bar(cancellation);
    print!("\x1b7\x1b[{rows};1H\x1b[2K");
    print!("{content}");
    print!("\x1b8");
    let _ = io::stdout().flush();
}

fn execution_composer_bar(cancellation: &ExecutionCancellation) -> String {
    let snapshot = cancellation.input_snapshot();
    let mut visible_draft = snapshot.draft.clone();
    if !visible_draft.is_empty() {
        let cursor = snapshot.cursor.min(visible_draft.len());
        if visible_draft.is_char_boundary(cursor) {
            visible_draft.insert(cursor, '▏');
        }
    }
    let placeholder = if snapshot.queued_count == 0 {
        tr(Message::InteractiveInputPlaceholder).to_string()
    } else {
        format!(
            "{} · {} {}",
            tr(Message::InteractiveInputPlaceholder),
            tr(Message::InteractiveExecutionQueuedPrefix),
            snapshot.queued_count
        )
    };
    crate::interactive_input_bar(&visible_draft, Some(&placeholder), true)
}

pub(crate) fn redraw_execution_composer(cancellation: &ExecutionCancellation) {
    if crate::terminal_ui::owns_terminal() {
        crate::terminal_ui::sync_composer(cancellation);
        return;
    }
    let _render_guard = TERMINAL_RENDER_LOCK
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    if crate::interaction::permission_prompt_active() {
        return;
    }
    render_execution_composer_row(Some(cancellation));
}

#[cfg(test)]
mod submission_admission_transport_tests {
    use super::*;

    #[test]
    fn receipts_bind_the_exact_submission_and_do_not_claim_execution_success() {
        let frames = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let captured = frames.clone();
        let mut emitter = Emitter::with_event_sink(move |frame| captured.lock().unwrap().push(frame));
        emitter.submission_admission("conversation", "run-1", "assistant-1", None);
        emitter.submission_admission("conversation", "run-2", "assistant-2", Some("history unavailable"));
        let frames = frames.lock().unwrap();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0], json!({"type":"submission_admitted", "conversation_id":"conversation",
            "run_id":"run-1", "assistant_message_id":"assistant-1"}));
        assert_eq!(frames[1], json!({"type":"submission_rejected", "conversation_id":"conversation",
            "run_id":"run-2", "assistant_message_id":"assistant-2",
            "code":"request_rejected_before_admission", "message":"history unavailable"}));
    }
}

impl Emitter {
    /// Transport receipt, not model output or evidence of successful execution.
    pub(crate) fn submission_admission(&mut self, conversation_id: &str, run_id: &str,
        assistant_message_id: &str, rejection: Option<&str>) {
        if rejection.is_none() {
            if self.session_id.as_deref() != Some(conversation_id) {
                self.set_session_id(Some(conversation_id.to_owned()));
            }
            self.set_host_turn_identity(Some(run_id.to_owned()), Some(assistant_message_id.to_owned()));
        }
        if !self.json && self.event_sink.is_none() { return; }
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

    pub fn new(json: bool) -> Self {
        Self {
            json,
            printed_text: false,
            markdown_text: String::new(),
            event_sink: None,
            quiet_status: true,
            last_quiet_status: None,
            in_code_block: false,
            code_block_lang: None,
            code_block_buffer: String::new(),
            stream_text_buffer: String::new(),
            stream_text_is_preview: false,
            streaming_vertical_table: None,
            output_ended_with_newline: true,
            clean_before_first_text: true,
            terminal_reason: None,
            session_id: None,
            tool_event_store: None,
            iteration_index: None,
            last_tool_name: String::new(),
            pending_tool_items: Vec::new(),
            activity_spinner: None,
            compaction_active: false,
            review_active: false,
            sticky_footer_base: None,
            sticky_footer_context: None,
            last_context_usage: None,
            sticky_footer_plan: None,
            sticky_footer_visible_lines: 0,
            sticky_footer_reserved_lines: 0,
            sticky_footer_render_key: None,
            sticky_footer_suspended: false,
            execution_cancellation: None,
            transient_status_visible: false,
            trace_context: None,
            next_trace: None,
            active_core_trace_identity: None,
            canonical_trace_observed: false,
            host_trace_binding: None,
            recovery_transport_suppressed: false,
            terminal_transport_deferred: false,
            deferred_terminal_transport: Vec::new(),
            output_execution_context: None,
            journal_inbox: Default::default(),
            journal_presentation: Default::default(),
        }
    }

    pub(crate) fn defer_terminal_transport(&mut self) {
        self.terminal_transport_deferred = self.json;
    }

    /// A foreground native request may first have to settle a previously
    /// committed Run. Those frames retain the prior Run/message identities and
    /// are persisted through Core, but must not be mixed into the new request's
    /// transport stream.
    pub(crate) fn begin_recovery_transport_isolation(&mut self) {
        self.recovery_transport_suppressed = self.json;
    }

    pub(crate) fn end_recovery_transport_isolation(&mut self) {
        self.recovery_transport_suppressed = false;
        self.next_trace = None;
        self.active_core_trace_identity = None;
        self.canonical_trace_observed = false;
        self.host_trace_binding = None;
        self.discard_deferred_terminal_transport();
        self.terminal_reason = None;
    }

    pub(crate) fn flush_deferred_terminal_transport(&mut self) {
        self.terminal_transport_deferred = false;
        for value in std::mem::take(&mut self.deferred_terminal_transport) {
            self.emit_json_value(value);
        }
    }

    pub(crate) fn discard_deferred_terminal_transport(&mut self) {
        self.terminal_transport_deferred = false;
        self.deferred_terminal_transport.clear();
    }

    pub fn with_quiet_status(mut self, quiet_status: bool) -> Self {
        self.quiet_status = quiet_status;
        self
    }

    pub fn with_sticky_footer(mut self, footer: Option<String>) -> Self {
        if !self.json {
            self.sticky_footer_base =
                footer.and_then(|value| (!value.trim().is_empty()).then_some(value));
        }
        self
    }

    pub fn attach_execution_cancellation(&mut self, cancellation: ExecutionCancellation) {
        self.execution_cancellation = Some(cancellation);
        self.sticky_footer_render_key = None;
        self.draw_sticky_footer();
    }

    fn handle_native_delivery_result(&self, result: io::Result<()>) {
        if let Err(error) = result {
            eprintln!("native protocol delivery failed: {error}");
            if let Some(context) = &self.output_execution_context {
                context.request();
                return;
            }
            // Delivery callbacks can run outside the caller's thread scope.
            // The emitter's attached run is authoritative, not ambient TLS.
            if let Some(cancellation) = self.execution_cancellation.as_ref() {
                cancellation.request();
            } else if let Some(cancellation) = crate::interaction::current_execution_cancellation()
            {
                cancellation.request();
            }
        }
    }

    pub fn take_queued_user_inputs(&mut self) -> Vec<String> {
        self.execution_cancellation
            .as_ref()
            .map(ExecutionCancellation::take_queued_inputs)
            .unwrap_or_default()
    }

    pub fn take_draft_user_input(&mut self) -> Option<String> {
        self.execution_cancellation
            .as_ref()
            .and_then(ExecutionCancellation::draft_input)
    }

    pub fn wait_for_execution_input_release(&self) {
        if let Some(cancellation) = self.execution_cancellation.as_ref() {
            cancellation.wait_for_terminal_listener_release();
        }
    }

    #[cfg(test)]
    pub fn set_context_usage(
        &mut self,
        used_tokens: usize,
        context_window_tokens: usize,
        prompt_token_limit: usize,
        message_tokens: usize,
        tool_schema_tokens: usize,
        message_count: usize,
        tool_count: usize,
    ) {
        self.set_context_usage_with_breakdown(
            used_tokens,
            context_window_tokens,
            prompt_token_limit,
            message_tokens,
            tool_schema_tokens,
            message_count,
            tool_count,
            0,
            0,
            message_tokens,
            Value::Array(Vec::new()),
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn set_context_usage_with_breakdown(
        &mut self,
        used_tokens: usize,
        context_window_tokens: usize,
        prompt_token_limit: usize,
        message_tokens: usize,
        tool_schema_tokens: usize,
        message_count: usize,
        tool_count: usize,
        component_tokens: usize,
        tool_result_tokens: usize,
        conversation_tokens: usize,
        component_metrics: Value,
    ) {
        if context_window_tokens == 0 {
            return;
        }
        let usage = nola_core::agent::context_window::ContextUsageSnapshot {
            used_tokens,
            context_window_tokens,
            prompt_token_limit,
            usage_percent: used_tokens as f64 / context_window_tokens as f64 * 100.0,
            usage_is_estimated: true,
            ..Default::default()
        };
        self.last_context_usage = Some(usage.clone());
        if self.json {
            self.json_line(context_usage_event_with_breakdown(
                used_tokens,
                context_window_tokens,
                prompt_token_limit,
                message_tokens,
                tool_schema_tokens,
                message_count,
                tool_count,
                component_tokens,
                tool_result_tokens,
                conversation_tokens,
                &component_metrics,
            ));
            return;
        }
        if self.event_sink.is_some() {
            self.json_line(context_usage_event_with_breakdown(
                used_tokens,
                context_window_tokens,
                prompt_token_limit,
                message_tokens,
                tool_schema_tokens,
                message_count,
                tool_count,
                component_tokens,
                tool_result_tokens,
                conversation_tokens,
                &component_metrics,
            ));
        }
        // Context belongs to the composer footer. Rendering it while a turn is
        // active leaks a transient footer into scrollback and competes with
        // tool activity and assistant output for the same terminal rows.
    }

    /// Publishes an authoritative total when Core does not have component
    /// accounting. Unknown dimensions stay absent instead of being reported
    /// as zero or duplicating the total as message/conversation usage.
    pub fn set_context_usage_without_breakdown(
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
        let usage = nola_core::agent::context_window::ContextUsageSnapshot {
            cache_usage,
            used_tokens,
            context_window_tokens,
            prompt_token_limit,
            usage_percent: used_tokens as f64 / context_window_tokens as f64 * 100.0,
            usage_is_estimated: true,
            ..Default::default()
        };
        self.last_context_usage = Some(usage);
        if self.json || self.event_sink.is_some() {
            let mut event = context_usage_event_without_breakdown(
                used_tokens,
                context_window_tokens,
                prompt_token_limit,
                tool_count,
            );
            if let Some(message_count) = message_count {
                event["message_count"] = Value::from(message_count);
            }
            event["cache_usage"] = serde_json::to_value(cache_usage).unwrap_or(Value::Null);
            self.json_line(event);
        }
    }

    fn start_activity_spinner(&mut self, detail: &str) {
        if self.json || !sticky_footer_terminal_enabled() || detail.trim().is_empty() {
            return;
        }
        self.stop_activity_spinner();
        self.clear_transient_status_line();
        self.ensure_output_line_for_status();
        self.activity_spinner = Some(EmitterActivitySpinner::start(
            detail.trim(),
            self.execution_cancellation.clone(),
        ));
        self.output_ended_with_newline = false;
    }

    fn stop_activity_spinner(&mut self) {
        let Some(mut spinner) = self.activity_spinner.take() else {
            return;
        };
        let rendered = spinner.stop();
        if crate::terminal_ui::owns_terminal() {
            crate::terminal_ui::clear_activity();
            self.output_ended_with_newline = true;
            return;
        }
        if rendered {
            print!("{}", clear_current_output_line());
        }
        self.output_ended_with_newline = true;
        let _ = io::stdout().flush();
    }

    pub fn set_session_id(&mut self, session_id: Option<String>) {
        self.tool_event_store = session_id
            .as_ref()
            .and_then(|_| NolaStore::open_default().ok());
        let trace_context = self.trace_context.clone().unwrap_or_else(|| {
            Arc::new(Mutex::new(
                session_id
                    .as_ref()
                    .map(|session_id| {
                        AgentTraceContext::with_identifier(
                            session_id.clone(),
                            None,
                            uuid::Uuid::new_v4().to_string(),
                        )
                    })
                    .unwrap_or_else(|| {
                        let identifier = uuid::Uuid::new_v4().to_string();
                        AgentTraceContext::with_identifier(
                            format!("conversation_{identifier}"),
                            None,
                            identifier,
                        )
                    }),
            ))
        });
        self.trace_context = Some(trace_context);
        self.session_id = session_id;
    }

    pub fn set_host_turn_identity(
        &mut self,
        run_id: Option<String>,
        assistant_message_id: Option<String>,
    ) {
        let Some(run_id) = run_id.filter(|value| !value.trim().is_empty()) else {
            return;
        };
        let conversation_id = self
            .session_id
            .clone()
            .unwrap_or_else(|| format!("conversation_{}", uuid::Uuid::new_v4()));
        self.host_trace_binding = Some((conversation_id.clone(), run_id.clone()));
        if self.trace_context.as_ref().is_some_and(|context| {
            context.lock().ok().is_some_and(|context| {
                context.run_id == run_id
                    && context.conversation_id == conversation_id
                    && context.message_id == assistant_message_id
            })
        }) {
            return;
        }
        self.trace_context = Some(Arc::new(Mutex::new(AgentTraceContext::for_turn(
            conversation_id,
            assistant_message_id,
            run_id,
        ))));
        self.canonical_trace_observed = false;
        self.next_trace = None;
        self.active_core_trace_identity = None;
    }

    pub fn set_next_trace_message_id(&mut self, message_id: Option<String>) {
        if let Some(context) = self.trace_context.as_ref() {
            if let Ok(mut context) = context.lock() {
                context.set_next_message_id(message_id);
            }
        }
    }

    pub fn with_trace_context(mut self, trace_context: Arc<Mutex<AgentTraceContext>>) -> Self {
        self.trace_context = Some(trace_context);
        self
    }

    pub fn begin_core_event(&mut self, event: &AgentEvent, event_cursor: u64) {
        self.next_trace = self
            .trace_context
            .as_ref()
            .and_then(|context| context.lock().ok())
            .and_then(|mut context| {
                context.project_agent_event_at(
                    event,
                    event_cursor,
                    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                )
            });
        self.canonical_trace_observed |= self
            .next_trace
            .as_ref()
            .is_some_and(|trace| self.is_bound_core_run_trace(trace));
    }

    pub fn begin_core_trace(&mut self, trace: &AgentTraceEvent) {
        self.canonical_trace_observed |= self.is_bound_core_run_trace(trace);
        self.iteration_index = trace
            .turn_id
            .rsplit(':')
            .next()
            .and_then(|value| value.parse::<usize>().ok());
        self.active_core_trace_identity = Some((
            trace.trace_id.clone(),
            trace.run_id.clone(),
            trace.conversation_id.clone(),
            trace.message_id.clone(),
            trace.turn_id.clone(),
        ));
        self.next_trace = Some(trace.clone());
    }

    fn is_bound_core_run_trace(&self, trace: &AgentTraceEvent) -> bool {
        if trace.kind == nola_core::agent_trace::AgentTraceKind::UsageSnapshot {
            return false;
        }
        self.host_trace_binding
            .as_ref()
            .is_none_or(|(conversation_id, run_id)| {
                trace.conversation_id == *conversation_id && trace.run_id == *run_id
            })
    }

    /// Unbound context usage is session state, never a new Agent run. A usage
    /// trace belonging to the admitted Run still occupies a sequence slot;
    /// publish it separately before emitting the untraced footer projection.
    pub fn discard_core_state_snapshot_trace(&mut self) {
        let discarded_trace_id = self.next_trace.as_ref().map(|trace| trace.trace_id.clone());
        if self.canonical_trace_observed && self.next_trace.as_ref().is_some_and(|trace| {
            self.host_trace_binding.as_ref().is_some_and(|(conversation, run)|
                trace.conversation_id == *conversation && trace.run_id == *run)
        }) {
            self.end_core_event_without_output();
        }
        self.next_trace = None;
        if discarded_trace_id.as_ref().is_some_and(|trace_id| {
            self.active_core_trace_identity
                .as_ref()
                .is_some_and(|identity| &identity.0 == trace_id)
        }) {
            self.active_core_trace_identity = None;
        }
    }

    /// Publishes the committed Core terminal with its actual execution owner.
    /// The canonical Run is a separate transport binding, never its session ID.
    pub fn core_turn_terminal(&mut self, envelope: &CoreEventEnvelope<AgentEvent>) {
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

    /// Publishes Core's canonical Task lifecycle. This is intentionally
    /// separate from tool Trace: short foreground tools never become Tasks.
    pub fn core_task_updated(
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

    pub fn core_skill_runtime_updated(
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

    pub fn core_capabilities_activated(
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

    /// Publishes Core's checkpointed permission lifecycle. Native Hosts use
    /// this projection to clear stale prompts and restore stable request IDs;
    /// Host-specific authorization details remain in permission trace events.
    pub fn core_permission_projection_updated(
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

    pub(crate) fn journal_observer(&self) -> Option<Box<dyn crate::runtime_output::JournalPublicationObserver>> {
        (!self.json).then(|| Box::new(crate::terminal_journal_presentation::JournalObserver(
            self.journal_inbox.clone(),
        )) as Box<dyn crate::runtime_output::JournalPublicationObserver>)
    }

    pub(crate) fn committed_assistant_journal(
        &mut self,
        through: nola_domain::assistant_journal::AssistantJournalCursor,
    ) {
        if self.recovery_transport_suppressed {
            return;
        }
        if !self.json {
            let updates = std::mem::take(&mut self.journal_inbox.lock()
                .unwrap_or_else(|error| error.into_inner()).updates);
            let mut presentation = std::mem::take(&mut self.journal_presentation);
            for update in updates {
                presentation.present_journal_update(self, update);
            }
            self.journal_presentation = presentation;
            return;
        }
        self.emit_json_value(json!({
            "type": "assistant_journal_committed_v1",
            "session_id": through.identity.conversation_id,
            "source_host_id": "local",
            "through": through,
        }));
    }

    /// A bounded hint to read immutable completion evidence. Clients must
    /// durably catch up/admit the seal before treating the Run as completed.
    pub(crate) fn committed_assistant_run_terminal(
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

    /// Publishes Core's checkpointed memory-job lifecycle. Native Hosts use
    /// this projection to restore queued/running/retry state without deriving
    /// background work from foreground tool traces or process lifetime.
    pub fn core_memory_runtime_projection_updated(
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

    pub fn end_core_event(&mut self) {
        if !self.json {
            if let Some(trace) = self.next_trace.take() {
                if let Some(store) = self.tool_event_store.as_ref() {
                    let _ = store.record_trace_event(&trace);
                }
            }
            return;
        }
        if self.next_trace.is_some() {
            self.json_line(json!({"type": "agent_trace"}));
        }
        self.next_trace = None;
    }

    pub fn end_core_event_without_output(&mut self) {
        // "Without output" only applies to the human terminal presentation.
        // Native Hosts consume the JSON event sink as a canonical, strictly
        // sequenced protocol. Dropping a trace here creates a sequence gap and
        // causes strict clients to reject every subsequent event in the turn.
        if self.json && self.next_trace.is_some() {
            self.json_line(json!({"type": "agent_trace"}));
            return;
        }
        if let Some(trace) = self.next_trace.take() {
            if let Some(store) = self.tool_event_store.as_ref() {
                let _ = store.record_trace_event(&trace);
            }
        }
    }

    /// Hands the current Core trace to a richer Host event that will carry the
    /// same trace envelope plus Host-only protocol fields. This prevents a
    /// bare duplicate trace from reaching strict native clients first.
    pub fn handoff_core_trace_to_host_event(&mut self) {
        let handed_off_trace_id = self.next_trace.take().map(|trace| {
            if let Some(store) = self.tool_event_store.as_ref() {
                let _ = store.record_trace_event(&trace);
            }
            if self.json
                && self.event_sink.is_none()
                && !self.recovery_transport_suppressed
                && crate::tools::permission_trace_uses_direct_native_transport()
            {
                self.handle_native_delivery_result(crate::native_host_output::reserve_challenge(
                    &trace.event_id,
                ));
            }
            trace.trace_id
        });
        if handed_off_trace_id.as_ref().is_some_and(|trace_id| {
            self.active_core_trace_identity
                .as_ref()
                .is_some_and(|identity| &identity.0 == trace_id)
        }) {
            self.active_core_trace_identity = None;
        }
    }

    pub fn with_event_sink<F>(event_sink: F) -> Self
    where
        F: FnMut(Value) + Send + 'static,
    {
        Self {
            json: true,
            printed_text: false,
            markdown_text: String::new(),
            event_sink: Some(Box::new(event_sink)),
            quiet_status: true,
            last_quiet_status: None,
            in_code_block: false,
            code_block_lang: None,
            code_block_buffer: String::new(),
            stream_text_buffer: String::new(),
            stream_text_is_preview: false,
            streaming_vertical_table: None,
            output_ended_with_newline: true,
            clean_before_first_text: true,
            terminal_reason: None,
            session_id: None,
            tool_event_store: None,
            iteration_index: None,
            last_tool_name: String::new(),
            pending_tool_items: Vec::new(),
            activity_spinner: None,
            compaction_active: false,
            review_active: false,
            sticky_footer_base: None,
            sticky_footer_context: None,
            last_context_usage: None,
            sticky_footer_plan: None,
            sticky_footer_visible_lines: 0,
            sticky_footer_reserved_lines: 0,
            sticky_footer_render_key: None,
            sticky_footer_suspended: false,
            execution_cancellation: None,
            transient_status_visible: false,
            trace_context: None,
            next_trace: None,
            active_core_trace_identity: None,
            canonical_trace_observed: false,
            host_trace_binding: None,
            recovery_transport_suppressed: false,
            terminal_transport_deferred: false,
            deferred_terminal_transport: Vec::new(),
            output_execution_context: None,
            journal_inbox: Default::default(),
            journal_presentation: Default::default(),
        }
    }

    pub fn iteration(&mut self, index: usize) {
        self.iteration_index = Some(index);
        if !self.json && !self.quiet_status {
            self.status("turn", &format!("turn={index}"));
        }
    }

    pub fn text(&mut self, text: &str) {
        self.present_text(text, false);
    }

    pub fn preview_text(&mut self, text: &str) {
        self.present_text(text, true);
    }

    pub fn text_review(&mut self, state: nola_core::agent::parts::TextReviewState) {
        if self.json {
            return;
        }
        self.flush_pending_tool_block();
        self.flush_stream_text(true);
        self.stop_activity_spinner();
        let phase = match state {
            nola_core::agent::parts::TextReviewState::Pending => "answer_review_pending",
            nola_core::agent::parts::TextReviewState::Approved => "answer_review_approved",
            nola_core::agent::parts::TextReviewState::Rejected => "answer_review_rejected",
            nola_core::agent::parts::TextReviewState::Interrupted => "answer_review_interrupted",
        };
        self.print_status_line("review", &crate::i18n::localize_agent_phase(phase), None);
    }

    pub fn finish_reviewed_text_segment(&mut self) {
        self.flush_stream_text(true);
        if self.in_code_block {
            self.flush_code_buffer();
            self.in_code_block = false;
            self.code_block_lang = None;
        }
        if crate::terminal_ui::owns_terminal() {
            crate::terminal_ui::finish_assistant_segment();
            self.output_ended_with_newline = true;
        } else {
            self.ensure_output_line_for_status();
        }
    }

    fn present_text(&mut self, text: &str, preview: bool) {
        if self.json {
            // JSON clients read committed source journals; never publish a
            // second response-body transport.
            let _ = text;
        } else {
            if crate::terminal_ui::has_active_document() {
                self.stop_activity_spinner();
                self.clear_transient_status_line();
                self.flush_pending_tool_block();
                crate::terminal_ui::append_markdown(text);
                self.markdown_text.push_str(text);
                self.printed_text = true;
                self.draw_sticky_footer();
                return;
            }
            if self.stream_text_is_preview != preview && !self.stream_text_buffer.is_empty() {
                self.flush_stream_text(true);
            }
            self.stream_text_is_preview = preview;
            if !preview {
                self.stop_activity_spinner();
                self.clear_transient_status_line();
            }
            self.prepare_sticky_footer_for_durable_output();
            if !preview {
                self.flush_pending_tool_block();
            }
            self.enqueue_stream_text(text);
            self.markdown_text.push_str(text);
            self.printed_text = true;
            self.draw_sticky_footer();
        }
    }

    pub fn commit_text_preview(&mut self) {
        if self.json {
            return;
        }
        // Intermediate model/tool rounds share the same provisional text
        // channel. Publish the whole turn's bounded tool receipt only once
        // Core confirms that this preview is the accepted assistant answer.
        self.flush_pending_tool_block();
        self.flush_stream_text(true);
        crate::terminal_ui::commit_assistant_preview();
        self.stream_text_is_preview = false;
    }

    pub fn discard_text_preview(&mut self) {
        self.stream_text_buffer.clear();
        self.streaming_vertical_table = None;
        self.markdown_text.clear();
        self.in_code_block = false;
        self.code_block_lang = None;
        self.code_block_buffer.clear();
        self.stream_text_is_preview = false;
        crate::terminal_ui::discard_assistant_preview();
    }

    pub fn take_markdown_text(&mut self) -> String {
        std::mem::take(&mut self.markdown_text)
    }

    #[cfg(test)]
    pub(crate) fn has_text_preview(&self) -> bool {
        self.stream_text_is_preview
    }

    pub fn take_terminal_reason(&mut self) -> Option<String> {
        self.terminal_reason.take()
    }

    pub fn take_context_usage(
        &mut self,
    ) -> Option<nola_core::agent::context_window::ContextUsageSnapshot> {
        self.last_context_usage.take()
    }

    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    pub fn is_json_transport(&self) -> bool {
        self.json
    }

    pub fn reasoning(&mut self, text: &str) {
        if self.json {
            // Reasoning is represented by Core Reasoning Parts and Trace.
            let _ = text;
        }
    }

    pub fn status(&mut self, phase: &str, message: &str) {
        // Compaction also invokes a model. Its provider-attempt notification
        // must not replace the more specific, still-active lifecycle stage.
        if phase == "model_attempt_started" && (self.compaction_active || self.review_active) {
            return;
        }
        let phase = if phase == "model_attempt_started" {
            "processing"
        } else {
            phase
        };
        match phase {
            "compacting" => self.compaction_active = true,
            "reviewing" => self.review_active = true,
            "processing" => {
                self.compaction_active = false;
                self.review_active = false;
            }
            _ => {}
        }
        let message = if phase == "model_retry" && message == "model_retry" {
            agent_localized_text(Message::AgentStatusRetryingModel)
        } else {
            message
        };
        self.flush_stream_text(true);
        if self.json {
            // JSON clients consume the authoritative Core trace. Presentation
            // status must never manufacture a second run or sequence.
        } else if phase == "model_retry" {
            self.stop_activity_spinner();
            self.print_status_line(
                "error",
                agent_localized_text(Message::AgentStatusRetryingModel),
                Some(message),
            );
        } else if self.quiet_status {
            if phase == "processing" || phase == "compacting" || phase == "reviewing" {
                let detail = if phase == "compacting" || phase == "reviewing" {
                    message
                } else {
                    agent_localized_text(Message::AgentStatusThinking)
                };
                let quiet_key = format!("{phase}|{detail}");
                if self.last_quiet_status.as_deref() != Some(&quiet_key)
                    || self.activity_spinner.is_none()
                {
                    self.last_quiet_status = Some(quiet_key);
                    self.start_activity_spinner(detail);
                }
                return;
            }
            if phase != "decide" {
                return;
            }
            if let Some(detail) = should_show_quiet_status(phase, message) {
                self.stop_activity_spinner();
                let quiet_key = format!("{phase}|{detail}");
                if self.last_quiet_status.as_deref() != Some(&quiet_key)
                    || !self.transient_status_visible
                {
                    self.last_quiet_status = Some(quiet_key);
                    self.draw_transient_status_line(&detail);
                }
            }
        } else {
            self.stop_activity_spinner();
            self.print_status_line("status", phase, Some(message));
        }
    }

    pub fn tool_use(
        &mut self,
        request: &ToolRequest,
        repaired_from: Option<&str>,
        repaired_fields: Option<&[String]>,
    ) {
        self.prepare_sticky_footer_for_durable_output();
        // Journal visibility, not the presence/name of a tool, owns content.
        // A public narration stays in the transcript at its original position.
        self.flush_stream_text(true);
        if crate::terminal_ui::has_active_document() {
            crate::terminal_ui::finish_assistant_segment();
        }
        self.draw_sticky_footer();
        let mut debug_payload = tool_debug_use_payload(request, repaired_from, repaired_fields);
        if let Some(session_id) = self.session_id.as_deref() {
            debug_payload["session_id"] = Value::String(session_id.to_string());
        }
        if let Some(iteration) = self.iteration_index {
            debug_payload["iteration"] = Value::from(iteration as u64);
        }
        llm::log_local_debug_payload("tool_use", &debug_payload);
        if self.json {
            if self.next_trace.is_some() {
                self.json_line(json!({"protocol_version": 1, "type": "agent_trace"}));
            }
        } else {
            if is_internal_control_tool(&request.name) {
                return;
            }
            if matches!(
                request.name.as_str(),
                "propose_agent_plan" | "update_agent_todos"
            ) {
                return;
            }
            let is_read_activity = tool_request_is_read_activity(request);
            if !is_read_activity {
                self.flush_pending_tool_block();
            }
            self.last_tool_name = request.name.clone();
            let detail = quiet_tool_running_line(request).unwrap_or_else(|| {
                let localized_title;
                let title = if let Some(title) = tool_running_detail(&request.name) {
                    title
                } else {
                    localized_title = crate::i18n::localize_tool_display_name(&request.name);
                    localized_title.as_str()
                };
                tool_argument_summary(request)
                    .map(|args| format!("{title} · {args}"))
                    .unwrap_or_else(|| title.to_string())
            });
            // A sequence of reads/searches is one user-visible operation. Do
            // not tear down and recreate its live row for every individual
            // Core tool call: that produces a brief, distracting flash while
            // the completed receipts are intentionally waiting to be folded
            // into one compact history block.
            let activity_key = if self.quiet_status && is_read_activity {
                "tool|read-activity".to_string()
            } else {
                format!("tool|{detail}")
            };
            if self.last_quiet_status.as_deref() != Some(&activity_key)
                || self.activity_spinner.is_none()
            {
                self.last_quiet_status = Some(activity_key);
                self.start_activity_spinner(&detail);
            } else if let Some(spinner) = self.activity_spinner.as_ref() {
                // Consecutive reads share one stable row to avoid scrollback
                // flashes, but the row must still describe the operation that
                // is running now. Keeping the first tool's label for an entire
                // long turn made later user submissions look frozen.
                spinner.update_detail(&detail);
            }
        }
    }

    /// Cancellation is a lifecycle fact, not a failed process result. In
    /// particular, leaving a permission prompt may cancel an unstarted call.
    pub fn tool_cancelled(&mut self, request: &ToolRequest) {
        if self.json || is_internal_control_tool(&request.name) {
            return;
        }
        self.stop_activity_spinner();
        self.clear_transient_status_line();
        self.flush_stream_text(true);
        let args = if is_bash_command_tool(&request.name) {
            compact_bash_request_for_ui(request, agent_terminal_columns().saturating_sub(18))
        } else {
            tool_argument_summary(request)
        };
        self.push_pending_tool_item(
            request.name.clone(), args, Some(ui_text(UiText::Cancelled).to_string()), false, false,
        );
    }

    pub fn tool_result(&mut self, request: &ToolRequest, outcome: &ToolOutcome) {
        let continues_read_activity = !crate::terminal_ui::owns_terminal()
            && !self.json
            && self.quiet_status
            && !outcome.error
            && !is_internal_control_tool(&request.name)
            && tool_request_is_read_activity(request);
        if !continues_read_activity {
            self.stop_activity_spinner();
            self.clear_transient_status_line();
        }
        self.flush_stream_text(true);
        let mut debug_payload = tool_debug_result_payload(outcome);
        if let Some(session_id) = self.session_id.as_deref() {
            debug_payload["session_id"] = Value::String(session_id.to_string());
        }
        if let Some(iteration) = self.iteration_index {
            debug_payload["iteration"] = Value::from(iteration as u64);
        }
        llm::log_local_debug_payload("tool_result", &debug_payload);
        if self.json {
            if self.next_trace.is_none() {
                let event = tool_result_event_payload(request, outcome);
                self.next_trace = self
                    .trace_context
                    .as_ref()
                    .and_then(|context| context.lock().ok())
                    .and_then(|mut context| context.project_external_event(&event));
            }
            self.json_line(json!({"type": "agent_trace"}));
        } else if request.name == "propose_agent_plan" && !outcome.error {
            self.flush_pending_tool_block();
            self.clear_transient_status_line();
            self.clear_sticky_footer();
            self.ensure_output_line_for_status();
            for line in plan_card_lines(request) {
                emit_durable_terminal_line(&line);
            }
            self.output_ended_with_newline = true;
            self.sticky_footer_plan = plan_footer_summary_from_plan(request);
            self.sticky_footer_render_key = None;
            self.draw_sticky_footer();
            return;
        } else if request.name == "update_agent_todos" && !outcome.error {
            let next_plan = plan_footer_summary(request);
            let first_todo_view = self.sticky_footer_plan.is_none() && next_plan.is_some();
            let completion = if next_plan != self.sticky_footer_plan {
                plan_completion_summary(request)
            } else {
                None
            };
            if first_todo_view || completion.is_some() {
                self.flush_pending_tool_block();
                self.clear_transient_status_line();
                self.clear_sticky_footer();
                self.ensure_output_line_for_status();
                if first_todo_view {
                    for line in todo_card_lines(request) {
                        emit_durable_terminal_line(&line);
                    }
                }
                if let Some(completion) = completion {
                    if terminal_color_enabled() {
                        emit_durable_terminal_line(&format!("  \x1b[38;5;114m{completion}{CODE_COLOR_RESET}"));
                    } else {
                        emit_durable_terminal_line(&format!("  {completion}"));
                    }
                }
                self.output_ended_with_newline = true;
            }
            self.sticky_footer_plan = next_plan;
            self.sticky_footer_render_key = None;
            self.draw_sticky_footer();
            return;
        } else if self.quiet_status && !outcome.error {
            // Keep one compact receipt after the live spinner is cleared. The
            // canonical trace retains the full arguments and output; normal
            // mode renders only the action and resource summary.
            if is_internal_control_tool(&request.name) {
                return;
            }
            let tool_name = if !request.name.is_empty() {
                request.name.clone()
            } else {
                std::mem::take(&mut self.last_tool_name)
            };
            let args = if is_bash_command_tool(&tool_name) {
                compact_bash_request_for_ui(request, agent_terminal_columns().saturating_sub(18))
            } else {
                tool_argument_summary(request)
            };
            let detail = tool_done_detail(&tool_name).map(str::to_string);
            self.push_pending_tool_item(
                tool_name,
                args,
                detail,
                false,
                tool_request_is_read_activity(request),
            );
            // Keep the shared read/search activity row alive. The following
            // read reuses it; assistant text, an error, or a different tool
            // turns the pending receipts into stable compact history.
            if !continues_read_activity {
                self.draw_sticky_footer();
            }
            return;
        } else if outcome.error {
            if is_internal_control_tool(&request.name) {
                return;
            }
            let tool_name = if !request.name.is_empty() {
                request.name.clone()
            } else {
                std::mem::take(&mut self.last_tool_name)
            };
            let args = if is_bash_command_tool(&tool_name) {
                compact_bash_request_for_ui(request, agent_terminal_columns().saturating_sub(18))
            } else {
                tool_argument_summary(request)
            };
            let detail = tool_result_summary(request, outcome, true);
            self.push_pending_tool_item(tool_name, args, detail, true, false);
            if permission_recovery_hint(outcome).is_some() {
                self.flush_pending_tool_block();
                emit_durable_terminal_line(permission_recovery_hint(outcome).unwrap());
            }
        } else {
            if is_internal_control_tool(&request.name) {
                return;
            }
            let tool_name = if !request.name.is_empty() {
                request.name.clone()
            } else {
                std::mem::take(&mut self.last_tool_name)
            };
            let args = tool_argument_summary(request);
            let detail = tool_result_summary(request, outcome, false);
            self.push_pending_tool_item(
                tool_name,
                args,
                detail,
                false,
                tool_request_is_read_activity(request),
            );
        }
        // Interactive receipts have already settled at the transcript anchor.
        // Non-interactive presentation may still batch its compact summary.
    }

    fn push_pending_tool_item(
        &mut self,
        name: String,
        args: Option<String>,
        detail: Option<String>,
        is_error: bool,
        is_read_activity: bool,
    ) {
        let interactive_turn = (!self.json && crate::terminal_ui::owns_terminal())
            .then(|| {
                self.execution_cancellation
                    .as_ref()
                    .map(ExecutionCancellation::execution_id)
            })
            .flatten();
        let interactive = interactive_turn.is_some();
        if interactive && (!is_read_activity || is_error) {
            self.flush_pending_tool_block();
        }
        self.pending_tool_items.push(DisplayedToolItem {
            name,
            args,
            detail,
            is_error,
            is_read_activity,
        });
        if interactive {
            if is_read_activity && !is_error {
                // Completed reads form one semantic receipt, not one history
                // record per call. Update it in the main transcript immediately;
                // accepting the answer or changing action settles this same row.
                let block = Self::tool_history_block(
                    &self.pending_tool_items,
                    crate::terminal_ui::transcript_width(agent_terminal_columns()),
                    agent_terminal_rows(),
                );
                crate::terminal_ui::update_tool_receipt(interactive_turn.unwrap(), block);
            } else {
                self.flush_pending_tool_block();
            }
        }
    }

    fn flush_pending_tool_block(&mut self) {
        if self.pending_tool_items.is_empty() {
            return;
        }
        let items = std::mem::take(&mut self.pending_tool_items);
        if crate::terminal_ui::owns_terminal() {
            let block = Self::tool_history_block(
                &items,
                crate::terminal_ui::transcript_width(agent_terminal_columns()),
                agent_terminal_rows(),
            );
            // Remove the mutable receipt and commit it in one UI reduction,
            // before settling any activity row that follows it in the transcript.
            let read_receipt_owner = self
                .execution_cancellation
                .as_ref()
                .map(ExecutionCancellation::execution_id)
                .filter(|_| {
                    items
                        .iter()
                        .all(|item| item.is_read_activity && !item.is_error)
                });
            if let Some(turn_id) = read_receipt_owner {
                crate::terminal_ui::update_tool_receipt(turn_id, block);
                crate::terminal_ui::commit_tool_receipt(turn_id);
                crate::terminal_ui::clear_activity();
            } else {
                crate::terminal_ui::clear_activity();
                crate::terminal_ui::commit_history(&block);
            }
            self.draw_sticky_footer();
            return;
        }
        self.clear_transient_status_line();
        self.ensure_output_line_for_status();
        for line in
            Self::tool_history_lines(&items, agent_terminal_columns(), agent_terminal_rows())
        {
            println!("{line}");
        }
        self.output_ended_with_newline = true;
        self.draw_sticky_footer();
    }

    fn tool_history_block(items: &[DisplayedToolItem], width: usize, rows: usize) -> String {
        Self::tool_history_lines(items, width, rows).join("\n")
    }

    fn tool_history_lines(items: &[DisplayedToolItem], width: usize, _rows: usize) -> Vec<String> {
        let mut lines = Vec::new();
        let mut index = 0usize;
        while index < items.len() {
            let read_end = items[index..]
                .iter()
                .take_while(|item| !item.is_error && item.is_read_activity)
                .count()
                + index;
            // A sequence of reads/searches is one user-level inspection step.
            // Rendering each receipt separately makes long investigations look
            // like build logs and can bury the answer. Keep one stable summary;
            // the canonical transcript still owns every call and result.
            if read_end.saturating_sub(index) > 1 {
                let line = Self::collapsed_read_activity_line(&items[index..read_end], width);
                lines.push(if terminal_color_enabled() {
                    format!("{CODE_COLOR_COMMENT}{line}{CODE_COLOR_RESET}")
                } else {
                    line
                });
                index = read_end;
                continue;
            }
            let item = &items[index];
            let tool_name = if item.name.trim().is_empty() {
                "tool"
            } else {
                item.name.trim()
            };
            let result = item
                .detail
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .map(|detail| {
                    if item.is_error {
                        format!(" · {} · {detail}", tr(Message::AgentErrorLabel))
                    } else {
                        format!(" · {detail}")
                    }
                })
                .unwrap_or_default();
            let localized_name = crate::i18n::localize_tool_display_name(tool_name);
            let action = if is_bash_command_tool(tool_name) {
                item.args
                    .as_deref()
                    .and_then(bash_command_display_text)
                    .unwrap_or_else(|| localized_name.clone())
            } else if let Some(args) = item
                .args
                .as_deref()
                .filter(|value| !value.trim().is_empty())
            {
                format!("{localized_name}  {}", args.trim())
            } else {
                localized_name
            };
            // Reserve space for the result before shortening a long command.
            // Otherwise the very reason for a failure disappears off-screen.
            let result = truncate_display_value(&result, width.saturating_sub(12));
            let action_width = width.saturating_sub(display_width(&result));
            let line = format!("{}{result}", truncate_display_value(&format!("    • {action}"), action_width));
            lines.push(if terminal_color_enabled() {
                if item.is_error {
                    format!("\x1b[38;5;203m{line}{CODE_COLOR_RESET}")
                } else {
                    format!(
                        "{}{line}{}",
                        crate::INTERACTIVE_DIM,
                        crate::INTERACTIVE_RESET
                    )
                }
            } else {
                line
            });
            index += 1;
        }
        // The viewport owns clipping, not the transcript. Retain distinct
        // attempts and failures so scrolling back can explain what happened.
        lines
    }

    fn collapsed_read_activity_line(items: &[DisplayedToolItem], width: usize) -> String {
        let label = crate::i18n::localize_tool_display_name("read_file");
        let samples = items
            .iter()
            .filter_map(|item| {
                let args = item.args.as_deref()?;
                if is_bash_command_tool(&item.name) {
                    bash_command_display_text(args)
                } else {
                    Some(args.to_string())
                }
            })
            .take(3)
            .collect::<Vec<_>>()
            .join(" · ");
        let suffix = if samples.is_empty() {
            String::new()
        } else {
            format!(
                "  {}",
                truncate_display_value(&samples, width.saturating_sub(24))
            )
        };
        truncate_display_value(&format!("    • {label} ×{}{}", items.len(), suffix), width)
    }

    pub fn subagent_event(
        &mut self,
        parent_tool_id: &str,
        agent_type: &str,
        description: &str,
        stage: &str,
        detail: Option<&str>,
    ) {
        if self.quiet_status && !self.json {
            return;
        }
        self.stop_activity_spinner();
        self.clear_transient_status_line();
        self.flush_stream_text(true);
        if self.json {
            let mut payload = json!({
                "type": "subagent_event",
                "parent_tool_id": parent_tool_id,
                "agent_type": agent_type,
                "description": description,
                "stage": stage,
            });
            if let Some(detail) = detail.map(str::trim).filter(|value| !value.is_empty()) {
                payload["detail"] = Value::String(redact_sensitive_text(detail));
            }
            self.json_line(payload);
        } else {
            self.flush_pending_tool_block();
            let title = format!("{agent_type}: {description}");
            self.print_status_line(
                "agent",
                &title,
                Some(&format!(
                    "{}{}",
                    stage,
                    detail
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(|value| format!(" {value}"))
                        .unwrap_or_default()
                )),
            );
        }
    }

    pub fn done(&mut self, reason: &str) {
        self.compaction_active = false;
        self.review_active = false;
        let terminal_changed = self.terminal_reason.as_deref() != Some(reason);
        if terminal_changed {
            llm::log_local_debug_payload(
                "agent_terminal",
                &json!({
                    "type": "agent_terminal",
                    "reason": reason,
                }),
            );
        }
        self.terminal_reason = Some(reason.to_string());
        self.stop_activity_spinner();
        if !self.json && crate::terminal_ui::owns_terminal() {
            self.clear_transient_status_line();
            self.flush_stream_text(true);
            self.flush_pending_tool_block();
            if self.in_code_block {
                self.flush_code_buffer();
                self.in_code_block = false;
                self.code_block_lang = None;
            }
            crate::terminal_ui::finish_assistant();
            self.printed_text = false;
            self.clean_before_first_text = true;
            self.last_quiet_status = None;
            self.sticky_footer_suspended = false;
            self.draw_sticky_footer();
            return;
        }
        if self.json {
            // Native hosts consume the structured Core terminal event. JSON
            // mode keeps terminal output on that protocol instead of adding a
            // second CLI-only terminal record.
        } else {
            let preserve_execution_composer = self
                .execution_cancellation
                .as_ref()
                .is_some_and(|value| value.accepts_terminal_input());
            self.clear_transient_status_line();
            self.clear_sticky_footer();
            if !preserve_execution_composer {
                self.sticky_footer_base = None;
                self.sticky_footer_context = None;
                self.sticky_footer_plan = None;
            }
            let has_text = self.printed_text || !self.stream_text_buffer.is_empty();
            self.flush_stream_text(true);
            self.flush_pending_tool_block();
            if is_user_cancelled_error(reason)
                || reason.eq_ignore_ascii_case("cancelled")
                || reason.eq_ignore_ascii_case("returned_to_composer")
            {
                self.print_status_line(
                    "cancelled",
                    crate::i18n::interaction_text().question_cancelled,
                    None,
                );
            }
            if self.in_code_block {
                self.flush_code_buffer();
                print_code_fence_label(None);
                self.output_ended_with_newline = true;
                self.in_code_block = false;
                self.code_block_lang = None;
            }
            if has_text {
                if !crate::terminal_ui::owns_terminal() { println!(); }
                self.output_ended_with_newline = true;
                let _ = io::stdout().flush();
            }
            // One Core execution may atomically promote several queued user
            // inputs into consecutive Turns. Each Turn must close its own
            // renderer without making later text look like an already-ended
            // response or leaving a short Markdown tail buffered forever.
            self.printed_text = false;
            self.clean_before_first_text = true;
            self.last_quiet_status = None;
            self.sticky_footer_suspended = false;
            if preserve_execution_composer {
                self.draw_sticky_footer();
            } else {
                self.clear_sticky_footer();
            }
        }
    }

    #[allow(dead_code)]
    pub fn error(&mut self, message: &str) {
        let terminal_reason = if is_user_cancelled_error(message) {
            "cancelled"
        } else {
            "failed"
        };
        if self.terminal_reason.is_none() {
            llm::log_local_debug_payload(
                "agent_terminal",
                &json!({
                    "type": "agent_terminal",
                    "reason": terminal_reason,
                    "detail_bytes": message.len(),
                }),
            );
        }
        self.terminal_reason = Some(terminal_reason.to_string());
        self.stop_activity_spinner();
        if !self.json {
            self.clear_sticky_footer();
            self.sticky_footer_suspended = false;
            self.sticky_footer_base = None;
            self.sticky_footer_context = None;
            self.sticky_footer_plan = None;
        }
        self.clear_transient_status_line();
        self.flush_stream_text(true);
        if self.json {
            let event_type = if terminal_reason == "cancelled" {
                "cancelled"
            } else {
                "error"
            };
            let event = json!({"type": event_type, "detail": message, "reason": terminal_reason});
            if self.next_trace.is_none() && !self.canonical_trace_observed {
                self.next_trace = self
                    .trace_context
                    .as_ref()
                    .and_then(|context| context.lock().ok())
                    .and_then(|mut context| context.project_external_event(&event));
            }
            self.json_line(event);
        } else {
            self.flush_pending_tool_block();
            if is_user_cancelled_error(message) {
                self.print_status_line(
                    "cancelled",
                    tr(Message::InteractiveAllowPanelCancelled),
                    None,
                );
            } else {
                let label = format_error("").trim().to_string();
                self.print_status_line("error", &label, Some(message));
            }
            self.clear_sticky_footer();
        }
    }

    fn print_status_line(&mut self, kind: &str, title: &str, detail: Option<&str>) {
        if crate::terminal_ui::owns_terminal() {
            self.clear_transient_status_line();
            crate::terminal_ui::commit_history(&human_status_line(kind, title, detail));
            self.output_ended_with_newline = true;
            self.draw_sticky_footer();
            return;
        }
        self.clear_transient_status_line();
        self.ensure_output_line_for_status();
        println!("{}", human_status_line(kind, title, detail));
        self.output_ended_with_newline = true;
        self.draw_sticky_footer();
    }

    fn draw_transient_status_line(&mut self, detail: &str) {
        if self.json {
            return;
        }
        if crate::terminal_ui::owns_terminal() {
            crate::terminal_ui::set_activity("transient-status", "•", detail);
            self.transient_status_visible = true;
            self.output_ended_with_newline = true;
            return;
        }
        self.clear_transient_status_line();
        self.ensure_output_line_for_status();
        print!("{}", human_status_line("status", "", Some(detail)));
        self.transient_status_visible = true;
        self.output_ended_with_newline = false;
        let _ = io::stdout().flush();
        self.draw_sticky_footer();
    }

    fn clear_transient_status_line(&mut self) {
        if self.json || !self.transient_status_visible {
            return;
        }
        if crate::terminal_ui::owns_terminal() {
            crate::terminal_ui::clear_activity();
            self.transient_status_visible = false;
            self.output_ended_with_newline = true;
            return;
        }
        print!("{}", clear_current_output_line());
        self.transient_status_visible = false;
        self.output_ended_with_newline = true;
        let _ = io::stdout().flush();
    }

    fn ensure_output_line_for_status(&mut self) {
        // The compositor owns both transcript coordinates and the composer
        // cursor. A bare newline here scrolls its virtual tail without updating
        // either anchor, leaving an old preview as ghost transcript history.
        if crate::terminal_ui::owns_terminal() {
            return;
        }
        if self.printed_text && !self.output_ended_with_newline && !self.clean_before_first_text {
            println!();
            self.output_ended_with_newline = true;
        }
    }

    fn assistant_at_line_start(&self) -> bool {
        if crate::terminal_ui::owns_terminal() {
            crate::terminal_ui::assistant_at_line_start()
        } else {
            self.output_ended_with_newline
        }
    }

    fn track_rendered_markdown(&mut self, text: &str) {
        self.output_ended_with_newline = text.ends_with('\n');
    }

    fn json_line(&mut self, mut value: Value) {
        let trace = self.next_trace.take();
        if let Some(trace) = trace {
            if let Some(store) = self.tool_event_store.as_ref() {
                let _ = store.record_trace_event(&trace);
            }
            attach_agent_trace(&mut value, &trace);
        }
        if self.terminal_transport_deferred && is_terminal_transport_frame(&value) {
            self.deferred_terminal_transport.push(value);
            return;
        }
        self.emit_json_value(value);
    }

    fn emit_json_value(&mut self, value: Value) {
        if self.recovery_transport_suppressed {
            return;
        }
        self.emit_json_value_direct(value);
    }

    fn emit_json_value_direct(&mut self, value: Value) {
        if let Some(event_sink) = &mut self.event_sink {
            event_sink(value);
        } else if crate::tools::permission_trace_uses_direct_native_transport() {
            self.handle_native_delivery_result(crate::native_host_output::emit(value));
        } else {
            println!("{}", value);
            // Native clients consume one JSON object per line from a pipe.
            // Flush every protocol frame so a long model response cannot sit
            // in stdio buffering until the provider request has completed.
            let _ = io::stdout().flush();
        }
    }

    fn sticky_footer_text(&self) -> Option<String> {
        let usage = self
            .sticky_footer_context
            .as_ref()
            .map(format_footer_context_usage);
        let mut lines = self
            .sticky_footer_base
            .as_deref()
            .unwrap_or_default()
            .lines()
            .map(|line| {
                let trimmed = line.trim_start();
                trimmed
                    .strip_prefix("├─")
                    .or_else(|| trimmed.strip_prefix("╰─"))
                    .unwrap_or(trimmed)
                    .trim_start()
                    .to_string()
            })
            .collect::<Vec<_>>();
        if let Some(usage) = usage {
            merge_footer_context_usage(&mut lines, usage);
        }
        if let Some(plan) = self
            .sticky_footer_plan
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            lines.push(plan.to_string());
        }
        if let Some(cancellation) = self
            .execution_cancellation
            .as_ref()
            .filter(|value| value.accepts_terminal_input())
        {
            let snapshot = cancellation.input_snapshot();
            if !snapshot.draft.is_empty() {
                lines.push(format!(
                    "{} {}",
                    tr(Message::InteractiveExecutionDraftPrefix),
                    truncate_display_value(&snapshot.draft, 72)
                ));
            } else if snapshot.queued_count > 0 {
                lines.push(format!(
                    "{} · {} {}",
                    tr(Message::InteractiveInputPlaceholder),
                    tr(Message::InteractiveExecutionQueuedPrefix),
                    snapshot.queued_count,
                ));
            } else {
                lines.push(tr(Message::InteractiveInputPlaceholder).to_string());
            }
        }
        if lines.iter().all(|line| line.trim().is_empty()) {
            None
        } else {
            Some(lines.join("\n"))
        }
    }

    fn draw_sticky_footer(&mut self) {
        let has_execution_composer = self
            .execution_cancellation
            .as_ref()
            .is_some_and(|value| value.accepts_terminal_input());
        if self.json
            || (self.sticky_footer_base.is_none() && !has_execution_composer)
            || self.sticky_footer_suspended
            || !sticky_footer_terminal_enabled()
        {
            return;
        }
        if crate::terminal_ui::owns_terminal() {
            // Session facts belong to the persistent bottom surface, not to
            // this transient emitter. Update usage without replacing model,
            // permission and session information with an empty base.
            if let Some(usage) = self.sticky_footer_context.as_ref() {
                crate::terminal_ui::update_footer_context(format_footer_context_usage(usage));
            }
            if let Some(cancellation) = self.execution_cancellation.as_ref() {
                crate::terminal_ui::sync_composer(cancellation);
            }
            return;
        }
        let _render_guard = TERMINAL_RENDER_LOCK
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let Some(text) = self.sticky_footer_text() else {
            self.clear_sticky_footer();
            return;
        };
        let Ok((columns, rows)) = crossterm::terminal::size() else {
            return;
        };
        if rows == 0 || columns == 0 {
            return;
        }
        if self.sticky_footer_render_key.as_ref().is_some_and(
            |(rendered, rendered_columns, rendered_rows)| {
                rendered == &text
                    && *rendered_columns == columns
                    && *rendered_rows == rows
                    && self.sticky_footer_visible_lines > 0
            },
        ) {
            return;
        }
        let width = columns.saturating_sub(4).max(1) as usize;
        let terminal_line_budget = rows.saturating_sub(4).max(1) as usize;
        let max_lines = terminal_line_budget.min(MAX_STICKY_FOOTER_LINES);
        let lines = select_sticky_footer_lines(&text, width, max_lines);
        if lines.is_empty() {
            self.clear_sticky_footer();
            return;
        }
        let reserve_lines = lines
            .len()
            .saturating_sub(self.sticky_footer_reserved_lines);
        if reserve_lines > 0 {
            for _ in 0..reserve_lines {
                print!("\n");
            }
            print!("\x1b[{reserve_lines}A\r");
            self.sticky_footer_reserved_lines = self
                .sticky_footer_reserved_lines
                .saturating_add(reserve_lines);
        }
        let start_row = rows.saturating_sub(lines.len() as u16).saturating_add(1);
        let content_bottom_row = start_row.saturating_sub(1).max(1);
        print!("\x1b7\x1b[1;{content_bottom_row}r");
        self.clear_sticky_footer_rows(rows);
        for (index, line) in lines.iter().enumerate() {
            let row = start_row.saturating_add(index as u16);
            if has_execution_composer && index + 1 == lines.len() {
                if let Some(cancellation) = self.execution_cancellation.as_ref() {
                    let composer = execution_composer_bar(cancellation);
                    print!("\x1b[{row};1H{composer}");
                    continue;
                }
            }
            let branch = if lines.len() == 1 || index + 1 == lines.len() {
                "╰─"
            } else if index == 0 {
                "╭─"
            } else {
                "├─"
            };
            if terminal_color_enabled() {
                let content_color = if line.trim_start().starts_with('●') {
                    "\x1b[38;5;114m"
                } else if index == 0 {
                    "\x1b[38;5;250m"
                } else {
                    "\x1b[38;5;245m"
                };
                print!(
                    "\x1b[{row};1H\x1b[2K  \x1b[38;5;241m{branch} {content_color}{line}{CODE_COLOR_RESET}"
                );
            } else {
                print!("\x1b[{row};1H\x1b[2K  {branch} {line}");
            }
        }
        print!("\x1b8");
        self.sticky_footer_visible_lines = lines.len();
        self.sticky_footer_render_key = Some((text, columns, rows));
        let _ = io::stdout().flush();
    }

    fn prepare_sticky_footer_for_durable_output(&mut self) {
        if self.json {
            return;
        }
        // Restore the full terminal before durable output so terminal scrollback
        // keeps every assistant/tool line. The next render immediately restores
        // the same shared composer instead of disabling it for the whole turn.
        self.clear_sticky_footer();
        self.sticky_footer_suspended = false;
        self.sticky_footer_render_key = None;
    }

    fn clear_sticky_footer(&mut self) {
        self.sticky_footer_render_key = None;
        if crate::terminal_ui::owns_terminal() {
            // Durable output and emitter teardown never erase session facts.
            self.sticky_footer_visible_lines = 0;
            self.sticky_footer_reserved_lines = 0;
            return;
        }
        if self.json || !sticky_footer_terminal_enabled() {
            self.sticky_footer_visible_lines = 0;
            self.sticky_footer_reserved_lines = 0;
            return;
        }
        if self.sticky_footer_visible_lines == 0 && self.sticky_footer_reserved_lines == 0 {
            return;
        }
        let Ok((_, rows)) = crossterm::terminal::size() else {
            print!("\x1b[r");
            self.sticky_footer_visible_lines = 0;
            self.sticky_footer_reserved_lines = 0;
            let _ = io::stdout().flush();
            return;
        };
        if rows == 0 {
            print!("\x1b[r");
            self.sticky_footer_visible_lines = 0;
            self.sticky_footer_reserved_lines = 0;
            let _ = io::stdout().flush();
            return;
        }
        print!("\x1b7\x1b[r");
        self.clear_sticky_footer_rows(rows);
        print!("\x1b8");
        self.sticky_footer_visible_lines = 0;
        self.sticky_footer_reserved_lines = 0;
        let _ = io::stdout().flush();
    }

    fn clear_sticky_footer_rows(&self, rows: u16) {
        let lines = self.sticky_footer_visible_lines.min(rows as usize);
        if lines == 0 {
            return;
        }
        let start_row = rows.saturating_sub(lines as u16).saturating_add(1);
        for index in 0..lines {
            let row = start_row.saturating_add(index as u16);
            print!("\x1b[{row};1H\x1b[2K");
        }
    }

    fn enqueue_stream_text(&mut self, text: &str) {
        self.stream_text_buffer.push_str(text);
        self.flush_stream_text(false);
    }

    fn flush_stream_text(&mut self, force: bool) {
        loop {
            if self.stream_text_buffer.is_empty() {
                return;
            }
            if self.streaming_vertical_table.is_some() {
                let before = self.stream_text_buffer.len();
                self.flush_streaming_vertical_table_rows(force);
                if self.streaming_vertical_table.is_some() {
                    if force && self.stream_text_buffer.len() < before {
                        continue;
                    }
                    return;
                }
                if self.stream_text_buffer.is_empty() {
                    return;
                }
                continue;
            }
            if self.try_start_streaming_vertical_table() {
                continue;
            }

            let stable_len = if force {
                self.stream_text_buffer.len()
            } else if self.stream_text_buffer.len() < 2048 {
                stable_markdown_stream_flush_len(self.stream_text_buffer.as_str())
            } else {
                let stable = stable_markdown_stream_flush_len(self.stream_text_buffer.as_str());
                if stable > 0 {
                    stable
                } else if markdown_stream_tail_has_open_table(self.stream_text_buffer.as_str()) {
                    // A table must be rendered as one block unless it has
                    // already switched to the vertical streaming renderer.
                    // Flushing an arbitrary 96-character tail boundary here
                    // renders the first row and leaves later rows as raw pipes.
                    0
                } else {
                    keep_tail_start(self.stream_text_buffer.as_str(), 96)
                }
            };
            if stable_len == 0 {
                return;
            }
            let flush_len = markdown_stream_flush_len(self.stream_text_buffer.as_str(), stable_len);
            if flush_len == 0 {
                return;
            }
            let remaining = self.stream_text_buffer.split_off(flush_len);
            let emit = std::mem::take(&mut self.stream_text_buffer);
            if !emit.is_empty() {
                self.emit_markdown_text(&emit);
            }
            self.stream_text_buffer = remaining;
            if !force {
                return;
            }
        }
    }

    fn try_start_streaming_vertical_table(&mut self) -> bool {
        if self.streaming_vertical_table.is_some() {
            return false;
        }
        let Some((header, consumed)) =
            streaming_vertical_table_start(self.stream_text_buffer.as_str())
        else {
            return false;
        };
        let columns = header.len();
        self.stream_text_buffer.drain(..consumed);
        self.streaming_vertical_table = Some(StreamingVerticalTable {
            header,
            columns,
            emitted_rows: 0,
        });
        true
    }

    fn flush_streaming_vertical_table_rows(&mut self, force: bool) {
        let color_enabled = terminal_color_enabled();
        let mut emitted = false;
        let mut committed_rows = 0usize;
        while self.streaming_vertical_table.is_some() {
            let Some((raw_line, consumed, has_newline)) =
                next_stream_markdown_line(self.stream_text_buffer.as_str(), force)
            else {
                break;
            };
            if raw_line.trim().is_empty() {
                self.stream_text_buffer.drain(..consumed);
                self.streaming_vertical_table = None;
                self.emit_rendered_chunk("\n");
                self.output_ended_with_newline = true;
                emitted = true;
                continue;
            }
            let Some(state) = self.streaming_vertical_table.as_ref() else {
                break;
            };
            let Some(cells) = markdown_table_cells(raw_line)
                .and_then(|cells| normalize_markdown_table_row(cells, state.columns))
            else {
                self.streaming_vertical_table = None;
                break;
            };
            let header = state.header.clone();
            let include_separator = state.emitted_rows > 0;
            let rendered = render_markdown_table_vertical_row(
                &header,
                &cells,
                include_separator,
                color_enabled,
            );
            let rendered = indent_assistant_output(&rendered, self.assistant_at_line_start());
            let rendered = if has_newline {
                format!("{rendered}\n")
            } else {
                rendered
            };
            self.emit_rendered_chunk(&rendered);
            self.output_ended_with_newline = has_newline;
            if let Some(state) = self.streaming_vertical_table.as_mut() {
                state.emitted_rows += 1;
            }
            self.stream_text_buffer.drain(..consumed);
            emitted = true;
            committed_rows = committed_rows.saturating_add(1);
            if committed_rows >= STREAM_COMMIT_MAX_LINES && !self.stream_text_buffer.is_empty() {
                break;
            }
        }
        if force && self.stream_text_buffer.is_empty() {
            self.streaming_vertical_table = None;
        }
        if emitted {
            let _ = io::stdout().flush();
            self.draw_sticky_footer();
        }
    }

    fn emit_rendered_chunk(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }

        self.stop_activity_spinner();
        self.clear_transient_status_line();

        if crate::terminal_ui::owns_terminal() {
            if self.stream_text_is_preview {
                crate::terminal_ui::append_assistant_preview(text);
            } else {
                crate::terminal_ui::append_assistant(text);
            }
        } else {
            print!("{text}");
            let _ = io::stdout().flush();
        }
        self.track_rendered_markdown(text);
    }

    fn emit_code_fence_label(&mut self, lang: Option<&str>, opening: bool) {
        if crate::terminal_ui::owns_terminal() {
            let label = if opening {
                match lang {
                    Some(lang) if !lang.is_empty() => format!("┌─ {lang}\n"),
                    _ => "┌─ code\n".to_string(),
                }
            } else {
                "└────────\n".to_string()
            };
            let label = indent_assistant_output(&label, true);
            self.emit_rendered_chunk(&label);
        } else {
            print_code_fence_label(if opening { lang } else { None });
        }
        self.output_ended_with_newline = true;
    }

    fn emit_markdown_text(&mut self, text: &str) {
        if crate::terminal_ui::owns_terminal() && !self.in_code_block && !text.contains("```") {
            self.stop_activity_spinner();
            self.clear_transient_status_line();
            let rendered = render_markdown_text(text);
            let rendered = indent_assistant_output(&rendered, self.assistant_at_line_start());
            if self.stream_text_is_preview {
                crate::terminal_ui::append_assistant_preview(&rendered);
            } else {
                crate::terminal_ui::append_assistant(&rendered);
            }
            self.clean_before_first_text = false;
            self.track_rendered_markdown(&rendered);
            return;
        }
        if self.clean_before_first_text {
            if !crate::terminal_ui::owns_terminal() {
                print!("{}", clear_current_output_line());
            }
            self.clean_before_first_text = false;
            // Clearing leaves the cursor at the physical beginning of a new
            // output line; the assistant renderer owns indentation from here.
            self.output_ended_with_newline = true;
        }
        let mut remaining = text;
        while !remaining.is_empty() {
            if self.in_code_block {
                if let Some(end) = remaining.find("```") {
                    let (code, next) = remaining.split_at(end);
                    self.code_block_buffer.push_str(code);
                    self.flush_code_buffer();
                    self.emit_code_fence_label(None, false);
                    self.in_code_block = false;
                    self.code_block_lang = None;
                    remaining = &next[3..];
                } else {
                    self.code_block_buffer.push_str(remaining);
                    break;
                }
            } else if let Some(start) = remaining.find("```") {
                let (before, after) = remaining.split_at(start);
                let rendered = render_markdown_text(before);
                let rendered = indent_assistant_output(&rendered, self.assistant_at_line_start());
                self.emit_rendered_chunk(&rendered);
                remaining = &after[3..];
                self.flush_code_fence_line(true, remaining);
                self.in_code_block = true;
            } else {
                let rendered = render_markdown_text(remaining);
                let rendered = indent_assistant_output(&rendered, self.assistant_at_line_start());
                self.emit_rendered_chunk(&rendered);
                break;
            }
        }
        let _ = io::stdout().flush();
        self.draw_sticky_footer();
    }

    fn flush_code_fence_line(&mut self, opening: bool, after_marker: &str) {
        if let Some((lang, rest)) = after_marker.split_once('\n') {
            let normalized = normalize_code_lang(lang.trim());
            self.code_block_lang = if normalized.is_empty() {
                None
            } else {
                Some(normalized)
            };
            if opening {
                let lang = self.code_block_lang.clone();
                self.emit_code_fence_label(lang.as_deref(), true);
            } else {
                self.emit_code_fence_label(None, false);
            }
            if !rest.is_empty() {
                self.code_block_buffer.push_str(rest);
            }
        } else {
            if opening {
                self.code_block_lang = None;
                self.emit_code_fence_label(None, true);
                self.code_block_buffer.push_str(after_marker);
            } else {
                self.code_block_buffer.push_str(after_marker);
            }
        }
    }

    fn flush_code_buffer(&mut self) {
        if self.code_block_buffer.is_empty() {
            return;
        }
        let lang = self.code_block_lang.as_deref().unwrap_or("text");
        let mut emitted = String::new();
        for line in self.code_block_buffer.split('\n') {
            let line = line.trim_end_matches('\r');
            if terminal_color_enabled() {
                emitted.push_str(CODE_COLOR_FENCE);
                emitted.push_str("│ ");
                emitted.push_str(CODE_COLOR_RESET);
            } else {
                emitted.push_str("│ ");
            }
            emitted.push_str(&code_highlight_line(line, lang));
            emitted.push('\n');
        }
        if emitted.ends_with('\n') {
            emitted.pop();
        }
        let emitted = indent_assistant_output(&emitted, true);
        self.emit_rendered_chunk(&format!("{emitted}\n"));
        self.output_ended_with_newline = true;
        self.code_block_buffer.clear();
        self.draw_sticky_footer();
    }
}

fn indent_assistant_output(value: &str, mut at_line_start: bool) -> String {
    let mut output = String::with_capacity(value.len().saturating_add(16));
    for ch in value.chars() {
        if at_line_start && ch != '\n' && ch != '\r' {
            output.push_str("    ");
            at_line_start = false;
        }
        output.push(ch);
        if ch == '\n' {
            at_line_start = true;
        }
    }
    output
}

#[cfg(test)]
mod assistant_output_indent_tests {
    use super::indent_assistant_output;

    #[test]
    fn prefixes_each_rendered_assistant_line_with_four_spaces() {
        assert_eq!(
            indent_assistant_output("first\nsecond\n", true),
            "    first\n    second\n"
        );
    }

    #[test]
    fn streaming_continuation_does_not_repeat_indent_mid_line() {
        assert_eq!(
            indent_assistant_output(" continuation", false),
            " continuation"
        );
        assert_eq!(
            indent_assistant_output(" end\nnext", false),
            " end\n    next"
        );
    }

    #[test]
    fn blank_markdown_lines_remain_blank() {
        assert_eq!(
            indent_assistant_output("one\n\ntwo", true),
            "    one\n\n    two"
        );
    }
}

pub(crate) fn merge_footer_context_usage(lines: &mut Vec<String>, usage: String) {
    let Some(first) = lines.first_mut() else {
        lines.push(usage);
        return;
    };
    if first.trim().is_empty() {
        *first = usage;
        return;
    }

    let context_label = tr(Message::InteractiveContextLabel);
    if first.trim_start().starts_with(context_label) {
        let segments = first.split(" · ").collect::<Vec<_>>();
        let mut suffix_start = 1;
        if segments.get(suffix_start).is_some_and(|segment| {
            segment.contains('/') && segment.chars().any(|value| value.is_ascii_digit())
        }) {
            suffix_start += 1;
        }
        let suffix = segments[suffix_start..].join(" · ");
        *first = if suffix.is_empty() {
            usage
        } else {
            format!("{usage} · {suffix}")
        };
    } else {
        *first = format!("{usage} · {first}");
    }
}

#[cfg(test)]
fn context_usage_event(
    used_tokens: usize,
    context_window_tokens: usize,
    prompt_token_limit: usize,
    message_tokens: usize,
    tool_schema_tokens: usize,
    message_count: usize,
    tool_count: usize,
) -> Value {
    context_usage_event_with_breakdown(
        used_tokens,
        context_window_tokens,
        prompt_token_limit,
        message_tokens,
        tool_schema_tokens,
        message_count,
        tool_count,
        0,
        0,
        message_tokens,
        &Value::Array(Vec::new()),
    )
}

#[allow(clippy::too_many_arguments)]
fn context_usage_event_with_breakdown(
    used_tokens: usize,
    context_window_tokens: usize,
    prompt_token_limit: usize,
    message_tokens: usize,
    tool_schema_tokens: usize,
    message_count: usize,
    tool_count: usize,
    component_tokens: usize,
    tool_result_tokens: usize,
    conversation_tokens: usize,
    component_metrics: &Value,
) -> Value {
    json!({
        "type": "context_usage",
        "used_tokens": used_tokens,
        "limit_tokens": context_window_tokens,
        "context_window_tokens": context_window_tokens,
        "prompt_token_limit": prompt_token_limit,
        "reserved_output_tokens": context_window_tokens.saturating_sub(prompt_token_limit),
        "message_tokens": message_tokens,
        "tool_schema_tokens": tool_schema_tokens,
        "component_tokens": component_tokens,
        "tool_result_tokens": tool_result_tokens,
        "conversation_tokens": conversation_tokens,
        "component_metrics": component_metrics,
        "message_count": message_count,
        "tool_count": tool_count,
        "usage_percent": used_tokens as f64 * 100.0 / context_window_tokens.max(1) as f64,
    })
}

fn context_usage_event_without_breakdown(
    used_tokens: usize,
    context_window_tokens: usize,
    prompt_token_limit: usize,
    tool_count: usize,
) -> Value {
    json!({
        "type": "context_usage",
        "used_tokens": used_tokens,
        "limit_tokens": context_window_tokens,
        "context_window_tokens": context_window_tokens,
        "prompt_token_limit": prompt_token_limit,
        "reserved_output_tokens": context_window_tokens.saturating_sub(prompt_token_limit),
        "tool_count": tool_count,
        "tool_count_scope": "host_catalog",
        "breakdown_status": "unavailable",
        "usage_percent": used_tokens as f64 * 100.0 / context_window_tokens.max(1) as f64,
    })
}

impl Drop for Emitter {
    fn drop(&mut self) {
        self.stop_activity_spinner();
        self.clear_transient_status_line();
        if !self
            .execution_cancellation
            .as_ref()
            .is_some_and(|value| value.accepts_terminal_input())
        {
            self.clear_sticky_footer();
        }
    }
}

fn sticky_footer_terminal_enabled() -> bool {
    io::stdout().is_terminal()
        && std::env::var("TERM")
            .map(|value| value != "dumb")
            .unwrap_or(true)
}

const MAX_STICKY_FOOTER_LINES: usize = 5;

fn select_sticky_footer_lines(text: &str, width: usize, max_lines: usize) -> Vec<String> {
    if max_lines == 0 {
        return Vec::new();
    }
    let all = text.lines().collect::<Vec<_>>();
    if all.len() <= max_lines {
        return all
            .into_iter()
            .map(|line| truncate_display_value(line, width))
            .collect();
    }
    if max_lines == 1 {
        return vec![truncate_display_value(all[0], width)];
    }

    // The first rows contain context/model state. The last rows carry the
    // active plan and execution-input hint, so preserve both ends instead of
    // dropping the actionable tail on narrow terminals.
    let tail_count = (max_lines / 2).max(1);
    let head_count = max_lines.saturating_sub(tail_count);
    all.iter()
        .take(head_count)
        .chain(all.iter().skip(all.len().saturating_sub(tail_count)))
        .map(|line| truncate_display_value(line, width))
        .collect()
}

fn format_footer_context_usage(
    usage: &nola_core::agent::context_window::ContextUsageSnapshot,
) -> String {
    format_context_usage_footer(usage.used_tokens, usage.context_window_tokens)
}

pub(crate) fn format_context_usage_footer(
    used_tokens: usize,
    context_window_tokens: usize,
) -> String {
    if context_window_tokens == 0 {
        return format!(
            "{} ~{} {}",
            tr(Message::InteractiveContextLabel),
            format_footer_token_count(used_tokens),
            tr(Message::InteractiveTokensUnit)
        );
    }
    let percent = used_tokens
        .saturating_mul(100)
        .saturating_add(context_window_tokens.saturating_sub(1))
        / context_window_tokens;
    format!(
        "{} {}% · {}/{}",
        tr(Message::InteractiveContextLabel),
        percent.min(999),
        format_footer_token_count(used_tokens),
        format_footer_token_count(context_window_tokens)
    )
}

fn format_footer_token_count(tokens: usize) -> String {
    if tokens >= 1_000_000 {
        format!("{:.1}m", tokens as f64 / 1_000_000.0)
    } else if tokens >= 10_000 {
        format!("{:.0}k", tokens as f64 / 1_000.0)
    } else if tokens >= 1_000 {
        format!("{:.1}k", tokens as f64 / 1_000.0)
    } else {
        tokens.to_string()
    }
}

fn should_show_quiet_status(phase: &str, message: &str) -> Option<String> {
    match phase {
        "prepare" => {
            let _ = message;
            None
        }
        "decide" if message == "已停止" => Some(message.to_string()),
        "decide" => Some(agent_localized_text(Message::AgentStatusThinking).to_string()),
        "plan" => {
            if message == "model returned plain answer path"
                || message == "model returned plain answer path with no visible response text"
            {
                return Some(
                    agent_localized_text(Message::AgentStatusReplyingDirectly).to_string(),
                );
            }
            if let Some(rest) = message.strip_prefix("model requested ") {
                let count = rest
                    .split_whitespace()
                    .next()
                    .and_then(|value| value.parse::<usize>().ok())
                    .unwrap_or(0);
                if count > 0 {
                    return Some(agent_prepare_actions(count));
                }
            }
            None
        }
        _ => None,
    }
}
fn is_chinese_language() -> bool {
    matches!(current_language(), Language::ZhHans)
}

fn agent_localized_text(message: Message) -> &'static str {
    tr(message)
}
fn agent_prepare_actions(count: usize) -> String {
    let action_prefix = tr(Message::AgentPreparingToExecutePrefix);
    if is_chinese_language() {
        let action_suffix = tr(Message::AgentActionCountSuffixPlural);
        format!("{action_prefix} {count} {action_suffix}")
    } else if count == 1 {
        format!(
            "{action_prefix} 1 {}",
            tr(Message::AgentActionCountSuffixSingle)
        )
    } else {
        format!(
            "{action_prefix} {count} {}",
            tr(Message::AgentActionCountSuffixPlural)
        )
    }
}
fn is_internal_control_tool(name: &str) -> bool {
    matches!(
        name,
        "agent_task_status"
            | "set_thinking_mode"
            | "goal_complete"
            | "request_user_input"
            | "tool_output_list"
            | "tool_output_read"
            | "tool_output_search"
            | "tool_output_find"
            | "list_tasks"
            | "read_task"
            | "wait_task"
    )
}

fn tool_item_is_read_activity(name: &str) -> bool {
    matches!(
        name,
        "file_outline"
            | "list_dir"
            | "read_file"
            | "read_many_files"
            | "grep"
            | "glob"
            | "find_files"
            | "repo_map"
            | "project_overview"
            | "context_pack"
            | "instruction_context"
            | "agent_context"
            | "coding_conventions"
            | "workspace_status"
            | "related_files"
            | "file_dependencies"
            | "symbol_search"
            | "implementation_search"
            | "workspace_symbols"
            | "file_tree"
            | "read_symbol"
            | "code_navigation"
            | "impact_analysis"
            | "call_graph"
    )
}

fn tool_request_is_read_activity(request: &ToolRequest) -> bool {
    tool_item_is_read_activity(&request.name)
        || (is_bash_command_tool(&request.name)
            && request
                .args
                .get("operation_kind")
                .and_then(Value::as_str)
                .is_some_and(|kind| kind == "inspect"))
}

fn should_flush_tool_history_after_result(request: &ToolRequest, is_error: bool) -> bool {
    let _ = (request, is_error);
    // Results, including recoverable failures, remain in the current semantic
    // block. The live row owns the current operation; assistant text, a prompt,
    // or the turn boundary commits one deduplicated receipt. Treating every
    // failed retry as a boundary creates a permanent wall of near-identical
    // shell commands and makes the transcript look like a build log.
    false
}

fn tool_running_detail(name: &str) -> Option<&'static str> {
    match name {
        "file_outline"
        | "list_dir"
        | "read_file"
        | "read_many_files"
        | "grep"
        | "glob"
        | "find_files"
        | "repo_map"
        | "project_overview"
        | "context_pack"
        | "instruction_context"
        | "agent_context"
        | "coding_conventions"
        | "workspace_status"
        | "related_files"
        | "file_dependencies"
        | "symbol_search"
        | "implementation_search"
        | "workspace_symbols"
        | "file_tree"
        | "read_symbol"
        | "code_navigation"
        | "impact_analysis"
        | "call_graph" => Some(agent_localized_text(Message::AgentInspectingProject)),
        "edit_file" | "edit_file_lines" | "multi_edit_file" | "write_file" | "append_file"
        | "notebook_edit" => Some(agent_localized_text(Message::AgentEditingFiles)),
        "bash"
        | "bash_command_review"
        | "bash_background_status"
        | "bash_background_wait"
        | "bash_background_stop" => Some(agent_localized_text(Message::AgentRunningCommand)),
        "web_search" | "web_fetch" | "chrome_native_open_page" | "chrome_native_click" => {
            Some(agent_localized_text(Message::AgentAccessingNetwork))
        }
        "tool_search" => Some(agent_localized_text(Message::AgentSearchingTools)),
        "spawn_agent" => Some(agent_localized_text(Message::AgentStartingSubtask)),
        _ => None,
    }
}

fn quiet_tool_running_line(request: &ToolRequest) -> Option<String> {
    let name = request.name.trim();
    if name.is_empty() {
        return None;
    }
    let action = crate::i18n::localize_tool_display_name(name);
    let args = tool_argument_summary(request);
    if is_bash_command_tool(name) {
        return Some(
            compact_bash_request_for_ui(request, agent_terminal_columns().saturating_sub(18))
                .map(|command| format!("{action} · {command}"))
                .unwrap_or(action),
        );
    }
    Some(match args {
        Some(args) if !args.trim().is_empty() => format!("{action} · {args}"),
        _ => action,
    })
}

fn is_bash_command_tool(name: &str) -> bool {
    matches!(name, "bash" | "bash_command_review")
}

fn bash_command_display_text(args: &str) -> Option<String> {
    let value = args.trim();
    let mut command = value.strip_prefix("cmd=").unwrap_or(value).trim();
    if let Some((prefix, remainder)) = command.split_once("&&") {
        if prefix.trim_start().starts_with("cd ") {
            command = remainder.trim_start();
        }
    }
    command = command
        .strip_suffix("2>&1")
        .map(str::trim_end)
        .unwrap_or(command);
    command = command.strip_prefix("$ ").unwrap_or(command).trim();
    (!command.is_empty()).then(|| command.to_string())
}

fn format_tool_activity_line(name: &str, args: Option<&str>, result: &str) -> String {
    format_tool_activity_line_for_width(
        name,
        args,
        result,
        agent_terminal_columns().saturating_sub(1).max(24),
    )
}

fn format_tool_activity_line_for_width(
    name: &str,
    args: Option<&str>,
    result: &str,
    max_width: usize,
) -> String {
    let raw_name = if name.trim().is_empty() {
        "tool"
    } else {
        name.trim()
    };
    let name = crate::i18n::localize_tool_display_name(raw_name);
    let args = args
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| {
            let is_compact_line_range = value
                .chars()
                .all(|character| character.is_ascii_digit() || matches!(character, '-' | '+'))
                && value
                    .chars()
                    .any(|character| matches!(character, '-' | '+'));
            if matches!(raw_name, "read_file" | "file_outline" | "read_symbol")
                && !value.contains('=')
                && is_compact_line_range
            {
                format!("lines={value}")
            } else {
                value.to_string()
            }
        });
    let prefix = if is_bash_command_tool(raw_name) {
        if let Some(command) = args.as_deref().and_then(bash_command_display_text) {
            format!("  › {command}")
        } else {
            format!("  › {name}")
        }
    } else {
        match args.as_deref() {
            Some(args) => format!("  › {name}  {args}"),
            None => format!("  › {name}"),
        }
    };
    let max_width = max_width.max(24);
    if result.is_empty() {
        return truncate_display_value(&prefix, max_width);
    }
    let result_budget = display_width(result)
        .min(max_width.saturating_sub(12))
        .max(1);
    let visible_result = truncate_display_value(result, result_budget);
    let prefix_budget = max_width.saturating_sub(display_width(&visible_result));
    format!(
        "{}{}",
        truncate_display_value(&prefix, prefix_budget),
        visible_result
    )
}

#[cfg(test)]
mod tool_activity_line_width_tests {
    use super::*;

    #[test]
    fn document_table_budget_includes_assistant_gutter() {
        let source = "| Field | Value |\n| --- | --- |\n| session | a reasonably long value |\n| state | waiting for approval |\n";
        for width in [40, 60, 80] {
            let rendered = render_markdown_document(source, width);
            for line in rendered.lines() {
                assert!(display_width(&strip_ansi_sequences(line)) <= width, "{width}: {line}");
            }
        }
    }

    #[test]
    fn cancelled_tool_is_not_a_failed_execution() {
        let mut emitter = Emitter::new(false);
        let request = ToolRequest { id: "unstarted".into(), name: "bash".into(),
            args: Map::from_iter([("command".into(), json!("ls -la"))]) };
        emitter.tool_cancelled(&request);
        assert_eq!(emitter.pending_tool_items.len(), 1);
        let item = &emitter.pending_tool_items[0];
        assert!(!item.is_error);
        assert!(!item.is_read_activity);
        assert_eq!(item.detail.as_deref(), Some(ui_text(UiText::Cancelled)));
    }

    #[test]
    fn transcript_keeps_each_failure_and_preserves_reason_at_narrow_width() {
        let items = (0..4).map(|index| DisplayedToolItem {
            name: "bash".into(), args: Some("very-long-command-name-with-many-arguments".into()),
            detail: Some(format!("failure-{index}")), is_error: true, is_read_activity: false,
        }).collect::<Vec<_>>();
        let lines = Emitter::tool_history_lines(&items, 42, 10);
        assert_eq!(lines.len(), 4, "viewport height must not erase history");
        for (index, line) in lines.iter().enumerate() {
            assert!(line.contains(&format!("failure-{index}")), "{line}");
            assert!(display_width(&strip_ansi_sequences(line)) <= 42);
        }
    }

    #[test]
    fn tool_error_prefers_concrete_diagnostic_over_generic_display() {
        let request = ToolRequest { id: "failed".into(), name: "bash".into(), args: Map::new() };
        for text in ["working directory does not exist", r#"{"error":{"message":"working directory does not exist"}}"#,
            r#"{"stderr":"working directory does not exist"}"#] {
            let outcome = ToolOutcome { full_output_ref: None, data: None, execution: None,
                text: text.into(), display: "command failed".into(), error: true,
                artifacts: Vec::new(), activation: None };
            assert_eq!(tool_result_summary(&request, &outcome, true).as_deref(), Some("working directory does not exist"));
        }
        let mut outcome = ToolOutcome { full_output_ref: None,
            data: Some(json!({"type":"shell_execution", "stderr":"specific failure", "exit_code":2})),
            execution: None, text: "other output".into(), display: "command failed".into(), error: true,
            artifacts: Vec::new(), activation: None };
        assert_eq!(tool_result_summary(&request, &outcome, true).as_deref(), Some("specific failure"));
        outcome.data = Some(json!({"type":"shell_execution", "stderr":"", "exit_code":2}));
        assert_eq!(tool_result_summary(&request, &outcome, true), Some(format!("{} (2)", ui_text(UiText::ExitCode))));
    }

    #[test]
    fn cli_polish_permission_recovery_is_actionable_and_does_not_classify_shell_stderr() {
        let mut outcome = ToolOutcome { full_output_ref: None, data: None, execution: None,
            text: "Permission denied".into(), display: "Tool failed".into(), error: true,
            artifacts: vec![], activation: None };
        assert!(permission_recovery_hint(&outcome).unwrap().contains("/allow"));
        assert!(permission_recovery_hint(&outcome).unwrap().contains("deny"));
        outcome.text = "compiler stderr says Permission denied".into();
        assert!(permission_recovery_hint(&outcome).is_none());
        outcome.data = Some(json!({"code":"permission_denied","tool_execution_skipped":true}));
        assert!(permission_recovery_hint(&outcome).is_some());
        let projected = crate::core_agent_runtime::tool_part_outcome(None, None, Some(&json!("Permission denied")), true);
        assert!(permission_recovery_hint(&projected).unwrap().contains("/allow"));
        outcome.error = false;
        assert!(permission_recovery_hint(&outcome).is_none());
    }

    #[test]
    fn review_stage_survives_attempts_and_releases_on_generation_or_completion() {
        let mut emitter = Emitter::new(true);
        emitter.status("reviewing", "Reviewing answer");
        emitter.status("model_attempt_started", "");
        assert!(emitter.review_active);
        emitter.status("processing", "");
        assert!(!emitter.review_active);
        emitter.status("reviewing", "Reviewing answer");
        emitter.done("answered");
        assert!(!emitter.review_active);
    }

    #[test]
    fn nested_model_attempt_does_not_release_compaction_stage() {
        let mut emitter = Emitter::new(true);
        emitter.status("compacting", "Compacting context");
        assert!(emitter.compaction_active);
        emitter.status("model_attempt_started", "");
        assert!(emitter.compaction_active);
        emitter.status("working", "validation snapshot");
        assert!(emitter.compaction_active);
        emitter.status("processing", "");
        assert!(!emitter.compaction_active);
        emitter.status("model_attempt_started", "");
        assert!(!emitter.compaction_active);
    }

    #[test]
    fn activity_clock_reports_elapsed_without_estimating_completion() {
        for (seconds, expected) in [(0, "0s"), (59, "59s"), (60, "1m 00s"), (3671, "61m 11s")] {
            assert_eq!(
                timed_activity_detail("stage", std::time::Duration::from_secs(seconds)),
                format!("{expected} · stage")
            );
        }
    }

    #[test]
    fn completed_tool_line_fits_narrow_terminal_and_keeps_result() {
        let line = format_tool_activity_line_for_width(
            "custom_tool",
            Some("path=/a/very/long/project/location/with/a/large/source-file.swift"),
            " · completed",
            48,
        );
        assert!(display_width(&line) <= 48, "{line}");
        assert!(line.contains("completed"), "{line}");
        assert_eq!(line.matches("completed").count(), 1, "{line}");
        assert!(line.trim_start().starts_with('›'), "{line}");
    }

    #[test]
    fn completed_tool_line_counts_wide_characters_by_terminal_columns() {
        let line = format_tool_activity_line_for_width(
            "custom_tool",
            Some("path=项目/很长的中文文件名称.md"),
            " · 已完成",
            32,
        );
        assert!(display_width(&line) <= 32, "{line}");
        assert!(line.contains("已完成"), "{line}");
    }

    #[test]
    fn bash_activity_shows_the_command_after_a_leading_cd_within_available_width() {
        let request = ToolRequest {
            id: "call-bash".to_string(),
            name: "bash".to_string(),
            args: Map::from_iter([(
                "command".to_string(),
                Value::String(
                    "cd /Users/example/project && git check-ignore cache tmp artifact && echo done"
                        .to_string(),
                ),
            )]),
        };
        let summary = compact_bash_request_for_ui(&request, 42).expect("bash summary");
        assert!(summary.contains("git check-ignore"), "{summary}");
        assert!(!summary.contains("/Users/example/project"), "{summary}");
        assert!(display_width(&summary) <= 42, "{summary}");
    }

    #[test]
    fn consecutive_reads_collapse_into_one_bounded_history_line() {
        let items = (0..12)
            .map(|index| DisplayedToolItem {
                name: "read_file".to_string(),
                args: Some(format!("path=/tmp/source-{index}.rs")),
                detail: None,
                is_error: false,
                is_read_activity: true,
            })
            .collect::<Vec<_>>();
        let line = Emitter::collapsed_read_activity_line(&items, 56);
        assert!(line.contains("×12"), "{line}");
        assert!(line.contains("source-0.rs"), "{line}");
        assert!(display_width(&line) <= 56, "{line}");
        assert_eq!(line.lines().count(), 1, "{line}");
    }

    #[test]
    fn fifty_completed_reads_use_one_actual_terminal_width_receipt() {
        let items = (0..50)
            .map(|index| DisplayedToolItem {
                name: "read_file".into(),
                args: Some(format!(
                    "path=/a/long/project/directory/fixture-{index:02}.txt"
                )),
                detail: None,
                is_error: false,
                is_read_activity: true,
            })
            .collect::<Vec<_>>();
        let width = crate::terminal_ui::transcript_width(60);
        let lines = Emitter::tool_history_lines(&items, width, 24);
        assert_eq!(width, 58);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(lines[0].contains("×50"), "{lines:?}");
        assert!(display_width(strip_ansi_sequences(&lines[0]).as_str()) <= width);
    }

    #[test]
    fn bash_summary_keeps_relative_scope_and_hides_redirect_plumbing() {
        let cwd = std::env::current_dir()
            .expect("current directory")
            .join("cli")
            .to_string_lossy()
            .to_string();
        let request = ToolRequest {
            id: "call-metadata".to_string(),
            name: "bash".to_string(),
            args: Map::from_iter([
                (
                    "command".to_string(),
                    Value::String(
                        "cargo metadata --no-deps 2>&1 | python3 -c 'print(1)'".to_string(),
                    ),
                ),
                ("cwd".to_string(), Value::String(cwd)),
            ]),
        };
        let summary = compact_bash_request_for_ui(&request, 90).expect("bash summary");
        assert!(summary.starts_with("cargo metadata"), "{summary}");
        assert!(summary.contains("cli/"), "{summary}");
        assert!(!summary.contains("2>&1"), "{summary}");
        assert!(!summary.contains("/Users/"), "{summary}");
    }

    #[test]
    fn collapsed_inspection_shell_history_shows_commands_not_bash_or_cmd_prefixes() {
        let items = (0..14)
            .map(|index| DisplayedToolItem {
                name: "bash".to_string(),
                args: Some(format!("cmd=git show --stat revision-{index}")),
                detail: None,
                is_error: false,
                is_read_activity: true,
            })
            .collect::<Vec<_>>();
        let line = Emitter::collapsed_read_activity_line(&items, 72);
        assert!(line.contains("git show --stat revision-0"), "{line}");
        assert!(!line.contains("cmd="), "{line}");
        assert!(!line.contains("bash"), "{line}");
        assert!(line.contains("×14"), "{line}");
        assert!(display_width(&line) <= 72, "{line}");
    }

    #[test]
    fn read_results_stay_grouped_until_a_real_history_boundary() {
        let request = |name: &str, operation_kind: Option<&str>| ToolRequest {
            id: format!("call-{name}"),
            name: name.to_string(),
            args: operation_kind
                .map(|kind| {
                    Map::from_iter([(
                        "operation_kind".to_string(),
                        Value::String(kind.to_string()),
                    )])
                })
                .unwrap_or_default(),
        };
        assert!(!should_flush_tool_history_after_result(
            &request("read_file", None),
            false
        ));
        assert!(!should_flush_tool_history_after_result(
            &request("grep", None),
            false
        ));
        assert!(!should_flush_tool_history_after_result(
            &request("bash", Some("inspect")),
            false
        ));
        assert!(!should_flush_tool_history_after_result(
            &request("bash", Some("mutate")),
            false
        ));
        assert!(!should_flush_tool_history_after_result(
            &request("read_file", None),
            true
        ));
    }
}

fn tool_done_detail(name: &str) -> Option<&'static str> {
    match name {
        "edit_file" | "edit_file_lines" | "multi_edit_file" | "write_file" | "append_file"
        | "notebook_edit" => Some(agent_localized_text(Message::AgentFileModified)),
        _ => None,
    }
}

fn tool_error_detail(name: &str) -> Option<&'static str> {
    match name {
        "file_outline"
        | "list_dir"
        | "read_file"
        | "read_many_files"
        | "grep"
        | "glob"
        | "find_files"
        | "repo_map"
        | "project_overview"
        | "context_pack"
        | "instruction_context"
        | "agent_context"
        | "coding_conventions"
        | "workspace_status"
        | "related_files"
        | "file_dependencies"
        | "symbol_search"
        | "implementation_search"
        | "workspace_symbols"
        | "file_tree"
        | "read_symbol"
        | "code_navigation"
        | "impact_analysis"
        | "call_graph"
        | "tool_search" => None,
        "edit_file" | "edit_file_lines" | "multi_edit_file" | "write_file" | "append_file"
        | "notebook_edit" => Some(agent_localized_text(Message::AgentFailedToModifyFile)),
        "bash"
        | "bash_command_review"
        | "bash_background_status"
        | "bash_background_wait"
        | "bash_background_stop" => Some(agent_localized_text(Message::AgentCommandFailed)),
        "web_search" | "web_fetch" | "chrome_native_open_page" | "chrome_native_click" => {
            Some(agent_localized_text(Message::AgentNetworkRequestFailed))
        }
        "spawn_agent" => Some(agent_localized_text(Message::AgentSubtaskFailed)),
        _ => Some(agent_localized_text(Message::AgentOperationFailed)),
    }
}
