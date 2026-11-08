// SPDX-License-Identifier: Apache-2.0
mod support;
use nola_application::{ApplicationCommand, ApplicationCommandReceiptOutcome, ExecuteReceivedCommand, PluginCommand};
use nola_domain::plugin::{PluginInstallation, PluginManifest, PluginSkill};

fn install(expected_revision: Option<i64>) -> PluginCommand {
    PluginCommand::Install { manifest: PluginManifest {
        schema_version: 1, id: "fixture".into(), version: "1".into(), display_name: "Fixture".into(),
        description: "Test".into(), skills: vec![PluginSkill { id: "read".into(), path: "./skills/read".into() }],
        mcp_servers: vec![],
    }, package_digest: "a".repeat(64), expected_revision, now_ms: 100 }
}
fn submit(runtime: &mut support::ApplicationRuntime, request: &str, command: PluginCommand) -> bool {
    let command = ApplicationCommand::Plugin { command };
    let receipt = runtime.execute_received_command(ExecuteReceivedCommand {
        client_id: "plugin-test".into(), request_id: request.into(), command_type: "plugin".into(),
        command_json: serde_json::to_string(&command).unwrap(), expected_revision: None,
        received_at: "2026-10-15T00:00:00Z".into(),
    }, command).unwrap();
    matches!(receipt.outcome, ApplicationCommandReceiptOutcome::Committed { .. })
}
fn list(runtime: &mut support::ApplicationRuntime) -> Vec<PluginInstallation> {
    serde_json::from_value(runtime.execute(ApplicationCommand::Plugin {
        command: PluginCommand::List { after: None, limit: 128 },
    }).unwrap()).unwrap()
}

#[test]
fn install_disable_remove_reinstall_is_revision_checked_and_survives_restart() {
    let dir = tempfile::tempdir().unwrap(); let path = dir.path().join("plugins.db");
    let mut app = support::open_runtime(&path).unwrap();
    assert!(submit(&mut app, "install", install(None)));
    assert!(submit(&mut app, "install", install(None)), "same receipt replay is idempotent");
    assert!(!list(&mut app)[0].enabled);
    assert_eq!(list(&mut app)[0].revision, 1);
    assert!(!submit(&mut app, "conflict", install(None)));
    assert!(submit(&mut app, "enable", PluginCommand::SetEnabled {
        id: "fixture".into(), expected_revision: 1, enabled: true, now_ms: 100 }));
    assert!(!submit(&mut app, "replace-active", install(Some(2))));
    drop(app);
    let mut app = support::open_runtime(&path).unwrap();
    assert!(list(&mut app)[0].enabled);
    assert!(!submit(&mut app, "stale-disable", PluginCommand::SetEnabled {
        id: "fixture".into(), expected_revision: 1, enabled: false, now_ms: 100 }));
    assert!(submit(&mut app, "disable", PluginCommand::SetEnabled {
        id: "fixture".into(), expected_revision: 2, enabled: false, now_ms: 100 }));
    assert!(submit(&mut app, "remove", PluginCommand::Remove {
        id: "fixture".into(), expected_revision: 3, now_ms: 100 }));
    assert!(list(&mut app).is_empty());
    let removed = app.execute(ApplicationCommand::Plugin { command: PluginCommand::Get { id: "fixture".into() }}).unwrap();
    assert_eq!(removed["revision"], 4);
    assert_eq!(removed["removed_at_ms"], 100);
    assert!(!submit(&mut app, "reinstall-unchecked", install(None)));
    assert!(submit(&mut app, "reinstall", install(Some(4))));
    assert_eq!(list(&mut app)[0].revision, 5);
    assert!(!list(&mut app)[0].enabled);
}

#[test]
fn queries_are_read_only_mutations_require_receipts_and_pages_are_bounded() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = support::open_runtime(&dir.path().join("plugins.db")).unwrap();
    assert!(ApplicationCommand::Plugin { command: PluginCommand::Get { id: "fixture".into() }}.is_read_only_query());
    assert!(!ApplicationCommand::Plugin { command: install(None) }.is_read_only_query());
    assert!(app.execute(ApplicationCommand::Plugin { command: install(None) }).is_err());
    for limit in [0, 129, usize::MAX] {
        assert!(app.execute(ApplicationCommand::Plugin { command: PluginCommand::List { after: None, limit }}).is_err());
    }
    assert!(list(&mut app).is_empty());
}
