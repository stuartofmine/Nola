// SPDX-License-Identifier: Apache-2.0
//! Blocking HTTP client with retries, multi-endpoint fallback routing,
//! interruptible cancellation, 1-shot transactional `REPETITIVE_REASONING`
//! resample, streaming SSE, and optional audit request logging.

use std::fmt;
use std::io::BufReader;
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use nola_types::{
    GenerationParams, ModelProtocol, ModelRouteConfig, ModelStreamDelta, NormalizedModelResponse,
    StructuredModelRequest,
};
use reqwest::blocking::Client;
use serde_json::Value;
use url::Url;

use crate::adapter::{adapter_for, LlmError};
use crate::privacy::PrivacyRedactor;

static AUDIT_SEQ: AtomicU64 = AtomicU64::new(1);

const CANCEL_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// Returns true when the target URL's host is a loopback address (`localhost`, `127.0.0.1`, `::1`, etc.).
pub fn is_loopback_url(raw_url: &str) -> bool {
    let Ok(url) = Url::parse(raw_url) else {
        return false;
    };
    let Some(host) = url.host_str() else {
        return false;
    };
    let lower = host.trim_end_matches('.').to_ascii_lowercase();
    if lower == "localhost" || lower.ends_with(".localhost") {
        return true;
    }
    let clean = lower.trim_start_matches('[').trim_end_matches(']');
    clean
        .parse::<IpAddr>()
        .map(|ip| ip.is_loopback())
        .unwrap_or(false)
}

/// Per-call execution parameters for `LlmClient::complete_with_call_params`.
#[derive(Clone, Default)]
pub struct CallParams {
    pub generation: GenerationParams,
    /// Ordered fallback routes tried when the primary route fails with a
    /// retryable/exhausted status (408/429/5xx/529) or transport/timeout error.
    /// Never used when primary protocol is `ModelProtocol::SiwcResponses`.
    pub fallbacks: Vec<ModelRouteConfig>,
    /// Cooperative cancellation flag checked before requests, during backoff
    /// sleeps (in 50ms slices), and between every SSE event/line read.
    pub cancel: Option<Arc<AtomicBool>>,
    /// Optional hook invoked right before a transactional 1-shot resample after
    /// `LlmError::RepetitiveReasoning` so callers can reset buffered reasoning UI.
    pub on_stream_reset: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl fmt::Debug for CallParams {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CallParams")
            .field("generation", &self.generation)
            .field("fallbacks", &self.fallbacks)
            .field(
                "cancel",
                &self.cancel.as_ref().map(|c| c.load(Ordering::Relaxed)),
            )
            .field("has_on_stream_reset", &self.on_stream_reset.is_some())
            .finish()
    }
}

impl CallParams {
    pub fn with_generation(mut self, generation: GenerationParams) -> Self {
        self.generation = generation;
        self
    }

    pub fn with_fallbacks(mut self, fallbacks: Vec<ModelRouteConfig>) -> Self {
        self.fallbacks = fallbacks;
        self
    }

    pub fn with_cancel(mut self, cancel: Arc<AtomicBool>) -> Self {
        self.cancel = Some(cancel);
        self
    }

    pub fn with_stream_reset<F>(mut self, on_reset: F) -> Self
    where
        F: Fn() + Send + Sync + 'static,
    {
        self.on_stream_reset = Some(Arc::new(on_reset));
        self
    }
}

fn check_cancelled(cancel: Option<&Arc<AtomicBool>>) -> Result<(), LlmError> {
    if cancel.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
        return Err(LlmError::Cancelled);
    }
    Ok(())
}

fn sleep_interruptible(duration: Duration, cancel: Option<&Arc<AtomicBool>>) -> Result<(), LlmError> {
    check_cancelled(cancel)?;
    if duration.is_zero() {
        return Ok(());
    }
    let deadline = Instant::now() + duration;
    loop {
        check_cancelled(cancel)?;
        let now = Instant::now();
        if now >= deadline {
            return Ok(());
        }
        let step = (deadline - now).min(CANCEL_POLL_INTERVAL);
        std::thread::sleep(step);
    }
}

fn is_retryable_status(status: u16) -> bool {
    matches!(status, 408 | 429 | 500 | 502 | 503 | 504 | 529)
}

fn should_try_fallback(err: &LlmError) -> bool {
    match err {
        LlmError::Status { status, .. } => is_retryable_status(*status),
        LlmError::Http(_) | LlmError::Io(_) => true,
        LlmError::Cancelled
        | LlmError::RepetitiveReasoning
        | LlmError::Media(_)
        | LlmError::Wire(_)
        | LlmError::Json(_) => false,
    }
}

/// Concrete HTTP transport client for executing `StructuredModelRequest`s.
#[derive(Clone)]
pub struct LlmClient {
    http: Client,
    http_loopback: Client,
    max_retries: usize,
    privacy_redaction: bool,
    audit_dir: Option<PathBuf>,
    default_fallbacks: Vec<ModelRouteConfig>,
}

impl LlmClient {
    pub fn new(timeout_secs: u64) -> Result<Self, LlmError> {
        let timeout = Duration::from_secs(timeout_secs.max(10));
        let http = Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| LlmError::Http(e.to_string()))?;
        let http_loopback = Client::builder()
            .timeout(timeout)
            .no_proxy()
            .build()
            .map_err(|e| LlmError::Http(e.to_string()))?;
        let audit_dir = std::env::var("NOLA_EVAL_REQUEST_AUDIT_DIR")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .map(PathBuf::from);
        Ok(Self {
            http,
            http_loopback,
            max_retries: 3,
            privacy_redaction: true,
            audit_dir,
            default_fallbacks: Vec::new(),
        })
    }

    /// Select the proxy-bypassing client for loopback URLs (`127.0.0.1`, `localhost`, `::1`),
    /// and the standard system-proxy client for remote URLs.
    pub fn client_for_url(&self, url: &str) -> &Client {
        if is_loopback_url(url) {
            &self.http_loopback
        } else {
            &self.http
        }
    }

    pub fn with_privacy_redaction(mut self, enabled: bool) -> Self {
        self.privacy_redaction = enabled;
        self
    }

    pub fn with_max_retries(mut self, max_retries: usize) -> Self {
        self.max_retries = max_retries;
        self
    }

    pub fn with_fallbacks(mut self, fallbacks: Vec<ModelRouteConfig>) -> Self {
        self.default_fallbacks = fallbacks;
        self
    }

    /// Execute a model request (streaming SSE when `stream == true`) with automatic retry
    /// on transient HTTP 429 / 5xx status codes.
    pub fn complete(
        &self,
        route: &ModelRouteConfig,
        req: &StructuredModelRequest,
        stream: bool,
        on_delta: &mut dyn FnMut(ModelStreamDelta),
    ) -> Result<NormalizedModelResponse, LlmError> {
        self.complete_with_params(route, req, &GenerationParams::default(), stream, on_delta)
    }

    /// Execute a model request with optional advanced `GenerationParams` (`seed`, `stop`, `top_k`, etc.).
    pub fn complete_with_params(
        &self,
        route: &ModelRouteConfig,
        req: &StructuredModelRequest,
        params: &GenerationParams,
        stream: bool,
        on_delta: &mut dyn FnMut(ModelStreamDelta),
    ) -> Result<NormalizedModelResponse, LlmError> {
        let call_params = CallParams {
            generation: params.clone(),
            ..Default::default()
        };
        self.complete_with_call_params(route, req, &call_params, stream, on_delta)
    }

    /// Execute a model request with cooperative cancellation support.
    pub fn complete_cancellable(
        &self,
        route: &ModelRouteConfig,
        req: &StructuredModelRequest,
        params: &GenerationParams,
        cancel: Option<Arc<AtomicBool>>,
        stream: bool,
        on_delta: &mut dyn FnMut(ModelStreamDelta),
    ) -> Result<NormalizedModelResponse, LlmError> {
        let call_params = CallParams {
            generation: params.clone(),
            cancel,
            ..Default::default()
        };
        self.complete_with_call_params(route, req, &call_params, stream, on_delta)
    }

    /// Execute a model request with full `CallParams`: generation params, ordered
    /// multi-endpoint fallbacks (skipped for `SiwcResponses`), cooperative cancellation,
    /// and 1-shot transactional `REPETITIVE_REASONING` resampling.
    pub fn complete_with_call_params(
        &self,
        route: &ModelRouteConfig,
        req: &StructuredModelRequest,
        call_params: &CallParams,
        stream: bool,
        on_delta: &mut dyn FnMut(ModelStreamDelta),
    ) -> Result<NormalizedModelResponse, LlmError> {
        let cancel = call_params.cancel.as_ref();
        check_cancelled(cancel)?;

        let mut routes: Vec<&ModelRouteConfig> = vec![route];
        // Preserve SIWC no-fallback invariant: SIWC routes must NEVER silently fall back.
        if route.protocol != ModelProtocol::SiwcResponses {
            let fallbacks = if !call_params.fallbacks.is_empty() {
                &call_params.fallbacks
            } else {
                &self.default_fallbacks
            };
            routes.extend(fallbacks.iter());
        }

        let mut last_err: Option<LlmError> = None;
        for (route_idx, candidate) in routes.iter().enumerate() {
            check_cancelled(cancel)?;
            match self.complete_single_route(candidate, req, call_params, stream, on_delta) {
                Ok(resp) => return Ok(resp),
                Err(err) => {
                    let has_more_routes = route_idx + 1 < routes.len();
                    if has_more_routes && should_try_fallback(&err) {
                        last_err = Some(err);
                        continue;
                    }
                    return Err(err);
                }
            }
        }
        Err(last_err.unwrap_or_else(|| LlmError::Http("unknown llm request failure".into())))
    }

    fn complete_single_route(
        &self,
        route: &ModelRouteConfig,
        req: &StructuredModelRequest,
        call_params: &CallParams,
        stream: bool,
        on_delta: &mut dyn FnMut(ModelStreamDelta),
    ) -> Result<NormalizedModelResponse, LlmError> {
        let cancel = call_params.cancel.as_ref();
        check_cancelled(cancel)?;

        let adapter = adapter_for(route.protocol);
        let sanitized = if self.privacy_redaction {
            let extra_secrets: [&str; 1] = [route.api_key.as_str()];
            PrivacyRedactor::sanitize_request_with_secrets(&route.endpoint, req, &extra_secrets)
        } else {
            req.clone()
        };
        let built =
            adapter.build_request_with_params(route, &sanitized, &call_params.generation, stream)?;
        self.write_audit_snapshot(&built.url, &built.body);

        let http = self.client_for_url(&built.url);
        let mut last_err = None;
        let mut did_repetitive_resample = false;
        let has_tools = !req.prefix_stable_tools.is_empty();

        let body_bytes = serde_json::to_vec(&built.body)?;
        let mut header_map = reqwest::header::HeaderMap::new();
        header_map.insert(
            reqwest::header::CONTENT_TYPE,
            reqwest::header::HeaderValue::from_static("application/json"),
        );
        for (k, v) in &built.headers {
            let name = reqwest::header::HeaderName::from_bytes(k.as_bytes())
                .map_err(|e| LlmError::Http(format!("invalid header name '{k}': {e}")))?;
            let value = reqwest::header::HeaderValue::from_str(v)
                .map_err(|e| LlmError::Http(format!("invalid header value for '{k}': {e}")))?;
            header_map.insert(name, value);
        }

        for attempt in 0..=self.max_retries {
            check_cancelled(cancel)?;
            let rb = http
                .post(&built.url)
                .headers(header_map.clone())
                .body(body_bytes.clone());
            match rb.send() {
                Ok(resp) => {
                    check_cancelled(cancel)?;
                    let status = resp.status().as_u16();
                    if (200..300).contains(&status) {
                        let is_sse = resp
                            .headers()
                            .get("content-type")
                            .and_then(|v| v.to_str().ok())
                            .map(|ct| ct.to_ascii_lowercase().contains("text/event-stream"))
                            .unwrap_or(stream);
                        if is_sse {
                            let mut reader = BufReader::new(resp);
                            let mut emitted_text_delta = false;
                            let mut tracked_delta = |delta: ModelStreamDelta| {
                                if matches!(&delta, ModelStreamDelta::Text(t) if !t.is_empty()) {
                                    emitted_text_delta = true;
                                }
                                on_delta(delta);
                            };
                            match adapter.parse_sse_stream_cancellable(
                                route,
                                &mut reader,
                                call_params.cancel.clone(),
                                &mut tracked_delta,
                            ) {
                                Ok(parsed) => return Ok(parsed),
                                Err(LlmError::RepetitiveReasoning)
                                    if !emitted_text_delta
                                        && has_tools
                                        && attempt == 0
                                        && !did_repetitive_resample
                                        && self.max_retries > 0 =>
                                {
                                    did_repetitive_resample = true;
                                    if let Some(reset_cb) = &call_params.on_stream_reset {
                                        reset_cb();
                                    }
                                    sleep_interruptible(Duration::from_millis(500), cancel)?;
                                    continue;
                                }
                                Err(err) => return Err(err),
                            }
                        }
                        let body: Value = resp
                            .json()
                            .map_err(|e| LlmError::Http(format!("response json decode failed: {e}")))?;
                        return adapter.decode_response(route, &body);
                    }
                    let retry_after_secs = resp
                        .headers()
                        .get("retry-after")
                        .and_then(|v| v.to_str().ok())
                        .and_then(|s| s.trim().parse::<u64>().ok())
                        .map(|s| s.min(30));
                    let body = resp.text().unwrap_or_default();
                    let retryable = is_retryable_status(status);
                    last_err = Some(LlmError::Status { status, body });
                    if !retryable || attempt == self.max_retries {
                        break;
                    }
                    if let Some(secs) = retry_after_secs {
                        sleep_interruptible(Duration::from_secs(secs), cancel)?;
                        continue;
                    }
                }
                Err(err) => {
                    check_cancelled(cancel)?;
                    let retryable = err.is_timeout() || err.is_connect();
                    last_err = Some(LlmError::Http(err.to_string()));
                    if !retryable || attempt == self.max_retries {
                        break;
                    }
                }
            }
            let backoff_ms = 250u64.saturating_mul(1u64 << attempt.min(4));
            sleep_interruptible(Duration::from_millis(backoff_ms), cancel)?;
        }
        Err(last_err.unwrap_or_else(|| LlmError::Http("unknown llm request failure".into())))
    }

    fn write_audit_snapshot(&self, url: &str, body: &Value) {
        let Some(dir) = &self.audit_dir else {
            return;
        };
        if std::fs::create_dir_all(dir).is_err() {
            return;
        }
        let seq = AUDIT_SEQ.fetch_add(1, Ordering::Relaxed);
        let path = dir.join(format!("request-{seq:04}.json"));
        let doc = serde_json::json!({
            "seq": seq,
            "url": url,
            "body": body,
        });
        if let Ok(bytes) = serde_json::to_vec_pretty(&doc) {
            let _ = std::fs::write(path, bytes);
        }
    }
}
