// SPDX-License-Identifier: Apache-2.0
use super::*;

fn append_tool_history_probe_turn(store: &mut NolaStore, turn: usize) {
    store.begin_write_unit_of_work().unwrap();
    for slot in 0..34 {
        let ordinal = (turn * 34 + slot) as i64;
        let tool_result = slot > 0 && slot < 33 && slot % 2 == 0;
        let role = if slot == 0 || tool_result { "user" } else { "assistant" };
        let message = format!("message-{ordinal}");
        let part = format!("part-{ordinal}");
        store.connection.execute(
            "INSERT INTO messages(id,conversation_id,role,status,ordinal,created_at_ms) VALUES(?1,'scale',?2,'completed',?3,?3)",
            params![message, role, ordinal]).unwrap();
        let body = if tool_result {
            Some(serde_json::json!({"content":[{"type":"tool_result",
                "tool_use_id":format!("call-{turn}-{slot}"),
                "content":format!("Result {turn}/{slot}: {}", "x".repeat([1024,4096,16384][slot % 3]))}]}).to_string())
        } else { None };
        store.connection.execute(
            "INSERT INTO message_parts(id,message_id,kind,ordinal,state,content_text,content_json,created_at_ms,updated_at_ms)
             VALUES(?1,?2,?3,0,'completed',?4,?5,1,1)",
            params![part,message,if tool_result {"provider_message"} else {"text"},
                if slot == 0 {"继续研究"} else {"Research progress"},body]).unwrap();
    }
    store.commit_write_unit_of_work().unwrap();
}

/// Store read-path probe, not Application/Core or real-model acceptance.
/// One unit models a user turn with 16 tool call/result pairs and a final reply.
#[test]
#[ignore = "disk-backed multi-turn scale measurement"]
fn conversation_tail_thousandfold_tool_history_probe() {
    use std::time::Instant;
    fn percentile(values: &mut [u128], percent: usize) -> u128 {
        values.sort_unstable();
        values[((values.len() - 1) * percent).div_ceil(100)]
    }
    for turns in [1usize, 10, 1000] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("tool-history.sqlite");
        let mut store = NolaStore::open(&path).unwrap();
        store.connection.execute("INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms) VALUES('scale','Scale',1,1,1)", []).unwrap();
        let mut write_us = Vec::new();
        let ordinal = (turns * 34) as i64;
        for turn in 0..turns {
            let started = Instant::now();
            append_tool_history_probe_turn(&mut store, turn);
            write_us.push(started.elapsed().as_micros());
        }
        let mut reads = Vec::new();
        for _ in 0..3 {
            let started = Instant::now();
            let tail = store.load_conversation_tail_snapshot("scale", 16, 256, 1024).unwrap().unwrap();
            reads.push(started.elapsed().as_micros());
            assert_eq!(tail.total_message_count, (turns * 34) as i64);
            assert_eq!(tail.total_user_turns, turns as i64);
            assert_eq!(tail.conversation.messages.len(), 16);
            assert_eq!(tail.conversation.messages.last().unwrap().id, format!("message-{}", ordinal - 1));
        }
        drop(store);
        let started = Instant::now();
        let reopened = NolaStore::open(&path).unwrap();
        let tail = reopened.load_conversation_tail_snapshot("scale", 16, 256, 1024).unwrap().unwrap();
        assert_eq!(tail.total_user_turns, turns as i64);
        eprintln!("{}", serde_json::json!({"benchmark":"conversation_tail_tool_history",
            "turns":turns,"tool_calls":turns*16,"messages":ordinal,
            "write_turn_p50_us":percentile(&mut write_us,50),"write_turn_p95_us":percentile(&mut write_us,95),
            "tail_read_samples_us":reads,"reopen_and_tail_us":started.elapsed().as_micros(),
            "db_bytes":std::fs::metadata(&path).unwrap().len()}));
    }
}

fn concurrent_tool_history_probe(turns: usize) {
    use std::time::{Duration, Instant};
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("concurrent.sqlite");
    let wal = path.with_extension("sqlite-wal");
    let mut writer = NolaStore::open_archive_mode(&path, false).unwrap();
    writer.connection.execute("INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms) VALUES('scale','Scale',1,1,1)", []).unwrap();
    let reader = NolaStore::open_archive_mode(&path, false).unwrap();
    let mut read_us = Vec::new();
    let mut max_wal_bytes = 0;
    let mut previous_count = 0;
    let write_us = std::thread::scope(|scope| {
        let worker = scope.spawn(|| {
            let mut samples = Vec::new();
            for turn in 0..turns {
                let started = Instant::now();
                append_tool_history_probe_turn(&mut writer, turn);
                samples.push(started.elapsed().as_micros());
            }
            samples
        });
        while !worker.is_finished() {
            let started = Instant::now();
            let tail = reader.load_conversation_tail_snapshot("scale",16,256,1024).unwrap().unwrap();
            read_us.push(started.elapsed().as_micros());
            assert_eq!(tail.total_message_count % 34, 0, "reader observed a partial publication");
            assert_eq!(tail.total_user_turns * 34, tail.total_message_count, "count and classification must share one snapshot");
            assert!(tail.total_message_count >= previous_count);
            previous_count = tail.total_message_count;
            if previous_count > 0 {
                assert_eq!(tail.conversation.messages.len(), 16);
                assert_eq!(tail.conversation.messages.last().unwrap().id, format!("message-{}", previous_count - 1));
            }
            max_wal_bytes = max_wal_bytes.max(std::fs::metadata(&wal).map(|metadata| metadata.len()).unwrap_or(0));
            // Model an active UI consumer, not a permanently pinned snapshot.
            std::thread::sleep(Duration::from_millis(5));
        }
        worker.join().unwrap()
    });
    assert!(!read_us.is_empty());
    let tail = reader.load_conversation_tail_snapshot("scale",16,256,1024).unwrap().unwrap();
    assert_eq!(tail.total_message_count, (turns * 34) as i64);
    assert_eq!(tail.total_user_turns, turns as i64);
    let checkpoint: (i64,i64,i64) = writer.connection.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [],
        |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?))).unwrap();
    assert_eq!(checkpoint.0, 0, "completed read calls must release their WAL snapshots");
    assert_eq!(std::fs::metadata(&wal).unwrap().len(), 0);
    fn percentile(mut samples: Vec<u128>, percent: usize) -> u128 {
        samples.sort_unstable();
        samples[((samples.len() - 1) * percent).div_ceil(100)]
    }
    eprintln!("{}", serde_json::json!({"benchmark":"concurrent_tool_history", "turns":turns,
        "messages":turns*34,"tool_calls":turns*16,"reader_samples":read_us.len(),
        "read_p50_us":percentile(read_us.clone(),50),"read_p95_us":percentile(read_us.clone(),95),
        "read_p99_us":percentile(read_us,99),"write_p50_us":percentile(write_us.clone(),50),
        "write_p95_us":percentile(write_us.clone(),95),"write_p99_us":percentile(write_us,99),
        "max_wal_bytes":max_wal_bytes,"checkpoint_busy":checkpoint.0}));
}

#[test]
fn concurrent_tail_counts_and_bodies_share_committed_snapshot() {
    concurrent_tool_history_probe(20);
}

#[test]
#[ignore = "disk-backed concurrent thousandfold scale measurement"]
fn concurrent_tail_thousandfold_tool_history_probe() {
    concurrent_tool_history_probe(1000);
}

fn setup(path: &std::path::Path, raw: Option<&str>, sealed: bool) -> NolaStore {
    let store = NolaStore::open(path).unwrap();
    store.connection.execute_batch("INSERT INTO conversations(id,title,revision,created_at_ms,updated_at_ms) VALUES('c','Tail',1,1,1);
        INSERT INTO messages(id,conversation_id,role,status,ordinal,created_at_ms) VALUES('old','c','user','completed',0,1),('new','c','user','completed',1,1);
        INSERT INTO message_parts(id,message_id,kind,ordinal,state,content_text,created_at_ms,updated_at_ms) VALUES('p','old','provider_message',0,'completed','',1,1),('new-p','new','text',0,'completed','Current user',1,1);").unwrap();
    if sealed {
        let raw = raw.unwrap();
        let mut chunks = Vec::new();
        let mut start = 0;
        while start < raw.len() {
            let mut end = (start + 4096).min(raw.len());
            while !raw.is_char_boundary(end) {
                end -= 1;
            }
            chunks.push(&raw[start..end]);
            start = end;
        }
        let reference = nola_domain::CanonicalBodyReference::for_field(
            "body".into(),
            "c".into(),
            "old".into(),
            "p".into(),
            nola_domain::PartBodyField::ContentJson,
            raw.len() as u64,
            chunks.len() as u32,
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
        let mut offset = 0;
        for chunk in chunks {
            store
                .connection
                .execute(
                    "INSERT INTO canonical_text_chunks VALUES('body',?1,?2)",
                    params![offset, chunk],
                )
                .unwrap();
            offset += chunk.len();
        }
        store
            .connection
            .execute(
                "INSERT INTO message_part_body_references VALUES('p','content_json',?1)",
                [&encoded],
            )
            .unwrap();
    } else {
        store
            .connection
            .execute(
                "UPDATE message_parts SET content_json=?1 WHERE id='p'",
                params![raw],
            )
            .unwrap();
    }
    store
}

#[test]
fn published_provider_classification_survives_reopen_and_invalidates_on_body_changes() {
    let directory = tempfile::tempdir().unwrap();
    for sealed in [false, true] {
        let path = directory.path().join(format!("published-{sealed}.sqlite"));
        let mut store = setup(&path, Some(r#"{"content":[{"type":"tool_result","content":"old"}]}"#), sealed);
        store.begin_write_unit_of_work().unwrap();
        store.commit_write_unit_of_work().unwrap();
        assert_eq!(store.connection.query_row("SELECT has_tool_result FROM canonical_provider_classifications WHERE part_id='p'", [],
            |row| row.get::<_, i64>(0)).unwrap(), 1);
        drop(store);
        let mut store = NolaStore::open(&path).unwrap();
        assert_eq!(store.load_conversation_tail_snapshot("c",1,100,100).unwrap().unwrap().total_user_turns, 1);
        store.begin_write_unit_of_work().unwrap();
        if sealed {
            store.connection.execute("UPDATE canonical_text_chunks SET text='broken' WHERE content_id='body'", []).unwrap();
        } else {
            store.connection.execute("UPDATE message_parts SET content_json='broken' WHERE id='p'", []).unwrap();
        }
        assert!(store.load_conversation_tail_snapshot("c",1,100,100).is_err(), "dirty generation must not use old classification");
        store.rollback_write_unit_of_work().unwrap();
        assert_eq!(store.load_conversation_tail_snapshot("c",1,100,100).unwrap().unwrap().total_user_turns, 1);
        if !sealed {
            store.begin_write_unit_of_work().unwrap();
            store.connection.execute("UPDATE message_parts SET content_json='{\"content\":\"user text\"}' WHERE id='p'", []).unwrap();
            store.commit_write_unit_of_work().unwrap();
            assert_eq!(store.load_conversation_tail_snapshot("c",1,100,100).unwrap().unwrap().total_user_turns, 2);
        }
    }
}

#[test]
fn original_content_object_array_and_absence_preserve_turn_classification() {
    let dir = tempfile::tempdir().unwrap();
    for (index, (raw, expected)) in [
        (None, 2),
        (Some("null"), 2),
        (Some(r#"{"content":null}"#), 2),
        (Some(r#"{"content":"plain text"}"#), 2),
        (Some(r#"{"content":{}}"#), 2),
        (Some(r#"{"content":[{},null,0]}"#), 2),
        (Some(r#"{"content":{"type":"tool_result"}}"#), 1),
        (
            Some(r#"{"content":[{"type":"text"},{"type":"tool_result"}]}"#),
            1,
        ),
        (Some(r#"{"content":{"entry":{"type":"tool_result"}}}"#), 1),
    ]
    .into_iter()
    .enumerate()
    {
        for sealed in [false, true] {
            if sealed && raw.is_none() {
                continue;
            }
            let store = setup(&dir.path().join(format!("{index}-{sealed}")), raw, sealed);
            let tail = store
                .load_conversation_tail_snapshot("c", 1, 100, 100)
                .unwrap()
                .unwrap();
            assert_eq!(tail.total_message_count, 2);
            assert_eq!(tail.total_user_turns, expected);
            assert_eq!(tail.conversation.messages.len(), 1);
            assert_eq!(tail.conversation.messages[0].id, "new");
        }
    }
}

#[test]
fn array_item_after_64_and_large_ignored_string_are_not_truncated() {
    let dir = tempfile::tempdir().unwrap();
    let mut items = vec![r#"{"type":"text"}"#.to_owned(); 70];
    items.insert(
        0,
        format!(r#"{{"type":"text","text":"{}"}}"#, "x".repeat(1_000_000)),
    );
    items.push(r#"{"type":"tool_result","content":"result"}"#.into());
    let raw = format!(r#"{{"content":[{}]}}"#, items.join(","));
    for sealed in [false, true] {
        let store = setup(&dir.path().join(format!("{sealed}")), Some(&raw), sealed);
        assert_eq!(
            store
                .load_conversation_tail_snapshot("c", 1, 100, 100)
                .unwrap()
                .unwrap()
                .total_user_turns,
            1
        );
    }
}

#[test]
fn ambiguous_malformed_and_missing_reference_fail_closed() {
    let dir = tempfile::tempdir().unwrap();
    for (index, raw) in [
        r#"{"content":null,"content":[]}"#,
        r#"{"content":{"type":"text","type":"tool_result"}}"#,
        "{broken",
    ]
    .into_iter()
    .enumerate()
    {
        let store = setup(&dir.path().join(format!("bad-{index}")), Some(raw), false);
        assert!(store
            .load_conversation_tail_snapshot("c", 1, 100, 100)
            .is_err());
    }
    let store = setup(&dir.path().join("missing"), Some(r#"{"content":[]}"#), true);
    store
        .connection
        .execute(
            "DELETE FROM canonical_text_chunks WHERE content_id='body'",
            [],
        )
        .unwrap();
    assert!(store
        .load_conversation_tail_snapshot("c", 1, 100, 100)
        .is_err());
}

#[test]
fn unicode_short_tokens_and_immediate_object_values_keep_exact_semantics() {
    let dir = tempfile::tempdir().unwrap();
    for (index, (raw, expected)) in [
        (r#"{"content":["你","🙂",0,true,null,{}]}"#.to_string(), 2),
        (
            r#"{"content":{"你":"🙂","type":"tool_result"}}"#.to_string(),
            1,
        ),
        (
            r#"{"content":{"entry":{"type":"tool_result"},"尾":"好"}}"#.to_string(),
            1,
        ),
        (
            r#"{"content":{"entry":{"nested":{"type":"tool_result"}}}}"#.to_string(),
            2,
        ),
        (
            r#"{"content":[{"type":"text","text":"tool_result"},{"typ\u0065":"tool_result"}]}"#
                .to_string(),
            1,
        ),
        (
            format!(
                r#"{{"content":["{}",{{"type":"tool_result"}}]}}"#,
                "你🙂".repeat(700)
            ),
            1,
        ),
    ]
    .into_iter()
    .enumerate()
    {
        for sealed in [false, true] {
            let store = setup(
                &dir.path().join(format!("unicode-{index}-{sealed}")),
                Some(&raw),
                sealed,
            );
            let tail = store
                .load_conversation_tail_snapshot("c", 1, 4, 4)
                .unwrap()
                .unwrap();
            assert_eq!(tail.total_user_turns, expected, "{index}, sealed={sealed}");
        }
    }
}

#[test]
fn hidden_corrupt_body_is_not_read_but_visible_corruption_never_uses_inline_fallback() {
    let dir = tempfile::tempdir().unwrap();
    let store = setup(
        &dir.path().join("visibility"),
        Some(r#"{"content":[]}"#),
        true,
    );
    store
        .connection
        .execute_batch(
            "DELETE FROM canonical_text_chunks WHERE content_id='body';
        UPDATE message_parts SET content_json='{}' WHERE id='p';
        INSERT INTO conversation_projection_operations VALUES('hide','c','delete',NULL,2,2,3);
        INSERT INTO message_projection_events VALUES('hidden','hide','c','old',0,'deleted',3);",
        )
        .unwrap();
    let tail = store
        .load_conversation_tail_snapshot("c", 1, 100, 100)
        .unwrap()
        .unwrap();
    assert_eq!(tail.total_message_count, 1);
    assert_eq!(tail.total_user_turns, 1);
    assert_eq!(
        store
            .conversation_summary("c")
            .unwrap()
            .unwrap()
            .first_user_message
            .as_deref(),
        Some("Current user")
    );
    store.connection.execute_batch("INSERT INTO conversation_projection_operations VALUES('show','c','restore',NULL,3,3,4);
        INSERT INTO message_projection_events VALUES('shown','show','c','old',1,'completed',4);").unwrap();
    assert!(store
        .load_conversation_tail_snapshot("c", 1, 100, 100)
        .is_err());
}

#[test]
fn successful_tool_classification_does_not_hide_later_ambiguous_part() {
    let dir = tempfile::tempdir().unwrap();
    for sealed in [false, true] {
        let store = setup(
            &dir.path().join(format!("later-{sealed}")),
            Some(r#"{"content":{"type":"tool_result"}}"#),
            sealed,
        );
        store.connection.execute("INSERT INTO message_parts(id,message_id,kind,ordinal,state,content_json,created_at_ms,updated_at_ms) VALUES('bad','old','provider_message',1,'completed',?1,1,1)",
            [r#"{"content":[],"content":null}"#]).unwrap();
        assert!(store
            .load_conversation_tail_snapshot("c", 1, 100, 100)
            .is_err());
    }
}
