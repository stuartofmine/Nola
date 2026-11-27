// SPDX-License-Identifier: Apache-2.0
//! Real JSON-RPC 2.0 Language Server Protocol (LSP) client pool + diagnostics/navigation tool:
//! - Dedicated background reader thread with finite `recv_timeout(10s)` (never blocks forever!)
//! - Caches push `textDocument/publishDiagnostics` notifications
//! - Strictly rejects reverse server-initiated edit/command requests with JSON-RPC `-32601`
//! - Properly kills & reaps LSP child processes on `Drop` and evicts dead/timed-out sessions
//! - Enforces 8 MiB `Content-Length` cap, 8 KiB header cap, and rejects duplicate `Content-Length`
//! - Session-isolated pool keyed by `(session_id, workspace_root, language_id)`
//! - Built-in servers for Rust, Python, TypeScript/JS, Go, C/C++, Swift (`sourcekit-lsp`),
//!   and Kotlin (`kotlin-language-server`), plus `NOLA_LSP_SERVERS_JSON` env override
//! - Returns explicit structured errors when LSP servers are unavailable (no silent fallbacks!)

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;

use nola_types::{now_ms, CapabilityDomain, ToolExecutionResult, ToolSchema, ToolSpec, ToolTier};
use serde_json::{json, Value};

use crate::file_pipeline::resolve_path;
use crate::{ToolContext, ToolHandler};

const LSP_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_LSP_POOL_SESSIONS: usize = 8;
pub(crate) const MAX_LSP_FRAME_BYTES: usize = 8 * 1024 * 1024;
pub(crate) const MAX_LSP_HEADER_BYTES: usize = 8 * 1024;

#[derive(Debug, Clone)]
struct LspServerSpec {
    language_id: String,
    bin: String,
    args: Vec<String>,
    extensions: Vec<String>,
    source: &'static str,
}

fn builtin_lsp_servers() -> Vec<LspServerSpec> {
    vec![
        LspServerSpec {
            language_id: "rust".into(),
            bin: "rust-analyzer".into(),
            args: vec![],
            extensions: vec!["rs".into()],
            source: "builtin",
        },
        LspServerSpec {
            language_id: "python".into(),
            bin: "pyright-langserver".into(),
            args: vec!["--stdio".into()],
            extensions: vec!["py".into()],
            source: "builtin",
        },
        LspServerSpec {
            language_id: "typescript".into(),
            bin: "typescript-language-server".into(),
            args: vec!["--stdio".into()],
            extensions: vec!["ts".into(), "tsx".into(), "js".into(), "jsx".into()],
            source: "builtin",
        },
        LspServerSpec {
            language_id: "go".into(),
            bin: "gopls".into(),
            args: vec![],
            extensions: vec!["go".into()],
            source: "builtin",
        },
        LspServerSpec {
            language_id: "cpp".into(),
            bin: "clangd".into(),
            args: vec![],
            extensions: vec!["c".into(), "cc".into(), "cpp".into(), "h".into(), "hpp".into()],
            source: "builtin",
        },
        LspServerSpec {
            language_id: "swift".into(),
            bin: "sourcekit-lsp".into(),
            args: vec![],
            extensions: vec!["swift".into()],
            source: "builtin",
        },
        LspServerSpec {
            language_id: "kotlin".into(),
            bin: "kotlin-language-server".into(),
            args: vec![],
            extensions: vec!["kt".into(), "kts".into()],
            source: "builtin",
        },
    ]
}

fn parse_env_lsp_overrides(raw: &str) -> Vec<LspServerSpec> {
    let Ok(val) = serde_json::from_str::<Value>(raw) else {
        return Vec::new();
    };
    let mut overrides = Vec::new();
    let parse_obj = |obj: &serde_json::Map<String, Value>, fallback_lang: Option<&str>| -> Option<LspServerSpec> {
        let bin = obj.get("bin").or_else(|| obj.get("command")).and_then(|v| v.as_str())?.trim();
        if bin.is_empty() {
            return None;
        }
        let language_id = obj
            .get("language_id")
            .and_then(|v| v.as_str())
            .or(fallback_lang)
            .unwrap_or("custom")
            .trim()
            .to_ascii_lowercase();
        let args: Vec<String> = obj
            .get("args")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(ToString::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let mut extensions: Vec<String> = obj
            .get("extensions")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| {
                        v.as_str()
                            .map(|s| s.trim().trim_start_matches('.').to_ascii_lowercase())
                    })
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        if extensions.is_empty() {
            if let Some(ext) = fallback_lang.map(|s| s.trim_start_matches('.').to_ascii_lowercase()) {
                extensions.push(ext);
            }
        }
        Some(LspServerSpec {
            language_id,
            bin: bin.to_string(),
            args,
            extensions,
            source: "env",
        })
    };

    match val {
        Value::Array(arr) => {
            for item in arr {
                if let Some(obj) = item.as_object() {
                    if let Some(spec) = parse_obj(obj, None) {
                        overrides.push(spec);
                    }
                }
            }
        }
        Value::Object(map) => {
            for (k, v) in map {
                if let Some(obj) = v.as_object() {
                    if let Some(spec) = parse_obj(obj, Some(&k)) {
                        overrides.push(spec);
                    }
                }
            }
        }
        _ => {}
    }
    overrides
}

fn configured_lsp_servers() -> Vec<LspServerSpec> {
    let mut specs = builtin_lsp_servers();
    if let Ok(raw) = std::env::var("NOLA_LSP_SERVERS_JSON") {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            let env_specs = parse_env_lsp_overrides(trimmed);
            for ov in env_specs {
                if let Some(existing) = specs
                    .iter_mut()
                    .find(|s| s.language_id == ov.language_id)
                {
                    *existing = ov;
                } else {
                    specs.push(ov);
                }
            }
        }
    }
    specs
}

fn detect_lsp_server(ext: &str) -> Option<LspServerSpec> {
    let norm = ext.trim().trim_start_matches('.').to_ascii_lowercase();
    if norm.is_empty() {
        return None;
    }
    // Check env overrides first so custom entries take precedence even if extension overlaps.
    let specs = configured_lsp_servers();
    specs
        .iter()
        .find(|s| s.source == "env" && s.extensions.iter().any(|e| e == &norm))
        .or_else(|| {
            specs
                .iter()
                .find(|s| s.extensions.iter().any(|e| e == &norm))
        })
        .cloned()
}

fn is_binary_available(bin: &str) -> bool {
    let p = Path::new(bin);
    if p.components().count() > 1 {
        return p.is_file();
    }
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let cand = dir.join(bin);
            if cand.is_file() {
                return true;
            }
            #[cfg(target_os = "windows")]
            {
                let cand_exe = dir.join(format!("{bin}.exe"));
                if cand_exe.is_file() {
                    return true;
                }
            }
        }
    }
    false
}

/// Optional best-effort post-edit diagnostics collector.
/// Returns `None` immediately when no LSP server binary is configured/installed for `path`'s
/// extension, avoiding stalls on hosts without external language servers.
pub fn collect_post_edit_diagnostics(ctx: &ToolContext, path: &Path) -> Option<Value> {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let spec = detect_lsp_server(&ext)?;
    if !is_binary_available(&spec.bin) {
        return None;
    }
    ctx.lsp
        .query_scoped(
            &ctx.session_id,
            &ctx.workspace_root,
            path,
            "diagnostics",
            0,
            0,
            None,
        )
        .ok()
}

fn path_to_uri(path: &Path) -> String {
    url::Url::from_file_path(path)
        .map(|u| u.to_string())
        .unwrap_or_else(|_| format!("file://{}", path.display()))
}

/// Build the JSON-RPC 2.0 reply to a server-to-client request.
/// Only `workspace/configuration` is allowed (`result: []`); `workspace/applyEdit`,
/// `workspace/executeCommand`, and all unknown methods are rejected with `-32601`.
pub(crate) fn build_reverse_request_reply(req_id: Value, method: &str) -> Value {
    if method == "workspace/configuration" {
        json!({
            "jsonrpc": "2.0",
            "id": req_id,
            "result": []
        })
    } else {
        json!({
            "jsonrpc": "2.0",
            "id": req_id,
            "error": {
                "code": -32601,
                "message": "client does not allow server-initiated edits or commands"
            }
        })
    }
}

type PendingLspResponses = Arc<Mutex<BTreeMap<u64, mpsc::Sender<Result<Value, String>>>>>;

struct LspSession {
    child: Mutex<Child>,
    writer_tx: Mutex<Option<mpsc::Sender<Value>>>,
    next_id: AtomicU64,
    pending: PendingLspResponses,
    published_diagnostics: Arc<Mutex<BTreeMap<String, Vec<Value>>>>,
    opened_docs: Mutex<BTreeMap<PathBuf, i32>>,
    supports_pull_diagnostics: AtomicBool,
    closed: Arc<AtomicBool>,
    last_used_ms: AtomicI64,
}

impl Drop for LspSession {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::SeqCst);
        if let Ok(mut tx) = self.writer_tx.lock() {
            let _ = tx.take();
        }
        if let Ok(mut ch) = self.child.lock() {
            let _ = ch.kill();
            let _ = ch.wait();
        }
    }
}

impl LspSession {
    fn spawn(spec: &LspServerSpec, workspace_root: &Path) -> Result<Self, String> {
        let mut child = Command::new(&spec.bin)
            .args(&spec.args)
            .current_dir(workspace_root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("{} unavailable: {e}", spec.bin))?;
        let mut stdin_raw = child.stdin.take().ok_or("missing lsp stdin")?;
        let stdout_raw = child.stdout.take().ok_or("missing lsp stdout")?;

        let (writer_tx, writer_rx) = mpsc::channel::<Value>();
        let pending: PendingLspResponses = Arc::new(Mutex::new(BTreeMap::new()));
        let published_diagnostics: Arc<Mutex<BTreeMap<String, Vec<Value>>>> =
            Arc::new(Mutex::new(BTreeMap::new()));
        let closed = Arc::new(AtomicBool::new(false));

        // Dedicated non-blocking writer thread so stdout reader reverse-request replies
        // can NEVER deadlock on a full stdin pipe buffer!
        let closed_writer = Arc::clone(&closed);
        thread::spawn(move || {
            while let Ok(msg) = writer_rx.recv() {
                let Ok(body) = serde_json::to_vec(&msg) else {
                    continue;
                };
                let ok = write!(stdin_raw, "Content-Length: {}\r\n\r\n", body.len())
                    .and_then(|_| stdin_raw.write_all(&body))
                    .and_then(|_| stdin_raw.flush())
                    .is_ok();
                if !ok {
                    closed_writer.store(true, Ordering::SeqCst);
                    break;
                }
            }
        });

        let reply_tx = writer_tx.clone();
        let pending_bg = Arc::clone(&pending);
        let diags_bg = Arc::clone(&published_diagnostics);
        let closed_bg = Arc::clone(&closed);
        thread::spawn(move || {
            let mut reader = BufReader::new(stdout_raw);
            while let Ok(frame) = read_lsp_frame(&mut reader) {
                let has_method = frame.get("method").and_then(|v| v.as_str());
                let has_id = frame.get("id").cloned();
                match (has_id, has_method) {
                    // Server-to-client request: allow only workspace/configuration, reject all others!
                    (Some(req_id), Some(method)) => {
                        let reply = build_reverse_request_reply(req_id, method);
                        let _ = reply_tx.send(reply);
                    }
                    // Notification: cache `textDocument/publishDiagnostics`
                    (None, Some("textDocument/publishDiagnostics")) => {
                        if let Some(params) = frame.get("params") {
                            if let Some(uri) = params.get("uri").and_then(|v| v.as_str()) {
                                let list = params
                                    .get("diagnostics")
                                    .and_then(|v| v.as_array())
                                    .cloned()
                                    .unwrap_or_default();
                                diags_bg.lock().unwrap().insert(uri.to_string(), list);
                            }
                        }
                    }
                    // Response to our request
                    (Some(id_val), None) => {
                        if let Some(id) = id_val.as_u64() {
                            if let Some(tx) = pending_bg.lock().unwrap().remove(&id) {
                                let res =
                                    if let Some(err) = frame.get("error").filter(|v| !v.is_null()) {
                                        Err(err.to_string())
                                    } else {
                                        Ok(frame.get("result").cloned().unwrap_or(Value::Null))
                                    };
                                let _ = tx.send(res);
                            }
                        }
                    }
                    _ => {}
                }
            }
            closed_bg.store(true, Ordering::SeqCst);
            let waiters = std::mem::take(&mut *pending_bg.lock().unwrap());
            for (_, tx) in waiters {
                let _ = tx.send(Err("lsp server closed stdout".into()));
            }
        });

        let session = Self {
            child: Mutex::new(child),
            writer_tx: Mutex::new(Some(writer_tx)),
            next_id: AtomicU64::new(1),
            pending,
            published_diagnostics,
            opened_docs: Mutex::new(BTreeMap::new()),
            supports_pull_diagnostics: AtomicBool::new(false),
            closed,
            last_used_ms: AtomicI64::new(now_ms()),
        };
        let root_uri = path_to_uri(workspace_root);
        let init_res = session.request(
            "initialize",
            json!({
                "processId": std::process::id(),
                "rootUri": root_uri,
                "capabilities": {
                    "textDocument": {
                        "hover": {"contentFormat": ["markdown", "plaintext"]},
                        "definition": {"linkSupport": true},
                        "implementation": {"linkSupport": true},
                        "documentSymbol": {},
                        "publishDiagnostics": {}
                    }
                }
            }),
        )?;
        let pull_ok = init_res
            .get("capabilities")
            .and_then(|c| c.get("diagnosticProvider"))
            .is_some();
        session
            .supports_pull_diagnostics
            .store(pull_ok, Ordering::SeqCst);
        session.notify("initialized", json!({}))?;
        Ok(session)
    }

    fn is_healthy(&self) -> bool {
        !self.closed.load(Ordering::SeqCst)
    }

    fn touch(&self) {
        self.last_used_ms.store(now_ms(), Ordering::Relaxed);
    }

    fn is_idle(&self) -> bool {
        self.pending.lock().map(|p| p.is_empty()).unwrap_or(true)
    }

    fn enqueue_write(&self, msg: Value) -> Result<(), String> {
        let guard = self.writer_tx.lock().unwrap();
        let Some(tx) = guard.as_ref() else {
            return Err("lsp stdin closed".into());
        };
        tx.send(msg).map_err(|_| "lsp writer closed".to_string())
    }

    fn notify(&self, method: &str, params: Value) -> Result<(), String> {
        self.touch();
        self.enqueue_write(json!({"jsonrpc": "2.0", "method": method, "params": params}))
    }

    fn request(&self, method: &str, params: Value) -> Result<Value, String> {
        if !self.is_healthy() {
            return Err("lsp session closed".into());
        }
        self.touch();
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel();
        self.pending.lock().unwrap().insert(id, tx);
        if let Err(e) = self.enqueue_write(
            json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}),
        ) {
            self.pending.lock().unwrap().remove(&id);
            return Err(e);
        }
        match rx.recv_timeout(LSP_TIMEOUT) {
            Ok(res) => res,
            Err(_) => {
                self.pending.lock().unwrap().remove(&id);
                self.closed.store(true, Ordering::SeqCst);
                Err(format!(
                    "lsp request '{method}' timed out after {}s",
                    LSP_TIMEOUT.as_secs()
                ))
            }
        }
    }

    fn sync_document(&self, path: &Path, language_id: &str) -> Result<String, String> {
        let uri = path_to_uri(path);
        let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        let current_ver = self
            .opened_docs
            .lock()
            .unwrap()
            .get(path)
            .copied()
            .unwrap_or(0);
        let next_ver = current_ver + 1;
        if next_ver == 1 {
            self.notify(
                "textDocument/didOpen",
                json!({
                    "textDocument": {
                        "uri": uri,
                        "languageId": language_id,
                        "version": next_ver,
                        "text": text
                    }
                }),
            )?;
        } else {
            self.notify(
                "textDocument/didChange",
                json!({
                    "textDocument": {"uri": uri, "version": next_ver},
                    "contentChanges": [{"text": text}]
                }),
            )?;
        }
        self.opened_docs
            .lock()
            .unwrap()
            .insert(path.to_path_buf(), next_ver);
        Ok(uri)
    }
}

pub(crate) fn read_lsp_frame(reader: &mut dyn BufRead) -> Result<Value, String> {
    let mut content_length: Option<usize> = None;
    let mut header_bytes = 0usize;
    let mut line = Vec::new();
    loop {
        line.clear();
        loop {
            let avail = reader.fill_buf().map_err(|e| e.to_string())?;
            if avail.is_empty() {
                return Err("lsp stdout EOF".into());
            }
            if let Some(nl) = avail.iter().position(|&b| b == b'\n') {
                let take = nl + 1;
                header_bytes += take;
                if header_bytes > MAX_LSP_HEADER_BYTES {
                    return Err(format!(
                        "lsp header exceeds {MAX_LSP_HEADER_BYTES} bytes"
                    ));
                }
                line.extend_from_slice(&avail[..take]);
                reader.consume(take);
                break;
            } else {
                let take = avail.len();
                header_bytes += take;
                if header_bytes > MAX_LSP_HEADER_BYTES {
                    return Err(format!(
                        "lsp header exceeds {MAX_LSP_HEADER_BYTES} bytes"
                    ));
                }
                line.extend_from_slice(avail);
                reader.consume(take);
            }
        }
        let line_str = std::str::from_utf8(&line)
            .map_err(|_| "invalid UTF-8 in lsp header".to_string())?;
        let trimmed = line_str.trim();
        if trimmed.is_empty() {
            break;
        }
        if let Some((k, v)) = trimmed.split_once(':') {
            if k.trim().eq_ignore_ascii_case("content-length") {
                if content_length.is_some() {
                    return Err("duplicate Content-Length header in lsp frame".into());
                }
                let parsed = v
                    .trim()
                    .parse::<usize>()
                    .map_err(|_| format!("invalid lsp Content-Length: '{}'", v.trim()))?;
                content_length = Some(parsed);
            }
        }
    }
    let len = content_length.ok_or_else(|| "missing lsp Content-Length header".to_string())?;
    if len == 0 || len > MAX_LSP_FRAME_BYTES {
        return Err(format!("invalid lsp Content-Length: {len}"));
    }
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf).map_err(|e| e.to_string())?;
    serde_json::from_slice(&buf).map_err(|e| e.to_string())
}

type LspPoolKey = (String, PathBuf, String);

/// Per-(session, workspace, language) LSP pool so sessions never share mutable open-document
/// state and dead/timed-out sessions are automatically evicted.
#[derive(Default)]
pub struct LspPool {
    sessions: Mutex<BTreeMap<LspPoolKey, Arc<LspSession>>>,
}

impl LspPool {
    pub fn list_servers(&self) -> Vec<Value> {
        configured_lsp_servers()
            .into_iter()
            .map(|s| {
                let available = is_binary_available(&s.bin);
                json!({
                    "language_id": s.language_id,
                    "bin": s.bin,
                    "args": s.args,
                    "extensions": s.extensions,
                    "available": available,
                    "source": s.source
                })
            })
            .collect()
    }

    fn get_or_spawn(
        &self,
        session_id: &str,
        workspace_root: &Path,
        spec: &LspServerSpec,
    ) -> Result<Arc<LspSession>, String> {
        let key: LspPoolKey = (
            session_id.to_string(),
            workspace_root.to_path_buf(),
            spec.language_id.clone(),
        );
        if let Some(existing) = self.sessions.lock().unwrap().get(&key).cloned() {
            if existing.is_healthy() {
                existing.touch();
                return Ok(existing);
            }
        }
        let session = Arc::new(LspSession::spawn(spec, workspace_root)?);
        let mut guard = self.sessions.lock().unwrap();
        guard.retain(|_, s| s.is_healthy());
        guard.insert(key, Arc::clone(&session));
        while guard.len() > MAX_LSP_POOL_SESSIONS {
            let victim = guard
                .iter()
                .filter(|(_, s)| s.is_idle())
                .min_by_key(|(_, s)| s.last_used_ms.load(Ordering::Relaxed))
                .or_else(|| {
                    guard
                        .iter()
                        .min_by_key(|(_, s)| s.last_used_ms.load(Ordering::Relaxed))
                })
                .map(|(k, _)| k.clone());
            if let Some(k) = victim {
                guard.remove(&k);
            } else {
                break;
            }
        }
        Ok(session)
    }

    pub fn query(
        &self,
        workspace_root: &Path,
        path: &Path,
        action: &str,
        line: u32,
        character: u32,
        query: Option<&str>,
    ) -> Result<Value, String> {
        self.query_scoped("default", workspace_root, path, action, line, character, query)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn query_scoped(
        &self,
        session_id: &str,
        workspace_root: &Path,
        path: &Path,
        action: &str,
        line: u32,
        character: u32,
        query: Option<&str>,
    ) -> Result<Value, String> {
        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let Some(spec) = detect_lsp_server(&ext) else {
            return Err(format!("no LSP server mapping for '.{ext}'"));
        };
        let session = self.get_or_spawn(session_id, workspace_root, &spec)?;
        let res = (|| -> Result<Value, String> {
            if action == "workspace_symbols" {
                return session.request("workspace/symbol", json!({"query": query.unwrap_or("")}));
            }
            let uri = session.sync_document(path, &spec.language_id)?;
            match action {
                "hover" => session.request(
                    "textDocument/hover",
                    json!({"textDocument": {"uri": uri}, "position": {"line": line, "character": character}}),
                ),
                "definition" => session.request(
                    "textDocument/definition",
                    json!({"textDocument": {"uri": uri}, "position": {"line": line, "character": character}}),
                ),
                "implementation" => session.request(
                    "textDocument/implementation",
                    json!({"textDocument": {"uri": uri}, "position": {"line": line, "character": character}}),
                ),
                "references" => session.request(
                    "textDocument/references",
                    json!({
                        "textDocument": {"uri": uri},
                        "position": {"line": line, "character": character},
                        "context": {"includeDeclaration": true}
                    }),
                ),
                "document_symbols" => session.request(
                    "textDocument/documentSymbol",
                    json!({"textDocument": {"uri": uri}}),
                ),
                "diagnostics" => {
                    if session.supports_pull_diagnostics.load(Ordering::SeqCst) {
                        session.request(
                            "textDocument/diagnostic",
                            json!({"textDocument": {"uri": uri}}),
                        )
                    } else {
                        // Brief settle wait for push `publishDiagnostics` notification
                        std::thread::sleep(Duration::from_millis(250));
                        let list = session
                            .published_diagnostics
                            .lock()
                            .unwrap()
                            .get(&uri)
                            .cloned()
                            .unwrap_or_default();
                        Ok(json!({"items": list}))
                    }
                }
                other => Err(format!("unsupported lsp action '{other}'")),
            }
        })();

        if res.is_err() && !session.is_healthy() {
            let key: LspPoolKey = (
                session_id.to_string(),
                workspace_root.to_path_buf(),
                spec.language_id,
            );
            self.sessions.lock().unwrap().remove(&key);
        }
        res
    }
}

pub struct LspDiagnosticsTool;

impl ToolHandler for LspDiagnosticsTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "lsp".into(),
                description: "Query Language Server Protocol (list_servers, diagnostics, definition, implementation, references, hover, document_symbols, workspace_symbols). Spawns external LSP servers under Process permission admission.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": [
                                "list_servers",
                                "diagnostics",
                                "definition",
                                "implementation",
                                "references",
                                "hover",
                                "document_symbols",
                                "workspace_symbols"
                            ],
                            "description": "Default 'diagnostics'"
                        },
                        "path": { "type": "string", "description": "Target file path (required for all actions except list_servers)" },
                        "line": { "type": "integer", "description": "0-based line number" },
                        "character": { "type": "integer", "description": "0-based character column" },
                        "query": { "type": "string", "description": "Query string for workspace_symbols" }
                    }
                }),
            },
            tier: ToolTier::Deferred,
            domain: CapabilityDomain::Process,
            read_only: false,
        }
    }

    fn execute(&self, ctx: &ToolContext, args: Value) -> ToolExecutionResult {
        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("diagnostics");

        if action == "list_servers" {
            let servers = ctx.lsp.list_servers();
            let available_count = servers
                .iter()
                .filter(|s| s["available"].as_bool() == Some(true))
                .count();
            return ToolExecutionResult::ok_structured(
                json!({
                    "action": "list_servers",
                    "count": servers.len(),
                    "available_count": available_count,
                    "servers": servers
                }),
                format!(
                    "Listed {} configured LSP server(s) ({available_count} available on host)",
                    servers.len()
                ),
            );
        }

        let Some(raw_path) = args.get("path").and_then(|v| v.as_str()) else {
            return ToolExecutionResult::err_structured("missing_param", "missing 'path'", None);
        };
        let line = args.get("line").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        let character = args.get("character").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        let query = args.get("query").and_then(|v| v.as_str());
        let path = resolve_path(&ctx.workspace_root, raw_path);

        match ctx.lsp.query_scoped(
            &ctx.session_id,
            &ctx.workspace_root,
            &path,
            action,
            line,
            character,
            query,
        ) {
            Ok(result) => ToolExecutionResult::ok_structured(
                json!({
                    "backend": "lsp",
                    "action": action,
                    "path": path.display().to_string(),
                    "result": result
                }),
                format!("LSP {action} succeeded for {}", path.display()),
            ),
            Err(e) => ToolExecutionResult::err_structured(
                "lsp_unavailable",
                format!("LSP server unavailable for '{}': {e}", path.display()),
                Some(json!({
                    "action": action,
                    "path": path.display().to_string(),
                    "reason": e
                })),
            ),
        }
    }
}
