// SPDX-License-Identifier: Apache-2.0
//! Real Application -> narrow repository -> SQLite connection configuration.
use nola_application::{execute_application_command, ApplicationCommand, ModelCommand};
use nola_domain::{
    generation::ModelProtocol, model_capabilities::ModelExecutionCapabilities, model_connection::*,
};
use nola_store::NolaStore;
use serde_json::Value;

fn execute(store: &mut NolaStore, command: ModelCommand) -> Result<Value, String> {
    execute_application_command(store, ApplicationCommand::Model { command })
}

#[test]
fn import_failure_rolls_back_account_connection_and_profiles_together() {
    let mut store = NolaStore::open(":memory:").unwrap();
    let mut route = connection("atomic");
    route.account_id = Some("account".into());
    let account = ModelAccount { id: "account".into(), display_name: "Account".into(),
        source: ModelSourceKind::Manual, credential_ref: Some("private-reference".into()), state: ModelAccountState::Ready };
    let mut invalid = model("invalid", ModelProtocol::Responses);
    invalid.context_window = Some(0);
    let command = |models| ModelCommand::ImportConnection {
        input: route.clone(), expected_revision: None, account: Some(account.clone()),
        expected_account_revision: None, models, enable_models: true, updated_at_ms: 10,
    };
    assert!(execute(&mut store, command(vec![invalid])).is_err());
    assert_eq!(execute(&mut store, ModelCommand::ListAccounts).unwrap(), serde_json::json!([]));
    assert_eq!(execute(&mut store, ModelCommand::ListConnections).unwrap(), serde_json::json!([]));
    assert!(store.list_model_profiles().unwrap().is_empty());
    execute(&mut store, command(vec![model("valid", ModelProtocol::Responses)])).unwrap();
    assert_eq!(store.list_model_profiles().unwrap().len(), 1);
    let before_accounts = execute(&mut store, ModelCommand::ListAccounts).unwrap();
    let before_connections = execute(&mut store, ModelCommand::ListConnections).unwrap();
    let before_profiles = store.list_model_profiles().unwrap();
    let mut replacement = account.clone();
    replacement.credential_ref = Some("new-private-reference".into());
    assert!(execute(&mut store, ModelCommand::ImportConnection {
        input: route.clone(), expected_revision: Some(999), account: Some(replacement),
        expected_account_revision: Some(before_accounts[0]["revision"].as_i64().unwrap()),
        models: vec![model("replacement", ModelProtocol::Responses)], enable_models: true, updated_at_ms: 20,
    }).is_err());
    assert_eq!(execute(&mut store, ModelCommand::ListAccounts).unwrap(), before_accounts);
    assert_eq!(execute(&mut store, ModelCommand::ListConnections).unwrap(), before_connections);
    assert_eq!(store.list_model_profiles().unwrap(), before_profiles);

    let mut other = connection("shared");
    other.account_id = route.account_id.clone();
    save(&mut store, other, None);
    let result = execute(&mut store, ModelCommand::ImportConnection {
        input: route, expected_revision: Some(before_connections[0]["revision"].as_i64().unwrap()),
        account: Some(account), expected_account_revision: Some(before_accounts[0]["revision"].as_i64().unwrap()),
        models: vec![], enable_models: false, updated_at_ms: 30,
    });
    assert!(result.unwrap_err().contains("import_account_shared_requires_separate_binding"));
    assert_eq!(execute(&mut store, ModelCommand::ListAccounts).unwrap(), before_accounts);
}

fn connection(id: &str) -> ModelConnection {
    ModelConnection {
        id: id.into(),
        display_name: format!("Connection {id}"),
        base_url: "https://fixture.invalid/v1".into(),
        protocols: vec![ModelProtocol::Responses, ModelProtocol::ChatCompletions],
        account_id: None,
        source: ModelSourceKind::Manual,
        source_id: None,
        enabled: true,
    }
}

fn model(name: &str, protocol: ModelProtocol) -> CatalogModel {
    CatalogModel {
        model_id: name.into(),
        display_name: name.into(),
        protocol,
        context_window: Some(128000),
        output_limit: Some(8192),
        modalities: vec!["text".into()],
        reasoning_levels: vec![], summary_supported: None,
        execution_capabilities: Some(ModelExecutionCapabilities::default()),
    }
}

fn save(
    store: &mut NolaStore,
    input: ModelConnection,
    revision: Option<i64>,
) -> ModelConnectionSnapshot {
    serde_json::from_value(
        execute(
            store,
            ModelCommand::SaveConnection {
                input,
                expected_revision: revision,
                updated_at_ms: 1,
            },
        )
        .unwrap(),
    )
    .unwrap()
}

fn refresh(
    store: &mut NolaStore,
    id: &str,
    revision: i64,
    models: Vec<CatalogModel>,
    at: i64,
) -> ModelConnectionSnapshot {
    serde_json::from_value(
        execute(
            store,
            ModelCommand::RefreshConnectionCatalog {
                connection_id: id.into(),
                expected_revision: revision,
                models,
                observed_at_ms: at,
            },
        )
        .unwrap(),
    )
    .unwrap()
}

fn select(
    store: &mut NolaStore,
    id: &str,
    revision: i64,
    enabled: &[&str],
    selected: Option<&str>,
) -> ModelConnectionSnapshot {
    serde_json::from_value(
        execute(
            store,
            ModelCommand::ConfigureConnectionModels {
                connection_id: id.into(),
                expected_revision: revision,
                enabled_model_ids: enabled.iter().map(|s| (*s).into()).collect(),
                selected_model_id: selected.map(str::to_owned),
                updated_at_ms: 10,
            },
        )
        .unwrap(),
    )
    .unwrap()
}

#[test]
fn empty_connections_are_durable_and_do_not_require_discovery() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("models.sqlite");
    let mut store = NolaStore::open(&path).unwrap();
    let first = save(&mut store, connection("one"), None);
    assert_eq!(first.revision, 1);
    assert!(store.list_model_profiles().unwrap().is_empty());
    drop(store);
    let mut store = NolaStore::open(&path).unwrap();
    assert_eq!(
        execute(&mut store, ModelCommand::ListConnections)
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(execute(
        &mut store,
        ModelCommand::SaveConnection {
            input: connection("one"),
            expected_revision: None,
            updated_at_ms: 2,
        }
    )
    .unwrap_err()
    .contains("revision_conflict"));
    save(&mut store, connection("two"), None);
    assert_eq!(store.list_model_connections().unwrap().len(), 2);
}

#[test]
fn capability_evidence_override_and_declaration_are_separate_and_route_bound() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("capabilities.sqlite");
    let mut store = NolaStore::open(&path).unwrap();
    let initial = save(&mut store, connection("probe"), None);
    let catalog = refresh(
        &mut store,
        "probe",
        initial.revision,
        vec![model("text", ModelProtocol::Responses)],
        20,
    );
    let selected = select(
        &mut store,
        "probe",
        catalog.revision,
        &["responses:text"],
        Some("responses:text"),
    );
    let attempt = ModelProbeAttempt {
        request_id: "fixture-probe".into(),
        connection_id: "probe".into(),
        connection_revision: selected.revision,
        account_id: None,
        account_revision: None,
        credential_binding: "a".repeat(64),
        model_id: "text".into(),
        protocol: ModelProtocol::Responses,
        started_at_ms: 30,
        finished_at_ms: None,
        text_confirmed: None,
    };
    execute(
        &mut store,
        ModelCommand::ReserveModelProbe {
            input: attempt.clone(),
        },
    )
    .unwrap();
    assert!(execute(
        &mut store,
        ModelCommand::ReserveModelProbe {
            input: attempt.clone()
        }
    )
    .is_err());
    let evidence = ModelCapabilityEvidence {
        request_id: attempt.request_id.clone(),
        connection_revision: selected.revision,
        account_revision: None,
        credential_binding: attempt.credential_binding.clone(),
        observed_at_ms: 40,
        requests_sent: 1,
        text_generation: true,
    };
    let observed: ModelConnectionSnapshot = serde_json::from_value(
        execute(
            &mut store,
            ModelCommand::RecordConnectionCapabilityEvidence {
                connection_id: "probe".into(),
                expected_revision: selected.revision,
                model_id: "text".into(),
                protocol: ModelProtocol::Responses,
                evidence,
            },
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(observed.document.catalog, selected.document.catalog);
    let override_caps = ModelExecutionCapabilities {
        tool_calls: true,
        ..Default::default()
    };
    let overridden: ModelConnectionSnapshot = serde_json::from_value(
        execute(
            &mut store,
            ModelCommand::SetConnectionCapabilityOverride {
                connection_id: "probe".into(),
                expected_revision: observed.revision,
                model_id: "text".into(),
                protocol: ModelProtocol::Responses,
                capabilities: Some(override_caps),
                updated_at_ms: 50,
            },
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(overridden.document.catalog, selected.document.catalog);
    let profiles = execute(&mut store, ModelCommand::ListProfiles).unwrap();
    let source: Value = serde_json::from_str(profiles[0]["source_json"].as_str().unwrap()).unwrap();
    assert_eq!(source["executionCapabilityBasis"], "user_override");
    assert_eq!(source["executionCapabilities"]["toolCalls"], true);
    assert_eq!(profiles[0]["selected"], true);
    let cleared: ModelConnectionSnapshot = serde_json::from_value(
        execute(
            &mut store,
            ModelCommand::SetConnectionCapabilityOverride {
                connection_id: "probe".into(),
                expected_revision: overridden.revision,
                model_id: "text".into(),
                protocol: ModelProtocol::Responses,
                capabilities: None,
                updated_at_ms: 60,
            },
        )
        .unwrap(),
    )
    .unwrap();
    assert!(cleared.document.capability_assessments.as_ref().unwrap()[0]
        .evidence
        .is_some());
    let profiles = execute(&mut store, ModelCommand::ListProfiles).unwrap();
    let source: Value = serde_json::from_str(profiles[0]["source_json"].as_str().unwrap()).unwrap();
    assert_eq!(source["executionCapabilityBasis"], "catalog_declaration");
    assert_eq!(source["executionCapabilities"]["toolCalls"], false);
    let mut changed = cleared.document.connection.clone();
    changed.base_url = "https://other.fixture.invalid/v1".into();
    let changed = save(&mut store, changed, Some(cleared.revision));
    assert!(changed.document.capability_assessments.is_none());
    drop(store);
    let mut reopened = NolaStore::open(path).unwrap();
    assert!(execute(
        &mut reopened,
        ModelCommand::ReserveModelProbe { input: attempt }
    )
    .is_err());
}

#[test]
fn capability_evidence_rejects_independent_account_revision_change() {
    let directory = tempfile::tempdir().unwrap();
    let mut store = NolaStore::open(directory.path().join("account-probe.sqlite")).unwrap();
    let account = ModelAccount {
        id: "probe-account".into(),
        display_name: "Probe account".into(),
        source: ModelSourceKind::Manual,
        credential_ref: Some("opaque-fixture".into()),
        state: ModelAccountState::Ready,
    };
    let saved: ModelAccountSnapshot = serde_json::from_value(
        execute(
            &mut store,
            ModelCommand::SaveAccount {
                input: account.clone(),
                expected_revision: None,
                updated_at_ms: 1,
            },
        )
        .unwrap(),
    )
    .unwrap();
    let mut route = connection("account-probe");
    route.account_id = Some(account.id.clone());
    let route = save(&mut store, route, None);
    let route = refresh(
        &mut store,
        "account-probe",
        route.revision,
        vec![model("text", ModelProtocol::Responses)],
        2,
    );
    execute(
        &mut store,
        ModelCommand::ReserveModelProbe {
            input: ModelProbeAttempt {
                request_id: "account-probe-attempt".into(),
                connection_id: route.document.connection.id.clone(),
                connection_revision: route.revision,
                account_id: Some(account.id.clone()),
                account_revision: Some(saved.revision),
                credential_binding: "a".repeat(64),
                model_id: "text".into(),
                protocol: ModelProtocol::Responses,
                started_at_ms: 3,
                finished_at_ms: None,
                text_confirmed: None,
            },
        },
    )
    .unwrap();
    let mut changed = account;
    changed.state = ModelAccountState::Denied;
    execute(
        &mut store,
        ModelCommand::SaveAccount {
            input: changed,
            expected_revision: Some(saved.revision),
            updated_at_ms: 4,
        },
    )
    .unwrap();
    assert!(execute(
        &mut store,
        ModelCommand::RecordConnectionCapabilityEvidence {
            connection_id: route.document.connection.id,
            expected_revision: route.revision,
            model_id: "text".into(),
            protocol: ModelProtocol::Responses,
            evidence: ModelCapabilityEvidence {
                request_id: "account-probe-attempt".into(),
                connection_revision: route.revision,
                account_revision: Some(saved.revision),
                credential_binding: "a".repeat(64),
                observed_at_ms: 5,
                requests_sent: 1,
                text_generation: true,
            }
        }
    )
    .is_err());
}

#[test]
fn catalog_omission_and_return_preserve_user_capability_override() {
    let mut store = NolaStore::open(":memory:").unwrap();
    let initial = save(&mut store, connection("one"), None);
    let catalog = refresh(&mut store, "one", initial.revision,
        vec![model("alpha", ModelProtocol::Responses)], 2);
    let overridden: ModelConnectionSnapshot = serde_json::from_value(execute(&mut store,
        ModelCommand::SetConnectionCapabilityOverride {
            connection_id: "one".into(), expected_revision: catalog.revision,
            model_id: "alpha".into(), protocol: ModelProtocol::Responses,
            capabilities: Some(ModelExecutionCapabilities { tool_calls: false, ..Default::default() }),
            updated_at_ms: 3,
        }).unwrap()).unwrap();
    let absent = refresh(&mut store, "one", overridden.revision, vec![], 4);
    assert_eq!(absent.document.capability_assessments, overridden.document.capability_assessments);
    let returned = refresh(&mut store, "one", absent.revision,
        vec![model("alpha", ModelProtocol::Responses)], 5);
    select(&mut store, "one", returned.revision, &["responses:alpha"], Some("responses:alpha"));
    let profile = store.list_model_profiles().unwrap().pop().unwrap();
    let source: serde_json::Value = serde_json::from_str(&profile.source_json).unwrap();
    assert_eq!(source["executionCapabilityBasis"], "user_override");
    assert_eq!(source["executionCapabilities"]["toolCalls"], false);
}

#[test]
fn catalog_refresh_never_changes_user_selection_or_removes_manual_models() {
    let mut store = NolaStore::open(":memory:").unwrap();
    let saved = save(&mut store, connection("one"), None);
    let catalog = refresh(
        &mut store,
        "one",
        saved.revision,
        vec![
            model("alpha", ModelProtocol::Responses),
            model("beta", ModelProtocol::Responses),
        ],
        2,
    );
    assert!(store.list_model_profiles().unwrap().is_empty());
    let selected = select(
        &mut store,
        "one",
        catalog.revision,
        &["responses:alpha", "responses:beta"],
        Some("responses:alpha"),
    );
    let disabled = select(
        &mut store,
        "one",
        selected.revision,
        &["responses:alpha"],
        Some("responses:alpha"),
    );
    let before = store.list_model_profiles().unwrap();
    let refreshed = refresh(
        &mut store,
        "one",
        disabled.revision,
        vec![
            model("beta", ModelProtocol::Responses),
            model("new", ModelProtocol::Responses),
        ],
        20,
    );
    assert_eq!(before, store.list_model_profiles().unwrap());
    assert!(before
        .iter()
        .any(|p| p.model_name == "beta" && !p.enabled && !p.selected));
    // A missing provider listing does not revoke a manual/previously confirmed model.
    select(
        &mut store,
        "one",
        refreshed.revision,
        &["responses:alpha"],
        Some("responses:alpha"),
    );
    assert!(store
        .list_model_profiles()
        .unwrap()
        .iter()
        .any(|p| p.model_name == "alpha" && p.enabled));
}

#[test]
fn same_model_on_distinct_protocols_has_stable_independent_runtime_identity() {
    let mut store = NolaStore::open(":memory:").unwrap();
    let saved = save(&mut store, connection("one"), None);
    let catalog = refresh(
        &mut store,
        "one",
        saved.revision,
        vec![
            model("same", ModelProtocol::Responses),
            model("same", ModelProtocol::ChatCompletions),
        ],
        2,
    );
    let first = select(
        &mut store,
        "one",
        catalog.revision,
        &["responses:same", "chat_completions:same"],
        Some("responses:same"),
    );
    let profiles = store.list_model_profiles().unwrap();
    assert_eq!(profiles.len(), 2);
    assert_ne!(profiles[0].id, profiles[1].id);
    let selected = select(
        &mut store,
        "one",
        first.revision,
        &["responses:same", "chat_completions:same"],
        Some("chat_completions:same"),
    );
    let switched = store.list_model_profiles().unwrap();
    assert_eq!(switched.iter().filter(|p| p.selected).count(), 1);
    assert!(switched
        .iter()
        .filter(|p| p.selected)
        .all(
            |p| serde_json::from_str::<Value>(&p.source_json).unwrap()["wireProtocol"]
                == "chat_completions"
        ));
    let mut renamed = connection("one");
    renamed.display_name = "Renamed".into();
    save(&mut store, renamed, Some(selected.revision));
    let after = store.list_model_profiles().unwrap();
    assert!(profiles.iter().all(|p| after.iter().any(|a| a.id == p.id)));
}

#[test]
fn route_changes_invalidate_capabilities_and_stale_discovery_is_rejected() {
    let mut store = NolaStore::open(":memory:").unwrap();
    let saved = save(&mut store, connection("one"), None);
    let catalog = refresh(
        &mut store,
        "one",
        saved.revision,
        vec![model("alpha", ModelProtocol::Responses)],
        2,
    );
    let selected = select(
        &mut store,
        "one",
        catalog.revision,
        &["responses:alpha"],
        Some("responses:alpha"),
    );
    let mut changed = connection("one");
    changed.base_url = "https://other.invalid/v1".into();
    let changed = save(&mut store, changed, Some(selected.revision));
    assert!(changed.document.catalog.is_none());
    let source: Value =
        serde_json::from_str(&store.list_model_profiles().unwrap()[0].source_json).unwrap();
    assert!(source["executionCapabilities"].is_null());
    assert!(execute(
        &mut store,
        ModelCommand::RefreshConnectionCatalog {
            connection_id: "one".into(),
            expected_revision: selected.revision,
            models: vec![model("alpha", ModelProtocol::Responses)],
            observed_at_ms: 30,
        }
    )
    .unwrap_err()
    .contains("revision_conflict"));
    execute(
        &mut store,
        ModelCommand::ConfigureConnectionModels {
            connection_id: "one".into(),
            expected_revision: changed.revision,
            enabled_model_ids: vec!["responses:alpha".into()],
            selected_model_id: Some("responses:alpha".into()),
            updated_at_ms: 31,
        },
    )
    .unwrap();
    let source: Value =
        serde_json::from_str(&store.list_model_profiles().unwrap()[0].source_json).unwrap();
    assert_eq!(source["executionCapabilityBasis"], "protocol_agent_baseline");
    assert_eq!(source["schemaVersion"], 2);
    assert_eq!(source["executionCapabilities"]["toolCalls"], true);
}

#[test]
fn account_and_connection_identity_are_separate_and_secret_fields_are_rejected() {
    let mut store = NolaStore::open(":memory:").unwrap();
    let account = ModelAccount {
        id: "account".into(),
        display_name: "Subscription".into(),
        source: ModelSourceKind::Codex,
        credential_ref: Some("vault-handle".into()),
        state: ModelAccountState::Ready,
    };
    execute(
        &mut store,
        ModelCommand::SaveAccount {
            input: account.clone(),
            expected_revision: None,
            updated_at_ms: 1,
        },
    )
    .unwrap();
    let mut input = connection("one");
    input.account_id = Some(account.id.clone());
    save(&mut store, input, None);
    let mut malicious = serde_json::to_value(&account).unwrap();
    malicious["api_key"] = serde_json::json!("not-a-real-secret");
    assert!(serde_json::from_value::<ModelAccount>(malicious).is_err());
    let mut altered = account.clone();
    altered.source = ModelSourceKind::KimiWork;
    assert!(execute(
        &mut store,
        ModelCommand::SaveAccount {
            input: altered,
            expected_revision: Some(1),
            updated_at_ms: 2
        }
    )
    .is_err());
    assert_eq!(
        store.model_account("account").unwrap().unwrap().account,
        account
    );
}

#[test]
fn duplicate_sources_names_and_unowned_writes_are_rejected_without_partial_updates() {
    let mut store = NolaStore::open(":memory:").unwrap();
    let mut input = connection("one");
    input.source = ModelSourceKind::OpenCode;
    input.source_id = Some("provider-entry".into());
    let saved = save(&mut store, input.clone(), None);
    input.id = "two".into();
    input.display_name = "Different".into();
    assert!(execute(
        &mut store,
        ModelCommand::SaveConnection {
            input,
            expected_revision: None,
            updated_at_ms: 2
        }
    )
    .unwrap_err()
    .contains("source_already_imported"));
    let catalog = refresh(
        &mut store,
        "one",
        saved.revision,
        vec![model("alpha", ModelProtocol::Responses)],
        3,
    );
    select(
        &mut store,
        "one",
        catalog.revision,
        &["responses:alpha"],
        Some("responses:alpha"),
    );
    let profile = store.list_model_profiles().unwrap().remove(0);
    assert!(execute(
        &mut store,
        ModelCommand::DeleteProfile {
            id: profile.id.clone(),
            deleted_at_ms: 4
        }
    )
    .is_err());
    assert!(execute_application_command(
        &mut store,
        ApplicationCommand::PutProductSetting {
            key: "model.connection.one".into(),
            value_json: "{}".into(),
            updated_at_ms: 4,
        }
    )
    .is_err());
    assert_eq!(store.list_model_profiles().unwrap(), vec![profile]);
    assert_eq!(store.list_model_connections().unwrap().len(), 1);
}

#[test]
fn unknown_catalog_capability_remains_unknown_while_text_execution_is_available() {
    let mut store = NolaStore::open(":memory:").unwrap();
    let saved = save(&mut store, connection("one"), None);
    let mut unknown = model("unknown", ModelProtocol::Responses);
    unknown.execution_capabilities = None;
    let mut effort = model("effort", ModelProtocol::Responses);
    effort.execution_capabilities = None;
    effort.reasoning_levels = vec!["low".into(), "high".into()];
    let catalog = refresh(
        &mut store,
        "one",
        saved.revision,
        vec![model("known", ModelProtocol::Responses), unknown, effort],
        2,
    );
    execute(
        &mut store,
        ModelCommand::ConfigureConnectionModels {
            connection_id: "one".into(),
            expected_revision: catalog.revision,
            enabled_model_ids: vec!["responses:known".into(), "responses:unknown".into(), "responses:effort".into()],
            selected_model_id: Some("responses:known".into()),
            updated_at_ms: 3,
        },
    )
    .unwrap();
    let profiles = store.list_model_profiles().unwrap();
    assert_eq!(profiles.len(), 3);
    let source: Value = serde_json::from_str(
        &profiles
            .iter()
            .find(|p| p.model_name == "unknown")
            .unwrap()
            .source_json,
    )
    .unwrap();
    assert_eq!(source["executionCapabilityBasis"], "protocol_agent_baseline");
    assert_eq!(source["executionCapabilities"]["toolCalls"], true);
    assert!(store
        .model_connection("one")
        .unwrap()
        .unwrap()
        .document
        .catalog
        .unwrap()
        .models
        .iter()
        .find(|m| m.model_id == "unknown")
        .unwrap()
        .execution_capabilities
        .is_none());
    let current = store.model_connection("one").unwrap().unwrap();
    execute(&mut store, ModelCommand::SetConnectionCapabilityOverride {
        connection_id: "one".into(), expected_revision: current.revision,
        model_id: "effort".into(), protocol: ModelProtocol::Responses,
        capabilities: None, updated_at_ms: 4,
    }).unwrap();
    let profiles = store.list_model_profiles().unwrap();
    let source: Value = serde_json::from_str(&profiles.iter()
        .find(|profile| profile.model_name == "effort").unwrap().source_json).unwrap();
    assert_eq!(source["executionCapabilities"]["reasoning"], true);
    assert_eq!(source["executionCapabilityBasis"], "catalog_declaration");
    let current = store.model_connection("one").unwrap().unwrap();
    let mut entries = current.document.catalog.unwrap().models;
    entries.iter_mut().find(|entry| entry.model_id == "effort").unwrap()
        .reasoning_levels = vec!["low".into()];
    let changed = refresh(&mut store, "one", current.revision, entries, 5);
    select(&mut store, "one", changed.revision,
        &["responses:known", "responses:unknown", "responses:effort"], Some("responses:known"));
    let profiles = store.list_model_profiles().unwrap();
    let source: Value = serde_json::from_str(&profiles.iter()
        .find(|profile| profile.model_name == "effort").unwrap().source_json).unwrap();
    assert_eq!(source["reasoning"]["efforts"], serde_json::json!(["low"]));
}

#[test]
fn durable_connection_write_failure_rolls_back_created_runtime_profiles() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("rollback.sqlite");
    let mut store = NolaStore::open(&path).unwrap();
    let saved = save(&mut store, connection("one"), None);
    let catalog = refresh(
        &mut store,
        "one",
        saved.revision,
        vec![model("alpha", ModelProtocol::Responses)],
        2,
    );
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection
        .execute_batch(
            "CREATE TRIGGER reject_connection BEFORE UPDATE ON product_settings
        WHEN NEW.key='model.connection.one' BEGIN SELECT RAISE(ABORT,'fixture failure'); END;",
        )
        .unwrap();
    assert!(execute(
        &mut store,
        ModelCommand::ConfigureConnectionModels {
            connection_id: "one".into(),
            expected_revision: catalog.revision,
            enabled_model_ids: vec!["responses:alpha".into()],
            selected_model_id: Some("responses:alpha".into()),
            updated_at_ms: 3,
        }
    )
    .is_err());
    assert!(store.list_model_profiles().unwrap().is_empty());
    assert_eq!(store.model_connection("one").unwrap().unwrap(), catalog);
}

#[test]
fn unknown_stored_format_is_rejected_without_rewriting_data() {
    let mut store = NolaStore::open(":memory:").unwrap();
    let saved = save(&mut store, connection("one"), None);
    let mut corrupt = serde_json::to_value(saved.document).unwrap();
    corrupt["format_version"] = serde_json::json!(9);
    let raw = corrupt.to_string();
    let setting = store
        .put_product_setting("model.connection.one", &raw, 2)
        .unwrap();
    assert!(execute(
        &mut store,
        ModelCommand::SaveConnection {
            input: connection("one"),
            expected_revision: Some(setting.revision),
            updated_at_ms: 3,
        }
    )
    .is_err());
    assert_eq!(
        store
            .product_setting("model.connection.one")
            .unwrap()
            .unwrap()
            .value_json,
        raw
    );
}

#[test]
fn stale_profile_revision_cannot_overwrite_parameters_in_connection_transaction() {
    let mut store = NolaStore::open(":memory:").unwrap();
    let saved = save(&mut store, connection("one"), None);
    let catalog = refresh(
        &mut store,
        "one",
        saved.revision,
        vec![model("alpha", ModelProtocol::Responses)],
        2,
    );
    let selected = select(
        &mut store,
        "one",
        catalog.revision,
        &["responses:alpha"],
        Some("responses:alpha"),
    );
    let before = store.list_model_profiles().unwrap();
    let revisions = before.iter().map(|p| (p.id.clone(), p.revision)).collect();
    let mut changed = before[0].clone();
    changed.parameters_json = "{\"maxOutputTokens\":1000}".into();
    store.upsert_model_profile(&changed).unwrap();
    assert!(store
        .commit_model_connection(
            &selected.document,
            Some(selected.revision),
            &before,
            &revisions,
            5
        )
        .is_err());
    assert_eq!(
        store.list_model_profiles().unwrap()[0].parameters_json,
        changed.parameters_json
    );
    assert_eq!(store.model_connection("one").unwrap().unwrap(), selected);
}

#[test]
fn imported_tools_default_on_then_provider_false_overrides_on_configuration() {
    let mut store = NolaStore::open(":memory:").unwrap();
    let saved = save(&mut store, connection("tools-default"), None);
    let mut entry = model("tool-model", ModelProtocol::Responses);
    entry.execution_capabilities = None;
    let catalog = refresh(&mut store, "tools-default", saved.revision, vec![entry.clone()], 2);
    let configure = |revision| ModelCommand::ConfigureConnectionModels {
        connection_id: "tools-default".into(), expected_revision: revision,
        enabled_model_ids: vec!["responses:tool-model".into()], selected_model_id: Some("responses:tool-model".into()), updated_at_ms: 3,
    };
    execute(&mut store, configure(catalog.revision)).unwrap();
    let source: Value = serde_json::from_str(&store.list_model_profiles().unwrap()[0].source_json).unwrap();
    assert_eq!(source["executionCapabilities"]["toolCalls"], true);
    entry.execution_capabilities = Some(ModelExecutionCapabilities::for_import(Some(false)));
    let revision = store.model_connection("tools-default").unwrap().unwrap().revision;
    let catalog = refresh(&mut store, "tools-default", revision, vec![entry], 4);
    execute(&mut store, configure(catalog.revision)).unwrap();
    let source: Value = serde_json::from_str(&store.list_model_profiles().unwrap()[0].source_json).unwrap();
    assert_eq!(source["executionCapabilities"]["toolCalls"], false);
    assert_eq!(source["executionCapabilityBasis"], "catalog_declaration");
}
