// SPDX-License-Identifier: Apache-2.0
mod support;

use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use nola_application::{
    ApplicationCommand, CreateRuntimeTask, PermissionIntervention,
    PermissionRequest, PermissionRequestCommand, PermissionRequestResolution,
    PermissionRequestStatus, RuntimeTask, RuntimeTaskCommand, RuntimeTaskPriority,
    RuntimeTaskResourceBudget, RuntimeTaskScheduling,
};
use nola_service_client::ApplicationServiceClient;
use serde_json::Value;

struct Service(Child);

impl Drop for Service {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn start_service(database: &std::path::Path, socket: &std::path::Path) -> Service {
    Service(
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

fn ready(client: &ApplicationServiceClient, child: &mut Service) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if client.health().is_ok() {
            return;
        }
        assert!(child.0.try_wait().expect("inspect service").is_none());
        thread::sleep(Duration::from_millis(20));
    }
    panic!("Application Service did not become ready");
}

fn task() -> RuntimeTask {
    let mut task = RuntimeTask {
        id: "permission-task".into(),
        kind: "agent".into(),
        status: "queued".into(),
        title: "Create report".into(),
        project_root: "/tmp/project".into(),
        owner_session_id: Some("conversation-permission".into()),
        parent_task_id: None,
        parent_tool_call_id: Some("permission-parent-call".into()),
        child_session_id: Some("child-permission".into()),
        agent_type: Some("general".into()),
        prompt: None,
        progress_summary: Some("queued".into()),
        last_activity: None,
        result_summary: None,
        error: None,
        output_path: None,
        unread: true,
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
        created_at_ms: 10,
        started_at_ms: None,
        ended_at_ms: None,
        updated_at_ms: 10,
        metadata_json: "{}".into(),
    };
    task.set_scheduling(RuntimeTaskScheduling {
        priority: RuntimeTaskPriority::Normal,
        queued_at_ms: task.created_at_ms,
        resource_budget: RuntimeTaskResourceBudget {
            max_wall_time_ms: Some(60_000),
            max_output_bytes: Some(1_048_576),
            ..Default::default()
        },
        output_bytes: 0,
        resource_usage: Default::default(),
        budget_exhaustion: None,
    })
    .expect("valid permission task scheduling");
    task
}

#[test]
fn pending_permission_and_custom_response_survive_real_service_restarts() {
    let directory = tempfile::tempdir().expect("temporary state");
    let database = directory.path().join("nola.db");
    let socket = directory.path().join("nola.sock");
    let client = ApplicationServiceClient::with_socket(&database, &socket);
    let mut first = start_service(&database, &socket);
    ready(&client, &mut first);

    let _: Value = client
        .execute(ApplicationCommand::RuntimeTask {
            command: RuntimeTaskCommand::Create {
                input: CreateRuntimeTask::try_from(task()).expect("create permission task input"),
            },
        })
        .expect("create runtime task");
    let prepared_claim: Option<nola_application::RuntimeTaskAttempt> = client.execute(ApplicationCommand::RuntimeTask {
        command: RuntimeTaskCommand::Claim {task_id:"permission-task".into(),worker_id:"permission-worker".into(),
            worker_generation:"permission-worker".into(),lease_token:"permission-lease".into(),now_ms:11,lease_until_ms:10_000}
    }).unwrap();
    assert!(prepared_claim.is_none(), "a Prepared agent cannot claim before actual source admission");
    // The parent source is independent of the service Task database. The real
    // Core transaction, not a marker or synthetic checkpoint, authorizes claim.
    let source_directory = tempfile::tempdir().unwrap();
    let mut source = support::open_runtime(source_directory.path().join("source.sqlite")).unwrap();
    let admission = support::agent_task_admission::source_fact_for(&mut source,
        "permission-task", "conversation-permission", "child-permission", "permission-parent-call", Some(7));
    let authority = client.import_agent_task_admission_with_request_id("permission-task-admission", &admission).unwrap();
    assert_eq!(authority.admission.as_ref(), Some(&admission));
    let attempt: Option<nola_application::RuntimeTaskAttempt> = client
        .execute(ApplicationCommand::RuntimeTask {
            command: RuntimeTaskCommand::Claim {
                task_id: "permission-task".into(),
                worker_id: "permission-worker".into(),
                worker_generation: "permission-worker".into(),
                lease_token: "permission-lease".into(),
                now_ms: 11,
                lease_until_ms: 10_000,
            },
        })
        .expect("claim permission task");
    let attempt = attempt.expect("permission claim grant");
    let suspension: Option<nola_application::RuntimeTaskPermissionSuspension> = client
        .execute(ApplicationCommand::RuntimeTask {
            command: RuntimeTaskCommand::SuspendForPermission {
                input: nola_application::SuspendRuntimeTaskForPermission {
                    task_id: attempt.task_id.clone(), attempt_id: attempt.id.clone(),
                    expected_attempt_revision: attempt.revision, worker_generation: attempt.worker_generation.clone(),
                    lease_token: attempt.lease_token.clone(), current_expiry_ms: attempt.lease_expires_at_ms,
                    permission_request: nola_store::StoredPermissionRequest {
                        closure: None,
                        request_id: "permission-1".into(), task_id: "permission-task".into(),
                        child_session_id: "child-permission".into(), capability: "file_write".into(),
                        input_json: serde_json::json!({"path": "/tmp/project/report.md"}).to_string(),
                        scope_rule: "file_write:/tmp/project/report.md".into(),
                        status: "pending".into(), decision: None, custom_response: None,
                        created_at_ms: 20, resolved_at_ms: None,
                        core_permission_request_id: Some("core-fixture-request".into()),
                        permission_instructions_json: "[]".into(),
                    },
                    continuation_id: "permission-continuation".into(),
                    continuation_json: serde_json::json!({
                        "child_session_id": "child-permission",
                        "core_permission_request_id": "core-fixture-request",
                        "run_id": "permission-run"
                    }).to_string(),
                    progress_summary: "Permission required".into(),
                    last_activity: Some("file_write".into()), now_ms: 20,
                },
            },
        })
        .expect("atomically suspend task with pending permission");
    assert!(suspension.is_some(), "suspension must atomically create the request and continuation");
    let suspension = suspension.unwrap();

    drop(first);
    let _ = std::fs::remove_file(&socket);
    let mut second = start_service(&database, &socket);
    ready(&client, &mut second);
    let pending: Option<PermissionRequest> = client
        .execute(ApplicationCommand::PermissionRequest {
            command: PermissionRequestCommand::PendingForTask {
                task_id: "permission-task".into(),
            },
        })
        .expect("restore pending permission");
    assert_eq!(pending.unwrap().request_id, "permission-1");
    let resolved: nola_application::PermissionResolutionOutcome = client
        .execute(ApplicationCommand::PermissionRequest {
            command: PermissionRequestCommand::Intervene {
                request_id: "permission-1".into(),
                intervention: PermissionIntervention::ProvideInstruction {
                    instruction: "Write under /tmp/project/drafts instead".into(),
                },
                resolved_at_ms: 30,
            },
        })
        .expect("resolve custom response");
    assert!(matches!(resolved, nola_application::PermissionResolutionOutcome::Resolved { .. }));

    drop(second);
    let _ = std::fs::remove_file(&socket);
    let mut third = start_service(&database, &socket);
    ready(&client, &mut third);
    let restored: Option<PermissionRequest> = client
        .execute(ApplicationCommand::PermissionRequest {
            command: PermissionRequestCommand::Get {
                request_id: "permission-1".into(),
            },
        })
        .expect("restore resolved permission");
    let restored = restored.expect("resolved permission retained");
    assert_eq!(restored.status, PermissionRequestStatus::Resolved);
    assert_eq!(
        restored.resolution,
        Some(PermissionRequestResolution::Intervention {
            intervention: PermissionIntervention::ProvideInstruction {
                instruction: "Write under /tmp/project/drafts instead".into(),
            },
        })
    );

    let grant: Option<nola_application::RuntimeTaskAttempt> = client.execute(ApplicationCommand::RuntimeTask {
        command: RuntimeTaskCommand::ResumeAfterPermissionFair {
            input: nola_application::ResumeRuntimeTaskAfterPermissionFair {
                task_id: "permission-task".into(), suspension_id: suspension.id.clone(),
                expected_suspension_revision: suspension.revision,
                permission_request_id: suspension.permission_request_id.clone(),
                continuation_id: suspension.continuation_id.clone(),
                worker_id: "resumed-worker".into(), worker_generation: "resumed-generation".into(),
                lease_token: "resumed-token".into(), now_ms: 40, lease_until_ms: 10_000,
                max_running_global: 2, max_running_project: 2,
            },
        },
    }).expect("grant exact continuation after decision restart");
    let grant = grant.expect("fresh resumed attempt");
    assert_ne!(grant.id, attempt.id);
    drop(third);
    let _ = std::fs::remove_file(&socket);
    let mut fourth = start_service(&database, &socket);
    ready(&client, &mut fourth);
    let renewed: nola_application::RuntimeTaskAttempt = client.execute(ApplicationCommand::RuntimeTask {
        command: RuntimeTaskCommand::RenewLease {
            task_id: grant.task_id.clone(), attempt_id: grant.id.clone(),
            expected_attempt_revision: grant.revision, worker_generation: grant.worker_generation.clone(),
            lease_token: grant.lease_token.clone(), current_expiry_ms: grant.lease_expires_at_ms,
            now_ms: 50, requested_expiry_ms: 11_000,
        },
    }).expect("renew persisted exact grant");
    assert!(renewed.revision > grant.revision);
    let proof: Option<nola_application::RuntimeTaskPermissionResume> = client.execute(ApplicationCommand::RuntimeTask {
        command: RuntimeTaskCommand::PermissionResume {
            task_id: grant.task_id.clone(), suspension_id: suspension.id.clone(),
            permission_request_id: suspension.permission_request_id.clone(),
            continuation_id: suspension.continuation_id.clone(),
        },
    }).expect("read immutable receipt after service restart and renewal");
    let proof = proof.expect("permission grant is durable");
    assert_eq!(proof.granted_attempt, grant, "receipt must not borrow the renewed lease");
    assert_eq!(proof.suspension.state, "resumed");
    let old_renewal: Result<nola_application::RuntimeTaskAttempt, _> = client.execute(ApplicationCommand::RuntimeTask {
        command: RuntimeTaskCommand::RenewLease {
            task_id: attempt.task_id, attempt_id: attempt.id,
            expected_attempt_revision: attempt.revision, worker_generation: attempt.worker_generation,
            lease_token: attempt.lease_token, current_expiry_ms: attempt.lease_expires_at_ms,
            now_ms: 60, requested_expiry_ms: 12_000,
        },
    });
    assert!(old_renewal.is_err(), "pre-permission worker regained authority after restart");
    drop(fourth);
}
