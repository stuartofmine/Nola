// SPDX-License-Identifier: Apache-2.0
//! Invalid imported facts must not alter the canonical ledger or its revision.
mod support;
use nola_application::{ApplicationCommand, CreateConversation};
use nola_protocol::{ConversationTranscriptImportSource, ImportConversationTranscript};
use nola_protocol::{TranscriptMessageInput, TranscriptPartInput};

fn message() -> TranscriptMessageInput {
    TranscriptMessageInput {
        id: "message".into(),
        run_id: None,
        role: "user".into(),
        status: "completed".into(),
        ordinal: 1,
        created_at_ms: 1,
        completed_at_ms: Some(2),
        parts: vec![TranscriptPartInput {
            id: "part".into(),
            kind: "text".into(),
            ordinal: 1,
            state: "completed".into(),
            content_text: "retained evidence".into(),
            content_json: None,
            content_digest: None,
            created_at_ms: 1,
            updated_at_ms: 2,
        }],
    }
}

#[test]
fn invalid_imports_leave_no_messages_parts_or_revision_changes() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("validation.sqlite");
    let mut application = support::open_runtime(&database).unwrap();
    application
        .execute(ApplicationCommand::CreateConversation {
            input: CreateConversation {
                id: "conversation".into(),
                title: "Before".into(),
                settings_json: None,
                created_at_ms: 1,
                updated_at_ms: 1,
            },
        })
        .unwrap();
    let observer = rusqlite::Connection::open_with_flags(
        &database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let mut cases = vec![Vec::new()];
    for mutation in 0..7 {
        let mut item = message();
        match mutation {
            0 => item.id = " padded ".into(),
            1 => item.parts[0].id = "x".repeat(129),
            2 => item.completed_at_ms = Some(0),
            3 => item.parts[0].updated_at_ms = 0,
            4 => item.parts[0].ordinal = 2,
            5 => item.parts[0].state = "streaming".into(),
            _ => {
                item.parts = (0..513)
                    .map(|index| {
                        let mut part = item.parts[0].clone();
                        part.id = format!("part-{index}");
                        part.ordinal = index + 1;
                        part
                    })
                    .collect()
            }
        }
        cases.push(vec![item]);
    }
    for (index, messages) in cases.into_iter().enumerate() {
        let result = application.execute(ApplicationCommand::ImportConversationTranscript {
            input: ImportConversationTranscript {
                import_id: format!("invalid-{index}"),
                source: ConversationTranscriptImportSource::ExternalArchive,
                conversation_id: "conversation".into(),
                expected_message_count: 0,
                messages,
                imported_at_ms: 3,
            },
        });
        assert!(
            result.is_err(),
            "invalid case {index} unexpectedly committed"
        );
        let state: (i64, i64, i64) = observer
            .query_row(
                "SELECT (SELECT COUNT(*) FROM messages), (SELECT COUNT(*) FROM message_parts),
                (SELECT revision FROM conversations WHERE id='conversation')",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(state, (0, 0, 1), "case {index} changed canonical state");
    }
    application
        .execute(ApplicationCommand::ImportConversationTranscript {
            input: ImportConversationTranscript {
                import_id: "valid".into(),
                source: ConversationTranscriptImportSource::ExternalArchive,
                conversation_id: "conversation".into(),
                expected_message_count: 0,
                messages: vec![message()],
                imported_at_ms: 3,
            },
        })
        .unwrap();
    drop(application);
    let mut application = support::open_runtime(&database).unwrap();
    let range = application
        .execute(ApplicationCommand::ReadConversationTextRange {
            conversation_id: "conversation".into(),
            message_id: "message".into(),
            part_id: "part".into(),
            offset_bytes: 0,
            limit_bytes: 32,
        })
        .unwrap();
    assert_eq!(range["text"], "retained evidence");
}
