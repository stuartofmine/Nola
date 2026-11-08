// SPDX-License-Identifier: Apache-2.0
//! Real Application receipts and SQLite edit concurrency; no model or UI mocks.

mod support;
use nola_application::{
    ApplicationCommand, ApplicationCommandReceiptOutcome, ExecuteReceivedCommand, McpCommand,
    McpServer,
};
use support::ApplicationRuntime;

fn submit(runtime: &mut ApplicationRuntime, id: &str, command: McpCommand) -> bool {
    let command = ApplicationCommand::Mcp { command };
    let command_json = serde_json::to_string(&command).unwrap();
    let receipt = runtime
        .execute_received_command(
            ExecuteReceivedCommand {
                client_id: "mcp-editor-test".into(),
                request_id: id.into(),
                command_type: "mcp".into(),
                command_json,
                expected_revision: None,
                received_at: "2026-09-06T00:00:00Z".into(),
            },
            command,
        )
        .unwrap();
    matches!(
        receipt.outcome,
        ApplicationCommandReceiptOutcome::Committed { .. }
    )
}

fn edit(expected_revision: i64, name: &str) -> McpCommand {
    McpCommand::Configure {
        id: "filesystem".into(),
        expected_revision,
        display_name: name.into(),
        enabled: true,
        configuration_json: r#"{"transport":"stdio"}"#.into(),
        updated_at_ms: 20,
    }
}

fn records(runtime: &mut ApplicationRuntime) -> Vec<McpServer> {
    serde_json::from_value(
        runtime
            .execute(ApplicationCommand::Mcp {
                command: McpCommand::List,
            })
            .unwrap(),
    )
    .unwrap()
}

#[test]
fn batch_failure_rolls_back_earlier_changes_and_rejection_survives_restart() {
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("mcp-batch.db");
    let create = || McpCommand::Create {
        id: "filesystem".into(),
        display_name: "Files".into(),
        enabled: true,
        configuration_json: r#"{"transport":"stdio"}"#.into(),
        created_at_ms: 10,
    };
    let invalid_batch = || McpCommand::ApplyChanges {
        commands: vec![create(), edit(999, "Invalid revision")],
    };
    let mut runtime = support::open_runtime(&database).unwrap();
    assert!(!submit(&mut runtime, "failed-batch", invalid_batch()));
    assert!(records(&mut runtime).is_empty());
    drop(runtime);
    let mut runtime = support::open_runtime(&database).unwrap();
    assert!(!submit(&mut runtime, "failed-batch", invalid_batch()));
    assert!(records(&mut runtime).is_empty());
    assert!(submit(
        &mut runtime,
        "valid-batch",
        McpCommand::ApplyChanges {
            commands: vec![create(), edit(1, "Configured"),]
        }
    ));
    let saved = records(&mut runtime);
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].display_name, "Configured");
    assert!(!submit(
        &mut runtime,
        "unchecked-delete",
        McpCommand::ApplyChanges {
            commands: vec![McpCommand::Delete {
                id: "filesystem".into(),
                deleted_at_ms: 30
            },]
        }
    ));
    assert!(!submit(
        &mut runtime,
        "nested-batch",
        McpCommand::ApplyChanges {
            commands: vec![McpCommand::ApplyChanges {
                commands: vec![create()]
            },]
        }
    ));
    assert_eq!(records(&mut runtime), saved);
}

#[test]
fn create_cannot_replace_live_or_deleted_identity_and_replay_is_idempotent() {
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("mcp-create.db");
    let create = || McpCommand::Create {
        id: "filesystem".into(),
        display_name: "Files".into(),
        enabled: true,
        configuration_json: r#"{"transport":"stdio"}"#.into(),
        created_at_ms: 10,
    };
    let mut runtime = support::open_runtime(&database).unwrap();
    assert!(submit(&mut runtime, "create", create()));
    let original = records(&mut runtime);
    assert!(submit(&mut runtime, "create", create()));
    assert!(!submit(&mut runtime, "duplicate", create()));
    assert_eq!(records(&mut runtime), original);
    assert!(submit(
        &mut runtime,
        "delete",
        McpCommand::DeleteChecked {
            id: "filesystem".into(),
            expected_revision: original[0].revision,
            deleted_at_ms: 20,
        }
    ));
    drop(runtime);
    let mut runtime = support::open_runtime(&database).unwrap();
    assert!(!submit(&mut runtime, "resurrect", create()));
    assert!(submit(&mut runtime, "create", create()));
    assert!(records(&mut runtime).is_empty());
}

#[test]
fn stale_edit_and_delete_cannot_overwrite_or_resurrect_server() {
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("mcp.db");
    let mut runtime = support::open_runtime(&database).unwrap();
    assert!(submit(
        &mut runtime,
        "create",
        McpCommand::Upsert {
            input: McpServer {
                id: "filesystem".into(),
                display_name: "Original".into(),
                enabled: true,
                configuration_json: r#"{"transport":"stdio"}"#.into(),
                revision: 0,
                created_at_ms: 10,
                updated_at_ms: 10,
                deleted_at_ms: None,
            }
        }
    ));
    let original = records(&mut runtime).remove(0);
    assert!(runtime
        .execute(ApplicationCommand::Mcp {
            command: edit(original.revision, "Unreceipted"),
        })
        .is_err());
    assert!(submit(
        &mut runtime,
        "edit",
        edit(original.revision, "New name")
    ));
    let saved = records(&mut runtime).remove(0);
    assert_eq!(saved.created_at_ms, original.created_at_ms);
    assert!(saved.revision > original.revision);
    assert!(!submit(
        &mut runtime,
        "stale-edit",
        edit(original.revision, "Lost update")
    ));
    assert!(!submit(
        &mut runtime,
        "stale-delete",
        McpCommand::DeleteChecked {
            id: saved.id.clone(),
            expected_revision: original.revision,
            deleted_at_ms: 30,
        }
    ));
    assert_eq!(records(&mut runtime), vec![saved.clone()]);
    drop(runtime);
    let mut runtime = support::open_runtime(&database).unwrap();
    assert!(!submit(
        &mut runtime,
        "stale-edit",
        edit(original.revision, "Lost update")
    ));
    assert_eq!(records(&mut runtime), vec![saved.clone()]);
    assert!(submit(
        &mut runtime,
        "delete",
        McpCommand::DeleteChecked {
            id: saved.id,
            expected_revision: saved.revision,
            deleted_at_ms: 30,
        }
    ));
    assert!(!submit(
        &mut runtime,
        "resurrect",
        edit(saved.revision, "Resurrected")
    ));
    drop(runtime);
    let mut runtime = support::open_runtime(&database).unwrap();
    assert!(records(&mut runtime).is_empty());
}
