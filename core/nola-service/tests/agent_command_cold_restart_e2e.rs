// SPDX-License-Identifier: Apache-2.0

use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use nola_application::{AgentCommand, AgentCommandLease, ApplicationCommand};
use nola_service_client::ApplicationServiceClient;

fn start_service(database: &std::path::Path, socket: &std::path::Path) -> Child {
    Command::new(env!("CARGO_BIN_EXE_nola-service"))
        .args(["--database"])
        .arg(database)
        .args(["--socket"])
        .arg(socket)
        .args(["--parent-pid", &std::process::id().to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start real Application Service")
}

fn ready(client: &ApplicationServiceClient, child: &mut Child) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if client.health().is_ok() {
            return;
        }
        assert!(child.try_wait().expect("inspect service").is_none());
        thread::sleep(Duration::from_millis(20));
    }
    panic!("Application Service did not become ready");
}

#[test]
fn leased_agent_command_replays_after_crash_and_disappears_only_after_ack() {
    let directory = tempfile::tempdir().expect("temporary state");
    let database = directory.path().join("nola.db");
    let socket = directory.path().join("nola.sock");
    let client = ApplicationServiceClient::with_socket(&database, &socket);
    let mut first = start_service(&database, &socket);
    ready(&client, &mut first);

    let input = AgentCommand {
        session_id: "conversation-command".into(),
        command_id: "command-1".into(),
        payload_json: serde_json::to_string(&nola_core::agent::AgentSubmission::Interrupt {
            goal_control: None, cancellation_id: "cancel-command-1".into(),
            reason: Some("stop requested".into()),
        }).unwrap(),
        created_at_ms: 10,
    };
    let inserted: bool = client
        .execute(ApplicationCommand::EnqueueAgentCommand {
            input: input.clone(),
        })
        .expect("enqueue command");
    assert!(inserted);
    let duplicate: bool = client
        .execute(ApplicationCommand::EnqueueAgentCommand { input })
        .expect("deduplicate command");
    assert!(!duplicate);
    let claimed: Option<AgentCommandLease> = client
        .execute(ApplicationCommand::ClaimNextAgentCommand {
            session_id: "conversation-command".into(),
            lease_owner: "consumer-a".into(),
            now_ms: 20,
            lease_until_ms: 120,
        })
        .expect("claim command");
    let claimed = claimed.unwrap();
    assert_eq!(claimed.delivery_attempt, 1);
    // Admission wraps submissions in the canonical boundary envelope. Verify
    // its typed intent, then compare those committed bytes across restart.
    let original_payload = claimed.payload_json;
    let original_boundary: nola_core::agent::host::AgentBoundaryInput =
        serde_json::from_str(&original_payload).unwrap();
    assert!(matches!(original_boundary,
        nola_core::agent::host::AgentBoundaryInput::Submission {
            submission: nola_core::agent::AgentSubmission::Interrupt { goal_control: None, cancellation_id, reason }
        } if cancellation_id == "cancel-command-1" && reason.as_deref() == Some("stop requested")));
    let unavailable: Option<AgentCommandLease> = client
        .execute(ApplicationCommand::ClaimNextAgentCommand {
            session_id: "conversation-command".into(),
            lease_owner: "consumer-b".into(),
            now_ms: 30,
            lease_until_ms: 130,
        })
        .expect("honor active lease");
    assert!(unavailable.is_none());

    first.kill().expect("kill first service");
    first.wait().expect("reap first service");
    let _ = std::fs::remove_file(&socket);
    let mut second = start_service(&database, &socket);
    ready(&client, &mut second);
    let replayed: Option<AgentCommandLease> = client
        .execute(ApplicationCommand::ClaimNextAgentCommand {
            session_id: "conversation-command".into(),
            lease_owner: "consumer-b".into(),
            now_ms: 121,
            lease_until_ms: 221,
        })
        .expect("replay expired lease");
    let replayed = replayed.unwrap();
    assert_eq!(replayed.delivery_attempt, 2);
    assert_eq!(replayed.payload_json, original_payload,
        "restart must preserve the original cancellation identity and payload");
    let acknowledged: bool = client
        .execute(ApplicationCommand::AcknowledgeAgentCommand {
            session_id: "conversation-command".into(),
            command_id: "command-1".into(),
            lease_owner: "consumer-b".into(),
        })
        .expect("acknowledge command");
    assert!(acknowledged);

    second.kill().expect("kill second service");
    second.wait().expect("reap second service");
    let _ = std::fs::remove_file(&socket);
    let mut third = start_service(&database, &socket);
    ready(&client, &mut third);
    let gone: Option<AgentCommandLease> = client
        .execute(ApplicationCommand::ClaimNextAgentCommand {
            session_id: "conversation-command".into(),
            lease_owner: "consumer-c".into(),
            now_ms: 300,
            lease_until_ms: 400,
        })
        .expect("command remains acknowledged after restart");
    assert!(gone.is_none());

    third.kill().expect("kill third service");
    third.wait().expect("reap third service");
}
