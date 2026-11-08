// SPDX-License-Identifier: Apache-2.0
//! Configuration admission and execution share the Domain generation contract.

use nola_application::{execute_application_command, ApplicationCommand};
use nola_domain::generation::GenerationOptions;
use nola_store::NolaStore;
use nola_store_port::conversation_profile::{
    CreateConversationInput, PatchConversationConfigurationInput,
};
use serde_json::{json, Value};

#[test]
fn seed_intent_persists_exact_values_and_null_patch_restores_inheritance() {
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("seed-intent.sqlite");
    let mut store = NolaStore::open(&database).unwrap();
    store.create_conversation(&CreateConversationInput {
        id: "seed".into(), title: "Seed".into(), revision: 1,
        settings_json: Some("{}".into()), created_at_ms: 1, updated_at_ms: 1,
    }).unwrap();
    let cases = [json!({"kind":"clear"}), json!({"kind":"value","value":"0"}),
        json!({"kind":"value","value":i64::MIN.to_string()}),
        json!({"kind":"value","value":i64::MAX.to_string()}), Value::Null];
    for (index, seed) in cases.iter().enumerate() {
        execute_application_command(&mut store, ApplicationCommand::PatchConversationConfiguration {
            input: PatchConversationConfigurationInput {
                conversation_id: "seed".into(), expected_revision: index as i64 + 1,
                patch_json: json!({"modelOverrides":{"configured:model":{"seed":seed}}}).to_string().parse().unwrap(),
                updated_at_ms: index as i64 + 2,
            },
        }).unwrap();
        drop(store);
        store = NolaStore::open(&database).unwrap();
        let connection = rusqlite::Connection::open(&database).unwrap();
        let raw: String = connection.query_row("SELECT settings_json FROM conversation_settings WHERE conversation_id='seed'", [], |row| row.get(0)).unwrap();
        let saved: Value = serde_json::from_str(&raw).unwrap();
        if seed.is_null() {
            assert!(saved["modelOverrides"]["configured:model"].get("seed").is_none());
        } else {
            assert_eq!(&saved["modelOverrides"]["configured:model"]["seed"], seed);
        }
    }
}

#[test]
fn connected_host_overrides_are_isolated_and_survive_reopen() {
    use base64::Engine;
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("remote-intent.sqlite");
    let mut store = NolaStore::open(&database).unwrap();
    store.create_conversation(&CreateConversationInput {
        id:"remote".into(), title:"Remote intent".into(), revision:1,
        settings_json:Some("{}".into()), created_at_ms:1, updated_at_ms:1,
    }).unwrap();
    let key = |host: &str| format!("connectedHost:{}:{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(host),
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode("same-profile"));
    let first = key("host:first");
    let second = key("host:second");
    let mut models = serde_json::Map::new();
    models.insert(first.clone(), json!({"temperature":0,"seed":{"kind":"value","value":"0"}}));
    models.insert(second.clone(), json!({"temperature":1}));
    execute_application_command(&mut store, ApplicationCommand::PatchConversationConfiguration {
        input:PatchConversationConfigurationInput { conversation_id:"remote".into(), expected_revision:1,
            patch_json:json!({"modelOverrides":models}).to_string().parse().unwrap(), updated_at_ms:2 }
    }).unwrap();
    drop(store);
    let _store = NolaStore::open(&database).unwrap();
    let connection = rusqlite::Connection::open(&database).unwrap();
    let raw: String = connection.query_row("SELECT settings_json FROM conversation_settings WHERE conversation_id='remote'", [], |row| row.get(0)).unwrap();
    let saved: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(saved["modelOverrides"][&first], json!({"temperature":0,"seed":{"kind":"value","value":"0"}}));
    assert_eq!(saved["modelOverrides"][&second], json!({"temperature":1}));
    assert!(saved["modelOverrides"].get("configured:same-profile").is_none());
}

#[test]
fn invalid_generation_never_commits_and_valid_zero_values_survive_reopen() {
    let root = tempfile::tempdir().unwrap();
    let database = root.path().join("generation.sqlite");
    let mut store = NolaStore::open(&database).unwrap();
    store
        .create_conversation(&CreateConversationInput {
            id: "c".into(),
            title: "Generation contract".into(),
            revision: 1,
            settings_json: Some("{}".into()),
            created_at_ms: 1,
            updated_at_ms: 1,
        })
        .unwrap();
    let read = || {
        let connection = rusqlite::Connection::open(&database).unwrap();
        let raw: String = connection
            .query_row(
                "SELECT settings_json FROM conversation_settings WHERE conversation_id='c'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        serde_json::from_str::<Value>(&raw).unwrap()
    };
    let original = read();
    let changes = store.pending_sync_changes(100).unwrap().len();
    for fields in [
        json!({"topK":0}),
        json!({"topK":1.5}),
        json!({"seed":"0"}),
        json!({"frequencyPenalty":2.1}),
        json!({"repetitionPenalty":0.5}),
    ] {
        let result = json!({"modelOverrides":{"configured:model":fields}}).to_string()
            .parse::<nola_domain::configuration_patch::ConversationConfigurationPatch>();
        assert!(result.is_err(), "invalid configuration was admitted");
        assert_eq!(
            read(),
            original,
            "rejection must not change persisted settings"
        );
        assert_eq!(store.pending_sync_changes(100).unwrap().len(), changes);
    }
    execute_application_command(&mut store, ApplicationCommand::PatchConversationConfiguration {
        input: PatchConversationConfigurationInput {
            conversation_id: "c".into(), expected_revision: 1,
            patch_json: json!({"modelOverrides":{"configured:model":{"topK":1,"seed":{"kind":"value","value":"0"},"presencePenalty":0}}}).to_string().parse().unwrap(),
            updated_at_ms: 3,
        }
    }).unwrap();
    drop(store);
    let _reopened = NolaStore::open(&database).unwrap();
    let saved = read();
    let fields = &saved["modelOverrides"]["configured:model"];
    assert_eq!(fields["seed"], json!({"kind":"value","value":"0"}));
    assert_eq!(fields["topK"], 1);
    let revision: i64 = rusqlite::Connection::open(&database)
        .unwrap()
        .query_row(
            "SELECT revision FROM conversations WHERE id='c'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(revision, 2);
    let execution = GenerationOptions {
        top_k: fields["topK"].as_u64(),
        seed: serde_json::from_value::<nola_domain::generation_override::SeedOverride>(fields["seed"].clone()).unwrap().resolved(),
        presence_penalty: fields["presencePenalty"].as_f64(),
        ..Default::default()
    };
    assert!(
        execution.validate().is_ok(),
        "admitted values must also be executable"
    );
}
