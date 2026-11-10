// SPDX-License-Identifier: Apache-2.0
//! Authenticated same-protocol model gateway. It never enters the Agent executor,
//! executes returned tools, retries a request or changes accounts after failure.
use crate::model_gateway_usage::GatewayUsageTracker;
use crate::{host_credentials::HostCredentialVault, store::NolaStore};
use nola_application::ModelProfile;
use nola_domain::generation::ModelProtocol;
use nola_domain::model_connection::{GatewayAttemptOutcome, GatewayAttemptRecord};
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::io::{Cursor, Read};
use std::sync::atomic::{AtomicUsize, Ordering};
use tiny_http::{Header, Method, Request, Response, StatusCode};

const TOKEN_KEY: &str = "nola-model-gateway-access";
const TOKEN_IDENTITY: &str = "nola-model-gateway-v1";
const MAX_GATEWAY_BODY_BYTES: u64 = 32 * 1024 * 1024;
static ACTIVE_REQUESTS: AtomicUsize = AtomicUsize::new(0);
struct GatewayPermit;
impl GatewayPermit {
    fn acquire() -> Option<Self> {
        ACTIVE_REQUESTS
            .try_update(Ordering::AcqRel, Ordering::Relaxed, |current| {
                (current < 4).then_some(current + 1)
            })
            .ok()
            .map(|_| Self)
    }
}
impl Drop for GatewayPermit {
    fn drop(&mut self) {
        ACTIVE_REQUESTS.fetch_sub(1, Ordering::AcqRel);
    }
}
struct GatewayBody {
    response: reqwest::blocking::Response,
    usage: GatewayUsageTracker,
    _permit: GatewayPermit,
}
impl Read for GatewayBody {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        match self.response.read(bytes) {
            Ok(0) => {
                self.usage.eof().map_err(std::io::Error::other)?;
                Ok(0)
            }
            Ok(count) => {
                self.usage.observe(&bytes[..count]);
                Ok(count)
            }
            Err(error) => {
                let _ = self.usage.finish(GatewayAttemptOutcome::Interrupted);
                Err(error)
            }
        }
    }
}

pub(crate) fn client_access_token(store: &NolaStore) -> Result<(i64, String), String> {
    let record = HostCredentialVault::open(store.path())?
        .record(TOKEN_KEY)?
        .filter(|record| record.identity == TOKEN_IDENTITY)
        .ok_or("gateway_token_required")?;
    Ok((
        record.revision,
        record.secret.ok_or("gateway_token_required")?,
    ))
}

#[derive(Debug, Serialize)]
pub struct GatewayAccessSnapshot {
    pub revision: i64,
    pub enabled: bool,
}

// Explicit setup response only; never Debug or an Application command receipt.
#[derive(Serialize)]
pub struct GatewayAccessToken {
    pub revision: i64,
    pub token: String,
}

pub fn access_snapshot(store: &NolaStore) -> Result<GatewayAccessSnapshot, String> {
    let _ = store.model_profiles()?;
    let record = HostCredentialVault::open(store.path())?.record(TOKEN_KEY)?;
    Ok(GatewayAccessSnapshot {
        revision: record.as_ref().map_or(0, |r| r.revision),
        enabled: record.is_some_and(|r| r.identity == TOKEN_IDENTITY && r.secret.is_some()),
    })
}

pub fn issue_access_token(
    store: &NolaStore,
    expected_revision: i64,
) -> Result<GatewayAccessToken, String> {
    let _ = store.model_profiles()?;
    let token = format!(
        "nola_gw_{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let revision = HostCredentialVault::open(store.path())?.write(
        TOKEN_KEY,
        TOKEN_IDENTITY,
        expected_revision,
        Some(&token),
        || Ok(()),
    )?;
    Ok(GatewayAccessToken { revision, token })
}

pub fn revoke_access_token(
    store: &NolaStore,
    expected_revision: i64,
) -> Result<GatewayAccessSnapshot, String> {
    let revision = HostCredentialVault::open(store.path())?.write(
        TOKEN_KEY,
        TOKEN_IDENTITY,
        expected_revision,
        None,
        || Ok(()),
    )?;
    Ok(GatewayAccessSnapshot {
        revision,
        enabled: false,
    })
}

fn authenticated(request: &Request, store: &NolaStore) -> Option<i64> {
    let tokens: Vec<&str> = request
        .headers()
        .iter()
        .filter_map(|header| {
            if header.field.equiv("Authorization") {
                header.value.as_str().strip_prefix("Bearer ")
            } else if header.field.equiv("x-api-key") {
                Some(header.value.as_str())
            } else {
                None
            }
        })
        .collect();
    if tokens.len() != 1 || tokens[0].len() > 1024 {
        return None;
    }
    let Ok(Some(record)) =
        HostCredentialVault::open(store.path()).and_then(|vault| vault.record(TOKEN_KEY))
    else {
        return None;
    };
    let Some(expected) = record.secret.filter(|_| record.identity == TOKEN_IDENTITY) else {
        return None;
    };
    let left = Sha256::digest(tokens[0].as_bytes());
    let right = Sha256::digest(expected.as_bytes());
    (left
        .iter()
        .zip(right.iter())
        .fold(0u8, |difference, (a, b)| difference | (a ^ b))
        == 0)
        .then_some(record.revision)
}

pub fn model_id(profile: &ModelProfile) -> String {
    format!("nola/{}/{}", profile.id, profile.revision)
}

fn parse_model_id(value: &str) -> Result<(&str, i64), &'static str> {
    let value = value
        .strip_prefix("nola/")
        .ok_or("gateway_model_identity_required")?;
    let (id, revision) = value
        .rsplit_once('/')
        .ok_or("gateway_model_identity_required")?;
    let revision = revision
        .parse::<i64>()
        .map_err(|_| "gateway_model_revision_invalid")?;
    if id.is_empty() || revision < 1 {
        return Err("gateway_model_revision_invalid");
    }
    Ok((id, revision))
}

fn json_response(status: u16, value: Value) -> Response<Box<dyn Read + Send>> {
    let bytes = value.to_string().into_bytes();
    let length = bytes.len();
    Response::new(
        StatusCode(status),
        vec![
            Header::from_bytes("Content-Type", "application/json").unwrap(),
            Header::from_bytes("Cache-Control", "no-store").unwrap(),
        ],
        Box::new(Cursor::new(bytes)),
        Some(length),
        None,
    )
}

fn error(status: u16, code: &str) -> Response<Box<dyn Read + Send>> {
    json_response(
        status,
        json!({"error":{"type":"nola_gateway_error","code":code,"message":code}}),
    )
}

fn available(profile: &ModelProfile) -> bool {
    profile.runtime == "remote"
        && profile.enabled
        && profile.deleted_at_ms.is_none()
        && serde_json::from_str::<Value>(&profile.source_json)
            .ok()
            .is_some_and(|source| {
                source["endpointEnabled"] != false && source["modelEnabled"] != false
                    && source["wireProtocol"] != "siwc_responses" && source["sourceKind"] != "nola_chatgpt"
            })
}

fn target_profile(
    store: &NolaStore,
    model: &str,
    protocol: ModelProtocol,
) -> Result<ModelProfile, (u16, &'static str)> {
    let (id, revision) = parse_model_id(model).map_err(|error| (400, error))?;
    let profile = store
        .model_profile(id)
        .map_err(|_| (503, "gateway_catalog_unavailable"))?
        .filter(available)
        .ok_or((404, "gateway_model_unavailable"))?;
    if profile.revision != revision {
        return Err((409, "gateway_model_configuration_changed"));
    }
    let source: Value = serde_json::from_str(&profile.source_json)
        .map_err(|_| (422, "gateway_model_configuration_invalid"))?;
    if source["wireProtocol"].as_str() != Some(protocol.as_str()) {
        return Err((400, "gateway_model_protocol_mismatch"));
    }
    Ok(profile)
}

/// Headers/body are streamed directly from the chosen provider. Dropping the
/// reader when the downstream client disconnects also drops the upstream body.
pub fn gateway_response(
    request: &mut Request,
    store: &NolaStore,
    path: &str,
    gateways: &[String],
) -> Response<Box<dyn Read + Send>> {
    let Some(client_access_revision) = authenticated(request, store) else {
        return error(401, "gateway_token_required");
    };
    if request.method() == &Method::Get && path == "/gateway/v1/models" {
        let profiles = match store.model_profiles() {
            Ok(profiles) => profiles,
            Err(_) => return error(503, "gateway_catalog_unavailable"),
        };
        let data:Vec<_> = profiles.iter().filter(|profile| available(profile)).map(|profile| {
            let source:Value = serde_json::from_str(&profile.source_json).unwrap_or(Value::Null);
            json!({"id":model_id(profile),"object":"model","created":profile.created_at_ms / 1000,
                "owned_by":"nola","name":profile.display_name,"nola_protocol":source["wireProtocol"]})
        }).collect();
        return json_response(200, json!({"object":"list","data":data}));
    }
    if request.method() != &Method::Post {
        return error(405, "gateway_method_not_allowed");
    }
    let protocol = match path {
        "/gateway/v1/chat/completions" => ModelProtocol::ChatCompletions,
        "/gateway/v1/responses" => ModelProtocol::Responses,
        "/gateway/v1/messages" => ModelProtocol::Anthropic,
        _ => return error(404, "gateway_route_not_found"),
    };
    let Some(permit) = GatewayPermit::acquire() else {
        return error(429, "gateway_concurrency_limit");
    };
    let mut bytes = vec![];
    if request
        .as_reader()
        .take(MAX_GATEWAY_BODY_BYTES + 1)
        .read_to_end(&mut bytes)
        .is_err()
    {
        return error(400, "gateway_body_read_failed");
    }
    if bytes.len() as u64 > MAX_GATEWAY_BODY_BYTES {
        return error(413, "gateway_body_too_large");
    }
    let mut body: Value = match serde_json::from_slice(&bytes) {
        Ok(Value::Object(map)) => Value::Object(map),
        _ => return error(400, "gateway_body_invalid"),
    };
    let Some(model) = body["model"].as_str() else {
        return error(400, "gateway_model_required");
    };
    let profile = match target_profile(store, model, protocol) {
        Ok(profile) => profile,
        Err((status, code)) => return error(status, code),
    };
    let source: Value = match serde_json::from_str(&profile.source_json) {
        Ok(source) => source,
        Err(_) => return error(422, "gateway_model_configuration_invalid"),
    };
    let Some(endpoint) = source["baseURL"].as_str() else {
        return error(422, "gateway_model_endpoint_missing");
    };
    if crate::model_connection_host::is_gateway(endpoint, gateways) {
        return error(409, "nola_gateway_loop");
    }
    let target = match crate::llm::normalized_model_endpoint(endpoint, protocol) {
        Ok(target) => target,
        Err(_) => return error(422, "gateway_model_endpoint_invalid"),
    };
    if target.query().is_some() {
        return error(422, "gateway_model_endpoint_query_not_allowed");
    }
    let credential = match crate::host_credentials::for_profile(store, &profile, None) {
        Ok(secret) => secret,
        Err(_) => return error(409, "gateway_model_account_unavailable"),
    };
    body["model"] = json!(profile.model_name);
    let client = match reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(300))
        .build()
    {
        Ok(client) => client,
        Err(_) => return error(503, "gateway_transport_unavailable"),
    };
    let mut upstream = client.post(target).json(&body);
    for header in request.headers() {
        if [
            "anthropic-version",
            "anthropic-beta",
            "openai-beta",
            "accept",
        ]
        .iter()
        .any(|name| header.field.equiv(name))
        {
            upstream = upstream.header(header.field.as_str().as_str(), header.value.as_str());
        }
    }
    if protocol == ModelProtocol::Anthropic {
        if !request
            .headers()
            .iter()
            .any(|h| h.field.equiv("anthropic-version"))
        {
            upstream = upstream.header("anthropic-version", "2023-06-01");
        }
        if let Some(secret) = &credential {
            upstream = upstream.header("x-api-key", secret);
        }
    } else if let Some(secret) = &credential {
        upstream = upstream.bearer_auth(secret);
    }
    let Some(connection_id) = source["connectionID"]
        .as_str()
        .or_else(|| source["endpointID"].as_str())
    else {
        return error(422, "gateway_connection_identity_missing");
    };
    let mut usage = match GatewayUsageTracker::start(
        store,
        GatewayAttemptRecord {
            id: uuid::Uuid::new_v4().to_string(),
            profile_id: profile.id.clone(),
            profile_revision: profile.revision,
            connection_id: connection_id.into(),
            account_id: source["accountID"].as_str().map(str::to_owned),
            client_access_revision,
            protocol,
            started_at_ms: chrono::Utc::now().timestamp_millis(),
            finished_at_ms: None,
            outcome: GatewayAttemptOutcome::InFlight,
            http_status: None,
            usage: None,
        },
    ) {
        Ok(usage) => usage,
        Err(_) => return error(503, "gateway_attempt_ledger_unavailable"),
    };
    let response = match upstream.send() {
        Ok(response) => response,
        Err(_) => {
            let _ = usage.finish(GatewayAttemptOutcome::TransportFailed);
            return error(502, "gateway_upstream_transport_failed");
        }
    };
    let status = response.status().as_u16();
    usage.response(
        status,
        response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.starts_with("text/event-stream")),
    );
    let mut headers = vec![
        Header::from_bytes("Cache-Control", "no-store").unwrap(),
        Header::from_bytes("X-Nola-Attempt-ID", usage.id()).unwrap(),
    ];
    for name in ["content-type", "retry-after", "request-id", "x-request-id"] {
        if let Some(value) = response
            .headers()
            .get(name)
            .and_then(|value| value.to_str().ok())
        {
            if let Ok(header) = Header::from_bytes(name, value) {
                headers.push(header);
            }
        }
    }
    Response::new(
        StatusCode(status),
        headers,
        Box::new(GatewayBody {
            response,
            usage,
            _permit: permit,
        }),
        None,
        None,
    )
}

#[cfg(test)]
#[path = "model_gateway_host_tests.rs"]
mod tests;
