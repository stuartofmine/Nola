// SPDX-License-Identifier: Apache-2.0
mod support;
use nola_application::{ApplicationCommand, CreateConversation, EditConversationMessage};
use nola_protocol::{ConversationTranscriptImportSource, ImportConversationTranscript};
use nola_protocol::{TranscriptMessageInput, TranscriptPartInput};

fn message(id: &str, run: &str, ordinal: i64) -> TranscriptMessageInput {
    TranscriptMessageInput {
        id: id.into(),
        run_id: Some(run.into()),
        role: "assistant".into(),
        status: "completed".into(),
        ordinal,
        created_at_ms: 1,
        completed_at_ms: Some(2),
        parts: vec![TranscriptPartInput {
            id: format!("{id}:text"),
            kind: "text".into(),
            ordinal: 1,
            state: "completed".into(),
            content_text: "evidence".into(),
            content_json: None,
            content_digest: None,
            created_at_ms: 1,
            updated_at_ms: 2,
        }],
    }
}

#[test]
fn foreign_run_import_and_edit_cannot_change_canonical_history() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("run-scope.sqlite");
    let mut runtime = support::open_runtime(&database).unwrap();
    for id in ["target", "other"] {
        runtime
            .execute(ApplicationCommand::CreateConversation {
                input: CreateConversation {
                    id: id.into(),
                    title: id.into(),
                    settings_json: None,
                    created_at_ms: 1,
                    updated_at_ms: 1,
                },
            })
            .unwrap();
    }
    // The target is an external archive, not an executed Run. Only the
    // foreign live Run exists, to exercise rejection of its wrong owner.
    let fixture = rusqlite::Connection::open(&database).unwrap();
    fixture.execute_batch("INSERT INTO runs(id,conversation_id,kind,status,next_event_sequence,created_at_ms,updated_at_ms,completed_at_ms)
        VALUES('foreign-run','other','conversation','running',1,1,2,NULL);").unwrap();
    let import = |run: &str| ApplicationCommand::ImportConversationTranscript {
        input: ImportConversationTranscript {
            import_id: format!("import-{run}"),
            source: ConversationTranscriptImportSource::ExternalArchive,
            conversation_id: "target".into(),
            expected_message_count: 0,
            messages: vec![message("original", run, 1)],
            imported_at_ms: 3,
        },
    };
    assert!(runtime
        .execute(import("foreign-run"))
        .unwrap_err()
        .contains("another conversation"));
    let count: i64 = fixture
        .query_row("SELECT COUNT(*) FROM messages", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 0);
    runtime.execute(import("own-run")).unwrap();
    let before: (i64, i64) = fixture
        .query_row(
            "SELECT revision, (SELECT COUNT(*) FROM messages) FROM conversations WHERE id='target'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(before, (2, 1));
    let error = runtime
        .execute(ApplicationCommand::EditConversationMessage {
            input: EditConversationMessage {
                operation_id: "foreign-edit".into(),
                conversation_id: "target".into(),
                expected_revision: 2,
                parent_message_id: "original".into(),
                replacement_message: message("replacement", "foreign-run", 2),
                committed_at_ms: 4,
            },
        })
        .unwrap_err();
    assert!(error.contains("another conversation"));
    let after: (i64, i64) = fixture
        .query_row(
            "SELECT revision, (SELECT COUNT(*) FROM messages) FROM conversations WHERE id='target'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(before, after);
    drop(runtime);
    let mut runtime = support::open_runtime(&database).unwrap();
    let original = runtime
        .execute(ApplicationCommand::ReadConversationTextRange {
            conversation_id: "target".into(),
            message_id: "original".into(),
            part_id: "original:text".into(),
            offset_bytes: 0,
            limit_bytes: 32,
        })
        .unwrap();
    assert_eq!(original["text"], "evidence");
}
