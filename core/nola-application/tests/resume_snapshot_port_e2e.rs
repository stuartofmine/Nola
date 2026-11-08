// SPDX-License-Identifier: Apache-2.0
mod support;
use nola_application::{
    acknowledge_resume_snapshot, load_resume_snapshot, AcknowledgeResumeSnapshot,
    ResumeMailboxCursor,
};
use nola_agent_engine::agent::recovery::CheckpointReason;
use nola_agent_engine::agent::{
    AgentCore, AgentCoreConfig, GoalRecordStatus, GoalRequirement, GoalRuntimeCommand,
    GoalRuntimeCommandEnvelope, SessionId,
};
use nola_store_port::{
    consistent_read::ConsistentReadRepository,
    remote_session::{AuthoritativeSessionRecord, SessionProjectionRepository},
    resume_snapshot::*,
    run_effect::StoredCheckpoint,
    runtime_task::StoredRuntimeTask,
};
fn checkpoint() -> StoredCheckpoint {
    let core = AgentCore::try_new(AgentCoreConfig {
        turn_identity: support::model_identity::identity(),
        session_id: Some(SessionId::from_string("c".into())),
        ..Default::default()
    })
    .unwrap();
    let mut checkpoint = core.checkpoint(CheckpointReason::Manual);
    checkpoint
        .state
        .goal_projection
        .apply(GoalRuntimeCommandEnvelope {
            operation_id: "start".into(),
            command: GoalRuntimeCommand::Start {
                id: "g".into(),
                objective: "resume".into(),
                constraints: vec![],
                requirements: vec![GoalRequirement {
                    acceptance_policy: nola_agent_engine::agent::goal::GoalAcceptancePolicy::ModelJudgment,
                    semantic_revision: 1, id: "check".into(),
                    description: "verify".into(),
                    dependencies: vec![],
                    status: GoalRecordStatus::Pending,
                    evidence_refs: vec![],
                }],
                now_epoch_ms: 1,
            },
        })
        .unwrap();
    StoredCheckpoint {
        conversation_id: "c".into(),
        revision: 1,
        event_cursor: checkpoint.last_event_cursor.sequence as i64,
        checkpoint_json: serde_json::to_string(&checkpoint).unwrap(),
        updated_at_ms: 1,
    }
}
fn task() -> StoredRuntimeTask {
    serde_json::from_value(serde_json::json!({"id":"task","kind":"shell","status":"completed","title":"Evidence","project_root":"/workspace","owner_session_id":"c","unread":true,"cancel_requested":false,"retry_count":0,"created_at_ms":1,"updated_at_ms":2,"metadata_json":"{}"})).unwrap()
}
#[test]
fn real_resume_helper_reads_snapshot_and_only_explicit_ack_consumes_cursor() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("resume-helper.db");
    let mut store = seed(&path);
    let result = load_resume_snapshot(&mut store, " /workspace ", Some("host"), 3)
        .unwrap().unwrap();
    assert!(store.runtime_task("task").unwrap().unwrap().unread);
    assert_eq!(result.mailbox_cursor, vec![ResumeMailboxCursor {
        task_id: "task".into(), updated_at_ms: 2,
    }]);
    acknowledge_resume_snapshot(&mut store, AcknowledgeResumeSnapshot {
        conversation_id: "c".into(), checkpoint_revision: 1,
        mailbox_cursor: result.mailbox_cursor,
    }).unwrap();
    drop(store);
    let store = nola_store::NolaStore::open(&path).unwrap();
    assert!(!store.runtime_task("task").unwrap().unwrap().unread);
}
fn seed(path: &std::path::Path) -> nola_store::NolaStore {
    let mut store = nola_store::NolaStore::open(path).unwrap();
    store
        .create_conversation(&nola_store::CreateConversationInput {
            id: "c".into(),
            title: "Goal".into(),
            revision: 1,
            settings_json: Some(r#"{"project_root":"/workspace"}"#.into()),
            created_at_ms: 1,
            updated_at_ms: 1,
        })
        .unwrap();
    let cp = checkpoint();
    store
        .save_checkpoint(&nola_store::SaveCheckpointInput {
            conversation_id: "c".into(),
            expected_revision: 0,
            revision: 1,
            event_cursor: cp.event_cursor,
            checkpoint_json: cp.checkpoint_json,
            updated_at_ms: 1,
        })
        .unwrap();
    store.create_runtime_task_with_event(&task()).unwrap();
    store
}
#[test]
fn sqlite_port_pins_candidates_session_and_facts_across_another_writer() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("snapshot.db");
    let mut store = seed(&path);
    let writer = rusqlite::Connection::open(&path).unwrap();
    ConsistentReadRepository::with_consistent_read(&mut store,|store| {
        assert_eq!(store.resumable_conversation_ids("/workspace").unwrap(),vec!["c"]);
        writer.execute_batch("BEGIN;UPDATE core_checkpoints SET revision=2 WHERE conversation_id='c';UPDATE durable_tasks SET updated_at_ms=3 WHERE id='task';COMMIT").unwrap();
        let Some(AuthoritativeSessionRecord::Local(cp))=store.load_authoritative_session("c").unwrap() else {panic!()};
        let facts=store.load_resume_facts("c","/workspace",None).unwrap();
        assert_eq!(cp.recovery.checkpoint.revision,1);assert_eq!(facts.runtime_tasks[0].updated_at_ms,2);Ok(())
    }).unwrap();
    assert_eq!(store.load_checkpoint("c").unwrap().unwrap().revision, 2);
    assert_eq!(
        store.runtime_task("task").unwrap().unwrap().updated_at_ms,
        3
    );
    let rejected: Result<(), String> =
        ConsistentReadRepository::with_consistent_read(&mut store, |_| Err("read rejected".into()));
    assert!(rejected.is_err());
    assert_eq!(store.load_checkpoint("c").unwrap().unwrap().revision, 2);
}
#[test]
fn mailbox_ack_rolls_back_with_receipt_and_replays_without_double_consumption() {
    use nola_application::{
        ApplicationCommand, ApplicationCommandReceiptOutcome, ExecuteReceivedCommand,
    };
    use nola_protocol::CommandMetadata;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ack.db");
    drop(seed(&path));
    let command = || ApplicationCommand::AcknowledgeResumeSnapshot {
        input: AcknowledgeResumeSnapshot {
            conversation_id: "c".into(),
            checkpoint_revision: 1,
            mailbox_cursor: vec![ResumeMailboxCursor {
                task_id: "task".into(),
                updated_at_ms: 2,
            }],
        },
    };
    let admission = |id: &str| ExecuteReceivedCommand {
        client_id: "resume-port".into(),
        request_id: id.into(),
        command_type: command().wire_command_name().into(),
        command_json: serde_json::to_string(&command()).unwrap(),
        expected_revision: None,
        received_at: "2026-09-08T00:00:00Z".into(),
    };
    let db = rusqlite::Connection::open(&path).unwrap();
    let unread = || {
        db.query_row(
            "SELECT unread FROM durable_tasks WHERE id='task'",
            [],
            |r| r.get::<_, bool>(0),
        )
        .unwrap()
    };
    db.execute_batch("CREATE TRIGGER fail_ack BEFORE UPDATE ON command_receipts WHEN NEW.status='committed' BEGIN SELECT RAISE(ABORT,'receipt failure');END").unwrap();
    let mut runtime = support::open_runtime(&path).unwrap();
    let result = runtime.execute_received_command(admission("failed"), command());
    if let Ok(receipt) = result {
        assert!(matches!(
            receipt.outcome,
            ApplicationCommandReceiptOutcome::Rejected { .. }
        ));
    }
    assert!(unread());
    db.execute_batch("DROP TRIGGER fail_ack").unwrap();
    drop(runtime);
    let mut runtime = support::open_runtime(&path).unwrap();
    let received = admission("success");
    let receipt = runtime
        .execute_received_command(received.clone(), command())
        .unwrap();
    assert!(matches!(
        receipt.outcome,
        ApplicationCommandReceiptOutcome::Committed { .. }
    ));
    assert!(!unread());
    let replay = runtime
        .execute_received_command(received, command())
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(
        replay.into_protocol_receipt(),
        receipt.into_protocol_receipt()
    );
    assert!(!unread());
}
