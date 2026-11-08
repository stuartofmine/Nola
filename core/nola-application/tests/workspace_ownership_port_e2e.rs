// SPDX-License-Identifier: Apache-2.0

use nola_application::{
    execute_application_command, ApplicationCommand, SessionRunLease, WorkspaceCheckpoint,
};
use nola_store::NolaStore;
use tempfile::tempdir;

#[test]
fn workspace_checkpoint_and_run_lease_survive_application_cold_reopen() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("workspace-ownership.db");
    {
        let mut store = NolaStore::open(&database).unwrap();
        let saved = execute_application_command(
            &mut store,
            ApplicationCommand::SaveWorkspaceCheckpoint {
                input: WorkspaceCheckpoint {
                    id: "checkpoint-1".into(),
                    root: "/workspace".into(),
                    git_root: "/workspace".into(),
                    revision: "abc123".into(),
                    source: "tool".into(),
                    label: "Before editing".into(),
                    created_at_ms: 10,
                    status: "ready".into(),
                    untracked: vec!["notes.md".into()],
                },
            },
        )
        .unwrap();
        assert_eq!(saved, serde_json::json!(true));

        let acquired = execute_application_command(
            &mut store,
            ApplicationCommand::AcquireSessionRunLease {
                input: SessionRunLease {
                    session_id: "conversation-1".into(),
                    owner: "mac-window-1".into(),
                    pid: 42,
                    process_start: "start-token".into(),
                    acquired_at_ms: 11,
                },
            },
        )
        .unwrap();
        assert_eq!(acquired, serde_json::json!(true));
    }

    let mut reopened = NolaStore::open(&database).unwrap();
    let checkpoints = execute_application_command(
        &mut reopened,
        ApplicationCommand::ListWorkspaceCheckpoints {
            root: "/workspace".into(),
        },
    )
    .unwrap();
    let checkpoints: Vec<WorkspaceCheckpoint> = serde_json::from_value(checkpoints).unwrap();
    assert_eq!(checkpoints.len(), 1);
    assert_eq!(checkpoints[0].untracked, vec!["notes.md"]);

    let lease = execute_application_command(
        &mut reopened,
        ApplicationCommand::GetSessionRunLease {
            session_id: "conversation-1".into(),
        },
    )
    .unwrap();
    let lease: Option<SessionRunLease> = serde_json::from_value(lease).unwrap();
    assert_eq!(lease.unwrap().owner, "mac-window-1");
}
