// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Host-owned retained MCP stdio process transport.
// - Security: Commands use direct argv execution; secrets remain in Host config and stderr never enters prompts.

use crate::mcp_host::{
    configured_server_id, decode_prompt, decode_resource, decode_resource_template,
    decode_rpc_response, decode_tool, is_catalog_change_notification, McpHostError,
    McpToolExecution,
};
use nola_core::agent::capability::{McpServerAvailability, McpServerDescriptor, McpTransportKind};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{mpsc, Arc, Mutex, OnceLock, RwLock};
use std::thread;
use std::time::{Duration, Instant};

const DEFAULT_TIMEOUT_MS: u64 = 15_000;
const MAX_TIMEOUT_MS: u64 = 21_600_000;
const MAX_MESSAGE_BYTES: usize = 4 * 1_024 * 1_024;
const MAX_STDERR_BYTES: usize = 64 * 1_024;
const MAX_LIST_PAGES: usize = 20;

#[derive(Debug, Clone, PartialEq, Eq)]
struct StdioMcpConfig {
    server_id: String,
    display_name: String,
    command: String,
    command_argv0: String,
    args: Vec<String>,
    environment: BTreeMap<String, String>,
    cwd: Option<PathBuf>,
    timeout_ms: u64,
    connection_ref: String,
}

struct StdioConnectionEntry {
    config: StdioMcpConfig,
    client: Option<StdioMcpClient>,
    retired: bool,
    resource: crate::mcp_connection_authority::ConnectionResource,
    descriptor: Option<McpServerDescriptor>,
}

enum StdoutEvent {
    Line(String),
    Failed(String),
    Closed,
}

struct StdioMcpClient {
    child: Child,
    stdin: ChildStdin,
    stdout_events: mpsc::Receiver<StdoutEvent>,
    stderr_tail: Arc<Mutex<Vec<u8>>>,
    timeout: Duration,
    protocol_version: String,
    server_instructions: Option<String>,
    next_request_id: u64,
    catalog_dirty: bool,
}

struct InitializingChild(Option<Child>);
impl Drop for InitializingChild {
    fn drop(&mut self) {
        if let Some(child) = self.0.as_mut() { let _ = child.kill(); let _ = child.wait(); }
    }
}

fn registry() -> &'static RwLock<BTreeMap<String, Arc<Mutex<StdioConnectionEntry>>>> {
    static REGISTRY: OnceLock<RwLock<BTreeMap<String, Arc<Mutex<StdioConnectionEntry>>>>> =
        OnceLock::new();
    REGISTRY.get_or_init(|| RwLock::new(BTreeMap::new()))
}

pub fn is_stdio_configuration(value: &Value) -> bool {
    let has_command = value.get("command").is_some_and(Value::is_array) || value
        .get("command")
        .and_then(Value::as_str)
        .is_some_and(|command| !command.trim().is_empty());
    let transport = value.get("type").and_then(Value::as_str).unwrap_or("stdio");
    has_command && matches!(transport, "stdio" | "local")
}

pub fn register_configured_server(value: &Value, index: usize) -> Result<(), McpHostError> {
    let server_id = configured_server_id(value, index)?;
    let resource = crate::mcp_connection_authority::resource_from_configuration(value)
        .map_err(|message|McpHostError::new("mcp_owner_missing",message))?;
    let mut config = launch_snapshots().lock().map_err(|_|McpHostError::new("mcp_registry_unavailable","MCP snapshots unavailable"))?
        .get(&resource.config_digest).cloned().ok_or_else(||McpHostError::new("mcp_snapshot_missing","MCP launch has not been frozen by this Host"))?;
    config.connection_ref = resource.connection_ref.clone();
    config.server_id = server_id.clone();
    config.display_name = value.get("display_name").or_else(||value.get("name")).and_then(Value::as_str).unwrap_or(&server_id).into();
    install(config, None, resource)
}

/// Remove admission first, then drain callers that already acquired the entry.
/// Must run on the Host worker, never the UI thread. Unique connection refs
/// prevent a retiring owner from touching a replacement instance.
pub fn revoke_connection(connection_ref: &str) -> StdioRetirement {
    let removed = registry().write().unwrap_or_else(|poisoned| poisoned.into_inner()).remove(connection_ref);
    StdioRetirement(removed)
}

pub struct StdioRetirement(Option<Arc<Mutex<StdioConnectionEntry>>>);
impl Drop for StdioRetirement {
    fn drop(&mut self) {
        if let Some(entry) = self.0.take() {
            let mut entry = entry.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            entry.retired = true;
            entry.client = None; // Drop kills and reaps the owned process.
        }
    }
}

fn launch_snapshots() -> &'static Mutex<BTreeMap<String,StdioMcpConfig>> {
    static SNAPSHOTS: OnceLock<Mutex<BTreeMap<String,StdioMcpConfig>>> = OnceLock::new();
    SNAPSHOTS.get_or_init(||Mutex::new(BTreeMap::new()))
}

fn discard_exited_client(entry: &mut StdioConnectionEntry) -> Result<(),McpHostError> {
    let exited = entry.client.as_mut().map(|client|client.child.try_wait())
        .transpose().map_err(|error|McpHostError::new("mcp_process_observation_failed",error.to_string()))?
        .flatten().is_some();
    if exited { entry.client = None; entry.descriptor = None; entry.resource.generation = None; }
    Ok(())
}

pub fn freeze_configuration(value:&Value, server:&str) -> Result<(String,String),McpHostError> {
    use sha2::{Digest,Sha256};
    let mut config = parse_config(value,server)?;
    // Minimal runtime environment, not the caller's provider/backend secrets.
    // Explicit server env is part of the privately frozen launch identity.
    let mut environment = BTreeMap::new();
    for name in ["PATH","HOME","USERPROFILE","SYSTEMROOT","WINDIR","COMSPEC","PATHEXT",
        "TEMP","TMP","TMPDIR","LANG","LC_ALL","LC_CTYPE","TZ"] {
        if let Some(value) = std::env::var_os(name) {
            environment.insert(name.into(),value.into_string().map_err(|_|McpHostError::new("mcp_environment_invalid","Runtime environment is not UTF-8"))?);
        }
    }
    environment.extend(config.environment);
    config.environment = environment;
    config.cwd = Some(config.cwd.unwrap_or(std::env::current_dir().map_err(|error|McpHostError::new("mcp_cwd_invalid",error.to_string()))?)
        .canonicalize().map_err(|error|McpHostError::new("mcp_cwd_invalid",error.to_string()))?);
    let cwd = config.cwd.as_ref().expect("frozen cwd");
    let requested = PathBuf::from(&config.command);
    let command = if requested.is_absolute() {requested} else if requested.components().count() > 1 {cwd.join(requested)} else {
        std::env::split_paths(&std::ffi::OsString::from(config.environment.get("PATH").cloned().unwrap_or_default()))
            .filter(|directory|directory.is_absolute()).flat_map(|directory| {
                let path = directory.join(&requested);
                #[allow(unused_mut)]
                let mut candidates = vec![path.clone()];
                // CreateProcess launches executable images, not implicit batch interpreters.
                #[cfg(windows)]
                if path.extension().is_none() { candidates.push(path.with_extension("exe")); candidates.push(path.with_extension("com")); }
                candidates
            }).find(|path|path.is_file())
            .ok_or_else(||McpHostError::new("mcp_program_missing","MCP program is not available on the frozen PATH"))?
    };
    // Python virtual environments and other launchers use argv[0] to find
    // their environment. Execute the approved resolved image, while preserving
    // the configured absolute invocation path rather than losing that context.
    config.command_argv0 = command.to_str().ok_or_else(||McpHostError::new("mcp_program_missing","MCP program path is not UTF-8"))?.into();
    let command = command.canonicalize().map_err(|error|McpHostError::new("mcp_program_missing",error.to_string()))?;
    config.command = command.to_str().ok_or_else(||McpHostError::new("mcp_program_missing","MCP program path is not UTF-8"))?.into();
    let digest = format!("{:x}",Sha256::digest(serde_json::to_vec(&json!({
        "command":config.command,"argv0":config.command_argv0,"args":config.args,"cwd":config.cwd,"env":config.environment,"timeout_ms":config.timeout_ms
    })).map_err(|error|McpHostError::new("mcp_identity_invalid",error.to_string()))?));
    let target = config.command.clone();
    let mut snapshots = launch_snapshots().lock().map_err(|_|McpHostError::new("mcp_registry_unavailable","MCP launch snapshots unavailable"))?;
    if snapshots.len() >= 128 && !snapshots.contains_key(&digest) {
        return Err(McpHostError::new("mcp_snapshot_limit","MCP launch snapshot limit reached"));
    }
    snapshots.insert(digest.clone(),config);
    Ok((target,digest))
}

pub fn connection_resource(reference:&str) -> Result<crate::mcp_connection_authority::ConnectionResource,McpHostError> {
    let entry = registry().read().map_err(|_|McpHostError::new("mcp_registry_unavailable","MCP registry unavailable"))?
        .get(reference).cloned().ok_or_else(||McpHostError::new("mcp_connection_required","MCP target is not registered"))?;
    let mut entry = entry.lock().map_err(|_|McpHostError::new("mcp_connection_unavailable","MCP connection unavailable"))?;
    if entry.retired { return Err(McpHostError::new("mcp_connection_retired","MCP connection retired")); }
    discard_exited_client(&mut entry)?;
    let mut resource = entry.resource.clone();
    if entry.client.is_none() { resource.generation = None; }
    Ok(resource)
}

pub fn cached_descriptor(reference:&str) -> Result<Option<McpServerDescriptor>,McpHostError> {
    let entry = registry().read().map_err(|_|McpHostError::new("mcp_registry_unavailable","MCP registry unavailable"))?
        .get(reference).cloned();
    let Some(entry) = entry else { return Ok(None); };
    let mut entry = entry.lock().map_err(|_|McpHostError::new("mcp_connection_unavailable","MCP connection unavailable"))?;
    discard_exited_client(&mut entry)?;
    Ok((!entry.retired && entry.client.is_some()).then(||entry.descriptor.clone()).flatten())
}

pub fn connect_approved(reference:&str,expected:&crate::mcp_connection_authority::ConnectionResource)
    -> Result<crate::mcp_connection_authority::ConnectionEstablished,McpHostError> {
    let entry = registry().read().map_err(|_|McpHostError::new("mcp_registry_unavailable","MCP registry unavailable"))?
        .get(reference).cloned().ok_or_else(||McpHostError::new("mcp_connection_required","MCP target is not registered"))?;
    let mut entry = entry.lock().map_err(|_|McpHostError::new("mcp_connection_unavailable","MCP connection unavailable"))?;
    discard_exited_client(&mut entry)?;
    let mut actual = entry.resource.clone();
    if entry.client.is_none() { actual.generation = None; }
    if entry.retired || reference != expected.connection_ref || actual != *expected {
        return Err(McpHostError::new("mcp_connection_changed","MCP resource changed; connection was not started"));
    }
    if entry.client.is_some() {
        let server = entry.descriptor.clone().ok_or_else(||McpHostError::new("mcp_descriptor_missing","Active MCP descriptor is missing"))?;
        return Ok(crate::mcp_connection_authority::ConnectionEstablished {server,resource:actual});
    }
    entry.resource.admission_id = uuid::Uuid::new_v4().to_string();
    let mut client = StdioMcpClient::connect(&entry.config)?;
    let descriptor = client.discover(entry.config.server_id.clone(),entry.config.display_name.clone(),reference.into())?;
    entry.resource.generation = Some(uuid::Uuid::new_v4().to_string());
    entry.descriptor = Some(descriptor.clone());
    entry.client = Some(client);
    Ok(crate::mcp_connection_authority::ConnectionEstablished {server:descriptor,resource:entry.resource.clone()})
}

pub fn refresh_connected(reference:&str) -> Result<(),McpHostError> {
    let entry = registry().read().map_err(|_|McpHostError::new("mcp_registry_unavailable","MCP registry unavailable"))?
        .get(reference).cloned();
    let Some(entry) = entry else { return Ok(()); };
    let mut entry = entry.lock().map_err(|_|McpHostError::new("mcp_connection_unavailable","MCP connection unavailable"))?;
    if entry.retired || entry.client.is_none() { return Ok(()); }
    let descriptor = entry.descriptor.clone().ok_or_else(||McpHostError::new("mcp_descriptor_missing","MCP descriptor missing"))?;
    match entry.client.as_mut().expect("checked client").discover(descriptor.server_id,descriptor.display_name,reference.into()) {
        Ok(descriptor) => { entry.descriptor = Some(descriptor); Ok(()) }
        Err(error) => { entry.client = None; entry.descriptor = None; Err(error) }
    }
}

pub fn call_tool(
    connection_ref: &str,
    tool_name: &str,
    arguments: Value,
) -> Result<McpToolExecution, McpHostError> {
    let output = request_capability(
        connection_ref,
        "tools/call",
        json!({"name": tool_name, "arguments": arguments}),
    )?;
    let is_error = output
        .get("isError")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let artifacts = super::mcp_host::mcp_result_artifacts(&output)?;
    Ok(McpToolExecution {
        output,
        is_error,
        artifacts,
    })
}

pub fn call_tool_with_cancellation(
    connection_ref: &str,
    tool_name: &str,
    arguments: Value,
    cancellation: &dyn crate::Cancellation,
) -> Result<McpToolExecution, McpHostError> {
    call_tool_with_cancellation_and_progress(
        connection_ref,
        tool_name,
        arguments,
        cancellation,
        &mut |_| {},
    )
}

pub fn call_tool_with_cancellation_and_progress(
    connection_ref: &str,
    tool_name: &str,
    arguments: Value,
    cancellation: &dyn crate::Cancellation,
    progress: &mut dyn FnMut(&Value),
) -> Result<McpToolExecution, McpHostError> {
    let output = request_capability_with_cancellation(
        connection_ref,
        "tools/call",
        json!({"name": tool_name, "arguments": arguments}),
        cancellation,
        progress,
    )?;
    let is_error = output
        .get("isError")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let artifacts = super::mcp_host::mcp_result_artifacts(&output)?;
    Ok(McpToolExecution {
        output,
        is_error,
        artifacts,
    })
}

pub fn request_capability(
    connection_ref: &str,
    method: &str,
    params: Value,
) -> Result<Value, McpHostError> {
    let connection = registry()
        .read()
        .map_err(|_| {
            McpHostError::new(
                "mcp_registry_unavailable",
                "MCP stdio registry is unavailable",
            )
        })?
        .get(connection_ref)
        .cloned()
        .ok_or_else(|| {
            McpHostError::new(
                "mcp_connection_unavailable",
                format!("MCP connection `{connection_ref}` is not registered by this Host"),
            )
        })?;
    let mut connection = connection.lock().map_err(|_| {
        McpHostError::new(
            "mcp_connection_unavailable",
            "MCP stdio connection state is unavailable",
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
        .expect("MCP stdio client initialized")
        .request(method, params);
    let output = match result {
        Ok(output) => output,
        Err(error) => {
            connection.client = None;
            return Err(error);
        }
    };
    Ok(output)
}

fn request_capability_with_cancellation(
    connection_ref: &str,
    method: &str,
    params: Value,
    cancellation: &dyn crate::Cancellation,
    progress: &mut dyn FnMut(&Value),
) -> Result<Value, McpHostError> {
    let connection = registry()
        .read()
        .map_err(|_| {
            McpHostError::new(
                "mcp_registry_unavailable",
                "MCP stdio registry is unavailable",
            )
        })?
        .get(connection_ref)
        .cloned()
        .ok_or_else(|| {
            McpHostError::new(
                "mcp_connection_unavailable",
                format!("MCP connection `{connection_ref}` is not registered by this Host"),
            )
        })?;
    let mut connection = connection.lock().map_err(|_| {
        McpHostError::new(
            "mcp_connection_unavailable",
            "MCP stdio connection state is unavailable",
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
        .expect("MCP stdio client initialized")
        .request_with_cancellation(method, params, cancellation, progress);
    match result {
        Ok(output) => Ok(output),
        Err(error) => {
            // A cancelled request may still emit a late response. Retaining
            // this stream would let that response satisfy the next request.
            connection.client = None;
            Err(error)
        }
    }
}

pub fn catalog_change_pending(value: &Value, index: usize) -> Result<bool, McpHostError> {
    let server_id = configured_server_id(value, index)?;
    let config = parse_config(value, &server_id)?;
    let connection = registry()
        .read()
        .map_err(|_| {
            McpHostError::new(
                "mcp_registry_unavailable",
                "MCP stdio registry is unavailable",
            )
        })?
        .get(&config.connection_ref)
        .cloned();
    let Some(connection) = connection else {
        return Ok(false);
    };
    let connection = connection.lock().map_err(|_| {
        McpHostError::new(
            "mcp_connection_unavailable",
            "MCP stdio connection state is unavailable",
        )
    })?;
    Ok(connection
        .client
        .as_ref()
        .is_some_and(|client| client.catalog_dirty))
}

fn install(config: StdioMcpConfig, client: Option<StdioMcpClient>, mut resource:crate::mcp_connection_authority::ConnectionResource) -> Result<(), McpHostError> {
    let mut connections = registry().write().map_err(|_| {
        McpHostError::new(
            "mcp_registry_unavailable",
            "MCP stdio registry is unavailable",
        )
    })?;
    if let Some(existing) = connections.get(&config.connection_ref) {
        let mut existing = existing.lock().map_err(|_| {
            McpHostError::new(
                "mcp_connection_unavailable",
                "MCP stdio connection state is unavailable",
            )
        })?;
        if existing.config == config {
            if client.is_some() {
                existing.client = client;
            }
            return Ok(());
        }
    }
    resource.admission_id = uuid::Uuid::new_v4().to_string();
    connections.insert(
        config.connection_ref.clone(),
        Arc::new(Mutex::new(StdioConnectionEntry { config, client, retired: false,resource,descriptor:None })),
    );
    Ok(())
}

fn parse_config(value: &Value, server_id: &str) -> Result<StdioMcpConfig, McpHostError> {
    // OpenCode's local MCP source embeds argv in command. Adapt that source
    // shape once on the Host, keeping argv boundaries and existing validation.
    let mut normalized;
    let value = if let Some(argv) = value.get("command").and_then(Value::as_array) {
        if value.get("args").is_some() {
            return Err(McpHostError::new("mcp_arguments_invalid", "Use command argv or separate args, not both"));
        }
        let (command, args) = argv.split_first().ok_or_else(||
            McpHostError::new("mcp_command_invalid", "MCP command argv is empty"))?;
        normalized = value.clone();
        normalized["command"] = command.clone();
        normalized["args"] = Value::Array(args.to_vec());
        &normalized
    } else { value };
    let command = value
        .get("command")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|command| !command.is_empty() && command.len() <= 4_096)
        .ok_or_else(|| {
            McpHostError::new(
                "mcp_command_invalid",
                "MCP stdio command is missing or too long",
            )
        })?
        .to_string();
    let args = value
        .get("args")
        .map(|args| {
            let args = args.as_array().ok_or_else(|| {
                McpHostError::new("mcp_arguments_invalid", "MCP stdio args must be an array")
            })?;
            if args.len() > 128 {
                return Err(McpHostError::new(
                    "mcp_arguments_invalid",
                    "MCP stdio has too many arguments",
                ));
            }
            args.iter()
                .map(|argument| {
                    argument
                        .as_str()
                        .filter(|argument| argument.len() <= 16_384 && !argument.contains('\0'))
                        .map(str::to_string)
                        .ok_or_else(|| {
                            McpHostError::new(
                                "mcp_arguments_invalid",
                                "MCP stdio arguments must be bounded text",
                            )
                        })
                })
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?
        .unwrap_or_default();
    let environment = value
        .get("env")
        .or_else(|| value.get("environment"))
        .map(|environment| {
            let environment = environment.as_object().ok_or_else(|| {
                McpHostError::new("mcp_environment_invalid", "MCP stdio env must be an object")
            })?;
            if environment.len() > 128 {
                return Err(McpHostError::new(
                    "mcp_environment_invalid",
                    "MCP stdio env has too many entries",
                ));
            }
            environment
                .iter()
                .map(|(key, value)| {
                    let value = value
                        .as_str()
                        .filter(|value| value.len() <= 65_536)
                        .ok_or_else(|| {
                            McpHostError::new(
                                "mcp_environment_invalid",
                                "MCP stdio env values must be bounded text",
                            )
                        })?;
                    if key.is_empty()
                        || key.contains('=')
                        || key.contains('\0')
                        || value.contains('\0')
                    {
                        return Err(McpHostError::new(
                            "mcp_environment_invalid",
                            "MCP stdio env contains an invalid name or value",
                        ));
                    }
                    Ok((key.clone(), value.to_string()))
                })
                .collect::<Result<BTreeMap<_, _>, _>>()
        })
        .transpose()?
        .unwrap_or_default();
    let cwd = value
        .get("cwd")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|cwd| !cwd.is_empty())
        .map(PathBuf::from);
    let timeout_ms = value
        .get("timeout_ms")
        .or_else(|| value.get("timeoutMs"))
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_TIMEOUT_MS)
        .clamp(1_000, MAX_TIMEOUT_MS);
    let connection_ref = value
        .get("connection_ref")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_default(); // Private pre-binding parse; never a registry authorization key.
    Ok(StdioMcpConfig {
        server_id: server_id.into(),
        display_name: value.get("display_name").or_else(||value.get("name")).and_then(Value::as_str).unwrap_or(server_id).into(),
        command_argv0: command.clone(),
        command,
        args,
        environment,
        cwd,
        timeout_ms,
        connection_ref,
    })
}

impl StdioMcpClient {
    fn connect(config: &StdioMcpConfig) -> Result<Self, McpHostError> {
        let mut command = Command::new(&config.command);
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.arg0(&config.command_argv0);
        }
        command
            .args(&config.args)
            .env_clear()
            .envs(&config.environment)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(cwd) = &config.cwd {
            command.current_dir(cwd);
        }
        let child = command.spawn().map_err(|error| {
            McpHostError::new(
                "mcp_process_start_failed",
                format!("Failed to start MCP server: {error}"),
            )
        })?;
        let mut child = InitializingChild(Some(child));
        let stdin = child.0.as_mut().expect("owned initializing child").stdin.take().ok_or_else(|| {
            McpHostError::new("mcp_process_pipe_failed", "MCP stdin is unavailable")
        })?;
        let stdout = child.0.as_mut().expect("owned initializing child").stdout.take().ok_or_else(|| {
            McpHostError::new("mcp_process_pipe_failed", "MCP stdout is unavailable")
        })?;
        let stderr = child.0.as_mut().expect("owned initializing child").stderr.take().ok_or_else(|| {
            McpHostError::new("mcp_process_pipe_failed", "MCP stderr is unavailable")
        })?;
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) => {
                        let _ = sender.send(StdoutEvent::Closed);
                        break;
                    }
                    Ok(_) if line.len() > MAX_MESSAGE_BYTES => {
                        let _ = sender.send(StdoutEvent::Failed(
                            "MCP stdio response exceeded the 4 MB limit".to_string(),
                        ));
                        break;
                    }
                    Ok(_) => {
                        let _ = sender.send(StdoutEvent::Line(line));
                    }
                    Err(error) => {
                        let _ = sender.send(StdoutEvent::Failed(error.to_string()));
                        break;
                    }
                }
            }
        });
        let stderr_tail = Arc::new(Mutex::new(Vec::new()));
        let stderr_capture = Arc::clone(&stderr_tail);
        thread::spawn(move || {
            let mut stderr = stderr;
            let mut chunk = [0_u8; 4_096];
            while let Ok(count) = stderr.read(&mut chunk) {
                if count == 0 {
                    break;
                }
                if let Ok(mut tail) = stderr_capture.lock() {
                    tail.extend_from_slice(&chunk[..count]);
                    if tail.len() > MAX_STDERR_BYTES {
                        let overflow = tail.len() - MAX_STDERR_BYTES;
                        tail.drain(..overflow);
                    }
                }
            }
        });
        let mut client = Self {
            child:child.0.take().expect("transfer initialized child"),
            stdin,
            stdout_events: receiver,
            stderr_tail,
            timeout: Duration::from_millis(config.timeout_ms),
            protocol_version: "2025-06-18".to_string(),
            server_instructions: None,
            next_request_id: 1,
            catalog_dirty: false,
        };
        let initialized = client.request(
            "initialize",
            json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "nola", "version": env!("CARGO_PKG_VERSION")}
            }),
        )?;
        client.protocol_version = initialized
            .get("protocolVersion")
            .and_then(Value::as_str)
            .unwrap_or("2025-06-18")
            .to_string();
        client.server_instructions = nola_core::agent::capability::bounded_mcp_server_instructions(
            initialized.get("instructions").and_then(Value::as_str),
        );
        client.notification("notifications/initialized", json!({}))?;
        Ok(client)
    }

    fn request(&mut self, method: &str, params: Value) -> Result<Value, McpHostError> {
        let id = self.next_request_id;
        self.next_request_id = self.next_request_id.saturating_add(1);
        self.write_payload(&json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))?;
        let deadline = Instant::now() + self.timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(McpHostError::new(
                    "mcp_request_timeout",
                    format!("MCP method `{method}` timed out"),
                ));
            }
            match self.stdout_events.recv_timeout(remaining) {
                Ok(StdoutEvent::Line(line)) => {
                    let decoded: Value = serde_json::from_str(line.trim()).map_err(|error| {
                        McpHostError::new(
                            "mcp_response_invalid",
                            format!("Invalid MCP stdio JSON: {error}"),
                        )
                    })?;
                    if decoded.get("id").is_none() {
                        self.catalog_dirty |= is_catalog_change_notification(&decoded);
                        continue;
                    }
                    return decode_rpc_response(line.trim(), Some(id));
                }
                Ok(StdoutEvent::Failed(error)) => {
                    return Err(McpHostError::new("mcp_response_read_failed", error))
                }
                Ok(StdoutEvent::Closed) => {
                    return Err(McpHostError::new("mcp_process_exited", self.exit_message()))
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    return Err(McpHostError::new(
                        "mcp_request_timeout",
                        format!("MCP method `{method}` timed out"),
                    ))
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(McpHostError::new("mcp_process_exited", self.exit_message()))
                }
            }
        }
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
                "MCP stdio request was cancelled before transport send",
            ));
        }
        let id = self.next_request_id;
        self.next_request_id = self.next_request_id.saturating_add(1);
        self.write_payload(&json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))?;
        let deadline = Instant::now() + self.timeout;
        loop {
            if cancellation.is_requested() {
                let notification = self.notification(
                    "notifications/cancelled",
                    json!({"requestId":id,"reason":"Cancelled by user"}),
                );
                if notification.is_ok() {
                    // Give the retained server a bounded opportunity to read
                    // the notification and finish its active request. The
                    // connection is discarded by the caller either way.
                    let _ = self.stdout_events.recv_timeout(Duration::from_millis(100));
                }
                let detail = notification
                    .err()
                    .map(|error| format!("; cancellation notification failed: {}", error.message))
                    .unwrap_or_default();
                return Err(McpHostError::new(
                    "mcp_cancelled_after_send",
                    format!(
                        "MCP stdio cancellation notification was sent after request {id}; verify side effects before retrying{detail}"
                    ),
                ));
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(McpHostError::new(
                    "mcp_request_timeout",
                    format!("MCP method `{method}` timed out"),
                ));
            }
            match self
                .stdout_events
                .recv_timeout(remaining.min(Duration::from_millis(50)))
            {
                Ok(StdoutEvent::Line(line)) => {
                    let decoded: Value = serde_json::from_str(line.trim()).map_err(|error| {
                        McpHostError::new(
                            "mcp_response_invalid",
                            format!("Invalid MCP stdio JSON: {error}"),
                        )
                    })?;
                    if decoded.get("id").is_none() {
                        self.catalog_dirty |= is_catalog_change_notification(&decoded);
                        if decoded.get("method").and_then(Value::as_str)
                            == Some("notifications/progress")
                        {
                            progress(decoded.get("params").unwrap_or(&Value::Null));
                        }
                        continue;
                    }
                    return decode_rpc_response(line.trim(), Some(id));
                }
                Ok(StdoutEvent::Failed(error)) => {
                    return Err(McpHostError::new("mcp_response_read_failed", error))
                }
                Ok(StdoutEvent::Closed) => {
                    return Err(McpHostError::new("mcp_process_exited", self.exit_message()))
                }
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(McpHostError::new("mcp_process_exited", self.exit_message()))
                }
            }
        }
    }

    fn notification(&mut self, method: &str, params: Value) -> Result<(), McpHostError> {
        self.write_payload(&json!({"jsonrpc":"2.0","method":method,"params":params}))
    }

    fn write_payload(&mut self, payload: &Value) -> Result<(), McpHostError> {
        let mut bytes = serde_json::to_vec(payload)
            .map_err(|error| McpHostError::new("mcp_request_invalid", error.to_string()))?;
        bytes.push(b'\n');
        self.stdin
            .write_all(&bytes)
            .and_then(|_| self.stdin.flush())
            .map_err(|error| {
                McpHostError::new(
                    "mcp_connection_failed",
                    format!("MCP stdio write failed: {error}"),
                )
            })
    }

    fn discover(
        &mut self,
        server_id: String,
        display_name: String,
        connection_ref: String,
    ) -> Result<McpServerDescriptor, McpHostError> {
        let tools = self.list_pages("tools/list", "tools", false)?;
        let resources = self.list_pages("resources/list", "resources", true)?;
        let templates = self.list_pages("resources/templates/list", "resourceTemplates", true)?;
        let prompts = self.list_pages("prompts/list", "prompts", true)?;
        Ok(McpServerDescriptor {
            server_id,
            display_name,
            transport: McpTransportKind::Stdio,
            connection_ref,
            availability: McpServerAvailability::Available,
            protocol_version: Some(self.protocol_version.clone()),
            instructions: self.server_instructions.clone(),
            tools: tools
                .into_iter()
                .filter_map(decode_tool)
                .map(|tool| (tool.name.clone(), tool))
                .collect(),
            resources: resources
                .into_iter()
                .filter_map(decode_resource)
                .map(|resource| (resource.resource_id.clone(), resource))
                .collect(),
            resource_templates: templates
                .into_iter()
                .filter_map(decode_resource_template)
                .map(|template| (template.template_id.clone(), template))
                .collect(),
            prompts: prompts
                .into_iter()
                .filter_map(decode_prompt)
                .map(|prompt| (prompt.name.clone(), prompt))
                .collect(),
        })
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
                .map(|cursor| json!({"cursor":cursor}))
                .unwrap_or_else(|| json!({}));
            let result = match self.request(method, params) {
                Ok(result) => result,
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
                .filter(|cursor| !cursor.is_empty())
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

    fn exit_message(&mut self) -> String {
        // The stdout reader can observe EOF a few milliseconds before the OS
        // makes the child's exit status available to `try_wait`. Preserve the
        // real status for diagnostics without allowing a broken MCP process to
        // hold the request path indefinitely.
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(100);
        let status = loop {
            match self.child.try_wait() {
                Ok(Some(status)) => break status.to_string(),
                Ok(None) if std::time::Instant::now() < deadline => {
                    std::thread::sleep(std::time::Duration::from_millis(2));
                }
                Ok(None) => break "unknown status".to_string(),
                Err(error) => break format!("status unavailable: {error}"),
            }
        };
        let stderr = self
            .stderr_tail
            .lock()
            .ok()
            .map(|tail| String::from_utf8_lossy(&tail).trim().to_string())
            .unwrap_or_default();
        if stderr.is_empty() {
            format!("MCP stdio process exited ({status})")
        } else {
            format!("MCP stdio process exited ({status}): {stderr}")
        }
    }
}

impl Drop for StdioMcpClient {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(windows)]
    #[test]
    fn frozen_path_resolves_native_executable_extension_without_current_directory_search() {
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("fixture.exe");
        std::fs::write(&executable, b"not launched").unwrap();
        let configured = json!({"type":"stdio","command":"fixture","cwd":directory.path(),
            "env":{"PATH":directory.path()}});
        let (program, _) = freeze_configuration(&configured, "native-extension").unwrap();
        assert_eq!(PathBuf::from(program), executable.canonicalize().unwrap());
        let missing = json!({"type":"stdio","command":"fixture","cwd":directory.path(),"env":{"PATH":""}});
        assert!(freeze_configuration(&missing, "no-cwd-search").is_err());
    }

    #[test]
    fn stdio_registration_freezes_private_minimal_environment_without_launching() {
        let directory = tempfile::tempdir().unwrap();
        let executable = std::env::current_exe().unwrap();
        let configured = crate::mcp_connection_authority::bind_configurations(&[json!({
            "server_id":"private-launch","type":"stdio","command":executable,"cwd":directory.path(),
            "env":{"EXPLICIT_SERVER_TOKEN":"synthetic-private-value"}
        })],"private-host",&uuid::Uuid::new_v4().to_string()).unwrap();
        register_configured_server(&configured[0],0).unwrap();
        let reference = configured[0]["connection_ref"].as_str().unwrap();
        let resource = connection_resource(reference).unwrap();
        assert!(resource.generation.is_none());
        assert!(!serde_json::to_string(&resource).unwrap().contains("synthetic-private-value"));
        assert_eq!(configured[0]["env"],json!({"EXPLICIT_SERVER_TOKEN":"synthetic-private-value"}));
        let snapshots = launch_snapshots().lock().unwrap();
        let launch = &snapshots[&resource.config_digest];
        let allowed = ["PATH","HOME","USERPROFILE","SYSTEMROOT","WINDIR","COMSPEC","PATHEXT",
            "TEMP","TMP","TMPDIR","LANG","LC_ALL","LC_CTYPE","TZ","EXPLICIT_SERVER_TOKEN"];
        assert!(launch.environment.keys().all(|key|allowed.contains(&key.as_str())));
        assert_eq!(launch.environment["EXPLICIT_SERVER_TOKEN"],"synthetic-private-value");
        assert!(registry().read().unwrap()[reference].lock().unwrap().client.is_none());
        assert_eq!(request_capability(reference,"tools/list",json!({})).unwrap_err().code,"mcp_connection_required");
        assert!(directory.path().read_dir().unwrap().next().is_none());
    }

    fn approved_stdio_fixture(value:&Value,server_id:String,display_name:String) -> Result<McpServerDescriptor,McpHostError> {
        let mut value = value.clone();
        value["server_id"] = json!(server_id);
        value["display_name"] = json!(display_name);
        let values = crate::mcp_connection_authority::connect_fixture_servers(&[value])?;
        serde_json::from_value(values[0].clone()).map_err(|error|McpHostError::new("mcp_fixture_invalid",error.to_string()))
    }

    #[test]
    fn retirement_revokes_admission_before_draining_without_holding_registry_lock() {
        let executable = std::env::current_exe().unwrap();
        let values = crate::mcp_connection_authority::bind_configurations(&[
            json!({"server_id":"retirement", "type":"stdio","command":executable})],
            "retirement-host",&uuid::Uuid::new_v4().to_string()).unwrap();
        let reference = values[0]["connection_ref"].as_str().unwrap().to_owned();
        register_configured_server(&values[0], 0).unwrap();
        let entry = registry().read().unwrap().get(&reference).unwrap().clone();
        let in_flight = entry.lock().unwrap();
        let (revoked_tx, revoked_rx) = mpsc::sync_channel(1);
        let (drained_tx, drained_rx) = mpsc::sync_channel(1);
        let owned_ref = reference.clone();
        let worker = thread::spawn(move || {
            let retirement = revoke_connection(&owned_ref);
            revoked_tx.send(()).unwrap();
            drop(retirement);
            drained_tx.send(()).unwrap();
        });
        revoked_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(request_capability(&reference, "tools/list", json!({})).unwrap_err().code,
            "mcp_connection_unavailable");
        let values = crate::mcp_connection_authority::bind_configurations(&[
            json!({"server_id":"unrelated", "type":"stdio","command":executable})],
            "retirement-host",&uuid::Uuid::new_v4().to_string()).unwrap();
        let other = values[0]["connection_ref"].as_str().unwrap().to_owned();
        register_configured_server(&values[0], 0).unwrap();
        assert!(matches!(drained_rx.try_recv(), Err(mpsc::TryRecvError::Empty)));
        drop(in_flight);
        drained_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        worker.join().unwrap();
        assert!(entry.lock().unwrap().retired);
        assert!(registry().read().unwrap().contains_key(&other));
        drop(revoke_connection(&other));
    }

    #[test]
    fn retained_stdio_call_sends_protocol_cancellation_and_discards_connection() {
        let marker = std::env::temp_dir().join(format!(
            "nola-mcp-cancel-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let script = r#"
import json, os, sys
marker = sys.argv[1]
for line in sys.stdin:
    request = json.loads(line)
    method = request.get('method', '')
    request_id = request.get('id')
    if method == 'notifications/initialized':
        continue
    if method == 'initialize':
        result = {'protocolVersion':'2025-06-18','capabilities':{}}
    elif method == 'tools/list':
        result = {'tools':[{'name':'slow_write','description':'slow mutation','inputSchema':{'type':'object'}}]}
    elif method in ('resources/list','resources/templates/list','prompts/list'):
        print(json.dumps({'jsonrpc':'2.0','id':request_id,'error':{'code':-32601,'message':'Method not found'}}), flush=True)
        continue
    elif method == 'tools/call':
        print(json.dumps({'jsonrpc':'2.0','method':'notifications/progress','params':{'progressToken':'phase','progress':1,'total':4,'message':'Preparing document'}}), flush=True)
        for cancellation_line in sys.stdin:
            cancellation = json.loads(cancellation_line)
            if cancellation.get('method') == 'notifications/cancelled':
                with open(marker, 'w') as output:
                    output.write(str(cancellation.get('params', {}).get('requestId')))
                print(json.dumps({'jsonrpc':'2.0','id':request_id,'result':{'isError':True,'content':[{'type':'text','text':'cancelled'}]}}), flush=True)
                break
        continue
    else:
        raise RuntimeError('unexpected method ' + method)
    print(json.dumps({'jsonrpc':'2.0','id':request_id,'result':result}), flush=True)
"#;
        let descriptor = approved_stdio_fixture(
            &json!({
                "command":"/usr/bin/env",
                "args":["python3","-u","-c",script,marker.to_string_lossy()],
                "timeout_ms":5_000,
                "connection_ref":"stdio:cancel-e2e"
            }),
            "cancel-e2e".to_string(),
            "Cancellation E2E".to_string(),
        )
        .expect("discover retained stdio server");
        let cancellation = crate::CancellationToken::default();
        let canceller = cancellation.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            canceller.request();
        });
        let mut progress_events = Vec::new();
        let error = call_tool_with_cancellation_and_progress(
            &descriptor.connection_ref,
            "slow_write",
            json!({"path":"unknown-side-effect.txt"}),
            &cancellation,
            &mut |progress| progress_events.push(progress.clone()),
        )
        .expect_err("cancelled stdio call");
        assert_eq!(error.code, "mcp_cancelled_after_send");
        assert!(error.message.contains("cancellation notification was sent"));
        assert_eq!(progress_events.len(), 1);
        assert_eq!(progress_events[0]["message"], "Preparing document");
        let deadline = Instant::now() + Duration::from_secs(1);
        while !marker.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(std::fs::read_to_string(&marker).unwrap(), "6");
        let _ = std::fs::remove_file(marker);
        let entry = registry()
            .read()
            .unwrap()
            .get(&descriptor.connection_ref)
            .cloned()
            .unwrap();
        assert!(entry.lock().unwrap().client.is_none());
    }

    #[test]
    fn retained_stdio_mutation_crash_is_not_retried_and_discards_connection() {
        let marker = std::env::temp_dir().join(format!(
            "nola-mcp-crash-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let script = r#"
import json, os, sys
marker = sys.argv[1]
for line in sys.stdin:
    request = json.loads(line)
    method = request.get('method', '')
    request_id = request.get('id')
    if method == 'notifications/initialized':
        continue
    if method == 'initialize':
        result = {'protocolVersion':'2025-06-18','capabilities':{}}
    elif method == 'tools/list':
        result = {'tools':[{'name':'crash_after_write','description':'mutate then crash','inputSchema':{'type':'object'}}]}
    elif method in ('resources/list','resources/templates/list','prompts/list'):
        print(json.dumps({'jsonrpc':'2.0','id':request_id,'error':{'code':-32601,'message':'Method not found'}}), flush=True)
        continue
    elif method == 'tools/call':
        count = 0
        if os.path.exists(marker):
            with open(marker) as current:
                count = int(current.read() or '0')
        with open(marker, 'w') as output:
            output.write(str(count + 1))
        os._exit(17)
    else:
        raise RuntimeError('unexpected method ' + method)
    print(json.dumps({'jsonrpc':'2.0','id':request_id,'result':result}), flush=True)
"#;
        let descriptor = approved_stdio_fixture(
            &json!({
                "command":"/usr/bin/env",
                "args":["python3","-u","-c",script,marker.to_string_lossy()],
                "timeout_ms":5_000,
                "connection_ref":"stdio:crash-after-mutate-e2e"
            }),
            "crash-after-mutate-e2e".to_string(),
            "Crash After Mutate E2E".to_string(),
        )
        .expect("discover retained stdio server");
        let cancellation = crate::CancellationToken::default();
        let error = call_tool_with_cancellation(
            &descriptor.connection_ref,
            "crash_after_write",
            json!({"path":"unknown-side-effect.txt"}),
            &cancellation,
        )
        .expect_err("server crash after mutation must remain an error");
        assert_eq!(error.code, "mcp_process_exited");
        assert!(error.message.contains("status: 17") || error.message.contains("exit status: 17"));
        assert_eq!(std::fs::read_to_string(&marker).unwrap(), "1");
        let entry = registry()
            .read()
            .unwrap()
            .get(&descriptor.connection_ref)
            .cloned()
            .unwrap();
        assert!(entry.lock().unwrap().client.is_none());
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(std::fs::read_to_string(&marker).unwrap(), "1");
        let _ = std::fs::remove_file(marker);
    }

    #[test]
    fn retained_stdio_mutation_timeout_is_not_retried_and_discards_connection() {
        let marker = std::env::temp_dir().join(format!(
            "nola-mcp-timeout-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let script = r#"
import json, os, sys, time
marker = sys.argv[1]
for line in sys.stdin:
    request = json.loads(line)
    method = request.get('method', '')
    request_id = request.get('id')
    if method == 'notifications/initialized':
        continue
    if method == 'initialize':
        result = {'protocolVersion':'2025-06-18','capabilities':{}}
    elif method == 'tools/list':
        result = {'tools':[{'name':'hang_after_write','description':'mutate then hang','inputSchema':{'type':'object'}}]}
    elif method in ('resources/list','resources/templates/list','prompts/list'):
        print(json.dumps({'jsonrpc':'2.0','id':request_id,'error':{'code':-32601,'message':'Method not found'}}), flush=True)
        continue
    elif method == 'tools/call':
        with open(marker, 'w') as output:
            output.write('1')
        time.sleep(30)
        continue
    else:
        raise RuntimeError('unexpected method ' + method)
    print(json.dumps({'jsonrpc':'2.0','id':request_id,'result':result}), flush=True)
"#;
        let descriptor = approved_stdio_fixture(
            &json!({
                "command":"/usr/bin/env",
                "args":["python3","-u","-c",script,marker.to_string_lossy()],
                "timeout_ms":150,
                "connection_ref":"stdio:timeout-after-mutate-e2e"
            }),
            "timeout-after-mutate-e2e".to_string(),
            "Timeout After Mutate E2E".to_string(),
        )
        .expect("discover retained stdio server");
        let cancellation = crate::CancellationToken::default();
        let started = Instant::now();
        let error = call_tool_with_cancellation(
            &descriptor.connection_ref,
            "hang_after_write",
            json!({"path":"unknown-side-effect.txt"}),
            &cancellation,
        )
        .expect_err("server timeout after mutation must remain an error");
        assert_eq!(error.code, "mcp_request_timeout");
        assert!(started.elapsed() < Duration::from_secs(2));
        assert_eq!(std::fs::read_to_string(&marker).unwrap(), "1");
        let entry = registry()
            .read()
            .unwrap()
            .get(&descriptor.connection_ref)
            .cloned()
            .unwrap();
        assert!(entry.lock().unwrap().client.is_none());
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(std::fs::read_to_string(&marker).unwrap(), "1");
        let _ = std::fs::remove_file(marker);
    }

    #[test]
    fn stdio_config_preserves_argv_and_environment_without_shell_parsing() {
        let config = parse_config(
            &json!({
                "command": "example-mcp",
                "args": ["--query", "a; rm -rf /"],
                "env": {"TOKEN": "secret"},
                "cwd": "/tmp",
                "connection_ref": "stdio:test"
            }),
            "test",
        )
        .expect("valid config");
        assert_eq!(config.command, "example-mcp");
        assert_eq!(config.args, vec!["--query", "a; rm -rf /"]);
        assert_eq!(
            config.environment.get("TOKEN").map(String::as_str),
            Some("secret")
        );
        assert_eq!(config.connection_ref, "stdio:test");
        let imported = json!({"type":"local","command":["example-mcp","--query","a; rm -rf /"],
            "environment":{"TOKEN":"secret"}});
        assert!(is_stdio_configuration(&imported));
        let imported = parse_config(&imported, "test").unwrap();
        assert_eq!(imported.command, config.command);
        assert_eq!(imported.args, config.args);
        assert_eq!(imported.environment, config.environment);
        assert!(parse_config(&json!({"type":"local","command":[]}), "test").is_err());
        assert!(parse_config(&json!({"command":["program"],"args":["ambiguous"]}), "test").is_err());
    }

    #[test]
    fn list_change_notifications_are_distinct_from_rpc_responses() {
        assert!(is_catalog_change_notification(&json!({
            "jsonrpc": "2.0",
            "method": "notifications/resources/list_changed"
        })));
        assert!(!is_catalog_change_notification(&json!({
            "jsonrpc": "2.0",
            "id": 4,
            "result": {"resources": []}
        })));
    }
}
