// SPDX-License-Identifier: Apache-2.0

use nola_application::{ApplicationCommand, SessionRunLease, WorkspaceCheckpoint};
use nola_service_client::ApplicationServiceClient;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

fn start(database: &std::path::Path, socket: &std::path::Path) -> Child {
    Command::new(env!("CARGO_BIN_EXE_nola-service"))
        .args(["--database"])
        .arg(database)
        .args(["--socket"])
        .arg(socket)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap()
}
fn ready(client: &ApplicationServiceClient, child: &mut Child) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if client.health().is_ok() {
            return;
        }
        assert!(child.try_wait().unwrap().is_none());
        thread::sleep(Duration::from_millis(20));
    }
    panic!("Application Service did not become ready");
}

#[test]
fn checkpoint_and_owner_safe_run_lease_survive_real_service_restart() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("nola.db");
    let socket = directory.path().join("nola.sock");
    let client = ApplicationServiceClient::with_socket(&database, &socket);
    let mut first = start(&database, &socket);
    ready(&client, &mut first);
    let saved: bool = client
        .execute(ApplicationCommand::SaveWorkspaceCheckpoint {
            input: WorkspaceCheckpoint {
                id: "cp-1".into(),
                root: "/workspace".into(),
                git_root: "/workspace".into(),
                revision: "abc123".into(),
                source: "snapshot".into(),
                label: "before edit".into(),
                created_at_ms: 10,
                status: "dirty".into(),
                untracked: vec!["notes.txt".into()],
            },
        })
        .unwrap();
    assert!(saved);
    let acquired: bool = client
        .execute(ApplicationCommand::AcquireSessionRunLease {
            input: SessionRunLease {
                session_id: "conversation-1".into(),
                owner: "owner-a".into(),
                pid: 42,
                process_start: "100".into(),
                acquired_at_ms: 20,
            },
        })
        .unwrap();
    assert!(acquired);
    first.kill().unwrap();
    first.wait().unwrap();
    let _ = std::fs::remove_file(&socket);

    let mut second = start(&database, &socket);
    ready(&client, &mut second);
    let checkpoints: Vec<WorkspaceCheckpoint> = client
        .execute(ApplicationCommand::ListWorkspaceCheckpoints {
            root: "/workspace".into(),
        })
        .unwrap();
    assert_eq!(checkpoints[0].revision, "abc123");
    let lease: Option<SessionRunLease> = client
        .execute(ApplicationCommand::GetSessionRunLease {
            session_id: "conversation-1".into(),
        })
        .unwrap();
    assert_eq!(lease.unwrap().owner, "owner-a");
    let wrong: bool = client
        .execute(ApplicationCommand::ReleaseSessionRunLease {
            session_id: "conversation-1".into(),
            owner: "owner-b".into(),
        })
        .unwrap();
    assert!(!wrong);
    let released: bool = client
        .execute(ApplicationCommand::ReleaseSessionRunLease {
            session_id: "conversation-1".into(),
            owner: "owner-a".into(),
        })
        .unwrap();
    assert!(released);
    let deleted: bool = client
        .execute(ApplicationCommand::DeleteWorkspaceCheckpoint { id: "cp-1".into() })
        .unwrap();
    assert!(deleted);
    second.kill().unwrap();
    second.wait().unwrap();
    let _ = std::fs::remove_file(&socket);

    let mut third = start(&database, &socket);
    ready(&client, &mut third);
    let checkpoints: Vec<WorkspaceCheckpoint> = client
        .execute(ApplicationCommand::ListWorkspaceCheckpoints {
            root: "/workspace".into(),
        })
        .unwrap();
    let lease: Option<SessionRunLease> = client
        .execute(ApplicationCommand::GetSessionRunLease {
            session_id: "conversation-1".into(),
        })
        .unwrap();
    assert!(checkpoints.is_empty());
    assert!(lease.is_none());
    third.kill().unwrap();
    third.wait().unwrap();
}
