// SPDX-License-Identifier: Apache-2.0
use super::*;

#[test]
fn exact_release_is_idempotent_rejects_reads_and_preserves_other_handles_and_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let mut store = NolaStore::open(&path).unwrap();
    let id = begin(&mut store, "one");
    declare(&mut store, &id);
    store.begin_write_unit_of_work().unwrap();
    transition(&mut store, &id, 3);
    let input = append_input(&store, &id, 3, "immutable shared bytes");
    store.commit_assistant_journal(&input).unwrap();
    let first = store.open_assistant_journal_snapshot(&id).unwrap();
    let second = store.open_assistant_journal_snapshot(&id).unwrap();
    store.commit_write_unit_of_work().unwrap();
    let part = store
        .read_assistant_snapshot_parts(&first, None, 1, 65536)
        .unwrap()
        .parts
        .remove(0);
    assert!(store.release_assistant_journal_snapshot(&first).is_err());
    store.begin_write_unit_of_work().unwrap();
    assert_eq!(
        store.release_assistant_journal_snapshot(&first).unwrap(),
        AssistantSnapshotRelease::Released
    );
    assert_eq!(
        store.release_assistant_journal_snapshot(&first).unwrap(),
        AssistantSnapshotRelease::AlreadyReleased
    );
    assert!(store
        .read_assistant_snapshot_parts(&first, None, 1, 65536)
        .is_err());
    assert!(store
        .read_assistant_snapshot_manifest(&first, &part.part_id, &part.bodies[0], 0, 1)
        .is_err());
    assert!(store
        .read_assistant_snapshot_chunk(&first, &part.part_id, &part.bodies[0], 0)
        .is_err());
    store.commit_write_unit_of_work().unwrap();
    drop(store);
    let mut store = NolaStore::open(&path).unwrap();
    store.begin_write_unit_of_work().unwrap();
    assert_eq!(
        store.release_assistant_journal_snapshot(&first).unwrap(),
        AssistantSnapshotRelease::AlreadyReleased
    );
    store.commit_write_unit_of_work().unwrap();
    assert_eq!(
        store
            .read_assistant_snapshot_parts(&second, None, 1, 65536)
            .unwrap()
            .parts,
        vec![part.clone()]
    );
    assert_eq!(
        store
            .read_assistant_snapshot_chunk(&second, &part.part_id, &part.bodies[0], 0)
            .unwrap()
            .text(),
        "immutable shared bytes"
    );
    assert_eq!(
        store
            .connection
            .query_row("SELECT count(*) FROM assistant_journal_chunks", [], |r| r
                .get::<_, i64>(
                0
            ))
            .unwrap(),
        1
    );
    assert_eq!(
        store
            .connection
            .query_row(
                "SELECT count(*) FROM assistant_journal_snapshots",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        2
    );
    assert_eq!(store.assistant_journal_head(&id).unwrap().sequence, 2);
}

#[test]
fn forged_or_foreign_release_cannot_change_another_snapshot_and_rollback_restores_readability() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = NolaStore::open(dir.path().join("db")).unwrap();
    let a = begin(&mut store, "a");
    let b = begin(&mut store, "b");
    declare(&mut store, &a);
    declare(&mut store, &b);
    store.begin_write_unit_of_work().unwrap();
    let first = store.open_assistant_journal_snapshot(&a).unwrap();
    let other = store.open_assistant_journal_snapshot(&b).unwrap();
    store.commit_write_unit_of_work().unwrap();
    let mut forged_owner = first.clone();
    forged_owner.at.identity = b;
    let mut forged_sequence = first.clone();
    forged_sequence.at.sequence += 1;
    let mut forged_digest = first.clone();
    forged_digest.at.record_digest = AssistantJournalCursor::initial(a).unwrap().record_digest;
    let mut forged_id = first.clone();
    forged_id.snapshot_id = "missing-snapshot".into();
    store.begin_write_unit_of_work().unwrap();
    for forged in [forged_owner, forged_sequence, forged_digest, forged_id] {
        assert!(store.release_assistant_journal_snapshot(&forged).is_err());
    }
    assert_eq!(
        store
            .connection
            .query_row(
                "SELECT count(*) FROM assistant_journal_snapshot_releases",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    assert_eq!(
        store.release_assistant_journal_snapshot(&first).unwrap(),
        AssistantSnapshotRelease::Released
    );
    store.rollback_write_unit_of_work().unwrap();
    assert_eq!(
        store
            .read_assistant_snapshot_parts(&first, None, 1, 65536)
            .unwrap()
            .parts
            .len(),
        1
    );
    assert_eq!(
        store
            .read_assistant_snapshot_parts(&other, None, 1, 65536)
            .unwrap()
            .parts
            .len(),
        1
    );
}

