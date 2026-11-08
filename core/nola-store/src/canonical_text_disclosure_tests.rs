// SPDX-License-Identifier: Apache-2.0
//! Product disclosure contracts on real canonical append/preparation paths.
use super::*;
use crate::semantic_json_projection::SCANNED_JSON_BYTES;
use crate::{
    AppendMessagesInput, CreateConversationInput, MessageInput, MessagePartInput, NolaStore,
};
use nola_domain::{
    CanonicalBodyReference, CanonicalPartOwner, HistoricalProjectionStatus, SemanticValue,
    StructuredValue,
};
use nola_store_port::canonical_body::CanonicalBodyRepository;

const ORIGINAL: &str = "未审核候选中🙂\noriginal answer";

#[test]
fn oversized_reference_cannot_be_confused_with_absent_json_or_inline_approval() {
    let directory = tempfile::tempdir().unwrap();
    let store = fixture(
        &directory.path().join("oversized.db"),
        ORIGINAL,
        None,
        false,
    );
    let oversized = format!("{{\"padding\":\"{}\"}}", "x".repeat(4096));
    // Normal writes reject this at the schema boundary. Separately inject a
    // corrupt historical row to prove readers fail closed as well; this is
    // intentionally not a successful product publication transaction.
    assert!(store.connection.execute(
        "INSERT INTO message_part_body_references(part_id,field,reference_json) VALUES('p','content_json',?1)",
        [&oversized],
    ).is_err());
    store
        .connection
        .pragma_update(None, "ignore_check_constraints", true)
        .unwrap();
    store.connection.execute(
        "INSERT INTO message_part_body_references(part_id,field,reference_json) VALUES('p','content_json',?1)",
        [&oversized],
    ).unwrap();
    store
        .connection
        .pragma_update(None, "ignore_check_constraints", false)
        .unwrap();
    assert!(
        CanonicalFieldReader::open_disclosure_source(&store.connection, "c", "m", "p").is_err()
    );
    assert!(allows_product_text(&store.connection, "c", "m", "p").is_err());
    assert!(store
        .read_conversation_text_range("c", "m", "p", 0, 32768)
        .is_err());
}

#[test]
fn large_publication_scans_review_once_not_once_per_utf8_page_after_reopen() {
    for sealed in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("large.db");
        let text = "中🙂abc".repeat(1_000_000);
        let json = raw(Some("\"approved\""), &text, "");
        SCANNED_JSON_BYTES.with(|bytes| bytes.set(0));
        let store = fixture(&path, &text, Some(&json), sealed);
        assert_eq!(
            SCANNED_JSON_BYTES.with(|bytes| bytes.get()),
            2 * json.len() as u64
        );
        drop(store);
        let store = NolaStore::open(&path).unwrap();
        SCANNED_JSON_BYTES.with(|bytes| bytes.set(0));
        let mut restored = String::new();
        let mut offset = 0;
        let mut pages = 0;
        loop {
            let page = store
                .read_conversation_text_range("c", "m", "p", offset, 32768)
                .unwrap();
            assert!(page.next_offset_bytes > offset);
            restored.push_str(&page.text);
            offset = page.next_offset_bytes;
            pages += 1;
            if page.end_of_content {
                break;
            }
        }
        assert!(pages >= 306);
        assert_eq!(restored, text);
        assert_eq!(SCANNED_JSON_BYTES.with(|bytes| bytes.get()), 0);
    }
}

#[test]
fn disclosure_sql_never_extracts_inline_json_column() {
    let directory = tempfile::tempdir().unwrap();
    let json = raw(Some("\"approved\""), ORIGINAL, &"x".repeat(10_000_000));
    let store = fixture(
        &directory.path().join("inline-plan.db"),
        ORIGINAL,
        Some(&json),
        false,
    );
    let root: i64 = store
        .connection
        .query_row(
            "SELECT rootpage FROM sqlite_master WHERE type='table' AND name='message_parts'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let json_column: i64 = store
        .connection
        .query_row(
            "SELECT cid FROM pragma_table_info('message_parts') WHERE name='content_json'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let opcodes: Vec<(String, i64, i64)> = store
        .connection
        .prepare(&format!("EXPLAIN {DISCLOSURE_QUERY}"))
        .unwrap()
        .query_map(params!["c", "m", "p"], |row| {
            Ok((row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let part_cursors: Vec<i64> = opcodes
        .iter()
        .filter_map(|(opcode, p1, p2)| (opcode == "OpenRead" && *p2 == root).then_some(*p1))
        .collect();
    assert!(
        !part_cursors.is_empty(),
        "fixture must identify actual Part table cursor"
    );
    assert!(
        !opcodes.iter().any(|(opcode, p1, p2)| opcode == "Column"
            && part_cursors.contains(p1)
            && *p2 == json_column),
        "disclosure must not load the inline JSON column, even to compute length"
    );
    assert!(allows_product_text(&store.connection, "c", "m", "p").unwrap());
    // This checks SQLite's actual column-read plan, not disk/cache page IO:
    // metadata and payload may still share physical SQLite pages.
}

#[test]
fn changed_review_is_fail_closed_until_publication_and_nested_rollback_restores_fact() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("review.db");
    let approved = raw(Some("\"approved\""), ORIGINAL, "");
    let rejected = raw(Some("\"rejected\""), ORIGINAL, "");
    let mut store = fixture(&path, ORIGINAL, Some(&approved), false);
    store.begin_write_unit_of_work().unwrap();
    {
        let tx = store.write_transaction().unwrap();
        tx.execute(
            "UPDATE message_parts SET content_json=?1 WHERE id='p'",
            [&rejected],
        )
        .unwrap();
        assert!(allows_product_text(&tx, "c", "m", "p").is_err());
        tx.commit().unwrap();
    }
    assert!(!allows_product_text(&store.connection, "c", "m", "p").unwrap());
    store.rollback_write_unit_of_work().unwrap();
    assert!(allows_product_text(&store.connection, "c", "m", "p").unwrap());
    {
        let tx = store.write_transaction().unwrap();
        tx.execute(
            "UPDATE message_parts SET content_json=?1 WHERE id='p'",
            [&rejected],
        )
        .unwrap();
        tx.commit().unwrap();
    }
    drop(store);
    let store = NolaStore::open(&path).unwrap();
    assert!(!allows_product_text(&store.connection, "c", "m", "p").unwrap());
    assert!(store
        .read_conversation_text_range("c", "m", "p", 0, 32768)
        .is_err());
}

#[test]
fn text_only_append_does_not_reparse_review_and_backing_mutation_invalidates() {
    let directory = tempfile::tempdir().unwrap();
    let json = raw(Some("\"approved\""), ORIGINAL, "");
    let mut store = fixture(
        &directory.path().join("backing.db"),
        ORIGINAL,
        Some(&json),
        true,
    );
    SCANNED_JSON_BYTES.with(|bytes| bytes.set(0));
    {
        let tx = store.write_transaction().unwrap();
        tx.execute("UPDATE message_parts SET updated_at_ms=3 WHERE id='p'", [])
            .unwrap();
        tx.commit().unwrap();
    }
    assert_eq!(SCANNED_JSON_BYTES.with(|bytes| bytes.get()), 0);
    let tx = store.write_transaction().unwrap();
    // Even a same-byte object update invalidates the derived generation: no
    // unverified mutable physical backing may retain an approval decision.
    tx.execute(
        "UPDATE canonical_text_chunks SET text=text WHERE content_id='json'",
        [],
    )
    .unwrap();
    assert!(allows_product_text(&tx, "c", "m", "p").is_err());
    drop(tx);
    assert!(allows_product_text(&store.connection, "c", "m", "p").unwrap());
}

fn raw(review: Option<&str>, text: &str, padding: &str) -> String {
    let review = review
        .map(|value| format!(",\"review\":{value}"))
        .unwrap_or_default();
    format!(
        "{{\"kind\":\"text\",\"id\":\"p\",\"message_id\":\"m\",\"turn_id\":\"r\",\"text\":{},\"is_complete\":true,\"padding\":{}{review}}}",
        serde_json::to_string(text).unwrap(),
        serde_json::to_string(padding).unwrap(),
    )
}

fn fixture(path: &std::path::Path, text: &str, json: Option<&str>, sealed: bool) -> NolaStore {
    let mut store = NolaStore::open(path).unwrap();
    store
        .create_conversation(&CreateConversationInput {
            id: "c".into(),
            title: "disclosure".into(),
            revision: 1,
            settings_json: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        })
        .unwrap();
    let mut references = Vec::new();
    if sealed {
        // The content quota covers both original fields already prepared in
        // this Store, not only the object currently being introduced.
        let quota = text.len() as u64 + json.map_or(0, |value| value.len() as u64) + 1024;
        for (id, field, original) in [
            ("text", PartBodyField::ContentText, Some(text)),
            ("json", PartBodyField::ContentJson, json),
        ] {
            let Some(original) = original else { continue };
            let chunks =
                crate::canonical_field_reader::borrowed_json_chunks(original).collect::<Vec<_>>();
            let reference = CanonicalBodyReference::for_field(
                id.into(),
                "c".into(),
                "m".into(),
                "p".into(),
                field,
                original.len() as u64,
                chunks.len() as u32,
            )
            .unwrap();
            store
                .prepare_body(&reference, &mut chunks.into_iter(), quota)
                .unwrap();
            references.push(reference);
        }
    }
    let input = AppendMessagesInput {
        conversation_id: "c".into(),
        expected_message_count: 0,
        updated_at_ms: 2,
        messages: vec![MessageInput {
            id: "m".into(),
            run_id: Some("r".into()),
            role: "assistant".into(),
            status: "completed".into(),
            ordinal: 1,
            created_at_ms: 1,
            completed_at_ms: Some(2),
            parts: vec![MessagePartInput {
                id: "p".into(),
                kind: "text".into(),
                ordinal: 1,
                state: "completed".into(),
                content_text: if sealed { String::new() } else { text.into() },
                content_json: if sealed {
                    None
                } else {
                    json.map(str::to_owned)
                },
                content_digest: None,
                created_at_ms: 1,
                updated_at_ms: 2,
            }],
        }],
    };
    if sealed {
        let tx = store.write_transaction().unwrap();
        crate::conversation_append_store::append_in_transaction(&tx, &input, None, &references)
            .unwrap();
        tx.commit().unwrap();
    } else {
        store.append_messages(&input).unwrap();
    }
    // A message's run_id is not admission evidence by itself. Historical
    // journal reads require the exact local Run as well as the Part owner;
    // deliberately keep this fixture checkpoint-free to exercise cold history.
    store
        .connection
        .execute(
            "INSERT INTO runs(id,conversation_id,kind,assistant_message_id,status,next_event_sequence,created_at_ms,updated_at_ms,completed_at_ms)
             VALUES('r','c','conversation','m','running',1,1,2,NULL)",
            [],
        )
        .unwrap();
    assert_eq!(
        store
            .connection
            .query_row("SELECT count(*) FROM core_checkpoints", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0,
    );
    store
}

fn historical(store: &NolaStore) -> nola_domain::HistoricalPartProjection {
    store
        .read_historical_part(
            &CanonicalPartOwner::new("c".into(), "m".into(), "p".into()).unwrap(),
            2,
            8192,
        )
        .unwrap()
        .unwrap()
}

fn assert_disclosure(store: &NolaStore, expected: bool) {
    let window = store
        .load_conversation_message_window("c", None, 1, 4)
        .unwrap()
        .unwrap();
    assert_eq!(window.deferred_text_parts.len(), 1);
    assert!(window.deferred_text_parts[0].is_final);
    assert_eq!(window.deferred_text_parts[0].can_read_original, expected);
    assert_eq!(
        allows_product_text(&store.connection, "c", "m", "p").unwrap(),
        expected
    );
    let range = store.read_conversation_text_range("c", "m", "p", 0, 32768);
    if expected {
        assert_eq!(range.unwrap().text, ORIGINAL);
    } else {
        assert!(range.is_err());
    }
    let matches = store
        .search_transcript_text("c", "original answer", 20)
        .unwrap();
    if expected {
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].message_id, "m");
        assert_eq!(matches[0].part_id, "p");
        assert_eq!(matches[0].start_character, 0);
        assert_eq!(matches[0].snippet, ORIGINAL);
    } else {
        assert!(matches.is_empty());
    }
    let projection = historical(store);
    assert_eq!(projection.canonical_kind, "text");
    if expected {
        assert_eq!(projection.status, HistoricalProjectionStatus::Supported);
        assert_eq!(projection.canonical_text.unwrap().text, ORIGINAL);
        assert_eq!(
            projection.fields["text"],
            SemanticValue::Complete {
                value: StructuredValue::String(ORIGINAL.into())
            }
        );
    } else {
        assert!(projection.canonical_text.is_none());
        assert!(!projection.fields.contains_key("text"));
        assert!(!projection.fields.contains_key("markdown"));
        let encoded = serde_json::to_string(&projection).unwrap();
        assert!(!encoded.contains("original answer"));
    }
    // A display decision never destroys or rewrites the retained raw fields.
    let field =
        CanonicalFieldReader::open(&store.connection, "c", "m", "p", PartBodyField::ContentText)
            .unwrap();
    assert_eq!(field.read_text(0).unwrap(), ORIGINAL);
}

#[test]
fn final_reviewed_text_windows_ranges_and_cold_history_share_disclosure_for_inline_and_sealed() {
    for sealed in [false, true] {
        for (review, public) in [
            (None, true),
            (Some("null"), true),
            (Some("\"approved\""), true),
            (Some("\"pending\""), false),
            (Some("\"rejected\""), false),
            (Some("\"interrupted\""), false),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("disclosure.db");
            let json = raw(review, ORIGINAL, "");
            let store = fixture(&path, ORIGINAL, Some(&json), sealed);
            assert_disclosure(&store, public);
            let projection = historical(&store);
            if review.is_some() {
                assert!(projection.fields.contains_key("review"));
            }
            assert!(projection.fields.contains_key("is_complete"));
            drop(store);
            let store = NolaStore::open(&path).unwrap();
            assert_disclosure(&store, public);
        }
    }
}

#[test]
fn duplicate_unknown_wrong_type_and_oversized_review_never_become_unreviewed_text() {
    let oversized = serde_json::to_string(&"approved".repeat(1000)).unwrap();
    for review in [
        "\"unknown\"",
        "true",
        "7",
        "{}",
        "[]",
        &oversized,
        "\"approved\",\"review\":\"rejected\"",
        "\"approved\",\"re\\u0076iew\":\"approved\"",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let json = raw(Some(review), ORIGINAL, "");
        let store = fixture(
            &dir.path().join("disclosure.db"),
            ORIGINAL,
            Some(&json),
            false,
        );
        assert_disclosure(&store, false);
    }
    for json in ["null", "[]", "\"unknown envelope\""] {
        let dir = tempfile::tempdir().unwrap();
        let store = fixture(
            &dir.path().join("disclosure.db"),
            ORIGINAL,
            Some(json),
            false,
        );
        assert_disclosure(&store, false);
    }
}

#[test]
fn late_review_after_large_unicode_json_uses_bounded_scalar_budget_and_preserves_raw_storage() {
    let padding = "中🙂\"\\\n".repeat(30_000);
    for sealed in [false, true] {
        for (review, public) in [("\"rejected\"", false), ("\"\\u0061pproved\"", true)] {
            let dir = tempfile::tempdir().unwrap();
            let json = raw(Some(review), ORIGINAL, &padding);
            assert!(json.len() > 256 * 1024);
            let store = fixture(
                &dir.path().join("disclosure.db"),
                ORIGINAL,
                Some(&json),
                sealed,
            );
            assert_disclosure(&store, public);
            let field = CanonicalFieldReader::open(
                &store.connection,
                "c",
                "m",
                "p",
                PartBodyField::ContentJson,
            )
            .unwrap();
            let extracted = field
                .extract_json_scalar_fields(&["review"], 512)
                .unwrap()
                .unwrap();
            assert_eq!(extracted.fields.len(), 1);
            assert!(!extracted.fields[0].duplicate_known_key);
            assert!(matches!(&extracted.fields[0].value,
                ExtractedValue::Complete(ScalarValue::String(value)) if value.len() <= 11));
            assert_eq!(field.total_utf8_bytes(), Some(json.len() as u64));
            assert_eq!(
                field.read_json(None).unwrap(),
                crate::canonical_field_reader::CanonicalJsonRead::Complete(json)
            );
        }
    }
}

#[test]
fn plain_original_without_json_keeps_public_range_and_correct_owner_admission() {
    let dir = tempfile::tempdir().unwrap();
    let store = fixture(&dir.path().join("disclosure.db"), ORIGINAL, None, false);
    assert!(allows_product_text(&store.connection, "c", "m", "p").unwrap());
    assert_eq!(
        store
            .read_conversation_text_range("c", "m", "p", 0, 32768)
            .unwrap()
            .text,
        ORIGINAL
    );
    // A Part without original JSON has no structured historical envelope.
    // Its ordinary text remains available through the canonical window/range.
    assert!(store
        .read_historical_part(
            &CanonicalPartOwner::new("c".into(), "m".into(), "p".into()).unwrap(),
            2,
            8192,
        )
        .unwrap()
        .is_none());
    let window = store
        .load_conversation_message_window("c", None, 1, 4)
        .unwrap()
        .unwrap();
    assert!(ORIGINAL.starts_with(&window.conversation.messages[0].parts[0].content_text));
    assert!(window.deferred_text_parts[0].can_read_original);
    assert_eq!(
        store
            .search_transcript_text("c", "original answer", 20)
            .unwrap()[0]
            .snippet,
        ORIGINAL
    );
    assert!(allows_product_text(&store.connection, "other", "m", "p").is_err());
}

#[test]
fn historical_disclosure_withholds_semantics_without_matching_run_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let json = raw(None, ORIGINAL, "");
    let store = fixture(
        &dir.path().join("disclosure.db"),
        ORIGINAL,
        Some(&json),
        false,
    );
    assert_disclosure(&store, true);
    store
        .connection
        .execute(
            "UPDATE messages SET run_id='unadmitted-run' WHERE id='m'",
            [],
        )
        .unwrap();
    let historical = store
        .read_historical_part(
            &CanonicalPartOwner::new("c".into(), "m".into(), "p".into()).unwrap(),
            2,
            8192,
        )
        .unwrap()
        .unwrap();
    // Historical canonical prose is readable without granting the embedded
    // AgentPart any execution identity. This is not a journal admission read:
    // the original text is independently public, while the mismatched turn's
    // semantic fields must remain unavailable (not promoted via a fallback).
    assert_eq!(
        historical.status,
        HistoricalProjectionStatus::Unsupported {
            code: "semantic_identity_unverified".into(),
        }
    );
    assert_eq!(
        historical.binding,
        Some(nola_domain::SemanticIdentityAssessment::DerivationRequired)
    );
    assert!(historical.fields.is_empty());
    assert_eq!(
        historical.owner,
        CanonicalPartOwner::new("c".into(), "m".into(), "p".into()).unwrap()
    );
    assert_eq!(historical.canonical_text.unwrap().text, ORIGINAL);
}
