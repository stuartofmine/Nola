// SPDX-License-Identifier: Apache-2.0
use nola_application::{execute_application_command, ApplicationCommand};
use nola_store::NolaStore;
use nola_store_port::conversation_profile::CreateConversationInput;
use serde_json::json;

fn command(value: serde_json::Value) -> ApplicationCommand {
    serde_json::from_value(value).unwrap()
}

fn fixture() -> NolaStore {
    let mut store = NolaStore::open(":memory:").unwrap();
    store.create_conversation(&CreateConversationInput {
        id: "c".into(), title: "Conversation".into(), revision: 1,
        settings_json: Some("{}".into()), created_at_ms: 1, updated_at_ms: 1,
    }).unwrap();
    store
}

#[test]
fn inactive_snapshot_and_permission_commit_use_one_revision_contract() {
    let mut store = fixture();
    let read = || command(json!({"type":"read_permission_configuration","conversation_id":"c"}));
    let initial = execute_application_command(&mut store, read()).unwrap();
    assert_eq!(initial["owner"], json!({"kind":"inactive"}));
    assert_eq!(initial["state"], "inactive");
    assert_eq!(initial["configuration_revision"], 1);
    let write = |revision| command(json!({
        "type":"set_conversation_permission", "conversation_id":"c",
        "expected_revision":revision, "expected_run_id":null, "expected_source_host_id":null,
        "profile":"allow_all", "updated_at_ms":2,
        "additional_configuration":json!({"memoryEnabledOverride":false}).to_string(),
    }));
    assert_eq!(execute_application_command(&mut store, write(1)).unwrap()["status"], "committed");
    assert_eq!(execute_application_command(&mut store, write(1)).unwrap()["status"], "conflict");
    let current = execute_application_command(&mut store, read()).unwrap();
    assert_eq!(current["configuration_revision"], 2);
    assert_eq!(current["permission_profile"], "allow_all");
    assert_eq!(current["settings"]["memoryEnabledOverride"], false);
}

#[test]
fn stale_owner_or_unresolved_observation_cannot_become_inactive_save() {
    let mut store = fixture();
    let stale = command(json!({
        "type":"set_conversation_permission", "conversation_id":"c",
        "expected_revision":1, "expected_run_id":"old-run", "expected_source_host_id":null,
        "profile":"allow_all", "updated_at_ms":2,
    }));
    assert!(execute_application_command(&mut store, stale).unwrap_err().contains("owner changed"));
    for extra in [
        json!({"unresolved_run_ids":["unknown-run"]}),
        json!({"source_unconfirmed":true}),
    ] {
        let mut read = json!({"type":"read_permission_configuration","conversation_id":"c"});
        read.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        assert!(execute_application_command(&mut store, command(read)).is_err());
    }
    assert_eq!(store.load_conversation("c").unwrap().unwrap().configuration_revision, 1);
}

#[test]
fn selection_owns_exclusive_fields_and_rejects_invalid_parameters() {
    use nola_application::conversation_configuration::SelectConversationExecutionInput;
    let input = |selection| serde_json::from_value::<SelectConversationExecutionInput>(json!({
        "conversation_id":"c", "expected_revision":7, "updated_at_ms":2, "selection":selection,
    })).unwrap();
    let selected = input(json!({"kind":"model","model_id":"configured:profile",
        "parameters":{"temperature":0.2}, "presentation":{"showThinking":true}})).into_patch().unwrap();
    let patch: serde_json::Value = serde_json::from_str(&selected.patch_json).unwrap();
    assert_eq!(patch["executionHostBinding"], serde_json::Value::Null);
    assert_eq!(patch["clientHostParameters"], serde_json::Value::Null);
    assert_eq!(patch["modelOverrides"]["configured:profile"]["temperature"], 0.2);
    assert_eq!(selected.expected_revision, 7);
    assert!(input(json!({"kind":"model","model_id":"wire-name",
        "parameters":{}, "presentation":{}})).into_patch().is_err());
    assert!(input(json!({"kind":"host","binding":{"baseURL":"https://user:secret@example.com"},
        "parameters":{}, "presentation":{}})).into_patch().is_err());
    let inherited = input(json!({"kind":"inherit"})).into_patch().unwrap();
    assert!(inherited.patch_json.fields()["modelSelectionID"].is_null());
    assert!(inherited.patch_json.fields()["executionHostBinding"].is_null());
    let location = input(json!({"kind":"location","execution_host":"remote"})).into_patch().unwrap();
    assert_eq!(location.patch_json.fields().len(), 1);
    assert_eq!(location.patch_json.fields()["executionHostRawValue"], "remote");
    let mut conflicting = input(json!({"kind":"inherit"}));
    conflicting.additional_configuration = Some(
        json!({"modelSelectionID":"configured:other"}).to_string().parse().unwrap());
    assert!(conflicting.into_patch().is_err());
}
