// SPDX-License-Identifier: Apache-2.0
//! MCP JSON-RPC 2.0 transports: `Stdio`, `StreamableHttp`, and `ServerSentEvents`
//! with finite timeouts, dedicated reader threads, proper `notifications/initialized`,
//! `nextCursor` pagination, resources/prompts RPCs, `notifications/*/list_changed`
//! cache invalidation, progress & cancellation support, dynamic OAuth bearer lookup,
//! and non-fatal per-request error isolation.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use nola_types::{McpServerConfig, McpTransport};
use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::oauth::McpOAuthManager;
use crate::McpError;

const RPC_TIMEOUT: Duration = Duration::from_secs(30);
const SSE_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const SSE_STREAM_TIMEOUT: Duration = Duration::from_secs(300);
const CANCEL_POLL_INTERVAL: Duration = Duration::from_millis(25);
const MAX_LIST_PAGES: usize = 32;
const JSONRPC_METHOD_NOT_FOUND: i64 = -32601;

/// Callback invoked when an MCP server emits `notifications/progress` for an active request.
/// Arguments: `(progress, total, message)`.
pub type ProgressCallback = Arc<dyn Fn(f64, Option<f64>, Option<&str>) + Send + Sync>;

/// Metadata describing an MCP resource returned by `resources/list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpResource {
    pub uri: String,
    pub name: String,
    pub description: Option<String>,
    pub mime_type: Option<String>,
}

/// Metadata describing a parameterized MCP resource template returned by `resources/templates/list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpResourceTemplate {
    pub uri_template: String,
    pub name: String,
    pub description: Option<String>,
    pub mime_type: Option<String>,
}

/// Content item returned by `resources/read`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpResourceContent {
    pub uri: String,
    pub mime_type: Option<String>,
    pub text: Option<String>,
    pub blob: Option<String>,
}

/// Prompt argument metadata returned inside `prompts/list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpPromptArgument {
    pub name: String,
    pub description: Option<String>,
    pub required: bool,
}

/// Metadata describing an MCP prompt template returned by `prompts/list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpPrompt {
    pub name: String,
    pub description: Option<String>,
    pub arguments: Vec<McpPromptArgument>,
}

/// Rendered message item returned by `prompts/get`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct McpPromptMessage {
    pub role: String,
    pub content: Value,
}

/// Explicit allowlist of safe environment variables inherited by stdio MCP child processes.
/// Prevents parent API keys (e.g. `OPENAI_API_KEY`, `ANTHROPIC_API_KEY`) from leaking.
const SAFE_STDIO_ENV_ALLOWLIST: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "LANG",
    "LC_ALL",
    "TMPDIR",
    "SHELL",
    "TERM",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
    "XDG_RUNTIME_DIR",
];

pub(crate) fn is_loopback_url(raw: &str) -> bool {
    let Ok(parsed) = url::Url::parse(raw) else {
        return false;
    };
    let Some(host) = parsed.host_str() else {
        return false;
    };
    let lower = host.trim_end_matches('.').to_ascii_lowercase();
    if lower == "localhost" || lower.ends_with(".localhost") {
        return true;
    }
    let clean = lower.trim_start_matches('[').trim_end_matches(']');
    clean
        .parse::<std::net::IpAddr>()
        .map(|ip| ip.is_loopback())
        .unwrap_or(false)
}

fn build_mcp_http_client(
    url: &str,
    timeout: Option<Duration>,
    connect_timeout: Option<Duration>,
) -> Result<Client, McpError> {
    let mut builder = Client::builder().redirect(reqwest::redirect::Policy::none());
    if let Some(t) = timeout {
        builder = builder.timeout(t);
    }
    if let Some(ct) = connect_timeout {
        builder = builder.connect_timeout(ct);
    }
    if is_loopback_url(url) {
        builder = builder.no_proxy();
    }
    builder
        .build()
        .map_err(|e| McpError::Transport(e.to_string()))
}

type PendingMap = Arc<Mutex<BTreeMap<u64, mpsc::Sender<Result<Value, McpError>>>>>;
type ProgressMap = Arc<Mutex<BTreeMap<String, ProgressCallback>>>;

pub(crate) struct ListChangedFlags {
    pub tools_dirty: AtomicBool,
    pub resources_dirty: AtomicBool,
    pub resource_templates_dirty: AtomicBool,
    pub prompts_dirty: AtomicBool,
}

impl ListChangedFlags {
    fn new_dirty() -> Self {
        Self {
            tools_dirty: AtomicBool::new(true),
            resources_dirty: AtomicBool::new(true),
            resource_templates_dirty: AtomicBool::new(true),
            prompts_dirty: AtomicBool::new(true),
        }
    }
}

struct ProgressGuard {
    map: ProgressMap,
    token: String,
}

impl Drop for ProgressGuard {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.map.lock() {
            guard.remove(&self.token);
        }
    }
}

enum TransportBackend {
    Stdio(StdioState),
    StreamableHttp(HttpState),
    ServerSentEvents(SseState),
}

struct StdioState {
    child: Mutex<Child>,
    stdin: Mutex<Option<ChildStdin>>,
    pending: PendingMap,
    closed: Arc<AtomicBool>,
}

impl Drop for StdioState {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::SeqCst);
        if let Ok(mut stdin) = self.stdin.lock() {
            let _ = stdin.take();
        }
        if let Ok(mut child) = self.child.lock() {
            #[cfg(unix)]
            {
                let pgid = child.id() as libc::pid_t;
                if pgid > 0 {
                    unsafe {
                        libc::kill(-pgid, libc::SIGKILL);
                    }
                }
            }
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

struct HttpState {
    client: Client,
    server_id: String,
    url: String,
    headers: BTreeMap<String, String>,
    oauth: Option<Arc<McpOAuthManager>>,
    session_id: Arc<Mutex<Option<String>>>,
}

struct SseState {
    client: Client,
    server_id: String,
    post_url: Arc<Mutex<String>>,
    headers: BTreeMap<String, String>,
    oauth: Option<Arc<McpOAuthManager>>,
    pending: PendingMap,
    closed: Arc<AtomicBool>,
}

impl Drop for SseState {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::SeqCst);
        fail_all_pending(&self.pending, "mcp sse connection closed");
    }
}

/// Active JSON-RPC 2.0 connection to a single MCP server.
pub struct McpConnection {
    next_id: AtomicU64,
    backend: TransportBackend,
    flags: Arc<ListChangedFlags>,
    progress_handlers: ProgressMap,
    server_capabilities: Mutex<Value>,
    tools_cache: Mutex<Option<Vec<Value>>>,
    resources_cache: Mutex<Option<Vec<McpResource>>>,
    resource_templates_cache: Mutex<Option<Vec<McpResourceTemplate>>>,
    prompts_cache: Mutex<Option<Vec<McpPrompt>>>,
}

impl McpConnection {
    pub fn open(
        cfg: &McpServerConfig,
        oauth: Option<Arc<McpOAuthManager>>,
    ) -> Result<Self, McpError> {
        let flags = Arc::new(ListChangedFlags::new_dirty());
        let progress_handlers: ProgressMap = Arc::new(Mutex::new(BTreeMap::new()));

        let backend = match cfg.transport {
            McpTransport::Stdio => {
                let cmd_name = cfg
                    .command
                    .as_deref()
                    .filter(|s| !s.trim().is_empty())
                    .ok_or_else(|| McpError::Transport("stdio mcp server missing command".into()))?;
                let mut cmd = Command::new(cmd_name);
                cmd.env_clear();
                for key in SAFE_STDIO_ENV_ALLOWLIST {
                    if let Some(val) = std::env::var_os(key) {
                        cmd.env(key, val);
                    }
                }
                cmd.args(&cfg.args)
                    .envs(&cfg.env)
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null());
                #[cfg(unix)]
                {
                    use std::os::unix::process::CommandExt;
                    cmd.process_group(0);
                }
                let mut child = cmd.spawn()?;
                let stdin = child
                    .stdin
                    .take()
                    .ok_or_else(|| McpError::Transport("failed to open child stdin".into()))?;
                let stdout = child
                    .stdout
                    .take()
                    .ok_or_else(|| McpError::Transport("failed to open child stdout".into()))?;

                let pending: PendingMap = Arc::new(Mutex::new(BTreeMap::new()));
                let closed = Arc::new(AtomicBool::new(false));
                let pending_bg = Arc::clone(&pending);
                let closed_bg = Arc::clone(&closed);
                let flags_bg = Arc::clone(&flags);
                let progress_bg = Arc::clone(&progress_handlers);
                thread::spawn(move || {
                    let mut reader = BufReader::new(stdout);
                    let mut line = String::new();
                    while let Ok(n) = reader.read_line(&mut line) {
                        if n == 0 {
                            break;
                        }
                        if let Ok(val) = serde_json::from_str::<Value>(line.trim()) {
                            dispatch_rpc_frame(&pending_bg, &flags_bg, &progress_bg, val);
                        }
                        line.clear();
                    }
                    closed_bg.store(true, Ordering::SeqCst);
                    fail_all_pending(&pending_bg, "mcp stdio closed");
                });

                TransportBackend::Stdio(StdioState {
                    child: Mutex::new(child),
                    stdin: Mutex::new(Some(stdin)),
                    pending,
                    closed,
                })
            }
            McpTransport::StreamableHttp => {
                let url = cfg
                    .url
                    .clone()
                    .filter(|s| !s.trim().is_empty())
                    .ok_or_else(|| {
                        McpError::Transport("streamable_http server missing url".into())
                    })?;
                let client = build_mcp_http_client(&url, Some(RPC_TIMEOUT), None)?;
                TransportBackend::StreamableHttp(HttpState {
                    client,
                    server_id: cfg.id.clone(),
                    url,
                    headers: cfg.headers.clone(),
                    oauth,
                    session_id: Arc::new(Mutex::new(None)),
                })
            }
            McpTransport::ServerSentEvents => {
                let url = cfg
                    .url
                    .clone()
                    .filter(|s| !s.trim().is_empty())
                    .ok_or_else(|| McpError::Transport("sse server missing url".into()))?;
                let client = build_mcp_http_client(&url, Some(RPC_TIMEOUT), None)?;
                let pending: PendingMap = Arc::new(Mutex::new(BTreeMap::new()));
                let post_url = Arc::new(Mutex::new(url.clone()));
                let closed = Arc::new(AtomicBool::new(false));

                let bearer = resolve_bearer(&oauth, &cfg.id)?;
                let sse_client = build_mcp_http_client(
                    &url,
                    Some(SSE_STREAM_TIMEOUT),
                    Some(Duration::from_secs(10)),
                )?;
                let hmap = build_headers(
                    &[("accept", "text/event-stream")],
                    &cfg.headers,
                    bearer.as_deref(),
                    None,
                );
                let resp = sse_client
                    .get(&url)
                    .headers(hmap)
                    .send()
                    .map_err(|e| McpError::Transport(e.to_string()))?;
                if !resp.status().is_success() {
                    return Err(McpError::Transport(format!(
                        "mcp sse GET returned HTTP {}",
                        resp.status()
                    )));
                }

                let (ep_tx, ep_rx) = mpsc::channel::<Result<String, String>>();
                let pending_bg = Arc::clone(&pending);
                let post_url_bg = Arc::clone(&post_url);
                let closed_bg = Arc::clone(&closed);
                let flags_bg = Arc::clone(&flags);
                let progress_bg = Arc::clone(&progress_handlers);
                let base_url = url.clone();
                thread::spawn(move || {
                    let mut reader = BufReader::new(resp);
                    let mut line = String::new();
                    let mut current_event = String::new();
                    let mut data_lines: Vec<String> = Vec::new();
                    let mut ep_tx = Some(ep_tx);

                    let mut flush_event = |ev: &mut String, data: &mut Vec<String>| -> bool {
                        if data.is_empty() {
                            ev.clear();
                            return true;
                        }
                        let joined = data.join("\n");
                        let d = joined.trim();
                        if ev == "endpoint" && !d.is_empty() {
                            match url::Url::parse(&base_url).and_then(|base| {
                                base.join(d).map(|resolved| (base, resolved))
                            }) {
                                Ok((base, resolved)) => {
                                    let same_origin = resolved.scheme() == base.scheme()
                                        && resolved.host_str() == base.host_str()
                                        && resolved.port_or_known_default()
                                            == base.port_or_known_default();
                                    if !same_origin {
                                        if let Some(tx) = ep_tx.take() {
                                            let _ = tx.send(Err(format!(
                                                "rejected cross-origin MCP SSE endpoint '{resolved}'"
                                            )));
                                        }
                                        ev.clear();
                                        data.clear();
                                        return false;
                                    }
                                    let resolved_str = resolved.to_string();
                                    *post_url_bg.lock().unwrap() = resolved_str.clone();
                                    if let Some(tx) = ep_tx.take() {
                                        let _ = tx.send(Ok(resolved_str));
                                    }
                                }
                                Err(e) => {
                                    if let Some(tx) = ep_tx.take() {
                                        let _ = tx.send(Err(format!(
                                            "invalid MCP SSE endpoint '{d}': {e}"
                                        )));
                                    }
                                    ev.clear();
                                    data.clear();
                                    return false;
                                }
                            }
                        } else if let Ok(val) = serde_json::from_str::<Value>(d) {
                            dispatch_rpc_frame(&pending_bg, &flags_bg, &progress_bg, val);
                        }
                        ev.clear();
                        data.clear();
                        true
                    };

                    while !closed_bg.load(Ordering::SeqCst) {
                        let Ok(n) = reader.read_line(&mut line) else {
                            break;
                        };
                        if n == 0 {
                            let _ = flush_event(&mut current_event, &mut data_lines);
                            break;
                        }
                        let trimmed = line.trim_end_matches(&['\r', '\n'][..]);
                        if trimmed.is_empty() {
                            if !flush_event(&mut current_event, &mut data_lines) {
                                break;
                            }
                        } else if let Some(ev) = trimmed.strip_prefix("event:") {
                            current_event = ev.trim().to_string();
                        } else if let Some(data) = trimmed.strip_prefix("data:") {
                            data_lines.push(data.strip_prefix(' ').unwrap_or(data).to_string());
                        }
                        line.clear();
                    }
                    if let Some(tx) = ep_tx.take() {
                        let _ = tx.send(Err("sse stream closed before endpoint event".into()));
                    }
                    closed_bg.store(true, Ordering::SeqCst);
                    fail_all_pending(&pending_bg, "mcp sse stream closed");
                });

                // Wait for the mandatory `event: endpoint` handshake frame!
                match ep_rx.recv_timeout(SSE_HANDSHAKE_TIMEOUT) {
                    Ok(Ok(_)) => {}
                    Ok(Err(e)) => {
                        closed.store(true, Ordering::SeqCst);
                        return Err(McpError::Transport(e));
                    }
                    Err(_) => {
                        closed.store(true, Ordering::SeqCst);
                        return Err(McpError::Transport(
                            "timed out waiting for MCP SSE endpoint event".into(),
                        ));
                    }
                }

                TransportBackend::ServerSentEvents(SseState {
                    client,
                    server_id: cfg.id.clone(),
                    post_url,
                    headers: cfg.headers.clone(),
                    oauth,
                    pending,
                    closed,
                })
            }
        };

        Ok(Self {
            next_id: AtomicU64::new(1),
            backend,
            flags,
            progress_handlers,
            server_capabilities: Mutex::new(Value::Null),
            tools_cache: Mutex::new(None),
            resources_cache: Mutex::new(None),
            resource_templates_cache: Mutex::new(None),
            prompts_cache: Mutex::new(None),
        })
    }

    pub fn is_healthy(&self) -> bool {
        match &self.backend {
            TransportBackend::Stdio(s) => !s.closed.load(Ordering::SeqCst),
            TransportBackend::ServerSentEvents(s) => !s.closed.load(Ordering::SeqCst),
            TransportBackend::StreamableHttp(_) => true,
        }
    }

    /// Returns whether the server declared `capabilities.resources` during `initialize`.
    pub fn supports_resources(&self) -> bool {
        self.server_capabilities
            .lock()
            .unwrap()
            .get("resources")
            .is_some_and(|v| !v.is_null())
    }

    /// Returns whether the server declared `capabilities.prompts` during `initialize`.
    pub fn supports_prompts(&self) -> bool {
        self.server_capabilities
            .lock()
            .unwrap()
            .get("prompts")
            .is_some_and(|v| !v.is_null())
    }

    /// Returns true if a `notifications/tools/list_changed` notification has invalidated the cache.
    pub fn tools_list_dirty(&self) -> bool {
        self.flags.tools_dirty.load(Ordering::SeqCst)
    }

    /// Returns true if a `notifications/resources/list_changed` notification has invalidated the cache.
    pub fn resources_list_dirty(&self) -> bool {
        self.flags.resources_dirty.load(Ordering::SeqCst)
    }

    /// Returns true if a `notifications/prompts/list_changed` notification has invalidated the cache.
    pub fn prompts_list_dirty(&self) -> bool {
        self.flags.prompts_dirty.load(Ordering::SeqCst)
    }

    /// Explicitly marks all cached capability lists dirty so the next list calls re-fetch.
    pub fn invalidate_caches(&self) {
        self.flags.tools_dirty.store(true, Ordering::SeqCst);
        self.flags.resources_dirty.store(true, Ordering::SeqCst);
        self.flags
            .resource_templates_dirty
            .store(true, Ordering::SeqCst);
        self.flags.prompts_dirty.store(true, Ordering::SeqCst);
    }

    pub fn initialize(&self) -> Result<Value, McpError> {
        self.initialize_inner(true)
    }

    fn initialize_inner(&self, allow_404_retry: bool) -> Result<Value, McpError> {
        let res = self.request_inner(
            "initialize",
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "nola-v2", "version": "2.0.0"}
            }),
            allow_404_retry,
            None,
            None,
        )?;
        if let Some(caps) = res.get("capabilities") {
            *self.server_capabilities.lock().unwrap() = caps.clone();
        }
        // Mandatory MCP handshake notification
        let _ = self.notify("notifications/initialized", json!({}));
        Ok(res)
    }

    fn list_paginated(
        &self,
        method: &str,
        field: &str,
        optional: bool,
    ) -> Result<Vec<Value>, McpError> {
        let mut items = Vec::new();
        let mut cursor: Option<String> = None;
        let mut seen_cursors = BTreeSet::new();

        for _ in 0..MAX_LIST_PAGES {
            let params = match &cursor {
                Some(c) => json!({ "cursor": c }),
                None => json!({}),
            };
            let res = match self.request(method, params) {
                Ok(v) => v,
                Err(McpError::Rpc {
                    code: JSONRPC_METHOD_NOT_FOUND,
                    ..
                }) if optional => return Ok(items),
                Err(e) => return Err(e),
            };
            if let Some(arr) = res.get(field).and_then(Value::as_array) {
                items.extend(arr.iter().cloned());
            }
            let next = res
                .get("nextCursor")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToString::to_string);
            match next {
                Some(next_cursor) if seen_cursors.insert(next_cursor.clone()) => {
                    cursor = Some(next_cursor);
                }
                _ => break,
            }
        }
        Ok(items)
    }

    /// List all tools across `nextCursor` pages. Automatically refreshes after
    /// `notifications/tools/list_changed`.
    pub fn list_tools(&self) -> Result<Vec<Value>, McpError> {
        if !self.flags.tools_dirty.load(Ordering::SeqCst) {
            if let Some(cached) = self.tools_cache.lock().unwrap().as_ref() {
                return Ok(cached.clone());
            }
        }
        self.flags.tools_dirty.store(false, Ordering::SeqCst);
        match self.list_paginated("tools/list", "tools", false) {
            Ok(tools) => {
                *self.tools_cache.lock().unwrap() = Some(tools.clone());
                Ok(tools)
            }
            Err(e) => {
                self.flags.tools_dirty.store(true, Ordering::SeqCst);
                Err(e)
            }
        }
    }

    /// List all resources across `nextCursor` pages. Automatically refreshes after
    /// `notifications/resources/list_changed`.
    pub fn list_resources(&self) -> Result<Vec<McpResource>, McpError> {
        if !self.flags.resources_dirty.load(Ordering::SeqCst) {
            if let Some(cached) = self.resources_cache.lock().unwrap().as_ref() {
                return Ok(cached.clone());
            }
        }
        self.flags.resources_dirty.store(false, Ordering::SeqCst);
        match self.list_paginated("resources/list", "resources", true) {
            Ok(raw) => {
                let parsed: Vec<McpResource> = raw.into_iter().filter_map(decode_resource).collect();
                *self.resources_cache.lock().unwrap() = Some(parsed.clone());
                Ok(parsed)
            }
            Err(e) => {
                self.flags.resources_dirty.store(true, Ordering::SeqCst);
                Err(e)
            }
        }
    }

    /// List all resource templates across `nextCursor` pages. Automatically refreshes after
    /// `notifications/resources/list_changed`.
    pub fn list_resource_templates(&self) -> Result<Vec<McpResourceTemplate>, McpError> {
        if !self.flags.resource_templates_dirty.load(Ordering::SeqCst) {
            if let Some(cached) = self.resource_templates_cache.lock().unwrap().as_ref() {
                return Ok(cached.clone());
            }
        }
        self.flags
            .resource_templates_dirty
            .store(false, Ordering::SeqCst);
        match self.list_paginated("resources/templates/list", "resourceTemplates", true) {
            Ok(raw) => {
                let parsed: Vec<McpResourceTemplate> = raw
                    .into_iter()
                    .filter_map(decode_resource_template)
                    .collect();
                *self.resource_templates_cache.lock().unwrap() = Some(parsed.clone());
                Ok(parsed)
            }
            Err(e) => {
                self.flags
                    .resource_templates_dirty
                    .store(true, Ordering::SeqCst);
                Err(e)
            }
        }
    }

    /// Read the contents of a specific resource URI via `resources/read`.
    pub fn read_resource(&self, uri: &str) -> Result<Vec<McpResourceContent>, McpError> {
        let res = self.request("resources/read", json!({ "uri": uri }))?;
        let contents = res
            .get("contents")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| decode_resource_content(item, uri))
                    .collect()
            })
            .unwrap_or_default();
        Ok(contents)
    }

    /// List all prompt templates across `nextCursor` pages. Automatically refreshes after
    /// `notifications/prompts/list_changed`.
    pub fn list_prompts(&self) -> Result<Vec<McpPrompt>, McpError> {
        if !self.flags.prompts_dirty.load(Ordering::SeqCst) {
            if let Some(cached) = self.prompts_cache.lock().unwrap().as_ref() {
                return Ok(cached.clone());
            }
        }
        self.flags.prompts_dirty.store(false, Ordering::SeqCst);
        match self.list_paginated("prompts/list", "prompts", true) {
            Ok(raw) => {
                let parsed: Vec<McpPrompt> = raw.into_iter().filter_map(decode_prompt).collect();
                *self.prompts_cache.lock().unwrap() = Some(parsed.clone());
                Ok(parsed)
            }
            Err(e) => {
                self.flags.prompts_dirty.store(true, Ordering::SeqCst);
                Err(e)
            }
        }
    }

    /// Render a prompt template via `prompts/get`.
    pub fn get_prompt(
        &self,
        name: &str,
        arguments: Option<BTreeMap<String, String>>,
    ) -> Result<Vec<McpPromptMessage>, McpError> {
        let params = match arguments {
            Some(args) if !args.is_empty() => json!({ "name": name, "arguments": args }),
            _ => json!({ "name": name }),
        };
        let res = self.request("prompts/get", params)?;
        let messages = res
            .get("messages")
            .and_then(Value::as_array)
            .map(|arr| arr.iter().filter_map(decode_prompt_message).collect())
            .unwrap_or_default();
        Ok(messages)
    }

    pub fn call_tool(&self, name: &str, arguments: Value) -> Result<Value, McpError> {
        self.call_tool_with_options(name, arguments, None, None)
    }

    /// Invoke an MCP tool with optional cancellation flag and progress callback.
    /// Attaches `_meta.progressToken` when `on_progress` is supplied, dispatches
    /// `notifications/progress` frames, and sends `notifications/cancelled`
    /// (`{requestId, reason}`) when `cancel` transitions to `true`.
    pub fn call_tool_with_options(
        &self,
        name: &str,
        arguments: Value,
        cancel: Option<Arc<AtomicBool>>,
        on_progress: Option<ProgressCallback>,
    ) -> Result<Value, McpError> {
        self.request_inner(
            "tools/call",
            json!({
                "name": name,
                "arguments": arguments
            }),
            true,
            cancel,
            on_progress,
        )
    }

    pub fn notify(&self, method: &str, params: Value) -> Result<(), McpError> {
        let payload = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params
        });
        match &self.backend {
            TransportBackend::Stdio(s) => {
                let mut g = s.stdin.lock().unwrap();
                let Some(stdin) = g.as_mut() else {
                    return Err(McpError::Transport("mcp stdio closed".into()));
                };
                writeln!(stdin, "{}", serde_json::to_string(&payload)?)?;
                stdin.flush()?;
                Ok(())
            }
            TransportBackend::StreamableHttp(http) => {
                let bearer = resolve_bearer(&http.oauth, &http.server_id)?;
                let sid = http.session_id.lock().unwrap().clone();
                let hmap = build_headers(
                    &[
                        ("content-type", "application/json"),
                        ("accept", "application/json, text/event-stream"),
                    ],
                    &http.headers,
                    bearer.as_deref(),
                    sid.as_deref(),
                );
                let _ = http.client.post(&http.url).headers(hmap).json(&payload).send();
                Ok(())
            }
            TransportBackend::ServerSentEvents(sse) => {
                let bearer = resolve_bearer(&sse.oauth, &sse.server_id)?;
                let target = sse.post_url.lock().unwrap().clone();
                let hmap = build_headers(
                    &[("content-type", "application/json")],
                    &sse.headers,
                    bearer.as_deref(),
                    None,
                );
                let _ = sse.client.post(&target).headers(hmap).json(&payload).send();
                Ok(())
            }
        }
    }

    pub fn request(&self, method: &str, params: Value) -> Result<Value, McpError> {
        self.request_inner(method, params, true, None, None)
    }

    fn send_cancelled(&self, request_id: u64) {
        let _ = self.notify(
            "notifications/cancelled",
            json!({
                "requestId": request_id,
                "reason": "Cancelled by caller"
            }),
        );
    }

    fn request_inner(
        &self,
        method: &str,
        mut params: Value,
        allow_404_retry: bool,
        cancel: Option<Arc<AtomicBool>>,
        on_progress: Option<ProgressCallback>,
    ) -> Result<Value, McpError> {
        if cancel.as_ref().is_some_and(|c| c.load(Ordering::SeqCst)) {
            return Err(McpError::Cancelled(
                "mcp request cancelled before send".into(),
            ));
        }

        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let _progress_guard = if let Some(cb) = on_progress.as_ref() {
            let token = format!("nola-progress-{id}");
            attach_progress_token(&mut params, &token);
            self.progress_handlers
                .lock()
                .unwrap()
                .insert(token.clone(), Arc::clone(cb));
            Some(ProgressGuard {
                map: Arc::clone(&self.progress_handlers),
                token,
            })
        } else {
            None
        };

        let payload = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": &params
        });

        match &self.backend {
            TransportBackend::Stdio(s) => {
                if s.closed.load(Ordering::SeqCst) {
                    return Err(McpError::Transport("mcp stdio process already exited".into()));
                }
                let (tx, rx) = mpsc::channel();
                s.pending.lock().unwrap().insert(id, tx);
                {
                    let mut g = s.stdin.lock().unwrap();
                    let Some(stdin) = g.as_mut() else {
                        s.pending.lock().unwrap().remove(&id);
                        return Err(McpError::Transport("mcp stdio closed".into()));
                    };
                    if let Err(e) = writeln!(stdin, "{}", serde_json::to_string(&payload)?)
                        .and_then(|_| stdin.flush())
                    {
                        s.pending.lock().unwrap().remove(&id);
                        return Err(McpError::Io(e));
                    }
                }
                self.wait_for_pending_reply(&s.pending, id, rx, cancel.as_deref(), "stdio")
            }
            TransportBackend::StreamableHttp(http) => {
                let bearer = resolve_bearer(&http.oauth, &http.server_id)?;
                let sid = http.session_id.lock().unwrap().clone();
                let hmap = build_headers(
                    &[
                        ("content-type", "application/json"),
                        ("accept", "application/json, text/event-stream"),
                    ],
                    &http.headers,
                    bearer.as_deref(),
                    sid.as_deref(),
                );

                let execute_http = {
                    let client = http.client.clone();
                    let url = http.url.clone();
                    let session_id = Arc::clone(&http.session_id);
                    let flags = Arc::clone(&self.flags);
                    let progress_map = Arc::clone(&self.progress_handlers);
                    move || -> Result<HttpRpcOutcome, McpError> {
                        send_streamable_http_request(
                            &client,
                            &url,
                            hmap,
                            &payload,
                            id,
                            &session_id,
                            &flags,
                            &progress_map,
                        )
                    }
                };

                let outcome = if let Some(cancel_flag) = cancel.as_ref() {
                    let (tx, rx) = mpsc::channel();
                    thread::spawn(move || {
                        let _ = tx.send(execute_http());
                    });
                    let deadline = Instant::now() + RPC_TIMEOUT;
                    loop {
                        if cancel_flag.load(Ordering::SeqCst) {
                            self.send_cancelled(id);
                            return Err(McpError::Cancelled(format!(
                                "mcp http request {id} cancelled"
                            )));
                        }
                        let remaining = deadline.saturating_duration_since(Instant::now());
                        if remaining.is_zero() {
                            return Err(McpError::Transport(format!(
                                "mcp http request {id} timed out after {}s",
                                RPC_TIMEOUT.as_secs()
                            )));
                        }
                        match rx.recv_timeout(remaining.min(CANCEL_POLL_INTERVAL)) {
                            Ok(res) => break res?,
                            Err(mpsc::RecvTimeoutError::Timeout) => continue,
                            Err(mpsc::RecvTimeoutError::Disconnected) => {
                                return Err(McpError::Transport(
                                    "mcp http worker disconnected".into(),
                                ));
                            }
                        }
                    }
                } else {
                    execute_http()?
                };

                match outcome {
                    HttpRpcOutcome::Reply(res) => res,
                    HttpRpcOutcome::SessionExpired => {
                        *http.session_id.lock().unwrap() = None;
                        if allow_404_retry && method != "initialize" {
                            let _ = self.initialize_inner(false)?;
                            return self.request_inner(method, params, false, cancel, on_progress);
                        }
                        Err(McpError::Transport("mcp http status 404 Not Found".into()))
                    }
                }
            }
            TransportBackend::ServerSentEvents(sse) => {
                if sse.closed.load(Ordering::SeqCst) {
                    return Err(McpError::Transport("mcp sse stream already closed".into()));
                }
                let bearer = resolve_bearer(&sse.oauth, &sse.server_id)?;
                let (tx, rx) = mpsc::channel();
                sse.pending.lock().unwrap().insert(id, tx);
                let target = sse.post_url.lock().unwrap().clone();
                let hmap = build_headers(
                    &[("content-type", "application/json")],
                    &sse.headers,
                    bearer.as_deref(),
                    None,
                );
                let send_res = sse.client.post(&target).headers(hmap).json(&payload).send();
                match send_res {
                    Ok(resp) if !resp.status().is_success() => {
                        sse.pending.lock().unwrap().remove(&id);
                        return Err(McpError::Transport(format!(
                            "mcp sse POST returned HTTP {}",
                            resp.status()
                        )));
                    }
                    Err(e) => {
                        sse.pending.lock().unwrap().remove(&id);
                        return Err(McpError::Transport(e.to_string()));
                    }
                    Ok(_) => {}
                }
                self.wait_for_pending_reply(&sse.pending, id, rx, cancel.as_deref(), "sse")
            }
        }
    }

    fn wait_for_pending_reply(
        &self,
        pending: &PendingMap,
        id: u64,
        rx: mpsc::Receiver<Result<Value, McpError>>,
        cancel: Option<&AtomicBool>,
        label: &str,
    ) -> Result<Value, McpError> {
        let deadline = Instant::now() + RPC_TIMEOUT;
        loop {
            if cancel.is_some_and(|c| c.load(Ordering::SeqCst)) {
                pending.lock().unwrap().remove(&id);
                self.send_cancelled(id);
                return Err(McpError::Cancelled(format!(
                    "mcp {label} request {id} cancelled"
                )));
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                pending.lock().unwrap().remove(&id);
                return Err(McpError::Transport(format!(
                    "mcp {label} request {id} timed out after {}s",
                    RPC_TIMEOUT.as_secs()
                )));
            }
            let step = if cancel.is_some() {
                remaining.min(CANCEL_POLL_INTERVAL)
            } else {
                remaining
            };
            match rx.recv_timeout(step) {
                Ok(res) => return res,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    pending.lock().unwrap().remove(&id);
                    return Err(McpError::Transport(format!("mcp {label} stream closed")));
                }
            }
        }
    }
}

enum HttpRpcOutcome {
    Reply(Result<Value, McpError>),
    SessionExpired,
}

#[allow(clippy::too_many_arguments)]
fn send_streamable_http_request(
    client: &Client,
    url: &str,
    hmap: HeaderMap,
    payload: &Value,
    id: u64,
    session_id: &Arc<Mutex<Option<String>>>,
    flags: &ListChangedFlags,
    progress_handlers: &ProgressMap,
) -> Result<HttpRpcOutcome, McpError> {
    let resp = client
        .post(url)
        .headers(hmap)
        .json(payload)
        .send()
        .map_err(|e| McpError::Transport(e.to_string()))?;
    let status = resp.status();
    if status.as_u16() == 404 {
        return Ok(HttpRpcOutcome::SessionExpired);
    }
    if !status.is_success() {
        let body = resp.text().unwrap_or_default();
        return Err(McpError::Transport(format!(
            "mcp http status {status}: {}",
            body.chars().take(300).collect::<String>()
        )));
    }
    if let Some(new_sid) = resp
        .headers()
        .get("mcp-session-id")
        .and_then(|v| v.to_str().ok())
    {
        *session_id.lock().unwrap() = Some(new_sid.to_string());
    }
    let is_sse = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(|ct| ct.to_ascii_lowercase().contains("text/event-stream"))
        .unwrap_or(false);
    if is_sse {
        let mut reader = BufReader::new(resp);
        let mut line = String::new();
        let mut data_lines: Vec<String> = Vec::new();
        loop {
            line.clear();
            let n = reader.read_line(&mut line).unwrap_or(0);
            if n == 0 {
                if !data_lines.is_empty() {
                    let joined = data_lines.join("\n");
                    if let Ok(val) = serde_json::from_str::<Value>(joined.trim()) {
                        dispatch_inline_notifications(&val, flags, progress_handlers);
                        if let Some(res) = match_rpc_reply(&val, id) {
                            return Ok(HttpRpcOutcome::Reply(res));
                        }
                    }
                }
                break;
            }
            let trimmed = line.trim_end_matches(&['\r', '\n'][..]);
            if trimmed.is_empty() {
                if !data_lines.is_empty() {
                    let joined = data_lines.join("\n");
                    data_lines.clear();
                    if let Ok(val) = serde_json::from_str::<Value>(joined.trim()) {
                        dispatch_inline_notifications(&val, flags, progress_handlers);
                        if let Some(res) = match_rpc_reply(&val, id) {
                            return Ok(HttpRpcOutcome::Reply(res));
                        }
                    }
                }
            } else if let Some(data) = trimmed.strip_prefix("data:") {
                data_lines.push(data.strip_prefix(' ').unwrap_or(data).to_string());
            }
        }
        return Err(McpError::Transport(
            "streamable_http SSE response lacked matching JSON-RPC reply".into(),
        ));
    }
    let val: Value = resp
        .json()
        .map_err(|e| McpError::Transport(format!("invalid json reply: {e}")))?;
    dispatch_inline_notifications(&val, flags, progress_handlers);
    let reply = match_rpc_reply(&val, id)
        .ok_or_else(|| McpError::Transport("streamable_http JSON lacked matching id".into()))?;
    Ok(HttpRpcOutcome::Reply(reply))
}

fn attach_progress_token(params: &mut Value, token: &str) {
    if let Some(obj) = params.as_object_mut() {
        let meta = obj.entry("_meta").or_insert_with(|| json!({}));
        if let Some(meta_obj) = meta.as_object_mut() {
            meta_obj.insert("progressToken".into(), Value::String(token.to_string()));
        } else {
            *meta = json!({ "progressToken": token });
        }
    }
}

fn text_field(val: &Value, key: &str) -> Option<String> {
    val.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
}

fn decode_resource(val: Value) -> Option<McpResource> {
    let uri = text_field(&val, "uri")?;
    let name = text_field(&val, "name").unwrap_or_else(|| uri.clone());
    Some(McpResource {
        uri,
        name,
        description: text_field(&val, "description"),
        mime_type: text_field(&val, "mimeType"),
    })
}

fn decode_resource_template(val: Value) -> Option<McpResourceTemplate> {
    let uri_template = text_field(&val, "uriTemplate")?;
    let name = text_field(&val, "name").unwrap_or_else(|| uri_template.clone());
    Some(McpResourceTemplate {
        uri_template,
        name,
        description: text_field(&val, "description"),
        mime_type: text_field(&val, "mimeType"),
    })
}

fn decode_resource_content(val: &Value, fallback_uri: &str) -> Option<McpResourceContent> {
    let uri = text_field(val, "uri").unwrap_or_else(|| fallback_uri.to_string());
    let text = val.get("text").and_then(Value::as_str).map(ToString::to_string);
    let blob = val.get("blob").and_then(Value::as_str).map(ToString::to_string);
    if text.is_none() && blob.is_none() {
        return None;
    }
    Some(McpResourceContent {
        uri,
        mime_type: text_field(val, "mimeType"),
        text,
        blob,
    })
}

fn decode_prompt(val: Value) -> Option<McpPrompt> {
    let name = text_field(&val, "name")?;
    let arguments = val
        .get("arguments")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|arg| {
            let arg_name = text_field(arg, "name")?;
            Some(McpPromptArgument {
                name: arg_name,
                description: text_field(arg, "description"),
                required: arg.get("required").and_then(Value::as_bool).unwrap_or(false),
            })
        })
        .collect();
    Some(McpPrompt {
        name,
        description: text_field(&val, "description"),
        arguments,
    })
}

fn decode_prompt_message(val: &Value) -> Option<McpPromptMessage> {
    let role = text_field(val, "role")?;
    let content = val.get("content")?.clone();
    Some(McpPromptMessage { role, content })
}

fn resolve_bearer(
    oauth: &Option<Arc<McpOAuthManager>>,
    server_id: &str,
) -> Result<Option<String>, McpError> {
    match oauth {
        Some(mgr) => mgr.valid_access_token(server_id),
        None => Ok(None),
    }
}

fn build_headers(
    defaults: &[(&str, &str)],
    custom: &BTreeMap<String, String>,
    bearer: Option<&str>,
    session_id: Option<&str>,
) -> HeaderMap {
    let mut map = HeaderMap::new();
    for (k, v) in defaults {
        if let (Ok(name), Ok(val)) = (HeaderName::from_bytes(k.as_bytes()), HeaderValue::from_str(v))
        {
            map.insert(name, val);
        }
    }
    for (k, v) in custom {
        if let (Ok(name), Ok(val)) = (HeaderName::from_bytes(k.as_bytes()), HeaderValue::from_str(v))
        {
            map.insert(name, val);
        }
    }
    if let Some(tok) = bearer {
        if let Ok(val) = HeaderValue::from_str(&format!("Bearer {tok}")) {
            map.insert(HeaderName::from_static("authorization"), val);
        }
    }
    if let Some(sid) = session_id {
        if let Ok(val) = HeaderValue::from_str(sid) {
            map.insert(HeaderName::from_static("mcp-session-id"), val);
        }
    }
    map
}

fn match_rpc_reply(val: &Value, expected_id: u64) -> Option<Result<Value, McpError>> {
    match val {
        Value::Array(items) => items
            .iter()
            .find(|item| item.get("id").and_then(|v| v.as_u64()) == Some(expected_id))
            .map(|item| extract_rpc_result(item.clone())),
        Value::Object(_) => {
            if val.get("id").and_then(|v| v.as_u64()) == Some(expected_id) {
                Some(extract_rpc_result(val.clone()))
            } else {
                None
            }
        }
        _ => None,
    }
}

fn dispatch_inline_notifications(
    val: &Value,
    flags: &ListChangedFlags,
    progress_handlers: &ProgressMap,
) {
    match val {
        Value::Array(items) => {
            for item in items {
                dispatch_inline_notifications(item, flags, progress_handlers);
            }
        }
        Value::Object(_) if val.get("id").is_none() => {
            handle_server_notification(val, flags, progress_handlers);
        }
        _ => {}
    }
}

fn handle_server_notification(
    val: &Value,
    flags: &ListChangedFlags,
    progress_handlers: &ProgressMap,
) {
    let Some(method) = val.get("method").and_then(Value::as_str) else {
        return;
    };
    match method {
        "notifications/tools/list_changed" => {
            flags.tools_dirty.store(true, Ordering::SeqCst);
        }
        "notifications/resources/list_changed" => {
            flags.resources_dirty.store(true, Ordering::SeqCst);
            flags
                .resource_templates_dirty
                .store(true, Ordering::SeqCst);
        }
        "notifications/prompts/list_changed" => {
            flags.prompts_dirty.store(true, Ordering::SeqCst);
        }
        "notifications/progress" => {
            if let Some(params) = val.get("params") {
                dispatch_progress_params(params, progress_handlers);
            }
        }
        _ => {}
    }
}

fn dispatch_progress_params(params: &Value, progress_handlers: &ProgressMap) {
    let token = match params.get("progressToken") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => return,
    };
    let Some(progress) = params.get("progress").and_then(Value::as_f64) else {
        return;
    };
    let total = params.get("total").and_then(Value::as_f64);
    let message = params.get("message").and_then(Value::as_str);

    let cb = progress_handlers.lock().unwrap().get(&token).cloned();
    if let Some(callback) = cb {
        callback(progress, total, message);
    }
}

pub(crate) fn dispatch_rpc_frame(
    pending: &PendingMap,
    flags: &ListChangedFlags,
    progress_handlers: &ProgressMap,
    val: Value,
) {
    if let Value::Array(items) = val {
        for item in items {
            dispatch_rpc_frame(pending, flags, progress_handlers, item);
        }
        return;
    }
    let Some(id) = val.get("id").and_then(|v| v.as_u64()) else {
        handle_server_notification(&val, flags, progress_handlers);
        return;
    };
    if let Some(tx) = pending.lock().unwrap().remove(&id) {
        let _ = tx.send(extract_rpc_result(val));
    }
}

fn fail_all_pending(pending: &PendingMap, reason: &str) {
    let waiters = std::mem::take(&mut *pending.lock().unwrap());
    for (_, tx) in waiters {
        let _ = tx.send(Err(McpError::Transport(reason.to_string())));
    }
}

fn extract_rpc_result(val: Value) -> Result<Value, McpError> {
    if let Some(err) = val.get("error").filter(|v| !v.is_null()) {
        let code = err.get("code").and_then(|v| v.as_i64()).unwrap_or(-32000);
        let message = err
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown json-rpc error")
            .to_string();
        return Err(McpError::Rpc { code, message });
    }
    if let Some(res) = val.get("result") {
        return Ok(res.clone());
    }
    Err(McpError::Transport(
        "json-rpc reply missing both 'result' and 'error'".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::net::TcpListener;

    fn read_http_request(stream: &mut std::net::TcpStream) -> (String, Value) {
        let mut buf = Vec::new();
        let mut tmp = [0u8; 4096];
        let header_end = loop {
            let n = stream.read(&mut tmp).unwrap();
            if n == 0 {
                break buf.len();
            }
            buf.extend_from_slice(&tmp[..n]);
            if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                break pos + 4;
            }
        };
        let header_str = String::from_utf8_lossy(&buf[..header_end]).to_string();
        let content_len = header_str
            .lines()
            .find_map(|l| {
                let (k, v) = l.split_once(':')?;
                k.eq_ignore_ascii_case("content-length")
                    .then(|| v.trim().parse::<usize>().ok())
                    .flatten()
            })
            .unwrap_or(0);
        while buf.len() < header_end + content_len {
            let n = stream.read(&mut tmp).unwrap();
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&tmp[..n]);
        }
        let body = if content_len > 0 {
            serde_json::from_slice(&buf[header_end..header_end + content_len])
                .unwrap_or(Value::Null)
        } else {
            Value::Null
        };
        (header_str, body)
    }

    #[test]
    fn sse_rpc_error_isolates_failing_call_without_killing_concurrent_request() {
        let pending: PendingMap = Arc::new(Mutex::new(BTreeMap::new()));
        let flags = ListChangedFlags::new_dirty();
        let progress: ProgressMap = Arc::new(Mutex::new(BTreeMap::new()));
        let (tx1, rx1) = mpsc::channel();
        let (tx2, rx2) = mpsc::channel();
        pending.lock().unwrap().insert(1, tx1);
        pending.lock().unwrap().insert(2, tx2);

        dispatch_rpc_frame(
            &pending,
            &flags,
            &progress,
            json!({"jsonrpc": "2.0", "id": 1, "error": {"code": -32602, "message": "bad args"}}),
        );
        dispatch_rpc_frame(
            &pending,
            &flags,
            &progress,
            json!({"jsonrpc": "2.0", "id": 2, "result": {"ok": true}}),
        );

        assert!(matches!(
            rx1.recv().unwrap(),
            Err(McpError::Rpc { code: -32602, .. })
        ));
        assert_eq!(rx2.recv().unwrap().unwrap(), json!({"ok": true}));
    }

    #[test]
    fn stdio_transport_clears_parent_secrets_and_loopback_helper_works() {
        assert!(is_loopback_url("http://127.0.0.1:9000/mcp"));
        assert!(is_loopback_url("http://localhost:9000/mcp"));
        assert!(is_loopback_url("http://[::1]:9000/mcp"));
        assert!(!is_loopback_url("https://mcp.example.com/v1"));

        // Verify stdio child process does NOT receive parent API keys while receiving explicit cfg.env
        std::env::set_var("NOLA_TEST_SECRET_API_KEY", "super-secret-leak-check");
        let mut custom_env = BTreeMap::new();
        custom_env.insert("NOLA_EXPLICIT_VAR".into(), "allowed_val".into());
        let cfg = McpServerConfig {
            id: "envcheck".into(),
            name: "EnvCheck".into(),
            transport: McpTransport::Stdio,
            enabled: true,
            command: Some("/bin/sh".into()),
            args: vec![
                "-c".into(),
                r#"read line; printf '{"jsonrpc":"2.0","id":1,"result":{"secret":"%s","explicit":"%s"}}\n' "$NOLA_TEST_SECRET_API_KEY" "$NOLA_EXPLICIT_VAR""#.into(),
            ],
            env: custom_env,
            url: None,
            headers: BTreeMap::new(),
            oauth_client_id: None,
            oauth_scopes: vec![],
        };
        let conn = McpConnection::open(&cfg, None).unwrap();
        let res = conn.request("ping", json!({})).unwrap();
        assert_eq!(res["secret"], "");
        assert_eq!(res["explicit"], "allowed_val");
    }

    #[test]
    fn rpc_result_ignores_null_error_field() {
        let res = extract_rpc_result(
            json!({"jsonrpc": "2.0", "id": 1, "error": null, "result": {"ok": 1}}),
        )
        .unwrap();
        assert_eq!(res, json!({"ok": 1}));
    }

    #[test]
    fn pagination_resources_templates_and_prompts_over_streamable_http() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            for stream in listener.incoming().take(12) {
                let mut stream = stream.unwrap();
                let (_hdrs, req) = read_http_request(&mut stream);
                let method = req["method"].as_str().unwrap_or("");
                let id = req.get("id").cloned();
                let cursor = req.pointer("/params/cursor").and_then(Value::as_str);

                let resp_body = match (method, cursor) {
                    ("initialize", _) => json!({
                        "jsonrpc": "2.0", "id": id,
                        "result": {
                            "protocolVersion": "2024-11-05",
                            "capabilities": { "tools": {}, "resources": {}, "prompts": {} }
                        }
                    })
                    .to_string(),
                    ("notifications/initialized", _) => String::new(),
                    ("tools/list", None) => json!({
                        "jsonrpc": "2.0", "id": id,
                        "result": {
                            "tools": [{"name": "t1", "description": "first", "inputSchema": {"type": "object"}}],
                            "nextCursor": "p2"
                        }
                    })
                    .to_string(),
                    ("tools/list", Some("p2")) => json!({
                        "jsonrpc": "2.0", "id": id,
                        "result": {
                            "tools": [{"name": "t2", "description": "second", "inputSchema": {"type": "object"}}]
                        }
                    })
                    .to_string(),
                    ("resources/list", None) => json!({
                        "jsonrpc": "2.0", "id": id,
                        "result": {
                            "resources": [{"uri": "file:///a.txt", "name": "a.txt", "mimeType": "text/plain"}],
                            "nextCursor": "r2"
                        }
                    })
                    .to_string(),
                    ("resources/list", Some("r2")) => json!({
                        "jsonrpc": "2.0", "id": id,
                        "result": {
                            "resources": [{"uri": "file:///b.txt", "name": "b.txt", "description": "doc B"}]
                        }
                    })
                    .to_string(),
                    ("resources/templates/list", None) => json!({
                        "jsonrpc": "2.0", "id": id,
                        "result": {
                            "resourceTemplates": [{"uriTemplate": "repo://{owner}/{name}", "name": "Repo"}],
                            "nextCursor": "rt2"
                        }
                    })
                    .to_string(),
                    ("resources/templates/list", Some("rt2")) => json!({
                        "jsonrpc": "2.0", "id": id,
                        "result": {
                            "resourceTemplates": [{"uriTemplate": "issue://{id}", "name": "Issue", "mimeType": "application/json"}]
                        }
                    })
                    .to_string(),
                    ("resources/read", _) => json!({
                        "jsonrpc": "2.0", "id": id,
                        "result": {
                            "contents": [{"uri": "file:///a.txt", "mimeType": "text/plain", "text": "hello resource"}]
                        }
                    })
                    .to_string(),
                    ("prompts/list", None) => json!({
                        "jsonrpc": "2.0", "id": id,
                        "result": {
                            "prompts": [{"name": "summarize", "description": "Summarize text", "arguments": [{"name": "topic", "required": true}]}],
                            "nextCursor": "pr2"
                        }
                    })
                    .to_string(),
                    ("prompts/list", Some("pr2")) => json!({
                        "jsonrpc": "2.0", "id": id,
                        "result": {
                            "prompts": [{"name": "review", "arguments": []}]
                        }
                    })
                    .to_string(),
                    ("prompts/get", _) => {
                        assert_eq!(req["params"]["name"], "summarize");
                        assert_eq!(req["params"]["arguments"]["topic"], "rust");
                        json!({
                            "jsonrpc": "2.0", "id": id,
                            "result": {
                                "messages": [{"role": "user", "content": {"type": "text", "text": "Summarize rust"}}]
                            }
                        })
                        .to_string()
                    }
                    other => panic!("unexpected RPC: {other:?}"),
                };

                let http_resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    resp_body.len(),
                    resp_body
                );
                let _ = stream.write_all(http_resp.as_bytes());
                let _ = stream.flush();
            }
        });

        let cfg = McpServerConfig {
            id: "pag".into(),
            name: "Paginated".into(),
            transport: McpTransport::StreamableHttp,
            enabled: true,
            command: None,
            args: vec![],
            env: BTreeMap::new(),
            url: Some(format!("http://{addr}/mcp")),
            headers: BTreeMap::new(),
            oauth_client_id: None,
            oauth_scopes: vec![],
        };
        let conn = McpConnection::open(&cfg, None).unwrap();
        conn.initialize().unwrap();
        assert!(conn.supports_resources());
        assert!(conn.supports_prompts());

        let tools = conn.list_tools().unwrap();
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0]["name"], "t1");
        assert_eq!(tools[1]["name"], "t2");

        let resources = conn.list_resources().unwrap();
        assert_eq!(resources.len(), 2);
        assert_eq!(resources[0].uri, "file:///a.txt");
        assert_eq!(resources[1].description.as_deref(), Some("doc B"));

        let templates = conn.list_resource_templates().unwrap();
        assert_eq!(templates.len(), 2);
        assert_eq!(templates[0].uri_template, "repo://{owner}/{name}");
        assert_eq!(templates[1].uri_template, "issue://{id}");

        let contents = conn.read_resource("file:///a.txt").unwrap();
        assert_eq!(contents.len(), 1);
        assert_eq!(contents[0].text.as_deref(), Some("hello resource"));

        let prompts = conn.list_prompts().unwrap();
        assert_eq!(prompts.len(), 2);
        assert_eq!(prompts[0].name, "summarize");
        assert!(prompts[0].arguments[0].required);
        assert_eq!(prompts[1].name, "review");

        let mut args = BTreeMap::new();
        args.insert("topic".into(), "rust".into());
        let msgs = conn.get_prompt("summarize", Some(args)).unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[0].content["text"], "Summarize rust");

        server.join().unwrap();
    }

    #[test]
    fn stdio_list_changed_notifications_invalidate_cached_lists() {
        let script = r#"
tool_ver=1
res_ver=1
prompt_ver=1
while IFS= read -r line; do
  case "$line" in
    *'"method":"tools/list"'*)
      id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
      printf '{"jsonrpc":"2.0","id":%s,"result":{"tools":[{"name":"tool_v%s","inputSchema":{"type":"object"}}]}}\n' "$id" "$tool_ver"
      if [ "$tool_ver" -eq 1 ]; then
        tool_ver=2
        printf '{"jsonrpc":"2.0","method":"notifications/tools/list_changed"}\n'
      fi
      ;;
    *'"method":"resources/list"'*)
      id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
      printf '{"jsonrpc":"2.0","id":%s,"result":{"resources":[{"uri":"mem://v%s","name":"v%s"}]}}\n' "$id" "$res_ver" "$res_ver"
      if [ "$res_ver" -eq 1 ]; then
        res_ver=2
        printf '{"jsonrpc":"2.0","method":"notifications/resources/list_changed"}\n'
      fi
      ;;
    *'"method":"prompts/list"'*)
      id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
      printf '{"jsonrpc":"2.0","id":%s,"result":{"prompts":[{"name":"prompt_v%s"}]}}\n' "$id" "$prompt_ver"
      if [ "$prompt_ver" -eq 1 ]; then
        prompt_ver=2
        printf '{"jsonrpc":"2.0","method":"notifications/prompts/list_changed"}\n'
      fi
      ;;
  esac
done
"#;
        let cfg = McpServerConfig {
            id: "notify".into(),
            name: "Notify".into(),
            transport: McpTransport::Stdio,
            enabled: true,
            command: Some("/bin/sh".into()),
            args: vec!["-c".into(), script.into()],
            env: BTreeMap::new(),
            url: None,
            headers: BTreeMap::new(),
            oauth_client_id: None,
            oauth_scopes: vec![],
        };
        let conn = McpConnection::open(&cfg, None).unwrap();

        let t1 = conn.list_tools().unwrap();
        assert_eq!(t1[0]["name"], "tool_v1");
        // Give reader thread a moment to ingest the trailing notification
        for _ in 0..40 {
            if conn.tools_list_dirty() {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(conn.tools_list_dirty());
        let t2 = conn.list_tools().unwrap();
        assert_eq!(t2[0]["name"], "tool_v2");
        // No notification emitted on v2 -> stays clean and cached
        assert!(!conn.tools_list_dirty());
        let t2_cached = conn.list_tools().unwrap();
        assert_eq!(t2_cached[0]["name"], "tool_v2");

        let r1 = conn.list_resources().unwrap();
        assert_eq!(r1[0].uri, "mem://v1");
        for _ in 0..40 {
            if conn.resources_list_dirty() {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(conn.resources_list_dirty());
        let r2 = conn.list_resources().unwrap();
        assert_eq!(r2[0].uri, "mem://v2");

        let p1 = conn.list_prompts().unwrap();
        assert_eq!(p1[0].name, "prompt_v1");
        for _ in 0..40 {
            if conn.prompts_list_dirty() {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(conn.prompts_list_dirty());
        let p2 = conn.list_prompts().unwrap();
        assert_eq!(p2[0].name, "prompt_v2");
    }

    #[test]
    fn stdio_progress_and_cancellation_notification() {
        let cancel_marker = tempfile::NamedTempFile::new().unwrap();
        let marker_path = cancel_marker.path().to_string_lossy().to_string();
        let script = format!(
            r#"
while IFS= read -r line; do
  case "$line" in
    *'"method":"tools/call"'*)
      id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
      tok=$(printf '%s' "$line" | sed -n 's/.*"progressToken":"\([^"]*\)".*/\1/p')
      printf '{{"jsonrpc":"2.0","method":"notifications/progress","params":{{"progressToken":"%s","progress":2.0,"total":5.0,"message":"working"}}}}\n' "$tok"
      ;;
    *'"method":"notifications/cancelled"'*)
      printf '%s\n' "$line" > "{marker_path}"
      break
      ;;
  esac
done
"#
        );
        let cfg = McpServerConfig {
            id: "cancel".into(),
            name: "Cancel".into(),
            transport: McpTransport::Stdio,
            enabled: true,
            command: Some("/bin/sh".into()),
            args: vec!["-c".into(), script],
            env: BTreeMap::new(),
            url: None,
            headers: BTreeMap::new(),
            oauth_client_id: None,
            oauth_scopes: vec![],
        };
        let conn = McpConnection::open(&cfg, None).unwrap();

        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_trigger = Arc::clone(&cancel);
        let progress_events = Arc::new(Mutex::new(Vec::<(f64, Option<f64>, Option<String>)>::new()));
        let progress_events_cb = Arc::clone(&progress_events);

        let on_progress: ProgressCallback = Arc::new(move |p, total, msg| {
            progress_events_cb
                .lock()
                .unwrap()
                .push((p, total, msg.map(ToString::to_string)));
            // Trigger cancellation as soon as progress arrives!
            cancel_trigger.store(true, Ordering::SeqCst);
        });

        let err = conn
            .call_tool_with_options("slow_op", json!({"x": 1}), Some(cancel), Some(on_progress))
            .unwrap_err();
        assert!(matches!(err, McpError::Cancelled(_)));

        let events = progress_events.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0], (2.0, Some(5.0), Some("working".into())));

        // Verify the child received `notifications/cancelled` with requestId & reason
        let mut recorded = String::new();
        for _ in 0..40 {
            recorded = std::fs::read_to_string(&marker_path).unwrap_or_default();
            if !recorded.trim().is_empty() {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        let cancelled_frame: Value = serde_json::from_str(recorded.trim()).unwrap();
        assert_eq!(cancelled_frame["method"], "notifications/cancelled");
        assert_eq!(cancelled_frame["params"]["requestId"], 1);
        assert!(cancelled_frame["params"]["reason"]
            .as_str()
            .unwrap_or("")
            .contains("Cancelled"));
    }

    #[test]
    fn streamable_http_sse_progress_and_cancellation() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let cancelled_received = Arc::new(AtomicBool::new(false));
        let cancelled_flag_srv = Arc::clone(&cancelled_received);

        let server = thread::spawn(move || {
            let mut handles = Vec::new();
            for stream in listener.incoming().take(2) {
                let mut stream = stream.unwrap();
                let cancelled_flag = Arc::clone(&cancelled_flag_srv);
                handles.push(thread::spawn(move || {
                    let (_hdrs, req) = read_http_request(&mut stream);
                    let method = req["method"].as_str().unwrap_or("");
                    if method == "tools/call" {
                        let tok = req
                            .pointer("/params/_meta/progressToken")
                            .and_then(Value::as_str)
                            .unwrap()
                            .to_string();
                        let frame = format!(
                            "event: message\ndata: {}\n\n",
                            json!({
                                "jsonrpc": "2.0",
                                "method": "notifications/progress",
                                "params": {
                                    "progressToken": tok,
                                    "progress": 1.0,
                                    "total": 4.0,
                                    "message": "streaming"
                                }
                            })
                        );
                        let headers = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n";
                        let _ = stream.write_all(headers.as_bytes());
                        let _ = stream.write_all(frame.as_bytes());
                        let _ = stream.flush();
                        // Hold connection open briefly until cancelled arrives
                        for _ in 0..40 {
                            if cancelled_flag.load(Ordering::SeqCst) {
                                break;
                            }
                            thread::sleep(Duration::from_millis(20));
                        }
                    } else if method == "notifications/cancelled" {
                        assert_eq!(req["params"]["requestId"], 1);
                        cancelled_flag.store(true, Ordering::SeqCst);
                        let _ = stream.write_all(
                            b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                        );
                    }
                }));
            }
            for h in handles {
                let _ = h.join();
            }
        });

        let cfg = McpServerConfig {
            id: "http-cancel".into(),
            name: "HttpCancel".into(),
            transport: McpTransport::StreamableHttp,
            enabled: true,
            command: None,
            args: vec![],
            env: BTreeMap::new(),
            url: Some(format!("http://{addr}/mcp")),
            headers: BTreeMap::new(),
            oauth_client_id: None,
            oauth_scopes: vec![],
        };
        let conn = McpConnection::open(&cfg, None).unwrap();

        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_trigger = Arc::clone(&cancel);
        let progress_seen = Arc::new(Mutex::new(Vec::new()));
        let progress_seen_cb = Arc::clone(&progress_seen);
        let on_progress: ProgressCallback = Arc::new(move |p, t, m| {
            progress_seen_cb
                .lock()
                .unwrap()
                .push((p, t, m.map(ToString::to_string)));
            cancel_trigger.store(true, Ordering::SeqCst);
        });

        let err = conn
            .call_tool_with_options("slow_http", json!({}), Some(cancel), Some(on_progress))
            .unwrap_err();
        assert!(matches!(err, McpError::Cancelled(_)));
        assert!(cancelled_received.load(Ordering::SeqCst));
        assert_eq!(
            *progress_seen.lock().unwrap(),
            vec![(1.0, Some(4.0), Some("streaming".into()))]
        );
        server.join().unwrap();
    }
}
