// SPDX-License-Identifier: Apache-2.0
use super::*;
use nola_domain::{
    CanonicalBranchIntent, CanonicalBranchTarget, CanonicalPartOwner, PartDerivation,
};
use nola_store_port::part_derivation::PartDerivationProvenance;

fn record_fixture(path: &std::path::Path, body: &str) -> PartDerivation {
    record_fixture_fields(path, body, None)
}

fn record_fixture_fields(path: &std::path::Path, body: &str, json: Option<&str>) -> PartDerivation {
    let mut source = NolaStore::open(path).unwrap();
    source
        .create_conversation(&CreateConversationInput {
            id: "root".into(),
            title: "Root".into(),
            revision: 1,
            settings_json: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        })
        .unwrap();
    source
        .append_messages(&AppendMessagesInput {
            conversation_id: "root".into(),
            expected_message_count: 0,
            updated_at_ms: 2,
            messages: vec![MessageInput {
                id: "m".into(),
                run_id: Some("original-run".into()),
                role: "assistant".into(),
                status: "completed".into(),
                ordinal: 1,
                created_at_ms: 1,
                completed_at_ms: Some(2),
                parts: vec![MessagePartInput {
                    id: "p".into(),
                    kind: "text".into(),
                    ordinal: 1,
                    state: "completed".into(),
                    content_text: body.into(),
                    content_json: json.map(str::to_owned),
                    content_digest: Some("business-digest".into()),
                    created_at_ms: 1,
                    updated_at_ms: 2,
                }],
            }],
        })
        .unwrap();
    source
        .create_conversation_branch(&CanonicalBranchIntent {
            operation_id: "branch-op".into(),
            source_conversation_id: "root".into(),
            expected_source_revision: 2,
            through_message_id: "m".into(),
            conversation: CanonicalBranchTarget {
                id: "target".into(),
                title: "Target".into(),
                settings_json: None,
                created_at_ms: 3,
                updated_at_ms: 3,
            },
            committed_at_ms: 3,
        })
        .unwrap();
    let (message,part):(String,String)=source.connection.query_row("SELECT m.id,p.id FROM messages m JOIN message_parts p ON p.message_id=m.id WHERE m.conversation_id='target'",[],|row|Ok((row.get(0)?,row.get(1)?))).unwrap();
    let record = source
        .part_derivation_record(&CanonicalPartOwner::new("target".into(), message, part).unwrap()).map(|record| record.map(|record| record.derivation))
        .unwrap()
        .unwrap();
    if let Some(json) = json {
        for (field, expected) in [
            (nola_domain::PartBodyField::ContentText, body),
            (nola_domain::PartBodyField::ContentJson, json),
        ] {
            let reader = crate::canonical_field_reader::CanonicalFieldReader::open(
                &source.connection,
                "target",
                record.target.owner.message_id(),
                record.target.owner.part_id(),
                field,
            )
            .unwrap();
            assert!(
                reader.sealed_reference().is_some(),
                "large branch field must be independently sealed"
            );
            let mut exported = String::new();
            let mut offset = 0;
            while offset < expected.len() as u64 {
                let range = reader.read_range(offset, 32768).unwrap();
                exported.push_str(&range.text);
                offset = range.next_offset_bytes;
            }
            assert_eq!(exported, expected);
        }
    }
    source.delete_conversation("root").unwrap();
    source.delete_conversation("target").unwrap();
    record
}
fn stage_base(
    store: &mut NolaStore,
    record: &PartDerivation,
    body: &str,
) -> (u64, CanonicalBodyReference) {
    stage_base_fields(store, record, body, None)
}

fn stage_base_fields(
    store: &mut NolaStore,
    record: &PartDerivation,
    body: &str,
    json: Option<&str>,
) -> (u64, CanonicalBodyReference) {
    let revision = store.archive_directory_revision().unwrap();
    store
        .begin_archive_restore(&BeginArchiveRestore {
            restore_id: "archive".into(),
            expected_directory_revision: revision,
            created_at_ms: 5,
        })
        .unwrap();
    store
        .stage_archive_entry(
            "archive",
            0,
            &ArchiveRestoreEntry::Profile {
                profile: ArchiveConversationProfile {
                    id: "target".into(),
                    title: "Restored".into(),
                    settings_json: None,
                    created_at_ms: 3,
                    updated_at_ms: 5,
                },
            },
        )
        .unwrap();
    let owner = &record.target.owner;
    let chunks: Vec<String> = body
        .as_bytes()
        .chunks(32768)
        .map(|bytes| String::from_utf8(bytes.to_vec()).unwrap())
        .collect();
    let reference = CanonicalBodyReference::for_field(
        "archive-body".into(),
        owner.conversation_id().into(),
        owner.message_id().into(),
        owner.part_id().into(),
        nola_domain::PartBodyField::ContentText,
        body.len() as u64,
        chunks.len() as u32,
    )
    .unwrap();
    let mut offset = 0;
    store
        .prepare_archive_content(
            "archive",
            &reference,
            &mut chunks.into_iter().map(|text| {
                let result = Ok((offset, text.clone()));
                offset += text.len() as u64;
                result
            }),
            256 * 1024 * 1024,
        )
        .unwrap();
    let mut references = vec![reference.clone()];
    if let Some(json) = json {
        let chunks: Vec<_> = json
            .as_bytes()
            .chunks(32768)
            .map(|bytes| String::from_utf8(bytes.to_vec()).unwrap())
            .collect();
        let json_ref = CanonicalBodyReference::for_field(
            "archive-json".into(),
            owner.conversation_id().into(),
            owner.message_id().into(),
            owner.part_id().into(),
            nola_domain::PartBodyField::ContentJson,
            json.len() as u64,
            chunks.len() as u32,
        )
        .unwrap();
        let mut offset = 0;
        store
            .prepare_archive_content(
                "archive",
                &json_ref,
                &mut chunks.into_iter().map(|text| {
                    let at = offset;
                    offset += text.len() as u64;
                    Ok((at, text))
                }),
                256 * 1024 * 1024,
            )
            .unwrap();
        references.push(json_ref);
    }
    store
        .stage_archive_entry(
            "archive",
            1,
            &ArchiveRestoreEntry::Message {
                conversation_id: "target".into(),
                message: ArchiveMessage {
                    id: owner.message_id().into(),
                    run_id: record.target.run_id.clone(),
                    role: "assistant".into(),
                    status: "completed".into(),
                    ordinal: 1,
                    created_at_ms: 1,
                    completed_at_ms: Some(2),
                    parts: vec![MessagePartInput {
                        id: owner.part_id().into(),
                        kind: record.target.kind.clone(),
                        ordinal: 1,
                        state: record.target.state.clone(),
                        content_text: String::new(),
                        content_json: None,
                        content_digest: Some("business-digest".into()),
                        created_at_ms: 1,
                        updated_at_ms: 2,
                    }],
                },
                references,
            },
        )
        .unwrap();
    (revision, reference)
}
fn stage_claim(
    store: &mut NolaStore,
    record: &PartDerivation,
    revision: u64,
) -> CommitArchiveRestore {
    let progress = store
        .stage_archive_entry(
            "archive",
            2,
            &ArchiveRestoreEntry::PartDerivation {
                record: record.clone(),
            },
        )
        .unwrap();
    CommitArchiveRestore {
        restore_id: "archive".into(),
        expected_directory_revision: revision,
        expected_entry_count: progress.next_sequence,
        manifest_digest: progress.manifest_digest,
        committed_at_ms: 5,
    }
}
fn finish(
    store: &mut NolaStore,
    input: &CommitArchiveRestore,
) -> StoreResult<ArchiveRestoreResult> {
    store.begin_write_unit_of_work()?;
    match store.commit_archive_restore(input) {
        Ok(result) => {
            store.commit_write_unit_of_work()?;
            Ok(result)
        }
        Err(error) => {
            store.rollback_write_unit_of_work()?;
            Err(error)
        }
    }
}
fn proof_count(store: &NolaStore) -> i64 {
    store
        .connection
        .query_row("SELECT COUNT(*) FROM message_part_derivations", [], |row| {
            row.get(0)
        })
        .unwrap()
}

#[test]
fn two_field_branch_archive_preserves_fingerprints_and_wrong_json_rolls_back() {
    let root = tempfile::tempdir().unwrap();
    let body = "t".repeat(200_000);
    let json = format!(
        r#"{{"text":"{}","exact":9007199254740993}}"#,
        "j".repeat(200_000)
    );
    let record = record_fixture_fields(&root.path().join("source.db"), &body, Some(&json));
    for wrong_json in [false, true] {
        let path = root.path().join(format!("restored-{wrong_json}.db"));
        let mut store = NolaStore::open(&path).unwrap();
        let changed = json.replacen('j', "k", 1);
        let restored_json = if wrong_json {
            changed.as_str()
        } else {
            json.as_str()
        };
        let (revision, _) = stage_base_fields(&mut store, &record, &body, Some(restored_json));
        let input = stage_claim(&mut store, &record, revision);
        if wrong_json {
            assert!(finish(&mut store, &input).is_err());
            assert!(store.load_conversation("target").unwrap().is_none());
            assert_eq!(proof_count(&store), 0);
            assert_eq!(store.archive_directory_revision().unwrap(), revision);
            let bindings: i64 = store
                .connection
                .query_row(
                    "SELECT count(*) FROM message_part_body_references",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(bindings, 0);
            continue;
        }
        finish(&mut store, &input).unwrap();
        drop(store);
        let store = NolaStore::open(&path).unwrap();
        let restored = store
            .part_derivation_record(&record.target.owner)
            .unwrap()
            .unwrap();
        assert_eq!(
            restored.derivation, record,
            "both field fingerprints and actual branch origin survive"
        );
        assert_eq!(restored.provenance, PartDerivationProvenance::Archive);
        for (field, expected) in [
            (nola_domain::PartBodyField::ContentText, body.as_str()),
            (nola_domain::PartBodyField::ContentJson, json.as_str()),
        ] {
            let reader = crate::canonical_field_reader::CanonicalFieldReader::open(
                &store.connection,
                "target",
                record.target.owner.message_id(),
                record.target.owner.part_id(),
                field,
            )
            .unwrap();
            assert!(reader.sealed_reference().is_some());
            let mut offset = 0;
            while offset < expected.len() as u64 {
                let range = reader.read_range(offset, 32768).unwrap();
                assert_eq!(
                    range.text.as_bytes(),
                    &expected.as_bytes()[offset as usize..range.next_offset_bytes as usize]
                );
                offset = range.next_offset_bytes;
            }
        }
    }
}

#[test]
fn ten_megabyte_branch_claim_restores_after_all_sources_deleted_and_stays_archive() {
    let root = tempfile::tempdir().unwrap();
    let body = "z".repeat(10_000_000);
    let mut record = record_fixture(&root.path().join("original.db"), &body);
    // Root metadata is an untrusted archive claim, not an authorization shortcut.
    record.root_source.as_mut().unwrap().owner = CanonicalPartOwner::new(
        "claimed-root".into(),
        "claimed-message".into(),
        "claimed-part".into(),
    )
    .unwrap();
    let path = root.path().join("restored.db");
    let mut store = NolaStore::open(&path).unwrap();
    let (revision, reference) = stage_base(&mut store, &record, &body);
    let input = stage_claim(&mut store, &record, revision);
    assert_eq!(proof_count(&store), 0);
    assert!(store.load_conversation("target").unwrap().is_none());
    finish(&mut store, &input).unwrap();
    drop(store);
    let mut store = NolaStore::open(&path).unwrap();
    let restored = store
        .part_derivation_record(&record.target.owner)
        .unwrap()
        .unwrap();
    assert_eq!(restored.derivation, record);
    assert_eq!(restored.provenance, PartDerivationProvenance::Archive);
    let mut offset = 0;
    while offset < body.len() as u64 {
        let page = crate::published_part_body::field_range(
            &store.connection,
            "target",
            record.target.owner.message_id(),
            record.target.owner.part_id(),
            nola_domain::PartBodyField::ContentText,
            offset,
            32768,
        )
        .unwrap()
        .1;
        assert_eq!(
            page.text.as_bytes(),
            &body.as_bytes()[offset as usize..page.next_offset_bytes as usize]
        );
        offset = page.next_offset_bytes;
    }
    assert_eq!(reference.total_utf8_bytes(), body.len() as u64);
    finish(&mut store, &input).unwrap();
    assert_eq!(proof_count(&store), 1);
}

#[test]
fn stage_requires_previous_exact_target_and_rejects_duplicate_target_or_external_origin() {
    let root = tempfile::tempdir().unwrap();
    let record = record_fixture(&root.path().join("original.db"), "original");
    let mut store = NolaStore::open(root.path().join("target.db")).unwrap();
    let revision = store.archive_directory_revision().unwrap();
    store
        .begin_archive_restore(&BeginArchiveRestore {
            restore_id: "early".into(),
            expected_directory_revision: revision,
            created_at_ms: 5,
        })
        .unwrap();
    assert!(store
        .stage_archive_entry(
            "early",
            0,
            &ArchiveRestoreEntry::PartDerivation {
                record: record.clone()
            }
        )
        .is_err());
    let (revision, _) = stage_base(&mut store, &record, "original");
    let mut forged = record.clone();
    forged.target.owner = CanonicalPartOwner::new(
        "wrong".into(),
        record.target.owner.message_id().into(),
        record.target.owner.part_id().into(),
    )
    .unwrap();
    assert!(store
        .stage_archive_entry(
            "archive",
            2,
            &ArchiveRestoreEntry::PartDerivation { record: forged }
        )
        .is_err());
    let input = stage_claim(&mut store, &record, revision);
    assert_eq!(
        store
            .stage_archive_entry(
                "archive",
                2,
                &ArchiveRestoreEntry::PartDerivation {
                    record: record.clone()
                }
            )
            .unwrap()
            .next_sequence,
        3
    );
    assert!(store
        .stage_archive_entry(
            "archive",
            3,
            &ArchiveRestoreEntry::PartDerivation {
                record: record.clone()
            }
        )
        .is_err());
    assert_eq!(
        store
            .archive_restore_progress("archive")
            .unwrap()
            .next_sequence,
        3
    );
    let mut wire =
        serde_json::to_value(ArchiveRestoreManifestEntry::PartDerivation { record }).unwrap();
    wire["provenance"] = "local_branch".into();
    assert!(serde_json::from_value::<ArchiveRestoreManifestEntry>(wire).is_err());
    assert_eq!(input.expected_entry_count, 3);
    assert_eq!(proof_count(&store), 0);
}

#[test]
fn mismatched_bytes_and_late_failure_preserve_old_history_outbox_and_proof() {
    for failure in ["bytes", "proof", "late"] {
        let root = tempfile::tempdir().unwrap();
        let record = record_fixture(&root.path().join("original.db"), "original");
        let mut store = NolaStore::open(root.path().join("target.db")).unwrap();
        store
            .create_conversation(&CreateConversationInput {
                id: "old".into(),
                title: "Old".into(),
                revision: 1,
                settings_json: None,
                created_at_ms: 1,
                updated_at_ms: 1,
            })
            .unwrap();
        let (revision, _) = stage_base(
            &mut store,
            &record,
            if failure == "bytes" {
                "changed!"
            } else {
                "original"
            },
        );
        let input = stage_claim(&mut store, &record, revision);
        let before: i64 = store
            .connection
            .query_row("SELECT COUNT(*) FROM sync_outbox", [], |row| row.get(0))
            .unwrap();
        if failure == "proof" {
            store.connection.execute_batch("CREATE TRIGGER fail_archive_proof BEFORE INSERT ON message_part_derivations BEGIN SELECT RAISE(ABORT,'proof insert failure'); END").unwrap();
        }
        if failure == "late" {
            store.connection.execute_batch("CREATE TRIGGER fail_archive_fact BEFORE INSERT ON sync_outbox WHEN NEW.entity_type='archive_restore_commit' BEGIN SELECT RAISE(ABORT,'final outbox failure'); END").unwrap();
        }
        assert!(finish(&mut store, &input).is_err());
        assert!(store.load_conversation("old").unwrap().is_some());
        assert!(store.load_conversation("target").unwrap().is_none());
        assert_eq!(proof_count(&store), 0);
        assert_eq!(
            store
                .connection
                .query_row("SELECT COUNT(*) FROM sync_outbox", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            before
        );
        assert!(!store.archive_restore_progress("archive").unwrap().completed);
        if failure == "proof" {
            store
                .connection
                .execute_batch("DROP TRIGGER fail_archive_proof")
                .unwrap();
            finish(&mut store, &input).unwrap();
            assert_eq!(proof_count(&store), 1);
        }
        if failure == "late" {
            store
                .connection
                .execute_batch("DROP TRIGGER fail_archive_fact")
                .unwrap();
            finish(&mut store, &input).unwrap();
            assert_eq!(proof_count(&store), 1);
        }
    }
}

#[test]
fn archive_claim_cross_database_sync_is_staged_then_atomically_imported_without_origin_upgrade() {
    let root = tempfile::tempdir().unwrap();
    let record = record_fixture(&root.path().join("original.db"), &"x".repeat(100_000));
    let left = root.path().join("left.db");
    let right = root.path().join("right.db");
    let mut sender = NolaStore::open(&left).unwrap();
    let (revision, _) = stage_base(&mut sender, &record, &"x".repeat(100_000));
    let input = stage_claim(&mut sender, &record, revision);
    finish(&mut sender, &input).unwrap();
    drop(sender);
    let mut cursor = 0;
    let mut saw_claim = false;
    let mut saw_commit = false;
    for _ in 0..30 {
        let mut sender = NolaStore::open(&left).unwrap();
        let entries = sender.pending_sync_changes(2).unwrap();
        if entries.is_empty() {
            break;
        }
        let mut receiver = NolaStore::open(&right).unwrap();
        let page = ApplyRemoteSyncPageInput {
            replica_id: "archive-peer".into(),
            next_cursor: cursor + entries.len() as i64,
            applied_at_ms: 6,
            changes: entries
                .iter()
                .enumerate()
                .map(|(index, item)| RemoteSyncChangeInput {
                    cursor: cursor + index as i64 + 1,
                    operation_id: item.operation_id.clone(),
                    entity_type: item.entity_type.clone(),
                    entity_id: item.entity_id.clone(),
                    revision: item.revision,
                    deleted: item.deleted,
                    payload_json: item.payload_json.clone(),
                    created_at_ms: item.created_at_ms,
                })
                .collect(),
        };
        saw_claim |= entries.iter().any(|item| {
            item.entity_type == "archive_restore_entry"
                && item.payload_json.contains("part_derivation")
        });
        let final_page = entries
            .iter()
            .any(|item| item.entity_type == "archive_restore_commit");
        receiver.apply_remote_sync_page(&page).unwrap();
        cursor = page.next_cursor;
        assert_eq!(proof_count(&receiver), i64::from(final_page));
        if !final_page {
            assert!(receiver.load_conversation("target").unwrap().is_none());
        }
        for item in &entries {
            sender
                .resolve_sync_change(&item.operation_id, "acknowledged", Some(cursor), 6)
                .unwrap();
        }
        saw_commit |= final_page;
    }
    assert!(saw_claim && saw_commit);
    let receiver = NolaStore::open(&right).unwrap();
    let imported = receiver
        .part_derivation_record(&record.target.owner)
        .unwrap()
        .unwrap();
    assert_eq!(imported.derivation, record);
    assert_eq!(imported.provenance, PartDerivationProvenance::Archive);
    assert!(receiver.pending_sync_changes(10).unwrap().is_empty());
}

#[test]
fn maximum_escaped_metadata_shape_fits_separate_record_and_entry_budgets() {
    let directory = tempfile::tempdir().unwrap();
    let mut record = record_fixture(&directory.path().join("original.db"), "original");
    // Fill every bounded string with non-control escaped characters and both
    // fingerprints with maximum-width integer metadata. The fixed shape is
    // still below 16 KiB; do not relax field limits just to manufacture padding.
    // This is a DTO encoding-budget unit, not a canonical target fixture: actual
    // staging requires a declared owner and final rechecks real metadata/bytes.
    let value = "\\".repeat(128);
    record.operation_id = value.clone();
    for group in [&mut record.target, &mut record.immediate_source]
        .into_iter()
        .chain(record.root_source.iter_mut())
    {
        group.owner = CanonicalPartOwner::new(value.clone(), value.clone(), value.clone()).unwrap();
        group.run_id = Some(value.clone());
        group.kind = value.clone();
        group.state = value.clone();
        group.fields[0].as_mut().unwrap().total_utf8_bytes = u64::MAX;
        let mut json = group.fields[0].clone().unwrap();
        json.field = nola_domain::PartBodyField::ContentJson;
        group.fields[1] = Some(json);
    }
    let record_bytes = serde_json::to_vec(&record).unwrap().len();
    assert!(record_bytes < 16384);
    assert_eq!(
        serde_json::from_slice::<PartDerivation>(&serde_json::to_vec(&record).unwrap()).unwrap(),
        record
    );
    let entry = ArchiveRestoreEntry::PartDerivation { record };
    let encoded = validate_entry(&entry).unwrap();
    assert!(encoded.len() > record_bytes && encoded.len() <= 131072);
}
