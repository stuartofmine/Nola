// SPDX-License-Identifier: Apache-2.0

use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use nola_application::{ApplicationCommand, TraceRecord, TraceRun};
use nola_core::agent_trace::{
    AgentTraceEvent, AgentTraceKind, AgentTracePayload, AgentTraceStatus, AgentTraceVisibility,
};
use nola_service_client::ApplicationServiceClient;

fn start_service(database: &std::path::Path, socket: &std::path::Path) -> Child {
    Command::new(env!("CARGO_BIN_EXE_nola-service"))
        .args(["--database"])
        .arg(database)
        .args(["--socket"])
        .arg(socket)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start real Application Service")
}

fn ready(client: &ApplicationServiceClient, child: &mut Child) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if client.health().is_ok() {
            return;
        }
        assert!(child.try_wait().expect("inspect service").is_none());
        thread::sleep(Duration::from_millis(20));
    }
    panic!("Application Service did not become ready");
}

fn event(sequence: u64, kind: AgentTraceKind, status: AgentTraceStatus) -> AgentTraceEvent {
    AgentTraceEvent {
        schema_version: 1,
        trace_id: "trace-cold".into(),
        run_id: "run-cold".into(),
        conversation_id: "conversation-cold".into(),
        message_id: Some("message-cold".into()),
        turn_id: "run-cold".into(),
        event_id: format!("event-{sequence}"),
        sequence,
        emitted_at: format!("2026-01-01T00:00:0{sequence}Z"),
        kind,
        span_id: None,
        parent_span_id: None,
        status,
        visibility: AgentTraceVisibility::Summary,
        payload: AgentTracePayload::default(),
    }
}

#[test]
fn trace_events_and_envelopes_survive_real_service_restarts() {
    let directory = tempfile::tempdir().expect("temporary state");
    let database = directory.path().join("nola.db");
    let socket = directory.path().join("nola.sock");
    let mut store = nola_store::NolaStore::open(&database).unwrap();
    let message = String::from("message-cold");
    let part = String::from("assistant-part-cold");
    let input = nola_store::BeginTurnInput {
        user_part_kind: "text".into(),
        expected_configuration_revision: None,
        expected_parent_message_id: None,
        initial_events: Vec::new(),
        conversation_id: "conversation-cold".into(),
        conversation_title: "Journal planning".into(),
        user_message_id: "user-cold".into(),
        user_part_id: "user-part-cold".into(),
        user_text: "继续 / continue".into(),
        user_content_json: None,
        additional_user_parts: vec![],
        assistant_message_id: message.clone(),
        assistant_part_id: part.clone(),
        initial_assistant_part_mutations: vec![],
        assistant_turn_bindings: vec![
            nola_domain::AssistantTurnBinding::Admission {
                turn_id: "run-cold".into(),
                assistant_message_id: message.clone(),
            },
            nola_domain::AssistantTurnBinding::Part { turn_id: "run-cold".into(), assistant_message_id: message.clone(), part_id: part.clone() },
        ],
        run_id: "run-cold".into(),
        context_checkpoint_id: None,
        checkpoint_revision: 1,
        checkpoint_event_cursor: 0,
        checkpoint_json: "{}".into(),
        invocation_config_json: "{}".into(),
        invocation_config_digest: "fixture".into(),
        outbox_effects: vec![],
        model_attempt_mutations: vec![],
        delegated_invocation_mutations: vec![],
        delegated_permission_mutations: vec![],
        memory_usage_entries: vec![],
        created_at_ms: 1,
    };
    store.begin_turn(&input).unwrap();
    drop(store);
    let client = ApplicationServiceClient::with_socket(&database, &socket);
    let mut first = start_service(&database, &socket);
    ready(&client, &mut first);
    let inserted: bool = client
        .execute(ApplicationCommand::RecordAgentTrace {
            event: event(1, AgentTraceKind::RunStarted, AgentTraceStatus::Running),
            envelope: Some(serde_json::json!({"type":"run.started","visible":true})),
        })
        .expect("record started trace");
    assert!(inserted);

    first.kill().unwrap();
    first.wait().unwrap();
    let _ = std::fs::remove_file(&socket);
    let mut second = start_service(&database, &socket);
    ready(&client, &mut second);
    let restored: Vec<TraceRecord> = client
        .execute(ApplicationCommand::TraceEventsAfter {
            trace_id: "trace-cold".into(),
            sequence: 0,
        })
        .expect("restore started trace");
    assert_eq!(restored.len(), 1);
    assert!(restored[0].envelope.is_some());
    let completed: bool = client
        .execute(ApplicationCommand::RecordAgentTrace {
            event: event(2, AgentTraceKind::RunCompleted, AgentTraceStatus::Succeeded),
            envelope: None,
        })
        .expect("record terminal trace");
    assert!(completed);

    second.kill().unwrap();
    second.wait().unwrap();
    let _ = std::fs::remove_file(&socket);
    let mut third = start_service(&database, &socket);
    ready(&client, &mut third);
    let runs: Vec<TraceRun> = client
        .execute(ApplicationCommand::TraceRunsForConversation {
            conversation_id: "conversation-cold".into(),
        })
        .expect("restore terminal run");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].status, "succeeded");
    assert_eq!(runs[0].last_sequence, 2);
    assert!(runs[0].finished_at.is_some());

    third.kill().unwrap();
    third.wait().unwrap();
}
