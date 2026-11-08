// SPDX-License-Identifier: Apache-2.0
//! Isolated fixtures exercise dormant field-aware consumers. Production JSON
//! publication remains closed; fixtures bind prepared objects directly by SQL.
use nola_domain::{CanonicalBodyReference, PartBodyField};
use nola_store::NolaStore;
use nola_store_port::{canonical_body::CanonicalBodyRepository, transcript_export::*};
use rusqlite::{params, Connection};

fn fixture(path: &std::path::Path) -> (NolaStore, Connection) {
    let store = NolaStore::open(path).unwrap();
    let sql = Connection::open(path).unwrap();
    sql.execute_batch(
        "PRAGMA foreign_keys=ON;
        INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms) VALUES('c','Original fields',1,1,9);
        INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms) VALUES('other','Other owner',1,1,9);
        INSERT INTO messages VALUES('m','c',NULL,'assistant','completed',1,2,8);",
    )
    .unwrap();
    (store, sql)
}
fn part(sql: &Connection, id: &str, ordinal: i64, json: Option<&str>) {
    sql.execute("INSERT INTO message_parts(id,message_id,kind,ordinal,state,content_text,content_json,content_digest,created_at_ms,updated_at_ms) VALUES(?1,'m','tool_result',?2,'completed','',?3,'retained-digest',3,7)",params![id,ordinal,json]).unwrap();
}
fn reference(
    id: &str,
    part: &str,
    field: PartBodyField,
    bytes: u64,
    count: u32,
) -> CanonicalBodyReference {
    CanonicalBodyReference::for_field(
        id.into(),
        "c".into(),
        "m".into(),
        part.into(),
        field,
        bytes,
        count,
    )
    .unwrap()
}
fn bind(sql: &Connection, reference: &CanonicalBodyReference) {
    let field = match reference.field() {
        PartBodyField::ContentText => "content_text",
        PartBodyField::ContentJson => "content_json",
    };
    sql.execute(
        "INSERT INTO message_part_body_references(part_id,field,reference_json) VALUES(?1,?2,?3)",
        params![
            reference.part_id(),
            field,
            serde_json::to_string(reference).unwrap()
        ],
    )
    .unwrap();
}
fn source(total: u64, json: bool) -> impl Iterator<Item = Result<(u64, String), String>> {
    (0..total.div_ceil(32768)).map(move |index| {
        let offset = index * 32768;
        let mut body = "x".repeat((total - offset).min(32768) as usize);
        let length = body.len();
        if length >= 12 {
            body.replace_range(length - 8..length - 1, "中😀");
        }
        if json && offset == 0 {
            body.replace_range(0..1, "\"");
        }
        if json && offset + length as u64 == total {
            body.replace_range(length - 1..length, "\"");
        }
        Ok((offset, body))
    })
}

#[test]
fn same_part_has_two_ten_million_byte_original_fields_without_duplicate_export_metadata_or_full_body_frames(
) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("dual.db");
    let (mut store, sql) = fixture(&path);
    part(&sql, "p", 1, None);
    let total = 10_000_000;
    for field in [PartBodyField::ContentText, PartBodyField::ContentJson] {
        let json = field == PartBodyField::ContentJson;
        let reference = reference(
            if json { "json" } else { "text" },
            "p",
            field,
            total,
            total.div_ceil(32768) as u32,
        );
        store
            .prepare_body(&reference, &mut source(total, json), 30_000_000)
            .unwrap();
        bind(&sql, &reference);
    }
    drop(store);
    let store = NolaStore::open(&path).unwrap();
    let page = store.load_export_page("c", None, None).unwrap().unwrap();
    assert_eq!(page.messages.len(), 1);
    assert_eq!(page.messages[0].parts.len(), 1);
    let part = &page.messages[0].parts[0];
    assert_eq!(part.kind, "tool_result");
    assert_eq!(part.content_text_utf8_bytes, total);
    assert_eq!(part.content_json_utf8_bytes, Some(total));
    assert_eq!(part.content_digest.as_deref(), Some("retained-digest"));
    assert_eq!((part.created_at_ms, part.updated_at_ms), (3, 7));
    assert!(serde_json::to_vec(&page).unwrap().len() < 4096);
    for field in [PartBodyField::ContentText, PartBodyField::ContentJson] {
        for expected in source(total, field == PartBodyField::ContentJson) {
            let (offset, original) = expected.unwrap();
            let range = store
                .read_export_range("c", "m", "p", field, 1, offset, 32768)
                .unwrap();
            assert_eq!(range.text, original);
            assert_eq!(range.total_utf8_bytes, total);
            assert_eq!(range.next_offset_bytes, offset + original.len() as u64);
            assert_eq!(range.end_of_content, range.next_offset_bytes == total);
        }
        let unicode = store
            .read_export_range("c", "m", "p", field, 1, 32760, 4)
            .unwrap();
        assert_eq!(unicode.text, "中");
        assert_eq!(unicode.next_offset_bytes, 32763);
        assert_eq!(
            store
                .read_export_range("c", "m", "p", field, 1, 32763, 4)
                .unwrap()
                .text,
            "😀"
        );
        assert!(store
            .read_export_range("c", "m", "p", field, 1, 32761, 4)
            .is_err());
        assert!(
            store
                .read_export_range("c", "m", "p", field, 1, total, 4)
                .unwrap()
                .end_of_content
        );
    }
    let window = store
        .load_conversation_message_window("c", None, 1, 4096)
        .unwrap()
        .unwrap();
    assert_eq!(window.conversation.messages[0].parts.len(), 1);
    assert_eq!(
        window.conversation.messages[0].parts[0].content_text.len(),
        4096
    );
    assert!(window.conversation.messages[0].parts[0]
        .content_json
        .is_none());
    assert_eq!(window.deferred_text_parts[0].total_utf8_bytes, total);
    let owner = nola_domain::CanonicalPartOwner::new("c".into(), "m".into(), "p".into()).unwrap();
    let semantic = store
        .read_historical_part(&owner, 1, 8192)
        .unwrap()
        .unwrap();
    assert!(
        semantic.source.is_some(),
        "window omission must not mean canonical JSON absence"
    );
    assert!(matches!(
        semantic.status,
        nola_domain::HistoricalProjectionStatus::Unsupported { .. }
    ));
    assert_eq!(
        sql.query_row(
            "SELECT COUNT(*) FROM message_part_body_references",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        2
    );
    assert_eq!(
        sql.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name='message_part_text_references'",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn absent_json_empty_json_and_literal_null_remain_distinct_from_referenced_json() {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, sql) = fixture(&directory.path().join("null.db"));
    for (id, ordinal, value) in [
        ("absent", 1, None),
        ("empty", 2, Some("")),
        ("literal", 3, Some("null")),
        ("referenced", 4, None),
    ] {
        part(&sql, id, ordinal, value);
    }
    let body = reference(
        "literal-object",
        "referenced",
        PartBodyField::ContentJson,
        4,
        1,
    );
    store
        .prepare_body(&body, &mut vec![Ok((0, "null".into()))].into_iter(), 100)
        .unwrap();
    bind(&sql, &body);
    let page = store.load_export_page("c", None, None).unwrap().unwrap();
    let parts = &page.messages[0].parts;
    assert_eq!(
        parts
            .iter()
            .map(|part| part.content_json_utf8_bytes)
            .collect::<Vec<_>>(),
        vec![None, Some(0), Some(4), Some(4)]
    );
    assert!(parts.iter().all(|part| part.content_text_utf8_bytes == 0));
    assert!(store
        .read_export_range("c", "m", "absent", PartBodyField::ContentJson, 1, 0, 4)
        .is_err());
    let empty = store
        .read_export_range("c", "m", "empty", PartBodyField::ContentJson, 1, 0, 4)
        .unwrap();
    assert_eq!(empty.total_utf8_bytes, 0);
    assert!(empty.end_of_content);
    assert!(empty.text.is_empty());
    for id in ["literal", "referenced"] {
        assert_eq!(
            store
                .read_export_range("c", "m", id, PartBodyField::ContentJson, 1, 0, 4)
                .unwrap()
                .text,
            "null"
        );
    }
}

#[test]
fn field_owner_visibility_finalization_and_conflicting_inline_source_are_checked_before_original_reads(
) {
    let directory = tempfile::tempdir().unwrap();
    let (mut store, sql) = fixture(&directory.path().join("owner.db"));
    part(&sql, "p", 1, None);
    let text = reference("text", "p", PartBodyField::ContentText, 4, 1);
    let json = reference("json", "p", PartBodyField::ContentJson, 4, 1);
    for body in [&text, &json] {
        store
            .prepare_body(body, &mut vec![Ok((0, "null".into()))].into_iter(), 100)
            .unwrap();
        bind(&sql, body);
    }
    for (conversation, message, part) in [
        ("other", "m", "p"),
        ("c", "foreign", "p"),
        ("c", "m", "foreign"),
    ] {
        assert!(store
            .read_export_range(
                conversation,
                message,
                part,
                PartBodyField::ContentJson,
                1,
                0,
                4
            )
            .is_err());
    }
    sql.execute("UPDATE message_part_body_references SET reference_json=?1 WHERE part_id='p' AND field='content_text'",params![serde_json::to_string(&json).unwrap()]).unwrap();
    assert!(store
        .read_export_range("c", "m", "p", PartBodyField::ContentText, 1, 0, 4)
        .is_err());
    assert!(store.load_export_page("c", None, None).is_err());
    sql.execute("UPDATE message_part_body_references SET reference_json=?1 WHERE part_id='p' AND field='content_text'",params![serde_json::to_string(&text).unwrap()]).unwrap();
    for (change, reset) in [
        (
            "UPDATE message_parts SET content_json='null' WHERE id='p'",
            "UPDATE message_parts SET content_json=NULL WHERE id='p'",
        ),
        (
            "UPDATE message_parts SET state='streaming' WHERE id='p'",
            "UPDATE message_parts SET state='completed' WHERE id='p'",
        ),
        (
            "UPDATE messages SET status='running' WHERE id='m'",
            "UPDATE messages SET status='completed' WHERE id='m'",
        ),
    ] {
        sql.execute_batch(change).unwrap();
        assert!(store
            .read_export_range("c", "m", "p", PartBodyField::ContentJson, 1, 0, 4)
            .is_err());
        sql.execute_batch(reset).unwrap();
    }
    sql.execute_batch("INSERT INTO conversation_projection_operations VALUES('hide','c','fixture',NULL,2,1,10);INSERT INTO message_projection_events VALUES('hidden','hide','c','m',0,'fixture',10)").unwrap();
    assert!(store
        .load_export_page("c", None, None)
        .unwrap()
        .unwrap()
        .messages
        .is_empty());
    assert!(store
        .read_export_range("c", "m", "p", PartBodyField::ContentJson, 1, 0, 4)
        .is_err());
    assert!(store
        .read_export_range("c", "m", "p", PartBodyField::ContentText, 1, 0, 4)
        .is_err());
}
