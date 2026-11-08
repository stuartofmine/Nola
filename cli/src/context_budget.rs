use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use reqwest::blocking::Client;
use reqwest::redirect::Policy;
use serde_json::{json, Value};
use url::Url;



use crate::runtime_config::RuntimeCompaction;

#[derive(Debug, Clone, Copy)]
struct ProviderProbeCacheEntry {
    value: Option<ProviderContextLimits>,
    cached_at: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ProviderContextLimits {
    pub(crate) max_input_tokens: Option<usize>,
    pub(crate) max_output_tokens: Option<usize>,
    pub(crate) max_total_tokens: Option<usize>,
}

impl ProviderContextLimits {
    fn total(tokens: usize) -> Self {
        Self {
            max_input_tokens: None,
            max_output_tokens: None,
            max_total_tokens: Some(tokens),
        }
    }

    fn effective_window(self) -> Option<usize> {
        self.max_total_tokens.or(self.max_input_tokens)
    }
}

static PROVIDER_PROBE_CACHE: OnceLock<Mutex<HashMap<String, ProviderProbeCacheEntry>>> =
    OnceLock::new();
const DEFAULT_CONTEXT_WINDOW_TOKENS: usize = 128_000;

#[derive(Debug, Clone)]
pub struct ContextBudget {
    pub context_window_tokens: Option<usize>,
    pub context_window_source: &'static str,
    pub token_threshold: usize,
    pub prompt_token_limit: usize,
    pub reserved_output_tokens: usize,
    /// Heuristic reservation for context planning only. It does not replace
    /// the admitted generation limit in the model profile or provider request.
    pub effective_output_budget: EffectiveOutputBudget,
    pub keep_recent_messages: usize,
    pub compacted_tool_chars: usize,
    pub compacted_assistant_chars: usize,
}

/// Context reservation estimate. Provider metadata can bound this estimate,
/// but it is not authority to silently change the user's generation limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EffectiveOutputBudget {
    pub requested_tokens: usize,
    pub provider_cap_tokens: Option<usize>,
    pub tokens: usize,
}

fn resolve_effective_output_budget(
    requested_tokens: usize,
    provider_cap_tokens: Option<usize>,
    context_window_tokens: Option<usize>,
) -> EffectiveOutputBudget {
    let provider_capped = provider_cap_tokens
        .filter(|cap| *cap > 0)
        .map(|cap| requested_tokens.min(cap))
        .unwrap_or(requested_tokens);
    EffectiveOutputBudget {
        requested_tokens,
        provider_cap_tokens,
        tokens: reserved_output_tokens_for_window(context_window_tokens, provider_capped),
    }
}

impl ContextBudget {
    pub fn from_runtime_with_provider_probe(
        runtime: &RuntimeCompaction,
        issue_mode: bool,
        endpoint: &str,
        api_key: &str,
        model: &str,
        provider: Option<&str>,
        allow_provider_probe: bool,
        max_output_tokens: u64,
    ) -> Self {
        // A configured context window overrides only the input/window
        // dimension. Keep probing exact provider metadata for its independent
        // output cap to inform reservation, not to rewrite generation limits.
        let provider_limits =
            provider_context_limits(endpoint, model, api_key, provider, allow_provider_probe);
        Self::from_frozen_provider_limits(runtime, issue_mode, provider_limits, max_output_tokens)
    }

    pub(crate) fn from_frozen_provider_limits(
        runtime: &RuntimeCompaction,
        issue_mode: bool,
        provider_limits: Option<ProviderContextLimits>,
        max_output_tokens: u64,
    ) -> Self {
        let configured_window = configured_context_window_tokens(runtime);
        let (context_window_tokens, context_window_source) =
            resolve_context_window_tokens(configured_window, provider_limits);
        let explicit_threshold = explicit_token_threshold(runtime);
        let requested_output_tokens = effective_configured_output_tokens(max_output_tokens);
        let effective_output_budget = resolve_effective_output_budget(
            requested_output_tokens,
            provider_limits.and_then(|limits| limits.max_output_tokens),
            context_window_tokens,
        );
        let reserved_output_tokens = effective_output_budget.tokens;
        let provider_prompt_limit = provider_limits
            .and_then(|limits| provider_prompt_token_limit(limits, reserved_output_tokens));
        let window_prompt_limit = context_window_tokens
            .map(|tokens| prompt_token_limit_for_window(tokens, reserved_output_tokens));
        let prompt_token_limit = match (provider_prompt_limit, window_prompt_limit) {
            (Some(provider), Some(window)) => Some(provider.min(window)),
            (provider, window) => provider.or(window),
        }
            .unwrap_or_else(|| explicit_threshold.unwrap_or(60_000))
            .clamp(512, 2_000_000);
        let token_threshold = explicit_threshold
            .or_else(|| context_window_tokens.map(default_compaction_threshold_for_window))
            .unwrap_or(60_000)
            .min(prompt_token_limit)
            .clamp(512, 2_000_000);
        let budget = Self {
            context_window_tokens,
            context_window_source,
            token_threshold,
            prompt_token_limit,
            reserved_output_tokens,
            effective_output_budget,
            keep_recent_messages: env_usize("NOLA_AGENT_CONTEXT_KEEP_RECENT_MESSAGES")
                .or(runtime.keep_recent_messages)
                .unwrap_or(if issue_mode { 8 } else { 8 })
                .clamp(4, 40),
            compacted_tool_chars: runtime
                .compacted_tool_chars
                .unwrap_or(if issue_mode { 900 } else { 1_400 })
                .clamp(400, 20_000),
            compacted_assistant_chars: runtime
                .compacted_assistant_chars
                .unwrap_or(if issue_mode { 600 } else { 900 })
                .clamp(300, 20_000),
        };
        debug_context_budget(context_window_tokens, &budget);
        budget
    }

    pub fn tighten_context_window(&mut self, tokens: usize, max_output_tokens: u64) {
        let tokens = tokens.clamp(2_000, 4_000_000);
        let tightened = self
            .context_window_tokens
            .map(|current| current.min(tokens))
            .unwrap_or(tokens);
        self.context_window_tokens = Some(tightened);
        self.context_window_source = "provider_error";
        self.effective_output_budget = resolve_effective_output_budget(
            effective_configured_output_tokens(max_output_tokens),
            self.effective_output_budget.provider_cap_tokens,
            Some(tightened),
        );
        self.reserved_output_tokens = self.effective_output_budget.tokens;
        self.prompt_token_limit =
            prompt_token_limit_for_window(tightened, self.reserved_output_tokens)
                .clamp(512, 2_000_000);
        self.token_threshold = self
            .token_threshold
            .min(default_compaction_threshold_for_window(tightened))
            .min(self.prompt_token_limit)
            .clamp(512, 2_000_000);
    }
}

fn resolve_context_window_tokens(
    configured_window: Option<usize>,
    provider_limits: Option<ProviderContextLimits>,
) -> (Option<usize>, &'static str) {
    if let Some(tokens) = configured_window {
        (Some(tokens), "configured")
    } else if let Some(tokens) = provider_limits.and_then(ProviderContextLimits::effective_window) {
        (Some(tokens), "provider_probe")
    } else {
        (Some(DEFAULT_CONTEXT_WINDOW_TOKENS), "default")
    }
}

fn provider_prompt_token_limit(
    limits: ProviderContextLimits,
    reserved_output_tokens: usize,
) -> Option<usize> {
    let total_limit = limits
        .max_total_tokens
        .map(|tokens| prompt_token_limit_for_window(tokens, reserved_output_tokens));
    let input_limit = limits.max_input_tokens.map(|tokens| {
        let safety_margin = (tokens / 20).clamp(512, 8_192);
        tokens.saturating_sub(safety_margin).max(512)
    });
    match (total_limit, input_limit) {
        (Some(total), Some(input)) => Some(total.min(input)),
        (Some(total), None) => Some(total),
        (None, Some(input)) => Some(input),
        (None, None) => None,
    }
}

fn default_compaction_threshold_for_window(tokens: usize) -> usize {
    nola_core::agent::policy::default_compaction_threshold_for_window(tokens)
}

fn prompt_token_limit_for_window(
    context_window_tokens: usize,
    reserved_output_tokens: usize,
) -> usize {
    nola_core::agent::policy::prompt_token_limit_for_window(
        context_window_tokens,
        reserved_output_tokens,
    )
}

fn reserved_output_tokens_for_window(
    context_window_tokens: Option<usize>,
    configured_output_tokens: usize,
) -> usize {
    nola_core::agent::policy::reserved_output_tokens_for_window(
        context_window_tokens,
        configured_output_tokens,
    )
}

fn configured_output_tokens() -> usize {
    nola_core::agent::DEFAULT_MODEL_OUTPUT_TOKENS
}

fn effective_configured_output_tokens(max_output_tokens: u64) -> usize {
    if max_output_tokens == 0 {
        configured_output_tokens()
    } else {
        nola_core::agent::context_output_reservation_tokens(
            usize::try_from(max_output_tokens).ok(),
            None,
        )
    }
}

fn explicit_token_threshold(runtime: &RuntimeCompaction) -> Option<usize> {
    env_usize("NOLA_AGENT_CONTEXT_TOKEN_THRESHOLD").or(runtime.token_threshold)
}

pub(crate) fn configured_context_window_tokens(runtime: &RuntimeCompaction) -> Option<usize> {
    let inherited_environment = || env_usize("NOLA_AGENT_CONTEXT_WINDOW_TOKENS");
    // A desktop Host request is a frozen, per-conversation contract selected
    // by the native client. Do not let an inherited shell environment silently
    // replace the window shown by the Mac UI after the request is hydrated.
    if std::env::var_os("NOLA_DESKTOP_HOST").is_some() {
        runtime.context_window_tokens.or_else(inherited_environment)
    } else {
        inherited_environment().or(runtime.context_window_tokens)
    }
    .map(|tokens| tokens.clamp(2_000, 4_000_000))
}

pub(crate) fn provider_context_limits(
    endpoint: &str,
    model: &str,
    api_key: &str,
    provider: Option<&str>,
    allow_provider_probe: bool,
) -> Option<ProviderContextLimits> {
    if !allow_provider_probe || !provider_context_probe_enabled() {
        return None;
    }
    let provider_lc = provider
        .map(str::to_string)
        .unwrap_or_default()
        .to_ascii_lowercase();
    let probe_protocol = provider_probe_protocol(&provider_lc, endpoint);

    let cache_key = provider_probe_cache_key(endpoint, model, &provider_lc, api_key);
    if let Some(cached) = cached_provider_probe(&cache_key) {
        return cached;
    }
    let value = match probe_protocol {
        ProviderProbeProtocol::OpenAICompatible => {
            probe_openai_models_context_limits(endpoint, model, api_key)
        }
        ProviderProbeProtocol::Anthropic => {
            probe_anthropic_context_window(endpoint, model, api_key)
        }
        ProviderProbeProtocol::Gemini => probe_gemini_context_window(endpoint, model, api_key),
        ProviderProbeProtocol::OpenRouter => {
            probe_openrouter_context_limits(endpoint, model, api_key)
        }
        protocol => probe_provider_context_window(endpoint, model, api_key, protocol)
            .map(ProviderContextLimits::total),
    };
    PROVIDER_PROBE_CACHE
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .insert(
            cache_key,
            ProviderProbeCacheEntry {
                value,
                cached_at: Instant::now(),
            },
        );
    value
}

fn probe_openai_models_context_limits(
    endpoint: &str,
    model: &str,
    api_key: &str,
) -> Option<ProviderContextLimits> {
    let (_, v1_base) = provider_probe_bases(endpoint)?;
    let client = provider_probe_client()?;
    get_json(&client, &format!("{v1_base}/models"), api_key)
        .and_then(|value| context_limits_from_models(&value, model))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProviderProbeProtocol {
    OpenAICompatible,
    Anthropic,
    Gemini,
    OpenRouter,
    Ollama,
    Vllm,
    LlamaCpp,
}

fn provider_probe_protocol(provider: &str, endpoint: &str) -> ProviderProbeProtocol {
    let endpoint_host = Url::parse(endpoint)
        .ok()
        .and_then(|url| url.host_str().map(str::to_ascii_lowercase))
        .unwrap_or_default();
    if provider == "openrouter" || endpoint_host == "openrouter.ai" {
        ProviderProbeProtocol::OpenRouter
    } else if matches!(provider, "anthropic" | "claude") || endpoint_host == "api.anthropic.com" {
        ProviderProbeProtocol::Anthropic
    } else if matches!(provider, "gemini" | "google" | "google-ai" | "google_ai")
        || endpoint_host == "generativelanguage.googleapis.com"
    {
        ProviderProbeProtocol::Gemini
    } else if provider == "ollama" {
        ProviderProbeProtocol::Ollama
    } else if provider == "vllm" {
        ProviderProbeProtocol::Vllm
    } else if matches!(provider, "llamacpp" | "llama.cpp" | "llama-cpp") {
        ProviderProbeProtocol::LlamaCpp
    } else {
        ProviderProbeProtocol::OpenAICompatible
    }
}

fn probe_anthropic_context_window(
    endpoint: &str,
    model: &str,
    api_key: &str,
) -> Option<ProviderContextLimits> {
    let url = provider_model_metadata_url(endpoint, model, "v1", &[])?;
    let client = provider_probe_client()?;
    let mut request = client.get(url).header("anthropic-version", "2023-06-01");
    if !api_key.trim().is_empty() {
        request = request.header("x-api-key", api_key.trim());
    }
    let response = request.send().ok()?;
    if !response.status().is_success() {
        return None;
    }
    response.json::<Value>().ok().and_then(|value|
        detail_model_token_limits(&value, "max_input_tokens", "max_tokens"))
}

fn probe_gemini_context_window(
    endpoint: &str,
    model: &str,
    api_key: &str,
) -> Option<ProviderContextLimits> {
    let url = provider_model_metadata_url(endpoint, model, "v1beta", &["openai"])?;
    let client = provider_probe_client()?;
    let mut request = client.get(url);
    if !api_key.trim().is_empty() {
        request = request.header("x-goog-api-key", api_key.trim());
    }
    let response = request.send().ok()?;
    if !response.status().is_success() {
        return None;
    }
    response.json::<Value>().ok().and_then(|value|
        detail_model_token_limits(&value, "inputTokenLimit", "outputTokenLimit"))
}

/// Detail endpoints identify the model in the URL. Keep output-only and low
/// limits as evidence; context-window heuristics cannot turn them into unknown.
fn detail_model_token_limits(value: &Value, input_field: &str, output_field: &str) -> Option<ProviderContextLimits> {
    let max_input_tokens = value_at_path(value, &[input_field]);
    let max_output_tokens = value.get(output_field).and_then(raw_output_token_cap);
    (max_input_tokens.is_some() || max_output_tokens.is_some()).then_some(ProviderContextLimits {
        max_input_tokens,
        max_output_tokens,
        max_total_tokens: None,
    })
}

fn raw_output_token_cap(value: &Value) -> Option<usize> {
    value.as_u64()
        .or_else(|| value.as_str().and_then(|value| value.trim().parse::<u64>().ok()))
        .and_then(|value| usize::try_from(value).ok())
}

#[cfg(test)]
mod detail_output_cap_tests {
    use super::*;

    #[test]
    fn provider_details_keep_output_only_zero_and_low_caps() {
        for (input, output) in [("max_input_tokens", "max_tokens"), ("inputTokenLimit", "outputTokenLimit")] {
            for cap in [0, 64, 256, 4_096, 64_000] {
                let mut value = serde_json::Map::new();
                value.insert(output.into(), json!(cap));
                let limits = detail_model_token_limits(&Value::Object(value.clone()), input, output).unwrap();
                assert_eq!(limits.max_output_tokens, Some(cap));
                assert_eq!(limits.max_input_tokens, None);
                value.insert(input.into(), json!(128_000));
                let limits = detail_model_token_limits(&Value::Object(value), input, output).unwrap();
                assert_eq!(limits.max_output_tokens, Some(cap));
                assert_eq!(limits.max_input_tokens, Some(128_000));
            }
        }
    }

    #[test]
    fn raw_caps_preserve_integer_facts_without_context_window_filters() {
        assert_eq!(raw_output_token_cap(&json!("0")), Some(0));
        assert_eq!(raw_output_token_cap(&json!("256")), Some(256));
        assert_eq!(raw_output_token_cap(&json!(5_000_000)), Some(5_000_000));
        for invalid in [json!(null), json!(-1), json!(1.5), json!("unknown")] {
            assert_eq!(raw_output_token_cap(&invalid), None);
        }
        assert!(detail_model_token_limits(&json!({}), "input", "output").is_none());
    }
}

fn provider_probe_client() -> Option<Client> {
    Client::builder()
        .no_proxy()
        .redirect(Policy::none())
        .timeout(Duration::from_millis(provider_context_probe_timeout_ms()))
        .build()
        .ok()
}

fn probe_openrouter_context_limits(
    endpoint: &str,
    model: &str,
    api_key: &str,
) -> Option<ProviderContextLimits> {
    let (_, v1_base) = provider_probe_bases(endpoint)?;
    let client = provider_probe_client()?;
    get_json(&client, &format!("{v1_base}/models"), api_key)
        .and_then(|value| context_limits_from_models(&value, model))
}

fn provider_model_metadata_url(
    endpoint: &str,
    model: &str,
    default_version: &str,
    compatibility_segments: &[&str],
) -> Option<Url> {
    let model = model.trim().strip_prefix("models/").unwrap_or(model.trim());
    if model.is_empty() {
        return None;
    }
    let mut url = Url::parse(endpoint.trim()).ok()?;
    if !url.username().is_empty() || url.password().is_some() {
        return None;
    }
    url.set_query(None);
    url.set_fragment(None);
    let mut segments = url
        .path_segments()?
        .filter(|segment| !segment.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();

    if let Some(index) = segments.iter().position(|segment| {
        compatibility_segments
            .iter()
            .any(|compatibility| segment == compatibility)
    }) {
        segments.truncate(index);
    }
    if let Some(index) = segments
        .iter()
        .position(|segment| segment == "messages" || segment == "models")
    {
        segments.truncate(index);
    }
    if !segments
        .last()
        .is_some_and(|segment| segment == "v1" || segment == "v1beta")
    {
        segments.push(default_version.to_string());
    }
    segments.push("models".to_string());
    segments.push(model.to_string());
    url.set_path("");
    url.path_segments_mut().ok()?.extend(segments);
    Some(url)
}

fn provider_probe_cache_key(endpoint: &str, model: &str, provider: &str, api_key: &str) -> String {
    use sha2::{Digest, Sha256};
    let identity = serde_json::to_vec(&(
        endpoint.trim(), model.trim(), provider.trim().to_ascii_lowercase(),
        format!("{:x}", Sha256::digest(api_key.as_bytes())),
    )).expect("string tuple serialization cannot fail");
    format!("{:x}", Sha256::digest(identity))
}

#[cfg(test)]
mod provider_probe_identity_tests {
    use super::provider_probe_cache_key as key;

    #[test]
    fn metadata_cache_preserves_model_path_query_case_and_field_boundaries() {
        let base = key("https://example.invalid/Path?key=a/", "Model", "openai", "secret");
        for candidate in [
            key("https://example.invalid/path?key=a/", "Model", "openai", "secret"),
            key("https://example.invalid/Path?key=a", "Model", "openai", "secret"),
            key("https://example.invalid/Path?key=a/", "model", "openai", "secret"),
            key("https://example.invalid/Path?key=a/", "Model", "openai", "other"),
        ] { assert_ne!(base, candidate); }
        assert_ne!(key("https://e.invalid/a|b", "c", "openai", "s"), key("https://e.invalid/a", "b|c", "openai", "s"));
        assert!(!base.contains("secret"));
    }
}

fn cached_provider_probe(key: &str) -> Option<Option<ProviderContextLimits>> {
    let cache = PROVIDER_PROBE_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut cache = cache.lock().unwrap_or_else(|err| err.into_inner());
    let entry = cache.get(key).copied()?;
    let ttl = if entry.value.is_some() {
        provider_probe_success_cache_ttl()
    } else {
        provider_probe_failure_cache_ttl()
    };
    if entry.cached_at.elapsed() <= ttl {
        Some(entry.value)
    } else {
        cache.remove(key);
        None
    }
}

fn probe_provider_context_window(
    endpoint: &str,
    model: &str,
    api_key: &str,
    protocol: ProviderProbeProtocol,
) -> Option<usize> {
    let (root, v1_base) = provider_probe_bases(endpoint)?;
    let total_budget = Duration::from_millis(provider_context_probe_timeout_ms());
    let client = provider_probe_client()?;
    let started = Instant::now();
    if protocol == ProviderProbeProtocol::Ollama {
        return post_json(
            &client,
            &format!("{root}/api/show"),
            api_key,
            &json!({"name": model}),
        )
        .and_then(|value| context_window_from_server_info(&value));
    }
    if protocol == ProviderProbeProtocol::Vllm {
        if let Some(tokens) = get_json(&client, &format!("{v1_base}/models"), api_key)
            .and_then(|value| context_window_from_models(&value, model))
        {
            return Some(tokens);
        }
    }
    if started.elapsed() >= total_budget {
        return None;
    }

    match protocol {
        ProviderProbeProtocol::LlamaCpp => {
            if let Some(tokens) = get_json(&client, &format!("{root}/props"), api_key)
                .and_then(|value| context_window_from_props(&value))
            {
                return Some(tokens);
            }
            if started.elapsed() < total_budget {
                if let Some(tokens) = get_json(&client, &format!("{root}/slots"), api_key)
                    .and_then(|value| context_window_from_slots(&value))
                {
                    return Some(tokens);
                }
            }
        }
        ProviderProbeProtocol::Vllm => {
            if started.elapsed() < total_budget {
                if let Some(tokens) = get_json(&client, &format!("{root}/get_model_info"), api_key)
                    .and_then(|value| context_window_from_server_info(&value))
                {
                    return Some(tokens);
                }
            }
        }
        ProviderProbeProtocol::OpenAICompatible
        | ProviderProbeProtocol::Anthropic
        | ProviderProbeProtocol::Gemini
        | ProviderProbeProtocol::OpenRouter
        | ProviderProbeProtocol::Ollama => return None,
    }
    if started.elapsed() >= total_budget {
        return None;
    }
    get_json(&client, &format!("{v1_base}/models"), api_key)
        .and_then(|value| context_window_from_models(&value, model))
}

fn provider_context_probe_enabled() -> bool {
    std::env::var("NOLA_AGENT_CONTEXT_PROVIDER_PROBE")
        .ok()
        .map(|value| {
            !matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "0" | "false" | "no" | "off" | "disable" | "disabled"
            )
        })
        .unwrap_or(true)
}

fn provider_context_probe_timeout_ms() -> u64 {
    std::env::var("NOLA_AGENT_CONTEXT_PROVIDER_PROBE_TIMEOUT_MS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| (100..=10_000).contains(value))
        .unwrap_or(1_500)
}

fn provider_probe_success_cache_ttl() -> Duration {
    Duration::from_secs(
        std::env::var("NOLA_AGENT_CONTEXT_PROVIDER_PROBE_CACHE_SEC")
            .ok()
            .and_then(|value| value.trim().parse::<u64>().ok())
            .unwrap_or(3_600)
            .clamp(60, 86_400),
    )
}

fn provider_probe_failure_cache_ttl() -> Duration {
    Duration::from_secs(
        std::env::var("NOLA_AGENT_CONTEXT_PROVIDER_PROBE_FAILURE_CACHE_SEC")
            .ok()
            .and_then(|value| value.trim().parse::<u64>().ok())
            .unwrap_or(300)
            .clamp(10, 3_600),
    )
}

fn provider_probe_bases(endpoint: &str) -> Option<(String, String)> {
    let mut url = Url::parse(endpoint.trim()).ok()?;
    if !url.username().is_empty() || url.password().is_some() {
        return None;
    }
    url.set_query(None);
    url.set_fragment(None);
    let mut segments = url
        .path_segments()?
        .filter(|segment| !segment.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    if segments.ends_with(&["chat".to_string(), "completions".to_string()]) {
        segments.truncate(segments.len().saturating_sub(2));
    }
    if segments.last().is_some_and(|segment| segment == "v1") {
        segments.pop();
    }
    let mut root_url = url.clone();
    root_url.set_path("");
    root_url.path_segments_mut().ok()?.extend(segments.clone());
    let root = root_url.as_str().trim_end_matches('/').to_string();
    segments.push("v1".to_string());
    let mut v1_url = url;
    v1_url.set_path("");
    v1_url.path_segments_mut().ok()?.extend(segments);
    let v1_base = v1_url.as_str().trim_end_matches('/').to_string();
    Some((root, v1_base))
}

fn get_json(client: &Client, url: &str, api_key: &str) -> Option<Value> {
    let mut request = client.get(url);
    if !api_key.trim().is_empty() {
        request = request.bearer_auth(api_key.trim());
    }
    let response = request.send().ok()?;
    if !response.status().is_success() {
        return None;
    }
    response.json::<Value>().ok()
}

fn post_json(client: &Client, url: &str, api_key: &str, body: &Value) -> Option<Value> {
    let mut request = client.post(url).json(body);
    if !api_key.trim().is_empty() {
        request = request.bearer_auth(api_key.trim());
    }
    let response = request.send().ok()?;
    if !response.status().is_success() {
        return None;
    }
    response.json::<Value>().ok()
}

fn context_window_from_props(value: &Value) -> Option<usize> {
    value_at_path(value, &["default_generation_settings", "n_ctx"])
        .or_else(|| value_at_path(value, &["n_ctx"]))
        .or_else(|| context_window_from_server_info(value))
}

fn context_window_from_slots(value: &Value) -> Option<usize> {
    value
        .as_array()?
        .iter()
        .filter_map(|slot| value_at_path(slot, &["n_ctx"]))
        .max()
}

fn context_window_from_server_info(value: &Value) -> Option<usize> {
    for path in [
        &["max_model_len"][..],
        &["max_model_length"][..],
        &["model_max_length"][..],
        &["context_length"][..],
        &["max_context_length"][..],
        &["max_context_window_tokens"][..],
        &["max_seq_len"][..],
        &["max_sequence_length"][..],
        &["max_total_tokens"][..],
        &["max_input_length"][..],
        &["config", "max_model_len"][..],
        &["config", "max_position_embeddings"][..],
        &["model_config", "max_model_len"][..],
        &["model_config", "max_position_embeddings"][..],
        &["model_info", "context_length"][..],
        &["model_info", "max_context_length"][..],
        &["model_info", "max_model_len"][..],
        &["parameters", "num_ctx"][..],
        &["options", "num_ctx"][..],
    ] {
        if let Some(tokens) = value_at_path(value, path) {
            return Some(tokens);
        }
    }
    recursive_context_window_from_value(value)
}

fn context_window_from_models(value: &Value, model: &str) -> Option<usize> {
    let model_lc = model.to_ascii_lowercase();
    let candidates = value
        .get("data")
        .and_then(Value::as_array)
        .or_else(|| value.get("models").and_then(Value::as_array))?;
    for candidate in candidates {
        let matches_model = model_lc.is_empty() || model_entry_matches(candidate, &model_lc);
        let tokens = model_context_tokens(candidate);
        if matches_model && tokens.is_some() {
            return tokens;
        }
    }
    None
}

fn model_entry_matches(entry: &Value, model_lc: &str) -> bool {
    for key in ["id", "model", "canonical_slug"] {
        if entry
            .get(key)
            .and_then(Value::as_str)
            .is_some_and(|value| value.to_ascii_lowercase() == model_lc)
        {
            return true;
        }
    }
    entry
        .get("aliases")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .any(|alias| alias.to_ascii_lowercase() == model_lc)
}

fn context_limits_from_models(value: &Value, model: &str) -> Option<ProviderContextLimits> {
    let model_lc = model.trim().to_ascii_lowercase();
    let candidates = value
        .get("data")
        .and_then(Value::as_array)
        .or_else(|| value.get("models").and_then(Value::as_array))?;
    // Output capabilities belong to an exact wire identity. Legacy aliases and
    // case-insensitive matches remain useful only as context-window hints.
    let exact = candidates.iter().find(|candidate| {
        !model.trim().is_empty()
            && ["id", "model"].iter().any(|key| candidate.get(*key).and_then(Value::as_str) == Some(model))
    });
    let candidate = exact.or_else(|| candidates.iter()
        .find(|candidate| model_lc.is_empty() || model_entry_matches(candidate, &model_lc)))?;
    let max_input_tokens = first_token_value(
        candidate,
        &[
            &["max_input_tokens"],
            &["inputTokenLimit"],
            &["max_input_length"],
            &["meta", "max_input_tokens"],
            &["metadata", "max_input_tokens"],
        ],
    );
    let max_total_tokens = first_token_value(
        candidate,
        &[
            &["context_length"],
            &["max_context_length"],
            &["max_context_window_tokens"],
            &["max_model_len"],
            &["max_model_length"],
            &["model_max_length"],
            &["max_total_tokens"],
            &["meta", "n_ctx"],
            &["meta", "context_length"],
            &["meta", "max_context_length"],
            &["metadata", "context_length"],
            &["metadata", "max_context_length"],
            &["metadata", "max_model_len"],
        ],
    )
    .or_else(|| {
        if max_input_tokens.is_none() {
            model_context_tokens(candidate)
        } else {
            None
        }
    });
    // Unlike the historical context parser, preserve output-only limits and
    // explicit zero. Domain admission must reject an impossible known cap,
    // rather than silently converting it to unknown/product defaults.
    let max_output_tokens = exact.and_then(|candidate| {
        [
            &["top_provider", "max_completion_tokens"][..],
            &["max_completion_tokens"][..],
            &["max_output_tokens"][..],
            &["outputTokenLimit"][..],
        ].into_iter().find_map(|path| {
            let value = path.iter().try_fold(candidate, |value, key| value.get(*key))?;
            raw_output_token_cap(value)
        })
    });
    (max_input_tokens.is_some() || max_total_tokens.is_some() || max_output_tokens.is_some()).then_some(ProviderContextLimits {
        max_input_tokens,
        max_output_tokens,
        max_total_tokens,
    })
}

fn first_token_value(value: &Value, paths: &[&[&str]]) -> Option<usize> {
    paths.iter().find_map(|path| value_at_path(value, path))
}

fn model_context_tokens(entry: &Value) -> Option<usize> {
    for path in [
        &["meta", "n_ctx"][..],
        &["meta", "context_length"][..],
        &["meta", "max_context_length"][..],
        &["meta", "max_context_window_tokens"][..],
        &["meta", "max_model_len"][..],
        &["metadata", "context_length"][..],
        &["metadata", "max_context_length"][..],
        &["metadata", "max_model_len"][..],
        &["context_length"][..],
        &["max_context_length"][..],
        &["max_context_window_tokens"][..],
        &["max_model_len"][..],
        &["max_model_length"][..],
        &["model_max_length"][..],
        &["max_total_tokens"][..],
        &["max_input_length"][..],
    ] {
        if let Some(tokens) = value_at_path(entry, path) {
            return Some(tokens);
        }
    }
    context_window_from_server_info(entry)
}

fn recursive_context_window_from_value(value: &Value) -> Option<usize> {
    let mut best = None;
    recursive_context_window_from_value_inner(value, 0, &mut best);
    best
}

fn recursive_context_window_from_value_inner(
    value: &Value,
    depth: usize,
    best: &mut Option<usize>,
) {
    if depth > 5 {
        return;
    }
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                if context_window_key(key) {
                    if let Some(tokens) = numeric_context_value(child) {
                        *best = Some(best.map_or(tokens, |current: usize| current.max(tokens)));
                    }
                }
                recursive_context_window_from_value_inner(child, depth + 1, best);
            }
        }
        Value::Array(items) => {
            for item in items {
                recursive_context_window_from_value_inner(item, depth + 1, best);
            }
        }
        _ => {}
    }
}

fn context_window_key(key: &str) -> bool {
    let normalized = key
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect::<String>()
        .to_ascii_lowercase();
    if normalized.ends_with("contextlength")
        || normalized.ends_with("contextwindowtokens")
        || normalized.ends_with("modellength")
        || normalized.ends_with("modellen")
    {
        return true;
    }
    matches!(
        normalized.as_str(),
        "nctx"
            | "ctxsize"
            | "numctx"
            | "contextlength"
            | "maxcontextlength"
            | "maxcontextwindowtokens"
            | "maxmodellength"
            | "maxmodellen"
            | "modelmaxlength"
            | "modelmaxlen"
            | "maxseqlen"
            | "maxsequencelength"
            | "maxpositionembeddings"
            | "maxtotaltokens"
            | "maxinputlength"
            | "maxinputtokens"
            | "inputtokenlimit"
    )
}

fn numeric_context_value(value: &Value) -> Option<usize> {
    match value {
        Value::Number(number) => number.as_u64().map(|value| value as usize),
        Value::String(text) => text.trim().parse::<usize>().ok(),
        _ => None,
    }
    .filter(|value| (512..=4_000_000).contains(value))
}

fn value_at_path(value: &Value, path: &[&str]) -> Option<usize> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    match current {
        Value::Number(number) => number.as_u64().map(|value| value as usize),
        Value::String(text) => text.trim().parse::<usize>().ok(),
        _ => None,
    }
    .filter(|value| (512..=4_000_000).contains(value))
}

fn debug_context_budget(context_window_tokens: Option<usize>, budget: &ContextBudget) {
    if !debug_context_enabled() {
        return;
    }
    eprintln!(
        "nola_context_budget context_window_tokens={} source={} threshold_tokens={} prompt_token_limit={} reserved_output_tokens={} keep_recent_messages={}",
        context_window_tokens
            .map(|value| value.to_string())
            .unwrap_or_else(|| "unknown".to_string()),
        budget.context_window_source,
        budget.token_threshold,
        budget.prompt_token_limit,
        budget.reserved_output_tokens,
        budget.keep_recent_messages,
    );
}

fn debug_context_enabled() -> bool {
    std::env::var("NOLA_LLM_DEBUG_CONTEXT")
        .ok()
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

pub fn estimate_tokens(messages: &[Value]) -> usize {
    nola_core::context_engine::estimate_json_values_tokens(messages)
}

pub fn estimate_json_chars(value: &Value) -> usize {
    serde_json::to_string(value)
        .map(|text| text.chars().count())
        .unwrap_or_else(|_| match value {
            Value::Null | Value::Bool(_) | Value::Number(_) => 0,
            Value::String(text) => text.chars().count(),
            Value::Array(items) => items.iter().map(estimate_json_chars).sum(),
            Value::Object(map) => map.values().map(estimate_json_chars).sum(),
        })
}

fn env_usize(name: &str) -> Option<usize> {
    std::env::var(name)
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
}

#[cfg(test)]
mod metadata_output_contract_tests {
    use super::*;

    #[test]
    fn exact_wire_identity_wins_over_earlier_casefold_and_alias_matches() {
        let metadata = json!({"data":[
            {"id":"MODEL","context_length":64000,"max_completion_tokens":1024},
            {"id":"other","aliases":["model"],"context_length":32000,"max_completion_tokens":2048},
            {"id":"model","context_length":128000,"max_output_tokens":8192}
        ]});
        let selected = context_limits_from_models(&metadata, "model").unwrap();
        assert_eq!(selected.max_total_tokens, Some(128000));
        assert_eq!(selected.max_output_tokens, Some(8192));
        let upper = context_limits_from_models(&metadata, "MODEL").unwrap();
        assert_eq!(upper.max_output_tokens, Some(1024));
        let legacy = context_limits_from_models(&metadata, "Model").unwrap();
        assert_eq!(legacy.max_total_tokens, Some(64000));
        assert_eq!(legacy.max_output_tokens, None);
    }

    #[test]
    fn aliases_and_canonical_slugs_supply_only_legacy_context_hints() {
        let metadata = json!({"models":[{"id":"actual-id","aliases":["friendly"],
            "canonical_slug":"slug","context_length":32000,"max_output_tokens":4096}]});
        for requested in ["friendly", "slug", "ACTUAL-ID", " actual-id ", "", "  "] {
            let limits = context_limits_from_models(&metadata, requested).unwrap();
            assert_eq!(limits.max_total_tokens, Some(32000), "{requested:?}");
            assert_eq!(limits.max_output_tokens, None, "{requested:?}");
        }
        assert_eq!(context_limits_from_models(&metadata, "actual-id").unwrap().max_output_tokens, Some(4096));
        assert_eq!(context_limits_from_models(&metadata, "unknown"), None);
    }

    #[test]
    fn output_only_metadata_survives_for_both_exact_identity_fields() {
        for identity in ["id", "model"] {
            for field in ["max_completion_tokens", "max_output_tokens", "outputTokenLimit"] {
                let mut entry = json!({identity:"model"});
                entry[field] = json!(256);
                assert_eq!(context_limits_from_models(&json!({"data":[entry]}), "model"),
                    Some(ProviderContextLimits {max_input_tokens:None,max_total_tokens:None,max_output_tokens:Some(256)}));
            }
        }
        let metadata = json!({"data":[{"id":"model","top_provider":{"max_completion_tokens":8192}}]});
        assert_eq!(context_limits_from_models(&metadata, "model").unwrap().max_output_tokens, Some(8192));
        assert_eq!(context_limits_from_models(&metadata, "MODEL"), None);
        assert_eq!(context_limits_from_models(&metadata, ""), None);
    }

    #[test]
    fn known_zero_output_cap_is_not_replaced_by_unknown_or_another_field() {
        for zero in [json!(0), json!("0")] {
            let metadata = json!({"data":[{"id":"model","top_provider":{"max_completion_tokens":zero},
                "max_output_tokens":8192}]});
            let limits = context_limits_from_models(&metadata, "model").unwrap();
            assert_eq!(limits.max_output_tokens, Some(0));
            assert_eq!(limits.max_total_tokens, None);
        }
        let metadata = json!({"data":[{"id":"model","max_output_tokens":0}]});
        assert_eq!(context_limits_from_models(&metadata, "model").unwrap().max_output_tokens, Some(0));
    }

    #[test]
    fn exact_output_only_entry_does_not_borrow_an_alias_context_contract() {
        let metadata = json!({"data":[
            {"id":"different","aliases":["model"],"max_input_tokens":128000,"max_output_tokens":64000},
            {"model":"model","max_output_tokens":4096}
        ]});
        assert_eq!(context_limits_from_models(&metadata, "model"), Some(ProviderContextLimits {
            max_input_tokens:None,max_total_tokens:None,max_output_tokens:Some(4096),
        }));
        for invalid in [json!(-1), json!(1.5), json!(false), Value::Null, json!("not-a-number")] {
            assert_eq!(context_limits_from_models(&json!({"data":[{"id":"model","max_output_tokens":invalid}]}), "model"), None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    fn serve_probe_response(body: &'static str) -> (String, thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind provider probe server");
        let address = listener.local_addr().expect("provider probe address");
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept provider probe");
            let mut request = Vec::new();
            let mut buffer = [0_u8; 2_048];
            loop {
                let count = stream.read(&mut buffer).expect("read provider probe");
                if count == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..count]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .expect("write provider probe response");
            String::from_utf8(request).expect("UTF-8 provider request")
        });
        (format!("http://{address}"), server)
    }

    #[test]
    fn unavailable_provider_metadata_uses_provider_agnostic_default() {
        assert_eq!(
            resolve_context_window_tokens(None, None),
            (Some(128_000), "default")
        );
        assert_eq!(
            resolve_context_window_tokens(
                None,
                Some(ProviderContextLimits {
                    max_input_tokens: None,
                    max_output_tokens: Some(16_384),
                    max_total_tokens: None,
                })
            ),
            (Some(128_000), "default")
        );
        assert_eq!(
            resolve_context_window_tokens(Some(65_536), None),
            (Some(65_536), "configured")
        );
    }

    #[test]
    fn provider_probe_protocol_uses_provider_and_official_endpoint() {
        assert_eq!(
            provider_probe_protocol("", "https://example.com/v1/chat/completions"),
            ProviderProbeProtocol::OpenAICompatible
        );
        assert_eq!(
            provider_probe_protocol("openai", "https://api.openai.com/v1/chat/completions"),
            ProviderProbeProtocol::OpenAICompatible
        );
        assert_eq!(
            provider_probe_protocol("ollama", "http://127.0.0.1:11434/v1"),
            ProviderProbeProtocol::Ollama
        );
        assert_eq!(
            provider_probe_protocol("vllm", "http://127.0.0.1:8000/v1"),
            ProviderProbeProtocol::Vllm
        );
        assert_eq!(
            provider_probe_protocol("llama.cpp", "http://127.0.0.1:8080/v1"),
            ProviderProbeProtocol::LlamaCpp
        );
        assert_eq!(
            provider_probe_protocol("", "https://api.anthropic.com/v1/messages"),
            ProviderProbeProtocol::Anthropic
        );
        assert_eq!(
            provider_probe_protocol(
                "",
                "https://generativelanguage.googleapis.com/v1beta/openai/chat/completions"
            ),
            ProviderProbeProtocol::Gemini
        );
        assert_eq!(
            provider_probe_protocol("anthropic", "https://proxy.example/v1/messages"),
            ProviderProbeProtocol::Anthropic
        );
        assert_eq!(
            provider_probe_protocol("google-ai", "https://proxy.example/v1beta/openai"),
            ProviderProbeProtocol::Gemini
        );
        assert_eq!(
            provider_probe_protocol("", "https://openrouter.ai/api/v1/chat/completions"),
            ProviderProbeProtocol::OpenRouter
        );
    }

    #[test]
    fn builds_provider_model_metadata_urls_without_compatibility_paths() {
        let anthropic = provider_model_metadata_url(
            "https://api.anthropic.com/v1/messages",
            "claude-sonnet-4-5",
            "v1",
            &[],
        )
        .expect("anthropic model URL");
        assert_eq!(
            anthropic.as_str(),
            "https://api.anthropic.com/v1/models/claude-sonnet-4-5"
        );

        let gemini = provider_model_metadata_url(
            "https://generativelanguage.googleapis.com/v1beta/openai/chat/completions",
            "models/gemini-2.5-pro",
            "v1beta",
            &["openai"],
        )
        .expect("Gemini model URL");
        assert_eq!(
            gemini.as_str(),
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-pro"
        );

        let proxied = provider_model_metadata_url(
            "https://proxy.example/anthropic/v1/messages",
            "claude custom/model",
            "v1",
            &[],
        )
        .expect("proxied model URL");
        assert_eq!(
            proxied.as_str(),
            "https://proxy.example/anthropic/v1/models/claude%20custom%2Fmodel"
        );
    }

    #[test]
    fn provider_probe_bases_preserve_proxy_prefix_and_drop_query() {
        assert_eq!(
            provider_probe_bases(
                "https://proxy.example/tenant/api/v1/chat/completions?deployment=primary#ignored"
            ),
            Some((
                "https://proxy.example/tenant/api".to_string(),
                "https://proxy.example/tenant/api/v1".to_string()
            ))
        );
        assert_eq!(
            provider_probe_bases("https://user:secret@proxy.example/v1"),
            None
        );
    }

    #[test]
    fn default_threshold_caps_large_windows_to_the_active_context_ceiling() {
        assert_eq!(default_compaction_threshold_for_window(262_144), 96_000);
        assert_eq!(default_compaction_threshold_for_window(200_000), 96_000);
        assert_eq!(default_compaction_threshold_for_window(100_000), 60_000);
    }

    #[test]
    fn prompt_limit_reserves_completion_and_safety_margin() {
        let reserved = reserved_output_tokens_for_window(Some(32_768), 4_096);
        assert_eq!(reserved, 4_096);
        assert_eq!(prompt_token_limit_for_window(32_768, reserved), 27_034);
    }

    #[test]
    fn compaction_budget_reserves_full_output_and_honors_the_configured_window() {
        let budget = ContextBudget::from_frozen_provider_limits(
            &RuntimeCompaction { context_window_tokens: Some(32_000), ..Default::default() },
            false,
            Some(ProviderContextLimits { max_input_tokens:None, max_output_tokens:Some(16_384), max_total_tokens:Some(128_000) }),
            16_384,
        );
        assert_eq!(budget.reserved_output_tokens,16_384);
        assert_eq!(budget.prompt_token_limit,14_016);
        assert!(budget.token_threshold <= budget.prompt_token_limit);
        assert_eq!(reserved_output_tokens_for_window(Some(128_000),64_000),64_000);
        assert_eq!(reserved_output_tokens_for_window(None,1_000_000),1_000_000);
    }

    #[test]
    fn parses_llama_cpp_context_window_from_props() {
        let value = serde_json::json!({
            "default_generation_settings": {"n_ctx": 102400},
            "model_alias": "qwen3.6-35b-a3b",
        });

        assert_eq!(context_window_from_props(&value), Some(102_400));
    }

    #[test]
    fn parses_context_window_from_openai_compatible_models() {
        let value = serde_json::json!({
            "data": [{
                "id": "qwen3.6-35b-a3b",
                "aliases": ["qwen3.6-35b-a3b"],
                "meta": {"n_ctx": 102400, "n_ctx_train": 262144}
            }]
        });

        assert_eq!(
            context_window_from_models(&value, "qwen3.6-35b-a3b"),
            Some(102_400)
        );
    }

    #[test]
    fn models_probe_never_borrows_another_models_context_window() {
        let value = serde_json::json!({
            "data": [
                {"id": "small-model", "context_length": 8192},
                {"id": "large-model", "context_length": 262144}
            ]
        });

        assert_eq!(context_window_from_models(&value, "missing-model"), None);
        assert_eq!(
            context_window_from_models(&value, "large-model"),
            Some(262_144)
        );
    }

    #[test]
    fn compatible_models_keep_input_and_total_limits_distinct() {
        let input_only = serde_json::json!({
            "data": [{
                "id": "input-limited-model",
                "max_input_tokens": 200000,
                "max_completion_tokens": 64000
            }]
        });
        let total = serde_json::json!({
            "data": [{
                "id": "total-limited-model",
                "context_length": 131072,
                "max_completion_tokens": 16384
            }]
        });

        assert_eq!(
            context_limits_from_models(&input_only, "input-limited-model"),
            Some(ProviderContextLimits {
                max_input_tokens: Some(200_000),
                max_output_tokens: Some(64_000),
                max_total_tokens: None,
            })
        );
        assert_eq!(
            context_limits_from_models(&total, "total-limited-model"),
            Some(ProviderContextLimits {
                max_input_tokens: None,
                max_output_tokens: Some(16_384),
                max_total_tokens: Some(131_072),
            })
        );
    }

    #[test]
    fn parses_official_anthropic_and_gemini_model_limits() {
        let anthropic = serde_json::json!({
            "id": "claude-sonnet-4-5",
            "max_input_tokens": 200000,
            "max_tokens": 64000
        });
        let gemini = serde_json::json!({
            "name": "models/gemini-2.5-pro",
            "inputTokenLimit": 1048576,
            "outputTokenLimit": 65536
        });

        assert_eq!(model_context_tokens(&anthropic), Some(200_000));
        assert_eq!(model_context_tokens(&gemini), Some(1_048_576));
    }

    #[test]
    fn provider_metadata_probes_use_protocol_specific_paths_and_authentication() {
        let (anthropic_origin, anthropic_server) = serve_probe_response(
            r#"{"id":"claude-sonnet-4-5","max_input_tokens":200000,"max_tokens":64000}"#,
        );
        let anthropic = probe_anthropic_context_window(
            &format!("{anthropic_origin}/v1/messages"),
            "claude-sonnet-4-5",
            "anthropic-secret",
        );
        assert_eq!(
            anthropic,
            Some(ProviderContextLimits {
                max_input_tokens: Some(200_000),
                max_output_tokens: Some(64_000),
                max_total_tokens: None,
            })
        );
        let anthropic_request = anthropic_server.join().expect("Anthropic request");
        let anthropic_request_lc = anthropic_request.to_ascii_lowercase();
        assert!(anthropic_request.starts_with("GET /v1/models/claude-sonnet-4-5 HTTP/1.1"));
        assert!(anthropic_request_lc.contains("x-api-key: anthropic-secret"));
        assert!(anthropic_request_lc.contains("anthropic-version: 2023-06-01"));
        assert!(!anthropic_request_lc.contains("authorization:"));

        let (gemini_origin, gemini_server) = serve_probe_response(
            r#"{"name":"models/gemini-2.5-pro","inputTokenLimit":1048576,"outputTokenLimit":65536}"#,
        );
        let gemini = probe_gemini_context_window(
            &format!("{gemini_origin}/v1beta/openai/chat/completions"),
            "models/gemini-2.5-pro",
            "gemini-secret",
        );
        assert_eq!(
            gemini,
            Some(ProviderContextLimits {
                max_input_tokens: Some(1_048_576),
                max_output_tokens: Some(65_536),
                max_total_tokens: None,
            })
        );
        let gemini_request = gemini_server.join().expect("Gemini request");
        let gemini_request_lc = gemini_request.to_ascii_lowercase();
        assert!(gemini_request.starts_with("GET /v1beta/models/gemini-2.5-pro HTTP/1.1"));
        assert!(gemini_request_lc.contains("x-goog-api-key: gemini-secret"));
        assert!(!gemini_request_lc.contains("authorization:"));
        assert!(!gemini_request
            .lines()
            .next()
            .unwrap_or_default()
            .contains("secret"));
    }

    #[test]
    fn provider_input_limit_does_not_reserve_output_twice() {
        let limits = ProviderContextLimits {
            max_input_tokens: Some(200_000),
            max_output_tokens: Some(64_000),
            max_total_tokens: None,
        };
        let prompt_limit = provider_prompt_token_limit(limits, 64_000);

        assert_eq!(prompt_limit, Some(191_808));
    }

    #[test]
    fn effective_output_budget_caps_request_and_reservation_once() {
        let budget = resolve_effective_output_budget(16_384, Some(2_048), Some(128_000));

        assert_eq!(budget.requested_tokens, 16_384);
        assert_eq!(budget.provider_cap_tokens, Some(2_048));
        assert_eq!(budget.tokens, 2_048);
    }

    #[test]
    fn dynamic_alias_without_output_metadata_keeps_product_default() {
        let alias = "路由器/自由模型-δ";
        let metadata = serde_json::json!({
            "data": [{
                "id": alias,
                "context_length": 200000,
                "top_provider": {"max_completion_tokens": null}
            }]
        });
        let limits = context_limits_from_models(&metadata, alias).expect("exact alias metadata");
        let budget = resolve_effective_output_budget(
            configured_output_tokens(),
            limits.max_output_tokens,
            limits.effective_window(),
        );

        assert_eq!(budget.provider_cap_tokens, None);
        assert_eq!(budget.tokens, configured_output_tokens());
    }

    #[test]
    fn parses_openrouter_total_and_output_limits_for_exact_model() {
        let value = serde_json::json!({
            "data": [{
                "id": "vendor/model-a",
                "name": "Model B display name",
                "context_length": 131072,
                "top_provider": {"max_completion_tokens": 16384}
            }]
        });

        assert_eq!(
            context_limits_from_models(&value, "vendor/model-a"),
            Some(ProviderContextLimits {
                max_input_tokens: None,
                max_output_tokens: Some(16_384),
                max_total_tokens: Some(131_072),
            })
        );
        assert_eq!(
            context_limits_from_models(&value, "Model B display name"),
            None
        );
    }

    #[test]
    fn parses_context_window_from_server_info_variants() {
        let vllm = serde_json::json!({"model_config": {"max_model_len": 65536}});
        let tgi = serde_json::json!({"max_input_length": 32768, "max_total_tokens": 49152});
        let ollama = serde_json::json!({"model_info": {"qwen3.context_length": 131072}});

        assert_eq!(context_window_from_server_info(&vllm), Some(65_536));
        assert_eq!(context_window_from_server_info(&tgi), Some(49_152));
        assert_eq!(context_window_from_server_info(&ollama), Some(131_072));
    }

    #[test]
    fn parses_context_window_from_slots() {
        let value = serde_json::json!([
            {"id": 0, "n_ctx": 8192},
            {"id": 1, "n_ctx": 102400}
        ]);

        assert_eq!(context_window_from_slots(&value), Some(102_400));
    }

    #[test]
    fn token_estimate_uses_ascii_and_cjk_aware_ratios() {
        let ascii = serde_json::json!({"content": "a".repeat(400)});
        let cjk = serde_json::json!({"content": "你".repeat(100)});

        let ascii_tokens = estimate_tokens(&[ascii]);
        let cjk_tokens = estimate_tokens(&[cjk]);

        assert!((100..=110).contains(&ascii_tokens), "{ascii_tokens}");
        assert!((100..=110).contains(&cjk_tokens), "{cjk_tokens}");
    }

    #[test]
    fn provider_error_tightening_is_monotonic_and_preserves_output_room() {
        let mut budget = ContextBudget {
            context_window_tokens: Some(262_144),
            context_window_source: "configured",
            token_threshold: 157_286,
            prompt_token_limit: 253_952,
            reserved_output_tokens: 8_192,
            effective_output_budget: EffectiveOutputBudget {
                requested_tokens: 8_192,
                provider_cap_tokens: None,
                tokens: 8_192,
            },
            keep_recent_messages: 12,
            compacted_tool_chars: 1_400,
            compacted_assistant_chars: 900,
        };

        budget.tighten_context_window(32_768, 8_192);
        let tightened_prompt_limit = budget.prompt_token_limit;

        assert_eq!(budget.context_window_tokens, Some(32_768));
        assert_eq!(budget.context_window_source, "provider_error");
        assert!(budget.token_threshold <= budget.prompt_token_limit);
        assert!(budget.prompt_token_limit < 32_768);
        assert!(budget.reserved_output_tokens > 0);

        budget.tighten_context_window(131_072, 8_192);
        assert_eq!(budget.context_window_tokens, Some(32_768));
        assert_eq!(budget.prompt_token_limit, tightened_prompt_limit);
    }

    #[test]
    fn configured_model_output_tokens_drive_prompt_reservation() {
        assert_eq!(effective_configured_output_tokens(32_768), 32_768);
        assert_eq!(effective_configured_output_tokens(250_000), 250_000);
    }
}
