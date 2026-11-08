// SPDX-License-Identifier: Apache-2.0
//! Published OpenCode provider OAuth dispatch. Source owns all provider tokens.
use crate::{host_credentials::HostCredentialVault, store::NolaStore};
use nola_application::{ApplicationCommand, ModelCommand};
use nola_domain::model_connection::{
    ModelAccount, ModelAccountSnapshot, ModelAccountState, ModelSourceKind,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    io::Read,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

const IDENTITY: &str = "opencode-local-owner";
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Connect {
    pub account_id: String,
    pub expected_revision: Option<i64>,
    pub display_name: String,
    pub origin: String,
    pub directory: String,
    pub username: String,
    pub password: String,
    pub acknowledge_local_service_access: bool,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountRequest {
    pub account_id: String,
    pub expected_revision: i64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Authorize {
    pub account_id: String,
    pub expected_revision: i64,
    pub request_id: String,
    pub provider: String,
    pub method: usize,
    pub method_binding: String,
    pub inputs: BTreeMap<String, String>,
    pub acknowledge_provider_login_changes: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Callback {
    pub account_id: String,
    pub expected_revision: i64,
    pub request_id: String,
    pub code: Option<String>,
    pub acknowledge_provider_login_changes: bool,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Condition {
    pub key: String,
    pub op: String,
    pub value: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Choice {
    pub label: String,
    pub value: String,
    pub hint: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Prompt {
    Text {
        key: String,
        message: String,
        placeholder: Option<String>,
        when: Option<Condition>,
    },
    Select {
        key: String,
        message: String,
        options: Vec<Choice>,
        when: Option<Condition>,
    },
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Method {
    #[serde(rename = "type")]
    pub kind: String,
    pub label: String,
    #[serde(default)]
    pub prompts: Vec<Prompt>,
}
#[derive(Serialize)]
pub struct MethodView {
    pub provider: String,
    pub method: usize,
    pub binding: String,
    pub declaration: Method,
}
#[derive(Serialize)]
pub struct Connected {
    pub account: ModelAccountSnapshot,
    pub origin: String,
    pub directory: String,
    pub owner_binding: String,
    pub methods: Vec<MethodView>,
    pub inference_supported: bool,
}
#[derive(Serialize)]
pub struct OwnerView {
    pub account_id: String,
    pub revision: i64,
    pub display_name: String,
    pub origin: String,
    pub directory: String,
}
#[derive(Clone, Serialize)]
pub struct Login {
    pub request_id: String,
    pub provider: String,
    pub method: usize,
    pub state: String,
    pub authorization_url: Option<String>,
    pub callback_mode: Option<String>,
    pub instructions: Option<String>,
    pub source_cancellation_supported: bool,
    pub inference_supported: bool,
}
#[derive(Serialize, Deserialize)]
struct Owner {
    origin: String,
    directory: String,
    username: String,
    password: String,
    binding: String,
}
struct Attempt {
    account: AccountRequest,
    db: std::path::PathBuf,
    owner: String,
    method_binding: String,
    created: Instant,
    login: Login,
}
static ATTEMPTS: OnceLock<Mutex<BTreeMap<String, Attempt>>> = OnceLock::new();
fn attempts() -> &'static Mutex<BTreeMap<String, Attempt>> {
    ATTEMPTS.get_or_init(Mutex::default)
}
fn hash(value: &impl Serialize) -> Result<String, String> {
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(value).map_err(|_| "opencode_invalid_data")?)
    ))
}
fn origin(value: &str) -> Result<String, String> {
    let url = reqwest::Url::parse(value).map_err(|_| "opencode_origin_invalid")?;
    if !matches!(url.scheme(), "http" | "https")
        || !matches!(url.host_str(), Some("127.0.0.1" | "[::1]"))
        || url.port().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
    {
        return Err("opencode_literal_loopback_origin_required".into());
    }
    Ok(url.origin().ascii_serialization())
}
fn text(value: &str, limit: usize) -> bool {
    !value.is_empty() && value.len() <= limit && !value.chars().any(char::is_control)
}
fn http(owner: &Owner, path: &str, body: Option<Value>) -> Result<Value, String> {
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|_| "opencode_transport_unavailable")?;
    let url = reqwest::Url::parse(&format!("{}{path}", owner.origin))
        .map_err(|_| "opencode_path_invalid")?;
    let request = if let Some(body) = body {
        client.post(url).json(&body)
    } else {
        client.get(url)
    };
    let response = request
        .query(&[("directory", &owner.directory)])
        .basic_auth(&owner.username, Some(&owner.password))
        .send()
        .map_err(|_| "opencode_transport_failed")?;
    if !response.status().is_success() {
        return Err("opencode_service_rejected".into());
    }
    let mut bytes = Vec::new();
    response
        .take(2 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "opencode_response_failed")?;
    if bytes.len() > 2 * 1024 * 1024 {
        return Err("opencode_response_too_large".into());
    }
    serde_json::from_slice(&bytes).map_err(|_| "opencode_response_invalid".into())
}
fn binding(owner: &Owner) -> Result<String, String> {
    let health = http(owner, "/global/health", None)?;
    if health["healthy"] != true || !health["version"].as_str().is_some_and(|v| text(v, 128)) {
        return Err("opencode_health_invalid".into());
    }
    let paths = http(owner, "/path", None)?;
    for key in ["home", "state", "config", "directory", "worktree"] {
        if !paths[key].as_str().is_some_and(|v| text(v, 4096)) {
            return Err("opencode_owner_invalid".into());
        }
    }
    if paths["directory"] != owner.directory {
        return Err("opencode_directory_changed".into());
    }
    hash(&json!([
        owner.origin,
        owner.directory,
        health["version"],
        paths
    ]))
}
fn schema(owner: &Owner) -> Result<(), String> {
    let value = http(owner, "/doc", None)?;
    if !value["openapi"]
        .as_str()
        .is_some_and(|v| v.starts_with("3."))
    {
        return Err("opencode_protocol_unsupported".into());
    }
    for (path, method) in [
        ("/path", "get"),
        ("/global/health", "get"),
        ("/provider/auth", "get"),
        ("/provider/{providerID}/oauth/authorize", "post"),
        ("/provider/{providerID}/oauth/callback", "post"),
    ] {
        if !value["paths"][path][method].is_object() {
            return Err("opencode_protocol_unsupported".into());
        }
    }
    Ok(())
}
fn prompt_parts(prompt: &Prompt) -> (&str, Option<&Condition>) {
    match prompt {
        Prompt::Text { key, when, .. } | Prompt::Select { key, when, .. } => (key, when.as_ref()),
    }
}
fn validate_method(method: &Method) -> Result<(), String> {
    if !matches!(method.kind.as_str(), "oauth" | "api")
        || !text(&method.label, 1024)
        || method.prompts.len() > 32
    {
        return Err("opencode_method_unsupported".into());
    }
    let mut keys = std::collections::BTreeSet::new();
    for prompt in &method.prompts {
        let (key, condition) = prompt_parts(prompt);
        if !text(key, 128) || !keys.insert(key) {
            return Err("opencode_prompt_invalid".into());
        }
        if let Some(when) = condition {
            if !text(&when.key, 128)
                || !matches!(when.op.as_str(), "eq" | "neq")
                || when.value.len() > 4096
            {
                return Err("opencode_prompt_invalid".into());
            }
        }
        match prompt {
            Prompt::Text {
                message,
                placeholder,
                ..
            } => {
                if !text(message, 4096) || placeholder.as_ref().is_some_and(|v| v.len() > 4096) {
                    return Err("opencode_prompt_invalid".into());
                }
            }
            Prompt::Select {
                message, options, ..
            } => {
                if !text(message, 4096) || options.is_empty() || options.len() > 128 {
                    return Err("opencode_prompt_invalid".into());
                }
                for option in options {
                    if !text(&option.label, 1024)
                        || !text(&option.value, 4096)
                        || option.hint.as_ref().is_some_and(|v| v.len() > 4096)
                    {
                        return Err("opencode_prompt_invalid".into());
                    }
                }
            }
        }
    }
    Ok(())
}
fn methods(owner: &Owner) -> Result<Vec<MethodView>, String> {
    let value = http(owner, "/provider/auth", None)?;
    let map: BTreeMap<String, Vec<Method>> =
        serde_json::from_value(value).map_err(|_| "opencode_methods_invalid")?;
    if map.len() > 256 {
        return Err("opencode_methods_too_large".into());
    }
    let mut result = Vec::new();
    for (provider, list) in map {
        if !text(&provider, 256) || list.len() > 64 {
            return Err("opencode_methods_invalid".into());
        }
        for (method, declaration) in list.into_iter().enumerate() {
            validate_method(&declaration)?;
            result.push(MethodView {
                binding: hash(&declaration)?,
                provider: provider.clone(),
                method,
                declaration,
            });
        }
    }
    Ok(result)
}
fn account(
    store: &NolaStore,
    id: &str,
    revision: Option<i64>,
) -> Result<Option<ModelAccountSnapshot>, String> {
    let result: Option<ModelAccountSnapshot> =
        store.application()?.execute(ApplicationCommand::Model {
            command: ModelCommand::GetAccount { id: id.into() },
        })?;
    if result.as_ref().map(|v| v.revision) != revision
        || result
            .as_ref()
            .is_some_and(|v| v.account.source != ModelSourceKind::OpenCode)
    {
        return Err("opencode_account_changed".into());
    }
    Ok(result)
}
fn owner(store: &NolaStore, account: &ModelAccountSnapshot) -> Result<Owner, String> {
    if account.account.state == ModelAccountState::Disconnected {
        return Err("opencode_owner_disconnected".into());
    }
    let reference = account
        .account
        .credential_ref
        .as_deref()
        .filter(|v| v.starts_with("opencode-owner-"))
        .ok_or("opencode_owner_missing")?;
    let record = HostCredentialVault::open(store.path())?
        .record(reference)?
        .filter(|v| v.identity == IDENTITY)
        .ok_or("opencode_owner_missing")?;
    let owner: Owner =
        serde_json::from_str(record.secret.as_deref().ok_or("opencode_owner_missing")?)
            .map_err(|_| "opencode_owner_invalid")?;
    if origin(&owner.origin)? != owner.origin {
        return Err("opencode_owner_invalid".into());
    }
    Ok(owner)
}
pub fn inspect(store: &NolaStore, request: AccountRequest) -> Result<Connected, String> {
    let saved = account(store, &request.account_id, Some(request.expected_revision))?
        .ok_or("opencode_account_missing")?;
    let owner = owner(store, &saved)?;
    if binding(&owner)? != owner.binding {
        return Err("opencode_owner_changed".into());
    }
    let methods = methods(&owner)?;
    account(store, &request.account_id, Some(request.expected_revision))?;
    Ok(Connected {
        account: saved,
        origin: owner.origin,
        directory: owner.directory,
        owner_binding: owner.binding,
        methods,
        inference_supported: false,
    })
}
pub fn owners(store: &NolaStore) -> Result<Vec<OwnerView>, String> {
    let list: Vec<ModelAccountSnapshot> =
        store.application()?.execute(ApplicationCommand::Model {
            command: ModelCommand::ListAccounts,
        })?;
    let mut result = Vec::new();
    for saved in list {
        if saved.account.source != ModelSourceKind::OpenCode {
            continue;
        }
        let Ok(owner) = owner(store, &saved) else {
            continue;
        };
        result.push(OwnerView {
            account_id: saved.account.id,
            revision: saved.revision,
            display_name: saved.account.display_name,
            origin: owner.origin,
            directory: owner.directory,
        });
    }
    Ok(result)
}
pub fn connect(store: &NolaStore, request: Connect) -> Result<Connected, String> {
    if !request.acknowledge_local_service_access
        || !text(&request.username, 256)
        || request.username.contains(':')
        || !text(&request.password, 16384)
        || !std::path::Path::new(&request.directory).is_absolute()
        || !text(&request.directory, 4096)
    {
        return Err("opencode_connection_confirmation_required".into());
    }
    account(store, &request.account_id, request.expected_revision)?;
    let mut owner = Owner {
        origin: origin(&request.origin)?,
        directory: request.directory,
        username: request.username,
        password: request.password,
        binding: String::new(),
    };
    schema(&owner)?;
    owner.binding = binding(&owner)?;
    let methods = methods(&owner)?;
    let handle = format!("opencode-owner-{}", uuid::Uuid::new_v4());
    let input = ModelAccount {
        id: request.account_id.clone(),
        display_name: request.display_name,
        source: ModelSourceKind::OpenCode,
        credential_ref: Some(handle.clone()),
        state: ModelAccountState::Unavailable,
    };
    input.validate()?;
    HostCredentialVault::open(store.path())?.write(
        &handle,
        IDENTITY,
        0,
        Some(&serde_json::to_string(&owner).map_err(|_| "opencode_owner_invalid")?),
        || account(store, &request.account_id, request.expected_revision).map(|_| ()),
    )?;
    let result: Result<ModelAccountSnapshot, String> =
        store.application()?.execute(ApplicationCommand::Model {
            command: ModelCommand::SaveAccount {
                input,
                expected_revision: request.expected_revision,
                updated_at_ms: chrono::Utc::now().timestamp_millis(),
            },
        });
    let saved = match result {
        Ok(saved) => saved,
        Err(error) => {
            let saved: Option<ModelAccountSnapshot> =
                store.application()?.execute(ApplicationCommand::Model {
                    command: ModelCommand::GetAccount {
                        id: request.account_id,
                    },
                })?;
            if saved
                .as_ref()
                .and_then(|v| v.account.credential_ref.as_deref())
                != Some(handle.as_str())
            {
                let _ = HostCredentialVault::open(store.path())
                    .and_then(|mut v| v.purge_deleted_endpoint(&handle));
            }
            return Err(error);
        }
    };
    Ok(Connected {
        account: saved,
        origin: owner.origin,
        directory: owner.directory,
        owner_binding: owner.binding,
        methods,
        inference_supported: false,
    })
}
fn inputs(method: &Method, values: &BTreeMap<String, String>) -> Result<(), String> {
    if values.len() > 32 || values.values().any(|v| v.len() > 4096) {
        return Err("opencode_inputs_invalid".into());
    }
    let mut active = std::collections::BTreeSet::new();
    for prompt in &method.prompts {
        let (key, condition) = prompt_parts(prompt);
        let visible = condition
            .map(|when| {
                let equal = values.get(&when.key) == Some(&when.value);
                if when.op == "eq" {
                    equal
                } else {
                    !equal
                }
            })
            .unwrap_or(true);
        if !visible {
            continue;
        }
        active.insert(key);
        let value = values.get(key).ok_or("opencode_prompt_input_required")?;
        if let Prompt::Select { options, .. } = prompt {
            if !options.iter().any(|v| v.value == *value) {
                return Err("opencode_select_value_invalid".into());
            }
        }
    }
    if values.keys().any(|key| !active.contains(key.as_str())) {
        return Err("opencode_hidden_or_unknown_input".into());
    }
    Ok(())
}
fn provider_path(provider: &str, action: &str) -> Result<String, String> {
    if matches!(provider, "." | "..") || provider.is_empty() {
        return Err("opencode_provider_invalid".into());
    }
    let mut url = reqwest::Url::parse("http://127.0.0.1/").map_err(|_| "opencode_path_invalid")?;
    url.path_segments_mut()
        .map_err(|_| "opencode_path_invalid")?
        .clear()
        .extend(["provider", provider, "oauth", action]);
    Ok(url.path().into())
}
fn db(store: &NolaStore) -> Result<std::path::PathBuf, String> {
    std::fs::canonicalize(store.path()).map_err(|_| "opencode_owner_unavailable".into())
}
pub fn authorize(store: &NolaStore, request: Authorize) -> Result<Login, String> {
    if !request.acknowledge_provider_login_changes
        || uuid::Uuid::parse_str(&request.request_id).is_err()
    {
        return Err("opencode_authorize_confirmation_required".into());
    }
    let reference = AccountRequest {
        account_id: request.account_id.clone(),
        expected_revision: request.expected_revision,
    };
    let connected = inspect(store, reference.clone())?;
    let selected = connected
        .methods
        .iter()
        .find(|v| {
            v.provider == request.provider
                && v.method == request.method
                && v.binding == request.method_binding
                && v.declaration.kind == "oauth"
        })
        .ok_or("opencode_method_changed")?;
    inputs(&selected.declaration, &request.inputs)?;
    let owner = owner(store, &connected.account)?;
    let owner_key = format!(
        "{}\n{}\n{}",
        owner.origin, owner.directory, request.provider
    );
    let mut registry = attempts().try_lock().map_err(|_| "opencode_login_busy")?;
    registry.retain(|_, v| v.created.elapsed() < Duration::from_secs(600));
    if registry.len() >= 64
        || registry.contains_key(&request.request_id)
        || registry.values().any(|v| {
            v.owner == owner_key && matches!(v.login.state.as_str(), "pending" | "unknown")
        })
    {
        return Err("opencode_login_pending_or_reused".into());
    }
    let login = Login {
        request_id: request.request_id.clone(),
        provider: request.provider.clone(),
        method: request.method,
        state: "unknown".into(),
        authorization_url: None,
        callback_mode: None,
        instructions: None,
        source_cancellation_supported: false,
        inference_supported: false,
    };
    registry.insert(
        request.request_id.clone(),
        Attempt {
            account: reference,
            db: db(store)?,
            owner: owner_key,
            method_binding: request.method_binding,
            created: Instant::now(),
            login,
        },
    );
    account(store, &request.account_id, Some(request.expected_revision))?;
    let value = http(
        &owner,
        &provider_path(&request.provider, "authorize")?,
        Some(json!({"method":request.method,"inputs":request.inputs})),
    )?;
    let url = value["url"]
        .as_str()
        .filter(|v| v.len() <= 16384)
        .ok_or("opencode_authorization_invalid")?;
    let parsed = reqwest::Url::parse(url).map_err(|_| "opencode_authorization_invalid")?;
    if parsed.scheme() != "https"
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err("opencode_authorization_url_rejected".into());
    }
    let mode = value["method"]
        .as_str()
        .filter(|v| matches!(*v, "auto" | "code"))
        .ok_or("opencode_authorization_invalid")?;
    let instruction = value["instructions"]
        .as_str()
        .filter(|v| v.len() <= 16384)
        .ok_or("opencode_authorization_invalid")?;
    account(store, &request.account_id, Some(request.expected_revision))?;
    let attempt = registry
        .get_mut(&request.request_id)
        .ok_or("opencode_login_missing")?;
    attempt.login.state = "pending".into();
    attempt.login.authorization_url = Some(url.into());
    attempt.login.callback_mode = Some(mode.into());
    attempt.login.instructions = Some(instruction.into());
    Ok(attempt.login.clone())
}

#[cfg(test)]
#[path = "model_opencode_login_tests.rs"]
mod tests;
pub fn callback(store: &NolaStore, request: Callback, forget: bool) -> Result<Login, String> {
    let mut registry = attempts().try_lock().map_err(|_| "opencode_login_busy")?;
    let attempt = registry
        .get_mut(&request.request_id)
        .ok_or("opencode_login_missing")?;
    if attempt.db != db(store)?
        || attempt.account.account_id != request.account_id
        || attempt.account.expected_revision != request.expected_revision
        || attempt.created.elapsed() >= Duration::from_secs(600)
    {
        return Err("opencode_login_binding_changed".into());
    }
    if forget {
        attempt.login.state = "stopped_locally".into();
        attempt.login.authorization_url = None;
        attempt.login.instructions = None;
        return Ok(attempt.login.clone());
    }
    if !request.acknowledge_provider_login_changes || attempt.login.state != "pending" {
        return Err("opencode_callback_confirmation_required".into());
    }
    let connected = inspect(store, attempt.account.clone())?;
    if !connected.methods.iter().any(|v| {
        v.provider == attempt.login.provider
            && v.method == attempt.login.method
            && v.binding == attempt.method_binding
    }) {
        return Err("opencode_method_changed".into());
    }
    let code = request.code.as_deref().filter(|v| !v.is_empty());
    if code.is_some_and(|v| v.len() > 16384)
        || (attempt.login.callback_mode.as_deref() == Some("code") && code.is_none())
        || (attempt.login.callback_mode.as_deref() == Some("auto") && code.is_some())
    {
        return Err("opencode_callback_code_invalid".into());
    }
    let owner = owner(store, &connected.account)?;
    let mut body = json!({"method":attempt.login.method});
    if let Some(code) = code {
        body["code"] = json!(code);
    }
    attempt.login.state = "unknown".into();
    attempt.login.authorization_url = None;
    attempt.login.instructions = None;
    let result = http(
        &owner,
        &provider_path(&attempt.login.provider, "callback")?,
        Some(body),
    )?;
    if result != true {
        return Err("opencode_callback_unconfirmed".into());
    }
    account(store, &request.account_id, Some(request.expected_revision))?;
    attempt.login.state = "source_callback_completed".into();
    Ok(attempt.login.clone())
}
