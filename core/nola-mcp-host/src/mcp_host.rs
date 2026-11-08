// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Host-owned remote MCP connection and capability discovery.
// - Security: Credentials and endpoints remain Host configuration; Core receives typed descriptors only.

use nola_core::agent::capability::{
    McpPromptArgumentDescriptor, McpPromptDescriptor, McpResourceDescriptor,
    McpResourceTemplateDescriptor, McpServerAvailability, McpServerDescriptor, McpToolDescriptor,
    McpTransportKind,
};
use nola_core::agent::host::{validate_host_artifacts, HostArtifact};
use reqwest::blocking::{Client, Response};
use reqwest::header::{HeaderName, HeaderValue, ACCEPT, CONTENT_TYPE, USER_AGENT};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::Duration;

const DEFAULT_PROTOCOL_VERSION: &str = "2025-06-18";
const DEFAULT_TIMEOUT_MS: u64 = 15_000;
const MAX_RESPONSE_BYTES: u64 = 4 * 1_024 * 1_024;
const MAX_LIST_PAGES: usize = 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpHostError {
    pub code: &'static str,
    pub message: String,
}

#[derive(Clone)]
struct McpConnectionConfig {
    endpoint: String,
    timeout_ms: u64,
    headers: Vec<(HeaderName, HeaderValue)>,
    resource: crate::mcp_connection_authority::ConnectionResource,
    server_id: String,
    display_name: String,
}

struct McpConnectionEntry {
    config: McpConnectionConfig,
    client: Option<StreamableHttpMcpClient>,
    retired: bool,
    generation: Option<String>,
    descriptor: Option<McpServerDescriptor>,
    admission_id: String,
}

#[derive(Debug, Clone)]
pub struct McpToolExecution {
    pub output: Value,
    pub is_error: bool,
    pub artifacts: Vec<HostArtifact>,
}

pub fn mcp_result_artifacts(output: &Value) -> Result<Vec<HostArtifact>, McpHostError> {
    let mut artifacts = output
        .pointer("/_meta/nola~1artifacts")
        .cloned()
        .map(serde_json::from_value::<Vec<HostArtifact>>)
        .transpose()
        .map_err(|error| {
            McpHostError::new(
                "mcp_artifact_invalid",
                format!("Invalid explicit MCP artifact contract: {error}"),
            )
        })?
        .unwrap_or_default();
    if let Some(content) = output.get("content").and_then(Value::as_array) {
        for block in content {
            let resource = match block.get("type").and_then(Value::as_str) {
                Some("resource_link") => Some(block),
                Some("resource") => block.get("resource"),
                _ => None,
            };
            let Some(resource) = resource else {
                continue;
            };
            let Some(uri) = resource
                .get("uri")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|uri| !uri.is_empty())
            else {
                return Err(McpHostError::new(
                    "mcp_artifact_invalid",
                    "MCP resource result is missing uri",
                ));
            };
            let name = resource
                .get("name")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_string)
                .or_else(|| {
                    url::Url::parse(uri).ok().and_then(|url| {
                        url.path_segments()
                            .and_then(|mut segments| segments.next_back())
                            .filter(|name| !name.is_empty())
                            .map(str::to_string)
                    })
                })
                .unwrap_or_else(|| "MCP resource".to_string());
            artifacts.push(HostArtifact {
                uri: uri.to_string(),
                name,
                mime_type: resource
                    .get("mimeType")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                operation: None,
            });
        }
    }
    let mut deduped = Vec::new();
    for artifact in artifacts {
        if !deduped
            .iter()
            .any(|existing: &HostArtifact| existing.uri == artifact.uri)
        {
            deduped.push(artifact);
        }
    }
    validate_host_artifacts(&deduped).map_err(|error| {
        McpHostError::new(
            "mcp_artifact_invalid",
            format!("MCP artifact contract failed validation: {error}"),
        )
    })?;
    Ok(deduped)
}

fn connection_registry() -> &'static RwLock<BTreeMap<String, Arc<Mutex<McpConnectionEntry>>>> {
    static REGISTRY: OnceLock<RwLock<BTreeMap<String, Arc<Mutex<McpConnectionEntry>>>>> =
        OnceLock::new();
    REGISTRY.get_or_init(|| RwLock::new(BTreeMap::new()))
}

fn install_connection(
    connection_ref: String,
    config: McpConnectionConfig,
    client: Option<StreamableHttpMcpClient>,
) -> Result<(), McpHostError> {
    let mut registry = connection_registry().write().map_err(|_| {
        McpHostError::new(
            "mcp_registry_unavailable",
            "MCP connection registry is unavailable",
        )
    })?;
    if let Some(existing) = registry.get(&connection_ref) {
        let mut existing = existing.lock().map_err(|_| {
            McpHostError::new(
                "mcp_connection_unavailable",
                "MCP connection state is unavailable",
            )
        })?;
        let unchanged = existing.config.endpoint == config.endpoint
            && existing.config.timeout_ms == config.timeout_ms
            && existing.config.headers == config.headers
            && existing.config.resource == config.resource;
        if unchanged {
            if client.is_some() {
                existing.client = client;
            }
            return Ok(());
        }
    }
    registry.insert(
        connection_ref,
        Arc::new(Mutex::new(McpConnectionEntry { config, client, retired: false, generation:None, descriptor:None,
            admission_id:uuid::Uuid::new_v4().to_string() })),
    );
    Ok(())
}

pub fn register_configured_servers(values: &[Value]) -> Result<(), McpHostError> {
    for (index, value) in values.iter().enumerate() {
        if value.get("enabled").and_then(Value::as_bool) == Some(false) {
            continue;
        }
        if crate::mcp_stdio::is_stdio_configuration(value) {
            crate::mcp_stdio::register_configured_server(value, index)?;
            continue;
        }
        let Some(endpoint) = value
            .get("endpoint")
            .or_else(|| value.get("url"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        let server_id = configured_server_id(value, index)?;
        let connection_ref = value
            .get("connection_ref")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .ok_or_else(||McpHostError::new("mcp_owner_missing","MCP target requires an owner-bound reference"))?;
        let timeout_ms = value
            .get("timeout_ms")
            .or_else(|| value.get("timeoutMs"))
            .and_then(Value::as_u64)
            .unwrap_or(DEFAULT_TIMEOUT_MS)
            .clamp(1_000, 120_000);
        let config = McpConnectionConfig {
            endpoint: endpoint.to_string(),
            timeout_ms,
            headers: configured_headers(value)?,
            resource: crate::mcp_connection_authority::resource_from_configuration(value)
                .map_err(|message|McpHostError::new("mcp_owner_missing",message))?,
            server_id: server_id.clone(),
            display_name: value.get("display_name").or_else(||value.get("name")).and_then(Value::as_str).unwrap_or(&server_id).into(),
        };
        install_connection(connection_ref, config, None)?;
    }
    Ok(())
}

/// Host-worker operation: revoke new admissions, then drain the exact instance.
pub fn revoke_connection(transport: McpTransportKind, connection_ref: &str) -> Result<McpRetirement, McpHostError> {
    if transport == McpTransportKind::Stdio {
        return Ok(McpRetirement { http: None, _stdio: Some(crate::mcp_stdio::revoke_connection(connection_ref)) });
    }
    if transport != McpTransportKind::StreamableHttp {
        return Err(McpHostError::new("mcp_transport_unsupported", "Cannot retire this MCP transport"));
    }
    let removed = connection_registry().write().unwrap_or_else(|poisoned| poisoned.into_inner()).remove(connection_ref);
    Ok(McpRetirement { http: removed, _stdio: None })
}

/// Holding this token keeps the removed connection alive; dropping it drains.
pub struct McpRetirement {
    http: Option<Arc<Mutex<McpConnectionEntry>>>,
    _stdio: Option<crate::mcp_stdio::StdioRetirement>,
}
impl Drop for McpRetirement {
    fn drop(&mut self) {
        if let Some(entry) = self.http.take() {
            let mut entry = entry.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            entry.retired = true;
            entry.client = None;
        }
    }
}

pub fn call_tool(
    transport: McpTransportKind,
    connection_ref: &str,
    tool_name: &str,
    arguments: Value,
) -> Result<McpToolExecution, McpHostError> {
    let output = request_capability(
        transport,
        connection_ref,
        "tools/call",
        json!({"name": tool_name, "arguments": arguments}),
    )?;
    let is_error = output
        .get("isError")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let artifacts = mcp_result_artifacts(&output)?;
    Ok(McpToolExecution {
        output,
        is_error,
        artifacts,
    })
}

pub fn call_tool_with_cancellation(
    transport: McpTransportKind,
    connection_ref: &str,
    tool_name: &str,
    arguments: Value,
    cancellation: &dyn crate::Cancellation,
) -> Result<McpToolExecution, McpHostError> {
    call_tool_with_cancellation_and_progress(
        transport,
        connection_ref,
        tool_name,
        arguments,
        cancellation,
        &mut |_| {},
    )
}

pub fn call_tool_with_cancellation_and_progress(
    transport: McpTransportKind,
    connection_ref: &str,
    tool_name: &str,
    arguments: Value,
    cancellation: &dyn crate::Cancellation,
    progress: &mut dyn FnMut(&Value),
) -> Result<McpToolExecution, McpHostError> {
    if cancellation.is_requested() {
        return Err(McpHostError::new(
            "mcp_cancelled_before_send",
            "MCP tool call was cancelled before it entered the transport",
        ));
    }
    if transport == McpTransportKind::Stdio {
        return crate::mcp_stdio::call_tool_with_cancellation_and_progress(
            connection_ref,
            tool_name,
            arguments,
            cancellation,
            progress,
        );
    }
    if transport == McpTransportKind::StreamableHttp {
        return call_http_tool_with_cancellation(
            connection_ref,
            tool_name,
            arguments,
            cancellation,
            progress,
        );
    }
    let connection_ref = connection_ref.to_string();
    let tool_name = tool_name.to_string();
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name(format!("nola-mcp-call-{tool_name}"))
        .spawn(move || {
            let result = call_tool(transport, &connection_ref, &tool_name, arguments);
            let _ = sender.send(result);
        })
        .map_err(|error| McpHostError::new("mcp_worker_start_failed", error.to_string()))?;

    loop {
        match receiver.recv_timeout(Duration::from_millis(50)) {
            Ok(result) => return result,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if cancellation.is_requested() {
                    return Err(McpHostError::new(
                        "mcp_cancelled_after_send",
                        "MCP cancellation was requested after the transport worker started; verify remote side effects before retrying",
                    ));
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                return Err(McpHostError::new(
                    "mcp_worker_failed",
                    "MCP transport worker exited without a result",
                ));
            }
        }
    }
}

fn call_http_tool_with_cancellation(
    connection_ref: &str,
    tool_name: &str,
    arguments: Value,
    cancellation: &dyn crate::Cancellation,
    progress: &mut dyn FnMut(&Value),
) -> Result<McpToolExecution, McpHostError> {
    let connection = connection_registry()
        .read()
        .map_err(|_| {
            McpHostError::new(
                "mcp_registry_unavailable",
                "MCP connection registry is unavailable",
            )
        })?
        .get(connection_ref)
        .cloned()
        .ok_or_else(|| {
            McpHostError::new(
                "mcp_connection_unavailable",
                format!("MCP HTTP connection `{connection_ref}` is not registered by this Host"),
            )
        })?;
    let mut connection = connection.lock().map_err(|_| {
        McpHostError::new(
            "mcp_connection_unavailable",
            "MCP connection state is unavailable",
        )
    })?;
    if connection.retired {
        return Err(McpHostError::new("mcp_connection_retired", "MCP connection was retired"));
    }
    if connection.client.is_none() { return Err(McpHostError::new("mcp_connection_required",
        "The MCP connection is not active; call mcp_connect and obtain any required approval")); }
    let result = connection
        .client
        .as_mut()
        .expect("MCP HTTP client initialized")
        .request_with_cancellation(
            "tools/call",
            json!({"name":tool_name,"arguments":arguments}),
            cancellation,
            progress,
        );
    let output = match result {
        Ok(output) => output,
        Err(error) => {
            // The active request may still produce a response after this
            // method returns. A fresh session prevents that response from
            // becoming input to the next invocation.
            connection.client = None;
            return Err(error);
        }
    };
    let is_error = output
        .get("isError")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let artifacts = mcp_result_artifacts(&output)?;
    Ok(McpToolExecution {
        output,
        is_error,
        artifacts,
    })
}

pub fn read_resource(
    transport: McpTransportKind,
    connection_ref: &str,
    uri: &str,
) -> Result<McpToolExecution, McpHostError> {
    let output = request_capability(
        transport,
        connection_ref,
        "resources/read",
        json!({"uri": uri}),
    )?;
    let artifacts = mcp_result_artifacts(&output)?;
    Ok(McpToolExecution {
        output,
        is_error: false,
        artifacts,
    })
}

pub fn get_prompt(
    transport: McpTransportKind,
    connection_ref: &str,
    prompt_name: &str,
    arguments: BTreeMap<String, String>,
) -> Result<McpToolExecution, McpHostError> {
    let output = request_capability(
        transport,
        connection_ref,
        "prompts/get",
        json!({"name": prompt_name, "arguments": arguments}),
    )?;
    let artifacts = mcp_result_artifacts(&output)?;
    Ok(McpToolExecution {
        output,
        is_error: false,
        artifacts,
    })
}

fn request_capability(
    transport: McpTransportKind,
    connection_ref: &str,
    method: &str,
    params: Value,
) -> Result<Value, McpHostError> {
    if transport == McpTransportKind::Stdio {
        return crate::mcp_stdio::request_capability(connection_ref, method, params);
    }
    if transport != McpTransportKind::StreamableHttp {
        return Err(McpHostError::new(
            "mcp_transport_unsupported",
            format!("MCP transport `{transport:?}` cannot execute `{method}`"),
        ));
    }
    let connection = connection_registry()
        .read()
        .map_err(|_| {
            McpHostError::new(
                "mcp_registry_unavailable",
                "MCP connection registry is unavailable",
            )
        })?
        .get(connection_ref)
        .cloned();
    let Some(connection) = connection else {
        return Err(McpHostError::new(
            "mcp_connection_unavailable",
            format!("MCP HTTP connection `{connection_ref}` is not registered by this Host"),
        ));
    };
    let mut connection = connection.lock().map_err(|_| {
        McpHostError::new(
            "mcp_connection_unavailable",
            "MCP connection state is unavailable",
        )
    })?;
    if connection.retired {
        return Err(McpHostError::new("mcp_connection_retired", "MCP connection was retired"));
    }
    if connection.client.is_none() { return Err(McpHostError::new("mcp_connection_required",
        "The MCP connection is not active; call mcp_connect and obtain any required approval")); }
    let result = connection
        .client
        .as_mut()
        .expect("MCP client initialized")
        .request(method, params);
    let output = match result {
        Ok(output) => output,
        Err(error) => {
            // The session may no longer be usable. Drop only this server's
            // client so the next independent invocation can re-initialize.
            // Never retry the current call automatically: the remote tool may
            // have completed before the transport failed.
            connection.client = None;
            return Err(error);
        }
    };
    Ok(output)
}

impl McpHostError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

pub fn describe_configured_servers(values: &[Value]) -> Result<Vec<Value>, McpHostError> {
    describe_servers(values,false)
}

/// Host-internal metadata for owner-bound registrations; performs no RPC.
pub fn describe_connected_cache(values:&[Value]) -> Result<Vec<Value>,McpHostError> {
    describe_servers(values,true)
}

fn describe_servers(values:&[Value], connected:bool) -> Result<Vec<Value>,McpHostError> {
    values.iter().enumerate().map(|(index,value)| {
        if value["enabled"] == false { return Ok(value.clone()); }
        let server_id = configured_server_id(value,index)?;
        let display_name = value.get("display_name").or_else(||value.get("name"))
            .and_then(Value::as_str).unwrap_or(&server_id).to_owned();
        let transport = if crate::mcp_stdio::is_stdio_configuration(value) {McpTransportKind::Stdio}
            else {McpTransportKind::StreamableHttp};
        let reference = value["_nola_mcp_connection"]["connection_ref"].as_str()
            .map(str::to_owned).unwrap_or_else(||format!("pending:{server_id}"));
        let cached = if connected {cached_descriptor(transport,&reference)?} else {None};
        let descriptor = cached.unwrap_or(McpServerDescriptor {
            server_id,display_name,transport,connection_ref:reference,
            availability:McpServerAvailability::AuthorizationRequired,protocol_version:None,instructions:None,
            tools:Default::default(),resources:Default::default(),resource_templates:Default::default(),prompts:Default::default(),
        });
        let mut merged = value.as_object().cloned().ok_or_else(||McpHostError::new("mcp_config_invalid","MCP configuration must be an object"))?;
        merged.extend(serde_json::to_value(descriptor).map_err(|error|McpHostError::new("mcp_descriptor_invalid",error.to_string()))?
            .as_object().expect("descriptor object").clone());
        Ok(Value::Object(merged))
    }).collect()
}

pub fn connection_resource(reference: &str) -> Result<crate::mcp_connection_authority::ConnectionResource,McpHostError> {
    let connection = connection_registry().read().map_err(|_|McpHostError::new("mcp_registry_unavailable","MCP registry unavailable"))?
        .get(reference).cloned();
    let Some(connection) = connection else { return crate::mcp_stdio::connection_resource(reference); };
    let connection = connection.lock().map_err(|_|McpHostError::new("mcp_connection_unavailable","MCP connection unavailable"))?;
    if connection.retired { return Err(McpHostError::new("mcp_connection_retired","MCP connection retired")); }
    let mut resource = connection.config.resource.clone();
    resource.admission_id = connection.admission_id.clone();
    resource.generation = connection.client.as_ref().and(connection.generation.clone());
    Ok(resource)
}

fn cached_descriptor(transport:McpTransportKind, reference:&str) -> Result<Option<McpServerDescriptor>,McpHostError> {
    if transport == McpTransportKind::Stdio { return crate::mcp_stdio::cached_descriptor(reference); }
    let connection = connection_registry().read().map_err(|_|McpHostError::new("mcp_registry_unavailable","MCP registry unavailable"))?
        .get(reference).cloned();
    let Some(connection) = connection else { return Ok(None); };
    let connection = connection.lock().map_err(|_|McpHostError::new("mcp_connection_unavailable","MCP connection unavailable"))?;
    Ok((!connection.retired && connection.client.is_some()).then(||connection.descriptor.clone()).flatten())
}

/// Only an admitted mcp_connect may enter this operation. Recheck the exact
/// immutable owner/config and generation while holding the original entry.
pub fn connect_approved(reference:&str, expected:&crate::mcp_connection_authority::ConnectionResource)
    -> Result<crate::mcp_connection_authority::ConnectionEstablished,McpHostError> {
    if expected.transport == "stdio" { return crate::mcp_stdio::connect_approved(reference,expected); }
    let connection = connection_registry().read().map_err(|_|McpHostError::new("mcp_registry_unavailable","MCP registry unavailable"))?
        .get(reference).cloned().ok_or_else(||McpHostError::new("mcp_connection_required","MCP target is not registered"))?;
    let mut connection = connection.lock().map_err(|_|McpHostError::new("mcp_connection_unavailable","MCP connection unavailable"))?;
    let mut actual = connection.config.resource.clone();
    actual.admission_id = connection.admission_id.clone();
    actual.generation = connection.client.as_ref().and(connection.generation.clone());
    if connection.retired || reference != expected.connection_ref || actual != *expected {
        return Err(McpHostError::new("mcp_connection_changed","MCP resource changed; connection was not started"));
    }
    if connection.client.is_some() {
        let server = connection.descriptor.clone().ok_or_else(||McpHostError::new("mcp_descriptor_missing","Active MCP descriptor is missing"))?;
        return Ok(crate::mcp_connection_authority::ConnectionEstablished {server,resource:actual});
    }
    // Consume this absent-connection admission before any physical handshake.
    // Failure/loss/cold restart cannot reuse the old pending approval identity.
    connection.admission_id = uuid::Uuid::new_v4().to_string();
    let config = connection.config.clone();
    let mut client = StreamableHttpMcpClient::connect(&config.endpoint,config.timeout_ms,config.headers)?;
    let descriptor = client.discover(config.server_id,config.display_name,reference.into())?;
    connection.generation = Some(uuid::Uuid::new_v4().to_string());
    connection.descriptor = Some(descriptor.clone());
    connection.client = Some(client);
    actual.generation = connection.generation.clone();
    actual.admission_id = connection.admission_id.clone();
    Ok(crate::mcp_connection_authority::ConnectionEstablished {server:descriptor,resource:actual})
}

/// List changes are observed on the existing admitted logical connection only.
/// Missing or lost connections remain pending authorization, never reconnect.
pub fn discover_connected_servers(values:&[Value]) -> Result<Vec<Value>,McpHostError> {
    for value in values {
        if value["enabled"] == false { continue; }
        let reference = value["connection_ref"].as_str().ok_or_else(||McpHostError::new("mcp_owner_missing","MCP target lacks its bound reference"))?;
        if crate::mcp_stdio::is_stdio_configuration(value) {
            crate::mcp_stdio::refresh_connected(reference)?;
            continue;
        }
        let connection = connection_registry().read().map_err(|_|McpHostError::new("mcp_registry_unavailable","MCP registry unavailable"))?
            .get(reference).cloned();
        let Some(connection) = connection else { continue; };
        let mut connection = connection.lock().map_err(|_|McpHostError::new("mcp_connection_unavailable","MCP connection unavailable"))?;
        if connection.retired || connection.client.is_none() { continue; }
        let config = connection.config.clone();
        let discovered = connection.client.as_mut().expect("checked client").discover(config.server_id,config.display_name,reference.into());
        match discovered {
            Ok(mut descriptor) => { descriptor.connection_ref = reference.into(); connection.descriptor = Some(descriptor); }
            Err(error) => { connection.client = None; connection.descriptor = None; return Err(error); }
        }
    }
    describe_servers(values,true)
}

pub fn configured_server_id(value: &Value, index: usize) -> Result<String, McpHostError> {
    if let Some(id) = value.get("server_id") {
        let id = id.as_str().filter(|id|!id.is_empty() && id.trim() == *id && id.len() <= 512
            && !id.chars().any(char::is_control)).ok_or_else(||McpHostError::new("mcp_server_id_invalid","MCP server_id is invalid"))?;
        return Ok(id.to_owned());
    }
    let raw = value
        .get("server_id")
        .or_else(|| value.get("name"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            McpHostError::new(
                "mcp_server_id_required",
                format!("MCP server at index {index} has no stable name"),
            )
        })?;
    let normalized = raw
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-') {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    if normalized.is_empty() {
        return Err(McpHostError::new(
            "mcp_server_id_invalid",
            format!("MCP server name at index {index} has no usable identifier"),
        ));
    }
    Ok(normalized)
}

fn configured_headers(value: &Value) -> Result<Vec<(HeaderName, HeaderValue)>, McpHostError> {
    let Some(headers) = value.get("headers").and_then(Value::as_object) else {
        return Ok(Vec::new());
    };
    headers
        .iter()
        .map(|(name, value)| {
            let text = value.as_str().ok_or_else(|| {
                McpHostError::new(
                    "mcp_header_invalid",
                    format!("MCP header `{name}` must be text"),
                )
            })?;
            let name = HeaderName::from_bytes(name.as_bytes()).map_err(|_| {
                McpHostError::new("mcp_header_invalid", "MCP header name is invalid")
            })?;
            let value = HeaderValue::from_str(text).map_err(|_| {
                McpHostError::new("mcp_header_invalid", "MCP header value is invalid")
            })?;
            Ok((name, value))
        })
        .collect()
}

struct StreamableHttpMcpClient {
    client: Client,
    endpoint: String,
    headers: Vec<(HeaderName, HeaderValue)>,
    session_id: Option<String>,
    protocol_version: String,
    server_instructions: Option<String>,
    next_request_id: u64,
    catalog_dirty: bool,
}

struct HttpResponseSnapshot {
    session_id: Option<String>,
    body: String,
}

enum HttpRequestEvent {
    Progress(Value),
    Finished(Result<HttpResponseSnapshot, McpHostError>),
}

fn send_http_payload_streaming(
    client: Client,
    endpoint: String,
    headers: Vec<(HeaderName, HeaderValue)>,
    session_id: Option<String>,
    payload: Value,
    events: std::sync::mpsc::Sender<HttpRequestEvent>,
) {
    let result = (|| {
        let mut request = client
            .post(endpoint)
            .header(ACCEPT, "application/json, text/event-stream")
            .header(CONTENT_TYPE, "application/json")
            .header(USER_AGENT, concat!("Nola/", env!("CARGO_PKG_VERSION")));
        for (name, value) in headers {
            request = request.header(name, value);
        }
        if let Some(session_id) = session_id {
            request = request.header("Mcp-Session-Id", session_id);
        }
        let mut response = request
            .json(&payload)
            .send()
            .map_err(|error| McpHostError::new("mcp_connection_failed", error.to_string()))?;
        let status = response.status();
        let response_session_id = response
            .headers()
            .get("mcp-session-id")
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let is_sse = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.to_ascii_lowercase().contains("text/event-stream"));
        if status.as_u16() == 401 || status.as_u16() == 403 {
            return Err(McpHostError::new(
                "mcp_authorization_required",
                format!(
                    "MCP server requires authorization (HTTP {})",
                    status.as_u16()
                ),
            ));
        }
        if !status.is_success() {
            return Err(McpHostError::new(
                "mcp_http_failed",
                format!("MCP server returned HTTP {}", status.as_u16()),
            ));
        }
        let mut body = Vec::new();
        let mut buffer = [0u8; 8 * 1_024];
        let mut parsed_until = 0usize;
        loop {
            let count = response.read(&mut buffer).map_err(|error| {
                McpHostError::new("mcp_response_read_failed", error.to_string())
            })?;
            if count == 0 {
                break;
            }
            body.extend_from_slice(&buffer[..count]);
            if body.len() as u64 > MAX_RESPONSE_BYTES {
                return Err(McpHostError::new(
                    "mcp_response_too_large",
                    "MCP response exceeded the 4 MB limit",
                ));
            }
            if !is_sse {
                continue;
            }
            while let Some(relative_end) = body[parsed_until..]
                .windows(2)
                .position(|window| window == b"\n\n")
            {
                let frame_end = parsed_until + relative_end + 2;
                if let Ok(frame) = std::str::from_utf8(&body[parsed_until..frame_end]) {
                    if let Ok(messages) = decode_rpc_messages(frame) {
                        for message in messages {
                            if message.get("method").and_then(Value::as_str)
                                == Some("notifications/progress")
                            {
                                let _ = events.send(HttpRequestEvent::Progress(
                                    message.get("params").cloned().unwrap_or(Value::Null),
                                ));
                            }
                        }
                    }
                }
                parsed_until = frame_end;
            }
        }
        let body = String::from_utf8(body).map_err(|error| {
            McpHostError::new(
                "mcp_response_invalid",
                format!("Invalid MCP response UTF-8: {error}"),
            )
        })?;
        Ok(HttpResponseSnapshot {
            session_id: response_session_id,
            body,
        })
    })();
    let _ = events.send(HttpRequestEvent::Finished(result));
}

fn send_http_payload(
    client: Client,
    endpoint: String,
    headers: Vec<(HeaderName, HeaderValue)>,
    session_id: Option<String>,
    payload: Value,
) -> Result<HttpResponseSnapshot, McpHostError> {
    let mut request = client
        .post(endpoint)
        .header(ACCEPT, "application/json, text/event-stream")
        .header(CONTENT_TYPE, "application/json")
        .header(USER_AGENT, concat!("Nola/", env!("CARGO_PKG_VERSION")));
    for (name, value) in headers {
        request = request.header(name, value);
    }
    if let Some(session_id) = session_id {
        request = request.header("Mcp-Session-Id", session_id);
    }
    let response = request
        .json(&payload)
        .send()
        .map_err(|error| McpHostError::new("mcp_connection_failed", error.to_string()))?;
    let status = response.status();
    let response_session_id = response
        .headers()
        .get("mcp-session-id")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let mut body = String::new();
    response
        .take(MAX_RESPONSE_BYTES + 1)
        .read_to_string(&mut body)
        .map_err(|error| McpHostError::new("mcp_response_read_failed", error.to_string()))?;
    if body.len() as u64 > MAX_RESPONSE_BYTES {
        return Err(McpHostError::new(
            "mcp_response_too_large",
            "MCP response exceeded the 4 MB limit",
        ));
    }
    if status.as_u16() == 401 || status.as_u16() == 403 {
        return Err(McpHostError::new(
            "mcp_authorization_required",
            format!(
                "MCP server requires authorization (HTTP {})",
                status.as_u16()
            ),
        ));
    }
    if !status.is_success() {
        return Err(McpHostError::new(
            "mcp_http_failed",
            format!(
                "MCP server returned HTTP {}: {}",
                status.as_u16(),
                truncate(&body, 500)
            ),
        ));
    }
    Ok(HttpResponseSnapshot {
        session_id: response_session_id,
        body,
    })
}

impl StreamableHttpMcpClient {
    fn connect(
        endpoint: &str,
        timeout_ms: u64,
        headers: Vec<(HeaderName, HeaderValue)>,
    ) -> Result<Self, McpHostError> {
        let parsed = reqwest::Url::parse(endpoint).map_err(|_| {
            McpHostError::new(
                "mcp_endpoint_invalid",
                "MCP endpoint must be an absolute URL",
            )
        })?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err(McpHostError::new(
                "mcp_transport_unsupported",
                "Remote MCP discovery supports only Streamable HTTP endpoints",
            ));
        }
        if !parsed.username().is_empty() || parsed.password().is_some() {
            return Err(McpHostError::new(
                "mcp_endpoint_credentials_forbidden",
                "Put MCP credentials in headers, not in the endpoint URL",
            ));
        }
        let client = Client::builder()
            .no_proxy()
            .connect_timeout(Duration::from_millis(timeout_ms.min(15_000)))
            .timeout(Duration::from_millis(timeout_ms))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|error| McpHostError::new("mcp_client_failed", error.to_string()))?;
        let mut connection = Self {
            client,
            endpoint: parsed.to_string(),
            headers,
            session_id: None,
            protocol_version: DEFAULT_PROTOCOL_VERSION.to_string(),
            server_instructions: None,
            next_request_id: 1,
            catalog_dirty: false,
        };
        let initialize = connection.request(
            "initialize",
            json!({
                "protocolVersion": DEFAULT_PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {"name": "nola", "version": env!("CARGO_PKG_VERSION")}
            }),
        )?;
        connection.protocol_version = initialize
            .get("protocolVersion")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(DEFAULT_PROTOCOL_VERSION)
            .to_string();
        connection.server_instructions =
            nola_core::agent::capability::bounded_mcp_server_instructions(
                initialize.get("instructions").and_then(Value::as_str),
            );
        connection.notification("notifications/initialized", json!({}))?;
        Ok(connection)
    }

    fn discover(
        &mut self,
        server_id: String,
        display_name: String,
        connection_ref: String,
    ) -> Result<McpServerDescriptor, McpHostError> {
        let tools = self.list_all("tools/list", "tools")?;
        let resources = self.list_all_optional("resources/list", "resources")?;
        let templates = self.list_all_optional("resources/templates/list", "resourceTemplates")?;
        let prompts = self.list_all_optional("prompts/list", "prompts")?;
        let tools = tools
            .into_iter()
            .filter_map(decode_tool)
            .map(|descriptor| (descriptor.name.clone(), descriptor))
            .collect::<BTreeMap<_, _>>();
        if tools.is_empty() && resources.is_empty() && templates.is_empty() && prompts.is_empty() {
            return Err(McpHostError::new(
                "mcp_capabilities_empty",
                "MCP handshake succeeded but returned no tools, resources, templates, or prompts",
            ));
        }
        Ok(McpServerDescriptor {
            server_id: server_id.clone(),
            display_name,
            transport: McpTransportKind::StreamableHttp,
            connection_ref,
            availability: McpServerAvailability::Available,
            protocol_version: Some(self.protocol_version.clone()),
            instructions: self.server_instructions.clone(),
            tools,
            resources: resources
                .into_iter()
                .filter_map(decode_resource)
                .map(|descriptor| (descriptor.resource_id.clone(), descriptor))
                .collect(),
            resource_templates: templates
                .into_iter()
                .filter_map(decode_resource_template)
                .map(|descriptor| (descriptor.template_id.clone(), descriptor))
                .collect(),
            prompts: prompts
                .into_iter()
                .filter_map(decode_prompt)
                .map(|descriptor| (descriptor.name.clone(), descriptor))
                .collect(),
        })
    }

    fn list_all(&mut self, method: &str, field: &str) -> Result<Vec<Value>, McpHostError> {
        self.list_pages(method, field, false)
    }

    fn list_all_optional(&mut self, method: &str, field: &str) -> Result<Vec<Value>, McpHostError> {
        self.list_pages(method, field, true)
    }

    fn list_pages(
        &mut self,
        method: &str,
        field: &str,
        optional: bool,
    ) -> Result<Vec<Value>, McpHostError> {
        let mut items = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..MAX_LIST_PAGES {
            let params = cursor
                .as_ref()
                .map(|value| json!({"cursor": value}))
                .unwrap_or_else(|| json!({}));
            let result = match self.request(method, params) {
                Ok(value) => value,
                Err(error) if optional && error.code == "mcp_method_not_found" => return Ok(items),
                Err(error) => return Err(error),
            };
            items.extend(
                result
                    .get(field)
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default(),
            );
            cursor = result
                .get("nextCursor")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string);
            if cursor.is_none() {
                return Ok(items);
            }
        }
        Err(McpHostError::new(
            "mcp_pagination_limit",
            format!("MCP method `{method}` exceeded {MAX_LIST_PAGES} pages"),
        ))
    }

    fn request(&mut self, method: &str, params: Value) -> Result<Value, McpHostError> {
        let id = self.next_request_id;
        self.next_request_id = self.next_request_id.saturating_add(1);
        let payload = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        let response = self.send(&payload)?;
        decode_rpc_response(&response, Some(id))
    }

    fn request_with_cancellation(
        &mut self,
        method: &str,
        params: Value,
        cancellation: &dyn crate::Cancellation,
        progress: &mut dyn FnMut(&Value),
    ) -> Result<Value, McpHostError> {
        if cancellation.is_requested() {
            return Err(McpHostError::new(
                "mcp_cancelled_before_send",
                "MCP HTTP request was cancelled before transport send",
            ));
        }
        let id = self.next_request_id;
        self.next_request_id = self.next_request_id.saturating_add(1);
        let payload = json!({"jsonrpc":"2.0","id":id,"method":method,"params":params});
        let (sender, receiver) = std::sync::mpsc::channel();
        let client = self.client.clone();
        let endpoint = self.endpoint.clone();
        let headers = self.headers.clone();
        let session_id = self.session_id.clone();
        std::thread::Builder::new()
            .name(format!("nola-mcp-http-request-{id}"))
            .spawn(move || {
                send_http_payload_streaming(client, endpoint, headers, session_id, payload, sender);
            })
            .map_err(|error| McpHostError::new("mcp_worker_start_failed", error.to_string()))?;
        loop {
            match receiver.recv_timeout(Duration::from_millis(50)) {
                Ok(HttpRequestEvent::Progress(params)) => progress(&params),
                Ok(HttpRequestEvent::Finished(Ok(snapshot))) => {
                    if let Some(session_id) = snapshot.session_id {
                        self.session_id = Some(session_id);
                    }
                    self.catalog_dirty |= contains_catalog_change_notification(&snapshot.body);
                    return decode_rpc_response(&snapshot.body, Some(id));
                }
                Ok(HttpRequestEvent::Finished(Err(error))) => return Err(error),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    if !cancellation.is_requested() {
                        continue;
                    }
                    let notification = json!({
                        "jsonrpc":"2.0",
                        "method":"notifications/cancelled",
                        "params":{"requestId":id,"reason":"Cancelled by user"}
                    });
                    let client = self.client.clone();
                    let endpoint = self.endpoint.clone();
                    let headers = self.headers.clone();
                    let session_id = self.session_id.clone();
                    let _ = std::thread::Builder::new()
                        .name(format!("nola-mcp-http-cancel-{id}"))
                        .spawn(move || {
                            let _ = send_http_payload(
                                client,
                                endpoint,
                                headers,
                                session_id,
                                notification,
                            );
                        });
                    return Err(McpHostError::new(
                        "mcp_cancelled_after_send",
                        format!(
                            "MCP HTTP cancellation notification was dispatched for request {id}; verify side effects before retrying"
                        ),
                    ));
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(McpHostError::new(
                        "mcp_worker_failed",
                        "MCP HTTP transport worker exited without a result",
                    ));
                }
            }
        }
    }

    fn notification(&mut self, method: &str, params: Value) -> Result<(), McpHostError> {
        let payload = json!({"jsonrpc": "2.0", "method": method, "params": params});
        let response = self.send(&payload)?;
        if response.trim().is_empty() {
            return Ok(());
        }
        decode_rpc_response(&response, None).map(|_| ())
    }

    fn send(&mut self, payload: &Value) -> Result<String, McpHostError> {
        let mut request = self
            .client
            .post(&self.endpoint)
            .header(ACCEPT, "application/json, text/event-stream")
            .header(CONTENT_TYPE, "application/json")
            .header(USER_AGENT, concat!("Nola/", env!("CARGO_PKG_VERSION")));
        for (name, value) in &self.headers {
            request = request.header(name, value);
        }
        if let Some(session_id) = &self.session_id {
            request = request.header("Mcp-Session-Id", session_id);
        }
        let response = request
            .json(payload)
            .send()
            .map_err(|error| McpHostError::new("mcp_connection_failed", error.to_string()))?;
        self.capture_response(response)
    }

    fn capture_response(&mut self, response: Response) -> Result<String, McpHostError> {
        let status = response.status();
        if let Some(session_id) = response
            .headers()
            .get("mcp-session-id")
            .and_then(|value| value.to_str().ok())
            .map(str::to_string)
        {
            self.session_id = Some(session_id);
        }
        let mut body = String::new();
        response
            .take(MAX_RESPONSE_BYTES + 1)
            .read_to_string(&mut body)
            .map_err(|error| McpHostError::new("mcp_response_read_failed", error.to_string()))?;
        if body.len() as u64 > MAX_RESPONSE_BYTES {
            return Err(McpHostError::new(
                "mcp_response_too_large",
                "MCP response exceeded the 4 MB limit",
            ));
        }
        if status.as_u16() == 401 || status.as_u16() == 403 {
            return Err(McpHostError::new(
                "mcp_authorization_required",
                format!(
                    "MCP server requires authorization (HTTP {})",
                    status.as_u16()
                ),
            ));
        }
        if !status.is_success() {
            return Err(McpHostError::new(
                "mcp_http_failed",
                format!(
                    "MCP server returned HTTP {}: {}",
                    status.as_u16(),
                    truncate(&body, 500)
                ),
            ));
        }
        if contains_catalog_change_notification(&body) {
            self.catalog_dirty = true;
        }
        Ok(body)
    }
}

/// Re-discovers retained HTTP servers only after one of them reported a
/// capability-list change. The active Turn keeps its frozen catalog; callers
/// install the returned descriptors when constructing the next Turn.
pub fn refresh_changed_servers(
    values: &[Value],
) -> Result<Option<Vec<Value>>, McpHostError> {
    let registry = connection_registry().read().map_err(|_| {
        McpHostError::new(
            "mcp_registry_unavailable",
            "MCP connection registry is unavailable",
        )
    })?;
    let mut dirty = false;
    for (index, value) in values.iter().enumerate() {
        if value.get("enabled").and_then(Value::as_bool) == Some(false) {
            continue;
        }
        if crate::mcp_stdio::is_stdio_configuration(value) {
            dirty |= crate::mcp_stdio::catalog_change_pending(value, index)?;
            continue;
        }
        let connection_ref = value
            .get("connection_ref")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .ok_or_else(||McpHostError::new("mcp_owner_missing","MCP refresh requires an owner-bound reference"))?;
        let Some(connection) = registry.get(&connection_ref) else {
            continue;
        };
        let connection = connection.lock().map_err(|_| {
            McpHostError::new(
                "mcp_connection_unavailable",
                "MCP connection state is unavailable",
            )
        })?;
        dirty |= connection
            .client
            .as_ref()
            .is_some_and(|client| client.catalog_dirty);
    }
    drop(registry);
    if !dirty {
        return Ok(None);
    }
    discover_connected_servers(values).map(Some)
}

pub fn decode_rpc_response(
    body: &str,
    expected_id: Option<u64>,
) -> Result<Value, McpHostError> {
    let messages = decode_rpc_messages(body)?;
    if messages.is_empty() && expected_id.is_none() {
        return Ok(Value::Null);
    }
    let decoded = match expected_id {
        Some(expected_id) => messages
            .into_iter()
            .find(|message| message.get("id").and_then(Value::as_u64) == Some(expected_id))
            .ok_or_else(|| {
                McpHostError::new(
                    "mcp_response_id_mismatch",
                    "MCP response does not contain the matching request id",
                )
            })?,
        None => messages
            .into_iter()
            .find(|message| message.get("result").is_some() || message.get("error").is_some())
            .unwrap_or(Value::Null),
    };
    if decoded.is_null() && expected_id.is_none() {
        return Ok(Value::Null);
    }
    if let Some(error) = decoded.get("error") {
        let code = if error.get("code").and_then(Value::as_i64) == Some(-32601) {
            "mcp_method_not_found"
        } else {
            "mcp_rpc_failed"
        };
        return Err(McpHostError::new(code, truncate(&error.to_string(), 500)));
    }
    decoded
        .get("result")
        .cloned()
        .ok_or_else(|| McpHostError::new("mcp_result_missing", "MCP response has no result"))
}

fn decode_rpc_messages(body: &str) -> Result<Vec<Value>, McpHostError> {
    let candidates = if body.trim().is_empty() {
        Vec::new()
    } else if body.trim_start().starts_with('{') {
        vec![body.trim()]
    } else {
        body.lines()
            .filter_map(|line| line.strip_prefix("data:"))
            .map(str::trim)
            .filter(|line| line.starts_with('{'))
            .collect::<Vec<_>>()
    };
    candidates
        .into_iter()
        .map(|candidate| {
            serde_json::from_str(candidate).map_err(|error| {
                McpHostError::new(
                    "mcp_response_invalid",
                    format!("Invalid MCP JSON-RPC response: {error}"),
                )
            })
        })
        .collect()
}

fn contains_catalog_change_notification(body: &str) -> bool {
    decode_rpc_messages(body)
        .is_ok_and(|messages| messages.iter().any(is_catalog_change_notification))
}

pub fn is_catalog_change_notification(message: &Value) -> bool {
    matches!(
        message.get("method").and_then(Value::as_str),
        Some(
            "notifications/tools/list_changed"
                | "notifications/resources/list_changed"
                | "notifications/prompts/list_changed"
        )
    )
}

pub fn decode_tool(value: Value) -> Option<McpToolDescriptor> {
    let name = value.get("name")?.as_str()?.trim().to_string();
    (!name.is_empty()).then(|| McpToolDescriptor {
        name,
        title: text_field(&value, "title"),
        description: text_field(&value, "description"),
        input_schema: value
            .get("inputSchema")
            .cloned()
            .unwrap_or_else(|| json!({"type": "object"})),
        output_schema: value.get("outputSchema").cloned(),
        read_only: annotation_bool(&value, "readOnlyHint"),
        destructive: annotation_bool(&value, "destructiveHint"),
    })
}

pub fn decode_resource(value: Value) -> Option<McpResourceDescriptor> {
    let uri = value.get("uri")?.as_str()?.trim().to_string();
    if uri.is_empty() {
        return None;
    }
    Some(McpResourceDescriptor {
        resource_id: uri.clone(),
        uri,
        name: text_field(&value, "name").unwrap_or_else(|| "Resource".to_string()),
        description: text_field(&value, "description"),
        mime_type: text_field(&value, "mimeType"),
        annotations: value
            .get("annotations")
            .and_then(Value::as_object)
            .map(object_to_btree)
            .unwrap_or_default(),
    })
}

pub fn decode_resource_template(value: Value) -> Option<McpResourceTemplateDescriptor> {
    let uri_template = value.get("uriTemplate")?.as_str()?.trim().to_string();
    if uri_template.is_empty() {
        return None;
    }
    Some(McpResourceTemplateDescriptor {
        template_id: uri_template.clone(),
        uri_template,
        name: text_field(&value, "name").unwrap_or_else(|| "Resource template".to_string()),
        description: text_field(&value, "description"),
        mime_type: text_field(&value, "mimeType"),
    })
}

pub fn decode_prompt(value: Value) -> Option<McpPromptDescriptor> {
    let name = value.get("name")?.as_str()?.trim().to_string();
    if name.is_empty() {
        return None;
    }
    Some(McpPromptDescriptor {
        name,
        title: text_field(&value, "title"),
        description: text_field(&value, "description"),
        arguments: value
            .get("arguments")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|argument| {
                let name = argument.get("name")?.as_str()?.trim().to_string();
                (!name.is_empty()).then(|| McpPromptArgumentDescriptor {
                    name,
                    description: text_field(argument, "description"),
                    required: argument
                        .get("required")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                })
            })
            .collect(),
    })
}

fn text_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn annotation_bool(value: &Value, key: &str) -> bool {
    value
        .get("annotations")
        .and_then(|annotations| annotations.get(key))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn object_to_btree(object: &Map<String, Value>) -> BTreeMap<String, Value> {
    object
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

fn truncate(value: &str, maximum: usize) -> String {
    value.chars().take(maximum).collect()
}

use std::io::Read;

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    fn delayed_http_mcp_server() -> (
        String,
        Arc<AtomicBool>,
        Arc<AtomicBool>,
        std::thread::JoinHandle<()>,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind MCP test server");
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let tool_received = Arc::new(AtomicBool::new(false));
        let cancellation_received = Arc::new(AtomicBool::new(false));
        let received = Arc::clone(&tool_received);
        let cancelled = Arc::clone(&cancellation_received);
        let handle = std::thread::spawn(move || {
            let mut handlers = Vec::new();
            for stream in listener.incoming().take(8) {
                let received = Arc::clone(&received);
                let cancelled = Arc::clone(&cancelled);
                handlers.push(std::thread::spawn(move || {
                    let mut stream = stream.expect("accept MCP request");
                    let mut request = Vec::new();
                    let mut buffer = [0u8; 4096];
                    let header_end = loop {
                        let count = stream.read(&mut buffer).expect("read MCP request");
                        if count == 0 {
                            return;
                        }
                        request.extend_from_slice(&buffer[..count]);
                        if let Some(position) = request.windows(4).position(|value| value == b"\r\n\r\n") {
                            break position + 4;
                        }
                    };
                    let headers = String::from_utf8_lossy(&request[..header_end]);
                    let content_length = headers
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().ok())
                                .flatten()
                        })
                        .unwrap_or_default();
                    while request.len() < header_end + content_length {
                        let count = stream.read(&mut buffer).expect("read MCP request body");
                        if count == 0 {
                            break;
                        }
                        request.extend_from_slice(&buffer[..count]);
                    }
                    let payload: Value = serde_json::from_slice(
                        &request[header_end..request.len().min(header_end + content_length)],
                    )
                    .expect("decode MCP request");
                    let method = payload["method"].as_str().unwrap_or_default();
                    let id = payload.get("id").cloned();
                    if method == "tools/call" {
                        let progress_frame = concat!(
                            "event: message\n",
                            "data: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\",\"params\":{\"progressToken\":\"http-phase\",\"progress\":1,\"total\":4,\"message\":\"正在读取远程资料\"}}\n\n"
                        );
                        let final_frame = format!(
                            "event: message\ndata: {}\n\n",
                            json!({
                                "jsonrpc":"2.0", "id":id,
                                "result":{"content":[{"type":"text","text":"written"}]}
                            })
                        );
                        let content_length = progress_frame.len() + final_frame.len();
                        write!(
                            stream,
                            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {content_length}\r\nConnection: close\r\n\r\n{progress_frame}"
                        )
                        .and_then(|_| stream.flush())
                        .expect("write MCP progress frame");
                        received.store(true, Ordering::SeqCst);
                        std::thread::sleep(Duration::from_millis(1_200));
                        stream
                            .write_all(final_frame.as_bytes())
                            .expect("write MCP final frame");
                        return;
                    }
                    let response = match method {
                        "notifications/initialized" => String::new(),
                        "notifications/cancelled" => {
                            assert_eq!(payload["params"]["requestId"], 6);
                            cancelled.store(true, Ordering::SeqCst);
                            String::new()
                        }
                        "initialize" => json!({
                            "jsonrpc":"2.0", "id":id,
                            "result":{"protocolVersion":DEFAULT_PROTOCOL_VERSION,"capabilities":{}}
                        })
                        .to_string(),
                        "tools/list" => json!({
                            "jsonrpc":"2.0", "id":id,
                            "result":{"tools":[{"name":"slow_write","description":"slow mutation","inputSchema":{"type":"object"}}]}
                        })
                        .to_string(),
                        "resources/list" | "resources/templates/list" | "prompts/list" => json!({
                            "jsonrpc":"2.0", "id":id,
                            "error":{"code":-32601,"message":"Method not found"}
                        })
                        .to_string(),
                        unexpected => panic!("unexpected MCP method {unexpected}"),
                    };
                    write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        response.len(),
                        response
                    )
                    .expect("write MCP response");
                }));
            }
            for handler in handlers {
                handler.join().expect("MCP request handler");
            }
        });
        (endpoint, tool_received, cancellation_received, handle)
    }

    fn crashing_http_mcp_server(
        marker: std::path::PathBuf,
    ) -> (String, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind crashing MCP test server");
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            for stream in listener.incoming().take(7) {
                let mut stream = stream.expect("accept crashing MCP request");
                let mut request = Vec::new();
                let mut buffer = [0u8; 4096];
                let header_end = loop {
                    let count = stream.read(&mut buffer).expect("read crashing MCP request");
                    if count == 0 {
                        return;
                    }
                    request.extend_from_slice(&buffer[..count]);
                    if let Some(position) =
                        request.windows(4).position(|value| value == b"\r\n\r\n")
                    {
                        break position + 4;
                    }
                };
                let headers = String::from_utf8_lossy(&request[..header_end]);
                let content_length = headers
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().ok())
                            .flatten()
                    })
                    .unwrap_or_default();
                while request.len() < header_end + content_length {
                    let count = stream
                        .read(&mut buffer)
                        .expect("read crashing MCP request body");
                    if count == 0 {
                        break;
                    }
                    request.extend_from_slice(&buffer[..count]);
                }
                let payload: Value = serde_json::from_slice(
                    &request[header_end..request.len().min(header_end + content_length)],
                )
                .expect("decode crashing MCP request");
                let method = payload["method"].as_str().unwrap_or_default();
                let id = payload.get("id").cloned();
                if method == "tools/call" {
                    std::fs::write(&marker, "1").expect("record HTTP mutation");
                    write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: 4096\r\nConnection: close\r\n\r\nevent: message\ndata: {{\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\""
                    )
                    .and_then(|_| stream.flush())
                    .expect("write truncated MCP response");
                    return;
                }
                let response = match method {
                    "notifications/initialized" => String::new(),
                    "initialize" => json!({
                        "jsonrpc":"2.0", "id":id,
                        "result":{"protocolVersion":DEFAULT_PROTOCOL_VERSION,"capabilities":{}}
                    })
                    .to_string(),
                    "tools/list" => json!({
                        "jsonrpc":"2.0", "id":id,
                        "result":{"tools":[{"name":"crash_after_write","description":"mutate then disconnect","inputSchema":{"type":"object"}}]}
                    })
                    .to_string(),
                    "resources/list" | "resources/templates/list" | "prompts/list" => json!({
                        "jsonrpc":"2.0", "id":id,
                        "error":{"code":-32601,"message":"Method not found"}
                    })
                    .to_string(),
                    unexpected => panic!("unexpected MCP method {unexpected}"),
                };
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    response.len(),
                    response
                )
                .expect("write crashing MCP setup response");
            }
        });
        (endpoint, handle)
    }

    #[test]
    fn cancelled_call_never_reaches_transport() {
        let cancellation = crate::CancellationToken::default();
        cancellation.request();
        let error = call_tool_with_cancellation(
            McpTransportKind::StreamableHttp,
            "http://127.0.0.1:1/should-not-connect",
            "write_document",
            json!({"path":"never-created.docx"}),
            &cancellation,
        )
        .expect_err("pre-cancelled calls must not reach the transport");
        assert_eq!(error.code, "mcp_cancelled_before_send");
    }

    #[test]
    fn cancellation_after_send_returns_promptly_and_marks_side_effect_unknown() {
        let (endpoint, tool_received, cancellation_received, server) = delayed_http_mcp_server();
        let discovered = crate::mcp_connection_authority::connect_fixture_servers(&[json!({
            "server_id":"delayed-cancellation-test",
            "endpoint":endpoint,
            "timeout_ms":5_000,
        })])
        .expect("discover delayed MCP server");
        let connection_ref = discovered[0]["connection_ref"]
            .as_str()
            .expect("connection ref")
            .to_string();
        let cancellation = crate::CancellationToken::default();
        let canceller = cancellation.clone();
        let received = Arc::clone(&tool_received);
        std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(1);
            while !received.load(Ordering::SeqCst) && std::time::Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(10));
            }
            canceller.request();
        });

        let started = std::time::Instant::now();
        let mut progress_events = Vec::new();
        let error = call_tool_with_cancellation_and_progress(
            McpTransportKind::StreamableHttp,
            &connection_ref,
            "slow_write",
            json!({"path":"possibly-written.docx"}),
            &cancellation,
            &mut |progress| progress_events.push(progress.clone()),
        )
        .expect_err("post-send cancellation must not claim a definitive result");
        assert_eq!(error.code, "mcp_cancelled_after_send");
        assert!(error
            .message
            .contains("verify side effects before retrying"));
        assert!(started.elapsed() < Duration::from_millis(700));
        assert!(tool_received.load(Ordering::SeqCst));
        assert_eq!(progress_events.len(), 1);
        assert_eq!(progress_events[0]["message"], "正在读取远程资料");
        server.join().expect("MCP test server");
        assert!(cancellation_received.load(Ordering::SeqCst));
    }

    #[test]
    fn http_mutation_disconnect_is_not_retried_after_transport_send() {
        let marker = std::env::temp_dir().join(format!(
            "nola-http-mcp-crash-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let (endpoint, server) = crashing_http_mcp_server(marker.clone());
        let discovered = crate::mcp_connection_authority::connect_fixture_servers(&[json!({
            "server_id":"http-crash-test",
            "endpoint":endpoint,
            "timeout_ms":5_000,
        })])
        .expect("discover crashing HTTP MCP server");
        let connection_ref = discovered[0]["connection_ref"]
            .as_str()
            .expect("connection ref")
            .to_string();
        let cancellation = crate::CancellationToken::default();
        let error = call_tool_with_cancellation(
            McpTransportKind::StreamableHttp,
            &connection_ref,
            "crash_after_write",
            json!({"path":"possibly-written.docx"}),
            &cancellation,
        )
        .expect_err("truncated HTTP MCP response must remain uncertain");
        assert!(matches!(
            error.code,
            "mcp_response_read_failed" | "mcp_response_invalid" | "mcp_connection_failed"
        ));
        server.join().expect("crashing HTTP MCP server");
        assert_eq!(std::fs::read_to_string(&marker).unwrap(), "1");
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(std::fs::read_to_string(&marker).unwrap(), "1");
        let _ = std::fs::remove_file(marker);
    }

    #[test]
    fn rpc_response_accepts_json_and_sse_and_checks_ids() {
        let json = r#"{"jsonrpc":"2.0","id":7,"result":{"tools":[]}}"#;
        assert_eq!(
            decode_rpc_response(json, Some(7)).unwrap()["tools"],
            json!([])
        );
        let sse =
            "event: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":8,\"result\":{\"ok\":true}}\n\n";
        assert_eq!(decode_rpc_response(sse, Some(8)).unwrap()["ok"], true);
        assert_eq!(
            decode_rpc_response(json, Some(8)).unwrap_err().code,
            "mcp_response_id_mismatch"
        );
        let response_then_notification = concat!(
            "event: message\n",
            "data: {\"jsonrpc\":\"2.0\",\"id\":9,\"result\":{\"tools\":[]}}\n\n",
            "event: message\n",
            "data: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/tools/list_changed\"}\n\n",
        );
        assert_eq!(
            decode_rpc_response(response_then_notification, Some(9)).unwrap()["tools"],
            json!([])
        );
        assert!(contains_catalog_change_notification(
            response_then_notification
        ));
    }

    #[test]
    fn configured_server_id_is_stable_and_rejects_empty_names() {
        assert_eq!(
            configured_server_id(&json!({"name": "Docs PROD"}), 0).unwrap(),
            "docs-prod"
        );
        assert_eq!(
            configured_server_id(&json!({"name": "***"}), 0)
                .unwrap_err()
                .code,
            "mcp_server_id_invalid"
        );
    }

    #[test]
    fn execution_uses_the_declared_transport_without_registry_guessing() {
        let http = read_resource(
            McpTransportKind::StreamableHttp,
            "missing-http",
            "docs://guide",
        )
        .expect_err("missing HTTP route");
        assert_eq!(http.code, "mcp_connection_unavailable");

        let stdio = read_resource(McpTransportKind::Stdio, "missing-stdio", "docs://guide")
            .expect_err("missing stdio route");
        assert_eq!(stdio.code, "mcp_connection_unavailable");

        let unsupported = read_resource(McpTransportKind::InProcess, "in-process", "docs://guide")
            .expect_err("unsupported transport");
        assert_eq!(unsupported.code, "mcp_transport_unsupported");
    }

    #[test]
    fn result_artifacts_use_only_explicit_mcp_resource_blocks_and_metadata() {
        let output = json!({
            "content": [
                {"type":"text","text":"file:///must-not-be-inferred.txt"},
                {"type":"resource_link","uri":"file:///workspace/result.txt","name":"result.txt","mimeType":"text/plain"},
                {"type":"resource","resource":{"uri":"nola://report/42","name":"report","mimeType":"application/json","text":"{}"}}
            ],
            "_meta": {
                "nola/artifacts": [
                    {"uri":"file:///workspace/explicit.docx","name":"explicit.docx","mime_type":"application/vnd.openxmlformats-officedocument.wordprocessingml.document","operation":"create"},
                    {"uri":"file:///workspace/result.txt","name":"duplicate","operation":"edit"}
                ]
            }
        });
        let artifacts = mcp_result_artifacts(&output).unwrap();
        assert_eq!(artifacts.len(), 3);
        assert!(artifacts
            .iter()
            .any(|artifact| artifact.uri == "file:///workspace/explicit.docx"));
        assert!(artifacts
            .iter()
            .any(|artifact| artifact.uri == "file:///workspace/result.txt"));
        assert!(artifacts
            .iter()
            .any(|artifact| artifact.uri == "nola://report/42"));
        assert!(!artifacts
            .iter()
            .any(|artifact| artifact.uri.contains("must-not-be-inferred")));
    }
}
