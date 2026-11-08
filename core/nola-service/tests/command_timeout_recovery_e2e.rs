// SPDX-License-Identifier: Apache-2.0
//! Real service commit followed by a deliberately delayed transport receipt.
#![cfg(unix)]

#[path = "../../nola-application/tests/support/model_identity.rs"]
mod fixture_model_identity;
use nola_application::{ApplicationCommand, ModelCommand, ModelProfile};
use nola_protocol::CommandReceiptOutcome;
use nola_service_client::ApplicationServiceClient;
use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::process::{Child, Command, Stdio};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    mpsc, Arc,
};
use std::thread;
use std::time::{Duration, Instant};

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

#[test]
fn committed_command_timeout_recovers_receipt_and_same_id_retry_does_not_write_twice() {
    exercise_committed_timeout(false);
}

#[test]
fn committed_admission_timeout_recovers_the_original_run_across_service_restart() {
    exercise_committed_timeout(true);
}

fn admission_command() -> ApplicationCommand {
    use nola_core::agent::{host::*, protocol::*};
    let capability = HostCapabilityDescriptor {
        id: "model.stream".into(), title: "Timeout fixture".into(), route: HostCapabilityRoute::Model,
        availability: CapabilityAvailability::Available, exposure: HostCapabilityExposure::Internal,
        permission_domain: None, input_schema: serde_json::json!({"type":"object"}),
        input_normalization: Default::default(),
        output_policy: HostOutputPolicy { preview_char_limit: 4000, persist_full_output: true, allow_artifacts: true },
        supports_streaming: true, supports_argument_streaming: false,
        parallelism: HostCapabilityParallelism::Serial, cancellation: HostCancellationMode::Cooperative,
        replay_safety: HostReplaySafety::Recoverable,
    };
    ApplicationCommand::BeginAssistantTurn { input: nola_application::BeginAssistantTurn {
        expected_configuration_revision: None,
        expected_parent_message_id: None,
        conversation_id: "admission".into(), conversation_title: "Timeout".into(),
        user_message_id: "user".into(), user_part_id: "user-part".into(), user_text: "Work".into(),
        user_content_json: None, assistant_message_id: "assistant".into(), context_checkpoint_id: None,
        core_config: AgentCoreConfig {
            turn_identity: fixture_model_identity::identity(),
            session_id: Some(SessionId::from_string("core-admission".into())), model: Some("fixture".into()),
            host_catalog: Some(HostCapabilityCatalog { host_id: HostId("fixture".into()), revision: 1,
                availability_epoch: 1, lease: None,
                capabilities: [("model.stream".into(), capability)].into_iter().collect() }),
            ..Default::default()
        },
        submission: AgentSubmission::UserInput {
            items: vec![AgentItem::user_message(ItemId::from_string("user".into()), "Work")],
            turn_context_items: vec![], turn_context_platform: Default::default(), turn_context_components: vec![],
            submission_id: Some("admitted-run".into()), submitted_at_epoch_ms: Some(1000),
            delivery: AgentInputDelivery::CurrentTurn,
        }, created_at_ms: 1000,
    } }
}

fn exercise_committed_timeout(admission: bool) {
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("state.db");
    let service_socket = root.path().join("service.sock");
    let proxy_socket = root.path().join("proxy.sock");
    let mut service = Service(
        Command::new(env!("CARGO_BIN_EXE_nola-service"))
            .arg("--database")
            .arg(&database)
            .arg("--socket")
            .arg(&service_socket)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let direct = ApplicationServiceClient::with_socket(&database, &service_socket);
    let deadline = Instant::now() + Duration::from_secs(5);
    while direct.health().is_err() {
        assert!(
            service.0.try_wait().unwrap().is_none(),
            "service exited during startup"
        );
        assert!(Instant::now() < deadline, "service did not become ready");
        thread::sleep(Duration::from_millis(10));
    }

    let listener = UnixListener::bind(&proxy_socket).unwrap();
    listener.set_nonblocking(true).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let worker_stop = Arc::clone(&stop);
    let (released_tx, released_rx) = mpsc::channel();
    let submissions = Arc::new(AtomicUsize::new(0));
    let worker_submissions = Arc::clone(&submissions);
    let upstream_socket = service_socket.clone();
    let worker = thread::spawn(move || {
        let mut delayed = false;
        while !worker_stop.load(Ordering::Acquire) {
            let (mut downstream, _) = match listener.accept() {
                Ok(connection) => connection,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(5));
                    continue;
                }
                Err(error) => panic!("proxy accept: {error}"),
            };
            // macOS can inherit O_NONBLOCK from the accept listener. The
            // fixture uses bounded blocking frame I/O, just like the service.
            downstream.set_nonblocking(false).unwrap();
            downstream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            downstream
                .set_write_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = Vec::new();
            BufReader::new(&mut downstream)
                .read_until(b'\n', &mut request)
                .unwrap();
            let envelope: Value = serde_json::from_slice(&request).unwrap();
            if envelope["request_id"] == "timeout-write" {
                worker_submissions.fetch_add(1, Ordering::SeqCst);
            }
            let mut upstream = UnixStream::connect(&upstream_socket).unwrap();
            upstream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            upstream
                .set_write_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            upstream.write_all(&request).unwrap();
            let mut receipt = Vec::new();
            BufReader::new(upstream)
                .read_until(b'\n', &mut receipt)
                .unwrap();
            if !delayed && envelope["request_id"] == "timeout-write" {
                delayed = true;
                // The actual Application Service has already returned its durable receipt.
                thread::sleep(Duration::from_millis(400));
                let _ = downstream.write_all(&receipt);
                released_tx.send(()).unwrap();
            } else {
                let _ = downstream.write_all(&receipt);
            }
        }
    });
    let _proxy = Proxy {
        stop,
        worker: Some(worker),
    };
    let client = ApplicationServiceClient::with_socket(&database, &proxy_socket)
        .with_request_timeout(Duration::from_millis(150))
        .unwrap();
    let command = || if admission { admission_command() } else { ApplicationCommand::Model {
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
    } };
    let error = client
        .execute_typed_with_request_id::<Value, _>("timeout-write", command())
        .unwrap_err();
    assert!(error.requires_reconciliation(), "{error}");
    assert!(error.to_string().contains("request_id=timeout-write"), "{error}");
    released_rx.recv_timeout(Duration::from_secs(3)).unwrap();
    assert_eq!(
        submissions.load(Ordering::SeqCst),
        1,
        "timeout must not silently resubmit"
    );
    let receipt = client
        .command_receipt("timeout-write")
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
    let load = || if admission { ApplicationCommand::ActiveRun { conversation_id: "admission".into() } } else { ApplicationCommand::Model {
        command: ModelCommand::GetProfile { id: "once".into() },
    } };
    let before: Value = client.execute(load()).unwrap();
    let replay: Value = client
        .execute_with_request_id("timeout-write", command())
        .unwrap();
    assert_eq!(replay, recovered_result);
    assert_eq!(
        submissions.load(Ordering::SeqCst),
        2,
        "only explicit retry sends another command"
    );
    let after: Value = client.execute(load()).unwrap();
    assert_eq!(
        before, after,
        "receipt replay must not create a new model revision"
    );
    assert_eq!(
        client
            .command_receipt("timeout-write")
            .unwrap()
            .unwrap()
            .request_id,
        receipt.request_id
    );
    let mut altered = serde_json::to_value(command()).unwrap();
    if admission {
        altered["input"]["user_text"] = Value::String("changed".into());
    } else {
        altered["command"]["input"]["parameters_json"] = Value::String("{\"temperature\":0.99}".into());
    }
    assert!(
        client
            .execute_with_request_id::<Value, _>("timeout-write", altered)
            .is_err(),
        "request identity must not accept a different payload"
    );
    assert_eq!(client.execute::<Value, _>(load()).unwrap(), before);
    drop(_proxy);
    drop(service);
    let _ = std::fs::remove_file(&service_socket);
    let mut restarted = Service(
        Command::new(env!("CARGO_BIN_EXE_nola-service"))
            .arg("--database")
            .arg(&database)
            .arg("--socket")
            .arg(&service_socket)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    while direct.health().is_err() {
        assert!(restarted.0.try_wait().unwrap().is_none());
        assert!(
            Instant::now() < deadline,
            "restarted service did not become ready"
        );
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(direct.execute::<Value, _>(load()).unwrap(), before);
    let cold_replay: Value = direct.execute_with_request_id("timeout-write", command()).unwrap();
    assert_eq!(cold_replay, recovered_result);
    let recovered = direct.command_receipt("timeout-write").unwrap().unwrap();
    assert_eq!(
        serde_json::to_value(recovered).unwrap(),
        serde_json::to_value(receipt).unwrap(),
        "the original terminal receipt must survive restart unchanged"
    );
}
