// SPDX-License-Identifier: Apache-2.0
//! Archive source bytes survive restore without recreating local execution trust.

use nola_application::stage_archive_manifest;
use nola_domain::archive_restore::*;
use nola_domain::tool_source::{ToolSourceClaim, ToolSourceKind, ToolSourceOriginalOwner};
use nola_domain::{CanonicalBodyReference, CanonicalPartOwner, MessagePartInput, PartBodyField};
use nola_store::{content_upload::ContentUploadStore, NolaStore};
use nola_store_port::archive_restore::ArchiveRestoreRepository;
use nola_store_port::content_upload::UploadOwner;
use nola_store_port::replica_sync::{ApplyRemoteSyncPageInput, RemoteSyncChangeInput};
use nola_store_port::tool_source::{ToolSourceProvenance, ToolSourceRepository};
use sha2::{Digest, Sha256};

fn owner() -> CanonicalPartOwner {
    CanonicalPartOwner::new("c".into(), "m".into(), "p".into()).unwrap()
}

fn chunks(raw: &str) -> Vec<(u64, String)> {
    let mut chunks = Vec::new();
    let mut offset = 0;
    while offset < raw.len() {
        let mut end = (offset + 32768).min(raw.len());
        while !raw.is_char_boundary(end) {
            end -= 1;
        }
        chunks.push((offset as u64, raw[offset..end].into()));
        offset = end;
    }
    chunks
}

fn source(kind: ToolSourceKind) -> (ToolSourceClaim, String) {
    let id = match kind {
        ToolSourceKind::ToolCall => "original-call-item",
        ToolSourceKind::ToolResult => "original-result-item",
    };
    let value = match kind {
        ToolSourceKind::ToolCall => serde_json::json!({
            "type":"tool_call", "id":id, "call_id":"actual-call", "name":"read_file",
            "arguments":{"path":"isolated.txt"}
        }),
        ToolSourceKind::ToolResult => serde_json::json!({
            "type":"tool_result", "id":id, "call_id":"actual-call", "name":"read_file",
            "output":{"text":"原文🙂\n".repeat(8_000)}, "is_error":false,
            "full_output_ref":null, "artifacts":[]
        }),
    };
    let raw = format!(" \n{}\n", serde_json::to_string(&value).unwrap());
    let claim = ToolSourceClaim {
        schema_version: 1,
        source_item_id: id.into(),
        kind,
        call_id: "actual-call".into(),
        name: "read_file".into(),
        owner: owner(),
        original_owner: ToolSourceOriginalOwner {
            part_owner: owner(),
            run_id: "original-run".into(),
            turn_id: "original-turn".into(),
        },
        body: CanonicalBodyReference::for_field(
            format!("client-must-not-select-{id}"),
            "c".into(),
            "m".into(),
            "p".into(),
            PartBodyField::ContentJson,
            raw.len() as u64,
            chunks(&raw).len() as u32,
        )
        .unwrap(),
        body_sha256: format!("{:x}", Sha256::digest(raw.as_bytes())),
        derivation: None,
    };
    (claim, raw)
}

fn stage_base(store: &mut NolaStore, uploads: &ContentUploadStore) -> u64 {
    let directory = store.archive_directory_revision().unwrap();
    store
        .begin_archive_restore(&BeginArchiveRestore {
            restore_id: "restore".into(),
            expected_directory_revision: directory,
            created_at_ms: 1,
        })
        .unwrap();
    let entries = [
        ArchiveRestoreManifestEntry::Profile {
            profile: ArchiveConversationProfile {
                id: "c".into(),
                title: "Archive".into(),
                settings_json: None,
                created_at_ms: 1,
                updated_at_ms: 2,
            },
        },
        ArchiveRestoreManifestEntry::Message {
            conversation_id: "c".into(),
            message: ArchiveMessage {
                id: "m".into(),
                run_id: Some("original-run".into()),
                role: "assistant".into(),
                status: "completed".into(),
                ordinal: 1,
                created_at_ms: 1,
                completed_at_ms: Some(2),
                parts: vec![MessagePartInput {
                    id: "p".into(),
                    kind: "tool".into(),
                    ordinal: 1,
                    state: "completed".into(),
                    content_text: "Visible presentation".into(),
                    content_json: Some("{}".into()),
                    content_digest: None,
                    created_at_ms: 1,
                    updated_at_ms: 2,
                }],
            },
            uploads: vec![],
        },
    ];
    for (sequence, entry) in entries.into_iter().enumerate() {
        stage_archive_manifest(
            store,
            uploads,
            &StageArchiveManifest {
                restore_id: "restore".into(),
                sequence: sequence as u64,
                entry,
            },
            1_000_000,
        )
        .unwrap();
    }
    directory
}

fn upload(uploads: &mut ContentUploadStore, id: &str, raw: &str) {
    let owner = UploadOwner {
        conversation_id: "c".into(),
        import_id: "restore".into(),
        part_id: "p".into(),
    };
    uploads.begin(id, &owner, raw.len() as u64).unwrap();
    for (offset, text) in chunks(raw) {
        uploads.append(id, &owner, offset, &text).unwrap();
    }
    uploads.seal(id, &owner).unwrap();
}

fn stage_source(
    store: &mut NolaStore,
    uploads: &ContentUploadStore,
    claim: ToolSourceClaim,
    upload_id: &str,
    sequence: u64,
) -> Result<ArchiveRestoreProgress, String> {
    stage_archive_manifest(
        store,
        uploads,
        &StageArchiveManifest {
            restore_id: "restore".into(),
            sequence,
            entry: ArchiveRestoreManifestEntry::ToolSource {
                claim,
                upload_id: upload_id.into(),
            },
        },
        1_000_000,
    )
}

fn commit(
    store: &mut NolaStore,
    directory: u64,
    progress: &ArchiveRestoreProgress,
) -> CommitArchiveRestore {
    let input = CommitArchiveRestore {
        restore_id: "restore".into(),
        expected_directory_revision: directory,
        expected_entry_count: progress.next_sequence,
        manifest_digest: progress.manifest_digest.clone(),
        committed_at_ms: 3,
    };
    store.begin_write_unit_of_work().unwrap();
    store.commit_archive_restore(&input).unwrap();
    store.commit_write_unit_of_work().unwrap();
    input
}

#[test]
fn archive_tool_source_upload_restores_exact_originals_as_untrusted_history_after_cold_open() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("archive.sqlite");
    let mut store = NolaStore::open(&path).unwrap();
    let mut uploads =
        ContentUploadStore::open(&directory.path().join("uploads.sqlite"), 1_000_000).unwrap();
    let revision = stage_base(&mut store, &uploads);
    let call = source(ToolSourceKind::ToolCall);
    let result = source(ToolSourceKind::ToolResult);
    upload(&mut uploads, "call-upload", &call.1);
    upload(&mut uploads, "result-upload", &result.1);
    stage_source(&mut store, &uploads, call.0.clone(), "call-upload", 2).unwrap();
    let progress =
        stage_source(&mut store, &uploads, result.0.clone(), "result-upload", 3).unwrap();
    assert!(store.load_conversation("c").unwrap().is_none());
    let input = commit(&mut store, revision, &progress);
    store.begin_write_unit_of_work().unwrap();
    store.commit_archive_restore(&input).unwrap();
    store.commit_write_unit_of_work().unwrap();
    drop(store);
    let cold = NolaStore::open(&path).unwrap();
    let conversation = cold.load_conversation("c").unwrap().unwrap();
    assert_eq!(
        conversation.messages[0].parts[0].content_json.as_deref(),
        Some("{}")
    );
    let bindings = cold
        .tool_sources_for_part(&owner(), conversation.revision)
        .unwrap();
    assert_eq!(bindings.len(), 2);
    for (index, (binding, (claim, expected))) in bindings.iter().zip([call, result]).enumerate() {
        assert_eq!(binding.source_item_id, claim.source_item_id);
        assert_eq!(binding.body_sha256, claim.body_sha256);
        assert_ne!(binding.body.content_id(), claim.body.content_id());
        match &binding.provenance {
            ToolSourceProvenance::Archive {
                claim: imported,
                restore_id,
                sequence,
            } => {
                assert_eq!(restore_id, "restore");
                assert_eq!(*sequence, index as u64 + 2);
                assert_eq!(imported.original_owner, claim.original_owner);
                assert_eq!(imported.body, binding.body);
            }
            other => panic!("archive cannot recreate local accepted authority: {other:?}"),
        }
        let mut raw = String::new();
        let mut offset = 0;
        loop {
            let page = cold
                .read_tool_source_range(binding, conversation.revision, offset, 4099)
                .unwrap();
            assert_eq!(page.offset_bytes, offset);
            assert_eq!(page.next_offset_bytes, offset + page.text.len() as u64);
            raw.push_str(&page.text);
            if page.end_of_content {
                break;
            }
            assert!(page.next_offset_bytes > offset);
            offset = page.next_offset_bytes;
        }
        assert_eq!(raw, expected);
    }
}

#[test]
fn archive_tool_source_bad_upload_digest_is_rejected_before_preparing_any_body() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("bad.sqlite");
    let mut store = NolaStore::open(&path).unwrap();
    let mut uploads =
        ContentUploadStore::open(&directory.path().join("uploads.sqlite"), 1_000_000).unwrap();
    stage_base(&mut store, &uploads);
    let (mut claim, raw) = source(ToolSourceKind::ToolCall);
    upload(&mut uploads, "call-upload", &raw);
    let before = store.archive_restore_progress("restore").unwrap();
    claim.body_sha256 = "0".repeat(64);
    assert!(stage_source(&mut store, &uploads, claim, "call-upload", 2).is_err());
    assert_eq!(store.archive_restore_progress("restore").unwrap(), before);
    let sql = rusqlite::Connection::open(&path).unwrap();
    let counts: (i64, i64) = sql
        .query_row(
            "SELECT (SELECT COUNT(*) FROM canonical_text_objects),
                (SELECT COUNT(*) FROM archive_restore_prepared)",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(counts, (0, 0));
    assert!(store.load_conversation("c").unwrap().is_none());
}

#[test]
fn archive_tool_source_install_fault_rolls_back_the_entire_visible_replacement() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("rollback.sqlite");
    let mut store = NolaStore::open(&path).unwrap();
    store
        .create_conversation(&nola_store::CreateConversationInput {
            id: "before".into(),
            title: "Must survive".into(),
            revision: 1,
            settings_json: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        })
        .unwrap();
    let mut uploads =
        ContentUploadStore::open(&directory.path().join("uploads.sqlite"), 1_000_000).unwrap();
    let revision = stage_base(&mut store, &uploads);
    let (claim, raw) = source(ToolSourceKind::ToolCall);
    upload(&mut uploads, "call-upload", &raw);
    let progress = stage_source(&mut store, &uploads, claim, "call-upload", 2).unwrap();
    let sql = rusqlite::Connection::open(&path).unwrap();
    sql.execute_batch(
        "CREATE TRIGGER reject_archive_source BEFORE INSERT ON canonical_tool_sources
        BEGIN SELECT RAISE(ABORT,'fixture archive source fault'); END;",
    )
    .unwrap();
    let input = CommitArchiveRestore {
        restore_id: "restore".into(),
        expected_directory_revision: revision,
        expected_entry_count: 3,
        manifest_digest: progress.manifest_digest.clone(),
        committed_at_ms: 3,
    };
    store.begin_write_unit_of_work().unwrap();
    assert!(store.commit_archive_restore(&input).is_err());
    store.rollback_write_unit_of_work().unwrap();
    assert_eq!(
        store.load_conversation("before").unwrap().unwrap().title,
        "Must survive"
    );
    assert!(store.load_conversation("c").unwrap().is_none());
    assert_eq!(store.archive_directory_revision().unwrap(), revision);
    assert_eq!(store.archive_restore_progress("restore").unwrap(), progress);
    assert_eq!(
        sql.query_row("SELECT COUNT(*) FROM canonical_tool_sources", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    sql.execute_batch("DROP TRIGGER reject_archive_source")
        .unwrap();
    commit(&mut store, revision, &progress);
    assert!(store.load_conversation("before").unwrap().is_none());
    let current = store.load_conversation("c").unwrap().unwrap().revision;
    assert_eq!(
        store
            .tool_sources_for_part(&owner(), current)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn archive_tool_source_sync_waits_for_body_seals_and_keeps_receiver_claim_provenance() {
    let directory = tempfile::tempdir().unwrap();
    let mut source_store = NolaStore::open(directory.path().join("source.sqlite")).unwrap();
    let destination_path = directory.path().join("destination.sqlite");
    let mut destination = NolaStore::open(&destination_path).unwrap();
    let mut uploads =
        ContentUploadStore::open(&directory.path().join("uploads.sqlite"), 1_000_000).unwrap();
    let revision = stage_base(&mut source_store, &uploads);
    let (claim, raw) = source(ToolSourceKind::ToolCall);
    upload(&mut uploads, "call-upload", &raw);
    let progress =
        stage_source(&mut source_store, &uploads, claim.clone(), "call-upload", 2).unwrap();
    commit(&mut source_store, revision, &progress);
    let first = source_store.pending_sync_changes(100).unwrap();
    assert!(first
        .iter()
        .any(|fact| fact.entity_type == "canonical_text_seal"));
    assert!(first
        .iter()
        .any(|fact| fact.entity_type == "archive_restore_entry"
            && serde_json::from_str::<serde_json::Value>(&fact.payload_json).unwrap()["entry"]
                ["kind"]
                == "tool_source"));
    assert!(
        !first
            .iter()
            .any(|fact| fact.entity_type == "archive_restore_commit"),
        "unacknowledged source seals must fence final visibility"
    );
    let mut cursor = 0;
    let mut committed = false;
    for _ in 0..16 {
        let page = source_store.pending_sync_changes(100).unwrap();
        if page.is_empty() {
            break;
        }
        let next_cursor = cursor + page.len() as i64;
        destination
            .apply_remote_sync_page(&ApplyRemoteSyncPageInput {
                replica_id: "archive-replica".into(),
                next_cursor,
                applied_at_ms: 10,
                changes: page
                    .iter()
                    .enumerate()
                    .map(|(index, fact)| RemoteSyncChangeInput {
                        cursor: cursor + index as i64 + 1,
                        operation_id: fact.operation_id.clone(),
                        entity_type: fact.entity_type.clone(),
                        entity_id: fact.entity_id.clone(),
                        revision: fact.revision,
                        deleted: fact.deleted,
                        payload_json: fact.payload_json.clone(),
                        created_at_ms: fact.created_at_ms,
                    })
                    .collect(),
            })
            .unwrap();
        committed |= page
            .iter()
            .any(|fact| fact.entity_type == "archive_restore_commit");
        if !committed {
            assert!(destination.load_conversation("c").unwrap().is_none());
        }
        for fact in page {
            assert!(source_store
                .resolve_sync_change(&fact.operation_id, "acknowledged", Some(next_cursor), 10)
                .unwrap());
        }
        cursor = next_cursor;
    }
    assert!(committed);
    assert!(source_store.pending_sync_changes(100).unwrap().is_empty());
    drop(destination);
    let cold = NolaStore::open(&destination_path).unwrap();
    let current = cold.load_conversation("c").unwrap().unwrap().revision;
    let binding = cold
        .tool_source_by_id("c", &claim.source_item_id, current)
        .unwrap()
        .unwrap();
    assert!(matches!(
        binding.provenance,
        ToolSourceProvenance::Archive { .. }
    ));
    assert_eq!(
        cold.read_tool_source_range(&binding, current, 0, 32768)
            .unwrap()
            .text,
        raw
    );
}
