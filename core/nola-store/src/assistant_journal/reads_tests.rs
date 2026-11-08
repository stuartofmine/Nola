// SPDX-License-Identifier: Apache-2.0
use super::*;

#[test]
fn admitted_empty_assistant_genesis_is_discoverable_without_inventing_a_part() {
    let directory = tempfile::tempdir().unwrap();
    let mut store = NolaStore::open(directory.path().join("genesis-window.sqlite")).unwrap();
    let id = begin(&mut store, "genesis-window");
    let expected = store.assistant_journal_head(&id).unwrap();
    assert_eq!(expected.sequence, 0);
    let rows: i64 = store.connection.query_row("SELECT COUNT(*) FROM assistant_journal_heads", [], |r| r.get(0)).unwrap();
    assert_eq!(rows, 0, "admission must not fabricate a visible journal record");
    let window = store.load_conversation(&id.conversation_id).unwrap().unwrap();
    assert_eq!(window_heads(&store.connection, &window).unwrap(), vec![expected]);

    // Even a real Run root does not authorize genesis discovery without its
    // admission fact. Model missing/corrupt provenance, never repair it here.
    store.connection.execute("DELETE FROM assistant_turn_message_bindings WHERE run_id=?1", [&id.run_id]).unwrap();
    assert!(window_heads(&store.connection, &window).unwrap().is_empty());
}

#[test]
fn historical_run_labels_do_not_invent_journal_owners_but_real_heads_remain_strict() {
    let directory = tempfile::tempdir().unwrap();
    let mut store = NolaStore::open(directory.path().join("historical-window.sqlite")).unwrap();
    let id = begin(&mut store, "history-window");
    let mut window = store.load_conversation(&id.conversation_id).unwrap().unwrap();
    for run in [None, Some("external-archive-run".to_string())] {
        for message in &mut window.messages {
            if message.role == "assistant" { message.run_id = run.clone(); }
        }
        assert!(window_heads(&store.connection, &window).unwrap().is_empty());
    }
    declare(&mut store, &id);
    let window = store.load_conversation(&id.conversation_id).unwrap().unwrap();
    assert_eq!(window_heads(&store.connection, &window).unwrap(), vec![store.assistant_journal_head(&id).unwrap()]);
    // Simulate corrupt persisted owner evidence, not an ordinary archive with
    // no journal. Discovery must never turn this into an empty successful read.
    store.connection.execute("UPDATE messages SET run_id=NULL WHERE id=?1", params![id.assistant_message_id]).unwrap();
    assert!(window_heads(&store.connection, &window).is_err());
}



fn bodyless_part(id: &AssistantJournalIdentity, name: &str) -> JournalPartHead {
    let AssistantJournalOperationV1::Declare(value) = declaration(id).operations.remove(0) else {
        panic!()
    };
    let mut part = value.part;
    part.part_id = name.into();
    part.bodies.clear();
    part
}

fn commit_operations(
    store: &mut NolaStore,
    id: &AssistantJournalIdentity,
    revision: i64,
    operations: Vec<AssistantJournalOperationV1>,
) {
    let transition_id = transition(store, id, revision);
    store
        .commit_assistant_journal(&CommitAssistantJournal {
            stamp: AssistantSourceCommitStamp::Transition { transition_id },
            expected: store.assistant_journal_head(id).unwrap(),
            batch: AssistantOperationBatchV1 {
                protocol_version: 1,
                identity: id.clone(),
                operations,
            },
            supplied_chunks: vec![],
        })
        .unwrap();
}

#[test]
fn part_changes_page_updates_and_retirements_at_the_fixed_target_only() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = NolaStore::open(dir.path().join("changes.sqlite")).unwrap();
    let id = begin(&mut store, "delta");
    declare(&mut store, &id);
    let base = store.assistant_journal_head(&id).unwrap();
    store.begin_write_unit_of_work().unwrap();
    commit_operations(&mut store, &id, 3, vec![
        AssistantJournalOperationV1::Declare(DeclareJournalPart { part: bodyless_part(&id, "a") }),
        AssistantJournalOperationV1::Declare(DeclareJournalPart { part: bodyless_part(&id, "b") }),
        AssistantJournalOperationV1::Declare(DeclareJournalPart { part: bodyless_part(&id, "c") }),
        AssistantJournalOperationV1::Remove(RemoveJournalPart {
            part_id: "c".into(), expected_generation: JournalNumber(1), expected_revision: JournalNumber(1),
        }),
    ]);
    let target = store.open_assistant_journal_snapshot(&id).unwrap();
    commit_operations(&mut store, &id, 4, vec![
        AssistantJournalOperationV1::Remove(RemoveJournalPart {
            part_id: "a".into(), expected_generation: JournalNumber(1), expected_revision: JournalNumber(1),
        }),
        AssistantJournalOperationV1::Declare(DeclareJournalPart { part: bodyless_part(&id, "future") }),
    ]);
    store.commit_write_unit_of_work().unwrap();
    let mut changes = Vec::new();
    let mut after = None;
    loop {
        let page = store.read_assistant_snapshot_changes(&target, &base, after.as_deref(), 1, 65536).unwrap();
        assert_eq!(page.changes.len(), 1);
        changes.extend(page.changes);
        if let Some(next) = page.next_part_id {
            assert!(after.as_ref().is_none_or(|old| old < &next));
            after = Some(next);
        } else { break; }
    }
    assert_eq!(changes.iter().map(|change| change.part_id.as_str()).collect::<Vec<_>>(), vec!["a", "b", "c"]);
    assert!(changes[0].head.is_some(), "a later removal cannot leak into the fixed target");
    assert!(changes[1].head.is_some());
    assert!(changes[2].head.is_none(), "retirement must be an explicit entry, not skipped");
    assert!(store.read_assistant_snapshot_changes(&target, &target.at, None, 10, 65536).unwrap().changes.is_empty());
    assert!(store.read_assistant_snapshot_changes(&target, &base, None, 1, 1).is_err());
    let future = store.assistant_journal_head(&id).unwrap();
    assert!(store.read_assistant_snapshot_changes(&target, &future, None, 10, 65536).is_err());
    let mut foreign = base.clone();
    foreign.identity.run_id = "foreign-run".into();
    assert!(store.read_assistant_snapshot_changes(&target, &foreign, None, 10, 65536).is_err());
    store.connection.execute("UPDATE assistant_journal_heads SET retained_floor=?1 WHERE owner=?2",
        params![number(target.at.sequence).unwrap(), owner(&id).unwrap()]).unwrap();
    assert!(store.read_assistant_snapshot_changes(&target, &base, None, 10, 65536).is_err(),
        "an expired base must request recovery, never silently omit retired history");
}

#[test]
fn presentation_order_survives_tombstones_cold_reopen_and_fixed_snapshot_pages() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ordered.db");
    let mut store = NolaStore::open_archive_mode(&path, false).unwrap();
    let id = begin(&mut store, "ordered");
    declare(&mut store, &id);
    store.begin_write_unit_of_work().unwrap();
    let mut first = bodyless_part(&id, "z-first");
    first.order_key.position = JournalNumber(1);
    let mut second = bodyless_part(&id, "a-second");
    second.order_key.position = JournalNumber(2);
    let mut task = bodyless_part(&id, "m-task-section");
    task.order_key.section = JournalSection::Tasks;
    let mut retired = bodyless_part(&id, "b-retired-in-frame");
    retired.order_key.position = JournalNumber(1);
    commit_operations(&mut store, &id, 3, vec![
        AssistantJournalOperationV1::Declare(DeclareJournalPart { part: second }),
        AssistantJournalOperationV1::Declare(DeclareJournalPart { part: task }),
        AssistantJournalOperationV1::Declare(DeclareJournalPart { part: first }),
        AssistantJournalOperationV1::Declare(DeclareJournalPart { part: retired }),
        AssistantJournalOperationV1::Remove(RemoveJournalPart {
            part_id: "b-retired-in-frame".into(), expected_generation: JournalNumber(1), expected_revision: JournalNumber(1),
        }),
    ]);
    let before = store.open_assistant_journal_snapshot(&id).unwrap();
    commit_operations(&mut store, &id, 4, vec![
        AssistantJournalOperationV1::Remove(RemoveJournalPart {
            part_id: "z-first".into(), expected_generation: JournalNumber(1), expected_revision: JournalNumber(1),
        }),
    ]);
    let after = store.open_assistant_journal_snapshot(&id).unwrap();
    store.commit_write_unit_of_work().unwrap();
    // Current-format cold reopen must retain the exact fixed-snapshot order.
    drop(store);
    let store = NolaStore::open_archive_mode(&path, false).unwrap();
    for (snapshot, expected) in [
        (&before, vec!["ordered-run-journal-part", "z-first", "a-second", "m-task-section"]),
        (&after, vec!["ordered-run-journal-part", "a-second", "m-task-section"]),
    ] {
        let mut cursor = None;
        let mut ids = Vec::new();
        loop {
            let page = store.read_assistant_snapshot_parts(snapshot, cursor.as_deref(), 1, 65536).unwrap();
            ids.extend(page.parts.into_iter().map(|part| part.part_id));
            match page.next_part_id {
                Some(next) => { assert_ne!(cursor.as_ref(), Some(&next)); cursor = Some(next); }
                None => break,
            }
        }
        assert_eq!(ids, expected);
    }
    let suffix = store.read_assistant_snapshot_parts(&after, Some("z-first"), 10, 65536).unwrap();
    assert_eq!(suffix.parts.iter().map(|p|p.part_id.as_str()).collect::<Vec<_>>(), vec!["a-second", "m-task-section"]);
    assert!(store.read_assistant_snapshot_parts(&after, Some("foreign-cursor"), 1, 65536).is_err());
    let retired_seek = store.read_assistant_snapshot_parts(&before, Some("b-retired-in-frame"), 1, 65536).unwrap();
    assert_eq!(retired_seek.parts[0].part_id, "z-first");
}

#[test]
fn snapshot_directory_pages_progress_through_retired_and_future_parts() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = NolaStore::open(dir.path().join("db")).unwrap();
    let id = begin(&mut store, "one");
    declare(&mut store, &id);
    store.begin_write_unit_of_work().unwrap();
    let mut retired = Vec::new();
    for index in 0..20 {
        let name = format!("aaa-retired-{index:03}");
        retired.push(AssistantJournalOperationV1::Declare(DeclareJournalPart {
            part: bodyless_part(&id, &name),
        }));
        retired.push(AssistantJournalOperationV1::Remove(RemoveJournalPart {
            part_id: name,
            expected_generation: JournalNumber(1),
            expected_revision: JournalNumber(1),
        }));
    }
    commit_operations(&mut store, &id, 3, retired);
    let snapshot = store.open_assistant_journal_snapshot(&id).unwrap();
    let future = (0..20)
        .map(|index| {
            AssistantJournalOperationV1::Declare(DeclareJournalPart {
                part: bodyless_part(&id, &format!("bbb-future-{index:03}")),
            })
        })
        .collect();
    commit_operations(&mut store, &id, 4, future);
    store.commit_write_unit_of_work().unwrap();
    let (mut cursor, mut visible, mut empty_pages) = (None, Vec::new(), 0);
    for page_index in 0..10 {
        let page = store
            .read_assistant_snapshot_parts(&snapshot, cursor.as_deref(), 5, 65536)
            .unwrap();
        if page.parts.is_empty() {
            empty_pages += 1;
        }
        visible.extend(page.parts);
        if let Some(next) = page.next_part_id {
            assert!(cursor.as_ref().is_none_or(|old| old < &next));
            cursor = Some(next);
        } else {
            assert_eq!(page_index, 8);
            break;
        }
    }
    assert_eq!(empty_pages, 8);
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].part_id, "one-run-journal-part");
}

fn query_steps(store: &NolaStore, snapshot: &AssistantJournalSnapshot, part: &str) -> (i32, i32) {
    let key = owner(&snapshot.at.identity).unwrap();
    let mut directory = store.connection.prepare(reads::PART_DIRECTORY_SQL).unwrap();
    let ids: Vec<String> = directory
        .query_map(params![key, -1, -1, "", 2], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(ids, vec![part]);
    let mut version = store.connection.prepare(reads::PART_VERSION_SQL).unwrap();
    let _: String = version
        .query_row(params![key, part, snapshot.at.sequence as i64], |r| {
            r.get(0)
        })
        .unwrap();
    (
        directory.get_status(rusqlite::StatementStatus::VmStep),
        version.get_status(rusqlite::StatementStatus::VmStep),
    )
}

/// Store-only baseline: 16 activity heads per batch, scaled independently of
/// body size. Each batch commits durably; never hide write latency in one giant
/// fixture transaction. Application checkpoint and UI costs are separate gates.
#[test]
#[ignore = "disk-backed 1/10/1000 activity scale baseline; run with --ignored --nocapture"]
fn activity_history_thousandfold_disk_baseline() {
    use std::time::Instant;
    fn percentile(values: &mut [u128], percent: usize) -> u128 {
        values.sort_unstable();
        values[((values.len() - 1) * percent).div_ceil(100)]
    }
    for scale in [1usize, 10, 1000] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("activity.sqlite");
        let mut store = NolaStore::open(&path).unwrap();
        let id = begin(&mut store, "activity-scale");
        declare(&mut store, &id);
        let base = store.assistant_journal_head(&id).unwrap();
        let mut writes = Vec::new();
        for batch in 0..scale {
            let operations = (0..16).map(|offset| {
                AssistantJournalOperationV1::Declare(DeclareJournalPart {
                    part: bodyless_part(&id, &format!("activity-{:08}", batch * 16 + offset)),
                })
            }).collect();
            let started = Instant::now();
            store.begin_write_unit_of_work().unwrap();
            commit_operations(&mut store, &id, batch as i64 + 3, operations);
            store.commit_write_unit_of_work().unwrap();
            writes.push(started.elapsed().as_micros());
        }
        store.begin_write_unit_of_work().unwrap();
        let snapshot = store.open_assistant_journal_snapshot(&id).unwrap();
        store.commit_write_unit_of_work().unwrap();
        let mut latencies = Vec::new();
        let mut after = None;
        let mut count = 0;
        loop {
            let started = Instant::now();
            let page = store.read_assistant_snapshot_changes(
                &snapshot, &base, after.as_deref(), 128, 256 * 1024,
            ).unwrap();
            latencies.push(started.elapsed().as_micros());
            for change in &page.changes {
                assert_eq!(change.part_id, format!("activity-{count:08}"));
                assert!(change.head.is_some());
                count += 1;
            }
            match page.next_part_id {
                Some(next) => {
                    assert!(after.as_ref().is_none_or(|old| old < &next));
                    after = Some(next);
                }
                None => break,
            }
        }
        assert_eq!(count, scale * 16);
        eprintln!("{}", serde_json::json!({
            "benchmark":"activity_history", "scale":scale, "parts":count,
            "write_batch_p50_us":percentile(&mut writes,50),
            "write_batch_p95_us":percentile(&mut writes,95),
            "delta_page_p50_us":percentile(&mut latencies,50),
            "delta_page_p95_us":percentile(&mut latencies,95),
            "delta_pages":latencies.len(),
            "db_bytes":std::fs::metadata(&path).unwrap().len(),
            "wal_bytes":std::fs::metadata(path.with_extension("sqlite-wal")).map(|m|m.len()).unwrap_or(0),
        }));
    }
}

#[test]
fn thousands_of_revisions_do_not_turn_one_part_page_into_a_history_scan() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = NolaStore::open(dir.path().join("db")).unwrap();
    let id = begin(&mut store, "one");
    declare(&mut store, &id);
    store.begin_write_unit_of_work().unwrap();
    let snapshot = store.open_assistant_journal_snapshot(&id).unwrap();
    let part = "one-run-journal-part";
    let before = query_steps(&store, &snapshot, part);
    for index in 0..2048 {
        commit_operations(
            &mut store,
            &id,
            index + 3,
            vec![AssistantJournalOperationV1::Update(UpdateJournalMetadata {
                part_id: part.into(),
                expected_generation: JournalNumber(1),
                expected_revision: JournalNumber(index as u64 + 1),
                metadata: JournalPartMetadata::Text(JournalTextMetadata {
                    is_final: index % 2 == 0,
                    visibility: JournalVisibility::Public,
                    review: None,
                }),
            })],
        );
    }
    store.commit_write_unit_of_work().unwrap();
    let after = query_steps(&store, &snapshot, part);
    eprintln!("snapshot directory/version VM steps: {before:?} -> {after:?} after 2048 revisions");
    assert!(
        after.0 <= before.0 + 10 && after.1 <= before.1 + 10,
        "VM steps grew with history: {before:?} -> {after:?}"
    );
    assert!(after.0 < 100 && after.1 < 100);
    for sql in [reads::PART_DIRECTORY_SQL, reads::PART_VERSION_SQL] {
        let mut explain = store
            .connection
            .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
            .unwrap();
        let arguments: Vec<rusqlite::types::Value> = if sql == reads::PART_DIRECTORY_SQL {
            vec![owner(&id).unwrap().into(), (-1).into(), (-1).into(), "".to_string().into(), 1.into()]
        } else {
            vec![owner(&id).unwrap().into(), part.to_string().into(), 1.into()]
        };
        let plan: Vec<String> = explain
            .query_map(rusqlite::params_from_iter(arguments), |r| r.get(3))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert!(
            plan.iter()
                .any(|line| line.contains("SEARCH") && line.contains("INDEX")),
            "{plan:?}"
        );
        assert!(
            !plan
                .iter()
                .any(|line| line.contains("SCAN") || line.contains("TEMP B-TREE")),
            "{plan:?}"
        );
    }
    let page = store
        .read_assistant_snapshot_parts(&snapshot, None, 1, 65536)
        .unwrap();
    assert_eq!(page.parts.len(), 1);
    assert_eq!(page.parts[0].revision, JournalNumber(1));
    assert!(page.next_part_id.is_none());
}

#[test]
fn thirty_two_mib_source_retains_linear_chunk_and_record_payloads() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = NolaStore::open(dir.path().join("db")).unwrap();
    let id = begin(&mut store, "large");
    declare(&mut store, &id);
    for index in 0..512 {
        let text = format!("{index:04}{}", "x".repeat(65532));
        store.begin_write_unit_of_work().unwrap();
        transition(&mut store, &id, index + 3);
        let input = append_input(&store, &id, index + 3, &text);
        store.commit_assistant_journal(&input).unwrap();
        store.commit_write_unit_of_work().unwrap();
    }
    let bytes: i64 = store
        .connection
        .query_row(
            "SELECT SUM(length(bytes)) FROM assistant_journal_chunks",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(bytes, 32 * 1024 * 1024);
    let record_bytes: i64 = store
        .connection
        .query_row(
            "SELECT SUM(length(batch)+length(header_json)) FROM assistant_journal_records",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(record_bytes < 513 * 4096);
    let refs: i64 = store
        .connection
        .query_row(
            "SELECT count(*) FROM assistant_journal_manifest_chunks",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(refs, 512);
    store.begin_write_unit_of_work().unwrap();
    let snapshot = store.open_assistant_journal_snapshot(&id).unwrap();
    store.commit_write_unit_of_work().unwrap();
    let part = store
        .read_assistant_snapshot_parts(&snapshot, None, 1, 65536)
        .unwrap()
        .parts
        .remove(0);
    assert_eq!(
        store
            .read_assistant_snapshot_chunk(&snapshot, &part.part_id, &part.bodies[0], 511)
            .unwrap()
            .byte_length(),
        65536
    );
}

#[test]
fn snapshot_payload_budget_resumes_before_unreturned_head_and_rejects_invalid_limits() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = NolaStore::open(dir.path().join("db")).unwrap();
    let id = begin(&mut store, "one");
    declare(&mut store, &id);
    store.begin_write_unit_of_work().unwrap();
    commit_operations(
        &mut store,
        &id,
        3,
        vec![AssistantJournalOperationV1::Declare(DeclareJournalPart {
            part: bodyless_part(&id, "aaa-visible"),
        })],
    );
    let snapshot = store.open_assistant_journal_snapshot(&id).unwrap();
    store.commit_write_unit_of_work().unwrap();
    let all = store
        .read_assistant_snapshot_parts(&snapshot, None, 10, 65536)
        .unwrap()
        .parts;
    assert_eq!(all.len(), 2);
    let budget = all
        .iter()
        .map(|head| serde_json::to_vec(head).unwrap().len())
        .max()
        .unwrap();
    let first = store
        .read_assistant_snapshot_parts(&snapshot, None, 10, budget)
        .unwrap();
    assert_eq!(first.parts, all[..1]);
    assert_eq!(first.next_part_id.as_deref(), Some("aaa-visible"));
    let second = store
        .read_assistant_snapshot_parts(&snapshot, first.next_part_id.as_deref(), 10, budget)
        .unwrap();
    assert_eq!(second.parts, all[1..]);
    assert!(second.next_part_id.is_none());
    for (records, bytes) in [
        (0, 65536),
        (129, 65536),
        (1, 0),
        (1, 1),
        (1, ASSISTANT_FRAME_MAX_BYTES + 1),
    ] {
        assert!(store
            .read_assistant_snapshot_parts(&snapshot, None, records, bytes)
            .is_err());
    }
}
