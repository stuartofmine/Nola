// SPDX-License-Identifier: Apache-2.0
// Application receipt, projection journal and SQLite restart integration.

mod support;
use nola_application::{ApplicationCommand, ExecuteReceivedCommand};
use nola_store::{CreateConversationInput, NolaStore};

fn update(id: &str, after_sequence: u64) -> ApplicationCommand {
    ApplicationCommand::LoadConversationProjectionWindowUpdateV1 {
        conversation_id: id.into(),
        before_ordinal: None,
        limit: 64,
        content_byte_limit: 16_384,
        after_sequence,
    }
}

#[test]
fn receipted_transcript_import_invalidates_the_visible_window() {
    use nola_application::CreateConversation;
    use nola_protocol::{ConversationTranscriptImportSource, ImportConversationTranscript};
    use nola_protocol::{
        TranscriptMessageInput as MessageInput, TranscriptPartInput as MessagePartInput,
    };
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("import-window.db");
    let mut runtime = support::open_runtime(&database).unwrap();
    runtime
        .execute(ApplicationCommand::CreateConversation {
            input: CreateConversation {
                id: "import-window".into(),
                title: "History".into(),
                settings_json: None,
                created_at_ms: 1,
                updated_at_ms: 1,
            },
        })
        .unwrap();
    assert!(runtime
        .execute(update("import-window", 0))
        .unwrap()
        .is_null());
    let command = ApplicationCommand::ImportConversationTranscript {
        input: ImportConversationTranscript {
            import_id: "archive-1".into(),
            source: ConversationTranscriptImportSource::ExternalArchive,
            conversation_id: "import-window".into(),
            expected_message_count: 0,
            imported_at_ms: 2,
            messages: vec![MessageInput {
                id: "imported-message".into(),
                run_id: None,
                role: "user".into(),
                status: "completed".into(),
                ordinal: 1,
                created_at_ms: 1,
                completed_at_ms: Some(1),
                parts: vec![MessagePartInput {
                    id: "imported-part".into(),
                    kind: "text".into(),
                    ordinal: 1,
                    state: "completed".into(),
                    content_text: "Imported evidence".into(),
                    content_json: None,
                    content_digest: None,
                    created_at_ms: 1,
                    updated_at_ms: 1,
                }],
            }],
        },
    };
    let receipt = runtime
        .execute_received_command(
            ExecuteReceivedCommand {
                client_id: "import-client".into(),
                request_id: "import-1".into(),
                command_type: "import_conversation_transcript".into(),
                command_json: serde_json::to_string(&command).unwrap(),
                expected_revision: None,
                received_at: "2026-09-08T00:00:00Z".into(),
            },
            command,
        )
        .unwrap();
    assert!(matches!(
        receipt.outcome,
        nola_application::ApplicationCommandReceiptOutcome::Committed { .. }
    ));
    // Prove persistence succeeded before checking visibility. A green import
    // receipt alone cannot establish that an already-connected UI will refresh.
    let evidence = runtime
        .execute(ApplicationCommand::SearchConversationText {
            conversation_id: "import-window".into(),
            query: "Imported evidence".into(),
            limit: 1,
        })
        .unwrap();
    assert_eq!(evidence.as_array().unwrap().len(), 1);
    let window = runtime.execute(update("import-window", 0)).unwrap();
    assert!(
        !window.is_null(),
        "committed import must advance the projection cursor so connected clients refresh"
    );
    assert_eq!(
        window["projection"]["messages"][0]["message_id"],
        "imported-message"
    );
    let sequence = window["event_sequence"].as_u64().unwrap();
    assert!(sequence > 0);
    drop(runtime);
    let mut runtime = support::open_runtime(&database).unwrap();
    assert_eq!(runtime.execute(update("import-window", 0)).unwrap(), window);
    assert!(runtime
        .execute(update("import-window", sequence))
        .unwrap()
        .is_null());
}

#[test]
fn window_update_tracks_receipted_changes_without_consuming_the_journal() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("window-updates.db");
    let mut store = NolaStore::open(&database).unwrap();
    store
        .create_conversation(&CreateConversationInput {
            id: "window".into(),
            title: "Before".into(),
            revision: 1,
            settings_json: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        })
        .unwrap();
    drop(store);
    let mut runtime = support::open_runtime(&database).unwrap();
    assert!(runtime.execute(update("window", 0)).unwrap().is_null());
    assert!(runtime.execute(update("missing", 0)).is_err());
    assert!(runtime.execute(update("window", 1)).is_err());
    let command = ApplicationCommand::RenameConversation {
        title_source: None,
        conversation_id: "window".into(),
        title: "After".into(),
        updated_at_ms: 2,
    };
    let command_json = serde_json::to_string(&command).unwrap();
    let receipt = runtime
        .execute_received_command(
            ExecuteReceivedCommand {
                client_id: "window-client".into(),
                request_id: "rename-window".into(),
                command_type: "rename_conversation".into(),
                command_json,
                expected_revision: None,
                received_at: "2026-09-05T00:00:00Z".into(),
            },
            command,
        )
        .unwrap();
    assert!(matches!(
        receipt.outcome,
        nola_application::ApplicationCommandReceiptOutcome::Committed { .. }
    ));
    let window = runtime.execute(update("window", 0)).unwrap();
    assert_eq!(window["projection"]["title"], "After");
    let sequence = window["event_sequence"].as_u64().unwrap();
    assert!(sequence > 0);
    assert_eq!(runtime.execute(update("window", 0)).unwrap(), window);
    assert!(runtime
        .execute(update("window", sequence))
        .unwrap()
        .is_null());
    assert!(runtime.execute(update("window", sequence + 1)).is_err());
    drop(runtime);
    let mut runtime = support::open_runtime(&database).unwrap();
    assert_eq!(runtime.execute(update("window", 0)).unwrap(), window);
    assert!(runtime
        .execute(update("window", sequence))
        .unwrap()
        .is_null());
}
