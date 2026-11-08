// SPDX-License-Identifier: Apache-2.0
mod support;
use nola_application::{
    ApplicationCommand, ApplicationCommandReceiptOutcome, ExecuteReceivedCommand,
};
use nola_agent_engine::agent_trace::{
    AgentTraceEvent, AgentTraceKind, AgentTracePayload, AgentTraceStatus, AgentTraceVisibility,
};
use serde_json::{json, Value};
use support::ApplicationRuntime;

fn event(sequence: u64) -> AgentTraceEvent {
    AgentTraceEvent {
        schema_version: 1,
        trace_id: "trace".into(),
        run_id: "run".into(),
        conversation_id: "conversation".into(),
        message_id: Some("message".into()),
        turn_id: "turn".into(),
        event_id: format!("event-{sequence}"),
        sequence,
        emitted_at: format!("2026-09-08T00:00:0{sequence}Z"),
        kind: if sequence == 1 {
            AgentTraceKind::RunStarted
        } else {
            AgentTraceKind::RunCompleted
        },
        span_id: Some("span".into()),
        parent_span_id: Some("parent".into()),
        status: if sequence == 1 {
            AgentTraceStatus::Running
        } else {
            AgentTraceStatus::Succeeded
        },
        visibility: AgentTraceVisibility::Summary,
        payload: AgentTracePayload::default(),
    }
}
fn command(sequence: u64) -> ApplicationCommand {
    ApplicationCommand::RecordAgentTrace {
        event: event(sequence),
        envelope: Some(json!({"source":"host","sequence":sequence})),
    }
}
fn envelope(id: &str, sequence: u64) -> ExecuteReceivedCommand {
    ExecuteReceivedCommand {
        client_id: "trace-port".into(),
        request_id: id.into(),
        command_type: "record_agent_trace".into(),
        command_json: serde_json::to_string(&command(sequence)).unwrap(),
        expected_revision: None,
        received_at: "2026-09-08T00:00:00Z".into(),
    }
}
fn events(runtime: &mut ApplicationRuntime, cursor: i64) -> Value {
    runtime
        .execute(ApplicationCommand::TraceEventsAfter {
            trace_id: "trace".into(),
            sequence: cursor,
        })
        .unwrap()
}
fn runs(runtime: &mut ApplicationRuntime, path: &std::path::Path) -> Value {
    // Diagnostic trace records alone never establish a canonical assistant Run.
    assert_eq!(runtime.execute(ApplicationCommand::TraceRunsForConversation {
        conversation_id: "conversation".into(),
    }).unwrap(), json!([]));
    let db = rusqlite::Connection::open(path).unwrap();
    let mut query = db.prepare("SELECT trace_id,run_id,message_id,turn_id,conversation_id,last_sequence,status,started_at,finished_at FROM trace_journal_runs ORDER BY trace_id").unwrap();
    let rows = query.query_map([], |row| Ok(json!({
        "trace_id": row.get::<_, String>(0)?, "run_id": row.get::<_, String>(1)?,
        "message_id": row.get::<_, Option<String>>(2)?, "turn_id": row.get::<_, String>(3)?,
        "conversation_id": row.get::<_, String>(4)?, "last_sequence": row.get::<_, i64>(5)?,
        "status": row.get::<_, String>(6)?, "started_at": row.get::<_, String>(7)?,
        "finished_at": row.get::<_, Option<String>>(8)?,
    }))).unwrap().collect::<Result<Vec<_>, _>>().unwrap();
    json!(rows)
}
#[test]
fn trace_port_receipt_failure_preserves_event_run_cursor_and_cold_replay() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("trace.db");
    let mut runtime = support::open_runtime(&path).unwrap();
    runtime
        .execute_received_command(envelope("start", 1), command(1))
        .unwrap();
    let baseline_events = events(&mut runtime, 0);
    let baseline_runs = runs(&mut runtime, &path);
    assert_eq!(
        baseline_events,
        json!([{"event":event(1),"envelope":{"source":"host","sequence":1}}])
    );
    assert_eq!(baseline_runs[0]["trace_id"], "trace");
    assert_eq!(baseline_runs[0]["run_id"], "run");
    assert_eq!(baseline_runs[0]["message_id"], "message");
    assert_eq!(baseline_runs[0]["turn_id"], "turn");
    assert_eq!(baseline_runs[0]["conversation_id"], "conversation");
    assert_eq!(baseline_runs[0]["last_sequence"], 1);
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TRIGGER fail_trace_receipt BEFORE UPDATE ON command_receipts WHEN NEW.status='committed' BEGIN SELECT RAISE(ABORT,'trace receipt fault'); END").unwrap();
    assert!(runtime
        .execute_received_command(envelope("failed", 2), command(2))
        .is_err());
    assert_eq!(events(&mut runtime, 0), baseline_events);
    assert_eq!(runs(&mut runtime, &path), baseline_runs);
    assert_eq!(events(&mut runtime, 1), json!([]));
    db.execute_batch("DROP TRIGGER fail_trace_receipt").unwrap();
    let mut receipt = runtime
        .execute_received_command(envelope("finish", 2), command(2))
        .unwrap();
    assert!(matches!(
        receipt.outcome,
        ApplicationCommandReceiptOutcome::Committed { .. }
    ));
    assert!(!receipt.replayed);
    receipt.replayed = true;
    let completed_events = events(&mut runtime, 0);
    let completed_runs = runs(&mut runtime, &path);
    assert_eq!(
        events(&mut runtime, 1),
        json!([{"event":event(2),"envelope":{"source":"host","sequence":2}}])
    );
    assert_eq!(completed_runs[0]["last_sequence"], 2);
    assert_eq!(completed_runs[0]["status"], "succeeded");
    assert_eq!(completed_runs[0]["started_at"], event(1).emitted_at);
    assert_eq!(completed_runs[0]["finished_at"], event(2).emitted_at);
    drop(runtime);
    let mut runtime = support::open_runtime(&path).unwrap();
    assert_eq!(
        runtime
            .execute_received_command(envelope("finish", 2), command(2))
            .unwrap(),
        receipt
    );
    assert_eq!(events(&mut runtime, 0), completed_events);
    assert_eq!(runs(&mut runtime, &path), completed_runs);
    assert_eq!(events(&mut runtime, 2), json!([]));
}
