// SPDX-License-Identifier: Apache-2.0

use nola_application::{
    execute_application_command, ApplicationCommand, CreateRuntimeTask, NewPermissionRequest,
    PermissionIntervention, PermissionRequest, PermissionRequestCommand,
    PermissionRequestResolution, PermissionRequestStatus, RuntimeTask, RuntimeTaskCommand,
    RuntimeTaskPriority, RuntimeTaskResourceBudget, RuntimeTaskResourceUsage,
    RuntimeTaskScheduling,
};
use nola_store::NolaStore;
use serde_json::json;
use tempfile::tempdir;

#[test]
fn task_cancellation_cannot_be_submitted_as_a_bulk_user_denial() {
    let command = json!({
        "type":"permission_request",
        "command":{"type":"deny_pending_for_task","task_id":"task-1","resolved_at_ms":30}
    });
    assert!(serde_json::from_value::<ApplicationCommand>(command).is_err(),
        "Host cancellation must not synthesize user decisions through a bulk command");
    // Reject the removed operation itself, rather than relying only on outer
    // envelope validation. A user's actual denial still names one request.
    assert!(serde_json::from_value::<PermissionRequestCommand>(json!({
        "type":"deny_pending_for_task","task_id":"task-1","resolved_at_ms":30
    })).is_err());
    assert!(serde_json::from_value::<PermissionRequestCommand>(json!({
        "type":"authorize","request_id":"permission-1",
        "decision":{"type":"deny"},"resolved_at_ms":30
    })).is_ok());
}

fn pending_request() -> NewPermissionRequest {
    NewPermissionRequest {
        core_permission_request_id: Some("core-fixture-request".into()),
        permission_instructions: Vec::new(),
        request_id: "permission-1".into(),
        task_id: "task-1".into(),
        child_session_id: "child-1".into(),
        capability: "write_file".into(),
        input: json!({"path":"/tmp/report.md","content":"hello"}),
        scope_rule: "write_file:/tmp/report.md".into(),
        created_at_ms: 10,
    }
}

fn parent_task() -> RuntimeTask {
    let mut task = RuntimeTask {
        id: "task-1".into(),
        kind: "agent".into(),
        status: "queued".into(),
        title: "Permission owner".into(),
        project_root: "/workspace".into(),
        owner_session_id: Some("parent-session".into()),
        parent_task_id: None,
        parent_tool_call_id: None,
        child_session_id: Some("child-1".into()),
        agent_type: Some("general".into()),
        prompt: Some("write a report".into()),
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
        input_watermark: None,
        completion_watermark: None,
        created_at_ms: 1,
        started_at_ms: None,
        ended_at_ms: None,
        updated_at_ms: 1,
        metadata_json: "{}".into(),
    };
    task.set_scheduling(RuntimeTaskScheduling {
        priority: RuntimeTaskPriority::Normal,
        queued_at_ms: task.created_at_ms,
        resource_budget: RuntimeTaskResourceBudget {
            max_wall_time_ms: Some(60_000),
            max_output_bytes: Some(1_048_576),
            max_cpu_time_ms: None,
            max_memory_bytes: None,
            max_processes: None,
        },
        output_bytes: 0,
        resource_usage: RuntimeTaskResourceUsage::default(),
        budget_exhaustion: None,
    })
    .expect("valid parent runtime task scheduling");
    task
}

#[test]
fn unbound_permissions_close_only_with_exact_worker_terminal_receipt() {
    for terminal in ["completed", "failed", "cancelled"] {
    for approved in [false, true] {
        let directory = tempdir().unwrap();
        let database = directory.path().join("unbound-running-stop.db");
        let mut store = NolaStore::open(&database).unwrap();
        let mut task = parent_task();
        task.kind = "shell_process".into();
        let watermark = task.input_watermark;
        execute_application_command(&mut store, ApplicationCommand::RuntimeTask {
            command: RuntimeTaskCommand::Create { input: CreateRuntimeTask::try_from(task).unwrap() },
        }).unwrap();
        let attempt = store.claim_runtime_task("task-1", "worker", "generation", "token", 20, 1000).unwrap().unwrap();
        execute_application_command(&mut store, ApplicationCommand::PermissionRequest {
            command: PermissionRequestCommand::Enqueue { input: pending_request() },
        }).unwrap();
        if approved {
            execute_application_command(&mut store, ApplicationCommand::PermissionRequest {
                command: PermissionRequestCommand::Authorize { request_id: "permission-1".into(),
                    decision: nola_application::PermissionAuthorizationDecision::AllowConversation, resolved_at_ms: 25 },
            }).unwrap();
            assert_eq!(store.effective_permission_grants(None, Some("task:task-1")).unwrap().len(), 1);
        }
        let before = store.permission_request("permission-1").unwrap().unwrap();
        if terminal == "cancelled" {
            assert!(store.request_runtime_task_cancel("task-1", 30).unwrap());
            assert!(store.effective_permission_grants(None, Some("task:task-1")).unwrap().is_empty());
            assert_late_permission_admission_rejected(&mut store);
        }
        assert_eq!(store.runtime_task("task-1").unwrap().unwrap().status, "running");
        assert_eq!(store.permission_request("permission-1").unwrap().unwrap(), before);
        assert!(store.runtime_task_terminal_receipt("task-1").unwrap().is_none());
        let inspect = rusqlite::Connection::open(&database).unwrap();
        inspect.execute_batch("CREATE TRIGGER reject_terminal_closure BEFORE UPDATE ON permission_requests
            WHEN NEW.closure_json IS NOT NULL BEGIN SELECT RAISE(ABORT,'closure fault'); END").unwrap();
        assert!(store.finish_runtime_task_detailed_with_event(
            &nola_application::RuntimeTaskLeaseWitness::from(&attempt), terminal, watermark,
            None, None, None, None, true, "finished", terminal, None, "{}", None, 40,
        ).is_err());
        assert_eq!(store.runtime_task("task-1").unwrap().unwrap().status, "running");
        assert!(store.runtime_task_terminal_receipt("task-1").unwrap().is_none());
        assert_eq!(store.permission_request("permission-1").unwrap().unwrap(), before);
        assert_eq!(store.effective_permission_grants(None, Some("task:task-1")).unwrap().len(),
            usize::from(approved && terminal != "cancelled"));
        inspect.execute_batch("DROP TRIGGER reject_terminal_closure").unwrap();
        assert!(store.finish_runtime_task_detailed_with_event(
            &nola_application::RuntimeTaskLeaseWitness::from(&attempt), terminal, watermark,
            None, None, None, None, true, "finished", terminal, None, "{}", None, 40,
        ).unwrap());
        drop(store);
        let mut store = NolaStore::open(&database).unwrap();
        let closed = store.permission_request("permission-1").unwrap().unwrap();
        let receipt = store.runtime_task_terminal_receipt("task-1").unwrap().unwrap();
        assert_eq!(receipt.status, terminal);
        assert_eq!(closed.status, "withdrawn");
        assert_eq!(closed.decision, before.decision);
        assert_eq!(closed.resolved_at_ms, before.resolved_at_ms);
        assert!(matches!(&closed.closure.as_ref().unwrap().source,
            nola_domain::permission_closure::PermissionClosureSource::UnboundTaskTerminal { terminal_receipt_id }
                if terminal_receipt_id == &receipt.receipt_id));
        assert!(matches!(receipt.source, nola_application::RuntimeTaskTerminalSource::WorkerFinish { .. }));
        assert!(store.pending_permissions_for_owner("parent-session").unwrap().is_empty());
        assert_late_permission_admission_rejected(&mut store);
        let replay = execute_application_command(&mut store, ApplicationCommand::PermissionRequest {
            command: PermissionRequestCommand::EnsureCoreRequest { input: pending_request() },
        }).unwrap();
        let replay: PermissionRequest = serde_json::from_value(replay).unwrap();
        assert_eq!(replay.status, PermissionRequestStatus::Withdrawn);
        assert_eq!(store.permission_request("permission-1").unwrap().unwrap(), closed);
        let mut conflicting = pending_request();
        conflicting.input = json!({"path":"/tmp/different"});
        assert!(execute_application_command(&mut store, ApplicationCommand::PermissionRequest {
            command: PermissionRequestCommand::EnsureCoreRequest { input: conflicting },
        }).is_err());
        let result = execute_application_command(&mut store, ApplicationCommand::PermissionRequest {
            command: PermissionRequestCommand::Authorize { request_id: "permission-1".into(),
                decision: nola_application::PermissionAuthorizationDecision::AllowConversation, resolved_at_ms: 50 },
        }).unwrap();
        assert!(matches!(serde_json::from_value::<nola_application::PermissionResolutionOutcome>(result).unwrap(),
            nola_application::PermissionResolutionOutcome::Closed { .. }));
        assert_eq!(store.permission_request("permission-1").unwrap().unwrap(), closed);
    }
    }
}

fn assert_late_permission_admission_rejected(store: &mut NolaStore) {
    for ensure in [false, true] {
        let mut input = pending_request();
        input.request_id = "late-request".into();
        input.core_permission_request_id = Some("late-core-occurrence".into());
        input.created_at_ms = 50;
        let command = if ensure {
            PermissionRequestCommand::EnsureCoreRequest { input }
        } else {
            PermissionRequestCommand::Enqueue { input }
        };
        assert!(execute_application_command(store, ApplicationCommand::PermissionRequest { command }).is_err());
        assert!(store.permission_request("late-request").unwrap().is_none());
        assert!(store.effective_permission_grants(None, Some("task:task-1")).unwrap().is_empty());
    }
}

#[test]
fn committed_stop_fences_permission_decision_and_grant_after_reopen() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("stop-permission.db");
    let mut store = NolaStore::open(&database).unwrap();
    execute_application_command(&mut store, ApplicationCommand::RuntimeTask {
        command: RuntimeTaskCommand::Create { input: CreateRuntimeTask::try_from(parent_task()).unwrap() },
    }).unwrap();
    execute_application_command(&mut store, ApplicationCommand::PermissionRequest {
        command: PermissionRequestCommand::Enqueue { input: pending_request() },
    }).unwrap();
    let db = rusqlite::Connection::open(&database).unwrap();
    // Retained Run binding forces cancellation to remain an intent, not a
    // manufactured terminal. Run execution itself is covered by CLI fixtures.
    db.execute("INSERT INTO runtime_task_run_bindings
        (task_id,owner_session_id,child_session_id,run_id,admission_attempt_id,input_watermark,bound_at_ms)
        VALUES('task-1','parent-session','child-1','run','attempt',NULL,10)", []).unwrap();
    assert!(store.request_runtime_task_cancel("task-1", 20).unwrap());
    let before = store.permission_request("permission-1").unwrap();
    drop(store);
    let mut store = NolaStore::open(&database).unwrap();
    use nola_application::PermissionAuthorizationDecision as Decision;
    let mut commands = [Decision::AllowOnce, Decision::AllowConversation, Decision::Deny].into_iter()
        .map(|decision| PermissionRequestCommand::Authorize {
            request_id: "permission-1".into(), decision, resolved_at_ms: 30,
        }).collect::<Vec<_>>();
    commands.push(PermissionRequestCommand::Intervene {
        request_id: "permission-1".into(),
        intervention: PermissionIntervention::ProvideInstruction { instruction: "changed scope".into() },
        resolved_at_ms: 30,
    });
    for command in commands {
        let result = execute_application_command(&mut store, ApplicationCommand::PermissionRequest { command }).unwrap();
        assert_eq!(result, json!({"type":"stop_pending", "request_id":"permission-1",
                                 "task_id":"task-1", "requested_at_ms":20}));
        assert_eq!(store.permission_request("permission-1").unwrap(), before);
        assert!(store.effective_permission_grants(None, Some("task:task-1")).unwrap().is_empty());
    }
}

#[test]
fn concurrent_stop_and_scope_authorization_have_one_serialized_result() {
    for iteration in 0..8 {
        let directory = tempdir().unwrap();
        let database = directory.path().join("permission-race.db");
        let mut store = NolaStore::open(&database).unwrap();
        execute_application_command(&mut store, ApplicationCommand::RuntimeTask {
            command: RuntimeTaskCommand::Create { input: CreateRuntimeTask::try_from(parent_task()).unwrap() },
        }).unwrap();
        execute_application_command(&mut store, ApplicationCommand::PermissionRequest {
            command: PermissionRequestCommand::Enqueue { input: pending_request() },
        }).unwrap();
        let db = rusqlite::Connection::open(&database).unwrap();
        db.execute("INSERT INTO runtime_task_run_bindings
            (task_id,owner_session_id,child_session_id,run_id,admission_attempt_id,input_watermark,bound_at_ms)
            VALUES('task-1','parent-session','child-1','run','attempt',NULL,10)", []).unwrap();
        drop(db);
        drop(store);
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let workers = (0..2).map(|side| {
            let path = database.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let mut store = NolaStore::open(path).unwrap();
                barrier.wait();
                if side == 0 {
                    execute_application_command(&mut store, ApplicationCommand::RuntimeTask {
                        command: RuntimeTaskCommand::RequestCancel { task_id: "task-1".into(), now_ms: 20 },
                    })
                } else {
                    execute_application_command(&mut store, ApplicationCommand::PermissionRequest {
                        command: PermissionRequestCommand::Authorize {
                            request_id: "permission-1".into(),
                            decision: nola_application::PermissionAuthorizationDecision::AllowConversation,
                            resolved_at_ms: 20,
                        },
                    })
                }
            })
        }).collect::<Vec<_>>();
        let results = workers.into_iter().map(|worker| worker.join().unwrap().unwrap()).collect::<Vec<_>>();
        assert_eq!(results[0], json!(true), "iteration {iteration}");
        let store = NolaStore::open(&database).unwrap();
        let request = store.permission_request("permission-1").unwrap().unwrap();
        let grants = store.effective_permission_grants(None, Some("task:task-1")).unwrap();
        match results[1]["type"].as_str().unwrap() {
            "resolved" => {
                assert_eq!(request.decision.as_deref(), Some("allow_conversation"));
                assert!(grants.is_empty(), "later stop must revoke the winning approval's grant");
                let db = rusqlite::Connection::open(&database).unwrap();
                let revoked: Option<i64> = db.query_row("SELECT revoked_at_ms FROM permission_grants
                    WHERE id='permission-request:permission-1:task-scope'", [], |row| row.get(0)).unwrap();
                assert_eq!(revoked, Some(20), "grant history must survive stop");
            }
            "stop_pending" => {
                assert_eq!(request.status, "pending");
                assert!(request.decision.is_none());
                assert!(grants.is_empty(), "losing approval must not publish a grant");
                assert_eq!(results[1]["requested_at_ms"], 20);
            }
            other => panic!("unexpected arbitration outcome: {other}"),
        }
        let task = store.runtime_task("task-1").unwrap().unwrap();
        assert!(task.cancel_requested);
        assert_eq!(task.status, "queued");
        assert!(store.runtime_task_terminal_receipt("task-1").unwrap().is_none());
    }
}

#[test]
fn stop_revokes_only_derived_grant_atomically_and_preserves_decision() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("revoke-on-stop.db");
    let mut store = NolaStore::open(&database).unwrap();
    let forged_grant = nola_store_port::permission::UpsertPermissionGrantInput {
        id:"permission-request:permission-1:task-scope".into(),
        scope:nola_store_port::permission::PermissionGrantScope::Host,
        conversation_id:None, host_id:Some("task:task-1".into()),
        capability_pattern:"cli::tool::allow_rule".into(), resource_pattern:"write_file:/tmp/report.md".into(),
        decision:nola_store_port::permission::StoredPermissionDecision::Allow,
        created_at_ms:15, updated_at_ms:15,
    };
    assert!(store.upsert_permission_grant(&forged_grant).unwrap_err().to_string().contains("permission decision"));
    execute_application_command(&mut store, ApplicationCommand::RuntimeTask {
        command: RuntimeTaskCommand::Create { input: CreateRuntimeTask::try_from(parent_task()).unwrap() },
    }).unwrap();
    execute_application_command(&mut store, ApplicationCommand::PermissionRequest {
        command: PermissionRequestCommand::Enqueue { input: pending_request() },
    }).unwrap();
    execute_application_command(&mut store, ApplicationCommand::PermissionRequest {
        command: PermissionRequestCommand::Authorize { request_id:"permission-1".into(),
            decision:nola_application::PermissionAuthorizationDecision::AllowConversation, resolved_at_ms:15 },
    }).unwrap();
    let original_request = store.permission_request("permission-1").unwrap();
    let db = rusqlite::Connection::open(&database).unwrap();
    db.execute("INSERT INTO runtime_task_run_bindings
        (task_id,owner_session_id,child_session_id,run_id,admission_attempt_id,input_watermark,bound_at_ms)
        VALUES('task-1','parent-session','child-1','run','attempt',NULL,10)", []).unwrap();
    // An independent grant with the same Host scope is not request-owned.
    db.execute("INSERT INTO permission_grants
        SELECT 'independent',scope,conversation_id,host_id,capability_pattern,resource_pattern,
        decision,created_at_ms,updated_at_ms,revoked_at_ms FROM permission_grants
        WHERE id='permission-request:permission-1:task-scope'", []).unwrap();
    db.execute_batch("CREATE TRIGGER reject_revoke BEFORE UPDATE ON permission_grants
        BEGIN SELECT RAISE(ABORT,'fixture revoke failure'); END;").unwrap();
    let original_task = store.runtime_task("task-1").unwrap();
    assert!(store.request_runtime_task_cancel("task-1",20).is_err());
    assert_eq!(store.runtime_task("task-1").unwrap(), original_task);
    assert_eq!(store.effective_permission_grants(None,Some("task:task-1")).unwrap().len(),2);
    db.execute_batch("DROP TRIGGER reject_revoke;").unwrap();
    assert!(store.request_runtime_task_cancel("task-1",20).unwrap());
    drop(store);
    let mut store = NolaStore::open(&database).unwrap();
    assert_eq!(store.permission_request("permission-1").unwrap(),original_request);
    assert!(store.upsert_permission_grant(&forged_grant).unwrap_err().to_string().contains("permission decision"));
    let grants = store.effective_permission_grants(None,Some("task:task-1")).unwrap();
    assert_eq!(grants.len(),1);
    assert_eq!(grants[0].id,"independent");
    assert!(store.runtime_task("task-1").unwrap().unwrap().cancel_requested);
    assert!(store.runtime_task_terminal_receipt("task-1").unwrap().is_none());
}

#[test]
fn permission_request_survives_cold_reopen_and_preserves_typed_resolution() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("permission-request-port.db");
    {
        let mut store = NolaStore::open(&database).unwrap();
        execute_application_command(
            &mut store,
            ApplicationCommand::RuntimeTask {
                command: RuntimeTaskCommand::Create {
                    input: CreateRuntimeTask::try_from(parent_task())
                        .expect("create parent runtime task input"),
                },
            },
        )
        .unwrap();
        execute_application_command(
            &mut store,
            ApplicationCommand::PermissionRequest {
                command: PermissionRequestCommand::Enqueue {
                    input: pending_request(),
                },
            },
        )
        .unwrap();
    }

    let mut reopened = NolaStore::open(&database).unwrap();
    let pending = execute_application_command(
        &mut reopened,
        ApplicationCommand::PermissionRequest {
            command: PermissionRequestCommand::Get {
                request_id: "permission-1".into(),
            },
        },
    )
    .unwrap();
    let pending: Option<PermissionRequest> = serde_json::from_value(pending).unwrap();
    let pending = pending.unwrap();
    assert_eq!(pending.status, PermissionRequestStatus::Pending);
    assert_eq!(pending.input["path"], "/tmp/report.md");
    assert_eq!(pending.resolution, None);

    execute_application_command(
        &mut reopened,
        ApplicationCommand::PermissionRequest {
            command: PermissionRequestCommand::Intervene {
                request_id: "permission-1".into(),
                intervention: PermissionIntervention::ProvideInstruction {
                    instruction: "write it under /tmp/approved instead".into(),
                },
                resolved_at_ms: 20,
            },
        },
    )
    .unwrap();
    drop(reopened);

    let mut reopened = NolaStore::open(&database).unwrap();
    let resolved = execute_application_command(
        &mut reopened,
        ApplicationCommand::PermissionRequest {
            command: PermissionRequestCommand::Get {
                request_id: "permission-1".into(),
            },
        },
    )
    .unwrap();
    let resolved: Option<PermissionRequest> = serde_json::from_value(resolved).unwrap();
    assert_eq!(
        resolved.unwrap().resolution,
        Some(PermissionRequestResolution::Intervention {
            intervention: PermissionIntervention::ProvideInstruction {
                instruction: "write it under /tmp/approved instead".into(),
            },
        })
    );
}

#[test]
fn return_to_composer_resolution_survives_cold_reopen() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("permission-return-to-composer.db");
    let mut store = NolaStore::open(&database).unwrap();
    execute_application_command(
        &mut store,
        ApplicationCommand::RuntimeTask {
            command: RuntimeTaskCommand::Create {
                input: CreateRuntimeTask::try_from(parent_task())
                    .expect("create parent runtime task input"),
            },
        },
    )
    .unwrap();
    execute_application_command(
        &mut store,
        ApplicationCommand::PermissionRequest {
            command: PermissionRequestCommand::Enqueue {
                input: pending_request(),
            },
        },
    )
    .unwrap();
    execute_application_command(
        &mut store,
        ApplicationCommand::PermissionRequest {
            command: PermissionRequestCommand::Intervene {
                request_id: "permission-1".into(),
                intervention: PermissionIntervention::ReturnToComposer,
                resolved_at_ms: 20,
            },
        },
    )
    .unwrap();
    drop(store);

    let mut reopened = NolaStore::open(&database).unwrap();
    let value = execute_application_command(
        &mut reopened,
        ApplicationCommand::PermissionRequest {
            command: PermissionRequestCommand::Get {
                request_id: "permission-1".into(),
            },
        },
    )
    .unwrap();
    let request: Option<PermissionRequest> = serde_json::from_value(value).unwrap();
    let request = request.unwrap();
    assert_eq!(request.status, PermissionRequestStatus::Resolved);
    assert_eq!(
        request.resolution,
        Some(PermissionRequestResolution::Intervention {
            intervention: PermissionIntervention::ReturnToComposer,
        })
    );
}

#[test]
fn permission_command_rejects_persistence_representation() {
    let raw = json!({
        "type": "permission_request",
        "command": {
            "type": "enqueue",
            "input": {
                "request_id": "permission-1",
                "task_id": "task-1",
                "child_session_id": "child-1",
                "capability": "write_file",
                "input": {"path":"/tmp/report.md"},
                "input_json": "{\"path\":\"/tmp/report.md\"}",
                "scope_rule": "write_file:/tmp/report.md",
                "created_at_ms": 10
            }
        }
    });
    assert!(serde_json::from_value::<ApplicationCommand>(raw).is_err());
}

mod support;

fn setup_scope_resolution(store: &mut NolaStore) {
    for task_id in ["task-1", "task-other"] {
        let mut task = parent_task();
        task.id = task_id.into();
        execute_application_command(store, ApplicationCommand::RuntimeTask {
            command: RuntimeTaskCommand::Create { input: CreateRuntimeTask::try_from(task).unwrap() },
        }).unwrap();
    }
    for id in ["permission-1", "same-scope", "other-resource", "other-capability", "other-task", "other-child", "other-scope", "with-guidance"] {
        let mut input = pending_request();
        input.request_id = id.into();
        input.core_permission_request_id = Some(format!("core-{id}"));
        input.scope_rule = json!({"version":2,"permission":"write","resource":"/tmp/report.md","tool":"write_file"}).to_string();
        match id {
            "other-resource" => input.input = json!({"path":"/tmp/different.md"}),
            "other-capability" => input.capability = "read_file".into(),
            "other-task" => input.task_id = "task-other".into(),
            "other-child" => input.child_session_id = "child-other".into(),
            "other-scope" => input.scope_rule = "different-scope".into(),
            "with-guidance" => input.permission_instructions.push(nola_host_protocol::PermissionInstruction {
                permission_request_id: "earlier-core-wait".into(), instruction: "Preserve comments".into(),
            }),
            _ => {}
        }
        execute_application_command(store, ApplicationCommand::PermissionRequest {
            command: PermissionRequestCommand::Enqueue { input },
        }).unwrap();
    }
}

fn scope_decision(request_id: &str, allow: bool, at: i64) -> ApplicationCommand {
    ApplicationCommand::PermissionRequest { command: PermissionRequestCommand::Authorize {
        request_id: request_id.into(),
        decision: if allow { nola_application::PermissionAuthorizationDecision::AllowConversation }
            else { nola_application::PermissionAuthorizationDecision::Deny },
        resolved_at_ms: at,
    } }
}

#[test]
fn scope_resolution_is_occurrence_bound_and_replay_never_revives_grants_or_sibling_waits() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("scope-resolution.db");
    let mut store = NolaStore::open(&database).unwrap();
    setup_scope_resolution(&mut store);
    let first = execute_application_command(&mut store, scope_decision("permission-1", true, 100)).unwrap();
    assert_eq!(first["type"], "resolved");
    assert_eq!(first["receipt"]["request_id"], "permission-1");
    assert_eq!(first["receipt"]["task_id"], "task-1");
    assert_eq!(first["receipt"]["child_session_id"], "child-1");
    assert_eq!(first["receipt"]["core_permission_request_id"], "core-permission-1");
    assert_eq!(first["receipt"]["resolved_at_ms"], 100);
    assert!(first["receipt"].get("input").is_none());
    for id in ["same-scope", "other-resource", "other-capability", "other-task", "other-child", "other-scope", "with-guidance"] {
        let sibling = store.permission_request(id).unwrap().unwrap();
        assert_eq!(sibling.status, "pending");
        assert!(sibling.decision.is_none(), "Host scope equality is not Core authority");
        assert!(sibling.resolved_at_ms.is_none());
    }
    let grants = store.effective_permission_grants(None, Some("task:task-1")).unwrap();
    assert_eq!(grants.len(), 1);
    store.revoke_permission_grant(&grants[0].id, 101).unwrap();
    let mut late = pending_request();
    late.request_id = "late-wait".into();
    late.core_permission_request_id = Some("late-core".into());
    late.scope_rule = grants[0].resource_pattern.clone();
    execute_application_command(&mut store, ApplicationCommand::PermissionRequest {
        command: PermissionRequestCommand::Enqueue { input: late },
    }).unwrap();
    drop(store);
    let mut store = NolaStore::open(&database).unwrap();
    let replay = execute_application_command(&mut store, scope_decision("permission-1", true, 999)).unwrap();
    assert_eq!(replay["type"], "already_resolved");
    assert_eq!(replay["receipt"], first["receipt"], "first actual time and decision remain immutable");
    let conflict = execute_application_command(&mut store, scope_decision("permission-1", false, 1000)).unwrap();
    assert_eq!(conflict["type"], "conflict");
    assert_eq!(conflict["receipt"], first["receipt"]);
    assert!(store.effective_permission_grants(None, Some("task:task-1")).unwrap().is_empty());
    assert_eq!(store.permission_request("same-scope").unwrap().unwrap().status, "pending");
    assert_eq!(store.permission_request("late-wait").unwrap().unwrap().status, "pending");
    let missing = execute_application_command(&mut store, scope_decision("missing", true, 1001)).unwrap();
    assert_eq!(missing, json!({"type":"missing","request_id":"missing"}));
}

#[test]
fn scope_resolution_failure_rolls_back_original_siblings_and_grant() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("scope-rollback.db");
    let mut store = NolaStore::open(&database).unwrap();
    setup_scope_resolution(&mut store);
    let inspection = rusqlite::Connection::open(&database).unwrap();
    // The original is written after the derived selection record;
    // force that last step to fail to expose any partial commit.
    inspection.execute_batch("CREATE TRIGGER reject_scope_original BEFORE UPDATE ON permission_requests WHEN NEW.request_id='permission-1' AND NEW.status='resolved' BEGIN SELECT RAISE(ABORT,'scope-write-fault'); END;").unwrap();
    assert!(execute_application_command(&mut store, scope_decision("permission-1", true, 100)).is_err());
    drop(store);
    let store = NolaStore::open(&database).unwrap();
    for id in ["permission-1", "same-scope"] {
        let request = store.permission_request(id).unwrap().unwrap();
        assert_eq!(request.status, "pending");
        assert!(request.decision.is_none());
        assert!(request.resolved_at_ms.is_none());
    }
    assert!(store.effective_permission_grants(None, Some("task:task-1")).unwrap().is_empty());
}

#[test]
fn first_scope_resolution_cannot_commit_without_its_matching_atomic_grant() {
    use nola_store_port::permission::{PermissionGrantScope, StoredPermissionDecision, UpsertPermissionGrantInput};
    let directory = tempdir().unwrap();
    let mut store = NolaStore::open(directory.path().join("scope-missing-grant.db")).unwrap();
    setup_scope_resolution(&mut store);
    // Models the stale pre-read / concurrent first enqueue boundary: the
    // authoritative Store sees a pending row but Application supplied no grant.
    assert!(store.resolve_permission_request("permission-1", "allow_conversation", None, 100, None).is_err());
    let request = store.permission_request("permission-1").unwrap().unwrap();
    let grant = UpsertPermissionGrantInput {
        id: "permission-request:permission-1:task-scope".into(),
        scope: PermissionGrantScope::Host, conversation_id: None, host_id: Some("task:task-1".into()),
        capability_pattern: "cli::tool::allow_rule".into(), resource_pattern: request.scope_rule,
        decision: StoredPermissionDecision::Allow, created_at_ms: 99, updated_at_ms: 100,
    };
    assert!(store.resolve_permission_request("permission-1", "allow_conversation", None, 100, Some(&grant)).is_err());
    assert_eq!(store.permission_request("permission-1").unwrap().unwrap().status, "pending");
    assert!(store.effective_permission_grants(None, Some("task:task-1")).unwrap().is_empty());
}

#[test]
fn permission_command_receipt_replay_and_new_transport_id_have_distinct_outcomes() {
    use nola_application::{ApplicationCommandReceiptOutcome, ExecuteReceivedCommand};
    let directory = tempdir().unwrap();
    let database = directory.path().join("resolution-request-identities.db");
    let mut store = NolaStore::open(&database).unwrap();
    setup_scope_resolution(&mut store);
    drop(store);
    let mut runtime = support::open_runtime(&database).unwrap();
    let mut submit = |transport_id: &str, allow: bool, at: i64| {
        let command = scope_decision("permission-1", allow, at);
        let envelope = ExecuteReceivedCommand {
            client_id: "permission-owner".into(), request_id: transport_id.into(),
            command_type: "permission_request".into(), command_json: serde_json::to_string(&command).unwrap(),
            expected_revision: None, received_at: "2026-10-12T00:00:00Z".into(),
        };
        match runtime.execute_received_command(envelope, command).unwrap().outcome {
            ApplicationCommandReceiptOutcome::Committed { result, .. } => result,
            other => panic!("resolution command was not committed: {other:?}"),
        }
    };
    let first = submit("first-transport", true, 100);
    assert_eq!(first["type"], "resolved");
    assert_eq!(submit("first-transport", true, 100), first);
    let replay = submit("new-transport", true, 200);
    assert_eq!(replay["type"], "already_resolved");
    assert_eq!(replay["receipt"], first["receipt"]);
    let conflict = submit("conflicting-transport", false, 300);
    assert_eq!(conflict["type"], "conflict");
    assert_eq!(conflict["receipt"], first["receipt"]);
}

#[test]
fn permission_resolution_rolls_back_when_receipt_commit_fails_then_replays() {
    use nola_application::{ApplicationCommandReceiptOutcome, ExecuteReceivedCommand};
    let directory = tempdir().unwrap();
    let database = directory.path().join("permission-receipt-rollback.db");
    let mut store = NolaStore::open(&database).unwrap();
    execute_application_command(
        &mut store,
        ApplicationCommand::RuntimeTask {
            command: RuntimeTaskCommand::Create {
                input: CreateRuntimeTask::try_from(parent_task()).unwrap(),
            },
        },
    )
    .unwrap();
    execute_application_command(
        &mut store,
        ApplicationCommand::PermissionRequest {
            command: PermissionRequestCommand::Enqueue {
                input: pending_request(),
            },
        },
    )
    .unwrap();
    drop(store);
    let mut runtime = support::open_runtime(&database).unwrap();
    let inspect = rusqlite::Connection::open(&database).unwrap();
    inspect.execute_batch("CREATE TRIGGER fail_permission_receipt BEFORE UPDATE ON command_receipts WHEN NEW.status='committed' BEGIN SELECT RAISE(ABORT,'permission receipt fault'); END;").unwrap();
    let command = ApplicationCommand::PermissionRequest {
        command: PermissionRequestCommand::Intervene {
            request_id: "permission-1".into(),
            intervention: PermissionIntervention::ProvideInstruction {
                instruction: "Preserve formatting".into(),
            },
            resolved_at_ms: 20,
        },
    };
    let envelope = ExecuteReceivedCommand {
        client_id: "permission-test".into(),
        request_id: "resolve".into(),
        command_type: "permission_request".into(),
        command_json: serde_json::to_string(&command).unwrap(),
        expected_revision: None,
        received_at: "2026-09-08T00:00:00Z".into(),
    };
    assert!(runtime
        .execute_received_command(
            envelope.clone(),
            serde_json::from_str(&envelope.command_json).unwrap()
        )
        .is_err());
    let store = NolaStore::open(&database).unwrap();
    let pending = store.permission_request("permission-1").unwrap().unwrap();
    assert_eq!(pending.status, "pending");
    assert_eq!(pending.decision, None);
    assert_eq!(pending.resolved_at_ms, None);
    drop(store);
    assert_eq!(
        inspect
            .query_row("SELECT COUNT(*) FROM command_receipts", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    inspect
        .execute_batch("DROP TRIGGER fail_permission_receipt")
        .unwrap();
    let committed = runtime
        .execute_received_command(
            envelope.clone(),
            serde_json::from_str(&envelope.command_json).unwrap(),
        )
        .unwrap();
    assert!(matches!(
        committed.outcome,
        ApplicationCommandReceiptOutcome::Committed { .. }
    ));
    drop(runtime);
    let mut runtime = support::open_runtime(&database).unwrap();
    let replay = runtime.execute_received_command(envelope, command).unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.outcome, committed.outcome);
    let store = NolaStore::open(&database).unwrap();
    assert_eq!(
        store
            .permission_request("permission-1")
            .unwrap()
            .unwrap()
            .decision
            .as_deref(),
        Some("provide_instruction")
    );
}

#[test]
fn custom_budget_rejection_keeps_transport_pending_and_legacy_requires_explicit_decision() {
    use nola_host_protocol::PermissionInstruction;
    for case in 0..4 {
        let mut store = NolaStore::open(":memory:").unwrap();
        execute_application_command(
            &mut store,
            ApplicationCommand::RuntimeTask {
                command: RuntimeTaskCommand::Create {
                    input: CreateRuntimeTask::try_from(parent_task()).unwrap(),
                },
            },
        )
        .unwrap();
        let mut input = pending_request();
        let instruction = match case {
            0 => "x".repeat(4097),
            1 => {
                input.permission_instructions = (0..16)
                    .map(|i| PermissionInstruction {
                        permission_request_id: format!("core-{i}"),
                        instruction: "x".into(),
                    })
                    .collect();
                "next".into()
            }
            2 => {
                input.permission_instructions = (0..3)
                    .map(|i| PermissionInstruction {
                        permission_request_id: format!("core-{i}"),
                        instruction: "x".repeat(4096),
                    })
                    .collect();
                "x".repeat(4096)
            }
            _ => {
                input.core_permission_request_id = None;
                "legacy instruction".into()
            }
        };
        execute_application_command(
            &mut store,
            ApplicationCommand::PermissionRequest {
                command: PermissionRequestCommand::Enqueue { input },
            },
        )
        .unwrap();
        let before = store.permission_request("permission-1").unwrap().unwrap();
        assert!(execute_application_command(
            &mut store,
            ApplicationCommand::PermissionRequest {
                command: PermissionRequestCommand::Intervene {
                    request_id: "permission-1".into(),
                    intervention: PermissionIntervention::ProvideInstruction {
                        instruction: instruction.clone()
                    },
                    resolved_at_ms: 20
                },
            }
        )
        .is_err());
        assert!(store
            .resolve_permission_request(
                "permission-1",
                "provide_instruction",
                Some(&instruction),
                20, None
            )
            .is_err());
        let after = store.permission_request("permission-1").unwrap().unwrap();
        assert_eq!(
            serde_json::to_value(&before).unwrap(),
            serde_json::to_value(&after).unwrap()
        );
        assert!(matches!(store
            .resolve_permission_request("permission-1", "allow_once", None, 21, None)
            .unwrap(), nola_store_port::permission::StoredPermissionResolutionOutcome::Resolved(_)));
    }
}

#[test]
fn ensure_core_permission_is_atomic_and_recovers_resolved_undelivered_response() {
    use nola_application::{ApplicationCommandReceiptOutcome, ExecuteReceivedCommand};
    let directory = tempdir().unwrap();
    let database = directory.path().join("ensure-permission.db");
    let mut store = NolaStore::open(&database).unwrap();
    execute_application_command(
        &mut store,
        ApplicationCommand::RuntimeTask {
            command: RuntimeTaskCommand::Create {
                input: CreateRuntimeTask::try_from(parent_task()).unwrap(),
            },
        },
    )
    .unwrap();
    drop(store);
    let input = pending_request();
    let command = ApplicationCommand::PermissionRequest {
        command: PermissionRequestCommand::EnsureCoreRequest {
            input: input.clone(),
        },
    };
    let envelope = ExecuteReceivedCommand {
        client_id: "ensure-test".into(),
        request_id: "ensure-first".into(),
        command_type: "permission_request".into(),
        command_json: serde_json::to_string(&command).unwrap(),
        expected_revision: None,
        received_at: "1000".into(),
    };
    let inspect = rusqlite::Connection::open(&database).unwrap();
    inspect.execute_batch("CREATE TRIGGER fail_ensure_receipt BEFORE UPDATE ON command_receipts WHEN NEW.status='committed' BEGIN SELECT RAISE(ABORT,'ensure fault'); END").unwrap();
    let mut runtime = support::open_runtime(&database).unwrap();
    assert!(runtime
        .execute_received_command(
            envelope.clone(),
            serde_json::from_str(&envelope.command_json).unwrap()
        )
        .is_err());
    assert_eq!(
        inspect
            .query_row("SELECT COUNT(*) FROM permission_requests", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    inspect
        .execute_batch("DROP TRIGGER fail_ensure_receipt")
        .unwrap();
    let committed = runtime
        .execute_received_command(
            envelope.clone(),
            serde_json::from_str(&envelope.command_json).unwrap(),
        )
        .unwrap();
    assert!(matches!(
        committed.outcome,
        ApplicationCommandReceiptOutcome::Committed { .. }
    ));
    drop(runtime);
    let mut runtime = support::open_runtime(&database).unwrap();
    let replay = runtime
        .execute_received_command(
            envelope.clone(),
            serde_json::from_str(&envelope.command_json).unwrap(),
        )
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.outcome, committed.outcome);
    drop(runtime);
    let pending_attempts: Vec<_> = (0..2)
        .map(|index| {
            let path = database.clone();
            let mut next = input.clone();
            next.request_id = format!("pending-attempt-{index}");
            std::thread::spawn(move || {
                let mut store = NolaStore::open(path).unwrap();
                execute_application_command(
                    &mut store,
                    ApplicationCommand::PermissionRequest {
                        command: PermissionRequestCommand::EnsureCoreRequest { input: next },
                    },
                )
                .unwrap()
            })
        })
        .collect();
    for handle in pending_attempts {
        let pending = handle.join().unwrap();
        assert_eq!(pending["request_id"], input.request_id);
        assert_eq!(pending["status"], "pending");
    }
    let mut store = NolaStore::open(&database).unwrap();
    assert!(matches!(store
        .resolve_permission_request(
            &input.request_id,
            "provide_instruction",
            Some("Keep formatting"),
            2000, None
        )
        .unwrap(), nola_store_port::permission::StoredPermissionResolutionOutcome::Resolved(_)));
    drop(store);
    // Independent delivery attempts race with fresh transport IDs after restart.
    let handles: Vec<_> = (0..2)
        .map(|index| {
            let path = database.clone();
            let mut next = input.clone();
            next.request_id = format!("attempt-{index}");
            next.created_at_ms += 100;
            std::thread::spawn(move || {
                let mut store = NolaStore::open(path).unwrap();
                execute_application_command(
                    &mut store,
                    ApplicationCommand::PermissionRequest {
                        command: PermissionRequestCommand::EnsureCoreRequest { input: next },
                    },
                )
                .unwrap()
            })
        })
        .collect();
    for handle in handles {
        let result = handle.join().unwrap();
        assert_eq!(result["request_id"], input.request_id);
        assert_eq!(result["status"], "resolved");
        assert_eq!(
            result["resolution"]["intervention"]["instruction"],
            "Keep formatting"
        );
    }
    assert_eq!(
        inspect
            .query_row("SELECT COUNT(*) FROM permission_requests", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    for field in 0..5 {
        let mut changed = input.clone();
        changed.request_id = format!("conflict-{field}");
        match field {
            0 => changed.child_session_id = "other".into(),
            1 => changed.capability = "other".into(),
            2 => changed.scope_rule = "other".into(),
            3 => changed.input = serde_json::json!({"changed":true}),
            _ => changed
                .permission_instructions
                .push(nola_host_protocol::PermissionInstruction {
                    permission_request_id: "previous".into(),
                    instruction: "different".into(),
                }),
        }
        let mut store = NolaStore::open(&database).unwrap();
        assert!(execute_application_command(
            &mut store,
            ApplicationCommand::PermissionRequest {
                command: PermissionRequestCommand::EnsureCoreRequest { input: changed }
            }
        )
        .is_err());
    }
    assert_eq!(
        inspect
            .query_row("SELECT COUNT(*) FROM permission_requests", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
}
