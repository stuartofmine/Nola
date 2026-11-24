// SPDX-License-Identifier: Apache-2.0
//! Single unified UTF-8/CJK token estimator and context budget calculator.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{HistoryMessage, StructuredModelRequest};

/// Estimate tokens accurately for both ASCII code/JSON and non-ASCII CJK text.
/// ASCII averages ~4 bytes/token; CJK/non-ASCII averages ~1 token per scalar.
pub fn estimate_text_tokens(text: &str) -> usize {
    let mut ascii = 0usize;
    let mut non_ascii = 0usize;
    for ch in text.chars() {
        if ch.is_ascii() {
            ascii += 1;
        } else {
            non_ascii += 1;
        }
    }
    ascii.div_ceil(4).saturating_add(non_ascii)
}

/// Estimate tokens for a JSON value (including structural framing overhead).
pub fn estimate_json_tokens(value: &Value) -> usize {
    let serialized = serde_json::to_string(value).unwrap_or_default();
    estimate_text_tokens(&serialized).saturating_add(4)
}

/// Estimate tokens for a single history message.
pub fn estimate_history_message_tokens(msg: &HistoryMessage) -> usize {
    match msg {
        HistoryMessage::User { content, media } => estimate_text_tokens(content)
            .saturating_add(media.len().saturating_mul(1_024))
            .saturating_add(6),
        HistoryMessage::Assistant {
            content,
            reasoning,
            tool_calls,
            ..
        } => {
            let calls: usize = tool_calls
                .iter()
                .map(|c| {
                    estimate_text_tokens(&c.name)
                        .saturating_add(estimate_json_tokens(&c.arguments))
                        .saturating_add(8)
                })
                .sum();
            estimate_text_tokens(content)
                .saturating_add(reasoning.as_deref().map(estimate_text_tokens).unwrap_or(0))
                .saturating_add(calls)
                .saturating_add(6)
        }
        HistoryMessage::Tool {
            tool_name, payload, ..
        } => estimate_text_tokens(tool_name)
            .saturating_add(estimate_json_tokens(&payload.to_model_value()))
            .saturating_add(6),
    }
}

/// Estimate total input tokens of a complete structured model request.
pub fn estimate_request_tokens(req: &StructuredModelRequest) -> usize {
    let prefix: usize = req
        .frozen_system_prefix
        .iter()
        .map(|b| estimate_text_tokens(&b.text).saturating_add(4))
        .sum();
    let tools: usize = req
        .prefix_stable_tools
        .iter()
        .map(|t| {
            estimate_text_tokens(&t.name)
                .saturating_add(estimate_text_tokens(&t.description))
                .saturating_add(estimate_json_tokens(&t.parameters))
        })
        .sum();
    let history: usize = req
        .history
        .iter()
        .map(estimate_history_message_tokens)
        .sum();
    let tail: usize = req
        .ephemeral_tail_reminders
        .iter()
        .map(|r| estimate_text_tokens(&r.text).saturating_add(8))
        .sum();
    prefix
        .saturating_add(tools)
        .saturating_add(history)
        .saturating_add(tail)
}

/// Single authoritative context budget policy (no competing hardcoded ceilings).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextBudget {
    pub context_window_tokens: usize,
    pub reserved_output_tokens: usize,
    /// Percentage of `prompt_token_limit` at which auto-compaction triggers (default 75%).
    pub auto_compact_percent: u8,
    /// Target percentage of `prompt_token_limit` after compaction (default 45%).
    pub compact_target_percent: u8,
}

impl ContextBudget {
    pub fn for_window(context_window_tokens: usize, reasoning_enabled: bool) -> Self {
        let window = context_window_tokens.clamp(4_096, 4_000_000);
        // Large-window reasoning models get 32,768 default output tokens so thinking
        // chains + structured tool-call JSON do not truncate mid-argument at 16,384.
        let default_output = if reasoning_enabled && window >= 128_000 {
            32_768
        } else {
            16_384
        };
        let reserved_output_tokens = default_output.min(window / 3).max(512);
        Self {
            context_window_tokens: window,
            reserved_output_tokens,
            auto_compact_percent: 75,
            compact_target_percent: 45,
        }
    }

    pub fn with_explicit_output_limit(mut self, max_output_tokens: Option<usize>) -> Self {
        if let Some(limit) = max_output_tokens.filter(|&v| v > 0) {
            self.reserved_output_tokens = limit.clamp(256, self.context_window_tokens.saturating_sub(512));
        }
        self
    }

    /// Safety margin reserved for tokenizer variance (5% bounded to [512, 4096]).
    pub fn safety_margin(&self) -> usize {
        (self.context_window_tokens / 20).clamp(512, 4_096)
    }

    /// Maximum input prompt tokens available before hitting the provider window.
    pub fn prompt_token_limit(&self) -> usize {
        self.context_window_tokens
            .saturating_sub(self.reserved_output_tokens)
            .saturating_sub(self.safety_margin())
            .max(1_024)
    }

    /// Auto-compaction threshold scaling cleanly with window size (no arbitrary 96K cap).
    pub fn auto_compact_threshold(&self) -> usize {
        let pct = self.auto_compact_percent.clamp(50, 92) as usize;
        self.prompt_token_limit().saturating_mul(pct) / 100
    }

    /// Post-compaction target tokens.
    pub fn compact_target_tokens(&self) -> usize {
        let pct = self
            .compact_target_percent
            .clamp(20, self.auto_compact_percent.saturating_sub(5).max(20)) as usize;
        (self.prompt_token_limit().saturating_mul(pct) / 100)
            .min(self.auto_compact_threshold().saturating_sub(1).max(1))
    }

    /// History budget after subtracting fixed prefix + tool schemas overhead.
    pub fn history_budget(&self, fixed_overhead_tokens: usize, limit_tokens: usize) -> usize {
        limit_tokens.saturating_sub(fixed_overhead_tokens).max(512)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_scales_across_128k_262k_and_1m_windows() {
        let b128 = ContextBudget::for_window(128_000, false);
        assert_eq!(b128.reserved_output_tokens, 16_384);
        assert!(b128.auto_compact_threshold() > 75_000 && b128.auto_compact_threshold() < 85_000);

        let b262 = ContextBudget::for_window(262_144, true);
        assert_eq!(b262.reserved_output_tokens, 32_768);
        // 262K window triggers compaction around ~168K, NOT prematurely at 96K
        assert!(b262.auto_compact_threshold() > 160_000);

        let b1m = ContextBudget::for_window(1_000_000, true);
        assert!(b1m.auto_compact_threshold() > 700_000);
    }

    #[test]
    fn token_estimator_handles_cjk_and_ascii_accurately() {
        assert_eq!(estimate_text_tokens("abcdefgh"), 2);
        // 6 CJK chars should estimate >= 6 tokens (not 2 like chars/4!)
        assert_eq!(estimate_text_tokens("你好世界测试"), 6);
    }
}
