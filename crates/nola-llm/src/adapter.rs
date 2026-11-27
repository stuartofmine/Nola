// SPDX-License-Identifier: Apache-2.0
//! Shared `ProviderWireAdapter` trait, per-protocol media validation,
//! tail-reminder formatting, and atomic `OutputLimit` response finalization.

use std::collections::BTreeMap;
use std::io::BufRead;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use nola_types::{
    GenerationParams, MediaAttachment, ModelCapabilities, ModelProtocol, ModelRouteConfig,
    ModelStopReason, ModelStreamDelta, ModelToolCall, NormalizedModelResponse,
    ProviderContinuation, RuntimeReminder, StructuredModelRequest, TokenUsage,
};
use serde_json::Value;
use thiserror::Error;

use crate::anthropic::AnthropicMessagesAdapter;
use crate::gemini::GeminiAdapter;
use crate::openai_chat::OpenAiChatAdapter;
use crate::openai_responses::OpenAiResponsesAdapter;
use crate::siwc::SiwcResponsesAdapter;

#[derive(Debug, Error)]
pub enum LlmError {
    #[error("http error: {0}")]
    Http(String),
    #[error("provider returned status {status}: {body}")]
    Status { status: u16, body: String },
    #[error("wire protocol error: {0}")]
    Wire(String),
    #[error("media policy error: {0}")]
    Media(String),
    #[error("request cancelled")]
    Cancelled,
    #[error("model entered repetitive reasoning loop")]
    RepetitiveReasoning,
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Normalize a route's `reasoning_effort` for wire transmission.
///
/// Returns `None` for `None`, blank strings, `"auto"`, `"automatic"`, and `"default"`
/// (case-insensitive) so provider-default selections NEVER leak onto the wire.
pub fn normalized_wire_reasoning_effort(raw: Option<&str>) -> Option<&str> {
    let trimmed = raw?.trim();
    if trimmed.is_empty()
        || trimmed.eq_ignore_ascii_case("auto")
        || trimmed.eq_ignore_ascii_case("automatic")
        || trimmed.eq_ignore_ascii_case("default")
    {
        return None;
    }
    Some(trimmed)
}

/// Maps a supported audio MIME type to its canonical OpenAI `input_audio.format` token.
pub fn audio_format_for_mime(mime: &str) -> Option<&'static str> {
    match mime.trim().to_ascii_lowercase().as_str() {
        "audio/wav" | "audio/x-wav" => Some("wav"),
        "audio/mp3" | "audio/mpeg" => Some("mp3"),
        "audio/ogg" => Some("ogg"),
        "audio/flac" => Some("flac"),
        "audio/m4a" => Some("m4a"),
        "audio/aac" => Some("aac"),
        _ => None,
    }
}

/// Returns true when `mime` is a supported inline audio MIME type.
pub fn is_supported_audio_mime(mime: &str) -> bool {
    audio_format_for_mime(mime).is_some()
}

/// Wire HTTP request built by a `ProviderWireAdapter`.
#[derive(Debug, Clone)]
pub struct BuiltHttpRequest {
    pub url: String,
    pub headers: BTreeMap<String, String>,
    pub body: Value,
}

/// Per-protocol media guardrails enforced uniformly across ALL 5 wire adapters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProtocolMediaLimits {
    pub max_bytes_per_item: usize,
    pub max_items_per_request: usize,
    pub supports_audio: bool,
}

impl ProtocolMediaLimits {
    pub const ANTHROPIC: Self = Self {
        max_bytes_per_item: 5 * 1024 * 1024,
        max_items_per_request: 80,
        supports_audio: false,
    };

    pub const OPENAI: Self = Self {
        max_bytes_per_item: 20 * 1024 * 1024,
        max_items_per_request: 80,
        supports_audio: false,
    };

    pub const OPENAI_CHAT: Self = Self {
        max_bytes_per_item: 20 * 1024 * 1024,
        max_items_per_request: 80,
        supports_audio: true,
    };

    pub const GEMINI: Self = Self {
        max_bytes_per_item: 20 * 1024 * 1024,
        max_items_per_request: 80,
        supports_audio: true,
    };

    pub fn validate_all(
        &self,
        attachments: &[&MediaAttachment],
        caps: &ModelCapabilities,
    ) -> Result<(), LlmError> {
        if attachments.len() > self.max_items_per_request {
            return Err(LlmError::Media(format!(
                "media attachment count {} exceeds protocol cap {}",
                attachments.len(),
                self.max_items_per_request
            )));
        }
        for att in attachments {
            let mime = att.mime_type.trim().to_ascii_lowercase();
            let is_image = matches!(
                mime.as_str(),
                "image/png" | "image/jpeg" | "image/jpg" | "image/gif" | "image/webp"
            );
            let is_pdf = mime == "application/pdf";
            let is_audio = is_supported_audio_mime(&mime);
            if !is_image && !is_pdf && !is_audio {
                return Err(LlmError::Media(format!(
                    "unsupported media mime type '{}'",
                    att.mime_type
                )));
            }
            if is_audio && !self.supports_audio {
                return Err(LlmError::Media(format!(
                    "audio media '{}' is not supported by this wire protocol",
                    att.mime_type
                )));
            }
            if is_image && !caps.image_input {
                return Err(LlmError::Media(
                    "model route does not declare image_input capability".into(),
                ));
            }
            if is_pdf && !caps.native_pdf_input {
                return Err(LlmError::Media(
                    "model route does not declare native_pdf_input capability".into(),
                ));
            }
            if att.raw_bytes_len > self.max_bytes_per_item {
                return Err(LlmError::Media(format!(
                    "media '{}' ({} bytes) exceeds protocol limit ({} bytes)",
                    att.filename, att.raw_bytes_len, self.max_bytes_per_item
                )));
            }
        }
        Ok(())
    }
}

/// Uniform wire adapter implemented by all supported LLM protocols.
pub trait ProviderWireAdapter: Send + Sync {
    fn protocol(&self) -> ModelProtocol;
    fn media_limits(&self) -> ProtocolMediaLimits;
    fn build_request(
        &self,
        route: &ModelRouteConfig,
        req: &StructuredModelRequest,
        stream: bool,
    ) -> Result<BuiltHttpRequest, LlmError>;
    fn build_request_with_params(
        &self,
        route: &ModelRouteConfig,
        req: &StructuredModelRequest,
        _params: &GenerationParams,
        stream: bool,
    ) -> Result<BuiltHttpRequest, LlmError> {
        self.build_request(route, req, stream)
    }
    fn decode_response(
        &self,
        route: &ModelRouteConfig,
        body: &Value,
    ) -> Result<NormalizedModelResponse, LlmError>;
    fn parse_sse_stream(
        &self,
        route: &ModelRouteConfig,
        reader: &mut dyn BufRead,
        on_delta: &mut dyn FnMut(ModelStreamDelta),
    ) -> Result<NormalizedModelResponse, LlmError>;
    fn parse_sse_stream_cancellable(
        &self,
        route: &ModelRouteConfig,
        reader: &mut dyn BufRead,
        cancel: Option<Arc<AtomicBool>>,
        on_delta: &mut dyn FnMut(ModelStreamDelta),
    ) -> Result<NormalizedModelResponse, LlmError> {
        let _ = cancel;
        self.parse_sse_stream(route, reader, on_delta)
    }
}

/// Resolve the canonical wire adapter for a `ModelProtocol`.
pub fn adapter_for(protocol: ModelProtocol) -> Box<dyn ProviderWireAdapter> {
    match protocol {
        ModelProtocol::OpenAiChat => Box::new(OpenAiChatAdapter),
        ModelProtocol::OpenAiResponses => Box::new(OpenAiResponsesAdapter),
        ModelProtocol::AnthropicMessages => Box::new(AnthropicMessagesAdapter),
        ModelProtocol::SiwcResponses => Box::new(SiwcResponsesAdapter),
        ModelProtocol::Gemini => Box::new(GeminiAdapter),
    }
}

/// Join frozen leading system blocks deterministically.
pub fn join_frozen_system_prefix(req: &StructuredModelRequest) -> String {
    req.frozen_system_prefix
        .iter()
        .map(|b| b.text.trim())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Format ephemeral tail reminders as a single trailing `user` message block.
/// NEVER emitted as `system` or `developer`, preserving leading KV cache!
pub fn format_tail_reminders(reminders: &[RuntimeReminder]) -> Option<String> {
    if reminders.is_empty() {
        return None;
    }
    let body = reminders
        .iter()
        .map(|r| format!("<runtime_reminder tag=\"{}\">\n{}\n</runtime_reminder>", r.tag, r.text.trim()))
        .collect::<Vec<_>>()
        .join("\n\n");
    Some(body)
}

/// Collect all inline media attachments across `req.history` for upfront validation.
pub fn collect_request_media(req: &StructuredModelRequest) -> Vec<&MediaAttachment> {
    let mut out = Vec::new();
    for msg in &req.history {
        match msg {
            nola_types::HistoryMessage::User { media, .. }
            | nola_types::HistoryMessage::Tool { media, .. } => {
                out.extend(media.iter());
            }
            nola_types::HistoryMessage::Assistant { .. } => {}
        }
    }
    out
}

/// Parse tool-call JSON arguments safely without crashing on empty strings,
/// always normalizing to a JSON object so Anthropic/OpenAI tool schemas stay valid.
pub fn parse_tool_arguments(raw: &str) -> Result<Value, LlmError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(Value::Object(serde_json::Map::new()));
    }
    let val: Value = serde_json::from_str(trimmed)
        .map_err(|e| LlmError::Wire(format!("malformed tool call JSON arguments: {e}")))?;
    match val {
        Value::Object(_) => Ok(val),
        Value::Null => Ok(Value::Object(serde_json::Map::new())),
        other => Ok(serde_json::json!({ "input": other })),
    }
}

/// Canonical response finalizer shared across ALL 4 wire protocols!
///
/// Architectural invariant:
/// Whenever `stop_reason == OutputLimit` or `ContentFilter`, any partially generated
/// `tool_calls` are ATOMICALLY DISCARDED so truncated JSON arguments can never
/// corrupt files or clobber a safety refusal.
#[allow(clippy::too_many_arguments)]
pub fn finalize_response(
    response_id: Option<String>,
    response_model: Option<String>,
    content: String,
    reasoning: Option<String>,
    mut tool_calls: Vec<ModelToolCall>,
    mut stop_reason: ModelStopReason,
    usage: TokenUsage,
    continuation: Option<ProviderContinuation>,
) -> NormalizedModelResponse {
    if matches!(
        stop_reason,
        ModelStopReason::OutputLimit | ModelStopReason::ContentFilter
    ) {
        tool_calls.clear();
    } else if !tool_calls.is_empty() {
        stop_reason = ModelStopReason::ToolCalls;
    }
    let reasoning = reasoning.filter(|r| !r.is_empty());
    NormalizedModelResponse {
        response_id,
        response_model,
        content,
        reasoning,
        tool_calls,
        stop_reason,
        usage,
        continuation,
    }
}
