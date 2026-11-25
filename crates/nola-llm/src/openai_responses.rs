// SPDX-License-Identifier: Apache-2.0
//! OpenAI Responses wire adapter (`responses`).
//! Shared by `SiwcResponsesAdapter` with clean continuation replay and strict media limits.

use std::collections::BTreeMap;
use std::io::BufRead;

use nola_types::{
    GenerationParams, HistoryMessage, MediaAttachment, ModelProtocol, ModelRouteConfig,
    ModelStopReason, ModelStreamDelta, ModelToolCall, NormalizedModelResponse,
    ProviderContinuation, StructuredModelRequest, TokenUsage,
};
use serde_json::{json, Value};

use crate::adapter::{
    collect_request_media, finalize_response, format_tail_reminders, join_frozen_system_prefix,
    parse_tool_arguments, BuiltHttpRequest, LlmError, ProtocolMediaLimits, ProviderWireAdapter,
};
use crate::openai_chat::parse_openai_usage;
use crate::sse::SseReader;

pub struct OpenAiResponsesAdapter;

fn responses_endpoint(base: &str) -> String {
    let trimmed = base.trim_end_matches('/');
    if trimmed.ends_with("/responses") {
        trimmed.to_string()
    } else if trimmed.ends_with("/v1") {
        format!("{trimmed}/responses")
    } else {
        format!("{trimmed}/v1/responses")
    }
}

fn responses_media_parts(text: &str, media: &[MediaAttachment]) -> Vec<Value> {
    let mut parts = Vec::new();
    if !text.is_empty() {
        parts.push(json!({"type": "input_text", "text": text}));
    }
    for att in media {
        let mime = att.mime_type.trim().to_ascii_lowercase();
        let data_url = format!("data:{mime};base64,{}", att.bytes_base64);
        if mime == "application/pdf" {
            parts.push(json!({
                "type": "input_file",
                "filename": att.filename,
                "file_data": data_url
            }));
        } else {
            parts.push(json!({
                "type": "input_image",
                "image_url": data_url
            }));
        }
    }
    parts
}

pub(crate) fn build_responses_body(
    protocol: ModelProtocol,
    route: &ModelRouteConfig,
    req: &StructuredModelRequest,
    params: &GenerationParams,
    stream: bool,
    force_store_false: bool,
) -> Result<Value, LlmError> {
    let all_media = collect_request_media(req);
    ProtocolMediaLimits::OPENAI.validate_all(&all_media, &route.capabilities)?;

    let mut input: Vec<Value> = Vec::new();
    for msg in &req.history {
        match msg {
            HistoryMessage::User { content, media } => {
                if media.is_empty() {
                    input.push(json!({
                        "type": "message",
                        "role": "user",
                        "content": [{"type": "input_text", "text": content}]
                    }));
                } else {
                    input.push(json!({
                        "type": "message",
                        "role": "user",
                        "content": responses_media_parts(content, media)
                    }));
                }
            }
            HistoryMessage::Assistant {
                content,
                tool_calls,
                continuation,
                ..
            } => {
                if let Some(cont) = continuation
                    .as_ref()
                    .filter(|c| c.protocol == protocol && !c.items.is_empty())
                {
                    for item in &cont.items {
                        if force_store_false {
                            let item_type = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
                            // Under store:false, drop reasoning items lacking encrypted_content and strip unstored server IDs
                            if item_type == "reasoning" && item.get("encrypted_content").is_none() {
                                continue;
                            }
                            let mut cleaned = item.clone();
                            if let Some(obj) = cleaned.as_object_mut() {
                                if item_type == "message" || item_type == "function_call" {
                                    obj.remove("id");
                                    obj.remove("status");
                                }
                            }
                            input.push(cleaned);
                        } else {
                            input.push(item.clone());
                        }
                    }
                } else {
                    if !content.is_empty() {
                        input.push(json!({
                            "type": "message",
                            "role": "assistant",
                            "content": [{"type": "output_text", "text": content}]
                        }));
                    }
                    for tc in tool_calls {
                        let mut fc = serde_json::Map::new();
                        fc.insert("type".into(), Value::String("function_call".into()));
                        fc.insert("call_id".into(), Value::String(tc.call_id.clone()));
                        fc.insert("name".into(), Value::String(tc.name.clone()));
                        if protocol == ModelProtocol::SiwcResponses {
                            fc.insert("namespace".into(), Value::String("nola".into()));
                        }
                        fc.insert(
                            "arguments".into(),
                            Value::String(
                                serde_json::to_string(&tc.arguments).unwrap_or_else(|_| "{}".into()),
                            ),
                        );
                        input.push(Value::Object(fc));
                    }
                }
            }
            HistoryMessage::Tool {
                call_id,
                payload,
                media,
                ..
            } => {
                input.push(json!({
                    "type": "function_call_output",
                    "call_id": call_id,
                    "output": payload.to_model_string()
                }));
                if !media.is_empty() {
                    input.push(json!({
                        "type": "message",
                        "role": "user",
                        "content": responses_media_parts("Attached media from tool output:", media)
                    }));
                }
            }
        }
    }

    if let Some(tail) = format_tail_reminders(&req.ephemeral_tail_reminders) {
        input.push(json!({
            "type": "message",
            "role": "user",
            "content": [{"type": "input_text", "text": tail}]
        }));
    }

    let mut body = serde_json::Map::new();
    body.insert("model".into(), Value::String(route.model.clone()));
    let instructions = join_frozen_system_prefix(req);
    if !instructions.is_empty() {
        body.insert("instructions".into(), Value::String(instructions));
    }
    body.insert("input".into(), Value::Array(input));
    body.insert("max_output_tokens".into(), json!(req.max_output_tokens));
    body.insert("stream".into(), Value::Bool(stream));
    if force_store_false {
        body.insert("store".into(), Value::Bool(false));
        if route.capabilities.reasoning {
            body.insert(
                "include".into(),
                json!(["reasoning.encrypted_content"]),
            );
        }
    }
    if route.capabilities.sampling_parameters {
        if let Some(t) = route.temperature {
            body.insert("temperature".into(), json!(t));
        }
        if let Some(tp) = route.top_p {
            body.insert("top_p".into(), json!(tp));
        }
        if let Some(seed) = params.seed {
            body.insert("seed".into(), json!(seed));
        }
    }
    if !params.stop.is_empty() {
        body.insert("stop".into(), json!(params.stop));
    }
    if let Some(effort) = route
        .reasoning_effort
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        if route.capabilities.reasoning {
            let mut r_obj = serde_json::Map::new();
            r_obj.insert("effort".into(), Value::String(effort.to_string()));
            if route.capabilities.supports_reasoning_summary
                && !effort.eq_ignore_ascii_case("none")
            {
                r_obj.insert("summary".into(), Value::String("auto".into()));
            }
            body.insert("reasoning".into(), Value::Object(r_obj));
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
                    "name": t.name,
                    "description": t.description,
                    "parameters": t.parameters,
                })
            })
            .collect();
        body.insert("tools".into(), Value::Array(tools));
    }
    if let Some(schema) = &req.json_schema {
        body.insert(
            "text".into(),
            json!({
                "format": {
                    "type": "json_schema",
                    "name": "nola_structured_output",
                    "strict": true,
                    "schema": schema
                }
            }),
        );
    }

    Ok(Value::Object(body))
}

pub(crate) fn decode_responses_body(
    protocol: ModelProtocol,
    route: &ModelRouteConfig,
    body: &Value,
) -> Result<NormalizedModelResponse, LlmError> {
    let response_id = body.get("id").and_then(|v| v.as_str()).map(ToString::to_string);
    let response_model = body
        .get("model")
        .and_then(|v| v.as_str())
        .map(ToString::to_string);

    let status = body.get("status").and_then(|v| v.as_str()).unwrap_or("completed");
    if status == "failed" || status == "cancelled" {
        let err = body
            .get("error")
            .map(ToString::to_string)
            .unwrap_or_else(|| status.to_string());
        return Err(LlmError::Wire(format!("responses API {status}: {err}")));
    }
    let incomplete_reason = body
        .get("incomplete_details")
        .and_then(|d| d.get("reason"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let stop_reason = if status == "incomplete" && incomplete_reason == "max_output_tokens" {
        ModelStopReason::OutputLimit
    } else if incomplete_reason == "content_filter" {
        ModelStopReason::ContentFilter
    } else {
        ModelStopReason::Stop
    };

    let output = body
        .get("output")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut content = String::new();
    let mut reasoning = String::new();
    let mut tool_calls = Vec::new();
    let mut cont_items = Vec::new();

    for item in &output {
        cont_items.push(item.clone());
        match item.get("type").and_then(|v| v.as_str()).unwrap_or("") {
            "message" => {
                if let Some(parts) = item.get("content").and_then(|v| v.as_array()) {
                    for p in parts {
                        if let Some(t) = p.get("text").and_then(|v| v.as_str()) {
                            content.push_str(t);
                        }
                    }
                }
            }
            "reasoning" => {
                if let Some(summary) = item.get("summary").and_then(|v| v.as_array()) {
                    for s in summary {
                        if let Some(t) = s.get("text").and_then(|v| v.as_str()) {
                            reasoning.push_str(t);
                        }
                    }
                }
            }
            "function_call" if stop_reason != ModelStopReason::OutputLimit => {
                let call_id = item
                    .get("call_id")
                    .or_else(|| item.get("id"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("call_0")
                    .to_string();
                let name = item
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let raw_args = item
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
            _ => {}
        }
    }

    let continuation = if cont_items.is_empty() || stop_reason == ModelStopReason::OutputLimit {
        None
    } else {
        Some(ProviderContinuation {
            protocol,
            model: response_model
                .clone()
                .unwrap_or_else(|| route.model.clone()),
            response_id: response_id.clone(),
            items: cont_items,
        })
    };

    let usage = parse_openai_usage(body.get("usage"));
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

pub(crate) fn parse_responses_sse(
    protocol: ModelProtocol,
    route: &ModelRouteConfig,
    reader: &mut dyn BufRead,
    on_delta: &mut dyn FnMut(ModelStreamDelta),
) -> Result<NormalizedModelResponse, LlmError> {
    let mut sse = SseReader::new(reader);
    let mut final_response_obj: Option<Value> = None;
    let mut content = String::new();
    let mut reasoning = String::new();
    let mut stop_reason = ModelStopReason::Stop;
    let mut usage = TokenUsage::default();
    let mut calls: BTreeMap<String, (String, String)> = BTreeMap::new();
    let mut item_id_to_call_id: BTreeMap<String, String> = BTreeMap::new();
    let mut saw_terminal = false;

    while let Some(ev) = sse.next_event()? {
        let trimmed = ev.data.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed == "[DONE]" {
            saw_terminal = true;
            continue;
        }
        let payload: Value = serde_json::from_str(trimmed)?;
        let kind = payload
            .get("type")
            .and_then(|v| v.as_str())
            .or(ev.event.as_deref())
            .unwrap_or("");
        match kind {
            "error" | "response.failed" => {
                return Err(LlmError::Wire(format!("responses SSE error ({kind}): {payload}")));
            }
            "response.output_text.delta" => {
                if let Some(d) = payload.get("delta").and_then(|v| v.as_str()) {
                    content.push_str(d);
                    on_delta(ModelStreamDelta::Text(d.to_string()));
                }
            }
            "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
                if let Some(d) = payload.get("delta").and_then(|v| v.as_str()) {
                    reasoning.push_str(d);
                    on_delta(ModelStreamDelta::Reasoning(d.to_string()));
                }
            }
            "response.output_item.added" => {
                if let Some(item) = payload.get("item") {
                    if item.get("type").and_then(|v| v.as_str()) == Some("function_call") {
                        let cid = item
                            .get("call_id")
                            .or_else(|| item.get("id"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("call_0")
                            .to_string();
                        if let Some(item_id) = item.get("id").and_then(|v| v.as_str()) {
                            item_id_to_call_id.insert(item_id.to_string(), cid.clone());
                        }
                        let name = item
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        calls.insert(cid.clone(), (name.clone(), String::new()));
                        on_delta(ModelStreamDelta::ToolCallStart { call_id: cid, name });
                    }
                }
            }
            "response.function_call_arguments.delta" => {
                let raw_id = payload
                    .get("call_id")
                    .or_else(|| payload.get("item_id"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("call_0");
                let cid = item_id_to_call_id
                    .get(raw_id)
                    .cloned()
                    .unwrap_or_else(|| raw_id.to_string());
                if let Some(d) = payload.get("delta").and_then(|v| v.as_str()) {
                    calls
                        .entry(cid.clone())
                        .or_insert_with(|| (String::new(), String::new()))
                        .1
                        .push_str(d);
                    on_delta(ModelStreamDelta::ToolCallArgsDelta {
                        call_id: cid,
                        delta: d.to_string(),
                    });
                }
            }
            "response.completed" | "response.incomplete" => {
                saw_terminal = true;
                if kind == "response.incomplete" {
                    stop_reason = ModelStopReason::OutputLimit;
                }
                if let Some(resp) = payload.get("response") {
                    usage = parse_openai_usage(resp.get("usage"));
                    final_response_obj = Some(resp.clone());
                }
            }
            _ => {}
        }
    }

    if let Some(resp) = final_response_obj {
        return decode_responses_body(protocol, route, &resp);
    }
    if !saw_terminal {
        return Err(LlmError::Wire(
            "responses SSE stream ended prematurely without completion event".into(),
        ));
    }

    let mut tool_calls = Vec::new();
    if stop_reason != ModelStopReason::OutputLimit {
        for (call_id, (name, raw_args)) in calls {
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
        None,
        Some(route.model.clone()),
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

impl ProviderWireAdapter for OpenAiResponsesAdapter {
    fn protocol(&self) -> ModelProtocol {
        ModelProtocol::OpenAiResponses
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
        self.build_request_with_params(route, req, &GenerationParams::default(), stream)
    }

    fn build_request_with_params(
        &self,
        route: &ModelRouteConfig,
        req: &StructuredModelRequest,
        params: &GenerationParams,
        stream: bool,
    ) -> Result<BuiltHttpRequest, LlmError> {
        let body = build_responses_body(
            ModelProtocol::OpenAiResponses,
            route,
            req,
            params,
            stream,
            false,
        )?;
        let mut headers = BTreeMap::new();
        headers.insert("content-type".into(), "application/json".into());
        if !route.api_key.trim().is_empty() {
            headers.insert(
                "authorization".into(),
                format!("Bearer {}", route.api_key.trim()),
            );
        }
        Ok(BuiltHttpRequest {
            url: responses_endpoint(&route.endpoint),
            headers,
            body,
        })
    }

    fn decode_response(
        &self,
        route: &ModelRouteConfig,
        body: &Value,
    ) -> Result<NormalizedModelResponse, LlmError> {
        decode_responses_body(ModelProtocol::OpenAiResponses, route, body)
    }

    fn parse_sse_stream(
        &self,
        route: &ModelRouteConfig,
        reader: &mut dyn BufRead,
        on_delta: &mut dyn FnMut(ModelStreamDelta),
    ) -> Result<NormalizedModelResponse, LlmError> {
        parse_responses_sse(ModelProtocol::OpenAiResponses, route, reader, on_delta)
    }
}
