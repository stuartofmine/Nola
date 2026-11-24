// SPDX-License-Identifier: Apache-2.0
//! MCP JSON-RPC 2.0 transports: `Stdio`, `StreamableHttp`, and `ServerSentEvents`
//! with finite timeouts and non-fatal per-request error isolation.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;

use nola_types::{McpServerConfig, McpTransport};
use reqwest::blocking::Client;
use serde_json::{json, Value};

use crate::McpError;

const RPC_TIMEOUT: Duration = Duration::from_secs(30);

enum TransportBackend {
    Stdio(Mutex<StdioState>),
    StreamableHttp(HttpState),
    ServerSentEvents(SseState),
}

struct StdioState {
    _child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

struct HttpState {
    client: Client,
    url: String,
    headers: BTreeMap<String, String>,
    bearer: Option<String>,
    session_id: Mutex<Option<String>>,
}

struct SseState {
    client: Client,
    post_url: Mutex<String>,
    headers: BTreeMap<String, String>,
    bearer: Option<String>,
    pending: Arc<Mutex<BTreeMap<u64, mpsc::Sender<Result<Value, McpError>>>>>,
}

/// Active JSON-RPC 2.0 connection to a single MCP server.
pub struct McpConnection {
    next_id: AtomicU64,
    backend: TransportBackend,
}

impl McpConnection {
    pub fn open(cfg: &McpServerConfig, bearer: Option<&str>) -> Result<Self, McpError> {
        let backend = match cfg.transport {
            McpTransport::Stdio => {
                let cmd_name = cfg
                    .command
                    .as_deref()
                    .filter(|s| !s.trim().is_empty())
                    .ok_or_else(|| McpError::Transport("stdio mcp server missing command".into()))?;
                let mut cmd = Command::new(cmd_name);
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
                TransportBackend::Stdio(Mutex::new(StdioState {
                    _child: child,
                    stdin,
                    stdout: BufReader::new(stdout),
                }))
            }
            McpTransport::StreamableHttp => {
                let url = cfg
                    .url
                    .clone()
                    .filter(|s| !s.trim().is_empty())
                    .ok_or_else(|| McpError::Transport("streamable_http server missing url".into()))?;
                let client = Client::builder()
                    .timeout(RPC_TIMEOUT)
                    .build()
                    .map_err(|e| McpError::Transport(e.to_string()))?;
                TransportBackend::StreamableHttp(HttpState {
                    client,
                    url,
                    headers: cfg.headers.clone(),
                    bearer: bearer.map(ToString::to_string),
                    session_id: Mutex::new(None),
                })
            }
            McpTransport::ServerSentEvents => {
                let url = cfg
                    .url
                    .clone()
                    .filter(|s| !s.trim().is_empty())
                    .ok_or_else(|| McpError::Transport("sse server missing url".into()))?;
                let client = Client::builder()
                    .timeout(RPC_TIMEOUT)
                    .build()
                    .map_err(|e| McpError::Transport(e.to_string()))?;
                let pending: Arc<Mutex<BTreeMap<u64, mpsc::Sender<Result<Value, McpError>>>>> =
                    Arc::new(Mutex::new(BTreeMap::new()));
                let post_url = Arc::new(Mutex::new(url.clone()));

                // Start background SSE reader routing JSON-RPC replies by id without tearing
                // down sibling in-flight calls on individual JSON-RPC errors.
                let sse_client = Client::builder()
                    .connect_timeout(Duration::from_secs(10))
                    .build()
                    .map_err(|e| McpError::Transport(e.to_string()))?;
                let mut req = sse_client.get(&url).header("accept", "text/event-stream");
                for (k, v) in &cfg.headers {
                    req = req.header(k.as_str(), v.as_str());
                }
                if let Some(tok) = bearer {
                    req = req.header("authorization", format!("Bearer {tok}"));
                }
                let resp = req.send().map_err(|e| McpError::Transport(e.to_string()))?;
                let pending_bg = Arc::clone(&pending);
                let post_url_bg = Arc::clone(&post_url);
                let base_url = url.clone();
                thread::spawn(move || {
                    let mut reader = BufReader::new(resp);
                    let mut line = String::new();
                    let mut current_event = String::new();
                    while let Ok(n) = reader.read_line(&mut line) {
                        if n == 0 {
                            break;
                        }
                        let trimmed = line.trim_end_matches(&['\r', '\n'][..]);
                        if let Some(ev) = trimmed.strip_prefix("event:") {
                            current_event = ev.trim().to_string();
                        } else if let Some(data) = trimmed.strip_prefix("data:") {
                            let d = data.trim();
                            if current_event == "endpoint" && !d.is_empty() {
                                if let Ok(base) = url::Url::parse(&base_url) {
                                    if let Ok(resolved) = base.join(d) {
                                        *post_url_bg.lock().unwrap() = resolved.to_string();
                                    }
                                }
                            } else if let Ok(val) = serde_json::from_str::<Value>(d) {
                                dispatch_sse_rpc(&pending_bg, val);
                            }
                        } else if trimmed.is_empty() {
                            current_event.clear();
                        }
                        line.clear();
                    }
                });

                let resolved_post = post_url.lock().unwrap().clone();
                TransportBackend::ServerSentEvents(SseState {
                    client,
                    post_url: Mutex::new(resolved_post),
                    headers: cfg.headers.clone(),
                    bearer: bearer.map(ToString::to_string),
                    pending,
                })
            }
        };

        Ok(Self {
            next_id: AtomicU64::new(1),
            backend,
        })
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

    pub fn request(&self, method: &str, params: Value) -> Result<Value, McpError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let payload = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        });

        match &self.backend {
            TransportBackend::Stdio(state) => {
                let mut g = state.lock().unwrap();
                let line = serde_json::to_string(&payload)?;
                writeln!(g.stdin, "{line}")?;
                g.stdin.flush()?;
                let mut resp_line = String::new();
                loop {
                    resp_line.clear();
                    let n = g.stdout.read_line(&mut resp_line)?;
                    if n == 0 {
                        return Err(McpError::Transport("mcp stdio closed unexpectedly".into()));
                    }
                    let Ok(val) = serde_json::from_str::<Value>(resp_line.trim()) else {
                        continue;
                    };
                    if val.get("id").and_then(|v| v.as_u64()) == Some(id) {
                        return extract_rpc_result(val);
                    }
                }
            }
            TransportBackend::StreamableHttp(http) => {
                let mut req = http
                    .client
                    .post(&http.url)
                    .header("content-type", "application/json")
                    .header("accept", "application/json, text/event-stream")
                    .json(&payload);
                for (k, v) in &http.headers {
                    req = req.header(k.as_str(), v.as_str());
                }
                if let Some(tok) = &http.bearer {
                    req = req.header("authorization", format!("Bearer {tok}"));
                }
                if let Some(sid) = http.session_id.lock().unwrap().as_deref() {
                    req = req.header("mcp-session-id", sid);
                }
                let resp = req.send().map_err(|e| McpError::Transport(e.to_string()))?;
                if let Some(sid) = resp
                    .headers()
                    .get("mcp-session-id")
                    .and_then(|v| v.to_str().ok())
                {
                    *http.session_id.lock().unwrap() = Some(sid.to_string());
                }
                if !resp.status().is_success() {
                    return Err(McpError::Transport(format!(
                        "mcp http status {}",
                        resp.status()
                    )));
                }
                let body = resp.text().map_err(|e| McpError::Transport(e.to_string()))?;
                // Support both direct JSON and single-message SSE frame responses
                if let Ok(val) = serde_json::from_str::<Value>(&body) {
                    return extract_rpc_result(val);
                }
                for line in body.lines() {
                    if let Some(data) = line.strip_prefix("data:") {
                        if let Ok(val) = serde_json::from_str::<Value>(data.trim()) {
                            if val.get("id").and_then(|v| v.as_u64()) == Some(id) {
                                return extract_rpc_result(val);
                            }
                        }
                    }
                }
                Err(McpError::Transport(
                    "streamable_http response lacked matching JSON-RPC reply".into(),
                ))
            }
            TransportBackend::ServerSentEvents(sse) => {
                let (tx, rx) = mpsc::channel();
                sse.pending.lock().unwrap().insert(id, tx);
                let target = sse.post_url.lock().unwrap().clone();
                let mut req = sse
                    .client
                    .post(&target)
                    .header("content-type", "application/json")
                    .json(&payload);
                for (k, v) in &sse.headers {
                    req = req.header(k.as_str(), v.as_str());
                }
                if let Some(tok) = &sse.bearer {
                    req = req.header("authorization", format!("Bearer {tok}"));
                }
                if let Err(e) = req.send() {
                    sse.pending.lock().unwrap().remove(&id);
                    return Err(McpError::Transport(e.to_string()));
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

/// Dispatch an incoming SSE JSON-RPC frame to ONLY its matching caller (non-fatal isolation).
pub(crate) fn dispatch_sse_rpc(
    pending: &Mutex<BTreeMap<u64, mpsc::Sender<Result<Value, McpError>>>>,
    val: Value,
) {
    let Some(id) = val.get("id").and_then(|v| v.as_u64()) else {
        return;
    };
    if let Some(tx) = pending.lock().unwrap().remove(&id) {
        let _ = tx.send(extract_rpc_result(val));
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
    Ok(val.get("result").cloned().unwrap_or(Value::Null))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_rpc_error_isolates_failing_call_without_killing_concurrent_request() {
        let pending: Mutex<BTreeMap<u64, mpsc::Sender<Result<Value, McpError>>>> =
            Mutex::new(BTreeMap::new());
        let (tx1, rx1) = mpsc::channel();
        let (tx2, rx2) = mpsc::channel();
        pending.lock().unwrap().insert(1, tx1);
        pending.lock().unwrap().insert(2, tx2);

        dispatch_sse_rpc(
            &pending,
            json!({"jsonrpc": "2.0", "id": 1, "error": {"code": -32602, "message": "bad args"}}),
        );
        dispatch_sse_rpc(
            &pending,
            json!({"jsonrpc": "2.0", "id": 2, "result": {"ok": true}}),
        );

        assert!(matches!(rx1.recv().unwrap(), Err(McpError::Rpc { code: -32602, .. })));
        assert_eq!(rx2.recv().unwrap().unwrap(), json!({"ok": true}));
    }
}
