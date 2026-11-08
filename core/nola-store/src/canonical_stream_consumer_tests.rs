// SPDX-License-Identifier: Apache-2.0
//! Real consumers of the private stream writer; no model or network fixture.
use crate::{canonical_field_reader::CanonicalFieldReader, NolaStore};
#[path = "../tests/support/terminal_sources.rs"]
mod terminal_sources;
use nola_domain::{
    CanonicalBranchIntent, CanonicalBranchTarget, CanonicalStreamOperation, CanonicalStreamOwner,
    PartBodyField,
};

fn owner() -> CanonicalStreamOwner {
    CanonicalStreamOwner {
        conversation_id: "c".into(),
        run_id: "r".into(),
        message_id: "m".into(),
        part_id: "p".into(),
        field: PartBodyField::ContentText,
    }
}

fn fixture(path: &std::path::Path, chunks: &[&str]) -> NolaStore {
    let mut store = NolaStore::open(path).unwrap();
    store.connection.execute_batch("INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms) VALUES('c','stream',1,1,1);
        INSERT INTO messages(id,conversation_id,run_id,role,status,ordinal,created_at_ms)
        VALUES('m','c','r','assistant','streaming',1,1);
        INSERT INTO runs(id,conversation_id,kind,assistant_message_id,status,next_event_sequence,created_at_ms,updated_at_ms)
        VALUES('r','c','conversation','m','running',1,1,1);
        INSERT INTO message_parts(id,message_id,kind,ordinal,state,created_at_ms,updated_at_ms)
        VALUES('p','m','text',0,'open',1,1);").unwrap();
    let tx = store.connection.transaction().unwrap();
    let mut head = crate::canonical_stream_store::apply(
        &tx,
        &CanonicalStreamOperation::Declare { owner: owner() },
    )
    .unwrap();
    for text in chunks {
        head = crate::canonical_stream_store::apply(
            &tx,
            &CanonicalStreamOperation::Append {
                owner: owner(),
                expected_head: head,
                delta: (*text).into(),
            },
        )
        .unwrap();
    }
    crate::canonical_stream_store::apply(
        &tx,
        &CanonicalStreamOperation::Close {
            owner: owner(),
            expected_head: head,
        },
    )
    .unwrap();
    tx.execute_batch(
        "UPDATE message_parts SET state='completed' WHERE id='p';
        UPDATE messages SET status='completed',completed_at_ms=2 WHERE id='m';",
    )
    .unwrap();
    crate::canonical_text_disclosure::flush(&tx).unwrap();
    tx.commit().unwrap();
    store
}

#[test]
fn chunked_consumers_hydrate_unicode_prefix_tail_ranges_and_cross_chunk_search() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("consumer.db");
    let store = fixture(&path, &["前界", "🙂后", "尾"]);
    drop(store);
    let store = NolaStore::open(&path).unwrap();
    let reader =
        CanonicalFieldReader::open(&store.connection, "c", "m", "p", PartBodyField::ContentText)
            .unwrap();
    assert_eq!(reader.total_utf8_bytes(), Some("前界🙂后尾".len() as u64));
    assert_eq!(reader.read_text(0).unwrap(), "前界🙂后尾");
    assert_eq!(reader.read_text_prefix(3).unwrap(), "前界🙂");
    assert_eq!(reader.read_text(2).unwrap(), "…后尾");
    let range = reader.read_range(3, 7).unwrap();
    assert_eq!(range.text, "界🙂");
    assert_eq!(range.next_offset_bytes, 10);
    let (snippet_offset, snippet) = reader.find_literal("界🙂").unwrap().unwrap();
    assert_eq!(snippet_offset, 0);
    assert!(snippet.contains("界🙂"));
    assert!(reader.find_literal("不存在").unwrap().is_none());
    let full = store.load_conversation("c").unwrap().unwrap();
    assert_eq!(full.messages[0].parts[0].content_text, "前界🙂后尾");
    let tail = store
        .load_conversation_tail_snapshot("c", 1, 2, 128)
        .unwrap()
        .unwrap();
    assert_eq!(tail.conversation.messages[0].parts[0].content_text, "…后尾");
}

#[test]
fn chunked_field_rejects_wrong_owner_and_hidden_source() {
    let dir = tempfile::tempdir().unwrap();
    let store = fixture(&dir.path().join("owner.db"), &["private"]);
    for (conversation, message, part) in [
        ("other", "m", "p"),
        ("c", "other", "p"),
        ("c", "m", "other"),
    ] {
        assert!(CanonicalFieldReader::open(
            &store.connection,
            conversation,
            message,
            part,
            PartBodyField::ContentText
        )
        .is_err());
    }
    store
        .connection
        .execute_batch(
            "INSERT INTO conversation_projection_operations VALUES('hide','c','delete',NULL,2,1,3);
        INSERT INTO message_projection_events VALUES('hidden','hide','c','m',0,'deleted',3);",
        )
        .unwrap();
    assert!(CanonicalFieldReader::open(
        &store.connection,
        "c",
        "m",
        "p",
        PartBodyField::ContentText
    )
    .is_err());
    assert!(store
        .load_conversation("c")
        .unwrap()
        .unwrap()
        .messages
        .is_empty());
}

#[test]
fn branch_copies_closed_stream_independently_of_deleted_source() {
    let dir = tempfile::tempdir().unwrap();
    for large in [false, true] {
        let path = dir.path().join(format!("branch-{large}.db"));
        let block = "a".repeat(20_000);
        let chunks = if large {
            vec![
                block.as_str(),
                block.as_str(),
                block.as_str(),
                block.as_str(),
                "界🙂尾",
            ]
        } else {
            vec!["前界", "🙂尾"]
        };
        let expected = chunks.concat();
        let mut store = fixture(&path, &chunks);
        // Complete the source execution through original decision facts and a
        // real journal seal; a closed stream alone is not a terminal Run.
        store.connection.execute("INSERT INTO messages(id,conversation_id,run_id,role,status,ordinal,created_at_ms)
            VALUES('u','c','r','user','completed',0,1)", []).unwrap();
        store.save_checkpoint(&crate::SaveCheckpointInput {
            conversation_id: "c".into(), expected_revision: 0, revision: 1,
            event_cursor: 0, checkpoint_json: "{}".into(), updated_at_ms: 1,
        }).unwrap();
        terminal_sources::settle_fixture(&mut store, "c", "r", "r", "u");
        let source_revision = store.connection.query_row("SELECT revision FROM conversations WHERE id='c'", [], |row| row.get(0)).unwrap();
        store
            .create_conversation_branch(&CanonicalBranchIntent {
                operation_id: "branch".into(),
                source_conversation_id: "c".into(),
                expected_source_revision: source_revision,
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
        store.delete_conversation("c").unwrap();
        drop(store);
        let store = NolaStore::open(&path).unwrap();
        let restored = store.load_conversation("target").unwrap().unwrap();
        let assistant = restored.messages.iter().find(|message| message.role == "assistant").unwrap();
        assert_eq!(assistant.parts[0].content_text, expected);
        let part = &assistant.parts[0];
        let reader = CanonicalFieldReader::open(
            &store.connection,
            "target",
            &assistant.id,
            &part.id,
            PartBodyField::ContentText,
        )
        .unwrap();
        if large {
            assert!(
                reader.sealed_reference().is_some(),
                "large stream branch must own its immutable copy"
            );
        }
        assert_eq!(reader.read_text(0).unwrap(), expected);
    }
}

#[test]
fn native_export_manifest_and_bounded_ranges_preserve_original_stream_bytes() {
    use nola_store_port::transcript_export::{ExportBodyField, TranscriptExportRepository};
    let dir = tempfile::tempdir().unwrap();
    let block = "你🙂".repeat(4000);
    let chunks = [block.as_str(), "跨", "🙂chunk", block.as_str()];
    let expected = chunks.concat();
    let store = fixture(&dir.path().join("export.db"), &chunks);
    let page = store.load_export_page("c", None, None).unwrap().unwrap();
    assert_eq!(page.messages.len(), 1);
    assert_eq!(
        page.messages[0].parts[0].content_text_utf8_bytes,
        expected.len() as u64
    );
    assert_eq!(page.messages[0].parts[0].content_json_utf8_bytes, None);
    let mut bytes = Vec::new();
    let mut offset = 0;
    while offset < expected.len() as u64 {
        let range = store
            .read_export_range(
                "c",
                "m",
                "p",
                ExportBodyField::ContentText,
                page.conversation.revision,
                offset,
                17,
            )
            .unwrap();
        assert!(range.text.len() <= 17);
        assert!(range.next_offset_bytes > offset);
        bytes.extend_from_slice(range.text.as_bytes());
        offset = range.next_offset_bytes;
    }
    assert_eq!(bytes, expected.as_bytes());
}

#[test]
fn corrupt_stream_cannot_be_resealed_as_a_valid_branch() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = fixture(&dir.path().join("corrupt.db"), &["original", "tail"]);
    store.connection.execute("UPDATE canonical_stream_chunks SET text='tampered' WHERE part_id='p' AND chunk_index=0", []).unwrap();
    let counts = || {
        store.connection.query_row("SELECT (SELECT count(*) FROM conversations),(SELECT count(*) FROM canonical_text_objects),(SELECT count(*) FROM message_part_body_references),(SELECT count(*) FROM sync_outbox)", [],
        |row| Ok((row.get::<_, i64>(0)?,row.get::<_, i64>(1)?,row.get::<_, i64>(2)?,row.get::<_, i64>(3)?))).unwrap()
    };
    let before = counts();
    let intent = CanonicalBranchIntent {
        operation_id: "bad-copy".into(),
        source_conversation_id: "c".into(),
        expected_source_revision: 1,
        through_message_id: "m".into(),
        conversation: CanonicalBranchTarget {
            id: "target".into(),
            title: "Target".into(),
            settings_json: None,
            created_at_ms: 3,
            updated_at_ms: 3,
        },
        committed_at_ms: 3,
    };
    assert!(store.create_conversation_branch(&intent).is_err());
    assert!(store.load_conversation("target").unwrap().is_none());
    let after = store.connection.query_row("SELECT (SELECT count(*) FROM conversations),(SELECT count(*) FROM canonical_text_objects),(SELECT count(*) FROM message_part_body_references),(SELECT count(*) FROM sync_outbox)", [],
        |row| Ok((row.get::<_, i64>(0)?,row.get::<_, i64>(1)?,row.get::<_, i64>(2)?,row.get::<_, i64>(3)?))).unwrap();
    assert_eq!(before, after);
}
