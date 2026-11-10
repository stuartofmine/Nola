// SPDX-License-Identifier: Apache-2.0
//! Real source acceptance precedes the independent Task-owner delivery ACK.
use super::*;
use nola_core::agent::host::*;
use serde_json::json;
#[path = "../../../core/nola-application/tests/support/mod.rs"]
mod source_support;

fn echo(
    request: &HostRequestEnvelope,
    sequence: u64,
    payload: HostResponse,
) -> HostResponseEnvelope {
    HostResponseEnvelope {
        request_id: request.request_id.clone(),
        attempt_id: request.attempt_id.clone(),
        delegated_invocation_id: request.delegated_invocation_id.clone(),
        effect_id: request.effect_id.clone(),
        host_id: request.host_id.clone(),
        lease_id: request.lease_id.clone(),
        session_id: request.session_id.clone(),
        turn_id: request.turn_id.clone(),
        catalog_revision: request.catalog_revision,
        availability_epoch: request.availability_epoch,
        delivery: request.delivery.clone(),
        sequence,
        payload,
    }
}

fn claim(source: &mut SessionStore) -> HostRequestEnvelope {
    let now = monotonic_epoch_millis();
    let requests = source
        .claim_committed_requests(RuntimeEffectDeliveryClaim {
            worker_generation: "delivery-source".into(),
            lease_owner: "source".into(),
            claimed_at_ms: now,
            lease_expires_at_ms: now + 60000,
            limit: 16,
        })
        .unwrap();
    assert_eq!(requests.len(), 1);
    requests.into_iter().next().unwrap()
}

fn contains_delivery(
    value: &Value,
    cursor: &nola_application::AcknowledgeRuntimeTaskResult,
) -> bool {
    match value {
        Value::Object(object) => {
            (object.get("type").and_then(Value::as_str) == Some("runtime_task_result_delivery")
                && object.get("cursor") == Some(&serde_json::to_value(cursor).unwrap()))
                || object
                    .values()
                    .any(|value| contains_delivery(value, cursor))
        }
        Value::Array(values) => values.iter().any(|value| contains_delivery(value, cursor)),
        _ => false,
    }
}

#[test]
fn committed_task_birth_recovers_after_target_failure_and_source_reopen() {
    let root = tempfile::tempdir().unwrap();
    let source_path = root.path().join("birth-source.sqlite");
    let target_path = root.path().join("birth-target.sqlite");
    let mut source_app = source_support::open_runtime(&source_path).unwrap();
    let fact = source_support::agent_task_admission::source_fact(&mut source_app);
    let mut target_app = source_support::open_runtime(&target_path).unwrap();
    source_support::agent_task_admission::draft(&mut target_app, None);
    let mut source = CliApplicationPort::InProcess(source_app);
    let mut target = CliApplicationPort::InProcess(target_app);
    let original_checkpoint = source
        .load_checkpoint("parent")
        .unwrap()
        .unwrap()
        .checkpoint_json;
    let db = rusqlite::Connection::open(&target_path).unwrap();
    db.execute_batch(
        "CREATE TRIGGER reject_birth BEFORE UPDATE ON runtime_agent_task_authorities
        WHEN NEW.phase='admitted' BEGIN SELECT RAISE(ABORT,'birth import fault'); END;",
    )
    .unwrap();
    assert!(source
        .recover_prepared_agent_tasks("parent", &mut target)
        .is_err());
    assert_eq!(
        nola_store::NolaStore::open(&target_path)
            .unwrap()
            .runtime_agent_task_authority("task")
            .unwrap()
            .unwrap()
            .phase,
        nola_domain::agent_task_admission::RuntimeAgentTaskAuthorityPhase::Prepared
    );
    drop(source);
    drop(target);
    db.execute_batch("DROP TRIGGER reject_birth").unwrap();
    let mut source =
        CliApplicationPort::InProcess(source_support::open_runtime(&source_path).unwrap());
    let mut target =
        CliApplicationPort::InProcess(source_support::open_runtime(&target_path).unwrap());
    assert_eq!(
        source
            .recover_prepared_agent_tasks("foreign", &mut target)
            .unwrap(),
        0
    );
    assert_eq!(
        source
            .recover_prepared_agent_tasks("parent", &mut target)
            .unwrap(),
        1
    );
    assert_eq!(
        source
            .recover_prepared_agent_tasks("parent", &mut target)
            .unwrap(),
        0
    );
    assert!(
        source
            .recover_agent_task_admission("parent", "task", &mut target)
            .unwrap(),
        "an exact lost-ACK retry reimports the original immutable proof"
    );
    let store = nola_store::NolaStore::open(&target_path).unwrap();
    let admitted = store.runtime_agent_task_authority("task").unwrap().unwrap();
    assert_eq!(admitted.admission.as_ref(), Some(&fact));
    assert_eq!(admitted.authority_revision, 2);
    let attempt_count: i64 = db
        .query_row("SELECT COUNT(*) FROM runtime_task_attempts", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(
        attempt_count, 0,
        "recovery grants admission but never spawns a worker"
    );
    assert_eq!(
        source
            .load_checkpoint("parent")
            .unwrap()
            .unwrap()
            .checkpoint_json,
        original_checkpoint,
        "recovery must not redispatch the committed spawn or change Core state"
    );
    // The admitted child can fairly claim its own slot while the parent Run
    // remains active; the foreground Goal is not a Task-worker slot owner.
    let now = monotonic_epoch_millis();
    let claimed: Option<nola_application::RuntimeTaskAttempt> = target.execute(
        nola_application::ApplicationCommand::ClaimAgentTaskFair {
            input: nola_application::ClaimAgentTaskFair {
                owner_conversation_id: "parent".into(), task_id: "task".into(),
                worker_id: "child-worker".into(), worker_generation: "child-generation".into(),
                lease_token: "child-lease".into(), now_ms: now, lease_until_ms: now + 60_000,
                max_running_global: 1, max_running_project: 1,
            },
        },
    ).unwrap();
    assert!(claimed.is_some(), "committed child admission must be claimable without an idle parent");
}

#[test]
fn missing_original_source_does_not_admit_prepared_work() {
    let root = tempfile::tempdir().unwrap();
    let target_path = root.path().join("birth-target.sqlite");
    let mut target_app = source_support::open_runtime(&target_path).unwrap();
    source_support::agent_task_admission::draft(&mut target_app, None);
    let mut target = CliApplicationPort::InProcess(target_app);
    let mut missing_source = CliApplicationPort::ephemeral().unwrap();
    assert_eq!(
        missing_source
            .recover_prepared_agent_tasks("parent", &mut target)
            .unwrap(),
        0
    );
    let store = nola_store::NolaStore::open(&target_path).unwrap();
    assert_eq!(
        store
            .runtime_agent_task_authority("task")
            .unwrap()
            .unwrap()
            .phase,
        nola_domain::agent_task_admission::RuntimeAgentTaskAuthorityPhase::Prepared
    );
}

// A subprocess entry into the production Service loop, not a protocol mock.
// Invoked by the process test below; an explicit --ignored run alone is harmless.
#[cfg(unix)]
#[test]
#[ignore]
fn task_admission_service_process_fixture() {
    let Some(database) = std::env::var_os("NOLA_TASK_ADMISSION_FIXTURE_DB") else { return };
    let socket = std::env::var_os("NOLA_TASK_ADMISSION_FIXTURE_SOCKET").unwrap();
    nola_service::serve(std::path::Path::new(&database), std::path::Path::new(&socket)).unwrap();
}

#[cfg(unix)]
#[test]
fn service_task_admission_rejected_receipt_recovers_after_process_restart() {
    use nola_protocol::CommandReceiptOutcome;
    use nola_service_client::ApplicationServiceClient;
    use std::process::{Child, Command, Stdio};
    use std::time::{Duration, Instant};

    struct Service(Child);
    impl Drop for Service {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    fn start(database: &std::path::Path, socket: &std::path::Path) -> Service {
        let mut service = Service(Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "session::task_result_delivery_tests::task_admission_service_process_fixture",
                "--ignored", "--nocapture"])
            .env("NOLA_TASK_ADMISSION_FIXTURE_DB", database)
            .env("NOLA_TASK_ADMISSION_FIXTURE_SOCKET", socket)
            .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
            .spawn().unwrap());
        let client = ApplicationServiceClient::with_socket(database, socket);
        let deadline = Instant::now() + Duration::from_secs(10);
        while client.health().is_err() {
            assert!(service.0.try_wait().unwrap().is_none(), "real Service exited before readiness");
            assert!(Instant::now() < deadline, "real Service readiness timed out");
            std::thread::sleep(Duration::from_millis(20));
        }
        service
    }

    // Keep the Unix socket path below the platform limit, including on macOS.
    let root = tempfile::Builder::new().prefix("nola-task-receipt-").tempdir_in("/tmp").unwrap();
    let source_path = root.path().join("source.db");
    let target_path = root.path().join("target.db");
    let socket = root.path().join("service.sock");
    let mut source_app = source_support::open_runtime(&source_path).unwrap();
    let fact = source_support::agent_task_admission::source_fact(&mut source_app);
    let mut target_app = source_support::open_runtime(&target_path).unwrap();
    source_support::agent_task_admission::draft(&mut target_app, None);
    drop(target_app);
    let db = rusqlite::Connection::open(&target_path).unwrap();
    db.execute_batch("CREATE TRIGGER reject_service_birth BEFORE UPDATE ON runtime_agent_task_authorities
        WHEN NEW.phase='admitted' BEGIN SELECT RAISE(ABORT,'service birth import fault'); END;").unwrap();
    let service = start(&target_path, &socket);
    let client = ApplicationServiceClient::with_socket(&target_path, &socket);
    let mut source = CliApplicationPort::InProcess(source_app);
    let mut target = CliApplicationPort::Service(client.clone());
    let original = format!("task-admission:{}:{}", fact.task_id, fact.admission_digest);
    let failure = source.recover_prepared_agent_tasks("parent", &mut target).unwrap_err();
    assert!(failure.contains("service birth import fault"), "{failure}");
    let rejected = client.command_receipt(&original).unwrap().unwrap();
    assert!(matches!(rejected.outcome, CommandReceiptOutcome::Rejected { .. }));
    assert_eq!(nola_store::NolaStore::open(&target_path).unwrap()
        .runtime_agent_task_authority("task").unwrap().unwrap().phase,
        nola_domain::agent_task_admission::RuntimeAgentTaskAuthorityPhase::Prepared);
    drop(target);
    drop(source);
    drop(service);
    db.execute_batch("DROP TRIGGER reject_service_birth").unwrap();

    let _restarted = start(&target_path, &socket);
    let reconnected = ApplicationServiceClient::with_socket(&target_path, &socket);
    let mut target = CliApplicationPort::Service(reconnected.clone());
    let mut source = CliApplicationPort::InProcess(source_support::open_runtime(&source_path).unwrap());
    assert_eq!(source.recover_prepared_agent_tasks("parent", &mut target).unwrap(), 1);
    assert!(matches!(reconnected.command_receipt(&original).unwrap().unwrap().outcome,
        CommandReceiptOutcome::Rejected { .. }), "the original rejection remains immutable");
    let retry = format!("{original}:retry:1");
    assert!(matches!(reconnected.command_receipt(&retry).unwrap().unwrap().outcome,
        CommandReceiptOutcome::Committed { .. }));
    assert_eq!(source.recover_prepared_agent_tasks("parent", &mut target).unwrap(), 0);
    assert!(source.recover_agent_task_admission("parent", "task", &mut target).unwrap());
    let authority = nola_store::NolaStore::open(&target_path).unwrap()
        .runtime_agent_task_authority("task").unwrap().unwrap();
    assert_eq!(authority.admission.as_ref(), Some(&fact));
    assert_eq!(authority.authority_revision, 2, "reconnect and lost-ACK replay must not admit twice");
    assert!(reconnected.command_receipt(&format!("{original}:retry:2")).unwrap().is_none());
    let attempts: i64 = db.query_row("SELECT COUNT(*) FROM runtime_task_attempts", [], |row| row.get(0)).unwrap();
    assert_eq!(attempts, 0, "recovery only imports authority and never dispatches a worker");
}

#[test]
fn opening_the_original_saved_session_recovers_its_prepared_task() {
    let _environment = crate::store::test_env_lock();
    let previous = std::env::var_os("NOLA_DB_PATH");
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("saved-birth.sqlite");
    std::env::set_var("NOLA_DB_PATH", &path);
    let mut app = source_support::open_runtime(&path).unwrap();
    let fact = source_support::agent_task_admission::source_fact(&mut app);
    source_support::agent_task_admission::draft(&mut app, None);
    drop(app);
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch(
        "CREATE TRIGGER reject_reopen_birth BEFORE UPDATE ON runtime_agent_task_authorities
         WHEN NEW.phase='admitted' BEGIN SELECT RAISE(ABORT,'reopen import fault'); END;",
    ).unwrap();
    let fresh = SessionStore::open(Some("new-unrelated-session"), false, "/fixture").unwrap();
    assert_eq!(fresh.id, "new-unrelated-session",
        "a pending Task import in another conversation must not block a fresh session");
    drop(fresh);
    assert!(SessionStore::open(Some("parent"), false, "/fixture").is_err(),
        "the original source must retain its failed admission instead of fabricating authority");
    db.execute_batch("DROP TRIGGER reject_reopen_birth").unwrap();
    let reopened = SessionStore::open(Some("parent"), false, "/fixture").unwrap();
    assert_eq!(reopened.id, "parent");
    let store = nola_store::NolaStore::open(&path).unwrap();
    let authority = store.runtime_agent_task_authority("task").unwrap().unwrap();
    assert_eq!(authority.admission.as_ref(), Some(&fact));
    assert_eq!(authority.authority_revision, 2);
    drop(reopened);
    match previous {
        Some(value) => std::env::set_var("NOLA_DB_PATH", value),
        None => std::env::remove_var("NOLA_DB_PATH"),
    }
}

#[test]
fn saved_and_ephemeral_results_commit_before_ack_and_retry_the_same_committed_response() {
    let _environment = crate::store::test_env_lock();
    let previous = std::env::var_os("NOLA_DB_PATH");
    for ephemeral in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let target_path = root.path().join("tasks.sqlite");
        let source_path = root.path().join("source.sqlite");
        std::env::set_var("NOLA_DB_PATH", &target_path);
        let mut target = source_support::open_runtime(&target_path).unwrap();
        source_support::agent_task_admission::draft(&mut target, None);
        let mut port = if ephemeral {
            CliApplicationPort::ephemeral().unwrap()
        } else {
            CliApplicationPort::InProcess(source_support::open_runtime(&source_path).unwrap())
        };
        let fact = match &mut port {
            CliApplicationPort::InProcess(app) => {
                source_support::agent_task_admission::source_fact(app)
            }
            _ => unreachable!(),
        };
        target
            .execute(ApplicationCommand::ImportHostAgentTaskAdmission { admission: fact })
            .unwrap();
        let attempt: nola_application::RuntimeTaskAttempt = serde_json::from_value(
            target
                .execute(ApplicationCommand::RuntimeTask {
                    command: nola_application::RuntimeTaskCommand::Claim {
                        task_id: "task".into(),
                        worker_id: "worker".into(),
                        worker_generation: "generation".into(),
                        lease_token: "token".into(),
                        now_ms: 1000,
                        lease_until_ms: 10000,
                    },
                })
                .unwrap(),
        )
        .unwrap();
        target
            .execute(ApplicationCommand::RuntimeTask {
                command: nola_application::RuntimeTaskCommand::FinishDetailedWithEvent {
                    final_observation: None,
                    lease: nola_application::RuntimeTaskLeaseWitness::from(&attempt),
                    status: "failed".into(),
                    completion_watermark: Some(7),
                    progress_summary: None,
                    result_summary: Some("Actual result".into()),
                    error: None,
                    output_path: None,
                    unread: true,
                    event_kind: "finished".into(),
                    event_status: "failed".into(),
                    event_summary: None,
                    event_detail_json: "{}".into(),
                    now_ms: 1001,
                },
            })
            .unwrap();
        drop(target);
        let task = crate::store::NolaStore::open(&target_path)
            .unwrap()
            .runtime_task("task")
            .unwrap()
            .unwrap();
        let cursor = crate::runtime_task_port::result_delivery_cursor(&task).unwrap();
        let active = port.active_run("parent").unwrap().unwrap();
        let mut source = SessionStore {
            id: "parent".into(),
            initial_configuration_revision: None,
            application_port: port,
            active_canonical_turn: Some(ActiveCanonicalTurn {
                run_id: active.run_id,
                kind: active.kind,
                next_event_sequence: active.next_event_sequence,
                invocation_config_digest: active.invocation_config_digest,
            }),
            task_execution: None,
            assistant_journal_notifications: Arc::default(),
            task_result_dispatches: Default::default(),
        };
        let model = claim(&mut source);
        source
            .accept_raw_host_response(echo(&model, 1, HostResponse::ExecutionStarted))
            .unwrap();
        source
            .accept_raw_host_response(echo(
                &model,
                2,
                HostResponse::ModelAttemptStarted {
                    provider: "fixture-provider".into(),
                    model: "fixture-model".into(),
                    reason: "initial".into(),
                },
            ))
            .unwrap();
        source.accept_raw_host_response(echo(&model,3,HostResponse::ModelCompleted {
            items:vec![],tool_calls:vec![json!({"call_id":"wait-result","name":"wait_task","arguments":{"task_id":"task"}})],usage:json!({}),
        })).unwrap();
        let wait = claim(&mut source);
        assert!(
            matches!(&wait.payload,HostRequest::ExecuteTool {capability_id,..} if capability_id=="wait_task")
        );
        source
            .accept_raw_host_response(echo(&wait, 1, HostResponse::ExecutionStarted))
            .unwrap();
        let call_id = match &wait.payload {
            HostRequest::ExecuteTool { call_id, .. } => call_id.clone(),
            _ => unreachable!(),
        };
        let business =
            json!({"completion_confirmed":true,"task":{"task_id":"task","status":"failed",
            "result":"Actual result","scheduling":{"budget_exhaustion":null}}})
            .to_string();
        let data = serde_json::to_value(
            crate::runtime_task_port::RuntimeTaskResultData::RuntimeTaskResultDelivery {
                cursor: cursor.clone(),
                text: business.clone(),
            },
        )
        .unwrap();
        let output = nola_host_protocol::tool_result::ToolResult {
            contract: nola_host_protocol::tool_result::ToolResultContract::V1,
            data: Some(data),
            presentation: nola_host_protocol::tool_result::ToolPresentation {
                text: "presentation is not authority".into(),
                summary: "failed".into(),
            },
            execution: Some(nola_host_protocol::tool_result::ExecutionReceipt {
                state: nola_host_protocol::tool_result::ExecutionState::Completed,
                ..Default::default()
            }),
            full_output_ref: None,
            memory_usage: None,
        };
        let for_model = output.clone().into_core_output();
        let received_business: Value =
            serde_json::from_str(for_model["text"].as_str().unwrap()).unwrap();
        assert_eq!(received_business["task"]["result"], "Actual result");
        assert_eq!(received_business["task"]["status"], "failed");
        assert!(received_business["task"]["scheduling"]["budget_exhaustion"].is_null());
        let response = echo(
            &wait,
            2,
            HostResponse::ToolCompleted {
                call_id: call_id.clone(),
                output: serde_json::to_value(output).unwrap(),
                outcome: HostToolOutcome::Succeeded,
                activation: None,
                full_output_ref: None,
                artifacts: vec![],
            },
        );
        assert!(
            nola_store::NolaStore::open(&target_path)
                .unwrap()
                .runtime_task("task")
                .unwrap()
                .unwrap()
                .unread,
            "constructing the result must not consume it before source acceptance"
        );
        let db = rusqlite::Connection::open(&target_path).unwrap();
        db.execute_batch(
            "CREATE TRIGGER reject_result_ack BEFORE UPDATE ON command_receipts
            WHEN NEW.command_type='runtime_task' AND NEW.status='committed'
            BEGIN SELECT RAISE(ABORT,'result ACK fault'); END;",
        )
        .unwrap();
        let error = source
            .accept_raw_host_response(response.clone())
            .unwrap_err();
        assert!(
            error.contains("source wait_task result committed"),
            "{error}"
        );
        let before = nola_store::NolaStore::open(&target_path)
            .unwrap()
            .runtime_task("task")
            .unwrap()
            .unwrap();
        assert!(before.unread);
        assert_ne!(
            serde_json::from_str::<Value>(&before.metadata_json).unwrap()["model_notified"],
            true
        );
        assert_ne!(
            serde_json::from_str::<Value>(&before.metadata_json).unwrap()["ui_notified"],
            true
        );
        let checkpoint = source
            .application_port
            .load_checkpoint("parent")
            .unwrap()
            .unwrap();
        assert!(contains_delivery(&serde_json::from_str::<Value>(&checkpoint.checkpoint_json).unwrap(),&cursor),
            "the exact result remains available through real cold materialization after ACK failure");
        let continuation = source.load_committed_requests().unwrap();
        assert!(
            continuation
                .iter()
                .all(|request| request.effect_id != wait.effect_id),
            "source-accepted wait_task must never be redispatched after target failure"
        );
        let model_messages = continuation
            .iter()
            .find_map(|request| match &request.payload {
                HostRequest::ModelStream { messages, .. } => Some(messages),
                _ => None,
            })
            .expect("actual next model request");
        let items = model_messages
            .iter()
            .cloned()
            .map(serde_json::from_value::<nola_core::agent::protocol::AgentItem>)
            .collect::<Result<Vec<_>, _>>()
            .expect("Core model requests contain typed AgentItems");
        let results = items
            .iter()
            .filter_map(|item| match item {
                nola_core::agent::protocol::AgentItem::ToolResult {
                    call_id: result_call_id,
                    name,
                    output,
                    is_error,
                    ..
                } if result_call_id.as_str() == call_id => {
                    assert_eq!(name, "wait_task");
                    assert!(!is_error);
                    Some(output)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(results.len(), 1, "the exact wait call has one model result");
        let delivered: Value = serde_json::from_str(
            results[0]["text"]
                .as_str()
                .expect("Task business result text"),
        )
        .expect("Task business result JSON");
        assert_eq!(delivered["task"]["task_id"], "task");
        assert_eq!(delivered["task"]["result"], "Actual result");
        assert_eq!(delivered["task"]["status"], "failed");
        assert!(delivered["task"]["scheduling"]["budget_exhaustion"].is_null());
        let mut changed = response.clone();
        if let HostResponse::ToolCompleted { output, .. } = &mut changed.payload {
            output["data"]["text"] = json!("a different result using the old cursor");
        }
        let error = source.accept_raw_host_response(changed).unwrap_err();
        assert!(
            error.contains("committed Host response identity or payload changed"),
            "{error}"
        );
        assert!(
            nola_store::NolaStore::open(&target_path)
                .unwrap()
                .runtime_task("task")
                .unwrap()
                .unwrap()
                .unread,
            "a changed source response cannot authorize a target ACK"
        );
        db.execute_batch("DROP TRIGGER reject_result_ack").unwrap();
        // The original source response is already committed (applied=false).
        // Its exact retained dispatch still authorizes reconciling this ACK.
        source.accept_raw_host_response(response.clone()).unwrap();
        let acknowledged = nola_store::NolaStore::open(&target_path)
            .unwrap()
            .runtime_task("task")
            .unwrap()
            .unwrap();
        assert!(!acknowledged.unread);
        assert_eq!(
            serde_json::from_str::<Value>(&acknowledged.metadata_json).unwrap()["model_notified"],
            true
        );
        assert_ne!(
            serde_json::from_str::<Value>(&acknowledged.metadata_json).unwrap()["ui_notified"],
            true
        );
        let head = source
            .application_port
            .load_checkpoint("parent")
            .unwrap()
            .unwrap();
        source.accept_raw_host_response(response).unwrap();
        assert_eq!(
            source
                .application_port
                .load_checkpoint("parent")
                .unwrap()
                .unwrap(),
            head,
            "lost ACK replay must not append a second source result"
        );
        assert_eq!(
            nola_store::NolaStore::open(&target_path)
                .unwrap()
                .runtime_task("task")
                .unwrap()
                .unwrap(),
            acknowledged
        );
        assert!(
            nola_store::NolaStore::open(&target_path)
                .unwrap()
                .load_checkpoint("parent")
                .unwrap()
                .is_none(),
            "cross-retention delivery must not copy the source transcript into the Task Store"
        );
        if ephemeral {
            assert!(!source_path.exists());
        }
        source.release_active_canonical_turn();
        assert!(source.task_result_dispatches.is_empty());
    }
    match previous {
        Some(value) => std::env::set_var("NOLA_DB_PATH", value),
        None => std::env::remove_var("NOLA_DB_PATH"),
    }
}
