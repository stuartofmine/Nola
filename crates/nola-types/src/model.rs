// SPDX-License-Identifier: Apache-2.0
//! Structured LLM request, response, continuation, and protocol types.
//!
//! Compile-time KV-cache invariant:
//! `HistoryMessage` has ONLY `User`, `Assistant`, and `Tool` variants — NEVER
//! `System`. Dynamic runtime guidance belongs exclusively in
//! `StructuredModelRequest::ephemeral_tail_reminders`.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{ToolModelPayload, ToolSchema};

/// Supported LLM wire protocols (exhaustive across the entire workspace).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ModelProtocol {
    #[serde(rename = "chat_completions", alias = "openai_chat", alias = "open_ai_chat", alias = "openai-compatible", alias = "openai")]
    OpenAiChat,
    #[serde(rename = "responses", alias = "openai_responses", alias = "open_ai_responses")]
    OpenAiResponses,
    #[serde(rename = "anthropic", alias = "anthropic_messages")]
    AnthropicMessages,
    #[serde(rename = "siwc_responses", alias = "siwc", alias = "chatgpt")]
    SiwcResponses,
    #[serde(rename = "gemini", alias = "google_gemini", alias = "google")]
    Gemini,
}

/// Alias for `ModelProtocol` used by callers referring to wire protocol contracts.
pub type WireProtocol = ModelProtocol;

impl ModelProtocol {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OpenAiChat => "chat_completions",
            Self::OpenAiResponses => "responses",
            Self::AnthropicMessages => "anthropic",
            Self::SiwcResponses => "siwc_responses",
            Self::Gemini => "gemini",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "chat_completions" | "openai_chat" | "open_ai_chat" | "openai-compatible" | "openai" => {
                Some(Self::OpenAiChat)
            }
            "responses" | "openai_responses" | "open_ai_responses" => Some(Self::OpenAiResponses),
            "anthropic" | "anthropic_messages" => Some(Self::AnthropicMessages),
            "siwc_responses" | "siwc" | "chatgpt" => Some(Self::SiwcResponses),
            "gemini" | "google_gemini" | "google" => Some(Self::Gemini),
            _ => None,
        }
    }
}

/// Capabilities declared for a concrete model route (accepts both camelCase and snake_case).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelCapabilities {
    #[serde(default = "default_true", alias = "tool_calls")]
    pub tool_calls: bool,
    #[serde(default, alias = "image_input")]
    pub image_input: bool,
    #[serde(default, alias = "native_pdf_input")]
    pub native_pdf_input: bool,
    #[serde(default)]
    pub reasoning: bool,
    #[serde(default = "default_true", alias = "sampling_parameters")]
    pub sampling_parameters: bool,
    #[serde(default, alias = "stream_reasoning")]
    pub stream_reasoning: bool,
    #[serde(default, alias = "supported_reasoning_levels")]
    pub supported_reasoning_levels: Vec<String>,
    #[serde(default, alias = "supports_reasoning_summary")]
    pub supports_reasoning_summary: bool,
}

fn default_true() -> bool {
    true
}

impl Default for ModelCapabilities {
    fn default() -> Self {
        Self {
            tool_calls: true,
            image_input: false,
            native_pdf_input: false,
            reasoning: false,
            sampling_parameters: true,
            stream_reasoning: false,
            supported_reasoning_levels: Vec::new(),
            supports_reasoning_summary: false,
        }
    }
}

/// 3-state deterministic seed override:
/// - `Inherit`: retain inherited/route seed
/// - `Clear`: explicitly unset any inherited seed
/// - `Value(v)`: force a specific integer seed
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "mode", content = "value", rename_all = "snake_case")]
pub enum SeedOverride {
    #[default]
    Inherit,
    Clear,
    Value(i64),
}

impl SeedOverride {
    pub fn resolve(self, inherited: Option<i64>) -> Option<i64> {
        match self {
            Self::Inherit => inherited,
            Self::Clear => None,
            Self::Value(v) => Some(v),
        }
    }
}

/// Optional generation parameters forwarded to compatible LLM wire adapters.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenerationParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<i64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stop: Vec<String>,
    #[serde(default, alias = "top_k", skip_serializing_if = "Option::is_none")]
    pub top_k: Option<u32>,
    #[serde(
        default,
        alias = "repetition_penalty",
        skip_serializing_if = "Option::is_none"
    )]
    pub repetition_penalty: Option<f32>,
    #[serde(
        default,
        alias = "thinking_budget",
        skip_serializing_if = "Option::is_none"
    )]
    pub thinking_budget: Option<usize>,
}

impl GenerationParams {
    pub fn apply_seed_override(&mut self, override_seed: SeedOverride) {
        self.seed = override_seed.resolve(self.seed);
    }
}

/// Concrete model connection & route configuration (`Debug` redacts `api_key`).
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelRouteConfig {
    pub connection_id: String,
    pub endpoint: String,
    pub protocol: ModelProtocol,
    pub model: String,
    #[serde(default)]
    pub api_key: String,
    pub context_window: usize,
    pub max_output_tokens: Option<usize>,
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
    #[serde(default)]
    pub capabilities: ModelCapabilities,
    pub reasoning_effort: Option<String>,
    pub prompt_cache_key: Option<String>,
}

impl std::fmt::Debug for ModelRouteConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModelRouteConfig")
            .field("connection_id", &self.connection_id)
            .field("endpoint", &self.endpoint)
            .field("protocol", &self.protocol)
            .field("model", &self.model)
            .field("api_key", &if self.api_key.is_empty() { "" } else { "[REDACTED]" })
            .field("context_window", &self.context_window)
            .field("max_output_tokens", &self.max_output_tokens)
            .field("capabilities", &self.capabilities)
            .finish()
    }
}

/// Immutable leading system prompt block (never mutated mid-turn).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrozenSystemBlock {
    pub id: String,
    pub text: String,
}

/// Ephemeral per-step runtime reminder (always serialized at the tail as `user`
/// with cache-tail semantics so it NEVER busts the leading system/history KV cache).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeReminder {
    pub tag: String,
    pub text: String,
}

/// Inline media attachment carried by a user or tool message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaAttachment {
    pub mime_type: String,
    pub filename: String,
    pub bytes_base64: String,
    pub sha256: String,
    pub raw_bytes_len: usize,
}

/// Structured tool invocation emitted by the model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelToolCall {
    pub call_id: String,
    pub name: String,
    pub arguments: Value,
}

/// Opaque per-protocol continuation payload (e.g., Anthropic thinking+signature
/// blocks, OpenAI Responses / SIWC encrypted reasoning & output items).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderContinuation {
    pub protocol: ModelProtocol,
    pub model: String,
    pub response_id: Option<String>,
    pub items: Vec<Value>,
}

/// Conversation history message. Structurally CANNOT be `System`!
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "snake_case")]
pub enum HistoryMessage {
    User {
        content: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        media: Vec<MediaAttachment>,
    },
    Assistant {
        content: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reasoning: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        tool_calls: Vec<ModelToolCall>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        continuation: Option<ProviderContinuation>,
    },
    Tool {
        call_id: String,
        tool_name: String,
        payload: ToolModelPayload,
        is_error: bool,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        media: Vec<MediaAttachment>,
    },
}

/// Structured, cache-safe model request passed into `ProviderWireAdapter`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StructuredModelRequest {
    pub frozen_system_prefix: Vec<FrozenSystemBlock>,
    pub prefix_stable_tools: Vec<ToolSchema>,
    pub history: Vec<HistoryMessage>,
    pub ephemeral_tail_reminders: Vec<RuntimeReminder>,
    pub max_output_tokens: usize,
    /// Optional JSON Schema enforcement for `--json-schema` structured output.
    pub json_schema: Option<Value>,
}

/// Canonical finish reason normalized identically across all 4 wire protocols.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelStopReason {
    Stop,
    ToolCalls,
    OutputLimit,
    ContentFilter,
}

/// Normalized token and prompt-cache usage.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    #[serde(default)]
    pub reasoning_tokens: u64,
    #[serde(default)]
    pub usage_reported: bool,
}

impl TokenUsage {
    pub fn cache_hit_rate_bps(&self) -> u32 {
        if self.input_tokens == 0 {
            return 0;
        }
        ((self.cache_read_tokens.saturating_mul(10_000)) / self.input_tokens).min(10_000) as u32
    }

    /// Normalized prompt-cache hit rate in basis points (`0..=10_000`).
    /// Returns `None` when the provider did not report usage (`!self.usage_reported`).
    /// When `input_excludes_cache` is true (e.g. Anthropic Messages API, where
    /// `input_tokens` only counts uncached tokens), divides `cache_read_tokens` by
    /// `input_tokens + cache_read_tokens + cache_write_tokens`.
    pub fn cache_hit_rate_bps_normalized(&self, input_excludes_cache: bool) -> Option<u32> {
        if !self.usage_reported {
            return None;
        }
        let denom = if input_excludes_cache {
            self.input_tokens
                .saturating_add(self.cache_read_tokens)
                .saturating_add(self.cache_write_tokens)
        } else {
            self.input_tokens
        };
        if denom == 0 {
            return Some(0);
        }
        Some(((self.cache_read_tokens.saturating_mul(10_000)) / denom).min(10_000) as u32)
    }
}

/// Normalized response produced by any `ProviderWireAdapter`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NormalizedModelResponse {
    pub response_id: Option<String>,
    pub response_model: Option<String>,
    pub content: String,
    pub reasoning: Option<String>,
    pub tool_calls: Vec<ModelToolCall>,
    pub stop_reason: ModelStopReason,
    pub usage: TokenUsage,
    pub continuation: Option<ProviderContinuation>,
}

/// Incremental streaming event emitted while decoding a model response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum ModelStreamDelta {
    Text(String),
    Reasoning(String),
    ToolCallStart { call_id: String, name: String },
    ToolCallArgsDelta { call_id: String, delta: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_usage_normalized_cache_hit_rate_and_unreported_flag() {
        let unreported = TokenUsage::default();
        assert!(!unreported.usage_reported);
        assert_eq!(unreported.cache_hit_rate_bps_normalized(false), None);
        assert_eq!(unreported.cache_hit_rate_bps_normalized(true), None);

        // Anthropic: input_tokens (100) excludes cache_read (800) and cache_write (100) -> 800 / 1000 = 8000 bps
        let anth = TokenUsage {
            input_tokens: 100,
            output_tokens: 50,
            cache_read_tokens: 800,
            cache_write_tokens: 100,
            reasoning_tokens: 0,
            usage_reported: true,
        };
        assert_eq!(anth.cache_hit_rate_bps_normalized(true), Some(8_000));

        // OpenAI: input_tokens (1000) includes cache_read (750) -> 750 / 1000 = 7500 bps
        let oai = TokenUsage {
            input_tokens: 1_000,
            output_tokens: 50,
            cache_read_tokens: 750,
            cache_write_tokens: 0,
            reasoning_tokens: 20,
            usage_reported: true,
        };
        assert_eq!(oai.cache_hit_rate_bps_normalized(false), Some(7_500));
    }

    #[test]
    fn seed_override_and_generation_params_and_gemini_protocol() {
        let mut params = GenerationParams {
            seed: Some(42),
            stop: vec!["END".into()],
            top_k: Some(40),
            repetition_penalty: Some(1.1),
            thinking_budget: None,
        };
        params.apply_seed_override(SeedOverride::Inherit);
        assert_eq!(params.seed, Some(42));
        params.apply_seed_override(SeedOverride::Clear);
        assert_eq!(params.seed, None);
        params.apply_seed_override(SeedOverride::Value(99));
        assert_eq!(params.seed, Some(99));

        assert_eq!(WireProtocol::parse("gemini"), Some(ModelProtocol::Gemini));
        assert_eq!(ModelProtocol::Gemini.as_str(), "gemini");

        let caps: ModelCapabilities = serde_json::from_str(
            r#"{"reasoning":true,"stream_reasoning":true,"supported_reasoning_levels":["low","high"],"supportsReasoningSummary":true}"#,
        )
        .unwrap();
        assert!(caps.stream_reasoning);
        assert!(caps.supports_reasoning_summary);
        assert_eq!(caps.supported_reasoning_levels, vec!["low", "high"]);
    }
}
