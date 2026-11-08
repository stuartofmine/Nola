// SPDX-License-Identifier: Apache-2.0
//! Actual source admission and a single durable Task authority; no parent-body mirror.
mod support;
use nola_application::{
    ApplicationCommand, ClaimAgentTaskFair, RecoverExpiredAgentTask, RuntimeTaskAttempt,
    RuntimeTaskCommand, RuntimeTaskLeaseWitness,
};
use nola_domain::agent_task_admission::{
    RuntimeAgentTaskAuthority, RuntimeAgentTaskAuthorityPhase,
};
use nola_store::NolaStore;
use serde_json::json;

fn seed(path: &std::path::Path) {
    support::agent_task_admission::admitted(path, None);
    assert!(
        NolaStore::open(path)
            .unwrap()
            .load_checkpoint("parent")
            .unwrap()
            .is_none(),
        "Task ownership must not copy the source parent's checkpoint or transcript"
    );
}
fn claim(token: &str, at: i64) -> ApplicationCommand {
    ApplicationCommand::ClaimAgentTaskFair {
        input: ClaimAgentTaskFair {
            owner_conversation_id: "parent".into(),
            task_id: "task".into(),
            worker_id: format!("worker-{token}"),
            worker_generation: format!("generation-{token}"),
            lease_token: token.into(),
            now_ms: at,
            lease_until_ms: at + 1000,
            max_running_global: 4,
            max_running_project: 4,
        },
    }
}
fn recover(attempt: &RuntimeTaskAttempt, at: i64) -> ApplicationCommand {
    ApplicationCommand::RecoverExpiredAgentTask {
        input: RecoverExpiredAgentTask {
            owner_conversation_id: "parent".into(),
            lease: RuntimeTaskLeaseWitness::from(attempt),
            error: "worker expired before child admission".into(),
            retry_at_ms: at,
            now_ms: at,
        },
    }
}
fn task(path: &std::path::Path) -> nola_store_port::runtime_task::StoredRuntimeTask {
    NolaStore::open(path)
        .unwrap()
        .runtime_task("task")
        .unwrap()
        .unwrap()
}
fn leaf(path: &std::path::Path) -> RuntimeAgentTaskAuthority {
    NolaStore::open(path)
        .unwrap()
        .runtime_agent_task_authority("task")
        .unwrap()
        .unwrap()
}
fn rows(path: &std::path::Path) -> Vec<(String, Vec<Vec<rusqlite::types::Value>>)> {
    let db = rusqlite::Connection::open(path).unwrap();
    let tables=db.prepare("SELECT name FROM sqlite_schema WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name").unwrap()
        .query_map([],|r|r.get::<_,String>(0)).unwrap().collect::<Result<Vec<_>,_>>().unwrap();
    tables
        .into_iter()
        .map(|name| {
            let mut query = db
                .prepare(&format!("SELECT * FROM \"{}\"", name.replace('"', "\"\"")))
                .unwrap();
            let count = query.column_count();
            let mut rows = query
                .query_map([], |r| {
                    (0..count)
                        .map(|i| r.get::<_, rusqlite::types::Value>(i))
                        .collect()
                })
                .unwrap()
                .collect::<Result<Vec<Vec<_>>, _>>()
                .unwrap();
            rows.sort_by_cached_key(|r| format!("{r:?}"));
            (name, rows)
        })
        .collect()
}
#[test]
fn real_cross_store_admission_short_expiry_and_successor_have_one_owner() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("task.db");
    seed(&path);
    let initial = leaf(&path);
    assert_eq!(initial.phase, RuntimeAgentTaskAuthorityPhase::Admitted);
    let mut app = support::open_runtime(&path).unwrap();
    let old: RuntimeTaskAttempt =
        serde_json::from_value(app.execute(claim("old", 1000)).unwrap()).unwrap();
    assert_eq!(task(&path).status, "running");
    assert_eq!(
        task(&path).ownership_token.as_deref(),
        Some(old.lease_token.as_str())
    );
    drop(app);
    let mut cold = support::open_runtime(&path).unwrap();
    assert_eq!(cold.execute(recover(&old, 2000)).unwrap(), json!(true));
    assert_eq!(task(&path).status, "retry_waiting");
    assert_eq!(task(&path).retry_count, 1);
    assert!(task(&path).ownership_token.is_none());
    let next: RuntimeTaskAttempt =
        serde_json::from_value(cold.execute(claim("next", 2001)).unwrap()).unwrap();
    assert_ne!(next.id, old.id);
    assert_ne!(next.lease_token, old.lease_token);
    assert_eq!(next.attempt_number, 2);
    assert_eq!(
        task(&path).ownership_token.as_deref(),
        Some(next.lease_token.as_str())
    );
    assert!(
        cold.execute(claim("illegal-running", 2002)).is_err(),
        "Running claim exclusion is strict"
    );
    assert_eq!(
        cold.execute(recover(&old, 3000)).unwrap(),
        json!(false),
        "old witness cannot retire successor"
    );
    let current = leaf(&path);
    assert_eq!(current.admission, initial.admission);
    assert!(current.authority_revision > initial.authority_revision);
}
#[test]
fn source_not_committed_and_prepared_task_never_grant_execution() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("prepared.db");
    let mut target = support::open_runtime(&path).unwrap();
    support::agent_task_admission::draft(&mut target, None);
    let before = rows(&path);
    assert!(target.execute(claim("unproven", 1000)).is_err());
    assert_eq!(rows(&path), before);
    let mut source = support::open_runtime(":memory:").unwrap();
    assert!(source
        .execute(ApplicationCommand::ExportAgentTaskAdmission {
            conversation_id: "parent".into(),
            run_id: "parent-run".into(),
            task_id: "task".into(),
            event_cursor: 1
        })
        .is_err());
    assert_eq!(leaf(&path).phase, RuntimeAgentTaskAuthorityPhase::Prepared);
    let fact = support::agent_task_admission::source_fact(&mut source);
    let args = serde_json::json!({"type":"import_host_agent_task_admission","admission":fact});
    assert!(
        serde_json::from_value::<ApplicationCommand>(args).is_err(),
        "ordinary JSON cannot import a self-made source proof"
    );
    target
        .execute(ApplicationCommand::ImportHostAgentTaskAdmission {
            admission: fact.clone(),
        })
        .unwrap();
    let admitted = leaf(&path);
    let immutable = rows(&path);
    target
        .execute(ApplicationCommand::ImportHostAgentTaskAdmission { admission: fact })
        .unwrap();
    assert_eq!(leaf(&path), admitted);
    assert_eq!(
        rows(&path),
        immutable,
        "same actual source fact is idempotent"
    );
}
#[test]
fn authority_failure_rolls_back_grant_and_expiry_retirement() {
    for during_claim in [true, false] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("rollback.db");
        seed(&path);
        let mut app = support::open_runtime(&path).unwrap();
        let old = if during_claim {
            None
        } else {
            Some(
                serde_json::from_value::<RuntimeTaskAttempt>(
                    app.execute(claim("old", 1000)).unwrap(),
                )
                .unwrap(),
            )
        };
        let before = rows(&path);
        rusqlite::Connection::open(&path).unwrap().execute_batch(
            "CREATE TRIGGER reject_authority_update BEFORE UPDATE OF authority_revision ON runtime_agent_task_authorities
             BEGIN SELECT RAISE(ABORT,'injected authority commit failure'); END;").unwrap();
        assert!(app
            .execute(
                old.as_ref()
                    .map_or_else(|| claim("new", 1000), |old| recover(old, 2000))
            )
            .is_err());
        assert_eq!(
            rows(&path),
            before,
            "Task/Attempt/leaf/events/budgets commit or roll back together"
        );
    }
}
#[test]
fn renewal_cas_winner_is_not_retired_and_expired_grants_are_not_revived() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("renew.db");
    seed(&path);
    let mut app = support::open_runtime(&path).unwrap();
    let old: RuntimeTaskAttempt =
        serde_json::from_value(app.execute(claim("old", 1000)).unwrap()).unwrap();
    let next: RuntimeTaskAttempt = serde_json::from_value(
        app.execute(ApplicationCommand::RuntimeTask {
            command: RuntimeTaskCommand::RenewLease {
                task_id: old.task_id.clone(),
                attempt_id: old.id.clone(),
                expected_attempt_revision: old.revision,
                worker_generation: old.worker_generation.clone(),
                lease_token: old.lease_token.clone(),
                current_expiry_ms: old.lease_expires_at_ms,
                now_ms: 1999,
                requested_expiry_ms: 2999,
            },
        })
        .unwrap(),
    )
    .unwrap();
    let before = rows(&path);
    assert_eq!(app.execute(recover(&old, 2001)).unwrap(), json!(false));
    assert_eq!(rows(&path), before);
    assert!(app
        .execute(ApplicationCommand::RuntimeTask {
            command: RuntimeTaskCommand::RenewLease {
                task_id: next.task_id.clone(),
                attempt_id: next.id.clone(),
                expected_attempt_revision: next.revision,
                worker_generation: next.worker_generation.clone(),
                lease_token: next.lease_token.clone(),
                current_expiry_ms: next.lease_expires_at_ms,
                now_ms: 2999,
                requested_expiry_ms: 3999
            }
        })
        .is_err());
    assert_eq!(app.execute(recover(&next, 3000)).unwrap(), json!(true));
}
#[test]
fn lost_claim_ack_uses_the_same_receipt_and_never_creates_another_attempt() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("receipt.db");
    seed(&path);
    let command = claim("old", 1000);
    let raw = serde_json::to_string(&command).unwrap();
    let request = nola_application::ExecuteReceivedCommand {
        client_id: "worker".into(),
        request_id: "stable-claim".into(),
        command_type: "claim_agent_task_fair".into(),
        command_json: raw.clone(),
        expected_revision: None,
        received_at: "1000".into(),
    };
    let mut app = support::open_runtime(&path).unwrap();
    let committed = app
        .execute_received_command(request.clone(), serde_json::from_str(&raw).unwrap())
        .unwrap();
    assert!(matches!(
        committed.outcome,
        nola_application::ApplicationCommandReceiptOutcome::Committed { .. }
    ));
    drop(app);
    let before = rows(&path);
    let mut cold = support::open_runtime(&path).unwrap();
    assert!(
        cold.execute_received_command(request, serde_json::from_str(&raw).unwrap())
            .unwrap()
            .replayed
    );
    assert_eq!(rows(&path), before);
    let count: i64 = rusqlite::Connection::open(&path)
        .unwrap()
        .query_row("SELECT count(*) FROM runtime_task_attempts", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(count, 1);
}
