// SPDX-License-Identifier: Apache-2.0
//! Crosses the shipped client/process/socket/Application/Store boundary.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use nola_application::{ApplicationCommand, CreateConversation, PrepareRemoteSubmissionInput};
use nola_service_client::ApplicationServiceClient;
use serde_json::Value;

#[test]
fn shared_client_launches_service_and_round_trips_durable_conversation() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("nola-service-client-e2e-{nonce}"));
    std::fs::create_dir_all(&root).unwrap();
    let database = root.join("nola.db");
    let socket = root.join("nola.sock");
    let executable = PathBuf::from(env!("CARGO_BIN_EXE_nola-service"));
    let launcher = root.join("launch-service.sh");
    let launches = root.join("launches.txt");
    std::fs::write(&launcher,
        "#!/bin/sh\nprintf 'launch\\n' >> \"$NOLA_FIXTURE_LAUNCH_LOG\"\nexec \"$NOLA_FIXTURE_SERVICE\" \"$@\"\n"
    ).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&launcher, std::fs::Permissions::from_mode(0o700)).unwrap();
    std::env::set_var("NOLA_FIXTURE_SERVICE", executable);
    std::env::set_var("NOLA_FIXTURE_LAUNCH_LOG", &launches);
    std::env::set_var("NOLA_APPLICATION_SERVICE_EXECUTABLE", launcher);

    let client = ApplicationServiceClient::with_socket(&database, &socket);
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
    let workers: Vec<_> = (0..8)
        .map(|_| {
            let client = client.clone();
            let barrier = std::sync::Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                client.ensure_running()
            })
        })
        .collect();
    for worker in workers {
        worker.join().unwrap().unwrap();
    }
    assert_eq!(
        std::fs::read_to_string(&launches).unwrap().lines().count(),
        1,
        "concurrent clients must share one service launch"
    );
    client.ensure_running().unwrap();
    let health = client.health().unwrap();
    assert_eq!(health["service"], "nola-application");
    assert!(
        health.get("store_schema_version").is_none(),
        "transport health must not expose Application persistence details"
    );

    let _: Value = client
        .execute(ApplicationCommand::CreateConversation {
            input: CreateConversation {
                id: "client-e2e-conversation".to_string(),
                title: "Client process E2E".to_string(),
                settings_json: None,
                created_at_ms: 1,
                updated_at_ms: 1,
            },
        })
        .unwrap();

    let loaded: Value = client
        .execute(ApplicationCommand::LoadConversation {
            conversation_id: "client-e2e-conversation".to_string(),
        })
        .unwrap();
    assert_eq!(loaded["title"], "Client process E2E");
    let projection: Value = client
        .execute(ApplicationCommand::PrepareRemoteSubmission {
            input: PrepareRemoteSubmissionInput {
                client_id: "fixture-client".into(),
                source_host_id: "host".into(),
                conversation_id: "client-e2e-conversation".to_string(),
                conversation_title: "Client process E2E".to_string(),
                submission_id: "client-run-1".to_string(),
                user_message_id: "client-run-1:user".to_string(),
                user_part_id: "client-run-1:user:text".to_string(),
                user_text: "Committed through service".to_string(),
                user_content_json: None,
                assistant_message_id: "client-run-1:assistant".to_string(),
                assistant_part_id: "client-run-1:assistant:text".to_string(),
                context_checkpoint_id: None,
                created_at_ms: 2,
            },
        })
        .unwrap();
    assert_eq!(projection["identity"]["submission_id"], "client-run-1");
    assert_eq!(projection["phase"], "prepared");
    let projected: Value = client
        .execute(ApplicationCommand::LoadConversation {
            conversation_id: "client-e2e-conversation".to_string(),
        })
        .unwrap();
    assert_eq!(
        projected["messages"],
        serde_json::json!([]),
        "pending intent is not canonical execution"
    );
    let pending: Value = client
        .execute(ApplicationCommand::ReadRemoteSubmissionIntent {
            conversation_id: "client-e2e-conversation".into(),
        })
        .unwrap();
    assert_eq!(pending, projection);
    assert!(database.is_file());
    // A busy owned service must not be mistaken for an exited process.
    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;
    let mut held = Vec::new();
    for _ in 0..128 {
        let mut stream = UnixStream::connect(&socket).unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_millis(1)))
            .unwrap();
        stream
            .set_write_timeout(Some(std::time::Duration::from_millis(100)))
            .unwrap();
        let _ = stream.write_all(b"{");
        match stream.read(&mut [0; 1]) {
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                held.push(stream)
            }
            Ok(0) => (),
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::BrokenPipe
                ) =>
            {
                ()
            }
            result => panic!("unexpected admission result: {result:?}"),
        }
    }
    let error = client
        .ensure_running()
        .expect_err("all connection slots are occupied");
    assert!(error.contains("alive but not responding"), "{error}");
    assert_eq!(
        std::fs::read_to_string(&launches).unwrap().lines().count(),
        1,
        "a health failure must not spawn a replacement for a live owned service"
    );
    drop(held);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while client.health().is_err() {
        assert!(
            std::time::Instant::now() < deadline,
            "service did not recover after load"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    client.ensure_running().unwrap();
}
