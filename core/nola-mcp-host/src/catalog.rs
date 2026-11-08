// SPDX-License-Identifier: Apache-2.0
//! Identical Core descriptors for desktop and embedded MCP Hosts.
use nola_core::agent::{host::*, capability::*};
use serde_json::{json, Value};

pub fn tool_id(server: &McpServerDescriptor, tool: &McpToolDescriptor) -> String {
    nola_core::code_intelligence::stable_capability_id(&server.server_id,&tool.name)
        .map(str::to_owned).unwrap_or_else(||mcp_model_tool_id(&server.server_id,&tool.name))
}
pub fn tool_descriptor(server: &McpServerDescriptor, tool: &McpToolDescriptor) -> HostCapabilityDescriptor {
    let capability_id=tool_id(server,tool);
    let read_only=nola_core::code_intelligence::stable_capability_id(&server.server_id,&tool.name)
        .and_then(nola_core::code_intelligence::operation_for_capability_id)
        .map(nola_core::code_intelligence::CodeIntelligenceOperation::is_read_only).unwrap_or(tool.read_only);
    let destructive=!read_only && tool.destructive;
    let mut input_schema=tool.input_schema.clone();
    if let (Some(object),Some(description))=(input_schema.as_object_mut(),tool.description.as_deref().filter(|s|!s.trim().is_empty())) {
        object.insert("description".into(),Value::String(description.into()));
    }
HostCapabilityDescriptor {
                        id: capability_id.clone(),
                        title: format!(
                            "{} / {}",
                            server.display_name,
                            tool.title.as_deref().unwrap_or(&tool.name)
                        ),
                        route: HostCapabilityRoute::Tool,
                        availability: CapabilityAvailability::Available,
                        // MCP registries can be arbitrarily large and can change at
                        // runtime. Keep their schemas out of the initial model request;
                        // Core's frozen catalog remains searchable through tool_search
                        // and promotes the selected capability on the next model step.
                        exposure: HostCapabilityExposure::Deferred,
                        permission_domain: Some(if destructive {"mcp_destructive"} else {"mcp"}.into()),
                        input_schema,
                        input_normalization: HostInputNormalization {
                            mcp_route: crate::mcp_connection_authority::permission_resource(&server.connection_ref).ok().map(|resource|
                                nola_core::agent::host::HostMcpToolRoute {server_id:server.server_id.clone(),tool_name:tool.name.clone(),
                                    connection_ref:server.connection_ref.clone(),config_digest:resource.config_digest}),
                            origin: nola_core::agent::host::HostCapabilityOrigin {
                                kind: nola_core::agent::host::HostCapabilityOriginKind::Mcp,
                                provider_id: Some(server.server_id.clone()),
                                provider_title: Some(server.display_name.clone()),
                                namespace: Some("mcp".to_string()),
                            },
                            effect: nola_core::agent::host::HostCapabilityEffectDeclaration::fixed(
                                if read_only {
                                    nola_core::agent::host::HostCapabilityEffect::Investigate
                                } else {
                                    nola_core::agent::host::HostCapabilityEffect::Mutate
                                },
                            ),
                            permission_resource:
                                nola_core::agent::host::HostPermissionResourceDeclaration::FullInput,
                            ..Default::default()
                        },
                        output_policy: HostOutputPolicy {
                            preview_char_limit: 8_000,
                            persist_full_output: true,
                            allow_artifacts: true,
                        },
                        supports_streaming: false,
                        supports_argument_streaming: false,
                        parallelism: if read_only {
                            HostCapabilityParallelism::ConcurrentSafe
                        } else {
                            HostCapabilityParallelism::Serial
                        },
                        cancellation: HostCancellationMode::BestEffort,
                        replay_safety: if read_only {
                            HostReplaySafety::Idempotent
                        } else {
                            HostReplaySafety::RequiresRecoveryDecision
                        },
                    }
}
pub fn connect_descriptor(configured: Vec<Value>) -> HostCapabilityDescriptor {
    let entries=configured.iter().filter(|v|v["enabled"]!=false).filter_map(|v| {
        let reference=v["connection_ref"].as_str()?;
        let name=v["display_name"].as_str().or_else(||v["server_id"].as_str()).unwrap_or("MCP");
        let target=crate::mcp_connection_authority::resource_from_configuration(v).ok().map(|r|r.target).unwrap_or_default();
        Some(json!({"const":reference,"title":format!("{name} · {target}")}))
    }).collect::<Vec<_>>();
    let references=entries.iter().map(|v|v["const"].clone()).collect::<Vec<_>>();
    let description=entries.iter().map(|v|format!("{}: {}",v["title"].as_str().unwrap_or("MCP"),v["const"].as_str().unwrap_or_default())).collect::<Vec<_>>().join("; ");
    HostCapabilityDescriptor {
        id:"mcp_connect".into(),title:"Connect a configured MCP server (new connections need approval)".into(),
        route:HostCapabilityRoute::Tool,availability:CapabilityAvailability::Available,
        exposure:HostCapabilityExposure::Visible,permission_domain:Some("mcp_connection".into()),
        input_schema:json!({"type":"object","description":format!("Connect one configured MCP server after approval. Then use its discovered tools; connecting does not authorize tool calls. Configured servers: {description}"),"properties":{"connection_ref":{"type":"string","enum":references,"anyOf":entries}},"required":["connection_ref"],"additionalProperties":false}),
        input_normalization:HostInputNormalization {
            origin:HostCapabilityOrigin {kind:HostCapabilityOriginKind::Mcp,provider_id:None,provider_title:None,namespace:Some("mcp".into())},
            effect:HostCapabilityEffectDeclaration::fixed(HostCapabilityEffect::Execute),
            permission_resource:HostPermissionResourceDeclaration::FullInput,..Default::default()
        },
        output_policy:HostOutputPolicy {preview_char_limit:8000,persist_full_output:true,allow_artifacts:true},
        supports_streaming:false,supports_argument_streaming:false,parallelism:HostCapabilityParallelism::Serial,
        cancellation:HostCancellationMode::BestEffort,replay_safety:HostReplaySafety::RequiresRecoveryDecision,
    }
}
