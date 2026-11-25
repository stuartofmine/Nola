// SPDX-License-Identifier: Apache-2.0
//! Managed Browser Automation (`browser` tool) & Chrome Native Bridge queue:
//! - Strict URL scheme validation (`http`/`https` only — blocks `file://`, `localhost.evil.com`)
//! - Full RFC 6455 fragmented WebSocket frame reassembly + PING/PONG + CLOSE + leftover upgrade buffer
//! - Passes CDP payload over `stdin` (never leaking secrets in `/proc/cmdline`!) with finite timeout
//! - Scrubs sensitive env vars (`*_API_KEY`, `*_TOKEN`, `*_SECRET`, `ANTHROPIC_*`, `OPENAI_*`, `AWS_*`)
//!   before spawning the Python CDP helper subprocess
//! - Supports `target="attached"` routing through a shared `NativeBrowserBridge`
//! - Checks child liveness via `try_wait()`, surfaces JS `exceptionDetails` & `{ok:false}` as errors
//! - Bounded `NativeBrowserBridge` queue.

use std::collections::{BTreeMap, VecDeque};
use std::fs;
use std::io::Write;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use nola_types::{
    new_id, sha256_hex, CapabilityDomain, MediaAttachment, ToolExecutionResult, ToolSchema,
    ToolSpec, ToolTier,
};
use reqwest::blocking::Client;
use serde_json::{json, Value};
use url::{Host, Url};

use crate::{ToolContext, ToolHandler};

const MAX_BRIDGE_ITEMS: usize = 256;

/// Returns true iff an environment variable name looks like a secret/credential
/// that should be scrubbed before spawning external helper subprocesses.
pub(crate) fn is_sensitive_env_key(key: &str) -> bool {
    let upper = key.to_ascii_uppercase();
    upper.ends_with("_API_KEY")
        || upper.ends_with("_TOKEN")
        || upper.ends_with("_SECRET")
        || upper.starts_with("ANTHROPIC_")
        || upper.starts_with("OPENAI_")
        || upper.starts_with("AWS_")
}

fn scrub_sensitive_env(cmd: &mut Command) {
    for (k, _) in std::env::vars_os() {
        if let Some(s) = k.to_str() {
            if is_sensitive_env_key(s) {
                cmd.env_remove(s);
            }
        }
    }
}

/// Queue backing the Chrome native-messaging host (`/api/native-browser/host/poll` & `/result`).
#[derive(Default)]
pub struct NativeBrowserBridge {
    pending: Mutex<VecDeque<Value>>,
    results: Mutex<BTreeMap<String, Value>>,
    last_host_poll_ms: Mutex<Option<i64>>,
}

impl NativeBrowserBridge {
    pub fn enqueue(&self, command: Value) -> String {
        let id = new_id();
        let envelope = match command {
            Value::Object(mut map) => {
                map.insert("command_id".into(), Value::String(id.clone()));
                Value::Object(map)
            }
            other => json!({"command_id": id, "payload": other}),
        };
        let mut q = self.pending.lock().unwrap();
        while q.len() >= MAX_BRIDGE_ITEMS {
            q.pop_front();
        }
        q.push_back(envelope);
        id
    }

    /// Alias for `enqueue` that also waits up to `timeout` for the attached browser host to reply.
    pub fn queue_command(&self, command: Value, timeout: Duration) -> Result<Value, String> {
        let id = self.enqueue(command);
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(res) = self.take_result(&id) {
                return Ok(res);
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "attached browser bridge timed out after {}ms waiting for command {id}",
                    timeout.as_millis()
                ));
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    pub fn poll_next(&self) -> Option<Value> {
        *self.last_host_poll_ms.lock().unwrap() = Some(nola_types::now_ms());
        self.pending.lock().unwrap().pop_front()
    }

    pub fn submit_result(&self, command_id: String, result: Value) {
        let mut r = self.results.lock().unwrap();
        if r.len() >= MAX_BRIDGE_ITEMS {
            if let Some(first_key) = r.keys().next().cloned() {
                r.remove(&first_key);
            }
        }
        r.insert(command_id, result);
    }

    pub fn take_result(&self, command_id: &str) -> Option<Value> {
        self.results.lock().unwrap().remove(command_id)
    }

    pub fn status(&self) -> Value {
        let last = *self.last_host_poll_ms.lock().unwrap();
        let connected = last
            .map(|t| nola_types::now_ms().saturating_sub(t) < 30_000)
            .unwrap_or(false);
        json!({
            "connected": connected,
            "pending_commands": self.pending.lock().unwrap().len(),
            "last_poll_ms": last
        })
    }
}

struct ManagedBrowserSession {
    child: Option<Child>,
    cdp_port: u16,
    user_data_dir: PathBuf,
}

impl ManagedBrowserSession {
    fn is_alive(&mut self) -> bool {
        match self.child.as_mut() {
            Some(ch) => matches!(ch.try_wait(), Ok(None)),
            None => false,
        }
    }
}

impl Drop for ManagedBrowserSession {
    fn drop(&mut self) {
        if let Some(mut ch) = self.child.take() {
            let _ = ch.kill();
            let _ = ch.wait();
        }
        let _ = fs::remove_dir_all(&self.user_data_dir);
    }
}

/// Manages a local loopback Chromium CDP session plus the native extension bridge.
#[derive(Default)]
pub struct BrowserManager {
    session: Mutex<Option<ManagedBrowserSession>>,
    pub native_bridge: Arc<NativeBrowserBridge>,
}

impl BrowserManager {
    /// Construct a `BrowserManager` sharing an externally-owned `NativeBrowserBridge`
    /// (e.g. shared with `web_server.rs` so `target="attached"` routes to the live bridge).
    pub fn with_native_bridge(bridge: Arc<NativeBrowserBridge>) -> Self {
        Self {
            session: Mutex::new(None),
            native_bridge: bridge,
        }
    }

    pub fn native_bridge(&self) -> &Arc<NativeBrowserBridge> {
        &self.native_bridge
    }

    pub fn status(&self) -> Value {
        let mut g = self.session.lock().unwrap();
        if let Some(s) = g.as_mut() {
            if !s.is_alive() {
                *g = None;
            }
        }
        match g.as_ref() {
            Some(s) => json!({
                "running": true,
                "cdp_endpoint": format!("http://127.0.0.1:{}", s.cdp_port),
                "native_bridge": self.native_bridge.status()
            }),
            None => json!({
                "running": false,
                "native_bridge": self.native_bridge.status()
            }),
        }
    }

    fn start(&self, headless: bool) -> Result<Value, String> {
        let mut g = self.session.lock().unwrap();
        if let Some(s) = g.as_mut() {
            if s.is_alive() {
                return Ok(json!({
                    "running": true,
                    "cdp_endpoint": format!("http://127.0.0.1:{}", s.cdp_port)
                }));
            }
            *g = None;
        }
        let bin = find_chromium_binary()
            .ok_or_else(|| "no Chrome/Chromium binary found on PATH".to_string())?;
        let port = allocate_loopback_port()?;
        let profile = std::env::temp_dir().join(format!("nola-v2-browser-{}-{}", port, new_id()));
        let _ = fs::create_dir_all(&profile);

        let mut cmd = Command::new(&bin);
        cmd.arg(format!("--remote-debugging-port={port}"))
            .arg("--remote-debugging-address=127.0.0.1")
            .arg(format!("--user-data-dir={}", profile.display()))
            .arg("--no-first-run")
            .arg("--no-default-browser-check")
            .arg("--disable-gpu")
            .arg("--no-sandbox");
        if headless {
            cmd.arg("--headless=new");
        }
        cmd.arg("about:blank")
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        let child = cmd.spawn().map_err(|e| {
            let _ = fs::remove_dir_all(&profile);
            format!("failed to launch {bin}: {e}")
        })?;

        let client = Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .map_err(|e| e.to_string())?;
        let version_url = format!("http://127.0.0.1:{port}/json/version");
        let deadline = Instant::now() + Duration::from_secs(8);
        let mut ready = false;
        while Instant::now() < deadline {
            if client
                .get(&version_url)
                .send()
                .map(|r| r.status().is_success())
                .unwrap_or(false)
            {
                ready = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(150));
        }
        if !ready {
            let mut ch = child;
            let _ = ch.kill();
            let _ = ch.wait();
            let _ = fs::remove_dir_all(&profile);
            return Err(format!(
                "browser launched ({bin}) but CDP port {port} did not become ready"
            ));
        }

        *g = Some(ManagedBrowserSession {
            child: Some(child),
            cdp_port: port,
            user_data_dir: profile,
        });
        Ok(json!({
            "running": true,
            "binary": bin,
            "cdp_endpoint": format!("http://127.0.0.1:{port}")
        }))
    }

    fn stop(&self) -> Value {
        let mut g = self.session.lock().unwrap();
        let was_running = g.take().is_some();
        json!({"stopped": was_running})
    }

    fn cdp_port(&self) -> Result<u16, String> {
        let mut g = self.session.lock().unwrap();
        if let Some(s) = g.as_mut() {
            if !s.is_alive() {
                *g = None;
            }
        }
        g.as_ref()
            .map(|s| s.cdp_port)
            .ok_or_else(|| "browser is not running; call browser with action='start' first".into())
    }

    fn list_tabs(&self) -> Result<Vec<Value>, String> {
        let port = self.cdp_port()?;
        let client = Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .map_err(|e| e.to_string())?;
        let resp = client
            .get(format!("http://127.0.0.1:{port}/json/list"))
            .send()
            .map_err(|e| format!("cdp list failed: {e}"))?;
        let tabs: Vec<Value> = resp.json().map_err(|e| e.to_string())?;
        Ok(tabs
            .into_iter()
            .filter(|t| t.get("type").and_then(|v| v.as_str()) == Some("page"))
            .collect())
    }

    fn run_cdp_python_script(
        &self,
        op: &str,
        params: &Value,
        screenshot_out: Option<&Path>,
    ) -> Result<Value, String> {
        let tabs = self.list_tabs()?;
        let ws_url = tabs
            .first()
            .and_then(|t| t.get("webSocketDebuggerUrl"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| "no active browser page target".to_string())?;

        // RFC 6455 compliant stdlib-only Python CDP WebSocket client (reads request from stdin!)
        let py = r#"
import base64, json, os, socket, struct, sys
from urllib.parse import urlparse

req = json.loads(sys.stdin.read())
ws_url = req['ws_url']
u = urlparse(ws_url)
sock = socket.create_connection((u.hostname, u.port), timeout=15)
key = base64.b64encode(os.urandom(16)).decode()
hdr = f"GET {u.path} HTTP/1.1\r\nHost: {u.hostname}:{u.port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
sock.sendall(hdr.encode())
resp = b""
while b"\r\n\r\n" not in resp:
    chunk = sock.recv(4096)
    if not chunk: raise EOFError("ws upgrade closed")
    resp += chunk
head, leftover = resp.split(b"\r\n\r\n", 1)
if b"101 " not in head.split(b"\r\n", 1)[0]:
    raise RuntimeError("ws upgrade rejected: " + head[:200].decode("utf-8", "replace"))
rx_buf = bytearray(leftover)

def ws_send_frame(opcode, data):
    mask = os.urandom(4)
    header = bytearray([0x80 | opcode])
    n = len(data)
    if n < 126: header.append(0x80 | n)
    elif n < 65536: header.extend([0x80 | 126, (n >> 8) & 0xff, n & 0xff])
    else: header.extend([0x80 | 127] + list(struct.pack("!Q", n)))
    header.extend(mask)
    masked = bytes(b ^ mask[i % 4] for i, b in enumerate(data))
    sock.sendall(bytes(header) + masked)

def ws_send(obj):
    ws_send_frame(1, json.dumps(obj).encode())

def recv_exact(n):
    while len(rx_buf) < n:
        c = sock.recv(max(4096, n - len(rx_buf)))
        if not c: raise EOFError("ws closed")
        rx_buf.extend(c)
    out = bytes(rx_buf[:n])
    del rx_buf[:n]
    return out

def ws_recv():
    fragments = bytearray()
    in_msg = False
    while True:
        b1, b2 = recv_exact(2)
        fin = (b1 & 0x80) != 0
        op = b1 & 0x0f
        n = b2 & 0x7f
        if n == 126: n = struct.unpack("!H", recv_exact(2))[0]
        elif n == 127: n = struct.unpack("!Q", recv_exact(8))[0]
        payload = recv_exact(n)
        if op == 9:
            ws_send_frame(10, payload)
            continue
        if op == 8:
            raise EOFError("ws server closed")
        if op == 1:
            fragments = bytearray(payload)
            in_msg = True
        elif op == 0 and in_msg:
            fragments.extend(payload)
        else:
            continue
        if fin and in_msg:
            return json.loads(fragments.decode("utf-8", "replace"))

mid = 0
def cdp(method, **p):
    global mid
    mid += 1
    ws_send({"id": mid, "method": method, "params": p})
    while True:
        msg = ws_recv()
        if msg.get("id") == mid:
            if "error" in msg: raise RuntimeError(str(msg["error"]))
            res = msg.get("result", {})
            if isinstance(res, dict) and res.get("exceptionDetails"):
                raise RuntimeError("JS exception: " + str(res["exceptionDetails"]))
            return res

op = req["op"]
params = req.get("params", {})
out = {}
if op == "navigate":
    cdp("Page.enable")
    out = cdp("Page.navigate", url=params["url"])
elif op == "snapshot":
    cdp("Runtime.enable")
    js = "JSON.stringify({url:location.href,title:document.title,text:(document.body?.innerText||'').slice(0,12000),links:Array.from(document.querySelectorAll('a,button,input,textarea,select')).slice(0,80).map((e,i)=>({idx:i,tag:e.tagName.toLowerCase(),text:(e.innerText||e.value||e.getAttribute('aria-label')||'').trim().slice(0,80),id:e.id||''}))})"
    r = cdp("Runtime.evaluate", expression=js, returnByValue=True)
    out = json.loads(r.get("result", {}).get("value", "{}"))
elif op == "click":
    sel = json.dumps(params.get("selector", ""))
    js = f"(()=>{{const e=document.querySelector({sel});if(!e)return {{ok:false,error:'selector not found'}};e.scrollIntoView();e.click();return {{ok:true,tag:e.tagName}};}})()"
    r = cdp("Runtime.evaluate", expression=js, returnByValue=True)
    out = r.get("result", {}).get("value", {})
elif op in ("type_text", "type"):
    sel = json.dumps(params.get("selector", ""))
    txt = json.dumps(params.get("text", ""))
    js = f"(()=>{{const e=document.querySelector({sel});if(!e)return {{ok:false,error:'selector not found'}};e.focus();e.value={txt};e.dispatchEvent(new Event('input',{{bubbles:true}}));e.dispatchEvent(new Event('change',{{bubbles:true}}));return {{ok:true}};}})()"
    r = cdp("Runtime.evaluate", expression=js, returnByValue=True)
    out = r.get("result", {}).get("value", {})
elif op == "eval":
    expr = params.get("expression") or params.get("script") or ""
    r = cdp("Runtime.evaluate", expression=expr, returnByValue=True)
    out = {"ok": True, "value": r.get("result", {}).get("value")}
elif op == "scroll":
    dy = int(params.get("delta_y", 0))
    dx = int(params.get("delta_x", 0))
    js = f"(()=>{{window.scrollBy({dx},{dy});return {{ok:true,scrollX:window.scrollX,scrollY:window.scrollY}};}})()"
    r = cdp("Runtime.evaluate", expression=js, returnByValue=True)
    out = r.get("result", {}).get("value", {})
elif op == "press_key":
    key = params.get("key", "")
    cdp("Input.dispatchKeyEvent", type="keyDown", key=key)
    cdp("Input.dispatchKeyEvent", type="keyUp", key=key)
    out = {"ok": True, "key": key}
elif op == "screenshot":
    r = cdp("Page.captureScreenshot", format="png")
    raw = base64.b64decode(r["data"])
    with open(req["screenshot_out"], "wb") as f: f.write(raw)
    out = {"screenshot_path": req["screenshot_out"], "bytes": len(raw)}
sock.close()
print(json.dumps(out))
"#;
        let payload = json!({
            "ws_url": ws_url,
            "op": op,
            "params": params,
            "screenshot_out": screenshot_out.map(|p| p.display().to_string())
        });
        let mut cmd = Command::new("python3");
        cmd.arg("-c")
            .arg(py)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        scrub_sensitive_env(&mut cmd);
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("failed to spawn cdp helper: {e}"))?;
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(payload.to_string().as_bytes());
        }
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) => {
                    if Instant::now() >= deadline {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err("CDP operation timed out after 20s".into());
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(_) => break,
            }
        }
        let output = child
            .wait_with_output()
            .map_err(|e| format!("cdp helper io error: {e}"))?;
        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        serde_json::from_str(stdout.trim()).map_err(|e| format!("invalid cdp output: {e}"))
    }
}

fn validate_browser_url(raw: &str, network_allowed: bool) -> Result<(), String> {
    let parsed = Url::parse(raw).map_err(|e| format!("invalid url: {e}"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("browser navigate only allows http:// and https:// URLs".into());
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("URL userinfo is not allowed in browser navigate".into());
    }
    if !network_allowed {
        let is_loopback = match parsed.host() {
            Some(Host::Domain(d)) => d.eq_ignore_ascii_case("localhost"),
            Some(Host::Ipv4(v4)) => v4.is_loopback(),
            Some(Host::Ipv6(v6)) => v6.is_loopback(),
            None => false,
        };
        if !is_loopback {
            return Err("browser navigation to non-loopback host blocked by network policy".into());
        }
    }
    Ok(())
}

fn find_chromium_binary() -> Option<String> {
    for candidate in [
        "google-chrome",
        "chromium",
        "chromium-browser",
        "microsoft-edge",
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    ] {
        if Path::new(candidate).is_file() {
            return Some(candidate.to_string());
        }
        if Command::new("which")
            .arg(candidate)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            return Some(candidate.to_string());
        }
    }
    None
}

fn allocate_loopback_port() -> Result<u16, String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    Ok(listener.local_addr().map_err(|e| e.to_string())?.port())
}

pub struct BrowserTool;

impl ToolHandler for BrowserTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "browser".into(),
                description: "Automate a managed Chromium CDP browser or attached Chrome extension bridge (action: start|status|navigate|snapshot|click|type_text|type|eval|scroll|press_key|screenshot|tabs|stop; target: managed|attached).".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": ["start", "status", "navigate", "snapshot", "click", "type_text", "type", "eval", "scroll", "press_key", "screenshot", "tabs", "stop"]
                        },
                        "target": {
                            "type": "string",
                            "enum": ["managed", "attached"],
                            "description": "Execution target: 'managed' local Chromium CDP (default) or 'attached' native browser extension bridge"
                        },
                        "url": { "type": "string" },
                        "selector": { "type": "string" },
                        "text": { "type": "string" },
                        "expression": { "type": "string", "description": "JavaScript expression for action=eval" },
                        "delta_y": { "type": "integer", "description": "Vertical scroll delta in pixels for action=scroll" },
                        "delta_x": { "type": "integer", "description": "Optional horizontal scroll delta in pixels for action=scroll" },
                        "key": { "type": "string", "description": "Key name for action=press_key (e.g. 'Enter', 'Escape', 'Tab')" },
                        "timeout_ms": { "type": "integer", "description": "Wait timeout in ms for target=attached (default 15000)" },
                        "headless": { "type": "boolean", "description": "Default true" }
                    },
                    "required": ["action"]
                }),
            },
            tier: ToolTier::Deferred,
            domain: CapabilityDomain::Browser,
            read_only: false,
        }
    }

    fn execute(&self, ctx: &ToolContext, args: Value) -> ToolExecutionResult {
        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("status");
        let target = args
            .get("target")
            .and_then(|v| v.as_str())
            .unwrap_or("managed");

        if action == "navigate" {
            let Some(u) = args.get("url").and_then(|v| v.as_str()) else {
                return ToolExecutionResult::err_structured(
                    "missing_param",
                    "browser navigate requires 'url'",
                    None,
                );
            };
            if let Err(e) = validate_browser_url(u, ctx.network_allowed) {
                return ToolExecutionResult::err_structured("browser_url_blocked", e, None);
            }
        }

        if target == "attached"
            && matches!(
                action,
                "navigate"
                    | "click"
                    | "type"
                    | "type_text"
                    | "eval"
                    | "screenshot"
                    | "snapshot"
                    | "scroll"
                    | "press_key"
            )
        {
            let timeout_ms = args
                .get("timeout_ms")
                .and_then(|v| v.as_u64())
                .unwrap_or(15_000)
                .clamp(100, 30_000);
            let mut cmd = args.clone();
            if let Some(obj) = cmd.as_object_mut() {
                obj.insert("action".into(), Value::String(action.to_string()));
                obj.insert("target".into(), Value::String("attached".into()));
            }
            return match ctx
                .browser
                .native_bridge()
                .queue_command(cmd, Duration::from_millis(timeout_ms))
            {
                Ok(v) => {
                    if v.get("ok") == Some(&Value::Bool(false)) {
                        let msg = v
                            .get("error")
                            .and_then(|e| e.as_str())
                            .unwrap_or("attached browser action failed")
                            .to_string();
                        ToolExecutionResult::err_structured("browser_action_failed", msg, Some(v))
                    } else {
                        let mut res = ToolExecutionResult::ok_structured(
                            v.clone(),
                            format!("Attached browser {action} completed"),
                        );
                        if action == "screenshot" {
                            if let Some(b64) = v
                                .get("image_base64")
                                .or_else(|| v.get("data"))
                                .and_then(|s| s.as_str())
                            {
                                if let Ok(bytes) = B64.decode(b64) {
                                    res = res.with_media(vec![MediaAttachment {
                                        mime_type: "image/png".into(),
                                        filename: "attached_browser_screenshot.png".into(),
                                        sha256: sha256_hex(&bytes),
                                        raw_bytes_len: bytes.len(),
                                        bytes_base64: b64.to_string(),
                                    }]);
                                }
                            }
                        }
                        res
                    }
                }
                Err(e) => ToolExecutionResult::err_structured("attached_browser_error", e, None),
            };
        }

        match action {
            "status" => ToolExecutionResult::ok_structured(
                ctx.browser.status(),
                "Checked browser automation status",
            ),
            "start" => {
                let headless = args
                    .get("headless")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true);
                match ctx.browser.start(headless) {
                    Ok(v) => ToolExecutionResult::ok_structured(
                        v,
                        "Started managed Chromium CDP session",
                    ),
                    Err(e) => ToolExecutionResult::err_structured("browser_start_error", e, None),
                }
            }
            "stop" => {
                ToolExecutionResult::ok_structured(ctx.browser.stop(), "Stopped managed browser")
            }
            "tabs" => match ctx.browser.list_tabs() {
                Ok(tabs) => {
                    let count = tabs.len();
                    ToolExecutionResult::ok_structured(
                        json!({"count": count, "tabs": tabs}),
                        format!("Listed {count} browser tab(s)"),
                    )
                }
                Err(e) => ToolExecutionResult::err_structured("browser_error", e, None),
            },
            "navigate" | "snapshot" | "click" | "type_text" | "type" | "eval" | "scroll"
            | "press_key" => {
                if action == "press_key"
                    && args
                        .get("key")
                        .and_then(|v| v.as_str())
                        .is_none_or(|s| s.is_empty())
                {
                    return ToolExecutionResult::err_structured(
                        "missing_param",
                        "browser press_key requires non-empty 'key'",
                        None,
                    );
                }
                match ctx.browser.run_cdp_python_script(action, &args, None) {
                    Ok(v) => {
                        if v.get("ok") == Some(&Value::Bool(false)) {
                            let msg = v
                                .get("error")
                                .and_then(|e| e.as_str())
                                .unwrap_or("browser action failed")
                                .to_string();
                            ToolExecutionResult::err_structured(
                                "browser_action_failed",
                                msg,
                                Some(v),
                            )
                        } else {
                            ToolExecutionResult::ok_structured(
                                v,
                                format!("Browser {action} completed"),
                            )
                        }
                    }
                    Err(e) => ToolExecutionResult::err_structured("browser_action_error", e, None),
                }
            }
            "screenshot" => {
                let out_path =
                    std::env::temp_dir().join(format!("nola-browser-shot-{}.png", new_id()));
                match ctx
                    .browser
                    .run_cdp_python_script("screenshot", &args, Some(&out_path))
                {
                    Ok(v) => {
                        let mut media = Vec::new();
                        if let Ok(bytes) = fs::read(&out_path) {
                            media.push(MediaAttachment {
                                mime_type: "image/png".into(),
                                filename: "browser_screenshot.png".into(),
                                sha256: sha256_hex(&bytes),
                                raw_bytes_len: bytes.len(),
                                bytes_base64: B64.encode(&bytes),
                            });
                        }
                        ToolExecutionResult::ok_structured(
                            v,
                            format!("Captured browser screenshot to {}", out_path.display()),
                        )
                        .with_media(media)
                    }
                    Err(e) => {
                        ToolExecutionResult::err_structured("browser_screenshot_error", e, None)
                    }
                }
            }
            other => ToolExecutionResult::err_structured(
                "invalid_action",
                format!("unsupported browser action '{other}'"),
                None,
            ),
        }
    }
}
