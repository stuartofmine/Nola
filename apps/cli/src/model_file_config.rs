// SPDX-License-Identifier: Apache-2.0
//! Validate and publish a file-declared model catalog without contacting providers.
use super::*;
use crate::i18n::{ui_text, UiText};

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

pub(crate) fn file_command(action: &str, source: &Path) -> String {
    let path = source.to_string_lossy();
    if path.chars().any(char::is_control) {
        format!("nola config {action} <config-path>")
    } else {
        let quoted = if cfg!(windows) {
            path.replace('\'', "''")
        } else {
            path.replace('\'', "'\\''")
        };
        format!("nola config {action} '{quoted}'")
    }
}

impl FileModels {
    #[cfg(test)]
    fn parse(value: Value) -> Result<Self, String> {
        Self::parse_with_command(value, "nola config check")
    }

    pub(crate) fn parse_at(value: Value, source: &std::path::Path) -> Result<Self, String> {
        let command = file_command("check", source);
        Self::parse_with_command(value, &command).map_err(|error| format!("{source:?}\n{error}"))
    }

    fn parse_with_command(mut value: Value, command: &str) -> Result<Self, String> {
        // Request-local cache preferences are parsed by RuntimeConfig, not
        // imported into the credential/model catalog. Keep all other fields strict.
        if let Some(fields) = value.as_object_mut() { fields.remove("openai_cache"); }
        let config: Self = serde_json::from_value(value)
            .map_err(|_| "invalid llm connections configuration; use connections and default_model, not single-route fields")?;
        if config.connections.is_empty() || config.connections.len() > 64 {
            return Err("llm.connections requires 1..64 connections".into());
        }
        let mut ids = BTreeSet::new();
        let mut names = BTreeSet::new();
        for item in &config.connections {
            if !ids.insert(&item.id) || item.id.is_empty() || item.id.len() > 128
                || !item.id.bytes().all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b)) {
                return Err("connection IDs must be unique ASCII letters/digits/underscore/hyphen".into());
            }
            if !names.insert(item.name.trim().to_ascii_lowercase()) {
                return Err(format!("llm.connections: duplicate name for {}; give each connection a different name", item.id));
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
            let same_connection = config.connections.iter().any(|c| c.id == config.default_model.connection);
            let same_model = config.connections.iter().flat_map(|c| &c.models).any(|m| m.id == config.default_model.model);
            let examples = config.connections.iter()
                .filter(|c| !same_connection || c.id == config.default_model.connection)
                .flat_map(|c| c.models.iter().map(move |m| (c, m)))
                .filter(|(_, m)| same_connection || !same_model || m.id == config.default_model.model)
                .take(8).map(|(c, m)| json!({"connection": c.id, "model": m.id}).to_string())
                .collect::<Vec<_>>().join("\n  ");
            use crate::i18n::interaction_text;
            let invalid = json!({"connection": config.default_model.connection, "model": config.default_model.model});
            return Err(format!(
                "{}\n{} {invalid}\n{}\n  {examples}\n{} `{command}`",
                ui_text(UiText::MissingDefaultModel), interaction_text().current_label,
                ui_text(UiText::DefaultModelChoices), ui_text(UiText::ApplyConfigAgain)
            ));
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
        .ok_or("configured model is unavailable; check the file with nola config check and restart Nola")?;
    Ok((profile.id.clone(), profile.revision))
}

pub(crate) fn resolve_default(store: &NolaStore) -> Result<(String, i64), String> {
    let setting: Option<nola_application::ProductSetting> = store.application()?.execute(
        ApplicationCommand::GetProductSetting { key: "chat.default_model".into() })?;
    let config = nola_application::model_selection::parse_default_chat_configuration(setting.as_ref().map(|v| v.value_json.as_str()))?;
    let nola_application::model_selection::DefaultChatSelection::Profile { profile_id } = config.selection else {
        return Err("select a default model in the model center or correct the configuration and restart Nola".into());
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
    let missing = || format!("{}\n  nola model apply-config <config-path>", ui_text(UiText::LocateModelConfig));
    let home = home.ok_or_else(missing)?;
    // RuntimeConfig loads JSON after JSONC, giving JSON the same precedence.
    for name in ["config.json", "config.jsonc"] {
        let path = home.join(".nola").join(name);
        if path.try_exists().map_err(|error| format!("cannot inspect {}: {error}", path.display()))? {
            return path.into_os_string().into_string().map_err(|_| "config path is not UTF-8".into());
        }
    }
    Err(format!("~/.nola/config.json / ~/.nola/config.jsonc\n{}", missing()))
}

pub(crate) fn read_document(path: &str) -> Result<Value, String> {
    let bytes = read_selected(path).map_err(|reason| {
        let guidance = match reason.as_str() {
            "source_not_a_bounded_regular_file" | "source_too_large" => UiText::CheckModelConfigSize,
            _ => UiText::CheckModelConfigFile,
        };
        format!("{path:?}\n{} ({reason})", ui_text(guidance))
    })?;
    let text = std::str::from_utf8(&bytes).map_err(|_| format!("{path:?}: UTF-8 required"))?;
    parse_config_jsonc(text)
        .map_err(|reason| format!("{path:?}\n{} ({reason})", ui_text(UiText::CheckModelConfigJson)))
}

pub(crate) fn declarations(document: &Value, path: &Path) -> Result<FileModels, String> {
    let llm = document.get("llm").cloned()
        .ok_or_else(|| format!("{path:?}\n{}", ui_text(UiText::AddModelConfigLlm)))?;
    FileModels::parse_at(llm, path)
}

#[derive(Serialize)]
pub(crate) struct DeclaredConnection<'a> {
    pub id: &'a str,
    pub origin: String,
    pub protocol: &'static str,
    pub models: Vec<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key_env: Option<&'a str>,
}

impl FileModels {
    pub(crate) fn declared_connections(&self) -> Vec<DeclaredConnection<'_>> {
        self.connections.iter().map(|connection| DeclaredConnection {
            id: &connection.id,
            // Do not include URL userinfo, paths, query parameters or fragments.
            origin: url::Url::parse(&connection.endpoint).map(|url| url.origin().ascii_serialization()).unwrap_or_default(),
            protocol: connection.protocol.as_str(),
            models: connection.models.iter().map(|model| model.id.as_str()).collect(),
            api_key_env: connection.api_key_env.as_deref(),
        }).collect()
    }

    pub(crate) fn check_gateway_routes(&self, gateways: &[String]) -> Result<(), String> {
        if let Some(item) = self.connections.iter().find(|item| is_gateway(&item.endpoint, gateways)) {
            return Err(format!("llm.connections[{}].endpoint: nola_gateway_loop; use the upstream model endpoint", item.id));
        }
        Ok(())
    }
}

pub(crate) fn apply(store: &NolaStore, config: &FileModels, gateways: &[String]) -> Result<Value, String> {
    apply_with_sink(store, config, gateways, &mut VaultImportCredentialSink)
}

pub(crate) fn apply_on_startup(store: &NolaStore, config: &FileModels, gateways: &[String], source: &Path) -> Result<Value, String> {
    // A repository can declare a route, but cannot bind arbitrary environment
    // secrets to that route merely by being opened in the CLI.
    let user_paths: Vec<_> = crate::platform_paths::home_dir().into_iter()
        .flat_map(|home| ["config.jsonc", "config.json"].map(|name| home.join(".nola").join(name))).collect();
    let mut user_routes = Vec::new();
    let restrict_environment = !user_paths.iter().any(|path| path == source);
    if restrict_environment && config.connections.iter().any(|item| item.api_key_env.is_some()) {
        for path in user_paths {
            if !path.try_exists().map_err(|e| e.to_string())? { continue; }
            let document = read_document(path.to_str().ok_or("config path is not UTF-8")?)?;
            if document.get("llm").is_some_and(|llm| llm.get("connections").is_some()) {
                user_routes = declarations(&document, &path)?.connections;
            }
        }
    }
    apply_catalog(store, config, gateways, &mut VaultImportCredentialSink, |item| {
        !restrict_environment || user_routes.iter().any(|route|
            route.api_key_env == item.api_key_env && route.protocol == item.protocol && route.endpoint == item.endpoint)
    })
}

fn apply_with_sink(store: &NolaStore, config: &FileModels, gateways: &[String], sink: &mut impl ImportCredentialSink) -> Result<Value, String> {
    apply_catalog(store, config, gateways, sink, |_| true)
}

fn apply_catalog(store: &NolaStore, config: &FileModels, gateways: &[String], sink: &mut impl ImportCredentialSink,
    allow_environment: impl Fn(&FileConnection) -> bool) -> Result<Value, String> {
    // Validate all declarations, credentials and ownership before the first write.
    config.check_gateway_routes(gateways)?;
    let existing: Vec<ModelConnectionSnapshot> = execute(store, ModelCommand::ListConnections)?;
    // An earlier declaration can release its old name for a later one.
    let mut names: std::collections::BTreeMap<_, _> = existing.iter().map(|snapshot| {
        let connection = &snapshot.document.connection;
        (connection.display_name.trim().to_ascii_lowercase(), connection.id.clone())
    }).collect();
    let mut prepared = vec![];
    for item in &config.connections {
        let mut connection = item.connection();
        let name = connection.display_name.trim().to_ascii_lowercase();
        if let Some(owner) = names.get(&name).filter(|id| *id != &connection.id) {
            return Err(format!("llm.connections[{}].name: already used by {owner}; choose a different name in this file", item.id));
        }
        let current = existing.iter().find(|snapshot| snapshot.document.connection.id == connection.id);
        if let Some(previous) = current {
            names.remove(&previous.document.connection.display_name.trim().to_ascii_lowercase());
        }
        names.insert(name, connection.id.clone());
        if current.is_some_and(|s| s.document.connection.source != ModelSourceKind::Manual
            || s.document.connection.source_id != connection.source_id) {
            return Err("connection ID is owned by a different source".into());
        }
        let read_environment = allow_environment(item);
        let secret = item.api_key_env.as_ref().filter(|_| read_environment).and_then(|name| std::env::var(name)
            .ok().filter(|v| !v.trim().is_empty()));
        let existing_account = if let Some(id) = current.as_ref().and_then(|s| s.document.connection.account_id.clone()) {
            let account = execute::<Option<ModelAccountSnapshot>>(store, ModelCommand::GetAccount { id })?
                .ok_or_else(|| format!("{}: configured account is unavailable", item.id))?;
            Some(account)
        } else { None };
        connection.account_id = existing_account.as_ref().map(|s| s.account.id.clone());
        let reusable = match existing_account.as_ref() {
            Some(account) if current.is_some_and(|s| s.document.connection.same_route(&connection))
                && account.account.source == connection.source && account.account.state == ModelAccountState::Ready => {
                    match account.account.credential_ref.as_deref() {
                        Some(reference) => sink.matches_binding(store, &connection, reference, secret.as_deref())?,
                        None => false,
                    }
                }
            _ => false,
        };
        if secret.is_none() && !reusable {
            if existing_account.is_some() {
                return Err("existing credential cannot be reused for this route; provide credentials explicitly".into());
            }
            if let Some(name) = &item.api_key_env {
                if !read_environment {
                    return Err(format!("{}: declare the endpoint, protocol and api_key_env in the user configuration to bind {name} on startup", item.id));
                }
                return Err(format!("{}: set {name} in the environment, then retry", item.id));
            }
        }
        let secret = if reusable { None } else { secret };
        prepared.push(PrivateCandidate { public: ImportCandidate { candidate_id: connection.id.clone(), connection,
            models: item.catalog(), credential_status: if secret.is_some() { "provided" } else { "none" }.into(),
            issue: None, existing_revision: current.map(|s| s.revision) }, secret, existing_account });
    }
    let mut app = store.application()?;
    let mut staged = Vec::new();
    let mut connections = Vec::with_capacity(prepared.len());
    for candidate in prepared {
        let connection = match prepare_import_account(store, &candidate, candidate.public.connection.clone(), sink) {
            Ok(connection) => connection,
            Err(error) => return Err(discard_staged_secrets(store, sink, &staged, error)),
        };
        if let Some(reference) = connection.account.as_ref().and_then(|account| account.credential_ref.as_ref()) {
            staged.extend(connection.input.protocols.iter().map(|protocol| format!("{reference}:{}", protocol.as_str())));
        }
        connections.push(connection);
    }
    let command = ModelCommand::ApplyFileCatalog {
        connections, default_connection: format!("config-{}", config.default_model.connection),
        default_model: config.default_model.model.clone(), updated_at_ms: chrono::Utc::now().timestamp_millis(),
    };
    app.execute_typed(ApplicationCommand::Model { command }).map_err(|error| {
        if error.requires_reconciliation() {
            // A committed account may already refer to these secrets.
            error.to_string()
        } else {
            discard_staged_secrets(store, sink, &staged, error.to_string())
        }
    })
}

fn discard_staged_secrets(store: &NolaStore, sink: &mut impl ImportCredentialSink, keys: &[String], error: String) -> String {
    match sink.discard_staged(store, keys) {
        Ok(()) => error,
        Err(cleanup) => format!("{error}; credential cleanup: {cleanup}"),
    }
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
        assert!(error.contains("llm.default_model") && error.contains("nola config check"));
        assert!(!error.contains("PRIVATE_CREDENTIAL_VARIABLE") && !error.contains("https://a.invalid"));
    }
    #[test]
    fn missing_default_prioritizes_the_declared_connection_and_identifies_the_file() {
        let mut input = fixture();
        input["connections"][0]["models"] = json!((0..12).map(|n|
            json!({"id":format!("other-{n}"),"name":format!("Other {n}")})).collect::<Vec<_>>());
        input["default_model"] = json!({"connection":"b","model":"typo"});
        let source = std::path::Path::new("model files/QA's models.jsonc");
        let error = FileModels::parse_at(input, source).err().expect("missing model");
        assert!(error.contains(r#"{"connection":"b","model":"one"}"#) && !error.contains("other-0"));
        assert!(error.contains("QA's models.jsonc") && error.contains("nola config check"));
        #[cfg(unix)]
        assert!(error.contains(r#"'model files/QA'\''s models.jsonc'"#));
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
        let first = apply_with_sink(&store, &FileModels::parse(fixture()).unwrap(), &[], &mut NoSecrets).unwrap();
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
        let second = apply_with_sink(&store, &FileModels::parse(next).unwrap(), &[], &mut NoSecrets).unwrap();
        assert_ne!(first["default_profile_id"], second["default_profile_id"]);
        assert_eq!(store.model_profiles().unwrap().len(), 3);
        assert_eq!(resolve_default(&store).unwrap().0, second["default_profile_id"].as_str().unwrap());
    }
    #[test]
    fn unchanged_startup_preserves_revisions_and_late_failure_rolls_back_all_routes() {
        let root = tempfile::tempdir().unwrap();
        let store = NolaStore::open(&root.path().join("catalog.sqlite")).unwrap();
        let config = FileModels::parse(fixture()).unwrap();
        let first = apply_with_sink(&store, &config, &[], &mut NoSecrets).unwrap();
        let snapshot = || -> Value {
            json!({
                "connections": execute::<Vec<ModelConnectionSnapshot>>(&store, ModelCommand::ListConnections).unwrap(),
                "profiles": store.model_profiles().unwrap(),
                "default": resolve_default(&store).unwrap(),
            })
        };
        let before = snapshot();
        assert_eq!(apply_with_sink(&store, &config, &[], &mut NoSecrets).unwrap(), first);
        assert_eq!(snapshot(), before);

        let connections = config.connections.iter().map(|item| {
            let mut input = item.connection();
            let current = execute::<Option<ModelConnectionSnapshot>>(&store,
                ModelCommand::GetConnection { id: input.id.clone() }).unwrap().unwrap();
            input.display_name.push_str(" changed");
            nola_application::FileCatalogConnection {
                input, expected_revision: Some(current.revision), models: item.catalog(),
                account: None, expected_account_revision: None,
            }
        }).collect();
        let error = execute::<Value>(&store, ModelCommand::ApplyFileCatalog {
            connections, default_connection: "config-b".into(), default_model: "absent".into(),
            updated_at_ms: chrono::Utc::now().timestamp_millis(),
        }).unwrap_err();
        assert!(error.contains("file default"), "{error}");
        assert_eq!(snapshot(), before);
    }

    #[test] fn preflight_failure_does_not_save_earlier_connections() {
        let root = tempfile::tempdir().unwrap();
        let store = NolaStore::open(&root.path().join("canonical.sqlite")).unwrap();
        let mut v = fixture(); v["connections"][1]["api_key_env"] = json!("NOLA_TEST_ABSENT_CONFIG_SECRET_7A1F");
        assert!(apply_with_sink(&store, &FileModels::parse(v).unwrap(), &[], &mut NoSecrets).is_err());
        assert!(store.model_profiles().unwrap().is_empty());
    }
    #[test] fn reapply_reuses_owned_account_without_environment_but_never_across_routes() {
        let root = tempfile::tempdir().unwrap();
        let store = NolaStore::open(&root.path().join("canonical.sqlite")).unwrap();
        apply_with_sink(&store, &FileModels::parse(fixture()).unwrap(), &[], &mut NoSecrets).unwrap();
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
        apply_with_sink(&store, &FileModels::parse(config.clone()).unwrap(), &[], &mut NoSecrets).unwrap();
        let persisted: ModelAccountSnapshot = execute::<Option<ModelAccountSnapshot>>(&store,
            ModelCommand::GetAccount { id: account.account.id.clone() }).unwrap().unwrap();
        assert_eq!(persisted, account);
        config["connections"][0]["endpoint"] = json!("https://other.invalid/v1");
        assert!(apply_with_sink(&store, &FileModels::parse(config).unwrap(), &[], &mut NoSecrets)
            .unwrap_err().contains("cannot be reused"));
    }
}
