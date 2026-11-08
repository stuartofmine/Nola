// SPDX-License-Identifier: Apache-2.0
mod support;
use nola_application::{
    ApplicationCommand, ApplicationCommandReceiptOutcome, AutomationCommand, AutomationDefinition,
    AutomationDefinitionCommand, ExecuteReceivedCommand,
};
use nola_store::NolaStore;
use nola_store_port::automation::AutomationRepository;

#[test]
fn conditional_definition_command_preserves_reviewed_revision_and_replay() {
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("conditional.db");
    let mut store = NolaStore::open(&database).unwrap();
    let stored = store.upsert_automation_definition(&nola_store::StoredAutomationDefinition {
        execution_spec: serde_json::from_value(serde_json::json!({"kind":"agent","spec":{"prompt":"Execute this task once","execution_scope":{"kind":"local_workspace","root":"/fixture"},"permission_profile":"cautious","model_selection":"use_current_default_at_admission","context_refs":[],"skill_refs":[]}})).unwrap(),
        id:"review".into(),name:"Script".into(),schedule_format:"script".into(),enabled:false,revision:0,
        metadata_json:"{}".into(),created_at_ms:1,updated_at_ms:1,deleted_at_ms:None,
    }).unwrap();
    let mut runtime = support::open_runtime(&database).unwrap();
    let command = serde_json::to_string(&ApplicationCommand::Automation { command: AutomationCommand::Definition {
        command: AutomationDefinitionCommand::UpdateIfRevision { expected_revision: stored.revision, input: AutomationDefinition {
            execution_spec: serde_json::from_value(serde_json::json!({"kind":"agent","spec":{"prompt":"Execute this task once","execution_scope":{"kind":"local_workspace","root":"/fixture"},"permission_profile":"cautious","model_selection":"use_current_default_at_admission","context_refs":[],"skill_refs":[]}})).unwrap(),
            id:stored.id.clone(),name:stored.name.clone(),schedule_format:stored.schedule_format.clone(),enabled:true,
            revision:stored.revision,metadata_json:"{\"approved\":true}".into(),created_at_ms:1,updated_at_ms:2,deleted_at_ms:None,
        } },
    } }).unwrap();
    let envelope = |id: &str| ExecuteReceivedCommand { client_id:"conditional-test".into(),request_id:id.into(),command_type:"automation".into(),command_json:command.clone(),expected_revision:None,received_at:"2026-09-22T00:00:00Z".into() };
    let first = runtime.execute_received_command(envelope("approve"), serde_json::from_str(&command).unwrap()).unwrap();
    assert!(store.automation_definition("review").unwrap().unwrap().enabled);
    let revision = store.automation_definition("review").unwrap().unwrap().revision;
    let outbox = store.pending_sync_changes(100).unwrap().len();
    let replay = runtime.execute_received_command(envelope("approve"), serde_json::from_str(&command).unwrap()).unwrap();
    assert!(replay.replayed); assert_eq!(replay.outcome, first.outcome);
    runtime.execute_received_command(envelope("stale"), serde_json::from_str(&command).unwrap()).unwrap();
    assert_eq!(store.automation_definition("review").unwrap().unwrap().revision, revision);
    assert_eq!(store.pending_sync_changes(100).unwrap().len(), outbox);
}

#[test]
fn automation_port_preserves_receipt_atomicity_and_cold_replay() {
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("automation-port.db");
    let mut runtime = support::open_runtime(&database).unwrap();
    let inspect = rusqlite::Connection::open(&database).unwrap();
    let command = ApplicationCommand::Automation {
        command: AutomationCommand::Definition {
            command: AutomationDefinitionCommand::Upsert {
                input: AutomationDefinition {
                    execution_spec: serde_json::from_value(serde_json::json!({"kind":"agent","spec":{"prompt":"Execute this task once","execution_scope":{"kind":"local_workspace","root":"/fixture"},"permission_profile":"cautious","model_selection":"use_current_default_at_admission","context_refs":[],"skill_refs":[]}})).unwrap(),
                    id: "daily".into(),
                    name: "Daily review".into(),
                    schedule_format: "0 8 * * *".into(),
                    enabled: true,
                    revision: 1,
                    metadata_json: "{}".into(),
                    created_at_ms: 1,
                    updated_at_ms: 1,
                    deleted_at_ms: None,
                },
            },
        },
    };
    let json = serde_json::to_string(&command).unwrap();
    let envelope = || ExecuteReceivedCommand {
        client_id: "automation-port-test".into(),
        request_id: "upsert".into(),
        command_type: "automation".into(),
        command_json: json.clone(),
        expected_revision: None,
        received_at: "2026-09-08T00:00:00Z".into(),
    };
    inspect.execute_batch("CREATE TRIGGER fail_automation_receipt BEFORE UPDATE ON command_receipts WHEN NEW.status='committed' BEGIN SELECT RAISE(ABORT,'automation receipt fault'); END;").unwrap();
    let baseline_outbox: i64 = inspect
        .query_row("SELECT COUNT(*) FROM sync_outbox", [], |r| r.get(0))
        .unwrap();
    let error = runtime
        .execute_received_command(envelope(), serde_json::from_str(&json).unwrap())
        .unwrap_err();
    assert!(
        error.to_string().contains("automation receipt fault"),
        "{error}"
    );
    let mut store = NolaStore::open(&database).unwrap();
    assert!(
        AutomationRepository::automation_definition(&store.automation_repository(), "daily")
            .unwrap()
            .is_none()
    );
    drop(store);
    assert_eq!(
        inspect
            .query_row("SELECT COUNT(*) FROM command_receipts", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        inspect
            .query_row("SELECT COUNT(*) FROM sync_outbox", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        baseline_outbox
    );
    inspect
        .execute_batch("DROP TRIGGER fail_automation_receipt")
        .unwrap();
    let committed = runtime
        .execute_received_command(envelope(), serde_json::from_str(&json).unwrap())
        .unwrap();
    assert!(matches!(
        committed.outcome,
        ApplicationCommandReceiptOutcome::Committed { .. }
    ));
    let outbox: i64 = inspect
        .query_row("SELECT COUNT(*) FROM sync_outbox", [], |r| r.get(0))
        .unwrap();
    drop(runtime);
    let mut runtime = support::open_runtime(&database).unwrap();
    let replay = runtime
        .execute_received_command(envelope(), serde_json::from_str(&json).unwrap())
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.outcome, committed.outcome);
    assert_eq!(
        inspect
            .query_row("SELECT COUNT(*) FROM sync_outbox", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        outbox
    );
    let mut store = NolaStore::open(&database).unwrap();
    assert_eq!(
        AutomationRepository::automation_definition(&store.automation_repository(), "daily")
            .unwrap()
            .unwrap()
            .name,
        "Daily review"
    );
}
