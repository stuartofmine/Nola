// SPDX-License-Identifier: Apache-2.0
// Canonical permission read fixtures; these do not execute permission decisions.
#[path = "../../nola-store/tests/support/remote_submission.rs"]
mod remote_submission_fixture;
#[path = "../../nola-store/tests/support/terminal_sources.rs"]
mod terminal_sources;
use remote_submission_fixture::RemoteSubmissionFixture;

use nola_application::{
    execute_application_command, ApplicationCommand, PermissionProjectionReadScope,
};
use nola_store::{BeginTurnInput, CreateConversationInput, NolaStore};
use serde_json::{json, Value};
#[path = "support/admission_frontier.rs"]
mod admission_frontier;

fn store() -> NolaStore {
    let mut store = NolaStore::open(":memory:").unwrap();
    store
        .create_conversation(&CreateConversationInput {
            id: "permissions".into(),
            title: "Permissions".into(),
            revision: 1,
            settings_json: None,
            created_at_ms: 1,
            updated_at_ms: 1,
        })
        .unwrap();
    store
}

#[test]
fn run_identity_query_reads_canonical_owner_without_a_delivery_lease_or_pending_effect() {
    let mut store = store();
    save(&mut store, checkpoint());
    let command: ApplicationCommand = serde_json::from_value(json!({
        "type":"load_run_identity", "run_id":"permission-run"
    })).unwrap();
    assert!(command.is_read_only_query());
    assert_eq!(execute_application_command(&mut store, command).unwrap(),
        json!({"run_id":"permission-run", "conversation_id":"permissions"}));
    assert!(execute_application_command(&mut store,
        ApplicationCommand::LoadRunIdentity { run_id:"missing".into() }).unwrap_err().contains("run_not_found"));
    assert!(execute_application_command(&mut store,
        ApplicationCommand::LoadRunIdentity { run_id:" permission-run ".into() }).is_err());
}

fn save(store: &mut NolaStore, payload: Value) {
    save_for_run(store, payload, "permission-run");
}

fn save_for_run(store: &mut NolaStore, payload: Value, run_id: &str) {
    let frontier = admission_frontier::read(store, "permissions");
    let revision = store
        .load_checkpoint("permissions")
        .unwrap()
        .map_or(1, |checkpoint| checkpoint.revision + 1);
    store
        .begin_turn(&BeginTurnInput {
            user_part_kind: "text".into(),
            expected_configuration_revision: frontier.0,
            expected_parent_message_id: frontier.1,
            conversation_id: "permissions".into(),
            conversation_title: "Permissions".into(),
            user_message_id: format!("{run_id}:user"),
            user_part_id: format!("{run_id}:user:text"),
            user_text: "fixture".into(),
            user_content_json: None,
            additional_user_parts: vec![],
            assistant_message_id: format!("{run_id}:assistant"),
            assistant_part_id: format!("{run_id}:assistant:text"),
            run_id: run_id.into(),
            context_checkpoint_id: None,
            created_at_ms: 1,
            checkpoint_revision: revision,
            checkpoint_event_cursor: 7,
            checkpoint_json: payload.to_string(),
            invocation_config_json: "{}".into(),
            invocation_config_digest: "permission-fixture".into(),
            outbox_effects: vec![],
            initial_events: vec![nola_store_port::run_creation::InitialCoreRunEvent {
                core_session_id: "permissions".into(),
                core_turn_id: Some(run_id.into()),
                core_cursor: 7,
                core_revision: 1,
                event_kind: "turn_started".into(),
                payload_json:
                    json!({"type":"turn_started","session_id":"permissions","turn_id":run_id})
                        .to_string(),
            }],
            initial_assistant_part_mutations: vec![],
            assistant_turn_bindings: vec![],
            memory_usage_entries: vec![],
            delegated_invocation_mutations: vec![],
            delegated_permission_mutations: vec![],
            model_attempt_mutations: vec![],
        })
        .unwrap();
}

fn checkpoint() -> Value {
    json!({
        "revision": 1, "session_id": "permissions", "last_event_cursor": {"sequence": 7},
        "state": {"session_id": "permissions", "execution": {
            "permissions": {"revision": 5, "pending": []}
        }}
    })
}

fn query(store: &mut NolaStore) -> Result<Value, String> {
    execute_application_command(
        store,
        ApplicationCommand::LoadConversationPermissionProjectionV1 {
            conversation_id: "permissions".into(),
            run_id: "permission-run".into(),
            scope: PermissionProjectionReadScope::CurrentRun,
        },
    )
}

#[test]
fn configuration_snapshot_and_save_share_the_exact_active_owner() {
    let mut store = store();
    save(&mut store, checkpoint());
    let read: ApplicationCommand = serde_json::from_value(json!({
        "type":"read_permission_configuration", "conversation_id":"permissions",
    })).unwrap();
    assert!(read.is_read_only_query());
    let snapshot = execute_application_command(&mut store, read).unwrap();
    assert_eq!(snapshot["owner"], json!({"kind":"local","run_id":"permission-run",
        "assistant_message_id":"permission-run:assistant"}));
    assert_eq!(snapshot["state"], "pending");
    let revision = snapshot["configuration_revision"].as_i64().unwrap();
    let patch = |run: Option<&str>| serde_json::from_value::<ApplicationCommand>(json!({
        "type":"set_conversation_permission", "conversation_id":"permissions",
        "expected_revision":revision, "expected_run_id":run, "expected_source_host_id":null,
        "profile":"allow_all", "updated_at_ms":2,
    })).unwrap();
    assert!(execute_application_command(&mut store, patch(None)).unwrap_err().contains("owner changed"));
    assert!(execute_application_command(&mut store, patch(Some("foreign"))).unwrap_err().contains("owner changed"));
    assert_eq!(store.load_conversation("permissions").unwrap().unwrap().configuration_revision, revision);
    let committed = execute_application_command(&mut store, patch(Some("permission-run"))).unwrap();
    assert_eq!(committed["status"], "committed");
    let read = serde_json::from_value::<ApplicationCommand>(json!({
        "type":"read_permission_configuration", "conversation_id":"permissions",
        "minimum_policy_revision":revision + 1,
    })).unwrap();
    let pending = execute_application_command(&mut store, read).unwrap();
    assert_eq!(pending["configuration_revision"], revision + 1);
    assert_eq!(pending["permission_profile"], "allow_all");
    assert_eq!(pending["state"], "pending", "saving cannot fabricate a Core acknowledgement");
}

#[test]
fn remote_snapshot_uses_source_admission_and_never_invents_local_acknowledgement() {
    let mut store = store();
    store.admit_remote_fixture(&nola_store::AdmittedReplicaTurnInput {
        user_part_kind: "text".into(),
        source_host_id: "host".into(), conversation_id: "permissions".into(),
        conversation_title: "Permissions".into(), user_message_id: "remote-user".into(),
        user_part_id: "remote-user-body".into(), user_text: "Read".into(), user_content_json: None,
        assistant_message_id: "remote-assistant".into(), assistant_part_id: "remote-body".into(),
        run_id: "remote-run".into(), context_checkpoint_id: None, created_at_ms: 1,
    }).unwrap();
    let read = |host| serde_json::from_value::<ApplicationCommand>(json!({
        "type":"read_permission_configuration", "conversation_id":"permissions",
        "source_host_id":host, "client_id":"fixture-client",
    })).unwrap();
    assert!(execute_application_command(&mut store, read("foreign")).is_err());
    let wrong_client = serde_json::from_value::<ApplicationCommand>(json!({
        "type":"read_permission_configuration", "conversation_id":"permissions",
        "source_host_id":"host", "client_id":"foreign-client",
    })).unwrap();
    assert!(execute_application_command(&mut store, wrong_client).is_err());
    let snapshot = execute_application_command(&mut store, read("host")).unwrap();
    assert_eq!(snapshot["owner"]["kind"], "remote");
    assert_eq!(snapshot["owner"]["identity"]["submission_id"], "remote-run");
    assert!(snapshot["active_policy"].is_null());
    assert_eq!(snapshot["state"], "pending");
    let revision = snapshot["configuration_revision"].as_i64().unwrap();
    let write = serde_json::from_value::<ApplicationCommand>(json!({
        "type":"set_conversation_permission", "conversation_id":"permissions",
        "expected_revision":revision, "expected_run_id":"remote-run",
        "expected_source_host_id":"host", "profile":"allow_all", "updated_at_ms":2,
    })).unwrap();
    assert_eq!(execute_application_command(&mut store, write).unwrap()["status"], "committed");
    assert!(store.peek_agent_command("permissions").unwrap().is_none(),
        "a mirror revision must not become a source policy command");
}

#[test]
fn selective_permission_state_requires_exact_store_frontier() {
    use nola_store_port::conversation_query::CurrentPermissionState;
    let mut store = store();
    save(&mut store, checkpoint());
    let stored = store.load_checkpoint("permissions").unwrap().unwrap();
    let current = CurrentPermissionState {
        active_permission_policy: None,
        conversation_id: "permissions".into(), revision: stored.revision,
        event_cursor: 7, session_id: "permissions".into(), state_session_id: "permissions".into(),
        active_turn_id: None, permissions: json!({"revision": 5, "pending": []}),
    };
    let cold: Value = serde_json::from_str(&store.load_session_permission_inputs("permissions", None).unwrap().unwrap()).unwrap();
    let hot: Value = serde_json::from_str(&store.load_session_permission_inputs("permissions", Some(&current)).unwrap().unwrap()).unwrap();
    assert_eq!(hot, cold);
    let mut stale = current.clone();
    stale.revision += 1;
    assert!(store.load_session_permission_inputs("permissions", Some(&stale)).is_err());
    let mut foreign = current.clone();
    foreign.conversation_id = "foreign".into();
    assert!(store.load_session_permission_inputs("permissions", Some(&foreign)).is_err());
    let mut cursor = current;
    cursor.event_cursor += 1;
    assert!(store.load_session_permission_inputs("permissions", Some(&cursor)).is_err());
}

#[test]
fn permission_query_excludes_large_unrelated_checkpoint_content() {
    let mut store = store();
    let mut payload = checkpoint();
    payload["state"]["unrelated_history"] = json!("x".repeat(10_000_000));
    save(&mut store, payload);
    assert!(
        store
            .load_session_permission_inputs("permissions", None)
            .unwrap()
            .unwrap()
            .len()
            < 1024
    );
    assert_eq!(
        query(&mut store).unwrap(),
        json!({"conversation_id":"permissions", "run_id":"permission-run", "session_id":"permissions",
            "active_turn_id":null, "event_cursor":7, "permission_projection_revision":5,
            "permissions":{"revision": 5, "pending": [],"active_policy":null}})
    );
}

#[test]
fn permission_query_rejects_wrong_identity_and_cursor() {
    for invalid in 0..5 {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("corrupt-permission.db");
        let mut store = NolaStore::open(&path).unwrap();
        save(&mut store, checkpoint());
        let mut payload = checkpoint();
        match invalid {
            0 => payload["session_id"] = json!("wrong"),
            1 => payload["state"]["session_id"] = json!("wrong"),
            2 => payload["last_event_cursor"]["sequence"] = json!(8),
            3 => payload["last_event_cursor"]["sequence"] = json!(true),
            _ => {
                payload["session_id"] = json!("same-foreign-session");
                payload["state"]["session_id"] = json!("same-foreign-session");
            }
        }
        let corrupt = rusqlite::Connection::open(&path).unwrap();
        corrupt.execute("UPDATE core_checkpoint_bodies SET checkpoint_json=?1 WHERE conversation_id='permissions'",
            [payload.to_string()]).unwrap();
        assert!(query(&mut store).is_err());
    }
}

#[test]
fn cold_permission_read_requires_independent_turn_admission_evidence() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("missing-admission.db");
    let mut store = NolaStore::open(&path).unwrap();
    save(&mut store, checkpoint());
    assert!(query(&mut store).unwrap().is_object());
    let corrupt = rusqlite::Connection::open(&path).unwrap();
    corrupt
        .execute(
            "DELETE FROM run_events WHERE run_id='permission-run' AND event_kind='turn_started'",
            [],
        )
        .unwrap();
    assert!(query(&mut store)
        .unwrap_err()
        .contains("admission evidence"));
}

#[test]
fn permission_control_reads_bind_empty_pending_to_exact_owner_and_revision() {
    let mut store = store();
    save(&mut store, checkpoint());
    let command = json!({
        "type":"load_conversation_permission_projection_v1",
        "conversation_id":"permissions", "run_id":"permission-run",
        "scope":{"type":"control", "session_id":"permissions", "active_turn_id":null,
            "event_cursor":6, "permission_projection_revision":5}
    });
    let current =
        execute_application_command(&mut store, serde_json::from_value(command.clone()).unwrap())
            .unwrap();
    assert_eq!(
        current["event_cursor"], 7,
        "echo the actual frontier, not the requested older cursor"
    );
    assert_eq!(current["permissions"]["pending"], json!([]));
    for (pointer, value) in [
        ("/run_id", json!("foreign-run")),
        ("/scope/session_id", json!("foreign-session")),
        ("/scope/active_turn_id", json!("foreign-turn")),
        ("/scope/event_cursor", json!(8)),
        ("/scope/permission_projection_revision", json!(4)),
        ("/scope/permission_projection_revision", json!(6)),
    ] {
        let mut invalid = command.clone();
        *invalid.pointer_mut(pointer).unwrap() = value;
        assert!(
            execute_application_command(&mut store, serde_json::from_value(invalid).unwrap())
                .is_err(),
            "empty pending must not bypass {pointer} ownership"
        );
    }
    let mut missing_turn = command.clone();
    missing_turn["scope"]
        .as_object_mut()
        .unwrap()
        .remove("active_turn_id");
    assert!(serde_json::from_value::<ApplicationCommand>(missing_turn).is_err());
    assert!(
        serde_json::from_value::<ApplicationCommand>(json!({
            "type":"load_conversation_permission_projection_v1", "conversation_id":"permissions"
        }))
        .is_err(),
        "the unscoped legacy query must not remain usable"
    );
}

#[test]
fn successor_run_cannot_reuse_an_empty_predecessor_permission_view() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("permission-owner.db");
    let mut store = NolaStore::open(&path).unwrap();
    save(&mut store, checkpoint());
    assert_eq!(query(&mut store).unwrap()["run_id"], "permission-run");
    terminal_sources::settle_fixture(&mut store, "permissions", "permission-run", "permission-run", "permission-run:user");
    // This read-contract fixture deliberately gives the successor equal
    // permission/session/cursor values: Run ownership must still distinguish it.
    let fixture = rusqlite::Connection::open(&path).unwrap();
    fixture
        .execute(
            "UPDATE runs SET created_at_ms=999 WHERE id='permission-run'",
            [],
        )
        .unwrap();
    save_for_run(&mut store, checkpoint(), "successor-run");
    assert!(query(&mut store).is_err());
    let successor = execute_application_command(
        &mut store,
        ApplicationCommand::LoadConversationPermissionProjectionV1 {
            conversation_id: "permissions".into(),
            run_id: "successor-run".into(),
            scope: PermissionProjectionReadScope::CurrentRun,
        },
    )
    .unwrap();
    assert_eq!(successor["run_id"], "successor-run");
    assert_eq!(successor["permissions"], json!({"revision":5,"pending":[],"active_policy":null}));
}

#[test]
fn permission_query_preserves_interruption_receipts_without_turning_them_into_authorization() {
    let mut store = store();
    let receipt = json!({
        "request_id": "request-interrupted", "turn_id": "turn-1", "call_id": "call-1",
        "capability_id": "file.read", "resource": "file:///workspace/document.txt",
        "outcome": {"type": "interrupted", "reason": "user cancelled the Run"}
    });
    let mut payload = checkpoint();
    payload["state"]["execution"]["permissions"]["receipts"] = json!([receipt.clone()]);
    save(&mut store, payload);
    let view = query(&mut store).unwrap();
    assert_eq!(view["permissions"], json!({"revision":5,"pending":[],"active_policy":null}));
    assert!(
        view["permissions"].get("receipts").is_none(),
        "control reads must not copy receipt history"
    );
    let saved = store.load_checkpoint("permissions").unwrap().unwrap();
    let saved: Value = serde_json::from_str(&saved.checkpoint_json).unwrap();
    assert_eq!(
        saved["state"]["execution"]["permissions"]["receipts"],
        json!([receipt])
    );
    assert!(store
        .effective_permission_grants(Some("permissions"), None)
        .unwrap()
        .is_empty());
}

#[test]
fn permission_query_excludes_stale_local_permissions_when_a_remote_run_owns_conversation() {
    let mut store = store();
    assert!(query(&mut store).unwrap().is_null());
    store
        .save_checkpoint(&nola_store::SaveCheckpointInput {
            conversation_id: "permissions".into(),
            expected_revision: 0,
            revision: 1,
            event_cursor: 7,
            checkpoint_json: checkpoint().to_string(),
            updated_at_ms: 2,
        })
        .unwrap();
    assert!(
        query(&mut store).is_err(),
        "an unowned local checkpoint is not permission authority"
    );
    store
        .admit_remote_fixture(&nola_store::AdmittedReplicaTurnInput {
            user_part_kind: "text".into(),
            source_host_id: "host".into(),
            conversation_id: "permissions".into(),
            conversation_title: "Permissions".into(),
            user_message_id: "remote-user".into(),
            user_part_id: "remote-user-body".into(),
            user_text: "Read".into(),
            user_content_json: None,
            assistant_message_id: "remote-assistant".into(),
            assistant_part_id: "remote-body".into(),
            run_id: "remote-run".into(),
            context_checkpoint_id: None,
            created_at_ms: 1,
        })
        .unwrap();
    assert!(query(&mut store).unwrap().is_null());
}
