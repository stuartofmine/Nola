use std::collections::{HashMap, HashSet};
use std::time::{SystemTime, UNIX_EPOCH};

use nola_core::tool_catalog::builtin_tool_metadata;
use nola_core::tool_policy::{
    resolve_tool_policy, CapabilityAuthorization, ConversationToolPolicy, ResultEgressGrant,
    RuntimeCapabilityManifest, RuntimeResultEgressReceipt, RuntimeToolManifest, ToolPlatform,
    ToolPolicyInput, ToolPolicyOutput, ToolRisk, ToolRoute, UserToolPolicy,
};
use serde_json::Value;

#[derive(Debug, Clone)]
pub struct ToolCandidate {
    pub schema: Value,
    pub route: ToolRoute,
    pub result_egress_receipt: Option<RuntimeResultEgressReceipt>,
    pub result_egress_grant: Option<ResultEgressGrant>,
}

#[derive(Debug, Clone)]
pub struct ToolPlanOptions {
    pub platform: ToolPlatform,
    pub conversation_mode: String,
    pub network_policy: String,
    pub denied_tools: Vec<String>,
    pub allow_result_egress_routes: Vec<ToolRoute>,
}

pub fn candidates_from_schemas(schemas: &[Value], route: ToolRoute) -> Vec<ToolCandidate> {
    schemas
        .iter()
        .cloned()
        .map(|schema| ToolCandidate {
            schema,
            route,
            result_egress_receipt: None,
            result_egress_grant: None,
        })
        .collect()
}

pub fn candidate_with_result_egress(
    schema: Value,
    route: ToolRoute,
    receipt: RuntimeResultEgressReceipt,
    grant: ResultEgressGrant,
) -> ToolCandidate {
    ToolCandidate {
        schema,
        route,
        result_egress_receipt: Some(receipt),
        result_egress_grant: Some(grant),
    }
}

pub fn resolve_tool_schemas(
    candidates: Vec<ToolCandidate>,
    options: &ToolPlanOptions,
) -> (Vec<Value>, ToolPolicyOutput) {
    let mut seen = HashSet::new();
    let mut candidates = candidates
        .into_iter()
        .filter_map(|candidate| {
            let name = candidate
                .schema
                .pointer("/function/name")
                .and_then(Value::as_str)?
                .trim()
                .to_ascii_lowercase();
            if name.is_empty() || !seen.insert(name.clone()) {
                return None;
            }
            Some((name, candidate))
        })
        .collect::<Vec<_>>();
    let available_tool_ids = candidates
        .iter()
        .map(|(name, _)| name.clone())
        .collect::<Vec<_>>();
    // The Host catalog is the capability boundary. User wording must not mutate the exposed
    // tools; the model selects semantically and Core evaluates the proposed call.
    let selected_tool_ids = available_tool_ids.iter().cloned().collect::<HashSet<_>>();
    candidates.retain(|(name, _)| selected_tool_ids.contains(name));
    let mut routes = Vec::new();
    let mut capabilities = Vec::new();
    let mut authorizations = HashMap::new();
    let mut tools = Vec::new();
    let mut result_egress_receipts = Vec::new();
    let mut result_egress_grants = Vec::new();
    for (name, candidate) in candidates {
        if !routes.contains(&candidate.route) {
            routes.push(candidate.route);
        }
        let Some(metadata) = builtin_tool_metadata(&name) else {
            tools.push(RuntimeToolManifest {
                tool_id: name.clone(),
                handler_id: Some(name),
                schema: candidate.schema,
                route: Some(candidate.route),
                available: true,
            });
            continue;
        };
        let capability = metadata.capability;
        if capability != "none" && !capabilities.contains(&capability) {
            capabilities.push(capability.clone());
        }
        let authorization = if capability == "network"
            && matches!(
                options.network_policy.trim().to_ascii_lowercase().as_str(),
                "deny" | "denied" | "disabled" | "forbid"
            ) {
            CapabilityAuthorization::Denied
        } else {
            CapabilityAuthorization::Ask
        };
        if capability != "none" {
            authorizations.insert(capability, authorization);
        }
        tools.push(RuntimeToolManifest {
            tool_id: name.clone(),
            handler_id: Some(name),
            schema: candidate.schema,
            route: Some(candidate.route),
            available: true,
        });
        if let Some(receipt) = candidate.result_egress_receipt {
            result_egress_receipts.push(receipt);
        }
        if let Some(grant) = candidate.result_egress_grant {
            result_egress_grants.push(grant);
        }
    }
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64;
    let runtime_egress_scope = result_egress_receipts.first().map(|receipt| {
        (
            receipt.manifest_id.clone(),
            receipt.host_id.clone(),
            receipt.client_id.clone(),
            receipt.session_id.clone(),
            receipt.issued_at_ms,
            receipt.expires_at_ms,
        )
    });
    let (manifest_id, host_id, client_id, session_id, issued_at_ms, expires_at_ms) =
        runtime_egress_scope.unwrap_or_else(|| {
            (
                format!(
                    "cli-tool-plan:{:?}:{}",
                    options.platform,
                    options.conversation_mode.trim().to_ascii_lowercase()
                ),
                String::new(),
                String::new(),
                String::new(),
                now_ms,
                now_ms.saturating_add(60_000),
            )
        });
    let input = ToolPolicyInput {
        platform: options.platform,
        now_ms,
        runtime: RuntimeCapabilityManifest {
            manifest_id,
            manifest_version: 1,
            issued_at_ms,
            expires_at_ms,
            host_id: (!host_id.is_empty()).then_some(host_id),
            client_id: (!client_id.is_empty()).then_some(client_id),
            session_id: (!session_id.is_empty()).then_some(session_id),
            routes,
            capabilities,
            authorizations,
            result_egress_receipts,
            tools,
            ..RuntimeCapabilityManifest::default()
        },
        conversation: ConversationToolPolicy {
            mode: options.conversation_mode.clone(),
            ..ConversationToolPolicy::default()
        },
        user_policy: UserToolPolicy {
            deny_tools: options.denied_tools.clone(),
            max_risk: ToolRisk::Critical,
            allow_result_egress_routes: options.allow_result_egress_routes.clone(),
            ..UserToolPolicy::default()
        },
        result_egress_grants,
        ..ToolPolicyInput::default()
    };
    let output = resolve_tool_policy(&input);
    let enabled = output
        .enabled_tools
        .iter()
        .map(|tool| tool.schema.clone())
        .collect();
    (enabled, output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn schema(name: &str) -> Value {
        json!({
            "type": "function",
            "function": {"name": name, "parameters": {"type": "object"}}
        })
    }

    fn options(platform: ToolPlatform) -> ToolPlanOptions {
        ToolPlanOptions {
            platform,
            conversation_mode: "agent".into(),
            network_policy: "allow".into(),
            denied_tools: Vec::new(),
            allow_result_egress_routes: Vec::new(),
        }
    }

    #[test]
    fn device_only_candidate_does_not_open_local_catalog() {
        let candidates =
            candidates_from_schemas(&[schema("device_get_location")], ToolRoute::Device);
        let options = options(ToolPlatform::Web);
        let (enabled, output) = resolve_tool_schemas(candidates, &options);
        assert!(enabled.is_empty());
        assert_eq!(output.disabled_tools[0].name, "device_get_location");
        assert_eq!(
            output.disabled_tools[0].reason,
            "result_egress_receipt_missing"
        );
    }

    #[test]
    fn device_candidate_requires_exact_egress_receipt_and_session_grant() {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .min(u64::MAX as u128) as u64;
        let manifest_id = "paired-device-manifest".to_string();
        let receipt = RuntimeResultEgressReceipt {
            tool_id: "device_get_location".into(),
            route: ToolRoute::Device,
            manifest_id: manifest_id.clone(),
            host_id: "host-a".into(),
            client_id: "client-a".into(),
            session_id: "session-a".into(),
            issued_at_ms: now_ms,
            expires_at_ms: now_ms.saturating_add(60_000),
            allow_result_egress: true,
        };
        let grant = ResultEgressGrant {
            tool_id: "device_get_location".into(),
            route: ToolRoute::Device,
            manifest_id,
            host_id: "host-a".into(),
            client_id: "client-a".into(),
            session_id: "session-a".into(),
            expires_at_ms: now_ms.saturating_add(60_000),
        };
        let candidate = candidate_with_result_egress(
            schema("device_get_location"),
            ToolRoute::Device,
            receipt,
            grant,
        );
        let mut options = options(ToolPlatform::Web);
        options.allow_result_egress_routes = vec![ToolRoute::Device];

        let (enabled, output) = resolve_tool_schemas(vec![candidate], &options);

        assert_eq!(enabled.len(), 1, "disabled={:?}", output.disabled_tools);
        assert_eq!(output.enabled_tools[0].name, "device_get_location");
    }

    #[test]
    fn network_and_tool_denials_only_narrow_candidates() {
        let mut options = options(ToolPlatform::Cli);
        options.network_policy = "deny".into();
        options.denied_tools = vec!["bash".into()];
        let candidates = candidates_from_schemas(
            &[schema("read_file"), schema("web_search"), schema("bash")],
            ToolRoute::AgentHost,
        );
        let (enabled, output) = resolve_tool_schemas(candidates, &options);
        assert_eq!(enabled.len(), 1);
        assert_eq!(enabled[0]["function"]["name"], "read_file");
        assert_eq!(output.disabled_tools.len(), 2);
    }

    #[test]
    fn web_turn_catalog_is_derived_only_from_candidates_and_policy() {
        let options = options(ToolPlatform::Web);
        let candidates = candidates_from_schemas(
            &[
                schema("web_search"),
                schema("web_fetch"),
                schema("system_time"),
                schema("read_file"),
                schema("bash"),
                schema("computer_click"),
            ],
            ToolRoute::AgentHost,
        );
        let (enabled, _) = resolve_tool_schemas(candidates, &options);
        let names = enabled
            .iter()
            .filter_map(|schema| schema.pointer("/function/name").and_then(Value::as_str))
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            vec![
                "web_search",
                "web_fetch",
                "system_time",
                "read_file",
                "bash",
                "computer_click",
            ]
        );
    }

    #[test]
    fn generic_cli_schemas_are_core_owned_and_host_protocols_stay_host_only() {
        let unknown = crate::tools::tool_schemas()
            .into_iter()
            .filter_map(|schema| {
                let name = schema
                    .pointer("/function/name")
                    .and_then(Value::as_str)?
                    .to_string();
                builtin_tool_metadata(&name).is_none().then_some(name)
            })
            .collect::<Vec<_>>();
        assert!(
            unknown.is_empty(),
            "generic tools must have Core-owned metadata: {unknown:?}"
        );
        assert!(!crate::tools::tool_schemas().iter().any(|schema| {
            schema.pointer("/function/name").and_then(Value::as_str) == Some("skill_update")
        }));
        assert!(crate::tools::tool_schemas_for_host().iter().any(|schema| {
            schema.pointer("/function/name").and_then(Value::as_str) == Some("skill_update")
        }));
    }
}
