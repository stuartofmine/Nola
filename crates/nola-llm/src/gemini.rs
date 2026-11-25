// SPDX-License-Identifier: Apache-2.0
//! Google Gemini Generative Language wire adapter (`gemini`).
//!
//! Guarantees:
//! - Immutable `systemInstruction` built exclusively from `frozen_system_prefix`.
//! - Ephemeral tail reminders stay in trailing `user` `contents` (never hoisted to `systemInstruction`).
//! - Consecutive `user` turns (including `functionResponse` + tail reminders) are merged cleanly.
//! - Reports `usage.usage_reported = true` and extracts `thoughtsTokenCount` -> `reasoning_tokens`
//!   and `cachedContentTokenCount` -> `cache_read_tokens`.
//! - Atomically discards truncated `functionCall` parts on `MAX_TOKENS` or safety `ContentFilter`.

use std::collections::BTreeMap;
use std::io::BufRead;

use nola_types::{
    GenerationParams, HistoryMessage, MediaAttachment, ModelProtocol, ModelRouteConfig,
    ModelStopReason, ModelStreamDelta, ModelToolCall, NormalizedModelResponse,
    StructuredModelRequest, TokenUsage,
};
use serde_json::{json, Value};

use crate::adapter::{
    collect_request_media, finalize_response, format_tail_reminders, join_frozen_system_prefix,
    BuiltHttpRequest, LlmError, ProtocolMediaLimits, ProviderWireAdapter,
};
use crate::sse::SseReader;

pub struct GeminiAdapter;

fn gemini_endpoint(base: &str, model: &str, stream: bool) -> String {
    let trimmed = base.trim().trim_end_matches('/');
    let action = if stream {
        "streamGenerateContent?alt=sse"
    } else {
        "generateContent"
    };
    if trimmed.contains(":generateContent") || trimmed.contains(":streamGenerateContent") {
        if stream && !trimmed.contains("alt=sse") {
            let sep = if trimmed.contains('?') { '&' } else { '?' };
            return format!("{trimmed}{sep}alt=sse");
        }
        return trimmed.to_string();
    }
    let root = if trimmed.is_empty() {
        "https://generativelanguage.googleapis.com"
    } else {
        trimmed
    };
    let clean_model = model.trim().trim_start_matches("models/");
    if root.ends_with("/v1beta") || root.ends_with("/v1") {
        format!("{root}/models/{clean_model}:{action}")
    } else {
        format!("{root}/v1beta/models/{clean_model}:{action}")
    }
}

fn push_or_merge_content(contents: &mut Vec<Value>, role: &str, mut new_parts: Vec<Value>) {
    if new_parts.is_empty() {
        return;
    }
    if let Some(last) = contents.last_mut() {
        if last.get("role").and_then(|v| v.as_str()) == Some(role) {
            if let Some(arr) = last.get_mut("parts").and_then(|v| v.as_array_mut()) {
                arr.append(&mut new_parts);
                return;
            }
        }
    }
    contents.push(json!({
        "role": role,
        "parts": new_parts
    }));
}

fn media_parts(media: &[MediaAttachment]) -> Vec<Value> {
    media
        .iter()
        .map(|att| {
            let raw_mime = att.mime_type.trim().to_ascii_lowercase();
            let mime = if raw_mime == "image/jpg" {
                "image/jpeg"
            } else {
                raw_mime.as_str()
            };
            json!({
                "inlineData": {
                    "mimeType": mime,
                    "data": att.bytes_base64
                }
            })
        })
        .collect()
}

fn map_gemini_finish_reason(raw: &str) -> ModelStopReason {
    match raw.to_ascii_uppercase().as_str() {
        "MAX_TOKENS" => ModelStopReason::OutputLimit,
        "SAFETY" | "RECITATION" | "BLOCKLIST" | "PROHIBITED_CONTENT" | "SPII" => {
            ModelStopReason::ContentFilter
        }
        _ => ModelStopReason::Stop,
    }
}

fn parse_gemini_usage(v: Option<&Value>) -> TokenUsage {
    let Some(u) = v else {
        return TokenUsage::default();
    };
    TokenUsage {
        input_tokens: u
            .get("promptTokenCount")
            .and_then(|x| x.as_u64())
            .unwrap_or(0),
        output_tokens: u
            .get("candidatesTokenCount")
            .and_then(|x| x.as_u64())
            .unwrap_or(0),
        cache_read_tokens: u
            .get("cachedContentTokenCount")
            .and_then(|x| x.as_u64())
            .unwrap_or(0),
        cache_write_tokens: 0,
        reasoning_tokens: u
            .get("thoughtsTokenCount")
            .and_then(|x| x.as_u64())
            .unwrap_or(0),
        usage_reported: true,
    }
}

impl ProviderWireAdapter for GeminiAdapter {
    fn protocol(&self) -> ModelProtocol {
        ModelProtocol::Gemini
    }

    fn media_limits(&self) -> ProtocolMediaLimits {
        ProtocolMediaLimits::GEMINI
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

        let mut body = serde_json::Map::new();
        let system_text = join_frozen_system_prefix(req);
        if !system_text.is_empty() {
            body.insert(
                "systemInstruction".into(),
                json!({ "parts": [{ "text": system_text }] }),
            );
        }

        let mut contents: Vec<Value> = Vec::new();
        for msg in &req.history {
            match msg {
                HistoryMessage::User { content, media } => {
                    let mut parts = Vec::new();
                    if !content.is_empty() {
                        parts.push(json!({"text": content}));
                    }
                    parts.extend(media_parts(media));
                    push_or_merge_content(&mut contents, "user", parts);
                }
                HistoryMessage::Assistant {
                    content,
                    tool_calls,
                    ..
                } => {
                    let mut parts = Vec::new();
                    if !content.is_empty() {
                        parts.push(json!({"text": content}));
                    }
                    for tc in tool_calls {
                        parts.push(json!({
                            "functionCall": {
                                "name": tc.name,
                                "args": tc.arguments
                            }
                        }));
                    }
                    push_or_merge_content(&mut contents, "model", parts);
                }
                HistoryMessage::Tool {
                    tool_name,
                    payload,
                    is_error,
                    media,
                    ..
                } => {
                    let mut parts = vec![json!({
                        "functionResponse": {
                            "name": tool_name,
                            "response": {
                                "output": payload.to_model_string(),
                                "is_error": is_error
                            }
                        }
                    })];
                    parts.extend(media_parts(media));
                    push_or_merge_content(&mut contents, "user", parts);
                }
            }
        }

        if let Some(tail) = format_tail_reminders(&req.ephemeral_tail_reminders) {
            push_or_merge_content(&mut contents, "user", vec![json!({"text": tail})]);
        }
        if contents.is_empty() {
            contents.push(json!({"role": "user", "parts": [{"text": "Continue."}]}));
        }
        body.insert("contents".into(), Value::Array(contents));

        if route.capabilities.tool_calls && !req.prefix_stable_tools.is_empty() {
            let decls: Vec<Value> = req
                .prefix_stable_tools
                .iter()
                .map(|t| {
                    json!({
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.parameters
                    })
                })
                .collect();
            body.insert(
                "tools".into(),
                json!([{ "functionDeclarations": decls }]),
            );
        }

        let mut gen_cfg = serde_json::Map::new();
        gen_cfg.insert("maxOutputTokens".into(), json!(req.max_output_tokens));
        if route.capabilities.sampling_parameters {
            if let Some(t) = route.temperature {
                gen_cfg.insert("temperature".into(), json!(t));
            }
            if let Some(tp) = route.top_p {
                gen_cfg.insert("topP".into(), json!(tp));
            }
            if let Some(tk) = params.top_k {
                gen_cfg.insert("topK".into(), json!(tk));
            }
            if let Some(seed) = params.seed {
                gen_cfg.insert("seed".into(), json!(seed));
            }
        }
        if !params.stop.is_empty() {
            gen_cfg.insert("stopSequences".into(), json!(params.stop));
        }
        if route.capabilities.reasoning {
            if let Some(budget) = params.thinking_budget {
                gen_cfg.insert(
                    "thinkingConfig".into(),
                    json!({ "thinkingBudget": budget, "includeThoughts": budget > 0 }),
                );
            } else if let Some(effort) = route.reasoning_effort.as_deref().map(str::trim) {
                if effort.eq_ignore_ascii_case("none") {
                    gen_cfg.insert("thinkingConfig".into(), json!({ "thinkingBudget": 0 }));
                } else if !effort.is_empty() {
                    gen_cfg.insert(
                        "thinkingConfig".into(),
                        json!({
                            "thinkingLevel": effort.to_ascii_uppercase(),
                            "includeThoughts": true
                        }),
                    );
                }
            }
        }
        if let Some(schema) = &req.json_schema {
            gen_cfg.insert("responseMimeType".into(), json!("application/json"));
            gen_cfg.insert("responseSchema".into(), schema.clone());
        }
        body.insert("generationConfig".into(), Value::Object(gen_cfg));

        let mut headers = BTreeMap::new();
        headers.insert("content-type".into(), "application/json".into());
        if !route.api_key.trim().is_empty() {
            headers.insert("x-goog-api-key".into(), route.api_key.trim().to_string());
        }

        Ok(BuiltHttpRequest {
            url: gemini_endpoint(&route.endpoint, &route.model, stream),
            headers,
            body: Value::Object(body),
        })
    }

    fn decode_response(
        &self,
        route: &ModelRouteConfig,
        body: &Value,
    ) -> Result<NormalizedModelResponse, LlmError> {
        let response_id = body
            .get("responseId")
            .and_then(|v| v.as_str())
            .map(ToString::to_string);
        let response_model = body
            .get("modelVersion")
            .and_then(|v| v.as_str())
            .map(ToString::to_string)
            .or_else(|| Some(route.model.clone()));
        let usage = parse_gemini_usage(body.get("usageMetadata"));

        let Some(candidate) = body
            .get("candidates")
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.first())
        else {
            return Err(LlmError::Wire(
                "gemini response missing candidates[0]".into(),
            ));
        };

        let stop_reason = candidate
            .get("finishReason")
            .and_then(|v| v.as_str())
            .map(map_gemini_finish_reason)
            .unwrap_or(ModelStopReason::Stop);

        let mut content = String::new();
        let mut reasoning = String::new();
        let mut tool_calls = Vec::new();
        if let Some(parts) = candidate
            .get("content")
            .and_then(|c| c.get("parts"))
            .and_then(|p| p.as_array())
        {
            for (idx, part) in parts.iter().enumerate() {
                if let Some(text) = part.get("text").and_then(|v| v.as_str()) {
                    if part.get("thought").and_then(|v| v.as_bool()).unwrap_or(false) {
                        reasoning.push_str(text);
                    } else {
                        content.push_str(text);
                    }
                } else if let Some(fc) = part.get("functionCall") {
                    if stop_reason != ModelStopReason::OutputLimit {
                        let name = fc
                            .get("name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let args = fc.get("args").cloned().unwrap_or_else(|| json!({}));
                        let call_id = fc
                            .get("id")
                            .and_then(|v| v.as_str())
                            .map(ToString::to_string)
                            .unwrap_or_else(|| format!("gemini_call_{idx}"));
                        if !name.is_empty() {
                            tool_calls.push(ModelToolCall {
                                call_id,
                                name,
                                arguments: args,
                            });
                        }
                    }
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
        let mut tool_calls = Vec::new();
        let mut stop_reason = ModelStopReason::Stop;
        let mut usage = TokenUsage::default();
        let mut saw_any_chunk = false;

        while let Some(ev) = sse.next_event()? {
            let trimmed = ev.data.trim();
            if trimmed.is_empty() || trimmed == "[DONE]" {
                continue;
            }
            let chunk: Value = serde_json::from_str(trimmed)?;
            saw_any_chunk = true;
            if let Some(err) = chunk.get("error").filter(|v| !v.is_null()) {
                return Err(LlmError::Wire(format!("gemini SSE error: {err}")));
            }
            if response_id.is_none() {
                response_id = chunk
                    .get("responseId")
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string);
            }
            if response_model.is_none() {
                response_model = chunk
                    .get("modelVersion")
                    .and_then(|v| v.as_str())
                    .map(ToString::to_string);
            }
            if let Some(u) = chunk.get("usageMetadata").filter(|v| v.is_object()) {
                usage = parse_gemini_usage(Some(u));
            }
            if let Some(cand) = chunk
                .get("candidates")
                .and_then(|v| v.as_array())
                .and_then(|a| a.first())
            {
                if let Some(fr) = cand.get("finishReason").and_then(|v| v.as_str()) {
                    stop_reason = map_gemini_finish_reason(fr);
                }
                if let Some(parts) = cand
                    .get("content")
                    .and_then(|c| c.get("parts"))
                    .and_then(|p| p.as_array())
                {
                    for part in parts {
                        if let Some(text) = part.get("text").and_then(|v| v.as_str()) {
                            if !text.is_empty() {
                                if part.get("thought").and_then(|v| v.as_bool()).unwrap_or(false) {
                                    reasoning.push_str(text);
                                    on_delta(ModelStreamDelta::Reasoning(text.to_string()));
                                } else {
                                    content.push_str(text);
                                    on_delta(ModelStreamDelta::Text(text.to_string()));
                                }
                            }
                        } else if let Some(fc) = part.get("functionCall") {
                            let name = fc
                                .get("name")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            let args = fc.get("args").cloned().unwrap_or_else(|| json!({}));
                            let call_id = fc
                                .get("id")
                                .and_then(|v| v.as_str())
                                .map(ToString::to_string)
                                .unwrap_or_else(|| format!("gemini_call_{}", tool_calls.len()));
                            if !name.is_empty() {
                                on_delta(ModelStreamDelta::ToolCallStart {
                                    call_id: call_id.clone(),
                                    name: name.clone(),
                                });
                                tool_calls.push(ModelToolCall {
                                    call_id,
                                    name,
                                    arguments: args,
                                });
                            }
                        }
                    }
                }
            }
        }

        if !saw_any_chunk {
            return Err(LlmError::Wire("gemini SSE stream ended empty".into()));
        }

        Ok(finalize_response(
            response_id,
            response_model.or_else(|| Some(route.model.clone())),
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
