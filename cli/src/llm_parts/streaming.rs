// SPDX-License-Identifier: Apache-2.0
//! Request orchestration, cancellation, delivery, retries and streaming transport.

use super::stream_state::merge_usage_from_envelope;
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader};
use std::net::SocketAddr;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use serde_json::{json, Value};

use crate::execution_context::ExecutionContext;
use crate::i18n::{tr, Message};

use super::contracts::{
    effective_max_output_tokens, resolved_model_protocol, LlmConfig, LlmEvent,
    ProviderAttemptError, ProviderFailure, StreamedChoice, ToolCall, ToolSlot,
};
use super::diagnostics::{
    debug_context_metrics, debug_response_usage, log_local_debug_payload, log_model_response_debug,
    trim_debug_detail,
};
use super::provider_protocols::{
    anthropic_request_body, apply_thinking_policy, detect_chat_provider,
    non_stream_required_tools_enabled, normalize_anthropic_endpoint, normalize_chat_endpoint,
    normalize_openai_compatible_messages, parse_anthropic_stream_event, ChatProvider, OpenAiFlavor,
};
use super::responses::stream_responses;
use super::stream_state::{
    append_choice_text, capture_openai_response_identity, stream_has_progress, stream_progress_size,
};
use super::transport::{
    build_stream_client, classify_network_error_hint, llm_cancelled_message, recv_stream_line,
    resolve_dns_fallbacks_for_host, response_retry_after_ms, send_request_with_cancellation,
    stream_first_event_timeout, stream_idle_timeout, stream_tail_idle_timeout,
    SendRequestResult, StreamLineRecv,
};

#[cfg(test)]
#[path = "streaming_contract_tests.rs"]
mod protocol_contract_tests;

impl ProviderFailure {
    // Cancellation is a locally observed control fact, never a classification
    // of translated display text or a provider response body.
    pub(super) fn cancelled(cfg: &LlmConfig) -> Self {
        Self {
            code: "cancelled".into(),
            message: "The model request was cancelled.".into(),
            diagnostic: "local model cancellation".into(),
            retryable: false,
            same_target_retryable: false,
            fallback_eligible: false,
            exhausts_target: false,
            status: None,
            retry_after_ms: None,
            provider: cfg.provider.clone(),
            model: Some(cfg.model.clone()),
        }
    }

    pub fn display_message(&self) -> String {
        let provider = self
            .provider
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("openai-compatible");
        let model = self
            .model
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("unknown-model");
        format!("{provider}/{model}: {}", self.message)
    }

    fn classify(cfg: &LlmConfig, error: &str) -> Self {
        if error == super::stream_state::REPETITIVE_REASONING {
            return Self {
                code: error.into(),
                message: "The model repeated its reasoning without making progress. Generation was stopped; retry or select another model.".into(),
                diagnostic: error.into(), retryable: false, same_target_retryable: false,
                fallback_eligible: false, exhausts_target: false, status: None, retry_after_ms: None,
                provider: cfg.provider.clone(), model: Some(cfg.model.clone()),
            };
        }
        if error == nola_core::agent::model_tool_batch::MODEL_TOOL_BATCH_LIMIT_EXCEEDED {
            return Self {
                code: error.to_string(),
                message: error.to_string(),
                diagnostic: error.to_string(),
                retryable: false,
                same_target_retryable: false,
                fallback_eligible: false,
                exhausts_target: false,
                status: None,
                retry_after_ms: None,
                provider: cfg.provider.clone(),
                model: Some(cfg.model.clone()),
            };
        }
        let projection = model_failure_projection(error);
        let exhausts_target = structured_provider_error_is_exhausted_quota(error);
        Self {
            code: projection.code.to_string(),
            message: projection.message.to_string(),
            retryable: projection.retryable,
            same_target_retryable: is_retryable_llm_error(error)
                && should_retry_same_provider_candidate(error),
            fallback_eligible: is_fallback_eligible_llm_error(error),
            exhausts_target,
            status: provider_error_status(error),
            retry_after_ms: None,
            provider: cfg.provider.clone(),
            model: Some(cfg.model.clone()),
            diagnostic: trim_debug_detail(error),
        }
    }

    fn from_http(
        cfg: &LlmConfig,
        status: u16,
        retry_after_ms: Option<u64>,
        detail: String,
    ) -> Self {
        let status_label = reqwest::StatusCode::from_u16(status)
            .map(|value| value.to_string())
            .unwrap_or_else(|_| status.to_string());
        let diagnostic = format!(
            "{} ({status_label}): {}",
            tr(Message::LlmRequestFailedPrefix),
            detail.trim()
        );
        let context_limit =
            structured_provider_error_is_context_limit(&detail) == Some(true) || status == 413;
        let exhausts_target = structured_provider_error_is_exhausted_quota(&detail);
        let same_target_retryable = !context_limit
            && !exhausts_target
            && matches!(status, 408 | 409 | 425 | 429 | 500..=599);
        let mut failure = Self::classify(cfg, &diagnostic);
        failure.status = Some(status);
        failure.retry_after_ms = retry_after_ms;
        failure.exhausts_target = exhausts_target;
        failure.same_target_retryable = same_target_retryable;
        failure.fallback_eligible = exhausts_target || same_target_retryable;
        failure.retryable = failure.fallback_eligible;
        failure.diagnostic = trim_debug_detail(&diagnostic);
        failure
    }

    fn consumer(cfg: &LlmConfig, message: String) -> Self {
        Self {
            code: "consumer_rejected".to_string(),
            message: "The model stream consumer rejected an event.".to_string(),
            retryable: false,
            same_target_retryable: false,
            fallback_eligible: false,
            exhausts_target: false,
            status: None,
            retry_after_ms: None,
            provider: cfg.provider.clone(),
            model: Some(cfg.model.clone()),
            diagnostic: trim_debug_detail(&message),
        }
    }
}

impl ProviderAttemptError {
    pub(super) fn into_failure(self, cfg: &LlmConfig) -> ProviderFailure {
        match self {
            Self::Admission(message) => ProviderFailure::consumer(cfg, message),
            Self::WorkerDisconnected => ProviderFailure {
                code: "transport_worker_disconnected".into(),
                message: "The local HTTP request worker ended without a response.".into(),
                diagnostic: "HTTP request worker disconnected before returning a response".into(),
                // This is a local execution failure, not provider overload or
                // a proven unsent request. Do not switch providers blindly.
                retryable: false,
                same_target_retryable: false,
                fallback_eligible: false,
                exhausts_target: false,
                status: None,
                retry_after_ms: None,
                provider: cfg.provider.clone(),
                model: Some(cfg.model.clone()),
            },
            Self::Http {
                status,
                retry_after_ms,
                detail,
            } => ProviderFailure::from_http(cfg, status, retry_after_ms, detail),
            Self::Message(message) => ProviderFailure::classify(cfg, &message),
        }
    }
}

/// Binds a main-turn provider cache identity to Core's canonical model prefix.
/// Callers must pass Core's digest of the stable prompt and exact Step tool
/// projection; rendered messages and tool JSON are intentionally never hashed
/// by the Host.
pub(crate) fn apply_main_turn_prompt_cache_identity(
    cfg: &mut LlmConfig,
    model_prefix_digest: &str,
) {
    if !cfg.openai_prompt_cache
        || cfg
            .openai_prompt_cache_key
            .as_deref()
            .is_some_and(|key| !key.trim().is_empty())
    {
        return;
    }
    let digest = model_prefix_digest.trim().to_ascii_lowercase();
    if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return;
    }
    let provider = prompt_cache_provider_id(cfg);
    let namespace = format!(
        "{}|{}|{}",
        cfg.openai_prompt_cache_key_prefix
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("nola"),
        provider,
        cfg.model.trim()
    );
    // Keep the provider's 64-character key bound while retaining the full
    // digest entropy: namespace a reversible digest word instead of truncating
    // the hash or introducing another hashing dependency.
    let namespace_hash = namespace
        .bytes()
        .fold(0xcbf29ce484222325_u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
        });
    let digest_head = u64::from_str_radix(&digest[..16], 16).unwrap_or_default();
    cfg.openai_prompt_cache_key = Some(format!(
        "{:016x}{}",
        digest_head ^ namespace_hash,
        &digest[16..]
    ));
}

pub(crate) fn prompt_cache_provider_id(cfg: &LlmConfig) -> &'static str {
    match detect_chat_provider(cfg.provider.as_deref(), &cfg.endpoint, &cfg.model) {
        ChatProvider::Anthropic => "anthropic",
        ChatProvider::OpenAiCompatible(OpenAiFlavor::OpenAi) => "openai",
        ChatProvider::OpenAiCompatible(OpenAiFlavor::DashScope) => "dashscope",
        ChatProvider::OpenAiCompatible(OpenAiFlavor::DeepSeek) => "deepseek",
        ChatProvider::OpenAiCompatible(OpenAiFlavor::LlamaCpp) => "llamacpp",
        ChatProvider::OpenAiCompatible(OpenAiFlavor::Vllm) => "vllm",
        ChatProvider::OpenAiCompatible(OpenAiFlavor::Generic) => "openai-compatible",
        ChatProvider::OpenAiCompatible(OpenAiFlavor::OpenRouter) => "openrouter",
    }
}

/// Exact output-limit field emitted by the streaming protocol adapters.
/// Keeping this next to request construction prevents provenance from guessing
/// based on a model name or a dynamic router response.
pub(crate) fn output_token_parameter_name(cfg: &LlmConfig) -> &'static str {
    super::contracts::resolved_output_token_field(
        resolved_model_protocol(cfg.provider.as_deref(), &cfg.endpoint),
        &cfg.endpoint,
        cfg.chat_output_token_field,
    )
    .as_str()
}

pub const OPENAI_COMPATIBLE_CONTINUATION_NAMESPACE: &str = "openai-compatible.chat-completions";

pub fn provider_continuation_namespace(cfg: &LlmConfig) -> &'static str {
    match resolved_model_protocol(cfg.provider.as_deref(), &cfg.endpoint) {
        nola_domain::generation::ModelProtocol::Responses => "openai.responses",
        nola_domain::generation::ModelProtocol::SiwcResponses => "openai.siwc.responses",
        nola_domain::generation::ModelProtocol::Anthropic => "anthropic.messages",
        nola_domain::generation::ModelProtocol::ChatCompletions => {
            OPENAI_COMPATIBLE_CONTINUATION_NAMESPACE
        }
    }
}

fn tool_arguments_text(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::Null => None,
        Value::String(arguments) => {
            (!arguments.trim().is_empty()).then(|| normalize_tool_arguments_text(arguments))
        }
        arguments => serde_json::to_string(arguments).ok(),
    }
}

/// OpenAI-compatible providers are required to return `function.arguments` as
/// JSON text for an object. Some gateways incorrectly JSON-encode that text a
/// second time, producing `"{\"path\":...}"`. Normalize that transport
/// defect once at the model boundary so every tool continues to receive and
/// validate the same object-shaped arguments.
fn normalize_tool_arguments_text(arguments: &str) -> String {
    let mut candidate = arguments.trim().to_string();
    for _ in 0..2 {
        match nola_core::strict_json::decode(candidate.as_bytes()) {
            Ok(Value::String(inner)) if !inner.trim().is_empty() => {
                candidate = inner.trim().to_string();
            }
            Ok(value) => {
                return serde_json::to_string(&value).unwrap_or(candidate);
            }
            // Once an outer JSON string has been decoded, preserve the
            // decoded candidate even when its inner object is incomplete.
            // Returning the original wrapper would turn malformed object
            // arguments into a valid JSON string and misclassify the failure
            // as a root-type mismatch instead of a truncated tool call.
            Err(_) => return candidate,
        }
    }
    candidate
}

/// Converts provider tool arguments into the canonical JSON value consumed by
/// Core. Keeping this conversion at the Host boundary prevents a transport
/// quirk from leaking into permission matching, persisted transcripts, and UI
/// tool details. Core still validates and defensively normalizes older data.
pub(crate) fn normalized_tool_arguments_value(arguments: &str) -> Value {
    let normalized = normalize_tool_arguments_text(arguments);
    nola_core::strict_json::decode(normalized.as_bytes()).unwrap_or(Value::String(normalized))
}

fn tool_argument_fragment_text(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::Null => None,
        // Streaming APIs already carry the JSON argument bytes inside a JSON
        // string. Serializing the Value again adds quotes around every chunk,
        // so two valid fragments become an invalid sequence of JSON strings.
        Value::String(arguments) => Some(arguments.clone()),
        arguments => serde_json::to_string(arguments).ok(),
    }
}

fn append_openai_tool_delta(
    slots: &mut BTreeMap<usize, ToolSlot>,
    tool_delta: &Value,
) -> Result<Option<LlmEvent>, String> {
    let index = tool_delta.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
    nola_core::agent::model_tool_batch::validate_model_tool_batch_size(
        slots.len() + usize::from(!slots.contains_key(&index)),
    )
    .map_err(str::to_string)?;
    let slot = slots.entry(index).or_default();
    if let Some(id) = tool_delta.get("id").and_then(Value::as_str) {
        if !id.is_empty() {
            if !slot.id.is_empty() && slot.id != id {
                return Err(format!(
                    "provider changed tool call identity at index {index}: {} -> {id}",
                    slot.id
                ));
            }
            slot.id = id.to_string();
        }
    }
    let mut arguments_delta = String::new();
    if let Some(function) = tool_delta.get("function") {
        if let Some(name) = function.get("name").and_then(Value::as_str) {
            slot.name.push_str(name);
        }
        if let Some(arguments) = tool_argument_fragment_text(function.get("arguments")) {
            slot.arguments.push_str(&arguments);
            arguments_delta = arguments;
        }
    }
    Ok(
        (!slot.id.is_empty() && !slot.name.is_empty()).then(|| LlmEvent::ToolCallDelta {
            call_id: slot.id.clone(),
            name: slot.name.clone(),
            arguments_delta,
        }),
    )
}

pub fn stream_chat<F>(
    cfg: &LlmConfig,
    messages: &[Value],
    tools: &[Value],
    tool_choice: &str,
    mut on_event: F,
) -> Result<StreamedChoice, String>
where
    F: FnMut(LlmEvent),
{
    stream_chat_checked(cfg, messages, tools, tool_choice, |event| {
        on_event(event);
        Ok(())
    })
    .map_err(|failure| failure.diagnostic)
}

enum ModelDelivery {
    Text(LlmEvent),
    Barrier(LlmEvent, mpsc::SyncSender<Result<(), String>>),
}

// Bound queued text to at most 1 MiB. A slow durable writer must not force the
// network reader to present every transport token as a separate transaction.
const MODEL_DELIVERY_CHUNK_BYTES: usize = 4096;
const MODEL_DELIVERY_QUEUE_CAPACITY: usize = 256;
const MODEL_DELIVERY_BATCH_BYTES: usize = 16 * 1024;

/// Local consumer scheduling only. Idle ticks are not provider events and
/// never become Core/Host events unless the presentation owner has real text
/// whose deadline is due.
pub(crate) enum ModelPresentationNotification {
    Provider(LlmEvent),
    Idle,
}

fn deliver_presentation_notification(
    notification: ModelPresentationNotification,
    cancellation: &ExecutionContext,
    locally_cancelled: &mut bool,
    on_event: &mut impl FnMut(ModelPresentationNotification) -> Result<(), String>,
) -> Result<(), String> {
    if matches!(&notification, ModelPresentationNotification::Idle) && cancellation.is_requested() {
        *locally_cancelled = true;
        return Err(llm_cancelled_message());
    }
    on_event(notification)
}

fn consume_model_deliveries(
    receiver: &mpsc::Receiver<ModelDelivery>,
    on_event: &mut impl FnMut(LlmEvent) -> Result<(), String>,
) -> Result<(), String> {
    consume_model_deliveries_with_ticks(receiver, None, &mut |notification| match notification {
        ModelPresentationNotification::Provider(event) => on_event(event),
        ModelPresentationNotification::Idle => Ok(()),
    })
}

fn consume_model_deliveries_with_ticks(
    receiver: &mpsc::Receiver<ModelDelivery>,
    idle_cadence: Option<Duration>,
    on_event: &mut impl FnMut(ModelPresentationNotification) -> Result<(), String>,
) -> Result<(), String> {
    let mut pending = None;
    loop {
        let delivery = match pending.take() {
            Some(delivery) => delivery,
            None => match idle_cadence {
                Some(cadence) => match receiver.recv_timeout(cadence) {
                    Ok(delivery) => delivery,
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        on_event(ModelPresentationNotification::Idle)?;
                        continue;
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
                },
                None => match receiver.recv() {
                    Ok(delivery) => delivery,
                    Err(_) => return Ok(()),
                },
            },
        };
        match delivery {
            ModelDelivery::Barrier(event, acknowledgement) => {
                let result = on_event(ModelPresentationNotification::Provider(event));
                let _ = acknowledgement.send(result.clone());
                result?;
            }
            ModelDelivery::Text(mut event) => {
                // Drain only data already available, never wait to form a
                // batch. First text and slow providers remain immediate.
                while let Ok(next) = receiver.try_recv() {
                    let merged = match (&mut event, &next) {
                        (
                            LlmEvent::TextDelta(text),
                            ModelDelivery::Text(LlmEvent::TextDelta(delta)),
                        )
                        | (
                            LlmEvent::ReasoningDelta(text),
                            ModelDelivery::Text(LlmEvent::ReasoningDelta(delta)),
                        ) if text.len() + delta.len() <= MODEL_DELIVERY_BATCH_BYTES => {
                            text.push_str(delta);
                            true
                        }
                        _ => false,
                    };
                    if !merged {
                        pending = Some(next);
                        break;
                    }
                }
                on_event(ModelPresentationNotification::Provider(event))?;
            }
        }
    }
}

#[cfg(test)]
mod model_delivery_tests {
    use super::*;

    #[test]
    fn requested_cancellation_prevents_idle_from_flushing_the_paused_tail() {
        let cancellation = ExecutionContext::default();
        let (sender, receiver) = mpsc::sync_channel(2);
        sender
            .send(ModelDelivery::Text(LlmEvent::TextDelta(
                "pending tail".into(),
            )))
            .unwrap();
        let mut locally_cancelled = false;
        let mut callbacks = 0;
        let result = consume_model_deliveries_with_ticks(
            &receiver,
            Some(Duration::from_millis(1)),
            &mut |notification| {
                deliver_presentation_notification(
                    notification,
                    &cancellation,
                    &mut locally_cancelled,
                    &mut |notification| {
                        callbacks += 1;
                        assert!(matches!(
                            notification,
                            ModelPresentationNotification::Provider(LlmEvent::TextDelta(_))
                        ));
                        // Cancel before the first local presentation deadline. There
                        // are no additional provider callbacks to discover the token.
                        cancellation.request();
                        Ok(())
                    },
                )
            },
        );
        assert_eq!(callbacks, 1, "cancelled Idle reached the tail owner");
        assert!(locally_cancelled);
        assert_eq!(result.unwrap_err(), llm_cancelled_message());
        drop(sender);
    }

    #[test]
    fn idle_consumer_rejection_is_distinct_from_local_cancellation() {
        let mut locally_cancelled = false;
        let result = deliver_presentation_notification(
            ModelPresentationNotification::Idle,
            &ExecutionContext::default(),
            &mut locally_cancelled,
            &mut |_| Err("durable consumer rejected presentation".into()),
        );
        assert!(!locally_cancelled);
        assert_eq!(
            result.unwrap_err(),
            "durable consumer rejected presentation"
        );
    }

    #[test]
    fn paused_consumer_gets_local_ticks_without_fabricated_provider_deltas() {
        let (sender, receiver) = mpsc::sync_channel(2);
        sender
            .send(ModelDelivery::Text(LlmEvent::TextDelta(
                "paused phrase".into(),
            )))
            .unwrap();
        let mut text = String::new();
        let mut ticks = 0;
        let result = consume_model_deliveries_with_ticks(
            &receiver,
            Some(Duration::from_millis(1)),
            &mut |notification| match notification {
                ModelPresentationNotification::Provider(LlmEvent::TextDelta(delta)) => {
                    text.push_str(&delta);
                    Ok(())
                }
                ModelPresentationNotification::Provider(_) => panic!("unexpected provider event"),
                ModelPresentationNotification::Idle => {
                    ticks += 1;
                    Err("stop local presentation consumer".into())
                }
            },
        );
        assert_eq!(text, "paused phrase");
        assert_eq!(ticks, 1);
        assert_eq!(result.unwrap_err(), "stop local presentation consumer");
        drop(sender);
    }

    #[test]
    fn local_ticks_do_not_overtake_the_queued_attempt_barrier() {
        let (sender, receiver) = mpsc::sync_channel(3);
        sender
            .send(ModelDelivery::Text(LlmEvent::TextDelta("before".into())))
            .unwrap();
        let (acknowledgement, accepted) = mpsc::sync_channel(1);
        sender
            .send(ModelDelivery::Barrier(
                LlmEvent::SamplingStarted {
                    provider: "next-provider".into(),
                    model: "next-model".into(),
                },
                acknowledgement,
            ))
            .unwrap();
        let mut order = Vec::new();
        let result = consume_model_deliveries_with_ticks(
            &receiver,
            Some(Duration::from_millis(1)),
            &mut |notification| {
                match notification {
                    ModelPresentationNotification::Provider(LlmEvent::TextDelta(_)) => {
                        order.push("text")
                    }
                    ModelPresentationNotification::Provider(LlmEvent::SamplingStarted {
                        ..
                    }) => order.push("attempt"),
                    ModelPresentationNotification::Provider(_) => {
                        panic!("unexpected provider event")
                    }
                    ModelPresentationNotification::Idle => {
                        order.push("tick");
                        return Err("stop".into());
                    }
                }
                Ok(())
            },
        );
        assert_eq!(order, vec!["text", "attempt", "tick"]);
        assert_eq!(accepted.try_recv().unwrap(), Ok(()));
        assert_eq!(result.unwrap_err(), "stop");
        drop(sender);
    }

    #[test]
    fn queued_text_is_coalesced_without_crossing_channels_or_control_barriers() {
        let (sender, receiver) = mpsc::sync_channel(256);
        for _ in 0..100 {
            sender
                .send(ModelDelivery::Text(LlmEvent::TextDelta("界".into())))
                .unwrap();
        }
        sender
            .send(ModelDelivery::Text(LlmEvent::ReasoningDelta(
                "reasoning".into(),
            )))
            .unwrap();
        let (ack, accepted) = mpsc::sync_channel(1);
        sender
            .send(ModelDelivery::Barrier(
                LlmEvent::SamplingStarted {
                    provider: "fixture".into(),
                    model: "fixture".into(),
                },
                ack,
            ))
            .unwrap();
        sender
            .send(ModelDelivery::Text(LlmEvent::TextDelta("after".into())))
            .unwrap();
        drop(sender);
        let mut events = Vec::new();
        consume_model_deliveries(&receiver, &mut |event| {
            events.push(event);
            Ok(())
        })
        .unwrap();
        assert_eq!(events.len(), 4);
        assert!(matches!(&events[0], LlmEvent::TextDelta(text) if text == &"界".repeat(100)));
        assert!(matches!(&events[1], LlmEvent::ReasoningDelta(text) if text == "reasoning"));
        assert!(matches!(&events[2], LlmEvent::SamplingStarted { .. }));
        assert!(matches!(&events[3], LlmEvent::TextDelta(text) if text == "after"));
        assert_eq!(accepted.recv().unwrap(), Ok(()));
    }

    #[test]
    fn rejected_text_drops_the_following_control_barrier_without_acknowledging_it() {
        let (sender, receiver) = mpsc::sync_channel(4);
        sender
            .send(ModelDelivery::Text(LlmEvent::TextDelta("preview".into())))
            .unwrap();
        let (ack, accepted) = mpsc::sync_channel(1);
        sender
            .send(ModelDelivery::Barrier(
                LlmEvent::SamplingStarted {
                    provider: "next".into(),
                    model: "next".into(),
                },
                ack,
            ))
            .unwrap();
        drop(sender);
        let result = consume_model_deliveries(&receiver, &mut |_| Err("durable rejection".into()));
        assert_eq!(result.unwrap_err(), "durable rejection");
        assert!(accepted.recv().is_err());
    }

    #[test]
    fn coalesced_batches_stay_bounded_and_lossless() {
        let (sender, receiver) = mpsc::sync_channel(32);
        let chunk = "界".repeat(1024);
        for _ in 0..20 {
            sender
                .send(ModelDelivery::Text(LlmEvent::TextDelta(chunk.clone())))
                .unwrap();
        }
        drop(sender);
        let mut text = String::new();
        let mut batches = 0;
        consume_model_deliveries(&receiver, &mut |event| {
            let LlmEvent::TextDelta(delta) = event else {
                panic!("unexpected event")
            };
            assert!(delta.len() <= MODEL_DELIVERY_BATCH_BYTES);
            text.push_str(&delta);
            batches += 1;
            Ok(())
        })
        .unwrap();
        assert_eq!(text, chunk.repeat(20));
        assert_eq!(batches, 4);
    }

    #[test]
    fn slow_durable_consumer_gets_immediate_first_text_then_one_backlog_batch() {
        let (sender, receiver) = mpsc::sync_channel(256);
        let (first_visible, first_accepted) = mpsc::sync_channel(1);
        let (burst_ready, burst_queued) = mpsc::sync_channel(1);
        let producer = thread::spawn(move || {
            sender
                .send(ModelDelivery::Text(LlmEvent::TextDelta("first".into())))
                .unwrap();
            first_accepted.recv().unwrap();
            for _ in 0..100 {
                sender
                    .send(ModelDelivery::Text(LlmEvent::TextDelta("界".into())))
                    .unwrap();
            }
            burst_ready.send(()).unwrap();
        });
        let mut deltas = Vec::new();
        consume_model_deliveries(&receiver, &mut |event| {
            let LlmEvent::TextDelta(delta) = event else {
                panic!("unexpected event")
            };
            deltas.push(delta);
            if deltas.len() == 1 {
                first_visible.send(()).unwrap();
                // Simulate a durable transaction while transport keeps filling
                // the bounded queue, without timing-sensitive sleeps.
                burst_queued.recv().unwrap();
            }
            Ok(())
        })
        .unwrap();
        producer.join().unwrap();
        assert_eq!(deltas, vec!["first".to_string(), "界".repeat(100)]);
    }
}

/// Text is losslessly coalesced across a bounded producer/consumer queue.
/// Control events remain synchronous durable barriers: in particular a retry
/// cannot start another physical request before its acknowledgement. Consumer
/// rejection cancels the producer and never enters provider fallback.
pub fn stream_chat_checked<F>(
    cfg: &LlmConfig,
    messages: &[Value],
    tools: &[Value],
    tool_choice: &str,
    mut on_event: F,
) -> Result<StreamedChoice, ProviderFailure>
where
    F: FnMut(LlmEvent) -> Result<(), String>,
{
    stream_chat_checked_notifications(cfg, messages, tools, tool_choice, None, |notification| {
        match notification {
            ModelPresentationNotification::Provider(event) => on_event(event),
            ModelPresentationNotification::Idle => Ok(()),
        }
    })
}

pub(crate) fn stream_chat_checked_with_presentation_ticks<F>(
    cfg: &LlmConfig,
    messages: &[Value],
    tools: &[Value],
    tool_choice: &str,
    on_event: F,
) -> Result<StreamedChoice, ProviderFailure>
where
    F: FnMut(ModelPresentationNotification) -> Result<(), String>,
{
    stream_chat_checked_notifications(
        cfg,
        messages,
        tools,
        tool_choice,
        Some(Duration::from_millis(25)),
        on_event,
    )
}

fn stream_chat_checked_notifications<F>(
    cfg: &LlmConfig,
    messages: &[Value],
    tools: &[Value],
    tool_choice: &str,
    idle_cadence: Option<Duration>,
    mut on_event: F,
) -> Result<StreamedChoice, ProviderFailure>
where
    F: FnMut(ModelPresentationNotification) -> Result<(), String>,
{
    let _execution_scope = cfg.cancellation.enter();
    let mut producer_config = cfg.clone();
    producer_config.cancellation = cfg.cancellation.child();
    let producer_cancellation = producer_config.cancellation.clone();
    let measurement_context = nola_measurement::context();
    let measurement_handoff = nola_measurement::enabled().then(||
        std::sync::Arc::new(std::sync::Mutex::new(None::<nola_measurement::Context>)));
    thread::scope(|scope| {
        let (sender, receiver) = mpsc::sync_channel(MODEL_DELIVERY_QUEUE_CAPACITY);
        let producer_measurement_handoff = measurement_handoff.clone();
        let producer = scope.spawn(move || {
            let _measurement_scope = nola_measurement::enter(measurement_context);
            stream_chat_checked_serial(&producer_config, messages, tools, tool_choice, |event| {
                let reasoning = matches!(&event, LlmEvent::ReasoningDelta(_));
                match event {
                    LlmEvent::TextDelta(text) | LlmEvent::ReasoningDelta(text) => {
                        // Preserve channel identity while splitting oversized
                        // provider frames at UTF-8 boundaries.
                        let mut remaining = text.as_str();
                        while !remaining.is_empty() {
                            let mut end = remaining.len().min(MODEL_DELIVERY_CHUNK_BYTES);
                            while !remaining.is_char_boundary(end) {
                                end -= 1;
                            }
                            let chunk = remaining[..end].to_string();
                            sender
                                .send(ModelDelivery::Text(if reasoning {
                                    LlmEvent::ReasoningDelta(chunk)
                                } else {
                                    LlmEvent::TextDelta(chunk)
                                }))
                                .map_err(|_| "model consumer closed".to_string())?;
                            remaining = &remaining[end..];
                        }
                        Ok(())
                    }
                    event => {
                        let sampling = matches!(&event, LlmEvent::SamplingStarted { .. });
                        let (acknowledgement, accepted) = mpsc::sync_channel(1);
                        sender
                            .send(ModelDelivery::Barrier(event, acknowledgement))
                            .map_err(|_| "model consumer closed".to_string())?;
                        let result = accepted
                            .recv()
                            .unwrap_or_else(|_| Err("model consumer closed".to_string()));
                        if sampling {
                            if let Some(handoff) = &producer_measurement_handoff {
                                if let Ok(context) = handoff.try_lock() {
                                    if let Some(context) = context.as_ref() { nola_measurement::inherit_attempt(context); }
                                }
                            }
                        }
                        result
                    }
                }
            })
        });
        let mut locally_cancelled = false;
        let accepted =
            consume_model_deliveries_with_ticks(&receiver, idle_cadence, &mut |notification| {
                let sampling = matches!(&notification,
                    ModelPresentationNotification::Provider(LlmEvent::SamplingStarted { .. }));
                let result = deliver_presentation_notification(
                    notification,
                    &cfg.cancellation,
                    &mut locally_cancelled,
                    &mut on_event,
                );
                if sampling {
                    if let Some(handoff) = &measurement_handoff {
                        if let Ok(mut context) = handoff.try_lock() { *context = Some(nola_measurement::context()); }
                    }
                }
                result
            });
        if accepted.is_err() {
            producer_cancellation.request();
        }
        drop(receiver);
        let result = producer
            .join()
            .map_err(|_| ProviderFailure::consumer(cfg, "model producer panicked".to_string()))?;
        if locally_cancelled {
            // Cooperative cancellation is local control, not consumer
            // rejection or a provider failure eligible for retry/fallback.
            return Err(ProviderFailure::cancelled(cfg));
        }
        accepted.map_err(|error| ProviderFailure::consumer(cfg, error))?;
        result
    })
}

fn stream_chat_checked_serial<F>(
    cfg: &LlmConfig,
    messages: &[Value],
    tools: &[Value],
    tool_choice: &str,
    mut on_event: F,
) -> Result<StreamedChoice, ProviderFailure>
where
    F: FnMut(LlmEvent) -> Result<(), String>,
{
    let _execution_scope = cfg.cancellation.enter();
    // Terminal input belongs to the enclosing Host turn (or initialization
    // command). Provider workers only observe its shared cancellation token.
    if (cfg.siwc.is_some() || resolved_model_protocol(cfg.provider.as_deref(), &cfg.endpoint)
        == nola_domain::generation::ModelProtocol::SiwcResponses) && !cfg.fallbacks.is_empty() {
        return Err(ProviderFailure::consumer(cfg, "SIWC does not permit provider fallback".into()));
    }
    let candidates = super::contracts::admitted_provider_targets(cfg);

    let candidate_count = candidates.len();
    for (candidate_index, candidate) in candidates.into_iter().enumerate() {
        let mut published_model_output = false;
        let mut admission_error = None;
        let result = stream_chat_single(&candidate, messages, tools, tool_choice, |event| {
            if matches!(event, LlmEvent::TextDelta(_) | LlmEvent::ReasoningDelta(_)) {
                published_model_output = true;
            }
            let result = on_event(event);
            if let Err(error) = &result {
                admission_error = Some(error.clone());
            }
            result
        });
        if let Some(error) = admission_error {
            return Err(ProviderFailure::consumer(&candidate, error));
        }
        match result {
            Ok(mut choice) => {
                choice.provider_namespace =
                    Some(provider_continuation_namespace(&candidate).to_string());
                choice.provider_model = Some(candidate.model.clone());
                return Ok(choice);
            }
            Err(error)
                if !published_model_output
                    && candidate_index + 1 < candidate_count
                    && error.fallback_eligible =>
            {
                let next_index = candidate_index + 2;
                let failure = error;
                log_local_debug_payload(
                    "llm_provider_fallback",
                    &json!({
                        "type": "llm_provider_fallback",
                        "failed_candidate": candidate_index + 1,
                        "next_candidate": next_index,
                        "candidate_count": candidate_count,
                        "failure": failure,
                    }),
                );
                on_event(LlmEvent::Retrying {
                    attempt: next_index,
                    delay_ms: 0,
                    failure,
                    target_exhausted: true,
                })
                .map_err(|error| ProviderFailure::consumer(&candidate, error))?;
            }
            Err(error) => return Err(error),
        }
    }
    Err(ProviderFailure::classify(
        cfg,
        "LLM provider fallback chain ended unexpectedly",
    ))
}

fn stream_chat_single<F>(
    cfg: &LlmConfig,
    messages: &[Value],
    tools: &[Value],
    tool_choice: &str,
    mut on_event: F,
) -> Result<StreamedChoice, ProviderFailure>
where
    F: FnMut(LlmEvent) -> Result<(), String>,
{
    if cfg.native_model_route.is_some()
        && resolved_model_protocol(cfg.provider.as_deref(), &cfg.endpoint)
            != nola_domain::generation::ModelProtocol::ChatCompletions {
        return Err(ProviderFailure::consumer(cfg, "native MLX channel requires its Chat Completions adapter".into()));
    }
    let max_retries = if resolved_model_protocol(cfg.provider.as_deref(), &cfg.endpoint)
        == nola_domain::generation::ModelProtocol::SiwcResponses { 0 } else { cfg.max_retries.min(6) };
    for retry_index in 0..=max_retries {
        let mut measured_text = false;
        let mut measured_reasoning = false;
        use sha2::Digest;
        let target = format!(
            "{}:{}",
            provider_continuation_namespace(cfg),
            format!(
                "{:x}",
                sha2::Sha256::digest(cfg.endpoint.trim().trim_end_matches('/').as_bytes())
            )
        );
        on_event(LlmEvent::SamplingStarted {
            provider: target,
            model: cfg.model.clone(),
        })
        .map_err(|error| ProviderFailure::consumer(cfg, error))?;
        let measurement = nola_measurement::Span::start(nola_measurement::Stage::ProviderSampling);
        let _measurement_scope = nola_measurement::enter(measurement.child_context());
        let mut admission_error = None;
        let attempt_started_at = Instant::now();
        let mut emitted_provider_event = false;
        let mut emitted_nontransactional_event = false;
        let mut emitted_tool_transport_event = false;
        let mut reasoning_guard = super::stream_state::ReasoningProgressGuard::default();
        let result = {
            let mut forward = |event| {
                reasoning_guard.observe(&event)?;
                match &event {
                    LlmEvent::TextDelta(text) if !measured_text && !text.is_empty() => {
                        measured_text = true;
                        nola_measurement::point(nola_measurement::Stage::AdapterFirstText);
                    }
                    LlmEvent::ReasoningDelta(text) if !measured_reasoning && !text.is_empty() => {
                        measured_reasoning = true;
                        nola_measurement::point(nola_measurement::Stage::AdapterFirstReasoning);
                    }
                    _ => {}
                }
                if !emitted_provider_event {
                    let elapsed_ms = attempt_started_at
                        .elapsed()
                        .as_millis()
                        .min(u64::MAX as u128) as u64;
                    if let Err(error) = on_event(LlmEvent::StageTiming {
                        stage: "provider_first_event",
                        elapsed_ms,
                    }) {
                        admission_error = Some(error.clone());
                        return Err(error);
                    }
                }
                emitted_provider_event = true;
                if matches!(event, LlmEvent::TextDelta(_) | LlmEvent::ReasoningDelta(_)) {
                    emitted_nontransactional_event = true;
                }
                if matches!(event, LlmEvent::ToolCallDelta { .. }) {
                    emitted_tool_transport_event = true;
                }
                let result = on_event(event);
                if let Err(error) = &result {
                    admission_error = Some(error.clone());
                }
                result
            };
            match resolved_model_protocol(cfg.provider.as_deref(), &cfg.endpoint) {
                nola_domain::generation::ModelProtocol::Anthropic => {
                    stream_anthropic_chat(cfg, messages, tools, tool_choice, &mut forward)
                }
                nola_domain::generation::ModelProtocol::Responses | nola_domain::generation::ModelProtocol::SiwcResponses => {
                    stream_responses(cfg, messages, tools, tool_choice, &mut forward)
                }
                nola_domain::generation::ModelProtocol::ChatCompletions => {
                    stream_openai_compatible_chat(cfg, messages, tools, tool_choice, &mut forward)
                }
            }
        };
        if let Some(error) = admission_error {
            measurement.finish(nola_measurement::Outcome::Rejected);
            return Err(ProviderFailure::consumer(cfg, error));
        }
        if cfg.cancellation.is_requested() {
            measurement.finish(nola_measurement::Outcome::Cancelled);
            return Err(ProviderFailure::cancelled(cfg));
        }
        match result {
            Ok(choice) => {
                measurement.finish(choice.sampling_measurement_outcome());
                log_model_response_debug(cfg, &choice);
                return Ok(choice);
            }
            Err(err) => {
                measurement.finish(nola_measurement::Outcome::Failed);
                let failure = err.into_failure(cfg);
                // Tool-capable rounds are transactional: callers do not
                // execute a tool until ModelCompleted supplies the canonical
                // calls. A truncated SSE may therefore be safely resampled
                // even after reasoning/tool argument fragments were observed.
                // Tool-free answer streams remain conservative because their
                // text may already be visible to the user.
                let retryable_round = should_retry_incomplete_stream_round(
                    emitted_provider_event,
                    emitted_nontransactional_event,
                    emitted_tool_transport_event,
                    !tools.is_empty(),
                );
                let Some(delay) =
                    (retryable_round && retry_index < max_retries && failure.same_target_retryable)
                        .then(|| llm_retry_delay(&failure, retry_index + 1, cfg))
                        .flatten()
                else {
                    return Err(failure);
                };
                let delay_ms = delay.as_millis().min(u64::MAX as u128) as u64;
                log_local_debug_payload(
                    "llm_retry",
                    &json!({
                        "type": "llm_retry",
                        "attempt": retry_index + 1,
                        "max_retries": max_retries,
                        "delay_ms": delay_ms,
                        "failure": failure,
                    }),
                );
                on_event(LlmEvent::Retrying {
                    attempt: retry_index + 1,
                    delay_ms,
                    failure,
                    target_exhausted: false,
                })
                .map_err(|error| ProviderFailure::consumer(cfg, error))?;
                wait_for_llm_retry(delay, &cfg.cancellation)
                    .map_err(|error| ProviderFailure::classify(cfg, &error))?;
            }
        }
    }
    Err(ProviderFailure::classify(
        cfg,
        "LLM retry loop ended unexpectedly",
    ))
}

fn should_retry_incomplete_stream_round(
    emitted_provider_event: bool,
    emitted_nontransactional_event: bool,
    emitted_tool_transport_event: bool,
    has_tool_catalog: bool,
) -> bool {
    // Once a tool delta exists, the round is transactional regardless of an
    // earlier reasoning preamble: no tool executes until the complete call is
    // committed. Discarding partial arguments and resampling is safer than
    // failing a long task. Merely sending a tool catalog is not enough to make
    // already-visible answer text transactional.
    emitted_tool_transport_event
        || (!emitted_nontransactional_event && (!emitted_provider_event || has_tool_catalog))
}

fn finalize_stream_completion(
    choice: &mut StreamedChoice,
    saw_terminal_event: bool,
) -> Result<(), String> {
    if !saw_terminal_event && !choice.tool_calls.is_empty() {
        choice.tool_calls.clear();
        return Err(
            "incomplete read: LLM stream closed before the provider completed a tool call; partial tool arguments were discarded"
                .to_string(),
        );
    }
    if choice.finish_reason.is_empty() {
        choice.finish_reason = if !saw_terminal_event {
            "stream_incomplete"
        } else if choice.tool_calls.is_empty() {
            "stop"
        } else {
            "tool_calls"
        }
        .to_string();
    }
    Ok(())
}

pub(crate) fn configured_llm_max_retries() -> usize {
    std::env::var("NOLA_LLM_MAX_RETRIES")
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(default_llm_max_retries())
        .min(6)
}

pub(crate) const fn default_llm_max_retries() -> usize {
    2
}

/// Stable, bounded failure information that may cross the Host/Application
/// boundary. Raw provider bodies remain diagnostic trace data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ModelFailureProjection {
    pub code: &'static str,
    pub message: &'static str,
    pub retryable: bool,
}

pub(crate) fn model_failure_projection(error: &str) -> ModelFailureProjection {
    let status = provider_error_status(error);
    let (code, message) =
        if structured_provider_error_is_context_limit(error) == Some(true) || status == Some(413) {
            (
                "context_length_exceeded",
                "The model context window was exceeded.",
            )
        } else if structured_provider_error_is_invalid_media(error) {
            (
                "invalid_media_input",
                "The provider rejected an image or file. Verify its content and the model's media support before retrying.",
            )
        } else if structured_provider_error_is_exhausted_quota(error) {
            ("quota_exhausted", "The provider quota is exhausted.")
        } else if error == "incomplete read: LLM stream closed before the provider completed a tool call; partial tool arguments were discarded" {
            ("tool_transaction_incomplete", "The model stream ended before its tool calls completed. No partial tool call was executed.")
        } else if error == "Responses incomplete: content_filter" || structured_provider_error_is_content_filter(error) {
            ("content_filtered", "The provider stopped this response because of its content policy.")
        } else if error == "Responses incomplete: unknown_reason" {
            ("responses_incomplete", "The provider stopped this response without a supported completion reason.")
        } else if error == "Responses interrupted with uncommitted tool calls"
            || error == "Responses output limit with uncommitted tool calls" {
            ("responses_tool_transaction_incomplete", "The provider did not finish its tool calls. No tool from this response was executed.")
        } else if error == "Responses stream timed out" {
            ("responses_timeout", "The provider stopped sending response events before completion.")
        } else if error == "Responses stream transport read failed" {
            ("responses_transport_error", "The response connection failed before usable output arrived.")
        } else if error == "Responses provider reported failure" {
            ("responses_provider_failed", "The provider reported a failed response.")
        } else if error.starts_with("Responses stream ended without response.completed")
            || error.starts_with("Responses did not complete:")
        {
            (
                "responses_incomplete",
                "The provider did not complete its Responses stream.",
            )
        } else if error.starts_with("invalid Responses stream:")
            || error.starts_with("invalid Responses completed envelope")
            || error.starts_with("Responses function call")
        {
            (
                "responses_protocol_error",
                "The provider sent an invalid Responses stream.",
            )
        } else {
            match status {
                Some(401 | 403) => (
                    "authentication_failed",
                    "The provider rejected its credentials.",
                ),
                Some(404) => (
                    "model_unavailable",
                    "The requested model or endpoint is unavailable.",
                ),
                Some(400) => (
                    "invalid_model_request",
                    "The provider rejected the model request.",
                ),
                Some(429) => ("rate_limited", "The provider rate limit was reached."),
                Some(500..=599) => (
                    "provider_unavailable",
                    "The provider is temporarily unavailable.",
                ),
                _ => ("provider_failed", "The model request failed."),
            }
        };
    ModelFailureProjection {
        code,
        message,
        retryable: is_retryable_llm_error(error)
            || structured_provider_error_is_exhausted_quota(error),
    }
}

fn provider_error_status(error: &str) -> Option<u16> {
    direct_stream_provider_error_status(error).or_else(|| {
        [400u16, 401, 403, 404, 408, 409, 413, 425, 429]
            .into_iter()
            .chain(500u16..=599)
            .find(|status| error.contains(&format!("({status}")))
    })
}

/// Some compatible gateways serialize their own error envelope into assistant
/// content. Recognize only the complete reserved wrapper, never prose containing
/// an error word or a quoted/code-fenced JSON example. Preserve its typed error.
fn reject_wrapped_gateway_error(choice: &StreamedChoice) -> Result<(), ProviderAttemptError> {
    if !choice.tool_calls.is_empty() || !choice.reasoning.is_empty() || choice.content.len()>32768 {
        return Ok(());
    }
    let Ok(wrapper)=serde_json::from_str::<Value>(&choice.content) else {return Ok(());};
    let Some(object)=wrapper.as_object().filter(|object|object.len()==1) else {return Ok(());};
    let Some(message)=object.get("message").and_then(Value::as_str) else {return Ok(());};
    let Some(message)=message.strip_prefix("Internal service error: ") else {return Ok(());};
    let Some((status,body))=message.split_once(" - ") else {return Ok(());};
    let Ok(status)=status.parse::<u16>() else {return Ok(());};
    if !(400..=599).contains(&status) {return Ok(());}
    let parsed=serde_json::from_str::<Value>(body).or_else(|_| {
        // The observed gateway double-escapes the upstream JSON object.
        let unescaped:String=serde_json::from_str(&format!("\"{body}\""))?;
        serde_json::from_str(&unescaped)
    });
    let Ok(parsed)=parsed else {return Ok(());};
    let Some(envelope)=parsed.as_object() else {return Ok(());};
    if envelope.keys().any(|key|key!="error" && key!="id") {return Ok(());}
    let Some(error)=envelope.get("error").and_then(Value::as_object) else {return Ok(());};
    if !error.get("message").is_some_and(Value::is_string)
        || !["code","type"].iter().any(|key|error.get(*key).and_then(Value::as_str).is_some_and(|s|!s.is_empty())) {
        return Ok(());
    }
    Err(ProviderAttemptError::Http { status, retry_after_ms:None, detail:parsed.to_string() })
}

/// Hold only a possible gateway envelope until its complete shape is known.
/// Ordinary text is released on the first mismatching fragment; memory is bounded.
#[derive(Default)]
struct GatewayEnvelopeBuffer {
    pending: String,
    released: bool,
}

impl GatewayEnvelopeBuffer {
    fn push(&mut self, text: &str) -> Option<String> {
        if self.released { return Some(text.to_owned()); }
        self.pending.push_str(text);
        let mut remaining = self.pending.as_str();
        let mut possible = true;
        for token in ["{", "\"message\"", ":", "\"Internal service error: "] {
            remaining = remaining.trim_start_matches([' ', '\n', '\r', '\t']);
            if token.starts_with(remaining) { break; }
            if let Some(rest) = remaining.strip_prefix(token) {
                remaining = rest;
            } else {
                possible = false;
                break;
            }
        }
        if possible && self.pending.len() <= 32768 { return None; }
        self.released = true;
        Some(std::mem::take(&mut self.pending))
    }
}

#[cfg(test)]
mod wrapped_gateway_error_tests {
    use super::*;
    #[test]
    fn gateway_buffer_releases_normal_text_and_bounds_candidates() {
        let mut buffer = GatewayEnvelopeBuffer::default();
        assert_eq!(buffer.push("你好"), Some("你好".into()));
        assert_eq!(buffer.push("继续"), Some("继续".into()));
        let mut buffer = GatewayEnvelopeBuffer::default();
        assert_eq!(buffer.push("{\n \"message\" : \""), None);
        assert_eq!(buffer.push("hello\"}"), Some("{\n \"message\" : \"hello\"}".into()));
        let mut buffer = GatewayEnvelopeBuffer::default();
        assert!(buffer.push("{\"message\":\"Internal service error: ").is_none());
        assert!(buffer.push(&"x".repeat(32769)).is_some());
    }
    #[test]
    fn content_filter_projection_uses_typed_error_not_message_words() {
        let projection = model_failure_projection(r#"LLM error (400): {"error":{"code":"data_inspection_failed","message":"blocked"}}"#);
        assert_eq!(projection.code, "content_filtered");
        assert!(!projection.retryable);
        assert!(!structured_provider_error_is_content_filter(r#"{"error":{"message":"data_inspection_failed"}}"#));
    }
    #[test]
    fn complete_wrapped_and_double_escaped_failures_are_not_answers() {
        let error=json!({"error":{"code":"data_inspection_failed","type":"DATA_INSPECTION_FAILED","message":"blocked","param":null},"id":"upstream"}).to_string();
        let escaped=serde_json::to_string(&error).unwrap();
        for body in [error.as_str(),&escaped[1..escaped.len()-1]] {
            let choice=StreamedChoice {content:json!({"message":format!("Internal service error: 400 - {body}")}).to_string(),..Default::default()};
            assert!(matches!(reject_wrapped_gateway_error(&choice),Err(ProviderAttemptError::Http {status:400,..})));
        }
    }
    #[test]
    fn explanations_json_examples_and_normal_messages_are_preserved() {
        for content in ["Internal service error: 400", "{\"message\":\"hello\"}",
            "{\"error\":{\"code\":\"data_inspection_failed\"}}",
            "Example: {\"message\":\"Internal service error: 400 - {}\"}",
            "```json\n{\"message\":\"Internal service error: 400 - {}\"}\n```",
            "{\"message\":\"Internal service error: 400 - {}\"}"] {
            assert!(reject_wrapped_gateway_error(&StreamedChoice {content:content.into(),..Default::default()}).is_ok());
        }
    }
}

fn structured_provider_error_is_content_filter(error: &str) -> bool {
    let Some(start) = error.find('{') else { return false; };
    let Ok(value) = serde_json::from_str::<Value>(&error[start..]) else { return false; };
    let Some(error) = value.get("error").and_then(Value::as_object) else { return false; };
    ["code", "type"].iter().any(|key| matches!(
        error.get(*key).and_then(Value::as_str),
        Some("data_inspection_failed" | "DATA_INSPECTION_FAILED" | "content_filter")
    ))
}

fn structured_provider_error_is_invalid_media(error: &str) -> bool {
    let Some(start) = error.find('{') else { return false; };
    let Ok(value) = serde_json::from_str::<Value>(&error[start..]) else { return false; };
    // Only typed error codes are trusted for classification. Do not inspect
    // echoed prompts or copy arbitrary provider text (which may contain secrets).
    let Some(error) = value.get("error").and_then(Value::as_object) else { return false; };
    ["code", "type"].iter().any(|key| matches!(error.get(*key).and_then(Value::as_str),
        Some("invalid_image" | "invalid_image_format" | "invalid_image_url"
            | "image_parse_error" | "unsupported_image" | "unsupported_image_format"
            | "invalid_base64_image" | "invalid_file" | "unsupported_file_format")))
}

fn llm_retry_delay(failure: &ProviderFailure, attempt: usize, cfg: &LlmConfig) -> Option<Duration> {
    if !failure.same_target_retryable {
        return None;
    }
    let initial = std::env::var("NOLA_LLM_RETRY_INITIAL_DELAY_MS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(2_000)
        .clamp(100, 30_000);
    let maximum = std::env::var("NOLA_LLM_RETRY_MAX_DELAY_MS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(120_000)
        .clamp(initial, 600_000);
    let header_delay = failure.retry_after_ms;
    let exponent = attempt.saturating_sub(1).min(16) as u32;
    let backoff = initial.saturating_mul(2u64.saturating_pow(exponent));
    let jitter = cfg
        .cancellation
        .execution_id()
        .wrapping_add(attempt as u64 * 97)
        % 251;
    let delay = match header_delay {
        Some(delay) if delay > maximum => return None,
        Some(delay) => delay,
        None => backoff.saturating_add(jitter).min(maximum),
    };
    Some(Duration::from_millis(delay))
}

pub(crate) fn structured_provider_error_is_context_limit(error: &str) -> Option<bool> {
    fn error_object_is_context_limit(value: &Value) -> bool {
        let Some(object) = value.as_object() else {
            return false;
        };
        let typed_code = ["code", "type", "error_code"].into_iter().any(|key| {
            object.get(key).and_then(Value::as_str).is_some_and(|code| {
                matches!(
                    code.trim().to_ascii_lowercase().as_str(),
                    "context_length_exceeded"
                        | "context_window_exceeded"
                        | "context_length_error"
                        | "prompt_too_long"
                        | "input_too_long"
                        | "request_too_large"
                )
            })
        });
        let payload_too_large = ["status", "status_code", "http_status"]
            .into_iter()
            .any(|key| object.get(key).and_then(Value::as_u64) == Some(413))
            || object.get("code").and_then(Value::as_u64) == Some(413);
        // A specific provider code is authoritative. Generic gateway wrapper
        // codes carry no failure category of their own, so retain a precise
        // context-limit diagnostic from the provider message inside them.
        let has_specific_non_context_error_code = ["code", "error_code"].into_iter().any(|key| {
            object.get(key).is_some_and(|value| {
                (value.is_string() || value.is_number())
                    && !value.as_str().is_some_and(|code| {
                        matches!(
                            code.trim().to_ascii_lowercase().as_str(),
                            "context_length_exceeded"
                                | "context_window_exceeded"
                                | "context_length_error"
                                | "prompt_too_long"
                                | "input_too_long"
                                | "request_too_large"
                                | "provider_failed"
                        )
                    })
                    && value.as_u64() != Some(413)
            })
        });
        let provider_message = object
            .get("message")
            .and_then(Value::as_str)
            .map(str::to_ascii_lowercase)
            .is_some_and(|message| {
                [
                    "prompt is too long",
                    "input is too long for requested model",
                    "exceeds the context window",
                    "exceeds the maximum context length",
                    "maximum context length is",
                    "maximum context length exceeded",
                    "reduce the length of the messages",
                    "exceeds the available context size",
                    "greater than the context length",
                    "context window exceeds limit",
                    "exceeded model token limit",
                    "request entity too large",
                    "prompt too long",
                ]
                .into_iter()
                .any(|marker| message.contains(marker))
            });
        typed_code
            || payload_too_large
            || (!has_specific_non_context_error_code && provider_message)
    }

    fn error_container_is_context_limit(value: &Value, depth: usize) -> bool {
        if depth > 3 {
            return false;
        }
        match value {
            Value::Object(object) => {
                error_object_is_context_limit(value)
                    || ["detail", "details", "cause", "inner_error", "innererror"]
                        .into_iter()
                        .any(|key| {
                            object.get(key).is_some_and(|nested| {
                                error_container_is_context_limit(nested, depth + 1)
                            })
                        })
            }
            Value::Array(values) => values
                .iter()
                .any(|value| error_container_is_context_limit(value, depth + 1)),
            _ => false,
        }
    }

    // HTTP adapters retain the provider JSON body inside a localized error
    // prefix. Extract every balanced object so a gateway diagnostic cannot
    // hide a later provider error. Request payloads outside a typed error
    // envelope are deliberately ignored.
    let mut saw_error_envelope = false;
    let mut consumed_until = 0usize;
    for (start, character) in error.char_indices() {
        if start < consumed_until || character != '{' {
            continue;
        }
        let mut depth = 0usize;
        let mut in_string = false;
        let mut escaped = false;
        for (relative, character) in error[start..].char_indices() {
            if in_string {
                if escaped {
                    escaped = false;
                } else if character == '\\' {
                    escaped = true;
                } else if character == '"' {
                    in_string = false;
                }
                continue;
            }
            match character {
                '"' => in_string = true,
                '{' => depth += 1,
                '}' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        let end = start + relative + character.len_utf8();
                        if let Ok(value) = serde_json::from_str::<Value>(&error[start..end]) {
                            consumed_until = end;
                            let has_error_envelope = value.as_object().is_some_and(|object| {
                                object.contains_key("error")
                                    || object.contains_key("errors")
                                    || object.contains_key("failure")
                            });
                            if has_error_envelope {
                                saw_error_envelope = true;
                                if value.as_object().is_some_and(|object| {
                                    ["error", "errors", "failure"].into_iter().any(|key| {
                                        object.get(key).is_some_and(|value| {
                                            error_container_is_context_limit(value, 0)
                                        })
                                    })
                                }) {
                                    return Some(true);
                                }
                            }
                        }
                        break;
                    }
                }
                _ => {}
            }
        }
    }
    saw_error_envelope.then_some(false)
}

fn is_retryable_llm_error(error: &str) -> bool {
    // Context overflow belongs to Core's bounded compaction recovery. Some
    // gateways mislabel it as 429/5xx, so typed provider codes must win over
    // status-based Host retries and provider fallback.
    if structured_provider_error_is_context_limit(error) == Some(true) {
        return false;
    }
    if structured_provider_error_is_exhausted_quota(error) {
        return false;
    }
    let lower = error.to_ascii_lowercase();
    if lower.contains("cancelled")
        || lower.contains("canceled")
        || lower.contains("(400")
        || lower.contains("(401")
        || lower.contains("(403")
        || lower.contains("(404")
        || lower.contains("invalid api key")
        || lower.contains("authentication")
    {
        return false;
    }
    direct_stream_provider_error_status(error)
        .is_some_and(|status| matches!(status, 408 | 409 | 425 | 429 | 500..=599))
        || [408u16, 409, 425, 429]
            .into_iter()
            .chain(500u16..=599)
            .any(|status| lower.contains(&format!("({status}")))
        || [
            "rate limit",
            "too many requests",
            "overloaded",
            "temporarily unavailable",
            "service unavailable",
            "gateway timeout",
            "timed out",
            "timeout",
        ]
        .iter()
        .any(|marker| lower.contains(marker))
        || is_retryable_stream_transport_message(&lower)
}

fn is_fallback_eligible_llm_error(error: &str) -> bool {
    is_retryable_llm_error(error) || structured_provider_error_is_exhausted_quota(error)
}

fn structured_provider_error_is_exhausted_quota(error: &str) -> bool {
    let Some(json_start) = error.find('{') else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<Value>(&error[json_start..]) else {
        return false;
    };

    fn find_key<'a>(value: &'a Value, key: &str, depth: usize) -> Option<&'a Value> {
        if depth > 6 {
            return None;
        }
        match value {
            Value::Object(object) => object.get(key).or_else(|| {
                object
                    .values()
                    .find_map(|child| find_key(child, key, depth + 1))
            }),
            Value::Array(values) => values
                .iter()
                .find_map(|child| find_key(child, key, depth + 1)),
            _ => None,
        }
    }

    let remaining_is_zero = [
        "X-RateLimit-Remaining",
        "x-ratelimit-remaining",
        "remaining",
    ]
    .into_iter()
    .filter_map(|key| find_key(&value, key, 0))
    .any(|remaining| remaining.as_u64() == Some(0) || remaining.as_str() == Some("0"));
    let has_reset_boundary = ["X-RateLimit-Reset", "x-ratelimit-reset", "reset"]
        .into_iter()
        .any(|key| find_key(&value, key, 0).is_some());
    let is_daily_limit = find_key(&value, "limit_source", 0)
        .and_then(Value::as_str)
        .is_some_and(|source| source.split(['_', '-']).any(|part| part == "daily"));

    remaining_is_zero && (has_reset_boundary || is_daily_limit)
}

fn direct_stream_provider_error_status(error: &str) -> Option<u16> {
    // An HTTP-200 SSE stream can terminate with a provider error object. Keep
    // this deliberately limited to a direct JSON envelope so JSON mentioned
    // in prompts or diagnostics cannot alter retry policy.
    let envelope = serde_json::from_str::<Value>(error.trim()).ok()?;
    let object = envelope.as_object()?;
    fn status(value: &Value, depth: usize) -> Option<u16> {
        if depth > 3 {
            return None;
        }
        match value {
            Value::Object(object) => {
                for key in ["code", "status", "status_code", "http_status"] {
                    let parsed = object.get(key).and_then(|value| {
                        value
                            .as_u64()
                            .or_else(|| value.as_str()?.parse::<u64>().ok())
                    });
                    if let Some(parsed) = parsed.and_then(|value| u16::try_from(value).ok()) {
                        return Some(parsed);
                    }
                }
                object.values().find_map(|value| status(value, depth + 1))
            }
            Value::Array(values) => values.iter().find_map(|value| status(value, depth + 1)),
            _ => None,
        }
    }
    ["error", "errors", "failure"]
        .into_iter()
        .find_map(|key| object.get(key).and_then(|value| status(value, 0)))
}

fn should_retry_same_provider_candidate(error: &str) -> bool {
    // A provider error delivered inside an otherwise successful SSE response
    // means this routed candidate has already failed. Let the outer candidate
    // chain fail over immediately instead of resampling the same route.
    direct_stream_provider_error_status(error).is_none()
}

fn is_retryable_stream_transport_message(message: &str) -> bool {
    [
        "connection reset",
        "connection aborted",
        "connection closed",
        "connection refused",
        "incomplete read",
        "incompleteread",
        "end of file before message length reached",
        "network is unreachable",
        "unexpected eof",
        "http2 error",
        "http/2 stream error",
        "h2 protocol error",
    ]
    .iter()
    .any(|marker| message.contains(marker))
}

fn wait_for_llm_retry(delay: Duration, cancellation: &ExecutionContext) -> Result<(), String> {
    let started = Instant::now();
    while started.elapsed() < delay {
        if cancellation.is_requested() {
            return Err(llm_cancelled_message());
        }
        let remaining = delay.saturating_sub(started.elapsed());
        thread::sleep(remaining.min(Duration::from_millis(50)));
    }
    Ok(())
}

#[cfg(test)]
mod retry_policy_tests {
    use std::io::Read;
    use std::net::TcpListener;
    use std::thread;
    use std::time::{Duration, Instant};

    use super::{
        is_fallback_eligible_llm_error, is_retryable_llm_error, model_failure_projection,
        send_request_with_cancellation, should_treat_stream_read_as_complete,
        structured_provider_error_is_context_limit, ExecutionContext, SendRequestResult,
    };

    #[test]
    fn context_overflow_requires_a_typed_error_envelope() {
        for error in [
            r#"{"error":{"code":"context_length_exceeded","message":"入力が長すぎます"}}"#,
            r#"{"errors":[{"type":"gateway_failure"},{"error_code":"prompt_too_long"}]}"#,
            r#"gateway diagnostic {"detail":{"code":"context_length_exceeded"}} provider {"error":{"type":"input_too_long"}}"#,
            r#"{"error":{"type":"invalid_request_error","message":"prompt is too long: 9001 tokens > 8192 maximum"}}"#,
            r#"{"error":{"detail":{"code":"context_length_exceeded"}}}"#,
            r#"{"errors":[{"message":"context_length_exceeded","detail":{"type":"prompt_too_long"}}]}"#,
            r#"{"error":{"code":413}}"#,
            r#"{"failure":{"error_code":"input_too_long"}}"#,
        ] {
            assert_eq!(
                structured_provider_error_is_context_limit(error),
                Some(true),
                "{error}"
            );
        }

        for error in [
            r#"{"detail":{"code":"context_length_exceeded"}}"#,
            r#"{"details":{"type":"prompt_too_long"}}"#,
            r#"{"error":"context_length_exceeded"}"#,
            r#"{"errors":["prompt_too_long"]}"#,
        ] {
            assert_ne!(
                structured_provider_error_is_context_limit(error),
                Some(true),
                "{error}"
            );
        }
        assert_eq!(
            structured_provider_error_is_context_limit(
                "maximum context length exceeded; 搜索 current records"
            ),
            None
        );
        assert_eq!(
            structured_provider_error_is_context_limit(
                r#"{"code":"context_length_exceeded","message":"not an error envelope"}"#
            ),
            None
        );
    }

    #[test]
    fn retries_only_transient_provider_failures() {
        assert!(is_retryable_llm_error(
            "LLM request failed (429 Too Many Requests): rate limit"
        ));
        assert!(!is_retryable_llm_error(
            r#"LLM request failed (429 Too Many Requests): {"error":{"code":429,"metadata":{"headers":{"X-RateLimit-Remaining":"0","X-RateLimit-Reset":"1789257600000"},"limit_source":"openrouter_free_tier_daily"}}}"#
        ));
        assert!(is_fallback_eligible_llm_error(
            r#"LLM request failed (429 Too Many Requests): {"error":{"code":429,"metadata":{"headers":{"X-RateLimit-Remaining":"0","X-RateLimit-Reset":"1789257600000"},"limit_source":"openrouter_free_tier_daily"}}}"#
        ));
        assert!(is_retryable_llm_error(
            "LLM request failed (503 Service Unavailable): overloaded"
        ));
        assert!(is_retryable_llm_error("connection reset by peer"));
        assert!(is_retryable_llm_error(
            "LLM stream read failed: request or response body error: unexpected EOF during chunk size line"
        ));
        assert!(!is_retryable_llm_error(
            "LLM request failed (401 Unauthorized): invalid API key"
        ));
        assert!(!is_retryable_llm_error(
            "maximum context length is 64000 tokens"
        ));
        assert!(!is_retryable_llm_error(
            "LLM request failed (400 Bad Request): invalid tools"
        ));
        assert!(!is_retryable_llm_error(
            r#"LLM request failed (503 Service Unavailable): {"errors":[{"code":"gateway_failure"},{"type":"prompt_too_long"}]}"#
        ));
        assert!(!is_retryable_llm_error(
            r#"LLM request failed (503 Service Unavailable): {"error":{"code":"provider_failed","message":"maximum context length exceeded"}}"#
        ));
        assert!(is_retryable_llm_error(
            r#"LLM request failed (503 Service Unavailable): {"detail":{"code":"context_length_exceeded"}}"#
        ));
        assert!(is_retryable_llm_error(
            r#"{"choices":[],"error":{"code":502,"metadata":{"error_type":"provider_unavailable"}}}"#
        ));
        assert!(!super::should_retry_same_provider_candidate(
            r#"{"choices":[],"error":{"code":502,"metadata":{"error_type":"provider_unavailable"}}}"#
        ));
        assert!(is_retryable_llm_error(
            r#"{"choices":[],"errors":[{"status_code":"503","type":"upstream_error"}]}"#
        ));
    }

    #[test]
    fn user_failure_projection_is_structured_and_does_not_copy_provider_body() {
        let media = model_failure_projection(r#"HTTP (400): {"error":{"code":"invalid_image","message":"secret echoed content"}}"#);
        assert_eq!(media.code, "invalid_media_input");
        assert!(!media.message.contains("secret"));
        let echoed = model_failure_projection(r#"HTTP (400): {"error":{"code":"invalid_request","request":{"code":"invalid_image"}}}"#);
        assert_eq!(echoed.code, "invalid_model_request");
        let raw = r#"LLM request failed (429 Too Many Requests): {"error":{"code":429,"message":"account user_123 secret","metadata":{"headers":{"X-RateLimit-Remaining":"0","X-RateLimit-Reset":"1789257600000"},"limit_source":"openrouter_free_tier_daily"}}}"#;
        let failure = model_failure_projection(raw);
        assert_eq!(failure.code, "quota_exhausted");
        assert_eq!(failure.message, "The provider quota is exhausted.");
        assert!(failure.retryable);
        assert!(!failure.message.contains("user_123"));
        let invalid = model_failure_projection(
            "LLM request failed (400 Bad Request): invalid Responses input",
        );
        assert_eq!(invalid.code, "invalid_model_request");
        assert_eq!(invalid.message, "The provider rejected the model request.");
        let incomplete =
            model_failure_projection("Responses stream ended without response.completed");
        assert_eq!(incomplete.code, "responses_incomplete");
        for (error, code) in [
            ("Responses incomplete: content_filter", "content_filtered"),
            ("Responses incomplete: unknown_reason", "responses_incomplete"),
            ("Responses interrupted with uncommitted tool calls", "responses_tool_transaction_incomplete"),
            ("Responses output limit with uncommitted tool calls", "responses_tool_transaction_incomplete"),
            ("Responses stream timed out", "responses_timeout"),
            ("Responses stream transport read failed", "responses_transport_error"),
            ("Responses provider reported failure", "responses_provider_failed"),
            ("invalid Responses completed envelope: secret provider body", "responses_protocol_error"),
        ] {
            let projection = model_failure_projection(error);
            assert_eq!(projection.code, code, "{error}");
            assert!(!projection.message.contains("secret provider body"));
        }
    }

    #[test]
    fn partial_stream_unexpected_eof_enters_bounded_continuation_recovery() {
        assert!(should_treat_stream_read_as_complete(
            "LLM stream read failed: request or response body error: error reading a body from connection: unexpected EOF during chunk size line"
        ));
    }

    #[test]
    fn http2_protocol_stream_failure_uses_transaction_safe_recovery() {
        let error = "error sending request for url (https://provider.invalid/api): http2 error: stream error received: unspecific protocol error detected";

        assert!(is_retryable_llm_error(error));
        assert!(should_treat_stream_read_as_complete(error));
        // No published delta can be resampled by the bounded provider retry.
        assert!(super::should_retry_incomplete_stream_round(
            false, false, false, true
        ));
        // Published answer text is retained and continued semantically instead
        // of being replayed by the provider retry loop.
        assert!(!super::should_retry_incomplete_stream_round(
            true, true, false, true
        ));
        // Partial tool arguments have no side effect and are safe to discard
        // before resampling the model round.
        assert!(super::should_retry_incomplete_stream_round(
            true, true, true, true
        ));
    }

    #[test]
    fn partial_tool_stream_failure_remains_retryable_after_arguments_are_discarded() {
        assert!(is_retryable_llm_error(
            "incomplete read: LLM stream closed before the provider completed a tool call; partial tool arguments were discarded"
        ));
        assert!(super::should_retry_incomplete_stream_round(
            true, true, true, true
        ));
        assert!(!super::should_retry_incomplete_stream_round(
            true, true, false, true
        ));
    }

    #[test]
    fn duplicate_native_tool_arguments_remain_invalid_instead_of_last_key_winning() {
        let raw = r#"{"messages":[{"version":"0.9.1","version":"1.0"}]}"#;
        for input in [raw.to_owned(), serde_json::to_string(raw).unwrap()] {
            assert_eq!(super::normalized_tool_arguments_value(&input), serde_json::Value::String(raw.into()));
        }
        let valid = r#"{"messages":[{"version":"0.9.1"}]}"#;
        assert_eq!(super::normalized_tool_arguments_value(valid), serde_json::from_str::<serde_json::Value>(valid).unwrap());
    }

    #[test]
    fn double_encoded_truncated_tool_arguments_keep_the_inner_protocol_error() {
        let encoded = serde_json::to_string("{\"path\":\"large.cpp\",\"content\":\"partial")
            .expect("encode outer provider string");
        let normalized = super::normalize_tool_arguments_text(&encoded);

        assert_eq!(normalized, "{\"path\":\"large.cpp\",\"content\":\"partial");
        assert!(serde_json::from_str::<serde_json::Value>(&normalized).is_err());
    }

    #[test]
    fn blocking_request_send_returns_promptly_after_cancellation() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind local test server");
        let address = listener.local_addr().expect("read local address");
        listener.set_nonblocking(true).expect("bound test accept");
        let cancellation = ExecutionContext::default();
        let cancel = cancellation.clone();
        let server = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(2);
            let (mut stream, _) = loop {
                match listener.accept() {
                    Ok(connection) => break connection,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(
                            Instant::now() < deadline,
                            "test request never reached loopback server"
                        );
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("accept test request: {error}"),
                }
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .expect("bound test read");
            let mut request = [0u8; 1024];
            assert!(stream.read(&mut request).expect("read test request") > 0);
            // Exercise cancellation of an actual in-flight request, not a
            // timer that can fire before client construction has completed.
            cancel.request();
            thread::sleep(Duration::from_millis(500));
        });
        let request = reqwest::blocking::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(2))
            .build()
            .expect("build test client")
            .get(format!("http://{address}/v1/chat/completions"));

        let started = Instant::now();
        let result = send_request_with_cancellation(request, &cancellation);

        assert!(matches!(result, SendRequestResult::Cancelled));
        assert!(started.elapsed() < Duration::from_millis(400));
        server.join().expect("join local test server");
    }
}

#[cfg(test)]
mod stream_completion_tests {
    use super::{
        append_openai_tool_delta, capture_openai_response_identity, finalize_stream_completion,
        LlmEvent, StreamedChoice, ToolCall, ToolSlot,
    };
    use serde_json::json;
    use std::collections::BTreeMap;

    #[test]
    fn unfinished_tool_transaction_has_explicit_safe_failure_code() {
        let mut choice = StreamedChoice { tool_calls: vec![ToolCall {
            id: "partial".into(), name: "write_file".into(), arguments: "{\"path\":".into(),
        }], ..Default::default() };
        let error = finalize_stream_completion(&mut choice, false).unwrap_err();
        let projection = super::model_failure_projection(&error);
        assert_eq!(projection.code, "tool_transaction_incomplete");
        assert!(projection.retryable);
        assert!(choice.tool_calls.is_empty());
        assert!(!projection.message.contains("write_file"));
    }

    #[test]
    fn tool_batch_overflow_does_not_allocate_or_emit_the_overflowing_slot() {
        use nola_core::agent::model_tool_batch::*;
        let mut slots = BTreeMap::new();
        for index in 0..MAX_MODEL_TOOL_CALLS_PER_BATCH {
            append_openai_tool_delta(
                &mut slots,
                &json!({
                    "index": index, "id": format!("call-{index}"),
                    "function": {"name": "read_file", "arguments": "{"}
                }),
            )
            .unwrap();
        }
        // More fragments for an existing call consume no additional slot.
        assert!(append_openai_tool_delta(
            &mut slots,
            &json!({"index": 0,
            "function": {"arguments": "}"}})
        )
        .is_ok());
        assert_eq!(
            append_openai_tool_delta(&mut slots, &json!({"index": 500})).unwrap_err(),
            MODEL_TOOL_BATCH_LIMIT_EXCEEDED
        );
        assert_eq!(slots.len(), MAX_MODEL_TOOL_CALLS_PER_BATCH);
    }

    #[test]
    fn anthropic_tool_batch_bounds_slots_even_without_call_identity() {
        use nola_core::agent::model_tool_batch::*;
        for event_type in ["content_block_start", "content_block_delta"] {
            let mut slots = BTreeMap::new();
            let mut choice = StreamedChoice::default();
            for index in 0..=MAX_MODEL_TOOL_CALLS_PER_BATCH {
                let event = json!({"type": event_type, "index": index,
                    "content_block": {"type": "tool_use"},
                    "delta": {"type": "input_json_delta", "partial_json": "{"}});
                let result = super::parse_anthropic_stream_event(
                    &event,
                    &mut choice,
                    &mut slots,
                    &mut |_| Ok(()),
                );
                if index < MAX_MODEL_TOOL_CALLS_PER_BATCH {
                    assert!(result.is_ok());
                } else {
                    assert_eq!(result.unwrap_err(), MODEL_TOOL_BATCH_LIMIT_EXCEEDED);
                }
            }
            assert_eq!(slots.len(), MAX_MODEL_TOOL_CALLS_PER_BATCH);
        }
    }

    fn retryable_after_events(events: &[LlmEvent], has_tools: bool) -> bool {
        let emitted_provider_event = !events.is_empty();
        let emitted_nontransactional_event = events
            .iter()
            .any(|event| matches!(event, LlmEvent::TextDelta(_) | LlmEvent::ReasoningDelta(_)));
        !emitted_nontransactional_event && (!emitted_provider_event || has_tools)
    }

    #[test]
    fn provider_retry_never_replays_already_published_text_or_reasoning() {
        assert!(retryable_after_events(&[], false));
        assert!(retryable_after_events(
            &[LlmEvent::ToolCallDelta {
                call_id: "call-1".into(),
                name: "write_file".into(),
                arguments_delta: "{\"path\":".into(),
            }],
            true,
        ));
        assert!(!retryable_after_events(
            &[LlmEvent::TextDelta("visible".into())],
            true,
        ));
        assert!(!retryable_after_events(
            &[LlmEvent::ReasoningDelta("visible reasoning".into())],
            true,
        ));
    }

    #[test]
    fn response_identity_uses_the_first_nonempty_provider_values() {
        let mut choice = StreamedChoice::default();
        capture_openai_response_identity(
            &mut choice,
            &json!({"id":"response-1","model":"publisher/resolved-model"}),
        );
        capture_openai_response_identity(
            &mut choice,
            &json!({"id":"response-2","model":"publisher/other-model"}),
        );

        assert_eq!(choice.response_id.as_deref(), Some("response-1"));
        assert_eq!(
            choice.response_model.as_deref(),
            Some("publisher/resolved-model")
        );
    }

    #[test]
    fn interrupted_text_stream_is_not_reported_as_stop() {
        let mut choice = StreamedChoice {
            content: "partial answer".to_string(),
            ..StreamedChoice::default()
        };
        finalize_stream_completion(&mut choice, false).unwrap();
        assert_eq!(choice.finish_reason, "stream_incomplete");
        assert_eq!(choice.sampling_measurement_outcome(), nola_measurement::Outcome::Incomplete);
    }

    #[test]
    fn interrupted_tool_stream_discards_partial_call() {
        let mut choice = StreamedChoice {
            tool_calls: vec![ToolCall {
                id: "call-1".to_string(),
                name: "write_file".to_string(),
                arguments: "{\"path\":".to_string(),
            }],
            ..StreamedChoice::default()
        };
        assert!(finalize_stream_completion(&mut choice, false).is_err());
        assert!(choice.tool_calls.is_empty());
    }

    #[test]
    fn terminal_tool_stream_keeps_executable_call() {
        let mut choice = StreamedChoice {
            tool_calls: vec![ToolCall {
                id: "call-1".to_string(),
                name: "read_file".to_string(),
                arguments: "{\"path\":\"README.md\"}".to_string(),
            }],
            ..StreamedChoice::default()
        };
        finalize_stream_completion(&mut choice, true).unwrap();
        assert_eq!(choice.finish_reason, "tool_calls");
        assert_eq!(choice.tool_calls.len(), 1);
    }

    #[test]
    fn openai_argument_fragments_reassemble_as_one_json_object() {
        let mut slots = BTreeMap::<usize, ToolSlot>::new();
        let first = append_openai_tool_delta(
            &mut slots,
            &json!({
                "index": 0,
                "id": "call-1",
                "function": {"name": "write_file", "arguments": "{\"path\":"}
            }),
        )
        .unwrap()
        .expect("first tool event");
        let second = append_openai_tool_delta(
            &mut slots,
            &json!({"index": 0, "function": {"arguments": "\"多语言.txt\"}"}}),
        )
        .unwrap()
        .expect("second tool event");

        assert!(matches!(
            first,
            LlmEvent::ToolCallDelta { ref arguments_delta, .. }
                if arguments_delta == "{\"path\":"
        ));
        assert!(matches!(
            second,
            LlmEvent::ToolCallDelta { ref arguments_delta, .. }
                if arguments_delta == "\"多语言.txt\"}"
        ));
        let slot = slots.get(&0).unwrap();
        assert_eq!(slot.id, "call-1");
        assert_eq!(slot.name, "write_file");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&slot.arguments).unwrap(),
            json!({"path": "多语言.txt"})
        );
    }

    #[test]
    fn openai_stream_rejects_a_call_id_change_in_the_same_slot() {
        let mut slots = BTreeMap::<usize, ToolSlot>::new();
        append_openai_tool_delta(
            &mut slots,
            &json!({"index": 0, "id": "call-original", "function": {"name": "read_file"}}),
        )
        .unwrap();
        let error = append_openai_tool_delta(
            &mut slots,
            &json!({"index": 0, "id": "call-replaced", "function": {"arguments": "{}"}}),
        )
        .unwrap_err();

        assert!(error.contains("call-original -> call-replaced"));
    }
}

#[cfg(test)]
mod stream_transport_recovery_tests {
    use super::super::contracts::LlmProviderTarget;
    use super::{stream_chat, LlmConfig, LlmEvent, ProviderFailure, OPENAI_COMPATIBLE_CONTINUATION_NAMESPACE};
    use serde_json::json;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::Duration;

    fn read_request(stream: &mut TcpStream) -> serde_json::Value {
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("set provider read timeout");
        let mut request = Vec::new();
        let mut buffer = [0_u8; 4096];
        let (header_end, content_length) = loop {
            let read = stream.read(&mut buffer).expect("read provider request");
            assert!(read > 0, "provider request ended before headers");
            request.extend_from_slice(&buffer[..read]);
            let Some(header_end) = request.windows(4).position(|part| part == b"\r\n\r\n") else {
                continue;
            };
            let headers = String::from_utf8_lossy(&request[..header_end]);
            let content_length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                })
                .expect("provider request content length");
            break (header_end + 4, content_length);
        };
        while request.len() < header_end + content_length {
            let read = stream
                .read(&mut buffer)
                .expect("read provider request body");
            assert!(read > 0, "provider request ended before body");
            request.extend_from_slice(&buffer[..read]);
        }
        serde_json::from_slice(&request[header_end..header_end + content_length])
            .expect("provider request JSON")
    }

    fn write_sse(stream: &mut TcpStream, events: &str) {
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            events.len(),
            events
        )
        .expect("write provider response");
    }

    fn write_truncated_sse(stream: &mut TcpStream, events: &str) {
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            events.len() + 128,
            events
        )
        .expect("write truncated provider response");
    }

    fn write_error_response(
        stream: &mut TcpStream,
        status: &str,
        body: &str,
        retry_after_ms: Option<u64>,
    ) {
        let retry_header = retry_after_ms
            .map(|value| format!("Retry-After-Ms: {value}\r\n"))
            .unwrap_or_default();
        write!(
            stream,
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\n{retry_header}Content-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        )
        .expect("write provider error response");
    }

    fn config(endpoint: String) -> LlmConfig {
        LlmConfig {
            siwc: None,
            native_model_route: None,
            compaction_output_budget: None,
            chat_output_token_field: Default::default(),
            provider: None,
            fallbacks: Vec::new(),
            endpoint,
            api_key: "test-key".to_string(),
            model: "test-model".to_string(),
            temperature: 0.2,
            top_p: 0.9,
            max_output_tokens: 256,
            max_retries: crate::llm::default_llm_max_retries(),
            thinking_enabled: false, reasoning_summary_supported: None,
            reasoning_selection: None,
            sampling_override: false,
            generation_options: Default::default(),
            cancellation: Default::default(),
            openai_prompt_cache: false,
            openai_prompt_cache_key: None,
            openai_prompt_cache_key_prefix: None,
            openai_prompt_cache_retention: None,
            openai_prompt_cache_history_breakpoint: false,
            openai_prompt_cache_history_breakpoint_endpoints: None,
            parallel_tool_calls: None,
        }
    }

    #[test]
    fn chat_active_tool_stream_outlives_old_absolute_deadline() {
        active_tool_stream_outlives_old_absolute_deadline(false);
    }

    #[test]
    fn anthropic_active_tool_stream_outlives_old_absolute_deadline() {
        active_tool_stream_outlives_old_absolute_deadline(true);
    }

    fn active_tool_stream_outlives_old_absolute_deadline(anthropic: bool) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            read_request(&mut stream);
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n").unwrap();
            let mut send = |value: serde_json::Value| {
                writeln!(stream, "data: {value}\n").unwrap();
                stream.flush().unwrap();
            };
            if anthropic {
                send(json!({"type":"content_block_start","index":0,"content_block":{
                    "type":"tool_use","id":"call_long","name":"write_file","input":{}}}));
            } else {
                send(json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_long",
                    "type":"function","function":{"name":"write_file","arguments":""}}]}}]}));
            }
            // Real socket regression: each fragment makes progress, but the
            // complete transaction takes 48s (beyond the removed 45s cap).
            for index in 0..24 {
                thread::sleep(Duration::from_secs(2));
                let fragment = if index == 0 { "{\"path\":\"x\",\"content\":\"" }
                    else if index == 23 { "\"}" } else { "x" };
                if anthropic {
                    send(json!({"type":"content_block_delta","index":0,
                        "delta":{"type":"input_json_delta","partial_json":fragment}}));
                } else {
                    send(json!({"choices":[{"delta":{"tool_calls":[{"index":0,
                        "function":{"arguments":fragment}}]}}]}));
                }
            }
            if anthropic {
                send(json!({"type":"content_block_stop","index":0}));
                send(json!({"type":"message_delta","delta":{"stop_reason":"tool_use"}}));
                send(json!({"type":"message_stop"}));
            } else {
                send(json!({"choices":[{"delta":{},"finish_reason":"tool_calls"}]}));
                writeln!(stream, "data: [DONE]\n").unwrap();
            }
        });
        let mut cfg = config(format!("http://{address}/v1"));
        cfg.max_retries = 0;
        if anthropic { cfg.provider = Some("anthropic".into()); }
        let result = stream_chat(&cfg, &[json!({"role":"user","content":"write"})],
            &[json!({"type":"function","function":{"name":"write_file",
                "parameters":{"type":"object","properties":{"path":{"type":"string"},"content":{"type":"string"}}}}})],
            "auto", |_| {});
        server.join().unwrap();
        let choice = result.expect("active tool transaction must not time out");
        assert_eq!(choice.tool_calls.len(), 1);
        let arguments: serde_json::Value = serde_json::from_str(&choice.tool_calls[0].arguments).unwrap();
        assert_eq!(arguments["content"], "x".repeat(22));
    }

    #[test]
    fn native_channel_rejection_precedes_http_and_cannot_fall_back() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
        let mut cfg = config(endpoint.clone());
        cfg.native_model_route = Some(serde_json::from_value(json!({
            "adapter":"mlx", "adapter_contract_revision":1, "model_id":"wrong-model",
            "profile_ref":{"profile_id":"native-profile", "admitted_revision":1}
        })).unwrap());
        cfg.fallbacks.push(crate::llm::LlmProviderTarget {
            endpoint, provider: None, model: Some("fallback".into()), api_key: Some("fallback-key".into()),
        });
        let mut starts = 0;
        let failure = super::stream_chat_checked(&cfg, &[json!({"role":"user","content":"hello"})], &[], "none", |event| {
            if matches!(event, LlmEvent::SamplingStarted { .. }) { starts += 1; }
            Ok(())
        }).unwrap_err();
        assert!(!failure.retryable);
        assert!(!failure.fallback_eligible);
        assert_eq!(starts, 1);
        assert_eq!(listener.accept().unwrap_err().kind(), std::io::ErrorKind::WouldBlock);
    }

    #[test]
    fn local_worker_loss_has_a_typed_non_provider_failure() {
        let cfg = config("http://127.0.0.1:1/v1".into());
        let failure = super::ProviderAttemptError::WorkerDisconnected.into_failure(&cfg);
        assert_eq!(failure.code, "transport_worker_disconnected");
        assert!(!failure.retryable);
        assert!(!failure.same_target_retryable);
        assert!(!failure.fallback_eligible);
        assert!(!failure.exhausts_target);
        assert_eq!(failure.status, None);
        assert_eq!(failure.retry_after_ms, None);
        assert_eq!(failure.model.as_deref(), Some("test-model"));
        assert!(!cfg.cancellation.is_requested());
    }

    #[test]
    fn repetitive_reasoning_is_failure_not_success_or_automatic_retry() {
        let cfg = config("http://127.0.0.1:1/v1".into());
        let failure = super::ProviderAttemptError::Message(super::super::stream_state::REPETITIVE_REASONING.into()).into_failure(&cfg);
        assert_eq!(failure.code, "model_repetitive_reasoning");
        assert!(!failure.retryable);
        assert!(!failure.same_target_retryable);
        assert!(!failure.fallback_eligible);
    }

    #[test]
    fn local_cancellation_is_typed_control_and_never_provider_retry_or_fallback() {
        let cfg = config("http://127.0.0.1:1/v1".into());
        let failure = ProviderFailure::cancelled(&cfg);
        assert_eq!(failure.code, "cancelled");
        assert!(!failure.retryable);
        assert!(!failure.same_target_retryable);
        assert!(!failure.fallback_eligible);
        assert!(!failure.exhausts_target);
        assert_eq!(failure.status, None);
        assert_eq!(failure.retry_after_ms, None);
        assert_eq!(failure.model.as_deref(), Some("test-model"));
        // A provider-controlled string cannot manufacture local cancellation.
        assert_ne!(ProviderFailure::classify(&cfg, "cancelled").code, "cancelled");
        assert_ne!(ProviderFailure::classify(&cfg, "用户取消").code, "cancelled");
    }

    #[test]
    fn checked_stream_preserves_local_cancellation_in_the_returned_failure() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (cancelled, cancellation_observed) = std::sync::mpsc::channel();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            read_request(&mut stream);
            write!(stream, concat!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n",
                "data: {{\"choices\":[{{\"delta\":{{\"content\":\"started\"}},\"finish_reason\":null}}]}}\n\n",
            )).unwrap();
            // No remote EOF or completion may race ahead of local control.
            cancellation_observed.recv_timeout(Duration::from_secs(5)).unwrap();
        });
        let cfg = config(format!("http://{address}/v1"));
        let cancellation = cfg.cancellation.clone();
        let mut observed_text = false;
        let failure = super::stream_chat_checked(
            &cfg,
            &[json!({"role":"user","content":"test"})],
            &[],
            "none",
            |event| {
                if matches!(event, LlmEvent::TextDelta(_)) {
                    observed_text = true;
                    cancellation.request();
                    cancelled.send(()).unwrap();
                }
                Ok(())
            },
        ).unwrap_err();
        server.join().unwrap();
        assert!(observed_text);
        assert_eq!(failure.code, "cancelled");
        assert!(!failure.retryable);
        assert!(!failure.same_target_retryable);
        assert!(!failure.fallback_eligible);
    }

    #[test]
    fn chat_output_limit_field_is_bound_to_adapter_or_explicit_profile_contract() {
        let generic = config("https://proxy.invalid/v1".into());
        assert_eq!(super::output_token_parameter_name(&generic), "max_tokens");
        let official = config("https://api.openai.com/v1".into());
        assert_eq!(
            super::output_token_parameter_name(&official),
            "max_completion_tokens"
        );
        let mut explicit = generic.clone();
        explicit.chat_output_token_field =
            nola_application::ChatOutputTokenField::MaxCompletionTokens;
        assert_eq!(
            super::output_token_parameter_name(&explicit),
            "max_completion_tokens"
        );
        explicit.chat_output_token_field = nola_application::ChatOutputTokenField::MaxTokens;
        explicit.endpoint = "https://api.openai.com/v1".into();
        assert_eq!(super::output_token_parameter_name(&explicit), "max_tokens");
        explicit.provider = Some("openai-responses".into());
        assert_eq!(
            super::output_token_parameter_name(&explicit),
            "max_output_tokens"
        );
    }

    #[test]
    fn explicit_chat_output_limit_field_reaches_the_http_request() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let body = read_request(&mut stream);
            assert_eq!(body["max_completion_tokens"], 256);
            assert!(body.get("max_tokens").is_none());
            write_sse(&mut stream, "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n");
        });
        let mut cfg = config(format!("http://{address}/v1"));
        cfg.chat_output_token_field = nola_application::ChatOutputTokenField::MaxCompletionTokens;
        let result = stream_chat(
            &cfg,
            &[json!({"role":"user","content":"hello"})],
            &[],
            "auto",
            |_| {},
        );
        server.join().unwrap();
        assert_eq!(result.unwrap().content, "ok");
    }

    #[test]
    fn checked_initial_sampling_requires_admission_before_network() {
        let cfg = config("http://127.0.0.1:1/v1".into());
        let mut samples = Vec::new();
        let error = super::stream_chat_checked(&cfg, &[], &[], "none", |event| match event {
            LlmEvent::SamplingStarted { provider, model } => {
                samples.push((provider, model));
                Err("503 durable attempt admission refused".into())
            }
            _ => panic!("no provider or retry event may precede initial admission"),
        })
        .unwrap_err();
        assert_eq!(error.diagnostic, "503 durable attempt admission refused");
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].1, "test-model");
        assert!(samples[0]
            .0
            .starts_with("openai-compatible.chat-completions:"));
        assert!(!samples[0].0.contains("test-key"));
    }

    #[test]
    fn checked_retry_rejection_stops_before_second_http_or_fallback() {
        let primary = TcpListener::bind("127.0.0.1:0").unwrap();
        let primary_address = primary.local_addr().unwrap();
        let fallback = TcpListener::bind("127.0.0.1:0").unwrap();
        let fallback_address = fallback.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = primary.accept().unwrap();
            read_request(&mut stream);
            write_error_response(
                &mut stream,
                "429 Too Many Requests",
                r#"{"error":{"message":"retry"}}"#,
                Some(0),
            );
            drop(stream);
            primary
        });
        let mut cfg = config(format!("http://{primary_address}/v1"));
        cfg.fallbacks.push(LlmProviderTarget {
            provider: None,
            endpoint: format!("http://{fallback_address}/v1"),
            api_key: None,
            model: None,
        });
        let mut retries = 0;
        let mut observed_failure = None;
        let error = super::stream_chat_checked(
            &cfg,
            &[json!({"role":"user","content":"test"})],
            &[],
            "none",
            |event| match event {
                LlmEvent::Retrying { failure, .. } => {
                    retries += 1;
                    observed_failure = Some(failure);
                    Err("503 receipt commit refused".into())
                }
                _ => Ok(()),
            },
        )
        .unwrap_err();
        assert_eq!(error.diagnostic, "503 receipt commit refused");
        assert_eq!(retries, 1);
        let observed_failure = observed_failure.expect("typed retry failure");
        assert_eq!(observed_failure.code, "rate_limited");
        assert_eq!(observed_failure.status, Some(429));
        assert_eq!(observed_failure.retry_after_ms, Some(0));
        assert!(observed_failure.retryable);
        assert!(observed_failure.same_target_retryable);
        let primary = server.join().unwrap();
        for listener in [primary, fallback] {
            listener.set_nonblocking(true).unwrap();
            assert_eq!(
                listener.accept().unwrap_err().kind(),
                std::io::ErrorKind::WouldBlock
            );
        }
    }

    #[test]
    fn checked_fallback_rejection_never_contacts_next_provider() {
        let primary = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = primary.local_addr().unwrap();
        let fallback = TcpListener::bind("127.0.0.1:0").unwrap();
        let fallback_address = fallback.local_addr().unwrap();
        let retries = super::default_llm_max_retries();
        let server = thread::spawn(move || {
            for _ in 0..=retries {
                let (mut stream, _) = primary.accept().unwrap();
                read_request(&mut stream);
                write_error_response(
                    &mut stream,
                    "429 Too Many Requests",
                    r#"{"error":{"message":"retry"}}"#,
                    Some(0),
                );
            }
            primary
        });
        let mut cfg = config(format!("http://{address}/v1"));
        cfg.fallbacks.push(LlmProviderTarget {
            provider: None,
            endpoint: format!("http://{fallback_address}/v1"),
            api_key: None,
            model: None,
        });
        let mut notices = 0;
        let error = super::stream_chat_checked(
            &cfg,
            &[json!({"role":"user","content":"test"})],
            &[],
            "none",
            |event| {
                if matches!(event, LlmEvent::Retrying { .. }) {
                    notices += 1;
                    if notices > retries {
                        return Err("503 fallback commit rejected".into());
                    }
                }
                Ok(())
            },
        )
        .unwrap_err();
        assert_eq!(error.diagnostic, "503 fallback commit rejected");
        assert_eq!(notices, retries + 1);
        for listener in [server.join().unwrap(), fallback] {
            listener.set_nonblocking(true).unwrap();
            assert_eq!(
                listener.accept().unwrap_err().kind(),
                std::io::ErrorKind::WouldBlock
            );
        }
    }

    #[test]
    fn checked_delta_rejection_stops_current_stream_without_retry_or_fallback() {
        let primary = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = primary.local_addr().unwrap();
        let fallback = TcpListener::bind("127.0.0.1:0").unwrap();
        let fallback_address = fallback.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = primary.accept().unwrap();
            read_request(&mut stream);
            write_sse(&mut stream, concat!(
                "data: {\"choices\":[{\"delta\":{\"content\":\"first\"},\"finish_reason\":null}]}\n\n",
                "data: {\"choices\":[{\"delta\":{\"content\":\"second\"},\"finish_reason\":null}]}\n\n",
                "data: [DONE]\n\n"));
            primary
        });
        let mut cfg = config(format!("http://{address}/v1"));
        cfg.fallbacks.push(LlmProviderTarget {
            provider: None,
            endpoint: format!("http://{fallback_address}/v1"),
            api_key: None,
            model: None,
        });
        let mut deltas = 0;
        let error = super::stream_chat_checked(
            &cfg,
            &[json!({"role":"user","content":"test"})],
            &[],
            "none",
            |event| match event {
                LlmEvent::SamplingStarted { .. } | LlmEvent::StageTiming { .. } => Ok(()),
                LlmEvent::TextDelta(_) => {
                    deltas += 1;
                    Err("connection reset by receipt store".into())
                }
                _ => panic!("unexpected provider event before rejected text delta"),
            },
        )
        .unwrap_err();
        assert_eq!(error.diagnostic, "connection reset by receipt store");
        assert_eq!(deltas, 1);
        for listener in [server.join().unwrap(), fallback] {
            listener.set_nonblocking(true).unwrap();
            assert_eq!(
                listener.accept().unwrap_err().kind(),
                std::io::ErrorKind::WouldBlock
            );
        }
    }

    #[test]
    fn transient_primary_failure_uses_explicit_fallback_before_any_visible_output() {
        let primary = TcpListener::bind("127.0.0.1:0").expect("bind primary provider");
        let primary_address = primary.local_addr().expect("primary address");
        let primary_server = thread::spawn(move || {
            for _ in 0..=super::default_llm_max_retries() {
                let (mut stream, _) = primary.accept().expect("accept primary request");
                read_request(&mut stream);
                write_error_response(
                    &mut stream,
                    "503 Service Unavailable",
                    r#"{"error":{"message":"primary unavailable"}}"#,
                    Some(0),
                );
            }
        });
        let fallback = TcpListener::bind("127.0.0.1:0").expect("bind fallback provider");
        let fallback_address = fallback.local_addr().expect("fallback address");
        let fallback_server = thread::spawn(move || {
            let (mut stream, _) = fallback.accept().expect("accept fallback request");
            read_request(&mut stream);
            write_sse(
                &mut stream,
                concat!(
                    "data: {\"id\":\"response-42\",\"model\":\"resolved-model\",\"choices\":[{\"delta\":{\"content\":\"fallback answer\"},\"finish_reason\":null}]}\n\n",
                    "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
                    "data: [DONE]\n\n"
                ),
            );
        });
        let mut cfg = config(format!("http://{primary_address}/v1"));
        cfg.fallbacks.push(LlmProviderTarget {
            provider: Some("openai".to_string()),
            endpoint: format!("http://{fallback_address}/v1"),
            api_key: Some("fallback-key".to_string()),
            model: Some("fallback-model".to_string()),
        });
        let events = Arc::new(Mutex::new(Vec::new()));
        let observed = Arc::clone(&events);
        let choice = stream_chat(
            &cfg,
            &[json!({"role":"user", "content":"recover"})],
            &[],
            "auto",
            move |event| observed.lock().expect("record event").push(event),
        )
        .expect("fallback succeeds");

        primary_server.join().expect("primary server");
        fallback_server.join().expect("fallback server");
        assert_eq!(choice.content, "fallback answer");
        assert_eq!(
            choice.provider_namespace.as_deref(),
            Some(OPENAI_COMPATIBLE_CONTINUATION_NAMESPACE)
        );
        assert_eq!(choice.provider_model.as_deref(), Some("fallback-model"));
        assert_eq!(choice.response_model.as_deref(), Some("resolved-model"));
        assert_eq!(choice.response_id.as_deref(), Some("response-42"));
        let events = events.lock().expect("events");
        assert_eq!(
            events
                .iter()
                .filter_map(|event| match event {
                    LlmEvent::TextDelta(text) => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            vec!["fallback answer"]
        );
    }

    #[test]
    fn sse_provider_error_envelope_fails_over_without_retrying_failed_candidate() {
        let primary = TcpListener::bind("127.0.0.1:0").expect("bind primary provider");
        let primary_address = primary.local_addr().expect("primary address");
        let primary_server = thread::spawn(move || {
            let (mut stream, _) = primary.accept().expect("accept primary request");
            read_request(&mut stream);
            write_sse(
                &mut stream,
                concat!(
                    "data: {\"choices\":[],\"error\":{\"code\":502,\"metadata\":{\"error_type\":\"provider_unavailable\"}},\"model\":\"provider/failed-model\"}\n\n",
                    "data: [DONE]\n\n"
                ),
            );
            primary
        });
        let fallback = TcpListener::bind("127.0.0.1:0").expect("bind fallback provider");
        let fallback_address = fallback.local_addr().expect("fallback address");
        let fallback_server = thread::spawn(move || {
            let (mut stream, _) = fallback.accept().expect("accept fallback request");
            read_request(&mut stream);
            write_sse(
                &mut stream,
                concat!(
                    "data: {\"model\":\"provider/working-model\",\"choices\":[{\"delta\":{\"content\":\"recovered\"},\"finish_reason\":null}]}\n\n",
                    "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
                    "data: [DONE]\n\n"
                ),
            );
        });
        let mut cfg = config(format!("http://{primary_address}/v1"));
        cfg.fallbacks.push(LlmProviderTarget {
            provider: None,
            endpoint: format!("http://{fallback_address}/v1"),
            api_key: None,
            model: Some("fallback-model".to_string()),
        });

        let choice = stream_chat(
            &cfg,
            &[json!({"role":"user", "content":"recover"})],
            &[],
            "auto",
            |_| {},
        )
        .expect("fallback succeeds");

        let primary = primary_server.join().expect("primary server");
        fallback_server.join().expect("fallback server");
        primary.set_nonblocking(true).expect("primary nonblocking");
        assert!(matches!(
            primary.accept(),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock
        ));
        assert_eq!(choice.content, "recovered");
        assert_eq!(
            choice.response_model.as_deref(),
            Some("provider/working-model")
        );
    }

    #[test]
    fn visible_primary_output_prevents_cross_provider_replay() {
        let primary = TcpListener::bind("127.0.0.1:0").expect("bind partial provider");
        let primary_address = primary.local_addr().expect("primary address");
        let primary_server = thread::spawn(move || {
            let (mut stream, _) = primary.accept().expect("accept primary request");
            read_request(&mut stream);
            write_truncated_sse(
                &mut stream,
                "data: {\"choices\":[{\"delta\":{\"content\":\"visible primary\"},\"finish_reason\":null}]}\n\n",
            );
        });
        let fallback = TcpListener::bind("127.0.0.1:0").expect("bind forbidden fallback");
        fallback
            .set_nonblocking(true)
            .expect("set fallback nonblocking");
        let fallback_address = fallback.local_addr().expect("fallback address");
        let mut cfg = config(format!("http://{primary_address}/v1"));
        cfg.fallbacks.push(LlmProviderTarget {
            provider: None,
            endpoint: format!("http://{fallback_address}/v1"),
            api_key: None,
            model: None,
        });

        let choice = stream_chat(
            &cfg,
            &[json!({"role":"user", "content":"do not replay"})],
            &[],
            "auto",
            |_| {},
        )
        .expect("visible incomplete response remains the terminal attempt");
        primary_server.join().expect("primary server");
        assert_eq!(choice.content, "visible primary");
        assert_eq!(choice.finish_reason, "stream_incomplete");
        assert!(
            matches!(fallback.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock),
            "fallback received a replay after visible output"
        );
    }

    #[test]
    fn real_http_partial_tool_attempt_is_discarded_before_clean_retry() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind fault provider");
        let address = listener.local_addr().expect("fault provider address");
        let server = thread::spawn(move || {
            let (mut first, _) = listener.accept().expect("accept first provider request");
            read_request(&mut first);
            write_truncated_sse(
                &mut first,
                concat!(
                    "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"partial-call\",\"type\":\"function\",\"function\":{\"name\":\"write_file\",\"arguments\":\"{\\\"path\\\":\"}}]},\"finish_reason\":null}]}\n\n"
                ),
            );
            drop(first);

            let (mut second, _) = listener.accept().expect("accept retry provider request");
            read_request(&mut second);
            write_sse(
                &mut second,
                concat!(
                    "data: {\"choices\":[{\"delta\":{\"content\":\"clean answer\"},\"finish_reason\":null}]}\n\n",
                    "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
                    "data: [DONE]\n\n"
                ),
            );
        });

        let events = Arc::new(Mutex::new(Vec::new()));
        let observed = Arc::clone(&events);
        let choice = stream_chat(
            &config(format!("http://{address}/v1")),
            &[json!({"role":"user", "content":"create nothing"})],
            &[json!({
                "type":"function",
                "function":{
                    "name":"write_file",
                    "description":"test",
                    "parameters":{"type":"object"}
                }
            })],
            "auto",
            move |event| observed.lock().expect("record provider event").push(event),
        )
        .expect("clean retry succeeds");

        server.join().expect("fault provider server");
        assert_eq!(choice.content, "clean answer");
        assert!(
            choice.tool_calls.is_empty(),
            "partial call became executable"
        );
        let events = events.lock().expect("read provider events");
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, LlmEvent::Retrying { .. }))
                .count(),
            1
        );
        assert_eq!(
            events
                .iter()
                .filter_map(|event| match event {
                    LlmEvent::TextDelta(delta) => Some(delta.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            vec!["clean answer"]
        );
    }

    #[test]
    fn real_http_rate_limit_retries_once_without_publishing_partial_output() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind rate-limit provider");
        let address = listener.local_addr().expect("rate-limit provider address");
        let server = thread::spawn(move || {
            let (mut first, _) = listener.accept().expect("accept rate-limited request");
            read_request(&mut first);
            write_error_response(
                &mut first,
                "429 Too Many Requests",
                r#"{"error":{"message":"temporarily rate limited"}}"#,
                Some(0),
            );
            drop(first);

            let (mut second, _) = listener.accept().expect("accept retry request");
            read_request(&mut second);
            write_sse(
                &mut second,
                concat!(
                    "data: {\"choices\":[{\"delta\":{\"content\":\"rate-limit recovered\"},\"finish_reason\":null}]}\n\n",
                    "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
                    "data: [DONE]\n\n"
                ),
            );
        });

        let events = Arc::new(Mutex::new(Vec::new()));
        let observed = Arc::clone(&events);
        let choice = stream_chat(
            &config(format!("http://{address}/v1")),
            &[json!({"role":"user", "content":"retry safely"})],
            &[],
            "auto",
            move |event| observed.lock().expect("record provider event").push(event),
        )
        .expect("rate-limited request recovers");

        server.join().expect("rate-limit provider server");
        assert_eq!(choice.content, "rate-limit recovered");
        let events = events.lock().expect("read provider events");
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, LlmEvent::Retrying { .. }))
                .count(),
            1
        );
        assert_eq!(
            events
                .iter()
                .filter_map(|event| match event {
                    LlmEvent::TextDelta(delta) => Some(delta.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            vec!["rate-limit recovered"]
        );
    }

    #[test]
    fn exhausted_daily_quota_skips_same_provider_retry_and_uses_next_provider() {
        let primary = TcpListener::bind("127.0.0.1:0").expect("bind primary provider");
        let fallback = TcpListener::bind("127.0.0.1:0").expect("bind fallback provider");
        let primary_address = primary.local_addr().expect("primary address");
        let fallback_address = fallback.local_addr().expect("fallback address");
        let primary_server = thread::spawn(move || {
            let (mut stream, _) = primary.accept().expect("accept primary request");
            read_request(&mut stream);
            write_error_response(
                &mut stream,
                "429 Too Many Requests",
                r#"{"error":{"metadata":{"headers":{"X-RateLimit-Remaining":"0","X-RateLimit-Reset":"1789257600000"},"limit_source":"daily"}}}"#,
                None,
            );
        });
        let fallback_server = thread::spawn(move || {
            let (mut stream, _) = fallback.accept().expect("accept fallback request");
            read_request(&mut stream);
            write_sse(
                &mut stream,
                concat!(
                    "data: {\"choices\":[{\"delta\":{\"content\":\"fallback answer\"},\"finish_reason\":null}]}\n\n",
                    "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
                    "data: [DONE]\n\n"
                ),
            );
        });

        let mut cfg = config(format!("http://{primary_address}/v1"));
        cfg.fallbacks = vec![LlmProviderTarget {
            provider: None,
            endpoint: format!("http://{fallback_address}/v1"),
            api_key: None,
            model: Some("fallback-model".into()),
        }];
        let events = Arc::new(Mutex::new(Vec::new()));
        let observed = Arc::clone(&events);
        let choice = stream_chat(
            &cfg,
            &[json!({"role":"user", "content":"hello"})],
            &[],
            "none",
            move |event| observed.lock().expect("record event").push(event),
        )
        .expect("fallback succeeds");

        primary_server.join().expect("primary server");
        fallback_server.join().expect("fallback server");
        assert_eq!(choice.content, "fallback answer");
        assert_eq!(choice.provider_model.as_deref(), Some("fallback-model"));
        let retries = events
            .lock()
            .expect("read events")
            .iter()
            .filter_map(|event| match event {
                LlmEvent::Retrying { attempt, .. } => Some(*attempt),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(retries, vec![2]);
    }

    #[test]
    fn typed_context_overflow_bypasses_host_retry_despite_transient_http_status() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind overflow provider");
        let address = listener.local_addr().expect("overflow provider address");
        let server = thread::spawn(move || {
            let (mut first, _) = listener.accept().expect("accept overflow request");
            read_request(&mut first);
            write_error_response(
                &mut first,
                "503 Service Unavailable",
                r#"{"errors":[{"code":"gateway_failure"},{"type":"prompt_too_long","message":"入力が長すぎます"}]}"#,
                Some(0),
            );
            drop(first);

            listener
                .set_nonblocking(true)
                .expect("inspect overflow retries");
            let deadline = std::time::Instant::now() + Duration::from_millis(350);
            while std::time::Instant::now() < deadline {
                match listener.accept() {
                    Ok(_) => panic!("typed context overflow triggered a Host-private retry"),
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("inspect overflow retry: {error}"),
                }
            }
        });

        let events = Arc::new(Mutex::new(Vec::new()));
        let observed = Arc::clone(&events);
        let error = stream_chat(
            &config(format!("http://{address}/v1")),
            &[json!({"role":"user", "content":"long canonical turn"})],
            &[],
            "none",
            move |event| observed.lock().expect("record overflow event").push(event),
        )
        .expect_err("typed overflow must return to Core for compaction");

        server.join().expect("overflow provider server");
        assert!(error.contains("prompt_too_long"));
        assert_eq!(
            events
                .lock()
                .expect("read overflow events")
                .iter()
                .filter(|event| matches!(event, LlmEvent::Retrying { .. }))
                .count(),
            0
        );
    }

    #[test]
    fn real_http_unauthorized_fails_without_retry_or_visible_output() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind unauthorized provider");
        let address = listener
            .local_addr()
            .expect("unauthorized provider address");
        let server = thread::spawn(move || {
            let (mut first, _) = listener.accept().expect("accept unauthorized request");
            read_request(&mut first);
            write_error_response(
                &mut first,
                "401 Unauthorized",
                r#"{"error":{"message":"invalid test credential"}}"#,
                None,
            );
            drop(first);

            listener
                .set_nonblocking(true)
                .expect("make unauthorized listener nonblocking");
            let deadline = std::time::Instant::now() + Duration::from_millis(500);
            while std::time::Instant::now() < deadline {
                match listener.accept() {
                    Ok(_) => panic!("401 response triggered a retry"),
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("inspect unauthorized provider: {error}"),
                }
            }
        });

        let events = Arc::new(Mutex::new(Vec::new()));
        let observed = Arc::clone(&events);
        let error = stream_chat(
            &config(format!("http://{address}/v1")),
            &[json!({"role":"user", "content":"do not retry auth"})],
            &[],
            "auto",
            move |event| observed.lock().expect("record provider event").push(event),
        )
        .expect_err("401 must fail immediately");

        server.join().expect("unauthorized provider server");
        assert!(error.contains("401 Unauthorized"), "{error}");
        let events = events.lock().expect("read provider events");
        assert!(
            events
                .iter()
                .all(|event| matches!(event, LlmEvent::SamplingStarted { .. })),
            "401 published provider events: {events:?}"
        );
    }

    fn assert_real_http_visible_partial_is_not_retried(
        delta: &'static str,
        expected_content: &str,
        expected_reasoning: &str,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind visible partial provider");
        let address = listener
            .local_addr()
            .expect("visible partial provider address");
        let server = thread::spawn(move || {
            let (mut first, _) = listener.accept().expect("accept visible partial request");
            read_request(&mut first);
            write_truncated_sse(&mut first, delta);
            drop(first);

            listener
                .set_nonblocking(true)
                .expect("make visible partial listener nonblocking");
            let deadline = std::time::Instant::now() + Duration::from_millis(750);
            while std::time::Instant::now() < deadline {
                match listener.accept() {
                    Ok(_) => panic!("visible provider delta triggered a whole-request retry"),
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("inspect visible partial provider: {error}"),
                }
            }
        });

        let events = Arc::new(Mutex::new(Vec::new()));
        let observed = Arc::clone(&events);
        let choice = stream_chat(
            &config(format!("http://{address}/v1")),
            &[json!({"role":"user", "content":"inspect only"})],
            &[json!({
                "type":"function",
                "function":{
                    "name":"read_file",
                    "description":"test",
                    "parameters":{"type":"object"}
                }
            })],
            "auto",
            move |event| observed.lock().expect("record visible event").push(event),
        )
        .expect("visible partial stream is retained for continuation");

        server.join().expect("visible partial provider server");
        assert_eq!(choice.finish_reason, "stream_incomplete");
        assert_eq!(choice.content, expected_content);
        assert_eq!(choice.reasoning, expected_reasoning);
        assert_eq!(
            events
                .lock()
                .expect("read visible events")
                .iter()
                .filter(|event| matches!(event, LlmEvent::Retrying { .. }))
                .count(),
            0
        );
    }

    #[test]
    fn anthropic_sse_context_overflow_returns_to_core_without_host_retry() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind Anthropic provider");
        let address = listener.local_addr().expect("Anthropic provider address");
        let server = thread::spawn(move || {
            let (mut first, _) = listener.accept().expect("accept Anthropic request");
            read_request(&mut first);
            write_sse(
                &mut first,
                concat!(
                    "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":120000}}}\n\n",
                    "data: {\"type\":\"error\",\"error\":{\"type\":\"prompt_too_long\",\"message\":\"入力が長すぎます\"}}\n\n"
                ),
            );
            drop(first);

            listener
                .set_nonblocking(true)
                .expect("inspect Anthropic retries");
            let deadline = std::time::Instant::now() + Duration::from_millis(350);
            while std::time::Instant::now() < deadline {
                match listener.accept() {
                    Ok(_) => panic!("Anthropic typed overflow triggered a Host-private retry"),
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("inspect Anthropic retry: {error}"),
                }
            }
        });

        let events = Arc::new(Mutex::new(Vec::new()));
        let observed = Arc::clone(&events);
        let mut cfg = config(format!("http://{address}"));
        cfg.provider = Some("anthropic".to_string());
        let error = stream_chat(
            &cfg,
            &[json!({"role":"user", "content":"continue canonical work"})],
            &[],
            "none",
            move |event| observed.lock().expect("record Anthropic event").push(event),
        )
        .expect_err("typed Anthropic overflow must return to Core");

        server.join().expect("Anthropic provider server");
        assert_eq!(
            super::structured_provider_error_is_context_limit(&error),
            Some(true)
        );
        assert_eq!(
            events
                .lock()
                .expect("read Anthropic events")
                .iter()
                .filter(|event| matches!(event, LlmEvent::Retrying { .. }))
                .count(),
            0
        );
    }

    #[test]
    fn compatible_sse_wrapped_gateway_failure_never_completes_successfully() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            read_request(&mut stream);
            let upstream = json!({"error":{"code":"data_inspection_failed","message":"blocked","type":"DATA_INSPECTION_FAILED"}}).to_string();
            let escaped = serde_json::to_string(&upstream).unwrap();
            let content = json!({"message":format!("Internal service error: 400 - {}", &escaped[1..escaped.len()-1])}).to_string();
            let mut frames = String::new();
            for chunk in content.as_bytes().chunks(13) {
                let delta = json!({"choices":[{"delta":{"content":std::str::from_utf8(chunk).unwrap()},"finish_reason":null}]});
                frames.push_str(&format!("data: {delta}\n\n"));
            }
            frames.push_str("data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n");
            write_sse(&mut stream, &frames);
        });
        let mut published = Vec::new();
        let error = stream_chat(
            &config(format!("http://{address}/v1")),
            &[json!({"role":"user","content":"hello"})],
            &[], "none", |event| published.push(event),
        ).expect_err("a stop frame cannot turn an upstream failure into an answer");
        server.join().unwrap();
        assert_eq!(super::model_failure_projection(&error).code, "content_filtered");
        assert!(!published.iter().any(|event| matches!(event, LlmEvent::TextDelta(_))));
    }

    #[test]
    fn compatible_sse_context_overflow_is_not_an_empty_completion() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind compatible provider");
        let address = listener.local_addr().expect("compatible provider address");
        let server = thread::spawn(move || {
            let (mut first, _) = listener.accept().expect("accept compatible request");
            read_request(&mut first);
            write_sse(
                &mut first,
                concat!(
                    "data: {\"id\":\"response-before-error\",\"choices\":[]}\n\n",
                    "data: {\"errors\":[{\"code\":\"gateway_failure\"},{\"type\":\"context_length_exceeded\",\"message\":\"入力が長すぎます\"}]}\n\n"
                ),
            );
            drop(first);

            listener
                .set_nonblocking(true)
                .expect("inspect compatible retries");
            let deadline = std::time::Instant::now() + Duration::from_millis(350);
            while std::time::Instant::now() < deadline {
                match listener.accept() {
                    Ok(_) => panic!("SSE typed overflow triggered a Host-private retry"),
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("inspect compatible retry: {error}"),
                }
            }
        });

        let events = Arc::new(Mutex::new(Vec::new()));
        let observed = Arc::clone(&events);
        let error = stream_chat(
            &config(format!("http://{address}/v1")),
            &[json!({"role":"user", "content":"continue canonical work"})],
            &[],
            "none",
            move |event| {
                observed
                    .lock()
                    .expect("record compatible event")
                    .push(event)
            },
        )
        .expect_err("typed SSE overflow must return to Core instead of completing empty");

        server.join().expect("compatible provider server");
        assert_eq!(
            super::structured_provider_error_is_context_limit(&error),
            Some(true)
        );
        assert_eq!(
            events
                .lock()
                .expect("read compatible events")
                .iter()
                .filter(|event| matches!(event, LlmEvent::Retrying { .. }))
                .count(),
            0
        );
    }

    #[test]
    fn compatible_sse_null_empty_or_malformed_errors_do_not_abort_completion() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind compatible provider");
        let address = listener.local_addr().expect("compatible provider address");
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept compatible request");
            read_request(&mut stream);
            write_sse(
                &mut stream,
                concat!(
                    "data: not-json\n\n",
                    "data: {\"error\":null,\"errors\":[]}\n\n",
                    "data: {\"error\":\"malformed\"}\n\n",
                    "data: {\"errors\":{}}\n\n",
                    "data: {\"errors\":\"malformed\"}\n\n",
                    "data: {\"choices\":[{\"delta\":{\"content\":\"completed\"},\"finish_reason\":\"stop\"}]}\n\n",
                    "data: [DONE]\n\n"
                ),
            );
        });

        let choice = stream_chat(
            &config(format!("http://{address}/v1")),
            &[json!({"role":"user", "content":"continue canonical work"})],
            &[],
            "none",
            |_| {},
        )
        .expect("non-error envelopes must not abort a valid completion");

        server.join().expect("compatible provider server");
        assert_eq!(choice.content, "completed");
        assert_eq!(choice.finish_reason, "stop");
    }

    #[test]
    fn real_http_visible_text_partial_is_not_whole_request_retried() {
        assert_real_http_visible_partial_is_not_retried(
            "data: {\"choices\":[{\"delta\":{\"content\":\"visible text\"},\"finish_reason\":null}]}\n\n",
            "visible text",
            "",
        );
    }

    #[test]
    fn real_http_visible_reasoning_partial_is_not_whole_request_retried() {
        assert_real_http_visible_partial_is_not_retried(
            "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"visible thought\"},\"finish_reason\":null}]}\n\n",
            "",
            "visible thought",
        );
    }
}

/// Applies the native-tool contract even at a tool-free synthesis boundary.
///
/// Several OpenAI-compatible providers retain enough conversational tool
/// history to emit another native call when `tools` is omitted. Core uses an
/// empty projection as an explicit protocol boundary, so the transport must
/// preserve that intent with `tool_choice: none` rather than relying on an
/// absent field to imply it.
fn apply_openai_tool_contract(body: &mut Value, tools: &[Value], tool_choice: &str) {
    if tools.is_empty() {
        if tool_choice == "none" {
            body["tool_choice"] = Value::String("none".to_string());
        }
        return;
    }
    body["tools"] = Value::Array(tools.to_vec());
    let normalized = match tool_choice {
        "required" => "required",
        "none" => "none",
        _ => "auto",
    };
    body["tool_choice"] = Value::String(normalized.to_string());
}

#[cfg(test)]
mod openai_tool_contract_tests {
    use super::apply_openai_tool_contract;
    use serde_json::{json, Value};

    #[test]
    fn empty_core_projection_is_transmitted_as_explicit_tool_free_boundary() {
        let mut body = json!({"messages": []});
        apply_openai_tool_contract(&mut body, &[], "none");
        assert_eq!(body["tool_choice"], Value::String("none".to_string()));
        assert!(body.get("tools").is_none());
    }

    #[test]
    fn executable_projection_keeps_tools_and_requested_choice() {
        let mut body = json!({"messages": []});
        let tools = vec![json!({
            "type": "function",
            "function": {"name": "web_search", "parameters": {"type": "object"}}
        })];
        apply_openai_tool_contract(&mut body, &tools, "auto");
        assert_eq!(body["tools"], Value::Array(tools));
        assert_eq!(body["tool_choice"], Value::String("auto".to_string()));
    }
}

pub(super) fn stream_openai_compatible_chat<F>(
    cfg: &LlmConfig,
    messages: &[Value],
    tools: &[Value],
    tool_choice: &str,
    on_event: F,
) -> Result<StreamedChoice, ProviderAttemptError>
where
    F: FnMut(LlmEvent) -> Result<(), String>,
{
    let body = openai_request_body(cfg, messages, tools, tool_choice)?;
    let started = Instant::now();
    let result = stream_openai_body(cfg, &body, true, on_event);
    super::contracts::retain_request(result, cfg, body, started)
}

pub(super) fn openai_request_body(
    cfg: &LlmConfig, messages: &[Value], tools: &[Value], tool_choice: &str,
) -> Result<Value, String> {
    let normalized_messages = normalize_openai_compatible_messages(messages);
    if let Some(selection) = &cfg.reasoning_selection {
        selection.validate(effective_max_output_tokens(cfg)?)?;
        if matches!(
            selection,
            nola_core::agent::reasoning::ReasoningSelection::Budget { .. }
        ) {
            return Err(
                "Chat Completions adapter does not support a numeric thinking budget".into(),
            );
        }
    }
    let mut body = json!({
        "messages": normalized_messages,
        "stream": true,
    });
    body[output_token_parameter_name(cfg)] = json!(effective_max_output_tokens(cfg)?);
    if cfg.sampling_override {
        body["temperature"] = json!(cfg.temperature.clamp(0.0, 2.0));
        body["top_p"] = json!(cfg.top_p.clamp(0.0, 1.0));
    }
    if !cfg.model.trim().is_empty() {
        body["model"] = Value::String(cfg.model.clone());
    }
    apply_thinking_policy(cfg, &mut body);
    cfg.generation_options.validate()?;
    if let Value::Object(options) =
        serde_json::to_value(&cfg.generation_options).map_err(|e| e.to_string())?
    {
        body.as_object_mut().unwrap().extend(options);
    }
    if let Some(nola_core::agent::reasoning::ReasoningSelection::Effort { value }) =
        &cfg.reasoning_selection
    {
        body["reasoning_effort"] = json!(value);
    }
    if matches!(
        detect_chat_provider(cfg.provider.as_deref(), &cfg.endpoint, &cfg.model),
        ChatProvider::OpenAiCompatible(OpenAiFlavor::OpenAi)
    ) && cfg.openai_prompt_cache
    {
        if let Some(prompt_cache_key) = cfg
            .openai_prompt_cache_key
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            body["prompt_cache_key"] = Value::String(prompt_cache_key.to_string());

            if let Some(retention) = cfg
                .openai_prompt_cache_retention
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                body["prompt_cache_retention"] = Value::String(retention.to_string());
            }
        }
    }
    apply_openai_tool_contract(&mut body, tools, tool_choice);
    if !tools.is_empty() {
        if let Some(parallel) = cfg.parallel_tool_calls {
            body["parallel_tool_calls"] = Value::Bool(parallel);
        }
    }

    if tool_choice == "required" && non_stream_required_tools_enabled(cfg) {
        body["stream"] = Value::Bool(false);
    }
    if body["stream"] != false && matches!(
        detect_chat_provider(cfg.provider.as_deref(), &cfg.endpoint, &cfg.model),
        ChatProvider::OpenAiCompatible(OpenAiFlavor::OpenAi)
    ) {
        body["stream_options"] = json!({"include_usage": true});
    }
    debug_context_metrics(
        "openai_compatible",
        &normalized_messages,
        tools,
        tool_choice,
        &body,
    );
    Ok(body)
}

pub(super) fn stream_openai_body<F>(
    cfg: &LlmConfig, body: &Value, allow_transport_fallback: bool, mut on_event: F,
) -> Result<StreamedChoice, ProviderAttemptError>
where F: FnMut(LlmEvent) -> Result<(), String> {
    let normalized_messages = body["messages"].as_array().map(Vec::as_slice).unwrap_or_default();
    let tools = body["tools"].as_array().map(Vec::as_slice).unwrap_or_default();
    let tool_choice = body["tool_choice"].as_str().unwrap_or("auto");
    let endpoint = normalize_chat_endpoint(&cfg.endpoint)?;
    if body["stream"] == false {
        return complete_openai_compatible_chat(cfg, body, &endpoint, allow_transport_fallback, on_event);
    }
    log_llm_debug_event(
        cfg,
        "request_start",
        &endpoint,
        body["messages"].as_array().map(Vec::as_slice).unwrap_or_default(),
        body["tools"].as_array().map(Vec::as_slice).unwrap_or_default(),
        body["tool_choice"].as_str().unwrap_or("auto"),
        "mode=stream",
    );

    let parsed_endpoint = reqwest::Url::parse(&endpoint)
        .map_err(|err| format!("{} {err}", tr(Message::LlmRequestFailedPrefix)))?;
    let endpoint_host = parsed_endpoint
        .host_str()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let endpoint_port = parsed_endpoint.port_or_known_default().unwrap_or_else(|| {
        if parsed_endpoint.scheme() == "https" {
            443
        } else {
            80
        }
    });
    let fallback_addresses = if endpoint_host.is_empty() || !allow_transport_fallback {
        Vec::new()
    } else {
        resolve_dns_fallbacks_for_host(&endpoint_host, endpoint_port)
    };
    let has_fallback_addresses = !fallback_addresses.is_empty();

    let mut last_err: Option<String> = None;
    let mut attempts: Vec<Option<SocketAddr>> = Vec::with_capacity(fallback_addresses.len() + 1);
    attempts.push(None);
    for address in fallback_addresses.iter().copied() {
        attempts.push(Some(address));
    }

    for (attempt_index, resolved_addr) in attempts.into_iter().enumerate() {
        let client = if cfg.native_model_route.is_some() {
            super::transport::build_native_stream_client()?
        } else { build_stream_client(parsed_endpoint.host_str(), resolved_addr)? };
        let mut request = client
            .post(&endpoint)
            .header(CONTENT_TYPE, "application/json")
            .header(ACCEPT, "text/event-stream")
            .json(&body);
        if !cfg.api_key.trim().is_empty() {
            request = request.header(AUTHORIZATION, format!("Bearer {}", cfg.api_key));
        }

        if let Some(route) = &cfg.native_model_route {
            crate::desktop_host_control::validate_native_model_channel(
                route, &cfg.endpoint, &cfg.model, &cfg.api_key).map_err(ProviderAttemptError::Admission)?;
        }
        super::diagnostics::record_prepared_request(&request, &[cfg.api_key.as_str()]);
        let response = match send_request_with_cancellation(request, &cfg.cancellation) {
            SendRequestResult::Response(response) => response,
            SendRequestResult::Error(err) => {
                let err_text = classify_network_error_hint(&err);
                last_err = Some(err_text.clone());
                let lower_error = err.to_string().to_ascii_lowercase();
                log_llm_debug_event(
                    cfg,
                    "request_send_error",
                    &endpoint,
                    &normalized_messages,
                    tools,
                    tool_choice,
                    &format!(
                        "attempt={attempt_index} fallback_addr={:?} error={err_text}",
                        resolved_addr
                    ),
                );
                if allow_transport_fallback && should_fallback_non_stream_error_message(&lower_error) && resolved_addr.is_none()
                {
                    let mut fallback_body = body.clone();
                    if let Some(stream) = fallback_body.get_mut("stream") {
                        *stream = Value::Bool(false);
                    }
                    log_llm_debug_event(
                        cfg,
                        "request_fallback",
                        &endpoint,
                        &normalized_messages,
                        tools,
                        tool_choice,
                        "reason=send_error_connection_rejected_or_closed",
                    );
                    return complete_openai_compatible_chat(
                        cfg,
                        &fallback_body,
                        &endpoint,
                        allow_transport_fallback,
                        &mut on_event,
                    );
                }

                let should_retry = resolved_addr.is_none() && has_fallback_addresses;
                if should_retry {
                    continue;
                }
                return Err(
                    format!("{}: {}", tr(Message::LlmRequestFailedPrefix), err_text).into(),
                );
            }
            SendRequestResult::WorkerDisconnected => return Err(ProviderAttemptError::WorkerDisconnected),
            SendRequestResult::Cancelled => return Err(llm_cancelled_message().into()),
        };

        let status = response.status();
        if !status.is_success() {
            let retry_after_ms = response_retry_after_ms(&response);
            let detail = response.text().unwrap_or_default();
            log_llm_debug_event(
                cfg,
                "request_http_error",
                &endpoint,
                &normalized_messages,
                tools,
                tool_choice,
                &format!(
                    "attempt={attempt_index} status={status} body={}",
                    trim_debug_detail(&detail)
                ),
            );
            return Err(ProviderAttemptError::Http {
                status: status.as_u16(),
                retry_after_ms,
                detail,
            });
        }

        let stream_idle_timeout = stream_idle_timeout();
        let stream_first_event_timeout = stream_first_event_timeout(stream_idle_timeout);
        let stream_tail_idle_timeout = stream_tail_idle_timeout();
        let read_failed_prefix = tr(Message::LlmStreamReadFailedPrefix).to_string();
        // Bound read-ahead so a fast provider cannot enqueue an entire answer
        // before the terminal cancellation listener gets scheduled.
        let (line_tx, line_rx) = mpsc::sync_channel::<Result<String, String>>(32);
        let reader_cancellation = cfg.cancellation.clone();
        thread::spawn(move || {
            let reader = BufReader::new(response);
            for line in reader.lines() {
                if reader_cancellation.is_requested() {
                    break;
                }
                match line {
                    Ok(line) => {
                        if !line.starts_with("data: ") {
                            continue;
                        }
                        if line_tx.send(Ok(line)).is_err() {
                            break;
                        }
                    }
                    Err(err) => {
                        let _ = line_tx.send(Err(format!("{read_failed_prefix} {err}")));
                        break;
                    }
                }
            }
        });
        let mut choice = StreamedChoice::default();
        let mut slots: BTreeMap<usize, ToolSlot> = BTreeMap::new();
        let mut text_tool_protocol = TextToolProtocolBuffer::default();
        let mut gateway_envelope = GatewayEnvelopeBuffer::default();
        let mut saw_stream_data = false;
        let mut saw_terminal_event = false;
        let mut observed_progress_size = 0usize;
        let mut last_meaningful_progress_at = std::time::Instant::now();

        loop {
            let progress_size = stream_progress_size(&choice, &slots) + gateway_envelope.pending.len();
            if progress_size != observed_progress_size {
                observed_progress_size = progress_size;
                last_meaningful_progress_at = std::time::Instant::now();
            }
            let has_progress = stream_has_progress(&choice, &slots);
            if has_progress && last_meaningful_progress_at.elapsed() >= stream_tail_idle_timeout {
                break;
            }
            let mut recv_timeout = if has_progress {
                stream_tail_idle_timeout
            } else {
                stream_first_event_timeout
            };
            if has_progress {
                recv_timeout = recv_timeout.min(
                    stream_tail_idle_timeout.saturating_sub(last_meaningful_progress_at.elapsed()),
                );
            }
            let line = match recv_stream_line(&line_rx, recv_timeout, &cfg.cancellation) {
                StreamLineRecv::Line(line) => line,
                StreamLineRecv::Error(err) => {
                    log_llm_debug_event(
                        cfg,
                        "stream_read_error",
                        &endpoint,
                        &normalized_messages,
                        tools,
                        tool_choice,
                        &format!("attempt={attempt_index} error={err}"),
                    );
                    if should_treat_stream_read_as_complete(&err) {
                        if saw_stream_data {
                            log_llm_debug_event(
                                cfg,
                                "stream_read_recovery",
                                &endpoint,
                                &normalized_messages,
                                tools,
                                tool_choice,
                                "mark_buffered_content_incomplete",
                            );
                            break;
                        }
                        if allow_transport_fallback && should_fallback_non_stream_error_message(&err.to_ascii_lowercase())
                            && resolved_addr.is_none()
                        {
                            let mut fallback_body = body.clone();
                            if let Some(stream) = fallback_body.get_mut("stream") {
                                *stream = Value::Bool(false);
                            }
                            log_llm_debug_event(
                                cfg,
                                "request_fallback",
                                &endpoint,
                                &normalized_messages,
                                tools,
                                tool_choice,
                                "reason=stream_read_connection_closed_before_complete",
                            );
                            return complete_openai_compatible_chat(
                                cfg,
                                &fallback_body,
                                &endpoint,
                                allow_transport_fallback,
                                &mut on_event,
                            );
                        }
                    }
                    return Err(err.into());
                }
                StreamLineRecv::Timeout => {
                    if stream_has_progress(&choice, &slots) {
                        break;
                    }
                    return Err(format!(
                        "{} stream idle timeout after {}s",
                        tr(Message::LlmStreamReadFailedPrefix),
                        recv_timeout.as_secs()
                    )
                    .into());
                }
                StreamLineRecv::Disconnected => {
                    if saw_stream_data || stream_has_progress(&choice, &slots) {
                        break;
                    }
                    return Err("LLM connection closed before the first stream event"
                        .to_string()
                        .into());
                }
                StreamLineRecv::Cancelled => return Err(llm_cancelled_message().into()),
            };
            let raw = line.trim_start_matches("data: ").trim();
            if raw.is_empty() {
                continue;
            }
            if raw == "[DONE]" {
                saw_terminal_event = true;
                break;
            }
            saw_stream_data = true;
            let parsed: Value = match serde_json::from_str(raw) {
                Ok(value) => value,
                Err(_) => continue,
            };
            let has_error_envelope = ["error", "errors"].into_iter().any(|key| {
                parsed.get(key).is_some_and(|errors| match errors {
                    Value::Array(values) => !values.is_empty(),
                    Value::Object(object) => !object.is_empty(),
                    _ => false,
                })
            });
            if has_error_envelope {
                // Compatible gateways can return a terminal typed failure
                // after establishing an HTTP 200 SSE stream. Preserve the
                // envelope before the missing `choices` field can turn it
                // into an empty or incomplete success.
                return Err(serde_json::to_string(&parsed)
                    .unwrap_or_else(|_| "{\"error\":{}}".to_string())
                    .into());
            }
            merge_usage_from_envelope(&mut choice.usage, &parsed);
            capture_openai_response_identity(&mut choice, &parsed);
            let Some(delta) = parsed
                .get("choices")
                .and_then(Value::as_array)
                .and_then(|choices| choices.first())
                .and_then(|choice| choice.get("delta"))
            else {
                continue;
            };

            if let Some(text) = delta.get("reasoning_content").and_then(Value::as_str) {
                if !text.is_empty() {
                    choice.reasoning.push_str(text);
                    on_event(LlmEvent::ReasoningDelta(text.to_string()))?;
                }
            }
            // A few OpenAI-compatible providers mirror a reasoning prefix into
            // `content` in the same SSE frame. Publish reasoning first so the
            // presentation gate can reconcile both channels before showing it.
            if let Some(text) = delta.get("content").and_then(Value::as_str) {
                if !text.is_empty() {
                    if let Some(visible) = gateway_envelope.push(text) {
                        if !text_tool_protocol.push(&mut choice, &visible, &mut on_event)? {
                            break;
                        }
                    }
                }
            }

            for tool_delta in delta
                .get("tool_calls")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if let Some(event) = append_openai_tool_delta(&mut slots, tool_delta)? {
                    on_event(event)?;
                }
            }

            if let Some(reason) = parsed
                .get("choices")
                .and_then(Value::as_array)
                .and_then(|choices| choices.first())
                .and_then(|choice| choice.get("finish_reason"))
                .and_then(Value::as_str)
            {
                choice.finish_reason = reason.to_string();
                if !reason.trim().is_empty() {
                    saw_terminal_event = true;
                }
            }
            thread::yield_now();
            if cfg.cancellation.is_requested() {
                return Err(llm_cancelled_message().into());
            }
        }

        if !gateway_envelope.pending.is_empty() {
            if slots.is_empty() && choice.reasoning.is_empty() {
                reject_wrapped_gateway_error(&StreamedChoice {
                    content: gateway_envelope.pending.clone(),
                    ..Default::default()
                })?;
            }
            text_tool_protocol.push(&mut choice, &gateway_envelope.pending, &mut on_event)?;
        }
        let text_tool_calls = text_tool_protocol.finish(&mut choice, &mut on_event)?;
        let native_tool_calls = slots
            .into_iter()
            .enumerate()
            .filter_map(|(fallback_index, (_slot_index, slot))| {
                if slot.name.trim().is_empty() {
                    return None;
                }
                Some(ToolCall {
                    id: if slot.id.trim().is_empty() {
                        format!("call_{fallback_index}")
                    } else {
                        slot.id
                    },
                    name: slot.name,
                    arguments: if slot.arguments.trim().is_empty() {
                        "{}".to_string()
                    } else {
                        normalize_tool_arguments_text(&slot.arguments)
                    },
                })
            })
            .collect::<Vec<_>>();
        choice.tool_calls = if native_tool_calls.is_empty() {
            text_tool_calls.unwrap_or_default()
        } else {
            native_tool_calls
        };
        finalize_stream_completion(&mut choice, saw_terminal_event)?;
        reject_wrapped_gateway_error(&choice)?;

        log_llm_debug_event(
            cfg,
            "request_done",
            &endpoint,
            &normalized_messages,
            tools,
            tool_choice,
            &format!(
                "status=ok attempts={attempt_index} streamed={} tools={}",
                choice.content.len(),
                choice.tool_calls.len()
            ),
        );

        return Ok(choice);
    }

    Err(format!(
        "{}: {}",
        tr(Message::LlmRequestFailedPrefix),
        last_err.unwrap_or_else(|| "unknown stream error".to_string())
    )
    .into())
}

fn should_treat_stream_read_as_complete(raw_error: &str) -> bool {
    should_fallback_non_stream_error_message(&raw_error.to_ascii_lowercase())
}

const CANONICAL_TOOL_CALLS_OPEN: &str = "<tool_calls>";

#[derive(Debug, Clone, Copy)]
struct TextToolProtocolDialect {
    namespace: Option<&'static str>,
    root_element: &'static str,
}

const TEXT_TOOL_PROTOCOL_DIALECTS: [TextToolProtocolDialect; 2] = [
    TextToolProtocolDialect {
        namespace: Some("｜｜DSML｜｜"),
        root_element: "tool_calls",
    },
    TextToolProtocolDialect {
        namespace: None,
        root_element: "tool_calls",
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TextToolProtocolTag {
    Reserved,
    Possible,
    Ordinary,
}

/// Classifies an XML-like tag from the text-tool protocol grammar.
///
/// This deliberately does not enumerate malformed provider spellings. Once a
/// tag selects a registered protocol namespace (DSML), the complete frame is
/// control data and the protocol parser owns validation. The unnamespaced
/// canonical dialect is selected by its root element. Partial tags remain
/// buffered so transport chunk boundaries cannot leak them as answer text.
fn classify_text_tool_protocol_tag(candidate: &str) -> TextToolProtocolTag {
    let Some(mut tag) = candidate.strip_prefix('<') else {
        return TextToolProtocolTag::Ordinary;
    };
    tag = tag.strip_prefix('/').unwrap_or(tag);

    for dialect in TEXT_TOOL_PROTOCOL_DIALECTS {
        if let Some(namespace) = dialect.namespace {
            if namespace.starts_with(tag) {
                return TextToolProtocolTag::Possible;
            }
            if let Some(namespaced) = tag.strip_prefix(namespace) {
                return if namespaced.contains('>') {
                    TextToolProtocolTag::Reserved
                } else {
                    TextToolProtocolTag::Possible
                };
            }
            continue;
        }

        let root = tag
            .split(|character: char| character.is_whitespace() || matches!(character, '>' | '/'))
            .next()
            .unwrap_or_default();
        if root == dialect.root_element {
            return if tag.contains('>') {
                TextToolProtocolTag::Reserved
            } else {
                TextToolProtocolTag::Possible
            };
        }
        if !root.is_empty() && dialect.root_element.starts_with(root) && !tag.contains('>') {
            return TextToolProtocolTag::Possible;
        }
    }
    TextToolProtocolTag::Ordinary
}

fn text_tool_protocol_boundary(text: &str) -> (Option<usize>, usize) {
    let mut retained_suffix = 0;
    for (index, character) in text.char_indices() {
        if character != '<' {
            continue;
        }
        match classify_text_tool_protocol_tag(&text[index..]) {
            TextToolProtocolTag::Reserved => return (Some(index), 0),
            TextToolProtocolTag::Possible => retained_suffix = text.len() - index,
            TextToolProtocolTag::Ordinary => {}
        }
    }
    (None, retained_suffix)
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum TextToolProtocolMode {
    #[default]
    Undecided,
    Text,
    Protocol,
}

#[derive(Debug, Default)]
struct TextToolProtocolBuffer {
    mode: TextToolProtocolMode,
    pending: String,
    protocol: String,
}

impl TextToolProtocolBuffer {
    fn push<F>(
        &mut self,
        choice: &mut StreamedChoice,
        text: &str,
        on_event: &mut F,
    ) -> Result<bool, String>
    where
        F: FnMut(LlmEvent) -> Result<(), String>,
    {
        match self.mode {
            TextToolProtocolMode::Protocol => {
                self.protocol.push_str(text);
                Ok(true)
            }
            TextToolProtocolMode::Text | TextToolProtocolMode::Undecided => {
                self.pending.push_str(text);
                let (protocol_position, retained_suffix_length) =
                    text_tool_protocol_boundary(&self.pending);
                if let Some(position) = protocol_position {
                    let visible = self.pending[..position].to_string();
                    self.protocol.push_str(&self.pending[position..]);
                    self.pending.clear();
                    self.mode = TextToolProtocolMode::Protocol;
                    return append_choice_text(choice, &visible, on_event);
                }

                let emit_length = self.pending.len().saturating_sub(retained_suffix_length);
                if emit_length == 0 {
                    return Ok(true);
                }
                let visible = self.pending[..emit_length].to_string();
                self.pending.drain(..emit_length);
                self.mode = TextToolProtocolMode::Text;
                append_choice_text(choice, &visible, on_event)
            }
        }
    }

    fn finish<F>(
        &mut self,
        choice: &mut StreamedChoice,
        on_event: &mut F,
    ) -> Result<Option<Vec<ToolCall>>, String>
    where
        F: FnMut(LlmEvent) -> Result<(), String>,
    {
        if self.mode == TextToolProtocolMode::Undecided && !self.pending.is_empty() {
            self.mode = TextToolProtocolMode::Text;
            let pending = std::mem::take(&mut self.pending);
            append_choice_text(choice, &pending, on_event)?;
        }
        if self.mode != TextToolProtocolMode::Protocol {
            return Ok(None);
        }
        if let Some(tool_calls) = parse_text_tool_calls(&self.protocol) {
            choice.finish_reason = "tool_calls".to_string();
            return Ok(Some(tool_calls));
        }
        // A recognized protocol opener changes the semantic channel. Even a
        // malformed provider payload is control data, never assistant prose.
        // Do not replay it into the transcript; let the caller recover the
        // protocol-error turn instead.
        self.protocol.clear();
        choice.finish_reason = "protocol_error".to_string();
        Ok(None)
    }
}

fn parse_text_tool_calls(content: &str) -> Option<Vec<ToolCall>> {
    let normalized = TEXT_TOOL_PROTOCOL_DIALECTS
        .iter()
        .filter_map(|dialect| dialect.namespace)
        .fold(content.to_string(), |value, namespace| {
            value.replace(namespace, "")
        });
    let envelope = normalized.trim();
    parse_canonical_text_tool_calls(envelope)
}

fn parse_canonical_text_tool_calls(envelope: &str) -> Option<Vec<ToolCall>> {
    let inner = envelope
        .strip_prefix(CANONICAL_TOOL_CALLS_OPEN)?
        .strip_suffix("</tool_calls>")?;
    let mut rest = inner;
    let mut calls = Vec::new();

    while !rest.trim().is_empty() {
        rest = rest.trim_start();
        let (call_open, call_close) = if rest.starts_with("<invoke") {
            ("<invoke", "</invoke>")
        } else if rest.starts_with("<tool") && !rest.starts_with("<tool_calls") {
            ("<tool", "</tool>")
        } else {
            return None;
        };
        rest = &rest[rest.find(call_open)?..];
        let tag_end = rest.find('>')?;
        let call_tag = &rest[..=tag_end];
        let name = xml_attribute(call_tag, "name")?;
        if name.trim().is_empty() {
            return None;
        }
        let after_tag = &rest[tag_end + 1..];
        let call_end = after_tag.find(call_close)?;
        let parameters_text = &after_tag[..call_end];
        let mut parameters = serde_json::Map::new();
        let mut parameter_rest = parameters_text;

        while !parameter_rest.trim().is_empty() {
            parameter_rest = parameter_rest.trim_start();
            let parameter_start = parameter_rest.find("<parameter")?;
            if !parameter_rest[..parameter_start].trim().is_empty() {
                return None;
            }
            parameter_rest = &parameter_rest[parameter_start..];
            let parameter_tag_end = parameter_rest.find('>')?;
            let parameter_tag = &parameter_rest[..=parameter_tag_end];
            let parameter_name = xml_attribute(parameter_tag, "name")?;
            let value_start = &parameter_rest[parameter_tag_end + 1..];
            let parameter_end = value_start.find("</parameter>")?;
            let raw_value = xml_unescape(value_start[..parameter_end].trim());
            let force_string = xml_attribute(parameter_tag, "string")
                .map(|value| value.eq_ignore_ascii_case("true"))
                .unwrap_or(true);
            let value = if force_string {
                Value::String(raw_value)
            } else {
                serde_json::from_str(&raw_value).unwrap_or(Value::String(raw_value))
            };
            parameters.insert(parameter_name, value);
            parameter_rest = &value_start[parameter_end + "</parameter>".len()..];
        }

        calls.push(ToolCall {
            id: format!("call_text_{}", calls.len()),
            name,
            arguments: Value::Object(parameters).to_string(),
        });
        rest = &after_tag[call_end + call_close.len()..];
    }

    (!calls.is_empty()).then_some(calls)
}

fn xml_attribute(tag: &str, name: &str) -> Option<String> {
    for quote in ['"', '\''] {
        let prefix = format!("{name}={quote}");
        let Some(start) = tag.find(&prefix).map(|index| index + prefix.len()) else {
            continue;
        };
        let value = &tag[start..];
        if let Some(end) = value.find(quote) {
            return Some(xml_unescape(&value[..end]));
        }
    }
    None
}

fn xml_unescape(text: &str) -> String {
    text.replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

fn should_fallback_non_stream_error_message(message: &str) -> bool {
    is_retryable_stream_transport_message(message)
}

fn log_llm_debug_event(
    cfg: &LlmConfig,
    phase: &str,
    endpoint: &str,
    messages: &[Value],
    tools: &[Value],
    tool_choice: &str,
    detail: &str,
) {
    let provider = if detect_chat_provider(cfg.provider.as_deref(), &cfg.endpoint, &cfg.model)
        == ChatProvider::Anthropic
    {
        "anthropic"
    } else {
        "openai"
    };
    super::diagnostics::log_llm_debug_event(
        cfg,
        provider,
        phase,
        endpoint,
        messages,
        tools,
        tool_choice,
        detail,
    );
}

fn complete_openai_compatible_chat<F>(
    cfg: &LlmConfig,
    body: &Value,
    endpoint: &str,
    allow_transport_fallback: bool,
    mut on_event: F,
) -> Result<StreamedChoice, ProviderAttemptError>
where
    F: FnMut(LlmEvent) -> Result<(), String>,
{
    let started_at = Instant::now();
    let parsed_endpoint = reqwest::Url::parse(endpoint)
        .map_err(|err| format!("{} {err}", tr(Message::LlmRequestFailedPrefix)))?;
    let endpoint_host = parsed_endpoint
        .host_str()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let endpoint_port = parsed_endpoint.port_or_known_default().unwrap_or_else(|| {
        if parsed_endpoint.scheme() == "https" {
            443
        } else {
            80
        }
    });
    let fallback_addresses = if endpoint_host.is_empty() || !allow_transport_fallback {
        Vec::new()
    } else {
        resolve_dns_fallbacks_for_host(&endpoint_host, endpoint_port)
    };
    let has_fallback_addresses = !fallback_addresses.is_empty();

    log_llm_debug_event(
        cfg,
        "request_start",
        endpoint,
        &[],
        &[],
        "auto",
        "mode=non_stream",
    );

    let mut last_err: Option<String> = None;
    let mut attempts: Vec<Option<SocketAddr>> = Vec::with_capacity(fallback_addresses.len() + 1);
    attempts.push(None);
    for address in fallback_addresses.iter().copied() {
        attempts.push(Some(address));
    }

    for resolved_addr in attempts {
        let client = if cfg.native_model_route.is_some() {
            super::transport::build_native_stream_client()?
        } else { build_stream_client(parsed_endpoint.host_str(), resolved_addr)? };
        let mut request = client
            .post(endpoint)
            .header(CONTENT_TYPE, "application/json")
            .json(body);
        if !cfg.api_key.trim().is_empty() {
            request = request.header(AUTHORIZATION, format!("Bearer {}", cfg.api_key));
        }

        if let Some(route) = &cfg.native_model_route {
            crate::desktop_host_control::validate_native_model_channel(
                route, &cfg.endpoint, &cfg.model, &cfg.api_key).map_err(ProviderAttemptError::Admission)?;
        }
        super::diagnostics::record_prepared_request(&request, &[cfg.api_key.as_str()]);
        let response = match send_request_with_cancellation(request, &cfg.cancellation) {
            SendRequestResult::Response(response) => response,
            SendRequestResult::Error(err) => {
                let err_text = classify_network_error_hint(&err);
                last_err = Some(err_text.clone());
                log_llm_debug_event(
                    cfg,
                    "request_send_error",
                    endpoint,
                    &[],
                    &[],
                    "auto",
                    &format!("attempt={:?} error={err_text}", resolved_addr),
                );
                if resolved_addr.is_none() && has_fallback_addresses {
                    continue;
                }
                return Err(
                    format!("{}: {}", tr(Message::LlmRequestFailedPrefix), err_text).into(),
                );
            }
            SendRequestResult::WorkerDisconnected => return Err(ProviderAttemptError::WorkerDisconnected),
            SendRequestResult::Cancelled => return Err(llm_cancelled_message().into()),
        };

        let status = response.status();
        if !status.is_success() {
            let retry_after_ms = response_retry_after_ms(&response);
            let detail = response.text().unwrap_or_default();
            log_llm_debug_event(
                cfg,
                "request_http_error",
                endpoint,
                &[],
                &[],
                "auto",
                &format!("status={status} body={}", trim_debug_detail(&detail)),
            );
            return Err(ProviderAttemptError::Http {
                status: status.as_u16(),
                retry_after_ms,
                detail,
            });
        }

        if cfg.cancellation.is_requested() {
            return Err(llm_cancelled_message().into());
        }
        let parsed: Value = response
            .json()
            .map_err(|err| format!("{} {err}", tr(Message::LlmStreamReadFailedPrefix)))?;
        if cfg.cancellation.is_requested() {
            return Err(llm_cancelled_message().into());
        }
        debug_response_usage(&parsed);
        let Some(choice_value) = parsed
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
        else {
            log_llm_debug_event(
                cfg,
                "request_parse_error",
                endpoint,
                &[],
                &[],
                "auto",
                "missing choices field",
            );
            return Err(format!("{} missing choices", tr(Message::LlmRequestFailedPrefix)).into());
        };
        let message = choice_value.get("message").unwrap_or(&Value::Null);
        let mut choice = StreamedChoice::default();
        merge_usage_from_envelope(&mut choice.usage, &parsed);
        capture_openai_response_identity(&mut choice, &parsed);
        if let Some(text) = message.get("content").and_then(Value::as_str) {
            choice.content = text.to_owned();
        }
        if let Some(text) = message.get("reasoning_content").and_then(Value::as_str) {
            if !text.is_empty() {
                choice.reasoning.push_str(text);
            }
        }
        let native_tool_calls = message
            .get("tool_calls")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .enumerate()
            .filter_map(|(index, tool_call)| {
                let function = tool_call.get("function")?;
                let name = function.get("name").and_then(Value::as_str)?;
                if name.trim().is_empty() {
                    return None;
                }
                Some(ToolCall {
                    id: tool_call
                        .get("id")
                        .and_then(Value::as_str)
                        .filter(|id| !id.trim().is_empty())
                        .map(str::to_string)
                        .unwrap_or_else(|| format!("call_{index}")),
                    name: name.to_string(),
                    arguments: tool_arguments_text(function.get("arguments"))
                        .unwrap_or_else(|| "{}".to_string()),
                })
            })
            .collect::<Vec<_>>();
        choice.tool_calls = native_tool_calls;
        choice.finish_reason = choice_value
            .get("finish_reason")
            .and_then(Value::as_str)
            .unwrap_or("stop")
            .to_string();
        if !choice.tool_calls.is_empty() {
            choice.finish_reason = "tool_calls".to_string();
        }
        reject_wrapped_gateway_error(&choice)?;
        if !choice.reasoning.is_empty() {
            on_event(LlmEvent::ReasoningDelta(choice.reasoning.clone()))?;
        }
        let content = std::mem::take(&mut choice.content);
        if !content.is_empty() {
            append_choice_text(&mut choice, &content, &mut on_event)?;
        }
        log_llm_debug_event(
            cfg,
            "request_done",
            endpoint,
            &[],
            &[],
            "auto",
            &format!(
                "status=ok content_len={} tool_calls={}",
                choice.content.len(),
                choice.tool_calls.len()
            ),
        );
        return super::contracts::retain_request(Ok(choice), cfg, body.clone(), started_at);
    }

    Err(format!(
        "{}: {}",
        tr(Message::LlmRequestFailedPrefix),
        last_err.unwrap_or_else(|| "unknown completion error".to_string())
    )
    .into())
}

pub(super) fn stream_anthropic_chat<F>(
    cfg: &LlmConfig,
    messages: &[Value],
    tools: &[Value],
    tool_choice: &str,
    on_event: F,
) -> Result<StreamedChoice, ProviderAttemptError>
where
    F: FnMut(LlmEvent) -> Result<(), String>,
{
    let body = anthropic_request_body(cfg, messages, tools, tool_choice)?;
    debug_context_metrics("anthropic", messages, tools, tool_choice, &body);
    let started = Instant::now();
    let result = stream_anthropic_body(cfg, &body, true, on_event);
    super::contracts::retain_request(result, cfg, body, started)
}

pub(super) fn stream_anthropic_body<F>(
    cfg: &LlmConfig, body: &Value, allow_transport_fallback: bool, mut on_event: F,
) -> Result<StreamedChoice, ProviderAttemptError>
where F: FnMut(LlmEvent) -> Result<(), String> {
    let messages = body["messages"].as_array().map(Vec::as_slice).unwrap_or_default();
    let tools = body["tools"].as_array().map(Vec::as_slice).unwrap_or_default();
    let tool_choice = body["tool_choice"]["type"].as_str().unwrap_or("auto");
    let endpoint = normalize_anthropic_endpoint(&cfg.endpoint)?;
    log_llm_debug_event(
        cfg,
        "request_start",
        &endpoint,
        body["messages"].as_array().map(Vec::as_slice).unwrap_or_default(),
        body["tools"].as_array().map(Vec::as_slice).unwrap_or_default(),
        body["tool_choice"]["type"].as_str().unwrap_or("auto"),
        "mode=stream",
    );
    let parsed_endpoint = reqwest::Url::parse(&endpoint)
        .map_err(|err| format!("{} {err}", tr(Message::LlmRequestFailedPrefix)))?;
    let endpoint_host = parsed_endpoint
        .host_str()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let endpoint_port = parsed_endpoint.port_or_known_default().unwrap_or_else(|| {
        if parsed_endpoint.scheme() == "https" {
            443
        } else {
            80
        }
    });
    let fallback_addresses = if endpoint_host.is_empty() || !allow_transport_fallback {
        Vec::new()
    } else {
        resolve_dns_fallbacks_for_host(&endpoint_host, endpoint_port)
    };
    let has_fallback_addresses = !fallback_addresses.is_empty();

    let mut last_err: Option<String> = None;
    let mut attempts: Vec<Option<SocketAddr>> = Vec::with_capacity(fallback_addresses.len() + 1);
    attempts.push(None);
    for address in fallback_addresses.iter().copied() {
        attempts.push(Some(address));
    }

    for (attempt_index, resolved_addr) in attempts.into_iter().enumerate() {
        let client = if cfg.native_model_route.is_some() {
            super::transport::build_native_stream_client()?
        } else { build_stream_client(parsed_endpoint.host_str(), resolved_addr)? };
        let mut request = client
            .post(&endpoint)
            .header(CONTENT_TYPE, "application/json")
            .header(ACCEPT, "text/event-stream")
            .header("anthropic-version", "2023-06-01")
            .json(&body);
        if !cfg.api_key.trim().is_empty() {
            request = request.header("x-api-key", cfg.api_key.trim());
        }

        if let Some(route) = &cfg.native_model_route {
            crate::desktop_host_control::validate_native_model_channel(
                route, &cfg.endpoint, &cfg.model, &cfg.api_key).map_err(ProviderAttemptError::Admission)?;
        }
        super::diagnostics::record_prepared_request(&request, &[cfg.api_key.as_str()]);
        let response = match send_request_with_cancellation(request, &cfg.cancellation) {
            SendRequestResult::Response(response) => response,
            SendRequestResult::Error(err) => {
                let err_text = classify_network_error_hint(&err);
                last_err = Some(err_text.clone());
                let should_retry = resolved_addr.is_none() && has_fallback_addresses;
                log_llm_debug_event(
                    cfg,
                    "request_send_error",
                    &endpoint,
                    messages,
                    tools,
                    tool_choice,
                    &format!("attempt={:?} error={err_text}", resolved_addr),
                );
                if should_retry {
                    continue;
                }
                return Err(
                    format!("{}: {}", tr(Message::LlmRequestFailedPrefix), err_text).into(),
                );
            }
            SendRequestResult::WorkerDisconnected => return Err(ProviderAttemptError::WorkerDisconnected),
            SendRequestResult::Cancelled => return Err(llm_cancelled_message().into()),
        };

        let status = response.status();
        if !status.is_success() {
            let retry_after_ms = response_retry_after_ms(&response);
            let detail = response.text().unwrap_or_default();
            log_llm_debug_event(
                cfg,
                "request_http_error",
                &endpoint,
                messages,
                tools,
                tool_choice,
                &format!(
                    "attempt={attempt_index} status={status} body={}",
                    trim_debug_detail(&detail)
                ),
            );
            return Err(ProviderAttemptError::Http {
                status: status.as_u16(),
                retry_after_ms,
                detail,
            });
        }

        let stream_idle_timeout = stream_idle_timeout();
        let stream_first_event_timeout = stream_first_event_timeout(stream_idle_timeout);
        let stream_tail_idle_timeout = stream_tail_idle_timeout();
        let read_failed_prefix = tr(Message::LlmStreamReadFailedPrefix).to_string();
        let (line_tx, line_rx) = mpsc::sync_channel::<Result<String, String>>(32);
        let reader_cancellation = cfg.cancellation.clone();
        thread::spawn(move || {
            let reader = BufReader::new(response);
            for line in reader.lines() {
                if reader_cancellation.is_requested() {
                    break;
                }
                match line {
                    Ok(line) => {
                        if !line.starts_with("data: ") {
                            continue;
                        }
                        if line_tx.send(Ok(line)).is_err() {
                            break;
                        }
                    }
                    Err(err) => {
                        let _ = line_tx.send(Err(format!("{read_failed_prefix} {err}")));
                        break;
                    }
                }
            }
        });

        let mut choice = StreamedChoice::default();
        let mut slots: BTreeMap<usize, ToolSlot> = BTreeMap::new();
        let mut saw_stream_data = false;
        let mut saw_terminal_event = false;
        let mut observed_progress_size = 0usize;
        let mut last_meaningful_progress_at = std::time::Instant::now();

        loop {
            let progress_size = stream_progress_size(&choice, &slots);
            if progress_size != observed_progress_size {
                observed_progress_size = progress_size;
                last_meaningful_progress_at = std::time::Instant::now();
            }
            let has_progress = stream_has_progress(&choice, &slots);
            if has_progress && last_meaningful_progress_at.elapsed() >= stream_tail_idle_timeout {
                break;
            }
            let mut recv_timeout = if has_progress {
                stream_tail_idle_timeout
            } else {
                stream_first_event_timeout
            };
            if has_progress {
                recv_timeout = recv_timeout.min(
                    stream_tail_idle_timeout.saturating_sub(last_meaningful_progress_at.elapsed()),
                );
            }
            let line = match recv_stream_line(&line_rx, recv_timeout, &cfg.cancellation) {
                StreamLineRecv::Line(line) => line,
                StreamLineRecv::Error(err) => {
                    log_llm_debug_event(
                        cfg,
                        "stream_read_error",
                        &endpoint,
                        messages,
                        tools,
                        tool_choice,
                        &format!("attempt={attempt_index} error={err}"),
                    );
                    if should_treat_stream_read_as_complete(&err) && saw_stream_data {
                        log_llm_debug_event(
                            cfg,
                            "stream_read_recovery",
                            &endpoint,
                            messages,
                            tools,
                            tool_choice,
                            "mark_buffered_content_incomplete",
                        );
                        break;
                    }
                    return Err(err.into());
                }
                StreamLineRecv::Timeout => {
                    if stream_has_progress(&choice, &slots) {
                        break;
                    }
                    return Err(format!(
                        "{} stream idle timeout after {}s",
                        tr(Message::LlmStreamReadFailedPrefix),
                        recv_timeout.as_secs()
                    )
                    .into());
                }
                StreamLineRecv::Disconnected => {
                    if saw_stream_data || stream_has_progress(&choice, &slots) {
                        break;
                    }
                    return Err("LLM connection closed before the first stream event"
                        .to_string()
                        .into());
                }
                StreamLineRecv::Cancelled => return Err(llm_cancelled_message().into()),
            };
            let raw = line.trim_start_matches("data: ").trim();
            if raw.is_empty() {
                continue;
            }
            if raw == "[DONE]" {
                saw_terminal_event = true;
                break;
            }
            saw_stream_data = true;
            let parsed: Value = match serde_json::from_str(raw) {
                Ok(value) => value,
                Err(_) => continue,
            };
            merge_usage_from_envelope(&mut choice.usage, &parsed);
            parse_anthropic_stream_event(&parsed, &mut choice, &mut slots, &mut on_event)?;
            thread::yield_now();
            if cfg.cancellation.is_requested() {
                return Err(llm_cancelled_message().into());
            }
            if parsed
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(|event_type| event_type == "message_stop")
            {
                saw_terminal_event = true;
                break;
            }
        }

        choice.tool_calls = slots
            .into_iter()
            .enumerate()
            .filter_map(|(fallback_index, (_slot_index, slot))| {
                if slot.name.trim().is_empty() {
                    return None;
                }
                Some(ToolCall {
                    id: if slot.id.trim().is_empty() {
                        format!("call_{fallback_index}")
                    } else {
                        slot.id
                    },
                    name: slot.name,
                    arguments: if slot.arguments.trim().is_empty() {
                        "{}".to_string()
                    } else {
                        normalize_tool_arguments_text(&slot.arguments)
                    },
                })
            })
            .collect();
        finalize_stream_completion(&mut choice, saw_terminal_event)?;

        log_llm_debug_event(
            cfg,
            "request_done",
            &endpoint,
            messages,
            tools,
            tool_choice,
            &format!(
                "status=ok attempts={attempt_index} streamed={} tools={}",
                choice.content.len(),
                choice.tool_calls.len()
            ),
        );

        return Ok(choice);
    }

    Err(format!(
        "{}: {}",
        tr(Message::LlmRequestFailedPrefix),
        last_err.unwrap_or_else(|| "unknown stream error".to_string())
    )
    .into())
}
