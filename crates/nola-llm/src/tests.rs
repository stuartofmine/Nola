// SPDX-License-Identifier: Apache-2.0
//! Gate 3 & Gate 4: Parameterized 4-Protocol Wire Conformance Matrix.
//! Verifies all 4 protocols (`OpenAiChat`, `OpenAiResponses`, `AnthropicMessages`,
//! `SiwcResponses`) against every invariant that broke in v1.

use std::io::Cursor;

use nola_types::{
    FrozenSystemBlock, HistoryMessage, MediaAttachment, ModelCapabilities, ModelProtocol,
    ModelRouteConfig, ModelStopReason, ModelStreamDelta, ModelToolCall, ProviderContinuation,
    RuntimeReminder, StructuredModelRequest, ToolModelPayload, ToolSchema,
};
use serde_json::json;

use crate::{adapter_for, is_local_or_lan_endpoint, PrivacyRedactor};

const ALL_PROTOCOLS: [ModelProtocol; 4] = [
    ModelProtocol::OpenAiChat,
    ModelProtocol::OpenAiResponses,
    ModelProtocol::AnthropicMessages,
    ModelProtocol::SiwcResponses,
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
