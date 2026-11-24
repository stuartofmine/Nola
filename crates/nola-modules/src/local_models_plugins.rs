// SPDX-License-Identifier: Apache-2.0
//! Local Models / Acquisition catalog & Harness Plugins module.

use std::io::Read;
use std::sync::Arc;
use std::time::Duration;

use nola_tools::{ToolContext, ToolHandler, ToolRegistry};
use nola_types::{
    new_id, now_ms, CapabilityDomain, LocalModelRecord, PluginRecord, ToolExecutionResult,
    ToolSchema, ToolSpec, ToolTier,
};
use serde_json::{json, Value};
use url::{Host, Url};

use crate::ModulesSuite;

const MAX_PROBE_BODY_BYTES: u64 = 256 * 1024;

pub const KIND_LOCAL_MODEL: &str = "local_model";
pub const KIND_PLUGIN: &str = "plugin";

pub fn register_tools(suite: &Arc<ModulesSuite>, registry: &mut ToolRegistry) {
    registry.register(Arc::new(LocalModelManageTool {
        suite: Arc::clone(suite),
    }));
    registry.register(Arc::new(PluginManageTool {
        suite: Arc::clone(suite),
    }));
}

struct LocalModelManageTool {
    suite: Arc<ModulesSuite>,
}

impl ToolHandler for LocalModelManageTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "local_model_manage".into(),
                description: "Register, list, or probe local LLM engines (Strata/Ollama/vLLM/llama.cpp).".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "action": { "type": "string", "enum": ["register", "list", "probe", "delete"] },
                        "id": { "type": "string" },
                        "name": { "type": "string" },
                        "engine": { "type": "string" },
                        "endpoint": { "type": "string" }
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
            "register" => {
                let rec = LocalModelRecord {
                    id: args
                        .get("id")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string)
                        .unwrap_or_else(new_id),
                    name: args
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("local-model")
                        .to_string(),
                    engine: args
                        .get("engine")
                        .and_then(|v| v.as_str())
                        .unwrap_or("llama.cpp")
                        .to_string(),
                    path: args.get("path").and_then(|v| v.as_str()).map(ToString::to_string),
                    size_bytes: args.get("size_bytes").and_then(|v| v.as_u64()).unwrap_or(0),
                    quantization: args
                        .get("quantization")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string),
                    installed: true,
                    updated_at_ms: now_ms(),
                };
                match self.suite.upsert_record(KIND_LOCAL_MODEL, &rec.id, &rec) {
                    Ok(()) => ToolExecutionResult::ok_structured(
                        serde_json::to_value(&rec).unwrap_or(json!({})),
                        format!("Registered local model '{}'", rec.name),
                    ),
                    Err(e) => ToolExecutionResult::err_structured("local_model_error", e.to_string(), None),
                }
            }
            "list" => match self.suite.list_records::<LocalModelRecord>(KIND_LOCAL_MODEL) {
                Ok(items) => {
                    let count = items.len();
                    ToolExecutionResult::ok_structured(
                        json!({"count": count, "models": items}),
                        format!("Listed {count} local model(s)"),
                    )
                }
                Err(e) => ToolExecutionResult::err_structured("local_model_error", e.to_string(), None),
            },
            "probe" => {
                let endpoint = args
                    .get("endpoint")
                    .and_then(|v| v.as_str())
                    .unwrap_or("http://127.0.0.1:8081");
                let raw_url = format!("{}/v1/models", endpoint.trim_end_matches('/'));
                let parsed = match Url::parse(&raw_url) {
                    Ok(u) if matches!(u.scheme(), "http" | "https") => u,
                    _ => {
                        return ToolExecutionResult::err_structured(
                            "invalid_endpoint",
                            "endpoint must be a valid http(s) URL",
                            None,
                        );
                    }
                };
                let is_loopback = match parsed.host() {
                    Some(Host::Domain(d)) => d.eq_ignore_ascii_case("localhost"),
                    Some(Host::Ipv4(v4)) => v4.is_loopback(),
                    Some(Host::Ipv6(v6)) => v6.is_loopback(),
                    None => false,
                };
                if !is_loopback {
                    if !ctx.network_allowed {
                        return ToolExecutionResult::err_structured(
                            "network_denied",
                            "non-loopback endpoint probe blocked by network policy",
                            None,
                        );
                    }
                    if let Err(e) = nola_tools::web::resolve_and_validate_public_url(&raw_url) {
                        return ToolExecutionResult::err_structured("ssrf_blocked", e, None);
                    }
                }
                let res = reqwest::blocking::Client::builder()
                    .timeout(Duration::from_secs(5))
                    .redirect(reqwest::redirect::Policy::none())
                    .build()
                    .and_then(|c| c.get(parsed).send());
                match res {
                    Ok(resp) if resp.status().is_success() => {
                        let mut buf = Vec::new();
                        let _ = resp.take(MAX_PROBE_BODY_BYTES).read_to_end(&mut buf);
                        let body: Value = serde_json::from_slice(&buf).unwrap_or(json!({}));
                        ToolExecutionResult::ok_structured(
                            json!({"endpoint": endpoint, "reachable": true, "models": body}),
                            format!("Probed local endpoint {endpoint}"),
                        )
                    }
                    Ok(resp) => ToolExecutionResult::err_structured(
                        "probe_failed",
                        format!("{raw_url} returned HTTP {}", resp.status()),
                        None,
                    ),
                    Err(e) => ToolExecutionResult::err_structured("probe_error", e.to_string(), None),
                }
            }
            "delete" => {
                let Some(id) = args.get("id").and_then(|v| v.as_str()) else {
                    return ToolExecutionResult::err_structured("missing_param", "missing 'id'", None);
                };
                match self.suite.delete_record(KIND_LOCAL_MODEL, id) {
                    Ok(deleted) => ToolExecutionResult::ok_structured(
                        json!({"id": id, "deleted": deleted}),
                        format!("Deleted local model '{id}'"),
                    ),
                    Err(e) => ToolExecutionResult::err_structured("local_model_error", e.to_string(), None),
                }
            }
            other => ToolExecutionResult::err_structured(
                "invalid_action",
                format!("unsupported local_model action '{other}'"),
                None,
            ),
        }
    }
}

struct PluginManageTool {
    suite: Arc<ModulesSuite>,
}

impl ToolHandler for PluginManageTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "plugin_manage".into(),
                description: "Register, list, enable/disable, or remove Harness Plugins.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "action": { "type": "string", "enum": ["register", "list", "delete"] },
                        "id": { "type": "string" },
                        "name": { "type": "string" },
                        "version": { "type": "string" },
                        "entry_path": { "type": "string" }
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
            "register" => {
                let rec = PluginRecord {
                    id: args
                        .get("id")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string)
                        .unwrap_or_else(new_id),
                    name: args
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("plugin")
                        .to_string(),
                    version: args
                        .get("version")
                        .and_then(|v| v.as_str())
                        .unwrap_or("0.1.0")
                        .to_string(),
                    entry_path: args
                        .get("entry_path")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    enabled: true,
                    capabilities: Vec::new(),
                };
                match self.suite.upsert_record(KIND_PLUGIN, &rec.id, &rec) {
                    Ok(()) => ToolExecutionResult::ok_structured(
                        serde_json::to_value(&rec).unwrap_or(json!({})),
                        format!("Registered plugin '{}'", rec.name),
                    ),
                    Err(e) => ToolExecutionResult::err_structured("plugin_error", e.to_string(), None),
                }
            }
            "list" => match self.suite.list_records::<PluginRecord>(KIND_PLUGIN) {
                Ok(items) => {
                    let count = items.len();
                    ToolExecutionResult::ok_structured(
                        json!({"count": count, "plugins": items}),
                        format!("Listed {count} plugin(s)"),
                    )
                }
                Err(e) => ToolExecutionResult::err_structured("plugin_error", e.to_string(), None),
            },
            "delete" => {
                let Some(id) = args.get("id").and_then(|v| v.as_str()) else {
                    return ToolExecutionResult::err_structured("missing_param", "missing 'id'", None);
                };
                match self.suite.delete_record(KIND_PLUGIN, id) {
                    Ok(deleted) => ToolExecutionResult::ok_structured(
                        json!({"id": id, "deleted": deleted}),
                        format!("Deleted plugin '{id}'"),
                    ),
                    Err(e) => ToolExecutionResult::err_structured("plugin_error", e.to_string(), None),
                }
            }
            other => ToolExecutionResult::err_structured(
                "invalid_action",
                format!("unsupported plugin action '{other}'"),
                None,
            ),
        }
    }
}
