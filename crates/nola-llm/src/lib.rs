// SPDX-License-Identifier: Apache-2.0
//! Unified 5-protocol LLM wire adapters (`OpenAiChat`, `OpenAiResponses`,
//! `AnthropicMessages`, `SiwcResponses`, `Gemini`), bounded SSE decoding with
//! `ReasoningProgressGuard`, full wire privacy redaction, and blocking HTTP
//! client with retries, multi-endpoint fallbacks, and interruptible cancellation.

pub mod adapter;
pub mod anthropic;
pub mod client;
pub mod gemini;
pub mod openai_chat;
pub mod openai_responses;
pub mod privacy;
pub mod siwc;
pub mod sse;

#[cfg(test)]
mod tests;

pub use adapter::{
    adapter_for, audio_format_for_mime, finalize_response, format_tail_reminders,
    is_supported_audio_mime, normalized_wire_reasoning_effort, BuiltHttpRequest, LlmError,
    ProtocolMediaLimits, ProviderWireAdapter,
};
pub use client::{is_loopback_url, CallParams, LlmClient};
pub use privacy::{is_local_or_lan_endpoint, should_redact_pii, PrivacyRedactor, RedactionOptions};
pub use sse::{
    should_stop_repetitive_output, ReasoningProgressGuard, SseEvent, SseReader, MAX_SSE_LINE_BYTES,
    REPETITIVE_REASONING_CODE,
};
