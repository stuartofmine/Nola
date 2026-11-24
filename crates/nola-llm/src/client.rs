// SPDX-License-Identifier: Apache-2.0
//! Blocking HTTP client with retries, streaming SSE, and optional audit request logging.

use std::io::BufReader;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use nola_types::{
    ModelRouteConfig, ModelStreamDelta, NormalizedModelResponse, StructuredModelRequest,
};
use reqwest::blocking::Client;
use serde_json::Value;

use crate::adapter::{adapter_for, LlmError};
use crate::privacy::PrivacyRedactor;

static AUDIT_SEQ: AtomicU64 = AtomicU64::new(1);

/// Concrete HTTP transport client for executing `StructuredModelRequest`s.
#[derive(Clone)]
pub struct LlmClient {
    http: Client,
    max_retries: usize,
    privacy_redaction: bool,
    audit_dir: Option<PathBuf>,
}

impl LlmClient {
    pub fn new(timeout_secs: u64) -> Result<Self, LlmError> {
        let http = Client::builder()
            .timeout(Duration::from_secs(timeout_secs.max(10)))
            .build()
            .map_err(|e| LlmError::Http(e.to_string()))?;
        let audit_dir = std::env::var("NOLA_EVAL_REQUEST_AUDIT_DIR")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .map(PathBuf::from);
        Ok(Self {
            http,
            max_retries: 3,
            privacy_redaction: true,
            audit_dir,
        })
    }

    pub fn with_privacy_redaction(mut self, enabled: bool) -> Self {
        self.privacy_redaction = enabled;
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
        let adapter = adapter_for(route.protocol);
        let sanitized = if self.privacy_redaction {
            PrivacyRedactor::sanitize_request(&route.endpoint, req)
        } else {
            req.clone()
        };
        let built = adapter.build_request(route, &sanitized, stream)?;
        self.write_audit_snapshot(&built.url, &built.body);

        let mut last_err = None;
        for attempt in 0..=self.max_retries {
            let mut rb = self.http.post(&built.url).json(&built.body);
            for (k, v) in &built.headers {
                rb = rb.header(k.as_str(), v.as_str());
            }
            match rb.send() {
                Ok(resp) => {
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
                            return adapter.parse_sse_stream(route, &mut reader, on_delta);
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
                    let retryable = matches!(status, 408 | 429 | 500 | 502 | 503 | 504 | 529);
                    last_err = Some(LlmError::Status { status, body });
                    if !retryable || attempt == self.max_retries {
                        break;
                    }
                    if let Some(secs) = retry_after_secs {
                        std::thread::sleep(Duration::from_secs(secs));
                        continue;
                    }
                }
                Err(err) => {
                    let retryable = err.is_timeout() || err.is_connect();
                    last_err = Some(LlmError::Http(err.to_string()));
                    if !retryable || attempt == self.max_retries {
                        break;
                    }
                }
            }
            let backoff_ms = 250u64.saturating_mul(1u64 << attempt.min(4));
            std::thread::sleep(Duration::from_millis(backoff_ms));
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
