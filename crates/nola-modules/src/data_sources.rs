// SPDX-License-Identifier: Apache-2.0
//! DataSources registry, real bounded connection probing, and usage provenance tracking.

use std::fs::{self, File};
use std::io::Read;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use nola_store::Store;
use nola_tools::web::resolve_and_validate_public_url;
use nola_tools::{ToolContext, ToolHandler, ToolRegistry};
use nola_types::{
    new_id, now_ms, CapabilityDomain, DataSourceRecord, ToolExecutionResult, ToolSchema, ToolSpec,
    ToolTier,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::ModulesSuite;

pub const KIND_DATA_SOURCE: &str = "data_source";
pub const KIND_DATA_SOURCE_USAGE: &str = "data_source_usage";

const MAX_HTTP_PROBE_BYTES: u64 = 64 * 1024;
const MAX_LOCAL_PREVIEW_BYTES: usize = 4 * 1024;
const MAX_DIR_ENTRIES_SCAN: usize = 200;

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

pub fn register_tools(suite: &Arc<ModulesSuite>, registry: &mut ToolRegistry) {
    registry.register(Arc::new(DataSourceManageTool {
        suite: Arc::clone(suite),
    }));
}

struct DataSourceManageTool {
    suite: Arc<ModulesSuite>,
}

impl ToolHandler for DataSourceManageTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "data_source_manage".into(),
                description: "Register, list, test, record usage, or delete configured DataSources.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "action": { "type": "string", "enum": ["create", "list", "touch", "test", "usage", "delete"] },
                        "id": { "type": "string" },
                        "name": { "type": "string" },
                        "kind": { "type": "string" },
                        "location": { "type": "string" },
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

    fn execute(&self, _ctx: &ToolContext, args: Value) -> ToolExecutionResult {
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
}
