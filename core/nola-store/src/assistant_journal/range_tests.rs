// SPDX-License-Identifier: Apache-2.0
use super::*;

fn append(store: &mut NolaStore, id: &AssistantJournalIdentity, revision: i64, text: &str) {
    store.begin_write_unit_of_work().unwrap();
    transition(store, id, revision);
    let input = append_input(store, id, revision, text);
    store.commit_assistant_journal(&input).unwrap();
    store.commit_write_unit_of_work().unwrap();
}
fn snapshot(
    store: &mut NolaStore,
    id: &AssistantJournalIdentity,
) -> (AssistantJournalSnapshot, JournalPartHead) {
    store.begin_write_unit_of_work().unwrap();
    let s = store.open_assistant_journal_snapshot(id).unwrap();
    store.commit_write_unit_of_work().unwrap();
    let p = store
        .read_assistant_snapshot_parts(&s, None, 1, 65536)
        .unwrap()
        .parts
        .remove(0);
    (s, p)
}

#[test]
fn range_crosses_chunks_preserves_utf8_and_obeys_raw_byte_budget() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = NolaStore::open(dir.path().join("db")).unwrap();
    let id = begin(&mut store, "one");
    declare(&mut store, &id);
    for (index, text) in ["ab中", "🌍c", "文de"].into_iter().enumerate() {
        append(&mut store, &id, index as i64 + 3, text);
    }
    let (s, p) = snapshot(&mut store, &id);
    let r = &p.bodies[0];
    let whole = "ab中🌍c文de";
    for offset in 0..=whole.len() {
        for limit in 4..=10 {
            let result =
                store.read_assistant_snapshot_range(&s, &p.part_id, r, offset as u64, limit);
            if !whole.is_char_boundary(offset) {
                assert!(result.is_err());
                continue;
            }
            let result = result.unwrap();
            let next = result.next_offset_bytes as usize;
            assert_eq!(result.offset_bytes, offset as u64);
            assert_eq!(result.total_utf8_bytes, whole.len() as u64);
            assert_eq!(result.text, &whole[offset..next]);
            assert!(result.text.len() <= limit);
            assert_eq!(result.end_of_content, next == whole.len());
            if next < whole.len() {
                assert!(next > offset);
                assert!(
                    whole[next..].chars().next().unwrap().len_utf8() > limit - result.text.len()
                );
            }
        }
    }
    assert_eq!(
        store
            .read_assistant_snapshot_range(&s, &p.part_id, r, 2, 7)
            .unwrap()
            .text,
        "中🌍"
    );
    assert!(store
        .read_assistant_snapshot_range(&s, &p.part_id, r, whole.len() as u64 + 1, 4)
        .is_err());
    for invalid in [0, 3, 32769, usize::MAX] {
        assert!(store
            .read_assistant_snapshot_range(&s, &p.part_id, r, 0, invalid)
            .is_err());
    }
}

#[test]
fn ranges_require_exact_active_snapshot_owner_and_content_reference() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = NolaStore::open(dir.path().join("db")).unwrap();
    let id = begin(&mut store, "one");
    declare(&mut store, &id);
    let (empty, empty_part) = snapshot(&mut store, &id);
    let result = store
        .read_assistant_snapshot_range(&empty, &empty_part.part_id, &empty_part.bodies[0], 0, 4)
        .unwrap();
    assert!(result.text.is_empty() && result.end_of_content);
    append(&mut store, &id, 3, "first");
    let (s, p) = snapshot(&mut store, &id);
    append(&mut store, &id, 4, "later");
    let (new, new_part) = snapshot(&mut store, &id);
    assert_eq!(
        store
            .read_assistant_snapshot_range(&s, &p.part_id, &p.bodies[0], 0, 32)
            .unwrap()
            .text,
        "first"
    );
    assert!(store
        .read_assistant_snapshot_range(&s, &p.part_id, &new_part.bodies[0], 0, 32)
        .is_err());
    let mut forged = s.clone();
    forged.at.identity.assistant_message_id = "another-assistant".into();
    assert!(store
        .read_assistant_snapshot_range(&forged, &p.part_id, &p.bodies[0], 0, 32)
        .is_err());
    assert!(store
        .read_assistant_snapshot_range(&s, "another-part", &p.bodies[0], 0, 32)
        .is_err());
    store.begin_write_unit_of_work().unwrap();
    store.release_assistant_journal_snapshot(&s).unwrap();
    store.commit_write_unit_of_work().unwrap();
    assert!(store
        .read_assistant_snapshot_range(&s, &p.part_id, &p.bodies[0], 0, 32)
        .is_err());
    assert_eq!(
        store
            .read_assistant_snapshot_range(&new, &p.part_id, &new_part.bodies[0], 0, 32)
            .unwrap()
            .text,
        "firstlater"
    );
}

fn suffix_query_steps(
    store: &NolaStore,
    s: &AssistantJournalSnapshot,
    p: &JournalPartHead,
) -> (i32, usize) {
    let r = &p.bodies[0];
    let mut q = store.connection.prepare(range::RANGE_SQL).unwrap();
    let rows: Vec<Vec<u8>> = q
        .query_map(
            params![
                owner(&s.at.identity).unwrap(),
                p.part_id,
                content::field(r.field).unwrap(),
                r.content_generation.0 as i64,
                r.utf8_bytes.0 as i64 - 16,
                r.utf8_bytes.0 as i64,
                17
            ],
            |r| r.get(5),
        )
        .unwrap()
        .map(Result::unwrap)
        .collect();
    (
        q.get_status(rusqlite::StatementStatus::VmStep),
        rows.iter().map(Vec::len).sum(),
    )
}

#[test]
fn fixed_suffix_seek_does_not_read_a_thirty_two_mib_prefix() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = NolaStore::open(dir.path().join("db")).unwrap();
    let id = begin(&mut store, "large");
    declare(&mut store, &id);
    let mut before = None;
    for index in 0..512 {
        append(
            &mut store,
            &id,
            index + 3,
            &format!("{index:04}{}", "x".repeat(65532)),
        );
        if index == 31 {
            let (s, p) = snapshot(&mut store, &id);
            before = Some(suffix_query_steps(&store, &s, &p));
        }
    }
    let (s, p) = snapshot(&mut store, &id);
    let after = suffix_query_steps(&store, &s, &p);
    let before = before.unwrap();
    eprintln!("suffix query at 2MiB vs 32MiB: VM/loaded chunk bytes {before:?} -> {after:?}");
    assert!(after.0 <= before.0 + 10 && after.0 < 150);
    assert_eq!(before.1, 65536);
    assert_eq!(after.1, 65536);
    let r = &p.bodies[0];
    let tail = store
        .read_assistant_snapshot_range(&s, &p.part_id, r, r.utf8_bytes.0 - 16, 16)
        .unwrap();
    assert_eq!(tail.text, "x".repeat(16));
    assert!(tail.end_of_content);
    let mut q = store
        .connection
        .prepare(&format!("EXPLAIN QUERY PLAN {}", range::RANGE_SQL))
        .unwrap();
    let plan: Vec<String> = q
        .query_map(
            params![
                owner(&id).unwrap(),
                p.part_id,
                content::field(r.field).unwrap(),
                r.content_generation.0 as i64,
                0,
                r.utf8_bytes.0 as i64,
                17
            ],
            |r| r.get(3),
        )
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert!(
        plan.iter()
            .any(|p| p.contains("SEARCH m USING INDEX assistant_journal_manifest_byte_range")),
        "{plan:?}"
    );
    assert!(
        !plan
            .iter()
            .any(|p| p.contains("SCAN") || p.contains("TEMP B-TREE")),
        "{plan:?}"
    );
}


#[test]
fn activity_preview_uses_public_text_at_the_completion_frontier() {
    let mut store=NolaStore::open(":memory:").unwrap();
    let id=begin(&mut store,"activity-preview");
    declare(&mut store,&id);
    append(&mut store,&id,3,"公开结果🙂");
    let part=format!("{}-journal-part",id.run_id);
    // Publish final public text through the ordinary journal update path.
    let head=store.assistant_journal_part(&id,&part).unwrap().unwrap();
    store.begin_write_unit_of_work().unwrap();
    transition(&mut store,&id,4);
    store.commit_assistant_journal(&CommitAssistantJournal {
        stamp:AssistantSourceCommitStamp::Transition{transition_id:format!("{}-4",id.run_id)},
        expected:store.assistant_journal_head(&id).unwrap(),
        batch:AssistantOperationBatchV1{protocol_version:1,identity:id.clone(),operations:vec![
            AssistantJournalOperationV1::Update(UpdateJournalMetadata{part_id:part.clone(),
                expected_generation:head.generation,expected_revision:head.revision,
                metadata:JournalPartMetadata::Text(JournalTextMetadata{is_final:true,
                    visibility:JournalVisibility::Public,review:None})})]},supplied_chunks:vec![],
    }).unwrap();
    store.commit_write_unit_of_work().unwrap();
    let frontier=store.assistant_journal_head(&id).unwrap();
    let entry=nola_domain::assistant_run_terminal::AssistantRunTerminalManifestEntry::Assistant{
        through:frontier,ordinal:1,created_at_ms:1,
        state:nola_domain::assistant_run_terminal::AssistantRunTerminalStatus::Completed,completed_at_ms:4};
    // This fixture supplies only the immutable frontier; the Application detail
    // path separately requires the existing canonical terminal validation.
    store.connection.execute("INSERT INTO assistant_run_terminal_manifest(run_id,ordinal,entry_json) VALUES(?1,0,?2)",
        params![id.run_id,serde_json::to_string(&entry).unwrap()]).unwrap();
    assert_eq!(activity_result_text(&store.connection,&id.conversation_id,&id.run_id,&[part.clone()]).unwrap(),
        (Some("公开结果🙂".into()),false));
    assert!(activity_result_text(&store.connection,"foreign",&id.run_id,&[part]).is_err());
}
