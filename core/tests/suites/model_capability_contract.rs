use nola_core::agent::capability::{
    mcp_model_tool_id, McpCapabilityCatalog, McpPromptArgumentDescriptor, McpPromptDescriptor,
    McpResourceDescriptor, McpResourceTemplateDescriptor, McpServerAvailability,
    McpServerDescriptor, McpToolDescriptor, McpTransportKind, MCP_PROMPT_GET_CAPABILITY_ID,
    MCP_RESOURCE_READ_CAPABILITY_ID, MCP_RESOURCE_TEMPLATE_READ_CAPABILITY_ID,
};
use nola_core::agent::capability_projection::AgentModelToolProjection;
use nola_core::agent::{
    model_instruction_prefix_digest, model_instruction_profile_fragment,
    resolve_model_capability_profile, resolve_model_instruction_profile, ModelCapabilityProfile,
    ModelInstructionProfile,
};
use serde_json::json;
use std::collections::BTreeMap;

#[test]
fn mcp_model_projection_is_stable_bounded_and_structural() {
    let first = mcp_model_tool_id("docs", "search");
    assert_eq!(first, mcp_model_tool_id("docs", "search"));
    assert_ne!(first, mcp_model_tool_id("issues", "search"));
    assert!(first.len() <= 64);
    assert!(first
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || character == '_'));

    let catalog = McpCapabilityCatalog {
        revision: 7,
        servers: BTreeMap::from([(
            "docs".to_string(),
            McpServerDescriptor {
                server_id: "docs".to_string(),
                display_name: "Documentation".to_string(),
                transport: McpTransportKind::StreamableHttp,
                connection_ref: "fixture:docs".to_string(),
                availability: McpServerAvailability::Available,
                protocol_version: Some("2025-06-18".to_string()),
                instructions: None,
                tools: BTreeMap::from([(
                    "search".to_string(),
                    McpToolDescriptor {
                        name: "search".to_string(),
                        title: Some("Search docs".to_string()),
                        description: None,
                        input_schema: json!({"type":"object","required":["query"],"properties":{"query":{"type":"string"}}}),
                        output_schema: None,
                        read_only: true,
                        destructive: false,
                    },
                )]),
                resources: BTreeMap::from([(
                    "guide".to_string(),
                    McpResourceDescriptor {
                        resource_id: "guide".to_string(),
                        uri: "docs://guide".to_string(),
                        name: "Guide".to_string(),
                        description: None,
                        mime_type: Some("text/markdown".to_string()),
                        annotations: BTreeMap::new(),
                    },
                )]),
                resource_templates: BTreeMap::from([(
                    "docs://guide/{section}".to_string(),
                    McpResourceTemplateDescriptor {
                        template_id: "docs://guide/{section}".to_string(),
                        uri_template: "docs://guide/{section}".to_string(),
                        name: "Guide section".to_string(),
                        description: None,
                        mime_type: Some("text/markdown".to_string()),
                    },
                )]),
                prompts: BTreeMap::from([(
                    "summarize".to_string(),
                    McpPromptDescriptor {
                        name: "summarize".to_string(),
                        title: Some("Summarize".to_string()),
                        description: None,
                        arguments: vec![McpPromptArgumentDescriptor {
                            name: "tone".to_string(),
                            description: None,
                            required: false,
                        }],
                    },
                )]),
            },
        )]),
    };
    let definitions = catalog.model_tool_definitions();
    assert_eq!(definitions.len(), 4);
    assert_eq!(definitions[0]["function"]["name"], first);
    assert_eq!(
        definitions[0]["function"]["parameters"]["required"][0],
        "query"
    );
    let resource = definitions
        .iter()
        .find(|definition| definition["function"]["name"] == MCP_RESOURCE_READ_CAPABILITY_ID)
        .expect("fixed resource capability");
    assert_eq!(
        resource["function"]["parameters"]["additionalProperties"],
        false
    );
    assert_eq!(
        resource["function"]["parameters"]["properties"]["resource_id"]["maxLength"],
        2048
    );
    let resource_template = definitions
        .iter()
        .find(|definition| {
            definition["function"]["name"] == MCP_RESOURCE_TEMPLATE_READ_CAPABILITY_ID
        })
        .expect("fixed resource template capability");
    assert_eq!(
        resource_template["function"]["parameters"]["properties"]["variables"]["maxProperties"],
        64
    );
    let prompt = definitions
        .iter()
        .find(|definition| definition["function"]["name"] == MCP_PROMPT_GET_CAPABILITY_ID)
        .expect("fixed prompt capability");
    assert_eq!(
        prompt["function"]["parameters"]["properties"]["arguments"]["maxProperties"],
        64
    );
}

#[test]
fn unknown_profile_degrades_conservatively() {
    let plan = ModelCapabilityProfile::default().invocation_plan(64_000, true);
    assert!(!plan.native_tool_calls);
    assert!(!plan.parallel_tool_calls);
    assert!(!plan.reasoning);
    assert!(!plan.reasoning_continuity);
    assert!(!plan.prompt_cache);
    assert!(!plan.request_continuation);
    assert!(!plan.stream_text);
    assert_eq!(plan.effective_context_window_tokens, 64_000);
    assert_eq!(plan.max_request_continuations, 0);
    assert_eq!(plan.semantic_digest.len(), 64);
}

#[test]
fn invocation_semantics_invalidate_model_prefix_identity() {
    let projection = AgentModelToolProjection::compile(
        1,
        "turn",
        vec![
            json!({"type":"function","function":{"name":"read_file","parameters":{"type":"object"}}}),
        ],
    ).unwrap().projection;
    let stable = "a".repeat(64);
    let profile = ModelCapabilityProfile {
        profile_id: "fixture".to_string(),
        revision: 1,
        instruction_profile: ModelInstructionProfile::OpenAi,
        native_tool_calls: true,
        parallel_tool_calls: true,
        reasoning: true,
        reasoning_continuity: true,
        prompt_cache: true,
        vision_input: true,
        native_pdf_input: false,
        context_window_tokens: Some(128_000),
        max_output_tokens: Some(8_192),
        request_continuation: true,
        max_request_continuations: 2,
        stream_text: true,
        stream_reasoning: true,
        stream_tool_calls: true,
        direct_tool_capability_ids: Vec::new(),
    };
    let cached = profile.invocation_plan(128_000, true);
    let mut no_cache = profile;
    no_cache.prompt_cache = false;
    let uncached = no_cache.invocation_plan(128_000, true);

    let first =
        projection.model_prefix_digest_with_invocation_policy(&stable, &cached.semantic_digest);
    let second =
        projection.model_prefix_digest_with_invocation_policy(&stable, &uncached.semantic_digest);
    assert_eq!(first.len(), 64);
    assert_ne!(first, second);
}

#[test]
fn model_instruction_profile_is_typed_and_part_of_invocation_identity() {
    assert_eq!(
        resolve_model_instruction_profile(Some("dashscope"), "qwen3.5-35b-a3b"),
        ModelInstructionProfile::Qwen
    );
    assert_eq!(
        resolve_model_instruction_profile(Some("custom"), "unknown-model"),
        ModelInstructionProfile::Generic
    );
    assert_eq!(
        resolve_model_instruction_profile(Some("openai"), "qwen3.5-35b-a3b"),
        ModelInstructionProfile::Qwen
    );

    let generic = ModelCapabilityProfile::default().invocation_plan(64_000, false);
    let qwen = ModelCapabilityProfile {
        instruction_profile: ModelInstructionProfile::Qwen,
        ..ModelCapabilityProfile::default()
    }
    .invocation_plan(64_000, false);
    assert_ne!(generic.semantic_digest, qwen.semantic_digest);

    let fragment =
        model_instruction_profile_fragment(ModelInstructionProfile::Qwen).expect("qwen overlay");
    assert!(fragment.cacheable);
    assert!(fragment.content.contains("native tool-call protocol"));
    assert!(model_instruction_profile_fragment(ModelInstructionProfile::Generic).is_none());
    assert_eq!(
        model_instruction_prefix_digest("stable", ModelInstructionProfile::Generic),
        "stable"
    );
    assert_ne!(
        model_instruction_prefix_digest("stable", ModelInstructionProfile::Qwen),
        "stable"
    );
}

#[test]
fn reasoning_without_continuity_disables_request_continuation() {
    let profile = ModelCapabilityProfile {
        profile_id: "reasoning-without-continuity".to_string(),
        revision: 1,
        reasoning: true,
        reasoning_continuity: false,
        request_continuation: true,
        max_request_continuations: 3,
        ..ModelCapabilityProfile::default()
    };
    let plan = profile.invocation_plan(64_000, false);
    assert!(!plan.request_continuation);
    assert_eq!(plan.max_request_continuations, 0);
}

#[test]
fn model_profile_resolution_is_bounded_and_unknown_models_are_conservative() {
    assert_eq!(
        resolve_model_instruction_profile(Some("custom"), "my-qwen-proxy"),
        ModelInstructionProfile::Generic
    );
    assert_eq!(
        resolve_model_instruction_profile(None, "Qwen/Qwen3.5-35B-A3B"),
        ModelInstructionProfile::Qwen
    );

    let fallback = resolve_model_capability_profile(Some("openai-compatible"), "private-model");
    assert!(fallback.profile_id.contains("generic.fallback"));
    assert!(fallback.native_tool_calls);
    assert!(!fallback.parallel_tool_calls);
    assert!(!fallback.reasoning);
    assert!(!fallback.reasoning_continuity);
    assert!(!fallback.stream_reasoning);

    let qwen = resolve_model_capability_profile(Some("dashscope"), "private-deployment-id");
    assert!(qwen.profile_id.contains("qwen.provider"));
    assert!(!qwen.parallel_tool_calls);
    assert!(!qwen.reasoning_continuity);
    assert_eq!(
        ModelInstructionProfile::parse("Claude"),
        Some(ModelInstructionProfile::Anthropic)
    );
    assert_eq!(ModelInstructionProfile::parse("unknown"), None);
}
