// SPDX-License-Identifier: Apache-2.0
use super::*;
use crate::{params, NolaStore};

fn install_json(store: &NolaStore, original: &str, sealed: bool) {
    if !sealed {
        store
            .connection
            .execute(
                "UPDATE message_parts SET content_json=?1 WHERE id='p'",
                [original],
            )
            .unwrap();
        return;
    }
    let mut chunks = Vec::new();
    let mut start = 0;
    while start < original.len() {
        let mut end = (start + 4096).min(original.len());
        while !original.is_char_boundary(end) {
            end -= 1;
        }
        chunks.push((start, &original[start..end]));
        start = end;
    }
    let reference = CanonicalBodyReference::for_field(
        "json-body".into(),
        "c".into(),
        "m".into(),
        "p".into(),
        PartBodyField::ContentJson,
        original.len() as u64,
        chunks.len() as u32,
    )
    .unwrap();
    let encoded = serde_json::to_string(&reference).unwrap();
    store
        .connection
        .execute(
            "INSERT INTO canonical_text_objects VALUES('json-body',?1)",
            [&encoded],
        )
        .unwrap();
    for (offset, text) in chunks {
        store
            .connection
            .execute(
                "INSERT INTO canonical_text_chunks VALUES('json-body',?1,?2)",
                params![offset, text],
            )
            .unwrap();
    }
    store
        .connection
        .execute(
            "INSERT INTO message_part_body_references VALUES('p','content_json',?1)",
            [&encoded],
        )
        .unwrap();
}

#[test]
fn json_absent_null_and_empty_object_are_distinct_from_zero_budget() {
    let dir = tempfile::tempdir().unwrap();
    let store = setup(&dir.path().join("absent"));
    let reader =
        CanonicalFieldReader::open(&store.connection, "c", "m", "p", PartBodyField::ContentJson)
            .unwrap();
    assert_eq!(
        reader.read_json(Some(0)).unwrap(),
        CanonicalJsonRead::Absent
    );
    drop(reader);
    let text =
        CanonicalFieldReader::open(&store.connection, "c", "m", "p", PartBodyField::ContentText)
            .unwrap();
    assert!(text.read_json(None).is_err());
    drop(text);
    for sealed in [false, true] {
        for (index, original) in ["null", "{}"].into_iter().enumerate() {
            let store = setup(&dir.path().join(format!("{sealed}-{index}")));
            install_json(&store, original, sealed);
            let reader = CanonicalFieldReader::open(
                &store.connection,
                "c",
                "m",
                "p",
                PartBodyField::ContentJson,
            )
            .unwrap();
            assert_eq!(
                reader.read_json(Some(0)).unwrap(),
                CanonicalJsonRead::Deferred {
                    total_utf8_bytes: original.len() as u64
                }
            );
            assert_eq!(
                reader.read_json(None).unwrap(),
                CanonicalJsonRead::Complete(original.into())
            );
            assert_eq!(
                reader.read_json(Some(original.len())).unwrap(),
                CanonicalJsonRead::Complete(original.into())
            );
        }
    }
}

#[test]
fn json_inline_and_sealed_keep_exact_unicode_numeric_and_escape_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let original = format!(
        r#" {{ "seed":9007199254740993,"max":9223372036854775807,"min":-9223372036854775808,"decimal":1.2300e+02,"escaped":"\u4f60","text":"{}🙂" }} "#,
        "你好".repeat(12000)
    );
    for sealed in [false, true] {
        let store = setup(&dir.path().join(format!("{sealed}")));
        install_json(&store, &original, sealed);
        let reader = CanonicalFieldReader::open(
            &store.connection,
            "c",
            "m",
            "p",
            PartBodyField::ContentJson,
        )
        .unwrap();
        assert_eq!(
            reader.read_json(Some(original.len() - 1)).unwrap(),
            CanonicalJsonRead::Deferred {
                total_utf8_bytes: original.len() as u64
            }
        );
        assert_eq!(
            reader.read_json(Some(original.chars().count())).unwrap(),
            CanonicalJsonRead::Deferred {
                total_utf8_bytes: original.len() as u64
            }
        );
        assert_eq!(
            reader.read_json(Some(original.len())).unwrap(),
            CanonicalJsonRead::Complete(original.clone())
        );
        assert_eq!(
            reader.read_json(None).unwrap(),
            CanonicalJsonRead::Complete(original.clone())
        );
    }
}

#[test]
fn json_reader_preserves_owner_reference_and_field_separation() {
    let dir = tempfile::tempdir().unwrap();
    let store = setup(&dir.path().join("db"));
    install_json(&store, r#"{"original":true}"#, true);
    assert!(CanonicalFieldReader::open(
        &store.connection,
        "foreign",
        "m",
        "p",
        PartBodyField::ContentJson
    )
    .is_err());
    let text =
        CanonicalFieldReader::open(&store.connection, "c", "m", "p", PartBodyField::ContentText)
            .unwrap();
    assert_eq!(text.read_text(0).unwrap(), "你好 old");
    assert!(text.read_json(None).is_err());
    drop(text);
    store
        .connection
        .execute(
            "UPDATE message_parts SET content_json='{}' WHERE id='p'",
            [],
        )
        .unwrap();
    assert!(CanonicalFieldReader::open(
        &store.connection,
        "c",
        "m",
        "p",
        PartBodyField::ContentJson
    )
    .is_err());
    store
        .connection
        .execute(
            "UPDATE message_parts SET content_json=NULL WHERE id='p'",
            [],
        )
        .unwrap();
    store
        .connection
        .execute(
            "DELETE FROM canonical_text_chunks WHERE content_id='json-body'",
            [],
        )
        .unwrap();
    let deferred =
        CanonicalFieldReader::open(&store.connection, "c", "m", "p", PartBodyField::ContentJson)
            .unwrap();
    assert_eq!(
        deferred.read_json(Some(0)).unwrap(),
        CanonicalJsonRead::Deferred {
            total_utf8_bytes: r#"{"original":true}"#.len() as u64,
        }
    );
    drop(deferred);
    let result =
        CanonicalFieldReader::open(&store.connection, "c", "m", "p", PartBodyField::ContentJson)
            .and_then(|reader| reader.read_json(None));
    assert!(result.is_err());
}

fn setup(path: &std::path::Path) -> NolaStore {
    let store = NolaStore::open(path).unwrap();
    store.connection.execute_batch("INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms) VALUES('c','Fields',1,1,1);
        INSERT INTO messages(id,conversation_id,role,status,ordinal,created_at_ms,completed_at_ms) VALUES('m','c','user','completed',0,1,1);
        INSERT INTO message_parts(id,message_id,kind,ordinal,state,content_text,content_json,created_at_ms,updated_at_ms) VALUES('p','m','text',0,'completed','你好 old',NULL,1,1);").unwrap();
    store
}
fn seal(store: &NolaStore) {
    let reference =
        CanonicalBodyReference::for_field("body".into(), "c".into(), "m".into(), "p".into(), nola_domain::PartBodyField::ContentText, 9, 2)
            .unwrap();
    let encoded = serde_json::to_string(&reference).unwrap();
    store
        .connection
        .execute(
            "INSERT INTO canonical_text_objects VALUES('body',?1)",
            [&encoded],
        )
        .unwrap();
    store
        .connection
        .execute(
            "INSERT INTO canonical_text_chunks VALUES('body',0,'你好'),('body',6,'end')",
            [],
        )
        .unwrap();
    store
        .connection
        .execute("UPDATE message_parts SET content_text='' WHERE id='p'", [])
        .unwrap();
    store
        .connection
        .execute(
            "INSERT INTO message_part_body_references VALUES('p','content_text',?1)",
            [&encoded],
        )
        .unwrap();
}

#[test]
fn absent_empty_inline_and_exact_field_identity_remain_distinct() {
    let dir = tempfile::tempdir().unwrap();
    let store = setup(&dir.path().join("db"));
    let text =
        CanonicalFieldReader::open(&store.connection, "c", "m", "p", PartBodyField::ContentText)
            .unwrap();
    assert!(matches!(
        text.backing(),
        CanonicalFieldBacking::Inline { .. }
    ));
    assert_eq!(text.read_range(0, 6).unwrap().text, "你好");
    assert!(text.read_range(1, 4).is_err());
    drop(text);
    let json =
        CanonicalFieldReader::open(&store.connection, "c", "m", "p", PartBodyField::ContentJson)
            .unwrap();
    assert_eq!(json.total_utf8_bytes(), None);
    assert!(json.read_range(0, 4).is_err());
    drop(json);
    store
        .connection
        .execute(
            "UPDATE message_parts SET content_text='',content_json='null'",
            [],
        )
        .unwrap();
    let text =
        CanonicalFieldReader::open(&store.connection, "c", "m", "p", PartBodyField::ContentText)
            .unwrap();
    assert_eq!(text.total_utf8_bytes(), Some(0));
    assert!(text.read_range(0, 4).unwrap().text.is_empty());
    drop(text);
    let json =
        CanonicalFieldReader::open(&store.connection, "c", "m", "p", PartBodyField::ContentJson)
            .unwrap();
    assert_eq!(json.read_range(0, 4).unwrap().text, "null");
    assert!(json.read_text(0).is_err());
    drop(json);
    assert!(CanonicalFieldReader::open(
        &store.connection,
        "foreign",
        "m",
        "p",
        PartBodyField::ContentText
    )
    .is_err());
    assert!(CanonicalFieldReader::open(
        &store.connection,
        "c",
        "wrong",
        "p",
        PartBodyField::ContentText
    )
    .is_err());
}

#[test]
fn sealed_fields_keep_finalization_and_independent_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let store = setup(&dir.path().join("db"));
    seal(&store);
    let reader =
        CanonicalFieldReader::open(&store.connection, "c", "m", "p", PartBodyField::ContentText)
            .unwrap();
    assert!(matches!(reader.backing(), CanonicalFieldBacking::Sealed(_)));
    assert_eq!(reader.read_text(2).unwrap(), "…nd");
    drop(reader);
    store
        .connection
        .execute("UPDATE messages SET status='streaming' WHERE id='m'", [])
        .unwrap();
    assert!(CanonicalFieldReader::open(
        &store.connection,
        "c",
        "m",
        "p",
        PartBodyField::ContentText
    )
    .is_err());
    store
        .connection
        .execute("UPDATE messages SET status='completed' WHERE id='m'", [])
        .unwrap();
    store
        .connection
        .execute("UPDATE message_parts SET state='open' WHERE id='p'", [])
        .unwrap();
    assert!(CanonicalFieldReader::open(
        &store.connection,
        "c",
        "m",
        "p",
        PartBodyField::ContentText
    )
    .is_err());
    store
        .connection
        .execute(
            "UPDATE message_parts SET state='completed',content_json='{}' WHERE id='p'",
            [],
        )
        .unwrap();
    let text =
        CanonicalFieldReader::open(&store.connection, "c", "m", "p", PartBodyField::ContentText)
            .unwrap();
    assert_eq!(text.read_text(2).unwrap(), "…nd");
    assert!(text.read_json(None).is_err());
    drop(text);
    let json =
        CanonicalFieldReader::open(&store.connection, "c", "m", "p", PartBodyField::ContentJson)
            .unwrap();
    assert_eq!(
        json.read_json(None).unwrap(),
        CanonicalJsonRead::Complete("{}".into())
    );
    assert!(json.read_text(0).is_err());
    drop(json);
    store
        .connection
        .execute(
            "UPDATE message_parts SET content_json=NULL,content_text='conflict' WHERE id='p'",
            [],
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
}

#[test]
fn field_view_pins_revision_and_does_not_complete_parent_write_transaction() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let mut reader_store = setup(&path);
    let writer = Connection::open(&path).unwrap();
    let reader = CanonicalFieldReader::open(
        &reader_store.connection,
        "c",
        "m",
        "p",
        PartBodyField::ContentText,
    )
    .unwrap();
    writer
        .execute(
            "UPDATE message_parts SET content_text='new body',updated_at_ms=2 WHERE id='p'",
            [],
        )
        .unwrap();
    assert_eq!(reader.read_text(0).unwrap(), "你好 old");
    drop(reader);
    let reader = CanonicalFieldReader::open(
        &reader_store.connection,
        "c",
        "m",
        "p",
        PartBodyField::ContentText,
    )
    .unwrap();
    assert_eq!(reader.read_text(0).unwrap(), "new body");
    drop(reader);
    reader_store.begin_write_unit_of_work().unwrap();
    reader_store
        .connection
        .execute(
            "UPDATE message_parts SET content_text=?1 WHERE id='p'",
            params!["inside"],
        )
        .unwrap();
    {
        let reader = CanonicalFieldReader::open(
            &reader_store.connection,
            "c",
            "m",
            "p",
            PartBodyField::ContentText,
        )
        .unwrap();
        assert_eq!(reader.read_text(0).unwrap(), "inside");
    }
    assert!(reader_store.has_write_unit_of_work());
    reader_store.rollback_write_unit_of_work().unwrap();
}

#[test]
fn inline_unicode_tails_preserve_character_limits_at_every_byte_alignment() {
    let directory = tempfile::tempdir().unwrap();
    let store = setup(&directory.path().join("db"));
    for original in ["", "a", "🙂中aéz", "中🙂文🦀", "long-prefix-🙂中é"] {
        store.connection.execute("UPDATE message_parts SET content_text=?1 WHERE id='p'", [original]).unwrap();
        let reader = CanonicalFieldReader::open(&store.connection, "c", "m", "p", PartBodyField::ContentText).unwrap();
        assert_eq!(reader.read_text(0).unwrap(), original);
        for limit in 1..=original.chars().count() + 1 {
            let count = original.chars().count();
            let suffix: String = original.chars().skip(count.saturating_sub(limit)).collect();
            let expected = if count > limit { format!("…{suffix}") } else { suffix };
            assert_eq!(reader.read_text(limit).unwrap(), expected, "original={original:?}, limit={limit}");
        }
    }
}
