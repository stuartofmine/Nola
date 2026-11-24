// SPDX-License-Identifier: Apache-2.0
//! Scheduled Automations & External Messaging Channels module.

use std::sync::Arc;
use std::time::Duration;

use nola_tools::{ToolContext, ToolHandler, ToolRegistry};
use nola_types::{
    new_id, now_ms, AutomationRecord, CapabilityDomain, MessagingChannel, ToolExecutionResult,
    ToolSchema, ToolSpec, ToolTier,
};
use serde_json::{json, Value};

use crate::ModulesSuite;

pub const KIND_AUTOMATION: &str = "automation";
pub const KIND_MESSAGING: &str = "messaging_channel";

pub fn register_tools(suite: &Arc<ModulesSuite>, registry: &mut ToolRegistry) {
    registry.register(Arc::new(AutomationManageTool {
        suite: Arc::clone(suite),
    }));
    registry.register(Arc::new(MessagingManageTool {
        suite: Arc::clone(suite),
    }));
}

struct AutomationManageTool {
    suite: Arc<ModulesSuite>,
}

impl ToolHandler for AutomationManageTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "automation_manage".into(),
                description: "Create, list, enable/disable, or delete scheduled Automations.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "action": { "type": "string", "enum": ["create", "list", "delete"] },
                        "id": { "type": "string" },
                        "name": { "type": "string" },
                        "schedule": { "type": "string" },
                        "prompt": { "type": "string" },
                        "enabled": { "type": "boolean" }
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
                let rec = AutomationRecord {
                    id: args
                        .get("id")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string)
                        .unwrap_or_else(new_id),
                    name: args
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Automation")
                        .to_string(),
                    schedule: args
                        .get("schedule")
                        .and_then(|v| v.as_str())
                        .unwrap_or("0 * * * *")
                        .to_string(),
                    prompt: args
                        .get("prompt")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    workspace_root: self.suite.workspace_root.display().to_string(),
                    enabled: args.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true),
                    last_run_ms: None,
                    updated_at_ms: now_ms(),
                };
                match self.suite.upsert_record(KIND_AUTOMATION, &rec.id, &rec) {
                    Ok(()) => ToolExecutionResult::ok_structured(
                        serde_json::to_value(&rec).unwrap_or(json!({})),
                        format!("Saved automation '{}'", rec.name),
                    ),
                    Err(e) => ToolExecutionResult::err_structured("automation_error", e.to_string(), None),
                }
            }
            "list" => match self.suite.list_records::<AutomationRecord>(KIND_AUTOMATION) {
                Ok(items) => {
                    let count = items.len();
                    ToolExecutionResult::ok_structured(
                        json!({"count": count, "automations": items}),
                        format!("Listed {count} automation(s)"),
                    )
                }
                Err(e) => ToolExecutionResult::err_structured("automation_error", e.to_string(), None),
            },
            "delete" => {
                let Some(id) = args.get("id").and_then(|v| v.as_str()) else {
                    return ToolExecutionResult::err_structured("missing_param", "missing 'id'", None);
                };
                match self.suite.delete_record(KIND_AUTOMATION, id) {
                    Ok(deleted) => ToolExecutionResult::ok_structured(
                        json!({"id": id, "deleted": deleted}),
                        format!("Deleted automation '{id}'"),
                    ),
                    Err(e) => ToolExecutionResult::err_structured("automation_error", e.to_string(), None),
                }
            }
            other => ToolExecutionResult::err_structured(
                "invalid_action",
                format!("unsupported automation action '{other}'"),
                None,
            ),
        }
    }
}

struct MessagingManageTool {
    suite: Arc<ModulesSuite>,
}

impl ToolHandler for MessagingManageTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            schema: ToolSchema {
                name: "messaging_manage".into(),
                description: "Configure, list, or send notifications through Messaging Channels (webhook/Slack/Telegram).".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "action": { "type": "string", "enum": ["create", "list", "send", "delete"] },
                        "id": { "type": "string" },
                        "name": { "type": "string" },
                        "kind": { "type": "string" },
                        "endpoint": { "type": "string" },
                        "text": { "type": "string" }
                    },
                    "required": ["action"]
                }),
            },
            tier: ToolTier::Deferred,
            domain: CapabilityDomain::Network,
            read_only: false,
        }
    }

    fn execute(&self, ctx: &ToolContext, args: Value) -> ToolExecutionResult {
        match args.get("action").and_then(|v| v.as_str()).unwrap_or("") {
            "create" => {
                let ch = MessagingChannel {
                    id: args
                        .get("id")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string)
                        .unwrap_or_else(new_id),
                    kind: args
                        .get("kind")
                        .and_then(|v| v.as_str())
                        .unwrap_or("webhook")
                        .to_string(),
                    name: args
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Channel")
                        .to_string(),
                    endpoint: args
                        .get("endpoint")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    enabled: true,
                    updated_at_ms: now_ms(),
                };
                match self.suite.upsert_record(KIND_MESSAGING, &ch.id, &ch) {
                    Ok(()) => ToolExecutionResult::ok_structured(
                        serde_json::to_value(&ch).unwrap_or(json!({})),
                        format!("Saved messaging channel '{}'", ch.name),
                    ),
                    Err(e) => ToolExecutionResult::err_structured("messaging_error", e.to_string(), None),
                }
            }
            "list" => match self.suite.list_records::<MessagingChannel>(KIND_MESSAGING) {
                Ok(items) => {
                    let count = items.len();
                    ToolExecutionResult::ok_structured(
                        json!({"count": count, "channels": items}),
                        format!("Listed {count} messaging channel(s)"),
                    )
                }
                Err(e) => ToolExecutionResult::err_structured("messaging_error", e.to_string(), None),
            },
            "send" => {
                if !ctx.network_allowed {
                    return ToolExecutionResult::err_structured(
                        "network_denied",
                        "outbound messaging blocked by network policy",
                        None,
                    );
                }
                let Some(id) = args.get("id").and_then(|v| v.as_str()) else {
                    return ToolExecutionResult::err_structured("missing_param", "missing 'id'", None);
                };
                let text = args.get("text").and_then(|v| v.as_str()).unwrap_or("");
                let Ok(Some(ch)) = self.suite.get_record::<MessagingChannel>(KIND_MESSAGING, id) else {
                    return ToolExecutionResult::err_structured("not_found", format!("channel '{id}' not found"), None);
                };
                if !ch.enabled {
                    return ToolExecutionResult::err_structured(
                        "channel_disabled",
                        format!("messaging channel '{}' is disabled", ch.name),
                        None,
                    );
                }
                let (parsed, host, addrs) =
                    match nola_tools::web::resolve_and_validate_public_url(&ch.endpoint) {
                        Ok(v) => v,
                        Err(e) => {
                            return ToolExecutionResult::err_structured("ssrf_blocked", e, None);
                        }
                    };
                let client = reqwest::blocking::Client::builder()
                    .timeout(Duration::from_secs(10))
                    .redirect(reqwest::redirect::Policy::none())
                    .resolve_to_addrs(&host, &addrs)
                    .build();
                match client.and_then(|c| c.post(parsed).json(&json!({"text": text})).send()) {
                    Ok(resp) if resp.status().is_success() => ToolExecutionResult::ok_structured(
                        json!({"channel": ch.name, "status": resp.status().as_u16()}),
                        format!("Delivered message to '{}'", ch.name),
                    ),
                    Ok(resp) => ToolExecutionResult::err_structured(
                        "delivery_failed",
                        format!("channel returned HTTP {}", resp.status()),
                        None,
                    ),
                    Err(e) => ToolExecutionResult::err_structured("delivery_error", e.to_string(), None),
                }
            }
            "delete" => {
                let Some(id) = args.get("id").and_then(|v| v.as_str()) else {
                    return ToolExecutionResult::err_structured("missing_param", "missing 'id'", None);
                };
                match self.suite.delete_record(KIND_MESSAGING, id) {
                    Ok(deleted) => ToolExecutionResult::ok_structured(
                        json!({"id": id, "deleted": deleted}),
                        format!("Deleted channel '{id}'"),
                    ),
                    Err(e) => ToolExecutionResult::err_structured("messaging_error", e.to_string(), None),
                }
            }
            other => ToolExecutionResult::err_structured(
                "invalid_action",
                format!("unsupported messaging action '{other}'"),
                None,
            ),
        }
    }
}
