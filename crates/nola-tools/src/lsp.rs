// SPDX-License-Identifier: Apache-2.0
//! Real JSON-RPC 2.0 Language Server Protocol (LSP) client pool + diagnostics/navigation tool:
//! - Dedicated background reader thread with finite `recv_timeout(10s)` (never blocks forever!)
//! - Caches push `textDocument/publishDiagnostics` notifications AND responds to server requests
//! - Properly kills & reaps LSP child processes on `Drop` and evicts dead/timed-out sessions
//! - Uses `url::Url::from_file_path` and 16 MiB `Content-Length` cap
//! - Read-only `python3 -B -c "import ast..."` fallback (never writes `.pyc`!)

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;

use nola_types::{CapabilityDomain, ToolExecutionResult, ToolSchema, ToolSpec, ToolTier};
use serde_json::{json, Value};

use crate::file_pipeline::resolve_path;
use crate::{ToolContext, ToolHandler};

const LSP_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_LSP_FRAME_BYTES: usize = 16 * 1024 * 1024;

struct LspServerSpec {
    language_id: &'static str,
    bin: &'static str,
    args: &'static [&'static str],
}

fn detect_lsp_server(ext: &str) -> Option<LspServerSpec> {
    match ext {
        "rs" => Some(LspServerSpec {
            language_id: "rust",
            bin: "rust-analyzer",
            args: &[],
        }),
        "py" => Some(LspServerSpec {
            language_id: "python",
            bin: "pyright-langserver",
            args: &["--stdio"],
        }),
        "ts" | "tsx" | "js" | "jsx" => Some(LspServerSpec {
            language_id: "typescript",
            bin: "typescript-language-server",
            args: &["--stdio"],
        }),
        "go" => Some(LspServerSpec {
            language_id: "go",
            bin: "gopls",
            args: &[],
        }),
        "c" | "cc" | "cpp" | "h" | "hpp" => Some(LspServerSpec {
            language_id: "cpp",
            bin: "clangd",
            args: &[],
        }),
        _ => None,
    }
}

fn path_to_uri(path: &Path) -> String {
    url::Url::from_file_path(path)
        .map(|u| u.to_string())
        .unwrap_or_else(|_| format!("file://{}", path.display()))
}

struct LspSession {
    child: Mutex<Child>,
    stdin: Arc<Mutex<Option<ChildStdin>>>,
    next_id: AtomicU64,
    pending: Arc<Mutex<BTreeMap<u64, mpsc::Sender<Result<Value, String>>>>>,
    published_diagnostics: Arc<Mutex<BTreeMap<String, Vec<Value>>>>,
    opened_docs: Mutex<BTreeMap<PathBuf, i32>>,
    supports_pull_diagnostics: AtomicBool,
    closed: Arc<AtomicBool>,
}

impl Drop for LspSession {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::SeqCst);
        if let Ok(mut stdin) = self.stdin.lock() {
            let _ = stdin.take();
        }
        if let Ok(mut ch) = self.child.lock() {
            let _ = ch.kill();
            let _ = ch.wait();
        }
    }
}

impl LspSession {
    fn spawn(spec: &LspServerSpec, workspace_root: &Path) -> Result<Self, String> {
        let mut child = Command::new(spec.bin)
            .args(spec.args)
            .current_dir(workspace_root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("{} unavailable: {e}", spec.bin))?;
        let stdin_raw = child.stdin.take().ok_or("missing lsp stdin")?;
        let stdout_raw = child.stdout.take().ok_or("missing lsp stdout")?;

        let stdin = Arc::new(Mutex::new(Some(stdin_raw)));
        let pending: Arc<Mutex<BTreeMap<u64, mpsc::Sender<Result<Value, String>>>>> =
            Arc::new(Mutex::new(BTreeMap::new()));
        let published_diagnostics: Arc<Mutex<BTreeMap<String, Vec<Value>>>> =
            Arc::new(Mutex::new(BTreeMap::new()));
        let closed = Arc::new(AtomicBool::new(false));

        let stdin_bg = Arc::clone(&stdin);
        let pending_bg = Arc::clone(&pending);
        let diags_bg = Arc::clone(&published_diagnostics);
        let closed_bg = Arc::clone(&closed);
        thread::spawn(move || {
            let mut reader = BufReader::new(stdout_raw);
            while let Ok(frame) = read_lsp_frame(&mut reader) {
                let has_method = frame.get("method").and_then(|v| v.as_str());
                let has_id = frame.get("id").cloned();
                match (has_id, has_method) {
                    // Server-to-client request: reply immediately so server never deadlocks!
                    (Some(req_id), Some(method)) => {
                        let result = if method == "workspace/configuration" {
                            json!([])
                        } else {
                            Value::Null
                        };
                        let reply = json!({"jsonrpc": "2.0", "id": req_id, "result": result});
                        let _ = write_lsp_frame(&stdin_bg, &reply);
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
                                let res = if let Some(err) = frame.get("error") {
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
            stdin,
            next_id: AtomicU64::new(1),
            pending,
            published_diagnostics,
            opened_docs: Mutex::new(BTreeMap::new()),
            supports_pull_diagnostics: AtomicBool::new(false),
            closed,
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

    fn notify(&self, method: &str, params: Value) -> Result<(), String> {
        write_lsp_frame(
            &self.stdin,
            &json!({"jsonrpc": "2.0", "method": method, "params": params}),
        )
    }

    fn request(&self, method: &str, params: Value) -> Result<Value, String> {
        if !self.is_healthy() {
            return Err("lsp session closed".into());
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel();
        self.pending.lock().unwrap().insert(id, tx);
        if let Err(e) = write_lsp_frame(
            &self.stdin,
            &json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}),
        ) {
            self.pending.lock().unwrap().remove(&id);
            return Err(e);
        }
        match rx.recv_timeout(LSP_TIMEOUT) {
            Ok(res) => res,
            Err(_) => {
                self.pending.lock().unwrap().remove(&id);
                self.closed.store(true, Ordering::SeqCst);
                Err(format!("lsp request '{method}' timed out after {}s", LSP_TIMEOUT.as_secs()))
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

fn write_lsp_frame(stdin: &Mutex<Option<ChildStdin>>, msg: &Value) -> Result<(), String> {
    let body = serde_json::to_vec(msg).map_err(|e| e.to_string())?;
    let mut guard = stdin.lock().unwrap();
    let Some(w) = guard.as_mut() else {
        return Err("lsp stdin closed".into());
    };
    write!(w, "Content-Length: {}\r\n\r\n", body.len()).map_err(|e| e.to_string())?;
    w.write_all(&body).map_err(|e| e.to_string())?;
    w.flush().map_err(|e| e.to_string())
}

fn read_lsp_frame(reader: &mut dyn BufRead) -> Result<Value, String> {
    let mut content_length: usize = 0;
    let mut header = String::new();
    loop {
        header.clear();
        let n = reader.read_line(&mut header).map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("lsp stdout EOF".into());
        }
        let trimmed = header.trim();
        if trimmed.is_empty() {
            break;
        }
        if let Some((k, v)) = trimmed.split_once(':') {
            if k.trim().eq_ignore_ascii_case("content-length") {
                content_length = v.trim().parse().unwrap_or(0);
            }
        }
    }
    if content_length == 0 || content_length > MAX_LSP_FRAME_BYTES {
        return Err(format!("invalid lsp Content-Length: {content_length}"));
    }
    let mut buf = vec![0u8; content_length];
    reader.read_exact(&mut buf).map_err(|e| e.to_string())?;
    serde_json::from_slice(&buf).map_err(|e| e.to_string())
}

/// Per-(workspace, language) LSP pool that never kills sibling workspace sessions
/// and automatically evicts dead/timed-out sessions.
#[derive(Default)]
pub struct LspPool {
    sessions: Mutex<BTreeMap<(PathBuf, String), Arc<LspSession>>>,
}

impl LspPool {
    fn get_or_spawn(
        &self,
        workspace_root: &Path,
        spec: &LspServerSpec,
    ) -> Result<Arc<LspSession>, String> {
        let key = (workspace_root.to_path_buf(), spec.language_id.to_string());
        if let Some(existing) = self.sessions.lock().unwrap().get(&key).cloned() {
            if existing.is_healthy() {
                return Ok(existing);
            }
        }
        let session = Arc::new(LspSession::spawn(spec, workspace_root)?);
        self.sessions
            .lock()
            .unwrap()
            .insert(key, Arc::clone(&session));
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
        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let Some(spec) = detect_lsp_server(&ext) else {
            return Err(format!("no LSP server mapping for '.{ext}'"));
        };
        let session = self.get_or_spawn(workspace_root, &spec)?;
        let res = (|| -> Result<Value, String> {
            if action == "workspace_symbols" {
                return session.request("workspace/symbol", json!({"query": query.unwrap_or("")}));
            }
            let uri = session.sync_document(path, spec.language_id)?;
            match action {
                "hover" => session.request(
                    "textDocument/hover",
                    json!({"textDocument": {"uri": uri}, "position": {"line": line, "character": character}}),
                ),
                "definition" => session.request(
                    "textDocument/definition",
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
                        session.request("textDocument/diagnostic", json!({"textDocument": {"uri": uri}}))
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
            let key = (workspace_root.to_path_buf(), spec.language_id.to_string());
            self.sessions.lock().unwrap().remove(&key);
        }
        res
    }

    pub fn fallback_diagnostics(&self, workspace_root: &Path, path: &Path) -> Vec<Value> {
        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        match ext.as_str() {
            "py" => run_python_ast_check(path),
            "json" => run_json_syntax_check(path),
            "rs" => run_rust_quick_check(workspace_root),
            _ => Vec::new(),
        }
    }
}

/// Pure read-only Python AST syntax check (NEVER creates `__pycache__/*.pyc`!).
fn run_python_ast_check(path: &Path) -> Vec<Value> {
    let Ok(out) = Command::new("python3")
        .args([
            "-B",
            "-c",
            "import ast,sys; ast.parse(open(sys.argv[1], encoding='utf-8').read(), filename=sys.argv[1])",
        ])
        .arg(path)
        .output()
    else {
        return Vec::new();
    };
    if out.status.success() {
        return Vec::new();
    }
    vec![json!({
        "severity": "error",
        "source": "python_ast",
        "message": String::from_utf8_lossy(&out.stderr).trim()
    })]
}

fn run_json_syntax_check(path: &Path) -> Vec<Value> {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    match serde_json::from_str::<Value>(&raw) {
        Ok(_) => Vec::new(),
        Err(e) => vec![json!({
            "severity": "error",
            "source": "json",
            "line": e.line(),
            "column": e.column(),
            "message": e.to_string()
        })],
    }
}

fn run_rust_quick_check(workspace_root: &Path) -> Vec<Value> {
    if !workspace_root.join("Cargo.toml").exists() {
        return Vec::new();
    }
    let Ok(out) = Command::new("cargo")
        .args(["check", "--message-format=short", "--quiet"])
        .current_dir(workspace_root)
        .output()
    else {
        return Vec::new();
    };
    if out.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&out.stderr)
        .lines()
        .filter(|l| l.contains("error"))
        .take(20)
        .map(|l| json!({"severity": "error", "source": "cargo", "message": l}))
        .collect()
}

pub struct LspDiagnosticsTool;

impl ToolHandler for LspDiagnosticsTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "lsp".into(),
                description: "Query Language Server Protocol (diagnostics, definition, references, hover, document_symbols, workspace_symbols) with automatic read-only syntax fallback.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": ["diagnostics", "definition", "references", "hover", "document_symbols", "workspace_symbols"],
                            "description": "Default 'diagnostics'"
                        },
                        "path": { "type": "string" },
                        "line": { "type": "integer", "description": "0-based line number" },
                        "character": { "type": "integer", "description": "0-based character column" },
                        "query": { "type": "string", "description": "Query string for workspace_symbols" }
                    },
                    "required": ["path"]
                }),
            },
            tier: ToolTier::Deferred,
            domain: CapabilityDomain::FilesRead,
            read_only: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, args: Value) -> ToolExecutionResult {
        let Some(raw_path) = args.get("path").and_then(|v| v.as_str()) else {
            return ToolExecutionResult::err_structured("missing_param", "missing 'path'", None);
        };
        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("diagnostics");
        let line = args.get("line").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        let character = args.get("character").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        let query = args.get("query").and_then(|v| v.as_str());
        let path = resolve_path(&ctx.workspace_root, raw_path);

        match ctx
            .lsp
            .query(&ctx.workspace_root, &path, action, line, character, query)
        {
            Ok(result) => ToolExecutionResult::ok_structured(
                json!({
                    "backend": "lsp",
                    "action": action,
                    "path": path.display().to_string(),
                    "result": result
                }),
                format!("LSP {action} succeeded for {}", path.display()),
            ),
            Err(lsp_err) if action == "diagnostics" => {
                let diags = ctx.lsp.fallback_diagnostics(&ctx.workspace_root, &path);
                let count = diags.len();
                ToolExecutionResult::ok_structured(
                    json!({
                        "backend": "fallback_syntax",
                        "lsp_note": lsp_err,
                        "path": path.display().to_string(),
                        "count": count,
                        "diagnostics": diags
                    }),
                    format!("Fallback diagnostics ({count}) for {}", path.display()),
                )
            }
            Err(e) => ToolExecutionResult::err_structured("lsp_unavailable", e, None),
        }
    }
}
