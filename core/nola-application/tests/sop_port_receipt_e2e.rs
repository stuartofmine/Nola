// SPDX-License-Identifier: Apache-2.0
mod support;
use nola_application::{
    ApplicationCommand, ApplicationCommandReceiptOutcome, ExecuteReceivedCommand, SopRunCommand,
};
use nola_agent_engine::agent::sop::*;
use serde_json::json;
fn definition() -> SopDefinition {
    SopDefinition {
        schema_version: SOP_SCHEMA_VERSION,
        id: "application.sop".into(),
        version: 1,
        title: "Application SOP".into(),
        inputs: json!({}),
        steps: vec![SopStep {
            id: "work".into(),
            title: "Work".into(),
            depends_on: Vec::new(),
            mode: SopExecutionMode::Inline,
            concurrent: false,
            inputs: json!({}),
            outputs: json!({}),
            preconditions: Vec::new(),
            completion: SopCompletionContract {
                description: "work is complete".into(),
                evidence_required: true,
            },
            required_capabilities: Vec::new(),
            required_permissions: Vec::new(),
            budget: SopStepBudget::default(),
            retry: SopRetryPolicy::default(),
            artifacts: Vec::new(),
            verifier: None,
            resume_key: Some("work-key".into()),
        }],
        verification: SopVerificationContract::default(),
        compensation: SopCompensationContract::default(),
    }
}

#[test]
fn sop_port_receipt_failure_rolls_back_transition_and_replays_after_restart() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("sop-receipt.db");
    let mut runtime = support::open_runtime(&path).unwrap();
    runtime
        .execute(ApplicationCommand::SopRun {
            command: SopRunCommand::Create {
                definition: definition(),
                run_id: "run".into(),
                inputs: json!({}),
                owner_session_id: None,
                project_root: "/workspace".into(),
                now_ms: 1,
            },
        })
        .unwrap();
    let command = || ApplicationCommand::SopRun {
        command: SopRunCommand::Apply {
            run_id: "run".into(),
            expected_revision: 1,
            event: SopRunEvent::StartStep {
                step_id: "work".into(),
                executor_ref: "executor".into(),
            },
            now_ms: 2,
        },
    };
    let envelope = |id: &str| ExecuteReceivedCommand {
        client_id: "sop-port".into(),
        request_id: id.into(),
        command_type: "sop_run".into(),
        command_json: serde_json::to_string(&command()).unwrap(),
        expected_revision: None,
        received_at: "2026-09-08T00:00:00Z".into(),
    };
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection.execute_batch("CREATE TRIGGER fail_sop_receipt BEFORE UPDATE ON command_receipts WHEN NEW.status='committed' BEGIN SELECT RAISE(ABORT,'receipt failure'); END").unwrap();
    assert!(runtime
        .execute_received_command(envelope("failure"), command())
        .is_err());
    let revision: i64 = connection
        .query_row(
            "SELECT revision FROM sop_runs WHERE run_id='run'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(revision, 1);
    connection
        .execute_batch("DROP TRIGGER fail_sop_receipt")
        .unwrap();
    let receipt = runtime
        .execute_received_command(envelope("success"), command())
        .unwrap();
    assert!(matches!(
        receipt.outcome,
        ApplicationCommandReceiptOutcome::Committed { .. }
    ));
    assert!(!receipt.replayed);
    let mut expected = receipt;
    expected.replayed = true;
    drop(runtime);
    let mut runtime = support::open_runtime(&path).unwrap();
    let replay = runtime
        .execute_received_command(envelope("success"), command())
        .unwrap();
    assert_eq!(replay, expected);
    let revision: i64 = connection
        .query_row(
            "SELECT revision FROM sop_runs WHERE run_id='run'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(revision, 2);
}
