use nola_store::{
    AppendMessagesInput, CreateConversationInput, MessageInput, MessagePartInput, NolaStore,
};
use tempfile::tempdir;

#[test]
fn append_rolls_back_parts_revision_and_publication_when_sync_enqueue_fails() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("publication-failure.db");
    let mut store = NolaStore::open(&path).unwrap();
    store
        .create_conversation(&CreateConversationInput {
            id: "c".into(),
            title: "Atomic publication".into(),
            revision: 1,
            settings_json: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        })
        .unwrap();
    store
        .append_messages(&AppendMessagesInput {
            conversation_id: "c".into(),
            expected_message_count: 0,
            messages: vec![message("original", 1, "original body")],
            updated_at_ms: 2,
        })
        .unwrap();
    let fixture = rusqlite::Connection::open(&path).unwrap();
    let before: (i64,i64,i64) = fixture.query_row(
        "SELECT (SELECT COUNT(*) FROM sync_outbox),revision,updated_at_ms FROM conversations WHERE id='c'",
        [], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
    ).unwrap();
    // The trigger fails after append has inserted all Parts and advanced the
    // conversation revision, but before the publication can be committed.
    fixture
        .execute_batch(
            "CREATE TRIGGER fixture_reject_append_publication
        BEFORE INSERT ON sync_outbox WHEN NEW.entity_type='conversation_append'
        BEGIN SELECT RAISE(ABORT,'fixture sync publication unavailable'); END;",
        )
        .unwrap();
    let input = AppendMessagesInput {
        conversation_id: "c".into(),
        expected_message_count: 1,
        messages: vec![
            message("second", 2, "second body"),
            message("third", 3, "third body"),
        ],
        updated_at_ms: 4,
    };
    let error = store.append_messages(&input).unwrap_err();
    assert!(error
        .to_string()
        .contains("fixture sync publication unavailable"));
    let after: (i64,i64,i64) = fixture.query_row(
        "SELECT (SELECT COUNT(*) FROM sync_outbox),revision,updated_at_ms FROM conversations WHERE id='c'",
        [], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
    ).unwrap();
    assert_eq!(before, after);
    let counts: (i64, i64) = fixture
        .query_row(
            "SELECT (SELECT COUNT(*) FROM messages),(SELECT COUNT(*) FROM message_parts)",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(counts, (1, 1));
    fixture
        .execute_batch("DROP TRIGGER fixture_reject_append_publication")
        .unwrap();
    drop(fixture);
    drop(store);
    let mut store = NolaStore::open(&path).unwrap();
    let window = store
        .load_conversation_message_window("c", None, 10, 1024)
        .unwrap()
        .unwrap();
    assert_eq!(window.total_message_count, 1);
    assert_eq!(
        window.conversation.messages[0].parts[0].content_text,
        "original body"
    );
    // The failed append consumed neither the message identities nor the count
    // precondition. An identical retry after the external failure can commit.
    let result = store.append_messages(&input).unwrap();
    assert_eq!(result.appended_message_count, 2);
    assert_eq!(result.message_count, 3);
    assert_eq!(result.conversation_revision, 3);
}

#[test]
fn import_commit_rechecks_runs_started_after_admission() {
    use nola_store_port::transcript_write::TranscriptWriteRepository;
    for status in [
        "running",
        "waiting_permission",
        "suspended",
        "queued",
        "completed",
        "failed",
        "cancelled",
        "sealed",
    ] {
        let directory = tempdir().unwrap();
        let path = directory.path().join("admission.db");
        let mut store = NolaStore::open(&path).unwrap();
        store
            .create_conversation(&CreateConversationInput {
                id: "c".into(),
                title: "Admission".into(),
                revision: 1,
                settings_json: None,
                created_at_ms: 1,
                updated_at_ms: 1,
            })
            .unwrap();
        store
            .append_messages(&AppendMessagesInput {
                conversation_id: "c".into(),
                expected_message_count: 0,
                messages: vec![message("original", 1, "preserved")],
                updated_at_ms: 2,
            })
            .unwrap();
        assert!(!store
            .conversation_write_repository()
            .has_active_run("c")
            .unwrap());
        // Fixture-only second writer models a Run committed during preparation.
        // A maintenance Run intentionally has no assistant Part to hydrate.
        let other = rusqlite::Connection::open(&path).unwrap();
        other.execute("INSERT INTO runs(id,conversation_id,kind,status,next_event_sequence,created_at_ms,updated_at_ms)
            VALUES('run','c','maintenance',?1,1,3,3)", [status]).unwrap();
        // Old-looking labels are not formal result seals and remain active.
        let terminal = status == "sealed";
        assert_eq!(
            store
                .conversation_write_repository()
                .has_active_run("c")
                .unwrap(),
            !terminal
        );
        let outbox_before: i64 = other
            .query_row("SELECT COUNT(*) FROM sync_outbox", [], |row| row.get(0))
            .unwrap();
        let result = store.append_messages(&AppendMessagesInput {
            conversation_id: "c".into(),
            expected_message_count: 1,
            messages: vec![message("imported", 2, "new body")],
            updated_at_ms: 4,
        });
        assert_eq!(result.is_ok(), terminal, "{status}: {result:?}");
        let outbox_after: i64 = other
            .query_row("SELECT COUNT(*) FROM sync_outbox", [], |row| row.get(0))
            .unwrap();
        assert_eq!(outbox_after, outbox_before + if terminal { 1 } else { 0 });
        drop(other);
        drop(store);
        let reopened = NolaStore::open(&path).unwrap();
        let window = reopened
            .load_conversation_message_window("c", None, 10, 1024)
            .unwrap()
            .unwrap();
        assert_eq!(window.total_message_count, if terminal { 2 } else { 1 });
        assert_eq!(window.conversation.revision, if terminal { 3 } else { 2 });
        assert_eq!(
            window.conversation.messages[0].parts[0].content_text,
            "preserved"
        );
    }
}

#[test]
fn import_rejects_run_identity_claimed_by_another_conversation_before_commit() {
    use nola_store_port::transcript_write::TranscriptWriteRepository;
    let directory = tempdir().unwrap();
    let path = directory.path().join("ownership.db");
    let mut store = NolaStore::open(&path).unwrap();
    for id in ["target", "foreign"] {
        store
            .create_conversation(&CreateConversationInput {
                id: id.into(),
                title: id.into(),
                revision: 1,
                settings_json: None,
                created_at_ms: 1,
                updated_at_ms: 1,
            })
            .unwrap();
    }
    assert!(store
        .conversation_write_repository()
        .conversation_id_for_run("claimed-run")
        .unwrap()
        .is_none());
    let other = rusqlite::Connection::open(&path).unwrap();
    other.execute("INSERT INTO runs(id,conversation_id,kind,status,next_event_sequence,created_at_ms,updated_at_ms)
        VALUES('claimed-run','foreign','maintenance','completed',1,2,2)", []).unwrap();
    let outbox_before: i64 = other
        .query_row("SELECT COUNT(*) FROM sync_outbox", [], |row| row.get(0))
        .unwrap();
    let mut imported = message("imported", 1, "foreign ownership must not become visible");
    imported.run_id = Some("claimed-run".into());
    let error = store
        .append_messages(&AppendMessagesInput {
            conversation_id: "target".into(),
            expected_message_count: 0,
            messages: vec![imported],
            updated_at_ms: 3,
        })
        .unwrap_err();
    assert!(error.to_string().contains("another conversation"));
    let outbox_after: i64 = other
        .query_row("SELECT COUNT(*) FROM sync_outbox", [], |row| row.get(0))
        .unwrap();
    assert_eq!(outbox_before, outbox_after);
    drop(other);
    drop(store);
    let store = NolaStore::open(&path).unwrap();
    let window = store
        .load_conversation_message_window("target", None, 10, 1024)
        .unwrap()
        .unwrap();
    assert_eq!(window.total_message_count, 0);
    assert_eq!(window.conversation.revision, 1);
    assert_eq!(
        store
            .conversation_id_for_run("claimed-run")
            .unwrap()
            .as_deref(),
        Some("foreign")
    );
}

fn message(id: &str, ordinal: i64, text: &str) -> MessageInput {
    MessageInput {
        id: id.into(),
        run_id: None,
        role: if ordinal == 1 { "user" } else { "assistant" }.into(),
        status: "completed".into(),
        ordinal,
        created_at_ms: 1_700_000_000_000 + ordinal,
        completed_at_ms: Some(1_700_000_000_000 + ordinal),
        parts: vec![MessagePartInput {
            id: format!("part-{id}"),
            kind: "text".into(),
            ordinal: 1,
            state: "completed".into(),
            content_text: text.into(),
            content_json: Some(format!(
                "{{\"text\":{}}}",
                serde_json::to_string(text).unwrap()
            )),
            content_digest: None,
            created_at_ms: 1_700_000_000_000 + ordinal,
            updated_at_ms: 1_700_000_000_000 + ordinal,
        }],
    }
}

#[test]
fn history_byte_window_preserves_large_messages_and_advances_without_gaps() {
    let directory = tempdir().unwrap();
    let mut store = NolaStore::open(directory.path().join("nola.db")).unwrap();
    store
        .create_conversation(&CreateConversationInput {
            id: "window".into(),
            title: "Window".into(),
            revision: 1,
            settings_json: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        })
        .unwrap();
    let large = "x".repeat(10_000_000);
    store
        .append_messages(&AppendMessagesInput {
            conversation_id: "window".into(),
            expected_message_count: 0,
            messages: vec![
                message("large", 1, &large),
                message("middle", 2, &"m".repeat(100)),
                message("latest", 3, &"n".repeat(100)),
            ],
            updated_at_ms: 4,
        })
        .unwrap();
    let tail = store
        .load_conversation_message_window("window", None, 64, 500)
        .unwrap()
        .unwrap();
    assert_eq!(tail.total_message_count, 3);
    assert_eq!(
        tail.conversation
            .messages
            .iter()
            .map(|m| m.id.as_str())
            .collect::<Vec<_>>(),
        vec!["middle", "latest"]
    );
    assert!(tail.has_more);
    assert_eq!(tail.before_ordinal, Some(2));
    let older = store
        .load_conversation_message_window("window", tail.before_ordinal, 64, 500)
        .unwrap()
        .unwrap();
    assert!(!older.has_more);
    assert_eq!(older.conversation.messages.len(), 1);
    let preview = &older.conversation.messages[0].parts[0];
    assert_eq!(preview.content_text.len(), 500);
    assert!(
        preview.content_json.is_none(),
        "duplicated full JSON must not escape the byte budget"
    );
    assert_eq!(older.deferred_text_parts.len(), 1);
    let reference = &older.deferred_text_parts[0];
    assert_eq!(reference.message_id, "large");
    assert_eq!(reference.part_id, "part-large");
    assert_eq!(reference.total_utf8_bytes, 10_000_000);
    assert_eq!(reference.preview_utf8_bytes, 500);
    drop(store);
    let store = NolaStore::open(directory.path().join("nola.db")).unwrap();
    let mut offset = 0_u64;
    while offset < large.len() as u64 {
        let range = store
            .read_conversation_text_range("window", "large", "part-large", offset, 32 * 1024)
            .unwrap();
        assert!(range.text.len() <= 32 * 1024);
        assert!(
            range.next_offset_bytes > offset,
            "range must advance at {offset}"
        );
        assert_eq!(range.total_utf8_bytes, large.len() as u64);
        // Do not dump ten million characters into diagnostics on a mismatch.
        assert!(
            range.text.as_bytes()
                == &large.as_bytes()[offset as usize..range.next_offset_bytes as usize],
            "canonical text mismatch at byte {offset}"
        );
        offset = range.next_offset_bytes;
        assert_eq!(range.end_of_content, offset == large.len() as u64);
    }
}

#[test]
fn oversized_unicode_preview_respects_even_subscalar_budgets() {
    let directory = tempdir().unwrap();
    let mut store = NolaStore::open(directory.path().join("unicode.db")).unwrap();
    store
        .create_conversation(&CreateConversationInput {
            id: "unicode".into(),
            title: "Unicode".into(),
            revision: 1,
            settings_json: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        })
        .unwrap();
    let text = "😀中文".repeat(100);
    store
        .append_messages(&AppendMessagesInput {
            conversation_id: "unicode".into(),
            expected_message_count: 0,
            messages: vec![message("unicode-message", 1, &text)],
            updated_at_ms: 2,
        })
        .unwrap();
    for budget in 1..=12 {
        let window = store
            .load_conversation_message_window("unicode", None, 10, budget)
            .unwrap()
            .unwrap();
        assert_eq!(window.conversation.messages.len(), 1);
        assert_eq!(window.before_ordinal, Some(1));
        assert!(!window.has_more);
        let part = &window.conversation.messages[0].parts[0];
        assert!(part.content_text.len() <= budget);
        assert!(text.starts_with(&part.content_text));
        assert!(part.content_json.is_none());
        assert_eq!(
            window.deferred_text_parts[0].preview_utf8_bytes,
            part.content_text.len() as u64
        );
        assert_eq!(
            window.deferred_text_parts[0].total_utf8_bytes,
            text.len() as u64
        );
    }
}

#[test]
fn history_window_budget_counts_utf8_bytes_and_structured_content() {
    let directory = tempdir().unwrap();
    let mut store = NolaStore::open(directory.path().join("nola.db")).unwrap();
    store
        .create_conversation(&CreateConversationInput {
            id: "utf8-window".into(),
            title: "UTF-8 window".into(),
            revision: 1,
            settings_json: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        })
        .unwrap();
    let text = "\u{6587}".repeat(50);
    store
        .append_messages(&AppendMessagesInput {
            conversation_id: "utf8-window".into(),
            expected_message_count: 0,
            messages: (1..=3)
                .map(|ordinal| message(&format!("utf8-{ordinal}"), ordinal, &text))
                .collect(),
            updated_at_ms: 4,
        })
        .unwrap();
    // Each message has 150 bytes of text plus 161 bytes of JSON. Two
    // messages exceed 400 bytes, although their character counts do not.
    let mut cursor = None;
    for expected in (1..=3).rev() {
        let window = store
            .load_conversation_message_window("utf8-window", cursor, 64, 400)
            .unwrap()
            .unwrap();
        assert_eq!(window.conversation.messages.len(), 1);
        let selected = &window.conversation.messages[0];
        assert_eq!(selected.id, format!("utf8-{expected}"));
        assert_eq!(selected.parts[0].content_text, text);
        assert!(
            selected.parts[0].content_json.is_none(),
            "window JSON is loaded by the shared owner-bound reader"
        );
        let owner = nola_domain::CanonicalPartOwner::new(
            "utf8-window".into(),
            selected.id.clone(),
            selected.parts[0].id.clone(),
        )
        .unwrap();
        assert!(store
            .read_historical_part(&owner, window.conversation.revision, 8192)
            .unwrap()
            .is_some());
        assert_eq!(window.has_more, expected > 1);
        cursor = window.before_ordinal;
    }
}

#[test]
fn append_is_monotonic_and_stale_writer_rolls_back_without_touching_history() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("nola.db");
    let mut store = NolaStore::open(&database).unwrap();
    store
        .create_conversation(&CreateConversationInput {
            id: "conversation-1".into(),
            title: "Append E2E".into(),
            revision: 1,
            settings_json: None,
            created_at_ms: 1_700_000_000_000,
            updated_at_ms: 1_700_000_000_000,
        })
        .unwrap();
    store
        .append_messages(&AppendMessagesInput {
            conversation_id: "conversation-1".into(),
            expected_message_count: 0,
            messages: vec![message("message-1", 1, "old reply stays")],
            updated_at_ms: 1_700_000_000_000,
        })
        .unwrap();

    let result = store
        .append_messages(&AppendMessagesInput {
            conversation_id: "conversation-1".into(),
            expected_message_count: 1,
            messages: vec![message("message-2", 2, "new reply appends")],
            updated_at_ms: 1_700_000_000_100,
        })
        .unwrap();
    assert_eq!(result.message_count, 2);

    let stale = store.append_messages(&AppendMessagesInput {
        conversation_id: "conversation-1".into(),
        expected_message_count: 1,
        messages: vec![message("message-stale", 2, "must not appear")],
        updated_at_ms: 1_700_000_000_200,
    });
    assert!(stale.is_err());
    drop(store);

    let reopened = NolaStore::open(&database).unwrap();
    let conversation = reopened
        .load_conversation("conversation-1")
        .unwrap()
        .unwrap();
    let texts = conversation
        .messages
        .iter()
        .flat_map(|message| message.parts.iter())
        .map(|part| part.content_text.as_str())
        .collect::<Vec<_>>();
    assert_eq!(texts, vec!["old reply stays", "new reply appends"]);
}
