// SPDX-License-Identifier: Apache-2.0
//! Kimi-owned local login management; no OAuth token import or Agent execution.
use crate::{host_credentials::HostCredentialVault, store::NolaStore};
use nola_application::{ApplicationCommand, ModelCommand};
use nola_domain::model_connection::{
    ModelAccount, ModelAccountSnapshot, ModelAccountState, ModelSourceKind,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    io::Read,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};

const IDENTITY: &str = "kimi-local-server-owner";
const PROVIDER: &str = "managed:kimi-code";
const LOGIN: &str = "/api/v1/oauth/login";
#[cfg(test)]
#[path = "model_kimi_login_tests.rs"]
mod tests;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Connect {
    pub account_id: String,
    pub expected_revision: Option<i64>,
    pub display_name: String,
    pub origin: String,
    pub server_bearer: String,
    pub acknowledge_local_service_access: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Operation {
    pub account_id: String,
    pub expected_revision: i64,
    pub server_id: String,
    pub request_id: String,
    pub flow_id: Option<String>,
    pub region: Option<String>,
    pub acknowledge_provider_login_changes: bool,
}
#[derive(Serialize)]
pub struct Status {
    pub account: ModelAccountSnapshot,
    pub origin: String,
    pub server_id: String,
    pub state: String,
    pub flow_id: Option<String>,
    pub verification_url: Option<String>,
    pub user_code: Option<String>,
    pub poll_interval_seconds: Option<u64>,
    pub inference_supported: bool,
    pub cancellation_confirmed: Option<bool>,
}
#[derive(Serialize)]
pub struct OwnerSummary {
    pub account_id: String,
    pub revision: i64,
    pub display_name: String,
    pub origin: String,
    pub server_id: String,
}
#[derive(Serialize, Deserialize)]
struct Owner {
    origin: String,
    server_id: String,
    server_bearer: String,
}
struct LocalOperations {
    used: BTreeMap<String, Instant>,
}
type Locks = BTreeMap<String, Arc<Mutex<LocalOperations>>>;
static OWNERS: OnceLock<Mutex<Locks>> = OnceLock::new();
fn owner_lock(origin: &str) -> Result<Arc<Mutex<LocalOperations>>, String> {
    let mut locks = OWNERS
        .get_or_init(Mutex::default)
        .lock()
        .map_err(|_| "kimi_owner_busy")?;
    if !locks.contains_key(origin) && locks.len() >= 32 {
        return Err("kimi_owner_budget_exhausted".into());
    }
    Ok(locks
        .entry(origin.into())
        .or_insert_with(|| {
            Arc::new(Mutex::new(LocalOperations {
                used: BTreeMap::new(),
            }))
        })
        .clone())
}
fn origin(value: &str) -> Result<String, String> {
    let url = reqwest::Url::parse(value).map_err(|_| "kimi_origin_invalid")?;
    if !matches!(url.scheme(), "http" | "https")
        || !matches!(url.host_str(), Some("127.0.0.1" | "[::1]"))
        || url.port().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
    {
        return Err("kimi_literal_loopback_origin_required".into());
    }
    Ok(url.origin().ascii_serialization())
}
fn transport(
    owner: &Owner,
    method: reqwest::Method,
    path: &str,
    body: Option<Value>,
    envelope: bool,
) -> Result<Value, String> {
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|_| "kimi_transport_unavailable")?;
    let mut request = client
        .request(method, format!("{}{path}", owner.origin))
        .bearer_auth(&owner.server_bearer);
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request.send().map_err(|_| "kimi_transport_failed")?;
    if !response.status().is_success() {
        return Err("kimi_service_rejected".into());
    }
    let mut bytes = Vec::new();
    response
        .take(2 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "kimi_response_failed")?;
    if bytes.len() > 2 * 1024 * 1024 {
        return Err("kimi_response_too_large".into());
    }
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| "kimi_response_invalid")?;
    if !envelope {
        return Ok(value);
    }
    if value["code"].as_i64() != Some(0) || value.get("data").is_none() {
        return Err("kimi_service_rejected".into());
    }
    Ok(value["data"].clone())
}
fn get(owner: &Owner, path: &str) -> Result<Value, String> {
    transport(owner, reqwest::Method::GET, path, None, true)
}
fn server(owner: &Owner) -> Result<String, String> {
    let meta = get(owner, "/api/v1/meta")?;
    if meta["dangerous_bypass_auth"] != false {
        return Err("kimi_authenticated_service_required".into());
    }
    meta["server_id"]
        .as_str()
        .filter(|id| !id.is_empty() && id.len() <= 256)
        .map(str::to_owned)
        .ok_or("kimi_server_identity_missing".into())
}
fn verify_schema(owner: &Owner) -> Result<(), String> {
    let schema = transport(owner, reqwest::Method::GET, "/openapi.json", None, false)?;
    if !schema["openapi"]
        .as_str()
        .is_some_and(|v| v.starts_with("3."))
    {
        return Err("kimi_protocol_not_supported".into());
    }
    for (path, method) in [
        ("/api/v1/meta", "get"),
        ("/api/v1/auth", "get"),
        (LOGIN, "get"),
        (LOGIN, "post"),
        (LOGIN, "delete"),
    ] {
        if !schema["paths"][path][method].is_object() {
            return Err("kimi_protocol_not_supported".into());
        }
    }
    Ok(())
}
fn current(
    store: &NolaStore,
    id: &str,
    revision: Option<i64>,
) -> Result<Option<ModelAccountSnapshot>, String> {
    let account: Option<ModelAccountSnapshot> =
        store.application()?.execute(ApplicationCommand::Model {
            command: ModelCommand::GetAccount { id: id.into() },
        })?;
    if account.as_ref().map(|a| a.revision) != revision
        || account
            .as_ref()
            .is_some_and(|a| a.account.source != ModelSourceKind::KimiCode)
    {
        return Err("kimi_account_revision_changed".into());
    }
    Ok(account)
}
fn owner(store: &NolaStore, account: &ModelAccountSnapshot) -> Result<Owner, String> {
    if account.account.state == ModelAccountState::Disconnected {
        return Err("kimi_owner_disconnected".into());
    }
    let reference = account
        .account
        .credential_ref
        .as_deref()
        .filter(|v| v.starts_with("kimi-owner-"))
        .ok_or("kimi_owner_missing")?;
    let record = HostCredentialVault::open(store.path())?
        .record(reference)?
        .filter(|v| v.identity == IDENTITY)
        .ok_or("kimi_owner_missing")?;
    let owner: Owner = serde_json::from_str(record.secret.as_deref().ok_or("kimi_owner_missing")?)
        .map_err(|_| "kimi_owner_invalid")?;
    if origin(&owner.origin)? != owner.origin {
        return Err("kimi_owner_invalid".into());
    }
    Ok(owner)
}
pub fn owners(store: &NolaStore) -> Result<Vec<OwnerSummary>, String> {
    let accounts: Vec<ModelAccountSnapshot> =
        store.application()?.execute(ApplicationCommand::Model {
            command: ModelCommand::ListAccounts,
        })?;
    let mut result = Vec::new();
    for account in accounts {
        if account.account.source != ModelSourceKind::KimiCode
            || !account
                .account
                .credential_ref
                .as_deref()
                .is_some_and(|v| v.starts_with("kimi-owner-"))
        {
            continue;
        }
        let Ok(owner) = owner(store, &account) else {
            continue;
        };
        result.push(OwnerSummary {
            account_id: account.account.id,
            revision: account.revision,
            display_name: account.account.display_name,
            origin: owner.origin,
            server_id: owner.server_id,
        });
    }
    Ok(result)
}
fn auth_state(owner: &Owner) -> Result<String, String> {
    let auth = get(owner, "/api/v1/auth")?;
    if auth["managed_provider"].is_null() {
        return Ok("unauthenticated".into());
    }
    if auth["managed_provider"]["name"] != PROVIDER {
        return Err("kimi_provider_changed".into());
    }
    match auth["managed_provider"]["status"].as_str() {
        Some(value @ ("authenticated" | "expired" | "revoked" | "unauthenticated")) => {
            Ok(value.into())
        }
        _ => Err("kimi_auth_state_invalid".into()),
    }
}
fn status(account: ModelAccountSnapshot, owner: &Owner, state: String) -> Status {
    Status {
        account,
        origin: owner.origin.clone(),
        server_id: owner.server_id.clone(),
        state,
        flow_id: None,
        verification_url: None,
        user_code: None,
        poll_interval_seconds: None,
        inference_supported: false,
        cancellation_confirmed: None,
    }
}
pub fn connect(store: &NolaStore, request: Connect) -> Result<Status, String> {
    if !request.acknowledge_local_service_access
        || request.server_bearer.is_empty()
        || request.server_bearer.len() > 16_384
        || request.server_bearer.chars().any(char::is_control)
    {
        return Err("kimi_connection_confirmation_required".into());
    }
    current(store, &request.account_id, request.expected_revision)?;
    let mut owner = Owner {
        origin: origin(&request.origin)?,
        server_id: String::new(),
        server_bearer: request.server_bearer,
    };
    let lock = owner_lock(&owner.origin)?;
    let _guard = lock.try_lock().map_err(|_| "kimi_owner_busy")?;
    owner.server_id = server(&owner)?;
    verify_schema(&owner)?;
    let state = auth_state(&owner)?;
    let handle = format!("kimi-owner-{}", uuid::Uuid::new_v4());
    let input = ModelAccount {
        id: request.account_id.clone(),
        display_name: request.display_name,
        source: ModelSourceKind::KimiCode,
        credential_ref: Some(handle.clone()),
        state: ModelAccountState::Unavailable,
    };
    input.validate()?;
    HostCredentialVault::open(store.path())?.write(
        &handle,
        IDENTITY,
        0,
        Some(&serde_json::to_string(&owner).map_err(|_| "kimi_owner_invalid")?),
        || current(store, &request.account_id, request.expected_revision).map(|_| ()),
    )?;
    let saved: Result<ModelAccountSnapshot, String> =
        store.application()?.execute(ApplicationCommand::Model {
            command: ModelCommand::SaveAccount {
                input,
                expected_revision: request.expected_revision,
                updated_at_ms: chrono::Utc::now().timestamp_millis(),
            },
        });
    match saved {
        Ok(account) => Ok(status(account, &owner, state)),
        Err(error) => {
            // Unique newly created private slot only; never source OAuth data.
            let referenced: Option<ModelAccountSnapshot> =
                store.application()?.execute(ApplicationCommand::Model {
                    command: ModelCommand::GetAccount {
                        id: request.account_id,
                    },
                })?;
            if referenced
                .as_ref()
                .and_then(|a| a.account.credential_ref.as_deref())
                != Some(handle.as_str())
            {
                let _ = HostCredentialVault::open(store.path())
                    .and_then(|mut v| v.purge_deleted_endpoint(&handle));
            }
            Err(error)
        }
    }
}
fn verification_url(value: &Value) -> Result<String, String> {
    let text = value
        .as_str()
        .filter(|v| v.len() <= 16_384)
        .ok_or("kimi_verification_url_invalid")?;
    let url = reqwest::Url::parse(text).map_err(|_| "kimi_verification_url_invalid")?;
    if url.scheme() != "https"
        || !matches!(
            url.host_str(),
            Some(
                "auth.kimi.com"
                    | "auth.kimi.ai"
                    | "www.kimi.com"
                    | "www.kimi.ai"
                    | "kimi.com"
                    | "kimi.ai"
            )
        )
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
    {
        return Err("kimi_verification_url_invalid".into());
    }
    Ok(text.into())
}
fn flow(mut result: Status, value: &Value, expected: Option<&str>) -> Result<Status, String> {
    if value["provider"] != PROVIDER {
        return Err("kimi_provider_changed".into());
    }
    let id = value["flow_id"]
        .as_str()
        .filter(|v| !v.is_empty() && v.len() <= 256)
        .ok_or("kimi_flow_invalid")?;
    if expected.is_some_and(|expected| expected != id) {
        return Err("kimi_flow_changed".into());
    }
    result.flow_id = Some(id.into());
    result.state = match value["status"].as_str() {
        Some(value @ ("pending" | "authenticated" | "denied" | "expired" | "cancelled")) => {
            value.into()
        }
        _ => return Err("kimi_flow_invalid".into()),
    };
    if result.state == "pending" {
        result.verification_url = Some(verification_url(&value["verification_uri_complete"])?);
        result.user_code = Some(
            value["user_code"]
                .as_str()
                .filter(|v| !v.is_empty() && v.len() <= 256)
                .ok_or("kimi_flow_invalid")?
                .into(),
        );
        result.poll_interval_seconds = Some(
            value["interval"]
                .as_u64()
                .filter(|v| *v >= 1 && *v <= 300)
                .ok_or("kimi_flow_invalid")?,
        );
    }
    Ok(result)
}
pub fn operate(store: &NolaStore, action: &str, request: Operation) -> Result<Status, String> {
    if !matches!(action, "start" | "status" | "cancel")
        || uuid::Uuid::parse_str(&request.request_id).is_err()
    {
        return Err("kimi_operation_invalid".into());
    }
    let account = current(store, &request.account_id, Some(request.expected_revision))?
        .ok_or("kimi_account_missing")?;
    let owner = owner(store, &account)?;
    if owner.server_id != request.server_id {
        return Err("kimi_server_changed".into());
    }
    let lock = owner_lock(&owner.origin)?;
    let mut guard = lock.try_lock().map_err(|_| "kimi_owner_busy")?;
    if server(&owner)? != owner.server_id {
        return Err("kimi_server_changed".into());
    }
    let path = format!("{LOGIN}?provider=managed%3Akimi-code");
    let baseline = status(account, &owner, "unknown".into());
    let result = if action == "start" {
        if !request.acknowledge_provider_login_changes || request.flow_id.is_some() {
            return Err("kimi_login_confirmation_required".into());
        }
        if request
            .region
            .as_deref()
            .is_some_and(|v| !matches!(v, "mainland-cn" | "global"))
        {
            return Err("kimi_region_invalid".into());
        }
        guard
            .used
            .retain(|_, created| created.elapsed() < Duration::from_secs(1800));
        if guard.used.contains_key(&request.request_id) || guard.used.len() >= 128 {
            return Err("kimi_login_request_reused_or_busy".into());
        }
        let existing = get(&owner, &path)?;
        if !existing.is_null() && existing["status"] == "pending" {
            return Err("kimi_existing_flow_requires_source_management".into());
        }
        current(store, &request.account_id, Some(request.expected_revision))?;
        guard.used.insert(request.request_id, Instant::now());
        let mut body = json!({"provider":PROVIDER});
        if let Some(region) = request.region {
            body["region"] = json!(region);
        }
        let value = transport(&owner, reqwest::Method::POST, LOGIN, Some(body), true)?;
        flow(baseline, &value, None)?
    } else if let Some(id) = request.flow_id.as_deref() {
        let value = get(&owner, &path)?;
        let mut result = flow(baseline, &value, Some(id))?;
        if action == "cancel" {
            if !request.acknowledge_provider_login_changes {
                return Err("kimi_cancel_confirmation_required".into());
            }
            if result.state == "pending" {
                current(store, &request.account_id, Some(request.expected_revision))?;
                let cancelled = transport(&owner, reqwest::Method::DELETE, &path, None, true)?;
                if cancelled["cancelled"].as_bool().is_none() {
                    return Err("kimi_cancel_unconfirmed".into());
                }
                result.cancellation_confirmed = cancelled["cancelled"].as_bool();
                result.state = cancelled["status"]
                    .as_str()
                    .filter(|v| {
                        matches!(
                            *v,
                            "cancelled" | "authenticated" | "denied" | "expired" | "pending"
                        )
                    })
                    .ok_or("kimi_cancel_unconfirmed")?
                    .into();
                result.verification_url = None;
                result.user_code = None;
            } else {
                result.cancellation_confirmed = Some(false);
            }
        }
        result
    } else {
        if action != "status" {
            return Err("kimi_flow_required".into());
        }
        // Reopening Nola may have lost its in-memory flow handle. Observe the
        // selected provider's current flow without claiming Nola created it.
        // Any later cancellation still requires fresh provider-wide consent.
        let observed = get(&owner, &path)?;
        if observed.is_null() {
            status(baseline.account, &owner, auth_state(&owner)?)
        } else {
            flow(baseline, &observed, None)?
        }
    };
    // Status is observational; never changes account material or enables inference.
    current(store, &request.account_id, Some(request.expected_revision))?;
    Ok(result)
}
