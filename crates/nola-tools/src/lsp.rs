// SPDX-License-Identifier: Apache-2.0
//! Non-killing per-workspace diagnostic & LSP pool (`lsp_diagnostics`).
//! Supports lightweight syntax/type checks without holding a global mutex across child IO.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

use nola_types::{CapabilityDomain, ToolExecutionResult, ToolSchema, ToolSpec, ToolTier};
use serde_json::{json, Value};

use crate::file_pipeline::resolve_path;
use crate::{ToolContext, ToolHandler};

/// Per-workspace diagnostic cache & server handle registry (never kills sibling workspaces!).
#[derive(Default)]
pub struct LspPool {
    cached_diagnostics: Mutex<BTreeMap<PathBuf, Vec<Value>>>,
}

impl LspPool {
    pub fn check_file(&self, workspace_root: &Path, path: &Path) -> Vec<Value> {
        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();

        let diags = match ext.as_str() {
            "py" => run_python_syntax_check(path),
            "json" => run_json_syntax_check(path),
            "rs" => run_rust_quick_check(workspace_root),
            _ => Vec::new(),
        };
        if let Ok(mut g) = self.cached_diagnostics.lock() {
            g.insert(path.to_path_buf(), diags.clone());
        }
        diags
    }
}

fn run_python_syntax_check(path: &Path) -> Vec<Value> {
    let Ok(out) = Command::new("python3")
        .args(["-m", "py_compile"])
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
        "source": "py_compile",
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
                name: "lsp_diagnostics".into(),
                description: "Run non-blocking syntax/compiler diagnostics for a workspace file.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string" }
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
        let path = resolve_path(&ctx.workspace_root, raw_path);
        let diags = ctx.lsp.check_file(&ctx.workspace_root, &path);
        let count = diags.len();
        ToolExecutionResult::ok_structured(
            json!({
                "path": path.display().to_string(),
                "count": count,
                "diagnostics": diags
            }),
            format!("Collected {count} diagnostic(s) for {}", path.display()),
        )
    }
}
