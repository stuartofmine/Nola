// SPDX-License-Identifier: Apache-2.0
//! Bounded transparent observation of provider usage. No prompt/output text is
//! persisted; provider totals are snapshots, never additive stream deltas.
use crate::store::NolaStore;
use nola_application::{ApplicationCommand, ModelCommand};
use nola_domain::model_connection::{
    GatewayAttemptOutcome, GatewayAttemptRecord, GatewayAttemptSnapshot, GatewayUsage,
};
use serde_json::Value;
use std::path::PathBuf;

const MAX_OBSERVATION_BYTES: usize = 2 * 1024 * 1024;
pub(crate) struct GatewayUsageTracker {
    database: PathBuf,
    snapshot: GatewayAttemptSnapshot,
    buffer: Vec<u8>,
    event_data: Vec<u8>,
    event_name: String,
    event_oversized: bool,
    streaming: bool,
    oversized: bool,
    terminal: bool,
    provider_failed: bool,
    finalized: bool,
}
impl GatewayUsageTracker {
    pub fn start(store: &NolaStore, record: GatewayAttemptRecord) -> Result<Self, String> {
        let snapshot = store.application()?.execute(ApplicationCommand::Model {
            command: ModelCommand::RecordGatewayAttempt {
                input: record,
                expected_revision: None,
            },
        })?;
        Ok(Self {
            database: store.path().to_owned(),
            snapshot,
            buffer: vec![],
            event_data: vec![],
            event_name: String::new(),
            event_oversized: false,
            streaming: false,
            oversized: false,
            terminal: false,
            provider_failed: false,
            finalized: false,
        })
    }
    pub fn id(&self) -> &str {
        &self.snapshot.record.id
    }
    pub fn response(&mut self, status: u16, streaming: bool) {
        self.snapshot.record.http_status = Some(status);
        self.streaming = streaming;
        self.provider_failed = status >= 400;
    }
    pub fn observe(&mut self, bytes: &[u8]) {
        if !self.streaming {
            if !self.oversized
                && self.buffer.len().saturating_add(bytes.len()) <= MAX_OBSERVATION_BYTES
            {
                self.buffer.extend_from_slice(bytes);
            } else {
                self.buffer.clear();
                self.oversized = true;
            }
            return;
        }
        for byte in bytes {
            if *byte == b'\n' {
                if !self.oversized {
                    let line = std::mem::take(&mut self.buffer);
                    let line = line.strip_suffix(b"\r").unwrap_or(&line);
                    if line.is_empty() {
                        if !self.event_oversized {
                            let data = std::mem::take(&mut self.event_data);
                            if data.trim_ascii() == b"[DONE]" {
                                self.terminal = true;
                            } else if let Ok(value) = serde_json::from_slice::<Value>(&data) {
                                if matches!(
                                    self.event_name.as_str(),
                                    "response.completed" | "message_stop"
                                ) {
                                    self.terminal = true;
                                }
                                if matches!(
                                    self.event_name.as_str(),
                                    "response.failed" | "response.incomplete" | "error"
                                ) {
                                    self.provider_failed = true;
                                }
                                self.event(&value);
                            }
                        }
                        self.event_data.clear();
                        self.event_name.clear();
                        self.event_oversized = false;
                    } else if let Some(data) = line.strip_prefix(b"data:") {
                        if !self.event_oversized
                            && self.event_data.len().saturating_add(data.len() + 1)
                                <= MAX_OBSERVATION_BYTES
                        {
                            self.event_data.extend_from_slice(data);
                            self.event_data.push(b'\n');
                        } else {
                            self.event_data.clear();
                            self.event_oversized = true;
                        }
                    } else if let Some(name) = line.strip_prefix(b"event:") {
                        if name.len() <= 128 {
                            self.event_name =
                                String::from_utf8_lossy(name.trim_ascii()).into_owned();
                        }
                    }
                } else {
                    self.event_data.clear();
                    self.event_oversized = true;
                }
                self.buffer.clear();
                self.oversized = false;
            } else if !self.oversized {
                if self.buffer.len() < MAX_OBSERVATION_BYTES {
                    self.buffer.push(*byte);
                } else {
                    self.buffer.clear();
                    self.oversized = true;
                }
            }
        }
    }
    fn event(&mut self, value: &Value) {
        if matches!(
            value["type"].as_str(),
            Some("response.completed" | "message_stop")
        ) {
            self.terminal = true;
        }
        if matches!(
            value["type"].as_str(),
            Some("response.failed" | "response.incomplete" | "error")
        ) || value.get("error").is_some_and(|error| !error.is_null())
        {
            self.provider_failed = true;
        }
        for usage in [
            &value["usage"],
            &value["response"]["usage"],
            &value["message"]["usage"],
        ] {
            self.merge_usage(usage);
        }
    }
    fn merge_usage(&mut self, value: &Value) {
        if !value.is_object() {
            return;
        }
        let incoming = GatewayUsage {
            input_tokens: value["input_tokens"]
                .as_u64()
                .or_else(|| value["prompt_tokens"].as_u64()),
            output_tokens: value["output_tokens"]
                .as_u64()
                .or_else(|| value["completion_tokens"].as_u64()),
            cached_input_tokens: value["input_tokens_details"]["cached_tokens"]
                .as_u64()
                .or_else(|| value["prompt_tokens_details"]["cached_tokens"].as_u64())
                .or_else(|| value["cache_read_input_tokens"].as_u64()),
            cache_creation_input_tokens: value["cache_creation_input_tokens"].as_u64(),
            reasoning_output_tokens: value["output_tokens_details"]["reasoning_tokens"]
                .as_u64()
                .or_else(|| value["completion_tokens_details"]["reasoning_tokens"].as_u64()),
        };
        if incoming == GatewayUsage::default() {
            return;
        }
        let current = self
            .snapshot
            .record
            .usage
            .get_or_insert_with(Default::default);
        let merge = |current: &mut Option<u64>, next: Option<u64>| {
            if let Some(next) = next {
                *current = Some(current.map_or(next, |old| old.max(next)));
            }
        };
        merge(&mut current.input_tokens, incoming.input_tokens);
        merge(&mut current.output_tokens, incoming.output_tokens);
        merge(
            &mut current.cached_input_tokens,
            incoming.cached_input_tokens,
        );
        merge(
            &mut current.cache_creation_input_tokens,
            incoming.cache_creation_input_tokens,
        );
        merge(
            &mut current.reasoning_output_tokens,
            incoming.reasoning_output_tokens,
        );
    }
    pub fn eof(&mut self) -> Result<(), String> {
        if !self.streaming && !self.oversized {
            if let Ok(value) = serde_json::from_slice::<Value>(&self.buffer) {
                self.event(&value);
            }
        }
        let outcome = if self.provider_failed {
            GatewayAttemptOutcome::UpstreamRejected
        } else if self.streaming && !self.terminal {
            GatewayAttemptOutcome::Interrupted
        } else {
            GatewayAttemptOutcome::Completed
        };
        self.finish(outcome)
    }
    pub fn finish(&mut self, outcome: GatewayAttemptOutcome) -> Result<(), String> {
        if self.finalized {
            return Ok(());
        }
        self.snapshot.record.outcome = outcome;
        self.snapshot.record.finished_at_ms = Some(
            chrono::Utc::now()
                .timestamp_millis()
                .max(self.snapshot.record.started_at_ms),
        );
        let store = NolaStore::open(&self.database)?;
        self.snapshot = store.application()?.execute(ApplicationCommand::Model {
            command: ModelCommand::RecordGatewayAttempt {
                input: self.snapshot.record.clone(),
                expected_revision: Some(self.snapshot.revision),
            },
        })?;
        self.finalized = true;
        Ok(())
    }
}
impl Drop for GatewayUsageTracker {
    fn drop(&mut self) {
        if !self.finalized {
            // A failed write leaves the durable initial attempt InFlight with
            // unknown completion; it never fabricates completion or zero usage.
            let _ = self.finish(GatewayAttemptOutcome::Interrupted);
        }
    }
}

pub fn list_gateway_attempts(
    store: &NolaStore,
    limit: usize,
    offset: usize,
) -> Result<Vec<GatewayAttemptSnapshot>, String> {
    store.application()?.execute(ApplicationCommand::Model {
        command: ModelCommand::ListGatewayAttempts { limit, offset },
    })
}

#[cfg(test)]
#[path = "model_gateway_usage_tests.rs"]
mod tests;
