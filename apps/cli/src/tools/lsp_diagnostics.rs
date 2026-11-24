// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Zero-config, persistent Language Server Protocol (LSP) diagnostics
//            manager for automatic post-edit feedback (OpenCode-style).
// - Boundary: Advisory read-only diagnostics; lazily spawns at most one server
//             per (workspace_root, language) with bounded timeouts and clean drop.

#[cfg(unix)]
use std::collections::HashMap;
#[cfg(unix)]
use std::io::{BufRead, BufReader, Read, Write};
#[cfg(unix)]
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
#[cfg(unix)]
use std::sync::{mpsc, Mutex, OnceLock};
#[cfg(unix)]
use std::time::{Duration, Instant};

#[cfg(unix)]
use serde_json::{json, Value};

#[cfg(unix)]
const LSP_INIT_TIMEOUT: Duration = Duration::from_secs(3);
#[cfg(unix)]
const LSP_DIAG_WAIT: Duration = Duration::from_millis(1200);
#[cfg(unix)]
const LSP_SETTLE_GRACE: Duration = Duration::from_millis(120);
#[cfg(unix)]
const MAX_DIAGNOSTIC_ITEMS: usize = 8;

/// Queries a persistent, auto-detected LSP server (if installed in `PATH`) for
/// diagnostics on `path` after a committed file edit. Returns `None` when no
/// matching server is installed, when timed out, or when diagnostics are clean.
#[cfg(unix)]
pub(crate) fn query_lsp_diagnostics(path: &Path) -> Option<String> {
    if std::env::var_os("NOLA_DISABLE_AUTO_LSP").is_some() {
        return None;
    }
    let canonical = path.canonicalize().ok()?;
    let spec = detect_server_spec(&canonical)?;
    let root = find_workspace_root(&canonical, spec.root_markers);
    let content = std::fs::read_to_string(&canonical).ok()?;
    let pool = LSP_POOL.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = pool.lock().ok()?;
    let key = (root.clone(), spec.language_id.to_string());
    let needs_spawn = guard
        .get_mut(&key)
        .map(|client| !client.is_alive())
        .unwrap_or(true);
    if needs_spawn {
        guard.remove(&key);
        let client = LspClient::spawn(&spec.program, spec.args, &root).ok()?;
        guard.insert(key.clone(), client);
    }
    let client = guard.get_mut(&key)?;
    match client.diagnose_file(&canonical, spec.language_id, &content, LSP_DIAG_WAIT) {
        Ok(diags) => format_diagnostics(&canonical, &diags),
        Err(_) => {
            guard.remove(&key);
            None
        }
    }
}

#[cfg(not(unix))]
pub(crate) fn query_lsp_diagnostics(_path: &std::path::Path) -> Option<String> {
    None
}

#[cfg(unix)]
static LSP_POOL: OnceLock<Mutex<HashMap<(PathBuf, String), LspClient>>> = OnceLock::new();

#[cfg(unix)]
#[derive(Debug, Clone)]
struct ServerSpec {
    language_id: &'static str,
    program: String,
    args: &'static [&'static str],
    root_markers: &'static [&'static str],
}

#[cfg(unix)]
fn detect_server_spec(path: &Path) -> Option<ServerSpec> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    let candidates: &[(&str, &str, &[&str], &[&str])] = match ext.as_str() {
        "py" | "pyi" => &[
            ("python", "basedpyright-langserver", &["--stdio"], &["pyproject.toml", "setup.py", ".git"]),
            ("python", "pyright-langserver", &["--stdio"], &["pyproject.toml", "setup.py", ".git"]),
        ],
        "ts" | "tsx" => &[
            ("typescript", "typescript-language-server", &["--stdio"], &["tsconfig.json", "package.json", ".git"]),
        ],
        "rs" => &[
            ("rust", "rust-analyzer", &[], &["Cargo.toml"]),
        ],
        "go" => &[
            ("go", "gopls", &[], &["go.mod", ".git"]),
        ],
        "c" | "cc" | "cpp" | "cxx" | "h" | "hpp" => &[
            ("cpp", "clangd", &["--background-index=false", "--log=error"], &["compile_commands.json", "CMakeLists.txt", ".git"]),
        ],
        _ => return None,
    };
    for (lang, bin, args, markers) in candidates {
        if let Some(resolved) = find_in_path(bin) {
            return Some(ServerSpec {
                language_id: lang,
                program: resolved,
                args,
                root_markers: markers,
            });
        }
    }
    None
}

#[cfg(unix)]
fn find_in_path(binary: &str) -> Option<String> {
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(binary);
        if candidate.is_file() {
            return candidate.to_str().map(ToOwned::to_owned);
        }
    }
    None
}

#[cfg(unix)]
fn find_workspace_root(file: &Path, markers: &[&str]) -> PathBuf {
    let mut cursor = file.parent();
    while let Some(dir) = cursor {
        if markers.iter().any(|m| dir.join(m).exists()) {
            return dir.to_path_buf();
        }
        cursor = dir.parent();
    }
    file.parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(unix)]
#[derive(Debug, Clone)]
struct LspDiagnosticItem {
    line: u64,
    col: u64,
    severity: u64,
    message: String,
    source: Option<String>,
}

#[cfg(unix)]
struct LspClient {
    child: Child,
    stdin: ChildStdin,
    rx: mpsc::Receiver<Value>,
    next_id: i64,
    open_versions: HashMap<String, i64>,
    cached_diags: HashMap<String, Vec<LspDiagnosticItem>>,
}

#[cfg(unix)]
impl LspClient {
    fn spawn(program: &str, args: &[&str], root: &Path) -> Result<Self, String> {
        let mut child = Command::new(program)
            .args(args)
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|err| format!("spawn {program}: {err}"))?;
        let stdin = child.stdin.take().ok_or("missing lsp stdin")?;
        let stdout = child.stdout.take().ok_or("missing lsp stdout")?;
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || read_lsp_loop(stdout, tx));

        let mut client = Self {
            child,
            stdin,
            rx,
            next_id: 1,
            open_versions: HashMap::new(),
            cached_diags: HashMap::new(),
        };
        let root_uri = url::Url::from_directory_path(root)
            .map_err(|_| "invalid workspace root")?
            .to_string();
        let init_id = client.alloc_id();
        client.send(&json!({
            "jsonrpc": "2.0",
            "id": init_id,
            "method": "initialize",
            "params": {
                "processId": std::process::id(),
                "rootUri": root_uri,
                "capabilities": {
                    "textDocument": {
                        "publishDiagnostics": { "relatedInformation": false }
                    }
                }
            }
        }))?;
        let deadline = Instant::now() + LSP_INIT_TIMEOUT;
        let mut initialized = false;
        while Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match client.rx.recv_timeout(remaining) {
                Ok(msg) => {
                    client.handle_incoming(&msg);
                    if msg.get("id").and_then(Value::as_i64) == Some(init_id) {
                        initialized = true;
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        if !initialized {
            let _ = client.child.kill();
            let _ = client.child.wait();
            return Err("lsp initialize timed out".into());
        }
        client.send(&json!({
            "jsonrpc": "2.0",
            "method": "initialized",
            "params": {}
        }))?;
        Ok(client)
    }

    fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    fn alloc_id(&mut self) -> i64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn send(&mut self, msg: &Value) -> Result<(), String> {
        let body = serde_json::to_string(msg).map_err(|e| e.to_string())?;
        let header = format!("Content-Length: {}\r\n\r\n", body.len());
        self.stdin
            .write_all(header.as_bytes())
            .and_then(|_| self.stdin.write_all(body.as_bytes()))
            .and_then(|_| self.stdin.flush())
            .map_err(|e| e.to_string())
    }

    fn handle_incoming(&mut self, msg: &Value) {
        if msg.get("method").and_then(Value::as_str) == Some("textDocument/publishDiagnostics") {
            if let Some(params) = msg.get("params") {
                if let Some(uri) = params.get("uri").and_then(Value::as_str) {
                    let items = params
                        .get("diagnostics")
                        .and_then(Value::as_array)
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|d| {
                                    let severity = d.get("severity").and_then(Value::as_u64).unwrap_or(1);
                                    if severity > 2 {
                                        return None;
                                    }
                                    let start = d.get("range")?.get("start")?;
                                    let line = start.get("line").and_then(Value::as_u64)? + 1;
                                    let col = start.get("character").and_then(Value::as_u64)? + 1;
                                    let message = d.get("message").and_then(Value::as_str)?.trim().to_string();
                                    let source = d.get("source").and_then(Value::as_str).map(ToOwned::to_owned);
                                    Some(LspDiagnosticItem {
                                        line,
                                        col,
                                        severity,
                                        message,
                                        source,
                                    })
                                })
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    self.cached_diags.insert(uri.to_string(), items);
                }
            }
        } else if let Some(id) = msg.get("id") {
            // Answer server-initiated requests (e.g. client/registerCapability)
            // with an empty result so the server never stalls waiting on us.
            if msg.get("method").is_some() {
                let _ = self.send(&json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": null
                }));
            }
        }
    }

    fn diagnose_file(
        &mut self,
        path: &Path,
        language_id: &str,
        content: &str,
        wait: Duration,
    ) -> Result<Vec<LspDiagnosticItem>, String> {
        let uri = url::Url::from_file_path(path)
            .map_err(|_| "invalid file path")?
            .to_string();
        // Drain any stale notifications queued before this edit.
        while let Ok(msg) = self.rx.try_recv() {
            self.handle_incoming(&msg);
        }
        self.cached_diags.remove(&uri);

        if let Some(ver) = self.open_versions.get_mut(&uri) {
            *ver += 1;
            let next_ver = *ver;
            self.send(&json!({
                "jsonrpc": "2.0",
                "method": "textDocument/didChange",
                "params": {
                    "textDocument": { "uri": uri, "version": next_ver },
                    "contentChanges": [{ "text": content }]
                }
            }))?;
        } else {
            self.open_versions.insert(uri.clone(), 1);
            self.send(&json!({
                "jsonrpc": "2.0",
                "method": "textDocument/didOpen",
                "params": {
                    "textDocument": {
                        "uri": uri,
                        "languageId": language_id,
                        "version": 1,
                        "text": content
                    }
                }
            }))?;
        }

        let deadline = Instant::now() + wait;
        let mut received_for_uri = false;
        while Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let timeout = if received_for_uri {
                remaining.min(LSP_SETTLE_GRACE)
            } else {
                remaining
            };
            match self.rx.recv_timeout(timeout) {
                Ok(msg) => {
                    let is_target = msg.get("method").and_then(Value::as_str)
                        == Some("textDocument/publishDiagnostics")
                        && msg.pointer("/params/uri").and_then(Value::as_str) == Some(uri.as_str());
                    self.handle_incoming(&msg);
                    if is_target {
                        received_for_uri = true;
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => break,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err("lsp server exited".into());
                }
            }
        }
        Ok(self.cached_diags.get(&uri).cloned().unwrap_or_default())
    }
}

#[cfg(unix)]
impl Drop for LspClient {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(unix)]
fn read_lsp_loop(stdout: ChildStdout, tx: mpsc::Sender<Value>) {
    let mut reader = BufReader::new(stdout);
    loop {
        let mut content_length: Option<usize> = None;
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => return,
                Ok(_) => {}
            }
            let trimmed = line.trim();
            if trimmed.is_empty() {
                break;
            }
            if let Some((name, val)) = trimmed.split_once(':') {
                if name.eq_ignore_ascii_case("Content-Length") {
                    content_length = val.trim().parse::<usize>().ok();
                }
            }
        }
        let Some(len) = content_length.filter(|l| *l > 0 && *l <= 8 * 1024 * 1024) else {
            continue;
        };
        let mut buf = vec![0u8; len];
        if reader.read_exact(&mut buf).is_err() {
            return;
        }
        if let Ok(val) = serde_json::from_slice::<Value>(&buf) {
            if tx.send(val).is_err() {
                return;
            }
        }
    }
}

#[cfg(unix)]
fn format_diagnostics(path: &Path, items: &[LspDiagnosticItem]) -> Option<String> {
    // Surface errors (severity 1) first; only emit warnings if there are errors or
    // keep concise warnings capped at MAX_DIAGNOSTIC_ITEMS.
    let mut sorted = items.to_vec();
    sorted.sort_by_key(|d| (d.severity, d.line, d.col));
    let errors_only: Vec<_> = sorted.iter().filter(|d| d.severity == 1).collect();
    if errors_only.is_empty() {
        return None;
    }
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file");
    let mut lines = Vec::new();
    for item in errors_only.iter().take(MAX_DIAGNOSTIC_ITEMS) {
        let src = item
            .source
            .as_deref()
            .map(|s| format!(" ({s})"))
            .unwrap_or_default();
        lines.push(format!(
            "{name}:{}:{} [error]{src}: {}",
            item.line,
            item.col,
            item.message.replace('\n', " ")
        ));
    }
    if errors_only.len() > MAX_DIAGNOSTIC_ITEMS {
        lines.push(format!(
            "...and {} more LSP error(s)",
            errors_only.len() - MAX_DIAGNOSTIC_ITEMS
        ));
    }
    Some(lines.join("\n"))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn mock_lsp_client_receives_publish_diagnostics_on_open_and_change() {
        let dir = std::env::temp_dir().join(format!(
            "nola-lsp-diag-test-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("sample.py");
        std::fs::write(&target, "x = unknown_var\n").unwrap();

        // Tiny Python mock LSP server that handles initialize, didOpen, and didChange.
        let mock_server = dir.join("mock_lsp.py");
        std::fs::write(
            &mock_server,
            r#"#!/usr/bin/env python3
import json, sys

def read_msg():
    cl = None
    while True:
        line = sys.stdin.buffer.readline()
        if not line:
            return None
        s = line.decode('utf-8', 'replace').strip()
        if not s:
            break
        if s.lower().startswith('content-length:'):
            cl = int(s.split(':', 1)[1].strip())
    if not cl:
        return None
    body = sys.stdin.buffer.read(cl)
    return json.loads(body.decode('utf-8'))

def send_msg(obj):
    data = json.dumps(obj).encode('utf-8')
    sys.stdout.buffer.write(f"Content-Length: {len(data)}\r\n\r\n".encode('ascii') + data)
    sys.stdout.buffer.flush()

while True:
    msg = read_msg()
    if msg is None:
        break
    m = msg.get("method")
    if m == "initialize":
        send_msg({"jsonrpc": "2.0", "id": msg["id"], "result": {"capabilities": {}}})
    elif m in ("textDocument/didOpen", "textDocument/didChange"):
        params = msg["params"]
        uri = params["textDocument"]["uri"]
        text = params["textDocument"]["text"] if m == "textDocument/didOpen" else params["contentChanges"][0]["text"]
        diags = []
        if "unknown_var" in text:
            diags.append({
                "range": {"start": {"line": 0, "character": 4}, "end": {"line": 0, "character": 15}},
                "severity": 1,
                "source": "mock-pyright",
                "message": "\"unknown_var\" is not defined"
            })
        send_msg({"jsonrpc": "2.0", "method": "textDocument/publishDiagnostics", "params": {"uri": uri, "diagnostics": diags}})
"#,
        )
        .unwrap();

        let mut client =
            LspClient::spawn("python3", &["-B", mock_server.to_str().unwrap()], &dir)
                .expect("mock LSP should initialize");
        let diags1 = client
            .diagnose_file(&target, "python", "x = unknown_var\n", Duration::from_secs(2))
            .unwrap();
        let formatted1 = format_diagnostics(&target, &diags1).expect("should report error");
        assert!(formatted1.contains("sample.py:1:5 [error] (mock-pyright)"), "{formatted1}");
        assert!(formatted1.contains("unknown_var"), "{formatted1}");

        // After fixing the file, didChange returns clean diagnostics (None).
        let diags2 = client
            .diagnose_file(&target, "python", "x = 42\n", Duration::from_secs(2))
            .unwrap();
        assert!(format_diagnostics(&target, &diags2).is_none());

        drop(client);
        let _ = std::fs::remove_dir_all(dir);
    }
}
