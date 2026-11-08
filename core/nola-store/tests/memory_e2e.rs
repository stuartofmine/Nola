use nola_store::{
    CreateConversationInput, NolaStore, SaveCheckpointInput, StoredMemoryRecord,
    StoredMemoryUsageEntry,
};

fn memory(id: &str, namespace_json: &str, content: &str, now: i64) -> StoredMemoryRecord {
    StoredMemoryRecord {
        id: id.into(),
        owner_json: serde_json::to_string(&nola_domain::memory_owner::MemoryExecutionBinding {
            workspace_id: Some("workspace-fixture".into()),
            ..nola_domain::memory_owner::MemoryExecutionBinding::local_profile()
        }.owner_for(&serde_json::from_str(namespace_json).unwrap(), nola_domain::MemoryScope::Global).unwrap()).unwrap(),
        namespace_json: namespace_json.into(),
        scope: "global".into(),
        kind: "preference".into(),
        content: content.into(),
        summary: Some("initial".into()),
        tags_json: "[\"release\"]".into(),
        confidence: 0.8,
        importance: 0.7,
        sensitivity: None,
        retention_json: "{\"mode\":\"until_superseded\"}".into(),
        expires_at: None,
        supersedes_json: "[]".into(),
        provenance_json: "{\"kind\":\"explicit_user\",\"source\":\"conversation-a\",\"session_id\":\"session-a\",\"turn_id\":null,\"item_ids\":[\"message-a\"],\"observed_at\":null}".into(),
        created_at_ms: now,
        updated_at_ms: now,
        last_used_at_ms: None,
        deleted_at_ms: None,
    }
}

#[test]
fn memory_identity_namespace_and_soft_delete_survive_reopen() {
    let temp = tempfile::tempdir().expect("tempdir");
    let path = temp.path().join("nola.sqlite");
    let personal = "{\"kind\":\"personal\"}";
    let workspace = "{\"kind\":\"workspace\"}";
    let content = "Prefer concise release notes with a verification section.";

    let mut store = NolaStore::open(&path).expect("open");
    let first = store
        .save_memory_record_deduplicated(&memory("memory-a", personal, content, 100))
        .expect("save first");
    let mut refreshed = memory("memory-new-id", personal, content, 200);
    refreshed.summary = Some("refreshed".into());
    refreshed.tags_json = "[\"verification\"]".into();
    let second = store
        .save_memory_record_deduplicated(&refreshed)
        .expect("refresh duplicate");
    assert_eq!(first.id, second.id);
    assert_eq!(second.created_at_ms, 100);
    assert_eq!(second.updated_at_ms, 200);
    assert_eq!(second.summary.as_deref(), Some("refreshed"));

    let other_namespace = store
        .save_memory_record_deduplicated(&memory("memory-workspace", workspace, content, 210))
        .expect("save workspace memory");
    assert_ne!(other_namespace.id, second.id);

    let recalled = store
        .search_memory_records(
            r#"{"kind":"profile","profile_id":"local"}"#,
            personal,
            "global",
            Some("%concise%"),
            None,
            "2026-08-18T00:00:00.000Z",
            10,
        )
        .expect("recall");
    assert_eq!(recalled.len(), 1);
    assert_eq!(recalled[0].id, "memory-a");
    assert!(store.delete_memory_record("memory-a", 300).expect("delete"));
    drop(store);

    let reopened = NolaStore::open(&path).expect("reopen");
    assert!(reopened.memory_record("memory-a").unwrap().is_none());
    assert!(reopened
        .search_memory_records(
            r#"{"kind":"profile","profile_id":"local"}"#,
            personal,
            "global",
            None,
            None,
            "2026-08-18T00:00:00.000Z",
            10
        )
        .unwrap()
        .is_empty());
    assert_eq!(reopened.list_memory_records(10).unwrap().len(), 1);
}

#[test]
fn expired_memory_is_not_recalled_but_remains_a_durable_fact() {
    let temp = tempfile::tempdir().expect("tempdir");
    let path = temp.path().join("nola.sqlite");
    let personal = "{\"kind\":\"personal\"}";
    let mut expired = memory("expired", personal, "Obsolete preference", 100);
    expired.expires_at = Some("2020-01-01T00:00:00.000Z".into());
    expired.retention_json =
        "{\"mode\":\"expires_at\",\"expires_at\":\"2020-01-01T00:00:00.000Z\"}".into();

    let mut store = NolaStore::open(&path).expect("open");
    store.upsert_memory_record(&expired).expect("save expired");
    assert!(store
        .search_memory_records(
            r#"{"kind":"profile","profile_id":"local"}"#,
            personal,
            "global",
            None,
            None,
            "2026-08-18T00:00:00.000Z",
            10
        )
        .unwrap()
        .is_empty());
    drop(store);

    let reopened = NolaStore::open(&path).expect("reopen");
    let durable = reopened
        .memory_record("expired")
        .unwrap()
        .expect("durable fact");
    assert_eq!(durable.provenance_json, expired.provenance_json);
    assert_eq!(durable.retention_json, expired.retention_json);
}

#[test]
fn refreshed_observation_preserves_last_use_metadata() {
    let mut store = NolaStore::open(":memory:").unwrap();
    let namespace = "{\"kind\":\"personal\"}";
    let mut initial = memory("original", namespace, "Remember this", 100);
    initial.last_used_at_ms = Some(200);
    store.upsert_memory_record(&initial).unwrap();
    let mut refreshed = memory("duplicate", namespace, "Remember this", 300);
    let unchanged = store.save_memory_record_deduplicated(&refreshed).unwrap();
    assert_eq!(unchanged.updated_at_ms, 100);
    assert_eq!(unchanged.last_used_at_ms, Some(200));
    refreshed.summary = Some("Updated summary".into());
    let result = store.save_memory_record_deduplicated(&refreshed).unwrap();
    assert_eq!(result.id, "original");
    assert_eq!(result.created_at_ms, 100);
    assert_eq!(result.updated_at_ms, 300);
    assert_eq!(result.last_used_at_ms, Some(200));
}

#[test]
fn inferred_duplicate_cannot_replace_explicit_user_provenance() {
    let mut store = NolaStore::open(":memory:").unwrap();
    let namespace = "{\"kind\":\"personal\"}";
    let content = "Always include a verification section.";
    let mut explicit = memory("user-fact", namespace, content, 100);
    explicit.confidence = 1.0;
    explicit.summary = Some("User correction".into());
    store
        .save_memory_record_deduplicated(&explicit)
        .expect("save explicit fact");

    let mut inferred = memory("automatic-candidate", namespace, content, 200);
    inferred.confidence = 0.7;
    inferred.summary = Some("Model extraction".into());
    inferred.provenance_json = "{\"kind\":\"agent_inference\",\"source\":\"memory-worker\",\"session_id\":\"session-a\",\"turn_id\":\"turn-a\",\"item_ids\":[\"message-a\"],\"observed_at\":\"2026-09-10T00:00:00Z\"}".into();

    let stored = store
        .save_memory_record_deduplicated(&inferred)
        .expect("deduplicate inferred candidate");
    assert_eq!(stored.id, "user-fact");
    assert_eq!(stored.provenance_json, explicit.provenance_json);
    assert_eq!(stored.confidence, 1.0);
    assert_eq!(stored.summary.as_deref(), Some("User correction"));
    assert_eq!(stored.updated_at_ms, 100);
    assert!(store
        .memory_record("automatic-candidate")
        .unwrap()
        .is_none());
}

#[test]
fn deduplication_cannot_create_self_supersession_or_hide_the_original_after_reopen() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("self-supersession.db");
    let namespace = "{\"kind\":\"personal\"}";
    let content = "Keep the canonical verification workflow.";
    let mut store = NolaStore::open(&path).unwrap();
    store
        .save_memory_record_deduplicated(&memory("original", namespace, content, 100))
        .unwrap();
    let mut duplicate = memory("replacement", namespace, content, 200);
    duplicate.supersedes_json = "[\"original\"]".into();

    let error = store
        .save_memory_record_deduplicated(&duplicate)
        .unwrap_err();
    assert!(error
        .to_string()
        .contains("memory_supersession_identity_conflict"));
    drop(store);

    let reopened = NolaStore::open(&path).unwrap();
    let original = reopened.memory_record("original").unwrap().unwrap();
    assert_eq!(original.supersedes_json, "[]");
    assert_eq!(original.updated_at_ms, 100);
    assert!(reopened.memory_record("replacement").unwrap().is_none());
    assert_eq!(
        reopened
            .search_memory_records(
            r#"{"kind":"profile","profile_id":"local"}"#,
                namespace,
                "global",
                Some("%canonical verification%"),
                None,
                "2026-09-10T00:00:00.000Z",
                10,
            )
            .unwrap()
            .into_iter()
            .map(|record| record.id)
            .collect::<Vec<_>>(),
        vec!["original"]
    );
}

fn checkpoint_fixture(store: &mut NolaStore) -> SaveCheckpointInput {
    store
        .create_conversation(&CreateConversationInput {
            id: "maintenance".into(),
            title: "Memory maintenance".into(),
            revision: 1,
            settings_json: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        })
        .unwrap();
    let initial = SaveCheckpointInput {
        conversation_id: "maintenance".into(),
        expected_revision: 0,
        revision: 1,
        event_cursor: 1,
        checkpoint_json: "{\"job\":\"pending\"}".into(),
        updated_at_ms: 1,
    };
    store.save_checkpoint(&initial).unwrap();
    SaveCheckpointInput {
        expected_revision: 1,
        revision: 2,
        event_cursor: 2,
        checkpoint_json: "{\"job\":\"completed\"}".into(),
        updated_at_ms: 500,
        ..initial
    }
}

fn usage(id: &str, memory_id: &str, session_id: &str, used_at_ms: i64) -> StoredMemoryUsageEntry {
    StoredMemoryUsageEntry {
        usage_id: id.into(),
        memory_id: memory_id.into(),
        session_id: session_id.into(),
        turn_id: format!("turn-{session_id}"),
        used_at_ms,
        context_payload_digest: format!("digest-{session_id}"),
        citation_ids_json: "[]".into(),
    }
}

fn next_checkpoint(store: &mut NolaStore, conversation_id: &str, now: i64) -> SaveCheckpointInput {
    store
        .create_conversation(&CreateConversationInput {
            id: conversation_id.into(),
            title: conversation_id.into(),
            revision: 1,
            settings_json: None,
            created_at_ms: now,
            updated_at_ms: now,
        })
        .unwrap();
    store
        .save_checkpoint(&SaveCheckpointInput {
            conversation_id: conversation_id.into(),
            expected_revision: 0,
            revision: 1,
            event_cursor: 1,
            checkpoint_json: "{\"state\":\"before\"}".into(),
            updated_at_ms: now,
        })
        .unwrap();
    SaveCheckpointInput {
        conversation_id: conversation_id.into(),
        expected_revision: 1,
        revision: 2,
        event_cursor: 2,
        checkpoint_json: "{\"state\":\"after\"}".into(),
        updated_at_ms: now + 1,
    }
}

#[test]
fn usage_is_idempotent_atomic_and_ranks_memory_across_sessions() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("usage.db");
    let namespace = "{\"kind\":\"personal\"}";
    let mut store = NolaStore::open(&path).unwrap();
    store
        .upsert_memory_record(&memory("used", namespace, "shared recall phrase", 100))
        .unwrap();
    store
        .upsert_memory_record(&memory("unused", namespace, "shared recall phrase", 200))
        .unwrap();

    let first_checkpoint = next_checkpoint(&mut store, "conversation-a", 300);
    let first_usage = usage("usage-a", "used", "session-a", 500);
    store
        .save_checkpoint_with_memories(&first_checkpoint, &[], &[], &[first_usage.clone()])
        .unwrap();

    let second_checkpoint = next_checkpoint(&mut store, "conversation-b", 600);
    let second_usage = usage("usage-b", "used", "session-b", 700);
    store
        .save_checkpoint_with_memories(
            &second_checkpoint,
            &[],
            &[],
            &[first_usage.clone(), second_usage],
        )
        .unwrap();
    drop(store);

    let mut reopened = NolaStore::open(&path).unwrap();
    let stats = reopened
        .memory_usage_stats(&["used".into(), "unused".into()])
        .unwrap();
    assert_eq!(stats.len(), 1);
    assert_eq!(stats[0].memory_id, "used");
    assert_eq!(stats[0].usage_count, 2);
    assert_eq!(stats[0].last_used_at_ms, 700);
    assert_eq!(
        reopened
            .memory_record("used")
            .unwrap()
            .unwrap()
            .last_used_at_ms,
        Some(700)
    );
    assert_eq!(reopened.list_memory_records(2).unwrap()[0].id, "used");
    assert_eq!(
        reopened
            .search_memory_records(
            r#"{"kind":"profile","profile_id":"local"}"#,
                namespace,
                "global",
                Some("%shared recall phrase%"),
                None,
                "2026-09-10T00:00:00.000Z",
                2,
            )
            .unwrap()[0]
            .id,
        "used"
    );

    let conflict_checkpoint = SaveCheckpointInput {
        expected_revision: 2,
        revision: 3,
        event_cursor: 3,
        checkpoint_json: "{\"state\":\"conflicting\"}".into(),
        updated_at_ms: 800,
        ..first_checkpoint
    };
    let mut conflicting = first_usage;
    conflicting.memory_id = "unused".into();
    let error = reopened
        .save_checkpoint_with_memories(&conflict_checkpoint, &[], &[], &[conflicting])
        .unwrap_err();
    assert!(error.to_string().contains("memory usage identity conflict"));
    assert_eq!(
        reopened
            .load_checkpoint("conversation-a")
            .unwrap()
            .unwrap()
            .revision,
        2
    );
    assert_eq!(
        reopened.memory_usage_stats(&["used".into()]).unwrap()[0].usage_count,
        2
    );
    assert_eq!(
        reopened
            .memory_record("unused")
            .unwrap()
            .unwrap()
            .last_used_at_ms,
        None
    );
}

#[test]
fn maintenance_conflicts_roll_back_checkpoint_memories_and_sync_after_restart() {
    for scenario in ["deleted", "namespace", "scope", "stale"] {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("maintenance.db");
        let mut store = NolaStore::open(&path).unwrap();
        let checkpoint = checkpoint_fixture(&mut store);
        let original = memory("existing", "{\"kind\":\"personal\"}", "User decision", 300);
        store.upsert_memory_record(&original).unwrap();
        if scenario == "deleted" {
            store.delete_memory_record("existing", 350).unwrap();
        }
        drop(store);
        let sync_count = || {
            let connection = rusqlite::Connection::open_with_flags(
                &path,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            )
            .unwrap();
            connection
                .query_row("SELECT COUNT(*) FROM sync_outbox", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap()
        };
        let before = sync_count();
        let mut store = NolaStore::open(&path).unwrap();
        let mut candidate = original.clone();
        candidate.content = "Background replacement".into();
        candidate.updated_at_ms = 400;
        let expected = match scenario {
            "deleted" => "memory_deleted_conflict",
            "namespace" => {
                candidate.namespace_json = "{\"kind\":\"workspace\"}".into();
                candidate.owner_json = r#"{"kind":"workspace","profile_id":"local","workspace_id":"fixture"}"#.into();
                "memory_scope_conflict"
            }
            "scope" => {
                candidate.scope = "conversation".into();
                candidate.owner_json = r#"{"kind":"conversation","profile_id":"local","conversation_id":"fixture"}"#.into();
                "memory_scope_conflict"
            }
            "stale" => {
                candidate.updated_at_ms = 200;
                "memory_stale_candidate"
            }
            _ => unreachable!(),
        };
        let fresh = memory(
            "new-before-conflict",
            &original.namespace_json,
            "New candidate",
            400,
        );
        let error = store
            .save_checkpoint_with_memories(&checkpoint, &[fresh, candidate], &[], &[])
            .unwrap_err();
        assert!(error.to_string().contains(expected), "{scenario}: {error}");
        drop(store);
        assert_eq!(sync_count(), before, "{scenario}: partial sync publication");
        let store = NolaStore::open(&path).unwrap();
        let durable = store.load_checkpoint("maintenance").unwrap().unwrap();
        assert_eq!(durable.revision, 1);
        assert_eq!(durable.checkpoint_json, "{\"job\":\"pending\"}");
        assert!(store
            .memory_record("new-before-conflict")
            .unwrap()
            .is_none());
        let existing = store.memory_record("existing").unwrap();
        if scenario == "deleted" {
            assert!(existing.is_none());
        } else {
            assert_eq!(existing.unwrap().content, original.content);
        }
    }
}

#[test]
fn maintenance_preserves_canonical_identity_and_last_use_with_checkpoint() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("maintenance.db");
    let mut store = NolaStore::open(&path).unwrap();
    let checkpoint = checkpoint_fixture(&mut store);
    let mut original = memory("existing", "{\"kind\":\"personal\"}", "User decision", 100);
    original.last_used_at_ms = Some(250);
    store.upsert_memory_record(&original).unwrap();
    let mut candidate = original.clone();
    candidate.summary = Some("Consolidated explanation".into());
    candidate.created_at_ms = 400;
    candidate.updated_at_ms = 400;
    candidate.last_used_at_ms = None;
    let (_, accepted) = store
        .save_checkpoint_with_memories(&checkpoint, &[candidate], &[], &[])
        .unwrap();
    assert_eq!(accepted[0].id, "existing");
    drop(store);
    let store = NolaStore::open(&path).unwrap();
    let durable = store.memory_record("existing").unwrap().unwrap();
    assert_eq!(durable.created_at_ms, 100);
    assert_eq!(durable.updated_at_ms, 400);
    assert_eq!(durable.last_used_at_ms, Some(250));
    assert_eq!(durable.summary.as_deref(), Some("Consolidated explanation"));
    assert_eq!(
        store
            .load_checkpoint("maintenance")
            .unwrap()
            .unwrap()
            .revision,
        2
    );
}

#[test]
fn maintenance_rechecks_input_revision_after_another_writer_commits() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("race.db");
    let mut first = NolaStore::open(&path).unwrap();
    let checkpoint = checkpoint_fixture(&mut first);
    let original = memory("input", "{\"kind\":\"personal\"}", "Old preference", 100);
    first.upsert_memory_record(&original).unwrap();
    let (_, revision) = first.memory_record_with_revision("input").unwrap().unwrap();
    let mut second = NolaStore::open(&path).unwrap();
    let mut changed = original;
    changed.content = "New preference".into();
    changed.updated_at_ms = 200;
    second.save_memory_record_deduplicated(&changed).unwrap();
    drop(second);
    let error = first
        .save_checkpoint_with_memories(&checkpoint, &[], &[("input".into(), revision)], &[])
        .unwrap_err();
    assert!(error.to_string().contains("memory_input_conflict"));
    drop(first);
    let reopened = NolaStore::open(&path).unwrap();
    assert_eq!(
        reopened
            .load_checkpoint("maintenance")
            .unwrap()
            .unwrap()
            .revision,
        1
    );
    assert_eq!(
        reopened.memory_record("input").unwrap().unwrap().content,
        "New preference"
    );
}
