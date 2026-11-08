use nola_store::{CreateConversationInput, NolaStore, SaveCheckpointInput};
use tempfile::tempdir;

#[test]
fn checkpoint_compare_and_swap_survives_reopen_and_rejects_stale_writer() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("nola.db");
    let mut first = NolaStore::open(&database).unwrap();
    first
        .create_conversation(&CreateConversationInput {
            id: "conversation-1".into(),
            title: "Checkpoint E2E".into(),
            revision: 1,
            settings_json: None,
            created_at_ms: 1_700_000_000_000,
            updated_at_ms: 1_700_000_000_000,
        })
        .unwrap();
    first
        .save_checkpoint(&SaveCheckpointInput {
            conversation_id: "conversation-1".into(),
            expected_revision: 0,
            revision: 1,
            event_cursor: 7,
            checkpoint_json: "{\"state\":\"first\"}".into(),
            updated_at_ms: 1_700_000_000_100,
        })
        .unwrap();

    let mut second = NolaStore::open(&database).unwrap();
    second
        .save_checkpoint(&SaveCheckpointInput {
            conversation_id: "conversation-1".into(),
            expected_revision: 1,
            revision: 2,
            event_cursor: 8,
            checkpoint_json: "{\"state\":\"second\"}".into(),
            updated_at_ms: 1_700_000_000_200,
        })
        .unwrap();
    let stale = first.save_checkpoint(&SaveCheckpointInput {
        conversation_id: "conversation-1".into(),
        expected_revision: 1,
        revision: 2,
        event_cursor: 9,
        checkpoint_json: "{\"state\":\"stale\"}".into(),
        updated_at_ms: 1_700_000_000_300,
    });
    assert!(stale.is_err());
    drop(first);
    drop(second);

    let reopened = NolaStore::open(&database).unwrap();
    let checkpoint = reopened.load_checkpoint("conversation-1").unwrap().unwrap();
    assert_eq!(checkpoint.revision, 2);
    assert_eq!(checkpoint.event_cursor, 8);
    assert_eq!(checkpoint.checkpoint_json, "{\"state\":\"second\"}");
}
