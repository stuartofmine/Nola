// SPDX-License-Identifier: Apache-2.0
//! Consolidated transport receipt timeout recovery and envelope CAS precondition scenarios.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::process::{Child, Command, Stdio};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    mpsc, Arc,
};
use std::thread;
use std::time::{Duration, Instant};

use crate::support::model_identity as fixture_model_identity;
use nola_application::{ApplicationCommand, ModelCommand, ModelProfile};
use nola_protocol::{CommandReceiptOutcome, APPLICATION_SERVICE_PROTOCOL_VERSION};
use nola_service_client::ApplicationServiceClient;
use serde_json::{json, Value};

struct Service(Child);
impl Drop for Service {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct Proxy {
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Drop for Proxy {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn start_service(database: &std::path::Path, socket: &std::path::Path) -> Service {
    let _ = std::fs::remove_file(socket);
    let mut service = Service(
        Command::new(env!("CARGO_BIN_EXE_nola-service"))
            .arg("--database")
            .arg(database)
            .arg("--socket")
            .arg(socket)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let client = ApplicationServiceClient::with_socket(database, socket);
    let deadline = Instant::now() + Duration::from_secs(5);
    while client.health().is_err() {
        assert!(
            service.0.try_wait().unwrap().is_none(),
            "service exited during startup"
        );
        assert!(Instant::now() < deadline, "service did not become ready");
        thread::sleep(Duration::from_millis(10));
    }
    service
}

fn admission_command() -> ApplicationCommand {
    use nola_core::agent::{host::*, protocol::*};
    let capability = HostCapabilityDescriptor {
        id: "model.stream".into(),
        title: "Timeout fixture".into(),
        route: HostCapabilityRoute::Model,
        availability: CapabilityAvailability::Available,
        exposure: HostCapabilityExposure::Internal,
        permission_domain: None,
        input_schema: json!({"type":"object"}),
        input_normalization: Default::default(),
        output_policy: HostOutputPolicy {
            preview_char_limit: 4000,
            persist_full_output: true,
            allow_artifacts: true,
        },
        supports_streaming: true,
        supports_argument_streaming: false,
        parallelism: HostCapabilityParallelism::Serial,
        cancellation: HostCancellationMode::Cooperative,
        replay_safety: HostReplaySafety::Recoverable,
    };
    ApplicationCommand::BeginAssistantTurn {
        input: nola_application::BeginAssistantTurn {
            expected_configuration_revision: None,
            expected_parent_message_id: None,
            conversation_id: "admission".into(),
            conversation_title: "Timeout".into(),
            user_message_id: "user".into(),
            user_part_id: "user-part".into(),
            user_text: "Work".into(),
            user_content_json: None,
            assistant_message_id: "assistant".into(),
            context_checkpoint_id: None,
            core_config: AgentCoreConfig {
                turn_identity: fixture_model_identity::identity(),
                session_id: Some(SessionId::from_string("core-admission".into())),
                model: Some("fixture".into()),
                host_catalog: Some(HostCapabilityCatalog {
                    host_id: HostId("fixture".into()),
                    revision: 1,
                    availability_epoch: 1,
                    lease: None,
                    capabilities: [("model.stream".into(), capability)].into_iter().collect(),
                }),
                ..Default::default()
            },
            submission: AgentSubmission::UserInput {
                items: vec![AgentItem::user_message(
                    ItemId::from_string("user".into()),
                    "Work",
                )],
                turn_context_items: vec![],
                turn_context_platform: Default::default(),
                turn_context_components: vec![],
                submission_id: Some("admitted-run".into()),
                submitted_at_epoch_ms: Some(1000),
                delivery: AgentInputDelivery::CurrentTurn,
            },
            created_at_ms: 1000,
        },
    }
}

#[test]
fn committed_model_and_admission_timeouts_recover_receipts_and_replay_across_restart() {
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("state.db");
    let service_socket = root.path().join("service.sock");
    let proxy_socket = root.path().join("proxy.sock");
    let service = start_service(&database, &service_socket);
    let direct = ApplicationServiceClient::with_socket(&database, &service_socket);

    let listener = UnixListener::bind(&proxy_socket).unwrap();
    listener.set_nonblocking(true).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let worker_stop = Arc::clone(&stop);
    let (released_tx, released_rx) = mpsc::channel();
    let model_submissions = Arc::new(AtomicUsize::new(0));
    let admission_submissions = Arc::new(AtomicUsize::new(0));
    let worker_model_subs = Arc::clone(&model_submissions);
    let worker_adm_subs = Arc::clone(&admission_submissions);
    let upstream_socket = service_socket.clone();
    let worker = thread::spawn(move || {
        let mut delayed_ids = std::collections::BTreeSet::new();
        while !worker_stop.load(Ordering::Acquire) {
            let (mut downstream, _) = match listener.accept() {
                Ok(connection) => connection,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(5));
                    continue;
                }
                Err(error) => panic!("proxy accept: {error}"),
            };
            downstream.set_nonblocking(false).unwrap();
            downstream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
            downstream.set_write_timeout(Some(Duration::from_secs(2))).unwrap();
            let mut request = Vec::new();
            BufReader::new(&mut downstream)
                .read_until(b'\n', &mut request)
                .unwrap();
            let envelope: Value = serde_json::from_slice(&request).unwrap();
            let req_id = envelope["request_id"].as_str().unwrap_or_default().to_string();
            if req_id == "timeout-write-model" {
                worker_model_subs.fetch_add(1, Ordering::SeqCst);
            } else if req_id == "timeout-write-admission" {
                worker_adm_subs.fetch_add(1, Ordering::SeqCst);
            }
            let mut upstream = UnixStream::connect(&upstream_socket).unwrap();
            upstream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
            upstream.set_write_timeout(Some(Duration::from_secs(2))).unwrap();
            upstream.write_all(&request).unwrap();
            let mut receipt = Vec::new();
            BufReader::new(upstream)
                .read_until(b'\n', &mut receipt)
                .unwrap();
            if (req_id == "timeout-write-model" || req_id == "timeout-write-admission")
                && delayed_ids.insert(req_id)
            {
                thread::sleep(Duration::from_millis(400));
                let _ = downstream.write_all(&receipt);
                released_tx.send(()).unwrap();
            } else {
                let _ = downstream.write_all(&receipt);
            }
        }
    });
    let proxy = Proxy {
        stop,
        worker: Some(worker),
    };
    let client = ApplicationServiceClient::with_socket(&database, &proxy_socket)
        .with_request_timeout(Duration::from_millis(150))
        .unwrap();

    let model_cmd = || ApplicationCommand::Model {
        command: ModelCommand::UpsertProfile {
            input: ModelProfile {
                id: "once".into(),
                display_name: "Once".into(),
                provider: "fixture".into(),
                model_name: "once".into(),
                runtime: "remote".into(),
                source_json: "{}".into(),
                parameters_json: "{}".into(),
                selected: true,
                enabled: true,
                revision: 0,
                created_at_ms: 1,
                updated_at_ms: 1,
                deleted_at_ms: None,
            },
        },
    };
    let model_load = || ApplicationCommand::Model {
        command: ModelCommand::GetProfile { id: "once".into() },
    };
    let adm_load = || ApplicationCommand::ActiveRun {
        conversation_id: "admission".into(),
    };

    let mut post_restart_checks = Vec::new();
    for (admission, req_id, counter) in [
        (false, "timeout-write-model", &model_submissions),
        (true, "timeout-write-admission", &admission_submissions),
    ] {
        let command = || {
            if admission {
                admission_command()
            } else {
                model_cmd()
            }
        };
        let load = || {
            if admission {
                adm_load()
            } else {
                model_load()
            }
        };
        let error = client
            .execute_typed_with_request_id::<Value, _>(req_id, command())
            .unwrap_err();
        assert!(error.requires_reconciliation(), "{error}");
        assert!(error.to_string().contains(&format!("request_id={req_id}")));
        released_rx.recv_timeout(Duration::from_secs(3)).unwrap();
        assert_eq!(counter.load(Ordering::SeqCst), 1);
        let receipt = client
            .command_receipt(req_id)
            .unwrap()
            .expect("durable receipt after timeout");
        assert!(matches!(
            receipt.outcome,
            CommandReceiptOutcome::Committed { .. }
        ));
        let recovered_result: Value = client.reconcile_command_result(&error).unwrap();
        if admission {
            assert_eq!(recovered_result["begin"]["run_id"], "admitted-run");
            assert!(recovered_result.get("checkpoint").is_none());
            assert!(recovered_result.get("snapshot").is_none());
        }
        let before: Value = client.execute(load()).unwrap();
        let replay: Value = client.execute_with_request_id(req_id, command()).unwrap();
        assert_eq!(replay, recovered_result);
        assert_eq!(counter.load(Ordering::SeqCst), 2);
        let after: Value = client.execute(load()).unwrap();
        assert_eq!(before, after);
        assert_eq!(
            client.command_receipt(req_id).unwrap().unwrap().request_id,
            receipt.request_id
        );
        let mut altered = serde_json::to_value(command()).unwrap();
        if admission {
            altered["input"]["user_text"] = Value::String("changed".into());
        } else {
            altered["command"]["input"]["parameters_json"] =
                Value::String("{\"temperature\":0.99}".into());
        }
        assert!(client
            .execute_with_request_id::<Value, _>(req_id, altered)
            .is_err());
        assert_eq!(client.execute::<Value, _>(load()).unwrap(), before);
        post_restart_checks.push((admission, req_id.to_string(), before, recovered_result, receipt));
    }

    drop(proxy);
    drop(service);
    let _restarted = start_service(&database, &service_socket);
    for (admission, req_id, before, recovered_result, receipt) in post_restart_checks {
        let command = if admission {
            admission_command()
        } else {
            model_cmd()
        };
        let load = if admission { adm_load() } else { model_load() };
        assert_eq!(direct.execute::<Value, _>(load).unwrap(), before);
        let cold_replay: Value = direct.execute_with_request_id(&req_id, command).unwrap();
        assert_eq!(cold_replay, recovered_result);
        let recovered = direct.command_receipt(&req_id).unwrap().unwrap();
        assert_eq!(
            serde_json::to_value(recovered).unwrap(),
            serde_json::to_value(receipt).unwrap()
        );
    }
}

#[test]
fn revision_preconditions_conflict_reject_and_replay_durably_across_restart() {
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("state.db");
    let socket = root.path().join("service.sock");
    let service = start_service(&database, &socket);
    let client = ApplicationServiceClient::with_socket(&database, &socket);

    // 1. Missing-profile CAS conflict + unsupported DeleteProfile precondition rejection.
    let mut stream = UnixStream::connect(&socket).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
    stream.set_write_timeout(Some(Duration::from_secs(3))).unwrap();
    let envelope = json!({
        "protocol_version": APPLICATION_SERVICE_PROTOCOL_VERSION,
        "client_id": "revision-fixture", "request_id": "conditional-write",
        "expected_revision": 42,
        "command": {"type": "model", "arguments": {
            "command": {"type": "upsert_profile", "input": {
                "id": "must-not-exist", "display_name": "Fixture", "provider": "fixture",
                "model_name": "fixture", "runtime": "remote", "source_json": "{}",
                "parameters_json": "{}", "selected": false, "enabled": true,
                "revision": 0, "created_at_ms": 1, "updated_at_ms": 1, "deleted_at_ms": null
            }}
        }}
    });
    serde_json::to_writer(&mut stream, &envelope).unwrap();
    stream.write_all(b"\n").unwrap();
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line).unwrap();
    let receipt: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(receipt["status"], "conflict", "{receipt}");
    assert_eq!(receipt["conflict"]["scope_id"], "model_profile:must-not-exist");
    assert_eq!(receipt["conflict"]["expected_revision"], 42);
    assert_eq!(receipt["conflict"]["actual_revision"], 0);
    let stored: Value = client
        .execute(ApplicationCommand::Model {
            command: ModelCommand::GetProfile {
                id: "must-not-exist".into(),
            },
        })
        .unwrap();
    assert!(stored.is_null());

    let input =
        serde_json::from_value(envelope["command"]["arguments"]["command"]["input"].clone())
            .unwrap();
    let _: Value = client
        .execute(ApplicationCommand::Model {
            command: ModelCommand::UpsertProfile { input },
        })
        .unwrap();
    let load_missing = || {
        client
            .execute::<Value, _>(ApplicationCommand::Model {
                command: ModelCommand::GetProfile {
                    id: "must-not-exist".into(),
                },
            })
            .unwrap()
    };
    let before = load_missing();
    assert!(!before.is_null());
    let rejection = client
        .submit_with_revision(
            "unsupported-delete",
            Some(42),
            ApplicationCommand::Model {
                command: ModelCommand::DeleteProfile {
                    id: "must-not-exist".into(),
                    deleted_at_ms: 2,
                },
            },
        )
        .unwrap();
    match rejection.outcome {
        CommandReceiptOutcome::Rejected { error, .. } => {
            assert_eq!(error.code, "unsupported_revision_precondition");
        }
        other => panic!("unsupported condition must reject before mutation: {other:?}"),
    }
    assert_eq!(load_missing(), before);
    assert!(client.command_receipt("unsupported-delete").unwrap().is_none());

    // 2. Model parameter CAS create, winner, stale conflict, idempotent replay, and cold restart.
    let edit = |temperature: f64| ApplicationCommand::Model {
        command: ModelCommand::ConfigureParameters {
            input: ModelProfile {
                id: "cas".into(),
                display_name: "CAS".into(),
                provider: "fixture".into(),
                model_name: "fixture".into(),
                runtime: "remote".into(),
                source_json: "{}".into(),
                parameters_json: json!({"temperature": temperature}).to_string(),
                selected: false,
                enabled: true,
                revision: 0,
                created_at_ms: 1,
                updated_at_ms: 2,
                deleted_at_ms: None,
            },
        },
    };
    let load_cas = || {
        client
            .execute::<Value, _>(ApplicationCommand::Model {
                command: ModelCommand::GetProfile { id: "cas".into() },
            })
            .unwrap()
    };
    let created = client
        .submit_with_revision("create", Some(0), edit(0.1))
        .unwrap();
    let first = load_cas();
    let revision = first["revision"].as_u64().unwrap();
    assert!(revision > 0);
    assert!(matches!(
        created.outcome,
        CommandReceiptOutcome::Committed {
            committed_revision: Some(value), ..
        } if value == revision
    ));
    let winner = client
        .submit_with_revision("winner", Some(revision), edit(0.2))
        .unwrap();
    assert!(matches!(
        winner.outcome,
        CommandReceiptOutcome::Committed { .. }
    ));
    let current = load_cas();
    let conflict = client
        .submit_with_revision("stale", Some(revision), edit(0.9))
        .unwrap();
    match &conflict.outcome {
        CommandReceiptOutcome::Conflict { conflict, .. } => {
            assert_eq!(conflict.scope_id, "model_profile:cas");
            assert_eq!(conflict.expected_revision, revision);
            assert_eq!(
                conflict.actual_revision,
                current["revision"].as_u64().unwrap()
            );
        }
        other => panic!("expected conflict, got {other:?}"),
    }
    assert_eq!(load_cas(), current);
    assert_eq!(
        client
            .submit_with_revision("create", Some(0), edit(0.1))
            .unwrap(),
        created
    );
    assert_eq!(load_cas(), current);

    drop(service);
    let _restarted = start_service(&database, &socket);
    assert_eq!(
        client
            .submit_with_revision("stale", Some(revision), edit(0.9))
            .unwrap(),
        conflict
    );
    assert_eq!(load_cas(), current);
    let _: Value = client
        .execute(ApplicationCommand::Model {
            command: ModelCommand::DeleteProfile {
                id: "cas".into(),
                deleted_at_ms: 3,
            },
        })
        .unwrap();
    let deleted = client
        .submit_with_revision("recreate", Some(0), edit(0.3))
        .unwrap();
    assert!(matches!(
        deleted.outcome,
        CommandReceiptOutcome::Conflict { .. }
    ));
    assert!(load_cas().is_null());
}
