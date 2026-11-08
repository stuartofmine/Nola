// SPDX-License-Identifier: Apache-2.0
//! Narrow planner reads authorize exact current refs and never read chunk bodies.
use super::*;

#[test]
fn planning_manifest_is_bounded_owner_exact_and_does_not_read_bodies() {
    let directory = tempfile::tempdir().unwrap();
    let mut store = NolaStore::open(directory.path().join("planning-manifest.db")).unwrap();
    let id = begin(&mut store, "planning");
    declare(&mut store, &id);
    let part = format!("{}-journal-part", id.run_id);
    for (revision, value) in [(3, "首🙂"), (4, "尾🙂")] {
        store.begin_write_unit_of_work().unwrap();
        transition(&mut store, &id, revision);
        let input = append_input(&store, &id, revision, value);
        store.commit_assistant_journal(&input).unwrap();
        store.commit_write_unit_of_work().unwrap();
    }
    let reference = store
        .assistant_journal_part(&id, &part)
        .unwrap()
        .unwrap()
        .bodies
        .remove(0);
    let first = store
        .assistant_journal_content_manifest(&id, &part, &reference, 0, 1)
        .unwrap();
    assert_eq!(first.chunks.len(), 1);
    assert_eq!(first.next_chunk, 1);
    assert!(!first.complete);
    let second = store
        .assistant_journal_content_manifest(&id, &part, &reference, 1, 1)
        .unwrap();
    assert_eq!(second.next_chunk, 2);
    assert!(second.complete);
    assert_eq!(
        first.chunks[0].utf8_bytes,
        JournalNumber("首🙂".len() as u64)
    );
    let at_end = store
        .assistant_journal_content_manifest(&id, &part, &reference, 2, 1)
        .unwrap();
    assert!(at_end.chunks.is_empty() && at_end.complete);
    for (after, limit) in [(0, 0), (0, ASSISTANT_PAGE_MAX_RECORDS + 1), (3, 1)] {
        assert!(store
            .assistant_journal_content_manifest(&id, &part, &reference, after, limit)
            .is_err());
    }
    for foreign in [
        AssistantJournalIdentity {
            source_store_epoch: "foreign".into(),
            ..id.clone()
        },
        AssistantJournalIdentity {
            run_id: "foreign".into(),
            ..id.clone()
        },
        AssistantJournalIdentity {
            assistant_message_id: "foreign".into(),
            ..id.clone()
        },
        AssistantJournalIdentity {
            conversation_id: "foreign".into(),
            ..id.clone()
        },
    ] {
        assert!(store
            .assistant_journal_content_manifest(&foreign, &part, &reference, 0, 1)
            .is_err());
    }
    assert!(store
        .assistant_journal_content_manifest(&id, "foreign", &reference, 0, 1)
        .is_err());
    for foreign in [
        JournalContentRef {
            content_generation: JournalNumber(2),
            ..reference.clone()
        },
        JournalContentRef {
            chunk_count: JournalNumber(1),
            ..reference.clone()
        },
        JournalContentRef {
            field: JournalContentField::Title,
            ..reference.clone()
        },
    ] {
        assert!(store
            .assistant_journal_content_manifest(&id, &part, &foreign, 0, 1)
            .is_err());
    }
    store.begin_write_unit_of_work().unwrap();
    let unpublished = store
        .prepare_assistant_content(
            &id,
            &part,
            &empty_journal_content(&id, &part, JournalContentField::Markdown, JournalNumber(99))
                .unwrap(),
            &[AssistantTextChunkV1::new("unpublished".into()).unwrap()],
        )
        .unwrap();
    assert!(store
        .assistant_journal_content_manifest(&id, &part, &unpublished, 0, 1)
        .is_err());
    store.rollback_write_unit_of_work().unwrap();
    // Isolated corruption fixture: manifest-only queries do not fetch bodies.
    store
        .connection
        .execute(
            "UPDATE assistant_journal_chunks SET bytes=CAST('corrupt' AS BLOB)",
            [],
        )
        .unwrap();
    assert_eq!(
        store
            .assistant_journal_content_manifest(&id, &part, &reference, 0, 1)
            .unwrap()
            .chunks,
        first.chunks
    );
    store
        .connection
        .execute(
            "DELETE FROM assistant_journal_manifest_chunks WHERE chunk_index=0",
            [],
        )
        .unwrap();
    assert!(store
        .assistant_journal_content_manifest(&id, &part, &reference, 0, 1)
        .is_err());
}
