// SPDX-License-Identifier: Apache-2.0

use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use nola_application::{
    ApplicationCommand, CreateRuntimeTask, RuntimeTask, RuntimeTaskCommand,
    RuntimeTaskLeaseWitness, RuntimeTaskResourceBudget, RuntimeTaskScheduling,
};
use nola_service_client::ApplicationServiceClient;

struct OwnedService(Child);
impl std::ops::Deref for OwnedService {
    type Target = Child;
    fn deref(&self) -> &Child {
        &self.0
    }
}
impl std::ops::DerefMut for OwnedService {
    fn deref_mut(&mut self) -> &mut Child {
        &mut self.0
    }
}
impl Drop for OwnedService {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn start_service(database: &std::path::Path, socket: &std::path::Path) -> OwnedService {
    OwnedService(
        Command::new(env!("CARGO_BIN_EXE_nola-service"))
            .args(["--database"])
            .arg(database)
            .args(["--socket"])
            .arg(socket)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start real Application Service"),
    )
}

fn wait_until_ready(client: &ApplicationServiceClient, child: &mut Child) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if client.health().is_ok() {
            return;
        }
        assert!(
            child
                .try_wait()
                .expect("inspect Application Service")
                .is_none(),
            "Application Service exited before becoming ready"
        );
        thread::sleep(Duration::from_millis(20));
    }
    panic!("Application Service did not become ready");
}

fn restart_service(
    child: &mut Child,
    database: &std::path::Path,
    socket: &std::path::Path,
    client: &ApplicationServiceClient,
) -> OwnedService {
    child.kill().expect("kill Application Service");
    child.wait().expect("reap Application Service");
    let _ = std::fs::remove_file(socket);
    let mut restarted = start_service(database, socket);
    wait_until_ready(client, &mut restarted);
    restarted
}

fn execute(client: &ApplicationServiceClient, command: RuntimeTaskCommand) -> serde_json::Value {
    client
        .execute(ApplicationCommand::RuntimeTask { command })
        .expect("execute runtime task command through real service")
}

fn task() -> CreateRuntimeTask {
    CreateRuntimeTask {
        id: "long-task-1".into(),
        // This fixture exercises generic worker lease/receipt CAS over IPC.
        kind: "shell_process".into(),
        title: "Build a long report".into(),
        project_root: "/tmp/nola-long-task".into(),
        owner_session_id: Some("conversation-long-task".into()),
        parent_task_id: None,
        parent_tool_call_id: Some("tool-long-task".into()),
        child_session_id: Some("child-long-task".into()),
        agent_type: Some("general".into()),
        prompt: Some("Continue until the report is complete".into()),
        initial_progress_summary: Some("queued".into()),
        initial_activity: None,
        output_path: None,
        initially_unread: true,
        input_watermark: Some(7),
        created_at_ms: 100,
        metadata: serde_json::json!({"sop_revision": 1}),
        scheduling: RuntimeTaskScheduling {
            priority: Default::default(),
            queued_at_ms: 100,
            resource_budget: RuntimeTaskResourceBudget {
                max_wall_time_ms: Some(60_000),
                max_output_bytes: Some(1_048_576),
                ..Default::default()
            },
            output_bytes: 0,
            resource_usage: Default::default(),
            budget_exhaustion: None,
        },
    }
}

fn terminal_command(
    lease: &RuntimeTaskLeaseWitness,
    status: &str,
    now_ms: i64,
) -> RuntimeTaskCommand {
    RuntimeTaskCommand::FinishDetailedWithEvent {
        lease: lease.clone(),
        status: status.into(),
        completion_watermark: Some(7),
        progress_summary: Some(status.into()),
        result_summary: None,
        error: None,
        output_path: None,
        unread: true,
        event_kind: "finished".into(),
        event_status: status.into(),
        event_summary: None,
        event_detail_json: "{}".into(),
        final_observation: None,
        now_ms,
    }
}


#[test]
fn creation_event_failure_rolls_back_task_and_success_has_one_initial_event() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("nola.db");
    let socket = directory.path().join("nola.sock");
    let client = ApplicationServiceClient::with_socket(&database, &socket);
    let mut service = start_service(&database, &socket);
    wait_until_ready(&client, &mut service);
    let connection = rusqlite::Connection::open(&database).unwrap();
    connection.execute_batch("CREATE TRIGGER reject_task_creation_event BEFORE INSERT ON durable_task_events BEGIN SELECT RAISE(ABORT, 'fixture event failure'); END;").unwrap();
    let failed = client
        .execute::<serde_json::Value, _>(ApplicationCommand::RuntimeTask {
            command: RuntimeTaskCommand::Create { input: task() },
        })
        .unwrap_err();
    assert!(failed.contains("fixture event failure"), "{failed}");
    assert!(execute(
        &client,
        RuntimeTaskCommand::Get {
            id: "long-task-1".into()
        }
    )
    .is_null());
    assert_eq!(
        execute(
            &client,
            RuntimeTaskCommand::ListEvents {
                task_id: "long-task-1".into(),
                limit: 10,
            }
        ),
        serde_json::json!([])
    );
    connection
        .execute_batch("DROP TRIGGER reject_task_creation_event")
        .unwrap();
    execute(&client, RuntimeTaskCommand::Create { input: task() });
    let events = execute(
        &client,
        RuntimeTaskCommand::ListEvents {
            task_id: "long-task-1".into(),
            limit: 10,
        },
    );
    assert_eq!(events.as_array().unwrap().len(), 1);
    assert_eq!(events[0]["kind"], "created");
    assert_eq!(events[0]["status"], "queued");
    assert!(client
        .execute::<serde_json::Value, _>(ApplicationCommand::RuntimeTask {
            command: RuntimeTaskCommand::Create { input: task() },
        })
        .is_err());
    assert_eq!(
        execute(
            &client,
            RuntimeTaskCommand::ListEvents {
                task_id: "long-task-1".into(),
                limit: 10,
            }
        ),
        events
    );
}

#[test]
fn claimed_task_recovers_after_worker_crash_and_cancellation_survives_service_restart() {
    let directory = tempfile::tempdir().expect("temporary service state");
    let database = directory.path().join("nola.db");
    let socket = directory.path().join("nola.sock");
    let client = ApplicationServiceClient::with_socket(&database, &socket);
    let mut service = start_service(&database, &socket);
    wait_until_ready(&client, &mut service);

    let _ = execute(&client, RuntimeTaskCommand::Create { input: task() });
    let claimed = execute(
        &client,
        RuntimeTaskCommand::Claim {
            task_id: "long-task-1".into(),
            worker_id: "worker-a".into(),
            worker_generation: "worker-a".into(),
            lease_token: "lease-a".into(),
            now_ms: 110,
            lease_until_ms: 210,
        },
    );
    assert!(claimed.is_object());
    let claimed_attempt: nola_application::RuntimeTaskAttempt =
        serde_json::from_value(claimed).expect("decode claimed attempt identity");
    assert_eq!(claimed_attempt.worker_generation, "worker-a");
    assert_eq!(claimed_attempt.lease_token, "lease-a");
    assert_eq!(claimed_attempt.revision, 1);
    let renewed: nola_application::RuntimeTaskAttempt = client
        .execute(ApplicationCommand::RuntimeTask {
            command: RuntimeTaskCommand::RenewLease {
                task_id: claimed_attempt.task_id.clone(),
                attempt_id: claimed_attempt.id.clone(),
                expected_attempt_revision: claimed_attempt.revision,
                worker_generation: claimed_attempt.worker_generation.clone(),
                lease_token: claimed_attempt.lease_token.clone(),
                current_expiry_ms: claimed_attempt.lease_expires_at_ms,
                now_ms: 111,
                requested_expiry_ms: 211,
            },
        })
        .expect("renew exact runtime task attempt over service IPC");
    assert_eq!(renewed.id, claimed_attempt.id);
    assert_eq!(renewed.revision, 2);
    assert_eq!(renewed.lease_expires_at_ms, 211);
    let old_lease = RuntimeTaskLeaseWitness::from(&claimed_attempt);
    assert_eq!(
        execute(&client, terminal_command(&old_lease, "completed", 112)).as_bool(),
        Some(false)
    );
    assert_eq!(
        execute(
            &client,
            RuntimeTaskCommand::Retry {
                lease: old_lease.clone(),
                retry_at_ms: 220,
                error: "stale revision".into(),
                now_ms: 112,
            }
        )
        .as_bool(),
        Some(false)
    );
    let renewed_lease = RuntimeTaskLeaseWitness::from(&renewed);
    assert_eq!(
        execute(&client, terminal_command(&renewed_lease, "completed", 211)).as_bool(),
        Some(false)
    );
    assert_eq!(
        execute(
            &client,
            RuntimeTaskCommand::Retry {
                lease: renewed_lease,
                retry_at_ms: 220,
                error: "expired lease".into(),
                now_ms: 211,
            }
        )
        .as_bool(),
        Some(false)
    );

    service = restart_service(&mut service, &database, &socket, &client);
    let after_crash: Option<RuntimeTask> = serde_json::from_value(execute(
        &client,
        RuntimeTaskCommand::Get {
            id: "long-task-1".into(),
        },
    ))
    .expect("decode task after crash");
    let after_crash = after_crash.expect("claimed task survives service crash");
    assert_eq!(after_crash.status, "running");
    assert_eq!(after_crash.ownership_token.as_deref(), Some("lease-a"));

    let recovered = execute(
        &client,
        RuntimeTaskCommand::RecoverExpired {
            task_id: "long-task-1".into(),
            error: "worker process disappeared".into(),
            retry_at_ms: 220,
            now_ms: 220,
        },
    );
    assert_eq!(recovered.as_bool(), Some(true));
    let reclaimed = execute(
        &client,
        RuntimeTaskCommand::Claim {
            task_id: "long-task-1".into(),
            worker_id: "worker-b".into(),
            worker_generation: "worker-b".into(),
            lease_token: "lease-b".into(),
            now_ms: 221,
            lease_until_ms: 400,
        },
    );
    assert!(reclaimed.is_object());
    let reclaimed_attempt: nola_application::RuntimeTaskAttempt =
        serde_json::from_value(reclaimed).expect("decode reclaimed attempt authority");

    let cancellation_requested = execute(
        &client,
        RuntimeTaskCommand::RequestCancel {
            task_id: "long-task-1".into(),
            now_ms: 230,
        },
    );
    assert_eq!(cancellation_requested.as_bool(), Some(true));
    service = restart_service(&mut service, &database, &socket, &client);
    let pending_cancel: Option<RuntimeTask> = serde_json::from_value(execute(
        &client,
        RuntimeTaskCommand::Get {
            id: "long-task-1".into(),
        },
    ))
    .expect("decode pending cancellation");
    let pending_cancel = pending_cancel.expect("cancellation survives restart");
    assert_eq!(pending_cancel.status, "running");
    assert!(pending_cancel.cancel_requested);
    assert_eq!(pending_cancel.cancel_requested_at_ms, Some(230));
    let current_lease = RuntimeTaskLeaseWitness::from(&reclaimed_attempt);
    for status in ["completed", "failed"] {
        assert_eq!(
            execute(&client, terminal_command(&current_lease, status, 240)).as_bool(),
            Some(false)
        );
    }
    assert_eq!(
        execute(
            &client,
            RuntimeTaskCommand::Retry {
                lease: current_lease.clone(),
                retry_at_ms: 250,
                error: "durable cancellation".into(),
                now_ms: 240,
            }
        )
        .as_bool(),
        Some(false)
    );

    let cancelled = execute(
        &client,
        RuntimeTaskCommand::FinishDetailedWithEvent {
            lease: RuntimeTaskLeaseWitness::from(&reclaimed_attempt),
            status: "cancelled".into(),
            completion_watermark: Some(7),
            progress_summary: Some("cancelled".into()),
            result_summary: None,
            error: None,
            output_path: None,
            unread: true,
            event_kind: "cancelled".into(),
            event_status: "cancelled".into(),
            event_summary: None,
            event_detail_json: "{}".into(),
            final_observation: None,
            now_ms: 240,
        },
    );
    assert_eq!(cancelled.as_bool(), Some(true));
    let duplicate_finish = execute(
        &client,
        RuntimeTaskCommand::FinishDetailedWithEvent {
            lease: RuntimeTaskLeaseWitness::from(&reclaimed_attempt),
            status: "cancelled".into(),
            completion_watermark: Some(7),
            progress_summary: Some("cancelled".into()),
            result_summary: None,
            error: None,
            output_path: None,
            unread: true,
            event_kind: "cancelled".into(),
            event_status: "cancelled".into(),
            event_summary: None,
            event_detail_json: "{}".into(),
            final_observation: None,
            now_ms: 241,
        },
    );
    assert_eq!(
        duplicate_finish.as_bool(),
        Some(false),
        "terminal transition is idempotent"
    );
    let cannot_reclaim = execute(
        &client,
        RuntimeTaskCommand::Claim {
            task_id: "long-task-1".into(),
            worker_id: "worker-c".into(),
            worker_generation: "worker-c".into(),
            lease_token: "lease-c".into(),
            now_ms: 500,
            lease_until_ms: 600,
        },
    );
    assert!(cannot_reclaim.is_null());

    let final_task: Option<RuntimeTask> = serde_json::from_value(execute(
        &client,
        RuntimeTaskCommand::Get {
            id: "long-task-1".into(),
        },
    ))
    .expect("decode final task");
    let final_task = final_task.expect("terminal task remains durable");
    assert_eq!(final_task.status, "cancelled");
    assert_eq!(final_task.retry_count, 1);
    assert_eq!(final_task.completion_watermark, Some(7));
    let inspect = rusqlite::Connection::open(&database).unwrap();
    for (attempt, revision) in [
        (&claimed_attempt, renewed.revision + 1),
        (&reclaimed_attempt, reclaimed_attempt.revision + 1),
    ] {
        let durable: (String, i64) = inspect
            .query_row(
                "SELECT state,revision FROM runtime_task_attempts WHERE id=?1",
                [&attempt.id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(durable, ("superseded".into(), revision));
    }

    service.kill().expect("stop final Application Service");
    service.wait().expect("reap final Application Service");
}

#[test]
fn cancellation_wins_when_worker_dies_before_acknowledging_it() {
    let directory = tempfile::tempdir().expect("temporary service state");
    let database = directory.path().join("nola.db");
    let socket = directory.path().join("nola.sock");
    let client = ApplicationServiceClient::with_socket(&database, &socket);
    let mut service = start_service(&database, &socket);
    wait_until_ready(&client, &mut service);

    let _ = execute(&client, RuntimeTaskCommand::Create { input: task() });
    assert!(execute(
        &client,
        RuntimeTaskCommand::Claim {
            task_id: "long-task-1".into(),
            worker_id: "worker-a".into(),
            worker_generation: "worker-a".into(),
            lease_token: "lease-a".into(),
            now_ms: 110,
            lease_until_ms: 210,
        },
    )
    .is_object());
    assert_eq!(
        execute(
            &client,
            RuntimeTaskCommand::RequestCancel {
                task_id: "long-task-1".into(),
                now_ms: 120,
            },
        )
        .as_bool(),
        Some(true)
    );

    service = restart_service(&mut service, &database, &socket, &client);
    assert_eq!(
        execute(
            &client,
            RuntimeTaskCommand::RecoverExpired {
                task_id: "long-task-1".into(),
                error: "worker crashed before cancellation acknowledgement".into(),
                retry_at_ms: 220,
                now_ms: 220,
            },
        )
        .as_bool(),
        Some(true)
    );
    let final_task: Option<RuntimeTask> = serde_json::from_value(execute(
        &client,
        RuntimeTaskCommand::Get {
            id: "long-task-1".into(),
        },
    ))
    .expect("decode recovered cancellation");
    let final_task = final_task.expect("task survives worker crash");
    assert_eq!(final_task.status, "cancelled");
    assert_eq!(final_task.retry_count, 0);
    assert!(final_task.retry_at_ms.is_none());
    assert_eq!(final_task.ended_at_ms, Some(220));

    service.kill().expect("stop final Application Service");
    service.wait().expect("reap final Application Service");
}
