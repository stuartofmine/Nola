// SPDX-License-Identifier: Apache-2.0
//! Anthropic Messages wire adapter (`anthropic`).
//!
//! Guarantees:
//! - `body["system"]` contains ONLY `frozen_system_prefix` with ephemeral cache breakpoint.
//! - `ephemeral_tail_reminders` are NEVER hoisted into `body["system"]`!
//! - Consecutive `user` turns (including warm-compaction forks + tool_results + tail reminders)
//!   are ALWAYS merged into one valid `role: "user"` block array.
//! - Full `thinking` + `signature` / `redacted_thinking` continuation round-trip.
//! - 5 MiB per-item & 80-item media limits; optional cache usage tokens default to 0.
//! - `stop_reason == "max_tokens"` atomically discards truncated `tool_use` calls.

use std::collections::BTreeMap;
use std::io::BufRead;

use nola_types::{
    GenerationParams, HistoryMessage, MediaAttachment, ModelProtocol, ModelRouteConfig,
    ModelStopReason, ModelStreamDelta, ModelToolCall, NormalizedModelResponse,
    ProviderContinuation, StructuredModelRequest, TokenUsage,
};
use serde_json::{json, Value};

use crate::adapter::{
    collect_request_media, finalize_response, format_tail_reminders, parse_tool_arguments,
    BuiltHttpRequest, LlmError, ProtocolMediaLimits, ProviderWireAdapter,
};
use crate::sse::SseReader;

pub struct AnthropicMessagesAdapter;

fn anthropic_endpoint(base: &str) -> String {
    let trimmed = base.trim_end_matches('/');
    if trimmed.ends_with("/v1/messages") || trimmed.ends_with("/messages") {
        trimmed.to_string()
    } else if trimmed.ends_with("/v1") {
        format!("{trimmed}/messages")
    } else {
        format!("{trimmed}/v1/messages")
    }
}

fn push_or_merge_role(messages: &mut Vec<Value>, role: &str, mut new_blocks: Vec<Value>) {
    if new_blocks.is_empty() {
        return;
    }
    if let Some(last) = messages.last_mut() {
        if last.get("role").and_then(|v| v.as_str()) == Some(role) {
            if let Some(arr) = last.get_mut("content").and_then(|v| v.as_array_mut()) {
                arr.append(&mut new_blocks);
                return;
            }
        }
    }
    messages.push(json!({
        "role": role,
        "content": new_blocks
    }));
}

fn push_or_merge_user(messages: &mut Vec<Value>, new_blocks: Vec<Value>) {
    push_or_merge_role(messages, "user", new_blocks);
}

fn media_blocks(media: &[MediaAttachment]) -> Vec<Value> {
    media
        .iter()
        .map(|att| {
            let raw_mime = att.mime_type.trim().to_ascii_lowercase();
            let mime = if raw_mime == "image/jpg" {
                "image/jpeg"
            } else {
                raw_mime.as_str()
            };
            let block_type = if mime == "application/pdf" {
                "document"
            } else {
                "image"
            };
            json!({
                "type": block_type,
                "source": {
                    "type": "base64",
                    "media_type": mime,
                    "data": att.bytes_base64
                }
            })
        })
        .collect()
}

impl ProviderWireAdapter for AnthropicMessagesAdapter {
    fn protocol(&self) -> ModelProtocol {
        ModelProtocol::AnthropicMessages
    }

    fn media_limits(&self) -> ProtocolMediaLimits {
        ProtocolMediaLimits::ANTHROPIC
    }

    fn build_request(
        &self,
        route: &ModelRouteConfig,
        req: &StructuredModelRequest,
        stream: bool,
    ) -> Result<BuiltHttpRequest, LlmError> {
        self.build_request_with_params(route, req, &GenerationParams::default(), stream)
    }

    fn build_request_with_params(
        &self,
        route: &ModelRouteConfig,
        req: &StructuredModelRequest,
        params: &GenerationParams,
        stream: bool,
    ) -> Result<BuiltHttpRequest, LlmError> {
        let all_media = collect_request_media(req);
        self.media_limits()
            .validate_all(&all_media, &route.capabilities)?;

        let mut system_blocks: Vec<Value> = req
            .frozen_system_prefix
            .iter()
            .filter(|b| !b.text.trim().is_empty())
            .map(|b| json!({"type": "text", "text": b.text}))
            .collect();
        if let Some(last) = system_blocks.last_mut().and_then(|v| v.as_object_mut()) {
            last.insert("cache_control".into(), json!({"type": "ephemeral"}));
        }

        let mut messages: Vec<Value> = Vec::new();
        for msg in &req.history {
            match msg {
                HistoryMessage::User { content, media } => {
                    let mut blocks = Vec::new();
                    if !content.is_empty() {
                        blocks.push(json!({"type": "text", "text": content}));
                    }
                    blocks.extend(media_blocks(media));
                    push_or_merge_user(&mut messages, blocks);
                }
                HistoryMessage::Assistant {
                    content,
                    tool_calls,
                    continuation,
                    ..
                } => {
                    let blocks = if let Some(cont) = continuation
                        .as_ref()
                        .filter(|c| c.protocol == ModelProtocol::AnthropicMessages && !c.items.is_empty())
                    {
                        cont.items.clone()
                    } else {
                        let mut b = Vec::new();
                        if !content.is_empty() {
                            b.push(json!({"type": "text", "text": content}));
                        }
                        for tc in tool_calls {
                            b.push(json!({
                                "type": "tool_use",
                                "id": tc.call_id,
                                "name": tc.name,
                                "input": tc.arguments
                            }));
                        }
                        b
                    };
                    if !blocks.is_empty() {
                        push_or_merge_role(&mut messages, "assistant", blocks);
                    }
                }
                HistoryMessage::Tool {
                    call_id,
                    payload,
                    is_error,
                    media,
                    ..
                } => {
                    let mut tr_content = Vec::new();
                    let text = payload.to_model_string();
                    if !text.is_empty() {
                        tr_content.push(json!({"type": "text", "text": text}));
                    }
                    tr_content.extend(media_blocks(media));
                    let content_val = if tr_content.is_empty() {
                        Value::String(String::new())
                    } else {
                        Value::Array(tr_content)
                    };
                    push_or_merge_user(
                        &mut messages,
                        vec![json!({
                            "type": "tool_result",
                            "tool_use_id": call_id,
                            "is_error": is_error,
                            "content": content_val
                        })],
                    );
                }
            }
        }

        // Ephemeral tail reminders stay strictly in the trailing `user` message (merged cleanly).
        if let Some(tail) = format_tail_reminders(&req.ephemeral_tail_reminders) {
            push_or_merge_user(&mut messages, vec![json!({"type": "text", "text": tail})]);
        }
        if let Some(schema) = &req.json_schema {
            push_or_merge_user(
                &mut messages,
                vec![json!({
                    "type": "text",
                    "text": format!("Return ONLY valid JSON conforming to this JSON Schema:\n{}", serde_json::to_string(schema).unwrap_or_default())
                })],
            );
        }
        // Anthropic requires non-empty `messages` starting with a `user` turn.
        if messages.is_empty()
            || messages.first().and_then(|m| m.get("role")).and_then(|v| v.as_str()) != Some("user")
        {
            messages.insert(
                0,
                json!({"role": "user", "content": [{"type": "text", "text": "Continue."}]}),
            );
        }

        // Attach a message-level ephemeral cache breakpoint on the last cacheable
        // user/tool_result content block (in addition to system & tools breakpoints).
        if let Some(last_user) = messages
            .iter_mut()
            .rev()
            .find(|m| m.get("role").and_then(|v| v.as_str()) == Some("user"))
        {
            if let Some(blocks) = last_user.get_mut("content").and_then(|v| v.as_array_mut()) {
                if let Some(block) = blocks.iter_mut().rev().find(|b| {
                    matches!(
                        b.get("type").and_then(|t| t.as_str()),
                        Some("text" | "tool_result" | "image" | "document")
                    )
                }) {
                    if let Some(obj) = block.as_object_mut() {
                        obj.insert("cache_control".into(), json!({"type": "ephemeral"}));
                    }
                }
            }
        }

        let mut body = serde_json::Map::new();
        body.insert("model".into(), Value::String(route.model.clone()));
        body.insert("max_tokens".into(), json!(req.max_output_tokens));
        body.insert("stream".into(), Value::Bool(stream));
        if !system_blocks.is_empty() {
            body.insert("system".into(), Value::Array(system_blocks));
        }
        body.insert("messages".into(), Value::Array(messages));

        // Canonical Anthropic reasoning wire contract:
        // - effort == "none" -> thinking: {"type": "disabled"} (no output_config)
        // - explicit thinking budget (params.thinking_budget or "budget:<N>") -> thinking: {"type": "enabled", "budget_tokens": N}
        // - effort in low|medium|high|max -> output_config: {"effort": effort} (NEVER converted to budget_tokens)
        let effort_norm = route
            .reasoning_effort
            .as_deref()
            .map(|s| s.trim().to_ascii_lowercase())
            .filter(|s| !s.is_empty());
        let explicit_budget = params.thinking_budget.or_else(|| {
            effort_norm
                .as_deref()
                .and_then(|e| e.strip_prefix("budget:"))
                .and_then(|n| n.trim().parse::<usize>().ok())
        });

        let mut explicit_thinking_enabled = false;
        if effort_norm.as_deref() == Some("none") {
            body.insert("thinking".into(), json!({"type": "disabled"}));
        } else if let Some(budget) = explicit_budget {
            if route.capabilities.reasoning && budget > 0 {
                body.insert(
                    "thinking".into(),
                    json!({"type": "enabled", "budget_tokens": budget}),
                );
                explicit_thinking_enabled = true;
            }
        } else if let Some(effort) = effort_norm.as_deref() {
            if route.capabilities.reasoning
                && matches!(effort, "low" | "medium" | "high" | "max")
            {
                body.insert("output_config".into(), json!({"effort": effort}));
            }
        }

        if !explicit_thinking_enabled && route.capabilities.sampling_parameters {
            // Claude 4.x rejects requests specifying both temperature and top_p simultaneously.
            if let Some(t) = route.temperature {
                body.insert("temperature".into(), json!(t));
            } else if let Some(tp) = route.top_p {
                body.insert("top_p".into(), json!(tp));
            }
            if let Some(tk) = params.top_k {
                body.insert("top_k".into(), json!(tk));
            }
        }
        if !params.stop.is_empty() {
            body.insert("stop_sequences".into(), json!(params.stop));
        }
        if route.capabilities.tool_calls && !req.prefix_stable_tools.is_empty() {
            let mut tools: Vec<Value> = req
                .prefix_stable_tools
                .iter()
                .map(|t| {
                    json!({
                        "name": t.name,
                        "description": t.description,
                        "input_schema": t.parameters
                    })
                })
                .collect();
            if let Some(last) = tools.last_mut().and_then(|v| v.as_object_mut()) {
                last.insert("cache_control".into(), json!({"type": "ephemeral"}));
            }
            body.insert("tools".into(), Value::Array(tools));
        }

        let mut headers = BTreeMap::new();
        headers.insert("content-type".into(), "application/json".into());
        headers.insert("anthropic-version".into(), "2023-06-01".into());
        if !route.api_key.trim().is_empty() {
            headers.insert("x-api-key".into(), route.api_key.trim().to_string());
        }

        Ok(BuiltHttpRequest {
            url: anthropic_endpoint(&route.endpoint),
            headers,
            body: Value::Object(body),
        })
    }

    fn decode_response(
        &self,
        route: &ModelRouteConfig,
        body: &Value,
    ) -> Result<NormalizedModelResponse, LlmError> {
        let response_id = body.get("id").and_then(|v| v.as_str()).map(ToString::to_string);
        let response_model = body
            .get("model")
            .and_then(|v| v.as_str())
            .map(ToString::to_string);

        let stop_reason = match body.get("stop_reason").and_then(|v| v.as_str()).unwrap_or("") {
            "max_tokens" => ModelStopReason::OutputLimit,
            "tool_use" => ModelStopReason::ToolCalls,
            "refusal" => ModelStopReason::ContentFilter,
            _ => ModelStopReason::Stop,
        };

        let blocks = body
            .get("content")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let mut content = String::new();
        let mut reasoning = String::new();
        let mut tool_calls = Vec::new();
        let mut cont_blocks = Vec::new();

        for block in &blocks {
            cont_blocks.push(block.clone());
            match block.get("type").and_then(|v| v.as_str()).unwrap_or("") {
                "text" => {
                    if let Some(t) = block.get("text").and_then(|v| v.as_str()) {
                        content.push_str(t);
                    }
                }
                "thinking" => {
                    if let Some(t) = block.get("thinking").and_then(|v| v.as_str()) {
                        reasoning.push_str(t);
                    }
                }
                "tool_use" if stop_reason != ModelStopReason::OutputLimit => {
                    let call_id = block
                        .get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("toolu_0")
                        .to_string();
                    let name = block
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let input = block
                        .get("input")
                        .cloned()
                        .unwrap_or_else(|| json!({}));
                    if !name.is_empty() {
                        tool_calls.push(ModelToolCall {
                            call_id,
                            name,
                            arguments: input,
                        });
                    }
                }
                _ => {}
            }
        }

        let continuation = if cont_blocks.is_empty() || stop_reason == ModelStopReason::OutputLimit {
            None
        } else {
            Some(ProviderContinuation {
                protocol: ModelProtocol::AnthropicMessages,
                model: response_model
                    .clone()
                    .unwrap_or_else(|| route.model.clone()),
                response_id: response_id.clone(),
                items: cont_blocks,
            })
        };

        let usage = parse_anthropic_usage(body.get("usage"));
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
            continuation,
        ))
    }

    fn parse_sse_stream(
        &self,
        route: &ModelRouteConfig,
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
        // index -> (kind, id_or_sig, name, buffer)
        let mut blocks: BTreeMap<usize, (String, String, String, String)> = BTreeMap::new();
        let mut saw_terminal = false;

        while let Some(ev) = sse.next_event()? {
            let trimmed = ev.data.trim();
            if trimmed.is_empty() {
                continue;
            }
            let payload: Value = serde_json::from_str(trimmed)?;
            let kind = payload
                .get("type")
                .and_then(|v| v.as_str())
                .or(ev.event.as_deref())
                .unwrap_or("");
            match kind {
                "error" => {
                    return Err(LlmError::Wire(format!("anthropic SSE error: {payload}")));
                }
                "message_stop" => {
                    saw_terminal = true;
                }
                "message_start" => {
                    if let Some(m) = payload.get("message") {
                        response_id = m.get("id").and_then(|v| v.as_str()).map(ToString::to_string);
                        response_model = m
                            .get("model")
                            .and_then(|v| v.as_str())
                            .map(ToString::to_string);
                        let u = parse_anthropic_usage(m.get("usage"));
                        usage.input_tokens = u.input_tokens;
                        usage.cache_read_tokens = u.cache_read_tokens;
                        usage.cache_write_tokens = u.cache_write_tokens;
                        if u.usage_reported {
                            usage.usage_reported = true;
                        }
                    }
                }
                "content_block_start" => {
                    let idx = payload.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                    if let Some(cb) = payload.get("content_block") {
                        let btype = cb.get("type").and_then(|v| v.as_str()).unwrap_or("").to_string();
                        let id = cb.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                        let name = cb.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                        let init_data = cb.get("data").and_then(|v| v.as_str()).unwrap_or("").to_string();
                        if btype == "tool_use" && !name.is_empty() {
                            on_delta(ModelStreamDelta::ToolCallStart {
                                call_id: id.clone(),
                                name: name.clone(),
                            });
                        }
                        blocks.insert(idx, (btype, id, name, init_data));
                    }
                }
                "content_block_delta" => {
                    let idx = payload.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                    if let Some(delta) = payload.get("delta") {
                        let dtype = delta.get("type").and_then(|v| v.as_str()).unwrap_or("");
                        match dtype {
                            "text_delta" => {
                                if let Some(t) = delta.get("text").and_then(|v| v.as_str()) {
                                    content.push_str(t);
                                    if let Some(b) = blocks.get_mut(&idx) {
                                        b.3.push_str(t);
                                    }
                                    on_delta(ModelStreamDelta::Text(t.to_string()));
                                }
                            }
                            "thinking_delta" => {
                                if let Some(t) = delta.get("thinking").and_then(|v| v.as_str()) {
                                    reasoning.push_str(t);
                                    if let Some(b) = blocks.get_mut(&idx) {
                                        b.3.push_str(t);
                                    }
                                    on_delta(ModelStreamDelta::Reasoning(t.to_string()));
                                }
                            }
                            "signature_delta" => {
                                if let Some(s) = delta.get("signature").and_then(|v| v.as_str()) {
                                    if let Some(b) = blocks.get_mut(&idx) {
                                        b.1.push_str(s);
                                    }
                                }
                            }
                            "input_json_delta" => {
                                if let Some(j) = delta.get("partial_json").and_then(|v| v.as_str()) {
                                    if let Some(b) = blocks.get_mut(&idx) {
                                        b.3.push_str(j);
                                        on_delta(ModelStreamDelta::ToolCallArgsDelta {
                                            call_id: b.1.clone(),
                                            delta: j.to_string(),
                                        });
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
                "message_delta" => {
                    if let Some(sr) = payload
                        .get("delta")
                        .and_then(|d| d.get("stop_reason"))
                        .and_then(|v| v.as_str())
                    {
                        stop_reason = match sr {
                            "max_tokens" => ModelStopReason::OutputLimit,
                            "tool_use" => ModelStopReason::ToolCalls,
                            "refusal" => ModelStopReason::ContentFilter,
                            _ => ModelStopReason::Stop,
                        };
                    }
                    if let Some(u) = payload.get("usage") {
                        let parsed = parse_anthropic_usage(Some(u));
                        if parsed.output_tokens > 0 {
                            usage.output_tokens = parsed.output_tokens;
                        }
                        if parsed.usage_reported {
                            usage.usage_reported = true;
                        }
                    }
                }
                _ => {}
            }
        }

        if !saw_terminal {
            return Err(LlmError::Wire(
                "anthropic SSE stream ended prematurely without message_stop".into(),
            ));
        }

        let mut tool_calls = Vec::new();
        let mut cont_blocks = Vec::new();
        for (_, (btype, id_or_sig, name, buf)) in blocks {
            match btype.as_str() {
                "text" if !buf.is_empty() => cont_blocks.push(json!({"type": "text", "text": buf})),
                "thinking" => cont_blocks.push(json!({
                    "type": "thinking",
                    "thinking": buf,
                    "signature": id_or_sig
                })),
                "redacted_thinking" => cont_blocks.push(json!({
                    "type": "redacted_thinking",
                    "data": buf
                })),
                "tool_use" if stop_reason != ModelStopReason::OutputLimit => {
                    let parsed = parse_tool_arguments(&buf)?;
                    cont_blocks.push(json!({
                        "type": "tool_use",
                        "id": id_or_sig,
                        "name": name,
                        "input": parsed
                    }));
                    tool_calls.push(ModelToolCall {
                        call_id: id_or_sig,
                        name,
                        arguments: parsed,
                    });
                }
                _ => {}
            }
        }

        let continuation = if cont_blocks.is_empty() || stop_reason == ModelStopReason::OutputLimit {
            None
        } else {
            Some(ProviderContinuation {
                protocol: ModelProtocol::AnthropicMessages,
                model: response_model
                    .clone()
                    .unwrap_or_else(|| route.model.clone()),
                response_id: response_id.clone(),
                items: cont_blocks,
            })
        };

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
            continuation,
        ))
    }
}

fn parse_anthropic_usage(v: Option<&Value>) -> TokenUsage {
    let Some(u) = v else {
        return TokenUsage::default();
    };
    TokenUsage {
        input_tokens: u.get("input_tokens").and_then(|x| x.as_u64()).unwrap_or(0),
        output_tokens: u.get("output_tokens").and_then(|x| x.as_u64()).unwrap_or(0),
        cache_read_tokens: u
            .get("cache_read_input_tokens")
            .and_then(|x| x.as_u64())
            .unwrap_or(0),
        cache_write_tokens: u
            .get("cache_creation_input_tokens")
            .and_then(|x| x.as_u64())
            .unwrap_or(0),
        reasoning_tokens: 0,
        usage_reported: true,
    }
}
