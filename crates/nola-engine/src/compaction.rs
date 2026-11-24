// SPDX-License-Identifier: Apache-2.0
//! Two-Tier Context Compaction Engine:
//! - Tier 1 (Zero-Cost Micro-Compaction): ALWAYS runs first and commits partial savings
//!   immediately by shrinking older oversized tool payloads while keeping recent 6 untouched.
//! - Tier 2 (Structured Summary Compaction): runs if history still exceeds threshold,
//!   validated against exact `ContextBudget::history_budget(...)`, never permanently locking out.

use nola_types::{
    estimate_history_message_tokens, ContextBudget, HistoryMessage, ToolModelPayload,
};
use serde_json::json;

const RECENT_TOOL_KEEP_COUNT: usize = 6;
const MICRO_COMPACT_CHAR_THRESHOLD: usize = 1_200;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CompactionReport {
    pub tokens_before: usize,
    pub tokens_after: usize,
    pub micro_compacted_tools: usize,
    pub summary_compacted_messages: usize,
}

impl CompactionReport {
    pub fn saved_tokens(&self) -> usize {
        self.tokens_before.saturating_sub(self.tokens_after)
    }
}

pub fn estimate_history_tokens(history: &[HistoryMessage]) -> usize {
    history.iter().map(estimate_history_message_tokens).sum()
}

/// Run Tier-1 micro-compaction first (committing any partial savings!), then Tier-2
/// structured summary compaction only if history still exceeds the threshold.
pub fn compact_history(
    history: &mut Vec<HistoryMessage>,
    budget: ContextBudget,
    fixed_overhead_tokens: usize,
    summarizer: Option<&dyn Fn(&[HistoryMessage]) -> Option<String>>,
) -> CompactionReport {
    let tokens_before = estimate_history_tokens(history);
    let trigger = budget.history_budget(fixed_overhead_tokens, budget.auto_compact_threshold());
    let target = budget.history_budget(fixed_overhead_tokens, budget.compact_target_tokens());

    if tokens_before <= trigger {
        return CompactionReport {
            tokens_before,
            tokens_after: tokens_before,
            ..Default::default()
        };
    }

    // --- Tier 1: Zero-cost Micro-Compaction (ALWAYS commits partial savings!) ---
    let mut micro_compacted_tools = 0usize;
    let tool_indices: Vec<usize> = history
        .iter()
        .enumerate()
        .filter_map(|(i, m)| matches!(m, HistoryMessage::Tool { .. }).then_some(i))
        .collect();
    let eligible_count = tool_indices.len().saturating_sub(RECENT_TOOL_KEEP_COUNT);

    for &idx in tool_indices.iter().take(eligible_count) {
        if let HistoryMessage::Tool {
            tool_name,
            payload,
            media,
            ..
        } = &mut history[idx]
        {
            let serialized = payload.to_model_string();
            if serialized.len() > MICRO_COMPACT_CHAR_THRESHOLD || !media.is_empty() {
                let summary: String = payload.ui_summary().chars().take(160).collect();
                let preview: String = serialized.chars().take(240).collect();
                *payload = ToolModelPayload::structured(
                    json!({
                        "compacted": true,
                        "tool": tool_name,
                        "original_bytes": serialized.len(),
                        "summary": summary,
                        "preview": preview
                    }),
                    summary,
                );
                media.clear();
                micro_compacted_tools += 1;
            }
        }
    }

    let after_tier1 = estimate_history_tokens(history);
    if after_tier1 <= target || history.len() <= 6 {
        return CompactionReport {
            tokens_before,
            tokens_after: after_tier1,
            micro_compacted_tools,
            summary_compacted_messages: 0,
        };
    }

    // --- Tier 2: Structured Summary Compaction (preserves recent tail & validates budget!) ---
    // Choose split boundary cleanly on a User or Assistant turn keeping >= 4 recent messages.
    let keep_recent = 6.min(history.len().saturating_sub(2));
    let mut split_idx = history.len().saturating_sub(keep_recent);
    // Avoid splitting between an Assistant tool_calls turn and its matching Tool results!
    while split_idx > 1 && matches!(history.get(split_idx), Some(HistoryMessage::Tool { .. })) {
        split_idx -= 1;
    }
    if split_idx < 2 {
        return CompactionReport {
            tokens_before,
            tokens_after: after_tier1,
            micro_compacted_tools,
            summary_compacted_messages: 0,
        };
    }

    let old_slice = &history[..split_idx];
    let summary_text = summarizer
        .and_then(|f| f(old_slice))
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| deterministic_ledger_summary(old_slice));

    let mut candidate = Vec::with_capacity(history.len() - split_idx + 1);
    candidate.push(HistoryMessage::User {
        content: format!("<compacted_history_summary>\n{summary_text}\n</compacted_history_summary>"),
        media: Vec::new(),
    });
    candidate.extend(history[split_idx..].iter().cloned());

    let after_tier2 = estimate_history_tokens(&candidate);
    if after_tier2 < after_tier1 {
        *history = candidate;
        CompactionReport {
            tokens_before,
            tokens_after: after_tier2,
            micro_compacted_tools,
            summary_compacted_messages: split_idx,
        }
    } else {
        CompactionReport {
            tokens_before,
            tokens_after: after_tier1,
            micro_compacted_tools,
            summary_compacted_messages: 0,
        }
    }
}

fn deterministic_ledger_summary(slice: &[HistoryMessage]) -> String {
    let mut lines = vec![format!("Compacted {} earlier messages:", slice.len())];
    for msg in slice {
        match msg {
            HistoryMessage::User { content, .. } => {
                let first: String = content.lines().next().unwrap_or("").chars().take(160).collect();
                lines.push(format!("- User: {first}"));
            }
            HistoryMessage::Assistant {
                content,
                tool_calls,
                ..
            } => {
                if !tool_calls.is_empty() {
                    let names: Vec<&str> = tool_calls.iter().map(|c| c.name.as_str()).collect();
                    lines.push(format!("- Assistant called tools: {}", names.join(", ")));
                } else if !content.trim().is_empty() {
                    let first: String = content.lines().next().unwrap_or("").chars().take(160).collect();
                    lines.push(format!("- Assistant: {first}"));
                }
            }
            HistoryMessage::Tool {
                tool_name,
                payload,
                is_error,
                ..
            } => {
                let tag = if *is_error { "ERR" } else { "OK" };
                let summary: String = payload.ui_summary().chars().take(140).collect();
                lines.push(format!("- Tool[{tool_name} {tag}]: {summary}"));
            }
        }
    }
    lines.join("\n")
}
