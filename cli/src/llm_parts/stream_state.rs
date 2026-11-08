// SPDX-License-Identifier: Apache-2.0
//! Provider-neutral stream accumulation and completion identity helpers.

use super::contracts::{LlmEvent, LlmUsage, StreamedChoice, ToolSlot};
use serde_json::Value;
use std::collections::BTreeMap;

pub(super) const REPETITIVE_REASONING: &str = "model_repetitive_reasoning";

/// Per physical attempt, independent of SSE chunk boundaries and provider.
/// Reuse the existing conservative repeated-block detector for reasoning too.
/// This is an explicit failed generation, never a successful shortened answer.
#[derive(Default)]
pub(super) struct ReasoningProgressGuard {
    recent: String,
    unchecked_bytes: usize,
}

impl ReasoningProgressGuard {
    pub(super) fn observe(&mut self, event: &LlmEvent) -> Result<(), String> {
        let LlmEvent::ReasoningDelta(delta) = event else { return Ok(()); };
        self.recent.push_str(delta);
        self.unchecked_bytes += delta.len();
        if self.unchecked_bytes < 128 { return Ok(()); }
        self.unchecked_bytes = 0;
        if let Some((boundary, _)) = self.recent.char_indices().rev().nth(4095) {
            self.recent.drain(..boundary);
        }
        if should_stop_repetitive_output(&self.recent) {
            return Err(REPETITIVE_REASONING.into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod reasoning_progress_tests {
    use super::*;

    #[test]
    fn stops_repeated_reasoning_across_arbitrary_chunks() {
        for size in [1, 7, 128, 4096] {
            let mut guard = ReasoningProgressGuard::default();
            let repeated = "Need maybe call web_fetch. ".repeat(180);
            assert!(repeated.as_bytes().chunks(size).any(|chunk| {
                guard.observe(&LlmEvent::ReasoningDelta(String::from_utf8(chunk.to_vec()).unwrap())).is_err()
            }));
        }
    }

    #[test]
    fn accepts_long_distinct_reasoning_and_ignores_intentional_answer_repetition() {
        let mut guard = ReasoningProgressGuard::default();
        for i in 0..500 {
            guard.observe(&LlmEvent::ReasoningDelta(format!("检查第 {i} 个候选，保留独立证据。"))).unwrap();
        }
        guard.observe(&LlmEvent::TextDelta("Repeated answer. ".repeat(500))).unwrap();
    }
}

pub(super) fn capture_openai_response_identity(choice: &mut StreamedChoice, envelope: &Value) {
    if choice.response_id.is_none() {
        choice.response_id = envelope
            .get("id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(str::to_string);
    }
    if choice.response_model.is_none() {
        choice.response_model = envelope
            .get("model")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|model| !model.is_empty())
            .map(str::to_string);
    }
}

pub(super) fn append_choice_text<F>(
    choice: &mut StreamedChoice,
    text: &str,
    on_event: &mut F,
) -> Result<bool, String>
where
    F: FnMut(LlmEvent) -> Result<(), String>,
{
    if text.is_empty() {
        return Ok(true);
    }
    let mut candidate = String::with_capacity(choice.content.len() + text.len());
    candidate.push_str(&choice.content);
    candidate.push_str(text);
    if should_stop_repetitive_output(&candidate) {
        if choice.finish_reason.is_empty() {
            choice.finish_reason = "repetition_guard".to_string();
        }
        return Ok(false);
    }
    choice.content.push_str(text);
    on_event(LlmEvent::TextDelta(text.to_string()))?;
    Ok(true)
}

pub(super) fn should_stop_repetitive_output(text: &str) -> bool {
    const MIN_CHARS: usize = 1800;
    const RECENT_CHAR_WINDOW: usize = 4096;
    const MIN_REPEAT_BLOCK_CHARS: usize = 128;
    const MAX_REPEAT_BLOCK_CHARS: usize = 1024;
    const REPEAT_LIMIT: usize = 3;

    if text.chars().count() < MIN_CHARS {
        return false;
    }

    // Repeated headings, table cells, and list labels are normal in long Markdown
    // answers. A generation loop repeats a substantial block at the output tail.
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

pub(super) fn stream_progress_size(
    choice: &StreamedChoice,
    slots: &BTreeMap<usize, ToolSlot>,
) -> usize {
    choice.content.len()
        + choice.reasoning.len()
        + slots
            .values()
            .map(|slot| slot.id.len() + slot.name.len() + slot.arguments.len())
            .sum::<usize>()
}

pub(super) fn stream_has_progress(
    choice: &StreamedChoice,
    slots: &BTreeMap<usize, ToolSlot>,
) -> bool {
    stream_progress_size(choice, slots) > 0
}

pub(super) fn merge_usage_from_envelope(target: &mut LlmUsage, envelope: &Value) {
    let usage = envelope.get("usage").or_else(|| {
        envelope
            .get("message")
            .and_then(|message| message.get("usage"))
    });
    let Some(usage) = usage else { return };

    if let Some(value) = usage
        .get("prompt_tokens")
        .or_else(|| usage.get("input_tokens"))
        .and_then(Value::as_u64)
    {
        target.input_tokens = usize::try_from(value).ok();
    }
    if let Some(value) = usage
        .get("completion_tokens")
        .or_else(|| usage.get("output_tokens"))
        .and_then(Value::as_u64)
    {
        target.output_tokens = usize::try_from(value).ok();
    }

    let cached = usage
        .get("prompt_tokens_details")
        .and_then(|details| details.get("cached_tokens"))
        .or_else(|| usage.get("input_tokens_details").and_then(|details| details.get("cached_tokens")))
        .or_else(|| usage.get("cache_read_input_tokens"));
    if let Some(value) = cached.and_then(Value::as_u64) {
        target.cached_input_tokens = Some(usize::try_from(value).unwrap_or(usize::MAX));
    }
    if let Some(value) = usage
        .get("cache_creation_input_tokens")
        .and_then(Value::as_u64)
    {
        target.cache_write_input_tokens = Some(usize::try_from(value).unwrap_or(usize::MAX));
    }

    if let Some(value) = usage.get("input_tokens_details").and_then(|details| details.get("cache_write_tokens")).and_then(Value::as_u64) {
        target.cache_write_input_tokens = usize::try_from(value).ok();
    }
    if let Some(value) = usage.get("output_tokens_details").or_else(|| usage.get("completion_tokens_details"))
        .and_then(|details| details.get("reasoning_tokens")).and_then(Value::as_u64) {
        target.reasoning_tokens = usize::try_from(value).ok();
    }
    if let Some(value) = usage.get("total_tokens").and_then(Value::as_u64) {
        target.total_tokens = usize::try_from(value).ok();
    }
}
