// SPDX-License-Identifier: Apache-2.0
//! Opt-in, disk-backed Store measurements, not an Application/Core benchmark.
//! The checkpoint is intentionally constant-size; Core serialization cost is
//! excluded and must be measured independently. Each size owns one TempDir.
use super::*;
use nola_domain::{
    apply_canonical_stream, CanonicalStreamHead, CanonicalStreamOperation, CanonicalStreamOwner,
    PartBodyField,
};
use std::{path::Path, time::Instant};

const CHUNK: usize = 32768;

fn owner() -> CanonicalStreamOwner {
    CanonicalStreamOwner {
        conversation_id: "c".into(),
        run_id: "r".into(),
        message_id: "a".into(),
        part_id: "p".into(),
        field: PartBodyField::ContentText,
    }
}
fn begin(store: &mut NolaStore) {
    store
        .begin_turn(&BeginTurnInput {
            user_part_kind: "text".into(),
            expected_configuration_revision: None,
            expected_parent_message_id: None,
            initial_events: Vec::new(),
            initial_assistant_part_mutations: vec![],
            memory_usage_entries: vec![],
            delegated_invocation_mutations: vec![],
            delegated_permission_mutations: vec![],
            model_attempt_mutations: vec![],
            conversation_id: "c".into(),
            conversation_title: "stream benchmark".into(),
            user_message_id: "u".into(),
            user_part_id: "up".into(),
            user_text: "fixture".into(),
            user_content_json: None,
            additional_user_parts: vec![],
            assistant_message_id: "a".into(),
            assistant_part_id: "p".into(),
            assistant_turn_bindings: vec![],
            run_id: "r".into(),
            context_checkpoint_id: None,
            checkpoint_revision: 1,
            checkpoint_event_cursor: 0,
            checkpoint_json: "{}".into(),
            invocation_config_json: "{}".into(),
            invocation_config_digest: "fixture".into(),
            outbox_effects: vec![],
            created_at_ms: 1,
        })
        .unwrap();
}
fn commit(store: &mut NolaStore, revision: i64, operations: Vec<PartMutation>) {
    assert!(
        store
            .commit_transition(&CommitTransitionInput {
                transition_id: format!("performance-{revision}"),
                run_id: "r".into(),
                expected_checkpoint_revision: revision,
                checkpoint_revision: revision + 1,
                checkpoint_event_cursor: 0,
                checkpoint: nola_store_port::run_effect::CheckpointWrite::Baseline { checkpoint_json: "{}".into() },
                submission_reduction_json: None, host_event: None,
                events: vec![],
                ensured_messages: vec![],
                assistant_turn_bindings: vec![],
                part_message_owners: Default::default(),
                part_mutations: operations,
                outbox_effects: vec![],
                model_attempt_mutations: vec![],
                delegated_invocation_mutations: vec![],
                delegated_permission_mutations: vec![],
                effect_delivery_attempt_mutations: vec![],
                acknowledged_effect_ids: vec![],
                memory_usage_entries: vec![],
                run_status: None,
                assistant_status: None,
                assistant_status_message_id: None,
                committed_at_ms: revision + 1,
            })
            .unwrap()
            .applied
    );
}
fn mutation(operation: CanonicalStreamOperation) -> PartMutation {
    PartMutation::Stream {
        operation,
        updated_at_ms: 2,
    }
}
fn chunk(index: usize) -> String {
    let mut text = format!("{index:016x}");
    let middle = CHUNK - 16 - 256;
    text.push_str(&"中😀".repeat(middle / 7));
    text.push_str(&"x".repeat(middle % 7));
    text.push_str(&format!("{index:016x}").repeat(16));
    assert_eq!(text.len(), CHUNK);
    text
}
fn bytes(path: &Path) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}
fn sizes(path: &Path) -> serde_json::Value {
    serde_json::json!({"db_bytes":bytes(path),"wal_bytes":bytes(&path.with_extension("db-wal")),"shm_bytes":bytes(&path.with_extension("db-shm"))})
}

/// SQLite page-cache counters, not OS physical bytes or fsync counts. Resetting
/// counters changes no database state and uses this test's owned connection.
fn cache_counter(connection: &Connection, operation: i32, reset: bool) -> i32 {
    let (mut current, mut highwater) = (0, 0);
    // SAFETY: the borrowed connection is alive on this thread throughout the
    // call; SQLite writes only to the two live integer outputs.
    let code = unsafe {
        rusqlite::ffi::sqlite3_db_status(
            connection.handle(),
            operation,
            &mut current,
            &mut highwater,
            i32::from(reset),
        )
    };
    assert_eq!(code, rusqlite::ffi::SQLITE_OK);
    current
}
fn io(connection: &Connection, reset: bool) -> serde_json::Value {
    serde_json::json!({
        "sqlite_cache_page_writes":cache_counter(connection,rusqlite::ffi::SQLITE_DBSTATUS_CACHE_WRITE,reset),
        "sqlite_cache_page_misses":cache_counter(connection,rusqlite::ffi::SQLITE_DBSTATUS_CACHE_MISS,reset),
    })
}
fn resolve(store: &NolaStore) -> CanonicalStreamHead {
    crate::canonical_stream_read::resolve(&store.connection, "c", "a", "p")
        .unwrap()
        .unwrap()
}

#[test]
#[ignore = "disk-backed 2/8/32 MiB production transition benchmark; run explicitly with --ignored --nocapture"]
fn production_stream_2_8_32_mib_disk_acceptance() {
    // The previous size is dropped and its owned directory removed before the
    // next iteration, bounding disk usage on developer machines.
    for mib in [2usize, 8, 32] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("stream.db");
        let mut store = NolaStore::open(&path).unwrap();
        begin(&mut store);
        let total = mib * 1024 * 1024;
        let declare = CanonicalStreamOperation::Declare { owner: owner() };
        let mut head = apply_canonical_stream(None, &declare).unwrap();
        commit(&mut store, 1, vec![mutation(declare)]);
        io(&store.connection, true);
        let started = Instant::now();
        let mut digest = Sha256::new();
        for index in 0..total / CHUNK {
            let text = chunk(index);
            digest.update(text.as_bytes());
            let operation = CanonicalStreamOperation::Append {
                owner: owner(),
                expected_head: head.clone(),
                delta: text,
            };
            let next = apply_canonical_stream(Some(&head), &operation).unwrap();
            commit(&mut store, index as i64 + 2, vec![mutation(operation)]);
            head = next;
        }
        let append_ms = started.elapsed().as_secs_f64() * 1000.0;
        let append_io = io(&store.connection, true);
        let append_sizes = sizes(&path);
        assert_eq!(resolve(&store), head);
        assert_eq!(head.total_utf8_bytes, total as u64);
        let expected_digest = format!("{:x}", digest.finalize());
        let close = CanonicalStreamOperation::Close {
            owner: owner(),
            expected_head: head.clone(),
        };
        let close_started = Instant::now();
        commit(
            &mut store,
            (total / CHUNK) as i64 + 2,
            vec![
                mutation(close),
                PartMutation::Close {
                    part_id: "p".into(),
                    state: "completed".into(),
                    content_json: None,
                    content_digest: Some(expected_digest.clone()),
                    updated_at_ms: 3,
                },
            ],
        );
        let close_ms = close_started.elapsed().as_secs_f64() * 1000.0;
        let close_io = io(&store.connection, true);
        let closed = resolve(&store);
        assert!(closed.closed);
        let last = chunk(total / CHUNK - 1);
        let expected_suffix = &last[CHUNK - 128..];
        let reads_started = Instant::now();
        for _ in 0..100 {
            let reader = crate::canonical_field_reader::CanonicalFieldReader::open(
                &store.connection,
                "c",
                "a",
                "p",
                PartBodyField::ContentText,
            )
            .unwrap();
            assert_eq!(
                reader.read_text(128).unwrap(),
                format!("…{expected_suffix}")
            );
            let range = reader.read_range(total as u64 - 128, 128).unwrap();
            assert_eq!(range.text, expected_suffix);
            assert!(range.end_of_content);
        }
        let reads_ms = reads_started.elapsed().as_secs_f64() * 1000.0;
        let reads_io = io(&store.connection, true);
        let page_size: i64 = store
            .connection
            .query_row("PRAGMA page_size", [], |r| r.get(0))
            .unwrap();
        let journal_mode: String = store
            .connection
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        let final_sizes = sizes(&path);
        drop(store);
        let store = NolaStore::open(&path).unwrap();
        let recovered = resolve(&store);
        assert_eq!(recovered, closed);
        let verify_started = Instant::now();
        let mut actual = Sha256::new();
        let mut offset = 0;
        while offset < total as u64 {
            let range =
                crate::canonical_stream_read::range(&store.connection, &recovered, offset, CHUNK)
                    .unwrap();
            assert!(range.next_offset_bytes > offset);
            actual.update(range.text.as_bytes());
            offset = range.next_offset_bytes;
        }
        assert_eq!(offset, total as u64);
        assert_eq!(format!("{:x}", actual.finalize()), expected_digest);
        let verify_ms = verify_started.elapsed().as_secs_f64() * 1000.0;
        eprintln!(
            "{}",
            serde_json::json!({"scope":"Store commit_transition only; constant checkpoint; not App/Core",
            "mib":mib,"chunk_bytes":CHUNK,"append_count":total/CHUNK,"append_ms":append_ms,"append_io":append_io,"append_sizes":append_sizes,
            "close_ms":close_ms,"close_io":close_io,"tail_and_range_iterations":100,"tail_and_range_ms":reads_ms,"tail_and_range_io":reads_io,
            "cold_complete_digest_ms":verify_ms,"page_size":page_size,"journal_mode":journal_mode,"final_sizes":final_sizes})
        );
        drop(store);
        drop(dir);
    }
}
