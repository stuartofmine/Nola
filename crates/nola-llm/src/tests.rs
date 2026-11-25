// SPDX-License-Identifier: Apache-2.0
//! Gate 3 & Gate 4: Parameterized 4-Protocol Wire Conformance Matrix.
//! Verifies all 4 protocols (`OpenAiChat`, `OpenAiResponses`, `AnthropicMessages`,
//! `SiwcResponses`) against every invariant that broke in v1.

use std::io::Cursor;

use nola_types::{
    FrozenSystemBlock, GenerationParams, HistoryMessage, MediaAttachment, ModelCapabilities,
    ModelProtocol, ModelRouteConfig, ModelStopReason, ModelStreamDelta, ModelToolCall,
    ProviderContinuation, RuntimeReminder, StructuredModelRequest, ToolModelPayload, ToolSchema,
};
use serde_json::json;

use crate::{adapter_for, is_local_or_lan_endpoint, is_loopback_url, LlmClient, PrivacyRedactor};

const ALL_PROTOCOLS: [ModelProtocol; 5] = [
    ModelProtocol::OpenAiChat,
    ModelProtocol::OpenAiResponses,
    ModelProtocol::AnthropicMessages,
    ModelProtocol::SiwcResponses,
    ModelProtocol::Gemini,
];

fn sample_route(protocol: ModelProtocol) -> ModelRouteConfig {
    ModelRouteConfig {
        connection_id: "test".into(),
        endpoint: "http://127.0.0.1:8081".into(),
        protocol,
        model: "test-model".into(),
        api_key: "test-key".into(),
        context_window: 262_144,
        max_output_tokens: Some(32_768),
        temperature: Some(0.2),
        top_p: Some(0.95),
        capabilities: ModelCapabilities {
            tool_calls: true,
            image_input: true,
            native_pdf_input: true,
            reasoning: true,
            sampling_parameters: true,
            stream_reasoning: true,
            supported_reasoning_levels: vec!["low".into(), "medium".into(), "high".into()],
            supports_reasoning_summary: false,
        },
        reasoning_effort: Some("medium".into()),
        prompt_cache_key: Some("session-cache-1".into()),
    }
}

fn sample_request(with_reminders: bool) -> StructuredModelRequest {
    StructuredModelRequest {
        frozen_system_prefix: vec![FrozenSystemBlock {
            id: "core".into(),
            text: "You are Nola v2.".into(),
        }],
        prefix_stable_tools: vec![
            ToolSchema {
                name: "read_file".into(),
                description: "Read a file".into(),
                parameters: json!({"type": "object"}),
            },
            ToolSchema {
                name: "bash".into(),
                description: "Run shell".into(),
                parameters: json!({"type": "object"}),
            },
        ],
        history: vec![
            HistoryMessage::User {
                content: "Check main.rs".into(),
                media: vec![],
            },
            HistoryMessage::Assistant {
                content: String::new(),
                reasoning: Some("Old turn thinking".into()),
                tool_calls: vec![ModelToolCall {
                    call_id: "c1".into(),
                    name: "read_file".into(),
                    arguments: json!({"path": "main.rs"}),
                }],
                continuation: None,
            },
            HistoryMessage::Tool {
                call_id: "c1".into(),
                tool_name: "read_file".into(),
                payload: ToolModelPayload::text("1: fn main() {}"),
                is_error: false,
                media: vec![],
            },
            HistoryMessage::User {
                content: "Now run tests".into(),
                media: vec![],
            },
        ],
        ephemeral_tail_reminders: if with_reminders {
            vec![RuntimeReminder {
                tag: "skill".into(),
                text: "Skill body injected at tail".into(),
            }]
        } else {
            vec![]
        },
        max_output_tokens: 32_768,
        json_schema: None,
    }
}

#[test]
fn all_four_protocols_preserve_system_prefix_and_never_emit_mid_system_roles() {
    for protocol in ALL_PROTOCOLS {
        let adapter = adapter_for(protocol);
        let route = sample_route(protocol);
        let r0 = adapter
            .build_request(&route, &sample_request(false), true)
            .unwrap();
        let r1 = adapter
            .build_request(&route, &sample_request(true), true)
            .unwrap();

        match protocol {
            ModelProtocol::OpenAiChat => {
                // Message 0 is identical frozen system prompt
                assert_eq!(r0.body["messages"][0], r1.body["messages"][0]);
                // NO message after index 0 may have role "system" or "developer"
                let msgs = r1.body["messages"].as_array().unwrap();
                for m in &msgs[1..] {
                    let role = m["role"].as_str().unwrap();
                    assert_ne!(role, "system");
                    assert_ne!(role, "developer");
                }
                assert_eq!(msgs.last().unwrap()["role"], "user");
                // Prior-turn reasoning stripped from older assistant turn before last user turn
                assert!(msgs[2].get("reasoning_content").is_none());
                assert_eq!(r1.body["stream_options"]["include_usage"], true);
            }
            ModelProtocol::AnthropicMessages => {
                // body["system"] MUST be byte-for-byte identical even when tail reminders exist!
                assert_eq!(r0.body["system"], r1.body["system"]);
                let msgs = r1.body["messages"].as_array().unwrap();
                // Must NEVER have consecutive user turns
                for w in msgs.windows(2) {
                    assert!(
                        !(w[0]["role"] == "user" && w[1]["role"] == "user"),
                        "consecutive user messages in Anthropic payload"
                    );
                }
            }
            ModelProtocol::OpenAiResponses | ModelProtocol::SiwcResponses => {
                assert_eq!(r0.body["instructions"], r1.body["instructions"]);
                let input = r1.body["input"].as_array().unwrap();
                assert_eq!(input.last().unwrap()["role"], "user");
            }
            ModelProtocol::Gemini => {
                assert_eq!(r0.body["systemInstruction"], r1.body["systemInstruction"]);
                let contents = r1.body["contents"].as_array().unwrap();
                assert_eq!(contents.last().unwrap()["role"], "user");
            }
        }
    }
}

#[test]
fn all_four_protocols_atomically_discard_truncated_tool_calls_on_output_limit() {
    // 1. OpenAI Chat Completions
    let chat = adapter_for(ModelProtocol::OpenAiChat);
    let chat_resp = chat
        .decode_response(
            &sample_route(ModelProtocol::OpenAiChat),
            &json!({
                "id": "chatcmpl-1",
                "model": "qwen3",
                "choices": [{
                    "finish_reason": "length",
                    "message": {
                        "role": "assistant",
                        "content": "partial",
                        "tool_calls": [{
                            "id": "c1",
                            "type": "function",
                            "function": {"name": "write_file", "arguments": "{\"path\":\"a.py\",\"content\":\"TRUNCATED"}
                        }]
                    }
                }]
            }),
        )
        .unwrap();
    assert_eq!(chat_resp.stop_reason, ModelStopReason::OutputLimit);
    assert!(chat_resp.tool_calls.is_empty());

    // 2. OpenAI Responses & 3. SIWC Responses
    for proto in [ModelProtocol::OpenAiResponses, ModelProtocol::SiwcResponses] {
        let ad = adapter_for(proto);
        let resp = ad
            .decode_response(
                &sample_route(proto),
                &json!({
                    "id": "resp_1",
                    "model": "gpt-5",
                    "status": "incomplete",
                    "incomplete_details": {"reason": "max_output_tokens"},
                    "output": [{
                        "type": "function_call",
                        "call_id": "c1",
                        "name": "write_file",
                        "arguments": "{\"broken"
                    }]
                }),
            )
            .unwrap();
        assert_eq!(resp.stop_reason, ModelStopReason::OutputLimit);
        assert!(resp.tool_calls.is_empty());
    }

    // 4. Anthropic Messages
    let anth = adapter_for(ModelProtocol::AnthropicMessages);
    let anth_resp = anth
        .decode_response(
            &sample_route(ModelProtocol::AnthropicMessages),
            &json!({
                "id": "msg_1",
                "model": "claude-sonnet",
                "stop_reason": "max_tokens",
                "content": [
                    {"type": "text", "text": "partial"},
                    {"type": "tool_use", "id": "tu_1", "name": "write_file", "input": {"path": "x"}}
                ],
                "usage": {"input_tokens": 100, "output_tokens": 4096}
            }),
        )
        .unwrap();
    assert_eq!(anth_resp.stop_reason, ModelStopReason::OutputLimit);
    assert!(anth_resp.tool_calls.is_empty());
    assert_eq!(anth_resp.usage.cache_read_tokens, 0);
}

#[test]
fn all_four_protocols_enforce_media_caps_and_case_insensitive_mime() {
    for protocol in ALL_PROTOCOLS {
        let adapter = adapter_for(protocol);
        let route = sample_route(protocol);
        let limits = adapter.media_limits();

        // Case-insensitive MIME accepted
        let mut req_ok = sample_request(false);
        req_ok.history.push(HistoryMessage::User {
            content: "look".into(),
            media: vec![MediaAttachment {
                mime_type: "IMAGE/PNG".into(),
                filename: "a.png".into(),
                bytes_base64: "AAAA".into(),
                sha256: "00".into(),
                raw_bytes_len: 128,
            }],
        });
        assert!(adapter.build_request(&route, &req_ok, false).is_ok());

        // Oversized attachment rejected per protocol limit (5 MiB Anthropic / 20 MiB OpenAI)
        let mut req_big = sample_request(false);
        req_big.history.push(HistoryMessage::User {
            content: "huge".into(),
            media: vec![MediaAttachment {
                mime_type: "image/png".into(),
                filename: "huge.png".into(),
                bytes_base64: "AA".into(),
                sha256: "00".into(),
                raw_bytes_len: limits.max_bytes_per_item + 1,
            }],
        });
        assert!(adapter.build_request(&route, &req_big, false).is_err());

        // >80 attachments rejected across ALL 4 protocols (including Responses!)
        let mut req_many = sample_request(false);
        req_many.history.push(HistoryMessage::User {
            content: "many".into(),
            media: (0..81)
                .map(|i| MediaAttachment {
                    mime_type: "image/png".into(),
                    filename: format!("{i}.png"),
                    bytes_base64: "AA".into(),
                    sha256: "00".into(),
                    raw_bytes_len: 10,
                })
                .collect(),
        });
        assert!(adapter.build_request(&route, &req_many, false).is_err());
    }
}

#[test]
fn continuation_roundtrip_and_streaming_reasoning_parity() {
    // Chat SSE supports both `reasoning` and `reasoning_content`
    let sse_stream = concat!(
        "data: {\"id\":\"c1\",\"model\":\"qwen3\",\"choices\":[{\"delta\":{\"reasoning\":\"step1 \"}}]}\n\n",
        "data: {\"id\":\"c1\",\"model\":\"qwen3\",\"choices\":[{\"delta\":{\"reasoning_content\":\"step2\"}}]}\n\n",
        "data: {\"id\":\"c1\",\"model\":\"qwen3\",\"choices\":[{\"delta\":{\"content\":\"done\"},\"finish_reason\":\"stop\"}]}\n\n",
        "data: {\"id\":\"c1\",\"choices\":[],\"usage\":{\"prompt_tokens\":100,\"completion_tokens\":20,\"prompt_tokens_details\":{\"cached_tokens\":80}}}\n\n",
        "data: [DONE]\n\n"
    );
    let mut cursor = Cursor::new(sse_stream.as_bytes());
    let mut deltas = Vec::new();
    let resp = adapter_for(ModelProtocol::OpenAiChat)
        .parse_sse_stream(&sample_route(ModelProtocol::OpenAiChat), &mut cursor, &mut |d| {
            deltas.push(d)
        })
        .unwrap();
    assert_eq!(resp.reasoning.as_deref(), Some("step1 step2"));
    assert_eq!(resp.content, "done");
    assert_eq!(resp.usage.cache_read_tokens, 80);
    assert!(deltas.iter().any(|d| matches!(d, ModelStreamDelta::Reasoning(_))));

    // Anthropic thinking+signature continuation round-trip
    let anth = adapter_for(ModelProtocol::AnthropicMessages);
    let route = sample_route(ModelProtocol::AnthropicMessages);
    let decoded = anth
        .decode_response(
            &route,
            &json!({
                "id": "msg_9",
                "model": "claude",
                "stop_reason": "end_turn",
                "content": [
                    {"type": "thinking", "thinking": "deep", "signature": "sig_abc"},
                    {"type": "text", "text": "answer"}
                ],
                "usage": {"input_tokens": 50, "output_tokens": 10}
            }),
        )
        .unwrap();
    let cont: ProviderContinuation = decoded.continuation.expect("anthropic continuation");
    let mut req = sample_request(false);
    req.history.push(HistoryMessage::Assistant {
        content: "answer".into(),
        reasoning: Some("deep".into()),
        tool_calls: vec![],
        continuation: Some(cont),
    });
    let built = anth.build_request(&route, &req, false).unwrap();
    let last_assistant = built.body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .rfind(|m| m["role"] == "assistant")
        .unwrap();
    assert_eq!(last_assistant["content"][0]["signature"], "sig_abc");

    // Privacy redaction bypasses LAN/localhost and redacts cloud
    assert!(is_local_or_lan_endpoint("http://127.0.0.1:8081"));
    assert!(is_local_or_lan_endpoint("http://192.168.1.50:8080"));
    assert!(!is_local_or_lan_endpoint("https://api.openai.com/v1"));
    let redacted = PrivacyRedactor::redact_text("token sk-1234567890abcdefghijklmno");
    assert_eq!(redacted, "token [REDACTED_API_KEY]");
}

#[test]
fn anthropic_wire_contract_reasoning_cache_breakpoint_empty_tool_result_and_usage() {
    let anth = adapter_for(ModelProtocol::AnthropicMessages);

    // 1. effort == "medium" -> output_config.effort = "medium" (NEVER thinking.budget_tokens!)
    let route_med = sample_route(ModelProtocol::AnthropicMessages);
    let mut req = sample_request(false);
    req.history.push(HistoryMessage::Tool {
        call_id: "c_empty".into(),
        tool_name: "bash".into(),
        payload: ToolModelPayload::text(""),
        is_error: false,
        media: vec![],
    });
    let built_med = anth.build_request(&route_med, &req, true).unwrap();
    assert_eq!(built_med.body["output_config"]["effort"], "medium");
    assert!(built_med.body.get("thinking").is_none());

    // Empty tool_result must be "" rather than synthetic "(empty)"
    let msgs = built_med.body["messages"].as_array().unwrap();
    let last_user_blocks = msgs.last().unwrap()["content"].as_array().unwrap();
    let empty_tr = last_user_blocks
        .iter()
        .find(|b| b.get("tool_use_id").and_then(|v| v.as_str()) == Some("c_empty"))
        .unwrap();
    assert_eq!(empty_tr["content"], "");
    // Message-level ephemeral cache breakpoint present on last user content block
    assert_eq!(
        last_user_blocks.last().unwrap()["cache_control"],
        json!({"type": "ephemeral"})
    );

    // 2. effort == "none" -> thinking.type = "disabled", NO output_config
    let mut route_none = sample_route(ModelProtocol::AnthropicMessages);
    route_none.reasoning_effort = Some("none".into());
    let built_none = anth.build_request(&route_none, &req, false).unwrap();
    assert_eq!(built_none.body["thinking"], json!({"type": "disabled"}));
    assert!(built_none.body.get("output_config").is_none());

    // 3. explicit thinking budget ("budget:4096") -> thinking.type = "enabled", budget_tokens = 4096
    let mut route_budget = sample_route(ModelProtocol::AnthropicMessages);
    route_budget.reasoning_effort = Some("budget:4096".into());
    let built_budget = anth.build_request(&route_budget, &req, false).unwrap();
    assert_eq!(
        built_budget.body["thinking"],
        json!({"type": "enabled", "budget_tokens": 4096})
    );
    assert!(built_budget.body.get("output_config").is_none());
    assert!(built_budget.body.get("temperature").is_none());

    // 4. usage_reported = true when usage object present
    let decoded = anth
        .decode_response(
            &route_med,
            &json!({
                "id": "msg_u",
                "model": "claude",
                "stop_reason": "end_turn",
                "content": [{"type": "text", "text": "ok"}],
                "usage": {
                    "input_tokens": 100,
                    "output_tokens": 20,
                    "cache_read_input_tokens": 800,
                    "cache_creation_input_tokens": 100
                }
            }),
        )
        .unwrap();
    assert!(decoded.usage.usage_reported);
    assert_eq!(decoded.usage.cache_hit_rate_bps_normalized(true), Some(8_000));
}

#[test]
fn openai_responses_chat_siwc_gemini_and_loopback_proxy_contracts() {
    let params = GenerationParams {
        seed: Some(123),
        stop: vec!["STOP".into()],
        top_k: Some(32),
        repetition_penalty: Some(1.05),
        thinking_budget: None,
    };

    // OpenAI Responses: reasoning.summary ONLY when supports_reasoning_summary && effort != "none"
    let resp_ad = adapter_for(ModelProtocol::OpenAiResponses);
    let mut r_no_sum = sample_route(ModelProtocol::OpenAiResponses);
    r_no_sum.capabilities.supports_reasoning_summary = false;
    let b_no_sum = resp_ad
        .build_request_with_params(&r_no_sum, &sample_request(false), &params, false)
        .unwrap();
    assert!(b_no_sum.body["reasoning"].get("summary").is_none());
    assert!(b_no_sum.body.get("seed").is_none());
    assert!(b_no_sum.body.get("stop").is_none());

    let mut r_with_sum = sample_route(ModelProtocol::OpenAiResponses);
    r_with_sum.capabilities.supports_reasoning_summary = true;
    let b_with_sum = resp_ad
        .build_request_with_params(&r_with_sum, &sample_request(false), &params, false)
        .unwrap();
    assert_eq!(b_with_sum.body["reasoning"]["summary"], "auto");

    r_with_sum.reasoning_effort = Some("none".into());
    let b_none = resp_ad
        .build_request_with_params(&r_with_sum, &sample_request(false), &params, false)
        .unwrap();
    assert!(b_none.body["reasoning"].get("summary").is_none());

    // OpenAI Responses reasoning_tokens + usage_reported
    let decoded_resp = resp_ad
        .decode_response(
            &r_no_sum,
            &json!({
                "id": "resp_1",
                "model": "gpt-5",
                "status": "completed",
                "output": [{"type": "message", "content": [{"type": "output_text", "text": "hi"}]}],
                "usage": {
                    "input_tokens": 200,
                    "output_tokens": 50,
                    "output_tokens_details": {"reasoning_tokens": 17}
                }
            }),
        )
        .unwrap();
    assert!(decoded_resp.usage.usage_reported);
    assert_eq!(decoded_resp.usage.reasoning_tokens, 17);

    // OpenAI Chat: seed/stop/top_k/repetition_penalty + completion_tokens_details.reasoning_tokens
    let chat_ad = adapter_for(ModelProtocol::OpenAiChat);
    let chat_route = sample_route(ModelProtocol::OpenAiChat);
    let b_chat = chat_ad
        .build_request_with_params(&chat_route, &sample_request(false), &params, false)
        .unwrap();
    assert_eq!(b_chat.body["seed"], 123);
    assert_eq!(b_chat.body["stop"], json!(["STOP"]));
    assert_eq!(b_chat.body["top_k"], 32);
    let decoded_chat = chat_ad
        .decode_response(
            &chat_route,
            &json!({
                "id": "chat_1",
                "model": "qwen3",
                "choices": [{"finish_reason": "stop", "message": {"role": "assistant", "content": "ok"}}],
                "usage": {
                    "prompt_tokens": 100,
                    "completion_tokens": 40,
                    "completion_tokens_details": {"reasoning_tokens": 12}
                }
            }),
        )
        .unwrap();
    assert!(decoded_chat.usage.usage_reported);
    assert_eq!(decoded_chat.usage.reasoning_tokens, 12);

    // SIWC: official endpoint, stream:true, store:false, strip forbidden params & x-nola-read-only, nola namespace
    let siwc_ad = adapter_for(ModelProtocol::SiwcResponses);
    let mut siwc_route = sample_route(ModelProtocol::SiwcResponses);
    siwc_route.endpoint = "https://api.openai.com/v1".into();
    let mut siwc_req = sample_request(false);
    siwc_req.prefix_stable_tools[0].parameters = json!({
        "type": "object",
        "x-nola-read-only": true,
        "properties": {"path": {"type": "string"}}
    });
    let b_siwc = siwc_ad
        .build_request_with_params(&siwc_route, &siwc_req, &params, false)
        .unwrap();
    assert_eq!(b_siwc.url, "https://api.openai.com/v1/responses");
    assert_eq!(b_siwc.body["stream"], true);
    assert_eq!(b_siwc.body["store"], false);
    assert!(b_siwc.body.get("max_output_tokens").is_none());
    assert!(b_siwc.body.get("temperature").is_none());
    assert!(b_siwc.body.get("top_p").is_none());
    assert!(b_siwc.body.get("seed").is_none());
    let ns_tool = &b_siwc.body["tools"][0];
    assert_eq!(ns_tool["type"], "namespace");
    assert_eq!(ns_tool["name"], "nola");
    assert!(ns_tool["tools"][0]["parameters"].get("x-nola-read-only").is_none());

    // Gemini decode + SSE
    let gem_ad = adapter_for(ModelProtocol::Gemini);
    let gem_route = sample_route(ModelProtocol::Gemini);
    let b_gem = gem_ad
        .build_request_with_params(&gem_route, &sample_request(false), &params, false)
        .unwrap();
    assert!(b_gem.url.ends_with("/v1beta/models/test-model:generateContent"));
    assert_eq!(b_gem.body["generationConfig"]["seed"], 123);
    assert_eq!(b_gem.body["generationConfig"]["stopSequences"], json!(["STOP"]));
    let decoded_gem = gem_ad
        .decode_response(
            &gem_route,
            &json!({
                "responseId": "gem_1",
                "modelVersion": "gemini-2.5-pro",
                "candidates": [{
                    "finishReason": "STOP",
                    "content": {
                        "role": "model",
                        "parts": [
                            {"text": "thinking...", "thought": true},
                            {"text": "hello world"}
                        ]
                    }
                }],
                "usageMetadata": {
                    "promptTokenCount": 120,
                    "candidatesTokenCount": 30,
                    "cachedContentTokenCount": 60,
                    "thoughtsTokenCount": 9
                }
            }),
        )
        .unwrap();
    assert_eq!(decoded_gem.content, "hello world");
    assert_eq!(decoded_gem.reasoning.as_deref(), Some("thinking..."));
    assert!(decoded_gem.usage.usage_reported);
    assert_eq!(decoded_gem.usage.reasoning_tokens, 9);
    assert_eq!(decoded_gem.usage.cache_read_tokens, 60);

    // Loopback proxy bypass helper
    assert!(is_loopback_url("http://127.0.0.1:8081/v1/chat/completions"));
    assert!(is_loopback_url("http://localhost:11434/v1"));
    assert!(is_loopback_url("http://[::1]:8080/v1"));
    assert!(!is_loopback_url("https://api.openai.com/v1/responses"));
    let client = LlmClient::new(15).unwrap();
    let _ = client.client_for_url("http://127.0.0.1:8081");
}

#[test]
fn forensic_batch1_wire_adapter_fixes() {
    // 1. SSE BOM stripping + SIWC namespace unwrapping in decode & SSE (including ToolCallStart) + strip stop
    let siwc = adapter_for(ModelProtocol::SiwcResponses);
    let siwc_route = sample_route(ModelProtocol::SiwcResponses);
    let params = GenerationParams {
        seed: Some(7),
        stop: vec!["END".into()],
        ..Default::default()
    };
    let built_siwc = siwc
        .build_request_with_params(&siwc_route, &sample_request(false), &params, true)
        .unwrap();
    assert!(built_siwc.body.get("stop").is_none());
    assert!(built_siwc.body.get("seed").is_none());

    let decoded_siwc = siwc
        .decode_response(
            &siwc_route,
            &json!({
                "id": "r1",
                "model": "gpt-5",
                "status": "completed",
                "output": [
                    {"type": "function_call", "call_id": "c1", "name": "nola.read_file", "arguments": "{}"},
                    {"type": "function_call", "call_id": "c2", "name": "nola/bash", "arguments": "{}"},
                    {"type": "function_call", "call_id": "c3", "name": "nola:grep", "arguments": "{}"}
                ]
            }),
        )
        .unwrap();
    let names: Vec<&str> = decoded_siwc.tool_calls.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, vec!["read_file", "bash", "grep"]);

    let bom_sse = concat!(
        "\u{feff}data: {\"type\":\"response.output_item.added\",\"item\":{\"type\":\"function_call\",\"call_id\":\"c1\",\"name\":\"nola.read_file\"}}\n\n",
        "data: {\"type\":\"response.function_call_arguments.delta\",\"call_id\":\"c1\",\"delta\":\"{\\\"path\\\":\\\"a.rs\\\"}\"}\n\n",
        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"r2\",\"model\":\"gpt-5\",\"status\":\"completed\",\"output\":[]}}\n\n"
    );
    let mut cursor = Cursor::new(bom_sse.as_bytes());
    let mut started_names = Vec::new();
    let sse_resp = siwc
        .parse_sse_stream(&siwc_route, &mut cursor, &mut |d| {
            if let ModelStreamDelta::ToolCallStart { name, .. } = d {
                started_names.push(name);
            }
        })
        .unwrap();
    assert_eq!(started_names, vec!["read_file"]);
    assert_eq!(sse_resp.tool_calls.len(), 1);
    assert_eq!(sse_resp.tool_calls[0].name, "read_file");

    // 2. Gemini schema sanitization, continuation thoughtSignature, id echo, non-object args, user-first
    let gem = adapter_for(ModelProtocol::Gemini);
    let gem_route = sample_route(ModelProtocol::Gemini);
    let mut gem_req = sample_request(false);
    gem_req.prefix_stable_tools[0].parameters = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "tool.json",
        "type": "object",
        "additionalProperties": false,
        "x-nola-read-only": true,
        "properties": {
            "path": {"type": "string", "default": "main.rs"}
        }
    });
    let gem_decoded = gem
        .decode_response(
            &gem_route,
            &json!({
                "responseId": "g1",
                "modelVersion": "gemini-2.5-pro",
                "candidates": [{
                    "finishReason": "STOP",
                    "content": {
                        "role": "model",
                        "parts": [
                            {"functionCall": {"id": "fc_99", "name": "read_file", "args": "not-an-object"}, "thoughtSignature": "sig_xyz"}
                        ]
                    }
                }]
            }),
        )
        .unwrap();
    assert_eq!(gem_decoded.tool_calls[0].call_id, "fc_99");
    assert_eq!(gem_decoded.tool_calls[0].arguments, json!({}));
    assert!(gem_decoded.continuation.is_some());

    gem_req.history = vec![
        HistoryMessage::Assistant {
            content: String::new(),
            reasoning: None,
            tool_calls: gem_decoded.tool_calls.clone(),
            continuation: gem_decoded.continuation.clone(),
        },
        HistoryMessage::Tool {
            call_id: "fc_99".into(),
            tool_name: "read_file".into(),
            payload: ToolModelPayload::text("ok"),
            is_error: false,
            media: vec![],
        },
    ];
    let gem_built = gem.build_request(&gem_route, &gem_req, false).unwrap();
    assert_eq!(gem_built.body["contents"][0]["role"], "user");
    assert_eq!(
        gem_built.body["contents"][1]["parts"][0]["thoughtSignature"],
        "sig_xyz"
    );
    assert_eq!(
        gem_built.body["contents"][2]["parts"][0]["functionResponse"]["id"],
        "fc_99"
    );
    let decl_params = &gem_built.body["tools"][0]["functionDeclarations"][0]["parameters"];
    assert!(decl_params.get("additionalProperties").is_none());
    assert!(decl_params.get("$schema").is_none());
    assert!(decl_params.get("$id").is_none());
    assert!(decl_params.get("x-nola-read-only").is_none());
    assert!(decl_params["properties"]["path"].get("default").is_none());

    // 3. Anthropic thinking budget >= 1024, max_tokens > budget_tokens, empty signature skipped, model mismatch rejected
    let anth = adapter_for(ModelProtocol::AnthropicMessages);
    let mut anth_route = sample_route(ModelProtocol::AnthropicMessages);
    anth_route.reasoning_effort = Some("budget:256".into());
    let mut anth_req = sample_request(false);
    anth_req.max_output_tokens = 1024;
    let anth_built = anth.build_request(&anth_route, &anth_req, false).unwrap();
    assert_eq!(anth_built.body["thinking"]["budget_tokens"], 1024);
    assert!(anth_built.body["max_tokens"].as_u64().unwrap() > 1024);

    let anth_dec = anth
        .decode_response(
            &anth_route,
            &json!({
                "id": "m1",
                "model": "claude-sonnet",
                "stop_reason": "end_turn",
                "content": [
                    {"type": "thinking", "thinking": "unsigned", "signature": ""},
                    {"type": "text", "text": "hello"}
                ]
            }),
        )
        .unwrap();
    let cont_items = anth_dec.continuation.unwrap().items;
    assert_eq!(cont_items.len(), 1);
    assert_eq!(cont_items[0]["type"], "text");

    // 4. OpenAI Responses buffers parallel Tool media + normalizes image/jpg -> image/jpeg
    let resp_ad = adapter_for(ModelProtocol::OpenAiResponses);
    let resp_route = sample_route(ModelProtocol::OpenAiResponses);
    let mut resp_req = sample_request(false);
    resp_req.history = vec![
        HistoryMessage::User { content: "hi".into(), media: vec![] },
        HistoryMessage::Tool {
            call_id: "c1".into(),
            tool_name: "read_file".into(),
            payload: ToolModelPayload::text("img1"),
            is_error: false,
            media: vec![MediaAttachment {
                mime_type: "image/jpg".into(),
                filename: "a.jpg".into(),
                bytes_base64: "AAAA".into(),
                sha256: "00".into(),
                raw_bytes_len: 3,
            }],
        },
        HistoryMessage::Tool {
            call_id: "c2".into(),
            tool_name: "read_file".into(),
            payload: ToolModelPayload::text("img2"),
            is_error: false,
            media: vec![],
        },
    ];
    let resp_built = resp_ad.build_request(&resp_route, &resp_req, false).unwrap();
    let inp = resp_built.body["input"].as_array().unwrap();
    assert_eq!(inp[1]["type"], "function_call_output");
    assert_eq!(inp[2]["type"], "function_call_output");
    assert_eq!(inp[3]["role"], "user");
    assert!(inp[3]["content"][1]["image_url"]
        .as_str()
        .unwrap()
        .starts_with("data:image/jpeg;base64,"));
}
