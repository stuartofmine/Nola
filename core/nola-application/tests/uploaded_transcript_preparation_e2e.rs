// SPDX-License-Identifier: Apache-2.0
//! Real storage preparation, not a substitute for native restore/publication E2E.
use nola_application::uploaded_transcript_preparation::prepare_uploaded_transcript;
use nola_protocol::uploaded_transcript::UploadedTranscriptManifest;
use nola_store::{
    canonical_text_store::CanonicalTextStore, content_upload::ContentUploadStore,
    transcript_preparation::TranscriptContentPreparationStore,
};
use nola_store_port::content_upload::UploadOwner;
use serde_json::json;

fn manifest(bytes: u64) -> UploadedTranscriptManifest {
    serde_json::from_value(json!({
        "expected_conversation_revision":1,
        "transcript":{"import_id":"i","source":"backup_restore","conversation_id":"c",
            "expected_message_count":0,"imported_at_ms":2,"messages":[{
                "id":"m","run_id":null,"role":"user","status":"completed","ordinal":1,
                "created_at_ms":1,"completed_at_ms":2,"parts":[{
                    "id":"p","kind":"text","ordinal":1,"state":"completed","content_text":"",
                    "content_json":null,"content_digest":null,"created_at_ms":1,"updated_at_ms":2
                }]}]},
        "uploads":[{"upload_id":"u","message_id":"m","part_id":"p","expected_utf8_bytes":bytes}]
    }))
    .unwrap()
}

#[test]
fn ten_million_bytes_prepare_replay_and_restart_without_a_full_body_buffer() {
    let directory = tempfile::tempdir().unwrap();
    let upload_path = directory.path().join("uploads.db");
    let canonical_path = directory.path().join("text.db");
    let owner = UploadOwner {
        conversation_id: "c".into(),
        import_id: "i".into(),
        part_id: "p".into(),
    };
    let bytes = 10_000_000;
    let mut uploads = ContentUploadStore::open(&upload_path, bytes).unwrap();
    uploads.begin("u", &owner, bytes).unwrap();
    let mut offset = 0;
    while offset < bytes {
        let length = (bytes - offset).min(32768) as usize;
        // Multi-byte text exercises byte offsets rather than character offsets.
        let text = "\u{00e9}".repeat(length / 2);
        uploads.append("u", &owner, offset, &text).unwrap();
        offset += length as u64;
    }
    let canonical = CanonicalTextStore::open(&canonical_path, bytes).unwrap();
    // An unsealed upload must not become prepared content.
    let mut repository = TranscriptContentPreparationStore::new(uploads, canonical);
    assert!(prepare_uploaded_transcript(&mut repository, &manifest(bytes), bytes).is_err());
    drop(repository);
    let mut uploads = ContentUploadStore::open_existing(&upload_path, bytes).unwrap();
    uploads.seal("u", &owner).unwrap();
    let canonical = CanonicalTextStore::open(&canonical_path, bytes).unwrap();
    let mut repository = TranscriptContentPreparationStore::new(uploads, canonical);
    assert!(prepare_uploaded_transcript(&mut repository, &manifest(bytes - 1), bytes).is_err());
    let references = prepare_uploaded_transcript(&mut repository, &manifest(bytes), bytes).unwrap();
    assert_eq!(
        prepare_uploaded_transcript(&mut repository, &manifest(bytes), bytes).unwrap(),
        references
    );
    let mut redirected = manifest(bytes);
    redirected.transcript.messages[0].id = "other-message".into();
    redirected.uploads[0].message_id = "other-message".into();
    assert!(prepare_uploaded_transcript(&mut repository, &redirected, bytes).is_err());
    let mut foreign = manifest(bytes);
    foreign.transcript.import_id = "foreign".into();
    assert!(prepare_uploaded_transcript(&mut repository, &foreign, bytes).is_err());
    drop(repository);
    let canonical = CanonicalTextStore::open_existing(&canonical_path, bytes).unwrap();
    let tail = canonical
        .read_range(&references[0], bytes - 128, 128)
        .unwrap();
    assert_eq!(tail.text, "\u{00e9}".repeat(64));
    assert!(tail.end_of_content);
    assert_eq!(tail.next_offset_bytes, bytes);
    assert!(serde_json::to_vec(&references).unwrap().len() < 1024);
}

#[test]
fn ten_million_bytes_publish_to_canonical_part_and_recall_after_restarting() {
    use nola_application::uploaded_transcript_publication::publish_uploaded_transcript;
    use nola_store::{CreateConversationInput, NolaStore};
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("main.db");
    let upload_path = directory.path().join("uploads.db");
    let bytes = 10_000_000;
    let owner = UploadOwner {
        conversation_id: "c".into(),
        import_id: "i".into(),
        part_id: "p".into(),
    };
    let mut uploads = ContentUploadStore::open(&upload_path, bytes).unwrap();
    uploads.begin("u", &owner, bytes).unwrap();
    let mut offset = 0;
    while offset < bytes {
        let length = (bytes - offset).min(32768) as usize;
        uploads
            .append("u", &owner, offset, &"é".repeat(length / 2))
            .unwrap();
        offset += length as u64;
    }
    uploads.seal("u", &owner).unwrap();
    drop(uploads);
    let uploads = ContentUploadStore::open_existing(&upload_path, bytes).unwrap();
    let mut store = NolaStore::open(&path).unwrap();
    store
        .create_conversation(&CreateConversationInput {
            id: "c".into(),
            title: "Large original".into(),
            revision: 1,
            settings_json: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        })
        .unwrap();
    store.begin_write_unit_of_work().unwrap();
    let result =
        publish_uploaded_transcript(&mut store, &uploads, &manifest(bytes), bytes).unwrap();
    store.commit_write_unit_of_work().unwrap();
    assert_eq!(result.message_count, 1);
    drop(store);
    let mut store = NolaStore::open(&path).unwrap();
    let tail = store
        .read_conversation_text_range("c", "m", "p", bytes - 128, 128)
        .unwrap();
    assert_eq!(tail.text, "é".repeat(64));
    assert_eq!(tail.total_utf8_bytes, bytes);
    assert!(tail.end_of_content);
    let window = store
        .load_conversation_message_window("c", None, 10, 1024)
        .unwrap()
        .unwrap();
    assert_eq!(window.total_message_count, 1);
    assert!(window.conversation.messages[0].parts[0].content_text.len() <= 1024);
    // Durable manifest replay can return its committed result without reading
    // staging; this remains true after the client reconnects with a new request.
    store.begin_write_unit_of_work().unwrap();
    assert_eq!(
        publish_uploaded_transcript(&mut store, &uploads, &manifest(bytes), bytes).unwrap(),
        result
    );
    store.commit_write_unit_of_work().unwrap();
    let mut changed = manifest(bytes);
    changed.transcript.messages[0].parts[0].created_at_ms = 0;
    assert!(publish_uploaded_transcript(&mut store, &uploads, &changed, bytes).is_err());
}
