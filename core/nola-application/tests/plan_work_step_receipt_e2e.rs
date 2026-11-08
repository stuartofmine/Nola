// SPDX-License-Identifier: Apache-2.0
//! A terminal Work step and its plan link roll back with the outer receipt.
mod support;
use nola_application::{
    execute_application_command, ApplicationCommand, ApplicationCommandReceiptOutcome,
    ConversationPlanCommand, ConversationPlanSopIdentity, ExecuteReceivedCommand,
    WorkStepTransactionCommand, WorkStepTransactionPhase,
};
use nola_protocol::CommandMetadata;
use nola_store::NolaStore;
use serde_json::json;
fn admission(id: &str, command: &ApplicationCommand) -> ExecuteReceivedCommand {
    ExecuteReceivedCommand {
        client_id: "plan-port".into(),
        request_id: id.into(),
        command_type: command.wire_command_name().into(),
        command_json: serde_json::to_string(command).unwrap(),
        expected_revision: None,
        received_at: "2026-09-08T00:00:00Z".into(),
    }
}
fn advance(phase: WorkStepTransactionPhase, revision: i64) -> ApplicationCommand {
    ApplicationCommand::WorkStepTransaction {
        command: WorkStepTransactionCommand::Advance {
            id: "tx".into(),
            expected_revision: revision,
            phase,
            workspace_checkpoint_id: None,
            result: Some(json!({"written":true})),
            result_ref: None,
            error: None,
            updated_at_ms: revision + 2,
        },
    }
}
#[test]
fn receipt_failure_preserves_plan_and_step_then_replay_is_stable() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("step.db");
    let mut store = NolaStore::open(&path).unwrap();
    execute_application_command(&mut store,ApplicationCommand::ConversationPlan { command:ConversationPlanCommand::Save {
        expected_revision: None,
        conversation_id:"work".into(),sop:ConversationPlanSopIdentity {run_id:"r".into(),sop_id:"test".into(),sop_version:1},
        todos:json!([{"id":"s","content":"Write","status":"in_progress","priority":"high","dependencyIds":[],"approvalStatus":"not_required","capabilityRefs":[],"expectedArtifactRefs":[],"successCriteria":["exists"],"verificationStatus":"pending"}]),updated_at_ms:1 } }).unwrap();
    execute_application_command(
        &mut store,
        ApplicationCommand::WorkStepTransaction {
            command: WorkStepTransactionCommand::Prepare {
                id: "tx".into(),
                work_id: "work".into(),
                step_id: "s".into(),
                tool_call_id: "call".into(),
                root: "/workspace".into(),
                effect_kind: "workspace_write".into(),
                workspace_checkpoint_id: None,
                request: json!({"path":"a"}),
                created_at_ms: 1,
            },
        },
    )
    .unwrap();
    execute_application_command(
        &mut store,
        advance(WorkStepTransactionPhase::Checkpointed, 1),
    )
    .unwrap();
    execute_application_command(
        &mut store,
        advance(WorkStepTransactionPhase::EffectObserved, 2),
    )
    .unwrap();
    let before = (
        store.conversation_plan("work").unwrap(),
        store.work_step_transaction("tx").unwrap(),
        store.conversation_plan_revisions("work").unwrap(),
    );
    drop(store);
    let db = rusqlite::Connection::open(&path).unwrap();
    let count = || {
        db.query_row(
            "SELECT (SELECT COUNT(*) FROM sync_outbox)+(SELECT COUNT(*) FROM projection_events)",
            [],
            |r| r.get::<_, i64>(0),
        )
        .unwrap()
    };
    let initial_count = count();
    db.execute_batch("CREATE TRIGGER receipt_failure BEFORE UPDATE ON command_receipts WHEN NEW.status='committed' BEGIN SELECT RAISE(ABORT,'injected receipt failure'); END").unwrap();
    let mut runtime = support::open_runtime(&path).unwrap();
    let command = advance(WorkStepTransactionPhase::Committed, 3);
    let failed = runtime.execute_received_command(
        admission("failed", &command),
        advance(WorkStepTransactionPhase::Committed, 3),
    );
    if let Ok(receipt) = failed {
        assert!(matches!(
            receipt.outcome,
            ApplicationCommandReceiptOutcome::Rejected { .. }
        ));
    }
    let store = NolaStore::open(&path).unwrap();
    assert_eq!(
        (
            store.conversation_plan("work").unwrap(),
            store.work_step_transaction("tx").unwrap(),
            store.conversation_plan_revisions("work").unwrap()
        ),
        before
    );
    assert_eq!(count(), initial_count);
    drop(store);
    db.execute_batch("DROP TRIGGER receipt_failure").unwrap();
    drop(runtime);
    let mut runtime = support::open_runtime(&path).unwrap();
    let received = admission("success", &command);
    let receipt = runtime
        .execute_received_command(
            received.clone(),
            advance(WorkStepTransactionPhase::Committed, 3),
        )
        .unwrap();
    assert!(matches!(
        receipt.outcome,
        ApplicationCommandReceiptOutcome::Committed { .. }
    ));
    let after_count = count();
    let replay = runtime.execute_received_command(received, command).unwrap();
    assert!(replay.replayed);
    assert_eq!(
        replay.into_protocol_receipt(),
        receipt.into_protocol_receipt()
    );
    assert_eq!(count(), after_count);
    let store = NolaStore::open(&path).unwrap();
    assert_eq!(
        store.work_step_transaction("tx").unwrap().unwrap().phase,
        "committed"
    );
    let plan = store.conversation_plan("work").unwrap().unwrap();
    assert_eq!(plan.revision, 2);
    assert!(plan.todos_json.contains("tx"));
}
