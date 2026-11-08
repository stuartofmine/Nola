// SPDX-License-Identifier: Apache-2.0
use super::*;

fn configured_store(root: &std::path::Path) -> NolaStore {
    let store = NolaStore::open(&root.join("canonical.sqlite")).unwrap();
    store.upsert_model_profile(nola_application::ModelProfile {
        id:"adapter-model".into(),display_name:"Fixture".into(),model_name:"upstream-model".into(),provider:"openai-compatible".into(),runtime:"remote".into(),
        source_json:json!({"schemaVersion":1,"endpointID":"fixture-endpoint","baseURL":"https://fixture.invalid/v1","wireProtocol":"responses","executionCapabilities":{"toolCalls":false,"imageInput":false,"reasoning":false,"samplingParameters":false}}).to_string(),
        parameters_json:json!({"maxContextLength":16000}).to_string(),selected:true,enabled:true,revision:1,created_at_ms:1,updated_at_ms:1,deleted_at_ms:None,
    }).unwrap();
    store
}

#[test]
fn client_export_cannot_turn_owned_siwc_into_a_gateway_key() {
    let root = tempfile::tempdir().unwrap(); let store = configured_store(root.path());
    let mut profile = store.model_profile("adapter-model").unwrap().unwrap();
    let mut source: Value = serde_json::from_str(&profile.source_json).unwrap();
    source["wireProtocol"] = json!("siwc_responses"); source["sourceKind"] = json!("nola_chatgpt");
    profile.source_json = source.to_string(); store.upsert_model_profile(profile).unwrap();
    let profile = store.model_profile("adapter-model").unwrap().unwrap();
    let path = root.path().join("opencode.json"); std::fs::write(&path,"{}").unwrap();
    let result = plan_client_connection(&store, ClientConnectionRequest {source:ModelSourceKind::OpenCode,
        path:path.to_str().unwrap().into(),model_id:crate::model_gateway_host::model_id(&profile)},"http://127.0.0.1:8765/gateway/v1");
    assert!(matches!(result,Err(error) if error == "siwc_authorization_cannot_be_exported_as_gateway_api"));
    assert_eq!(std::fs::read_to_string(path).unwrap(),"{}");
}

#[test]
fn confirmed_connection_receipt_survives_restart_and_disconnect_preserves_user_edits() {
    let root = tempfile::tempdir().unwrap();
    let store = configured_store(root.path());
    let path = root.path().join("opencode.jsonc");
    std::fs::write(&path, "{\n// retain comment\n\"theme\":\"dark\"\n}").unwrap();
    let access = crate::model_gateway_host::issue_access_token(&store, 0).unwrap();
    let plan = plan_client_connection(
        &store,
        ClientConnectionRequest {
            source: ModelSourceKind::OpenCode,
            path: path.to_str().unwrap().into(),
            model_id: "nola/adapter-model/1".into(),
        },
        "http://127.0.0.1:8765/gateway/v1",
    )
    .unwrap();
    assert!(!serde_json::to_string(&plan)
        .unwrap()
        .contains(&access.token));
    let receipt = commit_client_connection(&store, &plan.plan_id).unwrap();
    assert!(!serde_json::to_string(&receipt)
        .unwrap()
        .contains(&access.token));
    assert!(commit_client_connection(&store, &plan.plan_id).is_err());
    let inserted = std::fs::read_to_string(&path).unwrap();
    assert!(inserted.contains(&access.token));
    std::fs::write(&path, inserted.replace("dark", "light")).unwrap();
    drop(store);
    let reopened = NolaStore::open(&root.path().join("canonical.sqlite")).unwrap();
    assert_eq!(
        list_client_connections(&reopened).unwrap()[0].receipt_id,
        receipt.receipt_id
    );
    disconnect_client_connection(&reopened, &receipt.receipt_id).unwrap();
    let final_config = std::fs::read_to_string(&path).unwrap();
    assert!(final_config.contains("retain comment"));
    assert!(final_config.contains("light"));
    assert!(!final_config.contains(&access.token));
    assert!(list_client_connections(&reopened).unwrap().is_empty());
}

#[test]
fn token_rotation_between_plan_and_commit_does_not_write_client_file() {
    let root = tempfile::tempdir().unwrap();
    let store = configured_store(root.path());
    let path = root.path().join("codex.toml");
    std::fs::write(&path, "# untouched\n").unwrap();
    let access = crate::model_gateway_host::issue_access_token(&store, 0).unwrap();
    let plan = plan_client_connection(
        &store,
        ClientConnectionRequest {
            source: ModelSourceKind::Codex,
            path: path.to_str().unwrap().into(),
            model_id: "nola/adapter-model/1".into(),
        },
        "http://127.0.0.1:8765/gateway/v1",
    )
    .unwrap();
    crate::model_gateway_host::revoke_access_token(&store, access.revision).unwrap();
    assert!(commit_client_connection(&store, &plan.plan_id).is_err());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "# untouched\n");
    assert!(list_client_connections(&store).unwrap().is_empty());
}

#[test]
fn jsonc_owned_fields_round_trip_preserves_comments_and_unrelated_edits() {
    let original = b"{\n // keep this comment\n \"theme\": \"dark\",\n \"provider\": {\"other\": {\"npm\":\"custom\"}},\n}\n";
    let owned = fields(
        ModelSourceKind::OpenCode,
        "responses",
        "nola/profile/1",
        "http://127.0.0.1:8765/gateway/v1",
        "fixture-gateway-secret",
    )
    .unwrap();
    let inserted = edit(ModelSourceKind::OpenCode, original, &owned, false).unwrap();
    assert!(inserted.contains("keep this comment"));
    assert!(inserted.contains("fixture-gateway-secret"));
    assert!(edit(
        ModelSourceKind::OpenCode,
        inserted.as_bytes(),
        &owned,
        false
    )
    .is_err());
    let concurrent = inserted.replace("dark", "light");
    let removed = edit(
        ModelSourceKind::OpenCode,
        concurrent.as_bytes(),
        &owned,
        true,
    )
    .unwrap();
    assert!(removed.contains("keep this comment"));
    assert!(removed.contains("light"));
    assert!(removed.contains("custom"));
    assert!(!removed.contains("fixture-gateway-secret"));
    assert!(!removed.contains("nola-responses"));
}

#[test]
fn toml_adapters_preserve_defaults_and_refuse_owned_field_changes() {
    for source in [ModelSourceKind::Codex, ModelSourceKind::KimiCode] {
        let original = b"# keep this comment\nmodel = 'existing-model'\n[other]\nvalue = 7\n";
        let owned = fields(
            source,
            "responses",
            "nola/profile/1",
            "http://127.0.0.1:8765/gateway/v1",
            "fixture-gateway-secret",
        )
        .unwrap();
        let inserted = edit(source, original, &owned, false).unwrap();
        assert!(inserted.contains("# keep this comment"));
        assert!(inserted.contains("model = 'existing-model'"));
        let modified = inserted.replace("fixture-gateway-secret", "user-modified-secret");
        assert_eq!(
            edit(source, modified.as_bytes(), &owned, true).unwrap_err(),
            "client_owned_field_changed"
        );
        let removed = edit(source, inserted.as_bytes(), &owned, true).unwrap();
        assert!(removed.contains("value = 7"));
        assert!(!removed.contains("fixture-gateway-secret"));
    }
    assert!(fields(ModelSourceKind::Codex, "anthropic", "model", "url", "token").is_err());
    assert!(fields(
        ModelSourceKind::KimiWork,
        "responses",
        "model",
        "url",
        "token"
    )
    .is_err());
}

#[test]
fn configuration_replace_detects_changed_file_without_overwriting_it() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("fixture.jsonc");
    std::fs::write(&path, b"{\"user\":true}").unwrap();
    assert_eq!(
        replace_selected(path.to_str().unwrap(), b"{}", "{}").unwrap_err(),
        "client_config_changed"
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"{\"user\":true}");
    replace_selected(
        path.to_str().unwrap(),
        b"{\"user\":true}",
        "{\"user\":false}",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o077,
            0
        );
    }
}
