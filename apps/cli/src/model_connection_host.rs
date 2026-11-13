// SPDX-License-Identifier: Apache-2.0
//! Explicit, read-only source import. Preview tokens own private credential data;
//! clients receive only sanitized candidates and never round-trip source secrets.
use crate::{host_credentials::HostCredentialVault, store::NolaStore};
use nola_application::{ApplicationCommand, ModelCommand};
use nola_domain::{
    generation::ModelProtocol, model_capabilities::ModelExecutionCapabilities, model_connection::*,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

#[path = "model_file_config.rs"]
pub(crate) mod file_config;

const MAX_SOURCE_BYTES: u64 = 2 * 1024 * 1024;
const PLAN_LIFETIME_MS: i64 = 10 * 60 * 1000;

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportPlanRequest {
    pub source: ModelSourceKind,
    pub path: String,
    pub credential_path: Option<String>,
    pub include_credentials: bool,
    pub allow_environment: bool,
}

#[derive(Clone, Serialize)]
pub struct ImportCandidate {
    pub candidate_id: String,
    pub connection: ModelConnection,
    pub models: Vec<CatalogModel>,
    pub credential_status: String,
    pub issue: Option<String>,
    pub existing_revision: Option<i64>,
}

#[derive(Serialize)]
pub struct ImportPlan {
    pub plan_id: String,
    pub candidates: Vec<ImportCandidate>,
    pub expires_at_ms: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportSelection {
    pub candidate_id: String,
    pub display_name: Option<String>,
    pub update_existing: bool,
    pub enable_models: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportCommitRequest {
    pub plan_id: String,
    pub selections: Vec<ImportSelection>,
}

#[derive(Debug, Serialize)]
pub struct ImportItemResult {
    pub candidate_id: String,
    pub connection_id: String,
    pub status: String,
    pub error: Option<String>,
}

// Intentionally not Debug or Serialize.
struct PrivateCandidate {
    public: ImportCandidate,
    secret: Option<String>,
    existing_account: Option<ModelAccountSnapshot>,
}
struct PrivatePlan {
    request: ImportPlanRequest,
    database: PathBuf,
    digest: Vec<u8>,
    candidates: Vec<PrivateCandidate>,
    expires_at_ms: i64,
}
static PLANS: OnceLock<Mutex<BTreeMap<String, PrivatePlan>>> = OnceLock::new();

fn execute<T: serde::de::DeserializeOwned>(
    store: &NolaStore,
    command: ModelCommand,
) -> Result<T, String> {
    store
        .application()?
        .execute(ApplicationCommand::Model { command })
}

pub(crate) fn read_selected(path: &str) -> Result<Vec<u8>, String> {
    let path = Path::new(path);
    if !path.is_absolute() {
        return Err("source_path_must_be_absolute".into());
    }
    let metadata = std::fs::symlink_metadata(path).map_err(|_| "source_unavailable")?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > MAX_SOURCE_BYTES
    {
        return Err("source_not_a_bounded_regular_file".into());
    }
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path).map_err(|_| "source_access_denied")?;
    if !file.metadata().map_err(|_| "source_unavailable")?.is_file() {
        return Err("source_not_a_regular_file".into());
    }
    let mut bytes = vec![];
    file.take(MAX_SOURCE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "source_read_failed")?;
    if bytes.len() as u64 > MAX_SOURCE_BYTES {
        return Err("source_too_large".into());
    }
    Ok(bytes)
}

/// Share the JSONC parser with the reverse client adapter; no hand-written
/// comment stripping or alternate configuration format conversion.
pub(crate) fn jsonc_options() -> jsonc_parser::ParseOptions {
    jsonc_parser::ParseOptions {
        allow_comments: true,
        allow_trailing_commas: true,
        allow_loose_object_property_names: false,
        allow_missing_commas: false,
        allow_single_quoted_strings: false,
        allow_hexadecimal_numbers: false,
        allow_unary_plus_numbers: false,
        allow_bare_decimal_point_numbers: false,
        allow_non_finite_numbers: false,
        allow_extended_string_escapes: false,
    }
}

fn parse_jsonc(bytes: &[u8]) -> Result<Value, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "source_must_be_utf8")?;
    let root = jsonc_parser::cst::CstRootNode::parse(text, &jsonc_options())
        .map_err(|_| "invalid_source_json")?;
    root.to_serde_value()
        .ok_or_else(|| "invalid_source_json".into())
}

fn parse_document(source: ModelSourceKind, bytes: &[u8]) -> Result<Value, String> {
    if source == ModelSourceKind::OpenCode {
        return parse_jsonc(bytes);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| "source_must_be_utf8")?;
    let value: toml::Value = toml::from_str(text).map_err(|_| "invalid_source_toml")?;
    serde_json::to_value(value).map_err(|_| "invalid_source_document".into())
}

fn text<'a>(value: &'a Value, field: &str) -> Option<&'a str> {
    value.get(field)?.as_str().filter(|s| !s.is_empty())
}

fn literal_secret(value: Option<&str>, request: &ImportPlanRequest) -> Option<String> {
    if !request.include_credentials {
        return None;
    }
    let value = value?;
    if let Some(name) = value
        .strip_prefix("{env:")
        .and_then(|v| v.strip_suffix('}'))
    {
        return environment(name, request);
    }
    // Never execute interpolation or read paths named by source strings.
    if value.contains("{file:") || value.contains("${") || value.chars().any(char::is_control) {
        return None;
    }
    (!value.is_empty()).then(|| value.to_owned())
}

fn environment(name: &str, request: &ImportPlanRequest) -> Option<String> {
    if !request.include_credentials
        || !request.allow_environment
        || name.is_empty()
        || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
    {
        return None;
    }
    std::env::var(name)
        .ok()
        .filter(|s| !s.is_empty() && !s.chars().any(char::is_control))
}

// Provider lists can expose Codex/Magpie objects or plain effort names.
// Preserve catalog observations (including future names); Core decides which
// controls are executable. An explicit empty list is not a fallback signal.
fn catalog_reasoning_levels(config: &Value) -> Vec<String> {
    let values = config.get("supported_reasoning_levels")
        .or_else(|| config.get("reasoning_levels"))
        .and_then(Value::as_array);
    let mut levels = Vec::new();
    for item in values.into_iter().flatten() {
        if let Some(effort) = item.as_str().or_else(|| item.get("effort").and_then(Value::as_str)) {
            if !effort.is_empty() && !levels.iter().any(|value| value == effort) {
                levels.push(effort.to_owned());
            }
        }
    }
    levels
}

fn make_model(id: &str, config: &Value, protocol: ModelProtocol) -> CatalogModel {
    let modalities = config["modalities"]["input"]
        .as_array()
        .or_else(|| config["architecture"]["input_modalities"].as_array())
        .or_else(|| config["modalities"].as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_else(|| vec!["text".into()]);
    let supported = config["supported_parameters"].as_array();
    let supports = |name: &str| supported.is_some_and(|items| items.iter().any(|v| v.as_str() == Some(name)));
    let tool_calls = config.get("tool_call").and_then(Value::as_bool)
        .or_else(|| supported.map(|_| supports("tools")));
    let reasoning_levels = catalog_reasoning_levels(config);
    let has_metadata = !reasoning_levels.is_empty() || tool_calls.is_some() || supported.is_some() || config["reasoning"].is_boolean()
        || config["temperature"].is_boolean() || config["architecture"]["input_modalities"].is_array()
        || config["modalities"]["input"].is_array();
    let declared = has_metadata.then(|| ModelExecutionCapabilities {
        tool_calls: ModelExecutionCapabilities::for_import(tool_calls).tool_calls,
        image_input: modalities.iter().any(|v| v == "image"),
        reasoning: config["reasoning"].as_bool().unwrap_or_else(|| !reasoning_levels.is_empty() || supports("reasoning") || supports("reasoning_effort")),
        sampling_parameters: config["temperature"].as_bool().unwrap_or_else(|| supports("temperature")),
        ..Default::default()
    });
    CatalogModel {
        model_id: id.into(),
        display_name: text(config, "display_name").or_else(|| text(config, "name")).unwrap_or(id).into(),
        protocol,
        context_window: config["limit"]["context"]
            .as_u64()
            .or_else(|| config["context_window"].as_u64())
            .or_else(|| config["max_context_size"].as_u64())
            .or_else(|| config["context_length"].as_u64()),
        output_limit: config["limit"]["output"].as_u64()
            .or_else(|| config["top_provider"]["max_completion_tokens"].as_u64())
            .or_else(|| config["max_output_tokens"].as_u64()),
        modalities,
        reasoning_levels,
        summary_supported: config["supports_reasoning_summary_parameter"].as_bool(),
        execution_capabilities: declared,
    }
}

fn origin(url: &str) -> Option<(String, String, u16)> {
    let url = url::Url::parse(url).ok()?;
    let host = url.host_str()?;
    let host = if host == "localhost"
        || host
            .trim_matches(['[', ']'])
            .parse::<std::net::IpAddr>()
            .ok()
            .is_some_and(|ip| ip.is_loopback())
    {
        "loopback"
    } else {
        host
    };
    Some((
        url.scheme().into(),
        host.into(),
        url.port_or_known_default()?,
    ))
}

pub(crate) fn is_gateway(url: &str, gateways: &[String]) -> bool {
    let candidate = origin(url);
    candidate.is_some() && gateways.iter().any(|gateway| origin(gateway) == candidate)
}

fn parse_candidates(
    request: &ImportPlanRequest,
    document: &Value,
    auth: &Value,
    gateways: &[String],
) -> Result<Vec<PrivateCandidate>, String> {
    let providers = match request.source {
        ModelSourceKind::OpenCode => document.get("provider"),
        ModelSourceKind::Codex => document.get("model_providers"),
        ModelSourceKind::KimiCode => document.get("providers"),
        ModelSourceKind::KimiDesktop | ModelSourceKind::KimiWork => {
            return Err("subscription_source_adapter_not_supported".into())
        }
        ModelSourceKind::Manual => return Err("manual_connections_use_save_connection".into()),
        ModelSourceKind::NolaChatgpt => return Err("siwc_requires_owned_authorization".into()),
    };
    let mut entries = providers
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if request.source == ModelSourceKind::Codex
        && text(document, "model_provider").unwrap_or("openai") == "openai"
    {
        entries.entry("openai").or_insert_with(|| json!({"name":"OpenAI",
            "base_url":text(document,"openai_base_url").unwrap_or("https://api.openai.com/v1"),
            "wire_api":"responses", "requires_openai_auth":true}));
    }
    if entries.len() > 256 {
        return Err("source_has_too_many_providers".into());
    }
    let mut result = vec![];
    for (provider_id, provider) in entries {
        let mut issue = None;
        let (base_url, protocol, secret) = match request.source {
            ModelSourceKind::OpenCode => {
                let options = &provider["options"];
                let protocol = match text(&provider, "npm") {
                    Some("@ai-sdk/openai-compatible") => Some(ModelProtocol::ChatCompletions),
                    Some("@ai-sdk/openai") => Some(ModelProtocol::Responses),
                    Some("@ai-sdk/anthropic") => Some(ModelProtocol::Anthropic),
                    _ => None,
                };
                if options
                    .get("headers")
                    .is_some_and(|v| v.as_object().is_some_and(|m| !m.is_empty()))
                {
                    issue = Some("custom_headers_require_source_adapter".into());
                }
                let auth_entry = &auth[&provider_id];
                let secret = literal_secret(text(options, "apiKey"), request).or_else(|| {
                    (auth_entry["type"].as_str() == Some("api"))
                        .then(|| literal_secret(text(auth_entry, "key"), request))
                        .flatten()
                });
                if auth_entry["type"].as_str() == Some("oauth") {
                    issue = Some("subscription_credential_reuse_not_supported".into());
                }
                (text(options, "baseURL"), protocol, secret)
            }
            ModelSourceKind::Codex => {
                let protocol = match text(&provider, "wire_api").unwrap_or("responses") {
                    "responses" => Some(ModelProtocol::Responses),
                    _ => None,
                };
                if ["auth", "http_headers", "env_http_headers", "query_params"]
                    .iter()
                    .any(|key| provider.get(*key).is_some())
                {
                    issue = Some("custom_auth_or_headers_not_supported".into());
                }
                let uses_account = provider["requires_openai_auth"].as_bool() == Some(true)
                    || (provider_id == "openai" && provider["requires_openai_auth"].as_bool() != Some(false));
                let subscription = uses_account && match auth["auth_mode"].as_str() {
                    Some("chatgpt") => true,
                    Some("apikey" | "api_key") => false,
                    _ => auth["tokens"].as_object().is_some_and(|tokens|
                        ["access_token", "refresh_token"].iter().any(|field|
                            tokens.get(*field).and_then(Value::as_str).is_some_and(|value| !value.is_empty()))),
                };
                // The selected source mode owns credential meaning. A stale
                // API-key field must not turn a subscription login into API auth.
                let secret = if subscription { None } else {
                    literal_secret(text(&provider, "experimental_bearer_token"), request)
                        .or_else(|| text(&provider, "env_key").and_then(|name| environment(name, request)))
                        .or_else(|| uses_account
                            .then(|| literal_secret(text(auth, "OPENAI_API_KEY"), request)).flatten())
                };
                if secret.is_none() && issue.is_none() {
                    if subscription {
                        issue = Some("subscription_credential_reuse_not_supported".into());
                    } else if uses_account || text(&provider, "env_key").is_some()
                        || text(&provider, "experimental_bearer_token").is_some() {
                        issue = Some("api_credential_required".into());
                    }
                }
                (text(&provider, "base_url"), protocol, secret)
            }
            ModelSourceKind::KimiCode => {
                let protocol = match text(&provider, "type") {
                    Some("kimi") | Some("openai") => Some(ModelProtocol::ChatCompletions),
                    Some("openai_responses") => Some(ModelProtocol::Responses),
                    Some("anthropic") => Some(ModelProtocol::Anthropic),
                    _ => None,
                };
                if provider
                    .get("custom_headers")
                    .is_some_and(|v| v.as_object().is_some_and(|m| !m.is_empty()))
                {
                    issue = Some("custom_headers_require_source_adapter".into());
                }
                if provider.get("oauth").is_some() {
                    issue = Some("subscription_credential_reuse_not_supported".into());
                }
                let secret = literal_secret(text(&provider, "api_key"), request)
                    .or_else(|| {
                        text(&provider, "api_key_env").and_then(|name| environment(name, request))
                    })
                    .or_else(|| {
                        (text(&provider, "type") == Some("kimi"))
                            .then(|| {
                                literal_secret(text(&provider["env"], "KIMI_API_KEY"), request)
                            })
                            .flatten()
                    });
                (
                    text(&provider, "base_url").or_else(|| {
                        (text(&provider, "type") == Some("kimi"))
                            .then(|| text(&provider["env"], "KIMI_BASE_URL"))
                            .flatten()
                    }),
                    protocol,
                    secret,
                )
            }
            _ => unreachable!(),
        };
        if protocol.is_none() {
            issue = Some("explicit_supported_protocol_required".into());
        }
        let mut base_url = base_url.unwrap_or("").to_owned();
        // Invalid URLs may contain credentials; they must never enter preview JSON.
        let safe_url = url::Url::parse(&base_url).ok().is_some_and(|url| {
            matches!(url.scheme(), "http" | "https")
                && url.host_str().is_some()
                && url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none()
        });
        if !safe_url {
            base_url.clear();
            issue = Some("invalid_source_connection_url".into());
        }
        if is_gateway(&base_url, gateways) {
            issue = Some("nola_gateway_loop".into());
        }
        let source_id = format!(
            "{:x}",
            Sha256::digest(format!("{}\0{}", request.path, provider_id).as_bytes())
        );
        let connection_id = format!("import-{}", &source_id[..32]);
        let connection = ModelConnection {
            id: connection_id.clone(),
            display_name: text(&provider, "name").unwrap_or(&provider_id).into(),
            base_url,
            protocols: protocol.into_iter().collect(),
            account_id: None,
            source: request.source,
            source_id: Some(source_id),
            enabled: true,
        };
        if connection.validate().is_err() && issue.is_none() {
            issue = Some("invalid_source_connection".into());
        }
        let mut models = vec![];
        if let Some(protocol) = protocol {
            match request.source {
                ModelSourceKind::OpenCode => {
                    if let Some(entries) = provider["models"].as_object() {
                        for (id, config) in entries {
                            models.push(make_model(id, config, protocol));
                        }
                    }
                }
                ModelSourceKind::Codex => {
                    let default_provider = text(document, "model_provider").unwrap_or("openai");
                    let mut seen = BTreeSet::new();
                    if default_provider == provider_id {
                        if let Some(id) = text(document, "model") {
                            let mut model = make_model(id, &Value::Null, protocol);
                            model.context_window = document["model_context_window"].as_u64();
                            models.push(model);
                            seen.insert(id);
                        }
                    }
                    // Profiles inherit the top-level provider, not another
                    // profile's route. Read only the explicitly selected file.
                    if let Some(profiles) = document["profiles"].as_object() {
                        for profile in profiles.values() {
                            if text(profile, "model_provider").unwrap_or(default_provider) != provider_id {
                                continue;
                            }
                            if let Some(id) = text(profile, "model") {
                                if seen.insert(id) {
                                    let mut model = make_model(id, &Value::Null, protocol);
                                    model.context_window = profile["model_context_window"].as_u64();
                                    models.push(model);
                                }
                            }
                        }
                    }
                }
                ModelSourceKind::KimiCode => {
                    if let Some(entries) = document["models"].as_object() {
                        for (_, config) in entries {
                            if text(config, "provider") == Some(provider_id.as_str()) {
                                if let Some(id) = text(config, "model") {
                                    let mut model = make_model(id, config, protocol);
                                    model.display_name =
                                        text(config, "display_name").unwrap_or(id).into();
                                    model.output_limit = config["max_output_size"].as_u64();
                                    model.reasoning_levels = config["support_efforts"]
                                        .as_array()
                                        .map(|values| {
                                            values
                                                .iter()
                                                .filter_map(Value::as_str)
                                                .map(str::to_owned)
                                                .collect()
                                        })
                                        .unwrap_or_default();
                                    if let Some(capabilities) = config["capabilities"].as_array() {
                                        let has = |name| {
                                            capabilities
                                                .iter()
                                                .any(|value| value.as_str() == Some(name))
                                        };
                                        model.execution_capabilities =
                                            Some(ModelExecutionCapabilities {
                                                tool_calls: has("tool_use"),
                                                image_input: has("image_in"),
                                                reasoning: has("thinking"),
                                                ..Default::default()
                                            });
                                        if has("image_in") {
                                            model.modalities.push("image".into());
                                        }
                                    }
                                    models.push(model);
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        let credential_status = if secret.is_some() {
            "available"
        } else if !request.include_credentials {
            "not_requested"
        } else {
            "unavailable"
        };
        result.push(PrivateCandidate {
            public: ImportCandidate {
                candidate_id: connection_id,
                connection,
                models,
                credential_status: credential_status.into(),
                issue,
                existing_revision: None,
            },
            secret,
            existing_account: None,
        });
    }
    Ok(result)
}

fn read_plan_source(
    request: &ImportPlanRequest,
    gateways: &[String],
) -> Result<(Vec<PrivateCandidate>, Vec<u8>), String> {
    if !matches!(
        request.source,
        ModelSourceKind::OpenCode | ModelSourceKind::Codex | ModelSourceKind::KimiCode
    ) {
        return Err("source_adapter_not_supported".into());
    }
    let bytes = read_selected(&request.path)?;
    let mut digest = Sha256::new();
    digest.update((bytes.len() as u64).to_be_bytes());
    digest.update(&bytes);
    let auth = if request.include_credentials {
        if let Some(path) = &request.credential_path {
            let auth = read_selected(path)?;
            digest.update((auth.len() as u64).to_be_bytes());
            digest.update(&auth);
            parse_jsonc(&auth)?
        } else {
            Value::Null
        }
    } else {
        Value::Null
    };
    let candidates = parse_candidates(
        request,
        &parse_document(request.source, &bytes)?,
        &auth,
        gateways,
    )?;
    for candidate in &candidates {
        if let Some(secret) = &candidate.secret {
            digest.update((secret.len() as u64).to_be_bytes());
            digest.update(secret.as_bytes());
        }
    }
    Ok((candidates, digest.finalize().to_vec()))
}

pub fn plan_import(
    store: &NolaStore,
    request: ImportPlanRequest,
    gateways: &[String],
) -> Result<ImportPlan, String> {
    let (mut candidates, digest) = read_plan_source(&request, gateways)?;
    let existing: Vec<ModelConnectionSnapshot> = execute(store, ModelCommand::ListConnections)?;
    for candidate in &mut candidates {
        if let Some(existing) = existing.iter().find(|s| {
            s.document.connection.source == request.source
                && s.document.connection.source_id == candidate.public.connection.source_id
        }) {
            candidate.public.connection.id = existing.document.connection.id.clone();
            candidate.public.connection.account_id =
                existing.document.connection.account_id.clone();
            candidate.public.existing_revision = Some(existing.revision);
            if let Some(id) = &existing.document.connection.account_id {
                candidate.existing_account =
                    execute(store, ModelCommand::GetAccount { id: id.clone() })?;
                if candidate.public.issue.as_deref() == Some("api_credential_required")
                    && candidate.public.connection.base_url == existing.document.connection.base_url
                    && candidate.public.connection.protocols == existing.document.connection.protocols
                    && candidate.existing_account.as_ref().is_some_and(|account|
                        account.account.state == ModelAccountState::Ready && account.account.credential_ref.is_some()) {
                    candidate.public.issue = None;
                }
            }
        }
    }
    let now = chrono::Utc::now().timestamp_millis();
    let expires_at_ms = now + PLAN_LIFETIME_MS;
    let plan_id = uuid::Uuid::new_v4().to_string();
    let public = ImportPlan {
        plan_id: plan_id.clone(),
        candidates: candidates.iter().map(|c| c.public.clone()).collect(),
        expires_at_ms,
    };
    let mut plans = PLANS
        .get_or_init(Default::default)
        .lock()
        .map_err(|_| "import_planner_unavailable")?;
    plans.retain(|_, p| p.expires_at_ms > now);
    if plans.len() >= 16 {
        return Err("too_many_pending_import_plans".into());
    }
    plans.insert(
        plan_id,
        PrivatePlan {
            request,
            database: store.path().to_owned(),
            digest,
            candidates,
            expires_at_ms,
        },
    );
    Ok(public)
}

pub fn commit_import(
    store: &NolaStore,
    request: ImportCommitRequest,
    gateways: &[String],
) -> Result<Vec<ImportItemResult>, String> {
    commit_import_with_sink(store, request, gateways, &mut VaultImportCredentialSink)
}

pub(crate) trait ImportCredentialSink {
    fn store(
        &mut self,
        store: &NolaStore,
        connection: &ModelConnection,
        account_id: &str,
        secret: &str,
    ) -> Result<String, String>;
}
struct VaultImportCredentialSink;
impl ImportCredentialSink for VaultImportCredentialSink {
    fn store(
        &mut self,
        store: &NolaStore,
        connection: &ModelConnection,
        _account_id: &str,
        secret: &str,
    ) -> Result<String, String> {
        let reference = format!("account-credential-{}", uuid::Uuid::new_v4());
        let mut vault = HostCredentialVault::open(store.path())?;
        for protocol in &connection.protocols {
            vault.write(
                &format!("{reference}:{}", protocol.as_str()),
                &crate::host_credentials::identity(&connection.base_url, protocol.as_str())?,
                0,
                Some(secret),
                || Ok(()),
            )?;
        }
        Ok(reference)
    }
}

pub(crate) fn commit_import_with_sink(
    store: &NolaStore,
    request: ImportCommitRequest,
    gateways: &[String],
    sink: &mut impl ImportCredentialSink,
) -> Result<Vec<ImportItemResult>, String> {
    let mut identities = BTreeSet::new();
    if request.selections.is_empty()
        || request.selections.len() > 256
        || request
            .selections
            .iter()
            .any(|s| !identities.insert(s.candidate_id.as_str()))
    {
        return Err("invalid_import_selection".into());
    }
    let plan = PLANS
        .get_or_init(Default::default)
        .lock()
        .map_err(|_| "import_planner_unavailable")?
        .remove(&request.plan_id)
        .ok_or("import_plan_expired_or_consumed")?;
    if plan.database != store.path() || plan.expires_at_ms <= chrono::Utc::now().timestamp_millis()
    {
        return Err("import_plan_expired_or_wrong_workspace".into());
    }
    let (_, digest) = read_plan_source(&plan.request, gateways)?;
    if digest != plan.digest {
        return Err("import_source_changed_preview_again".into());
    }
    if request.selections.iter().any(|s| {
        !plan
            .candidates
            .iter()
            .any(|c| c.public.candidate_id == s.candidate_id)
    }) {
        return Err("unknown_import_candidate".into());
    }
    let mut results = vec![];
    for selection in request.selections {
        let candidate = plan
            .candidates
            .iter()
            .find(|c| c.public.candidate_id == selection.candidate_id)
            .expect("checked candidate");
        let outcome = import_candidate(store, candidate, &selection, gateways, sink);
        results.push(ImportItemResult {
            candidate_id: selection.candidate_id,
            connection_id: candidate.public.connection.id.clone(),
            status: if outcome.is_ok() {
                "imported"
            } else {
                "failed"
            }
            .into(),
            error: outcome.err(),
        });
    }
    Ok(results)
}

fn import_candidate(
    store: &NolaStore,
    candidate: &PrivateCandidate,
    selection: &ImportSelection,
    gateways: &[String],
    sink: &mut impl ImportCredentialSink,
) -> Result<(), String> {
    if let Some(issue) = &candidate.public.issue {
        return Err(issue.clone());
    }
    if candidate.public.existing_revision.is_some() && !selection.update_existing {
        return Err("connection_already_imported".into());
    }
    let mut connection = candidate.public.connection.clone();
    if is_gateway(&connection.base_url, gateways) {
        return Err("nola_gateway_loop".into());
    }
    if let Some(name) = &selection.display_name {
        connection.display_name = name.clone();
    }
    connection.validate()?;
    let current: Option<ModelConnectionSnapshot> = execute(
        store,
        ModelCommand::GetConnection {
            id: connection.id.clone(),
        },
    )?;
    if current.as_ref().map(|snapshot| snapshot.revision) != candidate.public.existing_revision {
        return Err("connection_changed_refresh_preview".into());
    }
    let now = chrono::Utc::now().timestamp_millis();
    let mut account_update = None;
    let mut account_revision = None;
    if let Some(secret) = &candidate.secret {
        if candidate
            .existing_account
            .as_ref()
            .is_some_and(|snapshot| snapshot.account.source != connection.source)
        {
            return Err("source_account_identity_changed".into());
        }
        let account_id = candidate
            .existing_account
            .as_ref()
            .map(|snapshot| snapshot.account.id.clone())
            .unwrap_or_else(|| format!("import-account-{}", uuid::Uuid::new_v4()));
        let reference = sink.store(store, &connection, &account_id, secret)?;
        account_update = Some(ModelAccount {
            id: account_id.clone(),
            display_name: connection.display_name.clone(),
            source: connection.source,
            credential_ref: Some(reference),
            state: ModelAccountState::Ready,
        });
        account_revision = candidate
            .existing_account
            .as_ref()
            .map(|snapshot| snapshot.revision);
        connection.account_id = Some(account_id);
    }
    let _: ModelConnectionSnapshot = execute(
        store,
        ModelCommand::ImportConnection {
            input: connection,
            expected_revision: candidate.public.existing_revision,
            account: account_update,
            expected_account_revision: account_revision,
            models: candidate.public.models.clone(),
            enable_models: selection.enable_models,
            updated_at_ms: now,
        },
    )?;
    Ok(())
}

/// Frozen profile identity selects one account and exact protocol binding.
/// Account unavailability is an error, never an anonymous or global-key fallback.
pub fn credential_for_profile(
    store: &NolaStore,
    source: &Value,
    protocol: &str,
) -> Result<Option<String>, String> {
    let Some(account_id) = source["accountID"].as_str() else {
        return Ok(None);
    };
    let account: Option<ModelAccountSnapshot> = execute(
        store,
        ModelCommand::GetAccount {
            id: account_id.into(),
        },
    )?;
    let account = account.ok_or("model_account_unavailable")?.account;
    if account.state != ModelAccountState::Ready {
        return Err("model_account_not_ready".into());
    }
    let reference = account
        .credential_ref
        .ok_or("model_account_credential_missing")?;
    let record = HostCredentialVault::open(store.path())?
        .record(&format!("{reference}:{protocol}"))?
        .ok_or("model_account_credential_missing")?;
    let bound = crate::host_credentials::identity(
        source["baseURL"]
            .as_str()
            .ok_or("model_connection_url_missing")?,
        protocol,
    )?;
    if record.identity != bound {
        return Err("model_account_credential_route_changed".into());
    }
    record
        .secret
        .map(Some)
        .ok_or("model_account_credential_cleared".into())
}

/// Explicit read-only discovery; never sends a model completion as a probe.
#[cfg(test)]
pub fn discover_catalog(
    store: &NolaStore,
    connection_id: &str,
    expected_revision: i64,
    gateways: &[String],
) -> Result<ModelConnectionSnapshot, String> {
    discover_catalog_request(
        store,
        connection_id,
        expected_revision,
        gateways,
        &uuid::Uuid::new_v4().to_string(),
    )
}

pub fn discover_catalog_request(
    store: &NolaStore,
    connection_id: &str,
    expected_revision: i64,
    gateways: &[String],
    request_id: &str,
) -> Result<ModelConnectionSnapshot, String> {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static ACTIVE: AtomicUsize = AtomicUsize::new(0);
    if ACTIVE
        .try_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
            (n < 3).then_some(n + 1)
        })
        .is_err()
    {
        return Err("model_discovery_busy".into());
    }
    struct Permit;
    impl Drop for Permit {
        fn drop(&mut self) {
            ACTIVE.fetch_sub(1, Ordering::SeqCst);
        }
    }
    let _permit = Permit;
    let snapshot: Option<ModelConnectionSnapshot> = execute(
        store,
        ModelCommand::GetConnection {
            id: connection_id.into(),
        },
    )?;
    let snapshot = snapshot.ok_or("model_connection_not_found")?;
    let operation = crate::model_catalog_discovery::Operation::begin_catalog(
        store,
        request_id,
        connection_id,
        expected_revision,
    )?;
    if snapshot.revision != expected_revision {
        return Err("model_connection_revision_conflict".into());
    }
    let connection = snapshot.document.connection;
    if !connection.enabled {
        return Err("model_connection_disabled".into());
    }
    if is_gateway(&connection.base_url, gateways) {
        return Err("nola_gateway_loop".into());
    }
    let protocol = connection
        .protocols
        .first()
        .copied()
        .ok_or("model_connection_protocol_missing")?;
    let secret = credential_for_profile(
        store,
        &json!({"accountID":connection.account_id,"baseURL":connection.base_url}),
        protocol.as_str(),
    )?;
    let bytes = operation.fetch(
        &format!("{}/models", connection.base_url.trim_end_matches('/')),
        protocol.as_str(),
        secret.as_deref(),
    )?;
    let body: Value = serde_json::from_slice(&bytes).map_err(|_| "model_catalog_invalid_json")?;
    let entries = body["data"]
        .as_array()
        .ok_or("model_catalog_invalid_shape")?;
    if entries.len().saturating_mul(connection.protocols.len()) > 4096 {
        return Err("model_catalog_too_large".into());
    }
    let mut seen = BTreeSet::new();
    let mut models = vec![];
    for entry in entries {
        let id = entry["id"]
            .as_str()
            .ok_or("model_catalog_model_id_missing")?;
        if !seen.insert(id) {
            return Err("model_catalog_duplicate_model".into());
        }
        for protocol in &connection.protocols {
            models.push(make_model(id, entry, *protocol));
        }
    }
    operation.commit(|| {
        execute(
            store,
            ModelCommand::RefreshConnectionCatalog {
                connection_id: connection_id.into(),
                expected_revision,
                models,
                observed_at_ms: chrono::Utc::now().timestamp_millis(),
            },
        )
    })
}

#[cfg(test)]
#[path = "model_connection_host_tests.rs"]
mod tests;
