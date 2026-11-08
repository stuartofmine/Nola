// SPDX-License-Identifier: Apache-2.0
//! Canonical import, journal and receipt share one SQLite commit boundary.
mod support;
use nola_application::{
    ApplicationCommand, ApplicationCommandReceiptOutcome, ApplicationProjectionScope,
    CreateConversation, ExecuteReceivedCommand,
};
use nola_kernel::{ConversationProjectionReducer, ProjectionApplyError};
use nola_protocol::{ConversationTranscriptImportSource, ImportConversationTranscript};
use nola_protocol::{
    TranscriptMessageInput as MessageInput, TranscriptPartInput as MessagePartInput,
};
use support::ApplicationRuntime;

fn fixture(path: &std::path::Path) -> (ApplicationRuntime, ExecuteReceivedCommand) {
    let mut runtime = support::open_runtime(path).unwrap();
    runtime
        .execute(ApplicationCommand::CreateConversation {
            input: CreateConversation {
                id: "history".into(),
                title: "History".into(),
                settings_json: None,
                created_at_ms: 1,
                updated_at_ms: 1,
            },
        })
        .unwrap();
    let command = ApplicationCommand::ImportConversationTranscript {
        input: ImportConversationTranscript {
            import_id: "archive".into(),
            source: ConversationTranscriptImportSource::ExternalArchive,
            conversation_id: "history".into(),
            expected_message_count: 0,
            imported_at_ms: 2,
            messages: vec![MessageInput {
                id: "message".into(),
                run_id: None,
                role: "user".into(),
                status: "completed".into(),
                ordinal: 1,
                created_at_ms: 1,
                completed_at_ms: Some(1),
                parts: vec![MessagePartInput {
                    id: "part".into(),
                    kind: "text".into(),
                    ordinal: 1,
                    state: "completed".into(),
                    content_text: "private-history-evidence".repeat(1000),
                    content_json: None,
                    content_digest: None,
                    created_at_ms: 1,
                    updated_at_ms: 1,
                }],
            }],
        },
    };
    let input = ExecuteReceivedCommand {
        client_id: "importer".into(),
        request_id: "import".into(),
        command_type: "import_conversation_transcript".into(),
        command_json: serde_json::to_string(&command).unwrap(),
        expected_revision: None,
        received_at: "2026-09-08T00:00:00Z".into(),
    };
    (runtime, input)
}

#[test]
fn import_invalidation_is_bounded_replay_safe_and_requires_snapshot() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("replay.db");
    let (mut runtime, input) = fixture(&path);
    let before = runtime.conversation_projection_snapshot("history").unwrap();
    let mut reducer =
        ConversationProjectionReducer::restore(before.projection.unwrap(), before.current_sequence);
    let receipt = runtime
        .execute_received_command(
            input.clone(),
            serde_json::from_str(&input.command_json).unwrap(),
        )
        .unwrap();
    assert!(matches!(
        receipt.outcome,
        ApplicationCommandReceiptOutcome::Committed { .. }
    ));
    let suffix = runtime
        .projection_event_suffix(
            ApplicationProjectionScope {
                scope_type: "conversation".into(),
                scope_id: "history".into(),
            },
            0,
            10,
        )
        .unwrap();
    assert_eq!(suffix.events.len(), 1);
    let event = suffix.events[0].event.clone();
    let encoded = serde_json::to_string(&event).unwrap();
    assert!(encoded.len() < 1024);
    assert!(!encoded.contains("private-history-evidence"));
    let unchanged = reducer.projection().clone();
    assert_eq!(
        reducer.apply(event),
        Err(ProjectionApplyError::SnapshotRequired { revision: 2 })
    );
    assert_eq!(reducer.last_sequence(), 0);
    assert_eq!(reducer.projection(), &unchanged);
    drop(runtime);
    let mut runtime = support::open_runtime(&path).unwrap();
    let replay = runtime
        .execute_received_command(
            input.clone(),
            serde_json::from_str(&input.command_json).unwrap(),
        )
        .unwrap();
    assert!(replay.replayed);
    let restored = runtime.conversation_projection_snapshot("history").unwrap();
    assert_eq!(restored.current_sequence, 1);
    let restored = reducer_from(restored.projection.unwrap(), restored.current_sequence);
    assert_eq!(restored.projection().messages.len(), 1);
    assert_eq!(restored.projection().messages[0].message_id.0, "message");
}

fn reducer_from(
    projection: nola_domain::ConversationProjection,
    sequence: u64,
) -> ConversationProjectionReducer {
    ConversationProjectionReducer::restore(projection, sequence)
}

#[test]
fn journal_failure_rolls_back_messages_revision_and_receipt() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("rollback.db");
    let (mut runtime, input) = fixture(&path);
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch(
        "CREATE TRIGGER fail_import_journal BEFORE INSERT ON projection_events
        BEGIN SELECT RAISE(ABORT,'injected journal failure'); END;",
    )
    .unwrap();
    assert!(runtime
        .execute_received_command(
            input.clone(),
            serde_json::from_str(&input.command_json).unwrap()
        )
        .is_err());
    let counts: (i64, i64, i64, i64) = db
        .query_row(
            "SELECT (SELECT COUNT(*) FROM messages),
        (SELECT COUNT(*) FROM projection_events),(SELECT COUNT(*) FROM command_receipts),
        (SELECT revision FROM conversations WHERE id='history')",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(counts, (0, 0, 0, 1));
    db.execute_batch("DROP TRIGGER fail_import_journal")
        .unwrap();
    let retry = runtime
        .execute_received_command(
            input.clone(),
            serde_json::from_str(&input.command_json).unwrap(),
        )
        .unwrap();
    assert!(matches!(
        retry.outcome,
        ApplicationCommandReceiptOutcome::Committed { .. }
    ));
}
