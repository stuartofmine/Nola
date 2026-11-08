// SPDX-License-Identifier: Apache-2.0
//! Idle canonical revert resolves metadata separately from bounded Core context.
mod support;
use nola_application::{
    ApplicationCommand, ApplicationCommandReceiptOutcome, ExecuteReceivedCommand,
    RevertConversation,
};
use nola_agent_engine::agent::{AgentCoreConfig, SessionId};
use nola_protocol::CommandMetadata;
use nola_store::NolaStore;
use serde_json::Value;

fn command(keep_message: bool) -> ApplicationCommand {
    ApplicationCommand::RevertConversation {
        input: RevertConversation {
            conversation_id: "history".into(),
            operation_id: "revert-old-boundary".into(),
            through_message_id: "m40".into(),
            keep_message,
            committed_at_ms: 100,
            core_config: AgentCoreConfig {
                turn_identity: support::model_identity::identity(),
                session_id: Some(SessionId::from_string("history".into())),
                ..Default::default()
            },
        },
    }
}
fn received(id: &str, command: &ApplicationCommand) -> ExecuteReceivedCommand {
    ExecuteReceivedCommand {
        client_id: "bounded-revert".into(),
        request_id: id.into(),
        command_type: command.wire_command_name().into(),
        command_json: serde_json::to_string(command).unwrap(),
        expected_revision: None,
        received_at: "2026-09-08T00:00:00Z".into(),
    }
}
fn count(db: &rusqlite::Connection, table: &str) -> i64 {
    db.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
        row.get(0)
    })
    .unwrap()
}
#[test]
fn cold_idle_revert_outside_tail_window_keeps_originals_and_receipt_transaction_atomic() {
    for keep in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("history.db");
        drop(NolaStore::open(&path).unwrap());
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute(
            "INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms) VALUES('history','History',1,1,1)",
            [],
        )
        .unwrap();
        for ordinal in 1..=80 {
            let id = format!("m{ordinal}");
            db.execute(
                "INSERT INTO messages VALUES(?1,'history',NULL,'user','completed',?2,1,2)",
                rusqlite::params![id, ordinal],
            )
            .unwrap();
            db.execute("INSERT INTO message_parts(id,message_id,kind,ordinal,state,content_text,created_at_ms,updated_at_ms) VALUES(?1,?2,'text',1,'completed',?3,1,2)",
                rusqlite::params![format!("p{ordinal}"), id, if ordinal == 1 { "x".repeat(10_000_000) } else { format!("evidence-{ordinal}") }]).unwrap();
        }
        let store = NolaStore::open(&path).unwrap();
        let tail = store
            .load_conversation_message_window("history", None, 32, 32768)
            .unwrap()
            .unwrap();
        assert!(tail
            .conversation
            .messages
            .iter()
            .all(|message| message.id != "m40"));
        assert!(store.load_checkpoint("history").unwrap().is_none());
        drop(store);
        let mut runtime = support::open_runtime(&path).unwrap();
        let before_outbox = count(&db, "sync_outbox");
        let before_events = count(&db, "projection_events");
        db.execute_batch("CREATE TRIGGER fail_revert_receipt BEFORE UPDATE ON command_receipts WHEN NEW.status='committed' BEGIN SELECT RAISE(ABORT,'receipt fault'); END").unwrap();
        let input = command(keep);
        let outcome = runtime.execute_received_command(received("failed-revert", &input), input);
        if let Ok(receipt) = outcome {
            assert!(matches!(
                receipt.outcome,
                ApplicationCommandReceiptOutcome::Rejected { .. }
            ));
        }
        assert_eq!(count(&db, "message_projection_events"), 0);
        assert_eq!(count(&db, "sync_outbox"), before_outbox);
        assert_eq!(count(&db, "projection_events"), before_events);
        assert_eq!(count(&db, "core_checkpoints"), 0);
        assert_eq!(
            db.query_row(
                "SELECT revision FROM conversations WHERE id='history'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            1
        );
        db.execute_batch("DROP TRIGGER fail_revert_receipt")
            .unwrap();
        drop(runtime);
        let mut runtime = support::open_runtime(&path).unwrap();
        let input = command(keep);
        let admission = received("successful-revert", &input);
        let receipt = runtime
            .execute_received_command(admission.clone(), input)
            .unwrap();
        let result: Value = match &receipt.outcome {
            ApplicationCommandReceiptOutcome::Committed { result, .. } => result.clone(),
            other => panic!("{other:?}"),
        };
        assert!(
            result["execution"]["control"]["history_item_count"].as_u64().unwrap() > 20,
            "notice ownership must not discard retained prefix: {result}"
        );
        assert!(result.get("checkpoint").is_none());
        assert!(result.get("snapshot").is_none());
        let store = NolaStore::open(&path).unwrap();
        let checkpoint = store.load_checkpoint("history").unwrap().unwrap().checkpoint_json;
        assert!(
            checkpoint.len() < 100_000,
            "revert must not copy 10MB original into Core"
        );
        assert!(!checkpoint.contains("evidence-41"));
        assert_eq!(checkpoint.contains("evidence-40"), keep);
        let window = store
            .load_conversation_message_window("history", None, 80, 1024)
            .unwrap()
            .unwrap();
        assert_eq!(window.total_message_count, if keep { 40 } else { 39 });
        assert_eq!(
            window.conversation.messages.last().unwrap().id,
            if keep { "m40" } else { "m39" }
        );
        assert_eq!(
            db.query_row(
                "SELECT length(CAST(content_text AS BLOB)) FROM message_parts WHERE id='p1'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            10_000_000
        );
        let after_events = count(&db, "projection_events");
        let after_hides = count(&db, "message_projection_events");
        let after_outbox = count(&db, "sync_outbox");
        let expected = receipt.into_protocol_receipt();
        drop(store);
        drop(runtime);
        let mut runtime = support::open_runtime(&path).unwrap();
        let replay = runtime
            .execute_received_command(admission, command(keep))
            .unwrap();
        assert!(replay.replayed);
        assert_eq!(replay.into_protocol_receipt(), expected);
        assert_eq!(count(&db, "projection_events"), after_events);
        assert_eq!(count(&db, "message_projection_events"), after_hides);
        assert_eq!(count(&db, "sync_outbox"), after_outbox);
        let mut hidden = match command(keep) {
            ApplicationCommand::RevertConversation { input } => input,
            _ => unreachable!(),
        };
        hidden.operation_id = "hidden-boundary".into();
        hidden.through_message_id = "m70".into();
        let rejected_command = ApplicationCommand::RevertConversation { input: hidden };
        let rejected = runtime
            .execute_received_command(
                received("hidden-boundary", &rejected_command),
                rejected_command,
            )
            .unwrap();
        assert!(matches!(
            rejected.outcome,
            ApplicationCommandReceiptOutcome::Rejected { .. }
        ));
        assert_eq!(count(&db, "message_projection_events"), after_hides);
        // Existing nonempty checkpoints continue to use Core's real retained
        // history and boundary validation, rather than replacing it from Store.
        let mut second = match command(true) {
            ApplicationCommand::RevertConversation { input } => input,
            _ => unreachable!(),
        };
        second.operation_id = "checkpoint-revert".into();
        second.through_message_id = "m20".into();
        let second_command = ApplicationCommand::RevertConversation { input: second };
        let second = runtime
            .execute_received_command(
                received("checkpoint-revert", &second_command),
                second_command,
            )
            .unwrap();
        assert!(matches!(
            second.outcome,
            ApplicationCommandReceiptOutcome::Committed { .. }
        ));
        let store = NolaStore::open(&path).unwrap();
        assert_eq!(
            store
                .load_conversation_message_window("history", None, 10, 1024)
                .unwrap()
                .unwrap()
                .total_message_count,
            20
        );
    }
}
