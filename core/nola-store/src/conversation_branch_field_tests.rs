// SPDX-License-Identifier: Apache-2.0
//! Real branch publication, replica replay and owner-independent field lifetime.
use super::*;

fn seed(store: &mut NolaStore) -> (CanonicalBranchIntent, String, String) {
    store
        .create_conversation(&CreateConversationInput {
            id: "source".into(),
            title: "Source".into(),
            revision: 1,
            settings_json: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        })
        .unwrap();
    let text = "中😀".repeat(12_000);
    let json = serde_json::json!({"output": "界😀".repeat(50_000), "number": 123}).to_string();
    store
        .append_messages(&AppendMessagesInput {
            conversation_id: "source".into(),
            expected_message_count: 0,
            updated_at_ms: 2,
            messages: vec![MessageInput {
                id: "m".into(),
                run_id: None,
                role: "assistant".into(),
                status: "completed".into(),
                ordinal: 1,
                created_at_ms: 1,
                completed_at_ms: Some(2),
                parts: vec![MessagePartInput {
                    id: "p".into(),
                    kind: "tool_result".into(),
                    ordinal: 1,
                    state: "completed".into(),
                    content_text: text.clone(),
                    content_json: Some(json.clone()),
                    content_digest: None,
                    created_at_ms: 1,
                    updated_at_ms: 2,
                }],
            }],
        })
        .unwrap();
    (
        CanonicalBranchIntent {
            operation_id: "branch".into(),
            source_conversation_id: "source".into(),
            expected_source_revision: 2,
            through_message_id: "m".into(),
            conversation: nola_domain::CanonicalBranchTarget {
                id: "target".into(),
                title: "Target".into(),
                settings_json: None,
                created_at_ms: 3,
                updated_at_ms: 3,
            },
            committed_at_ms: 3,
        },
        text,
        json,
    )
}

fn assert_fields(store: &NolaStore, conversation: &str, text: &str, json: &str) {
    let conversation = store.load_conversation(conversation).unwrap().unwrap();
    let part = &conversation.messages[0].parts[0];
    assert_eq!(part.content_text, text);
    assert_eq!(part.content_json.as_deref(), Some(json));
}

fn replicate(source: &mut NolaStore, path: &std::path::Path) {
    let mut cursor = 0;
    loop {
        let entries = source.pending_sync_changes(19).unwrap();
        if entries.is_empty() {
            break;
        }
        let mut page = ApplyRemoteSyncPageInput {
            replica_id: "branch-replica".into(),
            next_cursor: cursor + entries.len() as i64,
            applied_at_ms: 4,
            changes: entries
                .iter()
                .enumerate()
                .map(|(index, entry)| RemoteSyncChangeInput {
                    cursor: cursor + index as i64 + 1,
                    operation_id: entry.operation_id.clone(),
                    entity_type: entry.entity_type.clone(),
                    entity_id: entry.entity_id.clone(),
                    revision: entry.revision,
                    deleted: entry.deleted,
                    payload_json: entry.payload_json.clone(),
                    created_at_ms: entry.created_at_ms,
                })
                .collect(),
        };
        let mut target = NolaStore::open(path).unwrap();
        cursor = target.apply_remote_sync_page(&page).unwrap().cursor;
        // Transport delivery cursors advance even when operation receipts are
        // redelivered. Keep identities/payloads exact, not the old cursor range.
        for (index, change) in page.changes.iter_mut().enumerate() {
            change.cursor = cursor + index as i64 + 1;
        }
        page.next_cursor = cursor + page.changes.len() as i64;
        cursor = target.apply_remote_sync_page(&page).unwrap().cursor;
        for entry in entries {
            source
                .resolve_sync_change(&entry.operation_id, "acknowledged", Some(cursor), 4)
                .unwrap();
        }
    }
}

#[test]
fn dual_fields_branch_replica_cold_restart_and_source_deletion() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("source.db");
    let mut store = NolaStore::open(&path).unwrap();
    let (input, text, json) = seed(&mut store);
    store.create_conversation_branch(&input).unwrap();
    let refs: Vec<String> = store
        .connection
        .prepare("SELECT reference_json FROM canonical_text_objects ORDER BY id")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let refs: Vec<CanonicalBodyReference> = refs
        .iter()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    let target_refs: Vec<_> = refs
        .iter()
        .filter(|r| r.conversation_id() == "target")
        .collect();
    assert_eq!(target_refs.len(), 2);
    assert_ne!(target_refs[0].content_id(), target_refs[1].content_id());
    assert_ne!(target_refs[0].field(), target_refs[1].field());
    let replica = dir.path().join("replica.db");
    replicate(&mut store, &replica);
    assert_fields(&NolaStore::open(&replica).unwrap(), "target", &text, &json);
    store.delete_conversation("source").unwrap();
    drop(store);
    let mut store = NolaStore::open(&path).unwrap();
    assert_fields(&store, "target", &text, &json);
    // A second branch reads both sealed source fields, not empty inline slots.
    let first = store.load_conversation("target").unwrap().unwrap();
    let mut next = input;
    next.operation_id = "branch-again".into();
    next.source_conversation_id = "target".into();
    next.expected_source_revision = first.revision;
    next.through_message_id = first.messages[0].id.clone();
    next.conversation.id = "third".into();
    store.create_conversation_branch(&next).unwrap();
    assert_fields(&store, "third", &text, &json);
}

fn counts(store: &NolaStore) -> (i64, i64, i64) {
    store.connection.query_row(
        "SELECT (SELECT COUNT(*) FROM canonical_text_objects), (SELECT COUNT(*) FROM sync_outbox),
         (SELECT COUNT(*) FROM canonical_text_chunks)", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    ).unwrap()
}

#[test]
fn second_field_failure_rolls_back_first_field_and_all_publication() {
    let mut store = NolaStore::open(":memory:").unwrap();
    let (input, text, json) = seed(&mut store);
    let before = counts(&store);
    store
        .connection
        .execute_batch(
            "CREATE TRIGGER reject_branch_json BEFORE INSERT ON canonical_text_objects
        WHEN json_extract(NEW.reference_json,'$.conversation_id')='target'
         AND json_extract(NEW.reference_json,'$.field')='content_json'
        BEGIN SELECT RAISE(ABORT,'second field failure'); END",
        )
        .unwrap();
    assert!(store.create_conversation_branch(&input).is_err());
    assert!(store.load_conversation("target").unwrap().is_none());
    assert_eq!(counts(&store), before);
    store
        .connection
        .execute_batch("DROP TRIGGER reject_branch_json")
        .unwrap();
    store.create_conversation_branch(&input).unwrap();
    assert_fields(&store, "target", &text, &json);
}

#[test]
fn missing_second_source_field_chunk_rolls_back_new_branch() {
    let mut store = NolaStore::open(":memory:").unwrap();
    let (mut input, _, _) = seed(&mut store);
    store.create_conversation_branch(&input).unwrap();
    let first = store.load_conversation("target").unwrap().unwrap();
    let json_id = identity("branch", "body-json", "p");
    store.connection.execute(
        "DELETE FROM canonical_text_chunks WHERE content_id=?1 AND offset_bytes=(SELECT MAX(offset_bytes) FROM canonical_text_chunks WHERE content_id=?1)",
        params![json_id],
    ).unwrap();
    let before = counts(&store);
    input.operation_id = "broken-copy".into();
    input.source_conversation_id = "target".into();
    input.expected_source_revision = first.revision;
    input.through_message_id = first.messages[0].id.clone();
    input.conversation.id = "third".into();
    assert!(store.create_conversation_branch(&input).is_err());
    assert!(store.load_conversation("third").unwrap().is_none());
    assert_eq!(counts(&store), before);
}

#[test]
fn sealed_geometry_corruption_cannot_be_republished_as_a_valid_branch() {
    for field in [
        nola_domain::PartBodyField::ContentText,
        nola_domain::PartBodyField::ContentJson,
    ] {
        for extra_tail in [false, true] {
            let mut store = NolaStore::open(":memory:").unwrap();
            let (mut input, _, _) = seed(&mut store);
            store.create_conversation_branch(&input).unwrap();
            let first = store.load_conversation("target").unwrap().unwrap();
            let id = identity(
                "branch",
                if field == nola_domain::PartBodyField::ContentText {
                    "body"
                } else {
                    "body-json"
                },
                "p",
            );
            let raw: String = store
                .connection
                .query_row(
                    "SELECT reference_json FROM canonical_text_objects WHERE id=?1",
                    params![id],
                    |r| r.get(0),
                )
                .unwrap();
            let reference: CanonicalBodyReference = serde_json::from_str(&raw).unwrap();
            if extra_tail {
                store
                    .connection
                    .execute(
                        "INSERT INTO canonical_text_chunks VALUES(?1,?2,'x')",
                        params![id, reference.total_utf8_bytes()],
                    )
                    .unwrap();
            } else {
                // Keep owner bindings internally equal and every byte readable;
                // only the declared chunk geometry is false.
                let changed = CanonicalBodyReference::for_field(
                    reference.content_id().into(),
                    reference.conversation_id().into(),
                    reference.message_id().into(),
                    reference.part_id().into(),
                    field,
                    reference.total_utf8_bytes(),
                    reference.chunk_count() + 1,
                )
                .unwrap();
                let encoded = serde_json::to_string(&changed).unwrap();
                store
                    .connection
                    .execute(
                        "UPDATE canonical_text_objects SET reference_json=?1 WHERE id=?2",
                        params![encoded, id],
                    )
                    .unwrap();
                store.connection.execute("UPDATE message_part_body_references SET reference_json=?1 WHERE part_id=?2 AND field=?3",
                    params![encoded, reference.part_id(), if field == nola_domain::PartBodyField::ContentText { "content_text" } else { "content_json" }]).unwrap();
            }
            let before = counts(&store);
            input.operation_id = "invalid-geometry".into();
            input.source_conversation_id = "target".into();
            input.expected_source_revision = first.revision;
            input.through_message_id = first.messages[0].id.clone();
            input.conversation.id = "third".into();
            assert!(
                store.create_conversation_branch(&input).is_err(),
                "field={field:?}, extra_tail={extra_tail}"
            );
            assert!(store.load_conversation("third").unwrap().is_none());
            assert_eq!(counts(&store), before);
        }
    }
}
