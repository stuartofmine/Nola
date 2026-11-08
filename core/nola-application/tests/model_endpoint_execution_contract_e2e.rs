// SPDX-License-Identifier: Apache-2.0
//! Endpoint edits must not carry model execution authority to another route.

use nola_application::{
    execute_application_command, ApplicationCommand, ModelCommand, ModelExecutionCapabilities,
    ModelProfile,
};
use nola_store::NolaStore;
use rusqlite::Connection;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use tempfile::tempdir;

const ORIGINAL_URL: &str = "https://fixture.invalid/v1";

fn profile(id: &str, endpoint: &str, selected: bool, tools: bool) -> ModelProfile {
    ModelProfile {
        id: id.into(),
        display_name: id.into(),
        provider: "openai-compatible".into(),
        model_name: format!("model-{id}"),
        runtime: "remote".into(),
        source_json: json!({
            "endpointID":endpoint, "endpointName":"Original", "endpointEnabled":true,
            "baseURL":ORIGINAL_URL, "wireProtocol":"responses", "capabilities":["chat"],
            "executionCapabilities":{
                "toolCalls":tools,"imageInput":false,"reasoning":false,"samplingParameters":false
            },
            "fixtureMetadata":{"preserve":true}
        })
        .to_string(),
        parameters_json: json!({"maxOutputTokens":16384}).to_string(),
        selected,
        enabled: true,
        revision: 0,
        created_at_ms: 1,
        updated_at_ms: 1,
        deleted_at_ms: None,
    }
}

fn seed(store: &mut NolaStore) {
    for input in [
        profile("first", "endpoint", true, true),
        profile("second", "endpoint", false, false),
        profile("foreign", "other", false, true),
    ] {
        execute_application_command(
            store,
            ApplicationCommand::Model {
                command: ModelCommand::UpsertProfile { input },
            },
        )
        .unwrap();
    }
}

fn profiles(store: &mut NolaStore) -> Vec<ModelProfile> {
    serde_json::from_value(
        execute_application_command(
            store,
            ApplicationCommand::Model {
                command: ModelCommand::ListProfiles,
            },
        )
        .unwrap(),
    )
    .unwrap()
}

fn get_profile(store: &mut NolaStore, id: &str) -> ModelProfile {
    serde_json::from_value::<Option<ModelProfile>>(
        execute_application_command(
            store,
            ApplicationCommand::Model {
                command: ModelCommand::GetProfile { id: id.into() },
            },
        )
        .unwrap(),
    )
    .unwrap()
    .unwrap()
}

fn revisions(profiles: &[ModelProfile]) -> BTreeMap<String, i64> {
    profiles
        .iter()
        .filter(|profile| {
            serde_json::from_str::<Value>(&profile.source_json).unwrap()["endpointID"] == "endpoint"
        })
        .map(|profile| (profile.id.clone(), profile.revision))
        .collect()
}

fn configure(
    store: &mut NolaStore,
    expected_revisions: BTreeMap<String, i64>,
    base_url: &str,
    protocol: &str,
    name: &str,
    enabled: bool,
    at: i64,
) -> Result<Value, String> {
    execute_application_command(
        store,
        ApplicationCommand::Model {
            command: ModelCommand::ConfigureEndpoint {
                endpoint_id: "endpoint".into(),
                expected_revisions,
                name: name.into(),
                base_url: base_url.into(),
                wire_protocol: protocol.into(),
                enabled,
                updated_at_ms: at,
            },
        },
    )
}

fn sync_rows(connection: &Connection) -> Vec<String> {
    connection
        .prepare(
            "SELECT json_object('sequence',local_sequence,'operation',operation_id,
         'type',entity_type,'id',entity_id,'revision',revision,'payload',payload_json,
         'status',status) FROM sync_outbox ORDER BY local_sequence",
        )
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<Vec<String>, _>>()
        .unwrap()
}

#[test]
fn endpoint_url_or_protocol_change_invalidates_execution_but_keeps_models_readable() {
    for (base_url, protocol) in [
        ("https://different.invalid/v1", "responses"),
        ("https://fixture.invalid/v2", "responses"),
        (ORIGINAL_URL, "chat_completions"),
    ] {
        let directory = tempdir().unwrap();
        let database = directory.path().join("endpoint-route.db");
        let mut store = NolaStore::open(&database).unwrap();
        seed(&mut store);
        let before = profiles(&mut store);
        let returned: Vec<ModelProfile> = serde_json::from_value(
            configure(
                &mut store,
                revisions(&before),
                base_url,
                protocol,
                "Renamed",
                true,
                20,
            )
            .unwrap(),
        )
        .unwrap();
        let after = profiles(&mut store);
        assert_eq!(returned, after);
        assert_eq!(after.len(), before.len());
        for previous in &before {
            let current = get_profile(&mut store, &previous.id);
            if previous.id == "foreign" {
                assert_eq!(&current, previous);
                continue;
            }
            let source: Value = serde_json::from_str(&current.source_json).unwrap();
            assert!(source["executionCapabilities"].is_null());
            assert!(ModelExecutionCapabilities::from_source(&source)
                .unwrap_err()
                .starts_with("model_execution_capabilities_required:"));
            assert_eq!(source["baseURL"], base_url);
            assert_eq!(source["wireProtocol"], protocol);
            assert_eq!(source["fixtureMetadata"], json!({"preserve":true}));
            assert_eq!(current.model_name, previous.model_name);
            assert_eq!(current.parameters_json, previous.parameters_json);
            assert_eq!(current.selected, previous.selected);
            assert_eq!(current.enabled, previous.enabled);
            assert!(current.revision > previous.revision);
        }
        drop(store);
        let mut reopened = NolaStore::open(&database).unwrap();
        assert_eq!(profiles(&mut reopened), after);
    }
}

#[test]
fn endpoint_metadata_and_equivalent_url_edits_preserve_explicit_execution_declarations() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("endpoint-metadata.db");
    let mut store = NolaStore::open(&database).unwrap();
    seed(&mut store);
    let before = profiles(&mut store);
    for (base_url, name, enabled, at) in [
        (" HTTPS://FIXTURE.invalid:443/v1 ", "Renamed", false, 20),
        (ORIGINAL_URL, "Renamed again", true, 30),
    ] {
        let current = profiles(&mut store);
        configure(
            &mut store,
            revisions(&current),
            base_url,
            "responses",
            name,
            enabled,
            at,
        )
        .unwrap();
        for previous in &before {
            let current = get_profile(&mut store, &previous.id);
            if previous.id == "foreign" {
                assert_eq!(&current, previous);
                continue;
            }
            let source: Value = serde_json::from_str(&current.source_json).unwrap();
            let original_source: Value = serde_json::from_str(&previous.source_json).unwrap();
            assert_eq!(
                source["executionCapabilities"],
                original_source["executionCapabilities"]
            );
            assert_eq!(
                ModelExecutionCapabilities::from_source(&source)
                    .unwrap()
                    .tool_calls,
                previous.id == "first"
            );
            assert_eq!(source["baseURL"], ORIGINAL_URL);
            assert_eq!(source["endpointName"], name);
            assert_eq!(source["endpointEnabled"], enabled);
            assert_eq!(current.model_name, previous.model_name);
            assert_eq!(current.parameters_json, previous.parameters_json);
            assert_eq!(current.selected, previous.selected);
        }
    }
    let after = profiles(&mut store);
    drop(store);
    let mut reopened = NolaStore::open(&database).unwrap();
    assert_eq!(profiles(&mut reopened), after);
}

#[test]
fn stale_endpoint_route_edit_cannot_invalidate_declarations_or_write_profile_revisions() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("endpoint-stale.db");
    let mut store = NolaStore::open(&database).unwrap();
    seed(&mut store);
    let stale_revisions = revisions(&profiles(&mut store));
    configure(
        &mut store,
        stale_revisions.clone(),
        ORIGINAL_URL,
        "responses",
        "Current",
        true,
        20,
    )
    .unwrap();
    let before = profiles(&mut store);
    let connection = Connection::open(&database).unwrap();
    let before_sync = sync_rows(&connection);
    let error = configure(
        &mut store,
        stale_revisions,
        "https://different.invalid/v1",
        "chat_completions",
        "Stale",
        false,
        30,
    )
    .unwrap_err();
    assert!(error.contains("revision_conflict"), "{error}");
    assert_eq!(profiles(&mut store), before);
    assert_eq!(sync_rows(&connection), before_sync);
    for previous in &before {
        assert_eq!(get_profile(&mut store, &previous.id), *previous);
    }
    drop(store);
    let mut reopened = NolaStore::open(&database).unwrap();
    assert_eq!(profiles(&mut reopened), before);
    assert_eq!(sync_rows(&connection), before_sync);
}

#[test]
fn native_resolution_uses_exact_profile_and_core_automatic_reasoning() {
    let mut store = NolaStore::open(":memory:").unwrap();
    for (id, url, selected) in [("first", ORIGINAL_URL, true), ("second", "https://second.invalid/v1", false)] {
        let mut input = profile(id, id, selected, false);
        let mut source: Value = serde_json::from_str(&input.source_json).unwrap();
        source["baseURL"] = json!(url);
        source["reasoning"] = json!({"control":"automatic"});
        source["executionCapabilities"]["reasoning"] = json!(true);
        source["credentialRef"] = json!(format!("fixture:{id}"));
        input.source_json = source.to_string();
        execute_application_command(&mut store, ApplicationCommand::Model {
            command: ModelCommand::UpsertProfile { input },
        }).unwrap();
    }
    let selected = profiles(&mut store).into_iter().find(|p|p.id=="second").unwrap();
    let query = ApplicationCommand::ResolveRemoteModelProfile {
        profile_id: selected.id.clone(), expected_revision: selected.revision, request: json!({}),
    };
    assert!(query.is_read_only_query());
    let result = execute_application_command(&mut store, query).unwrap();
    assert_eq!(result["resolved"]["endpoint"], "https://second.invalid/v1");
    assert_eq!(result["credential_ref"], "fixture:second");
    assert_eq!(result["resolved"]["runtime"]["reasoning_selection"], Value::Null);
    assert!(execute_application_command(&mut store, ApplicationCommand::ResolveRemoteModelProfile {
        profile_id:selected.id, expected_revision:selected.revision+1, request:json!({}),
    }).is_err());
}
