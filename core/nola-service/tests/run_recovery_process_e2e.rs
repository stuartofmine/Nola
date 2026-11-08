// SPDX-License-Identifier: Apache-2.0
//! Entire run setup, successor creation and recovery execute through real Service IPC.
#[path = "../../nola-application/tests/support/model_identity.rs"]
mod fixture_model_identity;
use nola_application::{
    ApplicationCommand, BeginAssistantTurn, ClaimCommittedHostRequests, CommittedHostRequests,
    ReduceAndAcceptHostResponse, ReduceAndCommitActiveSubmission,
};
use nola_core::agent::{host::*, protocol::*};
use nola_service_client::ApplicationServiceClient;
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    path::Path,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
fn runtime_capability(id: &str, route: HostCapabilityRoute) -> HostCapabilityDescriptor {
    HostCapabilityDescriptor {
        id: id.to_string(),
        title: id.to_string(),
        route,
        availability: CapabilityAvailability::Available,
        exposure: HostCapabilityExposure::Internal,
        permission_domain: None,
        input_schema: serde_json::json!({"type": "object"}),
        input_normalization: Default::default(),
        output_policy: HostOutputPolicy {
            preview_char_limit: 4_000,
            persist_full_output: true,
            allow_artifacts: true,
        },
        supports_streaming: id == "model.stream",
        supports_argument_streaming: false,
        parallelism: HostCapabilityParallelism::Serial,
        cancellation: HostCancellationMode::Cooperative,
        replay_safety: HostReplaySafety::Recoverable,
    }
}

fn runtime_config() -> AgentCoreConfig {
    let mut capabilities = BTreeMap::new();
    capabilities.insert(
        "model.stream".to_string(),
        runtime_capability("model.stream", HostCapabilityRoute::Model),
    );
    AgentCoreConfig {
        turn_identity: fixture_model_identity::identity(),
        session_id: Some(SessionId::from_string(
            "completion-conversation".to_string(),
        )),
        model: Some("test-model".to_string()),
        host_catalog: Some(HostCapabilityCatalog {
            host_id: HostId("recovery-host".to_string()),
            revision: 1,
            availability_epoch: 1,
            lease: None,
            capabilities,
        }),
        ..AgentCoreConfig::default()
    }
}

fn response_for_request(
    request: &HostRequestEnvelope,
    sequence: u64,
    payload: HostResponse,
) -> HostResponseEnvelope {
    HostResponseEnvelope {
        delegated_invocation_id: request.delegated_invocation_id.clone(),
        attempt_id: request.attempt_id.clone(),
        request_id: request.request_id.clone(),
        effect_id: request.effect_id.clone(),
        host_id: request.host_id.clone(),
        lease_id: request.lease_id.clone(),
        delivery: request.delivery.clone(),
        session_id: request.session_id.clone(),
        turn_id: request.turn_id.clone(),
        catalog_revision: request.catalog_revision,
        availability_epoch: request.availability_epoch,
        sequence,
        payload,
    }
}

fn response_command(
    request: &HostRequestEnvelope,
    run: &str,
    digest: &str,
    sequence: u64,
    received_at_ms: i64,
    payload: HostResponse,
) -> ApplicationCommand {
    ApplicationCommand::ReduceAndAcceptHostResponse {
        input: ReduceAndAcceptHostResponse {
            conversation_id: "completion-conversation".into(),
            run_id: run.into(),
            invocation_config_digest: digest.into(),
            response: response_for_request(request, sequence, payload),
            received_at_ms,
        },
    }
}

fn user_input(id: &str, delivery: AgentInputDelivery) -> AgentSubmission {
    AgentSubmission::UserInput {
        items: vec![AgentItem::user_message(ItemId::from_string(id.into()), id)],
        turn_context_items: vec![],
        turn_context_platform: Default::default(),
        turn_context_components: vec![],
        submission_id: Some(format!("submit-{id}")),
        submitted_at_epoch_ms: Some(1000),
        delivery,
    }
}

struct Service(Child);

impl Drop for Service {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn start(database: &Path, socket: &Path, client: &ApplicationServiceClient) -> Service {
    let mut service = Service(
        Command::new(env!("CARGO_BIN_EXE_nola-service"))
            .arg("--database")
            .arg(database)
            .arg("--socket")
            .arg(socket)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if client.health().is_ok() {
            return service;
        }
        assert!(
            service.0.try_wait().unwrap().is_none(),
            "service exited during startup"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("service startup timed out");
}

fn pending(client: &ApplicationServiceClient, run: &str) -> Vec<HostRequestEnvelope> {
    let value: Value = client
        .execute(ApplicationCommand::LoadCommittedHostRequests { run_id: run.into() })
        .unwrap();
    serde_json::from_value(value["requests"].clone()).unwrap()
}
fn complete(
    client: &ApplicationServiceClient,
    request: &HostRequestEnvelope,
    run: &str,
    digest: &str,
    text: &str,
    claimed_at_ms: i64,
) {
    let claimed: CommittedHostRequests = serde_json::from_value(
        client
            .execute(ApplicationCommand::ClaimCommittedHostRequests {
                input: ClaimCommittedHostRequests {
                    run_id: run.into(),
                    worker_generation: format!("service-generation-{claimed_at_ms}"),
                    lease_owner: format!("service-owner-{claimed_at_ms}"),
                    claimed_at_ms,
                    lease_expires_at_ms: claimed_at_ms + 60_000,
                    limit: 1,
                },
            })
            .unwrap(),
    )
    .unwrap();
    let request = claimed
        .requests
        .into_iter()
        .find(|candidate| candidate.effect_id == request.effect_id)
        .expect("formal claim must return the pending model effect");
    assert!(request.delivery.is_some());
    assert!(request.attempt_id.is_some());
    assert!(!request.attempt_started);
    let _: Value = client
        .execute(response_command(
            &request,
            run,
            digest,
            1,
            claimed_at_ms + 1,
            HostResponse::ExecutionStarted,
        ))
        .unwrap();
    let _: Value = client
        .execute(response_command(
            &request,
            run,
            digest,
            2,
            claimed_at_ms + 2,
            HostResponse::ModelAttemptStarted {
                provider: "fixture-provider".into(),
                model: "test-model".into(),
                reason: "initial".into(),
            },
        ))
        .unwrap();
    let _: Value = client
        .execute(response_command(
            &request,
            run,
            digest,
            3,
            claimed_at_ms + 3,
            HostResponse::ModelCompleted {
                items: vec![serde_json::to_value(AgentItem::assistant_message(
                    ItemId::from_string(format!("answer-{}", request.request_id)),
                    text,
                ))
                .unwrap()],
                tool_calls: vec![],
                usage: json!({"input_tokens":80,"output_tokens":10}),
            },
        ))
        .unwrap();
}
#[test]
fn queued_successor_survives_service_restart_in_its_own_run_without_replaying_the_parent() {
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("nola.db");
    let socket = root.path().join("nola.sock");
    let client = ApplicationServiceClient::with_socket(&database, &socket);
    let first_process = start(&database, &socket, &client);
    let begun: Value = client
        .execute(ApplicationCommand::BeginAssistantTurn {
            input: BeginAssistantTurn {
                expected_configuration_revision: None,
                expected_parent_message_id: None,
                conversation_id: "completion-conversation".into(),
                conversation_title: "Recovery IPC".into(),
                user_message_id: "initial".into(),
                user_part_id: "initial:body".into(),
                user_text: "initial".into(),
                user_content_json: None,
                assistant_message_id: "submit-initial:assistant".into(),
                context_checkpoint_id: None,
                core_config: runtime_config(),
                submission: user_input("initial", AgentInputDelivery::CurrentTurn),
                created_at_ms: 1000,
            },
        })
        .unwrap();
    let run = begun["begin"]["run_id"].as_str().unwrap();
    let digest = begun["invocation_config_digest"].as_str().unwrap();
    let first = pending(&client, run).remove(0);
    assert!(matches!(first.payload, HostRequest::ModelStream { .. }));
    let _: Value = client
        .execute(ApplicationCommand::ReduceAndCommitActiveSubmission {
            input: ReduceAndCommitActiveSubmission {
                conversation_id: "completion-conversation".into(),
                run_id: run.into(),
                transition_id: "queued-next".into(),
                invocation_config_digest: digest.into(),
                submission: user_input("next", AgentInputDelivery::NextTurn),
                committed_at_ms: 1100,
            },
        })
        .unwrap();
    complete(&client, &first, run, digest, "First turn finished.", 1_200);
    assert!(pending(&client, run).is_empty());
    let promote = || ApplicationCommand::PromoteQueuedAssistantTurn {
        input: nola_application::PromoteQueuedAssistantTurn {
            conversation_id: "completion-conversation".into(),
            created_at_ms: 1_300,
        },
    };
    let promoted: Value = client
        .execute_with_request_id("promote-next", promote())
        .unwrap();
    let successor_run = promoted["begin"]["run_id"].as_str().unwrap();
    let successor_digest = promoted["invocation_config_digest"].as_str().unwrap();
    assert_ne!(
        successor_run, run,
        "one accepted user turn owns one formal Run"
    );
    assert_eq!(
        successor_digest, digest,
        "queued input keeps its frozen invocation"
    );
    let successor = pending(&client, successor_run);
    assert_eq!(successor.len(), 1);
    assert!(matches!(
        successor[0].payload,
        HostRequest::ModelStream { .. }
    ));
    assert_ne!(successor[0].turn_id, first.turn_id);
    assert_eq!(successor[0].turn_id.as_deref(), Some(successor_run));
    drop(first_process);
    let _ = std::fs::remove_file(&socket);
    let second_process = start(&database, &socket, &client);
    let recovery = || ApplicationCommand::RecoverOrphanedRuns {
        conversation_id: Some("completion-conversation".into()),
        reason: "service_restart".into(),
        interrupted_at: "after".into(),
        interrupted_at_ms: 3000,
    };
    let changed: Value = client
        .execute_with_request_id("recover-original", recovery())
        .unwrap();
    assert_eq!(changed, json!(0));
    assert_eq!(
        serde_json::to_value(pending(&client, successor_run)).unwrap(),
        serde_json::to_value(&successor).unwrap()
    );
    let active: Value = client
        .execute(ApplicationCommand::ActiveRun {
            conversation_id: "completion-conversation".into(),
        })
        .unwrap();
    assert_eq!(active["run_id"], successor_run);
    drop(second_process);
    let _ = std::fs::remove_file(&socket);
    let third_process = start(&database, &socket, &client);
    let replay: Value = client
        .execute_with_request_id("recover-original", recovery())
        .unwrap();
    assert_eq!(replay, changed);
    let promotion_replay: Value = client
        .execute_with_request_id("promote-next", promote())
        .unwrap();
    assert_eq!(
        promotion_replay, promoted,
        "cold receipt replay must not admit a third Run"
    );
    complete(
        &client,
        &successor[0],
        successor_run,
        successor_digest,
        "Successor completed after cold recovery.",
        4_000,
    );
    assert!(pending(&client, run).is_empty());
    assert!(pending(&client, successor_run).is_empty());
    let active: Value = client
        .execute(ApplicationCommand::ActiveRun {
            conversation_id: "completion-conversation".into(),
        })
        .unwrap();
    assert!(active.is_null());
    drop(third_process);
    let db = rusqlite::Connection::open_with_flags(
        &database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let rows: (i64, String) = db
        .query_row("SELECT count(*),min(status) FROM runs", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    assert_eq!(rows, (2, "sealed".into()));
    for owned_run in [run, successor_run] {
        let result: String = db
            .query_row(
                "SELECT json_extract(result_json,'$.value.kind') FROM run_results WHERE run_id=?1",
                [owned_run],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(result, "answered");
        let terminal_events: i64 = db
            .query_row(
                "SELECT count(*) FROM run_events WHERE run_id=?1 AND event_kind='turn_completed'",
                [owned_run],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(terminal_events, 1);
        let active_attempt: Option<String> = db
            .query_row(
                "SELECT active_attempt_id FROM runs WHERE id=?1",
                [owned_run],
                |row| row.get(0),
            )
            .unwrap();
        assert!(active_attempt.is_none());
    }
    let attempts: Vec<Value> = db
        .prepare("SELECT record_json FROM run_attempts ORDER BY id")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .map(|row| serde_json::from_str(&row.unwrap()).unwrap())
        .collect();
    assert_eq!(
        attempts.len(),
        2,
        "Two logical model effects each have one initial physical Attempt"
    );
    assert_ne!(attempts[0]["id"], attempts[1]["id"]);
    assert_ne!(attempts[0]["effect_id"], attempts[1]["effect_id"]);
    let expected_owners = [
        (first.effect_id.0.as_str(), run),
        (successor[0].effect_id.0.as_str(), successor_run),
    ];
    for attempt in &attempts {
        let effect = attempt["effect_id"].as_str().unwrap();
        let owner = expected_owners
            .iter()
            .find(|(expected, _)| *expected == effect)
            .unwrap_or_else(|| panic!("unexpected physical effect: {attempt}"));
        assert_eq!(attempt["run_id"], owner.1);
        assert_eq!(attempt["state"], "succeeded");
        assert_eq!(attempt["provider"], "fixture-provider");
        assert_eq!(attempt["model"], "test-model");
        assert_eq!(attempt["invocation_id"], attempt["effect_id"]);
        assert!(
            attempt["predecessor_id"].is_null(),
            "A separately admitted successor is not a provider retry"
        );
        assert!(attempt["started_at_ms"].is_i64() && attempt["completed_at_ms"].is_i64());
        assert_eq!(
            attempt["usage"],
            json!({"input_tokens":80,"output_tokens":10})
        );
    }
}
