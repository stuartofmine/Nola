// SPDX-License-Identifier: Apache-2.0
//! Envelope conditions must not silently become unconditional writes.
#![cfg(unix)]

use nola_application::{ApplicationCommand, ModelCommand};
use nola_protocol::{CommandReceiptOutcome, APPLICATION_SERVICE_PROTOCOL_VERSION};
use nola_service_client::ApplicationServiceClient;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct Service(Child);
impl Drop for Service {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn revision_preconditions_conflict_or_reject_without_mutation() {
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("state.db");
    let socket = root.path().join("service.sock");
    let mut service = Service(
        Command::new(env!("CARGO_BIN_EXE_nola-service"))
            .arg("--database")
            .arg(&database)
            .arg("--socket")
            .arg(&socket)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let client = ApplicationServiceClient::with_socket(&database, &socket);
    let deadline = Instant::now() + Duration::from_secs(5);
    while client.health().is_err() {
        assert!(service.0.try_wait().unwrap().is_none());
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    let mut stream = UnixStream::connect(&socket).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(3)))
        .unwrap();
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
    // Profile upsert now supports CAS. A missing profile has revision zero,
    // so this must be a conflict, not an unsupported condition or a write.
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
    assert!(
        stored.is_null(),
        "conditional write must not execute: {stored}"
    );

    let input = serde_json::from_value(
        envelope["command"]["arguments"]["command"]["input"].clone(),
    ).unwrap();
    let _: Value = client.execute(ApplicationCommand::Model {
        command: ModelCommand::UpsertProfile { input },
    }).unwrap();
    let load = || client.execute::<Value, _>(ApplicationCommand::Model {
        command: ModelCommand::GetProfile { id: "must-not-exist".into() },
    }).unwrap();
    let before = load();
    assert!(!before.is_null());
    // Delete does not expose an envelope CAS scope. Reject it before any
    // mutation, even though the profile now exists and would be deletable.
    let rejection = client.submit_with_revision(
        "unsupported-delete", Some(42), ApplicationCommand::Model {
            command: ModelCommand::DeleteProfile {
                id: "must-not-exist".into(), deleted_at_ms: 2,
            },
        },
    ).unwrap();
    match rejection.outcome {
        CommandReceiptOutcome::Rejected { error, .. } => {
            assert_eq!(error.code, "unsupported_revision_precondition");
        }
        other => panic!("unsupported condition must reject before mutation: {other:?}"),
    }
    assert_eq!(load(), before);
    assert!(client.command_receipt("unsupported-delete").unwrap().is_none());
}

#[test]
fn model_parameter_cas_conflict_and_success_replay_durably() {
    use nola_application::ModelProfile;
    use nola_protocol::CommandReceiptOutcome;
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("state.db");
    let socket = root.path().join("service.sock");
    let start = || {
        Service(
            Command::new(env!("CARGO_BIN_EXE_nola-service"))
                .arg("--database")
                .arg(&database)
                .arg("--socket")
                .arg(&socket)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        )
    };
    let mut service = start();
    let client = ApplicationServiceClient::with_socket(&database, &socket);
    let ready = |service: &mut Service| {
        let deadline = Instant::now() + Duration::from_secs(5);
        while client.health().is_err() {
            assert!(service.0.try_wait().unwrap().is_none());
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
    };
    ready(&mut service);
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
    let load = || {
        client
            .execute::<Value, _>(ApplicationCommand::Model {
                command: ModelCommand::GetProfile { id: "cas".into() },
            })
            .unwrap()
    };
    let created = client
        .submit_with_revision("create", Some(0), edit(0.1))
        .unwrap();
    let first = load();
    let revision = first["revision"].as_u64().unwrap();
    assert!(revision > 0);
    assert!(matches!(created.outcome, CommandReceiptOutcome::Committed {
        committed_revision: Some(value), ..
    } if value == revision));
    let winner = client
        .submit_with_revision("winner", Some(revision), edit(0.2))
        .unwrap();
    assert!(matches!(
        winner.outcome,
        CommandReceiptOutcome::Committed { .. }
    ));
    let current = load();
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
    assert_eq!(load(), current);
    assert_eq!(
        client
            .submit_with_revision("create", Some(0), edit(0.1))
            .unwrap(),
        created
    );
    assert_eq!(load(), current);
    drop(service);
    service = start();
    ready(&mut service);
    assert_eq!(
        client
            .submit_with_revision("stale", Some(revision), edit(0.9))
            .unwrap(),
        conflict
    );
    assert_eq!(load(), current);
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
    assert!(load().is_null());
}
