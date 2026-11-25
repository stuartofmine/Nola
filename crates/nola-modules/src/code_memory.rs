// SPDX-License-Identifier: Apache-2.0
//! Native Rust Code-Memory workspace symbol & text indexer backed by SQLite
//! `module_records` (`kind = "code_memory_index"`).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path};
use std::sync::{Arc, OnceLock};
use std::time::{Instant, UNIX_EPOCH};

use ignore::WalkBuilder;
use nola_store::Store;
use nola_tools::{path_within_workspace, ToolContext, ToolHandler, ToolRegistry};
use nola_types::{
    now_ms, sha256_hex, CapabilityDomain, ToolExecutionResult, ToolSchema, ToolSpec, ToolTier,
};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::ModulesSuite;

pub const KIND_CODE_MEMORY_INDEX: &str = "code_memory_index";

const MAX_INDEXED_FILES: usize = 5_000;
const MAX_INDEXED_FILE_BYTES: u64 = 1024 * 1024; // 1 MiB per source file
const MAX_SNIPPET_CONTEXT_LINES: usize = 120;

const SUPPORTED_EXTENSIONS: &[&str] = &[
    "rs", "ts", "tsx", "js", "jsx", "svelte", "py", "kt", "swift", "go", "java", "c", "cpp", "h",
];

const SKIP_DIR_NAMES: &[&str] = &[
    ".git",
    "target",
    "node_modules",
    "build",
    "dist",
    ".gradle",
    ".next",
    ".svelte-kit",
    "__pycache__",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeSymbol {
    pub name: String,
    pub kind: String,
    pub file: String,
    pub line: usize,
    pub signature: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexedFileEntry {
    pub path: String,
    pub mtime_ms: i64,
    pub size: u64,
    pub symbols: Vec<CodeSymbol>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceCodeIndex {
    pub workspace_root: String,
    pub indexed_at_ms: i64,
    pub files: BTreeMap<String, IndexedFileEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexStats {
    pub workspace_root: String,
    pub files_indexed: usize,
    pub files_reused: usize,
    pub total_files: usize,
    pub total_symbols: usize,
    pub duration_ms: u64,
}

fn index_storage_id(workspace_root: &Path) -> String {
    let canonical = workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.to_path_buf());
    sha256_hex(canonical.to_string_lossy().as_bytes())
}

fn is_supported_source_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| {
            let lower = e.to_ascii_lowercase();
            SUPPORTED_EXTENSIONS.contains(&lower.as_str())
        })
        .unwrap_or(false)
}

fn has_skipped_component(rel_path: &Path) -> bool {
    rel_path.components().any(|c| {
        matches!(c, Component::Normal(name) if name
            .to_str()
            .is_some_and(|s| SKIP_DIR_NAMES.contains(&s)))
    })
}

fn normalize_rel_path(workspace_root: &Path, abs_path: &Path) -> String {
    let rel = abs_path.strip_prefix(workspace_root).unwrap_or(abs_path);
    rel.to_string_lossy().replace('\\', "/")
}

fn file_mtime_ms(meta: &fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn walk_source_files(workspace_root: &Path) -> Vec<(String, std::path::PathBuf)> {
    let mut files = Vec::new();
    let mut builder = WalkBuilder::new(workspace_root);
    builder
        .hidden(false)
        .git_ignore(true)
        .git_global(false)
        .git_exclude(true)
        .ignore(true)
        .parents(false)
        .filter_entry(|entry| {
            if entry.depth() == 0 {
                return true;
            }
            if let Some(name) = entry.file_name().to_str() {
                if SKIP_DIR_NAMES.contains(&name) {
                    return false;
                }
            }
            true
        });

    for entry in builder.build().flatten() {
        if files.len() >= MAX_INDEXED_FILES {
            break;
        }
        let path = entry.path();
        if !path.is_file() || !is_supported_source_file(path) {
            continue;
        }
        let rel = path.strip_prefix(workspace_root).unwrap_or(path);
        if has_skipped_component(rel) {
            continue;
        }
        files.push((normalize_rel_path(workspace_root, path), path.to_path_buf()));
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    files
}

struct SymbolMatchers {
    rust: Vec<(&'static str, Regex)>,
    ts_js: Vec<(&'static str, Regex)>,
    python: Vec<(&'static str, Regex)>,
    go: Vec<(&'static str, Regex)>,
    java_kt: Vec<(&'static str, Regex)>,
    swift: Vec<(&'static str, Regex)>,
    c_cpp: Vec<(&'static str, Regex)>,
}

fn matchers() -> &'static SymbolMatchers {
    static MATCHERS: OnceLock<SymbolMatchers> = OnceLock::new();
    MATCHERS.get_or_init(|| SymbolMatchers {
        rust: vec![
            (
                "function",
                Regex::new(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?(?:unsafe\s+)?(?:const\s+)?(?:extern\s+\S+\s+)?fn\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap(),
            ),
            (
                "struct",
                Regex::new(r"^\s*(?:pub(?:\([^)]*\))?\s+)?struct\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap(),
            ),
            (
                "enum",
                Regex::new(r"^\s*(?:pub(?:\([^)]*\))?\s+)?enum\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap(),
            ),
            (
                "trait",
                Regex::new(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:unsafe\s+)?trait\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap(),
            ),
            (
                "type",
                Regex::new(r"^\s*(?:pub(?:\([^)]*\))?\s+)?type\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap(),
            ),
            (
                "module",
                Regex::new(r"^\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap(),
            ),
        ],
        ts_js: vec![
            (
                "function",
                Regex::new(r"^\s*(?:export\s+)?(?:default\s+)?(?:async\s+)?function\s*\*?\s+([A-Za-z_$][A-Za-z0-9_$]*)").unwrap(),
            ),
            (
                "class",
                Regex::new(r"^\s*(?:export\s+)?(?:default\s+)?(?:abstract\s+)?class\s+([A-Za-z_$][A-Za-z0-9_$]*)").unwrap(),
            ),
            (
                "interface",
                Regex::new(r"^\s*(?:export\s+)?interface\s+([A-Za-z_$][A-Za-z0-9_$]*)").unwrap(),
            ),
            (
                "type",
                Regex::new(r"^\s*(?:export\s+)?type\s+([A-Za-z_$][A-Za-z0-9_$]*)\s*(?:<[^>]*>)?\s*=").unwrap(),
            ),
            (
                "enum",
                Regex::new(r"^\s*(?:export\s+)?(?:const\s+)?enum\s+([A-Za-z_$][A-Za-z0-9_$]*)").unwrap(),
            ),
            (
                "export",
                Regex::new(r"^\s*export\s+(?:const|let|var)\s+([A-Za-z_$][A-Za-z0-9_$]*)").unwrap(),
            ),
        ],
        python: vec![
            (
                "function",
                Regex::new(r"^\s*(?:async\s+)?def\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap(),
            ),
            (
                "class",
                Regex::new(r"^\s*class\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap(),
            ),
        ],
        go: vec![
            (
                "method",
                Regex::new(r"^\s*func\s*\([^)]+\)\s*([A-Za-z_][A-Za-z0-9_]*)").unwrap(),
            ),
            (
                "function",
                Regex::new(r"^\s*func\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap(),
            ),
            (
                "struct",
                Regex::new(r"^\s*type\s+([A-Za-z_][A-Za-z0-9_]*)\s+struct\b").unwrap(),
            ),
            (
                "interface",
                Regex::new(r"^\s*type\s+([A-Za-z_][A-Za-z0-9_]*)\s+interface\b").unwrap(),
            ),
        ],
        java_kt: vec![
            (
                "class",
                Regex::new(r"^\s*(?:(?:public|private|protected|internal|abstract|final|open|data|sealed|static)\s+)*(?:class|object)\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap(),
            ),
            (
                "interface",
                Regex::new(r"^\s*(?:(?:public|private|protected|internal|sealed)\s+)*interface\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap(),
            ),
            (
                "enum",
                Regex::new(r"^\s*(?:(?:public|private|protected|internal)\s+)*enum\s+(?:class\s+)?([A-Za-z_][A-Za-z0-9_]*)").unwrap(),
            ),
            (
                "function",
                Regex::new(r"^\s*(?:(?:public|private|protected|internal|override|suspend|inline|open|final|static)\s+)*fun\s+(?:<[^>]+>\s+)?(?:[A-Za-z0-9_<>?]+\.)?([A-Za-z_][A-Za-z0-9_]*)").unwrap(),
            ),
            (
                "method",
                Regex::new(r"^\s*(?:(?:public|private|protected|static|final|synchronized|abstract)\s+)+[A-Za-z0-9_<>\[\]?]+\s+([A-Za-z_][A-Za-z0-9_]*)\s*\(").unwrap(),
            ),
        ],
        swift: vec![
            (
                "function",
                Regex::new(r"^\s*(?:(?:public|private|fileprivate|internal|open|static|class|mutating|nonmutating|override|final)\s+)*func\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap(),
            ),
            (
                "struct",
                Regex::new(r"^\s*(?:(?:public|private|fileprivate|internal|open|final)\s+)*struct\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap(),
            ),
            (
                "class",
                Regex::new(r"^\s*(?:(?:public|private|fileprivate|internal|open|final)\s+)*(?:class|actor)\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap(),
            ),
            (
                "enum",
                Regex::new(r"^\s*(?:(?:public|private|fileprivate|internal|open|indirect)\s+)*enum\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap(),
            ),
            (
                "interface",
                Regex::new(r"^\s*(?:(?:public|private|fileprivate|internal|open)\s+)*protocol\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap(),
            ),
        ],
        c_cpp: vec![
            (
                "struct",
                Regex::new(r"^\s*(?:typedef\s+)?struct\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap(),
            ),
            (
                "class",
                Regex::new(r"^\s*class\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap(),
            ),
            (
                "enum",
                Regex::new(r"^\s*(?:typedef\s+)?enum\s+(?:class\s+)?([A-Za-z_][A-Za-z0-9_]*)").unwrap(),
            ),
            (
                "function",
                Regex::new(r"^(?:static\s+|inline\s+|extern\s+|virtual\s+|constexpr\s+)*[A-Za-z_][A-Za-z0-9_:<>*&\s]{0,60}\s+([A-Za-z_][A-Za-z0-9_]*)\s*\([^;]*\)\s*(?:const\s*)?(?:\{|$)").unwrap(),
            ),
        ],
    })
}

fn extract_symbols(rel_path: &str, ext: &str, content: &str) -> Vec<CodeSymbol> {
    let m = matchers();
    let rules: &[(&str, Regex)] = match ext {
        "rs" => &m.rust,
        "ts" | "tsx" | "js" | "jsx" | "svelte" => &m.ts_js,
        "py" => &m.python,
        "go" => &m.go,
        "java" | "kt" => &m.java_kt,
        "swift" => &m.swift,
        "c" | "cpp" | "h" => &m.c_cpp,
        _ => return Vec::new(),
    };

    let mut symbols = Vec::new();
    for (idx, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty()
            || trimmed.starts_with("//")
            || trimmed.starts_with("/*")
            || trimmed.starts_with('*')
            || trimmed.starts_with('#')
        {
            continue;
        }
        for (kind, re) in rules {
            if let Some(caps) = re.captures(line) {
                if let Some(name_match) = caps.get(1) {
                    let name = name_match.as_str();
                    // Filter common C/Java control-flow keywords mis-captured as functions
                    if matches!(name, "if" | "for" | "while" | "switch" | "catch" | "return") {
                        break;
                    }
                    let signature: String = trimmed.chars().take(160).collect();
                    symbols.push(CodeSymbol {
                        name: name.to_string(),
                        kind: (*kind).to_string(),
                        file: rel_path.to_string(),
                        line: idx + 1,
                        signature,
                    });
                    break;
                }
            }
        }
    }
    symbols
}

/// Index or incrementally update workspace symbols in SQLite `module_records`.
pub fn index_workspace(
    store: &Store,
    workspace_root: &Path,
    force_full: bool,
) -> Result<IndexStats, String> {
    let start = Instant::now();
    if !workspace_root.exists() {
        return Err(format!(
            "workspace root '{}' does not exist",
            workspace_root.display()
        ));
    }

    let key = index_storage_id(workspace_root);
    let existing = if force_full {
        None
    } else {
        store
            .get_module_record::<WorkspaceCodeIndex>(KIND_CODE_MEMORY_INDEX, &key)
            .map_err(|e| e.to_string())?
    };
    let prev_files = existing.map(|idx| idx.files).unwrap_or_default();

    let walked = walk_source_files(workspace_root);
    let mut next_files = BTreeMap::new();
    let mut files_indexed = 0usize;
    let mut files_reused = 0usize;

    for (rel_path, abs_path) in walked {
        let Ok(meta) = fs::metadata(&abs_path) else {
            continue;
        };
        let size = meta.len();
        if size > MAX_INDEXED_FILE_BYTES {
            continue;
        }
        let mtime = file_mtime_ms(&meta);

        if let Some(prev) = prev_files.get(&rel_path) {
            if prev.mtime_ms == mtime && prev.size == size {
                next_files.insert(rel_path, prev.clone());
                files_reused += 1;
                continue;
            }
        }

        let Ok(content) = fs::read_to_string(&abs_path) else {
            continue;
        };
        let ext = abs_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let symbols = extract_symbols(&rel_path, &ext, &content);
        next_files.insert(
            rel_path.clone(),
            IndexedFileEntry {
                path: rel_path,
                mtime_ms: mtime,
                size,
                symbols,
            },
        );
        files_indexed += 1;
    }

    let total_files = next_files.len();
    let total_symbols: usize = next_files.values().map(|f| f.symbols.len()).sum();

    let index = WorkspaceCodeIndex {
        workspace_root: workspace_root.display().to_string(),
        indexed_at_ms: now_ms(),
        files: next_files,
    };
    store
        .put_module_record(KIND_CODE_MEMORY_INDEX, &key, &index)
        .map_err(|e| e.to_string())?;

    Ok(IndexStats {
        workspace_root: workspace_root.display().to_string(),
        files_indexed,
        files_reused,
        total_files,
        total_symbols,
        duration_ms: start.elapsed().as_millis() as u64,
    })
}

fn load_or_build_index(
    store: &Store,
    workspace_root: &Path,
) -> Result<WorkspaceCodeIndex, String> {
    let key = index_storage_id(workspace_root);
    if let Some(idx) = store
        .get_module_record::<WorkspaceCodeIndex>(KIND_CODE_MEMORY_INDEX, &key)
        .map_err(|e| e.to_string())?
    {
        return Ok(idx);
    }
    index_workspace(store, workspace_root, false)?;
    store
        .get_module_record::<WorkspaceCodeIndex>(KIND_CODE_MEMORY_INDEX, &key)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "failed to load newly built code memory index".to_string())
}

/// Search indexed symbols by name, kind, and optional path substring with ranked matches.
pub fn search_symbols(
    store: &Store,
    workspace_root: &Path,
    query: &str,
    kind: Option<&str>,
    path_filter: Option<&str>,
    limit: usize,
) -> Result<Vec<Value>, String> {
    let idx = load_or_build_index(store, workspace_root)?;
    let q_lower = query.trim().to_ascii_lowercase();
    let kind_lower = kind.map(|k| k.trim().to_ascii_lowercase());
    let path_lower = path_filter.map(|p| p.trim().to_ascii_lowercase());
    let cap = limit.clamp(1, 200);

    let mut scored: Vec<(u32, &CodeSymbol)> = Vec::new();
    for entry in idx.files.values() {
        if let Some(ref pf) = path_lower {
            if !pf.is_empty() && !entry.path.to_ascii_lowercase().contains(pf) {
                continue;
            }
        }
        for sym in &entry.symbols {
            if let Some(ref kf) = kind_lower {
                if !kf.is_empty() && sym.kind.to_ascii_lowercase() != *kf {
                    continue;
                }
            }
            let score = if q_lower.is_empty() {
                10
            } else {
                let name_l = sym.name.to_ascii_lowercase();
                let sig_l = sym.signature.to_ascii_lowercase();
                let file_l = sym.file.to_ascii_lowercase();
                if name_l == q_lower {
                    100
                } else if name_l.starts_with(&q_lower) {
                    80
                } else if name_l.contains(&q_lower) {
                    60
                } else if sig_l.contains(&q_lower) {
                    35
                } else if file_l.contains(&q_lower) {
                    20
                } else {
                    0
                }
            };
            if score > 0 {
                scored.push((score, sym));
            }
        }
    }

    scored.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| a.1.file.cmp(&b.1.file))
            .then_with(|| a.1.line.cmp(&b.1.line))
    });

    Ok(scored
        .into_iter()
        .take(cap)
        .map(|(score, sym)| {
            json!({
                "name": sym.name,
                "kind": sym.kind,
                "file": sym.file,
                "line": sym.line,
                "signature": sym.signature,
                "score": score
            })
        })
        .collect())
}

/// Bounded fast literal or regex text search across workspace source files.
pub fn search_text(
    workspace_root: &Path,
    query: &str,
    is_regex: bool,
    path_filter: Option<&str>,
    limit: usize,
) -> Result<Vec<Value>, String> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Err("query must not be empty".into());
    }
    let compiled = if is_regex {
        Some(Regex::new(trimmed).map_err(|e| format!("invalid regex: {e}"))?)
    } else {
        None
    };
    let path_lower = path_filter.map(|p| p.trim().to_ascii_lowercase());
    let cap = limit.clamp(1, 200);
    let files = walk_source_files(workspace_root);
    let mut hits = Vec::new();
    let needle_lower = trimmed.to_ascii_lowercase();

    for (rel_path, abs_path) in files {
        if hits.len() >= cap {
            break;
        }
        if let Some(ref pf) = path_lower {
            if !pf.is_empty() && !rel_path.to_ascii_lowercase().contains(pf) {
                continue;
            }
        }
        let Ok(meta) = fs::metadata(&abs_path) else {
            continue;
        };
        if meta.len() > MAX_INDEXED_FILE_BYTES {
            continue;
        }
        let Ok(content) = fs::read_to_string(&abs_path) else {
            continue;
        };
        for (idx, line) in content.lines().enumerate() {
            let matched = match &compiled {
                Some(re) => re.is_match(line),
                None => line.to_ascii_lowercase().contains(&needle_lower),
            };
            if matched {
                let snippet: String = line.chars().take(240).collect();
                hits.push(json!({
                    "file": rel_path,
                    "line": idx + 1,
                    "snippet": snippet
                }));
                if hits.len() >= cap {
                    break;
                }
            }
        }
    }
    Ok(hits)
}

/// Read the exact source snippet around a symbol's file and line range.
pub fn read_symbol_snippet(
    store: &Store,
    workspace_root: &Path,
    name: Option<&str>,
    file: Option<&str>,
    line: Option<usize>,
    context_lines: usize,
) -> Result<Value, String> {
    let mut resolved_file = file.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let mut resolved_line = line;
    let mut resolved_symbol: Option<Value> = None;

    if resolved_file.is_none() || resolved_line.is_none() {
        let Some(sym_name) = name.map(str::trim).filter(|s| !s.is_empty()) else {
            return Err("provide either 'name' or both 'file' and 'line'".into());
        };
        let matches = search_symbols(
            store,
            workspace_root,
            sym_name,
            None,
            resolved_file.as_deref(),
            5,
        )?;
        let Some(best) = matches.into_iter().next() else {
            return Err(format!("symbol '{sym_name}' not found in workspace index"));
        };
        resolved_file = best
            .get("file")
            .and_then(|v| v.as_str())
            .map(ToString::to_string);
        resolved_line = best
            .get("line")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize);
        resolved_symbol = Some(best);
    }

    let rel_file = resolved_file.ok_or_else(|| "missing file path".to_string())?;
    let target_line = resolved_line.unwrap_or(1).max(1);
    let abs_path = workspace_root.join(&rel_file);
    if !path_within_workspace(workspace_root, &abs_path) {
        return Err(format!("path '{rel_file}' escapes workspace root"));
    }
    let content = fs::read_to_string(&abs_path)
        .map_err(|e| format!("failed to read '{rel_file}': {e}"))?;
    let all_lines: Vec<&str> = content.lines().collect();
    if all_lines.is_empty() {
        return Ok(json!({
            "file": rel_file,
            "start_line": 1,
            "end_line": 0,
            "target_line": target_line,
            "symbol": resolved_symbol,
            "snippet": ""
        }));
    }

    let ctx = context_lines.clamp(1, MAX_SNIPPET_CONTEXT_LINES);
    let start_line = target_line.saturating_sub(ctx / 3).max(1);
    let end_line = (target_line + ctx).min(all_lines.len());
    let mut snippet = String::new();
    for line_no in start_line..=end_line {
        let text = all_lines.get(line_no - 1).copied().unwrap_or("");
        snippet.push_str(&format!("{line_no:>5} | {text}\n"));
    }

    Ok(json!({
        "file": rel_file,
        "start_line": start_line,
        "end_line": end_line,
        "target_line": target_line,
        "symbol": resolved_symbol,
        "snippet": snippet
    }))
}

pub fn register_tools(suite: &Arc<ModulesSuite>, registry: &mut ToolRegistry) {
    registry.register(Arc::new(CodebaseIndexTool {
        suite: Arc::clone(suite),
    }));
    registry.register(Arc::new(CodebaseSearchTool {
        suite: Arc::clone(suite),
    }));
    registry.register(Arc::new(CodebaseTextSearchTool {
        suite: Arc::clone(suite),
    }));
    registry.register(Arc::new(CodebaseReadSymbolTool {
        suite: Arc::clone(suite),
    }));
}

struct CodebaseIndexTool {
    suite: Arc<ModulesSuite>,
}

impl ToolHandler for CodebaseIndexTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "codebase_index".into(),
                description: "Index or incrementally update workspace source symbols into SQLite Code-Memory.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "force_full": { "type": "boolean", "description": "Re-parse all files ignoring cached mtime/size (default false)" }
                    }
                }),
            },
            tier: ToolTier::Deferred,
            domain: CapabilityDomain::Memory,
            read_only: false,
        }
    }

    fn execute(&self, ctx: &ToolContext, args: Value) -> ToolExecutionResult {
        let force_full = args
            .get("force_full")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let root = if ctx.workspace_root.exists() {
            &ctx.workspace_root
        } else {
            &self.suite.workspace_root
        };
        match index_workspace(&self.suite.store, root, force_full) {
            Ok(stats) => ToolExecutionResult::ok_structured(
                serde_json::to_value(&stats).unwrap_or(json!({})),
                format!(
                    "Indexed {} files ({} symbols, {} reused) in {}ms",
                    stats.total_files, stats.total_symbols, stats.files_reused, stats.duration_ms
                ),
            ),
            Err(e) => ToolExecutionResult::err_structured("codebase_index_error", e, None),
        }
    }
}

struct CodebaseSearchTool {
    suite: Arc<ModulesSuite>,
}

impl ToolHandler for CodebaseSearchTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "codebase_search".into(),
                description: "Search indexed workspace symbols by name, kind, or path with ranked matches.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "Symbol name or substring" },
                        "kind": { "type": "string", "description": "Optional symbol kind (function, struct, class, enum, trait, interface, method, export)" },
                        "path": { "type": "string", "description": "Optional relative path substring filter" },
                        "limit": { "type": "integer", "description": "Maximum matches (default 20)" }
                    },
                    "required": ["query"]
                }),
            },
            tier: ToolTier::Deferred,
            domain: CapabilityDomain::Memory,
            read_only: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, args: Value) -> ToolExecutionResult {
        let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
        let kind = args.get("kind").and_then(|v| v.as_str());
        let path_filter = args.get("path").and_then(|v| v.as_str());
        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(20) as usize;
        let root = if ctx.workspace_root.exists() {
            &ctx.workspace_root
        } else {
            &self.suite.workspace_root
        };
        match search_symbols(&self.suite.store, root, query, kind, path_filter, limit) {
            Ok(matches) => {
                let count = matches.len();
                let files: BTreeSet<&str> = matches
                    .iter()
                    .filter_map(|m| m.get("file").and_then(|v| v.as_str()))
                    .collect();
                ToolExecutionResult::ok_structured(
                    json!({
                        "query": query,
                        "count": count,
                        "files_matched": files.len(),
                        "matches": matches
                    }),
                    format!("Found {count} symbol match(es) for '{query}'"),
                )
            }
            Err(e) => ToolExecutionResult::err_structured("codebase_search_error", e, None),
        }
    }
}

struct CodebaseTextSearchTool {
    suite: Arc<ModulesSuite>,
}

impl ToolHandler for CodebaseTextSearchTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "codebase_text_search".into(),
                description: "Fast bounded literal or regex text search across indexed workspace source files.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "query": { "type": "string" },
                        "regex": { "type": "boolean", "description": "Treat query as regex (default false)" },
                        "path": { "type": "string", "description": "Optional relative path substring filter" },
                        "limit": { "type": "integer", "description": "Maximum matches (default 30)" }
                    },
                    "required": ["query"]
                }),
            },
            tier: ToolTier::Deferred,
            domain: CapabilityDomain::Memory,
            read_only: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, args: Value) -> ToolExecutionResult {
        let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
        let is_regex = args.get("regex").and_then(|v| v.as_bool()).unwrap_or(false);
        let path_filter = args.get("path").and_then(|v| v.as_str());
        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(30) as usize;
        let root = if ctx.workspace_root.exists() {
            &ctx.workspace_root
        } else {
            &self.suite.workspace_root
        };
        match search_text(root, query, is_regex, path_filter, limit) {
            Ok(matches) => {
                let count = matches.len();
                ToolExecutionResult::ok_structured(
                    json!({
                        "query": query,
                        "regex": is_regex,
                        "count": count,
                        "matches": matches
                    }),
                    format!("Found {count} text match(es) for '{query}'"),
                )
            }
            Err(e) => ToolExecutionResult::err_structured("codebase_text_search_error", e, None),
        }
    }
}

struct CodebaseReadSymbolTool {
    suite: Arc<ModulesSuite>,
}

impl ToolHandler for CodebaseReadSymbolTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "codebase_read_symbol".into(),
                description: "Read the exact source snippet around a symbol's file and line range.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "name": { "type": "string", "description": "Symbol name to resolve in the index" },
                        "file": { "type": "string", "description": "Optional relative file path" },
                        "line": { "type": "integer", "description": "Optional 1-based line number" },
                        "context_lines": { "type": "integer", "description": "Lines of snippet context (default 30)" }
                    }
                }),
            },
            tier: ToolTier::Deferred,
            domain: CapabilityDomain::Memory,
            read_only: true,
        }
    }

    fn execute(&self, ctx: &ToolContext, args: Value) -> ToolExecutionResult {
        let name = args.get("name").and_then(|v| v.as_str());
        let file = args.get("file").and_then(|v| v.as_str());
        let line = args
            .get("line")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize);
        let context_lines = args
            .get("context_lines")
            .and_then(|v| v.as_u64())
            .unwrap_or(30) as usize;
        let root = if ctx.workspace_root.exists() {
            &ctx.workspace_root
        } else {
            &self.suite.workspace_root
        };
        match read_symbol_snippet(&self.suite.store, root, name, file, line, context_lines) {
            Ok(data) => {
                let target_file = data["file"].as_str().unwrap_or("").to_string();
                let target_line = data["target_line"].as_u64().unwrap_or(1);
                ToolExecutionResult::ok_structured(
                    data,
                    format!("Read symbol snippet at {target_file}:{target_line}"),
                )
            }
            Err(e) => ToolExecutionResult::err_structured("codebase_read_symbol_error", e, None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_memory_indexes_searches_and_reads_symbols_incrementally() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        fs::create_dir_all(&src).unwrap();
        fs::create_dir_all(dir.path().join("node_modules/pkg")).unwrap();
        fs::write(
            dir.path().join("node_modules/pkg/ignored.ts"),
            "export function shouldNeverBeIndexed() {}\n",
        )
        .unwrap();
        fs::write(
            src.join("engine.rs"),
            "pub struct QuantumPlanner {\n    pub depth: usize,\n}\n\npub fn compute_trajectory(step: usize) -> usize {\n    step * 42\n}\n",
        )
        .unwrap();
        fs::write(
            src.join("widget.ts"),
            "export interface DashboardState { ready: boolean }\nexport function renderDashboard(state: DashboardState) { return state.ready; }\n",
        )
        .unwrap();

        let store = Store::open_in_memory().unwrap();
        let suite = Arc::new(ModulesSuite::new(
            store.clone(),
            dir.path().to_path_buf(),
            None,
        ));
        let mut reg = ToolRegistry::with_builtin_tools();
        suite.register_tools(&mut reg);
        let ctx = ToolContext::new(dir.path());

        // 1. First index parses both source files and skips node_modules
        let idx1 = reg.execute("codebase_index", &ctx, json!({}));
        assert!(!idx1.is_error, "{}", idx1.payload.to_model_value());
        let idx1_v = idx1.payload.to_model_value();
        assert_eq!(idx1_v["total_files"], 2);
        assert_eq!(idx1_v["files_indexed"], 2);
        assert!(idx1_v["total_symbols"].as_u64().unwrap() >= 4);

        // 2. Incremental second index reuses cached entries
        let idx2 = reg.execute("codebase_index", &ctx, json!({}));
        assert!(!idx2.is_error);
        let idx2_v = idx2.payload.to_model_value();
        assert_eq!(idx2_v["files_indexed"], 0);
        assert_eq!(idx2_v["files_reused"], 2);

        // 3. Symbol search finds Rust & TS symbols and skips ignored dirs
        let sym_hit = reg.execute(
            "codebase_search",
            &ctx,
            json!({"query": "compute_trajectory", "kind": "function"}),
        );
        assert!(!sym_hit.is_error);
        let sym_v = sym_hit.payload.to_model_value();
        assert_eq!(sym_v["count"], 1);
        assert_eq!(sym_v["matches"][0]["file"], "src/engine.rs");
        assert_eq!(sym_v["matches"][0]["line"], 5);

        let ignored_hit = reg.execute(
            "codebase_search",
            &ctx,
            json!({"query": "shouldNeverBeIndexed"}),
        );
        assert_eq!(ignored_hit.payload.to_model_value()["count"], 0);

        // 4. Text search (literal & regex)
        let txt_hit = reg.execute(
            "codebase_text_search",
            &ctx,
            json!({"query": "step \\* 42", "regex": true}),
        );
        assert!(!txt_hit.is_error);
        assert_eq!(txt_hit.payload.to_model_value()["count"], 1);

        // 5. Read symbol snippet
        let read_res = reg.execute(
            "codebase_read_symbol",
            &ctx,
            json!({"name": "renderDashboard", "context_lines": 10}),
        );
        assert!(!read_res.is_error, "{}", read_res.payload.to_model_value());
        let read_v = read_res.payload.to_model_value();
        assert_eq!(read_v["file"], "src/widget.ts");
        assert!(read_v["snippet"]
            .as_str()
            .unwrap()
            .contains("renderDashboard"));
    }
}
