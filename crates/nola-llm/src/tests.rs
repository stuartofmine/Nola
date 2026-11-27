// SPDX-License-Identifier: Apache-2.0
//! Gate 3 & Gate 4: Parameterized 4-Protocol Wire Conformance Matrix.
//! Verifies all 4 protocols (`OpenAiChat`, `OpenAiResponses`, `AnthropicMessages`,
//! `SiwcResponses`) against every invariant that broke in v1.

use std::io::{BufRead, BufReader, Cursor, Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use nola_types::{
    FrozenSystemBlock, GenerationParams, HistoryMessage, MediaAttachment, ModelCapabilities,
    ModelProtocol, ModelRouteConfig, ModelStopReason, ModelStreamDelta, ModelToolCall,
    ProviderContinuation, RuntimeReminder, StructuredModelRequest, ToolModelPayload, ToolSchema,
};
use serde_json::json;

use crate::{
    adapter_for, is_local_or_lan_endpoint, is_loopback_url, normalized_wire_reasoning_effort,
    should_redact_pii, CallParams, LlmClient, LlmError, PrivacyRedactor, ReasoningProgressGuard,
    SseReader, MAX_SSE_LINE_BYTES,
};

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
        anthropic_cache_ttl_1h: false,
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

#[test]
fn deliverable1_full_wire_privacy_redaction_and_lan_endpoint_detection() {
    // 1. Local / LAN / CGNAT / ULA / mDNS / container bridge detection
    for local in [
        "http://localhost:11434/v1",
        "https://LOCALHOST/v1/chat/completions",
        "http://api.localhost:8080/v1",
        "http://127.0.0.1:8080/v1",
        "http://[::1]:8000/v1",
        "::1",
        "::",
        "http://0.0.0.0:11434",
        "http://user:pass@127.0.0.1:8080/v1",
        "http://192.168.5.2:18081/v1",
        "http://10.0.0.5:8000/v1",
        "http://172.16.1.10:11434/v1",
        "http://169.254.1.5:8080/v1",
        "http://100.64.0.1:8080/v1",
        "http://100.127.255.254:8080/v1",
        "http://mac-studio.local:8080/v1",
        "http://host.docker.internal:8080/v1",
        "http://gateway.docker.internal:8080/v1",
        "http://host.containers.internal:8080/v1",
        "http://[fd00::1]:8080/v1",
        "http://[fc00::1]:8080/v1",
        "http://[fe80::1]:8080/v1",
    ] {
        assert!(is_local_or_lan_endpoint(local), "expected local/LAN: {local}");
    }
    for remote in [
        "",
        "https://api.openai.com/v1",
        "https://api.anthropic.com/v1",
        "http://8.8.8.8:8080/v1",
        "http://172.32.0.1:8080/v1",
        "http://100.63.255.254:8080/v1",
        "http://100.128.0.1:8080/v1",
        "http://localhost.evil.com/v1",
        "http://127.0.0.1.evil.com/v1",
    ] {
        assert!(!is_local_or_lan_endpoint(remote), "expected remote: {remote}");
    }

    // 2. NOLA_REDACT_PII override & defaults
    let prev_env = std::env::var("NOLA_REDACT_PII").ok();
    std::env::remove_var("NOLA_REDACT_PII");
    assert!(!should_redact_pii("http://192.168.1.5:8080/v1"));
    assert!(should_redact_pii("https://api.openai.com/v1"));
    std::env::set_var("NOLA_REDACT_PII", "1");
    assert!(should_redact_pii("http://127.0.0.1:8080/v1"));
    std::env::set_var("NOLA_REDACT_PII", "0");
    assert!(!should_redact_pii("https://api.openai.com/v1"));
    match prev_env {
        Some(v) => std::env::set_var("NOLA_REDACT_PII", v),
        None => std::env::remove_var("NOLA_REDACT_PII"),
    }

    // 3. Credential redaction is ALWAYS active even on LAN/local while PII is preserved on LAN
    let redactor = PrivacyRedactor;
    let aws_prod = concat!("AKIA", "IOSFODNN7EXAMPLE");
    let aws_temp = concat!("ASIA", "1234567890ABCDEF");
    let google_key = concat!("AIza", "SyA12345678901234567890123456789012");
    let slack_key = concat!("xoxb-", "123456789012-abcdef");
    let gh_pat = concat!("github_pat_", "1234567890abcdef123456");

    let mut lan_val = json!({
        "note": format!(
            "email alice@example.com phone 13800138000 id 11010519990101123X \
             {aws_prod} {aws_temp} {google_key} {slack_key} {gh_pat} \
             https://u:p@git.example.com/r.git Bearer my-secret-bearer-token \
             Authorization: Basic dXNlcjpwYXNz live-route-secret-999"
        ),
        "private": "-----BEGIN RSA PRIVATE KEY-----\nabc\n-----END RSA PRIVATE KEY-----",
        "nested": {
            "accessToken": "opaque-secret",
            "Cookie": "sid=123",
            "embedded": r#"{"password":"hidden","email":"alice@example.com"}"#
        }
    });
    let lan_hits = redactor.redact_json_value_for_endpoint(
        &mut lan_val,
        "http://192.168.1.50:8080/v1",
        &["live-route-secret-999"],
    );
    assert!(lan_hits >= 10, "expected >=10 credential redactions, got {lan_hits}");
    let lan_note = lan_val["note"].as_str().unwrap();
    // PII preserved on LAN
    assert!(lan_note.contains("alice@example.com"), "{lan_note}");
    assert!(lan_note.contains("13800138000"), "{lan_note}");
    assert!(lan_note.contains("11010519990101123X"), "{lan_note}");
    // All credentials scrubbed even on LAN
    for secret in [
        aws_prod,
        aws_temp,
        google_key,
        slack_key,
        gh_pat,
        "u:p@",
        "my-secret-bearer-token",
        "dXNlcjpwYXNz",
        "live-route-secret-999",
        "opaque-secret",
        "sid=123",
        "hidden",
        "BEGIN RSA PRIVATE KEY",
    ] {
        assert!(!lan_val.to_string().contains(secret), "secret leaked on LAN: {secret}");
    }
    // Idempotency
    let second_hits = redactor.redact_json_value_for_endpoint(
        &mut lan_val,
        "http://192.168.1.50:8080/v1",
        &["live-route-secret-999"],
    );
    assert_eq!(second_hits, 0);

    // 4. Cloud endpoint redacts BOTH PII and credentials
    let mut cloud_val = json!({
        "text": "contact alice@example.com phone 13800138000 id 11010519990101123X",
        "phone_num": 13800138000u64,
        "a@example.com": 1,
        "b@example.com": 2
    });
    let cloud_hits =
        redactor.redact_json_value_for_endpoint(&mut cloud_val, "https://api.openai.com/v1", &[]);
    assert!(cloud_hits >= 6);
    assert_eq!(
        cloud_val["text"],
        "contact [EMAIL] phone [PHONE] id [ID]"
    );
    assert_eq!(cloud_val["phone_num"], "[PHONE]");
    assert_eq!(cloud_val["[EMAIL]"], 1);
    assert_eq!(cloud_val["[EMAIL] 2"], 2);

    // 5. Round-2 single-line LABELED_SECRET preserves non-secret schema/algorithm/env placeholders
    let yaml_and_shell = concat!(
        "card_token: sha256\n",
        "session_token: mask_secret\n",
        "pw_hash: bcrypt\n",
        "password: argon2\n",
        "passwd: pbkdf2\n",
        "api_key: hmac\n",
        "refresh_token: none\n",
        "access_token: false\n",
        "client_secret: true\n",
        "token: null\n",
        "api_key: os.environ[\"API_KEY\"]\n",
        "echo \"TOKEN=$DISPATCH_EVENT_API_TOKEN\"\n",
        "echo \"LEN=${#DISPATCH_EVENT_API_TOKEN}\"\n",
        "echo \"TPL={{API_TOKEN}}\"\n",
        "TOKEN=\n/usr/bin/python3\n",
        "Return only this exact ASCII token: NOLA_MAC_REAL_OK\n",
        "\"token\": \"json-secret-value\"\n",
        "access_token: LiveAccessSecret999\n",
        "real_token=abc123456789\n",
        "client_secret='my live secret'\n",
    );
    let mut req = sample_request(false);
    req.history.push(HistoryMessage::Tool {
        call_id: "t1".into(),
        tool_name: "read_file".into(),
        payload: ToolModelPayload::text(yaml_and_shell),
        is_error: false,
        media: vec![],
    });
    let sanitized = PrivacyRedactor::sanitize_request("http://127.0.0.1:8080/v1", &req);
    let HistoryMessage::Tool {
        payload: ToolModelPayload::Text(content),
        ..
    } = sanitized.history.last().unwrap()
    else {
        panic!("expected text tool payload");
    };
    for preserved in [
        "card_token: sha256",
        "session_token: mask_secret",
        "password: argon2",
        "passwd: pbkdf2",
        "api_key: hmac",
        "refresh_token: none",
        "access_token: false",
        "client_secret: true",
        "token: null",
        "api_key: os.environ[\"API_KEY\"]",
        "TOKEN=$DISPATCH_EVENT_API_TOKEN",
        "LEN=${#DISPATCH_EVENT_API_TOKEN}",
        "TPL={{API_TOKEN}}",
        "TOKEN=\n/usr/bin/python3\n",
        "Return only this exact ASCII token: NOLA_MAC_REAL_OK",
    ] {
        assert!(content.contains(preserved), "expected preserved `{preserved}` in:\n{content}");
    }
    assert!(content.contains("\"token\": \"[CREDENTIAL]\""), "{content}");
    assert!(content.contains("access_token: [CREDENTIAL]"), "{content}");
    assert!(content.contains("real_token=[CREDENTIAL]"), "{content}");
    assert!(content.contains("client_secret='[CREDENTIAL]'"), "{content}");
    assert!(!content.contains("json-secret-value"));
    assert!(!content.contains("LiveAccessSecret999"));
    assert!(!content.contains("abc123456789"));
    assert!(!content.contains("my live secret"));
}

#[test]
fn deliverable2_automatic_never_leaks_and_deepseek_thinking_control() {
    assert_eq!(normalized_wire_reasoning_effort(None), None);
    assert_eq!(normalized_wire_reasoning_effort(Some("")), None);
    assert_eq!(normalized_wire_reasoning_effort(Some("   ")), None);
    assert_eq!(normalized_wire_reasoning_effort(Some("auto")), None);
    assert_eq!(normalized_wire_reasoning_effort(Some("Automatic")), None);
    assert_eq!(normalized_wire_reasoning_effort(Some("DEFAULT")), None);
    assert_eq!(normalized_wire_reasoning_effort(Some("none")), Some("none"));
    assert_eq!(normalized_wire_reasoning_effort(Some("high")), Some("high"));

    // None of the 5 protocols may ever leak automatic/auto/default onto the wire
    for raw_effort in ["automatic", "AUTO", "default", ""] {
        for proto in ALL_PROTOCOLS {
            let mut route = sample_route(proto);
            route.reasoning_effort = Some(raw_effort.into());
            let built = adapter_for(proto)
                .build_request(&route, &sample_request(false), false)
                .unwrap();
            assert!(built.body.get("reasoning_effort").is_none(), "{proto:?} leaked");
            assert!(built.body.get("reasoning").is_none(), "{proto:?} leaked");
            assert!(built.body.get("thinking").is_none(), "{proto:?} leaked");
            assert!(built.body.get("output_config").is_none(), "{proto:?} leaked");
            if let Some(gen_cfg) = built.body.get("generationConfig") {
                assert!(gen_cfg.get("thinkingConfig").is_none(), "{proto:?} leaked");
            }
        }
    }

    // Standard OpenAI Chat: explicit "none" sends reasoning_effort="none"
    let chat = adapter_for(ModelProtocol::OpenAiChat);
    let mut chat_route = sample_route(ModelProtocol::OpenAiChat);
    chat_route.reasoning_effort = Some("none".into());
    let chat_none = chat
        .build_request(&chat_route, &sample_request(false), false)
        .unwrap();
    assert_eq!(chat_none.body["reasoning_effort"], "none");
    assert!(chat_none.body.get("thinking").is_none());

    // DeepSeek Chat: "none" -> thinking={"type":"disabled"}, "high" -> thinking={"type":"enabled"}, "automatic" -> omitted
    let mut ds_route = sample_route(ModelProtocol::OpenAiChat);
    ds_route.endpoint = "https://api.deepseek.com/v1".into();
    ds_route.model = "deepseek-r1".into();
    ds_route.reasoning_effort = Some("automatic".into());
    let ds_auto = chat
        .build_request(&ds_route, &sample_request(false), false)
        .unwrap();
    assert!(ds_auto.body.get("thinking").is_none());
    assert!(ds_auto.body.get("reasoning_effort").is_none());

    ds_route.reasoning_effort = Some("none".into());
    let ds_none = chat
        .build_request(&ds_route, &sample_request(false), false)
        .unwrap();
    assert_eq!(ds_none.body["thinking"], json!({"type": "disabled"}));
    assert!(ds_none.body.get("reasoning_effort").is_none());

    ds_route.reasoning_effort = Some("high".into());
    let ds_high = chat
        .build_request(&ds_route, &sample_request(false), false)
        .unwrap();
    assert_eq!(ds_high.body["thinking"], json!({"type": "enabled"}));
}

#[test]
fn deliverable3_reasoning_progress_guard_and_bounded_sse_reader() {
    assert_eq!(MAX_SSE_LINE_BYTES, 16 * 1024 * 1024);

    // ReasoningProgressGuard stops repeated reasoning loops across arbitrary chunk sizes
    for size in [1, 7, 128, 4096] {
        let mut guard = ReasoningProgressGuard::new();
        let repeated = "Need maybe call web_fetch. ".repeat(180);
        let tripped = repeated.as_bytes().chunks(size).any(|chunk| {
            let s = String::from_utf8(chunk.to_vec()).unwrap();
            matches!(
                guard.observe_delta(&ModelStreamDelta::Reasoning(s)),
                Err(LlmError::RepetitiveReasoning)
            )
        });
        assert!(tripped, "expected guard to trip for chunk size {size}");
    }

    // Accepts long distinct reasoning and ignores intentional repetition in final answer text
    let mut guard = ReasoningProgressGuard::new();
    for i in 0..500 {
        guard
            .observe_delta(&ModelStreamDelta::Reasoning(format!(
                "Inspecting candidate {i}, keeping independent evidence. "
            )))
            .unwrap();
    }
    guard
        .observe_delta(&ModelStreamDelta::Text("Repeated answer. ".repeat(500)))
        .unwrap();

    // Bounded SSE rejects lines exceeding max_line_bytes without unbounded allocation
    let huge_line = format!("data: {}\n\n", "x".repeat(200));
    let mut cursor = Cursor::new(huge_line.as_bytes());
    let mut reader = SseReader::new(&mut cursor).with_max_line_bytes(64);
    let err = reader.next_event().unwrap_err();
    assert!(matches!(err, LlmError::Wire(msg) if msg.contains("exceeded maximum")));

    // Cancellation stops SseReader between events
    let two_events = "data: {\"a\":1}\n\ndata: {\"a\":2}\n\n";
    let cancel = Arc::new(AtomicBool::new(false));
    let mut cursor2 = Cursor::new(two_events.as_bytes());
    let mut reader2 = SseReader::new(&mut cursor2).with_cancel(Some(cancel.clone()));
    let first = reader2.next_event().unwrap().unwrap();
    assert_eq!(first.data, "{\"a\":1}");
    cancel.store(true, Ordering::Relaxed);
    let err2 = reader2.next_event().unwrap_err();
    assert!(matches!(err2, LlmError::Cancelled));
}

fn read_http_request(stream: &mut std::net::TcpStream) {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut content_len = 0usize;
    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line).unwrap_or(0);
        if n == 0 || line == "\r\n" || line == "\n" {
            break;
        }
        let lower = line.to_ascii_lowercase();
        if let Some(rest) = lower.strip_prefix("content-length:") {
            content_len = rest.trim().parse().unwrap_or(0);
        }
    }
    if content_len > 0 {
        let mut body = vec![0u8; content_len];
        let _ = reader.read_exact(&mut body);
    }
}

fn spawn_mock_server(responses: Vec<String>) -> (String, Arc<AtomicUsize>, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let hits = Arc::new(AtomicUsize::new(0));
    let hits_bg = hits.clone();
    let handle = thread::spawn(move || {
        for resp in responses {
            let Ok((mut stream, _)) = listener.accept() else {
                break;
            };
            read_http_request(&mut stream);
            hits_bg.fetch_add(1, Ordering::SeqCst);
            let _ = stream.write_all(resp.as_bytes());
            let _ = stream.flush();
        }
    });
    (format!("http://{addr}"), hits, handle)
}

#[test]
fn deliverable3_and_4_repetitive_reasoning_resample_fallbacks_siwc_and_cancellation() {
    // 1. Transactional 1-shot REPETITIVE_REASONING resample when tools are present & no TextDelta emitted
    let mut repetitive_sse = String::new();
    for _ in 0..180 {
        repetitive_sse.push_str(
            "data: {\"id\":\"r1\",\"model\":\"m\",\"choices\":[{\"delta\":{\"reasoning_content\":\"Need maybe call web_fetch. \"}}]}\n\n",
        );
    }
    repetitive_sse.push_str("data: [DONE]\n\n");
    let clean_sse = concat!(
        "data: {\"id\":\"r2\",\"model\":\"m\",\"choices\":[{\"delta\":{\"content\":\"Recovered cleanly!\"},\"finish_reason\":\"stop\"}]}\n\n",
        "data: [DONE]\n\n"
    );
    let resp1 = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        repetitive_sse.len(),
        repetitive_sse
    );
    let resp2 = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        clean_sse.len(),
        clean_sse
    );
    let (ep, hits, handle) = spawn_mock_server(vec![resp1, resp2]);
    let mut route = sample_route(ModelProtocol::OpenAiChat);
    route.endpoint = ep;
    let client = LlmClient::new(15).unwrap();
    let resets = Arc::new(AtomicUsize::new(0));
    let resets_cb = resets.clone();
    let call_params = CallParams::default().with_stream_reset(move || {
        resets_cb.fetch_add(1, Ordering::SeqCst);
    });
    let out = client
        .complete_with_call_params(&route, &sample_request(false), &call_params, true, &mut |_| {})
        .unwrap();
    handle.join().unwrap();
    assert_eq!(out.content, "Recovered cleanly!");
    assert_eq!(hits.load(Ordering::SeqCst), 2);
    assert_eq!(resets.load(Ordering::SeqCst), 1);

    // 1b. No resample when visible TextDelta was already emitted OR when tools are empty
    let mut text_then_rep = String::from(
        "data: {\"id\":\"r3\",\"model\":\"m\",\"choices\":[{\"delta\":{\"content\":\"visible\"}}]}\n\n",
    );
    text_then_rep.push_str(&repetitive_sse);
    let resp_text_rep = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        text_then_rep.len(),
        text_then_rep
    );
    let (ep_nr, hits_nr, h_nr) = spawn_mock_server(vec![resp_text_rep]);
    let mut route_nr = sample_route(ModelProtocol::OpenAiChat);
    route_nr.endpoint = ep_nr;
    let err_nr = client
        .complete_with_call_params(&route_nr, &sample_request(false), &CallParams::default(), true, &mut |_| {})
        .unwrap_err();
    h_nr.join().unwrap();
    assert!(matches!(err_nr, LlmError::RepetitiveReasoning));
    assert_eq!(hits_nr.load(Ordering::SeqCst), 1);

    // 2. Ordered multi-endpoint fallback routing (primary 503 -> fallback1 429 -> fallback2 200)
    let err_503 = "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 4\r\nConnection: close\r\n\r\nbusy".to_string();
    let err_429 = "HTTP/1.1 429 Too Many Requests\r\nContent-Length: 4\r\nConnection: close\r\n\r\nrate".to_string();
    let ok_json = json!({
        "id": "chat_fb",
        "model": "fb-model",
        "choices": [{"finish_reason": "stop", "message": {"role": "assistant", "content": "from fallback 2"}}]
    })
    .to_string();
    let ok_200 = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        ok_json.len(),
        ok_json
    );
    let (ep_p, hits_p, hp) = spawn_mock_server(vec![err_503.clone()]);
    let (ep_f1, hits_f1, hf1) = spawn_mock_server(vec![err_429]);
    let (ep_f2, hits_f2, hf2) = spawn_mock_server(vec![ok_200]);

    let mut primary = sample_route(ModelProtocol::OpenAiChat);
    primary.endpoint = ep_p;
    let mut fb1 = sample_route(ModelProtocol::OpenAiChat);
    fb1.endpoint = ep_f1;
    let mut fb2 = sample_route(ModelProtocol::OpenAiChat);
    fb2.endpoint = ep_f2;

    let fast_client = LlmClient::new(15).unwrap().with_max_retries(0);
    let fb_resp = fast_client
        .complete_with_call_params(
            &primary,
            &sample_request(false),
            &CallParams::default().with_fallbacks(vec![fb1, fb2.clone()]),
            false,
            &mut |_| {},
        )
        .unwrap();
    hp.join().unwrap();
    hf1.join().unwrap();
    hf2.join().unwrap();
    assert_eq!(fb_resp.content, "from fallback 2");
    assert_eq!(hits_p.load(Ordering::SeqCst), 1);
    assert_eq!(hits_f1.load(Ordering::SeqCst), 1);
    assert_eq!(hits_f2.load(Ordering::SeqCst), 1);

    // 3. SIWC no-fallback invariant: SiwcResponses primary NEVER falls back
    let (ep_siwc, hits_siwc, h_siwc) = spawn_mock_server(vec![err_503]);
    let mut siwc_route = sample_route(ModelProtocol::SiwcResponses);
    siwc_route.endpoint = ep_siwc;
    let siwc_err = fast_client
        .complete_with_call_params(
            &siwc_route,
            &sample_request(false),
            &CallParams::default().with_fallbacks(vec![fb2]),
            true,
            &mut |_| {},
        )
        .unwrap_err();
    h_siwc.join().unwrap();
    assert!(matches!(siwc_err, LlmError::Status { status: 503, .. }));
    assert_eq!(hits_siwc.load(Ordering::SeqCst), 1);

    // 4. Interruptible cancellation during Retry-After backoff (< 500ms instead of 30s!)
    let err_retry_after = "HTTP/1.1 429 Too Many Requests\r\nRetry-After: 30\r\nContent-Length: 4\r\nConnection: close\r\n\r\nwait".to_string();
    let (ep_cancel, _hits_c, h_cancel) = spawn_mock_server(vec![err_retry_after]);
    let mut cancel_route = sample_route(ModelProtocol::OpenAiChat);
    cancel_route.endpoint = ep_cancel;
    let cancel_flag = Arc::new(AtomicBool::new(false));
    let cancel_bg = cancel_flag.clone();
    let trigger = thread::spawn(move || {
        thread::sleep(Duration::from_millis(80));
        cancel_bg.store(true, Ordering::Relaxed);
    });
    let started = Instant::now();
    let cancel_err = LlmClient::new(15)
        .unwrap()
        .with_max_retries(2)
        .complete_cancellable(
            &cancel_route,
            &sample_request(false),
            &GenerationParams::default(),
            Some(cancel_flag),
            false,
            &mut |_| {},
        )
        .unwrap_err();
    trigger.join().unwrap();
    h_cancel.join().unwrap();
    assert!(matches!(cancel_err, LlmError::Cancelled));
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "expected fast interruptible cancel, took {:?}",
        started.elapsed()
    );
}

#[test]
fn deliverable5_anthropic_ttl_1h_and_audio_media_support() {
    // 1. Anthropic ttl: "1h" cache control when configured via GenerationParams
    let anth = adapter_for(ModelProtocol::AnthropicMessages);
    let anth_route = sample_route(ModelProtocol::AnthropicMessages);
    let params_1h = GenerationParams {
        anthropic_cache_ttl_1h: true,
        ..Default::default()
    };
    let built_1h = anth
        .build_request_with_params(&anth_route, &sample_request(false), &params_1h, false)
        .unwrap();
    assert_eq!(
        built_1h.body["system"][0]["cache_control"],
        json!({"type": "ephemeral", "ttl": "1h"})
    );
    assert_eq!(
        built_1h.body["tools"].as_array().unwrap().last().unwrap()["cache_control"],
        json!({"type": "ephemeral", "ttl": "1h"})
    );
    let last_user_blocks = built_1h.body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .rfind(|m| m["role"] == "user")
        .unwrap()["content"]
        .as_array()
        .unwrap();
    assert_eq!(
        last_user_blocks.last().unwrap()["cache_control"],
        json!({"type": "ephemeral", "ttl": "1h"})
    );

    // Default (false) omits ttl
    let built_default = anth
        .build_request(&anth_route, &sample_request(false), false)
        .unwrap();
    assert_eq!(
        built_default.body["system"][0]["cache_control"],
        json!({"type": "ephemeral"})
    );

    // 2. Audio media support across all 5 protocols
    let audio_cases = [
        ("audio/wav", "wav"),
        ("audio/mp3", "mp3"),
        ("audio/mpeg", "mp3"),
        ("audio/ogg", "ogg"),
        ("audio/flac", "flac"),
        ("audio/m4a", "m4a"),
        ("audio/aac", "aac"),
    ];
    let chat = adapter_for(ModelProtocol::OpenAiChat);
    let gem = adapter_for(ModelProtocol::Gemini);
    let chat_route = sample_route(ModelProtocol::OpenAiChat);
    let gem_route = sample_route(ModelProtocol::Gemini);

    for (mime, expected_fmt) in audio_cases {
        let mut req = sample_request(false);
        req.history.push(HistoryMessage::User {
            content: "transcribe".into(),
            media: vec![MediaAttachment {
                mime_type: mime.into(),
                filename: format!("clip.{expected_fmt}"),
                bytes_base64: "UklGRg==".into(),
                sha256: "00".into(),
                raw_bytes_len: 4,
            }],
        });

        // OpenAI Chat encodes as input_audio {data, format}
        let chat_built = chat.build_request(&chat_route, &req, false).unwrap();
        let last_msg = chat_built.body["messages"].as_array().unwrap().last().unwrap();
        assert_eq!(
            last_msg["content"][1],
            json!({
                "type": "input_audio",
                "input_audio": { "data": "UklGRg==", "format": expected_fmt }
            })
        );

        // Gemini encodes as inlineData {mimeType, data}
        let gem_built = gem.build_request(&gem_route, &req, false).unwrap();
        let last_content = gem_built.body["contents"].as_array().unwrap().last().unwrap();
        assert_eq!(
            last_content["parts"].as_array().unwrap().last().unwrap(),
            &json!({
                "inlineData": { "mimeType": mime, "data": "UklGRg==" }
            })
        );
    }

    // Adapters without audio support reject gracefully with LlmError::Media
    let mut audio_req = sample_request(false);
    audio_req.history.push(HistoryMessage::User {
        content: "listen".into(),
        media: vec![MediaAttachment {
            mime_type: "audio/wav".into(),
            filename: "clip.wav".into(),
            bytes_base64: "UklGRg==".into(),
            sha256: "00".into(),
            raw_bytes_len: 4,
        }],
    });
    for unsupported_proto in [
        ModelProtocol::AnthropicMessages,
        ModelProtocol::OpenAiResponses,
        ModelProtocol::SiwcResponses,
    ] {
        let route = sample_route(unsupported_proto);
        let err = adapter_for(unsupported_proto)
            .build_request(&route, &audio_req, false)
            .unwrap_err();
        assert!(
            matches!(err, LlmError::Media(_)),
            "expected Media error for {unsupported_proto:?}, got {err:?}"
        );
    }
}
