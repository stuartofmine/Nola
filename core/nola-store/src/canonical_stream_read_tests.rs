// SPDX-License-Identifier: Apache-2.0
use super::*;
use crate::NolaStore;
use nola_domain::{CanonicalStreamOwner, PartBodyField};
use sha2::{Digest, Sha256};

#[test]
fn complete_digest_hashes_current_unicode_generation_including_empty_replacement() {
    let (_dir, mut store, empty) = fixture();
    let hash = |text: &str| format!("{:x}", Sha256::digest(text.as_bytes()));
    assert_eq!(
        complete_digest(&store.connection, &empty).unwrap(),
        hash("")
    );
    let first_text = format!("{}中", "a".repeat(32765));
    let first = append(&mut store, empty, &first_text);
    let head = append(&mut store, first, "🙂e\u{301}尾");
    assert_eq!(
        complete_digest(&store.connection, &head).unwrap(),
        hash(&format!("{first_text}🙂e\u{301}尾"))
    );
    let mut head = head;
    for replacement in ["新🙂", ""] {
        let tx = store
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .unwrap();
        head = crate::canonical_stream_store::apply(
            &tx,
            &CanonicalStreamOperation::Replace {
                owner: owner(),
                expected_head: head,
                text: replacement.into(),
            },
        )
        .unwrap();
        tx.commit().unwrap();
        assert_eq!(
            complete_digest(&store.connection, &head).unwrap(),
            hash(replacement)
        );
    }
}

#[test]
fn complete_digest_reaudits_old_generation_after_a_later_transaction_changes_it() {
    let (_dir, mut store, empty) = fixture();
    let first = append(&mut store, empty, "old");
    let old_generation = first.generation;
    let old = append(&mut store, first, "tail");
    let tx = store
        .connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    let head = crate::canonical_stream_store::apply(
        &tx,
        &CanonicalStreamOperation::Replace {
            owner: owner(),
            expected_head: old,
            text: "current".into(),
        },
    )
    .unwrap();
    tx.commit().unwrap();
    assert_eq!(
        complete_digest(&store.connection, &head).unwrap(),
        format!("{:x}", Sha256::digest(b"current"))
    );
    require_complete(&store.connection, &head).unwrap();
    let tx = store
        .connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    tx.execute(
        "UPDATE canonical_stream_chunks SET text='bad' WHERE generation=?1 AND chunk_index=0",
        [integer(old_generation).unwrap()],
    )
    .unwrap();
    tx.commit().unwrap();
    assert_eq!(
        range(&store.connection, &head, 0, 32).unwrap().text,
        "current"
    );
    assert!(complete_digest(&store.connection, &head).is_err());
    assert!(require_complete(&store.connection, &head).is_err());
}

#[test]
fn complete_digest_rejects_extra_tail_in_replaced_generation() {
    let (_dir, mut store, empty) = fixture();
    let old = append(&mut store, empty, "old");
    let old_generation = old.generation;
    let tx = store
        .connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    let head = crate::canonical_stream_store::apply(
        &tx,
        &CanonicalStreamOperation::Replace {
            owner: owner(),
            expected_head: old,
            text: "new".into(),
        },
    )
    .unwrap();
    tx.commit().unwrap();
    complete_digest(&store.connection, &head).unwrap();
    store.connection.execute(
        "INSERT INTO canonical_stream_chunks SELECT part_id,generation,1,3,'extra',manifest_root FROM canonical_stream_chunks WHERE generation=?1 AND chunk_index=0",
        [integer(old_generation).unwrap()],
    ).unwrap();
    assert_eq!(range(&store.connection, &head, 0, 32).unwrap().text, "new");
    assert!(complete_digest(&store.connection, &head).is_err());
    assert!(require_complete(&store.connection, &head).is_err());
}

fn owner() -> CanonicalStreamOwner {
    CanonicalStreamOwner {
        conversation_id: "c".into(),
        run_id: "r".into(),
        message_id: "m".into(),
        part_id: "p".into(),
        field: PartBodyField::ContentText,
    }
}
fn fixture() -> (tempfile::TempDir, NolaStore, CanonicalStreamHead) {
    let dir = tempfile::tempdir().unwrap();
    let mut store = NolaStore::open(dir.path().join("read.sqlite")).unwrap();
    store.connection.execute_batch("INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms) VALUES('c','stream',0,1,1);
        INSERT INTO messages(id,conversation_id,run_id,role,status,ordinal,created_at_ms)
        VALUES('first','c','r','assistant','completed',0,1),('m','c','r','assistant','streaming',1,1);
        INSERT INTO runs(id,conversation_id,kind,assistant_message_id,status,next_event_sequence,created_at_ms,updated_at_ms)
        VALUES('r','c','conversation','first','running',1,1,1);
        INSERT INTO message_parts(id,message_id,kind,ordinal,state,created_at_ms,updated_at_ms)
        VALUES('p','m','text',0,'open',1,1);").unwrap();
    let tx = store
        .connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    let head = crate::canonical_stream_store::apply(
        &tx,
        &CanonicalStreamOperation::Declare { owner: owner() },
    )
    .unwrap();
    tx.commit().unwrap();
    (dir, store, head)
}
fn append(store: &mut NolaStore, head: CanonicalStreamHead, text: &str) -> CanonicalStreamHead {
    let tx = store
        .connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    let next = crate::canonical_stream_store::apply(
        &tx,
        &CanonicalStreamOperation::Append {
            owner: owner(),
            expected_head: head,
            delta: text.into(),
        },
    )
    .unwrap();
    tx.commit().unwrap();
    next
}

#[test]
fn missing_schema_or_head_is_absent_not_an_empty_stream() {
    let uninitialized = Connection::open_in_memory().unwrap();
    assert_eq!(resolve(&uninitialized, "c", "m", "p").unwrap(), None);
    let dir = tempfile::tempdir().unwrap();
    let store = NolaStore::open(dir.path().join("absent.sqlite")).unwrap();
    assert_eq!(resolve(&store.connection, "c", "m", "p").unwrap(), None);
    assert_eq!(resolve(&store.connection, "c", "m", "p").unwrap(), None);
}

#[test]
fn unicode_ranges_noninitial_assistant_close_and_stale_head() {
    let (_dir, mut store, empty) = fixture();
    assert_eq!(
        resolve(&store.connection, "c", "m", "p").unwrap(),
        Some(empty.clone())
    );
    require_complete(&store.connection, &empty).unwrap();
    let first = append(&mut store, empty.clone(), "a🙂");
    let head = append(&mut store, first, "中e\u{301}!");
    assert!(range(&store.connection, &empty, 0, 4).is_err());
    require_complete(&store.connection, &head).unwrap();
    let mut offset = 0;
    let mut text = String::new();
    while offset < head.total_utf8_bytes {
        let page = range(&store.connection, &head, offset, 4).unwrap();
        assert!(page.text.len() <= 4);
        assert!(page.next_offset_bytes > offset);
        offset = page.next_offset_bytes;
        text.push_str(&page.text);
    }
    assert_eq!(text, "a🙂中e\u{301}!");
    assert!(range(&store.connection, &head, 2, 4).is_err());
    let tx = store
        .connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    let closed = crate::canonical_stream_store::apply(
        &tx,
        &CanonicalStreamOperation::Close {
            owner: owner(),
            expected_head: head,
        },
    )
    .unwrap();
    tx.execute(
        "UPDATE message_parts SET state='completed' WHERE id='p'",
        [],
    )
    .unwrap();
    tx.commit().unwrap();
    assert_eq!(
        range(&store.connection, &closed, 0, 100).unwrap().text,
        text
    );
    require_complete(&store.connection, &closed).unwrap();
    assert!(resolve(&store.connection, "foreign", "m", "p").is_err());
}

#[test]
fn replacement_keeps_old_ancestry_and_complete_audits_it() {
    let (_dir, mut store, head) = fixture();
    let first = append(&mut store, head, "old");
    let tx = store
        .connection
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .unwrap();
    let replaced = crate::canonical_stream_store::apply(
        &tx,
        &CanonicalStreamOperation::Replace {
            owner: owner(),
            expected_head: first,
            text: "new".into(),
        },
    )
    .unwrap();
    tx.commit().unwrap();
    assert_eq!(
        range(&store.connection, &replaced, 0, 4).unwrap().text,
        "new"
    );
    require_complete(&store.connection, &replaced).unwrap();
    store
        .connection
        .execute(
            "UPDATE canonical_stream_chunks SET text='bad' WHERE generation=1",
            [],
        )
        .unwrap();
    // Local range does not claim to audit an old generation's complete ancestry.
    assert_eq!(
        range(&store.connection, &replaced, 0, 4).unwrap().text,
        "new"
    );
    assert!(require_complete(&store.connection, &replaced).is_err());
}

#[test]
fn corrupt_bytes_missing_chunks_manifest_and_extra_tail_fail() {
    for attack in [
        "UPDATE canonical_stream_chunks SET text='xxx' WHERE chunk_index=0",
        "DELETE FROM canonical_stream_chunks WHERE chunk_index=0",
        "UPDATE canonical_stream_chunks SET offset_bytes=1 WHERE chunk_index=0",
        "UPDATE canonical_stream_manifests SET previous_root='missing' WHERE operation='append'",
        "INSERT INTO canonical_stream_chunks SELECT part_id,generation,2,6,'extra',manifest_root FROM canonical_stream_chunks WHERE chunk_index=1",
    ] {
        let (_dir,mut store,head)=fixture();
        let head=append(&mut store,head,"abc");
        let head=append(&mut store,head,"def");
        store.connection.execute_batch(attack).unwrap();
        assert!(range(&store.connection,&head,0,6).is_err(),"{attack}");
        assert!(require_complete(&store.connection,&head).is_err(),"{attack}");
    }
}

#[test]
fn persisted_owner_inline_conflict_and_oversize_head_fail() {
    for attack in [
        "UPDATE messages SET role='user' WHERE id='m'",
        "UPDATE messages SET conversation_id='other' WHERE id='m'",
        "UPDATE messages SET run_id='foreign-run' WHERE id='m'",
        "UPDATE message_parts SET kind='tool' WHERE id='p'",
        "UPDATE message_parts SET content_text='duplicate' WHERE id='p'",
        "PRAGMA ignore_check_constraints=ON; UPDATE canonical_stream_heads SET head_json=printf('%5000s','x')",
    ] {
        let (_dir,store,_head)=fixture();
        store.connection.execute("INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms) VALUES('other','other',0,1,1)",[]).unwrap();
        store.connection.execute_batch(attack).unwrap();
        assert!(resolve(&store.connection,"c","m","p").is_err(),"{attack}");
    }
}

#[test]
fn tail_checks_only_bounded_suffix_and_full_audit_detects_old_corruption() {
    let (_dir, mut store, mut head) = fixture();
    for _ in 0..8 {
        head = append(&mut store, head, &"a".repeat(32768));
    }
    head = append(&mut store, head, "中🙂e\u{301}尾");
    assert_eq!(tail(&store.connection, &head, 3).unwrap(), "…e\u{301}尾");
    store
        .connection
        .execute(
            "UPDATE canonical_stream_chunks SET text=?1 WHERE chunk_index=0",
            ["b".repeat(32768)],
        )
        .unwrap();
    assert_eq!(tail(&store.connection, &head, 3).unwrap(), "…e\u{301}尾");
    assert!(require_complete(&store.connection, &head).is_err());
    assert!(tail(&store.connection, &head, 0).is_err());
    store
        .connection
        .execute_batch(
            "INSERT INTO conversation_projection_operations VALUES('hide','c','delete',NULL,2,1,3);
        INSERT INTO message_projection_events VALUES('hidden','hide','c','m',0,'deleted',3);",
        )
        .unwrap();
    assert!(resolve(&store.connection, "c", "m", "p").is_err());
    assert!(tail(&store.connection, &head, 3).is_err());
}

#[test]
fn extra_tail_checks_use_two_indexed_bounds_not_generation_scans() {
    let (_dir, store, _head) = fixture();
    let mut query = store
        .connection
        .prepare(&format!("EXPLAIN QUERY PLAN {CHECK_TAIL_SQL}"))
        .unwrap();
    let details: Vec<String> = query
        .query_map(params!["p", 1, 5, 100], |row| row.get(3))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(
        details
            .iter()
            .any(|s| s.contains("SEARCH") && s.contains("chunk_index>")),
        "{details:?}"
    );
    assert!(
        details
            .iter()
            .any(|s| s.contains("SEARCH") && s.contains("offset_bytes>")),
        "{details:?}"
    );
    assert!(
        !details
            .iter()
            .any(|s| s.contains("SCAN canonical_stream_chunks")),
        "{details:?}"
    );
}
