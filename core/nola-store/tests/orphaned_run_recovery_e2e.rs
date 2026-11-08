use nola_store::{BeginTurnInput, NolaStore};
use rusqlite::{params, Connection};
use tempfile::tempdir;

fn begin_input(conversation_id: &str, run_id: &str, ordinal: i64) -> BeginTurnInput {
    BeginTurnInput {
        user_part_kind: "text".into(),
        expected_configuration_revision: None,
        expected_parent_message_id: None,
        initial_events: Vec::new(),
        initial_assistant_part_mutations: vec![],
        memory_usage_entries: vec![],
        delegated_invocation_mutations: vec![],
        delegated_permission_mutations: vec![],
        model_attempt_mutations: vec![],
        conversation_id: conversation_id.into(),
        conversation_title: format!("Conversation {ordinal}"),
        user_message_id: format!("{run_id}-user"),
        user_part_id: format!("{run_id}-user-part"),
        user_text: "continue the task".into(),
        user_content_json: None,
        additional_user_parts: Vec::new(),
        assistant_message_id: format!("{run_id}-assistant"),
        assistant_part_id: format!("{run_id}-assistant-part"),
        assistant_turn_bindings: vec![],
        run_id: run_id.into(),
        context_checkpoint_id: None,
        checkpoint_revision: 0,
        checkpoint_event_cursor: 0,
        checkpoint_json: "{}".into(),
        invocation_config_json: "{}".into(),
        invocation_config_digest: format!("config-{ordinal}"),
        outbox_effects: Vec::new(),
        created_at_ms: 1_700_000_000_000 + ordinal,
    }
}

fn canonical_statuses(connection: &Connection, run_id: &str) -> (String, String, String) {
    connection
        .query_row(
            "SELECT r.status, m.status, p.state
             FROM runs r
             JOIN messages m ON m.id = r.assistant_message_id
             JOIN message_parts p ON p.message_id = m.id
             WHERE r.id = ?1",
            params![run_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap()
}

use nola_store_port::store_maintenance::{RunRecoveryPlan, StoreMaintenanceRepository};

fn trace(db: &Connection, id: &str, conversation: &str, turn: &str) {
    db.execute("INSERT INTO trace_journal_runs(trace_id,run_id,conversation_id,turn_id,started_at,updated_at) VALUES(?1,?1,?2,?3,'before','before')",params![id,conversation,turn]).unwrap();
}
#[test]
fn recovery_only_retires_traces_and_never_fabricates_a_result_from_terminal_events() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("facts.db");
    let mut store = NolaStore::open(&path).unwrap();
    store.begin_turn(&begin_input("session", "run", 1)).unwrap();
    let db = Connection::open(&path).unwrap();
    db.execute(
        "INSERT INTO run_events VALUES('event','run',1,'turn_completed','{}',1)",
        [],
    )
    .unwrap();
    db.execute("INSERT INTO run_events VALUES('start','run',2,'turn_started','{\"turn_id\":\"successor\"}',2)",[]).unwrap();
    trace(&db, "preserved", "session", "successor");
    trace(&db, "retired", "session", "older");
    let revision: i64 = db
        .query_row("SELECT revision FROM conversations", [], |r| r.get(0))
        .unwrap();
    let count = store
        .with_run_recovery_plan(None, "restart", "after", 100, |facts| {
            assert_eq!(
                facts
                    .run
                    .as_ref()
                    .unwrap()
                    .latest_started_turn_id
                    .as_deref(),
                Some("successor")
            );
            assert_eq!(facts.checkpoint.as_ref().unwrap().checkpoint.checkpoint_json, "{}");
            assert_eq!(facts.traces.len(), 2);
            Ok(RunRecoveryPlan {
                release_session_lease: None,
                effect_delivery_attempt_mutations: vec![],
                preserve_all_traces: false,
                preserve_trace_turn_id: Some("successor".into()),
                delegated_permission_mutations: vec![],
            })
        })
        .unwrap();
    assert_eq!(count, 1);
    assert_eq!(
        canonical_statuses(&db, "run"),
        ("queued".into(), "queued".into(), "open".into())
    );
    assert_eq!(
        db.query_row("SELECT revision FROM conversations", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        revision
    );
    assert_eq!(
        db.query_row(
            "SELECT finished_at FROM trace_journal_runs WHERE trace_id='preserved'",
            [],
            |r| r.get::<_, Option<String>>(0)
        )
        .unwrap(),
        None
    );
    // Without an original result/seal the Run remains recoverable, not terminal.
    assert_eq!(
        store
            .with_run_recovery_plan(None, "restart", "later", 101, |facts| {
                assert!(facts.run.is_some());
                assert_eq!(facts.traces.len(), 1);
                Ok(RunRecoveryPlan {
                    preserve_all_traces: true,
                    ..Default::default()
                })
            })
            .unwrap(),
        0
    );
}

#[test]
fn scoped_lease_is_a_fact_and_other_connection_cannot_write_during_planning() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("race.db");
    let mut store = NolaStore::open(&path).unwrap();
    store.begin_turn(&begin_input("session", "run", 1)).unwrap();
    let db = Connection::open(&path).unwrap();
    db.execute("INSERT INTO session_run_leases(session_id,owner,pid,acquired_at_ms) VALUES('session','live',1,1)",[]).unwrap();
    // Fail immediately only for the competing write inside the recovery lock,
    // not for fixture setup while Store background persistence may still run.
    db.busy_timeout(std::time::Duration::ZERO).unwrap();
    assert_eq!(store.with_run_recovery_plan(Some("session"),"restart","after",100,|facts| {
        assert!(facts.has_session_lease);
        let error=db.execute("UPDATE core_checkpoint_bodies SET checkpoint_json='null'",[]).unwrap_err();
        assert!(matches!(error,rusqlite::Error::SqliteFailure(ref e,_) if e.code==rusqlite::ErrorCode::DatabaseBusy));
        Ok(RunRecoveryPlan {preserve_all_traces:true,..Default::default()})
    }).unwrap(),0);
    assert_eq!(canonical_statuses(&db, "run").0, "queued");
}

#[test]
fn failure_after_exact_lease_delete_rolls_back_lease_and_all_run_records() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("lease-delete-rollback.db");
    let mut store = NolaStore::open(&path).unwrap();
    let mut input = begin_input("session", "run", 1);
    input.outbox_effects.push(nola_store::OutboxEffectInput {
        delegated_invocation_id: None, model_attempt_id: None,
        effect_id: "effect".into(), request_id: "request".into(),
        effect_kind: "tool.execute".into(), write: outbox_fixture::tool("effect", "request", serde_json::json!({"tool":"read_file"})),
    });
    store.begin_turn(&input).unwrap();
    let db = Connection::open(&path).unwrap();
    trace(&db, "trace", "session", "turn");
    let lease = nola_store_port::workspace_ownership::StoredSessionRunLease {
        session_id: "session".into(), owner: "original-owner".into(), pid: 123,
        process_start: "original-process-start".into(), acquired_at_ms: 456,
    };
    db.execute("INSERT INTO session_run_leases(session_id,owner,pid,process_start,acquired_at_ms) VALUES(?1,?2,?3,?4,?5)",
        params![lease.session_id, lease.owner, lease.pid, lease.process_start, lease.acquired_at_ms]).unwrap();
    fn rows(db: &Connection, table: &str) -> Vec<Vec<rusqlite::types::Value>> {
        // Table names are fixed test constants below, never external input.
        let mut statement = db.prepare(&format!("SELECT * FROM {table} ORDER BY rowid")).unwrap();
        let columns = statement.column_count();
        let values = statement.query_map([], |row| (0..columns)
            .map(|index| row.get::<_, rusqlite::types::Value>(index)).collect())
            .unwrap().collect::<Result<Vec<_>, _>>().unwrap();
        values
    }
    let tables = ["session_run_leases", "runs", "messages", "message_parts",
        "trace_journal_runs", "run_events", "effect_outbox", "effect_delivery_attempts",
        "core_checkpoints", "conversations"];
    let before: Vec<_> = tables.iter().map(|table| rows(&db, table)).collect();
    db.execute_batch("CREATE TRIGGER reject_trace_recovery BEFORE UPDATE ON trace_journal_runs
        BEGIN SELECT RAISE(ABORT,'trace recovery fault'); END").unwrap();
    let error = store.with_run_recovery_plan(Some("session"), "restart", "after", 1_000, |facts| {
        assert_eq!(facts.session_lease.as_ref(), Some(&lease));
        assert_eq!(facts.effects.len(), 1);
        assert_eq!(facts.traces.len(), 1);
        Ok(RunRecoveryPlan {
            release_session_lease: Some(lease.clone()),
            // The trace write follows the exact lease DELETE and fails.
            ..Default::default()
        })
    }).unwrap_err();
    assert!(error.to_string().contains("trace recovery fault"), "{error}");
    for (table, expected) in tables.iter().zip(before) {
        assert_eq!(rows(&db, table), expected, "{table} changed despite transaction rollback");
    }
    let restored = db.query_row(
        "SELECT session_id,owner,pid,process_start,acquired_at_ms FROM session_run_leases WHERE session_id='session'", [],
        |row| Ok(nola_store_port::workspace_ownership::StoredSessionRunLease {
            session_id: row.get(0)?, owner: row.get(1)?, pid: row.get(2)?,
            process_start: row.get(3)?, acquired_at_ms: row.get(4)?,
        })).unwrap();
    assert_eq!(restored, lease, "all five original lease fields must survive rollback");
}

#[test]
fn callback_failure_rolls_back_previous_conversation_and_joins_outer_unit_of_work() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("rollback.db");
    let mut store = NolaStore::open(&path).unwrap();
    store.begin_turn(&begin_input("a", "a-run", 1)).unwrap();
    store.begin_turn(&begin_input("b", "b-run", 2)).unwrap();
    let db = Connection::open(&path).unwrap();
    trace(&db, "trace", "a", "turn");
    let mut calls = 0;
    let error = store
        .with_run_recovery_plan(None, "restart", "after", 100, |_| {
            calls += 1;
            if calls == 2 {
                Err("planner failed".into())
            } else {
                Ok(RunRecoveryPlan::default())
            }
        })
        .unwrap_err();
    assert!(error.to_string().contains("planner failed"));
    assert_eq!(calls, 2);
    assert_eq!(canonical_statuses(&db, "a-run").0, "queued");
    assert_eq!(
        db.query_row("SELECT finished_at FROM trace_journal_runs", [], |r| r
            .get::<_, Option<
            String,
        >>(
            0
        ))
        .unwrap(),
        None
    );
    store.begin_write_unit_of_work().unwrap();
    assert_eq!(
        store
            .with_run_recovery_plan(None, "restart", "after", 100, |_| Ok(RunRecoveryPlan::default()))
            .unwrap(),
        1
    );
    // Releasing the adapter's savepoint cannot publish an outer receipt UoW.
    assert_eq!(canonical_statuses(&db, "a-run").0, "queued");
    store.rollback_write_unit_of_work().unwrap();
    assert_eq!(canonical_statuses(&db, "a-run").0, "queued");
    assert_eq!(store
        .with_run_recovery_plan(None, "restart", "after", 100, |_| Ok(RunRecoveryPlan::default()))
        .unwrap(), 1);
    assert_eq!(canonical_statuses(&db, "a-run").0, "queued");
}

#[test]
fn trace_apply_failure_rolls_back_run_parts_and_revision() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("trace-fault.db");
    let mut store = NolaStore::open(&path).unwrap();
    store.begin_turn(&begin_input("session", "run", 1)).unwrap();
    let db = Connection::open(&path).unwrap();
    trace(&db, "trace", "session", "turn");
    let before = canonical_statuses(&db, "run");
    let revision: i64 = db
        .query_row("SELECT revision FROM conversations", [], |r| r.get(0))
        .unwrap();
    db.execute_batch("CREATE TRIGGER fail_recovery_trace BEFORE UPDATE ON trace_journal_runs BEGIN SELECT RAISE(ABORT,'trace fault'); END").unwrap();
    let error = store
        .with_run_recovery_plan(None, "restart", "after", 100, |_| Ok(RunRecoveryPlan::default()))
        .unwrap_err();
    assert!(error.to_string().contains("trace fault"));
    assert_eq!(canonical_statuses(&db, "run"), before);
    assert_eq!(
        db.query_row("SELECT revision FROM conversations", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        revision
    );
}

#[test]
fn malformed_started_event_is_not_mistaken_for_absent_initial_event() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("started.db");
    let mut store = NolaStore::open(&path).unwrap();
    store.begin_turn(&begin_input("session", "run", 1)).unwrap();
    let db = Connection::open(&path).unwrap();
    store
        .with_run_recovery_plan(None, "restart", "after", 100, |facts| {
            assert!(facts.run.as_ref().unwrap().latest_started_turn_id.is_none());
            Ok(RunRecoveryPlan::default())
        })
        .unwrap();
    for (sequence, raw) in ["{}", "not-json", "{\"turn_id\":3}", "{\"turn_id\":null}"]
        .iter()
        .enumerate()
    {
        db.execute(
            "INSERT INTO run_events VALUES(?1,'run',?2,'turn_started',?3,1)",
            params![format!("malformed-{sequence}"), sequence as i64 + 1, raw],
        )
        .unwrap();
        store
            .with_run_recovery_plan(None, "restart", "after", 100, |facts| {
                assert_eq!(
                    facts
                        .run
                        .as_ref()
                        .unwrap()
                        .latest_started_turn_id
                        .as_deref(),
                    Some("")
                );
                Ok(RunRecoveryPlan::default())
            })
            .unwrap();
    }
    assert_eq!(canonical_statuses(&db, "run").0, "queued");
}
#[path = "support/outbox.rs"]
mod outbox_fixture;
