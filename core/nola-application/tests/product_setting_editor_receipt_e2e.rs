// SPDX-License-Identifier: Apache-2.0
mod support;
use nola_application::{
    ApplicationCommand, ApplicationCommandReceiptOutcome, ExecuteReceivedCommand,
};
use serde_json::Value;
use support::ApplicationRuntime;

fn command(expected_revision: Option<i64>, value: &str) -> ApplicationCommand {
    ApplicationCommand::PutProductSettingChecked {
        key: "editor.fixture".into(),
        value_json: value.into(),
        expected_revision,
        updated_at_ms: 100,
    }
}

fn submit(
    runtime: &mut ApplicationRuntime,
    id: &str,
    command: ApplicationCommand,
) -> Option<Value> {
    let command_json = serde_json::to_string(&command).unwrap();
    let receipt = runtime
        .execute_received_command(
            ExecuteReceivedCommand {
                client_id: "setting-editor".into(),
                request_id: id.into(),
                command_type: "put_product_setting_checked".into(),
                command_json,
                expected_revision: None,
                received_at: "2026-09-07T00:00:00Z".into(),
            },
            command,
        )
        .unwrap();
    match receipt.outcome {
        ApplicationCommandReceiptOutcome::Committed { result, .. } => Some(result),
        _ => None,
    }
}

fn read(runtime: &mut ApplicationRuntime) -> Value {
    runtime
        .execute(ApplicationCommand::GetProductSetting {
            key: "editor.fixture".into(),
        })
        .unwrap()
}

#[test]
fn concurrent_editors_cannot_overwrite_and_committed_receipt_survives_reopen() {
    let root = tempfile::tempdir().unwrap();
    let db = root.path().join("settings.db");
    let mut runtime = support::open_runtime(&db).unwrap();
    let first = submit(&mut runtime, "create", command(None, "[]")).unwrap();
    assert_eq!(first["revision"], 1);
    assert!(submit(&mut runtime, "duplicate-create", command(None, "[1]")).is_none());
    let second = submit(&mut runtime, "edit", command(Some(1), "[2]")).unwrap();
    assert_eq!(second["revision"], 2);
    assert!(submit(&mut runtime, "stale", command(Some(1), "[3]")).is_none());
    assert_eq!(read(&mut runtime), second);
    drop(runtime);
    let mut runtime = support::open_runtime(&db).unwrap();
    assert_eq!(
        submit(&mut runtime, "edit", command(Some(1), "[2]")).unwrap(),
        second
    );
    assert_eq!(read(&mut runtime), second);
}

#[test]
fn invalid_json_or_unadmitted_write_cannot_change_the_setting() {
    let root = tempfile::tempdir().unwrap();
    let mut runtime = support::open_runtime(root.path().join("settings.db")).unwrap();
    assert!(runtime.execute(command(None, "[]")).is_err());
    assert_eq!(read(&mut runtime), Value::Null);
    assert!(submit(&mut runtime, "bad-revision", command(Some(0), "[]")).is_none());
    let first = submit(&mut runtime, "create", command(None, "[]")).unwrap();
    assert!(submit(&mut runtime, "bad-json", command(Some(1), "not JSON")).is_none());
    assert_eq!(read(&mut runtime), first);
}

#[test]
fn rejected_receipt_does_not_turn_into_a_write_when_the_precondition_later_matches() {
    let root = tempfile::tempdir().unwrap();
    let db = root.path().join("settings.db");
    let mut runtime = support::open_runtime(&db).unwrap();
    assert!(submit(&mut runtime, "too-early", command(Some(1), "[99]")).is_none());
    let first = submit(&mut runtime, "create", command(None, "[]")).unwrap();
    drop(runtime);
    let mut runtime = support::open_runtime(&db).unwrap();
    assert!(submit(&mut runtime, "too-early", command(Some(1), "[99]")).is_none());
    assert_eq!(read(&mut runtime), first);
}

#[test]
fn default_chat_preference_validates_schema_and_preserves_receipt_cas_across_restart() {
    let root = tempfile::tempdir().unwrap();
    let db = root.path().join("chat-settings.db");
    let mut runtime = support::open_runtime(&db).unwrap();
    let command = |revision, value: &str| ApplicationCommand::PutProductSettingChecked {
        key: "chat.default_model".into(), value_json: value.into(),
        expected_revision: revision, updated_at_ms: 100,
    };
    let read = |runtime: &mut ApplicationRuntime| runtime.execute(ApplicationCommand::GetProductSetting { key: "chat.default_model".into() }).unwrap();
    assert!(submit(&mut runtime, "malformed", command(None, "{}")).is_none());
    assert_eq!(read(&mut runtime), Value::Null);
    let profile = r#"{"version":1,"selection":{"kind":"profile","profile_id":"api:connection:model"}}"#;
    let saved = submit(&mut runtime, "create-default", command(None, profile)).unwrap();
    let automatic = r#"{"version":1,"selection":{"kind":"automatic"}}"#;
    assert!(submit(&mut runtime, "stale-default", command(None, automatic)).is_none());
    assert_eq!(read(&mut runtime), saved);
    drop(runtime);
    let mut runtime = support::open_runtime(&db).unwrap();
    assert_eq!(submit(&mut runtime, "create-default", command(None, profile)).unwrap(), saved);
    assert_eq!(read(&mut runtime), saved);
    let reset = submit(&mut runtime, "reset-default", command(Some(1), automatic)).unwrap();
    assert_eq!(reset["revision"], 2);
}
