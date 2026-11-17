use nola_domain::CanonicalBodyReference;
use nola_store::{
    BeginCommandReceiptInput, CompleteCommandReceiptOutcome, CreateConversationInput, MessageInput,
    MessagePartInput, NolaStore,
};
use nola_store_port::{
    transcript_publication::{PreparedTranscriptPublication, TranscriptPublicationRepository},
    transcript_write::AppendMessagesInput,
};

fn fixture(path: &std::path::Path) -> (NolaStore, PreparedTranscriptPublication) {
    let mut store = NolaStore::open(path).unwrap();
    store
        .create_conversation(&CreateConversationInput {
            id: "c".into(),
            title: "Original".into(),
            revision: 1,
            settings_json: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        })
        .unwrap();
    let reference =
        CanonicalBodyReference::for_field("u".into(), "c".into(), "m".into(), "p".into(), nola_domain::PartBodyField::ContentText, 11, 2).unwrap();
    store
        .prepare_transcript_content(
            &reference,
            &mut [Ok((0, "abc中".into())), Ok((6, "😀x".into()))].into_iter(),
            100,
        )
        .unwrap();
    let mut input = PreparedTranscriptPublication {
        import_id: "import".into(),
        manifest_json: "{\"identity\":1}".into(),
        expected_conversation_revision: 1,
        references: vec![reference],
        append: AppendMessagesInput {
            conversation_id: "c".into(),
            expected_message_count: 0,
            updated_at_ms: 2,
            messages: vec![MessageInput {
                id: "m".into(),
                run_id: None,
                role: "user".into(),
                status: "completed".into(),
                ordinal: 1,
                created_at_ms: 1,
                completed_at_ms: Some(2),
                parts: vec![MessagePartInput {
                    id: "p".into(),
                    kind: "text".into(),
                    ordinal: 1,
                    state: "completed".into(),
                    content_text: String::new(),
                    content_json: None,
                    content_digest: None,
                    created_at_ms: 1,
                    updated_at_ms: 2,
                }],
            }],
        },
    };
    input.manifest_json = serde_json::json!({
        "expected_conversation_revision":1,
        "transcript":{"import_id":"import","source":"backup_restore","conversation_id":"c",
            "expected_message_count":0,"imported_at_ms":2,"messages":input.append.messages},
        "uploads":[{"upload_id":"u","message_id":"m","part_id":"p","expected_utf8_bytes":11}]
    })
    .to_string();
    (store, input)
}

fn counts(path: &std::path::Path) -> (i64, i64, i64, i64, i64, i64) {
    rusqlite::Connection::open(path)
        .unwrap()
        .query_row(
            "SELECT (SELECT COUNT(*) FROM messages),(SELECT COUNT(*) FROM message_parts),
         (SELECT COUNT(*) FROM message_part_body_references WHERE field='content_text'),
         (SELECT COUNT(*) FROM transcript_import_publications),
         (SELECT COUNT(*) FROM sync_outbox),revision FROM conversations WHERE id='c'",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )
        .unwrap()
}

#[test]
fn receipt_reference_and_sync_publication_commit_together_and_replay_after_restart() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("main.db");
    let (mut store, input) = fixture(&path);
    let before = counts(&path);
    assert!(store.publish_transcript(&input).is_err());
    store.begin_write_unit_of_work().unwrap();
    store
        .begin_command_receipt(&BeginCommandReceiptInput {
            client_id: "client".into(),
            request_id: "request".into(),
            command_type: "uploaded_transcript".into(),
            command_json: input.manifest_json.clone(),
            expected_revision: Some(1),
            received_at: "2026-09-08T00:00:00Z".into(),
        })
        .unwrap();
    let result = store.publish_transcript(&input).unwrap();
    store
        .complete_command_receipt(
            "client",
            "request",
            "2026-09-08T00:00:01Z",
            &CompleteCommandReceiptOutcome::Committed {
                committed_revision: Some(2),
                result_json: serde_json::to_string(&result).unwrap(),
            },
        )
        .unwrap();
    assert_eq!(
        counts(&path),
        before,
        "another connection cannot observe facts before the receipt commit"
    );
    store.commit_write_unit_of_work().unwrap();
    drop(store);
    let mut reopened = NolaStore::open(&path).unwrap();
    assert_eq!(
        reopened
            .command_receipt("client", "request")
            .unwrap()
            .unwrap()
            .status,
        "committed"
    );
    assert_eq!(
        reopened
            .transcript_publication_replay("import", &input.manifest_json)
            .unwrap(),
        Some(result.clone())
    );
    let after = counts(&path);
    assert_eq!(
        (after.0, after.1, after.2, after.3, after.5),
        (1, 1, 1, 1, 2)
    );
    reopened.begin_write_unit_of_work().unwrap();
    assert_eq!(reopened.publish_transcript(&input).unwrap(), result);
    reopened.commit_write_unit_of_work().unwrap();
    assert_eq!(counts(&path), after);
    assert!(reopened
        .transcript_publication_replay("import", "{\"identity\":2}")
        .is_err());
}

#[test]
fn late_outbox_failure_rolls_back_all_visible_facts_but_keeps_prepared_content() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("main.db");
    let (mut store, input) = fixture(&path);
    let before = counts(&path);
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection.execute_batch("CREATE TRIGGER fail_uploaded_append BEFORE INSERT ON sync_outbox
        WHEN NEW.entity_type='conversation_append' BEGIN SELECT RAISE(ABORT,'injected late failure'); END;").unwrap();
    store.begin_write_unit_of_work().unwrap();
    assert!(store
        .publish_transcript(&input)
        .unwrap_err()
        .to_string()
        .contains("injected late failure"));
    store.commit_write_unit_of_work().unwrap();
    drop(store);
    assert_eq!(counts(&path), before);
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM canonical_text_objects", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    connection
        .execute_batch("DROP TRIGGER fail_uploaded_append")
        .unwrap();
    let mut reopened = NolaStore::open(&path).unwrap();
    reopened.begin_write_unit_of_work().unwrap();
    reopened.publish_transcript(&input).unwrap();
    reopened.commit_write_unit_of_work().unwrap();
    assert_eq!(counts(&path).0, 1);
}

#[test]
fn stale_revision_and_missing_content_never_publish_empty_parts() {
    for failure in ["revision", "tail", "active_run"] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("main.db");
        let (mut store, input) = fixture(&path);
        store.preflight_transcript_publication("c", 1, 0).unwrap();
        let connection = rusqlite::Connection::open(&path).unwrap();
        connection.execute_batch(if failure == "revision" {
            "UPDATE conversations SET revision=2 WHERE id='c'"
        } else if failure == "tail" { "DELETE FROM canonical_text_chunks WHERE offset_bytes=6" }
        else { "INSERT INTO runs(id,conversation_id,kind,status,next_event_sequence,created_at_ms,updated_at_ms)
                VALUES('run','c','maintenance','running',1,3,3)" }).unwrap();
        let before = counts(&path);
        store.begin_write_unit_of_work().unwrap();
        assert!(store.publish_transcript(&input).is_err(), "{failure}");
        store.commit_write_unit_of_work().unwrap();
        assert_eq!(counts(&path), before);
    }
}

#[test]
fn changed_prepared_write_cannot_reuse_an_unchanged_manifest_encoding() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("main.db");
    let (mut store, input) = fixture(&path);
    let before = counts(&path);
    for mutation in ["message", "binding", "revision", "count"] {
        let mut changed = input.clone();
        match mutation {
            "message" => changed.append.messages[0].role = "assistant".into(),
            "binding" => {
                changed.references[0] = CanonicalBodyReference::for_field(
                    "other".into(),
                    "c".into(),
                    "m".into(),
                    "p".into(),
                    nola_domain::PartBodyField::ContentText,
                    11,
                    2,
                )
                .unwrap()
            }
            "revision" => changed.expected_conversation_revision = 2,
            _ => changed.append.expected_message_count = 1,
        }
        store.begin_write_unit_of_work().unwrap();
        assert!(store.publish_transcript(&changed).is_err(), "{mutation}");
        store.commit_write_unit_of_work().unwrap();
        assert_eq!(counts(&path), before);
    }
}

#[test]
fn preparation_failure_and_changed_chunk_replay_preserve_the_original_object() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("main.db");
    let (mut store, input) = fixture(&path);
    assert!(store
        .prepare_transcript_content(
            &input.references[0],
            &mut [Ok((0, "CHANGED".into())), Ok((7, "tail".into()))].into_iter(),
            100
        )
        .is_err());
    let failed =
        CanonicalBodyReference::for_field("failed".into(), "c".into(), "m2".into(), "p2".into(), nola_domain::PartBodyField::ContentText, 11, 2)
            .unwrap();
    assert!(store
        .prepare_transcript_content(
            &failed,
            &mut [Ok((0, "abc中".into())), Err("source interrupted".into())].into_iter(),
            100
        )
        .is_err());
    drop(store);
    let connection = rusqlite::Connection::open(&path).unwrap();
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM canonical_text_objects", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        connection
            .query_row(
                "SELECT text FROM canonical_text_chunks WHERE content_id='u' AND offset_bytes=0",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "abc中"
    );
    assert_eq!(counts(&path).0, 0);
}
