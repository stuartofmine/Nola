// SPDX-License-Identifier: Apache-2.0
use super::*;
use crate::NolaStore;

#[test]
fn tail_window_uses_reverse_index_without_offset_history_scan() {
    let directory = tempfile::tempdir().unwrap();
    let store = NolaStore::open(directory.path().join("tail-window.sqlite")).unwrap();
    store.connection.execute_batch("INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms) VALUES('tail','Tail',1,1,1);
        WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<40)
        INSERT INTO messages(id,conversation_id,role,status,ordinal,created_at_ms)
        SELECT 'm-'||x,'tail','assistant','completed',x,1 FROM n;").unwrap();
    let steps = || {
        let mut statement = store.connection.prepare(TAIL_MESSAGE_SQL).unwrap();
        let ids = statement.query_map(params!["tail",16], |row| row.get::<_, String>(0)).unwrap()
            .collect::<Result<Vec<_>,_>>().unwrap();
        assert_eq!(ids.len(), 16);
        statement.get_status(rusqlite::StatementStatus::VmStep)
    };
    let small = steps();
    store.connection.execute_batch("WITH RECURSIVE n(x) AS (SELECT 41 UNION ALL SELECT x+1 FROM n WHERE x<40000)
        INSERT INTO messages(id,conversation_id,role,status,ordinal,created_at_ms)
        SELECT 'm-'||x,'tail','assistant','completed',x,1 FROM n;").unwrap();
    let large = steps();
    assert!(large <= small + 32, "tail selection grew with history: {small} -> {large}");
    let (tail, _) = load_conversation_projection_with_window(&store.connection, "tail",
        ConversationMessageWindow::Tail { limit:16 }, Some((256,1024))).unwrap().unwrap();
    assert_eq!(tail.messages.first().unwrap().ordinal, 39985);
    assert_eq!(tail.messages.last().unwrap().ordinal, 40000);
}

fn fixture(path: &std::path::Path, sealed: bool) -> NolaStore {
    let store = NolaStore::open(path).unwrap();
    store.connection.execute_batch("INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms) VALUES('c','Reader',1,1,1);
        INSERT INTO messages(id,conversation_id,role,status,ordinal,created_at_ms,completed_at_ms) VALUES('m','c','user','completed',0,1,1);
        INSERT INTO message_parts(id,message_id,kind,ordinal,state,content_text,content_digest,created_at_ms,updated_at_ms)
        VALUES('p','m','text',0,'completed','你好🙂end','original-digest',1,1);").unwrap();
    if sealed {
        let reference = nola_domain::CanonicalBodyReference::for_field(
            "body".into(),
            "c".into(),
            "m".into(),
            "p".into(),
            nola_domain::PartBodyField::ContentText,
            13,
            2,
        )
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
            .execute_batch(
                "INSERT INTO canonical_text_chunks VALUES('body',0,'你好'),('body',6,'🙂end');
            UPDATE message_parts SET content_text='' WHERE id='p';",
            )
            .unwrap();
        store
            .connection
            .execute(
                "INSERT INTO message_part_body_references VALUES('p','content_text',?1)",
                [&encoded],
            )
            .unwrap();
    }
    store
}

fn read(store: &NolaStore, text_limit: usize, json_limit: usize) -> StoredPart {
    load_conversation_projection_with_window(
        &store.connection,
        "c",
        ConversationMessageWindow::All,
        Some((text_limit, json_limit)),
    )
    .unwrap()
    .unwrap()
    .0
    .messages
    .remove(0)
    .parts
    .remove(0)
}

fn install_json(store: &NolaStore, raw: &str, sealed: bool) {
    if !sealed {
        store
            .connection
            .execute(
                "UPDATE message_parts SET content_json=?1 WHERE id='p'",
                [raw],
            )
            .unwrap();
        return;
    }
    let reference = nola_domain::CanonicalBodyReference::for_field(
        "json-body".into(),
        "c".into(),
        "m".into(),
        "p".into(),
        nola_domain::PartBodyField::ContentJson,
        raw.len() as u64,
        1,
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
    store
        .connection
        .execute(
            "INSERT INTO canonical_text_chunks VALUES('json-body',0,?1)",
            [raw],
        )
        .unwrap();
    store
        .connection
        .execute(
            "INSERT INTO message_part_body_references VALUES('p','content_json',?1)",
            [&encoded],
        )
        .unwrap();
}

#[test]
fn inline_and_sealed_hydration_share_exact_unicode_tail_and_digest_rules() {
    let directory = tempfile::tempdir().unwrap();
    for sealed in [false, true] {
        let store = fixture(&directory.path().join(format!("{sealed}.db")), sealed);
        for (limit, expected, digest) in [
            (0, "你好🙂end", true),
            (6, "你好🙂end", true),
            (4, "…🙂end", false),
            (1, "…d", false),
        ] {
            let part = read(&store, limit, 0);
            assert_eq!(part.content_text, expected);
            assert_eq!(part.content_digest.is_some(), digest);
        }
    }
}

#[test]
fn raw_json_is_not_reencoded_and_existing_json_limit_projection_is_preserved() {
    let directory = tempfile::tempdir().unwrap();
    let store = fixture(&directory.path().join("db"), false);
    let raw = " { \"opaque\" : [1, true], \"text\": \"original\" } ";
    store
        .connection
        .execute("UPDATE message_parts SET content_json=?1", [raw])
        .unwrap();
    assert_eq!(read(&store, 0, 0).content_json.as_deref(), Some(raw));
    assert_eq!(read(&store, 1, 0).content_json.as_deref(), Some(raw));
    assert_eq!(read(&store, 1, 5).content_json, None);
    store
        .connection
        .execute(
            "UPDATE message_parts SET content_json=?1",
            [r#"{"review":{},"text":"你好🙂end"}"#],
        )
        .unwrap();
    let part = read(&store, 2, 8);
    assert_eq!(part.content_text, "…nd");
    assert_eq!(
        part.content_json, None,
        "review projection must obey its final byte budget"
    );
}

#[test]
fn complete_json_preserves_raw_integer_and_whitespace_with_exact_utf8_byte_budget() {
    let directory = tempfile::tempdir().unwrap();
    let raw =
        " \n{ \"n\" : 9223372036854775807, \"above_js\":9007199254740993, \"text\":\"你好🙂\" }\t";
    for sealed in [false, true] {
        let store = fixture(&directory.path().join(format!("exact-{sealed}")), false);
        install_json(&store, raw, sealed);
        for limit in [0, raw.len(), raw.len() + 1] {
            let part = read(&store, 0, limit);
            assert_eq!(part.content_json.as_deref(), Some(raw));
            assert!(!part.content_json_deferred);
            assert_eq!(part.content_digest.as_deref(), Some("original-digest"));
        }
        for limit in [raw.len() - 1, raw.chars().count()] {
            let part = read(&store, 0, limit);
            assert_eq!(part.content_json, None);
            assert!(part.content_json_deferred);
            assert!(part.content_digest.is_none());
        }
    }
}

#[test]
fn bounded_review_projection_preserves_other_raw_fields_and_clears_digest() {
    let directory = tempfile::tempdir().unwrap();
    let raw = format!(" {{ \"review\" : {{\"n\":9223372036854775807}}, \"text\":\"{}\", \"keep\":9007199254740993 }} ", "你好🙂".repeat(1000));
    for sealed in [false, true] {
        let store = fixture(&directory.path().join(format!("review-{sealed}")), false);
        install_json(&store, &raw, sealed);
        let part = read(&store, 2, 128);
        assert!(!part.content_json_deferred);
        let projected = part.content_json.expect("bounded review projection");
        assert!(projected.len() <= 128);
        assert!(projected.contains("\"n\":9223372036854775807"));
        assert!(projected.contains("\"keep\":9007199254740993"));
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&projected).unwrap()["text"],
            "…nd"
        );
        assert!(part.content_digest.is_none());
        let tail = store
            .load_conversation_tail_snapshot("c", 1, 2, 128)
            .unwrap()
            .unwrap();
        assert_eq!(tail.total_user_turns, 1);
        assert_eq!(
            tail.conversation.messages[0].parts[0]
                .content_json
                .as_deref(),
            Some(projected.as_str())
        );
        assert!(tail.conversation.messages[0].parts[0]
            .content_digest
            .is_none());
    }
}

#[test]
fn oversized_or_ambiguous_review_cannot_escape_final_budget() {
    let directory = tempfile::tempdir().unwrap();
    let huge = "x".repeat(4000);
    let cases = [
        format!(r#"{{"review":{{"large":"{huge}"}},"text":"{huge}"}}"#),
        format!(r#"{{"review":{{}},"other":"{huge}","text":"{huge}"}}"#),
        format!(r#"{{"review":null,"text":"{huge}"}}"#),
        format!(r#"{{"review":{{}},"review":{{}},"text":"{huge}"}}"#),
        format!(r#"{{"review":{{}},"text":"{huge}","text":"second"}}"#),
    ];
    for (index, raw) in cases.iter().enumerate() {
        for sealed in [false, true] {
            let store = fixture(
                &directory.path().join(format!("reject-{index}-{sealed}")),
                false,
            );
            install_json(&store, raw, sealed);
            if index >= 3 {
                assert!(
                    load_conversation_projection_with_window(
                        &store.connection,
                        "c",
                        ConversationMessageWindow::All,
                        Some((2, 128))
                    )
                    .is_err(),
                    "ambiguous case {index}, sealed={sealed}"
                );
                continue;
            }
            let part = read(&store, 2, 128);
            assert_eq!(part.content_json, None, "case {index}, sealed={sealed}");
            assert!(part.content_digest.is_none());
        }
    }
}

#[test]
fn missing_sealed_json_never_falls_back_to_inline_or_empty_value() {
    let directory = tempfile::tempdir().unwrap();
    let store = fixture(&directory.path().join("broken-json"), false);
    install_json(&store, r#"{"review":{},"text":"original"}"#, true);
    store
        .connection
        .execute(
            "DELETE FROM canonical_text_chunks WHERE content_id='json-body'",
            [],
        )
        .unwrap();
    for limit in [0, 8, 128] {
        assert!(load_conversation_projection_with_window(
            &store.connection,
            "c",
            ConversationMessageWindow::All,
            Some((2, limit))
        )
        .is_err());
    }
}

#[test]
fn sealed_conflict_still_fails_instead_of_reading_an_inline_fallback() {
    let directory = tempfile::tempdir().unwrap();
    let store = fixture(&directory.path().join("db"), true);
    store
        .connection
        .execute("UPDATE message_parts SET content_text='conflict'", [])
        .unwrap();
    assert!(load_conversation_projection(&store.connection, "c").is_err());
}
