// SPDX-License-Identifier: Apache-2.0
//! A catalog replacement retains disabled members without selecting them.

use nola_store::{NolaStore, StoredModelProfile};
use std::collections::BTreeMap;

fn profile(id: &str, selected: bool, enabled: bool) -> StoredModelProfile {
    StoredModelProfile {
        id: id.into(),
        display_name: id.into(),
        provider: "openai-compatible".into(),
        model_name: id.into(),
        runtime: "remote".into(),
        source_json: r#"{"schemaVersion":2,"endpointID":"fixture","baseURL":"https://example.invalid/v1"}"#.into(),
        parameters_json: "{}".into(),
        selected,
        enabled,
        revision: 1,
        created_at_ms: 1,
        updated_at_ms: 1,
        deleted_at_ms: None,
    }
}

#[test]
fn disabled_models_survive_scoped_catalog_replacement() {
    let mut store = NolaStore::open(":memory:").unwrap();
    let selected = profile("selected", true, true);
    let disabled = profile("disabled", false, false);
    store.replace_model_profiles("openai-compatible", "remote", &[selected.clone(), disabled.clone()], 2, &BTreeMap::new()).unwrap();

    let current = store.list_model_profiles().unwrap();
    assert_eq!(current.len(), 2);
    assert!(current.iter().any(|model| model.id == "selected" && model.selected && model.enabled));
    assert!(current.iter().any(|model| model.id == "disabled" && !model.selected && !model.enabled));

    let revisions = current.iter().map(|model| (model.id.clone(), model.revision)).collect();
    assert!(store.replace_model_profiles("openai-compatible", "remote", &[selected, profile("disabled", true, false)], 3, &revisions).is_err());
    assert!(store.list_model_profiles().unwrap().iter().any(|model| model.id == "disabled" && !model.selected && !model.enabled));
}

#[test]
fn catalog_with_only_disabled_models_has_no_selection() {
    let mut store = NolaStore::open(":memory:").unwrap();
    store.replace_model_profiles("openai-compatible", "remote", &[profile("disabled", false, false)], 2, &BTreeMap::new()).unwrap();
    let models = store.list_model_profiles().unwrap();
    assert_eq!(models.len(), 1);
    assert!(!models[0].selected);
    assert!(!models[0].enabled);
}

#[test]
fn selection_displacement_is_scoped_to_provider_and_runtime() {
    let mut store = NolaStore::open(":memory:").unwrap();
    let mut local = profile("local", true, true);
    local.provider = "mlx".into();
    local.runtime = "local".into();
    store.upsert_model_profile(&local).unwrap();
    let default_json = r#"{"version":1,"selection":{"kind":"profile","profile_id":"local"}}"#;
    let default_before = store.put_product_setting("chat.default_model", default_json, 1).unwrap();
    let local_before = store.list_model_profiles().unwrap().into_iter().find(|p| p.id == "local").unwrap();
    store.upsert_model_profile(&profile("remote-a", true, true)).unwrap();
    store.upsert_model_profile(&profile("remote-b", true, true)).unwrap();
    let profiles = store.list_model_profiles().unwrap();
    let local_after = profiles.iter().find(|p| p.id == "local").unwrap();
    assert!(local_after.selected);
    assert_eq!(local_before.revision, local_after.revision);
    assert_eq!(local_before.updated_at_ms, local_after.updated_at_ms);
    assert!(!profiles.iter().find(|p| p.id == "remote-a").unwrap().selected);
    let remote_before = profiles.iter().find(|p| p.id == "remote-b").unwrap();
    let mut another_local = local.clone();
    another_local.id = "local-next".into();
    store.upsert_model_profile(&another_local).unwrap();
    let profiles = store.list_model_profiles().unwrap();
    let remote_after = profiles.iter().find(|p| p.id == "remote-b").unwrap();
    assert!(remote_after.selected);
    assert_eq!(remote_before.revision, remote_after.revision);
    assert_eq!(remote_before.updated_at_ms, remote_after.updated_at_ms);
    let default_after = store.product_setting("chat.default_model").unwrap().unwrap();
    assert_eq!(default_before.revision, default_after.revision);
    assert_eq!(default_after.value_json, default_json);
    store.delete_model_profile("local", 10).unwrap();
    assert_eq!(store.product_setting("chat.default_model").unwrap().unwrap().value_json, default_json);
}

#[test]
fn incompatible_schema_does_not_rewrite_model_profiles_or_selection_index() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("selection-migration.db");
    let mut store = NolaStore::open(&path).unwrap();
    store.upsert_model_profile(&profile("remote", true, true)).unwrap();
    drop(store);
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection.execute_batch("DROP INDEX idx_model_profiles_selected;
        CREATE UNIQUE INDEX idx_model_profiles_selected ON model_profiles(selected)
        WHERE selected=1 AND enabled=1 AND deleted_at_ms IS NULL;
        PRAGMA user_version=60;").unwrap();
    connection.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)").unwrap();
    drop(connection);
    let before = std::fs::read(&path).unwrap();
    assert!(matches!(NolaStore::open(&path), Err(nola_store::StoreError::IncompatibleSchema)));
    assert_eq!(std::fs::read(&path).unwrap(),before);
}
