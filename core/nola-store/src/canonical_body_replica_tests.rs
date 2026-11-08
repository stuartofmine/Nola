// SPDX-License-Identifier: Apache-2.0
//! Real SQLite-to-SQLite immutable JSON preparation, without visible Part bindings.
use super::*;
use nola_domain::{CanonicalBodyReference, PartBodyField};
use nola_store_port::canonical_body::CanonicalBodyRepository;

fn reference(id: &str, chunks: &[String]) -> CanonicalBodyReference {
    CanonicalBodyReference::for_field(
        id.into(),
        "c".into(),
        "m".into(),
        "p".into(),
        PartBodyField::ContentJson,
        chunks.iter().map(|s| s.len() as u64).sum(),
        chunks.len() as u32,
    )
    .unwrap()
}
fn facts(reference: &CanonicalBodyReference, chunks: &[String]) -> Vec<RemoteSyncChangeInput> {
    let mut offset = 0;
    let mut output = Vec::new();
    for text in chunks {
        let entity_id = chunk_id(reference, offset);
        output.push(RemoteSyncChangeInput {
            cursor: 0,
            operation_id: format!("chunk:{entity_id}"),
            entity_type: "canonical_text_chunk".into(),
            entity_id,
            revision: 1,
            deleted: false,
            payload_json: serde_json::to_string(&ChunkFact {
                reference: reference.clone(),
                offset_bytes: offset,
                text: text.clone(),
            })
            .unwrap(),
            created_at_ms: 1,
        });
        offset += text.len() as u64;
    }
    output.push(RemoteSyncChangeInput {
        cursor: 0,
        operation_id: format!("seal:{}", reference.content_id()),
        entity_type: "canonical_text_seal".into(),
        entity_id: reference.content_id().into(),
        revision: 1,
        deleted: false,
        payload_json: serde_json::to_string(reference).unwrap(),
        created_at_ms: 1,
    });
    output
}
fn receive(
    store: &mut NolaStore,
    facts: &[RemoteSyncChangeInput],
    cursor: i64,
) -> StoreResult<ApplyRemoteSyncPageResult> {
    let mut changes = facts.to_vec();
    for (index, fact) in changes.iter_mut().enumerate() {
        fact.cursor = cursor + index as i64 + 1;
    }
    store.apply_remote_sync_page(&ApplyRemoteSyncPageInput {
        replica_id: "json-peer".into(),
        next_cursor: cursor + changes.len() as i64,
        applied_at_ms: 2,
        changes,
    })
}
fn count(store: &NolaStore, table: &str) -> i64 {
    store
        .connection
        .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
}

#[test]
fn json_enqueued_original_chunks_survive_cross_database_page_restarts() {
    let directory = tempfile::tempdir().unwrap();
    let source_path = directory.path().join("source.db");
    let target_path = directory.path().join("target.db");
    let original = format!(
        " {{\"z\":\"{}\",\"a\":[null,true,1e2]}}\n",
        "界😀\\\"\\\\".repeat(900_000)
    );
    let mut chunks = Vec::new();
    let mut offset = 0;
    while offset < original.len() {
        let mut end = (offset + 30_001).min(original.len());
        while !original.is_char_boundary(end) {
            end -= 1;
        }
        chunks.push(original[offset..end].to_owned());
        offset = end;
    }
    let reference = reference("json-original", &chunks);
    let mut source = NolaStore::open(&source_path).unwrap();
    let mut offset = 0;
    source
        .prepare_body(
            &reference,
            &mut chunks.iter().map(|chunk| {
                let result = Ok((offset, chunk.clone()));
                offset += chunk.len() as u64;
                result
            }),
            MAX_REPLICA_CONTENT_BYTES,
        )
        .unwrap();
    enqueue(&source.connection, &reference, 1).unwrap();
    drop(source);
    let mut cursor = 0;
    loop {
        let mut source = NolaStore::open(&source_path).unwrap();
        let pending = source.pending_sync_changes(29).unwrap();
        if pending.is_empty() {
            break;
        }
        let changes: Vec<_> = pending
            .iter()
            .map(|entry| RemoteSyncChangeInput {
                cursor: 0,
                operation_id: entry.operation_id.clone(),
                entity_type: entry.entity_type.clone(),
                entity_id: entry.entity_id.clone(),
                revision: entry.revision,
                deleted: entry.deleted,
                payload_json: entry.payload_json.clone(),
                created_at_ms: entry.created_at_ms,
            })
            .collect();
        let mut target = NolaStore::open(&target_path).unwrap();
        cursor = receive(&mut target, &changes, cursor).unwrap().cursor;
        assert_eq!(count(&target, "message_parts"), 0);
        for item in &pending {
            source
                .resolve_sync_change(&item.operation_id, "acknowledged", Some(cursor), 3)
                .unwrap();
        }
    }
    let target = NolaStore::open(&target_path).unwrap();
    require_complete(&target.connection, &reference).unwrap();
    let mut offset = 0;
    while offset < original.len() as u64 {
        let range = target
            .read_prepared_body_range(&reference, offset, 32_768)
            .unwrap();
        assert_eq!(
            range.text.as_bytes(),
            &original.as_bytes()[offset as usize..range.next_offset_bytes as usize]
        );
        assert!(range.next_offset_bytes > offset);
        offset = range.next_offset_bytes;
    }
    assert!(target.pending_sync_changes(10).unwrap().is_empty());
    assert_eq!(count(&target, "message_part_body_references"), 0);
}

#[test]
fn invalid_json_tail_rolls_back_last_chunk_seal_and_page_cursor() {
    let mut store = NolaStore::open(":memory:").unwrap();
    let chunks = vec!["{\"x\":\"".into(), "tail\"}garbage".into()];
    let reference = reference("invalid", &chunks);
    let input = facts(&reference, &chunks);
    receive(&mut store, &input[..1], 0).unwrap();
    assert!(receive(&mut store, &input[1..], 1).is_err());
    assert_eq!(store.sync_cursor("json-peer").unwrap().unwrap().cursor, 1);
    assert_eq!(count(&store, "canonical_text_replica_chunks"), 1);
    assert_eq!(count(&store, "canonical_text_objects"), 0);
    assert_eq!(count(&store, "canonical_text_chunks"), 0);
    assert_eq!(count(&store, "message_parts"), 0);
}

#[test]
fn json_field_swap_and_changed_operation_replay_are_rejected() {
    let mut store = NolaStore::open(":memory:").unwrap();
    let chunks = vec!["null".into()];
    let reference = reference("bound", &chunks);
    let input = facts(&reference, &chunks);
    receive(&mut store, &input, 0).unwrap();
    for index in 0..2 {
        let mut altered = input[index].clone();
        let mut payload: serde_json::Value = serde_json::from_str(&altered.payload_json).unwrap();
        if index == 0 {
            payload["reference"]["field"] = "content_text".into();
        } else {
            payload["field"] = "content_text".into();
        }
        altered.payload_json = payload.to_string();
        assert!(receive(&mut store, &[altered], 2).is_err());
    }
    let mut altered = input[0].clone();
    let mut payload: serde_json::Value = serde_json::from_str(&altered.payload_json).unwrap();
    payload["text"] = "true".into();
    altered.payload_json = payload.to_string();
    assert!(receive(&mut store, &[altered], 2).is_err());
    assert_eq!(store.sync_cursor("json-peer").unwrap().unwrap().cursor, 2);
    assert_eq!(
        store
            .read_prepared_body_range(&reference, 0, 10)
            .unwrap()
            .text,
        "null"
    );
}

#[test]
fn existing_and_same_operation_seals_revalidate_json_even_when_bytes_match() {
    let mut store = NolaStore::open(":memory:").unwrap();
    let chunks = vec!["null".into()];
    let reference = reference("corrupt", &chunks);
    let input = facts(&reference, &chunks);
    receive(&mut store, &input, 0).unwrap();
    // Same length and identical canonical/staging bytes, so geometry and equality alone pass.
    store
        .connection
        .execute("UPDATE canonical_text_chunks SET text='nulk'", [])
        .unwrap();
    store
        .connection
        .execute("UPDATE canonical_text_replica_chunks SET text='nulk'", [])
        .unwrap();
    assert!(require_complete(&store.connection, &reference).is_err());
    assert!(enqueue(&store.connection, &reference, 2).is_err());
    assert!(receive(&mut store, &input[1..], 2).is_err());
    assert_eq!(store.sync_cursor("json-peer").unwrap().unwrap().cursor, 2);
    store
        .connection
        .execute(
            "DELETE FROM sync_entity_revisions WHERE entity_type='canonical_text_seal'",
            [],
        )
        .unwrap();
    assert!(receive(&mut store, &input[1..], 2).is_err());
    assert_eq!(store.sync_cursor("json-peer").unwrap().unwrap().cursor, 2);
}

#[test]
fn text_and_json_incomplete_objects_share_global_reservation_quota() {
    let mut store = NolaStore::open(":memory:").unwrap();
    for (index, field) in [PartBodyField::ContentText, PartBodyField::ContentJson]
        .into_iter()
        .enumerate()
    {
        let bytes = 150 * 1024 * 1024;
        let reference = CanonicalBodyReference::for_field(
            format!("quota-{index}"),
            "c".into(),
            "m".into(),
            "p".into(),
            field,
            bytes,
            (bytes / 32768) as u32,
        )
        .unwrap();
        let first = facts(&reference, &["\"".into()]).remove(0);
        if index == 0 {
            receive(&mut store, &[first], 0).unwrap();
        } else {
            assert!(receive(&mut store, &[first], 1)
                .unwrap_err()
                .to_string()
                .contains("quota"));
        }
    }
    assert_eq!(count(&store, "canonical_text_replica_chunks"), 1);
    assert_eq!(store.sync_cursor("json-peer").unwrap().unwrap().cursor, 1);
}

#[test]
fn first_replica_chunk_cannot_poison_an_existing_prepared_objects_field_or_bytes() {
    let mut store = NolaStore::open(":memory:").unwrap();
    let reference = reference("prepared", &["null".into()]);
    store
        .prepare_body(
            &reference,
            &mut vec![Ok((0, "null".into()))].into_iter(),
            100,
        )
        .unwrap();
    let input = facts(&reference, &["null".into()]);
    for field_swap in [true, false] {
        let mut altered = input[0].clone();
        let mut payload: serde_json::Value = serde_json::from_str(&altered.payload_json).unwrap();
        if field_swap {
            payload["reference"]["field"] = "content_text".into();
        } else {
            payload["text"] = "true".into();
        }
        altered.payload_json = payload.to_string();
        assert!(receive(&mut store, &[altered], 0).is_err());
        assert_eq!(count(&store, "canonical_text_replica_chunks"), 0);
        assert!(store.sync_cursor("json-peer").unwrap().is_none());
    }
    receive(&mut store, &input, 0).unwrap();
    require_complete(&store.connection, &reference).unwrap();
}
