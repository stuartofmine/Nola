// SPDX-License-Identifier: Apache-2.0
use nola_application::stage_archive_manifest;
use nola_domain::archive_restore::*;
use nola_store::{content_upload::ContentUploadStore, NolaStore};
use nola_store_port::{archive_restore::ArchiveRestoreRepository, content_upload::UploadOwner};

fn profile(restore: &str) -> StageArchiveManifest {
    StageArchiveManifest {
        restore_id: restore.into(),
        sequence: 0,
        entry: ArchiveRestoreManifestEntry::Profile {
            profile: ArchiveConversationProfile {
                id: "c".into(),
                title: "Archive".into(),
                settings_json: Some("{}".into()),
                created_at_ms: 1,
                updated_at_ms: 2,
            },
        },
    }
}
fn manifest(restore: &str, bytes: u64) -> StageArchiveManifest {
    serde_json::from_value(serde_json::json!({"restore_id":restore,"sequence":1,"entry":{"kind":"message","conversation_id":"c","message":{"id":"m","run_id":null,"role":"user","status":"completed","ordinal":1,"created_at_ms":1,"completed_at_ms":2,"parts":[{"id":"p","kind":"text","ordinal":1,"state":"completed","content_text":"","content_json":null,"content_digest":null,"created_at_ms":1,"updated_at_ms":2}]},"uploads":[{"field":"content_text","upload_id":"body","message_id":"m","part_id":"p","expected_utf8_bytes":bytes}]}})).unwrap()
}

#[test]
fn dual_field_uploads_stage_commit_reopen_and_reject_field_reinterpretation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dual.db");
    let mut store = NolaStore::open(&path).unwrap();
    let mut uploads = ContentUploadStore::open(&dir.path().join("uploads.db"), 1_000_000).unwrap();
    store
        .begin_archive_restore(&BeginArchiveRestore {
            restore_id: "restore".into(),
            expected_directory_revision: 0,
            created_at_ms: 1,
        })
        .unwrap();
    stage_archive_manifest(&mut store, &uploads, &profile("restore"), 1_000_000).unwrap();
    let owner = UploadOwner {
        conversation_id: "c".into(),
        import_id: "restore".into(),
        part_id: "p".into(),
    };
    let text = "原始 text";
    let json = " {\"seed\":9007199254740993,\"text\":\"原始\"} ";
    for (id, raw) in [("body", text), ("json-body", json)] {
        uploads.begin(id, &owner, raw.len() as u64).unwrap();
        uploads.append(id, &owner, 0, raw).unwrap();
        uploads.seal(id, &owner).unwrap();
    }
    let mut input = manifest("restore", text.len() as u64);
    if let ArchiveRestoreManifestEntry::Message {
        message, uploads, ..
    } = &mut input.entry
    {
        message.parts[0].kind = "tool_result".into();
        uploads.push(ArchivePartUpload {
            upload_id: "json-body".into(),
            message_id: "m".into(),
            part_id: "p".into(),
            field: nola_domain::PartBodyField::ContentJson,
            expected_utf8_bytes: json.len() as u64,
        });
    }
    let mut invalid_id = input.clone();
    if let ArchiveRestoreManifestEntry::Message { uploads, .. } = &mut invalid_id.entry {
        uploads[1].upload_id = " invalid-upload".into();
    }
    assert!(stage_archive_manifest(&mut store, &uploads, &invalid_id, 1_000_000).is_err());
    assert!(store
        .prepared_archive_reference("restore", "body")
        .unwrap()
        .is_none());
    let mut invalid = input.clone();
    if let ArchiveRestoreManifestEntry::Message { uploads, .. } = &mut invalid.entry {
        uploads[1].field = nola_domain::PartBodyField::ContentText;
    }
    assert!(stage_archive_manifest(&mut store, &uploads, &invalid, 1_000_000).is_err());
    assert!(
        store
            .prepared_archive_reference("restore", "body")
            .unwrap()
            .is_none(),
        "a bad second binding must be rejected before preparing the first body"
    );
    let progress = stage_archive_manifest(&mut store, &uploads, &input, 1_000_000).unwrap();
    // A previously prepared object cannot be reinterpreted as the other field.
    if let ArchiveRestoreManifestEntry::Message { uploads, .. } = &mut invalid.entry {
        uploads.remove(0);
    }
    assert!(stage_archive_manifest(&mut store, &uploads, &invalid, 1_000_000).is_err());
    assert_eq!(
        stage_archive_manifest(&mut store, &uploads, &input, 1_000_000).unwrap(),
        progress
    );
    store.begin_write_unit_of_work().unwrap();
    store
        .commit_archive_restore(&CommitArchiveRestore {
            restore_id: "restore".into(),
            expected_directory_revision: 0,
            expected_entry_count: 2,
            manifest_digest: progress.manifest_digest,
            committed_at_ms: 3,
        })
        .unwrap();
    store.commit_write_unit_of_work().unwrap();
    drop(store);
    let reopened = NolaStore::open(&path).unwrap();
    let part = &reopened.load_conversation("c").unwrap().unwrap().messages[0].parts[0];
    assert_eq!(part.content_text, text);
    assert_eq!(part.content_json.as_deref(), Some(json));
}

#[test]
fn malformed_json_upload_never_becomes_staged_history() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = NolaStore::open(dir.path().join("bad.db")).unwrap();
    let mut uploads = ContentUploadStore::open(&dir.path().join("uploads.db"), 1000).unwrap();
    store
        .begin_archive_restore(&BeginArchiveRestore {
            restore_id: "restore".into(),
            expected_directory_revision: 0,
            created_at_ms: 1,
        })
        .unwrap();
    let before = stage_archive_manifest(&mut store, &uploads, &profile("restore"), 1000).unwrap();
    let owner = UploadOwner {
        conversation_id: "c".into(),
        import_id: "restore".into(),
        part_id: "p".into(),
    };
    uploads.begin("body", &owner, 7).unwrap();
    uploads.append("body", &owner, 0, "{broken").unwrap();
    uploads.seal("body", &owner).unwrap();
    let mut input = manifest("restore", 7);
    if let ArchiveRestoreManifestEntry::Message { uploads, .. } = &mut input.entry {
        uploads[0].field = nola_domain::PartBodyField::ContentJson;
    }
    assert!(stage_archive_manifest(&mut store, &uploads, &input, 1000).is_err());
    assert_eq!(store.archive_restore_progress("restore").unwrap(), before);
    assert!(store.load_conversation("c").unwrap().is_none());
    assert!(store
        .prepared_archive_reference("restore", "body")
        .unwrap()
        .is_none());
    let mut wire = serde_json::to_value(input).unwrap();
    wire["entry"]["uploads"][0]
        .as_object_mut()
        .unwrap()
        .remove("field");
    assert!(serde_json::from_value::<StageArchiveManifest>(wire).is_err());
}

#[test]
fn staged_uploads_are_restore_owned_invisible_and_resume_exactly_after_final_commit() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("archive.db");
    let mut store = NolaStore::open(&path).unwrap();
    let mut uploads = ContentUploadStore::open(&dir.path().join("uploads.db"), 1_000_000).unwrap();
    let begin = BeginArchiveRestore {
        restore_id: "restore".into(),
        expected_directory_revision: 0,
        created_at_ms: 1,
    };
    store.begin_archive_restore(&begin).unwrap();
    stage_archive_manifest(&mut store, &uploads, &profile("restore"), 1_000_000).unwrap();
    assert!(store.load_conversation("c").unwrap().is_none());
    assert!(store
        .archive_restore_accepts_upload("restore", "c")
        .unwrap());
    let owner = UploadOwner {
        conversation_id: "c".into(),
        import_id: "restore".into(),
        part_id: "p".into(),
    };
    let text = "中😀\"\\".repeat(10_000);
    assert!(
        text.len() > 32768,
        "Preflight must not invent a single-chunk reference"
    );
    uploads.begin("body", &owner, text.len() as u64).unwrap();
    let mut offset = 0;
    while offset < text.len() {
        let mut end = (offset + 32768).min(text.len());
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        uploads
            .append("body", &owner, offset as u64, &text[offset..end])
            .unwrap();
        offset = end;
    }
    let input = manifest("restore", text.len() as u64);
    assert!(stage_archive_manifest(&mut store, &uploads, &input, 1_000_000).is_err());
    uploads.seal("body", &owner).unwrap();
    let progress = stage_archive_manifest(&mut store, &uploads, &input, 1_000_000).unwrap();
    assert_eq!(progress.next_sequence, 2);
    assert!(store.load_conversation("c").unwrap().is_none());
    assert_eq!(store.archive_directory_revision().unwrap(), 0);
    store
        .begin_archive_restore(&BeginArchiveRestore {
            restore_id: "foreign".into(),
            expected_directory_revision: 0,
            created_at_ms: 1,
        })
        .unwrap();
    stage_archive_manifest(&mut store, &uploads, &profile("foreign"), 1_000_000).unwrap();
    assert!(stage_archive_manifest(
        &mut store,
        &uploads,
        &manifest("foreign", text.len() as u64),
        1_000_000
    )
    .is_err());
    drop(store);
    let mut store = NolaStore::open(&path).unwrap();
    assert_eq!(
        stage_archive_manifest(&mut store, &uploads, &input, 1_000_000).unwrap(),
        progress
    );
    store.begin_write_unit_of_work().unwrap();
    store
        .commit_archive_restore(&CommitArchiveRestore {
            restore_id: "restore".into(),
            expected_directory_revision: 0,
            expected_entry_count: 2,
            manifest_digest: progress.manifest_digest,
            committed_at_ms: 3,
        })
        .unwrap();
    store.commit_write_unit_of_work().unwrap();
    assert_eq!(
        store.load_conversation("c").unwrap().unwrap().messages[0].parts[0].content_text,
        text
    );
    assert!(!store
        .archive_restore_accepts_upload("restore", "c")
        .unwrap());
    assert!(
        stage_archive_manifest(&mut store, &uploads, &input, 1_000_000)
            .unwrap()
            .completed
    );
    let mut changed = input;
    changed.sequence = 2;
    assert!(stage_archive_manifest(&mut store, &uploads, &changed, 1_000_000).is_err());
}

#[test]
fn external_archive_message_cannot_supply_a_canonical_reference_or_second_body_source() {
    let input = manifest("restore", 10_000_000);
    assert!(serde_json::to_vec(&input).unwrap().len() < 1024);
    let mut wire = serde_json::to_value(&input).unwrap();
    wire["entry"]["references"] = serde_json::json!([]);
    assert!(serde_json::from_value::<StageArchiveManifest>(wire).is_err());
    let mut store = NolaStore::open(":memory:").unwrap();
    let uploads = ContentUploadStore::open(std::path::Path::new(":memory:"), 100).unwrap();
    store
        .begin_archive_restore(&BeginArchiveRestore {
            restore_id: "restore".into(),
            expected_directory_revision: 0,
            created_at_ms: 1,
        })
        .unwrap();
    stage_archive_manifest(&mut store, &uploads, &profile("restore"), 100).unwrap();
    let mut input = manifest("restore", 10);
    if let ArchiveRestoreManifestEntry::Message { message, .. } = &mut input.entry {
        message.parts[0].content_text = "conflicting inline source".into();
    }
    assert!(stage_archive_manifest(&mut store, &uploads, &input, 100)
        .unwrap_err()
        .contains("field binding"));
    assert!(store.load_conversation("c").unwrap().is_none());
    assert_eq!(
        store
            .archive_restore_progress("restore")
            .unwrap()
            .next_sequence,
        1
    );
}
