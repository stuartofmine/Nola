// SPDX-License-Identifier: Apache-2.0
//! OpenAI Chat Completions wire adapter (`chat_completions`).
//!
//! Guarantees:
//! - Message 0 is the immutable frozen system prefix.
//! - Zero mid-conversation `system` or `developer` messages (tail reminders are `user`).
//! - Prior-turn reasoning is omitted from older turns; no retroactive `""` backfill.
//! - Supports both `delta.reasoning_content` and `delta.reasoning` (Qwen/DeepSeek/vLLM/Strata).
//! - Sends `stream_options: {"include_usage": true}` when streaming.
//! - Atomically discards truncated tool calls when `finish_reason == "length"`.

use std::collections::BTreeMap;
use std::io::BufRead;

use nola_types::{
    HistoryMessage, MediaAttachment, ModelProtocol, ModelRouteConfig, ModelStopReason,
    ModelStreamDelta, ModelToolCall, NormalizedModelResponse, StructuredModelRequest, TokenUsage,
};
use serde_json::{json, Value};

use crate::adapter::{
    collect_request_media, finalize_response, format_tail_reminders, join_frozen_system_prefix,
    parse_tool_arguments, BuiltHttpRequest, LlmError, ProtocolMediaLimits, ProviderWireAdapter,
};
use crate::sse::SseReader;

pub struct OpenAiChatAdapter;

fn chat_endpoint(base: &str) -> String {
    let trimmed = base.trim_end_matches('/');
    if trimmed.ends_with("/chat/completions") {
        trimmed.to_string()
    } else if trimmed.ends_with("/v1") {
        format!("{trimmed}/chat/completions")
    } else {
        format!("{trimmed}/v1/chat/completions")
    }
}

fn user_content_value(text: &str, media: &[MediaAttachment]) -> Value {
    if media.is_empty() {
        return Value::String(text.to_string());
    }
    let mut parts = Vec::new();
    if !text.is_empty() {
        parts.push(json!({"type": "text", "text": text}));
    }
    for att in media {
        let mime = att.mime_type.trim().to_ascii_lowercase();
        let data_url = format!("data:{mime};base64,{}", att.bytes_base64);
        if mime == "application/pdf" {
            parts.push(json!({
                "type": "file",
                "file": { "filename": att.filename, "file_data": data_url }
            }));
        } else {
            parts.push(json!({
                "type": "image_url",
                "image_url": { "url": data_url }
            }));
        }
    }
    Value::Array(parts)
}

impl ProviderWireAdapter for OpenAiChatAdapter {
    fn protocol(&self) -> ModelProtocol {
        ModelProtocol::OpenAiChat
    }

    fn media_limits(&self) -> ProtocolMediaLimits {
        ProtocolMediaLimits::OPENAI
    }

    fn build_request(
        &self,
        route: &ModelRouteConfig,
        req: &StructuredModelRequest,
        stream: bool,
    ) -> Result<BuiltHttpRequest, LlmError> {
        let all_media = collect_request_media(req);
        self.media_limits()
            .validate_all(&all_media, &route.capabilities)?;

        let mut messages: Vec<Value> = Vec::new();
        let system_text = join_frozen_system_prefix(req);
        if !system_text.is_empty() {
            messages.push(json!({
                "role": "system",
                "content": system_text
            }));
        }

        // Only assistant messages AFTER the last user turn keep `reasoning_content`
        // (avoids mutating historical turns or polluting KV prefixes across turns).
        let last_user_idx = req
            .history
            .iter()
            .rposition(|m| matches!(m, HistoryMessage::User { .. }));

        for (idx, msg) in req.history.iter().enumerate() {
            match msg {
                HistoryMessage::User { content, media } => {
                    messages.push(json!({
                        "role": "user",
                        "content": user_content_value(content, media),
                    }));
                }
                HistoryMessage::Assistant {
                    content,
                    reasoning,
                    tool_calls,
                    ..
                } => {
                    let mut obj = serde_json::Map::new();
                    obj.insert("role".into(), Value::String("assistant".into()));
                    obj.insert("content".into(), Value::String(content.clone()));
                    let in_active_turn = last_user_idx.map(|u| idx > u).unwrap_or(true);
                    if in_active_turn && route.capabilities.reasoning {
                        if let Some(r) = reasoning.as_deref().filter(|s| !s.is_empty()) {
                            obj.insert("reasoning_content".into(), Value::String(r.to_string()));
                        }
                    }
                    if !tool_calls.is_empty() {
                        let calls: Vec<Value> = tool_calls
                            .iter()
                            .map(|tc| {
                                json!({
                                    "id": tc.call_id,
                                    "type": "function",
                                    "function": {
                                        "name": tc.name,
                                        "arguments": serde_json::to_string(&tc.arguments).unwrap_or_else(|_| "{}".into())
                                    }
                                })
                            })
                            .collect();
                        obj.insert("tool_calls".into(), Value::Array(calls));
                    }
                    messages.push(Value::Object(obj));
                }
                HistoryMessage::Tool {
                    call_id,
                    tool_name,
                    payload,
                    media,
                    ..
                } => {
                    messages.push(json!({
                        "role": "tool",
                        "tool_call_id": call_id,
                        "name": tool_name,
                        "content": payload.to_model_string(),
                    }));
                    if !media.is_empty() {
                        messages.push(json!({
                            "role": "user",
                            "content": user_content_value("Attached media from tool output:", media),
                        }));
                    }
                }
            }
        }

        if let Some(tail) = format_tail_reminders(&req.ephemeral_tail_reminders) {
            messages.push(json!({
                "role": "user",
                "content": tail
            }));
        }

        let mut body = serde_json::Map::new();
        body.insert("model".into(), Value::String(route.model.clone()));
        body.insert("messages".into(), Value::Array(messages));
        body.insert("max_tokens".into(), json!(req.max_output_tokens));
        body.insert("stream".into(), Value::Bool(stream));
        if stream {
            body.insert("stream_options".into(), json!({"include_usage": true}));
        }
        if route.capabilities.sampling_parameters {
            if let Some(t) = route.temperature {
                body.insert("temperature".into(), json!(t));
            }
            if let Some(tp) = route.top_p {
                body.insert("top_p".into(), json!(tp));
            }
        }
        if let Some(effort) = route.reasoning_effort.as_deref().filter(|s| !s.is_empty()) {
            if route.capabilities.reasoning {
                body.insert("reasoning_effort".into(), Value::String(effort.to_string()));
            }
        }
        if let Some(cache_key) = route.prompt_cache_key.as_deref().filter(|s| !s.is_empty()) {
            body.insert("prompt_cache_key".into(), Value::String(cache_key.to_string()));
        }
        if route.capabilities.tool_calls && !req.prefix_stable_tools.is_empty() {
            let tools: Vec<Value> = req
                .prefix_stable_tools
                .iter()
                .map(|t| {
                    json!({
                        "type": "function",
                        "function": {
                            "name": t.name,
                            "description": t.description,
                            "parameters": t.parameters,
                        }
                    })
                })
                .collect();
            body.insert("tools".into(), Value::Array(tools));
        }
        if let Some(schema) = &req.json_schema {
            body.insert(
                "response_format".into(),
                json!({
                    "type": "json_schema",
                    "json_schema": {
                        "name": "nola_structured_output",
                        "strict": true,
                        "schema": schema
                    }
                }),
            );
        }

        let mut headers = BTreeMap::new();
        headers.insert("content-type".into(), "application/json".into());
        if !route.api_key.trim().is_empty() {
            headers.insert(
                "authorization".into(),
                format!("Bearer {}", route.api_key.trim()),
            );
        }

        Ok(BuiltHttpRequest {
            url: chat_endpoint(&route.endpoint),
            headers,
            body: Value::Object(body),
        })
    }

    fn decode_response(
        &self,
        _route: &ModelRouteConfig,
        body: &Value,
    ) -> Result<NormalizedModelResponse, LlmError> {
        let choice = body
            .get("choices")
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.first())
            .ok_or_else(|| LlmError::Wire("chat completion response missing choices[0]".into()))?;
        let msg = choice
            .get("message")
            .ok_or_else(|| LlmError::Wire("chat completion choice missing message".into()))?;

        let content = msg
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let reasoning = msg
            .get("reasoning_content")
            .or_else(|| msg.get("reasoning"))
            .and_then(|v| v.as_str())
            .map(ToString::to_string);

        let finish = choice
            .get("finish_reason")
            .and_then(|v| v.as_str())
            .unwrap_or("stop");
        let stop_reason = map_finish_reason(finish);

        let mut tool_calls = Vec::new();
        if stop_reason != ModelStopReason::OutputLimit {
            if let Some(arr) = msg.get("tool_calls").and_then(|v| v.as_array()) {
                for (i, tc) in arr.iter().enumerate() {
                    let call_id = tc
                        .get("id")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string)
                        .unwrap_or_else(|| format!("call_{i}"));
                    let func = tc.get("function").unwrap_or(&Value::Null);
                    let name = func
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let raw_args = func
                        .get("arguments")
                        .and_then(|v| v.as_str())
                        .unwrap_or("{}");
                    if !name.is_empty() {
                        tool_calls.push(ModelToolCall {
                            call_id,
                            name,
                            arguments: parse_tool_arguments(raw_args)?,
                        });
                    }
                }
            }
        }

        let usage = parse_openai_usage(body.get("usage"));
        Ok(finalize_response(
            body.get("id").and_then(|v| v.as_str()).map(ToString::to_string),
            body.get("model")
                .and_then(|v| v.as_str())
                .map(ToString::to_string),
            content,
            reasoning,
            tool_calls,
            stop_reason,
            usage,
            None,
        ))
    }

    fn parse_sse_stream(
        &self,
        _route: &ModelRouteConfig,
        reader: &mut dyn BufRead,
        on_delta: &mut dyn FnMut(ModelStreamDelta),
    ) -> Result<NormalizedModelResponse, LlmError> {
        let mut sse = SseReader::new(reader);
        let mut response_id: Option<String> = None;
        let mut response_model: Option<String> = None;
        let mut content = String::new();
        let mut reasoning = String::new();
        let mut stop_reason = ModelStopReason::Stop;
        let mut usage = TokenUsage::default();
        let mut partial_calls: BTreeMap<usize, (String, String, String)> = BTreeMap::new();

        while let Some(ev) = sse.next_event()? {
            let trimmed = ev.data.trim();
            if trimmed.is_empty() || trimmed == "[DONE]" {
                continue;
            }
            let chunk: Value = serde_json::from_str(trimmed)?;
            if response_id.is_none() {
                response_id = chunk
                    .get("id")
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string);
            }
            if response_model.is_none() {
                response_model = chunk
                    .get("model")
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string);
            }
            if chunk.get("usage").is_some() {
                usage = parse_openai_usage(chunk.get("usage"));
            }
            let Some(choice) = chunk
                .get("choices")
                .and_then(|v| v.as_array())
                .and_then(|a| a.first())
            else {
                continue;
            };
            if let Some(fr) = choice.get("finish_reason").and_then(|v| v.as_str()) {
                stop_reason = map_finish_reason(fr);
            }
            let Some(delta) = choice.get("delta") else {
                continue;
            };
            if let Some(text) = delta.get("content").and_then(|v| v.as_str()) {
                if !text.is_empty() {
                    content.push_str(text);
                    on_delta(ModelStreamDelta::Text(text.to_string()));
                }
            }
            if let Some(r) = delta
                .get("reasoning_content")
                .or_else(|| delta.get("reasoning"))
                .and_then(|v| v.as_str())
            {
                if !r.is_empty() {
                    reasoning.push_str(r);
                    on_delta(ModelStreamDelta::Reasoning(r.to_string()));
                }
            }
            if let Some(calls) = delta.get("tool_calls").and_then(|v| v.as_array()) {
                for (pos, tc) in calls.iter().enumerate() {
                    let idx = tc
                        .get("index")
                        .and_then(|v| v.as_u64())
                        .map(|n| n as usize)
                        .unwrap_or(pos);
                    let entry = partial_calls
                        .entry(idx)
                        .or_insert_with(|| (format!("call_{idx}"), String::new(), String::new()));
                    if let Some(id) = tc.get("id").and_then(|v| v.as_str()) {
                        if !id.is_empty() {
                            entry.0 = id.to_string();
                        }
                    }
                    if let Some(func) = tc.get("function") {
                        if let Some(name) = func.get("name").and_then(|v| v.as_str()) {
                            if !name.is_empty() {
                                let was_empty = entry.1.is_empty();
                                entry.1.push_str(name);
                                if was_empty {
                                    on_delta(ModelStreamDelta::ToolCallStart {
                                        call_id: entry.0.clone(),
                                        name: entry.1.clone(),
                                    });
                                }
                            }
                        }
                        if let Some(args_delta) = func.get("arguments").and_then(|v| v.as_str()) {
                            if !args_delta.is_empty() {
                                entry.2.push_str(args_delta);
                                on_delta(ModelStreamDelta::ToolCallArgsDelta {
                                    call_id: entry.0.clone(),
                                    delta: args_delta.to_string(),
                                });
                            }
                        }
                    }
                }
            }
        }

        let mut tool_calls = Vec::new();
        if stop_reason != ModelStopReason::OutputLimit {
            for (_, (call_id, name, raw_args)) in partial_calls {
                if !name.is_empty() {
                    tool_calls.push(ModelToolCall {
                        call_id,
                        name,
                        arguments: parse_tool_arguments(&raw_args)?,
                    });
                }
            }
        }

        Ok(finalize_response(
            response_id,
            response_model,
            content,
            if reasoning.is_empty() {
                None
            } else {
                Some(reasoning)
            },
            tool_calls,
            stop_reason,
            usage,
            None,
        ))
    }
}

fn map_finish_reason(raw: &str) -> ModelStopReason {
    match raw {
        "length" => ModelStopReason::OutputLimit,
        "tool_calls" | "function_call" => ModelStopReason::ToolCalls,
        "content_filter" => ModelStopReason::ContentFilter,
        _ => ModelStopReason::Stop,
    }
}

pub(crate) fn parse_openai_usage(v: Option<&Value>) -> TokenUsage {
    let Some(u) = v else {
        return TokenUsage::default();
    };
    let input_tokens = u
        .get("prompt_tokens")
        .or_else(|| u.get("input_tokens"))
        .and_then(|x| x.as_u64())
        .unwrap_or(0);
    let output_tokens = u
        .get("completion_tokens")
        .or_else(|| u.get("output_tokens"))
        .and_then(|x| x.as_u64())
        .unwrap_or(0);
    let cache_read_tokens = u
        .get("prompt_tokens_details")
        .or_else(|| u.get("input_tokens_details"))
        .and_then(|d| d.get("cached_tokens"))
        .and_then(|x| x.as_u64())
        .unwrap_or(0);
    TokenUsage {
        input_tokens,
        output_tokens,
        cache_read_tokens,
        cache_write_tokens: 0,
    }
}
