// SPDX-License-Identifier: Apache-2.0
//! A mutation must not reload document bodies to form its acknowledgement.
use crate::{NolaStore, StoredDocumentSource};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

#[test]
fn replacement_acknowledgement_never_reads_back_document_bodies() {
    let directory = tempfile::tempdir().unwrap();
    let mut store = NolaStore::open(directory.path().join("documents.db")).unwrap();
    let reads = Arc::new(AtomicUsize::new(0));
    let observed = reads.clone();
    store
        .connection
        .create_scalar_function(
            "reject_document_body_read",
            1,
            rusqlite::functions::FunctionFlags::SQLITE_UTF8,
            move |_| {
                observed.fetch_add(1, Ordering::Relaxed);
                Err::<String, _>(rusqlite::Error::UserFunctionError(Box::new(
                    std::io::Error::other("mutation attempted to read back full body"),
                )))
            },
        )
        .unwrap();
    // Fixture-only view: writes reach a real table, while selecting a body
    // fails. No production hook, allocator mock or timing threshold is needed.
    store.connection.execute_batch(
        "ALTER TABLE document_sources RENAME TO document_write_fixture;
         CREATE VIEW document_sources AS SELECT id,scope,uri,kind,mime_type,sha256,
             byte_count,title,reject_document_body_read(text_content) AS text_content,
             metadata_json,created_at_ms,updated_at_ms FROM document_write_fixture;
         CREATE TRIGGER document_fixture_insert INSTEAD OF INSERT ON document_sources BEGIN
             INSERT INTO document_write_fixture(id,scope,uri,kind,mime_type,sha256,
                 byte_count,title,text_content,metadata_json,created_at_ms,updated_at_ms)
             VALUES(NEW.id,NEW.scope,NEW.uri,NEW.kind,NEW.mime_type,NEW.sha256,
                 NEW.byte_count,NEW.title,NEW.text_content,NEW.metadata_json,NEW.created_at_ms,NEW.updated_at_ms);
         END;
         CREATE TRIGGER document_fixture_delete INSTEAD OF DELETE ON document_sources BEGIN
             DELETE FROM document_write_fixture WHERE id=OLD.id;
         END;"
    ).unwrap();
    let source = StoredDocumentSource {
        id: "large".into(),
        scope: "conversation:1".into(),
        uri: "fixture://large".into(),
        kind: "uploaded_document".into(),
        mime_type: Some("text/plain".into()),
        sha256: None,
        byte_count: Some(2_000_000),
        title: Some("Large".into()),
        text_content: Some("x".repeat(2_000_000)),
        metadata_json: "{}".into(),
        created_at_ms: 1,
        updated_at_ms: 1,
    };
    let count = store
        .replace_document_sources("conversation:1", "uploaded_document", &[source])
        .unwrap();
    assert_eq!(count, 1);
    assert_eq!(reads.load(Ordering::Relaxed), 0);
    let stored_bytes: i64 = store
        .connection
        .query_row(
            "SELECT length(text_content) FROM document_write_fixture WHERE id='large'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(stored_bytes, 2_000_000);
    // Prove the trap is active and would reject the previous post-commit List.
    assert!(store
        .list_document_sources("conversation:1", Some("uploaded_document"))
        .is_err());
    assert!(reads.load(Ordering::Relaxed) > 0);
}
