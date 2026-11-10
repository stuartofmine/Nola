// SPDX-License-Identifier: Apache-2.0
use super::*;

fn request(source: ModelSourceKind) -> ImportPlanRequest {
    ImportPlanRequest {
        source,
        path: "/explicit/fixture/config".into(),
        credential_path: None,
        include_credentials: true,
        allow_environment: false,
    }
}

#[test]
fn opencode_uses_sdk_protocol_and_never_serializes_credentials() {
    let request = request(ModelSourceKind::OpenCode);
    let document = json!({"provider":{
        "custom":{"npm":"@ai-sdk/openai","name":"Custom","options":{"baseURL":"https://fixture.invalid/v1","apiKey":"isolated-fixture-secret"},"models":{"model-one":{"name":"One","tool_call":true,"limit":{"context":128000,"output":8000}}}},
        "unknown":{"options":{"baseURL":"https://unknown.invalid/v1","apiKey":"not-an-inferred-key"},"models":{"model-two":{}}}
    }});
    let candidates = parse_candidates(&request, &document, &Value::Null, &[]).unwrap();
    let valid = candidates
        .iter()
        .find(|c| c.public.connection.display_name == "Custom")
        .unwrap();
    assert_eq!(
        valid.public.connection.protocols,
        vec![ModelProtocol::Responses]
    );
    assert_eq!(valid.secret.as_deref(), Some("isolated-fixture-secret"));
    assert_eq!(valid.public.models[0].context_window, Some(128000));
    assert!(
        valid.public.models[0]
            .execution_capabilities
            .unwrap()
            .tool_calls
    );
    assert!(!serde_json::to_string(&valid.public)
        .unwrap()
        .contains("isolated-fixture-secret"));
    assert!(candidates
        .iter()
        .find(|c| c.public.connection.display_name == "unknown")
        .unwrap()
        .public
        .issue
        .is_some());
}

#[test]
fn codex_import_collects_profiles_by_provider_without_following_catalog_paths() {
    let document = json!({"model_provider":"one", "model":"current", "model_providers":{
        "one":{"base_url":"https://one.invalid/v1"},
        "two":{"base_url":"https://two.invalid/v1"}},
        "profiles":{
            "inherited":{"model":"extra"},
            "duplicate":{"model":"current"},
            "other":{"model_provider":"two","model":"separate"},
            "path":{"model_catalog_json":"/must/not/read"}
        }});
    let candidates = parse_candidates(&request(ModelSourceKind::Codex), &document, &Value::Null, &[]).unwrap();
    let one = candidates.iter().find(|c| c.public.connection.base_url.contains("one.invalid")).unwrap();
    let ids: BTreeSet<_> = one.public.models.iter().map(|m| m.model_id.as_str()).collect();
    assert_eq!(ids, BTreeSet::from(["current", "extra"]));
    assert_eq!(one.public.models.len(), 2);
    let two = candidates.iter().find(|c| c.public.connection.base_url.contains("two.invalid")).unwrap();
    assert_eq!(two.public.models.len(), 1);
    assert_eq!(two.public.models[0].model_id, "separate");
}

#[test]
fn codex_responses_default_is_source_contract_and_oauth_is_not_copied() {
    let request = request(ModelSourceKind::Codex);
    let document = parse_document(
        ModelSourceKind::Codex,
        br#"
model = "fixture-model"
model_provider = "local"
[model_providers.local]
name = "Local"
base_url = "https://fixture.invalid/v1"
env_key = "DO_NOT_READ_UNAUTHORIZED_ENV"
"#,
    )
    .unwrap();
    let candidates = parse_candidates(&request, &document, &Value::Null, &[]).unwrap();
    assert_eq!(
        candidates[0].public.connection.protocols,
        vec![ModelProtocol::Responses]
    );
    assert!(candidates[0].secret.is_none());
    assert_eq!(candidates[0].public.models[0].model_id, "fixture-model");
    let oauth = parse_candidates(
        &request,
        &json!({"model":"fixture"}),
        &json!({"tokens":{"access_token":"private-oauth"}}),
        &[],
    )
    .unwrap();
    assert_eq!(
        oauth[0].public.issue.as_deref(),
        Some("subscription_credential_reuse_not_supported")
    );
    assert!(oauth[0].secret.is_none());
    let stale_api = parse_candidates(&request, &json!({"model":"fixture"}),
        &json!({"auth_mode":"chatgpt","OPENAI_API_KEY":"stale-api-key","tokens":{"access_token":"private-oauth"}}), &[]).unwrap();
    assert!(stale_api[0].secret.is_none());
    assert_eq!(stale_api[0].public.issue.as_deref(), Some("subscription_credential_reuse_not_supported"));
    let api_mode = parse_candidates(&request, &json!({"model":"fixture"}),
        &json!({"auth_mode":"apikey","OPENAI_API_KEY":"selected-api-key","tokens":{"access_token":"stale-oauth"}}), &[]).unwrap();
    assert_eq!(api_mode[0].secret.as_deref(), Some("selected-api-key"));
    assert!(api_mode[0].public.issue.is_none());
    let missing = parse_candidates(&request, &json!({"model":"fixture"}), &json!({"tokens":null}), &[]).unwrap();
    assert_eq!(missing[0].public.issue.as_deref(), Some("api_credential_required"));

    assert!(!serde_json::to_string(&oauth[0].public)
        .unwrap()
        .contains("private-oauth"));
}

#[test]
fn kimi_code_is_distinct_from_desktop_and_work_and_preserves_explicit_models() {
    let request = request(ModelSourceKind::KimiCode);
    let document = parse_document(
        ModelSourceKind::KimiCode,
        br#"
[providers.fixture]
type = "openai_responses"
base_url = "https://fixture.invalid/v1"
api_key = "isolated-kimi-fixture"
[models.alias]
provider = "fixture"
model = "actual-model"
max_context_size = 256000
"#,
    )
    .unwrap();
    let candidates = parse_candidates(&request, &document, &Value::Null, &[]).unwrap();
    assert_eq!(candidates[0].public.models[0].model_id, "actual-model");
    assert_eq!(candidates[0].public.models[0].context_window, Some(256000));
    assert_eq!(
        candidates[0].secret.as_deref(),
        Some("isolated-kimi-fixture")
    );
    for source in [ModelSourceKind::KimiDesktop, ModelSourceKind::KimiWork] {
        assert!(parse_candidates(
            &ImportPlanRequest {
                source,
                ..request.clone()
            },
            &document,
            &Value::Null,
            &[]
        )
        .is_err());
    }
}

#[test]
fn jsonc_strings_comments_and_trailing_commas_are_parsed_without_evaluation() {
    let document = parse_jsonc(
        br#"{
        // comment
        "url":"https://fixture.invalid/a/*literal*/", /* comment */
        "values":["escaped\\\"",],
    }"#,
    )
    .unwrap();
    assert_eq!(document["url"], "https://fixture.invalid/a/*literal*/");
    assert_eq!(document["values"].as_array().unwrap().len(), 1);
    assert!(parse_jsonc(br#"{"x":1/*unterminated}"#).is_err());
    assert!(literal_secret(
        Some("{file:/not-authorized}"),
        &request(ModelSourceKind::OpenCode)
    )
    .is_none());
}

#[test]
fn preview_redacts_invalid_urls_and_rejects_gateway_aliases() {
    let request = request(ModelSourceKind::OpenCode);
    let document = json!({"provider":{
        "leak":{"npm":"@ai-sdk/openai-compatible","options":{"baseURL":"https://fixture.invalid/v1?api_key=private-query"}},
        "loop":{"npm":"@ai-sdk/openai-compatible","options":{"baseURL":"http://localhost:8080/v1"}}
    }});
    let candidates = parse_candidates(
        &request,
        &document,
        &Value::Null,
        &["http://127.0.0.1:8080".into()],
    )
    .unwrap();
    let public: Vec<_> = candidates.iter().map(|c| &c.public).collect();
    assert!(!serde_json::to_string(&public)
        .unwrap()
        .contains("private-query"));
    assert!(public
        .iter()
        .any(|c| c.issue.as_deref() == Some("nola_gateway_loop")));
}

#[test]
fn source_change_after_preview_never_writes_connection_or_account() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("opencode.json");
    std::fs::write(&config,br#"{"provider":{"fixture":{"npm":"@ai-sdk/openai-compatible","options":{"baseURL":"https://fixture.invalid/v1"},"models":{"model":{}}}}}"#).unwrap();
    let store = NolaStore::open(&root.path().join("canonical.sqlite")).unwrap();
    let plan = plan_import(
        &store,
        ImportPlanRequest {
            path: config.to_string_lossy().into_owned(),
            ..request(ModelSourceKind::OpenCode)
        },
        &[],
    )
    .unwrap();
    std::fs::write(&config, b"{}").unwrap();
    let result = commit_import(
        &store,
        ImportCommitRequest {
            plan_id: plan.plan_id,
            selections: vec![ImportSelection {
                candidate_id: plan.candidates[0].candidate_id.clone(),
                display_name: None,
                update_existing: false,
                enable_models: true,
            }],
        },
        &[],
    );
    assert!(result.unwrap_err().contains("source_changed"));
    let connections: Vec<ModelConnectionSnapshot> =
        execute(&store, ModelCommand::ListConnections).unwrap();
    let accounts: Vec<ModelAccountSnapshot> = execute(&store, ModelCommand::ListAccounts).unwrap();
    assert!(connections.is_empty() && accounts.is_empty());
}

#[test]
fn reimport_preserves_selection_and_enabled_models_missing_from_source() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("opencode.json");
    let write_source = |models: Value| std::fs::write(&path, json!({"provider":{"fixture":{
        "npm":"@ai-sdk/openai-compatible","options":{"baseURL":"https://fixture.invalid/v1"},
        "models":models}}}).to_string()).unwrap();
    let store = NolaStore::open(&root.path().join("isolated.sqlite")).unwrap();
    let input = ImportPlanRequest { path: path.to_string_lossy().into_owned(), include_credentials: false,
        ..request(ModelSourceKind::OpenCode) };
    write_source(json!({"original":{}}));
    for update in [false, true] {
        if update { write_source(json!({"new":{}})); }
        let plan = plan_import(&store, input.clone(), &[]).unwrap();
        let result = commit_import(&store, ImportCommitRequest { plan_id: plan.plan_id,
            selections: vec![ImportSelection { candidate_id: plan.candidates[0].candidate_id.clone(),
                display_name: None, update_existing: update, enable_models: true }] }, &[]).unwrap();
        assert_eq!(result[0].status, "imported", "{:?}", result);
    }
    let profiles = store.model_profiles().unwrap();
    assert_eq!(profiles.len(), 2);
    assert!(profiles.iter().all(|p| p.enabled));
    assert_eq!(profiles.iter().find(|p| p.selected).unwrap().model_name, "original");
}

#[test]
fn import_commits_profiles_and_exact_route_credentials_with_partial_results() {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("opencode.json");
    std::fs::write(&config,br#"{"provider":{
        "valid":{"npm":"@ai-sdk/openai-compatible","options":{"baseURL":"https://fixture.invalid/v1","apiKey":"isolated-import-key"},"models":{"model":{}}},
        "invalid":{"npm":"unknown-sdk","options":{"baseURL":"https://another.invalid/v1"}}
    }}"#).unwrap();
    let store = NolaStore::open(&root.path().join("canonical.sqlite")).unwrap();
    let input = ImportPlanRequest {
        path: config.to_string_lossy().into_owned(),
        ..request(ModelSourceKind::OpenCode)
    };
    let plan = plan_import(&store, input.clone(), &[]).unwrap();
    let selections = plan
        .candidates
        .iter()
        .map(|c| ImportSelection {
            candidate_id: c.candidate_id.clone(),
            display_name: None,
            update_existing: false,
            enable_models: true,
        })
        .collect();
    let results = commit_import(
        &store,
        ImportCommitRequest {
            plan_id: plan.plan_id,
            selections,
        },
        &[],
    )
    .unwrap();
    assert_eq!(results.iter().filter(|r| r.status == "imported").count(), 1);
    assert_eq!(results.iter().filter(|r| r.status == "failed").count(), 1);
    let profiles = store.model_profiles().unwrap();
    assert_eq!(profiles.len(), 1);
    assert_eq!(
        crate::host_credentials::for_profile(&store, &profiles[0], None).unwrap(),
        Some("isolated-import-key".into())
    );
    let mut altered = profiles[0].clone();
    let mut source: Value = serde_json::from_str(&altered.source_json).unwrap();
    source["baseURL"] = json!("https://different.invalid/v1");
    altered.source_json = source.to_string();
    assert!(crate::host_credentials::for_profile(&store, &altered, None).is_err());
    let second = plan_import(&store, input, &[]).unwrap();
    assert!(second
        .candidates
        .iter()
        .any(|c| c.existing_revision.is_some()));
    let existing = second
        .candidates
        .iter()
        .find(|candidate| candidate.existing_revision.is_some())
        .unwrap();
    let account_id = existing.connection.account_id.clone();
    let result = commit_import(
        &store,
        ImportCommitRequest {
            plan_id: second.plan_id,
            selections: vec![ImportSelection {
                candidate_id: existing.candidate_id.clone(),
                display_name: None,
                update_existing: true,
                enable_models: false,
            }],
        },
        &[],
    )
    .unwrap();
    assert_eq!(result[0].status, "imported");
    let accounts: Vec<ModelAccountSnapshot> = execute(&store, ModelCommand::ListAccounts).unwrap();
    assert_eq!(accounts.len(), 1);
    assert_eq!(Some(accounts[0].account.id.clone()), account_id);
}

#[test]
fn discovery_is_a_bounded_get_and_keeps_model_selection_unchanged() {
    let root = tempfile::tempdir().unwrap();
    let store = NolaStore::open(&root.path().join("canonical.sqlite")).unwrap();
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let base_url = format!("http://{}/v1", server.server_addr());
    let saved: ModelConnectionSnapshot = execute(
        &store,
        ModelCommand::SaveConnection {
            input: ModelConnection {
                id: "discover".into(),
                display_name: "Discovery".into(),
                base_url,
                protocols: vec![ModelProtocol::Responses],
                account_id: None,
                source: ModelSourceKind::Manual,
                source_id: None,
                enabled: true,
            },
            expected_revision: None,
            updated_at_ms: 1,
        },
    )
    .unwrap();
    let worker = std::thread::spawn(move || {
        let request = server
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap()
            .unwrap();
        assert_eq!(request.method(), &tiny_http::Method::Get);
        assert_eq!(request.url(), "/v1/models");
        request
            .respond(tiny_http::Response::from_string(
                r#"{"data":[{"id":"discovered"}]}"#,
            ))
            .unwrap();
    });
    let refreshed = discover_catalog(&store, "discover", saved.revision, &[]).unwrap();
    worker.join().unwrap();
    assert_eq!(
        refreshed.document.catalog.unwrap().models[0].model_id,
        "discovered"
    );
    assert!(store.model_profiles().unwrap().is_empty());
}

#[test]
fn cancelled_or_stale_discovery_never_replaces_saved_catalog() {
    for cancel_request in [true, false] {
        let root = tempfile::tempdir().unwrap();
        let store = NolaStore::open(&root.path().join("canonical.sqlite")).unwrap();
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let saved: ModelConnectionSnapshot = execute(
            &store,
            ModelCommand::SaveConnection {
                input: ModelConnection {
                    id: "fixture".into(),
                    display_name: "Before".into(),
                    base_url: format!("http://{}/v1", server.server_addr()),
                    protocols: vec![ModelProtocol::Responses],
                    account_id: None,
                    source: ModelSourceKind::Manual,
                    source_id: None,
                    enabled: true,
                },
                expected_revision: None,
                updated_at_ms: 1,
            },
        )
        .unwrap();
        let request_id = uuid::Uuid::new_v4().to_string();
        let request_copy = request_id.clone();
        let path = store.path().to_path_buf();
        let (completed, result) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let store = NolaStore::open(&path).unwrap();
            completed
                .send(discover_catalog_request(
                    &store,
                    "fixture",
                    saved.revision,
                    &[],
                    &request_copy,
                ))
                .unwrap();
        });
        let upstream = server
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap()
            .unwrap();
        if cancel_request {
            assert!(crate::model_catalog_discovery::cancel(
                &store,
                &request_id,
                "fixture",
                saved.revision
            )
            .unwrap());
            // The provider has not answered. Cancellation must interrupt the
            // network wait, not merely suppress a write ten seconds later.
            assert_eq!(
                result
                    .recv_timeout(std::time::Duration::from_secs(2))
                    .unwrap()
                    .unwrap_err(),
                "model_discovery_cancelled"
            );
            drop(upstream);
        } else {
            let mut changed = saved.document.connection.clone();
            changed.display_name = "User edit".into();
            let _: ModelConnectionSnapshot = execute(
                &store,
                ModelCommand::SaveConnection {
                    input: changed,
                    expected_revision: Some(saved.revision),
                    updated_at_ms: 2,
                },
            )
            .unwrap();
            upstream
                .respond(tiny_http::Response::from_string(
                    r#"{"data":[{"id":"late-provider-result"}]}"#,
                ))
                .unwrap();
            assert!(result
                .recv_timeout(std::time::Duration::from_secs(2))
                .unwrap()
                .is_err());
        }
        worker.join().unwrap();
        let current: Option<ModelConnectionSnapshot> = execute(
            &store,
            ModelCommand::GetConnection {
                id: "fixture".into(),
            },
        )
        .unwrap();
        let current = current.unwrap();
        assert!(current.document.catalog.is_none());
        assert_eq!(
            current.document.connection.display_name,
            if cancel_request {
                "Before"
            } else {
                "User edit"
            }
        );
        assert!(store.model_profiles().unwrap().is_empty());
    }
}
#[test]
fn provider_catalog_metadata_and_unknown_tool_default_are_distinct() {
    let model = super::make_model("fixture", &serde_json::json!({
        "context_length":64000,"top_provider":{"max_completion_tokens":8000},
        "architecture":{"input_modalities":["text","image"]},
        "supported_parameters":["tools","reasoning","temperature"]
    }), ModelProtocol::Responses);
    assert_eq!(model.context_window, Some(64000));
    assert_eq!(model.output_limit, Some(8000));
    let capabilities = model.execution_capabilities.unwrap();
    assert!(capabilities.tool_calls && capabilities.image_input && capabilities.reasoning && capabilities.sampling_parameters);
    let text = super::make_model("fixture", &serde_json::json!({"supported_parameters":[]}), ModelProtocol::Responses);
    assert!(!text.execution_capabilities.unwrap().tool_calls);
    let unknown = super::make_model("fixture", &serde_json::json!({}), ModelProtocol::Responses);
    assert!(unknown.execution_capabilities.is_none());
    let partial = super::make_model("fixture", &serde_json::json!({"reasoning":true}), ModelProtocol::Responses);
    assert!(partial.execution_capabilities.unwrap().tool_calls);
}


#[test]
fn provider_explicit_tool_flag_overrides_import_default_and_parameter_list() {
    for (metadata, expected) in [
        (serde_json::json!({"tool_call":false,"supported_parameters":["tools"]}), false),
        (serde_json::json!({"tool_call":true,"supported_parameters":[]}), true),
        (serde_json::json!({"temperature":true}), true),
    ] {
        let model = super::make_model("fixture", &metadata, ModelProtocol::Responses);
        assert_eq!(model.execution_capabilities.unwrap().tool_calls, expected);
    }
}

#[test]
fn codex_builtin_without_credentials_cannot_be_imported_as_anonymous() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("config.toml");
    std::fs::write(&source, "model = \"fixture\"\n").unwrap();
    let store = NolaStore::open(&root.path().join("isolated.sqlite")).unwrap();
    let input = ImportPlanRequest {path:source.to_string_lossy().into_owned(), include_credentials:false,
        ..request(ModelSourceKind::Codex)};
    let plan = plan_import(&store, input, &[]).unwrap();
    assert_eq!(plan.candidates[0].issue.as_deref(), Some("api_credential_required"));
    let result = commit_import(&store, ImportCommitRequest {plan_id:plan.plan_id,
        selections:vec![ImportSelection {candidate_id:plan.candidates[0].candidate_id.clone(),
            display_name:None,update_existing:false,enable_models:true}]}, &[]).unwrap();
    assert_eq!(result[0].status,"failed");
    assert!(store.model_profiles().unwrap().is_empty());
    let connections:Vec<ModelConnectionSnapshot> = execute(&store, ModelCommand::ListConnections).unwrap();
    let accounts:Vec<ModelAccountSnapshot> = execute(&store, ModelCommand::ListAccounts).unwrap();
    assert!(connections.is_empty() && accounts.is_empty());
}

#[test]
fn codex_explicit_openai_route_and_existing_api_credential_are_not_replaced() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("config.toml");
    let document = r#"model = "fixture"
[model_providers.openai]
name = "Explicit route"
base_url = "http://127.0.0.1:12345/v1"
wire_api = "responses"
requires_openai_auth = false
experimental_bearer_token = "isolated-import-key"
"#;
    std::fs::write(&source, document).unwrap();
    let store = NolaStore::open(&root.path().join("isolated.sqlite")).unwrap();
    let mut input = ImportPlanRequest {path:source.to_string_lossy().into_owned(), ..request(ModelSourceKind::Codex)};
    for update in [false,true] {
        input.include_credentials = !update;
        let plan = plan_import(&store,input.clone(),&[]).unwrap();
        assert_eq!(plan.candidates[0].connection.base_url,"http://127.0.0.1:12345/v1");
        assert!(plan.candidates[0].issue.is_none());
        let result = commit_import(&store,ImportCommitRequest {plan_id:plan.plan_id,
            selections:vec![ImportSelection {candidate_id:plan.candidates[0].candidate_id.clone(),
                display_name:None,update_existing:update,enable_models:true}]},&[]).unwrap();
        assert_eq!(result[0].status,"imported");
    }
    let profiles=store.model_profiles().unwrap();
    assert_eq!(crate::host_credentials::for_profile(&store,&profiles[0],None).unwrap(),Some("isolated-import-key".into()));
    std::fs::write(&source,document.replace("12345","12346")).unwrap();
    let changed=plan_import(&store,input,&[]).unwrap();
    assert_eq!(changed.candidates[0].issue.as_deref(),Some("api_credential_required"));
}
