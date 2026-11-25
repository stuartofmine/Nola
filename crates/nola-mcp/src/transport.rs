// SPDX-License-Identifier: Apache-2.0
//! MCP JSON-RPC 2.0 transports: `Stdio`, `StreamableHttp`, and `ServerSentEvents`
//! with finite timeouts, dedicated reader threads, proper `notifications/initialized`,
//! dynamic OAuth bearer lookup, and non-fatal per-request error isolation.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;

use nola_types::{McpServerConfig, McpTransport};
use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use serde_json::{json, Value};

use crate::oauth::McpOAuthManager;
use crate::McpError;

const RPC_TIMEOUT: Duration = Duration::from_secs(30);
const SSE_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

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

fn is_loopback_url(raw: &str) -> bool {
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
    session_id: Mutex<Option<String>>,
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

/// Active JSON-RPC 2.0 connection to a single MCP server.
pub struct McpConnection {
    next_id: AtomicU64,
    backend: TransportBackend,
}

impl McpConnection {
    pub fn open(
        cfg: &McpServerConfig,
        oauth: Option<Arc<McpOAuthManager>>,
    ) -> Result<Self, McpError> {
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
                thread::spawn(move || {
                    let mut reader = BufReader::new(stdout);
                    let mut line = String::new();
                    while let Ok(n) = reader.read_line(&mut line) {
                        if n == 0 {
                            break;
                        }
                        if let Ok(val) = serde_json::from_str::<Value>(line.trim()) {
                            dispatch_rpc_frame(&pending_bg, val);
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
                    session_id: Mutex::new(None),
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
                let sse_client =
                    build_mcp_http_client(&url, None, Some(Duration::from_secs(10)))?;
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
                let base_url = url.clone();
                thread::spawn(move || {
                    let mut reader = BufReader::new(resp);
                    let mut line = String::new();
                    let mut current_event = String::new();
                    let mut data_lines: Vec<String> = Vec::new();
                    let mut ep_tx = Some(ep_tx);

                    let mut flush_event = |ev: &mut String, data: &mut Vec<String>| {
                        if data.is_empty() {
                            ev.clear();
                            return;
                        }
                        let joined = data.join("\n");
                        let d = joined.trim();
                        if ev == "endpoint" && !d.is_empty() {
                            if let Ok(base) = url::Url::parse(&base_url) {
                                if let Ok(resolved) = base.join(d) {
                                    let resolved_str = resolved.to_string();
                                    *post_url_bg.lock().unwrap() = resolved_str.clone();
                                    if let Some(tx) = ep_tx.take() {
                                        let _ = tx.send(Ok(resolved_str));
                                    }
                                }
                            }
                        } else if let Ok(val) = serde_json::from_str::<Value>(d) {
                            dispatch_rpc_frame(&pending_bg, val);
                        }
                        ev.clear();
                        data.clear();
                    };

                    while let Ok(n) = reader.read_line(&mut line) {
                        if n == 0 {
                            flush_event(&mut current_event, &mut data_lines);
                            break;
                        }
                        let trimmed = line.trim_end_matches(&['\r', '\n'][..]);
                        if trimmed.is_empty() {
                            flush_event(&mut current_event, &mut data_lines);
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
                    Ok(Err(e)) => return Err(McpError::Transport(e)),
                    Err(_) => {
                        return Err(McpError::Transport(
                            "timed out waiting for MCP SSE endpoint event".into(),
                        ))
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
        })
    }

    pub fn is_healthy(&self) -> bool {
        match &self.backend {
            TransportBackend::Stdio(s) => !s.closed.load(Ordering::SeqCst),
            TransportBackend::ServerSentEvents(s) => !s.closed.load(Ordering::SeqCst),
            TransportBackend::StreamableHttp(_) => true,
        }
    }

    pub fn initialize(&self) -> Result<Value, McpError> {
        let res = self.request(
            "initialize",
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "nola-v2", "version": "2.0.0"}
            }),
        )?;
        // Mandatory MCP handshake notification
        let _ = self.notify("notifications/initialized", json!({}));
        Ok(res)
    }

    pub fn list_tools(&self) -> Result<Vec<Value>, McpError> {
        let res = self.request("tools/list", json!({}))?;
        Ok(res
            .get("tools")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default())
    }

    pub fn call_tool(&self, name: &str, arguments: Value) -> Result<Value, McpError> {
        self.request(
            "tools/call",
            json!({
                "name": name,
                "arguments": arguments
            }),
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
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let payload = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
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
                match rx.recv_timeout(RPC_TIMEOUT) {
                    Ok(res) => res,
                    Err(_) => {
                        s.pending.lock().unwrap().remove(&id);
                        Err(McpError::Transport(format!(
                            "mcp stdio request {id} timed out after {}s",
                            RPC_TIMEOUT.as_secs()
                        )))
                    }
                }
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
                let resp = http
                    .client
                    .post(&http.url)
                    .headers(hmap)
                    .json(&payload)
                    .send()
                    .map_err(|e| McpError::Transport(e.to_string()))?;
                let status = resp.status();
                if status.as_u16() == 404 {
                    *http.session_id.lock().unwrap() = None;
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
                    *http.session_id.lock().unwrap() = Some(new_sid.to_string());
                }
                let is_sse = resp
                    .headers()
                    .get("content-type")
                    .and_then(|v| v.to_str().ok())
                    .map(|ct| ct.to_ascii_lowercase().contains("text/event-stream"))
                    .unwrap_or(false);
                if is_sse {
                    // Stream line-by-line so keep-alive SSE streams return immediately on reply!
                    let mut reader = BufReader::new(resp);
                    let mut line = String::new();
                    while let Ok(n) = reader.read_line(&mut line) {
                        if n == 0 {
                            break;
                        }
                        if let Some(data) = line.trim().strip_prefix("data:") {
                            if let Ok(val) = serde_json::from_str::<Value>(data.trim()) {
                                if let Some(res) = match_rpc_reply(&val, id) {
                                    return res;
                                }
                            }
                        }
                        line.clear();
                    }
                    return Err(McpError::Transport(
                        "streamable_http SSE response lacked matching JSON-RPC reply".into(),
                    ));
                }
                let val: Value = resp
                    .json()
                    .map_err(|e| McpError::Transport(format!("invalid json reply: {e}")))?;
                match_rpc_reply(&val, id).ok_or_else(|| {
                    McpError::Transport("streamable_http JSON lacked matching id".into())
                })?
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
                match rx.recv_timeout(RPC_TIMEOUT) {
                    Ok(res) => res,
                    Err(_) => {
                        sse.pending.lock().unwrap().remove(&id);
                        Err(McpError::Transport(format!(
                            "mcp sse request {id} timed out after {}s",
                            RPC_TIMEOUT.as_secs()
                        )))
                    }
                }
            }
        }
    }
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

pub(crate) fn dispatch_rpc_frame(pending: &PendingMap, val: Value) {
    if let Value::Array(items) = val {
        for item in items {
            dispatch_rpc_frame(pending, item);
        }
        return;
    }
    let Some(id) = val.get("id").and_then(|v| v.as_u64()) else {
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
    if let Some(err) = val.get("error") {
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

    #[test]
    fn sse_rpc_error_isolates_failing_call_without_killing_concurrent_request() {
        let pending: PendingMap = Arc::new(Mutex::new(BTreeMap::new()));
        let (tx1, rx1) = mpsc::channel();
        let (tx2, rx2) = mpsc::channel();
        pending.lock().unwrap().insert(1, tx1);
        pending.lock().unwrap().insert(2, tx2);

        dispatch_rpc_frame(
            &pending,
            json!({"jsonrpc": "2.0", "id": 1, "error": {"code": -32602, "message": "bad args"}}),
        );
        dispatch_rpc_frame(
            &pending,
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
}
