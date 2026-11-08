// SPDX-License-Identifier: Apache-2.0
use nola_store::{CreateConversationInput, NolaStore};
use nola_store_port::conversation_revert::ApplyConversationProjectionInput;
use nola_store_port::transcript_write::{MessageInput, TranscriptEditRepository};

#[test]
fn edit_repository_rechecks_idle_and_run_ownership_in_commit_transaction() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("edit-admission.db");
    let mut store = NolaStore::open(&database).unwrap();
    for id in ["target", "other"] {
        store
            .create_conversation(&CreateConversationInput {
                id: id.into(),
                title: id.into(),
                revision: 1,
                settings_json: None,
                created_at_ms: 1,
                updated_at_ms: 1,
            })
            .unwrap();
    }
    let fixture = rusqlite::Connection::open(&database).unwrap();
    fixture.execute_batch("INSERT INTO runs(id,conversation_id,kind,status,next_event_sequence,created_at_ms,updated_at_ms) VALUES('active','target','conversation','running',1,1,1),('foreign','other','conversation','running',1,1,1);").unwrap();
    let mut input = ApplyConversationProjectionInput {
        operation_id: "edit".into(),
        conversation_id: "target".into(),
        expected_revision: 1,
        operation_kind: "edit".into(),
        parent_message_id: None,
        mutations: vec![],
        appended_messages: vec![],
        created_at_ms: 2,
    };
    let error = store
        .conversation_write_repository()
        .apply_conversation_projection(&input)
        .unwrap_err();
    assert!(error.to_string().contains("Run is active"));
    fixture
        // Remove the isolated active-guard fixture. Historical import below
        // carries no execution Run and does not manufacture a terminal result.
        .execute("DELETE FROM runs WHERE id='active'", [])
        .unwrap();
    input.appended_messages.push(MessageInput {
        id: "replacement".into(),
        run_id: Some("foreign".into()),
        role: "user".into(),
        status: "completed".into(),
        ordinal: 1,
        created_at_ms: 2,
        completed_at_ms: Some(2),
        parts: vec![],
    });
    let error = store
        .conversation_write_repository()
        .apply_conversation_projection(&input)
        .unwrap_err();
    assert!(error.to_string().contains("another conversation"));
    assert_eq!(
        fixture
            .query_row(
                "SELECT revision FROM conversations WHERE id='target'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    assert_eq!(
        fixture
            .query_row(
                "SELECT COUNT(*) FROM conversation_projection_operations",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    assert_eq!(
        fixture
            .query_row("SELECT COUNT(*) FROM messages", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    input.appended_messages[0].run_id = Some("active".into());
    assert!(
        store
            .conversation_write_repository()
            .apply_conversation_projection(&input)
            .unwrap()
            .applied
    );
    drop(store);
    let mut reopened = NolaStore::open(&database).unwrap();
    assert!(
        !reopened
            .conversation_write_repository()
            .apply_conversation_projection(&input)
            .unwrap()
            .applied
    );
}
