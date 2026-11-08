// SPDX-License-Identifier: Apache-2.0
//! Real-process E2E for the desktop single-writer Application boundary.

#![cfg(unix)]

#[path = "../../nola-application/tests/support/model_identity.rs"]
mod fixture_model_identity;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use nola_application::{
    ApplicationCommand, BeginAssistantTurn, ClaimCommittedHostRequests, CommittedHostRequests,
    HeartbeatEffectDelivery, ReduceAndAcceptHostResponse,
};
use nola_core::agent::host::{
    CapabilityAvailability, EffectDeliveryLease, HostCancellationMode, HostCapabilityCatalog,
    HostCapabilityDescriptor, HostCapabilityExposure, HostCapabilityParallelism,
    HostCapabilityRoute, HostId, HostOutputPolicy, HostReplaySafety, HostRequest, HostResponse,
    HostResponseEnvelope, ModelDeltaChannel,
};
use nola_core::agent::{
    AgentCoreConfig, AgentInputDelivery, AgentItem, AgentSubmission, ItemId, SessionId,
};
use nola_protocol::{
    CommandEnvelope, CommandPayload, CommandReceipt, CommandReceiptOutcome,
    RuntimeEffectWorkReason, RuntimeEffectWorkSchedule, APPLICATION_SERVICE_PROTOCOL_VERSION,
};
use serde_json::{json, Value};

struct ServiceProcess(Child);

#[test]
fn recovered_admission_has_the_same_nonterminal_receipt_on_submit_and_lookup() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("accepted.db");
    let socket = directory.path().join("accepted.sock");
    // Read-only queries have fresh-read semantics and never enter admission.
    // Use a real write so recovery proves a pending mutation is not reexecuted.
    let arguments = json!({"key":"interrupted-write", "value_json":"true", "updated_at_ms":1});
    // Seed the durable boundary left by an interrupted execution, not a mock transport.
    {
        let mut store = nola_store::NolaStore::open(&database).unwrap();
        store
            .begin_command_receipt(&nola_store::BeginCommandReceiptInput {
                client_id: "service-process-e2e".into(),
                request_id: "interrupted".into(),
                command_type: "put_product_setting".into(),
                command_json: arguments.to_string(),
                expected_revision: None,
                received_at: "2026-09-06T00:00:00Z".into(),
            })
            .unwrap();
    }
    for _ in 0..2 {
        let service = start_service(&database, &socket);
        let looked_up = request(
            &socket,
            "lookup",
            "command_receipt",
            json!({"request_id": "interrupted"}),
        );
        assert_eq!(looked_up["status"], "accepted");
        let mut stream = UnixStream::connect(&socket).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let envelope = CommandEnvelope {
            protocol_version: APPLICATION_SERVICE_PROTOCOL_VERSION,
            client_id: "service-process-e2e".into(),
            request_id: "interrupted".into(),
            expected_revision: None,
            command: CommandPayload {
                command_type: "put_product_setting".into(),
                arguments: arguments.clone(),
            },
        };
        serde_json::to_writer(&mut stream, &envelope).unwrap();
        stream.write_all(b"\n").unwrap();
        stream.flush().unwrap();
        let mut line = String::new();
        BufReader::new(stream).read_line(&mut line).unwrap();
        let submitted: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(
            submitted, looked_up,
            "submission must not fabricate terminal failure"
        );
        let after = request(
            &socket,
            "lookup-after",
            "command_receipt",
            json!({"request_id": "interrupted"}),
        );
        assert_eq!(
            after, looked_up,
            "retry must not reexecute an interrupted command"
        );
        drop(service);
    }
    let store = nola_store::NolaStore::open(&database).unwrap();
    let receipt = store
        .command_receipt("service-process-e2e", "interrupted")
        .unwrap()
        .unwrap();
    assert_eq!(receipt.status, "accepted");
    assert!(receipt.completed_at.is_none());
    assert!(
        store
            .product_setting("interrupted-write")
            .unwrap()
            .is_none(),
        "accepted recovery must not execute the write"
    );
}

impl Drop for ServiceProcess {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn start_service(database: &Path, socket: &Path) -> ServiceProcess {
    let child = Command::new(env!("CARGO_BIN_EXE_nola-service"))
        .args([
            "--database",
            database.to_str().unwrap(),
            "--socket",
            socket.to_str().unwrap(),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("start real nola-service process");
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if UnixStream::connect(socket).is_ok() {
            return ServiceProcess(child);
        }
        thread::sleep(Duration::from_millis(20));
    }
    panic!("nola-service did not bind {}", socket.display());
}

#[test]
fn explicit_development_reset_recreates_only_an_incompatible_schema() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("incompatible.db");
    let socket = directory.path().join("incompatible.sock");
    let connection = rusqlite::Connection::open(&database).unwrap();
    connection
        .execute_batch("CREATE TABLE legacy_state(value TEXT); PRAGMA user_version=99;")
        .unwrap();
    drop(connection);

    let child = Command::new(env!("CARGO_BIN_EXE_nola-service"))
        .args([
            "--database",
            database.to_str().unwrap(),
            "--socket",
            socket.to_str().unwrap(),
            "--recreate-incompatible-store",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("start resetting nola-service process");
    let service = ServiceProcess(child);
    let client = nola_service_client::ApplicationServiceClient::with_socket(&database, &socket);
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if client.health().is_ok() {
            drop(service);
            let reopened = nola_store::NolaStore::open(&database)
                .expect("recreated database must use the current canonical schema");
            drop(reopened);
            let connection = rusqlite::Connection::open(&database).unwrap();
            let legacy_count: i64 = connection
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE name='legacy_state'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(legacy_count, 0);
            return;
        }
        thread::sleep(Duration::from_millis(20));
    }
    panic!("resetting nola-service did not become ready");
}

#[test]
fn incompatible_schema_exits_with_a_typed_startup_failure_and_preserves_data() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("incompatible.db");
    let socket = directory.path().join("incompatible.sock");
    let connection = rusqlite::Connection::open(&database).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE legacy_state(value TEXT); \
             INSERT INTO legacy_state VALUES('keep'); \
             PRAGMA application_id=1313819713; PRAGMA user_version=11;",
        )
        .unwrap();
    drop(connection);
    let original = fs::read(&database).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_nola-service"))
        .args([
            "--database",
            database.to_str().unwrap(),
            "--socket",
            socket.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let envelope: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(envelope["type"], "nola.service.startup_failure");
    assert_eq!(envelope["protocol_version"], 1);
    assert_eq!(envelope["code"], "incompatible_canonical_store");
    assert_eq!(fs::read(&database).unwrap(), original);
}

fn request(socket: &Path, request_id: &str, command_type: &str, arguments: Value) -> Value {
    let mut stream = UnixStream::connect(socket).expect("connect to real nola-service process");
    let envelope = CommandEnvelope {
        protocol_version: APPLICATION_SERVICE_PROTOCOL_VERSION,
        request_id: request_id.to_string(),
        client_id: "service-process-e2e".to_string(),
        expected_revision: None,
        command: CommandPayload {
            command_type: command_type.to_string(),
            arguments,
        },
    };
    serde_json::to_writer(&mut stream, &envelope).unwrap();
    stream.write_all(b"\n").unwrap();
    stream.flush().unwrap();
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).unwrap();
    let response: CommandReceipt = serde_json::from_str(&line).unwrap();
    assert_eq!(response.request_id, request_id);
    match response.outcome {
        CommandReceiptOutcome::Committed { result, .. } => result,
        outcome => panic!("service rejected request: {outcome:?}"),
    }
}

fn execute(socket: &Path, request_id: &str, command: Value) -> Value {
    let Value::Object(mut arguments) = command else {
        panic!("Application command fixture must be an object");
    };
    let command_type = arguments
        .remove("type")
        .and_then(|value| value.as_str().map(str::to_owned))
        .expect("Application command fixture must include a type");
    request(socket, request_id, &command_type, Value::Object(arguments))
}

#[test]
fn health_remains_responsive_while_another_client_holds_a_partial_frame() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("partial-frame.db");
    let socket = directory.path().join("partial-frame.sock");
    let _service = start_service(&database, &socket);

    let mut stalled = UnixStream::connect(&socket).expect("open partial request connection");
    stalled
        .write_all(br#"{"protocol_version":1,"request_id":"partial""#)
        .expect("write partial request");
    stalled.flush().expect("flush partial request");

    let started = Instant::now();
    let health = request(&socket, "health-during-partial", "health", json!({}));
    assert_eq!(health["service"], "nola-application");
    assert!(
        started.elapsed() < Duration::from_millis(500),
        "health was serialized behind a partial request: {:?}",
        started.elapsed()
    );
}

fn stream_config() -> AgentCoreConfig {
    let capability = HostCapabilityDescriptor {
        id: "model.stream".to_string(),
        title: "Model stream".to_string(),
        route: HostCapabilityRoute::Model,
        availability: CapabilityAvailability::Available,
        exposure: HostCapabilityExposure::Internal,
        permission_domain: None,
        input_schema: json!({"type": "object"}),
        input_normalization: Default::default(),
        output_policy: HostOutputPolicy {
            preview_char_limit: 4_000,
            persist_full_output: true,
            allow_artifacts: true,
        },
        supports_streaming: true,
        supports_argument_streaming: false,
        parallelism: HostCapabilityParallelism::Serial,
        cancellation: HostCancellationMode::Cooperative,
        replay_safety: HostReplaySafety::Recoverable,
    };
    AgentCoreConfig {
        turn_identity: fixture_model_identity::identity(),
        session_id: Some(SessionId::from_string("service-memory".to_string())),
        model: Some("fixture-model".to_string()),
        host_catalog: Some(HostCapabilityCatalog {
            host_id: HostId("service-memory-host".to_string()),
            revision: 1,
            availability_epoch: 1,
            lease: None,
            capabilities: std::collections::BTreeMap::from([(
                "model.stream".to_string(),
                capability,
            )]),
        }),
        ..AgentCoreConfig::default()
    }
}

fn begin_runtime_wait_turn(socket: &Path, suffix: &str, created_at_ms: i64) -> Value {
    let mut config = stream_config();
    // A Core session belongs to exactly one canonical conversation, including
    // these simultaneous foreign/new work schedules.
    config.session_id = Some(SessionId::from_string(format!(
        "runtime-wait-conversation-{suffix}"
    )));
    execute(
        socket,
        &format!("runtime-wait-begin-{suffix}"),
        serde_json::to_value(ApplicationCommand::BeginAssistantTurn {
            input: BeginAssistantTurn {
                expected_configuration_revision: None,
                expected_parent_message_id: None,
                conversation_id: format!("runtime-wait-conversation-{suffix}"),
                conversation_title: "Runtime wait".into(),
                user_message_id: format!("runtime-wait-user-{suffix}"),
                user_part_id: format!("runtime-wait-user-{suffix}:text"),
                user_text: "schedule committed work".into(),
                user_content_json: None,
                assistant_message_id: format!("runtime-wait-assistant-{suffix}"),
                context_checkpoint_id: None,
                core_config: config,
                submission: AgentSubmission::UserInput {
                    items: vec![AgentItem::user_message(
                        ItemId::from_string(format!("runtime-wait-input-{suffix}")),
                        "schedule committed work",
                    )],
                    turn_context_items: Vec::new(),
                    turn_context_platform: Default::default(),
                    turn_context_components: Vec::new(),
                    submission_id: Some(format!("runtime-wait-submission-{suffix}")),
                    submitted_at_epoch_ms: Some(created_at_ms),
                    delivery: AgentInputDelivery::CurrentTurn,
                },
                created_at_ms,
            },
        })
        .unwrap(),
    )
}

fn epoch_ms() -> i64 {
    i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap()
}

#[test]
fn runtime_effect_work_wait_wakes_early_for_new_committed_work_without_a_receipt() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("runtime-work-commit.db");
    let socket = directory.path().join("runtime-work-commit.sock");
    let _service = start_service(&database, &socket);
    begin_runtime_wait_turn(&socket, "foreign", epoch_ms());
    let wait_socket = socket.clone();
    let started = Instant::now();
    let waiter = thread::spawn(move || {
        request(
            &wait_socket,
            "runtime-work-wait-new",
            "runtime_effect_work_wait",
            json!({"conversation_ids":["runtime-wait-conversation-new"], "limit": 4, "wait_ms": 3_000}),
        )
    });
    let waiter_deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let health = request(&socket, "runtime-work-health", "health", json!({}));
        if health["projection_waiters"].as_u64() == Some(1) {
            break;
        }
        assert!(
            Instant::now() < waiter_deadline,
            "Runtime work waiter was not registered"
        );
        thread::sleep(Duration::from_millis(10));
    }
    let begun = begin_runtime_wait_turn(&socket, "new", epoch_ms());
    let expected_run = begun["begin"]["run_id"].as_str().unwrap();
    let schedule: RuntimeEffectWorkSchedule =
        serde_json::from_value(waiter.join().unwrap()).unwrap();
    assert_eq!(schedule.runs.len(), 1);
    assert_eq!(schedule.runs[0].run_id, expected_run);
    assert_eq!(schedule.runs[0].reason, RuntimeEffectWorkReason::Pending);
    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(request(
        &socket,
        "runtime-work-wait-lookup",
        "command_receipt",
        json!({"request_id":"runtime-work-wait-new"}),
    )
    .is_null());
}

#[test]
fn runtime_effect_work_wait_wakes_at_a_future_lease_expiry() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("runtime-work-expiry.db");
    let socket = directory.path().join("runtime-work-expiry.sock");
    let _service = start_service(&database, &socket);
    let now_ms = epoch_ms();
    let begun = begin_runtime_wait_turn(&socket, "expiry", now_ms);
    let run_id = begun["begin"]["run_id"].as_str().unwrap().to_string();
    let lease_expires_at_ms = epoch_ms() + 500;
    let claimed: CommittedHostRequests = serde_json::from_value(execute(
        &socket,
        "runtime-work-expiry-claim",
        serde_json::to_value(ApplicationCommand::ClaimCommittedHostRequests {
            input: ClaimCommittedHostRequests {
                run_id: run_id.clone(),
                worker_generation: "runtime-wait-generation".into(),
                lease_owner: "runtime-wait-owner".into(),
                claimed_at_ms: epoch_ms(),
                lease_expires_at_ms,
                limit: 1,
            },
        })
        .unwrap(),
    ))
    .unwrap();
    let attempt_id = claimed.requests[0]
        .delivery
        .as_ref()
        .unwrap()
        .attempt_id
        .clone();
    let started = Instant::now();
    let schedule: RuntimeEffectWorkSchedule = serde_json::from_value(request(
        &socket,
        "runtime-work-wait-expiry",
        "runtime_effect_work_wait",
        json!({"conversation_ids":["runtime-wait-conversation-expiry"], "limit": 1, "wait_ms": 3_000}),
    ))
    .unwrap();
    assert_eq!(schedule.runs.len(), 1);
    assert_eq!(schedule.runs[0].run_id, run_id);
    assert_eq!(
        schedule.runs[0].attempt_id.as_deref(),
        Some(attempt_id.as_str())
    );
    assert_eq!(
        schedule.runs[0].reason,
        RuntimeEffectWorkReason::RecoverExpiredLease
    );
    assert!(started.elapsed() >= Duration::from_millis(350));
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[test]
fn disconnect_releases_a_real_runtime_effect_work_waiter() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("runtime-work-disconnect.db");
    let socket = directory.path().join("runtime-work-disconnect.sock");
    let _service = start_service(&database, &socket);
    let mut stream = UnixStream::connect(&socket).unwrap();
    let envelope = CommandEnvelope {
        protocol_version: APPLICATION_SERVICE_PROTOCOL_VERSION,
        request_id: "runtime-work-disconnect".into(),
        client_id: "service-process-e2e".into(),
        expected_revision: None,
        command: CommandPayload {
            command_type: "runtime_effect_work_wait".into(),
            arguments: json!({"conversation_ids":["runtime-wait-disconnect"],"limit":1,"wait_ms":30_000}),
        },
    };
    serde_json::to_writer(&mut stream, &envelope).unwrap();
    stream.write_all(b"\n").unwrap();
    stream.flush().unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while request(
        &socket,
        "runtime-work-disconnect-health",
        "health",
        json!({}),
    )["projection_waiters"]
        .as_u64()
        != Some(1)
    {
        assert!(
            Instant::now() < deadline,
            "Runtime work waiter was not registered"
        );
        thread::sleep(Duration::from_millis(10));
    }
    drop(stream);
    while request(
        &socket,
        "runtime-work-disconnected-health",
        "health",
        json!({}),
    )["projection_waiters"]
        .as_u64()
        != Some(0)
    {
        assert!(
            Instant::now() < deadline,
            "disconnected Runtime waiter was retained"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

fn response_for(
    request: &nola_core::agent::host::HostRequestEnvelope,
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

fn read_effect_delivery_attempt(
    database: &Path,
    attempt_id: &str,
) -> nola_domain::EffectDeliveryAttempt {
    let connection =
        rusqlite::Connection::open_with_flags(database, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
    let record_json: String = connection
        .query_row(
            "SELECT record_json FROM effect_delivery_attempts WHERE id=?1",
            [attempt_id],
            |row| row.get(0),
        )
        .unwrap();
    serde_json::from_str(&record_json).unwrap()
}

#[test]
fn heartbeat_round_trips_through_the_real_service_and_fences_the_next_response() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("effect-delivery-heartbeat.db");
    let socket = directory.path().join("effect-delivery-heartbeat.sock");
    let _service = start_service(&database, &socket);

    let begun = execute(
        &socket,
        "heartbeat-begin",
        serde_json::to_value(ApplicationCommand::BeginAssistantTurn {
            input: BeginAssistantTurn {
                expected_configuration_revision: None,
                expected_parent_message_id: None,
                conversation_id: "service-memory".into(),
                conversation_title: "Heartbeat".into(),
                user_message_id: "heartbeat-user".into(),
                user_part_id: "heartbeat-user:text".into(),
                user_text: "renew the claimed model effect".into(),
                user_content_json: None,
                assistant_message_id: "heartbeat-assistant".into(),
                context_checkpoint_id: None,
                core_config: stream_config(),
                submission: AgentSubmission::UserInput {
                    items: vec![AgentItem::user_message(
                        ItemId::from_string("heartbeat-user".into()),
                        "renew the claimed model effect",
                    )],
                    turn_context_items: Vec::new(),
                    turn_context_platform: Default::default(),
                    turn_context_components: Vec::new(),
                    submission_id: Some("heartbeat-submission".into()),
                    submitted_at_epoch_ms: Some(1_000),
                    delivery: AgentInputDelivery::CurrentTurn,
                },
                created_at_ms: 1_000,
            },
        })
        .unwrap(),
    );
    let run_id = begun["begin"]["run_id"].as_str().unwrap().to_string();
    let invocation_config_digest = begun["invocation_config_digest"]
        .as_str()
        .unwrap()
        .to_string();
    let claimed: CommittedHostRequests = serde_json::from_value(execute(
        &socket,
        "heartbeat-claim",
        serde_json::to_value(ApplicationCommand::ClaimCommittedHostRequests {
            input: ClaimCommittedHostRequests {
                run_id: run_id.clone(),
                worker_generation: "service-generation".into(),
                lease_owner: "service-worker".into(),
                claimed_at_ms: 1_100,
                lease_expires_at_ms: 2_000,
                limit: 1,
            },
        })
        .unwrap(),
    ))
    .unwrap();
    let mut model = claimed
        .requests
        .into_iter()
        .next()
        .expect("claimed model effect");
    let original = model.delivery.clone().expect("claimed delivery lease");

    execute(
        &socket,
        "heartbeat-start",
        serde_json::to_value(ApplicationCommand::ReduceAndAcceptHostResponse {
            input: ReduceAndAcceptHostResponse {
                conversation_id: "service-memory".into(),
                run_id: run_id.clone(),
                invocation_config_digest: invocation_config_digest.clone(),
                response: response_for(&model, 1, HostResponse::ExecutionStarted),
                received_at_ms: 1_200,
            },
        })
        .unwrap(),
    );
    let started = read_effect_delivery_attempt(&database, &original.attempt_id);

    let renewed: EffectDeliveryLease = serde_json::from_value(execute(
        &socket,
        "heartbeat-renew",
        serde_json::to_value(ApplicationCommand::HeartbeatEffectDelivery {
            input: HeartbeatEffectDelivery {
                run_id: run_id.clone(),
                attempt_id: original.attempt_id.clone(),
                worker_generation: original.worker_generation.clone(),
                lease_owner: original.lease_owner.clone(),
                lease_token: original.lease_token.clone(),
                current_lease_expires_at_ms: original.lease_expires_at_ms,
                heartbeat_at_ms: 1_300,
                requested_lease_expires_at_ms: 3_000,
            },
        })
        .unwrap(),
    ))
    .unwrap();
    assert_eq!(renewed.attempt_id, original.attempt_id);
    assert_eq!(renewed.worker_generation, original.worker_generation);
    assert_eq!(renewed.lease_owner, original.lease_owner);
    assert_eq!(renewed.lease_token, original.lease_token);
    assert_eq!(renewed.lease_expires_at_ms, 3_000);

    let persisted = read_effect_delivery_attempt(&database, &original.attempt_id);
    assert_eq!(persisted.heartbeat_at_ms, Some(1_300));
    assert_eq!(persisted.lease_expires_at_ms, Some(3_000));
    assert_eq!(persisted.revision, started.revision + 1);

    model.delivery = Some(renewed);
    execute(
        &socket,
        "heartbeat-progress",
        serde_json::to_value(ApplicationCommand::ReduceAndAcceptHostResponse {
            input: ReduceAndAcceptHostResponse {
                conversation_id: "service-memory".into(),
                run_id,
                invocation_config_digest,
                response: response_for(
                    &model,
                    2,
                    HostResponse::ToolProgress {
                        title: "renewed".into(),
                        detail: Some("accepted after heartbeat".into()),
                        metadata: Value::Null,
                    },
                ),
                received_at_ms: 1_400,
            },
        })
        .unwrap(),
    );
}

#[cfg(target_os = "macos")]
fn resident_kib(pid: u32) -> u64 {
    let output = Command::new("ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output()
        .expect("measure service RSS");
    String::from_utf8(output.stdout)
        .expect("RSS output")
        .trim()
        .parse()
        .expect("numeric RSS")
}

#[cfg(target_os = "macos")]
#[test]
fn long_stream_keeps_real_service_rss_bounded_and_evicts_terminal_core() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("stream-memory.db");
    let socket = directory.path().join("stream-memory.sock");
    let service = start_service(&database, &socket);

    let begun = execute(
        &socket,
        "stream-begin",
        serde_json::to_value(ApplicationCommand::BeginAssistantTurn {
            input: BeginAssistantTurn {
                expected_configuration_revision: None,
                expected_parent_message_id: None,
                conversation_id: "service-memory".to_string(),
                conversation_title: "Memory".to_string(),
                user_message_id: "service-memory-user".to_string(),
                user_part_id: "service-memory-user:text".to_string(),
                user_text: "stream without retaining recursive checkpoints".to_string(),
                user_content_json: None,
                assistant_message_id: "service-memory-assistant".to_string(),
                context_checkpoint_id: None,
                core_config: stream_config(),
                submission: AgentSubmission::UserInput {
                    items: vec![AgentItem::user_message(
                        ItemId::from_string("service-memory-user".to_string()),
                        "stream without retaining recursive checkpoints",
                    )],
                    turn_context_items: Vec::new(),
                    turn_context_platform: Default::default(),
                    turn_context_components: Vec::new(),
                    submission_id: Some("service-memory-submission".to_string()),
                    submitted_at_epoch_ms: Some(1_000),
                    delivery: AgentInputDelivery::CurrentTurn,
                },
                created_at_ms: 1_000,
            },
        })
        .unwrap(),
    );
    let initial_window = execute(
        &socket,
        "stream-initial-window",
        serde_json::to_value(ApplicationCommand::LoadConversationProjectionWindowV1 {
            conversation_id: "service-memory".into(),
            before_ordinal: None,
            limit: 4,
            content_byte_limit: 4096,
        })
        .unwrap(),
    );
    assert_eq!(
        initial_window["event_sequence"], 1,
        "first turn must publish one bounded refresh even when it creates the conversation"
    );
    let initial_messages = initial_window["projection"]["messages"].as_array().unwrap();
    assert_eq!(initial_messages.len(), 2);
    assert_eq!(
        initial_messages[1]["message_id"], "service-memory-assistant",
        "assistant placeholder must exist before any model response"
    );
    let run_id = begun["begin"]["run_id"].as_str().unwrap().to_string();
    let invocation_config_digest = begun["invocation_config_digest"]
        .as_str()
        .unwrap()
        .to_string();
    let committed: CommittedHostRequests = serde_json::from_value(execute(
        &socket,
        "stream-claim-effects",
        serde_json::to_value(ApplicationCommand::ClaimCommittedHostRequests {
            input: ClaimCommittedHostRequests {
                run_id: run_id.clone(),
                worker_generation: "stream-generation".into(),
                lease_owner: "stream-worker".into(),
                claimed_at_ms: 1_001,
                lease_expires_at_ms: 10_000,
                limit: 1,
            },
        })
        .unwrap(),
    ))
    .unwrap();
    let model = committed
        .requests
        .into_iter()
        .find(|request| matches!(request.payload, HostRequest::ModelStream { .. }))
        .expect("model request");
    let text_part_id = match &model.payload {
        HostRequest::ModelStream { text_part_id, .. } => text_part_id.clone(),
        _ => unreachable!(),
    };
    execute(
        &socket,
        "stream-execution-started",
        serde_json::to_value(ApplicationCommand::ReduceAndAcceptHostResponse {
            input: ReduceAndAcceptHostResponse {
                conversation_id: "service-memory".to_string(),
                run_id: run_id.clone(),
                invocation_config_digest: invocation_config_digest.clone(),
                response: response_for(&model, 1, HostResponse::ExecutionStarted),
                received_at_ms: 1_002,
            },
        })
        .unwrap(),
    );
    execute(
        &socket,
        "stream-model-attempt-started",
        serde_json::to_value(ApplicationCommand::ReduceAndAcceptHostResponse {
            input: ReduceAndAcceptHostResponse {
                conversation_id: "service-memory".to_string(),
                run_id: run_id.clone(),
                invocation_config_digest: invocation_config_digest.clone(),
                response: response_for(
                    &model,
                    2,
                    HostResponse::ModelAttemptStarted {
                        provider: "fixture-provider".into(),
                        model: "fixture-model".into(),
                        reason: "initial".into(),
                    },
                ),
                received_at_ms: 1_003,
            },
        })
        .unwrap(),
    );

    let baseline = resident_kib(service.0.id());
    let mut peak = baseline;
    let sync_connection = rusqlite::Connection::open_with_flags(
        &database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let queue_bytes = || -> i64 {
        sync_connection.query_row(
        "SELECT COALESCE(SUM(length(CAST(payload_json AS BLOB))),0) FROM sync_outbox WHERE status='pending'",
        [], |row| row.get(0),
    ).unwrap()
    };
    let mut queue_samples = Vec::new();
    let projection_event_count = || -> i64 {
        sync_connection.query_row(
            "SELECT COUNT(*) FROM projection_events WHERE scope_type='conversation' AND scope_id='service-memory'",
            [], |row| row.get(0),
        ).unwrap()
    };
    let before_deltas = projection_event_count();
    let journal_client =
        nola_service_client::ApplicationServiceClient::with_socket(&database, &socket);
    let mut prior_publication: Option<nola_domain::assistant_journal::AssistantJournalCursor> =
        None;
    for delta_index in 1_u64..=2_048 {
        let sequence = delta_index + 2;
        let accepted = execute(
            &socket,
            &format!("stream-{delta_index}"),
            serde_json::to_value(ApplicationCommand::ReduceAndAcceptHostResponse {
                input: ReduceAndAcceptHostResponse {
                    conversation_id: "service-memory".to_string(),
                    run_id: run_id.clone(),
                    invocation_config_digest: invocation_config_digest.clone(),
                    response: response_for(
                        &model,
                        sequence,
                        HostResponse::ModelDelta {
                            channel: ModelDeltaChannel::Text,
                            part_id: text_part_id.clone(),
                            delta: "x".to_string(),
                        },
                    ),
                    received_at_ms: 1_003 + delta_index as i64,
                },
            })
            .unwrap(),
        );
        let publications: Vec<nola_domain::assistant_journal::AssistantJournalCursor> =
            serde_json::from_value(accepted["commit"]["assistant_publications"].clone()).unwrap();
        assert_eq!(
            publications.len(),
            1,
            "each accepted delta publishes its committed journal watermark"
        );
        let publication = publications.into_iter().next().unwrap();
        assert_eq!(publication.identity.conversation_id, "service-memory");
        assert_eq!(publication.identity.run_id, run_id);
        assert_eq!(
            publication.identity.assistant_message_id,
            "service-memory-assistant"
        );
        if let Some(prior) = &prior_publication {
            assert_eq!(publication.identity, prior.identity);
            assert!(
                publication.sequence > prior.sequence,
                "journal notification did not advance"
            );
            assert_ne!(publication.record_digest, prior.record_digest);
        }
        if delta_index % 64 == 0 {
            peak = peak.max(resident_kib(service.0.id()));
        }
        if matches!(delta_index, 512 | 1024 | 2048) {
            queue_samples.push(queue_bytes());
            let snapshot: nola_application::AssistantJournalSnapshotHandle = journal_client
                .execute(ApplicationCommand::OpenAssistantJournalSnapshot {
                    at: publication.clone(),
                })
                .unwrap();
            let parts: nola_application::AssistantJournalPartPage = journal_client
                .execute(ApplicationCommand::ReadAssistantJournalParts {
                    snapshot: snapshot.clone(),
                    after_part_id: None,
                    max_records: 128,
                    max_encoded_bytes: 65536,
                })
                .unwrap();
            assert_eq!(parts.at, publication);
            assert!(parts.next_part_id.is_none());
            let text = parts
                .parts
                .iter()
                .find(|part| part.part_id == text_part_id.as_str())
                .expect("published assistant text part");
            assert_eq!(text.bodies.len(), 1);
            let range: nola_application::AssistantJournalTextRange = journal_client
                .execute(ApplicationCommand::ReadAssistantJournalRange {
                    snapshot: snapshot.clone(),
                    part_id: text.part_id.clone(),
                    content: text.bodies[0].clone(),
                    offset_bytes: nola_domain::assistant_journal_operations::JournalNumber(0),
                    limit_bytes: 4096,
                })
                .unwrap();
            assert_eq!(range.text, "x".repeat(delta_index as usize));
            assert!(range.end_of_content);
            let _: Value = journal_client
                .execute(ApplicationCommand::ReleaseAssistantJournalSnapshot { snapshot })
                .unwrap();
        }
        prior_publication = Some(publication);
    }
    assert_eq!(
        projection_event_count(),
        before_deltas,
        "body-only deltas must not invalidate/reload the conversation window"
    );

    // Command receipts are durable ACKs plus journal targets, never an
    // accumulating assistant body. The 2,048th single-byte delta must be as
    // bounded as the first, including the actually persisted wire result.
    let receipt_bytes: (i64, i64) = sync_connection.query_row(
        "SELECT MIN(length(CAST(result_json AS BLOB))), MAX(length(CAST(result_json AS BLOB))) FROM command_receipts WHERE request_id GLOB 'stream-[0-9]*'",
        [], |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    assert!(
        receipt_bytes.1 - receipt_bytes.0 < 1_024,
        "stream ACK grew with transcript: {receipt_bytes:?}"
    );
    let body_events: i64 = sync_connection.query_row(
        "SELECT COUNT(*) FROM command_receipts, json_each(command_receipts.result_json, '$.events') AS event WHERE request_id GLOB 'stream-[0-9]*' AND json_extract(event.value, '$.event.type')='part_updated'",
        [], |row| row.get(0),
    ).unwrap();
    assert_eq!(
        body_events, 0,
        "journal is the only streamed Part publication"
    );

    execute(
        &socket,
        "stream-complete",
        serde_json::to_value(ApplicationCommand::ReduceAndAcceptHostResponse {
            input: ReduceAndAcceptHostResponse {
                conversation_id: "service-memory".to_string(),
                run_id,
                invocation_config_digest,
                response: response_for(
                    &model,
                    2_051,
                    HostResponse::ModelCompleted {
                        items: Vec::new(),
                        tool_calls: Vec::new(),
                        usage: json!({}),
                    },
                ),
                received_at_ms: 4_000,
            },
        })
        .unwrap(),
    );
    thread::sleep(Duration::from_millis(100));
    let terminal = resident_kib(service.0.id());
    assert!(
        peak <= baseline + 256 * 1_024,
        "service RSS grew without a bound: baseline={baseline} KiB, peak={peak} KiB"
    );
    assert!(
        terminal <= baseline + 128 * 1_024,
        "terminal Core was not released: baseline={baseline} KiB, terminal={terminal} KiB"
    );
    let total_queue_bytes = queue_bytes();
    eprintln!("stream_replica_queue: samples_512_1024_2048={queue_samples:?} completed_bytes={total_queue_bytes}");

    // The command-receipt wrapper used to reload/replay the full checkpoint
    // before and after every delta just to publish a second transcript diff.
    // Exercise the actual service wrapper, not only Application's hot reducer:
    // assistant bodies belong exclusively to the assistant journal, while the
    // conversation feed carries only structural/control invalidations, including
    // completion. Journal watermarks above, not a parallel window invalidation,
    // are the lossless streaming publication contract.
    let (refresh_count, legacy_body_count, largest_refresh): (i64, i64, i64) = sync_connection
        .query_row(
            "SELECT COUNT(*),
                    COALESCE(SUM(CASE WHEN json_extract(payload_json,'$.mutation.type')
                        = 'invalidate_projection' THEN 0 ELSE 1 END),0),
                    COALESCE(MAX(length(CAST(payload_json AS BLOB))),0)
             FROM projection_events WHERE scope_type='conversation' AND scope_id='service-memory'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert!(
        refresh_count > before_deltas,
        "terminal structural invalidation was lost"
    );
    assert_eq!(
        legacy_body_count, 0,
        "live Core commands republished legacy transcript diffs"
    );
    assert!(
        largest_refresh < 1024,
        "refresh retained cumulative content: {largest_refresh}"
    );

    use nola_protocol::{
        PendingSyncChange, RemoteSyncChange, RemoteSyncPage, RemoteSyncPageResult,
        ReplicaSyncCommand,
    };
    use nola_service_client::ApplicationServiceClient;
    let sender = ApplicationServiceClient::with_socket(&database, &socket);
    let receiver_db = directory.path().join("stream-receiver.db");
    let receiver_socket = directory.path().join("stream-receiver.sock");
    let mut receiver_service = start_service(&receiver_db, &receiver_socket);
    let receiver = ApplicationServiceClient::with_socket(&receiver_db, &receiver_socket);
    let mut cursor = 0_i64;
    let mut pages = 0;
    let mut last = None;
    loop {
        let pending: Vec<PendingSyncChange> = sender
            .execute(ReplicaSyncCommand::PendingSyncChanges { limit: 128 })
            .unwrap();
        if pending.is_empty() {
            break;
        }
        pages += 1;
        assert!(pages <= 32, "bounded stream fixture must finish sync");
        let changes: Vec<_> = pending
            .iter()
            .map(|item| {
                cursor += 1;
                RemoteSyncChange {
                    cursor,
                    operation_id: item.operation_id.clone(),
                    entity_type: item.entity_type.clone(),
                    entity_id: item.entity_id.clone(),
                    revision: item.revision,
                    deleted: item.deleted,
                    payload_json: item.payload_json.clone(),
                    created_at_ms: item.created_at_ms,
                }
            })
            .collect();
        let outcome: RemoteSyncPageResult = receiver
            .execute(ReplicaSyncCommand::ApplyRemoteSyncPage {
                input: RemoteSyncPage {
                    replica_id: "stream-fixture".into(),
                    changes: changes.clone(),
                    next_cursor: cursor,
                    applied_at_ms: 5000,
                },
            })
            .unwrap();
        assert_eq!(outcome.applied, changes.len());
        assert_eq!(outcome.skipped, 0);
        last = changes.last().cloned();
        for change in &changes {
            let acknowledged: bool = sender
                .execute(ReplicaSyncCommand::ResolveSyncChange {
                    operation_id: change.operation_id.clone(),
                    status: "acknowledged".into(),
                    remote_cursor: Some(change.cursor),
                    resolved_at_ms: 5000,
                })
                .unwrap();
            assert!(acknowledged);
        }
        if pages == 1 {
            drop(receiver_service);
            receiver_service = start_service(&receiver_db, &receiver_socket);
        }
    }
    let canonical_rows = |path: &Path| -> String {
        let connection =
            rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                .unwrap();
        connection.query_row("SELECT json_group_array(json_object(
            'message_id',id,'run_id',run_id,'role',role,'status',status,'ordinal',ordinal,
            'part_id',part_id,'kind',kind,'part_ordinal',part_ordinal,'state',state,'text',content_text,'json',content_json,'digest',content_digest))
            FROM (SELECT m.id,m.run_id,m.role,m.status,m.ordinal,p.id AS part_id,p.kind,p.ordinal AS part_ordinal,
                p.state,p.content_text,p.content_json,p.content_digest
                FROM messages m JOIN message_parts p ON p.message_id=m.id
                WHERE m.conversation_id='service-memory' ORDER BY m.ordinal,p.ordinal)", [], |row| row.get(0)).unwrap()
    };
    let expected = canonical_rows(&database);
    assert_eq!(canonical_rows(&receiver_db), expected);
    drop(receiver_service);
    let _receiver_service = start_service(&receiver_db, &receiver_socket);
    let mut replay = last.expect("stream must publish content facts");
    replay.cursor = cursor + 1;
    let outcome: RemoteSyncPageResult = receiver
        .execute(ReplicaSyncCommand::ApplyRemoteSyncPage {
            input: RemoteSyncPage {
                replica_id: "stream-fixture".into(),
                changes: vec![replay],
                next_cursor: cursor + 1,
                applied_at_ms: 6000,
            },
        })
        .unwrap();
    assert_eq!(outcome.skipped, 1);
    assert_eq!(canonical_rows(&receiver_db), expected);
    let echoes: Vec<PendingSyncChange> = receiver
        .execute(ReplicaSyncCommand::PendingSyncChanges { limit: 1 })
        .unwrap();
    assert!(echoes.is_empty());
    assert!(
        pages > 1,
        "exercise page boundaries rather than one full snapshot"
    );
    assert!(
        total_queue_bytes < 2 * 1024 * 1024,
        "small streamed reply exceeded replica budget"
    );
    let earlier_growth = queue_samples[1] - queue_samples[0];
    let later_growth = queue_samples[2] - queue_samples[1];
    assert!(
        earlier_growth > 0 && later_growth * 4 <= earlier_growth * 9,
        "queue grows quadratically instead of publishing incremental content: {queue_samples:?}"
    );
}

#[test]
fn pending_remote_intent_survives_real_service_restart_without_canonical_execution() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("nola.db");
    let socket = directory.path().join("nola-service.sock");
    let mut service = start_service(&database, &socket);

    let health = request(&socket, "health-1", "health", json!({}));
    assert_eq!(health["service"], "nola-application");

    let prepared = execute(
        &socket,
        "begin-1",
        json!({
            "type": "prepare_remote_submission",
            "input": {
                "conversation_id": "conversation-1",
                "conversation_title": "Durable conversation",
                "source_host_id": "service-process-e2e-host",
                "client_id": "fixture-client",
                "submission_id": "run-1",
                "user_message_id": "user-message-1",
                "user_part_id": "user-message-1:text",
                "user_text": "Preserve this exact user message.",
                "user_content_json": null,
                "assistant_message_id": "assistant-message-1",
                "assistant_part_id": "assistant-message-1:text",
                "context_checkpoint_id": null,
                "created_at_ms": 1000
            }
        }),
    );
    let bound = execute(
        &socket,
        "bind-1",
        json!({"type":"bind_remote_submission_request","identity":prepared["identity"],
        "request":{"user_text":"Preserve this exact user message."}}),
    );

    service.0.kill().unwrap();
    service.0.wait().unwrap();
    drop(service);
    let service = start_service(&database, &socket);

    let conversation = execute(
        &socket,
        "load-1",
        json!({"type": "load_conversation", "conversation_id": "conversation-1"}),
    );
    let messages = conversation["messages"].as_array().unwrap();
    assert!(
        messages.is_empty(),
        "prepared intent must not publish messages"
    );
    let intent = execute(
        &socket,
        "pending-1",
        json!({"type":"read_remote_submission_intent","conversation_id":"conversation-1"}),
    );
    assert_eq!(intent["identity"]["submission_id"], "run-1");
    assert_eq!(intent["identity"]["user_message_id"], "user-message-1");
    assert_eq!(intent["phase"], "bound");
    assert_eq!(intent, bound);
    let frozen = execute(
        &socket,
        "frozen-request",
        json!({"type":"read_remote_submission_request","identity":intent["identity"],"request_digest":intent["request_digest"]}),
    );
    assert_eq!(frozen["user_text"], "Preserve this exact user message.");
    let active_run = execute(
        &socket,
        "active-run-1",
        json!({"type": "active_run", "conversation_id": "conversation-1"}),
    );
    assert!(active_run.is_null());
    drop(service);
}

#[test]
fn duplicate_request_identity_replays_one_durable_terminal_receipt() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("receipt.db");
    let socket = directory.path().join("receipt.sock");
    let _service = start_service(&database, &socket);
    let command = json!({
        "type": "prepare_remote_submission",
        "input": {
            "conversation_id": "receipt-conversation",
            "conversation_title": "One durable command",
            "source_host_id": "service-process-e2e-host",
            "client_id": "fixture-client",
            "submission_id": "receipt-run",
            "user_message_id": "receipt-user",
            "user_part_id": "receipt-user:text",
            "user_text": "Execute this request exactly once.",
            "user_content_json": null,
            "assistant_message_id": "receipt-assistant",
            "assistant_part_id": "receipt-assistant:text",
            "context_checkpoint_id": null,
            "created_at_ms": 1_000
        }
    });

    let first = execute(&socket, "stable-request", command.clone());
    let replay = execute(&socket, "stable-request", command);
    assert_eq!(first, replay);

    let scope = json!({
        "type": "conversation",
        "conversation_id": "receipt-conversation"
    });
    let suffix = request(
        &socket,
        "projection-suffix-1",
        "projection_event_suffix",
        json!({
            "scope": scope,
            "after": {
                "scope": {
                    "type": "conversation",
                    "conversation_id": "receipt-conversation"
                },
                "sequence": 0
            },
            "limit": 32
        }),
    );
    let events = suffix["events"].as_array().unwrap();
    assert!(!events.is_empty());
    assert_eq!(events[0]["cursor"]["sequence"], 1);
    for (index, event) in events.iter().enumerate() {
        assert_eq!(event["cursor"]["sequence"], (index + 1) as u64);
        assert_eq!(event["scope"]["type"], "conversation");
        assert_eq!(event["scope"]["conversation_id"], "receipt-conversation");
        assert!(event.get("mutation").is_some());
    }
    for pair in events.windows(2) {
        assert_eq!(
            pair[1]["cursor"]["sequence"].as_u64().unwrap(),
            pair[0]["cursor"]["sequence"].as_u64().unwrap() + 1
        );
    }
    let typed_suffix: nola_protocol::EventSuffix = serde_json::from_value(suffix.clone())
        .expect("service suffix must use the current typed projection protocol");
    typed_suffix
        .validate()
        .expect("service suffix identities, mutations and cursors must be valid");
    assert_eq!(
        suffix["next_cursor"]["sequence"],
        events.last().unwrap()["cursor"]["sequence"]
    );
    let event_count = events.len();

    let projection = execute(
        &socket,
        "conversation-projection-1",
        json!({
            "type": "load_conversation_projection_v1",
            "conversation_id": "receipt-conversation"
        }),
    );
    assert_eq!(projection["surface"]["committed_message_ids"], json!([]));
    assert_eq!(projection["surface"]["live_message_id"], Value::Null);
    assert_eq!(projection["surface"]["show_assistant_placeholder"], false);

    let caught_up = request(
        &socket,
        "projection-suffix-caught-up",
        "projection_event_suffix",
        json!({
            "scope": {
                "type": "conversation",
                "conversation_id": "receipt-conversation"
            },
            "after": suffix["next_cursor"].clone(),
            "limit": 32,
            "wait_ms": 1
        }),
    );
    assert!(caught_up["events"].as_array().unwrap().is_empty());
    assert_eq!(caught_up["next_cursor"], suffix["next_cursor"]);
    serde_json::from_value::<nola_protocol::EventSuffix>(caught_up)
        .expect("decode caught-up suffix")
        .validate()
        .expect("caught-up suffix must retain a valid cursor");

    let replayed_suffix = request(
        &socket,
        "projection-suffix-2",
        "projection_event_suffix",
        json!({
            "scope": {
                "type": "conversation",
                "conversation_id": "receipt-conversation"
            },
            "after": {
                "scope": {
                    "type": "conversation",
                    "conversation_id": "receipt-conversation"
                },
                "sequence": 0
            },
            "limit": 32
        }),
    );
    assert_eq!(
        replayed_suffix["events"].as_array().unwrap().len(),
        event_count
    );
    let replayed_suffix: nola_protocol::EventSuffix =
        serde_json::from_value(replayed_suffix).expect("decode replayed suffix");
    replayed_suffix
        .validate()
        .expect("validate replayed suffix");
    assert_eq!(
        replayed_suffix, typed_suffix,
        "command replay changed canonical events"
    );

    let connection = rusqlite::Connection::open(&database).unwrap();
    let receipt_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM command_receipts
             WHERE client_id='service-process-e2e' AND request_id='stable-request'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let message_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM messages WHERE conversation_id='receipt-conversation'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(receipt_count, 1);
    assert_eq!(
        message_count, 0,
        "preparation and command replay must not publish messages before source admission"
    );
    let run_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM runs WHERE conversation_id='receipt-conversation'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        run_count, 0,
        "a prepared remote intent is not an admitted Run"
    );
    let intent = execute(
        &socket,
        "read-replayed-intent",
        json!({"type":"read_remote_submission_intent","conversation_id":"receipt-conversation"}),
    );
    assert_eq!(
        intent, first,
        "replay must preserve the exact prepared intent"
    );
}
