use nola_store::{
    NolaStore, RenewRuntimeTaskLease, RuntimeTaskLeaseWitness, RuntimeTaskProgressMutation,
    StoredRuntimeTask,
};
use std::sync::{Arc, Barrier};
use std::thread;

#[test]
fn rejected_duplicate_task_releases_owned_transaction() {
    let root=tempfile::tempdir().unwrap();
    let mut store=NolaStore::open(root.path().join("duplicate-transaction.sqlite")).unwrap();
    store.create_runtime_task_with_event(&task("original")).unwrap();
    assert!(store.create_runtime_task_with_event(&task("original")).is_err());
    assert!(!store.has_write_unit_of_work());
    store.begin_write_unit_of_work().unwrap();
    store.create_runtime_task_with_event(&task("next")).unwrap();
    assert!(store.create_runtime_task_with_event(&task("original")).is_err());
    assert!(store.has_write_unit_of_work(),"nested rejection must preserve caller's transaction");
    store.commit_write_unit_of_work().unwrap();
    assert!(store.runtime_task("next").unwrap().is_some());
}

#[test]
#[cfg(unix)]
fn project_queries_share_admission_identity_after_reopen() {
    let root = tempfile::tempdir().unwrap();
    let actual = root.path().join("project");
    let alias = root.path().join("alias");
    let unrelated = root.path().join("unrelated");
    std::fs::create_dir(&actual).unwrap();
    std::fs::create_dir(&unrelated).unwrap();
    std::os::unix::fs::symlink(&actual, &alias).unwrap();
    let database = root.path().join("project-identity.sqlite");
    let mut store = NolaStore::open(&database).unwrap();
    let mut draft = task("alias-task");
    draft.project_root = alias.to_string_lossy().into_owned();
    store.create_runtime_task_with_event(&draft).unwrap();
    drop(store);

    let store = NolaStore::open(&database).unwrap();
    for query in [&actual, &alias] {
        let tasks = store.list_runtime_tasks(&query.to_string_lossy()).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].id, "alias-task");
        assert_eq!(tasks[0].owner_session_id.as_deref(), Some("parent-session"));
        assert_eq!(tasks[0].project_root, actual.canonicalize().unwrap().to_string_lossy());
    }
    assert!(store.list_runtime_tasks(&unrelated.to_string_lossy()).unwrap().is_empty());
}

#[test]
#[cfg(unix)]
fn writer_admission_treats_symlink_alias_as_the_same_project() {
    let root = tempfile::tempdir().unwrap();
    let actual = root.path().join("project");
    let alias = root.path().join("alias");
    std::fs::create_dir(&actual).unwrap();
    std::os::unix::fs::symlink(&actual, &alias).unwrap();
    let mut store = NolaStore::open(root.path().join("aliases.sqlite")).unwrap();
    let mut writer = task("first-writer");
    writer.kind = "agent".into();
    writer.metadata_json = r#"{"can_mutate":true}"#.into();
    writer.project_root = actual.to_string_lossy().into_owned();
    store.create_runtime_task_with_event(&writer).unwrap();
    writer.id = "second-writer".into();
    writer.project_root = alias.to_string_lossy().into_owned();
    assert!(store.create_runtime_task_with_event(&writer).is_err());
    assert!(store.runtime_task("second-writer").unwrap().is_none());
}

#[test]
fn concurrent_writer_admission_is_atomic_and_readers_are_independent() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("writer-admission.sqlite");
    NolaStore::open(&path).unwrap();
    let barrier = Arc::new(Barrier::new(2));
    let workers = (0..2).map(|index| {
        let path = path.clone();
        let barrier = barrier.clone();
        thread::spawn(move || {
            let mut store = NolaStore::open(path).unwrap();
            let mut draft = task(&format!("writer-{index}"));
            draft.kind = "agent".into();
            draft.metadata_json = r#"{"can_mutate":true}"#.into();
            barrier.wait();
            store.create_runtime_task_with_event(&draft).is_ok()
        })
    }).collect::<Vec<_>>();
    assert_eq!(workers.into_iter().filter_map(|worker| worker.join().ok()).filter(|ok| *ok).count(), 1);
    let mut store = NolaStore::open(&path).unwrap();
    let mut reader = task("reader");
    reader.kind = "agent".into();
    reader.metadata_json = r#"{"can_mutate":false}"#.into();
    store.create_runtime_task_with_event(&reader).unwrap();
    let mut other = task("other-project-writer");
    other.kind = "agent".into();
    other.project_root = "/other-project".into();
    other.metadata_json = r#"{"can_mutate":true}"#.into();
    store.create_runtime_task_with_event(&other).unwrap();
    let db = rusqlite::Connection::open(&path).unwrap();
    assert_eq!(db.query_row("SELECT COUNT(*) FROM durable_tasks WHERE id LIKE 'writer-%'", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
    assert_eq!(db.query_row("SELECT COUNT(*) FROM durable_task_events WHERE task_id LIKE 'writer-%'", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
}

fn task(id: &str) -> StoredRuntimeTask {
    // These Store tests exercise generic scheduler, lease, CAS and permission
    // continuation transactions, not Core agent admission or agent completion.
    // An agent must first be promoted by a real parent Run; inventing that
    // authority here would bypass the contract tested by Application E2Es.
    StoredRuntimeTask {
        id: id.into(),
        kind: "shell_process".into(),
        status: "queued".into(),
        title: "Durable analysis".into(),
        project_root: "/workspace".into(),
        owner_session_id: Some("parent-session".into()),
        parent_task_id: None,
        parent_tool_call_id: Some("tool-call".into()),
        child_session_id: Some("child-session".into()),
        agent_type: None,
        prompt: None,
        progress_summary: None,
        last_activity: None,
        result_summary: None,
        error: None,
        output_path: None,
        unread: false,
        cancel_requested: false,
        cancel_requested_at_ms: None,
        worker_id: None,
        ownership_token: None,
        lease_until_ms: None,
        heartbeat_at_ms: None,
        retry_count: 0,
        retry_at_ms: None,
        last_error: None,
        input_watermark: Some(7),
        completion_watermark: None,
        created_at_ms: 100,
        started_at_ms: None,
        ended_at_ms: None,
        updated_at_ms: 100,
        metadata_json: "{\"source\":\"e2e\"}".into(),
    }
}

#[test]
fn task_claim_rejects_a_clock_behind_persisted_frontier_without_writes() {
    for fair in [false, true] {
        for field in [
            "created_at_ms",
            "updated_at_ms",
            "started_at_ms",
            "heartbeat_at_ms",
        ] {
            let root = tempfile::tempdir().unwrap();
            let mut store = NolaStore::open(root.path().join("claim-clock.sqlite")).unwrap();
            let mut draft = task("future-frontier");
            draft.kind = "mcp".into();
            match field {
                "created_at_ms" => draft.created_at_ms = 500,
                "updated_at_ms" => draft.updated_at_ms = 500,
                "started_at_ms" => draft.started_at_ms = Some(500),
                "heartbeat_at_ms" => draft.heartbeat_at_ms = Some(500),
                _ => unreachable!(),
            }
            store.create_runtime_task_with_event(&draft).unwrap();
            let before = store.runtime_task(&draft.id).unwrap();
            let events = store.runtime_task_events(&draft.id, 100).unwrap();
            let claimed = if fair {
                store.claim_runtime_task_fair(
                    &draft.id,
                    "worker",
                    "generation",
                    "token",
                    400,
                    800,
                    4,
                    2,
                )
            } else {
                store.claim_runtime_task(&draft.id, "worker", "generation", "token", 400, 800)
            }
            .unwrap();
            assert!(claimed.is_none(), "{field}, fair={fair}");
            assert_eq!(store.runtime_task(&draft.id).unwrap(), before);
            assert_eq!(store.runtime_task_events(&draft.id, 100).unwrap(), events);
            assert!(store
                .runtime_task_current_attempt(&draft.id)
                .unwrap()
                .is_none());
            let ready = if fair {
                store.claim_runtime_task_fair(
                    &draft.id,
                    "worker",
                    "generation",
                    "token",
                    600,
                    900,
                    4,
                    2,
                )
            } else {
                store.claim_runtime_task(&draft.id, "worker", "generation", "token", 600, 900)
            }
            .unwrap();
            assert!(ready.is_some(), "a real later timestamp remains admissible");
        }
    }
}

#[test]
fn cold_recovery_rejects_backward_time_and_forward_expiry_never_revives_old_token() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("recovery-clock.sqlite");
    let mut store = NolaStore::open(&path).unwrap();
    let mut draft = task("clock-handoff");
    draft.kind = "mcp".into();
    store.create_runtime_task_with_event(&draft).unwrap();
    let original = store
        .claim_runtime_task(
            &draft.id,
            "old-worker",
            "old-generation",
            "old-token",
            600,
            900,
        )
        .unwrap()
        .unwrap();
    let original_task = store.runtime_task(&draft.id).unwrap();
    assert!(store
        .renew_runtime_task_lease(&renewal(&original, 599, 950))
        .unwrap()
        .is_none());
    assert_eq!(store.runtime_task(&draft.id).unwrap(), original_task);
    assert_eq!(
        store.runtime_task_attempt(&original.id).unwrap(),
        Some(original.clone())
    );
    // Exact, owned fault injection reproduces the failed real fixture: the
    // caller's expired lease projection is earlier than the committed claim.
    // Recovery must refuse it, not rewrite the Task or its actual history.
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute(
            "UPDATE durable_tasks SET lease_until_ms=550 WHERE id=?1",
            [&draft.id],
        )
        .unwrap();
    connection
        .execute(
            "UPDATE runtime_task_attempts SET lease_expires_at_ms=550 WHERE id=?1",
            [&original.id],
        )
        .unwrap();
    drop(store);
    let mut store = NolaStore::open(&path).unwrap();
    let before = store.runtime_task(&draft.id).unwrap();
    let attempt = store.runtime_task_attempt(&original.id).unwrap();
    let events = store.runtime_task_events(&draft.id, 100).unwrap();
    assert!(!store
        .recover_expired_runtime_task(&draft.id, "backward", 580, 580)
        .unwrap());
    assert!(!store
        .fail_expired_runtime_task_with_event(
            &draft.id,
            "backward",
            "reconciliation_required",
            "failed",
            None,
            "{}",
            580
        )
        .unwrap());
    assert_eq!(store.runtime_task(&draft.id).unwrap(), before);
    assert_eq!(store.runtime_task_attempt(&original.id).unwrap(), attempt);
    assert_eq!(store.runtime_task_events(&draft.id, 100).unwrap(), events);
    assert!(store
        .recover_expired_runtime_task(&draft.id, "actual later time", 1_000, 1_000)
        .unwrap());
    let successor = store
        .claim_runtime_task(
            &draft.id,
            "new-worker",
            "new-generation",
            "new-token",
            1_010,
            1_200,
        )
        .unwrap()
        .unwrap();
    let successor_task = store.runtime_task(&draft.id).unwrap();
    // A wall forward step beyond the absolute deadline cannot renew even the
    // correct token; the old generation also cannot borrow this successor.
    assert!(store
        .renew_runtime_task_lease(&renewal(&successor, 1_201, 1_500))
        .is_err());
    assert!(store
        .renew_runtime_task_lease(&renewal(&original, 1_020, 1_500))
        .is_err());
    assert_eq!(store.runtime_task(&draft.id).unwrap(), successor_task);
    assert_eq!(
        store.runtime_task_attempt(&successor.id).unwrap(),
        Some(successor)
    );
}

fn shell_dispatch_progress(store: &NolaStore, attempt: &nola_store::RuntimeTaskAttempt,
    phase: nola_domain::runtime_task_shell_delivery::ShellWorkerDispatchPhase,
) -> RuntimeTaskProgressMutation {
    use nola_domain::runtime_task_shell_delivery::{ShellWorkerDispatchWitness, SHELL_WORKER_DISPATCH_METADATA_KEY};
    let task = store.runtime_task(&attempt.task_id).unwrap().unwrap();
    let mut metadata: serde_json::Value = serde_json::from_str(&task.metadata_json).unwrap();
    metadata[SHELL_WORKER_DISPATCH_METADATA_KEY] = serde_json::to_value(
        ShellWorkerDispatchWitness::new(&attempt.task_id, &attempt.id, &attempt.worker_id,
            &attempt.worker_generation, &attempt.lease_token, phase).unwrap()).unwrap();
    RuntimeTaskProgressMutation {
        task_id: task.id, attempt_id: attempt.id.clone(), expected_attempt_revision: attempt.revision,
        worker_generation: attempt.worker_generation.clone(), lease_token: attempt.lease_token.clone(),
        current_expiry_ms: attempt.lease_expires_at_ms, expected_updated_at_ms: task.updated_at_ms,
        expected_metadata_json: task.metadata_json, metadata_json: metadata.to_string(),
        progress_summary: None, last_activity: None, activity: None, now_ms: 220,
    }
}

#[test]
fn delayed_observer_activity_is_attempt_fenced_and_event_failure_rolls_back_progress() {
    use nola_domain::runtime_task_shell_delivery::ShellWorkerDispatchPhase;
    let root=tempfile::tempdir().unwrap(); let path=root.path().join("late-observer.sqlite");
    let mut store=NolaStore::open(&path).unwrap();
    store.create_runtime_task_with_event(&task("observer")).unwrap();
    let first=store.claim_runtime_task("observer","worker-a","generation-a","token-a",200,400).unwrap().unwrap();
    // Freeze a real observer's original witness, then let a different owner
    // take over while the observer is still waiting on its read-only source.
    let mut late=shell_dispatch_progress(&store,&first,ShellWorkerDispatchPhase::Prepared);
    late.now_ms=280;
    late.progress_summary=Some("late A".into());
    late.activity=Some(nola_store_port::runtime_task::RuntimeTaskProgressActivity {
        summary:"late A".into(),detail_json:"{\"tool\":\"old-tool\"}".into()});
    assert!(store.retry_runtime_task(&RuntimeTaskLeaseWitness::from(&first),250,"handoff",240).unwrap());
    let second=store.claim_runtime_task("observer","worker-b","generation-b","token-b",260,600).unwrap().unwrap();
    let successor=store.runtime_task("observer").unwrap();
    let events=store.runtime_task_events("observer",100).unwrap();
    assert!(store.report_runtime_task_progress(&late).unwrap().is_none());
    assert_eq!(store.runtime_task("observer").unwrap(),successor);
    assert_eq!(store.runtime_task_attempt(&second.id).unwrap(),Some(second.clone()));
    assert_eq!(store.runtime_task_events("observer",100).unwrap(),events,
        "an in-flight old observer cannot append even one successor activity event");

    let mut current=shell_dispatch_progress(&store,&second,ShellWorkerDispatchPhase::Prepared);
    current.now_ms=280;
    current.progress_summary=Some("owner B".into());
    current.activity=Some(nola_store_port::runtime_task::RuntimeTaskProgressActivity {
        summary:"owner B".into(),detail_json:"{\"tool\":\"new-tool\"}".into()});
    let db=rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TRIGGER reject_activity BEFORE INSERT ON durable_task_events
        WHEN NEW.kind='activity' BEGIN SELECT RAISE(ABORT,'activity fault'); END;").unwrap();
    assert!(store.report_runtime_task_progress(&current).is_err());
    assert_eq!(store.runtime_task("observer").unwrap(),successor);
    assert_eq!(store.runtime_task_attempt(&second.id).unwrap(),Some(second));
    assert_eq!(store.runtime_task_events("observer",100).unwrap(),events);
    db.execute_batch("DROP TRIGGER reject_activity").unwrap();
    let committed=store.report_runtime_task_progress(&current).unwrap().unwrap();
    assert_eq!(committed.revision,current.expected_attempt_revision+1);
    let after=store.runtime_task_events("observer",100).unwrap();
    assert_eq!(after.len(),events.len()+1);
    assert_eq!(after[0].kind,"activity");
    assert_eq!(after[0].summary.as_deref(),Some("owner B"));
}

#[test]
fn shell_dispatch_prepared_is_claim_atomic_and_expired_retry_is_once() {
    use nola_domain::runtime_task_shell_delivery::*;
    let root = tempfile::tempdir().unwrap();
    let mut store = NolaStore::open(root.path().join("prepared.sqlite")).unwrap();
    store.create_runtime_task_with_event(&task("prepared")).unwrap();
    let attempt = store.claim_runtime_task("prepared", "worker", "generation", "secret-lease", 200, 300).unwrap().unwrap();
    let raw = store.runtime_task("prepared").unwrap().unwrap().metadata_json;
    let metadata: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let proof: ShellWorkerDispatchWitness = serde_json::from_value(metadata[SHELL_WORKER_DISPATCH_METADATA_KEY].clone()).unwrap();
    assert_eq!(proof.phase, ShellWorkerDispatchPhase::Prepared);
    assert!(proof.matches("prepared", &attempt.id, "worker", "generation", "secret-lease"));
    assert!(!raw.contains("secret-lease"));
    let stale = shell_dispatch_progress(&store, &attempt, ShellWorkerDispatchPhase::MayHaveExecuted);
    assert!(!store.recover_expired_runtime_task("prepared", "early", 250, 250).unwrap());
    assert!(store.recover_expired_runtime_task("prepared", "expired before dispatch", 400, 400).unwrap());
    assert!(!store.recover_expired_runtime_task("prepared", "duplicate", 400, 400).unwrap());
    assert_eq!(store.runtime_task("prepared").unwrap().unwrap().retry_count, 1);
    assert_eq!(store.runtime_task_attempt(&attempt.id).unwrap().unwrap().state, "superseded");
    let replacement = store.claim_runtime_task("prepared", "worker-2", "generation-2", "token-2", 400, 500).unwrap().unwrap();
    // The previous launcher's exact Prepared progress cannot admit its worker.
    assert!(store.report_runtime_task_progress(&stale).unwrap().is_none());
    assert_eq!(store.runtime_task_current_attempt("prepared").unwrap().unwrap(), replacement);
    let mut old_pid = task("old-pid");
    old_pid.metadata_json = serde_json::json!({"pid":987654}).to_string();
    store.create_runtime_task_with_event(&old_pid).unwrap();
    store.claim_runtime_task("old-pid", "worker", "generation", "other-token", 200, 300).unwrap().unwrap();
    assert!(store.recover_expired_runtime_task("old-pid", "this exact attempt was not dispatched", 400, 400).unwrap(),
        "stale process metadata is not admission or non-execution evidence for a new attempt");
}

#[test]
fn shell_dispatch_pid_zero_after_admission_cannot_replay_or_regress_metadata() {
    use nola_domain::runtime_task_shell_delivery::*;
    let root = tempfile::tempdir().unwrap();
    let mut store = NolaStore::open(root.path().join("admitted.sqlite")).unwrap();
    let mut draft = task("admitted");
    draft.metadata_json = serde_json::json!({"pid":0}).to_string();
    store.create_runtime_task_with_event(&draft).unwrap();
    let attempt = store.claim_runtime_task("admitted", "worker", "generation", "token", 200, 300).unwrap().unwrap();
    let mutation = shell_dispatch_progress(&store, &attempt, ShellWorkerDispatchPhase::MayHaveExecuted);
    let admitted = store.report_runtime_task_progress(&mutation).unwrap().unwrap();
    let before = store.runtime_task("admitted").unwrap().unwrap();
    let events = store.runtime_task_events_after_session_cursor("parent-session", 0, 100).unwrap();
    assert!(!store.recover_expired_runtime_task("admitted", "no PID is not proof", 400, 400).unwrap());
    assert_eq!(store.runtime_task("admitted").unwrap().unwrap(), before);
    assert_eq!(store.runtime_task_attempt(&attempt.id).unwrap().unwrap(), admitted);
    assert_eq!(store.runtime_task_events_after_session_cursor("parent-session", 0, 100).unwrap(), events);
    let backwards = shell_dispatch_progress(&store, &admitted, ShellWorkerDispatchPhase::Prepared);
    assert!(store.report_runtime_task_progress(&backwards).is_err());
    let mut erased = shell_dispatch_progress(&store, &admitted, ShellWorkerDispatchPhase::MayHaveExecuted);
    erased.metadata_json = "{}".into();
    assert!(store.report_runtime_task_progress(&erased).is_err());
    let mut replaced = shell_dispatch_progress(&store, &admitted, ShellWorkerDispatchPhase::MayHaveExecuted);
    let mut metadata: serde_json::Value = serde_json::from_str(&replaced.metadata_json).unwrap();
    metadata[SHELL_WORKER_DISPATCH_METADATA_KEY]["worker_generation"] = serde_json::json!("foreign");
    replaced.metadata_json = metadata.to_string();
    assert!(store.report_runtime_task_progress(&replaced).is_err());
    let final_observation = nola_store_port::runtime_task::RuntimeTaskFinalProgress {
        expected_updated_at_ms: before.updated_at_ms,
        expected_metadata_json: before.metadata_json.clone(), metadata_json: "{}".into(),
    };
    assert!(store.finish_runtime_task_detailed_with_event(
        &RuntimeTaskLeaseWitness::from(&admitted), "failed", None, None, None, Some("unknown"),
        None, true, "finished", "failed", None, "{}", Some(&final_observation), 230,
    ).is_err());
    assert_eq!(store.runtime_task("admitted").unwrap().unwrap(), before);
}

#[test]
fn shell_dispatch_claim_and_prepared_initialization_roll_back_together() {
    for fair in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("claim-rollback.sqlite");
        let mut store = NolaStore::open(&path).unwrap();
        store.create_runtime_task_with_event(&task("claim-rollback")).unwrap();
        let before = store.runtime_task("claim-rollback").unwrap().unwrap();
        let connection = rusqlite::Connection::open(&path).unwrap();
        connection.execute_batch("CREATE TRIGGER reject_shell_prepared BEFORE UPDATE OF metadata_json ON durable_tasks
            WHEN json_extract(NEW.metadata_json,'$._nola_shell_worker_dispatch.phase')='prepared'
            BEGIN SELECT RAISE(ABORT,'injected Prepared failure'); END;").unwrap();
        let result = if fair {
            store.claim_runtime_task_fair("claim-rollback", "worker", "generation", "token", 200, 300, 4, 2)
        } else {
            store.claim_runtime_task("claim-rollback", "worker", "generation", "token", 200, 300)
        };
        assert!(result.is_err());
        assert_eq!(store.runtime_task("claim-rollback").unwrap().unwrap(), before);
        assert!(store.runtime_task_current_attempt("claim-rollback").unwrap().is_none());
        let attempts: i64 = connection.query_row("SELECT COUNT(*) FROM runtime_task_attempts WHERE task_id='claim-rollback'", [], |row| row.get(0)).unwrap();
        assert_eq!(attempts, 0);
    }
}

#[test]
fn shell_dispatch_recovery_rejects_missing_foreign_and_corrupt_proofs_without_writes() {
    use nola_domain::runtime_task_shell_delivery::*;
    for case in ["missing", "attempt", "worker", "generation", "digest", "phase"] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("invalid.sqlite");
        let mut store = NolaStore::open(&path).unwrap();
        store.create_runtime_task_with_event(&task("invalid")).unwrap();
        let attempt = store.claim_runtime_task("invalid", "worker", "generation", "token", 200, 300).unwrap().unwrap();
        let mut metadata: serde_json::Value = serde_json::from_str(&store.runtime_task("invalid").unwrap().unwrap().metadata_json).unwrap();
        match case {
            "missing" => { metadata.as_object_mut().unwrap().remove(SHELL_WORKER_DISPATCH_METADATA_KEY); }
            "attempt" => metadata[SHELL_WORKER_DISPATCH_METADATA_KEY]["attempt_id"] = serde_json::json!("foreign"),
            "worker" => metadata[SHELL_WORKER_DISPATCH_METADATA_KEY]["worker_id"] = serde_json::json!("foreign"),
            "generation" => metadata[SHELL_WORKER_DISPATCH_METADATA_KEY]["worker_generation"] = serde_json::json!("foreign"),
            "digest" => metadata[SHELL_WORKER_DISPATCH_METADATA_KEY]["lease_token_digest"] = serde_json::json!("0".repeat(64)),
            "phase" => { metadata[SHELL_WORKER_DISPATCH_METADATA_KEY].as_object_mut().unwrap().remove("phase"); }
            _ => unreachable!(),
        }
        rusqlite::Connection::open(&path).unwrap().execute("UPDATE durable_tasks SET metadata_json=?1 WHERE id='invalid'", [metadata.to_string()]).unwrap();
        let before = store.runtime_task("invalid").unwrap().unwrap();
        assert!(!store.recover_expired_runtime_task("invalid", "unproven", 400, 400).unwrap(), "{case}");
        assert_eq!(store.runtime_task("invalid").unwrap().unwrap(), before, "{case}");
        assert_eq!(store.runtime_task_attempt(&attempt.id).unwrap().unwrap(), attempt, "{case}");
    }
}

#[test]
fn shell_dispatch_recovery_transaction_failure_retains_task_attempt_and_events() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("rollback.sqlite");
    let mut store = NolaStore::open(&path).unwrap();
    store.create_runtime_task_with_event(&task("rollback")).unwrap();
    let attempt = store.claim_runtime_task("rollback", "worker", "generation", "token", 200, 300).unwrap().unwrap();
    let before = store.runtime_task("rollback").unwrap().unwrap();
    let events = store.runtime_task_events_after_session_cursor("parent-session", 0, 100).unwrap();
    rusqlite::Connection::open(&path).unwrap().execute_batch(
        "CREATE TRIGGER fail_shell_recovery BEFORE INSERT ON durable_task_events WHEN NEW.kind='lease_recovered'
         BEGIN SELECT RAISE(ABORT,'injected recovery failure'); END;").unwrap();
    assert!(store.recover_expired_runtime_task("rollback", "fault", 400, 400).is_err());
    assert_eq!(store.runtime_task("rollback").unwrap().unwrap(), before);
    assert_eq!(store.runtime_task_attempt(&attempt.id).unwrap().unwrap(), attempt);
    assert_eq!(store.runtime_task_events_after_session_cursor("parent-session", 0, 100).unwrap(), events);
}

#[test]
fn current_attempt_is_exact_owned_lease_even_after_expiry_not_latest_history() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("current-attempt.sqlite");
    let mut store = NolaStore::open(&path).unwrap();
    store.create_runtime_task_with_event(&task("current")).unwrap();
    assert!(store.runtime_task_current_attempt("current").unwrap().is_none());
    let first = store.claim_runtime_task("current", "worker", "generation", "token", 200, 300).unwrap().unwrap();
    assert_eq!(store.runtime_task_current_attempt("current").unwrap(), Some(first.clone()));
    assert!(store.recover_expired_runtime_task("current", "expired", 400, 400).unwrap());
    assert!(store.runtime_task_current_attempt("current").unwrap().is_none());
    let second = store.claim_runtime_task("current", "worker-2", "generation-2", "token-2", 400, 500).unwrap().unwrap();
    assert_ne!(first.id, second.id);
    let reopened = NolaStore::open(&path).unwrap();
    assert_eq!(reopened.runtime_task_current_attempt("current").unwrap(), Some(second.clone()));
    // A corrupt current row cannot borrow a valid historical lease.
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute("UPDATE durable_tasks SET ownership_token='foreign' WHERE id='current'", []).unwrap();
    assert!(reopened.runtime_task_current_attempt("current").unwrap().is_none());
}

#[test]
fn legacy_permission_flag_without_continuation_cannot_block_fair_claim() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = NolaStore::open(temp.path().join("permission.sqlite")).unwrap();
    let mut stale = task("a-stale-permission");
    stale.status = "waiting_permission".into();
    stale.metadata_json = r#"{"_nola_permission_resolved":true}"#.into();
    store.create_runtime_task_with_event(&stale).unwrap();
    store.create_runtime_task_with_event(&task("z-ready")).unwrap();
    assert!(store.claim_runtime_task_fair(
        "z-ready", "worker", "generation", "token", 200, 1000, 4, 4,
    ).unwrap().is_some());
    assert_eq!(store.runtime_task("a-stale-permission").unwrap().unwrap().status, "waiting_permission");
}

#[test]
fn latest_permission_continuation_uses_attempt_order_not_random_id() {
    use nola_store_port::runtime_task::{SuspendRuntimeTaskForPermission, ResumeRuntimeTaskAfterPermissionFair};
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("same-clock.sqlite");
    let mut store = NolaStore::open(&path).unwrap();
    store.create_runtime_task_with_event(&task("same-clock")).unwrap();
    let mut attempt = store.claim_runtime_task("same-clock", "worker", "generation-1", "token-1", 200, 1000).unwrap().unwrap();
    for (index, id) in ["z-first", "a-second"].into_iter().enumerate() {
        let request_id = format!("permission-{index}");
        let mut suspension = store.suspend_runtime_task_for_permission(&SuspendRuntimeTaskForPermission {
            task_id: attempt.task_id.clone(), attempt_id: attempt.id.clone(),
            expected_attempt_revision: attempt.revision, worker_generation: attempt.worker_generation.clone(),
            lease_token: attempt.lease_token.clone(), current_expiry_ms: attempt.lease_expires_at_ms,
            permission_request: nola_store_port::permission::StoredPermissionRequest {
                closure: None,
                request_id: request_id.clone(), task_id: attempt.task_id.clone(), child_session_id: "child-session".into(),
                capability: "file.write".into(), input_json: "{}".into(), scope_rule: "file.write:/workspace".into(),
                status: "pending".into(), decision: None, custom_response: None, created_at_ms: 250,
                resolved_at_ms: None, core_permission_request_id: Some(request_id.clone()), permission_instructions_json: "[]".into(),
            },
            continuation_id: request_id.clone(),
            continuation_json: serde_json::json!({"child_session_id":"child-session", "run_id":"run", "core_permission_request_id":request_id}).to_string(),
            progress_summary: "permission".into(), last_activity: None, now_ms: 250,
        }).unwrap().unwrap();
        // Fix UUID ordering deterministically before a resume records its ID.
        notification_delivery_is_exact_independent_and_replayable_after_reopen(&path, "same-clock", &format!("permission:{request_id}"));
        if index == 1 {
            let old = nola_store_port::runtime_task::RuntimeTaskNotificationDelivery {
                owner_conversation_id: Some("parent-session".into()), task_id: "same-clock".into(),
                occurrence_id: "permission:permission-0".into(), consumer_id: "late-consumer".into(), command_id: "late".into(),
            };
            assert_eq!(store.acknowledge_runtime_task_delivery(&old).unwrap(),
                nola_store_port::runtime_task::RuntimeTaskNotificationAcknowledgement::Superseded);
            assert!(!store.runtime_task_notification_delivered(&old).unwrap());
        }
        let connection = rusqlite::Connection::open(&path).unwrap();
        connection.execute("UPDATE runtime_task_permission_suspensions SET id=?1 WHERE id=?2", [id, &suspension.id]).unwrap();
        suspension.id = id.into();
        assert_eq!(store.latest_runtime_task_permission_suspension("same-clock").unwrap().unwrap(), suspension);
        if index == 0 {
            connection.execute("UPDATE permission_requests SET status='resolved',decision='allow_once',resolved_at_ms=250 WHERE request_id=?1", [&request_id]).unwrap();
            attempt = store.resume_runtime_task_after_permission_fair(&ResumeRuntimeTaskAfterPermissionFair {
                task_id: attempt.task_id.clone(), suspension_id: suspension.id,
                expected_suspension_revision: suspension.revision, permission_request_id: request_id.clone(), continuation_id: request_id,
                worker_id: "worker".into(), worker_generation: "generation-2".into(), lease_token: "token-2".into(),
                now_ms: 250, lease_until_ms: 1000, max_running_global: 2, max_running_project: 2,
            }).unwrap().unwrap();
        }
    }
    drop(store);
    let store = NolaStore::open(&path).unwrap();
    assert_eq!(store.latest_runtime_task_permission_suspension("same-clock").unwrap().unwrap().id, "a-second");
}

#[test]
fn permission_continuation_survives_expiry_and_task_cancel_closes_it_atomically() {
    use nola_store_port::runtime_task::{SuspendRuntimeTaskForPermission, ResumeRuntimeTaskAfterPermissionFair};
    use nola_store_port::permission::StoredPermissionRequest;
    for mode in 0..5 {
    let explicit = mode == 1;
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("permission.sqlite");
    let mut store = NolaStore::open(&path).unwrap();
    store.create_runtime_task_with_event(&task("permission-task")).unwrap();
    let attempt = store.claim_runtime_task("permission-task", "worker", "generation", "token", 200, 1000).unwrap().unwrap();
    let suspension = store.suspend_runtime_task_for_permission(&SuspendRuntimeTaskForPermission {
        task_id: attempt.task_id.clone(), attempt_id: attempt.id.clone(),
        expected_attempt_revision: attempt.revision, worker_generation: attempt.worker_generation.clone(),
        lease_token: attempt.lease_token.clone(), current_expiry_ms: attempt.lease_expires_at_ms,
        permission_request: StoredPermissionRequest {
            closure: None,
            request_id: "permission".into(), task_id: attempt.task_id.clone(),
            child_session_id: "child-session".into(), capability: "file.write".into(),
            input_json: "{}".into(), scope_rule: "file.write:/workspace/output".into(),
            status: "pending".into(), decision: None, custom_response: None,
            created_at_ms: 250, resolved_at_ms: None,
            core_permission_request_id: Some("core-permission".into()),
            permission_instructions_json: "[]".into(),
        },
        continuation_id: "continuation".into(),
        continuation_json: r#"{"child_session_id":"child-session","core_permission_request_id":"core-permission","run_id":"run"}"#.into(),
        progress_summary: "Awaiting permission".into(), last_activity: None, now_ms: 250,
    }).unwrap().unwrap();
    drop(store);
    let mut store = NolaStore::open(&path).unwrap();
    assert!(!store.recover_expired_runtime_task("permission-task", "expired", 2100, 2000).unwrap());
    assert_eq!(store.runtime_task("permission-task").unwrap().unwrap().status, "waiting_permission");
    // A fault after updating the Task must roll back Task, request and continuation.
    let db = rusqlite::Connection::open(&path).unwrap();
    if mode >= 3 {
        // A bound Task records stop intent without pretending that its Run
        // or permission effect has already stopped. Test both decision orders.
        db.execute("INSERT INTO runtime_task_run_bindings
            (task_id,owner_session_id,child_session_id,run_id,admission_attempt_id,input_watermark,bound_at_ms)
            VALUES('permission-task','parent-session','child-session','run',?1,NULL,200)",
            [&attempt.id]).unwrap();
        let approve = || db.execute("UPDATE permission_requests SET status='resolved',
            decision='allow_once',resolved_at_ms=2100 WHERE request_id='permission'", []).unwrap();
        if mode == 3 { approve(); }
        assert!(store.request_runtime_task_cancel("permission-task", 2200).unwrap());
        if mode == 4 { approve(); }
        let requested = store.runtime_task("permission-task").unwrap().unwrap();
        assert_eq!(requested.status, "waiting_permission");
        assert!(requested.cancel_requested);
        assert!(store.runtime_task_terminal_receipt("permission-task").unwrap().is_none());
        let suspended_attempt = store.runtime_task_attempt(&attempt.id).unwrap();
        drop(store);
        let mut reopened = NolaStore::open(&path).unwrap();
        assert!(reopened.claim_runtime_task_fair("permission-task", "ordinary-worker", "ordinary-generation",
            "ordinary-token", 2300, 3000, 4, 4).unwrap().is_none());
        assert!(reopened.resume_runtime_task_after_permission_fair(&ResumeRuntimeTaskAfterPermissionFair {
            task_id: attempt.task_id.clone(), suspension_id: suspension.id.clone(),
            expected_suspension_revision: suspension.revision, permission_request_id: "permission".into(),
            continuation_id: "continuation".into(), worker_id: "new-worker".into(),
            worker_generation: "new-generation".into(), lease_token: "new-token".into(),
            now_ms: 2300, lease_until_ms: 3000, max_running_global: 4, max_running_project: 4,
        }).unwrap().is_none());
        assert_eq!(reopened.runtime_task("permission-task").unwrap().unwrap(), requested);
        assert_eq!(reopened.runtime_task_attempt(&attempt.id).unwrap(), suspended_attempt);
        assert_eq!(reopened.latest_runtime_task_permission_suspension("permission-task").unwrap().unwrap(), suspension);
        let decision: String = db.query_row("SELECT decision FROM permission_requests WHERE request_id='permission'",
            [], |row| row.get(0)).unwrap();
        assert_eq!(decision, "allow_once");
        continue;
    }
    // A stale legacy readiness flag cannot let a typed permission continuation
    // compete in the ordinary queue or starve genuinely queued work.
    db.execute("UPDATE durable_tasks SET metadata_json=json_set(metadata_json,'$._nola_permission_resolved',1) WHERE id='permission-task'", []).unwrap();
    store.create_runtime_task_with_event(&task("z-ready")).unwrap();
    assert!(store.claim_runtime_task_fair("z-ready", "ready-worker", "ready-generation", "ready-token", 2001, 3000, 4, 4).unwrap().is_some());
    if mode == 2 {
        let mut urgent = task("urgent");
        urgent.metadata_json = r#"{"_nola_scheduler":{"priority":"high","queued_at_ms":100}}"#.into();
        store.create_runtime_task_with_event(&urgent).unwrap();
        db.execute("UPDATE permission_requests SET status='resolved',decision='allow_once',resolved_at_ms=2050 WHERE request_id='permission'", []).unwrap();
        let resume = ResumeRuntimeTaskAfterPermissionFair {
            task_id: attempt.task_id.clone(), suspension_id: suspension.id.clone(),
            expected_suspension_revision: suspension.revision,
            permission_request_id: "permission".into(), continuation_id: "continuation".into(),
            worker_id: "new-worker".into(), worker_generation: "new-generation".into(), lease_token: "new-token".into(),
            now_ms: 2100, lease_until_ms: 3000, max_running_global: 4, max_running_project: 4,
        };
        assert!(store.resume_runtime_task_after_permission_fair(&resume).unwrap().is_none());
        assert!(store.claim_runtime_task_fair("urgent", "urgent-worker", "urgent-generation", "urgent-token", 2100, 3000, 4, 4).unwrap().is_some());
        let mut reused = resume.clone();
        reused.lease_token = attempt.lease_token.clone();
        assert!(store.resume_runtime_task_after_permission_fair(&reused).unwrap().is_none());
        let mut reused_generation = resume.clone();
        reused_generation.worker_generation = attempt.worker_generation.clone();
        assert!(store.resume_runtime_task_after_permission_fair(&reused_generation).unwrap().is_none());
        assert_eq!(store.pending_runtime_task_permission_suspension("permission-task").unwrap().unwrap(), suspension);
        let mut backwards = resume.clone();
        backwards.now_ms = 2049;
        assert!(store.resume_runtime_task_after_permission_fair(&backwards).unwrap().is_none());
        let successor = store.resume_runtime_task_after_permission_fair(&resume).unwrap().unwrap();
        assert_ne!(successor.id, attempt.id);
        assert_eq!(successor.attempt_number, attempt.attempt_number + 1);
        // Reconciliation returns the original committed grant even after a
        // renewal. It must never substitute the current lease snapshot.
        store.renew_runtime_task_lease(&renewal(&successor, 2150, 3100)).unwrap().unwrap();
        drop(store);
        let mut store = NolaStore::open(&path).unwrap();
        let exact = store.runtime_task_permission_resume("permission-task", &suspension.id, "permission", "continuation").unwrap().unwrap();
        assert_eq!(exact.granted_attempt, successor);
        assert_eq!(exact.suspension.state, "resumed");
        assert!(store.runtime_task_permission_resume("permission-task", &suspension.id, "permission", "foreign").is_err());
        assert!(store.resume_runtime_task_after_permission_fair(&resume).unwrap().is_none());
        assert!(!store.retry_runtime_task(&RuntimeTaskLeaseWitness::from(&attempt), 2400, "stale", 2200).unwrap());
        continue;
    }
    db.execute_batch("CREATE TRIGGER reject_permission_cancel BEFORE UPDATE ON runtime_task_permission_suspensions BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
    assert!(store.request_runtime_task_cancel("permission-task", 2100).is_err());
    assert_eq!(store.runtime_task("permission-task").unwrap().unwrap().status, "waiting_permission");
    let decision: Option<String> = db.query_row("SELECT decision FROM permission_requests WHERE request_id='permission'", [], |r| r.get(0)).unwrap();
    assert_eq!(decision, None);
    db.execute_batch("DROP TRIGGER reject_permission_cancel;").unwrap();
    db.execute_batch("CREATE TRIGGER reject_permission_closure BEFORE UPDATE ON permission_requests
        WHEN NEW.status='withdrawn' BEGIN SELECT RAISE(ABORT,'closure-fault'); END;").unwrap();
    assert!(store.request_runtime_task_cancel("permission-task", 2100).is_err());
    assert_eq!(store.runtime_task("permission-task").unwrap().unwrap().status, "waiting_permission");
    assert!(store.runtime_task_terminal_receipt("permission-task").unwrap().is_none());
    assert_eq!(store.latest_runtime_task_permission_suspension("permission-task").unwrap().unwrap(), suspension);
    assert!(store.permission_request("permission").unwrap().unwrap().closure.is_none());
    db.execute_batch("DROP TRIGGER reject_permission_closure;").unwrap();
    db.execute_batch("CREATE TRIGGER ignore_permission_closure BEFORE UPDATE ON permission_requests
        WHEN NEW.status='withdrawn' BEGIN SELECT RAISE(IGNORE); END;").unwrap();
    assert!(store.request_runtime_task_cancel("permission-task", 2100).is_err());
    assert!(store.runtime_task_terminal_receipt("permission-task").unwrap().is_none());
    assert_eq!(store.runtime_task("permission-task").unwrap().unwrap().status, "waiting_permission");
    assert!(store.permission_request("permission").unwrap().unwrap().closure.is_none());
    db.execute_batch("DROP TRIGGER ignore_permission_closure;").unwrap();
    if explicit {
        use nola_store_port::runtime_task::CancelRuntimeTaskPermissionSuspension;
        // Approval may win before cancellation, but must not make the
        // still-suspended task impossible to cancel or erase its decision.
        db.execute("UPDATE permission_requests SET status='resolved',decision='allow_once',resolved_at_ms=2150 WHERE request_id='permission'", []).unwrap();
        let input = CancelRuntimeTaskPermissionSuspension {
            task_id: "permission-task".into(), suspension_id: suspension.id.clone(),
            expected_suspension_revision: suspension.revision,
            permission_request_id: "permission".into(), continuation_id: "continuation".into(), requested_at_ms: 2200,
        };
        let mut foreign = input.clone();
        foreign.continuation_id = "other-continuation".into();
        assert!(store.cancel_runtime_task_permission_suspension(&foreign).unwrap().is_none());
        assert!(store.cancel_runtime_task_permission_suspension(&input).unwrap().is_some());
    } else {
        assert!(store.request_runtime_task_cancel("permission-task", 2200).unwrap());
    }
    let state: (String,Option<String>,String) = db.query_row("SELECT s.state,p.decision,a.state FROM runtime_task_permission_suspensions s JOIN permission_requests p ON p.request_id=s.permission_request_id JOIN runtime_task_attempts a ON a.id=s.attempt_id WHERE s.id=?1", [&suspension.id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(state, ("cancelled".into(), explicit.then(|| "allow_once".into()), "superseded".into()));
    let closed = store.permission_request("permission").unwrap().unwrap();
    let receipt = store.runtime_task_terminal_receipt("permission-task").unwrap().unwrap();
    assert_eq!(closed.status, "withdrawn");
    assert_eq!(closed.resolved_at_ms, explicit.then_some(2150));
    assert!(matches!(&closed.closure.as_ref().unwrap().source,
        nola_domain::permission_closure::PermissionClosureSource::UnboundTaskTerminal { terminal_receipt_id }
            if terminal_receipt_id == &receipt.receipt_id));
    let mut reopened = NolaStore::open(&path).unwrap();
    assert_eq!(reopened.permission_request("permission").unwrap().unwrap(), closed);
    assert!(reopened.pending_permission_for_task("permission-task").unwrap().is_none());
    assert!(matches!(reopened.resolve_permission_request("permission", "allow_once", None, 2300, None).unwrap(),
        nola_store_port::permission::StoredPermissionResolutionOutcome::Closed(request) if request == closed));
    assert!(!reopened.request_runtime_task_cancel("permission-task", 2300).unwrap());
    assert_eq!(reopened.runtime_task_terminal_receipt("permission-task").unwrap().unwrap(), receipt);
    assert!(store.resume_runtime_task_after_permission_fair(&ResumeRuntimeTaskAfterPermissionFair {
        task_id: attempt.task_id, suspension_id: suspension.id,
        expected_suspension_revision: suspension.revision,
        permission_request_id: "permission".into(), continuation_id: "continuation".into(),
        worker_id: "new-worker".into(), worker_generation: "new-generation".into(), lease_token: "new-token".into(),
        now_ms: 2300, lease_until_ms: 3000, max_running_global: 4, max_running_project: 4,
    }).unwrap().is_none());
    }
}

#[test]
fn uncertain_command_fences_retry_even_after_receipt_settlement_and_cancel() {
    use nola_store_port::protocol_journal::{BeginCommandReceiptInput, CompleteCommandReceiptOutcome};
    use nola_store_port::runtime_task::SuspendRuntimeTaskForCommandReconciliation;
    for settled in [false, true] {
        for cancelled in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let path = temp.path().join("reconciliation.sqlite");
            let mut store = NolaStore::open(&path).unwrap();
            store.create_runtime_task_with_event(&task("uncertain")).unwrap();
            let attempt = store.claim_runtime_task("uncertain", "worker", "generation", "token", 200, 1000).unwrap().unwrap();
            if settled {
                store.begin_command_receipt(&BeginCommandReceiptInput {
                    client_id: "client".into(), request_id: "request".into(),
                    command_type: "fixture".into(), command_json: "{}".into(),
                    expected_revision: None, received_at: "2026-10-06T00:00:00Z".into(),
                }).unwrap();
                store.complete_command_receipt("client", "request", "2026-10-06T00:00:01Z",
                    &CompleteCommandReceiptOutcome::Committed { committed_revision: None, result_json: "{}".into() }).unwrap();
            }
            if cancelled { store.request_runtime_task_cancel("uncertain", 220).unwrap(); }
            let input = SuspendRuntimeTaskForCommandReconciliation {
                lease: RuntimeTaskLeaseWitness::from(&attempt), client_id: "client".into(),
                request_id: "request".into(), failure_code: "command_transport_outcome_unknown".into(),
                progress_summary: None, now_ms: 250,
            };
            let mut forged = input.clone();
            forged.lease.worker_generation = "foreign".into();
            assert!(store.suspend_runtime_task_for_command_reconciliation(&forged).unwrap().is_none());
            let pending = store.suspend_runtime_task_for_command_reconciliation(&input).unwrap().unwrap();
            assert_eq!(pending.attempt_id, attempt.id);
            drop(store);
            let mut store = NolaStore::open(&path).unwrap();
            assert_eq!(store.pending_runtime_task_command_reconciliation("uncertain").unwrap(), Some(pending));
            assert!(store.claim_runtime_task("uncertain", "new-worker", "new-generation", "new-token", 2000, 3000).unwrap().is_none());
            assert!(!store.retry_runtime_task(&input.lease, 2500, "must not replay", 2000).unwrap());
        }
    }
}

#[test]
fn durable_task_survives_reopen_and_enforces_lease_ownership() {
    let temp = tempfile::tempdir().expect("tempdir");
    let path = temp.path().join("nola.sqlite");

    {
        let mut store = NolaStore::open(&path).expect("open");
        store
            .create_runtime_task_with_event(&task("task-1"))
            .expect("save");
        let first = store
            .append_runtime_task_event(
                "task-1",
                "queued",
                "queued",
                Some("Queued"),
                "{\"step\":1}",
                101,
            )
            .expect("event");
        assert_eq!(first, 2);
    }

    let mut worker_a = NolaStore::open(&path).expect("worker a");
    let mut worker_b = NolaStore::open(&path).expect("worker b");
    let attempt_a = worker_a
        .claim_runtime_task("task-1", "worker-a", "test-generation", "owner-a", 200, 300)
        .unwrap()
        .unwrap();
    assert!(!worker_b
        .claim_runtime_task("task-1", "worker-b", "test-generation", "owner-b", 200, 300)
        .unwrap()
        .is_some());
    let mut foreign = renewal(&attempt_a, 220, 320);
    foreign.lease_token = "owner-b".into();
    assert!(worker_b
        .renew_runtime_task_lease(&foreign)
        .unwrap()
        .is_none());
    let attempt_a = worker_a
        .renew_runtime_task_lease(&renewal(&attempt_a, 220, 320))
        .unwrap()
        .unwrap();
    assert_eq!(attempt_a.revision, 2);
    assert!(worker_a
        .retry_runtime_task(&RuntimeTaskLeaseWitness::from(&attempt_a), 400, "temporary", 230)
        .unwrap());
    assert!(!worker_b
        .claim_runtime_task("task-1", "worker-b", "test-generation", "owner-b", 399, 500)
        .unwrap()
        .is_some());
    let attempt_b = worker_b
        .claim_runtime_task("task-1", "worker-b", "test-generation", "owner-b", 400, 500)
        .unwrap()
        .expect("worker b claim");
    assert!(worker_b
        .finish_runtime_task_detailed_with_event(
            &RuntimeTaskLeaseWitness::from(&attempt_b),
            "completed",
            Some(7),
            Some("completed"),
            Some("Durable analysis completed"),
            None,
            None,
            true,
            "finished",
            "completed",
            Some("Durable analysis completed"),
            "{}",
            None,
            450,
        )
        .unwrap());

    let reopened = NolaStore::open(&path).expect("reopen");
    let finished = reopened.runtime_task("task-1").unwrap().expect("task");
    assert_eq!(finished.status, "completed");
    assert_eq!(finished.retry_count, 1);
    assert_eq!(finished.completion_watermark, Some(7));
    assert!(finished.ownership_token.is_none());
    let events = reopened
        .runtime_task_events_after_session_cursor("parent-session", 0, 10)
        .unwrap();
    assert_eq!(events.len(), 4);
    assert!(events.iter().any(|event| event.kind == "retry_scheduled"));
    assert!(events
        .iter()
        .any(|event| event.detail_json == "{\"step\":1}"));
    assert!(events.iter().any(|event| event.kind == "finished"));
}

#[test]
fn progress_cas_cannot_overwrite_a_renewed_or_cancelled_lease() {
    let temp = tempfile::tempdir().expect("tempdir");
    let path = temp.path().join("nola.sqlite");
    let mut store = NolaStore::open(&path).expect("open");
    store
        .create_runtime_task_with_event(&task("task-progress"))
        .unwrap();
    let attempt = store
        .claim_runtime_task(
            "task-progress",
            "worker",
            "test-generation",
            "lease",
            200,
            300,
        )
        .unwrap()
        .unwrap();
    let claimed = store.runtime_task("task-progress").unwrap().unwrap();
    let mutation = RuntimeTaskProgressMutation {
        task_id: claimed.id.clone(),
        attempt_id: attempt.id.clone(),
        expected_attempt_revision: attempt.revision,
        worker_generation: attempt.worker_generation.clone(),
        lease_token: attempt.lease_token.clone(),
        current_expiry_ms: attempt.lease_expires_at_ms,
        expected_updated_at_ms: claimed.updated_at_ms,
        expected_metadata_json: claimed.metadata_json.clone(),
        progress_summary: Some("first".into()),
        last_activity: Some("working".into()),
        activity: None,
        metadata_json: {
            let mut metadata: serde_json::Value = serde_json::from_str(&claimed.metadata_json).unwrap();
            metadata["progress"] = serde_json::json!(1);
            metadata.to_string()
        },
        now_ms: 220,
    };

    let mut expired = mutation.clone();
    expired.now_ms = expired.current_expiry_ms;
    assert!(store.report_runtime_task_progress(&expired).is_err());
    assert_eq!(
        store
            .runtime_task_attempt(&attempt.id)
            .unwrap()
            .unwrap()
            .revision,
        attempt.revision
    );

    let progressed_attempt = store
        .report_runtime_task_progress(&mutation)
        .unwrap()
        .unwrap();
    assert_eq!(progressed_attempt.revision, attempt.revision + 1);
    let progressed = store.runtime_task("task-progress").unwrap().unwrap();
    assert_eq!(progressed.progress_summary.as_deref(), Some("first"));
    assert_eq!(progressed.lease_until_ms, Some(300));

    assert!(store
        .renew_runtime_task_lease(&renewal(&progressed_attempt, 230, 330))
        .unwrap()
        .is_some());
    let mut stale = mutation.clone();
    stale.progress_summary = Some("stale overwrite".into());
    stale.now_ms = 240;
    assert!(store
        .report_runtime_task_progress(&stale)
        .unwrap()
        .is_none());
    let renewed = store.runtime_task("task-progress").unwrap().unwrap();
    assert_eq!(renewed.progress_summary.as_deref(), Some("first"));
    assert_eq!(renewed.lease_until_ms, Some(330));
    assert_eq!(renewed.heartbeat_at_ms, Some(230));

    let current = RuntimeTaskProgressMutation {
        attempt_id: progressed_attempt.id.clone(),
        expected_attempt_revision: progressed_attempt.revision + 1,
        worker_generation: progressed_attempt.worker_generation.clone(),
        lease_token: progressed_attempt.lease_token.clone(),
        current_expiry_ms: 330,
        expected_updated_at_ms: renewed.updated_at_ms,
        expected_metadata_json: renewed.metadata_json.clone(),
        now_ms: 250,
        ..stale
    };
    assert!(store
        .request_runtime_task_cancel("task-progress", 245)
        .unwrap());
    assert!(store
        .report_runtime_task_progress(&current)
        .unwrap()
        .is_none());
    let cancelled = store.runtime_task("task-progress").unwrap().unwrap();
    assert!(cancelled.cancel_requested);
    assert_eq!(
        cancelled.progress_summary.as_deref(),
        Some("cancellation_requested")
    );
    assert_eq!(cancelled.lease_until_ms, Some(330));
}

#[test]
fn reclaimed_task_requires_a_fresh_token_and_attempt() {
    let temp = tempfile::tempdir().expect("tempdir");
    let path = temp.path().join("nola.sqlite");
    let mut store = NolaStore::open(&path).expect("open");
    store
        .create_runtime_task_with_event(&task("task-aba"))
        .unwrap();
    let first = store
        .claim_runtime_task("task-aba", "worker", "generation-1", "same-token", 200, 300)
        .unwrap()
        .unwrap();
    assert_eq!(first.attempt_number, 1);
    assert_eq!(first.revision, 1);
    assert!(store
        .retry_runtime_task(&RuntimeTaskLeaseWitness::from(&first), 310, "retry", 250)
        .unwrap());
    assert!(matches!(store.claim_runtime_task("task-aba", "worker", "generation-2", "same-token", 310, 400),
        Err(nola_store::StoreError::Conflict(_))));
    let second = store
        .claim_runtime_task("task-aba", "worker", "generation-2", "fresh-token", 310, 400)
        .unwrap()
        .unwrap();
    assert_ne!(second.id, first.id);
    assert_eq!(second.attempt_number, 2);
    assert_eq!(second.worker_generation, "generation-2");
    let stale = store.runtime_task_attempt(&first.id).unwrap().unwrap();
    assert_eq!(stale.state, "superseded");
    assert_eq!(stale.revision, 2);
    let active = store.runtime_task_attempt(&second.id).unwrap().unwrap();
    assert_eq!(active.state, "leased");
    assert_eq!(active.revision, 1);
    assert!(store
        .renew_runtime_task_lease(&renewal(&first, 290, 410))
        .unwrap()
        .is_none());
}

#[test]
fn lease_renewal_is_single_winner_and_an_expired_attempt_cannot_be_revived() {
    let temp = tempfile::tempdir().expect("tempdir");
    let path = temp.path().join("nola.sqlite");
    let mut seed = NolaStore::open(&path).unwrap();
    seed.create_runtime_task_with_event(&task("task-renew-race"))
        .unwrap();
    let attempt = seed
        .claim_runtime_task("task-renew-race", "worker", "generation", "token", 100, 200)
        .unwrap()
        .unwrap();
    drop(seed);
    let barrier = Arc::new(Barrier::new(3));
    let mut workers = Vec::new();
    for expiry in [300, 301] {
        let path = path.clone();
        let barrier = Arc::clone(&barrier);
        let renewal = renewal(&attempt, 150, expiry);
        workers.push(thread::spawn(move || {
            let mut store = NolaStore::open(path).unwrap();
            barrier.wait();
            store.renew_runtime_task_lease(&renewal).unwrap()
        }));
    }
    barrier.wait();
    let winners = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .filter(Option::is_some)
        .count();
    assert_eq!(winners, 1);
    let mut store = NolaStore::open(&path).unwrap();
    let renewed = store.runtime_task_attempt(&attempt.id).unwrap().unwrap();
    assert_eq!(renewed.revision, 2);
    let expired = renewal(
        &renewed,
        renewed.lease_expires_at_ms,
        renewed.lease_expires_at_ms + 100,
    );
    assert!(store.renew_runtime_task_lease(&expired).is_err());
    assert_eq!(
        store.runtime_task_attempt(&attempt.id).unwrap().unwrap(),
        renewed
    );
}

#[test]
fn renewal_and_progress_share_one_attempt_revision_cas() {
    let temp = tempfile::tempdir().expect("tempdir");
    let path = temp.path().join("nola.sqlite");
    let mut seed = NolaStore::open(&path).unwrap();
    seed.create_runtime_task_with_event(&task("task-renew-progress-race"))
        .unwrap();
    let attempt = seed
        .claim_runtime_task(
            "task-renew-progress-race",
            "worker",
            "generation",
            "token",
            100,
            200,
        )
        .unwrap()
        .unwrap();
    let task = seed
        .runtime_task("task-renew-progress-race")
        .unwrap()
        .unwrap();
    drop(seed);

    let progress = RuntimeTaskProgressMutation {
        task_id: task.id.clone(),
        attempt_id: attempt.id.clone(),
        expected_attempt_revision: attempt.revision,
        worker_generation: attempt.worker_generation.clone(),
        lease_token: attempt.lease_token.clone(),
        current_expiry_ms: attempt.lease_expires_at_ms,
        expected_updated_at_ms: task.updated_at_ms,
        expected_metadata_json: task.metadata_json.clone(),
        progress_summary: Some("progress won".into()),
        last_activity: Some("working".into()),
        activity: None,
        metadata_json: task.metadata_json.clone(),
        now_ms: 150,
    };
    let renew = renewal(&attempt, 150, 300);
    let barrier = Arc::new(Barrier::new(3));
    let progress_worker = {
        let path = path.clone();
        let barrier = Arc::clone(&barrier);
        thread::spawn(move || {
            let mut store = NolaStore::open(path).unwrap();
            barrier.wait();
            store
                .report_runtime_task_progress(&progress)
                .unwrap()
                .is_some()
        })
    };
    let renew_worker = {
        let path = path.clone();
        let barrier = Arc::clone(&barrier);
        thread::spawn(move || {
            let mut store = NolaStore::open(path).unwrap();
            barrier.wait();
            store.renew_runtime_task_lease(&renew).unwrap().is_some()
        })
    };
    barrier.wait();
    let progress_won = progress_worker.join().unwrap();
    let renew_won = renew_worker.join().unwrap();
    assert_ne!(progress_won, renew_won, "exactly one revision CAS must win");
    let store = NolaStore::open(&path).unwrap();
    let final_attempt = store.runtime_task_attempt(&attempt.id).unwrap().unwrap();
    let final_task = store.runtime_task(&task.id).unwrap().unwrap();
    assert_eq!(final_attempt.revision, attempt.revision + 1);
    assert_eq!(
        final_task.progress_summary.as_deref() == Some("progress won"),
        progress_won
    );
    assert_eq!(final_task.lease_until_ms == Some(300), renew_won);
}

#[test]
fn expired_task_is_recovered_once_and_can_be_reclaimed() {
    let temp = tempfile::tempdir().expect("tempdir");
    let path = temp.path().join("nola.sqlite");
    let mut store = NolaStore::open(&path).expect("open");
    store
        .create_runtime_task_with_event(&task("task-expired"))
        .expect("save");
    assert!(store
        .claim_runtime_task(
            "task-expired",
            "worker-a",
            "test-generation",
            "owner-a",
            500,
            600
        )
        .unwrap()
        .is_some());
    assert!(!store
        .recover_expired_runtime_task("task-expired", "restart", 610, 599)
        .unwrap());
    assert!(store
        .recover_expired_runtime_task("task-expired", "restart", 610, 600)
        .unwrap());
    assert!(!store
        .recover_expired_runtime_task("task-expired", "again", 620, 600)
        .unwrap());
    assert!(!store
        .claim_runtime_task(
            "task-expired",
            "worker-b",
            "test-generation",
            "owner-b",
            609,
            700
        )
        .unwrap()
        .is_some());
    assert!(store
        .claim_runtime_task(
            "task-expired",
            "worker-b",
            "test-generation",
            "owner-b",
            610,
            700
        )
        .unwrap()
        .is_some());

    drop(store);
    let reopened = NolaStore::open(&path).expect("reopen");
    let recovered = reopened.runtime_task("task-expired").unwrap().unwrap();
    assert_eq!(recovered.status, "running");
    assert_eq!(recovered.retry_count, 1);
    assert_eq!(recovered.ownership_token.as_deref(), Some("owner-b"));
}

#[test]
fn fair_claim_is_fifo_within_priority_and_enforces_global_slots_atomically() {
    let temp = tempfile::tempdir().expect("tempdir");
    let path = temp.path().join("nola.sqlite");
    let mut store = NolaStore::open(&path).expect("open");
    let mut low = task("task-low");
    low.created_at_ms = 1;
    low.updated_at_ms = 1;
    low.metadata_json = r#"{"_nola_scheduler":{"priority":"low","queued_at_ms":1,"resource_budget":{},"output_bytes":0,"budget_exhaustion":null}}"#.into();
    let mut high_old = task("task-high-old");
    high_old.created_at_ms = 2;
    high_old.updated_at_ms = 2;
    high_old.metadata_json = r#"{"_nola_scheduler":{"priority":"high","queued_at_ms":2,"resource_budget":{},"output_bytes":0,"budget_exhaustion":null}}"#.into();
    let mut high_new = task("task-high-new");
    high_new.created_at_ms = 3;
    high_new.updated_at_ms = 3;
    high_new.metadata_json = r#"{"_nola_scheduler":{"priority":"high","queued_at_ms":3,"resource_budget":{},"output_bytes":0,"budget_exhaustion":null}}"#.into();
    store.create_runtime_task_with_event(&low).unwrap();
    store.create_runtime_task_with_event(&high_new).unwrap();
    store.create_runtime_task_with_event(&high_old).unwrap();

    assert!(!store
        .claim_runtime_task_fair("task-low", "w", "test-generation", "l", 10, 100, 1, 1)
        .unwrap()
        .is_some());
    assert!(!store
        .claim_runtime_task_fair("task-high-new", "w", "test-generation", "n", 10, 100, 1, 1)
        .unwrap()
        .is_some());
    let old_attempt = store
        .claim_runtime_task_fair("task-high-old", "w", "test-generation", "o", 10, 100, 1, 1)
        .unwrap()
        .expect("old high-priority claim");
    assert!(!store
        .claim_runtime_task_fair("task-high-new", "w2", "test-generation", "n", 11, 101, 1, 1)
        .unwrap()
        .is_some());
    assert!(store
        .finish_runtime_task_detailed_with_event(
            &RuntimeTaskLeaseWitness::from(&old_attempt),
            "completed",
            high_old.input_watermark,
            Some("completed"),
            None,
            None,
            None,
            false,
            "slot_released",
            "completed",
            None,
            "{}",
            None,
            12,
        )
        .unwrap());
    assert!(store
        .claim_runtime_task_fair("task-high-new", "w2", "test-generation", "n", 13, 103, 1, 1)
        .unwrap()
        .is_some());
    assert!(store
        .request_runtime_task_cancel("task-high-new", 14)
        .unwrap());
    let connection = rusqlite::Connection::open(&path).unwrap();
    let attempt_json: String = connection
        .query_row(
            "SELECT json_object('id',id,'task_id',task_id,'attempt_number',attempt_number,
             'worker_id',worker_id,'worker_generation',worker_generation,'lease_token',lease_token,
             'lease_expires_at_ms',lease_expires_at_ms,'state',state,'revision',revision,
             'claimed_at_ms',claimed_at_ms) FROM runtime_task_attempts
             WHERE task_id='task-high-new' AND state='leased'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let attempt: nola_store::RuntimeTaskAttempt = serde_json::from_str(&attempt_json).unwrap();
    assert!(store
        .renew_runtime_task_lease(&renewal(&attempt, 15, 105))
        .unwrap()
        .is_none());
}

fn renewal(
    attempt: &nola_store::RuntimeTaskAttempt,
    now_ms: i64,
    requested_expiry_ms: i64,
) -> RenewRuntimeTaskLease {
    RenewRuntimeTaskLease {
        task_id: attempt.task_id.clone(),
        attempt_id: attempt.id.clone(),
        expected_attempt_revision: attempt.revision,
        worker_generation: attempt.worker_generation.clone(),
        lease_token: attempt.lease_token.clone(),
        current_expiry_ms: attempt.lease_expires_at_ms,
        now_ms,
        requested_expiry_ms,
    }
}

#[test]
fn fair_claim_selects_one_global_project_head_and_ages_waiting_priority() {
    let temp = tempfile::tempdir().expect("tempdir");
    let path = temp.path().join("nola.sqlite");
    let mut store = NolaStore::open(&path).expect("open");

    let mut old_low = task("project-a-old-low");
    old_low.project_root = "/workspace/a".into();
    old_low.created_at_ms = 1;
    old_low.updated_at_ms = 1;
    old_low.metadata_json = r#"{"_nola_scheduler":{"priority":"low","queued_at_ms":1,"resource_budget":{},"output_bytes":0,"budget_exhaustion":null}}"#.into();

    let mut new_high = task("project-b-new-high");
    new_high.project_root = "/workspace/b".into();
    new_high.created_at_ms = 120_001;
    new_high.updated_at_ms = 120_001;
    new_high.metadata_json = r#"{"_nola_scheduler":{"priority":"high","queued_at_ms":120001,"resource_budget":{},"output_bytes":0,"budget_exhaustion":null}}"#.into();

    store.create_runtime_task_with_event(&old_low).unwrap();
    store.create_runtime_task_with_event(&new_high).unwrap();

    assert!(
        !store
            .claim_runtime_task_fair(
                "project-b-new-high",
                "worker-b",
                "test-generation",
                "owner-b",
                120_001,
                180_001,
                1,
                1,
            )
            .unwrap()
            .is_some(),
        "a caller from a newer project bypassed the globally aged project head"
    );
    assert!(store
        .claim_runtime_task_fair(
            "project-a-old-low",
            "worker-a",
            "test-generation",
            "owner-a",
            120_001,
            180_001,
            1,
            1,
        )
        .unwrap()
        .is_some());
}

#[test]
fn fair_claim_skips_a_project_that_already_uses_its_project_slot() {
    let temp = tempfile::tempdir().expect("tempdir");
    let path = temp.path().join("nola.sqlite");
    let mut store = NolaStore::open(&path).expect("open");

    let mut running_a = task("project-a-running");
    running_a.project_root = "/workspace/a".into();
    store.create_runtime_task_with_event(&running_a).unwrap();
    assert!(store
        .claim_runtime_task(
            "project-a-running",
            "worker-running",
            "test-generation",
            "owner-running",
            100,
            10_000,
        )
        .unwrap()
        .is_some());

    let mut queued_a = task("project-a-queued");
    queued_a.project_root = "/workspace/a".into();
    queued_a.created_at_ms = 101;
    queued_a.updated_at_ms = 101;
    queued_a.metadata_json = r#"{"_nola_scheduler":{"priority":"high","queued_at_ms":101,"resource_budget":{},"output_bytes":0,"budget_exhaustion":null}}"#.into();
    let mut queued_b = task("project-b-queued");
    queued_b.project_root = "/workspace/b".into();
    queued_b.created_at_ms = 102;
    queued_b.updated_at_ms = 102;
    queued_b.metadata_json = r#"{"_nola_scheduler":{"priority":"normal","queued_at_ms":102,"resource_budget":{},"output_bytes":0,"budget_exhaustion":null}}"#.into();
    store.create_runtime_task_with_event(&queued_a).unwrap();
    store.create_runtime_task_with_event(&queued_b).unwrap();

    assert!(store
        .claim_runtime_task_fair(
            "project-b-queued",
            "worker-b",
            "test-generation",
            "owner-b",
            200,
            10_000,
            2,
            1,
        )
        .unwrap()
        .is_some());
    assert!(!store
        .claim_runtime_task_fair(
            "project-a-queued",
            "worker-a",
            "test-generation",
            "owner-a",
            200,
            10_000,
            3,
            1,
        )
        .unwrap()
        .is_some());
}

#[test]
fn permission_wait_releases_the_slot_and_resumes_through_the_fair_queue() {
    use nola_store_port::runtime_task::{SuspendRuntimeTaskForPermission, ResumeRuntimeTaskAfterPermissionFair};
    let temp = tempfile::tempdir().expect("tempdir");
    let path = temp.path().join("nola.sqlite");
    let mut store = NolaStore::open(&path).expect("open");

    let mut waiting = task("waiting-agent");
    waiting.created_at_ms = 1;
    waiting.updated_at_ms = 1;
    waiting.project_root = "/workspace/a".into();
    waiting.metadata_json = r#"{"_nola_scheduler":{"priority":"normal","queued_at_ms":1,"resource_budget":{},"output_bytes":0,"budget_exhaustion":null}}"#.into();
    let mut runnable = task("runnable-shell");
    runnable.kind = "shell_process".into();
    runnable.project_root = "/workspace/b".into();
    runnable.created_at_ms = 2;
    runnable.updated_at_ms = 2;
    runnable.metadata_json = r#"{"_nola_scheduler":{"priority":"normal","queued_at_ms":2,"resource_budget":{},"output_bytes":0,"budget_exhaustion":null}}"#.into();
    store.create_runtime_task_with_event(&waiting).unwrap();
    store.create_runtime_task_with_event(&runnable).unwrap();
    let waiting_attempt = store
        .claim_runtime_task_fair(
            "waiting-agent",
            "agent-worker",
            "test-generation",
            "agent-owner",
            10,
            1_000,
            1,
            1,
        )
        .unwrap()
        .expect("waiting Agent claim");

    let suspension = store.suspend_runtime_task_for_permission(&SuspendRuntimeTaskForPermission {
        task_id: waiting_attempt.task_id.clone(), attempt_id: waiting_attempt.id.clone(),
        expected_attempt_revision: waiting_attempt.revision, worker_generation: waiting_attempt.worker_generation.clone(),
        lease_token: waiting_attempt.lease_token.clone(), current_expiry_ms: waiting_attempt.lease_expires_at_ms,
        permission_request: nola_store_port::permission::StoredPermissionRequest {
            closure: None,
            request_id: "waiting-permission".into(), task_id: "waiting-agent".into(),
            child_session_id: "child-session".into(), capability: "file.write".into(),
            input_json: "{}".into(), scope_rule: "file.write:/workspace/a/output".into(),
            status: "pending".into(), decision: None, custom_response: None,
            created_at_ms: 11, resolved_at_ms: None, core_permission_request_id: Some("core-permission".into()),
            permission_instructions_json: "[]".into(),
        },
        continuation_id: "waiting-continuation".into(),
        continuation_json: r#"{"child_session_id":"child-session","core_permission_request_id":"core-permission","run_id":"waiting-run"}"#.into(),
        progress_summary: "waiting for permission".into(), last_activity: None, now_ms: 11,
    }).unwrap().unwrap();

    let shell_attempt = store
            .claim_runtime_task_fair(
                "runnable-shell",
                "shell-worker",
                "test-generation",
                "shell-owner",
                20,
                1_000,
                1,
                1
            )
            .unwrap()
            .expect("an unresolved permission wait must not occupy an execution slot");

    rusqlite::Connection::open(&path).unwrap().execute(
        "UPDATE permission_requests SET status='resolved',decision='allow_once',resolved_at_ms=21 WHERE request_id='waiting-permission'", [],
    ).unwrap();
    let mut resume = ResumeRuntimeTaskAfterPermissionFair {
        task_id: waiting_attempt.task_id.clone(), suspension_id: suspension.id.clone(),
        expected_suspension_revision: suspension.revision, permission_request_id: suspension.permission_request_id.clone(),
        continuation_id: suspension.continuation_id.clone(), worker_id: "agent-worker".into(),
        worker_generation: "resumed-generation".into(), lease_token: "resumed-owner".into(),
        now_ms: 30, lease_until_ms: 1_000, max_running_global: 1, max_running_project: 1,
    };
    assert!(
        store.resume_runtime_task_after_permission_fair(&resume).unwrap().is_none(),
        "an approved Agent must not resume while the only slot is occupied"
    );
    assert!(store
        .finish_runtime_task_detailed_with_event(
            &RuntimeTaskLeaseWitness::from(&shell_attempt),
            "completed",
            Some(7),
            Some("completed"),
            None,
            None,
            None,
            false,
            "slot_released",
            "completed",
            None,
            "{}",
            None,
            40,
        )
        .unwrap());
    resume.now_ms = 41;
    resume.lease_until_ms = 2_000;
    let resumed_attempt = store.resume_runtime_task_after_permission_fair(&resume).unwrap().unwrap();
    assert_ne!(resumed_attempt.id, waiting_attempt.id);
    assert_ne!(resumed_attempt.worker_generation, waiting_attempt.worker_generation);
    assert_eq!(resumed_attempt.attempt_number, waiting_attempt.attempt_number + 1);
    let resumed = store.runtime_task("waiting-agent").unwrap().unwrap();
    assert_eq!(resumed.status, "running");
    assert_eq!(resumed.ownership_token.as_deref(), Some("resumed-owner"));
    assert_eq!(
        resumed.lease_until_ms,
        Some(resumed_attempt.lease_expires_at_ms)
    );
    assert!(store
        .renew_runtime_task_lease(&RenewRuntimeTaskLease {
            task_id: waiting_attempt.task_id.clone(),
            attempt_id: waiting_attempt.id.clone(),
            expected_attempt_revision: waiting_attempt.revision,
            worker_generation: waiting_attempt.worker_generation.clone(),
            lease_token: waiting_attempt.lease_token.clone(),
            current_expiry_ms: waiting_attempt.lease_expires_at_ms,
            now_ms: 42,
            requested_expiry_ms: 2_000,
        })
        .unwrap().is_none(), "original suspended attempt cannot renew");
    let renewed = store.renew_runtime_task_lease(&RenewRuntimeTaskLease {
        task_id: resumed_attempt.task_id.clone(), attempt_id: resumed_attempt.id.clone(),
        expected_attempt_revision: resumed_attempt.revision, worker_generation: resumed_attempt.worker_generation.clone(),
        lease_token: resumed_attempt.lease_token.clone(), current_expiry_ms: resumed_attempt.lease_expires_at_ms,
        now_ms: 42, requested_expiry_ms: 3_000,
    }).unwrap().expect("fresh resumed Agent heartbeat");
    assert_eq!(renewed.revision, resumed_attempt.revision + 1);
}

#[test]
fn concurrent_workers_cannot_overbook_one_durable_slot() {
    let temp = tempfile::tempdir().expect("tempdir");
    let path = temp.path().join("nola.sqlite");
    let mut seed = NolaStore::open(&path).expect("open");
    seed.create_runtime_task_with_event(&task("task-race"))
        .unwrap();
    drop(seed);

    let barrier = Arc::new(Barrier::new(3));
    let mut workers = Vec::new();
    for index in 0..2 {
        let path = path.clone();
        let barrier = Arc::clone(&barrier);
        workers.push(thread::spawn(move || {
            let mut store = NolaStore::open(path).unwrap();
            barrier.wait();
            store
                .claim_runtime_task_fair(
                    "task-race",
                    &format!("worker-{index}"),
                    "test-generation",
                    &format!("lease-{index}"),
                    200,
                    300,
                    1,
                    1,
                )
                .unwrap()
                .is_some()
        }));
    }
    barrier.wait();
    let claims = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .filter(|claimed| *claimed)
        .count();
    assert_eq!(claims, 1);
    let inspect = rusqlite::Connection::open(&path).unwrap();
    let attempts: i64 = inspect
        .query_row(
            "SELECT COUNT(*) FROM runtime_task_attempts WHERE task_id='task-race'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(attempts, 1, "the losing claims must not create attempts");
}

#[test]
fn runtime_task_event_cursor_is_global_durable_and_monotonic() {
    let temp = tempfile::tempdir().expect("tempdir");
    let path = temp.path().join("nola.sqlite");
    let mut writer = NolaStore::open(&path).expect("open writer");
    writer
        .create_runtime_task_with_event(&task("project-a-task"))
        .unwrap();
    let reader = NolaStore::open(&path).expect("open independent reader");

    let creation = reader.latest_runtime_task_event_id().unwrap();
    assert_eq!(creation, 1);
    let first = writer
        .append_runtime_task_event("project-a-task", "created", "queued", None, "{}", 10)
        .unwrap();
    assert!(first > creation);
    assert_eq!(reader.latest_runtime_task_event_id().unwrap(), first);

    let mut project_b = task("project-b-task");
    project_b.project_root = "/workspace/b".into();
    writer.create_runtime_task_with_event(&project_b).unwrap();
    let second = writer
        .append_runtime_task_event(
            "project-b-task",
            "finished",
            "completed",
            Some("done"),
            "{}",
            20,
        )
        .unwrap();
    assert!(second > first);
    assert_eq!(reader.latest_runtime_task_event_id().unwrap(), second);
}

fn finish_under_lease(
    store: &mut NolaStore,
    lease: &RuntimeTaskLeaseWitness,
    status: &str,
    now_ms: i64,
) -> nola_store::StoreResult<bool> {
    store.finish_runtime_task_detailed_with_event(
        lease, status, Some(7), Some(status), Some("owned result"), None, None,
        true, "finished", status, Some("owned result"), "{}", None, now_ms,
    )
}

fn notification_delivery_is_exact_independent_and_replayable_after_reopen(path: &std::path::Path, task_id: &str, occurrence_id: &str) {
    use nola_store_port::runtime_task::RuntimeTaskNotificationDelivery;
    let store = NolaStore::open(&path).unwrap();
    let delivery = RuntimeTaskNotificationDelivery {
        owner_conversation_id: Some("parent-session".into()), task_id: task_id.into(),
        occurrence_id: occurrence_id.into(), consumer_id: "cli".into(), command_id: format!("display:{occurrence_id}"),
    };
    let before = store.runtime_task(task_id).unwrap().unwrap();
    assert!(!store.runtime_task_notification_delivered(&delivery).unwrap());
    drop(store); // Fetch / renderer crash is not an ACK.
    let mut store = NolaStore::open(&path).unwrap();
    assert!(!store.runtime_task_notification_delivered(&delivery).unwrap());
    let mut foreign = delivery.clone(); foreign.owner_conversation_id = Some("another-session".into());
    assert!(store.acknowledge_runtime_task_delivery(&foreign).is_err());
    let mut wrong = delivery.clone(); wrong.occurrence_id = "terminal:forged".into();
    assert!(store.acknowledge_runtime_task_delivery(&wrong).is_err());
    assert_eq!(store.acknowledge_runtime_task_delivery(&delivery).unwrap(), nola_store_port::runtime_task::RuntimeTaskNotificationAcknowledgement::Delivered);
    let cursor = store.latest_runtime_task_event_id().unwrap();
    drop(store);
    let mut store = NolaStore::open(&path).unwrap();
    assert!(store.runtime_task_notification_delivered(&delivery).unwrap());
    assert_eq!(store.acknowledge_runtime_task_delivery(&delivery).unwrap(), nola_store_port::runtime_task::RuntimeTaskNotificationAcknowledgement::AlreadyDelivered);
    assert_eq!(store.latest_runtime_task_event_id().unwrap(), cursor, "duplicate ACK cannot create scheduler feedback");
    assert!(store.acknowledge_runtime_task_delivery(&wrong).is_err(), "same command with another payload conflicts");
    let mut other = delivery.clone(); other.consumer_id = "mac".into();
    assert!(!store.runtime_task_notification_delivered(&other).unwrap());
    assert_eq!(store.acknowledge_runtime_task_delivery(&other).unwrap(), nola_store_port::runtime_task::RuntimeTaskNotificationAcknowledgement::Delivered);
    assert_eq!(store.runtime_task(task_id).unwrap().unwrap(), before, "presentation cannot mutate the model mailbox or task state");
}

#[test]
fn shell_terminal_receipt_is_atomic_and_notification_owner_is_not_a_wildcard() {
    use nola_store_port::runtime_task::RuntimeTaskNotificationDelivery;
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("shell-notification.sqlite");
    let mut store = NolaStore::open(&path).unwrap();
    for (id, owner) in [("owned", Some("parent-session")), ("unowned", None)] {
        let mut draft = task(id);
        draft.owner_session_id = owner.map(str::to_string);
        store.create_runtime_task_with_event(&draft).unwrap();
        let attempt = store.claim_runtime_task(id, "worker", "generation", &format!("token-{id}"), 200, 400).unwrap().unwrap();
        let witness = RuntimeTaskLeaseWitness::from(&attempt);
        assert!(store.append_runtime_task_event(id, "notification_delivered", "delivered", None, "{}", 210).is_err());
        assert!(store.finish_runtime_task_detailed_with_event(&witness, "completed", Some(7), None, None, None, None,
            true, "notification_delivered", "delivered", None, "{}", None, 220).is_err());
        assert!(store.fail_expired_runtime_task_with_event(id, "error", "notification_delivered", "delivered", None, "{}", 500).is_err());
        assert_eq!(store.runtime_task(id).unwrap().unwrap().status, "running");
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute_batch("CREATE TRIGGER fail_receipt BEFORE INSERT ON runtime_task_terminal_receipts BEGIN SELECT RAISE(ABORT,'receipt fault'); END;").unwrap();
        assert!(finish_under_lease(&mut store, &witness, "completed", 220).is_err());
        assert_eq!(store.runtime_task(id).unwrap().unwrap().status, "running");
        assert_eq!(store.runtime_task_current_attempt(id).unwrap().unwrap(), attempt);
        db.execute_batch("DROP TRIGGER fail_receipt").unwrap();
        assert!(finish_under_lease(&mut store, &witness, "completed", 220).unwrap());
        let receipt = store.runtime_task_terminal_receipt(id).unwrap().unwrap();
        assert_eq!(receipt.task_id, id);
        assert_eq!(receipt.status, "completed");
        assert_eq!(receipt.attempts.len(), 1);
        let delivery = RuntimeTaskNotificationDelivery {
            owner_conversation_id: owner.map(str::to_string), task_id: id.into(),
            occurrence_id: format!("terminal:{}", receipt.receipt_id), consumer_id: "shell-cli".into(), command_id: format!("display-{id}"),
        };
        let mut wrong = delivery.clone();
        wrong.owner_conversation_id = if owner.is_some() { None } else { Some("foreign".into()) };
        assert!(store.acknowledge_runtime_task_delivery(&wrong).is_err());
        let mut stale = delivery.clone(); stale.occurrence_id = "terminal:old-receipt".into();
        assert!(store.acknowledge_runtime_task_delivery(&stale).is_err());
        assert_eq!(store.acknowledge_runtime_task_delivery(&delivery).unwrap(), nola_store_port::runtime_task::RuntimeTaskNotificationAcknowledgement::Delivered);
        drop(store);
        store = NolaStore::open(&path).unwrap();
        assert!(store.runtime_task_notification_delivered(&delivery).unwrap());
        assert_eq!(store.acknowledge_runtime_task_delivery(&delivery).unwrap(), nola_store_port::runtime_task::RuntimeTaskNotificationAcknowledgement::AlreadyDelivered);
        let mut other = delivery; other.consumer_id = "shell-other-ui".into();
        assert!(!store.runtime_task_notification_delivered(&other).unwrap());
        assert_eq!(store.acknowledge_runtime_task_delivery(&other).unwrap(), nola_store_port::runtime_task::RuntimeTaskNotificationAcknowledgement::Delivered);
    }
}

#[test]
fn old_attempt_cannot_retry_or_finish_after_a_new_claim_and_cold_reopen() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("terminal-aba.sqlite");
    let mut store = NolaStore::open(&path).unwrap();
    store.create_runtime_task_with_event(&task("terminal-aba")).unwrap();
    let first = store.claim_runtime_task("terminal-aba", "worker", "generation-a", "same-token", 200, 300).unwrap().unwrap();
    let old = RuntimeTaskLeaseWitness::from(&first);
    assert!(store.retry_runtime_task(&old, 250, "temporary", 220).unwrap());
    assert!(matches!(store.claim_runtime_task("terminal-aba", "worker", "generation-b", "same-token", 250, 500),
        Err(nola_store::StoreError::Conflict(_))));
    let second = store.claim_runtime_task("terminal-aba", "worker", "generation-b", "fresh-token", 250, 500).unwrap().unwrap();
    drop(store);
    let mut store = NolaStore::open(&path).unwrap();
    let before = store.runtime_task("terminal-aba").unwrap();
    let events = store.runtime_task_events("terminal-aba", 20).unwrap();
    assert!(!store.retry_runtime_task(&old, 400, "stale", 260).unwrap());
    assert!(!finish_under_lease(&mut store, &old, "completed", 260).unwrap());
    assert!(!finish_under_lease(&mut store, &old, "cancelled", 260).unwrap());
    assert_eq!(store.runtime_task("terminal-aba").unwrap(), before);
    assert_eq!(store.runtime_task_events("terminal-aba", 20).unwrap(), events);
    assert_eq!(store.runtime_task_attempt(&second.id).unwrap(), Some(second.clone()));
    assert!(finish_under_lease(&mut store, &RuntimeTaskLeaseWitness::from(&second), "completed", 270).unwrap());
    let revoked = store.runtime_task_attempt(&second.id).unwrap().unwrap();
    assert_eq!(revoked.state, "superseded");
    assert_eq!(revoked.revision, second.revision + 1);
}

#[test]
fn terminal_authority_rejects_each_stale_witness_field_and_mismatched_durable_owner() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("terminal-fields.sqlite");
    let mut store = NolaStore::open(&path).unwrap();
    store.create_runtime_task_with_event(&task("terminal-fields")).unwrap();
    let attempt = store.claim_runtime_task("terminal-fields", "worker", "generation", "token", 200, 400).unwrap().unwrap();
    let lease = RuntimeTaskLeaseWitness::from(&attempt);
    let before = store.runtime_task("terminal-fields").unwrap();
    let events = store.runtime_task_events("terminal-fields", 20).unwrap();
    for field in 0..6 {
        let mut invalid = lease.clone();
        match field {
            0 => invalid.task_id = "another-task".into(),
            1 => invalid.attempt_id = "another-attempt".into(),
            2 => invalid.expected_attempt_revision += 1,
            3 => invalid.worker_generation = "another-generation".into(),
            4 => invalid.lease_token = "another-token".into(),
            _ => invalid.current_expiry_ms += 1,
        }
        assert!(!store.retry_runtime_task(&invalid, 300, "stale", 220).unwrap());
        assert!(!finish_under_lease(&mut store, &invalid, "failed", 220).unwrap());
        assert_eq!(store.runtime_task_attempt(&attempt.id).unwrap(), Some(attempt.clone()));
        assert_eq!(store.runtime_task("terminal-fields").unwrap(), before);
        assert_eq!(store.runtime_task_events("terminal-fields", 20).unwrap(), events);
    }
    let db = rusqlite::Connection::open(&path).unwrap();
    for statement in [
        "UPDATE durable_tasks SET worker_id='another-worker' WHERE id='terminal-fields'",
        "UPDATE durable_tasks SET worker_id='worker',ownership_token='another-token' WHERE id='terminal-fields'",
        "UPDATE durable_tasks SET ownership_token='token',lease_until_ms=401 WHERE id='terminal-fields'",
    ] {
        db.execute_batch(statement).unwrap();
        assert!(!store.retry_runtime_task(&lease, 300, "mismatch", 220).unwrap());
        assert!(!finish_under_lease(&mut store, &lease, "completed", 220).unwrap());
        assert_eq!(store.runtime_task_attempt(&attempt.id).unwrap(), Some(attempt.clone()));
        assert_eq!(store.runtime_task_events("terminal-fields", 20).unwrap(), events);
    }
}

#[test]
fn terminal_authority_expires_at_the_lease_boundary_without_reviving_it() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("terminal-expiry.sqlite");
    let mut store = NolaStore::open(&path).unwrap();
    store.create_runtime_task_with_event(&task("terminal-expiry")).unwrap();
    let attempt = store.claim_runtime_task("terminal-expiry", "worker", "generation", "token", 200, 300).unwrap().unwrap();
    let lease = RuntimeTaskLeaseWitness::from(&attempt);
    let before = store.runtime_task("terminal-expiry").unwrap();
    for now_ms in [300, 301] {
        assert!(!store.retry_runtime_task(&lease, 400, "expired", now_ms).unwrap());
        for status in ["completed", "failed", "cancelled"] {
            assert!(!finish_under_lease(&mut store, &lease, status, now_ms).unwrap());
        }
    }
    assert_eq!(store.runtime_task("terminal-expiry").unwrap(), before);
    assert_eq!(store.runtime_task_attempt(&attempt.id).unwrap(), Some(attempt));
    assert_eq!(store.runtime_task_events("terminal-expiry", 20).unwrap().len(), 1);
}

#[test]
fn renewed_revision_and_accepted_timestamp_fence_old_or_backwards_terminal_writes() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = NolaStore::open(temp.path().join("terminal-renewal.sqlite")).unwrap();
    store.create_runtime_task_with_event(&task("terminal-renewal")).unwrap();
    let first = store.claim_runtime_task("terminal-renewal", "worker", "generation", "token", 200, 300).unwrap().unwrap();
    let old = RuntimeTaskLeaseWitness::from(&first);
    assert!(!store.retry_runtime_task(&old, 250, "before claim", 199).unwrap());
    assert!(!finish_under_lease(&mut store, &old, "completed", 199).unwrap());
    let refreshed = store.renew_runtime_task_lease(&renewal(&first, 230, 400)).unwrap().unwrap();
    assert!(!store.retry_runtime_task(&old, 300, "old revision", 240).unwrap());
    assert!(!finish_under_lease(&mut store, &old, "completed", 240).unwrap());
    let current = RuntimeTaskLeaseWitness::from(&refreshed);
    assert!(!store.retry_runtime_task(&current, 300, "before heartbeat", 229).unwrap());
    assert!(!finish_under_lease(&mut store, &current, "failed", 229).unwrap());
    assert_eq!(store.runtime_task_attempt(&refreshed.id).unwrap(), Some(refreshed.clone()));
    assert!(finish_under_lease(&mut store, &current, "completed", 240).unwrap());
    assert_eq!(store.runtime_task_attempt(&refreshed.id).unwrap().unwrap().revision, refreshed.revision + 1);
}

#[test]
fn durable_cancel_blocks_retry_and_success_but_allows_current_owned_cancellation() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("terminal-cancel.sqlite");
    let mut store = NolaStore::open(&path).unwrap();
    store.create_runtime_task_with_event(&task("terminal-cancel")).unwrap();
    let attempt = store.claim_runtime_task("terminal-cancel", "worker", "generation", "token", 200, 400).unwrap().unwrap();
    let lease = RuntimeTaskLeaseWitness::from(&attempt);
    assert!(store.request_runtime_task_cancel("terminal-cancel", 230).unwrap());
    drop(store);
    let mut store = NolaStore::open(&path).unwrap();
    let cancelled_request = store.runtime_task("terminal-cancel").unwrap();
    assert!(!store.retry_runtime_task(&lease, 300, "must not retry", 240).unwrap());
    for status in ["completed", "failed"] {
        assert!(!finish_under_lease(&mut store, &lease, status, 240).unwrap());
    }
    assert!(!finish_under_lease(&mut store, &lease, "cancelled", 229).unwrap());
    assert_eq!(store.runtime_task("terminal-cancel").unwrap(), cancelled_request);
    assert_eq!(store.runtime_task_attempt(&attempt.id).unwrap(), Some(attempt.clone()));
    assert!(finish_under_lease(&mut store, &lease, "cancelled", 240).unwrap());
    assert!(!finish_under_lease(&mut store, &lease, "cancelled", 241).unwrap());
    let final_task = store.runtime_task("terminal-cancel").unwrap().unwrap();
    assert_eq!(final_task.status, "cancelled");
    assert!(final_task.cancel_requested);
    assert_eq!(final_task.ended_at_ms, Some(240));
    assert_eq!(store.runtime_task_attempt(&attempt.id).unwrap().unwrap().state, "superseded");
    assert_eq!(store.runtime_task_events("terminal-cancel", 20).unwrap().iter().filter(|event| event.kind == "finished").count(), 1);
}

#[test]
fn terminal_event_failure_rolls_back_attempt_task_and_event_in_both_transaction_modes() {
    for retry in [false, true] {
        for outer in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let path = temp.path().join("terminal-event-rollback.sqlite");
            let mut store = NolaStore::open(&path).unwrap();
            store.create_runtime_task_with_event(&task("terminal-event")).unwrap();
            let attempt = store.claim_runtime_task("terminal-event", "worker", "generation", "token", 200, 400).unwrap().unwrap();
            let lease = RuntimeTaskLeaseWitness::from(&attempt);
            let before = store.runtime_task("terminal-event").unwrap();
            let events = store.runtime_task_events("terminal-event", 20).unwrap();
            let db = rusqlite::Connection::open(&path).unwrap();
            db.execute_batch("CREATE TRIGGER reject_terminal_event BEFORE INSERT ON durable_task_events BEGIN SELECT RAISE(ABORT,'fixture terminal event rejection'); END;").unwrap();
            if outer { store.begin_write_unit_of_work().unwrap(); }
            let result = if retry {
                store.retry_runtime_task(&lease, 300, "temporary", 240)
            } else {
                finish_under_lease(&mut store, &lease, "completed", 240)
            };
            assert!(result.is_err());
            assert_eq!(store.runtime_task("terminal-event").unwrap(), before);
            assert_eq!(store.runtime_task_attempt(&attempt.id).unwrap(), Some(attempt.clone()));
            assert_eq!(store.runtime_task_events("terminal-event", 20).unwrap(), events);
            if outer { store.rollback_write_unit_of_work().unwrap(); }
            drop(store);
            db.execute_batch("DROP TRIGGER reject_terminal_event").unwrap();
            let mut reopened = NolaStore::open(&path).unwrap();
            assert_eq!(reopened.runtime_task("terminal-event").unwrap(), before);
            assert_eq!(reopened.runtime_task_attempt(&attempt.id).unwrap(), Some(attempt.clone()));
            assert!(if retry {
                reopened.retry_runtime_task(&lease, 300, "temporary", 240).unwrap()
            } else {
                finish_under_lease(&mut reopened, &lease, "completed", 240).unwrap()
            });
            assert_eq!(reopened.runtime_task_attempt(&attempt.id).unwrap().unwrap().revision, attempt.revision + 1);
            assert_eq!(reopened.runtime_task_events("terminal-event", 20).unwrap().len(), events.len() + 1);
        }
    }
}

#[test]
fn task_transition_cas_failure_restores_the_already_fenced_attempt() {
    for retry in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("terminal-task-cas.sqlite");
        let mut store = NolaStore::open(&path).unwrap();
        store.create_runtime_task_with_event(&task("terminal-task-cas")).unwrap();
        let attempt = store.claim_runtime_task("terminal-task-cas", "worker", "generation", "token", 200, 400).unwrap().unwrap();
        let lease = RuntimeTaskLeaseWitness::from(&attempt);
        let before = store.runtime_task("terminal-task-cas").unwrap();
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute_batch("CREATE TRIGGER ignore_terminal_task BEFORE UPDATE ON durable_tasks WHEN NEW.status IN ('completed','retry_waiting') BEGIN SELECT RAISE(IGNORE); END;").unwrap();
        assert!(!if retry {
            store.retry_runtime_task(&lease, 300, "temporary", 240).unwrap()
        } else {
            finish_under_lease(&mut store, &lease, "completed", 240).unwrap()
        });
        assert_eq!(store.runtime_task("terminal-task-cas").unwrap(), before);
        assert_eq!(store.runtime_task_attempt(&attempt.id).unwrap(), Some(attempt));
        assert_eq!(store.runtime_task_events("terminal-task-cas", 20).unwrap().len(), 1);
    }
}

#[test]
fn expiry_operator_revokes_old_attempt_atomically_and_does_not_borrow_worker_authority() {
    for mode in 0..4 {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("terminal-recovery.sqlite");
        let mut store = NolaStore::open(&path).unwrap();
        store.create_runtime_task_with_event(&task("terminal-recovery")).unwrap();
        let attempt = store.claim_runtime_task("terminal-recovery", "worker", "generation", "token", 200, 300).unwrap().unwrap();
        let lease = RuntimeTaskLeaseWitness::from(&attempt);
        if mode == 3 { assert!(store.request_runtime_task_cancel("terminal-recovery", 250).unwrap()); }
        let before = store.runtime_task("terminal-recovery").unwrap();
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute_batch("CREATE TRIGGER reject_expiry_event BEFORE INSERT ON durable_task_events BEGIN SELECT RAISE(ABORT,'fixture expiry event rejection'); END;").unwrap();
        let recover = |store: &mut NolaStore, now_ms| match mode {
            0 => store.fail_expired_runtime_task("terminal-recovery", "expired", now_ms),
            1 => store.fail_expired_runtime_task_with_event("terminal-recovery", "expired", "expired", "failed", None, "{}", now_ms),
            _ => store.recover_expired_runtime_task("terminal-recovery", "expired", 350, now_ms),
        };
        assert!(!recover(&mut store, 299).unwrap());
        assert!(recover(&mut store, 300).is_err());
        assert_eq!(store.runtime_task("terminal-recovery").unwrap(), before);
        assert_eq!(store.runtime_task_attempt(&attempt.id).unwrap(), Some(attempt.clone()));
        db.execute_batch("DROP TRIGGER reject_expiry_event").unwrap();
        assert!(recover(&mut store, 300).unwrap());
        assert!(!recover(&mut store, 301).unwrap());
        drop(store);
        let mut reopened = NolaStore::open(&path).unwrap();
        assert_eq!(reopened.runtime_task_attempt(&attempt.id).unwrap().unwrap().state, "superseded");
        assert_eq!(reopened.runtime_task_attempt(&attempt.id).unwrap().unwrap().revision, attempt.revision + 1);
        assert!(!reopened.retry_runtime_task(&lease, 350, "stale", 301).unwrap());
        assert!(!finish_under_lease(&mut reopened, &lease, "cancelled", 301).unwrap());
        assert_eq!(reopened.runtime_task("terminal-recovery").unwrap().unwrap().status, match mode { 0 | 1 => "failed", 2 => "retry_waiting", _ => "cancelled" });
        assert_eq!(reopened.runtime_task_events("terminal-recovery", 20).unwrap().len(), 2);
    }
}

#[test]
fn claim_rejects_historical_lease_token_after_cold_reopen() {
    for fair in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("lease-nonce.sqlite");
        let mut store = NolaStore::open(&path).unwrap();
        store.create_runtime_task_with_event(&task("nonce-task")).unwrap();
        let claim = |store: &mut NolaStore, token: &str, now, expiry| {
            if fair {
                store.claim_runtime_task_fair("nonce-task", "worker", "generation", token, now, expiry, 4, 4)
            } else {
                store.claim_runtime_task("nonce-task", "worker", "generation", token, now, expiry)
            }
        };
        let first = claim(&mut store, "original", 200, 300).unwrap().unwrap();
        assert!(matches!(claim(&mut store, "original", 210, 310), Err(nola_store::StoreError::Conflict(_))));
        assert_eq!(store.runtime_task_current_attempt("nonce-task").unwrap(), Some(first.clone()));
        assert!(store.recover_expired_runtime_task("nonce-task", "expired", 350, 300).unwrap());
        drop(store);
        let mut store = NolaStore::open(&path).unwrap();
        let before = store.runtime_task("nonce-task").unwrap();
        assert!(matches!(claim(&mut store, "original", 350, 450), Err(nola_store::StoreError::Conflict(_))));
        assert_eq!(store.runtime_task("nonce-task").unwrap(), before);
        assert!(store.runtime_task_current_attempt("nonce-task").unwrap().is_none());
        let next = claim(&mut store, "successor", 350, 450).unwrap().unwrap();
        assert_ne!(first.id, next.id);
        assert_eq!(next.attempt_number, first.attempt_number + 1);
    }
}

#[test]
fn two_connections_have_one_winner_for_terminal_or_retry_authority() {
    for race_retry in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("terminal-race.sqlite");
        let mut store = NolaStore::open(&path).unwrap();
        store.create_runtime_task_with_event(&task("terminal-race")).unwrap();
        let attempt = store.claim_runtime_task("terminal-race", "worker", "generation", "token", 200, 400).unwrap().unwrap();
        let lease = RuntimeTaskLeaseWitness::from(&attempt);
        drop(store);
        let barrier = Arc::new(Barrier::new(3));
        let workers: Vec<_> = (0..2).map(|index| {
            let path = path.clone();
            let lease = lease.clone();
            let barrier = barrier.clone();
            thread::spawn(move || {
                let mut store = NolaStore::open(path).unwrap();
                barrier.wait();
                if index == 1 && race_retry {
                    store.retry_runtime_task(&lease, 300, "temporary", 240).unwrap()
                } else {
                    finish_under_lease(&mut store, &lease, if index == 0 { "completed" } else { "failed" }, 240).unwrap()
                }
            })
        }).collect();
        barrier.wait();
        assert_eq!(workers.into_iter().map(|worker| worker.join().unwrap()).filter(|won| *won).count(), 1);
        let reopened = NolaStore::open(&path).unwrap();
        let final_attempt = reopened.runtime_task_attempt(&attempt.id).unwrap().unwrap();
        assert_eq!(final_attempt.state, "superseded");
        assert_eq!(final_attempt.revision, attempt.revision + 1);
        assert_eq!(reopened.runtime_task_events("terminal-race", 20).unwrap().len(), 2);
        assert!(matches!(reopened.runtime_task("terminal-race").unwrap().unwrap().status.as_str(), "completed" | "failed" | "retry_waiting"));
    }
}

#[test]
fn worktree_file_writers_are_independent_but_overlaps_and_unknown_writers_remain_exclusive() {
    let root=tempfile::tempdir().unwrap();let path=root.path().join("scoped-writers.db");
    let mut store=NolaStore::open(&path).unwrap();
    let scoped=|id:&str,workspace:&str,gitdir:&str| {
        let mut value=task(id);value.kind="agent".into();value.project_root="/source".into();
        value.metadata_json=serde_json::json!({"can_mutate":true,"delegated_invocation":{
            "bounded_file_writes":true,"project_repository_identity":{"common_directory":"/source/.git","common_directory_identity":"repo"},
            "worktree":{"selection":{"root":workspace,"expected_head":"a".repeat(40),"expected_common_directory":"/source/.git"},
            "git_directory":gitdir,"root_identity":workspace,"git_directory_identity":gitdir,"common_directory_identity":"repo"}}}).to_string();value
    };
    let first=scoped("first","/worktrees/a","/source/.git/worktrees/a");store.create_runtime_task_with_event(&first).unwrap();
    let second=scoped("second","/worktrees/b","/source/.git/worktrees/b");store.create_runtime_task_with_event(&second).unwrap();
    for conflicting in [scoped("same-root","/worktrees/a","/source/.git/worktrees/different"),
        scoped("nested","/worktrees/a/sub","/source/.git/worktrees/nested"),
        scoped("same-index","/other-path","/source/.git/worktrees/a")] {
        assert!(store.create_runtime_task_with_event(&conflicting).is_err());
    }
    let mut broad=task("broad");broad.kind="agent".into();broad.project_root="/source".into();broad.metadata_json=serde_json::json!({"can_mutate":true}).to_string();
    assert!(store.create_runtime_task_with_event(&broad).is_err());
    drop(store);let mut store=NolaStore::open(&path).unwrap();
    assert!(store.create_runtime_task_with_event(&scoped("cold-overlap","/worktrees/a","/source/.git/worktrees/a")).is_err());
    store.create_runtime_task_with_event(&scoped("third","/worktrees/c","/source/.git/worktrees/c")).unwrap();
    assert_eq!(store.list_runtime_tasks("/source").unwrap().len(),3,"project ownership/query scope must stay original");
    // Generic scheduler fixture below carries no Agent authority and claims no actual Run.
    let generic=root.path().join("generic-fair.db");let mut queue=NolaStore::open(&generic).unwrap();
    let mut a=scoped("queue-a","/worktrees/a","/source/.git/worktrees/a");a.kind="shell_process".into();
    let mut b=scoped("queue-b","/worktrees/b","/source/.git/worktrees/b");b.kind="shell_process".into();
    queue.create_runtime_task_with_event(&a).unwrap();queue.create_runtime_task_with_event(&b).unwrap();
    assert!(queue.claim_runtime_task_fair("queue-a","w1","g1","l1",200,1000,2,2).unwrap().is_some());
    assert!(queue.claim_runtime_task_fair("queue-b","w2","g2","l2",200,1000,2,1).unwrap().is_none(),"explicit parent-project cap stays authoritative");
    assert!(queue.claim_runtime_task_fair("queue-b","w2","g2","l2",200,1000,2,2).unwrap().is_some());
}
