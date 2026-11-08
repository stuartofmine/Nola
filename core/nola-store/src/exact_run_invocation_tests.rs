// SPDX-License-Identifier: Apache-2.0
use super::*;
#[path = "../tests/support/terminal_sources.rs"]
mod terminal_sources;

#[test]
fn unsupported_nonzero_versions_are_rejected_without_changing_stored_facts() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("unsupported-version.sqlite");
    // This fixture deliberately changes the database version. No writer from
    // the initial current-version open may remain active during the rejection
    // checks; otherwise its archive ACK is unrelated to the rejected open.
    let mut store = NolaStore::open_archive_mode(&path, false).unwrap();
    fixture(&mut store);
    drop(store);
    let sql = Connection::open(&path).unwrap();
    let snapshot = |sql: &Connection| {
        let mut statement = sql.prepare("SELECT type,name,coalesce(sql,'') FROM sqlite_master ORDER BY type,name").unwrap();
        let schema = statement.query_map([], |row| Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?)))
            .unwrap().collect::<Result<Vec<_>,_>>().unwrap();
        let facts: Vec<_> = schema.iter().filter(|(kind, _, _)| kind == "table").map(|(_, name, _)| {
            let mut statement = sql.prepare(&format!("SELECT * FROM \"{}\"", name.replace('"', "\"\""))).unwrap();
            let columns = statement.column_count();
            let mut rows: Vec<String> = statement.query_map([], |row| {
                let values = (0..columns).map(|index| row.get::<_,rusqlite::types::Value>(index)).collect::<Result<Vec<_>,_>>()?;
                Ok(format!("{values:?}"))
            }).unwrap().collect::<Result<_,_>>().unwrap();
            rows.sort();
            (name.clone(), rows)
        }).collect();
        (schema, facts)
    };
    let original = snapshot(&sql);
    for version in [83, 27, 54, 62, 63, 64, 65, 66, 73, 74, crate::STORE_SCHEMA_VERSION + 1] {
        sql.pragma_update(None,"user_version",version).unwrap();
        assert!(matches!(NolaStore::open(&path), Err(StoreError::IncompatibleSchema)), "version {version}");
        assert_eq!(sql.query_row("PRAGMA user_version", [], |row| row.get::<_,i64>(0)).unwrap(),version);
        assert_eq!(snapshot(&sql),original,"version {version} must not change schema or source facts");
    }
}
use nola_store_port::run_recovery_inspection::RunRecoveryInspectionRepository;

fn fixture_input() -> BeginTurnInput {
    BeginTurnInput {
        user_part_kind: "text".into(),
        expected_configuration_revision: None,
        expected_parent_message_id: None,
        initial_events: Vec::new(),
        initial_assistant_part_mutations: vec![],
        conversation_id: "conversation".into(),
        conversation_title: "Fixture".into(),
        user_message_id: "user".into(),
        user_part_id: "user-part".into(),
        user_text: "private user history must not be returned".into(),
        user_content_json: None,
        additional_user_parts: vec![],
        assistant_message_id: "assistant".into(),
        assistant_part_id: "assistant-part".into(),
        assistant_turn_bindings: vec![],
        run_id: "run".into(),
        context_checkpoint_id: None,
        checkpoint_revision: 1,
        checkpoint_event_cursor: 0,
        checkpoint_json: "{}".into(),
        invocation_config_json: r#"{"model":"original"}"#.into(),
        invocation_config_digest: "original-digest".into(),
        outbox_effects: vec![],
        model_attempt_mutations: vec![],
        delegated_invocation_mutations: vec![],
        delegated_permission_mutations: vec![],
        memory_usage_entries: vec![],
        created_at_ms: 1,
    }
}

fn fixture(store: &mut NolaStore) {
    store.begin_turn(&fixture_input()).unwrap();
}

fn exact(store: &NolaStore) -> StoreResult<Option<StoredExactRunInvocation>> {
    store.load_exact_run_invocation("conversation", "run", "assistant")
}

#[test]
fn exact_invocation_memo_is_taken_from_the_same_pinned_snapshot() {
    use nola_store_port::run_effect::RunEffectRepository;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("exact-memo.sqlite");
    let mut store = NolaStore::open_archive_mode(&path, false).unwrap();
    fixture(&mut store);
    let checkpoint = store.load_checkpoint("conversation").unwrap().unwrap();
    assert!(exact(&store).unwrap().unwrap().core_checkpoint.unwrap().recovered_checkpoint.is_none());
    let writer = Connection::open(&path).unwrap();
    {
        let _snapshot = read_snapshot::pin(&store.connection).unwrap();
        store.cache_read_checkpoint(&checkpoint).unwrap();
        writer.execute("UPDATE conversations SET title='changed' WHERE id='conversation'", []).unwrap();
        let proof = exact(&store).unwrap().unwrap().core_checkpoint.unwrap();
        assert_eq!(proof.recovered_checkpoint, Some(checkpoint.clone()));
        assert_eq!(proof.recovery.recovery.checkpoint, checkpoint);
    }
    assert!(exact(&store).unwrap().unwrap().core_checkpoint.unwrap().recovered_checkpoint.is_none(),
        "A newer database snapshot must not carry an old recovery materialization");
    {
        let _snapshot = read_snapshot::pin(&store.connection).unwrap();
        store.cache_read_checkpoint(&checkpoint).unwrap();
    }
    store.connection.execute("UPDATE conversations SET title='local' WHERE id='conversation'", []).unwrap();
    assert!(exact(&store).unwrap().unwrap().core_checkpoint.unwrap().recovered_checkpoint.is_none());
}

#[test]
fn exact_initial_admission_preserves_core_identity_and_rejects_corrupt_first_fact() {
    let mut store = NolaStore::open(":memory:").unwrap();
    let mut input = fixture_input();
    input.checkpoint_event_cursor = 3;
    input.checkpoint_json = r#"{"session_id":"actual-core","state":{"session_id":"actual-core"},"revision":7,"last_event_cursor":{"sequence":3}}"#.into();
    input.initial_events = vec![nola_store_port::run_creation::InitialCoreRunEvent {
        core_session_id: "actual-core".into(),
        core_turn_id: Some("run".into()),
        core_cursor: 3,
        core_revision: 7,
        event_kind: "turn_started".into(),
        payload_json: r#"{"type":"turn_started","session_id":"actual-core","turn_id":"run"}"#
            .into(),
    }];
    store.begin_turn(&input).unwrap();
    let proof = exact(&store)
        .unwrap()
        .unwrap()
        .core_checkpoint
        .unwrap()
        .initial_admission
        .unwrap();
    assert_eq!(
        proof,
        StoredInitialCoreAdmission {
            session_id: "actual-core".into(),
            turn_id: "run".into(),
            revision: 7,
            event_cursor: 3,
        }
    );
    let original: String = store
        .connection
        .query_row(
            "SELECT payload_json FROM run_events WHERE run_id='run' AND event_kind='turn_started'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let valid: serde_json::Value = serde_json::from_str(&original).unwrap();
    let mut corrupt = Vec::new();
    for (pointer, value) in [
        ("/core_envelope/turn_id", serde_json::json!("other-run")),
        (
            "/core_envelope/session_id",
            serde_json::json!("other-session"),
        ),
        ("/core_envelope/cursor/sequence", serde_json::json!(-1)),
        ("/core_envelope/revision", serde_json::json!("7")),
    ] {
        let mut changed = valid.clone();
        *changed.pointer_mut(pointer).unwrap() = value;
        corrupt.push(changed.to_string());
    }
    corrupt.push("{".into());
    let mut oversized = valid.clone();
    oversized["padding"] = serde_json::json!("x".repeat(16384));
    corrupt.push(oversized.to_string());
    for raw in corrupt {
        store.connection.execute("UPDATE run_events SET payload_json=?1 WHERE run_id='run' AND event_kind='turn_started'", [raw]).unwrap();
        assert!(exact(&store).is_err());
    }
    store.connection.execute("UPDATE run_events SET payload_json=?1 WHERE run_id='run' AND event_kind='turn_started'", [original]).unwrap();
    assert_eq!(
        exact(&store)
            .unwrap()
            .unwrap()
            .core_checkpoint
            .unwrap()
            .initial_admission,
        Some(proof)
    );
    store
        .connection
        .execute(
            "DELETE FROM run_events WHERE run_id='run' AND event_kind='turn_started'",
            [],
        )
        .unwrap();
    assert!(exact(&store)
        .unwrap()
        .unwrap()
        .core_checkpoint
        .unwrap()
        .initial_admission
        .is_none());
}

#[test]
fn initial_fact_identity_is_bound_to_core_baseline_not_store_clock() {
    let directory = tempfile::tempdir().unwrap();
    let mut store = NolaStore::open(directory.path().join("initial-owner.db")).unwrap();
    let mut input = fixture_input();
    input.checkpoint_event_cursor = 3;
    input.checkpoint_json=r#"{"session_id":"actual-core","state":{"session_id":"actual-core"},"revision":7,"last_event_cursor":{"sequence":3}}"#.into();
    input.initial_events = vec![nola_store_port::run_creation::InitialCoreRunEvent {
        core_session_id: "foreign-core".into(),
        core_turn_id: Some("run".into()),
        core_cursor: 3,
        core_revision: 7,
        event_kind: "context_updated".into(),
        payload_json: r#"{"type":"context_updated","used_tokens":3}"#.into(),
    }];
    assert!(store.begin_turn(&input).is_err());
    for table in [
        "runs",
        "core_checkpoints",
        "core_checkpoint_bodies",
        "run_events",
    ] {
        assert_eq!(
            store
                .connection
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    input.initial_events[0].core_session_id = "actual-core".into();
    store.begin_turn(&input).unwrap();
    assert_eq!(store.checkpoint_revision("conversation").unwrap(), Some(1));
    let payload: String = store
        .connection
        .query_row(
            "SELECT payload_json FROM run_events WHERE event_kind='context_updated'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let payload: serde_json::Value = serde_json::from_str(&payload).unwrap();
    assert_eq!(payload["core_envelope"]["revision"], 7);
    assert_eq!(payload["core_envelope"]["session_id"], "actual-core");
}

#[test]
fn internal_and_maintenance_payloads_share_admission_rollback_and_reopen() {
    for maintenance in [false, true] {
        for fail_events in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("admission.db");
            let mut store = NolaStore::open_archive_mode(&path, false).unwrap();
            fixture(&mut store);
            terminal_sources::settle_fixture(&mut store, "conversation", "run", "run", "user");
            store
                .connection
                .execute_batch(
                    "CREATE TRIGGER reject_new_payload BEFORE INSERT ON run_invocation_payloads
            WHEN NEW.run_id='next' BEGIN SELECT RAISE(ABORT,'fixture next payload failure'); END;",
                )
                .unwrap();
            if fail_events {
                store
                    .connection
                    .execute_batch(
                        "DROP TRIGGER reject_new_payload;
                CREATE TRIGGER reject_new_payload BEFORE INSERT ON run_events
                WHEN NEW.run_id='next' AND NEW.event_kind='context_updated'
                BEGIN SELECT RAISE(ABORT,'fixture next payload failure'); END;",
                    )
                    .unwrap();
            }
            let initial_events = || {
                vec![
            nola_store_port::run_creation::InitialCoreRunEvent {
                core_session_id: "fixture-core".into(), core_turn_id: Some("next".into()), core_cursor: 1,
                core_revision: 3, event_kind: "turn_started".into(),
                payload_json: r#"{"type":"turn_started","session_id":"fixture-core","turn_id":"next"}"#.into(),
            },
            nola_store_port::run_creation::InitialCoreRunEvent {
                core_session_id: "fixture-core".into(), core_turn_id: Some("next".into()), core_cursor: 2,
                core_revision: 3, event_kind: "context_updated".into(),
                payload_json: r#"{"type":"context_updated","context_usage":{"used_tokens":3,"context_window_tokens":100,"prompt_token_limit":90,"usage_is_estimated":true}}"#.into(),
            },
        ]
            };
            let admit = |store: &mut NolaStore| -> StoreResult<()> {
                if maintenance {
                    store.begin_maintenance_run(&BeginMaintenanceRunInput {
                    initial_events: initial_events(),
                    conversation_id: "conversation".into(),
                    run_id: "next".into(),
                    context_checkpoint_id: None,
                    checkpoint_revision: 3,
                    checkpoint_event_cursor: 2,
                    checkpoint_json: r#"{"session_id":"fixture-core","state":{"session_id":"fixture-core"},"revision":3,"last_event_cursor":{"sequence":2}}"#.into(),
                    invocation_config_json: r#"{"model":"next"}"#.into(),
                    invocation_config_digest: "next-digest".into(),
                    outbox_effects: vec![OutboxEffectInput {
                        effect_id: "maintenance-effect".into(),
                        request_id: "maintenance-request".into(),
                        effect_kind: "compaction".into(),
                        write: crate::outbox_fixture::declare("maintenance-effect", "maintenance-request", serde_json::json!({"type":"compact","strategy":"summary","instructions":"","messages":[],"target_tokens":500,"context_window_tokens":128000})),
                        model_attempt_id: None,
                        delegated_invocation_id: None,
                    }],
                    model_attempt_mutations: vec![],
                    delegated_invocation_mutations: vec![],
                    delegated_permission_mutations: vec![],
                    memory_usage_entries: vec![],
                    created_at_ms: 3,
                })?;
                } else {
                    store.begin_internal_turn(&BeginInternalTurnInput {
                    initial_events: initial_events(),
                    conversation_id: "conversation".into(),
                    assistant_message_id: "assistant".into(),
                    expected_previous_run_id: "run".into(),
                    expected_checkpoint_revision: Some(2),
                    assistant_part_id: "next-part".into(),
                    assistant_turn_bindings: vec![],
                    initial_assistant_part_mutations: vec![],
                    run_id: "next".into(),
                    context_checkpoint_id: None,
                    checkpoint_revision: 3,
                    checkpoint_event_cursor: 2,
                    checkpoint_json: r#"{"session_id":"fixture-core","state":{"session_id":"fixture-core"},"revision":3,"last_event_cursor":{"sequence":2}}"#.into(),
                    invocation_config_json: r#"{"model":"next"}"#.into(),
                    invocation_config_digest: "next-digest".into(),
                    outbox_effects: vec![],
                    model_attempt_mutations: vec![],
                    delegated_invocation_mutations: vec![],
                    delegated_permission_mutations: vec![],
                    memory_usage_entries: vec![],
                    created_at_ms: 3,
                })?;
                }
                Ok(())
            };
            let error = admit(&mut store).unwrap_err().to_string();
            assert!(error.contains("fixture next payload failure"), "{error}");
            assert_eq!(
                store
                    .connection
                    .query_row("SELECT count(*) FROM runs WHERE id='next'", [], |r| r
                        .get::<_, i64>(0))
                    .unwrap(),
                0
            );
            assert_eq!(store.checkpoint_revision("conversation").unwrap(), Some(2));
            assert_eq!(
                store
                    .connection
                    .query_row(
                        "SELECT count(*) FROM run_events WHERE run_id='next'",
                        [],
                        |row| row.get::<_, i64>(0)
                    )
                    .unwrap(),
                0
            );
            assert_eq!(
                store
                    .connection
                    .query_row(
                        "SELECT run_id FROM messages WHERE id='assistant'",
                        [],
                        |r| r.get::<_, String>(0)
                    )
                    .unwrap(),
                "run"
            );
            store
                .connection
                .execute_batch("DROP TRIGGER reject_new_payload")
                .unwrap();
            admit(&mut store).unwrap();
            drop(store);
            let store = NolaStore::open_archive_mode(&path, false).unwrap();
            let facts: Vec<(String,String)> = store.connection.prepare("SELECT event_id,payload_json FROM run_events WHERE run_id='next' ORDER BY sequence").unwrap()
            .query_map([], |row| Ok((row.get(0)?,row.get(1)?))).unwrap().collect::<Result<_,_>>().unwrap();
            assert_eq!(facts.len(), 2);
            assert_eq!(facts[0].0, "core:next:1");
            assert_eq!(facts[1].0, "core:next:2");
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&facts[1].1).unwrap()["core_envelope"]
                    ["revision"],
                3
            );
            assert_eq!(
                store
                    .load_run_invocation_config("next")
                    .unwrap()
                    .unwrap()
                    .invocation_config_json,
                r#"{"model":"next"}"#
            );
            assert_eq!(
                store
                    .load_run_invocation_config("run")
                    .unwrap()
                    .unwrap()
                    .invocation_config_json,
                r#"{"model":"original"}"#
            );
            assert_eq!(
                store
                    .connection
                    .query_row("SELECT count(*) FROM run_invocation_payloads", [], |r| r
                        .get::<_, i64>(0))
                    .unwrap(),
                2
            );
        }
    }
}


#[test]
fn payload_insert_failure_rolls_back_entire_admission() {
    let directory = tempfile::tempdir().unwrap();
    let mut store = NolaStore::open(directory.path().join("rollback.db")).unwrap();
    store.connection.execute_batch("CREATE TRIGGER reject_payload BEFORE INSERT ON run_invocation_payloads BEGIN SELECT RAISE(ABORT,'fixture payload failure'); END;").unwrap();
    let error = store.begin_turn(&fixture_input()).unwrap_err().to_string();
    assert!(error.contains("fixture payload failure"), "{error}");
    for table in [
        "runs",
        "run_invocation_payloads",
        "messages",
        "message_parts",
        "core_checkpoints",
    ] {
        assert_eq!(
            store
                .connection
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0,
            "{table}"
        );
    }
    store
        .connection
        .execute_batch("DROP TRIGGER reject_payload")
        .unwrap();
    fixture(&mut store);
    assert!(exact(&store).unwrap().is_some());
}

#[test]
fn deleting_run_or_conversation_cascades_its_only_payload() {
    for direct_run in [true, false] {
        let directory = tempfile::tempdir().unwrap();
        let mut store = NolaStore::open(directory.path().join("delete.db")).unwrap();
        fixture(&mut store);
        if direct_run {
            // Archive replacement deletes Run execution state while retaining
            // the conversation. The child FK must cover that path too.
            store
                .connection
                .execute("DELETE FROM runs WHERE id='run'", [])
                .unwrap();
        } else {
            terminal_sources::settle_fixture(&mut store, "conversation", "run", "run", "user");
            assert!(store.delete_conversation("conversation").unwrap());
        }
        assert_eq!(
            store
                .connection
                .query_row("SELECT count(*) FROM run_invocation_payloads", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert!(store.load_run_invocation_config("run").unwrap().is_none());
    }
}

#[test]
fn exact_identity_only_returns_original_frozen_config_and_no_history() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("exact.sqlite");
    let mut store = NolaStore::open_archive_mode(&path, false).unwrap();
    fixture(&mut store);
    drop(store);
    let store = NolaStore::open_archive_mode(&path, false).unwrap();
    let record = exact(&store).unwrap().unwrap();
    assert_eq!(record.conversation_id, "conversation");
    assert_eq!(record.run_id, "run");
    assert_eq!(record.assistant_message_id, "assistant");
    assert_eq!(record.invocation_config_json, r#"{"model":"original"}"#);
    assert_eq!(record.invocation_config_digest, "original-digest");
    assert_eq!(record.status, "queued");
    assert_eq!(record.completed_at_ms, None);
    assert!(store
        .load_exact_run_invocation("wrong", "run", "assistant")
        .is_err());
    assert!(store
        .load_exact_run_invocation("conversation", "run", "user")
        .is_err());
    assert!(store
        .load_exact_run_invocation("conversation", "missing", "assistant")
        .unwrap()
        .is_none());
    assert!(store
        .load_exact_run_invocation("", "run", "assistant")
        .is_err());
}

#[test]
fn invalid_owner_remote_kind_and_missing_configuration_fail_closed() {
    let directory = tempfile::tempdir().unwrap();
    let mut store = NolaStore::open(directory.path().join("ownership.sqlite")).unwrap();
    fixture(&mut store);
    for mutation in [
        "UPDATE messages SET role='user' WHERE id='assistant'",
        "UPDATE messages SET run_id=NULL WHERE id='assistant'",
        "UPDATE runs SET kind='maintenance' WHERE id='run'",
        "DELETE FROM run_invocation_payloads WHERE run_id='run'",
        "UPDATE runs SET invocation_config_digest='' WHERE id='run'",
        "INSERT INTO remote_run_sources(run_id,source_host_id) VALUES('run','remote-host')",
    ] {
        store.connection.execute_batch("BEGIN IMMEDIATE").unwrap();
        store.connection.execute_batch(mutation).unwrap();
        assert!(exact(&store).is_err(), "{mutation}");
        if mutation.starts_with("DELETE FROM run_invocation_payloads") {
            assert!(
                store.load_run_invocation_config("run").is_err(),
                "missing local payload is not an unbound Run"
            );
        }
        assert!(!store.connection.is_autocommit());
        store.connection.execute_batch("ROLLBACK").unwrap();
        assert!(exact(&store).unwrap().is_some());
    }
}

#[test]
fn pinned_snapshot_cannot_mix_new_configuration_or_remote_owner_with_old_run() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("atomic.sqlite");
    let mut store = NolaStore::open_archive_mode(&path, false).unwrap();
    fixture(&mut store);
    let writer = Connection::open(&path).unwrap();
    let snapshot = read_snapshot::pin(&store.connection).unwrap();
    writer.execute_batch("BEGIN IMMEDIATE;
        UPDATE run_invocation_payloads SET config_json='{\"model\":\"new\"}' WHERE run_id='run';
        UPDATE runs SET invocation_config_digest='new-digest',status='sealed',completed_at_ms=2 WHERE id='run';
        INSERT INTO remote_run_sources(run_id,source_host_id) VALUES('run','remote-host');
        UPDATE core_checkpoints SET revision=2,event_cursor=7 WHERE conversation_id='conversation';
        UPDATE core_checkpoint_bodies SET checkpoint_json='{\"session_id\":\"new-core\"}'
            WHERE conversation_id='conversation';
        COMMIT;").unwrap();
    let record = exact(&store).unwrap().unwrap();
    assert_eq!(record.invocation_config_json, r#"{"model":"original"}"#);
    assert_eq!(record.invocation_config_digest, "original-digest");
    assert_eq!(record.status, "queued");
    assert_eq!(record.completed_at_ms, None);
    let checkpoint = record.core_checkpoint.unwrap().recovery.recovery.checkpoint;
    assert_eq!(checkpoint.conversation_id, "conversation");
    assert_eq!(checkpoint.revision, 1);
    assert_eq!(checkpoint.event_cursor, 0);
    assert_eq!(checkpoint.checkpoint_json, "{}");
    assert!(!store.connection.is_autocommit());
    drop(snapshot);
    assert!(exact(&store).is_err());
    assert!(store.connection.is_autocommit());
}
