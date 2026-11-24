// SPDX-License-Identifier: Apache-2.0
//! DataSources registry & usage tracking module.

use std::sync::Arc;

use nola_tools::{ToolContext, ToolHandler, ToolRegistry};
use nola_types::{
    new_id, now_ms, CapabilityDomain, DataSourceRecord, ToolExecutionResult, ToolSchema, ToolSpec,
    ToolTier,
};
use serde_json::{json, Value};

use crate::ModulesSuite;

pub const KIND_DATA_SOURCE: &str = "data_source";

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
                description: "Register, list, record usage, or delete configured DataSources.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "action": { "type": "string", "enum": ["create", "list", "touch", "delete"] },
                        "id": { "type": "string" },
                        "name": { "type": "string" },
                        "kind": { "type": "string" },
                        "location": { "type": "string" },
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
                ToolExecutionResult::ok_structured(
                    serde_json::to_value(&rec).unwrap_or(json!({})),
                    format!("Incremented usage count for '{}'", rec.name),
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
