// SPDX-License-Identifier: Apache-2.0
//! Unified 5-protocol LLM wire adapters (`OpenAiChat`, `OpenAiResponses`,
//! `AnthropicMessages`, `SiwcResponses`, `Gemini`), SSE decoding, privacy redaction,
//! and blocking HTTP client with retries and optional audit capture.

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
    adapter_for, finalize_response, format_tail_reminders, BuiltHttpRequest, LlmError,
    ProtocolMediaLimits, ProviderWireAdapter,
};
pub use client::{is_loopback_url, LlmClient};
pub use privacy::{is_local_or_lan_endpoint, PrivacyRedactor};
pub use sse::{SseEvent, SseReader};
