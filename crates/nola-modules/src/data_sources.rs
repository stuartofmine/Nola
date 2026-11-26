// SPDX-License-Identifier: Apache-2.0
//! DataSources registry, real bounded connection probing, and usage provenance tracking.

use std::fs::{self, File};
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use ignore::WalkBuilder;
use nola_store::{DocChunkRecord, Store};
use nola_tools::web::resolve_and_validate_public_url;
use nola_tools::{extract_document_text_from_bytes, ToolContext, ToolHandler, ToolRegistry};
use nola_types::{
    new_id, now_ms, sha256_hex, CapabilityDomain, DataSourceRecord, ToolExecutionResult,
    ToolSchema, ToolSpec, ToolTier,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::ModulesSuite;

pub const KIND_DATA_SOURCE: &str = "data_source";
pub const KIND_DATA_SOURCE_USAGE: &str = "data_source_usage";

const MAX_HTTP_PROBE_BYTES: u64 = 64 * 1024;
const MAX_LOCAL_PREVIEW_BYTES: usize = 4 * 1024;
const MAX_DIR_ENTRIES_SCAN: usize = 200;
const MAX_INDEX_FILES: usize = 2_000;
const MAX_INDEX_FILE_BYTES: u64 = 4 * 1024 * 1024;
const DEFAULT_CHUNK_CHARS: usize = 1_200;
const DEFAULT_CHUNK_OVERLAP_CHARS: usize = 200;
const SKIP_INDEX_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "build",
    "dist",
    "__pycache__",
];

const SENSITIVE_DIR_NAMES: &[&str] = &[
    ".ssh",
    ".aws",
    ".gnupg",
    ".kube",
    ".docker",
    ".azure",
    ".gcloud",
];

const SENSITIVE_FILE_NAMES: &[&str] = &[
    ".env",
    ".netrc",
    ".npmrc",
    ".pypirc",
    ".git-credentials",
    ".pgpass",
    ".my.cnf",
    "id_rsa",
    "id_dsa",
    "id_ecdsa",
    "id_ed25519",
    "id_ed25519_sk",
    "id_ecdsa_sk",
    "authorized_keys",
    "known_hosts",
    "vault.json",
    "vault.enc",
    "vault.db",
    "vault.key",
    "master.key",
    "credentials",
    "credentials.json",
    "secrets.json",
    "secrets.yaml",
    "secrets.yml",
    "shadow",
    "gshadow",
    "master.passwd",
];

const SENSITIVE_EXTENSIONS: &[&str] = &[
    "pem",
    "key",
    "p12",
    "pfx",
    "jks",
    "keystore",
    "kdbx",
    "ppk",
    "ovpn",
];

fn canonicalize_best_effort(path: &Path) -> Option<PathBuf> {
    if let Ok(c) = fs::canonicalize(path) {
        return Some(c);
    }
    let mut missing_tail = Vec::new();
    let mut cur = path.to_path_buf();
    while !cur.exists() {
        let name = cur.file_name()?.to_os_string();
        missing_tail.push(name);
        if !cur.pop() {
            return None;
        }
    }
    let mut base = fs::canonicalize(&cur).ok()?;
    for seg in missing_tail.into_iter().rev() {
        base.push(seg);
    }
    Some(base)
}

fn check_single_path_sensitive(path: &Path) -> bool {
    let norm = path.to_string_lossy().replace('\\', "/").to_ascii_lowercase();
    let trimmed = norm.trim_end_matches('/');
    const SYSTEM_SENSITIVE_PREFIXES: &[&str] = &[
        "/etc/shadow",
        "/etc/gshadow",
        "/etc/sudoers",
        "/etc/sudoers.d",
        "/etc/master.passwd",
        "/private/etc/shadow",
        "/private/etc/gshadow",
        "/private/etc/sudoers",
        "/private/etc/sudoers.d",
        "/private/etc/master.passwd",
    ];
    if SYSTEM_SENSITIVE_PREFIXES
        .iter()
        .any(|p| trimmed == *p || trimmed.starts_with(&format!("{p}/")))
    {
        return true;
    }

    let components: Vec<String> = path
        .components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_string_lossy().to_ascii_lowercase()),
            _ => None,
        })
        .collect();

    for (idx, seg) in components.iter().enumerate() {
        if SENSITIVE_DIR_NAMES.contains(&seg.as_str()) {
            return true;
        }
        if seg == ".nola" {
            if let Some(next) = components.get(idx + 1) {
                if next.starts_with("vault")
                    || next == "master.key"
                    || next == "secrets"
                    || next == "credentials"
                {
                    return true;
                }
            }
        }
        if seg == ".config" {
            if let Some(next) = components.get(idx + 1) {
                if matches!(next.as_str(), "gcloud" | "gh" | "aws" | "kube") {
                    return true;
                }
            }
        }
        if seg == "library" {
            if let Some(next) = components.get(idx + 1) {
                if next == "keychains" {
                    return true;
                }
            }
        }
    }

    if let Some(file_name) = components.last() {
        if SENSITIVE_FILE_NAMES.contains(&file_name.as_str())
            || file_name.starts_with(".env.")
            || file_name.starts_with("vault.")
            || file_name.starts_with("id_rsa.")
            || file_name.starts_with("id_ed25519.")
        {
            return true;
        }
        if let Some((_, ext)) = file_name.rsplit_once('.') {
            if SENSITIVE_EXTENSIONS.contains(&ext) {
                return true;
            }
        }
    }

    false
}

/// Returns `true` if `path` (or its canonicalized target, preventing symlink escapes)
/// points to a sensitive local credential/secret path that must never be indexed or exposed.
pub fn is_sensitive_local_path(path: &Path) -> bool {
    if check_single_path_sensitive(path) {
        return true;
    }
    if let Some(canon) = canonicalize_best_effort(path) {
        if check_single_path_sensitive(&canon) {
            return true;
        }
    }
    false
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataSourceUsageRecord {
    pub id: String,
    pub conversation_id: String,
    pub source_id: String,
    pub summary: String,
    pub used_at_ms: i64,
}

/// Perform real bounded read-only connection validation for a configured DataSource record.
/// NEVER fabricates `"verified"` — returns `"failed"` with a concrete error on any failure.
pub fn test_data_source_connection(record: &Value) -> Value {
    let start = Instant::now();
    let kind_raw = record
        .get("kind")
        .or_else(|| record.get("type"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    let kind = kind_raw.to_ascii_lowercase();
    let location = record
        .get("location")
        .or_else(|| record.get("path"))
        .or_else(|| record.get("url"))
        .or_else(|| record.get("endpoint"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();

    if location.is_empty() {
        return json!({
            "status": "failed",
            "ok": false,
            "kind": kind_raw,
            "location": location,
            "latencyMs": start.elapsed().as_millis() as u64,
            "error": "missing data source location/path/url"
        });
    }

    match kind.as_str() {
        "localfile" | "local_file" | "file" => {
            let path = Path::new(location);
            if is_sensitive_local_path(path) {
                return json!({
                    "status": "failed",
                    "ok": false,
                    "kind": kind_raw,
                    "location": location,
                    "latencyMs": start.elapsed().as_millis() as u64,
                    "error": format!("refused to access sensitive local path: {location}")
                });
            }
            if !path.exists() {
                return json!({
                    "status": "failed",
                    "ok": false,
                    "kind": kind_raw,
                    "location": location,
                    "latencyMs": start.elapsed().as_millis() as u64,
                    "error": format!("file does not exist: {location}")
                });
            }
            if !path.is_file() {
                return json!({
                    "status": "failed",
                    "ok": false,
                    "kind": kind_raw,
                    "location": location,
                    "latencyMs": start.elapsed().as_millis() as u64,
                    "error": format!("path is not a regular file: {location}")
                });
            }
            let size_bytes = fs::metadata(path).map(|m| m.len()).unwrap_or(0);
            match File::open(path) {
                Ok(mut f) => {
                    let mut buf = vec![0u8; MAX_LOCAL_PREVIEW_BYTES];
                    match f.read(&mut buf) {
                        Ok(n) => {
                            buf.truncate(n);
                            let preview: String = String::from_utf8_lossy(&buf)
                                .chars()
                                .take(512)
                                .collect();
                            json!({
                                "status": "verified",
                                "ok": true,
                                "kind": kind_raw,
                                "location": location,
                                "sizeBytes": size_bytes,
                                "preview": preview,
                                "latencyMs": start.elapsed().as_millis() as u64
                            })
                        }
                        Err(e) => json!({
                            "status": "failed",
                            "ok": false,
                            "kind": kind_raw,
                            "location": location,
                            "latencyMs": start.elapsed().as_millis() as u64,
                            "error": format!("file unreadable: {e}")
                        }),
                    }
                }
                Err(e) => json!({
                    "status": "failed",
                    "ok": false,
                    "kind": kind_raw,
                    "location": location,
                    "latencyMs": start.elapsed().as_millis() as u64,
                    "error": format!("failed to open file: {e}")
                }),
            }
        }
        "localfolder" | "local_folder" | "folder" | "dir" | "directory" => {
            let path = Path::new(location);
            if is_sensitive_local_path(path) {
                return json!({
                    "status": "failed",
                    "ok": false,
                    "kind": kind_raw,
                    "location": location,
                    "latencyMs": start.elapsed().as_millis() as u64,
                    "error": format!("refused to access sensitive local path: {location}")
                });
            }
            if !path.exists() {
                return json!({
                    "status": "failed",
                    "ok": false,
                    "kind": kind_raw,
                    "location": location,
                    "latencyMs": start.elapsed().as_millis() as u64,
                    "error": format!("directory does not exist: {location}")
                });
            }
            if !path.is_dir() {
                return json!({
                    "status": "failed",
                    "ok": false,
                    "kind": kind_raw,
                    "location": location,
                    "latencyMs": start.elapsed().as_millis() as u64,
                    "error": format!("path is not a directory: {location}")
                });
            }
            match fs::read_dir(path) {
                Ok(entries) => {
                    let mut count = 0usize;
                    let mut sample = Vec::new();
                    for entry in entries.flatten().take(MAX_DIR_ENTRIES_SCAN) {
                        count += 1;
                        if sample.len() < 10 {
                            sample.push(entry.file_name().to_string_lossy().into_owned());
                        }
                    }
                    json!({
                        "status": "verified",
                        "ok": true,
                        "kind": kind_raw,
                        "location": location,
                        "entryCount": count,
                        "sampleEntries": sample,
                        "latencyMs": start.elapsed().as_millis() as u64
                    })
                }
                Err(e) => json!({
                    "status": "failed",
                    "ok": false,
                    "kind": kind_raw,
                    "location": location,
                    "latencyMs": start.elapsed().as_millis() as u64,
                    "error": format!("directory unreadable: {e}")
                }),
            }
        }
        "http" | "https" | "url" | "api" | "rest" | "feed" => {
            let (parsed, host, addrs) = match resolve_and_validate_public_url(location) {
                Ok(v) => v,
                Err(e) => {
                    return json!({
                        "status": "failed",
                        "ok": false,
                        "kind": kind_raw,
                        "location": location,
                        "latencyMs": start.elapsed().as_millis() as u64,
                        "error": e
                    });
                }
            };
            let client = match reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(10))
                .redirect(reqwest::redirect::Policy::none())
                .resolve_to_addrs(&host, &addrs)
                .user_agent("Nola/2.0")
                .build()
            {
                Ok(c) => c,
                Err(e) => {
                    return json!({
                        "status": "failed",
                        "ok": false,
                        "kind": kind_raw,
                        "location": location,
                        "latencyMs": start.elapsed().as_millis() as u64,
                        "error": e.to_string()
                    });
                }
            };

            let resp = match client.head(parsed.clone()).send() {
                Ok(r) if r.status().as_u16() == 405 || r.status().as_u16() == 501 => {
                    client.get(parsed).send()
                }
                other => other,
            };

            match resp {
                Ok(r) => {
                    let status_code = r.status().as_u16();
                    let ok = r.status().is_success();
                    let content_type = r
                        .headers()
                        .get(reqwest::header::CONTENT_TYPE)
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("")
                        .to_string();
                    let mut body = Vec::new();
                    let _ = r.take(MAX_HTTP_PROBE_BYTES).read_to_end(&mut body);
                    json!({
                        "status": if ok { "verified" } else { "failed" },
                        "ok": ok,
                        "kind": kind_raw,
                        "location": location,
                        "httpStatus": status_code,
                        "contentType": content_type,
                        "bytesRead": body.len(),
                        "latencyMs": start.elapsed().as_millis() as u64,
                        "error": if ok { Value::Null } else { json!(format!("HTTP {status_code}")) }
                    })
                }
                Err(e) => json!({
                    "status": "failed",
                    "ok": false,
                    "kind": kind_raw,
                    "location": location,
                    "latencyMs": start.elapsed().as_millis() as u64,
                    "error": e.to_string()
                }),
            }
        }
        _ => json!({
            "status": "failed",
            "ok": false,
            "kind": kind_raw,
            "location": location,
            "latencyMs": start.elapsed().as_millis() as u64,
            "error": format!("unsupported data source kind '{kind_raw}' for connection probe")
        }),
    }
}

/// Record a provenance entry when a conversation uses a DataSource.
pub fn record_data_source_usage(
    store: &Store,
    conv_id: &str,
    source_id: &str,
    summary: &str,
) -> Result<Value, String> {
    let rec = DataSourceUsageRecord {
        id: new_id(),
        conversation_id: conv_id.to_string(),
        source_id: source_id.to_string(),
        summary: summary.chars().take(512).collect(),
        used_at_ms: now_ms(),
    };
    store
        .put_module_record(KIND_DATA_SOURCE_USAGE, &rec.id, &rec)
        .map_err(|e| e.to_string())?;

    if let Ok(Some(mut ds)) = store.get_module_record::<DataSourceRecord>(KIND_DATA_SOURCE, source_id)
    {
        ds.usage_count = ds.usage_count.saturating_add(1);
        ds.updated_at_ms = now_ms();
        let _ = store.put_module_record(KIND_DATA_SOURCE, &ds.id, &ds);
    }

    serde_json::to_value(&rec).map_err(|e| e.to_string())
}

/// List recorded DataSource usage provenance entries sorted newest-first.
pub fn list_data_source_usage(store: &Store) -> Vec<Value> {
    let mut items: Vec<DataSourceUsageRecord> = store
        .list_module_records(KIND_DATA_SOURCE_USAGE)
        .unwrap_or_default();
    items.sort_by(|a, b| b.used_at_ms.cmp(&a.used_at_ms));
    items
        .into_iter()
        .filter_map(|item| serde_json::to_value(item).ok())
        .collect()
}

/// Split extracted text into overlapping UTF-8-safe chunks (~1,200 chars, ~200-char overlap).
pub fn split_text_into_chunks(
    text: &str,
    chunk_chars: usize,
    overlap_chars: usize,
) -> Vec<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    let size = chunk_chars.max(100);
    let overlap = overlap_chars.min(size / 2);
    let chars: Vec<char> = trimmed.chars().collect();
    if chars.len() <= size {
        return vec![trimmed.to_string()];
    }
    let mut chunks = Vec::new();
    let mut start = 0usize;
    while start < chars.len() {
        let mut end = (start + size).min(chars.len());
        if end < chars.len() {
            let search_floor = start + (size * 2 / 3);
            if let Some(rel_nl) = chars[search_floor..end].iter().rposition(|&c| c == '\n') {
                end = search_floor + rel_nl + 1;
            } else if let Some(rel_sp) = chars[search_floor..end]
                .iter()
                .rposition(|&c| c.is_whitespace())
            {
                end = search_floor + rel_sp + 1;
            }
        }
        let slice: String = chars[start..end].iter().collect();
        let cleaned = slice.trim();
        if !cleaned.is_empty() {
            chunks.push(cleaned.to_string());
        }
        if end >= chars.len() {
            break;
        }
        let next_start = end.saturating_sub(overlap);
        start = if next_start <= start { end } else { next_start };
    }
    chunks
}

fn should_skip_index_path(rel_path: &Path) -> bool {
    rel_path.components().any(|c| {
        matches!(c, Component::Normal(name) if name
            .to_str()
            .is_some_and(|s| SKIP_INDEX_DIRS.contains(&s)))
    })
}

fn collect_source_files(kind: &str, location: &Path) -> Result<Vec<(String, PathBuf)>, String> {
    match kind {
        "localfile" | "local_file" | "file" => {
            if is_sensitive_local_path(location) {
                return Err(format!(
                    "refused to index sensitive local path: {}",
                    location.display()
                ));
            }
            if !location.is_file() {
                return Err(format!(
                    "data source file does not exist or is not a file: {}",
                    location.display()
                ));
            }
            let name = location
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("document")
                .to_string();
            Ok(vec![(name, location.to_path_buf())])
        }
        "localfolder" | "local_folder" | "folder" | "dir" | "directory" => {
            if is_sensitive_local_path(location) {
                return Err(format!(
                    "refused to index sensitive local path: {}",
                    location.display()
                ));
            }
            if !location.is_dir() {
                return Err(format!(
                    "data source folder does not exist or is not a directory: {}",
                    location.display()
                ));
            }
            let mut out = Vec::new();
            let mut builder = WalkBuilder::new(location);
            builder
                .hidden(false)
                .git_ignore(true)
                .git_global(false)
                .git_exclude(true)
                .follow_links(false);
            for entry in builder.build().flatten() {
                if out.len() >= MAX_INDEX_FILES {
                    break;
                }
                let path = entry.path();
                if entry
                    .file_type()
                    .is_some_and(|ft| ft.is_symlink())
                {
                    continue;
                }
                if !path.is_file() {
                    continue;
                }
                if is_sensitive_local_path(path) {
                    continue;
                }
                let rel = path.strip_prefix(location).unwrap_or(path);
                if should_skip_index_path(rel) {
                    continue;
                }
                let rel_str = rel.to_string_lossy().replace('\\', "/");
                out.push((rel_str, path.to_path_buf()));
            }
            out.sort_by(|a, b| a.0.cmp(&b.0));
            Ok(out)
        }
        other => Err(format!(
            "data source kind '{other}' does not support local RAG indexing"
        )),
    }
}

/// Index a configured `localFile` or `localFolder` DataSource into SQLite FTS5 `doc_chunks`.
/// Extracts UTF-8, PDF, DOCX, and XLSX via `nola_tools::extract_document_text_from_bytes`,
/// splits into overlapping ~1,200-char chunks, and replaces chunks atomically in `Store`.
pub fn index_data_source(store: &Store, source_id: &str) -> Result<usize, String> {
    let mut ds = store
        .get_module_record::<DataSourceRecord>(KIND_DATA_SOURCE, source_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("data source '{source_id}' not found"))?;
    let kind_norm = ds.kind.trim().to_ascii_lowercase();
    let location = Path::new(ds.location.trim());
    let files = collect_source_files(&kind_norm, location)?;

    let ts = now_ms();
    let mut chunks = Vec::new();
    let mut indexed_files = 0usize;

    for (rel_path, abs_path) in files {
        if is_sensitive_local_path(&abs_path) {
            continue;
        }
        let Ok(meta) = fs::symlink_metadata(&abs_path) else {
            continue;
        };
        if meta.file_type().is_symlink() {
            continue;
        }
        if !meta.is_file() || meta.len() == 0 || meta.len() > MAX_INDEX_FILE_BYTES {
            continue;
        }
        let Ok(bytes) = fs::read(&abs_path) else {
            continue;
        };
        let file_name = abs_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(&rel_path);
        let Some(text) = extract_document_text_from_bytes(file_name, "", &bytes) else {
            continue;
        };
        let parts = split_text_into_chunks(&text, DEFAULT_CHUNK_CHARS, DEFAULT_CHUNK_OVERLAP_CHARS);
        if parts.is_empty() {
            continue;
        }
        indexed_files += 1;
        for (idx, content) in parts.into_iter().enumerate() {
            let chunk_sha = sha256_hex(content.as_bytes());
            chunks.push(DocChunkRecord {
                id: format!("{source_id}:{rel_path}:{idx}"),
                source_id: source_id.to_string(),
                path: rel_path.clone(),
                chunk_index: idx as i64,
                content,
                sha256: chunk_sha,
                updated_at_ms: ts,
            });
        }
    }

    let chunk_count = store
        .replace_source_chunks(source_id, &chunks)
        .map_err(|e| e.to_string())?;

    if !ds.metadata.is_object() {
        ds.metadata = json!({});
    }
    if let Some(obj) = ds.metadata.as_object_mut() {
        obj.insert("indexed_files".into(), json!(indexed_files));
        obj.insert("indexed_chunks".into(), json!(chunk_count));
        obj.insert("indexed_at_ms".into(), json!(ts));
    }
    ds.updated_at_ms = ts;
    let _ = store.put_module_record(KIND_DATA_SOURCE, &ds.id, &ds);
    Ok(chunk_count)
}

pub fn register_tools(suite: &Arc<ModulesSuite>, registry: &mut ToolRegistry) {
    registry.register(Arc::new(DataSourceManageTool {
        suite: Arc::clone(suite),
    }));
    registry.register(Arc::new(DocumentSearchTool {
        suite: Arc::clone(suite),
    }));
}

/// Full-text RAG search over indexed DataSource document chunks (`doc_chunks_fts`).
pub struct DocumentSearchTool {
    suite: Arc<ModulesSuite>,
}

impl DocumentSearchTool {
    pub fn new(suite: Arc<ModulesSuite>) -> Self {
        Self { suite }
    }
}

impl ToolHandler for DocumentSearchTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "document_search".into(),
                description: "Search indexed DataSource document chunks (UTF-8, PDF, DOCX, XLSX) using SQLite FTS5 BM25 ranking.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "Search query" },
                        "source_ids": { "type": "array", "items": { "type": "string" }, "description": "Optional list of DataSource IDs to filter" },
                        "source_id": { "type": "string", "description": "Optional single DataSource ID shorthand" },
                        "limit": { "type": "integer", "description": "Maximum ranked chunks to return (default 10, max 50)" }
                    },
                    "required": ["query"]
                }),
            },
            tier: ToolTier::Deferred,
            domain: CapabilityDomain::System,
            read_only: true,
        }
    }

    fn execute(&self, _ctx: &ToolContext, args: Value) -> ToolExecutionResult {
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        if query.is_empty() {
            return ToolExecutionResult::err_structured(
                "missing_param",
                "missing non-empty 'query'",
                None,
            );
        }
        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(10)
            .clamp(1, 50) as usize;
        let mut source_ids: Vec<String> = args
            .get("source_ids")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::trim))
                    .filter(|s| !s.is_empty())
                    .map(ToString::to_string)
                    .collect()
            })
            .unwrap_or_default();
        if let Some(single) = args
            .get("source_id")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            if !source_ids.iter().any(|s| s == single) {
                source_ids.push(single.to_string());
            }
        }
        let filter_slice = (!source_ids.is_empty()).then_some(source_ids.as_slice());
        match self.suite.store.search_doc_chunks(filter_slice, query, limit) {
            Ok(hits) => {
                let count = hits.len();
                let results: Vec<Value> = hits
                    .into_iter()
                    .map(|h| {
                        let snippet: String = h.content.chars().take(400).collect();
                        json!({
                            "id": h.id,
                            "source_id": h.source_id,
                            "path": h.path,
                            "chunk_index": h.chunk_index,
                            "score": h.score,
                            "snippet": snippet,
                            "content": h.content,
                            "sha256": h.sha256,
                            "updated_at_ms": h.updated_at_ms
                        })
                    })
                    .collect();
                ToolExecutionResult::ok_structured(
                    json!({
                        "query": query,
                        "count": count,
                        "hits": results
                    }),
                    format!("Found {count} matching document chunk(s) for '{query}'"),
                )
            }
            Err(e) => ToolExecutionResult::err_structured("ds_search_error", e.to_string(), None),
        }
    }
}

struct DataSourceManageTool {
    suite: Arc<ModulesSuite>,
}

impl ToolHandler for DataSourceManageTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "data_source_manage".into(),
                description: "Register, list, test, index for RAG, search chunks, record usage, or delete configured DataSources.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "action": { "type": "string", "enum": ["create", "list", "touch", "test", "index", "search", "usage", "delete"] },
                        "id": { "type": "string" },
                        "name": { "type": "string" },
                        "kind": { "type": "string" },
                        "location": { "type": "string" },
                        "query": { "type": "string" },
                        "limit": { "type": "integer" },
                        "source_ids": { "type": "array", "items": { "type": "string" } },
                        "conversation_id": { "type": "string" },
                        "summary": { "type": "string" },
                        "metadata": { "type": "object" }
                    },
                    "required": ["action"]
                }),
            },
            tier: ToolTier::Deferred,
            domain: CapabilityDomain::System,
            read_only: false,
        }
    }

    fn execute(&self, ctx: &ToolContext, args: Value) -> ToolExecutionResult {
        match args.get("action").and_then(|v| v.as_str()).unwrap_or("") {
            "create" => {
                let rec = DataSourceRecord {
                    id: args
                        .get("id")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string)
                        .unwrap_or_else(new_id),
                    name: args
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("DataSource")
                        .to_string(),
                    kind: args
                        .get("kind")
                        .and_then(|v| v.as_str())
                        .unwrap_or("directory")
                        .to_string(),
                    location: args
                        .get("location")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    metadata: args.get("metadata").cloned().unwrap_or_else(|| json!({})),
                    usage_count: 0,
                    updated_at_ms: now_ms(),
                };
                match self.suite.upsert_record(KIND_DATA_SOURCE, &rec.id, &rec) {
                    Ok(()) => ToolExecutionResult::ok_structured(
                        serde_json::to_value(&rec).unwrap_or(json!({})),
                        format!("Registered data source '{}'", rec.name),
                    ),
                    Err(e) => ToolExecutionResult::err_structured("ds_error", e.to_string(), None),
                }
            }
            "list" => match self.suite.list_records::<DataSourceRecord>(KIND_DATA_SOURCE) {
                Ok(items) => {
                    let count = items.len();
                    ToolExecutionResult::ok_structured(
                        json!({"count": count, "data_sources": items}),
                        format!("Listed {count} data source(s)"),
                    )
                }
                Err(e) => ToolExecutionResult::err_structured("ds_error", e.to_string(), None),
            },
            "index" => {
                let Some(id) = args.get("id").and_then(|v| v.as_str()) else {
                    return ToolExecutionResult::err_structured("missing_param", "missing 'id'", None);
                };
                match index_data_source(&self.suite.store, id) {
                    Ok(chunk_count) => ToolExecutionResult::ok_structured(
                        json!({"id": id, "indexed": true, "chunk_count": chunk_count}),
                        format!("Indexed data source '{id}' ({chunk_count} chunk(s))"),
                    ),
                    Err(e) => ToolExecutionResult::err_structured("ds_index_error", e, None),
                }
            }
            "search" => {
                let mut search_args = args.clone();
                if let Some(id) = args.get("id").and_then(|v| v.as_str()) {
                    if let Some(obj) = search_args.as_object_mut() {
                        obj.entry("source_id").or_insert_with(|| json!(id));
                    }
                }
                DocumentSearchTool::new(Arc::clone(&self.suite)).execute(ctx, search_args)
            }
            "test" => {
                let record = if let Some(id) = args.get("id").and_then(|v| v.as_str()) {
                    match self.suite.get_record::<Value>(KIND_DATA_SOURCE, id) {
                        Ok(Some(v)) => v,
                        _ => {
                            return ToolExecutionResult::err_structured(
                                "not_found",
                                format!("data source '{id}' not found"),
                                None,
                            );
                        }
                    }
                } else {
                    args.clone()
                };
                let probe = test_data_source_connection(&record);
                let ok = probe.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
                if ok {
                    ToolExecutionResult::ok_structured(probe, "Data source connection verified")
                } else {
                    let msg = probe
                        .get("error")
                        .and_then(|v| v.as_str())
                        .unwrap_or("data source probe failed")
                        .to_string();
                    ToolExecutionResult::err_structured("ds_probe_failed", msg, Some(probe))
                }
            }
            "touch" => {
                let Some(id) = args.get("id").and_then(|v| v.as_str()) else {
                    return ToolExecutionResult::err_structured("missing_param", "missing 'id'", None);
                };
                let Ok(Some(mut rec)) = self.suite.get_record::<DataSourceRecord>(KIND_DATA_SOURCE, id) else {
                    return ToolExecutionResult::err_structured("not_found", format!("data source '{id}' not found"), None);
                };
                rec.usage_count = rec.usage_count.saturating_add(1);
                rec.updated_at_ms = now_ms();
                let _ = self.suite.upsert_record(KIND_DATA_SOURCE, &rec.id, &rec);
                if let Some(conv_id) = args.get("conversation_id").and_then(|v| v.as_str()) {
                    let summary = args
                        .get("summary")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Accessed data source");
                    let _ = record_data_source_usage(&self.suite.store, conv_id, &rec.id, summary);
                }
                ToolExecutionResult::ok_structured(
                    serde_json::to_value(&rec).unwrap_or(json!({})),
                    format!("Incremented usage count for '{}'", rec.name),
                )
            }
            "usage" => {
                let usage = list_data_source_usage(&self.suite.store);
                let count = usage.len();
                ToolExecutionResult::ok_structured(
                    json!({"count": count, "usage": usage}),
                    format!("Listed {count} data source usage record(s)"),
                )
            }
            "delete" => {
                let Some(id) = args.get("id").and_then(|v| v.as_str()) else {
                    return ToolExecutionResult::err_structured("missing_param", "missing 'id'", None);
                };
                let _ = self.suite.store.delete_source_chunks(id);
                match self.suite.delete_record(KIND_DATA_SOURCE, id) {
                    Ok(deleted) => ToolExecutionResult::ok_structured(
                        json!({"id": id, "deleted": deleted}),
                        format!("Deleted data source '{id}'"),
                    ),
                    Err(e) => ToolExecutionResult::err_structured("ds_error", e.to_string(), None),
                }
            }
            other => ToolExecutionResult::err_structured(
                "invalid_action",
                format!("unsupported data_source action '{other}'"),
                None,
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_source_probe_validates_files_folders_and_tracks_usage() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("notes.txt");
        fs::write(&file, "hello nola data source").unwrap();

        let ok_file = test_data_source_connection(&json!({
            "kind": "localFile",
            "location": file.display().to_string()
        }));
        assert_eq!(ok_file["status"], "verified");
        assert_eq!(ok_file["ok"], true);
        assert!(ok_file["preview"].as_str().unwrap().contains("hello nola"));

        let missing_file = test_data_source_connection(&json!({
            "kind": "localFile",
            "location": dir.path().join("missing.txt").display().to_string()
        }));
        assert_eq!(missing_file["status"], "failed");
        assert_eq!(missing_file["ok"], false);

        let ok_dir = test_data_source_connection(&json!({
            "kind": "localFolder",
            "location": dir.path().display().to_string()
        }));
        assert_eq!(ok_dir["status"], "verified");
        assert_eq!(ok_dir["entryCount"], 1);

        let blocked_http = test_data_source_connection(&json!({
            "kind": "http",
            "location": "http://127.0.0.1:9/private"
        }));
        assert_eq!(blocked_http["status"], "failed");

        let store = Store::open_in_memory().unwrap();
        let entry = record_data_source_usage(&store, "conv-42", "ds-1", "Queried notes.txt").unwrap();
        assert_eq!(entry["conversation_id"], "conv-42");
        let list = list_data_source_usage(&store);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0]["source_id"], "ds-1");
    }

    #[test]
    fn data_source_rag_indexing_chunking_and_document_search_work() {
        let dir = tempfile::tempdir().unwrap();
        let docs = dir.path().join("docs");
        fs::create_dir_all(docs.join("nested")).unwrap();
        fs::create_dir_all(docs.join("node_modules/pkg")).unwrap();
        fs::create_dir_all(docs.join(".git")).unwrap();

        let long_doc = format!(
            "{} QuantumFluxCalibrationProtocol {}",
            "alpha ".repeat(260),
            "omega ".repeat(260)
        );
        fs::write(docs.join("guide.md"), &long_doc).unwrap();
        fs::write(
            docs.join("nested/runbook.txt"),
            "Runbook: rotate TLS certificates every 90 days.",
        )
        .unwrap();
        fs::write(
            docs.join("node_modules/pkg/ignored.txt"),
            "ShouldNeverBeIndexedSecretToken",
        )
        .unwrap();
        fs::write(
            docs.join(".git/ignored.txt"),
            "ShouldNeverBeIndexedGitToken",
        )
        .unwrap();

        let store = Store::open_in_memory().unwrap();
        let suite = Arc::new(ModulesSuite::new(store.clone(), dir.path().to_path_buf(), None));
        let mut reg = ToolRegistry::default();
        register_tools(&suite, &mut reg);
        let ctx = ToolContext::new(dir.path());

        let created = reg.execute(
            "data_source_manage",
            &ctx,
            json!({
                "action": "create",
                "id": "ds-docs",
                "name": "Engineering Docs",
                "kind": "localFolder",
                "location": docs.display().to_string()
            }),
        );
        assert!(!created.is_error);

        let indexed = reg.execute(
            "data_source_manage",
            &ctx,
            json!({"action": "index", "id": "ds-docs"}),
        );
        assert!(!indexed.is_error);
        let chunk_count = indexed.payload.to_model_value()["chunk_count"]
            .as_u64()
            .unwrap();
        assert!(chunk_count >= 3, "expected overlapping chunks, got {chunk_count}");

        // Search via document_search tool
        let hit = reg.execute(
            "document_search",
            &ctx,
            json!({"query": "QuantumFluxCalibrationProtocol", "source_ids": ["ds-docs"], "limit": 5}),
        );
        assert!(!hit.is_error);
        let hits = hit.payload.to_model_value()["hits"].as_array().unwrap().clone();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0]["source_id"], "ds-docs");
        assert_eq!(hits[0]["path"], "guide.md");

        // Search via data_source_manage(action: "search")
        let hit2 = reg.execute(
            "data_source_manage",
            &ctx,
            json!({"action": "search", "id": "ds-docs", "query": "TLS certificates"}),
        );
        assert!(!hit2.is_error);
        assert_eq!(hit2.payload.to_model_value()["count"], 1);

        // Ignored dirs were never indexed
        let ignored = reg.execute(
            "document_search",
            &ctx,
            json!({"query": "ShouldNeverBeIndexedSecretToken"}),
        );
        assert_eq!(ignored.payload.to_model_value()["count"], 0);
    }

    #[test]
    fn sensitive_local_paths_and_symlinks_are_rejected_from_indexing() {
        assert!(is_sensitive_local_path(Path::new("/home/alice/.ssh/id_rsa")));
        assert!(is_sensitive_local_path(Path::new("/home/alice/.aws/credentials")));
        assert!(is_sensitive_local_path(Path::new("/home/alice/.gnupg/secring.gpg")));
        assert!(is_sensitive_local_path(Path::new("/home/alice/.kube/config")));
        assert!(is_sensitive_local_path(Path::new("/home/alice/.nola/vault.json")));
        assert!(is_sensitive_local_path(Path::new("/project/.env")));
        assert!(is_sensitive_local_path(Path::new("/project/.env.production")));
        assert!(is_sensitive_local_path(Path::new("/project/id_ed25519")));
        assert!(is_sensitive_local_path(Path::new("/project/tls.pem")));
        assert!(is_sensitive_local_path(Path::new("/project/private.key")));
        assert!(is_sensitive_local_path(Path::new("/etc/shadow")));
        assert!(!is_sensitive_local_path(Path::new("/project/docs/guide.md")));

        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_in_memory().unwrap();

        // 1. Direct sensitive folder root (.ssh) is rejected
        let ssh_dir = dir.path().join(".ssh");
        fs::create_dir_all(&ssh_dir).unwrap();
        fs::write(ssh_dir.join("id_rsa"), "PRIVATE_SSH_KEY_SECRET").unwrap();
        store
            .put_module_record(
                KIND_DATA_SOURCE,
                "ds-ssh",
                &DataSourceRecord {
                    id: "ds-ssh".into(),
                    name: "SSH".into(),
                    kind: "localFolder".into(),
                    location: ssh_dir.display().to_string(),
                    metadata: json!({}),
                    usage_count: 0,
                    updated_at_ms: now_ms(),
                },
            )
            .unwrap();
        let err = index_data_source(&store, "ds-ssh").unwrap_err();
        assert!(err.contains("sensitive"), "expected sensitive error, got: {err}");

        // 2. Direct sensitive file (.env) is rejected
        let env_file = dir.path().join(".env");
        fs::write(&env_file, "API_SECRET=SuperSecretEnvToken123").unwrap();
        store
            .put_module_record(
                KIND_DATA_SOURCE,
                "ds-env",
                &DataSourceRecord {
                    id: "ds-env".into(),
                    name: "Env".into(),
                    kind: "localFile".into(),
                    location: env_file.display().to_string(),
                    metadata: json!({}),
                    usage_count: 0,
                    updated_at_ms: now_ms(),
                },
            )
            .unwrap();
        assert!(index_data_source(&store, "ds-env").is_err());

        // 3. Symlink to sensitive file cannot bypass guard (canonicalize check)
        #[cfg(unix)]
        {
            let innocent_link = dir.path().join("harmless.txt");
            std::os::unix::fs::symlink(&env_file, &innocent_link).unwrap();
            assert!(is_sensitive_local_path(&innocent_link));
            store
                .put_module_record(
                    KIND_DATA_SOURCE,
                    "ds-sym",
                    &DataSourceRecord {
                        id: "ds-sym".into(),
                        name: "Sym".into(),
                        kind: "localFile".into(),
                        location: innocent_link.display().to_string(),
                        metadata: json!({}),
                        usage_count: 0,
                        updated_at_ms: now_ms(),
                    },
                )
                .unwrap();
            assert!(index_data_source(&store, "ds-sym").is_err());
        }

        // 4. Folder walk skips sensitive files while indexing safe docs
        let mixed = dir.path().join("mixed");
        fs::create_dir_all(mixed.join(".nola")).unwrap();
        fs::write(mixed.join("readme.md"), "PublicEngineeringHandbookMarker").unwrap();
        fs::write(mixed.join(".env"), "SecretDotEnvMarker").unwrap();
        fs::write(mixed.join("server.pem"), "SecretPemCertMarker").unwrap();
        fs::write(mixed.join("priv.key"), "SecretKeyMarker").unwrap();
        fs::write(mixed.join("id_ed25519"), "SecretEd25519Marker").unwrap();
        fs::write(mixed.join(".nola/vault.json"), "SecretVaultJsonMarker").unwrap();
        store
            .put_module_record(
                KIND_DATA_SOURCE,
                "ds-mixed",
                &DataSourceRecord {
                    id: "ds-mixed".into(),
                    name: "Mixed".into(),
                    kind: "localFolder".into(),
                    location: mixed.display().to_string(),
                    metadata: json!({}),
                    usage_count: 0,
                    updated_at_ms: now_ms(),
                },
            )
            .unwrap();
        let indexed_chunks = index_data_source(&store, "ds-mixed").unwrap();
        assert_eq!(indexed_chunks, 1);
        assert_eq!(
            store
                .search_doc_chunks(Some(&["ds-mixed".to_string()]), "PublicEngineeringHandbookMarker", 5)
                .unwrap()
                .len(),
            1
        );
        for leaked in [
            "SecretDotEnvMarker",
            "SecretPemCertMarker",
            "SecretKeyMarker",
            "SecretEd25519Marker",
            "SecretVaultJsonMarker",
        ] {
            assert_eq!(
                store
                    .search_doc_chunks(Some(&["ds-mixed".to_string()]), leaked, 5)
                    .unwrap()
                    .len(),
                0,
                "sensitive content '{leaked}' must never be indexed"
            );
        }
    }
}
