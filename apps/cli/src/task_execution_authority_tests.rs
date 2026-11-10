// SPDX-License-Identifier: Apache-2.0
//! Real admitted Store grants must cross the worker's explicit local adoption boundary.
use super::*;
use nola_application::{ResumeRuntimeTaskAfterPermissionFair, SuspendRuntimeTaskForPermission};
#[path = "../../../core/nola-application/tests/support/mod.rs"]
mod source_support;

#[test]
fn permission_store_grant_cannot_execute_until_explicit_local_adoption() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("authority.sqlite");
    let mut store = nola_store::NolaStore::open(&path).unwrap();
    let task_id = uuid::Uuid::new_v4().to_string();
    store
        .create_runtime_task_with_event(&nola_store::StoredRuntimeTask {
            id: task_id.clone(),
            kind: "agent".into(),
            status: "queued".into(),
            title: "permission".into(),
            project_root: "/fixture".into(),
            owner_session_id: Some("parent".into()),
            parent_task_id: None,
            parent_tool_call_id: Some("task-call".into()),
            child_session_id: Some("child".into()),
            agent_type: Some("general".into()),
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
            metadata_json: "{}".into(),
        })
        .unwrap();
    source_support::agent_task_admission::admit_existing(
        &path,
        &task_id,
        "parent",
        "child",
        "task-call",
        Some(7),
    );
    let first = store
        .claim_runtime_task(&task_id, "worker", "generation-1", "token-1", 200, 1000)
        .unwrap()
        .unwrap();
    let old_handle = register_runtime_task_attempt(first.clone());
    let authority = RuntimeTaskExecutionAuthority::new(old_handle.clone());
    let continuation = r#"{"child_session_id":"child","core_permission_request_id":"core-permission","run_id":"run"}"#;
    let suspended = authority
        .suspend_permission_with("permission", "continuation", continuation, |attempt| {
            store
                .suspend_runtime_task_for_permission(&SuspendRuntimeTaskForPermission {
                    task_id: task_id.clone(),
                    attempt_id: attempt.id.clone(),
                    expected_attempt_revision: attempt.revision,
                    worker_generation: attempt.worker_generation.clone(),
                    lease_token: attempt.lease_token.clone(),
                    current_expiry_ms: attempt.lease_expires_at_ms,
                    permission_request: nola_store_port::permission::StoredPermissionRequest {
                        closure: None,
                        request_id: "permission".into(),
                        task_id: task_id.clone(),
                        child_session_id: "child".into(),
                        capability: "file.write".into(),
                        input_json: "{}".into(),
                        scope_rule: "file.write:/fixture/out".into(),
                        status: "pending".into(),
                        decision: None,
                        custom_response: None,
                        created_at_ms: 250,
                        resolved_at_ms: None,
                        core_permission_request_id: Some("core-permission".into()),
                        permission_instructions_json: "[]".into(),
                    },
                    continuation_id: "continuation".into(),
                    continuation_json: continuation.into(),
                    progress_summary: "permission".into(),
                    last_activity: None,
                    now_ms: 250,
                })
                .map_err(|error| error.to_string())
        })
        .unwrap();
    assert!(authority
        .with_active::<()>(|_| panic!("suspended work executed"))
        .unwrap()
        .is_none());
    assert!(authority
        .with_renewal::<()>(|_| panic!("revoked lease renewed"))
        .unwrap()
        .is_none());
    rusqlite::Connection::open(&path).unwrap().execute(
        "UPDATE permission_requests SET status='resolved',decision='allow_once',resolved_at_ms=300 WHERE request_id='permission'", [],
    ).unwrap();
    let admitted = authority
        .prepare_permission_resume_with(|_, suspension| {
            store
                .resume_runtime_task_after_permission_fair(&ResumeRuntimeTaskAfterPermissionFair {
                    task_id: task_id.clone(),
                    suspension_id: suspension.id.clone(),
                    expected_suspension_revision: suspension.revision,
                    permission_request_id: "permission".into(),
                    continuation_id: "continuation".into(),
                    worker_id: "worker".into(),
                    worker_generation: "generation-2".into(),
                    lease_token: "token-2".into(),
                    now_ms: 350,
                    lease_until_ms: 1300,
                    max_running_global: 2,
                    max_running_project: 2,
                })
                .map_err(|error| error.to_string())
        })
        .unwrap()
        .unwrap();
    assert!(authority
        .with_active::<()>(|_| panic!("unconfirmed grant executed"))
        .unwrap()
        .is_none());
    let renewed = authority
        .with_renewal(|handle| {
            handle.with_attempt(|current| {
                let next = store
                    .renew_runtime_task_lease(&nola_store::RenewRuntimeTaskLease {
                        task_id: current.task_id.clone(),
                        attempt_id: current.id.clone(),
                        expected_attempt_revision: current.revision,
                        worker_generation: current.worker_generation.clone(),
                        lease_token: current.lease_token.clone(),
                        current_expiry_ms: current.lease_expires_at_ms,
                        now_ms: 400,
                        requested_expiry_ms: 1400,
                    })
                    .map_err(|error| error.to_string())?
                    .ok_or("renewal rejected")?;
                accept_runtime_task_attempt(current, &next, 1400)?;
                Ok(next)
            })
        })
        .unwrap()
        .unwrap();
    assert!(authority
        .with_active::<()>(|_| panic!("renewal implicitly activated grant"))
        .unwrap()
        .is_none());
    assert!(authority
        .activate_permission_resume("foreign", &admitted)
        .is_err());
    let active = authority
        .activate_permission_resume(&suspended.id, &admitted)
        .unwrap();
    assert_eq!(active.snapshot().unwrap(), renewed);
    assert_eq!(old_handle.snapshot().unwrap(), first);
    assert!(!store
        .retry_runtime_task(
            &RuntimeTaskLeaseWitness::from(&first),
            500,
            "old worker",
            400
        )
        .unwrap());
    // A completed pump must settle with its activated successor, not the
    // original handle captured before it waited for permission.
    let terminal = authority
        .active_lease()
        .unwrap()
        .unwrap()
        .snapshot()
        .unwrap();
    assert_eq!(terminal, renewed);
    assert!(!store
        .finish_runtime_task_detailed_with_event(
            &RuntimeTaskLeaseWitness::from(&first),
            "failed",
            Some(7),
            None,
            Some("done"),
            None,
            None,
            true,
            "finished",
            "failed",
            None,
            "{}",
            None,
            500,
        )
        .unwrap());
    assert!(store
        .finish_runtime_task_detailed_with_event(
            &RuntimeTaskLeaseWitness::from(&terminal),
            "failed",
            Some(7),
            None,
            Some("done"),
            None,
            None,
            true,
            "finished",
            "failed",
            None,
            "{}",
            None,
            500,
        )
        .unwrap());
    assert_eq!(
        store.runtime_task(&task_id).unwrap().unwrap().status,
        "failed"
    );
    authority.revoke();
    assert!(authority
        .with_active::<()>(|_| panic!("revoked work executed"))
        .unwrap()
        .is_none());
    release_runtime_task_handle(&active);
}
