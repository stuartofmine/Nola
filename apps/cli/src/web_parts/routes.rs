use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::fs;
use std::io::{Cursor, Read};
use std::net::ToSocketAddrs;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration as StdDuration, Instant, SystemTime, UNIX_EPOCH};

use base64::{engine::general_purpose, Engine as _};
use chrono::{Duration, Local, Utc};
use nola_core::agent::protocol::{AgentItem, AgentRole};
use nola_core::agent::{
    GoalRuntimeCommand, GoalRuntimeCommandEnvelope,
};
#[cfg(test)] use nola_core::agent::GoalStatus;
use nola_core::agent_trace::{AgentTraceEvent, AgentTraceKind};
#[cfg(test)]
use nola_core::context_engine::ContextPrepareOutput;
use nola_core::context_engine::{
    estimate_json_values_tokens, prepare_context, ContextBudgetSpec, ContextComponent,
    ContextComponentKind, ContextComponentSource, ContextPlatform, ContextPrepareInput,
    ContextPrepareMode, CONTEXT_ENGINE_SCHEMA_VERSION,
};
use nola_core::device_bridge::DeviceBridgeAdvertisement;
use nola_core::host_auth::{
    validate_host_permission_decision, HostPermissionAuthorizationDecision,
    HostPermissionChallenge, HostPermissionIntervention,
    HostPermissionResolution as WireHostPermissionResolution, HostPermissionResponse,
    HOST_PERMISSION_PROTOCOL_VERSION,
};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

use crate::runtime_output::RuntimeOutputPort;
use crate::builtin_skills;
use crate::config::Config;
use crate::core_agent_runtime::{
    compact_agent_core_session, run_agent_core_turn_with_cancellation,
    session_message_to_agent_items, submit_agent_core_goal_command,
};
use crate::data_sources::{DataSourceDraft, DataSourceStore, StoredDataSource};
use crate::runtime_control::RuntimeExecutionControl;
use crate::llm;
use crate::local_models;
use crate::store::NolaStore;
use crate::tool_policy::{candidates_from_schemas, resolve_tool_schemas, ToolPlanOptions};
use crate::tools::{
    enter_device_bridge_resolver, enter_device_bridge_tools, enter_host_permission_resolver,
    enter_host_permission_session, take_pending_host_permission_trace, tool_is_read_only,
    tool_spec_category, DeviceBridgeRuntimeTool,
    HostPermissionResolution as ToolPermissionResolution, ToolOutcome, ToolRequest,
};
use nola_core::tool_policy::{ToolPlatform, ToolRoute};
#[cfg(test)]
use crate::tools::tool_schemas;

static ID_COUNTER: AtomicU64 = AtomicU64::new(1);
static NATIVE_BRIDGES: OnceLock<Mutex<NativeBridgeStore>> = OnceLock::new();
static MOBILE_TOKENS: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();
static WEB_HOST_ACCESS_TOKEN: OnceLock<String> = OnceLock::new();
static WEB_BROWSER_SESSIONS: OnceLock<Mutex<HashMap<String, WebBrowserSession>>> = OnceLock::new();
static WEB_BROWSER_HANDOFFS: OnceLock<Mutex<HashMap<String, WebBrowserHandoff>>> = OnceLock::new();
static WEB_LOGIN_FAILURES: OnceLock<Mutex<HashMap<String, (u32, Instant)>>> = OnceLock::new();
static MOBILE_AUTH_NONCES: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();
static MOBILE_LOGIN_FAILURES: OnceLock<Mutex<HashMap<String, (u32, Instant)>>> = OnceLock::new();
static HOST_IDENTITY: OnceLock<Result<String, String>> = OnceLock::new();
static HOST_PERMISSION_CHALLENGES: OnceLock<Mutex<HashMap<String, HostPermissionChallenge>>> =
    OnceLock::new();
static ACTIVE_WEB_AGENT_RUNS: OnceLock<Mutex<HashMap<String, RuntimeExecutionControl>>> =
    OnceLock::new();
static DATA_SOURCE_STATE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

const WEB_SESSION_COOKIE: &str = "nola_web_session";
const WEB_CSRF_COOKIE: &str = "nola_web_csrf";
const WEB_SESSION_TTL_SEC: u64 = 12 * 60 * 60;
const WEB_SESSION_LIMIT: usize = 32;
const WEB_HANDOFF_TTL_SEC: u64 = 60;
const WEB_HANDOFF_LIMIT: usize = 64;
const WEB_LOGIN_FAILURE_LIMIT: u32 = 6;
const WEB_LOGIN_FAILURE_WINDOW_SEC: u64 = 5 * 60;

#[derive(Debug, Clone)]
struct WebBrowserSession {
    csrf_digest: String,
    created_at: Instant,
    expires_at: Instant,
}

#[derive(Debug, Clone)]
struct WebBrowserHandoff {
    target: String,
    created_at: Instant,
    expires_at: Instant,
}
static HOST_PERMISSION_WAITERS: OnceLock<
    Mutex<HashMap<String, mpsc::SyncSender<ToolPermissionResolution>>>,
> = OnceLock::new();
static HOST_DEVICE_TOOL_CHALLENGES: OnceLock<Mutex<HashMap<String, PendingDeviceToolChallenge>>> =
    OnceLock::new();
static HOST_DEVICE_TOOL_WAITERS: OnceLock<
    Mutex<HashMap<String, mpsc::SyncSender<nola_core::device_bridge::DeviceToolResult>>>,
> = OnceLock::new();
const MAX_LOCAL_FILE_BYTES: u64 = 64_000;
const MAX_LOCAL_FILE_CHARS: usize = 12_000;
const MAX_FOLDER_FILES: usize = 20;
const MAX_SCRIPT_CHARS: usize = 12_000;
const MAX_SCRIPT_LOG_CHARS: usize = 20_000;
const DEFAULT_SCRIPT_TIMEOUT_SEC: u64 = 30;
const MAX_SCRIPT_TIMEOUT_SEC: u64 = 120;
const NATIVE_HOST_STALE_SEC: u64 = 30;
const NATIVE_COMMAND_STALE_SEC: u64 = 300;
const MOBILE_TOKEN_TTL_SEC: u64 = 60 * 60 * 24 * 30;
const MOBILE_ATTACHMENT_MAX_BYTES: usize = 20 * 1024 * 1024;
const MOBILE_ATTACHMENT_MAX_CHARS: usize = 40_000;
const MAX_WEB_REQUEST_BODY_BYTES: usize = 32 * 1024 * 1024;
const MAX_CONCURRENT_WEB_REQUESTS: usize = 64;
static ACTIVE_WEB_REQUESTS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

struct ActiveWebRequestGuard;

impl Drop for ActiveWebRequestGuard {
    fn drop(&mut self) {
        ACTIVE_WEB_REQUESTS.fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
    }
}

fn try_acquire_web_request() -> Option<ActiveWebRequestGuard> {
    let previous = ACTIVE_WEB_REQUESTS.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
    if previous >= MAX_CONCURRENT_WEB_REQUESTS {
        ACTIVE_WEB_REQUESTS.fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
        None
    } else {
        Some(ActiveWebRequestGuard)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NativeBridgeKind {
    Browser,
}

#[derive(Debug, Clone)]
struct NativeBridgeStore {
    browser: NativeBridgeQueue,
}

impl Default for NativeBridgeStore {
    fn default() -> Self {
        Self {
            browser: NativeBridgeQueue::new(),
        }
    }
}

#[derive(Debug, Clone)]
struct NativeBridgeQueue {
    hosts: HashMap<String, NativeHost>,
    commands: HashMap<String, NativeCommand>,
    pending: VecDeque<String>,
}

impl NativeBridgeQueue {
    fn new() -> Self {
        Self {
            hosts: HashMap::new(),
            commands: HashMap::new(),
            pending: VecDeque::new(),
        }
    }
}

#[derive(Debug, Clone)]
struct NativeHost {
    last_seen: Instant,
    capabilities: Vec<String>,
    manifest_id: String,
    manifest_version: u64,
    instance_id: String,
    issued_at_ms: u64,
    expires_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum NativeCommandStatus {
    Pending,
    InFlight,
    Done,
}

#[derive(Debug, Clone)]
struct NativeCommand {
    id: String,
    kind: NativeBridgeKind,
    action: String,
    args: Value,
    preferred_host_id: Option<String>,
    created_at: Instant,
    status: NativeCommandStatus,
    host_id: Option<String>,
    host_instance_id: Option<String>,
    host_manifest_id: Option<String>,
    host_manifest_expires_at_ms: Option<u64>,
    claimed_at: Option<Instant>,
    result: Option<Value>,
}

use crate::store::{ConversationTitleSource, WebConversation};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WebMessage {
    message_id: String,
    conversation_id: String,
    role: String,
    content: String,
    include_in_context: bool,
    meta: Value,
    created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WebScheduledAction {
    execution_spec: nola_domain::automation_execution::AutomationDefinitionSpec,
    id: String,
    #[serde(default)]
    revision: i64,
    name: String,
    url: String,
    format: String,
    source_kind: String,
    source_kind_label: String,
    local_only: bool,
    user_prompt: String,
    frequency: String,
    weekday: Option<u8>,
    time: String,
    timezone: String,
    enabled: bool,
    next_scheduled_at: Option<String>,
    updated_at: String,
    latest_result: Option<Value>,
    script_approved: Option<bool>,
    script_runtime: Option<String>,
    script_cwd: Option<String>,
    script_preview: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WebExecutionResult {
    conversation_id: Option<String>,
    run_id: Option<String>,
    result_id: String,
    job_id: Option<String>,
    source_id: String,
    source_name: String,
    source_url: String,
    source_format: String,
    status: String,
    scheduled_at: String,
    fetched_at: String,
    created_at: String,
    llm_summary_text: String,
    preview_text: String,
    failure_reason: Option<String>,
    events: Vec<Value>,
    html_snapshot: Option<Value>,
    ack_count: u32,
    expired: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WebPermissionRule {
    id: u64,
    tool_name: String,
    behavior: String,
    #[serde(rename = "match")]
    match_pattern: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
struct AgentSettings {
    enable_local_dev_tools: bool,
    allowed_dirs: Vec<String>,
    bash_prefixes: Vec<String>,
    network_policy: String,
    permission_profile: String,
    response_style: String,
    custom_system_prompt: String,
    long_term_memory: String,
    user_preferences: String,
    mcp_servers: Vec<Value>,
    #[serde(skip)] // Runtime projection from Core; never a second persisted authority.
    builtin_skill_overrides: Vec<Value>,
    skill_settings_backup_key: Option<String>,
    uploaded_skills: Vec<Value>,
    uploaded_documents: Vec<Value>,
    agent_llm_endpoint: String,
    agent_llm_api_key: String,
    agent_llm_model: String,
    temperature: f64,
    top_p: f64,
    max_output_tokens: u64,
    max_context_tokens: u64,
    agent_mode_enabled: bool,
    host_tools_enabled: bool,
    execution_backend: String,
}

impl Default for AgentSettings {
    fn default() -> Self {
        Self {
            enable_local_dev_tools: false,
            allowed_dirs: Vec::new(),
            bash_prefixes: Vec::new(),
            network_policy: "ask".to_string(),
            permission_profile: "cautious".to_string(),
            response_style: "casual".to_string(),
            custom_system_prompt: String::new(),
            long_term_memory: String::new(),
            user_preferences: String::new(),
            mcp_servers: Vec::new(),
            builtin_skill_overrides: Vec::new(),
            skill_settings_backup_key: None,
            uploaded_skills: Vec::new(),
            uploaded_documents: Vec::new(),
            agent_llm_endpoint: String::new(),
            agent_llm_api_key: String::new(),
            agent_llm_model: String::new(),
            temperature: 0.7,
            top_p: 0.9,
            max_output_tokens: 16_384,
            max_context_tokens: 32768,
            agent_mode_enabled: false,
            // Native and Web clients use the Agent Host as their capability
            // boundary. Advertise Host tools by default, then let Core's
            // concrete-call permission policy allow reversible work and stop
            // destructive operations for confirmation. Users can still turn
            // Host tools off explicitly from Agent settings.
            host_tools_enabled: true,
            execution_backend: "nola".to_string(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct WebState {
    conversations: Vec<WebConversation>,
    messages: Vec<WebMessage>,
    scheduled_actions: Vec<WebScheduledAction>,
    execution_results: Vec<WebExecutionResult>,
    agent_settings: AgentSettings,
    permission_rules: Vec<WebPermissionRule>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConversationInput {
    expected_revision: Option<u64>,
    expected_configuration_revision: Option<i64>,
    title: Option<String>,
    selected_model_source: Option<String>,
    selected_model_id: Option<String>,
    execution_backend: Option<String>,
    external_execution_sessions: Option<HashMap<String, Option<String>>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MessageUpdateInput {
    content: Option<String>,
    include_in_context: Option<bool>,
    meta: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TruncateInput {
    message_id: String,
    keep_message: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BranchInput {
    message_id: Option<String>,
    title: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionMemoryInput {
    expected_configuration_revision: Option<i64>,
    expected_context_source: Option<nola_store_port::context::ContextCompactionSource>,
    session_memory: Option<String>,
    knowledge_points: Option<String>,
    wiki: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[cfg_attr(test, derive(Clone))]
struct GoalControlInput {
    // Browser transport binds here; paired transport binds its outer envelope.
    expected_host_id: Option<String>,
    operation_id: String,
    action: String,
    expected_goal_id: Option<String>,
    expected_control_token: Option<String>,
    #[serde(default)]
    objective: String,
    #[serde(default)]
    constraints: Vec<String>,
    #[serde(default)]
    wake_at_epoch_ms: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ScheduledActionInput {
    execution_spec: nola_domain::automation_execution::AutomationDefinitionSpec,
    name: Option<String>,
    url: Option<String>,
    query: Option<String>,
    path: Option<String>,
    format: Option<String>,
    frequency: Option<String>,
    weekday: Option<u8>,
    time: Option<String>,
    timezone: Option<String>,
    user_prompt: Option<String>,
    script: Option<String>,
    cwd: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ScheduledActionUpdateInput {
    expected_revision: i64,
    enabled: Option<bool>,
    definition: Option<ScheduledActionInput>,
}

#[derive(Debug, Deserialize)]
struct PermissionRuleInput {
    tool_name: Option<String>,
    behavior: Option<String>,
    #[serde(rename = "match")]
    match_pattern: Option<String>,
}

pub fn run_web_server(config: &Config) -> Result<(), String> {
    // Never listen with an ephemeral replacement identity when durable pairing
    // state is unreadable. Existing source receipts belong to that identity.
    initialize_host_identity()?;
    initialize_web_skill_sources_if_never_initialized(
        config.skill_dirs.as_deref(),
        builtin_skills::runtime_roots,
        std::path::Path::exists,
        || {
            let store = NolaStore::open_default()?;
            if store.load_web_agent_settings()?.as_ref().is_some_and(has_unapplied_web_skill_settings) {
                // Serve the explicit review page without materializing or reviving old Skill settings.
                return Ok(());
            }
            let settings = load_agent_settings_from_store(&store)?;
            builtin_skills::synchronize(&settings.builtin_skill_overrides)
            .map(|_| ())
        },
    )?;
    let server_mode = matches!(config.command, crate::config::CommandMode::Server);
    let (access_token, access_token_path) = load_or_create_web_host_access_token()?;
    let persistent_access_token = WEB_HOST_ACCESS_TOKEN.get_or_init(|| access_token).clone();
    mobile_tokens()
        .lock()
        .map_err(|_| "failed to initialize Web Host access token".to_string())?
        .insert(persistent_access_token.clone(), Instant::now());
    let address = format!("{}:{}", config.web_host, config.web_port);
    let socket = address
        .to_socket_addrs()
        .map_err(|err| format!("failed to resolve web bind address {address}: {err}"))?
        .next()
        .ok_or_else(|| format!("failed to resolve web bind address {address}"))?;
    let server = Arc::new(Server::http(socket).map_err(|err| {
        let is_addr_in_use = err
            .downcast_ref::<std::io::Error>()
            .is_some_and(|io_err| io_err.kind() == std::io::ErrorKind::AddrInUse)
            || err.to_string().to_ascii_lowercase().contains("already in use");
        if is_addr_in_use {
            format!(
                "failed to start web server: {address} is already in use ({err}); stop the other process or pass --port <port>"
            )
        } else {
            format!("failed to start web server on {address}: {err}")
        }
    })?);
    let shutdown_requested = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let shutdown_flag = Arc::clone(&shutdown_requested);
    let shutdown_server = Arc::clone(&server);
    let _signals = crate::execution_signals::ExecutionSignals::install_callback(move || {
        if !shutdown_flag.swap(true, std::sync::atomic::Ordering::SeqCst) {
            let controls: Vec<RuntimeExecutionControl> = active_web_agent_runs()
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .values()
                .cloned()
                .collect();
            for control in controls {
                control.request();
            }
            shutdown_server.unblock();
        }
    })?;
    let bound_socket = server.server_addr().to_ip().unwrap_or(socket);
    let mut config = config.clone();
    config.web_port = bound_socket.port();
    let config = &config;
    // Session owners recover through their scoped lease boundary. Server
    // startup must not sweep every conversation in one failure domain.
    let server_origin = format!("http://{bound_socket}");
    let client_api = format!("{server_origin}/api/mobile");
    let server_id = host_identity();
    let _messaging_channel_worker = crate::messaging_channel_worker::start_if_configured()?;
    let _goal_worker = start_web_goal_worker(config)?;
    let _automation_scheduler = start_web_automation_scheduler(config);
    if !bound_socket.ip().is_loopback() {
        eprintln!(
            "WARNING: remote Web access is enabled; use a trusted LAN or an HTTPS reverse proxy and keep the Host token private"
        );
    }
    if config.json_output {
        println!(
            "{}",
            json!({
                "type": "server.started",
                "protocol_version": 1,
                "mode": if server_mode { "server" } else { "web" },
                "origin": server_origin,
                "client_api": client_api,
                "host_id": server_id,
                "access_token": persistent_access_token,
                "credentials_path": access_token_path,
                "loopback": bound_socket.ip().is_loopback(),
                "transport_ingress": if config.tailcat_ingress { "tailcat" } else { "direct" },
                "local_trust": !config.tailcat_ingress && bound_socket.ip().is_loopback(),
            })
        );
    }
    if config.tailcat_ingress {
        eprintln!("Tailcat ingress: loopback requests require authentication. This flag does not start a Tailcat tunnel or create tunnel keys.");
    }
    if server_mode {
        eprintln!("Nola Agent Server listening on {server_origin}");
    } else {
        eprintln!(
            "Nola Web listening on {} (web root: {})",
            server_origin,
            config.web_root.display()
        );
    }
    eprintln!("Nola Server ID: {server_id}");
    eprintln!("Nola Server client API: {client_api}");
    if config.json_output {
        eprintln!("Nola Server access token: emitted only in the JSON startup handshake");
    } else {
        eprintln!("Nola Server access token: {persistent_access_token}");
    }
    eprintln!("Nola Server credentials: {}", access_token_path.display());
    let pairing_enabled = std::env::var("NOLA_MOBILE_PAIRING_SECRET")
        .ok()
        .is_some_and(|value| value.trim().chars().count() >= 16);
    if pairing_enabled {
        eprintln!("Mobile pairing enabled with NOLA_MOBILE_PAIRING_SECRET");
    } else {
        eprintln!(
            "Mobile pairing disabled: set NOLA_MOBILE_PAIRING_SECRET to at least 16 characters"
        );
    }
    if server_mode {
        eprintln!("Connect Nola clients with the server URL and access token shown above; requests use the authenticated, encrypted /api/mobile surface");
    } else {
        eprintln!("Remote browsers sign in at /login with the Server token; Nola clients use the authenticated, encrypted /api/mobile surface");
    }
    for request in server.incoming_requests() {
        if shutdown_requested.load(std::sync::atomic::Ordering::SeqCst) {
            break;
        }
        let Some(request_guard) = try_acquire_web_request() else {
            let _ = request.respond(json_response(
                StatusCode(503),
                json!({
                    "error": "server_busy",
                    "message": "Too many concurrent requests; retry shortly.",
                }),
            ));
            continue;
        };
        let request_slot = Arc::new(Mutex::new(Some(request)));
        let worker_request = Arc::clone(&request_slot);
        let config = config.clone();
        if let Err(err) = std::thread::Builder::new()
            .name("web-http".into())
            .stack_size(16 * 1024 * 1024)
            .spawn(move || {
                let _request_guard = request_guard;
                let Some(request) = worker_request
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .take()
                else {
                    return;
                };
                if let Err(err) = handle_request(request, &config) {
                    eprintln!("web request failed: {err}");
                }
            })
        {
            eprintln!("failed to spawn web request thread: {err}");
            if let Some(request) = request_slot
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .take()
            {
                let _ = request.respond(json_response(
                    StatusCode(503),
                    json!({
                        "error": "server_busy",
                        "message": "Server could not schedule request; retry shortly.",
                    }),
                ));
            }
        }
    }
    drop(_automation_scheduler);
    drop(_goal_worker);
    drop(_messaging_channel_worker);
    shutdown_all_tailcat_leases();
    crate::tools::shutdown_managed_browser_bridge();
    crate::local_models::shutdown();
    Ok(())
}

fn handle_request(mut request: Request, config: &Config) -> Result<(), String> {
    let method = request.method().clone();
    let url = request.url().to_string();
    let path = url.split('?').next().unwrap_or("/");
    let is_remote = !web_loopback_is_trusted(request.remote_addr(), config.tailcat_ingress);
    let is_public_remote_route = remote_web_host_route_allowed(&method, path);
    let has_web_session = !is_remote || web_session_is_authenticated(&request);
    let response = if path.starts_with("/gateway/") {
        match NolaStore::open_default() {
            Ok(store) => crate::model_gateway_host::gateway_response(&mut request, &store, path, &model_gateway_origins(config)),
            Err(_) => json_response(StatusCode(503), json!({"error":"gateway_unavailable"})),
        }
    } else if is_remote && !is_public_remote_route && !has_web_session {
        web_auth_required_response(&method, &url, path)
    } else if is_remote
        && has_web_session
        && web_method_requires_csrf(&method)
        && !web_csrf_exempt_path(path)
        && !web_csrf_is_valid(&request)
    {
        json_response(
            StatusCode(403),
            json!({
                "error": "csrf_validation_failed",
                "message": "The Web session request could not be verified. Reload the page and try again.",
            }),
        )
    } else if let Some((code, message)) = (!is_remote && path.starts_with("/api/"))
        .then(|| validate_local_trust_api_request(&request, &method).err())
        .flatten()
    {
        json_response(
            StatusCode(403),
            json!({
                "error": code,
                "message": message,
            }),
        )
    } else if request
        .body_length()
        .is_some_and(|length| length > MAX_WEB_REQUEST_BODY_BYTES)
    {
        json_response(
            StatusCode(413),
            json!({
                "error": "request_too_large",
                "maxBytes": MAX_WEB_REQUEST_BODY_BYTES,
            }),
        )
    } else {
        let language = request_language(&request);
        route_request(&mut request, &method, &url, path, config, language)
    };
    respond_web_response(request, with_web_security_headers(response))
}

fn route_request(
    request: &mut Request,
    method: &Method,
    url: &str,
    path: &str,
    config: &Config,
    _language: crate::i18n::Language,
) -> Response<Box<dyn Read + Send>> {
    if matches!(path, "/api/harness-plugins" | "/api/desktop-plugin-pages") {
        if method != &Method::Post { return method_not_allowed(); }
        if !(source_management_request_allowed(request, config.tailcat_ingress)
            || (web_session_is_authenticated(request) && web_csrf_is_valid(request))) {
            return json_response(StatusCode(403), json!({"error":"Open plugin management from the selected Host."}));
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct PluginAction { #[serde(default)] runtime: String, #[serde(default)] profile: String, action: String, #[serde(default)] source: String }
        let mut body = Vec::new();
        if request.as_reader().take(4097).read_to_end(&mut body).is_err() || body.len() > 4096 {
            return json_response(StatusCode(400), json!({"error":"Invalid plugin request"}));
        }
        if path == "/api/desktop-plugin-pages" {
            #[derive(Deserialize)]
            #[serde(deny_unknown_fields)]
            struct PageRequest { id: Option<String> }
            let Ok(input) = serde_json::from_slice::<PageRequest>(&body) else {
                return json_response(StatusCode(400), json!({"error":"Invalid desktop page request"}));
            };
            return match crate::harness_plugin::desktop::catalog(input.id.as_deref()) {
                Ok(value) => json_response(StatusCode(200), value),
                Err(error) => json_response(StatusCode(if error == "Desktop plugin page is unavailable" {404} else {500}), json!({"error":error})),
            }.with_header(Header::from_bytes("Cache-Control", "no-store").unwrap());
        }
        let Ok(input) = serde_json::from_slice::<PluginAction>(&body) else {
            return json_response(StatusCode(400), json!({"error":"Invalid plugin request"}));
        };
        let host = request_header(request, "Host").and_then(|value| reqwest::Url::parse(&format!("http://{value}")).ok())
            .and_then(|url| url.host_str().map(str::to_owned));
        let Some(host) = host else {return json_response(StatusCode(400), json!({"error":"Missing Host"}));};
        let local = source_management_request_allowed(request, config.tailcat_ingress);
        if input.action == "library" {
            if !input.runtime.is_empty() || !input.profile.is_empty() || !input.source.is_empty() {
                return json_response(StatusCode(400), json!({"error":"Library lists all runtime profiles"}));
            }
            return json_response(StatusCode(200), crate::harness_plugin::desktop::library(&host, local))
                .with_header(Header::from_bytes("Cache-Control", "no-store").unwrap());
        }
        return match crate::harness_plugin::manage(&input.runtime, &input.profile, &input.action, &input.source, &host, local) {
            Ok(snapshot) => json_response(StatusCode(200), snapshot),
            Err(error) => json_response(StatusCode(409), json!({"error":error})),
        };
    }
    if matches!(path, "/api/software-update" | "/api/software-update/check" | "/api/software-update/package") {
        return software_update_response(request, method, path)
            .with_header(Header::from_bytes("Cache-Control", "no-store").unwrap());
    }
    if path == "/api/agent-rooms" {
        return if method == &Method::Post { agent_room_response(request, config) } else { method_not_allowed() };
    }
    if path == "/api/model-connections" {
        return match method {
            &Method::Get | &Method::Head => model_connection_inventory_response(),
            &Method::Post => model_connection_command_response(request),
            _ => method_not_allowed(),
        };
    }
    if path == "/api/model-connections/discover" {
        return if method == &Method::Post { model_catalog_discover_response(request, config) } else { method_not_allowed() };
    }
    if path == "/api/model-connections/probe" {
        return if method == &Method::Post { model_capability_probe_response(request, config) } else { method_not_allowed() };
    }
    if path == "/api/model-connections/discover/cancel" {
        return if method == &Method::Post { model_catalog_cancel_response(request) } else { method_not_allowed() };
    }
    if matches!(path, "/api/model-connections/kimi-login/owners" | "/api/model-connections/kimi-login/connect" | "/api/model-connections/kimi-login/start" | "/api/model-connections/kimi-login/status" | "/api/model-connections/kimi-login/cancel") {
        return if method == &Method::Post { model_kimi_login_response(request, config.tailcat_ingress, path.rsplit('/').next().unwrap_or("")) } else { method_not_allowed() };
    }
    if matches!(path, "/api/model-connections/opencode-login/owners" | "/api/model-connections/opencode-login/connect" | "/api/model-connections/opencode-login/inspect" | "/api/model-connections/opencode-login/authorize" | "/api/model-connections/opencode-login/callback" | "/api/model-connections/opencode-login/forget") {
        return if method == &Method::Post { model_opencode_login_response(request, config.tailcat_ingress, path.rsplit('/').next().unwrap_or("")) } else { method_not_allowed() };
    }
    if matches!(path, "/api/model-connections/source-login/start" | "/api/model-connections/source-login/status" | "/api/model-connections/source-login/cancel") {
        return if method == &Method::Post { model_source_login_response(request, config.tailcat_ingress, path.rsplit('/').next().unwrap_or("")) } else { method_not_allowed() };
    }
    if matches!(path, "/api/model-connections/source-accounts/bind" | "/api/model-connections/source-accounts/refresh") {
        return if method == &Method::Post { model_source_account_response(request, config.tailcat_ingress, path.ends_with("/refresh")) } else { method_not_allowed() };
    }
    if path == "/api/model-gateway/access" {
        return if matches!(method, &Method::Get | &Method::Post) { model_gateway_access_response(request) } else { method_not_allowed() };
    }
    if path == "/api/model-gateway/clients" {
        return if method == &Method::Get {
            match NolaStore::open_default().and_then(|store| crate::model_client_adapter::list_client_connections(&store)) {
                Ok(values) => json_response(StatusCode(200), json!(values)),
                Err(_) => json_response(StatusCode(503), json!({"error":"client_connections_unavailable"})),
            }
        } else { method_not_allowed() };
    }
    if path == "/api/model-gateway/attempts" {
        return if method == &Method::Get {
            let page = (|| -> Result<(usize,usize),String> {
                let limit = query_param(url,"limit").map(|value|value.parse::<usize>()).transpose().map_err(|_|"invalid limit")?.unwrap_or(100);
                let offset = query_param(url,"offset").map(|value|value.parse::<usize>()).transpose().map_err(|_|"invalid offset")?.unwrap_or(0);
                if !(1..=200).contains(&limit) { return Err("invalid limit".into()); }
                Ok((limit,offset))
            })();
            let Ok((limit,offset)) = page else { return json_response(StatusCode(422),json!({"error":"invalid_gateway_page"})); };
            match NolaStore::open_default().and_then(|store| crate::model_gateway_usage::list_gateway_attempts(&store, limit, offset)) {
                Ok(values) => json_response(StatusCode(200), json!(values)),
                Err(_) => json_response(StatusCode(503), json!({"error":"gateway_attempts_unavailable"})),
            }
        } else { method_not_allowed() };
    }
    if let Some(action) = path.strip_prefix("/api/model-gateway/clients/").filter(|action| matches!(*action, "plan" | "commit" | "disconnect")) {
        return if method == &Method::Post { model_client_connection_response(request, config, action) } else { method_not_allowed() };
    }
    if matches!(path, "/api/model-import/plan" | "/api/model-import/commit") {
        return if method == &Method::Post { model_import_response(request, config, path.ends_with("/commit")) } else { method_not_allowed() };
    }
    if path == "/api/local-models/engine-status" {
        if !matches!(method, &Method::Get | &Method::Head) { return method_not_allowed(); }
        let engine = query_param(url, "engine").unwrap_or_default();
        if !matches!(engine.as_str(), "llama.cpp" | "vllm") {
            return json_response(StatusCode(400), json!({"error":"Unknown desktop engine."}));
        }
        let local = source_management_peer_allowed(request.remote_addr(), config.tailcat_ingress)
            && request_header(request, "Host").and_then(|host| reqwest::Url::parse(&format!("http://{host}")).ok())
                .is_some_and(|url| matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]")));
        return match crate::model_acquisition::engine_status(&engine, local) {
            Ok(value) => json_response(StatusCode(200), value),
            Err(message) => json_response(StatusCode(400), json!({"error":message})),
        };
    }
    if path == "/api/local-models/setup-engine" {
        if method != &Method::Post { return method_not_allowed(); }
        if !source_management_request_allowed(request, config.tailcat_ingress) {
            return json_response(StatusCode(403), json!({"error":"Open model settings on the Host to install an engine.","code":"install_on_host"}));
        }
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Setup { engine: String }
        let input = match read_json::<Setup>(request) { Ok(input) => input, Err(response) => return response };
        if !matches!(input.engine.as_str(), "llama.cpp" | "vllm") {
            return json_response(StatusCode(400), json!({"error":"Unknown desktop engine."}));
        }
        return match crate::model_acquisition::setup_engine(&input.engine) {
            Ok(value) => json_response(StatusCode(200), value),
            Err(message) => json_response(StatusCode(409), json!({"error":message})),
        };
    }
    if path == "/api/local-models/hardware" {
        return if matches!(method, &Method::Get | &Method::Head) { json_response(StatusCode(200), crate::model_acquisition::hardware()) } else { method_not_allowed() };
    }
    if let Some(action) = path.strip_prefix("/api/local-models/").filter(|action| matches!(*action, "search" | "recommend" | "install" | "install-plan" | "cancel")) {
        return if method == &Method::Post { acquisition::handle(request, action, config.tailcat_ingress) } else { method_not_allowed() };
    }
    if path == "/api/local-models" {
        return match method {
            &Method::Get | &Method::Head => json_response(
                StatusCode(200),
                with_local_first(acquisition::catalog()),
            ),
            _ => method_not_allowed(),
        };
    }
    if let Some((model_id, action)) = local_model_action_route(path) {
        return local_model_action_response(request, method, model_id, action);
    }
    match (method, path) {
        (&Method::Get, "/api/health") | (&Method::Head, "/api/health") => {
            return json_response(
                StatusCode(200),
                json!({
                    "ok": true,
                    "service": "nola-cli-web",
                    "mode": "rust-cli",
                    "pythonBackend": false,
                    "host": {
                        "id": host_identity(),
                        "mobileApiBase": "/api/mobile",
                        "auth": "hmac-sha256-v1",
                        "payloadEncryption": "aes-gcm-v1",
                        "permissionProtocolVersion": HOST_PERMISSION_PROTOCOL_VERSION,
                        "persistentAccessToken": WEB_HOST_ACCESS_TOKEN.get().is_some(),
                        "webLoginPath": "/login",
                        "webSessionAuth": "host-token-cookie-v1",
                    },
                }),
            );
        }
        (&Method::Get, "/api/models") | (&Method::Head, "/api/models") => {
            return models_response(config);
        }
        (&Method::Get, "/api/model-profiles") | (&Method::Head, "/api/model-profiles") => {
            return model_profiles_response();
        }
        (&Method::Get, "/api/expert-prompt-templates") | (&Method::Head, "/api/expert-prompt-templates") => {
            return expert_prompt_templates_response();
        }
        (&Method::Post, "/api/model-profiles") => {
            return create_remote_model_profile_response(request);
        }
        (&Method::Post, "/api/model-profiles/catalog") => {
            return mutate_remote_model_catalog_response(request);
        }
        (&Method::Post, "/api/model-profiles/catalog/replace") => {
            return replace_remote_model_catalog_response(request);
        }
        (&Method::Get, "/api/local-store") | (&Method::Head, "/api/local-store") => {
            return local_store_response();
        }
        (&Method::Get, "/api/me") | (&Method::Head, "/api/me") => {
            return json_response(
                StatusCode(200),
                json!({
                    "authenticated": true,
                    "local": true,
                    "backend": "cli-rust",
                    "client_uuid": "cli-local-user",
                    "email": "local@nola.cli",
                    "display_name": "Nola CLI",
                }),
            );
        }
        (&Method::Get, "/api/tasks") | (&Method::Head, "/api/tasks") => {
            return tasks_response();
        }
        (&Method::Post, "/api/tasks") => {
            return create_task_response(request);
        }
        (&Method::Get, "/api/agent/skill-settings-review") => return skill_settings_review_response(None),
        (&Method::Post, "/api/agent/skill-settings-review") => {
            let value = match read_json_value(request) { Ok(value) => value, Err(response) => return response };
            if value["use_core_settings"] != true || value["expected_revision"].as_i64().is_none() {
                return json_response(StatusCode(400), json!({"error":"Review confirmation and expected revision are required"}));
            }
            return skill_settings_review_response(value["expected_revision"].as_i64());
        }
        (&Method::Get, "/api/agent/settings") | (&Method::Head, "/api/agent/settings") => {
            return agent_settings_response();
        }
        (&Method::Get, "/api/assistant/memory-default") | (&Method::Head, "/api/assistant/memory-default") => {
            return assistant_memory_default_response(None);
        }
        (&Method::Put, "/api/assistant/memory-default") => {
            let value = match read_json_value(request) { Ok(value) => value, Err(response) => return response };
            let input = match serde_json::from_value::<crate::assistant_memory_preferences::MemoryDefaultInput>(value) {
                Ok(input) => input,
                Err(_) => return json_response(StatusCode(422), json!({"code":"invalid_memory_default"})),
            };
            return assistant_memory_default_response(Some(input.enabled));
        }
        (&Method::Post, "/api/agent/settings") => {
            return update_agent_settings_response(request);
        }
        (&Method::Get, "/api/assistant/retrieval-default") | (&Method::Head, "/api/assistant/retrieval-default") => {
            return assistant_retrieval_default_response(None);
        }
        (&Method::Put, "/api/assistant/retrieval-default") => {
            let value = match read_json_value(request) { Ok(value) => value, Err(response) => return response };
            let input = match serde_json::from_value::<crate::assistant_retrieval_preferences::RetrievalDefaultInput>(value) {
                Ok(input) => input,
                Err(_) => return json_response(StatusCode(422), json!({"code":"invalid_retrieval_default"})),
            };
            return assistant_retrieval_default_response(Some(input.enabled));
        }
        (&Method::Get, "/api/messaging-channel-worker")
        | (&Method::Head, "/api/messaging-channel-worker") => {
            return match crate::messaging_channel_worker::config_projection() {
                Ok(value) => json_response(StatusCode(200), value),
                Err(error) => json_response(StatusCode(500), json!({"error": error})),
            };
        }
        (&Method::Put, "/api/messaging-channel-worker") => {
            let body = match read_json_value(request) {
                Ok(value) => value,
                Err(response) => return response,
            };
            return match crate::messaging_channel_worker::update_config(&body) {
                Ok(value) => json_response(StatusCode(200), value),
                Err(error) => json_response(StatusCode(400), json!({"error": error})),
            };
        }
        (&Method::Get, "/api/messaging-channels") | (&Method::Head, "/api/messaging-channels") => {
            return match crate::messaging_channel_worker::list_connections() {
                Ok(value) => json_response(StatusCode(200), value),
                Err(error) => json_response(StatusCode(400), json!({"error": error})),
            };
        }
        (&Method::Put, "/api/messaging-channels") => {
            let body = match read_json_value(request) {
                Ok(value) => value,
                Err(response) => return response,
            };
            return match crate::messaging_channel_worker::save_connection(&body) {
                Ok(value) => json_response(StatusCode(200), value),
                Err(error) => json_response(StatusCode(400), json!({"error": error})),
            };
        }
        (&Method::Get, "/api/agent/permission_rules")
        | (&Method::Head, "/api/agent/permission_rules") => {
            return list_permission_rules_response();
        }
        (&Method::Post, "/api/agent/permission_rules") => {
            return create_permission_rule_response(request);
        }
        (&Method::Post, "/api/agent/host-permission/decision") => {
            return web_host_permission_decision_response(request);
        }
        (&Method::Get, "/api/agent/tools") | (&Method::Head, "/api/agent/tools") => {
            return agent_tools_response(config);
        }
        (&Method::Get, "/api/native-browser/status")
        | (&Method::Head, "/api/native-browser/status") => {
            return native_status_response(NativeBridgeKind::Browser);
        }
        (&Method::Post, "/api/native-browser/command") => {
            return native_browser_command_response(request);
        }
        (&Method::Post, "/api/native-browser/host/poll") => {
            return native_host_poll_response(request, NativeBridgeKind::Browser);
        }
        (&Method::Post, "/api/native-browser/host/result") => {
            return native_host_result_response(request, NativeBridgeKind::Browser);
        }
        (&Method::Post, "/api/mobile/login") => {
            return mobile_login_response(request, config.tailcat_ingress);
        }
        (&Method::Post, "/api/mobile/web-handoff") => {
            return mobile_web_handoff_response(request);
        }
        (&Method::Get, "/api/web/session") | (&Method::Head, "/api/web/session") => {
            return web_session_response(request, config);
        }
        (&Method::Post, "/api/web/login") => {
            return web_json_login_response(request, config.tailcat_ingress);
        }
        (&Method::Post, "/api/web/logout") => {
            return web_logout_response(request, false);
        }
        (&Method::Post, "/api/mobile/transport/tailcat") => {
            return mobile_tailcat_transport_response(request, config);
        }
        (&Method::Get, "/api/mobile/me") | (&Method::Head, "/api/mobile/me") => {
            return mobile_me_response(request);
        }
        (&Method::Get, "/api/mobile/agent/settings")
        | (&Method::Head, "/api/mobile/agent/settings") => {
            return mobile_agent_settings_response(request);
        }
        (&Method::Post, "/api/mobile/agent/tool-plan/prepare") => return host_tool_plan_response(request, config, true, false),
        (&Method::Post, "/api/mobile/agent/tool-plan/read") => return host_tool_plan_response(request, config, true, true),
        (&Method::Post, "/api/agent/tool-plan/prepare") => return host_tool_plan_response(request, config, false, false),
        (&Method::Post, "/api/agent/tool-plan/read") => return host_tool_plan_response(request, config, false, true),
        (&Method::Post, "/api/mobile/attachments/parse") => {
            return mobile_attachment_parse_response(request);
        }
        (&Method::Post, "/api/mobile/auxiliary/prepare") => {
            return auxiliary_mobile_response(request, config, "prepare");
        }
        (&Method::Post, "/api/mobile/agent-rooms") => return mobile_agent_room_response(request, config),
        (&Method::Post, "/api/mobile/chat/expert-messages/admit") => return mobile_expert_admit_response(request, config),
        (&Method::Post, "/api/mobile/chat/expert-messages/parent") => return mobile_expert_parent_response(request),
        (&Method::Post, "/api/mobile/chat/expert-messages/manifest") => return mobile_expert_manifest_response(request),
        (&Method::Post, "/api/mobile/auxiliary/invoke") => {
            return auxiliary_mobile_response(request, config, "invoke");
        }
        (&Method::Post, "/api/mobile/auxiliary/admit") => {
            return auxiliary_mobile_response(request, config, "admit");
        }
        (&Method::Post, "/api/mobile/auxiliary/inspect") => {
            return auxiliary_mobile_response(request, config, "inspect");
        }
        (&Method::Post, "/api/mobile/auxiliary/cancel") => {
            return auxiliary_mobile_response(request, config, "cancel");
        }
        (&Method::Post, "/api/mobile/chat/agent/v2/resume") => {
            return host_run_resume_response(request, true);
        }
        (&Method::Post, "/api/mobile/chat/agent/v2/observe") => {
            return run_observation_response(request, true);
        }
        (&Method::Post, "/api/mobile/chat/agent/v2/configuration") => {
            return paired_permission_configuration_response(request);
        }
        (&Method::Post, "/api/mobile/chat/agent/v2/submission-frontier") => {
            return submission_frontier_response(request);
        }
        (&Method::Post, "/api/mobile/chat/agent/v2/context/compact") => {
            return paired_context_compaction_response(request, config);
        }
        (&Method::Post, "/api/mobile/chat/agent/v2/submission") => {
            return submission_receipt_response(request, true);
        }
        (&Method::Post, "/api/chat/agent/v2/submission") => {
            return submission_receipt_response(request, false);
        }
        (&Method::Post, "/api/chat/agent/v2/observe") => {
            return run_observation_response(request, false);
        }
        (&Method::Post, "/api/chat/agent/v2/task/recovery") => {
            return web_owned_task_recovery_response(request);
        }
        (&Method::Post, "/api/chat/agent/v2/recovery") => {
            return web_unknown_effect_recovery_response(request);
        }
        (&Method::Post, "/api/chat/agent/v2/resume") => {
            return host_run_resume_response(request, false);
        }
        (&Method::Post, "/api/mobile/chat/agent/v2/stream") => {
            return mobile_chat_agent_stream_response(request, config);
        }
        (&Method::Post, "/api/mobile/chat/agent/v2/traces") => {
            return mobile_chat_agent_trace_replay_response(request);
        }
        (&Method::Post, "/api/mobile/chat/agent/v2/ui/changes") => {
            return mobile_ui_changes_response(request);
        }
        (&Method::Post, "/api/mobile/chat/agent/v2/ui/preview") => {
            return mobile_ui_preview_response(request);
        }
        (&Method::Post, "/api/mobile/chat/agent/v2/ui/revision") => {
            return mobile_ui_revision_response(request);
        }
        (&Method::Post, "/api/mobile/chat/agent/v2/ui/queued-input/cancel") => {
            return mobile_cancel_queued_input_response(request);
        }
        (&Method::Post, "/api/mobile/chat/agent/v2/ui/queued-input") => {
            return mobile_ui_queued_input_response(request);
        }
        (&Method::Post, "/api/mobile/chat/agent/v2/ui/interactions") => {
            return mobile_ui_interaction_response(request);
        }
        (&Method::Post, "/api/mobile/chat/agent/v2/ui/surfaces") => {
            return mobile_ui_surfaces_response(request);
        }
        (&Method::Post, "/api/mobile/chat/agent/v2/journal") => {
            return mobile_assistant_journal_response(request);
        }
        (&Method::Post, "/api/mobile/chat/agent/v2/execution-selection") => {
            return paired_execution_selection_response(request);
        }
        (&Method::Post, "/api/mobile/chat/agent/v2/progress") => {
            return paired_conversation_progress_response(request);
        }
        (&Method::Post, "/api/mobile/chat/agent/v2/goal") => {
            return paired_conversation_goal_response(request, config);
        }
        (&Method::Post, "/api/chat/agent/v2/journal") => {
            return browser_assistant_journal_response(request);
        }
        (&Method::Post, "/api/mobile/chat/agent/v2/cancel") => {
            return mobile_chat_agent_cancel_response(request);
        }
        (&Method::Post, "/api/mobile/chat/agent/v2/task/cancel/receipt") => {
            return mobile_chat_agent_task_cancel_receipt_response(request);
        }
        (&Method::Post, "/api/mobile/chat/agent/v2/task/cancel") => {
            return mobile_chat_agent_task_cancel_response(request);
        }
        (&Method::Post, "/api/mobile/chat/agent/v2/task/message") => {
            return mobile_chat_agent_task_message_response(request);
        }
        (&Method::Post, "/api/mobile/chat/agent/v2/task/events") => {
            return mobile_chat_agent_task_events_response(request);
        }
        (&Method::Post, "/api/mobile/chat/agent/v2/permission") => {
            return mobile_host_permission_decision_response(request);
        }
        (&Method::Post, "/api/mobile/chat/agent/v2/device-tool") => {
            return mobile_device_tool_result_response(request);
        }
        (&Method::Post,"/api/chat/expert-messages/admit") => { return expert_admit_response(request); }
        (&Method::Post,"/api/chat/edited-expert-resend/prepare") => { return expert_resend_prepare_response(request); }
        (&Method::Post,"/api/chat/in-place-resend/prepare") => { return in_place_resend_prepare_response(request); }
        (&Method::Post,"/api/chat/edited-resend/prepare") => {
            return edited_resend_prepare_response(request);
        }
        (&Method::Post, "/api/chat/agent/v2/stream") => {
            return chat_agent_stream_response(request, config);
        }
        (&Method::Post, "/api/chat/agent/v2/cancel") => {
            return chat_agent_cancel_response(request);
        }
        (&Method::Post, "/api/chat/agent/v2/task/cancel/receipt") => {
            return chat_agent_task_cancel_receipt_response(request);
        }
        (&Method::Post, "/api/chat/agent/v2/task/cancel") => {
            return chat_agent_task_cancel_response(request);
        }
        (&Method::Post, "/api/chat/agent/v2/task/message/receipt") => {
            return chat_agent_task_message_receipt_response(request);
        }
        (&Method::Post, "/api/mobile/chat/agent/v2/task/message/receipt") => {
            return mobile_chat_agent_task_message_receipt_response(request);
        }
        (&Method::Post, "/api/chat/agent/v2/task/message") => {
            return chat_agent_task_message_response(request);
        }
        (&Method::Get, "/api/scheduled-actions") | (&Method::Head, "/api/scheduled-actions") => {
            return list_scheduled_actions_response();
        }
        (&Method::Post, "/api/scheduled-actions") => {
            return create_scheduled_action_response(request);
        }
        (&Method::Get, "/api/automation-skills") | (&Method::Head, "/api/automation-skills") => {
            return automation_skills_response(config);
        }
        (&Method::Post, "/api/activity") => {
            let input = match read_json::<nola_application::ActivityQuery>(request) {
                Ok(input) => input, Err(response) => return response,
            };
            return activity_read_response(nola_application::ApplicationCommand::ListActivity { input });
        }
        (&Method::Post, "/api/activity/detail") => {
            #[derive(Deserialize)]
            struct Owner { conversation_id: String, run_id: String, assistant_message_id: String }
            let input = match read_json::<Owner>(request) {
                Ok(input) => input, Err(response) => return response,
            };
            return activity_read_response(nola_application::ApplicationCommand::LoadActivityDetail {
                conversation_id: input.conversation_id, run_id: input.run_id, assistant_message_id: input.assistant_message_id,
            });
        }
        (&Method::Get, "/api/results") | (&Method::Head, "/api/results") => {
            return list_results_response();
        }
        (&Method::Get, "/api/documents/search") | (&Method::Head, "/api/documents/search") => {
            return document_search_response(url);
        }
        (&Method::Get, "/api/workspace/today") | (&Method::Head, "/api/workspace/today") => {
            return workspace_today_response(url);
        }
        (&Method::Get, "/api/workspace/diff") | (&Method::Head, "/api/workspace/diff") => {
            return workspace_diff_response(url, config);
        }
        (&Method::Get, "/api/workspace/checkpoints")
        | (&Method::Head, "/api/workspace/checkpoints") => {
            return workspace_checkpoints_response(url, config);
        }
        (&Method::Post, "/api/workspace/undo") => {
            return workspace_undo_redo_response(request, url, config, false);
        }
        (&Method::Post, "/api/workspace/redo") => {
            return workspace_undo_redo_response(request, url, config, true);
        }
        (&Method::Get, "/api/workspace/worktrees") | (&Method::Head, "/api/workspace/worktrees") => {
            return workspace_worktrees_list_response(url, config);
        }
        (&Method::Post, "/api/workspace/worktrees") => {
            return workspace_worktrees_create_response(request, url, config);
        }
        (&Method::Get, "/api/home") | (&Method::Head, "/api/home") => {
            return home_response();
        }
        (&Method::Post, "/api/memory") => {
            let input = match read_json::<crate::memory_admin::MemoryWriteRequest>(request) {
                Ok(input) => input, Err(response) => return response,
            };
            let result = crate::application_port::CliApplicationPort::persistent_default()
                .and_then(|mut application| crate::memory_admin::write_memory(&mut application, input));
            return match result {
                Ok(receipt) => json_response(StatusCode(200), receipt),
                Err(_) => json_response(StatusCode(409), json!({"error": "Memory write could not be confirmed"})),
            };
        }
        (&Method::Get, "/api/memory") | (&Method::Head, "/api/memory") => {
            let records = crate::application_port::CliApplicationPort::persistent_default()
                .and_then(|mut application| crate::memory_admin::list_memories(&mut application, 2_000));
            return match records {
                Ok(records) => json_response(StatusCode(200), json!({"type": "memory_records", "limit": 2_000, "records": records})),
                Err(_) => json_response(StatusCode(500), json!({"error": "Memory records unavailable"})),
            };
        }
        (&Method::Get, "/api/data-sources") | (&Method::Head, "/api/data-sources") => {
            return data_sources_response(url);
        }
        (&Method::Post, "/api/data-sources") => {
            return create_data_source_response(request);
        }
        (&Method::Get, "/api/data-sources/usage") | (&Method::Head, "/api/data-sources/usage") => {
            return data_source_usage_response();
        }
        (&Method::Get, "/favicon.ico") | (&Method::Head, "/favicon.ico") => {
            return file_response(&config.web_root, "app/static/favicon.png");
        }
        (&Method::Get, "/apple-touch-icon.png")
        | (&Method::Head, "/apple-touch-icon.png")
        | (&Method::Get, "/apple-touch-icon-precomposed.png")
        | (&Method::Head, "/apple-touch-icon-precomposed.png") => {
            return file_response(&config.web_root, "app/static/apple-touch-icon.png");
        }
        (&Method::Get, _) | (&Method::Head, _) if path.starts_with("/static/") => {
            let rel = path.trim_start_matches("/static/");
            return file_response(&config.web_root, &format!("app/static/{rel}"));
        }
        _ => {}
    }

    if path == "/api/chat/conversations" {
        return match method {
            &Method::Get | &Method::Head => list_conversations_response(),
            &Method::Post => create_conversation_response(request),
            _ => method_not_allowed(),
        };
    }

    if let Some((source_id, suffix)) = data_source_route(path) {
        return match (method, suffix) {
            (&Method::Patch, "") => update_data_source_response(request, &source_id),
            (&Method::Delete, "") => delete_data_source_response(&source_id),
            (&Method::Post, "test") => test_data_source_response(&source_id),
            _ => method_not_allowed(),
        };
    }

    if let Some((conversation_id, suffix)) = chat_conversation_route(path) {
        let conversation_id = conversation_id.as_str();
        return match (method, suffix) {
            (&Method::Get, "") | (&Method::Head, "") => read_conversation_response(conversation_id),
            (&Method::Get, "queued-input") | (&Method::Head, "queued-input") => conversation_queued_input_response(conversation_id, url),
            (&Method::Post, "queued-input/cancel") => conversation_cancel_queued_input_response(request, conversation_id),
            (&Method::Post, "ui-surface-changes") => conversation_ui_changes_response(request, conversation_id),
            (&Method::Post, "ui-surface-preview") => conversation_ui_preview_response(request, conversation_id),
            (&Method::Post, "ui-surface-revision") => conversation_ui_revision_response(request, conversation_id),
            (&Method::Post, "ui-interactions") => conversation_ui_interaction_response(request, conversation_id),
            (&Method::Get, "ui-surfaces") | (&Method::Head, "ui-surfaces") => conversation_ui_surfaces_response(conversation_id),
            (&Method::Get,"edited-expert-resend")|(&Method::Head,"edited-expert-resend")=>expert_resend_load_response(url,conversation_id),
            (&Method::Get, suffix)|(&Method::Head,suffix) if suffix.starts_with("expert-messages/") => expert_source_response(url,conversation_id,&suffix["expert-messages/".len()..]),
            (&Method::Get,"edited-resend")|(&Method::Head,"edited-resend")=>edited_resend_load_response(url,conversation_id),
            (&Method::Patch, "") => update_conversation_response(request, conversation_id),
            (&Method::Get, "workspace-review") | (&Method::Head, "workspace-review") => {
                conversation_workspace_review_response(conversation_id, config)
            }
            (&Method::Get, "workspace") | (&Method::Head, "workspace") => {
                conversation_workspace_get_response(conversation_id, config)
            }
            (&Method::Patch, "workspace") => {
                conversation_workspace_patch_response(request, conversation_id, config)
            }
            (&Method::Get, "context-usage") | (&Method::Head, "context-usage") => {
                conversation_context_usage_response(conversation_id)
            }
            (&Method::Get, "configuration") | (&Method::Head, "configuration") => {
                conversation_configuration_response(conversation_id, query_param(url, "minimum_policy_revision").and_then(|value|value.parse::<i64>().ok()).filter(|value|*value>0))
            }
            (&Method::Patch, "configuration") => {
                patch_conversation_configuration_response(request, conversation_id)
            }
            (&Method::Delete, "") => delete_conversation_response(config, conversation_id),
            (&Method::Get, "messages") | (&Method::Head, "messages") => {
                list_messages_response(url, conversation_id)
            }
            (&Method::Post, "admit-user-message") => admit_user_message_response(request, conversation_id),
            (&Method::Get, "browser") | (&Method::Head, "browser") => {
                conversation_browser_response(conversation_id, None)
            }
            (&Method::Post, "browser/switch-page") => {
                let input = match read_json::<ConversationBrowserSwitchRequest>(request) {
                    Ok(value) => value,
                    Err(response) => return response,
                };
                conversation_browser_response(conversation_id, Some(input.index))
            }
            (&Method::Post, "resources") => {
                put_conversation_pdf_resource_response(request, conversation_id)
            }
            (&Method::Get, "resources") | (&Method::Head, "resources") => {
                list_conversation_resources_response(conversation_id)
            }
            (&Method::Get, suffix) | (&Method::Head, suffix)
                if suffix.starts_with("resources/") =>
            {
                let Some(resource_id) = decode_path_segment(suffix.trim_start_matches("resources/")) else {
                    return json_response(StatusCode(400), json!({"error": "invalid resource id"}));
                };
                read_conversation_pdf_resource_response(
                    conversation_id,
                    &resource_id,
                    method == &Method::Head,
                )
            }
            (&Method::Delete, suffix) if suffix.starts_with("resources/") => {
                let Some(resource_id) = decode_path_segment(suffix.trim_start_matches("resources/")) else {
                    return json_response(StatusCode(400), json!({"error": "invalid resource id"}));
                };
                delete_conversation_resource_response(
                    conversation_id,
                    &resource_id,
                )
            }
            (&Method::Patch, suffix) if suffix.starts_with("messages/") => {
                let Some(message_id) = decode_path_segment(suffix.trim_start_matches("messages/")) else {
                    return json_response(StatusCode(400), json!({"error": "invalid message id"}));
                };
                update_message_response(
                    request,
                    conversation_id,
                    &message_id,
                )
            }
            (&Method::Delete, suffix) if suffix.starts_with("messages/") => {
                let Some(message_id) = decode_path_segment(suffix.trim_start_matches("messages/")) else {
                    return json_response(StatusCode(400), json!({"error": "invalid message id"}));
                };
                delete_message_response(conversation_id, &message_id)
            }
            (&Method::Post, "truncate") => {
                truncate_conversation_response(config, request, conversation_id)
            }
            (&Method::Post, "branch") => branch_conversation_response(request, conversation_id),
            (&Method::Get, "historical-part") | (&Method::Head, "historical-part") => {
                historical_part_response(url, conversation_id)
            }
            (&Method::Get, "export") | (&Method::Head, "export") => {
                export_conversation_response(url, conversation_id)
            }
            (&Method::Get, "context") | (&Method::Head, "context") => {
                context_report_response(conversation_id, config)
            }
            (&Method::Get, "recall") | (&Method::Head, "recall") => {
                conversation_recall_response(url, conversation_id, crate::session::search_session_text)
            }
            (&Method::Get, "goal") | (&Method::Head, "goal") => {
                conversation_goal_response(conversation_id)
            }
            (&Method::Get, "user-input-waits") | (&Method::Head, "user-input-waits") => {
                conversation_user_input_waits_response(conversation_id)
            }
            (&Method::Post, "user-input-waits/respond") => {
                conversation_user_input_response(request, conversation_id)
            }
            (&Method::Get, "plan") | (&Method::Head, "plan") => {
                conversation_plan_response(conversation_id)
            }
            (&Method::Post, "goal") => {
                control_conversation_goal_response(request, conversation_id, config)
            }
            (&Method::Get, "traces") | (&Method::Head, "traces") => {
                conversation_traces_response(url, conversation_id)
            }
            (&Method::Patch, "session-memory") => {
                update_session_memory_response(request, conversation_id)
            }
            (&Method::Get, "session-memory") | (&Method::Head, "session-memory") => {
                match NolaStore::open_default().and_then(|store|store.web_session_memory_edit_state(conversation_id)) {
                    Ok(state)=>json_response(StatusCode(200),state),
                    Err(error)=>json_response(StatusCode(409),json!({"code":"context_read_conflict","error":error})),
                }
            }
            (&Method::Post, "compact") => compact_response(request, conversation_id, config),
            (&Method::Get, "memory/search") | (&Method::Head, "memory/search") => {
                memory_search_response(url, conversation_id)
            }
            (&Method::Patch, suffix) if suffix.starts_with("data-sources/") => {
                let Some(source_id) = decode_path_segment(suffix.trim_start_matches("data-sources/")) else {
                    return json_response(StatusCode(400), json!({"error": "invalid data source id"}));
                };
                update_conversation_data_source_response(
                    request,
                    conversation_id,
                    &source_id,
                )
            }
            _ => method_not_allowed(),
        };
    }

    if let Some(id) = path.strip_prefix("/api/memory/") {
        if method != &Method::Delete { return method_not_allowed(); }
        let Some(id) = decode_path_segment(id) else {
            return json_response(StatusCode(400), json!({"error": "Invalid memory id"}));
        };
        let result = crate::application_port::CliApplicationPort::persistent_default()
            .and_then(|mut application| crate::memory_admin::delete_memory(&mut application, &id));
        return match result {
            Ok(receipt) => json_response(StatusCode(200), receipt),
            Err(_) => json_response(StatusCode(409), json!({"error": "Memory deletion could not be confirmed"})),
        };
    }

    if let Some((action_id, suffix)) = scheduled_action_route(path) {
        let action_id = action_id.as_str();
        return match (method, suffix) {
            (&Method::Delete, "") => delete_scheduled_action_response(action_id),
            (&Method::Patch, "") => update_scheduled_action_response(request, action_id),
            (&Method::Post, "approve-script") => approve_script_action_response(request, action_id),
            (&Method::Post, "run-now") => run_scheduled_action_now_response(request, action_id, config),
            _ => method_not_allowed(),
        };
    }

    if let Some(result_id) = result_detail_route(path) {
        return match method {
            &Method::Get | &Method::Head => result_detail_response(&result_id),
            _ => method_not_allowed(),
        };
    }

    if let Some(rule_id) = permission_rule_route(path) {
        return match method {
            &Method::Delete => delete_permission_rule_response(rule_id),
            _ => method_not_allowed(),
        };
    }

    if let Some(id) = path
        .strip_prefix("/api/model-endpoints/")
        .and_then(|value| value.strip_suffix("/credential"))
    {
        return match method {
            &Method::Get | &Method::Post => {
                model_endpoint_credential_response(request, &percent_decode_path(id))
            }
            _ => method_not_allowed(),
        };
    }
    if let Some(id) = path
        .strip_prefix("/api/model-endpoints/")
        .and_then(|value| value.strip_suffix("/discover"))
    {
        return match method {
            &Method::Post => discover_remote_models_response(&percent_decode_path(id)),
            _ => method_not_allowed(),
        };
    }
    if let Some(id) = path
        .strip_prefix("/api/model-endpoints/")
        .and_then(|value| value.strip_suffix("/configuration"))
    {
        return match method {
            &Method::Post => update_model_endpoint_response(request, &percent_decode_path(id)),
            _ => method_not_allowed(),
        };
    }
    if let Some(id) = path
        .strip_prefix("/api/model-profiles/")
        .and_then(|value| value.strip_suffix("/select"))
    {
        return match method {
            &Method::Post => select_model_profile_response(request, &percent_decode_path(id)),
            _ => method_not_allowed(),
        };
    }
    if let Some(id) = path
        .strip_prefix("/api/model-profiles/")
        .and_then(|value| value.strip_suffix("/parameters"))
    {
        return match method {
            &Method::Post => update_model_parameters_response(request, &percent_decode_path(id)),
            _ => method_not_allowed(),
        };
    }
    match (method, path) {
        (&Method::Get, "/") | (&Method::Head, "/") => redirect_response("/chat"),
        (&Method::Get, "/models")
        | (&Method::Head, "/models")
        | (&Method::Get, "/model-test")
        | (&Method::Head, "/model-test")
        | (&Method::Get, "/plugin-page")
        | (&Method::Head, "/plugin-page")
        | (&Method::Get, "/capabilities")
        | (&Method::Head, "/capabilities")
        | (&Method::Get, "/activity")
        | (&Method::Head, "/activity")
        | (&Method::Get, "/results")
        | (&Method::Head, "/results")
        | (&Method::Get, "/dashboard")
        | (&Method::Head, "/dashboard")
        | (&Method::Get, "/data-sources")
        | (&Method::Head, "/data-sources")
        | (&Method::Get, "/settings")
        | (&Method::Head, "/settings")
        | (&Method::Get, "/scheduled-actions")
        | (&Method::Head, "/scheduled-actions")
        | (&Method::Get, "/workspace-review")
        | (&Method::Head, "/workspace-review")
        | (&Method::Get, "/today")
        | (&Method::Head, "/today")
        | (&Method::Get, "/conversations")
        | (&Method::Head, "/conversations")
        | (&Method::Get, "/chat")
        | (&Method::Head, "/chat")
        | (&Method::Get, "/login")
        | (&Method::Head, "/login") => {
            if !config.web_root.join("app/static/ui/index.html").is_file() {
                return html_response(
                    StatusCode(503),
                    error_html(
                        503,
                        "Web UI is not built. Run npm ci && npm run build in apps/web/frontend.",
                    ),
                );
            }
            file_response(&config.web_root, "app/static/ui/index.html")
        }
        (&Method::Get, _) | (&Method::Head, _) if path.starts_with("/ui/") => {
            let rel = path.trim_start_matches("/ui/");
            file_response(&config.web_root, &format!("app/static/ui/{rel}"))
        }
        (&Method::Get, "/rooms") | (&Method::Head, "/rooms") => redirect_response("/chat"),
        (&Method::Get, "/register") | (&Method::Head, "/register") => redirect_response("/login"),
        (&Method::Post, "/login") => web_form_login_response(request, config.tailcat_ingress),
        (&Method::Post, "/register") => redirect_response("/login?error=registration_unavailable"),
        (&Method::Post, "/logout") => web_logout_response(request, true),
        (&Method::Post, "/settings/password") => redirect_response("/settings"),
        (&Method::Get, _) if path.starts_with("/auth/mobile-handoff/") => {
            web_mobile_handoff_response(request, path)
        }
        (_, _) if path.starts_with("/auth/mobile-handoff/") => method_not_allowed(),
        (&Method::Get, _) | (&Method::Head, _) if path.starts_with("/auth/") => {
            redirect_response("/login")
        }
        (&Method::Post, "/link-backend") => redirect_response("/settings"),
        (_, _) if path.starts_with("/api/") => {
            json_response(StatusCode(404), json!({"error": "not_found", "path": path}))
        }
        _ => html_response(StatusCode(404), error_html(404, "Not found")),
    }
}

fn web_browser_sessions() -> &'static Mutex<HashMap<String, WebBrowserSession>> {
    WEB_BROWSER_SESSIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn web_browser_handoffs() -> &'static Mutex<HashMap<String, WebBrowserHandoff>> {
    WEB_BROWSER_HANDOFFS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn web_cleanup_handoffs_locked(handoffs: &mut HashMap<String, WebBrowserHandoff>, now: Instant) {
    handoffs.retain(|_, handoff| handoff.expires_at > now);
}

fn insert_web_handoff(
    handoffs: &mut HashMap<String, WebBrowserHandoff>,
    target: &str,
    now: Instant,
) -> String {
    web_cleanup_handoffs_locked(handoffs, now);
    while handoffs.len() >= WEB_HANDOFF_LIMIT {
        let Some(oldest) = handoffs
            .iter()
            .min_by_key(|(_, handoff)| handoff.created_at)
            .map(|(digest, _)| digest.clone())
        else {
            break;
        };
        handoffs.remove(&oldest);
    }
    let raw_code = nola_core::mobile_crypto::generate_mobile_access_token();
    handoffs.insert(
        web_secret_digest(&raw_code),
        WebBrowserHandoff {
            target: safe_web_next(target),
            created_at: now,
            expires_at: now + StdDuration::from_secs(WEB_HANDOFF_TTL_SEC),
        },
    );
    raw_code
}

fn consume_web_handoff(code: &str) -> Option<String> {
    let now = Instant::now();
    let mut handoffs = web_browser_handoffs().lock().ok()?;
    web_cleanup_handoffs_locked(&mut handoffs, now);
    handoffs
        .remove(&web_secret_digest(code))
        .map(|handoff| handoff.target)
}

fn mobile_web_handoff_response(request: &mut Request) -> Response<Box<dyn Read + Send>> {
    let Some(token) = mobile_bearer_token(request) else {
        return json_response(StatusCode(401), json!({"error": "unauthorized"}));
    };
    let body = match read_mobile_json_value(request, &token) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let target = safe_web_next(
        body.get("target")
            .and_then(Value::as_str)
            .unwrap_or("/dashboard"),
    );
    let code = {
        let Ok(mut handoffs) = web_browser_handoffs().lock() else {
            return mobile_encrypted_json_response(
                StatusCode(500),
                json!({"error": "handoff_creation_failed"}),
                &token,
            );
        };
        insert_web_handoff(&mut handoffs, &target, Instant::now())
    };
    mobile_encrypted_json_response(
        StatusCode(201),
        json!({
            "path": format!("/auth/mobile-handoff/{code}"),
            "expiresIn": WEB_HANDOFF_TTL_SEC,
            "target": target,
        }),
        &token,
    )
}

fn web_mobile_handoff_response(request: &Request, path: &str) -> Response<Box<dyn Read + Send>> {
    let code = path
        .strip_prefix("/auth/mobile-handoff/")
        .unwrap_or("")
        .trim();
    if code.is_empty() || code.contains('/') {
        return redirect_response("/login?error=invalid_handoff");
    }
    let Some(target) = consume_web_handoff(code) else {
        return redirect_response("/login?error=expired_handoff");
    };
    let (session, csrf) = match create_web_session() {
        Ok(value) => value,
        Err(_) => return redirect_response("/login?error=session_creation_failed"),
    };
    with_web_session_cookies(redirect_response(&target), request, &session, &csrf)
        .with_header(Header::from_bytes(&b"Cache-Control"[..], &b"no-store"[..]).unwrap())
}

fn web_login_failures() -> &'static Mutex<HashMap<String, (u32, Instant)>> {
    WEB_LOGIN_FAILURES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn web_secret_digest(value: &str) -> String {
    nola_core::mobile_crypto::mobile_body_sha256(value.as_bytes())
}

fn request_cookie(request: &Request, name: &str) -> Option<String> {
    request_header(request, "Cookie")?
        .split(';')
        .filter_map(|part| part.trim().split_once('='))
        .find_map(|(key, value)| (key == name).then(|| value.trim().to_string()))
        .filter(|value| !value.is_empty())
}

fn web_cleanup_sessions_locked(sessions: &mut HashMap<String, WebBrowserSession>, now: Instant) {
    sessions.retain(|_, session| session.expires_at > now);
}

fn web_session_is_authenticated(request: &Request) -> bool {
    let Some(raw_session) = request_cookie(request, WEB_SESSION_COOKIE) else {
        return false;
    };
    let digest = web_secret_digest(&raw_session);
    let now = Instant::now();
    let Ok(mut sessions) = web_browser_sessions().lock() else {
        return false;
    };
    web_cleanup_sessions_locked(&mut sessions, now);
    sessions.contains_key(&digest)
}

fn web_csrf_token_matches_session(request: &Request) -> bool {
    let Some(raw_session) = request_cookie(request, WEB_SESSION_COOKIE) else {
        return false;
    };
    let Some(cookie_token) = request_cookie(request, WEB_CSRF_COOKIE) else {
        return false;
    };
    let Some(header_token) = request_header(request, "X-Nola-CSRF") else {
        return false;
    };
    if !nola_core::mobile_crypto::constant_time_secret_eq(&cookie_token, header_token) {
        return false;
    }
    let session_digest = web_secret_digest(&raw_session);
    let csrf_digest = web_secret_digest(header_token);
    let now = Instant::now();
    let Ok(mut sessions) = web_browser_sessions().lock() else {
        return false;
    };
    web_cleanup_sessions_locked(&mut sessions, now);
    sessions.get(&session_digest).is_some_and(|session| {
        nola_core::mobile_crypto::constant_time_secret_eq(&session.csrf_digest, &csrf_digest)
    })
}

fn web_csrf_is_valid(request: &Request) -> bool {
    if !web_request_is_same_origin(request) {
        return false;
    }
    web_csrf_token_matches_session(request)
}

fn web_request_is_same_origin(request: &Request) -> bool {
    if request_header(request, "Sec-Fetch-Site")
        .is_some_and(|value| value.eq_ignore_ascii_case("cross-site"))
    {
        return false;
    }
    let Some(origin) = request_header(request, "Origin") else {
        return false;
    };
    let Some(host) = request_header(request, "Host") else {
        return false;
    };
    let origin = origin.trim_end_matches('/');
    origin.eq_ignore_ascii_case(&format!("http://{host}"))
        || origin.eq_ignore_ascii_case(&format!("https://{host}"))
}

fn web_method_requires_csrf(method: &Method) -> bool {
    !matches!(method, &Method::Get | &Method::Head)
}

fn web_csrf_exempt_path(path: &str) -> bool {
    path == "/login"
        || path == "/api/web/login"
        || path.starts_with("/api/mobile/")
}

fn web_auth_required_response(
    method: &Method,
    url: &str,
    path: &str,
) -> Response<Box<dyn Read + Send>> {
    if matches!(method, &Method::Get | &Method::Head) && !path.starts_with("/api/") {
        let next = safe_web_next(url);
        return redirect_response(&format!("/login?next={}", encode_query_component(&next)));
    }
    json_response(
        StatusCode(401),
        json!({
            "error": "web_session_required",
            "message": "Sign in with the Web Host access token.",
            "login": "/login",
        }),
    )
}

fn safe_web_next(value: &str) -> String {
    if value.contains('\\') || value.chars().any(char::is_control) {
        return "/dashboard".to_string();
    }
    let value = value.trim();
    if !value.starts_with('/') || value.starts_with("//") {
        return "/dashboard".to_string();
    }
    let Ok(url) = reqwest::Url::parse(&format!("https://nola.invalid{value}")) else {
        return "/dashboard".to_string();
    };
    if url.host_str() != Some("nola.invalid")
        || url.path().starts_with("//")
        || url.path().starts_with("/login")
        || url.path() == "/api"
        || url.path().starts_with("/api/")
    {
        return "/dashboard".to_string();
    }
    format!(
        "{}{}{}",
        url.path(),
        url.query()
            .map(|value| format!("?{value}"))
            .unwrap_or_default(),
        url.fragment()
            .map(|value| format!("#{value}"))
            .unwrap_or_default()
    )
}

fn encode_query_component(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

const WEB_LOGIN_TAILCAT_GLOBAL_FAILURE_LIMIT: u32 = 60;

fn web_login_key_from_parts(
    peer: Option<&std::net::SocketAddr>,
    tailcat_ingress: bool,
    client_id: Option<&str>,
    forwarded_for: Option<&str>,
    user_agent: Option<&str>,
) -> String {
    let ip = peer
        .map(|address| address.ip().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    if !tailcat_ingress || !peer.is_some_and(|address| address.ip().is_loopback()) {
        return ip;
    }
    let hint = client_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| forwarded_for.map(str::trim).filter(|value| !value.is_empty()))
        .or_else(|| user_agent.map(str::trim).filter(|value| !value.is_empty()))
        .unwrap_or("default");
    let ua = user_agent.map(str::trim).unwrap_or("");
    let digest = web_secret_digest(&format!("{hint}\n{ua}"));
    format!("tailcat:{ip}:{}", &digest[..16])
}

fn web_login_key(request: &Request, tailcat_ingress: bool) -> String {
    web_login_key_from_parts(
        request.remote_addr(),
        tailcat_ingress,
        request_header(request, "X-Nola-Client-Id"),
        request_header(request, "X-Forwarded-For"),
        request_header(request, "User-Agent"),
    )
}

fn web_login_is_rate_limited(key: &str) -> bool {
    let now = Instant::now();
    let mut failures = web_login_failures()
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    failures.retain(|_, (_, last)| {
        now.saturating_duration_since(*last).as_secs() < WEB_LOGIN_FAILURE_WINDOW_SEC
    });
    if failures
        .get(key)
        .is_some_and(|(count, _)| *count >= WEB_LOGIN_FAILURE_LIMIT)
    {
        return true;
    }
    if key.starts_with("tailcat:") {
        let total_tailcat_failures: u32 = failures
            .iter()
            .filter(|(candidate, _)| candidate.starts_with("tailcat:"))
            .map(|(_, (count, _))| *count)
            .fold(0u32, u32::saturating_add);
        if total_tailcat_failures >= WEB_LOGIN_TAILCAT_GLOBAL_FAILURE_LIMIT {
            return true;
        }
    }
    false
}

fn record_web_login_failure(key: &str) {
    let now = Instant::now();
    let mut failures = web_login_failures()
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    failures.retain(|_, (_, last)| {
        now.saturating_duration_since(*last).as_secs() < WEB_LOGIN_FAILURE_WINDOW_SEC
    });
    if failures.len() >= 1024 && !failures.contains_key(key) {
        return;
    }
    let entry = failures.entry(key.to_string()).or_insert((0, now));
    entry.0 = entry.0.saturating_add(1);
    entry.1 = now;
}

fn clear_web_login_failures(key: &str) {
    let mut failures = web_login_failures()
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    failures.remove(key);
}

fn web_host_token_matches(candidate: &str) -> bool {
    WEB_HOST_ACCESS_TOKEN.get().is_some_and(|expected| {
        nola_core::mobile_crypto::constant_time_secret_eq(candidate.trim(), expected)
    })
}

fn insert_web_session(
    sessions: &mut HashMap<String, WebBrowserSession>,
    now: Instant,
) -> (String, String) {
    web_cleanup_sessions_locked(sessions, now);
    while sessions.len() >= WEB_SESSION_LIMIT {
        let Some(oldest) = sessions
            .iter()
            .min_by_key(|(_, session)| session.created_at)
            .map(|(digest, _)| digest.clone())
        else {
            break;
        };
        sessions.remove(&oldest);
    }
    let raw_session = nola_core::mobile_crypto::generate_mobile_access_token();
    let raw_csrf = nola_core::mobile_crypto::generate_mobile_access_token();
    sessions.insert(
        web_secret_digest(&raw_session),
        WebBrowserSession {
            csrf_digest: web_secret_digest(&raw_csrf),
            created_at: now,
            expires_at: now + StdDuration::from_secs(WEB_SESSION_TTL_SEC),
        },
    );
    (raw_session, raw_csrf)
}

fn create_web_session() -> Result<(String, String), String> {
    let mut sessions = web_browser_sessions()
        .lock()
        .map_err(|_| "failed to lock Web session store".to_string())?;
    Ok(insert_web_session(&mut sessions, Instant::now()))
}

fn revoke_web_session(request: &Request) {
    let Some(raw_session) = request_cookie(request, WEB_SESSION_COOKIE) else {
        return;
    };
    if let Ok(mut sessions) = web_browser_sessions().lock() {
        sessions.remove(&web_secret_digest(&raw_session));
    }
}

fn request_uses_https(request: &Request) -> bool {
    std::env::var("NOLA_TRUST_HTTPS_PROXY")
        .ok()
        .is_some_and(|value| value.eq_ignore_ascii_case("true") || value == "1")
        && request_header(request, "X-Forwarded-Proto")
            .is_some_and(|value| value.eq_ignore_ascii_case("https"))
}

fn web_cookie_header(
    name: &str,
    value: &str,
    max_age: u64,
    http_only: bool,
    secure: bool,
) -> Header {
    let mut cookie = format!("{name}={value}; Path=/; Max-Age={max_age}; SameSite=Strict");
    if http_only {
        cookie.push_str("; HttpOnly");
    }
    if secure {
        cookie.push_str("; Secure");
    }
    Header::from_bytes(&b"Set-Cookie"[..], cookie.as_bytes()).unwrap()
}

fn web_clear_cookie_header(name: &str, http_only: bool, secure: bool) -> Header {
    let mut cookie = format!(
        "{name}=; Path=/; Max-Age=0; Expires=Thu, 01 Jan 1970 00:00:00 GMT; SameSite=Strict"
    );
    if http_only {
        cookie.push_str("; HttpOnly");
    }
    if secure {
        cookie.push_str("; Secure");
    }
    Header::from_bytes(&b"Set-Cookie"[..], cookie.as_bytes()).unwrap()
}

fn with_web_session_cookies(
    response: Response<Box<dyn Read + Send>>,
    request: &Request,
    session: &str,
    csrf: &str,
) -> Response<Box<dyn Read + Send>> {
    let secure = request_uses_https(request);
    response
        .with_header(web_cookie_header(
            WEB_SESSION_COOKIE,
            session,
            WEB_SESSION_TTL_SEC,
            true,
            secure,
        ))
        .with_header(web_cookie_header(
            WEB_CSRF_COOKIE,
            csrf,
            WEB_SESSION_TTL_SEC,
            false,
            secure,
        ))
        .with_header(Header::from_bytes(&b"Cache-Control"[..], &b"no-store"[..]).unwrap())
}

fn web_login_result(
    request: &Request,
    token: &str,
    tailcat_ingress: bool,
) -> Result<(String, String), Response<Box<dyn Read + Send>>> {
    let key = web_login_key(request, tailcat_ingress);
    if web_login_is_rate_limited(&key) {
        return Err(json_response(
            StatusCode(429),
            json!({"error": "too_many_login_attempts", "message": "Try again in a few minutes."}),
        ));
    }
    if !web_host_token_matches(token) {
        record_web_login_failure(&key);
        return Err(json_response(
            StatusCode(401),
            json!({"error": "invalid_host_token", "message": "The Web Host token is invalid."}),
        ));
    }
    clear_web_login_failures(&key);
    create_web_session().map_err(|message| {
        json_response(
            StatusCode(500),
            json!({"error": "session_creation_failed", "message": message}),
        )
    })
}

fn web_loopback_is_trusted(peer: Option<&std::net::SocketAddr>, tailcat_ingress: bool) -> bool {
    !tailcat_ingress && peer.is_some_and(|address| address.ip().is_loopback())
}

fn is_loopback_hostname(host: &str) -> bool {
    matches!(
        host.to_ascii_lowercase().as_str(),
        "127.0.0.1" | "localhost" | "[::1]"
    )
}

fn is_loopback_host_header(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.is_empty()
        || trimmed.bytes().any(|byte| {
            byte.is_ascii_whitespace()
                || matches!(byte, b'/' | b'@' | b'?' | b'#' | b'\\' | b',')
        })
    {
        return false;
    }
    let Ok(parsed) = reqwest::Url::parse(&format!("http://{trimmed}")) else {
        return false;
    };
    parsed.username().is_empty()
        && parsed.password().is_none()
        && parsed.path() == "/"
        && parsed.query().is_none()
        && parsed.fragment().is_none()
        && parsed.host_str().is_some_and(is_loopback_hostname)
}

fn is_loopback_origin_header(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.bytes().any(|byte| byte.is_ascii_whitespace()) {
        return false;
    }
    let Ok(parsed) = reqwest::Url::parse(trimmed) else {
        return false;
    };
    matches!(parsed.scheme(), "http" | "https")
        && parsed.username().is_empty()
        && parsed.password().is_none()
        && parsed.path() == "/"
        && parsed.query().is_none()
        && parsed.fragment().is_none()
        && parsed.host_str().is_some_and(is_loopback_hostname)
}

fn is_mutating_http_method(method: &Method) -> bool {
    matches!(
        method,
        &Method::Post | &Method::Put | &Method::Patch | &Method::Delete
    )
}

fn validate_local_trust_api_headers(
    method: &Method,
    host_headers: &[&str],
    origin_headers: &[&str],
    sec_fetch_site_headers: &[&str],
) -> Result<(), (&'static str, &'static str)> {
    if host_headers.len() > 1
        || host_headers
            .first()
            .is_some_and(|host| !is_loopback_host_header(host))
    {
        return Err((
            "invalid_local_host",
            "Local Web API requests require a loopback Host header.",
        ));
    }
    if is_mutating_http_method(method) {
        if origin_headers.len() > 1
            || origin_headers
                .first()
                .is_some_and(|origin| !is_loopback_origin_header(origin))
        {
            return Err((
                "cross_origin_forbidden",
                "Local Web API mutations reject non-loopback Origin headers.",
            ));
        }
        if sec_fetch_site_headers
            .iter()
            .any(|site| site.trim().eq_ignore_ascii_case("cross-site"))
        {
            return Err((
                "cross_site_forbidden",
                "Local Web API mutations reject cross-site browser requests.",
            ));
        }
    }
    Ok(())
}

fn validate_local_trust_api_request(
    request: &Request,
    method: &Method,
) -> Result<(), (&'static str, &'static str)> {
    let host_headers: Vec<&str> = request
        .headers()
        .iter()
        .filter(|header| header.field.to_string().eq_ignore_ascii_case("Host"))
        .map(|header| header.value.as_str())
        .collect();
    let origin_headers: Vec<&str> = request
        .headers()
        .iter()
        .filter(|header| header.field.to_string().eq_ignore_ascii_case("Origin"))
        .map(|header| header.value.as_str())
        .collect();
    let sec_fetch_site_headers: Vec<&str> = request
        .headers()
        .iter()
        .filter(|header| header.field.to_string().eq_ignore_ascii_case("Sec-Fetch-Site"))
        .map(|header| header.value.as_str())
        .collect();
    validate_local_trust_api_headers(
        method,
        &host_headers,
        &origin_headers,
        &sec_fetch_site_headers,
    )
}

#[cfg(test)]
mod local_trust_host_csrf_tests {
    use super::*;

    #[test]
    fn loopback_host_header_blocks_dns_rebinding_and_allows_loopback() {
        for valid in [
            "127.0.0.1",
            "127.0.0.1:8787",
            "localhost",
            "LOCALHOST:3000",
            "[::1]",
            "[::1]:8787",
        ] {
            assert!(is_loopback_host_header(valid), "expected valid host: {valid}");
            assert!(validate_local_trust_api_headers(&Method::Get, &[valid], &[], &[]).is_ok());
        }
        // Absent Host header stays allowed for minimal HTTP/1.0 or local CLI probes.
        assert!(validate_local_trust_api_headers(&Method::Get, &[], &[], &[]).is_ok());

        for invalid in [
            "",
            "evil.com",
            "rebind.evil.com:8787",
            "localhost.evil.com",
            "127.0.0.1.nip.io:8787",
            "127.0.0.1@evil.com",
            "192.168.1.5:8787",
            "0.0.0.0:8787",
            "localhost/extra",
            "localhost:abc",
        ] {
            assert!(!is_loopback_host_header(invalid), "expected invalid host: {invalid}");
            assert_eq!(
                validate_local_trust_api_headers(&Method::Get, &[invalid], &[], &[])
                    .unwrap_err()
                    .0,
                "invalid_local_host"
            );
        }
        assert_eq!(
            validate_local_trust_api_headers(
                &Method::Get,
                &["127.0.0.1:8787", "evil.com"],
                &[],
                &[],
            )
            .unwrap_err()
            .0,
            "invalid_local_host"
        );
    }

    #[test]
    fn local_trust_mutations_block_cross_site_origin_and_sec_fetch_site_while_allowing_cli() {
        for method in [Method::Post, Method::Put, Method::Patch, Method::Delete] {
            // CLI / curl with no Origin or Sec-Fetch-Site continues working.
            assert!(validate_local_trust_api_headers(&method, &["127.0.0.1:8787"], &[], &[]).is_ok());

            // Same-origin / local loopback browser requests succeed.
            for origin in [
                "http://127.0.0.1:8787",
                "http://localhost:5173",
                "https://[::1]:8787",
            ] {
                assert!(validate_local_trust_api_headers(
                    &method,
                    &["127.0.0.1:8787"],
                    &[origin],
                    &["same-origin"],
                )
                .is_ok());
            }

            // Cross-origin and null origins are rejected.
            for bad_origin in [
                "https://evil.com",
                "http://localhost.evil.com:8787",
                "null",
                "file://localhost",
            ] {
                assert_eq!(
                    validate_local_trust_api_headers(
                        &method,
                        &["127.0.0.1:8787"],
                        &[bad_origin],
                        &[],
                    )
                    .unwrap_err()
                    .0,
                    "cross_origin_forbidden"
                );
            }

            // Cross-site Sec-Fetch-Site is rejected.
            assert_eq!(
                validate_local_trust_api_headers(
                    &method,
                    &["127.0.0.1:8787"],
                    &[],
                    &["cross-site"],
                )
                .unwrap_err()
                .0,
                "cross_site_forbidden"
            );
            assert_eq!(
                validate_local_trust_api_headers(
                    &method,
                    &["127.0.0.1:8787"],
                    &["http://127.0.0.1:8787"],
                    &["Cross-Site"],
                )
                .unwrap_err()
                .0,
                "cross_site_forbidden"
            );
        }

        // Read-only GET with loopback Host is not blocked by Origin/Sec-Fetch-Site.
        assert!(validate_local_trust_api_headers(
            &Method::Get,
            &["127.0.0.1:8787"],
            &["https://evil.com"],
            &["cross-site"],
        )
        .is_ok());
    }
}

#[cfg(test)]
mod tailcat_ingress_security_tests {
    use super::*;
    #[test]
    fn tailcat_loopback_requires_auth_and_never_becomes_a_local_account_peer() {
        for peer in ["127.0.0.1:5001", "[::1]:5001"] {
            let address = peer.parse().unwrap();
            assert!(web_loopback_is_trusted(Some(&address), false));
            assert!(!web_loopback_is_trusted(Some(&address), true));
            assert!(!source_management_peer_allowed(Some(&address), true));
            assert!(mobile_login_peer_allowed(Some(&address), false, false));
            assert!(!mobile_login_peer_allowed(Some(&address), true, false));
            assert!(mobile_login_peer_allowed(Some(&address), true, true));
        }
        assert!(!web_loopback_is_trusted(None, false));
        assert!(!web_loopback_is_trusted(Some(&"192.0.2.1:5001".parse().unwrap()), false));
        // Authentication stays at the existing gate; journal mutation isn't a
        // public route/CSRF exemption merely because the peer is a tunnel.
        assert!(!remote_web_host_route_allowed(&Method::Post, "/api/assistant-journal"));
    }

    #[test]
    fn tailcat_login_rate_limit_isolates_distinct_clients_on_loopback() {
        let loopback: std::net::SocketAddr = "127.0.0.1:5001".parse().unwrap();
        let nonce = uuid::Uuid::new_v4().to_string();
        let client_a = web_login_key_from_parts(
            Some(&loopback),
            true,
            Some(&format!("client-a-{nonce}")),
            None,
            Some("Mozilla/5.0 A"),
        );
        let client_b = web_login_key_from_parts(
            Some(&loopback),
            true,
            Some(&format!("client-b-{nonce}")),
            None,
            Some("Mozilla/5.0 B"),
        );
        assert_ne!(client_a, client_b);
        assert!(client_a.starts_with("tailcat:127.0.0.1:"));
        assert_eq!(
            web_login_key_from_parts(Some(&loopback), false, Some("client-a"), None, None),
            "127.0.0.1"
        );

        for _ in 0..WEB_LOGIN_FAILURE_LIMIT {
            record_web_login_failure(&client_a);
        }
        assert!(web_login_is_rate_limited(&client_a));
        assert!(
            !web_login_is_rate_limited(&client_b),
            "one tailcat client failing login must not lock out distinct remote clients"
        );
        clear_web_login_failures(&client_a);
        clear_web_login_failures(&client_b);
    }
}

fn web_session_response(request: &Request, config: &Config) -> Response<Box<dyn Read + Send>> {
    let loopback = web_loopback_is_trusted(request.remote_addr(), config.tailcat_ingress);
    let session_authenticated = web_session_is_authenticated(request);
    json_response(
        StatusCode(200),
        json!({
            "authenticated": loopback || session_authenticated,
            "sessionAuthenticated": session_authenticated,
            "localTrust": loopback,
            "tailcatAvailable": config.tailcat_helper.is_some(),
            "transportIngress": if config.tailcat_ingress { "tailcat" } else { "direct" },
            "expiresIn": if session_authenticated { WEB_SESSION_TTL_SEC } else { 0 },
            "hostId": host_identity(),
        }),
    )
    .with_header(Header::from_bytes(&b"Cache-Control"[..], &b"no-store"[..]).unwrap())
}

fn web_json_login_response(
    request: &mut Request,
    tailcat_ingress: bool,
) -> Response<Box<dyn Read + Send>> {
    let body = match read_json_value(request) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let token = body.get("token").and_then(Value::as_str).unwrap_or("");
    let (session, csrf) = match web_login_result(request, token, tailcat_ingress) {
        Ok(value) => value,
        Err(response) => return response,
    };
    with_web_session_cookies(
        json_response(
            StatusCode(200),
            json!({
                "authenticated": true,
                "expiresIn": WEB_SESSION_TTL_SEC,
                "hostId": host_identity(),
            }),
        ),
        request,
        &session,
        &csrf,
    )
}

fn read_urlencoded_form(
    request: &mut Request,
) -> Result<HashMap<String, String>, (StatusCode, String)> {
    let body = read_bounded_utf8(request.as_reader(), MAX_URLENCODED_FORM_BODY_BYTES).map_err(
        |error| match error {
            BoundedBodyError::TooLarge => (StatusCode(413), "login form is too large".to_string()),
            BoundedBodyError::Read(message) => {
                (StatusCode(400), format!("failed to read login form: {message}"))
            }
        },
    )?;
    Ok(body
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .map(|(key, value)| (percent_decode(key), percent_decode(value)))
        .collect())
}

fn web_form_login_response(
    request: &mut Request,
    tailcat_ingress: bool,
) -> Response<Box<dyn Read + Send>> {
    let form = match read_urlencoded_form(request) {
        Ok(value) => value,
        Err((status, message)) => {
            return html_response(status, error_html(status.0, &message));
        }
    };
    let next = safe_web_next(form.get("next").map(String::as_str).unwrap_or("/dashboard"));
    let token = form.get("token").map(String::as_str).unwrap_or("");
    let (session, csrf) = match web_login_result(request, token, tailcat_ingress) {
        Ok(value) => value,
        Err(response) => {
            let status = response.status_code();
            let error = if status == StatusCode(429) {
                "rate_limited"
            } else {
                "invalid_token"
            };
            return redirect_response(&format!(
                "/login?error={error}&next={}",
                encode_query_component(&next)
            ));
        }
    };
    with_web_session_cookies(redirect_response(&next), request, &session, &csrf)
}

fn web_logout_response(request: &Request, redirect: bool) -> Response<Box<dyn Read + Send>> {
    revoke_web_session(request);
    let secure = request_uses_https(request);
    let response = if redirect {
        redirect_response("/login?logged_out=1")
    } else {
        json_response(StatusCode(200), json!({"authenticated": false}))
    };
    response
        .with_header(web_clear_cookie_header(WEB_SESSION_COOKIE, true, secure))
        .with_header(web_clear_cookie_header(WEB_CSRF_COOKIE, false, secure))
        .with_header(Header::from_bytes(&b"Cache-Control"[..], &b"no-store"[..]).unwrap())
}

fn with_web_security_headers(
    response: Response<Box<dyn Read + Send>>,
) -> Response<Box<dyn Read + Send>> {
    response
        .with_header(Header::from_bytes(&b"X-Content-Type-Options"[..], &b"nosniff"[..]).unwrap())
        .with_header(Header::from_bytes(&b"X-Frame-Options"[..], &b"DENY"[..]).unwrap())
        .with_header(Header::from_bytes(&b"Referrer-Policy"[..], &b"no-referrer"[..]).unwrap())
        .with_header(
            Header::from_bytes(
                &b"Content-Security-Policy"[..],
                &b"frame-ancestors 'none'; base-uri 'self'; form-action 'self'"[..],
            )
            .unwrap(),
        )
}

fn models_response(config: &Config) -> Response<Box<dyn Read + Send>> {
    let model = if config.model.trim().is_empty() {
        "configured-model"
    } else {
        config.model.trim()
    };
    json_response(
        StatusCode(200),
        json!({
            "models": [
                {
                    "id": model,
                    "name": model,
                    "source": "cli",
                }
            ],
            "activeModel": model,
            "endpointConfigured": !config.endpoint.trim().is_empty(),
        }),
    )
}

fn local_model_action_route(path: &str) -> Option<(&str, &str)> {
    let rest = path.strip_prefix("/api/local-models/")?;
    let (model_id, action) = rest.split_once('/')?;
    if model_id.is_empty()
        || action.is_empty()
        || action.contains('/')
        || !local_models::contains_model(model_id)
    {
        return None;
    }
    Some((model_id, action))
}

fn local_model_action_response(
    request: &mut Request,
    method: &Method,
    model_id: &str,
    action: &str,
) -> Response<Box<dyn Read + Send>> {
    match (method, action) {
        (&Method::Post, "download") => match local_models::start_download(model_id) {
            Ok(()) => json_response(
                StatusCode(202),
                json!({"ok": true, "modelId": model_id, "status": "downloading"}),
            ),
            Err(error) => json_response(StatusCode(400), json!({"error": error})),
        },
        (&Method::Post, "start") => match local_models::start_model(model_id) {
            Ok(endpoint) => json_response(
                StatusCode(200),
                json!({
                    "ok": true,
                    "modelId": endpoint.model_id,
                    "contextWindowTokens": endpoint.context_window_tokens,
                    "running": true,
                    "localFirst": true,
                }),
            ),
            Err(error) => json_response(StatusCode(409), json!({"error": error})),
        },
        (&Method::Post, "select") => match local_models::select_model(model_id) {
            Ok(()) => json_response(
                StatusCode(200),
                json!({"ok": true, "modelId": model_id, "selected": true}),
            ),
            Err(error) => json_response(StatusCode(400), json!({"error": error})),
        },
        (&Method::Post, "stop") => match local_models::stop_model(Some(model_id)) {
            Ok(()) => json_response(StatusCode(200), json!({"ok": true, "modelId": model_id})),
            Err(error) => json_response(StatusCode(409), json!({"error": error})),
        },
        (&Method::Post, "configuration") => {
            #[derive(serde::Deserialize)]
            #[serde(rename_all = "camelCase", deny_unknown_fields)]
            struct ConfigurationWrite {
                expected_revision: u64,
                parameters: local_models::LocalModelParameters,
            }
            let input = match read_json::<ConfigurationWrite>(request) {
                Ok(value) => value,
                Err(response) => return response,
            };
            match local_models::save_parameters_at_revision(
                model_id,
                input.parameters,
                input.expected_revision,
            ) {
                Ok(parameters) => json_response(
                    StatusCode(200),
                    json!({"ok": true, "modelId": model_id, "parameters": parameters}),
                ),
                Err(error) if error.starts_with("revision_conflict:") => json_response(
                    StatusCode(409),
                    json!({"code": "revision_conflict", "error": error}),
                ),
                Err(error) => json_response(StatusCode(400), json!({"error": error})),
            }
        }
        (&Method::Delete, "installation") => match local_models::remove_model(model_id) {
            Ok(()) => json_response(StatusCode(200), json!({"ok": true, "modelId": model_id})),
            Err(error) => json_response(StatusCode(409), json!({"error": error})),
        },
        _ => method_not_allowed(),
    }
}

fn local_store_response() -> Response<Box<dyn Read + Send>> {
    match NolaStore::open_default() {
        Ok(store) => json_response(
            StatusCode(200),
            json!({
                "ok": true,
                "database": store.path().display().to_string(),
                "config": "~/.nola/config.json",
                "ownership": {
                    "configJson": ["llm", "models", "defaults", "ui", "permission defaults"],
                    "sqlite": ["sessions", "messages", "tool_events", "goals", "memories", "documents", "permission_grants", "sync_outbox"]
                }
            }),
        ),
        Err(err) => json_response(
            StatusCode(500),
            json!({
                "ok": false,
                "error": err,
            }),
        ),
    }
}

fn tasks_response() -> Response<Box<dyn Read + Send>> {
    json_response(
        StatusCode(200),
        json!({
            "tasks": [
                {
                    "id": 1,
                    "name": "Local session",
                    "description": "Signed in to CLI-local web backend",
                    "status": "completed",
                },
                {
                    "id": 2,
                    "name": "Storage mode",
                    "description": "Local-first",
                    "status": "completed",
                },
            ],
            "client_uuid": "cli-local-user",
            "logged_in": true,
            "localFirst": true,
            "backend": "cli-rust",
        }),
    )
}

fn create_task_response(request: &mut Request) -> Response<Box<dyn Read + Send>> {
    let body = match read_json_value(request) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let token_cost = match non_negative_int_field(&body, "token_cost", 1) {
        Ok(value) => value,
        Err(err) => return json_response(StatusCode(400), json!({"success": false, "error": err})),
    };
    let requests = match non_negative_int_field(&body, "requests", 1) {
        Ok(value) => value,
        Err(err) => return json_response(StatusCode(400), json!({"success": false, "error": err})),
    };
    json_response(
        StatusCode(200),
        json!({
            "success": true,
            "token_cost": token_cost,
            "requests": requests,
            "localFirst": true,
            "backend": "cli-rust",
        }),
    )
}

fn read_conversation_response(conversation_id: &str) -> Response<Box<dyn Read + Send>> {
    match NolaStore::open_default().and_then(|store| store.get_web_conversation(conversation_id)) {
        Ok(Some(mut conversation)) => {
            match append_expert_context_hint(conversation_id, &mut conversation) {
                Ok(())=>json_response(StatusCode(200),conversation),
                Err(_)=>json_response(StatusCode(503),json!({"code":"conversation_owner_unavailable","error":"Cannot confirm conversation ownership"})),
            }
        },
        Ok(None) => json_response(StatusCode(404), json!({"error": "conversation not found"})),
        Err(error) => json_response(StatusCode(500), conversation_inventory_failure(&error)),
    }
}

/// Read-only navigation metadata comes from the protected ContextOwner. Even
/// an unavailable/retired Room must never turn its child into an ordinary chat.
fn append_expert_context_hint(conversation_id: &str, conversation: &mut Value) -> Result<(),String> {
    let mut app=crate::application_port::CliApplicationPort::persistent_default()?;
    append_expert_context_hint_from_application(&mut app,conversation_id,conversation)
}

fn append_expert_context_hint_from_application(app: &mut crate::application_port::CliApplicationPort, conversation_id: &str, conversation: &mut Value) -> Result<(),String> {
    use nola_application::{ApplicationCommand,agent_room::{AgentRoomCommand,ConversationRoomAssociation}};
    let Some(raw)=app.load_conversation_settings(conversation_id)? else {return Ok(());};
    let settings:Value=serde_json::from_str(&raw).map_err(|_|"invalid conversation settings")?;
    let Some(raw_owner)=settings.get(nola_domain::agent_room::CONTEXT_OWNER_FIELD) else {return Ok(());};
    let owner:nola_domain::agent_room::ContextOwner=serde_json::from_value(raw_owner.clone()).map_err(|_|"invalid expert owner")?;
    conversation["expertContextOwner"]=json!({"roomId":owner.room_id,"participantId":owner.participant_id,"agentId":owner.agent_id,"agentRevision":null,"isCurrent":false});
    let reverse:Value=app.execute(ApplicationCommand::GetProductSetting {key:format!("{}{}",nola_application::agent_room::ROOM_CONVERSATION_KEY_PREFIX,owner.room_id)})?;
    if let Some(raw)=reverse.get("value_json").and_then(Value::as_str) {
        let link:Value=serde_json::from_str(raw).map_err(|_|"invalid expert parent association")?;
        if let Some(parent)=link.get("conversation_id").and_then(Value::as_str) {
            let association:Option<ConversationRoomAssociation>=app.execute(ApplicationCommand::AgentRoom {command:AgentRoomCommand::ForConversation {conversation_id:parent.into()}})?;
            if let Some(association)=association.filter(|item|item.room_id==owner.room_id) {
                conversation["expertParentConversationId"]=json!(association.conversation_id);
                if let Some(member)=association.room.room.participants().iter().find(|member|member.id==owner.participant_id && member.agent_id==owner.agent_id && member.context_conversation_id==conversation_id) {
                    conversation["expertContextOwner"]["agentRevision"]=json!(member.agent_revision);
                    conversation["expertContextOwner"]["isCurrent"]=json!(member.active);
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod expert_context_route_tests {
    use super::*;
    #[test]
    fn retired_expert_without_current_room_is_still_a_readonly_child() {
        let directory=tempfile::tempdir().unwrap();
        let mut app=crate::application_port::CliApplicationPort::persistent(&directory.path().join("context.sqlite")).unwrap();
        app.create_conversation(nola_application::CreateConversation {
            id:"expert-child".into(),title:"Retired".into(),created_at_ms:1,updated_at_ms:1,
            settings_json:Some(json!({"agentRoomOwner":{"room_id":"retired-room","participant_id":"retired-member","agent_id":"expert"}}).to_string()),
        }).unwrap();
        let mut value=json!({"conversationId":"expert-child"});
        append_expert_context_hint_from_application(&mut app,"expert-child",&mut value).unwrap();
        assert_eq!(value["expertContextOwner"]["participantId"],"retired-member");
        assert_eq!(value["expertContextOwner"]["isCurrent"],false);
        assert!(value["expertContextOwner"]["agentRevision"].is_null());
        assert!(value.get("expertParentConversationId").is_none());
    }
}

fn list_conversations_response() -> Response<Box<dyn Read + Send>> {
    match NolaStore::open_default().and_then(|store| {
        store
            .load_web_conversations()?
            .into_iter()
            .map(|value| {
                serde_json::from_value::<WebConversation>(value)
                    .map_err(|err| format!("parse web conversation failed: {err}"))
            })
            .collect::<Result<Vec<_>, _>>()
    }) {
        Ok(mut conversations) => {
            conversations.sort_by(|a, b| {
                b.updated_at
                    .cmp(&a.updated_at)
                    .then_with(|| a.conversation_id.cmp(&b.conversation_id))
            });
            json_response(
                StatusCode(200),
                json!({"conversations": conversations, "localFirst": true, "backend": "cli-rust"}),
            )
        }
        Err(err) => json_response(StatusCode(500), conversation_inventory_failure(&err)),
    }
}

fn create_conversation_response(request: &mut Request) -> Response<Box<dyn Read + Send>> {
    let input = match read_json::<ConversationInput>(request) {
        Ok(input) => input,
        Err(response) => return response,
    };
    let now = now_string();
    let user_title = input.title.as_deref().filter(|title| !title.trim().is_empty());
    let title = match user_title {
        Some(title) => normalize_title(Some(title)),
        None => crate::i18n::new_conversation_title(request_language(request)).to_owned(),
    };
    let mut conversation = WebConversation::new(new_id("conv"), title, now.clone(), now);
    if user_title.is_some() {
        conversation.title_source = ConversationTitleSource::User;
    }
    if let Err(err) = apply_model_selection(
        &mut conversation,
        input.selected_model_source.as_deref(),
        input.selected_model_id.as_deref(),
    ) {
        return json_response(StatusCode(400), json!({"error": err}));
    }
    if let Err(err) = validate_web_conversation_model(&conversation) {
        return json_response(StatusCode(400), json!({"error": err}));
    }
    if let Some(backend) = input.execution_backend.as_deref() {
        if let Err(err) = apply_execution_backend(&mut conversation, backend) {
            return json_response(StatusCode(400), json!({"error": err}));
        }
    }
    if let Some(sessions)=input.external_execution_sessions {
        conversation.external_execution_sessions=sessions.into_iter().filter_map(|(backend,session)|session.map(|session|(backend,session))).collect();
    }
    match NolaStore::open_default()
        .and_then(|store| {
            store.create_web_conversation(&json!(conversation))?;
            store.get_web_conversation(&conversation.conversation_id)?.ok_or("New conversation disappeared".into())
        })
    {
        Ok(saved) => json_response(StatusCode(200), with_local_first(saved)),
        Err(err) => json_response(StatusCode(500), json!({"error": err})),
    }
}

fn validate_web_conversation_model(conversation: &WebConversation) -> Result<(), String> {
    match conversation.selected_model_source.as_str() {
        "auto" => Ok(()),
        "local" if local_models::contains_model(&conversation.selected_model_id) => Ok(()),
        "cloud" => {
            let profile = NolaStore::open_default()?
                .model_profile(&conversation.selected_model_id)?
                .ok_or("Selected model profile is unavailable")?;
            let source: Value = serde_json::from_str(&profile.source_json)
                .map_err(|_| "Selected model profile is invalid")?;
            if profile.runtime != "remote"
                || !profile.enabled
                || profile.deleted_at_ms.is_some()
                || source["endpointEnabled"] == false
                || source["modelEnabled"] == false
                || !source["capabilities"]
                    .as_array()
                    .is_some_and(|values| values.contains(&json!("chat")))
            {
                return Err("Selected model is not available for chat".into());
            }
            Ok(())
        }
        _ => Err("Selected model is unavailable".into()),
    }
}

fn update_conversation_response(request:&mut Request,conversation_id:&str)->Response<Box<dyn Read+Send>> {
    let input=match read_json::<ConversationInput>(request) {Ok(value)=>value,Err(response)=>return response};
    let store=match NolaStore::open_default() {Ok(value)=>value,Err(error)=>return json_response(StatusCode(500),json!({"error":error}))};
    let changes_model=input.selected_model_source.is_some() || input.selected_model_id.is_some();
    let changes_configuration=changes_model || input.execution_backend.is_some() || input.external_execution_sessions.is_some();
    if input.title.is_some() && changes_configuration {
        return json_response(StatusCode(422),json!({"code":"independent_title_and_configuration_commands_required"}));
    }
    if let Some(title)=input.title {
        let Some(expected)=input.expected_revision.and_then(|value|i64::try_from(value).ok()).filter(|value|*value>0) else {
            return json_response(StatusCode(400),json!({"code":"title_revision_required"}));
        };
        return match store.rename_web_conversation(conversation_id,&normalize_title(Some(&title)),ConversationTitleSource::User,expected) {
            Ok(true)=>match store.get_web_conversation(conversation_id) {Ok(Some(saved))=>json_response(StatusCode(200),with_local_first(saved)),Ok(None)=>not_found("Conversation not found"),Err(error)=>json_response(StatusCode(500),json!({"error":error}))},
            Ok(false)=>not_found("Conversation not found"),
            Err(error)=>json_response(StatusCode(409),json!({"code":"title_commit_conflict","error":error})),
        };
    }
    let Some(expected)=input.expected_configuration_revision.filter(|value|*value>0) else {
        return json_response(StatusCode(400),json!({"code":"configuration_revision_required"}));
    };
    let mut patch=serde_json::Map::new();
    if changes_model {
        let Some(source)=input.selected_model_source.as_deref() else {return json_response(StatusCode(422),json!({"code":"model_source_required"}));};
        let id=input.selected_model_id.as_deref().unwrap_or("");
        let selection=match crate::store::web_model_selection(&json!({"selectedModelSource":source,"selectedModelId":id})) {
            Ok(value)=>value,Err(error)=>return json_response(StatusCode(422),json!({"error":error})),
        };
        let selection=if selection.is_none() {
            match store.resolve_default_web_model_selection() {Ok(value)=>value,Err(error)=>return json_response(StatusCode(422),json!({"error":error}))}
        } else {selection};
        patch.insert("modelSelectionID".into(),selection.map(Value::String).unwrap_or(Value::Null));
    }
    if let Some(backend)=input.execution_backend {patch.insert("executionBackend".into(),json!(backend));}
    if let Some(sessions)=input.external_execution_sessions {patch.insert("externalExecutionSessions".into(),json!(sessions));}
    if patch.is_empty() {return json_response(StatusCode(422),json!({"code":"empty_configuration_patch"}));}
    use nola_domain::conversation_configuration::ConfigurationCommitResult;
    let result = if changes_model {
        (|| -> Result<ConfigurationCommitResult,String> {
            use nola_application::conversation_configuration::{ConversationExecutionSelection,SelectConversationExecutionInput};
            let selection = match patch.get("modelSelectionID").and_then(Value::as_str) {
                Some(id)=>ConversationExecutionSelection::Model {model_id:id.into(),parameters:Default::default(),presentation:Default::default()},
                None=>ConversationExecutionSelection::Inherit,
            };
            let mut app=crate::application_port::CliApplicationPort::persistent_default()?;
            app.execute(nola_application::ApplicationCommand::SelectConversationExecution { input:SelectConversationExecutionInput {
                conversation_id:conversation_id.into(),expected_revision:expected,selection,execution_host:None,
                execution_backend:patch.get("executionBackend").and_then(Value::as_str).map(str::to_owned),
                external_execution_sessions:patch.get("externalExecutionSessions").and_then(Value::as_object).cloned(),
                additional_configuration:None,updated_at_ms:Utc::now().timestamp_millis(),
            }})
        })()
    } else { store.patch_web_conversation_configuration(conversation_id,expected,Value::Object(patch)) };
    match result {
        Ok(ConfigurationCommitResult::Committed {..})=>match store.get_web_conversation(conversation_id) {Ok(Some(saved))=>json_response(StatusCode(200),with_local_first(saved)),Ok(None)=>not_found("Conversation not found"),Err(error)=>json_response(StatusCode(500),json!({"error":error}))},
        Ok(conflict@ConfigurationCommitResult::Conflict {..})=>json_response(StatusCode(409),json!(conflict)),
        Ok(missing@ConfigurationCommitResult::NotFound {..})=>json_response(StatusCode(404),json!(missing)),
        Err(error)=>json_response(StatusCode(422),json!({"code":"invalid_conversation_configuration","error":error})),
    }
}

fn delete_conversation_response(
    config: &Config,
    conversation_id: &str,
) -> Response<Box<dyn Read + Send>> {
    let result = NolaStore::open_default().and_then(|store| {
        if store.get_web_conversation(conversation_id)?.is_none() {
            return Ok(false);
        }
        // Core's idle CloseSession currently emits a lifecycle Host effect,
        // which idle submission admission rejects. The canonical deletion
        // transaction itself guards every nonterminal Run and cascades owned
        // resources. Only retire the process cache after that succeeds.
        let deleted = store.delete_web_conversation(conversation_id)?;
        if deleted {
            crate::core_agent_runtime::invalidate_core_agent_session(conversation_id)?;
            cleanup_conversation_session_workspace(config, conversation_id);
        }
        Ok(deleted)
    });
    match result {
        Ok(true) => json_response(StatusCode(200), json!({"ok": true, "localFirst": true})),
        Ok(false) => not_found("会话不存在"),
        Err(err) if err.contains("cannot delete or replace a conversation while its Run is active") => {
            json_response(
                StatusCode(409),
                json!({"error":"active_run","code":"active_run"}),
            )
        }
        Err(err) => json_response(StatusCode(500), json!({"error": err})),
    }
}

fn historical_part_response(url: &str, conversation_id: &str) -> Response<Box<dyn Read + Send>> {
    let request = (|| -> Result<_, String> {
        let message = query_param(url, "message_id").ok_or("message_id is required")?;
        let part = query_param(url, "part_id").ok_or("part_id is required")?;
        let revision = query_param(url, "revision")
            .ok_or("revision is required")?
            .parse::<i64>()
            .map_err(|_| "revision must be a positive integer")?;
        if revision < 1 {
            return Err("revision must be a positive integer".into());
        }
        let owner = nola_protocol::CanonicalPartOwner::new(conversation_id.into(), message, part)
            .map_err(|error| error.to_string())?;
        Ok((owner, revision))
    })();
    let (owner, revision) = match request {
        Ok(value) => value,
        Err(error) => return json_response(StatusCode(400), json!({"error":error})),
    };
    let result = crate::application_port::CliApplicationPort::persistent_default().and_then(
        |mut application| crate::historical_part::read(&mut application, owner, revision),
    );
    match result {
        Ok(value) => json_response(StatusCode(200), json!(value)),
        Err(error) => json_response(StatusCode(409), json!({"error":error})),
    }
}

/// Append a user-visible chat message without creating the default assistant
/// Run. Expert-only composer sends use this before issuing anchored submissions.
fn admit_user_message_response(request: &mut Request, conversation_id: &str) -> Response<Box<dyn Read + Send>> {
    let input = match read_json::<nola_protocol::AdmitConversationUserMessage>(request) {
        Ok(input) => input,
        Err(response) => return response,
    };
    if input.conversation_id != conversation_id {
        return json_response(StatusCode(400), json!({"error":"conversation_owner_mismatch"}));
    }
    let result = crate::application_port::CliApplicationPort::persistent_default().and_then(|mut application| {
        application.execute::<Value>(nola_application::ApplicationCommand::AdmitConversationUserMessage { input })
    });
    match result {
        Ok(value) => json_response(StatusCode(200), value),
        Err(error) => json_response(StatusCode(409), json!({"error":error})),
    }
}

fn list_messages_response(url: &str, conversation_id: &str) -> Response<Box<dyn Read + Send>> {
    let limit = match query_param(url, "limit") {
        Some(value) => match value.parse::<usize>() {
            Ok(value) if (1..=200).contains(&value) => value,
            _ => {
                return json_response(
                    StatusCode(400),
                    json!({"error":"limit must be an integer between 1 and 200"}),
                )
            }
        },
        None => 48,
    };
    let before = match query_param(url, "before") {
        Some(value) => match value.parse::<i64>() {
            Ok(value) if value > 0 => Some(value),
            _ => {
                return json_response(
                    StatusCode(400),
                    json!({"error":"before must be a positive message ordinal"}),
                )
            }
        },
        None => None,
    };
    let expected_revision = match query_param(url, "expectedRevision") {
        Some(value) => match value.parse::<u64>() {
            Ok(value) => Some(value),
            _ => {
                return json_response(
                    StatusCode(400),
                    json!({"error":"expectedRevision must be a non-negative integer"}),
                )
            }
        },
        None => None,
    };
    let expected_event_sequence = match query_param(url, "expectedEventSequence") {
        Some(value) => match value.parse::<u64>() {
            Ok(value) => Some(value),
            _ => {
                return json_response(
                    StatusCode(400),
                    json!({"error":"expectedEventSequence must be a non-negative integer"}),
                )
            }
        },
        None => None,
    };
    match NolaStore::open_default().and_then(|store| {
        if store.get_web_conversation(conversation_id)?.is_none() {
            return Err("not_found".to_string());
        }
        let window = store.load_web_message_window(conversation_id, before, limit)?;
        let mut application = store.application()?;
        let can_branch_during_maintenance = match application.active_run(conversation_id)? {
            Some(run) if run.kind == "maintenance" && run.assistant_message_id.is_none() => {
                application.load_committed_host_requests(&run.run_id).is_ok_and(|effects| {
                    effects.requests.iter().any(|request| matches!(
                        request.payload, nola_core::agent::host::HostRequest::Compact { .. }
                    ))
                })
            }
            _ => false,
        };
        let assistant_journal_heads = local_browser_journal_heads(
            &mut application, window.assistant_journal_heads,
        );
        let messages = window
            .messages
            .into_iter()
            .map(|value| {
                serde_json::from_value::<WebMessage>(value)
                    .map_err(|err| format!("parse web message failed: {err}"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok((
            messages,
            window.total_count,
            window.before_ordinal,
            window.has_more,
            window.projection,
            window.projection_revision,
            window.event_sequence,
            assistant_journal_heads,
            can_branch_during_maintenance,
        ))
    }) {
        Ok((
            messages,
            total_count,
            before_ordinal,
            has_more,
            conversation_projection,
            projection_revision,
            event_sequence,
            assistant_journal_heads,
            can_branch_during_maintenance,
        )) => {
            if expected_revision.is_some_and(|expected| expected != projection_revision)
                || expected_event_sequence.is_some_and(|expected| expected != event_sequence)
            {
                return json_response(
                    StatusCode(409),
                    json!({
                        "error": "conversation snapshot changed; reload the message window",
                        "code": "stale_message_window",
                        "projectionRevision": projection_revision,
                        "eventSequence": event_sequence,
                    }),
                );
            }
            let task_projection = crate::session::load_session_task_projection(conversation_id)
                .ok()
                .flatten();
            let memory_runtime_projection =
                crate::session::load_session_memory_runtime_projection(conversation_id)
                    .ok()
                    .flatten();
            let permission_projection =
                crate::session::load_session_permission_projection(conversation_id)
                    .ok()
                    .flatten();
            let skill_projection =
                match crate::session::load_session_skill_projection(conversation_id) {
                    Ok(projection) => projection,
                    Err(error) => {
                        return json_response(StatusCode(500), json!({"error": error}));
                    }
                };
            json_response(
                StatusCode(200),
                json!({
                    "messages": messages,
                    "conversationProjection": conversation_projection,
                    "assistantJournalHeads": assistant_journal_heads,
                    "sourceHostId": host_identity(),
                    "messageWindow": {
                        "totalCount": total_count,
                        "before": before_ordinal,
                        "hasMore": has_more,
                        "canBranchDuringMaintenance": can_branch_during_maintenance,
                        "limit": limit,
                        "projectionRevision": projection_revision,
                        "eventSequence": event_sequence
                    },
                    "taskProjection": task_projection,
                    "memoryRuntimeProjection": memory_runtime_projection,
                    "skillProjection": skill_projection,
                    "permissionProjection": permission_projection,
                    "localFirst": true,
                    "backend": "cli-rust"
                }),
            )
        }
        Err(err) if err == "not_found" => not_found("会话不存在"),
        Err(err) => json_response(StatusCode(500), json!({"error": err})),
    }
}

fn conversation_traces_response(
    url: &str,
    conversation_id: &str,
) -> Response<Box<dyn Read + Send>> {
    let requested_trace_id = query_param(url, "trace_id")
        .or_else(|| query_param(url, "traceId"))
        .filter(|value| !value.trim().is_empty());
    let after_sequence = query_param(url, "after_sequence")
        .or_else(|| query_param(url, "afterSequence"))
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(0);
    let summary_only = query_param(url, "summaryOnly").as_deref() == Some("1");
    let store = match NolaStore::open_default() {
        Ok(store) => store,
        Err(err) => return json_response(StatusCode(500), json!({"error": err})),
    };
    match store.get_web_conversation(conversation_id) {
        Ok(Some(_)) => {}
        Ok(None) => return not_found("会话不存在"),
        Err(err) => return json_response(StatusCode(500), json!({"error": err})),
    }
    let runs = match store.trace_runs_for_session(conversation_id) {
        Ok(runs) => runs,
        Err(err) => return json_response(StatusCode(500), json!({"error": err})),
    };
    let mut traces = Vec::new();
    for run in runs {
        if requested_trace_id
            .as_ref()
            .is_some_and(|requested| requested != &run.trace_id)
        {
            continue;
        }
        let cursor = if requested_trace_id.is_some() {
            after_sequence
        } else {
            0
        };
        let events = if summary_only {
            Vec::new()
        } else {
            match store.trace_events_after(&run.trace_id, cursor) {
                Ok(events) => events,
                Err(err) => return json_response(StatusCode(500), json!({"error": err})),
            }
        };
        traces.push(json!({
            "traceId": run.trace_id,
            "runId": run.run_id,
            "messageId": run.message_id,
            "status": run.status,
            "lastSequence": run.last_sequence,
            "terminalReason": run.terminal_reason,
            "startedAt": run.started_at,
            "finishedAt": run.finished_at,
            "afterSequence": cursor,
            "nextAfterSequence": if summary_only { run.last_sequence } else { events.last().map(|event| event.sequence).unwrap_or(cursor) },
            "events": events,
        }));
    }
    if requested_trace_id.is_some() && traces.is_empty() {
        return not_found("Trace 不存在");
    }
    json_response(
        StatusCode(200),
        json!({
            "conversationId": conversation_id,
            "traces": traces,
            "localFirst": true,
            "backend": "cli-rust",
        }),
    )
    .with_header(Header::from_bytes(&b"Cache-Control"[..], &b"no-store"[..]).unwrap())
}

fn update_message_response(
    request: &mut Request,
    conversation_id: &str,
    message_id: &str,
) -> Response<Box<dyn Read + Send>> {
    if message_id.is_empty() || message_id.contains('/') {
        return not_found("消息不存在");
    }
    let input = match read_json::<MessageUpdateInput>(request) {
        Ok(input) => input,
        Err(response) => return response,
    };
    let store = match NolaStore::open_default() {
        Ok(store) => store,
        Err(err) => return json_response(StatusCode(500), json!({"error": err})),
    };
    let Some(_) = (match store.get_web_conversation(conversation_id) {
        Ok(value) => value,
        Err(err) => return json_response(StatusCode(500), json!({"error": err})),
    }) else {
        return not_found("会话不存在");
    };
    let mut messages = match store.load_web_messages_for_conversation(conversation_id) {
        Ok(values) => values
            .into_iter()
            .filter_map(|value| serde_json::from_value::<WebMessage>(value).ok())
            .collect::<Vec<_>>(),
        Err(err) => return json_response(StatusCode(500), json!({"error": err})),
    };
    let Some(message) = messages
        .iter_mut()
        .find(|item| item.conversation_id == conversation_id && item.message_id == message_id)
    else {
        return not_found("消息不存在");
    };
    if let Some(content) = input.content {
        let content = content.trim().to_string();
        if content.is_empty() {
            return json_response(StatusCode(400), json!({"error": "消息内容不能为空"}));
        }
        message.content = content;
    }
    if let Some(include) = input.include_in_context {
        message.include_in_context = include;
    }
    if let Some(meta) = input.meta {
        message.meta = sanitize_meta(meta);
    }
    let updated_message = message.clone();
    if let Err(err) = store.update_web_message(&json!(updated_message)) {
        return json_response(StatusCode(500), json!({"error": err}));
    }
    json_response(StatusCode(200), with_local_first(json!(updated_message)))
}

fn delete_message_response(
    conversation_id: &str,
    message_id: &str,
) -> Response<Box<dyn Read + Send>> {
    if message_id.is_empty() || message_id.contains('/') {
        return not_found("消息不存在");
    }
    let store = match NolaStore::open_default() {
        Ok(store) => store,
        Err(err) => return json_response(StatusCode(500), json!({"error": err})),
    };
    let Some(_) = (match store.get_web_conversation(conversation_id) {
        Ok(value) => value,
        Err(err) => return json_response(StatusCode(500), json!({"error": err})),
    }) else {
        return not_found("会话不存在");
    };
    match store.delete_web_message(conversation_id, message_id) {
        Ok(false) => return not_found("消息不存在"),
        Ok(true) => {}
        Err(err) => return json_response(StatusCode(500), json!({"error": err})),
    }
    json_response(StatusCode(200), json!({"ok": true, "localFirst": true}))
}

#[cfg(test)]
fn sorted_conversation_messages(state: &WebState, conversation_id: &str) -> Vec<WebMessage> {
    let mut messages = state
        .messages
        .iter()
        .filter(|item| item.conversation_id == conversation_id)
        .cloned()
        .collect::<Vec<_>>();
    messages.sort_by(|a, b| {
        a.created_at
            .cmp(&b.created_at)
            .then_with(|| a.message_id.cmp(&b.message_id))
    });
    messages
}

fn load_store_conversation_messages(
    store: &NolaStore,
    conversation_id: &str,
) -> Result<(WebConversation, Vec<WebMessage>), Response<Box<dyn Read + Send>>> {
    let Some(conversation_value) = store
        .get_web_conversation(conversation_id)
        .map_err(|err| json_response(StatusCode(500), json!({"error": err})))?
    else {
        return Err(not_found("会话不存在"));
    };
    let conversation = serde_json::from_value::<WebConversation>(conversation_value)
        .map_err(|err| json_response(StatusCode(500), json!({"error": err.to_string()})))?;
    let messages = store
        .load_web_messages_for_conversation(conversation_id)
        .map_err(|err| json_response(StatusCode(500), json!({"error": err})))?
        .into_iter()
        .filter_map(|value| serde_json::from_value::<WebMessage>(value).ok())
        .collect::<Vec<_>>();
    Ok((conversation, messages))
}

fn truncate_conversation_response(
    config: &Config,
    request: &mut Request,
    conversation_id: &str,
) -> Response<Box<dyn Read + Send>> {
    let input = match read_json::<TruncateInput>(request) {
        Ok(input) => input,
        Err(response) => return response,
    };
    let target_id = input.message_id.trim();
    if target_id.is_empty() {
        return json_response(StatusCode(400), json!({"error": "messageId is required"}));
    }
    let keep_message = input.keep_message.unwrap_or(true);
    let store = match NolaStore::open_default() {
        Ok(store) => store,
        Err(err) => return json_response(StatusCode(500), json!({"error": err})),
    };
    let (_conversation, messages) = match load_store_conversation_messages(&store, conversation_id)
    {
        Ok(value) => value,
        Err(response) => return response,
    };
    let Some(target_index) = messages
        .iter()
        .position(|item| item.message_id == target_id)
    else {
        return not_found("消息不存在");
    };
    let through_core_message_id = messages[target_index].message_id.clone();
    let keep_count = if keep_message {
        target_index + 1
    } else {
        target_index
    };
    let original_len = messages.len();
    let kept = messages
        .iter()
        .take(keep_count)
        .cloned()
        .collect::<Vec<_>>();
    let removed = original_len.saturating_sub(kept.len());
    if let Err(error) = config.host_invocation_request().and_then(|request| crate::core_agent_runtime::revert_agent_core_conversation(
        &request,
        conversation_id,
        &through_core_message_id,
        keep_message,
    )) {
        return json_response(
            StatusCode(409),
            json!({
                "error": error,
                "code": "revert_checkpoint_rejected"
            }),
        );
    }
    json_response(
        StatusCode(200),
        json!({
            "ok": true,
            "removed": removed,
            "messages": kept,
            "localFirst": true,
            "backend": "cli-rust",
        }),
    )
}

fn branch_conversation_response(
    request: &mut Request,
    conversation_id: &str,
) -> Response<Box<dyn Read + Send>> {
    let input = match read_json::<BranchInput>(request) {
        Ok(input) => input,
        Err(response) => return response,
    };
    let store = match NolaStore::open_default() {
        Ok(store) => store,
        Err(err) => return json_response(StatusCode(500), json!({"error": err})),
    };
    let source: WebConversation = match store.get_web_conversation(conversation_id) {
        Ok(Some(value)) => match serde_json::from_value(value) {
            Ok(value) => value,
            Err(error) => {
                return json_response(StatusCode(500), json!({"error": error.to_string()}))
            }
        },
        Ok(None) => return not_found("消息不存在"),
        Err(error) => return json_response(StatusCode(500), json!({"error": error})),
    };
    let through_message_id = input
        .message_id
        .as_deref()
        .map(str::trim)
        .unwrap_or_default();
    let now = now_string();
    let mut branch = source.clone();
    branch.conversation_id = new_id("conv");
    let fallback_title = format!("{} copy", source.title);
    branch.title = normalize_title(input.title.as_deref().or(Some(fallback_title.as_str())));
    branch.title_source = ConversationTitleSource::User;
    branch.created_at = now.clone();
    branch.updated_at = now;
    let result = match store.create_web_conversation_branch(
        conversation_id,
        through_message_id,
        &json!(branch),
    ) {
        Ok(result) => result,
        Err(error) => {
            return json_response(
                StatusCode(409),
                json!({
                    "error": error,
                    "code": "branch_commit_rejected"
                }),
            )
        }
    };
    let saved_branch = match store.get_web_conversation(&branch.conversation_id) {
        Ok(Some(saved)) => saved,
        Ok(None) => {
            return json_response(
                StatusCode(500),
                json!({"error": "Branched conversation disappeared"}),
            )
        }
        Err(error) => return json_response(StatusCode(500), json!({"error": error})),
    };
    json_response(
        StatusCode(200),
        json!({
            "conversation": saved_branch,
            "messageCount": result.message_count,
            "localFirst": true,
            "backend": "cli-rust",
        }),
    )
}

fn web_core_message_id(message: &WebMessage) -> Option<String> {
    message
        .meta
        .get("coreMessageId")
        .or_else(|| message.meta.get("core_message_id"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .or_else(|| {
            message
                .meta
                .get("agentTrace")
                .and_then(Value::as_object)
                .and_then(|trace| trace.get("messageId"))
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .map(str::to_string)
        })
        .or_else(|| {
            (message.role == "user" && !message.message_id.trim().is_empty())
                .then(|| message.message_id.clone())
        })
}

fn export_conversation_response(
    url: &str,
    conversation_id: &str,
) -> Response<Box<dyn Read + Send>> {
    let format = query_param(url, "format")
        .unwrap_or_else(|| "markdown".to_string())
        .to_ascii_lowercase();
    let store = match NolaStore::open_default() {
        Ok(store) => store,
        Err(err) => return json_response(StatusCode(500), json!({"error": err})),
    };
    let (conversation, messages) = match load_store_conversation_messages(&store, conversation_id) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let filename_base = safe_filename_part(&conversation.title);
    if format == "json" {
        let canonical = match store.load_canonical_archive(conversation_id) {
            Ok(Some(canonical)) => canonical,
            Ok(None) => {
                return json_response(
                    StatusCode(404),
                    json!({"error": "Conversation is unavailable"}),
                )
            }
            Err(error) => return json_response(StatusCode(500), json!({"error": error})),
        };
        let content = match serde_json::to_string_pretty(&json!({
            "conversation": conversation,
            "messages": messages,
            "canonical": canonical,
        })) {
            Ok(content) => content,
            Err(error) => {
                return json_response(StatusCode(500), json!({"error": error.to_string()}))
            }
        };
        return json_response(
            StatusCode(200),
            json!({
                "filename": format!("{filename_base}.json"),
                "mimeType": "application/json;charset=utf-8",
                "content": content,
                "localFirst": true,
            }),
        );
    }
    let mut content = format!(
        "# {}\n\n- Exported: {}\n- Conversation ID: {}\n\n",
        conversation.title,
        now_string(),
        conversation.conversation_id
    );
    for message in messages {
        let role = match message.role.as_str() {
            "assistant" => "Nola",
            "user" => "User",
            other => other,
        };
        content.push_str(&format!("## {role}\n\n{}\n\n", message.content.trim()));
    }
    json_response(
        StatusCode(200),
        json!({
            "filename": format!("{filename_base}.md"),
            "mimeType": "text/markdown;charset=utf-8",
            "content": content,
            "localFirst": true,
        }),
    )
}

fn context_report_response(
    conversation_id: &str,
    config: &Config,
) -> Response<Box<dyn Read + Send>> {
    let store = match NolaStore::open_default() {
        Ok(store) => store,
        Err(err) => return json_response(StatusCode(500), json!({"error": err})),
    };
    let (conversation, messages) = match load_store_conversation_messages(&store, conversation_id) {
        Ok(value) => value,
        Err(response) => return response,
    };
    with_state_read(|state| {
        let settings = match load_agent_settings_from_store(&store) {
            Ok(settings) => settings,
            Err(error) => {
                return json_response(
                    StatusCode(500),
                    json!({"error":error,"code":"agent_settings_snapshot_failed"}),
                )
            }
        };
        let skills = match web_skill_runtime_for_settings(config.skill_dirs.as_deref(), &settings) {
            Ok(skills) => skills,
            Err(error) => {
                return json_response(
                    StatusCode(502),
                    json!({"error":error,"code":"skill_catalog_unavailable"}),
                )
            }
        };
        let report_body = json!({
            "conversationId": conversation_id,
            "contextReportOnly": true,
        });
        let core_messages = agent_context_messages_from_request(&report_body, &settings);
        let core_components =
            match web_context_components_from_request_with_skills(&report_body, &settings, &skills)
            {
                Ok(components) => components,
                Err(error) => {
                    return json_response(
                        StatusCode(502),
                        json!({"error": error, "code": "context_source_unavailable"}),
                    )
                }
            };
        let report_tools =
            match resolve_web_agent_host_tools(&settings, &state.permission_rules, &skills) {
                Ok(tools) => crate::agent_host_driver::initial_model_tool_schemas(tools),
                Err(error) => {
                    return json_response(
                        StatusCode(502),
                        json!({"error":error,"code":"skill_catalog_unavailable"}),
                    )
                }
            };
        let uploaded_skills_tokens = core_components
            .iter()
            .filter(|component| component.kind == ContextComponentKind::Skill)
            .filter_map(|component| component.content.as_str())
            .map(estimate_tokens)
            .sum::<usize>();
        let core_prepared = prepare_context(&ContextPrepareInput {
            schema_version: CONTEXT_ENGINE_SCHEMA_VERSION,
            mode: ContextPrepareMode::Normal,
            platform: ContextPlatform::Web,
            components: core_components,
            messages: core_messages.clone(),
            tools: report_tools,
            budget: ContextBudgetSpec {
                context_window_tokens: Some(
                    usize::try_from(settings.max_context_tokens).unwrap_or(usize::MAX),
                ),
                reserved_output_tokens: usize::try_from(settings.max_output_tokens)
                    .unwrap_or(usize::MAX),
                keep_recent_messages: 8,
                ..ContextBudgetSpec::default()
            },
            existing_summary: non_empty_string(conversation.session_memory.trim())
                .map(ToOwned::to_owned),
            summary_result: None,
            evaluated_at: None,
        });
        let core_component_messages = core_prepared
            .messages
            .iter()
            .filter(|message| {
                message
                    .get("id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| id.starts_with("nola-context-component-"))
            })
            .cloned()
            .collect::<Vec<_>>();
        let core_tool_result_messages = core_prepared
            .messages
            .iter()
            .filter(|message| message.get("role").and_then(Value::as_str) == Some("tool"))
            .cloned()
            .collect::<Vec<_>>();
        let core_component_tokens = estimate_json_values_tokens(&core_component_messages);
        let core_tool_result_tokens = estimate_json_values_tokens(&core_tool_result_messages);
        let core_conversation_tokens = core_prepared
            .metrics
            .message_tokens
            .saturating_sub(core_component_tokens)
            .saturating_sub(core_tool_result_tokens);
        let core_component_metrics = serde_json::to_value(&core_prepared.component_metrics)
            .unwrap_or_else(|_| Value::Array(Vec::new()));
        let message_tokens = estimate_json_values_tokens(&core_messages);
        let session_memory_tokens = estimate_tokens(&conversation.session_memory);
        let long_term_memory_tokens = estimate_tokens(&settings.long_term_memory);
        let user_preferences_tokens = estimate_tokens(&settings.user_preferences);
        let mcp_context_resources =
            crate::resource_access::mcp_model_context_resources(&settings.mcp_servers);
        let mcp_servers_tokens =
            estimate_tokens(&serde_json::to_string(&mcp_context_resources).unwrap_or_default());
        let document_context_resources =
            crate::resource_access::data_source_model_context_resources(
                &settings.uploaded_documents,
            );
        let uploaded_documents_tokens = enabled_items_text_tokens(&document_context_resources);
        let mut data_sources =
            context_data_sources(&document_context_resources, &mcp_context_resources);
        data_sources.extend(persisted_data_source_values(conversation_id));
        let data_source_count = data_sources.len();
        let context_pack = context_pack_value(
            conversation_id,
            &data_sources,
            &document_context_resources,
            &mcp_context_resources,
        );
        let persisted_data_source_tokens = data_sources
            .iter()
            .filter_map(|source| {
                source
                    .get("metadata")
                    .and_then(|metadata| metadata.get("tokenEstimate"))
                    .and_then(Value::as_u64)
            })
            .map(|value| usize::try_from(value).unwrap_or(usize::MAX))
            .sum::<usize>();
        let data_sources_tokens = mcp_servers_tokens
            .saturating_add(uploaded_documents_tokens)
            .saturating_add(persisted_data_source_tokens);
        let runtime = conversation.last_context_usage.as_ref();
        let runtime_number = |keys: &[&str]| -> Option<usize> {
            let value = runtime?;
            keys.iter().find_map(|key| {
                value
                    .get(*key)
                    .and_then(Value::as_u64)
                    .map(|number| usize::try_from(number).unwrap_or(usize::MAX))
            })
        };
        let has_recorded_usage = runtime_number(&["used_tokens", "usedTokens"]).is_some()
            && runtime_number(&[
                "context_window_tokens",
                "contextWindowTokens",
                "limit_tokens",
                "limitTokens",
            ])
            .is_some_and(|window| window > 0);
        let configured_window = usize::try_from(settings.max_context_tokens).unwrap_or(usize::MAX);
        let context_window = runtime_number(&[
            "context_window_tokens",
            "contextWindowTokens",
            "limit_tokens",
            "limitTokens",
        ])
        .filter(|window| *window > 0)
        .unwrap_or(configured_window);
        let prompt_limit = runtime_number(&["prompt_token_limit", "promptTokenLimit"])
            .unwrap_or(core_prepared.metrics.prompt_token_limit)
            .max(1);
        let reserved_output_tokens =
            runtime_number(&["reserved_output_tokens", "reservedOutputTokens"])
                .unwrap_or(core_prepared.metrics.reserved_output_tokens);
        let total = runtime_number(&["used_tokens", "usedTokens"])
            .unwrap_or(core_prepared.metrics.total_prompt_tokens);
        let runtime_message_tokens = runtime_number(&["message_tokens", "messageTokens"])
            .unwrap_or(core_prepared.metrics.message_tokens);
        let tool_schema_tokens = runtime_number(&["tool_schema_tokens", "toolSchemaTokens"])
            .unwrap_or(core_prepared.metrics.tool_schema_tokens);
        let component_tokens = runtime_number(&["component_tokens", "componentTokens"])
            .unwrap_or(core_component_tokens);
        let tool_result_tokens = runtime_number(&["tool_result_tokens", "toolResultTokens"])
            .unwrap_or(core_tool_result_tokens);
        let runtime_conversation_tokens =
            runtime_number(&["conversation_tokens", "conversationTokens"])
                .unwrap_or(core_conversation_tokens);
        let component_metrics = runtime
            .and_then(|value| {
                value
                    .get("component_metrics")
                    .or_else(|| value.get("componentMetrics"))
            })
            .cloned()
            .unwrap_or(core_component_metrics);
        let usage_basis = runtime
            .and_then(|value| value.get("usage_basis").or_else(|| value.get("usageBasis")))
            .and_then(Value::as_str)
            .unwrap_or("current_conversation");
        let agent_scaffold_tokens = runtime_conversation_tokens.saturating_sub(message_tokens);
        let percentage = total
            .saturating_mul(100)
            .checked_div(context_window)
            .unwrap_or(0);
        let suggestion_details = context_suggestion_details(
            percentage,
            total,
            context_window,
            message_tokens,
            session_memory_tokens,
            long_term_memory_tokens,
            user_preferences_tokens,
            mcp_servers_tokens,
            uploaded_skills_tokens,
            uploaded_documents_tokens,
            data_sources_tokens,
            agent_scaffold_tokens,
            messages
                .iter()
                .filter(|item| item.include_in_context)
                .count(),
            messages
                .iter()
                .filter(|item| !item.include_in_context)
                .count(),
        );
        let mut response = json!({
            "conversationId": conversation_id,
            "sessionMemory": conversation.session_memory,
            "knowledgePoints": conversation.knowledge_points,
            "wiki": conversation.wiki,
            "lastCompactedAt": conversation.last_compacted_at,
            "longTermMemory": settings.long_term_memory,
            "userPreferences": settings.user_preferences,
            "mcpServers": settings.mcp_servers.iter()
                .filter_map(crate::resource_access::mcp_api_projection)
                .collect::<Vec<_>>(),
            "skills": builtin_skills::merged_catalog(
                &settings.builtin_skill_overrides,
                &settings.uploaded_skills,
            ).unwrap_or_else(|_| settings.uploaded_skills.clone()),
            "uploadedSkills": builtin_skills::merged_catalog(
                &settings.builtin_skill_overrides,
                &settings.uploaded_skills,
            ).unwrap_or_else(|_| settings.uploaded_skills.clone()),
            "uploadedDocuments": crate::resource_access::data_source_api_projections(
                &settings.uploaded_documents,
            ),
            "dataSources": data_sources,
            "contextPack": context_pack,
            "messageCount": messages.iter().filter(|item| item.include_in_context).count(),
            "archivedMessageCount": messages.iter().filter(|item| !item.include_in_context).count(),
            "totalMessageCount": messages.len(),
            "rawMemoryEntryCount": 0,
            "messageTokens": message_tokens,
            "sessionMemoryTokens": session_memory_tokens,
            "longTermMemoryTokens": long_term_memory_tokens,
            "userPreferencesTokens": user_preferences_tokens,
            "mcpServersTokens": mcp_servers_tokens,
            "uploadedSkillsTokens": uploaded_skills_tokens,
            "uploadedDocumentsTokens": uploaded_documents_tokens,
            "dataSourcesTokens": data_sources_tokens,
            "dataSourceCount": data_source_count,
            "agentScaffoldTokens": agent_scaffold_tokens,
            "rawMemorySummaryTokens": 0,
            "totalTokens": total,
            "contextWindowTokens": context_window,
            "promptTokenLimit": prompt_limit,
            "reservedOutputTokens": reserved_output_tokens,
            "runtimeMessageTokens": runtime_message_tokens,
            "toolSchemaTokens": tool_schema_tokens,
            "estimatedTokenLimit": prompt_limit,
            "percentage": percentage.min(100),
            "suggestionDetails": suggestion_details,
            "preparation": {
                "action": core_prepared.action,
                "reason": core_prepared.reason,
            },
            "localFirst": true,
            "backend": "cli-rust",
        });
        response["componentTokens"] = Value::from(component_tokens);
        response["toolResultTokens"] = Value::from(tool_result_tokens);
        response["runtimeConversationTokens"] = Value::from(runtime_conversation_tokens);
        response["componentMetrics"] = component_metrics;
        response["usageBasis"] = Value::String(usage_basis.to_string());
        response["usageSource"] = Value::String(
            if has_recorded_usage {
                "last_run"
            } else {
                "estimated"
            }
            .to_string(),
        );
        json_response(StatusCode(200), response)
    })
}

fn conversation_recall_response(
    url: &str,
    conversation_id: &str,
    search: impl FnOnce(&str, &str, usize) -> Result<Vec<nola_protocol::ConversationRecallHit>, String>,
) -> Response<Box<dyn Read + Send>> {
    let query = query_param(url, "q").unwrap_or_default().trim().to_string();
    let command = nola_protocol::ConversationRecallCommand::SearchConversationText {
        conversation_id: conversation_id.into(), query: query.clone(), limit: 20,
    };
    if let Err(error) = command.validate() {
        return json_response(StatusCode(400), json!({"error": error}));
    }
    match search(conversation_id, &query, 20) {
        Ok(hits) => json_response(StatusCode(200), json!({
            "conversationId": conversation_id, "query": query, "limit": 20, "hits": hits,
        })),
        Err(_) => json_response(StatusCode(500), json!({"error": "Conversation recall unavailable"})),
    }
}

#[cfg(test)]
mod conversation_recall_web_tests {
    use super::*;

    #[test]
    fn recall_rejects_invalid_queries_before_reading_and_binds_response_owner() {
        for url in ["/recall", "/recall?q=%20"] {
            let response = conversation_recall_response(url, "one", |_, _, _| panic!("invalid search"));
            assert_eq!(response.status_code(), StatusCode(400));
        }
        let response = conversation_recall_response("/recall?q=%E9%A2%84%E7%AE%97%20CODE", "one", |id, query, limit| {
            assert_eq!(id, "one"); assert_eq!(query, "预算 CODE"); assert_eq!(limit, 20);
            Ok(vec![])
        });
        assert_eq!(response.status_code(), StatusCode(200));
        let mut body = String::new(); response.into_reader().read_to_string(&mut body).unwrap();
        let value: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(value["conversationId"], "one"); assert_eq!(value["query"], "预算 CODE");
        assert_eq!(value["hits"], json!([]));
    }

    #[test]
    fn recall_failure_does_not_expose_host_paths_or_fabricate_empty_success() {
        let response = conversation_recall_response("/recall?q=test", "one", |_, _, _| Err("private-host-path".into()));
        assert_eq!(response.status_code(), StatusCode(500));
        let mut body = String::new(); response.into_reader().read_to_string(&mut body).unwrap();
        assert!(!body.contains("private-host-path"));
    }
}

fn update_session_memory_response(request:&mut Request,conversation_id:&str)->Response<Box<dyn Read+Send>> {
    let input=match read_json::<SessionMemoryInput>(request) {Ok(value)=>value,Err(response)=>return response};
    let store=match NolaStore::open_default() {Ok(value)=>value,Err(error)=>return json_response(StatusCode(500),json!({"error":error}))};
    let changes_knowledge=input.knowledge_points.is_some() || input.wiki.is_some();
    if input.session_memory.is_some() && changes_knowledge {return json_response(StatusCode(422),json!({"code":"independent_context_and_knowledge_commands_required"}));}
    if let Some(summary)=input.session_memory {
        let Some(source)=input.expected_context_source.filter(|source|source.conversation_id==conversation_id) else {return json_response(StatusCode(400),json!({"code":"context_source_required"}));};
        if let Err(error)=store.replace_web_context_summary(source,summary) {return json_response(StatusCode(409),json!({"code":"context_commit_conflict","error":error}));}
    } else if changes_knowledge {
        let Some(expected)=input.expected_configuration_revision.filter(|value|*value>0) else {return json_response(StatusCode(400),json!({"code":"configuration_revision_required"}));};
        let mut fields=serde_json::Map::new();
        if let Some(value)=input.knowledge_points {fields.insert("keyPointsMarkdown".into(),json!(value));}
        if let Some(value)=input.wiki {fields.insert("wikiMarkdown".into(),json!(value));}
        use nola_domain::conversation_configuration::ConfigurationCommitResult;
        match store.patch_web_conversation_configuration(conversation_id,expected,json!({"knowledge":fields})) {
            Ok(ConfigurationCommitResult::Committed {..})=>{},
            Ok(conflict@ConfigurationCommitResult::Conflict {..})=>return json_response(StatusCode(409),json!(conflict)),
            Ok(missing@ConfigurationCommitResult::NotFound {..})=>return json_response(StatusCode(404),json!(missing)),
            Err(error)=>return json_response(StatusCode(422),json!({"error":error})),
        }
    } else {return json_response(StatusCode(422),json!({"code":"empty_context_or_knowledge_patch"}));}
    match store.web_session_memory_edit_state(conversation_id) {Ok(state)=>json_response(StatusCode(200),state),Err(error)=>json_response(StatusCode(409),json!({"code":"context_read_conflict","error":error}))}
}

fn conversation_plan_response(conversation_id: &str) -> Response<Box<dyn Read + Send>> {
    let unavailable = || json_response(StatusCode(503), json!({"code": "conversation_plan_unavailable", "error": "Conversation plan is unavailable."}));
    let store = match NolaStore::open_default() {
        Ok(store) => store,
        Err(_) => return unavailable(),
    };
    match store.get_web_conversation(conversation_id) {
        Ok(Some(_)) => {}
        Ok(None) => return json_response(StatusCode(404), json!({"code": "conversation_not_found"})),
        Err(_) => return unavailable(),
    }
    match store.session_plan_record(conversation_id) {
        Ok(plan) => match conversation_plan_payload(conversation_id, plan) {
            Some(value) => json_response(StatusCode(200), value),
            None => unavailable(),
        },
        Err(_) => unavailable(),
    }
}

fn conversation_plan_payload(conversation_id: &str, plan: Option<nola_application::ConversationPlan>) -> Option<serde_json::Value> {
    if plan.as_ref().is_some_and(|plan| plan.conversation_id != conversation_id) {
        return None;
    }
    Some(json!({"conversationId": conversation_id, "authoritative": true, "plan": plan}))
}

#[cfg(test)]
mod conversation_plan_read_tests {
    use super::*;

    #[test]
    fn conversation_plan_read_keeps_owner_and_authoritative_empty_state() {
        assert_eq!(conversation_plan_payload("a", None), Some(json!({"conversationId":"a","authoritative":true,"plan":null})));
        let plan: nola_application::ConversationPlan = serde_json::from_value(json!({
            "conversation_id":"a", "revision":2, "updated_at_ms":0,
            "sop":{"runId":"run","sopId":"sop","sopVersion":1}, "todos":[]
        })).unwrap();
        assert!(conversation_plan_payload("other", Some(plan.clone())).is_none());
        let value = conversation_plan_payload("a", Some(plan)).unwrap();
        assert_eq!(value["plan"]["revision"], 2);
        assert_eq!(value["plan"]["todos"], json!([]));
    }
}

fn goal_projection_response(projection: Option<&nola_core::agent::GoalProjection>) -> Value {
    projection.map(|projection| {
        let mut value = json!(projection);
        value.as_object_mut().expect("Goal projection object")
            .insert("control_token".into(), json!(projection.control_token()));
        value.as_object_mut().expect("Goal projection object")
            .insert("can_submit".into(), json!(projection.control_observation().map(|value| value.can_submit)));
        let controls = projection.control_observation();
        let object = value.as_object_mut().expect("Goal projection object");
        object.insert("can_pause".into(), json!(controls.as_ref().is_some_and(|value| value.can_pause)));
        object.insert("can_resume".into(), json!(controls.as_ref().is_some_and(|value| value.can_resume)));
        object.insert("can_cancel".into(), json!(controls.as_ref().is_some_and(|value| value.can_cancel)));
        value
    }).unwrap_or(Value::Null)
}

fn conversation_goal_response(conversation_id: &str) -> Response<Box<dyn Read + Send>> {
    let store = match NolaStore::open_default() {
        Ok(store) => store,
        Err(error) => return json_response(StatusCode(500), json!({"error": error})),
    };
    match store.get_web_conversation(conversation_id) {
        Ok(Some(_)) => {}
        Ok(None) => {
            return json_response(StatusCode(404), json!({"error": "conversation not found"}));
        }
        Err(error) => return json_response(StatusCode(500), json!({"error": error})),
    }
    match crate::session::load_session_goal_projection(conversation_id) {
        Ok(projection) => {
            let value = goal_projection_response(projection.as_ref());
            json_response(StatusCode(200), json!({
                "conversationId": conversation_id,
                "goalProjection": value,
                "source_host_id": host_identity(),
                "authoritative": true,
            }))
        },
        Err(error) => json_response(StatusCode(400), json!({"error": error})),
    }
}

fn control_conversation_goal_response(
    request: &mut Request,
    conversation_id: &str,
    config: &Config,
) -> Response<Box<dyn Read + Send>> {
    let input = match read_json::<GoalControlInput>(request) {
        Ok(input) => input,
        Err(response) => return response,
    };
    if input.expected_host_id.as_deref() != Some(host_identity()) {
        return json_response(StatusCode(409), json!({"error":"Goal control Host identity changed"}));
    }
    let (status, mut value) = dispatch_conversation_goal_control(conversation_id, input, config);
    value.as_object_mut().expect("Goal control response object")
        .insert("source_host_id".into(), json!(host_identity()));
    json_response(status, value)
}

fn goal_control_result(status: StatusCode, value: Value) -> (StatusCode, Value) { (status, value) }

// Browser and paired native transports call this same Host adapter. Core owns
// guarded transitions, durable control admission, receipts and Run cancellation.
fn dispatch_conversation_goal_control(
    conversation_id: &str,
    input: GoalControlInput,
    config: &Config,
) -> (StatusCode, Value) {
    let mut application = match crate::application_port::CliApplicationPort::persistent_default() {
        Ok(application) => application,
        Err(error) => return goal_control_result(StatusCode(500), json!({"error":error})),
    };
    match application.execute::<Value>(nola_application::ApplicationCommand::LoadConversationMetadata {
        conversation_id: conversation_id.to_owned(),
    }) {
        Ok(value) if !value.is_null() => {},
        Ok(_) => return goal_control_result(StatusCode(404), json!({"error":"conversation not found"})),
        Err(error) => return goal_control_result(StatusCode(500), json!({"error":error})),
    }
    use nola_application::goal_control_intent::{GoalControlAction, PrepareGoalControlIntent};
    let action = match input.action.as_str() {
        "start" => GoalControlAction::Start,
        "pause" => GoalControlAction::Pause,
        "resume" => GoalControlAction::Resume,
        "cancel" => GoalControlAction::Cancel,
        "schedule" => GoalControlAction::Schedule,
        _ => return goal_control_result(StatusCode(400), json!({"error":"unsupported Goal action"})),
    };
    let expected_goal_id = input.expected_goal_id.clone();
    let envelope = match application.prepare_goal_control_intent(PrepareGoalControlIntent {
        conversation_id: conversation_id.into(), operation_id: input.operation_id,
        action, expected_goal_id: input.expected_goal_id,
        expected_control_token: input.expected_control_token,
        objective: input.objective, constraints: input.constraints,
        wake_at_epoch_ms: input.wake_at_epoch_ms,
    }) {
        Ok(envelope) => envelope,
        Err(error) => return goal_control_result(StatusCode(409), json!({"error":error})),
    };
    // A lost acknowledgement is resolved against the same durable operation;
    // do not start a replacement Goal or reinterpret the old control token.
    let projection = match crate::session::load_session_goal_projection(conversation_id) {
        Ok(projection) => projection,
        Err(error) => return goal_control_result(StatusCode(500), json!({"error":error})),
    };
    if let Some(projection) = projection {
        if let Some(operation) = projection.applied_operations.get(&envelope.operation_id) {
            let expected_digest = match nola_core::agent::goal::goal_command_digest(&envelope.command) {
                Ok(digest) => digest,
                Err(error) => return goal_control_result(StatusCode(500), json!({"error":error})),
            };
            if operation.command_sha256 != expected_digest {
                return goal_control_result(StatusCode(409), json!({"error":"Goal operation identity belongs to a different command"}));
            }
            return goal_control_result(if operation.rejection.is_some() { StatusCode(409) } else { StatusCode(200) },
                json!({"conversationId":conversation_id,"operationId":envelope.operation_id,
                    "goalProjection":goal_projection_response(Some(&projection)),
                    "error":operation.rejection,"authoritative":true}));
        }
        if input.action == "start" && projection.active.as_ref().is_some_and(|goal| !goal.status.is_terminal()) {
            return goal_control_result(StatusCode(409), json!({"error":"an unfinished Goal is already active"}));
        }
    }
    let operation_id = envelope.operation_id.clone();
    if input.action.trim() != "start" {
        match enqueue_web_goal_control(conversation_id, envelope.clone()) {
            Ok(true) => return goal_control_result(StatusCode(202), json!({
                "conversationId": conversation_id, "controlPending": true,
                "operationId": envelope.operation_id,
                "goalProjection": goal_projection_response(crate::session::load_session_goal_projection(conversation_id).ok().flatten().as_ref()),
                "authoritative": false,
            })),
            Ok(false) => {},
            Err(error) => return goal_control_result(StatusCode(409), json!({"error":error})),
        }
    }
    // Starting execution requires a model. Controlling an admitted Goal does
    // not: revoked credentials must never prevent pause/cancel.
    let result = if input.action.trim() == "start" {
        new_web_goal_invocation(config, conversation_id)
            .and_then(|request| submit_agent_core_goal_command(&request, envelope))
            .map(|_| ())
    } else if input.action.trim() == "resume" {
        expected_goal_id.as_deref().ok_or_else(|| "Goal resume requires the observed Goal identity".to_owned())
            .and_then(|goal_id| web_goal_invocation_for_selection(config, conversation_id, Some(goal_id)))
            .and_then(|request| crate::core_agent_runtime::rebind_goal_execution(&request))
            .and_then(|_| apply_idle_web_goal_control(conversation_id, envelope))
    } else {
        apply_idle_web_goal_control(conversation_id, envelope)
    };
    match result {
        Ok(_) => {
            wake_web_goal_worker();
            match crate::session::load_session_goal_projection(conversation_id) {
            Ok(projection) => {
                let rejection = projection.as_ref().and_then(|p| p.applied_operations.get(&operation_id)).and_then(|operation| operation.rejection.as_ref());
                goal_control_result(if rejection.is_some() { StatusCode(409) } else { StatusCode(200) }, json!({
                    "conversationId": conversation_id, "goalProjection": goal_projection_response(projection.as_ref()),
                    "operationId": operation_id, "error": rejection, "authoritative": true,
                }))
            },
            Err(error) => goal_control_result(StatusCode(500), json!({"error": error})),
            }
        },
        Err(error) if error == GOAL_HOST_DEFAULT_UNAVAILABLE => goal_control_result(StatusCode(409), json!({
            "code": "goal_host_default_unavailable", "conversationId": conversation_id, "operationId": operation_id, "error": error
        })),
        Err(error) => goal_control_result(StatusCode(409), json!({"error": error})),
    }
}

// Maintenance retains the conversation's tool catalog. Tool execution is
// prohibited by the compaction protocol, not by changing the cached prefix.
fn conversation_execution_config(config: &Config, resolved: ResolvedLlmSettings, conversation_id: &str) -> Config {
    let mut agent_config = config.clone();
    agent_config.runtime = resolved.runtime;
    agent_config.runtime.selected_profile = resolved.selected_profile;
    agent_config.endpoint = resolved.endpoint;
    agent_config.api_key = resolved.api_key;
    agent_config.model = resolved.model;
    agent_config.runtime.temperature = Some(resolved.temperature);
    agent_config.runtime.top_p = Some(resolved.top_p);
    agent_config.runtime.max_output_tokens = Some(resolved.max_output_tokens);
    agent_config.runtime.compaction.context_window_tokens =
        Some(usize::try_from(resolved.max_context_tokens).unwrap_or(usize::MAX));
    agent_config.max_iterations = 0;
    agent_config.json_output = true;
    agent_config.session_id = Some(conversation_id.to_string());
    agent_config.continue_last = false;
    agent_config.save_session = true;
    agent_config
}

fn execute_conversation_compaction(
    config: &Config,
    resolved: ResolvedLlmSettings,
    conversation_id: &str,
    initial_items: Vec<nola_core::agent::protocol::AgentItem>,
) -> Result<crate::core_agent_runtime::CoreManualCompactionResult, String> {
    let agent_config = conversation_execution_config(config, resolved, conversation_id);
    compact_agent_core_session(&agent_config.host_invocation_request()?, initial_items)
}

fn compaction_execution_error(error: String) -> Response<Box<dyn Read + Send>> {
    // A String error does not establish whether a maintenance commit happened.
    // Do not advertise a context-limit error or cautious automatic resubmission.
    json_response(StatusCode(500), json!({"error": error, "outcome_unknown": true}))
}

#[cfg(test)]
mod compaction_execution_contract_tests {
    use super::*;

    #[test]
    fn maintenance_preserves_enabled_and_disabled_tool_catalog_configuration() {
        let mut config = Config::for_test().unwrap();
        for (enabled, allowlist) in [(true, Some(vec!["read_file".into(), "bash".into()])),
            (true, None), (false, Some(Vec::new()))] {
            config.tools_enabled = enabled;
            config.tool_allowlist = allowlist.clone();
            let resolved = ResolvedLlmSettings {
                selected_profile: None,
                runtime: config.runtime.clone(), endpoint: config.endpoint.clone(),
                api_key: config.api_key.clone(), model: config.model.clone(),
                temperature: 0.3, top_p: 0.9, max_output_tokens: 8192,
                max_context_tokens: 128_000,
            };
            let actual = conversation_execution_config(&config, resolved, "original-session");
            assert_eq!(actual.tools_enabled, enabled);
            assert_eq!(actual.tool_allowlist, allowlist);
            assert_eq!(actual.session_id.as_deref(), Some("original-session"));
            assert_eq!(actual.runtime.temperature, Some(0.3));
            assert_eq!(actual.runtime.top_p, Some(0.9));
        }
    }

    #[test]
    fn untyped_maintenance_error_does_not_claim_no_commit_or_context_overflow() {
        let response = compaction_execution_error("receipt observation failed".into());
        assert_eq!(response.status_code(), StatusCode(500));
        let mut body = String::new();
        response.into_reader().read_to_string(&mut body).unwrap();
        let body: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(body["outcome_unknown"], true);
        assert!(body.get("compacted").is_none());
    }
}

fn compact_response(
    request: &mut Request,
    conversation_id: &str,
    config: &Config,
) -> Response<Box<dyn Read + Send>> {
    let mut body = match read_json_value(request) {
        Ok(value) => value,
        Err(response) => return response,
    };
    body["conversationId"] = Value::String(conversation_id.to_string());
    let body = match apply_conversation_request_configuration(body) {
        Ok(body) => body,
        Err(error) => return json_response(StatusCode(422), json!({"error": error})),
    };
    let state = load_state().unwrap_or_default();
    let resolved = match resolve_llm_settings(&body, config, Some(&state.agent_settings), Some(conversation_id)) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let messages = agent_context_messages_from_request(&body, &state.agent_settings);
    let initial_items = messages
        .iter()
        .enumerate()
        .flat_map(|(index, message)| {
            session_message_to_agent_items(message, conversation_id, index)
        })
        .collect::<Vec<_>>();
    let result = match execute_conversation_compaction(config, resolved, conversation_id, initial_items) {
        Ok(value) => value,
        Err(error) => {
            return compaction_execution_error(error)
        }
    };
    json_response(
        StatusCode(200),
        json!({
            "ok": true,
            "compacted": result.replaced_item_count > 0,
            "message": if result.replaced_item_count == 0 {
                "No complete historical turn is available to compact."
            } else {
                "Conversation context compacted."
            },
            "context": {
                "usedTokens": result.context_used_tokens,
                "contextWindowTokens": result.context_window_tokens,
                "usagePercent": result.context_usage_percent,
                "replacedItemCount": result.replaced_item_count,
                "remainingItemCount": result.remaining_item_count,
            },
            "localFirst": true,
        }),
    )
}

fn memory_search_response(url: &str, conversation_id: &str) -> Response<Box<dyn Read + Send>> {
    let query = query_param(url, "q").unwrap_or_default();
    let scope = query_param(url, "scope").unwrap_or_else(|| "global".to_string());
    let kind = query_param(url, "kind");
    let limit = query_param(url, "limit")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(12)
        .clamp(1, 50);
    let store = match NolaStore::open_default() {
        Ok(store) => store,
        Err(err) => return json_response(StatusCode(500), json!({"error": err})),
    };
    match store.get_web_conversation(conversation_id) {
        Ok(Some(_)) => {}
        Ok(None) => return not_found("会话不存在"),
        Err(err) => return json_response(StatusCode(500), json!({"error": err})),
    }
    let hits = match store.search_memories(&nola_domain::memory_owner::MemoryExecutionBinding {
        conversation_id: Some(conversation_id.to_string()),
        ..nola_domain::memory_owner::MemoryExecutionBinding::local_profile()
    }, &scope, non_empty_string(&query), kind.as_deref(), limit)
    {
        Ok(items) => items,
        Err(err) => return json_response(StatusCode(500), json!({"error": err})),
    };
    json_response(
        StatusCode(200),
        json!({
            "ok": true,
            "scope": scope,
            "kind": kind,
            "query": query,
            "limit": limit,
            "totalHits": hits.len(),
            "hits": hits,
        }),
    )
}


fn document_search_response(url: &str) -> Response<Box<dyn Read + Send>> {
    let query = query_param(url, "q").unwrap_or_default();
    let scope = query_param(url, "scope").unwrap_or_else(|| "web:local".to_string());
    let kind = query_param(url, "kind");
    let limit = query_param(url, "limit")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(20)
        .clamp(1, 100);
    match NolaStore::open_default().and_then(|store| {
        store.search_documents(&scope, non_empty_string(&query), kind.as_deref(), limit)
    }) {
        Ok(documents) => json_response(
            StatusCode(200),
            json!({
                "ok": true,
                "scope": scope,
                "kind": kind,
                "query": query,
                "limit": limit,
                "totalHits": documents.len(),
                "documents": documents,
                "localFirst": true,
                "backend": "cli-rust",
            }),
        ),
        Err(err) => json_response(StatusCode(500), json!({"error": err})),
    }
}

fn home_response() -> Response<Box<dyn Read + Send>> {
    let store = match NolaStore::open_default() {
        Ok(store) => store,
        Err(err) => return json_response(StatusCode(500), json!({"error": err})),
    };
    let actions = match store.load_web_scheduled_actions() {
        Ok(values) => values
            .into_iter()
            .filter_map(|value| serde_json::from_value::<WebScheduledAction>(value).ok())
            .collect::<Vec<_>>(),
        Err(err) => return json_response(StatusCode(500), json!({"error": err})),
    };
    let results = match store.load_web_execution_results() {
        Ok(values) => values
            .into_iter()
            .filter_map(|value| serde_json::from_value::<WebExecutionResult>(value).ok())
            .collect::<Vec<_>>(),
        Err(err) => return json_response(StatusCode(500), json!({"error": err})),
    };
    let tasks = actions
        .iter()
        .map(|action| {
            json!({
                "sourceId": action.id,
                "name": action.name,
                "format": action.format,
                "enabled": action.enabled,
                "scheduleFrequency": action.frequency,
                "scheduledTimeISO": action.time,
                "scheduledWeekday": action.weekday,
                "nextScheduledAt": action.next_scheduled_at,
                "latestResult": latest_result_for_action_values(&results, &action.id),
            })
        })
        .collect::<Vec<_>>();
    let success_count = results
        .iter()
        .filter(|item| matches!(item.status.as_str(), "answered" | "action_completed"))
        .count();
    let failed_count = results
        .iter()
        .filter(|item| matches!(item.status.as_str(), "failed" | "incomplete" | "blocked"))
        .count();
    json_response(
        StatusCode(200),
        json!({
            "tasks": tasks,
            "stats": {
                "totalTasks": actions.len(),
                "successCount": success_count,
                "failedCount": failed_count,
            },
            "localFirst": true,
            "backend": "cli-rust",
        }),
    )
}

fn with_data_source_store<T>(
    operation: impl FnOnce(&mut DataSourceStore) -> Result<T, String>,
) -> Result<T, String> {
    let lock = DATA_SOURCE_STATE_LOCK.get_or_init(|| Mutex::new(()));
    let _guard = lock
        .lock()
        .map_err(|_| "failed to lock data source state".to_string())?;
    let mut store = DataSourceStore::open_default()?;
    operation(&mut store)
}

fn data_sources_response(url: &str) -> Response<Box<dyn Read + Send>> {
    let conversation_id =
        query_param(url, "conversationId").or_else(|| query_param(url, "conversation_id"));
    match with_data_source_store(|store| {
        let sources = store
            .sources()
            .iter()
            .map(|source| data_source_api_value(store, source, conversation_id.as_deref()))
            .collect::<Vec<_>>();
        Ok(json!({
            "sources": sources,
            "conversationId": conversation_id,
            "localFirst": true,
            "schemaVersion": nola_core::data_source::DATA_SOURCE_SCHEMA_VERSION,
        }))
    }) {
        Ok(value) => json_response(StatusCode(200), value),
        Err(error) => json_response(StatusCode(500), json!({"error": error})),
    }
}

fn data_source_usage_response() -> Response<Box<dyn Read + Send>> {
    match with_data_source_store(|store| {
        Ok(json!({
            "usageRecords": store.usage_records().iter().take(100).collect::<Vec<_>>(),
            "localFirst": true,
        }))
    }) {
        Ok(value) => json_response(StatusCode(200), value),
        Err(error) => json_response(StatusCode(500), json!({"error": error})),
    }
}

fn create_data_source_response(request: &mut Request) -> Response<Box<dyn Read + Send>> {
    let value = match read_json_value(request) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let draft = match serde_json::from_value::<DataSourceDraft>(value) {
        Ok(draft) => draft,
        Err(error) => {
            return json_response(
                StatusCode(400),
                json!({"error": format!("invalid data source: {error}")}),
            )
        }
    };
    match with_data_source_store(|store| store.validate_and_save(&draft)) {
        Ok(source) => json_response(StatusCode(201), json!({"source": source, "tested": true})),
        Err(error) => json_response(StatusCode(400), json!({"error": error, "tested": false})),
    }
}

fn update_data_source_response(
    request: &mut Request,
    source_id: &str,
) -> Response<Box<dyn Read + Send>> {
    let value = match read_json_value(request) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let mut draft = match serde_json::from_value::<DataSourceDraft>(value) {
        Ok(draft) => draft,
        Err(error) => {
            return json_response(
                StatusCode(400),
                json!({"error": format!("invalid data source: {error}")}),
            )
        }
    };
    draft.id = Some(source_id.to_string());
    match with_data_source_store(|store| {
        if store.source(source_id).is_none() {
            return Err(format!("unknown data source: {source_id}"));
        }
        store.validate_and_save(&draft)
    }) {
        Ok(source) => json_response(StatusCode(200), json!({"source": source, "tested": true})),
        Err(error) => json_response(StatusCode(400), json!({"error": error, "tested": false})),
    }
}

fn test_data_source_response(source_id: &str) -> Response<Box<dyn Read + Send>> {
    match with_data_source_store(|store| store.refresh(source_id)) {
        Ok(source) => json_response(StatusCode(200), json!({"source": source, "tested": true})),
        Err(error) => json_response(StatusCode(400), json!({"error": error, "tested": false})),
    }
}

fn delete_data_source_response(source_id: &str) -> Response<Box<dyn Read + Send>> {
    match with_data_source_store(|store| store.remove(source_id)) {
        Ok(true) => json_response(StatusCode(200), json!({"deleted": true, "id": source_id})),
        Ok(false) => json_response(StatusCode(404), json!({"error": "data source not found"})),
        Err(error) => json_response(StatusCode(500), json!({"error": error})),
    }
}

fn update_conversation_data_source_response(
    request: &mut Request,
    conversation_id: &str,
    source_id: &str,
) -> Response<Box<dyn Read + Send>> {
    if source_id.is_empty() || source_id.contains('/') {
        return json_response(StatusCode(400), json!({"error": "invalid data source id"}));
    }
    let conversation_store = match NolaStore::open_default() {
        Ok(store) => store,
        Err(error) => return json_response(StatusCode(500), json!({"error": error})),
    };
    match conversation_store.get_web_conversation(conversation_id) {
        Ok(Some(_)) => {}
        Ok(None) => return not_found("会话不存在"),
        Err(error) => return json_response(StatusCode(500), json!({"error": error})),
    }
    let value = match read_json_value(request) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let enabled = match value.get("enabled") {
        Some(Value::Bool(enabled)) => Some(*enabled),
        Some(Value::Null) | None => None,
        _ => {
            return json_response(
                StatusCode(400),
                json!({"error": "enabled must be true, false, or null"}),
            )
        }
    };
    match with_data_source_store(|store| {
        store.set_conversation_override(conversation_id, source_id, enabled)?;
        let source = store
            .source(source_id)
            .ok_or_else(|| format!("unknown data source: {source_id}"))?;
        Ok(data_source_api_value(store, source, Some(conversation_id)))
    }) {
        Ok(source) => json_response(StatusCode(200), json!({"source": source})),
        Err(error) => json_response(StatusCode(400), json!({"error": error})),
    }
}

fn data_source_api_value(
    store: &DataSourceStore,
    source: &StoredDataSource,
    conversation_id: Option<&str>,
) -> Value {
    let resolution = conversation_id.map(|id| store.resolve(source, id));
    json!({
        "source": source,
        "conversationOverride": conversation_id.and_then(|id| store.conversation_override(id, &source.id)),
        "resolution": resolution,
    })
}

fn skill_settings_review_response(expected_revision: Option<i64>) -> Response<Box<dyn Read + Send>> {
    let result = (|| -> Result<Value, String> {
        use nola_application::{ApplicationCommand, ProductSetting};
        let mut application = crate::application_port::CliApplicationPort::persistent_default()?;
        if let Some(revision) = expected_revision {
            confirm_core_skill_settings(&mut application, revision)?;
            let settings=load_agent_settings_from_store(&NolaStore::open_default()?)?;
            builtin_skills::synchronize(&settings.builtin_skill_overrides)
                .map_err(|error|format!("Core settings were selected and the previous settings retained, but Skill materialization failed: {error}"))?;
            return Ok(json!({"confirmed":true}));
        }
        let current: ProductSetting = application.execute::<Option<ProductSetting>>(ApplicationCommand::GetProductSetting {
            key:"web.agent.settings".into(),
        })?.ok_or("No saved Web settings to review")?;
        let raw: Value = serde_json::from_str(&current.value_json).map_err(|error|error.to_string())?;
        let pending = has_unapplied_web_skill_settings(&raw);
        let previous = if pending { raw["builtin_skill_overrides"].clone() } else {
            let key = raw["skill_settings_backup_key"].as_str().filter(|key|key.starts_with("web.skill-settings.unapplied."))
                .ok_or("No retained Skill settings")?;
            let saved: ProductSetting = application.execute::<Option<ProductSetting>>(ApplicationCommand::GetProductSetting {key:key.into()})?
                .ok_or("Retained Skill settings are unavailable")?;
            serde_json::from_str(&saved.value_json).map_err(|error|error.to_string())?
        };
        Ok(json!({"expected_revision":current.revision,"pending":pending,"previous":previous,
            "current":crate::skill_catalog_port::load_web_builtin_skills()?}))
    })();
    match result { Ok(value)=>json_response(StatusCode(200),value), Err(error)=>json_response(StatusCode(409),json!({"error":error})) }
}

fn agent_settings_response() -> Response<Box<dyn Read + Send>> {
    let store = match NolaStore::open_default() {
        Ok(store) => store,
        Err(err) => return json_response(StatusCode(500), json!({"error": err})),
    };
    match load_agent_settings_from_store(&store) {
        Ok(settings) => json_response(StatusCode(200), agent_settings_api(&settings)),
        Err(err) => {
            let review = store.load_web_agent_settings().ok().flatten().as_ref().is_some_and(has_unapplied_web_skill_settings);
            json_response(StatusCode(500), json!({"error":err,"code":if review {"skill_settings_review_required"} else {"agent_settings_unavailable"}}))
        },
    }
}

fn update_agent_settings_response(request: &mut Request) -> Response<Box<dyn Read + Send>> {
    let value = match read_json_value(request) {
        Ok(value) => value,
        Err(response) => return response,
    };
    if let Err(error) = validate_model_setting_values(&value) {
        return json_response(
            StatusCode(422),
            json!({"error": error, "code": "invalid_model_parameters"}),
        );
    }
    if let Some(raw_profile) = value.get("permission_profile") {
        let valid = raw_profile
            .as_str()
            .and_then(crate::permissions::PermissionProfile::from_canonical_name)
            .is_some();
        if !valid {
            return json_response(
                StatusCode(422),
                json!({
                    "error": "permission_profile must be 'cautious' or 'allow_all'",
                    "code": "invalid_permission_profile"
                }),
            );
        }
    }
    let incoming_mcp_servers = value.get("mcp_servers").and_then(Value::as_array).cloned();
    let incoming_skills = value.get("skills").or_else(|| value.get("uploaded_skills")).and_then(Value::as_array).cloned();
    let skills_changed = incoming_skills.is_some();
    let skill_baseline = if skills_changed {
        match value.get("skill_baseline").and_then(Value::as_array) {
            Some(baseline) => baseline.clone(),
            None => return json_response(StatusCode(400), json!({
                "error":"Skill editing requires the original revision baseline", "code":"skill_baseline_required"
            })),
        }
    } else { Vec::new() };
    let mcp_settings_changed = incoming_mcp_servers.is_some();
    if mcp_settings_changed
        && value.as_object().is_some_and(|object| {
            object
                .keys()
                .any(|key| key != "mcp_servers" && key != "mcp_baseline")
        })
    {
        return json_response(
            StatusCode(400),
            json!({
                "error": "Save MCP changes separately from other settings",
                "code": "mixed_resource_mutation"
            }),
        );
    }
    let mcp_baseline = if mcp_settings_changed {
        match value.get("mcp_baseline").and_then(Value::as_array) {
            Some(baseline) => baseline.clone(),
            None => {
                return json_response(
                    StatusCode(400),
                    json!({
                        "error": "MCP editing requires the original server revision baseline",
                        "code": "mcp_baseline_required"
                    }),
                )
            }
        }
    } else {
        Vec::new()
    };
    let mut store = match NolaStore::open_default() {
        Ok(store) => store,
        Err(err) => return json_response(StatusCode(500), json!({"error": err})),
    };
    let mut settings = match load_agent_settings_from_store(&store) {
        Ok(settings) => settings,
        Err(err) => return json_response(StatusCode(500), json!({"error": err})),
    };
    let existing_mcp_servers = settings.mcp_servers.clone();
    merge_agent_settings(&mut settings, value);
    if mcp_settings_changed {
        let reconciled = match crate::resource_access::reconcile_mcp_resources(
            &existing_mcp_servers,
            incoming_mcp_servers.as_deref().unwrap_or_default(),
        ) {
            Ok(servers) => servers,
            Err(error) => {
                return json_response(
                    StatusCode(403),
                    json!({
                        "error": error,
                        "code": "mcp_resource_access_denied",
                        "message": "The MCP resource update was rejected by the resource access contract."
                    }),
                )
            }
        };
        settings.mcp_servers = match crate::mcp_host::describe_configured_servers(&reconciled) {
            Ok(servers) => servers,
            Err(error) => {
                return json_response(
                    StatusCode(422),
                    json!({
                        "error": error.message,
                        "code": error.code,
                        "message": "The MCP configuration was rejected. Saving a configuration does not establish or approve a connection."
                    }),
                )
            }
        };
    }
    if mcp_settings_changed {
        let mcp_catalog_revision =
            crate::agent_host_driver::configured_mcp_capability_catalog_revision(&settings.mcp_servers);
        if let Err(error) = crate::agent_host_driver::configured_mcp_capability_catalog(
            &settings.mcp_servers,
            mcp_catalog_revision,
        ) {
            return json_response(
                StatusCode(422),
                json!({
                    "error": error,
                    "code": "mcp_handshake_required",
                    "message": "An MCP endpoint must complete capability discovery before it can be enabled."
                }),
            );
        }
    }
    if mcp_settings_changed {
        if let Err(err) =
            crate::mcp_catalog_port::save_web_mcp_changes(&mcp_baseline, &settings.mcp_servers)
        {
            return json_response(
                StatusCode(409),
                json!({"error": err, "code": "mcp_update_failed"}),
            );
        }
        settings.mcp_servers = match crate::mcp_catalog_port::load_web_mcp_servers() {
            Ok(servers) => servers,
            Err(err) => return json_response(StatusCode(500), json!({"error": err})),
        };
        return json_response(StatusCode(200), agent_settings_api(&settings));
    }
    let existing_documents = match store.load_web_uploaded_documents() {
        Ok(documents) => documents,
        Err(err) => return json_response(StatusCode(500), json!({"error": err})),
    };
    settings.uploaded_documents = match crate::resource_access::reconcile_data_sources(
        &existing_documents,
        &settings.uploaded_documents,
    ) {
        Ok(documents) => documents,
        Err(error) => {
            return json_response(
                StatusCode(403),
                json!({
                    "error": error,
                    "code": "data_source_access_denied",
                    "message": "The data-source update was rejected by the resource access contract."
                }),
            )
        }
    };
    if let Err(err) = store.replace_web_uploaded_documents(&settings.uploaded_documents) {
        return json_response(StatusCode(500), json!({"error": err}));
    }
    if skills_changed {
        if let Err(err) = crate::skill_catalog_port::save_web_uploaded_skills(&skill_baseline, incoming_skills.as_deref().unwrap_or_default()) {
            return json_response(StatusCode(409), json!({"error": err, "code":"skill_update_failed"}));
        }
    }
    settings.uploaded_skills = match crate::skill_catalog_port::load_web_uploaded_skills() {
        Ok(skills) => skills,
        Err(err) => return json_response(StatusCode(500), json!({"error": err})),
    };
    settings.builtin_skill_overrides = match crate::skill_catalog_port::load_web_builtin_skills() {
        Ok(skills) => skills,
        Err(err) => return json_response(StatusCode(500), json!({"error": err})),
    };
    settings.mcp_servers = match crate::mcp_catalog_port::load_web_mcp_servers() {
        Ok(servers) => servers,
        Err(err) => return json_response(StatusCode(500), json!({"error": err})),
    };
    let mut settings_for_store = settings.clone();
    settings_for_store.uploaded_documents.clear();
    settings_for_store.uploaded_skills.clear();
    settings_for_store.mcp_servers.clear();
    if let Err(err) = store.save_web_agent_settings(
        &serde_json::to_value(&settings_for_store).unwrap_or_else(|_| json!({})),
    ) {
        return json_response(StatusCode(500), json!({"error": err}));
    }
    if let Err(error) =
        builtin_skills::synchronize(&settings.builtin_skill_overrides)
    {
        return json_response(
            StatusCode(500),
            json!({
                "error": error,
                "code": "skill_materialization_failed",
                "message": "The Skill settings were saved, but their runtime catalog could not be activated."
            }),
        );
    }
    json_response(StatusCode(200), agent_settings_api(&settings))
}

#[cfg(test)]
#[path = "routes_tests.rs"]
mod routes_tests;

#[cfg(test)]
mod p2_route_security_tests {
    use super::web_csrf_exempt_path;

    #[test]
    fn logout_is_not_csrf_exempt() {
        assert!(!web_csrf_exempt_path("/logout"));
        assert!(web_csrf_exempt_path("/login"));
        assert!(web_csrf_exempt_path("/api/web/login"));
        assert!(web_csrf_exempt_path("/api/mobile/ping"));
    }
}
