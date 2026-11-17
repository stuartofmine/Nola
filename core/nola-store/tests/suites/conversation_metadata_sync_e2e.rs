// SPDX-License-Identifier: Apache-2.0
use nola_domain::ConversationMetadataReplica;
use nola_store::*;

fn snapshot() -> String {
    let root = tempfile::tempdir().unwrap();
    let mut source = NolaStore::open(root.path().join("source.db")).unwrap();
    source
        .create_conversation(&CreateConversationInput {
            id: "c".into(),
            title: "Original".into(),
            revision: 1,
            settings_json: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        })
        .unwrap();
    source
        .append_messages(&AppendMessagesInput {
            conversation_id: "c".into(),
            expected_message_count: 0,
            updated_at_ms: 2,
            messages: vec![MessageInput {
                id: "m".into(),
                run_id: None,
                role: "user".into(),
                status: "completed".into(),
                ordinal: 1,
                created_at_ms: 1,
                completed_at_ms: Some(1),
                parts: vec![MessagePartInput {
                    id: "p".into(),
                    kind: "text".into(),
                    ordinal: 1,
                    state: "completed".into(),
                    content_text: "original evidence".into(),
                    content_json: None,
                    content_digest: None,
                    created_at_ms: 1,
                    updated_at_ms: 1,
                }],
            }],
        })
        .unwrap();
    serde_json::to_string(&source.load_conversation("c").unwrap().unwrap()).unwrap()
}

fn change(cursor: i64, revision: i64, kind: &str, payload: String) -> RemoteSyncChangeInput {
    RemoteSyncChangeInput {
        cursor,
        revision,
        entity_type: kind.into(),
        entity_id: "c".into(),
        operation_id: format!("op-{cursor}"),
        deleted: false,
        payload_json: payload,
        created_at_ms: cursor,
    }
}
fn metadata(cursor: i64, revision: i64, base: u64) -> RemoteSyncChangeInput {
    change(
        cursor,
        revision,
        "conversation_metadata",
        serde_json::to_string(
            &ConversationMetadataReplica::new(
                "c".into(),
                revision as u64,
                base,
                "New title".into(),
                Some(r#"{"memoryEnabled":true}"#.into()),
                4,
            )
            .unwrap(),
        )
        .unwrap(),
    )
}
fn page(changes: Vec<RemoteSyncChangeInput>) -> ApplyRemoteSyncPageInput {
    ApplyRemoteSyncPageInput {
        replica_id: "remote".into(),
        next_cursor: changes.last().unwrap().cursor,
        changes,
        applied_at_ms: 10,
    }
}

#[test]
fn metadata_preserves_original_parts_and_replays_after_restart() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("replica.db");
    let mut store = NolaStore::open(&path).unwrap();
    store
        .apply_remote_sync_page(&page(vec![change(1, 2, "conversation", snapshot())]))
        .unwrap();
    let before = store.load_conversation("c").unwrap().unwrap();
    let event = metadata(2, 4, 2);
    store
        .apply_remote_sync_page(&page(vec![event.clone()]))
        .unwrap();
    drop(store);
    let mut store = NolaStore::open(&path).unwrap();
    let after = store.load_conversation("c").unwrap().unwrap();
    assert_eq!(
        serde_json::to_value(&before.messages).unwrap(),
        serde_json::to_value(&after.messages).unwrap()
    );
    assert_eq!(after.title, "New title");
    assert_eq!(after.revision, 4);
    assert!(
        store.pending_sync_changes(10).unwrap().is_empty(),
        "remote receive must not echo into the outbox"
    );
    let mut replay = event;
    replay.cursor = 3;
    let outcome = store.apply_remote_sync_page(&page(vec![replay])).unwrap();
    assert_eq!(outcome.skipped, 1);
    assert_eq!(store.sync_cursor("remote").unwrap().unwrap().cursor, 3);
}

#[test]
fn missing_content_base_rolls_back_the_entire_page() {
    let root = tempfile::tempdir().unwrap();
    let mut store = NolaStore::open(root.path().join("replica.db")).unwrap();
    let error = store
        .apply_remote_sync_page(&page(vec![
            change(1, 2, "conversation", snapshot()),
            metadata(2, 4, 3),
        ]))
        .unwrap_err();
    assert!(error.to_string().contains("requires content base 3"));
    assert!(store.load_conversation("c").unwrap().is_none());
    assert!(store.sync_cursor("remote").unwrap().is_none());
}

#[test]
fn delayed_content_does_not_regress_newer_metadata() {
    let root = tempfile::tempdir().unwrap();
    let mut store = NolaStore::open(root.path().join("replica.db")).unwrap();
    let original = snapshot();
    store
        .apply_remote_sync_page(&page(vec![
            change(1, 2, "conversation", original.clone()),
            metadata(2, 4, 2),
        ]))
        .unwrap();
    let mut delayed: StoredConversation = serde_json::from_str(&original).unwrap();
    delayed.revision = 3;
    delayed.messages[0].parts[0].content_text = "new content in an older publication".into();
    store
        .apply_remote_sync_page(&page(vec![change(
            3,
            3,
            "conversation",
            serde_json::to_string(&delayed).unwrap(),
        )]))
        .unwrap();
    let current = store.load_conversation("c").unwrap().unwrap();
    assert_eq!(current.title, "New title");
    assert_eq!(current.revision, 4);
    assert_eq!(
        current.settings_json.as_deref(),
        Some(r#"{"memoryEnabled":true}"#)
    );
    assert_eq!(
        current.messages[0].parts[0].content_text,
        "new content in an older publication"
    );
}

#[test]
fn tombstone_blocks_metadata_and_full_snapshot_resurrection() {
    let root = tempfile::tempdir().unwrap();
    let mut store = NolaStore::open(root.path().join("replica.db")).unwrap();
    store
        .apply_remote_sync_page(&page(vec![change(1, 2, "conversation", snapshot())]))
        .unwrap();
    let mut deletion = change(2, 5, "conversation", "null".into());
    deletion.deleted = true;
    store.apply_remote_sync_page(&page(vec![deletion])).unwrap();
    assert_eq!(
        store
            .apply_remote_sync_page(&page(vec![metadata(3, 6, 2)]))
            .unwrap()
            .skipped,
        1
    );
    let mut late: StoredConversation = serde_json::from_str(&snapshot()).unwrap();
    late.revision = 7;
    assert_eq!(
        store
            .apply_remote_sync_page(&page(vec![change(
                4,
                7,
                "conversation",
                serde_json::to_string(&late).unwrap()
            )]))
            .unwrap()
            .skipped,
        1
    );
    assert!(store.load_conversation("c").unwrap().is_none());
}

#[test]
fn metadata_identity_mismatch_does_not_advance_cursor() {
    let root = tempfile::tempdir().unwrap();
    let mut store = NolaStore::open(root.path().join("replica.db")).unwrap();
    store
        .apply_remote_sync_page(&page(vec![change(1, 2, "conversation", snapshot())]))
        .unwrap();
    let mut wrong = metadata(2, 4, 2);
    wrong.entity_id = "other".into();
    assert!(store.apply_remote_sync_page(&page(vec![wrong])).is_err());
    assert_eq!(store.sync_cursor("remote").unwrap().unwrap().cursor, 1);
    assert_eq!(
        store.load_conversation("c").unwrap().unwrap().title,
        "Original"
    );
}

#[test]
fn sender_publishes_initial_base_before_offline_metadata_changes() {
    let root = tempfile::tempdir().unwrap();
    let sender_path = root.path().join("sender.db");
    let mut sender = NolaStore::open(&sender_path).unwrap();
    sender
        .create_conversation(&CreateConversationInput {
            id: "c".into(),
            title: "Initial".into(),
            revision: 1,
            settings_json: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        })
        .unwrap();
    sender
        .patch_conversation_configuration(&nola_store_port::conversation_profile::PatchConversationConfigurationInput {
            conversation_id: "c".into(),
            expected_revision: 1,
            patch_json: r#"{"memoryEnabledOverride":true}"#.parse().unwrap(),
            updated_at_ms: 2,
        })
        .unwrap();
    sender.rename_conversation("c", "Offline title", None, 3).unwrap();
    drop(sender);
    let sender = NolaStore::open(&sender_path).unwrap();
    let pending = sender.pending_sync_changes(10).unwrap();
    assert_eq!(pending.len(), 3);
    assert_eq!(pending[0].entity_type, "conversation");
    for item in &pending[1..] {
        assert_eq!(item.entity_type, "conversation_metadata");
        let metadata: ConversationMetadataReplica =
            serde_json::from_str(&item.payload_json).unwrap();
        assert_eq!(metadata.base_content_revision(), 1);
        assert!(item.payload_json.len() < 1024);
    }
    let changes = pending
        .into_iter()
        .enumerate()
        .map(|(index, entry)| RemoteSyncChangeInput {
            cursor: index as i64 + 1,
            operation_id: entry.operation_id,
            entity_type: entry.entity_type,
            entity_id: entry.entity_id,
            revision: entry.revision,
            deleted: entry.deleted,
            payload_json: entry.payload_json,
            created_at_ms: entry.created_at_ms,
        })
        .collect();
    let mut receiver = NolaStore::open(root.path().join("receiver.db")).unwrap();
    receiver.apply_remote_sync_page(&page(changes)).unwrap();
    let restored = receiver.load_conversation("c").unwrap().unwrap();
    assert_eq!(restored.title, "Offline title");
    assert_eq!(
        restored.settings_json.as_deref(),
        Some(r#"{"memoryEnabledOverride":true}"#)
    );
    assert_eq!(restored.revision, 3);
    assert!(restored.messages.is_empty());
}

#[test]
fn offline_appends_and_metadata_restore_in_order_without_republishing_old_text() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("sender.db");
    let mut sender = NolaStore::open(&path).unwrap();
    sender
        .create_conversation(&CreateConversationInput {
            id: "c".into(),
            title: "Initial".into(),
            revision: 1,
            settings_json: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        })
        .unwrap();
    for ordinal in 1..=2 {
        sender
            .append_messages(&AppendMessagesInput {
                conversation_id: "c".into(),
                expected_message_count: ordinal - 1,
                updated_at_ms: ordinal as i64 + 1,
                messages: vec![MessageInput {
                    id: format!("m{ordinal}"),
                    run_id: None,
                    role: "user".into(),
                    status: "completed".into(),
                    ordinal: ordinal as i64,
                    created_at_ms: 1,
                    completed_at_ms: Some(1),
                    parts: vec![MessagePartInput {
                        id: format!("p{ordinal}"),
                        kind: "text".into(),
                        ordinal: 1,
                        state: "completed".into(),
                        content_text: if ordinal == 1 {
                            "x".repeat(100_000)
                        } else {
                            "new evidence".into()
                        },
                        content_json: None,
                        content_digest: None,
                        created_at_ms: 1,
                        updated_at_ms: 1,
                    }],
                }],
            })
            .unwrap();
        sender
            .rename_conversation("c", &format!("Title {ordinal}"), None, ordinal as i64 + 2)
            .unwrap();
    }
    drop(sender);
    let sender = NolaStore::open(&path).unwrap();
    let pending = sender.pending_sync_changes(10).unwrap();
    assert_eq!(pending.len(), 5);
    assert_eq!(pending[3].entity_type, "conversation_append");
    assert!(
        pending[3].payload_json.len() < 2048,
        "second append must exclude old text"
    );
    let changes: Vec<_> = pending
        .into_iter()
        .enumerate()
        .map(|(index, entry)| RemoteSyncChangeInput {
            cursor: index as i64 + 1,
            operation_id: entry.operation_id,
            entity_type: entry.entity_type,
            entity_id: entry.entity_id,
            revision: entry.revision,
            deleted: entry.deleted,
            payload_json: entry.payload_json,
            created_at_ms: entry.created_at_ms,
        })
        .collect();
    let receiver_path = root.path().join("receiver.db");
    let mut receiver = NolaStore::open(&receiver_path).unwrap();
    assert!(receiver
        .apply_remote_sync_page(&page(vec![changes[3].clone()]))
        .is_err());
    assert!(receiver.sync_cursor("remote").unwrap().is_none());
    receiver
        .apply_remote_sync_page(&page(changes.clone()))
        .unwrap();
    drop(receiver);
    let mut receiver = NolaStore::open(&receiver_path).unwrap();
    let restored = receiver.load_conversation("c").unwrap().unwrap();
    assert_eq!(restored.title, "Title 2");
    assert_eq!(restored.messages.len(), 2);
    assert_eq!(restored.messages[0].parts[0].content_text.len(), 100_000);
    assert_eq!(restored.messages[1].parts[0].content_text, "new evidence");
    let mut replay = changes[3].clone();
    replay.cursor = 6;
    assert_eq!(
        receiver
            .apply_remote_sync_page(&page(vec![replay]))
            .unwrap()
            .skipped,
        1
    );
    assert_eq!(
        receiver
            .load_conversation("c")
            .unwrap()
            .unwrap()
            .messages
            .len(),
        2
    );
}
