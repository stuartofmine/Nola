// SPDX-License-Identifier: Apache-2.0
//! Bounded streaming Server-Sent Events (SSE) parser and `ReasoningProgressGuard`.

use std::io::{BufRead, Read};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use nola_types::ModelStreamDelta;

use crate::LlmError;

/// Maximum permitted length of a single SSE line (16 MiB).
pub const MAX_SSE_LINE_BYTES: usize = 16 * 1024 * 1024;

/// Canonical error code identifying a degenerate repetitive reasoning abort.
pub const REPETITIVE_REASONING_CODE: &str = "model_repetitive_reasoning";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    pub event: Option<String>,
    pub data: String,
}

/// Pull-based SSE reader over any `BufRead` with bounded line reads (`MAX_SSE_LINE_BYTES`)
/// and optional cooperative cancellation checks between line reads.
pub struct SseReader<'a> {
    inner: &'a mut dyn BufRead,
    raw_buf: Vec<u8>,
    saw_first_line: bool,
    max_line_bytes: usize,
    cancel: Option<Arc<AtomicBool>>,
}

impl<'a> SseReader<'a> {
    pub fn new(inner: &'a mut dyn BufRead) -> Self {
        Self {
            inner,
            raw_buf: Vec::new(),
            saw_first_line: false,
            max_line_bytes: MAX_SSE_LINE_BYTES,
            cancel: None,
        }
    }

    pub fn with_cancel(mut self, cancel: Option<Arc<AtomicBool>>) -> Self {
        self.cancel = cancel;
        self
    }

    pub fn with_max_line_bytes(mut self, max_line_bytes: usize) -> Self {
        self.max_line_bytes = max_line_bytes.max(1);
        self
    }

    fn check_cancelled(&self) -> Result<(), LlmError> {
        if self
            .cancel
            .as_ref()
            .is_some_and(|flag| flag.load(Ordering::Relaxed))
        {
            return Err(LlmError::Cancelled);
        }
        Ok(())
    }

    pub fn next_event(&mut self) -> Result<Option<SseEvent>, LlmError> {
        let mut event_name: Option<String> = None;
        let mut data_lines: Vec<String> = Vec::new();
        loop {
            self.check_cancelled()?;
            self.raw_buf.clear();
            let limit = (self.max_line_bytes as u64).saturating_add(1);
            let n = (&mut *self.inner)
                .take(limit)
                .read_until(b'\n', &mut self.raw_buf)?;
            self.check_cancelled()?;
            if n == 0 {
                if data_lines.is_empty() {
                    return Ok(None);
                }
                return Ok(Some(SseEvent {
                    event: event_name,
                    data: data_lines.join("\n"),
                }));
            }
            let effective_len = if self.raw_buf.ends_with(b"\r\n") {
                self.raw_buf.len().saturating_sub(2)
            } else if self.raw_buf.ends_with(b"\n") {
                self.raw_buf.len().saturating_sub(1)
            } else {
                self.raw_buf.len()
            };
            if effective_len > self.max_line_bytes {
                return Err(LlmError::Wire(format!(
                    "SSE line exceeded maximum allowed size of {} bytes",
                    self.max_line_bytes
                )));
            }
            let raw_str = std::str::from_utf8(&self.raw_buf).map_err(|e| {
                LlmError::Io(std::io::Error::new(std::io::ErrorKind::InvalidData, e))
            })?;
            let mut line = raw_str.trim_end_matches(&['\r', '\n'][..]);
            if !self.saw_first_line {
                self.saw_first_line = true;
                line = line.strip_prefix('\u{feff}').unwrap_or(line);
            }
            if line.is_empty() {
                if !data_lines.is_empty() {
                    return Ok(Some(SseEvent {
                        event: event_name,
                        data: data_lines.join("\n"),
                    }));
                }
                event_name = None;
                continue;
            }
            if line.starts_with(':') {
                continue;
            }
            if let Some(rest) = line.strip_prefix("event:") {
                event_name = Some(rest.trim().to_string());
            } else if let Some(rest) = line.strip_prefix("data:") {
                let value = rest.strip_prefix(' ').unwrap_or(rest);
                data_lines.push(value.to_string());
            }
        }
    }
}

/// Conservative tail repeated-block detector shared across all protocols.
/// Detects a degenerate loop repeating a substantial block (>= 128 chars, <= 1024 chars)
/// 3 contiguous times at the output tail once at least 1800 characters have accumulated.
pub fn should_stop_repetitive_output(text: &str) -> bool {
    const MIN_CHARS: usize = 1800;
    const RECENT_CHAR_WINDOW: usize = 4096;
    const MIN_REPEAT_BLOCK_CHARS: usize = 128;
    const MAX_REPEAT_BLOCK_CHARS: usize = 1024;
    const REPEAT_LIMIT: usize = 3;

    if text.chars().count() < MIN_CHARS {
        return false;
    }

    let mut recent = text
        .chars()
        .rev()
        .take(RECENT_CHAR_WINDOW)
        .collect::<Vec<_>>();
    recent.reverse();
    let max_block = MAX_REPEAT_BLOCK_CHARS.min(recent.len() / REPEAT_LIMIT);
    if max_block < MIN_REPEAT_BLOCK_CHARS {
        return false;
    }

    for block_len in MIN_REPEAT_BLOCK_CHARS..=max_block {
        let suffix_start = recent.len() - block_len;
        let suffix = &recent[suffix_start..];
        let repeated = (1..REPEAT_LIMIT).all(|repeat_index| {
            let start = recent.len() - block_len * (repeat_index + 1);
            &recent[start..start + block_len] == suffix
        });
        if repeated {
            return true;
        }
    }

    false
}

/// Per-physical-attempt streaming reasoning guard, independent of SSE chunk
/// boundaries and provider protocol.
///
/// Only observes reasoning deltas — intentional repetition in final answer text
/// is never rejected by this guard.
#[derive(Debug, Default, Clone)]
pub struct ReasoningProgressGuard {
    recent: String,
    unchecked_bytes: usize,
}

impl ReasoningProgressGuard {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn observe_delta(&mut self, delta: &ModelStreamDelta) -> Result<(), LlmError> {
        let ModelStreamDelta::Reasoning(text) = delta else {
            return Ok(());
        };
        self.observe_reasoning(text)
    }

    pub fn observe_reasoning(&mut self, delta: &str) -> Result<(), LlmError> {
        self.recent.push_str(delta);
        self.unchecked_bytes = self.unchecked_bytes.saturating_add(delta.len());
        if self.unchecked_bytes < 128 {
            return Ok(());
        }
        self.unchecked_bytes = 0;
        if let Some((boundary, _)) = self.recent.char_indices().rev().nth(4095) {
            self.recent.drain(..boundary);
        }
        if should_stop_repetitive_output(&self.recent) {
            return Err(LlmError::RepetitiveReasoning);
        }
        Ok(())
    }
}
