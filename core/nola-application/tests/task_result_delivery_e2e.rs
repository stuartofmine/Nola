// SPDX-License-Identifier: Apache-2.0
//! Exact result consumption must not acknowledge an undelivered peer or UI.
mod support;

use nola_application::{
    AcknowledgeRuntimeTaskResult, ApplicationCommand, ApplicationCommandReceiptOutcome,
    CreateRuntimeTask, ExecuteReceivedCommand, RuntimeTaskCommand, RuntimeTaskLeaseWitness,
};
use nola_agent_engine::agent::{host::*, AgentEvent, SessionId, TaskKind, TaskLineage, TaskState};
use nola_store::NolaStore;
use serde_json::{json, Value};

fn seed(path: &std::path::Path) -> [AcknowledgeRuntimeTaskResult; 2] {
    let mut app = support::open_runtime(path).unwrap();
    for (id, child, call) in [
        ("task", "child", "task-call"),
        ("peer", "peer-child", "peer-call"),
    ] {
        app.execute(ApplicationCommand::RuntimeTask {
            command: RuntimeTaskCommand::Create {
                input: CreateRuntimeTask {
                    id: id.into(),
                    kind: "agent".into(),
                    title: id.into(),
                    project_root: "/fixture".into(),
                    owner_session_id: Some("parent".into()),
                    parent_task_id: None,
                    parent_tool_call_id: Some(call.into()),
                    child_session_id: Some(child.into()),
                    agent_type: Some("explore".into()),
                    prompt: Some("Execute this task once".into()),
                    initial_progress_summary: None,
                    initial_activity: None,
                    output_path: None,
                    initially_unread: false,
                    input_watermark: Some(7),
                    created_at_ms: 100,
                    metadata: json!({}),
                    scheduling: nola_application::RuntimeTaskScheduling {
                        priority: nola_application::RuntimeTaskPriority::Normal,
                        queued_at_ms: 100,
                        resource_budget: Default::default(),
                        resource_usage: Default::default(),
                        output_bytes: 0,
                        budget_exhaustion: None,
                    },
                },
            },
        })
        .unwrap();
    }
    let first = support::agent_task_admission::source_fact(&mut app);
    app.execute(ApplicationCommand::ImportHostAgentTaskAdmission { admission: first })
        .unwrap();

    // Produce the peer from the SAME actual parent Run, not a second fabricated
    // source with colliding conversation/Run identities.
    let store = NolaStore::open(path).unwrap();
    let run = store.active_run("parent").unwrap().unwrap();
    let digest = run.invocation_config_digest.unwrap();
    let claim = |app: &mut support::ApplicationRuntime, at| -> HostRequestEnvelope {
        let requests: nola_application::CommittedHostRequests = serde_json::from_value(
            app.execute(ApplicationCommand::ClaimCommittedHostRequests {
                input: nola_application::ClaimCommittedHostRequests {
                    run_id: run.run_id.clone(),
                    worker_generation: "peer-source".into(),
                    lease_owner: "source".into(),
                    claimed_at_ms: at,
                    lease_expires_at_ms: 10000,
                    limit: 16,
                },
            })
            .unwrap(),
        )
        .unwrap();
        assert_eq!(requests.requests.len(), 1);
        requests.requests.into_iter().next().unwrap()
    };
    let accept = |app: &mut support::ApplicationRuntime,
                  request: &HostRequestEnvelope,
                  sequence,
                  payload,
                  at|
     -> nola_application::AcceptedHostResponse {
        serde_json::from_value(
            app.execute(ApplicationCommand::ReduceAndAcceptHostResponse {
                input: nola_application::ReduceAndAcceptHostResponse {
                    conversation_id: "parent".into(),
                    run_id: run.run_id.clone(),
                    invocation_config_digest: digest.clone(),
                    received_at_ms: at,
                    response: HostResponseEnvelope {
                        attempt_id: request.attempt_id.clone(),
                        delegated_invocation_id: request.delegated_invocation_id.clone(),
                        request_id: request.request_id.clone(),
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
                    },
                },
            })
            .unwrap(),
        )
        .unwrap()
    };
    let mut model = claim(&mut app, 700);
    assert!(matches!(model.payload, HostRequest::ModelStream { .. }));
    accept(&mut app, &model, 1, HostResponse::ExecutionStarted, 701);
    model.resume_after_sequence = Some(1);
    accept(
        &mut app,
        &model,
        2,
        HostResponse::ModelAttemptStarted {
            provider: "fixture-provider".into(),
            model: "fixture-model".into(),
            reason: "initial".into(),
        },
        702,
    );
    model.resume_after_sequence = Some(2);
    accept(
        &mut app,
        &model,
        3,
        HostResponse::ModelCompleted {
            items: vec![],
            tool_calls: vec![
                json!({"call_id":"peer-call","name":"fixture.task","arguments":{"task_id":"peer"}}),
            ],
            usage: json!({}),
        },
        703,
    );
    let tool = claim(&mut app, 710);
    assert!(matches!(tool.payload, HostRequest::ExecuteTool { .. }));
    accept(&mut app, &tool, 1, HostResponse::ExecutionStarted, 711);
    let promoted = accept(
        &mut app,
        &tool,
        2,
        HostResponse::TaskPromoted {
            task_id: "peer".into(),
            input_watermark: Some(7),
            kind: TaskKind::Agent,
            state: TaskState::Queued,
            progress: None,
            current_step: Some("Queued".into()),
            lineage: TaskLineage {
                child_session_id: Some(SessionId::from_string("peer-child".into())),
                parent_call_id: Some(nola_agent_engine::agent::CallId::from_string("peer-call".into())),
                ..Default::default()
            },
        },
        712,
    );
    let cursor = promoted
        .events
        .iter()
        .find_map(|event| match &event.event {
            AgentEvent::TaskUpdated { task, .. } if task.id == "peer" => {
                Some(event.cursor.sequence)
            }
            _ => None,
        })
        .unwrap();
    let admission = serde_json::from_value(
        app.execute(ApplicationCommand::ExportAgentTaskAdmission {
            conversation_id: "parent".into(),
            run_id: run.run_id.clone(),
            task_id: "peer".into(),
            event_cursor: cursor,
        })
        .unwrap(),
    )
    .unwrap();
    app.execute(ApplicationCommand::ImportHostAgentTaskAdmission { admission })
        .unwrap();
    drop(app);
    let mut store = NolaStore::open(path).unwrap();
    ["task", "peer"].map(|id| {
        let attempt = store
            .claim_runtime_task(
                id,
                &format!("worker-{id}"),
                &format!("generation-{id}"),
                &format!("token-{id}"),
                1000,
                5000,
            )
            .unwrap()
            .unwrap();
        assert!(store
            .finish_runtime_task_detailed_with_event(
                &RuntimeTaskLeaseWitness::from(&attempt),
                "failed",
                Some(7),
                Some("done"),
                Some(id),
                None,
                None,
                true,
                "failed",
                "failed",
                Some("done"),
                "{}",
                None,
                1100
            )
            .unwrap());
        let receipt = store.runtime_task_terminal_receipt(id).unwrap().unwrap();
        let task = store.runtime_task(id).unwrap().unwrap();
        assert!(task.unread);
        AcknowledgeRuntimeTaskResult {
            owner_conversation_id: "parent".into(),
            task_id: id.into(),
            terminal_receipt_id: receipt.receipt_id,
            expected_updated_at_ms: task.updated_at_ms,
        }
    })
}

fn command(input: AcknowledgeRuntimeTaskResult) -> ApplicationCommand {
    ApplicationCommand::RuntimeTask {
        command: RuntimeTaskCommand::AcknowledgeResult { input },
    }
}

fn state(path: &std::path::Path) -> Value {
    let store = NolaStore::open(path).unwrap();
    let receipts =
        ["task", "peer"].map(|id| store.runtime_task_terminal_receipt(id).unwrap().unwrap());
    let attempts = receipts
        .iter()
        .flat_map(|receipt| &receipt.attempts)
        .map(|attempt| {
            store
                .runtime_task_attempt(&attempt.id)
                .unwrap()
                .unwrap()
        })
        .collect::<Vec<_>>();
    json!([
        store.runtime_task("task").unwrap(),
        store.runtime_task("peer").unwrap(),
        store.runtime_task_terminal_receipt("task").unwrap(),
        store.runtime_task_terminal_receipt("peer").unwrap(),
        store.runtime_agent_task_authority("task").unwrap(),
        store.runtime_agent_task_authority("peer").unwrap(),
        attempts
    ])
}

#[test]
fn exact_result_ack_preserves_peer_and_ui_and_rejects_foreign_or_stale_proofs() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("results.db");
    let [first, peer] = seed(&path);
    let mut app = support::open_runtime(&path).unwrap();
    let before = state(&path);
    for input in [
        AcknowledgeRuntimeTaskResult {
            owner_conversation_id: "foreign".into(),
            ..first.clone()
        },
        AcknowledgeRuntimeTaskResult {
            terminal_receipt_id: peer.terminal_receipt_id.clone(),
            ..first.clone()
        },
        AcknowledgeRuntimeTaskResult {
            expected_updated_at_ms: first.expected_updated_at_ms - 1,
            ..first.clone()
        },
    ] {
        assert!(app.execute(command(input)).is_err());
        assert_eq!(state(&path), before);
    }
    assert_eq!(app.execute(command(first.clone())).unwrap(), json!(true));
    let after = state(&path);
    assert_eq!(
        after[1], before[1],
        "undelivered peer must stay entirely unchanged"
    );
    assert_eq!(after[2], before[2]);
    assert_eq!(after[3], before[3]);
    assert_eq!(after[0]["unread"], false);
    let old_metadata: Value =
        serde_json::from_str(before[0]["metadata_json"].as_str().unwrap()).unwrap();
    let metadata: Value =
        serde_json::from_str(after[0]["metadata_json"].as_str().unwrap()).unwrap();
    assert_eq!(metadata["model_notified"], true);
    assert_eq!(metadata["ui_notified"], old_metadata["ui_notified"]);
    assert_eq!(app.execute(command(first.clone())).unwrap(), json!(false));
    assert_eq!(state(&path), after);
    drop(app);
    let mut cold = support::open_runtime(&path).unwrap();
    assert_eq!(cold.execute(command(first)).unwrap(), json!(false));
    assert_eq!(state(&path), after);
    assert_eq!(cold.execute(command(peer)).unwrap(), json!(true));
}

#[test]
fn result_ack_receipt_failure_rolls_back_and_lost_ack_replays_after_restart() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("rollback.db");
    let [first, _] = seed(&path);
    let mut app = support::open_runtime(&path).unwrap();
    let encoded = serde_json::to_string(&command(first)).unwrap();
    let envelope = || ExecuteReceivedCommand {
        client_id: "result-delivery".into(),
        request_id: "exact-result".into(),
        command_type: "runtime_task".into(),
        command_json: encoded.clone(),
        expected_revision: None,
        received_at: "2026-10-08T00:00:00Z".into(),
    };
    let before = state(&path);
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TRIGGER fail_result_receipt BEFORE UPDATE ON command_receipts WHEN NEW.status='committed' BEGIN SELECT RAISE(ABORT,'result receipt fault'); END;").unwrap();
    let error = app
        .execute_received_command(envelope(), serde_json::from_str(&encoded).unwrap())
        .unwrap_err();
    assert!(error.to_string().contains("result receipt fault"));
    assert_eq!(state(&path), before);
    db.execute_batch("DROP TRIGGER fail_result_receipt")
        .unwrap();
    let accepted = app
        .execute_received_command(envelope(), serde_json::from_str(&encoded).unwrap())
        .unwrap();
    let ApplicationCommandReceiptOutcome::Committed { ref result, .. } = accepted.outcome else {
        panic!("result delivery did not commit");
    };
    assert_eq!(*result, json!(true));
    let after = state(&path);
    assert_eq!(after[1], before[1]);
    drop(app);
    let mut cold = support::open_runtime(&path).unwrap();
    let replay = cold
        .execute_received_command(envelope(), serde_json::from_str(&encoded).unwrap())
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.outcome, accepted.outcome);
    assert_eq!(state(&path), after);
}
