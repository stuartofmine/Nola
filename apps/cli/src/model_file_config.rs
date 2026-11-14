// SPDX-License-Identifier: Apache-2.0
//! Explicit file-to-Core connection authoring. No network, migration or startup writes.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ModelRef {
    pub connection: String,
    pub model: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FileModels {
    pub connections: Vec<FileConnection>,
    pub default_model: ModelRef,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FileConnection {
    id: String,
    name: String,
    endpoint: String,
    protocol: ModelProtocol,
    api_key_env: Option<String>,
    models: Vec<FileModel>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileModel {
    id: String,
    name: String,
    context_window: Option<u64>,
    output_limit: Option<u64>,
    capabilities: Option<ModelExecutionCapabilities>,
}

impl FileModels {
    pub(crate) fn parse(mut value: Value) -> Result<Self, String> {
        // Request-local cache preferences are parsed by RuntimeConfig, not
        // imported into the credential/model catalog. Keep all other fields strict.
        if let Some(fields) = value.as_object_mut() { fields.remove("openai_cache"); }
        let config: Self = serde_json::from_value(value)
            .map_err(|_| "invalid llm connections configuration; use connections and default_model, not single-route fields")?;
        if config.connections.is_empty() || config.connections.len() > 64 {
            return Err("llm.connections requires 1..64 connections".into());
        }
        let mut ids = BTreeSet::new();
        for item in &config.connections {
            if !ids.insert(&item.id) || item.id.is_empty() || item.id.len() > 128
                || !item.id.bytes().all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b)) {
                return Err("connection IDs must be unique ASCII letters/digits/underscore/hyphen".into());
            }
            if item.protocol == ModelProtocol::SiwcResponses { return Err("subscription accounts require owned authorization".into()); }
            item.connection().validate().map_err(|_| "invalid connection identity, URL or protocol")?;
            ModelConnectionDocument { format_version: 1, connection: item.connection(),
                catalog: Some(ModelCatalog { observed_at_ms: 0, source: ModelSourceKind::Manual, models: item.catalog() }),
                capability_assessments: None }.validate().map_err(|_| "invalid model catalog or capability declaration")?;
            let mut models = BTreeSet::new();
            if item.models.is_empty() || item.models.len() > 256 { return Err("connection requires 1..256 models".into()); }
            for model in &item.models {
                if model.id.trim().is_empty() || model.name.trim().is_empty() || !models.insert(&model.id)
                    || model.context_window == Some(0) || model.output_limit == Some(0) {
                    return Err("invalid or duplicate model declaration".into());
                }
            }
            if item.api_key_env.as_ref().is_some_and(|name| name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')) {
                return Err("invalid api_key_env name".into());
            }
        }
        if !config.connections.iter().any(|c| c.id == config.default_model.connection
            && c.models.iter().any(|m| m.id == config.default_model.model)) {
            let examples = config.connections.iter().flat_map(|connection| connection.models.iter()
                .map(move |model| json!({"connection": connection.id, "model": model.id}).to_string()))
                .take(8).collect::<Vec<_>>().join("\n  ");
            return Err(format!(
                "default_model must refer to a declared connection and model.\nCurrent: {}\nSet llm.default_model using connection/model IDs from llm.connections, for example:\n  {examples}\nSave the configuration, then run `nola model apply-config` again.",
                json!({"connection": config.default_model.connection, "model": config.default_model.model})));
        }
        Ok(config)
    }
}

impl FileConnection {
    fn connection(&self) -> ModelConnection {
        ModelConnection { id: format!("config-{}", self.id), display_name: self.name.clone(),
            base_url: self.endpoint.clone(), protocols: vec![self.protocol], account_id: None,
            source: ModelSourceKind::Manual, source_id: Some(format!("config:{}", self.id)), enabled: true }
    }
    fn catalog(&self) -> Vec<CatalogModel> {
        self.models.iter().map(|m| CatalogModel { model_id: m.id.clone(), display_name: m.name.clone(),
            protocol: self.protocol, context_window: m.context_window, output_limit: m.output_limit,
            modalities: vec!["text".into()], reasoning_levels: vec![], summary_supported: None, execution_capabilities: m.capabilities }).collect()
    }
}

pub(crate) fn resolve(store: &NolaStore, reference: &ModelRef) -> Result<(String, i64), String> {
    let connection = format!("config-{}", reference.connection);
    let profiles = store.model_profiles()?;
    let profile = profiles.iter().find(|p| p.deleted_at_ms.is_none() && p.enabled && p.model_name == reference.model
        && serde_json::from_str::<Value>(&p.source_json).ok().is_some_and(|s|
            s["connectionID"] == connection && s["endpointEnabled"] != false))
        .ok_or("configured model is unavailable; run nola model apply-config with this configuration file")?;
    Ok((profile.id.clone(), profile.revision))
}

pub(crate) fn resolve_default(store: &NolaStore) -> Result<(String, i64), String> {
    let setting: Option<nola_application::ProductSetting> = store.application()?.execute(
        ApplicationCommand::GetProductSetting { key: "chat.default_model".into() })?;
    let config = nola_application::model_selection::parse_default_chat_configuration(setting.as_ref().map(|v| v.value_json.as_str()))?;
    let nola_application::model_selection::DefaultChatSelection::Profile { profile_id } = config.selection else {
        return Err("select a default model in the model center or run nola model apply-config".into());
    };
    let profile = store.model_profile(&profile_id)?.filter(|p| p.enabled && p.deleted_at_ms.is_none())
        .ok_or("default model is unavailable; select an enabled model in the model center")?;
    Ok((profile.id, profile.revision))
}

pub(crate) fn selected_path(explicit: Option<&str>) -> Result<String, String> {
    selected_path_from(explicit, crate::platform_paths::home_dir().as_deref())
}

fn selected_path_from(explicit: Option<&str>, home: Option<&std::path::Path>) -> Result<String, String> {
    if let Some(path) = explicit {
        if !std::path::Path::new(path).is_absolute() {
            return Err("model apply-config requires an absolute config path when a path is supplied".into());
        }
        return Ok(path.to_owned());
    }
    let home = home.ok_or("cannot locate Nola home; use model apply-config <absolute-config-path>")?;
    // RuntimeConfig loads JSON after JSONC, giving JSON the same precedence.
    for name in ["config.json", "config.jsonc"] {
        let path = home.join(".nola").join(name);
        if path.try_exists().map_err(|error| format!("cannot inspect {}: {error}", path.display()))? {
            return path.into_os_string().into_string().map_err(|_| "config path is not UTF-8".into());
        }
    }
    Err("no ~/.nola/config.json or config.jsonc; use model apply-config <absolute-config-path>".into())
}

pub(crate) fn apply(store: &NolaStore, path: &str, gateways: &[String]) -> Result<Value, String> {
    let document = parse_jsonc(&read_selected(path)?)?;
    let config = FileModels::parse(document.get("llm").cloned().ok_or("missing llm object")?)?;
    apply_with_sink(store, config, gateways, &mut VaultImportCredentialSink)
}

fn apply_with_sink(store: &NolaStore, config: FileModels, gateways: &[String], sink: &mut impl ImportCredentialSink) -> Result<Value, String> {
    // Validate all declarations, credentials and ownership before the first write.
    let mut prepared = vec![];
    for item in &config.connections {
        let mut connection = item.connection();
        if is_gateway(&connection.base_url, gateways) { return Err("nola_gateway_loop".into()); }
        let current: Option<ModelConnectionSnapshot> = execute(store, ModelCommand::GetConnection { id: connection.id.clone() })?;
        if current.as_ref().is_some_and(|s| s.document.connection.source != ModelSourceKind::Manual
            || s.document.connection.source_id != connection.source_id) {
            return Err("connection ID is owned by a different source".into());
        }
        let secret = item.api_key_env.as_ref().and_then(|name| std::env::var(name)
            .ok().filter(|v| !v.trim().is_empty()));
        let existing_account = if let Some(id) = current.as_ref().and_then(|s| s.document.connection.account_id.clone()) {
            execute::<Option<ModelAccountSnapshot>>(store, ModelCommand::GetAccount { id })?
        } else { None };
        connection.account_id = existing_account.as_ref().map(|s| s.account.id.clone());
        if secret.is_none() {
            if let Some(account) = &existing_account {
                let same_route = current.as_ref().is_some_and(|snapshot|
                    snapshot.document.connection.same_route(&connection));
                if !same_route || account.account.source != connection.source
                    || account.account.state != ModelAccountState::Ready || account.account.credential_ref.is_none() {
                    return Err("existing credential cannot be reused for this route; provide credentials explicitly".into());
                }
            } else if item.api_key_env.is_some() {
                return Err("configured credential environment variable is missing".into());
            }
        }
        prepared.push(PrivateCandidate { public: ImportCandidate { candidate_id: connection.id.clone(), connection,
            models: item.catalog(), credential_status: if secret.is_some() { "provided" } else { "none" }.into(),
            issue: None, existing_revision: current.map(|s| s.revision) }, secret, existing_account });
    }
    let mut applied = vec![];
    for candidate in prepared {
        let id = candidate.public.connection.id.clone();
        import_candidate(store, &candidate, &ImportSelection { candidate_id: id.clone(), display_name: None,
            update_existing: true, enable_models: true }, gateways, sink)
            .map_err(|_| format!("configuration apply failed at {id}; already applied: {}. Inspect connections before retrying", applied.join(", ")))?;
        applied.push(id);
    }
    let (id, revision) = resolve(store, &config.default_model)?;
    let mut application = store.application()?;
    let old: Option<nola_application::ProductSetting> = application.execute(ApplicationCommand::GetProductSetting { key: "chat.default_model".into() })?;
    let _: Value = application.execute(ApplicationCommand::PutProductSettingChecked {
        key: "chat.default_model".into(), expected_revision: old.map(|v| v.revision),
        value_json: json!({"version":1,"selection":{"kind":"profile","profile_id":id}}).to_string(),
        updated_at_ms: chrono::Utc::now().timestamp_millis() })?;
    Ok(json!({"connections":applied,"default_profile_id":id,"default_profile_revision":revision}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn apply_config_path_uses_nola_home_and_preserves_explicit_selection() {
        let home = tempfile::tempdir().unwrap();
        assert!(selected_path_from(None, Some(home.path())).is_err());
        std::fs::create_dir(home.path().join(".nola")).unwrap();
        let jsonc = home.path().join(".nola").join("config.jsonc");
        std::fs::write(&jsonc, "{}").unwrap();
        assert_eq!(selected_path_from(None, Some(home.path())).unwrap(), jsonc.to_str().unwrap());
        let json = home.path().join(".nola").join("config.json");
        std::fs::write(&json, "{}").unwrap();
        assert_eq!(selected_path_from(None, Some(home.path())).unwrap(), json.to_str().unwrap());
        let explicit = home.path().join("explicit").join("config.json");
        assert_eq!(selected_path_from(explicit.to_str(), Some(home.path())).unwrap(), explicit.to_str().unwrap());
        assert!(selected_path_from(Some("relative.json"), Some(home.path())).is_err());
        assert!(selected_path_from(None, None).is_err());
    }
    fn fixture() -> Value { json!({"connections":[{"id":"a","name":"A","endpoint":"https://a.invalid/v1","protocol":"chat_completions","models":[{"id":"one","name":"One"},{"id":"two","name":"Two"}]},{"id":"b","name":"B","endpoint":"https://b.invalid/v1","protocol":"responses","models":[{"id":"one","name":"One"}]}],"default_model":{"connection":"a","model":"two"}}) }

    #[test]
    fn missing_default_explains_exact_ids_and_recovery_without_exposing_connection_secrets() {
        let mut input = fixture();
        input["connections"][0]["api_key_env"] = json!("PRIVATE_CREDENTIAL_VARIABLE");
        input["default_model"] = json!({"connection":"missing","model":"model-not-declared"});
        let error = FileModels::parse(input).err().expect("invalid default must fail");
        assert!(error.contains(r#"{"connection":"a","model":"two"}"#));
        assert!(error.contains("model-not-declared"));
        assert!(error.contains("llm.default_model") && error.contains("nola model apply-config"));
        assert!(!error.contains("PRIVATE_CREDENTIAL_VARIABLE") && !error.contains("https://a.invalid"));
    }
    #[test] fn accepts_multiple_connections_and_same_model_names() {
        let mut value=fixture();value["openai_cache"]=json!({"enabled":true,"history_breakpoint":true});
        assert!(FileModels::parse(value.clone()).is_ok());
        value["unexpected"]=json!(true);assert!(FileModels::parse(value).is_err());
    }
    #[test] fn rejects_ambiguous_defaults_duplicates_and_mixed_routes() {
        let mut v = fixture(); v["default_model"]["connection"] = json!("missing"); assert!(FileModels::parse(v).is_err());
        let mut v = fixture(); v["connections"][1]["id"] = json!("a"); assert!(FileModels::parse(v).is_err());
        let mut v = fixture(); v["endpoint"] = json!("https://old.invalid"); assert!(FileModels::parse(v).is_err());
    }
    struct NoSecrets;
    impl ImportCredentialSink for NoSecrets {
        fn store(&mut self, _: &NolaStore, _: &ModelConnection, _: &str, _: &str) -> Result<String,String> {
            panic!("anonymous fixture must not access credential vault")
        }
    }
    #[test] fn file_connections_share_core_profiles_and_default_and_can_be_reapplied() {
        let root = tempfile::tempdir().unwrap();
        let store = NolaStore::open(&root.path().join("canonical.sqlite")).unwrap();
        let first = apply_with_sink(&store, FileModels::parse(fixture()).unwrap(), &[], &mut NoSecrets).unwrap();
        assert_eq!(store.model_profiles().unwrap().len(), 3);
        for profile in store.model_profiles().unwrap() {
            let parameters: Value = serde_json::from_str(&profile.parameters_json).unwrap();
            assert!(parameters["topK"].is_null());
            assert!(parameters["repetitionPenalty"].is_null());
            let source: Value = serde_json::from_str(&profile.source_json).unwrap();
            let request = crate::model_profile_resolution::session_override_request(&json!({"topK":parameters["topK"],
                "repetitionPenalty":parameters["repetitionPenalty"],"stopSequences":""}),
                &source["reasoning"], source["wireProtocol"].as_str().unwrap()).unwrap();
            let resolved = crate::model_profile_resolution::resolve_remote_profile_settings(&profile, &request, &Default::default(), &[]).unwrap();
            // Anonymous catalog entries have no reasoning request capability,
            // but the transport must retain explicitly emitted reasoning events.
            assert_eq!(resolved.runtime.model_capabilities.reasoning, Some(false));
            assert_eq!(resolved.runtime.model_capabilities.stream_reasoning, Some(true));
        }
        assert_eq!(resolve_default(&store).unwrap().0, first["default_profile_id"].as_str().unwrap());
        let mut next = fixture(); next["default_model"] = json!({"connection":"b","model":"one"});
        let second = apply_with_sink(&store, FileModels::parse(next).unwrap(), &[], &mut NoSecrets).unwrap();
        assert_ne!(first["default_profile_id"], second["default_profile_id"]);
        assert_eq!(store.model_profiles().unwrap().len(), 3);
        assert_eq!(resolve_default(&store).unwrap().0, second["default_profile_id"].as_str().unwrap());
    }
    #[test] fn preflight_failure_does_not_save_earlier_connections() {
        let root = tempfile::tempdir().unwrap();
        let store = NolaStore::open(&root.path().join("canonical.sqlite")).unwrap();
        let mut v = fixture(); v["connections"][1]["api_key_env"] = json!("NOLA_TEST_ABSENT_CONFIG_SECRET_7A1F");
        assert!(apply_with_sink(&store, FileModels::parse(v).unwrap(), &[], &mut NoSecrets).is_err());
        assert!(store.model_profiles().unwrap().is_empty());
    }
    #[test] fn reapply_reuses_owned_account_without_environment_but_never_across_routes() {
        let root = tempfile::tempdir().unwrap();
        let store = NolaStore::open(&root.path().join("canonical.sqlite")).unwrap();
        apply_with_sink(&store, FileModels::parse(fixture()).unwrap(), &[], &mut NoSecrets).unwrap();
        let account: ModelAccountSnapshot = execute(&store, ModelCommand::SaveAccount {
            input: ModelAccount { id: "fixture-account".into(), display_name: "Fixture".into(),
                source: ModelSourceKind::Manual, credential_ref: Some("private-fixture-reference".into()),
                state: ModelAccountState::Ready }, expected_revision: None, updated_at_ms: 1,
        }).unwrap();
        let mut route: ModelConnectionSnapshot = execute::<Option<ModelConnectionSnapshot>>(&store,
            ModelCommand::GetConnection { id: "config-a".into() }).unwrap().unwrap();
        route.document.connection.account_id = Some(account.account.id.clone());
        let _: ModelConnectionSnapshot = execute(&store, ModelCommand::SaveConnection {
            input: route.document.connection, expected_revision: Some(route.revision), updated_at_ms: 2,
        }).unwrap();
        let mut config = fixture();
        config["connections"][0]["api_key_env"] = json!("NOLA_TEST_ABSENT_CONFIG_SECRET_7A1F");
        apply_with_sink(&store, FileModels::parse(config.clone()).unwrap(), &[], &mut NoSecrets).unwrap();
        let persisted: ModelAccountSnapshot = execute::<Option<ModelAccountSnapshot>>(&store,
            ModelCommand::GetAccount { id: account.account.id.clone() }).unwrap().unwrap();
        assert_eq!(persisted, account);
        config["connections"][0]["endpoint"] = json!("https://other.invalid/v1");
        assert!(apply_with_sink(&store, FileModels::parse(config).unwrap(), &[], &mut NoSecrets)
            .unwrap_err().contains("cannot be reused"));
    }
}
