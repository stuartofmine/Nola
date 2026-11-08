// SPDX-License-Identifier: Apache-2.0
//! Shared request, event and result values. No provider or orchestration dependencies.

use crate::execution_context::ExecutionContext;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Credential-bearing in-memory physical request facts; methods that prepare
/// forks and durable credential-free witnesses belong to request_snapshot.
#[derive(Clone)]
pub(crate) struct PreparedModelRequest {
    pub(super) config: LlmConfig,
    pub(super) body: Value,
    /// Accepted response, not part of the already-sent request prefix.
    pub(super) completed_response: Vec<Value>,
    pub(super) started_at: std::time::Instant,
}

impl std::fmt::Debug for PreparedModelRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Neither credentials nor conversation bodies may enter Debug logs.
        f.debug_struct("PreparedModelRequest").finish_non_exhaustive()
    }
}

/// Attach immutable request facts to an accepted result. No provider dispatch,
/// snapshot recovery, or maintenance orchestration is performed here.
pub(super) fn retain_request(
    result: Result<StreamedChoice, ProviderAttemptError>,
    config: &LlmConfig,
    body: Value,
    started_at: std::time::Instant,
) -> Result<StreamedChoice, ProviderAttemptError> {
    result.map(|mut choice| {
        if choice.request_snapshot.is_none() {
            choice.request_snapshot = Some(std::sync::Arc::new(PreparedModelRequest {
                config: config.clone(), body, completed_response: Vec::new(), started_at,
            }));
        }
        choice
    })
}

/// The same candidate policy is used by admission and physical request dispatch.
pub(crate) fn supports_provider_fallbacks(
    provider: Option<&str>,
    reasoning_selected: bool,
) -> bool {
    !reasoning_selected
        && !provider.is_some_and(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "openai-responses" | "responses" | "siwc_responses"
            )
        })
}

/// Resolve the same currently admitted targets for dispatch and durable request
/// recovery. Credentials always come from live configuration, never snapshots.
pub(super) fn admitted_provider_targets(config: &LlmConfig) -> Vec<LlmConfig> {
    let mut primary = config.clone();
    primary.fallbacks.clear();
    let mut targets = vec![primary];
    if supports_provider_fallbacks(config.provider.as_deref(), config.reasoning_selection.is_some()) {
        targets.extend(config.fallbacks.iter().map(|route| {
            let mut target = config.clone();
            target.siwc = None;
            target.native_model_route = None;
            target.provider = route.provider.clone();
            target.endpoint = route.endpoint.clone();
            if let Some(key) = &route.api_key { target.api_key = key.clone(); }
            if let Some(model) = &route.model { target.model = model.clone(); }
            target.fallbacks.clear();
            target
        }));
    }
    targets
}

/// Resolve wire identity independently of model names or provider flavors.
/// Explicit protocol adapters win; legacy undeclared endpoints only recognize
/// an exact authority, never a matching path/query/user-info substring.
pub(crate) fn resolved_model_protocol(
    provider: Option<&str>,
    endpoint: &str,
) -> nola_domain::generation::ModelProtocol {
    use nola_domain::generation::ModelProtocol;
    match provider
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "openai-responses" | "responses" => return ModelProtocol::Responses,
        "siwc_responses" => return ModelProtocol::SiwcResponses,
        "anthropic" | "claude" => return ModelProtocol::Anthropic,
        "openai" | "openai-compatible" | "chat_completions" | "deepseek" | "dashscope" | "qwen"
        | "llamacpp" | "llama.cpp" | "llama-cpp" | "vllm" => {
            return ModelProtocol::ChatCompletions;
        }
        _ => {}
    }
    if url::Url::parse(endpoint.trim())
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
        .as_deref()
        == Some("api.anthropic.com")
    {
        ModelProtocol::Anthropic
    } else {
        ModelProtocol::ChatCompletions
    }
}

/// One resolution shared by admission identity and physical JSON construction.
pub(crate) fn resolved_output_token_field(
    protocol: nola_domain::generation::ModelProtocol,
    endpoint: &str,
    preference: nola_application::ChatOutputTokenField,
) -> nola_host_protocol::ModelOutputTokenField {
    use nola_domain::generation::ModelProtocol;
    use nola_host_protocol::ModelOutputTokenField as Field;
    match protocol {
        ModelProtocol::Responses => Field::MaxOutputTokens,
        ModelProtocol::SiwcResponses => Field::Omitted,
        ModelProtocol::Anthropic => Field::MaxTokens,
        ModelProtocol::ChatCompletions => match preference {
            nola_application::ChatOutputTokenField::MaxTokens => Field::MaxTokens,
            nola_application::ChatOutputTokenField::MaxCompletionTokens => {
                Field::MaxCompletionTokens
            }
            nola_application::ChatOutputTokenField::Auto => {
                if url::Url::parse(endpoint.trim())
                    .ok()
                    .and_then(|url| url.host_str().map(str::to_owned))
                    .as_deref()
                    == Some("api.openai.com")
                {
                    Field::MaxCompletionTokens
                } else {
                    Field::MaxTokens
                }
            }
        },
    }
}

#[derive(Debug, Clone)]
pub struct LlmConfig {
    pub siwc: Option<crate::model_siwc::SiwcExecutionContext>,
    /// Trusted native adapter identity, distinct from its ephemeral HTTP channel.
    pub native_model_route: Option<nola_host_protocol::HostNativeModelRouteBinding>,
    /// Persisted provider identity selected by the active Nola configuration.
    /// Environment variables must not override this request-scoped fact.
    pub provider: Option<String>,
    pub fallbacks: Vec<LlmProviderTarget>,
    pub endpoint: String,
    pub api_key: String,
    pub model: String,
    pub temperature: f64,
    pub top_p: f64,
    pub max_output_tokens: u64,
    /// Maintenance marker freezing the model's configured output allowance.
    /// Compaction must not impose a separate, smaller output cap.
    pub compaction_output_budget: Option<CompactionOutputBudget>,
    pub chat_output_token_field: nola_application::ChatOutputTokenField,
    /// Retry budget captured when the request configuration is assembled.
    /// Keeping this request-scoped avoids reading mutable process environment
    /// state while a stream is already in flight.
    pub max_retries: usize,
    /// Frozen request preference. A provider may ignore it; admission still
    /// validates the complete native batch independently.
    pub parallel_tool_calls: Option<bool>,
    pub thinking_enabled: bool,
    pub reasoning_summary_supported: Option<bool>,
    pub reasoning_selection: Option<nola_core::agent::reasoning::ReasoningSelection>,
    pub sampling_override: bool,
    pub generation_options: nola_core::agent::reasoning::GenerationOptions,
    pub cancellation: ExecutionContext,
    pub openai_prompt_cache: bool,
    pub openai_prompt_cache_key: Option<String>,
    pub openai_prompt_cache_key_prefix: Option<String>,
    pub openai_prompt_cache_retention: Option<String>,
    /// Explicit endpoint support for Responses history cache breakpoints; off by default.
    pub openai_prompt_cache_history_breakpoint: bool,
    pub openai_prompt_cache_history_breakpoint_endpoints: Option<Vec<String>>,
}

#[derive(Debug, Clone)]
pub struct CompactionOutputBudget {
    tokens: u64,
}

impl LlmConfig {
    pub(crate) fn for_compaction(&self) -> Result<Self, String> {
        // Validate and freeze the original model preference without narrowing it.
        let admitted = nola_domain::generation::resolve_output_token_limit(Some(self.max_output_tokens))?;
        let tokens = admitted;
        let mut configured = self.clone();
        configured.max_output_tokens = tokens;
        configured.compaction_output_budget = Some(CompactionOutputBudget { tokens });
        Ok(configured)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlmProviderTarget {
    pub provider: Option<String>,
    pub endpoint: String,
    pub api_key: Option<String>,
    pub model: Option<String>,
}

/// Transport-owned model failure. Retry, fallback, persistence, and UI
/// projection consume this value directly instead of reclassifying a rendered
/// error message at every boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderFailure {
    pub code: String,
    pub message: String,
    pub retryable: bool,
    /// Whether retrying the same physical target can make progress.
    pub same_target_retryable: bool,
    pub fallback_eligible: bool,
    pub exhausts_target: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Bounded diagnostic text for logs. User projections must use `message`.
    pub diagnostic: String,
}

#[derive(Debug)]
pub(super) enum ProviderAttemptError {
    Admission(String),
    /// Local transport worker disappeared; never inferred from provider text.
    WorkerDisconnected,
    Http {
        status: u16,
        retry_after_ms: Option<u64>,
        detail: String,
    },
    Message(String),
}

impl From<&str> for ProviderAttemptError {
    fn from(value: &str) -> Self {
        value.to_string().into()
    }
}

impl From<String> for ProviderAttemptError {
    fn from(value: String) -> Self {
        Self::Message(value)
    }
}

#[derive(Debug, Clone)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LlmUsage {
    pub input_tokens: Option<usize>,
    pub cached_input_tokens: Option<usize>,
    pub cache_write_input_tokens: Option<usize>,
    pub output_tokens: Option<usize>,
    pub reasoning_tokens: Option<usize>,
    pub total_tokens: Option<usize>,
}

impl LlmUsage {
    pub(crate) fn cache_usage(&self, anthropic: bool, is_compaction: bool) -> nola_domain::PromptCacheUsage {
        let read = self.cached_input_tokens.map(|v| v as u64);
        let write = self.cache_write_input_tokens.map(|v| v as u64);
        // Anthropic reports uncached input separately; Chat/Responses include reads.
        let input = if anthropic {
            self.input_tokens.zip(read.zip(write)).and_then(|(input, (read, write))| (input as u64).checked_add(read)?.checked_add(write))
        } else { self.input_tokens.map(|input| input as u64) };
        nola_domain::PromptCacheUsage::measured(input, read, write, is_compaction)
    }
}

#[derive(Debug, Clone, Default)]
pub struct StreamedChoice {
    /// Exact provider usage when present; absence remains unknown, not zero.
    pub provider_raw_usage: Option<serde_json::Value>,
    /// In-memory, credential-scoped wire witness; never serialized into a transcript.
    pub(crate) request_snapshot: Option<std::sync::Arc<PreparedModelRequest>>,
    pub response_items: Vec<Value>,
    pub content: String,
    pub reasoning: String,
    pub tool_calls: Vec<ToolCall>,
    pub finish_reason: String,
    pub response_id: Option<String>,
    /// Concrete model identity reported by the provider response. Dynamic
    /// aliases may resolve to a different model for every physical request.
    pub response_model: Option<String>,
    /// Identity of the candidate that actually completed this request. This
    /// can differ from `LlmConfig` when an explicit fallback is selected.
    pub provider_namespace: Option<String>,
    pub provider_model: Option<String>,
    pub usage: LlmUsage,
}

impl StreamedChoice {
    /// Diagnostic completion of this sampling attempt, never task success.
    /// A partial response is intentionally an Ok value so Core can recover it.
    pub(crate) fn sampling_measurement_outcome(&self) -> nola_measurement::Outcome {
        use nola_measurement::Outcome;
        match self.finish_reason.as_str() {
            "stop" | "end_turn" | "stop_sequence" | "completed"
            | "tool_calls" | "tool_use" | "function_call" => Outcome::Succeeded,
            "protocol_error" => Outcome::Failed,
            "content_filter" | "refusal" => Outcome::Rejected,
            // Includes length/max_tokens, stream_incomplete, and missing or
            // unknown reasons. Do not manufacture completion from visible text.
            _ => Outcome::Incomplete,
        }
    }
}

#[derive(Debug, Clone)]
pub enum LlmEvent {
    SamplingStarted {
        provider: String,
        model: String,
    },
    TextDelta(String),
    ReasoningDelta(String),
    ToolCallDelta {
        call_id: String,
        name: String,
        arguments_delta: String,
    },
    StageTiming {
        stage: &'static str,
        elapsed_ms: u64,
    },
    Retrying {
        attempt: usize,
        delay_ms: u64,
        failure: ProviderFailure,
        /// True only when the current configured target is abandoned and the
        /// next fallback target will be sampled. Same-target transport retries
        /// must not poison target health for the rest of the Turn.
        target_exhausted: bool,
    },
}

#[derive(Debug, Default)]
pub(super) struct ToolSlot {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) arguments: String,
}

/// Provider encoding consumes an admitted value, not a context reservation or
/// an implicit default. Invalid callers fail before issuing a network request.
pub(super) fn effective_max_output_tokens(cfg: &LlmConfig) -> Result<u64, String> {
    if let Some(budget) = &cfg.compaction_output_budget {
        if cfg.max_output_tokens != budget.tokens || budget.tokens == 0 {
            return Err("compaction output budget changed after admission".into());
        }
        return Ok(budget.tokens);
    }
    nola_domain::generation::resolve_output_token_limit(Some(cfg.max_output_tokens))
}
